//! Real group menus retain scope through pointer, keyboard and accessibility input.

use super::*;

mod page_focus;

const WIDTH: f32 = 900.0;
const TOOLS_DESTINATION: &str = "Tools\nInspector, Console · Selected: Inspector";

fn setup() -> Harness {
    setup_with_central(false)
}

fn setup_with_central(central: bool) -> Harness {
    let mut harness = Harness::new(
        ["Editor", "Preview", "Inspector", "Console"]
            .map(str::to_owned)
            .to_vec(),
        ItemId::new(1),
        DockStyle::default(),
    );
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [
                    if central {
                        DockspaceNode::central_tabs([ItemId::new(1), ItemId::new(2)])
                    } else {
                        DockspaceNode::tabs([ItemId::new(1), ItemId::new(2)])
                    }
                    .named("Documents"),
                    DockspaceNode::tabs([ItemId::new(3), ItemId::new(4)]).named("Tools"),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("group-menu-regression", layout)
        .build()
        .unwrap();
    settle(&mut harness);
    harness
}

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

fn open_first_group(harness: &mut Harness) -> FullOutput {
    let output = settle(harness);
    let groups = nodes(&output)
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Button && node.label() == Some("Tab group actions")
        })
        .collect::<Vec<_>>();
    assert_eq!(groups.len(), 2);
    let anchor = groups
        .into_iter()
        .min_by(|(_, a), (_, b)| a.bounds().unwrap().x0.total_cmp(&b.bounds().unwrap().x0))
        .unwrap()
        .0;
    harness.run(WIDTH, vec![action(anchor, egui::accesskit::Action::Click)]);
    settle(harness)
}

fn entry(output: &FullOutput, label: &str) -> NodeId {
    nodes(output)
        .iter()
        .find(|(_, node)| {
            node.role() == Role::MenuItem && node.label() == Some(label) && !node.is_disabled()
        })
        .unwrap_or_else(|| panic!("missing enabled {label}"))
        .0
}

#[derive(Clone, Copy, Debug)]
enum DestinationInput {
    Accessibility,
    Keyboard,
    Touch,
}

#[allow(clippy::cast_possible_truncation)]
fn activate_destination_entry(
    harness: &mut Harness,
    output: &FullOutput,
    target: NodeId,
    input: DestinationInput,
    keyboard_key: Key,
) {
    match input {
        DestinationInput::Accessibility => {
            harness.run(WIDTH, vec![action(target, egui::accesskit::Action::Click)]);
        }
        DestinationInput::Keyboard => {
            harness.run(WIDTH, vec![action(target, egui::accesskit::Action::Focus)]);
            harness.run(WIDTH, vec![key(keyboard_key)]);
        }
        DestinationInput::Touch => {
            let bounds = nodes(output)
                .iter()
                .find(|(id, _)| *id == target)
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) * 0.5) as f32,
                ((bounds.y0 + bounds.y1) * 0.5) as f32,
            );
            for (phase, pressed) in [
                (egui::TouchPhase::Start, true),
                (egui::TouchPhase::End, false),
            ] {
                harness.run(
                    WIDTH,
                    vec![
                        Event::Touch {
                            device_id: egui::TouchDeviceId(1),
                            id: egui::TouchId(1),
                            phase,
                            pos,
                            force: None,
                        },
                        Event::PointerMoved(pos),
                        Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
    }
}

#[test]
fn group_menu_float_moves_only_that_group() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem
                && node.label() == Some("Move Tab Group to New Window"))
    );
    let target = entry(&output, "Float Tab Group");
    harness.run(WIDTH, vec![action(target, egui::accesskit::Action::Click)]);
    settle(&mut harness);
    let view = harness.dockspace.view();
    let first = view.item(ItemId::new(1)).unwrap();
    assert!(first.contained().is_some());
    assert_eq!(
        first
            .tabs()
            .expect("item is an inline tab")
            .items()
            .collect::<Vec<_>>(),
        &[ItemId::new(1), ItemId::new(2)]
    );
    assert!(view.item(ItemId::new(3)).unwrap().contained().is_none());
    assert!(view.item(ItemId::new(4)).unwrap().contained().is_none());
}

