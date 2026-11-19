//! Renderer-neutral product-action results shared by application adapters.

use crate::model::{
    DockspaceActionOutcome, DockspaceActionRejection, RootId, SurfaceId, WorkspaceVersion,
};

use super::{DockspacePresentationTransitionId, DockspacePresentationTransitionResult};

/// Product-level result of submitting one revision-bound action.
///
/// Adapters can use this type for command results without depending on another
/// UI adapter. It describes semantic status, independently of renderer output
/// ownership or native host failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockspaceActionStatus {
    /// The action committed or produced a valid no-op.
    Applied(DockspaceActionOutcome),
    /// The action was accepted and source ownership remains unchanged pending presentation.
    ///
    /// Match `transition` against a later committed report or notification.
    /// An operation terminal does not discharge physical window cleanup.
    Requested {
        /// Immediate product result describing the accepted transfer.
        outcome: DockspaceActionOutcome,
        /// Session-scoped identity of the eventual semantic terminal.
        transition: DockspacePresentationTransitionId,
    },
    /// Current policy or topology rejected the action.
    Rejected(DockspaceActionRejection),
    /// The action was prepared from an older workspace revision.
    Stale {
        /// Version carried by the prepared action.
        expected: WorkspaceVersion,
        /// Version accepted by the reducer boundary.
        accepted: WorkspaceVersion,
    },
    /// A presentation-gated action was accepted, but its exact transfer barrier failed.
    PresentationFailed {
        /// Stable root retained while the target presentation was prepared.
        root: RootId,
        /// Surface which retained ownership before the failed transfer.
        source_surface: SurfaceId,
        /// Surface whose exact presentation terminal settled the transfer.
        target_surface: SurfaceId,
        /// Stable terminal category reported by the core presentation authority.
        reason: DockspacePresentationTransitionResult,
    },
}
