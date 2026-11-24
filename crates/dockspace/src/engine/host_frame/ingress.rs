//! Ordered backend record replay and pointer receiver reduction.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PointerPresentationFence {
    workspace: WorkspaceVersion,
    requirements: RequirementRevision,
    drag_preview_surface: Option<SurfaceId>,
    contained_preview_surface: Option<SurfaceId>,
}

impl PointerPresentationFence {
    fn capture(engine: &DockEngine) -> Self {
        Self {
            workspace: engine.version,
            requirements: engine
                .presentation_authority
                .presentation_requirements
                .revision(),
            drag_preview_surface: engine
                .presentation_preview()
                .map(|preview| preview.visual().surface()),
            contained_preview_surface: engine
                .presentation_contained_transform_preview()
                .map(|preview| preview.surface()),
        }
    }
}

const fn pointer_move_outcome_allows_local_presentation_refresh(
    outcome: &InteractionOutcome,
) -> bool {
    match outcome {
        InteractionOutcome::DragBegan { .. }
        | InteractionOutcome::PreviewUpdated { .. }
        | InteractionOutcome::Cancelled { .. }
        | InteractionOutcome::ResizeUpdated { .. }
        | InteractionOutcome::ContainedTransformPreviewUpdated { .. }
        | InteractionOutcome::Rejected(_) => true,
        InteractionOutcome::TouchArmed { .. }
        | InteractionOutcome::TouchActivated { .. }
        | InteractionOutcome::TouchYieldedToContent { .. }
        | InteractionOutcome::ContextMenuRequested { .. }
        | InteractionOutcome::Scroll(_)
        | InteractionOutcome::CloseRequested { .. }
        | InteractionOutcome::ProductActionApplied(_)
        | InteractionOutcome::ProductActionRejected(_)
        | InteractionOutcome::TabStripControlActivated { .. }
        | InteractionOutcome::TabListMenuItemSelected { .. }
        | InteractionOutcome::TabListMenuDismissed { .. }
        | InteractionOutcome::TabListMenuFrameConsumed { .. }
        | InteractionOutcome::TabStripScrolled { .. }
        | InteractionOutcome::TabListMenuScrolled { .. }
        | InteractionOutcome::TabListMenuFocusMoved { .. }
        | InteractionOutcome::DragArmed { .. }
        | InteractionOutcome::DragDelivered { .. }
        | InteractionOutcome::ReleasePending { .. }
        | InteractionOutcome::ResizeBegan { .. }
        | InteractionOutcome::SplitterAdjusted { .. }
        | InteractionOutcome::ResizeDelivered { .. }
        | InteractionOutcome::ContainedPlacementApplied { .. }
        | InteractionOutcome::ContainedTransformBegan { .. }
        | InteractionOutcome::ContainedTransformDelivered { .. } => false,
    }
}

fn record_pointer_scroll_outcome_for_local_presentation_refresh(
    outcome: &InteractionOutcome,
    dirty_surfaces: &mut Vec<SurfaceId>,
) -> bool {
    match outcome {
        InteractionOutcome::Scroll(ScrollReductionOutcome::Applied(application)) => {
            if application.applied_delta() != 0.0 {
                dirty_surfaces.push(application.receiver().surface());
            }
            true
        }
        InteractionOutcome::Scroll(
            ScrollReductionOutcome::Began { .. }
            | ScrollReductionOutcome::Suppressed { .. }
            | ScrollReductionOutcome::Terminated { .. },
        ) => true,
        _ => false,
    }
}

