//! Interaction outcomes, cancellation reasons and causal events.

use super::*;

/// Why an active gesture was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InteractionCancelReason {
    /// The host cancelled live input after exhausting its ordinary input capacity.
    InputOverload,
    /// The host or application explicitly cancelled the gesture.
    ExplicitCancellation,
    /// The source button was released before the renderer began dragging.
    ReleasedBeforeDrag,
    /// Pointer capture was authoritatively lost.
    CaptureLost,
    /// Pointer capture authority was unavailable at a terminal edge.
    CaptureAuthorityUnavailable,
    /// The edge-local delivery endpoint became unavailable while a gesture was active.
    DeliveryAuthorityUnavailable,
    /// An edge was delivered by an endpoint other than the gesture's frozen source.
    DeliveryOwnerLost,
    /// The edge cannot prove the coordinate lane frozen by the current gesture.
    CoordinateAuthorityUnavailable,
    /// The platform explicitly terminated the owning pointer stream.
    PointerStreamCancelled,
    /// The provider reported the normal terminal release of an ephemeral stream.
    PointerStreamEnded,
    /// Button state became non-authoritative.
    UnknownButtonState,
    /// Hovered-target authority became unavailable.
    UnknownTargetAuthority,
    /// A foreign native window authoritatively blocked the pointer route.
    OpaquePointerBlocker,
    /// The source window could not enter the pass-through state required by a physical route.
    PointerRoutingUnavailable,
    /// The primary release did not deliver to the exact pressed Click-lane receiver.
    ClickReceiverMismatch,
    /// Physical native-window drag capability became non-authoritative.
    NativeCapabilityUnknown,
    /// Physical native-window drag capability became authoritatively unavailable.
    NativeCapabilityUnavailable,
    /// Native placement proof no longer matches current window facts.
    NativePlacementUnavailable,
    /// The frozen source no longer exists.
    SourceVanished,
    /// A durable workspace command invalidated the session.
    WorkspaceChanged,
    /// Application docking policy changed.
    PolicyChanged,
    /// A participating surface closed.
    SurfaceClosed,
    /// The required sealed scene is unavailable.
    SceneUnavailable,
    /// The exact pointer-provider incarnation owning the gesture was retired.
    PointerProviderRetired,
    /// Another mutually exclusive gesture replaced this one.
    ReplacedByNewGesture,
    /// A successful workspace restore advanced the epoch.
    WorkspaceRestored,
}

