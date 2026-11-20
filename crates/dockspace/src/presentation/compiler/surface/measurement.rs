//! Complete pane sizing and selection-dependent layout measurements.

use std::collections::BTreeMap;

use super::SceneCompilationError;
use crate::geometry::{AxisSizing, LogicalSize, PaneSizing};
use crate::graph::{Node, Workspace};
use crate::ids::{ItemId, NodeId, RootId, TabKey};
use crate::policy::TabBarVisibility;
use crate::presentation::config::DockPresentationConfig;
use crate::presentation::layout::{
    LayoutError, LayoutMetrics, RootMeasurement, measure_root_sizing,
};
use crate::presentation::manifest::{
    AuthoritativeSurfaceMeasurements, PaneSizingKey, SurfaceRequirements, TabBarRequirement,
};
use crate::presentation::scene::{PresentationPlan, TabBarSceneId};
use crate::presentation::tab_strip::{
    TabStripStateKey, TabStripStateStore, reveal_scroll_offset, tab_local_range,
};

pub(super) struct RootLayoutMeasurement<'workspace> {
    pub(super) pane_sizings: BTreeMap<PaneSizingKey, PaneSizing>,
    pub(super) tab_bar_visibility: BTreeMap<NodeId, TabBarVisibility>,
    pub(super) layout: RootMeasurement<'workspace>,
}

pub(super) fn measure_root_layout<'workspace>(
    workspace: &'workspace Workspace,
    config: &DockPresentationConfig,
    requirements: &SurfaceRequirements,
    measurements: AuthoritativeSurfaceMeasurements<'_>,
    root: RootId,
    tab_strip_states: &TabStripStateStore,
) -> Result<RootLayoutMeasurement<'workspace>, SceneCompilationError> {
    let surface = requirements.ticket().surface();
    let pane_sizings = root_pane_sizing_measurements(requirements, measurements, root)?;
    let measured = pane_sizings
        .iter()
        .map(|(key, sizing)| (key.item(), *sizing))
        .collect();
    let mut tab_bar_visibility = BTreeMap::new();
    let topology = workspace
        .root_structure(root)
        .ok_or(LayoutError::UnfinishedRoot { root })?;
    for node in &topology.nodes {
        if matches!(workspace.node(node.id), Some(Node::Tabs { .. }))
            && workspace.node_has_visible_content(node.id)
        {
            let id = TabBarSceneId {
                root,
                tabs: node.id,
            };
            let requirement = requirements
                .tab_bar(id)
                .ok_or(SceneCompilationError::MissingTabBarRequirement { id })?;
            tab_bar_visibility.insert(
                node.id,
                requirement
                    .resolved_policy(surface, tab_strip_states)
                    .visibility(),
            );
        }
    }
    let (sizing, insets) =
        compose_root_sizing(workspace, root, config, &measured, &tab_bar_visibility)?;
    let layout = measure_root_sizing(
        workspace,
        root,
        &sizing,
        LayoutMetrics::new(config.splitter_thickness())?
            .with_pane_overflow_policy(config.pane_overflow_policy()),
    )
    .map_err(|error| match error {
        LayoutError::MissingRoot { root } => SceneCompilationError::MissingRoot { surface, root },
        LayoutError::MissingNode { node } => SceneCompilationError::MissingNode {
            surface,
            root,
            node,
        },
        LayoutError::MissingLeafConstraints { node } => {
            SceneCompilationError::MissingLeafSizing { root, tabs: node }
        }
        LayoutError::Cycle { node } => SceneCompilationError::RepeatedNode {
            surface,
            root,
            node,
        },
        error => error.into(),
    })?
    .with_tab_content_insets(&insets);
    Ok(RootLayoutMeasurement {
        pane_sizings,
        tab_bar_visibility,
        layout,
    })
}

fn root_pane_sizing_measurements(
    requirements: &SurfaceRequirements,
    measurements: AuthoritativeSurfaceMeasurements<'_>,
    root: RootId,
) -> Result<BTreeMap<PaneSizingKey, PaneSizing>, SceneCompilationError> {
    requirements
        .pane_sizings()
        .filter(|key| key.root() == root)
        .map(|key| {
            measurements
                .pane_sizing(key)
                .map(|minimum| (key, minimum))
                .ok_or(SceneCompilationError::MissingPaneSizingMeasurement { key })
        })
        .collect()
}

