//! Configured cycling follows real focused widgets and keeps one deferred action per run.

use super::*;
use egui::{KeyboardShortcut, Modifiers};
use std::sync::{Arc, Mutex};

fn bindings() -> crate::DockInputConfig {
    crate::DockInputConfig {
        next_tab: Some(KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)),
        previous_tab: Some(KeyboardShortcut::new(
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::Tab,
        )),
        ..Default::default()
    }
}

fn shortcut(shortcut: KeyboardShortcut) -> Event {
    Event::Key {
        key: shortcut.logical_key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: shortcut.modifiers,
    }
}

fn shortcut_remains(ui: &Ui, shortcut: KeyboardShortcut) -> bool {
    ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, Event::Key { key, modifiers, pressed: true, .. }
                if *key == shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers))
        })
    })
}

fn present(harness: &mut Harness, width: f32, events: Vec<Event>) -> FullOutput {
    harness.run(width, events);
    let mut output = harness.run(width, Vec::new());
    for _ in 0..2 {
        output = harness.run(width, Vec::new());
    }
    output
}

fn assert_selected(harness: &Harness, expected: u64) {
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(expected))
            .unwrap()
            .is_selected()
    );
}

#[track_caller]
fn assert_tab_focus(harness: &Harness, output: &FullOutput, expected: u64) {
    let label = format!("Document {expected}");
    let (id, _) = nodes(output)
        .iter()
        .find(|(_, node)| node.role() == Role::Tab && node.label() == Some(label.as_str()))
        .expect("cycling must reveal the destination tab");
    let focused = harness
        .context
        .memory(egui::Memory::focused)
        .map(|focused| focused.value());
    assert_eq!(
        focused,
        Some(id.0),
        "expected {label}; actual focused control: {:?}",
        nodes(output)
            .iter()
            .find(|(id, _)| Some(id.0) == focused)
            .map(|(_, node)| (node.role(), node.label()))
    );
}

fn run_with(
    harness: &mut Harness,
    width: f32,
    events: Vec<Event>,
    mut paint: impl FnMut(&mut Ui, &mut Dockspace, &mut Panes),
) -> FullOutput {
    harness.frame += 1;
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0))),
        time: Some(f64::from(harness.frame) * 0.1),
        events,
        ..Default::default()
    };
    let output = crate::run_ui(&harness.context, raw, |ui| {
        paint(ui, &mut harness.dockspace, &mut harness.panes);
    });
    let (mut output, receipt) = output.into_parts();
    receipt.report(SurfacePresentationResult::Presented);
    output.textures_delta.clear();
    output
}

#[test]
fn configured_cycle_wraps_and_reveals_targets_from_tab_close_and_content_focus() {
    for callback in [false, true] {
        let mut harness = Harness::tabs(1);
        harness.dockspace.set_input(bindings());
        if callback {
            harness.local_style = Some((*harness.context.global_style()).clone());
        }
        present(&mut harness, 360.0, Vec::new());
        harness.context.memory_mut(|memory| {
            memory.request_focus(harness.panes.focus_target(ItemId::new(1)).unwrap())
        });
        harness.run(360.0, Vec::new());
        let output = present(
            &mut harness,
            360.0,
            vec![shortcut(bindings().previous_tab.unwrap())],
        );
        assert_selected(&harness, 6);
        assert_tab_focus(&harness, &output, 6);
        let output = present(
            &mut harness,
            360.0,
            vec![shortcut(bindings().next_tab.unwrap())],
        );
        assert_selected(&harness, 1);
        assert_tab_focus(&harness, &output, 1);
        harness.focus(360.0, Role::Button, "Close Document 1");
        let output = present(
            &mut harness,
            360.0,
            vec![shortcut(bindings().next_tab.unwrap())],
        );
        assert_selected(&harness, 2);
        assert_tab_focus(&harness, &output, 2);
        harness.run(360.0, vec![key(Key::Enter)]);
        present(&mut harness, 360.0, Vec::new());
        assert_eq!(
            harness.context.memory(egui::Memory::focused),
            harness.panes.focus_target(ItemId::new(2))
        );
    }
}

