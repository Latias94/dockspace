//! Device-independent pointer interaction transaction.

use super::contained_geometry::{contained_manipulation_point, contained_move_geometry};
use super::*;
use crate::input::intent::PointerId;
use crate::interaction::targeting::{TargetLatchDecision, TargetObservation};
use crate::presentation::compiler::{contained_minimum_size, pane_outer_sizing};
use crate::presentation::manifest::PaneSizingKey;

mod click;
mod native;

mod motion;
mod press;
mod release;
mod targeting;

impl DockEngine {
    fn update_gesture_docking_enabled(
        &mut self,
        cause: ReductionCause,
        owner: GestureOwner,
        enabled: bool,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<bool, EngineError> {
        if self.interaction.active_owner() != Some(owner) {
            return Ok(false);
        }
        let changed = self.interaction.set_docking_enabled(enabled);
        if !enabled && let InteractionStatus::Dragging { session } = self.interaction.status() {
            self.clear_drag_continuation(session);
            let dock_preview = self
                .interaction
                .active_drag(session)
                .ok()
                .and_then(|drag| drag.preview())
                .is_some_and(|preview| matches!(preview.proof(), PreviewProof::Dock { .. }));
            if dock_preview {
                self.interaction
                    .clear_preview(session, PreviewResolutionStatus::KnownNone)
                    .map_err(|source| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!("{source:?}"),
                    })?;
                interaction_events.push(InteractionEvent::new_caused(
                    cause,
                    self.version,
                    InteractionEventKind::PreviewCleared { session },
                ));
            }
            self.interaction
                .set_drop_affordance(session, None)
                .map_err(|source| EngineError::PointerInteractionInvariant {
                    cause,
                    detail: format!("{source:?}"),
                })?;
        }
        Ok(changed)
    }

