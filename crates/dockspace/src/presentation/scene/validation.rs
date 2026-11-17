//! Validation and canonicalization of complete semantic presentation plans.

use super::*;
#[cfg(test)]
use crate::graph::command::Edge;
use crate::graph::workspace::WorkspaceIndexView;
use crate::graph::{Axis, NORMALIZED_WEIGHT_TOLERANCE, Node, normalized_weight_sum};
#[cfg(test)]
use crate::interaction::targeting::field::{pane_center_basin, root_edge_band};
use crate::interaction::targeting::targets::DropTargetLocation;
use crate::presentation::layout::measure_resize_geometry;

mod record_order;
mod target_catalog;
use record_order::RecordOrder;
use target_catalog::{DropTargetCatalog, DropTargetOrder, MagneticTargetCoverage};

struct SemanticRecordOrder {
    panes: RecordOrder<PaneSceneId>,
    bars: RecordOrder<TabBarSceneId>,
    tabs: RecordOrder<TabSceneId>,
    gaps: RecordOrder<SplitterSceneId>,
    splitters: RecordOrder<SplitterSceneId>,
}

#[cfg(test)]
mod cost_tests;

#[cfg(test)]
mod guide_tests;

#[cfg(test)]
std::thread_local! {
    static SPLITTER_RESIZE_ARRAY_VALIDATIONS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

/// Reusable validator for one complete surface presentation plan.
///
/// The validator reuses one workspace-derived index when the caller already has
/// a requirement manifest, while retaining the standalone constructor used by
/// direct scene and drop-resolution tests.
pub(crate) struct PresentationPlanValidator<'a> {
    workspace: &'a Workspace,
    workspace_index: SceneWorkspaceIndex<'a>,
    policy: &'a DockPolicySnapshot,
    contained_projection: Option<ContainedLayoutProjection>,
}

impl<'a> PresentationPlanValidator<'a> {
    #[cfg(test)]
    pub(crate) fn new(
        workspace: &'a Workspace,
        policy: &'a DockPolicySnapshot,
    ) -> Result<Self, SceneBuildError> {
        Ok(Self {
            workspace,
            workspace_index: SceneWorkspaceIndex::new(workspace)?,
            policy,
            contained_projection: None,
        })
    }

    pub(crate) fn for_manifest_surface(
        workspace: &'a Workspace,
        workspace_version: WorkspaceVersion,
        manifest: &'a SceneRequirementManifest,
        policy: &'a DockPolicySnapshot,
        surface: SurfaceId,
    ) -> Result<Self, SceneBuildError> {
        let actual = manifest.workspace_index().version();
        let Some(workspace_index) = manifest.workspace_index().at_version(workspace_version) else {
            return Err(SceneBuildError::WorkspaceIndexVersionMismatch {
                expected: workspace_version,
                actual,
            });
        };
        if manifest.surface(surface).is_none() {
            return Err(SceneBuildError::UnexpectedSceneSurface { surface });
        }
        Ok(Self {
            workspace,
            workspace_index: SceneWorkspaceIndex::from_manifest_surface(
                workspace,
                workspace_index,
                surface,
            )?,
            policy,
            contained_projection: None,
        })
    }

    pub(crate) fn with_contained_projection(
        mut self,
        projection: Option<ContainedLayoutProjection>,
    ) -> Result<Self, SceneBuildError> {
        if let Some(projection) = projection {
            let contained = match &mut self.workspace_index {
                SceneWorkspaceIndex::Complete(index) => {
                    index.contained.get_mut(&projection.floating)
                }
                SceneWorkspaceIndex::SharedSurface(index) => {
                    index.contained.get_mut(&projection.floating)
                }
            }
            .ok_or(SceneBuildError::InvalidContainedRecord {
                surface: projection.surface,
                floating: projection.floating,
            })?;
            if contained.surface != projection.surface
                || contained.root != projection.root
                || contained.rect != projection.source_rect
                || !rect_has_area(projection.rect)
            {
                return Err(SceneBuildError::InvalidContainedRecord {
                    surface: projection.surface,
                    floating: projection.floating,
                });
            }
            contained.rect = projection.rect;
        }
        self.contained_projection = projection;
        Ok(self)
    }

    /// Validates arbitrary plans, including deliberately malformed test facts.
    #[cfg(test)]
    pub(crate) fn validate_and_canonicalize(
        &self,
        mut ready: PresentationPlan,
    ) -> Result<PresentationPlan, SceneBuildError> {
        self.bind_command_bases(&mut ready);
        let (record_order, drop_target_index) = self.begin_validation(&ready)?;
        if ready.measurement_ticket().is_some() {
            validate_ready_semantics(
                &ready,
                &record_order,
                self.workspace,
                &self.workspace_index,
                self.policy,
            )?;
        }
        let target_coverage = self.validate_drop_records(&ready, &drop_target_index)?;
        validate_magnetic_target_field(
            &ready,
            target_coverage,
            self.workspace,
            &self.workspace_index,
        )?;
        let target_order = drop_target_index.into_order();
        self.finish_validation(ready, record_order, target_order)
    }

    /// The compiler owns the record roster and the junction/field recipe.
    /// Keep checks on derived geometry and current input authority, without
    /// independently rebuilding those same structural outputs a second time.
    pub(crate) fn finalize_compiled(
        compiled: crate::presentation::compiler::CompiledSurfacePlan<'_>,
    ) -> Result<PresentationPlan, SceneBuildError> {
        let (ready, validator) = compiled.into_finalization()?;
        let (record_order, drop_target_index) = validator.begin_validation(&ready)?;
        validate_contained_records(&ready, validator.workspace, &validator.workspace_index)?;
        for (id, record) in record_order.panes.iter(&ready.content.pane_records) {
            if !rect_contains(ready.facts.bounds, record.bounds())
                || !rect_contains(record.bounds(), record.content_bounds())
            {
                return Err(SceneBuildError::InvalidPaneRecord {
                    surface: ready.surface,
                    id,
                });
            }
        }
        validate_tab_records(
            &ready,
            &record_order,
            validator.workspace,
            &validator.workspace_index,
            validator.policy,
        )?;
        validate_presentation_menu_anchors(&ready, validator.workspace)?;
        validate_splitter_geometry(
            &ready,
            &record_order,
            validator.workspace,
            &validator.workspace_index,
            validator.policy,
        )?;
        let target_coverage = validator.validate_drop_records(&ready, &drop_target_index)?;
        validate_compiled_target_coverage(&ready, target_coverage)?;
        let target_order = drop_target_index.into_order();
        validator.finish_validation(ready, record_order, target_order)
    }

    #[cfg(test)]
    fn bind_command_bases(&self, ready: &mut PresentationPlan) {
        ready.bind_workspace_selection(self.workspace);
        ready.command_basis.clear();
        let Some(presentation) = self.workspace.surface(ready.surface) else {
            return;
        };
        let roots =
            presentation
                .main_root
                .into_iter()
                .chain(presentation.contained.iter().filter_map(|floating| {
                    self.workspace
                        .contained_floating(*floating)
                        .map(|record| record.root)
                }));
        for root in roots {
            if let Some(basis) = self.workspace_index.root_fingerprint(root) {
                ready.bind_command_basis(root, basis.clone());
            }
        }
    }