#[test]
fn cycling_is_opt_in_and_runtime_changes_match_shift_and_alt_exactly() {
    let mut harness = Harness::tabs(1);
    present(&mut harness, 800.0, Vec::new());
    harness.focus_tab(800.0, "Document 1");
    present(
        &mut harness,
        800.0,
        vec![shortcut(bindings().next_tab.unwrap())],
    );
    assert_selected(&harness, 1);
    harness.dockspace.set_input(bindings());
    present(&mut harness, 800.0, Vec::new());
    assert_selected(&harness, 1);
    let output = present(
        &mut harness,
        800.0,
        vec![shortcut(bindings().previous_tab.unwrap())],
    );
    assert_selected(&harness, 6);
    assert_tab_focus(&harness, &output, 6);
    present(
        &mut harness,
        800.0,
        vec![shortcut(KeyboardShortcut::new(
            Modifiers::CTRL | Modifiers::ALT,
            Key::Tab,
        ))],
    );
    assert_selected(&harness, 6);
    let mut custom = bindings();
    custom.next_tab = Some(KeyboardShortcut::new(Modifiers::ALT, Key::F6));
    custom.previous_tab = None;
    harness.dockspace.set_input(custom);
    present(
        &mut harness,
        800.0,
        vec![shortcut(bindings().next_tab.unwrap())],
    );
    assert_selected(&harness, 6);
    present(
        &mut harness,
        800.0,
        vec![shortcut(custom.next_tab.unwrap())],
    );
    assert_selected(&harness, 1);
    harness.dockspace.set_input(Default::default());
    present(
        &mut harness,
        800.0,
        vec![shortcut(custom.next_tab.unwrap())],
    );
    assert_selected(&harness, 1);
}

#[test]
fn cycling_uses_the_focused_group_instead_of_the_pointer_group() {
    let mut harness = Harness::tabs(1);
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
    harness.dockspace = Dockspace::builder("cycle-focused-group", layout)
        .input(bindings())
        .build()
        .unwrap();
    present(&mut harness, 800.0, Vec::new());
    harness.context.memory_mut(|memory| {
        memory.request_focus(harness.panes.focus_target(ItemId::new(3)).unwrap())
    });
    harness.run(800.0, Vec::new());
    let output = present(
        &mut harness,
        800.0,
        vec![
            Event::PointerMoved(Pos2::new(30.0, 90.0)),
            shortcut(bindings().next_tab.unwrap()),
        ],
    );
    assert_selected(&harness, 1);
    assert_selected(&harness, 4);
    assert_tab_focus(&harness, &output, 4);
}

#[test]
fn cycling_from_strip_controls_uses_their_group() {
    let mut harness = Harness::tabs(1);
    harness.dockspace.set_input(bindings());
    present(&mut harness, 360.0, Vec::new());
    harness.focus(360.0, Role::Button, "Scroll tabs forward");
    let output = present(
        &mut harness,
        360.0,
        vec![shortcut(bindings().next_tab.unwrap())],
    );
    assert_selected(&harness, 2);
    assert_tab_focus(&harness, &output, 2);
}

#[test]
fn cycling_from_content_or_strip_controls_uses_the_offscreen_selection() {
    for content_focus in [false, true] {
        for (binding, expected) in [
            (bindings().next_tab.unwrap(), 1),
            (bindings().previous_tab.unwrap(), 5),
        ] {
            let mut harness = Harness::tabs(6);
            harness.dockspace.set_input(bindings());
            let output = present(&mut harness, 360.0, Vec::new());
            let backward = nodes(&output)
                .iter()
                .find_map(|(id, node)| {
                    (node.label() == Some("Scroll tabs backward")).then_some(*id)
                })
                .unwrap();
            for _ in 0..4 {
                present(
                    &mut harness,
                    360.0,
                    vec![Event::AccessKitActionRequest(
                        egui::accesskit::ActionRequest {
                            action: egui::accesskit::Action::Click,
                            target_tree: egui::accesskit::TreeId::ROOT,
                            target_node: backward,
                            data: None,
                        },
                    )],
                );
            }
            if content_focus {
                harness.context.memory_mut(|memory| {
                    memory.request_focus(harness.panes.focus_target(ItemId::new(6)).unwrap());
                });
            } else {
                harness.focus(360.0, Role::Button, "Scroll tabs forward");
            }
            let output = present(&mut harness, 360.0, Vec::new());
            assert_selected(&harness, 6);
            assert!(
                !nodes(&output).iter().any(|(_, node)| {
                    node.role() == Role::Tab && node.label() == Some("Document 6")
                }),
                "manual scrolling must leave the selected header completely offscreen"
            );
            let output = present(&mut harness, 360.0, vec![shortcut(binding)]);
            assert_selected(&harness, expected);
            assert_tab_focus(&harness, &output, expected);
        }
    }
}

