//! Renderer-neutral persistence payloads for validated workspaces.
//!
//! Snapshot node identities are local integers, not serialized [`crate::ids::NodeId`] values.
//! Restoring a snapshot always allocates fresh generational runtime identities and publishes only
//! a fully validated candidate.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::geometry::{GeometryError, LogicalRect};
use crate::graph::canonical::CanonicalizationError;
use crate::graph::validation::WorkspaceValidationErrors;
use crate::graph::{
    Axis, ContainedFloating, InvalidSplitWeight, Node, RootRecord, SplitWeight,
    SurfacePresentation, Workspace, WorkspaceBuildError, WorkspaceBuilder,
};
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::model::{DockspaceGridError, DockspaceGridShape, DockspaceGridShares};

#[cfg(test)]
mod visibility_tests;

/// A complete renderer-neutral workspace snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSnapshot {
    /// Flat node arena with snapshot-local integer identities.
    pub nodes: Vec<SnapshotNodeRecord>,
    /// Stable docking roots.
    pub roots: Vec<SnapshotRootRecord>,
    /// Stable presentation surfaces.
    pub surfaces: Vec<SnapshotSurfaceRecord>,
    /// Contained-floating presentations.
    pub contained_floatings: Vec<SnapshotContainedFloatingRecord>,
    /// Owned items excluded from presentation while retaining their placement.
    #[serde(default)]
    pub hidden_items: Vec<u64>,
}

/// One node and its snapshot-local integer identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotNodeRecord {
    /// Snapshot-local node identity.
    pub id: u64,
    /// Persisted node payload.
    pub node: SnapshotNode,
}

/// Persisted node payload using only durable or snapshot-local identities.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotNode {
    /// A bare real application pane without tabs chrome.
    Pane {
        /// Stable real application item identity.
        item: u64,
    },
    /// Ordered dockable items sharing one content rectangle.
    Tabs {
        /// Stable item identities in tab order.
        items: Vec<SnapshotTabKey>,
        /// Selected direct child identity.
        #[serde(deserialize_with = "deserialize_required_option")]
        selected: Option<SnapshotTabKey>,
        /// Complete direct-child permutation in most-recent-first order.
        mru: Vec<SnapshotTabKey>,
        /// Optional explicit title for this structural container.
        #[serde(deserialize_with = "deserialize_required_option")]
        layout_title: Option<String>,
    },
    /// Ordered N-ary split children.
    Split {
        /// Split layout direction.
        axis: SnapshotAxis,
        /// Snapshot-local child node identities.
        children: Vec<u64>,
        /// Exact normalized shares from the runtime model.
        weights: Vec<f64>,
        /// Optional explicit title for this structural container.
        #[serde(deserialize_with = "deserialize_required_option")]
        layout_title: Option<String>,
    },
    /// A real row-major grid with shared row and column tracks.
    Grid {
        /// Explicit dimensions or the durable Auto declaration.
        shape: SnapshotGridShape,
        /// Snapshot-local identities of every real child node.
        children: Vec<u64>,
        /// Complete stored axes, or explicit null for equal default shares.
        #[serde(deserialize_with = "deserialize_required_option")]
        shares: Option<SnapshotGridShares>,
        /// Optional title, encoded as an explicit required nullable field.
        #[serde(deserialize_with = "deserialize_required_option")]
        layout_title: Option<String>,
    },
}

/// Persisted viewport-independent grid dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotGridShape {
    /// Explicit positive dimensions checked before platform-size conversion.
    Fixed {
        /// Persisted positive row count.
        rows: u64,
        /// Persisted positive column count.
        columns: u64,
    },
    /// Dimensions will be resolved from the offered rectangle.
    #[serde(deserialize_with = "deserialize_auto_grid_shape")]
    Auto,
}

fn deserialize_auto_grid_shape<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Serde's internally tagged unit variant decoder ignores unknown fields.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AutoGridShape {}

    AutoGridShape::deserialize(deserializer).map(|_| ())
}

/// Exact independently normalized row and column shares from the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotGridShares {
    /// Top-to-bottom normalized row shares.
    pub rows: Vec<f64>,
    /// Left-to-right normalized column shares.
    pub columns: Vec<f64>,
}

