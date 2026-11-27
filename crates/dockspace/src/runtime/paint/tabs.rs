//! Pane, tab, and tab-bar paint records.

use crate::geometry::LogicalRect;
use crate::ids::{ItemId, RootId, TabKey, WorkspaceRevision};
use crate::policy::TabBarInteraction;
use crate::presentation::scene::{
    PaneRecord, PresentationPlan, TabBarRecord, TabBarSceneId, TabGroupDragRegionKind,
    TabGroupDragRegionRecord, TabRecord, TabStripMemberRecord,
    TabStripMemberVisibility as CoreTabStripMemberVisibility, TabsRecord,
};

use super::{
    ControlScope, ControlTarget, DockspaceControlId, DockspacePaintLayer, DockspaceSemanticOutput,
    DockspaceVisualId, SurfacePaintPlan, VisualIdentity,
};

/// The allocation and clip for mounting application content in a pane.
///
/// Both rectangles use surface-local logical coordinates. Moving a pane
/// across the edge of its surface can shrink its clip without resizing it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneContentGeometry {
    layout_bounds: LogicalRect,
    clip: LogicalRect,
}

impl PaneContentGeometry {
    /// The full allocation for sizing and positioning the content.
    #[must_use]
    pub const fn layout_bounds(self) -> LogicalRect {
        self.layout_bounds
    }

    /// The visible content region used to clip painting and pointer input.
    #[must_use]
    pub const fn clip(self) -> LogicalRect {
        self.clip
    }
}

/// Read-only geometry and identity for one real application Pane.
#[derive(Debug, Clone, Copy)]
pub struct PanePaintRecord<'plan> {
    pub(super) record: &'plan PaneRecord,
    pub(super) output: DockspaceSemanticOutput,
}

impl PanePaintRecord<'_> {
    /// Returns this pane's stable visual identity.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Pane(self.record.id()))
    }

    /// Returns the nearest active containing tab strip, when present.
    #[must_use]
    pub const fn tab_bar_visual_id(self) -> Option<DockspaceVisualId> {
        match self.record.containing_tab_bar() {
            Some(bar) => Some(DockspaceVisualId(VisualIdentity::TabBar(bar))),
            None => None,
        }
    }

    /// Returns the stable root owning this pane.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    /// Mount content in the full allocation and apply its clip separately.
    #[must_use]
    pub const fn content_geometry(self) -> PaneContentGeometry {
        PaneContentGeometry {
            layout_bounds: self.record.layout_bounds(),
            clip: self.record.clip(),
        }
    }

    /// Returns the stable application item rendered inside this pane.
    #[must_use]
    pub const fn item(self) -> ItemId {
        self.record.item()
    }

    /// Returns the semantic paint layer for this pane.
    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Independent background of one active Tabs container.
#[derive(Debug, Clone, Copy)]
pub struct TabsPaintRecord<'plan> {
    pub(super) record: &'plan TabsRecord,
    pub(super) output: DockspaceSemanticOutput,
}
impl TabsPaintRecord<'_> {
    /// Returns this container's stable visual identity.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tabs(self.record.id()))
    }
    /// Returns the complete container frame.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }
    /// Returns the remaining child slot below this container's header.
    #[must_use]
    pub const fn content_bounds(self) -> LogicalRect {
        self.record.content_bounds()
    }
    /// Returns the unclipped layout allocation and visible clip for this container's content slot.
    #[must_use]
    pub const fn content_geometry(self) -> PaneContentGeometry {
        PaneContentGeometry {
            layout_bounds: self.record.content_layout_bounds(),
            clip: self.record.content_bounds(),
        }
    }
    /// Returns whether this container is a root's empty central area with no visible tabs.
    #[must_use]
    pub const fn is_empty_central(self) -> bool {
        self.record.is_empty_central()
    }
    /// Returns the exact container clip.
    #[must_use]
    pub const fn clip(self) -> LogicalRect {
        self.record.clip()
    }
    /// Returns the owning root.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }
    /// Returns the semantic paint layer.
    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Read-only visible-tab geometry and state.
#[derive(Debug, Clone, Copy)]
pub struct TabPaintRecord<'plan> {
    pub(super) record: &'plan TabRecord,
    plan: &'plan PresentationPlan,
    output: DockspaceSemanticOutput,
    revision: WorkspaceRevision,
    selected: bool,
    operable: bool,
}

