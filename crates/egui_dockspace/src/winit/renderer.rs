//! WGPU presentation and texture-delta ownership for the single-window runner.

use std::{sync::Arc, time::Duration};

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
            Self::RecreationFailed | Self::ZeroSized | Self::Occluded | Self::Validation => {
                Duration::from_millis(250)
            }
        }
    }
}

pub(super) struct WgpuRenderer {
    pub(super) drop_reason: Option<DropReason>,
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    backend: wgpu::Backend,
    clear_color: wgpu::Color,
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    pub(super) device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    pub(super) config: wgpu::SurfaceConfiguration,
    needs_reconfigure: bool,
    pub(super) pending_textures: egui::TexturesDelta,
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

impl WgpuRenderer {
    pub(super) fn needs_another_frame(&self) -> bool {
        self.drop_reason.is_none() && self.needs_reconfigure
    }

    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    pub(super) const fn backend(&self) -> wgpu::Backend {
        self.backend
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) async fn new(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        clear_color: egui::Color32,
    ) -> Result<Self, egui_wgpu::WgpuError> {
        Self::new_with_backends(display, window, clear_color, wgpu::Backends::all()).await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) async fn new_with_backends(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        clear_color: egui::Color32,
        backends: wgpu::Backends,
    ) -> Result<Self, egui_wgpu::WgpuError> {
        Self::new_with_backends_and_claim(display, window, clear_color, backends, |_| {}).await
    }

    pub(super) async fn new_with_backends_and_claim(
        display: OwnedDisplayHandle,
        window: Arc<Window>,
        clear_color: egui::Color32,
        backends: wgpu::Backends,
        mut on_canvas_claim: impl FnMut(bool),
    ) -> Result<Self, egui_wgpu::WgpuError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        #[cfg(not(target_arch = "wasm32"))]
        let surface = Some(instance.create_surface(Arc::clone(&window))?);
        // A browser canvas cannot change context type. WebGPU requests must
        // succeed before claiming it; WebGL needs its canvas to select an adapter.
        #[cfg(target_arch = "wasm32")]
        let surface = if backends.contains(wgpu::Backends::GL) {
            on_canvas_claim(true);
            Some(instance.create_surface(Arc::clone(&window))?)
        } else {
            None
        };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: surface.as_ref(),
                ..Default::default()
            })
            .await?;
        let adapter_info = adapter.get_info();
        let adapter_limits = adapter.limits();
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
                    required_limits,
                    ..Default::default()
                })
                .await
            {
                Ok(pair) => break pair,
                Err(_) if candidates.peek().is_some() => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let surface = match surface {
            Some(surface) => surface,
            None => {
                on_canvas_claim(true);
                instance.create_surface(Arc::clone(&window))?
            }
        };
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        let size = window.inner_size();
        let (width, height) = (
            size.width.max(1).min(max_dim),
            size.height.max(1).min(max_dim),
        );
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or(egui_wgpu::WgpuError::NoSurfaceFormatsAvailable)?;
        config.format =
            egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        let [r, g, b, a] = egui::Rgba::from(clear_color).to_array();
        Ok(Self {
            #[cfg(all(feature = "web", target_arch = "wasm32"))]
            backend: adapter_info.backend,
            clear_color: wgpu::Color {
                r: f64::from(r),
                g: f64::from(g),
                b: f64::from(b),
                a: f64::from(a),
            },
            drop_reason: None,
            instance,
            surface: Some(surface),
            device,
            queue,
            renderer,
            config,
            needs_reconfigure: true,
            pending_textures: egui::TexturesDelta::default(),
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
                    eprintln!("Failed to recreate the dockspace surface: {error}");
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
        // Current-frame draws may still reference these textures until submission.
        for id in self.pending_textures.free.drain() {
            self.renderer.free_texture(&id);
        }
        window.pre_present_notify();
        self.queue.present(frame);
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
        DropReason, clamped_surface_size, effective_screen_descriptor, required_limits_candidates,
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
}