impl CoreHostFrame {
    fn refresh_presentation_after_pointer_segment(
        &mut self,
        before: PointerPresentationFence,
        reduced: &[crate::engine::transition::ReducedPointerEdge],
    ) -> Result<(), EngineError> {
        let after = PointerPresentationFence::capture(&self.candidate);
        if before.workspace == after.workspace && before.requirements == after.requirements {
            if reduced.is_empty() && self.try_refresh_presentation_surfaces([]) {
                return Ok(());
            }

            let moved_only = !reduced.is_empty()
                && reduced.iter().all(|edge| {
                    edge.edge().kind() == PointerEdgeKind::Moved
                        && edge
                            .interaction_outcomes()
                            .iter()
                            .all(pointer_move_outcome_allows_local_presentation_refresh)
                });
            if moved_only
                && self.try_refresh_presentation_surfaces(
                    [
                        before.drag_preview_surface,
                        after.drag_preview_surface,
                        before.contained_preview_surface,
                        after.contained_preview_surface,
                    ]
                    .into_iter()
                    .flatten(),
                )
            {
                return Ok(());
            }

            let mut dirty_surfaces = Vec::new();
            let scrolled_only = !reduced.is_empty()
                && reduced.iter().all(|edge| {
                    matches!(edge.edge().kind(), PointerEdgeKind::Scrolled(_))
                        && edge.interaction_outcomes().iter().all(|outcome| {
                            record_pointer_scroll_outcome_for_local_presentation_refresh(
                                outcome,
                                &mut dirty_surfaces,
                            )
                        })
                });
            dirty_surfaces.extend(
                [
                    before.drag_preview_surface,
                    after.drag_preview_surface,
                    before.contained_preview_surface,
                    after.contained_preview_surface,
                ]
                .into_iter()
                .flatten(),
            );
            if scrolled_only && self.try_refresh_presentation_surfaces(dirty_surfaces) {
                return Ok(());
            }
        }

        self.refresh_presentation_snapshot()
    }

    pub(super) fn retain_presented_interaction_authority_in_roster(&mut self) {
        let roster = &self.frozen_presentation_roster;
        self.frozen_pointer_outputs
            .retain(|surface, _| roster.contains_surface(*surface));
        self.frozen_pointer_presentations
            .retain(|surface, _| roster.contains_surface(*surface));
        #[cfg(test)]
        self.frozen_prepared_presentations
            .retain(|surface, _| roster.contains_surface(*surface));
    }

    /// Starts one immutable backend interval and reduces at most its first record.
    ///
    /// Platform records reduce immediately. A pointer record pauses replay after
    /// freezing its receiver challenge; the caller must answer it through
    /// [`Self::submit_backend_pointer_receiver_receipts`]. At a complete record
    /// boundary the caller explicitly resumes through [`Self::resume_backend_ingress`]
    /// or seals the processed prefix through [`Self::seal_backend_ingress_prefix`].
    /// The candidate watermark advances only at seal or batch completion and
    /// remains rollbackable until this host frame commits.
    ///
    /// # Errors
    ///
    /// Returns an error for an unavailable provider, a second batch, invalid
    /// interval or append authority, a poisoned frame, or failed record reduction.
    pub fn submit_backend_ingress(
        &mut self,
        batch: BackendIngressBatch,
    ) -> Result<BackendIngressProgress, CoreHostFrameError> {
        self.prepare_backend_ingress(batch)?;
        self.advance_backend_ingress()
    }

    pub(crate) fn prepare_backend_ingress(
        &mut self,
        batch: BackendIngressBatch,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self.backend_ingress.is_none() {
            return self.reject_value(CoreHostFrameError::BackendIngressUnexpected);
        }
        if self.backend_ingress_batch_submitted {
            return self.reject_value(CoreHostFrameError::DuplicateBackendIngressBatch);
        }
        if let Err(source) = self.candidate.backend_ingress.validate_batch(&batch) {
            return self.reject_value(CoreHostFrameError::BackendIngressRejected { source });
        }
        self.backend_ingress_batch_submitted = true;
        self.pending_backend_ingress = Some(HostBackendIngressCursor::new(batch));
        Ok(())
    }

    /// Answers and completes the paused pointer record without starting its successor.
    ///
    /// # Errors
    ///
    /// Returns an error without a pending pointer challenge, for inexact receipts,
    /// or when the frame or pointer reduction rejects the submitted evidence.
    pub fn submit_backend_pointer_receiver_receipts(
        &mut self,
        receipts: PointerReceiverReceiptBatch,
    ) -> Result<BackendIngressProgress, CoreHostFrameError> {
        if self.pending_backend_ingress.is_none() || self.pending_pointer_segment.is_none() {
            return self.reject_value(CoreHostFrameError::BackendIngressReceiptsUnexpected);
        }
        self.submit_pointer_receiver_receipts_inner(receipts, true)?;
        self.complete_backend_ingress_record()
    }

