//! Public captured-tab actions preserve direct-child scope and atomic approvals.

use super::*;
use crate::model::{DockspaceActionOutcome, DockspaceAxis, PreparedTab, TabLabel};
use crate::policy::DockItemRule;

const D: ItemId = ItemId::new(5);

fn composed_session() -> DockspaceSession {
    let tools = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([FIRST, SECOND]), DockspaceNode::pane(D)],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::composed_tabs([DockspaceNode::pane(THIRD), tools]),
            ),
        ),
        DockspaceSurfaceLayout::new(
            OTHER_SURFACE,
            DockspaceRootLayout::new(OTHER_ROOT, DockspaceNode::tabs([FOURTH])),
        ),
    ])
    .unwrap();
    let mut policy = DockPolicy::default();
    policy.set_close_capability(CloseCapability::DeferredAllowed);
    DockspaceSession::from_layout(layout, policy).unwrap()
}

fn tools(session: &DockspaceSession) -> PreparedTab {
    let key = session
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap()
        .tabs()
        .unwrap()
        .tabs()[1];
    session.capture_tab(key).unwrap()
}

#[test]
fn composable_tab_move_keeps_the_complete_subtree_as_one_destination_tab() {
    let mut session = composed_session();
    let tab = tools(&session);
    let key = tab.key();
    let node_key = crate::ids::NodeKey(key.layout().unwrap());
    assert_eq!(
        session.view().node(node_key).unwrap().root().unwrap().id(),
        ROOT
    );
    let prepared = session.prepare_dock_tab(tab, DockPlacement::Center(DockAnchor::Item(FOURTH)));
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_action(prepared).unwrap();
    let report = commit(frame);
    assert!(matches!(report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::ProductActionApplied(DockspaceActionOutcome::TabDocked {
            tab, source_root: ROOT, target_root: OTHER_ROOT, items, changed: true,
        })) if *tab == key && items == &[FIRST, SECOND, D]
    ));
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .content()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [THIRD]
    );
    let target = session
        .view()
        .root(OTHER_ROOT)
        .unwrap()
        .content()
        .unwrap()
        .tabs()
        .unwrap();
    assert_eq!(target.tabs(), [TabKey::for_item(FOURTH), key]);
    assert_eq!(
        session
            .view()
            .root(OTHER_ROOT)
            .unwrap()
            .active_items()
            .collect::<Vec<_>>(),
        [FIRST, D]
    );
    assert_eq!(
        session
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [FIRST, SECOND]
    );
    assert!(session.view().item(D).unwrap().tabs().is_none());
    assert_eq!(
        session.view().node(node_key).unwrap().root().unwrap().id(),
        OTHER_ROOT
    );
}

#[test]
fn a_layout_tab_occupies_the_root_only_when_every_real_descendant_belongs_to_it() {
    for extra in [false, true] {
        let tools = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::tabs([FIRST, SECOND]), DockspaceNode::pane(D)],
        )
        .unwrap()
        .named("Tools");
        let mut children = vec![tools];
        if extra {
            children.push(DockspaceNode::pane(THIRD));
        }
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::composed_tabs(children)),
        )])
        .unwrap()
        .with_hidden_items([SECOND])
        .unwrap();
        let session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        let root = session.view().root(ROOT).unwrap();
        let key = root.content().unwrap().tabs().unwrap().tabs()[0];
        let tab = session.capture_tab(key).unwrap();
        assert_eq!(root.contains_only_tab(&tab), !extra);
        assert_eq!(root.items().count(), if extra { 4 } else { 3 });
        assert!(!root.contains_only_tab(&session.capture_tab(TabKey::for_item(FIRST)).unwrap()));
    }
}

#[test]
fn composable_tab_close_requires_every_descendant_and_veto_is_atomic() {
    let mut session = composed_session();
    let key = tools(&session).key();
    let prepared = session.prepare_close_tab(tools(&session));
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_close_request(prepared).unwrap();
    let report = commit(frame);
    let plan = requested_plan(&report, submitted);
    assert_eq!(
        plan.target(),
        ClosePlanTarget::Tab {
            root: ROOT,
            tab: key
        }
    );
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [FIRST, SECOND, D]
    );
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    frame
        .resolve_close(plan.request(), plan.items()[1].token(), CloseDecision::Veto)
        .unwrap();
    commit(frame);
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .content()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [THIRD, FIRST, SECOND, D]
    );
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .active_items()
            .collect::<Vec<_>>(),
        [THIRD]
    );
    assert!(session.capture_tab(key).is_some());
    assert_eq!(session.pending_content_close_plans().count(), 0);
}

