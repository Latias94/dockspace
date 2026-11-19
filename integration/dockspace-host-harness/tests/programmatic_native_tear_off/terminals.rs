//! Accepted native transfers terminate independently of compensating window cleanup.

use super::*;
use dockspace::runtime::{
    DockspacePresentationTransitionId, DockspacePresentationTransitionResult,
};

fn present(
    session: &mut DockspaceSession,
    binding: NativeSurfaceBinding,
    bounds: PhysicalRect,
) -> HostFrameReport {
    let output = paint(session, binding, bounds);
    session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    idle(session)
}

fn assert_failed(
    report: &HostFrameReport,
    expected: DockspacePresentationTransitionId,
    result: DockspacePresentationTransitionResult,
) {
    let [terminal] = report.presentation_transitions() else {
        panic!("an accepted native operation must terminate exactly once: {report:?}");
    };
    assert_eq!(terminal.id(), expected);
    assert_eq!(terminal.result(), result);
    assert!(terminal.outcome().is_none());
    assert_eq!(
        report.notification().presentation_transitions(),
        report.presentation_transitions(),
    );
    assert!(!report.notification().is_empty());
    assert!(report.native_admissions().is_empty());
}

#[test]
fn retiring_a_retained_empty_surface_does_not_roll_back_create_cancellation() {
    let mut fixture = Fixture::new();
    let required = SurfaceId::new(10);
    let mut frame = native_frame(&mut fixture.session);
    frame
        .attach_empty_surface_current(required, RootId::new(10))
        .unwrap();
    finish(frame);
    fixture.session.retain_host_surface(required).unwrap();

    let action = fixture
        .session
        .prepare_move_to_new_window(DockMoveSource::Item(B), placement());
    let mut request = fixture.request(action);
    let ack = request.accept();
    fixture.observe(&request, NativeCreateReadiness::Created, Some(ack));

    let mut frame = native_frame(&mut fixture.session);
    assert!(frame.cancel_next_native_create().unwrap());
    let retirement = frame.retire_empty_surface_current(required).unwrap();
    let mut report = finish(frame);
    assert!(matches!(
        report.submitted_action_outcome(retirement),
        Some(HostInputOutcome::LogicalSurfaceRejected {
            surface,
            reason: dockspace::runtime::LogicalSurfaceRejection::SurfaceRequired,
        }) if *surface == required
    ));
    assert_failed(
        &report,
        request.transition,
        DockspacePresentationTransitionResult::Cancelled,
    );
    assert!(fixture.session.view().surface(required).is_some());
    assert!(!fixture.session.has_pending_native_create());
    let [close] = report.take_native_effects().try_into().unwrap();
    assert!(
        matches!(close.operation(), NativeEffectOperation::CompensatingClose { binding } if *binding == request.binding)
    );
    assert!(matches!(
        close.accepted(),
        Some(NativeEffectAcknowledgement::Close(_))
    ));
}

