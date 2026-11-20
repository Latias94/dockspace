//! Deterministic, renderer-neutral layout projection.

use std::collections::BTreeMap;
use std::sync::Arc;

use thiserror::Error;

use crate::engine::transition::WorkspaceVersion;
use crate::geometry::{Constraints, GeometryError, LogicalRect, LogicalSize, PaneSizing};
#[cfg(test)]
use crate::graph::WorkspaceData;
use crate::graph::command::{NodeSource, TrackWeights};
use crate::graph::error::{CommandError, ReferenceRole};
use crate::graph::workspace::WorkspaceIndex;
use crate::graph::{
    Axis, NORMALIZED_WEIGHT_TOLERANCE, Node, SplitWeight, Workspace, normalized_weight_sum,
};
use crate::ids::{NodeId, RootId};
use crate::presentation::config::PaneOverflowPolicy;

mod projection;
pub use projection::{
    ChildProjection, LayoutProjection, NodeMap, NodeResult, SplitMap, SplitProjection,
};
mod grid;
pub(crate) use grid::solve_grid_axis;
pub use grid::{GridMap, GridProjection, ResolvedGridShape};
mod reflow;
pub(crate) use reflow::FrozenRootLayout;

mod resize_geometry;
mod topology;
pub(crate) use resize_geometry::{ResizeGeometryFacts, measure_resize_geometry};

use crate::graph::root_structure::{RootNodeKind, RootStructure};

/// Minimum and maximum extent accepted by one child on a split axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisConstraint {
    min: f64,
    max: f64,
}

impl AxisConstraint {
    /// Creates a validated axis constraint.
    ///
    /// # Errors
    ///
    /// Returns an error when either bound is non-finite, the minimum is
    /// negative, or the maximum is smaller than the minimum.
    #[cfg(test)]
    pub fn new(min: f64, max: f64) -> Result<Self, AxisConstraintError> {
        if !min.is_finite() {
            return Err(AxisConstraintError::NonFiniteMinimum { value: min });
        }
        if !max.is_finite() {
            return Err(AxisConstraintError::NonFiniteMaximum { value: max });
        }
        if min < 0.0 {
            return Err(AxisConstraintError::NegativeMinimum { value: min });
        }
        if max < min {
            return Err(AxisConstraintError::MaximumBelowMinimum { min, max });
        }

        Ok(Self { min, max })
    }

    /// Returns the minimum extent.
    pub fn min(self) -> f64 {
        self.min
    }

    /// Returns the maximum extent.
    pub fn max(self) -> f64 {
        self.max
    }
}

/// Why an [`AxisConstraint`] could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[cfg(test)]
pub enum AxisConstraintError {
    /// The minimum is not a finite number.
    #[error("axis minimum is non-finite: {value}")]
    NonFiniteMinimum { value: f64 },
    /// The maximum is not a finite number.
    #[error("axis maximum is non-finite: {value}")]
    NonFiniteMaximum { value: f64 },
    /// The minimum is negative.
    #[error("axis minimum is negative: {value}")]
    NegativeMinimum { value: f64 },
    /// The maximum is smaller than the minimum.
    #[error("axis maximum {max} is smaller than minimum {min}")]
    MaximumBelowMinimum { min: f64, max: f64 },
}

/// A solved ordered split axis.
#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct AxisLayout {
    /// Extent assigned to each child in input order.
    pub sizes: Vec<f64>,
    /// Extent by which mandatory minima exceed the available extent.
    pub overflow: f64,
    /// Extent left unused because every child reached its maximum.
    pub unallocated: f64,
}

/// Explicit renderer-neutral measurements that affect layout geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutMetrics {
    splitter_thickness: f64,
    pane_overflow_policy: PaneOverflowPolicy,
}

impl LayoutMetrics {
    /// Creates validated layout metrics.
    ///
    /// # Errors
    ///
    /// Returns an error when `splitter_thickness` is non-finite or negative.
    pub fn new(splitter_thickness: f64) -> Result<Self, LayoutMetricsError> {
        if !splitter_thickness.is_finite() {
            return Err(LayoutMetricsError::NonFiniteSplitterThickness {
                value: splitter_thickness,
            });
        }
        if splitter_thickness < 0.0 {
            return Err(LayoutMetricsError::NegativeSplitterThickness {
                value: splitter_thickness,
            });
        }

        Ok(Self {
            splitter_thickness,
            pane_overflow_policy: PaneOverflowPolicy::default(),
        })
    }

    pub(crate) const fn with_pane_overflow_policy(mut self, policy: PaneOverflowPolicy) -> Self {
        self.pane_overflow_policy = policy;
        self
    }

    /// Returns the preferred extent reserved for every boundary between split children.
    ///
    /// Projection uses this exact thickness whenever it fits. If splitters alone
    /// exceed a split's available extent, they are compressed uniformly so the
    /// projection remains total for temporarily collapsed renderer surfaces.
    pub fn splitter_thickness(self) -> f64 {
        self.splitter_thickness
    }
}

/// Why [`LayoutMetrics`] could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum LayoutMetricsError {
    /// The splitter thickness is not a finite number.
    #[error("splitter thickness is non-finite: {value}")]
    NonFiniteSplitterThickness {
        /// Rejected thickness.
        value: f64,
    },
    /// The splitter thickness is negative.
    #[error("splitter thickness is negative: {value}")]
    NegativeSplitterThickness {
        /// Rejected thickness.
        value: f64,
    },
}

/// Transient split weights applied to one exact frozen split source.
///
/// Construction does not inspect a workspace. [`project_root_with_overrides`]
/// validates the exact source owner reference and complete weight collection
/// against the workspace being projected before using the override.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackWeightOverride<'state> {
    source: &'state NodeSource,
    weights: &'state TrackWeights,
}

impl<'state> TrackWeightOverride<'state> {
    /// Creates an override which will be validated during projection.
    #[must_use]
    pub const fn new(source: &'state NodeSource, weights: &'state TrackWeights) -> Self {
        Self { source, weights }
    }

    /// Returns the frozen split source.
    #[must_use]
    pub const fn source(self) -> &'state NodeSource {
        self.source
    }

    /// Returns the proposed transient weights.
    #[must_use]
    pub const fn weights(self) -> &'state TrackWeights {
        self.weights
    }
}

