//! Preview, divider and contained window paint records.

use super::*;

/// Borrowed non-visual drop target in one exact surface paint program.
///
/// Validation resolves its complete receiver identity and finite center before
/// a visitor receives it. Materialize an owned descriptor only when the host
/// needs to retain or register this receiver.
#[derive(Clone, Copy)]
pub struct DropTargetPaintRecord<'frame> {
    pub(super) output: SurfacePresentationOutputTicket,
    pub(super) region: &'frame PresentationHitRegion,
    pub(super) center: LogicalPoint,
}

impl fmt::Debug for DropTargetPaintRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.receiver().fmt(formatter)
    }
}

impl DropTargetPaintRecord<'_> {
    /// Returns the complete owned descriptor for this exact output.
    #[must_use]
    pub fn receiver(self) -> DockspaceReceiverDescriptor {
        DockspaceReceiverDescriptor {
            output: self.output,
            region: self.region.id(),
            lanes: self.region.lanes(),
            role: DockspaceReceiverRole::DropTarget,
            bounds: self.region.hit().rect(),
            center: self.center,
        }
    }
}

/// Stable renderer-facing preview geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockspacePreviewVisual {
    /// Highlights the resulting docking area or tab insertion position.
    ///
    /// The destination is selected from presented geometry. This rectangle
    /// projects the candidate layout, including source removal and target
    /// resizing, so it may differ from the destination's current bounds.
    /// Tab-gap moves use an insertion line at the future tab position.
    Dock {
        surface: SurfaceId,
        rect: LogicalRect,
    },
    /// Paint one contained-floating placement.
    Contained {
        surface: SurfaceId,
        rect: LogicalRect,
        fallback: bool,
    },
    /// Paint a source-hosted cue for a future native surface.
    Native {
        host_surface: SurfaceId,
        target_surface: SurfaceId,
        placement: PhysicalRect,
    },
}

/// Borrowed preview which belongs to one exact paint plan.
#[derive(Debug, Clone, Copy)]
pub struct DockspaceDragPreview<'plan> {
    pub(super) preview: &'plan InteractionPreview,
    pub(super) layout_projected: bool,
    pub(super) target: Option<DockspaceDropTargetFeedback>,
}

impl DockspaceDragPreview<'_> {
    /// Returns whether this output already lays out the complete root at this placement.
    ///
    /// The preview remains part of the exact output obligation; adapters must
    /// not paint a duplicate shell over the projected content.
    #[must_use]
    pub const fn is_layout_projected(self) -> bool {
        self.layout_projected
    }

    /// Returns the resolved destination in its current presented layout.
    ///
    /// Selection, bounds and insertion neighbors identify the destination before
    /// source removal; [`Self::visual`] retains the future layout rectangle.
    /// Contained placement and native tear-off cues have no docking target.
    #[must_use]
    pub const fn target(self) -> Option<DockspaceDropTargetFeedback> {
        self.target
    }

    /// Returns the renderer-neutral geometry which must be painted.
    #[must_use]
    pub fn visual(self) -> DockspacePreviewVisual {
        match *self.preview.visual() {
            PreviewVisual::Dock { surface, rect, .. } => {
                DockspacePreviewVisual::Dock { surface, rect }
            }
            PreviewVisual::Contained {
                surface,
                rect,
                fallback,
            } => DockspacePreviewVisual::Contained {
                surface,
                rect,
                fallback,
            },
            PreviewVisual::Native {
                host_surface,
                target_surface,
                placement,
            } => DockspacePreviewVisual::Native {
                host_surface,
                target_surface,
                placement,
            },
        }
    }
}

/// Borrowed contained-floating transform preview owned by one exact paint plan.
#[derive(Clone, Copy)]
pub struct DockspaceContainedTransformPreview<'plan> {
    pub(super) preview: &'plan ContainedTransformPreview,
    pub(super) layout_projected: bool,
}

impl fmt::Debug for DockspaceContainedTransformPreview<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceContainedTransformPreview")
            .field("surface", &self.surface())
            .field("floating", &self.floating())
            .field("rect", &self.rect())
            .field("layout_projected", &self.is_layout_projected())
            .finish()
    }
}