/// Why a semantic interaction input did not advance or deliver a gesture.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionRejection {
    /// The prepared action's receiver is absent from its frozen scene.
    ActionReceiverUnavailable {
        target: crate::presentation::hit::PresentationHitRegionKind,
    },
    /// The close intent named a target absent from the exact painted scene.
    CloseSceneTargetUnavailable { target: CloseSceneTarget },
    /// The exact painted target exposes no close control under frozen policy.
    CloseControlUnavailable { target: CloseSceneTarget },
    /// Pointer activation used a button which cannot activate a close control.
    CloseActivationButtonUnsupported { button: PointerButton },
    /// Pointer activation named a different surface than the painted scene.
    CloseActivationSurfaceMismatch {
        expected: SurfaceId,
        actual: SurfaceId,
    },
    /// Pointer activation missed the exact half-open close-control rectangle.
    CloseActivationHitMismatch { target: CloseSceneTarget },
    /// A structurally higher presentation layer owns the activation point.
    CloseActivationOccluded { target: CloseSceneTarget },
    /// The scene's coordinate capture no longer matches current platform authority.
    CloseCoordinateAuthorityUnavailable { surface: SurfaceId },
    /// Current topology cannot reproduce the source represented by the close control.
    CloseSourceUnavailable(crate::graph::error::CommandError),
    /// The claimed tab-strip control is absent from the exact current plan.
    TabStripControlUnavailable { control: TabStripControlId },
    /// The exact current plan exposes this control only as a disabled blocker.
    TabStripControlDisabled { control: TabStripControlId },
    /// Pointer activation missed the exact tab-strip control rectangle.
    TabStripControlHitMismatch { control: TabStripControlId },
    /// The tab-strip control record changed after its press.
    TabStripControlRecordChanged { control: TabStripControlId },
    /// The control's structural tab strip is no longer live.
    TabStripSourceUnavailable { key: TabStripStateKey },
    /// The exact current tab bar does not admit semantic scrolling.
    TabStripScrollDisabled { key: TabStripStateKey },
    /// The requested reveal item is absent from the exact current tab bar.
    TabStripScrollItemUnavailable { key: TabStripStateKey, item: TabKey },
    /// The exact tab-bar geometry changed after the scroll proof was prepared.
    TabStripRecordChanged { key: TabStripStateKey },
    /// The claimed menu instance is no longer the sole active core-owned menu.
    TabListMenuSessionUnavailable { session: TabListMenuSessionId },
    /// The requested reveal item is absent from the exact active menu.
    TabListMenuScrollItemUnavailable {
        session: TabListMenuSessionId,
        item: TabKey,
    },
    /// The requested focus item is absent from the exact active menu.
    TabListMenuFocusItemUnavailable {
        session: TabListMenuSessionId,
        item: TabKey,
    },
    /// The exact menu geometry changed after the scroll proof was prepared.
    TabListMenuRecordChanged { session: TabListMenuSessionId },
    /// The claimed row is absent from the exact current menu record.
    TabListMenuRowUnavailable {
        session: TabListMenuSessionId,
        tab: TabSceneId,
    },
    /// Pointer activation missed the exact clipped menu-row rectangle.
    TabListMenuRowHitMismatch {
        session: TabListMenuSessionId,
        tab: TabSceneId,
    },
    /// The exact menu-row record changed after its press.
    TabListMenuRowRecordChanged {
        session: TabListMenuSessionId,
        tab: TabSceneId,
    },
    /// The popup routing revision changed after the exact press was frozen.
    TabListMenuPopupRevisionChanged {
        session: TabListMenuSessionId,
        expected: PopupRoutingRevision,
        actual: PopupRoutingRevision,
    },
    /// The exact current plan no longer exposes the claimed menu-frame blocker.
    TabListMenuBlockerUnavailable { session: TabListMenuSessionId },
    /// Pointer activation missed the exact current menu-frame bounds.
    TabListMenuBlockerHitMismatch { session: TabListMenuSessionId },
    /// The exact menu-frame record changed after its press.
    TabListMenuBlockerRecordChanged { session: TabListMenuSessionId },
    /// The exact current plan no longer exposes the claimed popup backdrop.
    TabListMenuBackdropUnavailable { session: TabListMenuSessionId },
    /// Pointer activation missed the exact current popup-backdrop bounds.
    TabListMenuBackdropHitMismatch { session: TabListMenuSessionId },
    /// The exact popup-backdrop record changed after its press.
    TabListMenuBackdropRecordChanged { session: TabListMenuSessionId },
    /// No gesture is active.
    NoActiveGesture,
    /// Cancellation was not requested by the active gesture's source surface or by
    /// an exact current native window binding.
    GestureCancelDeliveryUnavailable { delivery: GestureCancelDelivery },
    /// This release belongs to the most recently consumed drag generation.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    DuplicateRelease { session: DragSessionId },
    /// A non-release input belongs to a drag generation that already settled.
    SessionConsumed { session: DragSessionId },
    /// The input names a different live session.
    SessionMismatch,
    /// Another stream already owns the engine's mutually exclusive docking
    /// gesture state. Journal input never replaces it implicitly.
    GestureBusy { status: InteractionStatus },
    /// A presentation ownership transfer already owns the shared preview lane.
    PresentationTransitionPending,
    /// A primary press reported a known capture owner incompatible with its
    /// immutable provider endpoint or exact presented native binding.
    CaptureOwnerMismatch {
        expected: PointerCaptureOwner,
        actual: PointerCaptureOwner,
    },
    /// A primary press did not identify an authoritative delivery endpoint.
    DeliveryAuthorityUnavailable,
    /// A primary press was delivered by an endpoint other than the exact
    /// immutable provider endpoint or presented native binding.
    DeliveryOwnerMismatch {
        expected: PointerEventDeliveryOwner,
        actual: PointerEventDeliveryOwner,
    },
    /// The input names a different pointer than the source press.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    PointerMismatch,
    /// The input names a different journal stream incarnation than the source
    /// press. A reused physical pointer id is deliberately not sufficient.
    PointerStreamMismatch {
        /// Stream which owns the live gesture.
        expected: PointerStreamId,
        /// Stream supplied by the later edge.
        actual: PointerStreamId,
    },
    /// The input names a different button than the source press.
    ButtonMismatch,
    /// The drag has been armed but not explicitly begun.
    DragNotBegun,
    /// Transitional contained-origin facts do not identify the exact complete root.
    #[doc(hidden)]
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    ContainedDragOriginMismatch,
    /// The pressed point is outside the exact painted surface bounds.
    ContainedGesturePointerOutsideSurface { surface: SurfaceId },
    /// The pressed point does not belong to the claimed exact chrome hit region.
    ContainedGestureHitMismatch {
        floating: FloatingPresentationId,
        kind: ContainedGestureKind,
    },
    /// A structurally higher contained layer owns the exact pressed point.
    ContainedGestureOccluded {
        floating: FloatingPresentationId,
        occluding: FloatingPresentationId,
    },
    /// The scene's coordinate capture no longer matches current platform authority.
    ContainedGestureCoordinateAuthorityUnavailable { surface: SurfaceId },
    /// The pressed point is outside the exact painted surface bounds.
    TabGesturePointerOutsideSurface { surface: SurfaceId },
    /// The pressed point does not belong to the claimed exact tab drag region.
    TabGestureHitMismatch { source: TabGestureSource },
    /// A structurally higher contained layer owns the exact pressed point.
    TabGestureOccluded {
        source: TabGestureSource,
        occluding: FloatingPresentationId,
    },
    /// The scene's coordinate capture no longer matches current platform authority.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    TabGestureCoordinateAuthorityUnavailable { surface: SurfaceId },
    /// The painted tab no longer belongs to the claimed workspace presentation.
    TabGestureSourceUnavailable { source: TabGestureSource },
    /// A new-presentation offer was supplied for an already contained title drag.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    ContainedPresentationOfferUnexpected,
    /// A later observation attempted to replace the session's frozen contained offer.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    ContainedPresentationOfferChanged,
    /// A later observation attempted to replace the session's frozen background-root offer.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    SurfaceBackgroundRootOfferChanged,
    /// A later observation attempted to replace the session's frozen native offer.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    NativePresentationOfferChanged,
    /// A native offer's recovery plan names a different root than its native proposal.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    NativePresentationRecoveryRootMismatch,
    /// The observation's local or routed provenance is stale or internally inconsistent.
    TargetAuthorityInvalid,
    /// The matching button is authoritatively still pressed.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    ButtonStillPressed,
    /// No preview was published for the release.
    PreviewMissing,
    /// The exact published preview was not acknowledged as painted.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    PreviewNotPainted,
    /// The preview's sealed scene is no longer current.
    StaleScene,
    /// Release re-resolution selected a different semantic target or command.
    TargetChanged,
    /// The proposed workspace command was rejected during final revalidation.
    CommandRejected(crate::graph::error::CommandError),
    /// Split weights were rejected during candidate validation.
    ResizeRejected(crate::graph::error::CommandError),
    /// The pressed point lies outside the claimed painted surface.
    SplitterGesturePointerOutsideSurface { surface: SurfaceId },
    /// No splitter target owns the pressed point on the authoritative layer.
    SplitterGestureHitUnavailable { surface: SurfaceId },
    /// The scene's coordinate capture no longer matches current platform authority.
    SplitterGestureCoordinateAuthorityUnavailable { surface: SurfaceId },
    /// An update named a surface other than the session's frozen owner.
    SplitterGestureSurfaceMismatch {
        expected: SurfaceId,
        actual: SurfaceId,
    },
    /// Frozen splitter geometry could not produce a valid resize proposal.
    SplitterResizeGeometryUnavailable,
    /// A resize was released before any validated weight proposal was supplied.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    ResizeProposalMissing,
    /// This release belongs to the most recently consumed contained transform.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    DuplicateContainedTransformRelease {
        session: ContainedTransformSessionId,
    },
    /// A non-release input belongs to a contained transform that already settled.
    ContainedTransformSessionConsumed {
        session: ContainedTransformSessionId,
    },
    /// The exact contained presentation is absent from the current painted plan.
    ContainedTransformPresentationUnavailable {
        /// Surface expected to paint the contained presentation.
        surface: SurfaceId,
        /// Stable contained presentation identity.
        floating: FloatingPresentationId,
    },
    /// Absolute pointer arithmetic could not produce finite transform geometry.
    ContainedTransformGeometryUnavailable,
    /// The current scene cannot authorize the contained transform placement.
    ContainedPlacementUnavailable(crate::input::intent::ContainedPlacementUnavailable),
    /// Release re-resolution no longer matches the exact painted transform proof.
    ContainedTransformChanged,
}