    /// Reduces at most the next record after a complete record boundary.
    ///
    /// A pointer record remains incomplete until its exact receiver receipts
    /// reduce successfully. No successor starts while that challenge is pending.
    ///
    /// # Errors
    ///
    /// Returns an error without a live complete-record boundary, for a poisoned
    /// frame, or when reduction of the next record fails.
    pub fn resume_backend_ingress(&mut self) -> Result<BackendIngressProgress, CoreHostFrameError> {
        self.ensure_backend_ingress_boundary()?;
        self.advance_backend_ingress()
    }

    /// Seals only the fully reduced prefix of the submitted immutable batch.
    ///
    /// The suffix never enters the commit guard and remains recorder-owned for
    /// retry. Sealing is forbidden while a pointer receiver challenge is pending.
    /// An indivisible append group must also reach its final record before seal.
    /// The returned watermark remains private until the whole host frame commits.
    ///
    /// # Errors
    ///
    /// Returns an error for a pending pointer challenge, an incomplete atomic
    /// group, a missing cursor, a poisoned frame, or revoked prefix authority.
    pub fn seal_backend_ingress_prefix(
        &mut self,
    ) -> Result<BackendIngressOrdinal, CoreHostFrameError> {
        self.ensure_backend_ingress_boundary()?;
        let cursor = self
            .pending_backend_ingress
            .take()
            .expect("validated backend ingress boundary retains its cursor");
        let batch = match cursor.into_completed_prefix() {
            Ok(batch) => batch,
            Err(source) => {
                return self.reject_value(CoreHostFrameError::BackendIngressRejected { source });
            }
        };
        self.finish_backend_ingress(batch)
    }

