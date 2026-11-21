//! Frozen touching-boundary geometry shared by pointer and semantic resizing.

use std::collections::{BTreeMap, BTreeSet};

use crate::geometry::{LogicalPoint, LogicalRect, LogicalSize};
use crate::graph::command::{NodeSource, TrackResize, TrackWeights};
use crate::graph::{Axis, Node, SplitWeight, Workspace};
use crate::ids::{NodeId, RootId};
use crate::interaction::{
    ActiveResize, FrozenResizeAxisGroup, FrozenResizeSplit, ResizeAdjustment, ResizeDeltaInterval,
};
use crate::presentation::config::PaneOverflowPolicy;
use crate::presentation::layout::{LayoutProjection, SplitProjection, solve_track_extents};
use crate::presentation::scene::{PresentationLayoutFacts, PresentationPlan, SplitterRecord};

mod auto_reflow;
pub(crate) mod piecewise;
use auto_reflow::{InheritedTrackTransfer, finish_auto_reflow};
pub(crate) use piecewise::FrozenPiecewiseAxis;

#[derive(Clone, Copy)]
pub(super) struct ResizeLayout<'a> {
    pub(super) facts: &'a PresentationLayoutFacts,
    pub(super) staged: Option<(
        RootId,
        &'a LayoutProjection,
        &'a BTreeMap<NodeId, TrackWeights>,
    )>,
}

impl<'a> ResizeLayout<'a> {
    pub(super) fn presented(plan: &'a PresentationPlan) -> Option<Self> {
        Some(Self {
            facts: plan.layout_facts()?,
            staged: None,
        })
    }

    pub(super) fn projection(self, root: RootId) -> Option<&'a LayoutProjection> {
        match self.staged {
            Some((id, projection, _)) if id == root => Some(projection),
            _ => Some(self.facts.root(root)?.projection()),
        }
    }

    fn split_weights(
        self,
        root: RootId,
        node: NodeId,
        original: &'a [SplitWeight],
    ) -> &'a [SplitWeight] {
        match self.staged {
            Some((id, _, weights)) if id == root => match weights.get(&node) {
                Some(TrackWeights::Split(weights)) => weights,
                _ => original,
            },
            _ => original,
        }
    }
}

pub(super) fn split_resize_updates(
    resize: &ActiveResize,
    current_pointer: LogicalPoint,
) -> Option<Vec<TrackResize>> {
    resize_axis_group_updates(
        &resize.axis_groups,
        [
            current_pointer.x() - resize.initial_pointer.x(),
            current_pointer.y() - resize.initial_pointer.y(),
        ],
    )
}

/// Freezes the complete touching frontier from the exact presented projection.
/// Missing layout facts never fall back to clipped handle geometry.
pub(super) fn prepare_resize_axis_groups(
    workspace: &Workspace,
    plan: &PresentationPlan,
    handles: Vec<(NodeSource, SplitterRecord)>,
) -> Option<Vec<FrozenResizeAxisGroup>> {
    prepare_projected_resize_axis_groups(workspace, ResizeLayout::presented(plan)?, &handles)
}

pub(super) fn prepare_projected_resize_axis_groups(
    workspace: &Workspace,
    layout: ResizeLayout<'_>,
    handles: &[(NodeSource, SplitterRecord)],
) -> Option<Vec<FrozenResizeAxisGroup>> {
    if layout.facts.config().resize_propagation()
        == crate::presentation::config::ResizePropagation::Adjacent
        && let Some(groups) = prepare_linear_resize_axis_groups(workspace, layout, handles)
    {
        return Some(groups);
    }
    piecewise::prepare(workspace, layout, handles)
}

