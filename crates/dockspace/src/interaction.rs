//! Explicit drag and resize session state.

use std::sync::Arc;

mod native_tear_off;
pub(crate) mod targeting;
mod timed;
pub use native_tear_off::NativeDragTearOff;
use targeting::{TargetAcquisition, TargetLatchDecision, TargetObservation, reconcile_target};
pub use timed::PointerContinuationToken;
pub(crate) use timed::{
    DragTimedContinuation, HoverSelectionState, PointerContinuationRequest,
    PointerContinuationSession, TabStripEdgeDirection, TabStripEdgeScrollState, TouchPressState,
};

use thiserror::Error;

use crate::close::plan::ContentCloseScope;
use crate::close::plan::{CloseItemRequirement, ClosePlan, ClosePlanTarget};
use crate::engine::event::{PendingReductionCause, ReductionCause};
use crate::engine::transition::WorkspaceVersion;
use crate::geometry::{
    Constraints, LogicalPoint, LogicalRect, LogicalSize, PhysicalPoint, PhysicalRect, ScaleFactor,
};
use crate::graph::command::{
    CommandOutcome, ContainedRosterSource, MovePayload, NodeSource, TrackResize, WorkspaceCommand,
};
use crate::graph::{Axis, SplitWeight};
use crate::ids::{
    FloatingPresentationId, InputSequence, RootId, SurfaceId, TabKey, WorkspaceEpoch,
};
use crate::input::intent::{
    Authority, CloseSceneTarget, ContainedGestureKind, ContainedPresentationOffer,
    ContainedTearOffProposal, ContainedTransformKind, NativePresentationOffer, PointerButton,
    PointerId, SurfaceBackgroundRootOffer, SurfacePointer, TabGestureSource,
};
use crate::input::journal::{
    DesktopRouteValidation, HostLocalPointerRoute, PointerCaptureOwner, PointerEventDeliveryOwner,
    PointerInputLease, PointerProviderScope, PointerStreamId, ScrollCancelReason, ScrollPhase,
    ScrollSequenceToken,
};
use crate::interaction::targeting::resolver::DropAffordance;
use crate::interaction::targeting::targets::DropTargetId;
use crate::model::{DockspaceActionOutcome, DockspaceActionRejection};
use crate::platform::frame::NativeCreateRequest;
use crate::platform::viewport::{CoordinateGeneration, ViewportBinding};
use crate::policy::PolicyRevision;
use crate::presentation::config::PresentationConfigRevision;
use crate::presentation::hit::PresentationHitRegionId;
use crate::presentation::manifest::{RequirementRevision, SurfaceMeasurementTicket};
use crate::presentation::observation::{
    PresentedSurfaceAuthority, SurfacePresentationOutputTicket,
};
use crate::presentation::scene::{
    PresentationLayoutFacts, SplitterResizeTarget, SurfaceCoordinateCapture, SurfaceSceneStamp,
    TabBarSceneId, TabListMenuBackdropRecord, TabListMenuRecord, TabListMenuRowRecord, TabSceneId,
    TabStripControlRecord,
};
use crate::presentation::tab_strip::{
    PopupRoutingRevision, TabListMenuSessionId, TabStripControlId, TabStripStateKey,
};

mod contained;
mod drag;
mod outcome;
mod preview;
mod resize;
mod session;
#[cfg(test)]
mod tests;

pub(crate) use contained::{
    ActiveContainedTransform, ContainedTransformPlacement, ContainedTransformStart,
    PublishedContainedTransformPreview,
};
pub use contained::{ContainedTransformPreview, ContainedTransformPreviewToken};
pub(crate) use drag::{
    ActiveDrag, ArmedDrag, ContainedDragIntent, DragArmStart, FrozenContainedDragOrigin,
    FrozenDragOrigin, JournalDragSourceGeometry, JournalDragThresholdOrigin,
};
pub use outcome::{
    InteractionCancelReason, InteractionCounterError, InteractionDelivery, InteractionEvent,
    InteractionEventKind, InteractionOutcome, InteractionRejection, ManipulationRejection,
    PreviewResolutionStatus, ScrollApplication, ScrollReductionOutcome, ScrollSuppressionReason,
    ScrollTerminationReason, WorkspaceDeliveryKind,
};
#[cfg(test)]
use preview::DragPreviewState;
pub(crate) use preview::{
    DragTargetState, ManipulationPositionKey, PreviewProof, PublishedPreview,
};
pub use preview::{InteractionPreview, PreviewToken, PreviewVisual};
use preview::{MagneticSourceGate, preview_acquisition_matches};
pub(crate) use resize::{
    ActiveResize, FrozenResizeAxisGroup, FrozenResizeSplit, ResizeAdjustment, ResizeDeltaInterval,
    ResizeStart,
};
pub(crate) use session::{ClickGeneration, GestureLane, GestureOwner};
pub use session::{
    ClickSessionId, ContainedTransformGeneration, ContainedTransformSessionId, DragGeneration,
    DragSessionId, PreviewSequence, ResizeGeneration, ResizeSessionId, ScrollSessionId,
};

