//! Checks touching-frontier transfers against the reflowed Auto layout.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::Axis;
use crate::graph::command::TrackResize;
use crate::graph::root_structure::RootNodeKind;
use crate::ids::{NodeId, RootId};
use crate::presentation::config::PaneOverflowPolicy;
use crate::presentation::layout::{
    FrozenRootLayout, GridProjection, LayoutProjection, measure_resize_geometry,
};

use super::piecewise::{FrozenTracks, TrackAllocation, grid_shares_match_extents};
use super::{FrozenPiecewiseAxis, merge_axis_update, rect_start, tolerance};

pub(super) fn finish_auto_reflow(
    samples: &[(&FrozenPiecewiseAxis, f64)],
    inherited: &[InheritedGridTransfer<'_>],
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
            let mut transfers = BTreeMap::<NodeId, Vec<GridTransfer<'_>>>::new();
            for target in inherited.iter().filter(|transfer| transfer.root == root) {
                transfers
                    .entry(target.node)
                    .or_default()
                    .push(GridTransfer {
                        target,
                        state: GridShares::Stored,
                    });
            }
            let feasible = |projection: &LayoutProjection| {
                samples.iter().all(|(frozen, delta)| {
                    frozen.reflow_preserves_drivers(root, projection, *delta)
                }) && measure_resize_geometry(projection, layout.splitter_thickness())
                    .root_is_feasible(node)
            };
            project_inherited_grid_transfers(layout, &direct, &mut transfers, feasible)?;
            for (node, axes) in transfers {
                for transfer in axes {
                    if let GridShares::Resize(update) = transfer.state {
                        merge_axis_update(updates, (root, node), transfer.target.axis, update)?;
                    }
                }
            }
        }
    }
    Some(())
}

pub(super) struct InheritedGridTransfer<'a> {
    pub(super) root: RootId,
    pub(super) node: NodeId,
    pub(super) axis: Axis,
    pub(super) expected: TrackAllocation,
    pub(super) tracks: &'a FrozenTracks,
    pub(super) overflow_policy: PaneOverflowPolicy,
}

impl InheritedGridTransfer<'_> {
    fn matches_cell_map(&self, grid: &GridProjection) -> bool {
        self.tracks.grid_basis().is_some_and(|basis| {
            grid.shape.rows == basis.rows()
                && grid.shape.columns == basis.columns()
                && grid.children.as_ref() == basis.children()
        })
    }

    fn extents_match(&self, grid: &GridProjection) -> bool {
        grid.axis(self.axis)
            .tracks
            .iter()
            .zip(&self.expected.extents)
            .all(|(track, &extent)| {
                (track.extent - extent).abs() <= tolerance(track.extent, extent)
            })
    }

    fn origin_matches(&self, grid: &GridProjection) -> bool {
        grid.axis(self.axis).tracks.first().is_some_and(|track| {
            let actual = rect_start(track.bounds, self.axis);
            let expected = self.expected.origin;
            (actual - expected).abs() <= tolerance(actual, expected)
        })
    }

    fn can_reuse_shares(&self, grid: &GridProjection) -> bool {
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

struct GridTransfer<'a> {
    target: &'a InheritedGridTransfer<'a>,
    state: GridShares,
}

enum GridShares {
    Stored,
    Resize(TrackResize),
    Reflowed,
}

fn project_inherited_grid_transfers(
    layout: &FrozenRootLayout,
    direct: &BTreeMap<NodeId, TrackResize>,
    transfers: &mut BTreeMap<NodeId, Vec<GridTransfer<'_>>>,
    feasible: impl Fn(&LayoutProjection) -> bool,
) -> Option<()> {
    let topology = layout.topology();
    let mut pending = Vec::new();
    loop {
        let mut updates = direct.clone();
        for (&node, axes) in &*transfers {
            for transfer in axes {
                if let GridShares::Resize(update) = &transfer.state {
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
            if !projection.nodes.get(&node.id)?.visible {
                continue;
            }
            let mut changed = false;
            if let Some(axes) = transfers.get_mut(&node.id) {
                let grid = projection.grids.get(&node.id)?;
                remapped |= !axes[0].target.matches_cell_map(grid);
                for transfer in axes {
                    if remapped {
                        if matches!(transfer.state, GridShares::Resize(_)) {
                            transfer.state = GridShares::Reflowed;
                            changed = true;
                        }
                        continue;
                    }
                    let target = transfer.target;
                    let extents_match = target.extents_match(grid);
                    allocations_match &= extents_match && target.origin_matches(grid);
                    if !extents_match
                        && matches!(transfer.state, GridShares::Stored)
                        && !target.can_reuse_shares(grid)
                        && let Some(update) = target
                            .tracks
                            .resize_inherited_grid(target.axis, &target.expected)
                    {
                        transfer.state = GridShares::Resize(update);
                        changed = true;
                    }
                }
            }
            reproject |= changed;
            if changed && !remapped {
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
        // Each axis can request a share change once and discard it once if its
        // original cell map no longer applies. The traversal never retries an
        // individual owner against the whole root.
    }
}
