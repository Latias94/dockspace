//! Provider enrollment and committed ingress watermarks.

use super::*;

/// Engine-owned publication state for the sole joined backend ingress lane.
///
/// Provider-local capture lives in [`BackendIngressRecorder`]. This ledger is
/// cloned into a host-frame candidate, so accepting a batch advances its
/// watermark only when the complete host frame commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendIngressAuthority {
    authority_domain: EngineAuthorityDomainId,
    active: Option<BackendIngressLease>,
    committed_through: BackendIngressOrdinal,
    committed_record_identity: BackendIngressRecordIdentity,
    #[cfg(test)]
    next_replacement_id: BackendIngressReplacementId,
    #[cfg(test)]
    pending_replacement: Option<BackendIngressReplacementState>,
}

#[derive(Debug, Clone)]
#[cfg(test)]
pub(crate) struct BackendIngressReplacementState {
    pub(super) handoff: BackendIngressReplacementId,
    pub(super) ticket_generation: u64,
    pub(super) ticket_monitor: Weak<()>,
    pub(super) platform: PlatformProviderReservation,
    pub(super) predecessor: BackendIngressLease,
    pub(super) recorded_through: BackendIngressOrdinal,
    pub(super) pointer_through: PointerEdgeSequence,
}

#[cfg(test)]
impl PartialEq for BackendIngressReplacementState {
    fn eq(&self, other: &Self) -> bool {
        self.handoff == other.handoff
            && self.ticket_generation == other.ticket_generation
            && Weak::ptr_eq(&self.ticket_monitor, &other.ticket_monitor)
            && self.platform == other.platform
            && self.predecessor == other.predecessor
            && self.recorded_through == other.recorded_through
            && self.pointer_through == other.pointer_through
    }
}

#[cfg(test)]
impl Eq for BackendIngressReplacementState {}

#[cfg(test)]
impl BackendIngressReplacementState {
    pub(crate) const fn handoff(&self) -> BackendIngressReplacementId {
        self.handoff
    }

    pub(crate) const fn platform(&self) -> PlatformProviderReservation {
        self.platform
    }

    pub(crate) const fn predecessor(&self) -> BackendIngressLease {
        self.predecessor
    }

    pub(crate) const fn pointer_through(&self) -> PointerEdgeSequence {
        self.pointer_through
    }

    pub(crate) fn drain_receipt(&self) -> BackendIngressDrainReceipt {
        BackendIngressDrainReceipt::from_replacement_state(
            self.predecessor,
            self.recorded_through,
            self.pointer_through,
        )
    }
}

impl BackendIngressAuthority {
    pub(crate) const fn new(authority_domain: EngineAuthorityDomainId) -> Self {
        Self {
            authority_domain,
            active: None,
            committed_through: BackendIngressOrdinal::ORIGIN,
            committed_record_identity: BackendIngressRecordIdentity::ORIGIN,
            #[cfg(test)]
            next_replacement_id: BackendIngressReplacementId::ORIGIN,
            #[cfg(test)]
            pending_replacement: None,
        }
    }

    pub(crate) const fn active(&self) -> Option<BackendIngressLease> {
        self.active
    }

    pub(crate) const fn committed_through(&self) -> BackendIngressOrdinal {
        self.committed_through
    }

    #[cfg(test)]
    pub(crate) fn reserve_replacement(
        &mut self,
        platform: PlatformProviderReservation,
        predecessor: &mut BackendIngressDrainReceipt,
    ) -> Result<BackendIngressProviderReplacementTicket, BackendIngressError> {
        if let Some(active) = self.active {
            return Err(BackendIngressError::ProviderAlreadyActive { active });
        }
        if self.pending_replacement.is_some() {
            return Err(BackendIngressError::ProviderReplacementPending);
        }
        predecessor.validate_active()?;
        if predecessor.lease.authority_domain() != self.authority_domain {
            return Err(BackendIngressError::ForeignAuthorityDomain {
                expected: self.authority_domain,
                submitted: predecessor.lease.authority_domain(),
            });
        }
        let handoff = self
            .next_replacement_id
            .checked_next()
            .ok_or(BackendIngressError::ReplacementIdentityExhausted)?;
        let predecessor = predecessor.transfer()?;
        self.next_replacement_id = handoff;
        self.pending_replacement = Some(BackendIngressReplacementState {
            handoff,
            ticket_generation: 1,
            ticket_monitor: Weak::new(),
            platform,
            predecessor: predecessor.lease,
            recorded_through: predecessor.recorded_through,
            pointer_through: predecessor.pointer_through,
        });
        let state = self
            .pending_replacement
            .as_mut()
            .expect("replacement was inserted immediately above");
        Ok(Self::issue_replacement_ticket(self.authority_domain, state))
    }

    #[cfg(test)]
    pub(crate) fn reissue_replacement_ticket(
        &mut self,
    ) -> Result<BackendIngressProviderReplacementTicket, BackendIngressError> {
        let state = self
            .pending_replacement
            .as_mut()
            .ok_or(BackendIngressError::ProviderReplacementNotPending)?;
        state.ticket_generation = state
            .ticket_generation
            .checked_add(1)
            .ok_or(BackendIngressError::ReplacementTicketGenerationExhausted)?;
        Ok(Self::issue_replacement_ticket(self.authority_domain, state))
    }

    #[cfg(test)]
    fn issue_replacement_ticket(
        authority_domain: EngineAuthorityDomainId,
        state: &mut BackendIngressReplacementState,
    ) -> BackendIngressProviderReplacementTicket {
        let monitor = Arc::new(());
        state.ticket_monitor = Arc::downgrade(&monitor);
        BackendIngressProviderReplacementTicket::new(
            authority_domain,
            state.handoff,
            state.ticket_generation,
            monitor,
        )
    }

