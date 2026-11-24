//! Observation prelude and initialization of the transactional frame.

use super::*;

impl CoreHostFramePrelude {
    pub(in crate::engine) fn new(
        engine: &DockEngine,
        presentation_host: PresentationHostLease,
        presentation_observers: BTreeSet<PresentationHostLease>,
    ) -> Result<Self, EngineError> {
        engine
            .presentation_authority
            .presentation
            .validate_lease(presentation_host)
            .map_err(presentation_ledger_error)?;
        let mut presentation_scopes = BTreeMap::new();
        for observer in presentation_observers {
            let scope = engine
                .presentation_authority
                .presentation
                .pending_stream_scope(observer)
                .map_err(presentation_ledger_error)?;
            presentation_scopes.insert(observer, scope);
        }
        Ok(Self {
            authority_domain: engine.authority_domain,
            presentation_host,
            predecessor_tick: engine.last_reducer_tick,
            presentation_host_frontier: engine.presentation_authority.presentation.host_frontier(),
            platform_provider_frontier: engine.viewport.platform_provider_frontier(),
            presentation_scopes,
            presentation_observations: BTreeMap::new(),
            item_identity_scope: None,
            native_binding_allocation_budget: None,
            poison: None,
        })
    }

    /// Limits new native lifetimes throughout this unpublished candidate,
    /// including observation-driven recovery and accepted pointer releases.
    pub(crate) fn set_native_binding_allocation_budget(&mut self, remaining: Option<usize>) {
        self.native_binding_allocation_budget = remaining;
    }

    /// Freezes the application identities admitted by a durable document session.
    #[cfg(feature = "serde")]
    pub(crate) fn restrict_item_identity_scope(&mut self, items: Arc<BTreeSet<ItemId>>) {
        self.item_identity_scope = Some(items);
    }

    /// Returns the exact pending presentation streams frozen for the rendering host.
    #[must_use]
    pub fn pending_presentation_streams(
        &self,
    ) -> impl ExactSizeIterator<Item = HostPresentationStreamId> + '_ {
        self.presentation_scopes[&self.presentation_host]
            .iter()
            .copied()
    }

    /// Returns the exact pending presentation streams for one enrolled observer.
    #[cfg(test)]
    pub fn pending_presentation_streams_for(
        &self,
        presentation_host: PresentationHostLease,
    ) -> Result<Vec<HostPresentationStreamId>, CoreHostFrameError> {
        self.presentation_scopes
            .get(&presentation_host)
            .map(|scope| scope.iter().copied().collect())
            .ok_or(
                CoreHostFrameError::PresentationObservationHostOutsideScope {
                    host: presentation_host,
                },
            )
    }

    /// Submits the rendering host's exact presentation observation batch.
    pub fn submit_presentation_observation(
        &mut self,
        observation: HostPresentationObservation,
    ) -> Result<(), CoreHostFrameError> {
        self.submit_presentation_observation_for(self.presentation_host, observation)
    }

    /// Submits one exact observation batch for an explicitly enrolled host.
    pub fn submit_presentation_observation_for(
        &mut self,
        presentation_host: PresentationHostLease,
        observation: HostPresentationObservation,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self
            .presentation_observations
            .contains_key(&presentation_host)
        {
            let error = if presentation_host == self.presentation_host {
                CoreHostFrameError::DuplicatePresentationObservation
            } else {
                CoreHostFrameError::DuplicatePresentationObservationForHost {
                    host: presentation_host,
                }
            };
            return self.reject(error);
        }
        let Some(scope) = self.presentation_scopes.get(&presentation_host).cloned() else {
            return self.reject(
                CoreHostFrameError::PresentationObservationHostOutsideScope {
                    host: presentation_host,
                },
            );
        };
        if let HostPresentationObservation::Batch(entries) = &observation {
            let mut submitted = BTreeSet::new();
            for entry in entries {
                if !submitted.insert(entry.stream()) {
                    return self.reject(
                        CoreHostFrameError::DuplicatePresentationObservationStream {
                            stream: entry.stream(),
                        },
                    );
                }
            }
            if let Some(stream) = scope
                .iter()
                .copied()
                .find(|stream| !submitted.contains(stream))
            {
                return self
                    .reject(CoreHostFrameError::MissingPresentationObservationStream { stream });
            }
            if let Some(stream) = submitted
                .iter()
                .copied()
                .find(|stream| !scope.contains(stream))
            {
                return self.reject(
                    CoreHostFrameError::PresentationObservationStreamOutsideScope { stream },
                );
            }
        }
        self.presentation_observations
            .insert(presentation_host, observation);
        Ok(())
    }

    /// Reduces all observations into one private rollback candidate and freezes
    /// the post-observation host-frame authority.
    ///
    /// # Errors
    ///
    /// Returns an error when the prelude is incomplete or stale, observation
    /// reduction fails, or the resulting pointer provider cannot be admitted
    /// against the sealed host, surface, and binding scope.
    pub fn seal(self, engine: &DockEngine) -> Result<CoreHostFrame, EngineError> {
        engine.seal_host_frame(self)
    }

    fn reject(&mut self, error: CoreHostFrameError) -> Result<(), CoreHostFrameError> {
        let retained = self.poison.get_or_insert(error).clone();
        Err(retained)
    }
}

