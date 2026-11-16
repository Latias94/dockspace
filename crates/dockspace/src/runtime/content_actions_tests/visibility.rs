use super::*;

use crate::model::{DockspaceActionRejection, DockspaceAxis, DockspaceContainedLayout};

fn set_visibility(session: &mut DockspaceSession, item: ItemId, visible: bool) -> HostFrameReport {
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.set_item_visibility_current(item, visible).unwrap();
    let report = commit(frame);
    assert!(matches!(
        report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::ProductActionApplied(
            DockspaceActionOutcome::ItemVisibilityChanged {
                item: actual,
                visible: actual_visible,
                ..
            }
        )) if *actual == item && *actual_visible == visible
    ));
    report
}

#[test]
fn visibility_is_durable_idempotent_and_keeps_tab_membership() {
    let mut session = session();
    let before = session.version();
    let report = set_visibility(&mut session, SECOND, false);
    assert!(report.workspace_changed());
    assert_ne!(session.version(), before);
    assert!(report.notification().document_changed());
    assert_eq!(report.events().len(), 1);
    assert_eq!(
        report.events()[0].kind(),
        &DockspaceEventKind::ItemVisibilityChanged {
            item: SECOND,
            visible: false,
        }
    );
    let pane = session.view().item(SECOND).unwrap();
    assert_eq!(pane.surface(), SURFACE);
    assert_eq!(pane.root(), ROOT);
    assert_eq!(pane.tab_index(), 1);
    assert_eq!(pane.tabs().items(), [FIRST, SECOND]);
    assert_eq!(pane.tabs().selected(), Some(FIRST));
    assert!(session.view().item_is_hidden(SECOND));
    assert_eq!(session.pending_content_close_plans().count(), 0);

    let version = session.version();
    let unchanged = set_visibility(&mut session, SECOND, false);
    assert!(!unchanged.workspace_changed());
    assert!(unchanged.notification().is_empty());
    assert_eq!(session.version(), version);

    let restored =
        DockspaceSession::from_layout(session.view().to_layout(), session.policy().clone())
            .unwrap();
    assert!(restored.view().item_is_hidden(SECOND));
    assert_eq!(restored.workspace(), session.workspace());

    set_visibility(&mut session, SECOND, true);
    assert!(!session.view().item_is_hidden(SECOND));
    assert_eq!(session.view().item(SECOND).unwrap().tab_index(), 1);
    assert_eq!(
        session.view().item(FIRST).unwrap().tabs().selected(),
        Some(FIRST)
    );
}

#[test]
fn hidden_content_needs_no_measurement_metadata_until_it_is_shown_again() {
    let mut session = session();
    for visible in [false, true] {
        set_visibility(&mut session, SECOND, visible);
        let mut pane_items = Vec::new();
        let mut tab_items = Vec::new();
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .measure_surface_with(SURFACE, |request| {
                Ok::<_, crate::runtime::DockspaceRuntimeError>(match request {
                    SurfaceMeasurementRequest::DockBounds { .. }
                    | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                        SurfaceMeasurementAnswer::Bounds(
                            LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
                        )
                    }
                    SurfaceMeasurementRequest::PaneMinimum { item, .. } => {
                        assert!(
                            visible || item != Some(SECOND),
                            "hidden content has no loaded size metadata"
                        );
                        pane_items.push(item.unwrap());
                        SurfaceMeasurementAnswer::PaneMinimum(LogicalSize::new(80.0, 60.0).unwrap())
                    }
                    SurfaceMeasurementRequest::TabIntrinsic { item, .. } => {
                        assert!(
                            visible || item != SECOND,
                            "hidden tabs have no text measurement"
                        );
                        tab_items.push(item);
                        SurfaceMeasurementAnswer::TabIntrinsic(96.0)
                    }
                    SurfaceMeasurementRequest::TabStrip { .. } => {
                        SurfaceMeasurementAnswer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                    }
                })
            })
            .unwrap();
        assert!(frame.paint_plan(SURFACE).unwrap().is_some());
        commit(frame);
        let expected = if visible {
            vec![FIRST, SECOND]
        } else {
            vec![FIRST]
        };
        assert_eq!(pane_items, expected);
        assert_eq!(tab_items, expected);
    }
}

#[test]
fn hiding_every_pane_preserves_complete_split_weights_and_surface_ownership() {
    let node = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([FIRST]), 0.2),
            (DockspaceNode::tabs([SECOND]), 0.3),
            (DockspaceNode::tabs([THIRD]), 0.5),
        ],
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, node),
    )])
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    let original = session.workspace().clone();

    for item in [SECOND, FIRST, THIRD] {
        set_visibility(&mut session, item, false);
    }
    assert!(session.view().surface(SURFACE).is_some());
    assert!(session.view().root(ROOT).is_some());
    assert_eq!(
        session.workspace().item_multiset(),
        original.item_multiset()
    );
    for item in [THIRD, FIRST, SECOND] {
        set_visibility(&mut session, item, true);
    }
    assert_eq!(session.workspace(), &original);
}

