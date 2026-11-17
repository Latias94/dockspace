use std::{collections::BTreeSet, fmt, sync::Arc};

use thiserror::Error;

use crate::geometry::LogicalRect;
use crate::graph::{
    Axis, ContainedFloating, Node, RootRecord, SplitWeight, SurfacePresentation, Workspace,
    WorkspaceBuilder,
};
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};

use super::{DockspaceGridError, DockspaceGridShape, DockspaceGridShares, DockspaceView};

/// Product-facing direction of a docking split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockspaceAxis {
    /// Children are arranged from left to right.
    Horizontal,
    /// Children are arranged from top to bottom.
    Vertical,
}

impl From<DockspaceAxis> for Axis {
    fn from(axis: DockspaceAxis) -> Self {
        match axis {
            DockspaceAxis::Horizontal => Self::Horizontal,
            DockspaceAxis::Vertical => Self::Vertical,
        }
    }
}

impl From<Axis> for DockspaceAxis {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Horizontal => Self::Horizontal,
            Axis::Vertical => Self::Vertical,
        }
    }
}

/// How a root redistributes space when pane constraints change weighted sizes.
///
/// This choice does not declare an empty docking anchor or change its lifetime.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    any(feature = "serde", test),
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(any(feature = "serde", test), serde(rename_all = "snake_case"))]
pub enum RootAllocation {
    /// Redistribute available space among eligible children by their weights.
    #[default]
    Weighted,
    /// Let the declared central branch absorb or yield constraint deltas first.
    ///
    /// Requires a central Tabs anchor. Other children share any remaining delta
    /// after that branch reaches its size constraint.
    PreferCentral,
}

/// Recursive product declaration of a pane, tabs container, split, or shared-track grid.
/// Compilation, cloning, and destruction use heap traversal rather than recursive calls.
pub struct DockspaceNode {
    kind: DockspaceNodeKind,
    central: bool,
    layout_title: Option<Arc<str>>,
}

enum DockspaceNodeKind {
    Pane(ItemId),
    Tabs {
        items: Vec<ItemId>,
        selected: Option<ItemId>,
    },
    ComposedTabs {
        children: Vec<DockspaceNode>,
        selected: Option<usize>,
    },
    Split {
        axis: DockspaceAxis,
        children: Vec<DockspaceNode>,
        weights: Vec<SplitWeight>,
    },
    Grid {
        shape: DockspaceGridShape,
        children: Vec<DockspaceNode>,
        shares: Option<DockspaceGridShares>,
    },
}

impl fmt::Debug for DockspaceNode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut node = formatter.debug_struct("DockspaceNode");
        match &self.kind {
            DockspaceNodeKind::Pane(item) => {
                node.field("item", item);
            }
            DockspaceNodeKind::Tabs { items, selected } => {
                node.field("items", items).field("selected", selected);
            }
            DockspaceNodeKind::ComposedTabs { children, selected } => {
                node.field("child_count", &children.len())
                    .field("selected_index", selected);
            }
            DockspaceNodeKind::Split {
                axis,
                children,
                weights,
            } => {
                node.field("axis", axis)
                    .field("child_count", &children.len())
                    .field("weights", weights);
            }
            DockspaceNodeKind::Grid {
                shape,
                children,
                shares,
            } => {
                node.field("shape", shape)
                    .field("child_count", &children.len())
                    .field("shares", shares);
            }
        }
        node.field("central", &self.central)
            .field("layout_title", &self.layout_title)
            .finish_non_exhaustive()
    }
}