/// Failure to solve an axis or project a workspace root.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LayoutError {
    /// The available extent is not finite.
    #[error("axis extent is non-finite: {value}")]
    NonFiniteExtent { value: f64 },
    /// The available extent is negative.
    #[error("axis extent is negative: {value}")]
    NegativeExtent { value: f64 },
    /// The weight and constraint lists describe different child counts.
    #[error("axis has {weights} weights but {constraints} constraints")]
    LengthMismatch { weights: usize, constraints: usize },
    /// A child weight is not finite.
    #[error("axis weight {index} is non-finite: {value}")]
    NonFiniteWeight { index: usize, value: f64 },
    /// A child weight is negative.
    #[error("axis weight {index} is negative: {value}")]
    NegativeWeight { index: usize, value: f64 },
    /// The central child does not exist in the input list.
    #[error("central child index {index} is outside child count {child_count}")]
    CentralIndexOutOfBounds { index: usize, child_count: usize },
    /// No child can receive weighted space.
    #[error("axis weights have no positive total")]
    ZeroWeightTotal,
    /// Finite inputs overflowed while their aggregate was computed.
    #[error("{quantity} aggregate is not finite")]
    NonFiniteAggregate { quantity: &'static str },
    /// The requested stable root does not exist.
    #[error("workspace does not contain root {root}")]
    MissingRoot { root: RootId },
    /// A reachable runtime node does not exist.
    #[error("workspace does not contain node {node:?}")]
    MissingNode { node: NodeId },
    /// No explicit size constraints were supplied for a tabs leaf.
    #[error("layout constraints are missing for tabs node {node:?}")]
    MissingLeafConstraints { node: NodeId },
    /// The workspace contains a cycle and cannot be projected.
    #[error("workspace contains a cycle through node {node:?}")]
    Cycle { node: NodeId },
    /// A split's ordered children and weights do not have the same cardinality.
    #[error("split node {node:?} has {children} children but {weights} weights")]
    SplitCardinalityMismatch {
        /// Invalid split node.
        node: NodeId,
        /// Number of ordered children.
        children: usize,
        /// Number of weights.
        weights: usize,
    },
    /// The configured splitter thickness overflowed while being accumulated.
    #[error(
        "split node {node:?} cannot represent {splitter_count} splitters of thickness {thickness}"
    )]
    NonFiniteSplitterExtent {
        /// Invalid split node.
        node: NodeId,
        /// Number of splitters between the node's children.
        splitter_count: usize,
        /// Configured thickness of each splitter.
        thickness: f64,
    },
    /// A root's declared central leaf is outside that root's subtree.
    #[error("root {root} central node {central:?} is not reachable from its root node")]
    CentralNodeNotReachable { root: RootId, central: NodeId },
    /// A split-weight override belongs to a different root than the projection.
    #[error(
        "split-weight override for root {override_root} cannot be used while projecting root {projected_root}"
    )]
    TrackWeightOverrideRootMismatch {
        /// Root currently being projected.
        projected_root: RootId,
        /// Root captured by the override source.
        override_root: RootId,
    },
    /// A split-weight override source is no longer exact for this workspace.
    #[error("split-weight override source is invalid: {source}")]
    InvalidTrackWeightOverrideSource {
        /// Frozen-reference validation failure.
        #[source]
        source: CommandError,
    },
    /// More than one transient override names the same split.
    #[error("split node {node:?} has more than one transient weight override")]
    DuplicateTrackWeightOverride { node: NodeId },
    /// A transient weight override names a tabs leaf rather than a split.
    #[error("transient weight override node {node:?} is not a split")]
    TrackWeightOverrideNodeNotTracks { node: NodeId },
    /// Transient weights do not match the target split's child count.
    #[error(
        "split node {node:?} has {children} children but its transient override has {weights} weights"
    )]
    TrackWeightOverrideLengthMismatch {
        /// Target split.
        node: NodeId,
        /// Durable child count.
        children: usize,
        /// Override weight count.
        weights: usize,
    },
    /// Transient split weights are individually valid but not normalized.
    #[error("split node {node:?} transient weights are not normalized; sum is {sum}")]
    TrackWeightOverrideNotNormalized {
        /// Target split.
        node: NodeId,
        /// Accumulated override weight.
        sum: f64,
    },
    /// A private graph edit has not installed its current immutable root body.
    #[error("root {root} has an unfinished structural edit")]
    UnfinishedRoot { root: RootId },
    /// Validated geometry could not be constructed for the projection.
    #[error(transparent)]
    Geometry(#[from] GeometryError),
}

/// Solves one split axis with stable, bounded water filling.
///
/// Without a central child, weights are normalized over the full extent. With
/// a central child, every other weight is interpreted as a share of the full
/// extent and the central child receives the remaining share. If siblings sum
/// to more than one, they are normalized and the central child yields first.
/// The central child's own weight is intentionally ignored.
///
/// Infeasible minima and maxima are reported instead of silently changing the
/// constraints. All other invalid input is rejected before allocation begins.
///
/// # Errors
///
/// Returns an error for non-finite or negative scalar input, mismatched child
/// lists, an invalid central index, a zero total weight without a central
/// child, or an aggregate which overflows `f64`.
#[cfg(test)]
pub fn solve_axis(
    extent: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    central_index: Option<usize>,
) -> Result<AxisLayout, LayoutError> {
    validate_inputs(extent, weights, constraints, central_index)?;
    let mut sizes = vec![0.0; weights.len()];
    let mut eligible = Vec::new();
    let outcome = solve_validated_axis_into(
        extent,
        weights,
        constraints,
        central_index,
        &mut sizes,
        &mut eligible,
        PaneOverflowPolicy::Clip,
    )?;
    Ok(AxisLayout {
        sizes,
        overflow: outcome.overflow,
        unallocated: outcome.unallocated,
    })
}

#[derive(Clone, Copy)]
struct AxisOutcome {
    overflow: f64,
    unallocated: f64,
}

fn solve_axis_into(
    extent: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    central_index: Option<usize>,
    sizes: &mut [f64],
    eligible: &mut Vec<usize>,
    overflow_policy: PaneOverflowPolicy,
) -> Result<AxisOutcome, LayoutError> {
    validate_inputs(extent, weights, constraints, central_index)?;
    solve_validated_axis_into(
        extent,
        weights,
        constraints,
        central_index,
        sizes,
        eligible,
        overflow_policy,
    )
}

fn solve_validated_axis_into(
    extent: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    central_index: Option<usize>,
    sizes: &mut [f64],
    eligible: &mut Vec<usize>,
    overflow_policy: PaneOverflowPolicy,
) -> Result<AxisOutcome, LayoutError> {
    debug_assert_eq!(sizes.len(), weights.len());
    if weights.is_empty() {
        return Ok(AxisOutcome {
            overflow: 0.0,
            unallocated: extent,
        });
    }

    let min_total = checked_sum(
        constraints.iter().map(|constraint| constraint.min),
        "minimum",
    )?;
    let max_total = checked_sum(
        constraints.iter().map(|constraint| constraint.max),
        "maximum",
    )?;

    if min_total > extent {
        match overflow_policy {
            PaneOverflowPolicy::Clip => {
                for (size, constraint) in sizes.iter_mut().zip(constraints) {
                    *size = constraint.min;
                }
            }
            PaneOverflowPolicy::Compress => {
                let mut prefix = 0.0;
                let mut previous = 0.0;
                for (size, constraint) in sizes.iter_mut().zip(constraints) {
                    prefix += constraint.min;
                    let boundary = (extent * (prefix / min_total)).clamp(previous, extent);
                    *size = boundary - previous;
                    previous = boundary;
                }
            }
        }
        return Ok(AxisOutcome {
            overflow: min_total - extent,
            unallocated: 0.0,
        });
    }
    if max_total < extent {
        for (size, constraint) in sizes.iter_mut().zip(constraints) {
            *size = constraint.max;
        }
        return Ok(AxisOutcome {
            overflow: 0.0,
            unallocated: extent - max_total,
        });
    }

    desired_sizes_into(extent, weights, central_index, sizes)?;
    for (size, constraint) in sizes.iter_mut().zip(constraints) {
        *size = size.clamp(constraint.min, constraint.max);
    }

    let assigned = checked_sum(sizes.iter().copied(), "assigned size")?;
    if assigned < extent {
        grow_to_extent(
            sizes,
            extent - assigned,
            weights,
            constraints,
            central_index,
            eligible,
        );
    } else if assigned > extent {
        shrink_to_extent(
            sizes,
            assigned - extent,
            weights,
            constraints,
            central_index,
            eligible,
        );
    }

    let assigned = if assigned == extent {
        assigned
    } else {
        checked_sum(sizes.iter().copied(), "assigned size")?
    };
    let tolerance = allocation_tolerance(extent, min_total, max_total);
    let overflow = if assigned > extent + tolerance {
        assigned - extent
    } else {
        0.0
    };
    let unallocated = if assigned + tolerance < extent {
        extent - assigned
    } else {
        0.0
    };

    Ok(AxisOutcome {
        overflow,
        unallocated,
    })
}

