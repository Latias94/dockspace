//! Frozen splitter tracks and resize gesture transitions.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveResize {
    pub(crate) session: ResizeSessionId,
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) surface: SurfaceId,
    pub(crate) scene: SurfaceSceneStamp,
    pub(crate) target: SplitterResizeTarget,
    pub(crate) coordinate_capture: SurfaceCoordinateCapture,
    pub(crate) authority: FrozenPresentationAuthority,
    pub(crate) initial_pointer: LogicalPoint,
    pub(crate) current_pointer: LogicalPoint,
    pub(crate) axis_groups: Vec<FrozenResizeAxisGroup>,
    pub(crate) updates: Vec<TrackResize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenResizeSplit {
    pub(crate) source: NodeSource,
    pub(crate) axis: Axis,
    pub(crate) has_driver: bool,
    pub(crate) bounds: LogicalRect,
    pub(crate) constraints: Constraints,
    pub(crate) weights: Vec<SplitWeight>,
    pub(crate) child_extents: Vec<f64>,
    pub(crate) child_visible: Vec<bool>,
    pub(crate) child_minimum_extents: Vec<f64>,
    pub(crate) child_maximum_extents: Vec<f64>,
    /// Extent changes per unit displacement along the containing group's axis.
    pub(crate) child_coefficients: Vec<f64>,
    pub(crate) central_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenResizeAxisGroup {
    pub(crate) axis: Axis,
    pub(crate) overflow_policy: crate::presentation::config::PaneOverflowPolicy,
    pub(crate) splits: Vec<FrozenResizeSplit>,
    pub(crate) piecewise: Option<Arc<crate::engine::FrozenPiecewiseAxis>>,
    pub(crate) common_delta: ResizeDeltaInterval,
}

impl FrozenResizeAxisGroup {
    pub(crate) fn sources(&self) -> impl Iterator<Item = &NodeSource> {
        self.splits.iter().map(|split| &split.source).chain(
            self.piecewise
                .iter()
                .flat_map(|piecewise| piecewise.sources()),
        )
    }

    pub(crate) fn resized_sources(&self) -> impl Iterator<Item = &NodeSource> {
        self.splits
            .iter()
            .filter(|split| split.axis == self.axis)
            .map(|split| &split.source)
            .chain(
                self.piecewise
                    .iter()
                    .flat_map(|piecewise| piecewise.sources()),
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResizeDeltaInterval {
    pub(crate) minimum: f64,
    pub(crate) maximum: f64,
}

/// A single displacement or the track updates from an ordered keyboard batch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResizeAdjustment {
    Displace(f64),
    Tracks(Arc<[TrackResize]>),
}

impl ResizeAdjustment {
    pub(crate) const fn new(delta: f64) -> Self {
        Self::Displace(delta)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResizeStart {
    pub(crate) owner: GestureOwner,
    pub(crate) journal_capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) surface: SurfaceId,
    pub(crate) scene: SurfaceSceneStamp,
    pub(crate) target: SplitterResizeTarget,
    pub(crate) coordinate_capture: SurfaceCoordinateCapture,
    pub(crate) authority: FrozenPresentationAuthority,
    pub(crate) initial_pointer: LogicalPoint,
    pub(crate) axis_groups: Vec<FrozenResizeAxisGroup>,
}

impl InteractionState {
    pub(crate) fn begin_resize(
        &mut self,
        epoch: WorkspaceEpoch,
        start: ResizeStart,
    ) -> Result<(ResizeSessionId, Option<InteractionStatus>), InteractionCounterError> {
        let generation = self
            .last_resize_generation
            .checked_next()
            .ok_or(InteractionCounterError::ResizeGenerationExhausted)?;
        self.last_resize_generation = generation;
        let session = ResizeSessionId::new(epoch, generation);
        let replaced = (self.status() != InteractionStatus::Idle).then_some(self.status());
        self.active = ActiveGesture::Resizing(Box::new(ActiveResize {
            session,
            owner: start.owner,
            journal_capture_authority: start.journal_capture_authority,
            button: start.button,
            surface: start.surface,
            scene: start.scene,
            target: start.target,
            coordinate_capture: start.coordinate_capture,
            authority: start.authority,
            initial_pointer: start.initial_pointer,
            current_pointer: start.initial_pointer,
            axis_groups: start.axis_groups,
            updates: Vec::new(),
        }));
        Ok((session, replaced))
    }

    pub(crate) fn active_resize(
        &self,
        session: ResizeSessionId,
    ) -> Result<&ActiveResize, InteractionRejection> {
        match &self.active {
            ActiveGesture::Resizing(resize) if resize.session == session => Ok(resize),
            ActiveGesture::Idle => Err(InteractionRejection::NoActiveGesture),
            ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => Err(InteractionRejection::SessionMismatch),
        }
    }

    pub(crate) fn set_resize_updates(
        &mut self,
        session: ResizeSessionId,
        current_pointer: LogicalPoint,
        updates: Vec<TrackResize>,
    ) -> Result<(), InteractionRejection> {
        match &mut self.active {
            ActiveGesture::Resizing(resize) if resize.session == session => {
                resize.current_pointer = current_pointer;
                resize.updates = updates;
                Ok(())
            }
            ActiveGesture::Idle => Err(InteractionRejection::NoActiveGesture),
            ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => Err(InteractionRejection::SessionMismatch),
        }
    }

    pub(crate) fn take_resize_for_release(
        &mut self,
        session: ResizeSessionId,
        owner: GestureOwner,
        button: PointerButton,
    ) -> Result<ActiveResize, InteractionRejection> {
        let resize = self.active_resize(session)?;
        if resize.owner != owner {
            return Err(InteractionRejection::owner_mismatch(resize.owner, owner));
        }
        if resize.button != button {
            return Err(InteractionRejection::ButtonMismatch);
        }
        let ActiveGesture::Resizing(resize) =
            std::mem::replace(&mut self.active, ActiveGesture::Idle)
        else {
            return Err(InteractionRejection::NoActiveGesture);
        };
        Ok(*resize)
    }

    pub(crate) fn cancel_resize(
        &mut self,
        session: ResizeSessionId,
    ) -> Result<InteractionStatus, InteractionRejection> {
        self.active_resize(session)?;
        let status = self.status();
        self.active = ActiveGesture::Idle;
        Ok(status)
    }
}
