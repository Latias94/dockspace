//! Splitter, structural gap, and junction presentation records.

use std::sync::Arc;

use crate::geometry::LogicalRect;
use crate::graph::{Axis, SplitWeight};
use crate::ids::{NodeId, RootId};
use crate::interaction::targeting::targets::SceneLayerKey;
use crate::presentation::hit::region::HitRegion;

/// Real structural owner of one shared track boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SplitterOwner {
    /// One axis of an ordinary Split.
    Split(NodeId),
    /// One shared row or column axis of a Grid.
    Grid { node: NodeId, axis: Axis },
}

impl SplitterOwner {
    /// Returns the actual structural owner.
    #[must_use]
    pub const fn node(self) -> NodeId {
        match self {
            Self::Split(node) | Self::Grid { node, .. } => node,
        }
    }
}

/// Structural identity of one rendered track boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SplitterSceneId {
    /// Owning root.
    pub root: RootId,
    /// Actual Split or Grid axis owning the separator.
    pub owner: SplitterOwner,
    /// Persistent preceding Split child or shared Grid track index.
    pub index: usize,
}

impl SplitterSceneId {
    /// Creates the identity of an ordinary Split boundary.
    #[must_use]
    pub const fn split(root: RootId, node: NodeId, index: usize) -> Self {
        Self {
            root,
            owner: SplitterOwner::Split(node),
            index,
        }
    }

    /// Creates the identity of a shared Grid boundary.
    #[must_use]
    pub const fn grid(root: RootId, node: NodeId, axis: Axis, index: usize) -> Self {
        Self {
            root,
            owner: SplitterOwner::Grid { node, axis },
            index,
        }
    }

    /// Returns the actual structural owner.
    #[must_use]
    pub const fn node(self) -> NodeId {
        self.owner.node()
    }
}

/// Exact resolved dimensions and structural cell map for a Grid boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GridBoundaryProof {
    rows: usize,
    columns: usize,
    children: Arc<[NodeId]>,
}

impl GridBoundaryProof {
    /// Returns the frozen shared row count.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }
    /// Returns the frozen shared column count.
    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns
    }
    /// Borrows the exact row-major structural roster, including hidden cells.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }
}

/// Core-compiled draw, hit, and resize semantics for one splitter.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitterRecord {
    id: SplitterSceneId,
    after_index: usize,
    active_ancestry: Option<usize>,
    draw_bounds: LogicalRect,
    hit: HitRegion,
    axis: Axis,
    before_bounds: LogicalRect,
    after_bounds: LogicalRect,
    before_minimum_extent: f64,
    after_minimum_extent: f64,
    before_maximum_extent: f64,
    after_maximum_extent: f64,
    child_extents: Arc<[f64]>,
    central_index: Option<usize>,
    weights: Arc<[SplitWeight]>,
    grid: Option<GridBoundaryProof>,
    layer: SceneLayerKey,
    operable: bool,
}

/// Presentation availability for one structural splitter gap.
///
/// Every visible adjacency in a compiled root is inventoried exactly once. Hidden
/// children keep their persistent positions without creating gaps. A collapsed gap
/// has no positive-area draw or hit rectangle and therefore cannot be painted
/// or interacted with, but remains explicit rather than silently disappearing
/// from the semantic plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SplitterGapPresentation {
    /// The gap has a positive-area draw record.
    Rendered,
    /// Compression left no positive-area splitter rectangle.
    Collapsed,
}

/// Complete structural inventory entry for one splitter gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SplitterGapRecord {
    id: SplitterSceneId,
    presentation: SplitterGapPresentation,
}

impl SplitterGapRecord {
    pub(crate) const fn new(id: SplitterSceneId, presentation: SplitterGapPresentation) -> Self {
        Self { id, presentation }
    }

    /// Returns the stable structural splitter-gap identity.
    #[must_use]
    pub const fn id(&self) -> SplitterSceneId {
        self.id
    }

    /// Returns whether this gap produced a positive-area splitter record.
    #[must_use]
    #[cfg(test)]
    pub const fn presentation(&self) -> SplitterGapPresentation {
        self.presentation
    }
}

