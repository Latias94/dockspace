//! Durable, renderer-neutral docking topology.
//!
//! Commands, queries, validation and transactions operate on this graph without
//! owning input, presentation or native-window lifecycle state.

mod authority;
pub(crate) mod canonical;
pub(crate) mod command;
#[cfg(test)]
mod composable_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod docking_cost;
pub(crate) mod error;
#[cfg(test)]
mod grid_tests;
pub(crate) mod operation;
pub(crate) mod root_structure;
mod shared_storage;
pub(crate) mod transaction;
pub(crate) mod validation;
mod visibility;
pub(crate) mod workspace;

pub use authority::RootReference;

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;
use thiserror::Error;

use crate::geometry::LogicalRect;
use crate::graph::canonical::CanonicalizationError;
use crate::graph::validation::WorkspaceValidationErrors;
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::model::{DockspaceGridError, DockspaceGridShape, DockspaceGridShares};
use shared_storage::NodeStore;

pub(crate) const NORMALIZED_WEIGHT_TOLERANCE: f64 = 1.0e-12;

/// Uses Neumaier compensation at persistent split-weight boundaries.
pub(crate) fn normalized_weight_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut sum = 0.0_f64;
    let mut correction = 0.0_f64;
    for value in values {
        let next = sum + value;
        if !next.is_finite() {
            return next;
        }
        correction += if sum.abs() >= value.abs() {
            (sum - next) + value
        } else {
            (value - next) + sum
        };
        sum = next;
    }
    sum + correction
}

pub(crate) fn is_exact_tab_mru(items: &[TabKey], selected: Option<TabKey>, mru: &[TabKey]) -> bool {
    if mru.len() != items.len() || mru.first().copied() != selected {
        return false;
    }
    match (items, mru) {
        ([], []) => true,
        ([item], [most_recent]) => item == most_recent,
        ([first, second], [most_recent, previous]) => {
            first != second
                && ((first == most_recent && second == previous)
                    || (first == previous && second == most_recent))
        }
        _ => {
            let mru_items: BTreeSet<_> = mru.iter().copied().collect();
            let visual_items: BTreeSet<_> = items.iter().copied().collect();
            mru_items.len() == mru.len() && mru_items == visual_items
        }
    }
}

/// Direction in which the children of a split are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Axis {
    /// Children are ordered from left to right.
    Horizontal,
    /// Children are ordered from top to bottom.
    Vertical,
}

/// A positive, finite share of a split's available extent.
///
/// A collection of weights is valid only when it is normalized. Use
/// [`SplitWeight::normalize`] when converting arbitrary application values.
/// Weights retain the same `f64` precision as logical layout geometry.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct SplitWeight(pub(crate) f64);

impl SplitWeight {
    /// Creates one positive, finite split weight.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSplitWeight`] when `value` is non-finite, zero, or negative.
    pub fn new(value: f64) -> Result<Self, InvalidSplitWeight> {
        if !value.is_finite() {
            return Err(InvalidSplitWeight::NonFinite { value });
        }
        if value <= 0.0 {
            return Err(InvalidSplitWeight::NonPositive { value });
        }
        Ok(Self(value))
    }

    /// Returns the scalar value represented by this weight.
    pub fn get(self) -> f64 {
        self.0
    }

    /// Converts arbitrary positive values into normalized split weights.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSplitWeights`] for an empty collection, an invalid scalar, an invalid
    /// accumulated sum, or a value that underflows to zero during normalization.
    pub fn normalize(
        values: impl IntoIterator<Item = f64>,
    ) -> Result<Vec<Self>, InvalidSplitWeights> {
        let values: Vec<f64> = values.into_iter().collect();
        if values.is_empty() {
            return Err(InvalidSplitWeights::Empty);
        }

        for (index, value) in values.iter().copied().enumerate() {
            SplitWeight::new(value)
                .map_err(|source| InvalidSplitWeights::Invalid { index, source })?;
        }
        let sum = normalized_weight_sum(values.iter().copied());
        if !sum.is_finite() || sum <= 0.0 {
            return Err(InvalidSplitWeights::InvalidSum);
        }

        let mut normalized = Vec::with_capacity(values.len());
        for (index, value) in values.into_iter().enumerate() {
            let normalized_value = value / sum;
            normalized.push(
                SplitWeight::new(normalized_value)
                    .map_err(|source| InvalidSplitWeights::Invalid { index, source })?,
            );
        }

        Ok(normalized)
    }
}