impl DockspaceContainedTransformPreview<'_> {
    /// Returns the logical surface which must paint the preview.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.preview.surface()
    }

    /// Returns the contained presentation being resized.
    #[must_use]
    pub const fn floating(self) -> FloatingPresentationId {
        self.preview.floating()
    }

    /// Returns the exact rectangle which must be painted.
    #[must_use]
    pub const fn rect(self) -> LogicalRect {
        self.preview.rect()
    }

    /// Returns whether this output already lays out the complete root at this rectangle.
    ///
    /// The preview remains part of the exact output obligation; adapters must
    /// not paint a duplicate shell over the projected content.
    #[must_use]
    pub const fn is_layout_projected(self) -> bool {
        self.layout_projected
    }
}

/// Read-only splitter geometry and resize constraints.
#[derive(Debug, Clone, Copy)]
pub struct SplitterPaintRecord<'plan> {
    pub(super) record: &'plan SplitterRecord,
    pub(super) output: DockspaceSemanticOutput,
}

impl SplitterPaintRecord<'_> {
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Splitter(*self.record.id()))
    }

    /// Returns the host input identity for this splitter.
    #[must_use]
    pub fn input_id(self) -> DockspaceControlId {
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::Splitter(SplitterControlTarget::from_record(self.record)),
        )
    }

    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    #[must_use]
    pub const fn draw_bounds(self) -> LogicalRect {
        self.record.draw_bounds()
    }

    #[must_use]
    pub const fn hit_bounds(self) -> LogicalRect {
        self.record.hit().rect()
    }

    #[must_use]
    pub const fn axis(self) -> DockspaceAxis {
        match self.record.axis() {
            Axis::Horizontal => DockspaceAxis::Horizontal,
            Axis::Vertical => DockspaceAxis::Vertical,
        }
    }

    #[must_use]
    pub const fn before_bounds(self) -> LogicalRect {
        self.record.before_bounds()
    }

    #[must_use]
    pub const fn after_bounds(self) -> LogicalRect {
        self.record.after_bounds()
    }

    #[must_use]
    pub const fn minimum_extents(self) -> (f64, f64) {
        (
            self.record.before_minimum_extent(),
            self.record.after_minimum_extent(),
        )
    }

    #[must_use]
    pub const fn maximum_extents(self) -> (f64, f64) {
        (
            self.record.before_maximum_extent(),
            self.record.after_maximum_extent(),
        )
    }

    #[must_use]
    pub const fn operable(self) -> bool {
        self.record.operable()
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Read-only atomic splitter-junction geometry.
#[derive(Clone, Copy)]
pub struct SplitterJunctionPaintRecord<'plan> {
    pub(super) record: &'plan SplitterJunctionRecord,
    pub(super) plan: &'plan PresentationPlan,
    pub(super) output: DockspaceSemanticOutput,
}

impl fmt::Debug for SplitterJunctionPaintRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SplitterJunctionPaintRecord")
            .field("root", &self.root())
            .field("hit_bounds", &self.hit_bounds())
            .field("arm_count", &self.arm_count())
            .field("layer", &self.layer())
            .finish_non_exhaustive()
    }
}

