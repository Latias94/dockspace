use std::fmt;

use crate::geometry::LogicalRect;
use crate::graph::{Node, SplitWeight, SurfacePresentation, Workspace};
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};

use super::{
    DockspaceAxis, DockspaceGridShape, DockspaceGridShares, DockspaceLayout, RootAllocation,
};
use crate::model::PreparedTabsGroup;

/// Read-only product view of a validated docking workspace.
#[derive(Clone, Copy)]
pub struct DockspaceView<'a> {
    workspace: &'a Workspace,
}

impl fmt::Debug for DockspaceView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceView")
            .field("surfaces", &self.surfaces().collect::<Vec<_>>())
            .finish()
    }
}

impl<'a> DockspaceView<'a> {
    pub(crate) const fn new(workspace: &'a Workspace) -> Self {
        Self { workspace }
    }

    /// Copies this view into an independent, validated logical layout.
    ///
    /// Preserves surfaces, roots, tab order and selection, hidden items, split weights, central
    /// regions, contained bounds and stacking order. The copy has no pane data,
    /// policy, presentation settings, document sidecars, native window bindings,
    /// queued input, gestures or output authority. Copying allocates owned graph
    /// storage and does not change the source workspace.
    ///
    /// For a recovery checkpoint of committed state, call this on
    /// [`DockspaceSession::view`](crate::runtime::DockspaceSession::view).
    /// A host frame's view instead copies that frame's current candidate. Start
    /// a new session with [`DockspaceSession::from_layout`](crate::runtime::DockspaceSession::from_layout)
    /// and supply its policy and presentation configuration separately. Native
    /// hosts must also establish fresh window ownership and presentation.
    ///
    /// Use the document persistence interfaces when external item keys, document
    /// generations or native hosting preferences must survive reconstruction.
    #[must_use]
    pub fn to_layout(self) -> DockspaceLayout {
        DockspaceLayout {
            workspace: self.workspace.clone(),
        }
    }

    /// Returns whether an owned item is hidden. Unknown items are not hidden.
    #[must_use]
    pub fn item_is_hidden(self, item: ItemId) -> bool {
        self.workspace.item_is_hidden(item)
    }

    /// Iterates hidden owned items in stable identity order.
    pub fn hidden_items(self) -> impl Iterator<Item = ItemId> + 'a {
        self.workspace.hidden_items()
    }

    /// Returns a logical surface by stable identity.
    #[must_use]
    pub fn surface(self, id: SurfaceId) -> Option<DockspaceSurfaceView<'a>> {
        self.workspace
            .surface(id)
            .map(|presentation| DockspaceSurfaceView {
                workspace: self.workspace,
                id,
                presentation,
            })
    }

    /// Iterates logical surfaces in stable identity order.
    pub fn surfaces(self) -> impl Iterator<Item = DockspaceSurfaceView<'a>> + 'a {
        self.workspace
            .surfaces()
            .map(move |(id, presentation)| DockspaceSurfaceView {
                workspace: self.workspace,
                id,
                presentation,
            })
    }

    /// Returns a docking root by stable identity.
    #[must_use]
    pub fn root(self, id: RootId) -> Option<DockspaceRootView<'a>> {
        self.workspace.root(id).map(|record| DockspaceRootView {
            workspace: self.workspace,
            id,
            content: record.node,
            central: record.central,
            allocation: record.allocation,
        })
    }

    /// Borrows a currently owned node by its session-local opaque identity.
    ///
    /// Removed identities return `None`. A key is a query handle within its
    /// originating session; it is neither persistent nor mutation authority.
    #[must_use]
    pub fn node(self, key: crate::ids::NodeKey) -> Option<DockspaceNodeView<'a>> {
        let root = self.workspace.node_root(key.0)?;
        Some(DockspaceNodeView {
            workspace: self.workspace,
            node: key.0,
            central: self.workspace.root(root)?.central,
        })
    }

    /// Returns a contained-floating presentation by stable identity.
    #[must_use]
    pub fn contained(self, id: FloatingPresentationId) -> Option<DockspaceContainedView<'a>> {
        self.workspace
            .contained_floating(id)
            .map(|contained| DockspaceContainedView {
                workspace: self.workspace,
                id,
                root: contained.root,
                rect: contained.rect,
            })
    }

    /// Locates one open item through stable product identities.
    ///
    /// This query hides runtime node identities and returns `None` when the item is not open.
    #[must_use]
    pub fn item(self, item: ItemId) -> Option<DockspaceItemView<'a>> {
        let location = self.workspace.locate_item_by_id(item)?;
        let root = location.root;
        let tabs = location.tabs().and_then(|node| {
            self.workspace.node(node).map(|_| DockspaceTabsView {
                workspace: self.workspace,
                node,
                central: self
                    .workspace
                    .root(root)
                    .is_some_and(|record| record.central == Some(node)),
                central_node: self.workspace.root(root).and_then(|record| record.central),
            })
        });
        let (surface, contained) = match self.workspace.presentation_for_root(root)? {
            crate::graph::workspace::RootPresentationOwner::Main { surface } => (surface, None),
            crate::graph::workspace::RootPresentationOwner::Contained { surface, floating } => {
                (surface, Some(floating))
            }
        };
        Some(DockspaceItemView {
            item,
            surface,
            root,
            contained,
            tab_index: location.tab_index(),
            tabs,
            workspace: self.workspace,
        })
    }
}