#[test]
fn group_menu_center_modes_use_group_titles_and_preserve_their_distinct_payloads() {
    for (label, preserve) in [
        ("Merge Tab Group Into…", false),
        ("Dock Tab Group as Layout Tab…", true),
    ] {
        let mut harness = setup();
        let source = harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .tabs()
            .unwrap()
            .key();
        let output = open_first_group(&mut harness);
        let mode = entry(&output, label);
        harness.run(WIDTH, vec![action(mode, egui::accesskit::Action::Click)]);
        let output = settle(&mut harness);
        assert!(
            nodes(&output)
                .iter()
                .any(|(_, node)| { node.role() == Role::Menu && node.label() == Some(label) })
        );
        let destination = entry(&output, TOOLS_DESTINATION);
        assert!(
            !nodes(&output).iter().any(|(_, node)| {
                node.role() == Role::MenuItem && node.label() == Some("Inspector")
            }),
            "destination identity belongs to the group, independently of its selected pane"
        );
        harness.run(
            WIDTH,
            vec![action(destination, egui::accesskit::Action::Click)],
        );
        let output = settle(&mut harness);
        let view = harness.dockspace.view();
        let moved = view.item(ItemId::new(1)).unwrap().tabs().unwrap();
        let destination = view.item(ItemId::new(3)).unwrap().tabs().unwrap();
        if preserve {
            assert_eq!(moved.key(), source);
            assert_ne!(moved.key(), destination.key());
            assert_eq!(destination.tabs().len(), 3);
            assert!(destination.selected().unwrap().is_layout());
            assert!(nodes(&output).iter().any(|(_, node)| {
                node.role() == Role::Tab && node.label() == Some("Documents")
            }));
        } else {
            assert_eq!(moved.key(), destination.key());
            assert_eq!(
                destination.items().collect::<Vec<_>>(),
                [
                    ItemId::new(3),
                    ItemId::new(4),
                    ItemId::new(1),
                    ItemId::new(2)
                ]
            );
        }
        assert!(
            !nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::MenuItem)
        );
    }
}