/// Projects a validated workspace root into renderer-neutral logical rectangles.
///
/// `read_leaf` supplies explicit constraints for every reachable tabs node, once
/// per leaf in left-to-right depth-first measurement order. Returning `None`
/// rejects that exact leaf. The reader is consumed during this call and is not
/// retained by the owned projection.
/// Split subtree constraints are composed deterministically from those leaves;
/// callers do not provide duplicate constraints for internal nodes. `metrics`
/// explicitly includes the preferred splitter thickness in measurement. Projection
/// reserves that thickness when possible and otherwise applies the documented
/// uniform compression rule from [`LayoutMetrics::splitter_thickness`].
///
/// # Errors
///
/// Returns an error when the root is missing, a private structural edit is
/// unfinished, a leaf has no explicit constraints, constraint composition is
/// impossible, or axis allocation fails. Workspace completion owns structural
/// validation before this call can borrow the root body.
#[cfg(test)]
pub fn project_root(
    workspace: &Workspace,
    root: RootId,
    bounds: LogicalRect,
    read_leaf: impl FnMut(NodeId) -> Option<Constraints>,
    metrics: LayoutMetrics,
) -> Result<LayoutProjection, LayoutError> {
    project_root_with_overrides(workspace, root, bounds, read_leaf, metrics, &[])
}

/// Projects a root while applying explicit, validated transient split weights.
///
/// Each override must name a current split in `root` through a complete
/// [`NodeSource`] owner reference. Overrides affect only this projection and never
/// mutate durable [`Workspace`] state. Splits without an override keep their
/// durable weights.
///
/// # Errors
///
/// Returns the same errors as [`project_root`], plus an error when an override
/// belongs to another root, is stale, duplicated, names a non-split node, has
/// the wrong cardinality, or is not normalized.
#[cfg(test)]
pub fn project_root_with_overrides(
    workspace: &Workspace,
    root: RootId,
    bounds: LogicalRect,
    mut read_leaf: impl FnMut(NodeId) -> Option<Constraints>,
    metrics: LayoutMetrics,
    weight_overrides: &[TrackWeightOverride<'_>],
) -> Result<LayoutProjection, LayoutError> {
    workspace
        .root(root)
        .ok_or(LayoutError::MissingRoot { root })?;
    let weight_overrides =
        validate_track_weight_overrides(workspace, root, weight_overrides, None)?;
    let record = *workspace
        .root(root)
        .ok_or(LayoutError::MissingRoot { root })?;
    let topology = Arc::clone(
        workspace
            .root_structure(root)
            .ok_or(LayoutError::UnfinishedRoot { root })?,
    );
    let (values, central_leaf_row) = topology.measure_with_reader(
        root,
        record.central,
        &mut |node| workspace.node_has_visible_content(node),
        &mut |node| read_leaf(node).ok_or(LayoutError::MissingLeafConstraints { node }),
        metrics,
        false,
    )?;
    project_measurements(
        topology,
        values,
        CentralRoute::tree(preferred_central_row(record.allocation, central_leaf_row)),
        bounds,
        metrics,
        false,
        None,
        &weight_overrides,
        BTreeMap::new(),
    )
}

/// Measures complete pane sizing before root bounds supply finite unbounded maxima.
///
/// Preferred sizes remain available to explicit sizing actions; ordinary projection
/// continues to honor the workspace's stored or transient split weights.
pub(crate) fn measure_root_sizing<'workspace>(
    workspace: &'workspace Workspace,
    root: RootId,
    leaf_sizing: &BTreeMap<NodeId, PaneSizing>,
    metrics: LayoutMetrics,
) -> Result<RootMeasurement<'workspace>, LayoutError> {
    let mut measured = measure_root_with_reader(workspace, root, metrics, true, &mut |node| {
        let sizing = leaf_sizing
            .get(&node)
            .ok_or(LayoutError::MissingLeafConstraints { node })?;
        let minimum = LogicalSize::new(sizing.width().minimum(), sizing.height().minimum())?;
        Ok(Constraints::new(minimum, minimum)?)
    })?;
    measured.leaf_sizing = Some(leaf_sizing.clone());
    Ok(measured)
}

/// Measures scene minima once, before root bounds determine the uniform leaf maximum.
#[cfg(test)]
pub(crate) fn measure_root_minimums<'workspace>(
    workspace: &'workspace Workspace,
    root: RootId,
    leaf_minimums: &BTreeMap<NodeId, LogicalSize>,
    metrics: LayoutMetrics,
) -> Result<RootMeasurement<'workspace>, LayoutError> {
    measure_root(
        workspace,
        root,
        LeafMeasurements::Minimums(leaf_minimums),
        metrics,
    )
}

/// One root's measurements over stable, dense structural rows.
pub(crate) struct RootMeasurement<'workspace> {
    workspace: &'workspace Workspace,
    root: RootId,
    metrics: LayoutMetrics,
    minimum: LogicalSize,
    topology: Arc<RootStructure>,
    values: Vec<NodeResult>,
    central_leaf_row: Option<usize>,
    minimums_only: bool,
    leaf_sizing: Option<BTreeMap<NodeId, PaneSizing>>,
    tab_projections: BTreeMap<usize, (Option<usize>, f64)>,
}