/// Read-only product location of one open item.
#[derive(Debug, Clone, Copy)]
pub struct DockspaceItemView<'a> {
    item: ItemId,
    surface: SurfaceId,
    root: RootId,
    contained: Option<FloatingPresentationId>,
    tab_index: Option<usize>,
    tabs: Option<DockspaceTabsView<'a>>,
    workspace: &'a Workspace,
}

impl<'a> DockspaceItemView<'a> {
    /// Returns the stable item identity.
    #[must_use]
    pub const fn item(self) -> ItemId {
        self.item
    }

    /// Returns whether this owned item contributes visible docking content.
    #[must_use]
    pub fn is_visible(self) -> bool {
        !self.is_hidden()
    }

    /// Returns whether this item remains owned but is excluded from presentation.
    #[must_use]
    pub fn is_hidden(self) -> bool {
        self.workspace.item_is_hidden(self.item)
    }

    /// Returns the logical surface presenting this item.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    /// Returns the stable root currently owning this item.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.root
    }

    /// Returns the contained-floating presentation, or `None` for a main root.
    #[must_use]
    pub const fn contained(self) -> Option<FloatingPresentationId> {
        self.contained
    }

    /// Returns the inline item's durable roster index, or `None` for a bare pane.
    #[must_use]
    pub const fn tab_index(self) -> Option<usize> {
        self.tab_index
    }

    /// Returns the immediate inline tabs owner, or `None` for a bare pane.
    /// A containing layout tab does not become a bare pane's immediate owner.
    #[must_use]
    pub const fn tabs(self) -> Option<DockspaceTabsView<'a>> {
        self.tabs
    }

    /// Returns whether this real content is visible through every ancestor tabs choice.
    #[must_use]
    pub fn is_active(self) -> bool {
        self.workspace.item_is_active(self.item)
    }
}

/// Read-only product view of one logical surface.
#[derive(Clone, Copy)]
pub struct DockspaceSurfaceView<'a> {
    workspace: &'a Workspace,
    id: SurfaceId,
    presentation: &'a SurfacePresentation,
}

impl fmt::Debug for DockspaceSurfaceView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceSurfaceView")
            .field("id", &self.id)
            .field("main_root", &self.main_root())
            .field("contained", &self.contained().collect::<Vec<_>>())
            .finish()
    }
}

impl<'a> DockspaceSurfaceView<'a> {
    /// Returns the stable surface identity.
    #[must_use]
    pub const fn id(self) -> SurfaceId {
        self.id
    }

    /// Returns whether the surface has no main root.
    #[must_use]
    pub const fn is_rootless(self) -> bool {
        self.presentation.main_root.is_none()
    }

