//! Source-aware projection of a transaction candidate from retained measurements.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use thiserror::Error;

use super::surface::{
    clip_rect, compose_root_sizing, operable_tab_chrome, rect_contains,
    tab_strip_optional_chrome_layout,
};
use super::tab_strip_layout::{TabStripFlow, desired_tab_width, select_menu_host, tab_leaf_bounds};
use crate::geometry::{GeometryError, LogicalRect, PaneSizing};
use crate::graph::command::{MovePayload, TabTarget};
use crate::graph::error::CommandError;
use crate::graph::workspace::RootPresentationOwner;
use crate::graph::{Node, TabLabel, Workspace};
use crate::ids::{ItemId, NodeId, RootId, TabKey};
use crate::policy::{DockPolicySnapshot, TabBarPolicy, TabBarVisibility};
use crate::presentation::layout::{
    LayoutError, LayoutMetrics, LayoutMetricsError, LayoutProjection, measure_root_sizing,
};
use crate::presentation::manifest::{PaneSizingKey, TabIntrinsicKey, TabStripKey};
use crate::presentation::scene::{
    PresentationLayoutFacts, PresentationPlan, TabBarSceneId, TabSceneId,
};

#[cfg(test)]
mod tests;

/// Immutable target-strip geometry derived from the unique transaction candidate.
#[derive(Debug, PartialEq)]
pub(crate) struct FutureTabStrip {
    pub(crate) bounds: LogicalRect,
    pub(crate) viewport: LogicalRect,
    pub(crate) scroll_offset: f64,
    pub(crate) maximum_scroll_offset: f64,
    pub(crate) tabs: Box<[FutureTab]>,
}

/// One future member, including members clipped out of the target viewport.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FutureTab {
    pub(crate) layout_title: Option<Arc<str>>,
    pub(crate) key: TabKey,
    pub(crate) ordinal: usize,
    pub(crate) full_bounds: LogicalRect,
    pub(crate) clip_bounds: LogicalRect,
    pub(crate) visible_bounds: Option<LogicalRect>,
    pub(crate) text_layout_bounds: Option<LogicalRect>,
    pub(crate) text_bounds: Option<LogicalRect>,
    pub(crate) close_visual_bounds: Option<LogicalRect>,
    pub(crate) selected: bool,
    pub(crate) payload: bool,
}

pub(crate) struct FutureTabStripProjection {
    pub(crate) insertion: LogicalRect,
    pub(crate) strip: Option<Arc<FutureTabStrip>>,
}

/// A reveal belongs to the exact presented single-member roster. New or
/// changed candidate rosters return to their declared automatic visibility.
pub(crate) fn candidate_tab_bar_policy(
    original: &Workspace,
    candidate: &Workspace,
    policy: &DockPolicySnapshot,
    target_plan: &PresentationPlan,
    root: RootId,
    tabs: NodeId,
) -> Result<TabBarPolicy, FutureLayoutProjectionError> {
    let Some(Node::Tabs { .. }) = candidate.node(tabs) else {
        return Err(FutureLayoutProjectionError::TargetIsNotTabs { root, node: tabs });
    };
    let items = candidate.visible_tab_keys(tabs);
    let central = candidate
        .root(root)
        .ok_or(FutureLayoutProjectionError::MissingRoot { root })?
        .central
        == Some(tabs);
    let rule = candidate.pane_local_target_rule(root, tabs, central)?;
    let declaration = policy.tab_bar_declaration(target_plan.surface(), Some(rule));
    let revealed = declaration.visibility() == TabBarVisibility::AutoHideSingle
        && items.len() == 1
        && original.visible_tab_keys(tabs) == items
        && target_plan
            .layout_facts()
            .and_then(|facts| facts.root(root))
            .and_then(|facts| facts.tab_bar_visibility(tabs))
            == Some(TabBarVisibility::Visible);
    Ok(declaration.resolved(items.len(), revealed))
}

