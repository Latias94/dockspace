//! Semantic input, configuration and document admission.

use super::*;

impl CoreHostFrame {
    /// Admits reconciled keys only for this final document-replacement input.
    #[cfg(feature = "serde")]
    pub(crate) fn append_document_replacement(
        &mut self,
        source: StableInputSourceId,
        sequence: SourceSequence,
        input: EngineInput,
        source_scope: &BTreeSet<ItemId>,
        target_scope: Arc<BTreeSet<ItemId>>,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        self.ensure_input_prefix_complete()?;
        if self.configuration_phase_started {
            return self.reject(CoreHostFrameError::SemanticAfterConfiguration);
        }
        if self.presentation_phase_started {
            return self.reject(CoreHostFrameError::InputAfterPresentation);
        }
        if !matches!(input, EngineInput::RestoreWorkspace(_)) {
            return self.reject(CoreHostFrameError::InputPhaseMismatch {
                phase: HostFrameInputPhase::Semantic,
                actual: input.priority(),
            });
        }
        if self.item_identity_scope.as_deref() != Some(source_scope)
            || !target_scope.is_superset(source_scope)
        {
            return self.reject(CoreHostFrameError::ItemIdentityScopeMismatch);
        }
        self.item_identity_scope = Some(target_scope);
        self.append_input(source, sequence, input)
    }

    /// Appends one semantic input in its actual provider/host arrival order.
    ///
    /// `source_sequence` advances the session-owned semantic writer watermark.
    /// `source` remains diagnostic provenance; it neither partitions replay
    /// authority nor reorders this frame.
    pub fn append_input(
        &mut self,
        source: StableInputSourceId,
        source_sequence: SourceSequence,
        input: EngineInput,
    ) -> Result<(), CoreHostFrameError> {
        self.append(
            HostFrameInputPhase::Semantic,
            source,
            source_sequence,
            input,
            false,
        )
    }

    /// Appends one explicit configuration input.
    ///
    /// Configuration closes semantic input and immediately updates this frame's
    /// private candidate. Measurements and paint use the resulting configuration;
    /// publication still waits for the complete frame to commit.
    pub fn append_configuration(
        &mut self,
        source: StableInputSourceId,
        source_sequence: SourceSequence,
        input: EngineInput,
    ) -> Result<(), CoreHostFrameError> {
        self.begin_configuration_phase()?;
        self.append(
            HostFrameInputPhase::Configuration,
            source,
            source_sequence,
            input,
            false,
        )
    }

    pub(crate) fn append_policy_replacement(
        &mut self,
        source: StableInputSourceId,
        source_sequence: SourceSequence,
        policy: DockPolicy,
    ) -> Result<(), CoreHostFrameError> {
        self.append_configuration(
            source,
            source_sequence,
            EngineInput::ReplacePolicy {
                expected: self.application_base,
                policy,
            },
        )
    }

    pub(crate) fn append_presentation_config_replacement(
        &mut self,
        source: StableInputSourceId,
        source_sequence: SourceSequence,
        config: DockPresentationConfig,
    ) -> Result<(), CoreHostFrameError> {
        self.append_configuration(
            source,
            source_sequence,
            EngineInput::ReplacePresentationConfig {
                expected: self.application_base,
                config,
            },
        )
    }

