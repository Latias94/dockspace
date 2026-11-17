use super::*;

fn split_session(central: bool) -> DockspaceSession {
    let source = if central {
        DockspaceNode::central_tabs([FIRST, SECOND])
    } else {
        DockspaceNode::tabs([FIRST, SECOND])
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::equal_split(
                crate::model::DockspaceAxis::Horizontal,
                [source, DockspaceNode::tabs([THIRD, FOURTH])],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut policy = DockPolicy::default();
    policy.set_close_capability(CloseCapability::DeferredAllowed);
    DockspaceSession::from_layout(layout, policy).unwrap()
}

fn open_group_plan(session: &mut DockspaceSession) -> DockspaceClosePlan {
    let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let group_key = crate::ids::NodeKey(group.source.node());
    let prepared = session.prepare_close_tabs(group);
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_close_request(prepared).unwrap();
    let report = commit(frame);
    let plan = requested_plan(&report, submitted);
    assert_eq!(
        plan.target(),
        ClosePlanTarget::Tabs {
            root: ROOT,
            group: group_key
        }
    );
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [FIRST, SECOND]
    );
    plan
}

#[test]
fn group_close_is_atomic_and_preserves_sibling_groups_and_central_anchors() {
    for central in [false, true] {
        let mut session = split_session(central);
        let plan = open_group_plan(&mut session);
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        commit(frame);
        assert!(session.view().item(FIRST).is_some());
        assert!(session.view().item(SECOND).is_some());
        let mut frame = session.begin_host_frame().unwrap();
        let submitted = frame
            .resolve_close(
                plan.request(),
                plan.items()[1].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        let report = commit(frame);
        assert!(
            matches!(report.submitted_action_outcome(submitted), Some(HostInputOutcome::CloseDecisionProcessed {
            application: Some(Ok(DockspaceCloseOutcome::TabsClosed { root: ROOT, items })), changed: true, ..
        }) if items == &[FIRST, SECOND])
        );
        session.workspace().validate().unwrap();
        assert!(session.view().item(FIRST).is_none());
        assert!(session.view().item(SECOND).is_none());
        assert!(session.view().item(THIRD).is_some());
        assert!(session.view().item(FOURTH).is_some());
        assert_eq!(
            session.view().root(ROOT).unwrap().central().is_some(),
            central
        );
    }
}

#[test]
fn group_close_veto_keeps_every_member_and_replayed_decisions_are_inert() {
    let mut session = split_session(false);
    let plan = open_group_plan(&mut session);
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    frame
        .resolve_close(plan.request(), plan.items()[1].token(), CloseDecision::Veto)
        .unwrap();
    commit(frame);
    assert_eq!(session.pending_content_close_plans().count(), 0);
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(
            plan.request(),
            plan.items()[1].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    commit(frame);
    assert_eq!(session.workspace().item_multiset().len(), 4);
}

#[test]
fn deferred_group_close_survives_sibling_selection_but_not_membership_changes() {
    for alter_membership in [false, true] {
        let mut session = split_session(false);
        let plan = open_group_plan(&mut session);
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        frame
            .resolve_close(
                plan.request(),
                plan.items()[1].token(),
                CloseDecision::Deferred,
            )
            .unwrap();
        commit(frame);
        let pending = session.pending_content_close_plans().next().unwrap();
        let continuation = pending.items()[1].deferred_token().unwrap();
        if alter_membership {
            let action = session.prepare_dock_item(SECOND, DockPlacement::After(THIRD));
            assert!(apply_action(&mut session, action));
        } else {
            let action = session.prepare_select_item(FOURTH);
            assert!(apply_action(&mut session, action));
            let action = session.prepare_open_item(ItemId::new(9), DockPlacement::After(THIRD));
            assert!(apply_action(&mut session, action));
        }
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .continue_deferred_close(plan.request(), continuation, DeferredCloseDecision::Allow)
            .unwrap();
        commit(frame);
        assert_eq!(session.view().item(FIRST).is_some(), alter_membership);
        assert_eq!(session.view().item(SECOND).is_some(), alter_membership);
        assert!(session.view().item(THIRD).is_some());
        assert!(session.view().item(FOURTH).is_some());
    }
}

#[test]
fn group_close_pending_approval_does_not_follow_a_group_to_another_root() {
    let mut session = session();
    let plan = open_group_plan(&mut session);
    let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let action = session.prepare_dock_tabs(group, DockPlacement::After(THIRD));
    assert!(apply_action(&mut session, action));
    assert_eq!(session.pending_content_close_plans().count(), 0);
    let mut frame = session.begin_host_frame().unwrap();
    for item in plan.items() {
        frame
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
    }
    commit(frame);
    assert_eq!(session.workspace().item_multiset().len(), 4);
    assert_eq!(session.view().item(FIRST).unwrap().root(), OTHER_ROOT);
    assert_eq!(session.view().item(SECOND).unwrap().root(), OTHER_ROOT);
}

#[test]
fn approved_group_close_reports_the_original_roster_after_local_reorder() {
    let mut session = split_session(false);
    let plan = open_group_plan(&mut session);
    let action = session.prepare_dock_item(SECOND, DockPlacement::Before(FIRST));
    assert!(apply_action(&mut session, action));
    assert_eq!(session.pending_content_close_plans().count(), 1);
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    let submitted = frame
        .resolve_close(
            plan.request(),
            plan.items()[1].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    let report = commit(frame);
    assert!(
        matches!(report.submitted_action_outcome(submitted), Some(HostInputOutcome::CloseDecisionProcessed {
        application: Some(Ok(DockspaceCloseOutcome::TabsClosed { root: ROOT, items })), ..
    }) if items == &[FIRST, SECOND])
    );
}

#[test]
fn group_close_preparation_retains_the_original_capture_version_and_session() {
    let mut session = split_session(false);
    let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let version = group.expected_version();
    let action = session.prepare_dock_item(FIRST, DockPlacement::After(THIRD));
    assert!(apply_action(&mut session, action));
    let prepared = session.prepare_close_tabs(group);
    assert_eq!(prepared.expected_version(), version);
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_close_request(prepared).unwrap();
    let report = commit(frame);
    assert!(matches!(
        report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::StaleRejected { .. })
    ));
    assert_eq!(session.workspace().item_multiset().len(), 4);

    let foreign = split_session(false);
    let group = foreign.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let prepared = session.prepare_close_tabs(group);
    let mut frame = session.begin_host_frame().unwrap();
    assert!(frame.submit_prepared_close_request(prepared).is_err());
}

#[test]
fn replaced_workspace_cannot_reuse_old_group_close_approvals() {
    let mut session = split_session(false);
    let plan = open_group_plan(&mut session);
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([FIRST, SECOND, THIRD, FOURTH])),
    )])
    .unwrap();
    let reset = session.prepare_layout_reset(layout).unwrap();
    commit(session.begin_layout_reset_frame(&reset).unwrap());
    assert_eq!(session.pending_content_close_plans().count(), 0);
    let before = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    for item in plan.items() {
        frame
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
    }
    commit(frame);
    assert_eq!(session.version(), before);
    assert_eq!(session.workspace().item_multiset().len(), 4);
}