#[test]
fn composable_tab_close_removes_a_whole_layout_after_all_decisions() {
    let mut session = composed_session();
    let key = tools(&session).key();
    let prepared = session.prepare_close_tab(tools(&session));
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_close_request(prepared).unwrap();
    let report = commit(frame);
    let plan = requested_plan(&report, submitted);
    let mut frame = session.begin_host_frame().unwrap();
    for item in plan.items() {
        frame
            .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
            .unwrap();
    }
    let report = commit(frame);
    assert!(report.events().iter().any(|event| matches!(event.kind(),
        DockspaceEventKind::Closed(DockspaceCloseOutcome::TabClosed { root: ROOT, tab, items })
        if *tab == key && items == &[FIRST, SECOND, D]
    )));
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .content()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [THIRD]
    );
    assert!(session.capture_tab(key).is_none());
    for item in [FIRST, SECOND, D] {
        assert!(session.view().item(item).is_none());
    }
}

#[test]
fn composable_tab_capture_cannot_refresh_itself_after_an_ancestor_selection_changes() {
    let mut session = composed_session();
    let captured = tools(&session);
    let prepared = session.prepare_select_item(D);
    assert!(apply_action(&mut session, prepared));
    let prepared =
        session.prepare_dock_tab(captured, DockPlacement::Center(DockAnchor::Item(FOURTH)));
    assert!(!apply_action(&mut session, prepared));
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .content()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [THIRD, FIRST, SECOND, D]
    );
    assert!(
        session.capture_tab(TabKey::for_item(D)).is_none(),
        "bare content has no implicit direct tab"
    );
}

#[test]
fn composable_group_center_modes_use_the_same_source_with_distinct_direct_children() {
    use crate::model::TabGroupDockMode;
    for mode in [
        TabGroupDockMode::MergeTabs,
        TabGroupDockMode::PreserveLayout,
    ] {
        let mut session = composed_session();
        let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
        let group_key = TabKey::for_layout(group.source.node());
        let action = session.prepare_dock_tabs_with_mode(
            group,
            DockPlacement::Center(DockAnchor::Item(FOURTH)),
            mode,
        );
        assert!(apply_action(&mut session, action));
        let target = session
            .view()
            .root(OTHER_ROOT)
            .unwrap()
            .content()
            .unwrap()
            .tabs()
            .unwrap();
        let expected = match mode {
            TabGroupDockMode::MergeTabs => vec![FOURTH.into(), FIRST.into(), SECOND.into()],
            TabGroupDockMode::PreserveLayout => vec![FOURTH.into(), group_key],
        };
        assert_eq!(target.tabs(), expected);
        assert_eq!(
            session
                .view()
                .root(OTHER_ROOT)
                .unwrap()
                .items()
                .collect::<Vec<_>>(),
            [FOURTH, FIRST, SECOND]
        );
        assert_eq!(
            session
                .view()
                .root(ROOT)
                .unwrap()
                .items()
                .collect::<Vec<_>>(),
            [THIRD, D]
        );
        assert_eq!(
            session
                .view()
                .root(OTHER_ROOT)
                .unwrap()
                .active_items()
                .collect::<Vec<_>>(),
            [FIRST]
        );
    }
}

fn capture_bare_pane_action(session: &mut DockspaceSession, close: bool) -> PreparedSurfaceAction {
    let select = session.prepare_select_item(D);
    assert!(apply_action(session, select));
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface(
            SURFACE,
            UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, 900.0, 600.0).unwrap(),
                LogicalSize::new(40.0, 40.0).unwrap(),
                70.0,
            )
            .unwrap(),
        )
        .unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let pane = plan.panes().find(|pane| pane.item() == D).unwrap();
    let action = if close {
        plan.prepare_pane_close(pane).unwrap()
    } else {
        let command = plan
            .pane_command(pane, DockspacePresentationCommandKind::Float)
            .unwrap()
            .unwrap();
        assert!(command.is_ready());
        plan.prepare_presentation_command(command).unwrap()
    };
    commit(frame);
    action
}

#[test]
fn composable_bare_pane_menu_floats_only_the_real_content() {
    let mut session = composed_session();
    let action = capture_bare_pane_action(&mut session, false);
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_surface_action(action).unwrap();
    let report = commit(frame);
    assert!(report.workspace_changed(), "{report:?}");
    assert!(session.view().item(D).unwrap().contained().is_some());
    assert!(session.view().item(D).unwrap().tabs().is_none());
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [THIRD, FIRST, SECOND]
    );
    assert!(session.view().item(FIRST).unwrap().contained().is_none());
}

