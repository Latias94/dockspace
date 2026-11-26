//! Typed visitor and owned `'static` display list for surface paint programs.
//!
//! Immediate-mode renderers can implement [`SurfaceProgramVisitor`] (overriding
//! only the chrome primitives they draw) and execute via
//! [`SurfacePaintPlan::execute_visitor`].
//!
//! Retained-mode UIs and deferred GPU renderers can call
//! [`SurfacePaintPlan::compile_display_list`] to materialize an owned,
//! `'static` [`SurfaceDisplayList`] together with its affine
//! [`SurfaceProgramExecutionReceipt`], then immediately confirm and commit the
//! host frame without holding any borrow on the session.

use std::convert::Infallible;

use crate::geometry::{LogicalRect, LogicalSize};
use crate::model::{
    DockspaceAxis, FloatingPresentationId, ItemId, RootId, SurfaceId, TabKey, TabLabel,
};
use crate::policy::TabBarInteraction;
use crate::runtime::{
    ContainedPaintRecord, ContainedResizeDirection, ContainedResizePaintRecord,
    DockspaceContainedTransformPreview, DockspaceControlId, DockspaceDragDecoration,
    DockspaceDragPreview, DockspaceDragSourceKind, DockspaceDropDirection,
    DockspaceDropEligibility, DockspaceDropTargetFeedback, DockspaceGuideScope,
    DockspacePaintLayer, DockspacePresentationMenuAnchorKind, DockspacePreviewTabBarPaintRecord,
    DockspacePreviewTabPaintRecord, DockspacePreviewVisual, DockspaceReceiverDescriptor,
    DockspaceRuntimeError, DockspaceSemanticOutput, DockspaceVisualId,
    DropAffordanceClusterPaintRecord, DropAffordanceTargetPaintRecord, DropGuidePaintRecord,
    DropGuideTargetPaintRecord, DropTargetPaintRecord, PaneContentGeometry, PanePaintRecord,
    PresentationMenuAnchorPaintRecord, SplitterJunctionPaintRecord, SplitterPaintRecord,
    SurfacePaintDisposition, SurfacePaintPlan, SurfacePaintProgramOperation,
    SurfacePaintProgramRoot, SurfacePaintProgramRootKind, SurfacePaintProgramStep,
    SurfacePaintProgramVisitError, SurfaceProgramExecutionReceipt, TabBarPaintRecord,
    TabGroupDragRegionKind, TabGroupDragRegionPaintRecord, TabListMenuBackdropPaintRecord,
    TabListMenuPaintRecord, TabListMenuRowId, TabListMenuRowPaintRecord, TabPaintRecord,
    TabStripControlKind, TabStripControlPaintRecord, TabStripMemberPaintRecord,
    TabStripMemberVisibility, TabsPaintRecord,
};

/// Typed visitor for executing or inspecting a canonical surface paint program.
///
/// Every method defaults to a no-op returning `Ok(())`, so a new UI integration
/// can start by implementing [`Self::pane`], [`Self::tab_bar`], [`Self::tab`],
/// and [`Self::splitter`], and opt into floating chrome, popups, drag previews,
/// and docking guides incrementally.
pub trait SurfaceProgramVisitor {
    /// Renderer error type returned if a step cannot be accepted.
    type Error;