    fn begin_validation<'plan>(
        &self,
        ready: &'plan PresentationPlan,
    ) -> Result<(SemanticRecordOrder, DropTargetCatalog<'plan>), SceneBuildError> {
        let surface = ready.surface;
        if ready.contained_projection() != self.contained_projection
            && let Some(projection) = ready.contained_projection().or(self.contained_projection)
        {
            return Err(SceneBuildError::InvalidContainedRecord {
                surface,
                floating: projection.floating,
            });
        }
        if let Some(expected) = self.workspace_index.bound_surface()
            && expected != surface
        {
            return Err(SceneBuildError::PresentationSurfaceMismatch {
                expected,
                actual: surface,
            });
        }
        let (record_order, drop_target_index) = validate_semantic_uniqueness(ready)?;
        validate_popup_plane(ready)?;
        if !rect_has_area(ready.facts.bounds) {
            return Err(SceneBuildError::EmptyReadySurfaceBounds { surface });
        }
        validate_surface_background(ready, self.workspace, &self.workspace_index)?;
        Ok((record_order, drop_target_index))
    }

    fn validate_drop_records<'catalog, 'plan>(
        &self,
        ready: &'plan PresentationPlan,
        drop_target_index: &'catalog DropTargetCatalog<'plan>,
    ) -> Result<MagneticTargetCoverage<'catalog, 'plan>, SceneBuildError> {
        let surface = ready.surface;
        let published_occlusions: HashSet<_> = ready
            .facts
            .drop_occlusions
            .iter()
            .map(|occlusion| occlusion.floating())
            .collect();
        for floating in self.workspace_index.contained_on_surface(surface) {
            if self
                .workspace
                .contained_floating(floating)
                .and_then(|record| self.workspace.root(record.root))
                .is_some_and(|root| !self.workspace.node_has_visible_content(root.node))
            {
                continue;
            }
            if !published_occlusions.contains(&floating) {
                return Err(SceneBuildError::MissingDropOcclusion { surface, floating });
            }
        }
        for occlusion in &ready.facts.drop_occlusions {
            let floating = occlusion.floating();
            if !self
                .workspace_index
                .contained_belongs_to_surface(surface, floating)
                || self
                    .workspace
                    .contained_floating(floating)
                    .and_then(|record| self.workspace.root(record.root))
                    .is_none_or(|root| !self.workspace.node_has_visible_content(root.node))
            {
                return Err(SceneBuildError::InvalidDropOcclusion { surface, floating });
            }
            let expected = self
                .workspace_index
                .contained_layer(floating)
                .ok_or(SceneBuildError::InvalidDropOcclusion { surface, floating })?;
            if occlusion.layer() != expected {
                return Err(SceneBuildError::DropOcclusionLayerMismatch {
                    surface,
                    floating,
                    expected,
                    actual: occlusion.layer(),
                });
            }
            let region = occlusion.region().rect();
            if !rect_has_area(region) {
                return Err(SceneBuildError::EmptyDropOcclusionRegion { surface, floating });
            }
            let expected = self
                .workspace_index
                .contained_rect(floating)
                .ok_or(SceneBuildError::InvalidDropOcclusion { surface, floating })?;
            if region != expected {
                return Err(SceneBuildError::DropOcclusionGeometryMismatch {
                    surface,
                    floating,
                    expected,
                    actual: region,
                });
            }
        }
        for target in &ready.facts.drop_targets {
            validate_drop_target(ready, target, self.workspace, &self.workspace_index)?;
        }
        let mut target_coverage = MagneticTargetCoverage::new(drop_target_index);
        for cluster in &ready.facts.drop_guide_clusters {
            validate_drop_guide_cluster(
                ready,
                &mut target_coverage,
                cluster,
                self.workspace,
                &self.workspace_index,
            )?;
        }
        Ok(target_coverage)
    }

    fn finish_validation(
        &self,
        mut ready: PresentationPlan,
        record_order: SemanticRecordOrder,
        target_order: DropTargetOrder,
    ) -> Result<PresentationPlan, SceneBuildError> {
        validate_contained_minimums(&ready, &self.workspace_index)?;
        canonicalize_ready(&mut ready, self.policy, record_order, target_order);
        if ready.measurement_ticket().is_some() {
            ready.link_surface_program()?;
        }
        Ok(ready)
    }
}

