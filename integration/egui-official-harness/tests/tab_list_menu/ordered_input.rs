use super::*;

struct Application {
    fixture: Fixture,
    editor: String,
    activations: usize,
    button: Option<egui::Id>,
}

impl Application {
    fn new() -> Self {
        Self {
            fixture: Fixture::new(Panes {
                editors: Some(Default::default()),
                ..Panes::default()
            }),
            editor: String::new(),
            activations: 0,
            button: None,
        }
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        let input = self.fixture.input(events, 320.0);
        let (output, _) = self
            .fixture
            .dock
            .run_single_surface_with_ui(
                &self.fixture.context,
                input,
                SURFACE,
                &mut self.fixture.panes,
                |ui| {
                    egui::Panel::top("app-tools")
                        .exact_size(44.0)
                        .frame(egui::Frame::NONE)
                        .show(ui, |ui| {
                            if ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, Key::F7)
                            }) && let Some(button) = self.button
                            {
                                ui.memory_mut(|memory| memory.request_focus(button));
                            }
                            let button = ui.button("App command");
                            self.activations += usize::from(button.clicked());
                            self.button = Some(button.id);
                            let editor = ui.add(egui::TextEdit::singleline(&mut self.editor));
                            if ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, Key::F6)
                            }) {
                                editor.request_focus();
                            }
                        });
                },
            )
            .unwrap();
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn open_menu(&mut self) -> FullOutput {
        self.run(Vec::new());
        let output = self.run(Vec::new());
        let trigger = node(&output, Role::Button, "List tabs").0;
        self.run(vec![accesskit(trigger, Action::Click)]);
        self.run(Vec::new());
        self.run(Vec::new())
    }

    fn send(&mut self, events: Vec<Event>, batched: bool) -> FullOutput {
        if batched {
            self.run(events);
        } else {
            for event in events {
                self.run(vec![event]);
            }
        }
        self.run(Vec::new());
        self.run(Vec::new())
    }

    fn target_row(&self, output: &FullOutput) -> NodeId {
        node(
            output,
            Role::MenuItem,
            &self.fixture.panes.label(ItemId::new(35)),
        )
        .0
    }
}

#[test]
fn application_shortcut_keeps_focus_after_menu_row_focus() {
    for batched in [false, true] {
        let mut app = Application::new();
        let output = app.open_menu();
        let row = app.target_row(&output);
        app.send(vec![accesskit(row, Action::Focus), key(Key::F6)], batched);
        app.run(vec![Event::Text("Q".into())]);
        assert_eq!(app.editor, "Q", "batched: {batched}");
    }
}

#[test]
fn enter_activates_only_the_focused_receiver_at_that_event() {
    for batched in [false, true] {
        for menu_first in [true, false] {
            let mut app = Application::new();
            let output = app.open_menu();
            let row = app.target_row(&output);
            let button = node(&output, Role::Button, "App command").0;
            let (receiver, later) = if menu_first {
                (row, button)
            } else {
                (button, row)
            };
            let output = app.send(
                vec![
                    accesskit(receiver, Action::Focus),
                    key(Key::Enter),
                    accesskit(later, Action::Focus),
                ],
                batched,
            );
            let expected = if menu_first {
                ItemId::new(35)
            } else {
                SELECTED
            };
            assert_eq!(
                app.activations,
                usize::from(!menu_first),
                "batched: {batched}, menu first: {menu_first}"
            );
            assert!(
                app.fixture.dock.view().item(expected).unwrap().is_active(),
                "batched: {batched}, menu first: {menu_first}"
            );
            let focus = output.platform_output.accesskit_update.unwrap().focus;
            assert_eq!(focus, later, "batched: {batched}, menu first: {menu_first}");
        }
    }
}

#[test]
fn terminal_menu_keys_leave_later_application_keys_available() {
    for terminal in [Key::Enter, Key::Escape] {
        for batched in [false, true] {
            let mut app = Application::new();
            let output = app.open_menu();
            let row = app.target_row(&output);
            let button = node(&output, Role::Button, "App command").0;
            let output = app.send(
                vec![
                    accesskit(row, Action::Focus),
                    key(terminal),
                    accesskit(button, Action::Focus),
                    key(Key::Enter),
                ],
                batched,
            );
            let expected = if terminal == Key::Enter {
                ItemId::new(35)
            } else {
                SELECTED
            };
            assert!(app.fixture.dock.view().item(expected).unwrap().is_active());
            assert_eq!(app.activations, 1, "{terminal:?}, batched: {batched}");
            assert_eq!(
                output.platform_output.accesskit_update.unwrap().focus,
                button,
                "{terminal:?}, batched: {batched}"
            );
        }
    }
}

