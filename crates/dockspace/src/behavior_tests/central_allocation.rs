//! Empty anchors and numeric allocation are independent product contracts.

use crate::geometry::{LogicalRect, LogicalSize};
use crate::model::{
    DockAnchor, DockPlacement, DockspaceAxis, DockspaceContainedLayout, DockspaceLayout,
    DockspaceLayoutError, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    FloatingPresentationId, ItemId, RootAllocation, RootId, SurfaceId,
};
use crate::policy::DockPolicy;
use crate::runtime::{
    DockspaceRuntimeError, DockspaceSession, SurfaceMeasurementAnswer, SurfaceMeasurementRequest,
    SurfaceUnavailableReason, TabStripMetrics,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const FLOATING_ROOT: RootId = RootId::new(2);
const FIRST: ItemId = ItemId::new(1);
const MIDDLE: ItemId = ItemId::new(2);
const LAST: ItemId = ItemId::new(3);
const NESTED: ItemId = ItemId::new(4);

fn weighted_layout(
    allocation: RootAllocation,
    axis: DockspaceAxis,
    empty: bool,
    nested: bool,
) -> DockspaceLayout {
    let anchor = DockspaceNode::central_tabs(if empty { Vec::new() } else { vec![MIDDLE] });
    let middle = if nested {
        let perpendicular = match axis {
            DockspaceAxis::Horizontal => DockspaceAxis::Vertical,
            DockspaceAxis::Vertical => DockspaceAxis::Horizontal,
        };
        DockspaceNode::equal_split(perpendicular, [anchor, DockspaceNode::tabs([NESTED])]).unwrap()
    } else {
        anchor
    };
    let content = DockspaceNode::split(
        axis,
        [
            (DockspaceNode::tabs([FIRST]), 1.0),
            (middle, 2.0),
            (DockspaceNode::tabs([LAST]), 1.0),
        ],
    )
    .unwrap();
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content).allocation(allocation),
    )])
    .unwrap()
}

fn measured_panes(
    session: &mut DockspaceSession,
    axis: DockspaceAxis,
    constrained: bool,
) -> Vec<(Option<ItemId>, LogicalRect)> {
    let config = session.presentation_config();
    let full_extent = 800.0 + 2.0 * config.splitter_thickness();
    let bounds = match axis {
        DockspaceAxis::Horizontal => LogicalRect::new(0.0, 0.0, full_extent, 600.0),
        DockspaceAxis::Vertical => LogicalRect::new(0.0, 0.0, 600.0, full_extent),
    }
    .unwrap();
    let tab_height = config.tab_bar_height();
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame
        .prepare_surface_with(SURFACE, |request| {
            Ok::<_, DockspaceRuntimeError>(match request {
                SurfaceMeasurementRequest::DockBounds { .. }
                | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                    SurfaceMeasurementAnswer::Bounds(bounds)
                }
                SurfaceMeasurementRequest::PaneSizing { item, .. } => {
                    let minimum = if constrained && item == Some(FIRST) {
                        match axis {
                            DockspaceAxis::Horizontal => LogicalSize::new(300.0, 0.0),
                            DockspaceAxis::Vertical => LogicalSize::new(0.0, 300.0 - tab_height),
                        }
                    } else {
                        LogicalSize::new(0.0, 0.0)
                    }
                    .unwrap();
                    SurfaceMeasurementAnswer::PaneSizing((minimum).into())
                }
                SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                    SurfaceMeasurementAnswer::TabIntrinsic(60.0)
                }
                SurfaceMeasurementRequest::TabStrip { .. } => {
                    SurfaceMeasurementAnswer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                }
            })
        })
        .unwrap()
        .ready()
        .expect("valid measured panes must compile");
    let panes = plan
        .panes()
        .map(|pane| (pane.selected(), pane.bounds()))
        .collect();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();
    panes
}