impl Clone for DockspaceNode {
    fn clone(&self) -> Self {
        enum Frame<'a> {
            Enter(&'a DockspaceNode),
            Finish(&'a DockspaceNode, usize),
        }
        let mut pending = vec![Frame::Enter(self)];
        let mut completed = Vec::new();
        while let Some(frame) = pending.pop() {
            match frame {
                Frame::Enter(node) => match &node.kind {
                    DockspaceNodeKind::ComposedTabs { children, .. }
                    | DockspaceNodeKind::Split { children, .. }
                    | DockspaceNodeKind::Grid { children, .. } => {
                        pending.push(Frame::Finish(node, completed.len()));
                        pending.extend(children.iter().rev().map(Frame::Enter));
                    }
                    DockspaceNodeKind::Pane(item) => completed.push(Self {
                        kind: DockspaceNodeKind::Pane(*item),
                        central: node.central,
                        layout_title: node.layout_title.clone(),
                    }),
                    DockspaceNodeKind::Tabs { items, selected } => completed.push(Self {
                        kind: DockspaceNodeKind::Tabs {
                            items: items.clone(),
                            selected: *selected,
                        },
                        central: node.central,
                        layout_title: node.layout_title.clone(),
                    }),
                },
                Frame::Finish(node, base) => {
                    let children = completed.split_off(base);
                    let kind = match &node.kind {
                        DockspaceNodeKind::ComposedTabs { selected, .. } => {
                            DockspaceNodeKind::ComposedTabs {
                                children,
                                selected: *selected,
                            }
                        }
                        DockspaceNodeKind::Split { axis, weights, .. } => {
                            DockspaceNodeKind::Split {
                                axis: *axis,
                                children,
                                weights: weights.clone(),
                            }
                        }
                        DockspaceNodeKind::Grid { shape, shares, .. } => DockspaceNodeKind::Grid {
                            shape: *shape,
                            shares: shares.clone(),
                            children,
                        },
                        _ => unreachable!("only recursive declarations have finish frames"),
                    };
                    completed.push(Self {
                        kind,
                        central: node.central,
                        layout_title: node.layout_title.clone(),
                    });
                }
            }
        }
        completed.pop().expect("the declaration contains its root")
    }
}

impl Drop for DockspaceNode {
    fn drop(&mut self) {
        let children = match &mut self.kind {
            DockspaceNodeKind::ComposedTabs { children, .. }
            | DockspaceNodeKind::Split { children, .. }
            | DockspaceNodeKind::Grid { children, .. } => children,
            _ => return,
        };
        let mut pending = std::mem::take(children);
        while let Some(mut node) = pending.pop() {
            if let DockspaceNodeKind::ComposedTabs { children, .. }
            | DockspaceNodeKind::Split { children, .. }
            | DockspaceNodeKind::Grid { children, .. } = &mut node.kind
            {
                pending.append(children);
            }
        }
    }
}

impl DockspaceNode {
    fn into_parts(mut self) -> (DockspaceNodeKind, bool, Option<Arc<str>>) {
        let kind = std::mem::replace(
            &mut self.kind,
            DockspaceNodeKind::Tabs {
                items: Vec::new(),
                selected: None,
            },
        );
        (kind, self.central, self.layout_title.take())
    }

    /// Creates real content without an implicit tabs container.
    #[must_use]
    pub const fn pane(item: ItemId) -> Self {
        Self {
            kind: DockspaceNodeKind::Pane(item),
            central: false,
            layout_title: None,
        }
    }

    /// Names a structural subtree when it is presented as a layout tab.
    /// Pane titles remain supplied by the application's content view.
    #[must_use]
    pub fn named(mut self, title: impl Into<Arc<str>>) -> Self {
        if !matches!(self.kind, DockspaceNodeKind::Pane(_)) {
            self.layout_title = Some(title.into());
        }
        self
    }

    /// Creates compact tabs containing real application items.
    #[must_use]
    pub fn tabs(items: impl IntoIterator<Item = ItemId>) -> Self {
        Self::tabs_inner(items, false)
    }

    /// Creates the root's retained central tabs anchor.
    #[must_use]
    pub fn central_tabs(items: impl IntoIterator<Item = ItemId>) -> Self {
        Self::tabs_inner(items, true)
    }

    fn tabs_inner(items: impl IntoIterator<Item = ItemId>, central: bool) -> Self {
        let items: Vec<_> = items.into_iter().collect();
        let selected = items.first().copied();
        Self {
            kind: DockspaceNodeKind::Tabs { items, selected },
            central,
            layout_title: None,
        }
    }

    /// Creates compact tabs with an explicit selected item.
    /// Returns an error when the selection does not match the roster.
    pub fn tabs_with_selection(
        items: impl IntoIterator<Item = ItemId>,
        selected: Option<ItemId>,
    ) -> Result<Self, DockspaceLayoutError> {
        Self::tabs_with_selection_inner(items, selected, false)
    }

