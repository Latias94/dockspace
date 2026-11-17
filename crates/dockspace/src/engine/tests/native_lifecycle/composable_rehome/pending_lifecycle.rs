//! Pending identities and late renderer observations after semantic revocation.

use super::*;

fn pending_nested_source(target: RehomeTarget) -> (BackgroundFixture, ViewportBinding) {
    let (mut fixture, source) = native_tab_source(true, SourceRemainder::None);
    let before = fixture.engine.workspace().clone();
    let version = fixture.engine.version();
    let input = prepared_tab_rehome(&fixture, true, target);
    let staged = submit_test_input(&mut fixture.engine, fixture.presentation_host, input)
        .expect("the exact nested payload must stage before any target output");
    assert!(matches!(
        staged.reduced_inputs()[0].outcome(),
        InputOutcome::ProductActionProcessed { .. }
    ));
    assert_source_retained(&fixture, &before, version, source);
    assert_no_source_release(&staged, source);
    assert!(
        fixture
            .engine
            .pending_mutation
            .as_ref()
            .and_then(PendingMutation::rehome)
            .is_some()
    );
    (fixture, source)
}

fn pending_target_output(fixture: &mut BackgroundFixture) -> HostPresentationOutput {
    let source = background_source_binding(fixture);
    let emitted = emit_target_preview(fixture);
    assert_no_source_release(&emitted, source);
    let outputs = emitted
        .presentation_emissions()
        .iter()
        .map(|emission| emission.output())
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        panic!("only the exact pending target may emit: {outputs:?}");
    };
    assert_eq!(output.surface(), TARGET_SURFACE);
    *output
}

fn assert_cancelled_without_transfer(fixture: &BackgroundFixture, transition: &EngineTransition) {
    assert!(fixture.engine.pending_mutation.is_none());
    assert!(transition.events().iter().any(|event| matches!(
        event.kind(),
        WorkspaceEventKind::PresentationRehomeSettled {
            root: SOURCE_ROOT,
            source_surface: SOURCE_SURFACE,
            target_surface: TARGET_SURFACE,
            result,
            outcome: None,
        } if *result != crate::engine::event::PresentationRehomeResult::Applied
    )));
    for item in [FIRST, SECOND] {
        assert_eq!(
            fixture
                .engine
                .workspace()
                .locate_item_by_id(item)
                .unwrap()
                .root,
            SOURCE_ROOT
        );
    }
}

fn settle_cancelled_target(
    fixture: &mut BackgroundFixture,
    source: ViewportBinding,
    output: HostPresentationOutput,
) {
    let after_cancellation = fixture.engine.workspace().clone();
    let version = fixture.engine.version();
    // This output has not received its renderer terminal yet. Its old semantic
    // authority is revoked, while its original stream still owns settlement.
    let late = observe_target_preview(fixture, output, true);
    assert_eq!(fixture.engine.workspace(), &after_cancellation);
    assert_eq!(fixture.engine.version(), version);
    assert!(fixture.engine.pending_mutation.is_none());
    assert_no_source_release(&late, source);
    assert!(late.events().iter().all(|event| !matches!(
        event.kind(),
        WorkspaceEventKind::CommandCommitted(_)
            | WorkspaceEventKind::PresentationRehomeSettled { .. }
    )));
    let replay = reject_retired_output(fixture, output);
    assert_eq!(fixture.engine.workspace(), &after_cancellation);
    assert_eq!(fixture.engine.version(), version);
    assert_no_source_release(&replay, source);
}

#[test]
fn pending_nested_rehome_reserves_new_root_and_floating_before_product_preparation() {
    const OPENED: ItemId = ItemId::new(9);

    for target in [RehomeTarget::Float, RehomeTarget::Main] {
        let (mut fixture, source) = pending_nested_source(target);
        let pending = fixture
            .engine
            .pending_mutation
            .as_ref()
            .and_then(PendingMutation::rehome)
            .unwrap()
            .clone();
        let crate::interaction::PreviewProof::PresentationRehome { command } =
            pending.preview.proof()
        else {
            panic!("the pending mutation must retain its exact original command");
        };
        let (reserved_root, reserved_floating) = match command {
            WorkspaceCommand::CreateContainedRoot { root, floating, .. } => {
                assert_eq!(target, RehomeTarget::Float);
                (*root, Some(*floating))
            }
            WorkspaceCommand::InstallMainRoot { root, .. } => {
                assert_eq!(target, RehomeTarget::Main);
                (*root, None)
            }
            other => panic!("the nested payload must retain its partial command: {other:?}"),
        };
        assert!(fixture.engine.workspace().root(reserved_root).is_none());
        if let Some(floating) = reserved_floating {
            assert!(
                fixture
                    .engine
                    .workspace()
                    .contained_floating(floating)
                    .is_none()
            );
            assert_eq!(pending.floating, Some(floating));
        }
        let next_root = fixture.engine.prepare_presentation_root_identity().unwrap();
        let next_floating = fixture
            .engine
            .prepare_presentation_floating_identity()
            .unwrap();
        assert!(next_root.get() > reserved_root.get());
        if let Some(floating) = reserved_floating {
            assert!(next_floating.get() > floating.get());
        }
        let native = fixture
            .engine
            .prepare_native_payload_transfer_identities(None, None)
            .expect("prospective native product allocation must use the same identity owner");
        assert_eq!(native.root(), next_root);
        assert_eq!(native.floating(), next_floating);

        let before = fixture.engine.workspace().clone();
        let version = fixture.engine.version();
        let prepared = fixture
            .engine
            .prepare_open_item(OPENED, DockPlacement::Main(TARGET_SURFACE));
        let input = fixture.engine.accept_prepared_action(prepared).unwrap();
        assert_source_retained(&fixture, &before, version, source);
        assert_eq!(
            fixture
                .engine
                .pending_mutation
                .as_ref()
                .and_then(PendingMutation::rehome),
            Some(&pending),
            "preparing another product action cannot replace the pending owner"
        );
        let opened = submit_test_input(&mut fixture.engine, fixture.presentation_host, input)
            .expect("an unrelated item must open using a fresh root identity");
        assert!(matches!(
            opened.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed {
                outcome: DockspaceActionOutcome::Opened { item: OPENED, .. },
                ..
            }
        ));
        assert_eq!(
            fixture
                .engine
                .workspace()
                .locate_item_by_id(OPENED)
                .unwrap()
                .root,
            next_root
        );
        assert!(fixture.engine.workspace().root(reserved_root).is_none());
        assert_no_source_release(&opened, source);
        assert_cancelled_without_transfer(&fixture, &opened);
    }
}