/// Failure to construct one split weight.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum InvalidSplitWeight {
    /// The supplied weight is NaN or infinite.
    #[error("split weight must be finite, got {value}")]
    NonFinite {
        /// Rejected scalar value.
        value: f64,
    },
    /// The supplied weight is zero or negative.
    #[error("split weight must be greater than zero, got {value}")]
    NonPositive {
        /// Rejected scalar value.
        value: f64,
    },
}

/// Failure to normalize a collection of split weights.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum InvalidSplitWeights {
    /// At least one value is required.
    #[error("split weights cannot be empty")]
    Empty,
    /// One input value is not a valid runtime weight.
    #[error("invalid split weight at index {index}: {source}")]
    Invalid {
        /// Position of the rejected value.
        index: usize,
        /// Scalar validation failure.
        source: InvalidSplitWeight,
    },
    /// The accumulated weight cannot be normalized.
    #[error("split weight sum is not finite and positive")]
    InvalidSum,
}

/// Runtime node in a docking forest.
#[derive(Debug, PartialEq)]
pub enum Node {
    /// One real application content pane without tabs chrome.
    Pane {
        /// Real application content owned by this pane.
        item: ItemId,
    },
    /// Ordered direct children sharing one content rectangle.
    Tabs {
        /// Direct children in visual tab order.
        items: Vec<TabKey>,
        /// Active direct child, or `None` for a permitted empty central container.
        selected: Option<TabKey>,
        /// Complete most-recent-first permutation, with the selected child first.
        mru: Vec<TabKey>,
        /// Explicit title when this container is presented as a layout tab.
        layout_title: Option<Arc<str>>,
    },
    /// Ordered N-ary split children.
    Split {
        /// Layout direction.
        axis: Axis,
        /// Child nodes in visual order.
        children: Vec<NodeId>,
        /// Normalized child shares, one per child.
        weights: Vec<SplitWeight>,
        /// Explicit title when this container is presented as a layout tab.
        layout_title: Option<Arc<str>>,
    },
    /// Row-major children sharing complete row and column track axes.
    Grid {
        /// Fixed dimensions or a shape resolved for the offered bounds.
        shape: DockspaceGridShape,
        /// Real child nodes in durable row-major order.
        children: Vec<NodeId>,
        /// Explicit row and column shares, or equal tracks when absent.
        shares: Option<DockspaceGridShares>,
        /// Explicit title when this grid is presented as a layout tab.
        layout_title: Option<Arc<str>>,
    },
}

impl Clone for Node {
    fn clone(&self) -> Self {
        #[cfg(test)]
        crate::interaction::targeting::resolver::structural_work::record_node_payload_clone(self);
        match self {
            Self::Pane { item } => Self::Pane { item: *item },
            Self::Tabs {
                items,
                selected,
                mru,
                layout_title,
            } => Self::Tabs {
                items: items.clone(),
                selected: *selected,
                mru: mru.clone(),
                layout_title: layout_title.clone(),
            },
            Self::Split {
                axis,
                children,
                weights,
                layout_title,
            } => Self::Split {
                axis: *axis,
                children: children.clone(),
                weights: weights.clone(),
                layout_title: layout_title.clone(),
            },
            Self::Grid {
                shape,
                children,
                shares,
                layout_title,
            } => Self::Grid {
                shape: *shape,
                children: children.clone(),
                shares: shares.clone(),
                layout_title: layout_title.clone(),
            },
        }
    }
}

/// Failure to edit an existing structural child slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NodeEditError {
    /// The index does not name a structural child.
    #[error("node has no structural child at index {index}")]
    NotStructuralChild { index: usize },
}

/// Borrowed label of one exact direct tab child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabLabel<'a> {
    /// Application title supplied for real content.
    Item(ItemId),
    /// Declared title or deterministic container-kind fallback.
    Layout { title: &'a str },
}

