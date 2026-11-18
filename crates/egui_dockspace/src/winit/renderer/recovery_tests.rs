//! Real X11/WGPU recovery with injected acquisition results and ordinary egui input.
//!
//! The GPU uploads and presents real outputs; input is framework input, not OS mouse delivery.

use std::{collections::BTreeMap, sync::mpsc};

use egui::{Color32, Context, Event, FullOutput, Pos2, RawInput, Rect, TextureHandle, Ui, vec2};
use web_time::Instant;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::x11::EventLoopBuilderExtX11,
    window::WindowId,
};

use super::*;
use crate::winit::{
    frame::{settle_presentation, stage_output},
    repaint::{Repaint, RepaintRequest},
};
use crate::{
    Dockspace, DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);

struct Panes {
    image: Option<egui::TextureId>,
    label: &'static str,
}

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(format!("Document {item}").into())
    }

    fn ui(&mut self, _: ItemId, ui: &mut Ui) -> PaneResponse {
        ui.label(self.label);
        if let Some(image) = self.image {
            ui.image((image, vec2(32.0, 32.0)));
        }
        PaneResponse::default()
    }
}

struct Journey {
    context: Context,
    dock: Dockspace,
    panes: Panes,
    window: Arc<Window>,
    renderer: WgpuRenderer,
    repaint: Repaint,
    requests: mpsc::Receiver<egui::RequestRepaintInfo>,
    started: Instant,
    frame: u32,
    origin: f32,
    tabs: BTreeMap<String, Rect>,
}