    #[cfg(test)]
    pub(crate) fn replacement_state(
        &self,
        authority_domain: EngineAuthorityDomainId,
        handoff: BackendIngressReplacementId,
        generation: u64,
        monitor: &Arc<()>,
    ) -> Result<BackendIngressReplacementState, BackendIngressError> {
        if authority_domain != self.authority_domain {
            return Err(BackendIngressError::ForeignAuthorityDomain {
                expected: self.authority_domain,
                submitted: authority_domain,
            });
        }
        let state = self
            .pending_replacement
            .as_ref()
            .ok_or(BackendIngressError::ProviderReplacementNotPending)?;
        if state.handoff != handoff || state.ticket_generation != generation {
            return Err(BackendIngressError::UnknownReplacementTicket);
        }
        let Some(current_monitor) = state.ticket_monitor.upgrade() else {
            return Err(BackendIngressError::UnknownReplacementTicket);
        };
        if !Arc::ptr_eq(&current_monitor, monitor) {
            return Err(BackendIngressError::UnknownReplacementTicket);
        }
        Ok(state.clone())
    }

    #[cfg(test)]
    pub(crate) fn replacement_ticket_abandoned(&self) -> bool {
        self.pending_replacement
            .as_ref()
            .is_some_and(|state| state.ticket_monitor.upgrade().is_none())
    }

    #[cfg(test)]
    pub(crate) fn pending_replacement_state(
        &self,
    ) -> Result<BackendIngressReplacementState, BackendIngressError> {
        self.pending_replacement
            .clone()
            .ok_or(BackendIngressError::ProviderReplacementNotPending)
    }

    #[cfg(test)]
    pub(crate) fn complete_replacement(
        &mut self,
        authority_domain: EngineAuthorityDomainId,
        handoff: BackendIngressReplacementId,
        generation: u64,
        monitor: &Arc<()>,
    ) -> Result<(), BackendIngressError> {
        let _ = self.replacement_state(authority_domain, handoff, generation, monitor)?;
        self.pending_replacement = None;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn abort_replacement(
        &mut self,
        handoff: BackendIngressReplacementId,
    ) -> Result<(), BackendIngressError> {
        let state = self
            .pending_replacement
            .as_ref()
            .ok_or(BackendIngressError::ProviderReplacementNotPending)?;
        if state.handoff != handoff {
            return Err(BackendIngressError::UnknownReplacementTicket);
        }
        self.pending_replacement = None;
        Ok(())
    }

    pub(crate) fn enroll(
        &mut self,
        platform: PlatformObservationLease,
        pointer: PointerInputLease,
        presentation_host: PresentationHostLease,
        pointer_committed_through: PointerEdgeSequence,
    ) -> Result<BackendIngressRecorder, BackendIngressError> {
        if let Some(active) = self.active {
            return Err(BackendIngressError::ProviderAlreadyActive { active });
        }
        let recorder = BackendIngressRecorder::new(
            platform,
            pointer,
            presentation_host,
            pointer_committed_through,
        )?;
        let lease = recorder.lease();
        if lease.authority_domain() != self.authority_domain {
            return Err(BackendIngressError::ForeignAuthorityDomain {
                expected: self.authority_domain,
                submitted: lease.authority_domain(),
            });
        }
        self.active = Some(lease);
        self.committed_through = BackendIngressOrdinal::ORIGIN;
        self.committed_record_identity = BackendIngressRecordIdentity::ORIGIN;
        Ok(recorder)
    }

    #[cfg(test)]
    pub(crate) fn revoke(&mut self, lease: BackendIngressLease) -> Result<(), BackendIngressError> {
        let Some(active) = self.active else {
            return Err(BackendIngressError::ProviderUnavailable);
        };
        if active != lease {
            return Err(BackendIngressError::ProviderLeaseMismatch {
                expected: active,
                submitted: lease,
            });
        }
        self.active = None;
        self.committed_through = BackendIngressOrdinal::ORIGIN;
        self.committed_record_identity = BackendIngressRecordIdentity::ORIGIN;
        Ok(())
    }

    pub(crate) fn validate_batch(
        &self,
        batch: &BackendIngressBatch,
    ) -> Result<(), BackendIngressError> {
        let active = self
            .active
            .ok_or(BackendIngressError::ProviderUnavailable)?;
        batch.validate_against(
            active,
            self.committed_through,
            self.committed_record_identity,
        )
    }

    pub(crate) fn commit_batch(
        &mut self,
        batch: &BackendIngressBatch,
    ) -> Result<(), BackendIngressError> {
        self.validate_batch(batch)?;
        if let Some(record) = batch.records.last() {
            self.committed_record_identity = record.identity;
        }
        self.committed_through = batch.through();
        Ok(())
    }

    pub(crate) fn commit_watermark(&self) -> Option<BackendIngressCommitWatermark> {
        self.active.map(|lease| {
            BackendIngressCommitWatermark::new(
                lease,
                self.committed_through,
                self.committed_record_identity,
            )
        })
    }

    pub(crate) fn commit_prefix_retirement(
        &mut self,
        receipt: &BackendIngressPrefixRetirementReceipt,
    ) -> Result<(), BackendIngressError> {
        let authority = receipt.authority()?;
        let active = self
            .active
            .ok_or(BackendIngressError::ProviderUnavailable)?;
        if authority.lease != active {
            return Err(BackendIngressError::PrefixRetirementLeaseMismatch {
                expected: active,
                submitted: authority.lease,
            });
        }
        if authority.through != self.committed_through {
            return Err(BackendIngressError::PrefixRetirementWatermarkMismatch {
                committed: self.committed_through,
                submitted: authority.through,
            });
        }
        Ok(())
    }
}