/// Composes the active child constraints while retaining sizing for every eligible item.
pub(crate) fn compose_root_sizing(
    workspace: &Workspace,
    root: RootId,
    config: &DockPresentationConfig,
    measured: &BTreeMap<ItemId, PaneSizing>,
    visibility: &BTreeMap<NodeId, TabBarVisibility>,
) -> Result<(BTreeMap<NodeId, PaneSizing>, BTreeMap<NodeId, f64>), SceneCompilationError> {
    let topology = workspace
        .root_structure(root)
        .ok_or(LayoutError::UnfinishedRoot { root })?;
    let mut sizing = BTreeMap::<NodeId, PaneSizing>::new();
    let mut insets = BTreeMap::new();
    let read_item = |item| {
        measured
            .get(&item)
            .copied()
            .ok_or(SceneCompilationError::MissingPaneSizingMeasurement {
                key: PaneSizingKey::new(root, item),
            })
    };
    for &row in &topology.postorder {
        let node = topology.nodes[row].id;
        let value = workspace
            .node(node)
            .ok_or(LayoutError::MissingNode { node })?;
        if matches!(value, Node::Split { .. }) && !topology.has_composition() {
            continue;
        }
        if !workspace.node_has_visible_content(node) {
            continue;
        }
        let value = match value {
            Node::Pane { item } => {
                pane_outer_sizing(read_item(*item)?, config, TabBarVisibility::Hidden)?
            }
            Node::Tabs { .. } => {
                let child = match workspace.visible_tab_selection(node) {
                    Some(tab) => match tab.item() {
                        Some(item) => read_item(item)?,
                        None => *sizing
                            .get(
                                &tab.layout()
                                    .ok_or(SceneCompilationError::MissingLeafSizing {
                                        root,
                                        tabs: node,
                                    })?,
                            )
                            .ok_or(SceneCompilationError::MissingLeafSizing { root, tabs: node })?,
                    },
                    None => PaneSizing::default(),
                };
                let visibility = visibility
                    .get(&node)
                    .copied()
                    .unwrap_or(TabBarVisibility::Hidden);
                if topology.has_composition() {
                    insets.insert(
                        node,
                        if visibility == TabBarVisibility::Visible {
                            config.tab_bar_height()
                        } else {
                            0.0
                        },
                    );
                }
                pane_outer_sizing(child, config, visibility)?
            }
            Node::Split { axis, children, .. } => {
                let children = children
                    .iter()
                    .filter_map(|child| sizing.get(child))
                    .copied()
                    .collect::<Vec<_>>();
                compose_split_sizing(*axis, &children, config.splitter_thickness())?
            }
            Node::Grid {
                shape: crate::model::DockspaceGridShape::Fixed { rows, columns },
                children,
                ..
            } => {
                let compose = |horizontal: bool| {
                    let count = if horizontal {
                        columns.get()
                    } else {
                        rows.get()
                    };
                    let mut tracks = vec![Vec::new(); count];
                    for (cell, child) in children.iter().enumerate() {
                        if let Some(sizing) = sizing.get(child) {
                            let track = if horizontal {
                                cell % columns.get()
                            } else {
                                cell / columns.get()
                            };
                            tracks[track].push(if horizontal {
                                sizing.width()
                            } else {
                                sizing.height()
                            });
                        }
                    }
                    let gaps = config.splitter_thickness() * count.saturating_sub(1) as f64;
                    let minimum = tracks
                        .iter()
                        .map(|track| track.iter().map(|axis| axis.minimum()).fold(0.0, f64::max))
                        .sum::<f64>()
                        + gaps;
                    let maximum = tracks
                        .iter()
                        .map(|track| {
                            if track.is_empty() {
                                return None;
                            }
                            track
                                .iter()
                                .map(|axis| axis.maximum())
                                .collect::<Option<Vec<_>>>()
                                .map(|values| values.into_iter().fold(0.0, f64::max))
                        })
                        .collect::<Option<Vec<_>>>()
                        .map(|tracks| tracks.iter().sum::<f64>() + gaps);
                    AxisSizing::new(minimum, maximum, None)
                        .map_err(|_| SceneCompilationError::PaneSizingOverflow)
                };
                PaneSizing::new(compose(true)?, compose(false)?)
            }
            Node::Grid { children, .. } => {
                let envelope = |horizontal: bool| {
                    let minimum = children
                        .iter()
                        .filter_map(|child| sizing.get(child))
                        .map(|child| {
                            if horizontal {
                                child.width().minimum()
                            } else {
                                child.height().minimum()
                            }
                        })
                        .fold(0.0, f64::max);
                    // Auto dimensions depend on offered space. A cell maximum
                    // cannot cap this container before its track count is known;
                    // exact track maxima are composed after shape resolution.
                    AxisSizing::new(minimum, None, None)
                        .map_err(|_| SceneCompilationError::PaneSizingOverflow)
                };
                PaneSizing::new(envelope(true)?, envelope(false)?)
            }
        };
        sizing.insert(node, value);
    }
    Ok((sizing, insets))
}