#[test]
fn unnamed_group_destinations_identify_direct_tabs_and_selection_for_every_menu_input() {
    let left_label = "Tabs\nEditor, Preview · Selected: Preview";
    let right_label = "Tabs\nConsole, Diagnostics · Selected: Console";
    for input in [
        DestinationInput::Accessibility,
        DestinationInput::Keyboard,
        DestinationInput::Touch,
    ] {
        for (mode_label, preserve) in [
            ("Merge Tab Group Into…", false),
            ("Dock Tab Group as Layout Tab…", true),
        ] {
            let mut harness = Harness::new(
                [
                    "Source A",
                    "Source B",
                    "Editor",
                    "Preview",
                    "Console",
                    "Diagnostics",
                ]
                .map(str::to_owned)
                .to_vec(),
                ItemId::new(1),
                DockStyle::default(),
            );
            let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(
                    RootId::new(1),
                    DockspaceNode::equal_split(
                        crate::DockspaceAxis::Horizontal,
                        [
                            DockspaceNode::tabs([ItemId::new(1), ItemId::new(2)]).named("Source"),
                            DockspaceNode::tabs_with_selection(
                                [ItemId::new(3), ItemId::new(4)],
                                Some(ItemId::new(4)),
                            )
                            .unwrap(),
                            DockspaceNode::tabs([ItemId::new(5), ItemId::new(6)]),
                        ],
                    )
                    .unwrap(),
                ),
            )])
            .unwrap();
            harness.dockspace = Dockspace::builder("unnamed-group-destinations", layout)
                .build()
                .unwrap();
            let output = settle(&mut harness);
            let source = harness
                .dockspace
                .view()
                .item(ItemId::new(1))
                .unwrap()
                .tabs()
                .unwrap()
                .key();
            let destination = harness
                .dockspace
                .view()
                .item(ItemId::new(5))
                .unwrap()
                .tabs()
                .unwrap()
                .key();
            let groups = nodes(&output)
                .iter()
                .filter(|(_, node)| {
                    node.role() == Role::Button && node.label() == Some("Tab group actions")
                })
                .collect::<Vec<_>>();
            assert_eq!(groups.len(), 3);
            let anchor = groups
                .into_iter()
                .min_by(|(_, left), (_, right)| {
                    left.bounds()
                        .unwrap()
                        .x0
                        .total_cmp(&right.bounds().unwrap().x0)
                })
                .unwrap()
                .0;
            harness.run(WIDTH, vec![action(anchor, egui::accesskit::Action::Click)]);
            let output = settle(&mut harness);
            let mode = entry(&output, mode_label);
            activate_destination_entry(&mut harness, &output, mode, input, Key::ArrowRight);
            let output = settle(&mut harness);
            for label in [left_label, right_label] {
                let target = entry(&output, label);
                let node = &nodes(&output)
                    .iter()
                    .find(|(id, _)| *id == target)
                    .unwrap()
                    .1;
                assert!(node.supports_action(egui::accesskit::Action::Click));
                assert!(node.supports_action(egui::accesskit::Action::Focus));
                assert!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.text() == label
                    )),
                    "visible and accessible destination names agree: {input:?}, {label}",
                );
            }
            let target = entry(&output, right_label);
            activate_destination_entry(&mut harness, &output, target, input, Key::Enter);
            settle(&mut harness);
            let view = harness.dockspace.view();
            let moved = view.item(ItemId::new(1)).unwrap().tabs().unwrap();
            let target = view.item(ItemId::new(5)).unwrap().tabs().unwrap();
            assert_eq!(target.key(), destination, "{input:?}, {mode_label}");
            let left = view.item(ItemId::new(3)).unwrap().tabs().unwrap();
            assert_eq!(
                left.items().collect::<Vec<_>>(),
                [ItemId::new(3), ItemId::new(4)]
            );
            assert_eq!(left.selected(), Some(ItemId::new(4).into()));
            if preserve {
                assert_eq!(moved.key(), source);
                assert_eq!(target.tabs().len(), 3);
                assert!(target.selected().unwrap().is_layout());
            } else {
                assert_eq!(moved.key(), target.key());
                assert_eq!(
                    target.items().collect::<Vec<_>>(),
                    [
                        ItemId::new(5),
                        ItemId::new(6),
                        ItemId::new(1),
                        ItemId::new(2)
                    ]
                );
            }
        }
    }
}

