//! Analytic geometry protects shared tracks and cap-contact continuations.

use super::*;
use crate::model::DockspaceGridShape;
use crate::presentation::config::ResizePropagation;

mod rounded_contacts;

fn grid_handle(
    fixture: &Fixture,
    owner: NodeId,
    axis: Axis,
    index: usize,
) -> (NodeSource, SplitterRecord) {
    let grid = fixture.projection.grids.get(&owner).unwrap();
    let tracks = grid.axis(axis);
    let before = tracks.tracks[index];
    let after = tracks.tracks[index + 1];
    let draw = before.following_gap.unwrap();
    let record = SplitterRecord::new(
        SplitterSceneId::grid(ROOT, owner, axis, index),
        draw,
        HitRegion::new(draw),
        axis,
        before.bounds,
        after.bounds,
        before.minimum,
        after.minimum,
        before.maximum,
        after.maximum,
        tracks
            .tracks
            .iter()
            .map(|track| track.extent)
            .collect::<Vec<_>>(),
        tracks.central_index,
        tracks.weights.clone(),
        SceneLayerKey::surface_base(),
        true,
    )
    .with_grid_proof(crate::presentation::scene::GridBoundaryProof::new(
        (grid.shape.rows, grid.shape.columns),
        Arc::clone(&grid.children),
    ));
    (
        fixture.workspace.capture_node_source(ROOT, owner).unwrap(),
        record,
    )
}

fn grid_fixture() -> (Fixture, NodeId, [NodeId; 4]) {
    let mut builder = Workspace::builder();
    let cells = [1, 2, 3, 4].map(|item| {
        builder.insert_node(Node::Pane {
            item: ItemId::new(item),
        })
    });
    let grid = builder.insert_node(Node::Grid {
        shape: DockspaceGridShape::Fixed {
            rows: std::num::NonZeroUsize::new(2).unwrap(),
            columns: std::num::NonZeroUsize::new(2).unwrap(),
        },
        children: cells.to_vec(),
        shares: None,
        layout_title: None,
    });
    let fixture = Fixture::new(
        finish(builder, grid, None),
        rect(404.0, 204.0),
        cells
            .into_iter()
            .map(|node| (node, constraints(20.0, 2000.0)))
            .collect(),
        4.0,
    );
    (fixture, grid, cells)
}

#[test]
fn shared_grid_junction_updates_both_axes_of_one_owner_atomically() {
    let (fixture, owner, cells) = grid_fixture();
    for node in cells {
        assert_eq!(
            fixture.projection.nodes[&node].rect.size(),
            LogicalSize::new(200.0, 100.0).unwrap()
        );
    }
    let groups = prepare_resize_axis_groups(
        &fixture.workspace,
        &fixture.plan,
        vec![
            grid_handle(&fixture, owner, Axis::Horizontal, 0),
            grid_handle(&fixture, owner, Axis::Vertical, 0),
        ],
    )
    .unwrap();
    let updates = resize_axis_group_updates(&groups, [20.0, 10.0]).unwrap();
    assert_eq!(
        updates.len(),
        1,
        "two axes must never emit duplicate owner updates"
    );
    let projected = fixture.project_updates(&updates);
    for (node, width, height) in [
        (cells[0], 220.0, 110.0),
        (cells[1], 180.0, 110.0),
        (cells[2], 220.0, 90.0),
        (cells[3], 180.0, 90.0),
    ] {
        close(projected.nodes[&node].rect.width(), width);
        close(projected.nodes[&node].rect.height(), height);
    }
    for (node, original) in &fixture.projection.nodes {
        assert_eq!(
            fixture
                .project_updates(&resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap())
                .nodes[node],
            *original,
            "returning to the press geometry must not accumulate drift"
        );
    }
}

#[test]
fn shared_grid_rejects_a_stale_resolved_cell_map_before_any_resize() {
    let (mut fixture, owner, _) = grid_fixture();
    let old = grid_handle(&fixture, owner, Axis::Horizontal, 0);
    let mut replacement = fixture.workspace.node(owner).unwrap().clone();
    let Node::Grid { children, .. } = &mut replacement else {
        unreachable!()
    };
    children.swap(0, 3);
    let mut builder = fixture.workspace.clone().into_builder();
    builder.replace_node(owner, replacement).unwrap();
    fixture.workspace = builder.build().unwrap();
    assert!(prepare_resize_axis_groups(&fixture.workspace, &fixture.plan, vec![old]).is_none());
}

