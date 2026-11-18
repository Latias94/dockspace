//! Accepted lifecycle tails must not block unrelated windows or fresh input.

use super::*;
use crate::close::plan::{CloseDecision, ClosePlanPhase};
use crate::runtime::{HostInputOutcome, NativeCloseDisposition};

#[test]
fn duplicate_native_registration_rejects_one_record_without_poisoning_the_prefix() {
    let (mut session, root) = managed_tear_off_session();
    let other = SurfaceId::new(20);
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .attach_empty_surface_current(other, RootId::new(20))
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();
    for _ in 0..2 {
        session
            .register_native_root(other, HostWindowToken::new(20))
            .unwrap();
    }
    session
        .publish_native_close(root, NativeCloseState::Requested, None)
        .unwrap();
    let report = commit_managed_frame_report(&mut session);
    assert!(matches!(report.inputs(), [
        HostInputOutcome::NativeSurfaceRegistered { .. },
        HostInputOutcome::NativeSurfaceRegistrationRejected { surface },
        HostInputOutcome::NativeCloseObservationApplied { close_requests },
    ] if *surface == other && close_requests.len() == 1));
    assert!(!report.native_input_pending());
    assert_eq!(report.native_bindings().len(), 2);
    assert!(
        commit_managed_frame_report(&mut session)
            .inputs()
            .is_empty()
    );
}

#[test]
fn a_native_token_conflict_rejects_one_record_without_blocking_an_unrelated_close() {
    let (mut session, root) = managed_tear_off_session();
    let other = SurfaceId::new(20);
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .attach_empty_surface_current(other, RootId::new(20))
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();
    session.register_native_root(other, WINDOW).unwrap();
    session
        .publish_native_close(root, NativeCloseState::Requested, None)
        .unwrap();
    let report = commit_managed_frame_report(&mut session);
    assert!(matches!(report.inputs(), [
        HostInputOutcome::NativeSurfaceRegistrationRejected { surface },
        HostInputOutcome::NativeCloseObservationApplied { close_requests },
    ] if *surface == other && close_requests.len() == 1));
    assert!(!report.native_input_pending());
}

#[test]
fn a_close_clear_accepted_before_destruction_commits_cannot_poison_later_registration() {
    let (mut session, root) = managed_tear_off_session();
    session
        .report_managed_native_snapshot(
            [(root, NativeWindowFacts::destroyed())],
            NativeWorkAreaRoster::Unknown,
        )
        .unwrap();
    session
        .publish_native_close(root, NativeCloseState::Clear, None)
        .unwrap();
    session
        .register_native_root(SURFACE, HostWindowToken::new(42))
        .unwrap();
    let report = commit_managed_frame_report(&mut session);
    assert!(!report.native_input_pending());
    assert!(
        matches!(report.inputs().last(), Some(HostInputOutcome::NativeSurfaceRegistered { binding })
        if binding.window_token() == HostWindowToken::new(42))
    );
    assert!(!session.is_current_native_binding(root));
    assert!(
        commit_managed_frame_report(&mut session)
            .inputs()
            .is_empty()
    );
}

#[test]
fn a_staging_close_clear_cannot_block_an_unrelated_root_close() {
    let PendingNativeCreate {
        mut session,
        root_binding,
        child_binding,
        placement,
        request,
    } = request_native_create();
    let Some(NativeEffectAcknowledgement::Create(created)) = request.accepted() else {
        panic!("the exact create must yield its acknowledgement");
    };
    let scale = native_scale();
    let facts = NativeWindowFacts::live()
        .with_outer_bounds(placement)
        .with_content_bounds(placement)
        .with_native_scale_factor(scale)
        .with_presentation_scale_factor(scale)
        .with_create_readiness(
            crate::runtime::NativeCreateReadiness::Created,
            Some(created),
        )
        .with_presentation(NativeWindowPresentationState::Hidden)
        .with_close(NativeCloseState::Clear, None);
    session
        .report_managed_native_snapshot(
            [(root_binding, root_window_facts()), (child_binding, facts)],
            NativeWorkAreaRoster::Exact(vec![work_area(1)]),
        )
        .unwrap();
    commit_managed_frame_report(&mut session);
    session
        .publish_native_close(child_binding, NativeCloseState::Requested, None)
        .unwrap();
    session
        .publish_native_close(child_binding, NativeCloseState::Clear, None)
        .unwrap();
    session
        .publish_native_close(root_binding, NativeCloseState::Requested, None)
        .unwrap();
    let report = commit_managed_frame_report(&mut session);
    assert!(
        matches!(report.inputs().last(), Some(HostInputOutcome::NativeCloseObservationApplied { close_requests })
        if close_requests.len() == 1 && close_requests[0].binding() == root_binding)
    );
    assert!(!report.native_input_pending());
    assert_eq!(session.view().item(ITEM).unwrap().surface(), SURFACE);
    assert!(
        commit_managed_frame_report(&mut session)
            .inputs()
            .is_empty()
    );
}

#[test]
fn exactly_cancelled_native_close_reestablishes_coordinates_and_fresh_presentation() {
    let (mut session, binding) = managed_tear_off_session();
    session
        .publish_native_close(binding, NativeCloseState::Requested, None)
        .unwrap();
    let observed = commit_managed_frame_report(&mut session);
    let [HostInputOutcome::NativeCloseObservationApplied { close_requests }] = observed.inputs()
    else {
        panic!("expected native close edge")
    };
    let close = close_requests[0];
    session
        .request_native_surface_close(close, NativeSurfaceCloseAction::RetainLayout)
        .unwrap();
    let requested = commit_managed_frame_report(&mut session);
    let [HostInputOutcome::NativeSurfaceCloseRequested { plan, .. }] = requested.inputs() else {
        panic!("expected close plan")
    };
    let plan = plan.clone();
    let mut approval = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    for item in plan.items() {
        approval
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
    }
    approval
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let mut approved = approval.commit().unwrap();
    let accept = take_only_native_effect(&mut approved);
    assert!(matches!(
        accept.operation(),
        NativeEffectOperation::ResolveNativeClose {
            resolution: NativeCloseDisposition::Accept,
            ..
        }
    ));
    let _ = accept.accepted();
    assert!(session.native_local_coordinates(binding).is_none());
    session.cancel_native_surface_close(close).unwrap();
    let mut cancelled = commit_managed_frame_report(&mut session);
    let cancel = take_only_native_effect(&mut cancelled);
    let Some(NativeEffectAcknowledgement::Close(acknowledgement)) = cancel.accepted() else {
        panic!("expected cancellation acknowledgement")
    };
    session
        .publish_native_close(binding, NativeCloseState::Clear, Some(acknowledgement))
        .unwrap();
    commit_managed_frame_report(&mut session);
    assert_eq!(
        session.engine.close_plan(plan.request()).unwrap().phase(),
        ClosePlanPhase::Cancelled
    );
    session
        .report_managed_native_snapshot(
            [(binding, root_window_facts())],
            NativeWorkAreaRoster::Exact(vec![work_area(1)]),
        )
        .unwrap();
    commit_managed_frame_report(&mut session);
    assert!(
        session.native_local_coordinates(binding).is_some(),
        "an exactly cancelled close must permit fresh input geometry"
    );
    measure_all_surfaces(&mut session);
    let (output, _) = paint_native_output(&mut session);
    session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    commit_managed_frame_report(&mut session);
    assert!(session.native_binding_can_report_focus(binding));
    let _ = presented_tab_receiver(&mut session, SECOND_ITEM);
}