    /// Creates central compact tabs with an explicit selected item.
    pub fn central_tabs_with_selection(
        items: impl IntoIterator<Item = ItemId>,
        selected: Option<ItemId>,
    ) -> Result<Self, DockspaceLayoutError> {
        Self::tabs_with_selection_inner(items, selected, true)
    }

    fn tabs_with_selection_inner(
        items: impl IntoIterator<Item = ItemId>,
        selected: Option<ItemId>,
        central: bool,
    ) -> Result<Self, DockspaceLayoutError> {
        let items: Vec<_> = items.into_iter().collect();
        if !match (items.is_empty(), selected) {
            (true, None) => true,
            (false, Some(item)) => items.contains(&item),
            _ => false,
        } {
            return Err(DockspaceLayoutError::InvalidSelection { selected });
        }
        Ok(Self {
            kind: DockspaceNodeKind::Tabs { items, selected },
            central,
            layout_title: None,
        })
    }

    /// Creates tabs whose direct children may be panes or complete layouts.
    /// Direct panes are stored inline; structural children retain their boundary.
    #[must_use]
    pub fn composed_tabs(children: impl IntoIterator<Item = Self>) -> Self {
        let children: Vec<_> = children.into_iter().collect();
        let selected = (!children.is_empty()).then_some(0);
        Self {
            kind: DockspaceNodeKind::ComposedTabs { children, selected },
            central: false,
            layout_title: None,
        }
    }

    /// Creates composed tabs with an explicit direct-child selection index.
    /// Returns an error for an absent or out-of-bounds selection.
    pub fn composed_tabs_with_selection(
        children: impl IntoIterator<Item = Self>,
        selected: Option<usize>,
    ) -> Result<Self, DockspaceLayoutError> {
        let children: Vec<_> = children.into_iter().collect();
        if !match (children.is_empty(), selected) {
            (true, None) => true,
            (false, Some(index)) => index < children.len(),
            _ => false,
        } {
            return Err(DockspaceLayoutError::InvalidChildSelection { selected });
        }
        Ok(Self {
            kind: DockspaceNodeKind::ComposedTabs { children, selected },
            central: false,
            layout_title: None,
        })
    }

    /// Creates a retained central tabs anchor with pane or layout children.
    #[must_use]
    pub fn central_composed_tabs(children: impl IntoIterator<Item = Self>) -> Self {
        let mut node = Self::composed_tabs(children);
        node.central = true;
        node
    }

    /// Creates a central composed anchor with an explicit direct-child index.
    pub fn central_composed_tabs_with_selection(
        children: impl IntoIterator<Item = Self>,
        selected: Option<usize>,
    ) -> Result<Self, DockspaceLayoutError> {
        let mut node = Self::composed_tabs_with_selection(children, selected)?;
        node.central = true;
        Ok(node)
    }

    /// Creates a weighted N-ary split and normalizes positive relative weights.
    pub fn split(
        axis: DockspaceAxis,
        children: impl IntoIterator<Item = (Self, f64)>,
    ) -> Result<Self, DockspaceLayoutError> {
        let (children, weights): (Vec<_>, Vec<_>) = children.into_iter().unzip();
        if children.len() < 2 {
            return Err(DockspaceLayoutError::SplitTooFewChildren {
                children: children.len(),
            });
        }
        let weights = SplitWeight::normalize(weights)
            .map_err(|_| DockspaceLayoutError::InvalidSplitWeights)?;
        Ok(Self {
            kind: DockspaceNodeKind::Split {
                axis,
                children,
                weights,
            },
            central: false,
            layout_title: None,
        })
    }

    /// Creates an equal-share N-ary split.
    pub fn equal_split(
        axis: DockspaceAxis,
        children: impl IntoIterator<Item = Self>,
    ) -> Result<Self, DockspaceLayoutError> {
        Self::split(axis, children.into_iter().map(|child| (child, 1.0)))
    }

    /// Creates a row-major grid with equal shared tracks for its resolved dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`DockspaceLayoutError::InvalidGrid`] for an empty roster, overflowing
    /// fixed capacity, excess children, or a fixed track without a durable child slot.
    pub fn grid(
        shape: DockspaceGridShape,
        children: impl IntoIterator<Item = Self>,
    ) -> Result<Self, DockspaceLayoutError> {
        let children: Vec<_> = children.into_iter().collect();
        shape.validate_children(children.len())?;
        Ok(Self {
            kind: DockspaceNodeKind::Grid {
                shape,
                children,
                shares: None,
            },
            central: false,
            layout_title: None,
        })
    }