#[test]
fn late_required_source_surface_removal_reports_the_original_request_rejection() {
    let mut fixture = Fixture::configured(3, false);
    let action = fixture
        .session
        .prepare_move_to_new_window(DockMoveSource::Item(B), placement());
    let mut source = fixture.request(action);
    let ack = source.accept();
    fixture.observe(&source, NativeCreateReadiness::Prepared, Some(ack));
    assert_eq!(
        present(&mut fixture.session, source.binding, source.placement).native_admissions(),
        &[source.binding],
    );
    present(&mut fixture.session, fixture.root, root_bounds());
    present(&mut fixture.session, source.binding, source.placement);

    let action = fixture
        .session
        .prepare_float_root(CONTAINED_ROOT, source.binding.surface());
    let mut frame = native_frame(&mut fixture.session);
    frame.submit_prepared_action(action).unwrap();
    finish(frame);
    present(&mut fixture.session, source.binding, source.placement);

    let action = fixture
        .session
        .prepare_move_to_new_window(DockMoveSource::Root(source.root), placement())
        .preserving_surfaces([source.binding.surface()]);
    let mut frame = native_frame(&mut fixture.session);
    let submitted = frame.submit_prepared_action(action).unwrap();
    let mut report = finish(frame);
    let Some(HostInputOutcome::ProductPresentationActionRequested { transition, .. }) =
        report.submitted_action_outcome(submitted)
    else {
        panic!("the second transfer must first return its accepted request");
    };
    let expected = *transition;
    assert!(report.presentation_transitions().is_empty());
    let [effect] = report.take_native_effects().try_into().unwrap();
    let NativeEffectOperation::CreateWindow {
        binding, placement, ..
    } = effect.operation()
    else {
        panic!("the accepted transfer must request its native child");
    };
    let (child, bounds) = (*binding, *placement);
    let Some(NativeEffectAcknowledgement::Create(ack)) = effect.accepted() else {
        panic!("the create retains its exact readiness acknowledgement");
    };

    let mut frame = native_frame(&mut fixture.session);
    let requested = frame.request_close_item_current(CONTAINED).unwrap();
    let report = finish(frame);
    let Some(HostInputOutcome::CloseRequested { plan, .. }) =
        report.submitted_action_outcome(requested)
    else {
        panic!("the sibling content must enter its ordinary close plan");
    };
    let mut frame = native_frame(&mut fixture.session);
    frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            dockspace::close::CloseDecision::Allow,
        )
        .unwrap();
    finish(frame);
    assert!(fixture.session.view().item(CONTAINED).is_none());
    assert!(fixture.session.has_pending_native_create());

    fixture
        .session
        .report_managed_native_snapshot(
            [
                (fixture.root, root_facts()),
                (source.binding, window_facts(source.placement)),
                (
                    child,
                    window_facts(bounds)
                        .with_create_readiness(NativeCreateReadiness::Prepared, Some(ack)),
                ),
            ],
            work_areas(),
        )
        .unwrap();
    let mut report = idle(&mut fixture.session);
    assert_failed(
        &report,
        expected,
        DockspacePresentationTransitionResult::ActionRejected(
            DockspaceActionRejection::SurfaceRequired {
                surface: source.binding.surface(),
            },
        ),
    );
    assert_eq!(
        fixture.session.view().item(B).unwrap().surface(),
        source.binding.surface()
    );
    assert!(!fixture.session.has_pending_native_create());
    let [close] = report.take_native_effects().try_into().unwrap();
    assert!(
        matches!(close.operation(), NativeEffectOperation::CompensatingClose { binding } if *binding == child)
    );
    assert!(matches!(
        close.accepted(),
        Some(NativeEffectAcknowledgement::Close(_))
    ));
    assert!(
        idle(&mut fixture.session)
            .presentation_transitions()
            .is_empty()
    );
    present(&mut fixture.session, source.binding, source.placement);
}

#[test]
fn required_surface_retirement_cannot_be_reversed_by_same_identifier_reattachment() {
    for reattach in [false, true] {
        let mut fixture = Fixture::new();
        let required = SurfaceId::new(10);
        let mut frame = native_frame(&mut fixture.session);
        frame
            .attach_empty_surface_current(required, RootId::new(10))
            .unwrap();
        finish(frame);
        let action = fixture
            .session
            .prepare_move_to_new_window(DockMoveSource::Item(B), placement())
            .preserving_surfaces([required]);
        let mut request = fixture.request(action);
        let ack = request.accept();
        fixture.observe(&request, NativeCreateReadiness::Created, Some(ack));

        let mut frame = native_frame(&mut fixture.session);
        frame.retire_empty_surface_current(required).unwrap();
        let mut report = finish(frame);
        assert_failed(
            &report,
            request.transition,
            DockspacePresentationTransitionResult::ActionRejected(
                DockspaceActionRejection::SurfaceRequired { surface: required },
            ),
        );
        for effect in report.take_native_effects() {
            assert!(
                matches!(effect.operation(), NativeEffectOperation::CompensatingClose { binding } if *binding == request.binding)
            );
            assert!(matches!(
                effect.accepted(),
                Some(NativeEffectAcknowledgement::Close(_))
            ));
        }
        if reattach {
            let mut frame = native_frame(&mut fixture.session);
            frame
                .attach_empty_surface_current(required, RootId::new(30))
                .unwrap();
            let report = finish(frame);
            assert!(report.presentation_transitions().is_empty());
        }
        fixture
            .session
            .report_managed_native_snapshot(
                [
                    (fixture.root, root_facts()),
                    (
                        request.binding,
                        window_facts(request.placement)
                            .with_create_readiness(NativeCreateReadiness::Prepared, None),
                    ),
                ],
                work_areas(),
            )
            .unwrap();
        let report = idle(&mut fixture.session);
        assert!(report.native_admissions().is_empty());
        assert!(report.presentation_transitions().is_empty());
        assert!(!fixture.session.has_pending_native_create());
        assert_eq!(fixture.session.view().item(B).unwrap().surface(), HOST);
    }
}

