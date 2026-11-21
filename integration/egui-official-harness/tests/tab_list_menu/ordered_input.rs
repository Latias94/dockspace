use super::*;

struct Application {
    fixture: Fixture,
    editor: String,
    activations: usize,
    button: Option<egui::Id>,
    closed: Vec<ItemId>,
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
            closed: Vec::new(),
        }
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        let input = self.fixture.input(events, 320.0);
        let (output, response) = self
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
                            ui.horizontal(|ui| {
                                if ui.button("Focus app command").clicked()
                                    && let Some(button) = self.button
                                {
                                    ui.memory_mut(|memory| memory.request_focus(button));
                                }
                                let button = ui.button("App command");
                                self.activations += usize::from(button.clicked());
                                self.button = Some(button.id);
                            });
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
        for request in response.close_requests() {
            for item in request.items() {
                self.closed.push(item.item());
                let _ = self
                    .fixture
                    .dock
                    .resolve_close(
                        request.request(),
                        item.token(),
                        egui_dockspace::CloseDecision::Allow,
                    )
                    .unwrap();
            }
        }
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
        let packets = if batched {
            vec![events]
        } else {
            events.into_iter().map(|event| vec![event]).collect()
        };
        let mut output = FullOutput::default();
        for packet in packets.into_iter().chain([Vec::new(), Vec::new()]) {
            output = self.run(packet);
            if !self.closed.is_empty() {
                break;
            }
        }
        output
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
fn application_click_hands_confirmation_off_from_a_product_menu() {
    for (opener, close) in [
        ("Layout actions", "Close Layout"),
        ("Tab group actions", "Close Tab Group"),
    ] {
        for batched in [false, true] {
            let mut app = Application::new();
            app.fixture.dock = Dockspace::builder(
                "product-menu-app-handoff",
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
            app.run(Vec::new());
            let output = app.run(Vec::new());
            let opener = node(&output, Role::Button, opener).0;
            app.send(vec![accesskit(opener, Action::Click)], true);
            let output = app.run(Vec::new());
            let close = node(&output, Role::MenuItem, close).0;
            let transfer = node(&output, Role::Button, "Focus app command").0;
            let confirm = node(&output, Role::Button, "App command").0;
            let output = app.send(
                vec![
                    accesskit(close, Action::Focus),
                    accesskit(transfer, Action::Click),
                    key(Key::Enter),
                ],
                batched,
            );
            assert_eq!(app.closed, [], "batched: {batched}");
            assert_eq!(app.activations, 1, "batched: {batched}");
            assert_eq!(
                output.platform_output.accesskit_update.unwrap().focus,
                confirm
            );
            assert!((2..=4).all(|item| app.fixture.dock.view().item(ItemId::new(item)).is_some()));
        }
    }
}

#[test]
fn tab_list_leaves_confirmation_for_an_application_drawn_after_docking() {
    for batched in [false, true] {
        let mut fixture = Fixture::new(Panes::default());
        let mut command = None;
        let mut activations = 0;
        let mut run = |events| {
            let mut input = fixture.input(events, 260.0);
            input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1_200.0, 260.0)));
            let (mut output, receipt) = egui_dockspace::run_ui(&fixture.context, input, |ui| {
                ui.columns(2, |columns| {
                    let [dock, application] = columns else {
                        unreachable!()
                    };
                    dock.shrink_clip_rect(dock.available_rect_before_wrap());
                    fixture
                        .dock
                        .show_single_surface(SURFACE, dock, &mut fixture.panes)
                        .unwrap();
                    if application.button("Focus app command").clicked()
                        && let Some(command) = command
                    {
                        application.memory_mut(|memory| memory.request_focus(command));
                    }
                    let button = application.button("App command");
                    activations += usize::from(button.clicked());
                    command = Some(button.id);
                });
            })
            .into_parts();
            receipt.report(SurfacePresentationResult::Presented);
            output.textures_delta.clear();
            output
        };

        run(Vec::new());
        let output = run(Vec::new());
        let opener = node(&output, Role::Button, "List tabs").0;
        run(vec![accesskit(opener, Action::Click)]);
        run(Vec::new());
        let output = run(Vec::new());
        let row = node(
            &output,
            Role::MenuItem,
            &Panes::default().label(ItemId::new(35)),
        )
        .0;
        let transfer = node(&output, Role::Button, "Focus app command").0;
        let confirm = node(&output, Role::Button, "App command").0;
        run(vec![accesskit(row, Action::Focus)]);
        let output = run(Vec::new());
        assert_eq!(output.platform_output.accesskit_update.unwrap().focus, row);

        let click = accesskit(transfer, Action::Click);
        if batched {
            run(vec![click, key(Key::Enter)]);
        } else {
            run(vec![click]);
            run(vec![key(Key::Enter)]);
        }
        let output = run(Vec::new());
        assert_eq!(activations, 1, "batched: {batched}");
        assert!(fixture.dock.view().item(SELECTED).unwrap().is_active());
        assert_eq!(
            output.platform_output.accesskit_update.unwrap().focus,
            confirm
        );
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
fn a_stale_product_menu_click_does_not_interrupt_current_keyboard_input() {
    for batched in [false, true] {
        let mut fixture = Fixture::new(Panes::default());
        let output = fixture.settle();
        let opener = node(&output, Role::Button, "Layout actions").0;
        fixture.run(vec![accesskit(opener, Action::Click)]);
        let output = fixture.settle();
        let stale_close = node(&output, Role::MenuItem, "Close Layout").0;

        fixture.run(vec![accesskit(opener, Action::Click)]);
        fixture.settle();
        fixture.run(vec![accesskit(opener, Action::Click)]);
        let output = fixture.settle();
        let active_panes = node(&output, Role::MenuItem, "Active Panes…").0;
        assert_ne!(stale_close, node(&output, Role::MenuItem, "Close Layout").0);
        fixture.run(vec![accesskit(active_panes, Action::Focus)]);
        let output = fixture.settle();
        assert_eq!(
            output.platform_output.accesskit_update.unwrap().focus,
            active_panes
        );

        let click = accesskit(stale_close, Action::Click);
        if batched {
            let (_, response) = fixture.run_with_response(vec![click, key(Key::Enter)]);
            assert_eq!(response.close_requests().count(), 0);
        } else {
            let (_, response) = fixture.run_with_response(vec![click]);
            assert_eq!(response.close_requests().count(), 0);
            let (_, response) = fixture.run_with_response(vec![key(Key::Enter)]);
            assert_eq!(response.close_requests().count(), 0);
        }
        let output = fixture.settle();
        assert!(
            nodes(&output).iter().any(|(_, node)| {
                node.role() == Role::Menu && node.label() == Some("Active panes")
            }),
            "batched: {batched}"
        );
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
    #[derive(Clone, Copy, Debug)]
    enum Click {
        AccessibleRow,
        VisibleRow,
        Backdrop,
    }

    for (disposition, target) in [
        (SurfacePresentationResult::Presented, Click::AccessibleRow),
        (SurfacePresentationResult::Presented, Click::VisibleRow),
        (SurfacePresentationResult::Presented, Click::Backdrop),
        (SurfacePresentationResult::Dropped, Click::AccessibleRow),
        (SurfacePresentationResult::Dropped, Click::Backdrop),
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

        let (row, node) = node(
            &output,
            Role::MenuItem,
            &fixture.panes.label(ItemId::new(35)),
        );
        let pointer_click = |point| {
            vec![
                Event::PointerMoved(point),
                pointer_button(point, true),
                pointer_button(point, false),
            ]
        };
        let click = match target {
            Click::AccessibleRow => vec![accesskit(*row, Action::Click)],
            Click::VisibleRow => {
                let bounds = node.bounds().unwrap();
                pointer_click(egui::pos2(
                    ((bounds.x0 + bounds.x1) * 0.5) as f32,
                    ((bounds.y0 + bounds.y1) * 0.5) as f32,
                ))
            }
            Click::Backdrop => pointer_click(egui::pos2(10.0, 50.0)),
        };
        if disposition == SurfacePresentationResult::Presented {
            pending.report(disposition);
            fixture.run(click);
        } else {
            for events in [click, Vec::new()] {
                let input = fixture.input(events, 200.0);
                let (later, _) = fixture
                    .dock
                    .run_single_surface(&fixture.context, input, SURFACE, &mut fixture.panes)
                    .unwrap();
                let (_, later) = later.into_parts();
                later.report(SurfacePresentationResult::Dropped);
                assert!(fixture.dock.view().item(SELECTED).unwrap().is_active());
            }
            pending.report(disposition);
        }

        let output = fixture.settle();
        assert!(
            fixture.dock.view().item(LAST).unwrap().is_active(),
            "{disposition:?}, {target:?}"
        );
        assert!(
            nodes(&output)
                .iter()
                .all(|(_, node)| node.role() != Role::Menu)
        );
    }
}

#[test]
fn a_row_can_receive_focus_and_confirmation_after_its_first_presentation() {
    for batched in [false, true] {
        let mut fixture = Fixture::new(Panes::default());
        let output = fixture.settle();
        let opener = node(&output, Role::Button, "List tabs").0;
        fixture.run(vec![accesskit(opener, Action::Click)]);
        let first = fixture.run(Vec::new());
        let target = ItemId::new(35);
        let row = node(&first, Role::MenuItem, &fixture.panes.label(target)).0;
        let focus = accesskit(row, Action::Focus);
        if batched {
            fixture.run(vec![focus, key(Key::Enter)]);
        } else {
            fixture.run(vec![focus]);
            fixture.run(vec![key(Key::Enter)]);
        }
        fixture.settle();
        assert!(
            fixture.dock.view().item(target).unwrap().is_active(),
            "batched: {batched}"
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
fn accessible_splitter_follows_the_same_pair_after_an_earlier_pane_closes() {
    use egui::accesskit::Orientation;
    use egui_dockspace::{CloseDecision, DockspaceAxis, DockspaceCloseRequestStatus};

    let mut fixture = Fixture::new(Panes::default());
    let layout = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        (1..=5).map(|item| DockspaceNode::pane(ItemId::new(item))),
    )
    .unwrap();
    fixture.dock = Dockspace::builder(
        "accessible-split-pair",
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), layout),
        )])
        .unwrap(),
    )
    .build()
    .unwrap();
    fn render(fixture: &mut Fixture, events: Vec<Event>) -> FullOutput {
        let input = fixture.input(events, 640.0);
        let (output, _) = fixture
            .dock
            .run_single_surface(&fixture.context, input, SURFACE, &mut fixture.panes)
            .unwrap();
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }
    fn dividers(output: &FullOutput) -> Vec<(NodeId, f64)> {
        let mut dividers: Vec<_> = nodes(output)
            .iter()
            .filter(|(_, node)| {
                node.role() == Role::Splitter
                    && node.orientation() == Some(Orientation::Horizontal)
                    && node.supports_action(Action::Increment)
            })
            .map(|(id, node)| (*id, node.bounds().unwrap().y0))
            .collect();
        dividers.sort_by(|left, right| left.1.total_cmp(&right.1));
        dividers
    }

    render(&mut fixture, Vec::new());
    let original = dividers(&render(&mut fixture, Vec::new()));
    let id = original[1].0;
    let close = fixture
        .dock
        .request_close_item_current(ItemId::new(1))
        .unwrap();
    let DockspaceCloseRequestStatus::Requested(close) = close.status() else {
        panic!("first pane did not receive a close request");
    };
    let plan = close.plan();
    let _ = fixture
        .dock
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Allow,
        )
        .unwrap();
    render(&mut fixture, Vec::new());
    let before = dividers(&render(&mut fixture, Vec::new()));
    assert_eq!(id, before[0].0);
    render(&mut fixture, vec![accesskit(id, Action::Increment)]);
    render(&mut fixture, Vec::new());
    let after = dividers(&render(&mut fixture, Vec::new()));
    assert!(after[0].1 > before[0].1);
    assert_eq!(after[1..], before[1..]);
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

