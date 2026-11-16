//! Floating edges preserve every key through the same constrained resize path.

use super::*;
use std::sync::{Arc, Mutex};

const WIDTH: f32 = 640.0;

fn settle(harness: &mut Harness) {
    for _ in 0..3 {
        harness.run(WIDTH, Vec::new());
    }
}

fn events(keys: &[Key]) -> Vec<Event> {
    keys.iter().copied().map(key).collect()
}

fn geometry(harness: &Harness) -> crate::LogicalRect {
    harness
        .dockspace
        .view()
        .contained(crate::FloatingPresentationId::new(1))
        .unwrap()
        .rect()
}

fn setup(callback: bool, label: &str) -> Harness {
    let mut harness = floating_harness(DockStyle::default());
    if callback {
        harness.local_style = Some((*harness.context.global_style()).clone());
    }
    settle(&mut harness);
    harness.focus(WIDTH, Role::Splitter, label);
    settle(&mut harness);
    harness
}

fn assert_batch_matches_separate_frames(keys: &[Key], at_maximum: bool, discards: usize) {
    for (label, horizontal) in [
        ("Resize floating right edge", true),
        ("Resize floating left edge", true),
        ("Resize floating bottom edge", false),
        ("Resize floating top edge", false),
    ] {
        let keys: Vec<_> = keys
            .iter()
            .map(|&key| match (horizontal, key) {
                (false, Key::ArrowRight) => Key::ArrowDown,
                (false, Key::ArrowLeft) => Key::ArrowUp,
                _ => key,
            })
            .collect();
        for callback in [false, true] {
            let fixture = || {
                let mut harness = setup(callback, label);
                if at_maximum {
                    let increment = if horizontal {
                        Key::ArrowRight
                    } else {
                        Key::ArrowDown
                    };
                    let mut reached_limit = false;
                    for _ in 0..64 {
                        let before = geometry(&harness);
                        harness.run(WIDTH, vec![key(increment)]);
                        settle(&mut harness);
                        if geometry(&harness) == before {
                            reached_limit = true;
                            break;
                        }
                    }
                    assert!(
                        reached_limit,
                        "the real edge reaches its constrained endpoint"
                    );
                }
                harness
            };
            let mut separate = fixture();
            for &key in &keys {
                separate.run(WIDTH, vec![super::key(key)]);
                settle(&mut separate);
            }
            let expected = geometry(&separate);

            let mut batched = fixture();
            let before = geometry(&batched);
            let focused = batched.context.memory(egui::Memory::focused);
            batched.context.options_mut(|options| {
                options.max_passes = (discards + 1).try_into().unwrap();
            });
            batched.context.add_plugin(OverflowNavigationLateDiscard {
                remaining: discards,
            });
            let output = batched.run(WIDTH, events(&keys));
            assert_eq!(output.platform_output.num_completed_passes, discards + 1);
            settle(&mut batched);
            let actual = geometry(&batched);
            assert_ne!(expected, before, "the ordered sequence must move the edge");
            assert!(
                [
                    actual.x() - expected.x(),
                    actual.y() - expected.y(),
                    actual.width() - expected.width(),
                    actual.height() - expected.height(),
                ]
                .iter()
                .all(|delta| delta.abs() < 1e-5),
                "{label}, callback={callback}, discards={discards}, keys={keys:?}: {actual:?} != {expected:?}",
            );
            assert_eq!(
                batched.context.memory(egui::Memory::focused),
                focused,
                "resizing must preserve the same focused edge"
            );
        }
    }
}

#[test]
fn contained_keyboard_batch_preserves_repeated_steps_and_late_discard() {
    for discards in [0, 2] {
        assert_batch_matches_separate_frames(&[Key::ArrowRight, Key::ArrowRight], false, discards);
    }
}

#[test]
fn contained_keyboard_batch_clamps_each_step_before_reversing_at_the_limit() {
    for discards in [0, 2] {
        assert_batch_matches_separate_frames(&[Key::ArrowRight, Key::ArrowLeft], true, discards);
    }
}

#[derive(Clone, Copy, Debug)]
enum ResizeInput {
    Keyboard(Key),
    Accessibility(egui::accesskit::Action),
}

fn resize_event(target: egui::Id, input: ResizeInput, horizontal: bool) -> Event {
    match input {
        ResizeInput::Keyboard(logical_key) => key(match (horizontal, logical_key) {
            (false, Key::ArrowRight) => Key::ArrowDown,
            (false, Key::ArrowLeft) => Key::ArrowUp,
            _ => logical_key,
        }),
        ResizeInput::Accessibility(action) => {
            Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                action,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: NodeId(target.value()),
                data: None,
            })
        }
    }
}

struct ResizeBatchProbe {
    remaining: Arc<Mutex<Option<Vec<Event>>>>,
}

impl egui::plugin::Plugin for ResizeBatchProbe {
    fn debug_name(&self) -> &'static str {
        "contained-adjustment-mixed-input"
    }

    fn output_hook(&mut self, context: &Context, output: &mut FullOutput) {
        if output.platform_output.num_completed_passes == 1 {
            let mut remaining = self.remaining.lock().unwrap();
            // Later discarded passes and idle runs must not replace the
            // target run's first-pass consumption observation.
            if remaining.is_none() {
                *remaining = Some(context.input(|input| input.events.clone()));
            }
        }
    }
}

