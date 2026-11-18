//! Typed tab menus share commands across pointer, keyboard and accessibility input.

use super::*;
use egui::accesskit::{Action, ActionRequest, TreeId};

mod customization;

const WIDTH: f32 = 900.0;
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);

fn settle(harness: &mut Harness) -> FullOutput {
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new())
}

fn action(target: NodeId, action: Action) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    })
}

fn node(output: &FullOutput, role: Role, label: &str) -> NodeId {
    nodes(output)
        .iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .unwrap_or_else(|| panic!("missing accessible {role:?}: {label}"))
        .0
}

fn menu_entry(output: &FullOutput, label: &str) -> NodeId {
    let id = node(output, Role::MenuItem, label);
    assert!(
        !nodes(output)
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .unwrap()
            .1
            .is_disabled()
    );
    id
}

#[derive(Clone, Copy, Debug)]
enum Open {
    Secondary,
    MenuKey,
    ShiftF10,
    Accessible,
}

#[allow(clippy::cast_possible_truncation)]
fn open(harness: &mut Harness, label: &str, trigger: Open) -> (FullOutput, NodeId) {
    let output = settle(harness);
    let target = node(&output, Role::Tab, label);
    match trigger {
        Open::Secondary => {
            let bounds = nodes(&output)
                .iter()
                .find(|(id, _)| *id == target)
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let position = egui::pos2(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            );
            harness.run(WIDTH, vec![Event::PointerMoved(position)]);
            for pressed in [true, false] {
                harness.run(
                    WIDTH,
                    vec![Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        }
        Open::Accessible => {
            harness.run(WIDTH, vec![action(target, Action::ShowContextMenu)]);
        }
        Open::MenuKey | Open::ShiftF10 => {
            harness.focus_tab(WIDTH, label);
            let event = match trigger {
                Open::MenuKey => key(Key::Menu),
                _ => Event::Key {
                    key: Key::F10,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
            };
            harness.run(WIDTH, vec![event]);
        }
    }
    (settle(harness), target)
}

fn composed() -> Harness {
    let mut harness = Harness::new(
        (1..=4).map(|item| format!("Document {item}")).collect(),
        C,
        DockStyle::default(),
    );
    let tools = DockspaceNode::equal_split(
        crate::DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([A, B]), DockspaceNode::pane(D)],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::composed_tabs([DockspaceNode::pane(C), tools]),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("typed-layout-tab-menu", layout)
        .build()
        .unwrap();
    harness
        .context
        .all_styles_mut(|style| style.animation_time = 0.0);
    settle(&mut harness);
    harness
}

#[test]
fn secondary_menu_key_shift_f10_and_accesskit_open_the_same_typed_tab_menu() {
    for trigger in [
        Open::Secondary,
        Open::MenuKey,
        Open::ShiftF10,
        Open::Accessible,
    ] {
        let mut harness = Harness::tabs(1);
        let (output, target) = open(&mut harness, "Document 2", trigger);
        assert!(
            nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::Menu && node.label() == Some("Tab actions")),
            "{trigger:?}"
        );
        menu_entry(&output, "Float Tab");
        menu_entry(&output, "Close Tab");
        assert!(
            !nodes(&output).iter().any(
                |(_, node)| node.role() == Role::Menu && node.label() == Some("Layout actions")
            )
        );
        harness.run(WIDTH, vec![key(Key::ArrowDown)]);
        harness.run(WIDTH, vec![key(Key::Escape)]);
        let output = settle(&mut harness);
        assert!(
            !nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::Menu)
        );
        assert_eq!(
            harness
                .context
                .memory(egui::Memory::focused)
                .map(|id| id.value()),
            Some(target.0)
        );
    }
}

#[test]
fn inactive_tab_menu_float_moves_only_its_exact_pane() {
    let mut harness = Harness::tabs(1);
    let (output, _) = open(&mut harness, "Document 2", Open::Secondary);
    assert!(harness.dockspace.view().item(A).unwrap().is_active());
    assert!(!harness.dockspace.view().item(B).unwrap().is_active());
    let float = menu_entry(&output, "Float Tab");
    harness.run(WIDTH, vec![action(float, Action::Click)]);
    settle(&mut harness);
    let view = harness.dockspace.view();
    assert!(view.item(B).unwrap().contained().is_some());
    assert!(view.item(A).unwrap().contained().is_none());
    assert_ne!(view.item(A).unwrap().root(), view.item(B).unwrap().root());
    assert_eq!(
        view.item(A)
            .unwrap()
            .tabs()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [A, C, D, ItemId::new(5), ItemId::new(6)]
    );
}

#[test]
fn layout_tab_menu_names_and_floats_the_complete_subtree() {
    let mut harness = composed();
    let (output, _) = open(&mut harness, "Tools", Open::Accessible);
    assert!(
        nodes(&output).iter().any(
            |(_, node)| node.role() == Role::Menu && node.label() == Some("Layout tab actions")
        )
    );
    assert!(harness.dockspace.view().item(C).unwrap().is_active());
    let float = menu_entry(&output, "Float Layout Tab");
    menu_entry(&output, "Close Layout Tab");
    harness.run(WIDTH, vec![action(float, Action::Click)]);
    let output = settle(&mut harness);
    node(&output, Role::TitleBar, "Tools");
    let view = harness.dockspace.view();
    let floating = view.item(A).unwrap().contained().unwrap();
    for item in [A, B, D] {
        assert_eq!(view.item(item).unwrap().contained(), Some(floating));
    }
    assert!(view.item(C).unwrap().contained().is_none());
    assert!(
        view.item(D).unwrap().tabs().is_none(),
        "floating never adds a fake header to bare content"
    );
}

#[test]
fn switching_an_ancestor_closes_the_inner_tab_menu_and_rejects_its_old_control() {
    let mut harness = composed();
    let activated = harness.dockspace.select_item_current(A).unwrap();
    assert!(matches!(
        activated.status(),
        crate::DockspaceActionStatus::Applied(crate::DockspaceActionOutcome::ItemActivated {
            item: A,
            changed: true,
        })
    ));
    let (output, _) = open(&mut harness, "Document 1", Open::Accessible);
    let old_float = menu_entry(&output, "Float Tab");
    let activated = harness.dockspace.select_item_current(C).unwrap();
    assert!(matches!(
        activated.status(),
        crate::DockspaceActionStatus::Applied(crate::DockspaceActionOutcome::ItemActivated {
            item: C,
            changed: true,
        })
    ));
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Menu)
    );
    harness.run(WIDTH, vec![action(old_float, Action::Click)]);
    settle(&mut harness);
    for item in [A, B, C, D] {
        assert!(
            harness
                .dockspace
                .view()
                .item(item)
                .unwrap()
                .contained()
                .is_none()
        );
    }
    assert!(harness.dockspace.view().item(C).unwrap().is_active());
}

