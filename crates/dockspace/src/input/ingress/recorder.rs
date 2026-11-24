//! Single-writer ingress capture, capacity and rollback boundaries.

use super::*;

/// Single-writer backend capture recorder.
///
/// The recorder is intentionally not cloneable. Calling its record methods in
/// the native event-loop callback order is the sole way to mint new ingress
/// ordinals. Producing a batch is non-destructive, which keeps every captured
/// interval retryable until the engine publishes its own commit watermark.
#[derive(Debug)]
pub struct BackendIngressRecorder {
    pub(super) coordination: BackendIngressCoordination,
    pub(super) lease: BackendIngressLease,
    pub(super) retired_through: BackendIngressOrdinal,
    pub(super) retired_record_identity: BackendIngressRecordIdentity,
    pub(super) last_ordinal: BackendIngressOrdinal,
    /// Monotonic append authority which deliberately does not rewind on rollback.
    pub(super) last_record_identity: BackendIngressRecordIdentity,
    pub(super) pointer_through: PointerEdgeSequence,
    pub(super) records: Vec<BackendIngressRecord>,
    pub(super) native_transport_capacity: Option<native_capacity::NativeTransportCapacity>,
}

/// Affine rollback boundary for one recorder-owned append transaction.
///
/// The fields are private so an adapter cannot forge an earlier ingress or
/// pointer watermark. Consuming this value through
/// [`BackendIngressRecorder::rollback_to`] restores only records appended by
/// the same recorder after the savepoint was minted.
#[derive(Debug)]
pub struct BackendIngressSavepoint {
    lease: BackendIngressLease,
    retired_through: BackendIngressOrdinal,
    recorded_through: BackendIngressOrdinal,
    boundary_record_identity: BackendIngressRecordIdentity,
    pointer_through: PointerEdgeSequence,
    record_count: usize,
}

impl BackendIngressRecorder {
    /// Creates the recorder for an already core-minted provider pair.
    ///
    /// The constructor remains crate-private because enrolling or replacing
    /// providers is engine authority, not an adapter decision.
    pub(crate) fn new(
        platform: PlatformObservationLease,
        pointer: PointerInputLease,
        presentation_host: PresentationHostLease,
        pointer_committed_through: PointerEdgeSequence,
    ) -> Result<Self, BackendIngressError> {
        Ok(Self {
            coordination: BackendIngressCoordination::new(),
            lease: BackendIngressLease::new(platform, pointer, presentation_host)?,
            retired_through: BackendIngressOrdinal::ORIGIN,
            retired_record_identity: BackendIngressRecordIdentity::ORIGIN,
            last_ordinal: BackendIngressOrdinal::ORIGIN,
            last_record_identity: BackendIngressRecordIdentity::ORIGIN,
            pointer_through: pointer_committed_through,
            records: Vec::new(),
            native_transport_capacity: None,
        })
    }

    /// Returns the exact provider pair owned by this recorder.
    #[must_use]
    pub const fn lease(&self) -> BackendIngressLease {
        self.lease
    }

    /// Returns the latest ordinal allocated by this recorder.
    #[must_use]
    pub const fn recorded_through(&self) -> BackendIngressOrdinal {
        self.last_ordinal
    }

    /// Returns the latest pointer edge sequence captured by this recorder.
    #[must_use]
    pub(crate) const fn pointer_through(&self) -> PointerEdgeSequence {
        self.pointer_through
    }

    /// Mints an affine rollback boundary for a fallible adapter append batch.
    #[must_use]
    pub fn savepoint(&self) -> BackendIngressSavepoint {
        BackendIngressSavepoint {
            lease: self.lease,
            retired_through: self.retired_through,
            recorded_through: self.last_ordinal,
            boundary_record_identity: self
                .records
                .last()
                .map_or(self.retired_record_identity, |record| record.identity),
            pointer_through: self.pointer_through,
            record_count: self.records.len(),
        }
    }