#[test]
fn composable_bare_pane_menu_close_keeps_its_own_approval_scope() {
    let mut session = composed_session();
    let action = capture_bare_pane_action(&mut session, true);
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_surface_action(action).unwrap();
    let report = commit(frame);
    let [HostInputOutcome::CloseRequested { plan, .. }] = report.inputs() else {
        panic!("{report:?}");
    };
    assert_eq!(plan.target(), ClosePlanTarget::Item { item: D });
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [D]
    );
    assert!(!report.workspace_changed());
    assert_eq!(
        session
            .view()
            .root(ROOT)
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [THIRD, FIRST, SECOND, D]
    );
}

#[test]
fn composable_bare_pane_menu_cannot_survive_switching_away_from_its_ancestor() {
    for close in [false, true] {
        let mut session = composed_session();
        let action = capture_bare_pane_action(&mut session, close).into_deferred();
        let select = session.prepare_select_item(THIRD);
        assert!(apply_action(&mut session, select));
        let before = session.version();
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(action).unwrap();
        let report = commit(frame);
        assert!(!report.workspace_changed(), "{report:?}");
        assert_eq!(session.version(), before);
        assert_eq!(session.pending_content_close_plans().count(), 0);
        assert!(session.view().item(D).unwrap().contained().is_none());
    }
}

fn reorder_session(policy: DockPolicy) -> DockspaceSession {
    let tools = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([FIRST, SECOND]), DockspaceNode::pane(D)],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::pane(THIRD),
                tools,
                DockspaceNode::pane(FOURTH),
            ]),
        ),
    )])
    .unwrap();
    DockspaceSession::from_layout(layout, policy).unwrap()
}

fn no_undocking_policy() -> DockPolicy {
    let mut policy = DockPolicy::default();
    for item in [FIRST, SECOND, THIRD, FOURTH, D] {
        let mut rule = DockItemRule::new();
        rule.set_allow_undocking(false);
        policy.set_item_rule(item, rule);
    }
    policy
}

#[test]
fn captured_direct_tabs_reorder_both_directions_with_undocking_disabled() {
    let mut session = reorder_session(no_undocking_policy());
    let layout = tools(&session).key();
    let first = TabKey::for_item(THIRD);
    let last = TabKey::for_item(FOURTH);
    let cases = [
        (
            layout,
            DockPlacement::Before(THIRD),
            [layout, first, last],
            true,
        ),
        (
            layout,
            DockPlacement::Before(FOURTH),
            [first, layout, last],
            true,
        ),
        (
            layout,
            DockPlacement::After(FOURTH),
            [first, last, layout],
            true,
        ),
        (
            layout,
            DockPlacement::After(THIRD),
            [first, layout, last],
            true,
        ),
        (
            layout,
            DockPlacement::Center(DockAnchor::Item(THIRD)),
            [first, layout, last],
            false,
        ),
        (
            last,
            DockPlacement::Before(THIRD),
            [last, first, layout],
            true,
        ),
        (
            last,
            DockPlacement::After(THIRD),
            [first, last, layout],
            true,
        ),
        (
            last,
            DockPlacement::Center(DockAnchor::Item(THIRD)),
            [first, last, layout],
            false,
        ),
    ];
    for (key, placement, expected, changed) in cases {
        let before = session.version();
        let tab = session.capture_tab(key).unwrap();
        let action = session.prepare_dock_tab(tab, placement);
        let mut frame = session.begin_host_frame().unwrap();
        let submitted = frame.submit_prepared_action(action).unwrap();
        let report = commit(frame);
        let expected_items = if key == layout {
            vec![FIRST, SECOND, D]
        } else {
            vec![FOURTH]
        };
        assert!(
            matches!(
                report.submitted_action_outcome(submitted),
                Some(HostInputOutcome::ProductActionApplied(DockspaceActionOutcome::TabDocked {
                    tab,
                    source_root: ROOT,
                    target_root: ROOT,
                    items,
                    changed: actual_changed,
                })) if *tab == key && *actual_changed == changed && *items == expected_items
            ),
            "{report:?}"
        );
        assert_eq!(report.workspace_changed(), changed);
        assert_eq!(session.version() == before, !changed);
        let root = session.view().root(ROOT).unwrap();
        let tabs = root.content().unwrap().tabs().unwrap();
        assert_eq!(tabs.tabs(), expected);
        assert_eq!(tabs.selected(), Some(first));
        assert_eq!(
            tabs.tab_label(layout),
            Some(TabLabel::Layout { title: "Tools" })
        );
        assert_eq!(root.active_items().collect::<Vec<_>>(), [THIRD]);
        assert_eq!(root.items().count(), 5);
        assert_eq!(
            root.items().collect::<std::collections::BTreeSet<_>>(),
            [FIRST, SECOND, THIRD, FOURTH, D].into_iter().collect()
        );
        for item in [FIRST, SECOND, THIRD, FOURTH, D] {
            assert!(session.view().item(item).unwrap().contained().is_none());
        }
    }
}

