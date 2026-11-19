//! Bounded execution with caller-owned, independently completed action receipts.

use dockspace::close::{
    CloseDecision, CloseDecisionToken, CloseRequestId, DeferredCloseDecision, DeferredCloseToken,
};
use dockspace::model::{DockspaceActionRejection, SurfaceId, WorkspaceVersion};
use dockspace::policy::DockPolicy;
#[cfg(feature = "serde")]
use dockspace::runtime::PreparedDockspaceDocumentRestore;
use dockspace::runtime::{
    DockPresentationConfig, DockspaceActionStatus, DockspaceCloseOutcome, DockspaceClosePlan,
    DockspaceCloseRejection, DockspaceCloseRequestRejection, DockspaceCloseResolution,
    DockspaceHostFrame, DockspacePresentationTransition, DockspacePresentationTransitionId,
    DockspacePresentationTransitionResult, DockspaceRuntimeError, DockspaceRuntimeErrorKind,
    DockspaceSubmittedAction, HostFrameReport, HostInputOutcome, NativeSurfaceBinding,
    PreparedCloseRequest, PreparedDockAction, PreparedDockspaceLayoutReset,
};
use egui_dockspace::{DockInputConfig, DockStyle};

use crate::NativeRuntimeErrorKind;

mod execution;
use execution::NativeActionExpectation;
pub(crate) use execution::NativeApplicationActions;

/// Why one native application request could not be queued.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeActionRequestError {
    /// Another accepted command is still executing or awaiting presentation.
    Busy,
    /// The native runtime has entered terminal shutdown.
    Stopped,
    /// This fixed-host facade cannot remove its own logical surface.
    ///
    /// Keep an empty central root or another presentation on this surface.
    /// Native surface replacement and rebinding require a separate lifecycle.
    HostSurfaceRequired {
        /// The native application's fixed logical host surface.
        surface: SurfaceId,
    },
}

impl NativeActionRequestError {
    pub(crate) const fn busy() -> Self {
        Self::Busy
    }

    pub(crate) const fn stopped() -> Self {
        Self::Stopped
    }
}

impl std::fmt::Display for NativeActionRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Busy => "a native dockspace application action is already pending",
            Self::Stopped => "the native dockspace runtime is stopped",
            Self::HostSurfaceRequired { .. } => {
                "keep a central dock area or another group on the application host surface"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for NativeActionRequestError {}

/// Stage at which one accepted native application action was interrupted.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeActionInterruption {
    /// The action never committed one core host-frame boundary.
    NotCommitted,
    /// The core frame committed, but complete application settlement was unavailable.
    CommittedOutcomeUnavailable,
    /// Shutdown drained without the exact presentation transition terminal.
    PresentationUnresolved,
}

/// Terminal native-host failure of one previously accepted application action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeActionFailure {
    kind: NativeRuntimeErrorKind,
    interruption: NativeActionInterruption,
}

impl NativeActionFailure {
    const fn new(kind: NativeRuntimeErrorKind, interruption: NativeActionInterruption) -> Self {
        Self { kind, interruption }
    }

    /// Returns the stable native runtime failure category.
    #[must_use]
    pub const fn kind(self) -> NativeRuntimeErrorKind {
        self.kind
    }

    /// Returns the exact action stage interrupted by the native failure.
    #[must_use]
    pub const fn interruption(self) -> NativeActionInterruption {
        self.interruption
    }
}

impl std::fmt::Display for NativeActionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stage = match self.interruption {
            NativeActionInterruption::NotCommitted => "before the action committed",
            NativeActionInterruption::CommittedOutcomeUnavailable => {
                "after its frame committed without complete application settlement"
            }
            NativeActionInterruption::PresentationUnresolved => {
                "while the presentation transition remained unresolved"
            }
        };
        write!(
            formatter,
            "native dockspace action stopped {stage} ({:?})",
            self.kind
        )
    }
}

impl std::error::Error for NativeActionFailure {}