impl Node {
    /// Trims tracks made entirely empty by a checked roster removal.
    /// This writer repair is never applied while validating external graph input.
    pub(crate) fn repair_grid_tracks(&mut self) -> Result<(), DockspaceGridError> {
        let Self::Grid {
            shape,
            children,
            shares,
            ..
        } = self
        else {
            return Ok(());
        };
        if children.is_empty() {
            return Err(DockspaceGridError::EmptyChildren);
        }
        if let DockspaceGridShape::Fixed { rows, columns } = shape {
            let count = children.len();
            let next_columns = columns.get().min(count);
            let next_rows = count.div_ceil(next_columns);
            if rows.get() != next_rows || columns.get() != next_columns {
                *rows = std::num::NonZeroUsize::new(next_rows)
                    .ok_or(DockspaceGridError::EmptyChildren)?;
                *columns = std::num::NonZeroUsize::new(next_columns)
                    .ok_or(DockspaceGridError::EmptyChildren)?;
                if let Some(current) = shares {
                    let rows = if current.row_weights().len() == next_rows {
                        current.row_weights().to_vec()
                    } else {
                        SplitWeight::normalize(current.rows().take(next_rows))
                            .map_err(|source| DockspaceGridError::InvalidRowShares { source })?
                    };
                    let columns = if current.column_weights().len() == next_columns {
                        current.column_weights().to_vec()
                    } else {
                        SplitWeight::normalize(current.columns().take(next_columns))
                            .map_err(|source| DockspaceGridError::InvalidColumnShares { source })?
                    };
                    *current = DockspaceGridShares::from_normalized(rows, columns)?;
                }
            }
        } else if shares
            .as_ref()
            .is_some_and(|shares| shares.validate_for(*shape, children.len()).is_err())
        {
            *shares = None;
        }
        Ok(())
    }

    /// Creates a pane without an implicit tabs container.
    pub const fn pane(item: ItemId) -> Self {
        Self::Pane { item }
    }

    /// Creates a tabs node and selects its first item, if present.
    pub fn tabs(items: impl IntoIterator<Item = ItemId>) -> Self {
        let items: Vec<_> = items.into_iter().map(TabKey::for_item).collect();
        let selected = items.first().copied();
        Self::tabs_from_keys(items, selected)
    }

    /// Creates a tabs node with an explicit selection.
    /// Full membership checks run when the workspace draft is built.
    pub fn tabs_with_selection(
        items: impl IntoIterator<Item = ItemId>,
        selected: Option<ItemId>,
    ) -> Self {
        Self::tabs_from_keys(
            items.into_iter().map(TabKey::for_item).collect(),
            selected.map(TabKey::for_item),
        )
    }

    pub(crate) fn tabs_from_keys(items: Vec<TabKey>, selected: Option<TabKey>) -> Self {
        let mru = initial_tab_mru(&items, selected);
        Self::Tabs {
            items,
            selected,
            mru,
            layout_title: None,
        }
    }

    /// Borrows this container's explicit layout title.
    pub fn layout_title(&self) -> Option<&str> {
        match self {
            Self::Pane { .. } => None,
            Self::Tabs { layout_title, .. }
            | Self::Split { layout_title, .. }
            | Self::Grid { layout_title, .. } => layout_title.as_deref(),
        }
    }

    /// Sets a structural container's title; pane titles remain application-owned.
    pub fn set_layout_title(&mut self, title: Option<Arc<str>>) {
        match self {
            Self::Pane { .. } => {}
            Self::Tabs { layout_title, .. }
            | Self::Split { layout_title, .. }
            | Self::Grid { layout_title, .. } => *layout_title = title,
        }
    }

