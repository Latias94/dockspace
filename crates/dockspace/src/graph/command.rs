//! Typed commands and topology references for durable workspace mutation.

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::geometry::LogicalRect;
pub use crate::graph::RootReference;
pub use crate::graph::workspace::ItemPlace;
use crate::graph::{Axis, SplitWeight};
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::model::{DockspaceGridShape, DockspaceGridShares};
use crate::policy::DockTargetRuleKey;

/// Programmatic request to close one item or one complete root.
///
/// The target carries only stable identity. The core captures all graph,
/// presentation, and policy facts when it accepts the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentCloseTarget {
    /// Close one item in its current tabs node.
    Item(ItemId),
    /// Close every item in one complete root.
    Root(RootId),
    /// Close one exact direct tab, including its complete layout subtree.
    Tab { source: TabSource },
    /// Close one exact tabs container without a representative content item.
    TabsSource { source: NodeSource },
}

/// Durable result of an approved content-close transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseCommitOutcome {
    /// One item was removed from its root.
    ItemClosed { item: ItemId, root: RootId },
    /// One complete root and all of its items were removed atomically.
    RootClosed { root: RootId, items: Vec<ItemId> },
    /// One complete tabs group was removed in one transaction.
    TabsClosed { root: RootId, items: Vec<ItemId> },
    /// One direct layout tab and every real descendant were removed atomically.
    TabClosed {
        root: RootId,
        tab: TabKey,
        items: Vec<ItemId>,
    },
    /// One complete surface roster and all of its items were removed atomically.
    SurfaceClosed {
        surface: SurfaceId,
        items: Vec<ItemId>,
    },
}

/// Opaque, collision-free snapshot of a root and its presentation owner.
///
/// Captured references compare the complete structural snapshot rather than a
/// hash. Cloning is cheap because the immutable records use shared ownership.
#[derive(Clone, PartialEq, Eq)]
pub struct NodeFingerprint(pub(crate) Arc<RootFingerprintData>);

impl NodeFingerprint {
    /// Returns the stable root identity captured by this snapshot.
    pub fn root(&self) -> RootId {
        self.0.root
    }

    /// Compares durable topology and tab membership while deliberately ignoring
    /// selection and MRU state.
    ///
    /// Pointer-journal press handling may select a tab before a later edge from
    /// the same actually presented output resolves a drop. That mutation must
    /// refresh command preconditions without authorizing a changed graph, tab
    /// order, split ratio, central node, or presentation owner.
    pub(crate) fn presentation_structure_eq(&self, other: &Self) -> bool {
        self.presentation_eq(other, true)
    }

    pub(crate) fn presentation_topology_eq(&self, other: &Self) -> bool {
        self.presentation_eq(other, false)
    }

    fn presentation_eq(&self, other: &Self, compare_weights: bool) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.0.root == other.0.root
            && self.0.root_node == other.0.root_node
            && self.0.central == other.0.central
            && self.0.allocation == other.0.allocation
            && self.0.presentation == other.0.presentation
            && self.0.nodes.len() == other.0.nodes.len()
            && self
                .0
                .nodes
                .iter()
                .zip(&other.0.nodes)
                .all(|(left, right)| {
                    left.id == right.id
                        && match (&left.node, &right.node) {
                            (
                                FingerprintNode::Tabs {
                                    items: left,
                                    hidden: left_hidden,
                                    ..
                                },
                                FingerprintNode::Tabs {
                                    items: right,
                                    hidden: right_hidden,
                                    ..
                                },
                            ) => left == right && left_hidden == right_hidden,
                            (
                                FingerprintNode::Split {
                                    axis: left_axis,
                                    children: left_children,
                                    weight_bits: left_weights,
                                },
                                FingerprintNode::Split {
                                    axis: right_axis,
                                    children: right_children,
                                    weight_bits: right_weights,
                                },
                            ) => {
                                left_axis == right_axis
                                    && left_children == right_children
                                    && (!compare_weights || left_weights == right_weights)
                            }
                            (
                                FingerprintNode::Grid {
                                    shape: left_shape,
                                    children: left_children,
                                    share_bits: left_shares,
                                },
                                FingerprintNode::Grid {
                                    shape: right_shape,
                                    children: right_children,
                                    share_bits: right_shares,
                                },
                            ) => {
                                left_shape == right_shape
                                    && left_children == right_children
                                    && (!compare_weights || left_shares == right_shares)
                            }
                            (
                                FingerprintNode::Pane {
                                    item: left,
                                    hidden: lh,
                                },
                                FingerprintNode::Pane {
                                    item: right,
                                    hidden: rh,
                                },
                            ) => left == right && lh == rh,
                            _ => false,
                        }
                })
    }
}