/// Stable public summary of the currently active gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionStatus {
    /// No gesture is active.
    Idle,
    /// A Click-lane receiver was pressed and awaits its matching release.
    Pressed { session: ClickSessionId },
    /// A source was pressed but the renderer has not begun dragging.
    Armed { session: DragSessionId },
    /// An explicit drag is active.
    Dragging { session: DragSessionId },
    /// A splitter resize is active.
    Resizing { session: ResizeSessionId },
    /// A contained floating is being moved or resized.
    ContainedTransforming {
        session: ContainedTransformSessionId,
    },
}

/// Authoritative endpoint which requested explicit gesture cancellation.
///
/// Surface-local immediate-mode adapters may only cancel a gesture from its
/// frozen source surface. Native event loops instead name the exact current
/// window binding which requested cancellation, allowing a target window to
/// cancel a cross-window docking gesture without guessing its source callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureCancelDelivery {
    /// Cancellation was requested by one logical surface callback.
    Surface(SurfaceId),
    /// Cancellation was requested by one exact native window incarnation.
    NativeBinding(ViewportBinding),
}

/// Core-private close action frozen by an authoritative Click-lane press.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenCloseClick {
    pub(crate) target: ClosePlanTarget,
    pub(crate) requirements: Vec<CloseItemRequirement>,
    pub(crate) prepared: ContentCloseScope,
}

/// Exact tab-strip control action frozen by an authoritative Click-lane press.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrozenTabStripControlClick {
    pub(crate) key: TabStripStateKey,
    pub(crate) record: TabStripControlRecord,
    pub(crate) output: Option<SurfacePresentationOutputTicket>,
}

/// Exact menu-row action frozen by an authoritative Click-lane press.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrozenTabListMenuRowClick {
    pub(crate) session: TabListMenuSessionId,
    pub(crate) record: TabListMenuRowRecord,
    pub(crate) revision: PopupRoutingRevision,
}

/// Exact menu-frame blocker action frozen by an authoritative Click-lane press.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenTabListMenuBlockerClick {
    pub(crate) session: TabListMenuSessionId,
    pub(crate) record: TabListMenuRecord,
    pub(crate) revision: PopupRoutingRevision,
}

/// Exact popup-backdrop dismissal frozen by an authoritative Click-lane press.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrozenTabListMenuBackdropClick {
    pub(crate) session: TabListMenuSessionId,
    pub(crate) record: TabListMenuBackdropRecord,
    pub(crate) revision: PopupRoutingRevision,
}

/// Immutable semantic action owned by one press/release click session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FrozenClickAction {
    Touch,
    Close(FrozenCloseClick),
    TabStripControl(FrozenTabStripControlClick),
    TabListMenuRow(FrozenTabListMenuRowClick),
    TabListMenuBlocker(FrozenTabListMenuBlockerClick),
    TabListMenuBackdrop(FrozenTabListMenuBackdropClick),
}

/// Exact global presentation epoch and surface proof which authorized a gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrozenPresentationAuthority {
    pub(crate) presented: PresentedSurfaceAuthority,
    pub(crate) popup_revision: PopupRoutingRevision,
}

impl FrozenPresentationAuthority {
    pub(crate) const fn new(
        presented: PresentedSurfaceAuthority,
        popup_revision: PopupRoutingRevision,
    ) -> Self {
        Self {
            presented,
            popup_revision,
        }
    }

    pub(crate) const fn surface(self) -> SurfaceId {
        self.presented.surface()
    }
}

