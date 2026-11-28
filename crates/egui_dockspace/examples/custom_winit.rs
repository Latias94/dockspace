//! Custom `winit` event loop and `wgpu` renderer integrated via `WinitIntegration`.
//!
//! Use this pattern when your application or engine already owns its `winit`
//! `ApplicationHandler` and GPU renderer instead of delegating the loop to
//! `egui_dockspace::winit::run_native`.
//!
//! Demonstrates:
//! - Direct `WinitIntegration` wiring (`new`, `set_max_texture_side`,
//!   `on_window_event`, `take_input`, `handle_output`, optional `init_accesskit`).
//! - Complete-output settlement via `Dockspace::run_single_surface_with_ui` and
//!   `DockspaceOutput::into_parts()`.
//! - Output backpressure handling with `DockspaceError::into_deferred_input()`.
//! - Root-window close confirmation with `ViewportCommand::CancelClose`.
//! - `wgpu 30` surface setup with uncaptured-error and device-lost callbacks.
//!
//! Run from the repository root:
//! `cargo run -p egui_dockspace --example custom_winit --features wgpu --locked`

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use dockspace_winit::RepaintScheduler;
use egui::{Context, RawInput, ViewportCommand, ViewportId};
use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, Dockspace, DockspaceAxis, DockspaceError,
    DockspaceErrorKind, DockspaceLayout, DockspaceNode, DockspaceOutput, DockspaceOutputReceipt,
    DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
    SurfacePresentationResult, winit::WinitIntegration,
};
use egui_wgpu::wgpu;
use web_time::Instant;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy, OwnedDisplayHandle},
    window::{Window, WindowAttributes, WindowId},
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const CODE: ItemId = ItemId::new(1);
const NOTES: ItemId = ItemId::new(2);
const PREVIEW: ItemId = ItemId::new(3);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<HostEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let context = Context::default();
    let repaint_proxy = proxy.clone();
    context.set_request_repaint_callback(move |info| {
        if let Some(deadline) = Instant::now().checked_add(info.delay) {
            let _ = repaint_proxy.send_event(HostEvent::Repaint(deadline));
        }
    });
    let mut host = CustomWinitHost::new(context, proxy);
    event_loop.run_app(&mut host)?;
    if let Some(error) = host.fatal_error {
        return Err(error.into());
    }
    Ok(())
}

enum HostEvent {
    Repaint(Instant),
    DeviceLost(String),
    #[cfg(feature = "accesskit")]
    AccessKit(egui_dockspace::winit::AccessKitEvent),
}

#[cfg(feature = "accesskit")]
impl From<egui_dockspace::winit::AccessKitEvent> for HostEvent {
    fn from(event: egui_dockspace::winit::AccessKitEvent) -> Self {
        Self::AccessKit(event)
    }
}

/// Application state owned completely independently of `winit` and `wgpu`.
struct EditorState {
    dockspace: Dockspace,
    panes: EditorPanes,
    unsaved_changes: bool,
    confirm_exit: bool,
    status: String,
}

