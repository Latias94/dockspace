//! One complete-output render attempt, shared by desktop and browser hosts.

use std::sync::Arc;

use egui::{Context, FullOutput, ViewportId};
use web_time::Instant;
use winit::{event_loop::ActiveEventLoop, window::Window};

use super::{
    ActionRequested, FrameControl, NativeApp, NativeError, NativeFrameReport, WinitIntegration,
    renderer::WgpuRenderer, repaint::Repaint,
};

// The renderer releases its surface before the retained window is dropped.
pub(super) struct HostWindow {
    pub(super) renderer: WgpuRenderer,
    pub(super) integration: WinitIntegration,
    pub(super) window: Arc<Window>,
}

impl HostWindow {
    pub(super) fn render(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &Context,
        app: &mut (impl NativeApp + ?Sized),
        repaint: &mut Repaint,
    ) -> Result<FrameControl, NativeError> {
        repaint.frame_started(Instant::now());
        let mut input = self.integration.take_input(&self.window);
        app.prepare_input(&mut input)
            .map_err(NativeError::Application)?;
        let started = Instant::now();
        let output = app
            .update(context, input)
            .map_err(NativeError::Application)?;
        let update_duration = started.elapsed();
        let (output, receipt) = output.into_parts();
        let FullOutput {
            platform_output,
            textures_delta: _,
            shapes,
            pixels_per_point,
            mut viewport_output,
        } = stage_output(output, &mut self.renderer.pending_textures)?;
        // Preserve platform delivery while retaining read-only diagnostics for the app.
        let accesskit_update = platform_output.accesskit_update.clone();
        let commands = viewport_output
            .remove(&ViewportId::ROOT)
            .map(|output| output.commands);
        let response = self.integration.handle_output(
            &self.window,
            event_loop,
            platform_output,
            commands.into_iter().flatten(),
        );
        if response
            .actions
            .iter()
            .any(|action| matches!(action, ActionRequested::Screenshot(_)))
        {
            return Err(NativeError::ScreenshotUnsupported);
        }
        // Every redraw runs UI and attempts painting, satisfying PaintWhileHidden as well.
        let tessellation_started = Instant::now();
        let primitives = context.tessellate(shapes, pixels_per_point);
        let tessellation_duration = tessellation_started.elapsed();
        let paint_started = Instant::now();
        let presentation = self
            .renderer
            .paint(&self.window, pixels_per_point, &primitives);
        let paint_duration = paint_started.elapsed();
        let now = Instant::now();
        settle_presentation(&self.renderer, receipt, presentation, repaint, now);
        let frame_duration = started.elapsed();
        let control = app
            .frame_finished(NativeFrameReport {
                context,
                presentation,
                drop_reason: self
                    .renderer
                    .drop_reason
                    .map(super::renderer::DropReason::description),
                accesskit_update: accesskit_update.as_ref(),
                primitives: &primitives,
                update_duration,
                tessellation_duration,
                paint_duration,
                frame_duration,
            })
            .map_err(NativeError::Application)?;
        if response.close_requested {
            Ok(FrameControl::Exit)
        } else {
            Ok(control)
        }
    }
}

pub(super) fn stage_output(
    mut output: FullOutput,
    pending_textures: &mut egui::TexturesDelta,
) -> Result<FullOutput, NativeError> {
    // Take texture ownership even when rejecting output. Nonempty deltas must not be dropped.
    pending_textures.append(std::mem::take(&mut output.textures_delta));
    if output
        .viewport_output
        .keys()
        .any(|id| *id != ViewportId::ROOT)
    {
        return Err(NativeError::AdditionalViewport);
    }
    Ok(output)
}

pub(super) fn settle_presentation(
    renderer: &WgpuRenderer,
    receipt: crate::DockspaceOutputReceipt,
    presentation: crate::SurfacePresentationResult,
    repaint: &mut Repaint,
    now: Instant,
) {
    if let Some(reason) = renderer.drop_reason {
        repaint.defer_until(now + reason.retry_delay());
        if reason.needs_retry() {
            repaint.request(now);
        }
    } else {
        repaint.resume();
    }
    if renderer.needs_another_frame() {
        repaint.request(now);
    }
    receipt.report(presentation);
}

#[cfg(test)]
mod tests {
    use super::{NativeError, stage_output};

    #[test]
    fn unsupported_viewport_retains_font_textures_without_panicking() {
        let context = egui::Context::default();
        context.set_embed_viewports(false);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.label("This first output contains a font texture.");
            ui.ctx().show_viewport_deferred(
                egui::ViewportId::from_hash_of("unsupported-window"),
                egui::ViewportBuilder::default(),
                |_, _| {},
            );
        });
        assert!(!output.textures_delta.is_empty());
        let mut pending = egui::TexturesDelta::default();
        let result = stage_output(output, &mut pending);
        let retained = !pending.is_empty();
        pending.clear();
        assert!(matches!(result, Err(NativeError::AdditionalViewport)));
        assert!(
            retained,
            "shutdown must own and retire the pending textures"
        );
    }
}
