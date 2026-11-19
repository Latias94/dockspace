//! Native keyboard controls and release retain callback order without a UI pass.

use crate::event::NativeKeyboardControlRecord;
use dockspace::runtime::DockspaceManipulationCancelReason as CancelReason;

use super::*;

fn keyboard_control(
    native: &NativeCoordinator,
    binding: NativeSurfaceBinding,
    ordinal: u64,
    key: egui::Key,
    pressed: bool,
) -> HostRecord {
    native.bridge.with_viewports(|routes| {
        let viewport = routes.viewport_for_window(binding.window()).unwrap();
        HostRecord::KeyboardControl(NativeKeyboardControlRecord::for_test(
            ordinal,
            routes.window(viewport).unwrap(),
            Some(viewport),
            binding,
            key,
            pressed,
        ))
    })
}

fn drain_callbacks_without_painting(
    native: &mut NativeCoordinator,
    receivers: &RecordedSource,
) -> Vec<HostFrameReport> {
    let context = egui::Context::default();
    let mut reports = Vec::new();
    for _ in 0..8 {
        if !native.bridge.has_pending_input() {
            assert!(!native.session.has_pending_native_input());
            return reports;
        }
        assert!(
            native
                .reduce_callback_prefix(&context, &mut test_budget())
                .unwrap()
                .is_some(),
            "each committed callback makes progress without painting egui",
        );
        let frame = native
            .begin_host_frame(|query| resolve(query, receivers))
            .unwrap();
        reports.push(finish(frame));
    }
    panic!("the bounded keyboard/release prefix failed to drain");
}

#[test]
fn shared_window_cancel_uses_the_current_window_after_placement_rebinding() {
    for cancel_first in [true, false] {
        let clock = dockspace::runtime::NativeMonotonicClock::new(|| Duration::from_millis(40));
        let (mut native, transport) = shared_routes_with_coordinator(coordinator_with_clock(clock));
        assert!(!native.session.is_current_native_binding(transport));
        assert!(native.current_window_route_for_binding(transport).is_some());
        let (receivers, source, target) = source_and_target(&mut native);
        let version = native.session.version();
        for (ordinal, point, state) in [(1, source, Some(ElementState::Pressed)), (2, target, None)]
        {
            record_pointer(
                &mut native,
                transport,
                ordinal,
                mouse_event(point, state, 22, None),
            );
        }
        let mut frame = native
            .begin_host_frame(|query| resolve(query, &receivers))
            .unwrap();
        assert!(matches!(
            preview(&mut frame),
            Some(DockspacePreviewVisual::Dock { .. })
        ));
        assert!(!finish(frame).workspace_changed());

        let cancel = keyboard_control(
            &native,
            transport,
            if cancel_first { 3 } else { 4 },
            egui::Key::Escape,
            true,
        );
        let release = HostRecord::WindowEvent(captured_record(
            &native,
            transport,
            if cancel_first { 4 } else { 3 },
            mouse_event(target, Some(ElementState::Released), 22, None),
        ));
        for callback in if cancel_first {
            [cancel, release]
        } else {
            [release, cancel]
        } {
            native.bridge.push_record(callback);
        }
        let reports = drain_callbacks_without_painting(&mut native, &receivers);
        let terminals: Vec<_> = reports
            .iter()
            .flat_map(|report| report.manipulation_feedback())
            .map(|feedback| feedback.status())
            .collect();
        assert_eq!(
            terminals,
            [if cancel_first {
                Status::Cancelled(CancelReason::ExplicitCancellation)
            } else {
                Status::Delivered(Delivery::Dock)
            }]
        );
        assert_eq!(native.session.version() == version, cancel_first);
        assert!(!native.session.has_active_gesture());
        assert!(!native.session.has_pending_presentation());
        assert_eq!(
            native
                .session
                .view()
                .item(ItemId::new(2))
                .unwrap()
                .surface(),
            if cancel_first {
                SECOND_SURFACE
            } else {
                FIRST_SURFACE
            }
        );
    }
}

