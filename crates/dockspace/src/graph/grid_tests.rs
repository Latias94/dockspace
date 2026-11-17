use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use crate::close::plan::{CloseItemRequirement, ContentCloseScope, prepare_scoped_content_close};
use crate::graph::command::{
    CloseCommitOutcome, CommandOutcome, DockTarget, GridResizeBasis, MovePayload, TrackResize,
    WorkspaceCommand,
};
use crate::graph::error::{CommandError, TransactionError};
use crate::graph::root_structure::RootNodeKind;
use crate::graph::transaction::WorkspaceTransaction;
use crate::graph::{Node, Workspace};
use crate::ids::{ItemId, RootId, SurfaceId, TabKey};
use crate::model::{
    DockspaceAxis, DockspaceGridError, DockspaceGridShape, DockspaceGridShares, DockspaceLayout,
    DockspaceLayoutError, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
};
#[cfg(feature = "serde")]
use crate::persistence::snapshot::{
    SnapshotGridShape, SnapshotNode, SnapshotRestoreError, WorkspaceSnapshot,
};
use crate::policy::{DockPolicy, DockPolicySnapshot};

const ROOT: RootId = RootId::new(1);
const TARGET: RootId = RootId::new(2);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);

fn fixed(rows: usize, columns: usize) -> DockspaceGridShape {
    DockspaceGridShape::Fixed {
        rows: NonZeroUsize::new(rows).unwrap(),
        columns: NonZeroUsize::new(columns).unwrap(),
    }
}

fn layout(content: DockspaceNode) -> DockspaceLayout {
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SurfaceId::new(1),
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap()
}

fn four_cells() -> Workspace {
    layout(
        DockspaceNode::grid(fixed(2, 2), [A, B, C, D].map(DockspaceNode::pane))
            .unwrap()
            .named("Tools"),
    )
    .into_workspace()
}

#[test]
fn fixed_grid_owns_real_bare_children_and_preserves_hidden_slots_and_title() {
    let declared = layout(
        DockspaceNode::grid(fixed(2, 2), [A, B, C, D].map(DockspaceNode::pane))
            .unwrap()
            .named("Tools"),
    )
    .with_hidden_items([B])
    .unwrap();
    let root = declared.view().root(ROOT).unwrap();
    let content = root.content().unwrap();
    let grid = content.grid().unwrap();
    assert_eq!(grid.shape(), fixed(2, 2));
    assert!(grid.shares().is_none());
    assert_eq!(content.layout_title(), Some("Tools"));
    assert_eq!(root.items().collect::<Vec<_>>(), [A, B, C, D]);
    assert_eq!(root.active_items().collect::<Vec<_>>(), [A, C, D]);
    assert_eq!(
        grid.children()
            .map(|node| node.item().unwrap())
            .collect::<Vec<_>>(),
        [A, B, C, D]
    );
    for item in [A, B, C, D] {
        assert!(declared.view().item(item).unwrap().tabs().is_none());
    }
    let workspace = declared.into_workspace();
    assert_eq!(workspace.nodes().count(), 5);
    let structure = workspace.root_structure(ROOT).unwrap();
    assert!(structure.has_composition());
    assert_eq!(structure.grid_rows.len(), 1);
    let RootNodeKind::Grid {
        payload, children, ..
    } = &structure.nodes[0].kind
    else {
        unreachable!()
    };
    assert_eq!(payload.shape(), fixed(2, 2));
    assert_eq!(children.len(), 4);
    assert!(
        payload.shares_payload(
            workspace
                .storage()
                .nodes
                .shared(structure.nodes[0].id)
                .unwrap()
        )
    );
    workspace.validate().unwrap();
}

#[test]
fn explicit_singleton_and_sparse_tail_grids_remain_real_containers() {
    for (shape, items) in [
        (fixed(1, 1), vec![A]),
        (fixed(2, 2), vec![A, B, C]),
        (DockspaceGridShape::Auto, vec![A]),
    ] {
        let declared = layout(
            DockspaceNode::grid(shape, items.iter().copied().map(DockspaceNode::pane)).unwrap(),
        );
        let cloned = declared.view().to_layout();
        assert_eq!(
            cloned
                .view()
                .root(ROOT)
                .unwrap()
                .content()
                .unwrap()
                .grid()
                .unwrap()
                .shape(),
            shape
        );
        assert_eq!(
            declared
                .view()
                .root(ROOT)
                .unwrap()
                .items()
                .collect::<Vec<_>>(),
            items
        );
        assert_eq!(declared.into_workspace().nodes().count(), items.len() + 1);
    }
}

#[test]
fn declarations_reject_empty_capacity_empty_tracks_and_mismatched_shares() {
    assert!(matches!(
        DockspaceNode::grid(DockspaceGridShape::Auto, []),
        Err(DockspaceLayoutError::InvalidGrid(
            DockspaceGridError::EmptyChildren
        ))
    ));
    assert!(matches!(
        DockspaceNode::grid(fixed(1, 1), [A, B].map(DockspaceNode::pane)),
        Err(DockspaceLayoutError::InvalidGrid(
            DockspaceGridError::TooManyChildren {
                children: 2,
                capacity: 1
            }
        ))
    ));
    assert!(matches!(
        DockspaceNode::grid(fixed(2, 3), [A, B, C].map(DockspaceNode::pane)),
        Err(DockspaceLayoutError::InvalidGrid(
            DockspaceGridError::UnoccupiedTrack {
                axis: DockspaceAxis::Vertical,
                index: 1
            }
        ))
    ));
    assert!(matches!(
        DockspaceNode::grid(fixed(usize::MAX, 2), [DockspaceNode::pane(A)]),
        Err(DockspaceLayoutError::InvalidGrid(
            DockspaceGridError::CapacityOverflow { .. }
        ))
    ));
    assert!(matches!(
        DockspaceNode::grid_with_shares(
            fixed(2, 2),
            [A, B, C, D].map(DockspaceNode::pane),
            DockspaceGridShares::new([1.0], [1.0; 4]).unwrap()
        ),
        Err(DockspaceLayoutError::InvalidGrid(
            DockspaceGridError::ShareShapeMismatch { .. }
        ))
    ));
}

