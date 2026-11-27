use std::sync::Arc;

use dockspace::geometry::{LogicalRect, LogicalSize, PhysicalPoint, PhysicalRect, ScaleFactor};
use dockspace::model::{
    DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, RootId,
};
use dockspace::runtime::{
    DockspaceSession, HostInputOutcome, HostWindowToken, NativeCloseState, NativePointerRoster,
    NativeSurfacePlacement, NativeWindowFacts, NativeWindowPresentationState, NativeWorkAreaRoster,
    SurfacePresentationResult, SurfaceUnavailableReason, UniformSurfaceMetrics,
};
use egui_dockspace::{PaneResponse, PaneView};

use super::*;
use crate::host_frame::{NativeHostFrame, retain_window_input_state};
use crate::mailbox::NativeHostBridge;
use crate::viewport_map::NativeViewportMap;

const ROOT_SURFACE: SurfaceId = SurfaceId::new(1);
const SURFACE: SurfaceId = SurfaceId::new(2);
const EDITOR: ItemId = ItemId::new(1);

struct Editor {
    target: egui::Id,
    text: String,
}

impl PaneView for Editor {
    fn title(&self, _: ItemId) -> Option<egui::WidgetText> {
        Some("Editor".into())
    }

    fn ui(&mut self, _: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        ui.add(egui::TextEdit::singleline(&mut self.text).id(self.target));
        PaneResponse::default()
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        (item == EDITOR).then_some(self.target)
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the regression crosses native snapshot, presentation retirement and real egui focus passes"
)]
fn minimized_surface_keeps_editor_focus_without_presented_receiver_authority() {
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            ROOT_SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::central_tabs([ItemId::new(2)]),
            ),
        ),
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::central_tabs([EDITOR])),
        ),
    ])
    .unwrap();
    let mut session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        Default::default(),
        egui_dockspace::DockStyle::default()
            .presentation_config()
            .unwrap(),
    )
    .unwrap();
    session
        .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
        .unwrap();
    session
        .configure_managed_native_capabilities(crate::capabilities::capabilities_for_backend(
            eframe::NativeWindowingBackend::Windows,
        ))
        .unwrap();
    session
        .register_native_root(ROOT_SURFACE, HostWindowToken::new(1))
        .unwrap();
    let mut registration = session.begin_host_frame().unwrap();
    registration
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let registration = registration.commit().unwrap();
    let initial_root = registration
        .inputs()
        .iter()
        .find_map(|outcome| match outcome {
            HostInputOutcome::NativeSurfaceRegistered { binding } => Some(*binding),
            _ => None,
        })
        .unwrap();
    let window = initial_root.window();
    let bounds = PhysicalRect::new(0.0, 0.0, 640.0, 480.0).unwrap();
    let scale = ScaleFactor::new(1.0).unwrap();
    let facts = NativeWindowFacts::live()
        .with_content_bounds(bounds)
        .with_outer_bounds(bounds)
        .with_native_scale_factor(scale)
        .with_presentation_scale_factor(scale)
        .with_presentation(NativeWindowPresentationState::Visible)
        .with_close(NativeCloseState::Clear, None);
    session
        .report_managed_native_snapshot([(window, facts)], NativeWorkAreaRoster::Unknown)
        .unwrap();
    let placements = [(ROOT_SURFACE, 0.0), (SURFACE, 320.0)].map(|(surface, x)| {
        NativeSurfacePlacement::new(
            surface,
            LogicalSize::new(320.0, 480.0).unwrap(),
            PhysicalPoint::new(x, 0.0).unwrap(),
            scale,
            PhysicalRect::new(x, 0.0, 320.0, 480.0).unwrap(),
        )
    });
    let mut shared = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    shared
        .set_native_surface_placements(window, &placements)
        .unwrap();
    shared
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    shared.commit().unwrap();
    let binding = session
        .native_surface_bindings_for_window(window)
        .find(|binding| binding.surface() == SURFACE)
        .unwrap();
    let root_binding = session
        .native_surface_bindings_for_window(window)
        .find(|binding| binding.surface() == ROOT_SURFACE)
        .unwrap();
    let mut measured = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    measured
        .measure_surface(
            SURFACE,
            UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, 320.0, 480.0).unwrap(),
                LogicalSize::new(32.0, 24.0).unwrap(),
                80.0,
            )
            .unwrap(),
        )
        .unwrap();
    measured.commit().unwrap();
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let execution = frame
        .paint_plan(SURFACE)
        .unwrap()
        .unwrap()
        .execute_surface_program(|_| Ok::<(), ()>(()))
        .unwrap();
    frame.confirm_surface_painted(execution).unwrap();
    let output = frame
        .commit()
        .unwrap()
        .take_painted_outputs()
        .pop()
        .unwrap();
    let artifact = output.presentation_artifact();

    let context = egui::Context::default();
    let mut editor = Editor {
        target: egui::Id::unique("minimized-editor"),
        text: "original".into(),
    };
    context
        .run_ui(egui::RawInput::default(), |ui| {
            editor.ui(EDITOR, ui);
            ui.memory_mut(|memory| memory.request_focus(editor.target));
        })
        .drop_without_applying_deltas();
    let mut receivers = NativeReceiverStore::default();
    receivers.presented.insert(
        SURFACE,
        vec![PresentedReceiverSet {
            binding,
            viewport: egui::ViewportId::ROOT,
            pixels_per_point: f64::from(context.pixels_per_point()),
            hit_authority: Arc::new(
                context
                    .capture_completed_pass_hit_authority(egui::ViewportId::ROOT)
                    .unwrap(),
            ),
            artifact,
            receivers: Vec::new(),
            scroll_receivers: Vec::new(),
            focus_bindings: vec![(EDITOR, editor.target)],
        }],
    );
    session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    let bridge = Arc::new(NativeHostBridge::new(NativeViewportMap::default()));
    bridge
        .bind_viewport_route(
            egui::ViewportId::ROOT,
            winit::window::WindowId::from(1),
            root_binding,
        )
        .unwrap();
    let mut settled = NativeHostFrame::new(
        session
            .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap(),
        Arc::clone(&bridge),
        &mut receivers,
        None,
    );
    settled
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    settled.commit().unwrap();
    assert!(session.presented_surface(SURFACE).is_some());
    assert!(
        !receivers
            .presented_focus_bindings(
                SURFACE,
                egui::ViewportId::ROOT,
                session.presented_surface(SURFACE)
            )
            .is_empty()
    );

    let mut successor = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let execution = successor
        .paint_plan(SURFACE)
        .unwrap()
        .unwrap()
        .execute_surface_program(|_| Ok::<(), ()>(()))
        .unwrap();
    successor.confirm_surface_painted(execution).unwrap();
    let successor = successor
        .commit()
        .unwrap()
        .take_painted_outputs()
        .pop()
        .unwrap();
    session
        .report_surface_presentation(successor, SurfacePresentationResult::Dropped)
        .unwrap();

    session
        .report_managed_native_snapshot(
            [(
                window,
                facts.with_presentation(NativeWindowPresentationState::Minimized),
            )],
            NativeWorkAreaRoster::Unknown,
        )
        .unwrap();
    let mut skipped = NativeHostFrame::new(
        session
            .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap(),
        Arc::clone(&bridge),
        &mut receivers,
        None,
    );
    assert!(skipped.frame.presented_surface(SURFACE).is_none());
    assert!(
        skipped
            .presented_focus_bindings(SURFACE, egui::ViewportId::ROOT)
            .is_empty()
    );
    skipped
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    skipped.commit().unwrap();
    assert!(
        receivers.presented.is_empty(),
        "the minimized scene retires the prior receiver artifact"
    );

    session
        .report_managed_native_snapshot([(window, facts)], NativeWorkAreaRoster::Unknown)
        .unwrap();
    let mut skipped = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    assert!(skipped.presented_surface(SURFACE).is_none());
    skipped
        .set_native_surface_placements(window, &placements[..1])
        .unwrap();
    assert!(
        skipped
            .native_surface_bindings_for_window(window)
            .all(|binding| binding.surface() != SURFACE)
    );
    let retained_focus = receivers.presented_focus_bindings(
        SURFACE,
        egui::ViewportId::ROOT,
        skipped.presented_surface(SURFACE),
    );
    skipped
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let skipped = skipped.suspend();
    assert!(
        !session.is_current_native_binding(initial_root),
        "shared-window placement rebinding supersedes the initial root surface generation while keeping its physical window live"
    );
    for pass in 0..2 {
        context
            .run_ui(egui::RawInput::default(), |_| {
                retain_window_input_state(
                    &session,
                    &context,
                    egui::Id::unique("minimized-workspace"),
                    // Pass both the original physical-window route tag and the
                    // current placement binding across the two skipped passes.
                    if pass == 0 {
                        initial_root
                    } else {
                        root_binding
                    },
                    &editor,
                );
                assert!(
                    context.viewport(|viewport| viewport
                        .this_pass
                        .widgets
                        .get(editor.target)
                        .is_none())
                );
                if pass == 1 {
                    assert_eq!(
                        egui_dockspace::native_support::sample_host_focus(
                            &context,
                            session.view().surface(SURFACE).unwrap(),
                            &editor,
                            egui::Id::unique("minimized-workspace"),
                            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0)),
                            retained_focus,
                        ),
                        dockspace::runtime::DockspaceHostFocus::Unknown,
                        "identity liveness alone must not become presentation-qualified focus evidence",
                    );
                }
            })
            .drop_without_applying_deltas();
        assert!(
            context.memory(|memory| memory.has_focus(editor.target)),
            "skipped pass {pass} must keep the existing editor identity alive"
        );
    }
    session.resume_update(skipped).unwrap().commit().unwrap();
    assert!(session.presented_surface(SURFACE).is_none());
    context
        .run_ui(
            egui::RawInput {
                events: vec![egui::Event::Text("~restored~".into())],
                ..Default::default()
            },
            |ui| {
                editor.ui(EDITOR, ui);
            },
        )
        .drop_without_applying_deltas();
    assert!(editor.text.contains("original"));
    assert!(
        editor.text.contains("~restored~"),
        "the returning editor must accept text without a second focus request"
    );
}