impl DockEngine {
    /// Begins the observation-only prelude of one core-owned host frame.
    ///
    /// The returned capability freezes only domain, predecessor, host identity,
    /// platform-provider authority frontier, and pending presentation-stream
    /// scope. The caller must submit every enrolled observation and call
    /// [`CoreHostFramePrelude::seal`] before any scene, pointer, contribution,
    /// or semantic-input authority exists.
    ///
    /// # Errors
    ///
    /// Returns an error when `presentation_host` was not minted by this exact
    /// engine authority domain.
    pub fn begin_host_frame(
        &self,
        presentation_host: PresentationHostLease,
    ) -> Result<CoreHostFramePrelude, EngineError> {
        self.begin_host_frame_with_observers(presentation_host, [])
    }

    /// Begins one core-owned host frame with an explicit rendering host and
    /// supplementary presentation observers.
    ///
    /// The rendering host is the only participant permitted to submit the
    /// complete contribution roster or stage current output. Each observer is
    /// frozen into the same reducer boundary solely so it can settle its own
    /// delayed presentation stream. This prevents a superseded host from
    /// fabricating a second render pass merely to retire its old output.
    ///
    /// # Errors
    ///
    /// Returns an error when an observer repeats, names the rendering host, or
    /// was not minted by this exact engine authority domain.
    pub fn begin_host_frame_with_observers(
        &self,
        presentation_host: PresentationHostLease,
        observers: impl IntoIterator<Item = PresentationHostLease>,
    ) -> Result<CoreHostFramePrelude, EngineError> {
        if let Some(provider) = self.pointer_journal.abandoned_surface_local_provider() {
            return Err(EngineError::SurfaceLocalPointerProviderAbandoned { provider });
        }
        let mut presentation_observers = BTreeSet::from([presentation_host]);
        for observer in observers {
            if observer == presentation_host {
                return Err(EngineError::HostFrameObserverIsRenderingHost { host: observer });
            }
            if !presentation_observers.insert(observer) {
                return Err(EngineError::HostFrameObserverDuplicate { host: observer });
            }
        }
        CoreHostFramePrelude::new(self, presentation_host, presentation_observers)
    }