/// One direct child using a stable content ID or a snapshot-local layout ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SnapshotTabKey {
    /// One real application item.
    Item(u64),
    /// One complete child layout.
    Layout(u64),
}

impl SnapshotTabKey {
    pub(crate) const fn item(self) -> Option<u64> {
        match self {
            Self::Item(item) => Some(item),
            Self::Layout(_) => None,
        }
    }
}

/// Persisted split direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotAxis {
    /// Children are ordered from left to right.
    Horizontal,
    /// Children are ordered from top to bottom.
    Vertical,
}

/// Persisted docking root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRootRecord {
    /// Stable root identity.
    pub id: u64,
    /// Snapshot-local topology root node.
    pub node: u64,
    /// Snapshot-local central tabs leaf.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub central: Option<u64>,
    /// Explicit redistribution policy; empty-anchor retention is independent.
    pub allocation: crate::model::RootAllocation,
}

/// Persisted presentation surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSurfaceRecord {
    /// Stable surface identity.
    pub id: u64,
    /// Stable root occupying the main dock area, when present.
    #[serde(deserialize_with = "deserialize_required_option")]
    pub main_root: Option<u64>,
    /// Stable contained-floating identities in roster order.
    pub contained: Vec<u64>,
}

pub(crate) fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Persisted contained-floating presentation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContainedFloatingRecord {
    /// Stable floating presentation identity.
    pub id: u64,
    /// Stable presented root identity.
    pub root: u64,
    /// Logical bounds relative to the owning surface.
    pub rect: SnapshotLogicalRect,
}

/// Persisted logical rectangle represented by scalar components.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotLogicalRect {
    /// Minimum horizontal coordinate.
    pub x: f64,
    /// Minimum vertical coordinate.
    pub y: f64,
    /// Non-negative width.
    pub width: f64,
    /// Non-negative height.
    pub height: f64,
}

/// Stable kind of entity declared by a snapshot record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotEntityKind {
    /// Snapshot-local graph node.
    Node,
    /// Stable docking root.
    Root,
    /// Stable presentation surface.
    Surface,
    /// Stable contained-floating presentation.
    ContainedFloating,
}

/// Record which owns a persisted reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReferenceOwner {
    /// A node record.
    Node(u64),
    /// A root record.
    Root(u64),
    /// A surface record.
    Surface(u64),
    /// A contained-floating record.
    ContainedFloating(u64),
}

/// Failure to capture a validated runtime workspace.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SnapshotCaptureError {
    /// The runtime workspace did not satisfy its durable invariants.
    #[error("cannot snapshot an invalid workspace: {0}")]
    InvalidWorkspace(#[source] WorkspaceValidationErrors),
    /// This platform exposed more nodes than the `u64` snapshot identity space can represent.
    #[error("workspace has too many nodes for the snapshot identity space")]
    NodeIdentitySpaceExhausted,
    /// A validated runtime record unexpectedly referenced an unindexed node.
    #[error("runtime record references unindexed node {node:?}")]
    UnindexedRuntimeNode {
        /// Runtime identity absent from the capture index.
        node: NodeId,
    },
}