impl InteractionRejection {
    pub(super) fn owner_mismatch(expected: GestureOwner, actual: GestureOwner) -> Self {
        Self::PointerStreamMismatch {
            expected: expected.stream(),
            actual: actual.stream(),
        }
    }
}

/// Public semantic category of a terminal manipulation rejection.
///
/// Categories deliberately omit scene proofs, graph nodes, and error details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ManipulationRejection {
    /// The required presentation is unavailable or no longer current.
    PresentationUnavailable,
    /// The source content or presentation is no longer available.
    SourceUnavailable,
    /// The release cannot deliver to the requested target.
    TargetUnavailable,
    /// Input delivery, capture, or coordinate authority is unavailable.
    InputAuthorityUnavailable,
    /// Another gesture or presentation transition owns the operation.
    GestureConflict,
    /// Current application policy denies the operation.
    PolicyDenied,
    /// A checked workspace command conflicts with current state.
    Conflict,
    /// The requested control or action is unavailable.
    ActionUnavailable,
    /// The engine could not complete the checked command.
    Internal,
}

impl ManipulationRejection {
    pub(crate) fn from_interaction(reason: &InteractionRejection) -> Self {
        match reason {
            InteractionRejection::CommandRejected(error)
            | InteractionRejection::ResizeRejected(error)
            | InteractionRejection::CloseSourceUnavailable(error) => match error {
                crate::graph::error::CommandError::Policy(_) => Self::PolicyDenied,
                error if error.is_expected_rejection() => Self::Conflict,
                _ => Self::Internal,
            },
            InteractionRejection::PreviewNotPainted | InteractionRejection::StaleScene => {
                Self::PresentationUnavailable
            }
            InteractionRejection::TabGestureSourceUnavailable { .. }
            | InteractionRejection::ContainedTransformPresentationUnavailable { .. }
            | InteractionRejection::TabStripSourceUnavailable { .. } => Self::SourceUnavailable,
            InteractionRejection::TargetChanged
            | InteractionRejection::PreviewMissing
            | InteractionRejection::ContainedTransformChanged
            | InteractionRejection::ContainedPlacementUnavailable(_) => Self::TargetUnavailable,
            InteractionRejection::TargetAuthorityInvalid
            | InteractionRejection::CaptureOwnerMismatch { .. }
            | InteractionRejection::DeliveryAuthorityUnavailable
            | InteractionRejection::DeliveryOwnerMismatch { .. }
            | InteractionRejection::CloseCoordinateAuthorityUnavailable { .. }
            | InteractionRejection::ContainedGestureCoordinateAuthorityUnavailable { .. }
            | InteractionRejection::TabGestureCoordinateAuthorityUnavailable { .. }
            | InteractionRejection::SplitterGestureCoordinateAuthorityUnavailable { .. } => {
                Self::InputAuthorityUnavailable
            }
            InteractionRejection::GestureBusy { .. }
            | InteractionRejection::PresentationTransitionPending => Self::GestureConflict,
            _ => Self::ActionUnavailable,
        }
    }