impl EditorState {
    fn new() -> Self {
        let split = DockspaceNode::split(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::central_tabs([CODE, NOTES]), 0.68),
                (DockspaceNode::tabs([PREVIEW]), 0.32),
            ],
        )
        .expect("static split is valid");
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, split),
        )])
        .expect("static layout is valid");
        Self {
            dockspace: Dockspace::builder("custom-winit", layout)
                .build()
                .expect("static dockspace builds"),
            panes: EditorPanes::new(),
            unsaved_changes: false,
            confirm_exit: false,
            status: "Caller-owned winit + WGPU loop. Drag tabs or try closing with unsaved edits."
                .to_owned(),
        }
    }

    fn run_frame(
        &mut self,
        context: &Context,
        input: RawInput,
    ) -> Result<DockspaceOutput, DockspaceError> {
        let close_requested = input.viewport().close_requested();
        let mut save_now = false;
        let mut discard_and_exit = false;
        let mut keep_editing = false;

        let (output, response) = self.dockspace.run_single_surface_with_ui(
            context,
            input,
            SURFACE,
            &mut self.panes,
            |ui| {
                if close_requested && self.unsaved_changes {
                    self.confirm_exit = true;
                    ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                }
                egui::Panel::top("custom-winit-toolbar").show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong("Custom winit + WGPU host");
                        ui.separator();
                        if ui
                            .add_enabled(self.unsaved_changes, egui::Button::new("Mark saved"))
                            .clicked()
                        {
                            save_now = true;
                        }
                        if self.confirm_exit {
                            ui.separator();
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                "Unsaved edits — exit anyway?",
                            );
                            if ui.button("Discard & exit").clicked() {
                                discard_and_exit = true;
                            }
                            if ui.button("Keep editing").clicked() {
                                keep_editing = true;
                            }
                        }
                    });
                    ui.small(&self.status);
                });
            },
        )?;

        if self.panes.edited_this_frame {
            self.panes.edited_this_frame = false;
            self.unsaved_changes = true;
            "Document modified (unsaved)".clone_into(&mut self.status);
        }
        if save_now {
            self.unsaved_changes = false;
            self.confirm_exit = false;
            "Marked saved — closing the window now exits immediately.".clone_into(&mut self.status);
        }
        if keep_editing {
            self.confirm_exit = false;
            "Close cancelled.".clone_into(&mut self.status);
        }
        if discard_and_exit {
            self.unsaved_changes = false;
            self.confirm_exit = false;
            context.send_viewport_cmd(ViewportCommand::Close);
        }

        // Approve tab close requests immediately while keeping document buffers in `self.panes`.
        for request in response
            .close_request_events()
            .iter()
            .filter(|request| !request.reused())
        {
            let plan = request.plan();
            for item in plan
                .items()
                .iter()
                .filter(|item| item.state() == CloseItemDecisionState::Pending)
            {
                let _ = self.dockspace.resolve_close(
                    plan.request(),
                    item.token(),
                    CloseDecision::Allow,
                )?;
            }
        }
        Ok(output)
    }
}

struct EditorPanes {
    code: String,
    notes: String,
    edited_this_frame: bool,
}

impl EditorPanes {
    fn new() -> Self {
        Self {
            code: "// Owned by the caller's winit ApplicationHandler.\nfn render_scene() {\n    // Draw your own 3D/2D GPU pass before the egui overlay pass.\n}\n"
                .to_owned(),
            notes: "Try editing this text, then clicking the OS window close button.\n\nWinitIntegration routes CloseRequested through egui so your app can veto it with ViewportCommand::CancelClose."
                .to_owned(),
            edited_this_frame: false,
        }
    }
}

impl PaneView for EditorPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        match item {
            CODE => Some("scene.rs".into()),
            NOTES => Some("Notes".into()),
            PREVIEW => Some("Viewport".into()),
            _ => None,
        }
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        match item {
            CODE => {
                let id = focus_id(item);
                let scroll = egui::ScrollArea::both().id_salt(id).show(ui, |ui| {
                    egui_dockspace::pane_text_edit(
                        ui,
                        id,
                        egui::TextEdit::multiline(&mut self.code)
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    )
                });
                self.edited_this_frame |= scroll.inner.changed();
                PaneResponse::default()
                    .with_scroll_area(ui, &scroll)
                    .with_content(&scroll.inner)
            }
            NOTES => {
                let id = focus_id(item);
                let scroll = egui::ScrollArea::vertical().id_salt(id).show(ui, |ui| {
                    egui_dockspace::pane_text_edit(
                        ui,
                        id,
                        egui::TextEdit::multiline(&mut self.notes).desired_width(f32::INFINITY),
                    )
                });
                self.edited_this_frame |= scroll.inner.changed();
                PaneResponse::default()
                    .with_scroll_area(ui, &scroll)
                    .with_content(&scroll.inner)
            }
            PREVIEW => {
                let (response, painter) =
                    ui.allocate_painter(ui.available_size(), egui::Sense::click());
                let rect = response.rect;
                painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
                painter.circle_stroke(
                    rect.center(),
                    rect.height().min(rect.width()) * 0.25,
                    egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                );
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Custom WGPU Viewport",
                    egui::FontId::proportional(15.0),
                    ui.visuals().text_color(),
                );
                PaneResponse::from(&response)
            }
            _ => PaneResponse::default(),
        }
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        matches!(item, CODE | NOTES).then(|| focus_id(item))
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        self.focus_target(item).is_some()
    }
}

