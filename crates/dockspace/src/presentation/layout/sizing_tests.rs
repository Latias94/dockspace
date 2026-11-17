use super::*;
use crate::geometry::AxisSizing;
use crate::graph::{RootRecord, SurfacePresentation, WorkspaceBuilder};
use crate::ids::{ItemId, SurfaceId};

const ROOT: RootId = RootId::new(1);

fn pane(width: (f64, Option<f64>), height: (f64, Option<f64>)) -> PaneSizing {
    PaneSizing::new(
        AxisSizing::new(width.0, width.1, None).unwrap(),
        AxisSizing::new(height.0, height.1, None).unwrap(),
    )
}

fn finish(mut builder: WorkspaceBuilder, root: NodeId) -> Workspace {
    builder.set_root(ROOT, RootRecord::new(root));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    builder.build().unwrap()
}

fn project(
    workspace: &Workspace,
    sizing: &BTreeMap<NodeId, PaneSizing>,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
) -> LayoutProjection {
    measure_root_sizing(workspace, ROOT, sizing, metrics)
        .unwrap()
        .project(bounds, &[])
        .unwrap()
}

fn size(axis: Axis, main: f64, cross: f64) -> LogicalSize {
    match axis {
        Axis::Horizontal => LogicalSize::new(main, cross).unwrap(),
        Axis::Vertical => LogicalSize::new(cross, main).unwrap(),
    }
}

fn rect_size(rect: LogicalRect) -> LogicalSize {
    LogicalSize::new(rect.width(), rect.height()).unwrap()
}

#[test]
fn a_single_root_leaf_retains_the_host_slot_and_caps_its_frame() {
    let mut builder = Workspace::builder();
    let leaf = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let workspace = finish(builder, leaf);
    let sizing = BTreeMap::from([(leaf, pane((100.0, Some(200.0)), (120.0, Some(120.0))))]);
    let bounds = LogicalRect::new(11.0, 17.0, 600.0, 400.0).unwrap();
    let projected = project(
        &workspace,
        &sizing,
        bounds,
        LayoutMetrics::new(0.0).unwrap(),
    );
    assert_eq!(projected.nodes[&leaf].slot, bounds);
    assert_eq!(
        projected.nodes[&leaf].rect,
        LogicalRect::new(11.0, 17.0, 200.0, 120.0).unwrap()
    );
    assert_eq!(
        projected.nodes[&leaf].constraints,
        Constraints::new(
            LogicalSize::new(100.0, 120.0).unwrap(),
            LogicalSize::new(200.0, 120.0).unwrap(),
        )
        .unwrap()
    );

    let collapsed = LogicalRect::new(11.0, 17.0, 50.0, 40.0).unwrap();
    for policy in [PaneOverflowPolicy::Clip, PaneOverflowPolicy::Compress] {
        let projected = project(
            &workspace,
            &sizing,
            collapsed,
            LayoutMetrics::new(0.0)
                .unwrap()
                .with_pane_overflow_policy(policy),
        );
        assert_eq!(projected.nodes[&leaf].slot, collapsed);
        assert_eq!(projected.nodes[&leaf].rect, collapsed);
    }
}

#[test]
fn alternating_nested_splits_compose_caps_and_keep_cross_axis_slots() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
        let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
        let c = builder.insert_node(Node::tabs([ItemId::new(3)]));
        let cross_axis = match axis {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
        let inner = builder.insert_node(Node::split(cross_axis, [b, c], [0.5, 0.5]).unwrap());
        let outer = builder.insert_node(Node::split(axis, [a, inner], [0.5, 0.5]).unwrap());
        let workspace = finish(builder, outer);
        let sizing = [(a, 180.0, 250.0), (b, 240.0, 120.0), (c, 160.0, 180.0)]
            .into_iter()
            .map(|(node, main, cross)| {
                let maximum = size(axis, main, cross);
                (
                    node,
                    pane(
                        (10.0, Some(maximum.width())),
                        (10.0, Some(maximum.height())),
                    ),
                )
            })
            .collect();
        let host = size(axis, 600.0, 500.0);
        let bounds = LogicalRect::new(3.0, 7.0, host.width(), host.height()).unwrap();
        let projected = project(
            &workspace,
            &sizing,
            bounds,
            LayoutMetrics::new(10.0).unwrap(),
        );
        assert_eq!(projected.nodes[&outer].slot, bounds);
        assert_eq!(
            rect_size(projected.nodes[&outer].rect),
            size(axis, 430.0, 310.0)
        );
        assert_eq!(
            rect_size(projected.nodes[&a].slot),
            size(axis, 180.0, 310.0)
        );
        assert_eq!(
            rect_size(projected.nodes[&a].rect),
            size(axis, 180.0, 250.0)
        );
        assert_eq!(
            rect_size(projected.nodes[&inner].rect),
            size(axis, 240.0, 310.0)
        );
        assert_eq!(
            rect_size(projected.nodes[&b].rect),
            size(axis, 240.0, 120.0)
        );
        assert_eq!(
            rect_size(projected.nodes[&c].slot),
            size(axis, 240.0, 180.0)
        );
        assert_eq!(
            rect_size(projected.nodes[&c].rect),
            size(axis, 160.0, 180.0)
        );
        assert_eq!(projected.splits.get(&outer).unwrap().unallocated, 170.0);
        assert_eq!(projected.splits.get(&inner).unwrap().unallocated, 0.0);
    }
}

