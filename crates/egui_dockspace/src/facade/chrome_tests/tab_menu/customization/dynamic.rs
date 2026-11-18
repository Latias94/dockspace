//! Menu edits preserve logical row identity and release obsolete input ownership.

use super::*;

fn tool_id(index: u8) -> egui::Id {
    egui::Id::unique(("dynamic-menu-tool", index))
}

fn tools(menu: &mut DockMenu) {
    menu.heading = "Document tools".into();
    menu.items = (1..=3)
        .map(|index| DockMenuItem::custom(tool_id(index), format!("Tool {index}")))
        .collect();
}

#[derive(Clone, Copy, Debug)]
enum Edit {
    Remove,
    Disable,
    Empty,
}

impl Edit {
    fn callback(self) -> fn(&mut DockMenu) {
        match self {
            Self::Remove => |menu| {
                tools(menu);
                menu.items.remove(0);
            },
            Self::Disable => |menu| {
                tools(menu);
                menu.items[0].enabled = false;
            },
            Self::Empty => |menu| {
                tools(menu);
                menu.items.clear();
            },
        }
    }
}

fn setup(callback: bool) -> Harness {
    let mut harness = Harness::tabs(1);
    if callback {
        harness.local_style = Some((*harness.context.global_style()).clone());
    }
    harness.panes.customize_menu = Some(tools);
    harness
}

#[allow(clippy::cast_possible_truncation)]
fn center(output: &FullOutput, id: NodeId) -> Pos2 {
    let bounds = nodes(output)
        .iter()
        .find(|(node, _)| *node == id)
        .unwrap()
        .1
        .bounds()
        .unwrap();
    egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    )
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    }
}

fn assert_one_action(harness: &Harness, index: u8) {
    assert_eq!(
        harness.panes.menu_actions,
        [DockMenuAction {
            context: DockMenuContext {
                surface: SURFACE,
                root: RootId::new(1),
                target: DockMenuTarget::Tab(B.into()),
            },
            id: tool_id(index),
        }]
    );
}

#[test]
fn removed_or_disabled_menu_rows_reject_held_release_and_late_accessibility_click() {
    for callback in [false, true] {
        for edit in [Edit::Remove, Edit::Disable, Edit::Empty] {
            let mut harness = setup(callback);
            let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
            let old = menu_entry(&output, "Tool 1");
            let position = center(&output, old);
            let version = harness.dockspace.version();
            harness.run(WIDTH, vec![Event::PointerMoved(position)]);
            harness.run(WIDTH, vec![button(position, true)]);
            harness.panes.customize_menu = Some(edit.callback());
            harness.run(WIDTH, vec![button(position, false)]);
            settle(&mut harness);
            harness.run(WIDTH, vec![action(old, Action::Click)]);
            assert!(
                harness.panes.menu_actions.is_empty(),
                "{callback}/{edit:?}: old input must not activate another row"
            );
            assert_eq!(harness.dockspace.version(), version);

            harness.panes.customize_menu = Some(tools);
            let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
            let fresh = menu_entry(&output, "Tool 2");
            batch_events(&mut harness, vec![action(fresh, Action::Click)], 2);
            assert_one_action(&harness, 2);
            assert_eq!(harness.dockspace.version(), version);
        }
    }
}

