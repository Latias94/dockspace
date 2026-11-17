//! Checked workspace queries used to capture mutation preconditions.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use crate::engine::transition::WorkspaceVersion;
use crate::graph::command::{
    ContainedRosterSource, DockFraction, Edge, EdgeTarget, EdgeTargetScope, FingerprintNode,
    FingerprintPresentation, ItemSource, MovePayload, NodeFingerprint, NodeFingerprintRecord,
    NodeSource, RootFingerprintData, RootReference, SurfaceRosterSource, TabSource, TabTarget,
};
use crate::graph::error::{CommandError, ReferenceRole};
use crate::graph::{Node, Workspace};
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::policy::DockTargetRuleKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParentLink {
    pub(crate) parent: NodeId,
    pub(crate) index: usize,
}

/// Actual immediate ownership of one real application item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemPlace {
    /// Compact direct item child inside a tabs container.
    InlineTab { tabs: NodeId, index: usize },
    /// Independently allocated pane without implicit tabs chrome.
    Pane { node: NodeId },
}

impl ItemPlace {
    /// Returns the immediate actual graph owner.
    pub const fn owner_node(self) -> NodeId {
        match self {
            Self::InlineTab { tabs, .. } => tabs,
            Self::Pane { node } => node,
        }
    }
    /// Returns the immediate inline tabs owner.
    pub const fn tabs(self) -> Option<NodeId> {
        match self {
            Self::InlineTab { tabs, .. } => Some(tabs),
            Self::Pane { .. } => None,
        }
    }
    /// Returns the actual inline roster index.
    pub const fn tab_index(self) -> Option<usize> {
        match self {
            Self::InlineTab { index, .. } => Some(index),
            Self::Pane { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ItemLocation {
    pub(crate) root: RootId,
    pub(crate) place: ItemPlace,
}
impl ItemLocation {
    pub(crate) const fn owner_node(self) -> NodeId {
        self.place.owner_node()
    }
    pub(crate) const fn tabs(self) -> Option<NodeId> {
        self.place.tabs()
    }
    pub(crate) const fn tab_index(self) -> Option<usize> {
        self.place.tab_index()
    }
}

/// Current presentation which exclusively owns a logical docking root.
///
/// A root keeps its identity when it moves between these presentation
/// carriers. Adapters can use this fact to preserve presentation identities
/// without inspecting workspace storage or guessing from geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootPresentationOwner {
    /// The root is the main content of a logical surface.
    Main {
        /// Surface which directly owns the root.
        surface: SurfaceId,
    },
    /// The root is presented by a contained floating on a logical surface.
    Contained {
        /// Surface which hosts the contained presentation.
        surface: SurfaceId,
        /// Stable contained presentation identity.
        floating: FloatingPresentationId,
    },
}

/// Immutable structural authority derived for one exact workspace revision.
///
/// Scene compilation shares this index across every target captured from the
/// same requirement manifest. Every new revision derives fresh semantic facts;
/// the graph owner separately retains its actual immutable root structures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceIndex {
    version: WorkspaceVersion,
    roots: BTreeMap<RootId, WorkspaceRootIndex>,
}

#[derive(Debug, Clone)]
struct WorkspaceRootIndex {
    root_node: NodeId,
    central: Option<NodeId>,
    owner: RootPresentationOwner,
    members: Arc<HashSet<NodeId>>,
    item_count: usize,
    fingerprint: NodeFingerprint,
    reference: RootReference,
}

impl PartialEq for WorkspaceRootIndex {
    fn eq(&self, other: &Self) -> bool {
        self.root_node == other.root_node
            && self.central == other.central
            && self.owner == other.owner
            && self.members == other.members
            && self.item_count == other.item_count
            && self.fingerprint == other.fingerprint
    }
}

impl Eq for WorkspaceRootIndex {}

impl WorkspaceIndex {
    pub(crate) fn build(
        workspace: &Workspace,
        version: WorkspaceVersion,
    ) -> Result<Self, CommandError> {
        let mut owners = BTreeMap::new();
        for (surface, presentation) in &workspace.storage().surfaces {
            if let Some(root) = presentation.main_root
                && owners
                    .insert(root, RootPresentationOwner::Main { surface: *surface })
                    .is_some()
            {
                return Err(CommandError::Invariant {
                    stage: "index unique root presentation owner",
                });
            }
            for floating in &presentation.contained {
                let root = workspace
                    .storage()
                    .contained_floatings
                    .get(floating)
                    .ok_or(CommandError::Invariant {
                        stage: "index contained presentation root",
                    })?
                    .root;
                if owners
                    .insert(
                        root,
                        RootPresentationOwner::Contained {
                            surface: *surface,
                            floating: *floating,
                        },
                    )
                    .is_some()
                {
                    return Err(CommandError::Invariant {
                        stage: "index unique root presentation owner",
                    });
                }
            }
        }

        let mut roots = BTreeMap::new();
        for root in workspace.storage().roots.keys().copied() {
            let owner = owners.get(&root).copied().ok_or(CommandError::Invariant {
                stage: "index root presentation owner",
            })?;
            roots.insert(root, workspace.build_root_index(root, owner)?);
        }
        if owners.len() != roots.len() {
            return Err(CommandError::Invariant {
                stage: "index presentation owner roster",
            });
        }
        Ok(Self { version, roots })
    }

    /// Rebinds canonical selection semantics without deriving topology again.
    ///
    /// The owner must have applied exactly one topology-preserving Select to
    /// this index's workspace. Membership stays shared;
    /// only the selected root's semantic snapshot receives a new value.
    pub(crate) fn after_selection(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        tabs: NodeId,
    ) -> Result<Self, CommandError> {
        if version.epoch() != self.version.epoch() {
            return Err(CommandError::Invariant {
                stage: "rebind selection within the same workspace epoch",
            });
        }
        if workspace.roots().count() != self.roots.len() {
            return Err(CommandError::Invariant {
                stage: "rebind selection with the same root roster",
            });
        }
        for (&other_root, other) in &self.roots {
            if other_root != root {
                workspace.verify_reference(
                    other_root,
                    other.root_node,
                    &other.reference,
                    ReferenceRole::Source,
                )?;
            }
        }
        let indexed = self
            .roots
            .get(&root)
            .ok_or(CommandError::MissingRoot { root })?;
        if !indexed.members.contains(&tabs) {
            return Err(CommandError::NodeOutsideRoot {
                role: ReferenceRole::Source,
                root,
                node: tabs,
            });
        }
        let (selected, mru) = match workspace.node(tabs) {
            Some(Node::Tabs { selected, mru, .. }) => (*selected, mru.as_slice()),
            Some(Node::Pane { .. } | Node::Split { .. } | Node::Grid { .. }) => {
                return Err(CommandError::NodeIsNotTabs { node: tabs });
            }
            None => {
                return Err(CommandError::MissingNode {
                    role: ReferenceRole::Source,
                    node: tabs,
                });
            }
        };
        let mut roots = self.roots.clone();
        let updated = roots
            .get_mut(&root)
            .ok_or(CommandError::MissingRoot { root })?;
        let current = workspace
            .root_reference(root)
            .ok_or(CommandError::MissingRoot { root })?;
        if current != &updated.reference && !current.follows_selection(&updated.reference, tabs) {
            return Err(CommandError::StaleNode {
                role: ReferenceRole::Source,
                node: tabs,
                expected: updated.reference.clone(),
                actual: current.clone(),
            });
        }
        updated.reference = current.clone();
        let fingerprint = Arc::make_mut(&mut updated.fingerprint.0);
        let record = fingerprint
            .nodes
            .iter_mut()
            .find(|record| record.id == tabs)
            .ok_or(CommandError::Invariant {
                stage: "rebind indexed tabs fingerprint",
            })?;
        match &mut record.node {
            FingerprintNode::Tabs {
                selected: previous,
                mru: previous_mru,
                ..
            } => {
                *previous = selected;
                *previous_mru = Arc::from(mru);
            }
            FingerprintNode::Pane { .. }
            | FingerprintNode::Split { .. }
            | FingerprintNode::Grid { .. } => {
                return Err(CommandError::NodeIsNotTabs { node: tabs });
            }
        }
        Ok(Self { version, roots })
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test(version: WorkspaceVersion) -> Self {
        Self {
            version,
            roots: BTreeMap::new(),
        }
    }

    pub(crate) const fn version(&self) -> WorkspaceVersion {
        self.version
    }

    pub(crate) fn root_fingerprint(&self, root: RootId) -> Option<&NodeFingerprint> {
        self.roots.get(&root).map(|record| &record.fingerprint)
    }

    pub(crate) fn at_version(&self, version: WorkspaceVersion) -> Option<WorkspaceIndexView<'_>> {
        (self.version == version).then_some(WorkspaceIndexView { index: self })
    }

    fn require_version(&self, version: WorkspaceVersion) -> Result<(), CommandError> {
        if self.version == version {
            Ok(())
        } else {
            Err(CommandError::Invariant {
                stage: "use workspace index for a different revision",
            })
        }
    }

    fn capture_reference<'index>(
        &'index self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        node: NodeId,
        role: ReferenceRole,
    ) -> Result<&'index WorkspaceRootIndex, CommandError> {
        self.require_version(version)?;
        let indexed = self
            .roots
            .get(&root)
            .ok_or(CommandError::MissingRoot { root })?;
        if workspace.root(root).is_none() {
            return Err(CommandError::MissingRoot { root });
        }
        if !workspace.storage().nodes.contains_key(node) {
            return Err(CommandError::MissingNode { role, node });
        }
        if !workspace.root_contains_node(root, node) {
            return Err(CommandError::NodeOutsideRoot { role, root, node });
        }
        workspace.verify_reference(root, node, &indexed.reference, role)?;
        Ok(indexed)
    }

