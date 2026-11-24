//! Contained floating-window movement and resizing.

use super::*;

/// Opaque identity of one exact contained transform preview publication.
///
/// This token is intentionally distinct from [`PreviewToken`], so a docking
/// preview acknowledgement cannot satisfy a contained transform release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContainedTransformPreviewToken {
    session: ContainedTransformSessionId,
    scene: SurfaceSceneStamp,
    sequence: PreviewSequence,
}

impl ContainedTransformPreviewToken {
    /// Returns the transform session which owns this preview.
    #[must_use]
    pub const fn session(self) -> ContainedTransformSessionId {
        self.session
    }

    /// Returns the sealed scene against which this preview was resolved.
    #[must_use]
    pub const fn scene(self) -> SurfaceSceneStamp {
        self.scene
    }
}

/// Exact contained rectangle which must be painted before transform delivery.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContainedTransformPreview {
    token: ContainedTransformPreviewToken,
    surface: SurfaceId,
    root: RootId,
    floating: FloatingPresentationId,
    rect: LogicalRect,
}

impl ContainedTransformPreview {
    /// Returns the opaque preview token.
    #[must_use]
    pub const fn token(self) -> ContainedTransformPreviewToken {
        self.token
    }

    /// Returns the ready host surface.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    /// Returns the contained root.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.root
    }

    /// Returns the contained presentation identity.
    #[must_use]
    pub const fn floating(self) -> FloatingPresentationId {
        self.floating
    }

    /// Returns the exact rectangle which must be painted.
    #[must_use]
    pub const fn rect(self) -> LogicalRect {
        self.rect
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PublishedContainedTransformPreview {
    public: ContainedTransformPreview,
    placement: ContainedTransformPlacement,
    pub(super) painted: bool,
}

impl PublishedContainedTransformPreview {
    pub(crate) const fn public(&self) -> &ContainedTransformPreview {
        &self.public
    }

    pub(crate) const fn placement(&self) -> ContainedTransformPlacement {
        self.placement
    }
}

/// Exact core-resolved geometry for one contained transform update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContainedTransformPlacement {
    scene: SurfaceSceneStamp,
    surface: SurfaceId,
    surface_bounds: LogicalRect,
    rect: LogicalRect,
}

impl ContainedTransformPlacement {
    pub(crate) const fn new(
        scene: SurfaceSceneStamp,
        surface: SurfaceId,
        surface_bounds: LogicalRect,
        rect: LogicalRect,
    ) -> Self {
        Self {
            scene,
            surface,
            surface_bounds,
            rect,
        }
    }

    pub(crate) const fn scene(self) -> SurfaceSceneStamp {
        self.scene
    }

    pub(crate) const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub(crate) const fn rect(self) -> LogicalRect {
        self.rect
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveContainedTransform {
    pub(crate) session: ContainedTransformSessionId,
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) surface: SurfaceId,
    pub(crate) root: RootId,
    pub(crate) floating: FloatingPresentationId,
    pub(crate) source_rect: LogicalRect,
    pub(crate) initial_pointer: LogicalPoint,
    pub(crate) current_pointer: LogicalPoint,
    pub(crate) kind: ContainedTransformKind,
    pub(crate) minimum_size: LogicalSize,
    pub(crate) scene: SurfaceSceneStamp,
    pub(crate) surface_bounds: LogicalRect,
    pub(crate) coordinate_capture: SurfaceCoordinateCapture,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) continuation: Option<SceneGestureContinuation>,
    pub(crate) preview: Option<PublishedContainedTransformPreview>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContainedTransformStart {
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) surface: SurfaceId,
    pub(crate) root: RootId,
    pub(crate) floating: FloatingPresentationId,
    pub(crate) source_rect: LogicalRect,
    pub(crate) initial_pointer: LogicalPoint,
    pub(crate) kind: ContainedTransformKind,
    pub(crate) minimum_size: LogicalSize,
    pub(crate) scene: SurfaceSceneStamp,
    pub(crate) surface_bounds: LogicalRect,
    pub(crate) coordinate_capture: SurfaceCoordinateCapture,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) continuation: Option<SceneGestureContinuationDraft>,
}

impl InteractionState {
    pub(crate) fn begin_contained_transform(
        &mut self,
        epoch: WorkspaceEpoch,
        start: ContainedTransformStart,
    ) -> Result<(ContainedTransformSessionId, Option<InteractionStatus>), InteractionCounterError>
    {
        let generation = self
            .last_contained_transform_generation
            .checked_next()
            .ok_or(InteractionCounterError::ContainedTransformGenerationExhausted)?;
        self.last_contained_transform_generation = generation;
        let session = ContainedTransformSessionId::new(epoch, generation);
        let continuation = start
            .continuation
            .map(|draft| draft.bind(SceneGestureSession::ContainedTransform(session)));
        let replaced = (self.status() != InteractionStatus::Idle).then_some(self.status());
        self.active = ActiveGesture::ContainedTransforming(Box::new(ActiveContainedTransform {
            session,
            owner: start.owner,
            journal_capture_authority: start.journal_capture_authority,
            button: start.button,
            surface: start.surface,
            root: start.root,
            floating: start.floating,
            source_rect: start.source_rect,
            initial_pointer: start.initial_pointer,
            current_pointer: start.initial_pointer,
            kind: start.kind,
            minimum_size: start.minimum_size,
            scene: start.scene,
            surface_bounds: start.surface_bounds,
            coordinate_capture: start.coordinate_capture,
            presentation: start.presentation,
            continuation,
            preview: None,
        }));
        Ok((session, replaced))
    }

