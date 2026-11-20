//! Absolute splitter restoration sharing the ordered adjustment calculation.

use std::collections::BTreeMap;

use crate::geometry::{AxisSizing, LogicalRect, PaneSizing};
use crate::graph::command::{TrackResize, TrackWeights};
use crate::graph::{Axis, Node};
use crate::ids::{NodeId, RootId};
use crate::interaction::{FrozenResizeAxisGroup, ResizeAdjustment, ResizeDeltaInterval};
use crate::policy::TabBarVisibility;
use crate::presentation::compiler::{pane_outer_sizing, reprojected_resize_handles};
use crate::presentation::manifest::PaneSizingKey;
use crate::presentation::scene::{
    SplitterOwner, SplitterRecord, SplitterResizeTarget, SplitterSceneId,
};
use crate::runtime::SurfaceSplitterAdjustment;

use super::splitter_geometry::{ResizeLayout, prepare_projected_resize_axis_groups};
use super::*;

impl DockEngine {
    pub(crate) fn prepare_splitter_adjustments(
        &self,
        plan: &PresentationPlan,
        target: SplitterResizeTarget,
        axis: Axis,
        step_size: f64,
        adjustments: impl IntoIterator<Item = SurfaceSplitterAdjustment>,
    ) -> Option<ResizeAdjustment> {
        let mut steps = adjustments.into_iter().peekable();
        let first = steps.next()?;
        let root = match target {
            SplitterResizeTarget::Handle(id) => id.root,
            SplitterResizeTarget::Junction(id) => id.root(),
        };
        let original = ResizeLayout::presented(plan)?;
        let root_projection = original.projection(root)?;
        let auto = root_projection.auto_reflow().is_some();
        let policy = self.policy_snapshot();
        if steps.peek().is_none() {
            if !auto {
                match first {
                    SurfaceSplitterAdjustment::Decrement => {
                        return Some(ResizeAdjustment::new(-step_size));
                    }
                    SurfaceSplitterAdjustment::Increment => {
                        return Some(ResizeAdjustment::new(step_size));
                    }
                    _ => {}
                }
            }
            let group = Self::prepare_splitter_resize_axis_group(
                &self.workspace,
                plan,
                target,
                axis,
                policy,
            )?;
            let delta = Self::splitter_step_delta(
                &self.workspace,
                original,
                |id| plan.splitter_record(id),
                target,
                &group,
                step_size,
                first,
            )?;
            let adjustment = ResizeAdjustment::new(delta);
            if auto {
                let updates = resize_axis_group_adjustment_updates(
                    std::slice::from_ref(&group),
                    axis,
                    adjustment.clone(),
                )?;
                crate::graph::operation::validate_track_resizes(&self.workspace, policy, &updates)
                    .ok()?;
            }
            return Some(adjustment);
        }

        // A key begins at the previous key's allocation. In Outward mode a
        // reversal can redistribute space without moving the final divider.
        // Keep intermediates private and publish their final weights once.
        let mut group =
            Self::prepare_splitter_resize_axis_group(&self.workspace, plan, target, axis, policy)?;
        let mut projected = None;
        let mut handles: Option<Vec<SplitterRecord>> = None;
        let mut weights = BTreeMap::<NodeId, TrackWeights>::new();
        let mut final_tracks = BTreeMap::<NodeId, TrackResize>::new();
        let mut steps = std::iter::once(first).chain(steps).peekable();
        while let Some(step) = steps.next() {
            let layout = ResizeLayout {
                staged: projected
                    .as_ref()
                    .map(|projection| (root, projection, &weights)),
                ..original
            };
            let record = |id| match &handles {
                Some(records) => records.iter().find(|record| *record.id() == id),
                None => plan.splitter_record(id),
            };
            let delta = Self::splitter_step_delta(
                &self.workspace,
                layout,
                record,
                target,
                &group,
                step_size,
                step,
            );
            let Some(delta) = delta else {
                if auto {
                    continue;
                }
                return None;
            };
            let updates = resize_axis_group_adjustment_updates(
                std::slice::from_ref(&group),
                axis,
                ResizeAdjustment::new(delta),
            );
            let Some(updates) = updates else {
                if auto {
                    continue;
                }
                return None;
            };
            let changed =
                crate::graph::operation::validate_track_resizes(&self.workspace, policy, &updates)
                    .ok()?;
            let mut advanced = false;
            for (update, (_, changed)) in updates.into_iter().zip(changed) {
                let node = update.source().node();
                if final_tracks.get(&node).map_or(changed, |previous| {
                    previous.weights() != update.weights()
                        || previous.grid_basis() != update.grid_basis()
                }) {
                    weights.insert(node, update.weights().clone());
                    final_tracks.insert(node, update.materialize_grid_basis(axis));
                    advanced = true;
                }
            }
            if !advanced || steps.peek().is_none() {
                continue;
            }
            let projection = root_projection.project_resize_step(&weights).ok()?;
            let Some(records) = reprojected_resize_handles(
                &self.workspace,
                plan,
                target,
                &projection,
                &weights,
                policy,
            )
            .ok()?
            else {
                // The accepted prefix can consume its own junction (for example,
                // two tees become one cross). Later keys have no such receiver.
                break;
            };
            let active = records
                .iter()
                .filter(|record| record.axis() == axis)
                .map(|record| {
                    let id = *record.id();
                    self.workspace
                        .capture_node_source(id.root, id.node())
                        .map(|source| (source, record.clone()))
                })
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let layout = ResizeLayout {
                staged: Some((root, &projection, &weights)),
                ..original
            };
            if active.is_empty() || active.iter().any(|(_, record)| !record.operable()) {
                break;
            }
            let Some(groups) =
                prepare_projected_resize_axis_groups(&self.workspace, layout, &active)
            else {
                // The current grid may have no feasible continuation after a
                // reflow (for example, the new incident track is fixed).
                break;
            };
            Self::authorize_resize_axis_groups(&self.workspace, policy, &groups).ok()?;
            group = groups.into_iter().find(|group| group.axis == axis)?;
            projected = Some(projection);
            handles = Some(records);
        }
        if final_tracks.is_empty() {
            return Some(ResizeAdjustment::new(0.0));
        }
        Some(ResizeAdjustment::Tracks(
            final_tracks.into_values().collect(),
        ))
    }

