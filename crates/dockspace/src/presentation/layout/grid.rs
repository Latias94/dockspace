//! Shared two-dimensional tracks, resolved once before exact allocation.

use super::*;
use crate::model::DockspaceGridShape;

/// Dimensions frozen for one exact Grid projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedGridShape {
    /// Number of shared tracks from top to bottom.
    pub rows: usize,
    /// Number of shared tracks from left to right.
    pub columns: usize,
}

/// One shared track and the structural gap following it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridTrackProjection {
    /// Largest active incident child minimum along this axis.
    pub minimum: f64,
    /// Envelope of active incident child frame maxima.
    pub maximum: f64,
    /// Whether the frame supplies the track maximum because it has no visible cell.
    pub(crate) maximum_follows_frame: bool,
    /// Shared slot extent allocated by the bounded axis solver.
    pub extent: f64,
    /// Full owner strip allocated to this track.
    pub bounds: LogicalRect,
    /// Structural gap after this track, including a collapsed zero-area gap.
    pub following_gap: Option<LogicalRect>,
}

/// All numeric results on one shared Grid axis.
#[derive(Debug, Clone, PartialEq)]
pub struct GridAxisProjection {
    /// Shared slots in persistent track order.
    pub tracks: Vec<GridTrackProjection>,
    /// Exact effective normalized shares used for this allocation.
    pub weights: Vec<SplitWeight>,
    /// Track containing the preferred central anchor, when applicable.
    pub central_index: Option<usize>,
    /// Mandatory minima exceeding the available track extent.
    pub overflow: f64,
    /// Extent left unused after all tracks reached their maxima.
    pub unallocated: f64,
}

/// Frozen dimensions, actual cell owners, and both shared axes.
#[derive(Debug, Clone, PartialEq)]
pub struct GridProjection {
    /// Actual graph owner of both axes.
    pub node: NodeId,
    /// Dimensions frozen before exact postorder measurement.
    pub shape: ResolvedGridShape,
    /// Actual row-major roster, including hidden children and excluding unused tail cells.
    pub children: Arc<[NodeId]>,
    /// Shared top-to-bottom allocation.
    pub rows: GridAxisProjection,
    /// Shared left-to-right allocation.
    pub columns: GridAxisProjection,
}

impl GridProjection {
    /// Returns a real child at this frozen cell, or None for an unused tail cell.
    #[cfg(test)]
    #[must_use]
    pub fn cell_child(&self, row: usize, column: usize) -> Option<NodeId> {
        if row >= self.shape.rows || column >= self.shape.columns {
            return None;
        }
        self.children
            .get(row * self.shape.columns + column)
            .copied()
    }

    /// Borrows columns for Horizontal or rows for Vertical.
    #[must_use]
    pub fn axis(&self, axis: Axis) -> &GridAxisProjection {
        match axis {
            Axis::Horizontal => &self.columns,
            Axis::Vertical => &self.rows,
        }
    }
}

/// Root-local Grid facts; ordinary roots allocate no Grid entries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridMap(BTreeMap<NodeId, GridProjection>);

impl GridMap {
    /// Returns the facts for this exact real Grid identity.
    #[must_use]
    pub fn get(&self, node: &NodeId) -> Option<&GridProjection> {
        self.0.get(node)
    }
    /// Visits Grid owners in complete generational identity order.
    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &GridProjection)> {
        self.0.iter()
    }
}

pub(crate) fn resolve_grid_shape(
    shape: DockspaceGridShape,
    count: usize,
    offered: LogicalSize,
    gap: f64,
) -> ResolvedGridShape {
    if let DockspaceGridShape::Fixed { rows, columns } = shape {
        return ResolvedGridShape {
            rows: rows.get(),
            columns: columns.get(),
        };
    }
    let count = count.max(1);
    let mut best = None;
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        if columns != count.div_ceil(rows) {
            continue;
        }
        let width = (offered.width() - gap * columns.saturating_sub(1) as f64).max(0.0);
        let height = (offered.height() - gap * rows.saturating_sub(1) as f64).max(0.0);
        let positive = width > 0.0 && height > 0.0;
        let (width, height) = if positive {
            (width, height)
        } else if offered.width() > 0.0 && offered.height() > 0.0 {
            (offered.width(), offered.height())
        } else {
            (1.0, 1.0)
        };
        let scale = width.max(height);
        let (w, h) = (
            (width / scale) / columns as f64,
            (height / scale) / rows as f64,
        );
        let distortion = (w - h).abs() / (w + h);
        let empty = rows * columns - count;
        let orientation = if offered.width() >= offered.height() {
            columns
        } else {
            rows
        };
        let key = (!positive, distortion, empty, usize::MAX - orientation);
        let better = best.as_ref().is_none_or(
            |(old, _): &((bool, f64, usize, usize), ResolvedGridShape)| {
                key.0
                    .cmp(&old.0)
                    .then_with(|| key.1.total_cmp(&old.1))
                    .then_with(|| key.2.cmp(&old.2))
                    .then_with(|| key.3.cmp(&old.3))
                    .is_lt()
            },
        );
        if better {
            best = Some((key, ResolvedGridShape { rows, columns }));
        }
    }
    best.expect("a nonempty roster has a tight one-row candidate")
        .1
}