impl SplitterJunctionPaintRecord<'_> {
    /// Returns the stable root containing every incident splitter.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root()
    }

    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::SplitterJunction(self.record.id()))
    }

    /// Returns the host input identity for this junction.
    #[must_use]
    pub fn input_id(self) -> DockspaceControlId {
        let id = self.record.id();
        let boundaries = [
            SplitterJunctionDirection::North,
            SplitterJunctionDirection::East,
            SplitterJunctionDirection::South,
            SplitterJunctionDirection::West,
        ]
        .map(|direction| {
            self.plan
                .splitter_record(id.arm(direction)?)
                .map(SplitterControlTarget::from_record)
        });
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::Junction(Arc::new(boundaries)),
        )
    }

    #[must_use]
    pub const fn hit_bounds(self) -> LogicalRect {
        self.record.hit().rect()
    }

    #[must_use]
    pub fn arm_count(self) -> usize {
        self.record.id().arm_count()
    }

    /// Returns whether this junction contains at least one incident splitter
    /// which moves along `axis`.
    #[must_use]
    pub const fn has_axis(self, axis: DockspaceAxis) -> bool {
        let id = self.record.id();
        match axis {
            DockspaceAxis::Horizontal => {
                id.arm(SplitterJunctionDirection::North).is_some()
                    || id.arm(SplitterJunctionDirection::South).is_some()
            }
            DockspaceAxis::Vertical => {
                id.arm(SplitterJunctionDirection::East).is_some()
                    || id.arm(SplitterJunctionDirection::West).is_some()
            }
        }
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Read-only contained-floating resize geometry.
#[derive(Debug, Clone, Copy)]
pub struct ContainedResizePaintRecord {
    pub(super) record: ContainedResizeRecord,
    operable: bool,
    pub(super) output: DockspaceSemanticOutput,
    pub(super) floating: FloatingPresentationId,
    root: RootId,
}

/// Product-facing chrome host of one root presentation command menu anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockspacePresentationMenuAnchorKind {
    /// The sole core-selected tab bar of a main or native-main root.
    TabBar,
    /// An independent band outside a main root's application content.
    RootControls,
    /// The title chrome of a contained-floating root.
    ContainedTitle,
}

/// Opaque read-only root presentation command menu anchor.
#[derive(Clone, Copy)]
pub struct PresentationMenuAnchorPaintRecord<'plan> {
    pub(super) record: &'plan PresentationMenuAnchorRecord,
    pub(super) bounds: LogicalRect,
    pub(super) operable: bool,
    pub(super) scope: ControlScope,
}

impl fmt::Debug for PresentationMenuAnchorPaintRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PresentationMenuAnchorPaintRecord")
            .field("visual_id", &self.visual_id())
            .field("kind", &self.kind())
            .field("bounds", &self.bounds())
            .field("layer", &self.layer())
            .field("operable", &self.operable())
            .finish_non_exhaustive()
    }
}

impl PresentationMenuAnchorPaintRecord<'_> {
    /// Returns the stable visual identity of this root-scoped control.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::PresentationMenuAnchor(self.record.root()))
    }

    /// Returns the host input identity for this root's command-menu opener.
    #[must_use]
    pub const fn input_id(self) -> DockspaceControlId {
        DockspaceControlId(self.scope, ControlTarget::Visual(self.visual_id()))
    }

    /// Returns the stable root whose commands this anchor opens.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.root()
    }

    /// Returns whether this anchor belongs to tab or contained-title chrome.
    #[must_use]
    pub const fn kind(self) -> DockspacePresentationMenuAnchorKind {
        match self.record.host() {
            PresentationMenuAnchorHost::TabBar(_) => DockspacePresentationMenuAnchorKind::TabBar,
            PresentationMenuAnchorHost::RootControls => {
                DockspacePresentationMenuAnchorKind::RootControls
            }
            PresentationMenuAnchorHost::ContainedTitle(_) => {
                DockspacePresentationMenuAnchorKind::ContainedTitle
            }
        }
    }

    /// Returns the opaque visual identity of the chrome hosting this anchor.
    #[must_use]
    pub const fn host_visual_id(self) -> DockspaceVisualId {
        match self.record.host() {
            PresentationMenuAnchorHost::RootControls => self.visual_id(),
            PresentationMenuAnchorHost::TabBar(bar) => {
                DockspaceVisualId(VisualIdentity::TabBar(bar))
            }
            PresentationMenuAnchorHost::ContainedTitle(floating) => {
                DockspaceVisualId(VisualIdentity::Contained(floating))
            }
        }
    }

    /// Returns the exact core-reserved draw and response bounds.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.bounds
    }

    /// Returns the independent control band's allocation, absent for tab/title anchors.
    #[must_use]
    pub const fn reserved_bounds(self) -> Option<LogicalRect> {
        self.record.reserved_bounds()
    }

    /// Returns the roster-derived paint layer.
    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }

    /// Returns whether any authoritative area of this exact control remains exposed.
    #[must_use]
    pub const fn operable(self) -> bool {
        self.operable
    }
}