impl<'plan> TabPaintRecord<'plan> {
    pub(super) fn new(plan: &SurfacePaintPlan<'plan>, record: &'plan TabRecord) -> Self {
        Self {
            record,
            plan: plan.plan,
            output: plan.semantic_output(),
            revision: plan.version.revision(),
            selected: plan.plan.tab_selected(record),
            operable: plan.tab_is_operable(record),
        }
    }

    pub(super) fn belongs_to(self, plan: &SurfacePaintPlan<'_>) -> bool {
        self.output == plan.semantic_output()
    }

    /// Returns the host input identity for this tab header.
    #[must_use]
    pub const fn input_id(self) -> DockspaceControlId {
        self.output.control_id(self.visual_id())
    }

    /// Returns the host input identity for this tab's close button.
    ///
    /// A layout tab closes a subtree. Its close identity expires when the
    /// workspace revision changes, while a pane tab keeps targeting one item.
    #[must_use]
    pub const fn close_input_id(self) -> DockspaceControlId {
        let revision = if self.key().is_layout() {
            Some(self.revision)
        } else {
            None
        };
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::TabClose(*self.record.id(), revision),
        )
    }

    /// Returns this tab's stable visual identity.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tab(*self.record.id()))
    }

    /// Returns the tab strip which owns this tab.
    #[must_use]
    pub const fn tab_bar_visual_id(self) -> DockspaceVisualId {
        let tab = self.record.id();
        DockspaceVisualId(VisualIdentity::TabBar(TabBarSceneId {
            root: tab.root,
            tabs: tab.tabs,
        }))
    }

    /// Returns the stable root owning this tab.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    /// Returns the direct tab key identifying this tab child.
    #[must_use]
    pub const fn key(self) -> TabKey {
        self.record.id().tab
    }

    /// Returns the pane item when this is a pane tab, or `None` for a layout tab.
    #[must_use]
    pub const fn item(self) -> Option<ItemId> {
        self.key().item()
    }

    /// Returns the exact label frozen with this output.
    #[must_use]
    pub fn label(self) -> crate::graph::TabLabel<'plan> {
        self.plan
            .tab_label(*self.record.id())
            .expect("compiled tabs retain their exact labels")
    }

    /// Returns the unclipped tab header rectangle before strip scrolling clips it.
    #[must_use]
    pub const fn full_bounds(self) -> LogicalRect {
        self.record.full_bounds()
    }

    /// Returns the visible portion of this tab clipped to the strip viewport.
    #[must_use]
    pub const fn visible_bounds(self) -> LogicalRect {
        self.record.visible_bounds()
    }

    /// Returns the visible clipped rectangle for the tab's title text.
    #[must_use]
    pub const fn text_bounds(self) -> LogicalRect {
        self.record.text_bounds()
    }

    /// Returns the allocation used to shape and position text before scrolling clips it.
    #[must_use]
    pub const fn text_layout_bounds(self) -> LogicalRect {
        self.record.text_layout_bounds()
    }

    /// Returns the logical hit-testing rectangle for selecting or dragging this tab.
    #[must_use]
    pub const fn drag_bounds(self) -> LogicalRect {
        self.record.drag_hit().rect()
    }

    /// Returns the visual rectangle for drawing this tab's close icon, when present.
    #[must_use]
    pub const fn close_visual_bounds(self) -> Option<LogicalRect> {
        self.record.close_visual_bounds()
    }

    /// Returns the interactive hit-testing rectangle for this tab's close control, when present.
    #[must_use]
    pub const fn close_bounds(self) -> Option<LogicalRect> {
        self.record.close_bounds()
    }

    /// Returns whether this tab is the active selection in its tab group.
    #[must_use]
    pub const fn selected(self) -> bool {
        self.selected
    }

    /// Returns whether the exact tab body may publish semantic actions.
    #[must_use]
    pub const fn operable(self) -> bool {
        self.operable
    }

    /// Returns the zero-based visual position of this tab in its strip.
    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.record.ordinal()
    }

    /// Returns the semantic paint layer for this tab.
    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Product-facing classification of one tab relative to its clipped strip viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TabStripMemberVisibility {
    /// The complete allocated tab rectangle is visible.
    Visible,
    /// A positive but incomplete portion intersects the viewport.
    PartiallyVisible,
    /// No positive portion intersects the viewport.
    Hidden,
}