/// Exact committed result of one application command.
///
/// Close requests return the core plan without deciding on the application's
/// behalf. Completed results never occupy the runtime's execution slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeActionStatus {
    /// Current native state rejected one Root continuation before replacement.
    ///
    /// The frame may still commit its earlier ordered input. The command slot
    /// reaches this terminal result without stopping the native runtime.
    RootContinuationRejected {
        /// Exact Root binding frozen when this command was queued.
        predecessor: NativeSurfaceBinding,
        /// Incoming logical Root before presentation identity rebasing.
        incoming_root: SurfaceId,
        /// Stable core category of the preparation rejection.
        kind: DockspaceRuntimeErrorKind,
    },
    /// A docking, selection, or reopening command reached its reported status.
    ///
    /// Native tear-off returns [`DockspaceActionStatus::Requested`] immediately,
    /// releasing execution capacity before the child reaches first-live. Match
    /// its transition identity in the application's notification handler.
    Dock(DockspaceActionStatus),
    /// A content-close request opened or reused one core-owned plan.
    CloseRequested {
        /// Current exact plan, including application decision tokens.
        plan: DockspaceClosePlan,
        /// Whether an unresolved plan already existed for this target.
        reused: bool,
    },
    /// Current policy or topology rejected a content-close request.
    CloseRejected(DockspaceCloseRequestRejection),
    /// One initial or deferred close decision was consumed.
    CloseDecisionProcessed {
        /// Exact resolution, including inert stale or duplicate tokens.
        resolution: DockspaceCloseResolution,
        /// Latest core plan when its retained detail remains available.
        plan: Option<DockspaceClosePlan>,
        /// Atomic close result after the final allow decision.
        application: Option<Result<DockspaceCloseOutcome, DockspaceCloseRejection>>,
        /// Whether this decision changed durable topology.
        changed: bool,
    },
    /// A complete runtime policy committed at its configuration boundary.
    PolicyChanged {
        /// Whether the policy differed from its previous value.
        changed: bool,
        /// Workspace version after the configuration input.
        version: WorkspaceVersion,
    },
    /// The arrangement lock committed while preserving all other policy rules.
    ArrangementLockChanged {
        /// Requested arrangement lock state.
        locked: bool,
        /// Whether the policy differed from its previous value.
        changed: bool,
        /// Workspace version after the configuration input.
        version: WorkspaceVersion,
    },
    /// Renderer style and its represented core settings committed together.
    StyleChanged {
        /// Whether the adapter style differed from its previous value.
        changed: bool,
        /// Whether core geometry or interaction timing changed.
        geometry_changed: bool,
    },
    /// Input bindings committed after all earlier native input was consumed.
    InputChanged {
        /// Whether the bindings differed from their previous value.
        changed: bool,
    },
    /// Shared motion settings and the supplied system preference committed.
    #[cfg(feature = "motion")]
    MotionChanged {
        /// Whether settings or preference differed from their previous values.
        changed: bool,
    },
    /// An application default or preset replaced the logical layout.
    ///
    /// Native bindings and output authority still require their fresh observations.
    LayoutReset {
        /// Candidate version immediately before the terminal replacement.
        previous: WorkspaceVersion,
        /// Workspace version published by the replacement.
        current: WorkspaceVersion,
    },
    /// A prepared document and its durable sidecars published atomically.
    #[cfg(feature = "serde")]
    DocumentRestored {
        /// Candidate version immediately before the terminal replacement.
        previous: WorkspaceVersion,
        /// Workspace version published by the replacement.
        current: WorkspaceVersion,
    },
    /// An application command was prepared from an obsolete workspace version.
    Stale {
        /// Version frozen when the command was queued.
        expected: WorkspaceVersion,
        /// Version accepted by the reducer.
        accepted: WorkspaceVersion,
    },
}

/// The exact committed terminal result of one accepted application request.
pub type NativeActionResult = Result<NativeActionStatus, NativeActionFailure>;