#[test]
fn measured_weighted_and_preferred_anchors_have_explicit_independent_sizes() {
    for axis in [DockspaceAxis::Horizontal, DockspaceAxis::Vertical] {
        for allocation in [RootAllocation::Weighted, RootAllocation::PreferCentral] {
            for empty in [false, true] {
                for nested in [false, true] {
                    let layout = weighted_layout(allocation, axis, empty, nested);
                    let mut session =
                        DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
                    for constrained in [false, true] {
                        let panes = measured_panes(&mut session, axis, constrained);
                        let expected = match (constrained, allocation) {
                            (false, _) => [200.0, 400.0, 200.0],
                            (true, RootAllocation::Weighted) => [300.0, 1000.0 / 3.0, 500.0 / 3.0],
                            (true, RootAllocation::PreferCentral) => [300.0, 300.0, 200.0],
                        };
                        for (item, size) in [Some(FIRST), (!empty).then_some(MIDDLE), Some(LAST)]
                            .into_iter()
                            .zip(expected)
                        {
                            let rect = panes
                                .iter()
                                .find(|(selected, _)| *selected == item)
                                .unwrap()
                                .1;
                            let actual = match axis {
                                DockspaceAxis::Horizontal => rect.width(),
                                DockspaceAxis::Vertical => rect.height(),
                            };
                            assert!(
                                (actual - size).abs() < 1.0e-6,
                                "{axis:?}, {allocation:?}, empty={empty}, nested={nested}: {actual} != {size}"
                            );
                        }
                        assert_eq!(session.view().root(ROOT).unwrap().allocation(), allocation);
                        assert_eq!(
                            session
                                .view()
                                .root(ROOT)
                                .unwrap()
                                .central()
                                .unwrap()
                                .tabs()
                                .unwrap()
                                .items()
                                .is_empty(),
                            empty
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn allocation_defaults_to_weighted_and_central_preference_requires_an_anchor() {
    let root = || DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([FIRST]));
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(SURFACE, root())]).unwrap();
    assert_eq!(
        layout.view().root(ROOT).unwrap().allocation(),
        RootAllocation::Weighted
    );
    assert!(matches!(
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            root().allocation(RootAllocation::PreferCentral)
        )]),
        Err(DockspaceLayoutError::CentralAllocationWithoutAnchor { root: ROOT })
    ));
}

#[test]
fn changing_only_allocation_invalidates_presented_structure_fingerprints() {
    let layout = weighted_layout(
        RootAllocation::Weighted,
        DockspaceAxis::Horizontal,
        false,
        false,
    );
    let original = layout.into_workspace();
    let record = *original.root(ROOT).unwrap();
    let mut draft = original.clone().into_builder();
    draft.set_root(ROOT, record.with_allocation(RootAllocation::PreferCentral));
    let changed = draft.build().unwrap();
    let before = original.root_fingerprint(ROOT).unwrap();
    let after = changed.root_fingerprint(ROOT).unwrap();
    assert!(!before.presentation_structure_eq(&after));
    assert!(!before.presentation_topology_eq(&after));
    assert_ne!(before, after);
    assert_eq!(original.item_multiset(), changed.item_multiset());
    assert_eq!(
        original.root(ROOT).unwrap().central,
        changed.root(ROOT).unwrap().central
    );
}

#[test]
fn moving_last_content_preserves_main_anchor_and_retires_contained_source_for_both_allocations() {
    for allocation in [RootAllocation::Weighted, RootAllocation::PreferCentral] {
        let main = DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [
                    DockspaceNode::central_tabs([FIRST]),
                    DockspaceNode::tabs([MIDDLE]),
                ],
            )
            .unwrap(),
        )
        .allocation(allocation);
        let floating = DockspaceContainedLayout::new(
            FloatingPresentationId::new(1),
            DockspaceRootLayout::new(FLOATING_ROOT, DockspaceNode::central_tabs([LAST]))
                .allocation(allocation),
            LogicalRect::new(10.0, 20.0, 320.0, 240.0).unwrap(),
        );
        let layout = DockspaceLayout::new([
            DockspaceSurfaceLayout::new(SURFACE, main).with_contained(floating)
        ])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        for item in [FIRST, LAST] {
            let mut frame = session.begin_host_frame().unwrap();
            frame
                .dock_item_current(item, DockPlacement::Center(DockAnchor::Item(MIDDLE)))
                .unwrap();
            frame
                .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
                .unwrap();
            frame.commit().unwrap();
        }
        let view = session.view();
        let root = view.root(ROOT).unwrap();
        assert_eq!(root.allocation(), allocation);
        assert!(root.central().unwrap().tabs().unwrap().items().is_empty());
        assert!(root.retains_empty_central_root());
        assert!(view.root(FLOATING_ROOT).is_none());
        assert_eq!(
            view.item(MIDDLE).unwrap().tabs().items(),
            &[MIDDLE, FIRST, LAST]
        );
        assert_eq!(view.item(MIDDLE).unwrap().tabs().selected(), Some(LAST));
        let restored = view.to_layout();
        assert_eq!(restored.view().root(ROOT).unwrap().allocation(), allocation);
    }
}