#[test]
fn incompatible_cross_axis_ranges_are_legal_with_independent_frames() {
    let mut builder = Workspace::builder();
    let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let split = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.5, 0.5]).unwrap());
    let workspace = finish(builder, split);
    let sizing = BTreeMap::from([
        (a, pane((100.0, Some(400.0)), (200.0, Some(240.0)))),
        (b, pane((100.0, Some(400.0)), (80.0, Some(120.0)))),
    ]);
    let bounds = LogicalRect::new(0.0, 0.0, 600.0, 220.0).unwrap();
    let projected = project(
        &workspace,
        &sizing,
        bounds,
        LayoutMetrics::new(0.0).unwrap(),
    );
    assert_eq!(projected.nodes[&split].constraints.min().height(), 200.0);
    assert_eq!(projected.nodes[&split].constraints.max().height(), 240.0);
    assert_eq!(projected.nodes[&a].slot.height(), 220.0);
    assert_eq!(projected.nodes[&b].slot.height(), 220.0);
    assert_eq!(projected.nodes[&a].rect.height(), 220.0);
    assert_eq!(projected.nodes[&b].rect.height(), 120.0);
}

#[test]
fn complete_constraints_keep_shared_maxima_while_pane_sizing_caps_independent_frames() {
    let mut builder = Workspace::builder();
    let top = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let bottom = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let right = builder.insert_node(Node::tabs([ItemId::new(3)]));
    let nested = builder.insert_node(Node::equal_split(Axis::Vertical, [top, bottom]).unwrap());
    let outer = builder.insert_node(Node::equal_split(Axis::Horizontal, [nested, right]).unwrap());
    let workspace = finish(builder, outer);
    let constraints = [
        (top, (80.0, 90.0)),
        (bottom, (120.0, 110.0)),
        (right, (150.0, 400.0)),
    ]
    .into_iter()
    .map(|(node, (width, height))| {
        (
            node,
            Constraints::new(
                LogicalSize::new(0.0, 0.0).unwrap(),
                LogicalSize::new(width, height).unwrap(),
            )
            .unwrap(),
        )
    })
    .collect::<BTreeMap<_, _>>();
    let bounds = LogicalRect::new(0.0, 0.0, 500.0, 300.0).unwrap();
    let metrics = LayoutMetrics::new(10.0).unwrap();
    let legacy = project_root(
        &workspace,
        ROOT,
        bounds,
        |node| constraints.get(&node).copied(),
        metrics,
    )
    .unwrap();
    assert_eq!(
        legacy.nodes[&nested].constraints.max(),
        LogicalSize::new(80.0, 210.0).unwrap()
    );
    assert_eq!(
        legacy
            .splits
            .get(&outer)
            .unwrap()
            .children
            .iter()
            .map(|child| child.maximum)
            .collect::<Vec<_>>(),
        [80.0, 150.0]
    );
    assert_eq!(legacy.nodes[&top].rect.width(), 80.0);
    assert_eq!(legacy.nodes[&bottom].rect.width(), 80.0);
    assert_eq!(
        reference::capture(&legacy),
        reference::project(
            &workspace,
            ROOT,
            bounds,
            LeafMeasurements::Constraints(&constraints),
            metrics,
            &[],
        )
        .unwrap()
    );

    let sizing = constraints
        .iter()
        .map(|(&node, constraints)| {
            (
                node,
                pane(
                    (0.0, Some(constraints.max().width())),
                    (0.0, Some(constraints.max().height())),
                ),
            )
        })
        .collect();
    let actual = project(&workspace, &sizing, bounds, metrics);
    assert_eq!(actual.nodes[&outer].slot, bounds);
    assert_eq!(
        actual.nodes[&nested].constraints.max(),
        LogicalSize::new(120.0, 210.0).unwrap()
    );
    assert_eq!(
        actual
            .splits
            .get(&outer)
            .unwrap()
            .children
            .iter()
            .map(|child| child.maximum)
            .collect::<Vec<_>>(),
        [120.0, 150.0]
    );
    assert_eq!(actual.nodes[&top].slot.width(), 120.0);
    assert_eq!(actual.nodes[&top].rect.width(), 80.0);
    assert_eq!(actual.nodes[&bottom].rect.width(), 120.0);
    assert_eq!(actual.nodes[&right].rect.width(), 150.0);
}