#[test]
fn outward_resize_consumes_nearest_capacity_and_reverses_from_frozen_geometry() {
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let mut builder = Workspace::builder();
        let cells = [1, 2, 3].map(|item| tabs(&mut builder, item));
        let split =
            builder.insert_node(Node::split(Axis::Horizontal, cells, [1.0, 1.0, 1.0]).unwrap());
        let fixture = Fixture::with_propagation(
            finish(builder, split, None),
            rect(304.0, 200.0),
            BTreeMap::from([
                (cells[0], constraints(20.0, 300.0)),
                (cells[1], constraints(80.0, 300.0)),
                (cells[2], constraints(20.0, 300.0)),
            ]),
            2.0,
            propagation,
        );
        let groups = fixture.groups(&[(split, 0)]);
        let updates = resize_axis_group_updates(&groups, [50.0, 0.0]).unwrap();
        let projected = fixture.project_updates(&updates);
        let expected = match propagation {
            ResizePropagation::Adjacent => [120.0, 80.0, 100.0],
            ResizePropagation::Outward => [150.0, 80.0, 70.0],
        };
        for (node, expected) in cells.into_iter().zip(expected) {
            close(projected.nodes[&node].rect.width(), expected);
        }
        let returned =
            fixture.project_updates(&resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap());
        assert_eq!(returned, *fixture.projection);
    }
}

#[test]
fn capped_cross_axis_child_keeps_its_frame_until_the_slot_reaches_the_cap() {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let b = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let across = builder.insert_node(Node::Grid {
        shape: DockspaceGridShape::Fixed {
            rows: std::num::NonZeroUsize::new(2).unwrap(),
            columns: std::num::NonZeroUsize::new(1).unwrap(),
        },
        children: vec![b, c],
        shares: None,
        layout_title: None,
    });
    let outer =
        builder.insert_node(Node::split(Axis::Horizontal, [a, across], [1.0, 1.0]).unwrap());
    let fixture = Fixture::with_mode(
        finish(builder, outer, None),
        rect(1001.0, 401.0),
        BTreeMap::from([
            (a, constraints(100.0, 2000.0)),
            (b, constraints(100.0, 300.0)),
            (c, constraints(100.0, 2000.0)),
        ]),
        1.0,
        ResizePropagation::Adjacent,
        true,
    );
    let groups = fixture.groups(&[(outer, 0)]);
    for (delta, capped_width, peer_width) in [
        (150.0, 300.0, 350.0),
        (250.0, 250.0, 250.0),
        (0.0, 300.0, 500.0),
    ] {
        let projected =
            fixture.project_updates(&resize_axis_group_updates(&groups, [delta, 0.0]).unwrap());
        close(projected.nodes[&b].rect.width(), capped_width);
        close(projected.nodes[&c].rect.width(), peer_width);
    }
}

#[test]
fn a_capped_leaf_keeps_adjacent_queries_on_the_linear_path() {
    let mut builder = Workspace::builder();
    let cells = [1, 2, 3, 4].map(|item| builder.insert_node(Node::pane(ItemId::new(item))));
    let band =
        builder.insert_node(Node::equal_split(Axis::Vertical, cells[..2].iter().copied()).unwrap());
    let root = builder.insert_node(
        Node::split(
            Axis::Horizontal,
            [band, cells[2], cells[3]],
            [100.0, 100.0, 816.0],
        )
        .unwrap(),
    );
    let workspace = finish(builder, root, None);
    for cap in [150.0, 900.0] {
        let fixture = Fixture::with_mode(
            workspace.clone(),
            rect(1024.0, 400.0),
            BTreeMap::from([
                (cells[0], constraints(1.0, cap)),
                (cells[1], constraints(1.0, 1200.0)),
                (cells[2], constraints(1.0, 1200.0)),
                (cells[3], constraints(1.0, 1200.0)),
            ]),
            4.0,
            ResizePropagation::Adjacent,
            true,
        );
        let groups = fixture.groups(&[(root, 0)]);
        assert!(groups[0].piecewise.is_none());
        close(groups[0].common_delta.minimum, -99.0);
        close(groups[0].common_delta.maximum, 99.0);
        let updates = resize_axis_group_updates(&groups, [99.0, 0.0]).unwrap();
        let projected = fixture.project_updates(&updates);
        close(projected.nodes[&cells[0]].rect.width(), cap.min(199.0));
        close(projected.nodes[&cells[1]].rect.width(), 199.0);
        close(projected.nodes[&cells[2]].rect.width(), 1.0);
        close(projected.nodes[&cells[3]].rect.width(), 816.0);
    }
}