/// Reprojects one future root using only measurements retained by presented outputs.
pub(crate) fn project_future_root(
    original: &Workspace,
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    target_plan: &PresentationPlan,
    source_layout_facts: Option<&PresentationLayoutFacts>,
    root: RootId,
) -> Result<Option<LayoutProjection>, FutureLayoutProjectionError> {
    let Some(target_facts) = target_plan.layout_facts() else {
        return Ok(None);
    };
    let target_root = target_facts
        .root(root)
        .ok_or(FutureLayoutProjectionError::TargetRootNotPresented { root })?;
    let mut pane_sizings = BTreeMap::<PaneSizingKey, PaneSizing>::new();
    collect_pane_sizings(target_facts, &mut pane_sizings)?;
    if let Some(source_facts) = source_layout_facts {
        if source_facts.config() != target_facts.config() {
            return Err(FutureLayoutProjectionError::PresentationConfigMismatch);
        }
        collect_pane_sizings(source_facts, &mut pane_sizings)?;
    }

    match workspace.presentation_for_root(root) {
        Some(RootPresentationOwner::Main { .. })
        | Some(RootPresentationOwner::Contained { .. }) => {}
        None => return Err(FutureLayoutProjectionError::TargetRootNotPresented { root }),
    };
    let topology = workspace
        .root_structure(root)
        .ok_or(LayoutError::UnfinishedRoot { root })?;
    let mut measured_items = BTreeMap::new();
    let mut visibility = BTreeMap::new();
    for record in &topology.nodes {
        let node = record.id;
        match workspace
            .node(node)
            .ok_or(FutureLayoutProjectionError::MissingNode { root, node })?
        {
            Node::Pane { item } => {
                if !workspace.item_is_hidden(*item) {
                    measured_items.insert(
                        *item,
                        candidate_pane_sizing(original, &pane_sizings, root, node, *item)?,
                    );
                }
            }
            Node::Tabs { .. } => {
                if !workspace.node_has_visible_content(node) {
                    continue;
                }
                visibility.insert(
                    node,
                    candidate_tab_bar_policy(original, workspace, policy, target_plan, root, node)?
                        .visibility(),
                );
                for item in workspace
                    .visible_tab_keys(node)
                    .iter()
                    .copied()
                    .filter_map(TabKey::item)
                {
                    measured_items.insert(
                        item,
                        candidate_pane_sizing(original, &pane_sizings, root, node, item)?,
                    );
                }
            }
            Node::Split { .. } | Node::Grid { .. } => {}
        }
    }
    let (sizing, insets) = compose_root_sizing(
        workspace,
        root,
        target_facts.config(),
        &measured_items,
        &visibility,
    )?;
    measure_root_sizing(
        workspace,
        root,
        &sizing,
        LayoutMetrics::new(target_facts.config().splitter_thickness())?
            .with_pane_overflow_policy(target_facts.config().pane_overflow_policy()),
    )?
    .with_tab_content_insets(&insets)
    .project(
        super::root_controls_layout(
            workspace,
            root,
            target_root.allocation(),
            target_facts.config(),
        )?
        .map_or(target_root.allocation(), |controls| controls.content),
        &[],
    )
    .map(Some)
    .map_err(FutureLayoutProjectionError::Layout)
}