/// Failure to restore a workspace snapshot.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SnapshotRestoreError {
    /// A fixed dimension is zero or cannot be represented by this platform.
    #[error("node {node} has invalid grid {axis:?} dimension {value}")]
    InvalidGridDimension {
        node: u64,
        axis: SnapshotAxis,
        value: u64,
    },
    /// The grid roster, shape, or persistent shared tracks are invalid.
    #[error("node {node} has invalid grid values: {source}")]
    InvalidGrid {
        node: u64,
        source: DockspaceGridError,
    },
    /// Two records declare the same identity.
    #[error("duplicate {kind:?} snapshot identity {id}")]
    DuplicateIdentity {
        /// Kind of duplicated record.
        kind: SnapshotEntityKind,
        /// Repeated numeric identity.
        id: u64,
    },
    /// A record points at an identity absent from the corresponding record set.
    #[error("{owner:?} references unknown {kind:?} identity {id}")]
    UnknownReference {
        /// Record containing the reference.
        owner: SnapshotReferenceOwner,
        /// Kind of referenced entity.
        kind: SnapshotEntityKind,
        /// Missing numeric identity.
        id: u64,
    },
    /// An application registry could not resolve a persisted item identity.
    #[error("application registry does not contain item {item}")]
    UnknownItem {
        /// Missing stable application item identity.
        item: ItemId,
    },
    /// One item was repeated in the persistent hidden item set.
    #[error("hidden item {item} is declared more than once")]
    DuplicateHiddenItem {
        /// Repeated hidden item identity.
        item: ItemId,
    },
    /// Visibility metadata references content absent from all tabs leaves.
    #[error("hidden item {item} is not owned by the snapshot")]
    UnknownHiddenItem {
        /// Unknown hidden item identity.
        item: ItemId,
    },
    /// A contained-floating record has invalid logical geometry.
    #[error("floating presentation {floating} has invalid geometry: {source}")]
    InvalidFloatingGeometry {
        /// Stable floating presentation identity.
        floating: u64,
        /// Geometry validation failure.
        source: GeometryError,
    },
    /// A split contains a non-finite or non-positive scalar weight.
    #[error("node {node} has invalid split weight at index {index}: {source}")]
    InvalidSplitWeight {
        /// Snapshot-local split node identity.
        node: u64,
        /// Invalid weight position.
        index: usize,
        /// Scalar validation failure.
        source: InvalidSplitWeight,
    },
    /// An internal draft operation failed while assembling the candidate.
    #[error("could not assemble workspace candidate: {0}")]
    Assembly(#[source] WorkspaceBuildError),
    /// The fully assembled candidate violates strict workspace invariants.
    #[error("restored workspace candidate is invalid: {0}")]
    InvalidWorkspace(#[source] WorkspaceValidationErrors),
    /// The strictly validated candidate could not be finalized.
    #[error("could not finalize restored workspace candidate: {0}")]
    Build(#[source] CanonicalizationError),
}

impl WorkspaceSnapshot {
    /// Captures a validated runtime workspace as a deterministic flat snapshot.
    ///
    /// Runtime [`NodeId`] values are remapped to contiguous snapshot-local integers. Stable item,
    /// root, surface, and floating identities retain their numeric representation.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotCaptureError`] when the runtime workspace is invalid, its node count does
    /// not fit the snapshot identity space, or a runtime node reference cannot be indexed.
    pub fn capture(workspace: &Workspace) -> Result<Self, SnapshotCaptureError> {
        workspace
            .validate()
            .map_err(SnapshotCaptureError::InvalidWorkspace)?;

        let node_ids = capture_node_ids(workspace)?;
        let nodes = workspace
            .nodes()
            .map(|(id, node)| capture_node(id, node, &node_ids))
            .collect::<Result<Vec<_>, _>>()?;
        let roots = workspace
            .roots()
            .map(|(id, root)| {
                Ok(SnapshotRootRecord {
                    id: id.get(),
                    node: captured_id(&node_ids, root.node)?,
                    central: root
                        .central
                        .map(|central| captured_id(&node_ids, central))
                        .transpose()?,
                    allocation: root.allocation,
                })
            })
            .collect::<Result<Vec<_>, SnapshotCaptureError>>()?;
        let surfaces = workspace
            .surfaces()
            .map(|(id, surface)| SnapshotSurfaceRecord {
                id: id.get(),
                main_root: surface.main_root.map(RootId::get),
                contained: surface.contained.iter().map(|id| id.get()).collect(),
            })
            .collect();
        let contained_floatings = workspace
            .contained_floatings()
            .map(|(id, floating)| SnapshotContainedFloatingRecord {
                id: id.get(),
                root: floating.root.get(),
                rect: SnapshotLogicalRect::from(floating.rect),
            })
            .collect();
        Ok(Self {
            nodes,
            roots,
            surfaces,
            contained_floatings,
            hidden_items: workspace.hidden_items().map(ItemId::get).collect(),
        })
    }

    /// Builds a fresh workspace candidate after checking every persisted item.
    ///
    /// `contains_item` is a read-only registry query called exactly once for every
    /// distinct item identity, after all graph and presentation validation succeeds.
    /// Every identity is queried even when an earlier one is missing. Returning
    /// `false` rejects the snapshot as stale application state.
    /// Submit the resulting candidate through an explicit
    /// [`crate::engine::EngineInput::ReplaceWorkspace`] reducer tick when
    /// publishing into an existing engine. The replacement advances the workspace
    /// epoch and makes every old runtime [`NodeId`] stale.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] for duplicate identities, unknown
    /// references or items, invalid scalar data, or any strict workspace invariant violation.
    pub fn build_candidate(
        &self,
        contains_item: impl Fn(ItemId) -> bool,
    ) -> Result<Workspace, SnapshotRestoreError> {
        let index = SnapshotIndex::new(self)?;
        preflight_references_and_scalars(self, &index)?;

        let mut builder = Workspace::builder();
        let runtime_nodes = allocate_runtime_nodes(self, &mut builder)?;
        populate_runtime_nodes(self, &runtime_nodes, &mut builder)?;
        populate_roots(self, &runtime_nodes, &mut builder)?;
        populate_surfaces(self, &mut builder);
        populate_contained_floatings(self, &mut builder)?;
        builder.set_hidden_items(self.hidden_items.iter().copied().map(ItemId::new).collect());

        builder
            .validate()
            .map_err(SnapshotRestoreError::InvalidWorkspace)?;
        let candidate = builder.build().map_err(SnapshotRestoreError::Build)?;
        validate_registered_items(self, contains_item)?;
        Ok(candidate)
    }
}

impl From<Axis> for SnapshotAxis {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Horizontal => Self::Horizontal,
            Axis::Vertical => Self::Vertical,
        }
    }
}