impl ContainedResizePaintRecord {
    /// Returns the host input identity for this exact contained resize edge.
    #[must_use]
    pub const fn input_id(self) -> DockspaceControlId {
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::ContainedResize(self.floating, self.root, self.direction()),
        )
    }

    #[must_use]
    pub const fn direction(self) -> ContainedResizeDirection {
        ContainedResizeDirection::from_core(self.record.direction())
    }

    #[must_use]
    pub const fn hit_bounds(self) -> LogicalRect {
        self.record.hit().rect()
    }

    /// Returns whether the exact contained root, its items, and its owning
    /// surface permit a transform under the policy frozen into this plan.
    #[must_use]
    pub const fn operable(self) -> bool {
        self.operable
    }
}

/// Read-only contained-floating chrome and transform geometry.
#[derive(Clone, Copy)]
pub struct ContainedPaintRecord<'plan> {
    pub(super) record: &'plan ContainedRecord,
    plan: &'plan PresentationPlan,
    pub(super) output: DockspaceSemanticOutput,
    revision: WorkspaceRevision,
}

impl fmt::Debug for ContainedPaintRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContainedPaintRecord")
            .field("visual_id", &self.visual_id())
            .field("outer_bounds", &self.outer_bounds())
            .field("title_operable", &self.title_operable())
            .field("close_operable", &self.close_operable())
            .finish_non_exhaustive()
    }
}

impl<'plan> ContainedPaintRecord<'plan> {
    pub(super) fn new(plan: &SurfacePaintPlan<'plan>, record: &'plan ContainedRecord) -> Self {
        Self {
            record,
            plan: plan.plan,
            output: plan.semantic_output(),
            revision: plan.version.revision(),
        }
    }

    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Contained(self.record.floating()))
    }

    /// Returns the host input identity for the contained title control.
    #[must_use]
    pub const fn input_id(self) -> DockspaceControlId {
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::ContainedTitle(self.floating(), self.root()),
        )
    }

    /// Returns the host input identity for closing the complete contained root.
    ///
    /// Workspace revisions invalidate this identity so a delayed activation
    /// cannot close a different set of panes in the same floating window.
    #[must_use]
    pub const fn close_input_id(self) -> DockspaceControlId {
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::ContainedClose(self.floating(), self.root(), self.revision),
        )
    }

    /// Returns this root's frozen content or container descriptor.
    #[must_use]
    pub fn label(self) -> crate::graph::TabLabel<'plan> {
        self.record
            .label()
            .or_else(|| self.plan.root_label(self.record.root()))
            .unwrap_or(crate::graph::TabLabel::Layout { title: "Layout" })
    }

    #[must_use]
    pub const fn floating(self) -> FloatingPresentationId {
        self.record.floating()
    }

    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.root()
    }

    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.record.ordinal()
    }

    /// Returns whether this presentation is frontmost in the complete core
    /// roster, including contained presentations clipped from this plan.
    #[must_use]
    pub const fn is_frontmost(self) -> bool {
        self.record.is_frontmost()
    }

    #[must_use]
    pub const fn outer_bounds(self) -> LogicalRect {
        self.record.outer_bounds()
    }

    #[must_use]
    pub const fn inner_bounds(self) -> LogicalRect {
        self.record.inner_bounds()
    }

    #[must_use]
    pub const fn title_bounds(self) -> LogicalRect {
        self.record.title_bounds()
    }

    #[must_use]
    pub const fn title_drag_bounds(self) -> LogicalRect {
        self.record.title_drag_hit().rect()
    }

    #[must_use]
    pub const fn content_bounds(self) -> LogicalRect {
        self.record.content_bounds()
    }

    #[must_use]
    pub const fn close_bounds(self) -> Option<LogicalRect> {
        self.record.close_bounds()
    }

    /// Returns whether any authoritative area of the title-drag control remains exposed.
    #[must_use]
    pub fn title_operable(self) -> bool {
        self.plan
            .region_is_operable(self.record.title_drag_hit().rect(), self.record.layer())
    }

    /// Returns whether the exact close control exists and remains exposed.
    #[must_use]
    pub fn close_operable(self) -> bool {
        self.record
            .close_bounds()
            .is_some_and(|bounds| self.plan.region_is_operable(bounds, self.record.layer()))
    }

    pub fn resize(self) -> impl ExactSizeIterator<Item = ContainedResizePaintRecord> + 'plan {
        let plan = self.plan;
        let layer = self.record.layer();
        let transform_operable = self.record.transform_operable();
        self.record
            .resize()
            .iter()
            .copied()
            .map(move |record| ContainedResizePaintRecord {
                operable: transform_operable && plan.region_is_operable(record.hit().rect(), layer),
                record,
                output: self.output,
                floating: self.floating(),
                root: self.root(),
            })
    }

    #[must_use]
    pub const fn minimum_size(self) -> LogicalSize {
        self.record.minimum_size()
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Stable root-level presentation command shown by renderer-owned menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockspacePresentationCommandKind {
    /// Present the root as contained floating content in its current logical surface.
    Float,
    /// Promote the root into a managed native child window.
    MoveToNewWindow,
    /// Recover the root into its current core-derived docking destination.
    DockBack,
    /// Dock the root at the left edge of its core-derived recovery target.
    DockLeft,
    /// Dock the root at the right edge of its core-derived recovery target.
    DockRight,
    /// Dock the root at the top edge of its core-derived recovery target.
    DockTop,
    /// Dock the root at the bottom edge of its core-derived recovery target.
    DockBottom,
    /// Merge the root into the center of its core-derived recovery target.
    DockCenter,
}

/// Why one exact presentation command is disabled in the current paint candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockspacePresentationCommandUnavailable {
    /// The root is already presented as contained floating content.
    AlreadyContained,
    /// The root is already the main presentation of a managed native child and cannot be
    /// promoted into another child window.
    AlreadyNative,
    /// Core rejected the product action for its existing stable reason.
    Action(DockspaceActionRejection),
}

