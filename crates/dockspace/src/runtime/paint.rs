//! Renderer-neutral measurement and primary read-only docking paint capabilities.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use crate::engine::SurfaceContributionToken;
use crate::geometry::{LogicalPoint, LogicalRect, LogicalSize, PhysicalRect};
use crate::graph::{Axis, OpenItemGeneration};
use crate::ids::{
    EngineAuthorityDomainId, FloatingPresentationId, HostPresentationAttemptId, ItemId, NodeId,
    RootId, SurfaceId, TabKey, WorkspaceEpoch, WorkspaceRevision,
};
use crate::interaction::targeting::guide::DropGuideClusterId;
use crate::interaction::targeting::resolver::DropAffordance;
use crate::interaction::targeting::targets::{DropTargetId, SceneLayerKey};
use crate::interaction::{
    ContainedTransformPreview, GestureCancelDelivery, InteractionPreview, PreviewVisual,
};
use crate::model::{DockspaceActionRejection, DockspaceAxis, ProductAction};
use crate::presentation::hit::{
    PresentationHitRegion, PresentationHitRegionId, PresentationHitRegionKind,
    PresentationPointerLane, PresentationPointerLanes,
};
use crate::presentation::observation::{
    HostInteractionPresentation, SurfacePresentationOutputTicket,
};
pub use crate::presentation::scene::TabGroupDragRegionKind;
use crate::presentation::scene::{
    ContainedLayoutOwner, ContainedRecord, ContainedResizeRecord, GridBoundaryProof,
    PresentationMenuAnchorHost, PresentationMenuAnchorRecord, PresentationPlan,
    ResolvedSurfaceProgramOperation, ResolvedSurfaceProgramRoot, ResolvedSurfaceProgramStep,
    SplitterJunctionDirection, SplitterJunctionId, SplitterJunctionRecord, SplitterRecord,
    SplitterSceneId, SurfaceScene, TabBarSceneId, TabSceneId,
};
#[cfg(test)]
use crate::presentation::scene::{SurfaceProgramRootOperation, SurfaceProgramRootPresentation};
use crate::presentation::tab_strip::{
    PopupRoutingRevision, TabListMenuSessionId, TabStripControlId,
};

use super::{
    DockspaceHostFrame, DockspaceInteractionError, DockspacePaneFocusObservation,
    DockspacePaneFocusRequest, DockspaceRuntimeError, PreparedPaneFocusObservation,
};

mod actions;
mod drag_decoration;
mod drop_feedback;
mod guides;
#[cfg(test)]
mod junction_query_tests;
mod pane_commands;
mod preview_tabs;
mod splitter_range;
mod tab_chrome;
mod tab_commands;
mod tab_input;
mod tabs;
mod tabs_group_commands;
pub use actions::ContainedResizeDirection;
pub use drag_decoration::{DockspaceDragDecoration, DockspaceDragSourceKind};
pub use drop_feedback::{
    DockspaceDropAction, DockspaceDropTargetFeedback, DockspaceDropTargetScope,
};
pub use guides::{
    DockspaceDropDirection, DockspaceDropEligibility, DockspaceGuideScope,
    DropAffordanceClusterPaintRecord, DropAffordancePaintRecord, DropAffordanceTargetPaintRecord,
    DropGuidePaintRecord, DropGuideTargetPaintRecord,
};
pub use preview_tabs::{DockspacePreviewTabBarPaintRecord, DockspacePreviewTabPaintRecord};
pub use splitter_range::SurfaceSplitterRange;
pub use tab_chrome::{
    RetainedTabListMenu, RetainedTabListOpener, TabListMenuBackdropPaintRecord,
    TabListMenuPaintRecord, TabListMenuRowId, TabListMenuRowPaintRecord, TabStripControlKind,
    TabStripControlPaintRecord,
};
pub use tab_input::RetainedTabStrip;
pub use tabs::{
    PaneContentGeometry, PanePaintRecord, TabBarPaintRecord, TabGroupDragRegionPaintRecord,
    TabPaintRecord, TabStripMemberPaintRecord, TabStripMemberVisibility, TabsPaintRecord,
};
pub use tabs_group_commands::DockspaceTabsGroupCommandKind;

mod commands;
mod program;
mod receivers;
mod records;

pub use program::{
    SurfacePaintProgramOperation, SurfacePaintProgramRoot, SurfacePaintProgramRootKind,
    SurfacePaintProgramStep, SurfacePaintProgramVisitError, SurfaceProgramExecutionReceipt,
};
pub use receivers::{DockspaceReceiverDescriptor, DockspaceReceiverRole};
pub use records::{
    ContainedPaintRecord, ContainedResizePaintRecord, DockspaceContainedTransformPreview,
    DockspaceDragPreview, DockspacePresentationCommandKind,
    DockspacePresentationCommandUnavailable, DockspacePresentationMenuAnchorKind,
    DockspacePreviewVisual, DropTargetPaintRecord, PresentationCommandPaintRecord,
    PresentationMenuAnchorPaintRecord, SplitterJunctionPaintRecord, SplitterPaintRecord,
};

/// Opaque identity for one exact semantic output supplied to a renderer.
///
/// The value proves only which core output a paint plan describes. It does not
/// prove that the renderer accepted or presented that output.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DockspaceSemanticOutput {
    pub(super) ticket: SurfacePresentationOutputTicket,
}

