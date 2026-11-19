//! Shared resize boundary geometry for presentation and ordered adjustments.

use super::*;
use crate::graph::command::TrackWeights;
use crate::graph::{Axis, SplitWeight};
use crate::presentation::layout::{ChildProjection, LayoutProjection, ResizeGeometryFacts};
use crate::presentation::scene::{SplitterOwner, SplitterResizeTarget};

pub(super) struct RootBoundaries<'a> {
    pub(super) policy: &'a DockPolicySnapshot,
    pub(super) config: &'a DockPresentationConfig,
    pub(super) surface: SurfaceId,
    pub(super) root: RootId,
    pub(super) layer: SceneLayerKey,
    pub(super) content_clip: LogicalRect,
    pub(super) projection: &'a LayoutProjection,
    pub(super) contacts: &'a ResizeGeometryFacts,
}

impl RootBoundaries<'_> {
    pub(super) fn emit(
        &self,
        node: NodeId,
        owner: &Node,
        clip: LogicalRect,
        tracks: Option<&TrackWeights>,
        mut publish: impl FnMut(
            SplitterGapRecord,
            Option<SplitterRecord>,
        ) -> Result<(), SceneCompilationError>,
    ) -> Result<(), SceneCompilationError> {
        let root = self.root;
        let projection = self.projection;
        let missing = || SceneCompilationError::MissingSplitProjection { root, split: node };
        match owner {
            Node::Split {
                axis,
                children,
                weights,
                ..
            } => {
                let axis = *axis;
                let split = projection.splits.get(&node).ok_or_else(missing)?;
                let hits = splitter_hit_rects(
                    split.children,
                    axis,
                    self.config.splitter_hit_extent(),
                    clip,
                )?;
                let operable = self
                    .policy
                    .splitter_resize_is_allowed(root, axis, self.surface)
                    && !(self.config.pane_overflow_policy()
                        == crate::presentation::config::PaneOverflowPolicy::Compress
                        && split.overflow > 0.0);
                let mut resize_facts = None;
                for ((index, draw), hit) in split.indexed_splitter_rects().zip(hits) {
                    let id = SplitterSceneId {
                        root,
                        owner: SplitterOwner::Split(node),
                        index,
                    };
                    let draw = intersect_rect(draw, clip)?;
                    let gap = gap_record(id, draw);
                    let Some(draw) = draw else {
                        publish(gap, None)?;
                        continue;
                    };
                    let after_index = split.following_child_index(index).ok_or_else(missing)?;
                    let before = children[index];
                    let after = children[after_index];
                    let bounds = |node| {
                        projection
                            .nodes
                            .get(&node)
                            .ok_or(SceneCompilationError::MissingNodeProjection { root, node })
                            .and_then(|result| Ok(clip_rect(result.rect, self.content_clip)?))
                    };
                    let (extents, weights) = resize_facts.get_or_insert_with(|| {
                        let effective = match tracks {
                            Some(TrackWeights::Split(weights)) => weights,
                            _ => weights,
                        };
                        (
                            split
                                .children
                                .iter()
                                .map(|child| child.extent)
                                .collect::<Arc<[f64]>>(),
                            Arc::<[SplitWeight]>::from(effective.as_slice()),
                        )
                    });
                    let a = &split.children[index];
                    let b = &split.children[after_index];
                    let record = SplitterRecord::new(
                        id,
                        draw,
                        HitRegion::new(hit),
                        axis,
                        bounds(before)?,
                        bounds(after)?,
                        a.minimum,
                        b.minimum,
                        a.maximum,
                        b.maximum,
                        Arc::clone(extents),
                        split.central_index,
                        Arc::clone(weights),
                        self.layer,
                        operable && self.contacts.splitter_is_operable(node, before, after),
                    )
                    .with_after_index(after_index);
                    publish(gap, Some(record))?;
                }
            }
            Node::Grid { .. } => {
                let grid = projection.grids.get(&node).ok_or_else(missing)?;
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let projection = grid.axis(axis);
                    let children = projection
                        .tracks
                        .iter()
                        .map(|track| ChildProjection {
                            visible: true,
                            minimum: track.minimum,
                            maximum: track.maximum,
                            extent: track.extent,
                            following_gap: track.following_gap,
                        })
                        .collect::<Vec<_>>();
                    let hits = splitter_hit_rects(
                        &children,
                        axis,
                        self.config.splitter_hit_extent(),
                        clip,
                    )?;
                    let extents = projection
                        .tracks
                        .iter()
                        .map(|track| track.extent)
                        .collect::<Arc<[f64]>>();
                    let weights = Arc::<[SplitWeight]>::from(projection.weights.as_slice());
                    let operable = self
                        .policy
                        .splitter_resize_is_allowed(root, axis, self.surface)
                        && projection.overflow == 0.0;
                    for (index, hit) in hits.into_iter().enumerate() {
                        let before = &projection.tracks[index];
                        let after = &projection.tracks[index + 1];
                        let id = SplitterSceneId::grid(root, node, axis, index);
                        let draw = before
                            .following_gap
                            .map(|gap| intersect_rect(gap, clip))
                            .transpose()?
                            .flatten();
                        let gap = gap_record(id, draw);
                        let Some(draw) = draw else {
                            publish(gap, None)?;
                            continue;
                        };
                        let complete = projection.unallocated
                            <= f64::EPSILON * 64.0 * before.extent.max(after.extent).max(1.0);
                        let record = SplitterRecord::new(
                            id,
                            draw,
                            HitRegion::new(hit),
                            axis,
                            clip_rect(before.bounds, clip)?,
                            clip_rect(after.bounds, clip)?,
                            before.minimum,
                            after.minimum,
                            before.maximum,
                            after.maximum,
                            Arc::clone(&extents),
                            projection.central_index,
                            Arc::clone(&weights),
                            self.layer,
                            operable
                                && complete
                                && self.contacts.grid_splitter_is_operable(grid, axis, index),
                        )
                        .with_grid_proof(
                            (grid.shape.rows, grid.shape.columns),
                            Arc::clone(&grid.children),
                        );
                        publish(gap, Some(record))?;
                    }
                }
            }
            Node::Pane { .. } | Node::Tabs { .. } => {}
        }
        Ok(())
    }
}