#[test]
fn peer_modifier_survives_source_pointer_motion() {
    for (peer_modifiers, source_modifiers, cancels) in [
        (ModifiersState::CONTROL, ModifiersState::CONTROL, true),
        (ModifiersState::CONTROL, ModifiersState::empty(), true),
        (ModifiersState::empty(), ModifiersState::CONTROL, false),
    ] {
        let (mut native, peer, source) = presented_routes();
        native.set_input(egui_dockspace::DockInputConfig {
            cancel_drag: Some(egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::Q,
            )),
            ..Default::default()
        });
        let (receivers, origin, target) = source_and_target(&mut native);
        apply_input_callback(
            &mut native,
            source,
            1,
            mouse_event(origin, Some(ElementState::Pressed), 22, None),
            &receivers,
        );
        apply_input_callback(
            &mut native,
            source,
            2,
            mouse_event(target, None, 11, None),
            &receivers,
        );
        let controls = [
            HostRecord::WindowEvent(captured_record(
                &native,
                peer,
                3,
                WindowEvent::ModifiersChanged(peer_modifiers.into()),
            )),
            HostRecord::WindowEvent(captured_record(
                &native,
                source,
                4,
                mouse_event(target, None, 11, Some(source_modifiers)),
            )),
            keyboard_control(&native, peer, 5, egui::Key::Q, true),
            HostRecord::WindowEvent(captured_record(
                &native,
                source,
                6,
                mouse_event(target, Some(ElementState::Released), 11, None),
            )),
        ];
        for control in controls {
            native.bridge.push_record(control);
        }
        let reports = drain_callbacks_without_painting(&mut native, &receivers);
        let outcomes: Vec<_> = reports
            .iter()
            .flat_map(|report| report.manipulation_feedback())
            .map(|feedback| feedback.status())
            .collect();
        let expected = if cancels {
            Status::Cancelled(CancelReason::ExplicitCancellation)
        } else {
            Status::Delivered(Delivery::Dock)
        };
        assert_eq!(outcomes, [expected]);
    }
}

#[test]
fn configured_cancel_and_mouseup_share_order_without_an_intermediate_ui_pass() {
    for custom_shortcut in [false, true] {
        for keyboard_on_source in [false, true] {
            for cancel_first in [false, true] {
                let (mut native, peer_binding, source_binding) = presented_routes();
                let modifiers = if custom_shortcut {
                    ModifiersState::CONTROL
                } else {
                    ModifiersState::empty()
                };
                let key = if custom_shortcut {
                    egui::Key::Q
                } else {
                    egui::Key::Escape
                };
                native.set_input(egui_dockspace::DockInputConfig {
                    cancel_drag: Some(egui::KeyboardShortcut::new(
                        if custom_shortcut {
                            egui::Modifiers::CTRL
                        } else {
                            egui::Modifiers::NONE
                        },
                        key,
                    )),
                    ..Default::default()
                });
                let (receivers, source, target) = source_and_target(&mut native);
                let version = native.session.version();
                for (ordinal, point, state, hover) in [
                    (1, source, Some(ElementState::Pressed), 22),
                    (2, target, None, 11),
                ] {
                    let (preview, _) = apply_input_callback(
                        &mut native,
                        source_binding,
                        ordinal,
                        mouse_event(point, state, hover, None),
                        &receivers,
                    );
                    if state.is_none() {
                        assert!(matches!(preview, Some(DockspacePreviewVisual::Dock { .. })));
                    }
                }
                assert!(native.session.native_pointer_capture_request().is_some());
                let keyboard_binding = if keyboard_on_source {
                    source_binding
                } else {
                    peer_binding
                };
                let modifier_record = captured_record(
                    &native,
                    keyboard_binding,
                    3,
                    WindowEvent::ModifiersChanged(modifiers.into()),
                );
                native
                    .bridge
                    .push_record(HostRecord::WindowEvent(modifier_record));
                let keyboard = keyboard_control(
                    &native,
                    keyboard_binding,
                    if cancel_first { 4 } else { 5 },
                    key,
                    true,
                );
                let release = HostRecord::WindowEvent(captured_record(
                    &native,
                    source_binding,
                    if cancel_first { 5 } else { 4 },
                    mouse_event(target, Some(ElementState::Released), 11, None),
                ));
                let events = if cancel_first {
                    [keyboard, release]
                } else {
                    [release, keyboard]
                };
                for event in events {
                    native.bridge.push_record(event);
                }
                // Both callbacks precede reduction. In particular there is no
                // egui run/pass between the key and physical mouse release.
                let reports = drain_callbacks_without_painting(&mut native, &receivers);
                assert_eq!(reports.len(), 3);
                let terminals: Vec<_> = reports
                    .iter()
                    .flat_map(|report| report.manipulation_feedback())
                    .collect();
                assert_eq!(terminals.len(), 1);
                if cancel_first {
                    assert_eq!(
                        terminals[0].status(),
                        Status::Cancelled(CancelReason::ExplicitCancellation)
                    );
                    assert!(reports.iter().all(|report| !report.workspace_changed()));
                    assert_eq!(native.session.version(), version);
                } else {
                    assert_eq!(terminals[0].status(), Status::Delivered(Delivery::Dock));
                    assert_ne!(native.session.version(), version);
                    assert!(reports[2].manipulation_feedback().is_empty());
                }
                assert!(!native.session.has_active_gesture());
                assert!(!native.session.has_pending_presentation());
                assert_eq!(
                    native
                        .session
                        .view()
                        .item(ItemId::new(2))
                        .unwrap()
                        .surface(),
                    if cancel_first {
                        SECOND_SURFACE
                    } else {
                        FIRST_SURFACE
                    },
                );
                if cancel_first {
                    let (receivers, source, target) = source_and_target(&mut native);
                    for (ordinal, point, state, hover) in [
                        (6, source, Some(ElementState::Pressed), 22),
                        (7, target, None, 11),
                    ] {
                        apply_input_callback(
                            &mut native,
                            source_binding,
                            ordinal,
                            mouse_event(point, state, hover, Some(modifiers)),
                            &receivers,
                        );
                    }
                    assert!(
                        native.session.native_pointer_capture_request().is_some(),
                        "the same pointer must begin another real drag after its cancelled release"
                    );
                    native.bridge.push_record(keyboard_control(
                        &native,
                        source_binding,
                        8,
                        key,
                        true,
                    ));
                    let terminal = drain_callbacks_without_painting(&mut native, &receivers);
                    assert_eq!(terminal.len(), 1);
                    assert!(
                        matches!(terminal[0].manipulation_feedback(), [feedback] if feedback.status() == Status::Cancelled(CancelReason::ExplicitCancellation))
                    );
                    assert!(!native.session.has_active_gesture());
                }
            }
        }
    }
}