    pub(crate) const fn from_product(reason: DockspaceActionRejection) -> Self {
        match reason {
            DockspaceActionRejection::PolicyDenied => Self::PolicyDenied,
            DockspaceActionRejection::Conflict => Self::Conflict,
            DockspaceActionRejection::IdentityExhausted => Self::Internal,
            DockspaceActionRejection::PresentationUnavailable { .. } => {
                Self::PresentationUnavailable
            }
            _ => Self::ActionUnavailable,
        }
    }
}

/// Public result class of resolving one drag observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewResolutionStatus {
    /// One exact dock or tear-off preview was published.
    Resolved,
    /// Authority proved there is no dock target and no eligible non-docking request.
    KnownNone,
    /// Geometry was hit, but every candidate was explicitly ineligible.
    Rejected,
    /// A required ready surface scene was unavailable.
    Unavailable,
    /// The provider could not authoritatively classify the current pointer target.
    UnknownAuthority,
    /// A foreign native window authoritatively owns the current pointer route.
    OpaqueBlocker,
    /// Native capability is temporarily non-authoritative.
    NativeCapabilityUnknown,
    /// The platform explicitly cannot support the required native interaction.
    NativeCapabilityUnavailable,
    /// The offered native placement no longer matches current route or work-area facts.
    NativePlacementUnavailable,
}

