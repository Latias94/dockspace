//! Complete pane sizing and selection-dependent layout measurements.

use std::collections::BTreeMap;

use super::SceneCompilationError;
use super::tabs::{reveal_tab_range, tab_local_range};
use crate::geometry::{AxisSizing, LogicalSize, PaneSizing};
use crate::graph::{Node, Workspace};
use crate::ids::{NodeId, RootId};
use crate::policy::TabBarVisibility;
use crate::presentation::config::DockPresentationConfig;
use crate::presentation::layout::{
    LayoutError, LayoutMetrics, RootMeasurement, measure_root_sizing,
};
use crate::presentation::manifest::{
    AuthoritativeSurfaceMeasurements, PaneSizingKey, SurfaceRequirements, TabBarRequirement,
};
use crate::presentation::scene::{PresentationPlan, TabBarSceneId};
use crate::presentation::tab_strip::{TabStripStateKey, TabStripStateStore};

pub(super) struct RootLayoutMeasurement<'workspace> {
    pub(super) pane_sizings: BTreeMap<PaneSizingKey, PaneSizing>,
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
    let sizing = root_leaf_sizing(
        workspace,
        config,
        requirements,
        root,
        &pane_sizings,
        tab_strip_states,
    )?;
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
    })?;
    Ok(RootLayoutMeasurement {
        pane_sizings,
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

fn root_leaf_sizing(
    workspace: &Workspace,
    config: &DockPresentationConfig,
    requirements: &SurfaceRequirements,
    root: RootId,
    pane_sizings: &BTreeMap<PaneSizingKey, PaneSizing>,
    tab_strip_states: &TabStripStateStore,
) -> Result<BTreeMap<NodeId, PaneSizing>, SceneCompilationError> {
    let mut sizing = BTreeMap::new();
    for (key, measured) in pane_sizings.iter().filter(|(key, _)| key.root() == root) {
        let key = *key;
        let Some(Node::Tabs { .. }) = workspace.node(key.tabs()) else {
            continue;
        };
        if !workspace.node_has_visible_content(key.tabs())
            || key.selected() != workspace.visible_tab_selection(key.tabs())
        {
            continue;
        }
        let id = TabBarSceneId {
            root: key.root(),
            tabs: key.tabs(),
        };
        let tab_bar = requirements
            .tab_bar(id)
            .ok_or(SceneCompilationError::MissingTabBarRequirement { id })?;
        sizing.insert(
            key.tabs(),
            pane_outer_sizing(
                *measured,
                config,
                tab_bar
                    .resolved_policy(requirements.ticket().surface(), tab_strip_states)
                    .visibility(),
            )?,
        );
    }
    Ok(sizing)
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
    selected: Option<crate::ids::ItemId>,
    tab_strip_states: &TabStripStateStore,
) -> Result<bool, SceneCompilationError> {
    let bar = requirement.id();
    let Some(layout) = plan.layout_facts() else {
        return Ok(false);
    };
    let Some(root) = layout.root(bar.root) else {
        return Ok(false);
    };
    let old_selected = plan.selected_in_tabs(bar.tabs);
    let Some(old_sizing) = root.pane_sizing(PaneSizingKey::new(bar.root, bar.tabs, old_selected))
    else {
        return Ok(false);
    };
    let Some(new_sizing) = root.pane_sizing(PaneSizingKey::new(bar.root, bar.tabs, selected))
    else {
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
                    .position(|member| member.tab().item == item)
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
                reveal_tab_range(
                    requested,
                    record.maximum_scroll_offset(),
                    record.viewport().width(),
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