/// Read-only member of one tab strip, including clipped visibility.
#[derive(Debug, Clone, Copy)]
pub struct TabStripMemberPaintRecord<'plan> {
    pub(super) plan: &'plan PresentationPlan,
    pub(super) record: TabStripMemberRecord,
}

impl<'plan> TabStripMemberPaintRecord<'plan> {
    /// Returns this member tab's stable visual identity.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tab(self.record.tab()))
    }

    /// Returns the visual identity of the tab strip containing this member.
    #[must_use]
    pub const fn tab_bar_visual_id(self) -> DockspaceVisualId {
        let tab = self.record.tab();
        DockspaceVisualId(VisualIdentity::TabBar(TabBarSceneId {
            root: tab.root,
            tabs: tab.tabs,
        }))
    }

    /// Returns the direct tab key identifying this member.
    #[must_use]
    pub const fn key(self) -> TabKey {
        self.record.tab().tab
    }

    /// Returns the pane item when this member is a pane tab, or `None` for a layout tab.
    #[must_use]
    pub const fn item(self) -> Option<ItemId> {
        self.key().item()
    }

    /// Returns the exact member label frozen with this output.
    #[must_use]
    pub fn label(self) -> crate::graph::TabLabel<'plan> {
        self.plan
            .tab_label(self.record.tab())
            .expect("compiled members retain exact labels")
    }

    /// Returns the zero-based visual position of this member in its strip.
    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.record.ordinal()
    }

    /// Returns the unclipped tab header rectangle before strip scrolling clips it.
    #[must_use]
    pub const fn full_bounds(self) -> LogicalRect {
        self.record.full_bounds()
    }

    /// Returns how much of this member intersects the clipped tab-strip viewport.
    #[must_use]
    pub const fn visibility(self) -> TabStripMemberVisibility {
        match self.record.visibility() {
            CoreTabStripMemberVisibility::Visible => TabStripMemberVisibility::Visible,
            CoreTabStripMemberVisibility::PartiallyVisible => {
                TabStripMemberVisibility::PartiallyVisible
            }
            CoreTabStripMemberVisibility::Hidden => TabStripMemberVisibility::Hidden,
        }
    }
}

/// Read-only tab-bar geometry and overflow state.
#[derive(Debug, Clone, Copy)]
pub struct TabBarPaintRecord<'plan> {
    pub(super) record: &'plan TabBarRecord,
    plan: &'plan PresentationPlan,
    output: DockspaceSemanticOutput,
    selected: Option<TabKey>,
}

/// Read-only geometry and identity for one whole-group drag region.
#[derive(Debug, Clone, Copy)]
pub struct TabGroupDragRegionPaintRecord {
    pub(super) bar: TabBarSceneId,
    pub(super) record: TabGroupDragRegionRecord,
    output: DockspaceSemanticOutput,
}

impl TabGroupDragRegionPaintRecord {
    pub(super) fn belongs_to(self, plan: &SurfacePaintPlan<'_>) -> bool {
        self.output == plan.semantic_output()
    }

    /// Returns a stable identity distinct from every other region of this bar.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabGroupDragRegion {
            bar: self.bar,
            region: self.record.kind(),
        })
    }

    /// Returns whether this is the leading grip or trailing empty region.
    #[must_use]
    pub const fn kind(self) -> TabGroupDragRegionKind {
        self.record.kind()
    }

    /// Returns the exact core-owned draw and response bounds.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }
}

impl<'plan> TabBarPaintRecord<'plan> {
    pub(super) fn new(plan: &SurfacePaintPlan<'plan>, record: &'plan TabBarRecord) -> Self {
        Self {
            record,
            plan: plan.plan,
            output: plan.semantic_output(),
            selected: plan.plan.selected_in_tabs(record.id().tabs),
        }
    }

    pub(super) fn belongs_to(self, plan: &SurfacePaintPlan<'_>) -> bool {
        self.output == plan.semantic_output()
    }