/// Kind of an exact workspace delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDeliveryKind {
    /// Move into a sealed docking target.
    Dock,
    /// Create, rehome, or move a contained floating by explicit request.
    Contained,
    /// Use the explicitly enabled contained fallback for an unavailable native request.
    ContainedFallback,
}

/// Successful delivery produced by one consumed release.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionDelivery {
    /// A checked workspace command committed or produced a valid no-op.
    Workspace {
        /// Semantic delivery mode.
        kind: WorkspaceDeliveryKind,
        /// Structured U3 command result.
        outcome: CommandOutcome,
        /// Whether durable workspace state changed.
        changed: bool,
    },
    /// A native create saga entered the effect ledger; source ownership did not move.
    NativeRequested(NativeCreateRequest),
}

/// One canonical offset mutation produced by an ordered scroll edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollApplication {
    session: Option<ScrollSessionId>,
    receiver: PresentationHitRegionId,
    phase: ScrollPhase,
    requested_delta: f64,
    applied_delta: f64,
    unapplied_delta: f64,
    offset: f64,
}

impl ScrollApplication {
    pub(crate) const fn new(
        session: Option<ScrollSessionId>,
        receiver: PresentationHitRegionId,
        phase: ScrollPhase,
        requested_delta: f64,
        applied_delta: f64,
        unapplied_delta: f64,
        offset: f64,
    ) -> Self {
        Self {
            session,
            receiver,
            phase,
            requested_delta,
            applied_delta,
            unapplied_delta,
            offset,
        }
    }

    /// Returns the core-owned smooth-session identity, or `None` for a discrete edge.
    #[must_use]
    #[cfg(test)]
    pub const fn session(self) -> Option<ScrollSessionId> {
        self.session
    }

    /// Returns the exact semantic receiver which consumed the edge.
    #[must_use]
    pub const fn receiver(self) -> PresentationHitRegionId {
        self.receiver
    }

    /// Returns the requested signed canonical offset change before clamping.
    #[must_use]
    #[cfg(test)]
    pub const fn requested_delta(self) -> f64 {
        self.requested_delta
    }

    /// Returns the signed canonical offset change applied after clamping.
    #[must_use]
    pub const fn applied_delta(self) -> f64 {
        self.applied_delta
    }

    /// Returns the resulting core-owned logical offset.
    #[must_use]
    #[cfg(test)]
    pub const fn offset(self) -> f64 {
        self.offset
    }
}

/// Why one ordered scroll edge consumed no docking semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollSuppressionReason {
    /// The adapter could not authoritatively determine the top receiver.
    ReceiverUnknown,
    /// A known framework receiver blocked docking delivery.
    FrameworkBlocked,
    /// The active workspace popup blocks scrolling on another surface.
    PopupBlocked,
    /// The framework authoritatively reported no receiver.
    NoReceiver,
    /// The docking canvas received the edge but owns no scroll capability.
    DockCanvas,
    /// A core-owned blocker was the exact top scroll receiver.
    DockBlocker(PresentationHitRegionId),
}

