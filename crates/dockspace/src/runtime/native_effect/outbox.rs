//! Framework-side custody of ordered native dispatches and observations.

use std::collections::VecDeque;

use super::{
    NativeCloseEffectAcknowledgement, NativeEffectDispatch, NativeEffectRequest,
    NativeEffectResult, NativeIndeterminateReason,
};
use crate::runtime::{NativeWindowBinding, NativeWindowFacts};

/// Holds the core effect FIFO and the single dispatch temporarily owned by a backend ticket.
///
/// The ticket carries framework-specific callback custody. While it is pending,
/// later requests stay queued in their original order.
#[derive(Debug)]
pub struct NativeEffectOutbox<Ticket> {
    queued: VecDeque<NativeEffectRequest>,
    pending: Option<(NativeEffectDispatch, Ticket)>,
}

impl<Ticket> Default for NativeEffectOutbox<Ticket> {
    fn default() -> Self {
        Self {
            queued: VecDeque::new(),
            pending: None,
        }
    }
}

impl<Ticket> NativeEffectOutbox<Ticket> {
    /// Appends a batch in the order emitted by the core.
    pub fn enqueue(&mut self, effects: impl IntoIterator<Item = NativeEffectRequest>) {
        self.queued.extend(effects);
    }

    /// Takes the next request once the previous deferred dispatch has settled.
    pub fn take_next(&mut self) -> Option<NativeEffectRequest> {
        if self.pending.is_some() {
            return None;
        }
        self.queued.pop_front()
    }

    /// Returns the queued requests in dispatch order.
    pub fn queued(&self) -> impl ExactSizeIterator<Item = &NativeEffectRequest> {
        self.queued.iter()
    }

    /// Returns the exact window and callback custody of the in-flight dispatch.
    pub fn pending(&self) -> Option<(NativeWindowBinding, &Ticket)> {
        let (dispatch, ticket) = self.pending.as_ref()?;
        Some((dispatch.operation().binding().window(), ticket))
    }

    /// Retains a started request until its corresponding backend observation.
    ///
    /// # Errors
    ///
    /// Returns both values if a previous dispatch is still in flight.
    pub fn await_observation(
        &mut self,
        dispatch: NativeEffectDispatch,
        ticket: Ticket,
    ) -> Result<(), (NativeEffectDispatch, Ticket)> {
        if self.pending.is_some() {
            return Err((dispatch, ticket));
        }
        self.pending = Some((dispatch, ticket));
        Ok(())
    }

    /// Claims the in-flight dispatch once its original ticket has settled.
    pub fn take_pending(&mut self) -> Option<(NativeEffectDispatch, Ticket)> {
        self.pending.take()
    }

    /// Settles an in-flight dispatch whose exact native window has retired.
    ///
    /// The backend may have applied the operation before retiring. Report
    /// the indeterminate result into the same FIFO as other observations.
    pub fn retire_pending(&mut self, window: NativeWindowBinding) -> Option<NativeEffectResult> {
        if self.pending()?.0 != window {
            return None;
        }
        let (dispatch, _) = self.pending.take()?;
        Some(dispatch.indeterminate(NativeIndeterminateReason::AcknowledgementLost))
    }
}

/// Deduplicates property snapshots while preserving one-shot effect receipts.
#[derive(Debug, Default)]
pub struct NativeWindowObservationOutbox {
    live: Option<NativeWindowFacts>,
    close: Option<NativeCloseEffectAcknowledgement>,
}

impl NativeWindowObservationOutbox {
    /// Publishes changed properties or an exact new acknowledgement.
    pub fn observe(&mut self, facts: NativeWindowFacts) -> Option<NativeWindowFacts> {
        let properties = facts.without_effect_acknowledgements();
        if facts == properties && self.live == Some(properties) {
            return None;
        }
        self.live = Some(properties);
        Some(facts)
    }

    /// Holds the close receipt until the corresponding physical destruction.
    ///
    /// # Errors
    ///
    /// Returns the receipt when an earlier close already owns destruction.
    pub fn await_destruction(
        &mut self,
        receipt: NativeCloseEffectAcknowledgement,
    ) -> Result<(), NativeCloseEffectAcknowledgement> {
        if self.close.is_some() {
            return Err(receipt);
        }
        self.close = Some(receipt);
        Ok(())
    }

    /// Returns whether a close receipt is awaiting destruction.
    #[must_use]
    pub const fn has_pending_close(&self) -> bool {
        self.close.is_some()
    }

    /// Observes actual destruction and correlates a retained close receipt exactly once.
    pub fn destroyed(&mut self) -> NativeWindowFacts {
        self.live = None;
        self.close.take().map_or_else(
            NativeWindowFacts::destroyed,
            NativeWindowFacts::destroyed_after,
        )
    }
}