#[test]
fn visibility_rollback_and_stale_preparation_do_not_leak_candidate_state() {
    let mut session = session();
    let prepared = session.prepare_set_item_visibility(SECOND, false);
    let mut abandoned = session.begin_host_frame().unwrap();
    let abandoned_action = abandoned.set_item_visibility_current(FIRST, false).unwrap();
    assert!(abandoned.view().item_is_hidden(FIRST));
    drop(abandoned);
    assert!(!session.view().item_is_hidden(FIRST));
    let before = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    let accepted = frame.submit_prepared_action(prepared).unwrap();
    let report = commit(frame);
    assert_ne!(abandoned_action, accepted);
    assert!(report.submitted_action_outcome(abandoned_action).is_none());
    assert_ne!(session.version(), before);

    let stale = session.prepare_set_item_visibility(FIRST, false);
    set_visibility(&mut session, SECOND, true);
    let mut frame = session.begin_host_frame().unwrap();
    let action = frame.submit_prepared_action(stale).unwrap();
    let report = commit(frame);
    assert!(matches!(
        report.submitted_action_outcome(action),
        Some(HostInputOutcome::StaleRejected { .. })
    ));
    assert!(!session.view().item_is_hidden(FIRST));

    let foreign = self::session().prepare_set_item_visibility(FIRST, false);
    let mut frame = session.begin_host_frame().unwrap();
    assert!(frame.submit_prepared_action(foreign).is_err());
    drop(frame);
    assert!(!session.view().item_is_hidden(FIRST));
}

#[test]
fn hiding_a_focused_pane_uses_visible_successors_and_showing_does_not_activate() {
    let mut session = session();
    let prepared = session.prepare_select_item(SECOND);
    assert!(apply_action(&mut session, prepared));
    assert_eq!(
        session.pane_focus_request(SURFACE).unwrap().item(),
        Some(SECOND)
    );

    set_visibility(&mut session, SECOND, false);
    assert_eq!(
        session.view().item(FIRST).unwrap().tabs().selected(),
        Some(FIRST)
    );
    assert_eq!(
        session.pane_focus_request(SURFACE).unwrap().item(),
        Some(FIRST)
    );
    set_visibility(&mut session, FIRST, false);
    let workspace_focus = session.pane_focus_request(SURFACE).unwrap();
    assert_eq!(workspace_focus.target(), DockspaceFocusTarget::Workspace);
    assert_eq!(
        session
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .visible_selected(),
        None
    );

    set_visibility(&mut session, SECOND, true);
    assert_eq!(
        session.view().item(SECOND).unwrap().tabs().selected(),
        Some(SECOND)
    );
    assert_eq!(session.pane_focus_request(SURFACE), Some(workspace_focus));
    let mut frame = session.begin_host_frame().unwrap();
    let select = frame.select_item_current(FIRST).unwrap();
    let report = commit(frame);
    assert_eq!(
        report.submitted_action_outcome(select),
        Some(&HostInputOutcome::ProductActionRejected(
            DockspaceActionRejection::ItemHidden { item: FIRST },
        ))
    );
    assert_eq!(session.pane_focus_request(SURFACE), Some(workspace_focus));
}

#[test]
fn hiding_a_pane_does_not_override_a_newer_focus_request_on_another_surface() {
    let mut session = session();
    for item in [FIRST, THIRD] {
        let prepared = session.prepare_select_item(item);
        assert!(apply_action(&mut session, prepared));
    }
    let pending = session.pane_focus_request(OTHER_SURFACE).unwrap();
    assert_eq!(pending.item(), Some(THIRD));
    set_visibility(&mut session, FIRST, false);
    assert_eq!(session.pane_focus_request(OTHER_SURFACE), Some(pending));
    assert!(session.pane_focus_request(SURFACE).is_none());
}

#[test]
fn closing_a_focused_pane_skips_its_hidden_neighbor() {
    let mut session = session();
    let prepared = session.prepare_dock_item(THIRD, DockPlacement::After(SECOND));
    assert!(apply_action(&mut session, prepared));
    set_visibility(&mut session, SECOND, false);
    let prepared = session.prepare_select_item(FIRST);
    assert!(apply_action(&mut session, prepared));
    let mut frame = session.begin_host_frame().unwrap();
    let request = frame.request_close_item_current(FIRST).unwrap();
    let plan = requested_plan(&commit(frame), request);
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    commit(frame);
    assert_eq!(
        session
            .view()
            .item(THIRD)
            .unwrap()
            .tabs()
            .visible_selected(),
        Some(THIRD)
    );
    assert_eq!(
        session.pane_focus_request(SURFACE).unwrap().item(),
        Some(THIRD)
    );
    assert!(session.view().item(SECOND).unwrap().is_hidden());
}

