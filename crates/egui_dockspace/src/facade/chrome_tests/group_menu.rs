//! Real group menus retain scope through pointer, keyboard and accessibility input.

use super::*;

const WIDTH: f32 = 900.0;

fn setup() -> Harness {
    let mut harness = Harness::new(
        ["Editor", "Preview", "Inspector", "Console"]
            .map(str::to_owned)
            .to_vec(),
        ItemId::new(1),
        DockStyle::default(),
    );
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [
                    DockspaceNode::central_tabs([ItemId::new(1), ItemId::new(2)]),
                    DockspaceNode::tabs([ItemId::new(3), ItemId::new(4)]),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("group-menu-regression", layout)
        .build()
        .unwrap();
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness) -> FullOutput {
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new())
}

fn action(target: NodeId, action: egui::accesskit::Action) -> Event {
    Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: target,
        data: None,
    })
}

fn open_first_group(harness: &mut Harness) -> FullOutput {
    let output = settle(harness);
    let groups = nodes(&output)
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Button && node.label() == Some("Tab group actions")
        })
        .collect::<Vec<_>>();
    assert_eq!(groups.len(), 2);
    let anchor = groups
        .into_iter()
        .min_by(|(_, a), (_, b)| a.bounds().unwrap().x0.total_cmp(&b.bounds().unwrap().x0))
        .unwrap()
        .0;
    harness.run(WIDTH, vec![action(anchor, egui::accesskit::Action::Click)]);
    settle(harness)
}

fn entry(output: &FullOutput, label: &str) -> NodeId {
    nodes(output)
        .iter()
        .find(|(_, node)| {
            node.role() == Role::MenuItem && node.label() == Some(label) && !node.is_disabled()
        })
        .unwrap_or_else(|| panic!("missing enabled {label}"))
        .0
}

#[test]
fn group_menu_float_moves_only_that_group() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem
                && node.label() == Some("Move Tab Group to New Window"))
    );
    let target = entry(&output, "Float Tab Group");
    harness.run(WIDTH, vec![action(target, egui::accesskit::Action::Click)]);
    settle(&mut harness);
    let view = harness.dockspace.view();
    let first = view.item(ItemId::new(1)).unwrap();
    assert!(first.contained().is_some());
    assert_eq!(
        first.tabs().items().collect::<Vec<_>>(),
        &[ItemId::new(1), ItemId::new(2)]
    );
    assert!(view.item(ItemId::new(3)).unwrap().contained().is_none());
    assert!(view.item(ItemId::new(4)).unwrap().contained().is_none());
}

#[test]
fn layout_change_dismisses_group_menu_and_rejects_its_old_widget_action() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    let stale_entry = entry(&output, "Float Tab Group");
    assert!(matches!(
        harness
            .dockspace
            .select_item_current(ItemId::new(2))
            .unwrap()
            .status(),
        crate::DockspaceActionStatus::Applied(_)
    ));
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
    harness.run(
        WIDTH,
        vec![action(stale_entry, egui::accesskit::Action::Click)],
    );
    settle(&mut harness);
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .contained()
            .is_none()
    );
}

#[test]
fn a_fresh_session_dismisses_a_popup_with_the_same_widget_and_workspace_version() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    let stale_entry = entry(&output, "Float Tab Group");
    let replacement = setup().dockspace;
    assert_eq!(harness.dockspace.id(), replacement.id());
    assert_eq!(harness.dockspace.version(), replacement.version());
    harness.dockspace = replacement;
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
    harness.run(
        WIDTH,
        vec![action(stale_entry, egui::accesskit::Action::Click)],
    );
    settle(&mut harness);
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .contained()
            .is_none()
    );
}

#[test]
fn keyboard_can_activate_atomic_group_close() {
    let mut harness = setup();
    open_first_group(&mut harness);
    harness.run(WIDTH, vec![key(Key::ArrowUp), key(Key::Enter)]);
    settle(&mut harness);
    let requests = harness
        .dockspace
        .session
        .pending_content_close_plans()
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 1);
    let plan = &requests[0];
    assert_eq!(
        plan.items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [ItemId::new(1), ItemId::new(2)]
    );
    assert!(harness.dockspace.view().item(ItemId::new(3)).is_some());
}

#[test]
fn action_menu_entrances_paint_an_immediate_outline_for_keyboard_focus() {
    let color = egui::Color32::from_rgb(29, 213, 117);
    for label in ["Tab group actions", "Layout actions"] {
        let mut harness = setup();
        let mut style = harness.dockspace.style().clone();
        style.visuals.tab_focus_stroke = Some(egui::Stroke::new(2.0, color));
        harness.dockspace.set_style(style).unwrap();
        harness.focus(WIDTH, Role::Button, label);
        let output = harness.run(WIDTH, Vec::new());
        let focused = harness.context.memory(egui::Memory::focused).unwrap();
        let bounds = nodes(&output)
            .iter()
            .find(|(id, node)| id.0 == focused.value() && node.label() == Some(label))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let bounds = Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        );
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.stroke.color == color
                    && rect.stroke.width == 2.0 && rect.rect.is_positive()
                    && bounds.contains_rect(rect.rect)
            )),
            "{label} must distinguish keyboard focus inside its allocated target"
        );
        assert!(
            !nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::MenuItem)
        );
    }
}