fn prepare_linear_resize_axis_groups(
    workspace: &Workspace,
    layout: ResizeLayout<'_>,
    handles: &[(NodeSource, SplitterRecord)],
) -> Option<Vec<FrozenResizeAxisGroup>> {
    if handles.is_empty() {
        return None;
    }
    let facts = layout.facts;
    let mut roots = BTreeMap::<RootId, NodeSource>::new();
    // Include both outer slots, and merge every incident gap before traversal.
    // No handle independently solves a child shared with another incident gap.
    let mut incidents = BTreeMap::<Axis, BTreeMap<(RootId, NodeId), Vec<Option<f64>>>>::new();
    for (source, record) in handles {
        let id = *record.id();
        if source.root() != id.root || source.node() != id.node() || !record.operable() {
            return None;
        }
        match roots.entry(id.root) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                workspace
                    .verify_reference(
                        id.root,
                        id.node(),
                        source.reference(),
                        crate::graph::error::ReferenceRole::Source,
                    )
                    .ok()?;
                entry.insert(source.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get().reference() != source.reference() {
                    return None;
                }
            }
        }
        let projection = layout.projection(id.root)?;
        // Reprojection can change constraints in an Auto branch even when
        // neither of its edges moves in the frozen linear frontier.
        if projection.auto_reflow().is_some() {
            return None;
        }
        let Node::Split {
            axis,
            children,
            weights,
            ..
        } = workspace.node(id.node())?
        else {
            return None;
        };
        let weights = layout.split_weights(id.root, id.node(), weights);
        let split = projection.splits.get(&id.node())?;
        if *axis != record.axis()
            || split.axis != *axis
            || id.index >= children.len().checked_sub(1)?
            || split.following_child_index(id.index) != Some(record.after_index())
            || !record
                .child_extents()
                .iter()
                .copied()
                .eq(split.children.iter().map(|child| child.extent))
            || record.central_index() != split.central_index
            || record.weights() != weights
            || record.before_minimum_extent() != split.children.get(id.index)?.minimum
            || record.after_minimum_extent() != split.children.get(record.after_index())?.minimum
            || record.before_maximum_extent() != split.children.get(id.index)?.maximum
            || record.after_maximum_extent() != split.children.get(record.after_index())?.maximum
        {
            return None;
        }
        let edges = incidents
            .entry(*axis)
            .or_default()
            .entry((id.root, id.node()))
            .or_insert_with(|| vec![None; children.len() + 1]);
        for index in id.index + 1..=record.after_index() {
            merge_edge(edges.get_mut(index)?, 1.0)?;
        }
    }

    incidents
        .into_iter()
        .map(|(axis, incidents)| {
            let mut group = FrozenResizeAxisGroup {
                axis,
                overflow_policy: facts.config().pane_overflow_policy(),
                splits: Vec::new(),
                piecewise: None,
                common_delta: ResizeDeltaInterval {
                    minimum: f64::NEG_INFINITY,
                    maximum: f64::INFINITY,
                },
            };
            let incident_roots = incidents
                .keys()
                .map(|(root, _)| *root)
                .collect::<BTreeSet<_>>();
            let mut found_incidents = 0;
            for root in incident_roots {
                let prototype = roots.get(&root)?;
                let root_facts = facts.root(root)?;
                let projection = layout.projection(root)?;
                let root_node = workspace.root(root)?.node;
                if projection.nodes.get(&root_node).map(|result| result.slot)
                    != Some(root_facts.content_bounds())
                {
                    return None;
                }
                let mut projected_nodes = projection.nodes.iter_preorder();
                let mut pending = vec![(root_node, [0.0, 0.0], false, false)];
                let mut visited = BTreeSet::new();
                while let Some((node, outer_edges, check_extent, inherits_slot)) = pending.pop() {
                    if !visited.insert(node) {
                        return None;
                    }
                    let (&projected_node, result) = projected_nodes.next()?;
                    if projected_node != node {
                        return None;
                    }
                    if !result.visible {
                        if incidents.contains_key(&(root, node)) {
                            return None;
                        }
                        pending.extend(
                            workspace
                                .node(node)?
                                .child_nodes()
                                .rev()
                                .map(|(_, child)| (child, [0.0, 0.0], false, false)),
                        );
                        continue;
                    }
                    let live = workspace.node(node)?;
                    let rect = result.rect;
                    let incident = incidents.get(&(root, node));
                    let affected = outer_edges != [0.0, 0.0] || incident.is_some();
                    if affected && matches!(live, Node::Grid { .. }) {
                        return None;
                    }
                    if affected || check_extent {
                        // A frame can stop following the changing slot when it
                        // reaches its cap. The piecewise solver handles that
                        // contact without stopping other children in the slot.
                        let constraints = result.constraints;
                        if outer_edges[1] != outer_edges[0]
                            && ((rect_extent(result.slot, axis) - rect_extent(rect, axis)).abs()
                                > rect_extent_tolerance(result.slot, axis)
                                || (inherits_slot
                                    && projection.caps_frames()
                                    && size_extent(constraints.max(), axis)
                                        < rect_extent(root_facts.content_bounds(), axis)))
                        {
                            return None;
                        }
                        intersect_extent_interval(
                            &mut group.common_delta,
                            rect_extent(rect, axis),
                            size_extent(constraints.min(), axis),
                            size_extent(constraints.max(), axis),
                            outer_edges[1] - outer_edges[0],
                            rect_extent_tolerance(rect, axis),
                        )?;
                    }
                    let Node::Split {
                        axis: split_axis,
                        children,
                        weights,
                        ..
                    } = live
                    else {
                        if incident.is_some() {
                            return None;
                        }
                        pending.extend(live.child_nodes().rev().map(|(_, child)| {
                            let visible = projection
                                .nodes
                                .get(&child)
                                .is_some_and(|result| result.visible);
                            (
                                child,
                                if visible { outer_edges } else { [0.0, 0.0] },
                                affected && visible,
                                true,
                            )
                        }));
                        continue;
                    };
                    let weights = layout.split_weights(root, node, weights);
                    let mut edges = vec![0.0; children.len() + 1];
                    let split = projection.splits.get(&node)?;
                    if *split_axis == axis {
                        let first = split.visible_child_indices().next()?;
                        let last = split.visible_child_indices().next_back()?;
                        edges[..=first].fill(outer_edges[0]);
                        edges[last + 1..].fill(outer_edges[1]);
                        if let Some(incident) = incident {
                            if incident.len() != edges.len() {
                                return None;
                            }
                            for (edge, movement) in edges.iter_mut().zip(incident) {
                                if let Some(movement) = movement {
                                    *edge = *movement;
                                }
                            }
                            found_incidents += 1;
                        }
                    } else if incident.is_some() {
                        return None;
                    }
                    if affected {
                        let bounds = rect;
                        validate_split_projection(
                            projection,
                            &split,
                            *split_axis,
                            bounds,
                            children,
                            weights,
                            facts.config().splitter_thickness(),
                            *split_axis == axis,
                        )?;
                        let coefficients = if *split_axis == axis {
                            edges
                                .windows(2)
                                .map(|pair| pair[1] - pair[0])
                                .collect::<Vec<_>>()
                        } else {
                            vec![0.0; children.len()]
                        };
                        if *split_axis == axis {
                            intersect_split_intervals(
                                &mut group.common_delta,
                                &split,
                                weights,
                                &coefficients,
                            )?;
                        }
                        group.splits.push(FrozenResizeSplit {
                            // Traversal proves membership; share the exact root snapshot.
                            source: NodeSource {
                                root,
                                node,
                                reference: prototype.reference().clone(),
                            },
                            axis: *split_axis,
                            has_driver: incident.is_some(),
                            bounds,
                            constraints: result.constraints,
                            weights: weights.to_vec(),
                            child_extents: split
                                .children
                                .iter()
                                .map(|child| child.extent)
                                .collect(),
                            child_visible: split
                                .children
                                .iter()
                                .map(|child| child.visible)
                                .collect(),
                            child_minimum_extents: split
                                .children
                                .iter()
                                .map(|child| child.minimum)
                                .collect(),
                            child_maximum_extents: split
                                .children
                                .iter()
                                .map(|child| child.maximum)
                                .collect(),
                            child_coefficients: coefficients,
                            central_index: split.central_index,
                        });
                    }
                    for (index, child) in children.iter().copied().enumerate().rev() {
                        let child_edges = if !split.children.get(index)?.visible {
                            [0.0, 0.0]
                        } else if *split_axis == axis {
                            [*edges.get(index)?, *edges.get(index + 1)?]
                        } else {
                            outer_edges
                        };
                        pending.push((
                            child,
                            child_edges,
                            affected && split.children[index].visible,
                            *split_axis != axis,
                        ));
                    }
                }
                if projected_nodes.next().is_some() {
                    return None;
                }
            }
            if found_incidents != incidents.len()
                || !group.common_delta.minimum.is_finite()
                || !group.common_delta.maximum.is_finite()
                || group.common_delta.minimum > 0.0
                || group.common_delta.maximum < 0.0
            {
                return None;
            }
            Some(group)
        })
        .collect()
}

