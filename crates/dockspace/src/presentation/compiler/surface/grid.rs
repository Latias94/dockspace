//! Grid boundaries borrow the same shared tracks that place real content.

use super::*;
use crate::graph::Axis;
use crate::presentation::layout::{ChildProjection, GridProjection};

pub(super) fn compile_grid_boundaries(
    policy: &DockPolicySnapshot,
    config: &DockPresentationConfig,
    surface: SurfaceId,
    root: RootId,
    node: NodeId,
    grid: &GridProjection,
    clip: LogicalRect,
    layer: SceneLayerKey,
    ancestry: Option<usize>,
    contacts: &crate::presentation::layout::ResizeGeometryFacts,
    ready: &mut PresentationPlan,
) -> Result<(), SceneCompilationError> {
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
        let hits = splitter_hit_rects(&children, axis, config.splitter_hit_extent(), clip)?;
        let extents = projection
            .tracks
            .iter()
            .map(|track| track.extent)
            .collect::<Arc<[f64]>>();
        let weights = Arc::<[SplitWeight]>::from(projection.weights.as_slice());
        for (index, hit) in hits.into_iter().enumerate() {
            let before = &projection.tracks[index];
            let after = &projection.tracks[index + 1];
            let id = SplitterSceneId::grid(root, node, axis, index);
            let gap = before
                .following_gap
                .map(|gap| intersect_rect(gap, clip))
                .transpose()?
                .flatten();
            ready.push_splitter_gap_record(SplitterGapRecord::new(
                id,
                if gap.is_some() {
                    SplitterGapPresentation::Rendered
                } else {
                    SplitterGapPresentation::Collapsed
                },
            ));
            let Some(gap) = gap else {
                continue;
            };
            let operable = policy.splitter_resize_is_allowed(root, axis, surface)
                && projection.overflow == 0.0
                && projection.unallocated
                    <= f64::EPSILON * 64.0 * before.extent.max(after.extent).max(1.0)
                && contacts.grid_splitter_is_operable(grid, axis, index);
            ready.push_splitter_record(
                SplitterRecord::new(
                    id,
                    gap,
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
                    layer,
                    operable,
                )
                .with_grid_proof(
                    (grid.shape.rows, grid.shape.columns),
                    Arc::clone(&grid.children),
                )
                .with_active_ancestry(ancestry),
            );
            ready.push_surface_program_root_operation(SurfaceProgramRootOperation::Splitter(id))?;
        }
    }
    Ok(())
}