#[test]
fn group_destination_hints_bound_long_titles_and_keep_selected_layout_scope() {
    let mut harness = Harness::new(
        vec![
            "Source".to_owned(),
            "文".repeat(80),
            "Second".to_owned(),
            "Third".to_owned(),
            "Nested child A".to_owned(),
            "Nested child B".to_owned(),
        ],
        ItemId::new(1),
        DockStyle::default(),
    );
    let nested = DockspaceNode::equal_split(
        crate::DockspaceAxis::Horizontal,
        [
            DockspaceNode::pane(ItemId::new(5)),
            DockspaceNode::pane(ItemId::new(6)),
        ],
    )
    .unwrap()
    .named("Nested");
    let destination = DockspaceNode::composed_tabs_with_selection(
        [
            DockspaceNode::pane(ItemId::new(2)),
            DockspaceNode::pane(ItemId::new(3)),
            DockspaceNode::pane(ItemId::new(4)),
            nested,
        ],
        Some(3),
    )
    .unwrap()
    .named("Workspace");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [
                    DockspaceNode::tabs([ItemId::new(1)]).named("Source"),
                    destination,
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("bounded-group-destination-hint", layout)
        .build()
        .unwrap();
    let output = open_first_group(&mut harness);
    let mode = entry(&output, "Merge Tab Group Into…");
    harness.run(WIDTH, vec![action(mode, egui::accesskit::Action::Click)]);
    let output = settle(&mut harness);
    let label = format!(
        "Workspace\n{}…, Second, Third, +1 more · Selected: Nested",
        "文".repeat(24),
    );
    let target = entry(&output, &label);
    let node = &nodes(&output)
        .iter()
        .find(|(id, _)| *id == target)
        .unwrap()
        .1;
    let bounds = node.bounds().unwrap();
    assert!(bounds.x0 >= 0.0 && bounds.x1 <= f64::from(WIDTH));
    assert!(label.chars().count() < 128);
    assert!(!node.label().unwrap().contains("Nested child"));
}

#[test]
fn central_group_can_merge_its_contents_but_cannot_move_its_protected_container() {
    let mut harness = setup_with_central(true);
    harness.panes.customize_menu = Some(|menu| {
        assert!(matches!(
            menu.context().target,
            crate::DockMenuTarget::TabGroup(_)
        ));
        for item in &mut menu.items {
            item.enabled = true;
        }
    });
    let central = harness
        .dockspace
        .view()
        .root(RootId::new(1))
        .unwrap()
        .central()
        .unwrap()
        .key();
    let output = open_first_group(&mut harness);
    let merge = entry(&output, "Merge Tab Group Into…");
    let preserve = nodes(&output)
        .iter()
        .find(|(_, node)| {
            node.role() == Role::MenuItem && node.label() == Some("Dock Tab Group as Layout Tab…")
        })
        .unwrap();
    assert!(preserve.1.is_disabled());
    assert!(!preserve.1.supports_action(egui::accesskit::Action::Click));
    harness.run(WIDTH, vec![action(merge, egui::accesskit::Action::Click)]);
    let output = settle(&mut harness);
    let target = entry(&output, TOOLS_DESTINATION);
    harness.run(WIDTH, vec![action(target, egui::accesskit::Action::Click)]);
    settle(&mut harness);
    let view = harness.dockspace.view();
    let retained = view.root(RootId::new(1)).unwrap().central().unwrap();
    assert_eq!(retained.key(), central);
    assert!(retained.tabs().unwrap().tabs().is_empty());
    assert_eq!(
        view.item(ItemId::new(3))
            .unwrap()
            .tabs()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [
            ItemId::new(3),
            ItemId::new(4),
            ItemId::new(1),
            ItemId::new(2)
        ]
    );
}

#[test]
fn keyboard_enters_and_returns_from_group_center_destinations() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    let mode = entry(&output, "Dock Tab Group as Layout Tab…");
    harness.run(WIDTH, vec![action(mode, egui::accesskit::Action::Focus)]);
    harness.run(WIDTH, vec![key(Key::ArrowRight)]);
    let output = settle(&mut harness);
    let target = entry(&output, TOOLS_DESTINATION);
    assert_eq!(
        harness
            .context
            .memory(egui::Memory::focused)
            .unwrap()
            .value(),
        target.0
    );
    harness.run(WIDTH, vec![key(Key::ArrowLeft)]);
    let output = settle(&mut harness);
    entry(&output, "Merge Tab Group Into…");
    assert!(!nodes(&output).iter().any(|(_, node)| {
        node.role() == Role::MenuItem && node.label() == Some(TOOLS_DESTINATION)
    }));
}

#[test]
fn a_group_destination_page_expires_with_its_workspace_and_reopens_at_the_overview() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    let preserve = entry(&output, "Dock Tab Group as Layout Tab…");
    harness.run(
        WIDTH,
        vec![action(preserve, egui::accesskit::Action::Click)],
    );
    let output = settle(&mut harness);
    let stale_destination = entry(&output, TOOLS_DESTINATION);
    let activated = harness
        .dockspace
        .select_item_current(ItemId::new(2))
        .unwrap();
    assert!(matches!(
        activated.status(),
        crate::DockspaceActionStatus::Applied(crate::DockspaceActionOutcome::ItemActivated {
            item,
            changed: true,
        }) if *item == ItemId::new(2)
    ));
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
    let version = harness.dockspace.version();
    harness.run(
        WIDTH,
        vec![action(stale_destination, egui::accesskit::Action::Click)],
    );
    settle(&mut harness);
    assert_eq!(harness.dockspace.version(), version);
    assert_ne!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .tabs()
            .unwrap()
            .key(),
        harness
            .dockspace
            .view()
            .item(ItemId::new(3))
            .unwrap()
            .tabs()
            .unwrap()
            .key(),
    );
    let output = open_first_group(&mut harness);
    entry(&output, "Merge Tab Group Into…");
    assert!(!nodes(&output).iter().any(|(_, node)| {
        node.role() == Role::MenuItem && node.label() == Some(TOOLS_DESTINATION)
    }));
}