/// Exact source facts retained across a presentation invalidation caused by
/// activating the same gesture.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SceneGestureContinuationSource {
    Drag {
        payload: MovePayload,
        source_surface: SurfaceId,
        complete_root: Option<NodeSource>,
        origin: FrozenDragOrigin,
        coordinates: SurfaceCoordinateCapture,
    },
    ContainedTransform {
        source: NodeSource,
        surface: SurfaceId,
        root: RootId,
        floating: FloatingPresentationId,
        source_rect: LogicalRect,
        expected_roster: ContainedRosterSource,
        coordinates: SurfaceCoordinateCapture,
    },
}

/// Core-owned continuation facts captured after an activation mutation and
/// bound to the newly minted gesture session by [`InteractionState`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SceneGestureContinuationDraft {
    pub(crate) activation: ReductionCause,
    pub(crate) owner: GestureOwner,
    pub(crate) origin: FrozenPresentationAuthority,
    pub(crate) workspace: WorkspaceVersion,
    pub(crate) policy: PolicyRevision,
    pub(crate) config: PresentationConfigRevision,
    pub(crate) requirements: RequirementRevision,
    pub(crate) popup_routing: PopupRoutingRevision,
    pub(crate) source: SceneGestureContinuationSource,
}

/// Exact active session to which one continuation belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneGestureSession {
    Drag(DragSessionId),
    ContainedTransform(ContainedTransformSessionId),
}

/// Unforgeable core-owned capability permitting only the source gesture to
/// survive a transient global presentation-gate loss while its own semantic
/// source and coordinate facts remain current.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SceneGestureContinuation {
    pub(crate) session: SceneGestureSession,
    pub(crate) draft: SceneGestureContinuationDraft,
}

impl SceneGestureContinuationDraft {
    pub(crate) fn bind(self, session: SceneGestureSession) -> SceneGestureContinuation {
        SceneGestureContinuation {
            session,
            draft: self,
        }
    }
}

/// Exact facts required to arm one journal-owned click.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClickStart {
    pub(crate) epoch: WorkspaceEpoch,
    pub(crate) owner: GestureOwner,
    pub(crate) capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) region: PresentationHitRegionId,
    pub(crate) measurement: SurfaceMeasurementTicket,
    pub(crate) coordinates: SurfaceCoordinateCapture,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) action: Arc<FrozenClickAction>,
}

/// Core-private active press/release click state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActiveClick {
    pub(crate) session: ClickSessionId,
    pub(crate) owner: GestureOwner,
    pub(crate) capture_authority: Authority<PointerCaptureOwner>,
    pub(crate) button: PointerButton,
    pub(crate) region: PresentationHitRegionId,
    pub(crate) measurement: SurfaceMeasurementTicket,
    pub(crate) coordinates: SurfaceCoordinateCapture,
    pub(crate) presentation: FrozenPresentationAuthority,
    pub(crate) action: Arc<FrozenClickAction>,
    pub(crate) touch: Option<TouchPressState>,
}

/// Read-only view of one pressed Click-lane receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveClickView {
    session: ClickSessionId,
    region: PresentationHitRegionId,
}

impl ActiveClickView {
    /// Returns the logical surface owning the pressed receiver.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.region.surface()
    }
}

/// Phase of the renderer-visible drag state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DragPhase {
    /// The source button is down, but the renderer has not crossed its drag threshold.
    Armed,
    /// The renderer explicitly began the drag.
    Dragging,
}

/// Read-only view of the exact drag currently owned by the state machine.
///
/// The view borrows its payload and has no public constructor, so adapters can
/// observe an active drag without forging or mutating core interaction state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveDragView<'state> {
    phase: DragPhase,
    session: DragSessionId,
    owner: GestureOwner,
    button: PointerButton,
    payload: &'state MovePayload,
    current_pointer: Option<LogicalPoint>,
    origin: &'state FrozenDragOrigin,
    contained_offer: Option<&'state ContainedPresentationOffer>,
    surface_background_offer: Option<&'state SurfaceBackgroundRootOffer>,
    native_offer: Option<&'state NativePresentationOffer>,
}

/// Read-only view of one scene-bound splitter resize session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveResizeView<'state> {
    session: ResizeSessionId,
    owner: GestureOwner,
    button: PointerButton,
    surface: SurfaceId,
    scene: SurfaceSceneStamp,
    target: SplitterResizeTarget,
    updates: &'state [TrackResize],
}

impl<'state> ActiveResizeView<'state> {
    /// Returns the exact resize generation.
    #[must_use]
    #[cfg(test)]
    pub const fn session(self) -> ResizeSessionId {
        self.session
    }