/// Projects every candidate member and the first payload tab's insertion marker once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn project_future_tab_strip(
    original: &Workspace,
    candidate: &Workspace,
    policy: &DockPolicySnapshot,
    target_plan: &PresentationPlan,
    source_layout_facts: Option<&PresentationLayoutFacts>,
    target: &TabTarget,
    payload: &MovePayload,
    projection: &LayoutProjection,
) -> Result<FutureTabStripProjection, FutureLayoutProjectionError> {
    let target_facts = target_plan
        .layout_facts()
        .ok_or(FutureLayoutProjectionError::TargetLayoutFactsMissing)?;
    let config = target_facts.config();
    if let Some(source_facts) = source_layout_facts
        && source_facts.config() != config
    {
        return Err(FutureLayoutProjectionError::PresentationConfigMismatch);
    }
    let projected_bounds = projection
        .nodes
        .get(&target.tabs())
        .map(|result| result.rect)
        .ok_or(FutureLayoutProjectionError::MissingNodeProjection {
            root: target.root(),
            node: target.tabs(),
        })?;
    let target_root = target_facts.root(target.root()).ok_or(
        FutureLayoutProjectionError::TargetRootNotPresented {
            root: target.root(),
        },
    )?;
    let Node::Tabs { .. } =
        candidate
            .node(target.tabs())
            .ok_or(FutureLayoutProjectionError::MissingNode {
                root: target.root(),
                node: target.tabs(),
            })?
    else {
        return Err(FutureLayoutProjectionError::TargetIsNotTabs {
            root: target.root(),
            node: target.tabs(),
        });
    };
    let items = candidate.visible_tab_keys(target.tabs());
    let selected = candidate.visible_tab_selection(target.tabs());
    let target_policy = candidate_tab_bar_policy(
        original,
        candidate,
        policy,
        target_plan,
        target.root(),
        target.tabs(),
    )?;
    let height = if target_policy.visibility() == TabBarVisibility::Visible {
        config.tab_bar_height()
    } else {
        0.0
    };
    let geometry = tab_leaf_bounds(
        clip_rect(projected_bounds, target_root.allocation())?,
        target_root.bounds(),
        height,
        None,
    )?;
    let node_bounds = geometry.bounds;
    let visible_bar = geometry.header;
    if visible_bar.width() <= 0.0 || visible_bar.height() <= 0.0 {
        return Ok(FutureTabStripProjection {
            insertion: node_bounds,
            strip: None,
        });
    }
    let original_selected = match original.node(target.tabs()) {
        Some(Node::Tabs { .. }) => original.visible_tab_selection(target.tabs()),
        // Center docking promotes a bare destination to Tabs in place. The
        // retained Pane had no tab selection or measured strip to inherit.
        Some(Node::Pane { .. }) => None,
        Some(Node::Split { .. } | Node::Grid { .. }) => {
            return Err(FutureLayoutProjectionError::TargetIsNotTabs {
                root: target.root(),
                node: target.tabs(),
            });
        }
        None => {
            return Err(FutureLayoutProjectionError::MissingNode {
                root: target.root(),
                node: target.tabs(),
            });
        }
    };

    let strip_key = TabStripKey::new(
        target.surface(),
        TabBarSceneId {
            root: target.root(),
            tabs: target.tabs(),
        },
    );
    let strip = target_facts
        .tab_strip(strip_key)
        .ok_or(FutureLayoutProjectionError::TabStripMeasurementUnavailable { key: strip_key })?;
    let include_presentation_menu = matches!(
        candidate.presentation_for_root(target.root()),
        Some(RootPresentationOwner::Main { surface }) if surface == target.surface()
    ) && future_presentation_menu_tab_bar(
        original,
        candidate,
        policy,
        target_plan,
        target_facts,
        target.surface(),
        target.root(),
        projection,
    )? == Some(target.tabs());
    let bar = geometry.header_allocation;
    let chrome = tab_strip_optional_chrome_layout(
        bar,
        items.len(),
        strip,
        config,
        include_presentation_menu,
    )?;
    let mut desired_widths = Vec::with_capacity(items.len());
    let mut close_allowed = Vec::with_capacity(items.len());
    for item in items.iter().copied() {
        let intrinsic = candidate_tab_intrinsic(
            candidate,
            target_plan,
            source_layout_facts,
            target.surface(),
            target.root(),
            target.tabs(),
            item,
        )?;
        let close =
            super::tab_close_capability(candidate, policy, target.root(), item).allows_close();
        desired_widths.push(desired_tab_width(intrinsic.content_width(), close, config));
        close_allowed.push(close);
    }

    let layout_viewport = chrome.viewport;
    let viewport = clip_rect(layout_viewport, visible_bar)?;
    let flow = TabStripFlow::allocate(
        &desired_widths,
        layout_viewport.width(),
        strip.after_tabs_width(),
        config.tab_min_width(),
    )?;
    let bar_id = TabBarSceneId {
        root: target.root(),
        tabs: target.tabs(),
    };
    let previous_bar = target_plan
        .tab_bar_records()
        .iter()
        .find(|bar| *bar.id() == bar_id);
    let requested_scroll = previous_bar.map_or(0.0, |bar| bar.scroll_offset());
    let selected_index =
        selected.and_then(|selected| items.iter().position(|item| *item == selected));
    let viewport_shrank =
        previous_bar.is_some_and(|bar| layout_viewport.width() < bar.layout_viewport().width());
    let scroll = flow.scroll_offset(
        requested_scroll,
        layout_viewport.width(),
        selected_index,
        original_selected != selected || viewport_shrank,
    );

    let payload_items = payload_items(original, payload);
    let first_payload = payload_items
        .first()
        .copied()
        .ok_or(FutureLayoutProjectionError::PayloadContainsNoItems)?;
    let payload_items = payload_items.into_iter().collect::<BTreeSet<_>>();
    let mut tabs = Vec::with_capacity(items.len());
    let mut cursor = layout_viewport.x() - scroll;
    for (ordinal, ((item, width), close)) in items
        .iter()
        .copied()
        .zip(flow.widths.iter().copied())
        .zip(close_allowed)
        .enumerate()
    {
        let full_bounds = LogicalRect::new(cursor, bar.y(), width, bar.height())?;
        let chrome = operable_tab_chrome(full_bounds, viewport, close, config)?;
        tabs.push(FutureTab {
            layout_title: match candidate.tab_label(target.tabs(), item) {
                Some(crate::graph::TabLabel::Layout { title }) => Some(Arc::from(title)),
                _ => None,
            },
            key: item,
            ordinal,
            full_bounds,
            clip_bounds: viewport,
            visible_bounds: chrome.as_ref().map(|chrome| chrome.visible),
            text_layout_bounds: chrome.as_ref().map(|chrome| chrome.text_layout),
            text_bounds: chrome.as_ref().map(|chrome| chrome.text),
            close_visual_bounds: chrome.as_ref().and_then(|chrome| chrome.close),
            selected: selected == Some(item),
            payload: payload_items.contains(&item),
        });
        cursor += width;
    }
    let strip = Some(Arc::new(FutureTabStrip {
        bounds: visible_bar,
        viewport,
        scroll_offset: scroll,
        maximum_scroll_offset: flow.maximum_scroll_offset(layout_viewport.width()),
        tabs: tabs.into_boxed_slice(),
    }));
    let payload_index = items
        .iter()
        .position(|item| *item == first_payload)
        .ok_or(FutureLayoutProjectionError::PayloadTabMissing { tab: first_payload })?;
    let payload_start = flow
        .tab_range(payload_index)
        .ok_or(FutureLayoutProjectionError::PayloadTabMissing { tab: first_payload })?
        .0;
    let marker_x = layout_viewport.x() - scroll + payload_start;
    let marker_width = config.splitter_thickness().min(viewport.width());
    if marker_width <= 0.0 {
        return Ok(FutureTabStripProjection {
            insertion: node_bounds,
            strip,
        });
    }
    let marker_min = (marker_x - marker_width * 0.5).clamp(
        viewport.x(),
        (viewport.max().x() - marker_width).max(viewport.x()),
    );
    Ok(FutureTabStripProjection {
        insertion: LogicalRect::new(marker_min, viewport.y(), marker_width, viewport.height())?,
        strip,
    })
}

