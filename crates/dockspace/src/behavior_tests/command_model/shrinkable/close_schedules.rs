use super::*;
use crate::close::{CloseDecision, DeferredCloseDecision, DeferredCloseToken};
use crate::model::{DockAnchor, DockPlacement};
use crate::policy::{CloseCapability, DockItemRule, DockPolicy};
use crate::runtime::{
    DockspaceCloseOutcome, DockspaceClosePlan, DockspaceCloseResolution, DockspaceHostFrame,
    DockspaceSession, HostFrameReport, HostInputOutcome, SurfaceUnavailableReason,
};

fn commit(mut frame: DockspaceHostFrame<'_>) -> HostFrameReport {
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap()
}

fn request(session: &mut DockspaceSession, item: ItemId, group: bool) -> DockspaceClosePlan {
    let prepared = if group {
        let source = session.capture_tabs_group(DockAnchor::Item(item)).unwrap();
        session.prepare_close_tabs(source)
    } else {
        session.prepare_close_item(item)
    };
    let before = session.workspace().clone();
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_prepared_close_request(prepared).unwrap();
    let report = commit(frame);
    let [
        HostInputOutcome::CloseRequested {
            plan,
            reused: false,
            ..
        },
    ] = report.inputs()
    else {
        panic!(
            "one model-owned request must produce one fresh close plan: {:?}",
            report.inputs()
        );
    };
    assert_eq!(session.workspace(), &before);
    plan.clone()
}

pub(super) fn close_immediately(
    workspace: &mut Workspace,
    expected: &[ItemId],
    item: ItemId,
    group: bool,
) {
    let mut session =
        DockspaceSession::from_workspace_for_test(workspace.clone(), DockPolicy::default())
            .unwrap();
    let plan = request(&mut session, item, group);
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>()
            .as_slice(),
        expected
    );
    for (index, item) in plan.items().iter().enumerate() {
        let before = session.workspace().clone();
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
        let report = commit(frame);
        let final_vote = index + 1 == plan.items().len();
        assert_eq!(report.workspace_changed(), final_vote);
        if final_vote {
            assert!(matches!(
                report.inputs(),
                [HostInputOutcome::CloseDecisionProcessed {
                    application: Some(Ok(_)),
                    changed: true,
                    ..
                }]
            ));
        } else {
            assert_eq!(
                session.workspace(),
                &before,
                "partial approvals cannot remove a member"
            );
        }
    }
    *workspace = session.workspace().clone();
}

#[derive(Clone, Copy, Debug)]
enum Edit {
    Select { item: ItemId },
    Hide { visible: bool },
    Reorder,
    Move { away: bool },
    Open { into_target: bool },
    CloseUnrelated,
    PolicyRoundTrip,
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (1..=4_u64).prop_map(|item| Edit::Select {
            item: ItemId::new(item)
        }),
        any::<bool>().prop_map(|visible| Edit::Hide { visible }),
        Just(Edit::Reorder),
        any::<bool>().prop_map(|away| Edit::Move { away }),
        any::<bool>().prop_map(|into_target| Edit::Open { into_target }),
        Just(Edit::CloseUnrelated),
        Just(Edit::PolicyRoundTrip),
    ]
}

fn policy() -> DockPolicy {
    let mut policy = DockPolicy::default();
    policy.set_close_capability(CloseCapability::DeferredAllowed);
    policy
}

