//! Framework-side custody of ordered native dispatches and observations.

use std::collections::VecDeque;

use super::{
    NativeCloseEffectAcknowledgement, NativeEffectDispatch, NativeEffectOperation,
    NativeEffectRequest, NativeEffectResult, NativeIndeterminateReason,
};
use crate::runtime::{
    NativeCloseDisposition, NativeRootContinuation, NativeWindowBinding, NativeWindowFacts,
};

/// Holds queued core effects and backend tickets for independently progressing windows.
///
/// An in-flight dispatch blocks subsequent work for its own window. A terminal
/// close or observation of earlier cleanup may proceed even while a ticket is pending.
#[derive(Debug)]
pub struct NativeEffectOutbox<Ticket> {
    queued: VecDeque<NativeEffectRequest>,
    pending: Vec<(NativeEffectDispatch, Ticket)>,
}

impl<Ticket> Default for NativeEffectOutbox<Ticket> {
    fn default() -> Self {
        Self {
            queued: VecDeque::new(),
            pending: Vec::new(),
        }
    }
}

impl<Ticket> NativeEffectOutbox<Ticket> {
    /// Appends a batch in the order emitted by the core.
    pub fn enqueue(&mut self, effects: impl IntoIterator<Item = NativeEffectRequest>) {
        self.queued.extend(effects);
    }

    /// Takes the earliest request whose window is ready, or a terminal cleanup.
    pub fn take_next(&mut self) -> Option<NativeEffectRequest> {
        let index = self.queued.iter().position(|request| {
            let operation = request.operation();
            is_destruction(operation)
                || matches!(operation, NativeEffectOperation::AwaitCleanup { .. })
                || self.pending_index(operation.binding().window()).is_none()
        })?;
        self.queued.remove(index)
    }

    /// Returns the queued requests in dispatch order.
    pub fn queued(&self) -> impl ExactSizeIterator<Item = &NativeEffectRequest> {
        self.queued.iter()
    }

    /// Returns whether there is any queued request or in-flight dispatch.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queued.is_empty() && self.pending.is_empty()
    }

    /// Iterates over each window and its outstanding backend ticket, in dispatch order.
    pub fn pending(&self) -> impl ExactSizeIterator<Item = (NativeWindowBinding, &Ticket)> {
        self.pending
            .iter()
            .map(|(dispatch, ticket)| (dispatch.operation().binding().window(), ticket))
    }

    /// Retains a started request until its corresponding backend observation.
    ///
    /// # Errors
    ///
    /// Returns both values if the same window already has a dispatch in flight.
    pub fn await_observation(
        &mut self,
        dispatch: NativeEffectDispatch,
        ticket: Ticket,
    ) -> Result<(), (NativeEffectDispatch, Ticket)> {
        if self
            .pending_index(dispatch.operation().binding().window())
            .is_some()
        {
            return Err((dispatch, ticket));
        }
        self.pending.push((dispatch, ticket));
        Ok(())
    }

    /// Claims a window's dispatch once its original ticket has settled.
    pub fn take_pending(
        &mut self,
        window: NativeWindowBinding,
    ) -> Option<(NativeEffectDispatch, Ticket)> {
        let index = self.pending_index(window)?;
        Some(self.pending.remove(index))
    }

    fn pending_index(&self, window: NativeWindowBinding) -> Option<usize> {
        self.pending()
            .position(|(owner, _)| owner.same_window_lifetime(window))
    }

    /// Settles an in-flight dispatch whose exact native window has retired.
    ///
    /// The backend may have applied the operation before retiring. Report
    /// the indeterminate result into the same FIFO as other observations.
    pub fn retire_pending(&mut self, window: NativeWindowBinding) -> Option<NativeEffectResult> {
        let (dispatch, _) = self.take_pending(window)?;
        Some(dispatch.indeterminate(NativeIndeterminateReason::AcknowledgementLost))
    }

    /// Settles the original dispatch when a committed root continuation replaces its binding.
    pub fn retire_continued_window(
        &mut self,
        continuation: NativeRootContinuation,
    ) -> Option<NativeEffectResult> {
        self.retire_pending(continuation.predecessor().window())
    }
}