fn overlay_frame(
    harness: &mut Harness,
    events: Vec<Event>,
    overlay: usize,
) -> (FullOutput, Vec<Event>) {
    harness.frame += 1;
    let mut remaining = Vec::new();
    let output = crate::run_ui(
        &harness.context,
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            time: Some(f64::from(harness.frame) * 0.1),
            events,
            ..Default::default()
        },
        |ui| {
            let id = egui::Id::unique(("tab-menu-application-overlay", overlay));
            if overlay == 0 {
                egui::Modal::new(id).show(ui.ctx(), |ui| {
                    ui.label("Application modal");
                });
            } else {
                let popup = egui::Popup::new(
                    id,
                    ui.ctx().clone(),
                    Rect::from_min_size(Pos2::new(450.0, 100.0), vec2(160.0, 60.0)),
                    ui.layer_id(),
                );
                let popup = if overlay == 1 {
                    egui::Popup::open_id(ui.ctx(), id);
                    popup.open_memory(None)
                } else {
                    popup.open(true)
                };
                popup.show(|ui| {
                    ui.label("Application popup");
                });
            }
            harness
                .dockspace
                .show_single_surface(SURFACE, ui, &mut harness.panes)
                .unwrap();
            remaining = ui.input(|input| input.events.clone());
        },
    );
    let (mut output, receipt) = output.into_parts();
    receipt.report(SurfacePresentationResult::Presented);
    output.textures_delta.clear();
    (output, remaining)
}

#[test]
fn modal_and_both_foreign_popup_modes_block_tab_menu_opening_and_old_commands() {
    for overlay in 0..3 {
        let mut harness = Harness::tabs(1);
        harness.focus_tab(WIDTH, "Document 2");
        let output = settle(&mut harness);
        let tab = node(&output, Role::Tab, "Document 2");
        overlay_frame(&mut harness, Vec::new(), overlay);
        let (output, remaining) = overlay_frame(
            &mut harness,
            vec![action(tab, Action::ShowContextMenu), key(Key::Menu)],
            overlay,
        );
        assert!(
            !nodes(&output).iter().any(|(_, node)| {
                node.role() == Role::Menu && node.label() == Some("Tab actions")
            })
        );
        assert!(
            remaining.contains(&key(Key::Menu)),
            "overlay {overlay} retains the context-menu key"
        );
        if overlay == 1 {
            assert!(egui::Popup::is_id_open(
                &harness.context,
                egui::Id::unique(("tab-menu-application-overlay", overlay))
            ));
        }

        let mut harness = Harness::tabs(1);
        let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
        let old_float = menu_entry(&output, "Float Tab");
        let version = harness.dockspace.version();
        overlay_frame(&mut harness, Vec::new(), overlay);
        let (output, _) = overlay_frame(
            &mut harness,
            vec![action(old_float, Action::Click)],
            overlay,
        );
        assert!(
            !nodes(&output).iter().any(|(_, node)| {
                node.role() == Role::Menu && node.label() == Some("Tab actions")
            })
        );
        assert_eq!(harness.dockspace.version(), version);
        assert!(
            harness
                .dockspace
                .view()
                .item(B)
                .unwrap()
                .contained()
                .is_none()
        );
    }
}

#[test]
fn escape_does_not_take_the_application_focus_after_a_tab_menu_is_open() {
    let mut harness = Harness::tabs(1);
    open(&mut harness, "Document 2", Open::Accessible);
    let mut application = None;
    let mut remaining = Vec::new();
    let output = crate::run_ui(
        &harness.context,
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            events: vec![key(Key::Escape)],
            ..Default::default()
        },
        |ui| {
            let response = ui.button("Application focus owner");
            response.request_focus();
            application = Some(response.id);
            harness
                .dockspace
                .show_single_surface(SURFACE, ui, &mut harness.panes)
                .unwrap();
            remaining = ui.input(|input| input.events.clone());
        },
    );
    let (mut output, receipt) = output.into_parts();
    receipt.report(SurfacePresentationResult::Presented);
    output.textures_delta.clear();
    assert!(remaining.contains(&key(Key::Escape)));
    assert_eq!(harness.context.memory(egui::Memory::focused), application);
    assert!(harness.dockspace.view().item(A).unwrap().is_active());
}
