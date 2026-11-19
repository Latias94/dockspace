//! Shared-window close correlation keeps physical capture and semantic ownership distinct.
//!
//! These tokenless models use real core close capabilities. They do not qualify
//! opaque eframe HoldClose tokens or physical Allow/Veto delivery.

use std::sync::{Arc, Mutex};

use dockspace::close::{CloseDecision, SurfaceCloseDisposition};
use dockspace::geometry::PhysicalPoint;
use dockspace::runtime::NativeSurfacePlacement;

use super::*;

fn layout() -> DockspaceLayout {
    DockspaceLayout::new([FIRST_SURFACE, SECOND_SURFACE].map(|surface| {
        let id = if surface == FIRST_SURFACE { 1 } else { 2 };
        DockspaceSurfaceLayout::new(
            surface,
            DockspaceRootLayout::new(
                RootId::new(id),
                DockspaceNode::central_tabs([ItemId::new(id)]),
            ),
        )
    }))
    .unwrap()
}

fn finish(mut frame: NativeHostFrame<'_>) -> HostFrameReport {
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap()
}

fn commit(native: &mut NativeCoordinator) -> HostFrameReport {
    let frame = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let report = finish(frame);
    native.settle_host_frame_report(&report).unwrap();
    report
}

fn shared_root_two() -> NativeCoordinator {
    let mut session = DockspaceSession::from_layout(layout(), DockPolicy::default()).unwrap();
    session.retain_host_surface(SECOND_SURFACE).unwrap();
    let mut native =
        NativeCoordinator::new(session, NativePointerRoster::Exact(Vec::new())).unwrap();
    native
        .register_native_root(SECOND_SURFACE, HostWindowToken::new(31))
        .unwrap();
    let original = commit(&mut native).native_bindings()[0];
    native
        .bind_viewport(ViewportId::ROOT, WindowId::from(31), original)
        .unwrap();
    native
        .report_snapshot(
            [(original, ready_window_facts(0.0))],
            NativeWorkAreaRoster::Unknown,
            test_managed_capabilities(),
        )
        .unwrap();
    commit(&mut native);
    let mut frame = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let placements = [FIRST_SURFACE, SECOND_SURFACE].map(|surface| {
        let x = if surface == FIRST_SURFACE { 0.0 } else { 360.0 };
        NativeSurfacePlacement::new(
            surface,
            LogicalSize::new(280.0, 440.0).unwrap(),
            PhysicalPoint::new(x, 20.0).unwrap(),
            ScaleFactor::new(1.0).unwrap(),
            PhysicalRect::new(x, 20.0, 280.0, 440.0).unwrap(),
        )
    });
    frame
        .frame
        .set_native_surface_placements(original.window(), &placements)
        .unwrap();
    let report = finish(frame);
    native.settle_host_frame_report(&report).unwrap();
    assert_eq!(native.viewport_binding(ViewportId::ROOT), Some(original));
    native
}

fn transport(native: &NativeCoordinator) -> NativeSurfaceBinding {
    native.viewport_binding(ViewportId::ROOT).unwrap()
}

fn reset_shared_root(native: &mut NativeCoordinator) -> NativeSurfaceBinding {
    let predecessor = transport(native);
    let reset = native.session.prepare_layout_reset(layout()).unwrap();
    let mut root_surface = SECOND_SURFACE;
    let mut frame = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame.frame.submit_layout_reset(&reset).unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit_publishing_root(&mut root_surface).unwrap();
    assert_eq!(report.native_root_continuations().len(), 2);
    native.settle_host_frame_report(&report).unwrap();
    let successor = transport(native);
    assert_eq!(successor.surface(), SECOND_SURFACE);
    assert_ne!(predecessor.window(), successor.window());
    assert_eq!(root_surface, SECOND_SURFACE);
    assert_eq!(
        native
            .bridge
            .with_viewports(|routes| routes.window(ViewportId::ROOT)),
        Some(WindowId::from(31))
    );
    native
        .report_snapshot(
            [(successor, ready_window_facts(0.0))],
            NativeWorkAreaRoster::Unknown,
            test_managed_capabilities(),
        )
        .unwrap();
    commit(native);
    predecessor
}

