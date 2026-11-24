//! Arming, updating, releasing and cancelling a drag gesture.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArmedDrag {
    pub(crate) session: DragSessionId,
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) payload: MovePayload,
    pub(crate) source_surface: SurfaceId,
    pub(crate) initial_pointer: Option<LogicalPoint>,
    pub(crate) journal_threshold_origin: JournalDragThresholdOrigin,
    pub(crate) complete_root: Option<NodeSource>,
    pub(crate) partial_detachable: bool,
    pub(crate) origin: FrozenDragOrigin,
    pub(crate) source_validated_at: WorkspaceVersion,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) source_layout_facts: Option<Arc<PresentationLayoutFacts>>,
    pub(crate) journal_source_geometry: Option<JournalDragSourceGeometry>,
    pub(crate) continuation: Option<SceneGestureContinuation>,
}

/// Press-time coordinate authority used only to decide when a journal drag starts.
///
/// A desktop-global provider never creates a synthetic logical coordinate
/// system across surfaces. Its threshold remains source-logical policy, but
/// the comparison is performed in desktop physical pixels using the frozen
/// source scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum JournalDragThresholdOrigin {
    SurfaceLocal(LogicalPoint),
    HostLocal {
        surface: SurfaceId,
        position: LogicalPoint,
    },
    NativeLocal {
        binding: ViewportBinding,
        coordinate_generation: CoordinateGeneration,
        position: LogicalPoint,
    },
    DesktopPhysical {
        position: PhysicalPoint,
        source_scale: ScaleFactor,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct JournalDragSourceGeometry {
    pub(crate) source_rect: LogicalRect,
    pub(crate) minimum_size: LogicalSize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenContainedDragOrigin {
    pub(crate) surface: SurfaceId,
    pub(crate) root: RootId,
    pub(crate) floating: FloatingPresentationId,
    pub(crate) intent: ContainedDragIntent,
    pub(crate) source_rect: LogicalRect,
    pub(crate) source_roster: ContainedRosterSource,
    pub(crate) initial_pointer: LogicalPoint,
    pub(crate) minimum_size: LogicalSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainedDragIntent {
    MoveWindow,
    DockContent,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FrozenDragOrigin {
    Workspace,
    Contained(FrozenContainedDragOrigin),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragArmStart {
    pub(crate) epoch: WorkspaceEpoch,
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) payload: MovePayload,
    pub(crate) source_surface: SurfaceId,
    pub(crate) initial_pointer: Option<LogicalPoint>,
    pub(crate) journal_threshold_origin: JournalDragThresholdOrigin,
    pub(crate) complete_root: Option<NodeSource>,
    pub(crate) partial_detachable: bool,
    pub(crate) origin: FrozenDragOrigin,
    pub(crate) source_validated_at: WorkspaceVersion,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) source_layout_facts: Option<Arc<PresentationLayoutFacts>>,
    pub(crate) journal_source_geometry: Option<JournalDragSourceGeometry>,
    pub(crate) continuation: Option<SceneGestureContinuationDraft>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveDrag {
    pub(crate) session: DragSessionId,
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) payload: MovePayload,
    pub(crate) source_surface: SurfaceId,
    pub(crate) initial_pointer: Option<LogicalPoint>,
    pub(crate) complete_root: Option<NodeSource>,
    pub(crate) partial_detachable: bool,
    pub(crate) origin: FrozenDragOrigin,
    pub(crate) source_validated_at: WorkspaceVersion,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) source_layout_facts: Option<Arc<PresentationLayoutFacts>>,
    pub(crate) journal_source_geometry: Option<JournalDragSourceGeometry>,
    pub(crate) continuation: Option<SceneGestureContinuation>,
    pub(crate) timed: Option<DragTimedContinuation>,
    pub(crate) capture_acquired_at: Option<std::time::Duration>,
    pub(crate) journal_presentation_reservation: Option<JournalPresentationReservation>,
    pub(crate) contained_offer: Option<ContainedPresentationOffer>,
    pub(crate) surface_background_offer: Option<SurfaceBackgroundRootOffer>,
    pub(crate) native_offer: Option<NativePresentationOffer>,
    pub(crate) target: DragTargetState,
}

impl ActiveDrag {
    pub(crate) const fn preview(&self) -> Option<&PublishedPreview> {
        self.target.preview()
    }

    #[cfg(test)]
    pub(crate) const fn preview_status(&self) -> PreviewResolutionStatus {
        self.target.status()
    }

    pub(crate) const fn drop_affordance(&self) -> Option<&DropAffordance> {
        self.target.affordance()
    }

    pub(crate) const fn last_advanced_position(&self) -> Option<ManipulationPositionKey> {
        self.target.last_advanced()
    }

    pub(crate) const fn target_acquisition(&self) -> Option<TargetAcquisition> {
        self.target.acquisition()
    }

    pub(crate) fn magnetic_targeting_allowed(&self) -> bool {
        matches!(self.target.magnetic_source_gate, MagneticSourceGate::Open)
    }

    pub(crate) const fn current_pointer(&self) -> Option<LogicalPoint> {
        self.target.current_pointer(self.initial_pointer)
    }
}

/// Core-minted identities frozen to one journal drag session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JournalPresentationReservation {
    pub(crate) surface: SurfaceId,
    pub(crate) root: RootId,
    pub(crate) floating: FloatingPresentationId,
}

impl InteractionState {
    pub(crate) fn arm_drag(
        &mut self,
        start: DragArmStart,
    ) -> Result<(DragSessionId, Option<InteractionStatus>), InteractionCounterError> {
        let generation = self
            .last_drag_generation
            .checked_next()
            .ok_or(InteractionCounterError::DragGenerationExhausted)?;
        self.last_drag_generation = generation;
        let session = DragSessionId::new(start.epoch, generation);
        let continuation = start
            .continuation
            .map(|draft| draft.bind(SceneGestureSession::Drag(session)));
        let replaced = (self.status() != InteractionStatus::Idle).then_some(self.status());
        self.active = ActiveGesture::Armed(Box::new(ArmedDrag {
            session,
            owner: start.owner,
            journal_capture_authority: start.journal_capture_authority,
            button: start.button,
            payload: start.payload,
            source_surface: start.source_surface,
            initial_pointer: start.initial_pointer,
            journal_threshold_origin: start.journal_threshold_origin,
            complete_root: start.complete_root,
            partial_detachable: start.partial_detachable,
            origin: start.origin,
            source_validated_at: start.source_validated_at,
            presentation: start.presentation,
            source_layout_facts: start.source_layout_facts,
            journal_source_geometry: start.journal_source_geometry,
            continuation,
        }));
        Ok((session, replaced))
    }

    pub(crate) fn begin_drag(
        &mut self,
        session: DragSessionId,
        owner: GestureOwner,
        button: PointerButton,
    ) -> Result<(), InteractionRejection> {
        let ActiveGesture::Armed(armed) = &self.active else {
            return Err(self.session_rejection(session));
        };
        if armed.session != session {
            return Err(InteractionRejection::SessionMismatch);
        }
        if armed.owner != owner {
            return Err(InteractionRejection::owner_mismatch(armed.owner, owner));
        }
        if armed.button != button {
            return Err(InteractionRejection::ButtonMismatch);
        }
        let target = DragTargetState::new(&armed.origin);
        self.active = ActiveGesture::Dragging(Box::new(ActiveDrag {
            session: armed.session,
            owner: armed.owner,
            journal_capture_authority: armed.journal_capture_authority,
            button: armed.button,
            payload: armed.payload.clone(),
            source_surface: armed.source_surface,
            initial_pointer: armed.initial_pointer,
            complete_root: armed.complete_root.clone(),
            partial_detachable: armed.partial_detachable,
            origin: armed.origin.clone(),
            source_validated_at: armed.source_validated_at,
            presentation: armed.presentation,
            source_layout_facts: armed.source_layout_facts.clone(),
            journal_source_geometry: armed.journal_source_geometry,
            continuation: armed.continuation.clone(),
            timed: None,
            capture_acquired_at: None,
            journal_presentation_reservation: None,
            contained_offer: None,
            surface_background_offer: None,
            native_offer: None,
            target,
        }));
        Ok(())
    }

    /// Rebinds an active native-local drag after the caller proves desktop continuity.
    ///
    /// All fallible identity checks precede mutation so the source continuation
    /// and drag owner change together without retaining target facts from the old lane.
    pub(crate) fn upgrade_drag_to_desktop(
        &mut self,
        session: DragSessionId,
        expected_owner: GestureOwner,
    ) -> Result<(), InteractionRejection> {
        let ActiveGesture::Dragging(drag) = &mut self.active else {
            return Err(InteractionRejection::SessionMismatch);
        };
        if drag.session != session
            || drag.owner != expected_owner
            || !matches!(expected_owner.lane(), GestureLane::NativeLocal { .. })
            || drag.continuation.as_ref().is_some_and(|continuation| {
                continuation.session != SceneGestureSession::Drag(session)
                    || continuation.draft.owner != expected_owner
            })
        {
            return Err(InteractionRejection::SessionMismatch);
        }

        let owner = GestureOwner::new(expected_owner.stream(), GestureLane::Desktop);
        drag.owner = owner;
        if let Some(continuation) = &mut drag.continuation {
            continuation.draft.owner = owner;
        }
        drag.timed = None;
        drag.target.reset_coordinate_lane();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn freeze_journal_presentation_reservation(
        &mut self,
        session: DragSessionId,
        owner: GestureOwner,
        surface: SurfaceId,
        root: RootId,
        floating: FloatingPresentationId,
        anchor: crate::input::intent::SurfacePointer,
        requested_rect: LogicalRect,
        minimum_size: LogicalSize,
        reserve_background_root: bool,
    ) -> Result<(), InteractionCounterError> {
        let drag = self
            .active_drag_mut(session)
            .map_err(|_| InteractionCounterError::StateInvariant)?;
        if drag.owner != owner
            || drag.journal_presentation_reservation.is_some()
            || drag.contained_offer.is_some()
            || drag.surface_background_offer.is_some()
            || drag.native_offer.is_some()
        {
            return Err(InteractionCounterError::StateInvariant);
        }
        match &drag.origin {
            FrozenDragOrigin::Workspace => {
                if drag
                    .complete_root
                    .as_ref()
                    .is_some_and(|source| source.root() != root)
                    || (drag.complete_root.is_some() && reserve_background_root)
                    || (drag.complete_root.is_none() && !reserve_background_root)
                {
                    return Err(InteractionCounterError::StateInvariant);
                }
            }
            FrozenDragOrigin::Contained(origin) => {
                if drag.complete_root.as_ref().map(NodeSource::root) != Some(origin.root)
                    || root != origin.root
                    || floating != origin.floating
                    || anchor
                        != crate::input::intent::SurfacePointer::new(
                            origin.surface,
                            origin.initial_pointer,
                        )
                    || requested_rect != origin.source_rect
                    || minimum_size != origin.minimum_size
                    || reserve_background_root
                {
                    return Err(InteractionCounterError::StateInvariant);
                }
            }
        }
        drag.journal_presentation_reservation = Some(JournalPresentationReservation {
            surface,
            root,
            floating,
        });
        if matches!(drag.origin, FrozenDragOrigin::Workspace) {
            drag.contained_offer = Some(ContainedPresentationOffer::front(
                root,
                floating,
                anchor,
                requested_rect,
                minimum_size,
            ));
            drag.surface_background_offer =
                reserve_background_root.then_some(SurfaceBackgroundRootOffer::new(root));
        }
        Ok(())
    }

    pub(crate) fn active_drag(
        &self,
        session: DragSessionId,
    ) -> Result<&ActiveDrag, InteractionRejection> {
        match &self.active {
            ActiveGesture::Dragging(drag) if drag.session == session => Ok(drag),
            ActiveGesture::Armed(armed) if armed.session == session => {
                Err(InteractionRejection::DragNotBegun)
            }
            _ => Err(self.session_rejection(session)),
        }
    }

    pub(crate) fn armed_drag(
        &self,
        session: DragSessionId,
    ) -> Result<&ArmedDrag, InteractionRejection> {
        match &self.active {
            ActiveGesture::Armed(armed) if armed.session == session => Ok(armed),
            _ => Err(self.session_rejection(session)),
        }
    }

    pub(crate) fn active_drag_mut(
        &mut self,
        session: DragSessionId,
    ) -> Result<&mut ActiveDrag, InteractionRejection> {
        let rejection = self.session_rejection(session);
        match &mut self.active {
            ActiveGesture::Dragging(drag) if drag.session == session => Ok(drag),
            ActiveGesture::Armed(armed) if armed.session == session => {
                Err(InteractionRejection::DragNotBegun)
            }
            _ => Err(rejection),
        }
    }

    pub(crate) fn publish_preview(
        &mut self,
        session: DragSessionId,
        scene: SurfaceSceneStamp,
        visual: PreviewVisual,
        proof: PreviewProof,
        acquisition: Option<TargetAcquisition>,
        tab_strip: Option<Arc<crate::presentation::compiler::FutureTabStrip>>,
    ) -> Result<(InteractionPreview, bool), InteractionCounterError> {
        if !preview_acquisition_matches(scene, &proof, acquisition) {
            return Err(InteractionCounterError::StateInvariant);
        }
        if let Ok(drag) = self.active_drag(session)
            && let Some(existing) = drag.preview()
            && existing.public.token.scene == scene
            && existing.public.visual == visual
            && existing.proof == proof
            && existing.public.tab_strip == tab_strip
        {
            let existing = existing.public.clone();
            self.active_drag_mut(session)
                .map_err(|_| InteractionCounterError::StateInvariant)?
                .target
                .set_acquisition(acquisition);
            return Ok((existing, false));
        }
        let sequence = self
            .last_preview_sequence
            .checked_next()
            .ok_or(InteractionCounterError::PreviewSequenceExhausted)?;
        let preview = InteractionPreview {
            token: PreviewToken {
                session,
                scene,
                sequence,
            },
            visual,
            tab_strip,
        };
        self.last_preview_sequence = sequence;
        let drag = self
            .active_drag_mut(session)
            .map_err(|_| InteractionCounterError::StateInvariant)?;
        drag.target.publish(
            PublishedPreview {
                public: preview.clone(),
                proof,
                painted: false,
                target_layout_basis: None,
            },
            acquisition,
        );
        Ok((preview, true))
    }

    pub(crate) fn bind_preview_layout_basis(
        &mut self,
        session: DragSessionId,
        basis: Option<SurfaceSceneStamp>,
    ) -> Result<(), InteractionRejection> {
        let drag = self.active_drag_mut(session)?;
        if let Some(preview) = drag.target.preview_mut() {
            preview.target_layout_basis = basis;
        }
        Ok(())
    }

    pub(crate) fn clear_preview(
        &mut self,
        session: DragSessionId,
        status: PreviewResolutionStatus,
    ) -> Result<bool, InteractionRejection> {
        Ok(self.active_drag_mut(session)?.target.clear_preview(status))
    }

    pub(crate) fn set_drag_position(
        &mut self,
        session: DragSessionId,
        position: ManipulationPositionKey,
    ) -> Result<(), InteractionRejection> {
        self.active_drag_mut(session)?
            .target
            .set_last_advanced(position);
        Ok(())
    }

    pub(crate) fn reconcile_drag_target(
        &self,
        session: DragSessionId,
        observation: TargetObservation,
        point: Option<LogicalPoint>,
        release_hysteresis: f64,
    ) -> Result<TargetLatchDecision, InteractionRejection> {
        Ok(self.active_drag(session)?.target.reconcile_acquisition(
            observation,
            point,
            release_hysteresis,
        ))
    }

    pub(crate) fn invalidate_drag_position(
        &mut self,
        session: DragSessionId,
    ) -> Result<(), InteractionRejection> {
        self.active_drag_mut(session)?
            .target
            .invalidate_last_advanced();
        Ok(())
    }

    pub(crate) fn clear_drag_feedback(
        &mut self,
        session: DragSessionId,
    ) -> Result<bool, InteractionRejection> {
        let drag = self.active_drag_mut(session)?;
        Ok(drag
            .target
            .clear_feedback(PreviewResolutionStatus::Unavailable))
    }

    pub(crate) fn clear_drag_target_feedback(
        &mut self,
        session: DragSessionId,
    ) -> Result<bool, InteractionRejection> {
        Ok(self
            .active_drag_mut(session)?
            .target
            .clear_target_feedback(PreviewResolutionStatus::Unavailable))
    }

    pub(crate) fn set_drop_affordance(
        &mut self,
        session: DragSessionId,
        affordance: Option<DropAffordance>,
    ) -> Result<(), InteractionRejection> {
        self.active_drag_mut(session)?
            .target
            .set_affordance(affordance);
        Ok(())
    }

    pub(crate) fn observe_presented_preview(
        &mut self,
        surface: SurfaceId,
        token: PreviewToken,
    ) -> bool {
        let ActiveGesture::Dragging(drag) = &mut self.active else {
            return false;
        };
        let Some(preview) = drag.target.preview_mut() else {
            return false;
        };
        if preview.public.token != token || preview.public.visual.surface() != surface {
            return false;
        }
        let changed = !preview.painted;
        preview.painted = true;
        changed
    }

    pub(crate) fn take_drag_for_release(
        &mut self,
        session: DragSessionId,
        owner: GestureOwner,
        button: PointerButton,
    ) -> Result<ActiveDrag, InteractionRejection> {
        let drag = self.active_drag(session)?;
        if drag.owner != owner {
            return Err(InteractionRejection::owner_mismatch(drag.owner, owner));
        }
        if drag.button != button {
            return Err(InteractionRejection::ButtonMismatch);
        }
        let ActiveGesture::Dragging(drag) =
            std::mem::replace(&mut self.active, ActiveGesture::Idle)
        else {
            return Err(InteractionRejection::NoActiveGesture);
        };
        self.last_consumed_drag = Some(session);
        Ok(*drag)
    }

    pub(crate) fn cancel_drag(
        &mut self,
        session: DragSessionId,
    ) -> Result<InteractionStatus, InteractionRejection> {
        match &self.active {
            ActiveGesture::Armed(armed) if armed.session == session => {}
            ActiveGesture::Dragging(drag) if drag.session == session => {}
            _ => return Err(self.session_rejection(session)),
        }
        let status = self.status();
        self.active = ActiveGesture::Idle;
        Ok(status)
    }

    fn session_rejection(&self, session: DragSessionId) -> InteractionRejection {
        if self.last_consumed_drag == Some(session) {
            InteractionRejection::SessionConsumed { session }
        } else if self.status() == InteractionStatus::Idle {
            InteractionRejection::NoActiveGesture
        } else {
            InteractionRejection::SessionMismatch
        }
    }
}