    /// Creates a row-major grid carrying both independently normalized track axes.
    ///
    /// # Errors
    ///
    /// Returns [`DockspaceLayoutError::InvalidGrid`] when the roster or share counts
    /// disagree with fixed dimensions, or the stored Auto dimensions cannot hold the roster.
    pub fn grid_with_shares(
        shape: DockspaceGridShape,
        children: impl IntoIterator<Item = Self>,
        shares: DockspaceGridShares,
    ) -> Result<Self, DockspaceLayoutError> {
        let children: Vec<_> = children.into_iter().collect();
        shares.validate_for(shape, children.len())?;
        Ok(Self {
            kind: DockspaceNodeKind::Grid {
                shape,
                children,
                shares: Some(shares),
            },
            central: false,
            layout_title: None,
        })
    }
}

/// Declarative content and stable identity of one docking root.
#[derive(Debug, Clone)]
pub struct DockspaceRootLayout {
    id: RootId,
    content: DockspaceNode,
    allocation: RootAllocation,
}

impl DockspaceRootLayout {
    /// Creates a root declaration.
    #[must_use]
    pub const fn new(id: RootId, content: DockspaceNode) -> Self {
        Self {
            id,
            content,
            allocation: RootAllocation::Weighted,
        }
    }

    /// Selects how pane constraints redistribute the root's weighted space.
    ///
    /// [`RootAllocation::PreferCentral`] requires a central tabs declaration;
    /// [`DockspaceLayout::new`] rejects it when the anchor is absent.
    #[must_use]
    pub const fn allocation(mut self, allocation: RootAllocation) -> Self {
        self.allocation = allocation;
        self
    }
}

/// Declarative contained-floating root and its durable surface-local bounds.
#[derive(Debug, Clone)]
pub struct DockspaceContainedLayout {
    id: FloatingPresentationId,
    root: DockspaceRootLayout,
    rect: LogicalRect,
}

impl DockspaceContainedLayout {
    /// Creates a contained-floating declaration.
    #[must_use]
    pub const fn new(
        id: FloatingPresentationId,
        root: DockspaceRootLayout,
        rect: LogicalRect,
    ) -> Self {
        Self { id, root, rect }
    }
}

/// Declarative main and contained roots presented by one logical surface.
#[derive(Debug, Clone)]
pub struct DockspaceSurfaceLayout {
    id: SurfaceId,
    main: Option<DockspaceRootLayout>,
    contained: Vec<DockspaceContainedLayout>,
}

impl DockspaceSurfaceLayout {
    /// Creates a surface with a main docking root.
    #[must_use]
    pub const fn new(id: SurfaceId, main: DockspaceRootLayout) -> Self {
        Self {
            id,
            main: Some(main),
            contained: Vec::new(),
        }
    }

    /// Creates a surface whose content consists only of contained-floating roots.
    ///
    /// The complete layout rejects this declaration if no contained root is attached.
    #[must_use]
    pub const fn rootless(id: SurfaceId) -> Self {
        Self {
            id,
            main: None,
            contained: Vec::new(),
        }
    }

    /// Appends a contained-floating root in back-to-front presentation order.
    #[must_use]
    pub fn with_contained(mut self, contained: DockspaceContainedLayout) -> Self {
        self.contained.push(contained);
        self
    }
}

/// A validated product layout backed by the core's canonical N-ary workspace.
#[derive(Clone)]
pub struct DockspaceLayout {
    pub(super) workspace: Workspace,
}

impl fmt::Debug for DockspaceLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceLayout")
            .field(
                "surfaces",
                &self
                    .workspace
                    .surfaces()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
            )
            .field(
                "roots",
                &self.workspace.roots().map(|(id, _)| id).collect::<Vec<_>>(),
            )
            .field("node_count", &self.workspace.nodes().count())
            .finish_non_exhaustive()
    }
}