/// Completion receiver owned by the caller of one accepted request.
///
/// The runtime retains only the sending half while this request executes. A
/// completed, unread receipt does not prevent later requests, saving or
/// quiescence. Dropping this receiver does not cancel an accepted request.
/// No async runtime or application-global result registry is required.
#[must_use = "retain the receipt to inspect completion, or explicitly drop it to ignore the result"]
#[derive(Debug)]
pub struct NativeActionReceipt {
    receiver: std::sync::mpsc::Receiver<NativeActionResult>,
}

impl NativeActionReceipt {
    /// Polls this request's exact result without blocking.
    ///
    /// `Empty` means the request is still executing or waiting for real host
    /// observations. `Disconnected` means no further result can arrive: either
    /// this receipt has already been consumed, or the runtime was dropped
    /// without observing a terminal result. Disconnection never proves native
    /// resource cleanup or successful quiescence. A fatal runtime that remains
    /// alive continues its ordinary shutdown and reports the exact result when
    /// available, or an explicit [`NativeActionFailure`].
    ///
    /// # Errors
    ///
    /// Returns [`std::sync::mpsc::TryRecvError`] for pending or disconnected work.
    pub fn try_recv(&self) -> Result<NativeActionResult, std::sync::mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}

/// One affine command retained only while it executes.
#[derive(Debug)]
pub(crate) struct NativeApplicationAction {
    expected: WorkspaceVersion,
    pub(crate) command: NativeApplicationCommand,
}

#[derive(Debug)]
pub(crate) enum NativeApplicationCommand {
    Dock(PreparedDockAction),
    Close(PreparedCloseRequest),
    DecideClose {
        request: CloseRequestId,
        token: CloseDecisionToken,
        decision: CloseDecision,
    },
    ContinueClose {
        request: CloseRequestId,
        token: DeferredCloseToken,
        decision: DeferredCloseDecision,
    },
    Policy {
        policy: Box<DockPolicy>,
        arrangement_lock: Option<bool>,
    },
    Style {
        style: Box<DockStyle>,
        presentation: Box<DockPresentationConfig>,
        changed: bool,
    },
    Input {
        input: DockInputConfig,
        changed: bool,
    },
    #[cfg(feature = "motion")]
    Motion {
        configuration: Box<crate::surface_driver::NativeMotionConfiguration>,
        changed: bool,
    },
    LayoutReset(Box<PreparedDockspaceLayoutReset>),
    LayoutResetRebindingRoot {
        reset: Box<PreparedDockspaceLayoutReset>,
        existing_root: NativeSurfaceBinding,
        incoming_root: SurfaceId,
    },
    #[cfg(feature = "serde")]
    DocumentRestore {
        restore: Box<PreparedDockspaceDocumentRestore>,
        root_surface: SurfaceId,
        root_mappings: Vec<(NativeSurfaceBinding, SurfaceId)>,
    },
}

impl From<PreparedDockAction> for NativeApplicationAction {
    fn from(action: PreparedDockAction) -> Self {
        Self {
            expected: action.expected_version(),
            command: NativeApplicationCommand::Dock(action),
        }
    }
}

impl From<PreparedCloseRequest> for NativeApplicationAction {
    fn from(request: PreparedCloseRequest) -> Self {
        Self {
            expected: request.expected_version(),
            command: NativeApplicationCommand::Close(request),
        }
    }
}

impl NativeApplicationAction {
    pub(crate) const fn new(expected: WorkspaceVersion, command: NativeApplicationCommand) -> Self {
        Self { expected, command }
    }

    pub(crate) const fn expected_version(&self) -> WorkspaceVersion {
        self.expected
    }