#[test]
fn late_discarded_passes_cycle_once_with_the_same_deferred_intent() {
    for callback in [false, true] {
        for count in [1, 3] {
            let mut harness = Harness::tabs(1);
            harness.dockspace.set_input(bindings());
            if callback {
                harness.local_style = Some((*harness.context.global_style()).clone());
            }
            present(&mut harness, 800.0, Vec::new());
            harness.focus_tab(800.0, "Document 1");
            harness
                .context
                .options_mut(|options| options.max_passes = 4.try_into().unwrap());
            harness
                .context
                .add_plugin(OverflowNavigationLateDiscard { remaining: 2 });
            let events = vec![shortcut(bindings().next_tab.unwrap()); count];
            let output = harness.run(800.0, events.clone());
            assert_eq!(output.platform_output.num_completed_passes, 3);
            let output = present(&mut harness, 800.0, Vec::new());
            let expected = 1 + count as u64;
            assert_selected(&harness, expected);
            assert_tab_focus(&harness, &output, expected);
            for _ in 0..3 {
                harness.run(800.0, Vec::new());
                assert_selected(&harness, expected);
            }
            let output = present(&mut harness, 800.0, events);
            let expected = 1 + (2 * count as u64) % 6;
            assert_selected(&harness, expected);
            assert_tab_focus(&harness, &output, expected);
        }
    }
}

#[test]
fn consecutive_and_repeated_key_presses_visit_every_tab() {
    for callback in [false, true] {
        let mut harness = Harness::tabs(1);
        harness.dockspace.set_input(bindings());
        if callback {
            harness.local_style = Some((*harness.context.global_style()).clone());
        }
        present(&mut harness, 360.0, Vec::new());
        harness.focus_tab(360.0, "Document 1");
        for expected in [2, 3, 4, 5, 6, 1, 2] {
            let mut event = shortcut(bindings().next_tab.unwrap());
            if let Event::Key { repeat, .. } = &mut event {
                *repeat = expected % 2 == 0;
            }
            let output = present(&mut harness, 360.0, vec![event]);
            assert_selected(&harness, expected);
            assert_tab_focus(&harness, &output, expected);
        }
        for _ in 0..4 {
            let mut event = shortcut(bindings().next_tab.unwrap());
            if let Event::Key { repeat, .. } = &mut event {
                *repeat = true;
            }
            harness.run(360.0, vec![event]);
        }
        let output = present(&mut harness, 360.0, Vec::new());
        assert_selected(&harness, 6);
        assert_tab_focus(&harness, &output, 6);
        let mut release = shortcut(bindings().next_tab.unwrap());
        if let Event::Key { pressed, .. } = &mut release {
            *pressed = false;
        }
        present(&mut harness, 360.0, vec![release]);
        assert_selected(&harness, 6);
    }
}

#[test]
fn batched_tab_shortcuts_preserve_every_press_and_event_order() {
    for callback in [false, true] {
        for (directions, expected) in [
            (vec![true, true], 3),
            (vec![true, false], 1),
            (vec![false, true], 1),
            (vec![true; 8], 3),
        ] {
            let mut harness = Harness::tabs(1);
            harness.dockspace.set_input(bindings());
            if callback {
                harness.local_style = Some((*harness.context.global_style()).clone());
            }
            present(&mut harness, 360.0, Vec::new());
            harness.focus_tab(360.0, "Document 1");
            let events = directions
                .into_iter()
                .flat_map(|forward| {
                    let binding = if forward {
                        bindings().next_tab.unwrap()
                    } else {
                        bindings().previous_tab.unwrap()
                    };
                    let press = shortcut(binding);
                    let mut release = press.clone();
                    if let Event::Key { pressed, .. } = &mut release {
                        *pressed = false;
                    }
                    [press, release]
                })
                .collect();
            let output = present(&mut harness, 360.0, events);
            assert_selected(&harness, expected);
            assert_tab_focus(&harness, &output, expected);
        }
    }
}

