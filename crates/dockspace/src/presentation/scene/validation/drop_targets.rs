//! Canonical docking target and guide validation.

use super::*;

pub(super) fn validate_compiled_target_coverage(
    ready: &PresentationPlan,
    mut coverage: MagneticTargetCoverage<'_, '_>,
) -> Result<(), SceneBuildError> {
    let invalid = || SceneBuildError::InvalidMagneticTargetField {
        surface: ready.surface,
    };
    // Zero content or unavailable guide geometry can omit a field/cluster.
    // Check the actual emitted coverage, without deriving their recipe again.
    for field in ready.facts.magnetic_target_field.roots() {
        for id in field.edges().targets() {
            coverage.get(&id).ok_or_else(&invalid)?;
        }
    }
    for field in ready.facts.magnetic_target_field.panes() {
        for id in std::iter::once(field.center_target()).chain(field.edges().targets()) {
            coverage.get(&id).ok_or_else(&invalid)?;
        }
    }
    if !coverage.is_complete() {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_magnetic_target_field(
    ready: &PresentationPlan,
    mut target_coverage: MagneticTargetCoverage<'_, '_>,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let invalid = || SceneBuildError::InvalidMagneticTargetField {
        surface: ready.surface,
    };
    let Some(layout) = ready.layout_facts() else {
        return if ready.facts.magnetic_target_field.is_empty() {
            Ok(())
        } else {
            Err(invalid())
        };
    };
    let config = layout.config();
    let expected_fraction = config.dock_fraction();

    let mut depths = BTreeMap::new();
    for facts in layout.roots() {
        let root = facts.root();
        let root_record = workspace.root(root).ok_or_else(&invalid)?;
        let mut pending = vec![(root_record.node, 0_usize)];
        while let Some((node, depth)) = pending.pop() {
            if depths.insert((root, node), depth).is_some() {
                return Err(invalid());
            }
            match workspace.node(node).ok_or_else(&invalid)? {
                Node::Pane { .. } => {}
                Node::Tabs { .. } => {
                    if let Some(child) = workspace
                        .visible_tab_selection(node)
                        .and_then(TabKey::layout)
                    {
                        pending.push((child, depth + 1));
                    }
                }
                Node::Split { children, .. } | Node::Grid { children, .. } => pending.extend(
                    children
                        .iter()
                        .rev()
                        .copied()
                        .map(|child| (child, depth + 1)),
                ),
            }
        }
    }

    let mut roots = BTreeMap::new();
    for field in ready.facts.magnetic_target_field.roots() {
        if roots.insert(field.root(), field).is_some() {
            return Err(invalid());
        }
    }
    if roots.len() != layout.roots().count() {
        return Err(invalid());
    }
    for facts in layout.roots() {
        let root = facts.root();
        let field = roots.get(&root).copied().ok_or_else(&invalid)?;
        let root_record = workspace.root(root).ok_or_else(&invalid)?;
        let layer = workspace_index.root_layer(root).ok_or_else(&invalid)?;
        let expected_edges = MagneticEdgeTargets::new(
            DropTargetId::OuterEdge {
                surface: ready.surface,
                root,
                node: root_record.node,
                edge: Edge::Left,
            },
            DropTargetId::OuterEdge {
                surface: ready.surface,
                root,
                node: root_record.node,
                edge: Edge::Right,
            },
            DropTargetId::OuterEdge {
                surface: ready.surface,
                root,
                node: root_record.node,
                edge: Edge::Top,
            },
            DropTargetId::OuterEdge {
                surface: ready.surface,
                root,
                node: root_record.node,
                edge: Edge::Bottom,
            },
        );
        if field.layer() != layer
            || field.bounds() != facts.bounds()
            || field.edge_band().to_bits() != root_edge_band(facts.bounds(), config).to_bits()
            || field.edges() != expected_edges
            || !magnetic_targets_are_canonical(
                ready,
                &mut target_coverage,
                expected_edges.targets(),
                layer,
                expected_fraction,
            )
        {
            return Err(invalid());
        }
    }

    let expected_panes = ready
        .facts
        .tabs_records
        .iter()
        .filter(|pane| rect_has_area(pane.content_bounds()))
        .map(|pane| ((pane.id().root, pane.id().tabs), pane.content_bounds()))
        .chain(
            ready
                .content
                .pane_records
                .iter()
                .filter(|pane| {
                    rect_has_area(pane.bounds())
                        && matches!(workspace.node(pane.owner_node()), Some(Node::Pane { .. }))
                })
                .map(|pane| ((pane.id().root, pane.owner_node()), pane.bounds())),
        )
        .collect::<BTreeMap<_, _>>();
    let mut panes = BTreeMap::new();
    for field in ready.facts.magnetic_target_field.panes() {
        let id = (field.root(), field.tabs());
        if panes.insert(id, field).is_some() {
            return Err(invalid());
        }
    }
    if panes.len() != expected_panes.len() {
        return Err(invalid());
    }
    for ((root, node), content) in expected_panes {
        let id = (root, node);
        let field = panes.get(&id).copied().ok_or_else(&invalid)?;
        let root_record = workspace.root(root).ok_or_else(&invalid)?;
        let layer = workspace_index.root_layer(root).ok_or_else(&invalid)?;
        let depth = depths.get(&id).copied().ok_or_else(&invalid)?;
        let center_target = DropTargetId::Center {
            surface: ready.surface,
            root,
            tabs: node,
        };
        let edge_target = |edge| {
            if root_record.node == node && root_record.central == Some(node) {
                DropTargetId::OuterEdge {
                    surface: ready.surface,
                    root,
                    node,
                    edge,
                }
            } else {
                DropTargetId::InnerEdge {
                    surface: ready.surface,
                    root,
                    node,
                    edge,
                }
            }
        };
        let expected_edges = MagneticEdgeTargets::new(
            edge_target(Edge::Left),
            edge_target(Edge::Right),
            edge_target(Edge::Top),
            edge_target(Edge::Bottom),
        );
        if field.layer() != layer
            || field.depth() != depth
            || field.content() != content
            || field.center() != pane_center_basin(content, config).map_err(|_| invalid())?
            || field.center_target() != center_target
            || field.edges() != expected_edges
            || !magnetic_targets_are_canonical(
                ready,
                &mut target_coverage,
                std::iter::once(center_target).chain(expected_edges.targets()),
                layer,
                expected_fraction,
            )
        {
            return Err(invalid());
        }
    }
    if !target_coverage.is_complete() {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn magnetic_targets_are_canonical(
    ready: &PresentationPlan,
    target_coverage: &mut MagneticTargetCoverage<'_, '_>,
    targets: impl IntoIterator<Item = DropTargetId>,
    layer: SceneLayerKey,
    expected_fraction: f64,
) -> bool {
    targets.into_iter().all(|target| {
        target.surface() == ready.surface
            && target_coverage.get(&target).is_some_and(|record| {
                record.layer() == layer
                    && match record.location() {
                        DropTargetLocation::Center { .. } => true,
                        DropTargetLocation::InnerEdge { fraction, .. }
                        | DropTargetLocation::OuterEdge { fraction, .. } => {
                            fraction.get().to_bits() == expected_fraction.to_bits()
                        }
                        DropTargetLocation::TabGap { .. }
                        | DropTargetLocation::SurfaceBackground { .. } => false,
                    }
            })
    })
}

pub(super) fn validate_surface_background(
    ready: &PresentationPlan,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let Some(presentation) = workspace.surface(ready.surface) else {
        if let Some(background) = &ready.facts.surface_background {
            return Err(SceneBuildError::UnexpectedSurfaceBackground {
                surface: ready.surface,
                target: background.id(),
            });
        }
        return Ok(());
    };

    match (presentation.main_root, &ready.facts.surface_background) {
        (None, None) => {
            return Err(SceneBuildError::MissingSurfaceBackground {
                surface: ready.surface,
            });
        }
        (Some(_), Some(background)) => {
            return Err(SceneBuildError::UnexpectedSurfaceBackground {
                surface: ready.surface,
                target: background.id(),
            });
        }
        (Some(_), None) => return Ok(()),
        (None, Some(_)) => {}
    }

    let background = ready.facts.surface_background.as_ref().ok_or(
        SceneBuildError::MissingSurfaceBackground {
            surface: ready.surface,
        },
    )?;
    if background.id().surface() != ready.surface
        || background.availability() != DropTargetAvailability::Available
        || !matches!(
            background.location(),
            DropTargetLocation::SurfaceBackground { surface }
                if surface == ready.surface
        )
    {
        return Err(SceneBuildError::InvalidSurfaceBackgroundRecord {
            surface: ready.surface,
            target: background.id(),
        });
    }
    if background.layer() != SceneLayerKey::surface_base()
        || workspace_index
            .contained_layers(ready.surface)
            .any(|layer| background.layer() >= layer)
    {
        return Err(SceneBuildError::SurfaceBackgroundLayerMismatch {
            surface: ready.surface,
            actual: background.layer(),
        });
    }
    if !rect_has_area(background.region().rect())
        || !rect_contains(ready.facts.bounds, background.region().rect())
    {
        return Err(SceneBuildError::InvalidSurfaceBackgroundHitRegion {
            surface: ready.surface,
        });
    }
    validate_drop_visual(ready, background)
}

pub(super) fn canonical_drop_target_availability(
    target: &DropTargetRecord,
    policy: &DockPolicySnapshot,
) -> DropTargetAvailability {
    if !target.availability().is_available() {
        return target.availability();
    }
    let allowed = match target.id().kind() {
        DropTargetKind::TabGap | DropTargetKind::Center => policy.allows_tab_merge(),
        DropTargetKind::InnerEdge | DropTargetKind::OuterEdge => policy.allows_edge_split(),
        DropTargetKind::SurfaceBackground => true,
    };
    if allowed {
        DropTargetAvailability::Available
    } else {
        DropTargetAvailability::Unavailable(DropTargetUnavailable::PolicyDisabled)
    }
}

pub(super) fn validate_drop_guide_cluster(
    ready: &PresentationPlan,
    target_coverage: &mut MagneticTargetCoverage<'_, '_>,
    cluster: &DropGuideClusterRecord,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let cluster_id = cluster.id();
    validate_drop_guide_cluster_identity(ready, cluster_id, workspace, workspace_index)?;
    let expected_layer = match cluster_id {
        DropGuideClusterId::Surface { .. } => SceneLayerKey::surface_base(),
        DropGuideClusterId::Inner { root, .. } | DropGuideClusterId::Outer { root, .. } => {
            workspace_index
                .root_layer(root)
                .ok_or(SceneBuildError::InvalidDropGuideCluster {
                    surface: ready.surface,
                    cluster: cluster_id,
                })?
        }
    };
    if cluster.layer() != expected_layer {
        return Err(SceneBuildError::DropGuideClusterLayerMismatch {
            surface: ready.surface,
            cluster: cluster_id,
            expected: expected_layer,
            actual: cluster.layer(),
        });
    }
    let activation = validate_drop_guide_activation(ready, cluster)?;
    for (slot, guide_target) in cluster.targets() {
        validate_drop_guide_target(
            ready,
            target_coverage,
            cluster,
            activation,
            slot,
            guide_target,
            workspace,
            workspace_index,
        )?;
    }
    validate_drop_guide_hit_separation(ready.surface, cluster)
}

fn validate_drop_guide_cluster_identity(
    ready: &PresentationPlan,
    cluster: DropGuideClusterId,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let scope_is_valid = match cluster {
        DropGuideClusterId::Surface { .. } => workspace
            .surface(ready.surface)
            .is_some_and(|presentation| presentation.main_root.is_none()),
        DropGuideClusterId::Inner { root, node, .. } => {
            workspace_index.semantic_node_exists(ready.surface, root, node)
                && matches!(
                    workspace.node(node),
                    Some(Node::Pane { .. } | Node::Tabs { .. })
                )
        }
        DropGuideClusterId::Outer { root, .. } => {
            workspace_index.root_belongs_to_surface(ready.surface, root)
                && workspace_index
                    .root_node(root)
                    .and_then(|node| workspace.node(node))
                    .is_some()
        }
    };
    if cluster.surface_id() != ready.surface || !scope_is_valid {
        return Err(SceneBuildError::InvalidDropGuideCluster {
            surface: ready.surface,
            cluster,
        });
    }
    Ok(())
}

fn validate_drop_guide_activation(
    ready: &PresentationPlan,
    cluster: &DropGuideClusterRecord,
) -> Result<LogicalRect, SceneBuildError> {
    let cluster_id = cluster.id();
    let activation = cluster.activation().rect();
    if !rect_has_area(activation) {
        return Err(SceneBuildError::EmptyDropGuideActivation {
            surface: ready.surface,
            cluster: cluster_id,
        });
    }
    if !rect_contains(ready.facts.bounds, activation) {
        return Err(SceneBuildError::DropGuideActivationOutsideSurface {
            surface: ready.surface,
            cluster: cluster_id,
        });
    }
    Ok(activation)
}

fn validate_drop_guide_target(
    ready: &PresentationPlan,
    target_coverage: &mut MagneticTargetCoverage<'_, '_>,
    cluster: &DropGuideClusterRecord,
    activation: LogicalRect,
    slot: DropGuideSlot,
    guide_target: &DropGuideTargetRecord,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let cluster_id = cluster.id();
    let target_id = guide_target.id();
    let target = match target_id {
        DropTargetId::SurfaceBackground { .. } => ready
            .facts
            .surface_background
            .as_ref()
            .filter(|target| target.id() == target_id),
        _ => target_coverage.get(&target_id),
    };
    let Some(target) = target else {
        return Err(SceneBuildError::TargetSemanticMismatch {
            surface: ready.surface,
            target: target_id,
        });
    };
    if !guide_target.shares_target(target) {
        return Err(SceneBuildError::TargetSemanticMismatch {
            surface: ready.surface,
            target: target_id,
        });
    }
    let target = target.as_ref();
    if !drop_guide_slot_matches(cluster_id, slot, target_id, workspace, workspace_index) {
        return Err(SceneBuildError::DropGuideTargetSlotMismatch {
            cluster: cluster_id,
            slot,
            target: target_id,
        });
    }
    if target.layer() != cluster.layer() {
        return Err(SceneBuildError::DropGuideTargetLayerMismatch {
            cluster: cluster_id,
            target: target_id,
            cluster_layer: cluster.layer(),
            target_layer: target.layer(),
        });
    }

    let hit = target.region().rect();
    if !rect_has_area(hit) {
        return Err(SceneBuildError::EmptyDropGuideHitRegion {
            surface: ready.surface,
            cluster: cluster_id,
            target: target_id,
        });
    }
    if !rect_contains(ready.facts.bounds, hit) {
        return Err(SceneBuildError::DropGuideHitRegionOutsideSurface {
            surface: ready.surface,
            cluster: cluster_id,
            target: target_id,
        });
    }
    if !rect_contains(activation, hit) {
        return Err(SceneBuildError::DropGuideHitRegionOutsideActivation {
            surface: ready.surface,
            cluster: cluster_id,
            target: target_id,
        });
    }

    let draw = guide_target.draw();
    if !rect_has_area(draw) {
        return Err(SceneBuildError::EmptyDropGuideDraw {
            surface: ready.surface,
            cluster: cluster_id,
            target: target_id,
        });
    }
    if !rect_contains(hit, draw) {
        return Err(SceneBuildError::DropGuideDrawOutsideHitRegion {
            surface: ready.surface,
            cluster: cluster_id,
            target: target_id,
        });
    }

    let preview = target.visual().rect();
    if !rect_contains(activation, preview) {
        return Err(SceneBuildError::DropGuidePreviewOutsideActivation {
            surface: ready.surface,
            cluster: cluster_id,
            target: target_id,
        });
    }
    Ok(())
}

fn validate_drop_guide_hit_separation(
    surface: SurfaceId,
    cluster: &DropGuideClusterRecord,
) -> Result<(), SceneBuildError> {
    // Every slot has already proved that its owner is the exact catalog Arc.
    for (index, (first_slot, first)) in cluster.targets().enumerate() {
        for (second_slot, second) in cluster.targets().skip(index + 1) {
            if rects_overlap_with_area(
                first.target().region().rect(),
                second.target().region().rect(),
            ) {
                return Err(SceneBuildError::OverlappingDropGuideHitRegions {
                    surface,
                    cluster: cluster.id(),
                    first: first_slot,
                    second: second_slot,
                });
            }
        }
    }
    Ok(())
}

fn drop_guide_slot_matches(
    cluster: DropGuideClusterId,
    slot: DropGuideSlot,
    target: DropTargetId,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> bool {
    match (cluster, slot) {
        (DropGuideClusterId::Surface { surface }, DropGuideSlot::Center) => {
            target == DropTargetId::SurfaceBackground { surface }
        }
        (DropGuideClusterId::Surface { .. }, DropGuideSlot::Edge(_)) => false,
        (
            DropGuideClusterId::Inner {
                surface,
                root,
                node,
            },
            DropGuideSlot::Center,
        ) => {
            target
                == (DropTargetId::Center {
                    surface,
                    root,
                    tabs: node,
                })
        }
        (
            DropGuideClusterId::Inner {
                surface,
                root,
                node,
            },
            DropGuideSlot::Edge(edge),
        ) => {
            let is_root_central = workspace_index.root_node(root) == Some(node)
                && workspace
                    .root(root)
                    .is_some_and(|root| root.central == Some(node));
            !is_root_central
                && target
                    == (DropTargetId::InnerEdge {
                        surface,
                        root,
                        node,
                        edge,
                    })
        }
        (DropGuideClusterId::Outer { .. }, DropGuideSlot::Center) => false,
        (DropGuideClusterId::Outer { surface, root }, DropGuideSlot::Edge(edge)) => {
            workspace_index.root_node(root).is_some_and(|node| {
                target
                    == (DropTargetId::OuterEdge {
                        surface,
                        root,
                        node,
                        edge,
                    })
            })
        }
    }
}

pub(super) fn validate_drop_target(
    ready: &PresentationPlan,
    target: &DropTargetRecord,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    if matches!(
        target.location(),
        DropTargetLocation::SurfaceBackground { .. }
    ) {
        return Err(SceneBuildError::SurfaceBackgroundInTopologyTargets {
            surface: ready.surface,
        });
    }
    validate_drop_visual(ready, target)?;
    let root = target
        .location()
        .root()
        .ok_or(SceneBuildError::TargetSemanticMismatch {
            surface: ready.surface,
            target: target.id(),
        })?;
    let expected =
        workspace_index
            .root_layer(root)
            .ok_or(SceneBuildError::TargetSemanticMismatch {
                surface: ready.surface,
                target: target.id(),
            })?;
    if target.layer() != expected {
        return Err(SceneBuildError::DropTargetLayerMismatch {
            surface: ready.surface,
            target: target.id(),
            expected,
            actual: target.layer(),
        });
    }
    if target.id().surface() != ready.surface
        || !target_structure_is_valid(workspace, workspace_index, ready.surface, target)
    {
        return Err(SceneBuildError::TargetSemanticMismatch {
            surface: ready.surface,
            target: target.id(),
        });
    }
    Ok(())
}

pub(super) fn target_structure_is_valid(
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
    surface: SurfaceId,
    record: &DropTargetRecord,
) -> bool {
    let Some(root) = record.location().root() else {
        return false;
    };
    if !workspace_index.root_belongs_to_surface(surface, root) {
        return false;
    }
    match record.location() {
        DropTargetLocation::TabGap { tabs, index, .. } => {
            workspace_index.contains_node(root, tabs)
                && workspace.node_has_visible_content(tabs)
                && matches!(workspace.node(tabs), Some(Node::Tabs { items, .. }) if index <= items.len())
        }
        DropTargetLocation::Center { tabs, .. } => {
            workspace_index.contains_node(root, tabs)
                && workspace.node_has_visible_content(tabs)
                && matches!(
                    workspace.node(tabs),
                    Some(Node::Pane { .. } | Node::Tabs { .. })
                )
        }
        DropTargetLocation::InnerEdge { node, .. } => {
            workspace_index.contains_node(root, node)
                && workspace.node_has_visible_content(node)
                && matches!(
                    workspace.node(node),
                    Some(Node::Pane { .. } | Node::Tabs { .. })
                )
        }
        DropTargetLocation::OuterEdge { node, .. } => workspace_index.root_node(root) == Some(node),
        DropTargetLocation::SurfaceBackground { .. } => false,
    }
}

pub(super) fn validate_drop_visual(
    ready: &PresentationPlan,
    target: &DropTargetRecord,
) -> Result<(), SceneBuildError> {
    let visual = target.visual().rect();
    if visual.width() <= 0.0 || visual.height() <= 0.0 {
        return Err(SceneBuildError::EmptyDropVisual {
            surface: ready.surface,
            target: target.id(),
        });
    }
    let bounds_min = ready.facts.bounds.min();
    let bounds_max = ready.facts.bounds.max();
    let visual_min = visual.min();
    let visual_max = visual.max();
    if visual_min.x() < bounds_min.x()
        || visual_min.y() < bounds_min.y()
        || visual_max.x() > bounds_max.x()
        || visual_max.y() > bounds_max.y()
    {
        return Err(SceneBuildError::DropVisualOutsideSurface {
            surface: ready.surface,
            target: target.id(),
        });
    }
    Ok(())
}
