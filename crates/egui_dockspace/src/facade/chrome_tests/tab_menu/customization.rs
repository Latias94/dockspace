//! Application menu rows share the existing input, focus and publication contracts.

use super::super::presentation_keyboard::batch_events;
use super::*;
use crate::{DockMenuContext, DockMenuItem, DockMenuItemKind, DockMenuTarget};

fn custom_tab_menu(menu: &mut DockMenu) {
    assert_eq!(menu.context().target, DockMenuTarget::Tab(B.into()));
    menu.heading = "Document tools".to_owned();
    menu.accessible_label = menu.heading.clone();
    menu.items
        .retain(|item| item.kind() != DockMenuItemKind::Close);
    if let Some(item) = menu.items.iter_mut().find(|item| {
        item.kind()
            == DockMenuItemKind::Presentation(crate::DockspacePresentationCommandKind::Float)
    }) {
        item.label = "Detach document".to_owned();
    }
    let mut disabled =
        DockMenuItem::custom(egui::Id::unique("disabled-document-tool"), "Disabled tool");
    disabled.enabled = false;
    let id = egui::Id::unique("bookmark-document");
    menu.items
        .insert(0, DockMenuItem::custom(id, "Bookmark document"));
    menu.items
        .insert(1, DockMenuItem::custom(id, "Duplicate ID"));
    menu.items.insert(0, disabled);
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn customized_tab_menu_uses_the_same_pointer_keyboard_and_accesskit_routes() {
    for callback in [false, true] {
        for trigger in [Open::Secondary, Open::ShiftF10, Open::Accessible] {
            let mut harness = Harness::tabs(1);
            if callback {
                harness.local_style = Some((*harness.context.global_style()).clone());
            }
            harness.panes.customize_menu = Some(custom_tab_menu);
            let (output, anchor) = open(&mut harness, "Document 2", trigger);
            node(&output, Role::Menu, "Document tools");
            assert!(nodes(&output).iter().all(|(_, node)| !matches!(
                node.label(),
                Some("Float Tab" | "Close Tab" | "Duplicate ID")
            )));
            menu_entry(&output, "Detach document");
            let disabled = node(&output, Role::MenuItem, "Disabled tool");
            let disabled_node = nodes(&output)
                .iter()
                .find(|(id, _)| *id == disabled)
                .unwrap()
                .1
                .clone();
            assert!(disabled_node.is_disabled());
            assert!(!disabled_node.supports_action(Action::Click));
            harness.run(WIDTH, vec![action(disabled, Action::Click)]);
            assert!(harness.panes.menu_actions.is_empty());
            let output = settle(&mut harness);
            let bookmark = menu_entry(&output, "Bookmark document");
            let input = match trigger {
                Open::Secondary => {
                    let bounds = nodes(&output)
                        .iter()
                        .find(|(id, _)| *id == bookmark)
                        .unwrap()
                        .1
                        .bounds()
                        .unwrap();
                    let pos = egui::pos2(
                        ((bounds.x0 + bounds.x1) / 2.0) as f32,
                        ((bounds.y0 + bounds.y1) / 2.0) as f32,
                    );
                    let button = |pressed| Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    };
                    harness.run(WIDTH, vec![Event::PointerMoved(pos)]);
                    harness.run(WIDTH, vec![button(true)]);
                    vec![button(false)]
                }
                Open::ShiftF10 => vec![key(Key::ArrowDown), key(Key::Enter)],
                Open::Accessible => vec![action(bookmark, Action::Click)],
                Open::MenuKey => unreachable!(),
            };
            let version = harness.dockspace.version();
            batch_events(&mut harness, input, 2);
            settle(&mut harness);
            assert_eq!(
                harness.panes.menu_actions,
                [DockMenuAction {
                    context: DockMenuContext {
                        surface: SURFACE,
                        root: RootId::new(1),
                        target: DockMenuTarget::Tab(B.into())
                    },
                    id: egui::Id::unique("bookmark-document"),
                }],
                "{callback}/{trigger:?}"
            );
            assert_eq!(harness.dockspace.version(), version);
            assert!(harness.dockspace.view().item(A).unwrap().is_active());
            assert!(!egui::Popup::is_any_open(&harness.context));
            assert_eq!(
                harness
                    .context
                    .memory(egui::Memory::focused)
                    .map(|id| id.value()),
                Some(anchor.0)
            );
            harness.run(WIDTH, vec![action(bookmark, Action::Click)]);
            assert_eq!(harness.panes.menu_actions.len(), 1);
        }
    }
}

