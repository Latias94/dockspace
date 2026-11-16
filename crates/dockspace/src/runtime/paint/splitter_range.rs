//! On-demand resize limits from the same touching-frontier solver as gestures.

use std::collections::BTreeMap;

use crate::geometry::{AxisSizing, LogicalRect};
use crate::graph::{Axis, Node};
use crate::ids::{NodeId, RootId};
use crate::interaction::{ResizeAdjustment, ResizeDeltaInterval};
use crate::policy::TabBarVisibility;
use crate::presentation::compiler::pane_outer_sizing;
use crate::presentation::manifest::PaneSizingKey;
use crate::presentation::scene::{SplitterResizeTarget, SplitterSceneId};

use super::super::SurfaceSplitterAdjustment;
use super::{DockspaceVisualId, SurfacePaintPlan, VisualIdentity};

/// Current and reachable sizes of the pane immediately before a splitter.
///
/// Values are fractions of the two adjacent panes' combined extent, excluding
/// their separator. Limits include all nested panes touching the moving edge,
/// not just the adjacent panes' own minimum sizes. They describe this exact
/// candidate and grant no input or presentation authority.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSplitterRange {
    value: f64,
    minimum: f64,
    maximum: f64,
}

impl SurfaceSplitterRange {
    /// Returns the current fraction occupied by the preceding pane.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    /// Returns the smallest reachable fraction under the full layout constraints.
    #[must_use]
    pub const fn minimum(self) -> f64 {
        self.minimum
    }

    /// Returns the largest reachable fraction under the full layout constraints.
    #[must_use]
    pub const fn maximum(self) -> f64 {
        self.maximum
    }
}

impl SurfacePaintPlan<'_> {
    /// Resolves the exact reachable range for a splitter in this candidate.
    ///
    /// Returns `None` when resizing is unavailable or the layout cannot prove
    /// its limits. This walks the root's touching layout and allocates temporary
    /// solver state. Query it when numeric accessibility values or application
    /// controls need the range, rather than for ordinary splitter drawing.
    #[must_use]
    pub fn splitter_resize_range(
        self,
        splitter: DockspaceVisualId,
    ) -> Option<SurfaceSplitterRange> {
        let VisualIdentity::Splitter(id) = splitter.0 else {
            return None;
        };
        let record = self
            .plan
            .splitter_record(id)
            .filter(|record| record.operable())?;
        let limits = self.candidate.splitter_resize_delta_interval(
            self.plan,
            SplitterResizeTarget::Handle(id),
            record.axis(),
        )?;
        let before = *record.child_extents().get(id.index)?;
        let after = *record.child_extents().get(record.after_index())?;
        let total = before + after;
        if !total.is_finite() || total <= 0.0 {
            return None;
        }
        Some(SurfaceSplitterRange {
            value: before / total,
            minimum: ((before + limits.minimum) / total).clamp(0.0, 1.0),
            maximum: ((before + limits.maximum) / total).clamp(0.0, 1.0),
        })
    }

    pub(super) fn splitter_adjustment_sequence(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        adjustments: impl IntoIterator<Item = SurfaceSplitterAdjustment>,
    ) -> Option<ResizeAdjustment> {
        let mut adjustments = adjustments.into_iter();
        let first = adjustments.next()?;
        let Some(second) = adjustments.next() else {
            return self
                .splitter_adjustment_delta(target, axis, first)
                .map(ResizeAdjustment::new);
        };
        let limits = self
            .candidate
            .splitter_resize_delta_interval(self.plan, target, axis)?;
        let mut sequence = ResizeAdjustment::new(0.0);
        for adjustment in [first, second].into_iter().chain(adjustments) {
            let delta = match adjustment {
                SurfaceSplitterAdjustment::Decrement => {
                    sequence.delta() - self.splitter_keyboard_step
                }
                SurfaceSplitterAdjustment::Increment => {
                    sequence.delta() + self.splitter_keyboard_step
                }
                SurfaceSplitterAdjustment::Minimum => limits.minimum,
                SurfaceSplitterAdjustment::Maximum => limits.maximum,
                SurfaceSplitterAdjustment::Equalize | SurfaceSplitterAdjustment::Preferred => self
                    .splitter_restoration_delta(
                        target,
                        axis,
                        limits,
                        adjustment,
                        sequence.delta(),
                    )?,
            }
            .clamp(limits.minimum, limits.maximum);
            sequence.record(delta);
        }
        Some(sequence)
    }

    fn splitter_adjustment_delta(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        adjustment: SurfaceSplitterAdjustment,
    ) -> Option<f64> {
        match adjustment {
            SurfaceSplitterAdjustment::Decrement => Some(-self.splitter_keyboard_step),
            SurfaceSplitterAdjustment::Increment => Some(self.splitter_keyboard_step),
            SurfaceSplitterAdjustment::Minimum
            | SurfaceSplitterAdjustment::Maximum
            | SurfaceSplitterAdjustment::Equalize
            | SurfaceSplitterAdjustment::Preferred => {
                let limits = self
                    .candidate
                    .splitter_resize_delta_interval(self.plan, target, axis)?;
                if adjustment == SurfaceSplitterAdjustment::Minimum {
                    Some(limits.minimum)
                } else if adjustment == SurfaceSplitterAdjustment::Maximum {
                    Some(limits.maximum)
                } else {
                    self.splitter_restoration_delta(target, axis, limits, adjustment, 0.0)
                }
            }
        }
    }

    fn splitter_restoration_delta(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        limits: ResizeDeltaInterval,
        adjustment: SurfaceSplitterAdjustment,
        cumulative_delta: f64,
    ) -> Option<f64> {
        let current_extents =
            if adjustment == SurfaceSplitterAdjustment::Preferred && cumulative_delta != 0.0 {
                Some(self.candidate.splitter_resize_axis_extents(
                    self.plan,
                    target,
                    axis,
                    cumulative_delta,
                )?)
            } else {
                None
            };
        let restore = |id: SplitterSceneId| {
            let record = self.plan.splitter_record(id)?;
            let before = *record.child_extents().get(id.index)?;
            let after = *record.child_extents().get(record.after_index())?;
            let delta = match adjustment {
                SurfaceSplitterAdjustment::Equalize => (after - before) * 0.5,
                SurfaceSplitterAdjustment::Preferred => {
                    let Node::Split { children, .. } = self.candidate.workspace().node(id.split)?
                    else {
                        return None;
                    };
                    let before_node = *children.get(id.index)?;
                    let after_node = *children.get(record.after_index())?;
                    let before_hint = self
                        .subtree_axis_sizing(id.root, before_node, axis, current_extents.as_ref())?
                        .preferred();
                    let after_hint = self
                        .subtree_axis_sizing(id.root, after_node, axis, current_extents.as_ref())?
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
                _ => return None,
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
                    if self.plan.splitter_record(id)?.axis() != axis {
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
        self,
        root: RootId,
        subtree: NodeId,
        axis: Axis,
        current_extents: Option<&BTreeMap<NodeId, f64>>,
    ) -> Option<AxisSizing> {
        let layout = self.plan.layout_facts()?;
        let facts = layout.root(root)?;
        let projection = facts.projection();
        let workspace = self.candidate.workspace();
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
                    let measured = facts.pane_sizing(PaneSizingKey::new(root, node, selected))?;
                    let visibility = if self
                        .plan
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