#[test]
fn saturated_caps_leave_root_slack_and_preserve_both_overflow_policies() {
    let mut builder = Workspace::builder();
    let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let split = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.5, 0.5]).unwrap());
    let workspace = finish(builder, split);
    let sizing = BTreeMap::from([
        (a, pane((100.0, Some(200.0)), (0.0, None))),
        (b, pane((100.0, Some(250.0)), (0.0, None))),
    ]);
    let bounds = LogicalRect::new(0.0, 0.0, 600.0, 100.0).unwrap();
    let projected = project(
        &workspace,
        &sizing,
        bounds,
        LayoutMetrics::new(0.0).unwrap(),
    );
    let outcome = projected.splits.get(&split).unwrap();
    assert_eq!(projected.nodes[&split].slot, bounds);
    assert_eq!(projected.nodes[&split].rect.width(), 450.0);
    assert_eq!(projected.nodes[&a].rect.width(), 200.0);
    assert_eq!(projected.nodes[&b].rect.width(), 250.0);
    assert_eq!(outcome.unallocated, 150.0);
    assert_eq!(outcome.overflow, 0.0);

    for (policy, expected) in [
        (PaneOverflowPolicy::Clip, 100.0),
        (PaneOverflowPolicy::Compress, 75.0),
    ] {
        let bounds = LogicalRect::new(0.0, 0.0, 150.0, 100.0).unwrap();
        let projected = project(
            &workspace,
            &sizing,
            bounds,
            LayoutMetrics::new(0.0)
                .unwrap()
                .with_pane_overflow_policy(policy),
        );
        assert_eq!(projected.nodes[&split].slot, bounds);
        assert_eq!(projected.nodes[&split].rect.width(), 150.0);
        assert_eq!(projected.nodes[&a].rect.width(), expected);
        assert_eq!(projected.nodes[&b].rect.width(), expected);
        assert_eq!(projected.nodes[&a].constraints.min().width(), 100.0);
        assert_eq!(projected.nodes[&b].constraints.min().width(), 100.0);
        assert_eq!(projected.splits.get(&split).unwrap().overflow, 50.0);
        assert_eq!(projected.splits.get(&split).unwrap().unallocated, 0.0);
    }
}

#[test]
fn hidden_panes_release_their_caps_and_restore_the_original_projection() {
    let mut builder = Workspace::builder();
    let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let split = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.5, 0.5]).unwrap());
    let mut workspace = finish(builder, split);
    let sizing = BTreeMap::from([
        (a, pane((100.0, Some(200.0)), (0.0, None))),
        (b, pane((100.0, Some(250.0)), (0.0, None))),
    ]);
    let bounds = LogicalRect::new(0.0, 0.0, 600.0, 100.0).unwrap();
    let metrics = LayoutMetrics::new(0.0).unwrap();
    let original = project(&workspace, &sizing, bounds, metrics);
    workspace
        .replace_hidden_items([ItemId::new(2)].into())
        .unwrap();
    let hidden = project(&workspace, &sizing, bounds, metrics);
    assert_eq!(hidden.nodes[&split].slot, bounds);
    assert_eq!(hidden.nodes[&split].rect.width(), 200.0);
    assert_eq!(hidden.nodes[&a].rect.width(), 200.0);
    assert!(!hidden.nodes[&b].visible);
    assert_eq!(hidden.nodes[&b].slot.width(), 0.0);
    assert_eq!(hidden.nodes[&b].rect.width(), 0.0);
    assert_eq!(hidden.splits.get(&split).unwrap().unallocated, 400.0);
    workspace
        .replace_hidden_items([ItemId::new(1), ItemId::new(2)].into())
        .unwrap();
    let empty = project(&workspace, &sizing, bounds, metrics);
    assert_eq!(empty.nodes[&split].slot, bounds);
    assert_eq!(
        rect_size(empty.nodes[&split].rect),
        size(Axis::Horizontal, 0.0, 0.0)
    );
    assert_eq!(empty.splits.get(&split).unwrap().unallocated, 600.0);
    for node in [a, b, split] {
        assert!(!empty.nodes[&node].visible);
    }
    workspace.replace_hidden_items(Default::default()).unwrap();
    assert_eq!(project(&workspace, &sizing, bounds, metrics), original);
}