    /// Returns the main docking root, when present.
    #[must_use]
    pub fn main_root(self) -> Option<DockspaceRootView<'a>> {
        self.presentation.main_root.and_then(|root| {
            self.workspace.root(root).map(|record| DockspaceRootView {
                workspace: self.workspace,
                id: root,
                content: record.node,
                central: record.central,
                allocation: record.allocation,
            })
        })
    }

    /// Visits active real content in main-root order, then contained roots back to front.
    /// Inactive layout branches and hidden content are excluded.
    pub fn active_items(self) -> impl Iterator<Item = ItemId> + 'a {
        let workspace = self.workspace;
        self.presentation
            .main_root
            .into_iter()
            .chain(
                self.presentation
                    .contained
                    .iter()
                    .filter_map(move |floating| {
                        workspace
                            .contained_floating(*floating)
                            .map(|record| record.root)
                    }),
            )
            .filter_map(move |root| workspace.root(root).map(|record| record.node))
            .flat_map(move |node| workspace.active_items_in_subtree(node))
    }

    /// Returns the number of contained-floating roots on this surface.
    #[must_use]
    pub fn contained_count(self) -> usize {
        self.presentation.contained.len()
    }

    /// Iterates contained-floating roots in back-to-front presentation order.
    pub fn contained(self) -> impl Iterator<Item = DockspaceContainedView<'a>> + 'a {
        let workspace = self.workspace;
        self.presentation.contained.iter().filter_map(move |id| {
            workspace
                .contained_floating(*id)
                .map(|contained| DockspaceContainedView {
                    workspace,
                    id: *id,
                    root: contained.root,
                    rect: contained.rect,
                })
        })
    }
}

/// Read-only product view of one stable docking root.
#[derive(Clone, Copy)]
pub struct DockspaceRootView<'a> {
    workspace: &'a Workspace,
    id: RootId,
    content: NodeId,
    central: Option<NodeId>,
    allocation: RootAllocation,
}

impl fmt::Debug for DockspaceRootView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceRootView")
            .field("id", &self.id)
            .field("content", &self.content())
            .field("allocation", &self.allocation)
            .finish()
    }
}

impl<'a> DockspaceRootView<'a> {
    /// Returns the stable root identity.
    #[must_use]
    pub const fn id(self) -> RootId {
        self.id
    }

    /// Visits all owned real content in visual preorder, including hidden and inactive content.
    pub fn items(self) -> impl Iterator<Item = ItemId> + 'a {
        self.workspace.items_in_subtree(self.content)
    }

    /// Visits active real content in visual layout order using selected tabs branches.
    pub fn active_items(self) -> impl Iterator<Item = ItemId> + 'a {
        self.workspace.active_items_in_subtree(self.content)
    }

    /// Returns the root topology without exposing its runtime identity.
    #[must_use]
    pub fn content(self) -> Option<DockspaceNodeView<'a>> {
        self.workspace
            .node(self.content)
            .map(|_| DockspaceNodeView {
                workspace: self.workspace,
                node: self.content,
                central: self.central,
            })
    }

    /// Returns the declared central docking anchor, when configured.
    #[must_use]
    pub fn central(self) -> Option<DockspaceNodeView<'a>> {
        self.central.and_then(|node| {
            self.workspace.node(node).map(|_| DockspaceNodeView {
                workspace: self.workspace,
                node,
                central: self.central,
            })
        })
    }

    /// Returns the root's constraint redistribution independently of its anchor.
    #[must_use]
    pub const fn allocation(self) -> RootAllocation {
        self.allocation
    }

    /// Returns whether an empty central leaf keeps this root alive.
    ///
    /// Only the main root of a logical surface retains an empty central area
    /// when a move or approved close removes its last item. An emptied contained
    /// source is retired. Explicitly declaring an empty contained anchor remains
    /// valid. Native hosts use this lifecycle rule before admitting a move or
    /// close that would empty the root.
    #[must_use]
    pub fn retains_empty_central_root(self) -> bool {
        self.central.is_some()
            && matches!(
                self.workspace.presentation_for_root(self.id),
                Some(crate::graph::workspace::RootPresentationOwner::Main { .. })
            )
    }

    /// Returns whether this complete root contains exactly `item` and no other
    /// content.
    ///
    /// Empty central leaves do not affect the result. The query traverses the
    /// complete root topology, so it also covers nested splits and a contained
    /// root whose central leaf is empty. Native hosts use this semantic query
    /// when deciding whether moving or closing the last item would retire the
    /// root that backs a fixed host surface.
    #[must_use]
    pub fn contains_only_item(self, item: ItemId) -> bool {
        self.workspace.collect_items_in_subtree(self.content) == [item]
    }

    /// Returns whether this root contains no content outside one captured tabs
    /// group.
    ///
    /// The captured group must still refer to the current node in this root.
    /// Empty central anchors are ignored, which keeps the query aligned with root
    /// retirement after a nested split has been drained.
    #[must_use]
    pub fn contains_only_tabs_group(self, group: &PreparedTabsGroup) -> bool {
        if group.source.root != self.id
            || self.workspace.root_reference(self.id) != Some(&group.source.reference)
        {
            return false;
        }
        let target = group.source.node;
        if self.workspace.node_root(target) != Some(self.id) {
            return false;
        }
        let group_items = self.workspace.collect_items_in_subtree(target);
        !group_items.is_empty()
            && group_items.len() == self.workspace.collect_items_in_subtree(self.content).len()
    }

    /// Returns whether this root has no real content outside one exact tab.
    ///
    /// A layout tab includes every hidden and inactive descendant. The captured
    /// membership must still match this root; this query grants no close or
    /// native-window retirement permission.
    #[must_use]
    pub fn contains_only_tab(self, tab: &super::PreparedTab) -> bool {
        let source = &tab.source;
        if source.root() != self.id
            || self.workspace.root_reference(self.id) != Some(source.reference())
            || self
                .workspace
                .capture_tab_source(source.root(), source.tabs(), source.tab())
                .as_ref()
                .ok()
                != Some(source)
        {
            return false;
        }
        match source.tab().item() {
            Some(item) => self.contains_only_item(item),
            None => source.tab().layout().is_some_and(|node| {
                let count = self.workspace.items_in_subtree(node).count();
                count != 0 && count == self.items().count()
            }),
        }
    }
}

