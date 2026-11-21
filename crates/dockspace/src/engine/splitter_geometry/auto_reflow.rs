//! Checks touching-frontier transfers against the reflowed Auto layout.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::Axis;
use crate::graph::command::TrackResize;
use crate::graph::root_structure::RootNodeKind;
use crate::ids::{NodeId, RootId};
use crate::presentation::config::PaneOverflowPolicy;
use crate::presentation::layout::{
    FrozenRootLayout, GridProjection, LayoutProjection, NodeResult, measure_resize_geometry,
};

use super::piecewise::{FrozenTracks, TrackAllocation, grid_shares_match_extents};
use super::{FrozenPiecewiseAxis, merge_axis_update, rect_extent, rect_start, tolerance};

pub(super) fn finish_auto_reflow(
    samples: &[(&FrozenPiecewiseAxis, f64)],
    inherited: &[InheritedTrackTransfer<'_>],
    updates: &mut BTreeMap<(RootId, NodeId), TrackResize>,
) -> Option<()> {
    let mut checked = BTreeSet::new();
    for (frozen, _) in samples {
        for (root, node, layout) in frozen.auto_reflow_roots() {
            if !checked.insert(root) {
                continue;
            }
            let direct = updates
                .iter()
                .filter_map(|(&(owner_root, node), update)| {
                    (owner_root == root).then_some((node, update.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            let mut transfers = BTreeMap::<NodeId, Vec<TrackTransfer<'_>>>::new();
            for target in inherited.iter().filter(|transfer| transfer.root == root) {
                transfers
                    .entry(target.node)
                    .or_default()
                    .push(TrackTransfer {
                        target,
                        state: TransferState::Stored,
                    });
            }
            let feasible = |projection: &LayoutProjection| {
                samples.iter().all(|(frozen, delta)| {
                    frozen.reflow_preserves_drivers(root, projection, *delta)
                }) && measure_resize_geometry(projection, layout.splitter_thickness())
                    .root_is_feasible(node)
            };
            project_inherited_transfers(layout, &direct, &mut transfers, feasible)?;
            for (node, axes) in transfers {
                for transfer in axes {
                    if let TransferState::Resize(update) = transfer.state {
                        merge_axis_update(updates, (root, node), transfer.target.axis, update)?;
                    }
                }
            }
        }
    }
    Some(())
}

pub(super) struct InheritedTrackTransfer<'a> {
    pub(super) root: RootId,
    pub(super) node: NodeId,
    pub(super) axis: Axis,
    pub(super) expected: TrackAllocation,
    pub(super) tracks: &'a FrozenTracks,
    pub(super) overflow_policy: PaneOverflowPolicy,
}

impl InheritedTrackTransfer<'_> {
    fn matches_cell_map(&self, grid: Option<&GridProjection>) -> bool {
        self.tracks.grid_basis().is_none_or(|basis| {
            grid.is_some_and(|grid| {
                grid.shape.rows == basis.rows()
                    && grid.shape.columns == basis.columns()
                    && grid.children.as_ref() == basis.children()
            })
        })
    }

    fn extents_match(&self, extents: impl ExactSizeIterator<Item = f64>) -> bool {
        extents.len() == self.expected.extents.len()
            && extents
                .zip(&self.expected.extents)
                .all(|(actual, &expected)| (actual - expected).abs() <= tolerance(actual, expected))
    }

    fn origin_matches(&self, node: &NodeResult) -> bool {
        let actual = rect_start(node.slot, self.axis);
        let expected = self.expected.origin;
        (actual - expected).abs() <= tolerance(actual, expected)
    }

    fn slot_matches(&self, node: &NodeResult) -> bool {
        let actual = rect_start(node.slot, self.axis) + rect_extent(node.slot, self.axis);
        let expected = self.expected.origin + self.expected.slot_extent;
        self.origin_matches(node) && (actual - expected).abs() <= tolerance(actual, expected)
    }

    fn can_reuse_grid_shares(&self, grid: &GridProjection) -> bool {
        let axis = grid.axis(self.axis);
        grid_shares_match_extents(
            self.tracks.weights(),
            &self.expected.extents,
            axis.tracks
                .iter()
                .map(|track| (track.minimum, track.maximum)),
            axis.central_index,
            self.overflow_policy,
        )
    }
}

struct TrackTransfer<'a> {
    target: &'a InheritedTrackTransfer<'a>,
    state: TransferState,
}

enum TransferState {
    Stored,
    Resize(TrackResize),
    Reflowed,
}

fn project_inherited_transfers(
    layout: &FrozenRootLayout,
    direct: &BTreeMap<NodeId, TrackResize>,
    transfers: &mut BTreeMap<NodeId, Vec<TrackTransfer<'_>>>,
    feasible: impl Fn(&LayoutProjection) -> bool,
) -> Option<()> {
    let topology = layout.topology();
    let mut pending = Vec::new();
    loop {
        let mut updates = direct.clone();
        for (&node, axes) in &*transfers {
            for transfer in axes {
                if let TransferState::Resize(update) = &transfer.state {
                    merge_axis_update(&mut updates, node, transfer.target.axis, update.clone())?;
                }
            }
        }
        let weights = updates
            .into_iter()
            .map(|(node, update)| (node, update.weights().clone()))
            .collect();
        let projection = layout.project(&weights).ok()?;
        let mut reproject = false;
        let mut allocations_match = true;
        pending.push((0, false));
        while let Some((row, mut remapped)) = pending.pop() {
            let node = &topology.nodes[row];
            let result = projection.nodes.get(&node.id)?;
            if !result.visible {
                continue;
            }
            let mut changed = false;
            if let Some(axes) = transfers.get_mut(&node.id) {
                let grid = projection.grids.get(&node.id);
                let map_changed = !axes.first()?.target.matches_cell_map(grid);
                remapped |= map_changed;
                for transfer in axes {
                    let target = transfer.target;
                    // When an ancestor remaps, each child can still inherit the
                    // old transfer if it occupies the expected slot on this axis.
                    if map_changed || (remapped && !target.slot_matches(result)) {
                        if matches!(transfer.state, TransferState::Resize(_)) {
                            transfer.state = TransferState::Reflowed;
                            changed = true;
                        }
                        continue;
                    }
                    let extents_match = if let Some(grid) = grid {
                        target.extents_match(grid.axis(target.axis).tracks.iter().map(|t| t.extent))
                    } else {
                        let split = projection.splits.get(&node.id)?;
                        target.extents_match(split.children.iter().map(|t| t.extent))
                    };
                    allocations_match &= extents_match && target.origin_matches(result);
                    if !extents_match
                        && matches!(transfer.state, TransferState::Stored)
                        && !grid.is_some_and(|grid| target.can_reuse_grid_shares(grid))
                        && let Some(update) = target
                            .tracks
                            .resize_inherited(target.axis, &target.expected)
                    {
                        transfer.state = TransferState::Resize(update);
                        changed = true;
                    }
                }
            }
            if changed {
                reproject = true;
                // Choose children from the next projection of this parent's
                // allocation. Disjoint branches can advance on the same pass.
                continue;
            }
            let children = match &node.kind {
                RootNodeKind::Pane { .. } => continue,
                RootNodeKind::Split { children, .. }
                | RootNodeKind::Tabs { children }
                | RootNodeKind::Grid { children, .. } => &topology.children[children.clone()],
            };
            pending.extend(children.iter().rev().map(|&child| (child, remapped)));
        }
        if !reproject {
            return (allocations_match && feasible(&projection)).then_some(());
        }
        // Each axis can propose a share change and discard it if its original
        // cell map or slot no longer applies. The traversal never retries an
        // individual owner against the whole root.
    }
}