impl DockspaceLayout {
    /// Compiles stable product declarations into one validated docking layout.
    ///
    /// # Errors
    ///
    /// Returns [`DockspaceLayoutError`] for duplicate stable identities, duplicate items, invalid
    /// central-region declarations, unusable contained bounds, or a topology rejected by core
    /// validation.
    pub fn new(
        surfaces: impl IntoIterator<Item = DockspaceSurfaceLayout>,
    ) -> Result<Self, DockspaceLayoutError> {
        let mut compiler = LayoutCompiler::default();
        for surface in surfaces {
            compiler.compile_surface(surface)?;
        }
        let workspace = compiler
            .builder
            .build()
            .map_err(|_| DockspaceLayoutError::InvalidTopology)?;
        Ok(Self { workspace })
    }

    /// Creates the valid empty product layout.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            workspace: Workspace::new(),
        }
    }

    /// Replaces the set of initially hidden owned items.
    ///
    /// Hiding retains tab order, split weights and presentation ownership.
    /// Selection moves to the next visible member, then the previous one. A
    /// fully hidden stack retains a legal logical selection for later showing.
    ///
    /// # Errors
    ///
    /// Returns [`DockspaceLayoutError::DuplicateHiddenItem`] for repeated input
    /// identities or [`DockspaceLayoutError::UnknownHiddenItem`] for items that
    /// this layout does not own.
    pub fn with_hidden_items(
        mut self,
        items: impl IntoIterator<Item = ItemId>,
    ) -> Result<Self, DockspaceLayoutError> {
        let mut hidden_items = BTreeSet::new();
        for item in items {
            if !hidden_items.insert(item) {
                return Err(DockspaceLayoutError::DuplicateHiddenItem { item });
            }
            if self.workspace.locate_item_by_id(item).is_none() {
                return Err(DockspaceLayoutError::UnknownHiddenItem { item });
            }
        }
        self.workspace
            .replace_hidden_items(hidden_items)
            .map_err(|_| DockspaceLayoutError::InvalidTopology)?;
        Ok(self)
    }

    /// Returns a read-only product view without exposing runtime graph identities.
    #[must_use]
    pub fn view(&self) -> DockspaceView<'_> {
        DockspaceView::new(&self.workspace)
    }

    pub(crate) fn into_workspace(self) -> Workspace {
        self.workspace
    }
}

/// Failure to compile a product layout declaration.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DockspaceLayoutError {
    /// The declared grid roster, dimensions, or shared tracks are invalid.
    #[error(transparent)]
    InvalidGrid(#[from] DockspaceGridError),
    /// A logical surface identity was declared more than once.
    #[error("surface {surface} is declared more than once")]
    DuplicateSurface {
        /// Repeated stable surface identity.
        surface: SurfaceId,
    },
    /// A root identity was declared more than once.
    #[error("root {root} is declared more than once")]
    DuplicateRoot {
        /// Repeated stable root identity.
        root: RootId,
    },
    /// A contained-floating identity was declared more than once.
    #[error("contained floating {floating} is declared more than once")]
    DuplicateContained {
        /// Repeated stable contained-floating identity.
        floating: FloatingPresentationId,
    },
    /// One real item is owned by more than one layout location.
    #[error("item {item} is declared more than once")]
    DuplicateItem {
        /// Repeated stable item identity.
        item: ItemId,
    },
    /// The hidden item set repeated one identity.
    #[error("hidden item {item} is declared more than once")]
    DuplicateHiddenItem {
        /// Repeated hidden identity.
        item: ItemId,
    },
    /// The hidden item set references content absent from this layout.
    #[error("hidden item {item} is not owned by the layout")]
    UnknownHiddenItem {
        /// Unknown hidden identity.
        item: ItemId,
    },
    /// A compact Tabs declaration has no valid selected item.
    #[error("tabs selection {selected:?} is not valid for its item roster")]
    InvalidSelection {
        /// Rejected selected item.
        selected: Option<ItemId>,
    },
    /// A composed tabs selection does not identify a direct child.
    #[error("composed tabs child selection {selected:?} is invalid")]
    InvalidChildSelection {
        /// Rejected direct-child index.
        selected: Option<usize>,
    },
    /// An N-ary split has fewer than two children.
    #[error("a split requires at least two children, got {children}")]
    SplitTooFewChildren {
        /// Supplied child count.
        children: usize,
    },
    /// Split weights are non-finite, non-positive, or cannot be normalized.
    #[error("split weights must be finite, positive, and normalizable")]
    InvalidSplitWeights,
    /// More than one Tabs container was marked as the root's central region.
    #[error("root {root} declares more than one central Tabs anchor")]
    MultipleCentralRegions {
        /// Root containing the conflicting declarations.
        root: RootId,
    },
    /// Central-first redistribution was requested without a central tabs anchor.
    #[error("root {root} prefers central allocation but has no central Tabs anchor")]
    CentralAllocationWithoutAnchor {
        /// Root with an incomplete allocation declaration.
        root: RootId,
    },
    /// A rootless surface has no contained-floating content.
    #[error("rootless surface {surface} must contain at least one floating root")]
    EmptyRootlessSurface {
        /// Empty stable surface identity.
        surface: SurfaceId,
    },
    /// A contained-floating rectangle has no usable area.
    #[error("contained floating {floating} must have positive width and height")]
    NonPositiveContainedRect {
        /// Contained-floating identity with unusable bounds.
        floating: FloatingPresentationId,
    },
    /// Core canonicalization or validation rejected the assembled topology.
    #[error("the declared docking topology is invalid")]
    InvalidTopology,
}