    /// Seals an observation-only prelude into one rollbackable host-frame candidate.
    fn seal_host_frame(&self, prelude: CoreHostFramePrelude) -> Result<CoreHostFrame, EngineError> {
        if let Some(source) = prelude.poison {
            return Err(EngineError::HostFramePoisoned { source });
        }
        if prelude.authority_domain != self.authority_domain {
            return Err(EngineError::HostFrameAuthorityDomainMismatch {
                expected: self.authority_domain,
                submitted: prelude.authority_domain,
            });
        }
        let current_host_frontier = self.presentation_authority.presentation.host_frontier();
        if prelude.presentation_host_frontier != current_host_frontier {
            return Err(EngineError::HostFramePresentationHostFrontierStale {
                submitted: prelude.presentation_host_frontier,
                current: current_host_frontier,
            });
        }
        let current_platform_provider_frontier = self.viewport.platform_provider_frontier();
        if prelude.platform_provider_frontier != current_platform_provider_frontier {
            return Err(EngineError::HostFramePlatformProviderFrontierStale {
                submitted: prelude.platform_provider_frontier.get(),
                current: current_platform_provider_frontier.get(),
            });
        }
        for host in prelude.presentation_scopes.keys().copied() {
            self.presentation_authority
                .presentation
                .validate_lease(host)
                .map_err(presentation_ledger_error)?;
        }
        if prelude.predecessor_tick != self.last_reducer_tick {
            return Err(EngineError::HostFramePredecessorStale {
                submitted: prelude.predecessor_tick,
                current: self.last_reducer_tick,
            });
        }

        let mut observation_batches = Vec::with_capacity(prelude.presentation_scopes.len());
        for (host, scope) in &prelude.presentation_scopes {
            let current_scope = self
                .presentation_authority
                .presentation
                .pending_stream_scope(*host)
                .map_err(presentation_ledger_error)?;
            if *scope != current_scope {
                if *host == prelude.presentation_host {
                    return Err(EngineError::HostFramePresentationScopeStale {
                        submitted: scope.iter().copied().collect(),
                        current: current_scope.into_iter().collect(),
                    });
                }
                return Err(EngineError::HostFrameSupplementaryPresentationScopeStale {
                    host: *host,
                    submitted: scope.iter().copied().collect(),
                    current: current_scope.into_iter().collect(),
                });
            }
            let observation = prelude
                .presentation_observations
                .get(host)
                .cloned()
                .ok_or_else(|| {
                    if *host == prelude.presentation_host {
                        EngineError::HostFramePresentationObservationMissing
                    } else {
                        EngineError::HostFrameSupplementaryPresentationObservationMissing {
                            host: *host,
                        }
                    }
                })?;
            observation_batches.push((*host, scope.clone(), observation));
        }

        let admission_workspace = self.version;
        let admission_requirements = self
            .presentation_authority
            .presentation_requirements
            .revision();
        let admission_surface_scope = self
            .presentation_authority
            .presentation_requirements
            .surfaces()
            .map(|(surface, _)| surface)
            .collect::<BTreeSet<_>>();
        let runtime_retention_revision = self.runtime_retention_revision;
        let mut candidate = self.candidate();
        candidate
            .viewport
            .set_native_binding_allocation_budget(prelude.native_binding_allocation_budget);
        let vacancy_ledger = TickVacancyLedger::capture(&candidate);
        let tick = candidate
            .last_reducer_tick
            .checked_next()
            .ok_or(EngineError::ReducerTickExhausted)?;
        candidate.last_reducer_tick = tick;
        let mut presentation_observation_outcomes = Vec::new();
        let mut changed_surfaces = BTreeSet::new();
        let mut observation_events = Vec::new();
        let mut observation_interaction_events = Vec::new();
        for (host, scope, observation) in observation_batches {
            let (mut outcomes, changed, mut lifecycle_events) = candidate
                .reduce_presentation_observation(
                    ReductionCause::SurfacePresentationObservationBatch { tick },
                    host,
                    &scope,
                    observation,
                    &mut observation_interaction_events,
                )?;
            presentation_observation_outcomes.append(&mut outcomes);
            changed_surfaces.extend(changed);
            observation_events.append(&mut lifecycle_events);
        }
        candidate.settle_presented_pending_mutation(
            &mut observation_events,
            &mut observation_interaction_events,
        )?;
        candidate.reconcile_interaction_after_presentation_observations(
            tick,
            &changed_surfaces,
            &mut observation_interaction_events,
        )?;

        CoreHostFrame::from_observed_candidate(
            prelude,
            candidate,
            tick,
            runtime_retention_revision,
            admission_workspace,
            admission_requirements,
            admission_surface_scope,
            vacancy_ledger,
            presentation_observation_outcomes,
            observation_events,
            observation_interaction_events,
        )
    }
}

