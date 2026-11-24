//! Splitter tracks, size constraints and junction geometry validation.

use super::*;

pub(in crate::presentation::scene) fn splitter_extent_satisfies_constraints(
    extent: Option<f64>,
    minimum: f64,
    maximum: f64,
) -> bool {
    let Some(extent) = extent else {
        return false;
    };
    if !extent.is_finite()
        || extent < 0.0
        || !minimum.is_finite()
        || minimum < 0.0
        || !maximum.is_finite()
        || maximum < minimum
    {
        return false;
    }
    let tolerance =
        f64::EPSILON * extent.abs().max(minimum.abs()).max(maximum.abs()).max(1.0) * 16.0;
    (extent >= minimum || minimum - extent <= tolerance)
        && (extent <= maximum || extent - maximum <= tolerance)
}

#[cfg(test)]
pub(super) fn validate_splitter_records(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
    policy: &DockPolicySnapshot,
) -> Result<(), SceneBuildError> {
    validate_splitter_record_sets(ready, record_order, workspace)?;
    validate_splitter_geometry(ready, record_order, workspace, workspace_index, policy)?;
    validate_splitter_junction_records(ready, record_order)
}

#[cfg(test)]
fn validate_splitter_record_sets(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
    workspace: &Workspace,
) -> Result<(), SceneBuildError> {
    let splitters = &record_order.splitters;
    let splitter_gaps = &record_order.gaps;
    let compiled_roots = ready
        .content
        .pane_records
        .iter()
        .map(|pane| pane.id().root)
        .collect::<BTreeSet<_>>();
    let mut expected_splitters = BTreeSet::new();
    for root in compiled_roots {
        let Some(root_record) = workspace.root(root) else {
            return Err(SceneBuildError::CompiledRootUnavailable {
                surface: ready.surface,
                root,
            });
        };
        let mut pending = vec![root_record.node];
        let mut visited = BTreeSet::new();
        while let Some(node) = pending.pop() {
            if !visited.insert(node) {
                continue;
            }
            match workspace.node(node) {
                Some(Node::Pane { .. }) => {}
                Some(Node::Tabs { .. }) => {
                    if let Some(child) = workspace
                        .visible_tab_selection(node)
                        .and_then(TabKey::layout)
                    {
                        pending.push(child);
                    }
                }
                Some(Node::Grid { children, .. }) => {
                    if let Some(grid) = ready
                        .layout_facts()
                        .and_then(|layout| layout.root(root))
                        .and_then(|facts| facts.projection().grids.get(&node))
                    {
                        if ready
                            .layout_facts()
                            .and_then(|layout| layout.root(root))
                            .and_then(|facts| facts.projection().nodes.get(&node))
                            .is_some_and(|result| result.visible)
                        {
                            for axis in [Axis::Horizontal, Axis::Vertical] {
                                for index in 0..grid.axis(axis).tracks.len().saturating_sub(1) {
                                    expected_splitters
                                        .insert(SplitterSceneId::grid(root, node, axis, index));
                                }
                            }
                        }
                    }
                    pending.extend(children.iter().rev().copied());
                }
                Some(Node::Split { children, .. }) => {
                    let mut visible = children
                        .iter()
                        .enumerate()
                        .filter_map(|(index, &child)| {
                            workspace.node_has_visible_content(child).then_some(index)
                        })
                        .peekable();
                    while let Some(index) = visible.next() {
                        if visible.peek().is_none() {
                            break;
                        }
                        expected_splitters.insert(SplitterSceneId {
                            root,
                            owner: crate::presentation::scene::SplitterOwner::Split(node),
                            index,
                        });
                    }
                    pending.extend(children.iter().rev().copied());
                }
                None => {
                    return Err(SceneBuildError::CompiledRootUnavailable {
                        surface: ready.surface,
                        root,
                    });
                }
            }
        }
    }
    if !splitter_gaps
        .entries()
        .iter()
        .map(|&(id, _)| id)
        .eq(expected_splitters)
    {
        return Err(SceneBuildError::SplitterGapRecordSetMismatch {
            surface: ready.surface,
        });
    }
    let rendered_splitters = splitter_gaps
        .iter(&ready.facts.splitter_gap_records)
        .filter_map(|(id, record)| {
            (record.presentation() == SplitterGapPresentation::Rendered).then_some(id)
        });
    if !splitters
        .entries()
        .iter()
        .map(|&(id, _)| id)
        .eq(rendered_splitters)
    {
        return Err(SceneBuildError::SplitterRecordSetMismatch {
            surface: ready.surface,
        });
    }
    Ok(())
}