/// Exact terminal cause of one core-owned smooth-scroll session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTerminationReason {
    /// The host cancelled live input after exhausting its ordinary input capacity.
    InputOverload,
    /// The provider emitted the matching terminal `End` edge.
    Completed,
    /// The provider explicitly cancelled the native sequence.
    Cancelled(ScrollCancelReason),
    /// The pointer stream was explicitly cancelled.
    StreamCancelled,
    /// The pointer stream completed with its normal terminal release.
    StreamEnded,
    /// The pointer-provider incarnation was retired.
    ProviderRetired,
    /// Known receiver evidence no longer corroborated the locked owner.
    ReceiverLost,
    /// The physical delivery endpoint changed during the sequence.
    DeliveryEndpointChanged,
    /// The exact native viewport binding was retired or replaced.
    BindingRetired,
    /// The presentation host which owned delivery was retired.
    PresentationHostRetired,
    /// The logical receiver surface no longer exists.
    SurfaceRemoved,
    /// Popup routing changed while a popup receiver was locked.
    PopupRoutingChanged,
    /// Policy changed while the semantic receiver was locked.
    PolicyChanged,
    /// Presentation configuration changed while conversion geometry was locked.
    PresentationConfigChanged,
}

/// Canonical reduction result for one ordered scroll edge or lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollReductionOutcome {
    /// A smooth begin locked one exact core-owned receiver.
    Began {
        /// Core-owned session identity.
        session: ScrollSessionId,
        /// Exact semantic receiver locked for the sequence.
        receiver: PresentationHitRegionId,
    },
    /// A receiver consumed the edge and updated its core-owned offset, possibly by zero at a bound.
    Applied(ScrollApplication),
    /// The edge was consumed without changing docking state.
    Suppressed {
        /// Core-owned smooth session, when this edge belongs to one.
        session: Option<ScrollSessionId>,
        /// Provider-owned sequence token, when this is a smooth edge.
        sequence: Option<ScrollSequenceToken>,
        /// Native phase carried by the edge.
        phase: ScrollPhase,
        /// Exact fail-closed reason.
        reason: ScrollSuppressionReason,
    },
    /// One smooth session reached its sole terminal transition.
    Terminated {
        /// Core-owned session identity.
        session: ScrollSessionId,
        /// Locked semantic receiver, absent for a sequence suppressed at `Begin`.
        receiver: Option<PresentationHitRegionId>,
        /// Exact terminal cause.
        reason: ScrollTerminationReason,
    },
}