    /// Closes provider and semantic ingress before configuration begins.
    ///
    /// Calling this method is an irreversible type-state transition for this
    /// frame candidate: later semantic or pointer input is rejected. The
    /// configuration itself remains speculative until the complete host frame
    /// commits.
    pub fn begin_configuration_phase(&mut self) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self.input_terminated_for_overload {
            return self.reject(CoreHostFrameError::InputAfterOverloadCancellation);
        }
        if self.presentation_phase_started {
            return self.reject(CoreHostFrameError::InputAfterPresentation);
        }
        self.ensure_input_prefix_complete()?;
        self.configuration_phase_started = true;
        Ok(())
    }

    pub(super) fn append_backend_input(
        &mut self,
        ordinal: BackendIngressOrdinal,
        input: EngineInput,
    ) -> Result<(), CoreHostFrameError> {
        debug_assert!(input.is_backend_ingress_fact());
        self.append(
            HostFrameInputPhase::Semantic,
            BACKEND_INGRESS_INPUT_SOURCE,
            SourceSequence::new(ordinal.get()),
            input,
            true,
        )
    }

    pub(super) fn append_backend_semantic_input(
        &mut self,
        ordinal: BackendIngressOrdinal,
        input: EngineInput,
    ) -> Result<(), CoreHostFrameError> {
        debug_assert!(!input.is_backend_ingress_fact());
        debug_assert!(!input.is_configuration_commit());
        self.append(
            HostFrameInputPhase::Semantic,
            BACKEND_INGRESS_INPUT_SOURCE,
            SourceSequence::new(ordinal.get()),
            input,
            true,
        )
    }

    pub(super) fn append(
        &mut self,
        phase: HostFrameInputPhase,
        source: StableInputSourceId,
        source_sequence: SourceSequence,
        input: EngineInput,
        from_backend_ingress: bool,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if let EngineInput::PublishHostFocusObservation { observation, .. } = &input
            && observation.host() != self.presentation_host
        {
            return self.reject(CoreHostFrameError::HostFocusOwnerMismatch {
                expected: self.presentation_host,
                submitted: observation.host(),
            });
        }
        if self.input_terminated_for_overload {
            return self.reject(CoreHostFrameError::InputAfterOverloadCancellation);
        }
        let overload_cancellation =
            if let EngineInput::CancelNativeInputForOverload { provider } = &input {
                self.ensure_backend_ingress_complete()?;
                let Some(expected) = self.backend_ingress else {
                    return self.reject(CoreHostFrameError::BackendIngressUnexpected);
                };
                if from_backend_ingress
                    || *provider != expected
                    || provider.presentation_host() != self.presentation_host
                {
                    return self.reject(CoreHostFrameError::BackendIngressRejected {
                        source: BackendIngressError::ProviderLeaseMismatch {
                            expected,
                            submitted: *provider,
                        },
                    });
                }
                true
            } else {
                false
            };
        let native_input_reset = matches!(
            &input,
            EngineInput::ObserveNativeInputDeviceRemoval { .. }
                | EngineInput::ObserveNativeInputBindingRetired { .. }
        );
        if self.pending_pointer_segment.is_some() {
            return self.reject(CoreHostFrameError::PointerReceiverReceiptsMissingBeforeInput);
        }
        if matches!(phase, HostFrameInputPhase::Configuration) {
            self.ensure_backend_ingress_complete()?;
        }
        if source == BACKEND_INGRESS_INPUT_SOURCE && !from_backend_ingress {
            return self.reject(CoreHostFrameError::ReservedInputSource {
                input_source: source,
            });
        }
        if self.backend_ingress.is_some()
            && input.is_backend_ingress_fact()
            && !from_backend_ingress
        {
            return self.reject(CoreHostFrameError::BackendIngressRequired);
        }
        let configuration = input.is_configuration_commit();
        if configuration != matches!(phase, HostFrameInputPhase::Configuration) {
            return self.reject(CoreHostFrameError::InputPhaseMismatch {
                phase,
                actual: input.priority(),
            });
        }
        if matches!(phase, HostFrameInputPhase::Semantic) && self.configuration_phase_started {
            return self.reject(CoreHostFrameError::SemanticAfterConfiguration);
        }
        if matches!(phase, HostFrameInputPhase::Semantic) && self.presentation_phase_started {
            return self.reject(CoreHostFrameError::InputAfterPresentation);
        }
        if let Some(item) = self.first_unadmitted_item(&input) {
            return self.reject(CoreHostFrameError::ItemIdentityOutsideScope { item });
        }
        let stamp = CoreCausalStamp(self.next_causal_ordinal);
        let Some(next_causal_ordinal) = self.next_causal_ordinal.checked_add(1) else {
            return self.reject(CoreHostFrameError::CausalOrdinalExhausted);
        };
        self.next_causal_ordinal = next_causal_ordinal;
        if !from_backend_ingress
            && let Err(error) = self
                .candidate
                .validate_and_advance_semantic_input_watermark([(source, source_sequence)])
        {
            return self.reject_input_prefix(error);
        }
        let Some(sequence) = self.candidate.last_input.checked_next() else {
            return self.reject_input_prefix(EngineError::InputSequenceExhausted);
        };
        self.candidate.last_input = sequence;
        let refresh_presented_interaction_authority = matches!(
            &input,
            EngineInput::SetNativeSurfacePlacements { .. }
                | EngineInput::RegisterViewport { .. }
                | EngineInput::BootstrapChildViewport { .. }
                | EngineInput::CreateNativeSurface { .. }
                | EngineInput::AdvanceNativeSurfaceCreate { .. }
                | EngineInput::PublishPlatformSnapshot { .. }
                | EngineInput::PublishNativeCloseObservation { .. }
                | EngineInput::PublishGlobalFocusObservation { .. }
                | EngineInput::ObserveNativeInputDeviceRemoval { .. }
                | EngineInput::ObserveNativeInputBindingRetired { .. }
                | EngineInput::ReplaceWorkspace(_)
                | EngineInput::ReplaceWorkspaceContinuingRoots { .. }
                | EngineInput::RestoreWorkspace(_)
        );
        let input = SequencedInput::new(stamp, sequence, source, source_sequence, input);
        let cause = input.cause(self.tick);
        let before_configuration_resize_surface = configuration
            .then(|| {
                self.candidate
                    .interaction
                    .active_resize_view()
                    .filter(|resize| !resize.updates().is_empty())
                    .map(|resize| resize.surface())
            })
            .flatten();
        let outcome = self.candidate.reduce_sequenced_input(
            input,
            self.tick,
            TickStartAuthority {
                policy: &self.tick_policy,
            },
            &mut self.application_base,
            &mut self.events,
            &mut self.interaction_events,
        );
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return self.reject_input_prefix(error);
            }
        };
        if let Err(error) = self
            .candidate
            .reconcile_pointer_provider_scope(cause, &mut self.interaction_events)
        {
            return self.reject_input_prefix(error);
        }
        if configuration {
            // Configuration may cancel an existing resize, but cannot edit its
            // weights or start a new one. Retain only the surface needed to
            // revoke that preview before compiling new-configuration geometry.
            let after_resize_surface = self
                .candidate
                .interaction
                .active_resize_view()
                .filter(|resize| !resize.updates().is_empty())
                .map(|resize| resize.surface());
            let changed_surfaces = if before_configuration_resize_surface == after_resize_surface {
                BTreeSet::new()
            } else {
                before_configuration_resize_surface
                    .into_iter()
                    .chain(after_resize_surface)
                    .collect()
            };
            let result = self
                .candidate
                .invalidate_uncovered_changed_resize_presentation(
                    &changed_surfaces,
                    sequence,
                    &BTreeSet::new(),
                )
                .and_then(|_| {
                    self.candidate
                        .cancel_revoked_presentation_caused(cause, &mut self.interaction_events)
                });
            if let Err(error) = result {
                return self.reject_input_prefix(error);
            }
        }
        self.vacancy_ledger
            .observe_workspace_reconciliation(outcome.outcome());
        self.vacancy_ledger.observe_bindings(&self.candidate);
        self.last_reduced_input = Some(sequence);
        self.last_causal_cause = Some(cause);
        self.reduced_inputs.push(outcome);
        if let Err(error) = self.candidate.report_reduction(
            &mut self.events,
            &mut self.interaction_events,
            |candidate, _, _| candidate.rebuild_presentation_requirements(sequence),
        ) {
            return self.reject_input_prefix(error);
        }
        if let Err(error) = self.refresh_presentation_snapshot() {
            return self.reject_input_prefix(error);
        }
        if refresh_presented_interaction_authority {
            self.refresh_presented_interaction_authority();
        }
        if overload_cancellation {
            self.input_terminated_for_overload = true;
        }
        if overload_cancellation || native_input_reset {
            self.staged_pointer_journal = self.candidate.pointer_journal.clone();
        }
        Ok(())
    }

    fn first_unadmitted_item(&self, input: &EngineInput) -> Option<ItemId> {
        let scope = self.item_identity_scope.as_ref()?;
        match input {
            EngineInput::ReplaceWorkspace(workspace)
            | EngineInput::ReplaceWorkspaceContinuingRoots { workspace, .. } => workspace
                .item_multiset()
                .into_keys()
                .find(|item| !scope.contains(item)),
            EngineInput::RestoreWorkspace(restore) => restore
                .workspace()
                .item_multiset()
                .into_keys()
                .find(|item| !scope.contains(item)),
            EngineInput::WorkspaceCommand { command, .. } => {
                command.opened_item().filter(|item| !scope.contains(item))
            }
            EngineInput::ProductAction {
                action: crate::model::ProductAction::OpenItem { item, .. },
                ..
            } => (!scope.contains(item)).then_some(*item),
            _ => None,
        }
    }

    pub(super) fn reject(&mut self, error: CoreHostFrameError) -> Result<(), CoreHostFrameError> {
        let retained = self.poison.get_or_insert(error).clone();
        Err(retained)
    }

    pub(super) fn reject_input_prefix(
        &mut self,
        source: EngineError,
    ) -> Result<(), CoreHostFrameError> {
        self.reject(CoreHostFrameError::InputPrefixReductionFailed {
            source: Box::new(source),
        })
    }

    pub(super) fn reject_value<T>(
        &mut self,
        error: CoreHostFrameError,
    ) -> Result<T, CoreHostFrameError> {
        let retained = self.poison.get_or_insert(error).clone();
        Err(retained)
    }
}