    /// Called at the start of one main or contained-floating root segment.
    fn root_begin(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one active application pane content slot.
    fn pane(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _pane: PanePaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one active Tabs container background frame.
    fn tabs(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _tabs: TabsPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one tab strip header background and overflow viewport.
    fn tab_bar(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _bar: TabBarPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one visible tab header inside its strip.
    fn tab(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _tab: TabPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one resizable split or grid divider.
    fn splitter(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _splitter: SplitterPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one atomic multi-axis splitter junction.
    fn splitter_junction(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _junction: SplitterJunctionPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one core-owned tab-strip overflow/menu control.
    fn tab_strip_control(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _control: TabStripControlPaintRecord,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for the title bar and resize borders of a contained-floating root.
    fn contained_controls(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _contained: ContainedPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one root-scoped presentation command menu anchor.
    fn presentation_menu_anchor(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        _anchor: PresentationMenuAnchorPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called at the end of the active root segment.
    fn root_end(&mut self, _root: SurfacePaintProgramRoot<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for a workspace-global popup backdrop.
    fn popup_backdrop(
        &mut self,
        _backdrop: TabListMenuBackdropPaintRecord,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for an open tab-list overflow menu popup.
    fn popup_menu(
        &mut self,
        _menu: TabListMenuPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for the active docking or rehome drag preview.
    fn drag_preview(&mut self, _preview: DockspaceDragPreview<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one non-visual drop target receiver in core hit order.
    fn drop_target(&mut self, _target: DropTargetPaintRecord<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for one docking-guide cluster and its optional live drag affordance.
    fn drop_guide(
        &mut self,
        _structural: DropGuidePaintRecord<'_>,
        _affordance: Option<DropAffordanceClusterPaintRecord<'_>>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for the active contained-floating resize/move preview.
    fn contained_transform_preview(
        &mut self,
        _preview: DockspaceContainedTransformPreview<'_>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called for the active drag source ghost decoration.
    fn drag_decoration(
        &mut self,
        _decoration: DockspaceDragDecoration<'_>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called at the interaction-feedback checkpoint after popup steps.
    fn interaction_feedback(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called once when the complete program seals for `semantic_output`.
    fn surface_end(
        &mut self,
        _semantic_output: DockspaceSemanticOutput,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn dispatch_step<V: SurfaceProgramVisitor>(
    visitor: &mut V,
    step: SurfacePaintProgramStep<'_>,
) -> Result<(), V::Error> {
    match step {
        SurfacePaintProgramStep::RootBegin { root, paint } => visitor.root_begin(root, paint),
        SurfacePaintProgramStep::Operation {
            root,
            operation,
            paint,
        } => match operation {
            SurfacePaintProgramOperation::Pane(pane) => visitor.pane(root, pane, paint),
            SurfacePaintProgramOperation::Tabs(tabs) => visitor.tabs(root, tabs, paint),
            SurfacePaintProgramOperation::TabBar(bar) => visitor.tab_bar(root, bar, paint),
            SurfacePaintProgramOperation::Tab(tab) => visitor.tab(root, tab, paint),
            SurfacePaintProgramOperation::Splitter(splitter) => {
                visitor.splitter(root, splitter, paint)
            }
            SurfacePaintProgramOperation::SplitterJunction(junction) => {
                visitor.splitter_junction(root, junction, paint)
            }
            SurfacePaintProgramOperation::TabStripControl(control) => {
                visitor.tab_strip_control(root, control, paint)
            }
            SurfacePaintProgramOperation::ContainedControls(contained) => {
                visitor.contained_controls(root, contained, paint)
            }
            SurfacePaintProgramOperation::PresentationMenuAnchor(anchor) => {
                visitor.presentation_menu_anchor(root, anchor, paint)
            }
        },
        SurfacePaintProgramStep::RootEnd(root) => visitor.root_end(root),
        SurfacePaintProgramStep::PopupBackdrop(backdrop) => visitor.popup_backdrop(backdrop),
        SurfacePaintProgramStep::PopupMenu { menu, paint } => visitor.popup_menu(menu, paint),
        SurfacePaintProgramStep::DragPreview(preview) => visitor.drag_preview(preview),
        SurfacePaintProgramStep::DropTarget(target) => visitor.drop_target(target),
        SurfacePaintProgramStep::DropGuide {
            structural,
            affordance,
        } => visitor.drop_guide(structural, affordance),
        SurfacePaintProgramStep::ContainedTransformPreview(preview) => {
            visitor.contained_transform_preview(preview)
        }
        SurfacePaintProgramStep::DragDecoration(decoration) => visitor.drag_decoration(decoration),
        SurfacePaintProgramStep::InteractionFeedback => visitor.interaction_feedback(),
        SurfacePaintProgramStep::SurfaceEnd { semantic_output } => {
            visitor.surface_end(semantic_output)
        }
    }
}

/// Owned, `'static` equivalent of [`TabLabel`] for retained UIs and display lists.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OwnedTabLabel {
    /// Application-owned item whose title comes from the host's item registry.
    Item(ItemId),
    /// Core-owned structural container label (for example a named Split or Grid tab).
    Layout(String),
}

impl OwnedTabLabel {
    /// Borrows this label as a core [`TabLabel`].
    #[must_use]
    pub fn as_borrowed(&self) -> TabLabel<'_> {
        match self {
            Self::Item(item) => TabLabel::Item(*item),
            Self::Layout(title) => TabLabel::Layout {
                title: title.as_str(),
            },
        }
    }
}

impl From<TabLabel<'_>> for OwnedTabLabel {
    fn from(label: TabLabel<'_>) -> Self {
        match label {
            TabLabel::Item(item) => Self::Item(item),
            TabLabel::Layout { title } => Self::Layout(title.to_owned()),
        }
    }
}

/// Owned contained-floating resize edge record.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedContainedResize {
    /// Host widget/accessibility input identity for this resize edge.
    pub input_id: DockspaceControlId,
    /// Cardinal or diagonal resize direction.
    pub direction: ContainedResizeDirection,
    /// Logical hit rectangle for this resize edge.
    pub hit_bounds: LogicalRect,
    /// Whether policy and occlusion allow resizing this edge.
    pub operable: bool,
}

impl From<ContainedResizePaintRecord> for OwnedContainedResize {
    fn from(record: ContainedResizePaintRecord) -> Self {
        Self {
            input_id: record.input_id(),
            direction: record.direction(),
            hit_bounds: record.hit_bounds(),
            operable: record.operable(),
        }
    }
}

/// Owned contained-floating root chrome geometry and identities.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedContainedFrame {
    /// Stable visual identity of this contained presentation.
    pub visual_id: DockspaceVisualId,
    /// Host input identity for the title bar control.
    pub input_id: DockspaceControlId,
    /// Host input identity for the close button (revision-scoped).
    pub close_input_id: DockspaceControlId,
    /// Frozen root label.
    pub label: OwnedTabLabel,
    /// Floating presentation identifier.
    pub floating: FloatingPresentationId,
    /// Owning root identifier.
    pub root: RootId,
    /// Z-order ordinal among contained roots on this surface.
    pub ordinal: usize,
    /// Whether this floating frame is frontmost in the complete core roster.
    pub is_frontmost: bool,
    /// Outer frame bounds including borders.
    pub outer_bounds: LogicalRect,
    /// Inner frame bounds inside borders.
    pub inner_bounds: LogicalRect,
    /// Title bar draw bounds.
    pub title_bounds: LogicalRect,
    /// Title bar drag hit bounds.
    pub title_drag_bounds: LogicalRect,
    /// Content area bounds below the title bar.
    pub content_bounds: LogicalRect,
    /// Close button bounds when close is enabled.
    pub close_bounds: Option<LogicalRect>,
    /// Whether the title drag area remains exposed and operable.
    pub title_operable: bool,
    /// Whether the close button exists and remains exposed.
    pub close_operable: bool,
    /// Minimum logical size for this contained frame.
    pub minimum_size: LogicalSize,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Resize edges and corners in canonical order.
    pub resize_edges: Vec<OwnedContainedResize>,
}

impl From<ContainedPaintRecord<'_>> for OwnedContainedFrame {
    fn from(record: ContainedPaintRecord<'_>) -> Self {
        Self {
            visual_id: record.visual_id(),
            input_id: record.input_id(),
            close_input_id: record.close_input_id(),
            label: record.label().into(),
            floating: record.floating(),
            root: record.root(),
            ordinal: record.ordinal(),
            is_frontmost: record.is_frontmost(),
            outer_bounds: record.outer_bounds(),
            inner_bounds: record.inner_bounds(),
            title_bounds: record.title_bounds(),
            title_drag_bounds: record.title_drag_bounds(),
            content_bounds: record.content_bounds(),
            close_bounds: record.close_bounds(),
            title_operable: record.title_operable(),
            close_operable: record.close_operable(),
            minimum_size: record.minimum_size(),
            layer: record.layer(),
            resize_edges: record.resize().map(OwnedContainedResize::from).collect(),
        }
    }
}

/// Owned root segment kind.
#[derive(Debug, Clone, PartialEq)]
pub enum OwnedRootKind {
    /// Ordinary main root of the logical surface.
    Main,
    /// Contained-floating root painted in z-order.
    Contained(OwnedContainedFrame),
}

/// Owned root header opening or closing a root segment in [`SurfaceDisplayList`].
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedSurfaceRoot {
    /// Stable root identifier.
    pub root: RootId,
    /// Main or contained-floating root classification.
    pub kind: OwnedRootKind,
    /// Frozen root label.
    pub label: OwnedTabLabel,
}

impl From<SurfacePaintProgramRoot<'_>> for OwnedSurfaceRoot {
    fn from(root: SurfacePaintProgramRoot<'_>) -> Self {
        let kind = match root.kind() {
            SurfacePaintProgramRootKind::Main => OwnedRootKind::Main,
            SurfacePaintProgramRootKind::Contained(contained) => {
                OwnedRootKind::Contained(contained.into())
            }
        };
        Self {
            root: root.root(),
            kind,
            label: root.label().into(),
        }
    }
}

/// Owned pane content placement record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedPane {
    /// Stable visual identity of this pane.
    pub visual_id: DockspaceVisualId,
    /// Nearest active containing tab strip, if any.
    pub tab_bar_visual_id: Option<DockspaceVisualId>,
    /// Owning root identifier.
    pub root: RootId,
    /// Application item identifier.
    pub item: ItemId,
    /// Full allocation and visible clip rectangle in surface-local logical coordinates.
    pub content_geometry: PaneContentGeometry,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition (`Visible` or `SourceGap`).
    pub paint: SurfacePaintDisposition,
}

/// Owned Tabs container background record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedTabs {
    /// Stable visual identity of this container.
    pub visual_id: DockspaceVisualId,
    /// Owning root identifier.
    pub root: RootId,
    /// Complete container bounds.
    pub bounds: LogicalRect,
    /// Child content slot bounds below the header.
    pub content_bounds: LogicalRect,
    /// Exact container clip rectangle.
    pub clip: LogicalRect,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned whole-group drag region on a tab bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedTabGroupDragRegion {
    /// Stable visual identity for this region.
    pub visual_id: DockspaceVisualId,
    /// Leading grip or trailing empty strip region.
    pub kind: TabGroupDragRegionKind,
    /// Logical draw and hit bounds.
    pub bounds: LogicalRect,
}

impl From<TabGroupDragRegionPaintRecord> for OwnedTabGroupDragRegion {
    fn from(record: TabGroupDragRegionPaintRecord) -> Self {
        Self {
            visual_id: record.visual_id(),
            kind: record.kind(),
            bounds: record.bounds(),
        }
    }
}

/// Owned tab-strip member summary including clipped visibility.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedTabStripMember {
    /// Stable visual identity of the tab.
    pub visual_id: DockspaceVisualId,
    /// Owning tab bar visual identity.
    pub tab_bar_visual_id: DockspaceVisualId,
    /// Direct tab key (item or layout subtree).
    pub key: TabKey,
    /// Direct item identifier, absent for layout tabs.
    pub item: Option<ItemId>,
    /// Frozen tab label.
    pub label: OwnedTabLabel,
    /// Stable order index in the strip.
    pub ordinal: usize,
    /// Unclipped allocated rectangle in strip coordinates.
    pub full_bounds: LogicalRect,
    /// Clipped visibility relative to the strip viewport.
    pub visibility: TabStripMemberVisibility,
}

impl From<TabStripMemberPaintRecord<'_>> for OwnedTabStripMember {
    fn from(record: TabStripMemberPaintRecord<'_>) -> Self {
        Self {
            visual_id: record.visual_id(),
            tab_bar_visual_id: record.tab_bar_visual_id(),
            key: record.key(),
            item: record.item(),
            label: record.label().into(),
            ordinal: record.ordinal(),
            full_bounds: record.full_bounds(),
            visibility: record.visibility(),
        }
    }
}

/// Owned tab bar geometry, overflow, and chrome regions.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedTabBar {
    /// Stable visual identity of this tab bar.
    pub visual_id: DockspaceVisualId,
    /// Host input identity for the group command menu opener.
    pub group_menu_input_id: DockspaceControlId,
    /// Independent Tabs container visual identity.
    pub tabs_visual_id: DockspaceVisualId,
    /// Owning root identifier.
    pub root: RootId,
    /// Currently selected direct child, even if scrolled out of view.
    pub selected: Option<TabKey>,
    /// Complete tab bar bounds.
    pub bounds: LogicalRect,
    /// Clipped tab viewport bounds inside the bar.
    pub viewport: LogicalRect,
    /// Current horizontal scroll offset.
    pub scroll_offset: f64,
    /// Maximum horizontal scroll offset.
    pub maximum_scroll_offset: f64,
    /// Tabs currently clipped out of view.
    pub hidden_tabs: Vec<TabKey>,
    /// Complete ordered member roster.
    pub members: Vec<OwnedTabStripMember>,
    /// Optional leading group drag grip bounds.
    pub group_grip_bounds: Option<LogicalRect>,
    /// Optional group action menu anchor bounds.
    pub group_menu_bounds: Option<LogicalRect>,
    /// Optional core-clipped leading application chrome allocation.
    pub leading_reserved_bounds: Option<LogicalRect>,
    /// Optional core-clipped trailing application chrome allocation.
    pub trailing_reserved_bounds: Option<LogicalRect>,
    /// Optional scrolling application chrome immediately after the last tab.
    pub after_tabs_bounds: Option<LogicalRect>,
    /// Whole-group drag regions in visual order.
    pub group_drag_regions: Vec<OwnedTabGroupDragRegion>,
    /// Policy interaction mode for this strip.
    pub interaction: TabBarInteraction,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned visible tab header geometry, identities, and state.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedTab {
    /// Stable visual identity of this tab.
    pub visual_id: DockspaceVisualId,
    /// Host input identity for the tab header.
    pub input_id: DockspaceControlId,
    /// Host input identity for the tab close button.
    pub close_input_id: DockspaceControlId,
    /// Owning tab bar visual identity.
    pub tab_bar_visual_id: DockspaceVisualId,
    /// Owning root identifier.
    pub root: RootId,
    /// Direct tab key (item or layout subtree).
    pub key: TabKey,
    /// Application item identifier, absent for layout tabs.
    pub item: Option<ItemId>,
    /// Frozen tab label.
    pub label: OwnedTabLabel,
    /// Full tab rectangle before strip clipping.
    pub full_bounds: LogicalRect,
    /// Clipped visible tab rectangle.
    pub visible_bounds: LogicalRect,
    /// Visible title clip rectangle.
    pub text_bounds: LogicalRect,
    /// Full title layout rectangle before strip clipping.
    pub text_layout_bounds: LogicalRect,
    /// Drag hit rectangle.
    pub drag_bounds: LogicalRect,
    /// Visual bounds of the close button, if shown.
    pub close_visual_bounds: Option<LogicalRect>,
    /// Hit bounds of the close button, if operable.
    pub close_bounds: Option<LogicalRect>,
    /// Whether this tab is currently selected.
    pub selected: bool,
    /// Whether this tab body remains exposed and operable.
    pub operable: bool,
    /// Visual ordinal in the strip.
    pub ordinal: usize,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned resizable split or grid divider.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedSplitter {
    /// Stable visual identity of this divider.
    pub visual_id: DockspaceVisualId,
    /// Host input identity for this divider.
    pub input_id: DockspaceControlId,
    /// Owning root identifier.
    pub root: RootId,
    /// Visual divider rectangle.
    pub draw_bounds: LogicalRect,
    /// Expanded pointer hit rectangle.
    pub hit_bounds: LogicalRect,
    /// Divider resize axis.
    pub axis: DockspaceAxis,
    /// Adjacent bounds before the divider.
    pub before_bounds: LogicalRect,
    /// Adjacent bounds after the divider.
    pub after_bounds: LogicalRect,
    /// Minimum extents of the adjacent children `(before, after)`.
    pub minimum_extents: (f64, f64),
    /// Maximum extents of the adjacent children `(before, after)`.
    pub maximum_extents: (f64, f64),
    /// Whether policy and occlusion permit dragging this divider.
    pub operable: bool,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned atomic splitter junction hit record.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedSplitterJunction {
    /// Stable visual identity of this junction.
    pub visual_id: DockspaceVisualId,
    /// Host input identity for this junction.
    pub input_id: DockspaceControlId,
    /// Owning root identifier.
    pub root: RootId,
    /// Pointer hit rectangle.
    pub hit_bounds: LogicalRect,
    /// Number of incident splitter arms.
    pub arm_count: usize,
    /// Whether any incident arm resizes horizontally.
    pub has_horizontal: bool,
    /// Whether any incident arm resizes vertically.
    pub has_vertical: bool,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned tab-strip overflow/menu control.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedTabStripControl {
    /// Stable visual identity of this control.
    pub visual_id: DockspaceVisualId,
    /// Host input identity of this control.
    pub input_id: DockspaceControlId,
    /// Owning tab bar visual identity.
    pub tab_bar_visual_id: DockspaceVisualId,
    /// Owning root identifier.
    pub root: RootId,
    /// Semantic control kind.
    pub kind: TabStripControlKind,
    /// Visual draw rectangle.
    pub bounds: LogicalRect,
    /// Pointer hit rectangle.
    pub hit_bounds: LogicalRect,
    /// Whether the control is enabled.
    pub enabled: bool,
    /// Whether this control owns an open tab-list menu.
    pub menu_open: bool,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned root-scoped presentation menu anchor control.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedPresentationMenuAnchor {
    /// Stable visual identity of this anchor.
    pub visual_id: DockspaceVisualId,
    /// Host input identity of this anchor.
    pub input_id: DockspaceControlId,
    /// Root whose presentation commands this anchor opens.
    pub root: RootId,
    /// Chrome host classification.
    pub kind: DockspacePresentationMenuAnchorKind,
    /// Visual identity of the hosting tab bar or contained frame.
    pub host_visual_id: DockspaceVisualId,
    /// Draw and hit rectangle.
    pub bounds: LogicalRect,
    /// Independent control band allocation, when present.
    pub reserved_bounds: Option<LogicalRect>,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Whether the anchor remains exposed and operable.
    pub operable: bool,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned popup backdrop geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedPopupBackdrop {
    /// Stable visual identity of this backdrop.
    pub visual_id: DockspaceVisualId,
    /// Associated tab-list menu visual identity.
    pub menu_visual_id: DockspaceVisualId,
    /// Full-surface backdrop rectangle.
    pub bounds: LogicalRect,
}

impl From<TabListMenuBackdropPaintRecord> for OwnedPopupBackdrop {
    fn from(record: TabListMenuBackdropPaintRecord) -> Self {
        Self {
            visual_id: record.visual_id(),
            menu_visual_id: record.menu_visual_id(),
            bounds: record.bounds(),
        }
    }
}

/// Owned row inside an open tab-list popup menu.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedPopupMenuRow {
    /// Row identity for keyboard/accessibility routing.
    pub input_id: TabListMenuRowId,
    /// Row visual identity.
    pub visual_id: DockspaceVisualId,
    /// Associated tab visual identity.
    pub tab_visual_id: DockspaceVisualId,
    /// Direct tab key.
    pub key: TabKey,
    /// Application item identifier, absent for layout tabs.
    pub item: Option<ItemId>,
    /// Frozen tab label.
    pub label: OwnedTabLabel,
    /// Row index in the menu.
    pub ordinal: usize,
    /// Full row layout rectangle.
    pub bounds: LogicalRect,
    /// Clipped visible hit rectangle, absent when scrolled out of view.
    pub interaction_bounds: Option<LogicalRect>,
    /// Whether this row represents the selected tab.
    pub selected: bool,
    /// Whether this row currently holds menu keyboard focus.
    pub focused: bool,
}

impl From<TabListMenuRowPaintRecord<'_>> for OwnedPopupMenuRow {
    fn from(record: TabListMenuRowPaintRecord<'_>) -> Self {
        Self {
            input_id: record.input_id(),
            visual_id: record.visual_id(),
            tab_visual_id: record.tab_visual_id(),
            key: record.key(),
            item: record.item(),
            label: record.label().into(),
            ordinal: record.ordinal(),
            bounds: record.bounds(),
            interaction_bounds: record.interaction_bounds(),
            selected: record.selected(),
            focused: record.focused(),
        }
    }
}

/// Owned open tab-list popup menu record.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedPopupMenu {
    /// Host input identity for the popup frame and scrollbar.
    pub input_id: DockspaceControlId,
    /// Host input identity of the opener button that opened this menu.
    pub opener_input_id: DockspaceControlId,
    /// Stable visual identity of this menu.
    pub visual_id: DockspaceVisualId,
    /// Owning tab bar visual identity.
    pub tab_bar_visual_id: DockspaceVisualId,
    /// Outer menu rectangle.
    pub bounds: LogicalRect,
    /// Clipped scrolling row viewport rectangle.
    pub viewport: LogicalRect,
    /// Current vertical scroll offset.
    pub scroll_offset: f64,
    /// Maximum vertical scroll offset.
    pub maximum_scroll_offset: f64,
    /// Ordered menu rows.
    pub rows: Vec<OwnedPopupMenuRow>,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Paired backdrop record.
    pub backdrop: OwnedPopupBackdrop,
    /// Core paint disposition.
    pub paint: SurfacePaintDisposition,
}

/// Owned future tab-bar geometry inside a drag preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedPreviewTabBar {
    /// Future tab-bar background bounds.
    pub bounds: LogicalRect,
    /// Clipped viewport rectangle.
    pub viewport: LogicalRect,
    /// Horizontal scroll offset applied to future tabs.
    pub scroll_offset: f64,
    /// Maximum horizontal scroll offset.
    pub maximum_scroll_offset: f64,
}

impl From<DockspacePreviewTabBarPaintRecord<'_>> for OwnedPreviewTabBar {
    fn from(record: DockspacePreviewTabBarPaintRecord<'_>) -> Self {
        Self {
            bounds: record.bounds(),
            viewport: record.viewport(),
            scroll_offset: record.scroll_offset(),
            maximum_scroll_offset: record.maximum_scroll_offset(),
        }
    }
}

/// Owned future tab member inside a drag preview.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedPreviewTab {
    /// Direct tab key.
    pub key: TabKey,
    /// Application item identifier, absent for layout tabs.
    pub item: Option<ItemId>,
    /// Frozen tab label.
    pub label: OwnedTabLabel,
    /// Index in the future strip.
    pub ordinal: usize,
    /// Whether this tab will be selected after drop.
    pub selected: bool,
    /// Whether this tab belongs to the dragged payload.
    pub is_payload: bool,
    /// Full unclipped tab bounds.
    pub full_bounds: LogicalRect,
    /// Target viewport clip bounds.
    pub clip_bounds: LogicalRect,
    /// Paintable clipped bounds, absent when scrolled out of view.
    pub visible_bounds: Option<LogicalRect>,
    /// Unclipped title layout bounds when visible.
    pub text_layout_bounds: Option<LogicalRect>,
    /// Clipped title bounds when visible.
    pub text_bounds: Option<LogicalRect>,
    /// Reserved close-button visual bounds when visible.
    pub close_visual_bounds: Option<LogicalRect>,
}

impl From<DockspacePreviewTabPaintRecord<'_>> for OwnedPreviewTab {
    fn from(record: DockspacePreviewTabPaintRecord<'_>) -> Self {
        Self {
            key: record.key(),
            item: record.item(),
            label: record.label().into(),
            ordinal: record.ordinal(),
            selected: record.selected(),
            is_payload: record.is_payload(),
            full_bounds: record.full_bounds(),
            clip_bounds: record.clip_bounds(),
            visible_bounds: record.visible_bounds(),
            text_layout_bounds: record.text_layout_bounds(),
            text_bounds: record.text_bounds(),
            close_visual_bounds: record.close_visual_bounds(),
        }
    }
}

/// Owned active docking/rehome drag preview.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedDragPreview {
    /// Projected preview geometry.
    pub visual: DockspacePreviewVisual,
    /// Whether the complete target layout is already projected in the scene.
    pub is_layout_projected: bool,
    /// Resolved target feedback, absent for floating/native previews.
    pub target: Option<DockspaceDropTargetFeedback>,
    /// Projected target tab bar for center-merge or tab-gap insertions.
    pub tab_bar: Option<OwnedPreviewTabBar>,
    /// Projected future tabs in candidate order.
    pub tabs: Vec<OwnedPreviewTab>,
}

impl From<DockspaceDragPreview<'_>> for OwnedDragPreview {
    fn from(preview: DockspaceDragPreview<'_>) -> Self {
        Self {
            visual: preview.visual(),
            is_layout_projected: preview.is_layout_projected(),
            target: preview.target(),
            tab_bar: preview.tab_bar().map(OwnedPreviewTabBar::from),
            tabs: preview.tabs().map(OwnedPreviewTab::from).collect(),
        }
    }
}

/// Owned static docking-guide button geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedDropGuideTarget {
    /// Opaque visual identity.
    pub visual_id: DockspaceVisualId,
    /// Center or edge direction represented by this button.
    pub direction: DockspaceDropDirection,
    /// Visible button rectangle.
    pub draw_bounds: LogicalRect,
    /// Pointer hit rectangle.
    pub hit_bounds: LogicalRect,
    /// Static preview rectangle for this target.
    pub preview_bounds: LogicalRect,
}

impl From<DropGuideTargetPaintRecord<'_>> for OwnedDropGuideTarget {
    fn from(record: DropGuideTargetPaintRecord<'_>) -> Self {
        Self {
            visual_id: record.visual_id(),
            direction: record.direction(),
            draw_bounds: record.draw_bounds(),
            hit_bounds: record.hit_bounds(),
            preview_bounds: record.preview_bounds(),
        }
    }
}

/// Owned live drag affordance button state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedDropAffordanceTarget {
    /// Opaque visual identity.
    pub visual_id: DockspaceVisualId,
    /// Center or edge direction represented by this button.
    pub direction: DockspaceDropDirection,
    /// Visible button rectangle.
    pub draw_bounds: LogicalRect,
    /// Whether this button currently wins the authoritative hit query.
    pub is_active: bool,
    /// Payload-specific drop eligibility.
    pub eligibility: DockspaceDropEligibility,
}

impl From<DropAffordanceTargetPaintRecord<'_>> for OwnedDropAffordanceTarget {
    fn from(record: DropAffordanceTargetPaintRecord<'_>) -> Self {
        Self {
            visual_id: record.visual_id(),
            direction: record.direction(),
            draw_bounds: record.draw_bounds(),
            is_active: record.is_active(),
            eligibility: record.eligibility(),
        }
    }
}

/// Owned live drag affordance cluster state.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedDropAffordanceCluster {
    /// Opaque cluster visual identity.
    pub visual_id: DockspaceVisualId,
    /// Owning root, absent for empty-surface guides.
    pub root: Option<RootId>,
    /// Surface, inner leaf, or outer root scope.
    pub scope: DockspaceGuideScope,
    /// Region that activated this cluster.
    pub activation_bounds: LogicalRect,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Payload-evaluated buttons in paint order.
    pub targets: Vec<OwnedDropAffordanceTarget>,
}

impl From<DropAffordanceClusterPaintRecord<'_>> for OwnedDropAffordanceCluster {
    fn from(record: DropAffordanceClusterPaintRecord<'_>) -> Self {
        Self {
            visual_id: record.visual_id(),
            root: record.root(),
            scope: record.scope(),
            activation_bounds: record.activation_bounds(),
            layer: record.layer(),
            targets: record
                .targets()
                .map(OwnedDropAffordanceTarget::from)
                .collect(),
        }
    }
}

/// Owned docking-guide cluster and optional live drag affordance.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedDropGuide {
    /// Opaque cluster visual identity.
    pub visual_id: DockspaceVisualId,
    /// Owning root, absent for empty-surface guides.
    pub root: Option<RootId>,
    /// Surface, inner leaf, or outer root scope.
    pub scope: DockspaceGuideScope,
    /// Activation region.
    pub activation_bounds: LogicalRect,
    /// Semantic paint layer.
    pub layer: DockspacePaintLayer,
    /// Structural target buttons in paint order.
    pub targets: Vec<OwnedDropGuideTarget>,
    /// Live drag affordance when a drag currently activates this cluster.
    pub affordance: Option<OwnedDropAffordanceCluster>,
}

