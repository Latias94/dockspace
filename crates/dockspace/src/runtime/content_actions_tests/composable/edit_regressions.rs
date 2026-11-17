//! Public regressions for edits that cross real layout-tab boundaries.

use super::*;
use crate::model::{DockspaceGridShape, TabGroupDockMode};

#[test]
fn merging_nested_tabs_into_parent_rebases_before_and_after_gaps() {
    let cases = [
        (
            true,
            DockPlacement::Before(FOURTH),
            [THIRD, FIRST, SECOND, FOURTH],
        ),
        (
            true,
            DockPlacement::After(FOURTH),
            [THIRD, FOURTH, FIRST, SECOND],
        ),
        (
            false,
            DockPlacement::Before(THIRD),
            [FIRST, SECOND, THIRD, FOURTH],
        ),
        (
            false,
            DockPlacement::After(THIRD),
            [THIRD, FIRST, SECOND, FOURTH],
        ),
    ];
    for through_grid in [false, true] {
        for (source_first, placement, expected) in cases {
            let tools = DockspaceNode::tabs([FIRST, SECOND]).named("Tools");
            let branch = if through_grid {
                DockspaceNode::grid(DockspaceGridShape::Auto, [tools]).unwrap()
            } else {
                tools
            };
            let children = if source_first {
                vec![
                    branch,
                    DockspaceNode::pane(THIRD),
                    DockspaceNode::pane(FOURTH),
                ]
            } else {
                vec![
                    DockspaceNode::pane(THIRD),
                    DockspaceNode::pane(FOURTH),
                    branch,
                ]
            };
            let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(ROOT, DockspaceNode::composed_tabs(children)),
            )])
            .unwrap();
            let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
            let outer = session.view().root(ROOT).unwrap().content().unwrap().key();
            let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
            let prepared =
                session.prepare_dock_tabs_with_mode(group, placement, TabGroupDockMode::MergeTabs);
            assert!(
                apply_action(&mut session, prepared),
                "through_grid={through_grid}, source_first={source_first}, {placement:?}"
            );
            let root = session.view().root(ROOT).unwrap();
            assert_eq!(root.items().collect::<Vec<_>>(), expected);
            let content = root.content().unwrap();
            assert_eq!(content.key(), outer);
            assert_eq!(
                content.tabs().unwrap().tabs(),
                expected.map(TabKey::for_item)
            );
            session.workspace().validate().unwrap();
        }
    }
}

#[test]
fn merging_the_only_nested_group_retains_its_empty_destination_container() {
    for through_grid in [false, true] {
        let tools = DockspaceNode::tabs([FIRST, SECOND]).named("Tools");
        let branch = if through_grid {
            DockspaceNode::grid(DockspaceGridShape::Auto, [tools]).unwrap()
        } else {
            tools
        };
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::composed_tabs([branch]).named("Outer")),
        )])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        let outer = session.view().root(ROOT).unwrap().content().unwrap().key();
        let group = session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
        let action = session.prepare_dock_tabs_with_mode(
            group,
            DockPlacement::Center(DockAnchor::Node(outer)),
            TabGroupDockMode::MergeTabs,
        );
        assert!(
            apply_action(&mut session, action),
            "through_grid={through_grid}"
        );
        let content = session.view().root(ROOT).unwrap().content().unwrap();
        assert_eq!(content.key(), outer);
        assert_eq!(content.layout_title(), Some("Outer"));
        assert_eq!(
            content.tabs().unwrap().tabs(),
            [FIRST, SECOND].map(TabKey::for_item)
        );
        assert_eq!(
            session
                .view()
                .surface(SURFACE)
                .unwrap()
                .main_root()
                .unwrap()
                .id(),
            ROOT
        );
        session.workspace().validate().unwrap();
    }
}