impl fmt::Debug for NodeFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeFingerprint")
            .field("root", &self.0.root)
            .field("nodes", &self.0.nodes.len())
            .field("presentation", &self.0.presentation)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootFingerprintData {
    pub(crate) root: RootId,
    pub(crate) root_node: NodeId,
    pub(crate) central: Option<NodeId>,
    pub(crate) allocation: crate::model::RootAllocation,
    pub(crate) nodes: Vec<NodeFingerprintRecord>,
    pub(crate) presentation: FingerprintPresentation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeFingerprintRecord {
    pub(crate) id: NodeId,
    pub(crate) node: FingerprintNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FingerprintNode {
    Pane {
        item: ItemId,
        hidden: bool,
    },
    Tabs {
        items: Arc<[TabKey]>,
        hidden: Vec<ItemId>,
        selected: Option<TabKey>,
        mru: Arc<[TabKey]>,
    },
    Split {
        axis: Axis,
        children: Arc<[NodeId]>,
        weight_bits: Arc<[u64]>,
    },
    Grid {
        shape: DockspaceGridShape,
        children: Arc<[NodeId]>,
        share_bits: Option<(Arc<[u64]>, Arc<[u64]>)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FingerprintPresentation {
    Main {
        surface: SurfaceId,
    },
    Contained {
        surface: SurfaceId,
        floating: FloatingPresentationId,
    },
}

/// A real content source frozen against one workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSource {
    pub(crate) root: RootId,
    pub(crate) place: ItemPlace,
    pub(crate) item: ItemId,
    pub(crate) reference: RootReference,
}

impl ItemSource {
    /// Captures an inline content tab; bare panes have no direct tab source.
    pub(crate) fn as_tab_source(&self) -> Option<TabSource> {
        Some(TabSource {
            root: self.root,
            tabs: self.tabs()?,
            tab: TabKey::for_item(self.item),
            reference: self.reference.clone(),
        })
    }
    /// Returns the source root.
    pub const fn root(&self) -> RootId {
        self.root
    }
    /// Returns the real content placement captured by this source.
    pub const fn place(&self) -> ItemPlace {
        self.place
    }
    /// Returns the real immediate owner node.
    pub const fn owner_node(&self) -> NodeId {
        self.place.owner_node()
    }
    /// Returns the immediate inline tabs owner, if present.
    pub const fn tabs(&self) -> Option<NodeId> {
        self.place.tabs()
    }
    /// Returns the inline tab index, if present.
    pub const fn tab_index(&self) -> Option<usize> {
        self.place.tab_index()
    }
    /// Returns the source item.
    pub const fn item(&self) -> ItemId {
        self.item
    }
    /// Returns the exact root-owner reference.
    pub const fn reference(&self) -> &RootReference {
        &self.reference
    }
}

/// A direct tab child frozen against one workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSource {
    pub(crate) root: RootId,
    pub(crate) tabs: NodeId,
    pub(crate) tab: TabKey,
    pub(crate) reference: RootReference,
}

impl TabSource {
    /// Returns the immediate tabs owner.
    pub const fn owner_node(&self) -> NodeId {
        self.tabs
    }

    /// Returns the source root.
    pub const fn root(&self) -> RootId {
        self.root
    }

    /// Returns the owning tabs container.
    pub const fn tabs(&self) -> NodeId {
        self.tabs
    }

    /// Returns the direct child identity.
    pub const fn tab(&self) -> TabKey {
        self.tab
    }

    /// Returns the exact root-owner reference.
    pub const fn reference(&self) -> &RootReference {
        &self.reference
    }
}

/// A source node frozen against one workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSource {
    pub(crate) root: RootId,
    pub(crate) node: NodeId,
    pub(crate) reference: RootReference,
}

/// A complete replacement of one owner's shared track weights.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackWeights {
    /// One normalized share for each split child.
    Split(Vec<SplitWeight>),
    /// Complete independently normalized row and column shares.
    Grid(DockspaceGridShares),
}

