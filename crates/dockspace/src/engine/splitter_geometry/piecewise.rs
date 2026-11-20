//! Frozen, piecewise track transfers for cap contacts and outward propagation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::*;
use crate::graph::command::GridResizeBasis;
use crate::model::DockspaceGridShares;
use crate::presentation::config::{PaneOverflowPolicy, ResizePropagation};
use crate::presentation::layout::{FrozenRootLayout, NodeResult, solve_grid_axis};
use crate::presentation::scene::SplitterOwner;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenPiecewiseAxis {
    axis: Axis,
    propagation: ResizePropagation,
    overflow_policy: PaneOverflowPolicy,
    roots: Vec<FrozenRoot>,
    sources: Vec<NodeSource>,
}

#[derive(Debug, Clone, PartialEq)]
struct FrozenRoot {
    source: NodeSource,
    nodes: BTreeMap<NodeId, FrozenNode>,
    drivers: BTreeMap<NodeId, Vec<usize>>,
    auto_reflow: Option<Arc<FrozenRootLayout>>,
}

#[derive(Debug, Clone, PartialEq)]
struct FrozenNode {
    result: NodeResult,
    kind: FrozenKind,
}

#[derive(Debug, Clone, PartialEq)]
enum FrozenKind {
    Pane,
    Tabs { child: Option<NodeId>, header: f64 },
    Parallel(Vec<NodeId>),
    Tracks(FrozenTracks),
}