fn observe_close(native: &mut NativeCoordinator, ordinal: u64) -> NativeSurfaceCloseRequest {
    let captured = transport(native);
    let event = NativeWindowEventRecord::for_test(
        ordinal,
        WindowId::from(31),
        Some(ViewportId::ROOT),
        Some(captured),
        WindowEvent::CloseRequested,
    );
    for _ in 0..2 {
        native
            .bridge
            .push_record(HostRecord::WindowEvent(event.clone()));
    }
    assert_eq!(
        native
            .reduce_callback_prefix(&egui::Context::default(), &mut test_budget())
            .unwrap()
            .unwrap()
            .get(),
        1
    );
    let report = commit(native);
    let requests = report
        .inputs()
        .iter()
        .filter_map(|input| match input {
            HostInputOutcome::NativeCloseObservationApplied { close_requests } => {
                Some(close_requests)
            }
            _ => None,
        })
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let [close]: [NativeSurfaceCloseRequest; 1] = requests.try_into().unwrap();
    assert_eq!(close.binding().window(), captured.window());
    assert!(native.settle_close_control_report(&report).unwrap());
    native.bridge.with_close_control(|control| {
        assert_eq!(control.request_for_binding(captured), Some(close));
        assert_eq!(control.request_for_binding(close.binding()), Some(close));
        assert_eq!(control.unsubmitted_requests().collect::<Vec<_>>(), [close]);
    });
    close
}

#[test]
fn rebound_shared_window_runs_its_external_root_close_policy_once() {
    for cancel in [false, true] {
        let mut native = shared_root_two();
        reset_shared_root(&mut native);
        let close = observe_close(&mut native, 51);
        assert_eq!(close.surface(), FIRST_SURFACE);
        assert_ne!(close.binding(), transport(&native));
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&outcomes);
        native.observe_reports_for_test(move |report| {
            for input in report.inputs() {
                let outcome = match input {
                    HostInputOutcome::NativeSurfaceCloseCancellationRequired { close, .. } => {
                        Some((*close, None))
                    }
                    HostInputOutcome::NativeSurfaceCloseRequested {
                        close, disposition, ..
                    } => Some((*close, Some(*disposition))),
                    _ => None,
                };
                if let Some(outcome) = outcome {
                    observed.lock().unwrap().push(outcome);
                }
            }
        });
        let context = egui::Context::default();
        let policy = if cancel {
            NativeWindowClosePolicy::Cancel
        } else {
            NativeWindowClosePolicy::RecoverPresentation
        };
        assert!(
            native
                .drive_close_policy(&context, policy, SECOND_SURFACE, &mut test_budget())
                .unwrap()
        );
        assert_eq!(
            native
                .bridge
                .with_close_control(|control| control.decision_requests().count()),
            usize::from(!cancel)
        );
        // RetainLayout still needs both panes' decisions. Advancing that plan
        // is progress, but must not submit the operating-system close twice.
        assert_eq!(
            native
                .drive_close_policy(&context, policy, SECOND_SURFACE, &mut test_budget())
                .unwrap(),
            !cancel
        );
        assert!(
            native
                .bridge
                .with_close_control(|control| control.decision_requests().next().is_none())
        );
        let recorded = native.session.native_input_recorded_count();
        assert!(
            !native
                .drive_close_policy(&context, policy, SECOND_SURFACE, &mut test_budget())
                .unwrap()
        );
        assert_eq!(native.session.native_input_recorded_count(), recorded);
        let outcomes = outcomes.lock().unwrap();
        assert_eq!(
            outcomes.as_slice(),
            &[(
                close,
                (!cancel).then_some(SurfaceCloseDisposition::RetainLayout)
            )]
        );
        assert_eq!(native.close_progress().requests_retained(), 1);
        assert_eq!(
            native
                .session
                .view()
                .item(ItemId::new(1))
                .unwrap()
                .surface(),
            FIRST_SURFACE
        );
        assert_eq!(
            native
                .session
                .view()
                .item(ItemId::new(2))
                .unwrap()
                .surface(),
            SECOND_SURFACE
        );
    }
}