#[test]
fn a_later_application_shortcut_can_redirect_confirmation() {
    for batched in [false, true] {
        let mut app = Application::new();
        let output = app.open_menu();
        let row = app.target_row(&output);
        let button = node(&output, Role::Button, "App command").0;
        let output = app.send(
            vec![accesskit(row, Action::Focus), key(Key::F7), key(Key::Enter)],
            batched,
        );
        assert!(app.fixture.dock.view().item(SELECTED).unwrap().is_active());
        assert_eq!(app.activations, 1, "batched: {batched}");
        assert_eq!(
            output.platform_output.accesskit_update.unwrap().focus,
            button
        );
    }
}

#[test]
fn product_menu_confirmation_keeps_its_target_before_a_later_focus_event() {
    for (opener, safe, close) in [
        ("Layout actions", "Active Panes…", "Close Layout"),
        ("Tab group actions", "Float Tab Group", "Close Tab Group"),
    ] {
        for (focus_first, batched) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut fixture = Fixture::new(Panes::default());
            fixture.dock = Dockspace::builder(
                "ordered-product-menu",
                DockspaceLayout::new([DockspaceSurfaceLayout::new(
                    SURFACE,
                    DockspaceRootLayout::new(
                        RootId::new(1),
                        DockspaceNode::central_tabs((2..=4).map(ItemId::new)),
                    ),
                )])
                .unwrap(),
            )
            .build()
            .unwrap();
            let output = fixture.settle();
            let opener = node(&output, Role::Button, opener).0;
            fixture.run(vec![accesskit(opener, Action::Click)]);
            let menu = fixture.settle();
            let safe = node(&menu, Role::MenuItem, safe).0;
            let close = node(&menu, Role::MenuItem, close).0;
            fixture.run(vec![accesskit(safe, Action::Focus)]);
            let menu = fixture.settle();
            assert_eq!(menu.platform_output.accesskit_update.unwrap().focus, safe);

            let mut events = [key(Key::Enter), accesskit(close, Action::Focus)];
            if focus_first {
                events.reverse();
            }
            let packets = if batched {
                vec![Vec::from(events)]
            } else {
                events.into_iter().map(|event| vec![event]).collect()
            };
            let mut requested = Vec::new();
            for packet in packets.into_iter().chain([Vec::new(), Vec::new()]) {
                let (_, response) = fixture.run_with_response(packet);
                for plan in response.close_requests() {
                    requested.extend(plan.items().iter().map(|item| item.item().get()));
                }
            }
            assert_eq!(
                requested,
                if focus_first { &[2, 3, 4][..] } else { &[] },
                "{close:?}, {batched}, {focus_first}"
            );
            for item in (2..=4).map(ItemId::new) {
                assert!(fixture.dock.view().item(item).is_some());
            }
        }
    }
}

#[test]
fn accepted_tab_list_opening_owns_the_next_packet_before_the_first_menu_pass() {
    let mut app = Application::new();
    app.run(Vec::new());
    let output = app.run(vec![key(Key::F7)]);
    let trigger = node(&output, Role::Button, "List tabs").0;
    app.run(vec![accesskit(trigger, Action::Click), key(Key::End)]);
    app.send(vec![key(Key::Enter)], true);
    assert_eq!(app.activations, 0);
    assert!(app.fixture.dock.view().item(LAST).unwrap().is_active());
}

#[test]
fn opening_the_list_keeps_focus_taken_by_an_application_shortcut() {
    for batched in [false, true] {
        let mut app = Application::new();
        app.run(Vec::new());
        let output = app.run(vec![key(Key::F7)]);
        let trigger = node(&output, Role::Button, "List tabs").0;
        app.send(
            vec![accesskit(trigger, Action::Click), key(Key::F6)],
            batched,
        );
        app.run(vec![Event::Text("Q".into())]);
        assert_eq!(app.editor, "Q", "batched: {batched}");
    }
}