#[allow(clippy::too_many_arguments)]
fn future_presentation_menu_tab_bar(
    original: &Workspace,
    candidate: &Workspace,
    policy: &DockPolicySnapshot,
    target_plan: &PresentationPlan,
    target_facts: &PresentationLayoutFacts,
    surface: crate::ids::SurfaceId,
    root: RootId,
    projection: &LayoutProjection,
) -> Result<Option<NodeId>, FutureLayoutProjectionError> {
    let root_record = candidate
        .root(root)
        .ok_or(FutureLayoutProjectionError::MissingRoot { root })?;
    let config = target_facts.config();
    let central = root_record.central;
    let visible_menu_available = |tabs| -> Result<Option<bool>, FutureLayoutProjectionError> {
        let Some(Node::Tabs { .. }) = candidate.node(tabs) else {
            return Ok(None);
        };
        if !candidate.node_has_visible_content(tabs) {
            return Ok(None);
        }
        let items = candidate.visible_tab_keys(tabs);
        let tab_bar =
            candidate_tab_bar_policy(original, candidate, policy, target_plan, root, tabs)?;
        if tab_bar.visibility() == TabBarVisibility::Hidden {
            return Ok(None);
        }
        let projected_bounds = projection
            .nodes
            .get(&tabs)
            .map(|result| result.rect)
            .ok_or(FutureLayoutProjectionError::MissingNodeProjection { root, node: tabs })?;
        let target_root = target_facts
            .root(root)
            .ok_or(FutureLayoutProjectionError::TargetRootNotPresented { root })?;
        let geometry = tab_leaf_bounds(
            clip_rect(projected_bounds, target_root.allocation())?,
            target_root.bounds(),
            config.tab_bar_height(),
            None,
        )?;
        if geometry.header.width() <= 0.0 || geometry.header.height() <= 0.0 {
            return Ok(None);
        }
        let key = TabStripKey::new(surface, TabBarSceneId { root, tabs });
        let strip = target_facts
            .tab_strip(key)
            .ok_or(FutureLayoutProjectionError::TabStripMeasurementUnavailable { key })?;
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
        candidate,
        surface,
        root,
        root_record.node,
        central,
        visible_menu_available,
    )
}