impl From<SnapshotAxis> for Axis {
    fn from(axis: SnapshotAxis) -> Self {
        match axis {
            SnapshotAxis::Horizontal => Self::Horizontal,
            SnapshotAxis::Vertical => Self::Vertical,
        }
    }
}

impl From<LogicalRect> for SnapshotLogicalRect {
    fn from(rect: LogicalRect) -> Self {
        Self {
            x: rect.x(),
            y: rect.y(),
            width: rect.width(),
            height: rect.height(),
        }
    }
}

#[derive(Debug)]
struct SnapshotIndex {
    nodes: HashSet<u64>,
    roots: HashSet<u64>,
    contained_floatings: HashSet<u64>,
}

impl SnapshotIndex {
    fn new(snapshot: &WorkspaceSnapshot) -> Result<Self, SnapshotRestoreError> {
        unique_ids(
            snapshot.surfaces.iter().map(|record| record.id),
            SnapshotEntityKind::Surface,
        )?;

        Ok(Self {
            nodes: unique_ids(
                snapshot.nodes.iter().map(|record| record.id),
                SnapshotEntityKind::Node,
            )?,
            roots: unique_ids(
                snapshot.roots.iter().map(|record| record.id),
                SnapshotEntityKind::Root,
            )?,
            contained_floatings: unique_ids(
                snapshot.contained_floatings.iter().map(|record| record.id),
                SnapshotEntityKind::ContainedFloating,
            )?,
        })
    }
}

fn unique_ids(
    ids: impl IntoIterator<Item = u64>,
    kind: SnapshotEntityKind,
) -> Result<HashSet<u64>, SnapshotRestoreError> {
    let mut unique = HashSet::new();
    for id in ids {
        if !unique.insert(id) {
            return Err(SnapshotRestoreError::DuplicateIdentity { kind, id });
        }
    }
    Ok(unique)
}

fn capture_node_ids(workspace: &Workspace) -> Result<HashMap<NodeId, u64>, SnapshotCaptureError> {
    let mut node_ids = HashMap::with_capacity(workspace.nodes().count());
    for (index, (node, _)) in workspace.nodes().enumerate() {
        let snapshot_id =
            u64::try_from(index).map_err(|_| SnapshotCaptureError::NodeIdentitySpaceExhausted)?;
        node_ids.insert(node, snapshot_id);
    }
    Ok(node_ids)
}

fn captured_id(node_ids: &HashMap<NodeId, u64>, node: NodeId) -> Result<u64, SnapshotCaptureError> {
    node_ids
        .get(&node)
        .copied()
        .ok_or(SnapshotCaptureError::UnindexedRuntimeNode { node })
}

