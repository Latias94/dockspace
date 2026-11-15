//! Auxiliary tab clicks use real egui responses and the ordinary close protocol.

use super::*;

fn middle(position: Pos2, pressed: bool) -> Event {
    pointer_button(position, PointerButton::Middle, pressed)
}

fn pointer_button(position: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton {
        pos: position,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn body(harness: &Harness, item: ItemId) -> Pos2 {
    harness.panes.tabs[&item].left_center() + egui::vec2(8.0, 0.0)
}

fn collect_requests(harness: &mut Harness, events: Vec<Event>) -> Vec<crate::DockspaceClosePlan> {
    harness.step(events);
    let mut requests = harness.close_requests.clone();
    for _ in 0..3 {
        harness.step(Vec::new());
        requests.extend(harness.close_requests.clone());
    }
    requests
}

#[test]
fn mixed_button_batches_never_close_the_last_widgets_tab() {
    for callback_mode in [false, true] {
        for middle_first in [false, true] {
            let mut harness = Harness::new(DockStyle::default());
            harness.callback_mode = callback_mode;
            let first = body(&harness, FIRST);
            let second = body(&harness, SECOND);
            let [first_button, second_button] = if middle_first {
                [PointerButton::Middle, PointerButton::Primary]
            } else {
                [PointerButton::Primary, PointerButton::Middle]
            };
            let requests = collect_requests(
                &mut harness,
                vec![
                    Event::PointerMoved(first),
                    pointer_button(first, first_button, true),
                    pointer_button(first, first_button, false),
                    Event::PointerMoved(second),
                    pointer_button(second, second_button, true),
                    pointer_button(second, second_button, false),
                ],
            );
            assert!(
                requests.is_empty(),
                "callback={callback_mode}, middle_first={middle_first}"
            );
            let primary = if middle_first { SECOND } else { FIRST };
            assert_eq!(
                harness.dock.view().item(FIRST).unwrap().tabs().selected(),
                Some(primary)
            );
            assert_eq!(
                harness.dock.view().item(FIRST).unwrap().tabs().items(),
                [FIRST, SECOND]
            );
        }
    }
}

#[test]
fn a_primary_click_elsewhere_cannot_activate_a_middle_clicked_close_icon() {
    for callback_mode in [false, true] {
        let mut harness = Harness::new(DockStyle::default());
        harness.callback_mode = callback_mode;
        let first = body(&harness, FIRST);
        let bounds = harness
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Close Second"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let close = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        assert!(
            collect_requests(
                &mut harness,
                vec![
                    Event::PointerMoved(first),
                    button(first, true),
                    button(first, false),
                    Event::PointerMoved(close),
                    middle(close, true),
                    middle(close, false),
                ]
            )
            .is_empty(),
            "callback={callback_mode}"
        );
        assert_eq!(
            harness.dock.view().item(FIRST).unwrap().tabs().items(),
            [FIRST, SECOND]
        );
    }
}

#[test]
fn separate_button_clicks_keep_their_own_tab_targets_across_frames() {
    for callback_mode in [false, true] {
        for middle_first in [false, true] {
            let mut harness = Harness::new(DockStyle::default());
            harness.callback_mode = callback_mode;
            let first = body(&harness, FIRST);
            let second = body(&harness, SECOND);
            let (middle_target, primary_target, first_button, second_button) = if middle_first {
                (FIRST, SECOND, PointerButton::Middle, PointerButton::Primary)
            } else {
                (SECOND, FIRST, PointerButton::Primary, PointerButton::Middle)
            };
            let mut requests = Vec::new();
            for (position, button) in [(first, first_button), (second, second_button)] {
                for pressed in [true, false] {
                    harness.step(vec![
                        Event::PointerMoved(position),
                        pointer_button(position, button, pressed),
                    ]);
                    requests.extend(harness.close_requests.clone());
                }
            }
            for _ in 0..3 {
                harness.step(Vec::new());
                requests.extend(harness.close_requests.clone());
            }
            assert_eq!(
                requests.len(),
                1,
                "callback={callback_mode}, middle_first={middle_first}"
            );
            assert_eq!(
                requests[0].target(),
                crate::ClosePlanTarget::Item {
                    item: middle_target
                }
            );
            assert!(
                harness
                    .dock
                    .view()
                    .item(primary_target)
                    .unwrap()
                    .is_selected()
            );
        }
    }
}

#[test]
fn overlapping_button_presses_across_frames_cannot_close_another_buttons_capture() {
    for callback_mode in [false, true] {
        for middle_first in [false, true] {
            let mut harness = Harness::new(DockStyle::default());
            harness.callback_mode = callback_mode;
            let first = body(&harness, FIRST);
            let second = body(&harness, SECOND);
            let [held, other] = if middle_first {
                [PointerButton::Middle, PointerButton::Primary]
            } else {
                [PointerButton::Primary, PointerButton::Middle]
            };
            let mut requests = Vec::new();
            for events in [
                vec![
                    Event::PointerMoved(first),
                    pointer_button(first, held, true),
                ],
                vec![
                    Event::PointerMoved(second),
                    pointer_button(second, other, true),
                    pointer_button(second, other, false),
                ],
                vec![
                    Event::PointerMoved(first),
                    pointer_button(first, held, false),
                ],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ] {
                harness.step(events);
                requests.extend(harness.close_requests.clone());
            }
            assert!(
                requests.is_empty(),
                "callback={callback_mode}, middle_first={middle_first}"
            );
            assert_eq!(
                harness.dock.view().item(FIRST).unwrap().tabs().items(),
                [FIRST, SECOND]
            );
        }
    }
}

#[test]
fn multiple_auxiliary_releases_and_later_motion_do_not_infer_a_tab_target() {
    for callback_mode in [false, true] {
        for second_click in [false, true] {
            let mut harness = Harness::new(DockStyle::default());
            harness.callback_mode = callback_mode;
            let first = body(&harness, FIRST);
            let second = body(&harness, SECOND);
            let mut events = vec![
                Event::PointerMoved(first),
                middle(first, true),
                middle(first, false),
                Event::PointerMoved(second),
            ];
            if second_click {
                events.extend([middle(second, true), middle(second, false)]);
            }
            assert!(
                collect_requests(&mut harness, events).is_empty(),
                "callback={callback_mode}, second_click={second_click}"
            );
            assert_eq!(
                harness.dock.view().item(FIRST).unwrap().tabs().items(),
                [FIRST, SECOND]
            );
        }
    }
}

#[test]
fn a_primary_double_click_elsewhere_cannot_equalize_a_middle_clicked_splitter() {
    for callback_mode in [false, true] {
        let mut harness = Harness::new(DockStyle::default());
        harness.callback_mode = callback_mode;
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::split(
                    crate::DockspaceAxis::Horizontal,
                    [
                        (DockspaceNode::tabs([FIRST]), 0.25),
                        (DockspaceNode::tabs([SECOND]), 0.75),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        harness.dock = Dockspace::builder("mixed-button-equalization", layout)
            .build()
            .unwrap();
        harness.settle();
        let tab = body(&harness, FIRST);
        let bounds = harness
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::Splitter)
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let splitter = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        let before = harness
            .dock
            .view()
            .root(ROOT)
            .unwrap()
            .content()
            .unwrap()
            .split()
            .unwrap()
            .weights()
            .collect::<Vec<_>>();
        harness.step(vec![
            Event::PointerMoved(tab),
            button(tab, true),
            button(tab, false),
        ]);
        assert!(
            collect_requests(
                &mut harness,
                vec![
                    button(tab, true),
                    button(tab, false),
                    Event::PointerMoved(splitter),
                    middle(splitter, true),
                    middle(splitter, false),
                ]
            )
            .is_empty()
        );
        let after = harness
            .dock
            .view()
            .root(ROOT)
            .unwrap()
            .content()
            .unwrap()
            .split()
            .unwrap()
            .weights()
            .collect::<Vec<_>>();
        assert_eq!(after, before, "callback={callback_mode}");
    }
}

fn click(
    harness: &mut Harness,
    position: Pos2,
) -> (Vec<DockspaceEvent>, Vec<crate::DockspaceClosePlan>) {
    let mut events = harness.step(vec![Event::PointerMoved(position)]);
    let mut requests = harness.close_requests.clone();
    for event in [middle(position, true), middle(position, false)] {
        events.extend(harness.step(vec![event]));
        requests.extend(harness.close_requests.clone());
    }
    for _ in 0..3 {
        events.extend(harness.step(Vec::new()));
        requests.extend(harness.close_requests.clone());
    }
    (events, requests)
}

#[test]
fn middle_click_on_inactive_tab_requests_close_once_without_selecting_it() {
    let mut harness = Harness::new(DockStyle {
        tab_close_visibility: crate::TabCloseVisibility::Hovered,
        ..Default::default()
    });
    let position = harness.panes.tabs[&SECOND].left_center() + egui::vec2(8.0, 0.0);
    let (events, requests) = click(&mut harness, position);
    assert_eq!(selections(&events, SECOND), 0);
    assert!(harness.dock.view().item(FIRST).unwrap().is_selected());
    assert!(harness.dock.view().item(SECOND).is_some());
    assert_eq!(requests.len(), 1);
    let plan = &requests[0];
    assert_eq!(plan.target(), crate::ClosePlanTarget::Item { item: SECOND });
    let deferred = harness
        .dock
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Deferred,
        )
        .unwrap();
    let DockspaceCloseOutcome::Processed {
        resolution: DockspaceCloseResolution::Deferred { continuation, .. },
        ..
    } = deferred.outcome()
    else {
        panic!("a middle click must retain the regular deferred close decision");
    };
    let continuation = *continuation;
    assert!(harness.dock.view().item(SECOND).is_some());
    harness
        .dock
        .resolve_deferred_close(plan.request(), continuation, DeferredCloseDecision::Allow)
        .unwrap();
    assert!(harness.dock.view().item(SECOND).is_none());
    assert!(harness.settle().is_empty());
}

#[test]
fn middle_click_respects_configuration_policy_and_application_tab_widgets() {
    for blocked in ["binding", "policy", "widget"] {
        let mut harness = Harness::new(DockStyle::default());
        match blocked {
            "binding" => harness.dock.set_input(crate::DockInputConfig {
                middle_click_closes_tab: false,
                ..Default::default()
            }),
            "policy" => {
                let mut policy = DockPolicy::default();
                policy.set_close_capability(CloseCapability::Disabled);
                harness.dock.set_policy(policy).unwrap();
            }
            "widget" => harness.panes.blocked_tab = Some(SECOND),
            _ => unreachable!(),
        }
        harness.settle();
        let position = harness.panes.tabs[&SECOND].left_center() + egui::vec2(8.0, 0.0);
        let (events, requests) = click(&mut harness, position);
        assert!(requests.is_empty(), "{blocked}");
        assert_eq!(selections(&events, SECOND), 0, "{blocked}");
        assert!(harness.dock.view().item(SECOND).is_some());
    }
}

#[test]
fn clipped_body_and_close_icon_use_one_close_request_in_both_facades() {
    for callback_mode in [false, true] {
        for close_icon in [false, true] {
            let style = if close_icon {
                DockStyle::default()
            } else {
                // The second label is visible, but its trailing close button
                // lies beyond the strip viewport.
                DockStyle {
                    tab_min_width: 400.0,
                    tab_max_width: 400.0,
                    ..Default::default()
                }
            };
            let mut harness = Harness::new(style);
            harness.callback_mode = callback_mode;
            harness.settle();
            let position = if close_icon {
                let node = harness
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some("Close Second"))
                    .unwrap()
                    .1
                    .bounds()
                    .unwrap();
                egui::pos2(
                    ((node.x0 + node.x1) * 0.5) as f32,
                    ((node.y0 + node.y1) * 0.5) as f32,
                )
            } else {
                assert!(
                    !harness
                        .nodes
                        .iter()
                        .any(|(_, node)| node.label() == Some("Close Second"))
                );
                harness.panes.tabs[&SECOND].left_center() + egui::vec2(8.0, 0.0)
            };
            let (events, requests) = click(&mut harness, position);
            assert_eq!(
                requests.len(),
                1,
                "callback_mode={callback_mode}, close_icon={close_icon}"
            );
            assert_eq!(
                requests[0].target(),
                crate::ClosePlanTarget::Item { item: SECOND }
            );
            assert_eq!(selections(&events, SECOND), 0);
            assert!(harness.dock.view().item(FIRST).unwrap().is_selected());
        }
    }
}

#[test]
fn discarded_passes_and_a_batched_middle_click_do_not_duplicate_closure() {
    let mut harness = Harness::new(DockStyle::default());
    let position = harness.panes.tabs[&SECOND].left_center() + egui::vec2(8.0, 0.0);
    harness.step(vec![Event::PointerMoved(position)]);
    harness
        .context
        .options_mut(|options| options.max_passes = NonZeroUsize::new(3).unwrap());
    harness.context.add_plugin(LateDiscard { remaining: 2 });
    let mut events = harness.step(vec![middle(position, true), middle(position, false)]);
    assert_eq!(harness.passes, 3);
    let mut requests = harness.close_requests.clone();
    for _ in 0..3 {
        events.extend(harness.step(Vec::new()));
        requests.extend(harness.close_requests.clone());
    }
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].target(),
        crate::ClosePlanTarget::Item { item: SECOND }
    );
    assert_eq!(selections(&events, SECOND), 0);
}