fn focus_id(item: ItemId) -> egui::Id {
    egui::Id::unique(("custom-winit-pane", item))
}

/// Minimal caller-owned `wgpu 30` renderer in reverse-dependency field drop order.
struct CustomWgpuRenderer {
    config: wgpu::SurfaceConfiguration,
    needs_reconfigure: bool,
    device_lost: Arc<AtomicBool>,
    pending_textures: egui::TexturesDelta,
    egui_renderer: egui_wgpu::Renderer,
    surface: wgpu::Surface<'static>,
    queue: wgpu::Queue,
    device: wgpu::Device,
    _adapter: wgpu::Adapter,
    _instance: wgpu::Instance,
}

impl CustomWgpuRenderer {
    fn new(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        proxy: EventLoopProxy<HostEvent>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY | wgpu::Backends::GL);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: Some(Box::new(display)),
        });
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = pollster::block_on(request_adapter(&instance, &surface))?;
        let adapter_limits = adapter.limits();
        let required_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        }
        .using_resolution(adapter_limits.clone())
        .using_alignment(adapter_limits);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("custom-winit device"),
                required_features: wgpu::Features::empty(),
                required_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            }))?;

        let device_lost = Arc::new(AtomicBool::new(false));
        device.on_uncaptured_error(Arc::new(|error| {
            eprintln!("custom_winit WGPU error: {error}");
        }));
        {
            let device_lost = Arc::clone(&device_lost);
            device.set_device_lost_callback(move |reason, message| {
                if reason != wgpu::DeviceLostReason::Destroyed {
                    device_lost.store(true, Ordering::Release);
                    let _ = proxy.send_event(HostEvent::DeviceLost(message));
                }
            });
        }

        let capabilities = surface.get_capabilities(&adapter);
        let format = egui_wgpu::preferred_framebuffer_format(&capabilities.formats)?;
        let size = window.inner_size();
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        let width = size.width.max(1).min(max_dim);
        let height = size.height.max(1).min(max_dim);
        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            wgpu::CompositeAlphaMode::Auto
        };
        let default_config = surface
            .get_default_config(&adapter, width, height)
            .ok_or("surface has no supported formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![format],
            ..default_config
        };
        surface.configure(&device, &config);
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        Ok(Self {
            config,
            needs_reconfigure: false,
            device_lost,
            pending_textures: egui::TexturesDelta::default(),
            egui_renderer,
            surface,
            queue,
            device,
            _adapter: adapter,
            _instance: instance,
        })
    }

    fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }

    fn paint(
        &mut self,
        window: &Window,
        pixels_per_point: f32,
        primitives: &[egui::ClippedPrimitive],
    ) -> SurfacePresentationResult {
        if self.device_lost.load(Ordering::Acquire) {
            return SurfacePresentationResult::Dropped;
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return SurfacePresentationResult::Dropped;
        }
        let max_dim = self.device.limits().max_texture_dimension_2d.max(1);
        let width = size.width.min(max_dim);
        let height = size.height.min(max_dim);
        if self.needs_reconfigure || self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.needs_reconfigure = false;
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.needs_reconfigure = true;
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.needs_reconfigure = true;
                return SurfacePresentationResult::Dropped;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                return SurfacePresentationResult::Dropped;
            }
        };

        for (id, image_deltas) in self.pending_textures.set.drain() {
            for image_delta in image_deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, id, &image_delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("custom-winit encoder"),
            });
        let user_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            primitives,
            &screen,
        );
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("custom-winit egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.09,
                            g: 0.10,
                            b: 0.12,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.egui_renderer
                .render(&mut render_pass.forget_lifetime(), primitives, &screen);
        }
        self.queue
            .submit(user_commands.into_iter().chain([encoder.finish()]));
        for id in self.pending_textures.free.drain() {
            self.egui_renderer.free_texture(&id);
        }
        window.pre_present_notify();
        self.queue.present(frame);
        let _ = self.device.poll(wgpu::PollType::Poll);
        SurfacePresentationResult::Presented
    }
}