fn validate_compiled_target_coverage(
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
fn validate_magnetic_target_field(
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
fn magnetic_targets_are_canonical(
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

fn validate_popup_plane(ready: &PresentationPlan) -> Result<(), SceneBuildError> {
    let backdrops = &ready.facts.tab_list_menu_backdrop_records;
    match ready.facts.popup.visual_for_surface(ready.surface) {
        None => {
            if ready.facts.popup_plane_bounds.is_some()
                || !backdrops.is_empty()
                || !ready.facts.tab_list_menu_records.is_empty()
            {
                return Err(SceneBuildError::PopupPlaneRecordSetMismatch {
                    surface: ready.surface,
                });
            }
        }
        Some(PopupPlaneRequirement::Active {
            revision,
            session,
            owner,
        }) => {
            let Some(popup_plane_bounds) = ready.facts.popup_plane_bounds else {
                return Err(SceneBuildError::PopupPlaneRecordSetMismatch {
                    surface: ready.surface,
                });
            };
            if backdrops.len() != 1
                || backdrops[0].session() != session
                || backdrops[0].revision() != revision
                || !rect_has_area(popup_plane_bounds)
                || backdrops[0].bounds() != popup_plane_bounds
            {
                return Err(SceneBuildError::PopupPlaneRecordSetMismatch {
                    surface: ready.surface,
                });
            }
            if ready.facts.tab_list_menu_records.len() != 1
                || ready.facts.tab_list_menu_records[0].session() != session
                || ready.facts.tab_list_menu_records[0].bar() != owner.bar()
            {
                return Err(SceneBuildError::PopupPlaneRecordSetMismatch {
                    surface: ready.surface,
                });
            }
        }
        Some(PopupPlaneRequirement::Inactive { .. }) => {
            unreachable!("inactive popup has no visual")
        }
    }
    Ok(())
}

fn validate_semantic_uniqueness(
    ready: &PresentationPlan,
) -> Result<(SemanticRecordOrder, DropTargetCatalog<'_>), SceneBuildError> {
    fn stable_duplicate<T: Copy + Ord>(values: impl IntoIterator<Item = T>) -> Option<T> {
        let mut values = values.into_iter().collect::<Vec<_>>();
        values.sort_unstable();
        values
            .windows(2)
            .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
    }

    let record_order = SemanticRecordOrder {
        panes: RecordOrder::new(ready.content.pane_records.iter().map(PaneRecord::id))
            .map_err(|id| SceneBuildError::DuplicatePane { id })?,
        bars: RecordOrder::new(
            ready
                .facts
                .tab_bar_records
                .iter()
                .map(|record| *record.id()),
        )
        .map_err(|id| SceneBuildError::DuplicateTabBar { id })?,
        tabs: RecordOrder::new(ready.facts.tab_records.iter().map(|record| *record.id()))
            .map_err(|id| SceneBuildError::DuplicateTab { id })?,
        gaps: RecordOrder::new(
            ready
                .facts
                .splitter_gap_records
                .iter()
                .map(SplitterGapRecord::id),
        )
        .map_err(|id| SceneBuildError::DuplicateSplitterGap { id })?,
        splitters: RecordOrder::new(
            ready
                .facts
                .splitter_records
                .iter()
                .map(|record| *record.id()),
        )
        .map_err(|id| SceneBuildError::DuplicateSplitter { id })?,
    };
    if let Some(id) = stable_duplicate(
        ready
            .facts
            .splitter_junction_records
            .iter()
            .map(SplitterJunctionRecord::id),
    ) {
        return Err(SceneBuildError::DuplicateSplitterJunction { id });
    }
    if let Some(floating) = stable_duplicate(
        ready
            .facts
            .contained_records
            .iter()
            .map(ContainedRecord::floating),
    ) {
        return Err(SceneBuildError::DuplicateContainedRecord { floating });
    }
    if let Some(root) = stable_duplicate(
        ready
            .facts
            .presentation_menu_anchor_records
            .iter()
            .map(|record| record.root()),
    ) {
        return Err(SceneBuildError::DuplicatePresentationMenuAnchor { root });
    }
    if let Some(floating) = stable_duplicate(
        ready
            .facts
            .contained_minimums
            .iter()
            .map(|measurement| measurement.floating()),
    ) {
        return Err(SceneBuildError::DuplicateContainedMinimumMeasurement { floating });
    }
    if let Some(floating) = stable_duplicate(
        ready
            .facts
            .drop_occlusions
            .iter()
            .map(|record| record.floating()),
    ) {
        return Err(SceneBuildError::DuplicateDropOcclusion { floating });
    }
    if let Some(id) = stable_duplicate(
        ready
            .facts
            .drop_guide_clusters
            .iter()
            .map(DropGuideClusterRecord::id),
    ) {
        return Err(SceneBuildError::DuplicateDropGuideCluster { id });
    }
    let target_catalog = DropTargetCatalog::new(
        &ready.facts.drop_targets,
        ready.facts.surface_background.as_deref(),
    )
    .map_err(|id| SceneBuildError::DuplicateDropTarget { id })?;
    let guide_targets = ready
        .facts
        .drop_guide_clusters
        .iter()
        .flat_map(|cluster| cluster.targets().map(|(_, guide_target)| guide_target.id()));
    if let Some(id) = stable_duplicate(guide_targets) {
        return Err(SceneBuildError::DuplicateDropTarget { id });
    }
    Ok((record_order, target_catalog))
}

fn validate_contained_minimums(
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

fn validate_surface_background(
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

#[cfg(test)]
fn validate_ready_semantics(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
    policy: &DockPolicySnapshot,
) -> Result<(), SceneBuildError> {
    validate_contained_records(ready, workspace, workspace_index)?;
    let expected_panes = expected_pane_records(ready, workspace, workspace_index)?;
    validate_pane_and_tab_records(
        ready,
        record_order,
        workspace,
        workspace_index,
        policy,
        &expected_panes,
    )?;
    validate_presentation_menu_anchors(ready, workspace)?;
    validate_splitter_records(ready, record_order, workspace, workspace_index, policy)?;
    Ok(())
}

fn validate_contained_records(
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

fn validate_presentation_menu_anchors(
    ready: &PresentationPlan,
    workspace: &Workspace,
) -> Result<(), SceneBuildError> {
    let presentation =
        workspace
            .surface(ready.surface)
            .ok_or(SceneBuildError::UnexpectedSceneSurface {
                surface: ready.surface,
            })?;
    let mut expected = BTreeMap::new();
    let mut independent_controls = None;
    if let Some(root) = presentation.main_root {
        if canonical_presentation_menu_tab_bar(ready, workspace, root)?.is_some() {
            expected.insert(root, None);
        } else if let Some(layout) = ready.layout_facts() {
            if let Some(facts) = layout.root(root) {
                independent_controls = crate::presentation::compiler::root_controls_layout(
                    workspace,
                    root,
                    facts.bounds(),
                    layout.config(),
                )
                .map_err(|_| SceneBuildError::InvalidPresentationMenuAnchor {
                    surface: ready.surface,
                    root,
                })?;
                if independent_controls.is_some() {
                    expected.insert(root, Some(PresentationMenuAnchorHost::RootControls));
                }
            }
        }
    }
    expected.extend(ready.facts.contained_records.iter().map(|record| {
        (
            record.root(),
            Some(PresentationMenuAnchorHost::ContainedTitle(
                record.floating(),
            )),
        )
    }));

    if ready.facts.presentation_menu_anchor_records.len() != expected.len() {
        return Err(SceneBuildError::PresentationMenuAnchorRecordSetMismatch {
            surface: ready.surface,
        });
    }
    for anchor in &ready.facts.presentation_menu_anchor_records {
        let root = anchor.root();
        let Some(expected_host) = expected.get(&root).copied() else {
            return Err(SceneBuildError::InvalidPresentationMenuAnchor {
                surface: ready.surface,
                root,
            });
        };
        if expected_host.is_some_and(|expected_host| expected_host != anchor.host()) {
            return Err(SceneBuildError::InvalidPresentationMenuAnchor {
                surface: ready.surface,
                root,
            });
        }
        let valid = match anchor.host() {
            PresentationMenuAnchorHost::TabBar(bar_id) => ready
                .facts
                .tab_bar_records
                .iter()
                .find(|bar| *bar.id() == bar_id)
                .is_some_and(|bar| {
                    if bar_id.root != root || anchor.layer() != bar.layer() {
                        return false;
                    }
                    match (anchor.ready_bounds(), anchor.ready_hit()) {
                        (None, None) => true,
                        (Some(bounds), Some(hit)) => {
                            hit.rect() == bounds
                                && rect_has_area(bounds)
                                && rect_contains(bar.bounds(), bounds)
                                && !rects_overlap_with_area(bar.viewport(), bounds)
                                && bar
                                    .group_grip_bounds()
                                    .is_none_or(|grip| !rects_overlap_with_area(grip, bounds))
                                && bar.group_drag().is_none_or(|group| {
                                    group.regions().all(|region| {
                                        !rects_overlap_with_area(region.bounds(), bounds)
                                    })
                                })
                                && ready
                                    .facts
                                    .tab_strip_control_records
                                    .iter()
                                    .filter(|control| control.id().bar() == bar_id)
                                    .all(|control| {
                                        !rects_overlap_with_area(control.bounds(), bounds)
                                    })
                        }
                        (None, Some(_)) | (Some(_), None) => false,
                    }
                }),
            PresentationMenuAnchorHost::RootControls => {
                independent_controls.as_ref().is_some_and(|controls| {
                    presentation.main_root == Some(root)
                        && anchor.reserved_bounds() == Some(controls.reserved)
                        && anchor.ready_bounds() == controls.menu
                        && anchor.ready_hit().map(|hit| hit.rect()) == controls.menu
                        && ready
                            .layout_facts()
                            .and_then(|facts| facts.root(root))
                            .is_some_and(|facts| facts.content_bounds() == controls.content)
                        && ready
                            .content
                            .pane_records
                            .iter()
                            .filter(|pane| pane.id().root == root)
                            .all(|pane| !rects_overlap_with_area(controls.reserved, pane.bounds()))
                        && rect_contains(ready.bounds(), controls.reserved)
                        && workspace.presentation_for_root(root).is_some_and(|owner| {
                            matches!(owner, RootPresentationOwner::Main { .. })
                        })
                })
            }
            PresentationMenuAnchorHost::ContainedTitle(floating) => ready
                .facts
                .contained_records
                .iter()
                .find(|contained| contained.floating() == floating)
                .is_some_and(|contained| {
                    if contained.root() != root || anchor.layer() != contained.layer() {
                        return false;
                    }
                    match (anchor.ready_bounds(), anchor.ready_hit()) {
                        (None, None) => true,
                        (Some(bounds), Some(hit)) => {
                            hit.rect() == bounds
                                && rect_has_area(bounds)
                                && rect_contains(contained.title_bounds(), bounds)
                                && !rects_overlap_with_area(
                                    contained.title_drag_hit().rect(),
                                    bounds,
                                )
                                && contained
                                    .close_bounds()
                                    .is_none_or(|close| !rects_overlap_with_area(close, bounds))
                        }
                        (None, Some(_)) | (Some(_), None) => false,
                    }
                }),
        };
        if !valid
            || (anchor.host() != PresentationMenuAnchorHost::RootControls
                && anchor.reserved_bounds().is_some())
        {
            return Err(SceneBuildError::InvalidPresentationMenuAnchor {
                surface: ready.surface,
                root,
            });
        }
    }
    Ok(())
}

fn canonical_presentation_menu_tab_bar(
    ready: &PresentationPlan,
    workspace: &Workspace,
    root: RootId,
) -> Result<Option<TabBarSceneId>, SceneBuildError> {
    let root_record = workspace
        .root(root)
        .ok_or(SceneBuildError::CompiledRootUnavailable {
            surface: ready.surface,
            root,
        })?;
    let is_compiled = |tabs| {
        ready
            .facts
            .tab_bar_records
            .iter()
            .any(|bar| *bar.id() == TabBarSceneId { root, tabs })
    };
    if let Some(central) = root_record.central
        && is_compiled(central)
    {
        return Ok(Some(TabBarSceneId {
            root,
            tabs: central,
        }));
    }

    let mut pending = vec![root_record.node];
    let mut visited = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !visited.insert(node) {
            return Err(SceneBuildError::CompiledRootUnavailable {
                surface: ready.surface,
                root,
            });
        }
        match workspace.node(node) {
            Some(Node::Tabs { .. }) if is_compiled(node) => {
                return Ok(Some(TabBarSceneId { root, tabs: node }));
            }
            Some(Node::Pane { .. }) => {}
            Some(Node::Tabs { .. }) => {
                if let Some(child) = workspace
                    .visible_tab_selection(node)
                    .and_then(TabKey::layout)
                {
                    pending.push(child);
                }
            }
            Some(Node::Split { children, .. } | Node::Grid { children, .. }) => {
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
    Ok(None)
}

#[cfg(test)]
fn expected_pane_records(
    ready: &PresentationPlan,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<BTreeMap<PaneSceneId, (ItemId, SceneLayerKey)>, SceneBuildError> {
    let mut roots = Vec::new();
    if let Some(root) = workspace
        .surface(ready.surface)
        .and_then(|presentation| presentation.main_root)
    {
        roots.push(root);
    }
    roots.extend(
        ready
            .facts
            .contained_records
            .iter()
            .filter(|record| rect_has_area(record.content_bounds()))
            .map(ContainedRecord::root),
    );
    let mut expected = BTreeMap::new();
    for root in roots {
        let layer =
            workspace_index
                .root_layer(root)
                .ok_or(SceneBuildError::CompiledRootUnavailable {
                    surface: ready.surface,
                    root,
                })?;
        let mut pending = vec![workspace_index.root_node(root).ok_or(
            SceneBuildError::CompiledRootUnavailable {
                surface: ready.surface,
                root,
            },
        )?];
        while let Some(node) = pending.pop() {
            match workspace.node(node) {
                Some(Node::Pane { item }) => {
                    if !workspace.item_is_hidden(*item) {
                        expected.insert(PaneSceneId { root, item: *item }, (*item, layer));
                    }
                }
                Some(Node::Tabs { .. }) => {
                    if let Some(selected) = workspace.visible_tab_selection(node) {
                        if let Some(item) = selected.item() {
                            expected.insert(PaneSceneId { root, item }, (item, layer));
                        }
                        if let Some(child) = selected.layout() {
                            pending.push(child);
                        }
                    }
                }
                Some(Node::Split { children, .. } | Node::Grid { children, .. }) => {
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
    Ok(expected)
}

#[cfg(test)]
fn validate_pane_and_tab_records(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
    policy: &DockPolicySnapshot,
    expected_panes: &BTreeMap<PaneSceneId, (ItemId, SceneLayerKey)>,
) -> Result<(), SceneBuildError> {
    let panes = &record_order.panes;
    if panes.entries().len() != expected_panes.len() {
        return Err(SceneBuildError::PaneRecordSetMismatch {
            surface: ready.surface,
        });
    }
    for (id, (item, layer)) in expected_panes {
        let Some(record) = panes.get(&ready.content.pane_records, *id) else {
            return Err(SceneBuildError::PaneRecordSetMismatch {
                surface: ready.surface,
            });
        };
        if record.item() != *item
            || record.layer() != *layer
            || !rect_contains(ready.facts.bounds, record.bounds())
            || !rect_contains(record.bounds(), record.content_bounds())
        {
            return Err(SceneBuildError::InvalidPaneRecord {
                surface: ready.surface,
                id: *id,
            });
        }
    }

    validate_tab_records(ready, record_order, workspace, workspace_index, policy)
}

fn validate_tab_records(
    ready: &PresentationPlan,
    record_order: &SemanticRecordOrder,
    workspace: &Workspace,
    workspace_index: &SceneWorkspaceIndex,
    policy: &DockPolicySnapshot,
) -> Result<(), SceneBuildError> {
    let panes =
        RecordOrder::new(ready.facts.tabs_records.iter().map(TabsRecord::id)).map_err(|id| {
            SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id,
            }
        })?;
    let bars = &record_order.bars;
    // The caller establishes membership, owner and selection through either
    // the expected roster or the sealed compiler. Policy needs only the
    // selected item's stable rule, not another complete root fingerprint.
    let pane_policy = |pane: &TabsRecord| {
        let declaration = policy.tab_bar_declaration(
            ready.surface,
            ready
                .selected_in_tabs(pane.id().tabs)
                .and_then(TabKey::item)
                .map(crate::policy::DockTargetRuleKey::Item)
                .or(Some(crate::policy::DockTargetRuleKey::Root(pane.id().root))),
        );
        let item_count = workspace.visible_tab_keys(pane.id().tabs).len();
        let revealed = declaration.visibility() == TabBarVisibility::AutoHideSingle
            && item_count == 1
            && bars
                .get(
                    &ready.facts.tab_bar_records,
                    TabBarSceneId {
                        root: pane.id().root,
                        tabs: pane.id().tabs,
                    },
                )
                .is_some();
        declaration.resolved(item_count, revealed)
    };
    let mut controls_by_bar = BTreeMap::<TabBarSceneId, BTreeMap<TabStripControlId, _>>::new();
    for control in &ready.facts.tab_strip_control_records {
        let bar = control.id().bar();
        if controls_by_bar
            .entry(bar)
            .or_default()
            .insert(control.id(), control)
            .is_some()
        {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: bar,
            });
        }
    }
    let mut menus_by_bar = BTreeMap::new();
    for menu in &ready.facts.tab_list_menu_records {
        if menus_by_bar.insert(menu.bar(), menu).is_some() {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: menu.bar(),
            });
        }
    }
    if ready.facts.tab_list_menu_records.len() > 1 {
        let id = ready.facts.tab_list_menu_records[0].bar();
        return Err(SceneBuildError::InvalidTabBarRecord {
            surface: ready.surface,
            id,
        });
    }
    let bar_exists = |id: &TabBarSceneId| bars.get(&ready.facts.tab_bar_records, *id).is_some();
    let foreign_control = controls_by_bar.iter().find_map(|(id, controls)| {
        (!bar_exists(id)
            && (controls.len() != 1
                || !controls.contains_key(&TabStripControlId::RevealTabBar(*id))
                || panes
                    .get(
                        &ready.facts.tabs_records,
                        TabBarSceneId {
                            root: id.root,
                            tabs: id.tabs,
                        },
                    )
                    .is_none()))
        .then_some(*id)
    });
    if let Some(id) =
        foreign_control.or_else(|| menus_by_bar.keys().find(|id| !bar_exists(id)).copied())
    {
        return Err(SceneBuildError::InvalidTabBarRecord {
            surface: ready.surface,
            id,
        });
    }
    let expected_bars = panes
        .iter(&ready.facts.tabs_records)
        .filter(|(_, pane)| {
            rect_has_area(pane.bounds())
                && pane_policy(pane).visibility() == TabBarVisibility::Visible
        })
        .count();
    if bars.entries().len() != expected_bars {
        return Err(SceneBuildError::TabBarRecordSetMismatch {
            surface: ready.surface,
        });
    }
    for (id, pane) in panes.iter(&ready.facts.tabs_records) {
        let bar_id = TabBarSceneId {
            root: id.root,
            tabs: id.tabs,
        };
        let tab_bar_policy = pane_policy(pane);
        if !rect_has_area(pane.bounds()) || tab_bar_policy.visibility() == TabBarVisibility::Hidden
        {
            if bar_exists(&bar_id) {
                return Err(SceneBuildError::InvalidTabBarRecord {
                    surface: ready.surface,
                    id: bar_id,
                });
            }
            if let Some(controls) = controls_by_bar.get(&bar_id) {
                let control = controls.get(&TabStripControlId::RevealTabBar(bar_id));
                let declaration = policy.tab_bar_declaration(
                    ready.surface,
                    ready
                        .selected_in_tabs(pane.id().tabs)
                        .and_then(TabKey::item)
                        .map(crate::policy::DockTargetRuleKey::Item),
                );
                let expected_bounds = ready.layout_facts().and_then(|facts| {
                    crate::presentation::compiler::reveal_tab_bar_bounds(
                        pane.bounds(),
                        facts.config(),
                    )
                    .ok()
                    .flatten()
                });
                let valid = declaration.visibility() == TabBarVisibility::AutoHideSingle
                    && workspace.visible_tab_keys(id.tabs).len() == 1
                    && controls.len() == 1
                    && control.is_some_and(|control| {
                        expected_bounds == Some(control.bounds())
                            && rect_has_area(control.bounds())
                            && rect_contains(pane.bounds(), control.bounds())
                            && control.hit().rect() == control.bounds()
                            && control.layer() == pane.layer()
                            && control.enabled()
                                == (declaration.interaction() == TabBarInteraction::Enabled)
                    });
                if !valid {
                    return Err(SceneBuildError::InvalidTabBarRecord {
                        surface: ready.surface,
                        id: bar_id,
                    });
                }
            }
            continue;
        }
        let Some(bar) = bars.get(&ready.facts.tab_bar_records, bar_id) else {
            return Err(SceneBuildError::TabBarRecordSetMismatch {
                surface: ready.surface,
            });
        };
        if bar.layer() != pane.layer()
            || bar.interaction() != tab_bar_policy.interaction()
            || !rect_has_area(bar.bounds())
            || !rect_contains(pane.bounds(), bar.bounds())
            || !rect_contains(bar.bounds(), bar.viewport())
            || !bar.scroll_offset().is_finite()
            || !bar.maximum_scroll_offset().is_finite()
            || bar.scroll_offset() < 0.0
            || bar.maximum_scroll_offset() < 0.0
            || bar.scroll_offset() > bar.maximum_scroll_offset()
        {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: bar_id,
            });
        }
    }

    let tabs = record_order.tabs.entries();
    if tabs.iter().any(|(id, _)| {
        !bar_exists(&TabBarSceneId {
            root: id.root,
            tabs: id.tabs,
        })
    }) {
        return Err(SceneBuildError::TabRecordSetMismatch {
            surface: ready.surface,
        });
    }
    for (bar_id, bar) in bars.iter(&ready.facts.tab_bar_records) {
        let Some(Node::Tabs { .. }) = workspace.node(bar_id.tabs) else {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: bar_id,
            });
        };
        let items = workspace.visible_tab_keys(bar_id.tabs);
        let selected = workspace.visible_tab_selection(bar_id.tabs);
        let group_grip_matches = match bar.group_grip_bounds() {
            None => true,
            Some(grip) => {
                !items.is_empty()
                    && rect_has_area(grip)
                    && grip.x() == bar.bounds().x()
                    && grip.y() == bar.bounds().y()
                    && grip.height() == bar.bounds().height()
                    && rect_contains(bar.bounds(), grip)
                    && !rects_overlap_with_area(grip, bar.viewport())
            }
        };
        let group_interaction_matches = match bar.interaction() {
            TabBarInteraction::Disabled => bar.group_drag().is_none(),
            TabBarInteraction::Enabled => {
                let expected_trailing_start = bar
                    .after_tabs_bounds()
                    .map(|bounds| bounds.max().x())
                    .or_else(|| {
                        bar.members()
                            .last()
                            .map(|member| member.full_bounds().max().x())
                    })
                    .unwrap_or(bar.viewport().x())
                    .min(bar.viewport().max().x());
                let expected_trailing = !items.is_empty()
                    && bar.maximum_scroll_offset() == 0.0
                    && expected_trailing_start < bar.viewport().max().x();
                let leading = bar.group_drag().and_then(|group| group.leading_grip());
                let leading_matches = match (bar.group_grip_bounds(), leading) {
                    (None, None) => true,
                    (Some(grip), Some(leading)) => {
                        leading.kind() == TabGroupDragRegionKind::LeadingGrip
                            && leading.bounds() == grip
                            && leading.hit().rect() == grip
                            && rect_has_area(leading.hit().rect())
                    }
                    _ => false,
                };
                let trailing_matches = match (
                    expected_trailing,
                    bar.group_drag().and_then(|group| group.trailing_empty()),
                ) {
                    (false, None) => true,
                    (true, Some(trailing)) => {
                        trailing.kind() == TabGroupDragRegionKind::TrailingEmpty
                            && trailing.bounds().x() == expected_trailing_start
                            && trailing.bounds().y() == bar.viewport().y()
                            && trailing.bounds().max().x() == bar.viewport().max().x()
                            && trailing.bounds().height() == bar.viewport().height()
                            && trailing.hit().rect() == trailing.bounds()
                            && rect_has_area(trailing.bounds())
                    }
                    _ => false,
                };
                leading_matches && trailing_matches
            }
        };
        let group_menu_matches = bar.group_menu_bounds().is_none_or(|bounds| {
            !items.is_empty()
                && rect_has_area(bounds)
                && rect_contains(bar.bounds(), bounds)
                && !rects_overlap_with_area(bounds, bar.viewport())
                && [
                    bar.group_grip_bounds(),
                    bar.leading_reserved_bounds(),
                    bar.trailing_reserved_bounds(),
                ]
                .into_iter()
                .flatten()
                .all(|other| !rects_overlap_with_area(bounds, other))
                && bar.group_drag().is_none_or(|group| {
                    group
                        .regions()
                        .all(|region| !rects_overlap_with_area(bounds, region.bounds()))
                })
                && ready
                    .facts
                    .tab_strip_control_records
                    .iter()
                    .filter(|control| control.id().bar() == bar_id)
                    .all(|control| !rects_overlap_with_area(bounds, control.bounds()))
                && ready
                    .facts
                    .presentation_menu_anchor_records
                    .iter()
                    .filter(|anchor| anchor.root() == bar_id.root)
                    .filter_map(|anchor| anchor.ready_bounds())
                    .all(|other| !rects_overlap_with_area(bounds, other))
        });
        if !group_grip_matches || !group_interaction_matches || !group_menu_matches {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: bar_id,
            });
        }
        let bar_key = (bar_id.root, bar_id.tabs);
        let start = tabs.partition_point(|(id, _)| (id.root, id.tabs) < bar_key);
        let end = tabs.partition_point(|(id, _)| (id.root, id.tabs) <= bar_key);
        let visible = &tabs[start..end];
        let visible_record = |item| {
            visible
                .binary_search_by_key(&item, |(id, _)| id.tab)
                .ok()
                .map(|row| &ready.facts.tab_records[visible[row].1])
        };
        let expected_hidden = items
            .iter()
            .copied()
            .filter(|item| visible_record(*item).is_none());
        if !bar.hidden_tabs().iter().copied().eq(expected_hidden) {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: bar_id,
            });
        }
        let members_match = bar.members().len() == items.len()
            && bar.members().iter().enumerate().all(|(ordinal, member)| {
                let tab = member.tab();
                let record = visible_record(tab.tab).filter(|record| *record.id() == tab);
                let visibility_matches = match (member.visibility(), record) {
                    (TabStripMemberVisibility::Visible, Some(record)) => {
                        member.full_bounds() == record.full_bounds()
                            && record.full_bounds() == record.visible_bounds()
                    }
                    (TabStripMemberVisibility::PartiallyVisible, Some(record)) => {
                        member.full_bounds() == record.full_bounds()
                            && record.full_bounds() != record.visible_bounds()
                    }
                    (TabStripMemberVisibility::PartiallyVisible, None) => true,
                    (TabStripMemberVisibility::Hidden, None) => true,
                    (TabStripMemberVisibility::Visible, None)
                    | (TabStripMemberVisibility::Hidden, Some(_)) => false,
                };
                let full = member.full_bounds();
                let preceding_edge_matches = if ordinal == 0 {
                    full.x() == bar.viewport().x() - bar.scroll_offset()
                } else {
                    bar.members()[ordinal - 1].full_bounds().max().x() == full.x()
                };
                member.ordinal() == ordinal
                    && tab.root == bar_id.root
                    && tab.tabs == bar_id.tabs
                    && items.get(ordinal).copied() == Some(tab.tab)
                    && rect_has_area(full)
                    && full.y() == bar.bounds().y()
                    && full.height() == bar.bounds().height()
                    && preceding_edge_matches
                    && visibility_matches
            });
        if !members_match {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id: bar_id,
            });
        }

        validate_tab_strip_controls_and_menu(
            ready,
            bar,
            &items,
            selected,
            controls_by_bar.get(&bar_id),
            menus_by_bar.get(&bar_id).copied(),
        )?;
        let mut invalid_tab: Option<(usize, TabSceneId)> = None;
        for &(_, row) in visible {
            let record = &ready.facts.tab_records[row];
            let id = *record.id();
            let ordinal_matches = items.get(record.ordinal()).copied() == Some(id.tab);
            let close_allowed = crate::presentation::compiler::tab_close_capability(
                workspace, policy, id.root, id.tab,
            )
            .allows_close();
            let close_visual_matches = match (close_allowed, record.close_visual_bounds()) {
                (false, None) => true,
                (true, Some(close)) => {
                    rect_has_area(close) && rect_contains(record.visible_bounds(), close)
                }
                // A clipped right edge may hide close chrome while leaving
                // enough visible tab body for selection and dragging.
                (true, None) => record.visible_bounds().max().x() < record.full_bounds().max().x(),
                _ => false,
            };
            let interaction_geometry_matches = match bar.interaction() {
                TabBarInteraction::Enabled => {
                    rect_has_area(record.drag_hit().rect())
                        && rect_contains(record.visible_bounds(), record.drag_hit().rect())
                        && record.close_bounds() == record.close_visual_bounds()
                        && record.close_bounds().is_none_or(|close| {
                            !rects_overlap_with_area(close, record.drag_hit().rect())
                        })
                }
                TabBarInteraction::Disabled => {
                    !rect_has_area(record.drag_hit().rect()) && record.close_bounds().is_none()
                }
            };
            let geometry_matches = rect_has_area(record.visible_bounds())
                && rect_is_exact_intersection(
                    record.visible_bounds(),
                    record.full_bounds(),
                    bar.viewport(),
                )
                && rect_contains(record.full_bounds(), record.text_layout_bounds())
                && rect_contains(record.visible_bounds(), record.text_bounds())
                && close_visual_matches
                && interaction_geometry_matches;
            if id.root != bar_id.root
                || id.tabs != bar_id.tabs
                || !ordinal_matches
                || ready.tab_selected(record) != (selected == Some(id.tab))
                || record.layer() != bar.layer()
                || !geometry_matches
                || !workspace_index.semantic_node_exists(ready.surface, id.root, id.tabs)
            {
                // The directory is sorted by ID, but diagnostics retain the
                // earliest invalid record in the original bar input order.
                if invalid_tab.is_none_or(|(first, _)| row < first) {
                    invalid_tab = Some((row, id));
                }
            }
        }
        if let Some((_, id)) = invalid_tab {
            return Err(SceneBuildError::InvalidTabRecord {
                surface: ready.surface,
                id,
            });
        }
    }
    Ok(())
}