impl DockspaceSemanticOutput {
    /// Returns the sole logical surface named by this output.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.ticket.surface()
    }

    #[cfg(test)]
    pub(crate) const fn scene_stamp(self) -> crate::presentation::scene::SurfaceSceneStamp {
        self.ticket.scene_stamp()
    }

    const fn control_id(self, target: DockspaceVisualId) -> DockspaceControlId {
        DockspaceControlId::new(self.ticket.scene_stamp(), target)
    }
}

impl fmt::Debug for DockspaceSemanticOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceSemanticOutput")
            .field("surface", &self.surface())
            .finish_non_exhaustive()
    }
}

/// Stable renderer identity whose structural storage remains core-private.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DockspaceVisualId(VisualIdentity);

/// Host widget and accessibility identity for an interactive docking control.
///
/// It remains stable across ordinary repainting and expires on session or layout
/// replacement. Controls with a narrower scope, such as a popup or an entire
/// floating root's close button, can expire earlier. Prepare actions using the
/// current paint record and its corresponding output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DockspaceControlId(ControlScope, ControlTarget);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ControlScope {
    domain: EngineAuthorityDomainId,
    epoch: WorkspaceEpoch,
    surface: SurfaceId,
}

impl DockspaceControlId {
    pub(super) const fn new(
        scene: crate::presentation::scene::SurfaceSceneStamp,
        target: DockspaceVisualId,
    ) -> Self {
        Self(
            ControlScope::from_scene(scene),
            ControlTarget::Visual(target),
        )
    }
}

