//! Publication and affine prepared-frame commit capabilities.

use super::*;

impl CoreHostFrame {
    /// Prepares this capability without publishing the candidate engine state.
    ///
    /// The returned commit capability keeps the original engine exclusively
    /// borrowed. Dropping it rolls back the complete frame. Commit still
    /// validates the affine surface-local producer attempt before publishing.
    #[cfg(test)]
    pub fn prepare<'a>(
        self,
        engine: &'a mut DockEngine,
    ) -> Result<PreparedHostFrameCommit<'a>, EngineError> {
        engine.prepare_host_frame(self)
    }

    /// Prepares this capability without borrowing its destination engine.
    pub fn prepare_owned(
        self,
        engine: &DockEngine,
    ) -> Result<OwnedPreparedHostFrameCommit, EngineError> {
        engine.prepare_host_frame_owned(self)
    }

    /// Atomically reduces and publishes this capability through its engine.
    #[cfg(test)]
    pub fn finish(self, engine: &mut DockEngine) -> Result<EngineTransition, EngineError> {
        self.prepare(engine)?.commit()
    }
}

impl CoreHostPresentationFrame {
    pub(crate) fn with_platform_effect_delivery_limit(mut self, limit: Option<usize>) -> Self {
        self.frame.platform_effect_delivery_limit = limit;
        self
    }
    #[cfg(feature = "serde")]
    pub(crate) fn item_identity_scope_matches(&self, expected: &BTreeSet<ItemId>) -> bool {
        self.frame.item_identity_scope.as_deref() == Some(expected)
    }

    /// Returns the exact post-input presentation authority.
    #[must_use]
    #[cfg(test)]
    pub const fn view(&self) -> HostFrameView<'_> {
        self.frame.view()
    }

    /// Returns the complete post-input logical surface roster.
    #[must_use]
    #[cfg(test)]
    pub fn surfaces(&self) -> impl ExactSizeIterator<Item = SurfaceId> + '_ {
        self.frame.surfaces()
    }

    /// Issues every post-input physical presentation obligation exactly once.
    pub fn take_presentation_obligations(
        &mut self,
    ) -> Result<Vec<HostPresentationObligation>, CoreHostFrameError> {
        self.frame.issue_presentation_obligations()
    }

    /// Consumes one post-input physical presentation obligation.
    pub fn resolve_presentation_obligation(
        &mut self,
        obligation: HostPresentationObligation,
        disposition: HostPresentationDisposition,
    ) -> Result<Option<HostPresentationEmissionRequest>, CoreHostFrameError> {
        self.frame
            .settle_presentation_obligation(obligation, disposition)
    }

    /// Explicitly declares every unissued physical output unavailable.
    ///
    /// This is an atomic convenience for hosts which know that this entire
    /// presentation pass cannot produce authoritative output. It never
    /// recovers partially checked-out tickets.
    #[cfg(test)]
    pub fn resolve_all_presentation_obligations_unavailable(
        &mut self,
        reason: HostPresentationUnavailableReason,
    ) -> Result<(), CoreHostFrameError> {
        if let Some(error) = &self.frame.poison {
            return Err(error.clone());
        }
        match self
            .frame
            .presentation_obligations
            .resolve_all_unissued(HostPresentationDisposition::Unavailable(reason))
        {
            Ok(()) => Ok(()),
            Err(error) => self.frame.reject(error),
        }
    }

    /// Pairs one actual post-input paint with its exact retained contribution.
    pub fn record_painted_surface_contribution(
        &mut self,
        obligation: HostPresentationObligation,
        token: SurfaceContributionToken,
        interaction: HostInteractionPresentation,
    ) -> Result<HostPresentationEmissionRequest, CoreHostFrameError> {
        self.frame
            .record_painted_surface_contribution(obligation, token, interaction)
    }

    /// Adds one post-input surface contribution.
    #[cfg(test)]
    pub fn push_surface_contribution(
        &mut self,
        contribution: PreparedSurfaceContribution,
    ) -> Result<(), CoreHostFrameError> {
        self.frame.push_surface_contribution(contribution)
    }

    /// Returns contributions staged in canonical surface order.
    #[must_use]
    #[cfg(test)]
    pub fn surface_contributions(
        &self,
    ) -> impl ExactSizeIterator<Item = &PreparedSurfaceContribution> {
        self.frame.surface_contributions()
    }

    /// Prepares the complete candidate without borrowing its destination engine.
    ///
    /// The returned affine capability may cross an enclosing host transaction's
    /// seal boundary. Its later commit revalidates the engine fence captured here.
    pub fn prepare_owned(
        self,
        engine: &DockEngine,
    ) -> Result<OwnedPreparedHostFrameCommit, EngineError> {
        self.frame.prepare_owned(engine)
    }

    /// Atomically publishes the complete candidate.
    #[cfg(test)]
    pub fn finish(self, engine: &mut DockEngine) -> Result<EngineTransition, EngineError> {
        self.frame.finish(engine)
    }
}