fn candidate_tab_intrinsic(
    candidate: &Workspace,
    target_plan: &PresentationPlan,
    source_layout_facts: Option<&PresentationLayoutFacts>,
    candidate_surface: crate::ids::SurfaceId,
    candidate_root: RootId,
    candidate_tabs: NodeId,
    tab: TabKey,
) -> Result<crate::presentation::manifest::TabIntrinsic, FutureLayoutProjectionError> {
    let candidate_key = TabIntrinsicKey::new(
        candidate_surface,
        TabSceneId {
            root: candidate_root,
            tabs: candidate_tabs,
            tab,
        },
    );
    let label = candidate
        .tab_label(candidate_tabs, tab)
        .ok_or(FutureLayoutProjectionError::TabIntrinsicUnavailable { key: candidate_key })?;
    // Canonicalization can retain a layout key while changing its fallback
    // label, for example when a Split boundary becomes Tabs. Only measurements
    // whose own frozen descriptor still matches can follow that identity.
    let compatible_facts = || {
        layout_facts(target_plan, source_layout_facts).filter(move |facts| match tab.item() {
            Some(item) => label == TabLabel::Item(item),
            None => tab.layout().is_some_and(|node| {
                facts
                    .roots()
                    .any(|root| root.node_label(node) == Some(label))
            }),
        })
    };
    if let Some(intrinsic) = compatible_facts().find_map(|facts| facts.tab_intrinsic(candidate_key))
    {
        return Ok(intrinsic);
    }

    compatible_facts()
        .find_map(|facts| facts.tab_intrinsic_for_tab(tab))
        .ok_or(FutureLayoutProjectionError::TabIntrinsicUnavailable { key: candidate_key })
}

fn layout_facts<'a>(
    target_plan: &'a PresentationPlan,
    source_layout_facts: Option<&'a PresentationLayoutFacts>,
) -> impl Iterator<Item = &'a PresentationLayoutFacts> {
    std::iter::once(target_plan.layout_facts())
        .chain(std::iter::once(source_layout_facts))
        .flatten()
}

