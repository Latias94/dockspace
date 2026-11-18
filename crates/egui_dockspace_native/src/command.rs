//! Application intents prepared by the native runtime at admission.

use dockspace::close::{
    CloseDecision, CloseDecisionToken, CloseRequestId, DeferredCloseDecision, DeferredCloseToken,
};
use dockspace::model::{
    DockPlacement, ItemId, NativeTearOffSource, NativeWindowPlacement, PreparedTab,
    PreparedTabsGroup, RootId, SurfaceId, TabGroupDockMode,
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
/// Consume [`NativeDockspaceApp::take_action_status`](crate::NativeDockspaceApp::take_action_status)
/// before submitting another command. Pane state always stays in the application.
///
/// ```
/// use dockspace::model::ItemId;
/// use egui_dockspace_native::{NativeCommand, NativeCommandRequestError, NativeDockspaceApp};
/// # fn raise<P: egui_dockspace::PaneView + Send + 'static>(app: &NativeDockspaceApp<P>) -> Result<(), NativeCommandRequestError> {
/// let command = NativeCommand::RaiseContained { item: ItemId::new(1) };
/// app.request_command(command)?;
/// // Consume take_action_status on a later application update before submitting more.
/// # Ok(())
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
    /// Move one open pane to an explicit docking placement.
    DockItem {
        /// Pane to move.
        item: ItemId,
        /// Destination in the current workspace.
        placement: DockPlacement,
    },
    /// Move one captured direct tab to an explicit docking placement.
    ///
    /// A layout tab carries its complete Pane/Tabs/Split/Grid subtree, including
    /// hidden and inactive content. Its original session and version are retained.
    DockTab {
        /// Exact direct child to move.
        tab: PreparedTab,
        /// Destination in the captured workspace version.
        placement: DockPlacement,
    },
    /// Move one previously captured tabs group as an atomic payload.
    ///
    /// The capture retains its original session and version. Admission never
    /// refreshes its source from an item that may have moved to another group.
    DockTabs {
        /// Exact complete group to move.
        group: PreparedTabsGroup,
        /// Destination in the captured workspace version.
        placement: DockPlacement,
        /// Center insertion semantics for the captured group.
        ///
        /// Edge placements keep their structural behavior. A center
        /// placement can either merge the group's direct tabs or preserve the
        /// complete group as one layout tab, matching the pointer gesture's
        /// modifier policy.
        mode: TabGroupDockMode,
    },
    /// Dock an entire root as one group.
    DockRoot {
        /// Complete group to move.
        root: RootId,
        /// Destination in the current workspace.
        placement: DockPlacement,
    },
    /// Recover a complete contained or native root to its core-derived destination.
    DockRootBack {
        /// Group whose recovery target the core revalidates.
        root: RootId,
    },
    /// Float one pane inside an existing logical surface, using current geometry.
    ///
    /// This creates contained content, not an OS window. A complete singleton
    /// preserves its root identity; part of a larger group receives a new root.
    FloatItem {
        /// Pane to float.
        item: ItemId,
        /// Existing surface that will contain the floating window.
        surface: SurfaceId,
    },
    /// Float one captured direct tab inside an existing logical surface.
    ///
    /// A layout tab preserves its complete subtree. The core derives placement
    /// and retains native source ownership until the exact target presentation.
    FloatTab {
        /// Exact direct child, retaining its captured session and version.
        tab: PreparedTab,
        /// Existing surface that will contain the floating content.
        surface: SurfaceId,
    },
    /// Float one previously captured tabs group inside an existing surface.
    ///
    /// The complete group moves once, preserving its order and selection. A
    /// group that forms the complete source root preserves that root's identity.
    /// Native source ownership remains until the target's exact first presentation.
    FloatTabs {
        /// Exact complete group to float, retaining its captured session and version.
        group: PreparedTabsGroup,
        /// Existing surface that will contain the floating group.
        surface: SurfaceId,
    },
    /// Float a complete Pane/Tabs/Split/Grid layout inside an existing surface.
    ///
    /// Preserves root identity, layout titles, track shares, central Tabs and selections.
    /// This creates contained content, not an OS window. Native source ownership
    /// remains until the target's exact presentation permits the existing transfer.
    FloatRoot {
        /// Complete layout to float as one unit.
        root: RootId,
        /// Existing surface that will contain the floating layout.
        surface: SurfaceId,
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
    /// Begin moving an item, captured tab, tabs group or complete root to a managed OS window.
    ///
    /// Source ownership remains until the child's exact first-live presentation.
    MoveToNewWindow {
        /// Content to transfer; captured tabs and groups retain their original authority.
        source: NativeTearOffSource,
        /// Requested desktop-physical outer bounds.
        placement: NativeWindowPlacement,
    },
    /// Begin native tear-off using core-derived placement from current platform facts.
    ///
    /// Missing source geometry or work-area facts produces a core-owned action
    /// rejection. The command never invents a desktop origin or moves content early.
    MoveToNewWindowDefault {
        /// Content to transfer; captured tabs and groups retain their original authority.
        source: NativeTearOffSource,
    },
    /// Begin moving a complete root to a newly managed OS window.
    ///
    /// Equivalent to [`Self::MoveToNewWindow`] with [`NativeTearOffSource::Root`].
    MoveRootToNewWindow {
        /// Complete group to promote.
        root: RootId,
        /// Requested desktop-physical outer bounds.
        placement: NativeWindowPlacement,
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
    /// use the replacement after the configuration boundary. Consume
    /// [`NativeActionStatus::PolicyChanged`](crate::NativeActionStatus::PolicyChanged)
    /// before submitting another command. An identical policy reports no change.
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