fn compose_split_sizing(
    axis: crate::graph::Axis,
    children: &[PaneSizing],
    gap: f64,
) -> Result<PaneSizing, SceneCompilationError> {
    let gaps = gap * children.len().saturating_sub(1) as f64;
    let compose = |horizontal: bool| -> Result<AxisSizing, SceneCompilationError> {
        let along = horizontal == (axis == crate::graph::Axis::Horizontal);
        let axes = children
            .iter()
            .map(|child| {
                if horizontal {
                    child.width()
                } else {
                    child.height()
                }
            })
            .collect::<Vec<_>>();
        let minimum = if along {
            axes.iter().map(|axis| axis.minimum()).sum::<f64>() + gaps
        } else {
            axes.iter().map(|axis| axis.minimum()).fold(0.0, f64::max)
        };
        let maximum = axes
            .iter()
            .map(|axis| axis.maximum())
            .collect::<Option<Vec<_>>>()
            .map(|values| {
                if along {
                    values.iter().sum::<f64>() + gaps
                } else {
                    values.into_iter().fold(0.0, f64::max)
                }
            });
        let preferred = axes
            .iter()
            .map(|axis| axis.preferred())
            .collect::<Option<Vec<_>>>()
            .map(|values| {
                if along {
                    values.iter().sum::<f64>() + gaps
                } else {
                    values.into_iter().fold(0.0, f64::max)
                }
            });
        AxisSizing::new(minimum, maximum, preferred)
            .map_err(|_| SceneCompilationError::PaneSizingOverflow)
    };
    Ok(PaneSizing::new(compose(true)?, compose(false)?))
}

/// Resolves the style floor and adds core-owned chrome exactly once.
pub(crate) fn pane_outer_sizing(
    measured: PaneSizing,
    config: &DockPresentationConfig,
    tab_bar_visibility: TabBarVisibility,
) -> Result<PaneSizing, SceneCompilationError> {
    let minimum = config.minimum_pane_size();
    let tab_height = if tab_bar_visibility == TabBarVisibility::Visible {
        config.tab_bar_height()
    } else {
        0.0
    };
    let resolve = |axis: AxisSizing, floor: f64, chrome: f64| {
        let maximum = axis.maximum();
        let minimum = axis.minimum().max(floor);
        let minimum = maximum.map_or(minimum, |maximum| minimum.min(maximum));
        let preferred = axis.preferred().map(|preferred| {
            maximum.map_or(preferred.max(minimum), |maximum| {
                preferred.clamp(minimum, maximum)
            }) + chrome
        });
        AxisSizing::new(
            minimum + chrome,
            maximum.map(|maximum| maximum + chrome),
            preferred,
        )
        .map_err(|_| SceneCompilationError::PaneSizingOverflow)
    };
    Ok(PaneSizing::new(
        resolve(measured.width(), minimum.width(), 0.0)?,
        resolve(measured.height(), minimum.height(), tab_height)?,
    ))
}