impl ControlScope {
    const fn from_scene(scene: crate::presentation::scene::SurfaceSceneStamp) -> Self {
        let requirement = scene.requirement();
        Self {
            domain: requirement.authority_domain(),
            epoch: requirement.workspace_epoch(),
            surface: scene.surface(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ControlTarget {
    Visual(DockspaceVisualId),
    TabClose(TabSceneId, Option<WorkspaceRevision>),
    TabGroupMenu(TabBarSceneId),
    ContainedTitle(FloatingPresentationId, RootId),
    ContainedClose(FloatingPresentationId, RootId, WorkspaceRevision),
    ContainedResize(FloatingPresentationId, RootId, ContainedResizeDirection),
    Splitter(SplitterControlTarget),
    Junction(Arc<[Option<SplitterControlTarget>; 4]>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SplitterControlTarget {
    Split {
        root: RootId,
        node: NodeId,
        axis: Axis,
        children: [NodeId; 2],
    },
    Indexed {
        id: SplitterSceneId,
        grid: Option<GridBoundaryProof>,
    },
}

impl SplitterControlTarget {
    fn from_record(record: &SplitterRecord) -> Self {
        let id = *record.id();
        match record.split_children() {
            Some(children) => Self::Split {
                root: id.root,
                node: id.node(),
                axis: record.axis(),
                children,
            },
            None => Self::Indexed {
                id,
                grid: record.grid().cloned(),
            },
        }
    }
}

/// Content identity preserved across docking within one session and workspace epoch.
///
/// Hosts may use this as a widget/view namespace. It carries no input authority
/// and changes on layout replacement even when the same item numbers reappear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DockspaceContentId {
    domain: crate::ids::EngineAuthorityDomainId,
    epoch: crate::ids::WorkspaceEpoch,
    item: ItemId,
}

/// Identity of an item's current open lifetime within a workspace.
///
/// Use this to associate delayed editor input with the pane that received it.
/// Moving or hiding a pane preserves its lifetime. Closing and reopening the
/// same item starts a new lifetime. This identity grants no input authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DockspaceOpenItemId {
    content: DockspaceContentId,
    generation: OpenItemGeneration,
}

impl DockspaceOpenItemId {
    fn in_workspace(
        stamp: DockspaceWorkspaceStamp,
        workspace: &crate::graph::Workspace,
        item: ItemId,
    ) -> Option<Self> {
        Some(Self {
            content: DockspaceContentId {
                domain: stamp.domain,
                epoch: stamp.version.epoch(),
                item,
            },
            generation: workspace.open_item_generation(item)?,
        })
    }
}

impl super::DockspaceSession {
    /// Returns the open lifetime of this item in the committed workspace.
    ///
    /// A host can compare this value with a retained paint plan when deciding
    /// whether an existing application widget may continue receiving input.
    #[must_use]
    pub fn open_item_identity(&self, item: ItemId) -> Option<DockspaceOpenItemId> {
        DockspaceOpenItemId::in_workspace(
            DockspaceWorkspaceStamp {
                domain: self.engine.authority_domain(),
                version: self.version(),
            },
            self.engine.workspace(),
            item,
        )
    }
}

/// Exact workspace revision within one independently constructed session.
///
/// Renderers can dismiss cached menus when this changes, including after a
/// session is rebuilt with the same item numbers and numeric workspace version.
/// This identity grants no input, action or presentation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DockspaceWorkspaceStamp {
    domain: crate::ids::EngineAuthorityDomainId,
    version: crate::model::WorkspaceVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum VisualIdentity {
    Pane(crate::presentation::scene::PaneSceneId),
    Tab(TabSceneId),
    TabBar(TabBarSceneId),
    Tabs(TabBarSceneId),
    TabGroupDragRegion {
        bar: TabBarSceneId,
        region: TabGroupDragRegionKind,
    },
    DragSource {
        root: RootId,
        node: crate::ids::NodeId,
    },
    TabStripControl(TabStripControlId),
    TabListMenu(TabListMenuSessionId),
    TabListMenuRow {
        session: TabListMenuSessionId,
        tab: TabSceneId,
    },
    TabListMenuBackdrop {
        surface: SurfaceId,
        session: TabListMenuSessionId,
        revision: PopupRoutingRevision,
    },
    PresentationMenuAnchor(RootId),
    Splitter(SplitterSceneId),
    SplitterJunction(SplitterJunctionId),
    Contained(FloatingPresentationId),
    DropGuide(DropGuideClusterId),
    DropTarget(DropTargetId),
    Receiver(PresentationHitRegionId),
}

/// Public semantic class of one opaque visual identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockspaceVisualKind {
    /// One real application content placement.
    Pane,
    /// One independent Tabs container background.
    Tabs,
    /// One visible tab.
    Tab,
    /// One tab strip.
    TabBar,
    /// One exact empty tab-strip region which can drag the complete group.
    TabGroupDragRegion,
    /// One core-owned root or subtree drag source.
    DragSource,
    /// One core-owned tab-strip scroll or menu control.
    TabStripControl,
    /// One open tab-list popup frame.
    TabListMenu,
    /// One item row within an open tab-list popup.
    TabListMenuRow,
    /// One full-surface popup backdrop.
    TabListMenuBackdrop,
    /// One root-scoped presentation command menu anchor.
    PresentationMenuAnchor,
    /// One ordinary splitter.
    Splitter,
    /// One atomic multi-splitter junction.
    SplitterJunction,
    /// One contained-floating presentation.
    Contained,
    /// One complete docking-guide cluster.
    DropGuide,
    /// One exact drop target.
    DropTarget,
    /// One semantic receiver without a standalone paint record.
    Receiver,
}

/// Core-derived decision for painting one exact visual in the current program.
///
/// A source gap suppresses the source visual while a drag projection represents
/// it elsewhere. The host preserves its semantic identity and the receivers
/// retained by this program. An immediate UI can preserve widget registration;
/// a retained or self-drawn UI can retain its own nodes and hit records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SurfacePaintDisposition {
    /// Paint the visual normally.
    Visible,
    /// Preserve interaction identity but omit the visual's pixels.
    SourceGap,
}

impl SurfacePaintDisposition {
    /// Returns whether the current program requires a source gap for this visual.
    #[must_use]
    pub const fn is_source_gap(self) -> bool {
        matches!(self, Self::SourceGap)
    }
}

impl DockspaceVisualId {
    pub(super) const fn from_pane_scene(id: crate::presentation::scene::PaneSceneId) -> Self {
        Self(VisualIdentity::Pane(id))
    }

    pub(super) const fn from_tab_scene(id: TabSceneId) -> Self {
        Self(VisualIdentity::Tab(id))
    }

    pub(super) const fn from_tab_bar_scene(id: TabBarSceneId) -> Self {
        Self(VisualIdentity::TabBar(id))
    }

    /// Returns the stable semantic class without exposing graph storage IDs.
    #[must_use]
    pub const fn kind(self) -> DockspaceVisualKind {
        match self.0 {
            VisualIdentity::Pane(_) => DockspaceVisualKind::Pane,
            VisualIdentity::Tabs(_) => DockspaceVisualKind::Tabs,
            VisualIdentity::Tab(_) => DockspaceVisualKind::Tab,
            VisualIdentity::TabBar(_) => DockspaceVisualKind::TabBar,
            VisualIdentity::TabGroupDragRegion { .. } => DockspaceVisualKind::TabGroupDragRegion,
            VisualIdentity::DragSource { .. } => DockspaceVisualKind::DragSource,
            VisualIdentity::TabStripControl(_) => DockspaceVisualKind::TabStripControl,
            VisualIdentity::TabListMenu(_) => DockspaceVisualKind::TabListMenu,
            VisualIdentity::TabListMenuRow { .. } => DockspaceVisualKind::TabListMenuRow,
            VisualIdentity::TabListMenuBackdrop { .. } => DockspaceVisualKind::TabListMenuBackdrop,
            VisualIdentity::PresentationMenuAnchor(..) => {
                DockspaceVisualKind::PresentationMenuAnchor
            }
            VisualIdentity::Splitter(_) => DockspaceVisualKind::Splitter,
            VisualIdentity::SplitterJunction(_) => DockspaceVisualKind::SplitterJunction,
            VisualIdentity::Contained(_) => DockspaceVisualKind::Contained,
            VisualIdentity::DropGuide(_) => DockspaceVisualKind::DropGuide,
            VisualIdentity::DropTarget(_) => DockspaceVisualKind::DropTarget,
            VisualIdentity::Receiver(_) => DockspaceVisualKind::Receiver,
        }
    }
}

impl fmt::Debug for DockspaceVisualId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DockspaceVisualId")
            .field(&self.kind())
            .finish()
    }
}