    pub(crate) fn verify_reference(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        node: NodeId,
        expected: &RootReference,
        role: ReferenceRole,
    ) -> Result<(), CommandError> {
        self.capture_reference(workspace, version, root, node, role)?;
        workspace.verify_reference(root, node, expected, role)
    }

    pub(crate) fn capture_item_source(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        owner: NodeId,
        item: ItemId,
    ) -> Result<ItemSource, CommandError> {
        let indexed =
            self.capture_reference(workspace, version, root, owner, ReferenceRole::Source)?;
        let place = workspace.item_place_in_owner(owner, item)?;
        Ok(ItemSource {
            root,
            place,
            item,
            reference: indexed.reference.clone(),
        })
    }

    pub(crate) fn capture_complete_root_source(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        payload: &MovePayload,
    ) -> Result<Option<NodeSource>, CommandError> {
        let (root, node, reference) = match payload {
            MovePayload::Item(source) => (source.root(), source.owner_node(), source.reference()),
            MovePayload::Tabs(source) | MovePayload::Subtree(source) => {
                (source.root(), source.node(), source.reference())
            }
        };
        let indexed =
            self.capture_reference(workspace, version, root, node, ReferenceRole::Source)?;
        workspace.verify_reference(root, node, reference, ReferenceRole::Source)?;

        let removes_complete_root = match payload {
            MovePayload::Item(source) => {
                workspace.item_move_removes_root(root, indexed.item_count)
                    && workspace
                        .item_place_in_owner(source.owner_node(), source.item())
                        .is_ok()
            }
            MovePayload::Tabs(source) => {
                source.node() == indexed.root_node
                    && matches!(
                        workspace.node(source.node()),
                        Some(Node::Tabs { items, .. }) if !items.is_empty()
                    )
            }
            MovePayload::Subtree(source) => {
                source.node() == indexed.root_node && indexed.item_count > 0
            }
        };
        Ok(removes_complete_root.then(|| NodeSource {
            root,
            node: indexed.root_node,
            reference: indexed.reference.clone(),
        }))
    }

    pub(crate) fn capture_tab_target(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        tabs: NodeId,
    ) -> Result<TabTarget, CommandError> {
        let indexed =
            self.capture_reference(workspace, version, root, tabs, ReferenceRole::Target)?;
        let central = indexed.central == Some(tabs);
        let rule = workspace.pane_local_target_rule(root, tabs, central)?;
        Ok(TabTarget {
            root,
            tabs,
            reference: indexed.reference.clone(),
            surface: indexed.owner.surface(),
            central,
            rule,
        })
    }

    pub(crate) fn capture_inner_edge_target(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        tabs: NodeId,
        edge: Edge,
        fraction: DockFraction,
    ) -> Result<EdgeTarget, CommandError> {
        let indexed =
            self.capture_reference(workspace, version, root, tabs, ReferenceRole::Target)?;
        let central = indexed.central == Some(tabs);
        let rule = workspace.pane_local_target_rule(root, tabs, central)?;
        Ok(EdgeTarget {
            root,
            node: tabs,
            reference: indexed.reference.clone(),
            edge,
            fraction,
            surface: indexed.owner.surface(),
            central,
            rule,
            scope: EdgeTargetScope::Inner,
        })
    }

