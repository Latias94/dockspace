//! Admission and publication of the official host-local pointer lane.

use std::sync::Arc;
use std::time::Duration;

use dockspace::model::{ItemId, SurfaceId};
use dockspace::runtime::{
    DockspaceHostFrame, DockspaceInteractionError, DockspaceRuntimeError, DockspaceSession,
    HostFrameReport, PaintedSurfaceOutput,
};
use egui::{Id, Modifiers, Ui};

use crate::focus::DeferredTabFocus;
use crate::input::DockInputConfig;
use crate::pass_settlement::{InputPassId, RunIdentity, pointer_packet};

mod receiver;
mod stream;
mod touch;

#[cfg(test)]
mod tests;

pub(crate) use receiver::{
    ProductPointerInputSnapshot, ProductPointerPresentation, has_foreign_drag,
};
pub(crate) use stream::event_modifiers;

use receiver::{PointerReceivers, PointerScenes};
use stream::{PendingPointerRecovery, PointerStreams};

#[derive(Default)]
pub(crate) struct ProductPointer {
    config: DockInputConfig,
    scenes: PointerScenes,
    streams: PointerStreams,
    observed: Option<ObservedRun>,
    observed_modifiers: Option<Modifiers>,
    recovery: PendingPointerRecovery,
}

/// egui's input watermark advances even when the Dockspace frame fails.
struct ObservedRun {
    identity: Arc<RunIdentity>,
    through: InputPassId,
    committed: bool,
}

/// Stream state belongs to the same unpublished update as the core input prefix.
#[derive(Default)]
#[must_use = "publish this input only after committing its host frame"]
pub(crate) struct PointerUpdate {
    streams: Option<PointerStreams>,
    pub(crate) observed_at: Option<Duration>,
    pub(crate) focus: DeferredTabFocus,
}

impl ProductPointer {
    pub(crate) const fn input(&self) -> DockInputConfig {
        self.config
    }

    pub(crate) fn set_input(&mut self, config: DockInputConfig) {
        self.config = config;
    }

    pub(crate) fn clear(&mut self) {
        let mut streams = std::mem::take(&mut self.streams);
        streams.clear();
        *self = Self {
            config: self.config,
            streams,
            ..Self::default()
        };
    }

    pub(crate) fn settle_report(
        &mut self,
        report: &mut HostFrameReport,
        presentation: ProductPointerPresentation,
    ) -> Option<PaintedSurfaceOutput> {
        let output = report.take_painted_outputs().pop();
        self.scenes.observe(report.surface_presentation_commits());
        if let Some(output) = output.as_ref() {
            self.scenes
                .emitted(output.presentation_artifact(), presentation);
        }
        output
    }

    pub(crate) fn presented_focus_bindings(
        &self,
        frame: &DockspaceHostFrame<'_>,
        surface: SurfaceId,
    ) -> &[(ItemId, Id)] {
        self.scenes.focus_bindings(frame.presented_surface(surface))
    }

    /// Reduces the original packet once and remembers consumed physical facts even on failure.
    pub(crate) fn submit(
        &mut self,
        frame: &mut DockspaceHostFrame<'_>,
        surface: SurfaceId,
        ui: &Ui,
    ) -> Result<PointerUpdate, DockspaceRuntimeError> {
        let Some(packet) = pointer_packet(ui) else {
            return Ok(PointerUpdate::default());
        };
        let observed_at = packet.observed_at();
        if let Some(run) = self.observed.as_mut()
            && Arc::ptr_eq(&run.identity, packet.run_identity())
        {
            run.through = packet.observed_input_pass();
            return if run.committed {
                Ok(PointerUpdate {
                    observed_at,
                    ..PointerUpdate::default()
                })
            } else {
                Err(DockspaceInteractionError::PointerInputAlreadySubmitted.into())
            };
        }

        let previous_pass = self.observed.as_ref().map(|run| run.through);
        let input_gap = previous_pass != packet.preceding_input_pass();
        if input_gap {
            self.recovery.mark_gap(&self.streams);
        }
        let modifiers = if previous_pass.is_none() || input_gap {
            packet.initial_modifiers()
        } else {
            self.observed_modifiers.unwrap_or(Modifiers::NONE)
        };
        self.observed = Some(ObservedRun {
            identity: Arc::clone(packet.run_identity()),
            through: packet.observed_input_pass(),
            committed: false,
        });

        let receivers =
            PointerReceivers::new(&self.scenes, frame.presented_surface(surface), ui, &packet);
        let mut streams = self.streams.clone();
        let samples = match streams.normalize(
            &packet,
            self.config,
            modifiers,
            &self.recovery,
            receivers.coordinates_supported(),
            receivers.foreign_drag(),
        ) {
            Ok(samples) => samples,
            Err(error) => {
                // No complete normalization is available. Retire the admitted streams on recovery.
                self.recovery.mark_gap(&self.streams);
                return Err(error.into());
            }
        };
        let blocker = receivers.stationary_blocker(
            frame,
            surface,
            &self.streams,
            &self.recovery,
            &samples.events,
        );
        self.observed_modifiers = Some(samples.modifiers);
        self.recovery.remember(&self.streams, &samples.events);
        let focus = receivers.submit(
            frame,
            surface,
            blocker.into_iter().chain(samples.events),
            &streams,
            samples.focused,
        )?;
        Ok(PointerUpdate {
            streams: Some(streams),
            observed_at,
            focus,
        })
    }

    /// An owned run encodes later passes without submitting the packet again.
    pub(crate) fn observe_later_pass(&mut self, identity: &Arc<RunIdentity>, through: InputPassId) {
        if let Some(run) = self.observed.as_mut()
            && Arc::ptr_eq(&run.identity, identity)
        {
            run.through = through;
        }
    }

    pub(crate) fn commit_input(&mut self, update: &mut PointerUpdate) {
        if let Some(streams) = update.streams.take() {
            self.streams = streams;
            self.recovery = PendingPointerRecovery::default();
            if let Some(run) = self.observed.as_mut() {
                run.committed = true;
            }
        }
    }
}

/// Schedules retained stationary work from the state that actually published.
///
/// Call after every successful update, including one that could not safely
/// append a continuation. The next run rechecks fresh output facts without
/// replaying the previous run's raw packet.
pub(crate) fn schedule_pointer_continuation(
    session: &DockspaceSession,
    context: &egui::Context,
    viewport: egui::ViewportId,
    observed_at: Option<Duration>,
) {
    if let (Some(request), Some(now)) = (session.pointer_continuation(), observed_at) {
        context.request_repaint_after_for(request.deadline().saturating_sub(now), viewport);
    }
}