    fn splitter_step_delta<'a>(
        workspace: &Workspace,
        layout: ResizeLayout<'_>,
        record: impl Fn(SplitterSceneId) -> Option<&'a SplitterRecord>,
        target: SplitterResizeTarget,
        group: &FrozenResizeAxisGroup,
        step_size: f64,
        step: SurfaceSplitterAdjustment,
    ) -> Option<f64> {
        use SurfaceSplitterAdjustment as Step;
        match step {
            Step::Decrement => Some(-step_size),
            Step::Increment => Some(step_size),
            Step::Minimum => Some(group.common_delta.minimum),
            Step::Maximum => Some(group.common_delta.maximum),
            Step::Equalize | Step::Preferred => Self::splitter_restoration_in_layout(
                workspace,
                layout,
                record,
                target,
                group,
                if step == Step::Equalize {
                    SplitterReset::Equalize
                } else {
                    SplitterReset::Preferred
                },
            ),
        }
    }

    /// Proves advertised Auto range endpoints with the shared resize evaluator.
    pub(crate) fn checked_splitter_resize_range(
        &self,
        plan: &PresentationPlan,
        splitter: SplitterSceneId,
    ) -> Option<ResizeDeltaInterval> {
        let record = plan.splitter_record(splitter)?;
        let axis = record.axis();
        let group = Self::prepare_splitter_resize_axis_group(
            &self.workspace,
            plan,
            SplitterResizeTarget::Handle(splitter),
            axis,
            self.policy_snapshot(),
        )?;
        let limits = group.common_delta;
        if plan
            .layout_facts()?
            .root(splitter.root)?
            .projection()
            .auto_reflow()
            .is_some()
        {
            for endpoint in [limits.minimum, limits.maximum] {
                let delta = match axis {
                    Axis::Horizontal => [endpoint, 0.0],
                    Axis::Vertical => [0.0, endpoint],
                };
                let updates = crate::engine::splitter_geometry::resize_axis_group_updates(
                    std::slice::from_ref(&group),
                    delta,
                )?;
                crate::graph::operation::validate_track_resizes(
                    self.workspace(),
                    self.policy_snapshot(),
                    &updates,
                )
                .ok()?;
            }
        }
        Some(limits)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn reset_splitter_from_candidate(
        &mut self,
        input: InputSequence,
        witness: &crate::presentation::scene::SurfacePlanScene,
        splitter: SplitterSceneId,
        reset: SplitterReset,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        let Some(plan) = self.recompile_splitter_plan(&self.workspace, witness.plan(), policy)
        else {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::SplitterResizeGeometryUnavailable,
            ));
        };
        let target = SplitterResizeTarget::Handle(splitter);
        if witness
            .plan()
            .splitter_record(splitter)
            .and_then(SplitterRecord::grid)
            != plan
                .splitter_record(splitter)
                .and_then(SplitterRecord::grid)
        {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::StaleScene,
            ));
        }
        let adjustment = plan.splitter_record(splitter).and_then(|record| {
            (record.operable() && plan.region_is_operable(record.hit().rect(), record.layer()))
                .then_some(())?;
            let axis = record.axis();
            let group = Self::prepare_splitter_resize_axis_group(
                &self.workspace,
                &plan,
                target,
                axis,
                policy,
            )?;
            Self::splitter_restoration_delta(&self.workspace, &plan, target, &group, reset)
                .map(ResizeAdjustment::new)
        });
        let Some(adjustment) = adjustment else {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::SplitterResizeGeometryUnavailable,
            ));
        };
        self.adjust_splitter_from_plan(
            input,
            &plan,
            splitter,
            adjustment,
            policy,
            events,
            interaction_events,
        )
    }

    fn recompile_splitter_plan(
        &self,
        workspace: &Workspace,
        basis: &PresentationPlan,
        policy: &DockPolicySnapshot,
    ) -> Option<PresentationPlan> {
        let requirements = &self.presentation_authority.presentation_requirements;
        let ticket = requirements.surface(basis.surface())?.ticket();
        let measurements = basis.layout_facts()?.measurements().reticket(ticket);
        compile_surface_plan(
            workspace,
            self.version,
            policy,
            &self.presentation_authority.presentation_config,
            requirements,
            &measurements,
            &self.presentation_authority.tab_strip_states,
            &[],
            self.contained_layout_projection(basis.surface()),
        )
        .ok()?
        .finalize()
        .ok()
    }

    fn splitter_restoration_delta(
        workspace: &Workspace,
        plan: &PresentationPlan,
        target: SplitterResizeTarget,
        group: &FrozenResizeAxisGroup,
        adjustment: SplitterReset,
    ) -> Option<f64> {
        Self::splitter_restoration_in_layout(
            workspace,
            ResizeLayout::presented(plan)?,
            |id| plan.splitter_record(id),
            target,
            group,
            adjustment,
        )
    }

    fn splitter_restoration_in_layout<'a>(
        workspace: &Workspace,
        layout: ResizeLayout<'_>,
        record: impl Fn(SplitterSceneId) -> Option<&'a SplitterRecord>,
        target: SplitterResizeTarget,
        group: &FrozenResizeAxisGroup,
        adjustment: SplitterReset,
    ) -> Option<f64> {
        let limits = group.common_delta;
        let axis = group.axis;
        let restore = |id: SplitterSceneId| {
            let record = record(id)?;
            let before = *record.child_extents().get(id.index)?;
            let after = *record.child_extents().get(record.after_index())?;
            let (before_hint, after_hint) = match (adjustment, id.owner) {
                (SplitterReset::Equalize, _) => (None, None),
                (SplitterReset::Preferred, SplitterOwner::Split(node)) => {
                    let Node::Split { children, .. } = workspace.node(node)? else {
                        return None;
                    };
                    let hint = |index| {
                        Self::subtree_axis_sizing(
                            workspace,
                            layout,
                            id.root,
                            *children.get(index)?,
                            axis,
                        )
                        .map(AxisSizing::preferred)
                    };
                    (hint(id.index)?, hint(record.after_index())?)
                }
                (
                    SplitterReset::Preferred,
                    SplitterOwner::Grid {
                        node,
                        axis: owner_axis,
                    },
                ) => {
                    if axis != owner_axis {
                        return None;
                    }
                    let hint = |track| {
                        Self::grid_track_preferred(workspace, layout, id.root, node, axis, track)
                    };
                    (hint(id.index)?, hint(record.after_index())?)
                }
            };
            let delta = if let Some(frozen) = &group.piecewise {
                piecewise_target_delta(limits, before_hint, after_hint, |delta| {
                    let tracks = frozen.track_extents(id.root, id.node(), delta)?;
                    Some((*tracks.get(id.index)?, *tracks.get(record.after_index())?))
                })?
            } else {
                match (before_hint, after_hint) {
                    (Some(first), Some(second)) if first > 0.0 || second > 0.0 => {
                        let scale = first.max(second);
                        let fraction = (first / scale) / (first / scale + second / scale);
                        (before + after) * fraction - before
                    }
                    (Some(first), None) => first - before,
                    (None, Some(second)) => after - second,
                    (Some(_), Some(_)) | (None, None) => (after - before) * 0.5,
                }
            };
            let scale = if record
                .central_index()
                .is_some_and(|central| central == id.index || central == record.after_index())
            {
                record.child_extents().iter().sum::<f64>()
            } else {
                before + after
            };
            if !delta.is_finite() || !scale.is_finite() {
                return None;
            }
            let delta = delta.clamp(limits.minimum, limits.maximum);
            // Only a central child in this pair depends on the whole split's
            // remainder. Unrelated large siblings must not suppress a locally
            // representable adjustment. Ordinary displacements use no endpoint.
            let precision = scale * f64::EPSILON * 64.0;
            Some(if delta.abs() <= precision { 0.0 } else { delta })
        };
        match target {
            SplitterResizeTarget::Handle(id) => restore(id),
            SplitterResizeTarget::Junction(junction) => {
                let mut displacement = None;
                for id in junction.splitters() {
                    if record(id)?.axis() != axis {
                        continue;
                    }
                    let delta = restore(id)?;
                    if displacement.is_some_and(|previous| previous != delta) {
                        return None;
                    }
                    displacement = Some(delta);
                }
                displacement
            }
        }
    }

    fn grid_track_preferred(
        workspace: &Workspace,
        layout: ResizeLayout<'_>,
        root: RootId,
        node: NodeId,
        axis: Axis,
        track: usize,
    ) -> Option<Option<f64>> {
        let projection = layout.projection(root)?;
        let grid = projection.grids.get(&node)?;
        let mut has_hint = false;
        let mut extent: f64 = 0.0;
        for (cell, &child) in grid.children.iter().enumerate() {
            let index = match axis {
                Axis::Horizontal => cell % grid.shape.columns,
                Axis::Vertical => cell / grid.shape.columns,
            };
            if index != track || !projection.nodes.get(&child)?.visible {
                continue;
            }
            let hint = Self::subtree_axis_sizing(workspace, layout, root, child, axis)?.preferred();
            has_hint |= hint.is_some();
            let actual = rect_extent(projection.nodes.get(&child)?.rect, axis);
            extent = extent.max(hint.unwrap_or(actual));
        }
        Some(has_hint.then_some(extent))
    }

    /// Resolves hints only for an explicit restoration, using the exact selected panes.
    fn subtree_axis_sizing(
        workspace: &Workspace,
        layout: ResizeLayout<'_>,
        root: RootId,
        subtree: NodeId,
        axis: Axis,
    ) -> Option<AxisSizing> {
        let facts = layout.facts.root(root)?;
        let projection = layout.projection(root)?;
        let mut pending = vec![(subtree, false)];
        let mut sizings = BTreeMap::<NodeId, AxisSizing>::new();
        while let Some((node, visited)) = pending.pop() {
            let result = projection.nodes.get(&node)?;
            if !result.visible {
                continue;
            }
            let sizing = match workspace.node(node)? {
                Node::Pane { item } => {
                    let measured = facts.pane_sizing(PaneSizingKey::new(root, *item))?;
                    let content = pane_outer_sizing(
                        measured,
                        layout.facts.config(),
                        TabBarVisibility::Hidden,
                    )
                    .ok()?;
                    match axis {
                        Axis::Horizontal => content.width(),
                        Axis::Vertical => content.height(),
                    }
                }
                Node::Tabs { .. } => {
                    let selected = workspace.visible_tab_selection(node);
                    if let Some(child) = selected.and_then(TabKey::layout) {
                        if !visited {
                            pending.push((node, true));
                            pending.push((child, false));
                            continue;
                        }
                        let child_sizing = *sizings.get(&child)?;
                        let chrome = if axis == Axis::Vertical
                            && facts.tab_bar_visibility(node) == Some(TabBarVisibility::Visible)
                        {
                            layout.facts.config().tab_bar_height()
                        } else {
                            0.0
                        };
                        AxisSizing::new(
                            child_sizing.minimum() + chrome,
                            child_sizing.maximum().map(|maximum| maximum + chrome),
                            child_sizing.preferred().map(|preferred| preferred + chrome),
                        )
                        .ok()?
                    } else {
                        let measured = match selected.and_then(TabKey::item) {
                            Some(item) => facts.pane_sizing(PaneSizingKey::new(root, item))?,
                            None => PaneSizing::default(),
                        };
                        let visibility = facts
                            .tab_bar_visibility(node)
                            .unwrap_or(TabBarVisibility::Hidden);
                        let outer =
                            pane_outer_sizing(measured, layout.facts.config(), visibility).ok()?;
                        match axis {
                            Axis::Horizontal => outer.width(),
                            Axis::Vertical => outer.height(),
                        }
                    }
                }
                Node::Split { children, .. } | Node::Grid { children, .. } if !visited => {
                    pending.push((node, true));
                    pending.extend(children.iter().rev().map(|&child| (child, false)));
                    continue;
                }
                Node::Grid { .. } => {
                    let grid = projection.grids.get(&node)?;
                    let tracks = grid.axis(axis);
                    let mut extent = 0.0;
                    let mut maximum = Some(0.0);
                    let mut has_hint = false;
                    let mut values = vec![(0.0_f64, Some(0.0_f64), false); tracks.tracks.len()];
                    for (cell, child) in grid.children.iter().enumerate() {
                        if !projection.nodes.get(child)?.visible {
                            continue;
                        }
                        let index = match axis {
                            Axis::Horizontal => cell % grid.shape.columns,
                            Axis::Vertical => cell / grid.shape.columns,
                        };
                        let sizing = sizings.get(child)?;
                        let hint = sizing.preferred();
                        has_hint |= hint.is_some();
                        let actual = rect_extent(projection.nodes.get(child)?.rect, axis);
                        let (extent, maximum, occupied) = &mut values[index];
                        *occupied = true;
                        *extent = extent.max(hint.unwrap_or(actual));
                        *maximum = maximum
                            .zip(sizing.maximum())
                            .map(|(max, value)| max.max(value));
                    }
                    for ((mut track_extent, mut track_maximum, occupied), facts) in
                        values.into_iter().zip(&tracks.tracks)
                    {
                        if !occupied {
                            track_extent = facts.extent;
                            track_maximum = None;
                        }
                        let gap = facts
                            .following_gap
                            .map_or(0.0, |gap| rect_extent(gap, axis));
                        extent += track_extent + gap;
                        maximum = maximum
                            .zip(track_maximum)
                            .map(|(max, value)| max + value + gap);
                    }
                    let minimum = match axis {
                        Axis::Horizontal => result.constraints.min().width(),
                        Axis::Vertical => result.constraints.min().height(),
                    };
                    let preferred = has_hint.then(|| {
                        maximum.map_or(extent.max(minimum), |max| extent.clamp(minimum, max))
                    });
                    AxisSizing::new(minimum, maximum, preferred).ok()?
                }
                Node::Split {
                    axis: split_axis,
                    children,
                    ..
                } => {
                    let split = projection.splits.get(&node)?;
                    let mut extent: f64 = 0.0;
                    let mut maximum = Some(0.0_f64);
                    let mut has_hint = false;
                    for (index, child) in children.iter().enumerate() {
                        let child_result = projection.nodes.get(child)?;
                        if !child_result.visible {
                            continue;
                        }
                        let child_sizing = sizings.get(child)?;
                        let hint = child_sizing.preferred();
                        has_hint |= hint.is_some();
                        let current = rect_extent(child_result.rect, axis);
                        let child_extent = hint.unwrap_or(current);
                        let gap = split
                            .children
                            .get(index)?
                            .following_gap
                            .map_or(0.0, |gap| rect_extent(gap, axis));
                        if *split_axis == axis {
                            extent += child_extent + gap;
                            maximum = maximum
                                .zip(child_sizing.maximum())
                                .map(|(maximum, child)| maximum + child + gap);
                        } else {
                            extent = extent.max(child_extent);
                            maximum = maximum
                                .zip(child_sizing.maximum())
                                .map(|(maximum, child)| maximum.max(child));
                        }
                    }
                    if !extent.is_finite() {
                        return None;
                    }
                    let minimum = match axis {
                        Axis::Horizontal => result.constraints.min().width(),
                        Axis::Vertical => result.constraints.min().height(),
                    };
                    let preferred = has_hint.then(|| {
                        maximum.map_or(extent.max(minimum), |maximum| {
                            extent.clamp(minimum, maximum)
                        })
                    });
                    AxisSizing::new(minimum, maximum, preferred).ok()?
                }
            };
            sizings.insert(node, sizing);
        }
        sizings.remove(&subtree)
    }
}