/// Owned contained-floating transform preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnedContainedTransformPreview {
    /// Logical surface owning the preview.
    pub surface: SurfaceId,
    /// Contained presentation being transformed.
    pub floating: FloatingPresentationId,
    /// Preview bounds.
    pub rect: LogicalRect,
    /// Whether the complete root is already projected at `rect`.
    pub is_layout_projected: bool,
}

impl From<DockspaceContainedTransformPreview<'_>> for OwnedContainedTransformPreview {
    fn from(preview: DockspaceContainedTransformPreview<'_>) -> Self {
        Self {
            surface: preview.surface(),
            floating: preview.floating(),
            rect: preview.rect(),
            is_layout_projected: preview.is_layout_projected(),
        }
    }
}

/// Owned active drag source ghost decoration.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedDragDecoration {
    /// Logical surface still presenting the drag source.
    pub source_surface: SurfaceId,
    /// Semantic kind of the dragged payload.
    pub kind: DockspaceDragSourceKind,
    /// Visual identity of the dragged source.
    pub source_visual: DockspaceVisualId,
    /// Original source bounds on `source_surface`.
    pub source_bounds: LogicalRect,
    /// Projected full-window bounds for a contained root drag, if any.
    pub projected_root_bounds: Option<LogicalRect>,
    /// Whether the complete source is already painted at its projection.
    pub source_is_projected: bool,
    /// Dragged item when dragging a single tab.
    pub item: Option<ItemId>,
    /// Dragged tab key for resource lookup.
    pub key: TabKey,
    /// Frozen label of the dragged payload.
    pub label: OwnedTabLabel,
}

