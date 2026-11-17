//! Body activation carries its own close authority when the X is clipped away.

use super::*;
use crate::model::{DockspaceContainedLayout, FloatingPresentationId};
use crate::policy::{TabBarInteraction, TabBarPolicy, TabBarVisibility};

#[derive(Debug, Clone, Copy)]
enum Delivery {
    CurrentCandidate,
    DeferredWitness,
}

fn clipped_session() -> DockspaceSession {
    let mut session = session();
    let config = DockPresentationConfig::builder()
        .tab_min_width(400.0)
        .tab_max_width(400.0)
        .build()
        .unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    frame.replace_presentation_config(config).unwrap();
    commit(frame);
    session
}

fn metrics() -> UniformSurfaceMetrics {
    UniformSurfaceMetrics::new(
        LogicalRect::new(0.0, 0.0, 640.0, 320.0).unwrap(),
        LogicalSize::new(32.0, 24.0).unwrap(),
        360.0,
    )
    .unwrap()
}

fn capture(session: &mut DockspaceSession, delivery: Delivery) -> PreparedSurfaceAction {
    let mut frame = session.begin_host_frame().unwrap();
    frame.measure_surface(SURFACE, metrics()).unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let partial = plan.tabs().find(|tab| tab.item() == Some(SECOND)).unwrap();
    assert!(partial.visible_bounds().width() < partial.full_bounds().width());
    assert!(partial.operable());
    assert_eq!(partial.close_bounds(), None);
    assert!(plan.prepare_tab_close(SECOND).is_none());
    assert!(plan.receiver_for_tab_close(partial).is_none());
    assert!(plan.receiver_for_tab_body(partial).is_some());
    let action = plan.prepare_tab_body_close(SECOND).unwrap();
    match delivery {
        Delivery::CurrentCandidate => {
            commit(frame);
        }
        Delivery::DeferredWitness => drop(frame),
    }
    action
}

fn submit(
    session: &mut DockspaceSession,
    action: PreparedSurfaceAction,
    delivery: Delivery,
) -> HostFrameReport {
    let mut frame = session.begin_host_frame().unwrap();
    match delivery {
        Delivery::CurrentCandidate => frame.submit_surface_action(action).unwrap(),
        Delivery::DeferredWitness => frame
            .submit_deferred_surface_action(action.into_deferred())
            .unwrap(),
    }
    commit(frame)
}

fn body_close_plan(report: &HostFrameReport) -> DockspaceClosePlan {
    let [
        HostInputOutcome::CloseRequested {
            plan,
            origin: HostCloseRequestOrigin::Interaction,
            reused: false,
        },
    ] = report.inputs()
    else {
        panic!("the exact body activation must request one interaction close: {report:?}");
    };
    assert!(!report.workspace_changed());
    assert_eq!(plan.target(), ClosePlanTarget::Item { item: SECOND });
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.items()[0].item(), SECOND);
    plan.clone()
}

#[test]
fn clipped_tab_body_close_uses_the_existing_deferred_decision_workflow() {
    for delivery in [Delivery::CurrentCandidate, Delivery::DeferredWitness] {
        let mut session = clipped_session();
        let action = capture(&mut session, delivery);
        let before = session.version();
        let plan = body_close_plan(&submit(&mut session, action, delivery));
        assert_eq!(session.version(), before);
        assert!(session.view().item(SECOND).is_some());

        let mut frame = session.begin_host_frame().unwrap();
        let decision = frame
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Deferred,
            )
            .unwrap();
        let report = commit(frame);
        let Some(HostInputOutcome::CloseDecisionProcessed {
            resolution:
                DockspaceCloseResolution::Deferred {
                    request,
                    item: SECOND,
                    continuation,
                },
            changed: false,
            application: None,
            ..
        }) = report.submitted_action_outcome(decision)
        else {
            panic!("the body close must retain the ordinary deferred token: {report:?}");
        };
        assert_eq!(*request, plan.request());
        let continuation = *continuation;
        assert_eq!(session.version(), before);
        assert!(session.view().item(SECOND).is_some());
        let mut frame = session.begin_host_frame().unwrap();
        let allowed = frame
            .continue_deferred_close(plan.request(), continuation, DeferredCloseDecision::Allow)
            .unwrap();
        let report = commit(frame);
        assert!(matches!(
            report.submitted_action_outcome(allowed),
            Some(HostInputOutcome::CloseDecisionProcessed {
                application: Some(Ok(DockspaceCloseOutcome::ItemClosed {
                    item: SECOND,
                    root: ROOT,
                })),
                changed: true,
                ..
            })
        ));
        assert!(report.workspace_changed(), "{delivery:?}: {report:?}");
        assert!(session.view().item(SECOND).is_none());
        for item in [FIRST, THIRD, FOURTH] {
            assert!(session.view().item(item).is_some());
        }
        assert_eq!(
            session
                .view()
                .item(FIRST)
                .unwrap()
                .tabs()
                .items()
                .collect::<Vec<_>>(),
            &[FIRST]
        );
        assert_eq!(session.pending_content_close_plans().count(), 0);
    }
}