/// Produces one deduplicated batch. Orthogonal frontiers constrain motion and
/// continuation; only the group matching a split's own axis emits its weights.
pub(super) fn resize_axis_group_updates(
    groups: &[FrozenResizeAxisGroup],
    delta: [f64; 2],
) -> Option<Vec<TrackResize>> {
    if groups.is_empty() || delta.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mut axes = BTreeSet::new();
    let mut updates = BTreeMap::new();
    let mut reflow = Vec::new();
    let mut inherited = Vec::new();
    for group in groups {
        if !axes.insert(group.axis) {
            return None;
        }
        let requested = match group.axis {
            Axis::Horizontal => delta[0],
            Axis::Vertical => delta[1],
        };
        let shared = requested.clamp(group.common_delta.minimum, group.common_delta.maximum);
        if let Some(piecewise) = &group.piecewise {
            let (delta, sample) = piecewise.track_sample(shared)?;
            for (key, update) in sample.updates {
                merge_axis_update(&mut updates, key, group.axis, update)?;
            }
            inherited.extend(
                sample
                    .tracks
                    .into_iter()
                    .filter_map(|((root, node), expected)| {
                        let (tracks, overflow_policy) = piecewise.inherited_tracks(root, node)?;
                        Some(InheritedTrackTransfer {
                            root,
                            node,
                            axis: group.axis,
                            expected,
                            tracks,
                            overflow_policy,
                        })
                    }),
            );
            reflow.push((piecewise.as_ref(), delta));
            continue;
        }
        for split in group.splits.iter().filter(|split| split.axis == group.axis) {
            let update = split_update(split, shared, group.overflow_policy)?;
            if updates
                .insert((split.source.root(), split.source.node()), update)
                .is_some()
            {
                return None;
            }
        }
    }
    finish_auto_reflow(&reflow, &inherited, &mut updates)?;
    Some(updates.into_values().collect())
}