/// Read-only product view of one contained-floating presentation.
#[derive(Clone, Copy)]
pub struct DockspaceContainedView<'a> {
    workspace: &'a Workspace,
    id: FloatingPresentationId,
    root: RootId,
    rect: LogicalRect,
}

impl fmt::Debug for DockspaceContainedView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceContainedView")
            .field("id", &self.id)
            .field("rect", &self.rect)
            .field("root", &self.root())
            .finish()
    }
}

impl<'a> DockspaceContainedView<'a> {
    /// Returns the stable contained-floating identity.
    #[must_use]
    pub const fn id(self) -> FloatingPresentationId {
        self.id
    }

    /// Returns durable surface-local bounds.
    #[must_use]
    pub const fn rect(self) -> LogicalRect {
        self.rect
    }

    /// Returns the presented docking root.
    #[must_use]
    pub fn root(self) -> Option<DockspaceRootView<'a>> {
        self.workspace
            .root(self.root)
            .map(|record| DockspaceRootView {
                workspace: self.workspace,
                id: self.root,
                content: record.node,
                central: record.central,
                allocation: record.allocation,
            })
    }
}

/// Read-only product view of one real pane or structural container.
#[derive(Clone, Copy)]
pub struct DockspaceNodeView<'a> {
    workspace: &'a Workspace,
    node: NodeId,
    central: Option<NodeId>,
}

impl fmt::Debug for DockspaceNodeView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(tabs) = self.tabs() {
            return formatter
                .debug_struct("DockspaceNodeView")
                .field("tabs", &tabs)
                .finish();
        }
        if let Some(split) = self.split() {
            return formatter
                .debug_struct("DockspaceNodeView")
                .field("split", &split)
                .finish();
        }
        if let Some(grid) = self.grid() {
            return formatter
                .debug_struct("DockspaceNodeView")
                .field("grid", &grid)
                .finish();
        }
        if let Some(item) = self.item() {
            return formatter
                .debug_struct("DockspaceNodeView")
                .field("item", &item)
                .finish();
        }
        formatter.write_str("DockspaceNodeView(<invalid>)")
    }
}

impl<'a> DockspaceNodeView<'a> {
    /// Returns this session-local opaque identity without granting mutation authority.
    #[must_use]
    pub const fn key(self) -> crate::ids::NodeKey {
        crate::ids::NodeKey(self.node)
    }