#[test]
fn disabled_mismatched_and_released_shortcuts_leave_the_real_mouse_release_effective() {
    for (shortcut, key, pressed) in [
        (None, egui::Key::Escape, true),
        (
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
            egui::Key::Q,
            true,
        ),
        (
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::Q,
            )),
            egui::Key::Q,
            true,
        ),
        (
            Some(egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
            egui::Key::Escape,
            false,
        ),
    ] {
        let (mut native, _, binding) = presented_routes();
        native.set_input(egui_dockspace::DockInputConfig {
            cancel_drag: shortcut,
            ..Default::default()
        });
        let (receivers, source, target) = source_and_target(&mut native);
        for (ordinal, point, state, hover) in [
            (1, source, Some(ElementState::Pressed), 22),
            (2, target, None, 11),
        ] {
            apply_input_callback(
                &mut native,
                binding,
                ordinal,
                mouse_event(point, state, hover, None),
                &receivers,
            );
        }
        native
            .bridge
            .push_record(keyboard_control(&native, binding, 3, key, pressed));
        native
            .bridge
            .push_record(HostRecord::WindowEvent(captured_record(
                &native,
                binding,
                4,
                mouse_event(target, Some(ElementState::Released), 11, None),
            )));
        let reports = drain_callbacks_without_painting(&mut native, &receivers);
        assert_eq!(reports.len(), 2);
        assert!(reports[0].manipulation_feedback().is_empty());
        assert!(
            matches!(reports[1].manipulation_feedback(), [feedback] if feedback.status() == Status::Delivered(Delivery::Dock))
        );
        assert!(!native.session.has_active_gesture());
    }
}

#[test]
fn a_keyboard_head_waits_for_earlier_retained_core_input_to_commit() {
    let (mut native, _, binding) = presented_routes();
    let (receivers, source, target) = source_and_target(&mut native);
    for (ordinal, point, state, hover) in [
        (1, source, Some(ElementState::Pressed), 22),
        (2, target, None, 11),
    ] {
        record_pointer(
            &mut native,
            binding,
            ordinal,
            mouse_event(point, state, hover, None),
        );
    }
    native.bridge.push_record(keyboard_control(
        &native,
        binding,
        3,
        egui::Key::Escape,
        true,
    ));
    let mut budget = test_budget();
    let remaining = budget.remaining_records();
    assert!(
        native
            .reduce_callback_prefix(&egui::Context::default(), &mut budget)
            .unwrap()
            .is_none()
    );
    assert_eq!(budget.remaining_records(), remaining);
    assert!(matches!(
        native.bridge.next_callback(),
        Some(crate::mailbox::NativeCallbackHead::KeyboardControl(_, _))
    ));
    finish(
        native
            .begin_host_frame(|query| resolve(query, &receivers))
            .unwrap(),
    );
    assert!(native.session.has_active_gesture());
    let reports = drain_callbacks_without_painting(&mut native, &receivers);
    assert!(
        matches!(reports[0].manipulation_feedback(), [feedback] if feedback.status() == Status::Cancelled(CancelReason::ExplicitCancellation))
    );
    assert!(!native.session.has_active_gesture());
}