    /// Iterates structural children with their original visual roster indices.
    pub fn child_nodes(&self) -> impl DoubleEndedIterator<Item = (usize, NodeId)> + Clone + '_ {
        let len = match self {
            Self::Pane { .. } => 0,
            Self::Tabs { items, .. } => items.len(),
            Self::Split { children, .. } | Self::Grid { children, .. } => children.len(),
        };
        (0..len).filter_map(move |index| {
            let child = match self {
                Self::Pane { .. } => None,
                Self::Tabs { items, .. } => items[index].layout(),
                Self::Split { children, .. } | Self::Grid { children, .. } => Some(children[index]),
            };
            child.map(|child| (index, child))
        })
    }

    /// Replaces an existing structural edge, preserving selection and MRU.
    /// Workspace operations validate replacement kind and exclusive ownership.
    pub fn replace_child(
        &mut self,
        index: usize,
        replacement: NodeId,
    ) -> Result<NodeId, NodeEditError> {
        let invalid = || NodeEditError::NotStructuralChild { index };
        match self {
            Self::Pane { .. } => Err(invalid()),
            Self::Tabs {
                items,
                selected,
                mru,
                ..
            } => {
                let slot = items.get_mut(index).ok_or_else(invalid)?;
                let previous = slot.layout().ok_or_else(invalid)?;
                let old = *slot;
                let new = TabKey::for_layout(replacement);
                *slot = new;
                if *selected == Some(old) {
                    *selected = Some(new);
                }
                for entry in mru {
                    if *entry == old {
                        *entry = new;
                    }
                }
                Ok(previous)
            }
            Self::Split { children, .. } | Self::Grid { children, .. } => {
                let slot = children.get_mut(index).ok_or_else(invalid)?;
                Ok(std::mem::replace(slot, replacement))
            }
        }
    }

    /// Creates an N-ary split and normalizes its raw weights.
    #[cfg(test)]
    pub fn split(
        axis: Axis,
        children: impl IntoIterator<Item = NodeId>,
        weights: impl IntoIterator<Item = f64>,
    ) -> Result<Self, NodeBuildError> {
        let children: Vec<_> = children.into_iter().collect();
        if children.len() < 2 {
            return Err(NodeBuildError::SplitTooFewChildren {
                children: children.len(),
            });
        }
        let weights = SplitWeight::normalize(weights)?;
        if children.len() != weights.len() {
            return Err(NodeBuildError::SplitWeightCountMismatch {
                children: children.len(),
                weights: weights.len(),
            });
        }
        Ok(Self::Split {
            axis,
            children,
            weights,
            layout_title: None,
        })
    }

    /// Creates an equal-share N-ary split.
    #[cfg(test)]
    pub fn equal_split(
        axis: Axis,
        children: impl IntoIterator<Item = NodeId>,
    ) -> Result<Self, NodeBuildError> {
        let children: Vec<_> = children.into_iter().collect();
        let weights = std::iter::repeat_n(1.0, children.len());
        Self::split(axis, children, weights)
    }
}

/// Failure to construct a node through the checked convenience API.
#[derive(Debug, Clone, PartialEq, Error)]
#[cfg(test)]
pub enum NodeBuildError {
    /// A split must have at least two children.
    #[error("a split requires at least two children, got {children}")]
    SplitTooFewChildren {
        /// Supplied child count.
        children: usize,
    },
    /// A split has a different number of children and weights.
    #[error("split has {children} children but {weights} weights")]
    SplitWeightCountMismatch {
        /// Supplied child count.
        children: usize,
        /// Supplied weight count.
        weights: usize,
    },
    /// Raw weights could not be normalized.
    #[error(transparent)]
    InvalidWeights(#[from] InvalidSplitWeights),
}

/// Durable metadata for one independently presentable dock-space root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRecord {
    /// Topology root node.
    pub node: NodeId,
    /// Reachable tabs leaf retained as the central docking anchor.
    ///
    /// A main dockspace retains this leaf when its last content leaves. A
    /// contained floating root retires when all of its content leaves.
    pub central: Option<NodeId>,
    /// Constraint redistribution, independently of empty-anchor retention.
    pub allocation: crate::model::RootAllocation,
}

impl RootRecord {
    /// Creates a root without central-region semantics.
    pub fn new(node: NodeId) -> Self {
        Self {
            node,
            central: None,
            allocation: crate::model::RootAllocation::Weighted,
        }
    }

    /// Assigns the reachable tabs leaf used as the central region.
    #[must_use]
    pub fn with_central(mut self, central: NodeId) -> Self {
        self.central = Some(central);
        self
    }

    /// Selects redistribution without changing the declared anchor.
    #[must_use]
    pub fn with_allocation(mut self, allocation: crate::model::RootAllocation) -> Self {
        self.allocation = allocation;
        self
    }
}

/// Presentation roster for one logical surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacePresentation {
    /// Root occupying the surface's main dock area, when present.
    pub main_root: Option<RootId>,
    /// Contained-floating presentation identities owned by this surface in back-to-front order.
    pub contained: Vec<FloatingPresentationId>,
}

impl SurfacePresentation {
    /// Creates a rooted surface with no contained-floating presentations.
    pub fn with_main(main_root: RootId) -> Self {
        Self {
            main_root: Some(main_root),
            contained: Vec::new(),
        }
    }

    /// Creates a rootless surface draft.
    ///
    /// A rootless surface becomes valid only after at least one contained presentation is attached.
    /// Transaction execution may use the empty form as intermediate state, but strict builder and
    /// persistence validation reject it.
    #[cfg(test)]
    pub fn rootless() -> Self {
        Self {
            main_root: None,
            contained: Vec::new(),
        }
    }
}

