//! One real keyboard packet preserves tab-list command order and its terminal tail.

use std::sync::{Arc, Mutex};

use crate::TabKey;
use dockspace::runtime::SurfacePaintProgramStep as Step;

use super::*;

const WIDTH: f32 = 640.0;
const LAST: ItemId = ItemId::new(40);

fn setup(callback: bool) -> Harness {
    let mut harness = Harness::new(
        (1..=LAST.get())
            .map(|item| format!("Document {item}"))
            .collect(),
        ItemId::new(1),
        DockStyle {
            tab_min_width: 180.0,
            tab_max_width: 180.0,
            ..Default::default()
        },
    );
    if callback {
        harness.local_style = Some((*harness.context.global_style()).clone());
    }
    let mut output = harness.run(WIDTH, Vec::new());
    for _ in 0..3 {
        output = harness.run(WIDTH, Vec::new());
    }
    let trigger = nodes(&output)
        .iter()
        .find_map(|(id, node)| (node.label() == Some("List tabs")).then_some(*id))
        .unwrap();
    harness.run(
        WIDTH,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: trigger,
                data: None,
            },
        )],
    );
    settle(&mut harness);
    harness.run(WIDTH, vec![key(Key::Home)]);
    let output = settle(&mut harness);
    let first = nodes(&output)
        .iter()
        .find_map(|(id, node)| {
            (node.role() == Role::MenuItem && node.label() == Some("Document 1")).then_some(*id)
        })
        .unwrap();
    assert_eq!(
        harness
            .context
            .memory(egui::Memory::focused)
            .map(|id| id.value()),
        Some(first.0)
    );
    assert_eq!(menu_state(&mut harness).unwrap().0, ItemId::new(1).into());
    harness
}

fn settle(harness: &mut Harness) -> FullOutput {
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new())
}

fn menu_state(harness: &mut Harness) -> Option<(TabKey, f64, bool)> {
    let mut frame = harness.dockspace.session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let mut state = None;
    plan.visit_surface_program(|step| {
        if let Step::PopupMenu { menu, .. } = step {
            let focused = menu.rows().find(|row| row.focused()).unwrap().key();
            let last_visible = menu
                .rows()
                .find(|row| row.item() == Some(LAST))
                .unwrap()
                .interaction_bounds()
                .is_some();
            state = Some((focused, menu.scroll_offset(), last_visible));
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    state
}

struct BatchProbe {
    discarded_passes: usize,
    remaining: Arc<Mutex<Vec<Event>>>,
}

impl egui::plugin::Plugin for BatchProbe {
    fn debug_name(&self) -> &'static str {
        "tab-list-keyboard-batch"
    }

    fn output_hook(&mut self, context: &Context, output: &mut FullOutput) {
        // Context::end_pass has already moved out its PlatformOutput; the
        // returned output retains the actual pass count at this hook.
        if output.platform_output.num_completed_passes == 1 {
            *self.remaining.lock().unwrap() = context.input(|input| input.events.clone());
        }
        if self.discarded_passes > 0 {
            self.discarded_passes -= 1;
            output
                .platform_output
                .request_discard_reasons
                .push(egui::RepaintCause::new_reason(
                    "retry after the tab-list keyboard batch",
                ));
        }
    }
}

fn batch(harness: &mut Harness, events: Vec<Event>, discards: usize) -> Vec<Event> {
    harness
        .context
        .options_mut(|options| options.max_passes = (discards + 1).try_into().unwrap());
    let remaining = Arc::new(Mutex::new(Vec::new()));
    harness.context.add_plugin(BatchProbe {
        discarded_passes: discards,
        remaining: remaining.clone(),
    });
    let output = harness.run(WIDTH, events);
    assert_eq!(output.platform_output.num_completed_passes, discards + 1);
    let observed = remaining.lock().unwrap().clone();
    observed
}

fn pressed_keys(events: &[Event]) -> Vec<Key> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Key {
                key, pressed: true, ..
            } => Some(*key),
            _ => None,
        })
        .collect()
}