#[test]
fn opening_keys_wait_for_a_presented_menu_even_when_the_first_output_is_dropped() {
    for disposition in [
        SurfacePresentationResult::Presented,
        SurfacePresentationResult::Dropped,
    ] {
        let mut fixture = Fixture::new(Panes::default());
        let output = fixture.settle();
        let trigger = node(&output, Role::Button, "List tabs").0;
        fixture.run(vec![
            accesskit(trigger, Action::Click),
            key(Key::End),
            key(Key::Enter),
        ]);

        let input = fixture.input(Vec::new(), 200.0);
        let (pending, _) = fixture
            .dock
            .run_single_surface(&fixture.context, input, SURFACE, &mut fixture.panes)
            .unwrap();
        let (output, pending) = pending.into_parts();
        assert!(
            nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::Menu)
        );

        let input = fixture.input(Vec::new(), 200.0);
        let (later, _) = fixture
            .dock
            .run_single_surface(&fixture.context, input, SURFACE, &mut fixture.panes)
            .unwrap();
        assert!(fixture.dock.view().item(SELECTED).unwrap().is_active());
        let (_, later) = later.into_parts();
        pending.report(disposition);
        later.report(SurfacePresentationResult::Dropped);

        let output = fixture.settle();
        assert!(fixture.dock.view().item(LAST).unwrap().is_active());
        assert!(
            nodes(&output)
                .iter()
                .all(|(_, node)| node.role() != Role::Menu)
        );
    }
}

#[test]
fn disabled_tab_list_opener_leaves_confirmation_available_to_the_application() {
    let mut fixture = Fixture::new(Panes::default());
    let mut activations = 0;
    let mut run = |events, enabled| {
        let input = fixture.input(events, 320.0);
        let (mut output, receipt) = egui_dockspace::run_ui(&fixture.context, input, |ui| {
            egui::Panel::top("app-tools").show(ui, |ui| {
                let button = ui.button("App command");
                if ui.input(|input| input.key_pressed(Key::F7)) {
                    button.request_focus();
                }
                activations += usize::from(button.clicked());
            });
            ui.add_enabled_ui(enabled, |ui| {
                fixture
                    .dock
                    .show_single_surface(SURFACE, ui, &mut fixture.panes)
                    .unwrap();
            });
        })
        .into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    };
    run(Vec::new(), true);
    let output = run(vec![key(Key::F7)], true);
    let trigger = node(&output, Role::Button, "List tabs").0;
    let output = run(
        vec![accesskit(trigger, Action::Click), key(Key::Enter)],
        false,
    );
    assert_eq!(activations, 1);
    assert!(
        nodes(&output)
            .iter()
            .all(|(_, node)| node.role() != Role::Menu)
    );
}

#[test]
fn old_grid_divider_id_expires_when_auto_grid_reflows() {
    use egui::accesskit::Orientation;
    use egui_dockspace::{DockspaceGridShape, DockspaceSurfaceLayout};

    let mut fixture = Fixture::new(Panes::default());
    let content = DockspaceNode::grid(
        DockspaceGridShape::Auto,
        (1..=6).map(|item| DockspaceNode::pane(ItemId::new(item))),
    )
    .unwrap();
    fixture.dock = Dockspace::builder(
        "reflowed-grid-boundary",
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), content),
        )])
        .unwrap(),
    )
    .style(DockStyle {
        minimum_pane_size: vec2(1.0, 1.0),
        ..DockStyle::default()
    })
    .build()
    .unwrap();
    let mut render = |width, height, events| {
        let mut input = fixture.input(events, height);
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height)));
        let (output, _) = fixture
            .dock
            .run_single_surface(&fixture.context, input, SURFACE, &mut fixture.panes)
            .unwrap();
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    };
    let divider = |output: &FullOutput| {
        nodes(output)
            .iter()
            .filter(|(_, node)| {
                node.role() == Role::Splitter
                    && node.orientation() == Some(Orientation::Horizontal)
                    && node.bounds().is_some()
            })
            .min_by(|(_, left), (_, right)| {
                left.bounds()
                    .unwrap()
                    .y0
                    .total_cmp(&right.bounds().unwrap().y0)
            })
            .map(|(id, node)| (*id, node.bounds().unwrap().y0))
            .unwrap()
    };
    render(320.0, 250.0, Vec::new());
    let (old, _) = divider(&render(320.0, 250.0, Vec::new()));
    render(220.0, 340.0, Vec::new());
    let (current, y) = divider(&render(220.0, 340.0, Vec::new()));
    assert_ne!(old, current);
    render(220.0, 340.0, vec![accesskit(old, Action::Increment)]);
    assert_eq!(divider(&render(220.0, 340.0, Vec::new())), (current, y));
    render(220.0, 340.0, vec![accesskit(current, Action::Increment)]);
    assert!(divider(&render(220.0, 340.0, Vec::new())).1 > y);
}