#[test]
#[cfg(feature = "serde")]
fn snapshots_require_and_preserve_explicit_allocation_without_legacy_defaults() {
    use crate::persistence::snapshot::WorkspaceSnapshot;

    for allocation in [RootAllocation::Weighted, RootAllocation::PreferCentral] {
        let layout = weighted_layout(allocation, DockspaceAxis::Horizontal, true, true);
        let snapshot = WorkspaceSnapshot::capture(&layout.into_workspace()).unwrap();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let decoded: WorkspaceSnapshot = serde_json::from_slice(&bytes).unwrap();
        let restored = decoded.build_candidate(|_| true).unwrap();
        assert_eq!(restored.root(ROOT).unwrap().allocation, allocation);
        assert_eq!(WorkspaceSnapshot::capture(&restored).unwrap(), snapshot);
        let mut invalid = serde_json::to_value(snapshot).unwrap();
        invalid["roots"][0]
            .as_object_mut()
            .unwrap()
            .remove("allocation");
        assert!(serde_json::from_value::<WorkspaceSnapshot>(invalid).is_err());
    }
}

#[test]
fn complete_root_float_and_return_preserve_anchor_topology_and_allocation() {
    use crate::graph::command::{ContainedPosition, RootPresentationTarget, WorkspaceCommand};
    use crate::graph::transaction::WorkspaceTransaction;

    for allocation in [RootAllocation::Weighted, RootAllocation::PreferCentral] {
        let mut workspace =
            weighted_layout(allocation, DockspaceAxis::Horizontal, true, true).into_workspace();
        let original = *workspace.root(ROOT).unwrap();
        let items = workspace.item_multiset();
        let floating = FloatingPresentationId::new(1);
        for float in [true, false] {
            let source = workspace.capture_node_source(ROOT, original.node).unwrap();
            let command = if float {
                WorkspaceCommand::RehomeRoot {
                    source,
                    target: RootPresentationTarget::Contained {
                        surface: SURFACE,
                        floating,
                        rect: LogicalRect::new(10.0, 20.0, 500.0, 400.0).unwrap(),
                        position: ContainedPosition::Front,
                    },
                }
            } else {
                WorkspaceCommand::PromoteContained {
                    source,
                    surface: SURFACE,
                    floating,
                }
            };
            WorkspaceTransaction::from_commands([command])
                .apply(&mut workspace, &Default::default())
                .unwrap();
            assert_eq!(workspace.root(ROOT), Some(&original));
            assert_eq!(workspace.item_multiset(), items);
            workspace.validate().unwrap();
        }
        assert!(workspace.contained_floatings().next().is_none());
        assert_eq!(workspace.surface(SURFACE).unwrap().main_root, Some(ROOT));
    }
}