    pub(crate) fn capture_outer_edge_target(
        &self,
        workspace: &Workspace,
        version: WorkspaceVersion,
        root: RootId,
        edge: Edge,
        fraction: DockFraction,
    ) -> Result<EdgeTarget, CommandError> {
        self.require_version(version)?;
        let indexed = self
            .roots
            .get(&root)
            .ok_or(CommandError::MissingRoot { root })?;
        workspace.verify_reference(
            root,
            indexed.root_node,
            &indexed.reference,
            ReferenceRole::Target,
        )?;
        Ok(EdgeTarget {
            root,
            node: indexed.root_node,
            reference: indexed.reference.clone(),
            edge,
            fraction,
            surface: indexed.owner.surface(),
            central: indexed.central == Some(indexed.root_node),
            rule: DockTargetRuleKey::Root(root),
            scope: EdgeTargetScope::Outer,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceIndexView<'a> {
    index: &'a WorkspaceIndex,
}

impl<'index> WorkspaceIndexView<'index> {
    #[cfg(test)]
    pub(crate) fn root_fingerprint(&self, root: RootId) -> Option<&'index NodeFingerprint> {
        self.index.root_fingerprint(root)
    }

    pub(crate) fn root_owner(&self, root: RootId) -> Option<RootPresentationOwner> {
        self.index.roots.get(&root).map(|record| record.owner)
    }

    pub(crate) fn root_node(&self, root: RootId) -> Option<NodeId> {
        self.index.roots.get(&root).map(|record| record.root_node)
    }

    pub(crate) fn root_contains_node(&self, root: RootId, node: NodeId) -> bool {
        self.index
            .roots
            .get(&root)
            .is_some_and(|record| record.members.contains(&node))
    }
}

impl RootPresentationOwner {
    pub(crate) const fn surface(self) -> SurfaceId {
        match self {
            Self::Main { surface } | Self::Contained { surface, .. } => surface,
        }
    }
}

impl Workspace {
    /// Empty central areas persist only in main dockspaces, not floating shells.
    pub(crate) fn retains_empty_central_root(&self, root: RootId) -> bool {
        self.root(root)
            .is_some_and(|record| record.central.is_some())
            && matches!(
                self.presentation_for_root(root),
                Some(RootPresentationOwner::Main { .. })
            )
    }

    /// Classifies a checked singleton payload using its current root item count.
    pub(crate) fn item_move_removes_root(&self, root: RootId, item_count: usize) -> bool {
        item_count == 1 && !self.retains_empty_central_root(root)
    }

    pub(crate) fn matches_surface_roster_source(&self, source: &SurfaceRosterSource) -> bool {
        let Some(presentation) = self.surface(source.surface()) else {
            return false;
        };
        if presentation.main_root != source.main_root()
            || presentation.contained.len() != source.contained().len()
        {
            return false;
        }
        if let Some(main) = source.main_source() {
            let Ok(actual) = self.capture_node_source(main.root(), main.node()) else {
                return false;
            };
            if actual != *main {
                return false;
            }
        }

        presentation
            .contained
            .iter()
            .zip(source.contained())
            .all(|(floating, expected)| {
                *floating == expected.floating()
                    && self.contained_floating(*floating).is_some_and(|record| {
                        record.root == expected.root() && record.rect == expected.rect()
                    })
                    && self
                        .capture_node_source(expected.root(), expected.source().node())
                        .is_ok_and(|actual| actual == *expected.source())
            })
    }

    /// Captures the complete contained roster of one surface.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::MissingSurface`] when `surface` is absent.
    pub fn capture_contained_roster(
        &self,
        surface: SurfaceId,
    ) -> Result<ContainedRosterSource, CommandError> {
        let presentation = self
            .storage()
            .surfaces
            .get(&surface)
            .ok_or(CommandError::MissingSurface { surface })?;
        Ok(ContainedRosterSource {
            surface,
            contained: Arc::from(presentation.contained.clone()),
        })
    }

    /// Captures one item source and the complete state of its owning root.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the root, tabs node, or item is absent, or
    /// the node does not belong to the supplied root.
    pub fn capture_item_source(
        &self,
        root: RootId,
        owner: NodeId,
        item: ItemId,
    ) -> Result<ItemSource, CommandError> {
        let reference = self.capture_reference(root, owner, ReferenceRole::Source)?;
        let place = self.item_place_in_owner(owner, item)?;
        Ok(ItemSource {
            root,
            place,
            item,
            reference,
        })
    }

    fn item_place_in_owner(&self, owner: NodeId, item: ItemId) -> Result<ItemPlace, CommandError> {
        match self.node(owner) {
            Some(Node::Pane { item: actual }) if *actual == item => {
                Ok(ItemPlace::Pane { node: owner })
            }
            Some(Node::Tabs { items, .. }) => items
                .iter()
                .position(|tab| tab.item() == Some(item))
                .map(|index| ItemPlace::InlineTab { tabs: owner, index })
                .ok_or(CommandError::ItemNotInTabs { tabs: owner, item }),
            Some(_) => Err(CommandError::ItemNotInTabs { tabs: owner, item }),
            None => Err(CommandError::MissingNode {
                role: ReferenceRole::Source,
                node: owner,
            }),
        }
    }

    /// Captures one direct tab child and its exact root-owner authority.
    pub fn capture_tab_source(
        &self,
        root: RootId,
        tabs: NodeId,
        tab: TabKey,
    ) -> Result<TabSource, CommandError> {
        let reference = self.capture_reference(root, tabs, ReferenceRole::Source)?;
        match self.node(tabs) {
            Some(Node::Tabs { items, .. }) if items.contains(&tab) => Ok(TabSource {
                root,
                tabs,
                tab,
                reference,
            }),
            Some(Node::Tabs { .. }) => Err(CommandError::TabNotInTabs { tabs, tab }),
            Some(Node::Pane { .. } | Node::Split { .. } | Node::Grid { .. }) => {
                Err(CommandError::NodeIsNotTabs { node: tabs })
            }
            None => Err(CommandError::MissingNode {
                role: ReferenceRole::Source,
                node: tabs,
            }),
        }
    }

    /// Locates an item once and captures its owning source.
    ///
    /// Valid workspaces contain every item exactly once and every node in exactly
    /// one root. Locating the tabs node before walking root subtrees avoids the
    /// former root-by-all-nodes nested scan on pointer-click hot paths.
    pub(crate) fn capture_item_source_by_id(
        &self,
        item: ItemId,
    ) -> Result<Option<ItemSource>, CommandError> {
        let Some(location) = self.locate_item_by_id(item) else {
            return Ok(None);
        };
        self.capture_item_source(location.root, location.owner_node(), item)
            .map(Some)
    }

    /// Locates one open item without capturing a structural fingerprint.
    ///
    /// This is the lightweight query counterpart to
    /// [`Self::capture_item_source_by_id`]. Mutation paths must still capture a
    /// source fingerprint before constructing a command.
    pub(crate) fn locate_item_by_id(&self, item: ItemId) -> Option<ItemLocation> {
        let place = self.nodes().find_map(|(owner, node)| match node {
            Node::Pane { item: actual } if *actual == item => Some(ItemPlace::Pane { node: owner }),
            Node::Tabs { items, .. } => items
                .iter()
                .position(|tab| tab.item() == Some(item))
                .map(|index| ItemPlace::InlineTab { tabs: owner, index }),
            _ => None,
        })?;
        Some(ItemLocation {
            root: self.node_root(place.owner_node())?,
            place,
        })
    }

    /// Returns whether real content is visible through every containing tabs choice.
    pub(crate) fn item_is_active(&self, item: ItemId) -> bool {
        if self.item_is_hidden(item) {
            return false;
        }
        let Some(location) = self.locate_item_by_id(item) else {
            return false;
        };
        self.item_location_is_active(item, location)
    }

    /// Checks an already located placement without scanning the item roster again.
    pub(crate) fn item_location_is_active(&self, item: ItemId, location: ItemLocation) -> bool {
        if self.item_is_hidden(item) {
            return false;
        }
        if let Some(tabs) = location.tabs()
            && self.visible_tab_selection(tabs) != Some(TabKey::for_item(item))
        {
            return false;
        }
        let mut node = location.owner_node();
        while let Some(parent) = self.parent_link(location.root, node) {
            if matches!(self.node(parent.parent), Some(Node::Tabs { .. }))
                && self.visible_tab_selection(parent.parent) != Some(TabKey::for_layout(node))
            {
                return false;
            }
            node = parent.parent;
        }
        true
    }

    /// Borrows the label of one exact direct child without selecting a descendant proxy.
    pub(crate) fn tab_label(
        &self,
        tabs: NodeId,
        tab: TabKey,
    ) -> Option<crate::graph::TabLabel<'_>> {
        let Node::Tabs { items, .. } = self.node(tabs)? else {
            return None;
        };
        if !items.contains(&tab) {
            return None;
        }
        if let Some(item) = tab.item() {
            return Some(crate::graph::TabLabel::Item(item));
        }
        let node = self.node(tab.layout()?)?;
        let fallback = match node {
            Node::Tabs { .. } => "Tabs",
            Node::Split { .. } => "Split",
            Node::Grid { .. } => "Grid",
            Node::Pane { .. } => return None,
        };
        Some(crate::graph::TabLabel::Layout {
            title: node
                .layout_title()
                .filter(|title| !title.is_empty())
                .unwrap_or(fallback),
        })
    }

    /// Captures one source node and the complete state of its owning root.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the root or node is absent, or the node
    /// does not belong to the supplied root.
    pub fn capture_node_source(
        &self,
        root: RootId,
        node: NodeId,
    ) -> Result<NodeSource, CommandError> {
        Ok(NodeSource {
            root,
            node,
            reference: self.capture_reference(root, node, ReferenceRole::Source)?,
        })
    }

    /// Captures one tabs target and the complete state of its owning root.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the root or node is absent, the node is
    /// outside the root, or it is neither a bare Pane nor a Tabs container.
    pub fn capture_tab_target(
        &self,
        root: RootId,
        tabs: NodeId,
    ) -> Result<TabTarget, CommandError> {
        let reference = self.capture_reference(root, tabs, ReferenceRole::Target)?;
        let (surface, central) = self.target_owner_facts(root, tabs)?;
        let rule = self.pane_local_target_rule(root, tabs, central)?;
        Ok(TabTarget {
            root,
            tabs,
            reference,
            surface,
            central,
            rule,
        })
    }

    /// Captures one branch-local edge of a bare Pane or Tabs container.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the root or node is absent, the node
    /// does not belong to the supplied root, or it is not a supported local
    /// target. A non-central Tabs container must have a selected direct child.
    pub fn capture_inner_edge_target(
        &self,
        root: RootId,
        tabs: NodeId,
        edge: Edge,
        fraction: DockFraction,
    ) -> Result<EdgeTarget, CommandError> {
        let reference = self.capture_reference(root, tabs, ReferenceRole::Target)?;
        let (surface, central) = self.target_owner_facts(root, tabs)?;
        let rule = self.pane_local_target_rule(root, tabs, central)?;
        Ok(EdgeTarget {
            root,
            node: tabs,
            reference,
            edge,
            fraction,
            surface,
            central,
            rule,
            scope: EdgeTargetScope::Inner,
        })
    }

    /// Captures one outer edge of a complete docking root.
    ///
    /// The target node is derived from the root record, so callers cannot
    /// relabel an arbitrary branch as the root boundary.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] when the root or its topology node is absent or
    /// the root has no authoritative presentation owner.
    pub fn capture_outer_edge_target(
        &self,
        root: RootId,
        edge: Edge,
        fraction: DockFraction,
    ) -> Result<EdgeTarget, CommandError> {
        let node = self
            .storage()
            .roots
            .get(&root)
            .ok_or(CommandError::MissingRoot { root })?
            .node;
        let reference = self.capture_reference(root, node, ReferenceRole::Target)?;
        let (surface, central) = self.target_owner_facts(root, node)?;
        Ok(EdgeTarget {
            root,
            node,
            reference,
            edge,
            fraction,
            surface,
            central,
            rule: DockTargetRuleKey::Root(root),
            scope: EdgeTargetScope::Outer,
        })
    }

    pub(crate) fn pane_local_target_rule(
        &self,
        root: RootId,
        tabs: NodeId,
        central: bool,
    ) -> Result<DockTargetRuleKey, CommandError> {
        pane_local_target_rule(self.storage(), root, tabs, central)
    }

    fn target_owner_facts(
        &self,
        root: RootId,
        node: NodeId,
    ) -> Result<(SurfaceId, bool), CommandError> {
        let central = self
            .storage()
            .roots
            .get(&root)
            .ok_or(CommandError::MissingRoot { root })?
            .central
            == Some(node);
        let surface = match self.presentation_for_root(root) {
            Some(RootPresentationOwner::Main { surface })
            | Some(RootPresentationOwner::Contained { surface, .. }) => surface,
            None => {
                return Err(CommandError::Invariant {
                    stage: "capture target presentation owner",
                });
            }
        };
        Ok((surface, central))
    }

    pub(crate) fn verify_reference(
        &self,
        root: RootId,
        node: NodeId,
        expected: &RootReference,
        role: ReferenceRole,
    ) -> Result<(), CommandError> {
        let actual = self.checked_root_reference(root, node, role)?;
        if actual == expected {
            Ok(())
        } else {
            Err(CommandError::StaleNode {
                role,
                node,
                expected: expected.clone(),
                actual: actual.clone(),
            })
        }
    }

    pub(crate) fn capture_reference(
        &self,
        root: RootId,
        node: NodeId,
        role: ReferenceRole,
    ) -> Result<RootReference, CommandError> {
        self.checked_root_reference(root, node, role).cloned()
    }

    fn checked_root_reference(
        &self,
        root: RootId,
        node: NodeId,
        role: ReferenceRole,
    ) -> Result<&RootReference, CommandError> {
        if !self.storage().roots.contains_key(&root) {
            return Err(CommandError::MissingRoot { root });
        }
        if !self.storage().nodes.contains_key(node) {
            return Err(CommandError::MissingNode { role, node });
        }
        if self.node_root(node) != Some(root) {
            return Err(CommandError::NodeOutsideRoot { role, root, node });
        }
        self.root_reference(root).ok_or(CommandError::Invariant {
            stage: "capture issued root reference",
        })
    }

    pub(crate) fn root_fingerprint(&self, root: RootId) -> Result<NodeFingerprint, CommandError> {
        let owner = self
            .presentation_in_storage(root)
            .ok_or(CommandError::Invariant {
                stage: "capture root presentation owner",
            })?;
        Ok(self.build_root_index(root, owner)?.fingerprint)
    }

    fn build_root_index(
        &self,
        root: RootId,
        owner: RootPresentationOwner,
    ) -> Result<WorkspaceRootIndex, CommandError> {
        let record = self
            .storage()
            .roots
            .get(&root)
            .ok_or(CommandError::MissingRoot { root })?;
        let presentation = match owner {
            RootPresentationOwner::Main { surface } => FingerprintPresentation::Main { surface },
            RootPresentationOwner::Contained { surface, floating } => {
                FingerprintPresentation::Contained { surface, floating }
            }
        };

        #[cfg(test)]
        crate::interaction::targeting::resolver::structural_work::record_root_fingerprint_build();

        let mut nodes = Vec::new();
        let mut stack = vec![record.node];
        let mut visited = HashSet::new();
        let mut item_count = 0_usize;
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                return Err(CommandError::Invariant {
                    stage: "capture acyclic root fingerprint",
                });
            }
            let node = self
                .storage()
                .nodes
                .get(id)
                .ok_or(CommandError::MissingNode {
                    role: ReferenceRole::Source,
                    node: id,
                })?;
            #[cfg(test)]
            crate::interaction::targeting::resolver::structural_work::record_root_fingerprint_node_visit();
            stack.extend(node.child_nodes().rev().map(|(_, child)| child));
            let snapshot = match node {
                Node::Pane { item } => {
                    item_count += 1;
                    FingerprintNode::Pane {
                        item: *item,
                        hidden: self.item_is_hidden(*item),
                    }
                }
                Node::Tabs {
                    items,
                    selected,
                    mru,
                    ..
                } => {
                    item_count = item_count
                        .checked_add(items.iter().filter(|tab| tab.item().is_some()).count())
                        .ok_or(CommandError::Invariant {
                            stage: "capture root item count",
                        })?;
                    FingerprintNode::Tabs {
                        items: Arc::from(items.as_slice()),
                        hidden: if self.storage.hidden_items.is_empty() {
                            Vec::new()
                        } else {
                            items
                                .iter()
                                .copied()
                                .filter_map(|tab| tab.item())
                                .filter(|item| self.item_is_hidden(*item))
                                .collect()
                        },
                        selected: *selected,
                        mru: Arc::from(mru.as_slice()),
                    }
                }
                Node::Split {
                    axis,
                    children,
                    weights,
                    ..
                } => FingerprintNode::Split {
                    axis: *axis,
                    children: Arc::from(children.as_slice()),
                    weight_bits: weights
                        .iter()
                        .map(|weight| weight.get().to_bits())
                        .collect(),
                },
                Node::Grid {
                    shape,
                    children,
                    shares,
                    ..
                } => FingerprintNode::Grid {
                    shape: *shape,
                    children: Arc::from(children.as_slice()),
                    share_bits: shares.as_ref().map(|shares| {
                        (
                            shares.rows().map(f64::to_bits).collect(),
                            shares.columns().map(f64::to_bits).collect(),
                        )
                    }),
                },
            };
            nodes.push(NodeFingerprintRecord { id, node: snapshot });
        }