#[test]
fn resizing_an_outer_split_preserves_the_far_grid_column_position() {
    let mut builder = Workspace::builder();
    let cells = [1, 2, 3, 4, 5].map(|item| {
        builder.insert_node(Node::Pane {
            item: ItemId::new(item),
        })
    });
    let grid = builder.insert_node(Node::Grid {
        shape: DockspaceGridShape::Fixed {
            rows: std::num::NonZeroUsize::new(2).unwrap(),
            columns: std::num::NonZeroUsize::new(2).unwrap(),
        },
        children: cells[1..].to_vec(),
        shares: None,
        layout_title: None,
    });
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, [cells[0], grid], [1.0, 1.0]).unwrap());
    let fixture = Fixture::new(
        finish(builder, split, None),
        rect(1004.0, 204.0),
        cells
            .into_iter()
            .map(|node| (node, constraints(20.0, 2000.0)))
            .collect(),
        4.0,
    );
    let groups = fixture.groups(&[(split, 0)]);
    assert!(groups[0].piecewise.is_some());
    let projected =
        fixture.project_updates(&resize_axis_group_updates(&groups, [20.0, 0.0]).unwrap());
    for node in [cells[1], cells[3]] {
        close(projected.nodes[&node].rect.width(), 228.0);
    }
    for node in [cells[2], cells[4]] {
        close(projected.nodes[&node].rect.width(), 248.0);
        close(
            projected.nodes[&node].rect.x(),
            fixture.projection.nodes[&node].rect.x(),
        );
    }
}

#[test]
fn zero_grid_motion_preserves_absent_and_latent_user_shares() {
    for shares in [
        None,
        Some(crate::model::DockspaceGridShares::new([1.0], [1.0, 2.0, 3.0, 4.0]).unwrap()),
    ] {
        let (base, owner, cells) = grid_fixture();
        let mut node = base.workspace.node(owner).unwrap().clone();
        let Node::Grid {
            shape,
            shares: current,
            ..
        } = &mut node
        else {
            unreachable!()
        };
        *shape = DockspaceGridShape::Auto;
        *current = shares;
        let mut builder = base.workspace.into_builder();
        builder.replace_node(owner, node).unwrap();
        let fixture = Fixture::new(
            builder.build().unwrap(),
            base.bounds,
            cells
                .into_iter()
                .map(|node| (node, constraints(20.0, 2000.0)))
                .collect(),
            4.0,
        );
        assert_eq!(fixture.projection.grids.get(&owner).unwrap().shape.rows, 2);
        let groups = prepare_resize_axis_groups(
            &fixture.workspace,
            &fixture.plan,
            vec![grid_handle(&fixture, owner, Axis::Horizontal, 0)],
        )
        .unwrap();
        let updates = resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap();
        assert!(
            updates.is_empty(),
            "unchanged Grid intent must not materialize projected default shares"
        );
        assert_eq!(fixture.project_updates(&updates), *fixture.projection);
    }
}

#[test]
fn outward_limits_preserve_subnormal_visible_shares_beside_a_hidden_owner() {
    let mut builder = Workspace::builder();
    let cells = [1, 2, 3].map(|item| tabs(&mut builder, item));
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, cells, [1e-310, 1.0, 1e-310]).unwrap());
    builder.set_hidden_items([ItemId::new(2)].into());
    let fixture = Fixture::with_propagation(
        finish(builder, split, None),
        rect(1001.0, 300.0),
        cells
            .into_iter()
            .map(|node| (node, constraints(0.0, 2000.0)))
            .collect(),
        1.0,
        ResizePropagation::Outward,
    );
    let groups = fixture.groups(&[(split, 0)]);
    for requested in [-10.0, 10.0, 0.0] {
        let updates = resize_axis_group_updates(&groups, [requested, 0.0]).unwrap();
        assert_eq!(
            updates[0].split_weights().unwrap()[1].get().to_bits(),
            1.0_f64.to_bits()
        );
        assert!(
            updates[0]
                .split_weights()
                .unwrap()
                .iter()
                .all(|weight| weight.get() > 0.0)
        );
        let projected = fixture.project_updates(&updates);
        close(projected.nodes[&cells[0]].rect.width(), 500.0 + requested);
        close(projected.nodes[&cells[2]].rect.width(), 500.0 - requested);
    }
}

