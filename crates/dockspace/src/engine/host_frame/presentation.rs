//! Candidate measurement, output obligations and presentation staging.

use super::*;

impl CoreHostFrame {
    pub(crate) fn capture_encoding_checkpoint(
        &mut self,
    ) -> Result<CoreEncodingCheckpoint, CoreHostFrameError> {
        self.begin_presentation_phase()?;
        Ok(CoreEncodingCheckpoint {
            presentation: self.candidate.presentation_authority.clone(),
            interaction: self.candidate.interaction.clone(),
            roster: self.frozen_presentation_roster.clone(),
            snapshot_changed: self.presentation_snapshot_changed,
            projection_changed: self.presentation_projection_changed,
        })
    }

    pub(crate) fn restart_encoding_from(
        &mut self,
        checkpoint: &CoreEncodingCheckpoint,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        let mut presentation = checkpoint.presentation.clone();
        presentation
            .scene
            .preserve_revision_frontier(&self.candidate.presentation_authority.scene);
        presentation.last_presentation_output_serial =
            presentation.last_presentation_output_serial.max(
                self.candidate
                    .presentation_authority
                    .last_presentation_output_serial,
            );
        self.candidate.presentation_authority = presentation;
        self.candidate.interaction = checkpoint.interaction.clone();
        self.frozen_presentation_roster = checkpoint.roster.clone();
        self.presentation_snapshot_changed = checkpoint.snapshot_changed;
        self.presentation_projection_changed = checkpoint.projection_changed;
        self.surface_contributions.clear();
        self.compiled_surface_outcomes.clear();
        self.staged_presentation_outputs.clear();
        self.staged_presentation_output_surfaces.clear();
        Ok(())
    }

    pub(crate) fn capture_host_focus_observation(
        &self,
        surface: SurfaceId,
        generation: crate::platform::focus::PaneFocusObservationGeneration,
        state: crate::platform::focus::HostFocusState,
    ) -> Option<crate::platform::focus::HostFocusObservation> {
        let (stream, host, _) = self
            .candidate
            .presentation_authority
            .presentation
            .active_surface_scope(surface)?;
        (host == self.presentation_host && self.candidate.workspace.surface(surface).is_some())
            .then(|| {
                crate::platform::focus::HostFocusObservation::new(
                    generation, host, stream, surface, state,
                )
            })
    }

    pub(crate) const fn presentation_attempt(&self) -> HostPresentationAttemptId {
        self.presentation_obligations.attempt()
    }

    pub(super) fn refresh_presentation_snapshot(&mut self) -> Result<(), EngineError> {
        let roster = HostPresentationRoster::capture(&self.candidate)?;
        self.presentation_projection_changed |= !self
            .frozen_presentation_roster
            .same_semantic_projection(&roster);
        self.presentation_snapshot_changed |= self.frozen_presentation_roster != roster;
        self.frozen_presentation_roster = roster;
        self.retain_presented_interaction_authority_in_roster();
        Ok(())
    }

    pub(crate) fn refresh_presentation_layout(&mut self) -> Result<(), CoreHostFrameError> {
        self.refresh_presentation_layout_except(None)
    }

    pub(crate) fn refresh_presentation_layout_except(
        &mut self,
        freshly_measured: Option<SurfaceId>,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        if self.presentation_phase_started {
            return Ok(());
        }
        let refreshed = self.candidate.refresh_presentation_layouts(
            self.tick,
            freshly_measured,
            &mut self.interaction_events,
        );
        match refreshed {
            Ok(false) => Ok(()),
            Ok(true) => match self.refresh_presentation_snapshot() {
                Ok(()) => Ok(()),
                Err(error) => self.reject_input_prefix(error),
            },
            Err(error) => self.reject_input_prefix(error),
        }
    }

    pub(super) fn try_refresh_presentation_surfaces(
        &mut self,
        dirty_surfaces: impl IntoIterator<Item = SurfaceId>,
    ) -> bool {
        let Some((snapshot_changed, projection_changed)) = self
            .frozen_presentation_roster
            .try_refresh_surfaces(&self.candidate, dirty_surfaces)
        else {
            return false;
        };
        self.presentation_snapshot_changed |= snapshot_changed;
        self.presentation_projection_changed |= projection_changed;
        true
    }