#[test]
fn closing_a_selected_layout_tab_uses_visible_order_and_preserves_mru() {
    let cases = [
        (false, vec![], FOURTH, [FOURTH, THIRD, D]),
        (false, vec![FOURTH], D, [D, THIRD, FOURTH]),
        (false, vec![FOURTH, D], THIRD, [THIRD, FOURTH, D]),
        (true, vec![], D, [D, THIRD, FOURTH]),
        (true, vec![D], FOURTH, [FOURTH, THIRD, D]),
    ];
    for (closing_last, hidden, successor, expected_mru) in cases {
        let tools = DockspaceNode::tabs([FIRST, SECOND]).named("Tools");
        let children = if closing_last {
            [
                DockspaceNode::pane(THIRD),
                DockspaceNode::pane(FOURTH),
                DockspaceNode::pane(D),
                tools,
            ]
        } else {
            [
                DockspaceNode::pane(THIRD),
                tools,
                DockspaceNode::pane(FOURTH),
                DockspaceNode::pane(D),
            ]
        };
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::composed_tabs(children)),
        )])
        .unwrap()
        .with_hidden_items(hidden.iter().copied())
        .unwrap();
        let mut policy = DockPolicy::default();
        policy.set_close_capability(CloseCapability::DeferredAllowed);
        let mut session = DockspaceSession::from_layout(layout, policy).unwrap();
        let select = session.prepare_select_item(FIRST);
        assert!(apply_action(&mut session, select));
        let outer = session.view().root(ROOT).unwrap().content().unwrap();
        let outer_key = outer.key();
        let tab_key = outer.tabs().unwrap().tabs()[if closing_last { 3 } else { 1 }];
        let prepared = session.prepare_close_tab(session.capture_tab(tab_key).unwrap());
        let mut frame = session.begin_host_frame().unwrap();
        let request = frame.submit_prepared_close_request(prepared).unwrap();
        let plan = requested_plan(&commit(frame), request);
        let mut frame = session.begin_host_frame().unwrap();
        for item in plan.items() {
            frame
                .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
                .unwrap();
        }
        let report = commit(frame);
        assert!(report.events().iter().any(|event| matches!(
            event.kind(),
            DockspaceEventKind::Closed(DockspaceCloseOutcome::TabClosed { tab, items, .. })
                if *tab == tab_key && items == &[FIRST, SECOND]
        )));
        let root = session.view().root(ROOT).unwrap();
        assert_eq!(root.items().collect::<Vec<_>>(), [THIRD, FOURTH, D]);
        assert_eq!(root.active_items().collect::<Vec<_>>(), [successor]);
        assert_eq!(
            session.workspace().tab_mru(outer_key.0).unwrap(),
            expected_mru.map(TabKey::for_item)
        );
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().item(),
            Some(successor)
        );
        assert_eq!(session.view().hidden_items().collect::<Vec<_>>(), hidden);
    }
}

#[test]
fn opening_into_an_inactive_layout_reveals_content_before_requesting_focus() {
    for bare_target in [false, true] {
        let tools = if bare_target {
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [DockspaceNode::pane(FIRST), DockspaceNode::pane(FOURTH)],
            )
            .unwrap()
            .named("Tools")
        } else {
            DockspaceNode::tabs([FIRST]).named("Tools")
        };
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::composed_tabs([DockspaceNode::pane(THIRD), tools]),
            ),
        )])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        assert_eq!(
            session
                .view()
                .root(ROOT)
                .unwrap()
                .active_items()
                .collect::<Vec<_>>(),
            [THIRD]
        );
        let open =
            session.prepare_open_item(SECOND, DockPlacement::Center(DockAnchor::Item(FIRST)));
        assert!(apply_action(&mut session, open));
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().item(),
            Some(SECOND)
        );
        assert!(session.view().item(SECOND).unwrap().is_active());
        let expected = if bare_target {
            vec![SECOND, FOURTH]
        } else {
            vec![SECOND]
        };
        assert_eq!(
            session
                .view()
                .root(ROOT)
                .unwrap()
                .active_items()
                .collect::<Vec<_>>(),
            expected
        );
        assert!(!session.view().item(THIRD).unwrap().is_active());
    }
}