#[test]
fn wheel_events_follow_their_own_pointer_positions_between_dockspaces() {
    for bare in [false, true] {
        for wheel_first in [false, true] {
            for batched in [false, true] {
                let context = Context::default();
                context.enable_accesskit();
                let mut docks = ["left-dockspace", "right-dockspace"].map(make_named_dock);
                let mut panes = ["Left", "Right"].map(|prefix| Panes {
                    title_prefix: Some(prefix),
                    ..Panes::default()
                });
                let mut tick = 0;
                let mut run = |events| {
                    tick += 1;
                    let input = RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 260.0))),
                        time: Some(f64::from(tick) * 0.1),
                        events,
                        ..Default::default()
                    };
                    let mut render = |ui: &mut Ui| {
                        ui.columns(2, |columns| {
                            for ((dock, panes), column) in
                                docks.iter_mut().zip(&mut panes).zip(columns)
                            {
                                column.shrink_clip_rect(column.available_rect_before_wrap());
                                dock.show_single_surface(SURFACE, column, panes).unwrap();
                            }
                        })
                    };
                    let mut output = if bare {
                        context.run_ui(input, &mut render)
                    } else {
                        let (output, receipt) =
                            egui_dockspace::run_ui(&context, input, &mut render).into_parts();
                        receipt.report(SurfacePresentationResult::Presented);
                        output
                    };
                    output.textures_delta.clear();
                    output
                };
                run(Vec::new());
                let output = run(Vec::new());
                for (id, _) in nodes(&output).iter().filter(|(_, node)| {
                    node.role() == Role::Button && node.label() == Some("List tabs")
                }) {
                    run(vec![accesskit(*id, Action::Click)]);
                    run(Vec::new());
                }
                let output = run(Vec::new());
                let [left, right] = ["Left", "Right"].map(|side| {
                    let label = format!("{side} document name with a unique suffix 35");
                    let bounds = node(&output, Role::MenuItem, &label).1.bounds().unwrap();
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) * 0.5) as f32,
                        ((bounds.y0 + bounds.y1) * 0.5) as f32,
                    )
                });
                let before = menu_offsets(&output);
                run(vec![Event::PointerMoved(left)]);
                run(Vec::new());
                run(Vec::new());
                let wheel = Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    source: egui::MouseWheelSource::Wheel,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, 4.0),
                    modifiers: egui::Modifiers::NONE,
                };
                let movement = Event::PointerMoved(right);
                let events = if wheel_first {
                    [wheel, movement]
                } else {
                    [movement, wheel]
                };
                if batched {
                    run(Vec::from(events));
                } else {
                    for event in events {
                        run(vec![event]);
                    }
                }
                run(Vec::new());
                let after = menu_offsets(&run(Vec::new()));
                let target = usize::from(!wheel_first);
                let mut expected = before;
                expected[target] -= 4.0;
                assert_eq!(
                    after, expected,
                    "bare: {bare}, batched: {batched}, wheel first: {wheel_first}"
                );

                // A new sample on the other menu must not inherit smoothing
                // from an earlier, larger sample.
                run(vec![Event::PointerMoved(left)]);
                let wheel = |delta| Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    source: egui::MouseWheelSource::Wheel,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, delta),
                    modifiers: egui::Modifiers::NONE,
                };
                run(vec![wheel(120.0)]);
                run(vec![Event::PointerMoved(right), wheel(4.0)]);
                for _ in 0..8 {
                    run(Vec::new());
                }
                let settled = menu_offsets(&run(Vec::new()));
                assert!(
                    (after[0] - settled[0] - 120.0).abs() < 0.001,
                    "{after:?} -> {settled:?}"
                );
                assert!(
                    (after[1] - settled[1] - 4.0).abs() < 0.001,
                    "{after:?} -> {settled:?}"
                );
            }
        }
    }
}

fn menu_offsets(output: &FullOutput) -> [f64; 2] {
    let mut bars = nodes(output)
        .iter()
        .filter_map(|(_, node)| {
            (node.role() == Role::ScrollBar && node.label() == Some("Open tabs scroll position"))
                .then(|| (node.bounds().unwrap().x0, node.numeric_value().unwrap()))
        })
        .collect::<Vec<_>>();
    bars.sort_by(|left, right| left.0.total_cmp(&right.0));
    let [left, right] = bars.try_into().expect("one menu per Dockspace");
    [left.1, right.1]
}