/// Frozen resolved dimensions and the complete cell map used by a grid resize.
///
/// The presentation owner proves that these dimensions describe the actual
/// presented grid. Command validation independently checks the durable roster
/// and derives effective shares from the exact source instead of trusting a
/// caller-supplied changed-axis mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridResizeBasis {
    rows: NonZeroUsize,
    columns: NonZeroUsize,
    children: Arc<[NodeId]>,
}

impl GridResizeBasis {
    /// Captures one legal resolved rectangle without choosing a viewport shape.
    #[must_use]
    pub fn new(rows: usize, columns: usize, children: Arc<[NodeId]>) -> Option<Self> {
        let rows = NonZeroUsize::new(rows)?;
        let columns = NonZeroUsize::new(columns)?;
        DockspaceGridShape::Fixed { rows, columns }
            .validate_children(children.len())
            .ok()?;
        Some(Self {
            rows,
            columns,
            children,
        })
    }

    /// Returns the frozen shared row count.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows.get()
    }

    /// Returns the frozen shared column count.
    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns.get()
    }

    /// Borrows the exact row-major roster, including hidden cells.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }
}

/// One exact shared-track replacement inside an atomic resize batch.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackResize {
    source: NodeSource,
    weights: TrackWeights,
    grid_basis: Option<GridResizeBasis>,
}

impl TrackResize {
    /// Creates one resize update from a frozen split source and normalized weights.
    ///
    /// Validation against the current workspace is deferred to command execution.
    #[must_use]
    pub fn split(source: NodeSource, weights: Vec<SplitWeight>) -> Self {
        Self {
            source,
            weights: TrackWeights::Split(weights),
            grid_basis: None,
        }
    }

    /// Creates a durable grid replacement without a presented-shape interpretation.
    ///
    /// Every changed stored axis requires policy authorization, including latent
    /// Auto shares. Use [`Self::grid_resolved`] for a proven presented shape.
    #[must_use]
    pub fn grid(source: NodeSource, shares: DockspaceGridShares) -> Self {
        Self {
            source,
            weights: TrackWeights::Grid(shares),
            grid_basis: None,
        }
    }

    /// Creates a grid update against one frozen presented shape and cell map.
    ///
    /// Both proposed axes must match the basis. Stored Auto shares with either
    /// dimension mismatched are latent, so policy compares both proposed axes
    /// against equal defaults. An effective no-op preserves the latent values.
    #[must_use]
    pub fn grid_resolved(
        source: NodeSource,
        basis: GridResizeBasis,
        shares: DockspaceGridShares,
    ) -> Self {
        Self {
            source,
            weights: TrackWeights::Grid(shares),
            grid_basis: Some(basis),
        }
    }

    /// Returns the exact frozen owner source.
    #[must_use]
    pub const fn source(&self) -> &NodeSource {
        &self.source
    }

    /// Borrows the complete proposed track values.
    #[must_use]
    pub const fn weights(&self) -> &TrackWeights {
        &self.weights
    }

    /// Borrows the frozen grid interpretation used for effective-axis validation.
    #[must_use]
    pub const fn grid_basis(&self) -> Option<&GridResizeBasis> {
        self.grid_basis.as_ref()
    }