    /// Removes every record appended after an exact recorder savepoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the savepoint belongs to another provider, when
    /// its prefix was already reclaimed, or when recorder state no longer
    /// contains the captured prefix.
    pub fn rollback_to(
        &mut self,
        savepoint: BackendIngressSavepoint,
    ) -> Result<(), BackendIngressError> {
        self.validate_savepoint(&savepoint)?;
        let _coordination = self.coordination.lock();
        for record in &self.records[savepoint.record_count..] {
            match record.liveness.state() {
                BackendIngressRecordState::Live => {}
                BackendIngressRecordState::Publishing | BackendIngressRecordState::Committed => {
                    return Err(BackendIngressError::RollbackCrossesPublishedRecord {
                        ordinal: record.ordinal,
                    });
                }
                BackendIngressRecordState::Revoked => {
                    return Err(BackendIngressError::RecordRevoked {
                        ordinal: record.ordinal,
                    });
                }
            }
        }
        let released_data_bytes =
            self.native_retained_data_bytes(&self.records[savepoint.record_count..])?;
        for record in &self.records[savepoint.record_count..] {
            record.liveness.revoke_live();
        }
        self.records.truncate(savepoint.record_count);
        if let Some(capacity) = self.native_transport_capacity.as_mut() {
            capacity.retained_data_bytes -= released_data_bytes;
        }
        self.last_ordinal = savepoint.recorded_through;
        self.pointer_through = savepoint.pointer_through;
        Ok(())
    }

    /// Finalizes an unexposed runtime-owned append transaction as one commit group.
    ///
    /// The runtime collects user iterators before opening the transaction and
    /// does not expose its recorder while appending. Consequently no batch or
    /// record clone can escape before these commit boundaries are frozen.
    pub(crate) fn finish_atomic_append(
        &mut self,
        savepoint: BackendIngressSavepoint,
    ) -> Result<(), BackendIngressError> {
        self.validate_savepoint(&savepoint)?;
        let _coordination = self.coordination.lock();
        let appended = &mut self.records[savepoint.record_count..];
        for record in appended.iter() {
            if record.liveness.state() != BackendIngressRecordState::Live {
                return Err(BackendIngressError::AtomicAppendNotLive {
                    ordinal: record.ordinal,
                });
            }
        }
        if let Some((last, interior)) = appended.split_last_mut() {
            for record in interior {
                record.commit_boundary_after = false;
            }
            last.commit_boundary_after = true;
        }
        Ok(())
    }

    fn validate_savepoint(
        &self,
        savepoint: &BackendIngressSavepoint,
    ) -> Result<(), BackendIngressError> {
        if savepoint.lease != self.lease {
            return Err(BackendIngressError::SavepointLeaseMismatch {
                expected: self.lease,
                submitted: savepoint.lease,
            });
        }
        if savepoint.retired_through != self.retired_through {
            return Err(BackendIngressError::SavepointPrefixReclaimed {
                saved: savepoint.retired_through,
                current: self.retired_through,
            });
        }
        if savepoint.record_count > self.records.len()
            || savepoint.recorded_through > self.last_ordinal
        {
            return Err(BackendIngressError::SavepointAhead {
                saved: savepoint.recorded_through,
                current: self.last_ordinal,
            });
        }
        let current_boundary = self
            .record_count_boundary(savepoint.record_count)
            .map(|record| (record.ordinal, record.identity))
            .unwrap_or((self.retired_through, self.retired_record_identity));
        if current_boundary
            != (
                savepoint.recorded_through,
                savepoint.boundary_record_identity,
            )
        {
            return Err(BackendIngressError::SavepointPrefixRewritten {
                boundary: savepoint.recorded_through,
            });
        }
        Ok(())
    }

    /// Stops this producer and returns its non-forgeable replacement proof.
    ///
    /// Consuming the recorder is the type-level join boundary: no caller can mint another record
    /// for this backend lifetime afterward. Captured suffix records which were not committed by
    /// the core are intentionally abandoned and cannot cross the subsequent lease replacement.
    #[must_use]
    #[cfg(test)]
    pub fn drain(self) -> BackendIngressDrainReceipt {
        let _coordination = self.coordination.lock();
        for record in &self.records {
            if record.liveness.is_live() {
                record.liveness.revoke_live();
            }
        }
        BackendIngressDrainReceipt {
            lease: self.lease,
            recorded_through: self.last_ordinal,
            pointer_through: self.pointer_through,
            consumed: false,
        }
    }