    pub(super) fn refresh_presented_interaction_authority(&mut self) {
        self.frozen_pointer_outputs.clear();
        self.frozen_pointer_presentations.clear();
        #[cfg(test)]
        self.frozen_prepared_presentations.clear();
        let pointer_surface = self
            .pointer_provider
            .and_then(|lease| lease.scope().surface_local())
            .map(|scope| scope.surface());
        for surface in self.frozen_presentation_roster.surfaces() {
            let Some(projection) = self
                .candidate
                .presentation_authority
                .scene
                .interaction_projection(surface)
            else {
                continue;
            };
            #[cfg(test)]
            self.frozen_prepared_presentations.insert(
                surface,
                JournalSurfacePresentation::from_interaction(projection),
            );
            if pointer_surface.is_none_or(|expected| expected == surface) {
                self.frozen_pointer_outputs.insert(
                    surface,
                    PointerReceiverPresentedOutput::from_interaction(projection),
                );
                self.frozen_pointer_presentations.insert(
                    surface,
                    JournalSurfacePresentation::from_interaction(projection),
                );
            }
        }
    }

    /// Issues the complete affine physical-presentation roster for this frame.
    ///
    /// Every returned ticket must be consumed exactly once by the
    /// presentation capability or a paired retained contribution. Issuance
    /// closes semantic input for this host frame.
    pub(in crate::engine) fn issue_presentation_obligations(
        &mut self,
    ) -> Result<Vec<HostPresentationObligation>, CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        self.ensure_backend_ingress_complete()?;
        self.presentation_phase_started = true;
        if let Err(error) = self
            .presentation_obligations
            .replace_unissued_roster(self.frozen_presentation_roster.clone())
        {
            return self.reject_value(error);
        }
        match self.presentation_obligations.issue() {
            Ok(obligations) => Ok(obligations),
            Err(error) => self.reject_value(error),
        }
    }

    /// Consumes one exact physical presentation ticket.
    ///
    /// `Painted` stages a core presentation emission. `Unavailable` explicitly
    /// settles the slot without creating presentation authority.
    pub(in crate::engine) fn settle_presentation_obligation(
        &mut self,
        obligation: HostPresentationObligation,
        disposition: HostPresentationDisposition,
    ) -> Result<Option<HostPresentationEmissionRequest>, CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        let request = match disposition {
            HostPresentationDisposition::Painted(submitted) => {
                let frozen = match self.presentation_obligations.frozen_output(&obligation) {
                    Ok(frozen) => frozen,
                    Err(error) => return self.reject_value(error),
                };
                let expected = frozen.interaction();
                if submitted != expected {
                    return self.reject_value(
                        CoreHostFrameError::PresentationInteractionMismatch {
                            surface: obligation.slot().surface(),
                            expected,
                            submitted,
                        },
                    );
                }
                let (surface, endpoint, payload) = frozen.into_parts();
                Some(self.stage_presentation_payload(
                    surface,
                    endpoint,
                    payload,
                    obligation.attempt(),
                    obligation.ordinal(),
                )?)
            }
            HostPresentationDisposition::Unavailable(_) => {
                if let Err(error) = self.presentation_obligations.validate(&obligation) {
                    return self.reject_value(error);
                }
                None
            }
        };
        self.presentation_obligations
            .resolve(obligation, disposition);
        Ok(request)
    }

    pub(super) fn stage_presentation_output(
        &mut self,
        surface: SurfaceId,
        frozen: FrozenSurfacePresentationOutput,
        obligation: &HostPresentationObligation,
    ) -> Result<HostPresentationEmissionRequest, CoreHostFrameError> {
        self.stage_presentation_payload(
            surface,
            frozen.endpoint(),
            frozen.payload(),
            obligation.attempt(),
            obligation.ordinal(),
        )
    }

    fn stage_presentation_payload(
        &mut self,
        surface: SurfaceId,
        endpoint: HostPresentationEndpoint,
        payload: HostPresentationOutputPayload,
        attempt: HostPresentationAttemptId,
        ordinal: u64,
    ) -> Result<HostPresentationEmissionRequest, CoreHostFrameError> {
        self.ensure_backend_ingress_complete()?;
        self.presentation_phase_started = true;
        if !self.staged_presentation_output_surfaces.insert(surface) {
            return self.reject_value(CoreHostFrameError::DuplicatePresentationOutput { surface });
        }
        let request = HostPresentationEmissionRequest::new(
            self.presentation_host,
            self.predecessor_tick,
            attempt,
            ordinal,
        );
        let scene = if let HostPresentationOutputPayload::Paint { scene: ticket, .. } = payload {
            let Some(scene) = self.candidate.scene().emitted_scene(ticket) else {
                return self
                    .reject_value(CoreHostFrameError::PresentationOutputUnavailable { surface });
            };
            Some(Arc::new(scene.clone()))
        } else {
            None
        };
        self.staged_presentation_outputs
            .push(StagedPresentationOutput {
                request,
                surface,
                endpoint,
                payload,
                scene,
            });
        Ok(request)
    }

    /// Records one actual paint of the frozen Ready candidate and contributes
    /// that exact same candidate without recomputing measurements.
    ///
    /// This is the only retained-contribution path. Pairing the contribution
    /// with a frame-local output request prevents a semantic ticket observed
    /// before paint from being misrepresented as an actual host output. The
    /// request becomes a concrete core emission only after the prepared candidate commits
    /// commits the complete host frame.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostFrameError`] when the token is not for the exact
    /// frozen Ready candidate or when another contribution already answers the
    /// surface slot. Any such error poisons the complete host frame.
    pub(in crate::engine) fn record_painted_surface_contribution(
        &mut self,
        obligation: HostPresentationObligation,
        token: SurfaceContributionToken,
        interaction: HostInteractionPresentation,
    ) -> Result<HostPresentationEmissionRequest, CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        let surface = token.surface();
        if obligation.slot() != (HostPresentationSlot::Surface { surface }) {
            return self.reject_value(CoreHostFrameError::PresentationObligationSurfaceMismatch {
                slot: obligation.slot(),
                surface,
            });
        }
        if !self.frozen_presentation_roster.contains_surface(surface) {
            return self.reject_value(CoreHostFrameError::SurfaceOutsideRoster { surface });
        }
        let frozen = match self.presentation_obligations.frozen_output(&obligation) {
            Ok(frozen) => frozen,
            Err(error) => return self.reject_value(error),
        };
        let HostPresentationOutputPayload::Paint { scene: ticket, .. } = frozen.payload() else {
            return self
                .reject_value(CoreHostFrameError::RetainedContributionNotPaintable { surface });
        };
        let expected_interaction = frozen.payload().interaction().unwrap_or_default();
        if interaction != expected_interaction {
            return self.reject_value(CoreHostFrameError::PresentationInteractionMismatch {
                surface,
                expected: expected_interaction,
                submitted: interaction,
            });
        }
        if token.base() != frozen.stamp() {
            return self.reject_value(CoreHostFrameError::RetainedContributionStampMismatch {
                surface,
                submitted: token.base(),
                expected: frozen.stamp(),
            });
        }
        if !token
            .coordinates
            .same_projection_authority(frozen.coordinates())
        {
            return self.reject_value(CoreHostFrameError::RetainedContributionCoordinateMismatch {
                surface,
            });
        }

        let request = self.stage_presentation_output(surface, frozen, &obligation)?;
        let contribution = PreparedSurfaceContribution {
            token,
            invalidated_resize: None,
            #[cfg(feature = "motion")]
            next_motion: None,
            policy_revision: token.ticket().policy(),
            state: PreparedSurfaceContributionState::Retained { ticket },
        };
        self.push_surface_contribution(contribution)?;
        self.presentation_obligations.resolve(
            obligation,
            HostPresentationDisposition::Painted(interaction),
        );
        Ok(request)
    }

    /// Adds one surface contribution reduced after the complete input prefix.
    ///
    /// Contributions are retained in canonical [`crate::ids::SurfaceId`] order,
    /// independent of adapter callback arrival order. A tick may include at most
    /// one contribution for each logical surface.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostFrameError::DuplicateSurfaceContribution`] without
    /// replacing the already-installed contribution for that surface.
    pub fn push_surface_contribution(
        &mut self,
        contribution: PreparedSurfaceContribution,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.poison {
            return Err(error.clone());
        }
        self.ensure_backend_ingress_complete()?;
        let surface = contribution.surface();
        if self.compiled_surface_outcomes.contains_key(&surface)
            && !matches!(&contribution.state, PreparedSurfaceContributionState::Retained { ticket }
                if self.candidate.scene().ready_candidate(surface).is_some_and(|candidate| candidate.output_ticket() == *ticket))
        {
            return self.reject(CoreHostFrameError::DuplicateSurfaceContribution { surface });
        }
        if !self.frozen_presentation_roster.contains_surface(surface) {
            return self.reject(CoreHostFrameError::SurfaceOutsideRoster { surface });
        }
        self.presentation_phase_started = true;
        let std::collections::btree_map::Entry::Vacant(slot) =
            self.surface_contributions.entry(surface)
        else {
            let error = CoreHostFrameError::DuplicateSurfaceContribution { surface };
            let retained = self.poison.get_or_insert(error).clone();
            return Err(retained);
        };
        slot.insert(contribution);
        Ok(())
    }

    /// Compiles and installs measured geometry in the private update, before encoding.
    /// Publication still happens only when the complete host frame commits.
    pub(crate) fn compile_surface_contribution(
        &mut self,
        contribution: PreparedSurfaceContribution,
    ) -> Result<(), CoreHostFrameError> {
        self.ensure_input_prefix_complete()?;
        self.presentation_phase_started = true;
        let surface = contribution.surface();
        if self.compiled_surface_outcomes.contains_key(&surface)
            || self.surface_contributions.contains_key(&surface)
        {
            return self.reject(CoreHostFrameError::DuplicateSurfaceContribution { surface });
        }
        let outcome = match self
            .candidate
            .reduce_surface_contribution_with_interaction_events(
                self.tick,
                contribution,
                self.candidate.policy.revision(),
                &mut self.interaction_events,
            ) {
            Ok(outcome) => outcome,
            Err(error) => {
                return self.reject_input_prefix(error);
            }
        };
        self.compiled_surface_outcomes.insert(surface, outcome);
        if let Err(error) = self.refresh_presentation_snapshot() {
            return self.reject_input_prefix(error);
        }
        Ok(())
    }

    pub(crate) fn surface_is_measured(&self, surface: SurfaceId) -> bool {
        self.compiled_surface_outcomes.contains_key(&surface)
    }

    pub(crate) fn output_admission_facts(
        &self,
        surface: SurfaceId,
    ) -> (
        &[HostPresentationObservationOutcome],
        crate::presentation::observation::PresentationStreamAdmission,
    ) {
        let endpoint = self
            .candidate
            .scene()
            .ready_candidate(surface)
            .map(|candidate| presentation_endpoint_from_capture(candidate.coordinate_capture()));
        (
            &self.presentation_observation_outcomes,
            endpoint.map_or(
                crate::presentation::observation::PresentationStreamAdmission::New,
                |endpoint| {
                    self.candidate
                        .presentation_authority
                        .presentation
                        .stream_admission(self.presentation_host, surface, endpoint)
                },
            ),
        )
    }

    /// Returns all surface contributions in canonical surface order.
    #[must_use]
    pub fn surface_contributions(
        &self,
    ) -> impl ExactSizeIterator<Item = &PreparedSurfaceContribution> {
        self.surface_contributions.values()
    }

    /// Closes this capability's semantic input prefix in place.
    pub(crate) fn begin_presentation_phase(&mut self) -> Result<(), CoreHostFrameError> {
        self.ensure_input_prefix_complete()?;
        self.refresh_presentation_layout()?;
        // Semantic input in this same frame may have retired the frozen
        // provider after its final segment was reduced. The reducer owns that
        // transition; presentation only requires the frozen segment and its
        // receipts to be complete.
        self.presentation_phase_started = true;
        Ok(())
    }

    /// Closes the semantic input prefix and returns a presentation-only capability.
    pub fn into_presentation(mut self) -> Result<CoreHostPresentationFrame, CoreHostFrameError> {
        self.begin_presentation_phase()?;
        if !self.presentation_obligations.is_issued() {
            if let Err(error) = self
                .presentation_obligations
                .replace_unissued_roster(self.frozen_presentation_roster.clone())
            {
                return self.reject_value(error);
            }
        }
        Ok(CoreHostPresentationFrame { frame: self })
    }
}