impl RootMeasurement<'_> {
    pub(crate) fn with_tab_content_insets(mut self, insets: &BTreeMap<NodeId, f64>) -> Self {
        for (&row, (_, inset)) in &mut self.tab_projections {
            *inset = insets
                .get(&self.topology.nodes[row].id)
                .copied()
                .unwrap_or(0.0);
        }
        self
    }

    pub(crate) fn minimum(&self) -> LogicalSize {
        self.minimum
    }

    pub(crate) fn project(
        self,
        bounds: LogicalRect,
        weight_overrides: &[TrackWeightOverride<'_>],
    ) -> Result<LayoutProjection, LayoutError> {
        let weights =
            validate_track_weight_overrides(self.workspace, self.root, weight_overrides, None)?;
        self.project_with_weights(bounds, &weights)
    }

    pub(crate) fn project_with_index(
        self,
        bounds: LogicalRect,
        weight_overrides: &[TrackWeightOverride<'_>],
        index: &WorkspaceIndex,
        version: WorkspaceVersion,
    ) -> Result<LayoutProjection, LayoutError> {
        let weights = validate_track_weight_overrides(
            self.workspace,
            self.root,
            weight_overrides,
            Some((index, version)),
        )?;
        self.project_with_weights(bounds, &weights)
    }

    fn project_with_weights(
        self,
        bounds: LogicalRect,
        weight_overrides: &BTreeMap<NodeId, TrackWeights>,
    ) -> Result<LayoutProjection, LayoutError> {
        project_measurements(
            self.topology,
            self.values,
            CentralRoute::tree(self.central_leaf_row),
            bounds,
            self.metrics,
            self.minimums_only,
            self.leaf_sizing.as_ref(),
            weight_overrides,
            self.tab_projections,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn project_measurements(
    topology: Arc<RootStructure>,
    values: Vec<NodeResult>,
    central: CentralRoute,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
    minimums_only: bool,
    leaf_sizing: Option<&BTreeMap<NodeId, PaneSizing>>,
    weight_overrides: &BTreeMap<NodeId, TrackWeights>,
    tab_projections: BTreeMap<usize, (Option<usize>, f64)>,
) -> Result<LayoutProjection, LayoutError> {
    let basis = ProjectionBasis {
        central,
        metrics,
        cap_frames: leaf_sizing.is_some(),
        tabs: (!tab_projections.is_empty()).then(|| Arc::new(tab_projections)),
    };
    let reflow = FrozenRootLayout::capture(
        &topology,
        &values,
        bounds,
        &basis,
        minimums_only,
        leaf_sizing,
        weight_overrides,
    );
    let mut projection = project_measurements_inner(
        topology,
        values,
        bounds,
        &basis,
        minimums_only,
        leaf_sizing,
        weight_overrides,
    )?;
    projection.auto_reflow = reflow;
    Ok(projection)
}

#[allow(clippy::too_many_arguments)]
fn project_measurements_inner(
    topology: Arc<RootStructure>,
    mut values: Vec<NodeResult>,
    bounds: LogicalRect,
    basis: &ProjectionBasis,
    minimums_only: bool,
    leaf_sizing: Option<&BTreeMap<NodeId, PaneSizing>>,
    weight_overrides: &BTreeMap<NodeId, TrackWeights>,
) -> Result<LayoutProjection, LayoutError> {
    let metrics = basis.metrics;
    if minimums_only {
        let minimum = values[0].constraints.min();
        let leaf_maximum = LogicalSize::new(
            bounds.width().max(minimum.width()),
            bounds.height().max(minimum.height()),
        )?;
        let mut bounded_axes = [false; 2];
        if let Some(sizings) = leaf_sizing {
            for (node, sizing) in sizings {
                if topology.row(node).is_some_and(|row| values[row].visible) {
                    bounded_axes[0] |= sizing.width().maximum().is_some();
                    bounded_axes[1] |= sizing.height().maximum().is_some();
                }
            }
        }
        let mut child_values = Vec::new();
        for &row in &topology.postorder {
            if !values[row].visible {
                continue;
            }
            let maximum = match &topology.nodes[row].kind {
                RootNodeKind::Pane { .. } | RootNodeKind::Tabs { .. } => match leaf_sizing {
                    Some(sizing) => {
                        let sizing = sizing.get(&topology.nodes[row].id).ok_or(
                            LayoutError::MissingLeafConstraints {
                                node: topology.nodes[row].id,
                            },
                        )?;
                        LogicalSize::new(
                            sizing.width().maximum().unwrap_or(leaf_maximum.width()),
                            sizing.height().maximum().unwrap_or(leaf_maximum.height()),
                        )?
                    }
                    None => leaf_maximum,
                },
                RootNodeKind::Split {
                    payload, children, ..
                } => {
                    let child_rows = &topology.children[children.clone()];
                    let binary_children;
                    let child_measurements: &[SubtreeMeasurement] = match child_rows {
                        [first, second] if values[*first].visible && values[*second].visible => {
                            binary_children = [
                                SubtreeMeasurement {
                                    constraints: values[*first].constraints,
                                    contains_central: false,
                                },
                                SubtreeMeasurement {
                                    constraints: values[*second].constraints,
                                    contains_central: false,
                                },
                            ];
                            &binary_children
                        }
                        _ => {
                            child_values.clear();
                            child_values.extend(child_rows.iter().filter_map(|&row| {
                                values[row].visible.then_some(SubtreeMeasurement {
                                    constraints: values[row].constraints,
                                    contains_central: false,
                                })
                            }));
                            &child_values
                        }
                    };
                    compose_frame_maximum(
                        payload.axis(),
                        child_measurements,
                        total_splitter_extent(
                            topology.nodes[row].id,
                            child_measurements.len(),
                            metrics,
                        )?,
                        bounded_axes,
                    )?
                }
                RootNodeKind::Grid { payload, .. } => {
                    if let crate::model::DockspaceGridShape::Fixed { rows, columns } =
                        payload.shape()
                    {
                        grid::grid_constraints(
                            &topology,
                            row,
                            &values,
                            ResolvedGridShape {
                                rows: rows.get(),
                                columns: columns.get(),
                            },
                            metrics,
                            leaf_maximum,
                        )?
                        .max()
                    } else {
                        leaf_maximum
                    }
                }
            };
            let measurement = &mut values[row];
            measurement.constraints = Constraints::new(measurement.constraints.min(), maximum)?;
        }
    }
    let mut projection = LayoutProjection {
        nodes: NodeMap::from_values(Arc::clone(&topology), values),
        splits: SplitMap::new(Arc::clone(&topology)),
        grids: GridMap::default(),
        auto_reflow: None,
        basis: basis.clone(),
    };
    basis.project_into(bounds, weight_overrides, &mut projection, None)?;
    if !topology.grid_rows.is_empty() {
        let shapes = projection
            .grids
            .iter()
            .map(|(&node, grid)| (node, grid.shape))
            .collect();
        grid::remeasure_frozen_grids(
            &topology,
            &mut projection.nodes.values,
            &shapes,
            metrics,
            bounds.size(),
            basis.tabs.as_deref().unwrap_or(&BTreeMap::new()),
        )?;
        basis.project_into(bounds, weight_overrides, &mut projection, Some(&shapes))?;
    }
    Ok(projection)
}

#[cfg(test)]
fn project_draft(
    draft: &crate::graph::WorkspaceBuilder,
    root: RootId,
    bounds: LogicalRect,
    leaves: LeafMeasurements<'_>,
    metrics: LayoutMetrics,
) -> Result<LayoutProjection, LayoutError> {
    let (topology, measurements) =
        RootStructure::compile_draft(draft.draft(), root, leaves, metrics)?;
    let prefers_central = draft
        .draft()
        .root(root)
        .is_some_and(|record| record.allocation == crate::model::RootAllocation::PreferCentral);
    let central_rows = measurements
        .iter()
        .enumerate()
        .filter_map(|(row, value)| (prefers_central && value.contains_central).then_some(row))
        .collect::<Vec<_>>();
    let empty_rect = LogicalRect::new(0.0, 0.0, 0.0, 0.0)?;
    let values = measurements
        .into_iter()
        .map(|value| NodeResult {
            visible: true,
            slot: empty_rect,
            rect: empty_rect,
            constraints: value.constraints,
        })
        .collect();
    project_measurements(
        Arc::new(topology),
        values,
        CentralRoute::Raw(central_rows),
        bounds,
        metrics,
        matches!(leaves, LeafMeasurements::Minimums(_)),
        None,
        &BTreeMap::new(),
        BTreeMap::new(),
    )
}

fn validate_track_weight_overrides(
    workspace: &Workspace,
    root: RootId,
    overrides: &[TrackWeightOverride<'_>],
    index: Option<(&WorkspaceIndex, WorkspaceVersion)>,
) -> Result<BTreeMap<NodeId, TrackWeights>, LayoutError> {
    let mut validated = BTreeMap::new();
    for weight_override in overrides {
        let source = weight_override.source;
        if source.root() != root {
            return Err(LayoutError::TrackWeightOverrideRootMismatch {
                projected_root: root,
                override_root: source.root(),
            });
        }
        let role = ReferenceRole::Source;
        if let Some((index, version)) = index {
            index.verify_reference(
                workspace,
                version,
                source.root(),
                source.node(),
                source.reference(),
                role,
            )
        } else {
            workspace.verify_reference(source.root(), source.node(), source.reference(), role)
        }
        .map_err(|source| LayoutError::InvalidTrackWeightOverrideSource { source })?;

        if validated.contains_key(&source.node()) {
            return Err(LayoutError::DuplicateTrackWeightOverride {
                node: source.node(),
            });
        }
        let weights = weight_override.weights;
        match (workspace.node(source.node()), weights) {
            (Some(Node::Split { children, .. }), TrackWeights::Split(weights)) => {
                if children.len() != weights.len() {
                    return Err(LayoutError::TrackWeightOverrideLengthMismatch {
                        node: source.node(),
                        children: children.len(),
                        weights: weights.len(),
                    });
                }
                let sum = normalized_weight_sum(weights.iter().map(|weight| weight.get()));
                if !sum.is_finite() || (sum - 1.0).abs() > NORMALIZED_WEIGHT_TOLERANCE {
                    return Err(LayoutError::TrackWeightOverrideNotNormalized {
                        node: source.node(),
                        sum,
                    });
                }
            }
            (
                Some(Node::Grid {
                    shape, children, ..
                }),
                TrackWeights::Grid(shares),
            ) => {
                let rows = shares.rows().len();
                let columns = shares.columns().len();
                let cardinality = rows.checked_mul(columns).is_some_and(|capacity| {
                    capacity >= children.len()
                        && children.len() >= columns
                        && children.len() > (rows - 1) * columns
                });
                let fixed_matches = match shape {
                    crate::model::DockspaceGridShape::Fixed {
                        rows: expected_rows,
                        columns: expected_columns,
                    } => rows == expected_rows.get() && columns == expected_columns.get(),
                    crate::model::DockspaceGridShape::Auto => true,
                };
                if !cardinality || !fixed_matches {
                    return Err(LayoutError::TrackWeightOverrideLengthMismatch {
                        node: source.node(),
                        children: children.len(),
                        weights: rows.saturating_mul(columns),
                    });
                }
            }
            _ => {
                return Err(LayoutError::TrackWeightOverrideNodeNotTracks {
                    node: source.node(),
                });
            }
        }
        validated.insert(source.node(), weights.clone());
    }

    Ok(validated)
}

#[derive(Debug, Clone, Copy)]
struct SubtreeMeasurement {
    constraints: Constraints,
    #[allow(
        dead_code,
        reason = "Frozen internal projection retains this fact for protocol validation and test inspection"
    )]
    contains_central: bool,
}

#[derive(Clone, Copy)]
#[cfg(test)]
enum LeafMeasurements<'leaves> {
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    Constraints(&'leaves BTreeMap<NodeId, Constraints>),
    Minimums(&'leaves BTreeMap<NodeId, LogicalSize>),
}

#[cfg(test)]
impl LeafMeasurements<'_> {
    fn get(self, node: NodeId) -> Result<Constraints, LayoutError> {
        match self {
            Self::Constraints(constraints) => constraints
                .get(&node)
                .copied()
                .ok_or(LayoutError::MissingLeafConstraints { node }),
            Self::Minimums(minimums) => {
                let minimum = minimums
                    .get(&node)
                    .copied()
                    .ok_or(LayoutError::MissingLeafConstraints { node })?;
                // Point constraints hold minima until root bounds supply leaf maxima.
                Ok(Constraints::new(minimum, minimum)?)
            }
        }
    }
}

#[cfg(test)]
fn measure_root<'workspace>(
    workspace: &'workspace Workspace,
    root: RootId,
    leaves: LeafMeasurements<'_>,
    metrics: LayoutMetrics,
) -> Result<RootMeasurement<'workspace>, LayoutError> {
    let minimums_only = matches!(leaves, LeafMeasurements::Minimums(_));
    measure_root_with_reader(workspace, root, metrics, minimums_only, &mut |node| {
        leaves.get(node)
    })
}

fn measure_root_with_reader<'workspace>(
    workspace: &'workspace Workspace,
    root: RootId,
    metrics: LayoutMetrics,
    minimums_only: bool,
    read_leaf: &mut impl FnMut(NodeId) -> Result<Constraints, LayoutError>,
) -> Result<RootMeasurement<'workspace>, LayoutError> {
    let record = *workspace
        .root(root)
        .ok_or(LayoutError::MissingRoot { root })?;
    let topology = Arc::clone(
        workspace
            .root_structure(root)
            .ok_or(LayoutError::UnfinishedRoot { root })?,
    );
    let (values, central_leaf_row) = topology.measure_with_reader(
        root,
        record.central,
        &mut |node| workspace.node_has_visible_content(node),
        read_leaf,
        metrics,
        minimums_only,
    )?;
    let mut values = values;
    let mut tab_projections = BTreeMap::new();
    if topology.has_composition() {
        let mut active = vec![false; topology.nodes.len()];
        active[0] = true;
        for (row, node) in topology.nodes.iter().enumerate() {
            values[row].visible &= active[row];
            match &node.kind {
                RootNodeKind::Tabs { children } => {
                    let selected = workspace
                        .visible_tab_selection(node.id)
                        .and_then(crate::ids::TabKey::layout)
                        .and_then(|node| topology.row(&node));
                    tab_projections.insert(row, (selected, 0.0));
                    for &child in &topology.children[children.clone()] {
                        active[child] = values[row].visible && selected == Some(child);
                    }
                }
                RootNodeKind::Split { children, .. } | RootNodeKind::Grid { children, .. } => {
                    for &child in &topology.children[children.clone()] {
                        active[child] = values[row].visible;
                    }
                }
                RootNodeKind::Pane { .. } => {}
            }
        }
    }
    Ok(RootMeasurement {
        workspace,
        root,
        metrics,
        minimum: values[0].constraints.min(),
        topology,
        values,
        central_leaf_row: preferred_central_row(record.allocation, central_leaf_row),
        minimums_only,
        leaf_sizing: None,
        tab_projections,
    })
}