/// A root presented as an in-surface floating container.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContainedFloating {
    /// Presented dock-space root.
    pub root: RootId,
    /// Durable logical bounds relative to the owning surface.
    ///
    /// A validated workspace requires both dimensions to be strictly positive.
    pub rect: LogicalRect,
}

impl ContainedFloating {
    /// Creates contained-floating presentation metadata for a workspace draft.
    ///
    /// [`WorkspaceBuilder::build`] and [`Workspace::validate`] reject a rectangle
    /// without strictly positive area.
    pub const fn new(root: RootId, rect: LogicalRect) -> Self {
        Self { root, rect }
    }
}

/// Raw graph storage. Only graph-owned completion can publish it as a Workspace.
#[derive(Debug, Clone, Default)]
pub(crate) struct WorkspaceData {
    pub(crate) nodes: NodeStore,
    pub(crate) roots: Arc<BTreeMap<RootId, RootRecord>>,
    pub(crate) surfaces: BTreeMap<SurfaceId, SurfacePresentation>,
    pub(crate) contained_floatings: BTreeMap<FloatingPresentationId, ContainedFloating>,
    pub(crate) hidden_items: Arc<BTreeSet<ItemId>>,
}

impl PartialEq for WorkspaceData {
    fn eq(&self, other: &Self) -> bool {
        self.roots == other.roots
            && self.surfaces == other.surfaces
            && self.contained_floatings == other.contained_floatings
            && self.hidden_items == other.hidden_items
            && self.nodes.len() == other.nodes.len()
            && self
                .nodes
                .iter()
                .all(|(id, node)| other.nodes.get(id) == Some(node))
    }
}

/// A canonical workspace. Mutable storage is owned by graph's implementation.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    storage: WorkspaceData,
    authority: authority::WorkspaceAuthority,
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        self.storage == other.storage
    }
}

impl Workspace {
    /// Creates the valid empty workspace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an uncommitted workspace draft.
    #[cfg(any(feature = "serde", test))]
    pub fn builder() -> WorkspaceBuilder {
        WorkspaceBuilder::new()
    }

    /// Converts this workspace into an editable draft, retaining node identities.
    #[cfg(test)]
    pub fn into_builder(self) -> WorkspaceBuilder {
        WorkspaceBuilder {
            workspace: self.storage,
        }
    }

    pub(crate) fn storage(&self) -> &WorkspaceData {
        &self.storage
    }

    /// Returns a node by runtime identity.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.storage.node(id)
    }

    /// Iterates nodes in deterministic arena order.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.storage.nodes()
    }

    /// Returns the tabs stack's selected-first MRU permutation.
    #[cfg(test)]
    pub fn tab_mru(&self, tabs: NodeId) -> Option<&[TabKey]> {
        self.storage.tab_mru(tabs)
    }

    /// Returns one root record.
    pub fn root(&self, id: RootId) -> Option<&RootRecord> {
        self.storage.root(id)
    }

    /// Iterates roots in stable identity order.
    pub fn roots(&self) -> impl Iterator<Item = (RootId, &RootRecord)> {
        self.storage.roots()
    }

    /// Returns one surface presentation.
    pub fn surface(&self, id: SurfaceId) -> Option<&SurfacePresentation> {
        self.storage.surface(id)
    }

    /// Iterates surfaces in stable identity order.
    pub fn surfaces(&self) -> impl Iterator<Item = (SurfaceId, &SurfacePresentation)> {
        self.storage.surfaces()
    }

    /// Returns one contained presentation.
    pub fn contained_floating(&self, id: FloatingPresentationId) -> Option<&ContainedFloating> {
        self.storage.contained_floating(id)
    }

    /// Iterates contained presentations in stable identity order.
    pub fn contained_floatings(
        &self,
    ) -> impl Iterator<Item = (FloatingPresentationId, &ContainedFloating)> {
        self.storage.contained_floatings()
    }

    /// Counts every owned item occurrence.
    pub fn item_multiset(&self) -> BTreeMap<ItemId, usize> {
        self.storage.item_multiset()
    }

    fn insert_runtime_node(&mut self, node: Node) -> NodeId {
        self.storage.insert_runtime_node(node)
    }

    fn remove_runtime_node(&mut self, id: NodeId) -> Option<Arc<Node>> {
        self.forget_placement(id);
        self.storage.remove_runtime_node(id)
    }
}