fn capture_node(
    id: NodeId,
    node: &Node,
    node_ids: &HashMap<NodeId, u64>,
) -> Result<SnapshotNodeRecord, SnapshotCaptureError> {
    let node = match node {
        Node::Pane { item } => SnapshotNode::Pane { item: item.get() },
        Node::Tabs {
            items,
            selected,
            mru,
            layout_title,
        } => SnapshotNode::Tabs {
            layout_title: layout_title.as_ref().map(ToString::to_string),
            items: items
                .iter()
                .copied()
                .map(|tab| capture_tab_key(tab, node_ids))
                .collect::<Result<_, _>>()?,
            selected: selected
                .map(|tab| capture_tab_key(tab, node_ids))
                .transpose()?,
            mru: mru
                .iter()
                .copied()
                .map(|tab| capture_tab_key(tab, node_ids))
                .collect::<Result<_, _>>()?,
        },
        Node::Split {
            axis,
            children,
            weights,
            layout_title,
        } => SnapshotNode::Split {
            layout_title: layout_title.as_ref().map(ToString::to_string),
            axis: (*axis).into(),
            children: children
                .iter()
                .map(|child| captured_id(node_ids, *child))
                .collect::<Result<Vec<_>, _>>()?,
            weights: weights.iter().map(|weight| weight.get()).collect(),
        },
        Node::Grid {
            shape,
            children,
            shares,
            layout_title,
        } => SnapshotNode::Grid {
            shape: match shape {
                DockspaceGridShape::Fixed { rows, columns } => SnapshotGridShape::Fixed {
                    rows: rows.get() as u64,
                    columns: columns.get() as u64,
                },
                DockspaceGridShape::Auto => SnapshotGridShape::Auto,
            },
            children: children
                .iter()
                .map(|child| captured_id(node_ids, *child))
                .collect::<Result<_, _>>()?,
            shares: shares.as_ref().map(|shares| SnapshotGridShares {
                rows: shares.rows().collect(),
                columns: shares.columns().collect(),
            }),
            layout_title: layout_title.as_ref().map(ToString::to_string),
        },
    };
    Ok(SnapshotNodeRecord {
        id: captured_id(node_ids, id)?,
        node,
    })
}

fn capture_tab_key(
    tab: TabKey,
    node_ids: &HashMap<NodeId, u64>,
) -> Result<SnapshotTabKey, SnapshotCaptureError> {
    match tab.0 {
        crate::ids::TabContent::Item(item) => Ok(SnapshotTabKey::Item(item.get())),
        crate::ids::TabContent::Layout(node) => {
            captured_id(node_ids, node).map(SnapshotTabKey::Layout)
        }
    }
}

fn restore_tab_key(
    tab: SnapshotTabKey,
    owner: SnapshotReferenceOwner,
    runtime_nodes: &HashMap<u64, NodeId>,
) -> Result<TabKey, SnapshotRestoreError> {
    match tab {
        SnapshotTabKey::Item(item) => Ok(TabKey::for_item(ItemId::new(item))),
        SnapshotTabKey::Layout(node) => {
            runtime_node(runtime_nodes, owner, node).map(TabKey::for_layout)
        }
    }
}