fn assert_mixed_batch_matches_separate_frames(inputs: &[ResizeInput], at_maximum: bool) {
    for (label, horizontal) in [
        ("Resize floating right edge", true),
        ("Resize floating bottom edge", false),
    ] {
        for callback in [false, true] {
            for discards in [0, 2] {
                let fixture = || {
                    let mut harness = setup(callback, label);
                    if at_maximum {
                        let increment = if horizontal {
                            Key::ArrowRight
                        } else {
                            Key::ArrowDown
                        };
                        for _ in 0..64 {
                            let before = geometry(&harness);
                            harness.run(WIDTH, vec![key(increment)]);
                            settle(&mut harness);
                            if geometry(&harness) == before {
                                return harness;
                            }
                        }
                        panic!("the edge must reach its actual constrained endpoint");
                    }
                    harness
                };
                let mut separate = fixture();
                let separate_target = separate.context.memory(egui::Memory::focused).unwrap();
                for &input in inputs {
                    separate.run(
                        WIDTH,
                        vec![resize_event(separate_target, input, horizontal)],
                    );
                    settle(&mut separate);
                }
                let expected = geometry(&separate);

                let mut batched = fixture();
                let target = batched.context.memory(egui::Memory::focused).unwrap();
                let remaining = Arc::new(Mutex::new(None));
                batched.context.add_plugin(ResizeBatchProbe {
                    remaining: Arc::clone(&remaining),
                });
                batched.context.add_plugin(OverflowNavigationLateDiscard {
                    remaining: discards,
                });
                batched.context.options_mut(|options| {
                    options.max_passes = (discards + 1).try_into().unwrap();
                });
                let output = batched.run(
                    WIDTH,
                    inputs
                        .iter()
                        .map(|&input| resize_event(target, input, horizontal))
                        .collect(),
                );
                assert_eq!(output.platform_output.num_completed_passes, discards + 1);
                settle(&mut batched);
                assert_eq!(
                    geometry(&batched),
                    expected,
                    "{label}, callback={callback}, discards={discards}, inputs={inputs:?}"
                );
                assert!(
                    remaining
                        .lock()
                        .unwrap()
                        .as_ref()
                        .expect("the target input run must be observed")
                        .is_empty(),
                    "accepted keyboard and accessibility events must each be consumed"
                );
                assert_eq!(
                    batched.context.memory(egui::Memory::focused),
                    Some(target),
                    "mixed input must retain the same focused edge"
                );
            }
        }
    }
}

#[test]
fn contained_accesskit_batch_preserves_repeated_adjustments() {
    assert_mixed_batch_matches_separate_frames(
        &[ResizeInput::Accessibility(egui::accesskit::Action::Increment); 2],
        false,
    );
}

#[test]
fn contained_accesskit_batch_clamps_before_reversing_at_the_endpoint() {
    assert_mixed_batch_matches_separate_frames(
        &[
            ResizeInput::Accessibility(egui::accesskit::Action::Increment),
            ResizeInput::Accessibility(egui::accesskit::Action::Decrement),
        ],
        true,
    );
}

#[test]
fn contained_mixed_input_batch_keeps_keyboard_and_accesskit_in_their_raw_order() {
    assert_mixed_batch_matches_separate_frames(
        &[
            ResizeInput::Keyboard(Key::ArrowRight),
            ResizeInput::Accessibility(egui::accesskit::Action::Increment),
            ResizeInput::Keyboard(Key::ArrowLeft),
        ],
        false,
    );
    assert_mixed_batch_matches_separate_frames(
        &[
            ResizeInput::Accessibility(egui::accesskit::Action::Decrement),
            ResizeInput::Keyboard(Key::ArrowRight),
            ResizeInput::Accessibility(egui::accesskit::Action::Decrement),
        ],
        true,
    );
}

#[test]
fn contained_accesskit_batch_adjusts_without_focus_and_preserves_foreign_requests() {
    for callback in [false, true] {
        for discards in [0, 2] {
            let mut harness = setup(callback, "Resize floating right edge");
            let target = harness.context.memory(egui::Memory::focused).unwrap();
            harness
                .context
                .memory_mut(|memory| memory.surrender_focus(target));
            settle(&mut harness);
            let before = geometry(&harness);
            let remaining = Arc::new(Mutex::new(None));
            harness.context.add_plugin(ResizeBatchProbe {
                remaining: Arc::clone(&remaining),
            });
            harness.context.add_plugin(OverflowNavigationLateDiscard {
                remaining: discards,
            });
            harness.context.options_mut(|options| {
                options.max_passes = (discards + 1).try_into().unwrap();
            });
            let increment = ResizeInput::Accessibility(egui::accesskit::Action::Increment);
            let foreign = resize_event(
                egui::Id::unique("another-accessibility-consumer"),
                increment,
                true,
            );
            let output = harness.run(
                WIDTH,
                vec![
                    resize_event(target, increment, true),
                    foreign.clone(),
                    resize_event(target, increment, true),
                ],
            );
            assert_eq!(output.platform_output.num_completed_passes, discards + 1);
            settle(&mut harness);
            let actual = geometry(&harness);
            assert_eq!(actual.min(), before.min());
            assert_eq!(actual.width(), before.width() + 32.0);
            assert_eq!(actual.height(), before.height());
            assert_eq!(
                remaining
                    .lock()
                    .unwrap()
                    .as_ref()
                    .expect("the target input run must be observed"),
                &vec![foreign],
            );
            assert_ne!(
                harness.context.memory(egui::Memory::focused),
                Some(target),
                "accessibility adjustments do not acquire keyboard focus"
            );
        }
    }
}