    pub(crate) fn submit(
        self,
        frame: &mut DockspaceHostFrame<'_>,
        published: WorkspaceVersion,
    ) -> Result<NativeApplicationSubmission, DockspaceRuntimeError> {
        if self.is_replacement() && self.expected != published {
            return Ok(NativeApplicationSubmission::Terminal(
                NativeActionStatus::Stale {
                    expected: self.expected,
                    accepted: published,
                },
            ));
        }
        let submitted = match self.command {
            NativeApplicationCommand::Dock(action) => match frame.submit_prepared_action(action) {
                Ok(submitted) => submitted,
                Err(error) if error.is_foreign_prepared_action() => {
                    return Ok(NativeApplicationSubmission::Terminal(
                        NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
                            DockspaceActionRejection::Conflict,
                        )),
                    ));
                }
                Err(error) => return Err(error),
            },
            NativeApplicationCommand::Close(request) => {
                let target = request.target();
                match frame.submit_prepared_close_request(request) {
                    Ok(submitted) => submitted,
                    Err(error) if error.is_foreign_prepared_close_request() => {
                        return Ok(NativeApplicationSubmission::Terminal(
                            NativeActionStatus::CloseRejected(
                                DockspaceCloseRequestRejection::Conflict { target },
                            ),
                        ));
                    }
                    Err(error) => return Err(error),
                }
            }
            NativeApplicationCommand::DecideClose {
                request,
                token,
                decision,
            } => frame.resolve_close(request, token, decision)?,
            NativeApplicationCommand::ContinueClose {
                request,
                token,
                decision,
            } => frame.continue_deferred_close(request, token, decision)?,
            NativeApplicationCommand::Policy { policy, .. } => {
                return Ok(NativeApplicationSubmission::Configuration(policy));
            }
            NativeApplicationCommand::Style {
                style,
                presentation,
                changed,
            } => {
                return Ok(NativeApplicationSubmission::Style {
                    style,
                    presentation,
                    changed,
                });
            }
            NativeApplicationCommand::Input { input, changed } => {
                return Ok(NativeApplicationSubmission::InputConfiguration { input, changed });
            }
            #[cfg(feature = "motion")]
            NativeApplicationCommand::Motion {
                configuration,
                changed,
            } => {
                return Ok(NativeApplicationSubmission::Motion {
                    configuration,
                    changed,
                });
            }
            NativeApplicationCommand::LayoutReset(reset) => {
                return Ok(NativeApplicationSubmission::Replacement(
                    NativeWorkspaceReplacement::Layout(reset),
                ));
            }
            NativeApplicationCommand::LayoutResetRebindingRoot {
                reset,
                existing_root,
                incoming_root,
            } => {
                return Ok(NativeApplicationSubmission::Replacement(
                    NativeWorkspaceReplacement::LayoutRebindingRoot {
                        reset,
                        existing_root,
                        incoming_root,
                    },
                ));
            }
            #[cfg(feature = "serde")]
            NativeApplicationCommand::DocumentRestore {
                restore,
                root_surface,
                root_mappings,
            } => {
                return Ok(NativeApplicationSubmission::Replacement(
                    NativeWorkspaceReplacement::Document {
                        restore,
                        root_surface,
                        root_mappings,
                    },
                ));
            }
        };
        Ok(NativeApplicationSubmission::Input(submitted))
    }

    fn expectation(&self) -> NativeActionExpectation {
        match &self.command {
            NativeApplicationCommand::Dock(_) => NativeActionExpectation::Dock,
            NativeApplicationCommand::Close(_) => NativeActionExpectation::Close,
            NativeApplicationCommand::DecideClose { .. }
            | NativeApplicationCommand::ContinueClose { .. } => {
                NativeActionExpectation::CloseDecision
            }
            NativeApplicationCommand::Policy {
                arrangement_lock, ..
            } => NativeActionExpectation::Policy(*arrangement_lock),
            NativeApplicationCommand::Style { .. } => NativeActionExpectation::Style,
            NativeApplicationCommand::Input { .. } => NativeActionExpectation::Input,
            #[cfg(feature = "motion")]
            NativeApplicationCommand::Motion { .. } => NativeActionExpectation::Motion,
            NativeApplicationCommand::LayoutReset(_)
            | NativeApplicationCommand::LayoutResetRebindingRoot { .. } => {
                NativeActionExpectation::LayoutReset
            }
            #[cfg(feature = "serde")]
            NativeApplicationCommand::DocumentRestore { .. } => {
                NativeActionExpectation::DocumentRestore
            }
        }
    }

    const fn is_replacement(&self) -> bool {
        match self.command {
            NativeApplicationCommand::LayoutReset(_)
            | NativeApplicationCommand::LayoutResetRebindingRoot { .. } => true,
            #[cfg(feature = "serde")]
            NativeApplicationCommand::DocumentRestore { .. } => true,
            _ => false,
        }
    }
}

