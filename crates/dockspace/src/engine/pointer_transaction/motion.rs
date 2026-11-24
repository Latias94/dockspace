//! Drag threshold, movement and the frozen presentation reservation.

use super::*;

impl DockEngine {
    pub(super) fn journal_drag_threshold_origin(
        &self,
        edge: &PointerEdge,
        desktop_route: Option<DesktopRouteValidation>,
    ) -> Result<JournalDragThresholdOrigin, EngineError> {
        match edge.location() {
            PointerEdgeLocation::SurfaceLocal {
                position: Authority::Known(position),
            } => Ok(JournalDragThresholdOrigin::SurfaceLocal(position)),
            PointerEdgeLocation::HostLocal { .. } => {
                let route = self.journal_host_local_manipulation(edge).ok_or(
                    EngineError::ReductionCauseInvariant {
                        detail: "host-local press has no authoritative manipulation route",
                    },
                )?;
                Ok(JournalDragThresholdOrigin::HostLocal {
                    surface: route.surface(),
                    position: route.position(),
                })
            }
            PointerEdgeLocation::NativeLocal { .. } => {
                let route = super::pointer_proof::validated_native_local_route(self, edge).ok_or(
                    EngineError::ReductionCauseInvariant {
                        detail: "native-local press has no current coordinate route",
                    },
                )?;
                Ok(JournalDragThresholdOrigin::NativeLocal {
                    binding: route.binding(),
                    coordinate_generation: route.coordinate_generation(),
                    position: route.surface_position(),
                })
            }
            PointerEdgeLocation::Desktop { .. } => {
                let route = desktop_route
                    .and_then(DesktopRouteValidation::dock_route)
                    .ok_or(EngineError::ReductionCauseInvariant {
                        detail: "desktop journal press has no validated coordinate route",
                    })?;
                let source_scale = self
                    .viewport
                    .viewport(route.binding().surface())
                    .filter(|record| record.binding() == route.binding())
                    .and_then(crate::platform::registry::ViewportRecord::coordinates)
                    .filter(|coordinates| {
                        coordinates.coordinate_generation() == route.coordinate_generation()
                    })
                    .map(CoordinateSnapshot::presentation_scale_factor)
                    .ok_or(EngineError::ReductionCauseInvariant {
                        detail: "desktop journal press lost its validated source scale",
                    })?;
                Ok(JournalDragThresholdOrigin::DesktopPhysical {
                    position: route.desktop_position(),
                    source_scale,
                })
            }
            PointerEdgeLocation::SurfaceLocal {
                position: Authority::Unknown(_),
            }
            | PointerEdgeLocation::NativeLocalToDesktop { .. } => {
                Err(EngineError::ReductionCauseInvariant {
                    detail: "journal press receiver has no authoritative local point",
                })
            }
        }
    }

    fn journal_drag_threshold_crossed(
        &self,
        origin: JournalDragThresholdOrigin,
        edge: &PointerEdge,
        source_logical_threshold: f64,
    ) -> bool {
        let (delta_x, delta_y, threshold) = match (origin, edge.location()) {
            (
                JournalDragThresholdOrigin::SurfaceLocal(initial),
                PointerEdgeLocation::SurfaceLocal {
                    position: Authority::Known(current),
                },
            ) => (
                current.x() - initial.x(),
                current.y() - initial.y(),
                source_logical_threshold,
            ),
            (
                JournalDragThresholdOrigin::HostLocal {
                    surface,
                    position: initial,
                },
                PointerEdgeLocation::HostLocal { .. },
            ) => {
                let Some(current) = self
                    .journal_host_local_manipulation(edge)
                    .filter(|route| route.surface() == surface)
                else {
                    return false;
                };
                (
                    current.position().x() - initial.x(),
                    current.position().y() - initial.y(),
                    source_logical_threshold,
                )
            }
            (
                JournalDragThresholdOrigin::NativeLocal {
                    binding,
                    coordinate_generation,
                    position: initial,
                },
                PointerEdgeLocation::NativeLocal { .. },
            ) => {
                let Some(current) = super::pointer_proof::validated_native_local_route(self, edge)
                    .filter(|route| {
                        route.binding() == binding
                            && route.coordinate_generation() == coordinate_generation
                    })
                else {
                    return false;
                };
                (
                    current.surface_position().x() - initial.x(),
                    current.surface_position().y() - initial.y(),
                    source_logical_threshold,
                )
            }
            (
                JournalDragThresholdOrigin::DesktopPhysical {
                    position: initial,
                    source_scale,
                },
                PointerEdgeLocation::Desktop { route },
            ) => {
                let Authority::Known(current) = route.position() else {
                    return false;
                };
                (
                    current.x() - initial.x(),
                    current.y() - initial.y(),
                    source_logical_threshold * source_scale.get(),
                )
            }
            _ => return false,
        };
        let distance_squared = delta_x * delta_x + delta_y * delta_y;
        let threshold_squared = threshold * threshold;
        distance_squared.is_finite()
            && threshold_squared.is_finite()
            && distance_squared >= threshold_squared
    }

