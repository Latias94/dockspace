//! WGPU presentation and texture-delta ownership for the single-window runner.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::SurfacePresentationResult;
use egui_wgpu::wgpu;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropReason {
    ZeroSized,
    RecreationFailed,
    Outdated,
    Lost,
    TimedOut,
    Occluded,
    Validation,
    DeviceLost,
}

impl DropReason {
    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::ZeroSized => "zero-sized window",
            Self::RecreationFailed => "surface recreation failed",
            Self::Outdated => "surface outdated",
            Self::Lost => "surface lost",
            Self::TimedOut => "surface timeout",
            Self::Occluded => "surface occluded",
            Self::Validation => "surface validation failure",
            Self::DeviceLost => "graphics device lost",
        }
    }

    pub(super) const fn needs_retry(self) -> bool {
        matches!(
            self,
            Self::RecreationFailed | Self::Outdated | Self::Lost | Self::TimedOut
        )
    }

    pub(super) const fn retry_delay(self) -> Duration {
        match self {
            Self::Outdated | Self::Lost | Self::TimedOut => Duration::from_millis(16),
            Self::RecreationFailed
            | Self::ZeroSized
            | Self::Occluded
            | Self::Validation
            | Self::DeviceLost => Duration::from_millis(250),
        }
    }
}

#[derive(Default)]
struct DeviceHealth {
    lost: AtomicBool,
    lost_reason: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
}

impl DeviceHealth {
    fn record_device_lost(&self, reason: wgpu::DeviceLostReason, message: String) {
        // Intentional shutdown drops or destroys the device cleanly; do not treat it as a fault.
        if reason == wgpu::DeviceLostReason::Destroyed {
            return;
        }
        let summary = if message.trim().is_empty() {
            "unknown driver or browser graphics reset".to_owned()
        } else {
            message
        };
        if let Ok(mut guard) = self.lost_reason.lock()
            && guard.is_none()
        {
            *guard = Some(summary);
        }
        self.lost.store(true, Ordering::Release);
    }

    fn record_uncaptured_error(&self, error: &wgpu::Error) {
        let message = error.to_string();
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(message);
        }
        if matches!(error, wgpu::Error::OutOfMemory { .. }) {
            self.record_device_lost(wgpu::DeviceLostReason::Unknown, error.to_string());
        }
    }

    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    fn lost_reason(&self) -> Option<String> {
        self.lost_reason.lock().ok().and_then(|guard| guard.clone())
    }
}

/// Ensures any staged `Queue::write_buffer` / `Queue::write_texture` writes are
/// flushed even if an early return skips explicit submission (see egui#7928).
struct RendererQueueGuard<'queue> {
    queue: &'queue wgpu::Queue,
    submitted: bool,
}