pub(super) fn validate_splitter_geometry(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
    policy: &DockPolicySnapshot,
) -> Result<(), SceneBuildError> {
    let splitters = &record_order.splitters;
    let resize_contacts = ready
        .layout_facts()
        .map(|layout| {
            layout
                .roots()
                .map(|root| {
                    (
                        root.root(),
                        measure_resize_geometry(
                            root.projection(),
                            layout.config().splitter_thickness(),
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut validated_resize_arrays: Option<&SplitterRecord> = None;
    for (id, record) in splitters.iter(&ready.facts.splitter_records) {
        if let SplitterOwner::Grid { node, axis } = id.owner {
            validate_grid_splitter(
                ready,
                record,
                workspace,
                workspace_index,
                policy,
                resize_contacts.get(&id.root),
                node,
                axis,
            )?;
            continue;
        }
        let Some(Node::Split { axis, children, .. }) = workspace.node(id.node()) else {
            return Err(SceneBuildError::InvalidSplitterRecord {
                surface: ready.surface,
                id,
            });
        };
        // Sibling gaps share immutable arrays; only that exact backing reuses this proof.
        let arrays_valid = validated_resize_arrays.is_some_and(|previous| {
            record.weights().len() == children.len()
                && record.child_extents().len() == children.len()
                && std::ptr::eq(previous.weights(), record.weights())
                && std::ptr::eq(previous.child_extents(), record.child_extents())
        }) || splitter_resize_arrays_are_valid(record, children.len());
        let before_extent = record.child_extents().get(id.index).copied();
        let after_extent = record.child_extents().get(record.after_index()).copied();
        let compressed =
            splitter_compression(ready, record).ok_or(SceneBuildError::InvalidSplitterRecord {
                surface: ready.surface,
                id,
            })?;
        if !workspace_index.semantic_node_exists(ready.surface, id.root, id.node())
            || id.index >= children.len().saturating_sub(1)
            || record.after_index() <= id.index
            || record.after_index() >= children.len()
            || record.split_children() != Some([children[id.index], children[record.after_index()]])
            || !workspace.node_has_visible_content(children[id.index])
            || !workspace.node_has_visible_content(children[record.after_index()])
            || children[id.index + 1..record.after_index()]
                .iter()
                .any(|&child| workspace.node_has_visible_content(child))
            || record.axis() != *axis
            || !arrays_valid
            || !rect_has_area(record.draw_bounds())
            || !rect_has_area(record.hit().rect())
            || !rect_contains(ready.facts.bounds, record.hit().rect())
            || !rect_contains(record.hit().rect(), record.draw_bounds())
            || !rect_contains(ready.facts.bounds, record.before_bounds())
            || !rect_contains(ready.facts.bounds, record.after_bounds())
            || !record.before_minimum_extent().is_finite()
            || record.before_minimum_extent() < 0.0
            || !record.after_minimum_extent().is_finite()
            || record.after_minimum_extent() < 0.0
            || (!compressed
                && (!splitter_extent_satisfies_constraints(
                    before_extent,
                    record.before_minimum_extent(),
                    record.before_maximum_extent(),
                ) || !splitter_extent_satisfies_constraints(
                    after_extent,
                    record.after_minimum_extent(),
                    record.after_maximum_extent(),
                )))
            || record
                .central_index()
                .is_some_and(|index| index >= children.len())
        {
            return Err(SceneBuildError::InvalidSplitterRecord {
                surface: ready.surface,
                id,
            });
        }
        validate_semantic_layer(ready.surface, id.root, record.layer(), workspace_index)?;
        if record.operable()
            != (policy.splitter_resize_is_allowed(id.root, record.axis(), ready.surface)
                && !compressed
                && resize_contacts.get(&id.root).is_some_and(|contacts| {
                    contacts.splitter_is_operable(
                        id.node(),
                        children[id.index],
                        children[record.after_index()],
                    )
                })
                && region_has_authoritative_area(
                    record.hit().rect(),
                    record.layer(),
                    &ready.facts.drop_occlusions,
                ))
        {
            return Err(SceneBuildError::InvalidSplitterRecord {
                surface: ready.surface,
                id,
            });
        }
        validated_resize_arrays = Some(record);
    }

    Ok(())
}

fn validate_grid_splitter(
    ready: &PresentationPlan,
    record: &SplitterRecord,
    workspace: &Workspace,
    index: &SceneWorkspaceIndex,
    policy: &DockPolicySnapshot,
    contacts: Option<&crate::presentation::layout::ResizeGeometryFacts>,
    node: NodeId,
    axis: Axis,
) -> Result<(), SceneBuildError> {
    let id = *record.id();
    let invalid = || SceneBuildError::InvalidSplitterRecord {
        surface: ready.surface,
        id,
    };
    let Some(Node::Grid { children, .. }) = workspace.node(node) else {
        return Err(invalid());
    };
    let layout = ready.layout_facts().ok_or_else(invalid)?;
    let root = layout.root(id.root).ok_or_else(invalid)?;
    let grid = root.projection().grids.get(&node).ok_or_else(invalid)?;
    let tracks = grid.axis(axis);
    let before = tracks.tracks.get(id.index).ok_or_else(invalid)?;
    let after = tracks.tracks.get(id.index + 1).ok_or_else(invalid)?;
    let proof = record.grid().ok_or_else(invalid)?;
    let clip_rect = |rect: LogicalRect, clip: LogicalRect| {
        let x = rect.x().max(clip.x()).min(clip.max().x());
        let y = rect.y().max(clip.y()).min(clip.max().y());
        LogicalRect::from_min_max(
            crate::geometry::LogicalPoint::new(x, y).map_err(|_| invalid())?,
            crate::geometry::LogicalPoint::new(
                rect.max().x().min(clip.max().x()).max(x),
                rect.max().y().min(clip.max().y()).max(y),
            )
            .map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())
    };
    // Boundary records use their Grid's local frame clipped to the root,
    // including overflow and rounding at the final track's far edge.
    let clip = clip_rect(
        clip_rect(
            root.projection().nodes.get(&node).ok_or_else(invalid)?.rect,
            root.content_bounds(),
        )?,
        root.bounds(),
    )?;
    let gap = before.following_gap.ok_or_else(invalid)?;
    let extents_match = record
        .child_extents()
        .iter()
        .copied()
        .eq(tracks.tracks.iter().map(|track| track.extent));
    let operable = policy.splitter_resize_is_allowed(id.root, axis, ready.surface)
        && tracks.overflow == 0.0
        && tracks.unallocated <= f64::EPSILON * 64.0 * before.extent.max(after.extent).max(1.0)
        && contacts
            .is_some_and(|contacts| contacts.grid_splitter_is_operable(grid, axis, id.index))
        && region_has_authoritative_area(
            record.hit().rect(),
            record.layer(),
            &ready.facts.drop_occlusions,
        );
    if !index.semantic_node_exists(ready.surface, id.root, node)
        || children.as_slice() != proof.children()
        || grid.children.as_ref() != proof.children()
        || grid.shape.rows != proof.rows()
        || grid.shape.columns != proof.columns()
        || record.axis() != axis
        || record.after_index() != id.index + 1
        || !splitter_resize_arrays_are_valid(record, tracks.tracks.len())
        || !extents_match
        || record.weights() != tracks.weights.as_slice()
        || record.draw_bounds() != clip_rect(gap, clip)?
        || record.before_bounds() != clip_rect(before.bounds, clip)?
        || record.after_bounds() != clip_rect(after.bounds, clip)?
        || record.before_minimum_extent() != before.minimum
        || record.before_maximum_extent() != before.maximum
        || record.after_minimum_extent() != after.minimum
        || record.after_maximum_extent() != after.maximum
        || !rect_has_area(record.draw_bounds())
        || !rect_has_area(record.hit().rect())
        || !rect_contains(record.hit().rect(), record.draw_bounds())
        || !rect_contains(ready.bounds(), record.hit().rect())
        || record.operable() != operable
    {
        return Err(invalid());
    }
    validate_semantic_layer(ready.surface, id.root, record.layer(), index)
}

fn splitter_compression(ready: &PresentationPlan, record: &SplitterRecord) -> Option<bool> {
    let Some(layout) = ready.layout_facts() else {
        return Some(false);
    };
    if layout.config().pane_overflow_policy()
        != crate::presentation::config::PaneOverflowPolicy::Compress
    {
        return Some(false);
    }
    let id = record.id();
    if let SplitterOwner::Grid { node, axis } = id.owner {
        return Some(
            layout
                .root(id.root)?
                .projection()
                .grids
                .get(&node)?
                .axis(axis)
                .overflow
                > 0.0,
        );
    }
    let split = layout.root(id.root)?.projection().splits.get(&id.node())?;
    if split.overflow == 0.0 {
        return Some(false);
    }
    let before = split.children.get(id.index)?;
    let after = split.children.get(record.after_index())?;
    (record.axis() == split.axis
        && record.before_minimum_extent() == before.minimum
        && record.before_maximum_extent() == before.maximum
        && record.after_minimum_extent() == after.minimum
        && record.after_maximum_extent() == after.maximum
        && record.child_extents().get(id.index) == Some(&before.extent)
        && record.child_extents().get(record.after_index()) == Some(&after.extent))
    .then_some(true)
}

#[cfg(test)]
fn validate_splitter_junction_records(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
) -> Result<(), SceneBuildError> {
    let splitters = &record_order.splitters;
    let expected_junctions = derive_splitter_junction_candidates(
        splitters
            .iter(&ready.facts.splitter_records)
            .map(|(_, record)| record),
    )
    .map_err(|source| SceneBuildError::InvalidSplitterJunctionGeometry {
        surface: ready.surface,
        source,
    })?
    .into_iter()
    .filter(|candidate| {
        region_has_authoritative_area(candidate.hit, candidate.layer, &ready.facts.drop_occlusions)
    })
    .map(|candidate| (candidate.id(), candidate))
    .collect::<BTreeMap<_, _>>();
    if ready.facts.splitter_junction_records.len() != expected_junctions.len() {
        return Err(SceneBuildError::SplitterJunctionRecordSetMismatch {
            surface: ready.surface,
        });
    }
    for junction in &ready.facts.splitter_junction_records {
        let Some(expected) = expected_junctions.get(&junction.id()).copied() else {
            return Err(SceneBuildError::InvalidSplitterJunctionRecord {
                surface: ready.surface,
                id: junction.id(),
            });
        };
        let expected_operable = expected.id().splitters().into_iter().all(|splitter| {
            splitters
                .get(&ready.facts.splitter_records, splitter)
                .is_some_and(|record| record.operable())
        });
        if junction.layer() != expected.layer
            || !rect_has_area(junction.hit().rect())
            || junction.hit().rect() != expected.hit
            || junction.operable() != expected_operable
        {
            return Err(SceneBuildError::InvalidSplitterJunctionRecord {
                surface: ready.surface,
                id: junction.id(),
            });
        }
    }
    Ok(())
}

fn splitter_resize_arrays_are_valid(record: &SplitterRecord, children: usize) -> bool {
    #[cfg(test)]
    SPLITTER_RESIZE_ARRAY_VALIDATIONS.with(|count| count.set(count.get() + 1));

    if record.weights().len() != children || record.child_extents().len() != children {
        return false;
    }
    let weight_sum = normalized_weight_sum(record.weights().iter().map(|weight| weight.get()));
    weight_sum.is_finite()
        && (weight_sum - 1.0).abs() <= NORMALIZED_WEIGHT_TOLERANCE
        && record
            .child_extents()
            .iter()
            .all(|extent| extent.is_finite() && *extent >= 0.0)
}