        let fingerprint = NodeFingerprint(Arc::new(RootFingerprintData {
            root,
            root_node: record.node,
            central: record.central,
            allocation: record.allocation,
            nodes,
            presentation,
        }));
        Ok(WorkspaceRootIndex {
            root_node: record.node,
            central: record.central,
            owner,
            members: Arc::new(visited),
            item_count,
            fingerprint,
            reference: self
                .root_reference(root)
                .cloned()
                .ok_or(CommandError::Invariant {
                    stage: "index issued root reference",
                })?,
        })
    }

    pub(crate) fn root_contains_node(&self, root: RootId, needle: NodeId) -> bool {
        self.root(root).is_some() && self.node_root(needle) == Some(root)
    }

    pub(crate) fn subtree_contains(&self, root: NodeId, needle: NodeId) -> bool {
        let mut stack = vec![root];
        let mut visited = HashSet::new();
        while let Some(node_id) = stack.pop() {
            if node_id == needle {
                return true;
            }
            if !visited.insert(node_id) {
                continue;
            }
            if let Some(node) = self.node(node_id) {
                stack.extend(node.child_nodes().rev().map(|(_, child)| child));
            }
        }
        false
    }

    pub(crate) fn parent_link(&self, root: RootId, needle: NodeId) -> Option<ParentLink> {
        let placement = self.placement(needle)?;
        (placement.root == root)
            .then_some(placement.parent)
            .flatten()
    }

    /// Follows current actual placement without another DFS.
    pub(crate) fn path_to_node(&self, root: RootId, needle: NodeId) -> Option<Vec<NodeId>> {
        let root_node = self.root(root)?.node;
        let mut path = Vec::new();
        let mut node = needle;
        loop {
            let placement = self.placement(node)?;
            if placement.root != root || path.len() > self.storage().nodes.len() {
                return None;
            }
            path.push(node);
            if node == root_node {
                path.reverse();
                return Some(path);
            }
            node = placement.parent?.parent;
        }
    }

    pub(crate) fn subtree_has_items(&self, root: NodeId) -> bool {
        self.items_in_subtree(root).next().is_some()
    }

    /// Visits real content along selected eligible branches in visual layout order.
    pub(crate) fn active_items_in_subtree(
        &self,
        node: NodeId,
    ) -> impl Iterator<Item = ItemId> + '_ {
        let mut pending = vec![node];
        std::iter::from_fn(move || {
            while let Some(node) = pending.pop() {
                match self.node(node)? {
                    Node::Pane { item } if !self.item_is_hidden(*item) => return Some(*item),
                    Node::Pane { .. } => {}
                    Node::Tabs { .. } => {
                        if let Some(tab) = self.visible_tab_selection(node) {
                            if let Some(item) = tab.item() {
                                return Some(item);
                            }
                            if let Some(child) = tab.layout() {
                                pending.push(child);
                            }
                        }
                    }
                    value @ (Node::Split { .. } | Node::Grid { .. }) => {
                        pending.extend(value.child_nodes().rev().map(|(_, child)| child))
                    }
                }
            }
            None
        })
    }

    /// Finds the first real content along selected eligible branches.
    pub(crate) fn first_active_item(&self, node: NodeId) -> Option<ItemId> {
        self.active_items_in_subtree(node).next()
    }

    /// Visits all owned real content in visual preorder, including hidden content.
    pub(crate) fn items_in_subtree(&self, root: NodeId) -> impl Iterator<Item = ItemId> + '_ {
        enum Visit<'a> {
            Node(NodeId),
            Tabs(&'a [TabKey]),
        }
        let mut pending = vec![Visit::Node(root)];
        let mut visited = HashSet::new();
        std::iter::from_fn(move || {
            while let Some(visit) = pending.pop() {
                let id = match visit {
                    Visit::Node(id) => id,
                    Visit::Tabs(tabs) => {
                        let Some((tab, remaining)) = tabs.split_first() else {
                            continue;
                        };
                        pending.push(Visit::Tabs(remaining));
                        if let Some(item) = tab.item() {
                            return Some(item);
                        }
                        let Some(child) = tab.layout() else {
                            continue;
                        };
                        child
                    }
                };
                if !visited.insert(id) {
                    continue;
                }
                let Some(node) = self.node(id) else {
                    continue;
                };
                match node {
                    Node::Pane { item } => return Some(*item),
                    Node::Tabs { items: tabs, .. } => {
                        pending.push(Visit::Tabs(tabs));
                    }
                    Node::Split { .. } | Node::Grid { .. } => {
                        pending.extend(
                            node.child_nodes()
                                .rev()
                                .map(|(_, child)| Visit::Node(child)),
                        );
                    }
                }
            }
            None
        })
    }

    pub(crate) fn collect_items_in_subtree(&self, root: NodeId) -> Vec<ItemId> {
        self.items_in_subtree(root).collect()
    }

    /// Returns whether closing this layout child would remove its root's central anchor.
    /// Callers still prove direct roster membership before admitting an action.
    pub(crate) fn layout_tab_contains_central(&self, root: RootId, tab: TabKey) -> bool {
        let Some(node) = tab.layout() else {
            return false;
        };
        self.root(root)
            .and_then(|record| record.central)
            .is_some_and(|central| self.subtree_contains(node, central))
    }

    /// Returns the current presentation owner of `root`.
    ///
    /// Valid workspaces have exactly one owner for every root. `None` means the
    /// root is absent or the workspace has not passed validation.
    #[must_use]
    pub fn presentation_for_root(&self, root: RootId) -> Option<RootPresentationOwner> {
        self.owned_presentation(root)
    }

    pub(crate) fn presentation_in_storage(&self, root: RootId) -> Option<RootPresentationOwner> {
        for (surface_id, surface) in &self.storage().surfaces {
            if surface.main_root == Some(root) {
                return Some(RootPresentationOwner::Main {
                    surface: *surface_id,
                });
            }
            for floating_id in &surface.contained {
                if self
                    .storage()
                    .contained_floatings
                    .get(floating_id)
                    .is_some_and(|floating| floating.root == root)
                {
                    return Some(RootPresentationOwner::Contained {
                        surface: *surface_id,
                        floating: *floating_id,
                    });
                }
            }
        }
        None
    }
}

