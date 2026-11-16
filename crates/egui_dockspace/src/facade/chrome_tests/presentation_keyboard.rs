//! Layout menus preserve ordered navigation and activation through real widgets.

use std::sync::{Arc, Mutex};

use super::*;

const WIDTH: f32 = 640.0;

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

fn setup(callback: bool) -> Harness {
    let mut harness = floating_harness(DockStyle::default());
    if callback {
        harness.local_style = Some((*harness.context.global_style()).clone());
    }
    let output = settle(&mut harness);
    let floating = harness
        .dockspace
        .view()
        .contained(crate::FloatingPresentationId::new(1))
        .unwrap()
        .rect();
    let anchor = nodes(&output)
        .iter()
        .find_map(|(id, node)| {
            let bounds = node.bounds()?;
            (node.role() == Role::Button
                && node.label() == Some("Layout actions")
                && bounds.y0 >= floating.y()
                && bounds.y1 <= floating.y() + floating.height())
            .then_some(*id)
        })
        .expect("the contained title exposes its real menu button");
    harness.run(WIDTH, vec![action(anchor, egui::accesskit::Action::Click)]);
    let output = settle(&mut harness);
    let left = nodes(&output)
        .iter()
        .find_map(|(id, node)| {
            (node.role() == Role::MenuItem
                && node.label() == Some("Dock Left")
                && !node.is_disabled())
            .then_some(*id)
        })
        .expect("the real menu contains an enabled directional command");
    harness.run(WIDTH, vec![action(left, egui::accesskit::Action::Focus)]);
    let output = settle(&mut harness);
    assert_eq!(focused_label(&harness, &output), Some("Dock Left"));
    harness
}

fn focused_label<'a>(harness: &Harness, output: &'a FullOutput) -> Option<&'a str> {
    let focused = harness.context.memory(egui::Memory::focused)?.value();
    nodes(output)
        .iter()
        .find(|(id, node)| id.0 == focused && node.role() == Role::MenuItem)
        .and_then(|(_, node)| node.label())
}

fn split(harness: &Harness) -> (crate::DockspaceAxis, Vec<Vec<ItemId>>) {
    let view = harness.dockspace.view();
    let split = view
        .surface(SURFACE)
        .unwrap()
        .main_root()
        .unwrap()
        .content()
        .unwrap()
        .split()
        .expect("the directional command creates a real main-root split");
    (
        split.axis(),
        split
            .children()
            .map(|node| node.tabs().unwrap().items().to_vec())
            .collect(),
    )
}

struct BatchProbe {
    discards: usize,
    remaining: Arc<Mutex<Vec<Event>>>,
}

impl egui::plugin::Plugin for BatchProbe {
    fn debug_name(&self) -> &'static str {
        "layout-menu-ordered-keyboard"
    }

    fn output_hook(&mut self, context: &Context, output: &mut FullOutput) {
        if output.platform_output.num_completed_passes == 1 {
            *self.remaining.lock().unwrap() = context.input(|input| input.events.clone());
        }
        if self.discards > 0 {
            self.discards -= 1;
            output
                .platform_output
                .request_discard_reasons
                .push(egui::RepaintCause::new_reason(
                    "retry after the complete layout-menu input batch",
                ));
        }
    }
}

fn batch_events(harness: &mut Harness, events: Vec<Event>, discards: usize) -> Vec<Event> {
    harness.context.options_mut(|options| {
        options.max_passes = (discards + 1).try_into().unwrap();
    });
    let remaining = Arc::new(Mutex::new(Vec::new()));
    harness.context.add_plugin(BatchProbe {
        discards,
        remaining: Arc::clone(&remaining),
    });
    let output = harness.run(WIDTH, events);
    assert_eq!(output.platform_output.num_completed_passes, discards + 1);
    remaining.lock().unwrap().clone()
}

fn batch(harness: &mut Harness, keys: &[Key], discards: usize) -> Vec<Key> {
    batch_events(harness, keys.iter().copied().map(key).collect(), discards)
        .iter()
        .filter_map(|event| match event {
            Event::Key {
                key, pressed: true, ..
            } => Some(*key),
            _ => None,
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
fn enabled_menu_entry(output: &FullOutput, label: &str) -> (NodeId, Pos2) {
    nodes(output)
        .iter()
        .find_map(|(id, node)| {
            let bounds = node.bounds()?;
            (node.role() == Role::MenuItem && node.label() == Some(label) && !node.is_disabled())
                .then_some((
                    *id,
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) * 0.5) as f32,
                        ((bounds.y0 + bounds.y1) * 0.5) as f32,
                    ),
                ))
        })
        .expect("the open menu exposes an enabled command")
}

#[test]
fn presentation_keyboard_batch_preserves_repeated_and_reversed_navigation() {
    for callback in [false, true] {
        for discards in [0, 2] {
            for keys in [
                vec![Key::ArrowDown, Key::ArrowDown],
                vec![Key::ArrowUp, Key::ArrowDown],
            ] {
                let mut separate = setup(callback);
                for &key in &keys {
                    separate.run(WIDTH, vec![super::key(key)]);
                    settle(&mut separate);
                }
                let output = settle(&mut separate);
                let expected = focused_label(&separate, &output).unwrap().to_owned();

                let mut batched = setup(callback);
                let remaining = batch(&mut batched, &keys, discards);
                let output = settle(&mut batched);
                assert_eq!(
                    focused_label(&batched, &output),
                    Some(expected.as_str()),
                    "callback={callback}, discards={discards}, keys={keys:?}"
                );
                assert!(remaining.is_empty(), "accepted navigation must be consumed");
            }
        }
    }
}