fn validate_tab_strip_controls_and_menu(
    ready: &PresentationPlan,
    bar: &TabBarRecord,
    items: &[TabKey],
    selected: Option<TabKey>,
    controls: Option<&BTreeMap<TabStripControlId, &TabStripControlRecord>>,
    menu: Option<&TabListMenuRecord>,
) -> Result<(), SceneBuildError> {
    let id = *bar.id();
    let expected_control_ids = TabStripControlId::for_bar(id);
    if let Some(controls) = controls {
        if controls.is_empty()
            || controls.len() > expected_control_ids.len()
            || controls
                .keys()
                .any(|control| !expected_control_ids.contains(control))
        {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id,
            });
        }
        let records = controls.values().copied().collect::<Vec<_>>();
        let valid_geometry = records.iter().all(|control| {
            rect_has_area(control.bounds())
                && control.hit().rect() == control.bounds()
                && rect_contains(bar.bounds(), control.bounds())
                && control.layer() == bar.layer()
        }) && records.iter().enumerate().all(|(index, control)| {
            records[index + 1..]
                .iter()
                .all(|other| !rects_overlap_with_area(control.bounds(), other.bounds()))
        });
        let interaction_enabled = bar.interaction() == TabBarInteraction::Enabled;
        let valid_enablement = records.iter().all(|control| {
            let expected = match control.id() {
                TabStripControlId::ScrollBackward(_) => {
                    interaction_enabled && bar.scroll_offset() > 0.0
                }
                TabStripControlId::ScrollForward(_) => {
                    interaction_enabled && bar.scroll_offset() < bar.maximum_scroll_offset()
                }
                TabStripControlId::TabListMenu(_) => {
                    interaction_enabled && bar.menu_geometry_availability().is_available()
                }
                TabStripControlId::RevealTabBar(_) => false,
            };
            control.enabled() == expected
        });
        let menu_control_present = controls.contains_key(&TabStripControlId::TabListMenu(id));
        if !valid_geometry || !valid_enablement || bar.maximum_scroll_offset() <= 0.0 {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id,
            });
        }
        if (bar.menu_geometry_availability().is_available() && !menu_control_present)
            || (menu.is_some() && !menu_control_present)
        {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id,
            });
        }
    } else if menu.is_some() || bar.menu_geometry_availability().is_available() {
        return Err(SceneBuildError::InvalidTabBarRecord {
            surface: ready.surface,
            id,
        });
    }

    let Some(menu) = menu else {
        return Ok(());
    };
    let Some(popup_plane_bounds) = ready.facts.popup_plane_bounds else {
        return Err(SceneBuildError::InvalidTabBarRecord {
            surface: ready.surface,
            id,
        });
    };
    let menu_control = controls
        .and_then(|controls| controls.get(&TabStripControlId::TabListMenu(id)))
        .copied();
    let valid_menu = bar.interaction() == TabBarInteraction::Enabled
        && bar.menu_geometry_availability().is_available()
        && menu.layer() == bar.layer()
        && menu.session().key() == TabStripStateKey::new(ready.surface, id)
        && menu_control.is_some_and(|control| rect_contains(popup_plane_bounds, control.bounds()))
        && rect_has_area(menu.bounds())
        && rect_contains(popup_plane_bounds, menu.bounds())
        && rect_has_area(menu.viewport())
        && rect_contains(menu.bounds(), menu.viewport())
        && menu.scroll_offset().is_finite()
        && menu.maximum_scroll_offset().is_finite()
        && menu.scroll_offset() >= 0.0
        && menu.maximum_scroll_offset() >= 0.0
        && menu.scroll_offset() <= menu.maximum_scroll_offset()
        && menu.rows().len() == items.len();
    if !valid_menu {
        return Err(SceneBuildError::InvalidTabBarRecord {
            surface: ready.surface,
            id,
        });
    }

    let mut focused = 0_usize;
    for (ordinal, row) in menu.rows().iter().copied().enumerate() {
        let tab = row.tab();
        focused += usize::from(row.focused());
        let hit_matches = match row.hit() {
            Some(hit) => {
                rect_has_area(hit.rect())
                    && rect_contains(row.bounds(), hit.rect())
                    && rect_contains(menu.viewport(), hit.rect())
                    && rect_is_exact_intersection(hit.rect(), row.bounds(), menu.viewport())
            }
            None => !rects_overlap_with_area(row.bounds(), menu.viewport()),
        };
        if row.ordinal() != ordinal
            || items.get(ordinal).copied() != Some(tab.tab)
            || tab.root != id.root
            || tab.tabs != id.tabs
            || !rect_has_area(row.bounds())
            || row.bounds().x() != menu.viewport().x()
            || row.bounds().width() != menu.viewport().width()
            || row.selected() != (selected == Some(tab.tab))
            || !hit_matches
        {
            return Err(SceneBuildError::InvalidTabBarRecord {
                surface: ready.surface,
                id,
            });
        }
    }
    if focused != 1 {
        return Err(SceneBuildError::InvalidTabBarRecord {
            surface: ready.surface,
            id,
        });
    }
    Ok(())
}