    /// Records one complete platform snapshot at the next backend position.
    pub fn record_platform_snapshot(
        &mut self,
        expected_epoch: WorkspaceEpoch,
        snapshot: PlatformSnapshot,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::PlatformSnapshot {
            expected_epoch,
            snapshot,
        })
    }

    /// Records permanent producer quiescence for one exact platform binding.
    ///
    /// The caller must first remove every route, callback, and sidecar capable of
    /// generating a later fact for this binding. A future fact which violates that
    /// promise is rejected fail-closed by the core after the guard is reclaimed.
    ///
    /// # Errors
    ///
    /// Returns an error when the binding belongs to another engine authority
    /// domain, the retained prefix already closes the same binding lane, or the
    /// backend ordinal cannot advance without wrapping.
    pub fn record_platform_binding_quiescence(
        &mut self,
        binding: ViewportBinding,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        if binding.authority_domain() != self.lease.authority_domain() {
            return Err(BackendIngressError::BindingQuiescenceAuthorityMismatch {
                expected: self.lease.authority_domain(),
                submitted: binding.authority_domain(),
            });
        }
        if self.records.iter().any(|record| {
            matches!(
                record.payload(),
                BackendIngressPayload::PlatformBindingQuiesced {
                    binding: recorded
                } if *recorded == binding
            )
        }) {
            return Err(BackendIngressError::DuplicateBindingQuiescence { binding });
        }
        self.push(BackendIngressPayload::PlatformBindingQuiesced { binding })
    }

    /// Records one exact binding-scoped native-close observation.
    pub fn record_native_close_observation(
        &mut self,
        expected_epoch: WorkspaceEpoch,
        observation: WindowCloseObservation,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::NativeCloseObservation {
            expected_epoch,
            observation,
        })
    }

    /// Records one globally consistent native-focus observation.
    pub fn record_global_focus_observation(
        &mut self,
        expected_epoch: WorkspaceEpoch,
        observation: crate::platform::focus::FocusObservationEnvelope,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::GlobalFocusObservation {
            expected_epoch,
            observation,
        })
    }

    /// Records one platform-effect result at the next backend position.
    pub fn record_platform_effect_result(
        &mut self,
        result: EffectResult,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::PlatformEffectResult(result))
    }

    /// Records native input device removal at the next backend position.
    pub fn record_native_input_device_removal(
        &mut self,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::NativeInputDeviceRemoval)
    }

    /// Records input termination for one exact native binding.
    ///
    /// This does not destroy, release or declare the binding quiescent. It
    /// preserves pointer sequence and previously accepted output obligations.
    pub fn record_native_input_binding_retired(
        &mut self,
        binding: ViewportBinding,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::NativeInputBindingRetired { binding })
    }

    /// Records one non-pointer semantic action at the next backend position.
    ///
    /// Platform facts must use their typed lanes and configuration remains a
    /// terminal host-frame phase, so neither can be smuggled through this path.
    pub fn record_semantic_input(
        &mut self,
        input: EngineInput,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        if matches!(input, EngineInput::CancelNativeInputForOverload { .. }) {
            return Err(BackendIngressError::SemanticInputIsHostControl);
        }
        if input.is_backend_ingress_fact() {
            return Err(BackendIngressError::SemanticInputIsBackendFact);
        }
        if input.is_configuration_commit() {
            return Err(BackendIngressError::SemanticInputIsConfiguration);
        }
        self.push(BackendIngressPayload::SemanticInput(Box::new(input)))
    }

    #[cfg(test)]
    pub(crate) fn record_semantic_input_receipt(
        &mut self,
        input: EngineInput,
    ) -> Result<BackendIngressRecordReceipt, BackendIngressError> {
        let ordinal = self.record_semantic_input(input)?;
        let record = self
            .records
            .last()
            .expect("a successful append retains its record");
        Ok(BackendIngressRecordReceipt {
            lease: record.lease,
            ordinal,
            identity: record.identity,
            liveness: record.liveness.clone(),
        })
    }

    pub(crate) fn record_presentation_observation(
        &mut self,
        entry: HostPresentationObservationEntry,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.push(BackendIngressPayload::PresentationObservation {
            host: self.lease.presentation_host,
            entry,
        })
    }

    /// Records one viewport registration through this recorder's exact platform provider.
    ///
    /// The provider identity is deliberately not supplied by the caller. This keeps a native
    /// runtime from accidentally pairing a current registration with a superseded platform
    /// provider while still allowing registration to share the backend's total input order.
    pub fn record_viewport_registration(
        &mut self,
        expected: WorkspaceVersion,
        surface: SurfaceId,
        token: WindowToken,
        role: ViewportRole,
        recovery_target: Option<SurfaceRecoveryTarget>,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.record_semantic_input(EngineInput::RegisterViewport {
            provider: self.lease.platform_provider(),
            expected,
            surface,
            token,
            role,
            recovery_target,
        })
    }

    /// Records the first registration of an existing docking-owned child viewport.
    ///
    /// The core resolves the current recovery anchor and mints any presentation identity; this
    /// recorder carries only adapter-observed bootstrap facts.
    pub fn record_child_viewport_bootstrap(
        &mut self,
        expected: WorkspaceVersion,
        surface: SurfaceId,
        token: WindowToken,
        recovery: SurfaceRecoveryBootstrap,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        self.record_semantic_input(EngineInput::BootstrapChildViewport {
            provider: self.lease.platform_provider(),
            expected,
            surface,
            token,
            recovery,
        })
    }

    /// Records one pointer segment at the next backend position.
    ///
    /// Segments remain edgewise because receiver evidence is requested after
    /// each edge. The pointer watermark must continue exactly from the preceding
    /// segment even when platform records occur between pointer records.
    pub fn record_pointer_segment(
        &mut self,
        segment: PointerEdgeJournal,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        if segment.len() > 1 {
            return Err(BackendIngressError::PointerSegmentMustBeEdgewise {
                edges: segment.len(),
            });
        }
        if segment.previous() != self.pointer_through {
            return Err(BackendIngressError::PointerSegmentPreviousMismatch {
                expected: self.pointer_through,
                submitted: segment.previous(),
            });
        }
        let through = segment.through();
        let ordinal = self.push(BackendIngressPayload::PointerSegment(segment))?;
        self.pointer_through = through;
        Ok(ordinal)
    }

    /// Returns the complete immutable stream captured so far.
    #[cfg(test)]
    pub fn pending_batch(&self) -> Result<BackendIngressBatch, BackendIngressError> {
        let mut records = Vec::with_capacity(self.records.len());
        records.extend_from_slice(&self.records);
        BackendIngressBatch::from_records(
            self.coordination.clone(),
            self.lease,
            self.retired_through,
            self.retired_record_identity,
            self.last_ordinal,
            records,
        )
    }

    /// Returns the immutable suffix after an engine-committed watermark.
    ///
    /// This operation does not consume records. Repeating it before more facts
    /// are recorded returns an exactly equal batch suitable for atomic retry.
    pub fn batch_after(
        &self,
        committed_through: BackendIngressOrdinal,
    ) -> Result<BackendIngressBatch, BackendIngressError> {
        self.batch_prefix_after(committed_through, NonZeroUsize::MAX)
    }

    /// Returns at most `max_records` complete records after a committed watermark.
    ///
    /// Only the selected records are cloned; the recorder remains unchanged.
    /// The batch ends at its actual last record, or at `committed_through` when
    /// the suffix is empty. Repeating the selection before more facts are recorded
    /// returns an exactly equal batch suitable for atomic retry.
    /// A selection stops before an atomic group which would cross the limit.
    ///
    /// # Errors
    ///
    /// Returns an error when the watermark predates the retired prefix, advances
    /// beyond captured input, no longer identifies a retained commit boundary,
    /// or the first pending atomic group cannot fit within the requested limit.
    pub fn batch_prefix_after(
        &self,
        committed_through: BackendIngressOrdinal,
        max_records: NonZeroUsize,
    ) -> Result<BackendIngressBatch, BackendIngressError> {
        if committed_through < self.retired_through {
            return Err(BackendIngressError::CommittedWatermarkRetired {
                committed: committed_through,
                retired: self.retired_through,
            });
        }
        if committed_through > self.last_ordinal {
            return Err(BackendIngressError::CommittedWatermarkAhead {
                committed: committed_through,
                recorded: self.last_ordinal,
            });
        }
        let start = self
            .records
            .partition_point(|record| record.ordinal <= committed_through);
        let previous_record_identity = if committed_through == self.retired_through {
            self.retired_record_identity
        } else {
            self.record_count_boundary(start)
                .filter(|record| record.ordinal == committed_through)
                .map(|record| record.identity)
                .ok_or(BackendIngressError::CommitWatermarkContentMismatch {
                    ordinal: committed_through,
                })?
        };
        if self
            .record_count_boundary(start)
            .is_some_and(|record| !record.is_commit_boundary())
        {
            return Err(BackendIngressError::AtomicGroupIncomplete {
                through: committed_through,
            });
        }
        let selected = &self.records[start..];
        let end = selected.len().min(max_records.get());
        let complete_end = selected[..end]
            .iter()
            .rposition(BackendIngressRecord::is_commit_boundary)
            .map_or(0, |index| index + 1);
        if end > 0 && complete_end == 0 {
            return Err(BackendIngressError::AtomicGroupExceedsPrefixLimit {
                first: selected[0].ordinal,
                max_records,
            });
        }
        let mut records = Vec::with_capacity(complete_end);
        records.extend_from_slice(&selected[..complete_end]);
        let through = records
            .last()
            .map_or(committed_through, |record| record.ordinal);
        BackendIngressBatch::from_records(
            self.coordination.clone(),
            self.lease,
            committed_through,
            previous_record_identity,
            through,
            records,
        )
    }

    /// Selects an ordinary bounded prefix or just the first indivisible group.
    /// Oversized groups are computed cooperatively by one private core cursor;
    /// their later independent suffix is never cloned into that attempt.
    pub(crate) fn replay_prefix_after(
        &self,
        committed_through: BackendIngressOrdinal,
        max_records: NonZeroUsize,
    ) -> Result<BackendIngressBatch, BackendIngressError> {
        match self.batch_prefix_after(committed_through, max_records) {
            Err(BackendIngressError::AtomicGroupExceedsPrefixLimit { .. }) => {
                let start = self
                    .records
                    .partition_point(|record| record.ordinal <= committed_through);
                let length = self.records[start..]
                    .iter()
                    .position(BackendIngressRecord::is_commit_boundary)
                    .and_then(|index| NonZeroUsize::new(index + 1))
                    .ok_or(BackendIngressError::AtomicGroupIncomplete {
                        through: self.last_ordinal,
                    })?;
                self.batch_prefix_after(committed_through, length)
            }
            result => result,
        }
    }

    /// Reclaims the exact prefix proven committed by the core.
    ///
    /// # Errors
    ///
    /// Returns an error when the watermark belongs to another backend lifetime,
    /// regresses behind the reclaimed prefix, or advances beyond captured input.
    pub fn retire_committed_prefix(
        &mut self,
        committed: BackendIngressCommitWatermark,
    ) -> Result<Option<BackendIngressPrefixRetirementReceipt>, BackendIngressError> {
        let retained = self.validate_committed_prefix(committed)?;
        if retained == 0 {
            return Ok(None);
        }
        let released_data_bytes = self.native_retained_data_bytes(&self.records[..retained])?;
        let quiescence_count = self.records[..retained]
            .iter()
            .filter(|record| {
                matches!(
                    record.payload(),
                    BackendIngressPayload::PlatformBindingQuiesced { .. }
                )
            })
            .count();
        let mut binding_quiescences = Vec::new();
        binding_quiescences
            .try_reserve_exact(quiescence_count)
            .map_err(|_| BackendIngressError::CapacityAllocationFailed)?;
        binding_quiescences.extend(self.records[..retained].iter().filter_map(|record| {
            match record.payload() {
                BackendIngressPayload::PlatformBindingQuiesced { binding } => Some(*binding),
                BackendIngressPayload::PlatformSnapshot { .. }
                | BackendIngressPayload::NativeCloseObservation { .. }
                | BackendIngressPayload::GlobalFocusObservation { .. }
                | BackendIngressPayload::PlatformEffectResult(_)
                | BackendIngressPayload::NativeInputDeviceRemoval
                | BackendIngressPayload::NativeInputBindingRetired { .. }
                | BackendIngressPayload::SemanticInput(_)
                | BackendIngressPayload::PresentationObservation { .. }
                | BackendIngressPayload::PointerSegment(_) => None,
            }
        }));
        binding_quiescences.sort_unstable();
        binding_quiescences.dedup();
        self.records.drain(..retained);
        if let Some(capacity) = self.native_transport_capacity.as_mut() {
            capacity.retained_data_bytes -= released_data_bytes;
        }
        self.retired_through = committed.through;
        self.retired_record_identity = committed.record_identity;
        if binding_quiescences.is_empty() {
            Ok(None)
        } else {
            Ok(Some(BackendIngressPrefixRetirementReceipt::new(
                self.lease,
                committed.through,
                binding_quiescences,
            )))
        }
    }

    pub(super) fn validate_committed_prefix(
        &self,
        committed: BackendIngressCommitWatermark,
    ) -> Result<usize, BackendIngressError> {
        if committed.lease != self.lease {
            return Err(BackendIngressError::CommitWatermarkLeaseMismatch {
                expected: self.lease,
                submitted: committed.lease,
            });
        }
        if committed.through < self.retired_through {
            return Err(BackendIngressError::RetirementWatermarkRegressed {
                retired: self.retired_through,
                submitted: committed.through,
            });
        }
        if committed.through > self.last_ordinal {
            return Err(BackendIngressError::CommittedWatermarkAhead {
                committed: committed.through,
                recorded: self.last_ordinal,
            });
        }
        if committed.through == self.retired_through {
            if committed.record_identity != self.retired_record_identity {
                return Err(BackendIngressError::CommitWatermarkContentMismatch {
                    ordinal: committed.through,
                });
            }
            return Ok(0);
        }
        let retained = self
            .records
            .partition_point(|record| record.ordinal <= committed.through);
        let committed_record = self.records.get(retained.saturating_sub(1)).ok_or(
            BackendIngressError::CommitWatermarkContentMismatch {
                ordinal: committed.through,
            },
        )?;
        if committed_record.ordinal != committed.through
            || committed_record.identity != committed.record_identity
        {
            return Err(BackendIngressError::CommitWatermarkContentMismatch {
                ordinal: committed.through,
            });
        }
        Ok(retained)
    }

    /// Returns the number of captured records not yet reclaimed.
    #[doc(hidden)]
    #[must_use]
    pub const fn retained_record_count(&self) -> usize {
        self.records.len()
    }

    /// Returns retained records in backend capture order without cloning them.
    #[must_use]
    pub(crate) fn retained_records(&self) -> &[BackendIngressRecord] {
        &self.records
    }

    /// Returns the record buffer capacity, including currently unused slots.
    #[must_use]
    pub(crate) fn record_buffer_capacity_bytes(&self) -> usize {
        self.records.capacity() * size_of::<BackendIngressRecord>()
    }

    /// Returns fixed shared data charged once through this recorder.
    ///
    /// Each append owns distinct payload and liveness allocations. This counts
    /// their data objects and the one coordination mutex, excluding payload
    /// descendants, Arc control blocks, and allocator overhead.
    #[must_use]
    pub(crate) fn shared_fixed_data_bytes(&self) -> usize {
        size_of::<Mutex<()>>()
            + self.records.len() * (size_of::<BackendIngressPayload>() + size_of::<AtomicU8>())
    }

    fn push(
        &mut self,
        payload: BackendIngressPayload,
    ) -> Result<BackendIngressOrdinal, BackendIngressError> {
        let retained_data_bytes = self.check_native_transport_append(&payload)?;
        let ordinal = self
            .last_ordinal
            .checked_next()
            .ok_or(BackendIngressError::OrdinalExhausted)?;
        let identity = self
            .last_record_identity
            .checked_next()
            .ok_or(BackendIngressError::RecordIdentityExhausted)?;
        let liveness = BackendIngressRecordLiveness::new();
        self.records.push(BackendIngressRecord {
            lease: self.lease,
            ordinal,
            identity,
            payload: Arc::new(payload),
            liveness,
            commit_boundary_after: true,
        });
        self.last_ordinal = ordinal;
        self.last_record_identity = identity;
        if let Some(capacity) = self.native_transport_capacity.as_mut() {
            capacity.retained_data_bytes = retained_data_bytes;
        }
        Ok(ordinal)
    }

    fn record_count_boundary(&self, record_count: usize) -> Option<&BackendIngressRecord> {
        record_count
            .checked_sub(1)
            .and_then(|index| self.records.get(index))
    }
}
