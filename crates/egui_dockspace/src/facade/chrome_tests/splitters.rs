//! Constrained separator values and endpoints through real keyboard focus.

use super::*;

fn splitter_values(output: &FullOutput) -> (f64, f64, f64) {
    let (_, node) = nodes(output)
        .iter()
        .find(|(_, node)| node.role() == Role::Splitter)
        .unwrap();
    (
        node.numeric_value().unwrap(),
        node.min_numeric_value().unwrap(),
        node.max_numeric_value().unwrap(),
    )
}

#[test]
fn home_end_move_focused_splitter_to_the_reported_accessible_limits() {
    for axis in [
        crate::DockspaceAxis::Horizontal,
        crate::DockspaceAxis::Vertical,
    ] {
        let style = DockStyle::default();
        let mut harness = Harness::new(
            vec!["First".to_owned(), "Second".to_owned()],
            ItemId::new(1),
            style.clone(),
        );
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::equal_split(
                    axis,
                    [
                        DockspaceNode::tabs([ItemId::new(1)]),
                        DockspaceNode::tabs([ItemId::new(2)]),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        harness.dockspace = Dockspace::builder(("separator-limits", axis), layout)
            .style(style)
            .build()
            .unwrap();
        harness.run(800.0, Vec::new());
        let initial = harness.run(800.0, Vec::new());
        let (value, minimum, maximum) = splitter_values(&initial);
        assert!(0.0 < minimum && minimum < value);
        assert!(value < maximum && maximum < 1.0);
        harness.focus(800.0, Role::Splitter, "Resize panes");
        for (key, expected) in [(Key::End, maximum), (Key::Home, minimum)] {
            harness.run(800.0, vec![super::key(key)]);
            let output = harness.run(800.0, Vec::new());
            let (actual, _, _) = splitter_values(&output);
            assert!(
                (actual - expected).abs() < 1e-5,
                "{axis:?} {key:?}: {actual} != {expected}"
            );
            assert!(nodes(&output).iter().any(|(id, node)| {
                node.role() == Role::Splitter
                    && harness
                        .context
                        .memory(egui::Memory::focused)
                        .is_some_and(|focused| focused.value() == id.0)
            }));
        }
    }
}

fn keyboard_resize_harness(axis: crate::DockspaceAxis, junction: bool, callback: bool) -> Harness {
    let mut harness = Harness::new(
        (1..=4).map(|index| format!("Pane {index}")).collect(),
        ItemId::new(1),
        DockStyle::default(),
    );
    let pane = |item| DockspaceNode::tabs([ItemId::new(item)]);
    let node = if junction {
        DockspaceNode::equal_split(
            crate::DockspaceAxis::Horizontal,
            [
                DockspaceNode::equal_split(crate::DockspaceAxis::Vertical, [pane(1), pane(2)])
                    .unwrap(),
                DockspaceNode::equal_split(crate::DockspaceAxis::Vertical, [pane(3), pane(4)])
                    .unwrap(),
            ],
        )
        .unwrap()
    } else {
        DockspaceNode::equal_split(axis, [pane(1), pane(2)]).unwrap()
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), node),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder(("ordered-keyboard-resize", axis, junction), layout)
        .build()
        .unwrap();
    if callback {
        harness.local_style = Some((*harness.context.global_style()).clone());
    }
    settle_keyboard_resize(&mut harness);
    let label = if junction {
        match axis {
            crate::DockspaceAxis::Horizontal => "Resize pane grid horizontally",
            crate::DockspaceAxis::Vertical => "Resize pane grid vertically",
        }
    } else {
        "Resize panes"
    };
    harness.focus(800.0, Role::Splitter, label);
    harness
}

fn settle_keyboard_resize(harness: &mut Harness) -> FullOutput {
    let mut output = harness.run(800.0, Vec::new());
    for _ in 0..3 {
        output = harness.run(800.0, Vec::new());
    }
    output
}

fn resize_key_events(keys: &[Key]) -> Vec<Event> {
    keys.iter()
        .flat_map(|&key| {
            let press = super::key(key);
            let mut release = press.clone();
            if let Event::Key { pressed, .. } = &mut release {
                *pressed = false;
            }
            [press, release]
        })
        .collect()
}

fn separator_geometry(output: &FullOutput) -> Vec<[f64; 7]> {
    nodes(output)
        .iter()
        .filter(|(_, node)| node.role() == Role::Splitter && node.label() == Some("Resize panes"))
        .map(|(_, node)| {
            let bounds = node.bounds().unwrap();
            [
                node.numeric_value().unwrap(),
                node.min_numeric_value().unwrap(),
                node.max_numeric_value().unwrap(),
                bounds.x0,
                bounds.y0,
                bounds.x1,
                bounds.y1,
            ]
        })
        .collect()
}

fn assert_keyboard_resize_batch_matches_separate_frames(junction: bool, keys: &[Key]) {
    assert_keyboard_resize_batch_matches_separate_frames_with_discards(junction, keys, 0);
}

fn assert_keyboard_resize_batch_matches_separate_frames_with_discards(
    junction: bool,
    keys: &[Key],
    discarded_passes: usize,
) {
    for axis in [
        crate::DockspaceAxis::Horizontal,
        crate::DockspaceAxis::Vertical,
    ] {
        let keys: Vec<_> = keys
            .iter()
            .map(|&key| match (axis, key) {
                (crate::DockspaceAxis::Vertical, Key::ArrowLeft) => Key::ArrowUp,
                (crate::DockspaceAxis::Vertical, Key::ArrowRight) => Key::ArrowDown,
                _ => key,
            })
            .collect();
        for callback in [false, true] {
            let mut ordered = keyboard_resize_harness(axis, junction, callback);
            for &key in &keys {
                ordered.run(800.0, resize_key_events(&[key]));
                settle_keyboard_resize(&mut ordered);
            }
            let expected = separator_geometry(&settle_keyboard_resize(&mut ordered));

            let mut batched = keyboard_resize_harness(axis, junction, callback);
            let focused = batched.context.memory(egui::Memory::focused).unwrap();
            batched.context.options_mut(|options| {
                options.max_passes = (discarded_passes + 1).try_into().unwrap();
            });
            batched.context.add_plugin(OverflowNavigationLateDiscard {
                remaining: discarded_passes,
            });
            let batch = batched.run(800.0, resize_key_events(&keys));
            assert_eq!(
                batch.platform_output.num_completed_passes,
                discarded_passes + 1,
            );
            let output = settle_keyboard_resize(&mut batched);
            let actual = separator_geometry(&output);
            assert!(!expected.is_empty());
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(&expected) {
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| (actual - expected).abs() < 1e-4),
                    "junction={junction}, axis={axis:?}, callback={callback}, keys={keys:?}: {actual:?} != {expected:?}",
                );
            }
            assert_eq!(
                batched.context.memory(egui::Memory::focused),
                Some(focused),
                "keyboard resizing must retain focus on the same axis control",
            );
        }
    }
}