#[test]
fn layout_change_dismisses_group_menu_and_rejects_its_old_widget_action() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    let stale_entry = entry(&output, "Float Tab Group");
    assert!(matches!(
        harness
            .dockspace
            .select_item_current(ItemId::new(2))
            .unwrap()
            .status(),
        crate::DockspaceActionStatus::Applied(_)
    ));
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
    harness.run(
        WIDTH,
        vec![action(stale_entry, egui::accesskit::Action::Click)],
    );
    settle(&mut harness);
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .contained()
            .is_none()
    );
}

#[test]
fn a_fresh_session_dismisses_a_popup_with_the_same_widget_and_workspace_version() {
    let mut harness = setup();
    let output = open_first_group(&mut harness);
    let stale_entry = entry(&output, "Float Tab Group");
    let replacement = setup().dockspace;
    assert_eq!(harness.dockspace.id(), replacement.id());
    assert_eq!(harness.dockspace.version(), replacement.version());
    harness.dockspace = replacement;
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
    harness.run(
        WIDTH,
        vec![action(stale_entry, egui::accesskit::Action::Click)],
    );
    settle(&mut harness);
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .contained()
            .is_none()
    );
}

#[test]
fn keyboard_can_activate_atomic_group_close() {
    for central in [false, true] {
        for together in [false, true] {
            let mut harness = setup_with_central(central);
            open_first_group(&mut harness);
            if together {
                harness.run(WIDTH, vec![key(Key::ArrowUp), key(Key::Enter)]);
            } else {
                harness.run(WIDTH, vec![key(Key::ArrowUp)]);
                harness.run(WIDTH, vec![key(Key::Enter)]);
            }
            settle(&mut harness);
            let requests = harness
                .dockspace
                .session
                .pending_content_close_plans()
                .collect::<Vec<_>>();
            assert_eq!(requests.len(), 1, "central={central}, together={together}");
            let plan = &requests[0];
            assert_eq!(
                plan.items()
                    .iter()
                    .map(|item| item.item())
                    .collect::<Vec<_>>(),
                [ItemId::new(1), ItemId::new(2)]
            );
            assert!(harness.dockspace.view().item(ItemId::new(3)).is_some());
        }
    }
}

#[test]
fn action_menu_entrances_paint_an_immediate_outline_for_keyboard_focus() {
    let color = egui::Color32::from_rgb(29, 213, 117);
    for label in ["Tab group actions", "Layout actions"] {
        let mut harness = setup();
        let mut style = harness.dockspace.style().clone();
        style.visuals.tab_focus_stroke = Some(egui::Stroke::new(2.0, color));
        harness.dockspace.set_style(style).unwrap();
        harness.focus(WIDTH, Role::Button, label);
        let output = harness.run(WIDTH, Vec::new());
        let focused = harness.context.memory(egui::Memory::focused).unwrap();
        let bounds = nodes(&output)
            .iter()
            .find(|(id, node)| id.0 == focused.value() && node.label() == Some(label))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let bounds = Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        );
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.stroke.color == color
                    && rect.stroke.width == 2.0 && rect.rect.is_positive()
                    && bounds.contains_rect(rect.rect)
            )),
            "{label} must distinguish keyboard focus inside its allocated target"
        );
        assert!(
            !nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::MenuItem)
        );
    }
}