impl WorkspaceData {
    /// Returns a node by runtime identity.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Iterates runtime nodes in deterministic arena order.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter()
    }

    /// Returns one tabs stack's complete most-recent-first item permutation.
    ///
    /// A valid populated tabs node has exactly one occurrence of every visual
    /// item here, with its selected item first. `None` means `tabs` is absent
    /// or is not a tabs node.
    #[cfg(test)]
    pub fn tab_mru(&self, tabs: NodeId) -> Option<&[TabKey]> {
        match self.nodes.get(tabs)? {
            Node::Tabs { mru, .. } => Some(mru),
            Node::Pane { .. } | Node::Split { .. } | Node::Grid { .. } => None,
        }
    }

    /// Returns a root record by stable identity.
    pub fn root(&self, id: RootId) -> Option<&RootRecord> {
        self.roots.get(&id)
    }

    /// Iterates roots in stable identity order.
    pub fn roots(&self) -> impl Iterator<Item = (RootId, &RootRecord)> {
        self.roots.iter().map(|(id, root)| (*id, root))
    }

    /// Returns presentation metadata for a logical surface.
    pub fn surface(&self, id: SurfaceId) -> Option<&SurfacePresentation> {
        self.surfaces.get(&id)
    }

    /// Iterates logical surfaces in stable identity order.
    pub fn surfaces(&self) -> impl Iterator<Item = (SurfaceId, &SurfacePresentation)> {
        self.surfaces.iter().map(|(id, surface)| (*id, surface))
    }

    /// Returns one contained-floating presentation.
    pub fn contained_floating(&self, id: FloatingPresentationId) -> Option<&ContainedFloating> {
        self.contained_floatings.get(&id)
    }

    /// Iterates contained-floating presentations in stable identity order.
    pub fn contained_floatings(
        &self,
    ) -> impl Iterator<Item = (FloatingPresentationId, &ContainedFloating)> {
        self.contained_floatings
            .iter()
            .map(|(id, floating)| (*id, floating))
    }

    /// Counts every item occurrence without hiding duplicate corruption.
    pub fn item_multiset(&self) -> BTreeMap<ItemId, usize> {
        let mut items = BTreeMap::new();
        #[cfg(test)]
        let mut item_visits = 0;
        for node in self.nodes.values() {
            if let Node::Pane { item } = node {
                *items.entry(*item).or_insert(0) += 1;
                #[cfg(test)]
                {
                    item_visits += 1;
                }
            }
            if let Node::Tabs {
                items: tab_items, ..
            } = node
            {
                #[cfg(test)]
                {
                    item_visits += tab_items.len();
                }
                for item in tab_items.iter().filter_map(|tab| tab.item()) {
                    *items.entry(item).or_insert(0) += 1;
                }
            }
        }
        #[cfg(test)]
        crate::interaction::targeting::resolver::structural_work::record_item_multiset_scan(
            item_visits,
        );
        items
    }

    pub(crate) fn insert_runtime_node(&mut self, node: Node) -> NodeId {
        self.nodes.insert(node)
    }

    pub(crate) fn remove_runtime_node(&mut self, id: NodeId) -> Option<Arc<Node>> {
        self.nodes.remove(id)
    }

    pub(crate) fn validate_candidate(&self) -> Result<(), WorkspaceValidationErrors> {
        crate::graph::validation::validate_workspace_data(self)
    }
}

/// Mutable assembly state for constructing and decoding a workspace.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceBuilder {
    pub(crate) workspace: WorkspaceData,
}

impl WorkspaceBuilder {
    /// Creates an empty workspace draft.
    #[cfg(any(feature = "serde", test))]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only raw storage for graph diagnostic fixtures.
    #[cfg(test)]
    pub(crate) fn draft(&self) -> &WorkspaceData {
        &self.workspace
    }

    /// Mutable raw storage; build must still normalize and validate it.
    #[cfg(test)]
    pub(crate) fn draft_mut(&mut self) -> &mut WorkspaceData {
        &mut self.workspace
    }

    /// Inserts a runtime node and returns its generational identity.
    pub fn insert_node(&mut self, node: Node) -> NodeId {
        self.workspace.insert_runtime_node(node)
    }