#[derive(Default)]
struct LayoutCompiler {
    builder: WorkspaceBuilder,
    surfaces: BTreeSet<SurfaceId>,
    roots: BTreeSet<RootId>,
    contained: BTreeSet<FloatingPresentationId>,
    items: BTreeSet<ItemId>,
}

impl LayoutCompiler {
    fn compile_surface(
        &mut self,
        surface: DockspaceSurfaceLayout,
    ) -> Result<(), DockspaceLayoutError> {
        if !self.surfaces.insert(surface.id) {
            return Err(DockspaceLayoutError::DuplicateSurface {
                surface: surface.id,
            });
        }
        if surface.main.is_none() && surface.contained.is_empty() {
            return Err(DockspaceLayoutError::EmptyRootlessSurface {
                surface: surface.id,
            });
        }

        let main_root = surface
            .main
            .map(|root| self.compile_root(root))
            .transpose()?;
        let mut contained_ids = Vec::with_capacity(surface.contained.len());
        for contained in surface.contained {
            if !self.contained.insert(contained.id) {
                return Err(DockspaceLayoutError::DuplicateContained {
                    floating: contained.id,
                });
            }
            if contained.rect.width() <= 0.0 || contained.rect.height() <= 0.0 {
                return Err(DockspaceLayoutError::NonPositiveContainedRect {
                    floating: contained.id,
                });
            }
            let root = self.compile_root(contained.root)?;
            self.builder
                .set_contained_floating(contained.id, ContainedFloating::new(root, contained.rect));
            contained_ids.push(contained.id);
        }

        self.builder.set_surface(
            surface.id,
            SurfacePresentation {
                main_root,
                contained: contained_ids,
            },
        );
        Ok(())
    }

    fn compile_root(&mut self, root: DockspaceRootLayout) -> Result<RootId, DockspaceLayoutError> {
        if !self.roots.insert(root.id) {
            return Err(DockspaceLayoutError::DuplicateRoot { root: root.id });
        }
        let mut central = None;
        let node = self.compile_node(root.content, root.id, &mut central)?;
        if root.allocation == RootAllocation::PreferCentral && central.is_none() {
            return Err(DockspaceLayoutError::CentralAllocationWithoutAnchor { root: root.id });
        }
        let record = match central {
            Some(central) => RootRecord::new(node).with_central(central),
            None => RootRecord::new(node),
        }
        .with_allocation(root.allocation);
        self.builder.set_root(root.id, record);
        Ok(root.id)
    }

