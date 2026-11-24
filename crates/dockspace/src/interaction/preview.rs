//! Published drag previews and evaluation state.

use super::*;

/// Opaque identity of one exact preview publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewToken {
    pub(super) session: DragSessionId,
    pub(super) scene: SurfaceSceneStamp,
    pub(super) sequence: PreviewSequence,
}

impl PreviewToken {
    /// Returns the drag session which owns this preview.
    #[must_use]
    pub const fn session(self) -> DragSessionId {
        self.session
    }

    /// Returns the sealed scene against which this preview was resolved.
    #[must_use]
    pub const fn scene(self) -> SurfaceSceneStamp {
        self.scene
    }
}

/// Exact renderer-neutral visual which must be painted before delivery.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewVisual {
    /// Highlight one resolved docking target.
    Dock {
        /// Surface containing the target.
        surface: SurfaceId,
        /// Structural target identity.
        target: DropTargetId,
        /// Exact highlight rectangle.
        rect: LogicalRect,
    },
    /// Show an immediate contained-floating placement.
    Contained {
        /// Host logical surface.
        surface: SurfaceId,
        /// Exact logical floating rectangle.
        rect: LogicalRect,
        /// Whether this is an explicitly enabled native fallback.
        fallback: bool,
    },
    /// Show a source-hosted cue for a future native-surface placement.
    Native {
        /// Existing surface whose presented output carries this cue.
        ///
        /// The prospective native target has no presentation stream before its
        /// lifecycle starts, so it cannot acknowledge a preview. Keeping this
        /// separate from [`Self::target_surface`] makes that boundary explicit.
        host_surface: SurfaceId,
        /// Reserved logical surface which the later native lifecycle may create.
        target_surface: SurfaceId,
        /// Exact desktop-physical placement.
        placement: PhysicalRect,
    },
}

impl PreviewVisual {
    /// Returns the surface which receives the preview.
    #[must_use]
    pub const fn surface(&self) -> SurfaceId {
        match *self {
            Self::Dock { surface, .. }
            | Self::Contained { surface, .. }
            | Self::Native {
                host_surface: surface,
                ..
            } => surface,
        }
    }

    /// Returns the reserved native target when this is a native tear-off cue.
    #[must_use]
    #[cfg(test)]
    pub const fn target_surface(&self) -> Option<SurfaceId> {
        match *self {
            Self::Native { target_surface, .. } => Some(target_surface),
            Self::Dock { .. } | Self::Contained { .. } => None,
        }
    }
}

/// Preview published by the core for an adapter to paint exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionPreview {
    pub(super) token: PreviewToken,
    pub(super) visual: PreviewVisual,
    pub(super) tab_strip: Option<Arc<crate::presentation::compiler::FutureTabStrip>>,
}

impl InteractionPreview {
    pub(crate) fn tab_strip(&self) -> Option<&crate::presentation::compiler::FutureTabStrip> {
        self.tab_strip.as_deref()
    }

    pub(crate) fn shared_tab_strip(
        &self,
    ) -> Option<Arc<crate::presentation::compiler::FutureTabStrip>> {
        self.tab_strip.clone()
    }

    /// Returns the opaque preview token.
    #[must_use]
    pub const fn token(&self) -> PreviewToken {
        self.token
    }