    /// Borrows proposed split weights, when the owner is a split.
    pub fn split_weights(&self) -> Option<&[SplitWeight]> {
        match &self.weights {
            TrackWeights::Split(weights) => Some(weights),
            TrackWeights::Grid(_) => None,
        }
    }

    /// Borrows both proposed grid axes, when the owner is a grid.
    pub const fn grid_shares(&self) -> Option<&DockspaceGridShares> {
        match &self.weights {
            TrackWeights::Grid(shares) => Some(shares),
            TrackWeights::Split(_) => None,
        }
    }
}

impl NodeSource {
    /// Returns the source root.
    pub const fn root(&self) -> RootId {
        self.root
    }

    /// Returns the source node.
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// Returns the exact root-owner reference.
    pub const fn reference(&self) -> &RootReference {
        &self.reference
    }
}

/// Exact source facts for one contained root in a frozen surface roster.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainedRootSource {
    floating: FloatingPresentationId,
    source: NodeSource,
    rect: LogicalRect,
}

impl ContainedRootSource {
    pub(crate) const fn new(
        floating: FloatingPresentationId,
        source: NodeSource,
        rect: LogicalRect,
    ) -> Self {
        Self {
            floating,
            source,
            rect,
        }
    }

    /// Returns the stable contained-presentation identity.
    #[must_use]
    pub const fn floating(&self) -> FloatingPresentationId {
        self.floating
    }

    /// Returns the stable root presented by the contained window.
    #[must_use]
    pub fn root(&self) -> RootId {
        self.source.root()
    }

    /// Returns the source-local rectangle frozen with the roster.
    #[must_use]
    pub const fn rect(&self) -> LogicalRect {
        self.rect
    }

    pub(crate) const fn source(&self) -> &NodeSource {
        &self.source
    }
}

/// Exact optional-main plus ordered contained ownership frozen from one surface.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SurfaceRosterSource {
    surface: SurfaceId,
    main: Option<NodeSource>,
    contained: Arc<[ContainedRootSource]>,
}

impl SurfaceRosterSource {
    pub(crate) fn new(
        surface: SurfaceId,
        main: Option<NodeSource>,
        contained: Vec<ContainedRootSource>,
    ) -> Self {
        Self {
            surface,
            main,
            contained: Arc::from(contained),
        }
    }

    /// Returns the surface whose complete roster was frozen.
    #[must_use]
    pub(crate) const fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// Returns the stable main root frozen for the surface, when present.
    #[must_use]
    pub(crate) fn main_root(&self) -> Option<RootId> {
        self.main.as_ref().map(NodeSource::root)
    }

    /// Returns contained roots in normative back-to-front order.
    #[must_use]
    pub(crate) fn contained(&self) -> &[ContainedRootSource] {
        &self.contained
    }

    pub(crate) const fn main_source(&self) -> Option<&NodeSource> {
        self.main.as_ref()
    }
}

/// A tabs target frozen against one workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabTarget {
    pub(crate) root: RootId,
    pub(crate) tabs: NodeId,
    pub(crate) reference: RootReference,
    pub(crate) surface: SurfaceId,
    pub(crate) central: bool,
    pub(crate) rule: DockTargetRuleKey,
}

impl TabTarget {
    /// Returns the target root.
    pub const fn root(&self) -> RootId {
        self.root
    }

    /// Returns the target tabs node.
    pub const fn tabs(&self) -> NodeId {
        self.tabs
    }

    /// Returns the exact root-owner reference.
    pub const fn reference(&self) -> &RootReference {
        &self.reference
    }

    /// Returns the logical surface that owned this target when it was captured.
    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// Returns whether this target was the root's declared central Tabs container.
    pub const fn is_central(&self) -> bool {
        self.central
    }

    /// Returns the real item or root policy scope frozen for this target.
    pub const fn rule(&self) -> DockTargetRuleKey {
        self.rule
    }
}

/// A validated payload share used by an edge insertion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockFraction(f64);