impl From<DockspaceDragDecoration<'_>> for OwnedDragDecoration {
    fn from(decoration: DockspaceDragDecoration<'_>) -> Self {
        Self {
            source_surface: decoration.source_surface(),
            kind: decoration.kind(),
            source_visual: decoration.source_visual(),
            source_bounds: decoration.source_bounds(),
            projected_root_bounds: decoration.projected_root_bounds(),
            source_is_projected: decoration.source_is_projected(),
            item: decoration.item(),
            key: decoration.key(),
            label: decoration.label().into(),
        }
    }
}

/// One owned step in canonical core composition order inside [`SurfaceDisplayList`].
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceDisplayStep {
    /// Starts a main or contained-floating root segment.
    RootBegin {
        /// Root header metadata.
        root: OwnedSurfaceRoot,
        /// Paint disposition for contained frame chrome.
        paint: SurfacePaintDisposition,
    },
    /// Application pane content placement.
    Pane(OwnedPane),
    /// Independent Tabs container background.
    Tabs(OwnedTabs),
    /// Tab strip header bar.
    TabBar(OwnedTabBar),
    /// Visible tab header.
    Tab(OwnedTab),
    /// Resizable split or grid divider.
    Splitter(OwnedSplitter),
    /// Atomic multi-axis splitter junction.
    SplitterJunction(OwnedSplitterJunction),
    /// Core-owned tab-strip overflow/menu control.
    TabStripControl(OwnedTabStripControl),
    /// Title bar and resize borders of a contained-floating root.
    ContainedControls {
        /// Contained frame geometry and controls.
        frame: OwnedContainedFrame,
        /// Core paint disposition.
        paint: SurfacePaintDisposition,
    },
    /// Root-scoped presentation command menu anchor.
    PresentationMenuAnchor(OwnedPresentationMenuAnchor),
    /// Ends the active root segment.
    RootEnd(OwnedSurfaceRoot),
    /// Workspace-global popup backdrop.
    PopupBackdrop(OwnedPopupBackdrop),
    /// Open tab-list popup menu.
    PopupMenu(OwnedPopupMenu),
    /// Active docking or rehome drag preview.
    DragPreview(OwnedDragPreview),
    /// Non-visual drop target receiver in core hit order.
    DropTarget(DockspaceReceiverDescriptor),
    /// Docking-guide cluster and optional live affordance.
    DropGuide(OwnedDropGuide),
    /// Active contained-floating resize/move preview.
    ContainedTransformPreview(OwnedContainedTransformPreview),
    /// Active drag source ghost decoration.
    DragDecoration(OwnedDragDecoration),
    /// Interaction feedback checkpoint after popup steps.
    InteractionFeedback,
}

