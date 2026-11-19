//! Retained visibility semantics through independently recorded public outputs.

use super::*;
use dockspace::model::DockspaceActionOutcome;
use dockspace::runtime::{
    DockspaceReceiverDescriptor, PaintedSurfaceOutput, SurfaceMeasurementAnswer as Answer,
    SurfaceMeasurementRequest as Request, SurfacePaintDisposition,
};
use dockspace_host_conformance::scenario::{JourneyId, LOCAL_PANE_VISIBILITY, ScenarioId};

struct RecordedVisibility {
    output: PaintedSurfaceOutput,
    receivers: Vec<(ItemId, DockspaceReceiverDescriptor)>,
}

fn record_visibility(session: &mut DockspaceSession) -> RecordedVisibility {
    let header = session.presentation_config().tab_bar_height();
    let mut frame = session.begin_host_frame().unwrap();
    let view = frame.view();
    let visible = [FIRST, SECOND, THIRD, CONTAINED]
        .into_iter()
        .filter(|item| view.item(*item).unwrap().is_visible())
        .collect::<Vec<_>>();
    let mut selected = [FIRST, THIRD, CONTAINED]
        .into_iter()
        .filter_map(|item| {
            view.item(item)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .visible_selected()
                .and_then(|tab| tab.item())
        })
        .collect::<Vec<_>>();
    selected.sort_unstable();
    let mut minima = Vec::new();
    let mut intrinsics = Vec::new();
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    frame
        .measure_surface_with(SURFACE, |request| {
            Ok::<_, dockspace::runtime::DockspaceRuntimeError>(match request {
                Request::DockBounds { .. } | Request::PopupPlaneBounds { .. } => {
                    Answer::Bounds(bounds)
                }
                Request::PaneSizing { item, .. } => {
                    let item = item.expect("an entirely hidden group has no empty-content request");
                    assert!(
                        visible.contains(&item),
                        "the hidden pane has no loaded metadata"
                    );
                    minima.push(item);
                    Answer::PaneSizing((LogicalSize::new(32.0, 24.0).unwrap()).into())
                }
                Request::TabIntrinsic { key, .. } => {
                    let Some(item) = key.item() else {
                        return Ok(Answer::TabIntrinsic(72.0));
                    };
                    assert!(
                        visible.contains(&item),
                        "the hidden tab has no text metadata"
                    );
                    intrinsics.push(item);
                    Answer::TabIntrinsic(72.0)
                }
                Request::TabStrip { .. } => {
                    Answer::TabStrip(dockspace::runtime::TabStripMetrics::new(0.0, 0.0).unwrap())
                }
            })
        })
        .unwrap();
    minima.sort_unstable();
    intrinsics.sort_unstable();
    assert_eq!(minima, visible);
    assert_eq!(intrinsics, visible);

    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let mut painted_tabs = Vec::new();
    let mut contained_count = 0;
    let mut splitter_count = 0;
    let group_hidden = !visible.contains(&FIRST) && !visible.contains(&SECOND);
    plan.visit_surface_program(|step| {
        if let Step::Operation { operation, .. } = step {
            match operation {
                Operation::Tab(tab) => painted_tabs.extend(tab.item()),
                Operation::Pane(pane) if group_hidden && pane.item() == THIRD => {
                    let geometry = pane.content_geometry();
                    assert_eq!(
                        geometry.layout_bounds(),
                        LogicalRect::new(0.0, header, bounds.width(), bounds.height() - header)
                            .unwrap()
                    );
                    assert_eq!(geometry.clip(), geometry.layout_bounds());
                }
                Operation::Tabs(tabs) if group_hidden && tabs.root() == ROOT => {
                    assert_eq!(tabs.bounds(), bounds);
                }
                Operation::ContainedControls(_) => contained_count += 1,
                Operation::Splitter(_) => splitter_count += 1,
                _ => {}
            }
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    painted_tabs.sort_unstable();
    assert_eq!(painted_tabs, visible);
    assert_eq!(contained_count, usize::from(visible.contains(&CONTAINED)));
    assert_eq!(splitter_count, usize::from(!group_hidden));
    let receivers = visible
        .iter()
        .map(|item| (*item, plan.tab_receiver(*item).unwrap()))
        .collect();
    for item in [FIRST, SECOND, THIRD, CONTAINED] {
        if !visible.contains(&item) {
            assert!(plan.tab_receiver(item).is_none());
            assert!(plan.prepare_tab_select(item).is_none());
            assert!(plan.prepare_tab_close(item).is_none());
        }
    }
    let (recording, receipt) = RecordingRenderer::execute(plan).unwrap();
    let mut content = recording
        .records
        .iter()
        .filter_map(|record| match record {
            OwnedRecord::PaneContent { item, paint, .. } => {
                assert_eq!(*paint, SurfacePaintDisposition::Visible);
                Some(*item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    content.sort_unstable();
    assert_eq!(content, selected);
    frame.confirm_surface_painted(receipt).unwrap();
    let mut outputs = commit(frame).take_painted_outputs();
    assert_eq!(outputs.len(), 1);
    let output = outputs.pop().unwrap();
    assert!(matches!(
        recording.records.last(),
        Some(OwnedRecord::SurfaceEnd(semantic)) if output.matches_semantic_output(*semantic)
    ));
    RecordedVisibility { output, receivers }
}

fn present_visibility(session: &mut DockspaceSession) {
    let recorded = record_visibility(session);
    let artifact = recorded.output.presentation_artifact();
    session
        .report_surface_presentation(recorded.output, SurfacePresentationResult::Presented)
        .unwrap();
    let settled = commit(session.begin_host_frame().unwrap());
    assert!(
        settled
            .surface_presentation_commits()
            .iter()
            .any(|acknowledgement| acknowledgement.presents(artifact))
    );
    for (_, receiver) in recorded.receivers {
        assert!(session.bind_presented_receiver(&receiver).is_some());
    }
}

fn set_visible(
    session: &mut DockspaceSession,
    target: ItemId,
    visible: bool,
) -> SemanticObservation {
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.set_item_visibility_current(target, visible).unwrap();
    let report = commit(frame);
    let Some(HostInputOutcome::ProductActionApplied(
        DockspaceActionOutcome::ItemVisibilityChanged {
            item: changed,
            visible: committed,
            changed: true,
        },
    )) = report.submitted_action_outcome(submitted)
    else {
        panic!("visibility must have one actual committed result: {report:?}");
    };
    let current = session.view().item(*changed).unwrap();
    assert_eq!(current.item(), target);
    assert_eq!(current.is_visible(), *committed);
    SemanticObservation::ItemVisibilityChanged {
        item: alias(current.item()),
        visible: current.is_visible(),
    }
}

#[test]
fn shared_l08_visibility_executes_through_independent_recording_and_public_actions() {
    let supported = [
        ScenarioCapability::LocalTabSelection,
        ScenarioCapability::LocalItemVisibility,
        ScenarioCapability::LocalSplitterAdjustment,
        ScenarioCapability::FullContainedProjection,
    ];
    assert_eq!(LOCAL_PANE_VISIBILITY.id(), ScenarioId::LocalPaneVisibility);
    assert_eq!(LOCAL_PANE_VISIBILITY.journey(), JourneyId::L08);
    assert_eq!(
        LOCAL_PANE_VISIBILITY.admission(&supported),
        ScenarioAdmission::Ready
    );
    let mut session = session();
    let original_rect = session.view().contained(FLOATING).unwrap().rect();
    let expected_selection = [SECOND, FIRST, FIRST, SECOND, SECOND, SECOND, SECOND, SECOND];
    present_visibility(&mut session);
    let mut observations = Vec::new();
    for (index, step) in LOCAL_PANE_VISIBILITY.steps().iter().enumerate() {
        let before_weights = weights(&session);
        let observed = match (step.selector(), step.action()) {
            (SemanticSelector::Item(target), SemanticAction::SetVisible(visible)) => {
                let observed = set_visible(&mut session, item(target), visible);
                assert_eq!(weights(&session), before_weights);
                observed
            }
            (SemanticSelector::Tab(target), SemanticAction::Activate) => {
                let action = paint_and_present(&mut session, *step);
                let mut frame = session.begin_host_frame().unwrap();
                frame.submit_surface_action(action).unwrap();
                let report = commit(frame);
                assert!(report.workspace_changed());
                let current = session.view().item(item(target)).unwrap();
                assert!(current.is_active());
                SemanticObservation::ItemSelected(alias(
                    current
                        .tabs()
                        .expect("item is an inline tab")
                        .visible_selected()
                        .and_then(|tab| tab.item())
                        .unwrap(),
                ))
            }
            (SemanticSelector::Splitter(ScenarioAxis::Horizontal), SemanticAction::Increment) => {
                let action = paint_and_present(&mut session, *step);
                let mut frame = session.begin_host_frame().unwrap();
                frame.submit_surface_action(action).unwrap();
                let report = commit(frame);
                assert!(report.workspace_changed());
                let after = weights(&session);
                assert!(after[0] > before_weights[0]);
                assert!(after[1] < before_weights[1]);
                let axis = session
                    .view()
                    .root(ROOT)
                    .unwrap()
                    .content()
                    .unwrap()
                    .split()
                    .unwrap()
                    .axis();
                SemanticObservation::SplitterAdjusted(match axis {
                    DockspaceAxis::Horizontal => ScenarioAxis::Horizontal,
                    DockspaceAxis::Vertical => ScenarioAxis::Vertical,
                })
            }
            unsupported => panic!("unsupported visibility scenario step: {unsupported:?}"),
        };
        observations.push(observed);
        let tabs = session
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .expect("fixture item is an inline tab");
        assert_eq!(tabs.items().collect::<Vec<_>>(), [FIRST, SECOND]);
        assert_eq!(
            tabs.selected().and_then(|tab| tab.item()),
            Some(expected_selection[index])
        );
        assert_eq!(
            tabs.visible_selected().and_then(|tab| tab.item()),
            (index != 2).then_some(expected_selection[index])
        );
        assert_eq!(
            tabs.visible_items().collect::<Vec<_>>(),
            [FIRST, SECOND]
                .into_iter()
                .filter(|item| session.view().item(*item).unwrap().is_visible())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            session.view().contained(FLOATING).unwrap().rect(),
            original_rect
        );
        assert_eq!(
            session
                .view()
                .item(THIRD)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            [THIRD]
        );
        assert_eq!(
            session
                .view()
                .item(CONTAINED)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            [CONTAINED]
        );
        assert!(session.pending_content_close_plans().next().is_none());
        assert!(!session.has_active_gesture());
        present_visibility(&mut session);
    }
    assert_eq!(observations, LOCAL_PANE_VISIBILITY.expected_observations());
    assert!(session.view().hidden_items().next().is_none());
}

#[test]
fn retained_outputs_cannot_reauthorize_a_hidden_receiver_and_fresh_show_actions_work() {
    for result in [
        SurfacePresentationResult::Presented,
        SurfacePresentationResult::Dropped,
    ] {
        let mut session = session();
        let initial = record_visibility(&mut session);
        let old_receiver = initial
            .receivers
            .iter()
            .find(|(item, _)| *item == SECOND)
            .unwrap()
            .1;
        assert!(session.bind_presented_receiver(&old_receiver).is_none());
        set_visible(&mut session, SECOND, false);
        session
            .report_surface_presentation(initial.output, result)
            .unwrap();
        commit(session.begin_host_frame().unwrap());
        assert!(session.bind_presented_receiver(&old_receiver).is_none());
        assert!(!session.view().item(SECOND).unwrap().is_visible());
        present_visibility(&mut session);
        assert!(session.bind_presented_receiver(&old_receiver).is_none());

        set_visible(&mut session, SECOND, true);
        present_visibility(&mut session);
        assert!(session.bind_presented_receiver(&old_receiver).is_none());
        let action = paint_and_present(&mut session, LOCAL_PANE_VISIBILITY.steps()[0]);
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_surface_action(action).unwrap();
        assert!(commit(frame).workspace_changed());
        assert_eq!(
            session
                .view()
                .item(SECOND)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .visible_selected()
                .and_then(|tab| tab.item()),
            Some(SECOND)
        );
        assert_eq!(
            session
                .view()
                .item(FIRST)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            [FIRST, SECOND]
        );
        present_visibility(&mut session);
    }
}

#[cfg(feature = "serde")]
#[test]
fn document_restore_keeps_hidden_groups_and_rebuilds_only_visible_public_output() {
    use dockspace::runtime::{DockspaceDocumentBootstrap, DockspaceDocumentId};

    const DOCUMENT: DockspaceDocumentId = DockspaceDocumentId::from_bytes([0x78; 16]);
    let mut bootstrap = DockspaceDocumentBootstrap::new(DOCUMENT);
    for (key, expected) in [
        ("pane:primary", FIRST),
        ("pane:secondary", SECOND),
        ("pane:third", THIRD),
        ("pane:contained", CONTAINED),
    ] {
        assert_eq!(bootstrap.ensure_item(key).unwrap(), expected);
    }
    let mut original = DockspaceSession::from_persistent_layout(
        session().view().to_layout(),
        DockPolicy::default(),
        bootstrap,
    )
    .unwrap();
    for item in [SECOND, FIRST, CONTAINED] {
        set_visible(&mut original, item, false);
    }
    present_visibility(&mut original);
    let bytes = original.save_document_json().unwrap();
    let mut restored =
        DockspaceSession::from_document_json(&bytes, DockPolicy::default(), |id, key| {
            id == DOCUMENT
                && matches!(
                    key,
                    "pane:primary" | "pane:secondary" | "pane:third" | "pane:contained"
                )
        })
        .unwrap();
    for item in [FIRST, SECOND, THIRD, CONTAINED] {
        let before = original.view().item(item).unwrap();
        let after = restored.view().item(item).unwrap();
        assert_eq!(after.surface(), before.surface());
        assert_eq!(after.root(), before.root());
        assert_eq!(after.contained(), before.contained());
        assert_eq!(after.tab_index(), before.tab_index());
        assert_eq!(
            after
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            before
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            after
                .tabs()
                .expect("item is an inline tab")
                .selected()
                .and_then(|tab| tab.item()),
            before
                .tabs()
                .expect("item is an inline tab")
                .selected()
                .and_then(|tab| tab.item())
        );
        assert_eq!(after.is_visible(), before.is_visible());
    }
    assert_eq!(
        restored.view().hidden_items().collect::<Vec<_>>(),
        [FIRST, SECOND, CONTAINED]
    );
    assert_eq!(
        restored
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .expect("item is an inline tab")
            .items()
            .collect::<Vec<_>>(),
        [FIRST, SECOND]
    );
    assert_eq!(
        restored
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .expect("item is an inline tab")
            .visible_selected()
            .and_then(|tab| tab.item()),
        None
    );
    present_visibility(&mut restored);
    for item in [SECOND, FIRST, CONTAINED] {
        set_visible(&mut restored, item, true);
        present_visibility(&mut restored);
    }
    assert!(restored.view().hidden_items().next().is_none());
    assert_eq!(weights(&restored), weights(&original));
    assert_eq!(
        restored.view().contained(FLOATING).unwrap().rect(),
        original.view().contained(FLOATING).unwrap().rect()
    );
    assert_eq!(
        restored
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .expect("item is an inline tab")
            .visible_selected()
            .and_then(|tab| tab.item()),
        Some(SECOND)
    );
}