/// Checks the selection-dependent inputs of an otherwise unchanged sealed surface.
pub(crate) fn selection_geometry_unchanged(
    plan: &PresentationPlan,
    config: &DockPresentationConfig,
    requirement: &TabBarRequirement,
    selected: Option<TabKey>,
    tab_strip_states: &TabStripStateStore,
) -> Result<bool, SceneCompilationError> {
    let bar = requirement.id();
    let Some(layout) = plan.layout_facts() else {
        return Ok(false);
    };
    let Some(root) = layout.root(bar.root) else {
        return Ok(false);
    };
    if root.projection().nodes.topology_has_composition() {
        return Ok(false);
    }
    let old_selected = plan.selected_in_tabs(bar.tabs);
    if old_selected.is_some_and(TabKey::is_layout) || selected.is_some_and(TabKey::is_layout) {
        return Ok(false);
    }
    let (Some(old_item), Some(new_item)) = (
        old_selected.and_then(TabKey::item),
        selected.and_then(TabKey::item),
    ) else {
        return Ok(false);
    };
    let Some(old_sizing) = root.pane_sizing(PaneSizingKey::new(bar.root, old_item)) else {
        return Ok(false);
    };
    let Some(new_sizing) = root.pane_sizing(PaneSizingKey::new(bar.root, new_item)) else {
        return Ok(false);
    };
    let visibility = requirement
        .resolved_policy(plan.surface(), tab_strip_states)
        .visibility();
    if pane_outer_sizing(old_sizing, config, visibility)?
        != pane_outer_sizing(new_sizing, config, visibility)?
    {
        return Ok(false);
    }
    let Some(record) = plan
        .tab_bar_records()
        .iter()
        .find(|record| *record.id() == bar)
    else {
        return Ok(visibility != TabBarVisibility::Visible);
    };
    let state = tab_strip_states.state(TabStripStateKey::new(plan.surface(), bar));
    let requested = state
        .and_then(|state| state.scroll_offset())
        .unwrap_or(0.0)
        .clamp(0.0, record.maximum_scroll_offset());
    let scroll = if state.is_none_or(|state| state.resolved_selection() != selected) {
        selected
            .and_then(|item| {
                record
                    .members()
                    .iter()
                    .position(|member| member.tab().tab == item)
            })
            .and_then(|index| {
                tab_local_range(
                    record
                        .members()
                        .iter()
                        .map(|member| member.allocated_width()),
                    index,
                )
            })
            .map_or(requested, |(start, end)| {
                reveal_scroll_offset(
                    requested,
                    record.maximum_scroll_offset(),
                    record.layout_viewport().width(),
                    start,
                    end,
                )
            })
    } else {
        requested
    };
    Ok(scroll == record.scroll_offset())
}

pub(crate) fn contained_minimum_size(
    config: &DockPresentationConfig,
    root_minimum: LogicalSize,
) -> Result<LogicalSize, SceneCompilationError> {
    let border = 2.0 * config.floating_border_width();
    let measured = LogicalSize::new(
        root_minimum.width() + border,
        root_minimum.height() + config.floating_title_height() + border,
    )?;
    let configured = config.minimum_floating_size();
    Ok(LogicalSize::new(
        measured.width().max(configured.width()),
        measured.height().max(configured.height()),
    )?)
}

#[cfg(test)]
mod sizing_tests {
    use super::*;

    #[test]
    fn chrome_overflow_is_a_configuration_error_and_hidden_chrome_is_not_added() {
        let sizing = PaneSizing::builder()
            .fixed_height(f64::MAX)
            .preferred_height(f64::MAX)
            .build()
            .unwrap();
        let config = DockPresentationConfig::builder()
            .tab_bar_height(f64::MAX)
            .build()
            .unwrap();
        assert!(matches!(
            pane_outer_sizing(sizing, &config, TabBarVisibility::Visible),
            Err(SceneCompilationError::PaneSizingOverflow)
        ));
        let hidden = pane_outer_sizing(sizing, &config, TabBarVisibility::Hidden).unwrap();
        assert_eq!(hidden.height(), sizing.height());
    }

    #[test]
    fn a_style_floor_clamps_preferences_but_never_enlarges_an_explicit_cap() {
        let measured = PaneSizing::builder()
            .preferred_width(20.0)
            .fixed_height(30.0)
            .preferred_height(30.0)
            .build()
            .unwrap();
        let actual = pane_outer_sizing(
            measured,
            &DockPresentationConfig::default(),
            TabBarVisibility::Visible,
        )
        .unwrap();
        assert_eq!(
            actual.width(),
            AxisSizing::new(80.0, None, Some(80.0)).unwrap()
        );
        assert_eq!(
            actual.height(),
            AxisSizing::new(58.0, Some(58.0), Some(58.0)).unwrap()
        );
    }
}