#[test]
fn empty_track_batches_preserve_graph_and_owner_authority_even_when_arrangement_is_locked() {
    for mut workspace in [Workspace::new(), four_cells()] {
        let before = workspace.clone();
        let structures = workspace
            .roots()
            .map(|(root, _)| {
                (
                    root,
                    workspace.root_reference(root).unwrap().clone(),
                    std::sync::Arc::clone(workspace.root_structure(root).unwrap()),
                )
            })
            .collect::<Vec<_>>();
        for locked in [false, true] {
            let mut policy = DockPolicy::default();
            policy.set_arrangement_locked(locked);
            let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
                tracks: Vec::new(),
            }])
            .apply(&mut workspace, &policy.snapshot(Default::default()))
            .unwrap();
            assert!(!report.changed());
            assert_eq!(
                report.outcomes(),
                [CommandOutcome::TracksResized {
                    tracks: Vec::new(),
                    changed: false
                }]
            );
            assert_eq!(workspace, before);
            for (root, reference, structure) in &structures {
                assert_eq!(workspace.root_reference(*root), Some(reference));
                assert!(std::sync::Arc::ptr_eq(
                    workspace.root_structure(*root).unwrap(),
                    structure
                ));
            }
        }
    }
}

#[test]
fn an_omitted_auto_grid_update_preserves_latent_dimensions_and_every_share_bit() {
    let shares = DockspaceGridShares::from_normalized(
        vec![crate::graph::SplitWeight::new(1.0).unwrap()],
        [0.1, 0.2, 0.3, f64::from_bits(0.4_f64.to_bits() + 1)]
            .map(|value| crate::graph::SplitWeight::new(value).unwrap())
            .into(),
    )
    .unwrap();
    let mut workspace = layout(
        DockspaceNode::grid_with_shares(
            DockspaceGridShape::Auto,
            [A, B, C, D].map(DockspaceNode::pane),
            shares.clone(),
        )
        .unwrap(),
    )
    .into_workspace();
    let grid = workspace.root(ROOT).unwrap().node;
    let before = workspace.node(grid).unwrap().clone();
    let fingerprint = workspace.root_fingerprint(ROOT).unwrap();
    let reference = workspace.root_reference(ROOT).unwrap().clone();
    let structure = std::sync::Arc::clone(workspace.root_structure(ROOT).unwrap());
    let source = workspace.capture_node_source(ROOT, grid).unwrap();
    // A viewport may resolve this Auto grid as 2x2 while its stored axes remain 1x4.
    // An input with no actual movement must omit that effective replacement.
    let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: Vec::new(),
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    assert!(!report.changed());
    assert_eq!(
        report.outcomes(),
        [CommandOutcome::TracksResized {
            tracks: Vec::new(),
            changed: false
        }]
    );
    assert_eq!(workspace.node(grid), Some(&before));
    assert_eq!(workspace.root_fingerprint(ROOT).unwrap(), fingerprint);
    assert_eq!(workspace.root_reference(ROOT), Some(&reference));
    assert!(std::sync::Arc::ptr_eq(
        workspace.root_structure(ROOT).unwrap(),
        &structure
    ));
    let Some(Node::Grid {
        shares: Some(actual),
        ..
    }) = workspace.node(grid)
    else {
        unreachable!()
    };
    assert_eq!(actual, &shares);
    assert_eq!(actual.rows().len(), 1);
    assert_eq!(actual.columns().len(), 4);
    // The original exact capability remains usable after the omitted update.
    let repeated = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(source, shares)],
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    assert!(!repeated.changed());
    assert_eq!(workspace.root_reference(ROOT), Some(&reference));
}

#[test]
fn default_grid_equal_track_batches_are_noops_but_one_bit_nonuniform_changes_are_real() {
    for shape in [fixed(2, 2), DockspaceGridShape::Auto] {
        let mut workspace =
            layout(DockspaceNode::grid(shape, [A, B, C, D].map(DockspaceNode::pane)).unwrap())
                .into_workspace();
        let grid = workspace.root(ROOT).unwrap().node;
        let before = workspace.clone();
        let reference = workspace.root_reference(ROOT).unwrap().clone();
        let source = workspace.capture_node_source(ROOT, grid).unwrap();
        let equal = DockspaceGridShares::new([1.0, 1.0], [1.0, 1.0]).unwrap();
        let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![TrackResize::grid(source.clone(), equal)],
        }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap();
        assert_eq!(
            report.outcomes(),
            [CommandOutcome::TracksResized {
                tracks: vec![grid],
                changed: false
            }]
        );
        assert_eq!(workspace, before);
        assert_eq!(workspace.root_reference(ROOT), Some(&reference));
        assert!(matches!(
            workspace.node(grid),
            Some(Node::Grid { shares: None, .. })
        ));

        let mut policy = DockPolicy::default();
        policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Vertical, false);
        let tiny = DockspaceGridShares::from_normalized(
            crate::graph::SplitWeight::normalize([1.0, 1.0]).unwrap(),
            [
                f64::from_bits(0.5_f64.to_bits() + 1),
                f64::from_bits(0.5_f64.to_bits() - 1),
            ]
            .map(|value| crate::graph::SplitWeight::new(value).unwrap())
            .into(),
        )
        .unwrap();
        let bits = tiny.columns().map(f64::to_bits).collect::<Vec<_>>();
        let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![TrackResize::grid(source, tiny)],
        }])
        .apply(&mut workspace, &policy.snapshot(Default::default()))
        .unwrap();
        assert!(report.changed());
        let Some(Node::Grid {
            shares: Some(actual),
            ..
        }) = workspace.node(grid)
        else {
            unreachable!()
        };
        assert_eq!(actual.columns().map(f64::to_bits).collect::<Vec<_>>(), bits);
    }
}

