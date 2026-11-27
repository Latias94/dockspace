//! Browser startup and asynchronous GPU initialization for a single canvas.

use std::{
    cell::{Cell, RefCell},
    pin::pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use egui::Context;
use egui_wgpu::wgpu;
use futures_util::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, HtmlCanvasElement};
use web_time::Instant;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy, OwnedDisplayHandle},
    platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys},
    window::{Window, WindowAttributes, WindowId},
};

use super::{
    FrameControl, NativeApp, NativeError, WinitIntegration,
    frame::HostWindow,
    renderer::{DropReason, WgpuRenderer},
    repaint::{Repaint, RepaintRequest},
};

const WEBGPU_INIT_TIMEOUT_SECONDS: u32 = 5;
const WEBGL_INIT_TIMEOUT_SECONDS: u32 = 10;
const MAX_INITIAL_PRESENT_FAILURES: u8 = 3;

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
    /// Graphics backend selection and fallback policy.
    pub renderer: WebRenderer,
}

/// Browser graphics selection for a complete-output host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WebRenderer {
    /// Try WebGPU, then WebGL2 if initialization fails, takes over five seconds,
    /// or cannot present the first frame.
    #[default]
    Auto,
    /// Use WebGL2 directly, including when the browser advertises WebGPU.
    WebGl,
}

impl WebOptions {
    /// Uses an existing canvas with the default dark clear color.
    #[must_use]
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        Self {
            canvas,
            clear_color: egui::Color32::from_rgb(24, 26, 30),
            renderer: WebRenderer::Auto,
        }
    }
}

enum InitOutcome {
    Ready(WgpuRenderer),
    RetryWebGlOnFreshCanvas(NativeError),
    Failed(NativeError),
}

fn is_secure_browser_context() -> bool {
    web_sys::window().is_some_and(|window| window.is_secure_context())
}

