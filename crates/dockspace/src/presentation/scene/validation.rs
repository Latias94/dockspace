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

mod contained;
mod drop_targets;
mod splitters;
mod tabs;

use contained::{validate_contained_minimums, validate_contained_records};
#[cfg(test)]
use drop_targets::validate_magnetic_target_field;
use drop_targets::{
    canonical_drop_target_availability, validate_compiled_target_coverage,
    validate_drop_guide_cluster, validate_drop_target, validate_surface_background,
};
#[cfg(test)]
pub(super) use splitters::splitter_extent_satisfies_constraints;
use splitters::validate_splitter_geometry;
#[cfg(test)]
use splitters::validate_splitter_records;
#[cfg(test)]
use tabs::{expected_pane_records, validate_pane_and_tab_records};
use tabs::{validate_presentation_menu_anchors, validate_tab_records};

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
            if !rect_contains(ready.facts.bounds, record.clip()) {
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