/// Complete owned, `'static` display list compiled from one [`SurfacePaintPlan`].
///
/// Retained UIs and deferred GPU render threads can store this value directly
/// after confirming and committing the host frame, then traverse [`Self::steps`]
/// in canonical back-to-front composition order.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceDisplayList {
    surface: SurfaceId,
    semantic_output: DockspaceSemanticOutput,
    steps: Vec<SurfaceDisplayStep>,
}

impl SurfaceDisplayList {
    /// Returns the logical surface represented by this display list.
    #[must_use]
    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// Returns the exact semantic output identity sealed with this display list.
    #[must_use]
    pub const fn semantic_output(&self) -> DockspaceSemanticOutput {
        self.semantic_output
    }

    /// Returns all paint steps in exact core composition order.
    #[must_use]
    pub fn steps(&self) -> &[SurfaceDisplayStep] {
        &self.steps
    }

    /// Iterates every active application pane in paint order.
    pub fn panes(&self) -> impl Iterator<Item = &OwnedPane> {
        self.steps.iter().filter_map(|step| match step {
            SurfaceDisplayStep::Pane(pane) => Some(pane),
            _ => None,
        })
    }

    /// Iterates every tab bar in paint order.
    pub fn tab_bars(&self) -> impl Iterator<Item = &OwnedTabBar> {
        self.steps.iter().filter_map(|step| match step {
            SurfaceDisplayStep::TabBar(bar) => Some(bar),
            _ => None,
        })
    }