    pub(crate) fn active_contained_transform(
        &self,
        session: ContainedTransformSessionId,
    ) -> Result<&ActiveContainedTransform, InteractionRejection> {
        match &self.active {
            ActiveGesture::ContainedTransforming(transform) if transform.session == session => {
                Ok(transform)
            }
            _ => Err(self.contained_transform_session_rejection(session)),
        }
    }

    pub(crate) fn active_contained_transform_mut(
        &mut self,
        session: ContainedTransformSessionId,
    ) -> Result<&mut ActiveContainedTransform, InteractionRejection> {
        let rejection = self.contained_transform_session_rejection(session);
        match &mut self.active {
            ActiveGesture::ContainedTransforming(transform) if transform.session == session => {
                Ok(transform)
            }
            _ => Err(rejection),
        }
    }

    pub(crate) fn set_contained_transform_pointer(
        &mut self,
        session: ContainedTransformSessionId,
        current_pointer: LogicalPoint,
    ) -> Result<(), InteractionRejection> {
        self.active_contained_transform_mut(session)?
            .current_pointer = current_pointer;
        Ok(())
    }

    pub(crate) fn publish_contained_transform_preview(
        &mut self,
        session: ContainedTransformSessionId,
        placement: ContainedTransformPlacement,
    ) -> Result<(ContainedTransformPreview, bool), InteractionCounterError> {
        if let Ok(transform) = self.active_contained_transform(session)
            && let Some(existing) = &transform.preview
            && existing.public.token.scene == placement.scene()
            && existing.public.rect == placement.rect()
            && existing.placement == placement
        {
            return Ok((existing.public, false));
        }
        let sequence = self
            .last_preview_sequence
            .checked_next()
            .ok_or(InteractionCounterError::PreviewSequenceExhausted)?;
        let transform = self
            .active_contained_transform(session)
            .map_err(|_| InteractionCounterError::StateInvariant)?;
        let preview = ContainedTransformPreview {
            token: ContainedTransformPreviewToken {
                session,
                scene: placement.scene(),
                sequence,
            },
            surface: placement.surface(),
            root: transform.root,
            floating: transform.floating,
            rect: placement.rect(),
        };
        self.last_preview_sequence = sequence;
        let transform = self
            .active_contained_transform_mut(session)
            .map_err(|_| InteractionCounterError::StateInvariant)?;
        transform.preview = Some(PublishedContainedTransformPreview {
            public: preview,
            placement,
            painted: false,
        });
        Ok((preview, true))
    }

    pub(crate) fn observe_presented_contained_transform_preview(
        &mut self,
        surface: SurfaceId,
        token: ContainedTransformPreviewToken,
    ) -> bool {
        let ActiveGesture::ContainedTransforming(transform) = &mut self.active else {
            return false;
        };
        let Some(preview) = transform.preview.as_mut() else {
            return false;
        };
        if preview.public.token() != token || preview.public.surface() != surface {
            return false;
        }
        let changed = !preview.painted;
        preview.painted = true;
        changed
    }

    pub(crate) fn take_contained_transform_for_release(
        &mut self,
        session: ContainedTransformSessionId,
        owner: GestureOwner,
        button: PointerButton,
    ) -> Result<ActiveContainedTransform, InteractionRejection> {
        let transform = self.active_contained_transform(session)?;
        if transform.owner != owner {
            return Err(InteractionRejection::owner_mismatch(transform.owner, owner));
        }
        if transform.button != button {
            return Err(InteractionRejection::ButtonMismatch);
        }
        let ActiveGesture::ContainedTransforming(transform) =
            std::mem::replace(&mut self.active, ActiveGesture::Idle)
        else {
            return Err(InteractionRejection::NoActiveGesture);
        };
        self.last_consumed_contained_transform = Some(session);
        Ok(*transform)
    }

    pub(crate) fn cancel_contained_transform(
        &mut self,
        session: ContainedTransformSessionId,
    ) -> Result<InteractionStatus, InteractionRejection> {
        self.active_contained_transform(session)?;
        let status = self.status();
        self.active = ActiveGesture::Idle;
        Ok(status)
    }

    fn contained_transform_session_rejection(
        &self,
        session: ContainedTransformSessionId,
    ) -> InteractionRejection {
        if self.last_consumed_contained_transform == Some(session) {
            InteractionRejection::ContainedTransformSessionConsumed { session }
        } else if self.status() == InteractionStatus::Idle {
            InteractionRejection::NoActiveGesture
        } else {
            InteractionRejection::SessionMismatch
        }
    }
}