#[test]
fn captured_tab_reorder_denies_crossed_inline_and_layout_descendant_barriers_atomically() {
    for (move_layout, placement, blocked) in [
        (true, DockPlacement::Before(THIRD), THIRD),
        (true, DockPlacement::After(FOURTH), FOURTH),
        (false, DockPlacement::After(FOURTH), SECOND),
        (false, DockPlacement::After(FOURTH), D),
    ] {
        let mut policy = no_undocking_policy();
        let mut rule = DockItemRule::new();
        rule.set_allow_undocking(false);
        rule.set_allow_tab_reorder(false);
        policy.set_item_rule(blocked, rule);
        let mut session = reorder_session(policy);
        let layout = tools(&session).key();
        let key = if move_layout {
            layout
        } else {
            TabKey::for_item(THIRD)
        };
        let tab = session.capture_tab(key).unwrap();
        let action = session.prepare_dock_tab(tab, placement);
        let before = session.version();
        let mut frame = session.begin_host_frame().unwrap();
        let submitted = frame.submit_prepared_action(action).unwrap();
        let report = commit(frame);
        assert_eq!(
            report.submitted_action_outcome(submitted),
            Some(&HostInputOutcome::ProductActionRejected(
                DockspaceActionRejection::PolicyDenied
            ))
        );
        assert!(!report.workspace_changed());
        assert_eq!(session.version(), before);
        let root = session.view().root(ROOT).unwrap();
        let tabs = root.content().unwrap().tabs().unwrap();
        assert_eq!(
            tabs.tabs(),
            [TabKey::for_item(THIRD), layout, TabKey::for_item(FOURTH)]
        );
        assert_eq!(tabs.selected(), Some(TabKey::for_item(THIRD)));
        assert_eq!(
            tabs.tab_label(layout),
            Some(TabLabel::Layout { title: "Tools" })
        );
        assert_eq!(
            root.items().collect::<Vec<_>>(),
            [THIRD, FIRST, SECOND, D, FOURTH]
        );
        assert_eq!(root.active_items().collect::<Vec<_>>(), [THIRD]);
        assert_eq!(session.pending_content_close_plans().count(), 0);
    }
}

#[test]
fn captured_same_parent_tab_reorder_cannot_refresh_stale_source_or_placement() {
    let mut session = reorder_session(no_undocking_policy());
    let captured = tools(&session);
    let layout = captured.key();
    let captured_version = captured.expected_version();
    let frozen = session.prepare_dock_tab(captured.clone(), DockPlacement::Before(THIRD));
    let action = session.prepare_dock_item(FOURTH, DockPlacement::Before(THIRD));
    assert!(apply_action(&mut session, action));
    let before = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_action(frozen).unwrap();
    let report = commit(frame);
    assert!(matches!(
        report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::StaleRejected { .. })
    ));
    assert!(!report.workspace_changed());
    assert_eq!(session.version(), before);

    let action = session.prepare_dock_tab(captured, DockPlacement::After(THIRD));
    assert_eq!(action.expected_version(), captured_version);
    let mut frame = session.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_action(action).unwrap();
    let report = commit(frame);
    assert!(matches!(
        report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::StaleRejected { .. })
    ));
    assert!(!report.workspace_changed());
    assert_eq!(session.version(), before);
    let root = session.view().root(ROOT).unwrap();
    assert_eq!(
        root.content().unwrap().tabs().unwrap().tabs(),
        [TabKey::for_item(FOURTH), TabKey::for_item(THIRD), layout]
    );
    assert_eq!(
        root.items().collect::<Vec<_>>(),
        [FOURTH, THIRD, FIRST, SECOND, D]
    );
    assert_eq!(root.active_items().collect::<Vec<_>>(), [THIRD]);
    assert_eq!(session.pending_content_close_plans().count(), 0);
}