    /// Returns the root currently owning this real node.
    #[must_use]
    pub fn root(self) -> Option<DockspaceRootView<'a>> {
        DockspaceView::new(self.workspace).root(self.workspace.node_root(self.node)?)
    }

    /// Visits all owned real content in visual preorder, including hidden and inactive content.
    pub fn items(self) -> impl Iterator<Item = ItemId> + 'a {
        self.workspace.items_in_subtree(self.node)
    }

    /// Returns whether this subtree contributes visible content or an empty central target.
    #[must_use]
    pub fn has_visible_content(self) -> bool {
        self.workspace.node_has_visible_content(self.node)
    }

    /// Returns whether this node is the root's central Tabs anchor.
    #[must_use]
    pub fn is_central(self) -> bool {
        self.central == Some(self.node)
    }

    /// Returns real content when this is a bare pane.
    #[must_use]
    pub fn item(self) -> Option<ItemId> {
        match self.workspace.node(self.node) {
            Some(Node::Pane { item }) => Some(*item),
            _ => None,
        }
    }

    /// Borrows this structural container's explicit layout title.
    #[must_use]
    pub fn layout_title(self) -> Option<&'a str> {
        self.workspace.node(self.node)?.layout_title()
    }

    /// Returns a view when this node is a Tabs container.
    #[must_use]
    pub fn tabs(self) -> Option<DockspaceTabsView<'a>> {
        match self.workspace.node(self.node) {
            Some(Node::Tabs { .. }) => Some(DockspaceTabsView {
                workspace: self.workspace,
                node: self.node,
                central: self.is_central(),
                central_node: self.central,
            }),
            _ => None,
        }
    }

    /// Returns a split view when this node is an N-ary split.
    #[must_use]
    pub fn split(self) -> Option<DockspaceSplitView<'a>> {
        match self.workspace.node(self.node) {
            Some(Node::Split {
                axis,
                children,
                weights,
                ..
            }) => Some(DockspaceSplitView {
                workspace: self.workspace,
                axis: (*axis).into(),
                children,
                weights,
                central: self.central,
            }),
            Some(Node::Pane { .. } | Node::Tabs { .. } | Node::Grid { .. }) | None => None,
        }
    }

    /// Returns a grid view with one durable row-major child roster.
    #[must_use]
    pub fn grid(self) -> Option<DockspaceGridView<'a>> {
        match self.workspace.node(self.node) {
            Some(Node::Grid {
                shape,
                children,
                shares,
                ..
            }) => Some(DockspaceGridView {
                workspace: self.workspace,
                shape: *shape,
                children,
                shares: shares.as_ref(),
                central: self.central,
            }),
            _ => None,
        }
    }
}

/// Read-only product view of a grid with shared row and column tracks.
#[derive(Clone, Copy)]
pub struct DockspaceGridView<'a> {
    workspace: &'a Workspace,
    shape: DockspaceGridShape,
    children: &'a [NodeId],
    shares: Option<&'a DockspaceGridShares>,
    central: Option<NodeId>,
}

impl fmt::Debug for DockspaceGridView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceGridView")
            .field("shape", &self.shape)
            .field("child_count", &self.children.len())
            .field("shares", &self.shares)
            .finish()
    }
}

impl<'a> DockspaceGridView<'a> {
    /// Returns declared dimensions without resolving Auto for a viewport.
    #[must_use]
    pub const fn shape(self) -> DockspaceGridShape {
        self.shape
    }

    /// Borrows explicit shared track values, or `None` for equal default shares.
    #[must_use]
    pub const fn shares(self) -> Option<&'a DockspaceGridShares> {
        self.shares
    }

    /// Returns the real durable child count, including hidden children.
    #[must_use]
    pub fn child_count(self) -> usize {
        self.children.len()
    }

    /// Visits every real child in durable row-major order, including hidden children.
    pub fn children(
        self,
    ) -> impl ExactSizeIterator<Item = DockspaceNodeView<'a>> + DoubleEndedIterator + 'a {
        self.children
            .iter()
            .copied()
            .map(move |node| DockspaceNodeView {
                workspace: self.workspace,
                node,
                central: self.central,
            })
    }
}

/// One direct tab child without a representative descendant item.
#[derive(Debug, Clone, Copy)]
pub enum DockspaceTabView<'a> {
    /// Compact real application content.
    Item(ItemId),
    /// A complete structural child layout.
    Layout(DockspaceNodeView<'a>),
}

/// Read-only product view of one tabs container.
#[derive(Debug, Clone, Copy)]
pub struct DockspaceTabsView<'a> {
    workspace: &'a Workspace,
    node: NodeId,
    central: bool,
    central_node: Option<NodeId>,
}