#[test]
fn visibility_obeys_arrangement_lock_independently_of_close_capability() {
    let mut session = session();
    let mut policy = session.policy().clone();
    policy.set_close_capability(CloseCapability::Disabled);
    let mut frame = session.begin_host_frame().unwrap();
    frame.replace_policy(policy.clone()).unwrap();
    commit(frame);
    set_visibility(&mut session, FIRST, false);
    assert_eq!(session.pending_content_close_plans().count(), 0);

    policy.set_arrangement_locked(true);
    let mut frame = session.begin_host_frame().unwrap();
    frame.replace_policy(policy).unwrap();
    commit(frame);
    let version = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    let hidden = frame.set_item_visibility_current(SECOND, false).unwrap();
    let shown = frame.set_item_visibility_current(FIRST, true).unwrap();
    let report = commit(frame);
    for action in [hidden, shown] {
        assert_eq!(
            report.submitted_action_outcome(action),
            Some(&HostInputOutcome::ProductActionRejected(
                DockspaceActionRejection::PolicyDenied,
            ))
        );
    }
    assert_eq!(session.version(), version);
    assert!(session.view().item_is_hidden(FIRST));
    assert!(!session.view().item_is_hidden(SECOND));
    assert!(report.notification().is_empty());
}

#[test]
fn hidden_items_survive_group_moves_and_reopen_visible_after_close() {
    let mut session = session();
    set_visibility(&mut session, SECOND, false);
    let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let prepared = session.prepare_dock_tabs(group, DockPlacement::Center(DockAnchor::Item(THIRD)));
    assert!(apply_action(&mut session, prepared));
    assert!(session.view().item_is_hidden(SECOND));
    let second = session.view().item(SECOND).unwrap();
    assert_eq!(second.surface(), OTHER_SURFACE);
    assert_eq!(second.tabs().items(), [THIRD, FOURTH, FIRST, SECOND]);

    let mut frame = session.begin_host_frame().unwrap();
    let request = frame.request_close_item_current(SECOND).unwrap();
    let plan = requested_plan(&commit(frame), request);
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    commit(frame);
    assert!(session.view().item(SECOND).is_none());
    assert!(!session.view().item_is_hidden(SECOND));
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .open_item_current(SECOND, DockPlacement::After(FIRST))
        .unwrap();
    commit(frame);
    assert!(!session.view().item_is_hidden(SECOND));
    assert_eq!(
        session.view().item(SECOND).unwrap().tabs().items(),
        [THIRD, FOURTH, FIRST, SECOND]
    );
}

#[test]
fn default_center_and_dock_back_prefer_visible_content_over_a_hidden_layout_slot() {
    for hidden_central in [false, true] {
        for menu in [false, true] {
            let hidden = if hidden_central {
                DockspaceNode::central_tabs([FIRST])
            } else {
                DockspaceNode::tabs([FIRST])
            };
            let main = DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [hidden, DockspaceNode::tabs([SECOND])],
            )
            .unwrap();
            let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(ROOT, main),
            )
            .with_contained(DockspaceContainedLayout::new(
                crate::model::FloatingPresentationId::new(1),
                DockspaceRootLayout::new(OTHER_ROOT, DockspaceNode::tabs([THIRD])),
                LogicalRect::new(100.0, 100.0, 300.0, 200.0).unwrap(),
            ))])
            .unwrap()
            .with_hidden_items([FIRST])
            .unwrap();
            let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
            if menu {
                let mut frame = session.begin_host_frame().unwrap();
                frame
                    .measure_surface(
                        SURFACE,
                        UniformSurfaceMetrics::new(
                            LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
                            LogicalSize::new(40.0, 40.0).unwrap(),
                            80.0,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                commit(frame);
                let mut prepare = session.begin_host_frame().unwrap();
                let plan = prepare.paint_plan(SURFACE).unwrap().unwrap();
                let command = plan
                    .presentation_command(OTHER_ROOT, DockspacePresentationCommandKind::DockCenter)
                    .unwrap()
                    .unwrap();
                assert!(command.is_ready());
                let action = plan.prepare_presentation_command(command).unwrap();
                commit(prepare);
                let mut execute = session.begin_host_frame().unwrap();
                execute.submit_surface_action(action).unwrap();
                commit(execute);
            } else {
                let action = session
                    .presentation_commands(OTHER_ROOT)
                    .dock_back()
                    .unwrap()
                    .into_prepared()
                    .unwrap();
                assert!(apply_action(&mut session, action));
            }
            let view = session.view();
            assert_eq!(view.item(FIRST).unwrap().tabs().items(), [FIRST]);
            assert!(view.item(FIRST).unwrap().is_hidden());
            assert_eq!(view.item(SECOND).unwrap().tabs().items(), [SECOND, THIRD]);
            assert!(view.item(THIRD).unwrap().is_selected());
            assert!(view.root(OTHER_ROOT).is_none());
            assert_eq!(
                view.root(ROOT)
                    .unwrap()
                    .content()
                    .unwrap()
                    .split()
                    .unwrap()
                    .weights()
                    .collect::<Vec<_>>(),
                [0.5, 0.5]
            );
        }
    }
}