/// Inverts the already-frozen monotone pair without resolving another layout.
fn piecewise_target_delta(
    limits: ResizeDeltaInterval,
    before_hint: Option<f64>,
    after_hint: Option<f64>,
    mut pair_at: impl FnMut(f64) -> Option<(f64, f64)>,
) -> Option<f64> {
    if !limits.minimum.is_finite()
        || !limits.maximum.is_finite()
        || limits.minimum > 0.0
        || limits.maximum < 0.0
        || [before_hint, after_hint]
            .into_iter()
            .flatten()
            .any(|hint| !hint.is_finite() || hint < 0.0)
    {
        return None;
    }
    let ratio = match (before_hint, after_hint) {
        (Some(before), Some(after)) if before > 0.0 || after > 0.0 => {
            let scale = before.max(after);
            Some((after / scale, before / scale))
        }
        (Some(_), None) | (None, Some(_)) => None,
        _ => Some((1.0, 1.0)),
    };
    let mut residual_at = |delta| {
        let (before, after) = pair_at(delta)?;
        if !before.is_finite() || !after.is_finite() || before < 0.0 || after < 0.0 {
            return None;
        }
        Some(match ratio {
            Some((before_weight, after_weight)) => before * before_weight - after * after_weight,
            None => match (before_hint, after_hint) {
                (Some(hint), None) => before - hint,
                (None, Some(hint)) => hint - after,
                _ => return None,
            },
        })
    };
    let current = 0.0;
    let initial = residual_at(current)?;
    if initial == 0.0 {
        return Some(current);
    }
    let increasing = initial < 0.0;
    let end = if increasing {
        limits.maximum
    } else {
        limits.minimum
    };
    let endpoint = residual_at(end)?;
    if (increasing && endpoint < initial) || (!increasing && endpoint > initial) {
        return None;
    }
    if endpoint == initial {
        return Some(current);
    }
    // An unreachable target uses the nearest point attaining the best endpoint
    // value. An exact plateau likewise stops at its edge nearest the current pair.
    let threshold = if increasing {
        endpoint.min(0.0)
    } else {
        endpoint.max(0.0)
    };
    let ordered = |value: f64| {
        let bits = value.to_bits();
        if value.is_sign_negative() {
            !bits
        } else {
            bits ^ (1 << 63)
        }
    };
    let from_ordered = |bits: u64| {
        f64::from_bits(if bits & (1 << 63) == 0 {
            !bits
        } else {
            bits ^ (1 << 63)
        })
    };
    let (mut near, mut far) = (ordered(current), ordered(end));
    let (mut near_error, mut far_error) = (initial, endpoint);
    // Searching representable displacements takes at most 64 comparisons,
    // including tiny local moves inside a much larger feasible outward interval.
    for _ in 0..64 {
        let distance = near.abs_diff(far);
        if distance <= 1 {
            break;
        }
        let middle = near.min(far) + distance / 2;
        let error = residual_at(from_ordered(middle))?;
        let reaches_target = if increasing {
            error >= threshold
        } else {
            error <= threshold
        };
        if reaches_target {
            far = middle;
            far_error = error;
        } else {
            near = middle;
            near_error = error;
        }
    }
    Some(from_ordered(if far_error.abs() < near_error.abs() {
        far
    } else {
        near
    }))
}