fn auto_child_fixture(
    axis: Axis,
    cell_minimum_height: f64,
) -> (Fixture, NodeId, NodeId, [NodeId; 4]) {
    let mut builder = Workspace::builder();
    let cells = [1, 2, 3, 4].map(|item| builder.insert_node(Node::pane(ItemId::new(item))));
    let peer = builder.insert_node(Node::pane(ItemId::new(5)));
    let grid = builder.insert_node(Node::Grid {
        shape: DockspaceGridShape::Auto,
        children: cells.to_vec(),
        shares: None,
        layout_title: None,
    });
    let children = match axis {
        Axis::Horizontal => [peer, grid],
        Axis::Vertical => [grid, peer],
    };
    let split = builder.insert_node(Node::equal_split(axis, children).unwrap());
    let bounds = match axis {
        Axis::Horizontal => rect(1004.0, 204.0),
        Axis::Vertical => rect(500.0, 412.0),
    };
    let constraints = cells
        .into_iter()
        .map(|node| {
            (
                node,
                Constraints::new(
                    LogicalSize::new(20.0, cell_minimum_height).unwrap(),
                    LogicalSize::new(2000.0, 2000.0).unwrap(),
                )
                .unwrap(),
            )
        })
        .chain([(peer, constraints(20.0, 2000.0))])
        .collect();
    let fixture = Fixture::new(finish(builder, split, None), bounds, constraints, 4.0);
    let shape = fixture.projection.grids.get(&grid).unwrap().shape;
    assert_eq!((shape.rows, shape.columns), (1, 4));
    (fixture, split, grid, cells)
}

#[test]
fn ancestor_only_auto_reflow_keeps_the_driven_boundary_exact_and_matches_commit() {
    let (fixture, split, grid, _) = auto_child_fixture(Axis::Horizontal, 20.0);
    let groups = fixture.groups(&[(split, 0)]);
    let updates = resize_axis_group_updates(&groups, [100.0, 0.0]).unwrap();
    let preview = fixture.project_updates(&updates);
    close(
        preview.splits.get(&split).unwrap().children[0].extent,
        600.0,
    );
    close(preview.nodes[&grid].slot.width(), 400.0);
    let shape = preview.grids.get(&grid).unwrap().shape;
    assert_eq!((shape.rows, shape.columns), (2, 2));

    let mut workspace = fixture.workspace.clone();
    crate::graph::transaction::WorkspaceTransaction::from_commands([
        crate::graph::command::WorkspaceCommand::ResizeTracks { tracks: updates },
    ])
    .apply(
        &mut workspace,
        &crate::policy::DockPolicy::default().snapshot(PolicyRevision::new(1)),
    )
    .unwrap();
    let committed = project_root(
        &workspace,
        ROOT,
        fixture.bounds,
        |node| fixture.constraints.get(&node).copied(),
        fixture.metrics,
    )
    .unwrap();
    assert_eq!(preview.nodes, committed.nodes);
    assert_eq!(preview.grids, committed.grids);
    assert_eq!(preview.splits, committed.splits);
}