    /// Replaces an existing node in the draft, including its complete tab history.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceBuildError::MissingNode`] for a stale or absent runtime identity.
    #[cfg(any(feature = "serde", test))]
    pub fn replace_node(&mut self, id: NodeId, node: Node) -> Result<Node, WorkspaceBuildError> {
        let previous = self
            .workspace
            .nodes
            .replace(id, node)
            .ok_or(WorkspaceBuildError::MissingNode { node: id })?;
        Ok(Arc::unwrap_or_clone(previous))
    }

    /// Replaces a tabs stack's complete most-recent-first item permutation.
    ///
    /// This draft API deliberately accepts malformed permutations so callers
    /// can decode untrusted state before one complete validation pass.
    /// [`Self::validate`] and [`Self::build`] reject missing, duplicate, foreign,
    /// or incorrectly ordered entries.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceBuildError`] when `tabs` is absent or is not a tabs node.
    #[cfg(test)]
    pub fn set_tab_mru(
        &mut self,
        tabs: NodeId,
        mru: impl IntoIterator<Item = ItemId>,
    ) -> Result<(), WorkspaceBuildError> {
        let node = self
            .workspace
            .nodes
            .get_mut(tabs)
            .ok_or(WorkspaceBuildError::MissingNode { node: tabs })?;
        let Node::Tabs { mru: history, .. } = node else {
            return Err(WorkspaceBuildError::NodeIsNotTabs { node: tabs });
        };
        *history = mru.into_iter().map(TabKey::for_item).collect();
        Ok(())
    }

    /// Inserts or replaces a stable root record.
    pub fn set_root(&mut self, id: RootId, root: RootRecord) -> Option<RootRecord> {
        let previous = self.workspace.roots.get(&id).copied();
        if previous != Some(root) {
            Arc::make_mut(&mut self.workspace.roots).insert(id, root);
        }
        previous
    }

    /// Inserts or replaces a logical surface presentation.
    pub fn set_surface(
        &mut self,
        id: SurfaceId,
        surface: SurfacePresentation,
    ) -> Option<SurfacePresentation> {
        self.workspace.surfaces.insert(id, surface)
    }

    /// Inserts or replaces a contained-floating record.
    pub fn set_contained_floating(
        &mut self,
        id: FloatingPresentationId,
        floating: ContainedFloating,
    ) -> Option<ContainedFloating> {
        self.workspace.contained_floatings.insert(id, floating)
    }

    /// Adds one floating identity to its owning surface's roster.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceBuildError::MissingSurface`] when the surface is absent.
    #[cfg(test)]
    pub fn attach_contained(
        &mut self,
        surface: SurfaceId,
        floating: FloatingPresentationId,
    ) -> Result<(), WorkspaceBuildError> {
        let presentation = self
            .workspace
            .surfaces
            .get_mut(&surface)
            .ok_or(WorkspaceBuildError::MissingSurface { surface })?;
        presentation.contained.push(floating);
        Ok(())
    }

    /// Canonicalizes and validates the complete draft, then returns it atomically.
    ///
    /// # Errors
    ///
    /// Returns [`CanonicalizationError`] when normalization would hide corruption, lose an item,
    /// or produce a workspace that fails strict validation.
    pub fn build(self) -> Result<Workspace, CanonicalizationError> {
        canonical::build_workspace(self.workspace)
    }

    /// Validates the draft without consuming or canonicalizing it.
    ///
    /// # Errors
    ///
    /// Returns every discovered [`crate::graph::validation::WorkspaceValidationError`].
    #[cfg(any(feature = "serde", test))]
    pub fn validate(&self) -> Result<(), WorkspaceValidationErrors> {
        self.workspace.validate_candidate()
    }
}

/// Failure while editing a workspace draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[cfg(any(feature = "serde", test))]
pub enum WorkspaceBuildError {
    /// A node replacement referenced a stale or missing runtime identity.
    #[error("workspace draft does not contain node {node:?}")]
    MissingNode {
        /// Missing runtime node identity.
        node: NodeId,
    },
    /// A tabs-only draft edit named a split node.
    #[error("workspace draft node {node:?} is not a tabs node")]
    #[cfg(test)]
    NodeIsNotTabs {
        /// Incorrect runtime node identity.
        node: NodeId,
    },
    /// A contained presentation was attached to a missing surface.
    #[error("workspace draft does not contain surface {surface}")]
    #[cfg(test)]
    MissingSurface {
        /// Missing stable surface identity.
        surface: SurfaceId,
    },
}