fn preflight_references_and_scalars(
    snapshot: &WorkspaceSnapshot,
    index: &SnapshotIndex,
) -> Result<(), SnapshotRestoreError> {
    if !snapshot.hidden_items.is_empty() {
        let owned_items: HashSet<_> = snapshot
            .nodes
            .iter()
            .flat_map(|record| match &record.node {
                SnapshotNode::Pane { item } => vec![*item],
                SnapshotNode::Tabs { items, .. } => {
                    items.iter().filter_map(|tab| tab.item()).collect()
                }
                SnapshotNode::Split { .. } | SnapshotNode::Grid { .. } => Vec::new(),
            })
            .collect();
        let mut hidden_items = HashSet::with_capacity(snapshot.hidden_items.len());
        for &item in &snapshot.hidden_items {
            if !hidden_items.insert(item) {
                return Err(SnapshotRestoreError::DuplicateHiddenItem {
                    item: ItemId::new(item),
                });
            }
            if !owned_items.contains(&item) {
                return Err(SnapshotRestoreError::UnknownHiddenItem {
                    item: ItemId::new(item),
                });
            }
        }
    }
    for record in &snapshot.nodes {
        if let SnapshotNode::Grid {
            shape,
            children,
            shares,
            ..
        } = &record.node
        {
            for &child in children {
                require_reference(
                    &index.nodes,
                    SnapshotReferenceOwner::Node(record.id),
                    SnapshotEntityKind::Node,
                    child,
                )?;
            }
            restore_grid_values(record.id, *shape, shares.as_ref(), children.len())?;
        }
        if let SnapshotNode::Tabs {
            items,
            selected,
            mru,
            ..
        } = &record.node
        {
            for tab in items.iter().chain(selected.iter()).chain(mru.iter()) {
                if let SnapshotTabKey::Layout(node) = tab {
                    require_reference(
                        &index.nodes,
                        SnapshotReferenceOwner::Node(record.id),
                        SnapshotEntityKind::Node,
                        *node,
                    )?;
                }
            }
        }
        if let SnapshotNode::Split {
            children, weights, ..
        } = &record.node
        {
            for child in children {
                require_reference(
                    &index.nodes,
                    SnapshotReferenceOwner::Node(record.id),
                    SnapshotEntityKind::Node,
                    *child,
                )?;
            }
            for (weight_index, weight) in weights.iter().copied().enumerate() {
                SplitWeight::new(weight).map_err(|source| {
                    SnapshotRestoreError::InvalidSplitWeight {
                        node: record.id,
                        index: weight_index,
                        source,
                    }
                })?;
            }
        }
    }

    for root in &snapshot.roots {
        let owner = SnapshotReferenceOwner::Root(root.id);
        require_reference(&index.nodes, owner, SnapshotEntityKind::Node, root.node)?;
        if let Some(central) = root.central {
            require_reference(&index.nodes, owner, SnapshotEntityKind::Node, central)?;
        }
    }

    for surface in &snapshot.surfaces {
        let owner = SnapshotReferenceOwner::Surface(surface.id);
        if let Some(main_root) = surface.main_root {
            require_reference(&index.roots, owner, SnapshotEntityKind::Root, main_root)?;
        }
        for floating in &surface.contained {
            require_reference(
                &index.contained_floatings,
                owner,
                SnapshotEntityKind::ContainedFloating,
                *floating,
            )?;
        }
    }

    for floating in &snapshot.contained_floatings {
        let owner = SnapshotReferenceOwner::ContainedFloating(floating.id);
        require_reference(&index.roots, owner, SnapshotEntityKind::Root, floating.root)?;
        restore_rect(floating)?;
    }
    Ok(())
}

fn require_reference(
    known: &HashSet<u64>,
    owner: SnapshotReferenceOwner,
    kind: SnapshotEntityKind,
    id: u64,
) -> Result<(), SnapshotRestoreError> {
    if known.contains(&id) {
        Ok(())
    } else {
        Err(SnapshotRestoreError::UnknownReference { owner, kind, id })
    }
}

fn validate_registered_items(
    snapshot: &WorkspaceSnapshot,
    contains_item: impl Fn(ItemId) -> bool,
) -> Result<(), SnapshotRestoreError> {
    let mut items = BTreeSet::new();
    for record in &snapshot.nodes {
        if let SnapshotNode::Pane { item } = record.node {
            items.insert(item);
        }
        if let SnapshotNode::Tabs {
            items: tab_items,
            selected,
            mru,
            ..
        } = &record.node
        {
            items.extend(tab_items.iter().copied().filter_map(SnapshotTabKey::item));
            items.extend(selected.iter().copied().filter_map(SnapshotTabKey::item));
            items.extend(mru.iter().copied().filter_map(SnapshotTabKey::item));
        }
    }

    let mut first_missing = None;
    for item in items.into_iter().map(ItemId::new) {
        if !contains_item(item) && first_missing.is_none() {
            first_missing = Some(item);
        }
    }
    first_missing.map_or(Ok(()), |item| {
        Err(SnapshotRestoreError::UnknownItem { item })
    })
}