/// Stack-local phase distinction within one rollbackable core frame.
pub(crate) enum NativeApplicationSubmission {
    Input(DockspaceSubmittedAction),
    Configuration(Box<DockPolicy>),
    Style {
        style: Box<DockStyle>,
        presentation: Box<DockPresentationConfig>,
        changed: bool,
    },
    InputConfiguration {
        input: DockInputConfig,
        changed: bool,
    },
    #[cfg(feature = "motion")]
    Motion {
        configuration: Box<crate::surface_driver::NativeMotionConfiguration>,
        changed: bool,
    },
    Replacement(NativeWorkspaceReplacement),
    Terminal(NativeActionStatus),
}

pub(crate) enum NativeWorkspaceReplacement {
    Layout(Box<PreparedDockspaceLayoutReset>),
    LayoutRebindingRoot {
        reset: Box<PreparedDockspaceLayoutReset>,
        existing_root: NativeSurfaceBinding,
        incoming_root: SurfaceId,
    },
    #[cfg(feature = "serde")]
    Document {
        restore: Box<PreparedDockspaceDocumentRestore>,
        root_surface: SurfaceId,
        root_mappings: Vec<(NativeSurfaceBinding, SurfaceId)>,
    },
}

/// Exact correlation within the committed frame; replacement is its unique terminal input.
pub(crate) enum NativeSubmittedApplicationAction {
    Input(DockspaceSubmittedAction),
    Style {
        style: Box<DockStyle>,
        submitted: Option<DockspaceSubmittedAction>,
        changed: bool,
    },
    InputConfiguration {
        input: DockInputConfig,
        changed: bool,
    },
    #[cfg(feature = "motion")]
    Motion {
        configuration: Box<crate::surface_driver::NativeMotionConfiguration>,
        changed: bool,
    },
    LayoutReset,
    #[cfg(feature = "serde")]
    DocumentRestore,
    Terminal(NativeActionStatus),
}

impl NativeSubmittedApplicationAction {
    pub(crate) fn commit_input(&mut self) -> Option<DockInputConfig> {
        let Self::InputConfiguration { input, changed } = self else {
            return None;
        };
        let input = *input;
        *self = Self::Terminal(NativeActionStatus::InputChanged { changed: *changed });
        Some(input)
    }

    /// Uses the candidate style for this update's encoding before publication.
    pub(crate) fn encoding_style(&self) -> Option<&DockStyle> {
        match self {
            Self::Style { style, .. } => Some(style),
            _ => None,
        }
    }

    /// Extracts adapter styling only after the matching core frame committed.
    pub(crate) fn commit_style(
        &mut self,
        report: &HostFrameReport,
    ) -> Result<Option<Box<DockStyle>>, ()> {
        let Self::Style {
            submitted, changed, ..
        } = self
        else {
            return Ok(None);
        };
        let geometry_changed = match submitted {
            Some(submitted) => match report.submitted_action_outcome(*submitted) {
                Some(HostInputOutcome::PresentationConfigReplaced { changed, .. }) => *changed,
                _ => return Err(()),
            },
            None => false,
        };
        let status = NativeActionStatus::StyleChanged {
            changed: *changed,
            geometry_changed,
        };
        let Self::Style { style, .. } = std::mem::replace(self, Self::Terminal(status)) else {
            unreachable!("the style submission was checked before replacement")
        };
        Ok(Some(style))
    }
}