#[derive(Debug, Clone, PartialEq)]
struct FrozenTracks {
    source: NodeSource,
    frame_extent: f64,
    children: Vec<(NodeId, usize)>,
    tracks: Vec<Track>,
    weights: Vec<SplitWeight>,
    central: Option<usize>,
    grid: Option<(GridResizeBasis, DockspaceGridShares)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Track {
    visible: bool,
    minimum: f64,
    maximum: f64,
    extent: f64,
    gap: f64,
}

#[derive(Clone, Copy)]
struct Scalar {
    value: f64,
    slope: f64,
}

impl Scalar {
    const fn fixed(value: f64) -> Self {
        Self { value, slope: 0.0 }
    }
    fn add(self, value: f64) -> Self {
        Self {
            value: self.value + value,
            ..self
        }
    }
    fn difference(self, other: Self) -> Self {
        Self {
            value: self.value - other.value,
            slope: self.slope - other.slope,
        }
    }
}

pub(super) struct Sample {
    pub(super) updates: BTreeMap<(RootId, NodeId), TrackResize>,
    pub(super) tracks: BTreeMap<(RootId, NodeId), Vec<f64>>,
}

#[derive(Debug)]
enum SampleError {
    Limit(f64),
    Invalid,
}

pub(super) fn prepare(
    workspace: &Workspace,
    layout: ResizeLayout<'_>,
    handles: &[(NodeSource, SplitterRecord)],
) -> Option<Vec<FrozenResizeAxisGroup>> {
    let facts = layout.facts;
    let mut axes = BTreeMap::<Axis, BTreeMap<RootId, Vec<(NodeId, usize)>>>::new();
    for (source, record) in handles {
        let id = *record.id();
        if source.root() != id.root || source.node() != id.node() || !record.operable() {
            return None;
        }
        workspace
            .verify_reference(
                id.root,
                id.node(),
                source.reference(),
                crate::graph::error::ReferenceRole::Source,
            )
            .ok()?;
        let projection = layout.projection(id.root)?;
        match (id.owner, workspace.node(id.node())?) {
            (
                SplitterOwner::Split(_),
                Node::Split {
                    axis,
                    children,
                    weights,
                    ..
                },
            ) => {
                let split = projection.splits.get(&id.node())?;
                if *axis != record.axis()
                    || layout.split_weights(id.root, id.node(), weights) != record.weights()
                    || split.following_child_index(id.index) != Some(record.after_index())
                    || children.len() != split.children.len()
                    || !record
                        .child_extents()
                        .iter()
                        .copied()
                        .eq(split.children.iter().map(|track| track.extent))
                {
                    return None;
                }
            }
            (SplitterOwner::Grid { axis, .. }, Node::Grid { children, .. }) => {
                let grid = projection.grids.get(&id.node())?;
                let proof = record.grid()?;
                let tracks = grid.axis(axis);
                if axis != record.axis()
                    || grid.shape.rows != proof.rows()
                    || grid.shape.columns != proof.columns()
                    || children.as_slice() != proof.children()
                    || children.as_slice() != grid.children.as_ref()
                    || tracks.weights.as_slice() != record.weights()
                    || id.index + 1 != record.after_index()
                    || id.index + 1 >= tracks.tracks.len()
                    || !record
                        .child_extents()
                        .iter()
                        .copied()
                        .eq(tracks.tracks.iter().map(|track| track.extent))
                {
                    return None;
                }
            }
            _ => return None,
        }
        axes.entry(record.axis())
            .or_default()
            .entry(id.root)
            .or_default()
            .push((id.node(), id.index));
    }
    axes.into_iter()
        .map(|(axis, roots)| {
            let mut frozen = FrozenPiecewiseAxis {
                axis,
                propagation: facts.config().resize_propagation(),
                overflow_policy: facts.config().pane_overflow_policy(),
                roots: Vec::new(),
                sources: Vec::new(),
            };
            let mut maximum_span: f64 = 0.0;
            for (root, drivers) in roots {
                let root_node = workspace.root(root)?.node;
                let source = workspace.capture_node_source(root, root_node).ok()?;
                let projection = layout.projection(root)?;
                if !projection
                    .nodes
                    .shares_structure(workspace.root_structure(root)?)
                {
                    return None;
                }
                if projection.nodes.get(&root_node)?.slot != facts.root(root)?.content_bounds() {
                    return None;
                }
                maximum_span =
                    maximum_span.max(rect_extent(projection.nodes.get(&root_node)?.rect, axis));
                let mut nodes = BTreeMap::new();
                for (&node, result) in projection.nodes.iter_preorder() {
                    if !result.visible {
                        continue;
                    }
                    let kind = match workspace.node(node)? {
                        Node::Pane { .. } => FrozenKind::Pane,
                        Node::Tabs { .. } => {
                            let child = workspace
                                .visible_tab_selection(node)
                                .and_then(crate::ids::TabKey::layout);
                            let header = if axis == Axis::Vertical
                                && facts.root(root)?.tab_bar_visibility(node)
                                    == Some(crate::policy::TabBarVisibility::Visible)
                            {
                                facts.config().tab_bar_height()
                            } else {
                                0.0
                            };
                            FrozenKind::Tabs { child, header }
                        }
                        Node::Split {
                            axis: split_axis,
                            children,
                            weights,
                            ..
                        } if *split_axis == axis => {
                            let split = projection.splits.get(&node)?;
                            let tracks = split
                                .children
                                .iter()
                                .map(|child| Track {
                                    visible: child.visible,
                                    minimum: child.minimum,
                                    maximum: child.maximum,
                                    extent: child.extent,
                                    gap: child
                                        .following_gap
                                        .map_or(0.0, |gap| rect_extent(gap, axis)),
                                })
                                .collect();
                            FrozenKind::Tracks(FrozenTracks {
                                source: NodeSource {
                                    root,
                                    node,
                                    reference: source.reference().clone(),
                                },
                                frame_extent: rect_extent(result.rect, axis),
                                children: children
                                    .iter()
                                    .copied()
                                    .enumerate()
                                    .filter(|(_, child)| {
                                        projection
                                            .nodes
                                            .get(child)
                                            .is_some_and(|result| result.visible)
                                    })
                                    .map(|(index, child)| (child, index))
                                    .collect(),
                                tracks,
                                weights: layout.split_weights(root, node, weights).to_vec(),
                                central: split.central_index,
                                grid: None,
                            })
                        }
                        Node::Split { children, .. } => FrozenKind::Parallel(
                            children
                                .iter()
                                .copied()
                                .filter(|child| {
                                    projection
                                        .nodes
                                        .get(child)
                                        .is_some_and(|result| result.visible)
                                })
                                .collect(),
                        ),
                        Node::Grid { children, .. } => {
                            let grid = projection.grids.get(&node)?;
                            let projected = grid.axis(axis);
                            let shares = DockspaceGridShares::from_normalized(
                                grid.rows.weights.clone(),
                                grid.columns.weights.clone(),
                            )
                            .ok()?;
                            let tracks = projected
                                .tracks
                                .iter()
                                .map(|track| Track {
                                    visible: true,
                                    minimum: track.minimum,
                                    maximum: track.maximum,
                                    extent: track.extent,
                                    gap: track
                                        .following_gap
                                        .map_or(0.0, |gap| rect_extent(gap, axis)),
                                })
                                .collect();
                            FrozenKind::Tracks(FrozenTracks {
                                source: NodeSource {
                                    root,
                                    node,
                                    reference: source.reference().clone(),
                                },
                                frame_extent: rect_extent(result.rect, axis),
                                children: children
                                    .iter()
                                    .copied()
                                    .enumerate()
                                    .filter(|(_, child)| {
                                        projection
                                            .nodes
                                            .get(child)
                                            .is_some_and(|result| result.visible)
                                    })
                                    .map(|(index, child)| {
                                        (
                                            child,
                                            match axis {
                                                Axis::Horizontal => index % grid.shape.columns,
                                                Axis::Vertical => index / grid.shape.columns,
                                            },
                                        )
                                    })
                                    .collect(),
                                tracks,
                                weights: projected.weights.clone(),
                                central: projected.central_index,
                                grid: Some((
                                    GridResizeBasis::new(
                                        grid.shape.rows,
                                        grid.shape.columns,
                                        Arc::clone(&grid.children),
                                    )?,
                                    shares,
                                )),
                            })
                        }
                    };
                    if let FrozenKind::Tracks(tracks) = &kind {
                        let allocated = tracks
                            .tracks
                            .iter()
                            .map(|track| track.extent + track.gap)
                            .sum::<f64>();
                        let frame = rect_extent(result.rect, axis);
                        // The relative baseline can retain coordinate roundoff,
                        // but must not conceal real overflow or unused space.
                        if !allocated.is_finite()
                            || (allocated - frame).abs()
                                > tolerance(allocated, frame)
                                    .max(rect_extent_tolerance(result.rect, axis))
                        {
                            return None;
                        }
                        for track in &tracks.tracks {
                            if track.visible
                                && (!track.extent.is_finite()
                                    || track.extent
                                        < track.minimum - tolerance(track.extent, track.minimum)
                                    || track.extent
                                        > track.maximum + tolerance(track.extent, track.maximum))
                            {
                                return None;
                            }
                        }
                    }
                    nodes.insert(
                        node,
                        FrozenNode {
                            result: *result,
                            kind,
                        },
                    );
                }
                let mut frozen_drivers = BTreeMap::<NodeId, Vec<usize>>::new();
                for (owner, index) in drivers {
                    frozen_drivers.entry(owner).or_default().push(index);
                }
                let mut root = FrozenRoot {
                    source,
                    nodes,
                    drivers: frozen_drivers,
                    auto_reflow: projection.auto_reflow().cloned(),
                };
                root.freeze_frame_extents(axis)?;
                frozen.roots.push(root);
            }
            if maximum_span <= 0.0 || !maximum_span.is_finite() {
                return None;
            }
            let (minimum, low) = frozen.sample_clamped(-maximum_span)?;
            let (maximum, high) = frozen.sample_clamped(maximum_span)?;
            if minimum == 0.0 && maximum == 0.0 {
                return None;
            }
            let affected = low
                .updates
                .keys()
                .chain(high.updates.keys())
                .copied()
                .collect::<BTreeSet<_>>();
            frozen.sources = affected
                .into_iter()
                .filter_map(|key| low.updates.get(&key).or_else(|| high.updates.get(&key)))
                .map(|update| update.source().clone())
                .collect();
            for root in &frozen.roots {
                for &owner in root.drivers.keys() {
                    if !frozen
                        .sources
                        .iter()
                        .any(|source| source.root() == root.source.root() && source.node() == owner)
                    {
                        let FrozenKind::Tracks(tracks) = &root.nodes.get(&owner)?.kind else {
                            return None;
                        };
                        frozen.sources.push(tracks.source.clone());
                    }
                }
            }
            Some(FrozenResizeAxisGroup {
                axis,
                splits: Vec::new(),
                common_delta: ResizeDeltaInterval { minimum, maximum },
                piecewise: Some(Arc::new(frozen)),
            })
        })
        .collect()
}

impl FrozenRoot {
    fn freeze_frame_extents(&mut self, axis: Axis) -> Option<()> {
        let root = self.source.node();
        let mut pending = vec![(root, rect_extent(self.nodes.get(&root)?.result.slot, axis))];
        while let Some((node, offered)) = pending.pop() {
            let node = self.nodes.get_mut(&node)?;
            let frame = offered.min(size_extent(node.result.constraints.max(), axis));
            match &mut node.kind {
                FrozenKind::Pane => {}
                FrozenKind::Tabs { child, header } => {
                    if let Some(child) = child {
                        pending.push((*child, frame - *header));
                    }
                }
                FrozenKind::Parallel(children) => {
                    pending.extend(children.iter().rev().map(|child| (*child, frame)));
                }
                FrozenKind::Tracks(tracks) => {
                    // Freeze the same numeric path used by sample(), rather
                    // than reconstructing lengths from absolute rectangles.
                    tracks.frame_extent = frame;
                    pending.extend(
                        tracks
                            .children
                            .iter()
                            .rev()
                            .map(|(child, track)| (*child, tracks.tracks[*track].extent)),
                    );
                }
            }
        }
        Some(())
    }
}

impl FrozenPiecewiseAxis {
    pub(super) fn auto_reflow_roots(
        &self,
    ) -> impl Iterator<Item = (RootId, NodeId, &Arc<FrozenRootLayout>)> {
        self.roots.iter().filter_map(|root| {
            root.auto_reflow
                .as_ref()
                .map(|layout| (root.source.root(), root.source.node(), layout))
        })
    }