#[test]
fn presentation_keyboard_batch_navigates_before_activation_and_late_discard() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut separate = setup(callback);
            for key in [Key::ArrowDown, Key::Enter] {
                separate.run(WIDTH, vec![super::key(key)]);
                settle(&mut separate);
            }
            let expected = split(&separate);
            assert_eq!(
                expected,
                (
                    crate::DockspaceAxis::Horizontal,
                    vec![vec![ItemId::new(1)], vec![ItemId::new(2)]],
                ),
                "Down from Dock Left activates Dock Right in separate frames"
            );
            let mut batched = setup(callback);
            batch(&mut batched, &[Key::ArrowDown, Key::Enter], discards);
            settle(&mut batched);
            assert_eq!(
                split(&batched),
                expected,
                "callback={callback}, discards={discards}: activation must use the ordered destination"
            );
            let version = batched.dockspace.version();
            settle(&mut batched);
            assert_eq!(batched.dockspace.version(), version);
        }
    }
}

#[test]
fn presentation_keyboard_batch_preserves_keys_after_terminal_activation() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let remaining = batch(
                &mut harness,
                &[
                    Key::ArrowDown,
                    Key::Enter,
                    Key::Space,
                    Key::Enter,
                    Key::ArrowUp,
                ],
                discards,
            );
            assert_eq!(remaining, [Key::Space, Key::Enter, Key::ArrowUp]);
            settle(&mut harness);
            assert_eq!(
                split(&harness),
                (
                    crate::DockspaceAxis::Horizontal,
                    vec![vec![ItemId::new(1)], vec![ItemId::new(2)]],
                )
            );
        }
    }
}

#[test]
fn presentation_keyboard_batch_escape_stops_before_the_remaining_activation_keys() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let version = harness.dockspace.version();
            let remaining = batch(
                &mut harness,
                &[Key::ArrowDown, Key::Escape, Key::Enter, Key::ArrowUp],
                discards,
            );
            assert_eq!(remaining, [Key::Enter, Key::ArrowUp]);
            let output = settle(&mut harness);
            assert!(!egui::Popup::is_any_open(&harness.context));
            assert_eq!(harness.dockspace.version(), version);
            let focused = harness
                .context
                .memory(egui::Memory::focused)
                .unwrap()
                .value();
            assert!(nodes(&output).iter().any(|(id, node)| {
                id.0 == focused
                    && node.role() == Role::Button
                    && node.label() == Some("Layout actions")
            }));
        }
    }
}

#[test]
fn presentation_menu_escape_blocks_an_accesskit_activation_in_the_same_packet() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let output = settle(&mut harness);
            let (right, _) = enabled_menu_entry(&output, "Dock Right");
            let version = harness.dockspace.version();
            let click = action(right, egui::accesskit::Action::Click);
            let remaining = batch_events(
                &mut harness,
                vec![key(Key::Escape), click.clone()],
                discards,
            );
            assert!(
                remaining.contains(&click),
                "the terminal suffix remains unconsumed"
            );
            settle(&mut harness);
            assert!(!egui::Popup::is_any_open(&harness.context));
            assert_eq!(
                harness.dockspace.version(),
                version,
                "Escape must dismiss before an AccessKit activation: callback={callback}, discards={discards}",
            );
            assert!(
                harness
                    .dockspace
                    .view()
                    .item(ItemId::new(2))
                    .unwrap()
                    .contained()
                    .is_some()
            );
        }
    }
}

#[test]
fn presentation_menu_escape_cancels_a_pointer_release_in_the_same_packet() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback);
            let output = settle(&mut harness);
            let (right, position) = enabled_menu_entry(&output, "Dock Right");
            let button = |pressed| Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            harness.run(WIDTH, vec![Event::PointerMoved(position)]);
            assert_eq!(
                harness
                    .context
                    .viewport(|viewport| viewport.hits.click.map(|hit| hit.id.value())),
                Some(right.0),
                "the real pointer must target the menu command before cancellation",
            );
            harness.run(WIDTH, vec![button(true)]);
            let version = harness.dockspace.version();
            batch_events(
                &mut harness,
                vec![key(Key::Escape), button(false)],
                discards,
            );
            if discards == 0 {
                assert_eq!(
                    harness.context.viewport(|viewport| viewport
                        .interact_widgets
                        .clicked
                        .map(|id| id.value())),
                    Some(right.0),
                    "the regression must include a genuine framework click",
                );
            }
            settle(&mut harness);
            assert!(!egui::Popup::is_any_open(&harness.context));
            assert_eq!(
                harness.dockspace.version(),
                version,
                "Escape must cancel the held menu click: callback={callback}, discards={discards}",
            );
            assert!(
                harness
                    .dockspace
                    .view()
                    .item(ItemId::new(2))
                    .unwrap()
                    .contained()
                    .is_some()
            );
        }
    }
}