#[test]
fn rebound_shared_window_clear_releases_both_capture_and_semantic_owners_once() {
    let mut native = shared_root_two();
    reset_shared_root(&mut native);
    let captured = transport(&native);
    let close = observe_close(&mut native, 61);
    assert_eq!(close.surface(), FIRST_SURFACE);
    assert_ne!(close.binding(), captured);
    let acknowledgement = begin_model_cancellation(&mut native, close, ViewportId::ROOT);
    native.bridge.with_close_control(|control| {
        assert!(control.references_binding(captured));
        assert!(control.references_binding(close.binding()));
        assert!(!control.accept_close(close));
    });
    native
        .publish_close(
            close.binding(),
            NativeCloseState::Clear,
            Some(acknowledgement),
        )
        .unwrap();
    assert!(
        native
            .bridge
            .with_close_control(|control| control.mark_clear_recorded(close.binding()))
    );
    let report = commit(&mut native);
    assert!(native.settle_close_control_report(&report).unwrap());
    assert!(!native.settle_close_control_report(&report).unwrap());
    native.bridge.with_close_control(|control| {
        assert!(!control.references_binding(captured));
        assert!(!control.references_binding(close.binding()));
        assert!(!control.has_pending_work());
    });
    assert_eq!(native.close_progress().cancellations_committed(), 1);
    assert!(
        native
            .session
            .native_window_bindings()
            .any(|window| window == captured.window())
    );
}

#[test]
fn rebound_shared_window_retires_its_transport_receipt_for_the_semantic_close() {
    let mut native = shared_root_two();
    reset_shared_root(&mut native);
    let captured = transport(&native);
    let close = observe_close(&mut native, 66);
    assert_ne!(close.binding(), captured);
    assert_eq!(
        native.native_close_retirement_binding(close),
        Some(captured)
    );
    native
        .session
        .request_native_surface_close(close, NativeSurfaceCloseAction::RetainLayout)
        .unwrap();
    let mut requested = commit(&mut native);
    assert!(requested.take_native_effects().is_empty());
    let plan = requested
        .inputs()
        .iter()
        .find_map(|input| match input {
            HostInputOutcome::NativeSurfaceCloseRequested {
                close: requested,
                plan,
                ..
            } if *requested == close => Some(plan),
            _ => None,
        })
        .unwrap();
    assert_eq!(plan.items().len(), 2);
    let mut frame = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    for item in plan.items() {
        frame
            .frame
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
    }
    let mut report = finish(frame);
    native.settle_host_frame_report(&report).unwrap();
    let [effect] = report.take_native_effects().try_into().unwrap();
    assert_eq!(
        effect.operation(),
        &NativeEffectOperation::ResolveNativeClose {
            close,
            resolution: NativeCloseDisposition::Accept,
        }
    );
    let Some(NativeEffectAcknowledgement::Close(acknowledgement)) = effect.accepted() else {
        panic!("the semantic close retains its exact destruction acknowledgement");
    };
    // Only the tokenless correlation model accepts here. A real Allow remains
    // guarded by the original opaque eframe request in production dispatch.
    assert!(
        native
            .bridge
            .retirement_begin_release(ViewportId::ROOT, captured, acknowledgement)
    );
    assert!(
        native
            .bridge
            .with_close_control(|control| control.accept_close(close))
    );
    native
        .bridge
        .push_record(HostRecord::WindowEvent(NativeWindowEventRecord::for_test(
            67,
            WindowId::from(31),
            Some(ViewportId::ROOT),
            Some(captured),
            WindowEvent::Destroyed,
        )));
    native.bridge.push_viewport_roster(live_roster([]));
    let context = egui::Context::default();
    assert!(
        native
            .reduce_callback_prefix(&context, &mut test_budget())
            .unwrap()
            .is_some()
    );
    commit(&mut native);
    assert!(
        !native
            .bridge
            .with_close_control(|control| control.has_pending_work())
    );
    assert!(
        native
            .reduce_callback_prefix(&context, &mut test_budget())
            .unwrap()
            .is_some()
    );
    commit(&mut native);
    assert!(
        !native
            .session
            .native_window_bindings()
            .any(|window| window == captured.window())
    );
    let retirement = native.prepare_committed_retirements().unwrap().unwrap();
    commit(&mut native);
    assert!(native.commit_retirements(retirement).is_empty());
    assert!(native.try_report_retirement_quiescence().unwrap());
    commit(&mut native);
    assert!(!native.session.recognizes_native_binding(captured));
    assert!(!native.session.recognizes_native_binding(close.binding()));
    assert!(native.session.view().surface(FIRST_SURFACE).is_some());
    assert!(native.session.view().surface(SECOND_SURFACE).is_some());
    assert!(
        !native
            .bridge
            .with_close_control(|control| control.has_pending_work())
    );
}