pub(super) fn paint_disposition(
    decoration: Option<DockspaceDragDecoration<'_>>,
    visual: DockspaceVisualId,
) -> SurfacePaintDisposition {
    if decoration.is_some_and(|decoration| decoration.omits_visual(visual)) {
        SurfacePaintDisposition::SourceGap
    } else {
        SurfacePaintDisposition::Visible
    }
}

/// Core-derived paint layer. Larger values are frontmost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DockspacePaintLayer(u64);

impl DockspacePaintLayer {
    const fn from_core(layer: SceneLayerKey) -> Self {
        Self(layer.get())
    }

    /// Returns the comparable renderer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Borrowed semantic program for one exact logical-surface output.
///
/// A host may consume its operations immediately or compile them into an owned
/// display list or retained-node update. Records borrow this frame: copy the
/// required geometry, identities, and content-slot information before committing
/// it. Content resources and renderer scheduling remain host-owned.
///
/// Preview-only acknowledgements cannot replace complete output settlement:
///
/// ```compile_fail
/// fn preview_only(plan: dockspace::runtime::SurfacePaintPlan<'_>) {
///     plan.prepare_drag_preview_painted();
/// }
/// ```
///
/// ```compile_fail
/// fn transform_only(plan: dockspace::runtime::SurfacePaintPlan<'_>) {
///     plan.prepare_contained_transform_preview_painted();
/// }
/// ```
#[derive(Clone, Copy)]
pub struct SurfacePaintPlan<'frame> {
    encoding_started: &'frame std::cell::Cell<bool>,
    encoding_pass: u64,
    pub(super) surface: SurfaceId,
    pub(super) attempt: HostPresentationAttemptId,
    pub(super) authority_domain: crate::ids::EngineAuthorityDomainId,
    pub(super) version: crate::model::WorkspaceVersion,
    pub(super) scene: crate::presentation::scene::SurfaceSceneStamp,
    pub(super) output: SurfacePresentationOutputTicket,
    presented: bool,
    pub(super) contribution: SurfaceContributionToken,
    pub(super) interaction: HostInteractionPresentation,
    pub(super) plan: &'frame PresentationPlan,
    pub(super) hit_manifest: &'frame crate::presentation::hit::PresentationHitManifest,
    pub(super) splitter_keyboard_step: f64,
    pub(super) gesture_cancel_delivery: Option<GestureCancelDelivery>,
    pub(super) drop_affordance: Option<&'frame DropAffordance>,
    pub(super) drag_preview: Option<&'frame InteractionPreview>,
    pub(super) drag_decoration: Option<DockspaceDragDecoration<'frame>>,
    pub(super) contained_transform_preview: Option<&'frame ContainedTransformPreview>,
    pub(super) pane_focus_request: Option<DockspacePaneFocusRequest>,
    pub(super) candidate: crate::engine::HostFrameView<'frame>,
    pub(super) program_valid: bool,
    pub(super) program_link: Option<&'frame crate::presentation::scene::SurfacePaintProgramLink>,
}

impl fmt::Debug for SurfacePaintPlan<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfacePaintPlan")
            .field("surface", &self.surface)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// One update's normal result before any pane content is encoded.
pub enum SurfaceWork<'frame> {
    /// Output capacity and measurements are ready; execute this paint plan.
    Ready(SurfacePaintPlan<'frame>),
    /// Painting is deferred without failing semantic frame publication.
    Deferred(SurfaceWorkDeferred),
}

/// A surface can be deferred while semantic and terminal progress still publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceWorkDeferred {
    /// Required surface or tab measurements have not yet been supplied.
    Unmeasured,
    /// Earlier renderer outputs still occupy the bounded in-flight capacity.
    OutputCapacity,
}

/// Whether this Ready surface may refresh its renderer measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMeasurementRefresh {
    /// No gesture or exact preview retains this surface's current geometry.
    Allowed,
    /// This surface owns the active press, drag, resize or contained transform.
    RetainGesture,
    /// This surface carries an exact manipulation or ownership-transfer preview.
    RetainPreview,
}

impl SurfaceMeasurementRefresh {
    /// Returns `true` when the host may submit updated measurements for this surface.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

impl<'frame> SurfaceWork<'frame> {
    /// Extracts the [`SurfacePaintPlan`] when this work item is [`Self::Ready`].
    pub fn ready(self) -> Option<SurfacePaintPlan<'frame>> {
        match self {
            Self::Ready(plan) => Some(plan),
            Self::Deferred(_) => None,
        }
    }
}

