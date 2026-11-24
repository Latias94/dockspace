//! Core-owned host-frame capabilities and their atomic phase transitions.

use super::*;

#[cfg(test)]
mod error_tests;

mod begin;
mod commit;
mod ingress;
mod input;
mod presentation;
mod view;

/// Derived encoding state retained while one host run tries its actual passes.
/// The semantic workspace and input journals stay in their single candidate.
#[derive(Debug)]
pub(crate) struct CoreEncodingCheckpoint {
    presentation: PresentationAuthorityState,
    interaction: InteractionState,
    roster: HostPresentationRoster,
    snapshot_changed: bool,
    projection_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerJournalSubmissionAuthority {
    #[cfg(test)]
    RawLease,
    SurfaceLocalProducer,
    BackendIngress,
}