impl Drop for RendererQueueGuard<'_> {
    fn drop(&mut self) {
        if !self.submitted {
            self.queue.submit([]);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum RendererInitError {
    #[error("WebGPU returned a software fallback adapter")]
    SoftwareFallbackWebGpu,
    #[error(transparent)]
    Wgpu(#[from] egui_wgpu::WgpuError),
}

// Fields are ordered in strict reverse dependency order so GPU resources tear
// down cleanly: egui renderer -> surface -> queue -> device -> adapter -> instance.
pub(super) struct WgpuRenderer {
    pub(super) drop_reason: Option<DropReason>,
    backend: wgpu::Backend,
    clear_color: wgpu::Color,
    health: Arc<DeviceHealth>,
    pub(super) config: wgpu::SurfaceConfiguration,
    needs_reconfigure: bool,
    pub(super) pending_textures: egui::TexturesDelta,
    renderer: egui_wgpu::Renderer,
    surface: Option<wgpu::Surface<'static>>,
    queue: wgpu::Queue,
    pub(super) device: wgpu::Device,
    _adapter: wgpu::Adapter,
    instance: wgpu::Instance,
}

fn is_software_fallback_webgpu_adapter(adapter_info: &wgpu::AdapterInfo) -> bool {
    adapter_info.backend == wgpu::Backend::BrowserWebGpu
        && adapter_info.device_type == wgpu::DeviceType::Cpu
}

fn required_limits_candidates(
    backend: wgpu::Backend,
    adapter_limits: &wgpu::Limits,
) -> Vec<wgpu::Limits> {
    let downlevel = wgpu::Limits::downlevel_webgl2_defaults()
        .using_resolution(adapter_limits.clone())
        .using_alignment(adapter_limits.clone());
    if backend == wgpu::Backend::Gl {
        return vec![downlevel];
    }
    let primary = wgpu::Limits::default()
        .using_resolution(adapter_limits.clone())
        .using_alignment(adapter_limits.clone());
    vec![primary, downlevel]
}

fn clamped_surface_size(
    size: winit::dpi::PhysicalSize<u32>,
    max_texture_dimension_2d: u32,
) -> Option<(u32, u32)> {
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let limit = max_texture_dimension_2d.max(1);
    Some((size.width.min(limit), size.height.min(limit)))
}

#[allow(clippy::cast_precision_loss)]
fn effective_screen_descriptor(
    physical_size: winit::dpi::PhysicalSize<u32>,
    clamped_size: (u32, u32),
    pixels_per_point: f32,
) -> egui_wgpu::ScreenDescriptor {
    let effective_ppp =
        if clamped_size.0 == physical_size.width && clamped_size.1 == physical_size.height {
            pixels_per_point
        } else {
            let scale_x = clamped_size.0 as f32 / physical_size.width.max(1) as f32;
            let scale_y = clamped_size.1 as f32 / physical_size.height.max(1) as f32;
            (pixels_per_point * scale_x.min(scale_y)).max(f32::EPSILON)
        };
    egui_wgpu::ScreenDescriptor {
        size_in_pixels: [clamped_size.0, clamped_size.1],
        pixels_per_point: effective_ppp,
    }
}

fn select_opaque_alpha_mode(supported: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    if supported.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        wgpu::CompositeAlphaMode::Auto
    }
}

const fn preferred_frame_latency(is_ios: bool) -> u32 {
    // Low-latency (1) minimizes pointer/drag-to-photon lag for interactive GUIs;
    // iOS requires 2 to avoid halving display refresh rate.
    if is_ios { 2 } else { 1 }
}

const fn backend_display_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        wgpu::Backend::Gl => {
            if cfg!(target_arch = "wasm32") {
                "WebGL2"
            } else {
                "OpenGL"
            }
        }
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Dx12 => "DirectX 12",
        wgpu::Backend::Noop => "Noop",
    }
}

fn log_gpu_diagnostic(is_error: bool, message: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        if is_error {
            web_sys::console::error_1(&message.into());
        } else {
            web_sys::console::info_1(&message.into());
        }
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        if is_error {
            eprintln!("{message}");
        }
    }
}

async fn request_best_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
    allow_fallback_adapter: bool,
) -> Result<wgpu::Adapter, egui_wgpu::WgpuError> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(adapter) = wgpu::util::initialize_adapter_from_env(instance, compatible_surface).await
    {
        return Ok(adapter);
    }
    let power_preference =
        wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance);
    match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
    {
        Ok(adapter) => Ok(adapter),
        Err(primary_error) if allow_fallback_adapter => instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                compatible_surface,
                force_fallback_adapter: true,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|_| primary_error.into()),
        Err(primary_error) => Err(primary_error.into()),
    }
}