#[test]
fn docking_into_an_inactive_layout_reveals_the_destination_before_requesting_focus() {
    for bare_target in [false, true] {
        let target = if bare_target { D } else { FIRST };
        let mut placements = vec![
            DockPlacement::Center(DockAnchor::Item(target)),
            DockPlacement::InnerEdge {
                anchor: DockAnchor::Item(target),
                edge: crate::model::DockEdge::Right,
                fraction: crate::model::DockFraction::new(0.5).unwrap(),
            },
        ];
        if !bare_target {
            placements.extend([DockPlacement::Before(target), DockPlacement::After(target)]);
        }
        for placement in placements {
            for payload in ["item", "tab", "merge", "preserve", "root"] {
                let mut session = composed_session();
                let before = session.workspace().item_multiset();
                let prepared = match payload {
                    "item" => session.prepare_dock_item(FOURTH, placement),
                    "tab" => session
                        .prepare_dock_tab(session.capture_tab(FOURTH.into()).unwrap(), placement),
                    "merge" | "preserve" => session.prepare_dock_tabs_with_mode(
                        session
                            .capture_tabs_group(DockAnchor::Item(FOURTH))
                            .unwrap(),
                        placement,
                        if payload == "merge" {
                            TabGroupDockMode::MergeTabs
                        } else {
                            TabGroupDockMode::PreserveLayout
                        },
                    ),
                    "root" => session.prepare_dock_root(OTHER_ROOT, placement),
                    _ => unreachable!(),
                };
                assert!(apply_action(&mut session, prepared));
                let focus = session.pane_focus_request(SURFACE).unwrap();
                assert_eq!(focus.item(), Some(FOURTH));
                assert!(
                    session.view().item(FOURTH).unwrap().is_active(),
                    "a successful dock must reveal focused content: bare={bare_target}, {payload}, {placement:?}"
                );
                assert!(!session.view().item(THIRD).unwrap().is_active());
                assert_eq!(session.view().item(FOURTH).unwrap().root(), ROOT);
                assert_eq!(session.workspace().item_multiset(), before);
                session.workspace().validate().unwrap();
            }
        }
    }
}

#[test]
fn reordering_an_inactive_layout_tab_preserves_the_active_panes_focus() {
    let mut session = composed_session();
    let prepared = session.prepare_select_item(THIRD);
    assert!(apply_action(&mut session, prepared));
    let focus = session.pane_focus_request(SURFACE).unwrap();
    assert_eq!(focus.item(), Some(THIRD));
    let prepared = session.prepare_dock_tab(tools(&session), DockPlacement::Before(THIRD));
    assert!(apply_action(&mut session, prepared));
    assert!(session.view().item(THIRD).unwrap().is_active());
    assert!(!session.view().item(FIRST).unwrap().is_active());
    assert_eq!(session.pane_focus_request(SURFACE), Some(focus));
}

#[test]
fn docking_hidden_content_into_an_inactive_layout_preserves_selection_and_focus() {
    for group in [false, true] {
        let mut session = composed_session();
        let selected = session.prepare_select_item(THIRD);
        assert!(apply_action(&mut session, selected));
        let hidden = session.prepare_set_item_visibility(FOURTH, false);
        assert!(apply_action(&mut session, hidden));
        let focus = session.pane_focus_request(SURFACE).unwrap();
        let placement = DockPlacement::Center(DockAnchor::Item(FIRST));
        let prepared = if group {
            session.prepare_dock_tabs(
                session
                    .capture_tabs_group(DockAnchor::Item(FOURTH))
                    .unwrap(),
                placement,
            )
        } else {
            session.prepare_dock_item(FOURTH, placement)
        };
        assert!(apply_action(&mut session, prepared));
        assert!(session.view().item(THIRD).unwrap().is_active());
        assert!(!session.view().item(FIRST).unwrap().is_active());
        assert!(session.view().item(FOURTH).unwrap().is_hidden());
        assert_eq!(session.view().item(FOURTH).unwrap().root(), ROOT);
        assert_eq!(session.pane_focus_request(SURFACE), Some(focus));
        session.workspace().validate().unwrap();
    }
}