#[test]
fn grid_track_updates_authorize_only_changed_axes_and_keep_joint_rejection_atomic() {
    let mut workspace = four_cells();
    let grid = workspace.root(ROOT).unwrap().node;
    let mut policy = DockPolicy::default();
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Vertical, false);
    let policy = policy.snapshot(Default::default());
    let source = workspace.capture_node_source(ROOT, grid).unwrap();
    let column = DockspaceGridShares::new([1.0, 1.0], [3.0, 2.0]).unwrap();
    let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(source, column.clone())],
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert_eq!(
        report.outcomes(),
        [CommandOutcome::TracksResized {
            tracks: vec![grid],
            changed: true
        }]
    );
    let before = workspace.clone();
    let source = workspace.capture_node_source(ROOT, grid).unwrap();
    let both = DockspaceGridShares::new([3.0, 2.0], [4.0, 1.0]).unwrap();
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(source.clone(), both)],
    }])
    .apply(&mut workspace, &policy)
    .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Command {
            source: CommandError::Policy(crate::policy::PolicyRejection::RootResizeAxisDisabled {
                root: ROOT,
                axis: crate::graph::Axis::Vertical
            }),
            ..
        }
    ));
    assert_eq!(workspace, before);
    let noop = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(source.clone(), column.clone())],
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert!(!noop.changed());
    let duplicate = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![
            TrackResize::grid(source.clone(), column.clone()),
            TrackResize::grid(source, column),
        ],
    }])
    .apply(&mut workspace, &policy)
    .unwrap_err();
    assert!(
        matches!(duplicate, TransactionError::Command { source: CommandError::DuplicateTrackResize { node }, .. } if node == grid)
    );
    assert_eq!(workspace, before);
}

#[test]
fn resolved_auto_grid_resizes_compare_both_axes_with_the_effective_shape() {
    for (latent, dimensions, columns) in [
        (
            DockspaceGridShares::new([1.0], [1.0, 2.0, 3.0, 4.0]).unwrap(),
            (2, 2),
            vec![11.0, 9.0],
        ),
        (
            DockspaceGridShares::new([3.0, 1.0], [1.0, 3.0]).unwrap(),
            (2, 3),
            vec![2.0, 3.0, 5.0],
        ),
    ] {
        let mut workspace = layout(
            DockspaceNode::grid_with_shares(
                DockspaceGridShape::Auto,
                [A, B, C, D].map(DockspaceNode::pane),
                latent,
            )
            .unwrap(),
        )
        .into_workspace();
        let grid = workspace.root(ROOT).unwrap().node;
        let Some(Node::Grid { children, .. }) = workspace.node(grid) else {
            unreachable!()
        };
        let basis =
            GridResizeBasis::new(dimensions.0, dimensions.1, children.clone().into()).unwrap();
        let source = workspace.capture_node_source(ROOT, grid).unwrap();
        let proposed = DockspaceGridShares::new([1.0, 1.0], columns).unwrap();
        let before = workspace.clone();
        let mut policy = DockPolicy::default();
        policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Vertical, false);
        let policy = policy.snapshot(Default::default());

        let raw = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![TrackResize::grid(source.clone(), proposed.clone())],
        }])
        .apply(&mut workspace, &policy)
        .unwrap_err();
        assert!(matches!(
            raw,
            TransactionError::Command {
                source: CommandError::Policy(
                    crate::policy::PolicyRejection::RootResizeAxisDisabled {
                        root: ROOT,
                        axis: crate::graph::Axis::Vertical
                    }
                ),
                ..
            }
        ));
        assert_eq!(workspace, before);

        let both = DockspaceGridShares::from_normalized(
            crate::graph::SplitWeight::normalize([3.0, 2.0]).unwrap(),
            proposed.column_weights().to_vec(),
        )
        .unwrap();
        let denied = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![TrackResize::grid_resolved(
                source.clone(),
                basis.clone(),
                both,
            )],
        }])
        .apply(&mut workspace, &policy)
        .unwrap_err();
        assert!(matches!(
            denied,
            TransactionError::Command {
                source: CommandError::Policy(
                    crate::policy::PolicyRejection::RootResizeAxisDisabled {
                        root: ROOT,
                        axis: crate::graph::Axis::Vertical
                    }
                ),
                ..
            }
        ));
        assert_eq!(workspace, before);

        let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![TrackResize::grid_resolved(source, basis, proposed.clone())],
        }])
        .apply(&mut workspace, &policy)
        .unwrap();
        assert_eq!(
            report.outcomes(),
            [CommandOutcome::TracksResized {
                tracks: vec![grid],
                changed: true
            }]
        );
        let Some(Node::Grid {
            shares: Some(actual),
            ..
        }) = workspace.node(grid)
        else {
            unreachable!()
        };
        assert_eq!(actual, &proposed);
        assert_eq!(workspace.collect_items_in_subtree(grid), [A, B, C, D]);
        workspace.validate().unwrap();
    }
}

