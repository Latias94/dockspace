//! The in-flight request owns completion until semantic and presentation settlement.

use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum NativeActionExpectation {
    Dock,
    Close,
    CloseDecision,
    Policy(Option<bool>),
    Style,
    Input,
    #[cfg(feature = "motion")]
    Motion,
    LayoutReset,
    #[cfg(feature = "serde")]
    DocumentRestore,
}

impl NativeActionExpectation {
    const fn is_replacement(self) -> bool {
        match self {
            Self::LayoutReset => true,
            #[cfg(feature = "serde")]
            Self::DocumentRestore => true,
            _ => false,
        }
    }
}

/// One bounded in-flight request, advanced by service or an owned paint update.
#[derive(Debug, Default)]
pub(crate) struct NativeApplicationActions {
    state: NativeApplicationActionState,
}

#[derive(Debug)]
enum NativeApplicationActionState {
    Vacant,
    Queued {
        action: NativeApplicationAction,
        completion: std::sync::mpsc::Sender<NativeActionResult>,
    },
    UncommittedFrame {
        completion: std::sync::mpsc::Sender<NativeActionResult>,
        expected: WorkspaceVersion,
        kind: NativeActionExpectation,
    },
    CommittedFrame {
        completion: std::sync::mpsc::Sender<NativeActionResult>,
        expected: WorkspaceVersion,
        kind: NativeActionExpectation,
    },
    AwaitingPresentation {
        completion: std::sync::mpsc::Sender<NativeActionResult>,
        transition: DockspacePresentationTransitionId,
        shutdown_kind: Option<NativeRuntimeErrorKind>,
    },
}

impl Default for NativeApplicationActionState {
    fn default() -> Self {
        Self::Vacant
    }
}

impl NativeApplicationActions {
    pub(crate) fn enqueue<T>(
        &mut self,
        command: T,
        prepare: impl FnOnce(T) -> NativeApplicationAction,
    ) -> Result<NativeActionReceipt, T> {
        if self.is_occupied() {
            return Err(command);
        }
        let (completion, receiver) = std::sync::mpsc::channel();
        self.state = NativeApplicationActionState::Queued {
            action: prepare(command),
            completion,
        };
        Ok(NativeActionReceipt { receiver })
    }

    pub(crate) const fn is_occupied(&self) -> bool {
        !matches!(self.state, NativeApplicationActionState::Vacant)
    }

    pub(crate) const fn has_queued(&self) -> bool {
        matches!(self.state, NativeApplicationActionState::Queued { .. })
    }

    pub(crate) const fn has_queued_replacement(&self) -> bool {
        matches!(
            &self.state,
            NativeApplicationActionState::Queued { action, .. } if action.is_replacement()
        )
    }

    pub(crate) const fn has_pending_replacement(&self) -> bool {
        self.has_queued_replacement()
            || matches!(
                self.state,
                NativeApplicationActionState::UncommittedFrame { kind, .. }
                    | NativeApplicationActionState::CommittedFrame { kind, .. }
                    if kind.is_replacement()
            )
    }

    /// A terminal replacement waits for frozen placement geometry to commit.
    pub(crate) fn take_after_placements(
        &mut self,
        placements_unsettled: bool,
    ) -> Option<NativeApplicationAction> {
        if placements_unsettled && self.has_queued_replacement() {
            return None;
        }
        self.take()
    }

    pub(crate) fn take(&mut self) -> Option<NativeApplicationAction> {
        let state = std::mem::take(&mut self.state);
        let NativeApplicationActionState::Queued { action, completion } = state else {
            self.state = state;
            return None;
        };
        self.state = NativeApplicationActionState::UncommittedFrame {
            completion,
            expected: action.expected_version(),
            kind: action.expectation(),
        };
        Some(action)
    }

    pub(crate) fn mark_committed(&mut self) {
        if !matches!(
            self.state,
            NativeApplicationActionState::UncommittedFrame { .. }
        ) {
            return;
        }
        let NativeApplicationActionState::UncommittedFrame {
            completion,
            expected,
            kind,
        } = std::mem::take(&mut self.state)
        else {
            unreachable!("the submitted frame was checked before advancing its phase")
        };
        self.state = NativeApplicationActionState::CommittedFrame {
            completion,
            expected,
            kind,
        };
    }