pub(super) fn splitter_extent_satisfies_constraints(
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
fn validate_splitter_records(
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

fn validate_splitter_geometry(
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
        root.projection().nodes.get(&node).ok_or_else(invalid)?.rect,
        root.content_bounds(),
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

fn validate_semantic_layer(
    surface: SurfaceId,
    root: RootId,
    actual: SceneLayerKey,
    workspace_index: &SceneWorkspaceIndex,
) -> Result<(), SceneBuildError> {
    let expected =
        workspace_index
            .root_layer(root)
            .ok_or(SceneBuildError::SemanticLayerMismatch {
                surface,
                root,
                expected: SceneLayerKey::surface_base(),
                actual,
            })?;
    if actual != expected {
        return Err(SceneBuildError::SemanticLayerMismatch {
            surface,
            root,
            expected,
            actual,
        });
    }
    Ok(())
}

fn canonicalize_ready(
    ready: &mut PresentationPlan,
    policy: &DockPolicySnapshot,
    record_order: SemanticRecordOrder,
    target_order: DropTargetOrder,
) {
    record_order
        .panes
        .apply(&mut ready.content_mut().pane_records);
    let facts = ready.facts_mut();
    facts.tabs_records.sort_unstable_by_key(TabsRecord::id);
    record_order.tabs.apply(&mut facts.tab_records);
    record_order.bars.apply(&mut facts.tab_bar_records);
    facts
        .tab_strip_control_records
        .sort_unstable_by_key(|record| record.id());
    facts
        .tab_list_menu_records
        .sort_unstable_by_key(TabListMenuRecord::session);
    record_order.gaps.apply(&mut facts.splitter_gap_records);
    record_order.splitters.apply(&mut facts.splitter_records);
    facts
        .splitter_junction_records
        .sort_unstable_by_key(SplitterJunctionRecord::id);
    facts
        .contained_records
        .sort_unstable_by_key(|record| record.ordinal());
    facts
        .presentation_menu_anchor_records
        .sort_unstable_by_key(|record| record.root());
    facts
        .contained_minimums
        .sort_unstable_by_key(|measurement| measurement.floating());
    facts
        .drop_occlusions
        .sort_unstable_by_key(|occlusion| occlusion.floating());
    facts
        .drop_guide_clusters
        .sort_unstable_by_key(DropGuideClusterRecord::id);
    target_order.apply(&mut facts.drop_targets);
    facts.magnetic_target_field.canonicalize();

    let mut availability_changed = false;
    for target in &mut facts.drop_targets {
        let availability = canonical_drop_target_availability(target, policy);
        if target.availability() != availability {
            Arc::make_mut(target).set_availability(availability);
            availability_changed = true;
        }
    }
    // Sorting retains each Arc; validated guides already share unchanged targets.
    if availability_changed {
        let canonical_targets = &facts.drop_targets;
        for cluster in &mut facts.drop_guide_clusters {
            cluster.for_each_target_mut(|guide| {
                if let Some(background) = facts.surface_background.as_ref()
                    && background.id() == guide.id()
                {
                    guide.rebind_target(Arc::clone(background));
                    return;
                }
                let index = canonical_targets
                    .binary_search_by_key(&guide.id(), |target| target.id())
                    .expect("validated guide target remains in the canonical catalog");
                guide.rebind_target(Arc::clone(&canonical_targets[index]));
            });
        }
    }
}

fn canonical_drop_target_availability(
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

fn validate_drop_guide_cluster(
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

fn validate_drop_target(
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

fn rect_has_area(rect: LogicalRect) -> bool {
    rect.width() > 0.0 && rect.height() > 0.0
}

fn rect_contains(outer: LogicalRect, inner: LogicalRect) -> bool {
    let outer_min = outer.min();
    let outer_max = outer.max();
    let inner_min = inner.min();
    let inner_max = inner.max();
    inner_min.x() >= outer_min.x()
        && inner_min.y() >= outer_min.y()
        && inner_max.x() <= outer_max.x()
        && inner_max.y() <= outer_max.y()
}

fn rects_overlap_with_area(left: LogicalRect, right: LogicalRect) -> bool {
    left.min().x().max(right.min().x()) < left.max().x().min(right.max().x())
        && left.min().y().max(right.min().y()) < left.max().y().min(right.max().y())
}

pub(super) fn region_has_authoritative_area(
    region: LogicalRect,
    layer: SceneLayerKey,
    occlusions: &[DropOcclusionRecord],
) -> bool {
    let occluders = occlusions
        .iter()
        .filter(|occlusion| occlusion.layer() > layer)
        .map(|occlusion| occlusion.region().rect())
        .filter(|occlusion| rects_overlap_with_area(region, *occlusion))
        .collect::<Vec<_>>();
    if occluders.is_empty() {
        return true;
    }

    let mut x_edges = vec![region.x(), region.max().x()];
    for occluder in &occluders {
        x_edges.push(occluder.x().max(region.x()));
        x_edges.push(occluder.max().x().min(region.max().x()));
    }
    x_edges.sort_by(|left, right| left.total_cmp(right));
    x_edges.dedup_by(|left, right| left.total_cmp(right).is_eq());

    for strip in x_edges.windows(2) {
        let [strip_min, strip_max] = [strip[0], strip[1]];
        if strip_min >= strip_max {
            continue;
        }
        let mut intervals = occluders
            .iter()
            .filter(|occluder| occluder.x() <= strip_min && occluder.max().x() >= strip_max)
            .map(|occluder| {
                (
                    occluder.y().max(region.y()),
                    occluder.max().y().min(region.max().y()),
                )
            })
            .collect::<Vec<_>>();
        intervals.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.total_cmp(&right.1))
        });

        let mut covered_to = region.y();
        for (minimum, maximum) in intervals {
            if minimum > covered_to {
                return true;
            }
            covered_to = covered_to.max(maximum);
            if covered_to >= region.max().y() {
                break;
            }
        }
        if covered_to < region.max().y() {
            return true;
        }
    }
    false
}

fn rect_is_exact_intersection(
    actual: LogicalRect,
    first: LogicalRect,
    second: LogicalRect,
) -> bool {
    let min_x = first.x().max(second.x());
    let min_y = first.y().max(second.y());
    let max_x = first.max().x().min(second.max().x());
    let max_y = first.max().y().min(second.max().y());
    max_x > min_x
        && max_y > min_y
        && actual.x() == min_x
        && actual.y() == min_y
        && actual.max().x() == max_x
        && actual.max().y() == max_y
}

fn target_structure_is_valid(
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

fn validate_drop_visual(
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

struct IndexedRoot {
    owner: RootPresentationOwner,
    layer: SceneLayerKey,
    root_node: NodeId,
    nodes: HashSet<NodeId>,
    #[allow(
        dead_code,
        reason = "Frozen internal projection retains this fact for protocol validation and test inspection"
    )]
    fingerprint: Option<NodeFingerprint>,
}

#[derive(Debug, Clone, Copy)]
struct IndexedContained {
    surface: SurfaceId,
    root: RootId,
    rect: LogicalRect,
    layer: SceneLayerKey,
}

enum SceneWorkspaceIndex<'a> {
    #[allow(
        dead_code,
        reason = "Retained internal protocol case is matched by the shared reducer and exercised by protocol tests"
    )]
    Complete(CompleteSceneWorkspaceIndex),
    SharedSurface(SharedSurfaceSceneWorkspaceIndex<'a>),
}

struct CompleteSceneWorkspaceIndex {
    roots: HashMap<RootId, IndexedRoot>,
    contained: HashMap<FloatingPresentationId, IndexedContained>,
    contained_by_surface: HashMap<SurfaceId, Vec<FloatingPresentationId>>,
}

struct SharedSurfaceSceneWorkspaceIndex<'a> {
    workspace_index: WorkspaceIndexView<'a>,
    surface: SurfaceId,
    contained: HashMap<FloatingPresentationId, IndexedContained>,
    contained_roster: Vec<FloatingPresentationId>,
}

