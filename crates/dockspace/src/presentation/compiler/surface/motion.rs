//! Materialize sampled pane bounds from a sealed target without solving its layout again.

use super::*;
use crate::presentation::manifest::PaneSizingKey;
use crate::presentation::scene::{
    PaneSceneId, PresentationMenuAnchorHost, ResolvedSurfaceProgramOperation,
    ResolvedSurfaceProgramStep, SurfaceProgramVisitError,
};

/// Rebuilds leaf content and chrome at their sampled sizes, retaining target constraints.
///
/// A popup on this surface settles immediately to the target so its complete popup
/// plane and exact receiver roster stay available. Other surfaces keep their motion.
#[allow(clippy::too_many_arguments)]
pub(crate) fn materialize_presentation_sample(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    manifest: &SceneRequirementManifest,
    tab_strip_states: &TabStripStateStore,
    target: &PresentationPlan,
    rects: &BTreeMap<PaneSceneId, LogicalRect>,
    ranks: &BTreeMap<PaneSceneId, usize>,
) -> Result<PresentationPlan, PresentationCompilationError> {
    let surface = target.surface();
    if target.layout_facts().is_some_and(|facts| {
        facts
            .roots()
            .any(|root| root.projection().nodes.topology_has_composition())
    }) {
        return Ok(target.clone());
    }
    if target.popup().visual_for_surface(surface).is_some() {
        return Ok(target.clone());
    }
    let moving_roots = target
        .pane_records()
        .iter()
        .filter(|pane| {
            rects
                .get(&pane.id())
                .is_some_and(|rect| *rect != pane.bounds())
        })
        .map(|pane| pane.id().root)
        .collect::<BTreeSet<_>>();
    if moving_roots.is_empty() {
        return Ok(target.clone());
    }

    let requirements = manifest
        .surface(surface)
        .ok_or(PresentationCompilationError::RequirementsVanished { surface })?;
    if let Some(actual) = target.measurement_ticket()
        && actual != requirements.ticket()
    {
        return Err(SceneCompilationError::TicketMismatch {
            expected: requirements.ticket(),
            actual,
        }
        .into());
    }
    if requirements.ticket().policy() != policy.revision() {
        return Err(SceneCompilationError::PolicyRevisionMismatch {
            ticket: requirements.ticket().policy(),
            snapshot: policy.revision(),
        }
        .into());
    }
    if tab_strip_states.popup_requirement() != target.popup() || manifest.popup() != target.popup()
    {
        return Err(SceneCompilationError::PopupPlaneStateMismatch.into());
    }
    let layout = target
        .layout_facts()
        .ok_or(SceneCompilationError::SurfaceProgramInvariant)?;
    let fraction = dock_fraction(layout.config())?;
    let mut sampled = target.begin_presentation_sample(moving_roots, ranks);
    target
        .visit_surface_program(|step| -> Result<(), SceneCompilationError> {
            match step {
                ResolvedSurfaceProgramStep::RootBegin(root) => {
                    let presentation = root
                        .contained_record()
                        .map_or(SurfaceProgramRootPresentation::Main, |contained| {
                            SurfaceProgramRootPresentation::Contained(contained.floating())
                        });
                    sampled.begin_surface_program_root(root.root(), presentation)?;
                }
                ResolvedSurfaceProgramStep::Operation { operation, .. } => match operation {
                    ResolvedSurfaceProgramOperation::Tabs(tabs_record) => {
                        let id = tabs_record.id();
                        let pane = target
                            .pane_records()
                            .iter()
                            .find(|pane| pane.containing_tab_bar() == Some(id));
                        let Some(Node::Tabs {
                            items: persistent_items,
                            ..
                        }) = workspace.node(id.tabs)
                        else {
                            return Err(SceneCompilationError::MissingNode {
                                surface,
                                root: id.root,
                                node: id.tabs,
                            });
                        };
                        let items = workspace.visible_tab_keys(id.tabs);
                        let selected = workspace.visible_tab_selection(id.tabs);
                        let root_layout =
                            layout
                                .root(id.root)
                                .ok_or(SceneCompilationError::MissingRoot {
                                    surface,
                                    root: id.root,
                                })?;
                        let sampled_bounds = pane
                            .and_then(|pane| rects.get(&pane.id()).copied())
                            .map(|rect| {
                                let inset =
                                    tabs_record.content_bounds().y() - tabs_record.bounds().y();
                                LogicalRect::from_min_max(
                                    LogicalPoint::new(rect.x(), rect.y() - inset)?,
                                    rect.max(),
                                )
                            })
                            .transpose()?
                            .unwrap_or_else(|| tabs_record.bounds());
                        let content_top = pane
                            .and_then(|pane| rects.get(&pane.id()))
                            .map_or(tabs_record.content_bounds().y(), |rect| rect.y());
                        let sizing = selected
                            .and_then(crate::ids::TabKey::item)
                            .map(|item| {
                                let key = PaneSizingKey::new(id.root, item);
                                root_layout.pane_sizing(key).ok_or(
                                    SceneCompilationError::MissingPaneSizingMeasurement { key },
                                )
                            })
                            .transpose()?
                            .unwrap_or_default();
                        let maximum = root_layout
                            .projection()
                            .nodes
                            .get(&id.tabs)
                            .ok_or(SceneCompilationError::MissingNodeProjection {
                                root: id.root,
                                node: id.tabs,
                            })?
                            .constraints
                            .max();
                        let capped_bounds = LogicalRect::from_min_max(
                            sampled_bounds.min(),
                            LogicalPoint::new(
                                sizing
                                    .width()
                                    .maximum()
                                    .map_or(sampled_bounds.max().x(), |_| {
                                        (sampled_bounds.x() + maximum.width())
                                            .min(sampled_bounds.max().x())
                                    }),
                                sizing
                                    .height()
                                    .maximum()
                                    .map_or(sampled_bounds.max().y(), |_| {
                                        (sampled_bounds.y() + maximum.height())
                                            .min(sampled_bounds.max().y())
                                    }),
                            )?,
                        )?;
                        let bounds = clip_rect(capped_bounds, root_layout.bounds())?;
                        let presentation_menu_anchor = target
                            .presentation_menu_anchor_records()
                            .iter()
                            .any(|anchor| {
                                anchor.host()
                                    == PresentationMenuAnchorHost::TabBar(TabBarSceneId {
                                        root: id.root,
                                        tabs: id.tabs,
                                    })
                            });
                        let content = compile_tabs_leaf(
                            policy,
                            layout.config(),
                            requirements,
                            layout.measurements(),
                            surface,
                            id.root,
                            id.tabs,
                            0,
                            bounds,
                            Some(content_top),
                            &items,
                            persistent_items,
                            selected,
                            tabs_record.layer(),
                            fraction,
                            tab_strip_states,
                            false,
                            presentation_menu_anchor,
                            None,
                            false,
                            None,
                            &mut sampled,
                        )?;
                        if let Some(item) = selected.and_then(crate::ids::TabKey::item) {
                            let branch = Some(sampled.push_active_tab_choice(
                                None,
                                id.tabs,
                                selected.expect("selected content has a key"),
                            ));
                            let pane_id = PaneSceneId {
                                root: id.root,
                                item,
                            };
                            sampled.push_pane_record(
                                PaneRecord::new(
                                    pane_id,
                                    id.tabs,
                                    content,
                                    content,
                                    tabs_record.layer(),
                                )
                                .with_context(
                                    content,
                                    Some(id),
                                    branch,
                                ),
                            );
                            sampled.push_surface_program_pane_operation(pane_id)?;
                        }
                    }
                    ResolvedSurfaceProgramOperation::Pane(_) => {}
                    ResolvedSurfaceProgramOperation::Splitter(record) => {
                        if sampled.partition_available(record.id().root) {
                            sampled.push_surface_program_root_operation(
                                SurfaceProgramRootOperation::Splitter(*record.id()),
                            )?;
                        }
                    }
                    ResolvedSurfaceProgramOperation::SplitterJunction(record) => {
                        if sampled.partition_available(record.id().root()) {
                            sampled.push_surface_program_root_operation(
                                SurfaceProgramRootOperation::SplitterJunction(record.id()),
                            )?;
                        }
                    }
                    ResolvedSurfaceProgramOperation::ContainedControls(record) => {
                        sampled.push_surface_program_root_operation(
                            SurfaceProgramRootOperation::ContainedControls(record.floating()),
                        )?;
                    }
                    ResolvedSurfaceProgramOperation::PresentationMenuAnchor(record)
                        if matches!(
                            record.host(),
                            PresentationMenuAnchorHost::RootControls
                                | PresentationMenuAnchorHost::ContainedTitle(_)
                        ) =>
                    {
                        sampled.push_surface_program_root_operation(
                            SurfaceProgramRootOperation::PresentationMenuAnchor(record.root()),
                        )?;
                    }
                    ResolvedSurfaceProgramOperation::TabBar(_)
                    | ResolvedSurfaceProgramOperation::Tab(_)
                    | ResolvedSurfaceProgramOperation::TabStripControl(_)
                    | ResolvedSurfaceProgramOperation::PresentationMenuAnchor(_) => {}
                },
                ResolvedSurfaceProgramStep::RootEnd(root) => {
                    sampled.end_surface_program_root(root.root())?;
                }
                // The fresh program defers the retained catalog and emits the same
                // global phases. Leaf compilation supplies its new local operations.
                ResolvedSurfaceProgramStep::PopupBackdrop(_)
                | ResolvedSurfaceProgramStep::PopupMenu(_)
                | ResolvedSurfaceProgramStep::DragPreview
                | ResolvedSurfaceProgramStep::DropTarget(_)
                | ResolvedSurfaceProgramStep::DropGuide(_)
                | ResolvedSurfaceProgramStep::ContainedTransformPreview
                | ResolvedSurfaceProgramStep::DragDecoration
                | ResolvedSurfaceProgramStep::InteractionFeedback
                | ResolvedSurfaceProgramStep::SurfaceEnd => {}
            }
            Ok(())
        })
        .map_err(|error| match error {
            SurfaceProgramVisitError::InvalidProgram => {
                SceneCompilationError::Scene(SceneBuildError::InvalidSurfaceProgram { surface })
            }
            SurfaceProgramVisitError::Sink(error) => error,
        })?;
    sampled
        .finish_surface_program()
        .map_err(SceneCompilationError::from)?;
    sampled
        .finish_presentation_sample()
        .map_err(SceneCompilationError::from)?;
    Ok(sampled)
}

#[cfg(test)]
mod tests;