fn track_constraints(
    child_rows: &[usize],
    values: &[NodeResult],
    shape: ResolvedGridShape,
    axis: Axis,
    neutral_maximum: f64,
) -> (Vec<AxisConstraint>, Vec<bool>) {
    let count = match axis {
        Axis::Horizontal => shape.columns,
        Axis::Vertical => shape.rows,
    };
    let mut tracks = vec![AxisConstraint { min: 0.0, max: 0.0 }; count];
    let mut occupied = vec![false; count];
    for (index, &row) in child_rows.iter().enumerate() {
        let result = &values[row];
        if !result.visible {
            continue;
        }
        let track = match axis {
            Axis::Horizontal => index % shape.columns,
            Axis::Vertical => index / shape.columns,
        };
        let constraints = result.constraints;
        let (minimum, maximum) = match axis {
            Axis::Horizontal => (constraints.min().width(), constraints.max().width()),
            Axis::Vertical => (constraints.min().height(), constraints.max().height()),
        };
        tracks[track].min = tracks[track].min.max(minimum);
        tracks[track].max = tracks[track].max.max(maximum);
        occupied[track] = true;
    }
    for (track, &occupied) in tracks.iter_mut().zip(&occupied) {
        if !occupied {
            track.max = neutral_maximum;
        }
    }
    (tracks, occupied)
}

pub(super) fn grid_constraints(
    topology: &RootStructure,
    row: usize,
    values: &[NodeResult],
    shape: ResolvedGridShape,
    metrics: LayoutMetrics,
    neutral: LogicalSize,
) -> Result<Constraints, LayoutError> {
    let RootNodeKind::Grid { children, .. } = &topology.nodes[row].kind else {
        unreachable!()
    };
    let children = &topology.children[children.clone()];
    let (columns, _) =
        track_constraints(children, values, shape, Axis::Horizontal, neutral.width());
    let (rows, _) = track_constraints(children, values, shape, Axis::Vertical, neutral.height());
    let sum = |tracks: &[AxisConstraint], maximum: bool| {
        checked_sum(
            tracks
                .iter()
                .map(|track| if maximum { track.max } else { track.min })
                .chain(std::iter::once(
                    metrics.splitter_thickness() * tracks.len().saturating_sub(1) as f64,
                )),
            "grid tracks",
        )
    };
    Ok(Constraints::new(
        LogicalSize::new(sum(&columns, false)?, sum(&rows, false)?)?,
        LogicalSize::new(sum(&columns, true)?, sum(&rows, true)?)?,
    )?)
}