fn rect_extent(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => rect.width(),
        Axis::Vertical => rect.height(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;

    use crate::geometry::PaneSizing;
    use crate::graph::{RootRecord, SurfacePresentation, Workspace};
    use crate::ids::{ItemId, SurfaceId};
    use crate::model::{DockspaceGridShape, DockspaceGridShares};
    use crate::policy::DockPolicy;
    use crate::presentation::config::ResizePropagation;
    use crate::presentation::layout::{LayoutMetrics, TrackWeightOverride, measure_root_sizing};
    use crate::presentation::manifest::{
        Measurement, SurfaceMeasurements, TabIntrinsic, TabStripMetrics,
    };
    use crate::presentation::tab_strip::TabStripStateStore;

    const ROOT: RootId = RootId::new(971);
    const SURFACE: SurfaceId = SurfaceId::new(972);
    const A: ItemId = ItemId::new(973);
    const B: ItemId = ItemId::new(974);
    const D: ItemId = ItemId::new(975);

    fn compiled(workspace: Workspace) -> (DockEngine, PresentationPlan) {
        let config = DockPresentationConfig::builder()
            .minimum_pane_size(1.0, 1.0)
            .tab_bar_height(20.0)
            .tab_group_grip_extent(20.0)
            .splitter_thickness(4.0)
            .build()
            .unwrap();
        compiled_with_sizing(workspace, config, |item| {
            let preferred = match item {
                A => Some(80.0),
                B => Some(300.0),
                _ => None,
            };
            PaneSizing::new(
                AxisSizing::default(),
                AxisSizing::new(0.0, None, preferred).unwrap(),
            )
        })
    }

    fn compiled_with_sizing(
        workspace: Workspace,
        config: DockPresentationConfig,
        sizing: impl Fn(ItemId) -> PaneSizing,
    ) -> (DockEngine, PresentationPlan) {
        compiled_with_bounds(
            workspace,
            config,
            LogicalRect::new(0.0, 0.0, 800.0, 500.0).unwrap(),
            sizing,
        )
    }

    fn compiled_with_bounds(
        workspace: Workspace,
        config: DockPresentationConfig,
        bounds: LogicalRect,
        sizing: impl Fn(ItemId) -> PaneSizing,
    ) -> (DockEngine, PresentationPlan) {
        let engine =
            DockEngine::new_with_presentation_config(workspace, DockPolicy::default(), config)
                .unwrap();
        let requirements = engine.presentation_requirements().surface(SURFACE).unwrap();
        let mut measurements = SurfaceMeasurements::new(requirements.ticket());
        measurements
            .set_bounds(requirements.bounds(), Measurement::Measured(bounds))
            .unwrap();
        for key in requirements.pane_sizings() {
            measurements
                .insert_pane_sizing(key, Measurement::Measured(sizing(key.item())))
                .unwrap();
        }
        for key in requirements.tab_intrinsics() {
            measurements
                .insert_tab_intrinsic(key, Measurement::Measured(TabIntrinsic::new(64.0).unwrap()))
                .unwrap();
        }
        for key in requirements.tab_strips() {
            measurements
                .insert_tab_strip(
                    key,
                    Measurement::Measured(TabStripMetrics::new(0.0, 0.0).unwrap()),
                )
                .unwrap();
        }
        let plan = compile_surface_plan(
            engine.workspace(),
            engine.version(),
            engine.policy_snapshot(),
            engine.presentation_config(),
            engine.presentation_requirements(),
            &measurements,
            &TabStripStateStore::default(),
            &[],
            None,
        )
        .unwrap()
        .finalize()
        .unwrap();
        (engine, plan)
    }

    fn prepare_group(
        engine: &DockEngine,
        plan: &PresentationPlan,
        target: SplitterResizeTarget,
        axis: Axis,
    ) -> Option<FrozenResizeAxisGroup> {
        DockEngine::prepare_splitter_resize_axis_group(
            engine.workspace(),
            plan,
            target,
            axis,
            engine.policy_snapshot(),
        )
    }

    #[test]
    fn selected_layout_restoration_combines_explicit_and_current_sizes_with_each_bar_once() {
        let mut builder = Workspace::builder();
        let inner = builder.insert_node(Node::tabs([A, B]));
        let bare = builder.insert_node(Node::pane(D));
        let split = builder.insert_node(Node::equal_split(Axis::Vertical, [inner, bare]).unwrap());
        let outer = builder.insert_node(Node::tabs_from_keys(
            vec![TabKey::for_layout(split)],
            Some(TabKey::for_layout(split)),
        ));
        builder.set_root(ROOT, RootRecord::new(outer));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let (engine, plan) = compiled(builder.build().unwrap());
        let actual = DockEngine::subtree_axis_sizing(
            engine.workspace(),
            ResizeLayout::presented(&plan).unwrap(),
            ROOT,
            outer,
            Axis::Vertical,
        )
        .unwrap();
        let current_bare = plan
            .layout_facts()
            .unwrap()
            .root(ROOT)
            .unwrap()
            .projection()
            .nodes[&bare]
            .rect
            .height();
        assert_eq!(
            actual.preferred(),
            Some(20.0 + 20.0 + 80.0 + 4.0 + current_bare)
        );
    }

    #[test]
    fn bare_content_and_empty_anchor_do_not_invent_a_preference_or_an_extra_bar() {
        let mut builder = Workspace::builder();
        let bare = builder.insert_node(Node::pane(A));
        builder.set_root(ROOT, RootRecord::new(bare));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let (engine, plan) = compiled(builder.build().unwrap());
        assert_eq!(
            DockEngine::subtree_axis_sizing(
                engine.workspace(),
                ResizeLayout::presented(&plan).unwrap(),
                ROOT,
                bare,
                Axis::Vertical
            )
            .unwrap()
            .preferred(),
            Some(80.0)
        );

        let mut builder = Workspace::builder();
        let empty = builder.insert_node(Node::tabs([]));
        builder.set_root(ROOT, RootRecord::new(empty).with_central(empty));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let (engine, plan) = compiled(builder.build().unwrap());
        let baseline = DockEngine::subtree_axis_sizing(
            engine.workspace(),
            ResizeLayout::presented(&plan).unwrap(),
            ROOT,
            empty,
            Axis::Vertical,
        )
        .unwrap();
        assert_eq!(baseline.minimum(), 21.0);
        assert_eq!(baseline.preferred(), None);
    }

    #[test]
    fn nested_layout_tabs_restore_preferences_and_equalize_on_the_linear_path() {
        let mut builder = Workspace::builder();
        let first = builder.insert_node(Node::pane(A));
        let second = builder.insert_node(Node::pane(D));
        let split =
            builder.insert_node(Node::split(Axis::Vertical, [first, second], [1.0, 3.0]).unwrap());
        let inner = builder.insert_node(Node::tabs_from_keys(
            vec![TabKey::for_layout(split)],
            Some(TabKey::for_layout(split)),
        ));
        let outer = builder.insert_node(Node::tabs_from_keys(
            vec![TabKey::for_layout(inner)],
            Some(TabKey::for_layout(inner)),
        ));
        builder.set_root(ROOT, RootRecord::new(outer));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let (engine, plan) = compiled(builder.build().unwrap());
        let id = SplitterSceneId::split(ROOT, split, 0);
        let target = SplitterResizeTarget::Handle(id);
        let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
        assert!(
            group.piecewise.is_none(),
            "ordinary nested Tabs retain the linear path"
        );
        assert_eq!(
            DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Preferred
            ),
            Some(-34.0)
        );
        assert_eq!(
            DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Equalize
            ),
            Some(114.0)
        );
    }

    #[test]
    fn fixed_zero_siblings_use_exact_piecewise_updates_and_restore_press_intent() {
        for axis in [Axis::Horizontal, Axis::Vertical] {
            for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
                for grid in [false, true] {
                    let mut builder = Workspace::builder();
                    let children = [A, B, D].map(|item| builder.insert_node(Node::pane(item)));
                    let owner = builder.insert_node(if grid {
                        let (rows, columns) = match axis {
                            Axis::Horizontal => (1, 3),
                            Axis::Vertical => (3, 1),
                        };
                        Node::Grid {
                            shape: DockspaceGridShape::Fixed {
                                rows: NonZeroUsize::new(rows).unwrap(),
                                columns: NonZeroUsize::new(columns).unwrap(),
                            },
                            children: children.to_vec(),
                            shares: None,
                            layout_title: None,
                        }
                    } else {
                        Node::equal_split(axis, children).unwrap()
                    });
                    builder.set_root(ROOT, RootRecord::new(owner));
                    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
                    let config = DockPresentationConfig::builder()
                        .minimum_pane_size(1.0, 1.0)
                        .tab_bar_height(20.0)
                        .tab_group_grip_extent(20.0)
                        .splitter_thickness(4.0)
                        .resize_propagation(propagation)
                        .build()
                        .unwrap();
                    let sizing = |item| {
                        let sizing = PaneSizing::builder().min_width(1.0).min_height(1.0);
                        if item == A {
                            match axis {
                                Axis::Horizontal => sizing.fixed_width(0.0),
                                Axis::Vertical => sizing.fixed_height(0.0),
                            }
                        } else {
                            sizing
                        }
                        .build()
                        .unwrap()
                    };
                    let (engine, plan) = compiled_with_bounds(
                        builder.build().unwrap(),
                        config,
                        LogicalRect::new(0.0, 0.0, 604.0, 624.0).unwrap(),
                        sizing,
                    );
                    let id = if grid {
                        SplitterSceneId::grid(ROOT, owner, axis, 1)
                    } else {
                        SplitterSceneId::split(ROOT, owner, 1)
                    };
                    let record = plan.splitter_record(id).unwrap();
                    let original_weights = record.weights().to_vec();
                    let group =
                        prepare_group(&engine, &plan, SplitterResizeTarget::Handle(id), axis)
                            .unwrap();
                    let frozen = group.piecewise.as_ref().unwrap();
                    assert_eq!(group.common_delta.minimum, -297.0);
                    assert_eq!(group.common_delta.maximum, 297.0);
                    assert_eq!(
                        frozen.track_extents(ROOT, owner, 0.0).unwrap(),
                        [0.0, 298.0, 298.0]
                    );
                    let root = plan.layout_facts().unwrap().root(ROOT).unwrap();
                    let leaves = children
                        .into_iter()
                        .zip([A, B, D].map(sizing))
                        .collect::<BTreeMap<_, _>>();
                    for delta in [-297.0, 20.0, 297.0, 0.0] {
                        let requested = match axis {
                            Axis::Horizontal => [delta, 0.0],
                            Axis::Vertical => [0.0, delta],
                        };
                        let updates = crate::engine::splitter_geometry::resize_axis_group_updates(
                            std::slice::from_ref(&group),
                            requested,
                        )
                        .unwrap();
                        if let Some(update) = updates.first() {
                            assert_eq!(updates.len(), 1);
                            let weights = if grid {
                                let shares = update.grid_shares().unwrap();
                                match axis {
                                    Axis::Horizontal => shares.column_weights(),
                                    Axis::Vertical => shares.row_weights(),
                                }
                            } else {
                                update.split_weights().unwrap()
                            };
                            assert!(weights.iter().all(|weight| weight.get() > 0.0));
                            if delta == 0.0 {
                                assert_eq!(weights, original_weights);
                            } else {
                                assert!(weights[0].get() <= f64::EPSILON);
                            }
                        } else {
                            assert!(grid && delta == 0.0);
                        }
                        let overrides = updates
                            .iter()
                            .map(|update| {
                                TrackWeightOverride::new(update.source(), update.weights())
                            })
                            .collect::<Vec<_>>();
                        let projected = measure_root_sizing(
                            engine.workspace(),
                            ROOT,
                            &leaves,
                            LayoutMetrics::new(4.0).unwrap(),
                        )
                        .unwrap()
                        .project(root.content_bounds(), &overrides)
                        .unwrap();
                        let expected = [0.0, 298.0 + delta, 298.0 - delta];
                        assert_eq!(frozen.track_extents(ROOT, owner, delta).unwrap(), expected);
                        for (child, expected) in children.into_iter().zip(expected) {
                            let actual = rect_extent(projected.nodes[&child].rect, axis);
                            assert!(
                                (actual - expected).abs() < 1.0e-9,
                                "{axis:?}/{propagation:?}/grid={grid}/delta={delta}: {actual} != {expected}"
                            );
                        }
                    }
                }
            }
        }
    }

    fn outward_tracks(
        first_hint: Option<f64>,
        second_hint: Option<f64>,
        grid: bool,
    ) -> (DockEngine, PresentationPlan, SplitterSceneId) {
        let mut builder = Workspace::builder();
        let children = [A, B, D].map(|item| builder.insert_node(Node::pane(item)));
        let owner = builder.insert_node(if grid {
            Node::Grid {
                shape: DockspaceGridShape::Fixed {
                    rows: NonZeroUsize::new(3).unwrap(),
                    columns: NonZeroUsize::new(1).unwrap(),
                },
                children: children.to_vec(),
                shares: Some(DockspaceGridShares::new([1.0, 1.0, 2.0], [1.0]).unwrap()),
                layout_title: None,
            }
        } else {
            Node::split(Axis::Vertical, children, [1.0, 1.0, 2.0]).unwrap()
        });
        builder.set_root(ROOT, RootRecord::new(owner));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let config = DockPresentationConfig::builder()
            .minimum_pane_size(1.0, 1.0)
            .tab_bar_height(20.0)
            .tab_group_grip_extent(20.0)
            .splitter_thickness(4.0)
            .resize_propagation(ResizePropagation::Outward)
            .build()
            .unwrap();
        let (engine, plan) = compiled_with_sizing(builder.build().unwrap(), config, |item| {
            let (maximum, hint) = match item {
                B => (Some(150.0), first_hint),
                D => (None, second_hint),
                _ => (None, None),
            };
            PaneSizing::new(
                AxisSizing::default(),
                AxisSizing::new(0.0, maximum, hint).unwrap(),
            )
        });
        (
            engine,
            plan,
            if grid {
                SplitterSceneId::grid(ROOT, owner, Axis::Vertical, 1)
            } else {
                SplitterSceneId::split(ROOT, owner, 1)
            },
        )
    }

    #[test]
    fn outward_preferred_uses_the_real_incident_ratio_after_spilling_past_a_cap() {
        for grid in [true, false] {
            let (engine, plan, id) = outward_tracks(Some(150.0), Some(100.0), grid);
            let target = SplitterResizeTarget::Handle(id);
            let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
            let frozen = group.piecewise.as_ref().unwrap();
            assert_eq!(
                frozen.track_extents(ROOT, id.node(), 0.0).unwrap(),
                [118.0, 118.0, 236.0]
            );
            let delta = DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Preferred,
            )
            .unwrap();
            assert!((delta - 136.0).abs() < 1.0e-10, "{delta}");
            let actual = frozen.track_extents(ROOT, id.node(), delta).unwrap();
            for (actual, expected) in actual.into_iter().zip([222.0, 150.0, 100.0]) {
                assert!((actual - expected).abs() < 1.0e-10);
            }
        }
    }

    #[test]
    fn outward_equalize_samples_the_real_pair_after_spilling_past_a_cap() {
        for grid in [false, true] {
            let (engine, plan, id) = outward_tracks(Some(150.0), Some(100.0), grid);
            let target = SplitterResizeTarget::Handle(id);
            let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
            let frozen = group.piecewise.as_ref().unwrap();
            assert_eq!(
                frozen.track_extents(ROOT, id.node(), 59.0).unwrap(),
                [145.0, 150.0, 177.0]
            );
            let delta = DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Equalize,
            )
            .unwrap();
            assert!((delta - 86.0).abs() < 1.0e-10, "{delta}");
            let actual = frozen.track_extents(ROOT, id.node(), delta).unwrap();
            for (actual, expected) in actual.into_iter().zip([172.0, 150.0, 150.0]) {
                assert!((actual - expected).abs() < 1.0e-10);
            }
        }
    }

    #[test]
    fn outward_equalize_junction_requires_the_same_real_pair_delta_on_every_incident() {
        let [e, f, g] = [976, 977, 978].map(ItemId::new);
        for matching in [false, true] {
            let mut builder = Workspace::builder();
            let mut insert_grid = |items: [ItemId; 3]| {
                let children = items.map(|item| builder.insert_node(Node::pane(item)));
                builder.insert_node(Node::Grid {
                    shape: DockspaceGridShape::Fixed {
                        rows: NonZeroUsize::new(3).unwrap(),
                        columns: NonZeroUsize::new(1).unwrap(),
                    },
                    children: children.to_vec(),
                    shares: Some(DockspaceGridShares::new([1.0, 1.0, 2.0], [1.0]).unwrap()),
                    layout_title: None,
                })
            };
            let grids = [insert_grid([A, B, D]), insert_grid([e, f, g])];
            let root = builder.insert_node(Node::equal_split(Axis::Horizontal, grids).unwrap());
            builder.set_root(ROOT, RootRecord::new(root));
            builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
            let config = DockPresentationConfig::builder()
                .minimum_pane_size(1.0, 1.0)
                .tab_bar_height(20.0)
                .tab_group_grip_extent(20.0)
                .splitter_thickness(4.0)
                .resize_propagation(ResizePropagation::Outward)
                .build()
                .unwrap();
            let (engine, plan) = compiled_with_sizing(builder.build().unwrap(), config, |item| {
                let maximum = if item == B || (item == f && matching) {
                    Some(150.0)
                } else if item == f {
                    Some(140.0)
                } else {
                    None
                };
                PaneSizing::new(
                    AxisSizing::default(),
                    AxisSizing::new(0.0, maximum, None).unwrap(),
                )
            });
            let incident = grids.map(|grid| SplitterSceneId::grid(ROOT, grid, Axis::Vertical, 1));
            let junction = plan
                .splitter_junction_records()
                .iter()
                .find(|record| {
                    incident
                        .iter()
                        .all(|id| record.id().splitters().contains(id))
                })
                .unwrap();
            let target = SplitterResizeTarget::Junction(junction.id());
            let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
            let delta = DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Equalize,
            );
            if matching {
                assert!((delta.unwrap() - 86.0).abs() < 1.0e-10);
            } else {
                assert_eq!(delta, None);
            }
        }
    }

    #[test]
    fn outward_preferred_keeps_single_hint_plateaus_and_zero_hint_fallbacks() {
        for (before, after, expected) in [
            (Some(150.0), None, 32.0),
            (None, Some(100.0), 136.0),
            (Some(0.0), Some(0.0), 86.0),
            (None, None, 86.0),
        ] {
            let (engine, plan, id) = outward_tracks(before, after, true);
            let target = SplitterResizeTarget::Handle(id);
            let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
            let delta = DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Preferred,
            )
            .unwrap();
            assert!(
                (delta - expected).abs() < 1.0e-10,
                "{before:?}/{after:?}: {delta}"
            );
        }
    }

    #[test]
    fn outward_preferred_retains_zero_displacements_for_sub_ulp_targets() {
        let nearly_current = f64::from_bits(118.0_f64.to_bits() + 1);
        let (engine, plan, id) = outward_tracks(Some(nearly_current), Some(236.0), true);
        let target = SplitterResizeTarget::Handle(id);
        let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
        assert_eq!(
            DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Preferred
            ),
            Some(0.0)
        );
    }

    #[test]
    fn outward_preferred_junction_requires_one_displacement_from_every_incident_pair() {
        let [e, f, g] = [976, 977, 978].map(ItemId::new);
        for matching_hints in [true, false] {
            let mut builder = Workspace::builder();
            let mut insert_grid = |items: [ItemId; 3]| {
                let children = items
                    .map(|item| builder.insert_node(Node::pane(item)))
                    .to_vec();
                builder.insert_node(Node::Grid {
                    shape: DockspaceGridShape::Fixed {
                        rows: NonZeroUsize::new(3).unwrap(),
                        columns: NonZeroUsize::new(1).unwrap(),
                    },
                    children,
                    shares: Some(DockspaceGridShares::new([1.0, 1.0, 2.0], [1.0]).unwrap()),
                    layout_title: None,
                })
            };
            let grids = [insert_grid([A, B, D]), insert_grid([e, f, g])];
            let root = builder.insert_node(Node::equal_split(Axis::Horizontal, grids).unwrap());
            builder.set_root(ROOT, RootRecord::new(root));
            builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
            let config = DockPresentationConfig::builder()
                .minimum_pane_size(1.0, 1.0)
                .tab_bar_height(20.0)
                .tab_group_grip_extent(20.0)
                .splitter_thickness(4.0)
                .resize_propagation(ResizePropagation::Outward)
                .build()
                .unwrap();
            let (engine, plan) = compiled_with_sizing(builder.build().unwrap(), config, |item| {
                let (maximum, preferred) = if item == B || item == f {
                    (Some(150.0), Some(150.0))
                } else if item == D || item == g {
                    (
                        None,
                        Some(if item == D || matching_hints {
                            100.0
                        } else {
                            80.0
                        }),
                    )
                } else {
                    (None, None)
                };
                PaneSizing::new(
                    AxisSizing::default(),
                    AxisSizing::new(0.0, maximum, preferred).unwrap(),
                )
            });
            let incident = grids.map(|grid| SplitterSceneId::grid(ROOT, grid, Axis::Vertical, 1));
            let junction = plan
                .splitter_junction_records()
                .iter()
                .map(|record| record.id())
                .find(|id| {
                    let handles = id.splitters();
                    incident.iter().all(|id| handles.contains(id))
                })
                .unwrap();
            let target = SplitterResizeTarget::Junction(junction);
            let group = prepare_group(&engine, &plan, target, Axis::Vertical).unwrap();
            let delta = DockEngine::splitter_restoration_delta(
                engine.workspace(),
                &plan,
                target,
                &group,
                SplitterReset::Preferred,
            );
            if matching_hints {
                assert!((delta.unwrap() - 136.0).abs() < 1.0e-10);
            } else {
                assert_eq!(delta, None);
            }
        }
    }

    #[test]
    fn frozen_pair_inversion_handles_subnormal_hints_and_distant_outward_limits() {
        let limits = ResizeDeltaInterval {
            minimum: -f64::MAX / 4.0,
            maximum: f64::MAX / 4.0,
        };
        let tiny = f64::from_bits(1);
        for hints in [
            (None, None),
            (Some(0.0), Some(0.0)),
            (Some(tiny), Some(tiny)),
        ] {
            let mut samples = 0;
            let delta = piecewise_target_delta(limits, hints.0, hints.1, |delta| {
                samples += 1;
                let before = (40.0 + delta).clamp(0.0, 100.0);
                Some((before, 100.0 - before))
            })
            .unwrap();
            assert!((delta - 10.0).abs() < 1.0e-12);
            assert!(samples <= 66);
        }
        assert_eq!(
            piecewise_target_delta(limits, Some(tiny), Some(tiny * 2.0), |_| {
                Some((100.0, 200.0))
            }),
            Some(0.0)
        );
        assert_eq!(
            piecewise_target_delta(limits, Some(0.0), Some(0.0), |_| Some((0.0, 0.0))),
            Some(0.0)
        );
    }
}