#[test]
fn unbounded_sizing_matches_the_minimum_only_primitive_and_resize_overrides() {
    for central in [false, true] {
        let mut builder = Workspace::builder();
        let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
        let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
        let c = builder.insert_node(Node::tabs([ItemId::new(3)]));
        let inner = builder.insert_node(Node::split(Axis::Vertical, [b, c], [0.7, 0.3]).unwrap());
        let outer =
            builder.insert_node(Node::split(Axis::Horizontal, [a, inner], [0.2, 0.8]).unwrap());
        let record = if central {
            RootRecord::new(outer).with_central(b)
        } else {
            RootRecord::new(outer)
        };
        builder.set_root(ROOT, record);
        builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
        let workspace = builder.build().unwrap();
        let minimums = BTreeMap::from([
            (a, LogicalSize::new(17.0, 11.0).unwrap()),
            (b, LogicalSize::new(31.0, 7.0).unwrap()),
            (c, LogicalSize::new(13.0, 19.0).unwrap()),
        ]);
        let sizing = minimums
            .iter()
            .map(|(&node, &minimum)| (node, PaneSizing::from_minimum(minimum)))
            .collect();
        let source = workspace.capture_node_source(ROOT, inner).unwrap();
        let weights = TrackWeights::Split(SplitWeight::normalize([0.4, 0.6]).unwrap());
        let overrides = [TrackWeightOverride::new(&source, &weights)];
        for policy in [PaneOverflowPolicy::Clip, PaneOverflowPolicy::Compress] {
            let metrics = LayoutMetrics::new(3.0)
                .unwrap()
                .with_pane_overflow_policy(policy);
            for (width, height) in [(300.0, 200.0), (51.0, 29.0), (41.0, 23.0)] {
                let bounds = LogicalRect::new(5.0, 7.0, width, height).unwrap();
                for overrides in [&[][..], &overrides[..]] {
                    let actual = measure_root_sizing(&workspace, ROOT, &sizing, metrics)
                        .unwrap()
                        .project(bounds, overrides)
                        .unwrap();
                    let expected = measure_root_minimums(&workspace, ROOT, &minimums, metrics)
                        .unwrap()
                        .project(bounds, overrides)
                        .unwrap();
                    assert_eq!(actual, expected);
                    assert_eq!(
                        actual.nodes[&outer].constraints.max(),
                        LogicalSize::new(2.0 * width.max(51.0) + 3.0, height.max(29.0)).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn preferred_sizes_do_not_override_persistent_split_weights() {
    let mut builder = Workspace::builder();
    let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let split = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.25, 0.75]).unwrap());
    let workspace = finish(builder, split);
    let sizing = BTreeMap::from([
        (
            a,
            PaneSizing::new(
                AxisSizing::new(0.0, None, Some(500.0)).unwrap(),
                AxisSizing::default(),
            ),
        ),
        (
            b,
            PaneSizing::new(
                AxisSizing::new(0.0, None, Some(10.0)).unwrap(),
                AxisSizing::default(),
            ),
        ),
    ]);
    let projected = project(
        &workspace,
        &sizing,
        LogicalRect::new(0.0, 0.0, 600.0, 100.0).unwrap(),
        LayoutMetrics::new(0.0).unwrap(),
    );
    assert_eq!(projected.nodes[&a].rect.width(), 150.0);
    assert_eq!(projected.nodes[&b].rect.width(), 450.0);
}