impl<'a> SceneWorkspaceIndex<'a> {
    #[cfg(test)]
    fn new(workspace: &Workspace) -> Result<Self, SceneBuildError> {
        let mut root_presentations = HashMap::with_capacity(workspace.roots().count());
        let mut contained = HashMap::with_capacity(workspace.contained_floatings().count());
        let mut contained_by_surface = HashMap::with_capacity(workspace.surfaces().count());
        for (surface, presentation) in workspace.surfaces() {
            if let Some(main_root) = presentation.main_root {
                root_presentations.insert(
                    main_root,
                    (
                        RootPresentationOwner::Main { surface },
                        SceneLayerKey::surface_base(),
                    ),
                );
            }
            for (index, floating) in presentation.contained.iter().copied().enumerate() {
                let layer = SceneLayerKey::contained(index)
                    .ok_or(SceneBuildError::ContainedLayerCapacityExceeded { surface, floating })?;
                if let Some(record) = workspace.contained_floating(floating) {
                    let owner = RootPresentationOwner::Contained { surface, floating };
                    root_presentations.insert(record.root, (owner, layer));
                    contained.insert(
                        floating,
                        IndexedContained {
                            surface,
                            root: record.root,
                            rect: record.rect,
                            layer,
                        },
                    );
                }
            }
            contained_by_surface.insert(surface, presentation.contained.clone());
        }

        let mut roots = HashMap::with_capacity(root_presentations.len());
        for (root, record) in workspace.roots() {
            let Some((owner, layer)) = root_presentations.get(&root).copied() else {
                continue;
            };
            let mut nodes = HashSet::new();
            let mut stack = vec![record.node];
            while let Some(node) = stack.pop() {
                if !nodes.insert(node) {
                    continue;
                }
                if let Some(value) = workspace.node(node) {
                    stack.extend(value.child_nodes().map(|(_, child)| child));
                }
            }
            let fingerprint = workspace.root_fingerprint(root).ok();
            roots.insert(
                root,
                IndexedRoot {
                    owner,
                    layer,
                    root_node: record.node,
                    nodes,
                    fingerprint,
                },
            );
        }
        Ok(Self::Complete(CompleteSceneWorkspaceIndex {
            roots,
            contained,
            contained_by_surface,
        }))
    }