#[test]
fn removing_the_focused_menu_row_keeps_escape_and_fresh_keyboard_input_operable() {
    for callback in [false, true] {
        for edit in [Edit::Remove, Edit::Disable, Edit::Empty] {
            for discards in [0, 2] {
                let mut harness = setup(callback);
                let (output, anchor) = open(&mut harness, "Document 2", Open::Accessible);
                let old = menu_entry(&output, "Tool 1");
                harness.run(WIDTH, vec![action(old, Action::Focus)]);
                settle(&mut harness);
                assert_eq!(
                    harness
                        .context
                        .memory(egui::Memory::focused)
                        .map(|id| id.value()),
                    Some(old.0)
                );
                let version = harness.dockspace.version();
                harness.panes.customize_menu = Some(edit.callback());
                batch_events(&mut harness, vec![key(Key::Escape)], discards);
                assert!(
                    !egui::Popup::is_any_open(&harness.context),
                    "{callback}/{edit:?}/{discards}: removing focus must not trap the menu"
                );
                assert!(harness.panes.menu_actions.is_empty());
                assert_eq!(
                    harness
                        .context
                        .memory(egui::Memory::focused)
                        .map(|id| id.value()),
                    Some(anchor.0),
                    "dismissal returns removed-row focus to the original tab"
                );
                harness.panes.customize_menu = Some(tools);
                open(&mut harness, "Document 2", Open::ShiftF10);
                harness.run(WIDTH, vec![key(Key::ArrowDown), key(Key::Enter)]);
                assert_one_action(&harness, 1);
                assert_eq!(harness.dockspace.version(), version);
            }
        }
    }
}

#[test]
fn reordering_an_open_menu_preserves_focused_identity_and_uses_the_new_navigation_order() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
            let middle = menu_entry(&output, "Tool 2");
            harness.run(WIDTH, vec![action(middle, Action::Focus)]);
            settle(&mut harness);
            harness.panes.customize_menu = Some(|menu| {
                tools(menu);
                menu.items.reverse();
            });
            let output = settle(&mut harness);
            assert_eq!(menu_entry(&output, "Tool 2"), middle);
            assert_eq!(
                harness
                    .context
                    .memory(egui::Memory::focused)
                    .map(|id| id.value()),
                Some(middle.0)
            );
            batch_events(
                &mut harness,
                vec![key(Key::ArrowDown), key(Key::Enter)],
                discards,
            );
            assert_one_action(&harness, 1);
        }
    }
}

#[test]
fn removing_a_menu_row_does_not_take_a_newer_editors_focus_or_text() {
    for callback in [false, true] {
        let mut harness = setup(callback);
        let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
        let old = menu_entry(&output, "Tool 1");
        harness.run(WIDTH, vec![action(old, Action::Focus)]);
        settle(&mut harness);
        let editor = egui::Id::unique("application-menu-search");
        let mut text = String::new();
        harness.panes.customize_menu = Some(Edit::Remove.callback());
        let output = crate::run_ui(
            &harness.context,
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
                events: vec![Event::Text("application text".into()), key(Key::Escape)],
                ..Default::default()
            },
            |ui| {
                ui.memory_mut(|memory| memory.request_focus(editor));
                ui.add(egui::TextEdit::singleline(&mut text).id(editor));
                harness
                    .dockspace
                    .show_single_surface(SURFACE, ui, &mut harness.panes)
                    .unwrap();
            },
        );
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        assert_eq!(text, "application text");
        assert_eq!(harness.context.memory(egui::Memory::focused), Some(editor));
        assert!(harness.panes.menu_actions.is_empty());
    }
}

#[test]
fn an_open_menu_can_navigate_after_losing_its_focused_row_or_becoming_empty() {
    for callback in [false, true] {
        for edit in [Edit::Remove, Edit::Disable, Edit::Empty] {
            for idle in [false, true] {
                let mut harness = setup(callback);
                let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
                let old = menu_entry(&output, "Tool 1");
                harness.run(WIDTH, vec![action(old, Action::Focus)]);
                settle(&mut harness);
                harness.panes.customize_menu = Some(edit.callback());
                if idle || matches!(edit, Edit::Empty) {
                    harness.run(WIDTH, Vec::new());
                    assert!(egui::Popup::is_any_open(&harness.context));
                }
                if matches!(edit, Edit::Empty) {
                    harness.panes.customize_menu = Some(tools);
                }
                batch_events(&mut harness, vec![key(Key::ArrowDown), key(Key::Enter)], 2);
                assert_one_action(&harness, if matches!(edit, Edit::Empty) { 1 } else { 2 });
                assert!(!egui::Popup::is_any_open(&harness.context));
            }
        }
    }
}