    pub(in crate::engine) fn direct_grid_maps_match(&self, plan: &PresentationPlan) -> bool {
        self.roots.iter().all(|root| {
            root.drivers.keys().all(|owner| {
                let Some(FrozenNode {
                    kind: FrozenKind::Tracks(tracks),
                    ..
                }) = root.nodes.get(owner)
                else {
                    return false;
                };
                let Some((basis, _)) = &tracks.grid else {
                    return true;
                };
                plan.layout_facts()
                    .and_then(|facts| facts.root(root.source.root()))
                    .and_then(|facts| facts.projection().grids.get(owner))
                    .is_some_and(|grid| {
                        grid.shape.rows == basis.rows()
                            && grid.shape.columns == basis.columns()
                            && grid.children.as_ref() == basis.children()
                    })
            })
        })
    }

    pub(super) fn reflow_preserves_drivers(
        &self,
        root: RootId,
        projection: &LayoutProjection,
        delta: f64,
    ) -> bool {
        let Some(frozen) = self
            .roots
            .iter()
            .find(|frozen| frozen.source.root() == root)
        else {
            return true;
        };
        frozen.drivers.iter().all(|(owner, drivers)| {
            let Some(node) = frozen.nodes.get(owner) else {
                return false;
            };
            let FrozenKind::Tracks(tracks) = &node.kind else {
                return false;
            };
            let Some(projected_node) = projection.nodes.get(owner).filter(|node| node.visible)
            else {
                return false;
            };
            if let Some((basis, _)) = &tracks.grid {
                let Some(grid) = projection.grids.get(owner) else {
                    return false;
                };
                if grid.shape.rows != basis.rows()
                    || grid.shape.columns != basis.columns()
                    || grid.children.as_ref() != basis.children()
                {
                    return false;
                }
            }
            drivers.iter().all(|&index| {
                let original = rect_start(node.result.rect, self.axis)
                    + tracks.tracks[..index]
                        .iter()
                        .map(|track| track.extent + track.gap)
                        .sum::<f64>()
                    + tracks.tracks[index].extent;
                let actual = if let Some(grid) = projection.grids.get(owner) {
                    let Some(track) = grid.axis(self.axis).tracks.get(index) else {
                        return false;
                    };
                    rect_start(track.bounds, self.axis) + track.extent
                } else {
                    let Some(split) = projection.splits.get(owner) else {
                        return false;
                    };
                    if split.axis != self.axis || split.following_child_index(index).is_none() {
                        return false;
                    }
                    rect_start(projected_node.rect, self.axis)
                        + split.children[..index]
                            .iter()
                            .map(|track| {
                                track.extent
                                    + track
                                        .following_gap
                                        .map_or(0.0, |gap| rect_extent(gap, self.axis))
                            })
                            .sum::<f64>()
                        + split.children[index].extent
                };
                let expected = original + delta;
                (actual - expected).abs() <= tolerance(actual, expected)
            })
        })
    }