impl Journey {
    fn new(window: Arc<Window>, event_loop: &ActiveEventLoop) -> Self {
        let context = Context::default();
        context.enable_accesskit();
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let (sender, requests) = mpsc::channel();
        context.set_request_repaint_callback(move |info| sender.send(info).unwrap());
        let content = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [
                DockspaceNode::tabs([A, B]),
                DockspaceNode::pane(ItemId::new(3)),
            ],
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), content),
        )])
        .unwrap();
        let renderer = pollster::block_on(WgpuRenderer::new_with_backends(
            event_loop.owned_display_handle(),
            Arc::clone(&window),
            Color32::BLACK,
            wgpu::Backends::GL,
        ))
        .unwrap();
        Self {
            context,
            dock: Dockspace::builder("wgpu-surface-recovery", layout)
                .build()
                .unwrap(),
            panes: Panes {
                image: None,
                label: "Original content",
            },
            window,
            renderer,
            repaint: Repaint::default(),
            requests,
            started: Instant::now(),
            frame: 0,
            origin: 0.0,
            tabs: BTreeMap::new(),
        }
    }

    fn texture(&self, name: &str, color: Color32) -> TextureHandle {
        self.context.load_texture(
            name,
            egui::ColorImage::filled([4, 4], color),
            egui::TextureOptions::NEAREST,
        )
    }

    fn step(&mut self, failure: Option<DropReason>, events: Vec<Event>) {
        self.frame += 1;
        let now = self.started + Duration::from_secs(u64::from(self.frame));
        self.repaint.frame_started(now);
        let output = self
            .dock
            .run_single_surface(
                &self.context,
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        Pos2::new(0.0, self.origin),
                        vec2(640.0, 480.0 - self.origin),
                    )),
                    time: Some(f64::from(self.frame)),
                    events,
                    ..Default::default()
                },
                SURFACE,
                &mut self.panes,
            )
            .unwrap()
            .0;
        let (output, receipt) = output.into_parts();
        let FullOutput {
            shapes,
            pixels_per_point,
            platform_output,
            ..
        } = stage_output(output, &mut self.renderer.pending_textures).unwrap();
        self.tabs = platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                if node.role() != egui::accesskit::Role::Tab {
                    return None;
                }
                let bounds = node.bounds()?;
                #[allow(clippy::cast_possible_truncation)]
                let rect = Rect::from_min_max(
                    egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                    egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                );
                Some((node.label()?.to_owned(), rect))
            })
            .collect();
        let primitives = self.context.tessellate(shapes, pixels_per_point);
        let result = self.renderer.paint_with_acquire(
            &self.window,
            pixels_per_point,
            &primitives,
            |surface| match failure {
                Some(DropReason::Lost) => wgpu::CurrentSurfaceTexture::Lost,
                Some(DropReason::Outdated) => wgpu::CurrentSurfaceTexture::Outdated,
                Some(DropReason::TimedOut) => wgpu::CurrentSurfaceTexture::Timeout,
                Some(other) => panic!("unsupported injected fault: {other:?}"),
                None => surface.get_current_texture(),
            },
        );
        assert_eq!(self.renderer.drop_reason, failure);
        assert_eq!(
            result,
            if failure.is_some() {
                SurfacePresentationResult::Dropped
            } else {
                SurfacePresentationResult::Presented
            }
        );
        settle_presentation(&self.renderer, receipt, result, &mut self.repaint, now);
        for info in self.requests.try_iter() {
            let request = RepaintRequest::new(info, now).unwrap();
            if request.is_current(&self.context) {
                self.repaint.request(request.deadline);
            }
        }
        if let Some(reason) = failure {
            assert_eq!(self.repaint.deadline(), Some(now + reason.retry_delay()));
            assert!(!self.repaint.take_due(now));
        }
    }

    fn selected(&self) -> ItemId {
        if self.dock.view().item(A).unwrap().is_active() {
            A
        } else {
            assert!(self.dock.view().item(B).unwrap().is_active());
            B
        }
    }

    fn click(&mut self, position: Pos2) {
        self.step(None, vec![Event::PointerMoved(position)]);
        self.step(None, vec![button(position, true)]);
        self.step(None, vec![button(position, false)]);
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn journey(window: Arc<Window>, event_loop: &ActiveEventLoop) {
    let mut test = Journey::new(window, event_loop);
    let obsolete = test.texture("texture-to-retire", Color32::BLUE);
    let obsolete_id = obsolete.id();
    for _ in 0..4 {
        test.step(None, Vec::new());
    }
    assert!(test.renderer.renderer.texture(&obsolete_id).is_some());
    test.click(test.tabs["Document 2"].center());
    assert_eq!(
        test.selected(),
        B,
        "normal presented input works before the faults"
    );

    let mut retained = test.texture("retained-updates", Color32::RED);
    test.panes.image = Some(retained.id());
    test.panes.label = "New glyphs: Ω λ";
    test.origin = 80.0;
    let version = test.dock.version();
    test.step(Some(DropReason::Lost), Vec::new());
    assert!(test.renderer.surface.is_none());
    assert!(test.renderer.renderer.texture(&retained.id()).is_none());
    assert_eq!(test.renderer.pending_textures.set[&retained.id()].len(), 1);
    let position = test.tabs["Document 1"].center();
    assert!(position.y > test.origin);

    retained.set_partial(
        [1, 2],
        egui::ColorImage::filled([1, 1], Color32::GREEN),
        egui::TextureOptions::LINEAR,
    );
    drop(obsolete);
    test.step(
        Some(DropReason::TimedOut),
        vec![Event::PointerMoved(position), button(position, true)],
    );
    assert!(
        test.renderer.surface.is_some(),
        "Lost must recreate the real surface"
    );
    test.step(Some(DropReason::Outdated), vec![button(position, false)]);
    assert_eq!(
        test.selected(),
        B,
        "unpresented coordinates cannot select another tab"
    );
    assert_eq!(test.dock.version(), version);
    assert!(!test.dock.has_active_gesture());
    assert!(test.renderer.renderer.texture(&retained.id()).is_none());
    assert!(test.renderer.renderer.texture(&obsolete_id).is_some());
    let pending = &test.renderer.pending_textures;
    assert_eq!(pending.set[&retained.id()].len(), 2);
    assert_eq!(pending.set[&retained.id()][0].pos, None);
    assert_eq!(pending.set[&retained.id()][1].pos, Some([1, 2]));
    assert!(pending.free.contains(&obsolete_id));
    assert!(
        pending.set.contains_key(&egui::TextureId::Managed(0)),
        "new glyphs retain their real font-atlas delta"
    );

    test.step(None, Vec::new());
    assert!(test.renderer.pending_textures.is_empty());
    let uploaded = test.renderer.renderer.texture(&retained.id()).unwrap();
    assert_eq!(
        uploaded.options,
        Some(egui::TextureOptions::LINEAR),
        "the partial update must reach the real renderer after the full upload"
    );
    let texture = uploaded.texture.as_ref().unwrap();
    assert_eq!(
        (texture.width(), texture.height()),
        (4, 4),
        "a partial update must retain the full texture's dimensions"
    );
    assert!(test.renderer.renderer.texture(&obsolete_id).is_none());
    test.step(None, Vec::new());
    test.click(test.tabs["Document 1"].center());
    assert_eq!(
        test.selected(),
        A,
        "freshly presented input works after recovery"
    );
    test.step(None, vec![Event::PointerGone]);
    for _ in 0..8 {
        test.step(None, Vec::new());
    }
    assert_eq!(
        test.repaint.deadline(),
        None,
        "settled recovery must not leave a polling loop"
    );
    test.panes.image = None;
    drop(retained);
    test.step(None, Vec::new());
    assert!(test.renderer.pending_textures.is_empty());
}

#[test]
#[ignore = "requires X11, a window manager and a software-capable WGPU backend"]
fn real_wgpu_surface_recovery_retains_textures_and_restores_presented_input() {
    #[derive(Default)]
    struct App {
        window: Option<Arc<Window>>,
        completed: bool,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Dockspace surface recovery")
                            .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
                    )
                    .unwrap(),
            );
            window.request_redraw();
            self.window = Some(window);
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            if matches!(event, WindowEvent::RedrawRequested) && !self.completed {
                journey(Arc::clone(self.window.as_ref().unwrap()), event_loop);
                self.completed = true;
                event_loop.exit();
            }
        }
    }

    let event_loop = EventLoop::builder()
        .with_x11()
        .with_any_thread(true)
        .build()
        .unwrap();
    let mut app = App::default();
    event_loop.run_app(&mut app).unwrap();
    assert!(app.completed);
}