/// Deduplicates property snapshots while preserving one-shot effect receipts.
#[derive(Debug)]
pub struct NativeWindowObservationOutbox {
    binding: NativeWindowBinding,
    live: Option<NativeWindowFacts>,
    close: Option<NativeEffectDispatch>,
}

impl NativeWindowObservationOutbox {
    /// Creates an outbox for one exact window observation binding.
    #[must_use]
    pub const fn new(binding: NativeWindowBinding) -> Self {
        Self {
            binding,
            live: None,
            close: None,
        }
    }

    /// Starts a new observation lifetime and abandons any unconfirmed close of the old one.
    pub fn reset_for_binding(&mut self, binding: NativeWindowBinding) {
        if self.binding != binding {
            *self = Self::new(binding);
        }
    }

    /// Returns the last accepted live properties, with any receipts removed.
    #[must_use]
    pub const fn live(&self) -> Option<NativeWindowFacts> {
        self.live
    }

    /// Prepares changed live properties or an exact receipt for submission.
    ///
    /// Feed destruction through [`Self::destroyed`]. Confirm the exact returned
    /// facts with [`Self::confirm_observation`] after core accepts the snapshot.
    #[must_use]
    pub fn observe(&self, facts: NativeWindowFacts) -> Option<NativeWindowFacts> {
        if facts.is_destroyed() {
            return None;
        }
        let properties = facts.without_effect_acknowledgements();
        (facts != properties || self.live != Some(properties)).then_some(facts)
    }

    /// Confirms the exact observation after the snapshot was accepted into core ingress.
    pub fn confirm_observation(&mut self, binding: NativeWindowBinding, facts: NativeWindowFacts) {
        if binding != self.binding {
            return;
        }
        if facts.is_destroyed() {
            self.live = None;
            if facts == self.destroyed()
                && let Some(dispatch) = self.take_pending_close()
            {
                let _ = dispatch.accepted();
            }
        } else {
            self.live = Some(facts.without_effect_acknowledgements());
        }
    }

    /// Holds the original close dispatch until physical destruction proves it.
    ///
    /// # Errors
    ///
    /// Returns the dispatch if it belongs to another binding, does not request
    /// destruction, or an earlier close already owns destruction.
    pub fn await_destruction(
        &mut self,
        dispatch: NativeEffectDispatch,
    ) -> Result<(), NativeEffectDispatch> {
        if self.close.is_some()
            || dispatch.operation().binding().window() != self.binding
            || !is_destruction(dispatch.operation())
        {
            return Err(dispatch);
        }
        self.close = Some(dispatch);
        Ok(())
    }

    /// Returns whether a close receipt is awaiting destruction.
    #[must_use]
    pub const fn has_pending_close(&self) -> bool {
        self.close.is_some()
    }

    /// Releases custody of a failed or cancelled close for explicit settlement.
    pub fn take_pending_close(&mut self) -> Option<NativeEffectDispatch> {
        self.close.take()
    }

    /// Prepares exact destruction facts without releasing the retained dispatch.
    #[must_use]
    pub fn destroyed(&self) -> NativeWindowFacts {
        self.close
            .as_ref()
            .map_or_else(NativeWindowFacts::destroyed, |dispatch| {
                NativeWindowFacts::destroyed_after(NativeCloseEffectAcknowledgement {
                    provider: dispatch.request.provider,
                    binding: dispatch.request.binding,
                    effect: dispatch.request.effect,
                })
            })
    }
}

fn is_destruction(operation: &NativeEffectOperation) -> bool {
    matches!(
        operation,
        NativeEffectOperation::ReleaseChild { .. }
            | NativeEffectOperation::CompensatingClose { .. }
            | NativeEffectOperation::ResolveNativeClose {
                resolution: NativeCloseDisposition::Accept,
                ..
            }
    )
}