    pub(crate) fn sources(&self) -> impl Iterator<Item = &NodeSource> {
        self.sources.iter()
    }

    pub(super) fn inherited_grid_weights(
        &self,
        root: RootId,
        node: NodeId,
    ) -> Option<(&[SplitWeight], PaneOverflowPolicy)> {
        let frozen = self
            .roots
            .iter()
            .find(|frozen| frozen.source.root() == root)?;
        if frozen.auto_reflow.is_none() || frozen.drivers.contains_key(&node) {
            return None;
        }
        let FrozenKind::Tracks(tracks) = &frozen.nodes.get(&node)?.kind else {
            return None;
        };
        tracks.grid.as_ref()?;
        Some((&tracks.weights, self.overflow_policy))
    }

    pub(super) fn track_sample(&self, delta: f64) -> Option<(f64, Sample)> {
        self.sample_clamped(delta)
    }

    #[cfg(test)]
    pub(super) fn updates(&self, delta: f64) -> Option<Vec<TrackResize>> {
        Some(
            self.sample_clamped(delta)?
                .1
                .updates
                .into_values()
                .collect(),
        )
    }

    pub(crate) fn track_extents(&self, root: RootId, node: NodeId, delta: f64) -> Option<Vec<f64>> {
        self.sample_clamped(delta)?.1.tracks.remove(&(root, node))
    }