impl DockspaceHostFrame<'_> {
    /// Seals semantic input and retains only the derived state needed for actual
    /// renderer multipass retries. No pane callback is invoked by this method.
    pub fn begin_encoding_pass(&mut self) -> Result<(), DockspaceRuntimeError> {
        if self.encoding.checkpoint.is_none() {
            self.complete_pointer_input()?;
            self.encoding.checkpoint = Some(self.frame.capture_encoding_checkpoint()?);
        }
        self.encoding.started.set(true);
        Ok(())
    }

    /// Discards the previous pass's private encoding and starts its replacement.
    /// Accepted input remains in the same candidate and cannot be replayed.
    pub fn restart_encoding_pass(&mut self) -> Result<(), DockspaceRuntimeError> {
        let checkpoint = self
            .encoding
            .checkpoint
            .as_ref()
            .ok_or(DockspaceInteractionError::EncodingPassUnavailable)?;
        let next = self
            .encoding
            .pass
            .checked_add(1)
            .ok_or(DockspaceInteractionError::EncodingPassExhausted)?;
        self.frame.restart_encoding_from(checkpoint)?;
        self.encoding.painted.clear();
        self.encoding.surfaces.clear();
        self.encoding.pass = next;
        self.encoding.started.set(true);
        Ok(())
    }

    /// Admits an output before encoding its pane content. Backpressure is a
    /// normal deferred result and never prevents a zero-output publication.
    pub fn surface_work(
        &mut self,
        surface: SurfaceId,
    ) -> Result<SurfaceWork<'_>, DockspaceRuntimeError> {
        self.complete_pointer_input()?;
        if self.frame.view().scene().surface(surface).is_none() {
            return Ok(SurfaceWork::Deferred(SurfaceWorkDeferred::Unmeasured));
        }
        if !self.reserve_surface_output(surface)? {
            return Ok(SurfaceWork::Deferred(SurfaceWorkDeferred::OutputCapacity));
        }
        Ok(match self.prepared_paint_plan(surface)? {
            Some(plan) => SurfaceWork::Ready(plan),
            None => SurfaceWork::Deferred(SurfaceWorkDeferred::Unmeasured),
        })
    }

    /// Reserves room for one surface output without sealing semantic or pointer input.
    ///
    /// Call before consuming ordinary pane input in an immediate UI. `false`
    /// means the host must retain that input until earlier outputs settle. A
    /// reservation belongs to this unpublished frame, survives [`Self::suspend`],
    /// and grants no painting or presentation authority. Later measurements and
    /// [`Self::surface_work`] recheck the actual output endpoint after input.
    ///
    /// # Errors
    ///
    /// Returns an error for a surface outside this frame's roster or when
    /// bounded output storage cannot be allocated.
    pub fn reserve_surface_output(
        &mut self,
        surface: SurfaceId,
    ) -> Result<bool, DockspaceRuntimeError> {
        let Some(scene) = self.frame.view().scene().surface(surface) else {
            return Err(DockspaceInteractionError::SurfaceOutsideRoster { surface }.into());
        };
        // Reserve every member before checking readiness. A resize may leave
        // several surfaces unmeasured; each still needs its own output slot
        // before the host consumes a shared window's buffered text or IME input.
        // The endpoint is unknown until measurement. Charge its output slot
        // now, then validate stream storage when the ready endpoint is known.
        let ready = scene.ready().is_some();
        let (observations, admission) = self.frame.output_admission_facts(surface);
        let stream = ready.then_some(admission);
        if self.encoding.surfaces.get(&surface) != Some(&stream) {
            let reserved = self.encoding.surfaces.len()
                - usize::from(self.encoding.surfaces.contains_key(&surface));
            if !self.session.presentation.reserve_output(
                &self.submitted_presentation,
                observations,
                reserved,
                stream,
                self.encoding
                    .surfaces
                    .iter()
                    .filter_map(|(&other, admission)| {
                        (other != surface).then_some(*admission).flatten()
                    }),
            )? {
                return Ok(false);
            }
            self.encoding.surfaces.insert(surface, stream);
        }
        Ok(true)
    }

    /// Returns the admitted paint plan, if its scene and output capacity are available.
    pub fn paint_plan(
        &mut self,
        surface: SurfaceId,
    ) -> Result<Option<SurfacePaintPlan<'_>>, DockspaceRuntimeError> {
        Ok(self.surface_work(surface)?.ready())
    }

    /// Freezes pointer input and returns the current candidate plan which the
    /// renderer may paint.
    ///
    /// # Errors
    ///
    /// Returns an error when the pointer-input phase cannot be completed.
    fn prepared_paint_plan(
        &mut self,
        surface: SurfaceId,
    ) -> Result<Option<SurfacePaintPlan<'_>>, DockspaceRuntimeError> {
        self.complete_pointer_input()?;
        let view = self.frame.view();
        let Some(scene) = view.scene().surface(surface).and_then(SurfaceScene::ready) else {
            return Ok(None);
        };
        let candidate = scene.candidate();
        let drop_affordance = view.presentation_drop_affordance(surface);
        let drag_decoration = view
            .interaction()
            .active_drag_view()
            .and_then(|drag| drag_decoration::resolve(view.scene(), drag));
        let version = view.version();
        let pane_focus_request = view
            .published_pane_focus_intent()
            .filter(|intent| intent.surface() == surface)
            .map(|intent| {
                DockspacePaneFocusRequest::from_intent(
                    self.session.engine.authority_domain(),
                    version.epoch(),
                    intent,
                )
            });
        let gesture_cancel_delivery =
            if view.interaction().status() == crate::interaction::InteractionStatus::Idle {
                None
            } else if let Some(record) = view.viewport().viewport(surface) {
                Some(GestureCancelDelivery::NativeBinding(record.binding()))
            } else {
                (view.interaction().active_surface() == Some(surface))
                    .then_some(GestureCancelDelivery::Surface(surface))
            };
        let plan = SurfacePaintPlan {
            encoding_started: &self.encoding.started,
            encoding_pass: self.encoding.pass,
            surface,
            attempt: self.frame.presentation_attempt(),
            authority_domain: self.session.engine.authority_domain(),
            version,
            scene: candidate.stamp(),
            output: candidate.output_ticket(),
            presented: self.presented_surface(surface).is_some_and(|presented| {
                presented.matches_semantic_output(DockspaceSemanticOutput {
                    ticket: candidate.output_ticket(),
                })
            }),
            contribution: view.begin_surface_contribution(surface)?,
            interaction: view
                .presentation_interaction(surface)
                .ok_or_else(|| DockspaceRuntimeError::surface_program_invariant(surface))?,
            plan: candidate.plan(),
            hit_manifest: candidate.hit_manifest(),
            splitter_keyboard_step: view.presentation_config().splitter_keyboard_step(),
            gesture_cancel_delivery,
            drop_affordance,
            drag_preview: view.presentation_drag_preview(surface),
            drag_decoration,
            contained_transform_preview: view.presentation_contained_transform_preview(surface),
            pane_focus_request,
            candidate: view,
            program_valid: candidate.paint_program().is_some(),
            program_link: candidate.paint_program(),
        };
        Ok(Some(plan))
    }

    /// Records complete emission of the current plan to the host's accepting sink,
    /// including every transient preview exposed by [`Self::paint_plan`].
    ///
    /// The sink may have drawn immediately or recorded owned work for later
    /// execution. This does not report GPU submission or presentation. A successful
    /// frame commit emits a [`super::PaintedSurfaceOutput`] which the output owner
    /// must later report as presented or dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when the surface was already answered or has no
    /// current paintable plan.
    pub fn confirm_surface_painted(
        &mut self,
        receipt: SurfaceProgramExecutionReceipt,
    ) -> Result<(), DockspaceRuntimeError> {
        let surface = receipt.surface;
        if self.encoding.painted.contains_key(&surface)
            || self
                .frame
                .surface_contributions()
                .any(|entry| entry.surface() == surface)
        {
            return Err(DockspaceInteractionError::SurfaceAlreadyAnswered { surface }.into());
        }
        let view = self.frame.view();
        let Some(candidate) = view
            .scene()
            .surface(surface)
            .and_then(SurfaceScene::ready)
            .map(|scene| scene.candidate())
        else {
            return Err(DockspaceInteractionError::SurfaceNotPaintable { surface }.into());
        };
        let expected_contribution = view.begin_surface_contribution(surface)?;
        let expected_interaction = view
            .presentation_interaction(surface)
            .ok_or_else(|| DockspaceRuntimeError::surface_program_invariant(surface))?;
        if receipt.encoding_pass != self.encoding.pass
            || receipt.attempt != self.frame.presentation_attempt()
            || receipt.output != candidate.output_ticket()
            || receipt.contribution != expected_contribution
            || receipt.interaction != expected_interaction
        {
            return Err(
                DockspaceInteractionError::SurfaceProgramExecutionMismatch { surface }.into(),
            );
        }
        self.encoding.painted.insert(surface, receipt);
        Ok(())
    }
}

