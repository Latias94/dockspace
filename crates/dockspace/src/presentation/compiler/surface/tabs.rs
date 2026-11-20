//! Tab-strip layout, optional chrome, overflow menus and pane scene records.

use std::{collections::BTreeSet, sync::Arc};

use super::guides::{GuideEdgeGeometry, compile_content_targets};
use super::{
    SceneCompilationError, checked_sum, clip_rect, intersect_rect, rect_contains, rect_has_area,
    tab_availability,
};
use crate::geometry::{GeometryError, LogicalPoint, LogicalRect};
use crate::graph::command::DockFraction;
use crate::graph::{Node, Workspace};
use crate::ids::{NodeId, RootId, SurfaceId};
use crate::interaction::targeting::targets::{
    DropTargetLocation, DropTargetRecord, DropVisual, SceneLayerKey,
};
use crate::policy::{DockPolicySnapshot, TabBarInteraction, TabBarVisibility};
use crate::presentation::compiler::tab_strip_layout::{
    TabLeafBounds, TabStripFlow, desired_tab_width, reveal_tab_bar_bounds, select_menu_host,
    tab_leaf_bounds,
};
use crate::presentation::config::DockPresentationConfig;
use crate::presentation::hit::region::HitRegion;
use crate::presentation::layout::{NodeMap, NodeResult};
use crate::presentation::manifest::{
    AuthoritativeSurfaceMeasurements, SurfaceMeasurements, SurfaceRequirements, TabBarRequirement,
    TabIntrinsicKey, TabStripControlMetrics, TabStripControlPlacement, TabStripKey,
};
use crate::presentation::scene::{
    PresentationMenuAnchorHost, PresentationMenuAnchorRecord, PresentationPlan,
    SurfaceProgramRootOperation, TabBarRecord, TabBarSceneId, TabGroupDragRecord,
    TabListMenuGeometryAvailability, TabListMenuRecord, TabListMenuRowRecord, TabRecord,
    TabSceneId, TabStripControlRecord, TabStripMemberRecord, TabStripMemberVisibility, TabsRecord,
};
use crate::presentation::tab_strip::{
    ActiveTabListMenu, TabStripControlId, TabStripStateKey, TabStripStateStore,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn select_presentation_menu_tab_bar(
    workspace: &Workspace,
    requirements: &SurfaceRequirements,
    measurements: AuthoritativeSurfaceMeasurements<'_>,
    config: &DockPresentationConfig,
    surface: SurfaceId,
    root: RootId,
    root_node: NodeId,
    central: Option<NodeId>,
    allocation: LogicalRect,
    root_bounds: LogicalRect,
    node_results: &NodeMap<NodeResult>,
    tab_strip_states: &TabStripStateStore,
) -> Result<Option<NodeId>, SceneCompilationError> {
    let visible_menu_available = |tabs| -> Result<Option<bool>, SceneCompilationError> {
        if !node_results.get(&tabs).is_some_and(|result| result.visible) {
            return Ok(None);
        }
        let id = TabBarSceneId { root, tabs };
        let requirement = requirements
            .tab_bar(id)
            .ok_or(SceneCompilationError::MissingTabBarRequirement { id })?;
        if requirement
            .resolved_policy(surface, tab_strip_states)
            .visibility()
            == TabBarVisibility::Hidden
        {
            return Ok(None);
        }
        let projected = node_results
            .get(&tabs)
            .map(|result| result.rect)
            .ok_or(SceneCompilationError::MissingNodeProjection { root, node: tabs })?;
        let geometry = tab_leaf_bounds(
            clip_rect(projected, allocation)?,
            root_bounds,
            config.tab_bar_height(),
            None,
        )?;
        if !rect_has_area(geometry.header) {
            return Ok(None);
        }
        let items = match workspace.node(tabs) {
            Some(Node::Tabs { .. }) => workspace.visible_tab_keys(tabs),
            Some(Node::Pane { .. } | Node::Split { .. } | Node::Grid { .. }) => return Ok(None),
            None => {
                return Err(SceneCompilationError::MissingNode {
                    surface,
                    root,
                    node: tabs,
                });
            }
        };
        let strip_key = TabStripKey::new(surface, id);
        let strip = measurements
            .tab_strip(strip_key)
            .ok_or(SceneCompilationError::MissingTabStripMeasurement { key: strip_key })?;
        let chrome = tab_strip_optional_chrome_layout(
            geometry.header_allocation,
            items.len(),
            strip,
            config,
            true,
        )?;
        Ok(Some(chrome.presentation_menu_bounds.is_some_and(
            |bounds| rect_contains(geometry.header, bounds),
        )))
    };

    select_menu_host(
        workspace,
        surface,
        root,
        root_node,
        central,
        visible_menu_available,
    )
}

mod chrome;
pub(in crate::presentation::compiler) use chrome::tab_strip_optional_chrome_layout;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_tabs_leaf(
    policy: &DockPolicySnapshot,
    config: &DockPresentationConfig,
    requirements: &SurfaceRequirements,
    measurements: &SurfaceMeasurements,
    surface: SurfaceId,
    root: RootId,
    tabs: NodeId,
    depth: usize,
    bounds: LogicalRect,
    clip: LogicalRect,
    content_top: Option<f64>,
    items: &[crate::ids::TabKey],
    persistent_items: &[crate::ids::TabKey],
    selected: Option<crate::ids::TabKey>,
    layer: SceneLayerKey,
    fraction: DockFraction,
    tab_strip_states: &TabStripStateStore,
    root_central: bool,
    presentation_menu_anchor: bool,
    outer_guides: Option<GuideEdgeGeometry>,
    emit_drop_targets: bool,
    active_ancestry: Option<usize>,
    ready: &mut PresentationPlan,
) -> Result<(LogicalRect, LogicalRect), SceneCompilationError> {
    let bar_id = TabBarSceneId { root, tabs };
    let tab_bar_requirement = requirements
        .tab_bar(bar_id)
        .ok_or(SceneCompilationError::MissingTabBarRequirement { id: bar_id })?;
    let resolved_policy = tab_bar_requirement.resolved_policy(surface, tab_strip_states);
    let tab_height = if resolved_policy.visibility() == TabBarVisibility::Visible {
        config.tab_bar_height()
    } else {
        0.0
    };
    let allocation = bounds;
    let TabLeafBounds {
        bounds,
        header: tab_bar,
        header_allocation,
        content,
        content_allocation,
    } = tab_leaf_bounds(
        bounds,
        clip,
        tab_height,
        content_top.filter(|_| resolved_policy.visibility() == TabBarVisibility::Visible),
    )?;
    ready.push_tabs_record(
        TabsRecord::new(bar_id, bounds, content, layer, active_ancestry),
        selected,
    );
    ready.push_surface_program_root_operation(SurfaceProgramRootOperation::Tabs(bar_id))?;
    if rect_has_area(tab_bar) {
        compile_tab_strip(
            policy,
            config,
            tab_bar_requirement,
            measurements,
            surface,
            root,
            tabs,
            header_allocation,
            tab_bar,
            items,
            persistent_items,
            selected,
            layer,
            tab_strip_states,
            presentation_menu_anchor,
            emit_drop_targets,
            ready,
        )?;
    } else if tab_bar_requirement.policy().visibility() == TabBarVisibility::AutoHideSingle
        && resolved_policy.visibility() == TabBarVisibility::Hidden
        && items.len() == 1
        && rect_has_area(bounds)
    {
        if let Some(bounds) = reveal_tab_bar_bounds(allocation, clip, config)? {
            let control = TabStripControlId::RevealTabBar(bar_id);
            ready.push_tab_strip_control_record(TabStripControlRecord::new(
                control,
                bounds,
                resolved_policy.interaction() == TabBarInteraction::Enabled,
                layer,
            ));
            ready.push_surface_program_root_operation(
                SurfaceProgramRootOperation::TabStripControl(control),
            )?;
        }
    }
    if emit_drop_targets {
        compile_content_targets(
            policy,
            config,
            surface,
            root,
            tabs,
            depth,
            bounds,
            content,
            layer,
            fraction,
            root_central,
            outer_guides,
            ready,
        )?;
    }
    Ok((content_allocation, content))
}

#[allow(clippy::too_many_arguments)]
fn compile_tab_strip(
    policy: &DockPolicySnapshot,
    config: &DockPresentationConfig,
    requirement: &TabBarRequirement,
    measurements: &SurfaceMeasurements,
    surface: SurfaceId,
    root: RootId,
    tabs: NodeId,
    bar: LogicalRect,
    visible_bar: LogicalRect,
    items: &[crate::ids::TabKey],
    persistent_items: &[crate::ids::TabKey],
    selected: Option<crate::ids::TabKey>,
    layer: SceneLayerKey,
    tab_strip_states: &TabStripStateStore,
    presentation_menu_anchor: bool,
    emit_drop_targets: bool,
    ready: &mut PresentationPlan,
) -> Result<(), SceneCompilationError> {
    let interaction = requirement.policy().interaction();
    let bar_id = TabBarSceneId { root, tabs };
    let state_key = TabStripStateKey::new(surface, bar_id);
    let state = tab_strip_states.state(state_key);
    let active_menu = tab_strip_states.active_menu_for(state_key);
    let strip_key = TabStripKey::new(surface, bar_id);
    let strip = measurements
        .measured_tab_strip(strip_key)
        .ok_or(SceneCompilationError::MissingTabStripMeasurement { key: strip_key })?;
    let chrome = tab_strip_optional_chrome_layout(
        bar,
        items.len(),
        strip,
        config,
        presentation_menu_anchor,
    )?;
    let visible_button = |bounds: &LogicalRect| rect_contains(visible_bar, *bounds);
    let clip_slot = |slot: Option<LogicalRect>| -> Result<Option<LogicalRect>, GeometryError> {
        Ok(slot
            .map(|bounds| intersect_rect(bounds, visible_bar))
            .transpose()?
            .flatten())
    };
    let presentation_menu_bounds = chrome.presentation_menu_bounds.filter(visible_button);
    let group_extent = chrome.group_extent;
    let group_grip_bounds = if group_extent > 0.0 {
        let grip = LogicalRect::new(bar.x(), bar.y(), group_extent, bar.height())?;
        intersect_rect(grip, visible_bar)?
    } else {
        None
    };
    let mut desired_widths = Vec::with_capacity(items.len());
    let mut content_widths = Vec::with_capacity(items.len());
    let mut close_allowed = Vec::with_capacity(items.len());
    for item in items.iter().copied() {
        let key = TabIntrinsicKey::new(
            surface,
            TabSceneId {
                root,
                tabs,
                tab: item,
            },
        );
        let intrinsic = measurements
            .measured_tab_intrinsic(key)
            .ok_or(SceneCompilationError::MissingTabIntrinsicMeasurement { key })?;
        let item_close_allowed = requirement
            .close_capability(item)
            .ok_or(SceneCompilationError::MissingTabCloseCapability {
                id: TabSceneId {
                    root,
                    tabs,
                    tab: item,
                },
            })?
            .allows_close();
        content_widths.push(intrinsic.menu_content_width());
        desired_widths.push(desired_tab_width(
            intrinsic.content_width(),
            item_close_allowed,
            config,
        ));
        close_allowed.push(item_close_allowed);
    }

    let layout_viewport = chrome.viewport;
    let viewport = clip_rect(layout_viewport, visible_bar)?;
    let viewport_width = layout_viewport.width();
    let control_rects = chrome
        .controls
        .map(|layout| layout.rects.map(|bounds| bounds.filter(visible_button)));
    let menu_geometry = if control_rects
        .as_ref()
        .is_some_and(|rects| rects[2].is_some())
        && strip
            .tab_list_menu()
            .is_some_and(crate::presentation::manifest::TabListMenuMetrics::can_allocate)
    {
        TabListMenuGeometryAvailability::Available
    } else {
        TabListMenuGeometryAvailability::Unavailable
    };
    let flow = TabStripFlow::allocate(
        &desired_widths,
        viewport_width,
        strip.after_tabs_width(),
        config.tab_min_width(),
    )?;
    let max_scroll = flow.maximum_scroll_offset(viewport_width);
    let selected_index =
        selected.and_then(|selected| items.iter().position(|item| *item == selected));
    let requested_scroll = state.and_then(|state| state.scroll_offset()).unwrap_or(0.0);
    let selection_changed = state.is_none_or(|state| state.resolved_selection() != selected);
    let viewport_shrank = state
        .and_then(|state| state.resolved_viewport_width())
        .is_some_and(|previous| viewport_width < previous);
    let scroll = flow.scroll_offset(
        requested_scroll,
        viewport_width,
        selected_index,
        selection_changed || viewport_shrank,
    );

    let mut full_tabs = Vec::with_capacity(items.len());
    let mut program_tabs = Vec::with_capacity(items.len());
    let mut cursor = layout_viewport.x() - scroll;
    let mut visible_items = BTreeSet::new();
    let mut members = Vec::with_capacity(items.len());
    for (ordinal, ((item, width), close_allowed)) in items
        .iter()
        .copied()
        .zip(flow.widths.iter().copied())
        .zip(close_allowed.iter().copied())
        .enumerate()
    {
        let full = LogicalRect::new(cursor, bar.y(), width, bar.height())?;
        let visible_intersection = intersect_rect(full, viewport)?;
        let visibility = match visible_intersection {
            None => TabStripMemberVisibility::Hidden,
            Some(visible) if visible == full => TabStripMemberVisibility::Visible,
            Some(_) => TabStripMemberVisibility::PartiallyVisible,
        };
        members.push(TabStripMemberRecord::new(
            TabSceneId {
                root,
                tabs,
                tab: item,
            },
            ordinal,
            width,
            full,
            visibility,
        ));
        if let Some(chrome) = operable_tab_chrome(full, viewport, close_allowed, config)? {
            let id = TabSceneId {
                root,
                tabs,
                tab: item,
            };
            let drag = if interaction == TabBarInteraction::Enabled {
                chrome.drag
            } else {
                LogicalRect::new(chrome.visible.x(), chrome.visible.y(), 0.0, 0.0)?
            };
            ready.push_tab_record(TabRecord::new(
                id,
                full,
                chrome.visible,
                chrome.text_layout,
                chrome.text,
                HitRegion::new(drag),
                chrome.close,
                if interaction == TabBarInteraction::Enabled {
                    chrome.close
                } else {
                    None
                },
                ordinal,
                layer,
            ));
            program_tabs.push(id);
            visible_items.insert(item);
        }
        full_tabs.push(full);
        cursor += width;
    }
    let after_tabs_bounds = (flow.after_tabs_width > 0.0)
        .then(|| LogicalRect::new(cursor, bar.y(), flow.after_tabs_width, bar.height()))
        .transpose()?;
    cursor += flow.after_tabs_width;
    let trailing_empty_bounds = if max_scroll == 0.0 {
        let trailing_start = cursor.clamp(viewport.x(), viewport.max().x());
        let trailing_width = (viewport.max().x() - trailing_start).max(0.0);
        (trailing_width > 0.0 && viewport.height() > 0.0)
            .then(|| {
                LogicalRect::from_min_max(
                    LogicalPoint::new(trailing_start, viewport.y())?,
                    viewport.max(),
                )
            })
            .transpose()?
    } else {
        None
    };
    let group_drag = if interaction == TabBarInteraction::Enabled && !items.is_empty() {
        TabGroupDragRecord::new(group_grip_bounds, trailing_empty_bounds)
    } else {
        None
    };
    ready.push_tab_bar_record(
        TabBarRecord::new(
            bar_id,
            visible_bar,
            viewport,
            layout_viewport,
            scroll,
            max_scroll,
            items
                .iter()
                .copied()
                .filter(|item| !visible_items.contains(item))
                .collect(),
            members,
            group_grip_bounds,
            group_drag,
            interaction,
            menu_geometry,
            layer,
        )
        .with_application_bounds(
            clip_slot(chrome.leading_reserved_bounds)?,
            clip_slot(chrome.trailing_reserved_bounds)?,
            after_tabs_bounds,
        )
        .with_group_menu_bounds(chrome.group_menu_bounds.filter(visible_button)),
    );
    if presentation_menu_anchor {
        let host = PresentationMenuAnchorHost::TabBar(bar_id);
        ready.push_presentation_menu_anchor_record(match presentation_menu_bounds {
            Some(bounds) => PresentationMenuAnchorRecord::ready(root, host, bounds, layer),
            None => PresentationMenuAnchorRecord::compacted(root, host, layer),
        });
    }

    let mut program_controls = Vec::new();
    if let Some([backward, forward, menu]) = control_rects {
        let enabled = interaction == TabBarInteraction::Enabled;
        if let Some(backward) = backward {
            let id = TabStripControlId::ScrollBackward(bar_id);
            ready.push_tab_strip_control_record(TabStripControlRecord::new(
                id,
                backward,
                enabled && scroll > 0.0,
                layer,
            ));
            program_controls.push(id);
        }
        if let Some(forward) = forward {
            let id = TabStripControlId::ScrollForward(bar_id);
            ready.push_tab_strip_control_record(TabStripControlRecord::new(
                id,
                forward,
                enabled && scroll < max_scroll,
                layer,
            ));
            program_controls.push(id);
        }
        if let Some(menu) = menu {
            let menu_enabled = enabled && menu_geometry.is_available();
            let id = TabStripControlId::TabListMenu(bar_id);
            ready.push_tab_strip_control_record(TabStripControlRecord::new(
                id,
                menu,
                menu_enabled,
                layer,
            ));
            program_controls.push(id);
        }
        if interaction == TabBarInteraction::Enabled
            && let Some(active_menu) = active_menu
            && let Some(menu) = menu
            && let Some(menu_metrics) = strip
                .tab_list_menu()
                .filter(|metrics| metrics.can_allocate())
        {
            compile_tab_list_menu(
                surface,
                bar_id,
                menu,
                layer,
                items,
                selected,
                &content_widths,
                menu_metrics,
                active_menu,
                ready,
            )?;
        }
    }

    ready.push_surface_program_root_operation(SurfaceProgramRootOperation::TabBar(bar_id))?;
    for tab in program_tabs {
        ready.push_surface_program_root_operation(SurfaceProgramRootOperation::Tab(tab))?;
    }
    for control in program_controls {
        ready.push_surface_program_root_operation(SurfaceProgramRootOperation::TabStripControl(
            control,
        ))?;
    }
    if presentation_menu_anchor && presentation_menu_bounds.is_some() {
        ready.push_surface_program_root_operation(
            SurfaceProgramRootOperation::PresentationMenuAnchor(root),
        )?;
    }

    if emit_drop_targets && interaction == TabBarInteraction::Enabled {
        let availability = tab_availability(policy);
        let gap_viewport = if let Some(after_tabs) = after_tabs_bounds {
            LogicalRect::new(
                viewport.x(),
                viewport.y(),
                (after_tabs.x() - viewport.x()).clamp(0.0, viewport.width()),
                viewport.height(),
            )?
        } else {
            viewport
        };
        // Both rosters have the same order. Merge them once when hidden members
        // require durable insertion indices; fully visible tabs need no mapping.
        let durable_indices = (items.len() != persistent_items.len()).then(|| {
            let mut indices = Vec::with_capacity(items.len());
            let mut visible = items.iter();
            let mut next = visible.next();
            for (index, item) in persistent_items.iter().enumerate() {
                if next == Some(item) {
                    indices.push(index);
                    next = visible.next();
                }
            }
            indices
        });
        for gap in tab_gap_geometries(gap_viewport, &full_tabs, config.splitter_thickness())? {
            ready.push_drop_target(Arc::new(DropTargetRecord::new(
                DropTargetLocation::TabGap {
                    surface,
                    root,
                    tabs,
                    // Visible gaps name durable insertion positions. Hidden
                    // members retain their relative order in the full roster.
                    index: durable_indices.as_ref().map_or(gap.index, |indices| {
                        indices
                            .get(gap.index)
                            .copied()
                            .unwrap_or(persistent_items.len())
                    }),
                },
                availability,
                HitRegion::new(gap.hit),
                layer,
                DropVisual::new(gap.visual),
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(in crate::presentation::compiler) struct TabStripControlLayout {
    pub(in crate::presentation::compiler) rects: [Option<LogicalRect>; 3],
    pub(in crate::presentation::compiler) viewport: LogicalRect,
}

pub(in crate::presentation::compiler) fn tab_strip_control_layout(
    content: LogicalRect,
    metrics: TabStripControlMetrics,
) -> Result<Option<TabStripControlLayout>, GeometryError> {
    let entries = [
        metrics.scroll_backward(),
        metrics.scroll_forward(),
        metrics.tab_list_menu(),
    ];
    let group_extent = |placement| {
        let controls = entries
            .iter()
            .flatten()
            .copied()
            .filter(|metric| metric.can_allocate() && metric.placement() == placement)
            .collect::<Vec<_>>();
        controls.iter().map(|metric| metric.extent()).sum::<f64>()
            + metrics.spacing()
                * f64::from(u32::try_from(controls.len().saturating_sub(1)).unwrap_or(u32::MAX))
    };
    let reserved_leading = group_extent(TabStripControlPlacement::ReservedLeading);
    let reserved_trailing = group_extent(TabStripControlPlacement::ReservedTrailing);
    if reserved_leading + reserved_trailing > content.width() {
        return Ok(None);
    }
    let viewport_x = (content.x() + reserved_leading).min(content.max().x());
    let viewport_right = (content.max().x() - reserved_trailing).max(viewport_x);
    let viewport = LogicalRect::from_min_max(
        LogicalPoint::new(viewport_x, content.y())?,
        LogicalPoint::new(viewport_right, content.max().y())?,
    )?;
    let overlay_leading = group_extent(TabStripControlPlacement::OverlayLeading);
    let overlay_trailing = group_extent(TabStripControlPlacement::OverlayTrailing);
    let overlay_fits = overlay_leading + overlay_trailing <= viewport.width();
    let overlay_divider = (viewport_x + overlay_leading).min(viewport_right);
    let mut rects = [None; 3];

    for placement in [
        TabStripControlPlacement::ReservedLeading,
        TabStripControlPlacement::OverlayLeading,
    ] {
        if placement == TabStripControlPlacement::OverlayLeading && !overlay_fits {
            continue;
        }
        let (mut cursor, right) = if placement == TabStripControlPlacement::ReservedLeading {
            (content.x(), viewport_x)
        } else {
            (viewport_x, overlay_divider)
        };
        for (index, metric) in entries.iter().copied().enumerate() {
            let Some(metric) =
                metric.filter(|metric| metric.can_allocate() && metric.placement() == placement)
            else {
                continue;
            };
            let next = (cursor + metric.extent()).min(right);
            rects[index] = Some(LogicalRect::from_min_max(
                LogicalPoint::new(cursor, content.y())?,
                LogicalPoint::new(next, content.max().y())?,
            )?);
            cursor = (next + metrics.spacing()).min(right);
        }
    }

    for placement in [
        TabStripControlPlacement::ReservedTrailing,
        TabStripControlPlacement::OverlayTrailing,
    ] {
        if placement == TabStripControlPlacement::OverlayTrailing && !overlay_fits {
            continue;
        }
        let (mut cursor, left) = if placement == TabStripControlPlacement::ReservedTrailing {
            (content.max().x(), viewport_right)
        } else {
            (viewport_right, overlay_divider)
        };
        for (index, metric) in entries.iter().copied().enumerate().rev() {
            let Some(metric) =
                metric.filter(|metric| metric.can_allocate() && metric.placement() == placement)
            else {
                continue;
            };
            let next = (cursor - metric.extent()).max(left);
            rects[index] = Some(LogicalRect::from_min_max(
                LogicalPoint::new(next, content.y())?,
                LogicalPoint::new(cursor, content.max().y())?,
            )?);
            cursor = (next - metrics.spacing()).max(left);
        }
    }

    Ok(rects
        .iter()
        .any(Option::is_some)
        .then_some(TabStripControlLayout { rects, viewport }))
}

#[allow(clippy::too_many_arguments)]
fn compile_tab_list_menu(
    surface: SurfaceId,
    bar: TabBarSceneId,
    anchor: LogicalRect,
    layer: SceneLayerKey,
    items: &[crate::ids::TabKey],
    selected: Option<crate::ids::TabKey>,
    content_widths: &[f64],
    metrics: crate::presentation::manifest::TabListMenuMetrics,
    active_menu: &ActiveTabListMenu,
    ready: &mut PresentationPlan,
) -> Result<(), SceneCompilationError> {
    if items.is_empty() || content_widths.len() != items.len() {
        return Ok(());
    }
    let key = TabStripStateKey::new(surface, bar);
    if active_menu.key() != key || active_menu.tabs() != items {
        return Err(SceneCompilationError::TabListMenuRosterMismatch { key });
    }
    let bounds = ready
        .popup_plane_bounds()
        .ok_or(SceneCompilationError::PopupPlaneStateMismatch)?;
    if !rect_contains(bounds, anchor) {
        return Err(SceneCompilationError::TabListMenuAnchorOutsidePopupPlane { key });
    }
    let row_gap_count = u32::try_from(items.len().saturating_sub(1))
        .map_err(|_| SceneCompilationError::GeometryCountOverflow)?;
    let row_heights = checked_sum(
        std::iter::repeat_n(metrics.row_height(), items.len()),
        "tab-list row heights",
    )?;
    let row_spacing = checked_sum(
        [metrics.row_spacing() * f64::from(row_gap_count)],
        "tab-list row spacing",
    )?;
    let rows_height = checked_sum([row_heights, row_spacing], "tab-list rows height")?;
    let vertical_padding = checked_sum(
        [metrics.vertical_padding(), metrics.vertical_padding()],
        "tab-list vertical padding",
    )?;
    let natural_height = checked_sum([rows_height, vertical_padding], "tab-list natural height")?;
    let desired_popup_height = natural_height.min(metrics.maximum_height());
    let space_below = (bounds.max().y() - anchor.max().y()).max(0.0);
    let space_above = (anchor.y() - bounds.y()).max(0.0);
    let (opens_below, available_height) = if desired_popup_height <= space_below {
        (true, space_below)
    } else if desired_popup_height <= space_above {
        (false, space_above)
    } else if space_below >= space_above {
        (true, space_below)
    } else {
        (false, space_above)
    };
    let popup_height = desired_popup_height.min(available_height);
    if popup_height <= 0.0 {
        return Err(SceneCompilationError::TabListMenuPopupSpaceUnavailable { key });
    }
    let needs_scrollbar = natural_height > popup_height;
    let horizontal_padding = checked_sum(
        [metrics.horizontal_padding(), metrics.horizontal_padding()],
        "tab-list horizontal padding",
    )?;
    let content_width = checked_sum(
        [
            content_widths.iter().copied().fold(0.0, f64::max),
            horizontal_padding,
        ],
        "tab-list content width",
    )?;
    let popup_width = checked_sum(
        [
            content_width,
            if needs_scrollbar {
                metrics.scrollbar_extent()
            } else {
                0.0
            },
        ],
        "tab-list popup width",
    )?
    .max(anchor.width())
    .min(bounds.width());
    if popup_width <= 0.0 {
        return Ok(());
    }
    // Clamp in popup-plane coordinates so a full-width menu has exactly zero travel.
    let x = bounds.x()
        + (anchor.max().x() - bounds.x() - popup_width).clamp(0.0, bounds.width() - popup_width);
    let y = if opens_below {
        anchor.max().y()
    } else {
        (anchor.y() - popup_height).max(bounds.y())
    };
    let popup = LogicalRect::from_min_max(
        LogicalPoint::new(x, y)?,
        LogicalPoint::new(
            (x + popup_width).min(bounds.max().x()),
            (y + popup_height).min(bounds.max().y()),
        )?,
    )?;
    let viewport_width = (popup.width()
        - horizontal_padding
        - if needs_scrollbar {
            metrics.scrollbar_extent()
        } else {
            0.0
        })
    .max(0.0);
    let viewport_height = (popup.height() - vertical_padding).max(0.0);
    if viewport_width <= 0.0 || viewport_height <= 0.0 {
        return Ok(());
    }
    let viewport = LogicalRect::new(
        popup.x() + metrics.horizontal_padding(),
        popup.y() + metrics.vertical_padding(),
        viewport_width,
        viewport_height,
    )?;
    let maximum_scroll = (rows_height - viewport.height()).max(0.0);
    let scroll = active_menu.scroll_offset().map_or_else(
        || {
            items
                .iter()
                .position(|&item| item == active_menu.focus())
                .map_or(0.0, |ordinal| {
                    let start = metrics.row_offset(ordinal);
                    crate::presentation::tab_strip::reveal_scroll_offset(
                        0.0,
                        maximum_scroll,
                        viewport.height(),
                        start,
                        start + metrics.row_height(),
                    )
                })
        },
        |offset| offset.clamp(0.0, maximum_scroll),
    );
    let mut rows = Vec::with_capacity(items.len());
    for (ordinal, item) in items.iter().copied().enumerate() {
        let y = viewport.y() + (metrics.row_offset(ordinal) - scroll);
        let row = LogicalRect::new(viewport.x(), y, viewport.width(), metrics.row_height())?;
        let hit = intersect_rect(row, viewport)?.map(HitRegion::new);
        rows.push(TabListMenuRowRecord::new(
            TabSceneId {
                root: bar.root,
                tabs: bar.tabs,
                tab: item,
            },
            ordinal,
            row,
            hit,
            selected == Some(item),
            active_menu.focus() == item,
        ));
    }
    ready.push_tab_list_menu_record(TabListMenuRecord::new(
        active_menu.session(),
        bar,
        popup,
        viewport,
        scroll,
        maximum_scroll,
        metrics,
        rows,
        layer,
    ));
    Ok(())
}

pub(in crate::presentation::compiler) struct TabChromeGeometry {
    pub(in crate::presentation::compiler) visible: LogicalRect,
    pub(in crate::presentation::compiler) text_layout: LogicalRect,
    pub(in crate::presentation::compiler) text: LogicalRect,
    pub(in crate::presentation::compiler) drag: LogicalRect,
    pub(in crate::presentation::compiler) close: Option<LogicalRect>,
}

pub(in crate::presentation::compiler) fn operable_tab_chrome(
    full: LogicalRect,
    viewport: LogicalRect,
    close_allowed: bool,
    config: &DockPresentationConfig,
) -> Result<Option<TabChromeGeometry>, GeometryError> {
    let Some(visible) = intersect_rect(full, viewport)? else {
        return Ok(None);
    };
    let operable = config
        .tab_close_extent()
        .min(config.tab_min_width())
        .min(full.width());
    if visible.width() < operable {
        return Ok(None);
    }
    let allocated_close = if close_allowed {
        let size = config
            .tab_close_extent()
            .min(full.height())
            .min(full.width());
        if size <= 0.0 {
            return Ok(None);
        }
        let padding = config
            .tab_horizontal_padding()
            .min((full.width() - size).max(0.0));
        let close = LogicalRect::new(
            full.max().x() - padding - size,
            full.y() + (full.height() - size) * 0.5,
            size,
            size,
        )?;
        Some(close)
    } else {
        None
    };
    let close = if let Some(close) = allocated_close {
        if rect_contains(visible, close) {
            Some(close)
        } else if visible.x() > full.x()
            && visible.max().x() == full.max().x()
            && visible.y() == full.y()
            && visible.max().y() == full.max().y()
        {
            // A left-clipped tail must retain complete close chrome before
            // it can expose selection or drag input beneath that control.
            return Ok(None);
        } else {
            None
        }
    } else {
        None
    };
    let drag = if let Some(close) = close {
        LogicalRect::from_min_max(
            visible.min(),
            LogicalPoint::new(close.x().max(visible.x()), visible.max().y())?,
        )?
    } else {
        visible
    };
    if drag.width() < operable || !rect_has_area(drag) {
        return Ok(None);
    }
    let padding = config.tab_horizontal_padding().min(full.width());
    let text_left = (full.x() + padding).min(full.max().x());
    let text_right = allocated_close.map_or(full.max().x() - padding, |close| close.x() - padding);
    let text_layout = LogicalRect::from_min_max(
        LogicalPoint::new(text_left, full.y())?,
        LogicalPoint::new(text_right.clamp(text_left, full.max().x()), full.max().y())?,
    )?;
    let text = clip_rect(text_layout, visible)?;
    Ok(Some(TabChromeGeometry {
        visible,
        text_layout,
        text,
        drag,
        close,
    }))
}

struct TabGapGeometry {
    index: usize,
    hit: LogicalRect,
    visual: LogicalRect,
}

fn tab_gap_geometries(
    viewport: LogicalRect,
    tabs: &[LogicalRect],
    marker_width: f64,
) -> Result<Vec<TabGapGeometry>, GeometryError> {
    if tabs.is_empty() {
        return Ok(Vec::new());
    }
    let centers = tabs
        .iter()
        .map(|tab| tab.x() + tab.width() * 0.5)
        .collect::<Vec<_>>();
    let mut gaps = Vec::with_capacity(tabs.len() + 1);
    for index in 0..=tabs.len() {
        let left = index
            .checked_sub(1)
            .map_or(viewport.x(), |previous| centers[previous]);
        let right = centers.get(index).copied().unwrap_or(viewport.max().x());
        let hit_left = left.max(viewport.x());
        let hit_right = right.min(viewport.max().x());
        if hit_right <= hit_left {
            continue;
        }
        let hit = LogicalRect::new(
            hit_left,
            viewport.y(),
            hit_right - hit_left,
            viewport.height(),
        )?;
        let marker_x = tabs.get(index).map_or(viewport.max().x(), |tab| tab.x());
        let width = marker_width.min(viewport.width());
        if width <= 0.0 {
            continue;
        }
        let marker_min = (marker_x - width * 0.5)
            .clamp(viewport.x(), (viewport.max().x() - width).max(viewport.x()));
        // A passive marker may be narrower than one ULP at this tab strip's
        // absolute position. Round its corners outward within the viewport.
        let marker_max = (marker_min + width)
            .max(marker_min.next_up())
            .min(viewport.max().x());
        let marker_min = marker_min.min(marker_max.next_down()).max(viewport.x());
        let visual = LogicalRect::from_min_max(
            LogicalPoint::new(marker_min, viewport.y())?,
            LogicalPoint::new(marker_max, viewport.max().y())?,
        )?;
        gaps.push(TabGapGeometry { index, hit, visual });
    }
    Ok(gaps)
}

#[cfg(test)]
mod tests;