#[cfg(test)]
impl PreparedHostFrameCommit<'_> {
    /// Returns the exact transition which will be published on commit.
    #[must_use]
    pub const fn transition(&self) -> &EngineTransition {
        &self.transition
    }

    /// Publishes the already validated candidate.
    ///
    /// # Errors
    ///
    /// Returns a typed producer error without changing the destination engine
    /// if the affine surface-local frame attempt is no longer exact.
    pub fn commit(self) -> Result<EngineTransition, EngineError> {
        let Self {
            engine,
            candidate,
            transition,
            backend_ingress_commit_guard,
            surface_pointer_commit,
        } = self;
        match backend_ingress_commit_guard {
            Some(guard) => guard
                .publish_with(|| {
                    publish_prepared_host_candidate(
                        engine,
                        candidate,
                        transition,
                        surface_pointer_commit,
                    )
                })
                .map_err(|source| EngineError::BackendIngress { source })?,
            None => publish_prepared_host_candidate(
                engine,
                candidate,
                transition,
                surface_pointer_commit,
            ),
        }
    }
}

impl OwnedPreparedHostFrameCommit {
    /// Borrows the final unpublished candidate for runtime storage admission.
    pub(crate) const fn candidate_engine(&self) -> &DockEngine {
        &self.candidate
    }

    pub(crate) fn has_new_undelivered_platform_effects(&self, engine: &DockEngine) -> bool {
        self.candidate
            .viewport
            .effects()
            .has_undelivered_request_after(engine.viewport.latest_effect_id())
    }
    /// Returns the exact transition which will be published on commit.
    #[must_use]
    pub const fn transition(&self) -> &EngineTransition {
        &self.transition
    }

    #[cfg(feature = "serde")]
    pub(crate) fn candidate_workspace(&self) -> &Workspace {
        self.candidate.workspace()
    }

    #[cfg(feature = "serde")]
    pub(crate) const fn candidate_presentation_identity_frontier(
        &self,
    ) -> PresentationIdentityFrontier {
        self.candidate.presentation_identity_frontier()
    }

    /// Publishes the candidate if its destination still has the frozen authority.
    ///
    /// # Errors
    ///
    /// Returns the same typed staleness errors as ordinary host-frame preparation
    /// when any reducer, provider, or presentation frontier changed after prepare.
    pub fn commit(self, engine: &mut DockEngine) -> Result<EngineTransition, EngineError> {
        engine.validate_host_frame_commit_fence(self.fence)?;
        let Self {
            fence: _,
            candidate,
            transition,
            backend_ingress_commit_guard,
            surface_pointer_commit,
        } = self;
        match backend_ingress_commit_guard {
            Some(guard) => guard
                .publish_with(|| {
                    publish_prepared_host_candidate(
                        engine,
                        candidate,
                        transition,
                        surface_pointer_commit,
                    )
                })
                .map_err(|source| EngineError::BackendIngress { source })?,
            None => publish_prepared_host_candidate(
                engine,
                candidate,
                transition,
                surface_pointer_commit,
            ),
        }
    }
}

fn publish_prepared_host_candidate(
    engine: &mut DockEngine,
    mut candidate: DockEngine,
    transition: EngineTransition,
    surface_pointer_commit: Option<SurfaceLocalPointerFrameCommit>,
) -> Result<EngineTransition, EngineError> {
    candidate.mark_runtime_boundary_published();
    if let Some(mut commit) = surface_pointer_commit {
        let previous = commit
            .publish_with(|| std::mem::replace(engine, candidate))
            .map_err(|source| EngineError::SurfaceLocalPointerCommit { source })?;
        drop(previous);
    } else {
        *engine = candidate;
    }
    Ok(transition)
}