impl WgpuRenderer {
    pub(super) fn needs_another_frame(&self) -> bool {
        self.drop_reason.is_none() && self.needs_reconfigure
    }

    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    pub(super) const fn backend(&self) -> wgpu::Backend {
        self.backend
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) const fn backend_name(&self) -> &'static str {
        backend_display_name(self.backend)
    }

    pub(super) fn device_lost_reason(&self) -> Option<String> {
        self.health.is_lost().then(|| {
            self.health
                .lost_reason()
                .unwrap_or_else(|| "unknown graphics device loss".to_owned())
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) async fn new(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        clear_color: egui::Color32,
        on_device_lost: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self, egui_wgpu::WgpuError> {
        let backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY | wgpu::Backends::GL);
        match Self::new_with_backends_and_hooks(
            display,
            window,
            clear_color,
            backends,
            false,
            |_| {},
            on_device_lost,
        )
        .await
        {
            Ok(renderer) => Ok(renderer),
            Err(RendererInitError::Wgpu(error)) => Err(error),
            Err(RendererInitError::SoftwareFallbackWebGpu) => {
                unreachable!("desktop renderer never rejects software WebGPU adapters")
            }
        }
    }

    #[cfg(all(test, target_os = "linux", feature = "x11"))]
    pub(super) async fn new_with_backends(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        clear_color: egui::Color32,
        backends: wgpu::Backends,
    ) -> Result<Self, egui_wgpu::WgpuError> {
        match Self::new_with_backends_and_hooks(
            display,
            window,
            clear_color,
            backends,
            false,
            |_| {},
            |_| {},
        )
        .await
        {
            Ok(renderer) => Ok(renderer),
            Err(RendererInitError::Wgpu(error)) => Err(error),
            Err(RendererInitError::SoftwareFallbackWebGpu) => {
                unreachable!("desktop renderer never rejects software WebGPU adapters")
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn new_with_backends_and_hooks(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        clear_color: egui::Color32,
        backends: wgpu::Backends,
        reject_software_webgpu: bool,
        mut on_canvas_claim: impl FnMut(bool),
        on_device_lost: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self, RendererInitError> {
        let descriptor = wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: Some(Box::new(display)),
        };
        let instance = wgpu::Instance::new(descriptor);
        #[cfg(not(target_arch = "wasm32"))]
        let surface = Some(
            instance
                .create_surface(Arc::clone(&window))
                .map_err(egui_wgpu::WgpuError::from)?,
        );
        // A browser canvas cannot change context type. WebGPU requests must
        // succeed before claiming it; WebGL needs its canvas to select an adapter.
        #[cfg(target_arch = "wasm32")]
        let surface = if backends.contains(wgpu::Backends::GL) {
            on_canvas_claim(true);
            Some(
                instance
                    .create_surface(Arc::clone(&window))
                    .map_err(egui_wgpu::WgpuError::from)?,
            )
        } else {
            None
        };
        let adapter =
            request_best_adapter(&instance, surface.as_ref(), !reject_software_webgpu).await?;
        let adapter_info = adapter.get_info();
        if reject_software_webgpu && is_software_fallback_webgpu_adapter(&adapter_info) {
            return Err(RendererInitError::SoftwareFallbackWebGpu);
        }
        let adapter_limits = adapter.limits();
        let memory_hints = if cfg!(target_arch = "wasm32") {
            wgpu::MemoryHints::MemoryUsage
        } else {
            wgpu::MemoryHints::Performance
        };
        let mut candidates = required_limits_candidates(adapter_info.backend, &adapter_limits)
            .into_iter()
            .peekable();
        let (device, queue) = loop {
            let required_limits = candidates
                .next()
                .expect("at least one required_limits candidate");
            match adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("dockspace device"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: memory_hints.clone(),
                    ..Default::default()
                })
                .await
            {
                Ok(pair) => break pair,
                Err(_) if candidates.peek().is_some() => continue,
                Err(error) => return Err(egui_wgpu::WgpuError::from(error).into()),
            }
        };
        let health = Arc::new(DeviceHealth::default());
        let on_device_lost = Arc::new(on_device_lost);
        {
            let health = Arc::clone(&health);
            let on_device_lost = Arc::clone(&on_device_lost);
            device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                let message = format!("Dockspace WGPU uncaptured error: {error}");
                log_gpu_diagnostic(true, &message);
                let is_oom = matches!(error, wgpu::Error::OutOfMemory { .. });
                health.record_uncaptured_error(&error);
                if is_oom {
                    on_device_lost(error.to_string());
                }
            }));
        }
        {
            let health = Arc::clone(&health);
            device.set_device_lost_callback(move |reason, message| {
                if reason == wgpu::DeviceLostReason::Destroyed {
                    return;
                }
                let formatted = format!("Dockspace WGPU device lost ({reason:?}): {message}");
                log_gpu_diagnostic(true, &formatted);
                health.record_device_lost(reason, message);
                on_device_lost(
                    health
                        .lost_reason()
                        .unwrap_or_else(|| "unknown graphics device loss".to_owned()),
                );
            });
        }
        let surface = match surface {
            Some(surface) => surface,
            None => {
                on_canvas_claim(true);
                instance
                    .create_surface(Arc::clone(&window))
                    .map_err(egui_wgpu::WgpuError::from)?
            }
        };
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        let size = window.inner_size();
        let (width, height) = (
            size.width.max(1).min(max_dim),
            size.height.max(1).min(max_dim),
        );
        let capabilities = surface.get_capabilities(&adapter);
        let format = egui_wgpu::preferred_framebuffer_format(&capabilities.formats)?;
        let default_config = surface
            .get_default_config(&adapter, width, height)
            .ok_or(egui_wgpu::WgpuError::NoSurfaceFormatsAvailable)?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: preferred_frame_latency(cfg!(target_os = "ios")),
            alpha_mode: select_opaque_alpha_mode(&capabilities.alpha_modes),
            view_formats: vec![format],
            ..default_config
        };
        let renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        log_gpu_diagnostic(
            false,
            &format!(
                "Dockspace WGPU adapter: {} ({}, {:?})",
                adapter_info.name,
                backend_display_name(adapter_info.backend),
                adapter_info.device_type
            ),
        );
        let [r, g, b, a] = egui::Rgba::from(clear_color).to_array();
        Ok(Self {
            backend: adapter_info.backend,
            clear_color: wgpu::Color {
                r: f64::from(r),
                g: f64::from(g),
                b: f64::from(b),
                a: f64::from(a),
            },
            drop_reason: None,
            health,
            config,
            needs_reconfigure: true,
            pending_textures: egui::TexturesDelta::default(),
            renderer,
            surface: Some(surface),
            queue,
            device,
            _adapter: adapter,
            instance,
        })
    }

    pub(super) fn paint(
        &mut self,
        window: &Arc<Window>,
        pixels_per_point: f32,
        clipped_primitives: &[egui::ClippedPrimitive],
    ) -> SurfacePresentationResult {
        self.paint_with_acquire(window, pixels_per_point, clipped_primitives, |surface| {
            surface.get_current_texture()
        })
    }

    fn paint_with_acquire(
        &mut self,
        window: &Arc<Window>,
        pixels_per_point: f32,
        clipped_primitives: &[egui::ClippedPrimitive],
        acquire: impl FnOnce(&wgpu::Surface<'_>) -> wgpu::CurrentSurfaceTexture,
    ) -> SurfacePresentationResult {
        self.drop_reason = None;
        if self.health.is_lost() {
            self.drop_reason = Some(DropReason::DeviceLost);
            return SurfacePresentationResult::Dropped;
        }
        let size = window.inner_size();
        let max_dim = self.device.limits().max_texture_dimension_2d;
        let Some((width, height)) = clamped_surface_size(size, max_dim) else {
            self.drop_reason = Some(DropReason::ZeroSized);
            return SurfacePresentationResult::Dropped;
        };
        if self.surface.is_none() {
            match self.instance.create_surface(Arc::clone(window)) {
                Ok(surface) => self.surface = Some(surface),
                Err(error) => {
                    self.drop_reason = Some(DropReason::RecreationFailed);
                    log_gpu_diagnostic(
                        true,
                        &format!("Failed to recreate the dockspace surface: {error}"),
                    );
                    return SurfacePresentationResult::Dropped;
                }
            }
            self.needs_reconfigure = true;
        }
        let surface = self
            .surface
            .as_ref()
            .expect("the surface was created above");
        if self.needs_reconfigure || self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            surface.configure(&self.device, &self.config);
            self.needs_reconfigure = false;
        }
        let frame = match acquire(surface) {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Reconfigure only after this frame has been presented and released.
                self.needs_reconfigure = true;
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.drop_reason = Some(DropReason::Outdated);
                self.needs_reconfigure = true;
                return SurfacePresentationResult::Dropped;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.drop_reason = Some(DropReason::Lost);
                // Release the lost surface before creating its replacement next frame.
                self.surface = None;
                return SurfacePresentationResult::Dropped;
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.drop_reason = Some(DropReason::TimedOut);
                return SurfacePresentationResult::Dropped;
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.drop_reason = Some(DropReason::Occluded);
                return SurfacePresentationResult::Dropped;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.drop_reason = Some(DropReason::Validation);
                return SurfacePresentationResult::Dropped;
            }
        };

        let mut queue_guard = RendererQueueGuard {
            queue: &self.queue,
            submitted: false,
        };

        // Acquire before consuming any deltas. Failed acquisitions retain the initial font atlas,
        // subsequent patches, and deferred frees until a later frame can upload and submit them.
        for (id, image_deltas) in self.pending_textures.set.drain() {
            for image_delta in image_deltas {
                self.renderer
                    .update_texture(&self.device, &self.queue, id, &image_delta);
            }
        }
        let screen_descriptor =
            effective_screen_descriptor(size, (width, height), pixels_per_point);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dockspace encoder"),
            });
        let user_commands = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            clipped_primitives,
            &screen_descriptor,
        );
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("dockspace egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer.render(
                &mut render_pass.forget_lifetime(),
                clipped_primitives,
                &screen_descriptor,
            );
        }
        self.queue
            .submit(user_commands.into_iter().chain([encoder.finish()]));
        queue_guard.submitted = true;
        // Current-frame draws may still reference these textures until submission.
        for id in self.pending_textures.free.drain() {
            self.renderer.free_texture(&id);
        }
        window.pre_present_notify();
        self.queue.present(frame);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::Poll);
        SurfacePresentationResult::Presented
    }
}