pub(super) fn remeasure_frozen_grids(
    topology: &RootStructure,
    values: &mut [NodeResult],
    shapes: &BTreeMap<NodeId, ResolvedGridShape>,
    metrics: LayoutMetrics,
    offered: LogicalSize,
    tabs: &BTreeMap<usize, (Option<usize>, f64)>,
) -> Result<(), LayoutError> {
    for &row in &topology.postorder {
        if !values[row].visible {
            continue;
        }
        let constraints = match &topology.nodes[row].kind {
            RootNodeKind::Pane { .. } => continue,
            RootNodeKind::Tabs { .. } => {
                let Some((Some(child), inset)) = tabs.get(&row) else {
                    continue;
                };
                let child = values[*child].constraints;
                Constraints::new(
                    LogicalSize::new(child.min().width(), child.min().height() + inset)?,
                    LogicalSize::new(child.max().width(), child.max().height() + inset)?,
                )?
            }
            RootNodeKind::Split {
                payload, children, ..
            } => {
                let children = topology.children[children.clone()]
                    .iter()
                    .filter_map(|&child| {
                        values[child].visible.then_some(SubtreeMeasurement {
                            constraints: values[child].constraints,
                            contains_central: false,
                        })
                    })
                    .collect::<Vec<_>>();
                let gaps = total_splitter_extent(topology.nodes[row].id, children.len(), metrics)?;
                Constraints::new(
                    compose_minimum(payload.axis(), &children, gaps)?,
                    compose_frame_maximum(payload.axis(), &children, gaps, [true, true])?,
                )?
            }
            RootNodeKind::Grid { .. } => grid_constraints(
                topology,
                row,
                values,
                shapes[&topology.nodes[row].id],
                metrics,
                offered,
            )?,
        };
        values[row].constraints = constraints;
    }
    Ok(())
}

