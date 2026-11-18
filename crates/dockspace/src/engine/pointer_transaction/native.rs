//! Pointer-driven native placement and delivery through the shared create saga.

use super::*;

impl DockEngine {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_journal_native_delivery(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        drag: crate::interaction::NativeDragTearOff,
        source_presentation: PresentedSurfaceAuthority,
        payload: MovePayload,
        pane_focus: PaneFocusDisposition,
        command: WorkspaceCommand,
        offer: NativePresentationOffer,
        policy: &DockPolicySnapshot,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        let session = drag.session();
        let proposal = offer.proposal();
        if !matches!(proposal.placement(), NativePlacementProof::TearOff(_)) {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::TargetChanged,
            ));
        }
        if !self
            .viewport
            .native_placement_is_current(proposal.placement())
        {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::TargetChanged,
            ));
        }
        let normalized = crate::platform::frame::NativeCreateProposal::from_tear_off(proposal)
            .with_drag_tear_off(drag);
        let request = match self.start_native_root_create(
            cause,
            focus_causal,
            source_presentation,
            payload,
            command,
            normalized,
            pane_focus,
            &[],
            policy,
        )? {
            Ok(request) => request,
            Err(rejection) => {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(
                        rejection.into_interaction_command_error(),
                    ),
                ));
            }
        };
        interaction_events.push(InteractionEvent::new_caused(
            cause,
            self.version,
            InteractionEventKind::NativePresentationRequested(request),
        ));
        Ok(InteractionOutcome::DragDelivered {
            session,
            delivery: InteractionDelivery::NativeRequested(request),
        })
    }

    /// Resolves native placement separately from the actual desktop hover.
    /// Foreign windows remain opaque docking blockers; an explicit edge-local
    /// intent can only request creation through this drag's existing authority.
    pub(super) fn resolve_journal_native_preview(
        &self,
        cause: ReductionCause,
        drag: &crate::interaction::ActiveDrag,
        edge: &PointerEdge,
        desktop_route: Option<DesktopRouteValidation>,
        policy: &DockPolicySnapshot,
    ) -> Result<PreviewDecision, EngineError> {
        let intent = edge.native_tear_off();
        let placement = match desktop_route {
            Some(DesktopRouteValidation::Known(ValidatedDesktopRoute::NoWindow {
                position: Authority::Known(position),
                work_area,
            })) => intent
                .map(|intent| (position, intent.work_area))
                .or_else(|| match work_area {
                    Authority::Known(work_area) => Some((position, work_area)),
                    Authority::Unknown(_) => None,
                }),
            Some(DesktopRouteValidation::Known(ValidatedDesktopRoute::Foreign {
                position: Authority::Known(position),
            })) => intent.map(|intent| (position, intent.work_area)),
            _ => None,
        };
        let Some((desktop_position, work_area_route)) = placement else {
            return Ok(PreviewDecision::Clear(
                PreviewResolutionStatus::NativePlacementUnavailable,
            ));
        };
        if intent
            .and_then(|intent| intent.drag)
            .is_some_and(|identity| {
                identity
                    != crate::interaction::NativeDragTearOff::new(drag.owner.stream(), drag.session)
            })
        {
            return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Rejected));
        }
        let pointer_provider = drag.owner.lease();
        if self.platform_provider() != Some(work_area_route.provider())
            || self.viewport.work_area_generation() != work_area_route.generation()
        {
            return Ok(PreviewDecision::Clear(
                PreviewResolutionStatus::NativePlacementUnavailable,
            ));
        }
        let Some(work_area) = self.viewport.work_area(work_area_route.token()) else {
            return Ok(PreviewDecision::Clear(
                PreviewResolutionStatus::NativePlacementUnavailable,
            ));
        };
        if policy.check_tear_off(TearOffPresentation::Native).is_err() {
            return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Rejected));
        }
        let source_input = self.native_drag_source_input(drag);
        let capability = self.viewport.physical_native_drag_capability(source_input);
        if !capability.is_supported() || !self.physical_drag_has_route(drag, edge) {
            let status = Self::physical_drag_preview_status(capability);
            return Ok(PreviewDecision::Clear(status));
        }
        let Some(reservation) = drag.journal_presentation_reservation else {
            return Ok(PreviewDecision::Clear(
                PreviewResolutionStatus::NativePlacementUnavailable,
            ));
        };
        let (source_geometry, initial_pointer) = match &drag.origin {
            FrozenDragOrigin::Workspace => {
                let (Some(source_geometry), Some(initial_pointer)) =
                    (drag.journal_source_geometry, drag.initial_pointer)
                else {
                    return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Unavailable));
                };
                (source_geometry, initial_pointer)
            }
            FrozenDragOrigin::Contained(origin) => (
                JournalDragSourceGeometry {
                    source_rect: origin.source_rect,
                    minimum_size: origin.minimum_size,
                },
                origin.initial_pointer,
            ),
        };
        let cursor_offset = match LogicalSize::new(
            initial_pointer.x() - source_geometry.source_rect.x(),
            initial_pointer.y() - source_geometry.source_rect.y(),
        ) {
            Ok(offset) => offset,
            Err(_) => return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Rejected)),
        };
        let request = TearOffPlacementRequest::new(
            cursor_offset,
            source_geometry.source_rect.size(),
            source_geometry.minimum_size,
            work_area_route.token(),
        );
        let placement = match solve_tear_off_placement(
            pointer_provider,
            work_area_route.provider(),
            edge.pointer(),
            source_input,
            desktop_position,
            work_area,
            work_area_route.generation(),
            request,
        ) {
            Ok(placement) => placement,
            Err(_) => {
                return Ok(PreviewDecision::Clear(
                    PreviewResolutionStatus::NativePlacementUnavailable,
                ));
            }
        };
        if placement.pointer() != drag.owner.pointer()
            || placement.desktop_position() != desktop_position
        {
            return Ok(PreviewDecision::Clear(
                PreviewResolutionStatus::NativePlacementUnavailable,
            ));
        }
        let source_scene = self
            .presentation_authority
            .scene
            .ready_surface(drag.source_surface)
            .map(|ready| ready.stamp());
        let Some(source_scene) = source_scene else {
            return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Unavailable));
        };
        let command = Self::tear_off_command_from_complete_root(
            &drag.payload,
            RootPresentationTarget::NewSurface {
                surface: reservation.surface,
            },
            reservation.root,
            drag.complete_root.clone(),
        );
        let staged = match self.stage_journal_workspace_command(cause, &command, policy)? {
            Ok(staged) => staged,
            Err(_) => return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Rejected)),
        };
        let recovery_anchor = self
            .surface_recovery_target(drag.source_surface)
            .map(|target| target.anchor())
            .or_else(|| {
                self.root_recovery_anchors
                    .get(&drag.source_surface)
                    .copied()
            });
        let Some(recovery_anchor) = recovery_anchor else {
            return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Rejected));
        };
        let recovery_target = SurfaceRecoveryTarget::with_converted_main(
            recovery_anchor,
            ConvertedMainRecovery::new(
                reservation.root,
                reservation.floating,
                source_geometry.minimum_size,
            ),
        );
        let obligation = self.authorize_surface_recovery_obligation(
            self.last_input,
            SurfaceRecoveryObligationId::default(),
            &staged.publication.workspace,
            reservation.surface,
            recovery_target,
            policy,
        );
        if obligation.is_err() {
            return Ok(PreviewDecision::Clear(PreviewResolutionStatus::Rejected));
        }
        let offer = NativePresentationOffer::new(
            NativeTearOffProposal::new(
                reservation.surface,
                reservation.root,
                placement,
                ConvertedMainRecovery::new(
                    reservation.root,
                    reservation.floating,
                    source_geometry.minimum_size,
                ),
            )
            .map_err(|error| EngineError::PointerInteractionInvariant {
                cause,
                detail: error.to_string(),
            })?,
            None,
        );
        Ok(PreviewDecision::Publish {
            scene: source_scene,
            visual: PreviewVisual::Native {
                host_surface: drag.source_surface,
                target_surface: reservation.surface,
                placement: offer.proposal().physical_placement(),
            },
            proof: Box::new(PreviewProof::Native { command, offer }),
            tab_strip: None,
        })
    }

    pub(super) fn physical_drag_has_route(
        &self,
        drag: &crate::interaction::ActiveDrag,
        edge: &PointerEdge,
    ) -> bool {
        let Some(binding) = self.viewport.confirmed_drag_source(drag.owner.pointer()) else {
            return false;
        };
        if binding.surface() != drag.source_surface {
            return false;
        }
        if self.viewport.drag_source_input(drag.owner.pointer())
            == Some(crate::platform::NativeDragSourceInput::Preserve)
        {
            // Release reports capture after the button transition. Reuse the
            // release gate for post-edge None after an unknown sample; ordinary
            // motion still requires the retained exact capture owner.
            if edge.delivery_owner()
                != Authority::Known(PointerEventDeliveryOwner::Native(binding.window()))
            {
                return false;
            }
            if matches!(
                edge.kind(),
                PointerEdgeKind::ButtonReleased(PointerButton::Primary)
                    | PointerEdgeKind::ContactEnded(PointerButton::Primary)
            ) && edge.capture_owner() == Authority::Known(PointerCaptureOwner::None)
            {
                return matches!(
                    self.journal_capture_action_gate(drag.owner, edge.capture_owner(), true),
                    Ok(JournalCaptureActionGate::Authorized)
                );
            }
            matches!(self.interaction.active_journal_capture_authority(),
                Some(Authority::Known(PointerCaptureOwner::Native(current))) if current == binding.window())
        } else {
            true
        }
    }

    pub(super) fn physical_drag_preview_status(
        capability: PlatformCapability,
    ) -> PreviewResolutionStatus {
        match capability {
            PlatformCapability::Unknown(_) => PreviewResolutionStatus::NativeCapabilityUnknown,
            PlatformCapability::Unsupported(_) => {
                PreviewResolutionStatus::NativeCapabilityUnavailable
            }
            PlatformCapability::Supported => PreviewResolutionStatus::Rejected,
        }
    }
}