impl NativeApplicationSubmission {
    /// Appends configuration or replacement after all earlier local and focus input.
    pub(crate) fn finish(
        self,
        frame: &mut DockspaceHostFrame<'_>,
    ) -> Result<NativeSubmittedApplicationAction, DockspaceRuntimeError> {
        match self {
            Self::Input(submitted) => Ok(NativeSubmittedApplicationAction::Input(submitted)),
            Self::Configuration(policy) => frame
                .replace_policy(*policy)
                .map(NativeSubmittedApplicationAction::Input),
            Self::Style {
                style,
                presentation,
                changed,
            } => {
                let submitted = if frame.presentation_config() == presentation.as_ref() {
                    None
                } else {
                    Some(frame.replace_presentation_config(*presentation)?)
                };
                Ok(NativeSubmittedApplicationAction::Style {
                    style,
                    submitted,
                    changed,
                })
            }
            Self::InputConfiguration { input, changed } => {
                if changed {
                    // A binding change ends the previous gesture through its
                    // existing semantic cancellation action. This frame keeps
                    // its original bindings while encoding captured raw input.
                    for surface in frame.surfaces() {
                        let cancel = frame
                            .paint_plan(surface)?
                            .and_then(|plan| plan.prepare_gesture_cancel());
                        if let Some(cancel) = cancel {
                            frame.submit_surface_action(cancel)?;
                            break;
                        }
                    }
                }
                Ok(NativeSubmittedApplicationAction::InputConfiguration { input, changed })
            }
            #[cfg(feature = "motion")]
            Self::Motion {
                configuration,
                changed,
            } => Ok(NativeSubmittedApplicationAction::Motion {
                configuration,
                changed,
            }),
            Self::Replacement(NativeWorkspaceReplacement::Layout(reset)) => {
                frame.submit_layout_reset(&reset)?;
                Ok(NativeSubmittedApplicationAction::LayoutReset)
            }
            Self::Replacement(NativeWorkspaceReplacement::LayoutRebindingRoot {
                reset,
                existing_root,
                incoming_root,
            }) => {
                if let Err(error) =
                    frame.submit_native_layout_reset(&reset, existing_root, incoming_root)
                {
                    return root_continuation_rejection(error, existing_root, incoming_root);
                }
                Ok(NativeSubmittedApplicationAction::LayoutReset)
            }
            #[cfg(feature = "serde")]
            Self::Replacement(NativeWorkspaceReplacement::Document {
                restore,
                root_surface,
                root_mappings,
            }) => {
                if let Some(&(existing_root, incoming_root)) = root_mappings.first() {
                    if let Err(error) =
                        frame.submit_native_document_restore_with_roots(&restore, root_mappings)
                    {
                        return root_continuation_rejection(error, existing_root, incoming_root);
                    }
                } else {
                    frame.submit_native_document_restore(&restore, root_surface)?;
                }
                Ok(NativeSubmittedApplicationAction::DocumentRestore)
            }
            Self::Terminal(status) => Ok(NativeSubmittedApplicationAction::Terminal(status)),
        }
    }
}

fn root_continuation_rejection(
    error: DockspaceRuntimeError,
    predecessor: NativeSurfaceBinding,
    incoming_root: SurfaceId,
) -> Result<NativeSubmittedApplicationAction, DockspaceRuntimeError> {
    // Preparation rejects before append. Protocol and publication failures still
    // propagate so a poisoned candidate can never be committed as a rejection.
    if matches!(
        error.kind(),
        DockspaceRuntimeErrorKind::OperationConflict
            | DockspaceRuntimeErrorKind::InvalidConfiguration
    ) {
        Ok(NativeSubmittedApplicationAction::Terminal(
            NativeActionStatus::RootContinuationRejected {
                predecessor,
                incoming_root,
                kind: error.kind(),
            },
        ))
    } else {
        Err(error)
    }
}
