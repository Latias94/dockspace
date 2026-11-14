//! Complete the last inventory from exact destruction, without a new UI output.

use super::*;

impl NativeHostBridge {
    /// Quarantine closes new creation admission. Every materialized lifetime
    /// must also have committed destruction before an empty inventory is true.
    /// Attachment retirement alone proves neither fact; some hosts retain their
    /// attachment past the last event-loop callback.
    pub(crate) fn capture_destroyed_window_roster(&self) -> bool {
        let mut records = self.lock();
        if records.mode != HostIngressMode::Quarantined
            || !records.terminal_roster.needs_capture()
            || records.event_boundary_pending
            || records.retained_record_count() != 0
            || records.in_flight_viewport_roster.is_some()
            || records.output_resources.values().next().is_some()
            || records.window_resources.awaiting_staging_count() != 0
            || !records.retirements.all_window_lifetimes_destroyed()
            || !records
                .viewports
                .all_window_lifetimes_destroyed(&records.retirements)
        {
            return false;
        }
        let Some(backend) = records
            .committed_viewport_roster
            .as_ref()
            .map(|key| key.roster.backend())
        else {
            return false;
        };
        let Some(payload) = records.terminal_roster.capture_empty(backend) else {
            return false;
        };
        records.record_terminal_viewport_roster(payload)
    }
}