/// One opaque layout command frozen from an exact paint candidate.
#[derive(Clone, PartialEq)]
pub struct PresentationCommandPaintRecord {
    pub(super) version: crate::model::WorkspaceVersion,
    pub(super) output: DockspaceSemanticOutput,
    pub(super) root: RootId,
    pub(super) kind: DockspacePresentationCommandKind,
    pub(super) action: Option<ProductAction>,
    pub(super) unavailable: Option<DockspacePresentationCommandUnavailable>,
}

impl PresentationCommandPaintRecord {
    pub(super) fn ready(
        plan: SurfacePaintPlan<'_>,
        root: RootId,
        kind: DockspacePresentationCommandKind,
        action: ProductAction,
    ) -> Self {
        Self {
            version: plan.version,
            output: plan.semantic_output(),
            root,
            kind,
            action: Some(action),
            unavailable: None,
        }
    }

    pub(super) fn unavailable(
        plan: SurfacePaintPlan<'_>,
        root: RootId,
        kind: DockspacePresentationCommandKind,
        reason: DockspacePresentationCommandUnavailable,
    ) -> Self {
        Self {
            version: plan.version,
            output: plan.semantic_output(),
            root,
            kind,
            action: None,
            unavailable: Some(reason),
        }
    }

    /// Returns the root containing this command's source payload.
    #[must_use]
    pub const fn root(&self) -> RootId {
        self.root
    }

    /// Returns this command's stable semantic kind.
    #[must_use]
    pub const fn kind(&self) -> DockspacePresentationCommandKind {
        self.kind
    }

    /// Returns whether core admitted this command for the exact paint candidate.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.action.is_some()
    }

    /// Returns the stable reason this command is disabled, if any.
    #[must_use]
    pub const fn unavailable_reason(&self) -> Option<DockspacePresentationCommandUnavailable> {
        self.unavailable
    }
}

impl fmt::Debug for PresentationCommandPaintRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PresentationCommandPaintRecord")
            .field("surface", &self.output.surface())
            .field("root", &self.root)
            .field("kind", &self.kind)
            .field("ready", &self.is_ready())
            .field("unavailable", &self.unavailable)
            .finish_non_exhaustive()
    }
}