impl<'frame> SurfacePaintPlan<'frame> {
    /// Returns the session-bound workspace revision used for this paint candidate.
    #[must_use]
    pub const fn workspace_stamp(self) -> DockspaceWorkspaceStamp {
        DockspaceWorkspaceStamp {
            domain: self.authority_domain,
            version: self.version,
        }
    }

    /// Returns a stable content namespace for this session and workspace epoch.
    ///
    /// Unlike slot identity, this remains unchanged when an item docks into a
    /// different group. It is a view key, not a receiver or action capability.
    #[must_use]
    pub const fn content_identity(self, item: ItemId) -> DockspaceContentId {
        DockspaceContentId {
            domain: self.authority_domain,
            epoch: self.version.epoch(),
            item,
        }
    }

    /// Returns the lifetime of an item still open in this paint candidate.
    ///
    /// Hidden items remain open. An absent or closed item returns `None`.
    #[must_use]
    pub fn open_item_identity(self, item: ItemId) -> Option<DockspaceOpenItemId> {
        DockspaceOpenItemId::in_workspace(self.workspace_stamp(), self.candidate.workspace(), item)
    }

    /// Returns whether this output's root has solver-backed partition geometry.
    ///
    /// During topology transitions content may move independently of splitters.
    /// Adapters omit structural controls until an output restores a partition.
    #[must_use]
    pub fn partition_available(self, root: RootId) -> bool {
        self.plan.partition_available(root)
    }

    /// Returns the frozen motion inputs for non-interactive decoration tracks.
    #[cfg(feature = "motion")]
    #[must_use]
    pub fn motion_frame(self) -> Option<crate::motion::MotionFrame> {
        self.candidate.motion_frame()
    }

    /// Returns whether this exact program includes a manipulation preview.
    ///
    /// Hosts preserve the current candidate until its release presentation
    /// obligation settles, including after the physical pointer was released.
    #[must_use]
    pub fn has_manipulation_preview(&self) -> bool {
        self.drag_preview.is_some()
            || self.contained_transform_preview.is_some()
            || self.plan.contained_projection().is_some()
    }