impl DockFraction {
    /// Creates a finite fraction strictly between zero and one.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDockFraction`] when `value` is non-finite or outside
    /// the open interval `(0, 1)`.
    pub fn new(value: f64) -> Result<Self, InvalidDockFraction> {
        if value.is_finite() && value > 0.0 && value < 1.0 {
            Ok(Self(value))
        } else {
            Err(InvalidDockFraction { value })
        }
    }

    /// Returns the scalar fraction.
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// Failure to construct a [`DockFraction`].
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("dock fraction must be finite and strictly between zero and one, got {value}")]
pub struct InvalidDockFraction {
    /// Rejected scalar.
    pub value: f64,
}

/// Physical side of a target branch receiving an edge insertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Edge {
    /// Insert before the target on the horizontal axis.
    Left,
    /// Insert after the target on the horizontal axis.
    Right,
    /// Insert before the target on the vertical axis.
    Top,
    /// Insert after the target on the vertical axis.
    Bottom,
}

/// Semantic extent of one frozen edge target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTargetScope {
    /// Split one bare Pane or Tabs container at a branch-local edge.
    Inner,
    /// Split the complete docking root at its outer boundary.
    Outer,
}

/// An edge target frozen against one workspace state.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeTarget {
    pub(crate) root: RootId,
    pub(crate) node: NodeId,
    pub(crate) reference: RootReference,
    pub(crate) edge: Edge,
    pub(crate) fraction: DockFraction,
    pub(crate) surface: SurfaceId,
    pub(crate) central: bool,
    pub(crate) rule: DockTargetRuleKey,
    pub(crate) scope: EdgeTargetScope,
}

impl EdgeTarget {
    /// Returns the target root.
    pub const fn root(&self) -> RootId {
        self.root
    }

    /// Returns the target branch node.
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// Returns the exact root-owner reference.
    pub const fn reference(&self) -> &RootReference {
        &self.reference
    }

    /// Returns the insertion side.
    pub const fn edge(&self) -> Edge {
        self.edge
    }

    /// Returns the payload share of the target branch.
    pub const fn fraction(&self) -> DockFraction {
        self.fraction
    }

    /// Returns the logical surface that owned this target when it was captured.
    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// Returns whether this branch was the root's declared central node.
    pub const fn is_central(&self) -> bool {
        self.central
    }

    /// Returns the exact semantic rule represented by this edge.
    pub const fn rule(&self) -> DockTargetRuleKey {
        self.rule
    }

    /// Returns whether this proof names an inner or outer edge.
    pub const fn scope(&self) -> EdgeTargetScope {
        self.scope
    }
}

/// Existing topology location receiving an opened or moved payload.
#[derive(Debug, Clone, PartialEq)]
pub enum DockTarget {
    /// Append a payload to an existing tabs stack.
    Center(TabTarget),
    /// Insert a payload into an exact pre-removal tab gap.
    TabGap {
        /// Frozen tabs target.
        target: TabTarget,
        /// Gap index in the target's current tab sequence.
        index: usize,
    },
    /// Split a bare Pane or Tabs container at an explicit local edge and fraction.
    InnerEdge(EdgeTarget),
    /// Split the complete target root at an explicit outer edge and fraction.
    OuterEdge(EdgeTarget),
}

impl DockTarget {
    /// Returns the frozen logical target surface.
    pub const fn surface(&self) -> SurfaceId {
        match self {
            Self::Center(target) | Self::TabGap { target, .. } => target.surface(),
            Self::InnerEdge(target) | Self::OuterEdge(target) => target.surface(),
        }
    }

    /// Returns whether the exact target is the root's central node.
    pub const fn is_central(&self) -> bool {
        match self {
            Self::Center(target) | Self::TabGap { target, .. } => target.is_central(),
            Self::InnerEdge(target) | Self::OuterEdge(target) => target.is_central(),
        }
    }

    /// Returns the core-minted semantic policy rule for this target.
    pub const fn rule(&self) -> DockTargetRuleKey {
        match self {
            Self::Center(target) | Self::TabGap { target, .. } => target.rule(),
            Self::InnerEdge(target) | Self::OuterEdge(target) => target.rule(),
        }
    }
}