fn initial_tab_mru(items: &[TabKey], selected: Option<TabKey>) -> Vec<TabKey> {
    let mut mru = Vec::with_capacity(items.len());
    if let Some(selected) = selected.filter(|selected| items.contains(selected)) {
        mru.push(selected);
    }
    mru.extend(items.iter().copied().filter(|item| Some(*item) != selected));
    mru
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_tab_keys_keep_one_graph_node_and_real_content_ownership() {
        use crate::graph::command::WorkspaceCommand;
        use crate::graph::transaction::WorkspaceTransaction;
        use crate::policy::DockPolicySnapshot;

        let items = [ItemId::new(1), ItemId::new(2), ItemId::new(3)];
        let keys = items.map(TabKey::for_item);
        let root = RootId::new(1);
        let mut builder = Workspace::builder();
        let tabs = builder.insert_node(Node::tabs(items));
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(root));
        let mut workspace = builder.build().unwrap();
        let policy = DockPolicySnapshot::default();
        let source = workspace.capture_tab_source(root, tabs, keys[1]).unwrap();
        WorkspaceTransaction::from_commands([WorkspaceCommand::Select { source }])
            .apply(&mut workspace, &policy)
            .unwrap();
        let source = workspace.capture_tab_source(root, tabs, keys[1]).unwrap();
        WorkspaceTransaction::from_commands([WorkspaceCommand::Reorder {
            source,
            insertion_index: 0,
        }])
        .apply(&mut workspace, &policy)
        .unwrap();

        assert_eq!(workspace.nodes().count(), 1);
        assert_eq!(
            workspace.visible_tab_keys(tabs).as_ref(),
            [keys[1], keys[0], keys[2]]
        );
        assert_eq!(
            workspace.tab_mru(tabs),
            Some([keys[1], keys[0], keys[2]].as_slice())
        );
        assert_eq!(workspace.first_active_item(tabs), Some(items[1]));
        assert_eq!(
            workspace.item_multiset(),
            BTreeMap::from(items.map(|item| (item, 1)))
        );
    }

    #[test]
    fn layout_tabs_own_real_containers_without_becoming_application_items() {
        let item = ItemId::new(1);
        let mut builder = Workspace::builder();
        let child = builder.insert_node(Node::tabs([item]));
        let layout = TabKey::for_layout(child);
        assert_ne!(layout, TabKey::for_item(item));
        assert_eq!(layout.item(), None);
        let tabs = builder.insert_node(Node::tabs_from_keys(vec![layout], Some(layout)));
        let root = RootId::new(1);
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(root));
        builder.validate().unwrap();
        let workspace = builder.build().unwrap();
        assert_eq!(workspace.nodes().count(), 2);
        assert_eq!(workspace.item_multiset(), BTreeMap::from([(item, 1)]));
        assert_eq!(workspace.first_active_item(tabs), Some(item));
    }

    #[test]
    fn exact_tab_mru_requires_selected_first_unique_membership() {
        let a = TabKey::for_item(ItemId::new(1));
        let b = TabKey::for_item(ItemId::new(2));
        let c = TabKey::for_item(ItemId::new(3));
        let cases: &[(&[TabKey], Option<TabKey>, &[TabKey], bool)] = &[
            (&[], None, &[], true),
            (&[], Some(a), &[], false),
            (&[], None, &[a], false),
            (&[a], Some(a), &[a], true),
            (&[a], None, &[a], false),
            (&[a], Some(b), &[b], false),
            (&[a], Some(a), &[], false),
            (&[a, b], Some(a), &[a, b], true),
            (&[a, b], Some(b), &[b, a], true),
            (&[a, a], Some(a), &[a, a], false),
            (&[a, a], Some(a), &[a, b], false),
            (&[a, b], Some(a), &[a, a], false),
            (&[a, b], Some(a), &[a, c], false),
            (&[a, b], Some(a), &[b, a], false),
            (&[a, b, c], Some(c), &[c, a, b], true),
            (&[a, b, c], Some(c), &[c, a, a], false),
        ];
        for &(items, selected, mru, expected) in cases {
            assert_eq!(
                is_exact_tab_mru(items, selected, mru),
                expected,
                "items={items:?}, selected={selected:?}, mru={mru:?}"
            );
        }
    }
}