fn merge_axis_update<Key: Ord>(
    updates: &mut BTreeMap<Key, TrackResize>,
    key: Key,
    axis: Axis,
    mut update: TrackResize,
) -> Option<()> {
    if let Some(previous) = updates.get(&key) {
        // A junction can change both axes of one Grid in one graph update.
        let basis = update.grid_basis()?;
        if previous.grid_basis()? != basis {
            return None;
        }
        let previous = previous.grid_shares()?;
        let shares = update.grid_shares()?;
        let (rows, columns) = match axis {
            Axis::Horizontal => (previous.row_weights(), shares.column_weights()),
            Axis::Vertical => (shares.row_weights(), previous.column_weights()),
        };
        update = TrackResize::grid_resolved(
            update.source().clone(),
            basis.clone(),
            crate::model::DockspaceGridShares::from_normalized(rows.to_vec(), columns.to_vec())
                .ok()?,
        );
    }
    updates.insert(key, update);
    Some(())
}

pub(super) fn resize_axis_group_adjustment_updates(
    groups: &[FrozenResizeAxisGroup],
    axis: Axis,
    adjustment: ResizeAdjustment,
) -> Option<Vec<TrackResize>> {
    let delta = match adjustment {
        ResizeAdjustment::Displace(delta) => delta,
        ResizeAdjustment::Tracks(tracks) => return Some(tracks.to_vec()),
    };
    resize_axis_group_updates(
        groups,
        match axis {
            Axis::Horizontal => [delta, 0.0],
            Axis::Vertical => [0.0, delta],
        },
    )
}

