//! Preview qualification and terminal pointer delivery.

use super::*;

impl DockEngine {
    pub(super) fn reduce_journal_primary_release(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        owner: GestureOwner,
        edge: &PointerEdge,
        receipt: &ValidatedPointerReceiverReceipt,
        snapshot: &JournalPresentationSnapshot,
        desktop_delivery_route: Option<DesktopRouteValidation>,
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
            true,
            interaction_events,
        )?;
        let capture_unavailable = capture_outcome.is_none();
        if let Some(outcome) = capture_outcome {
            outcomes.push(outcome);
        }
        if !capture_authorized {
            if capture_unavailable {
                if let Some(outcome) = self.cancel_gesture_owner(
                    cause,
                    owner,
                    InteractionCancelReason::CaptureAuthorityUnavailable,
                    interaction_events,
                )? {
                    outcomes.push(outcome);
                }
            }
            return Ok(outcomes);
        }
        if let Some(outcome) = self.cancel_revoked_presentation_caused(cause, interaction_events)? {
            outcomes.push(outcome);
            return Ok(outcomes);
        }
        match self.interaction.status() {
            InteractionStatus::Pressed { session } => {
                outcomes.push(self.finish_journal_click_release(
                    cause,
                    focus_causal,
                    owner,
                    session,
                    edge,
                    receipt,
                    snapshot,
                    desktop_delivery_route,
                    policy,
                    events,
                    interaction_events,
                )?);
            }
            InteractionStatus::Armed { .. } => {
                if let Some(outcome) = self.cancel_gesture_owner(
                    cause,
                    owner,
                    InteractionCancelReason::ReleasedBeforeDrag,
                    interaction_events,
                )? {
                    outcomes.push(outcome);
                }
            }
            InteractionStatus::Dragging { session } => {
                let drag = self.interaction.active_drag(session).map_err(|source| {
                    EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!("{source:?}"),
                    }
                })?;
                if !self.drag_source_presentation_allows_targeting(drag) {
                    outcomes.push(self.cancel_drag_release(
                        cause,
                        owner,
                        InteractionCancelReason::SceneUnavailable,
                        interaction_events,
                    )?);
                    return Ok(outcomes);
                }
                let position = journal_position_key(self, owner.stream(), edge);
                // Release is always resolved against its own edge facts. This
                // keeps same-point blocker/receiver changes from authorizing a
                // cached preview, while the outside-all native route remains
                // independently resolvable without a hover receipt.
                let mut evaluation = self.resolve_journal_drag_preview(
                    cause,
                    drag,
                    position,
                    edge,
                    receipt,
                    snapshot,
                    desktop_hover_route,
                    policy,
                )?;
                if let Some(preview) =
                    self.retained_position_preview(cause, drag, position, &mut evaluation)?
                {
                    evaluation.decision = Self::frozen_drag_release_decision(preview);
                    evaluation.observation = drag
                        .target_acquisition()
                        .map_or(TargetObservation::Unavailable, |target| {
                            target.observation()
                        });
                    evaluation.target_layout_basis = preview.target_layout_basis();
                }
                let outcome = self.finish_drag_release(
                    cause,
                    focus_causal,
                    owner,
                    session,
                    Some(position),
                    evaluation,
                    policy,
                    &mut outcomes,
                    events,
                    interaction_events,
                )?;
                outcomes.push(outcome);
            }
            InteractionStatus::Resizing { session } => {
                outcomes.push(self.finish_journal_resize_release(
                    cause,
                    owner,
                    session,
                    edge,
                    policy,
                    events,
                    interaction_events,
                )?);
            }
            InteractionStatus::ContainedTransforming { session } => {
                outcomes.extend(self.finish_journal_contained_transform_release(
                    cause,
                    owner,
                    session,
                    edge,
                    policy,
                    events,
                    interaction_events,
                )?);
            }
            InteractionStatus::Idle => {}
        }
        Ok(outcomes)
    }

    fn release_decision_matches_preview(
        preview: &crate::interaction::PublishedPreview,
        release_decision: &PreviewDecision,
    ) -> bool {
        let PreviewDecision::Publish {
            scene,
            visual,
            proof,
            tab_strip,
        } = release_decision
        else {
            return false;
        };
        preview.public().token().scene() == *scene
            && preview.public().visual() == visual
            && preview.proof() == proof.as_ref()
            && preview.public().tab_strip() == tab_strip.as_deref()
    }

    pub(super) fn frozen_drag_release_decision(
        preview: &crate::interaction::PublishedPreview,
    ) -> PreviewDecision {
        PreviewDecision::Publish {
            scene: preview.public().token().scene(),
            visual: preview.public().visual().clone(),
            proof: Box::new(preview.proof().clone()),
            tab_strip: preview.public().shared_tab_strip(),
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine) fn finish_drag_release_after_evaluation(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        owner: GestureOwner,
        session: crate::interaction::DragSessionId,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        let drag = self.interaction.active_drag(session).map_err(|source| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("release lost its active drag: {source:?}"),
            }
        })?;
        if !self.drag_source_presentation_allows_targeting(drag) {
            return self.cancel_drag_release(
                cause,
                owner,
                InteractionCancelReason::SceneUnavailable,
                interaction_events,
            );
        }
        let evaluation = PreviewEvaluation::new(
            drag.preview().map_or(
                PreviewDecision::Clear(drag.preview_status()),
                Self::frozen_drag_release_decision,
            ),
            drag.drop_affordance().cloned(),
            drag.target_acquisition()
                .map_or(TargetObservation::Unavailable, |target| {
                    target.observation()
                }),
            drag.preview()
                .and_then(|preview| preview.target_layout_basis()),
        );
        self.finish_drag_release(
            cause,
            focus_causal,
            owner,
            session,
            drag.last_advanced_position(),
            evaluation,
            policy,
            &mut Vec::new(),
            events,
            interaction_events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_drag_release(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        owner: GestureOwner,
        session: crate::interaction::DragSessionId,
        position: Option<ManipulationPositionKey>,
        mut evaluation: PreviewEvaluation,
        policy: &DockPolicySnapshot,
        outcomes: &mut Vec<InteractionOutcome>,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        let drag = self.interaction.active_drag(session).map_err(|source| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("release lost its active drag: {source:?}"),
            }
        })?;
        if drag.owner != owner {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::SessionMismatch,
            ));
        }
        if let Some(reason) = self.drag_release_cancellation(drag, &evaluation) {
            return self.cancel_drag_release(cause, owner, reason, interaction_events);
        }
        let existing = drag.preview().filter(|preview| {
            Self::release_decision_matches_preview(preview, &evaluation.decision)
        });
        let awaits_preview = !existing.is_some_and(|preview| preview.painted())
            && self.drag_release_has_native_resource_obligation(drag, &evaluation.decision);
        let prepared_mutation = evaluation.prepared_mutation.take();
        let release_decision = if awaits_preview {
            // A release-time preview is published only if it becomes an exact
            // presentation obligation. An already published token is reused.
            let newly_published = existing.is_none();
            let decision = evaluation.decision.clone();
            let outcome = self.apply_preview_evaluation(
                cause,
                owner,
                session,
                evaluation,
                interaction_events,
            )?;
            if let Some(position) = position {
                self.advance_drag_position(cause, session, position)?;
            }
            if newly_published {
                outcomes.extend(outcome);
            }
            decision
        } else {
            evaluation.decision
        };
        let drag =
            match self
                .interaction
                .take_drag_for_release(session, owner, PointerButton::Primary)
            {
                Ok(drag) => drag,
                Err(error) => return Ok(InteractionOutcome::Rejected(error)),
            };
        let physical_route = if owner.is_desktop() {
            let route = self
                .viewport
                .drag_source(owner.pointer())
                .filter(|binding| binding.surface() == drag.source_surface)
                .zip(self.viewport.drag_source_input(owner.pointer()));
            self.viewport
                .end_drag_routing(owner.pointer())
                .map_err(|source| EngineError::Viewport {
                    input: self.last_input,
                    source,
                })?;
            route
        } else {
            None
        };
        if !matches!(release_decision, PreviewDecision::Publish { .. }) {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::PreviewMissing,
            ));
        }
        if awaits_preview {
            return self.defer_drag_release(
                cause,
                focus_causal,
                session,
                drag,
                physical_route,
                release_decision,
                policy,
            );
        }
        self.deliver_drag_release(
            cause,
            focus_causal,
            session,
            drag,
            release_decision,
            prepared_mutation,
            policy,
            events,
            interaction_events,
        )
    }

    pub(super) fn cancel_drag_release(
        &mut self,
        cause: ReductionCause,
        owner: GestureOwner,
        reason: InteractionCancelReason,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        self.cancel_gesture_owner(cause, owner, reason, interaction_events)?
            .ok_or(EngineError::ReductionCauseInvariant {
                detail: "validated release failed to cancel its owner",
            })
    }

    fn drag_release_has_native_resource_obligation(
        &self,
        drag: &crate::interaction::ActiveDrag,
        decision: &PreviewDecision,
    ) -> bool {
        let PreviewDecision::Publish { visual, proof, .. } = decision else {
            return false;
        };
        if matches!(proof.as_ref(), PreviewProof::Native { .. }) {
            return true;
        }
        if visual.surface() == drag.source_surface
            || !self
                .bound_surface_recoveries
                .contains_key(&drag.source_surface)
        {
            return false;
        }
        let source_items = self.surface_items(drag.source_surface);
        let moved_items = match &drag.payload {
            MovePayload::Item(source) => BTreeSet::from([source.item()]),
            MovePayload::Tabs(source) | MovePayload::Subtree(source) => self
                .workspace
                .collect_items_in_subtree(source.node())
                .into_iter()
                .collect(),
        };
        source_items == moved_items
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine) fn deliver_drag_release(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        session: crate::interaction::DragSessionId,
        drag: crate::interaction::ActiveDrag,
        release_decision: PreviewDecision,
        prepared_mutation: Option<PreparedDropMutation>,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        let PreviewDecision::Publish { proof, .. } = release_decision else {
            return Err(EngineError::PointerInteractionInvariant {
                cause,
                detail: "validated drag release lost its publish decision".to_owned(),
            });
        };
        let pane_focus = match self.freeze_payload_focus(&drag.payload) {
            Ok(pane_focus) => match &drag.payload {
                MovePayload::Item(source) => PaneFocusDisposition::Set(source.item()),
                MovePayload::Tabs(_) | MovePayload::Subtree(_) => pane_focus,
            },
            Err(error) => {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(error),
                ));
            }
        };
        let (command, kind, focus_surface) = match *proof {
            PreviewProof::Dock { target, command } => {
                (command, WorkspaceDeliveryKind::Dock, target.surface())
            }
            PreviewProof::Contained {
                command,
                proposal,
                fallback,
            } => (
                command,
                if fallback {
                    WorkspaceDeliveryKind::ContainedFallback
                } else {
                    WorkspaceDeliveryKind::Contained
                },
                proposal.surface(),
            ),
            PreviewProof::Native { command, offer } => {
                let presentation = drag.presentation;
                return self.finish_journal_native_delivery(
                    cause,
                    focus_causal,
                    crate::interaction::NativeDragTearOff::new(drag.owner.stream(), session),
                    presentation.presented,
                    drag.payload.clone(),
                    pane_focus,
                    command,
                    offer,
                    policy,
                    interaction_events,
                );
            }
            PreviewProof::PresentationRehome { .. } => {
                return Err(EngineError::ReductionCauseInvariant {
                    detail: "programmatic presentation rehome proof entered pointer delivery",
                });
            }
        };
        let activation = match &drag.payload {
            MovePayload::Item(source) => JournalDeliveryActivation::SelectItem {
                item: source.item(),
                source_root: source.root(),
            },
            MovePayload::Tabs(_) | MovePayload::Subtree(_) => JournalDeliveryActivation::None,
        };
        let (outcome, changed) = match self.apply_journal_workspace_command_with_authority(
            cause,
            &command,
            policy,
            WorkspacePublicationAuthority::Ordinary,
            activation,
            prepared_mutation,
            events,
        )? {
            Ok(result) => result,
            Err(error) => {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(error),
                ));
            }
        };
        if let Some(binding) = self
            .viewport
            .viewport(focus_surface)
            .filter(|record| record.can_accept_activation())
            .map(crate::platform::registry::ViewportRecord::binding)
        {
            let _ = self.start_viewport_activation(
                ViewportActivationRequest::drop_committed(binding, pane_focus),
                focus_causal,
                events,
            )?;
        } else if let PaneFocusDisposition::Set(item) = pane_focus {
            self.request_product_item_focus(item, focus_causal)?;
        }
        interaction_events.push(InteractionEvent::new_caused(
            cause,
            self.version,
            InteractionEventKind::Delivered { session, kind },
        ));
        Ok(InteractionOutcome::DragDelivered {
            session,
            delivery: InteractionDelivery::Workspace {
                kind,
                outcome,
                changed,
            },
        })
    }
}