    fn compile_node(
        &mut self,
        node: DockspaceNode,
        root: RootId,
        central: &mut Option<NodeId>,
    ) -> Result<NodeId, DockspaceLayoutError> {
        enum Frame {
            Enter {
                node: DockspaceNode,
                inline: bool,
            },
            FinishTabs {
                selected: Option<usize>,
                base: usize,
                central: bool,
                title: Option<Arc<str>>,
            },
            FinishSplit {
                axis: DockspaceAxis,
                weights: Vec<SplitWeight>,
                base: usize,
                central: bool,
                title: Option<Arc<str>>,
            },
            FinishGrid {
                shape: DockspaceGridShape,
                shares: Option<DockspaceGridShares>,
                base: usize,
                title: Option<Arc<str>>,
            },
        }
        enum Child {
            Inline(ItemId),
            Layout(NodeId),
        }
        let mut pending = vec![Frame::Enter {
            node,
            inline: false,
        }];
        let mut completed = Vec::new();
        while let Some(frame) = pending.pop() {
            let (mut value, is_central, title) = match frame {
                Frame::Enter { node, inline } => {
                    let (kind, is_central, title) = node.into_parts();
                    match kind {
                        DockspaceNodeKind::Pane(item) => {
                            if !self.items.insert(item) {
                                return Err(DockspaceLayoutError::DuplicateItem { item });
                            }
                            if inline {
                                completed.push(Child::Inline(item));
                                continue;
                            }
                            (Node::pane(item), false, None)
                        }
                        DockspaceNodeKind::Tabs { items, selected } => {
                            for item in &items {
                                if !self.items.insert(*item) {
                                    return Err(DockspaceLayoutError::DuplicateItem {
                                        item: *item,
                                    });
                                }
                            }
                            (
                                Node::tabs_with_selection(items, selected),
                                is_central,
                                title,
                            )
                        }
                        DockspaceNodeKind::ComposedTabs { children, selected } => {
                            pending.push(Frame::FinishTabs {
                                selected,
                                base: completed.len(),
                                central: is_central,
                                title,
                            });
                            pending.extend(
                                children
                                    .into_iter()
                                    .rev()
                                    .map(|node| Frame::Enter { node, inline: true }),
                            );
                            continue;
                        }
                        DockspaceNodeKind::Split {
                            axis,
                            children,
                            weights,
                        } => {
                            pending.push(Frame::FinishSplit {
                                axis,
                                weights,
                                base: completed.len(),
                                central: is_central,
                                title,
                            });
                            pending.extend(children.into_iter().rev().map(|node| Frame::Enter {
                                node,
                                inline: false,
                            }));
                            continue;
                        }
                        DockspaceNodeKind::Grid {
                            shape,
                            children,
                            shares,
                        } => {
                            pending.push(Frame::FinishGrid {
                                shape,
                                shares,
                                base: completed.len(),
                                title,
                            });
                            pending.extend(children.into_iter().rev().map(|node| Frame::Enter {
                                node,
                                inline: false,
                            }));
                            continue;
                        }
                    }
                }
                Frame::FinishTabs {
                    selected,
                    base,
                    central,
                    title,
                } => {
                    let items: Vec<_> = completed
                        .split_off(base)
                        .into_iter()
                        .map(|child| match child {
                            Child::Inline(item) => TabKey::for_item(item),
                            Child::Layout(node) => TabKey::for_layout(node),
                        })
                        .collect();
                    let selected = selected.and_then(|index| items.get(index).copied());
                    (Node::tabs_from_keys(items, selected), central, title)
                }
                Frame::FinishSplit {
                    axis,
                    weights,
                    base,
                    central,
                    title,
                } => {
                    let children = completed
                        .split_off(base)
                        .into_iter()
                        .map(|child| match child {
                            Child::Layout(node) => node,
                            Child::Inline(_) => unreachable!("split children are allocated"),
                        })
                        .collect();
                    (
                        Node::Split {
                            axis: axis.into(),
                            children,
                            weights,
                            layout_title: None,
                        },
                        central,
                        title,
                    )
                }
                Frame::FinishGrid {
                    shape,
                    shares,
                    base,
                    title,
                } => {
                    let children = completed
                        .split_off(base)
                        .into_iter()
                        .map(|child| match child {
                            Child::Layout(node) => node,
                            Child::Inline(_) => unreachable!("grid children are allocated"),
                        })
                        .collect();
                    (
                        Node::Grid {
                            shape,
                            children,
                            shares,
                            layout_title: None,
                        },
                        false,
                        title,
                    )
                }
            };
            value.set_layout_title(title);
            let compiled = self.builder.insert_node(value);
            if is_central && central.replace(compiled).is_some() {
                return Err(DockspaceLayoutError::MultipleCentralRegions { root });
            }
            completed.push(Child::Layout(compiled));
        }
        match completed.pop() {
            Some(Child::Layout(node)) => Ok(node),
            _ => Err(DockspaceLayoutError::InvalidTopology),
        }
    }
}
