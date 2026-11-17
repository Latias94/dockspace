//! Application callbacks shared by desktop and browser complete-output hosts.

use std::{error::Error, time::Duration};

use egui::{Context, RawInput};

use crate::{DockspaceOutput, SurfacePresentationResult};

/// Application failures returned from a complete-output frame callback.
pub type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Startup and execution failure of a complete-output winit host.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    /// The event loop could not start or continue.
    #[error("native event loop failed: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    /// The window system could not create the window.
    #[error("native window creation failed: {0}")]
    Window(#[from] winit::error::OsError),
    /// WGPU could not initialize its renderer and presentation surface.
    #[error("native renderer initialization failed: {0}")]
    Renderer(#[from] egui_wgpu::WgpuError),
    /// A browser graphics backend did not finish its asynchronous requests.
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    #[error("{backend} renderer initialization timed out after 10 seconds")]
    RendererTimeout { backend: &'static str },
    /// The application's frame callback failed.
    #[error("native application failed: {0}")]
    Application(#[source] Box<dyn Error + Send + Sync>),
    /// A single-window runner cannot create another viewport.
    #[error("the single-window runner received a separate native viewport")]
    AdditionalViewport,
    /// A screenshot requires renderer readback, which this runner does not provide.
    #[error("the single-window runner does not support screenshot readback")]
    ScreenshotUnsupported,
}

/// Scheduling decision after one renderer attempt and its exact receipt report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameControl {
    /// Wait for input or a requested repaint deadline.
    #[default]
    Continue,
    /// Request another application frame immediately.
    Repaint,
    /// Exit the event loop after this frame.
    Exit,
}

/// Read-only observations after the renderer reported this exact output.
///
/// Durations measure CPU work through the return from `queue.present`. They do
/// not measure GPU completion, display scanout, or input delivery.
pub struct NativeFrameReport<'frame> {
    /// Context used by the application run which produced this output.
    pub context: &'frame Context,
    /// Exact renderer disposition already reported to the docking receipt.
    pub presentation: SurfacePresentationResult,
    /// Renderer diagnostic when the output was dropped, if one is available.
    pub drop_reason: Option<&'static str>,
    /// Accessibility tree update from this complete output, when generated.
    pub accesskit_update: Option<&'frame egui::accesskit::TreeUpdate>,
    /// Tessellated paint primitives retained for this renderer attempt.
    pub primitives: &'frame [egui::ClippedPrimitive],
    /// CPU time spent in [`NativeApp::update`].
    pub update_duration: Duration,
    /// CPU time spent tessellating the output's shapes.
    pub tessellation_duration: Duration,
    /// CPU time spent in the renderer's paint and presentation attempt.
    pub paint_duration: Duration,
    /// Total CPU time from the application update through renderer settlement.
    pub frame_duration: Duration,
}

/// Application code inside a complete-output winit runner.
///
/// The application owns its Dockspace and panes. `update` must return the exact
/// output of its owned egui run; the runner does not start a second egui run.
/// The desktop runner may borrow the application; the browser runner owns it
/// because browser event dispatch continues after startup returns.
pub trait NativeApp {
    /// Optionally adjusts input before application timing begins.
    fn prepare_input(&mut self, _input: &mut RawInput) -> AppResult<()> {
        Ok(())
    }

    /// Produces one complete output, including its affine presentation receipt.
    fn update(&mut self, context: &Context, input: RawInput) -> AppResult<DockspaceOutput>;

    /// Applies commands or observes diagnostics after exact renderer settlement.
    fn frame_finished(&mut self, _frame: NativeFrameReport<'_>) -> AppResult<FrameControl> {
        Ok(FrameControl::Continue)
    }
}