/// Directional arm incident to one structural splitter junction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SplitterJunctionDirection {
    /// Splitter geometry extending above the junction center.
    North,
    /// Splitter geometry extending right of the junction center.
    East,
    /// Splitter geometry extending below the junction center.
    South,
    /// Splitter geometry extending left of the junction center.
    West,
}

/// Stable identity of one three- or four-arm splitter junction.
///
/// A splitter which passes through the junction occupies both opposite arms.
/// Directional identity preserves the complete touching frontier; it does not
/// collapse the junction to an arbitrary perpendicular pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SplitterJunctionId {
    north: Option<SplitterSceneId>,
    east: Option<SplitterSceneId>,
    south: Option<SplitterSceneId>,
    west: Option<SplitterSceneId>,
}

impl SplitterJunctionId {
    pub(crate) const fn new(
        north: Option<SplitterSceneId>,
        east: Option<SplitterSceneId>,
        south: Option<SplitterSceneId>,
        west: Option<SplitterSceneId>,
    ) -> Self {
        Self {
            north,
            east,
            south,
            west,
        }
    }

    /// Returns the splitter occupying one directional arm, when present.
    #[must_use]
    pub const fn arm(self, direction: SplitterJunctionDirection) -> Option<SplitterSceneId> {
        match direction {
            SplitterJunctionDirection::North => self.north,
            SplitterJunctionDirection::East => self.east,
            SplitterJunctionDirection::South => self.south,
            SplitterJunctionDirection::West => self.west,
        }
    }

    /// Returns the number of occupied directional arms.
    #[must_use]
    pub fn arm_count(self) -> usize {
        [self.north, self.east, self.south, self.west]
            .into_iter()
            .flatten()
            .count()
    }

    pub(crate) const fn root(self) -> RootId {
        if let Some(splitter) = self.north {
            splitter.root
        } else if let Some(splitter) = self.east {
            splitter.root
        } else if let Some(splitter) = self.south {
            splitter.root
        } else if let Some(splitter) = self.west {
            splitter.root
        } else {
            panic!("a splitter junction always has at least three arms")
        }
    }

    /// Returns every distinct incident splitter in canonical identity order.
    #[must_use]
    pub fn splitters(self) -> Vec<SplitterSceneId> {
        let mut splitters = [self.north, self.east, self.south, self.west]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        splitters.sort_unstable();
        splitters.dedup();
        splitters
    }
}

/// Exact shared hit region for one structural splitter junction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitterJunctionRecord {
    id: SplitterJunctionId,
    hit: HitRegion,
    layer: SceneLayerKey,
    operable: bool,
}

/// Structural splitter target resolved from one authoritative presentation plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitterResizeTarget {
    /// One ordinary splitter handle owns the point.
    Handle(SplitterSceneId),
    /// One three- or four-arm splitter junction owns all incident handles.
    Junction(SplitterJunctionId),
}

impl SplitterJunctionRecord {
    pub(crate) fn new(
        id: SplitterJunctionId,
        hit: HitRegion,
        layer: SceneLayerKey,
        operable: bool,
    ) -> Self {
        Self {
            id,
            hit,
            layer,
            operable,
        }
    }

    /// Returns the complete directional junction identity.
    #[must_use]
    pub const fn id(&self) -> SplitterJunctionId {
        self.id
    }

    /// Returns the exact region which owns atomic multi-handle resize input.
    #[must_use]
    pub const fn hit(&self) -> HitRegion {
        self.hit
    }

    /// Returns the roster-derived semantic layer.
    #[must_use]
    pub const fn layer(&self) -> SceneLayerKey {
        self.layer
    }

    /// Returns whether every incident splitter accepts an atomic resize.
    #[must_use]
    pub const fn operable(&self) -> bool {
        self.operable
    }
}

