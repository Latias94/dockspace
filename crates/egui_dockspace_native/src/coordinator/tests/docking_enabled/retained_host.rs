//! Shared-window pointer moves preserve the host declaration at every boundary.

use super::*;

fn shared_mouse_event(position: PhysicalPosition<f64>, state: Option<ElementState>) -> WindowEvent {
    let facts = PointerEventFacts {
        surface_position: Some(position),
        desktop_position: Some(position),
        hover: PointerWindowRoute::Window(WindowId::from(22)),
        capture: PointerWindowRoute::Window(WindowId::from(22)),
        ..PointerEventFacts::default()
    };
    match state {
        Some(state) => WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            primary_press: None,
            state,
            button: MouseButton::Left,
            facts,
        },
        None => WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position,
            facts,
        },
    }
}

#[test]
fn shared_window_pointer_preserves_fixed_host_and_allows_the_reverse_move() {
    let layout = DockspaceLayout::new([FIRST_SURFACE, SECOND_SURFACE].map(|surface| {
        let id = if surface == FIRST_SURFACE { 1 } else { 2 };
        DockspaceSurfaceLayout::new(
            surface,
            DockspaceRootLayout::new(RootId::new(id), DockspaceNode::tabs([ItemId::new(id)])),
        )
    }))
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    session.retain_host_surface(SECOND_SURFACE).unwrap();
    let native = NativeCoordinator::new(session, NativePointerRoster::Exact(Vec::new())).unwrap();
    let (mut native, transport) = shared_routes_with_coordinator(native);
    let (receivers, source, target) = source_and_target(&mut native);
    let version = native.session.version();

    for (ordinal, point, state) in [
        (1, source, Some(ElementState::Pressed)),
        (2, target, None),
        (3, target, Some(ElementState::Released)),
    ] {
        let (visual, report) = apply_input_callback(
            &mut native,
            transport,
            ordinal,
            shared_mouse_event(point, state),
            &receivers,
        );
        assert!(
            !matches!(visual, Some(DockspacePreviewVisual::Dock { .. })),
            "a last-tab move cannot preview the retirement of the fixed host",
        );
        assert!(!report.workspace_changed());
        assert_eq!(native.session.version(), version);
        if state.is_none() {
            assert!(native.session.has_active_gesture());
        }
        assert_eq!(
            native
                .session
                .view()
                .item(ItemId::new(2))
                .unwrap()
                .surface(),
            SECOND_SURFACE,
        );
        assert!(native.session.view().surface(SECOND_SURFACE).is_some());
        assert!(native.session.view().root(RootId::new(2)).is_some());
        assert!(!native.session.has_pending_presentation());
    }
    assert!(!native.session.has_active_gesture());
    assert!(!native.session.has_pending_native_input());

    let (receivers, source, target) =
        source_and_target_for(&mut native, FIRST_SURFACE, ItemId::new(1), SECOND_SURFACE);
    let mut delivered = false;
    for (ordinal, point, state) in [
        (4, source, Some(ElementState::Pressed)),
        (5, target, None),
        (6, target, Some(ElementState::Released)),
    ] {
        record_pointer(
            &mut native,
            transport,
            ordinal,
            shared_mouse_event(point, state),
        );
        let mut frame = native
            .begin_host_frame(|query| resolve(query, &receivers))
            .unwrap();
        let visual = preview_for(&mut frame, SECOND_SURFACE);
        if state.is_none() {
            assert!(matches!(
                visual,
                Some(DockspacePreviewVisual::Dock {
                    surface: SECOND_SURFACE,
                    ..
                }),
            ));
        }
        let report = finish(frame);
        delivered |= report
            .manipulation_feedback()
            .iter()
            .any(|feedback| feedback.status() == Status::Delivered(Delivery::Dock));
    }
    assert!(delivered);
    assert!(native.session.view().surface(FIRST_SURFACE).is_none());
    assert!(native.session.view().root(RootId::new(1)).is_none());
    assert!(native.session.view().surface(SECOND_SURFACE).is_some());
    assert!(native.session.view().root(RootId::new(2)).is_some());
    for item in [ItemId::new(1), ItemId::new(2)] {
        assert_eq!(
            native.session.view().item(item).unwrap().surface(),
            SECOND_SURFACE
        );
    }
    assert!(!native.session.has_active_gesture());
    assert!(!native.session.has_pending_presentation());
    assert!(!native.session.has_pending_native_input());
}