    pub(crate) fn settle_submission(
        &mut self,
        submitted: Option<NativeSubmittedApplicationAction>,
        report: &HostFrameReport,
    ) -> Result<bool, ()> {
        self.mark_committed();
        let input = match submitted {
            Some(NativeSubmittedApplicationAction::Input(submitted)) => {
                report.submitted_action_outcome(submitted)
            }
            Some(
                NativeSubmittedApplicationAction::Style { .. }
                | NativeSubmittedApplicationAction::InputConfiguration { .. },
            ) => return Err(()),
            #[cfg(feature = "motion")]
            Some(NativeSubmittedApplicationAction::Motion { .. }) => return Err(()),
            Some(NativeSubmittedApplicationAction::LayoutReset) => report
                .inputs()
                .iter()
                .find(|input| matches!(input, HostInputOutcome::LayoutReset { .. })),
            #[cfg(feature = "serde")]
            Some(NativeSubmittedApplicationAction::DocumentRestore) => report
                .inputs()
                .iter()
                .find(|input| matches!(input, HostInputOutcome::DocumentRestored { .. })),
            Some(NativeSubmittedApplicationAction::Terminal(status)) => {
                let NativeApplicationActionState::CommittedFrame { expected, kind, .. } =
                    &self.state
                else {
                    return Err(());
                };
                let valid = match status {
                    NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
                        DockspaceActionRejection::Conflict,
                    )) => matches!(kind, NativeActionExpectation::Dock),
                    NativeActionStatus::CloseRejected(
                        DockspaceCloseRequestRejection::Conflict { .. },
                    ) => matches!(kind, NativeActionExpectation::Close),
                    NativeActionStatus::Stale {
                        expected: stale, ..
                    } => *expected == stale && kind.is_replacement(),
                    NativeActionStatus::RootContinuationRejected { .. } => kind.is_replacement(),
                    NativeActionStatus::StyleChanged { .. } => {
                        matches!(kind, NativeActionExpectation::Style)
                    }
                    NativeActionStatus::InputChanged { .. } => {
                        matches!(kind, NativeActionExpectation::Input)
                    }
                    #[cfg(feature = "motion")]
                    NativeActionStatus::MotionChanged { .. } => {
                        matches!(kind, NativeActionExpectation::Motion)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(());
                }
                self.complete(Ok(status));
                return Ok(true);
            }
            None => None,
        };
        self.settle(input, report.presentation_transitions())
    }

    pub(crate) fn settle(
        &mut self,
        input: Option<&HostInputOutcome>,
        transitions: &[DockspacePresentationTransition],
    ) -> Result<bool, ()> {
        match &self.state {
            NativeApplicationActionState::Vacant | NativeApplicationActionState::Queued { .. } => {
                Ok(false)
            }
            NativeApplicationActionState::UncommittedFrame { expected, kind, .. }
            | NativeApplicationActionState::CommittedFrame { expected, kind, .. } => {
                let Some(input) = input else {
                    self.set_terminal_failure(
                        NativeRuntimeErrorKind::Internal,
                        NativeActionInterruption::CommittedOutcomeUnavailable,
                    );
                    return Err(());
                };
                let settlement = match (kind, input) {
                    (
                        NativeActionExpectation::Dock,
                        HostInputOutcome::ProductActionApplied(outcome),
                    ) => ReductionSettlement::Terminal(DockspaceActionStatus::Applied(
                        outcome.clone(),
                    )),
                    (
                        NativeActionExpectation::Dock,
                        HostInputOutcome::ProductPresentationActionRequested {
                            outcome: outcome @ dockspace::model::DockspaceActionOutcome::NativeRootTearOffRequested { .. },
                            transition,
                        },
                    ) => ReductionSettlement::Terminal(DockspaceActionStatus::Requested {
                        outcome: outcome.clone(),
                        transition: *transition,
                    }),
                    (
                        NativeActionExpectation::Dock,
                        HostInputOutcome::ProductPresentationActionRequested { transition, .. },
                    ) => ReductionSettlement::Presentation {
                        transition: *transition,
                    },
                    (
                        NativeActionExpectation::Dock,
                        HostInputOutcome::ProductActionRejected(reason),
                    ) => ReductionSettlement::Terminal(DockspaceActionStatus::Rejected(*reason)),
                    (
                        _,
                        HostInputOutcome::StaleRejected {
                            expected: stale,
                            accepted,
                        },
                    ) if *stale == *expected => {
                        let status = if matches!(kind, NativeActionExpectation::Dock) {
                            NativeActionStatus::Dock(DockspaceActionStatus::Stale {
                                expected: *stale,
                                accepted: *accepted,
                            })
                        } else {
                            NativeActionStatus::Stale {
                                expected: *stale,
                                accepted: *accepted,
                            }
                        };
                        ReductionSettlement::ApplicationTerminal(status)
                    }
                    (
                        NativeActionExpectation::Close,
                        HostInputOutcome::CloseRequested { plan, reused, .. },
                    ) => ReductionSettlement::ApplicationTerminal(
                        NativeActionStatus::CloseRequested {
                            plan: plan.clone(),
                            reused: *reused,
                        },
                    ),
                    (NativeActionExpectation::Close, HostInputOutcome::CloseRejected(reason)) => {
                        ReductionSettlement::ApplicationTerminal(NativeActionStatus::CloseRejected(
                            *reason,
                        ))
                    }
                    (
                        NativeActionExpectation::CloseDecision,
                        HostInputOutcome::CloseDecisionProcessed {
                            resolution,
                            plan,
                            application,
                            changed,
                        },
                    ) => ReductionSettlement::ApplicationTerminal(
                        NativeActionStatus::CloseDecisionProcessed {
                            resolution: *resolution,
                            plan: plan.clone(),
                            application: application.clone(),
                            changed: *changed,
                        },
                    ),
                    (
                        NativeActionExpectation::Policy(arrangement_lock),
                        HostInputOutcome::PolicyReplaced { changed, version },
                    ) => ReductionSettlement::ApplicationTerminal(match arrangement_lock {
                        Some(locked) => NativeActionStatus::ArrangementLockChanged {
                            locked: *locked,
                            changed: *changed,
                            version: *version,
                        },
                        None => NativeActionStatus::PolicyChanged {
                            changed: *changed,
                            version: *version,
                        },
                    }),
                    (
                        NativeActionExpectation::LayoutReset,
                        HostInputOutcome::LayoutReset { previous, current },
                    ) => {
                        ReductionSettlement::ApplicationTerminal(NativeActionStatus::LayoutReset {
                            previous: *previous,
                            current: *current,
                        })
                    }
                    #[cfg(feature = "serde")]
                    (
                        NativeActionExpectation::DocumentRestore,
                        HostInputOutcome::DocumentRestored { previous, current },
                    ) => ReductionSettlement::ApplicationTerminal(
                        NativeActionStatus::DocumentRestored {
                            previous: *previous,
                            current: *current,
                        },
                    ),
                    _ => {
                        self.set_terminal_failure(
                            NativeRuntimeErrorKind::Internal,
                            NativeActionInterruption::CommittedOutcomeUnavailable,
                        );
                        return Err(());
                    }
                };
                match settlement {
                    ReductionSettlement::Terminal(status) => {
                        self.complete(Ok(NativeActionStatus::Dock(status)));
                        Ok(true)
                    }
                    ReductionSettlement::ApplicationTerminal(status) => {
                        self.complete(Ok(status));
                        Ok(true)
                    }
                    ReductionSettlement::Presentation { transition } => {
                        let (NativeApplicationActionState::UncommittedFrame { completion, .. }
                        | NativeApplicationActionState::CommittedFrame { completion, .. }) =
                            std::mem::take(&mut self.state)
                        else {
                            unreachable!("presentation follows the exact submitted action")
                        };
                        self.state = NativeApplicationActionState::AwaitingPresentation {
                            completion,
                            transition,
                            shutdown_kind: None,
                        };
                        self.settle_presentation(transitions).map(|_| true)
                    }
                }
            }
            NativeApplicationActionState::AwaitingPresentation { .. } => {
                self.settle_presentation(transitions)
            }
        }
    }

    pub(crate) fn settle_presentation_transitions(
        &mut self,
        transitions: &[DockspacePresentationTransition],
    ) -> Result<bool, ()> {
        if !matches!(
            self.state,
            NativeApplicationActionState::AwaitingPresentation { .. }
        ) {
            return Ok(false);
        }
        self.settle_presentation(transitions)
    }

    fn settle_presentation(
        &mut self,
        transitions: &[DockspacePresentationTransition],
    ) -> Result<bool, ()> {
        let NativeApplicationActionState::AwaitingPresentation {
            transition: requested_transition,
            ..
        } = &self.state
        else {
            return Ok(false);
        };
        // Core validates the frozen command and its completed payload. The host
        // correlates only the opaque session-scoped request identity.
        let Some(transition) = transitions
            .iter()
            .find(|transition| transition.id() == *requested_transition)
        else {
            return Ok(false);
        };
        let status = if transition.result() == DockspacePresentationTransitionResult::Applied {
            let Some(outcome) = transition.outcome().cloned() else {
                self.set_terminal_failure(
                    NativeRuntimeErrorKind::Internal,
                    NativeActionInterruption::CommittedOutcomeUnavailable,
                );
                return Err(());
            };
            DockspaceActionStatus::Applied(outcome)
        } else {
            DockspaceActionStatus::PresentationFailed {
                root: transition.root(),
                source_surface: transition.source_surface(),
                target_surface: transition.target_surface(),
                reason: transition.result(),
            }
        };
        self.complete(Ok(NativeActionStatus::Dock(status)));
        Ok(true)
    }

    fn complete(&mut self, result: NativeActionResult) {
        let state = std::mem::take(&mut self.state);
        let completion = match state {
            NativeApplicationActionState::Queued { completion, .. }
            | NativeApplicationActionState::UncommittedFrame { completion, .. }
            | NativeApplicationActionState::CommittedFrame { completion, .. }
            | NativeApplicationActionState::AwaitingPresentation { completion, .. } => completion,
            NativeApplicationActionState::Vacant => return,
        };
        let _ = completion.send(result);
    }

    pub(crate) fn begin_shutdown(&mut self, kind: NativeRuntimeErrorKind) -> bool {
        if let NativeApplicationActionState::AwaitingPresentation { shutdown_kind, .. } =
            &mut self.state
        {
            if shutdown_kind.is_none() {
                *shutdown_kind = Some(kind);
                return true;
            }
            return false;
        }
        match self.state {
            NativeApplicationActionState::Queued { .. }
            | NativeApplicationActionState::UncommittedFrame { .. } => {
                self.set_terminal_failure(kind, NativeActionInterruption::NotCommitted);
                true
            }
            NativeApplicationActionState::CommittedFrame { .. } => {
                self.set_terminal_failure(
                    kind,
                    NativeActionInterruption::CommittedOutcomeUnavailable,
                );
                true
            }
            NativeApplicationActionState::AwaitingPresentation { .. } => unreachable!(
                "awaiting presentation shutdown state is handled before terminalization"
            ),
            NativeApplicationActionState::Vacant => false,
        }
    }

    pub(crate) fn finish_shutdown_exhausted(&mut self) -> bool {
        let kind = match &self.state {
            NativeApplicationActionState::AwaitingPresentation {
                shutdown_kind: Some(kind),
                ..
            } => *kind,
            _ => return false,
        };
        self.set_terminal_failure(kind, NativeActionInterruption::PresentationUnresolved);
        true
    }

    fn set_terminal_failure(
        &mut self,
        kind: NativeRuntimeErrorKind,
        interruption: NativeActionInterruption,
    ) {
        self.complete(Err(NativeActionFailure::new(kind, interruption)));
    }
}

enum ReductionSettlement {
    Terminal(DockspaceActionStatus),
    ApplicationTerminal(NativeActionStatus),
    Presentation {
        transition: DockspacePresentationTransitionId,
    },
}
