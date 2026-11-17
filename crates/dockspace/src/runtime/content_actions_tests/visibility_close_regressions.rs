//! Public regressions for visibility and close effects across layout ancestors.

use super::*;
use crate::model::{DockspaceActionOutcome, DockspaceGridShape, DockspaceTabView};

fn singleton_layout(bare: bool) -> DockspaceNode {
    if bare {
        DockspaceNode::grid(DockspaceGridShape::Auto, [DockspaceNode::pane(FIRST)])
            .unwrap()
            .named("Tools")
    } else {
        DockspaceNode::tabs([FIRST]).named("Tools")
    }
}

#[test]
fn hiding_the_last_focused_layout_descendant_follows_the_surviving_ancestor_selection() {
    for bare in [false, true] {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::composed_tabs([DockspaceNode::pane(THIRD), singleton_layout(bare)]),
            ),
        )])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        let select = session.prepare_select_item(FIRST);
        assert!(apply_action(&mut session, select));
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().item(),
            Some(FIRST)
        );
        let content = session.view().root(ROOT).unwrap().content().unwrap();
        let outer = content.key();
        let tools = content.tabs().unwrap().tabs()[1];

        let mut frame = session.begin_host_frame().unwrap();
        let hidden = frame.set_item_visibility_current(FIRST, false).unwrap();
        let report = commit(frame);
        assert!(matches!(
            report.submitted_action_outcome(hidden),
            Some(HostInputOutcome::ProductActionApplied(
                DockspaceActionOutcome::ItemVisibilityChanged {
                    item: FIRST,
                    visible: false,
                    changed: true,
                    ..
                }
            ))
        ));
        let root = session.view().root(ROOT).unwrap();
        assert_eq!(root.items().collect::<Vec<_>>(), [THIRD, FIRST]);
        assert_eq!(root.active_items().collect::<Vec<_>>(), [THIRD]);
        assert_eq!(root.content().unwrap().key(), outer);
        assert_eq!(root.content().unwrap().tabs().unwrap().tabs()[1], tools);
        assert!(session.view().item_is_hidden(FIRST));
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().target(),
            DockspaceFocusTarget::Pane(THIRD),
            "bare={bare}: focus must follow the visible ancestor selection"
        );

        let successor = session.pane_focus_request(SURFACE).unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        frame.set_item_visibility_current(FIRST, true).unwrap();
        commit(frame);
        assert!(!session.view().item_is_hidden(FIRST));
        assert!(!session.view().item(FIRST).unwrap().is_active());
        assert_eq!(session.pane_focus_request(SURFACE), Some(successor));
        assert_eq!(
            session
                .view()
                .root(ROOT)
                .unwrap()
                .active_items()
                .collect::<Vec<_>>(),
            [THIRD]
        );
        session.workspace().validate().unwrap();
    }
}

#[test]
fn closing_the_last_layout_descendant_selects_the_next_visual_ancestor_sibling() {
    for bare in [false, true] {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::composed_tabs([
                    DockspaceNode::pane(THIRD),
                    singleton_layout(bare),
                    DockspaceNode::pane(FOURTH),
                ]),
            ),
        )])
        .unwrap();
        let mut policy = DockPolicy::default();
        policy.set_close_capability(CloseCapability::DeferredAllowed);
        let mut session = DockspaceSession::from_layout(layout, policy).unwrap();
        let select = session.prepare_select_item(FIRST);
        assert!(apply_action(&mut session, select));
        let outer = session.view().root(ROOT).unwrap().content().unwrap().key();
        let mut frame = session.begin_host_frame().unwrap();
        let requested = frame.request_close_item_current(FIRST).unwrap();
        let plan = requested_plan(&commit(frame), requested);
        assert_eq!(plan.target(), ClosePlanTarget::Item { item: FIRST });
        let mut frame = session.begin_host_frame().unwrap();
        let allowed = frame
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        let report = commit(frame);
        assert!(matches!(
            report.submitted_action_outcome(allowed),
            Some(HostInputOutcome::CloseDecisionProcessed {
                application: Some(Ok(DockspaceCloseOutcome::ItemClosed {
                    item: FIRST,
                    root: ROOT,
                })),
                changed: true,
                ..
            })
        ));
        assert!(session.view().item(FIRST).is_none());
        let root = session.view().root(ROOT).unwrap();
        assert_eq!(root.content().unwrap().key(), outer);
        assert_eq!(root.items().collect::<Vec<_>>(), [THIRD, FOURTH]);
        assert_eq!(
            root.active_items().collect::<Vec<_>>(),
            [FOURTH],
            "bare={bare}: pruning the emptied layout must preserve its visual gap"
        );
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().item(),
            Some(FOURTH)
        );
        assert_eq!(session.pending_content_close_plans().count(), 0);
        session.workspace().validate().unwrap();
    }
}