    fn sample_clamped(&self, requested: f64) -> Option<(f64, Sample)> {
        if !requested.is_finite() {
            return None;
        }
        let mut delta = requested;
        // Every correction lands on a concrete track capacity or frame contact.
        // A contact is visited at most once while moving monotonically to zero.
        let contacts = self
            .roots
            .iter()
            .map(|root| {
                root.nodes.len() * 2
                    + root
                        .nodes
                        .values()
                        .map(|node| match &node.kind {
                            FrozenKind::Tracks(tracks) => tracks.tracks.len() * 2,
                            _ => 0,
                        })
                        .sum::<usize>()
            })
            .sum::<usize>();
        for _ in 0..=contacts {
            match self.sample(delta) {
                Ok(sample) => return Some((delta, sample)),
                Err(SampleError::Limit(limit))
                    if limit.is_finite() && limit.abs() < delta.abs() =>
                {
                    delta = if delta > 0.0 {
                        limit.clamp(0.0, delta)
                    } else {
                        limit.clamp(delta, 0.0)
                    };
                }
                _ => return None,
            }
        }
        None
    }

    fn sample(&self, delta: f64) -> Result<Sample, SampleError> {
        let mut sample = Sample {
            updates: BTreeMap::new(),
            tracks: BTreeMap::new(),
        };
        for root in &self.roots {
            let initial = root
                .nodes
                .get(&root.source.node())
                .ok_or(SampleError::Invalid)?
                .result;
            let mut pending = vec![(
                root.source.node(),
                Scalar::fixed(rect_start(initial.slot, self.axis)),
                Scalar::fixed(rect_extent(initial.slot, self.axis)),
            )];
            while let Some((id, origin, offered)) = pending.pop() {
                let node = root.nodes.get(&id).ok_or(SampleError::Invalid)?;
                let maximum = size_extent(node.result.constraints.max(), self.axis);
                let frame = if offered.value >= maximum {
                    Scalar::fixed(maximum)
                } else {
                    offered
                };
                let minimum = size_extent(node.result.constraints.min(), self.axis);
                require_range(frame, minimum, maximum, delta)?;
                match &node.kind {
                    FrozenKind::Pane => {}
                    FrozenKind::Tabs { child, header } => {
                        if let Some(child) = child {
                            pending.push((*child, origin.add(*header), frame.add(-*header)));
                        }
                    }
                    FrozenKind::Parallel(children) => {
                        pending.extend(children.iter().rev().map(|child| (*child, origin, frame)))
                    }
                    FrozenKind::Tracks(tracks) => {
                        let drivers = root.drivers.get(&id).map_or(&[][..], Vec::as_slice);
                        let (extents, starts) = tracks.allocate(
                            origin,
                            frame,
                            rect_start(node.result.rect, self.axis),
                            drivers,
                            delta,
                            self.propagation,
                        )?;
                        let moved = extents
                            .iter()
                            .zip(&tracks.tracks)
                            .any(|(extent, track)| *extent != track.extent);
                        let key = (root.source.root(), id);
                        if (moved || (!drivers.is_empty() && tracks.grid.is_none()))
                            && let Some(update) = tracks.update(
                                self.axis,
                                &extents,
                                drivers.is_empty() && root.auto_reflow.is_none(),
                                self.overflow_policy,
                            )?
                        {
                            sample.updates.insert(key, update);
                        }
                        for &(child, track) in tracks.children.iter().rev() {
                            pending.push((
                                child,
                                starts[track],
                                Scalar {
                                    value: extents[track],
                                    slope: starts[track + 1].slope - starts[track].slope,
                                },
                            ));
                        }
                        sample.tracks.insert(key, extents);
                    }
                }
            }
        }
        Ok(sample)
    }
}

impl FrozenTracks {
    fn allocate(
        &self,
        origin: Scalar,
        frame: Scalar,
        old_origin: f64,
        drivers: &[usize],
        delta: f64,
        propagation: ResizePropagation,
    ) -> Result<(Vec<f64>, Vec<Scalar>), SampleError> {
        let visible = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| track.visible.then_some(index))
            .collect::<Vec<_>>();
        if visible.is_empty() {
            return Err(SampleError::Invalid);
        }
        let visible_share = crate::graph::normalized_weight_sum(
            self.weights
                .iter()
                .zip(&self.tracks)
                .filter_map(|(weight, track)| (track.extent > 0.0).then_some(weight.get())),
        );
        let share_floor = f64::EPSILON.max(f64::from_bits(1) / visible_share);
        let total = visible
            .iter()
            .map(|&index| self.tracks[index].extent)
            .sum::<f64>();
        // Work in displacement from each original boundary. Subtracting two
        // rounded prefixes can manufacture capacity at a saturated track.
        let frame_change = Scalar {
            value: frame.value - self.frame_extent,
            slope: frame.slope,
        };
        let mut pinned = BTreeMap::from([(0, Scalar::fixed(0.0)), (visible.len(), frame_change)]);
        let origin_movement = origin.value - old_origin;
        for &driver in drivers {
            let boundary = visible
                .iter()
                .position(|index| *index == driver)
                .ok_or(SampleError::Invalid)?
                + 1;
            if boundary == visible.len() {
                return Err(SampleError::Invalid);
            }
            pinned.insert(
                boundary,
                Scalar {
                    value: delta - origin_movement,
                    slope: 1.0 - origin.slope,
                },
            );
        }
        let pinned = pinned.into_iter().collect::<Vec<_>>();
        let mut extents = self
            .tracks
            .iter()
            .map(|track| track.extent)
            .collect::<Vec<_>>();
        let mut track_slopes = vec![0.0; self.tracks.len()];
        for endpoints in pinned.windows(2) {
            let (first, start) = endpoints[0];
            let (last, end) = endpoints[1];
            let transfer = end.difference(start);
            let from_end =
                (end.value + origin_movement).abs() >= (start.value + origin_movement).abs();
            let mut order = visible[first..last].to_vec();
            if from_end {
                order.reverse();
            }
            if propagation == ResizePropagation::Adjacent {
                order.truncate(1);
            }
            let minimum = order
                .iter()
                .map(|&index| {
                    (self.minimum(index, total, share_floor) - self.tracks[index].extent).min(0.0)
                })
                .sum::<f64>();
            let maximum = order.iter().fold(0.0, |capacity, &index| {
                (capacity + (self.tracks[index].maximum - self.tracks[index].extent).max(0.0))
                    .min(f64::MAX)
            });
            require_transfer(transfer, minimum, maximum, delta)?;
            let mut remaining = transfer.value;
            let remaining_slope = transfer.slope;
            for (position, &index) in order.iter().enumerate() {
                let track = self.tracks[index];
                let requested = track.extent + remaining;
                let bounded = if remaining == 0.0 {
                    track.extent
                } else {
                    requested.clamp(self.minimum(index, total, share_floor), track.maximum)
                };
                extents[index] = bounded;
                // An unclipped request consumes the whole transfer. Subtracting
                // the old extent can round differently from the request, losing
                // its derivative or leaking a residue into a farther track.
                // The final track can touch a hard bound after require_range
                // admitted its rounded endpoint. Keep the derivative toward
                // zero there, while retaining the bounded geometry. Earlier
                // tracks must pass even a tiny real overflow to the next one.
                if bounded == requested
                    || (position + 1 == order.len()
                        && (bounded - requested).abs() <= tolerance(bounded, requested))
                {
                    track_slopes[index] = remaining_slope;
                    break;
                }
                remaining -= bounded - track.extent;
            }
        }
        let mut starts = vec![origin; self.tracks.len() + 1];
        let mut position = origin;
        for (index, track) in self.tracks.iter().enumerate() {
            starts[index] = position;
            position = Scalar {
                value: position.value + extents[index] + track.gap,
                slope: position.slope + track_slopes[index],
            };
        }
        starts[self.tracks.len()] = position;
        Ok((extents, starts))
    }

