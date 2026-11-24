//! Docking and contained placement evaluation for current pointer authority.

use super::*;

impl DockEngine {
    pub(super) fn resolve_journal_drag_preview(
        &self,
        cause: ReductionCause,
        drag: &crate::interaction::ActiveDrag,
        position: ManipulationPositionKey,
        edge: &PointerEdge,
        receipt: &ValidatedPointerReceiverReceipt,
        snapshot: &JournalPresentationSnapshot,
        desktop_route: Option<DesktopRouteValidation>,
        policy: &DockPolicySnapshot,
    ) -> Result<PreviewEvaluation, EngineError> {
        let outside_all = matches!(
            desktop_route,
            Some(DesktopRouteValidation::Known(
                ValidatedDesktopRoute::NoWindow { .. }
            ))
        );
        if outside_all
            || (matches!(
                desktop_route,
                Some(DesktopRouteValidation::Known(
                    ValidatedDesktopRoute::Foreign { .. }
                ))
            ) && edge.native_tear_off().is_some())
        {
            let decision =
                self.resolve_journal_native_preview(cause, drag, edge, desktop_route, policy)?;
            return Ok(if outside_all {
                PreviewEvaluation::outside_all(decision)
            } else {
                PreviewEvaluation::without_affordance(decision)
            });
        }
        let Some(edge_point) = position.logical_point() else {
            return Ok(PreviewEvaluation::without_affordance(
                PreviewDecision::Clear(Self::journal_route_preview_status(edge, desktop_route)),
            ));
        };
        let hover = match receipt.observation() {
            PointerReceiverObservation::Unknown(_) => {
                return Ok(PreviewEvaluation::without_affordance(
                    PreviewDecision::Clear(PreviewResolutionStatus::UnknownAuthority),
                ));
            }
            PointerReceiverObservation::NotApplicable => {
                return Err(EngineError::PointerInteractionInvariant {
                    cause,
                    detail: "active drag edge has no hover receiver observation".to_owned(),
                });
            }
            PointerReceiverObservation::Presented(observation) => observation
                .probes()
                .iter()
                .find_map(|probe| match probe {
                    PointerReceiverProbeReceipt::HoverHit(hover) => Some(*hover),
                    PointerReceiverProbeReceipt::Delivery(_) => None,
                })
                .ok_or_else(|| EngineError::PointerInteractionInvariant {
                    cause,
                    detail: "active drag edge receipt has no hover probe".to_owned(),
                })?,
        };
        match hover.disposition() {
            PointerReceiverHoverHitDisposition::Unknown(_) => {
                return Ok(PreviewEvaluation::without_affordance(
                    PreviewDecision::Clear(PreviewResolutionStatus::UnknownAuthority),
                ));
            }
            PointerReceiverHoverHitDisposition::Blocked => {
                return Ok(PreviewEvaluation::without_affordance(
                    PreviewDecision::Clear(PreviewResolutionStatus::OpaqueBlocker),
                ));
            }
            PointerReceiverHoverHitDisposition::Dock(_)
            | PointerReceiverHoverHitDisposition::EmptyLane => {}
        }
        let (Some(output), Some(authority), Some(point)) =
            (hover.output(), hover.authority(), hover.point())
        else {
            return Err(EngineError::PointerInteractionInvariant {
                cause,
                detail: "known hover receipt has no atomic presentation binding".to_owned(),
            });
        };
        if point != edge_point {
            return Err(EngineError::PointerInteractionInvariant {
                cause,
                detail: "known hover receipt point differs from its journal edge".to_owned(),
            });
        }
        let presentation = snapshot.presentation(output, authority).map_err(|source| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("{source:?}"),
            }
        })?;
        if !self.presentation_authority_is_current(Self::freeze_journal_presentation(presentation))
        {
            return Ok(PreviewEvaluation::without_affordance(
                PreviewDecision::Clear(PreviewResolutionStatus::UnknownAuthority),
            ));
        }
        if presentation.surface() != drag.source_surface && drag.owner.is_desktop() {
            let capability = self
                .viewport
                .physical_cross_surface_drag_capability(self.native_drag_source_input(drag));
            if !capability.is_supported() || !self.physical_drag_has_route(drag, edge) {
                return Ok(PreviewEvaluation::without_affordance(
                    PreviewDecision::Clear(Self::physical_drag_preview_status(capability)),
                ));
            }
        }
        Self::validate_journal_desktop_presentation(edge, desktop_route, authority)?;
        self.validate_journal_native_local_presentation(edge, authority)?;
        let winner = presentation
            .hit_manifest()
            .resolve_exclusive_with_suppression(
                PresentationPointerLane::HoverDrop,
                point,
                self.hover_drop_suppression(),
            )
            .map_err(|_| EngineError::PointerReceiverGeometry {
                source: PointerReceiverGeometryError::ReceiverWinnerAmbiguous {
                    sequence: edge.sequence(),
                    lane: PresentationPointerLane::HoverDrop,
                },
            })?
            .map(|region| region.id());
        let claimed = match hover.disposition() {
            PointerReceiverHoverHitDisposition::Dock(region) => Some(region),
            PointerReceiverHoverHitDisposition::EmptyLane => None,
            PointerReceiverHoverHitDisposition::Blocked
            | PointerReceiverHoverHitDisposition::Unknown(_) => {
                unreachable!("blocked and unknown hover receipts return before winner validation")
            }
        };
        if winner != claimed {
            return Err(EngineError::PointerReceiverGeometry {
                source: PointerReceiverGeometryError::ReceiverWinnerMismatch {
                    sequence: edge.sequence(),
                    lane: PresentationPointerLane::HoverDrop,
                    claimed,
                    winner,
                },
            });
        }
        if claimed
            .is_some_and(|region| !self.presented_receiver_is_current(presentation.plan(), region))
        {
            return Ok(PreviewEvaluation::without_affordance(
                PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
            ));
        }
        // Framework clearance and the source-suppressed semantic winner are
        // proven independently. The existing solver remains the only target
        // policy; adapters never reconstruct payload membership.
        let query = self
            .resolve_presented_drop_with_current_index(
                &presentation,
                drag.source_layout_facts.as_deref(),
                policy,
                drag.session,
                drag.payload.clone(),
                drag.surface_background_offer,
                point,
                drag.magnetic_targeting_allowed(),
            )
            .map_err(|source| EngineError::PointerInteractionInvariant {
                cause,
                detail: source.to_string(),
            })?;
        let (resolution, affordance, observation) = query.into_evaluation_parts();
        let mut prepared_mutation = None;
        let decision = match resolution {
            DropResolution::Resolved(resolved) => {
                if resolved.session() != drag.session || resolved.source() != &drag.payload {
                    return Err(EngineError::PointerInteractionInvariant {
                        cause,
                        detail: "drop resolver changed the frozen journal drag identity".to_owned(),
                    });
                }
                let target = resolved.target_id();
                let valid = presentation
                    .hit_manifest()
                    .region_for_kind(PresentationHitRegionKind::DropTarget(target))
                    .is_some_and(|region| {
                        self.presented_receiver_is_current(presentation.plan(), region.id())
                    });
                if !valid {
                    return Ok(PreviewEvaluation::without_affordance(
                        PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                    ));
                }
                let scene = resolved.scene_stamp();
                let rect = resolved.visual().rect();
                let command = resolved.command().clone();
                let tab_strip = resolved.preview_tab_strip();
                let mutation = resolved.into_prepared_mutation();
                if self
                    .retained_host_surfaces_for_publication(
                        mutation.workspace(),
                        WorkspacePublicationAuthority::Ordinary,
                    )
                    .is_err()
                {
                    return Ok(PreviewEvaluation::without_affordance(
                        PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                    ));
                }
                prepared_mutation = Some(mutation);
                PreviewDecision::Publish {
                    scene,
                    visual: PreviewVisual::Dock {
                        surface: presentation.surface(),
                        target,
                        rect,
                    },
                    proof: Box::new(PreviewProof::Dock { target, command }),
                    tab_strip,
                }
            }
            DropResolution::KnownNone(_) => {
                let (decision, mutation) = self.resolve_journal_local_contained_candidate(
                    cause,
                    drag,
                    position,
                    SurfacePointer::new(presentation.surface(), point),
                    policy,
                )?;
                prepared_mutation = mutation;
                decision
            }
            DropResolution::Rejected(_) => {
                PreviewDecision::Clear(PreviewResolutionStatus::Rejected)
            }
            DropResolution::Unavailable(_) => {
                return Err(EngineError::PointerInteractionInvariant {
                    cause,
                    detail: "frozen presented drop resolver returned unavailable".to_owned(),
                });
            }
        };
        let target_layout_basis = presentation
            .plan()
            .contained_projection()
            .filter(|projection| {
                projection.owner
                    == crate::presentation::scene::ContainedLayoutOwner::Drag(drag.session)
            })
            .and_then(|projection| projection.target_basis)
            .unwrap_or(presentation.scene());
        let affordance = affordance.map(|mut affordance| {
            affordance.bind_target_layout_basis(target_layout_basis);
            affordance
        });
        let mut evaluation =
            PreviewEvaluation::new(decision, affordance, observation, Some(target_layout_basis));
        evaluation.prepared_mutation = prepared_mutation;
        Ok(evaluation)
    }

    pub(in crate::engine) fn resolve_presented_drop_with_current_index(
        &self,
        presentation: &JournalSurfacePresentation,
        source_layout_facts: Option<&crate::presentation::scene::PresentationLayoutFacts>,
        policy: &DockPolicySnapshot,
        session: crate::interaction::DragSessionId,
        source: MovePayload,
        surface_background_offer: Option<crate::input::intent::SurfaceBackgroundRootOffer>,
        point: crate::geometry::LogicalPoint,
        allow_magnetic: bool,
    ) -> Result<crate::interaction::targeting::resolver::DropQuery, DropResolutionError> {
        let targeting = if !self.interaction.docking_enabled() {
            DropTargeting::Disabled
        } else if allow_magnetic {
            DropTargeting::Magnetic
        } else {
            DropTargeting::Explicit
        };
        crate::interaction::targeting::resolver::resolve_presented_drop_with_group_mode(
            presentation.scene(),
            presentation.plan(),
            source_layout_facts,
            &self.workspace,
            self.version,
            self.presentation_authority
                .presentation_requirements
                .workspace_index(),
            policy,
            session,
            source,
            surface_background_offer,
            point,
            targeting,
            self.interaction.group_dock_mode(),
        )
    }

    pub(super) fn resolve_journal_local_contained_candidate(
        &self,
        cause: ReductionCause,
        drag: &crate::interaction::ActiveDrag,
        position: ManipulationPositionKey,
        pointer: SurfacePointer,
        policy: &DockPolicySnapshot,
    ) -> Result<(PreviewDecision, Option<PreparedDropMutation>), EngineError> {
        let candidate = match &drag.origin {
            FrozenDragOrigin::Contained(origin) => {
                self.core_contained_move_candidate(drag, origin, position, pointer.surface())
            }
            FrozenDragOrigin::Workspace => self.core_contained_candidate(drag, pointer, None),
        };
        match candidate {
            CoreContainedCandidate::None => Ok((
                PreviewDecision::Clear(PreviewResolutionStatus::KnownNone),
                None,
            )),
            CoreContainedCandidate::Proposal(proposal) => {
                self.resolve_contained_proposal(cause, drag, proposal, policy)
            }
            CoreContainedCandidate::Rejected => Ok((
                PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                None,
            )),
            CoreContainedCandidate::Cancel(reason) => Ok((PreviewDecision::Cancel(reason), None)),
        }
    }

    fn core_contained_move_candidate(
        &self,
        drag: &crate::interaction::ActiveDrag,
        origin: &FrozenContainedDragOrigin,
        position: ManipulationPositionKey,
        destination: SurfaceId,
    ) -> CoreContainedCandidate {
        if destination != origin.surface {
            return CoreContainedCandidate::None;
        }
        if !self.frozen_drag_origin_is_current(drag.origin.clone()) {
            return CoreContainedCandidate::Rejected;
        }
        let Some(point) = contained_manipulation_point(position, origin.surface) else {
            return CoreContainedCandidate::None;
        };
        let (scene, bounds) =
            match Self::ready_surface_bounds(self.scene(), self.version, origin.surface) {
                Ok(ready) => ready,
                Err(
                    ContainedPlacementUnavailable::StaleSurface { .. }
                    | ContainedPlacementUnavailable::PendingPaintSurface { .. },
                ) => return CoreContainedCandidate::None,
                Err(_) => {
                    return CoreContainedCandidate::Cancel(
                        InteractionCancelReason::SceneUnavailable,
                    );
                }
            };
        let previous = drag.preview().and_then(|preview| match preview.proof() {
            PreviewProof::Contained { proposal, .. }
                if proposal.surface() == origin.surface
                    && proposal.root() == origin.root
                    && proposal.floating() == origin.floating
                    && proposal.placement().surface_bounds() == bounds
                    && drag.last_advanced_position().and_then(|previous| {
                        contained_manipulation_point(previous, origin.surface)
                    }) == Some(point) =>
            {
                Some((proposal.placement().requested_rect(), proposal.rect()))
            }
            _ => None,
        });
        let (requested, rect) = match previous.map(Ok).unwrap_or_else(|| {
            contained_move_geometry(bounds, origin.source_rect, origin.initial_pointer, point)
        }) {
            Ok(geometry) => geometry,
            Err(()) => return CoreContainedCandidate::Rejected,
        };
        let placement = ContainedPlacementProof::new(
            scene,
            origin.surface,
            requested,
            origin.minimum_size,
            bounds,
            rect,
        );
        CoreContainedCandidate::Proposal(crate::input::intent::ContainedTearOffProposal::new(
            origin.root,
            origin.floating,
            placement,
            ContainedPosition::Front,
        ))
    }

    fn valid_contained_move_proposal(
        &self,
        origin: &FrozenContainedDragOrigin,
        proposal: crate::input::intent::ContainedTearOffProposal,
    ) -> bool {
        let proof = proposal.placement();
        let Some(current) = self.scene().ready_surface(proof.surface()) else {
            return false;
        };
        let requested = proof.requested_rect();
        // Existing movement has its own sizing rule, with the same exact scene,
        // epoch and bounds checks as placement. An oversized source stays intact.
        // Reconstruct the movement's canonical corners: subtracting translated
        // corners can round a valid extent differently from the source extent.
        current.stamp() == proof.scene()
            && proof.scene().requirement().workspace_epoch() == self.version.epoch()
            && current.plan().bounds() == proof.surface_bounds()
            && proof.surface() == origin.surface
            && proof.minimum_size() == origin.minimum_size
            && LogicalRect::from_min_size(requested.min(), origin.source_rect.size())
                == Ok(requested)
            && clamp_moved_contained_rect(proof.surface_bounds(), requested)
                == Ok(proof.clamped_rect())
    }

    pub(in crate::engine) fn resolve_contained_proposal(
        &self,
        cause: ReductionCause,
        drag: &crate::interaction::ActiveDrag,
        proposal: crate::input::intent::ContainedTearOffProposal,
        policy: &DockPolicySnapshot,
    ) -> Result<(PreviewDecision, Option<PreparedDropMutation>), EngineError> {
        let Some(command) = self.contained_presentation_command(
            &drag.payload,
            proposal,
            drag.complete_root.clone(),
            match &drag.origin {
                FrozenDragOrigin::Contained(origin) => Some(origin),
                FrozenDragOrigin::Workspace => None,
            },
        ) else {
            return Ok((
                PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                None,
            ));
        };
        let geometry_is_current = match &drag.origin {
            FrozenDragOrigin::Contained(origin) => {
                self.valid_contained_move_proposal(origin, proposal)
            }
            FrozenDragOrigin::Workspace => self
                .validate_contained_placement(proposal.placement())
                .is_ok(),
        };
        if !geometry_is_current || !self.valid_contained_command_structure(drag, proposal, &command)
        {
            return Ok((
                PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                None,
            ));
        }
        let prepared = match crate::graph::operation::prepare_transaction(
            self.workspace.as_ref(),
            policy,
            std::slice::from_ref(&command),
        ) {
            Ok(prepared) => prepared,
            Err(TransactionError::Command { index: 0, source })
                if source.is_expected_rejection() =>
            {
                return Ok((
                    PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                    None,
                ));
            }
            Err(source) => {
                return Err(EngineError::PointerInteractionInvariant {
                    cause,
                    detail: source.to_string(),
                });
            }
        };
        let (workspace, report) = prepared.into_parts();
        let staged = match self.stage_journal_workspace_draft(
            cause,
            policy,
            WorkspacePublicationAuthority::Ordinary,
            JournalDeliveryActivation::None,
            workspace,
            &report,
        )? {
            Ok(staged) => staged,
            Err(_) => {
                return Ok((
                    PreviewDecision::Clear(PreviewResolutionStatus::Rejected),
                    None,
                ));
            }
        };
        let mutation = PreparedDropMutation::new(
            self.version,
            policy.revision(),
            command.clone(),
            staged.publication.workspace.into_workspace(),
            report,
        );
        Ok((
            PreviewDecision::Publish {
                scene: proposal.placement().scene(),
                visual: PreviewVisual::Contained {
                    surface: proposal.surface(),
                    rect: proposal.rect(),
                    fallback: false,
                },
                proof: Box::new(PreviewProof::Contained {
                    command,
                    proposal,
                    fallback: false,
                }),
                tab_strip: None,
            },
            Some(mutation),
        ))
    }

    pub(super) fn apply_preview_evaluation(
        &mut self,
        cause: ReductionCause,
        owner: GestureOwner,
        session: crate::interaction::DragSessionId,
        evaluation: PreviewEvaluation,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<Option<InteractionOutcome>, EngineError> {
        let PreviewEvaluation {
            decision,
            affordance,
            observation,
            target_layout_basis,
            prepared_mutation: _,
        } = evaluation;
        self.interaction
            .set_drop_affordance(session, affordance)
            .map_err(|source| EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("{source:?}"),
            })?;
        match decision {
            PreviewDecision::Publish {
                scene,
                visual,
                proof,
                tab_strip,
            } => {
                let (preview, changed) = self
                    .interaction
                    .publish_preview(
                        session,
                        scene,
                        visual,
                        *proof,
                        observation.acquisition(),
                        tab_strip,
                    )
                    .map_err(|source| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!("{source:?}"),
                    })?;
                self.interaction
                    .bind_preview_layout_basis(session, target_layout_basis)
                    .map_err(|source| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!("{source:?}"),
                    })?;
                if changed {
                    interaction_events.push(InteractionEvent::new_caused(
                        cause,
                        self.version,
                        InteractionEventKind::PreviewPublished {
                            preview: preview.clone(),
                        },
                    ));
                }
                Ok(Some(InteractionOutcome::PreviewUpdated {
                    session,
                    preview: Some(preview),
                    status: PreviewResolutionStatus::Resolved,
                }))
            }
            PreviewDecision::Clear(status) => {
                let changed =
                    self.interaction
                        .clear_preview(session, status)
                        .map_err(|source| EngineError::PointerInteractionInvariant {
                            cause,
                            detail: format!("{source:?}"),
                        })?;
                if changed {
                    interaction_events.push(InteractionEvent::new_caused(
                        cause,
                        self.version,
                        InteractionEventKind::PreviewCleared { session },
                    ));
                }
                Ok(Some(InteractionOutcome::PreviewUpdated {
                    session,
                    preview: None,
                    status,
                }))
            }
            PreviewDecision::Cancel(reason) => {
                self.cancel_gesture_owner(cause, owner, reason, interaction_events)
            }
        }
    }

    pub(super) fn retained_position_preview<'drag>(
        &self,
        cause: ReductionCause,
        drag: &'drag crate::interaction::ActiveDrag,
        position: ManipulationPositionKey,
        evaluation: &mut PreviewEvaluation,
    ) -> Result<Option<&'drag crate::interaction::PublishedPreview>, EngineError> {
        let compatible_preview = drag.preview().filter(|preview| {
            self.current_contained_projection_is_owned(drag.source_surface)
                && self.drag_preview_authority_is_current(drag)
                && drag
                    .drop_affordance()
                    .map(DropAffordance::guide_suppression)
                    == evaluation
                        .affordance
                        .as_ref()
                        .map(DropAffordance::guide_suppression)
                && preview.target_layout_basis() == evaluation.target_layout_basis
        });
        let latch_observation = match (compatible_preview, evaluation.observation) {
            (
                Some(preview),
                TargetObservation::Surface {
                    layer, candidate, ..
                },
            ) => TargetObservation::surface(preview.public().token().scene(), layer, candidate),
            _ => evaluation.observation,
        };
        let retain_current = self
            .interaction
            .reconcile_drag_target(
                drag.session,
                latch_observation,
                position.logical_point(),
                self.presentation_config().target_release_hysteresis(),
            )
            .map_err(|source| EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("drag target latch rejected the position: {source:?}"),
            })?
            == TargetLatchDecision::RetainCurrent;
        if retain_current {
            let target = drag.target_acquisition().ok_or_else(|| {
                EngineError::PointerInteractionInvariant {
                    cause,
                    detail: "latched drag retained no target acquisition".to_owned(),
                }
            })?;
            let retained_target_is_visible = evaluation
                .affordance
                .as_mut()
                .is_some_and(|next| next.activate_target(target.target()));
            if !retained_target_is_visible {
                evaluation.affordance = drag.drop_affordance().cloned();
            }
            return drag.preview().map(Some).ok_or_else(|| {
                EngineError::PointerInteractionInvariant {
                    cause,
                    detail: "retained drag has no preview".to_owned(),
                }
            });
        }
        // Only the source layout advanced. Preserve an exact preview when
        // the current hover winner still gives the same command and visual.
        Ok(compatible_preview
            .filter(|preview| Self::contains_same_preview_command(preview, &evaluation.decision)))
    }

    pub(super) fn contains_same_preview_command(
        previous: &crate::interaction::PublishedPreview,
        decision: &PreviewDecision,
    ) -> bool {
        let PreviewDecision::Publish {
            visual,
            proof,
            tab_strip,
            ..
        } = decision
        else {
            return false;
        };
        if previous.public().visual() != visual
            || previous.public().tab_strip() != tab_strip.as_deref()
        {
            return false;
        }
        match (previous.proof(), proof.as_ref()) {
            (
                PreviewProof::Dock {
                    target: old_target,
                    command: old,
                },
                PreviewProof::Dock { target, command },
            ) => old_target == target && old == command,
            (
                PreviewProof::Contained {
                    command: old,
                    fallback: old_fallback,
                    ..
                },
                PreviewProof::Contained {
                    command, fallback, ..
                },
            ) => old == command && old_fallback == fallback,
            _ => false,
        }
    }
}