#[test]
fn escape_preempts_a_custom_menu_activation_in_the_same_packet() {
    for callback in [false, true] {
        let mut harness = Harness::tabs(1);
        if callback {
            harness.local_style = Some((*harness.context.global_style()).clone());
        }
        harness.panes.customize_menu = Some(custom_tab_menu);
        let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
        let bookmark = menu_entry(&output, "Bookmark document");
        harness.run(WIDTH, vec![action(bookmark, Action::Focus)]);
        settle(&mut harness);
        batch_events(
            &mut harness,
            vec![key(Key::Escape), action(bookmark, Action::Click)],
            2,
        );
        assert!(harness.panes.menu_actions.is_empty());
        assert!(!egui::Popup::is_any_open(&harness.context));
    }
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn keyboard_reveals_a_custom_row_below_the_menus_scroll_viewport() {
    let mut harness = Harness::tabs(1);
    harness.panes.customize_menu = Some(|menu| {
        menu.items = (0..40)
            .map(|index| {
                DockMenuItem::custom(
                    egui::Id::unique(("long-menu", index)),
                    format!("Tool {index}"),
                )
            })
            .collect();
    });
    let (output, _) = open(&mut harness, "Document 2", Open::ShiftF10);
    let last = menu_entry(&output, "Tool 39");
    let initial_top = nodes(&output)
        .iter()
        .find(|(id, _)| *id == last)
        .unwrap()
        .1
        .bounds()
        .unwrap()
        .y0;
    assert!(initial_top > 600.0);
    harness.run(WIDTH, vec![key(Key::ArrowUp)]);
    let output = settle(&mut harness);
    let bounds = nodes(&output)
        .iter()
        .find(|(id, _)| *id == last)
        .unwrap()
        .1
        .bounds()
        .unwrap();
    assert!(
        bounds.y0 >= 0.0 && bounds.y1 <= 600.0,
        "keyboard focus must reveal the real row: {bounds:?}"
    );
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    harness.run(WIDTH, vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        harness.run(
            WIDTH,
            vec![Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    assert_eq!(harness.panes.menu_actions.len(), 1);
    assert_eq!(
        harness.panes.menu_actions[0].id,
        egui::Id::unique(("long-menu", 39))
    );
}

#[test]
fn an_owned_run_that_rejects_final_output_does_not_deliver_a_custom_menu_action() {
    struct RejectOutput;

    impl egui::plugin::Plugin for RejectOutput {
        fn debug_name(&self) -> &'static str {
            "reject-the-custom-menu-output"
        }

        fn output_hook(&mut self, _context: &Context, output: &mut FullOutput) {
            output.shapes.clear();
        }
    }

    let mut harness = Harness::tabs(1);
    harness.panes.customize_menu = Some(custom_tab_menu);
    let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
    let bookmark = menu_entry(&output, "Bookmark document");
    harness.context.add_plugin(RejectOutput);
    let result = harness.dockspace.run_single_surface(
        &harness.context,
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            events: vec![action(bookmark, Action::Click)],
            ..Default::default()
        },
        SURFACE,
        &mut harness.panes,
    );
    assert_eq!(
        result
            .err()
            .expect("replaced output must reject publication")
            .kind(),
        crate::DockspaceErrorKind::OperationConflict
    );
    assert!(harness.panes.menu_actions.is_empty());
}