    fn minimum(&self, index: usize, total: f64, share_floor: f64) -> f64 {
        let track = self.tracks[index];
        // Durable shares remain positive at zero-minimum contacts.
        track.minimum.max(if track.extent > 0.0 {
            track.extent.min(total * share_floor)
        } else {
            0.0
        })
    }

    fn update(
        &self,
        axis: Axis,
        extents: &[f64],
        constraints_frozen: bool,
        overflow_policy: PaneOverflowPolicy,
    ) -> Result<Option<TrackResize>, SampleError> {
        let unchanged = extents
            .iter()
            .zip(&self.tracks)
            .all(|(extent, track)| *extent == track.extent);
        let mut weights = if unchanged {
            self.weights.clone()
        } else {
            let original = self
                .tracks
                .iter()
                .map(|track| track.extent)
                .collect::<Vec<_>>();
            if self
                .tracks
                .iter()
                .any(|track| track.visible && track.extent == 0.0)
            {
                weights_with_zero_tracks(&self.weights, &self.tracks, extents, self.central)
            } else {
                super::weights_for_extents(&self.weights, &original, extents, self.central)
            }
            .ok_or(SampleError::Invalid)?
        };
        // A cap may round the projected split one ULP inside its limit.
        // If reaching that limit only rounds the stored shares, keep the
        // original bits. A small movement that changes proportions still commits.
        if !unchanged && self.rounded_contact(extents, &weights) {
            weights.clone_from(&self.weights);
        }
        Ok(Some(if let Some((basis, grid)) = &self.grid {
            // Caps break the inverse from allocated pixels to shares. For an
            // inherited resize, retain the original shares if they still
            // produce these tracks under the same constraints.
            if weights == self.weights
                || (constraints_frozen
                    && grid_shares_match_extents(
                        &self.weights,
                        extents,
                        self.tracks
                            .iter()
                            .map(|track| (track.minimum, track.maximum)),
                        self.central,
                        overflow_policy,
                    ))
            {
                return Ok(None);
            }
            let shares = match axis {
                Axis::Horizontal => {
                    DockspaceGridShares::from_normalized(grid.row_weights().to_vec(), weights)
                }
                Axis::Vertical => {
                    DockspaceGridShares::from_normalized(weights, grid.column_weights().to_vec())
                }
            }
            .map_err(|_| SampleError::Invalid)?;
            TrackResize::grid_resolved(self.source.clone(), basis.clone(), shares)
        } else {
            TrackResize::split(self.source.clone(), weights)
        }))
    }