fn compose_constraints(
    node: NodeId,
    axis: Axis,
    children: &[SubtreeMeasurement],
    metrics: LayoutMetrics,
    minimums_only: bool,
) -> Result<Constraints, LayoutError> {
    if children.is_empty() {
        return Err(LayoutError::SplitCardinalityMismatch {
            node,
            children: 0,
            weights: 0,
        });
    }
    let splitter_extent = total_splitter_extent(node, children.len(), metrics)?;
    let minimum = compose_minimum(axis, children, splitter_extent)?;
    let maximum = if minimums_only {
        minimum
    } else {
        compose_maximum(axis, children, splitter_extent)?
    };
    Ok(Constraints::new(minimum, maximum)?)
}

fn compose_minimum(
    axis: Axis,
    children: &[SubtreeMeasurement],
    splitter_extent: f64,
) -> Result<LogicalSize, LayoutError> {
    let min_widths = children.iter().map(|child| child.constraints.min().width());
    let min_heights = children
        .iter()
        .map(|child| child.constraints.min().height());

    let (width, height) = match axis {
        Axis::Horizontal => (
            checked_sum(
                min_widths.chain(std::iter::once(splitter_extent)),
                "subtree minimum width",
            )?,
            min_heights.fold(0.0, f64::max),
        ),
        Axis::Vertical => (
            min_widths.fold(0.0, f64::max),
            checked_sum(
                min_heights.chain(std::iter::once(splitter_extent)),
                "subtree minimum height",
            )?,
        ),
    };
    Ok(LogicalSize::new(width, height)?)
}

fn compose_maximum(
    axis: Axis,
    children: &[SubtreeMeasurement],
    splitter_extent: f64,
) -> Result<LogicalSize, LayoutError> {
    // Complete-constraint protocol tests retain a shared cross-axis rectangle.
    compose_maximum_with_cross(axis, children, splitter_extent, CrossMaximum::Intersection)
}

fn compose_frame_maximum(
    axis: Axis,
    children: &[SubtreeMeasurement],
    splitter_extent: f64,
    bounded_axes: [bool; 2],
) -> Result<LogicalSize, LayoutError> {
    let cross_is_bounded = match axis {
        Axis::Horizontal => bounded_axes[1],
        Axis::Vertical => bounded_axes[0],
    };
    // A wholly unbounded axis keeps the legacy finite stand-in, which already
    // covers the root slot and minimum. Declared caps bound independent frames.
    let cross = if cross_is_bounded {
        CrossMaximum::Envelope
    } else {
        CrossMaximum::Intersection
    };
    compose_maximum_with_cross(axis, children, splitter_extent, cross)
}

#[derive(Clone, Copy)]
enum CrossMaximum {
    Intersection,
    Envelope,
}

fn compose_maximum_with_cross(
    axis: Axis,
    children: &[SubtreeMeasurement],
    splitter_extent: f64,
    cross: CrossMaximum,
) -> Result<LogicalSize, LayoutError> {
    let main_extents = children.iter().map(|child| match axis {
        Axis::Horizontal => child.constraints.max().width(),
        Axis::Vertical => child.constraints.max().height(),
    });
    let quantity = match axis {
        Axis::Horizontal => "subtree maximum width",
        Axis::Vertical => "subtree maximum height",
    };
    let main = checked_sum(
        main_extents.chain(std::iter::once(splitter_extent)),
        quantity,
    )?;
    let cross_extents = children.iter().map(|child| match axis {
        Axis::Horizontal => child.constraints.max().height(),
        Axis::Vertical => child.constraints.max().width(),
    });
    let cross = match cross {
        CrossMaximum::Intersection => cross_extents.fold(f64::INFINITY, f64::min),
        CrossMaximum::Envelope => cross_extents.fold(0.0, f64::max),
    };
    let (width, height) = match axis {
        Axis::Horizontal => (main, cross),
        Axis::Vertical => (cross, main),
    };
    Ok(LogicalSize::new(width, height)?)
}