    pub(in crate::engine) fn reduce_journal_move(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        owner: GestureOwner,
        edge: &PointerEdge,
        receipt: &ValidatedPointerReceiverReceipt,
        snapshot: &JournalPresentationSnapshot,
        desktop_hover_route: Option<DesktopRouteValidation>,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<Vec<InteractionOutcome>, EngineError> {
        let mut outcomes = Vec::new();
        if self.interaction.active_owner() != Some(owner) {
            return Ok(outcomes);
        }
        let (delivery_authorized, delivery_outcome) = self.gate_journal_delivery_action(
            cause,
            owner,
            edge.delivery_owner(),
            interaction_events,
        )?;
        if let Some(outcome) = delivery_outcome {
            outcomes.push(outcome);
        }
        if !delivery_authorized {
            return Ok(outcomes);
        }
        let (capture_authorized, capture_outcome) = self.gate_journal_capture_action(
            cause,
            owner,
            edge.capture_owner(),
            false,
            interaction_events,
        )?;
        if let Some(outcome) = capture_outcome {
            outcomes.push(outcome);
        }
        if !capture_authorized {
            return Ok(outcomes);
        }
        if matches!(owner.lease().scope(), PointerProviderScope::NativeJoined(_))
            && !self.stationary_edge_has_authority(owner, edge)
        {
            self.clear_pointer_continuation(owner);
        }
        if let Some(outcome) = self.cancel_revoked_presentation_caused(cause, interaction_events)? {
            outcomes.push(outcome);
            return Ok(outcomes);
        }
        if let Some(touch_outcomes) = self.reduce_touch_move(
            cause,
            focus_causal,
            owner,
            edge,
            receipt,
            snapshot,
            desktop_hover_route,
            policy,
            events,
            interaction_events,
        )? {
            outcomes.extend(touch_outcomes);
            return Ok(outcomes);
        }
        if let InteractionStatus::Armed { session } = self.interaction.status() {
            let armed = match self.interaction.armed_drag(session) {
                Ok(armed) => armed,
                Err(error) => {
                    outcomes.push(InteractionOutcome::Rejected(error));
                    return Ok(outcomes);
                }
            };
            if armed.initial_pointer.is_none() {
                return Err(EngineError::ReductionCauseInvariant {
                    detail: "journal-owned armed drag has no initial pointer",
                });
            }
            let threshold_origin = armed.journal_threshold_origin;
            let threshold = self
                .presentation_authority
                .presentation_config
                .pointer_drag_start_distance();
            if !self.journal_drag_threshold_crossed(threshold_origin, edge, threshold) {
                return Ok(outcomes);
            }
            if let Some(rejection) = self.pending_presentation_transition_gesture_rejection() {
                outcomes.push(InteractionOutcome::Rejected(rejection));
                return Ok(outcomes);
            }
            let button = armed.button;
            let source_surface = armed.source_surface;
            match self.interaction.begin_drag(session, owner, button) {
                Ok(()) => {
                    if owner.is_desktop() {
                        let drag = self.interaction.active_drag(session).map_err(|_| {
                            EngineError::ReductionCauseInvariant {
                                detail: "new native drag lost its active payload",
                            }
                        })?;
                        let input = self.native_drag_source_input(drag);
                        if self
                            .viewport
                            .physical_cross_surface_drag_capability(input)
                            .is_supported()
                        {
                            let _ = self
                                .viewport
                                .begin_drag_routing(owner.pointer(), source_surface, input)
                                .map_err(|source| EngineError::Viewport {
                                    input: self.last_input,
                                    source,
                                })?;
                        }
                    }
                    self.freeze_journal_drag_presentation_reservation(cause, session, owner)?;
                    outcomes.push(InteractionOutcome::DragBegan { session });
                }
                Err(error) => {
                    outcomes.push(InteractionOutcome::Rejected(error));
                    return Ok(outcomes);
                }
            }
        }

        let status = self.interaction.status();
        if let InteractionStatus::Resizing { session } = status {
            outcomes.push(self.update_journal_resize(cause, owner, session, edge, policy)?);
            return Ok(outcomes);
        }
        if let InteractionStatus::ContainedTransforming { session } = status {
            outcomes.push(self.update_journal_contained_transform(
                cause,
                owner,
                session,
                edge,
                interaction_events,
            )?);
            return Ok(outcomes);
        }
        let InteractionStatus::Dragging { session } = status else {
            return Ok(outcomes);
        };
        let drag = self.interaction.active_drag(session).map_err(|source| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("{source:?}"),
            }
        })?;
        let position = journal_position_key(self, owner.stream(), edge);
        if !self.drag_source_presentation_allows_targeting(drag) {
            if let Some(evaluation) = Self::drag_exit_without_source_presentation(
                drag,
                position,
                desktop_hover_route,
                receipt.native_hit(),
            ) {
                let outcome = self.apply_position_evaluation(
                    cause,
                    owner,
                    session,
                    position,
                    evaluation,
                    interaction_events,
                )?;
                outcomes.extend(outcome);
            } else {
                outcomes.push(InteractionOutcome::Rejected(
                    InteractionRejection::StaleScene,
                ));
            }
            return Ok(outcomes);
        }
        let evaluation = self.resolve_journal_drag_preview(
            cause,
            drag,
            position,
            edge,
            receipt,
            snapshot,
            desktop_hover_route,
            policy,
        )?;
        if let Some(outcome) = self.apply_position_evaluation(
            cause,
            owner,
            session,
            position,
            evaluation,
            interaction_events,
        )? {
            outcomes.push(outcome);
        }
        self.advance_drag_continuation(cause, owner, session, edge, receipt, snapshot)?;
        Ok(outcomes)
    }

    fn freeze_journal_drag_presentation_reservation(
        &mut self,
        cause: ReductionCause,
        session: crate::interaction::DragSessionId,
        owner: GestureOwner,
    ) -> Result<(), EngineError> {
        let drag = self
            .interaction
            .active_drag(session)
            .map_err(|source| EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("{source:?}"),
            })?
            .clone();
        let (
            initial_pointer,
            source_geometry,
            existing_root,
            existing_floating,
            reserve_background_root,
        ) = match &drag.origin {
            FrozenDragOrigin::Workspace => {
                let initial_pointer = drag.initial_pointer.ok_or_else(|| {
                    EngineError::PointerInteractionInvariant {
                        cause,
                        detail: "journal drag activation has no frozen initial pointer".to_owned(),
                    }
                })?;
                let source_geometry = drag.journal_source_geometry.ok_or_else(|| {
                    EngineError::PointerInteractionInvariant {
                        cause,
                        detail: "journal drag activation has no press-bound source geometry"
                            .to_owned(),
                    }
                })?;
                (
                    initial_pointer,
                    source_geometry,
                    drag.complete_root.as_ref().map(NodeSource::root),
                    None,
                    drag.complete_root.is_none(),
                )
            }
            FrozenDragOrigin::Contained(origin) => (
                origin.initial_pointer,
                JournalDragSourceGeometry {
                    source_rect: origin.source_rect,
                    minimum_size: origin.minimum_size,
                },
                Some(origin.root),
                Some(origin.floating),
                false,
            ),
        };
        let reservation = self.reserve_drag_presentation_identities(
            existing_root.is_none(),
            existing_floating.is_none(),
        )?;
        let root = existing_root.or(reservation.root()).ok_or_else(|| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: "drag presentation reservation did not provide a root".to_owned(),
            }
        })?;
        let floating = existing_floating
            .or(reservation.floating())
            .ok_or_else(|| EngineError::PointerInteractionInvariant {
                cause,
                detail: "drag presentation reservation did not provide a floating identity"
                    .to_owned(),
            })?;
        self.interaction
            .freeze_journal_presentation_reservation(
                session,
                owner,
                reservation.surface(),
                root,
                floating,
                SurfacePointer::new(drag.source_surface, initial_pointer),
                source_geometry.source_rect,
                source_geometry.minimum_size,
                reserve_background_root,
            )
            .map_err(|source| EngineError::PointerInteractionInvariant {
                cause,
                detail: source.to_string(),
            })
    }

    pub(super) fn apply_position_evaluation(
        &mut self,
        cause: ReductionCause,
        owner: GestureOwner,
        session: crate::interaction::DragSessionId,
        position: ManipulationPositionKey,
        mut evaluation: PreviewEvaluation,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<Option<InteractionOutcome>, EngineError> {
        let drag = self.interaction.active_drag(session).map_err(|source| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("{source:?}"),
            }
        })?;
        let outcome = if let Some(preview) =
            self.retained_position_preview(cause, drag, position, &mut evaluation)?
        {
            let preview = preview.public().clone();
            self.interaction
                .set_drop_affordance(session, evaluation.affordance)
                .map_err(|source| EngineError::PointerInteractionInvariant {
                    cause,
                    detail: format!("retained drag lost its affordance: {source:?}"),
                })?;
            Some(InteractionOutcome::PreviewUpdated {
                session,
                preview: Some(preview),
                status: PreviewResolutionStatus::Resolved,
            })
        } else {
            self.apply_preview_evaluation(cause, owner, session, evaluation, interaction_events)?
        };
        self.advance_drag_position(cause, session, position)?;
        Ok(outcome)
    }

    pub(super) fn advance_drag_position(
        &mut self,
        cause: ReductionCause,
        session: crate::interaction::DragSessionId,
        position: ManipulationPositionKey,
    ) -> Result<(), EngineError> {
        if self.interaction.status() == (InteractionStatus::Dragging { session }) {
            self.interaction
                .set_drag_position(session, position)
                .map_err(|source| EngineError::PointerInteractionInvariant {
                    cause,
                    detail: format!("drag lost its exact advanced position: {source:?}"),
                })?;
        }
        Ok(())
    }
}