    pub(super) fn ensure_backend_ingress_boundary(&mut self) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self.pending_pointer_segment.is_some()
            || !self
                .pending_backend_ingress
                .as_ref()
                .is_some_and(HostBackendIngressCursor::at_record_boundary)
        {
            return self.reject(CoreHostFrameError::BackendIngressBoundaryRequired);
        }
        Ok(())
    }

    fn complete_backend_ingress_record(
        &mut self,
    ) -> Result<BackendIngressProgress, CoreHostFrameError> {
        let Some(cursor) = self.pending_backend_ingress.as_mut() else {
            return self.reject_value(CoreHostFrameError::BackendIngressIncomplete);
        };
        cursor.complete_record();
        if cursor.is_complete() {
            let cursor = self
                .pending_backend_ingress
                .take()
                .expect("completed backend ingress retains its cursor");
            self.finish_backend_ingress(cursor.into_batch())?;
            Ok(BackendIngressProgress::Complete)
        } else {
            Ok(BackendIngressProgress::RecordBoundary {
                completed_records: cursor.completed_records(),
                can_seal: cursor.can_seal(),
            })
        }
    }

    fn finish_backend_ingress(
        &mut self,
        batch: BackendIngressBatch,
    ) -> Result<BackendIngressOrdinal, CoreHostFrameError> {
        if let Err(source) = self.candidate.backend_ingress.commit_batch(&batch) {
            return self.reject_value(CoreHostFrameError::BackendIngressRejected { source });
        }
        let through = batch.through();
        self.backend_ingress_commit_guard = Some(batch.commit_guard());
        self.backend_ingress_complete = true;
        Ok(through)
    }

    fn advance_backend_ingress(&mut self) -> Result<BackendIngressProgress, CoreHostFrameError> {
        let next = {
            let Some(cursor) = self.pending_backend_ingress.as_mut() else {
                return self.reject_value(CoreHostFrameError::BackendIngressIncomplete);
            };
            let Some(next) = cursor.next() else {
                let cursor = self
                    .pending_backend_ingress
                    .take()
                    .expect("checked backend ingress cursor remains present");
                let batch = cursor.into_batch();
                self.finish_backend_ingress(batch)?;
                return Ok(BackendIngressProgress::Complete);
            };
            next
        };

        let (ordinal, payload) = next;
        match payload {
            BackendIngressPayload::PlatformSnapshot {
                expected_epoch,
                snapshot,
            } => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen")
                    .platform_provider();
                self.append_backend_input(
                    ordinal,
                    EngineInput::PublishPlatformSnapshot {
                        provider,
                        expected_epoch,
                        snapshot,
                    },
                )?;
            }
            BackendIngressPayload::PlatformBindingQuiesced { .. } => {
                // This is an adapter-retention fact, not a semantic mutation. The recorder
                // turns it into an affine proof only after this complete batch commits.
            }
            BackendIngressPayload::NativeCloseObservation {
                expected_epoch,
                observation,
            } => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen")
                    .platform_provider();
                self.append_backend_input(
                    ordinal,
                    EngineInput::PublishNativeCloseObservation {
                        provider,
                        expected_epoch,
                        observation,
                    },
                )?;
            }
            BackendIngressPayload::GlobalFocusObservation {
                expected_epoch,
                observation,
            } => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen")
                    .platform_provider();
                self.append_backend_input(
                    ordinal,
                    EngineInput::PublishGlobalFocusObservation {
                        provider,
                        expected_epoch,
                        observation,
                    },
                )?;
            }
            BackendIngressPayload::PlatformEffectResult(result) => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen")
                    .platform_provider();
                self.append_backend_input(
                    ordinal,
                    EngineInput::ReportPlatformEffect {
                        provider,
                        expected_epoch: result.receipt_epoch(),
                        result,
                    },
                )?;
            }
            BackendIngressPayload::NativeInputDeviceRemoval => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen");
                self.append_backend_input(
                    ordinal,
                    EngineInput::ObserveNativeInputDeviceRemoval { provider },
                )?;
            }
            BackendIngressPayload::NativeInputBindingRetired { binding } => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen");
                self.append_backend_input(
                    ordinal,
                    EngineInput::ObserveNativeInputBindingRetired { provider, binding },
                )?;
            }
            BackendIngressPayload::SemanticInput(input) => {
                self.append_backend_semantic_input(ordinal, *input)?;
            }
            BackendIngressPayload::PresentationObservation { host, entry } => {
                self.reduce_backend_presentation_observation(ordinal, host, entry)?;
            }
            BackendIngressPayload::PointerSegment(segment) => {
                let provider = self
                    .backend_ingress
                    .expect("validated backend ingress remains frozen")
                    .pointer_provider();
                self.submit_pointer_journal_inner(
                    provider,
                    segment,
                    PointerJournalSubmissionAuthority::BackendIngress,
                    None,
                )?;
                return Ok(BackendIngressProgress::ReceiverReceiptsRequired);
            }
        }
        self.complete_backend_ingress_record()
    }

    fn reduce_backend_presentation_observation(
        &mut self,
        _ordinal: BackendIngressOrdinal,
        host: PresentationHostLease,
        entry: HostPresentationObservationEntry,
    ) -> Result<(), CoreHostFrameError> {
        let expected_host = self
            .backend_ingress
            .expect("validated backend ingress remains frozen")
            .presentation_host();
        if host != expected_host {
            return self.reject(CoreHostFrameError::BackendIngressRejected {
                source: BackendIngressError::PresentationHostLeaseMismatch {
                    expected: expected_host,
                    submitted: host,
                },
            });
        }
        if self.configuration_phase_started {
            return self.reject(CoreHostFrameError::SemanticAfterConfiguration);
        }
        if self.presentation_phase_started {
            return self.reject(CoreHostFrameError::InputAfterPresentation);
        }
        if self.pending_pointer_segment.is_some() {
            return self.reject(CoreHostFrameError::PointerReceiverReceiptsMissingBeforeInput);
        }

        let stamp = CoreCausalStamp(self.next_causal_ordinal);
        let Some(next_causal_ordinal) = self.next_causal_ordinal.checked_add(1) else {
            return self.reject(CoreHostFrameError::CausalOrdinalExhausted);
        };
        self.next_causal_ordinal = next_causal_ordinal;
        let cause = ReductionCause::PresentationObservation {
            tick: self.tick,
            ordinal: stamp.ordinal(),
            host,
            stream: entry.stream(),
        };

        let reduction = self.candidate.reduce_presentation_observation_entry(
            cause,
            host,
            entry,
            &mut self.interaction_events,
        );
        let (mut outcomes, changed_surfaces, mut lifecycle_events) = match reduction {
            Ok(reduction) => reduction,
            Err(error) => {
                return self.reject_input_prefix(error);
            }
        };

        self.events.append(&mut lifecycle_events);
        if let Err(error) = self
            .candidate
            .settle_presented_pending_mutation(&mut self.events, &mut self.interaction_events)
        {
            return self.reject_input_prefix(error);
        }
        if let Err(error) = self
            .candidate
            .reconcile_interaction_after_ordered_presentation_observation(
                cause,
                &changed_surfaces,
                &mut self.interaction_events,
            )
        {
            return self.reject_input_prefix(error);
        }
        self.presentation_observation_outcomes.append(&mut outcomes);
        self.last_causal_cause = Some(cause);
        if let Err(error) = self.refresh_presentation_snapshot() {
            return self.reject_input_prefix(error);
        }
        self.refresh_presented_interaction_authority();
        self.application_base = self.candidate.version;
        Ok(())
    }

    /// Returns the sole pointer-provider incarnation frozen at seal. `None`
    /// means this frame uses no journal-based pointer protocol.
    #[must_use]
    pub const fn pointer_provider(&self) -> Option<PointerInputLease> {
        self.pointer_provider
    }

    /// Stages one contiguous desktop-global pointer journal segment and
    /// freezes the exact receiver candidate roster adapters must answer.
    ///
    /// The journal is validated against a frame-local speculative ledger. Its
    /// watermark and edge tickets are not published until `finish` has also
    /// validated every exact receiver receipt against the outputs current at
    /// this segment's causal reduction position. Callers may submit another
    /// segment after staging this segment's receipts, with semantic inputs in
    /// between; the core assigns each edge-bearing segment one ordinal in that
    /// shared lane. An empty provider checkpoint advances only provider-owned
    /// authority and consumes no synthetic raw-event ordinal.
    ///
    /// # Errors
    ///
    /// Returns a structural error and poisons this frame when the provider,
    /// watermark, edge scope, or candidate roster is invalid. A surface-local
    /// lease is rejected even when it was copied from a valid affine producer.
    #[doc(hidden)]
    #[cfg(test)]
    pub fn submit_pointer_journal(
        &mut self,
        provider: PointerInputLease,
        journal: PointerEdgeJournal,
    ) -> Result<(), CoreHostFrameError> {
        self.submit_pointer_journal_inner(
            provider,
            journal,
            PointerJournalSubmissionAuthority::RawLease,
            None,
        )
    }

    /// Stages one surface-local journal through its affine producer capability.
    ///
    /// Renderer adapters should prefer this boundary over transporting a copied
    /// lease directly. Consuming the producer into a drain receipt then makes
    /// additional surface-local submission impossible by construction.
    ///
    /// # Errors
    ///
    /// Returns an error when the producer, journal ordering, or complete
    /// frame checkpoint does not match this frame's frozen pointer authority.
    pub fn submit_surface_pointer_journal(
        &mut self,
        provider: &SurfaceLocalPointerProvider,
        journal: PointerEdgeJournal,
    ) -> Result<(), CoreHostFrameError> {
        self.submit_pointer_journal_inner(
            provider.lease(),
            journal,
            PointerJournalSubmissionAuthority::SurfaceLocalProducer,
            Some(provider),
        )
    }

    fn submit_pointer_journal_inner(
        &mut self,
        provider: PointerInputLease,
        journal: PointerEdgeJournal,
        authority: PointerJournalSubmissionAuthority,
        surface_provider: Option<&SurfaceLocalPointerProvider>,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self.input_terminated_for_overload {
            return self.reject(CoreHostFrameError::InputAfterOverloadCancellation);
        }
        if self.backend_ingress.is_some()
            && authority != PointerJournalSubmissionAuthority::BackendIngress
        {
            return self.reject(CoreHostFrameError::BackendIngressRequired);
        }
        if provider.scope().is_local()
            && authority != PointerJournalSubmissionAuthority::SurfaceLocalProducer
        {
            return self
                .reject(CoreHostFrameError::SurfaceLocalPointerProducerRequired { provider });
        }
        if self.configuration_phase_started {
            return self.reject(CoreHostFrameError::SemanticAfterConfiguration);
        }
        if self.presentation_phase_started {
            return self.reject(CoreHostFrameError::InputAfterPresentation);
        }
        let Some(expected) = self.pointer_provider else {
            return self.reject(CoreHostFrameError::PointerJournalUnexpected);
        };
        if provider != expected {
            return self.reject(CoreHostFrameError::PointerJournalProviderMismatch {
                expected,
                submitted: provider,
            });
        }
        if self.pending_pointer_segment.is_some() {
            return self
                .reject(CoreHostFrameError::PointerReceiverReceiptsMissingBeforeNextSegment);
        }
        if journal.edges().len() > 1 {
            return self.reject(CoreHostFrameError::PointerJournalMustBeEdgewise);
        }
        let mut pending_surface_commit = None;
        if authority == PointerJournalSubmissionAuthority::SurfaceLocalProducer {
            let surface_provider =
                surface_provider.expect("surface-local submission authority retains its producer");
            if let Some(commit) = self.surface_pointer_commit.as_mut() {
                if let Err(source) =
                    commit.extend(surface_provider, journal.previous(), journal.through())
                {
                    return self.reject(CoreHostFrameError::SurfaceLocalPointerProducerRejected {
                        source,
                    });
                }
            } else {
                pending_surface_commit = match surface_provider
                    .begin_frame_submission(journal.previous(), journal.through())
                {
                    Ok(commit) => Some(commit),
                    Err(source) => {
                        return self.reject(
                            CoreHostFrameError::SurfaceLocalPointerProducerRejected { source },
                        );
                    }
                };
            }
        }
        let prepared = match self
            .staged_pointer_journal
            .prepare_candidate(provider, journal.clone())
        {
            Ok(prepared) => prepared,
            Err(source) => {
                return self.reject(CoreHostFrameError::PointerJournalRejected { source });
            }
        };
        let commit = match self.staged_pointer_journal.commit_prepared(prepared) {
            Ok(commit) => commit,
            Err(source) => {
                return self.reject(CoreHostFrameError::PointerJournalRejected { source });
            }
        };
        debug_assert_eq!(journal.edges().len(), commit.accepted_edges().len());
        let mut specs = Vec::with_capacity(journal.edges().len());
        let mut handoff = None;
        for (edge, accepted) in journal.edges().iter().zip(commit.accepted_edges()) {
            handoff = self
                .candidate
                .prepare_native_handoff_projection(accepted.stream(), edge);
            let edge_view = handoff.map(|projection| projection.edge_view(edge));
            let effective = edge_view.as_ref().unwrap_or(edge);
            let spec = if let Some(projection) =
                handoff.filter(|projection| projection.upgrades_owner())
            {
                self.candidate
                    .native_handoff_candidate_spec(projection, edge)
            } else {
                pointer_receiver_candidate_spec(&self.candidate, effective, accepted.stream())
            };
            specs.push(spec.with_metadata(edge.metadata()));
        }
        let attempt = match self
            .pointer_receiver_attempt_issuer
            .as_ref()
            .expect("a live pointer provider retains its receiver attempt issuer")
            .issue(provider)
        {
            Ok(attempt) => attempt,
            Err(source) => {
                return self.reject(CoreHostFrameError::PointerReceiverAttempt { source });
            }
        };
        let candidates = match PointerReceiverCandidateRoster::freeze_referenced_outputs(
            attempt,
            specs,
            &self.frozen_pointer_outputs,
        ) {
            Ok(candidates) => candidates,
            Err(source) => {
                return self.reject(CoreHostFrameError::PointerReceiverRoster { source });
            }
        };
        let stamp = CoreCausalStamp(self.next_causal_ordinal);
        if !journal.edges().is_empty() {
            let Some(next_causal_ordinal) = self.next_causal_ordinal.checked_add(1) else {
                return self.reject(CoreHostFrameError::CausalOrdinalExhausted);
            };
            self.next_causal_ordinal = next_causal_ordinal;
        }
        self.pending_pointer_segment = Some(HostPointerProtocolSegment::new(
            stamp, provider, journal, candidates, handoff,
        ));
        if let Some(commit) = pending_surface_commit {
            self.surface_pointer_commit = Some(commit);
        }
        self.pointer_segment_submitted = true;
        Ok(())
    }

    /// Returns the exact candidate roster for the most recently staged segment.
    ///
    /// The returned IDs are opaque and are valid only for this one host-frame
    /// attempt. An adapter must echo every candidate once in its receipt batch.
    #[must_use]
    pub fn pointer_receiver_candidates(&self) -> Option<&PointerReceiverCandidateRoster> {
        self.pending_pointer_segment
            .as_ref()
            .map(HostPointerProtocolSegment::candidates)
    }

    /// Stages the complete exact receiver receipt batch for the most recently
    /// frozen pointer segment.
    ///
    /// Presentation observations have already reduced before this method is
    /// available. Final output-authority validation still occurs in `finish`
    /// at the segment's causal position, so a subsequently retired output
    /// fails the whole frame.
    pub fn submit_pointer_receiver_receipts(
        &mut self,
        receipts: PointerReceiverReceiptBatch,
    ) -> Result<(), CoreHostFrameError> {
        self.submit_pointer_receiver_receipts_inner(receipts, false)
    }

    fn submit_pointer_receiver_receipts_inner(
        &mut self,
        receipts: PointerReceiverReceiptBatch,
        from_backend_ingress: bool,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self.backend_ingress.is_some() && !from_backend_ingress {
            return self.reject(CoreHostFrameError::BackendIngressRequired);
        }
        if self.pointer_provider.is_none() {
            return self.reject(CoreHostFrameError::PointerReceiverReceiptsUnexpected);
        }
        if self.presentation_phase_started {
            return self.reject(CoreHostFrameError::InputAfterPresentation);
        }
        let Some(segment) = self.pending_pointer_segment.take() else {
            return self.reject(CoreHostFrameError::PointerReceiverReceiptBeforeJournal);
        };
        let presentation_before = PointerPresentationFence::capture(&self.candidate);
        let protocol = segment.into_prepared(receipts);
        let reduction = self.candidate.reduce_host_pointer_protocol(
            self.tick,
            protocol,
            &self.frozen_pointer_outputs,
            &self.frozen_pointer_presentations,
            &self.tick_policy,
            &mut self.events,
            &mut self.interaction_events,
        );
        let reduced = match reduction {
            Ok(reduced) => reduced,
            Err(error) => {
                return self.reject_input_prefix(error);
            }
        };
        let segment_cause = reduced
            .last()
            .map(crate::engine::transition::ReducedPointerEdge::cause);
        if let Some(cause) = segment_cause
            && let Err(error) = self
                .candidate
                .reconcile_pointer_provider_scope(cause, &mut self.interaction_events)
        {
            return self.reject_input_prefix(error);
        }
        self.last_causal_cause = segment_cause.or(self.last_causal_cause);
        self.vacancy_ledger.observe_bindings(&self.candidate);
        if let Err(error) =
            self.refresh_presentation_after_pointer_segment(presentation_before, &reduced)
        {
            return self.reject_input_prefix(error);
        }
        self.reduced_pointer_edges.extend(reduced);
        Ok(())
    }

    pub(super) fn ensure_backend_ingress_complete(&mut self) -> Result<(), CoreHostFrameError> {
        if self.backend_ingress.is_some()
            && (!self.backend_ingress_batch_submitted
                || !self.backend_ingress_complete
                || self.pending_backend_ingress.is_some())
        {
            return self.reject(CoreHostFrameError::BackendIngressIncomplete);
        }
        Ok(())
    }

    pub(super) fn ensure_input_prefix_complete(&mut self) -> Result<(), CoreHostFrameError> {
        self.ensure_backend_ingress_complete()?;
        if let Some(provider) = self.pointer_provider {
            if self.backend_ingress.is_none() && !self.pointer_segment_submitted {
                return self.reject_value(
                    CoreHostFrameError::PointerJournalMissingBeforePresentation { provider },
                );
            }
            if self.pending_pointer_segment.is_some() {
                return self.reject_value(
                    CoreHostFrameError::PointerReceiverReceiptsMissingBeforePresentation,
                );
            }
        }
        Ok(())
    }
}