#[test]
fn resolved_auto_grid_equal_updates_preserve_latent_share_bits_and_source_authority() {
    let latent = DockspaceGridShares::from_normalized(
        crate::graph::SplitWeight::normalize([1.0]).unwrap(),
        [0.1, 0.2, 0.3, f64::from_bits(0.4_f64.to_bits() + 1)]
            .map(|value| crate::graph::SplitWeight::new(value).unwrap())
            .into(),
    )
    .unwrap();
    let mut workspace = layout(
        DockspaceNode::grid_with_shares(
            DockspaceGridShape::Auto,
            [A, B, C, D].map(DockspaceNode::pane),
            latent,
        )
        .unwrap(),
    )
    .into_workspace();
    let grid = workspace.root(ROOT).unwrap().node;
    let Some(Node::Grid { children, .. }) = workspace.node(grid) else {
        unreachable!()
    };
    let basis = GridResizeBasis::new(2, 2, children.clone().into()).unwrap();
    let source = workspace.capture_node_source(ROOT, grid).unwrap();
    let before = workspace.node(grid).unwrap().clone();
    let fingerprint = workspace.root_fingerprint(ROOT).unwrap();
    let reference = workspace.root_reference(ROOT).unwrap().clone();
    let structure = std::sync::Arc::clone(workspace.root_structure(ROOT).unwrap());
    let mut policy = DockPolicy::default();
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Horizontal, false);
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Vertical, false);
    let report = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid_resolved(
            source.clone(),
            basis.clone(),
            DockspaceGridShares::new([1.0, 1.0], [1.0, 1.0]).unwrap(),
        )],
    }])
    .apply(&mut workspace, &policy.snapshot(Default::default()))
    .unwrap();
    assert_eq!(
        report.outcomes(),
        [CommandOutcome::TracksResized {
            tracks: vec![grid],
            changed: false
        }]
    );
    assert_eq!(workspace.node(grid), Some(&before));
    assert_eq!(workspace.root_fingerprint(ROOT).unwrap(), fingerprint);
    assert_eq!(workspace.root_reference(ROOT), Some(&reference));
    assert!(std::sync::Arc::ptr_eq(
        workspace.root_structure(ROOT).unwrap(),
        &structure
    ));

    let tiny = DockspaceGridShares::from_normalized(
        crate::graph::SplitWeight::normalize([1.0, 1.0]).unwrap(),
        [
            f64::from_bits(0.5_f64.to_bits() + 1),
            f64::from_bits(0.5_f64.to_bits() - 1),
        ]
        .map(|value| crate::graph::SplitWeight::new(value).unwrap())
        .into(),
    )
    .unwrap();
    let columns = tiny.columns().map(f64::to_bits).collect::<Vec<_>>();
    let mut policy = DockPolicy::default();
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Vertical, false);
    let moved = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid_resolved(source, basis, tiny)],
    }])
    .apply(&mut workspace, &policy.snapshot(Default::default()))
    .unwrap();
    assert!(moved.changed());
    let Some(Node::Grid {
        shares: Some(actual),
        ..
    }) = workspace.node(grid)
    else {
        unreachable!()
    };
    assert_eq!(
        actual.columns().map(f64::to_bits).collect::<Vec<_>>(),
        columns
    );
}

#[test]
fn resolved_grid_resize_bases_reject_wrong_cell_maps_and_axis_cardinalities_without_mutation() {
    for shape in [DockspaceGridShape::Auto, fixed(2, 2)] {
        let mut workspace =
            layout(DockspaceNode::grid(shape, [A, B, C, D].map(DockspaceNode::pane)).unwrap())
                .into_workspace();
        let grid = workspace.root(ROOT).unwrap().node;
        let Some(Node::Grid { children, .. }) = workspace.node(grid) else {
            unreachable!()
        };
        let children = children.clone();
        let before = workspace.clone();
        let source = workspace.capture_node_source(ROOT, grid).unwrap();
        let proposed = DockspaceGridShares::new([1.0, 1.0], [3.0, 2.0]).unwrap();
        let mut reordered = children.clone();
        reordered.swap(0, 1);
        let wrong_map = TrackResize::grid_resolved(
            source.clone(),
            GridResizeBasis::new(2, 2, reordered.into()).unwrap(),
            proposed.clone(),
        );
        let valid = TrackResize::grid_resolved(
            source.clone(),
            GridResizeBasis::new(2, 2, children.clone().into()).unwrap(),
            proposed.clone(),
        );
        assert_ne!(wrong_map, valid);
        let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![wrong_map],
        }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap_err();
        assert!(matches!(
            error,
            TransactionError::Command {
                source: CommandError::GridResizeBasisMismatch { node },
                ..
            } if node == grid
        ));
        assert_eq!(workspace, before);

        let wrong_dimensions = TrackResize::grid_resolved(
            source,
            GridResizeBasis::new(1, 4, children.into()).unwrap(),
            proposed,
        );
        let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![wrong_dimensions],
        }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap_err();
        assert!(matches!(
            error,
            TransactionError::Command {
                source: CommandError::InvalidGrid {
                    source: DockspaceGridError::ShareShapeMismatch { .. },
                    ..
                },
                ..
            }
        ));
        assert_eq!(workspace, before);
    }
}

#[test]
fn an_invalid_resolved_grid_basis_rejects_every_owner_in_the_resize_batch() {
    let grid = |items: [ItemId; 2]| {
        DockspaceNode::grid(fixed(1, 2), items.map(DockspaceNode::pane)).unwrap()
    };
    let mut workspace = layout(
        DockspaceNode::equal_split(DockspaceAxis::Horizontal, [grid([A, B]), grid([C, D])])
            .unwrap(),
    )
    .into_workspace();
    let root = workspace.root(ROOT).unwrap().node;
    let Some(Node::Split { children, .. }) = workspace.node(root) else {
        unreachable!()
    };
    let first = children[0];
    let second = children[1];
    let Some(Node::Grid { children, .. }) = workspace.node(second) else {
        unreachable!()
    };
    let mut wrong_map = children.clone();
    wrong_map.swap(0, 1);
    let proposed = DockspaceGridShares::new([1.0], [3.0, 2.0]).unwrap();
    let tracks = vec![
        TrackResize::grid(
            workspace.capture_node_source(ROOT, first).unwrap(),
            proposed.clone(),
        ),
        TrackResize::grid_resolved(
            workspace.capture_node_source(ROOT, second).unwrap(),
            GridResizeBasis::new(1, 2, wrong_map.into()).unwrap(),
            proposed,
        ),
    ];
    let before = workspace.clone();
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks { tracks }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Command {
            source: CommandError::GridResizeBasisMismatch { node },
            ..
        } if node == second
    ));
    assert_eq!(workspace, before);
}

#[test]
fn grid_shares_change_exact_fingerprints_but_not_viewport_independent_topology() {
    let mut workspace = four_cells();
    let grid = workspace.root(ROOT).unwrap().node;
    let before = workspace.root_fingerprint(ROOT).unwrap();
    let source = workspace.capture_node_source(ROOT, grid).unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(
            source.clone(),
            DockspaceGridShares::new([1.0, 3.0], [2.0, 3.0]).unwrap(),
        )],
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    let after = workspace.root_fingerprint(ROOT).unwrap();
    assert_ne!(before, after);
    assert!(!before.presentation_structure_eq(&after));
    assert!(before.presentation_topology_eq(&after));
    let committed = workspace.clone();
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(
            source,
            DockspaceGridShares::new([1.0; 2], [1.0; 2]).unwrap(),
        )],
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Command {
            source: CommandError::StaleNode { .. },
            ..
        }
    ));
    assert_eq!(workspace, committed);
}