#[test]
fn tab_list_keyboard_batch_keeps_repeated_and_ordered_navigation() {
    for callback in [false, true] {
        for discards in [0, 2] {
            for (keys, expected) in [
                (vec![Key::ArrowDown, Key::ArrowDown], ItemId::new(3)),
                (vec![Key::End, Key::ArrowUp], ItemId::new(39)),
                (
                    vec![Key::Home, Key::ArrowDown, Key::ArrowUp],
                    ItemId::new(1),
                ),
            ] {
                let mut harness = setup(callback);
                let remaining = batch(
                    &mut harness,
                    keys.iter().copied().map(key).collect(),
                    discards,
                );
                let output = settle(&mut harness);
                assert_eq!(
                    menu_state(&mut harness).unwrap().0,
                    TabKey::from(expected),
                    "{keys:?}, callback={callback}, discards={discards}"
                );
                assert!(
                    pressed_keys(&remaining).is_empty(),
                    "every accepted navigation event must be consumed: {remaining:?}"
                );
                let label = format!("Document {}", expected.get());
                let target = nodes(&output)
                    .iter()
                    .find_map(|(id, node)| {
                        (node.role() == Role::MenuItem && node.label() == Some(label.as_str()))
                            .then_some(*id)
                    })
                    .unwrap();
                assert_eq!(
                    harness
                        .context
                        .memory(egui::Memory::focused)
                        .map(|id| id.value()),
                    Some(target.0)
                );
            }
        }
    }
}

#[test]
fn tab_list_keyboard_batch_activates_an_offscreen_destination() {
    for callback in [false, true] {
        for discards in [0, 2] {
            for activate in [Key::Enter, Key::Space] {
                let mut harness = setup(callback);
                assert!(
                    !menu_state(&mut harness).unwrap().2,
                    "End must target a row that has no current pointer hit rectangle"
                );
                let remaining = batch(&mut harness, vec![key(Key::End), key(activate)], discards);
                let output = settle(&mut harness);
                assert!(
                    harness.dockspace.view().item(LAST).unwrap().is_selected(),
                    "End then {activate:?} must activate the terminal row in the same packet"
                );
                assert!(menu_state(&mut harness).is_none());
                assert!(
                    nodes(&output)
                        .iter()
                        .all(|(_, node)| node.role() != Role::Menu)
                );
                assert!(pressed_keys(&remaining).is_empty());
                assert_eq!(
                    harness.context.memory(egui::Memory::focused),
                    harness.panes.focus_target(LAST)
                );
            }
        }
    }
}

#[test]
fn tab_list_keyboard_batch_escape_closes_after_navigation() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let remaining = batch(
                &mut harness,
                vec![key(Key::End), key(Key::Escape)],
                discards,
            );
            let output = settle(&mut harness);
            assert!(menu_state(&mut harness).is_none());
            assert!(
                harness
                    .dockspace
                    .view()
                    .item(ItemId::new(1))
                    .unwrap()
                    .is_selected()
            );
            assert!(pressed_keys(&remaining).is_empty());
            let trigger = nodes(&output)
                .iter()
                .find_map(|(id, node)| (node.label() == Some("List tabs")).then_some(*id))
                .unwrap();
            assert_eq!(
                harness
                    .context
                    .memory(egui::Memory::focused)
                    .map(|id| id.value()),
                Some(trigger.0)
            );
        }
    }
}

#[test]
fn tab_list_keyboard_batch_preserves_the_events_after_its_terminal_command() {
    for terminal in [Key::Enter, Key::Escape] {
        for discards in [0, 2] {
            let mut harness = setup(false);
            let remaining = batch(
                &mut harness,
                vec![
                    key(Key::End),
                    key(terminal),
                    key(Key::ArrowDown),
                    Event::Text("application tail".into()),
                ],
                discards,
            );
            assert_eq!(pressed_keys(&remaining), vec![Key::ArrowDown]);
            assert!(
                remaining.contains(&Event::Text("application tail".into())),
                "the menu must leave trailing application input to its original owner"
            );
            settle(&mut harness);
            assert!(menu_state(&mut harness).is_none());
            let selected = if terminal == Key::Enter {
                LAST
            } else {
                ItemId::new(1)
            };
            assert!(
                harness
                    .dockspace
                    .view()
                    .item(selected)
                    .unwrap()
                    .is_selected()
            );
        }
    }
}

#[test]
fn tab_list_keyboard_batch_escape_after_tab_uses_the_new_input_owner() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let output = harness.run(WIDTH, vec![key(Key::Tab)]);
            let second = nodes(&output)
                .iter()
                .find_map(|(id, node)| {
                    (node.role() == Role::MenuItem && node.label() == Some("Document 2"))
                        .then_some(*id)
                })
                .unwrap();
            assert_eq!(
                harness
                    .context
                    .memory(egui::Memory::focused)
                    .map(|id| id.value()),
                Some(second.0),
                "the new row has not had an idle pass to install its focus filter"
            );
            let remaining = batch(
                &mut harness,
                vec![key(Key::End), key(Key::Escape)],
                discards,
            );
            assert!(pressed_keys(&remaining).is_empty(), "{remaining:?}");
            settle(&mut harness);
            assert!(menu_state(&mut harness).is_none());
            assert!(
                harness
                    .dockspace
                    .view()
                    .item(ItemId::new(1))
                    .unwrap()
                    .is_selected()
            );
        }
    }
}