    /// Iterates every visible tab header in paint order.
    pub fn tabs(&self) -> impl Iterator<Item = &OwnedTab> {
        self.steps.iter().filter_map(|step| match step {
            SurfaceDisplayStep::Tab(tab) => Some(tab),
            _ => None,
        })
    }

    /// Iterates every resizable splitter in paint order.
    pub fn splitters(&self) -> impl Iterator<Item = &OwnedSplitter> {
        self.steps.iter().filter_map(|step| match step {
            SurfaceDisplayStep::Splitter(splitter) => Some(splitter),
            _ => None,
        })
    }

    /// Iterates every non-visual drop target receiver emitted by this program.
    pub fn drop_targets(&self) -> impl Iterator<Item = DockspaceReceiverDescriptor> + '_ {
        self.steps.iter().filter_map(|step| match step {
            SurfaceDisplayStep::DropTarget(receiver) => Some(*receiver),
            _ => None,
        })
    }
}

struct DisplayListCollector {
    surface: SurfaceId,
    semantic_output: Option<DockspaceSemanticOutput>,
    steps: Vec<SurfaceDisplayStep>,
}

impl SurfaceProgramVisitor for DisplayListCollector {
    type Error = Infallible;

    fn root_begin(
        &mut self,
        root: SurfacePaintProgramRoot<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::RootBegin {
            root: root.into(),
            paint,
        });
        Ok(())
    }

    fn pane(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        pane: PanePaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::Pane(OwnedPane {
            visual_id: pane.visual_id(),
            tab_bar_visual_id: pane.tab_bar_visual_id(),
            root: pane.root(),
            item: pane.item(),
            content_geometry: pane.content_geometry(),
            layer: pane.layer(),
            paint,
        }));
        Ok(())
    }

    fn tabs(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        tabs: TabsPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::Tabs(OwnedTabs {
            visual_id: tabs.visual_id(),
            root: tabs.root(),
            bounds: tabs.bounds(),
            content_bounds: tabs.content_bounds(),
            clip: tabs.clip(),
            layer: tabs.layer(),
            paint,
        }));
        Ok(())
    }

    fn tab_bar(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        bar: TabBarPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::TabBar(OwnedTabBar {
            visual_id: bar.visual_id(),
            group_menu_input_id: bar.group_menu_input_id(),
            tabs_visual_id: bar.tabs_visual_id(),
            root: bar.root(),
            selected: bar.selected(),
            bounds: bar.bounds(),
            viewport: bar.viewport(),
            scroll_offset: bar.scroll_offset(),
            maximum_scroll_offset: bar.maximum_scroll_offset(),
            hidden_tabs: bar.hidden_tabs().to_vec(),
            members: bar.members().map(OwnedTabStripMember::from).collect(),
            group_grip_bounds: bar.group_grip_bounds(),
            group_menu_bounds: bar.group_menu_bounds(),
            leading_reserved_bounds: bar.leading_reserved_bounds(),
            trailing_reserved_bounds: bar.trailing_reserved_bounds(),
            after_tabs_bounds: bar.after_tabs_bounds(),
            group_drag_regions: bar
                .group_drag_regions()
                .map(OwnedTabGroupDragRegion::from)
                .collect(),
            interaction: bar.interaction(),
            layer: bar.layer(),
            paint,
        }));
        Ok(())
    }

    fn tab(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        tab: TabPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::Tab(OwnedTab {
            visual_id: tab.visual_id(),
            input_id: tab.input_id(),
            close_input_id: tab.close_input_id(),
            tab_bar_visual_id: tab.tab_bar_visual_id(),
            root: tab.root(),
            key: tab.key(),
            item: tab.item(),
            label: tab.label().into(),
            full_bounds: tab.full_bounds(),
            visible_bounds: tab.visible_bounds(),
            text_bounds: tab.text_bounds(),
            text_layout_bounds: tab.text_layout_bounds(),
            drag_bounds: tab.drag_bounds(),
            close_visual_bounds: tab.close_visual_bounds(),
            close_bounds: tab.close_bounds(),
            selected: tab.selected(),
            operable: tab.operable(),
            ordinal: tab.ordinal(),
            layer: tab.layer(),
            paint,
        }));
        Ok(())
    }

    fn splitter(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        splitter: SplitterPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::Splitter(OwnedSplitter {
            visual_id: splitter.visual_id(),
            input_id: splitter.input_id(),
            root: splitter.root(),
            draw_bounds: splitter.draw_bounds(),
            hit_bounds: splitter.hit_bounds(),
            axis: splitter.axis(),
            before_bounds: splitter.before_bounds(),
            after_bounds: splitter.after_bounds(),
            minimum_extents: splitter.minimum_extents(),
            maximum_extents: splitter.maximum_extents(),
            operable: splitter.operable(),
            layer: splitter.layer(),
            paint,
        }));
        Ok(())
    }

    fn splitter_junction(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        junction: SplitterJunctionPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::SplitterJunction(
            OwnedSplitterJunction {
                visual_id: junction.visual_id(),
                input_id: junction.input_id(),
                root: junction.root(),
                hit_bounds: junction.hit_bounds(),
                arm_count: junction.arm_count(),
                has_horizontal: junction.has_axis(DockspaceAxis::Horizontal),
                has_vertical: junction.has_axis(DockspaceAxis::Vertical),
                layer: junction.layer(),
                paint,
            },
        ));
        Ok(())
    }

    fn tab_strip_control(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        control: TabStripControlPaintRecord,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::TabStripControl(OwnedTabStripControl {
                visual_id: control.visual_id(),
                input_id: control.input_id(),
                tab_bar_visual_id: control.tab_bar_visual_id(),
                root: control.root(),
                kind: control.kind(),
                bounds: control.bounds(),
                hit_bounds: control.hit_bounds(),
                enabled: control.enabled(),
                menu_open: control.menu_open(),
                layer: control.layer(),
                paint,
            }));
        Ok(())
    }

    fn contained_controls(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        contained: ContainedPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::ContainedControls {
            frame: contained.into(),
            paint,
        });
        Ok(())
    }

    fn presentation_menu_anchor(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        anchor: PresentationMenuAnchorPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::PresentationMenuAnchor(
            OwnedPresentationMenuAnchor {
                visual_id: anchor.visual_id(),
                input_id: anchor.input_id(),
                root: anchor.root(),
                kind: anchor.kind(),
                host_visual_id: anchor.host_visual_id(),
                bounds: anchor.bounds(),
                reserved_bounds: anchor.reserved_bounds(),
                layer: anchor.layer(),
                operable: anchor.operable(),
                paint,
            },
        ));
        Ok(())
    }

    fn root_end(&mut self, root: SurfacePaintProgramRoot<'_>) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::RootEnd(root.into()));
        Ok(())
    }

    fn popup_backdrop(
        &mut self,
        backdrop: TabListMenuBackdropPaintRecord,
    ) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::PopupBackdrop(backdrop.into()));
        Ok(())
    }

    fn popup_menu(
        &mut self,
        menu: TabListMenuPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::PopupMenu(OwnedPopupMenu {
                input_id: menu.input_id(),
                opener_input_id: menu.opener_input_id(),
                visual_id: menu.visual_id(),
                tab_bar_visual_id: menu.tab_bar_visual_id(),
                bounds: menu.bounds(),
                viewport: menu.viewport(),
                scroll_offset: menu.scroll_offset(),
                maximum_scroll_offset: menu.maximum_scroll_offset(),
                rows: menu.rows().map(OwnedPopupMenuRow::from).collect(),
                layer: menu.layer(),
                backdrop: menu.backdrop().into(),
                paint,
            }));
        Ok(())
    }

    fn drag_preview(&mut self, preview: DockspaceDragPreview<'_>) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::DragPreview(preview.into()));
        Ok(())
    }

    fn drop_target(&mut self, target: DropTargetPaintRecord<'_>) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::DropTarget(target.receiver()));
        Ok(())
    }

    fn drop_guide(
        &mut self,
        structural: DropGuidePaintRecord<'_>,
        affordance: Option<DropAffordanceClusterPaintRecord<'_>>,
    ) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::DropGuide(OwnedDropGuide {
                visual_id: structural.visual_id(),
                root: structural.root(),
                scope: structural.scope(),
                activation_bounds: structural.activation_bounds(),
                layer: structural.layer(),
                targets: structural
                    .targets()
                    .map(OwnedDropGuideTarget::from)
                    .collect(),
                affordance: affordance.map(OwnedDropAffordanceCluster::from),
            }));
        Ok(())
    }

    fn contained_transform_preview(
        &mut self,
        preview: DockspaceContainedTransformPreview<'_>,
    ) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::ContainedTransformPreview(
                preview.into(),
            ));
        Ok(())
    }

    fn drag_decoration(
        &mut self,
        decoration: DockspaceDragDecoration<'_>,
    ) -> Result<(), Self::Error> {
        self.steps
            .push(SurfaceDisplayStep::DragDecoration(decoration.into()));
        Ok(())
    }

    fn interaction_feedback(&mut self) -> Result<(), Self::Error> {
        self.steps.push(SurfaceDisplayStep::InteractionFeedback);
        Ok(())
    }

    fn surface_end(&mut self, semantic_output: DockspaceSemanticOutput) -> Result<(), Self::Error> {
        debug_assert_eq!(semantic_output.surface(), self.surface);
        self.semantic_output = Some(semantic_output);
        Ok(())
    }
}