fn merge_edge(edge: &mut Option<f64>, movement: f64) -> Option<()> {
    if edge.is_some_and(|previous| previous != movement) {
        return None;
    }
    *edge = Some(movement);
    Some(())
}

fn rect_start(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => rect.x(),
        Axis::Vertical => rect.y(),
    }
}

fn rect_extent(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => rect.width(),
        Axis::Vertical => rect.height(),
    }
}

fn size_extent(size: LogicalSize, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => size.width(),
        Axis::Vertical => size.height(),
    }
}

fn tolerance(left: f64, right: f64) -> f64 {
    left.abs().max(right.abs()).max(1.0) * f64::EPSILON * 64.0
}

fn rect_extent_tolerance(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => tolerance(rect.min().x(), rect.max().x()),
        Axis::Vertical => tolerance(rect.min().y(), rect.max().y()),
    }
}

fn intersect_extent_interval(
    interval: &mut ResizeDeltaInterval,
    extent: f64,
    minimum: f64,
    maximum: f64,
    coefficient: f64,
    coordinate_tolerance: f64,
) -> Option<()> {
    if !extent.is_finite()
        || !minimum.is_finite()
        || !maximum.is_finite()
        || minimum < 0.0
        || minimum > maximum
    {
        return None;
    }
    let lower_gap = minimum - extent;
    let upper_gap = maximum - extent;
    let lower_tolerance = tolerance(extent, minimum).max(coordinate_tolerance);
    let upper_tolerance = tolerance(extent, maximum).max(coordinate_tolerance);
    if lower_gap > lower_tolerance || upper_gap < -upper_tolerance {
        return None;
    }
    if coefficient == 0.0 {
        return Some(());
    }
    // Near a constraint, the linear rectangle alone cannot distinguish a
    // rounded contact from real remaining travel. Let the piecewise solver
    // inspect the allocation before deciding how much movement is available.
    if (lower_gap < 0.0 && lower_gap >= -lower_tolerance)
        || (upper_gap > 0.0 && upper_gap <= upper_tolerance)
    {
        return None;
    }
    let lower = lower_gap.min(0.0) / coefficient;
    let upper = upper_gap.max(0.0) / coefficient;
    interval.minimum = interval.minimum.max(lower.min(upper));
    interval.maximum = interval.maximum.min(lower.max(upper));
    (interval.minimum <= interval.maximum).then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_split_projection(
    projection: &LayoutProjection,
    split: &SplitProjection<'_>,
    axis: Axis,
    bounds: LogicalRect,
    children: &[NodeId],
    weights: &[SplitWeight],
    thickness: f64,
    require_full_slot: bool,
) -> Option<()> {
    let count = children.len();
    if count < 2
        || split.axis != axis
        || count != weights.len()
        || count != split.children.len()
        || split.children.iter().enumerate().any(|(index, child)| {
            child.following_gap.is_some()
                != (child.visible
                    && split.children[index + 1..]
                        .iter()
                        .any(|child| child.visible))
        })
        || split.central_index.is_some_and(|central| central >= count)
    {
        return None;
    }
    let visible_count = split.visible_child_indices().count();
    let gaps = std::iter::repeat_n(thickness, visible_count.saturating_sub(1)).sum::<f64>();
    let available = split.children.iter().map(|child| child.extent).sum::<f64>();
    let extent = rect_extent(bounds, axis);
    // Child allocation and rectangle max-minus-min may differ by a few ULPs.
    // Judge residuals at the bounds' coordinate scale, without relaxing gaps.
    let extent_tolerance =
        tolerance(available + gaps, extent).max(rect_extent_tolerance(bounds, axis));
    if !available.is_finite()
        || available <= 0.0
        || !gaps.is_finite()
        || gaps > extent
        || !split.overflow.is_finite()
        || split.overflow < 0.0
        || split.overflow > extent_tolerance
        || !split.unallocated.is_finite()
        || split.unallocated < 0.0
        || (require_full_slot && split.unallocated > extent_tolerance)
        || (available + gaps - extent).abs() > extent_tolerance
        || split.splitter_rects().any(|gap| {
            (rect_extent(gap, axis) - thickness).abs()
                > rect_extent_tolerance(gap, axis).max(tolerance(thickness, thickness))
        })
    {
        return None;
    }
    let mut fixed = ResizeDeltaInterval {
        minimum: 0.0,
        maximum: 0.0,
    };
    for (index, child) in children.iter().enumerate() {
        let actual = split.children[index].extent;
        let child_bounds = projection.nodes.get(child)?.rect;
        let projected = rect_extent(child_bounds, axis);
        if !split.children[index].visible {
            if actual != 0.0
                || projected != 0.0
                || split.children[index].minimum != 0.0
                || split.children[index].maximum != 0.0
            {
                return None;
            }
            continue;
        }
        if actual <= 0.0
            || (actual - projected).abs()
                > tolerance(actual, projected).max(rect_extent_tolerance(child_bounds, axis))
        {
            return None;
        }
        intersect_extent_interval(
            &mut fixed,
            actual,
            split.children[index].minimum,
            split.children[index].maximum,
            0.0,
            0.0,
        )?;
    }
    Some(())
}

fn intersect_split_intervals(
    interval: &mut ResizeDeltaInterval,
    split: &SplitProjection<'_>,
    weights: &[SplitWeight],
    coefficients: &[f64],
) -> Option<()> {
    let total = split.children.iter().map(|child| child.extent).sum::<f64>();
    let total_coefficient = coefficients.iter().sum::<f64>();
    intersect_extent_interval(
        interval,
        total,
        total * f64::EPSILON,
        f64::MAX,
        total_coefficient,
        0.0,
    )?;
    let visible_weight_total = if split.children.iter().all(|child| child.visible) {
        1.0
    } else {
        crate::graph::normalized_weight_sum(
            weights
                .iter()
                .zip(split.children)
                .filter_map(|(weight, child)| child.visible.then_some(weight.get())),
        )
    };
    // Hidden siblings may reserve nearly all durable weight. A positive visual
    // fraction must still map to a representable positive durable weight.
    let minimum_share = f64::EPSILON.max(f64::from_bits(1) / visible_weight_total);
    for (child, &coefficient) in split.children.iter().zip(coefficients) {
        if !child.visible {
            if coefficient != 0.0 {
                return None;
            }
            continue;
        }
        let extent = child.extent;
        intersect_extent_interval(
            interval,
            extent,
            child.minimum,
            child.maximum,
            coefficient,
            0.0,
        )?;
        // Keep normalized f64 shares positive as the total changes. A smaller
        // existing share remains valid at zero displacement.
        let share_floor = (extent / total).min(minimum_share);
        let margin = (extent - share_floor * total).max(0.0);
        let slope = coefficient - share_floor * total_coefficient;
        if share_floor <= 0.0 {
            return None;
        }
        if slope > 0.0 {
            interval.minimum = interval.minimum.max(-margin / slope);
        } else if slope < 0.0 {
            interval.maximum = interval.maximum.min(-margin / slope);
        }
    }
    (interval.minimum <= interval.maximum).then_some(())
}

fn split_update(
    split: &FrozenResizeSplit,
    delta: f64,
    overflow_policy: PaneOverflowPolicy,
) -> Option<TrackResize> {
    let extents = split
        .child_extents
        .iter()
        .zip(&split.child_coefficients)
        .map(|(&extent, &coefficient)| extent + coefficient * delta)
        .collect::<Vec<_>>();
    if extents == split.child_extents {
        return Some(TrackResize::split(
            split.source.clone(),
            split.weights.clone(),
        ));
    }
    let mut fixed = ResizeDeltaInterval {
        minimum: 0.0,
        maximum: 0.0,
    };
    intersect_extent_interval(
        &mut fixed,
        rect_extent(split.bounds, split.axis)
            + split.child_coefficients.iter().sum::<f64>() * delta,
        size_extent(split.constraints.min(), split.axis),
        size_extent(split.constraints.max(), split.axis),
        0.0,
        rect_extent_tolerance(split.bounds, split.axis),
    )?;
    for (index, &extent) in extents.iter().enumerate() {
        intersect_extent_interval(
            &mut fixed,
            extent,
            split.child_minimum_extents[index],
            split.child_maximum_extents[index],
            0.0,
            0.0,
        )?;
    }
    let mut weights = weights_for_extents(
        &split.weights,
        &split.child_extents,
        &extents,
        split.central_index,
    )?;
    if !split.has_driver
        && weights != split.weights
        && stored_shares_match_extents(
            &split.weights,
            &extents,
            split
                .child_minimum_extents
                .iter()
                .copied()
                .zip(&split.child_maximum_extents)
                .zip(&split.child_visible)
                .map(|((minimum, &maximum), &visible)| (minimum, maximum, visible)),
            split.central_index,
            overflow_policy,
        )
    {
        weights.clone_from(&split.weights);
    }
    Some(TrackResize::split(split.source.clone(), weights))
}

fn stored_shares_match_extents(
    weights: &[SplitWeight],
    extents: &[f64],
    constraints: impl Iterator<Item = (f64, f64, bool)> + Clone,
    central_index: Option<usize>,
    overflow_policy: PaneOverflowPolicy,
) -> bool {
    solve_track_extents(
        extents.iter().sum(),
        weights,
        constraints,
        central_index,
        overflow_policy,
    )
    .is_ok_and(|solved| {
        solved.len() == extents.len()
            && solved.iter().zip(extents).all(|(&actual, &expected)| {
                (actual - expected).abs() <= tolerance(actual, expected)
            })
    })
}

fn weights_for_extents(
    weights: &[SplitWeight],
    old_extents: &[f64],
    extents: &[f64],
    central: Option<usize>,
) -> Option<Vec<SplitWeight>> {
    let total = extents.iter().sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    let visible_share = if old_extents.iter().all(|&extent| extent > 0.0) {
        1.0
    } else {
        crate::graph::normalized_weight_sum(
            weights
                .iter()
                .zip(old_extents)
                .filter_map(|(weight, &extent)| (extent > 0.0).then_some(weight.get())),
        )
    };
    let mut shares = extents
        .iter()
        .zip(old_extents)
        .zip(weights)
        .map(|((&extent, &old_extent), weight)| {
            if old_extent > 0.0 {
                extent / total * visible_share
            } else {
                weight.get()
            }
        })
        .collect::<Vec<_>>();
    if let Some(central) = central {
        let mut siblings = 0.0;
        for (index, share) in shares.iter_mut().enumerate() {
            if index != central && old_extents[index] > 0.0 {
                siblings += *share;
            }
        }
        let central_share = shares.get_mut(central)?;
        let residual = visible_share - siblings;
        // Rounded siblings can exhaust the residual. Central layout ignores its
        // own weight, so retain the raw positive share in that case.
        if residual > 0.0 {
            *central_share = residual;
        }
    }
    shares.into_iter().map(weight).collect::<Option<Vec<_>>>()
}

fn weight(value: f64) -> Option<SplitWeight> {
    SplitWeight::new(value).ok()
}

#[cfg(test)]
mod tests;