#[test]
fn rebound_shared_window_rejects_old_close_and_destroyed_callbacks_on_reused_window_id() {
    let mut native = shared_root_two();
    let old_close = observe_close(&mut native, 71);
    let old_acknowledgement = begin_model_cancellation(&mut native, old_close, ViewportId::ROOT);
    // Reset only continues a live window. Complete the original cancellation
    // before replacing its lifetime; keep its exact handles for late callbacks.
    native
        .publish_close(
            old_close.binding(),
            NativeCloseState::Clear,
            Some(old_acknowledgement),
        )
        .unwrap();
    assert!(
        native
            .bridge
            .with_close_control(|control| control.mark_clear_recorded(old_close.binding()))
    );
    let clear = commit(&mut native);
    assert!(native.settle_close_control_report(&clear).unwrap());
    assert!(
        !native
            .bridge
            .with_close_control(|control| control.has_pending_work())
    );
    assert_eq!(native.close_progress().cancellations_committed(), 1);
    let predecessor = reset_shared_root(&mut native);
    let captured = transport(&native);
    let close = observe_close(&mut native, 81);
    assert_eq!(close.surface(), FIRST_SURFACE);
    assert_ne!(close.binding(), captured);
    let destroyed = |binding, ordinal| {
        NativeWindowEventRecord::for_test(
            ordinal,
            WindowId::from(31),
            Some(ViewportId::ROOT),
            Some(binding),
            WindowEvent::Destroyed,
        )
    };
    native.bridge.with_close_control(|control| {
        assert_eq!(control.bind_request(old_close), Ok(false));
        control.mark_policy_recorded(old_close);
        assert_eq!(control.unsubmitted_requests().collect::<Vec<_>>(), [close]);
        assert_eq!(
            control.begin_cancellation(old_close, old_acknowledgement),
            None
        );
        assert!(!control.accept_close(old_close));
        assert!(control.accept_close(close));
        assert!(!control.accept_close(close));
        let stale = destroyed(predecessor, 82);
        assert_eq!(
            control.accepted_destroyed_matches(&stale, captured),
            Err(())
        );
        assert!(!control.settle_accepted_destroyed(&stale, captured));
        assert!(!control.settle_accepted_destroyed(&stale, predecessor));
        assert!(control.references_binding(captured));
        assert!(control.references_binding(close.binding()));
        let current = destroyed(captured, 83);
        assert_eq!(
            control.accepted_destroyed_matches(&current, captured),
            Ok(true)
        );
        assert!(control.settle_accepted_destroyed(&current, captured));
        assert!(!control.settle_accepted_destroyed(&current, captured));
        assert!(!control.has_pending_work());
    });
    assert!(native.session.is_current_native_binding(close.binding()));
    assert_eq!(native.close_progress().requests_retained(), 2);
}
