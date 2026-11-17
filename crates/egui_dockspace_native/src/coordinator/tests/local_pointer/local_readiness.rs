//! Local host readiness grants paint admission, never an already presented receiver.

use dockspace::runtime::{NativeGlobalFocus, NativeLocalReadiness};

use super::*;

fn local_facts(readiness: Option<NativeLocalReadiness>) -> NativeWindowFacts {
    let geometry = geometry();
    let facts = NativeWindowFacts::live()
        .with_content_size(geometry.content_size())
        .with_native_scale_factor(geometry.native_scale_factor())
        .with_presentation_scale_factor(ScaleFactor::new(1.5).unwrap())
        .with_input(NativeWindowInputState::ReceivesInput, None);
    readiness.map_or(facts, |readiness| facts.with_local_readiness(readiness))
}

fn local_capabilities() -> NativeHostCapabilities {
    NativeHostCapabilities::none_supported()
        .with(NativeHostCapability::AuthoritativeInventory)
        .with(NativeHostCapability::AuthoritativeButtonState)
        .with(NativeHostCapability::PointerHitTestObservation)
}

fn encode(
    native: &mut NativeCoordinator,
    binding: NativeSurfaceBinding,
) -> (Scene, PaintedSurfaceOutput) {
    let mut frame = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let scene = render(&mut frame);
    assert!(!scene.preview);
    let mut report = finish(frame);
    let output = take_output(&mut report, &scene, binding);
    (scene, output)
}

fn record_raw(
    native: &mut NativeCoordinator,
    binding: NativeSurfaceBinding,
    ordinal: u64,
    event: WindowEvent,
) {
    let record = capture(native, binding, ordinal, event);
    native.bridge.push_record(HostRecord::WindowEvent(record));
    reduce_one(native);
}

fn assert_no_delivery(report: &HostFrameReport) {
    assert!(!report.workspace_changed());
    assert!(
        report
            .manipulation_feedback()
            .iter()
            .all(|feedback| !matches!(feedback.status(), Status::Delivered(_)))
    );
}

#[test]
fn ready_without_os_visibility_or_desktop_origin_paints_before_receivers_can_interact() {
    let (mut native, binding, now) = measured_fixture(
        local_facts(Some(NativeLocalReadiness::Ready)),
        local_capabilities(),
        NativePointerRoster::Exact(Vec::new()),
    );
    let version = native.session.version();
    assert!(native.session.native_local_coordinates(binding).is_some());
    let (scene, output) = encode(&mut native, binding);
    let receiver = scene.tabs[0].1;
    let source = scene.point(FIRST, 0.25);
    let target = scene.point(THIRD, 0.625);
    assert!(native.session.presented_surface(FIRST_SURFACE).is_none());
    assert!(native.session.bind_presented_receiver(&receiver).is_none());

    now.store(40, Ordering::SeqCst);
    for (ordinal, point, state) in [
        (1, source, Some(ElementState::Pressed)),
        (2, target, None),
        (3, target, Some(ElementState::Released)),
    ] {
        record_raw(&mut native, binding, ordinal, raw_event(point, state, 2.0));
        let report = finish(
            native
                .begin_host_frame(|query| match query {
                    NativeReceiverQuery::WindowHit(_) => {
                        NativeReceiverAnswer::WindowHit(NativeWindowHit::Unknown)
                    }
                    NativeReceiverQuery::Surface(_) => {
                        panic!("an unpresented output owns no surface receiver challenge")
                    }
                })
                .unwrap(),
        );
        assert_no_delivery(&report);
        assert!(!native.session.has_active_gesture());
        assert_eq!(native.session.version(), version);
        assert_eq!(order(&native), [FIRST, SECOND, THIRD]);
    }

    native
        .session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    assert!(native.session.bind_presented_receiver(&receiver).is_none());
    idle(&mut native);
    assert!(native.session.bind_presented_receiver(&receiver).is_some());

    for (ordinal, point, state) in [
        (4, source, Some(ElementState::Pressed)),
        (5, target, None),
        (6, target, Some(ElementState::Released)),
    ] {
        record_raw(&mut native, binding, ordinal, raw_event(point, state, 2.0));
        let report = input_boundary(&mut native, &scene, CAPTURED_AT);
        if state == Some(ElementState::Released) {
            assert_eq!(
                feedback(&report).status(),
                Status::Delivered(Delivery::Dock)
            );
            assert!(report.workspace_changed());
        } else {
            assert!(report.manipulation_feedback().is_empty());
            assert!(native.session.has_active_gesture());
        }
    }
    assert_eq!(order(&native), [SECOND, THIRD, FIRST]);
    assert!(!native.session.has_active_gesture());
    assert!(native.next_window_event().unwrap().is_none());
    assert_eq!(native.bridge.input_backlog_for_test(), (0, None));
    assert!(native.pointer_translator.can_coalesce_idle_cursor_moves());
}