#[test]
fn batched_header_keys_follow_order_and_activate_the_final_tab() {
    for callback in [false, true] {
        for (keys, expected, activate) in [
            (vec![Key::ArrowRight, Key::ArrowRight], 3, false),
            (
                vec![Key::End, Key::ArrowLeft, Key::Home, Key::ArrowRight],
                2,
                false,
            ),
            (vec![Key::ArrowRight, Key::ArrowLeft], 1, false),
            (vec![Key::End, Key::Enter], 6, true),
        ] {
            let mut harness = Harness::tabs(1);
            if callback {
                harness.local_style = Some((*harness.context.global_style()).clone());
            }
            present(&mut harness, 360.0, Vec::new());
            harness.focus_tab(360.0, "Document 1");
            let events = keys
                .into_iter()
                .flat_map(|key| {
                    let press = super::key(key);
                    let mut release = press.clone();
                    if let Event::Key { pressed, .. } = &mut release {
                        *pressed = false;
                    }
                    [press, release]
                })
                .collect();
            let output = present(&mut harness, 360.0, events);
            assert_selected(&harness, expected);
            if activate {
                assert_eq!(
                    harness.context.memory(egui::Memory::focused),
                    harness.panes.focus_target(ItemId::new(expected)),
                );
            } else {
                assert_tab_focus(&harness, &output, expected);
            }
        }
    }
}

struct HeaderActivationProbe {
    discarded_passes: usize,
    remaining: Arc<Mutex<Vec<Event>>>,
}

impl egui::plugin::Plugin for HeaderActivationProbe {
    fn debug_name(&self) -> &'static str {
        "header-activation-input-tail"
    }

    fn output_hook(&mut self, context: &Context, output: &mut FullOutput) {
        if output.platform_output.num_completed_passes == 1 {
            *self.remaining.lock().unwrap() = context.input(|input| input.events.clone());
        }
        if self.discarded_passes > 0 {
            self.discarded_passes -= 1;
            output
                .platform_output
                .request_discard_reasons
                .push(egui::RepaintCause::new_reason(
                    "retry after a header hands focus to content",
                ));
        }
    }
}