#[test]
fn workspace_replacement_terminates_the_original_create_and_retains_cleanup() {
    // Replacement overlaps the retired Root, its successor and the Child
    // awaiting compensation. Quiescence cannot release the old Root before commit.
    let mut fixture = Fixture::configured(3, false);
    let predecessor = fixture.root;
    let action = fixture
        .session
        .prepare_move_to_new_window(DockMoveSource::Item(B), placement());
    let mut request = fixture.request(action);
    let ack = request.accept();
    fixture.observe(&request, NativeCreateReadiness::Created, Some(ack));
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        HOST,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::central_tabs([A, B, SIBLING, CONTAINED]),
        ),
    )])
    .unwrap();
    let reset = fixture.session.prepare_layout_reset(layout).unwrap();
    let mut rollback = fixture
        .session
        .begin_native_layout_reset_frame(&reset, |_| NativeReceiverAnswer::Unknown)
        .unwrap();
    rollback
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    drop(rollback);
    assert!(fixture.session.has_pending_native_create());

    let frame = fixture
        .session
        .begin_native_layout_reset_frame(&reset, |_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let mut report = finish(frame);
    assert_failed(
        &report,
        request.transition,
        DockspacePresentationTransitionResult::WorkspaceChanged,
    );
    assert!(!fixture.session.has_pending_native_create());
    assert_eq!(fixture.session.view().item(B).unwrap().surface(), HOST);
    let [successor] = report.native_bindings() else {
        panic!("replacement must retain only the new Root binding as current");
    };
    fixture.root = *successor;
    assert_ne!(fixture.root, predecessor);
    assert!(fixture.session.recognizes_native_binding(predecessor));
    assert!(fixture.session.recognizes_native_binding(request.binding));
    let effects = report.take_native_effects();
    let [close] = effects.try_into().unwrap();
    assert!(
        matches!(close.operation(), NativeEffectOperation::CompensatingClose { binding } if *binding == request.binding)
    );
    let Some(NativeEffectAcknowledgement::Close(destroyed)) = close.accepted() else {
        panic!("the old child must retain its exact destruction acknowledgement");
    };
    fixture
        .session
        .report_native_binding_quiescence(predecessor)
        .unwrap();
    fixture
        .session
        .report_managed_native_snapshot(
            [
                (fixture.root, root_facts()),
                (
                    request.binding,
                    NativeWindowFacts::destroyed_after(destroyed),
                ),
            ],
            work_areas(),
        )
        .unwrap();
    let report = idle(&mut fixture.session);
    assert!(report.presentation_transitions().is_empty());
    assert!(report.native_admissions().is_empty());
    fixture
        .session
        .report_native_binding_quiescence(request.binding)
        .unwrap();
    let report = idle(&mut fixture.session);
    assert!(report.presentation_transitions().is_empty());
    assert!(!fixture.session.recognizes_native_binding(predecessor));
    assert!(!fixture.session.recognizes_native_binding(request.binding));
}