    /// Returns the frozen logical owner surface.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    pub(crate) const fn target(self) -> SplitterResizeTarget {
        self.target
    }

    /// Returns the latest validated atomic transient updates.
    #[must_use]
    pub const fn updates(self) -> &'state [TrackResize] {
        self.updates
    }
}

impl<'state> ActiveDragView<'state> {
    /// Returns whether the drag is armed or actively dragging.
    #[must_use]
    pub const fn phase(self) -> DragPhase {
        self.phase
    }

    /// Returns the exact drag generation.
    #[must_use]
    pub const fn session(self) -> DragSessionId {
        self.session
    }

    /// Returns the exact journal stream which owns this drag.
    #[must_use]
    #[cfg(test)]
    pub const fn journal_stream(self) -> PointerStreamId {
        self.owner.stream()
    }

    /// Returns the frozen payload captured when the drag was armed.
    #[must_use]
    pub const fn payload(self) -> &'state MovePayload {
        self.payload
    }

    pub(crate) fn contained_projection_rect(self) -> Option<LogicalRect> {
        let FrozenDragOrigin::Contained(origin) = self.origin else {
            return None;
        };
        let current = self.current_pointer?;
        LogicalRect::new(
            origin.source_rect.x() + current.x() - origin.initial_pointer.x(),
            origin.source_rect.y() + current.y() - origin.initial_pointer.y(),
            origin.source_rect.width(),
            origin.source_rect.height(),
        )
        .ok()
    }

    /// Returns the first contained-presentation offer frozen for this session.
    #[must_use]
    #[cfg(test)]
    pub const fn contained_offer(self) -> Option<&'state ContainedPresentationOffer> {
        self.contained_offer
    }
}

/// Read-only view of the exact contained transform owned by the core.
///
/// Adapters can render and route the gesture from this view without keeping a
/// second copy of the frozen rectangle, pointer origin, or current preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveContainedTransformView<'state> {
    session: ContainedTransformSessionId,
    owner: GestureOwner,
    button: PointerButton,
    surface: SurfaceId,
    root: RootId,
    floating: FloatingPresentationId,
    source_rect: LogicalRect,
    initial_pointer: LogicalPoint,
    current_pointer: LogicalPoint,
    kind: ContainedTransformKind,
    minimum_size: LogicalSize,
    preview: Option<&'state ContainedTransformPreview>,
}

impl<'state> ActiveContainedTransformView<'state> {
    /// Returns the frozen host surface.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    /// Returns the exact current preview eligible for painting.
    #[must_use]
    #[cfg(test)]
    pub const fn preview(self) -> Option<&'state ContainedTransformPreview> {
        self.preview
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ActiveGesture {
    Idle,
    Pressed(Box<ActiveClick>),
    Armed(Box<ArmedDrag>),
    Dragging(Box<ActiveDrag>),
    Resizing(Box<ActiveResize>),
    ContainedTransforming(Box<ActiveContainedTransform>),
}

/// Core-owned transient interaction state.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionState {
    active: ActiveGesture,
    docking_enabled: bool,
    group_dock_mode: crate::model::TabGroupDockMode,
    last_click_generation: ClickGeneration,
    last_drag_generation: DragGeneration,
    last_resize_generation: ResizeGeneration,
    last_contained_transform_generation: ContainedTransformGeneration,
    last_preview_sequence: PreviewSequence,
    last_continuation_generation: u64,
    last_consumed_drag: Option<DragSessionId>,
    last_consumed_contained_transform: Option<ContainedTransformSessionId>,
}

impl InteractionState {
    pub(crate) const fn group_dock_mode(&self) -> crate::model::TabGroupDockMode {
        self.group_dock_mode
    }

    pub(crate) fn set_group_dock_mode(&mut self, mode: crate::model::TabGroupDockMode) -> bool {
        std::mem::replace(&mut self.group_dock_mode, mode) != mode
    }

    pub(crate) const fn docking_enabled(&self) -> bool {
        self.docking_enabled
    }