pub(super) fn prepare_grid_projection(
    topology: &RootStructure,
    row: usize,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
    central: &mut CentralRoute,
    overrides: &BTreeMap<NodeId, TrackWeights>,
    shapes: Option<&BTreeMap<NodeId, ResolvedGridShape>>,
    projection: &mut LayoutProjection,
) -> Result<Vec<(usize, LogicalRect)>, LayoutError> {
    let node = &topology.nodes[row];
    let RootNodeKind::Grid {
        payload, children, ..
    } = &node.kind
    else {
        unreachable!()
    };
    let child_rows = &topology.children[children.clone()];
    let shape = shapes
        .and_then(|shapes| shapes.get(&node.id))
        .copied()
        .unwrap_or_else(|| {
            resolve_grid_shape(
                payload.shape(),
                child_rows.len(),
                bounds.size(),
                metrics.splitter_thickness(),
            )
        });
    let shares = overrides
        .get(&node.id)
        .and_then(|weights| match weights {
            TrackWeights::Grid(shares) => Some(shares),
            _ => None,
        })
        .or_else(|| payload.shares());
    let matched = shares.filter(|shares| {
        shares.rows().len() == shape.rows && shares.columns().len() == shape.columns
    });
    if overrides.contains_key(&node.id)
        && matched.is_none()
        && matches!(payload.shape(), DockspaceGridShape::Fixed { .. })
    {
        return Err(LayoutError::TrackWeightOverrideLengthMismatch {
            node: node.id,
            children: shape.rows * shape.columns,
            weights: shares.map_or(0, |shares| shares.rows().len() * shares.columns().len()),
        });
    }
    let central_child = match central {
        CentralRoute::Tree { leaf, next } if *next == row => {
            let index = child_rows.iter().rposition(|&child| child <= *leaf);
            if let Some(index) = index {
                *next = child_rows[index];
            }
            index
        }
        _ => None,
    };
    let solve = |axis, weights: Vec<SplitWeight>| -> Result<GridAxisProjection, LayoutError> {
        let count = weights.len();
        let extent = match axis {
            Axis::Horizontal => bounds.width(),
            Axis::Vertical => bounds.height(),
        };
        let (gaps, thickness) = projected_splitter_geometry(node.id, count, extent, metrics)?;
        let (constraints, occupied) = track_constraints(
            child_rows,
            &projection.nodes.values,
            shape,
            axis,
            extent.max(1.0),
        );
        let central_index = central_child.map(|index| match axis {
            Axis::Horizontal => index % shape.columns,
            Axis::Vertical => index / shape.columns,
        });
        let mut sizes = vec![0.0; count];
        let outcome = solve_axis_into(
            extent - gaps,
            &weights
                .iter()
                .map(|weight| weight.get())
                .collect::<Vec<_>>(),
            &constraints,
            central_index,
            &mut sizes,
            &mut Vec::new(),
            metrics.pane_overflow_policy,
        )?;
        let mut cursor = match axis {
            Axis::Horizontal => bounds.x(),
            Axis::Vertical => bounds.y(),
        };
        let mut tracks = Vec::with_capacity(count);
        for (index, ((&size, constraint), occupied)) in
            sizes.iter().zip(&constraints).zip(occupied).enumerate()
        {
            let rect = match axis {
                Axis::Horizontal => LogicalRect::new(cursor, bounds.y(), size, bounds.height())?,
                Axis::Vertical => LogicalRect::new(bounds.x(), cursor, bounds.width(), size)?,
            };
            cursor += size;
            let following_gap = if index + 1 < count {
                let gap = match axis {
                    Axis::Horizontal => {
                        LogicalRect::new(cursor, bounds.y(), thickness, bounds.height())?
                    }
                    Axis::Vertical => {
                        LogicalRect::new(bounds.x(), cursor, bounds.width(), thickness)?
                    }
                };
                cursor += thickness;
                Some(gap)
            } else {
                None
            };
            tracks.push(GridTrackProjection {
                minimum: constraint.min,
                maximum: constraint.max,
                maximum_follows_frame: !occupied,
                extent: size,
                bounds: rect,
                following_gap,
            });
        }
        Ok(GridAxisProjection {
            tracks,
            weights,
            central_index,
            overflow: outcome.overflow,
            unallocated: outcome.unallocated,
        })
    };
    let columns = solve(
        Axis::Horizontal,
        matched.map_or_else(
            || {
                SplitWeight::normalize(vec![1.0; shape.columns])
                    .expect("a nonzero equal track count has valid shares")
            },
            |shares| shares.column_weights().to_vec(),
        ),
    )?;
    let rows = solve(
        Axis::Vertical,
        matched.map_or_else(
            || {
                SplitWeight::normalize(vec![1.0; shape.rows])
                    .expect("a nonzero equal track count has valid shares")
            },
            |shares| shares.row_weights().to_vec(),
        ),
    )?;
    let mut placements = Vec::with_capacity(child_rows.len());
    for (index, &child) in child_rows.iter().enumerate() {
        let column = &columns.tracks[index % shape.columns];
        let row = &rows.tracks[index / shape.columns];
        let bounds = if projection.nodes.at_row(child).visible {
            LogicalRect::from_min_max(
                crate::geometry::LogicalPoint::new(column.bounds.x(), row.bounds.y())?,
                crate::geometry::LogicalPoint::new(column.bounds.max().x(), row.bounds.max().y())?,
            )?
        } else {
            LogicalRect::new(column.bounds.x(), row.bounds.y(), 0.0, 0.0)?
        };
        placements.push((child, bounds));
    }
    projection.grids.0.insert(
        node.id,
        GridProjection {
            node: node.id,
            shape,
            children: child_rows
                .iter()
                .map(|&row| topology.nodes[row].id)
                .collect(),
            rows,
            columns,
        },
    );
    Ok(placements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::AxisSizing;
    use crate::graph::{RootRecord, SurfacePresentation};
    use crate::ids::{ItemId, SurfaceId};
    use crate::model::DockspaceGridShares;
    use std::num::NonZeroUsize;

    const ROOT: RootId = RootId::new(8001);

    fn fixed(rows: usize, columns: usize) -> DockspaceGridShape {
        DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(rows).unwrap(),
            columns: NonZeroUsize::new(columns).unwrap(),
        }
    }

    fn fixture(
        count: usize,
        shape: DockspaceGridShape,
        shares: Option<DockspaceGridShares>,
        hidden: &[usize],
    ) -> (Workspace, NodeId, Vec<NodeId>) {
        let mut builder = Workspace::builder();
        let children = (0..count)
            .map(|index| builder.insert_node(Node::pane(ItemId::new(index as u64 + 1))))
            .collect::<Vec<_>>();
        let grid = builder.insert_node(Node::Grid {
            shape,
            children: children.clone(),
            shares,
            layout_title: None,
        });
        builder.set_root(ROOT, RootRecord::new(grid));
        builder.set_surface(SurfaceId::new(8002), SurfacePresentation::with_main(ROOT));
        builder.set_hidden_items(
            hidden
                .iter()
                .map(|&index| ItemId::new(index as u64 + 1))
                .collect(),
        );
        (builder.build().unwrap(), grid, children)
    }

    fn project(
        workspace: &Workspace,
        children: &[NodeId],
        bounds: LogicalRect,
        sizing: &BTreeMap<NodeId, PaneSizing>,
        policy: PaneOverflowPolicy,
    ) -> LayoutProjection {
        let sizing = children
            .iter()
            .copied()
            .map(|node| (node, sizing.get(&node).copied().unwrap_or_default()))
            .collect();
        measure_root_sizing(
            workspace,
            ROOT,
            &sizing,
            LayoutMetrics::new(4.0)
                .unwrap()
                .with_pane_overflow_policy(policy),
        )
        .unwrap()
        .project(bounds, &[])
        .unwrap()
    }

    #[test]
    fn fixed_tracks_allocate_four_exact_cells_and_both_axes_share_resizes() {
        for (columns, rows, widths, heights) in [
            ([1.0, 1.0], [1.0, 1.0], [200.0, 200.0], [100.0, 100.0]),
            ([220.0, 180.0], [110.0, 90.0], [220.0, 180.0], [110.0, 90.0]),
        ] {
            let (workspace, grid, children) = fixture(
                4,
                fixed(2, 2),
                Some(DockspaceGridShares::new(rows, columns).unwrap()),
                &[],
            );
            let result = project(
                &workspace,
                &children,
                LogicalRect::new(11.0, 17.0, 404.0, 204.0).unwrap(),
                &BTreeMap::new(),
                PaneOverflowPolicy::Clip,
            );
            for (cell, child) in children.iter().enumerate() {
                let column = cell % 2;
                let row = cell / 2;
                let expected = LogicalRect::new(
                    11.0 + if column == 0 { 0.0 } else { widths[0] + 4.0 },
                    17.0 + if row == 0 { 0.0 } else { heights[0] + 4.0 },
                    widths[column],
                    heights[row],
                )
                .unwrap();
                let actual = result.nodes[child].slot;
                for (actual, expected) in [
                    (actual.x(), expected.x()),
                    (actual.y(), expected.y()),
                    (actual.width(), expected.width()),
                    (actual.height(), expected.height()),
                ] {
                    assert!((actual - expected).abs() < 1.0e-10);
                }
                assert_eq!(actual.x(), result.nodes[&children[column]].slot.x());
                assert_eq!(actual.width(), result.nodes[&children[column]].slot.width());
                assert_eq!(actual.y(), result.nodes[&children[row * 2]].slot.y());
                assert_eq!(
                    actual.height(),
                    result.nodes[&children[row * 2]].slot.height()
                );
            }
            let grid = result.grids.get(&grid).unwrap();
            assert_eq!(grid.columns.tracks.len(), 2);
            assert_eq!(grid.rows.tracks.len(), 2);
            assert_eq!(
                grid.columns.tracks[0].following_gap.unwrap().height(),
                204.0
            );
            assert_eq!(grid.rows.tracks[0].following_gap.unwrap().width(), 404.0);
        }
    }

    #[test]
    fn sparse_tail_and_hidden_cells_preserve_shared_slot_indices() {
        for (count, hidden) in [(3, vec![]), (4, vec![0]), (4, vec![0, 1])] {
            let (workspace, grid, children) = fixture(count, fixed(2, 2), None, &hidden);
            let result = project(
                &workspace,
                &children,
                LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
                &BTreeMap::new(),
                PaneOverflowPolicy::Clip,
            );
            let facts = result.grids.get(&grid).unwrap();
            assert_eq!(
                facts.shape,
                ResolvedGridShape {
                    rows: 2,
                    columns: 2
                }
            );
            assert_eq!(facts.cell_child(1, 0), Some(children[2]));
            assert_eq!(facts.cell_child(1, 1), children.get(3).copied());
            for (cell, child) in children.iter().enumerate() {
                if hidden.contains(&cell) {
                    assert!(!result.nodes[child].visible);
                    assert_eq!(result.nodes[child].slot.width(), 0.0);
                } else {
                    assert_eq!(
                        result.nodes[child].slot,
                        LogicalRect::new(
                            if cell % 2 == 0 { 0.0 } else { 204.0 },
                            if cell / 2 == 0 { 0.0 } else { 104.0 },
                            200.0,
                            100.0
                        )
                        .unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn a_capped_cell_does_not_cap_an_uncapped_peer_on_the_same_column() {
        let (workspace, _, children) = fixture(4, fixed(2, 2), None, &[]);
        let capped = PaneSizing::new(
            AxisSizing::new(10.0, Some(80.0), None).unwrap(),
            AxisSizing::default(),
        );
        let result = project(
            &workspace,
            &children,
            LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
            &BTreeMap::from([(children[0], capped)]),
            PaneOverflowPolicy::Clip,
        );
        assert_eq!(result.nodes[&children[0]].rect.width(), 80.0);
        assert_eq!(result.nodes[&children[0]].slot.width(), 200.0);
        assert_eq!(result.nodes[&children[2]].rect.width(), 200.0);
    }

    #[test]
    fn fully_capped_tracks_redistribute_to_uncapped_columns() {
        let (workspace, grid, children) = fixture(4, fixed(2, 2), None, &[]);
        let capped = PaneSizing::new(
            AxisSizing::new(10.0, Some(80.0), None).unwrap(),
            AxisSizing::default(),
        );
        let result = project(
            &workspace,
            &children,
            LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
            &BTreeMap::from([(children[0], capped), (children[2], capped)]),
            PaneOverflowPolicy::Clip,
        );
        let grid = result.grids.get(&grid).unwrap();
        assert_eq!(grid.columns.tracks[0].extent, 80.0);
        assert_eq!(grid.columns.tracks[1].extent, 320.0);
    }

    #[test]
    fn insufficient_extent_uses_the_same_clip_and_compress_policies() {
        let (workspace, grid, children) = fixture(4, fixed(2, 2), None, &[]);
        let minimum = PaneSizing::from_minimum(LogicalSize::new(120.0, 80.0).unwrap());
        let sizing = children
            .iter()
            .copied()
            .map(|child| (child, minimum))
            .collect();
        for (policy, width, height) in [
            (PaneOverflowPolicy::Clip, 120.0, 80.0),
            (PaneOverflowPolicy::Compress, 98.0, 48.0),
        ] {
            let result = project(
                &workspace,
                &children,
                LogicalRect::new(0.0, 0.0, 200.0, 100.0).unwrap(),
                &sizing,
                policy,
            );
            let grid = result.grids.get(&grid).unwrap();
            assert_eq!(grid.columns.tracks[0].extent, width);
            assert_eq!(grid.rows.tracks[0].extent, height);
            assert_eq!(grid.columns.overflow, 44.0);
            assert_eq!(grid.rows.overflow, 64.0);
        }
    }

    #[test]
    fn zero_and_gap_only_frames_stay_total_and_keep_structural_gaps() {
        let (workspace, grid, children) = fixture(4, fixed(2, 2), None, &[]);
        for (width, height) in [(0.0, 0.0), (2.0, 1.0), (4.0, 4.0)] {
            let result = project(
                &workspace,
                &children,
                LogicalRect::new(0.0, 0.0, width, height).unwrap(),
                &BTreeMap::new(),
                PaneOverflowPolicy::Compress,
            );
            let grid = result.grids.get(&grid).unwrap();
            assert_eq!(grid.columns.tracks[0].extent, 0.0);
            assert_eq!(grid.rows.tracks[0].extent, 0.0);
            assert_eq!(grid.columns.tracks[0].following_gap.unwrap().width(), width);
            assert_eq!(grid.rows.tracks[0].following_gap.unwrap().height(), height);
        }
    }

    #[test]
    fn auto_resolution_is_deterministic_transposes_and_ignores_hidden_members() {
        let landscape = LogicalSize::new(404.0, 204.0).unwrap();
        assert_eq!(
            resolve_grid_shape(DockspaceGridShape::Auto, 5, landscape, 4.0),
            ResolvedGridShape {
                rows: 2,
                columns: 3
            }
        );
        assert_eq!(
            resolve_grid_shape(
                DockspaceGridShape::Auto,
                5,
                LogicalSize::new(204.0, 404.0).unwrap(),
                4.0
            ),
            ResolvedGridShape {
                rows: 3,
                columns: 2
            }
        );
        let shares = DockspaceGridShares::new([1.0], [3.0, 2.0, 1.0, 1.0, 1.0]).unwrap();
        let (workspace, grid, children) = fixture(5, DockspaceGridShape::Auto, Some(shares), &[0]);
        let result = project(
            &workspace,
            &children,
            LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
            &BTreeMap::new(),
            PaneOverflowPolicy::Clip,
        );
        let grid = result.grids.get(&grid).unwrap();
        assert_eq!(
            grid.shape,
            ResolvedGridShape {
                rows: 2,
                columns: 3
            }
        );
        assert_eq!(
            grid.columns.weights,
            SplitWeight::normalize([1.0, 1.0, 1.0]).unwrap()
        );
        assert_eq!(
            grid.rows.weights,
            SplitWeight::normalize([1.0, 1.0]).unwrap()
        );
    }

    #[test]
    fn nested_auto_freezes_the_provisional_offered_shape_before_exact_minimums() {
        let mut builder = Workspace::builder();
        let children = (1..=4)
            .map(|item| builder.insert_node(Node::pane(ItemId::new(item))))
            .collect::<Vec<_>>();
        let grid = builder.insert_node(Node::Grid {
            shape: DockspaceGridShape::Auto,
            children: children.clone(),
            shares: None,
            layout_title: None,
        });
        let sibling = builder.insert_node(Node::pane(ItemId::new(5)));
        let split =
            builder.insert_node(Node::equal_split(Axis::Horizontal, [grid, sibling]).unwrap());
        builder.set_root(ROOT, RootRecord::new(split));
        builder.set_surface(SurfaceId::new(8002), SurfacePresentation::with_main(ROOT));
        let workspace = builder.build().unwrap();
        let sizing = children
            .iter()
            .map(|&child| {
                (
                    child,
                    PaneSizing::builder().min_width(300.0).build().unwrap(),
                )
            })
            .chain([
                (
                    grid,
                    PaneSizing::builder().min_width(300.0).build().unwrap(),
                ),
                (sibling, PaneSizing::default()),
            ])
            .collect();
        let result =
            measure_root_sizing(&workspace, ROOT, &sizing, LayoutMetrics::new(4.0).unwrap())
                .unwrap()
                .project(LogicalRect::new(0.0, 0.0, 804.0, 204.0).unwrap(), &[])
                .unwrap();
        let shape = result.grids.get(&grid).unwrap().shape;
        assert_eq!(
            shape,
            ResolvedGridShape {
                rows: 2,
                columns: 2
            }
        );
        assert_eq!(result.nodes[&grid].rect.width(), 604.0);
        assert_ne!(
            shape,
            resolve_grid_shape(
                DockspaceGridShape::Auto,
                4,
                result.nodes[&grid].rect.size(),
                4.0
            ),
            "final allocation must not feed shape resolution again"
        );
        assert_eq!(result.nodes[&children[0]].slot.width(), 300.0);
        assert_eq!(result.nodes[&children[2]].slot.width(), 300.0);
    }

    #[test]
    fn both_auto_share_axes_survive_only_when_both_dimensions_match() {
        let shares = DockspaceGridShares::new([3.0, 1.0], [3.0, 2.0, 1.0]).unwrap();
        let (workspace, grid, children) =
            fixture(5, DockspaceGridShape::Auto, Some(shares.clone()), &[]);
        let result = project(
            &workspace,
            &children,
            LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
            &BTreeMap::new(),
            PaneOverflowPolicy::Clip,
        );
        let facts = result.grids.get(&grid).unwrap();
        assert_eq!(facts.rows.weights, shares.row_weights());
        assert_eq!(facts.columns.weights, shares.column_weights());
        let transposed = project(
            &workspace,
            &children,
            LogicalRect::new(0.0, 0.0, 204.0, 404.0).unwrap(),
            &BTreeMap::new(),
            PaneOverflowPolicy::Clip,
        );
        let facts = transposed.grids.get(&grid).unwrap();
        assert_eq!(
            facts.shape,
            ResolvedGridShape {
                rows: 3,
                columns: 2
            }
        );
        assert_eq!(
            facts.rows.weights,
            SplitWeight::normalize([1.0, 1.0, 1.0]).unwrap()
        );
        assert_eq!(
            facts.columns.weights,
            SplitWeight::normalize([1.0, 1.0]).unwrap()
        );
    }

    #[test]
    fn auto_scores_remain_total_at_subnormal_and_collapsed_bounds() {
        for size in [
            LogicalSize::new(0.0, 0.0).unwrap(),
            LogicalSize::new(f64::from_bits(1), f64::from_bits(1)).unwrap(),
            LogicalSize::new(f64::MAX, 1.0).unwrap(),
        ] {
            let first = resolve_grid_shape(DockspaceGridShape::Auto, 5, size, 4.0);
            assert_eq!(
                first,
                resolve_grid_shape(DockspaceGridShape::Auto, 5, size, 4.0)
            );
            assert!(first.rows * first.columns >= 5);
            assert_eq!(first.rows, 5_usize.div_ceil(first.columns));
            assert_eq!(first.columns, 5_usize.div_ceil(first.rows));
        }
    }
}
