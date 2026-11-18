//! Ordered cancellation also targets a press without a desktop capture request.

use super::*;
use crate::event::NativeKeyboardControlRecord;

#[test]
fn ordered_keyboard_cancel_ends_local_or_desktop_press_before_a_drag_starts() {
    for desktop in [false, true] {
        let (mut native, binding, scene, now) = fixture_with_desktop(desktop);
        now.store(40, Ordering::SeqCst);
        assert!(native.session.native_pointer_gesture_source().is_none());
        let source = scene.point(FIRST, 0.25);
        let event = if desktop {
            raw_desktop_event(source, Some(ElementState::Pressed))
        } else {
            raw_event(source, Some(ElementState::Pressed), 2.0)
        };
        let record = capture(&native, binding, 1, event);
        native.bridge.push_record(HostRecord::WindowEvent(record));
        reduce_one(&mut native);
        assert!(
            native.session.native_pointer_gesture_source().is_none(),
            "pending input is not a committed gesture projection"
        );
        let pressed = input_boundary(&mut native, &scene, CAPTURED_AT);
        assert!(pressed.manipulation_feedback().is_empty());
        assert!(native.session.has_active_gesture());
        let (pointer, source_binding) = native.session.native_pointer_gesture_source().unwrap();
        assert_eq!(source_binding, binding);
        assert!(native.session.native_pointer_capture_request().is_none());
        assert!(native.session.native_pointer_continuation().is_none());
        let version = native.session.version();
        native.bridge.push_record(HostRecord::KeyboardControl(
            NativeKeyboardControlRecord::for_test(
                2,
                WindowId::from(WINDOW),
                Some(ViewportId::ROOT),
                binding,
                egui::Key::Escape,
                true,
            ),
        ));
        let release = if desktop {
            raw_desktop_event(source, Some(ElementState::Released))
        } else {
            raw_event(source, Some(ElementState::Released), 2.0)
        };
        native.bridge.push_record(HostRecord::WindowEvent(capture(
            &native, binding, 3, release,
        )));
        reduce_one(&mut native);
        let cancelled = idle(&mut native);
        assert!(
            matches!(cancelled.manipulation_feedback(), [feedback] if feedback.status() == Status::Cancelled(CancelReason::ExplicitCancellation))
        );
        assert!(!native.session.has_active_gesture());
        assert!(native.session.native_pointer_gesture_source().is_none());
        assert_eq!(native.session.version(), version);
        let recorded = native.session.native_input_recorded_count();
        assert!(
            native
                .reduce_callback_prefix(&egui::Context::default(), &mut test_budget())
                .unwrap()
                .is_some()
        );
        assert_eq!(
            native.session.native_input_recorded_count(),
            recorded,
            "the cancelled physical release is a suppressed tail"
        );
        let released = idle(&mut native);
        assert!(released.manipulation_feedback().is_empty());
        let event = if desktop {
            raw_desktop_event(source, Some(ElementState::Pressed))
        } else {
            raw_event(source, Some(ElementState::Pressed), 2.0)
        };
        native
            .bridge
            .push_record(HostRecord::WindowEvent(capture(&native, binding, 4, event)));
        reduce_one(&mut native);
        input_boundary(&mut native, &scene, CAPTURED_AT);
        assert_eq!(
            native.session.native_pointer_gesture_source(),
            Some((pointer, binding)),
            "the same native pointer can begin a fresh committed press"
        );
    }
}