/// Inspects a paint plan through a typed [`SurfaceProgramVisitor`] without minting a receipt.
///
/// # Errors
///
/// Returns [`SurfacePaintProgramVisitError`] if the sealed program fails validation
/// or the visitor returns an error.
pub fn visit_with_visitor<V: SurfaceProgramVisitor>(
    plan: SurfacePaintPlan<'_>,
    visitor: &mut V,
) -> Result<(), SurfacePaintProgramVisitError<V::Error>> {
    plan.visit_surface_program(|step| dispatch_step(visitor, step))
}

/// Executes a paint plan through a typed [`SurfaceProgramVisitor`] and returns
/// its affine [`SurfaceProgramExecutionReceipt`].
///
/// # Errors
///
/// Returns [`SurfacePaintProgramVisitError`] if the sealed program fails validation
/// or the visitor returns an error.
pub fn execute_with_visitor<V: SurfaceProgramVisitor>(
    plan: SurfacePaintPlan<'_>,
    visitor: &mut V,
) -> Result<SurfaceProgramExecutionReceipt, SurfacePaintProgramVisitError<V::Error>> {
    plan.execute_surface_program(|step| dispatch_step(visitor, step))
}

/// Compiles a [`SurfacePaintPlan`] into an owned `'static` [`SurfaceDisplayList`]
/// and its matching affine [`SurfaceProgramExecutionReceipt`].
///
/// Pass the returned receipt to [`crate::runtime::DockspaceHostFrame::confirm_surface_painted`]
/// before committing the frame, then report actual presentation through
/// [`crate::runtime::DockspaceSession::report_surface_presentation`].
///
/// # Errors
///
/// Returns an internal runtime error if the sealed surface program fails validation.
pub fn compile_display_list(
    plan: SurfacePaintPlan<'_>,
) -> Result<(SurfaceDisplayList, SurfaceProgramExecutionReceipt), DockspaceRuntimeError> {
    let surface = plan.surface();
    let fallback_output = plan.semantic_output();
    let mut collector = DisplayListCollector {
        surface,
        semantic_output: None,
        steps: Vec::new(),
    };
    let receipt = execute_with_visitor(plan, &mut collector).map_err(|error| match error {
        SurfacePaintProgramVisitError::InvalidProgram { surface } => {
            DockspaceRuntimeError::surface_program_invariant(surface)
        }
        SurfacePaintProgramVisitError::Sink(never) => match never {},
    })?;
    let list = SurfaceDisplayList {
        surface,
        semantic_output: collector.semantic_output.unwrap_or(fallback_output),
        steps: collector.steps,
    };
    Ok((list, receipt))
}