/// Result of reducing one renderer intent.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionOutcome {
    /// A local touch completed its finite long-press qualification.
    TouchArmed {
        pointer: PointerId,
        surface: SurfaceId,
    },
    /// A local touch activated chrome on early release.
    TouchActivated {
        pointer: PointerId,
        surface: SurfaceId,
    },
    /// Pre-arm motion yielded without changing selection or layout.
    TouchYieldedToContent {
        pointer: PointerId,
        surface: SurfaceId,
    },
    /// A stationary armed release requests the host's semantic context menu.
    ContextMenuRequested {
        pointer: PointerId,
        surface: SurfaceId,
        root: RootId,
        tab: Option<TabKey>,
        position: LogicalPoint,
    },
    /// One pointer-journal scroll reduction or lifecycle terminal transition.
    Scroll(ScrollReductionOutcome),
    /// One exact close plan was opened without mutating workspace topology.
    CloseRequested {
        plan: ClosePlan,
        /// True when repeated activation reused the same unresolved request.
        reused: bool,
    },
    /// One scene-bound presentation action committed or produced a valid no-op.
    ProductActionApplied(DockspaceActionOutcome),
    /// One scene-bound presentation action was rejected without mutation.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    ProductActionRejected(DockspaceActionRejection),
    /// A core-owned tab-strip control settled on matching release.
    TabStripControlActivated {
        /// Exact control whose press and release matched.
        control: TabStripControlId,
        /// Whether core-owned transient presentation state changed.
        changed: bool,
        /// The resulting sole active menu, when this was a menu toggle/open.
        menu: Option<TabListMenuSessionId>,
    },
    /// One exact menu row selected its current item and closed the menu atomically.
    TabListMenuItemSelected {
        /// Menu instance which owned the selected row.
        session: TabListMenuSessionId,
        /// Stable row/tab identity selected by the release.
        tab: TabSceneId,
        /// Checked workspace selection result.
        outcome: CommandOutcome,
        /// Whether the durable workspace selection changed.
        changed: bool,
    },
    /// A matching backdrop release atomically closed the exact active menu.
    TabListMenuDismissed {
        /// Menu instance closed by the backdrop release.
        session: TabListMenuSessionId,
    },
    /// A matching release inside the menu frame was consumed without dismissal.
    TabListMenuFrameConsumed {
        /// Menu instance whose frame consumed the release.
        session: TabListMenuSessionId,
    },
    /// One exact tab-strip scroll adjustment updated core-owned presentation state.
    TabStripScrolled {
        /// Structural bar whose offset was adjusted.
        bar: TabBarSceneId,
        /// Resulting core-clamped logical offset.
        offset: f64,
        /// Whether the core-owned offset changed.
        changed: bool,
    },
    /// One exact active tab-list menu scroll adjustment updated core-owned state.
    TabListMenuScrolled {
        /// Exact popup session whose offset was adjusted.
        session: TabListMenuSessionId,
        /// Resulting offset for direct scrolling. Reveals resolve at layout time.
        offset: Option<f64>,
        /// Whether the core-owned scroll state changed.
        changed: bool,
    },
    /// One exact menu navigation atomically updated focus and reveal scroll.
    TabListMenuFocusMoved {
        /// Exact popup session whose focus was adjusted.
        session: TabListMenuSessionId,
        /// Resulting core-owned focus item.
        tab: TabKey,
        /// Whether focus or reveal state changed.
        changed: bool,
    },
    /// A new drag generation was armed.
    DragArmed {
        session: DragSessionId,
        replaced: Option<InteractionStatus>,
    },
    /// An armed drag explicitly began.
    DragBegan { session: DragSessionId },
    /// The current target observation produced a new preview state.
    PreviewUpdated {
        session: DragSessionId,
        preview: Option<InteractionPreview>,
        status: PreviewResolutionStatus,
    },
    /// The first authoritative matching release delivered exactly once.
    DragDelivered {
        session: DragSessionId,
        delivery: InteractionDelivery,
    },
    /// Release was consumed, but delivery is waiting for this exact preview to
    /// be observed as presented by the host.
    ReleasePending {
        session: DragSessionId,
        preview: PreviewToken,
    },
    /// An active matching gesture was cancelled.
    Cancelled {
        status: InteractionStatus,
        reason: InteractionCancelReason,
    },
    /// A resize gesture began and replaced any previous gesture.
    ResizeBegan {
        session: ResizeSessionId,
        replaced: Option<InteractionStatus>,
    },
    /// The resize proposal was prevalidated and stored for painting.
    ResizeUpdated {
        session: ResizeSessionId,
        splits: Vec<TrackResize>,
    },
    /// One scene-bound keyboard or accessibility resize committed.
    SplitterAdjusted {
        outcome: CommandOutcome,
        changed: bool,
    },
    /// The first authoritative matching resize release committed exactly once.
    ResizeDelivered {
        session: ResizeSessionId,
        outcome: CommandOutcome,
        changed: bool,
    },
    /// One scene-proof-bearing programmatic contained placement committed.
    ContainedPlacementApplied {
        /// Checked workspace command result.
        outcome: CommandOutcome,
        /// Whether durable workspace state changed.
        changed: bool,
    },
    /// A contained transform began and replaced any previous gesture.
    ContainedTransformBegan {
        session: ContainedTransformSessionId,
        replaced: Option<InteractionStatus>,
    },
    /// The core published an exact contained transform rectangle.
    ContainedTransformPreviewUpdated {
        session: ContainedTransformSessionId,
        preview: ContainedTransformPreview,
    },
    /// The first authoritative matching transform release committed exactly once.
    ContainedTransformDelivered {
        session: ContainedTransformSessionId,
        outcome: CommandOutcome,
        changed: bool,
    },
    /// A semantic input was consumed without mutating published interaction state.
    Rejected(InteractionRejection),
}

/// Committed interaction event generated only after the engine publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionEvent {
    cause: PendingReductionCause,
    version: WorkspaceVersion,
    kind: InteractionEventKind,
}