#[test]
fn invalid_track_owner_and_fixed_cardinality_reject_the_complete_batch_before_mutation() {
    let mut builder = four_cells().into_builder();
    let target = builder.insert_node(Node::tabs([ItemId::new(5)]));
    builder.set_root(TARGET, crate::graph::RootRecord::new(target));
    builder.set_surface(
        SurfaceId::new(2),
        crate::graph::SurfacePresentation::with_main(TARGET),
    );
    let mut workspace = builder.build().unwrap();
    let before = workspace.clone();
    let grid = workspace.root(ROOT).unwrap().node;
    let grid_source = workspace.capture_node_source(ROOT, grid).unwrap();
    let target_source = workspace.capture_node_source(TARGET, target).unwrap();
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![
            TrackResize::grid(
                grid_source.clone(),
                DockspaceGridShares::new([1.0, 1.0], [2.0, 3.0]).unwrap(),
            ),
            TrackResize::grid(
                target_source,
                DockspaceGridShares::new([1.0], [1.0]).unwrap(),
            ),
        ],
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap_err();
    assert!(
        matches!(error, TransactionError::Command { source: CommandError::TrackOwnerKindMismatch { node }, .. } if node == target)
    );
    assert_eq!(workspace, before);
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::grid(
            grid_source,
            DockspaceGridShares::new([1.0], [1.0; 4]).unwrap(),
        )],
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Command {
            source: CommandError::InvalidGrid {
                source: DockspaceGridError::ShareShapeMismatch { .. },
                ..
            },
            ..
        }
    ));
    assert_eq!(workspace, before);
}

#[test]
fn auto_grid_retains_legal_stored_dimensions_then_drops_them_when_the_roster_no_longer_fits() {
    let declared = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SurfaceId::new(1),
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::grid_with_shares(
                    DockspaceGridShape::Auto,
                    [A, B, C, D].map(DockspaceNode::pane),
                    DockspaceGridShares::new([1.0, 2.0], [2.0, 3.0]).unwrap(),
                )
                .unwrap(),
            ),
        ),
        DockspaceSurfaceLayout::new(
            SurfaceId::new(2),
            DockspaceRootLayout::new(TARGET, DockspaceNode::tabs([ItemId::new(5)])),
        ),
    ])
    .unwrap();
    let mut workspace = declared.into_workspace();
    let grid = workspace.root(ROOT).unwrap().node;
    for (item, keep_shares) in [(C, true), (D, false)] {
        let source = workspace.capture_item_source_by_id(item).unwrap().unwrap();
        let target = workspace
            .capture_tab_target(TARGET, workspace.root(TARGET).unwrap().node)
            .unwrap();
        WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
            payload: MovePayload::Item(source),
            target: DockTarget::Center(target),
        }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap();
        let Node::Grid { shape, shares, .. } = workspace.node(grid).unwrap() else {
            unreachable!()
        };
        assert_eq!(*shape, DockspaceGridShape::Auto);
        assert_eq!(shares.is_some(), keep_shares);
        workspace.validate().unwrap();
    }
    assert_eq!(workspace.collect_items_in_subtree(grid), [A, B]);
}

#[test]
fn moving_a_grid_to_center_preserves_one_layout_tab_and_every_real_child() {
    let mut declared = four_cells().into_builder();
    let target = declared.insert_node(Node::tabs([ItemId::new(5)]));
    declared.set_root(TARGET, crate::graph::RootRecord::new(target));
    declared.set_surface(
        SurfaceId::new(2),
        crate::graph::SurfacePresentation::with_main(TARGET),
    );
    let mut workspace = declared.build().unwrap();
    let grid = workspace.root(ROOT).unwrap().node;
    let before = workspace.item_multiset();
    let source = workspace.capture_node_source(ROOT, grid).unwrap();
    let target_ref = workspace.capture_tab_target(TARGET, target).unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
        payload: MovePayload::Subtree(source),
        target: DockTarget::Center(target_ref),
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    assert_eq!(workspace.item_multiset(), before);
    assert_eq!(workspace.node_root(grid), Some(TARGET));
    assert!(
        matches!(workspace.node(target), Some(Node::Tabs { items, selected, .. }) if items == &[TabKey::for_item(ItemId::new(5)), TabKey::for_layout(grid)] && *selected == Some(TabKey::for_layout(grid)))
    );
    assert_eq!(
        workspace.tab_label(target, TabKey::for_layout(grid)),
        Some(crate::graph::TabLabel::Layout { title: "Tools" })
    );
    assert!(
        matches!(workspace.node(grid), Some(Node::Grid { children, .. }) if children.len() == 4)
    );
    workspace.validate().unwrap();
}