async fn request_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, Box<dyn std::error::Error>> {
    if let Ok(adapter) = wgpu::util::initialize_adapter_from_env(instance, Some(surface)).await {
        return Ok(adapter);
    }
    let power_preference =
        wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance);
    match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: Some(surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
    {
        Ok(adapter) => Ok(adapter),
        Err(_) => Ok(instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                compatible_surface: Some(surface),
                force_fallback_adapter: true,
                apply_limit_buckets: false,
            })
            .await?),
    }
}

struct WindowHost {
    renderer: CustomWgpuRenderer,
    integration: WinitIntegration,
    window: Arc<Window>,
}

struct CustomWinitHost {
    context: Context,
    proxy: EventLoopProxy<HostEvent>,
    editor: EditorState,
    repaint: RepaintScheduler,
    pending_input: Option<RawInput>,
    window: Option<WindowHost>,
    fatal_error: Option<String>,
}

impl CustomWinitHost {
    fn new(context: Context, proxy: EventLoopProxy<HostEvent>) -> Self {
        Self {
            context,
            proxy,
            editor: EditorState::new(),
            repaint: RepaintScheduler::default(),
            pending_input: None,
            window: None,
            fatal_error: None,
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(host) = &mut self.window else {
            return;
        };
        let now = Instant::now();
        self.repaint.frame_started(now);

        // Retry deferred input in exact order before taking fresh window input.
        let input = self
            .pending_input
            .take()
            .unwrap_or_else(|| host.integration.take_input(&host.window));

        let output = match self.editor.run_frame(&self.context, input) {
            Ok(output) => output,
            Err(error) if error.kind() == DockspaceErrorKind::OutputBackpressure => {
                self.pending_input = error.into_deferred_input().ok();
                self.repaint.request(now + Duration::from_millis(16));
                return;
            }
            Err(error) => {
                self.fatal_error = Some(error.to_string());
                event_loop.exit();
                return;
            }
        };

        let (mut full_output, receipt): (egui::FullOutput, DockspaceOutputReceipt) =
            output.into_parts();
        host.renderer
            .pending_textures
            .append(std::mem::take(&mut full_output.textures_delta));
        let root_commands = full_output
            .viewport_output
            .remove(&ViewportId::ROOT)
            .map(|output| output.commands)
            .into_iter()
            .flatten();
        let response = host.integration.handle_output(
            &host.window,
            event_loop,
            full_output.platform_output,
            root_commands,
        );
        let primitives = self
            .context
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let presentation =
            host.renderer
                .paint(&host.window, full_output.pixels_per_point, &primitives);
        // Settle the affine receipt only after the renderer reports the exact outcome.
        receipt.report(presentation);
        if presentation == SurfacePresentationResult::Dropped || host.renderer.needs_reconfigure {
            self.repaint
                .request(Instant::now() + Duration::from_millis(16));
        }
        if response.close_requested {
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<HostEvent> for CustomWinitHost {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title("Dockspace — Custom WinitIntegration + WGPU")
            .with_inner_size(LogicalSize::new(1120.0, 740.0))
            .with_visible(!cfg!(feature = "accesskit"));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fatal_error = Some(error.to_string());
                event_loop.exit();
                return;
            }
        };
        let mut integration = WinitIntegration::new(self.context.clone(), &window, event_loop);
        #[cfg(feature = "accesskit")]
        {
            integration.init_accesskit(event_loop, &window, self.proxy.clone());
            window.set_visible(true);
        }
        let renderer = match CustomWgpuRenderer::new(
            event_loop.owned_display_handle(),
            Arc::clone(&window),
            self.proxy.clone(),
        ) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fatal_error = Some(error.to_string());
                event_loop.exit();
                return;
            }
        };
        integration.set_max_texture_side(renderer.max_texture_side());
        window.request_redraw();
        self.window = Some(WindowHost {
            renderer,
            integration,
            window,
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        match event {
            HostEvent::Repaint(deadline) => self.repaint.request(deadline),
            HostEvent::DeviceLost(reason) => {
                self.fatal_error = Some(format!("graphics device lost: {reason}"));
                event_loop.exit();
            }
            #[cfg(feature = "accesskit")]
            HostEvent::AccessKit(event) => {
                if let Some(host) = &mut self.window {
                    let response = host.integration.on_accesskit_event(&host.window, &event);
                    if response.repaint {
                        self.repaint.request(Instant::now());
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(host) = &mut self.window else {
            return;
        };
        if host.window.id() != id {
            return;
        }
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            host.renderer.needs_reconfigure = true;
        }
        if event == WindowEvent::RedrawRequested {
            self.redraw(event_loop);
            return;
        }
        let response = host.integration.on_window_event(&host.window, &event);
        if response.repaint
            || matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
            )
        {
            self.repaint.request(Instant::now());
        }
        if event == WindowEvent::Destroyed {
            self.window = None;
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.repaint.take_due(now)
            && let Some(host) = &self.window
        {
            host.window.request_redraw();
        }
        event_loop.set_control_flow(
            self.repaint
                .deadline()
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_input(close_requested: bool, time: f64) -> RawInput {
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1120.0, 740.0),
            )),
            time: Some(time),
            ..Default::default()
        };
        if close_requested {
            raw.viewports
                .get_mut(&ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
        }
        raw
    }

    #[test]
    fn backpressure_defers_input_until_receipts_settle() {
        let context = Context::default();
        let mut editor = EditorState::new();
        let mut receipts = Vec::new();
        let mut deferred = None;

        for step in 0..300 {
            let input = raw_input(false, f64::from(step) * 0.016);
            match editor.run_frame(&context, input) {
                Ok(output) => {
                    let (mut full_output, receipt) = output.into_parts();
                    full_output.textures_delta.clear();
                    receipts.push(receipt);
                }
                Err(error) if error.kind() == DockspaceErrorKind::OutputBackpressure => {
                    deferred = Some(
                        error
                            .into_deferred_input()
                            .expect("backpressure carries input"),
                    );
                    break;
                }
                Err(error) => panic!("unexpected error: {error}"),
            }
        }
        let deferred = deferred.expect("holding receipts eventually triggers backpressure");
        for receipt in receipts {
            receipt.report(SurfacePresentationResult::Presented);
        }

        let (mut full_output, receipt) = editor
            .run_frame(&context, deferred)
            .expect("replaying deferred input succeeds after receipts settle")
            .into_parts();
        full_output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);
    }

    #[test]
    fn unsaved_changes_veto_window_close_until_marked_saved() {
        let context = Context::default();
        let mut editor = EditorState::new();
        editor.unsaved_changes = true;

        let (mut full_output, receipt) = editor
            .run_frame(&context, raw_input(true, 0.1))
            .expect("dirty close frame succeeds")
            .into_parts();
        let commands = &full_output.viewport_output[&ViewportId::ROOT].commands;
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(editor.confirm_exit);
        full_output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);

        editor.unsaved_changes = false;
        editor.confirm_exit = false;
        let (mut full_output, receipt) = editor
            .run_frame(&context, raw_input(true, 0.2))
            .expect("clean close frame succeeds")
            .into_parts();
        let commands = &full_output.viewport_output[&ViewportId::ROOT].commands;
        assert!(!commands.contains(&ViewportCommand::CancelClose));
        full_output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);
    }
}