    /// Returns the core-owned measurement decision for this exact surface.
    ///
    /// An unrelated surface's gesture does not freeze this surface. Adapter
    /// resource retention or output backpressure can impose additional deferral.
    #[must_use]
    pub fn measurement_refresh(&self) -> SurfaceMeasurementRefresh {
        if self.has_manipulation_preview() {
            SurfaceMeasurementRefresh::RetainPreview
        } else if self.candidate.interaction().active_surface() == Some(self.surface) {
            SurfaceMeasurementRefresh::RetainGesture
        } else {
            SurfaceMeasurementRefresh::Allowed
        }
    }

    /// Returns the logical surface rendered by this paint plan.
    #[must_use]
    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// Returns the exact semantic output represented by this paint plan.
    #[must_use]
    pub const fn semantic_output(&self) -> DockspaceSemanticOutput {
        DockspaceSemanticOutput {
            ticket: self.output,
        }
    }

    /// Returns whether this semantic output has a current, accepted presentation.
    /// Use a presented receiver to qualify pointer input.
    #[must_use]
    pub const fn is_presented(&self) -> bool {
        self.presented
    }

    /// Returns the currently observed pane focus on this surface.
    ///
    /// Selection, pending focus requests, and restoration history do not establish
    /// this value. Workspace, external, unknown, and other-surface focus return
    /// `None`.
    #[must_use]
    pub const fn focused_pane(&self) -> Option<ItemId> {
        self.candidate
            .viewport_focus()
            .current_pane_focus(self.surface)
    }

    /// Returns the exact pane-focus request admitted for this surface.
    #[must_use]
    pub const fn pane_focus_request(&self) -> Option<DockspacePaneFocusRequest> {
        self.pane_focus_request
    }

    /// Prepares one exact final-pass observation for this plan's focus request.
    #[must_use]
    pub fn prepare_pane_focus_observation(
        self,
        request: DockspacePaneFocusRequest,
        observation: DockspacePaneFocusObservation,
    ) -> Option<PreparedPaneFocusObservation> {
        (self.pane_focus_request == Some(request))
            .then(|| PreparedPaneFocusObservation::new(request, observation))
    }

    /// Returns the surface's full logical bounds used to compile this plan.
    #[must_use]
    pub fn bounds(&self) -> LogicalRect {
        self.plan.bounds()
    }