#[test]
fn removing_a_fixed_tail_compacts_children_and_preserves_surviving_track_ratios() {
    let column_weights = vec![
        crate::graph::SplitWeight::new(0.2).unwrap(),
        crate::graph::SplitWeight::new(f64::from_bits(0.8_f64.to_bits() + 1)).unwrap(),
    ];
    let column_bits = column_weights
        .iter()
        .map(|weight| weight.get().to_bits())
        .collect::<Vec<_>>();
    let shares = DockspaceGridShares::from_normalized(
        crate::graph::SplitWeight::normalize([2.0, 3.0]).unwrap(),
        column_weights,
    )
    .unwrap();
    let source =
        DockspaceNode::grid_with_shares(fixed(2, 2), [A, B, C].map(DockspaceNode::pane), shares)
            .unwrap();
    let declared = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(SurfaceId::new(1), DockspaceRootLayout::new(ROOT, source)),
        DockspaceSurfaceLayout::new(
            SurfaceId::new(2),
            DockspaceRootLayout::new(TARGET, DockspaceNode::tabs([D])),
        ),
    ])
    .unwrap();
    let mut workspace = declared.into_workspace();
    let grid = workspace.root(ROOT).unwrap().node;
    let source = workspace.capture_item_source_by_id(C).unwrap().unwrap();
    let target = workspace
        .capture_tab_target(TARGET, workspace.root(TARGET).unwrap().node)
        .unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
        payload: MovePayload::Item(source),
        target: DockTarget::Center(target),
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    let view = crate::model::DockspaceView::new(&workspace);
    let remaining = view.root(ROOT).unwrap().content().unwrap().grid().unwrap();
    assert_eq!(remaining.shape(), fixed(1, 2));
    assert_eq!(
        remaining.shares().unwrap().rows().collect::<Vec<_>>(),
        [1.0]
    );
    assert_eq!(
        remaining
            .shares()
            .unwrap()
            .columns()
            .map(f64::to_bits)
            .collect::<Vec<_>>(),
        column_bits
    );
    assert_eq!(workspace.root(ROOT).unwrap().node, grid);
    assert_eq!(view.root(ROOT).unwrap().items().collect::<Vec<_>>(), [A, B]);
    assert_eq!(
        view.root(TARGET).unwrap().items().collect::<Vec<_>>(),
        [D, C]
    );
    workspace.validate().unwrap();
}

#[test]
fn extracting_the_last_bare_child_retires_nested_empty_grids_before_strict_normalization() {
    let inner = DockspaceNode::grid(fixed(1, 1), [DockspaceNode::pane(A)]).unwrap();
    let outer = DockspaceNode::grid(DockspaceGridShape::Auto, [inner]).unwrap();
    let declared = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SurfaceId::new(1),
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [outer, DockspaceNode::central_tabs([B])],
                )
                .unwrap(),
            ),
        ),
        DockspaceSurfaceLayout::new(
            SurfaceId::new(2),
            DockspaceRootLayout::new(TARGET, DockspaceNode::tabs([D])),
        ),
    ])
    .unwrap();
    let mut workspace = declared.into_workspace();
    let central = workspace.root(ROOT).unwrap().central.unwrap();
    let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
    let target = workspace
        .capture_tab_target(TARGET, workspace.root(TARGET).unwrap().node)
        .unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
        payload: MovePayload::Item(source),
        target: DockTarget::Center(target),
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    assert_eq!(workspace.root(ROOT).unwrap().node, central);
    assert!(
        !workspace
            .nodes()
            .any(|(_, node)| matches!(node, Node::Grid { .. }))
    );
    assert_eq!(
        workspace.item_multiset(),
        BTreeMap::from([(A, 1), (B, 1), (D, 1)])
    );
    workspace.validate().unwrap();
}

#[test]
fn closing_all_bare_grid_layouts_destroys_whole_subtrees_for_tab_and_group_scopes() {
    for whole_group in [false, true] {
        let grid = DockspaceNode::grid(fixed(2, 2), [A, B, D].map(DockspaceNode::pane)).unwrap();
        let workspace = layout(DockspaceNode::composed_tabs([DockspaceNode::pane(C), grid]))
            .with_hidden_items([B])
            .unwrap()
            .into_workspace();
        let tabs = workspace.root(ROOT).unwrap().node;
        let Node::Tabs { items, .. } = workspace.node(tabs).unwrap() else {
            unreachable!()
        };
        let tab = items[1];
        let node = tab.layout().unwrap();
        let policy = DockPolicySnapshot::default();
        let members = workspace.collect_items_in_subtree(if whole_group { tabs } else { node });
        let requirements = members
            .iter()
            .map(|item| CloseItemRequirement::new(*item, policy.pane_close_capability(*item)))
            .collect();
        let scope = if whole_group {
            ContentCloseScope::Tabs {
                root: ROOT,
                tabs,
                owner: workspace.presentation_for_root(ROOT).unwrap(),
                items: members.iter().copied().collect(),
                requirements,
            }
        } else {
            ContentCloseScope::Tab {
                root: ROOT,
                tabs,
                tab,
                node,
                owner: workspace.presentation_for_root(ROOT).unwrap(),
                items: members.iter().copied().collect(),
                requirements,
            }
        };
        let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
        if whole_group {
            assert_eq!(
                commit.outcome,
                CloseCommitOutcome::TabsClosed {
                    root: ROOT,
                    items: members
                }
            );
            assert_eq!(commit.candidate, Workspace::new());
        } else {
            assert_eq!(
                commit.outcome,
                CloseCommitOutcome::TabClosed {
                    root: ROOT,
                    tab,
                    items: members
                }
            );
            assert_eq!(commit.candidate.item_multiset(), BTreeMap::from([(C, 1)]));
            assert_eq!(commit.candidate.nodes().count(), 1);
        }
        commit.candidate.validate().unwrap();
    }
}