#[test]
fn pending_nested_rehome_rejects_source_hide_then_target_hide_revokes_late_presentation() {
    for target in [RehomeTarget::Float, RehomeTarget::Main] {
        let (mut fixture, source) = pending_nested_source(target);
        let before = fixture.engine.workspace().clone();
        let version = fixture.engine.version();
        let output = pending_target_output(&mut fixture);
        let pending = fixture.engine.pending_mutation.clone();
        let source_hidden = submit_test_input(
            &mut fixture.engine,
            fixture.presentation_host,
            EngineInput::SetItemVisibility {
                expected: version,
                item: FIRST,
                visible: false,
            },
        )
        .expect("a source edit must receive a typed reservation rejection");
        assert!(matches!(
            source_hidden.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionRejected {
                reason: DockspaceActionRejection::Conflict,
                ..
            }
        ));
        assert_source_retained(&fixture, &before, version, source);
        assert_eq!(fixture.engine.pending_mutation, pending);
        assert_no_source_release(&source_hidden, source);

        let target_hidden = submit_test_input(
            &mut fixture.engine,
            fixture.presentation_host,
            EngineInput::SetItemVisibility {
                expected: version,
                item: TARGET,
                visible: false,
            },
        )
        .expect("the unrelated target item remains editable during a pending rehome");
        assert!(matches!(
            target_hidden.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed { .. }
        ));
        assert!(fixture.engine.workspace().item_is_hidden(TARGET));
        assert!(!fixture.engine.workspace().item_is_hidden(FIRST));
        assert_eq!(
            fixture.engine.workspace().item_multiset(),
            before.item_multiset()
        );
        assert_no_source_release(&target_hidden, source);
        assert_cancelled_without_transfer(&fixture, &target_hidden);
        settle_cancelled_target(&mut fixture, source, output);
    }
}

#[test]
fn pending_nested_rehome_policy_or_workspace_replacement_revokes_late_presentation() {
    for target in [RehomeTarget::Float, RehomeTarget::Main] {
        for reset in [false, true] {
            let (mut fixture, source) = pending_nested_source(target);
            let before = fixture.engine.workspace().clone();
            let version = fixture.engine.version();
            let output = pending_target_output(&mut fixture);
            let input = if reset {
                EngineInput::ReplaceWorkspace(before.clone().into_builder().build().unwrap())
            } else {
                let mut policy = fixture.engine.policy().clone();
                policy.set_allow_native_surfaces(false);
                EngineInput::ReplacePolicy {
                    expected: version,
                    policy,
                }
            };
            let cancelled =
                submit_test_input(&mut fixture.engine, fixture.presentation_host, input).expect(
                    "an authoritative replacement must cancel without transferring the source",
                );
            if reset {
                assert!(matches!(
                    cancelled.reduced_inputs()[0].outcome(),
                    InputOutcome::WorkspaceReplaced { .. }
                ));
                assert_ne!(fixture.engine.version().epoch(), version.epoch());
                // Reset retires the old child incarnation through its own
                // lifecycle; the old preview must add no later retirement.
            } else {
                assert!(matches!(
                    cancelled.reduced_inputs()[0].outcome(),
                    InputOutcome::PolicyReplaced { changed: true, .. }
                ));
                assert_eq!(fixture.engine.version().epoch(), version.epoch());
                assert_no_source_release(&cancelled, source);
            }
            assert_eq!(
                fixture.engine.workspace().item_multiset(),
                before.item_multiset()
            );
            assert_cancelled_without_transfer(&fixture, &cancelled);
            settle_cancelled_target(&mut fixture, source, output);
        }
    }
}
