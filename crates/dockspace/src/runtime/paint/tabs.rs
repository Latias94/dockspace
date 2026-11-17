//! Pane, tab, and tab-bar paint records.

use crate::geometry::LogicalRect;
use crate::ids::{ItemId, RootId, TabKey};
use crate::policy::TabBarInteraction;
use crate::presentation::scene::{
    PaneRecord, PresentationPlan, TabBarRecord, TabBarSceneId, TabGroupDragRegionKind,
    TabGroupDragRegionRecord, TabRecord, TabStripMemberRecord,
    TabStripMemberVisibility as CoreTabStripMemberVisibility, TabsRecord,
};

use super::{DockspacePaintLayer, DockspaceVisualId, VisualIdentity};

/// Read-only geometry and identity for one real application Pane.
#[derive(Debug, Clone, Copy)]
pub struct PanePaintRecord<'plan> {
    pub(super) record: &'plan PaneRecord,
}

impl PanePaintRecord<'_> {
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

    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }

    #[must_use]
    pub const fn content_bounds(self) -> LogicalRect {
        self.record.content_bounds()
    }

    #[must_use]
    pub const fn item(self) -> ItemId {
        self.record.item()
    }

    /// Returns the exact application content clip.
    #[must_use]
    pub const fn clip(self) -> LogicalRect {
        self.record.clip()
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Independent background of one active Tabs container.
#[derive(Debug, Clone, Copy)]
pub struct TabsPaintRecord<'plan> {
    pub(super) record: &'plan TabsRecord,
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
    pub(super) plan: &'plan PresentationPlan,
    pub(super) selected: bool,
    pub(super) operable: bool,
}

impl<'plan> TabPaintRecord<'plan> {
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

    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    #[must_use]
    pub const fn key(self) -> TabKey {
        self.record.id().tab
    }

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

    #[must_use]
    pub const fn full_bounds(self) -> LogicalRect {
        self.record.full_bounds()
    }

    #[must_use]
    pub const fn visible_bounds(self) -> LogicalRect {
        self.record.visible_bounds()
    }

    #[must_use]
    pub const fn text_bounds(self) -> LogicalRect {
        self.record.text_bounds()
    }

    /// Returns the allocation used to shape and position text before scrolling clips it.
    #[must_use]
    pub const fn text_layout_bounds(self) -> LogicalRect {
        self.record.text_layout_bounds()
    }

    #[must_use]
    pub const fn drag_bounds(self) -> LogicalRect {
        self.record.drag_hit().rect()
    }

    #[must_use]
    pub const fn close_visual_bounds(self) -> Option<LogicalRect> {
        self.record.close_visual_bounds()
    }

    #[must_use]
    pub const fn close_bounds(self) -> Option<LogicalRect> {
        self.record.close_bounds()
    }

    #[must_use]
    pub const fn selected(self) -> bool {
        self.selected
    }

    /// Returns whether the exact tab body may publish semantic actions.
    #[must_use]
    pub const fn operable(self) -> bool {
        self.operable
    }

    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.record.ordinal()
    }

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
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tab(self.record.tab()))
    }

    #[must_use]
    pub const fn tab_bar_visual_id(self) -> DockspaceVisualId {
        let tab = self.record.tab();
        DockspaceVisualId(VisualIdentity::TabBar(TabBarSceneId {
            root: tab.root,
            tabs: tab.tabs,
        }))
    }

    #[must_use]
    pub const fn key(self) -> TabKey {
        self.record.tab().tab
    }

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

    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.record.ordinal()
    }

    #[must_use]
    pub const fn full_bounds(self) -> LogicalRect {
        self.record.full_bounds()
    }

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
    pub(super) plan: &'plan PresentationPlan,
    pub(super) selected: Option<TabKey>,
}

/// Read-only geometry and identity for one whole-group drag region.
#[derive(Debug, Clone, Copy)]
pub struct TabGroupDragRegionPaintRecord {
    pub(super) bar: TabBarSceneId,
    pub(super) record: TabGroupDragRegionRecord,
}

impl TabGroupDragRegionPaintRecord {
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
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabBar(*self.record.id()))
    }

    /// Returns the independent Tabs container represented by this strip.
    #[must_use]
    pub const fn tabs_visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tabs(*self.record.id()))
    }

    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().root
    }

    /// Returns the group's selected direct child, including a fully clipped tab.
    #[must_use]
    pub const fn selected(self) -> Option<TabKey> {
        self.selected
    }

    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }

    #[must_use]
    pub const fn viewport(self) -> LogicalRect {
        self.record.viewport()
    }

    #[must_use]
    pub const fn scroll_offset(self) -> f64 {
        self.record.scroll_offset()
    }

    #[must_use]
    pub const fn maximum_scroll_offset(self) -> f64 {
        self.record.maximum_scroll_offset()
    }

    #[must_use]
    pub fn hidden_tabs(self) -> &'plan [TabKey] {
        self.record.hidden_tabs()
    }

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
            .map(move |record| TabGroupDragRegionPaintRecord { bar, record })
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
        })
    }

    #[must_use]
    pub const fn interaction(self) -> TabBarInteraction {
        self.record.interaction()
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}