#[test]
fn closing_a_grid_group_preserves_the_actual_central_tabs_path_as_a_legal_singleton_grid() {
    let grid = DockspaceNode::grid(
        fixed(1, 2),
        [DockspaceNode::central_tabs([A]), DockspaceNode::pane(B)],
    )
    .unwrap();
    let workspace =
        layout(DockspaceNode::composed_tabs([DockspaceNode::pane(C), grid])).into_workspace();
    let tabs = workspace.root(ROOT).unwrap().node;
    let central = workspace.root(ROOT).unwrap().central.unwrap();
    let Node::Tabs { items, .. } = workspace.node(tabs).unwrap() else {
        unreachable!()
    };
    let grid_tab = items[1];
    let source = workspace.capture_tab_source(ROOT, tabs, grid_tab).unwrap();
    assert!(matches!(
        crate::graph::operation::validate_layout_tab_close(&workspace, &source),
        Err(CommandError::CentralNodeWouldDetach { .. })
    ));
    let policy = DockPolicySnapshot::default();
    let scope = ContentCloseScope::Tabs {
        root: ROOT,
        tabs,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B, C]),
        requirements: [C, A, B]
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .into(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(commit.candidate.root(ROOT).unwrap().central, Some(central));
    assert!(
        matches!(commit.candidate.node(central), Some(Node::Tabs { items, .. }) if items.is_empty())
    );
    let Node::Grid {
        shape, children, ..
    } = commit.candidate.node(grid_tab.layout().unwrap()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(*shape, fixed(1, 1));
    assert_eq!(children, &[central]);
    commit.candidate.validate().unwrap();
}

#[test]
fn composed_grid_move_hide_restore_and_close_keep_the_exact_content_scope() {
    let grid = DockspaceNode::grid(
        fixed(1, 2),
        [DockspaceNode::pane(A), DockspaceNode::tabs([B, D])],
    )
    .unwrap()
    .named("Tools");
    let mut workspace =
        layout(DockspaceNode::composed_tabs([DockspaceNode::pane(C), grid])).into_workspace();
    let all_items = BTreeMap::from([(A, 1), (B, 1), (C, 1), (D, 1)]);
    assert_eq!(workspace.item_multiset(), all_items);
    let tabs = workspace.root(ROOT).unwrap().node;
    let Node::Tabs { items, .. } = workspace.node(tabs).unwrap() else {
        unreachable!()
    };
    let tab = items[1];
    let grid = tab.layout().unwrap();
    let policy = DockPolicySnapshot::default();
    let old_scope = ContentCloseScope::Tab {
        root: ROOT,
        tabs,
        tab,
        node: grid,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B, D]),
        requirements: [A, B, D]
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .into(),
    };
    let source = workspace.capture_item_source_by_id(B).unwrap().unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::SetItemVisibility {
        source,
        visible: false,
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert_eq!(workspace.item_multiset(), all_items);
    assert!(old_scope.matches(&workspace, &policy));
    workspace.validate().unwrap();

    let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
    let old_pane = source.owner_node();
    let target = workspace.capture_tab_target(ROOT, tabs).unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
        payload: MovePayload::Item(source),
        target: DockTarget::Center(target),
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert_eq!(workspace.item_multiset(), all_items);
    assert!(workspace.node(old_pane).is_none());
    assert_eq!(workspace.root(ROOT).unwrap().node, tabs);
    assert_eq!(workspace.collect_items_in_subtree(tabs), [C, B, D, A]);
    assert!(matches!(
        workspace.node(grid),
        Some(Node::Grid { shape, children, .. })
            if *shape == fixed(1, 1) && children.len() == 1
    ));
    assert!(!old_scope.matches(&workspace, &policy));
    assert!(prepare_scoped_content_close(&workspace, &policy, &old_scope).is_err());
    workspace.validate().unwrap();

    #[cfg(feature = "serde")]
    let workspace = {
        let snapshot = WorkspaceSnapshot::capture(&workspace).unwrap();
        let restored = snapshot
            .build_candidate(|item| [A, B, C, D].contains(&item))
            .unwrap();
        assert_eq!(WorkspaceSnapshot::capture(&restored).unwrap(), snapshot);
        restored
    };
    assert_eq!(workspace.item_multiset(), all_items);
    assert_eq!(workspace.hidden_items().collect::<Vec<_>>(), [B]);
    let root = crate::model::DockspaceView::new(&workspace)
        .root(ROOT)
        .unwrap();
    assert_eq!(root.items().collect::<Vec<_>>(), [C, B, D, A]);
    assert_eq!(root.active_items().collect::<Vec<_>>(), [A]);
    let tabs = workspace.root(ROOT).unwrap().node;
    let Node::Tabs { items, .. } = workspace.node(tabs).unwrap() else {
        unreachable!()
    };
    let tab = items[1];
    let grid = tab.layout().unwrap();
    assert_eq!(workspace.node(grid).unwrap().layout_title(), Some("Tools"));
    let scope = ContentCloseScope::Tab {
        root: ROOT,
        tabs,
        tab,
        node: grid,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([B, D]),
        requirements: [B, D]
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .into(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabClosed {
            root: ROOT,
            tab,
            items: vec![B, D]
        }
    );
    assert_eq!(
        commit.candidate.item_multiset(),
        BTreeMap::from([(A, 1), (C, 1)])
    );
    assert_eq!(commit.candidate.collect_items_in_subtree(tabs), [C, A]);
    assert!(matches!(
        commit.candidate.node(tabs),
        Some(Node::Tabs { items, selected, .. })
            if items == &[TabKey::for_item(C), TabKey::for_item(A)]
                && *selected == Some(TabKey::for_item(A))
    ));
    assert_eq!(commit.candidate.nodes().count(), 1);
    assert!(commit.candidate.hidden_items().next().is_none());
    commit.candidate.validate().unwrap();
}

#[test]
fn protected_grid_central_survives_hide_move_group_close_restore_and_reopen() {
    let grid = DockspaceNode::grid(
        fixed(1, 2),
        [DockspaceNode::central_tabs([A]), DockspaceNode::pane(B)],
    )
    .unwrap()
    .named("Tools");
    let mut workspace =
        layout(DockspaceNode::composed_tabs([DockspaceNode::pane(C), grid])).into_workspace();
    let all_items = BTreeMap::from([(A, 1), (B, 1), (C, 1)]);
    let tabs = workspace.root(ROOT).unwrap().node;
    let central = workspace.root(ROOT).unwrap().central.unwrap();
    let policy = DockPolicySnapshot::default();
    let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::SetItemVisibility {
        source,
        visible: false,
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert_eq!(workspace.item_multiset(), all_items);
    workspace.validate().unwrap();

    let source = workspace.capture_item_source_by_id(B).unwrap().unwrap();
    let target = workspace.capture_tab_target(ROOT, tabs).unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
        payload: MovePayload::Item(source),
        target: DockTarget::Center(target),
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert_eq!(workspace.item_multiset(), all_items);
    assert_eq!(workspace.collect_items_in_subtree(tabs), [C, A, B]);
    assert_eq!(workspace.hidden_items().collect::<Vec<_>>(), [A]);
    assert!(workspace.item_is_active(B));
    assert!(!workspace.item_is_active(A));
    let Node::Tabs { items, .. } = workspace.node(tabs).unwrap() else {
        unreachable!()
    };
    let tab = items[1];
    let grid = tab.layout().unwrap();
    assert!(matches!(
        workspace.node(grid),
        Some(Node::Grid { shape, children, .. })
            if *shape == fixed(1, 1) && children == &[central]
    ));
    let source = workspace.capture_tab_source(ROOT, tabs, tab).unwrap();
    assert_eq!(
        crate::graph::operation::validate_layout_tab_close(&workspace, &source),
        Err(CommandError::CentralNodeWouldDetach {
            root: ROOT,
            source_node: grid,
            central,
        })
    );
    workspace.validate().unwrap();

    let scope = ContentCloseScope::Tabs {
        root: ROOT,
        tabs,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B, C]),
        requirements: [C, A, B]
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .into(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabsClosed {
            root: ROOT,
            items: vec![C, A, B]
        }
    );
    let workspace = commit.candidate;
    assert!(workspace.item_multiset().is_empty());
    assert!(workspace.hidden_items().next().is_none());
    assert_eq!(workspace.root(ROOT).unwrap().node, tabs);
    assert_eq!(workspace.root(ROOT).unwrap().central, Some(central));
    assert!(matches!(
        workspace.node(tabs),
        Some(Node::Tabs { items, selected, .. })
            if items == &[tab] && *selected == Some(tab)
    ));
    assert!(matches!(
        workspace.node(grid),
        Some(Node::Grid { shape, children, .. })
            if *shape == fixed(1, 1) && children == &[central]
    ));
    assert!(matches!(
        workspace.node(central),
        Some(Node::Tabs { items, selected: None, mru, .. })
            if items.is_empty() && mru.is_empty()
    ));
    workspace.validate().unwrap();

    #[cfg(feature = "serde")]
    let mut restored = {
        let snapshot = WorkspaceSnapshot::capture(&workspace).unwrap();
        let restored = snapshot.build_candidate(|_| false).unwrap();
        assert_eq!(WorkspaceSnapshot::capture(&restored).unwrap(), snapshot);
        restored
    };
    #[cfg(not(feature = "serde"))]
    let mut restored = workspace;
    let central = restored.root(ROOT).unwrap().central.unwrap();
    let target = restored.capture_tab_target(ROOT, central).unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Open {
        item: D,
        target: DockTarget::Center(target),
    }])
    .apply(&mut restored, &policy)
    .unwrap();
    assert_eq!(restored.item_multiset(), BTreeMap::from([(D, 1)]));
    assert_eq!(restored.root(ROOT).unwrap().central, Some(central));
    let root = crate::model::DockspaceView::new(&restored)
        .root(ROOT)
        .unwrap();
    assert_eq!(root.items().collect::<Vec<_>>(), [D]);
    assert_eq!(root.active_items().collect::<Vec<_>>(), [D]);
    let Node::Tabs { items, .. } = restored.node(restored.root(ROOT).unwrap().node).unwrap() else {
        unreachable!()
    };
    let grid = items[0].layout().unwrap();
    assert_eq!(restored.node(grid).unwrap().layout_title(), Some("Tools"));
    restored.validate().unwrap();
}