fn payload_items(original: &Workspace, payload: &MovePayload) -> Vec<TabKey> {
    match payload {
        MovePayload::Item(source) => vec![TabKey::for_item(source.item())],
        MovePayload::Subtree(source) => match original.node(source.node()) {
            Some(Node::Pane { item }) => vec![TabKey::for_item(*item)],
            Some(_) => vec![TabKey::for_layout(source.node())],
            None => Vec::new(),
        },
        MovePayload::Tabs(source) => match original.node(source.node()) {
            Some(Node::Tabs { .. }) => original.visible_tab_keys(source.node()).into_owned(),
            _ => Vec::new(),
        },
    }
}

fn collect_pane_sizings(
    facts: &PresentationLayoutFacts,
    minimums: &mut BTreeMap<PaneSizingKey, PaneSizing>,
) -> Result<(), FutureLayoutProjectionError> {
    for (key, minimum) in facts.roots().flat_map(|root| root.pane_sizings()) {
        if let Some(previous) = minimums.insert(key, minimum)
            && previous != minimum
        {
            return Err(FutureLayoutProjectionError::ConflictingPaneSizing { key });
        }
    }
    Ok(())
}

fn candidate_pane_sizing(
    original: &Workspace,
    minimums: &BTreeMap<PaneSizingKey, PaneSizing>,
    candidate_root: RootId,
    candidate_tabs: NodeId,
    item: ItemId,
) -> Result<PaneSizing, FutureLayoutProjectionError> {
    let candidate_key = PaneSizingKey::new(candidate_root, item);
    if let Some(minimum) = minimums.get(&candidate_key) {
        return Ok(*minimum);
    }

    let source = original.capture_item_source_by_id(item)?.ok_or(
        FutureLayoutProjectionError::PaneSizingUnavailable {
            root: candidate_root,
            tabs: candidate_tabs,
            item,
        },
    )?;
    let source_key = PaneSizingKey::new(source.root(), item);
    minimums
        .get(&source_key)
        .copied()
        .ok_or(FutureLayoutProjectionError::PaneSizingUnavailable {
            root: candidate_root,
            tabs: candidate_tabs,
            item,
        })
}

/// Failure to project the post-transaction layout from retained authority.
#[derive(Debug, Error)]
pub(crate) enum FutureLayoutProjectionError {
    #[error("target root {root} has no retained layout facts")]
    TargetRootNotPresented { root: RootId },
    #[error("candidate workspace is missing target root {root}")]
    MissingRoot { root: RootId },
    #[error("candidate root {root} is missing node {node:?}")]
    MissingNode { root: RootId, node: NodeId },
    #[error("candidate projection omitted node {root}/{node:?}")]
    MissingNodeProjection { root: RootId, node: NodeId },
    #[error("candidate target {root}/{node:?} is not a Tabs container")]
    TargetIsNotTabs { root: RootId, node: NodeId },
    #[error("pane sizing for item {item} in {root}/{tabs:?} was not presented")]
    PaneSizingUnavailable {
        root: RootId,
        tabs: NodeId,
        item: ItemId,
    },
    #[error("retained outputs disagree about the pane sizing for exact key {key:?}")]
    ConflictingPaneSizing { key: PaneSizingKey },
    #[error("target presentation omitted retained layout facts")]
    TargetLayoutFactsMissing,
    #[error("tab intrinsic measurement {key:?} is unavailable")]
    TabIntrinsicUnavailable { key: TabIntrinsicKey },
    #[error("tab-strip measurement {key:?} is unavailable")]
    TabStripMeasurementUnavailable { key: TabStripKey },
    #[error("drag payload contains no item")]
    PayloadContainsNoItems,
    #[error("drag payload tab {tab:?} vanished from the candidate target")]
    PayloadTabMissing { tab: TabKey },
    #[error("source and target outputs were compiled with different presentation configs")]
    PresentationConfigMismatch,
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Geometry(#[from] GeometryError),
    #[error(transparent)]
    Metrics(#[from] LayoutMetricsError),
    #[error(transparent)]
    Scene(#[from] super::SceneCompilationError),
    #[error(transparent)]
    Layout(#[from] LayoutError),
}