fn gap_record(id: SplitterSceneId, draw: Option<LogicalRect>) -> SplitterGapRecord {
    SplitterGapRecord::new(
        id,
        if draw.is_some() {
            SplitterGapPresentation::Rendered
        } else {
            SplitterGapPresentation::Collapsed
        },
    )
}

/// Recomputes the exact captured boundary or junction in the staged geometry.
/// A consumed prefix can collapse a boundary or change its junction's arms.
pub(crate) fn reprojected_resize_handles(
    workspace: &Workspace,
    plan: &PresentationPlan,
    target: SplitterResizeTarget,
    projection: &LayoutProjection,
    weights: &BTreeMap<NodeId, TrackWeights>,
    policy: &DockPolicySnapshot,
) -> Result<Option<Vec<SplitterRecord>>, SceneCompilationError> {
    let original = match target {
        SplitterResizeTarget::Handle(id) => id,
        SplitterResizeTarget::Junction(id) => {
            let Some(first) = id.splitters().into_iter().next() else {
                return Ok(None);
            };
            first
        }
    };
    let (Some(facts), Some(record)) = (plan.layout_facts(), plan.splitter_record(original)) else {
        return Ok(None);
    };
    let root = original.root;
    let Some(fact) = facts.root(root) else {
        return Ok(None);
    };
    let content = fact.content_bounds();
    let clip = clip_rect(content, fact.bounds())?;
    let contacts = measure_resize_geometry(projection, facts.config().splitter_thickness());
    let compiler = RootBoundaries {
        policy,
        config: facts.config(),
        surface: plan.surface(),
        root,
        layer: record.layer(),
        content_clip: clip,
        projection,
        contacts: &contacts,
    };
    let mut records = Vec::new();
    let mut emit_node = |node| {
        if let Some(result) = projection.nodes.get(&node).filter(|result| result.visible)
            && let Some(owner) = workspace.node(node)
        {
            let bounds = clip_rect(clip_rect(result.rect, content)?, clip)?;
            compiler.emit(node, owner, bounds, weights.get(&node), |_, record| {
                records.extend(record.map(|mut record| {
                    record.retain_operability(
                        plan.region_is_operable(record.hit().rect(), record.layer()),
                    );
                    record
                }));
                Ok(())
            })?;
        }
        Ok::<_, SceneCompilationError>(())
    };
    match target {
        SplitterResizeTarget::Handle(id) => emit_node(id.node())?,
        SplitterResizeTarget::Junction(_) => {
            for (node, _) in projection.splits.iter() {
                emit_node(*node)?;
            }
            for (node, _) in projection.grids.iter() {
                emit_node(*node)?;
            }
        }
    }
    match target {
        SplitterResizeTarget::Handle(id) => {
            records.retain(|record| *record.id() == id);
        }
        SplitterResizeTarget::Junction(id) => {
            let present = derive_splitter_junction_candidates(&records)?
                .iter()
                .any(|candidate| {
                    candidate.id() == id && plan.region_is_operable(candidate.hit, candidate.layer)
                });
            if !present {
                return Ok(None);
            }
            let splitters = id.splitters();
            records.retain(|record| splitters.contains(record.id()));
        }
    }
    Ok((!records.is_empty()).then_some(records))
}