#[test]
fn item_tab_and_tabs_close_preserve_visual_successors_through_multiple_empty_ancestors() {
    const LAST: ItemId = ItemId::new(5);

    for scope in ["item", "tab", "tabs"] {
        for hide_next in [false, true] {
            let items = if scope == "item" {
                vec![FIRST]
            } else {
                vec![FIRST, SECOND]
            };
            let leaf = DockspaceNode::tabs(items.iter().copied()).named("Leaf");
            let middle = DockspaceNode::composed_tabs([leaf]).named("Middle");
            let tools = DockspaceNode::composed_tabs([middle]).named("Tools");
            let hidden = if hide_next { vec![FOURTH] } else { vec![] };
            let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(
                    ROOT,
                    DockspaceNode::composed_tabs([
                        DockspaceNode::pane(THIRD),
                        tools,
                        DockspaceNode::pane(FOURTH),
                        DockspaceNode::pane(LAST),
                    ]),
                ),
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
            let prepared = match scope {
                "item" => session.prepare_close_item(FIRST),
                "tabs" => session.prepare_close_tabs(
                    session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap(),
                ),
                "tab" => {
                    let DockspaceTabView::Layout(tools) =
                        outer.tabs().unwrap().children().nth(1).unwrap()
                    else {
                        panic!("the second outer tab owns the nested layout")
                    };
                    let DockspaceTabView::Layout(middle) =
                        tools.tabs().unwrap().children().next().unwrap()
                    else {
                        panic!("Tools owns the complete Middle layout")
                    };
                    let leaf = middle.tabs().unwrap().tabs()[0];
                    session.prepare_close_tab(session.capture_tab(leaf).unwrap())
                }
                _ => unreachable!(),
            };
            let mut frame = session.begin_host_frame().unwrap();
            let requested = frame.submit_prepared_close_request(prepared).unwrap();
            let plan = requested_plan(&commit(frame), requested);
            assert_eq!(
                plan.items()
                    .iter()
                    .map(|item| item.item())
                    .collect::<Vec<_>>(),
                items
            );
            let mut frame = session.begin_host_frame().unwrap();
            for item in plan.items() {
                frame
                    .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
                    .unwrap();
            }
            let report = commit(frame);
            assert!(report.workspace_changed());
            let root = session.view().root(ROOT).unwrap();
            assert_eq!(root.content().unwrap().key(), outer_key);
            assert_eq!(root.items().collect::<Vec<_>>(), [THIRD, FOURTH, LAST]);
            assert_eq!(
                root.content().unwrap().tabs().unwrap().tabs(),
                [THIRD, FOURTH, LAST].map(TabKey::for_item)
            );
            let successor = if hide_next { LAST } else { FOURTH };
            assert_eq!(
                root.active_items().collect::<Vec<_>>(),
                [successor],
                "scope={scope}, hide_next={hide_next}"
            );
            let expected_mru = if hide_next {
                [LAST, THIRD, FOURTH]
            } else {
                [FOURTH, THIRD, LAST]
            };
            assert_eq!(
                session.workspace().tab_mru(outer_key.0).unwrap(),
                expected_mru.map(TabKey::for_item)
            );
            assert_eq!(session.view().hidden_items().collect::<Vec<_>>(), hidden);
            assert_eq!(
                session.pane_focus_request(SURFACE).unwrap().item(),
                Some(successor)
            );
            assert_eq!(session.pending_content_close_plans().count(), 0);
            session.workspace().validate().unwrap();
        }
    }
}

#[test]
fn closing_the_last_central_descendant_keeps_its_visible_layout_path_and_protection() {
    for close_group in [false, true] {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::composed_tabs([
                    DockspaceNode::pane(THIRD),
                    DockspaceNode::composed_tabs([DockspaceNode::central_tabs([FIRST])])
                        .named("Tools"),
                    DockspaceNode::pane(FOURTH),
                ]),
            ),
        )])
        .unwrap()
        .with_hidden_items([FOURTH])
        .unwrap();
        let mut policy = DockPolicy::default();
        policy.set_close_capability(CloseCapability::DeferredAllowed);
        let mut session = DockspaceSession::from_layout(layout, policy).unwrap();
        let select = session.prepare_select_item(FIRST);
        assert!(apply_action(&mut session, select));
        let root = session.view().root(ROOT).unwrap();
        let central = root.central().unwrap().key();
        let tools = root.content().unwrap().tabs().unwrap().tabs()[1];
        let before = session.workspace().clone();
        let protected = session.prepare_close_tab(session.capture_tab(tools).unwrap());
        let mut frame = session.begin_host_frame().unwrap();
        let denied = frame.submit_prepared_close_request(protected).unwrap();
        let report = commit(frame);
        assert!(matches!(
            report.submitted_action_outcome(denied),
            Some(HostInputOutcome::CloseRejected(_))
        ));
        assert_eq!(session.workspace(), &before);

        let prepared = if close_group {
            session.prepare_close_tabs(session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap())
        } else {
            session.prepare_close_item(FIRST)
        };
        let mut frame = session.begin_host_frame().unwrap();
        let requested = frame.submit_prepared_close_request(prepared).unwrap();
        let plan = requested_plan(&commit(frame), requested);
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        let report = commit(frame);
        assert!(report.workspace_changed());
        let root = session.view().root(ROOT).unwrap();
        assert_eq!(root.central().unwrap().key(), central);
        assert_eq!(root.items().collect::<Vec<_>>(), [THIRD, FOURTH]);
        assert!(root.active_items().next().is_none());
        assert_eq!(
            root.content().unwrap().tabs().unwrap().selected(),
            Some(tools)
        );
        assert!(root.central().unwrap().tabs().unwrap().tabs().is_empty());
        assert_eq!(session.view().hidden_items().collect::<Vec<_>>(), [FOURTH]);
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().target(),
            DockspaceFocusTarget::Workspace
        );
        assert!(session.capture_tab(tools).is_some());
        session.workspace().validate().unwrap();
    }
}