#[test]
fn header_activation_ends_ownership_before_trailing_navigation() {
    for callback in [false, true] {
        for discards in [0, 2] {
            for (keys, expected, tail) in [
                (vec![Key::Enter, Key::ArrowRight], 1, vec![Key::ArrowRight]),
                (
                    vec![Key::End, Key::Enter, Key::ArrowLeft],
                    6,
                    vec![Key::ArrowLeft],
                ),
                (
                    vec![Key::ArrowRight, Key::Space, Key::ArrowRight, Key::Home],
                    2,
                    vec![Key::ArrowRight, Key::Home],
                ),
            ] {
                let setup = || {
                    let mut harness = Harness::tabs(1);
                    if callback {
                        harness.local_style = Some((*harness.context.global_style()).clone());
                    }
                    present(&mut harness, 360.0, Vec::new());
                    harness.focus_tab(360.0, "Document 1");
                    harness
                };
                let mut separate = setup();
                for &key in &keys {
                    present(&mut separate, 360.0, vec![super::key(key)]);
                }
                assert_selected(&separate, expected);
                assert_eq!(
                    separate.context.memory(egui::Memory::focused),
                    separate.panes.focus_target(ItemId::new(expected)),
                    "separate input frames enter the content before its navigation keys"
                );

                let mut batched = setup();
                let remaining = Arc::new(Mutex::new(Vec::new()));
                batched.context.options_mut(|options| {
                    options.max_passes = (discards + 1).try_into().unwrap();
                });
                batched.context.add_plugin(HeaderActivationProbe {
                    discarded_passes: discards,
                    remaining: remaining.clone(),
                });
                let output = batched.run(360.0, keys.iter().copied().map(super::key).collect());
                assert_eq!(output.platform_output.num_completed_passes, discards + 1);
                let remaining = remaining.lock().unwrap().clone();
                let remaining_keys: Vec<_> = remaining
                    .iter()
                    .filter_map(|event| match event {
                        Event::Key {
                            key, pressed: true, ..
                        } => Some(*key),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    remaining_keys, tail,
                    "the header owns only the prefix through activation: {keys:?}, callback={callback}, discards={discards}"
                );
                present(&mut batched, 360.0, Vec::new());
                assert_selected(&batched, expected);
                assert_eq!(
                    batched.context.memory(egui::Memory::focused),
                    batched.panes.focus_target(ItemId::new(expected)),
                    "one input packet must preserve the selection and content focus of separate frames"
                );
            }
        }
    }
}

#[test]
fn header_navigation_starts_at_focused_tab_and_configured_cycle_at_selection() {
    let mut harness = Harness::tabs(1);
    harness.dockspace.set_input(bindings());
    present(&mut harness, 1100.0, Vec::new());
    harness.focus_tab(1100.0, "Document 3");
    assert_selected(&harness, 1);
    let output = present(&mut harness, 1100.0, vec![super::key(Key::ArrowRight)]);
    assert_selected(&harness, 4);
    assert_tab_focus(&harness, &output, 4);
    harness.focus_tab(1100.0, "Document 2");
    let output = present(
        &mut harness,
        1100.0,
        vec![shortcut(bindings().next_tab.unwrap())],
    );
    assert_selected(&harness, 5);
    assert_tab_focus(&harness, &output, 5);
}

#[test]
fn overlapping_bindings_choose_backward_once() {
    let mut harness = Harness::tabs(1);
    let mut configured = bindings();
    configured.previous_tab = configured.next_tab;
    harness.dockspace.set_input(configured);
    present(&mut harness, 800.0, Vec::new());
    harness.focus_tab(800.0, "Document 1");
    let output = present(
        &mut harness,
        800.0,
        vec![shortcut(configured.next_tab.unwrap())],
    );
    assert_selected(&harness, 6);
    assert_tab_focus(&harness, &output, 6);
}

#[test]
fn foreign_focus_keeps_its_shortcut_even_with_pointer_over_the_dockspace() {
    let mut harness = Harness::tabs(1);
    harness.dockspace.set_input(bindings());
    present(&mut harness, 800.0, Vec::new());
    let target = egui::Id::unique("tab-cycle-foreign-editor");
    let mut text = String::new();
    let event = shortcut(bindings().next_tab.unwrap());
    let mut remaining = false;
    for events in [
        Vec::new(),
        vec![Event::PointerMoved(Pos2::new(100.0, 200.0)), event.clone()],
    ] {
        run_with(&mut harness, 800.0, events, |ui, dockspace, panes| {
            let response = ui.add(egui::TextEdit::singleline(&mut text).id(target));
            response.request_focus();
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
            remaining |= shortcut_remains(ui, bindings().next_tab.unwrap());
        });
    }
    assert_selected(&harness, 1);
    assert_eq!(harness.context.memory(egui::Memory::focused), Some(target));
    assert!(
        remaining,
        "foreign input must remain available to its owner"
    );
}

#[test]
fn disabled_ui_leaves_the_shortcut_unconsumed() {
    let mut harness = Harness::tabs(1);
    harness.dockspace.set_input(bindings());
    present(&mut harness, 800.0, Vec::new());
    harness.focus_tab(800.0, "Document 1");
    let event = shortcut(bindings().next_tab.unwrap());
    let mut remaining = false;
    run_with(
        &mut harness,
        800.0,
        vec![event.clone()],
        |ui, dockspace, panes| {
            ui.disable();
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
            remaining |= shortcut_remains(ui, bindings().next_tab.unwrap());
        },
    );
    present(&mut harness, 800.0, Vec::new());
    assert_selected(&harness, 1);
    assert!(remaining);
}

#[test]
fn modal_and_both_egui_popup_ownership_modes_block_cycling() {
    for overlay in 0..3 {
        let mut harness = Harness::tabs(1);
        harness.dockspace.set_input(bindings());
        present(&mut harness, 800.0, Vec::new());
        harness.focus_tab(800.0, "Document 1");
        let event = shortcut(bindings().next_tab.unwrap());
        let mut remaining = false;
        for events in [Vec::new(), vec![event.clone()]] {
            run_with(&mut harness, 800.0, events, |ui, dockspace, panes| {
                let id = egui::Id::unique(("tab-cycle-overlay", overlay));
                if overlay == 0 {
                    egui::Modal::new(id).show(ui.ctx(), |ui| {
                        ui.label("Modal content");
                    });
                } else {
                    if overlay == 1 {
                        egui::Popup::open_id(ui.ctx(), id);
                    }
                    let popup = egui::Popup::new(
                        id,
                        ui.ctx().clone(),
                        Rect::from_min_size(Pos2::new(400.0, 100.0), vec2(80.0, 40.0)),
                        ui.layer_id(),
                    );
                    let popup = if overlay == 1 {
                        popup.open_memory(None)
                    } else {
                        popup
                    };
                    popup.show(|ui| {
                        ui.label("Popup content");
                    });
                }
                dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
                remaining |= shortcut_remains(ui, bindings().next_tab.unwrap());
            });
        }
        assert_selected(&harness, 1);
        assert!(remaining, "overlay mode {overlay} must retain the shortcut");
    }
}

#[test]
fn core_overflow_popup_retains_the_shortcut() {
    let (mut harness, width, _) = overflow_menu_with_first_row_focused();
    harness.dockspace.set_input(bindings());
    let event = shortcut(bindings().next_tab.unwrap());
    let mut remaining = false;
    let output = run_with(
        &mut harness,
        width,
        vec![event.clone()],
        |ui, dockspace, panes| {
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
            remaining |= shortcut_remains(ui, bindings().next_tab.unwrap());
        },
    );
    assert_selected(&harness, 2);
    assert!(
        nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Menu)
    );
    assert!(remaining);
}

