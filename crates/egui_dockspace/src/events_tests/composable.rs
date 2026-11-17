//! Layout-tab close controls retain a complete, atomic real-content scope.

use super::*;

const OUTSIDE: ItemId = ItemId::new(3);
const BARE: ItemId = ItemId::new(4);

fn composed() -> Harness {
    let mut harness = Harness::new(DockStyle::default());
    let tools = DockspaceNode::equal_split(
        crate::DockspaceAxis::Horizontal,
        [
            DockspaceNode::tabs([FIRST, SECOND]),
            DockspaceNode::pane(BARE),
        ],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([DockspaceNode::pane(OUTSIDE), tools]),
        ),
    )])
    .unwrap();
    harness.dock.reset_layout(layout).unwrap();
    harness.settle();
    harness
}

fn collect_requests(harness: &mut Harness, events: Vec<Event>) -> Vec<crate::DockspaceClosePlan> {
    harness.step(events);
    let mut requests = harness.close_requests.clone();
    for _ in 0..3 {
        harness.step(Vec::new());
        requests.extend(harness.close_requests.clone());
    }
    requests
}

#[test]
fn layout_tab_close_keeps_all_descendants_until_final_allow_and_any_veto_preserves_them() {
    for veto in [false, true] {
        let mut harness = composed();
        let layout_key = harness
            .dock
            .view()
            .surface(SURFACE)
            .unwrap()
            .main_root()
            .unwrap()
            .content()
            .unwrap()
            .tabs()
            .unwrap()
            .tabs()
            .iter()
            .copied()
            .find(|tab| tab.is_layout())
            .unwrap();
        let target = harness
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == Role::Button && node.label() == Some("Close Layout Tab Tools")
            })
            .expect("layout tabs have the default typed close control")
            .0;
        let requests = collect_requests(
            &mut harness,
            vec![Event::AccessKitActionRequest(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: target,
                data: None,
            })],
        );
        assert_eq!(requests.len(), 1);
        let plan = &requests[0];
        assert_eq!(
            plan.target(),
            crate::ClosePlanTarget::Tab {
                root: ROOT,
                tab: layout_key
            }
        );
        assert_eq!(
            plan.items()
                .iter()
                .map(|item| item.item())
                .collect::<Vec<_>>(),
            [FIRST, SECOND, BARE]
        );
        for item in &plan.items()[..2] {
            let recorded = harness
                .dock
                .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
                .unwrap();
            assert!(matches!(
                recorded.outcome(),
                DockspaceCloseOutcome::Processed {
                    resolution: DockspaceCloseResolution::Recorded { item: recorded, .. },
                    application: None,
                    ..
                } if *recorded == item.item()
            ));
            for item in [FIRST, SECOND, BARE, OUTSIDE] {
                assert!(
                    harness.dock.view().item(item).is_some(),
                    "partial approvals keep the whole subtree"
                );
            }
        }
        let resolved = harness
            .dock
            .resolve_close(
                plan.request(),
                plan.items()[2].token(),
                if veto {
                    CloseDecision::Veto
                } else {
                    CloseDecision::Allow
                },
            )
            .unwrap();
        assert!(matches!(
            (resolved.outcome(), veto),
            (
                DockspaceCloseOutcome::Processed {
                    resolution: DockspaceCloseResolution::Vetoed { item: BARE, .. },
                    application: None,
                    ..
                },
                true
            ) | (
                DockspaceCloseOutcome::Processed {
                    resolution: DockspaceCloseResolution::Approved { .. },
                    application: Some(Ok(_)),
                    ..
                },
                false
            )
        ));
        for item in [FIRST, SECOND, BARE] {
            assert_eq!(harness.dock.view().item(item).is_some(), veto);
        }
        assert!(harness.dock.view().item(OUTSIDE).unwrap().is_active());
    }
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn middle_click_on_an_inactive_layout_tab_closes_its_subtree_without_selecting_it() {
    let mut harness = composed();
    let bounds = harness
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Tab && node.label() == Some("Tools"))
        .unwrap()
        .1
        .bounds()
        .unwrap();
    let position = egui::pos2(
        (bounds.x0 + 4.0) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    harness.step(vec![Event::PointerMoved(position)]);
    harness.step(vec![Event::PointerButton {
        pos: position,
        button: PointerButton::Middle,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    let requests = collect_requests(
        &mut harness,
        vec![Event::PointerButton {
            pos: position,
            button: PointerButton::Middle,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(requests.len(), 1);
    assert!(
        matches!(requests[0].target(), crate::ClosePlanTarget::Tab { root: ROOT, tab } if tab.is_layout())
    );
    assert_eq!(
        requests[0]
            .items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [FIRST, SECOND, BARE]
    );
    assert!(harness.dock.view().item(OUTSIDE).unwrap().is_active());
    assert!(!harness.dock.view().item(FIRST).unwrap().is_active());
}

#[test]
fn tab_menu_close_uses_the_same_complete_layout_close_protocol() {
    let mut harness = composed();
    let tab = harness
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Tab && node.label() == Some("Tools"))
        .unwrap()
        .0;
    harness.step(vec![Event::AccessKitActionRequest(ActionRequest {
        action: Action::ShowContextMenu,
        target_tree: TreeId::ROOT,
        target_node: tab,
        data: None,
    })]);
    harness.settle();
    let close = harness
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::MenuItem && node.label() == Some("Close Layout Tab"))
        .expect("the built-in tab menu provides its own close command")
        .0;
    let requests = collect_requests(
        &mut harness,
        vec![Event::AccessKitActionRequest(ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: close,
            data: None,
        })],
    );
    assert_eq!(requests.len(), 1);
    assert!(
        matches!(requests[0].target(), crate::ClosePlanTarget::Tab { root: ROOT, tab } if tab.is_layout())
    );
    assert_eq!(
        requests[0]
            .items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [FIRST, SECOND, BARE]
    );
    assert!(harness.dock.view().item(OUTSIDE).unwrap().is_active());
}