    fn from_manifest_surface(
        workspace: &Workspace,
        workspace_index: WorkspaceIndexView<'a>,
        surface: SurfaceId,
    ) -> Result<Self, SceneBuildError> {
        let presentation = workspace
            .surface(surface)
            .ok_or(SceneBuildError::UnexpectedSceneSurface { surface })?;
        let mut contained = HashMap::with_capacity(presentation.contained.len());
        let mut contained_roster = Vec::with_capacity(presentation.contained.len());
        for (index, floating) in presentation.contained.iter().copied().enumerate() {
            let layer = SceneLayerKey::contained(index)
                .ok_or(SceneBuildError::ContainedLayerCapacityExceeded { surface, floating })?;
            contained_roster.push(floating);
            if let Some(record) = workspace.contained_floating(floating) {
                contained.insert(
                    floating,
                    IndexedContained {
                        surface,
                        root: record.root,
                        rect: record.rect,
                        layer,
                    },
                );
            }
        }
        Ok(Self::SharedSurface(SharedSurfaceSceneWorkspaceIndex {
            workspace_index,
            surface,
            contained,
            contained_roster,
        }))
    }

    fn bound_surface(&self) -> Option<SurfaceId> {
        match self {
            Self::Complete(_) => None,
            Self::SharedSurface(index) => Some(index.surface),
        }
    }