impl InteractionEvent {
    pub(crate) const fn new(
        input: InputSequence,
        version: WorkspaceVersion,
        kind: InteractionEventKind,
    ) -> Self {
        Self {
            cause: PendingReductionCause::input(input),
            version,
            kind,
        }
    }

    pub(crate) const fn new_caused(
        cause: ReductionCause,
        version: WorkspaceVersion,
        kind: InteractionEventKind,
    ) -> Self {
        Self {
            cause: PendingReductionCause::bound(cause),
            version,
            kind,
        }
    }

    pub(crate) fn bind_input(&mut self, cause: ReductionCause) -> bool {
        self.cause.bind_input(cause)
    }

    /// Returns the exact core-minted cause of this published event.
    #[must_use]
    pub fn cause(&self) -> ReductionCause {
        self.cause
            .published()
            .expect("published interaction events always have a bound reduction cause")
    }

    /// Returns the semantic event payload.
    #[must_use]
    pub const fn kind(&self) -> &InteractionEventKind {
        &self.kind
    }
}

/// Published interaction event payload.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionEventKind {
    /// A consumed drag release awaits exact final presentation.
    ReleasePending { session: DragSessionId },
    /// One consumed release or otherwise unowned press was rejected once.
    Rejected {
        /// Gesture identity retained from before the terminal edge, or `Idle` for a press.
        status: InteractionStatus,
        /// Public semantic category without private validation details.
        reason: ManipulationRejection,
    },
    /// A smooth-scroll session reached one authoritative lifecycle terminal.
    ScrollTerminated {
        /// Core-owned session identity.
        session: ScrollSessionId,
        /// Locked semantic receiver, absent for a sequence suppressed at `Begin`.
        receiver: Option<PresentationHitRegionId>,
        /// Exact lifecycle terminal cause.
        reason: ScrollTerminationReason,
    },
    /// One preview publication replaced the previous preview.
    PreviewPublished { preview: InteractionPreview },
    /// A scene authority change invalidated preview facts while preserving the source drag.
    PreviewCleared { session: DragSessionId },
    /// One gesture was explicitly cancelled or invalidated.
    Cancelled {
        status: InteractionStatus,
        reason: InteractionCancelReason,
    },
    /// One exact workspace delivery committed.
    Delivered {
        session: DragSessionId,
        kind: WorkspaceDeliveryKind,
    },
    /// One splitter resize committed.
    ResizeDelivered { session: ResizeSessionId },
    /// One contained transform preview publication replaced the previous one.
    ContainedTransformPreviewPublished { preview: ContainedTransformPreview },
    /// One contained transform committed its exact painted rectangle.
    ContainedTransformDelivered {
        session: ContainedTransformSessionId,
    },
    /// A native create saga entered the effect ledger without moving source content.
    NativePresentationRequested(NativeCreateRequest),
    /// The native create saga ended the originating drag's semantic transfer.
    NativePresentationSettled {
        session: DragSessionId,
        result: crate::engine::event::PresentationRehomeResult,
    },
}

/// Fatal exhaustion or an impossible internal interaction transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum InteractionCounterError {
    /// Click generations cannot advance without wrapping.
    #[error("click session generation is exhausted")]
    ClickGenerationExhausted,
    /// Drag generations cannot advance without wrapping.
    #[error("drag session generation is exhausted")]
    DragGenerationExhausted,
    /// Resize generations cannot advance without wrapping.
    #[error("resize session generation is exhausted")]
    ResizeGenerationExhausted,
    /// Contained transform generations cannot advance without wrapping.
    #[error("contained transform session generation is exhausted")]
    ContainedTransformGenerationExhausted,
    /// Preview identities cannot advance without wrapping.
    #[error("preview sequence is exhausted")]
    PreviewSequenceExhausted,
    /// Timed continuation identities cannot advance without wrapping.
    #[error("pointer continuation generation is exhausted")]
    ContinuationGenerationExhausted,
    /// Internal code attempted to publish a preview outside its live drag.
    #[error("interaction state invariant failed while publishing a preview")]
    StateInvariant,
}