impl SurfacePaintPlan<'_> {
    /// Inspects this paint plan through a typed [`SurfaceProgramVisitor`] without
    /// minting a paint receipt.
    ///
    /// # Errors
    ///
    /// Returns [`SurfacePaintProgramVisitError`] on program validation or visitor failure.
    pub fn visit_visitor<V: SurfaceProgramVisitor>(
        self,
        visitor: &mut V,
    ) -> Result<(), SurfacePaintProgramVisitError<V::Error>> {
        visit_with_visitor(self, visitor)
    }

    /// Executes this paint plan through a typed [`SurfaceProgramVisitor`] and
    /// returns the affine receipt required by
    /// [`crate::runtime::DockspaceHostFrame::confirm_surface_painted`].
    ///
    /// # Errors
    ///
    /// Returns [`SurfacePaintProgramVisitError`] on program validation or visitor failure.
    pub fn execute_visitor<V: SurfaceProgramVisitor>(
        self,
        visitor: &mut V,
    ) -> Result<SurfaceProgramExecutionReceipt, SurfacePaintProgramVisitError<V::Error>> {
        execute_with_visitor(self, visitor)
    }

    /// Compiles this paint plan into an owned `'static` [`SurfaceDisplayList`]
    /// and its matching affine [`SurfaceProgramExecutionReceipt`].
    ///
    /// # Errors
    ///
    /// Returns an internal runtime error if the sealed surface program fails validation.
    pub fn compile_display_list(
        self,
    ) -> Result<(SurfaceDisplayList, SurfaceProgramExecutionReceipt), DockspaceRuntimeError> {
        compile_display_list(self)
    }
}