fn model_step(
    model: &mut Semantics,
    session: &mut DockspaceSession,
    action: ModelAction,
    apply: impl FnOnce(&mut DockspaceHostFrame<'_>),
) -> bool {
    let Some(Prediction::Success { next, changed }) = model.predict(action) else {
        return false;
    };
    let mut frame = session.begin_host_frame().unwrap();
    apply(&mut frame);
    let report = commit(frame);
    assert!(
        matches!(report.inputs(), [HostInputOutcome::ProductActionApplied(_)]),
        "model-valid action {action:?} must be applied: {:?}",
        report.inputs()
    );
    assert_eq!(report.workspace_changed(), changed, "{action:?}");
    *model = next;
    model.assert_workspace(session.workspace(), action);
    changed
}

fn edit(model: &mut Semantics, session: &mut DockspaceSession, edit: Edit, group: bool) -> bool {
    let target = ItemId::new(2);
    match edit {
        Edit::Select { item } => {
            model_step(model, session, ModelAction::Select { item }, |frame| {
                frame.select_item_current(item).unwrap();
            });
            false
        }
        Edit::Hide { visible } => {
            model_step(
                model,
                session,
                ModelAction::Visibility {
                    item: target,
                    visible,
                },
                |frame| {
                    frame.set_item_visibility_current(target, visible).unwrap();
                },
            );
            false
        }
        Edit::Reorder => {
            model_step(
                model,
                session,
                ModelAction::Reorder {
                    item: target,
                    insertion: 0,
                },
                |frame| {
                    let first = frame
                        .view()
                        .item(target)
                        .unwrap()
                        .tabs()
                        .items()
                        .next()
                        .unwrap();
                    frame
                        .dock_item_current(target, DockPlacement::Before(first))
                        .unwrap();
                },
            );
            false
        }
        Edit::Move { away } => {
            let destination = ItemId::new(if away { 3 } else { 1 });
            model_step(
                model,
                session,
                ModelAction::CenterMove {
                    item: target,
                    target_item: destination,
                },
                |frame| {
                    frame
                        .dock_item_current(
                            target,
                            DockPlacement::Center(DockAnchor::Item(destination)),
                        )
                        .unwrap();
                },
            )
        }
        Edit::Open { into_target } => {
            let destination = ItemId::new(if into_target { 1 } else { 3 });
            let changed = model_step(
                model,
                session,
                ModelAction::Open {
                    item: ItemId::new(9),
                    target_item: destination,
                },
                |frame| {
                    frame
                        .open_item_current(
                            ItemId::new(9),
                            DockPlacement::Center(DockAnchor::Item(destination)),
                        )
                        .unwrap();
                },
            );
            group && into_target && changed
        }
        Edit::CloseUnrelated => {
            let action = ModelAction::Close {
                item: ItemId::new(8),
                group: false,
            };
            if let Some(Prediction::Success { next, .. }) = model.predict(action) {
                let plan = request(session, ItemId::new(8), false);
                let mut frame = session.begin_host_frame().unwrap();
                frame
                    .resolve_close(
                        plan.request(),
                        plan.items()[0].token(),
                        CloseDecision::Allow,
                    )
                    .unwrap();
                assert!(commit(frame).workspace_changed());
                *model = next;
                model.assert_workspace(session.workspace(), action);
            }
            false
        }
        Edit::PolicyRoundTrip => {
            let mut changed = policy();
            let mut rule = DockItemRule::default();
            rule.set_close_capability(Some(CloseCapability::Immediate));
            changed.set_item_rule(target, rule);
            for policy in [changed, policy()] {
                let before = session.workspace().clone();
                let mut frame = session.begin_host_frame().unwrap();
                frame.replace_policy(policy).unwrap();
                let report = commit(frame);
                assert!(matches!(
                    report.inputs(),
                    [HostInputOutcome::PolicyReplaced { changed: true, .. }]
                ));
                assert_eq!(session.workspace(), &before);
            }
            true
        }
    }
}

fn run_schedule(group: bool, deferred: bool, veto: bool, edits: &[Edit]) {
    let target = ItemId::new(2);
    let action = ModelAction::Close {
        item: target,
        group,
    };
    let mut model = Semantics::initial();
    let mut session =
        DockspaceSession::from_workspace_for_test(initial_workspace(), policy()).unwrap();
    let expected_items = model.items_to_close(target, group);
    let plan = request(&mut session, target, group);
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        expected_items
    );
    let last = *plan.items().last().unwrap();
    let mut continuation: Option<DeferredCloseToken> = None;
    for item in plan.items() {
        if *item == last && !deferred {
            continue;
        }
        let before = session.workspace().clone();
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .resolve_close(
                plan.request(),
                item.token(),
                if *item == last {
                    CloseDecision::Deferred
                } else {
                    CloseDecision::Allow
                },
            )
            .unwrap();
        let report = commit(frame);
        assert!(!report.workspace_changed());
        assert_eq!(session.workspace(), &before);
        if let [
            HostInputOutcome::CloseDecisionProcessed {
                resolution:
                    DockspaceCloseResolution::Deferred {
                        continuation: token,
                        ..
                    },
                ..
            },
        ] = report.inputs()
        {
            continuation = Some(*token);
        }
    }
    assert_eq!(continuation.is_some(), deferred);
    let mut stale = false;
    for &operation in edits {
        stale |= edit(&mut model, &mut session, operation, group);
        model.assert_workspace(session.workspace(), action);
    }
    let before = session.workspace().clone();
    let mut frame = session.begin_host_frame().unwrap();
    if let Some(token) = continuation {
        frame
            .continue_deferred_close(
                plan.request(),
                token,
                if veto {
                    DeferredCloseDecision::Veto
                } else {
                    DeferredCloseDecision::Allow
                },
            )
            .unwrap();
    } else {
        frame
            .resolve_close(
                plan.request(),
                last.token(),
                if veto {
                    CloseDecision::Veto
                } else {
                    CloseDecision::Allow
                },
            )
            .unwrap();
    }
    let report = commit(frame);
    let applies = !stale && !veto;
    assert_eq!(
        report.workspace_changed(),
        applies,
        "group={group}, deferred={deferred}, veto={veto}, edits={edits:?}"
    );
    if applies {
        let [
            HostInputOutcome::CloseDecisionProcessed {
                resolution: DockspaceCloseResolution::Approved { request },
                application: Some(Ok(outcome)),
                changed: true,
                ..
            },
        ] = report.inputs()
        else {
            panic!(
                "the last valid approval must commit once: {:?}",
                report.inputs()
            );
        };
        assert_eq!(*request, plan.request());
        match outcome {
            DockspaceCloseOutcome::ItemClosed { item, root } if !group => {
                assert_eq!((*item, *root), (target, ROOT));
            }
            DockspaceCloseOutcome::TabsClosed { root, items } if group => {
                assert_eq!(*root, ROOT);
                assert_eq!(items, &expected_items);
            }
            _ => panic!("the close must retain its original scope: {outcome:?}"),
        }
        let Some(Prediction::Success { next, .. }) = model.predict(action) else {
            panic!("closing a current scope remains applicable")
        };
        model = next;
    } else {
        assert_eq!(session.workspace(), &before);
        let [
            HostInputOutcome::CloseDecisionProcessed {
                application: None,
                changed: false,
                resolution,
                ..
            },
        ] = report.inputs()
        else {
            panic!("an ineligible close cannot apply: {:?}", report.inputs());
        };
        match resolution {
            DockspaceCloseResolution::Inert(_) if stale => {}
            DockspaceCloseResolution::Vetoed { request, item } if !stale && veto => {
                assert_eq!((*request, *item), (plan.request(), last.item()));
            }
            _ => panic!("unexpected terminal decision: {resolution:?}"),
        }
    }
    model.assert_workspace(session.workspace(), action);

    model_step(
        &mut model,
        &mut session,
        ModelAction::Open {
            item: target,
            target_item: ItemId::new(3),
        },
        |frame| {
            frame
                .open_item_current(
                    target,
                    DockPlacement::Center(DockAnchor::Item(ItemId::new(3))),
                )
                .unwrap();
        },
    );
    let before = session.workspace().clone();
    let mut frame = session.begin_host_frame().unwrap();
    for item in plan.items() {
        frame
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
    }
    if let Some(token) = continuation {
        frame
            .continue_deferred_close(plan.request(), token, DeferredCloseDecision::Allow)
            .unwrap();
    }
    let report = commit(frame);
    assert!(!report.workspace_changed());
    assert_eq!(
        report.inputs().len(),
        plan.items().len() + usize::from(deferred)
    );
    assert!(report.inputs().iter().all(|input| matches!(
        input,
        HostInputOutcome::CloseDecisionProcessed {
            resolution: DockspaceCloseResolution::Inert(_),
            application: None,
            changed: false,
            ..
        }
    )));
    assert_eq!(session.workspace(), &before);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn approval_schedules_follow_independent_scope_semantics(
        group in any::<bool>(), deferred in any::<bool>(), veto in any::<bool>(),
        edits in prop::collection::vec(edit_strategy(), 0..25),
    ) {
        run_schedule(group, deferred, veto, &edits);
    }
}

#[test]
fn departed_and_returned_members_never_revive_an_old_close_approval() {
    for group in [false, true] {
        for deferred in [false, true] {
            run_schedule(
                group,
                deferred,
                false,
                &[Edit::Move { away: true }, Edit::Move { away: false }],
            );
            run_schedule(group, deferred, false, &[Edit::PolicyRoundTrip]);
            run_schedule(
                group,
                deferred,
                false,
                &[
                    Edit::Reorder,
                    Edit::Hide { visible: false },
                    Edit::CloseUnrelated,
                ],
            );
        }
    }
}
