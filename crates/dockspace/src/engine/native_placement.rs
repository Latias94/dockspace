use super::*;

impl DockEngine {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reduce_native_surface_placements(
        &mut self,
        input: InputSequence,
        provider: PlatformObservationLease,
        expected: WorkspaceVersion,
        window: crate::platform::viewport::WindowBinding,
        placements: &[crate::platform::coordinates::NativeSurfacePlacement],
        focus_causal: FocusCausalStamp,
        context: PlatformSnapshotReductionContext<'_>,
    ) -> Result<InputOutcome, EngineError> {
        if expected != self.version {
            return Ok(InputOutcome::StaleRejected {
                expected,
                accepted_base: self.version,
            });
        }
        if let Some(rejected) = self.platform_provider_rejection(provider) {
            return Ok(rejected);
        }
        let Some(record) = self.viewport.registry().window(window).filter(|record| {
            record.role() == ViewportRole::Child
                || record.ownership() == ViewportOwnership::External
        }) else {
            return Err(EngineError::Viewport {
                input,
                source: ViewportCoordinatorError::Registry(
                    crate::platform::registry::ViewportRegistryError::WindowPlacementUnavailable {
                        binding: window,
                    },
                ),
            });
        };
        let child = (record.role() == ViewportRole::Child).then(|| record.observation_binding());
        if child.is_none() {
            for placement in placements {
                let surface = placement.surface();
                let conflict = if self.workspace.surface(surface).is_none() {
                    Some(
                        crate::platform::registry::ViewportRegistryError::MissingSurface {
                            surface,
                        },
                    )
                } else if self.bound_surface_recoveries.contains_key(&surface)
                    || self.viewport.has_surface_establishment(surface)
                {
                    Some(crate::platform::registry::ViewportRegistryError::SurfaceAlreadyRegistered { surface })
                } else {
                    None
                };
                if let Some(error) = conflict {
                    return Err(EngineError::Viewport {
                        input,
                        source: ViewportCoordinatorError::Registry(error),
                    });
                }
            }
        }
        let changed = self
            .viewport
            .set_surface_placements(window, placements)
            .map_err(|source| EngineError::Viewport { input, source })?;
        if child.is_none() {
            for placement in placements {
                if !self
                    .root_recovery_anchors
                    .contains_key(&placement.surface())
                {
                    let id = self
                        .last_root_recovery_anchor
                        .checked_next()
                        .ok_or(EngineError::RootRecoveryAnchorExhausted { input })?;
                    self.root_recovery_anchors.insert(
                        placement.surface(),
                        RootRecoveryAnchor::new(self.authority_domain, id, placement.surface()),
                    );
                    self.last_root_recovery_anchor = id;
                }
            }
        }
        let placements_changed = !changed.is_empty();
        for surface in changed {
            if child.is_none() || self.scene().surface(surface).is_some() {
                self.invalidate_surface_coordinate_scene(surface)
                    .map_err(|source| Self::scene_revision_error(input, source))?;
            }
        }
        if let Some(child) = child.filter(|_| placements_changed) {
            let actions = self
                .viewport
                .refresh_child_placement_readiness(child)
                .map_err(|source| EngineError::Viewport { input, source })?;
            if !actions.is_empty() {
                let recovery_batch = self.freeze_surface_recovery_batch(input, &actions)?;
                self.reduce_platform_lifecycle_actions(
                    input,
                    provider,
                    focus_causal,
                    &actions,
                    &recovery_batch,
                    context,
                )?;
            }
        }
        Ok(InputOutcome::NativeSurfacePlacementsChanged {
            window,
            changed: placements_changed,
        })
    }
}