impl<'a> DockspaceTabsView<'a> {
    /// Returns this session-local opaque identity without granting mutation authority.
    #[must_use]
    pub const fn key(self) -> crate::ids::NodeKey {
        crate::ids::NodeKey(self.node)
    }

    /// Returns every direct child key in durable order, including hidden members.
    #[must_use]
    pub fn tabs(self) -> &'a [TabKey] {
        match self.workspace.node(self.node) {
            Some(Node::Tabs { items, .. }) => items,
            _ => &[],
        }
    }

    /// Iterates each direct child as real content or a complete layout view.
    pub fn children(self) -> impl DoubleEndedIterator<Item = DockspaceTabView<'a>> + Clone + 'a {
        self.tabs()
            .iter()
            .copied()
            .filter_map(move |tab| self.child(tab))
    }

    /// Returns one exact direct child.
    #[must_use]
    pub fn child(self, tab: TabKey) -> Option<DockspaceTabView<'a>> {
        if !self.tabs().contains(&tab) {
            return None;
        }
        if let Some(item) = tab.item() {
            return Some(DockspaceTabView::Item(item));
        }
        let node = tab.layout()?;
        Some(DockspaceTabView::Layout(DockspaceNodeView {
            workspace: self.workspace,
            node,
            central: self.central_node,
        }))
    }

    /// Borrows the authoritative label of one direct child.
    #[must_use]
    pub fn tab_label(self, tab: TabKey) -> Option<crate::graph::TabLabel<'a>> {
        self.workspace.tab_label(self.node, tab)
    }

    /// Iterates directly owned real content, including hidden members.
    pub fn items(self) -> impl DoubleEndedIterator<Item = ItemId> + Clone + 'a {
        self.tabs().iter().filter_map(|tab| tab.item())
    }

    /// Returns the logical selected direct child.
    #[must_use]
    pub fn selected(self) -> Option<TabKey> {
        match self.workspace.node(self.node) {
            Some(Node::Tabs { selected, .. }) => *selected,
            _ => None,
        }
    }

    /// Iterates visible direct children in durable order.
    pub fn visible_tabs(self) -> impl DoubleEndedIterator<Item = TabKey> + Clone + 'a {
        self.tabs()
            .iter()
            .copied()
            .filter(move |tab| self.workspace.tab_is_visible(*tab))
    }

    /// Iterates visible directly owned real content.
    pub fn visible_items(self) -> impl DoubleEndedIterator<Item = ItemId> + Clone + 'a {
        self.visible_tabs().filter_map(TabKey::item)
    }

    /// Returns the selected eligible direct child.
    #[must_use]
    pub fn visible_selected(self) -> Option<TabKey> {
        self.workspace.visible_tab_selection(self.node)
    }

    /// Returns whether this container is the root's retained central anchor.
    #[must_use]
    pub const fn is_central(self) -> bool {
        self.central
    }
}

/// Read-only product view of one N-ary split.
#[derive(Clone, Copy)]
pub struct DockspaceSplitView<'a> {
    workspace: &'a Workspace,
    axis: DockspaceAxis,
    children: &'a [NodeId],
    weights: &'a [SplitWeight],
    central: Option<NodeId>,
}

impl fmt::Debug for DockspaceSplitView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceSplitView")
            .field("axis", &self.axis)
            .field("child_count", &self.child_count())
            .field("weights", &self.weights().collect::<Vec<_>>())
            .finish()
    }
}

impl<'a> DockspaceSplitView<'a> {
    /// Returns the split direction.
    #[must_use]
    pub const fn axis(self) -> DockspaceAxis {
        self.axis
    }

    /// Returns the number of ordered children.
    #[must_use]
    pub fn child_count(self) -> usize {
        self.children.len()
    }

    /// Iterates child nodes in visual order.
    pub fn children(self) -> impl Iterator<Item = DockspaceNodeView<'a>> + 'a {
        let workspace = self.workspace;
        let central = self.central;
        self.children
            .iter()
            .copied()
            .map(move |node| DockspaceNodeView {
                workspace,
                node,
                central,
            })
    }

    /// Iterates normalized child weights in visual order.
    pub fn weights(self) -> impl Iterator<Item = f64> + 'a {
        self.weights.iter().map(|weight| weight.get())
    }
}