fn total_splitter_extent(
    node: NodeId,
    child_count: usize,
    metrics: LayoutMetrics,
) -> Result<f64, LayoutError> {
    let splitter_count = child_count.saturating_sub(1);
    let thickness = metrics.splitter_thickness();
    let extent = std::iter::repeat_n(thickness, splitter_count).sum::<f64>();
    if extent.is_finite() {
        Ok(extent)
    } else {
        Err(LayoutError::NonFiniteSplitterExtent {
            node,
            splitter_count,
            thickness,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CentralRoute {
    Absent,
    Tree {
        leaf: usize,
        next: usize,
    },
    #[cfg(test)]
    Raw(Vec<usize>),
}

fn preferred_central_row(
    allocation: crate::model::RootAllocation,
    central: Option<usize>,
) -> Option<usize> {
    match allocation {
        crate::model::RootAllocation::Weighted => None,
        crate::model::RootAllocation::PreferCentral => central,
    }
}

impl CentralRoute {
    fn tree(leaf: Option<usize>) -> Self {
        match leaf {
            Some(leaf) => Self::Tree { leaf, next: 0 },
            None => Self::Absent,
        }
    }
}

struct SplitProjectionState {
    row: usize,
    next_child: usize,
    cursor: f64,
    splitter_thickness: f64,
    pending_child: bool,
}

enum ProjectionFrame {
    Enter { row: usize, bounds: LogicalRect },
    ContinueSplit(SplitProjectionState),
}

#[derive(Default)]
struct ProjectionScratch {
    constraints: Vec<AxisConstraint>,
    weights: Vec<f64>,
    sizes: Vec<f64>,
    eligible: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct ProjectionBasis {
    central: CentralRoute,
    metrics: LayoutMetrics,
    cap_frames: bool,
    tabs: Option<Arc<BTreeMap<usize, (Option<usize>, f64)>>>,
}

impl ProjectionBasis {
    fn project_into(
        &self,
        bounds: LogicalRect,
        weights: &BTreeMap<NodeId, TrackWeights>,
        projection: &mut LayoutProjection,
        grid_shapes: Option<&BTreeMap<NodeId, ResolvedGridShape>>,
    ) -> Result<(), LayoutError> {
        let topology = Arc::clone(&projection.nodes.topology);
        project_subtree(
            &topology,
            bounds,
            self.metrics,
            &mut self.central.clone(),
            self.cap_frames,
            weights,
            projection,
            self.tabs.as_deref().unwrap_or(&BTreeMap::new()),
            grid_shapes,
        )
    }
}

fn project_subtree(
    topology: &RootStructure,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
    central: &mut CentralRoute,
    cap_frames: bool,
    weight_overrides: &BTreeMap<NodeId, TrackWeights>,
    projection: &mut LayoutProjection,
    tab_projections: &BTreeMap<usize, (Option<usize>, f64)>,
    grid_shapes: Option<&BTreeMap<NodeId, ResolvedGridShape>>,
) -> Result<(), LayoutError> {
    let mut stack = Vec::with_capacity(topology.nodes.len().min(8));
    stack.push(ProjectionFrame::Enter { row: 0, bounds });
    let mut scratch = ProjectionScratch::default();
    let mut visited_prefix = 0;
    while let Some(frame) = stack.pop() {
        match frame {
            ProjectionFrame::Enter { row, bounds } => {
                assert!(
                    row <= visited_prefix,
                    "projection row skips its first-visit prefix"
                );
                if row == visited_prefix {
                    visited_prefix += 1;
                }
                if let Some(state) = prepare_split_projection(
                    topology,
                    row,
                    bounds,
                    metrics,
                    central,
                    cap_frames,
                    weight_overrides,
                    projection,
                    &mut scratch,
                )? {
                    stack.push(ProjectionFrame::ContinueSplit(state));
                } else if matches!(&topology.nodes[row].kind, RootNodeKind::Grid { .. }) {
                    let bounds = projection.nodes.at_row(row).rect;
                    let placements = grid::prepare_grid_projection(
                        topology,
                        row,
                        bounds,
                        metrics,
                        central,
                        weight_overrides,
                        grid_shapes,
                        projection,
                    )?;
                    for (row, bounds) in placements.into_iter().rev() {
                        stack.push(ProjectionFrame::Enter { row, bounds });
                    }
                } else if let RootNodeKind::Tabs { children } = &topology.nodes[row].kind
                    && !children.is_empty()
                {
                    let (selected, inset) =
                        tab_projections.get(&row).copied().unwrap_or((None, 0.0));
                    let rect = projection.nodes.at_row(row).rect;
                    let content = rect.inset_top(inset)?;
                    for &child in topology.children[children.clone()].iter().rev() {
                        let child_bounds =
                            if selected == Some(child) && projection.nodes.at_row(child).visible {
                                content
                            } else {
                                LogicalRect::new(content.x(), content.y(), 0.0, 0.0)?
                            };
                        stack.push(ProjectionFrame::Enter {
                            row: child,
                            bounds: child_bounds,
                        });
                    }
                }
            }
            ProjectionFrame::ContinueSplit(state) => {
                advance_split_projection(topology, state, &mut stack, projection)?
            }
        }
    }
    #[cfg(debug_assertions)]
    debug_assert_eq!(visited_prefix, topology.nodes.len());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare_split_projection(
    topology: &RootStructure,
    row: usize,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
    central: &mut CentralRoute,
    cap_frames: bool,
    weight_overrides: &BTreeMap<NodeId, TrackWeights>,
    projection: &mut LayoutProjection,
    scratch: &mut ProjectionScratch,
) -> Result<Option<SplitProjectionState>, LayoutError> {
    let node = &topology.nodes[row];
    let node_id = node.id;
    let slot = bounds;
    let bounds = if cap_frames {
        let maximum = projection.nodes.at_row(row).constraints.max();
        LogicalRect::new(
            slot.x(),
            slot.y(),
            slot.width().min(maximum.width()),
            slot.height().min(maximum.height()),
        )?
    } else {
        // The complete-constraint test adapter retains the frozen traversal contract.
        slot
    };
    projection.nodes.record_rect(row, slot, bounds);
    let RootNodeKind::Split {
        payload,
        children: child_range,
        split,
        ..
    } = &node.kind
    else {
        match central {
            CentralRoute::Tree { leaf, .. } if *leaf == row => *central = CentralRoute::Absent,
            CentralRoute::Tree { leaf, next } if *next == row => {
                if let RootNodeKind::Tabs { children } = &node.kind {
                    if let Some(&child) = topology.children[children.clone()]
                        .iter()
                        .rev()
                        .find(|&&child| child <= *leaf)
                    {
                        *next = child;
                    }
                }
            }
            _ => {}
        }
        return Ok(None);
    };
    let axis = payload.axis();
    let children = &topology.children[child_range.clone()];
    let axis_constraint = |&row: &usize| {
        let constraints = projection.nodes.at_row(row).constraints;
        match axis {
            Axis::Horizontal => AxisConstraint {
                min: constraints.min().width(),
                max: constraints.max().width(),
            },
            Axis::Vertical => AxisConstraint {
                min: constraints.min().height(),
                max: constraints.max().height(),
            },
        }
    };
    let binary_constraints;
    let axis_constraints: &[AxisConstraint] = match children {
        [first, second] => {
            binary_constraints = [axis_constraint(first), axis_constraint(second)];
            &binary_constraints
        }
        _ => {
            scratch.constraints.clear();
            scratch
                .constraints
                .extend(children.iter().map(axis_constraint));
            &scratch.constraints
        }
    };
    let mut visible_count = 0;
    for ((child, constraint), &row) in projection.splits.children[child_range.clone()]
        .iter_mut()
        .zip(axis_constraints)
        .zip(children)
    {
        child.visible = projection.nodes.at_row(row).visible;
        visible_count += usize::from(child.visible);
        child.minimum = constraint.min();
        child.maximum = constraint.max();
        child.following_gap = None;
    }
    let binary_weights;
    let stored_weights =
        if let Some(TrackWeights::Split(overrides)) = weight_overrides.get(&node_id) {
            scratch.weights.clear();
            scratch
                .weights
                .extend(overrides.iter().map(|weight| weight.get()));
            scratch.weights.as_slice()
        } else {
            match (children, payload.weights()) {
                ([_, _], [first, second]) => {
                    binary_weights = [first.get(), second.get()];
                    binary_weights.as_slice()
                }
                (_, weights) => {
                    scratch.weights.clear();
                    scratch
                        .weights
                        .extend(weights.iter().map(|weight| weight.get()));
                    scratch.weights.as_slice()
                }
            }
        };
    let visible_weights;
    let axis_weights = if visible_count == children.len() {
        stored_weights
    } else {
        let total =
            normalized_weight_sum(stored_weights.iter().zip(children).filter_map(
                |(&weight, &row)| projection.nodes.at_row(row).visible.then_some(weight),
            ));
        visible_weights = stored_weights
            .iter()
            .zip(children)
            .map(|(&weight, &row)| {
                if projection.nodes.at_row(row).visible {
                    weight / total
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        visible_weights.as_slice()
    };
    let central_index = match central {
        CentralRoute::Tree { leaf, next } if *next == row => {
            // The frontier's subtree contains this leaf; child starts are preorder rows.
            let index = children.iter().rposition(|&child| child <= *leaf);
            if let Some(index) = index {
                *next = children[index];
            }
            index
        }
        #[cfg(test)]
        CentralRoute::Raw(rows) => children
            .iter()
            .position(|row| rows.binary_search(row).is_ok()),
        _ => None,
    };
    let extent = match axis {
        Axis::Horizontal => bounds.width(),
        Axis::Vertical => bounds.height(),
    };
    let (splitter_extent, splitter_thickness) =
        projected_splitter_geometry(node_id, visible_count, extent, metrics)?;
    let mut binary_sizes;
    let sizes = match children {
        [_, _] => {
            binary_sizes = [0.0; 2];
            binary_sizes.as_mut_slice()
        }
        _ => {
            scratch.sizes.clear();
            scratch.sizes.resize(children.len(), 0.0);
            scratch.sizes.as_mut_slice()
        }
    };
    let solved = if visible_count == 0 {
        AxisOutcome {
            overflow: 0.0,
            unallocated: 0.0,
        }
    } else {
        solve_axis_into(
            extent - splitter_extent,
            axis_weights,
            axis_constraints,
            central_index,
            sizes,
            &mut scratch.eligible,
            metrics.pane_overflow_policy,
        )?
    };
    for (child, &extent) in projection.splits.children[child_range.clone()]
        .iter_mut()
        .zip(sizes.iter())
    {
        child.extent = extent;
    }
    projection.splits.set_outcome(
        *split,
        central_index,
        solved.overflow,
        solved.unallocated
            + match axis {
                Axis::Horizontal => slot.width() - bounds.width(),
                Axis::Vertical => slot.height() - bounds.height(),
            },
    );
    let cursor = match axis {
        Axis::Horizontal => bounds.x(),
        Axis::Vertical => bounds.y(),
    };
    Ok(Some(SplitProjectionState {
        row,
        next_child: 0,
        cursor,
        splitter_thickness,
        pending_child: false,
    }))
}

fn projected_splitter_geometry(
    node: NodeId,
    child_count: usize,
    available: f64,
    metrics: LayoutMetrics,
) -> Result<(f64, f64), LayoutError> {
    let configured_extent = total_splitter_extent(node, child_count, metrics)?;
    if configured_extent <= available {
        return Ok((configured_extent, metrics.splitter_thickness()));
    }

    let splitter_count = child_count.saturating_sub(1);
    debug_assert!(splitter_count > 0);
    #[allow(
        clippy::cast_precision_loss,
        reason = "an in-memory child count is exactly representable throughout any feasible workspace"
    )]
    let splitter_count = splitter_count as f64;
    Ok((available, available / splitter_count))
}

fn advance_split_projection(
    topology: &RootStructure,
    mut state: SplitProjectionState,
    stack: &mut Vec<ProjectionFrame>,
    projection: &mut LayoutProjection,
) -> Result<(), LayoutError> {
    let node = &topology.nodes[state.row];
    let RootNodeKind::Split {
        payload,
        children: child_range,
        ..
    } = &node.kind
    else {
        return Err(LayoutError::MissingNode { node: node.id });
    };
    let axis = payload.axis();
    let children = &topology.children[child_range.clone()];
    let bounds = projection.nodes.at_row(state.row).rect;
    if state.pending_child {
        state.pending_child = false;
        // Descendants cannot overwrite this acyclic parent's solved extent.
        let size = projection.splits.children[child_range.start + state.next_child - 1].extent;
        state.cursor += size;
        let previous = child_range.start + state.next_child - 1;
        if projection.splits.children[previous].visible
            && projection.splits.children[child_range.start + state.next_child..child_range.end]
                .iter()
                .any(|child| child.visible)
        {
            let splitter = match axis {
                Axis::Horizontal => LogicalRect::new(
                    state.cursor,
                    bounds.y(),
                    state.splitter_thickness,
                    bounds.height(),
                )?,
                Axis::Vertical => LogicalRect::new(
                    bounds.x(),
                    state.cursor,
                    bounds.width(),
                    state.splitter_thickness,
                )?,
            };
            projection.splits.children[child_range.start + state.next_child - 1].following_gap =
                Some(splitter);
            state.cursor += state.splitter_thickness;
        }
        stack.push(ProjectionFrame::ContinueSplit(state));
        return Ok(());
    }

    if state.next_child == children.len() {
        return Ok(());
    }

    let child =
        children
            .get(state.next_child)
            .copied()
            .ok_or(LayoutError::SplitCardinalityMismatch {
                node: node.id,
                children: children.len(),
                weights: children.len(),
            })?;
    let size = projection.splits.children[child_range.start + state.next_child].extent;
    let child_bounds = match (axis, projection.nodes.at_row(child).visible) {
        (_, false) => LogicalRect::new(bounds.x(), bounds.y(), 0.0, 0.0)?,
        (Axis::Horizontal, true) => {
            LogicalRect::new(state.cursor, bounds.y(), size, bounds.height())?
        }
        (Axis::Vertical, true) => LogicalRect::new(bounds.x(), state.cursor, bounds.width(), size)?,
    };
    state.next_child += 1;
    state.pending_child = true;
    stack.push(ProjectionFrame::ContinueSplit(state));
    stack.push(ProjectionFrame::Enter {
        row: child,
        bounds: child_bounds,
    });
    Ok(())
}

fn validate_inputs(
    extent: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    central_index: Option<usize>,
) -> Result<(), LayoutError> {
    if !extent.is_finite() {
        return Err(LayoutError::NonFiniteExtent { value: extent });
    }
    if extent < 0.0 {
        return Err(LayoutError::NegativeExtent { value: extent });
    }
    if weights.len() != constraints.len() {
        return Err(LayoutError::LengthMismatch {
            weights: weights.len(),
            constraints: constraints.len(),
        });
    }
    if let Some(index) = central_index
        && index >= weights.len()
    {
        return Err(LayoutError::CentralIndexOutOfBounds {
            index,
            child_count: weights.len(),
        });
    }

    for (index, &value) in weights.iter().enumerate() {
        if !value.is_finite() {
            return Err(LayoutError::NonFiniteWeight { index, value });
        }
        if value < 0.0 {
            return Err(LayoutError::NegativeWeight { index, value });
        }
    }

    let weight_total = checked_sum(weights.iter().copied(), "weight")?;
    if central_index.is_none() && !weights.is_empty() && weight_total <= 0.0 {
        return Err(LayoutError::ZeroWeightTotal);
    }

    Ok(())
}

fn desired_sizes_into(
    extent: f64,
    weights: &[f64],
    central_index: Option<usize>,
    sizes: &mut [f64],
) -> Result<(), LayoutError> {
    if let Some(central) = central_index {
        let sibling_total = checked_sum(
            weights
                .iter()
                .enumerate()
                .filter_map(|(index, weight)| (index != central).then_some(*weight)),
            "sibling weight",
        )?;
        let scale = if sibling_total > 1.0 {
            extent / sibling_total
        } else {
            extent
        };
        for (index, (size, weight)) in sizes.iter_mut().zip(weights).enumerate() {
            *size = if index == central {
                0.0
            } else {
                weight * scale
            };
        }
        let siblings = checked_sum(sizes.iter().copied(), "desired size")?;
        sizes[central] = (extent - siblings).max(0.0);
        return Ok(());
    }

    let weight_total = checked_sum(weights.iter().copied(), "weight")?;
    for (size, weight) in sizes.iter_mut().zip(weights) {
        *size = extent * (*weight / weight_total);
    }
    Ok(())
}

fn grow_to_extent(
    sizes: &mut [f64],
    mut remainder: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    central_index: Option<usize>,
    eligible: &mut Vec<usize>,
) {
    if let Some(central) = central_index {
        let amount = remainder.min((constraints[central].max - sizes[central]).max(0.0));
        apply_bounded_delta(&mut sizes[central], amount, true, constraints[central]);
        remainder -= amount;
    }
    redistribute(
        sizes,
        remainder,
        weights,
        constraints,
        true,
        central_index,
        eligible,
    );
}

fn shrink_to_extent(
    sizes: &mut [f64],
    mut excess: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    central_index: Option<usize>,
    eligible: &mut Vec<usize>,
) {
    if let Some(central) = central_index {
        let amount = excess.min((sizes[central] - constraints[central].min).max(0.0));
        apply_bounded_delta(&mut sizes[central], amount, false, constraints[central]);
        excess -= amount;
    }
    redistribute(
        sizes,
        excess,
        weights,
        constraints,
        false,
        central_index,
        eligible,
    );
}

fn redistribute(
    sizes: &mut [f64],
    mut amount: f64,
    weights: &[f64],
    constraints: &[AxisConstraint],
    grow: bool,
    excluded: Option<usize>,
    eligible: &mut Vec<usize>,
) {
    let tolerance = allocation_tolerance(amount, amount, amount);
    while amount > tolerance {
        eligible.clear();
        eligible.extend((0..sizes.len()).filter(|&index| {
            excluded != Some(index)
                && capacity(sizes[index], constraints[index], grow) > tolerance
                && weights[index] > 0.0
        }));
        if eligible.is_empty() {
            break;
        }
        let weight_total: f64 = eligible.iter().map(|&index| weights[index]).sum();
        let before = amount;
        for &index in eligible.iter() {
            let share = before * (weights[index] / weight_total);
            let delta = share.min(capacity(sizes[index], constraints[index], grow));
            apply_bounded_delta(&mut sizes[index], delta, grow, constraints[index]);
            amount -= delta;
        }
        if before - amount <= tolerance {
            break;
        }
    }

    // Consume floating-point residue and zero-weight capacity in stable child order.
    for index in 0..sizes.len() {
        if amount <= tolerance {
            break;
        }
        if excluded == Some(index) {
            continue;
        }
        let delta = amount.min(capacity(sizes[index], constraints[index], grow));
        apply_bounded_delta(&mut sizes[index], delta, grow, constraints[index]);
        amount -= delta;
    }
}

fn capacity(size: f64, constraint: AxisConstraint, grow: bool) -> f64 {
    if grow {
        (constraint.max - size).max(0.0)
    } else {
        (size - constraint.min).max(0.0)
    }
}

fn apply_bounded_delta(size: &mut f64, delta: f64, grow: bool, constraint: AxisConstraint) {
    if grow {
        *size = (*size + delta).min(constraint.max);
    } else {
        *size = (*size - delta).max(constraint.min);
    }
}

fn checked_sum(
    values: impl IntoIterator<Item = f64>,
    quantity: &'static str,
) -> Result<f64, LayoutError> {
    let sum = values.into_iter().sum::<f64>();
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(LayoutError::NonFiniteAggregate { quantity })
    }
}

fn allocation_tolerance(a: f64, b: f64, c: f64) -> f64 {
    a.abs().max(b.abs()).max(c.abs()).max(1.0) * f64::EPSILON * 64.0
}

#[cfg(test)]
mod override_tests;

#[cfg(test)]
mod overflow_tests;

#[cfg(test)]
mod sizing_tests;

#[cfg(test)]
mod visibility_tests;

#[cfg(test)]
mod reference;

#[cfg(test)]
mod topology_tests;

#[cfg(test)]
mod layout_cost;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{RootRecord, SurfacePresentation};
    use crate::ids::{ItemId, SurfaceId};

    #[test]
    fn measured_minimums_preserve_projection_constraints_and_resize_overrides() {
        let metrics = LayoutMetrics::new(3.0).unwrap();
        let root = RootId::new(1);
        for central in [false, true] {
            let mut builder = Workspace::builder();
            let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
            let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
            let c = builder.insert_node(Node::tabs([ItemId::new(3)]));
            let d = builder.insert_node(Node::tabs([ItemId::new(4)]));
            let inner =
                builder.insert_node(Node::split(Axis::Vertical, [b, c], [0.7, 0.3]).unwrap());
            let outer = builder.insert_node(
                Node::split(Axis::Horizontal, [a, inner, d], [0.2, 0.5, 0.3]).unwrap(),
            );
            let record = if central {
                RootRecord::new(outer).with_central(b)
            } else {
                RootRecord::new(outer)
            };
            builder.set_root(root, record);
            builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(root));
            let workspace = builder.build().unwrap();
            let minimums = BTreeMap::from([
                (a, LogicalSize::new(17.0, 11.0).unwrap()),
                (b, LogicalSize::new(31.0, 7.0).unwrap()),
                (c, LogicalSize::new(13.0, 19.0).unwrap()),
                (d, LogicalSize::new(23.0, 5.0).unwrap()),
            ]);
            let root_minimum = LogicalSize::new(77.0, 29.0).unwrap();
            let source = workspace.capture_node_source(root, inner).unwrap();
            let weights = TrackWeights::Split(SplitWeight::normalize([0.4, 0.6]).unwrap());
            let overrides = [TrackWeightOverride::new(&source, &weights)];

            for (width, height) in [(300.0, 200.0), (77.0, 29.0), (41.0, 23.0)] {
                let bounds = LogicalRect::new(5.0, 7.0, width, height).unwrap();
                let maximum = LogicalSize::new(
                    width.max(root_minimum.width()),
                    height.max(root_minimum.height()),
                )
                .unwrap();
                let constraints = minimums
                    .iter()
                    .map(|(node, minimum)| (*node, Constraints::new(*minimum, maximum).unwrap()))
                    .collect::<BTreeMap<_, _>>();
                for overrides in [&[][..], &overrides[..]] {
                    let measured =
                        measure_root_minimums(&workspace, root, &minimums, metrics).unwrap();
                    assert_eq!(measured.minimum(), root_minimum);
                    let actual = measured.project(bounds, overrides).unwrap();
                    let expected = project_root_with_overrides(
                        &workspace,
                        root,
                        bounds,
                        |node| constraints.get(&node).copied(),
                        metrics,
                        overrides,
                    )
                    .unwrap();
                    assert_eq!(actual, expected);
                    assert_eq!(actual.nodes.len(), 6);
                    assert_eq!(actual.nodes[&b].constraints, constraints[&b]);
                    assert_eq!(
                        actual.nodes[&inner].constraints.min(),
                        LogicalSize::new(31.0, 29.0).unwrap(),
                    );
                    assert_eq!(
                        actual.nodes[&inner].constraints.max(),
                        LogicalSize::new(maximum.width(), 2.0 * maximum.height() + 3.0).unwrap(),
                    );
                }
            }
        }
    }
}