#[test]
fn readiness_revocation_rejects_the_old_release_and_same_geometry_needs_new_presentation() {
    for (name, withdrawn) in [
        (
            "unavailable",
            local_facts(Some(NativeLocalReadiness::Unavailable)),
        ),
        (
            "unavailable despite Visible",
            local_facts(Some(NativeLocalReadiness::Unavailable))
                .with_presentation(NativeWindowPresentationState::Visible),
        ),
        ("unknown", local_facts(None)),
        (
            "hidden despite Ready",
            local_facts(Some(NativeLocalReadiness::Ready))
                .with_presentation(NativeWindowPresentationState::Hidden),
        ),
        (
            "minimized despite Ready",
            local_facts(Some(NativeLocalReadiness::Ready))
                .with_presentation(NativeWindowPresentationState::Minimized),
        ),
    ] {
        let ready = local_facts(Some(NativeLocalReadiness::Ready));
        let capabilities = local_capabilities();
        let (mut native, binding, now) =
            measured_fixture(ready, capabilities, NativePointerRoster::Exact(Vec::new()));
        let scene = paint_and_present(&mut native, binding);
        let version = native.session.version();
        let coordinates = native.session.native_local_coordinates(binding).unwrap();
        let receiver = scene.tabs[0].1;
        let source = scene.point(FIRST, 0.25);
        let target = scene.point(THIRD, 0.625);
        now.store(40, Ordering::SeqCst);
        for (ordinal, point, state) in [(1, source, Some(ElementState::Pressed)), (2, target, None)]
        {
            record_raw(&mut native, binding, ordinal, raw_event(point, state, 2.0));
            let report = input_boundary(&mut native, &scene, CAPTURED_AT);
            assert_no_delivery(&report);
            assert!(native.session.has_active_gesture(), "{name}");
        }
        let old_release = capture(
            &native,
            binding,
            3,
            raw_event(target, Some(ElementState::Released), 2.0),
        );

        native
            .report_snapshot(
                [(binding, withdrawn)],
                NativeWorkAreaRoster::Unknown,
                capabilities,
            )
            .unwrap();
        let revoked = idle(&mut native);
        assert_no_delivery(&revoked);
        assert!(!native.session.has_active_gesture(), "{name}");
        assert!(
            native.session.native_local_coordinates(binding).is_none(),
            "{name}"
        );
        assert!(
            native.session.presented_surface(FIRST_SURFACE).is_none(),
            "{name}"
        );
        assert!(
            native.session.bind_presented_receiver(&receiver).is_none(),
            "{name}"
        );
        let mut blocked = native
            .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap();
        assert!(
            blocked.frame.paint_plan(FIRST_SURFACE).unwrap().is_none(),
            "{name}"
        );
        drop(blocked);

        native
            .report_snapshot(
                [(binding, ready)],
                NativeWorkAreaRoster::Unknown,
                capabilities,
            )
            .unwrap();
        let restored = idle(&mut native);
        assert_no_delivery(&restored);
        let restored_coordinates = native.session.native_local_coordinates(binding).unwrap();
        assert_eq!(restored_coordinates.geometry(), coordinates.geometry());
        assert!(
            native.session.presented_surface(FIRST_SURFACE).is_none(),
            "{name}"
        );
        assert!(
            native.session.bind_presented_receiver(&receiver).is_none(),
            "{name}"
        );

        let (fresh, output) = encode(&mut native, binding);
        native
            .bridge
            .push_record(HostRecord::WindowEvent(old_release));
        reduce_one(&mut native);
        let late = finish(
            native
                .begin_host_frame(|_| panic!("a revoked old release cannot regain a receiver"))
                .unwrap(),
        );
        assert_no_delivery(&late);
        assert!(!native.session.has_active_gesture(), "{name}");
        assert_eq!(native.session.version(), version, "{name}");
        assert_eq!(order(&native), [FIRST, SECOND, THIRD], "{name}");

        native
            .session
            .report_surface_presentation(output, SurfacePresentationResult::Presented)
            .unwrap();
        idle(&mut native);
        assert!(
            native
                .session
                .bind_presented_receiver(&fresh.tabs[0].1)
                .is_some()
        );
        for (ordinal, point, state) in [
            (4, fresh.point(FIRST, 0.25), Some(ElementState::Pressed)),
            (5, fresh.point(THIRD, 0.625), None),
            (6, fresh.point(THIRD, 0.625), Some(ElementState::Released)),
        ] {
            record_raw(&mut native, binding, ordinal, raw_event(point, state, 2.0));
            let report = input_boundary(&mut native, &fresh, CAPTURED_AT);
            if state == Some(ElementState::Released) {
                assert_eq!(
                    feedback(&report).status(),
                    Status::Delivered(Delivery::Dock),
                    "{name}"
                );
            } else {
                assert_no_delivery(&report);
                assert!(native.session.has_active_gesture(), "{name}");
            }
        }
        assert_eq!(order(&native), [SECOND, THIRD, FIRST], "{name}");
        assert!(!native.session.has_active_gesture());
        assert!(!native.session.has_pending_native_input());
        assert_eq!(native.bridge.input_backlog_for_test(), (0, None));
    }
}

