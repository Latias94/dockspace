//! Native configuration reuses the core's host-clock-driven motion policy.

use std::time::Duration;

use dockspace::motion::{MotionFrame, MotionPreference, MotionSettings};
use eframe::egui;
use egui_dockspace::PaneView;

use super::NativeRuntimeState;
use crate::application_action::{
    NativeApplicationAction, NativeApplicationCommand, NativeApplicationSubmission,
    NativeSubmittedApplicationAction,
};
use crate::{NativeActionReceipt, NativeActionRequestError, NativeActionStatus};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeMotionConfiguration {
    pub(crate) settings: MotionSettings,
    pub(crate) preference: MotionPreference,
}

impl Default for NativeMotionConfiguration {
    fn default() -> Self {
        Self {
            settings: MotionSettings::default(),
            preference: MotionPreference::Unknown,
        }
    }
}

impl NativeMotionConfiguration {
    pub(crate) fn frame(self, now: Duration, ui: &egui::Ui) -> MotionFrame {
        MotionFrame {
            now,
            settings: self.settings,
            preference: if ui.style().animation_time <= 0.0 {
                MotionPreference::Reduced
            } else {
                self.preference
            },
        }
    }
}

impl<P: PaneView> NativeRuntimeState<P> {
    pub(crate) const fn motion_settings(&self) -> MotionSettings {
        self.motion.settings
    }

    pub(crate) const fn motion_preference(&self) -> MotionPreference {
        self.motion.preference
    }

    pub(crate) fn set_initial_motion(
        &mut self,
        configuration: NativeMotionConfiguration,
    ) -> Result<(), NativeActionRequestError> {
        self.ensure_application_slot_available()?;
        if self.root_context.is_some()
            || self.owned_update.is_some()
            || self.coordinator.has_pending_callback_records()
            || self.coordinator.session().has_active_gesture()
        {
            return Err(NativeActionRequestError::Busy);
        }
        self.motion = configuration;
        Ok(())
    }

    pub(crate) fn request_motion(
        &mut self,
        configuration: NativeMotionConfiguration,
    ) -> Result<NativeActionReceipt, NativeActionRequestError> {
        self.ensure_application_slot_available()?;
        self.enqueue_application_action(NativeApplicationAction::new(
            self.coordinator.session().version(),
            NativeApplicationCommand::Motion {
                changed: self.motion != configuration,
                configuration: Box::new(configuration),
            },
        ))
    }
}

#[cfg(test)]
mod tests;

impl NativeApplicationSubmission {
    pub(crate) fn encoding_motion(&self) -> Option<NativeMotionConfiguration> {
        match self {
            Self::Motion { configuration, .. } => Some(**configuration),
            _ => None,
        }
    }
}

impl NativeSubmittedApplicationAction {
    pub(crate) fn commit_motion(&mut self) -> Option<NativeMotionConfiguration> {
        let Self::Motion {
            configuration,
            changed,
        } = self
        else {
            return None;
        };
        let configuration = **configuration;
        *self = Self::Terminal(NativeActionStatus::MotionChanged { changed: *changed });
        Some(configuration)
    }
}