/// Application content detached by a move command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovePayload {
    /// Move one item out of its tabs stack.
    Item(ItemSource),
    /// Move and merge one complete tabs stack.
    /// An inner central container stays in its source root with no tabs.
    Tabs(NodeSource),
    /// Move an arbitrary complete subtree, preserving it as one tab at center targets.
    Subtree(NodeSource),
}

/// Content used to create a new independently presented root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootContent {
    /// Open one application item, increasing the item multiset by one.
    OpenItem(ItemId),
    /// Move existing workspace content without changing item ownership.
    Move(MovePayload),
}

/// Structural insertion position in one surface's contained roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainedPosition {
    /// Insert at the roster end, which is the frontmost presentation.
    Front,
    /// Insert immediately behind the named presentation.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    Before(FloatingPresentationId),
    /// Insert immediately in front of the named presentation.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    After(FloatingPresentationId),
}

/// Opaque snapshot of one complete contained roster used as a command precondition.
///
/// Capturing the whole back-to-front sequence prevents delayed focus input from recomputing a
/// raise against peer presentations that changed in the meantime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainedRosterSource {
    pub(crate) surface: SurfaceId,
    pub(crate) contained: Arc<[FloatingPresentationId]>,
}

impl ContainedRosterSource {
    /// Returns the surface whose roster was captured.
    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// Returns the captured contained identities in back-to-front order.
    pub fn contained(&self) -> &[FloatingPresentationId] {
        &self.contained
    }
}

/// Explicit presentation destination for an existing complete root.
///
/// Rehoming preserves the root and topology identities. Moving between two
/// contained hosts must also preserve the existing floating presentation
/// identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RootPresentationTarget {
    /// Present the root as the main dock area of a newly created logical surface.
    NewSurface { surface: SurfaceId },
    /// Install the root into an existing rootless surface's main dock area.
    Main { surface: SurfaceId },
    /// Present the root as a contained floating on an existing surface.
    Contained {
        surface: SurfaceId,
        floating: FloatingPresentationId,
        rect: LogicalRect,
        position: ContainedPosition,
    },
}