#[test]
fn a_mixed_axis_auto_junction_rejects_shape_changes_without_losing_either_driver() {
    let (fixture, split, grid, _) = auto_child_fixture(Axis::Vertical, 20.0);
    let handles = vec![
        grid_handle(&fixture, grid, Axis::Horizontal, 0),
        fixture.handle(split, 0),
    ];
    let groups = prepare_resize_axis_groups(&fixture.workspace, &fixture.plan, handles).unwrap();
    let valid = resize_axis_group_updates(&groups, [20.0, 10.0]).unwrap();
    let preview = fixture.project_updates(&valid);
    let grid_projection = preview.grids.get(&grid).unwrap();
    assert_eq!(
        (grid_projection.shape.rows, grid_projection.shape.columns),
        (1, 4)
    );
    close(grid_projection.columns.tracks[0].extent, 142.0);
    close(
        preview.splits.get(&split).unwrap().children[0].extent,
        214.0,
    );

    let bypass = [
        TrackResize::split(
            fixture.workspace.capture_node_source(ROOT, split).unwrap(),
            SplitWeight::normalize([304.0, 104.0]).unwrap(),
        ),
        TrackResize::grid(
            fixture.workspace.capture_node_source(ROOT, grid).unwrap(),
            crate::model::DockspaceGridShares::new([1.0], [142.0, 102.0, 122.0, 122.0]).unwrap(),
        ),
    ];
    let invalid = fixture.project_updates(&bypass);
    let changed = invalid.grids.get(&grid).unwrap();
    assert_eq!((changed.shape.rows, changed.shape.columns), (2, 2));
    close(changed.columns.tracks[0].extent, 248.0);
    assert!(resize_axis_group_updates(&groups, [20.0, 100.0]).is_none());
    assert_eq!(
        resize_axis_group_updates(&groups, [20.0, 10.0]).unwrap(),
        valid
    );
    assert_eq!(
        fixture
            .project_updates(&resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap())
            .nodes,
        fixture.projection.nodes
    );
}

#[test]
fn auto_reflow_checks_new_minima_and_the_actual_ancestor_boundary() {
    for (axis, minimum_height) in [(Axis::Horizontal, 120.0), (Axis::Vertical, 170.0)] {
        let (fixture, split, grid, _) = auto_child_fixture(axis, minimum_height);
        let groups = fixture.groups(&[(split, 0)]);
        let updates = groups[0]
            .piecewise
            .as_ref()
            .unwrap()
            .updates(100.0)
            .unwrap();
        let projected = fixture.project_updates(&updates);
        let shape = projected.grids.get(&grid).unwrap().shape;
        assert_eq!((shape.rows, shape.columns), (2, 2));
        let geometry = crate::presentation::layout::measure_resize_geometry(&projected, 4.0);
        match axis {
            Axis::Horizontal => {
                assert_eq!(projected.nodes[&grid].constraints.min().height(), 244.0);
                assert!(!geometry.root_is_feasible(split));
            }
            Axis::Vertical => {
                assert_eq!(projected.nodes[&grid].constraints.min().height(), 344.0);
                assert!(geometry.root_is_feasible(split));
                close(
                    projected.splits.get(&split).unwrap().children[0].extent,
                    344.0,
                );
            }
        }
        assert!(resize_axis_group_updates(&groups, delta_along(axis, 100.0)).is_none());
        assert!(resize_axis_group_updates(&groups, [0.0, 0.0]).is_some());
    }
}

#[test]
fn fixed_zero_visible_tracks_keep_hidden_share_bits_while_reaching_a_minimum() {
    for weights in [[1.0, 7.0, 1.0, 1.0], [1.0e-310, 1.0, 1.0e-310, 1.0e-310]] {
        let mut builder = Workspace::builder();
        let cells = [1, 2, 3, 4].map(|item| builder.insert_node(Node::pane(ItemId::new(item))));
        let split = builder.insert_node(Node::split(Axis::Horizontal, cells, weights).unwrap());
        builder.set_hidden_items([ItemId::new(2)].into());
        let constraints = BTreeMap::from([
            (cells[0], constraints(0.0, 0.0)),
            (cells[1], constraints(1.0, 2000.0)),
            (cells[2], constraints(1.0, 2000.0)),
            (cells[3], constraints(1.0, 2000.0)),
        ]);
        let fixture = Fixture::with_mode(
            finish(builder, split, None),
            rect(604.0, 204.0),
            constraints,
            4.0,
            ResizePropagation::Outward,
            true,
        );
        let original = fixture.handle(split, 2).1.weights()[1].get().to_bits();
        let groups = fixture.groups(&[(split, 2)]);
        let updates = resize_axis_group_updates(&groups, [-297.0, 0.0]).unwrap();
        let weights = updates[0].split_weights().unwrap();
        assert_eq!(weights[1].get().to_bits(), original);
        assert!(weights.iter().all(|weight| weight.get() > 0.0));
        let projected = fixture.project_updates(&updates);
        assert!(!projected.nodes[&cells[1]].visible);
        for (node, expected) in cells.into_iter().zip([0.0, 0.0, 1.0, 595.0]) {
            close(projected.nodes[&node].rect.width(), expected);
        }
    }
}