#[test]
fn pane_content_can_consume_the_shortcut_before_cycling() {
    struct ConsumingPanes<'a> {
        panes: &'a mut Panes,
        consumed: &'a mut bool,
        binding: KeyboardShortcut,
    }
    impl PaneView for ConsumingPanes<'_> {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.panes.title(item)
        }

        fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
            self.panes.focus_target(item)
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
            let response = self.panes.ui(item, ui);
            if self.panes.focus_state(item, ui.ctx()) == crate::PaneFocusState::Focused {
                *self.consumed |= ui.input_mut(|input| input.consume_shortcut(&self.binding));
            }
            response
        }
    }

    for binding in [
        bindings().next_tab.unwrap(),
        bindings().previous_tab.unwrap(),
    ] {
        let mut harness = Harness::tabs(1);
        harness.dockspace.set_input(bindings());
        present(&mut harness, 800.0, Vec::new());
        harness.context.memory_mut(|memory| {
            memory.request_focus(harness.panes.focus_target(ItemId::new(1)).unwrap())
        });
        harness.run(800.0, Vec::new());
        let mut consumed = false;
        run_with(
            &mut harness,
            800.0,
            vec![shortcut(binding)],
            |ui, dockspace, panes| {
                dockspace
                    .show_single_surface(
                        SURFACE,
                        ui,
                        &mut ConsumingPanes {
                            panes,
                            consumed: &mut consumed,
                            binding,
                        },
                    )
                    .unwrap();
            },
        );
        present(&mut harness, 800.0, Vec::new());
        assert!(consumed);
        assert_selected(&harness, 1);
        assert_eq!(
            harness.context.memory(egui::Memory::focused),
            harness.panes.focus_target(ItemId::new(1))
        );
    }
}

#[test]
fn unavailable_cycling_preserves_egui_traversal_from_tab_and_content_focus() {
    for content_focus in [false, true] {
        let mut baseline = None;
        for configured in [false, true] {
            let mut harness = Harness::new(
                vec!["Only document".to_owned()],
                ItemId::new(1),
                DockStyle::default(),
            );
            if configured {
                harness.dockspace.set_input(bindings());
            }
            present(&mut harness, 800.0, Vec::new());
            if content_focus {
                harness.context.memory_mut(|memory| {
                    memory.request_focus(harness.panes.focus_target(ItemId::new(1)).unwrap())
                });
                harness.run(800.0, Vec::new());
            } else {
                harness.focus_tab(800.0, "Only document");
            }
            let before = harness.context.memory(egui::Memory::focused);
            present(
                &mut harness,
                800.0,
                vec![shortcut(bindings().previous_tab.unwrap())],
            );
            assert_selected(&harness, 1);
            let after = harness.context.memory(egui::Memory::focused);
            assert_ne!(
                before, after,
                "the unavailable action must preserve traversal"
            );
            if configured {
                assert_eq!(
                    after, baseline,
                    "configuration must preserve native traversal"
                );
            } else {
                baseline = after;
            }
        }
    }
}