#[cfg(feature = "serde")]
#[test]
fn strict_grid_snapshot_roundtrip_keeps_titles_shares_hidden_content_and_real_registry_queries() {
    let declared = layout(
        DockspaceNode::grid_with_shares(
            fixed(2, 2),
            [A, B, C, D].map(DockspaceNode::pane),
            DockspaceGridShares::new([2.0, 3.0], [4.0, 1.0]).unwrap(),
        )
        .unwrap()
        .named("Tools"),
    )
    .with_hidden_items([B])
    .unwrap();
    let snapshot = WorkspaceSnapshot::capture(&declared.into_workspace()).unwrap();
    let decoded: WorkspaceSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let queried = std::cell::RefCell::new(Vec::new());
    let restored = decoded
        .build_candidate(|item| {
            queried.borrow_mut().push(item);
            true
        })
        .unwrap();
    assert_eq!(*queried.borrow(), [A, B, C, D]);
    assert_eq!(WorkspaceSnapshot::capture(&restored).unwrap(), snapshot);
    for corruption in [0, 1, 2, 3] {
        let mut broken = snapshot.clone();
        let record = broken
            .nodes
            .iter_mut()
            .find(|record| matches!(record.node, SnapshotNode::Grid { .. }))
            .unwrap();
        let SnapshotNode::Grid {
            shape,
            shares,
            children,
            ..
        } = &mut record.node
        else {
            unreachable!()
        };
        match corruption {
            0 => {
                *shape = SnapshotGridShape::Fixed {
                    rows: 0,
                    columns: 2,
                }
            }
            1 => shares.as_mut().unwrap().rows = vec![2.0, 3.0],
            2 => shares.as_mut().unwrap().columns = vec![1.0],
            3 => children[0] = record.id,
            _ => unreachable!(),
        }
        let calls = std::cell::Cell::new(0);
        assert!(
            broken
                .build_candidate(|_| {
                    calls.set(calls.get() + 1);
                    true
                })
                .is_err()
        );
        assert_eq!(calls.get(), 0);
    }
    let mut invalid = snapshot.clone();
    let record = invalid
        .nodes
        .iter_mut()
        .find(|record| matches!(record.node, SnapshotNode::Grid { .. }))
        .unwrap();
    let SnapshotNode::Grid { shares, .. } = &mut record.node else {
        unreachable!()
    };
    shares.as_mut().unwrap().rows = vec![0.0, 1.0];
    assert!(matches!(
        invalid.build_candidate(|_| true),
        Err(SnapshotRestoreError::InvalidGrid { .. })
    ));
    let mut missing = serde_json::to_value(&snapshot).unwrap();
    missing["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|record| record["node"]["kind"] == "grid")
        .unwrap()["node"]
        .as_object_mut()
        .unwrap()
        .remove("shares");
    assert!(serde_json::from_value::<WorkspaceSnapshot>(missing).is_err());
}