    pub(crate) fn set_docking_enabled(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.docking_enabled, enabled) != enabled
    }

    pub(crate) const fn active_surface(&self) -> Option<SurfaceId> {
        match &self.active {
            ActiveGesture::Idle => None,
            ActiveGesture::Pressed(click) => Some(click.presentation.surface()),
            ActiveGesture::Armed(drag) => Some(drag.source_surface),
            ActiveGesture::Dragging(drag) => Some(drag.source_surface),
            ActiveGesture::Resizing(resize) => Some(resize.surface),
            ActiveGesture::ContainedTransforming(transform) => Some(transform.surface),
        }
    }

    /// Returns the current public gesture state.
    #[must_use]
    pub const fn status(&self) -> InteractionStatus {
        match &self.active {
            ActiveGesture::Idle => InteractionStatus::Idle,
            ActiveGesture::Pressed(click) => InteractionStatus::Pressed {
                session: click.session,
            },
            ActiveGesture::Armed(drag) => InteractionStatus::Armed {
                session: drag.session,
            },
            ActiveGesture::Dragging(drag) => InteractionStatus::Dragging {
                session: drag.session,
            },
            ActiveGesture::Resizing(resize) => InteractionStatus::Resizing {
                session: resize.session,
            },
            ActiveGesture::ContainedTransforming(transform) => {
                InteractionStatus::ContainedTransforming {
                    session: transform.session,
                }
            }
        }
    }

    /// Returns the exact owner of the current gesture, if any.
    pub(crate) const fn active_owner(&self) -> Option<GestureOwner> {
        match &self.active {
            ActiveGesture::Idle => None,
            ActiveGesture::Pressed(click) => Some(click.owner),
            ActiveGesture::Armed(drag) => Some(drag.owner),
            ActiveGesture::Dragging(drag) => Some(drag.owner),
            ActiveGesture::Resizing(resize) => Some(resize.owner),
            ActiveGesture::ContainedTransforming(transform) => Some(transform.owner),
        }
    }

    /// Returns the current gesture's journal stream.
    pub(crate) const fn active_stream(&self) -> Option<PointerStreamId> {
        match self.active_owner() {
            Some(owner) => Some(owner.stream()),
            None => None,
        }
    }

    pub(crate) const fn active_journal_capture_authority(
        &self,
    ) -> Option<Authority<PointerCaptureOwner>> {
        match &self.active {
            ActiveGesture::Pressed(click) => Some(click.capture_authority),
            ActiveGesture::Armed(drag) => Some(drag.journal_capture_authority),
            ActiveGesture::Dragging(drag) => Some(drag.journal_capture_authority),
            ActiveGesture::Resizing(resize) => Some(resize.journal_capture_authority),
            ActiveGesture::ContainedTransforming(transform) => {
                Some(transform.journal_capture_authority)
            }
            ActiveGesture::Idle => None,
        }
    }

    pub(crate) fn update_journal_capture_authority(
        &mut self,
        stream: PointerStreamId,
        capture: Authority<PointerCaptureOwner>,
    ) -> bool {
        if self.active_stream() != Some(stream) {
            return false;
        }
        match &mut self.active {
            ActiveGesture::Pressed(click) => click.capture_authority = capture,
            ActiveGesture::Armed(drag) => drag.journal_capture_authority = capture,
            ActiveGesture::Dragging(drag) => drag.journal_capture_authority = capture,
            ActiveGesture::Resizing(resize) => resize.journal_capture_authority = capture,
            ActiveGesture::ContainedTransforming(transform) => {
                transform.journal_capture_authority = capture;
            }
            ActiveGesture::Idle => return false,
        }
        true
    }

    /// Returns the exact presentation proof frozen by the active gesture.
    pub(crate) const fn active_presentation_authority(
        &self,
    ) -> Option<FrozenPresentationAuthority> {
        match &self.active {
            ActiveGesture::Idle => None,
            ActiveGesture::Pressed(click) => Some(click.presentation),
            ActiveGesture::Armed(drag) => Some(drag.presentation),
            ActiveGesture::Dragging(drag) => Some(drag.presentation),
            ActiveGesture::Resizing(resize) => Some(resize.authority),
            ActiveGesture::ContainedTransforming(transform) => Some(transform.presentation),
        }
    }

    /// Returns the exact self-invalidation continuation held by the active
    /// source gesture, if one was minted during activation.
    pub(crate) const fn active_scene_gesture_continuation(
        &self,
    ) -> Option<&SceneGestureContinuation> {
        match &self.active {
            ActiveGesture::Armed(drag) => drag.continuation.as_ref(),
            ActiveGesture::Dragging(drag) => drag.continuation.as_ref(),
            ActiveGesture::ContainedTransforming(transform) => transform.continuation.as_ref(),
            ActiveGesture::Idle | ActiveGesture::Pressed(_) | ActiveGesture::Resizing(_) => None,
        }
    }

    /// Returns the public stable identity of the active click, if one is pressed.
    #[must_use]
    pub const fn active_click_view(&self) -> Option<ActiveClickView> {
        match &self.active {
            ActiveGesture::Pressed(click) => Some(ActiveClickView {
                session: click.session,
                region: click.region,
            }),
            ActiveGesture::Idle
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => None,
        }
    }

    pub(crate) fn begin_click(
        &mut self,
        start: ClickStart,
    ) -> Result<(ClickSessionId, Option<InteractionStatus>), InteractionCounterError> {
        if start.button != PointerButton::Primary {
            return Err(InteractionCounterError::StateInvariant);
        }
        let generation = self
            .last_click_generation
            .checked_next()
            .ok_or(InteractionCounterError::ClickGenerationExhausted)?;
        self.last_click_generation = generation;
        let session = ClickSessionId::new(start.epoch, generation);
        let replaced = (self.status() != InteractionStatus::Idle).then_some(self.status());
        self.active = ActiveGesture::Pressed(Box::new(ActiveClick {
            session,
            owner: start.owner,
            capture_authority: start.capture_authority,
            button: start.button,
            region: start.region,
            measurement: start.measurement,
            coordinates: start.coordinates,
            presentation: start.presentation,
            action: start.action,
            touch: None,
        }));
        Ok((session, replaced))
    }

    pub(crate) fn active_click(
        &self,
        session: ClickSessionId,
    ) -> Result<&ActiveClick, InteractionRejection> {
        match &self.active {
            ActiveGesture::Pressed(click) if click.session == session => Ok(click),
            ActiveGesture::Idle => Err(InteractionRejection::NoActiveGesture),
            _ => Err(InteractionRejection::SessionMismatch),
        }
    }

    pub(crate) fn active_click_mut(
        &mut self,
        session: ClickSessionId,
    ) -> Result<&mut ActiveClick, InteractionRejection> {
        match &mut self.active {
            ActiveGesture::Pressed(click) if click.session == session => Ok(click),
            ActiveGesture::Idle => Err(InteractionRejection::NoActiveGesture),
            _ => Err(InteractionRejection::SessionMismatch),
        }
    }

    pub(crate) fn take_click_for_release(
        &mut self,
        session: ClickSessionId,
        owner: GestureOwner,
        button: PointerButton,
    ) -> Result<ActiveClick, InteractionRejection> {
        let click = self.active_click(session)?;
        if click.owner != owner {
            return Err(InteractionRejection::owner_mismatch(click.owner, owner));
        }
        if click.button != button {
            return Err(InteractionRejection::ButtonMismatch);
        }
        let ActiveGesture::Pressed(click) =
            std::mem::replace(&mut self.active, ActiveGesture::Idle)
        else {
            return Err(InteractionRejection::NoActiveGesture);
        };
        Ok(*click)
    }

    /// Returns a read-only view of the current armed or active drag.
    #[must_use]
    pub const fn active_drag_view(&self) -> Option<ActiveDragView<'_>> {
        match &self.active {
            ActiveGesture::Armed(drag) => Some(ActiveDragView {
                phase: DragPhase::Armed,
                session: drag.session,
                owner: drag.owner,
                button: drag.button,
                payload: &drag.payload,
                current_pointer: drag.initial_pointer,
                origin: &drag.origin,
                contained_offer: None,
                surface_background_offer: None,
                native_offer: None,
            }),
            ActiveGesture::Dragging(drag) => Some(ActiveDragView {
                phase: DragPhase::Dragging,
                session: drag.session,
                owner: drag.owner,
                button: drag.button,
                payload: &drag.payload,
                current_pointer: drag.current_pointer(),
                origin: &drag.origin,
                contained_offer: drag.contained_offer.as_ref(),
                surface_background_offer: drag.surface_background_offer.as_ref(),
                native_offer: drag.native_offer.as_ref(),
            }),
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => None,
        }
    }

    /// Returns a read-only view of the core-owned contained transform.
    #[must_use]
    pub fn active_contained_transform_view(&self) -> Option<ActiveContainedTransformView<'_>> {
        match &self.active {
            ActiveGesture::ContainedTransforming(transform) => Some(ActiveContainedTransformView {
                session: transform.session,
                owner: transform.owner,
                button: transform.button,
                surface: transform.surface,
                root: transform.root,
                floating: transform.floating,
                source_rect: transform.source_rect,
                initial_pointer: transform.initial_pointer,
                current_pointer: transform.current_pointer,
                kind: transform.kind,
                minimum_size: transform.minimum_size,
                preview: transform
                    .preview
                    .as_ref()
                    .map(PublishedContainedTransformPreview::public),
            }),
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::Resizing(_) => None,
        }
    }

    /// Returns a read-only view of the active splitter resize.
    #[must_use]
    pub fn active_resize_view(&self) -> Option<ActiveResizeView<'_>> {
        match &self.active {
            ActiveGesture::Resizing(resize) => Some(ActiveResizeView {
                session: resize.session,
                owner: resize.owner,
                button: resize.button,
                surface: resize.surface,
                scene: resize.scene,
                target: resize.target,
                updates: &resize.updates,
            }),
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::ContainedTransforming(_) => None,
        }
    }

    /// Returns the exact contained transform preview eligible for painting.
    #[must_use]
    pub fn contained_transform_preview(&self) -> Option<&ContainedTransformPreview> {
        match &self.active {
            ActiveGesture::ContainedTransforming(transform) => transform
                .preview
                .as_ref()
                .map(PublishedContainedTransformPreview::public),
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::Resizing(_) => None,
        }
    }

    /// Returns the exact preview currently eligible for painting.
    #[must_use]
    pub fn preview(&self) -> Option<&InteractionPreview> {
        match &self.active {
            ActiveGesture::Dragging(drag) => drag.preview().map(PublishedPreview::public),
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => None,
        }
    }

    /// Returns the complete docking-guide affordance for the active drag.
    ///
    /// Affordance is independent from [`Self::preview`]: it remains available
    /// while a guide cluster is visible but no exact deliverable button is hit.
    /// It never acts as paint acknowledgement or delivery proof.
    #[must_use]
    #[cfg(test)]
    pub fn drop_affordance(&self) -> Option<&DropAffordance> {
        match &self.active {
            ActiveGesture::Dragging(drag) => drag.drop_affordance(),
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => None,
        }
    }

    /// Returns the validated resize override eligible for transient projection.
    #[must_use]
    pub fn resize_overrides(&self) -> Option<(ResizeSessionId, &[TrackResize])> {
        match &self.active {
            ActiveGesture::Resizing(resize) if !resize.updates.is_empty() => {
                Some((resize.session, &resize.updates))
            }
            ActiveGesture::Idle
            | ActiveGesture::Pressed(_)
            | ActiveGesture::Armed(_)
            | ActiveGesture::Dragging(_)
            | ActiveGesture::Resizing(_)
            | ActiveGesture::ContainedTransforming(_) => None,
        }
    }

    pub(crate) fn cancel_click(
        &mut self,
        session: ClickSessionId,
    ) -> Result<InteractionStatus, InteractionRejection> {
        self.active_click(session)?;
        let status = self.status();
        self.active = ActiveGesture::Idle;
        Ok(status)
    }

    pub(crate) fn cancel_active(&mut self) -> Option<InteractionStatus> {
        let status = self.status();
        if status == InteractionStatus::Idle {
            None
        } else {
            self.active = ActiveGesture::Idle;
            Some(status)
        }
    }

    /// Cancels the active gesture only when it is owned by `owner`.
    ///
    /// Journal cancellation and capture-loss edges must not affect a later
    /// stream incarnation that reused the same physical pointer id.
    pub(crate) fn cancel_owner(&mut self, owner: GestureOwner) -> Option<InteractionStatus> {
        if self.active_owner() != Some(owner) {
            return None;
        }
        self.cancel_active()
    }
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            active: ActiveGesture::Idle,
            docking_enabled: true,
            group_dock_mode: crate::model::TabGroupDockMode::MergeTabs,
            last_click_generation: ClickGeneration::default(),
            last_drag_generation: DragGeneration::default(),
            last_resize_generation: ResizeGeneration::default(),
            last_contained_transform_generation: ContainedTransformGeneration::default(),
            last_preview_sequence: PreviewSequence::default(),
            last_continuation_generation: 0,
            last_consumed_drag: None,
            last_consumed_contained_transform: None,
        }
    }
}