#[test]
fn tab_body_close_requests_are_rollbackable_and_veto_keeps_the_item() {
    let mut session = clipped_session();
    let action = capture(&mut session, Delivery::DeferredWitness).into_deferred();
    let before = session.version();
    let mut discarded = session.begin_host_frame().unwrap();
    discarded
        .submit_deferred_surface_action(action.clone())
        .unwrap();
    drop(discarded);
    assert_eq!(session.version(), before);
    assert_eq!(session.pending_content_close_plans().count(), 0);

    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_deferred_surface_action(action).unwrap();
    let plan = body_close_plan(&commit(frame));
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(plan.request(), plan.items()[0].token(), CloseDecision::Veto)
        .unwrap();
    let report = commit(frame);
    assert!(!report.workspace_changed());
    assert_eq!(session.version(), before);
    assert_eq!(session.pending_content_close_plans().count(), 0);
    assert_eq!(
        session
            .view()
            .item(SECOND)
            .unwrap()
            .tabs()
            .items()
            .collect::<Vec<_>>(),
        &[FIRST, SECOND]
    );
}

#[test]
fn tab_body_close_is_unavailable_when_close_or_tab_bar_policy_disables_it() {
    for policy in [
        {
            let mut policy = DockPolicy::default();
            policy.set_close_capability(CloseCapability::Disabled);
            policy
        },
        {
            let mut policy = DockPolicy::default();
            policy.set_tab_bar(TabBarPolicy::new(
                TabBarVisibility::Visible,
                TabBarInteraction::Disabled,
            ));
            policy
        },
        {
            let mut policy = DockPolicy::default();
            policy.set_tab_bar(TabBarPolicy::new(
                TabBarVisibility::Hidden,
                TabBarInteraction::Enabled,
            ));
            policy
        },
    ] {
        let mut session = clipped_session();
        let mut frame = session.begin_host_frame().unwrap();
        frame.replace_policy(policy).unwrap();
        commit(frame);
        let mut frame = session.begin_host_frame().unwrap();
        frame.measure_surface(SURFACE, metrics()).unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        for item in [FIRST, SECOND] {
            assert!(plan.prepare_tab_body_close(item).is_none());
        }
        drop(frame);
        assert_eq!(session.pending_content_close_plans().count(), 0);
    }
}

#[test]
fn a_contained_window_covering_the_tab_body_cannot_authorize_body_close() {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs([FIRST, SECOND])),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(OTHER_ROOT, DockspaceNode::tabs([THIRD])),
        LogicalRect::new(0.0, 0.0, 640.0, 180.0).unwrap(),
    ))])
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    frame.measure_surface(SURFACE, metrics()).unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    for item in [FIRST, SECOND] {
        let tab = plan.tabs().find(|tab| tab.item() == Some(item)).unwrap();
        assert!(!tab.operable(), "the contained frame must cover {item:?}");
        assert!(plan.prepare_tab_body_close(item).is_none());
    }
    assert!(plan.prepare_tab_body_close(THIRD).is_some());
}

#[derive(Debug, Clone, Copy)]
enum Invalidation {
    ClosePolicy,
    SourceMove,
    PresentationConfig,
}

#[test]
fn tab_body_close_rejects_a_stale_candidate_or_deferred_witness() {
    for delivery in [Delivery::CurrentCandidate, Delivery::DeferredWitness] {
        for invalidation in [
            Invalidation::ClosePolicy,
            Invalidation::SourceMove,
            Invalidation::PresentationConfig,
        ] {
            let mut session = clipped_session();
            let action = capture(&mut session, delivery);
            let mut frame = session.begin_host_frame().unwrap();
            match invalidation {
                Invalidation::ClosePolicy => {
                    let mut policy = DockPolicy::default();
                    policy.set_close_capability(CloseCapability::Disabled);
                    frame.replace_policy(policy).unwrap();
                }
                Invalidation::SourceMove => {
                    frame
                        .dock_item_current(SECOND, DockPlacement::After(THIRD))
                        .unwrap();
                }
                Invalidation::PresentationConfig => {
                    frame
                        .replace_presentation_config(DockPresentationConfig::default())
                        .unwrap();
                }
            }
            commit(frame);
            let before = session.version();
            let report = submit(&mut session, action, delivery);
            match (delivery, invalidation) {
                (Delivery::CurrentCandidate, Invalidation::PresentationConfig) => {
                    // Configuration revokes scene authority without changing
                    // WorkspaceVersion. Candidate interaction rejections do
                    // not produce a product-level close request.
                    assert!(report.inputs().is_empty(), "{report:?}");
                }
                _ => assert!(
                    matches!(report.inputs(), [HostInputOutcome::StaleRejected { .. }]),
                    "{delivery:?}/{invalidation:?}: {report:?}"
                ),
            }
            assert!(!report.workspace_changed());
            assert_eq!(session.version(), before);
            assert_eq!(session.pending_content_close_plans().count(), 0);
            for item in [FIRST, SECOND, THIRD, FOURTH] {
                assert!(session.view().item(item).is_some());
            }
        }
    }
}
