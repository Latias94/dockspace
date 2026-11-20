//! Checks touching-frontier transfers against the reflowed Auto layout.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::command::TrackResize;
use crate::graph::{Axis, SplitWeight};
use crate::ids::{NodeId, RootId};
use crate::presentation::config::PaneOverflowPolicy;
use crate::presentation::layout::{
    FrozenRootLayout, GridProjection, LayoutProjection, measure_resize_geometry,
};

use super::{FrozenPiecewiseAxis, merge_axis_update, piecewise, tolerance};

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
            let transfers = inherited
                .iter()
                .filter(|transfer| transfer.update.source().root() == root);
            let feasible = |projection: &LayoutProjection| {
                samples.iter().all(|(frozen, delta)| {
                    frozen.reflow_preserves_drivers(root, projection, *delta)
                }) && measure_resize_geometry(projection, layout.splitter_thickness())
                    .root_is_feasible(node)
            };
            let required = project_inherited_grid_transfers(layout, direct, transfers, feasible)?;
            for transfer in required {
                let update = &transfer.update;
                merge_axis_update(
                    updates,
                    (root, update.source().node()),
                    transfer.axis,
                    update.clone(),
                )?;
            }
        }
    }
    Some(())
}

pub(super) struct InheritedGridTransfer<'a> {
    pub(super) axis: Axis,
    pub(super) expected: Vec<f64>,
    pub(super) original: &'a [SplitWeight],
    pub(super) overflow_policy: PaneOverflowPolicy,
    pub(super) update: TrackResize,
}

impl InheritedGridTransfer<'_> {
    fn matching_grid<'a>(&self, projection: &'a LayoutProjection) -> Option<&'a GridProjection> {
        let grid = projection.grids.get(&self.update.source().node())?;
        let basis = self.update.grid_basis()?;
        (grid.shape.rows == basis.rows() && grid.shape.columns == basis.columns()).then_some(grid)
    }

    fn allocation_matches(&self, projection: &LayoutProjection) -> Option<bool> {
        let grid = self.matching_grid(projection)?;
        Some(
            grid.axis(self.axis)
                .tracks
                .iter()
                .zip(&self.expected)
                .all(|(track, &extent)| {
                    (track.extent - extent).abs() <= tolerance(track.extent, extent)
                }),
        )
    }

    fn can_reuse_shares(&self, projection: &LayoutProjection) -> bool {
        self.matching_grid(projection).is_some_and(|grid| {
            let axis = grid.axis(self.axis);
            piecewise::grid_shares_match_extents(
                self.original,
                &self.expected,
                axis.tracks
                    .iter()
                    .map(|track| (track.minimum, track.maximum)),
                axis.central_index,
                self.overflow_policy,
            )
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InheritedTransferState {
    Pending,
    Required,
    Reflowed,
}

fn project_inherited_grid_transfers<'a, 'frozen>(
    layout: &FrozenRootLayout,
    direct: BTreeMap<NodeId, TrackResize>,
    transfers: impl Iterator<Item = &'a InheritedGridTransfer<'frozen>>,
    feasible: impl Fn(&LayoutProjection) -> bool,
) -> Option<Vec<&'a InheritedGridTransfer<'frozen>>> {
    use InheritedTransferState::{Pending, Reflowed, Required};

    let project = |transfers: &[(&InheritedGridTransfer<'_>, InheritedTransferState)]| {
        let mut updates = direct.clone();
        for &(transfer, state) in transfers {
            if state == Required {
                let node = transfer.update.source().node();
                merge_axis_update(&mut updates, node, transfer.axis, transfer.update.clone())?;
            }
        }
        let weights = updates
            .iter()
            .map(|(&node, update)| (node, update.weights().clone()))
            .collect();
        layout.project(&weights).ok()
    };
    // Start with the existing Grid shares. Feeding pixel-derived shares into the
    // provisional pass can change nested Auto shapes before constraints are measured.
    let mut projection = project(&[])?;
    let mut transfers = transfers
        .map(|transfer| (transfer, Pending))
        .collect::<Vec<_>>();
    loop {
        let mut changed = false;
        let mut mismatch = false;
        for (transfer, state) in &mut transfers {
            match (*state, transfer.allocation_matches(&projection)) {
                (Pending, Some(false)) => *state = Required,
                (Required, None) => *state = Reflowed,
                (Required | Reflowed, Some(false)) => {
                    mismatch = true;
                    continue;
                }
                _ => continue,
            }
            changed = true;
        }
        if !changed {
            if mismatch || !feasible(&projection) {
                return None;
            }
            break;
        }
        // Each axis can be added once and retired once if its Grid changes shape.
        // A transient projection for one Grid cannot rewrite its siblings' shares.
        projection = project(&transfers)?;
    }

    for index in 0..transfers.len() {
        let (transfer, state) = transfers[index];
        if state != Required || !transfer.can_reuse_shares(&projection) {
            continue;
        }
        // Once its parent has reached the requested allocation, a child may
        // achieve the same geometry under its original shares and final constraints.
        transfers[index].1 = Pending;
        if let Some(candidate) = project(&transfers)
            && transfers.iter().all(|&(transfer, state)| {
                match transfer.allocation_matches(&candidate) {
                    Some(matches) => matches,
                    None => state != Required,
                }
            })
            && feasible(&candidate)
        {
            projection = candidate;
        } else {
            transfers[index].1 = Required;
        }
    }
    Some(
        transfers
            .into_iter()
            .filter_map(|(transfer, state)| (state == Required).then_some(transfer))
            .collect(),
    )
}