fn splitter_hit_rects(
    children: &[ChildProjection],
    axis: crate::graph::Axis,
    extent: f64,
    bounds: LogicalRect,
) -> Result<Vec<LogicalRect>, GeometryError> {
    children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| child.following_gap.as_ref().map(|gap| (index, gap)))
        .map(|(index, splitter)| {
            let center = match axis {
                crate::graph::Axis::Horizontal => splitter.x() + splitter.width() * 0.5,
                crate::graph::Axis::Vertical => splitter.y() + splitter.height() * 0.5,
            };
            let coordinate = |rect: &LogicalRect| match axis {
                crate::graph::Axis::Horizontal => rect.x() + rect.width() * 0.5,
                crate::graph::Axis::Vertical => rect.y() + rect.height() * 0.5,
            };
            let axis_min = match axis {
                crate::graph::Axis::Horizontal => bounds.x(),
                crate::graph::Axis::Vertical => bounds.y(),
            };
            let axis_max = match axis {
                crate::graph::Axis::Horizontal => bounds.max().x(),
                crate::graph::Axis::Vertical => bounds.max().y(),
            };
            let cell_min = children[..index]
                .iter()
                .rev()
                .find_map(|previous| previous.following_gap.as_ref())
                .map_or(axis_min, |previous| (coordinate(previous) + center) * 0.5);
            let cell_max = children
                .get(index + 1..)
                .and_then(|children| {
                    children
                        .iter()
                        .find_map(|child| child.following_gap.as_ref())
                })
                .map_or(axis_max, |next| (center + coordinate(next)) * 0.5);
            let hit_min = (center - extent * 0.5).max(cell_min);
            let hit_max = (center + extent * 0.5).min(cell_max).max(hit_min);
            let hit = match axis {
                crate::graph::Axis::Horizontal => {
                    LogicalRect::new(hit_min, splitter.y(), hit_max - hit_min, splitter.height())?
                }
                crate::graph::Axis::Vertical => {
                    LogicalRect::new(splitter.x(), hit_min, splitter.width(), hit_max - hit_min)?
                }
            };
            clip_rect(hit, bounds)
        })
        .collect()
}