impl SplitterRecord {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: SplitterSceneId,
        draw_bounds: LogicalRect,
        hit: HitRegion,
        axis: Axis,
        before_bounds: LogicalRect,
        after_bounds: LogicalRect,
        before_minimum_extent: f64,
        after_minimum_extent: f64,
        before_maximum_extent: f64,
        after_maximum_extent: f64,
        child_extents: impl Into<Arc<[f64]>>,
        central_index: Option<usize>,
        weights: impl Into<Arc<[SplitWeight]>>,
        layer: SceneLayerKey,
        operable: bool,
    ) -> Self {
        Self {
            id,
            after_index: id.index + 1,
            active_ancestry: None,
            draw_bounds,
            hit,
            axis,
            before_bounds,
            after_bounds,
            before_minimum_extent,
            after_minimum_extent,
            before_maximum_extent,
            after_maximum_extent,
            child_extents: child_extents.into(),
            central_index,
            weights: weights.into(),
            grid: None,
            layer,
            operable,
        }
    }

    pub(crate) fn with_grid_proof(
        mut self,
        shape: (usize, usize),
        children: Arc<[NodeId]>,
    ) -> Self {
        self.grid = Some(GridBoundaryProof {
            rows: shape.0,
            columns: shape.1,
            children,
        });
        self
    }

    /// Borrows the frozen shape and cell map for an actual Grid boundary.
    pub(crate) const fn grid(&self) -> Option<&GridBoundaryProof> {
        self.grid.as_ref()
    }

    pub(crate) fn with_active_ancestry(mut self, ancestry: Option<usize>) -> Self {
        self.active_ancestry = ancestry;
        self
    }
    pub(crate) const fn active_ancestry(&self) -> Option<usize> {
        self.active_ancestry
    }

    pub(crate) fn with_after_index(mut self, after_index: usize) -> Self {
        self.after_index = after_index;
        self
    }

    /// Returns the persistent index of the next visible child.
    #[must_use]
    pub const fn after_index(&self) -> usize {
        self.after_index
    }

    /// Returns the stable split-gap identity.
    #[must_use]
    pub const fn id(&self) -> &SplitterSceneId {
        &self.id
    }

    /// Returns the thin painted splitter rectangle.
    #[must_use]
    pub const fn draw_bounds(&self) -> LogicalRect {
        self.draw_bounds
    }

    /// Returns the exact expanded resize hit region.
    #[must_use]
    pub const fn hit(&self) -> HitRegion {
        self.hit
    }

    /// Returns the split axis controlled by this record.
    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }

    /// Returns the adjacent child bounds before the splitter.
    #[must_use]
    pub const fn before_bounds(&self) -> LogicalRect {
        self.before_bounds
    }

    /// Returns the adjacent child bounds after the splitter.
    #[must_use]
    pub const fn after_bounds(&self) -> LogicalRect {
        self.after_bounds
    }

    /// Returns the measured minimum extent of the child before this splitter.
    #[must_use]
    pub const fn before_minimum_extent(&self) -> f64 {
        self.before_minimum_extent
    }

    /// Returns the measured minimum extent of the child after this splitter.
    #[must_use]
    pub const fn after_minimum_extent(&self) -> f64 {
        self.after_minimum_extent
    }

    /// Returns the measured maximum extent of the child before this splitter.
    #[must_use]
    pub const fn before_maximum_extent(&self) -> f64 {
        self.before_maximum_extent
    }

    /// Returns the measured maximum extent of the child after this splitter.
    #[must_use]
    pub const fn after_maximum_extent(&self) -> f64 {
        self.after_maximum_extent
    }

    /// Returns actual projected extents for every direct split child.
    #[must_use]
    pub fn child_extents(&self) -> &[f64] {
        &self.child_extents
    }

    /// Returns the child containing the root's central leaf, when one exists.
    #[must_use]
    pub const fn central_index(&self) -> Option<usize> {
        self.central_index
    }

    /// Returns the effective split weights represented by this plan.
    #[must_use]
    pub fn weights(&self) -> &[SplitWeight] {
        &self.weights
    }

    /// Returns the roster-derived semantic layer.
    #[must_use]
    pub const fn layer(&self) -> SceneLayerKey {
        self.layer
    }

    /// Returns whether any authoritative hit area remains on this record's layer.
    ///
    /// A fully obscured splitter remains in the draw plan so adapters can paint
    /// the complete topology, but it must not participate in pointer, keyboard,
    /// or accessibility interaction.
    #[must_use]
    pub const fn operable(&self) -> bool {
        self.operable
    }

    pub(in crate::presentation) fn retain_operability(&mut self, condition: bool) {
        self.operable &= condition;
    }
}