#[test]
fn an_ordered_cancel_left_in_egui_input_cannot_cancel_a_later_native_drag() {
    struct Panes;

    impl egui_dockspace::PaneView for Panes {
        fn title(&self, _: ItemId) -> Option<egui::WidgetText> {
            Some("Ordered native keyboard control".into())
        }

        fn ui(&mut self, _: ItemId, ui: &mut egui::Ui) -> egui_dockspace::PaneResponse {
            ui.label("The keyboard event remains available to application widgets.");
            egui_dockspace::PaneResponse::default()
        }
    }

    let (mut native, _, binding) = presented_routes();
    let (receivers, source, target) = source_and_target(&mut native);
    for (ordinal, point, state, hover) in [
        (1, source, Some(ElementState::Pressed), 22),
        (2, target, None, 11),
    ] {
        apply_input_callback(
            &mut native,
            binding,
            ordinal,
            mouse_event(point, state, hover, None),
            &receivers,
        );
    }
    let previous = native.session.native_pointer_capture_request().unwrap();
    native.bridge.push_record(keyboard_control(
        &native,
        binding,
        3,
        egui::Key::Escape,
        true,
    ));
    for (ordinal, point, state, hover) in [
        (4, target, Some(ElementState::Released), 11),
        (5, source, Some(ElementState::Pressed), 22),
        (6, target, None, 11),
    ] {
        native
            .bridge
            .push_record(HostRecord::WindowEvent(captured_record(
                &native,
                binding,
                ordinal,
                mouse_event(point, state, hover, None),
            )));
    }
    let reports = drain_callbacks_without_painting(&mut native, &receivers);
    assert_eq!(reports.len(), 4);
    assert!(
        matches!(reports[0].manipulation_feedback(), [feedback] if feedback.status() == Status::Cancelled(CancelReason::ExplicitCancellation))
    );
    let current = native.session.native_pointer_capture_request().unwrap();
    assert_ne!(previous, current);
    assert_eq!(previous.pointer(), current.pointer());
    assert!(native.session.has_active_gesture());

    let context = egui::Context::default();
    context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let viewport = native.surface_viewport(SECOND_SURFACE).unwrap();
    let mut input = egui::RawInput {
        viewport_id: viewport,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 480.0),
        )),
        focused: true,
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    input.viewports.entry(viewport).or_default().focused = Some(true);
    let mut frame = native
        .begin_host_frame(|query| resolve(query, &receivers))
        .unwrap();
    let mut paint = None;
    let mut panes = Panes;
    let mut original_key_visible = [false; 2];
    let has_original_key = |ui: &egui::Ui| {
        ui.input(|state| {
            state.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            })
        })
    };
    let mut output = context.run_ui(input, |ui| {
        original_key_visible[0] = has_original_key(ui);
        paint = Some(
            frame
                .paint_surface(
                    egui::Id::unique("native-control-replay"),
                    SECOND_SURFACE,
                    ui,
                    &mut panes,
                    &egui_dockspace::DockStyle::default(),
                )
                .unwrap(),
        );
        original_key_visible[1] = has_original_key(ui);
    });
    let mut paint = paint.unwrap();
    let valid_output = paint.validate_final_output(&mut output);
    output.drop_without_applying_deltas();
    assert!(valid_output);
    assert!(paint.confirm_into(&mut frame.frame).unwrap());
    assert!(
        paint.take_local_actions().is_empty(),
        "painting the old raw key must not prepare cancellation of the new native gesture"
    );
    assert_eq!(
        original_key_visible,
        [true, true],
        "native docking must leave application keyboard input available"
    );
    let report = finish(frame);
    assert!(report.manipulation_feedback().is_empty());
    assert_eq!(
        native.session.native_pointer_capture_request(),
        Some(current)
    );
}