async fn initialize_renderer(
    display: OwnedDisplayHandle,
    window: Arc<Window>,
    clear_color: egui::Color32,
    renderer: WebRenderer,
    generation: u64,
    proxy: EventLoopProxy<UserEvent>,
) -> InitOutcome {
    if renderer == WebRenderer::Auto {
        if is_secure_browser_context() {
            let canvas_claimed = Cell::new(false);
            match initialize_backend(
                display.clone(),
                Arc::clone(&window),
                clear_color,
                wgpu::Backends::BROWSER_WEBGPU,
                "WebGPU",
                WEBGPU_INIT_TIMEOUT_SECONDS,
                &canvas_claimed,
                generation,
                proxy.clone(),
            )
            .await
            {
                Ok(renderer) => return InitOutcome::Ready(renderer),
                Err(error) => {
                    if canvas_claimed.get() {
                        web_sys::console::warn_1(
                            &format!("{error}. Replacing canvas to fall back to WebGL2 cleanly.")
                                .into(),
                        );
                        return InitOutcome::RetryWebGlOnFreshCanvas(error);
                    }
                    web_sys::console::warn_1(&format!("{error}. Trying WebGL2 instead.").into());
                }
            }
        } else {
            web_sys::console::info_1(
                &"WebGPU requires a secure context (HTTPS or localhost). Using WebGL2.".into(),
            );
        }
    }
    let canvas_claimed = Cell::new(false);
    match initialize_backend(
        display,
        window,
        clear_color,
        wgpu::Backends::GL,
        "WebGL2",
        WEBGL_INIT_TIMEOUT_SECONDS,
        &canvas_claimed,
        generation,
        proxy,
    )
    .await
    {
        Ok(renderer) => InitOutcome::Ready(renderer),
        Err(error) => InitOutcome::Failed(error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn initialize_backend(
    display: OwnedDisplayHandle,
    window: Arc<Window>,
    clear_color: egui::Color32,
    backends: wgpu::Backends,
    backend: &'static str,
    timeout_seconds: u32,
    canvas_claimed: &Cell<bool>,
    generation: u64,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<WgpuRenderer, NativeError> {
    let initialization = WgpuRenderer::new_with_backends_and_hooks(
        display,
        window,
        clear_color,
        backends,
        |claimed| canvas_claimed.set(claimed),
        move |reason| {
            let _ = proxy.send_event(UserEvent::DeviceLost { generation, reason });
        },
    );
    match select(
        pin!(initialization),
        pin!(TimeoutFuture::new(timeout_seconds * 1_000)),
    )
    .await
    {
        Either::Left((result, _)) => {
            let renderer = result?;
            web_sys::console::info_1(&format!("Dockspace renderer initialized: {backend}").into());
            Ok(renderer)
        }
        Either::Right(((), _)) => Err(NativeError::RendererTimeout {
            backend,
            timeout_seconds,
        }),
    }
}

struct WebGlContextLostGuard {
    canvas: HtmlCanvasElement,
    closure: Closure<dyn FnMut(Event)>,
}

impl WebGlContextLostGuard {
    fn attach(
        canvas: HtmlCanvasElement,
        generation: u64,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Option<Self> {
        let closure = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            event.prevent_default();
            let _ = proxy.send_event(UserEvent::DeviceLost {
                generation,
                reason: "browser WebGL2 context was lost".to_owned(),
            });
        });
        canvas
            .add_event_listener_with_callback("webglcontextlost", closure.as_ref().unchecked_ref())
            .ok()?;
        Some(Self { canvas, closure })
    }
}

impl Drop for WebGlContextLostGuard {
    fn drop(&mut self) {
        let _ = self.canvas.remove_event_listener_with_callback(
            "webglcontextlost",
            self.closure.as_ref().unchecked_ref(),
        );
    }
}

fn replace_canvas_in_dom(old: &HtmlCanvasElement) -> Option<HtmlCanvasElement> {
    let parent = old.parent_node()?;
    let cloned = old.clone_node_with_deep(true).ok()?;
    let fresh = cloned.dyn_into::<HtmlCanvasElement>().ok()?;
    let _ = fresh.remove_attribute("width");
    let _ = fresh.remove_attribute("height");
    parent.replace_child(&fresh, old).ok()?;
    Some(fresh)
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
    let active_renderer = options.renderer;
    event_loop.spawn_app(Runner {
        proxy,
        context,
        options,
        active_renderer,
        app,
        on_error,
        window: None,
        context_lost_guard: None,
        generation: Arc::new(AtomicU64::new(0)),
        repaint: Repaint::default(),
        has_presented: false,
        initial_present_failures: 0,
    });
    Ok(())
}

enum UserEvent {
    Repaint(RepaintRequest),
    GpuReady,
    DeviceLost { generation: u64, reason: String },
}

struct InitializingWindow {
    integration: WinitIntegration,
    window: Arc<Window>,
    result: Rc<RefCell<Option<InitOutcome>>>,
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
    active_renderer: WebRenderer,
    app: A,
    on_error: E,
    window: Option<WebWindow>,
    context_lost_guard: Option<WebGlContextLostGuard>,
    generation: Arc<AtomicU64>,
    repaint: Repaint,
    has_presented: bool,
    initial_present_failures: u8,
}

impl<A: NativeApp, E: FnMut(NativeError)> Runner<A, E> {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), NativeError> {
        if self.window.is_some() {
            self.repaint.resume();
            self.repaint.request(Instant::now());
            return Ok(());
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.context_lost_guard = WebGlContextLostGuard::attach(
            self.options.canvas.clone(),
            generation,
            self.proxy.clone(),
        );
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
        let renderer = self.active_renderer;
        let proxy = self.proxy.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let outcome = initialize_renderer(
                display,
                window,
                clear_color,
                renderer,
                generation,
                proxy.clone(),
            )
            .await;
            if let Some(completion) = completion.upgrade() {
                completion.replace(Some(outcome));
                let _ = proxy.send_event(UserEvent::GpuReady);
            }
        });
        Ok(())
    }

    fn fallback_to_webgl(&mut self, event_loop: &ActiveEventLoop, original_error: NativeError) {
        // Drop the existing listener, window, and surface first so event handlers
        // and GPU resources detach before the canvas element is replaced in the DOM.
        self.context_lost_guard = None;
        self.window = None;
        let Some(fresh_canvas) = replace_canvas_in_dom(&self.options.canvas) else {
            self.fail(event_loop, original_error);
            return;
        };
        self.options.canvas = fresh_canvas;
        self.active_renderer = WebRenderer::WebGl;
        self.initial_present_failures = 0;
        if let Err(error) = self.open(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn finish_initialization(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.window.take() else {
            return;
        };
        let WebWindow::Initializing(mut state) = state else {
            self.window = Some(state);
            return;
        };
        let outcome = state.result.borrow_mut().take();
        match outcome {
            Some(InitOutcome::Ready(renderer)) => {
                if renderer.backend() == wgpu::Backend::Gl {
                    self.active_renderer = WebRenderer::WebGl;
                }
                if let Some(reason) = renderer.device_lost_reason() {
                    let generation = self.generation.load(Ordering::Acquire);
                    drop(renderer);
                    drop(state);
                    self.handle_device_lost(event_loop, generation, reason);
                    return;
                }
                state.integration.set_max_texture_side(
                    renderer.device.limits().max_texture_dimension_2d as usize,
                );
                self.window = Some(WebWindow::Ready(HostWindow {
                    renderer,
                    integration: state.integration,
                    window: state.window,
                }));
                self.initial_present_failures = 0;
                self.repaint.resume();
                self.repaint.request(Instant::now());
            }
            Some(InitOutcome::RetryWebGlOnFreshCanvas(error)) => {
                drop(state);
                self.fallback_to_webgl(event_loop, error);
            }
            Some(InitOutcome::Failed(error)) => self.fail(event_loop, error),
            None => self.window = Some(WebWindow::Initializing(state)),
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: NativeError) {
        self.context_lost_guard = None;
        self.window = None;
        (self.on_error)(error);
        event_loop.exit();
    }

    fn handle_device_lost(
        &mut self,
        event_loop: &ActiveEventLoop,
        generation: u64,
        reason: String,
    ) {
        if generation != self.generation.load(Ordering::Acquire) {
            return;
        }
        let backend = match self.active_renderer {
            WebRenderer::Auto => "WebGPU",
            WebRenderer::WebGl => "WebGL2",
        };
        let error = NativeError::DeviceLost { backend, reason };
        if !self.has_presented && self.active_renderer == WebRenderer::Auto {
            web_sys::console::warn_1(
                &format!("{error}. Replacing canvas to fall back to WebGL2.").into(),
            );
            self.fallback_to_webgl(event_loop, error);
        } else {
            self.fail(event_loop, error);
        }
    }

    fn handle_initial_present_failure(&mut self, event_loop: &ActiveEventLoop, reason: DropReason) {
        let backend = match self.active_renderer {
            WebRenderer::Auto => "WebGPU",
            WebRenderer::WebGl => "WebGL2",
        };
        let error = NativeError::InitialPresentationFailed {
            backend,
            reason: reason.description(),
        };
        if self.active_renderer == WebRenderer::Auto {
            web_sys::console::warn_1(
                &format!("{error}. Replacing canvas to fall back to WebGL2.").into(),
            );
            self.fallback_to_webgl(event_loop, error);
        } else {
            self.fail(event_loop, error);
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(WebWindow::Ready(state)) = &mut self.window else {
            return;
        };
        let render_result =
            state.render(event_loop, &self.context, &mut self.app, &mut self.repaint);
        let drop_reason = state.renderer.drop_reason;
        let device_lost_reason = state.renderer.device_lost_reason();
        match render_result {
            Ok(control) => {
                if let Some(reason) = device_lost_reason {
                    let generation = self.generation.load(Ordering::Acquire);
                    self.handle_device_lost(event_loop, generation, reason);
                    return;
                }
                if drop_reason.is_none() {
                    self.has_presented = true;
                    self.initial_present_failures = 0;
                } else if !self.has_presented
                    && let Some(reason) = drop_reason
                {
                    match reason {
                        DropReason::ZeroSized | DropReason::Occluded => {
                            // Keep a paced startup check alive so a canvas that starts
                            // 0x0 or hidden still presents once layout settles.
                            self.repaint.request(Instant::now());
                        }
                        DropReason::Validation | DropReason::DeviceLost => {
                            self.handle_initial_present_failure(event_loop, reason);
                            return;
                        }
                        DropReason::Lost
                        | DropReason::RecreationFailed
                        | DropReason::Outdated
                        | DropReason::TimedOut => {
                            self.initial_present_failures =
                                self.initial_present_failures.saturating_add(1);
                            if self.initial_present_failures >= MAX_INITIAL_PRESENT_FAILURES {
                                self.handle_initial_present_failure(event_loop, reason);
                                return;
                            }
                        }
                    }
                }
                match control {
                    FrameControl::Continue => {}
                    FrameControl::Repaint => self.repaint.request(Instant::now()),
                    FrameControl::Exit => event_loop.exit(),
                }
            }
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
            UserEvent::DeviceLost { generation, reason } => {
                self.handle_device_lost(event_loop, generation, reason);
            }
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
        self.context_lost_guard = None;
        self.window = None;
    }
}