fn allocate_runtime_nodes(
    snapshot: &WorkspaceSnapshot,
    builder: &mut WorkspaceBuilder,
) -> Result<HashMap<u64, NodeId>, SnapshotRestoreError> {
    let mut runtime_nodes = HashMap::with_capacity(snapshot.nodes.len());
    for record in &snapshot.nodes {
        let runtime = builder.insert_node(Node::tabs([]));
        if runtime_nodes.insert(record.id, runtime).is_some() {
            return Err(SnapshotRestoreError::DuplicateIdentity {
                kind: SnapshotEntityKind::Node,
                id: record.id,
            });
        }
    }
    Ok(runtime_nodes)
}

fn populate_runtime_nodes(
    snapshot: &WorkspaceSnapshot,
    runtime_nodes: &HashMap<u64, NodeId>,
    builder: &mut WorkspaceBuilder,
) -> Result<(), SnapshotRestoreError> {
    for record in &snapshot.nodes {
        let runtime = runtime_node(
            runtime_nodes,
            SnapshotReferenceOwner::Node(record.id),
            record.id,
        )?;
        let node = restore_node(record, runtime_nodes)?;
        builder
            .replace_node(runtime, node)
            .map_err(SnapshotRestoreError::Assembly)?;
    }
    Ok(())
}