/// The sole durable mutation vocabulary for a docking workspace.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceCommand {
    /// Select one direct child in its current tabs container.
    Select { source: TabSource },
    /// Activate real content and every containing layout tab atomically.
    SelectItem { source: ItemSource },
    /// Show or hide an owned item while retaining its membership and split weights.
    SetItemVisibility { source: ItemSource, visible: bool },
    /// Reorder one direct child to an exact pre-removal gap in the same container.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    Reorder {
        source: TabSource,
        insertion_index: usize,
    },
    /// Open one not-yet-owned item into existing topology.
    Open { item: ItemId, target: DockTarget },
    /// Move an item, tabs stack, or complete subtree into existing topology.
    Move {
        payload: MovePayload,
        target: DockTarget,
    },
    /// Atomically replace complete shared-track values for distinct layout owners.
    /// An empty batch succeeds without changing graph state or owner authority.
    ResizeTracks { tracks: Vec<TrackResize> },
    /// Attach a logical surface with a new main root containing one empty central tabs leaf.
    ///
    /// This creates no native hosting and admits no application item.
    AttachEmptySurface { surface: SurfaceId, root: RootId },
    /// Retire a logical surface only when its entire main and contained forest owns no items.
    ///
    /// All empty roots and contained presentations owned by the surface are removed atomically.
    RetireEmptySurface { surface: SurfaceId },
    /// Create a new logical surface and its main root atomically.
    CreateSurfaceRoot {
        surface: SurfaceId,
        root: RootId,
        content: RootContent,
    },
    /// Create a contained-floating root and both presentation backlinks atomically.
    CreateContainedRoot {
        surface: SurfaceId,
        root: RootId,
        floating: FloatingPresentationId,
        rect: LogicalRect,
        position: ContainedPosition,
        content: RootContent,
    },
    /// Install a newly created root into an existing rootless surface.
    ///
    /// `root` is a caller-supplied fresh identity. Complete existing roots use
    /// [`WorkspaceCommand::RehomeRoot`], except that a contained root becoming main on its current
    /// surface uses [`WorkspaceCommand::PromoteContained`]. Neither path replaces the root identity.
    InstallMainRoot {
        surface: SurfaceId,
        root: RootId,
        content: RootContent,
    },
    /// Transfer an existing complete root to another presentation owner.
    ///
    /// Unlike `Create*Root`, this command never changes `RootId`, `NodeId`, or
    /// central-region identity.
    RehomeRoot {
        source: NodeSource,
        target: RootPresentationTarget,
    },
    /// Promote one exact contained root to the main slot of its current rootless surface.
    PromoteContained {
        source: NodeSource,
        surface: SurfaceId,
        floating: FloatingPresentationId,
    },
    /// Replace a contained rectangle only when its exact previous value still matches.
    ///
    /// This command is intended for application/offline mutation and scene-proven
    /// resize sessions that do not alter stacking. A contained title drag must use
    /// [`WorkspaceCommand::UpdateContainedPresentation`] so geometry and roster
    /// position share one exact transaction boundary.
    UpdateContainedRect {
        surface: SurfaceId,
        root: RootId,
        floating: FloatingPresentationId,
        expected_rect: LogicalRect,
        rect: LogicalRect,
    },
    /// Atomically update one existing contained presentation's rectangle and roster position.
    ///
    /// The complete root source, previous rectangle, and full surface roster are
    /// exact preconditions. Position is resolved only after removing `floating`
    /// from that frozen roster, so self anchors reject and peer anchors cannot
    /// be reinterpreted after concurrent changes.
    UpdateContainedPresentation {
        source: NodeSource,
        floating: FloatingPresentationId,
        expected_rect: LogicalRect,
        expected_roster: ContainedRosterSource,
        rect: LogicalRect,
        position: ContainedPosition,
    },
    /// Raise a contained presentation after explicit focus input.
    ///
    /// The complete captured roster is an exact precondition, so an old focus command never
    /// recomputes itself against newer peer state.
    RaiseContained {
        source: NodeSource,
        floating: FloatingPresentationId,
        expected_roster: ContainedRosterSource,
    },
    /// Remove a root whose topology contains no application item.
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    RemoveEmptyRoot { source: NodeSource },
}

impl WorkspaceCommand {
    /// Returns the application item introduced by this command, if any.
    ///
    /// Move commands only transfer content already owned by the workspace. The
    /// four opening forms are therefore the complete identity-admission surface.
    pub(crate) const fn opened_item(&self) -> Option<ItemId> {
        match self {
            Self::Open { item, .. } => Some(*item),
            Self::CreateSurfaceRoot {
                content: RootContent::OpenItem(item),
                ..
            }
            | Self::CreateContainedRoot {
                content: RootContent::OpenItem(item),
                ..
            }
            | Self::InstallMainRoot {
                content: RootContent::OpenItem(item),
                ..
            } => Some(*item),
            Self::Select { .. }
            | Self::SelectItem { .. }
            | Self::SetItemVisibility { .. }
            | Self::Reorder { .. }
            | Self::Move { .. }
            | Self::ResizeTracks { .. }
            | Self::AttachEmptySurface { .. }
            | Self::RetireEmptySurface { .. }
            | Self::CreateSurfaceRoot {
                content: RootContent::Move(_),
                ..
            }
            | Self::CreateContainedRoot {
                content: RootContent::Move(_),
                ..
            }
            | Self::InstallMainRoot {
                content: RootContent::Move(_),
                ..
            }
            | Self::RehomeRoot { .. }
            | Self::PromoteContained { .. }
            | Self::UpdateContainedRect { .. }
            | Self::UpdateContainedPresentation { .. }
            | Self::RaiseContained { .. }
            | Self::RemoveEmptyRoot { .. } => None,
        }
    }
}