    fn rounded_contact(&self, extents: &[f64], weights: &[SplitWeight]) -> bool {
        extents.iter().zip(&self.tracks).all(|(&extent, track)| {
            extent == track.extent
                || (extent == track.minimum && extent == track.extent.next_down())
                || (extent == track.maximum && extent == track.extent.next_up())
        }) && weights
            .iter()
            .zip(&self.weights)
            .all(|(new, old)| new.get().to_bits().abs_diff(old.get().to_bits()) <= 1)
    }
}

pub(super) fn grid_shares_match_extents(
    weights: &[SplitWeight],
    extents: &[f64],
    constraints: impl IntoIterator<Item = (f64, f64)>,
    central: Option<usize>,
    overflow_policy: PaneOverflowPolicy,
) -> bool {
    solve_grid_axis(
        extents.iter().sum(),
        weights,
        constraints,
        central,
        overflow_policy,
    )
    .is_ok_and(|solved| {
        solved
            .iter()
            .zip(extents)
            .all(|(&actual, &target)| (actual - target).abs() <= tolerance(actual, target))
    })
}

fn weights_with_zero_tracks(
    weights: &[SplitWeight],
    tracks: &[Track],
    extents: &[f64],
    central: Option<usize>,
) -> Option<Vec<SplitWeight>> {
    let total = extents.iter().sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    let visible_share = crate::graph::normalized_weight_sum(
        weights
            .iter()
            .zip(tracks)
            .filter_map(|(weight, track)| track.visible.then_some(weight.get())),
    );
    let minimum_share = (visible_share * f64::EPSILON).max(f64::from_bits(1));
    let zeros = tracks
        .iter()
        .zip(extents)
        .filter(|(track, extent)| track.visible && **extent == 0.0)
        .count();
    let positive_share = visible_share - minimum_share * zeros as f64;
    if positive_share <= 0.0 {
        return None;
    }
    let mut shares = weights
        .iter()
        .zip(tracks)
        .zip(extents)
        .map(|((weight, track), &extent)| {
            if !track.visible {
                weight.get()
            } else if extent == 0.0 {
                minimum_share
            } else {
                extent / total * positive_share
            }
        })
        .collect::<Vec<_>>();
    if let Some(central) = central.filter(|&index| tracks[index].visible) {
        let siblings = shares
            .iter()
            .zip(tracks)
            .enumerate()
            .filter_map(|(index, (&share, track))| {
                (index != central && track.visible).then_some(share)
            })
            .sum::<f64>();
        let residual = visible_share - siblings;
        if residual > 0.0 {
            shares[central] = residual;
        }
    }
    shares.into_iter().map(super::weight).collect()
}