#[test]
fn application_scroll_area_cannot_consume_the_menus_wheel_tail() {
    for (bare, app_first) in [(false, true), (false, false), (true, true), (true, false)] {
        for (retry, document_wheel) in [
            (false, 4.0),
            (true, 4.0),
            (false, 60.0),
            (false, -60.0),
            (true, -60.0),
            (false, -120.0),
        ] {
            let mut fixture = Fixture::new(Panes::default());
            fixture
                .context
                .options_mut(|options| options.max_passes = 2.try_into().unwrap());
            let mut document = (Pos2::ZERO, 300.0);
            let mut run = |events, seed, discard, document: &mut (Pos2, f32)| {
                let mut input = fixture.input(events, 260.0);
                input.time = Some(f64::from(fixture.tick) / 60.0);
                input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 260.0)));
                let mut render = |ui: &mut Ui| {
                    ui.columns(2, |columns| {
                        let [dock, column] = columns else {
                            unreachable!()
                        };
                        let mut paint_document = || {
                            let area = egui::ScrollArea::vertical()
                                .id_salt("document")
                                .auto_shrink([false; 2]);
                            let area = if seed {
                                area.vertical_scroll_offset(300.0)
                            } else {
                                area
                            };
                            let area = area.show(column, |ui| {
                                for line in 0..120 {
                                    ui.label(format!("Paragraph {line}"));
                                }
                            });
                            *document = (area.inner_rect.center(), area.state.clamped_offset().y);
                        };
                        if app_first {
                            paint_document();
                        }
                        dock.shrink_clip_rect(dock.available_rect_before_wrap());
                        fixture
                            .dock
                            .show_single_surface(SURFACE, dock, &mut fixture.panes)
                            .unwrap();
                        if !app_first {
                            paint_document();
                        }
                    });
                    if discard && ui.ctx().current_pass_index() == 0 {
                        ui.ctx().request_discard("document layout changed");
                    }
                };
                let mut output = if bare {
                    fixture.context.run_ui(input, &mut render)
                } else {
                    let (output, receipt) =
                        egui_dockspace::run_ui(&fixture.context, input, &mut render).into_parts();
                    receipt.report(SurfacePresentationResult::Presented);
                    output
                };
                output.textures_delta.clear();
                output
            };
            run(Vec::new(), true, false, &mut document);
            let output = run(Vec::new(), true, false, &mut document);
            let opener = node(&output, Role::Button, "List tabs").0;
            run(
                vec![accesskit(opener, Action::Click)],
                false,
                false,
                &mut document,
            );
            run(Vec::new(), false, false, &mut document);
            let output = run(Vec::new(), false, false, &mut document);
            let label = Panes::default().label(ItemId::new(35));
            let bounds = node(&output, Role::MenuItem, &label).1.bounds().unwrap();
            let menu_point = egui::pos2(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            );
            let menu_offset = |output: &FullOutput| {
                node(output, Role::ScrollBar, "Open tabs scroll position")
                    .1
                    .numeric_value()
                    .unwrap()
            };
            let before = run(
                vec![Event::PointerMoved(menu_point)],
                false,
                false,
                &mut document,
            );
            let (document_point, document_offset) = document;
            let wheel = |delta| Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                source: egui::MouseWheelSource::Wheel,
                phase: egui::TouchPhase::Move,
                delta: vec2(0.0, delta),
                modifiers: egui::Modifiers::NONE,
            };
            run(vec![wheel(120.0)], false, false, &mut document);
            let output = run(
                vec![Event::PointerMoved(document_point), wheel(document_wheel)],
                false,
                retry,
                &mut document,
            );
            assert_eq!(
                output.platform_output.num_completed_passes,
                1 + usize::from(retry)
            );
            let mut output = output;
            for _ in 0..36 {
                output = run(Vec::new(), false, false, &mut document);
            }
            let menu_delta = menu_offset(&before) - menu_offset(&output);
            let app_delta = document_offset - document.1;
            assert!(
                (menu_delta - 120.0).abs() < 0.001 && (app_delta - document_wheel).abs() < 0.001,
                "bare: {bare}, app first: {app_first}, retry: {retry}, menu: {menu_delta}, document: {app_delta}, wheel: {document_wheel}"
            );
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