#[test]
fn backward_cycling_preserves_destination_focus_when_the_surface_resizes() {
    for callback in [false, true] {
        for (before_width, after_width) in [(800.0, 360.0), (360.0, 800.0)] {
            let mut harness = Harness::tabs(1);
            harness.dockspace.set_input(bindings());
            if callback {
                harness.local_style = Some((*harness.context.global_style()).clone());
            }
            present(&mut harness, before_width, Vec::new());
            harness.context.memory_mut(|memory| {
                memory.request_focus(harness.panes.focus_target(ItemId::new(1)).unwrap())
            });
            harness.run(before_width, Vec::new());
            let output = present(
                &mut harness,
                after_width,
                vec![shortcut(bindings().previous_tab.unwrap())],
            );
            assert_selected(&harness, 6);
            assert_tab_focus(&harness, &output, 6);
        }
    }
}

#[test]
fn unrelated_tab_events_keep_native_editor_indentation_and_focus_traversal() {
    struct EditorPanes<'a> {
        panes: &'a mut Panes,
        indent: bool,
    }

    impl PaneView for EditorPanes<'_> {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.panes.title(item)
        }

        fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
            self.panes.focus_target(item)
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
            let target = self.focus_target(item).unwrap();
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.panes.text)
                    .id(target)
                    .lock_focus(self.indent),
            );
            PaneResponse::from(&response)
        }
    }

    for indent in [false, true] {
        let mut baseline = None;
        for input in [
            crate::DockInputConfig::default(),
            bindings(),
            crate::DockInputConfig {
                next_tab: Some(KeyboardShortcut::new(Modifiers::ALT, Key::F6)),
                ..Default::default()
            },
        ] {
            let mut harness = Harness::tabs(1);
            harness.dockspace.set_input(input);
            harness.panes.text = "document".to_owned();
            let mut paint = |ui: &mut Ui, dockspace: &mut Dockspace, panes: &mut Panes| {
                dockspace
                    .show_single_surface(SURFACE, ui, &mut EditorPanes { panes, indent })
                    .unwrap();
            };
            for _ in 0..3 {
                run_with(&mut harness, 800.0, Vec::new(), &mut paint);
            }
            let target = harness.panes.focus_target(ItemId::new(1)).unwrap();
            harness
                .context
                .memory_mut(|memory| memory.request_focus(target));
            run_with(&mut harness, 800.0, Vec::new(), &mut paint);
            run_with(&mut harness, 800.0, vec![key(Key::Tab)], &mut paint);
            run_with(&mut harness, 800.0, Vec::new(), &mut paint);
            assert_selected(&harness, 1);
            assert_eq!(
                harness.panes.text,
                if indent { "document\t" } else { "document" }
            );
            let focused = harness.context.memory(egui::Memory::focused);
            if indent {
                assert_eq!(focused, Some(target));
            } else {
                assert_ne!(focused, Some(target));
            }
            if let Some(baseline) = baseline {
                assert_eq!(focused, baseline);
            } else {
                baseline = Some(focused);
            }
        }
    }
}

#[test]
fn focus_loss_and_held_pointer_buttons_block_cycling() {
    for lost_focus in [false, true] {
        let mut harness = Harness::tabs(1);
        harness.dockspace.set_input(bindings());
        present(&mut harness, 800.0, Vec::new());
        harness.focus_tab(800.0, "Document 1");
        let mut events = Vec::new();
        if !lost_focus {
            events.push(Event::PointerMoved(Pos2::new(600.0, 400.0)));
            events.push(Event::PointerButton {
                pos: Pos2::new(600.0, 400.0),
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Modifiers::NONE,
            });
        }
        events.push(shortcut(bindings().next_tab.unwrap()));
        harness.frame += 1;
        harness.run_input(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(f64::from(harness.frame) * 0.1),
            focused: !lost_focus,
            events,
            ..Default::default()
        });
        assert_selected(&harness, 1);
    }
}