#[cfg(all(test, target_os = "linux", feature = "x11"))]
mod recovery_tests;

impl Drop for WgpuRenderer {
    fn drop(&mut self) {
        // The renderer and context are shutting down; no future frame will use these deltas.
        self.pending_textures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DeviceHealth, DropReason, clamped_surface_size, effective_screen_descriptor,
        is_software_fallback_webgpu_adapter, preferred_frame_latency, required_limits_candidates,
        select_opaque_alpha_mode,
    };
    use egui_wgpu::wgpu;
    use std::time::Duration;
    use winit::dpi::PhysicalSize;

    #[test]
    fn failed_surface_recreation_keeps_the_runner_wakeable() {
        assert!(DropReason::RecreationFailed.needs_retry());
        assert_eq!(
            DropReason::RecreationFailed.retry_delay(),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn transient_surface_loss_keeps_the_fast_recovery_budget() {
        assert_eq!(DropReason::Lost.retry_delay(), Duration::from_millis(16));
        assert_eq!(
            DropReason::Outdated.retry_delay(),
            Duration::from_millis(16)
        );
        assert_eq!(
            DropReason::TimedOut.retry_delay(),
            Duration::from_millis(16)
        );
    }

    #[test]
    fn clamped_surface_size_rejects_zero_and_caps_oversized_dimensions() {
        assert_eq!(clamped_surface_size(PhysicalSize::new(0, 800), 4096), None);
        assert_eq!(clamped_surface_size(PhysicalSize::new(800, 0), 4096), None);
        assert_eq!(
            clamped_surface_size(PhysicalSize::new(1920, 1080), 4096),
            Some((1920, 1080))
        );
        assert_eq!(
            clamped_surface_size(PhysicalSize::new(5120, 2880), 4096),
            Some((4096, 2880))
        );
    }

    #[test]
    fn effective_screen_descriptor_preserves_unclamped_and_scales_clamped_frames() {
        let normal = effective_screen_descriptor(PhysicalSize::new(1600, 900), (1600, 900), 2.0);
        assert_eq!(normal.size_in_pixels, [1600, 900]);
        assert!((normal.pixels_per_point - 2.0).abs() < f32::EPSILON);

        let clamped = effective_screen_descriptor(PhysicalSize::new(8192, 4096), (4096, 4096), 2.0);
        assert_eq!(clamped.size_in_pixels, [4096, 4096]);
        assert!((clamped.pixels_per_point - 1.0).abs() < 1e-6);
    }

    #[test]
    fn required_limits_candidates_use_alignment_and_offer_downlevel_webgpu_fallback() {
        let mut adapter_limits = wgpu::Limits::downlevel_webgl2_defaults();
        adapter_limits.max_texture_dimension_2d = 4096;
        adapter_limits.min_uniform_buffer_offset_alignment = 512;

        let gl_candidates = required_limits_candidates(wgpu::Backend::Gl, &adapter_limits);
        assert_eq!(gl_candidates.len(), 1);
        assert_eq!(gl_candidates[0].max_texture_dimension_2d, 4096);
        assert_eq!(gl_candidates[0].min_uniform_buffer_offset_alignment, 512);

        let webgpu_candidates =
            required_limits_candidates(wgpu::Backend::BrowserWebGpu, &adapter_limits);
        assert_eq!(webgpu_candidates.len(), 2);
        assert_eq!(
            webgpu_candidates[0].min_uniform_buffer_offset_alignment,
            512
        );
        assert_eq!(webgpu_candidates[1].max_texture_dimension_2d, 4096);
        assert_eq!(webgpu_candidates[1].max_storage_buffers_per_shader_stage, 0);
    }

    #[test]
    fn opaque_alpha_mode_and_frame_latency_follow_wgpu_gui_defaults() {
        assert_eq!(
            select_opaque_alpha_mode(&[
                wgpu::CompositeAlphaMode::PreMultiplied,
                wgpu::CompositeAlphaMode::Opaque,
            ]),
            wgpu::CompositeAlphaMode::Opaque
        );
        assert_eq!(
            select_opaque_alpha_mode(&[wgpu::CompositeAlphaMode::PreMultiplied]),
            wgpu::CompositeAlphaMode::Auto
        );
        assert_eq!(preferred_frame_latency(false), 1);
        assert_eq!(preferred_frame_latency(true), 2);
    }

    #[test]
    fn device_health_ignores_intentional_destroy_and_records_unexpected_loss() {
        let health = DeviceHealth::default();
        health.record_device_lost(wgpu::DeviceLostReason::Destroyed, "shutdown".to_owned());
        assert!(!health.is_lost());
        assert_eq!(health.lost_reason(), None);

        health.record_device_lost(wgpu::DeviceLostReason::Unknown, "TDR reset".to_owned());
        assert!(health.is_lost());
        assert_eq!(health.lost_reason().as_deref(), Some("TDR reset"));
    }

    #[test]
    fn software_fallback_webgpu_detection_only_rejects_cpu_browser_webgpu_adapters() {
        let adapter = |backend, device_type| wgpu::AdapterInfo {
            name: String::new(),
            vendor: 0,
            device: 0,
            device_type,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: None,
            limit_bucket: None,
        };

        assert!(is_software_fallback_webgpu_adapter(&adapter(
            wgpu::Backend::BrowserWebGpu,
            wgpu::DeviceType::Cpu,
        )));
        assert!(!is_software_fallback_webgpu_adapter(&adapter(
            wgpu::Backend::BrowserWebGpu,
            wgpu::DeviceType::Other,
        )));
        assert!(!is_software_fallback_webgpu_adapter(&adapter(
            wgpu::Backend::BrowserWebGpu,
            wgpu::DeviceType::DiscreteGpu,
        )));
        assert!(!is_software_fallback_webgpu_adapter(&adapter(
            wgpu::Backend::Gl,
            wgpu::DeviceType::Cpu,
        )));
        assert!(!is_software_fallback_webgpu_adapter(&adapter(
            wgpu::Backend::Vulkan,
            wgpu::DeviceType::Cpu,
        )));
    }
}