    /// Returns source decoration for the current core-owned drag.
    ///
    /// Unlike a docking preview, this record requires no presentation
    /// acknowledgement. It exists only after the core enters its active
    /// dragging phase and disappears with the terminal or cancelled gesture.
    #[must_use]
    const fn drag_decoration(&self) -> Option<DockspaceDragDecoration<'frame>> {
        self.drag_decoration
    }

    #[cfg(test)]
    pub(crate) fn panes(self) -> impl ExactSizeIterator<Item = PanePaintRecord<'frame>> {
        self.plan
            .pane_records()
            .iter()
            .map(move |record| PanePaintRecord {
                record,
                output: self.semantic_output(),
            })
    }

    #[cfg(test)]
    pub(crate) fn tabs(self) -> impl ExactSizeIterator<Item = TabPaintRecord<'frame>> {
        self.plan
            .tab_records()
            .iter()
            .map(move |record| TabPaintRecord::new(&self, record))
    }

    #[cfg(test)]
    pub(crate) fn tab_bars(self) -> impl ExactSizeIterator<Item = TabBarPaintRecord<'frame>> {
        self.plan
            .tab_bar_records()
            .iter()
            .map(move |record| TabBarPaintRecord::new(&self, record))
    }

    /// Returns one exact core-selected presentation menu anchor per visible root.
    #[cfg(test)]
    pub(crate) fn presentation_menu_anchors(
        self,
    ) -> impl Iterator<Item = PresentationMenuAnchorPaintRecord<'frame>> {
        self.plan
            .presentation_menu_anchor_records()
            .iter()
            .filter_map(move |record| {
                record
                    .ready_bounds()
                    .map(|bounds| PresentationMenuAnchorPaintRecord {
                        record,
                        bounds,
                        operable: self.plan.region_is_operable(bounds, record.layer()),
                        scope: ControlScope::from_scene(self.scene),
                    })
            })
    }

    #[cfg(test)]
    pub(crate) fn tab_strip_controls(
        self,
    ) -> impl ExactSizeIterator<Item = TabStripControlPaintRecord> + 'frame {
        let menu_open_bars = self.program_menu_open_bars();
        self.plan
            .tab_strip_control_records()
            .iter()
            .copied()
            .map(move |record| {
                TabStripControlPaintRecord::new(
                    &self,
                    record,
                    menu_open_bars.contains(&record.id().bar()),
                )
            })
    }

    pub(crate) fn tab_list_menus(self) -> impl Iterator<Item = TabListMenuPaintRecord<'frame>> {
        let plan = self.plan;
        plan.tab_list_menu_records()
            .iter()
            .filter_map(move |record| {
                let backdrop = *plan
                    .tab_list_menu_backdrop_records()
                    .iter()
                    .find(|backdrop| backdrop.session() == record.session())?;
                Some(TabListMenuPaintRecord::new(&self, record, backdrop))
            })
    }

    #[cfg(test)]
    pub(crate) fn splitters(self) -> impl ExactSizeIterator<Item = SplitterPaintRecord<'frame>> {
        self.plan
            .splitter_records()
            .iter()
            .map(move |record| SplitterPaintRecord {
                record,
                output: self.semantic_output(),
            })
    }

    #[cfg(test)]
    pub(crate) fn splitter_junctions(
        self,
    ) -> impl ExactSizeIterator<Item = SplitterJunctionPaintRecord<'frame>> {
        self.plan
            .splitter_junction_records()
            .iter()
            .map(move |record| SplitterJunctionPaintRecord {
                record,
                plan: self.plan,
                output: self.semantic_output(),
            })
    }

    #[cfg(test)]
    pub(crate) fn contained(self) -> impl ExactSizeIterator<Item = ContainedPaintRecord<'frame>> {
        self.plan
            .contained_records()
            .iter()
            .map(move |record| ContainedPaintRecord::new(&self, record))
    }

    #[cfg(test)]
    pub(crate) fn drop_guides(self) -> impl ExactSizeIterator<Item = DropGuidePaintRecord<'frame>> {
        self.plan
            .drop_guide_clusters()
            .iter()
            .map(|record| DropGuidePaintRecord { record })
    }

    /// Returns the payload-specific guide set visible at the current drag point.
    ///
    /// This value is independent from [`Self::drag_preview`]: a complete cluster
    /// remains visible while the pointer is between buttons or over a rejected
    /// target. The renderer must not infer active or disabled state from geometry.
    #[must_use]
    fn drop_affordance(&self) -> Option<DropAffordancePaintRecord<'frame>> {
        self.drop_affordance
            .map(|affordance| DropAffordancePaintRecord {
                affordance,
                output: self.semantic_output(),
            })
    }

    /// Returns the frozen label of one direct tab represented by this output or preview.
    #[must_use]
    pub fn tab_label(&self, key: TabKey) -> Option<crate::graph::TabLabel<'frame>> {
        if let Some(decoration) = self
            .drag_decoration()
            .filter(|decoration| decoration.key() == key)
        {
            return Some(decoration.label());
        }
        if let Some(item) = key.item().filter(|item| {
            self.plan
                .pane_records()
                .iter()
                .any(|pane| pane.item() == *item)
        }) {
            return Some(crate::graph::TabLabel::Item(item));
        }

        if let Some(tab) = self
            .plan
            .tab_records()
            .iter()
            .find(|tab| tab.id().tab == key)
        {
            return self.plan.tab_label(*tab.id());
        }
        if let Some(bar) = self
            .plan
            .tab_bar_records()
            .iter()
            .find(|bar| bar.members().iter().any(|member| member.tab().tab == key))
        {
            return self.plan.tab_label(TabSceneId {
                root: bar.id().root,
                tabs: bar.id().tabs,
                tab: key,
            });
        }
        let preview = self.drag_preview?;
        let tab = preview
            .tab_strip()?
            .tabs
            .iter()
            .find(|tab| tab.key == key)?;
        match key.item() {
            Some(item) => Some(crate::graph::TabLabel::Item(item)),
            None => tab
                .layout_title
                .as_deref()
                .map(|title| crate::graph::TabLabel::Layout { title }),
        }
    }

    #[must_use]
    fn drag_preview(&self) -> Option<DockspaceDragPreview<'frame>> {
        let preview = self.drag_preview?;
        let target = self
            .candidate
            .presentation_dock_target_context(preview)
            .and_then(|(plan, target, payload)| {
                drop_feedback::resolve(plan, target, payload, self.candidate.workspace())
            });
        let layout_projected = self.plan.contained_projection().is_some_and(|projection| {
            projection.owner == ContainedLayoutOwner::Drag(preview.token().session())
                && projection.surface == self.surface
                && matches!(preview.visual(), PreviewVisual::Contained { surface, rect, .. }
                    if *surface == projection.surface && *rect == projection.rect)
                && self.plan.contained_records().iter().any(|record| {
                    record.root() == projection.root && record.floating() == projection.floating
                })
        });
        Some(DockspaceDragPreview {
            preview,
            layout_projected,
            target,
        })
    }

    /// Returns the exact contained resize preview which this surface must paint.
    #[must_use]
    fn contained_transform_preview(&self) -> Option<DockspaceContainedTransformPreview<'frame>> {
        let preview = self.contained_transform_preview?;
        let layout_projected = self.plan.contained_projection().is_some_and(|projection| {
            projection.owner == ContainedLayoutOwner::Transform(preview.token().session())
                && projection.surface == self.surface
                && projection.surface == preview.surface()
                && projection.root == preview.root()
                && projection.floating == preview.floating()
                && projection.rect == preview.rect()
        }) && self.plan.contained_records().iter().any(|record| {
            record.root() == preview.root() && record.floating() == preview.floating()
        });
        Some(DockspaceContainedTransformPreview {
            preview,
            layout_projected,
        })
    }

    fn tab_record(&self, item: TabKey) -> Option<&'frame crate::presentation::scene::TabRecord> {
        self.plan
            .tab_records()
            .iter()
            .find(|record| record.id().tab == item)
    }

    /// Returns tab bars whose popup menu is present in the ordered program.
    fn program_menu_open_bars(&self) -> HashSet<TabBarSceneId> {
        if self.plan.tab_list_menu_records().is_empty() {
            return HashSet::new();
        }
        self.plan.surface_program_menu_open_bars().collect()
    }
}

#[cfg(test)]
mod tests;