#[test]
fn batched_splitter_keyboard_preserves_repeated_increments() {
    assert_keyboard_resize_batch_matches_separate_frames(
        false,
        &[Key::ArrowRight, Key::ArrowRight],
    );
}

#[test]
fn batched_splitter_keyboard_preserves_round_trip_order() {
    assert_keyboard_resize_batch_matches_separate_frames(false, &[Key::ArrowRight, Key::ArrowLeft]);
}

#[test]
fn batched_splitter_keyboard_preserves_endpoint_order() {
    assert_keyboard_resize_batch_matches_separate_frames(false, &[Key::Home, Key::End]);
}

#[test]
fn batched_splitter_keyboard_clamps_each_step_at_the_endpoint() {
    assert_keyboard_resize_batch_matches_separate_frames(
        false,
        &[Key::End, Key::ArrowRight, Key::ArrowLeft],
    );
}

#[test]
fn batched_junction_keyboard_preserves_repeated_increments() {
    assert_keyboard_resize_batch_matches_separate_frames(true, &[Key::ArrowRight, Key::ArrowRight]);
}

#[test]
fn batched_junction_keyboard_preserves_round_trip_order() {
    assert_keyboard_resize_batch_matches_separate_frames(true, &[Key::ArrowRight, Key::ArrowLeft]);
}

#[test]
fn batched_junction_keyboard_preserves_endpoint_order() {
    assert_keyboard_resize_batch_matches_separate_frames(true, &[Key::Home, Key::End]);
}

#[test]
fn batched_junction_keyboard_clamps_each_step_at_the_endpoint() {
    assert_keyboard_resize_batch_matches_separate_frames(
        true,
        &[Key::End, Key::ArrowRight, Key::ArrowLeft],
    );
}

#[test]
fn batched_splitter_keyboard_applies_once_across_late_discarded_passes() {
    assert_keyboard_resize_batch_matches_separate_frames_with_discards(
        false,
        &[Key::ArrowRight, Key::ArrowRight],
        2,
    );
}

#[test]
fn batched_junction_keyboard_applies_once_across_late_discarded_passes() {
    assert_keyboard_resize_batch_matches_separate_frames_with_discards(
        true,
        &[Key::ArrowRight, Key::ArrowRight],
        2,
    );
}

fn primary(position: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn double_click_equalizes_the_actual_splitter_and_can_be_disabled() {
    for enabled in [false, true] {
        let mut harness = Harness::new(
            vec!["First".into(), "Second".into()],
            ItemId::new(1),
            DockStyle::default(),
        );
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::split(
                    crate::DockspaceAxis::Horizontal,
                    [
                        (DockspaceNode::tabs([ItemId::new(1)]), 0.25),
                        (DockspaceNode::tabs([ItemId::new(2)]), 0.75),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        harness.dockspace = Dockspace::builder(("pointer-equalize", enabled), layout)
            .input(crate::DockInputConfig {
                double_click_equalizes_splitter: enabled,
                ..Default::default()
            })
            .build()
            .unwrap();
        harness.run(800.0, Vec::new());
        let initial = harness.run(800.0, Vec::new());
        let before = splitter_values(&initial).0;
        assert!((before - 0.5).abs() > 0.1);
        let node = nodes(&initial)
            .iter()
            .find(|(_, node)| node.role() == Role::Splitter)
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let position = egui::pos2(
            ((node.x0 + node.x1) * 0.5) as f32,
            ((node.y0 + node.y1) * 0.5) as f32,
        );
        harness.run(800.0, vec![Event::PointerMoved(position)]);
        for _ in 0..2 {
            harness.run(800.0, vec![primary(position, true)]);
            harness.run(800.0, vec![primary(position, false)]);
        }
        let output = harness.run(800.0, Vec::new());
        let after = splitter_values(&output).0;
        assert!(
            (after - if enabled { 0.5 } else { before }).abs() < 1e-5,
            "enabled={enabled}, before={before}, after={after}"
        );
    }
}
