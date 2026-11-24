//! Pane records, tab chrome and menu geometry validation.

use super::*;

pub(super) fn validate_presentation_menu_anchors(
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
pub(super) fn expected_pane_records(
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
pub(super) fn validate_pane_and_tab_records(
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
            || !rect_contains(ready.facts.bounds, record.clip())
        {
            return Err(SceneBuildError::InvalidPaneRecord {
                surface: ready.surface,
                id: *id,
            });
        }
    }

    validate_tab_records(ready, record_order, workspace, workspace_index, policy)
}

pub(super) fn validate_tab_records(
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
            && ready
                .layout_facts()
                .and_then(|facts| facts.root(pane.id().root))
                .and_then(|root| root.tab_bar_visibility(pane.id().tabs))
                == Some(TabBarVisibility::Visible);
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
    let expects_header = |pane: &TabsRecord| {
        pane.bounds().width() > 0.0
            && pane.content_bounds().y() > pane.bounds().y()
            && pane_policy(pane).visibility() == TabBarVisibility::Visible
    };
    let expected_bars = panes
        .iter(&ready.facts.tabs_records)
        .filter(|(_, pane)| expects_header(pane))
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
        if !expects_header(pane) {
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
                        facts.root(id.root)?.projection().nodes.get(&id.tabs)?.rect,
                        pane.clip(),
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
                    .clamp(bar.viewport().x(), bar.viewport().max().x());
                let expected_trailing = !items.is_empty()
                    && bar.maximum_scroll_offset() == 0.0
                    && bar.viewport().height() > 0.0
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
                    full.x() == bar.layout_viewport().x() - bar.scroll_offset()
                } else {
                    bar.members()[ordinal - 1].full_bounds().max().x() == full.x()
                };
                member.ordinal() == ordinal
                    && tab.root == bar_id.root
                    && tab.tabs == bar_id.tabs
                    && items.get(ordinal).copied() == Some(tab.tab)
                    && rect_has_area(full)
                    && full.y() == bar.layout_viewport().y()
                    && full.height() == bar.layout_viewport().height()
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
                // Clipping may hide close chrome while leaving enough tab
                // body for selection and dragging.
                (true, None) => {
                    record.visible_bounds().max().x() < record.full_bounds().max().x()
                        || record.visible_bounds().y() != record.full_bounds().y()
                        || record.visible_bounds().max().y() != record.full_bounds().max().y()
                }
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
        && menu_control
            .is_some_and(|control| rects_overlap_with_area(popup_plane_bounds, control.bounds()))
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