    pub(super) fn reduce_pointer_docking_enabled(
        &mut self,
        cause: ReductionCause,
        provider: PointerInputLease,
        pointer: PointerId,
        enabled: bool,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InputOutcome, EngineError> {
        let owner = self.interaction.active_owner().filter(|owner| {
            self.pointer_provider() == Some(provider)
                && owner.lease() == provider
                && owner.pointer() == pointer
                && matches!(
                    provider.scope(),
                    PointerProviderScope::HostLocal(_) | PointerProviderScope::SurfaceLocal(_)
                )
        });
        let changed = if let Some(owner) = owner {
            self.update_gesture_docking_enabled(cause, owner, enabled, interaction_events)?
        } else {
            false
        };
        Ok(InputOutcome::DockingEnabledUpdated { enabled, changed })
    }

    pub(super) fn reduce_native_docking_enabled(
        &mut self,
        cause: ReductionCause,
        provider: BackendIngressLease,
        binding: ViewportBinding,
        enabled: bool,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InputOutcome, EngineError> {
        let owner = self.native_gesture_directive_owner(provider, binding)?;
        let changed = if let Some(owner) = owner {
            self.update_gesture_docking_enabled(cause, owner, enabled, interaction_events)?
        } else {
            false
        };
        Ok(InputOutcome::DockingEnabledUpdated { enabled, changed })
    }

    fn native_gesture_directive_owner(
        &self,
        provider: BackendIngressLease,
        binding: ViewportBinding,
    ) -> Result<Option<GestureOwner>, EngineError> {
        self.require_managed_input_provider(provider)?;
        let window = binding.window();
        let current = self
            .viewport
            .registry()
            .window(window)
            .is_some_and(|record| record.is_locally_routeable());
        let keyboard_focus = self
            .viewport_focus
            .focus_observation()
            .is_some_and(|observation| {
                observation.focused()
                    == &Authority::Known(crate::platform::focus::GlobalFocusedWindow::Dock(window))
            });
        let source_matches = self
            .interaction
            .active_presentation_authority()
            .is_some_and(|authority| {
                matches!(
                    authority.presented.endpoint(),
                    crate::presentation::observation::HostPresentationEndpoint::Native(source)
                        if source.window() == window
                )
            });
        Ok(self.interaction.active_owner().filter(|owner| {
            current
                && owner.lease() == provider.pointer_provider()
                && (keyboard_focus || source_matches)
        }))
    }

    fn update_gesture_group_dock_mode(
        &mut self,
        cause: ReductionCause,
        owner: GestureOwner,
        mode: crate::model::TabGroupDockMode,
        events: &mut Vec<InteractionEvent>,
    ) -> Result<bool, EngineError> {
        if self.interaction.active_owner() != Some(owner) {
            return Ok(false);
        }
        let changed = self.interaction.set_group_dock_mode(mode);
        if changed && let InteractionStatus::Dragging { session } = self.interaction.status() {
            let is_dock = self
                .interaction
                .active_drag(session)
                .ok()
                .and_then(|drag| drag.preview())
                .is_some_and(|preview| matches!(preview.proof(), PreviewProof::Dock { .. }));
            if is_dock {
                self.interaction
                    .clear_preview(session, PreviewResolutionStatus::KnownNone)
                    .map_err(|source| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!("{source:?}"),
                    })?;
                events.push(InteractionEvent::new_caused(
                    cause,
                    self.version,
                    InteractionEventKind::PreviewCleared { session },
                ));
            }
            self.interaction
                .set_drop_affordance(session, None)
                .map_err(|source| EngineError::PointerInteractionInvariant {
                    cause,
                    detail: format!("{source:?}"),
                })?;
        }
        Ok(changed)
    }

    pub(super) fn reduce_pointer_group_dock_mode(
        &mut self,
        cause: ReductionCause,
        provider: PointerInputLease,
        pointer: PointerId,
        mode: crate::model::TabGroupDockMode,
        events: &mut Vec<InteractionEvent>,
    ) -> Result<InputOutcome, EngineError> {
        let owner = self.interaction.active_owner().filter(|owner| {
            self.pointer_provider() == Some(provider)
                && owner.lease() == provider
                && owner.pointer() == pointer
                && matches!(
                    provider.scope(),
                    PointerProviderScope::HostLocal(_) | PointerProviderScope::SurfaceLocal(_)
                )
        });
        let changed = match owner {
            Some(owner) => self.update_gesture_group_dock_mode(cause, owner, mode, events)?,
            None => false,
        };
        Ok(InputOutcome::GroupDockModeUpdated { mode, changed })
    }

    pub(super) fn reduce_native_group_dock_mode(
        &mut self,
        cause: ReductionCause,
        provider: BackendIngressLease,
        binding: ViewportBinding,
        mode: crate::model::TabGroupDockMode,
        events: &mut Vec<InteractionEvent>,
    ) -> Result<InputOutcome, EngineError> {
        let owner = self.native_gesture_directive_owner(provider, binding)?;
        let changed = match owner {
            Some(owner) => self.update_gesture_group_dock_mode(cause, owner, mode, events)?,
            None => false,
        };
        Ok(InputOutcome::GroupDockModeUpdated { mode, changed })
    }

    fn reduce_journal_pointer_edge(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        stream: PointerStreamId,
        capture_authority_after: Authority<PointerCaptureOwner>,
        edge: &PointerEdge,
        receipt: &ValidatedPointerReceiverReceipt,
        snapshot: &JournalPresentationSnapshot,
        desktop_delivery_route: Option<DesktopRouteValidation>,
        desktop_hover_route: Option<DesktopRouteValidation>,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<Vec<InteractionOutcome>, EngineError> {
        if stream.pointer() != edge.pointer() || receipt.candidate().sequence() != edge.sequence() {
            return Err(EngineError::ReductionCauseInvariant {
                detail: "journal edge, stream, and receipt sequence do not agree",
            });
        }
        let owner = self
            .interaction
            .active_owner()
            .filter(|owner| owner.stream() == stream)
            .or_else(|| self.journal_gesture_owner(stream, edge, desktop_delivery_route));
        let Some(owner) = owner else {
            return match edge.kind() {
                PointerEdgeKind::Scrolled(scroll) => {
                    self.reduce_scroll_edge(cause, stream, edge, scroll, receipt, snapshot)
                }
                PointerEdgeKind::StreamCancelled(reason) => self.terminate_journal_stream(
                    cause,
                    stream,
                    if reason
                        == crate::input::journal::PointerStreamCancelReason::ExplicitCancellation
                    {
                        InteractionCancelReason::ExplicitCancellation
                    } else {
                        InteractionCancelReason::PointerStreamCancelled
                    },
                    ScrollTerminationReason::StreamCancelled,
                    interaction_events,
                ),
                PointerEdgeKind::StreamEnded | PointerEdgeKind::ContactEnded(_) => self
                    .terminate_journal_stream(
                        cause,
                        stream,
                        InteractionCancelReason::PointerStreamEnded,
                        ScrollTerminationReason::StreamEnded,
                        interaction_events,
                    ),
                _ => Ok(Vec::new()),
            };
        };
        let status_before = self.interaction.status();
        let matching_owner = self.interaction.active_owner() == Some(owner);
        if let PointerEdgeKind::Continuation(token) = edge.kind()
            && !self.journal_continuation_is_current(stream, token)
        {
            // A revoked timer cannot reinterpret an old lane or ask receivers
            // about a newer gesture. Its real native capture loss still counts.
            if matching_owner
                && matches!(owner.lease().scope(), PointerProviderScope::NativeJoined(_))
            {
                return self.reduce_journal_capture_change(
                    cause,
                    owner,
                    capture_authority_after,
                    interaction_events,
                );
            }
            return Ok(Vec::new());
        }
        if let PointerEdgeKind::CaptureAcquired(token) = edge.kind()
            && !self.journal_capture_acquisition_is_current(stream, token)
        {
            if matching_owner
                && matches!(owner.lease().scope(), PointerProviderScope::NativeJoined(_))
            {
                return self.reduce_journal_capture_change(
                    cause,
                    owner,
                    capture_authority_after,
                    interaction_events,
                );
            }
            return Ok(Vec::new());
        }
        if matching_owner
            && matches!(
                edge.kind(),
                PointerEdgeKind::Moved
                    | PointerEdgeKind::Continuation(_)
                    | PointerEdgeKind::CaptureAcquired(_)
                    | PointerEdgeKind::ButtonReleased(PointerButton::Primary)
                    | PointerEdgeKind::ContactEnded(PointerButton::Primary)
            )
            && (!self.journal_edge_matches_gesture_lane(owner, edge)
                || (matches!(owner.lane(), GestureLane::NativeLocal { .. })
                    && super::pointer_proof::validated_native_local_route(self, edge).is_none()))
        {
            let mut outcomes = self
                .cancel_gesture_owner(
                    cause,
                    owner,
                    InteractionCancelReason::CoordinateAuthorityUnavailable,
                    interaction_events,
                )?
                .into_iter()
                .collect::<Vec<_>>();
            if matches!(edge.kind(), PointerEdgeKind::ContactEnded(_)) {
                outcomes.extend(
                    self.terminate_scroll_stream(stream, ScrollTerminationReason::StreamEnded),
                );
            }
            return Ok(outcomes);
        }
        if matching_owner
            && !matches!(
                edge.kind(),
                PointerEdgeKind::ButtonPressed(PointerButton::Primary)
            )
            && let Some(enabled) = edge.metadata().docking_enabled()
        {
            self.update_gesture_docking_enabled(cause, owner, enabled, interaction_events)?;
        }
        if matching_owner
            && !matches!(
                edge.kind(),
                PointerEdgeKind::ButtonPressed(PointerButton::Primary)
            )
            && let Some(mode) = edge.metadata().group_dock_mode()
        {
            self.update_gesture_group_dock_mode(cause, owner, mode, interaction_events)?;
        }
        let event_start = interaction_events.len();
        let mut outcomes = match edge.kind() {
            PointerEdgeKind::ButtonPressed(PointerButton::Primary) => {
                let press = self.reduce_journal_primary_press(
                    cause,
                    focus_causal,
                    owner,
                    capture_authority_after,
                    edge,
                    receipt,
                    snapshot,
                    desktop_delivery_route,
                    policy,
                    events,
                )?;
                let mut outcomes = if self.interaction.status() == InteractionStatus::Idle {
                    Vec::new()
                } else {
                    self.cancel_pending_release_obligations_caused(
                        cause,
                        InteractionCancelReason::ReplacedByNewGesture,
                        interaction_events,
                    )
                };
                outcomes.extend(press);
                Ok(outcomes)
            }
            PointerEdgeKind::Moved => self.reduce_journal_move(
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
            ),
            PointerEdgeKind::Continuation(token) => self.reduce_journal_continuation(
                cause,
                owner,
                token,
                edge,
                receipt,
                snapshot,
                policy,
                events,
                interaction_events,
            ),
            PointerEdgeKind::CaptureAcquired(token) => {
                self.reduce_journal_capture_acquired(cause, owner, token, edge, interaction_events)
            }
            PointerEdgeKind::ButtonReleased(PointerButton::Primary)
            | PointerEdgeKind::ContactEnded(PointerButton::Primary) => self
                .reduce_journal_primary_release(
                    cause,
                    focus_causal,
                    owner,
                    edge,
                    receipt,
                    snapshot,
                    desktop_delivery_route,
                    desktop_hover_route,
                    policy,
                    events,
                    interaction_events,
                ),
            PointerEdgeKind::CaptureChanged => self.reduce_journal_capture_change(
                cause,
                owner,
                capture_authority_after,
                interaction_events,
            ),
            PointerEdgeKind::StreamCancelled(reason) => self.terminate_journal_stream(
                cause,
                stream,
                if reason == crate::input::journal::PointerStreamCancelReason::ExplicitCancellation
                {
                    InteractionCancelReason::ExplicitCancellation
                } else {
                    InteractionCancelReason::PointerStreamCancelled
                },
                ScrollTerminationReason::StreamCancelled,
                interaction_events,
            ),
            PointerEdgeKind::StreamEnded => self.terminate_journal_stream(
                cause,
                stream,
                InteractionCancelReason::PointerStreamEnded,
                ScrollTerminationReason::StreamEnded,
                interaction_events,
            ),
            PointerEdgeKind::Scrolled(scroll) => {
                let mut outcomes = if matching_owner
                    && matches!(owner.lease().scope(), PointerProviderScope::NativeJoined(_))
                {
                    self.reduce_journal_capture_change(
                        cause,
                        owner,
                        capture_authority_after,
                        interaction_events,
                    )?
                } else {
                    Vec::new()
                };
                outcomes.extend(
                    self.reduce_scroll_edge(cause, stream, edge, scroll, receipt, snapshot)?,
                );
                Ok(outcomes)
            }
            PointerEdgeKind::ButtonPressed(_) | PointerEdgeKind::ButtonReleased(_)
                if matching_owner
                    && matches!(owner.lease().scope(), PointerProviderScope::NativeJoined(_)) =>
            {
                self.reduce_journal_capture_change(
                    cause,
                    owner,
                    capture_authority_after,
                    interaction_events,
                )
            }
            PointerEdgeKind::ButtonPressed(_)
            | PointerEdgeKind::ButtonReleased(_)
            | PointerEdgeKind::ContactEnded(_) => Ok(Vec::new()),
        }?;
        if status_before == InteractionStatus::Idle
            && matches!(
                edge.kind(),
                PointerEdgeKind::ButtonPressed(PointerButton::Primary)
            )
            && self.interaction.active_owner() == Some(owner)
        {
            self.update_gesture_docking_enabled(
                cause,
                owner,
                edge.metadata().docking_enabled().unwrap_or(true),
                interaction_events,
            )?;
            self.update_gesture_group_dock_mode(
                cause,
                owner,
                edge.metadata().group_dock_mode().unwrap_or_default(),
                interaction_events,
            )?;
        }
        if matches!(edge.kind(), PointerEdgeKind::ContactEnded(_)) {
            outcomes.extend(self.terminate_journal_stream(
                cause,
                stream,
                InteractionCancelReason::PointerStreamEnded,
                ScrollTerminationReason::StreamEnded,
                interaction_events,
            )?);
        }
        for outcome in &outcomes {
            let kind = match outcome {
                InteractionOutcome::ReleasePending { session, .. } => {
                    Some(InteractionEventKind::ReleasePending { session: *session })
                }
                _ => None,
            };
            if let Some(kind) = kind {
                interaction_events.push(InteractionEvent::new_caused(cause, self.version, kind));
            }
        }
        let consumed_release = matching_owner
            && matches!(
                edge.kind(),
                PointerEdgeKind::ButtonReleased(PointerButton::Primary)
                    | PointerEdgeKind::ContactEnded(PointerButton::Primary)
            )
            && self.interaction.status() == InteractionStatus::Idle;
        let unowned_press = status_before == InteractionStatus::Idle
            && matches!(
                edge.kind(),
                PointerEdgeKind::ButtonPressed(PointerButton::Primary)
            );
        let has_terminal = interaction_events[event_start..].iter().any(|event| {
            matches!(
                event.kind(),
                InteractionEventKind::Cancelled { .. }
                    | InteractionEventKind::Delivered { .. }
                    | InteractionEventKind::ResizeDelivered { .. }
                    | InteractionEventKind::ContainedTransformDelivered { .. }
                    | InteractionEventKind::ReleasePending { .. }
                    | InteractionEventKind::NativePresentationRequested(_)
            )
        });
        if (consumed_release || unowned_press) && !has_terminal {
            let rejection = outcomes.iter().rev().find_map(|outcome| match outcome {
                InteractionOutcome::Rejected(reason) => Some(
                    crate::interaction::ManipulationRejection::from_interaction(reason),
                ),
                InteractionOutcome::ProductActionRejected(reason) => Some(
                    crate::interaction::ManipulationRejection::from_product(*reason),
                ),
                _ => None,
            });
            if let Some(reason) = rejection {
                interaction_events.push(InteractionEvent::new_caused(
                    cause,
                    self.version,
                    InteractionEventKind::Rejected {
                        status: status_before,
                        reason,
                    },
                ));
            }
        }
        Ok(outcomes)
    }

    fn terminate_journal_stream(
        &mut self,
        cause: ReductionCause,
        stream: PointerStreamId,
        interaction_reason: InteractionCancelReason,
        scroll_reason: ScrollTerminationReason,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<Vec<InteractionOutcome>, EngineError> {
        let mut outcomes = self.terminate_scroll_stream(stream, scroll_reason);
        if let Some(owner) = self
            .interaction
            .active_owner()
            .filter(|owner| owner.stream() == stream)
        {
            if let Some(outcome) =
                self.cancel_gesture_owner(cause, owner, interaction_reason, interaction_events)?
            {
                outcomes.push(outcome);
            }
        }
        Ok(outcomes)
    }

    pub(super) fn reduce_host_pointer_protocol(
        &mut self,
        tick: ReducerTickId,
        protocol: PreparedHostPointerProtocol,
        frozen_pointer_outputs: &BTreeMap<SurfaceId, PointerReceiverPresentedOutput>,
        frozen_pointer_presentations: &BTreeMap<SurfaceId, JournalSurfacePresentation>,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<Vec<crate::engine::transition::ReducedPointerEdge>, EngineError> {
        let (stamp, provider, journal, candidates, receipts, handoff) = protocol.into_parts();
        // A SurfaceLocal lease freezes a native endpoint, including its window
        // incarnation. The host frame can span a platform or workspace
        // transition, so creation-time validation is not sufficient here.
        // Revalidate at every causal journal segment before any current output
        // or receipt can be joined to the old lease.
        self.validate_pointer_provider_scope(provider.scope())?;
        let prepared = self
            .pointer_journal
            .prepare_candidate(provider, journal)
            .map_err(|source| EngineError::PointerJournal { source })?;
        let edge_view = if let Some(edge) = prepared.journal().edges().first() {
            if let Some(projection) = handoff
                && self.prepare_native_handoff_projection(projection.stream(), edge) != handoff
            {
                return Err(EngineError::ReductionCauseInvariant {
                    detail: "native handoff preparation differs from its ordered segment",
                });
            }
            handoff.map(|projection| projection.edge_view(edge))
        } else {
            None
        };
        let validated_receipts = candidates
            .validate_against_presented_outputs(receipts, frozen_pointer_outputs)
            .map_err(|source| EngineError::PointerReceiverReceipt { source })?;
        let source_edge = edge_view
            .as_ref()
            .or_else(|| prepared.journal().edges().first());
        let native_view = source_edge
            .zip(validated_receipts.receipts().first())
            .map(|(edge, receipt)| resolve_native_pointer_edge(self, edge, receipt))
            .transpose()?;
        let effective_edges = native_view
            .as_ref()
            .map_or(prepared.journal().edges(), std::slice::from_ref);
        let desktop_hover_routes = effective_edges
            .iter()
            .filter_map(|edge| {
                edge.desktop_route().map(|route| {
                    (
                        edge.sequence(),
                        route.validate_against_registry(
                            self.authority_domain,
                            self.viewport.registry(),
                        ),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        let desktop_delivery_routes = effective_edges
            .iter()
            .filter_map(|edge| {
                validated_desktop_delivery_route(self, edge).map(|route| (edge.sequence(), route))
            })
            .collect::<BTreeMap<_, _>>();
        let snapshot = JournalPresentationSnapshot::from_validated_receipts(
            provider,
            frozen_pointer_presentations,
            &validated_receipts,
        )
        .map_err(|source| EngineError::JournalPresentationSnapshot {
            detail: source.to_string(),
        })?;
        self.validate_pointer_receiver_geometry(
            provider,
            effective_edges,
            &validated_receipts,
            &snapshot,
            &desktop_hover_routes,
            &desktop_delivery_routes,
        )?;
        let commit = self
            .pointer_journal
            .commit_prepared(prepared)
            .map_err(|source| EngineError::PointerJournal { source })?;
        let receipts_by_sequence = validated_receipts
            .receipts()
            .iter()
            .map(|receipt| (receipt.candidate().sequence(), receipt))
            .collect::<BTreeMap<_, _>>();
        let accepted_by_sequence = commit
            .accepted_edges()
            .iter()
            .map(|accepted| (accepted.ticket().sequence(), accepted))
            .collect::<BTreeMap<_, _>>();
        let ordinal = stamp.ordinal();
        let mut reduced = Vec::with_capacity(commit.journal().len());
        for edge in commit.journal().edges() {
            let accepted = accepted_by_sequence.get(&edge.sequence()).copied().ok_or(
                EngineError::ReductionCauseInvariant {
                    detail: "committed journal edge has no accepted stream ticket",
                },
            )?;
            let receipt = receipts_by_sequence.get(&edge.sequence()).copied().ok_or(
                EngineError::ReductionCauseInvariant {
                    detail: "committed journal edge has no validated receiver receipt",
                },
            )?;
            if accepted.ticket().sequence() != edge.sequence()
                || accepted.stream().pointer() != edge.pointer()
                || handoff.is_some_and(|projection| projection.stream() != accepted.stream())
            {
                return Err(EngineError::ReductionCauseInvariant {
                    detail: "accepted pointer identity does not match committed journal edge",
                });
            }
            let cause = ReductionCause::PointerEdge {
                tick,
                ordinal,
                stream: accepted.stream(),
                ticket: accepted.ticket(),
            };
            let focus_generation = self.last_focus_reducer_generation.checked_next().ok_or(
                EngineError::ReductionCauseInvariant {
                    detail: "pointer focus causal generation is exhausted",
                },
            )?;
            self.last_focus_reducer_generation = focus_generation;
            let focus_causal = FocusCausalStamp::new(focus_generation, cause);
            let effective = native_view.as_ref().unwrap_or(edge);
            let interaction = self.report_reduction(
                events,
                interaction_events,
                |engine, events, interaction_events| {
                    if let Some(projection) = handoff {
                        engine.apply_native_handoff_projection(projection, edge)?;
                    }
                    let mut interaction = engine.reduce_journal_pointer_edge(
                        cause,
                        focus_causal,
                        accepted.stream(),
                        accepted.capture_authority_for_reduction(),
                        effective,
                        receipt,
                        &snapshot,
                        desktop_delivery_routes.get(&edge.sequence()).copied(),
                        desktop_hover_routes.get(&edge.sequence()).copied(),
                        policy,
                        events,
                        interaction_events,
                    )?;
                    if let Some(outcome) =
                        engine.cancel_revoked_presentation_caused(cause, interaction_events)?
                    {
                        interaction.push(outcome);
                    }
                    Ok(interaction)
                },
            )?;
            reduced.push(crate::engine::transition::ReducedPointerEdge::new(
                tick,
                ordinal,
                accepted.stream(),
                accepted.ticket(),
                edge.clone(),
                accepted.button_authority_after(),
                accepted.capture_authority_after(),
                interaction,
            ));
        }
        Ok(reduced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{PlatformCapabilityReason, PlatformRequirement};

    #[test]
    fn unsupported_physical_drag_capability_has_a_typed_preview_status() {
        let capability = PlatformCapability::unsupported(
            PlatformRequirement::PointerHitTestControl,
            PlatformCapabilityReason::BackendUnsupported,
        );

        assert_eq!(
            DockEngine::physical_drag_preview_status(capability),
            PreviewResolutionStatus::NativeCapabilityUnavailable
        );
    }
}
