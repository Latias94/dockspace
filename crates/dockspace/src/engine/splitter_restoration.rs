//! Absolute splitter restoration sharing the ordered adjustment calculation.

use std::collections::BTreeMap;

use crate::geometry::{AxisSizing, LogicalRect};
use crate::graph::{Axis, Node};
use crate::ids::{NodeId, RootId};
use crate::interaction::{ResizeAdjustment, ResizeDeltaInterval};
use crate::policy::TabBarVisibility;
use crate::presentation::compiler::pane_outer_sizing;
use crate::presentation::manifest::PaneSizingKey;
use crate::presentation::scene::{SplitterResizeTarget, SplitterSceneId};

use super::*;

impl DockEngine {
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
        let Some(plan) = self.splitter_reset_projection(witness, policy) else {
            return Ok(InteractionOutcome::Rejected(
                InteractionRejection::SplitterResizeGeometryUnavailable,
            ));
        };
        let target = SplitterResizeTarget::Handle(splitter);
        let adjustment = plan.splitter_record(splitter).and_then(|record| {
            (record.operable() && plan.region_is_operable(record.hit().rect(), record.layer()))
                .then_some(())?;
            let axis = record.axis();
            let limits = self.splitter_resize_delta_interval(&plan, target, axis)?;
            self.splitter_restoration_delta(&plan, target, axis, limits, reset, 0.0)
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

    fn splitter_reset_projection(
        &self,
        witness: &crate::presentation::scene::SurfacePlanScene,
        policy: &DockPolicySnapshot,
    ) -> Option<PresentationPlan> {
        let requirements = &self.presentation_authority.presentation_requirements;
        let ticket = requirements.surface(witness.stamp().surface())?.ticket();
        let measurements = witness
            .plan()
            .layout_facts()?
            .measurements()
            .reticket(ticket);
        compile_surface_plan(
            &self.workspace,
            self.version,
            policy,
            &self.presentation_authority.presentation_config,
            requirements,
            &measurements,
            &self.presentation_authority.tab_strip_states,
            &[],
            self.contained_layout_projection(witness.stamp().surface()),
        )
        .ok()?
        .finalize()
        .ok()
    }

    pub(super) fn splitter_restoration_delta(
        &self,
        plan: &PresentationPlan,
        target: SplitterResizeTarget,
        axis: Axis,
        limits: ResizeDeltaInterval,
        adjustment: SplitterReset,
        cumulative_delta: f64,
    ) -> Option<f64> {
        let current_extents = if adjustment == SplitterReset::Preferred && cumulative_delta != 0.0 {
            Some(self.splitter_resize_axis_extents(plan, target, axis, cumulative_delta)?)
        } else {
            None
        };
        let restore = |id: SplitterSceneId| {
            let record = plan.splitter_record(id)?;
            let before = *record.child_extents().get(id.index)?;
            let after = *record.child_extents().get(record.after_index())?;
            let delta = match adjustment {
                SplitterReset::Equalize => (after - before) * 0.5,
                SplitterReset::Preferred => {
                    let Node::Split { children, .. } = self.workspace.node(id.split)? else {
                        return None;
                    };
                    let before_node = *children.get(id.index)?;
                    let after_node = *children.get(record.after_index())?;
                    let before_hint = self
                        .subtree_axis_sizing(
                            plan,
                            id.root,
                            before_node,
                            axis,
                            current_extents.as_ref(),
                        )?
                        .preferred();
                    let after_hint = self
                        .subtree_axis_sizing(
                            plan,
                            id.root,
                            after_node,
                            axis,
                            current_extents.as_ref(),
                        )?
                        .preferred();
                    let current_before = current_extents
                        .as_ref()
                        .map_or(Some(before), |extents| extents.get(&before_node).copied())?;
                    let current_after = current_extents
                        .as_ref()
                        .map_or(Some(after), |extents| extents.get(&after_node).copied())?;
                    match (before_hint, after_hint) {
                        (Some(first), Some(second)) if first > 0.0 || second > 0.0 => {
                            let scale = first.max(second);
                            let fraction = (first / scale) / (first / scale + second / scale);
                            cumulative_delta
                                + ((current_before + current_after) * fraction - current_before)
                        }
                        (Some(first), None) => cumulative_delta + (first - current_before),
                        (None, Some(second)) => cumulative_delta + (current_after - second),
                        (Some(_), Some(_)) | (None, None) => (after - before) * 0.5,
                    }
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
                    if plan.splitter_record(id)?.axis() != axis {
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

    /// Resolves hints only for an explicit restoration, using the exact selected panes.
    fn subtree_axis_sizing(
        &self,
        plan: &PresentationPlan,
        root: RootId,
        subtree: NodeId,
        axis: Axis,
        current_extents: Option<&BTreeMap<NodeId, f64>>,
    ) -> Option<AxisSizing> {
        let layout = plan.layout_facts()?;
        let facts = layout.root(root)?;
        let projection = facts.projection();
        let workspace = &self.workspace;
        let mut pending = vec![(subtree, false)];
        let mut sizings = BTreeMap::<NodeId, AxisSizing>::new();
        while let Some((node, visited)) = pending.pop() {
            let result = projection.nodes.get(&node)?;
            if !result.visible {
                continue;
            }
            let sizing = match workspace.node(node)? {
                Node::Tabs { .. } => {
                    let selected = workspace.visible_tab_selection(node);
                    let measured = facts.pane_sizing(PaneSizingKey::new(
                        root,
                        node,
                        selected.and_then(TabKey::item),
                    ))?;
                    let visibility = if plan
                        .tab_bar_records()
                        .iter()
                        .any(|bar| bar.id().root == root && bar.id().tabs == node)
                    {
                        TabBarVisibility::Visible
                    } else {
                        TabBarVisibility::Hidden
                    };
                    let outer = pane_outer_sizing(measured, layout.config(), visibility).ok()?;
                    match axis {
                        Axis::Horizontal => outer.width(),
                        Axis::Vertical => outer.height(),
                    }
                }
                Node::Split { children, .. } if !visited => {
                    pending.push((node, true));
                    pending.extend(children.iter().rev().map(|&child| (child, false)));
                    continue;
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
                        let current = current_extents.map_or_else(
                            || Some(rect_extent(child_result.rect, axis)),
                            |extents| extents.get(child).copied(),
                        )?;
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

fn rect_extent(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => rect.width(),
        Axis::Vertical => rect.height(),
    }
}