fn restore_node(
    record: &SnapshotNodeRecord,
    runtime_nodes: &HashMap<u64, NodeId>,
) -> Result<Node, SnapshotRestoreError> {
    match &record.node {
        SnapshotNode::Pane { item } => Ok(Node::pane(ItemId::new(*item))),
        SnapshotNode::Tabs {
            items,
            selected,
            mru,
            layout_title,
        } => Ok(Node::Tabs {
            layout_title: layout_title.as_deref().map(Arc::from),
            items: items
                .iter()
                .copied()
                .map(|tab| {
                    restore_tab_key(tab, SnapshotReferenceOwner::Node(record.id), runtime_nodes)
                })
                .collect::<Result<_, _>>()?,
            selected: selected
                .map(|tab| {
                    restore_tab_key(tab, SnapshotReferenceOwner::Node(record.id), runtime_nodes)
                })
                .transpose()?,
            mru: mru
                .iter()
                .copied()
                .map(|tab| {
                    restore_tab_key(tab, SnapshotReferenceOwner::Node(record.id), runtime_nodes)
                })
                .collect::<Result<_, _>>()?,
        }),
        SnapshotNode::Split {
            axis,
            children,
            weights,
            layout_title,
        } => {
            let children = children
                .iter()
                .copied()
                .map(|child| {
                    runtime_node(
                        runtime_nodes,
                        SnapshotReferenceOwner::Node(record.id),
                        child,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let weights = weights
                .iter()
                .copied()
                .enumerate()
                .map(|(index, weight)| {
                    SplitWeight::new(weight).map_err(|source| {
                        SnapshotRestoreError::InvalidSplitWeight {
                            node: record.id,
                            index,
                            source,
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Node::Split {
                axis: (*axis).into(),
                children,
                weights,
                layout_title: layout_title.as_deref().map(Arc::from),
            })
        }
        SnapshotNode::Grid {
            shape,
            children,
            shares,
            layout_title,
        } => {
            let (shape, shares) =
                restore_grid_values(record.id, *shape, shares.as_ref(), children.len())?;
            let children = children
                .iter()
                .copied()
                .map(|child| {
                    runtime_node(
                        runtime_nodes,
                        SnapshotReferenceOwner::Node(record.id),
                        child,
                    )
                })
                .collect::<Result<_, _>>()?;
            Ok(Node::Grid {
                shape,
                children,
                shares,
                layout_title: layout_title.as_deref().map(Arc::from),
            })
        }
    }
}

fn restore_grid_values(
    node: u64,
    shape: SnapshotGridShape,
    shares: Option<&SnapshotGridShares>,
    children: usize,
) -> Result<(DockspaceGridShape, Option<DockspaceGridShares>), SnapshotRestoreError> {
    let dimension = |value: u64, axis| {
        usize::try_from(value)
            .ok()
            .and_then(std::num::NonZeroUsize::new)
            .ok_or(SnapshotRestoreError::InvalidGridDimension { node, axis, value })
    };
    let shape = match shape {
        SnapshotGridShape::Fixed { rows, columns } => DockspaceGridShape::Fixed {
            rows: dimension(rows, SnapshotAxis::Vertical)?,
            columns: dimension(columns, SnapshotAxis::Horizontal)?,
        },
        SnapshotGridShape::Auto => DockspaceGridShape::Auto,
    };
    let invalid = |source| SnapshotRestoreError::InvalidGrid { node, source };
    shape.validate_children(children).map_err(invalid)?;
    let shares = shares
        .map(|shares| {
            let weights = |values: &[f64], rows: bool| {
                values
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, value)| {
                        SplitWeight::new(value).map_err(|source| {
                            let source =
                                crate::graph::InvalidSplitWeights::Invalid { index, source };
                            if rows {
                                DockspaceGridError::InvalidRowShares { source }
                            } else {
                                DockspaceGridError::InvalidColumnShares { source }
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            };
            let shares = DockspaceGridShares::from_normalized(
                weights(&shares.rows, true)?,
                weights(&shares.columns, false)?,
            )?;
            shares.validate_for(shape, children)?;
            Ok::<_, DockspaceGridError>(shares)
        })
        .transpose()
        .map_err(invalid)?;
    Ok((shape, shares))
}

fn populate_roots(
    snapshot: &WorkspaceSnapshot,
    runtime_nodes: &HashMap<u64, NodeId>,
    builder: &mut WorkspaceBuilder,
) -> Result<(), SnapshotRestoreError> {
    for root in &snapshot.roots {
        let owner = SnapshotReferenceOwner::Root(root.id);
        let node = runtime_node(runtime_nodes, owner, root.node)?;
        let central = root
            .central
            .map(|central| runtime_node(runtime_nodes, owner, central))
            .transpose()?;
        builder.set_root(
            RootId::new(root.id),
            RootRecord {
                node,
                central,
                allocation: root.allocation,
            },
        );
    }
    Ok(())
}

fn populate_surfaces(snapshot: &WorkspaceSnapshot, builder: &mut WorkspaceBuilder) {
    for surface in &snapshot.surfaces {
        builder.set_surface(
            SurfaceId::new(surface.id),
            SurfacePresentation {
                main_root: surface.main_root.map(RootId::new),
                contained: surface
                    .contained
                    .iter()
                    .copied()
                    .map(FloatingPresentationId::new)
                    .collect(),
            },
        );
    }
}

fn populate_contained_floatings(
    snapshot: &WorkspaceSnapshot,
    builder: &mut WorkspaceBuilder,
) -> Result<(), SnapshotRestoreError> {
    for floating in &snapshot.contained_floatings {
        builder.set_contained_floating(
            FloatingPresentationId::new(floating.id),
            ContainedFloating::new(RootId::new(floating.root), restore_rect(floating)?),
        );
    }
    Ok(())
}

fn restore_rect(
    floating: &SnapshotContainedFloatingRecord,
) -> Result<LogicalRect, SnapshotRestoreError> {
    LogicalRect::new(
        floating.rect.x,
        floating.rect.y,
        floating.rect.width,
        floating.rect.height,
    )
    .map_err(|source| SnapshotRestoreError::InvalidFloatingGeometry {
        floating: floating.id,
        source,
    })
}

fn runtime_node(
    runtime_nodes: &HashMap<u64, NodeId>,
    owner: SnapshotReferenceOwner,
    id: u64,
) -> Result<NodeId, SnapshotRestoreError> {
    runtime_nodes
        .get(&id)
        .copied()
        .ok_or(SnapshotRestoreError::UnknownReference {
            owner,
            kind: SnapshotEntityKind::Node,
            id,
        })
}

#[cfg(test)]
mod auto_grid_shape_tests {
    use super::SnapshotGridShape;

    #[test]
    fn auto_grid_shape_rejects_unknown_json_fields() {
        assert_eq!(
            serde_json::from_str::<SnapshotGridShape>(r#"{"kind":"auto"}"#).unwrap(),
            SnapshotGridShape::Auto
        );
        assert!(
            serde_json::from_str::<SnapshotGridShape>(r#"{"kind":"auto","extra":1}"#).is_err()
        );
    }
}
