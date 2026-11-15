//! Browser startup and asynchronous GPU initialization for a single canvas.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use egui::Context;
use web_sys::HtmlCanvasElement;
use web_time::Instant;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys},
    window::{Window, WindowAttributes, WindowId},
};

use super::{
    FrameControl, NativeApp, NativeError, WinitIntegration,
    frame::HostWindow,
    renderer::WgpuRenderer,
    repaint::{Repaint, RepaintRequest},
};

/// Configuration for one application-owned browser canvas.
///
/// Insert the canvas into the document before starting the host. Its CSS size
/// determines the available space; winit maintains its physical-pixel backing
/// size. Put borders and padding on a containing element, rather than the canvas.
#[derive(Clone)]
pub struct WebOptions {
    /// Canvas used for input and WGPU presentation.
    pub canvas: HtmlCanvasElement,
    /// Background under the complete egui output.
    pub clear_color: egui::Color32,
}

impl WebOptions {
    /// Uses an existing canvas with the default dark clear color.
    #[must_use]
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        Self {
            canvas,
            clear_color: egui::Color32::from_rgb(24, 26, 30),
        }
    }
}

/// Starts a complete-output application in one browser canvas.
///
/// The host takes ownership of `app` and returns after installing browser event
/// dispatch. GPU initialization is asynchronous: window input accumulates until
/// the renderer is ready, and no docking output is produced before that point.
///
/// The same frame pipeline as the desktop host reports Presented only after
/// submitting and presenting that exact output. Panes may float inside the
/// canvas; additional viewports and browser windows are unsupported.
///
/// `on_error` receives asynchronous initialization or frame failures before the
/// event loop exits. Configure fonts and theme on `context` before starting.
///
/// # Errors
///
/// Returns an error if the browser event loop cannot be created. Later failures
/// are delivered once through `on_error`.
pub fn run_web(
    options: WebOptions,
    context: Context,
    app: impl NativeApp + 'static,
    on_error: impl FnMut(NativeError) + 'static,
) -> Result<(), NativeError> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let repaint_proxy = proxy.clone();
    context.set_request_repaint_callback(move |request| {
        if let Some(request) = RepaintRequest::new(request, Instant::now()) {
            let _ = repaint_proxy.send_event(UserEvent::Repaint(request));
        }
    });
    event_loop.spawn_app(Runner {
        proxy,
        context,
        options,
        app,
        on_error,
        window: None,
        repaint: Repaint::default(),
    });
    Ok(())
}

enum UserEvent {
    Repaint(RepaintRequest),
    GpuReady,
}

struct InitializingWindow {
    integration: WinitIntegration,
    window: Arc<Window>,
    result: Rc<RefCell<Option<Result<WgpuRenderer, egui_wgpu::WgpuError>>>>,
}

enum WebWindow {
    Initializing(InitializingWindow),
    Ready(HostWindow),
}

impl WebWindow {
    fn input_parts(&mut self) -> (&Arc<Window>, &mut WinitIntegration) {
        match self {
            Self::Initializing(state) => (&state.window, &mut state.integration),
            Self::Ready(state) => (&state.window, &mut state.integration),
        }
    }
}

struct Runner<A, E> {
    proxy: EventLoopProxy<UserEvent>,
    context: Context,
    options: WebOptions,
    app: A,
    on_error: E,
    window: Option<WebWindow>,
    repaint: Repaint,
}

impl<A: NativeApp, E: FnMut(NativeError)> Runner<A, E> {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), NativeError> {
        if self.window.is_some() {
            self.repaint.resume();
            self.repaint.request(Instant::now());
            return Ok(());
        }
        let window = Arc::new(
            event_loop.create_window(
                WindowAttributes::default()
                    .with_title("Dockspace")
                    .with_canvas(Some(self.options.canvas.clone())),
            )?,
        );
        let integration = WinitIntegration::new(self.context.clone(), &window, event_loop);
        let result = Rc::new(RefCell::new(None));
        // Closing the host retires the slot; a late GPU result then drops normally.
        let completion = Rc::downgrade(&result);
        self.window = Some(WebWindow::Initializing(InitializingWindow {
            integration,
            window: Arc::clone(&window),
            result,
        }));
        let display = event_loop.owned_display_handle();
        let clear_color = self.options.clear_color;
        let proxy = self.proxy.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let renderer = WgpuRenderer::new(display, window, clear_color).await;
            if let Some(completion) = completion.upgrade() {
                completion.replace(Some(renderer));
                let _ = proxy.send_event(UserEvent::GpuReady);
            }
        });
        Ok(())
    }

    fn finish_initialization(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.window.take() else {
            return;
        };
        let WebWindow::Initializing(mut state) = state else {
            self.window = Some(state);
            return;
        };
        let result = state.result.borrow_mut().take();
        match result {
            Some(Ok(renderer)) => {
                state.integration.set_max_texture_side(
                    renderer.device.limits().max_texture_dimension_2d as usize,
                );
                self.window = Some(WebWindow::Ready(HostWindow {
                    renderer,
                    integration: state.integration,
                    window: state.window,
                }));
                self.repaint.resume();
                self.repaint.request(Instant::now());
            }
            Some(Err(error)) => self.fail(event_loop, NativeError::Renderer(error)),
            None => self.window = Some(WebWindow::Initializing(state)),
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: NativeError) {
        (self.on_error)(error);
        event_loop.exit();
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(WebWindow::Ready(state)) = &mut self.window else {
            return;
        };
        match state.render(event_loop, &self.context, &mut self.app, &mut self.repaint) {
            Ok(FrameControl::Continue) => {}
            Ok(FrameControl::Repaint) => self.repaint.request(Instant::now()),
            Ok(FrameControl::Exit) => event_loop.exit(),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn schedule(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }
        let Some(WebWindow::Ready(state)) = &self.window else {
            // Pending input and repaint requests cannot spin while GPU init awaits JS.
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        if self.repaint.take_due(Instant::now()) {
            state.window.request_redraw();
        }
        event_loop.set_control_flow(
            self.repaint
                .deadline()
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }
}

impl<A: NativeApp, E: FnMut(NativeError)> ApplicationHandler<UserEvent> for Runner<A, E> {
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
        let (window, integration) = state.input_parts();
        if window.id() != id {
            return;
        }
        match event {
            WindowEvent::Destroyed => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {
                let response = integration.on_window_event(window, &event);
                if response.repaint
                    || matches!(
                        event,
                        WindowEvent::Resized(_) | WindowEvent::Occluded(false)
                    )
                {
                    self.repaint.resume();
                    self.repaint.request(Instant::now());
                }
            }
        }
        self.schedule(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::GpuReady => self.finish_initialization(event_loop),
            UserEvent::Repaint(request) => {
                if request.is_current(&self.context) {
                    self.repaint.request(request.deadline);
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
