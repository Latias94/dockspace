//! Validation of contained placement and its pane ownership.

use super::*;

pub(super) fn validate_contained_minimums(
    ready: &PresentationPlan,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let surface = ready.surface;
    let published: HashSet<_> = ready
        .facts
        .contained_minimums
        .iter()
        .map(|measurement| measurement.floating())
        .collect();
    if let Some(floating) = ready
        .facts
        .contained_minimums
        .iter()
        .map(|measurement| measurement.floating())
        .filter(|floating| !workspace_index.contained_belongs_to_surface(surface, *floating))
        .min()
    {
        return Err(SceneBuildError::UnexpectedContainedMinimumMeasurement { surface, floating });
    }
    for floating in workspace_index.contained_on_surface(surface) {
        if !published.contains(&floating) {
            return Err(SceneBuildError::MissingContainedMinimumMeasurement { surface, floating });
        }
    }
    Ok(())
}

pub(super) fn validate_contained_records(
    ready: &PresentationPlan,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let expected = workspace
        .surface(ready.surface)
        .into_iter()
        .flat_map(|presentation| presentation.contained.iter().copied().enumerate())
        .filter_map(|(ordinal, floating)| {
            let indexed = workspace_index.contained_records().get(&floating)?;
            (workspace
                .root(indexed.root)
                .is_some_and(|root| workspace.node_has_visible_content(root.node))
                && rects_overlap_with_area(ready.facts.bounds, indexed.rect))
            .then_some((floating, ordinal, indexed.root, indexed.rect, indexed.layer))
        })
        .collect::<Vec<_>>();
    if ready.facts.contained_records.len() != expected.len() {
        return Err(SceneBuildError::ContainedRecordSetMismatch {
            surface: ready.surface,
        });
    }
    let minimums = ready
        .facts
        .contained_minimums
        .iter()
        .map(|measurement| (measurement.floating(), measurement.minimum_size()))
        .collect::<HashMap<_, _>>();
    const DIRECTIONS: [ContainedResizeDirection; 8] = [
        ContainedResizeDirection::NorthWest,
        ContainedResizeDirection::North,
        ContainedResizeDirection::NorthEast,
        ContainedResizeDirection::East,
        ContainedResizeDirection::SouthEast,
        ContainedResizeDirection::South,
        ContainedResizeDirection::SouthWest,
        ContainedResizeDirection::West,
    ];
    for (record, (floating, ordinal, root, durable, layer)) in
        ready.facts.contained_records.iter().zip(expected)
    {
        let valid_identity = record.floating() == floating
            && record.ordinal() == ordinal
            && record.root() == root
            && record.layer() == layer
            && minimums.get(&floating).copied() == Some(record.minimum_size());
        let valid_partition =
            rect_is_exact_intersection(record.outer_bounds(), ready.facts.bounds, durable)
                && rect_has_area(record.outer_bounds())
                && rect_contains(record.outer_bounds(), record.inner_bounds())
                && rect_contains(record.inner_bounds(), record.title_bounds())
                && rect_contains(record.inner_bounds(), record.content_bounds())
                && rect_contains(record.title_bounds(), record.title_drag_hit().rect())
                && !rects_overlap_with_area(record.title_bounds(), record.content_bounds())
                && record.close_bounds().is_none_or(|close| {
                    rect_has_area(close)
                        && rect_contains(record.title_bounds(), close)
                        && !rects_overlap_with_area(close, record.title_drag_hit().rect())
                });
        let valid_resize =
            record
                .resize()
                .iter()
                .zip(DIRECTIONS)
                .all(|(resize, direction)| {
                    resize.direction() == direction
                        && rect_contains(record.outer_bounds(), resize.hit().rect())
                })
                && record.resize().iter().enumerate().all(|(index, first)| {
                    record.resize().iter().skip(index + 1).all(|second| {
                        !rects_overlap_with_area(first.hit().rect(), second.hit().rect())
                    })
                });
        if !valid_identity || !valid_partition || !valid_resize {
            return Err(SceneBuildError::InvalidContainedRecord {
                surface: ready.surface,
                floating: record.floating(),
            });
        }
    }
    Ok(())
}
