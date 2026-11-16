//! Accessibility and keyboard adjustments share one ordered resize sequence.

use egui::accesskit::{Action, ActionRequest, TreeId};

use super::*;

#[derive(Debug, Clone, Copy)]
enum Adjustment {
    Keyboard(Key),
    Accessibility(Action),
}

fn events(target: egui::Id, axis: crate::DockspaceAxis, inputs: &[Adjustment]) -> Vec<Event> {
    inputs
        .iter()
        .flat_map(|input| match *input {
            Adjustment::Keyboard(key) => {
                let key = match (axis, key) {
                    (crate::DockspaceAxis::Vertical, Key::ArrowLeft) => Key::ArrowUp,
                    (crate::DockspaceAxis::Vertical, Key::ArrowRight) => Key::ArrowDown,
                    _ => key,
                };
                resize_key_events(&[key])
            }
            Adjustment::Accessibility(action) => {
                vec![Event::AccessKitActionRequest(ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: target.accesskit_id(),
                    data: None,
                })]
            }
        })
        .collect()
}

fn assert_batch_matches_separate_frames(inputs: &[Adjustment], discards: usize) {
    for junction in [false, true] {
        for axis in [
            crate::DockspaceAxis::Horizontal,
            crate::DockspaceAxis::Vertical,
        ] {
            for callback in [false, true] {
                let mut separate = keyboard_resize_harness(axis, junction, callback);
                let target = separate.context.memory(egui::Memory::focused).unwrap();
                for input in inputs {
                    let event_batch = events(target, axis, std::slice::from_ref(input));
                    separate.run(800.0, event_batch);
                    settle_keyboard_resize(&mut separate);
                    assert_eq!(
                        separate.context.memory(egui::Memory::focused),
                        Some(target),
                        "the separate-frame oracle must keep the same resize axis: junction={junction}, axis={axis:?}, callback={callback}, input={input:?}",
                    );
                }
                let expected = separator_geometry(&settle_keyboard_resize(&mut separate));

                let mut batched = keyboard_resize_harness(axis, junction, callback);
                let focused = batched.context.memory(egui::Memory::focused).unwrap();
                batched.context.options_mut(|options| {
                    options.max_passes = (discards + 1).try_into().unwrap();
                });
                batched.context.add_plugin(OverflowNavigationLateDiscard {
                    remaining: discards,
                });
                let input = events(focused, axis, inputs);
                let output = batched.run(800.0, input);
                assert_eq!(output.platform_output.num_completed_passes, discards + 1);
                let actual = separator_geometry(&settle_keyboard_resize(&mut batched));
                assert!(!expected.is_empty());
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert!(
                        actual
                            .iter()
                            .zip(expected)
                            .all(|(a, b)| (a - b).abs() < 1e-4),
                        "junction={junction}, axis={axis:?}, callback={callback}, discards={discards}, inputs={inputs:?}: {actual:?} != {expected:?}",
                    );
                }
                assert_eq!(
                    batched.context.memory(egui::Memory::focused),
                    Some(focused),
                    "adjustment batches must retain focus on the same resize axis",
                );
            }
        }
    }
}

#[test]
fn accesskit_resize_batch_keeps_repeated_and_opposing_steps() {
    use Adjustment::Accessibility as A;
    for discards in [0, 2] {
        for inputs in [
            [A(Action::Increment), A(Action::Increment)],
            [A(Action::Increment), A(Action::Decrement)],
        ] {
            assert_batch_matches_separate_frames(&inputs, discards);
        }
    }
}

#[test]
fn mixed_resize_batch_keeps_event_order_and_endpoint_clamping() {
    use Adjustment::{Accessibility as A, Keyboard as K};
    for discards in [0, 2] {
        for inputs in [
            vec![A(Action::Increment), K(Key::ArrowRight)],
            vec![K(Key::End), A(Action::Increment), A(Action::Decrement)],
            vec![K(Key::Home), A(Action::Decrement), A(Action::Increment)],
            vec![A(Action::Decrement), K(Key::End)],
        ] {
            assert_batch_matches_separate_frames(&inputs, discards);
        }
    }
}
