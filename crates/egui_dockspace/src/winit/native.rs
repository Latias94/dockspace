//! Complete-output single-window hosting with official winit and WGPU.

use std::{sync::Arc, time::Duration};

use web_time::Instant;

use egui::Context;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};

use super::{
    FrameControl, NativeApp, NativeError, WinitIntegration,
    frame::HostWindow,
    renderer::WgpuRenderer,
    repaint::{Repaint, RepaintRequest},
};

const HIDDEN_REPAINT_INTERVAL: Duration = Duration::from_millis(250);

fn needs_direct_update(window: &Window) -> bool {
    window.is_visible() == Some(false)
        // Runtime minimized-state queries can deadlock on macOS. Windows needs
        // this fallback because hidden/minimized windows may never receive RedrawRequested.
        || (cfg!(target_os = "windows") && window.is_minimized() == Some(true))
}

/// Configuration for one application-owned desktop window.
pub struct NativeOptions {
    /// Native title, initial dimensions, decorations, and other winit attributes.
    pub window: WindowAttributes,
    /// Background under the complete egui output.
    pub clear_color: egui::Color32,
}

impl Default for NativeOptions {
    fn default() -> Self {
        Self {
            window: WindowAttributes::default()
                .with_title("Dockspace")
                .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 760.0)),
            clear_color: egui::Color32::from_rgb(24, 26, 30),
        }
    }
}

/// Runs a desktop application in one OS window using official winit and WGPU.
///
/// Configure fonts and theme on `context` before calling. Panes may float inside
/// the window; separate OS viewports require a different host. Close requests
/// reach the root egui viewport and may be rejected with `ViewportCommand::CancelClose`.
/// The renderer reports Presented only after submitting and presenting the exact
/// complete output. Failed acquisitions report Dropped and retain texture updates.
/// Surface recovery is paced, and hidden-window updates run only when requested.
/// Those updates still process close and visibility commands on platforms which
/// withhold redraw events for hidden windows.
///
/// Applications selected at runtime can be passed as borrowed trait objects:
///
/// ```no_run
/// use egui_dockspace::winit::{NativeApp, NativeError, NativeOptions, run_native};
///
/// fn launch(app: &mut dyn NativeApp) -> Result<(), NativeError> {
///     let context = egui::Context::default();
///     run_native(NativeOptions::default(), context, app)
/// }
/// ```
///
/// # Errors
///
/// Returns startup failures, application callback failures, unsupported output,
/// or an event-loop failure. Call this on the platform's event-loop thread.
pub fn run_native(
    options: NativeOptions,
    context: Context,
    app: &mut (impl NativeApp + ?Sized),
) -> Result<(), NativeError> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let mut runner = Runner {
        proxy: event_loop.create_proxy(),
        context,
        options,
        app,
        window: None,
        repaint: Repaint::default(),
        failure: None,
    };
    event_loop.run_app(&mut runner)?;
    runner.failure.map_or(Ok(()), Err)
}

enum UserEvent {
    Repaint(RepaintRequest),
    #[cfg(feature = "accesskit")]
    AccessKit(egui_winit::accesskit_winit::Event),
}

#[cfg(feature = "accesskit")]
impl From<egui_winit::accesskit_winit::Event> for UserEvent {
    fn from(event: egui_winit::accesskit_winit::Event) -> Self {
        Self::AccessKit(event)
    }
}

struct Runner<'app, A: ?Sized> {
    proxy: EventLoopProxy<UserEvent>,
    context: Context,
    options: NativeOptions,
    app: &'app mut A,
    window: Option<HostWindow>,
    repaint: Repaint,
    failure: Option<NativeError>,
}

impl<A: NativeApp + ?Sized> Runner<'_, A> {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), NativeError> {
        if self.window.is_some() {
            self.repaint.resume();
            self.repaint.request(Instant::now());
            return Ok(());
        }
        // Accessibility must attach before the window has ever been shown.
        let window =
            Arc::new(event_loop.create_window(self.options.window.clone().with_visible(false))?);
        let renderer = pollster::block_on(WgpuRenderer::new(
            event_loop.owned_display_handle(),
            Arc::clone(&window),
            self.options.clear_color,
        ))?;
        let mut integration = WinitIntegration::new(self.context.clone(), &window, event_loop);
        #[cfg(feature = "accesskit")]
        integration.init_accesskit(event_loop, &window, self.proxy.clone());
        integration
            .set_max_texture_side(renderer.device.limits().max_texture_dimension_2d as usize);
        let proxy = self.proxy.clone();
        self.context.set_request_repaint_callback(move |request| {
            // Capture the deadline when requested, so queueing cannot restart its delay.
            if let Some(request) = RepaintRequest::new(request, Instant::now()) {
                let _ = proxy.send_event(UserEvent::Repaint(request));
            }
        });
        window.set_visible(self.options.window.visible);
        if self.options.window.visible && self.options.window.active {
            window.focus_window();
        }
        self.window = Some(HostWindow {
            renderer,
            integration,
            window,
        });
        self.repaint.request(Instant::now());
        Ok(())
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) -> Result<FrameControl, NativeError> {
        let Some(state) = &mut self.window else {
            return Ok(FrameControl::Continue);
        };
        state.render(event_loop, &self.context, self.app, &mut self.repaint)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: NativeError) {
        self.failure = Some(error);
        event_loop.exit();
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        match self.render(event_loop) {
            Ok(FrameControl::Exit) => event_loop.exit(),
            Ok(FrameControl::Repaint) => self.repaint.request(Instant::now()),
            Ok(FrameControl::Continue) => {}
            Err(error) => self.fail(event_loop, error),
        }
        if self
            .window
            .as_ref()
            .is_some_and(|state| needs_direct_update(&state.window))
        {
            // An application can still close or show a hidden window. Throttle
            // only requested work; an idle window needs no background polling.
            self.repaint
                .defer_until(Instant::now() + HIDDEN_REPAINT_INTERVAL);
        }
    }

    fn schedule(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }
        if self.repaint.take_due(Instant::now()) {
            if let Some(state) = &self.window {
                if needs_direct_update(&state.window) {
                    self.redraw(event_loop);
                } else {
                    state.window.request_redraw();
                }
            }
        }
        event_loop.set_control_flow(
            self.repaint
                .deadline()
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }
}

impl<A: NativeApp + ?Sized> ApplicationHandler<UserEvent> for Runner<'_, A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.open(event_loop) {
            self.fail(event_loop, error);
        }
        self.schedule(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.window else {
            return;
        };
        if state.window.id() != id {
            return;
        }
        if matches!(event, WindowEvent::Destroyed) {
            event_loop.exit();
            return;
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw(event_loop);
            self.schedule(event_loop);
            return;
        }
        let response = state.integration.on_window_event(&state.window, &event);
        if response.repaint
            || matches!(
                event,
                WindowEvent::Resized(_) | WindowEvent::Occluded(false)
            )
        {
            self.repaint.resume();
            self.repaint.request(Instant::now());
        }
        self.schedule(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Repaint(request) => {
                if request.is_current(&self.context) {
                    self.repaint.request(request.deadline);
                }
            }
            #[cfg(feature = "accesskit")]
            UserEvent::AccessKit(event) => {
                if let Some(state) = &mut self.window {
                    if state
                        .integration
                        .on_accesskit_event(&state.window, &event)
                        .repaint
                    {
                        self.repaint.resume();
                        self.repaint.request(Instant::now());
                    }
                }
            }
        }
        self.schedule(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.schedule(event_loop);
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.window = None;
    }
}