    /// Returns this tab strip's stable visual identity.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabBar(*self.record.id()))
    }

    /// Returns the host input identity for this group's command-menu opener.
    #[must_use]
    pub const fn group_menu_input_id(self) -> DockspaceControlId {
        DockspaceControlId(
            ControlScope::from_scene(self.output.ticket.scene_stamp()),
            ControlTarget::TabGroupMenu(*self.record.id()),
        )
    }

    /// Returns the independent Tabs container represented by this strip.
    #[must_use]
    pub const fn tabs_visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tabs(*self.record.id()))
    }

    /// Returns the stable root owning this tab strip.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    /// Returns the group's selected direct child, including a fully clipped tab.
    #[must_use]
    pub const fn selected(self) -> Option<TabKey> {
        self.selected
    }

    /// Returns the full logical rectangle of this tab strip.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }

    /// Returns the inner scrollable viewport rectangle showing tabs.
    #[must_use]
    pub const fn viewport(self) -> LogicalRect {
        self.record.viewport()
    }

    /// Returns the current horizontal scroll offset in logical points.
    #[must_use]
    pub const fn scroll_offset(self) -> f64 {
        self.record.scroll_offset()
    }

    /// Returns the maximum horizontal scroll offset in logical points.
    #[must_use]
    pub const fn maximum_scroll_offset(self) -> f64 {
        self.record.maximum_scroll_offset()
    }

    /// Returns tab keys that are completely outside the current scroll viewport.
    #[must_use]
    pub fn hidden_tabs(self) -> &'plan [TabKey] {
        self.record.hidden_tabs()
    }

    /// Iterates every direct tab member in visual order, including clipped members.
    pub fn members(
        self,
    ) -> impl ExactSizeIterator<Item = TabStripMemberPaintRecord<'plan>> + 'plan {
        self.record
            .members()
            .iter()
            .copied()
            .map(move |record| TabStripMemberPaintRecord {
                record,
                plan: self.plan,
            })
    }

    /// Returns the dedicated leading whole-group drag grip rectangle, when shown.
    #[must_use]
    pub const fn group_grip_bounds(self) -> Option<LogicalRect> {
        self.record.group_grip_bounds()
    }

    /// Returns the exact group-action anchor reserved outside tab and drag regions.
    #[must_use]
    pub const fn group_menu_bounds(self) -> Option<LogicalRect> {
        self.record.group_menu_bounds()
    }

    /// Returns the core-clipped leading application chrome allocation.
    #[must_use]
    pub const fn leading_reserved_bounds(self) -> Option<LogicalRect> {
        self.record.leading_reserved_bounds()
    }

    /// Returns the core-clipped trailing application chrome allocation.
    #[must_use]
    pub const fn trailing_reserved_bounds(self) -> Option<LogicalRect> {
        self.record.trailing_reserved_bounds()
    }

    /// Returns application chrome scrolling immediately after the last tab.
    /// Clip painting and widget hits to [`Self::viewport`].
    #[must_use]
    pub const fn after_tabs_bounds(self) -> Option<LogicalRect> {
        self.record.after_tabs_bounds()
    }

    /// Iterates every exact whole-group drag region in stable visual order.
    pub fn group_drag_regions(self) -> impl Iterator<Item = TabGroupDragRegionPaintRecord> + 'plan {
        let bar = *self.record.id();
        self.record
            .group_drag()
            .into_iter()
            .flat_map(|record| record.regions())
            .map(move |record| TabGroupDragRegionPaintRecord {
                bar,
                record,
                output: self.output,
            })
    }

    /// Returns one exact whole-group drag region by semantic role.
    #[must_use]
    pub fn group_drag_region(
        self,
        kind: TabGroupDragRegionKind,
    ) -> Option<TabGroupDragRegionPaintRecord> {
        Some(TabGroupDragRegionPaintRecord {
            bar: *self.record.id(),
            record: self.record.group_drag()?.region(kind)?,
            output: self.output,
        })
    }

    /// Returns the effective tab-bar interaction policy for this strip.
    #[must_use]
    pub const fn interaction(self) -> TabBarInteraction {
        self.record.interaction()
    }

    /// Returns the semantic paint layer for this tab strip.
    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}