impl CoreHostFrame {
    pub(in crate::engine) fn from_observed_candidate(
        prelude: CoreHostFramePrelude,
        candidate: DockEngine,
        tick: ReducerTickId,
        runtime_retention_revision: u64,
        admission_workspace: WorkspaceVersion,
        admission_requirements: RequirementRevision,
        admission_surface_scope: BTreeSet<SurfaceId>,
        vacancy_ledger: TickVacancyLedger,
        presentation_observation_outcomes: Vec<HostPresentationObservationOutcome>,
        observation_events: Vec<WorkspaceEvent>,
        observation_interaction_events: Vec<InteractionEvent>,
    ) -> Result<Self, EngineError> {
        let CoreHostFramePrelude {
            authority_domain,
            presentation_host,
            predecessor_tick,
            presentation_host_frontier,
            platform_provider_frontier,
            presentation_scopes,
            presentation_observations: _,
            item_identity_scope,
            native_binding_allocation_budget: _,
            poison: _,
        } = prelude;
        let frozen_presentation_roster = HostPresentationRoster::capture(&candidate)?;
        let pointer_provider = candidate.pointer_journal.active_lease();
        if let Some(provider) = pointer_provider {
            candidate.validate_pointer_provider_scope(provider.scope())?;
        }
        if let Some(host) = pointer_provider.and_then(|lease| lease.scope().local_host())
            && host != presentation_host
        {
            return Err(EngineError::PointerProviderHostOutsideFrameScope {
                provider: pointer_provider.expect("surface-local provider was present"),
                host,
            });
        }
        let backend_ingress = candidate.backend_ingress.active();
        if let Some(ingress) = backend_ingress {
            if ingress.presentation_host() != presentation_host {
                return Err(EngineError::BackendIngress {
                    source: BackendIngressError::PresentationHostLeaseMismatch {
                        expected: ingress.presentation_host(),
                        submitted: presentation_host,
                    },
                });
            }
            let platform = candidate.platform_provider();
            if platform != Some(ingress.platform_provider())
                || pointer_provider != Some(ingress.pointer_provider())
            {
                return Err(EngineError::BackendIngressProviderMismatch {
                    ingress,
                    platform,
                    pointer: pointer_provider,
                });
            }
        }
        let presentation_attempt = candidate
            .presentation_authority
            .issue_host_presentation_attempt()
            .map_err(|source| EngineError::HostFramePoisoned { source })?;
        let presentation_obligations = HostPresentationObligationSet::new(
            presentation_attempt,
            frozen_presentation_roster.clone(),
        );
        let tick_policy = candidate.policy.clone();
        let application_base = candidate.version;
        let mut frame = Self {
            platform_effect_delivery_limit: None,
            authority_domain,
            presentation_host,
            workspace: admission_workspace,
            requirements: admission_requirements,
            predecessor_tick,
            presentation_host_frontier,
            platform_provider_frontier,
            runtime_retention_revision,
            tick,
            admission_surface_scope,
            frozen_presentation_roster,
            presentation_obligations,
            backend_ingress,
            backend_ingress_batch_submitted: false,
            backend_ingress_complete: false,
            backend_ingress_commit_guard: None,
            pending_backend_ingress: None,
            pointer_provider,
            staged_pointer_journal: candidate.pointer_journal.clone(),
            surface_pointer_commit: None,
            frozen_pointer_outputs: BTreeMap::new(),
            frozen_pointer_presentations: BTreeMap::new(),
            #[cfg(test)]
            frozen_prepared_presentations: BTreeMap::new(),
            pointer_receiver_attempt_issuer: pointer_provider
                .map(|_| Arc::clone(&candidate.pointer_receiver_attempt_issuer)),
            pending_pointer_segment: None,
            pointer_segment_submitted: false,
            presentation_scopes,
            item_identity_scope,
            presentation_snapshot_changed: false,
            presentation_projection_changed: false,
            candidate,
            tick_policy,
            vacancy_ledger,
            application_base,
            presentation_observation_outcomes,
            reduced_inputs: Vec::new(),
            reduced_pointer_edges: Vec::new(),
            events: observation_events,
            interaction_events: observation_interaction_events,
            last_reduced_input: None,
            last_causal_cause: None,
            staged_presentation_outputs: Vec::new(),
            staged_presentation_output_surfaces: BTreeSet::new(),
            surface_contributions: BTreeMap::new(),
            compiled_surface_outcomes: BTreeMap::new(),
            presentation_phase_started: false,
            configuration_phase_started: false,
            input_terminated_for_overload: false,
            next_causal_ordinal: 0,
            poison: None,
        };
        frame.refresh_presented_interaction_authority();
        Ok(frame)
    }
}
