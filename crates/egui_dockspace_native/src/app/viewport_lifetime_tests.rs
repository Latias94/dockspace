//! Deferred paint declarations borrow the application lifetime without retaining it.

use dockspace::geometry::PhysicalRect;
use dockspace::runtime::{
    HostInputOutcome, HostWindowToken, NativePointerRoster, NativeSurfaceRole,
    SurfaceUnavailableReason,
};

use super::*;
use crate::deferred_viewport::{DeferredViewportDriver, viewport_id_for};
use std::sync::atomic::{AtomicUsize, Ordering};

struct DropPanes(Arc<AtomicUsize>);

impl PaneView for DropPanes {
    fn title(&self, _: dockspace::model::ItemId) -> Option<egui::WidgetText> {
        Some("Document".into())
    }

    fn ui(
        &mut self,
        _: dockspace::model::ItemId,
        _: &mut egui::Ui,
    ) -> egui_dockspace::PaneResponse {
        Default::default()
    }
}

impl Drop for DropPanes {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn retained_child_callbacks_do_not_keep_the_application_runtime_alive() {
    let layout = tests::app().with_view(|view| view.to_layout());
    let style = DockStyle::default();
    let session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        Default::default(),
        style.presentation_config().unwrap(),
    )
    .unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut app = NativeDockspaceApp::new(
        Id::unique("child-callback-provider-lifetime"),
        session,
        SurfaceId::new(1),
        DropPanes(Arc::clone(&drops)),
        style,
    )
    .unwrap();
    let weak = Arc::downgrade(&app.state);
    let context = egui::Context::default();
    context.set_embed_viewports(false);

    let layout = app.with_view(|view| view.to_layout());
    let mut session = DockspaceSession::from_layout(layout, Default::default()).unwrap();
    session
        .enable_managed_native_host(NativePointerRoster::Unknown)
        .unwrap();
    session
        .register_native_root(SurfaceId::new(1), HostWindowToken::new(99))
        .unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit().unwrap();
    let binding = report
        .inputs()
        .iter()
        .find_map(|outcome| match outcome {
            HostInputOutcome::NativeSurfaceRegistered { binding } => Some(*binding),
            _ => None,
        })
        .unwrap();
    let viewport = viewport_id_for(binding);
    let mut driver = DeferredViewportDriver::default();
    assert!(driver.insert(
        viewport,
        binding,
        PhysicalRect::new(40.0, 50.0, 320.0, 240.0).unwrap(),
        NativeSurfaceRole::Child,
    ));
    let specs = driver.retained_specs();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| {
        app.declare_child_viewports(ui.ctx(), &specs);
    });
    output.textures_delta.clear();
    let callback = output.viewport_output[&viewport]
        .viewport_ui_cb
        .clone()
        .unwrap();
    assert_eq!(
        Arc::strong_count(&app.state),
        1,
        "egui's retained callback must not become a runtime owner"
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(app);
    assert!(weak.upgrade().is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let mut late_output = context.run_ui(egui::RawInput::default(), |ui| {
        callback(ui);
    });
    late_output.textures_delta.clear();
    assert!(
        weak.upgrade().is_none(),
        "a late callback cannot revive the runtime"
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    output.drop_without_applying_deltas();
    late_output.drop_without_applying_deltas();
}
