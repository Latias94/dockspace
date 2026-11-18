//! Keyboard controls target an exact resize without needing a stationary request.

use super::*;
use crate::event::NativeKeyboardControlRecord;

fn editor_split_weights(native: &NativeCoordinator) -> [Vec<u64>; 2] {
    let columns = native
        .session
        .view()
        .root(RootId::new(1))
        .unwrap()
        .content()
        .unwrap()
        .split()
        .unwrap();
    let rows = columns.children().nth(1).unwrap().split().unwrap();
    [
        columns.weights().map(f64::to_bits).collect(),
        rows.weights().map(f64::to_bits).collect(),
    ]
}

#[test]
fn ordered_keyboard_cancel_ends_local_or_desktop_resize_and_allows_a_fresh_resize() {
    for desktop in [false, true] {
        for axis in [DockspaceAxis::Horizontal, DockspaceAxis::Vertical] {
            let (mut native, binding, _) = editor_fixture(desktop);
            let scene = present(&mut native, binding);
            let weights = editor_split_weights(&native);
            assert_eq!(
                scene.panes.len(),
                4,
                "the editor paints all four active panes"
            );
            let (_, visual, receiver) = *scene
                .splitters
                .iter()
                .find(|splitter| splitter.0 == axis)
                .unwrap();
            let bounds = receiver.bounds();
            let start = match axis {
                DockspaceAxis::Horizontal => {
                    LogicalPoint::new(receiver.center().x(), bounds.y() + bounds.height() * 0.25)
                }
                DockspaceAxis::Vertical => {
                    LogicalPoint::new(bounds.x() + bounds.width() * 0.125, receiver.center().y())
                }
            }
            .unwrap();
            assert!(
                !scene
                    .junctions
                    .iter()
                    .any(|junction| junction.contains(start))
            );
            assert!(
                !scene
                    .splitters
                    .iter()
                    .any(|(_, other, receiver)| *other != visual
                        && receiver.bounds().contains(start))
            );
            let target = LogicalPoint::new(
                start.x()
                    + if axis == DockspaceAxis::Horizontal {
                        18.0
                    } else {
                        0.0
                    },
                start.y()
                    + if axis == DockspaceAxis::Vertical {
                        18.0
                    } else {
                        0.0
                    },
            )
            .unwrap();
            let event = |point, state| {
                if desktop {
                    raw_desktop_event(point, state)
                } else {
                    raw_event(point, state, 2.0)
                }
            };
            let before = native.session.version();
            let mut ordinal = 0;
            for (point, state) in [(start, Some(ElementState::Pressed)), (target, None)] {
                let report = apply(
                    &mut native,
                    binding,
                    &scene,
                    &receiver,
                    &mut ordinal,
                    event(point, state),
                    true,
                );
                assert!(report.manipulation_feedback().is_empty());
            }
            assert!(native.session.has_active_gesture());
            let (pointer, source) = native.session.native_pointer_gesture_source().unwrap();
            assert_eq!(source, binding);
            assert!(native.session.native_pointer_capture_request().is_none());
            assert!(native.session.native_pointer_continuation().is_none());
            ordinal += 1;
            native.bridge.push_record(HostRecord::KeyboardControl(
                NativeKeyboardControlRecord::for_test(
                    ordinal,
                    WindowId::from(WINDOW),
                    Some(ViewportId::ROOT),
                    binding,
                    egui::Key::Escape,
                    true,
                ),
            ));
            ordinal += 1;
            native.bridge.push_record(HostRecord::WindowEvent(capture(
                &native,
                binding,
                ordinal,
                event(target, Some(ElementState::Released)),
            )));
            reduce_one(&mut native);
            assert!(
                native.session.native_pointer_gesture_source().is_none(),
                "the retained cancellation has not committed yet"
            );
            let cancelled = idle(&mut native);
            assert!(
                matches!(cancelled.manipulation_feedback(), [feedback] if feedback.status() == Status::Cancelled(CancelReason::ExplicitCancellation))
            );
            assert!(!native.session.has_active_gesture());
            assert!(native.session.native_pointer_gesture_source().is_none());
            assert_eq!(native.session.version(), before);
            let recorded = native.session.native_input_recorded_count();
            assert!(
                native
                    .reduce_callback_prefix(&egui::Context::default(), &mut test_budget())
                    .unwrap()
                    .is_some()
            );
            assert_eq!(native.session.native_input_recorded_count(), recorded);
            assert!(idle(&mut native).manipulation_feedback().is_empty());

            let fresh = present(&mut native, binding);
            assert_eq!(
                editor_split_weights(&native),
                weights,
                "cancellation preserves both nested splits' exact shares"
            );
            assert_eq!(
                fresh.panes, scene.panes,
                "the same editor measurements restore every pane's full geometry"
            );
            let (_, _, fresh_receiver) = *fresh
                .splitters
                .iter()
                .find(|splitter| splitter.1 == visual)
                .unwrap();
            assert_eq!(
                fresh_receiver.center(),
                receiver.center(),
                "cancellation restores the uncommitted splitter geometry"
            );
            for (point, state) in [
                (start, Some(ElementState::Pressed)),
                (target, None),
                (target, Some(ElementState::Released)),
            ] {
                let report = apply(
                    &mut native,
                    binding,
                    &fresh,
                    &fresh_receiver,
                    &mut ordinal,
                    event(point, state),
                    true,
                );
                if state == Some(ElementState::Pressed) {
                    assert_eq!(
                        native.session.native_pointer_gesture_source(),
                        Some((pointer, binding))
                    );
                }
                if state == Some(ElementState::Released) {
                    assert!(
                        matches!(report.manipulation_feedback(), [feedback] if feedback.status() == Status::Delivered(Delivery::SplitterResize))
                    );
                }
            }
            assert_ne!(native.session.version(), before);
            assert!(!native.session.has_active_gesture());
            assert!(native.session.native_pointer_gesture_source().is_none());
        }
    }
}
