//! Application intents prepared by the native runtime at admission.

use dockspace::close::{
    CloseDecision, CloseDecisionToken, CloseRequestId, DeferredCloseDecision, DeferredCloseToken,
};
use dockspace::model::{
    DockMoveSource, DockMoveTarget, DockPlacement, ItemId, PreparedTab, RootId,
};
use dockspace::policy::DockPolicy;

use crate::NativeActionRequestError;

/// An unaccepted command together with its admission failure.
///
/// No action was queued. Use [`Self::into_command`] to retain the original
/// value for a later retry without cloning its policy or captured tab or group.
/// Retrying preserves every capture's original session and version.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{reason}")]
pub struct NativeCommandRequestError {
    #[source]
    reason: NativeActionRequestError,
    command: NativeCommand,
}

impl NativeCommandRequestError {
    pub(crate) const fn new(reason: NativeActionRequestError, command: NativeCommand) -> Self {
        Self { reason, command }
    }

    /// Returns why the command was not accepted.
    #[must_use]
    pub const fn reason(&self) -> NativeActionRequestError {
        self.reason
    }

    /// Borrows the unchanged command that was not accepted.
    #[must_use]
    pub const fn command(&self) -> &NativeCommand {
        &self.command
    }

    /// Returns ownership of the unchanged command for storage or retry.
    #[must_use]
    pub fn into_command(self) -> NativeCommand {
        self.command
    }
}

/// One application intent for [`NativeDockspaceApp::request_command`](crate::NativeDockspaceApp::request_command).
///
/// Commands are ordinary values that menus and command palettes may
/// retain. Admission prepares them against the current session under its lock;
/// a stored command is not a prepared action or presentation authority.
/// A captured tab or tabs group retains its original session and version when
/// stored or cloned; admission never resolves its former source again.
/// Rejected admission returns this value in [`NativeCommandRequestError`].
/// Retain the returned [`crate::NativeActionReceipt`] to inspect completion.
/// An unread completed receipt does not block later commands. Pane state always
/// stays in the application.
///
/// ```
/// use dockspace::model::ItemId;
/// use egui_dockspace_native::{NativeCommand, NativeCommandRequestError, NativeDockspaceApp};
/// # fn raise<P: egui_dockspace::PaneView + Send + 'static>(app: &NativeDockspaceApp<P>) -> Result<egui_dockspace_native::NativeActionReceipt, NativeCommandRequestError> {
/// let command = NativeCommand::RaiseContained { item: ItemId::new(1) };
/// let receipt = app.request_command(command)?;
/// // Poll receipt.try_recv() on a later application update.
/// # Ok(receipt)
/// # }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum NativeCommand {
    /// Activate an open item through every containing Tabs choice and request its focus.
    ///
    /// An eligible native window is explicitly activated, including when its pane
    /// is already active. Native and pane focus still require host observations.
    SelectItem {
        /// Application content to activate, including a standalone Pane.
        item: ItemId,
    },
    /// Show or hide an open pane while retaining its layout and native ownership.
    ///
    /// Showing a pane does not activate its native window or request pane focus.
    /// Submit [`Self::SelectItem`] separately when activation is intended.
    SetItemVisibility {
        /// Application-owned pane identity.
        item: ItemId,
        /// Whether the pane participates in the visible layout.
        visible: bool,
    },
    /// Open a closed pane; an already-open pane keeps its location and selection.
    OpenItem {
        /// Application-owned pane identity.
        item: ItemId,
        /// Placement for newly opened content.
        placement: DockPlacement,
    },
    /// Move existing content to a dock placement, contained float or native window.
    ///
    /// Tab and tabs-group captures preserve their original session and version.
    /// A root source keeps the complete Pane/Tabs/Split/Grid payload. Rehoming
    /// preserves its root identity; docking into an existing layout may absorb
    /// it into that destination root. Native ownership changes wait for real first-live
    /// presentation; contained moves use current core geometry.
    Move {
        /// Exact semantic payload, including any captured authority.
        source: DockMoveSource,
        /// Destination and whole-group insertion semantics.
        target: DockMoveTarget,
    },
    /// Recover a complete contained or native root to its core-derived destination.
    DockRootBack {
        /// Group whose recovery target the core revalidates.
        root: RootId,
    },
    /// Raise the contained window owning this pane without selecting the pane.
    RaiseContained {
        /// Any pane in the contained group.
        item: ItemId,
    },
    /// Clamp a contained window into current visible bounds without raising it.
    ///
    /// This explicit command changes durable geometry. Passive resizing does not
    /// perform this recovery; current surface measurement must be available.
    BringContainedIntoView {
        /// Any pane in the contained group.
        item: ItemId,
    },
    /// Request a pane close without approving the application's decision.
    CloseItem {
        /// Pane for the core-owned close plan.
        item: ItemId,
    },
    /// Request an atomic close plan for one captured tab and all its real descendants.
    ///
    /// Hidden and inactive descendants participate in the core's approval workflow.
    /// The application must answer the returned decision tokens before content closes.
    CloseTab {
        /// Exact direct child to close, retaining its captured session and version.
        tab: PreparedTab,
    },
    /// Request an atomic close plan for every pane in a complete root.
    CloseRoot {
        /// Complete group for the close plan.
        root: RootId,
    },
    /// Answer an exact initial token from a core-owned close plan.
    DecideClose {
        /// Core-issued close request.
        request: CloseRequestId,
        /// Exact pending item decision.
        token: CloseDecisionToken,
        /// Explicit application decision.
        decision: CloseDecision,
    },
    /// Continue an exact deferred close decision.
    ContinueClose {
        /// Core-issued close request.
        request: CloseRequestId,
        /// Exact deferred continuation.
        token: DeferredCloseToken,
        /// Explicit continuation decision.
        decision: DeferredCloseDecision,
    },
    /// Replace the complete runtime docking policy, including all per-pane rules.
    ///
    /// Admission leaves the published policy unchanged until the host frame
    /// commits. Earlier input keeps its original policy; measurement and encoding
    /// use the replacement after the configuration boundary. The request receipt
    /// reports [`NativeActionStatus::PolicyChanged`](crate::NativeActionStatus::PolicyChanged).
    /// An identical policy reports no change.
    ///
    /// Retain an application-owned copy when later updates should preserve rules.
    /// The boxed value keeps ordinary command values small.
    ///
    /// ```
    /// use dockspace::policy::DockPolicy;
    /// use egui_dockspace_native::NativeCommand;
    ///
    /// let mut policy = DockPolicy::default();
    /// policy.set_allow_native_surfaces(true);
    /// let command = NativeCommand::SetPolicy { policy: Box::new(policy) };
    /// ```
    SetPolicy {
        /// Complete replacement policy, owned by this command until submission.
        policy: Box<DockPolicy>,
    },
    /// Change arrangement locking while preserving selection and content input.
    ///
    /// All other currently committed policy rules remain unchanged.
    SetArrangementLocked {
        /// Requested lock state.
        locked: bool,
    },
}