/// Structured result of one successfully staged command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Real content and its containing layout branches were activated.
    ItemActivated { item: ItemId, changed: bool },
    /// Selection was checked and optionally changed.
    Selected {
        tab: TabKey,
        tabs: NodeId,
        changed: bool,
    },
    /// One owned item's persistent visibility was checked and optionally changed.
    ItemVisibilityChanged {
        item: ItemId,
        root: RootId,
        visible: bool,
        changed: bool,
    },
    /// One tab was checked and optionally reordered.
    Reordered {
        tab: TabKey,
        tabs: NodeId,
        from: usize,
        to: usize,
        changed: bool,
    },
    /// One new item was opened.
    Opened { item: ItemId, root: RootId },
    /// Existing items were moved without changing ownership.
    Moved {
        items: Vec<ItemId>,
        source_root: RootId,
        target_root: RootId,
        changed: bool,
    },
    /// One atomic track-resize batch was checked and optionally stored.
    TracksResized { tracks: Vec<NodeId>, changed: bool },
    /// A logical surface with an empty central main root was attached.
    EmptySurfaceAttached { surface: SurfaceId, root: RootId },
    /// A logical surface and its complete item-free forest were retired.
    EmptySurfaceRetired { surface: SurfaceId },
    /// A new surface and main root were created.
    SurfaceRootCreated {
        surface: SurfaceId,
        root: RootId,
        items: Vec<ItemId>,
    },
    /// A new contained-floating root was created.
    ContainedRootCreated {
        surface: SurfaceId,
        root: RootId,
        floating: FloatingPresentationId,
        items: Vec<ItemId>,
    },
    /// A new main root was installed into an existing rootless surface.
    MainRootInstalled {
        surface: SurfaceId,
        root: RootId,
        items: Vec<ItemId>,
    },
    /// An existing root was transferred without changing topology identity.
    RootRehomed {
        root: RootId,
        surface: SurfaceId,
        floating: Option<FloatingPresentationId>,
        changed: bool,
    },
    /// One exact contained presentation became its surface's main root.
    ContainedPromoted {
        surface: SurfaceId,
        root: RootId,
        floating: FloatingPresentationId,
    },
    /// A contained rectangle was checked and optionally updated.
    ContainedRectUpdated {
        floating: FloatingPresentationId,
        changed: bool,
    },
    /// An existing contained presentation's rectangle and roster position were checked together.
    ContainedPresentationUpdated {
        floating: FloatingPresentationId,
        from: usize,
        to: usize,
        rect_changed: bool,
        order_changed: bool,
    },
    /// A contained roster position was checked and optionally raised.
    ContainedRaised {
        floating: FloatingPresentationId,
        from: usize,
        to: usize,
        changed: bool,
    },
    /// An empty root and its presentation were removed.
    EmptyRootRemoved { root: RootId },
}

impl CommandOutcome {
    pub(crate) const fn changes_workspace(&self) -> bool {
        match self {
            Self::ItemActivated { changed, .. }
            | Self::Selected { changed, .. }
            | Self::ItemVisibilityChanged { changed, .. }
            | Self::Reordered { changed, .. }
            | Self::Moved { changed, .. }
            | Self::TracksResized { changed, .. }
            | Self::RootRehomed { changed, .. }
            | Self::ContainedRectUpdated { changed, .. }
            | Self::ContainedRaised { changed, .. } => *changed,
            Self::ContainedPresentationUpdated {
                rect_changed,
                order_changed,
                ..
            } => *rect_changed || *order_changed,
            Self::Opened { .. }
            | Self::EmptySurfaceAttached { .. }
            | Self::EmptySurfaceRetired { .. }
            | Self::SurfaceRootCreated { .. }
            | Self::ContainedRootCreated { .. }
            | Self::MainRootInstalled { .. }
            | Self::ContainedPromoted { .. }
            | Self::EmptyRootRemoved { .. } => true,
        }
    }
}