#[test]
fn local_readiness_does_not_authorize_desktop_handoff_or_native_activation() {
    let capabilities = capabilities_for_backend(eframe::NativeWindowingBackend::X11);
    assert!(capabilities.supports(NativeHostCapability::DesktopPointerPosition));
    assert!(capabilities.supports(NativeHostCapability::WindowActivationControl));
    let facts = local_facts(Some(NativeLocalReadiness::Ready))
        .with_content_bounds(PhysicalRect::new(400.0, -300.0, 1200.0, 900.0).unwrap());
    let (mut native, binding, now) =
        measured_fixture(facts, capabilities, NativePointerRoster::Exact(Vec::new()));
    let scene = paint_and_present(&mut native, binding);
    let version = native.session.version();
    let source = scene.point(FIRST, 0.25);
    let target = scene.point(THIRD, 0.625);
    now.store(40, Ordering::SeqCst);

    // Matching desktop bounds and supported global capabilities cannot turn
    // local admission into observed OS visibility for a Desktop gesture.
    for (ordinal, point, state) in [
        (1, source, Some(ElementState::Pressed)),
        (2, target, None),
        (3, target, Some(ElementState::Released)),
    ] {
        record_raw(
            &mut native,
            binding,
            ordinal,
            raw_desktop_event(point, state),
        );
        let report = finish(
            native
                .begin_host_frame(|_| panic!("local readiness grants no Desktop receiver"))
                .unwrap(),
        );
        assert_no_delivery(&report);
        assert!(!native.session.has_active_gesture());
        assert_eq!(native.session.version(), version);
    }

    for (ordinal, event) in [
        (4, raw_event(source, Some(ElementState::Pressed), 2.0)),
        (5, raw_event(target, None, 2.0)),
        (6, raw_desktop_event(target, None)),
        (7, raw_event(target, Some(ElementState::Released), 2.0)),
    ] {
        record_raw(&mut native, binding, ordinal, event);
        let report = input_boundary(&mut native, &scene, CAPTURED_AT);
        if ordinal == 7 {
            assert_eq!(
                feedback(&report).status(),
                Status::Delivered(Delivery::Dock)
            );
        } else {
            assert_no_delivery(&report);
            assert!(native.session.has_active_gesture());
        }
    }
    // A successful local-only release also proves that the dual-location move
    // did not upgrade the existing gesture to the Desktop lane.
    assert_eq!(order(&native), [SECOND, THIRD, FIRST]);
    native
        .session
        .report_native_global_focus(NativeGlobalFocus::Foreign)
        .unwrap();
    idle(&mut native);
    let mut selection = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    selection.frame.select_item_current(SECOND).unwrap();
    let mut selected = finish(selection);
    assert!(native.session.view().item(SECOND).unwrap().is_active());
    assert!(selected.take_native_effects().is_empty());

    // The same provider can activate the same window once it independently
    // reports Visible; an unsupported backend cannot make this negative pass.
    native
        .report_snapshot(
            [(
                binding,
                facts.with_presentation(NativeWindowPresentationState::Visible),
            )],
            NativeWorkAreaRoster::Unknown,
            capabilities,
        )
        .unwrap();
    idle(&mut native);
    paint_and_present(&mut native, binding);
    let mut selection = native
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    selection.frame.select_item_current(THIRD).unwrap();
    let mut selected = finish(selection);
    let effects = selected.take_native_effects();
    assert!(matches!(effects.as_slice(), [request]
        if request.operation() == &NativeEffectOperation::RequestFocus { binding }));
}
