//! Application-facing motion configuration at the native service boundary.

use dockspace::motion::{MotionPreference, MotionSettings};
use egui_dockspace::PaneView;

use super::NativeDockspaceApp;
use crate::surface_driver::NativeMotionConfiguration;
use crate::{NativeActionReceipt, NativeActionRequestError};

impl<P: PaneView + Send + 'static> NativeDockspaceApp<P> {
    /// Configures shared pane motion before the native host receives input.
    ///
    /// Motion is enabled by default. The default system preference is Unknown:
    /// the adapter does not infer a portable OS accessibility fact. A zero egui
    /// animation time also requests reduced motion. Native window bounds and
    /// first-live ownership admission always retain their existing static path.
    ///
    /// # Errors
    ///
    /// Returns Busy after attachment/input or while the application slot is
    /// occupied, and Stopped after terminal shutdown.
    pub fn with_motion(
        self,
        settings: MotionSettings,
        preference: MotionPreference,
    ) -> Result<Self, NativeActionRequestError> {
        crate::notification::lock_state(&self.state).set_initial_motion(
            NativeMotionConfiguration {
                settings,
                preference,
            },
        )?;
        Ok(self)
    }

    /// Returns the currently committed motion settings.
    #[must_use]
    pub fn motion_settings(&self) -> MotionSettings {
        crate::notification::lock_state(&self.state).motion_settings()
    }

    /// Returns the supplied preference before egui's zero-time override.
    #[must_use]
    pub fn motion_preference(&self) -> MotionPreference {
        crate::notification::lock_state(&self.state).motion_preference()
    }

    /// Queues settings and an observed preference at the next eligible native boundary.
    ///
    /// The next eligible outer update freezes them with the native monotonic
    /// clock. Repeated egui passes reuse that sample. This changes no layout
    /// measurements, docking semantics or OS-window animation. Off and Reduced
    /// settle immediately. The returned receipt reports MotionChanged independently
    /// of later requests. An unread completed receipt does not retain capacity.
    ///
    /// # Errors
    ///
    /// Returns Busy while the slot is occupied, or Stopped after fatal shutdown.
    pub fn request_motion(
        &self,
        settings: MotionSettings,
        preference: MotionPreference,
    ) -> Result<NativeActionReceipt, NativeActionRequestError> {
        crate::notification::lock_state(&self.state).request_motion(NativeMotionConfiguration {
            settings,
            preference,
        })
    }
}