    /// Returns the exact visual to paint.
    #[must_use]
    pub const fn visual(&self) -> &PreviewVisual {
        &self.visual
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PreviewProof {
    Dock {
        target: DropTargetId,
        command: WorkspaceCommand,
    },
    Contained {
        command: WorkspaceCommand,
        proposal: ContainedTearOffProposal,
        fallback: bool,
    },
    Native {
        command: WorkspaceCommand,
        offer: NativePresentationOffer,
    },
    PresentationRehome {
        command: WorkspaceCommand,
    },
}

pub(super) fn preview_acquisition_matches(
    scene: SurfaceSceneStamp,
    proof: &PreviewProof,
    acquisition: Option<TargetAcquisition>,
) -> bool {
    match (proof, acquisition) {
        (PreviewProof::Dock { target, .. }, Some(acquisition)) => {
            acquisition.scene() == scene && acquisition.target() == *target
        }
        (
            PreviewProof::Contained { .. }
            | PreviewProof::Native { .. }
            | PreviewProof::PresentationRehome { .. },
            None,
        ) => true,
        (PreviewProof::Dock { .. }, None)
        | (
            PreviewProof::Contained { .. }
            | PreviewProof::Native { .. }
            | PreviewProof::PresentationRehome { .. },
            Some(_),
        ) => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ManipulationPositionKey {
    JournalSurface {
        scope: PointerProviderScope,
        position: Authority<LogicalPoint>,
    },
    JournalHost {
        manipulation: Option<HostLocalPointerRoute>,
        hover: Option<HostLocalPointerRoute>,
    },
    JournalNativeLocal {
        manipulation: Option<crate::input::journal::NativeSurfacePointerRoute>,
        hover: Option<crate::input::journal::NativeSurfacePointerRoute>,
    },
    JournalDesktop(DesktopRouteValidation),
}

impl ManipulationPositionKey {
    pub(crate) const fn surface(self) -> Option<SurfaceId> {
        match self {
            Self::JournalHost { hover, .. } => match hover {
                Some(route) => Some(route.surface()),
                None => None,
            },
            Self::JournalNativeLocal { hover, .. } => match hover {
                Some(route) => Some(route.binding().surface()),
                None => None,
            },
            Self::JournalSurface {
                scope: PointerProviderScope::SurfaceLocal(scope),
                ..
            } => Some(scope.surface()),
            Self::JournalDesktop(DesktopRouteValidation::Known(
                crate::input::journal::ValidatedDesktopRoute::Dock(route),
            )) => Some(route.binding().surface()),
            Self::JournalSurface { .. } | Self::JournalDesktop(_) => None,
        }
    }

    pub(crate) const fn hover_probe(self) -> Option<SurfacePointer> {
        match self {
            Self::JournalHost { hover, .. } => match hover {
                Some(route) => Some(SurfacePointer::new(route.surface(), route.position())),
                None => None,
            },
            Self::JournalNativeLocal { hover, .. } => match hover {
                Some(route) => Some(SurfacePointer::new(
                    route.binding().surface(),
                    route.surface_position(),
                )),
                None => None,
            },
            Self::JournalSurface {
                scope: PointerProviderScope::SurfaceLocal(scope),
                position: Authority::Known(point),
            } => Some(SurfacePointer::new(scope.surface(), point)),
            Self::JournalDesktop(DesktopRouteValidation::Known(
                crate::input::journal::ValidatedDesktopRoute::Dock(route),
            )) => Some(SurfacePointer::new(
                route.binding().surface(),
                route.surface_position(),
            )),
            Self::JournalSurface { .. } | Self::JournalDesktop(_) => None,
        }
    }

    /// Returns whether this edge can ask the adapter for a fresh hover fact.
    ///
    /// A surface-local provider still has a hover question when its coordinate
    /// is unknown; the adapter must answer that question with a typed unknown
    /// result instead of making the edge look not-applicable. A validated
    /// outside-all desktop route is the only active-drag position with no
    /// surface receiver to probe.
    pub(crate) const fn hover_probe_requested(self) -> bool {
        !matches!(
            self,
            Self::JournalDesktop(DesktopRouteValidation::Known(
                crate::input::journal::ValidatedDesktopRoute::NoWindow { .. },
            ))
        )
    }

    /// Returns the surface the adapter should associate with a hover probe,
    /// even when the event-time logical point itself is unavailable.
    pub(crate) const fn hover_surface(self) -> Option<SurfaceId> {
        match self {
            Self::JournalHost { hover, .. } => match hover {
                Some(route) => Some(route.surface()),
                None => None,
            },
            Self::JournalNativeLocal { hover, .. } => match hover {
                Some(route) => Some(route.binding().surface()),
                None => None,
            },
            Self::JournalSurface {
                scope: PointerProviderScope::SurfaceLocal(scope),
                ..
            } => Some(scope.surface()),
            Self::JournalDesktop(DesktopRouteValidation::Known(
                crate::input::journal::ValidatedDesktopRoute::Dock(route),
            )) => Some(route.binding().surface()),
            Self::JournalDesktop(DesktopRouteValidation::Known(
                crate::input::journal::ValidatedDesktopRoute::NoWindow { .. }
                | crate::input::journal::ValidatedDesktopRoute::Foreign { .. },
            ))
            | Self::JournalDesktop(DesktopRouteValidation::Unavailable(_))
            | Self::JournalSurface { .. } => None,
        }
    }

    pub(crate) const fn logical_point(self) -> Option<LogicalPoint> {
        match self.hover_probe() {
            Some(pointer) => Some(pointer.position()),
            None => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PublishedPreview {
    pub(super) public: InteractionPreview,
    pub(super) proof: PreviewProof,
    pub(super) painted: bool,
    pub(super) target_layout_basis: Option<SurfaceSceneStamp>,
}

impl PublishedPreview {
    pub(crate) const fn public(&self) -> &InteractionPreview {
        &self.public
    }

    pub(crate) const fn proof(&self) -> &PreviewProof {
        &self.proof
    }

    pub(crate) const fn painted(&self) -> bool {
        self.painted
    }

    pub(crate) const fn target_layout_basis(&self) -> Option<SurfaceSceneStamp> {
        self.target_layout_basis
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum DragPreviewState {
    Unresolved {
        status: PreviewResolutionStatus,
    },
    Published {
        preview: PublishedPreview,
        acquisition: Option<TargetAcquisition>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum MagneticSourceGate {
    Open,
    ExplicitTargetsOnly,
}

impl MagneticSourceGate {
    const fn from_origin(origin: &FrozenDragOrigin) -> Self {
        match origin {
            FrozenDragOrigin::Workspace => Self::Open,
            FrozenDragOrigin::Contained(origin) => match origin.intent {
                ContainedDragIntent::MoveWindow => Self::ExplicitTargetsOnly,
                ContainedDragIntent::DockContent => Self::Open,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragTargetState {
    pub(super) last_advanced: Option<ManipulationPositionKey>,
    /// 最近一次可用于重新取证的逻辑位置；authority key 失效时仍保留它。
    pub(super) last_logical_point: Option<LogicalPoint>,
    pub(super) magnetic_source_gate: MagneticSourceGate,
    pub(super) affordance: Option<DropAffordance>,
    pub(super) preview: DragPreviewState,
}

impl DragTargetState {
    pub(super) const fn new(origin: &FrozenDragOrigin) -> Self {
        Self {
            last_advanced: None,
            last_logical_point: None,
            magnetic_source_gate: MagneticSourceGate::from_origin(origin),
            affordance: None,
            preview: DragPreviewState::Unresolved {
                status: PreviewResolutionStatus::KnownNone,
            },
        }
    }

    pub(crate) const fn affordance(&self) -> Option<&DropAffordance> {
        self.affordance.as_ref()
    }

    pub(crate) const fn last_advanced(&self) -> Option<ManipulationPositionKey> {
        self.last_advanced
    }

    pub(crate) const fn current_pointer(
        &self,
        initial_pointer: Option<LogicalPoint>,
    ) -> Option<LogicalPoint> {
        if let Some(position) = self.last_advanced
            && let Some(point) = position.logical_point()
        {
            return Some(point);
        }
        if let Some(point) = self.last_logical_point {
            return Some(point);
        }
        initial_pointer
    }

    pub(crate) const fn acquisition(&self) -> Option<TargetAcquisition> {
        match &self.preview {
            DragPreviewState::Published { acquisition, .. } => *acquisition,
            DragPreviewState::Unresolved { .. } => None,
        }
    }

    pub(crate) const fn preview(&self) -> Option<&PublishedPreview> {
        match &self.preview {
            DragPreviewState::Published { preview, .. } => Some(preview),
            DragPreviewState::Unresolved { .. } => None,
        }
    }

    pub(super) fn preview_mut(&mut self) -> Option<&mut PublishedPreview> {
        match &mut self.preview {
            DragPreviewState::Published { preview, .. } => Some(preview),
            DragPreviewState::Unresolved { .. } => None,
        }
    }

    #[cfg(test)]
    pub(super) const fn status(&self) -> PreviewResolutionStatus {
        match &self.preview {
            DragPreviewState::Published { .. } => PreviewResolutionStatus::Resolved,
            DragPreviewState::Unresolved { status, .. } => *status,
        }
    }

    pub(super) fn publish(
        &mut self,
        preview: PublishedPreview,
        acquisition: Option<TargetAcquisition>,
    ) {
        self.preview = DragPreviewState::Published {
            preview,
            acquisition,
        };
    }

    pub(super) fn clear_preview(&mut self, status: PreviewResolutionStatus) -> bool {
        let changed = matches!(self.preview, DragPreviewState::Published { .. });
        self.preview = DragPreviewState::Unresolved { status };
        changed
    }

    pub(super) fn set_acquisition(&mut self, acquisition: Option<TargetAcquisition>) {
        match &mut self.preview {
            DragPreviewState::Published {
                acquisition: current,
                ..
            } => *current = acquisition,
            DragPreviewState::Unresolved { .. } => {}
        }
    }

    pub(super) fn clear_feedback(&mut self, status: PreviewResolutionStatus) -> bool {
        self.last_advanced = None;
        self.clear_target_feedback(status)
    }

    pub(super) fn clear_target_feedback(&mut self, status: PreviewResolutionStatus) -> bool {
        let affordance_cleared = self.affordance.take().is_some();
        let preview_cleared = self.clear_preview(status);
        affordance_cleared || preview_cleared
    }

    pub(super) fn reset_coordinate_lane(&mut self) {
        // Preserve source intent while discarding coordinate-dependent feedback.
        self.clear_feedback(PreviewResolutionStatus::Unavailable);
        self.last_logical_point = None;
    }

    pub(super) fn reconcile_acquisition(
        &self,
        observation: TargetObservation,
        point: Option<LogicalPoint>,
        release_hysteresis: f64,
    ) -> TargetLatchDecision {
        reconcile_target(self.acquisition(), observation, point, release_hysteresis)
    }

    pub(super) fn set_affordance(&mut self, affordance: Option<DropAffordance>) {
        self.affordance = affordance;
    }

    pub(super) fn set_last_advanced(&mut self, position: ManipulationPositionKey) {
        if let Some(point) = position.logical_point() {
            self.last_logical_point = Some(point);
        }
        self.last_advanced = Some(position);
    }

    pub(super) fn invalidate_last_advanced(&mut self) {
        self.last_advanced = None;
    }
}

impl InteractionState {
    pub(crate) fn prepare_presentation_rehome_preview(
        &mut self,
        epoch: WorkspaceEpoch,
        scene: SurfaceSceneStamp,
        visual: PreviewVisual,
        proof: PreviewProof,
    ) -> Result<PublishedPreview, InteractionCounterError> {
        let generation = self
            .last_drag_generation
            .checked_next()
            .ok_or(InteractionCounterError::DragGenerationExhausted)?;
        let sequence = self
            .last_preview_sequence
            .checked_next()
            .ok_or(InteractionCounterError::PreviewSequenceExhausted)?;
        self.last_drag_generation = generation;
        self.last_preview_sequence = sequence;
        Ok(PublishedPreview {
            public: InteractionPreview {
                token: PreviewToken {
                    session: DragSessionId::new(epoch, generation),
                    scene,
                    sequence,
                },
                visual,
                tab_strip: None,
            },
            proof,
            painted: false,
            target_layout_basis: None,
        })
    }
}