fn require_range(value: Scalar, minimum: f64, maximum: f64, delta: f64) -> Result<(), SampleError> {
    if !value.value.is_finite() || minimum > maximum || !minimum.is_finite() || !maximum.is_finite()
    {
        return Err(SampleError::Invalid);
    }
    let endpoint = if value.value < minimum - tolerance(value.value, minimum) {
        minimum
    } else if value.value > maximum + tolerance(value.value, maximum) {
        maximum
    } else {
        return Ok(());
    };
    limit_at(value, endpoint, delta)
}

fn require_transfer(
    transfer: Scalar,
    minimum: f64,
    maximum: f64,
    delta: f64,
) -> Result<(), SampleError> {
    // The frozen tracks have no capacity in this direction. Rounding is
    // permitted at a reachable endpoint, but it cannot create travel here.
    if (minimum == 0.0 && transfer.value < 0.0) || (maximum == 0.0 && transfer.value > 0.0) {
        return limit_at(transfer, 0.0, delta);
    }
    require_range(transfer, minimum, maximum, delta)
}

fn limit_at(value: Scalar, endpoint: f64, delta: f64) -> Result<(), SampleError> {
    if value.slope == 0.0 {
        return Err(SampleError::Invalid);
    }
    let intercept = (-value.slope).mul_add(delta, value.value);
    Err(SampleError::Limit((endpoint - intercept) / value.slope))
}

fn rect_start(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => rect.x(),
        Axis::Vertical => rect.y(),
    }
}

#[cfg(test)]
mod tests;