fn pane_local_target_rule(
    storage: &crate::graph::WorkspaceData,
    root: RootId,
    tabs: NodeId,
    central: bool,
) -> Result<DockTargetRuleKey, CommandError> {
    match storage.nodes.get(tabs) {
        Some(Node::Pane { item }) => Ok(DockTargetRuleKey::Item(*item)),
        Some(Node::Tabs {
            selected: Some(item),
            ..
        }) => item
            .item()
            .map(DockTargetRuleKey::Item)
            .map_or(Ok(DockTargetRuleKey::Root(root)), Ok),
        // An empty central leaf is a deliberate docking sink, not an
        // unselected pane. It has no item identity, so root scope is the
        // only stable policy identity available to the core.
        Some(Node::Tabs {
            items,
            selected: None,
            ..
        }) if central && items.is_empty() => Ok(DockTargetRuleKey::Root(root)),
        Some(Node::Tabs { selected: None, .. }) => Err(CommandError::TargetTabsUnselected { tabs }),
        Some(Node::Split { .. } | Node::Grid { .. }) => {
            Err(CommandError::NodeIsNotTabs { node: tabs })
        }
        None => Err(CommandError::MissingNode {
            role: ReferenceRole::Target,
            node: tabs,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceIndex;
    use crate::engine::transition::WorkspaceVersion;
    use crate::graph::command::{DockFraction, Edge};
    use crate::graph::error::{CommandError, ReferenceRole};
    use crate::graph::{Axis, Node, RootRecord, SurfacePresentation, Workspace, WorkspaceBuilder};
    use crate::ids::{ItemId, NodeId, RootId, SurfaceId, WorkspaceEpoch, WorkspaceRevision};
    use crate::policy::DockTargetRuleKey;

    fn index_version(revision: u64) -> WorkspaceVersion {
        WorkspaceVersion::new(WorkspaceEpoch::new(9), WorkspaceRevision::new(revision))
    }

    fn indexed_split_fixture() -> (Workspace, RootId, NodeId, NodeId, NodeId) {
        let root = RootId::new(1);
        let mut builder = Workspace::builder();
        let left = builder.insert_node(Node::tabs([ItemId::new(1)]));
        let right = builder.insert_node(Node::tabs([ItemId::new(2)]));
        let split = builder.insert_node(
            Node::equal_split(Axis::Horizontal, [left, right])
                .expect("two children form a valid split"),
        );
        builder.set_root(root, RootRecord::new(split).with_central(left));
        builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(root));
        (
            builder.build().expect("fixture should validate"),
            root,
            split,
            left,
            right,
        )
    }

    #[test]
    fn workspace_index_rejects_cross_revision_reuse_after_topology_changes() {
        let (workspace, root, split, left, right) = indexed_split_fixture();
        let first_version = index_version(1);
        let next_version = index_version(2);
        let first_index =
            WorkspaceIndex::build(&workspace, first_version).expect("first index should derive");
        let old_target = first_index
            .capture_tab_target(&workspace, first_version, root, left)
            .expect("first target should capture");

        let mut changed = workspace.clone().into_builder();
        *changed
            .draft_mut()
            .nodes
            .get_mut(split)
            .expect("fixture split should remain live") =
            Node::equal_split(Axis::Vertical, [right, left])
                .expect("changed topology remains valid");
        let changed = changed.build().expect("changed fixture should validate");

        assert!(matches!(
            first_index.capture_tab_target(&changed, next_version, root, left),
            Err(CommandError::Invariant {
                stage: "use workspace index for a different revision"
            })
        ));
        assert!(matches!(
            changed.verify_reference(
                root,
                left,
                &old_target.reference,
                ReferenceRole::Target,
            ),
            Err(CommandError::StaleNode { node, .. }) if node == left
        ));

        let next_index =
            WorkspaceIndex::build(&changed, next_version).expect("replacement index should derive");
        let next_target = next_index
            .capture_tab_target(&changed, next_version, root, left)
            .expect("replacement target should capture");
        assert_ne!(old_target.reference, next_target.reference);
    }

    #[test]
    fn workspace_index_replaces_presentation_owner_facts_atomically() {
        let (workspace, root, _, left, _) = indexed_split_fixture();
        let first_version = index_version(1);
        let next_version = index_version(2);
        let first_index =
            WorkspaceIndex::build(&workspace, first_version).expect("first index should derive");
        let old_target = first_index
            .capture_tab_target(&workspace, first_version, root, left)
            .expect("first target should capture");
        assert_eq!(old_target.surface(), SurfaceId::new(1));

        let mut changed = workspace.clone().into_builder();
        changed.draft_mut().surfaces.clear();
        changed
            .draft_mut()
            .surfaces
            .insert(SurfaceId::new(2), SurfacePresentation::with_main(root));
        let changed = changed.build().expect("rehome fixture should validate");
        let next_index =
            WorkspaceIndex::build(&changed, next_version).expect("replacement index should derive");
        let next_target = next_index
            .capture_tab_target(&changed, next_version, root, left)
            .expect("replacement target should capture");

        assert_eq!(next_target.surface(), SurfaceId::new(2));
        assert!(matches!(
            changed.verify_reference(
                root,
                left,
                &old_target.reference,
                ReferenceRole::Target,
            ),
            Err(CommandError::StaleNode { node, .. }) if node == left
        ));
    }

    #[test]
    fn workspace_index_builds_every_root_once_and_visits_each_member_once() {
        let mut builder = Workspace::builder();
        for offset in 0_u64..4 {
            let left = builder.insert_node(Node::tabs([ItemId::new(offset * 2 + 1)]));
            let right = builder.insert_node(Node::tabs([ItemId::new(offset * 2 + 2)]));
            let split = builder.insert_node(
                Node::equal_split(Axis::Horizontal, [left, right])
                    .expect("two children form a valid split"),
            );
            let root = RootId::new(offset + 1);
            builder.set_root(root, RootRecord::new(split).with_central(left));
            builder.set_surface(
                SurfaceId::new(offset + 1),
                SurfacePresentation::with_main(root),
            );
        }
        let workspace = builder.build().expect("multi-root fixture should validate");
        let root_count = workspace.roots().count();
        let node_count = workspace.nodes().count();

        crate::interaction::targeting::resolver::structural_work::reset();
        WorkspaceIndex::build(&workspace, index_version(1))
            .expect("multi-root index should derive");
        let work = crate::interaction::targeting::resolver::structural_work::snapshot();

        assert_eq!(work.root_fingerprint_builds, root_count);
        assert_eq!(work.root_fingerprint_node_visits, node_count);
    }

    #[test]
    fn selection_rebinds_semantics_without_rebuilding_membership_or_layout() {
        let root = RootId::new(1);
        let other_root = RootId::new(2);
        let mut builder = Workspace::builder();
        let tabs = builder.insert_node(Node::tabs([ItemId::new(1), ItemId::new(2)]));
        let other = builder.insert_node(Node::tabs([ItemId::new(3)]));
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_root(other_root, RootRecord::new(other));
        builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(root));
        builder.set_surface(
            SurfaceId::new(2),
            SurfacePresentation::with_main(other_root),
        );
        let mut workspace = builder.build().unwrap();
        let index = WorkspaceIndex::build(&workspace, index_version(1)).unwrap();
        let body = std::sync::Arc::clone(workspace.root_structure(root).unwrap());
        crate::presentation::layout::measure_root_minimums(
            &workspace,
            root,
            &std::collections::BTreeMap::from([(
                tabs,
                crate::geometry::LogicalSize::new(4.0, 3.0).unwrap(),
            )]),
            crate::presentation::layout::LayoutMetrics::new(1.0).unwrap(),
        )
        .unwrap();
        let source = workspace
            .capture_item_source(root, tabs, ItemId::new(2))
            .unwrap();
        crate::graph::transaction::WorkspaceTransaction::from_commands([
            crate::graph::command::WorkspaceCommand::Select {
                source: source
                    .as_tab_source()
                    .expect("fixture owns an inline item tab"),
            },
        ])
        .apply(
            &mut workspace,
            &crate::policy::DockPolicySnapshot::default(),
        )
        .unwrap();

        crate::interaction::targeting::resolver::structural_work::reset();
        let rebound = index
            .after_selection(&workspace, index_version(2), root, tabs)
            .unwrap();
        let work = crate::interaction::targeting::resolver::structural_work::snapshot();
        assert_eq!(work.root_fingerprint_builds, 0);
        assert_eq!(work.root_fingerprint_node_visits, 0);
        assert!(std::sync::Arc::ptr_eq(
            &index.roots[&root].members,
            &rebound.roots[&root].members,
        ));
        assert!(std::sync::Arc::ptr_eq(
            &body,
            workspace.root_structure(root).unwrap(),
        ));
        assert!(std::sync::Arc::ptr_eq(
            &index.root_fingerprint(other_root).unwrap().0,
            &rebound.root_fingerprint(other_root).unwrap().0,
        ));
        let previous = index.root_fingerprint(root).unwrap();
        let current = rebound
            .at_version(index_version(2))
            .unwrap()
            .root_fingerprint(root)
            .unwrap();
        assert_ne!(previous, current);
        assert!(previous.presentation_structure_eq(current));
        assert_eq!(current, &workspace.root_fingerprint(root).unwrap());
        assert_eq!(index.version(), index_version(1));
        assert_eq!(rebound.version(), index_version(2));
    }

    #[test]
    fn indexed_membership_keeps_foreign_root_nodes_out_of_target_capture() {
        let mut builder = Workspace::builder();
        let first = builder.insert_node(Node::tabs([ItemId::new(1)]));
        let second = builder.insert_node(Node::tabs([ItemId::new(2)]));
        let first_root = RootId::new(1);
        let second_root = RootId::new(2);
        builder.set_root(first_root, RootRecord::new(first));
        builder.set_root(second_root, RootRecord::new(second));
        builder.set_surface(
            SurfaceId::new(1),
            SurfacePresentation::with_main(first_root),
        );
        builder.set_surface(
            SurfaceId::new(2),
            SurfacePresentation::with_main(second_root),
        );
        let workspace = builder.build().unwrap();
        let version = index_version(1);
        let index = WorkspaceIndex::build(&workspace, version).unwrap();
        let view = index.at_version(version).unwrap();

        assert!(view.root_contains_node(first_root, first));
        assert!(view.root_contains_node(second_root, second));
        assert!(!view.root_contains_node(first_root, second));
        assert!(matches!(
            index.capture_tab_target(&workspace, version, first_root, second),
            Err(CommandError::NodeOutsideRoot {
                role: ReferenceRole::Target,
                root,
                node,
            }) if root == first_root && node == second
        ));
    }

    #[test]
    fn indexed_scene_targets_preserve_checked_capture_semantics() {
        let (workspace, root, _, left, _) = indexed_split_fixture();
        let version = index_version(1);
        let index = WorkspaceIndex::build(&workspace, version).expect("index should derive");
        let fraction = DockFraction::new(0.3).expect("fixture fraction should be valid");

        assert_eq!(
            index
                .capture_tab_target(&workspace, version, root, left)
                .expect("indexed tab target should capture"),
            workspace
                .capture_tab_target(root, left)
                .expect("checked tab target should capture"),
        );
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            assert_eq!(
                index
                    .capture_inner_edge_target(&workspace, version, root, left, edge, fraction)
                    .expect("indexed inner target should capture"),
                workspace
                    .capture_inner_edge_target(root, left, edge, fraction)
                    .expect("checked inner target should capture"),
            );
            assert_eq!(
                index
                    .capture_outer_edge_target(&workspace, version, root, edge, fraction)
                    .expect("indexed outer target should capture"),
                workspace
                    .capture_outer_edge_target(root, edge, fraction)
                    .expect("checked outer target should capture"),
            );
        }
    }

    #[test]
    fn pane_local_target_capture_accepts_only_an_empty_central_leaf_without_selection() {
        let root = RootId::new(1);
        let surface = SurfaceId::new(1);
        let mut builder = WorkspaceBuilder::new();
        let tabs = builder.insert_node(Node::tabs_with_selection([ItemId::new(1)], None));
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_surface(surface, SurfacePresentation::with_main(root));
        let fraction = DockFraction::new(0.5).expect("test fraction is valid");

        assert!(matches!(
            super::pane_local_target_rule(builder.draft(), root, tabs, false),
            Err(CommandError::TargetTabsUnselected { tabs: actual }) if actual == tabs
        ));
        assert!(builder.build().is_err());

        let mut builder = WorkspaceBuilder::new();
        let empty_central = builder.insert_node(Node::tabs([]));
        builder.set_root(
            root,
            RootRecord::new(empty_central).with_central(empty_central),
        );
        builder.set_surface(surface, SurfacePresentation::with_main(root));
        let workspace = builder.build().unwrap();
        assert_eq!(
            workspace
                .capture_tab_target(root, empty_central)
                .unwrap()
                .rule(),
            DockTargetRuleKey::Root(root)
        );
        assert_eq!(
            workspace
                .capture_inner_edge_target(root, empty_central, Edge::Left, fraction)
                .unwrap()
                .rule(),
            DockTargetRuleKey::Root(root)
        );
        assert_eq!(
            workspace
                .capture_outer_edge_target(root, Edge::Left, fraction)
                .unwrap()
                .rule(),
            DockTargetRuleKey::Root(root)
        );
    }

    #[test]
    fn item_source_lookup_finds_the_exact_owner_across_multiple_roots() {
        let first_root = RootId::new(1);
        let second_root = RootId::new(2);
        let mut builder = Workspace::builder();
        let first_tabs = builder.insert_node(Node::tabs([ItemId::new(1)]));
        let second_tabs = builder.insert_node(Node::tabs([ItemId::new(2), ItemId::new(3)]));
        builder.set_root(first_root, RootRecord::new(first_tabs));
        builder.set_root(second_root, RootRecord::new(second_tabs));
        builder.set_surface(
            SurfaceId::new(1),
            SurfacePresentation::with_main(first_root),
        );
        builder.set_surface(
            SurfaceId::new(2),
            SurfacePresentation::with_main(second_root),
        );
        let workspace = builder.build().expect("test workspace is valid");

        let source = workspace
            .capture_item_source_by_id(ItemId::new(3))
            .expect("source capture is valid")
            .expect("item is present");
        assert_eq!(source.root(), second_root);
        assert_eq!(source.tabs(), Some(second_tabs));
        assert_eq!(source.item(), ItemId::new(3));
        assert_eq!(
            workspace
                .capture_item_source_by_id(ItemId::new(99))
                .expect("a missing item is not an invalid capture"),
            None
        );
    }
}
