//! Fatal cleanup must make progress after rendering has stopped.

use super::*;

#[test]
fn standalone_service_drains_fatal_native_work_without_an_output_scope() {
    let mut state = test_state();
    let context = egui::Context::default();
    let mut frame = state
        .coordinator
        .begin_host_frame(|_| dockspace::runtime::NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit().unwrap();
    state.coordinator.settle_host_frame_report(&report).unwrap();
    let version = state.coordinator.session().version();
    let before = state.view().to_layout();
    let window = state
        .coordinator
        .session()
        .native_window_bindings()
        .next()
        .unwrap();
    let binding = state
        .coordinator
        .session()
        .native_surface_bindings_for_window(window)
        .next()
        .unwrap();
    state
        .coordinator
        .bind_viewport(ViewportId::ROOT, winit::window::WindowId::from(11), binding)
        .unwrap();
    state.stop_current_output(NativeHostProtocolError::InvalidWindowSnapshot.into());
    let cause = format!("{:?}", state.error().unwrap());
    let bridge = state.coordinator.dispatch_bridge();
    let record = bridge.with_viewports_mut(|routes| {
        crate::event::NativeWindowEventRecord::for_test_ingress(
            1,
            winit::window::WindowId::from(11),
            Some(ViewportId::ROOT),
            winit::event::WindowEvent::Destroyed,
            routes,
        )
    });
    bridge.push_record(crate::mailbox::HostRecord::WindowEvent(record));
    state.service_native_events(&context).unwrap();
    assert!(
        !state.is_shutdown_drained(),
        "Destroyed still requires a fresh complete roster"
    );
    assert_eq!(
        state.service_native_events(&context).unwrap(),
        NativeHostWake::Wait
    );
    assert!(
        !state.is_shutdown_drained(),
        "missing roster authority must not be guessed"
    );
    bridge.push_terminal_viewport_roster(
        crate::mailbox::NativeViewportRosterRecord::for_test_with_backend_and_work_areas(
            [],
            crate::work_area::FrozenWorkAreaRoster::Unknown,
            eframe::NativeWindowingBackend::X11,
        ),
    );
    assert!(!state.is_shutdown_drained());
    let pass = context.cumulative_pass_nr_for(ViewportId::ROOT);
    for _ in 0..32 {
        state.service_native_events(&context).unwrap();
        if state.is_shutdown_drained() {
            break;
        }
    }
    assert!(state.is_shutdown_drained(), "{:?}", state.pending_work());
    assert_eq!(state.coordinator.session().version(), version);
    assert_eq!(
        state.view().item(ItemId::new(1)).unwrap().tabs().items(),
        before.view().item(ItemId::new(1)).unwrap().tabs().items()
    );
    assert_eq!(format!("{:?}", state.error().unwrap()), cause);
    state.with_errors(|_, cleanup| assert!(cleanup.is_empty(), "{cleanup:?}"));
    assert!(eframe::current_native_output_token().is_none());
    assert_eq!(context.cumulative_pass_nr_for(ViewportId::ROOT), pass);
    assert_eq!(state.output_backlog().ordinary(), 0);
    assert_eq!(state.output_backlog().control(), 0);
}

#[test]
fn fatal_service_commits_the_first_queued_backend_roster_before_terminal_inventory() {
    use dockspace::geometry::{PhysicalRect, ScaleFactor};
    use dockspace::runtime::{NativeWindowFacts, NativeWindowPresentationState};

    for valid_geometry in [false, true] {
        let mut state = test_state();
        let context = egui::Context::default();
        let mut frame = state
            .coordinator
            .begin_host_frame(|_| dockspace::runtime::NativeReceiverAnswer::Unknown)
            .unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        let report = frame.commit().unwrap();
        state.coordinator.settle_host_frame_report(&report).unwrap();
        let binding = report.native_bindings()[0];
        let window = winit::window::WindowId::from(11);
        state
            .coordinator
            .bind_viewport(ViewportId::ROOT, window, binding)
            .unwrap();
        let bounds = PhysicalRect::new(0.0, 0.0, 640.0, 480.0).unwrap();
        let scale = ScaleFactor::new(1.0).unwrap();
        let facts = NativeWindowFacts::live()
            .with_content_bounds(bounds)
            .with_outer_bounds(bounds)
            .with_native_scale_factor(scale)
            .with_presentation_scale_factor(scale)
            .with_presentation(NativeWindowPresentationState::Visible);
        let bridge = state.coordinator.dispatch_bridge();
        // Compilation is controlled; ingress ordering, backend configuration,
        // shutdown frames and terminal snapshot settlement use production paths.
        bridge.push_viewport_roster(crate::mailbox::NativeViewportRosterRecord::for_test(
            [crate::window_snapshot::CompiledWindowObservation::new(
                binding, facts,
            )],
            valid_geometry,
        ));
        let version = state.coordinator.session().version();
        state.stop_current_output(NativeHostProtocolError::InvalidWindowSnapshot.into());
        let cause = format!("{:?}", state.error().unwrap());
        let destroyed = bridge.with_viewports_mut(|routes| {
            crate::event::NativeWindowEventRecord::for_test_ingress(
                100,
                window,
                Some(ViewportId::ROOT),
                winit::event::WindowEvent::Destroyed,
                routes,
            )
        });
        bridge.push_record(crate::mailbox::HostRecord::WindowEvent(destroyed));
        let pass = context.cumulative_pass_nr_for(ViewportId::ROOT);
        for _ in 0..32 {
            let wake = state.service_native_events(&context).unwrap();
            if state.is_shutdown_drained() || wake == NativeHostWake::Wait {
                break;
            }
        }
        assert!(
            state.is_shutdown_drained(),
            "valid_geometry={valid_geometry}: {:?}",
            state.pending_work()
        );
        assert_eq!(state.coordinator.session().version(), version);
        assert!(state.view().item(ItemId::new(1)).is_some());
        assert_eq!(format!("{:?}", state.error().unwrap()), cause);
        state.with_errors(|_, cleanup| assert!(cleanup.is_empty(), "{cleanup:?}"));
        assert_eq!(context.cumulative_pass_nr_for(ViewportId::ROOT), pass);
        assert_eq!(state.output_backlog().ordinary(), 0);
        assert_eq!(state.output_backlog().control(), 0);
    }
}