    fn root_belongs_to_surface(&self, surface: SurfaceId, root: RootId) -> bool {
        self.root_owner(root).is_some_and(|owner| match owner {
            RootPresentationOwner::Main {
                surface: owner_surface,
            }
            | RootPresentationOwner::Contained {
                surface: owner_surface,
                ..
            } => owner_surface == surface,
        })
    }

    fn contains_node(&self, root: RootId, node: NodeId) -> bool {
        match self {
            Self::Complete(index) => index
                .roots
                .get(&root)
                .is_some_and(|record| record.nodes.contains(&node)),
            Self::SharedSurface(index) => index.workspace_index.root_contains_node(root, node),
        }
    }

    fn semantic_node_exists(&self, surface: SurfaceId, root: RootId, node: NodeId) -> bool {
        self.root_belongs_to_surface(surface, root) && self.contains_node(root, node)
    }

    #[cfg(test)]
    fn root_fingerprint(&self, root: RootId) -> Option<&NodeFingerprint> {
        match self {
            Self::Complete(index) => index
                .roots
                .get(&root)
                .and_then(|record| record.fingerprint.as_ref()),
            Self::SharedSurface(index) => index.workspace_index.root_fingerprint(root),
        }
    }

    fn root_node(&self, root: RootId) -> Option<NodeId> {
        match self {
            Self::Complete(index) => index.roots.get(&root).map(|record| record.root_node),
            Self::SharedSurface(index) => index.workspace_index.root_node(root),
        }
    }

    fn root_layer(&self, root: RootId) -> Option<SceneLayerKey> {
        match self {
            Self::Complete(index) => index.roots.get(&root).map(|record| record.layer),
            Self::SharedSurface(index) => match index.workspace_index.root_owner(root)? {
                RootPresentationOwner::Main {
                    surface: owner_surface,
                } if owner_surface == index.surface => Some(SceneLayerKey::surface_base()),
                RootPresentationOwner::Contained {
                    surface: owner_surface,
                    floating,
                } if owner_surface == index.surface => {
                    index.contained.get(&floating).map(|record| record.layer)
                }
                _ => None,
            },
        }
    }

    fn root_owner(&self, root: RootId) -> Option<RootPresentationOwner> {
        match self {
            Self::Complete(index) => index.roots.get(&root).map(|record| record.owner),
            Self::SharedSurface(index) => index.workspace_index.root_owner(root),
        }
    }

    fn contained_belongs_to_surface(
        &self,
        surface: SurfaceId,
        floating: FloatingPresentationId,
    ) -> bool {
        self.contained_records()
            .get(&floating)
            .is_some_and(|record| {
                record.surface == surface
                    && self.root_owner(record.root)
                        == Some(RootPresentationOwner::Contained { surface, floating })
            })
    }

    fn contained_layer(&self, floating: FloatingPresentationId) -> Option<SceneLayerKey> {
        self.contained_records()
            .get(&floating)
            .map(|record| record.layer)
    }

    fn contained_rect(&self, floating: FloatingPresentationId) -> Option<LogicalRect> {
        self.contained_records()
            .get(&floating)
            .map(|record| record.rect)
    }

    fn contained_layers(&self, surface: SurfaceId) -> impl Iterator<Item = SceneLayerKey> + '_ {
        self.contained_records()
            .values()
            .filter(move |record| record.surface == surface)
            .map(|record| record.layer)
    }

    fn contained_on_surface(
        &self,
        surface: SurfaceId,
    ) -> impl Iterator<Item = FloatingPresentationId> + '_ {
        let roster = match self {
            Self::Complete(index) => index
                .contained_by_surface
                .get(&surface)
                .map_or(&[] as &[FloatingPresentationId], Vec::as_slice),
            Self::SharedSurface(index) if index.surface == surface => &index.contained_roster,
            Self::SharedSurface(_) => &[] as &[FloatingPresentationId],
        };
        roster.iter().copied()
    }

    fn contained_records(&self) -> &HashMap<FloatingPresentationId, IndexedContained> {
        match self {
            Self::Complete(index) => &index.contained,
            Self::SharedSurface(index) => &index.contained,
        }
    }
}
