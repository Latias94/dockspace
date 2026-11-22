//! Wheel input must reach official-egui tab chrome through its actual owner.

use super::*;
use dockspace::runtime::{
    SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
};

const WIDTH: f32 = 640.0;

fn setup() -> Harness {
    let mut harness = Harness::new(
        (1..=40).map(|item| format!("Document {item}")).collect(),
        ItemId::new(1),
        DockStyle {
            tab_min_width: 180.0,
            tab_max_width: 180.0,
            ..DockStyle::default()
        },
    );
    for _ in 0..3 {
        harness.run(WIDTH, Vec::new());
    }
    harness
}

fn wheel(delta: egui::Vec2) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "The test fixtures use small finite screen coordinates."
)]
fn center(rect: crate::LogicalRect) -> Pos2 {
    egui::pos2(
        ((rect.x() + rect.max().x()) * 0.5) as f32,
        ((rect.y() + rect.max().y()) * 0.5) as f32,
    )
}

fn strip(harness: &mut Harness) -> (Pos2, f64) {
    strips(harness)
        .pop()
        .expect("the fixture contains a tab strip")
}

fn strips(harness: &mut Harness) -> Vec<(Pos2, f64)> {
    let mut frame = harness.dockspace.session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let mut result = Vec::new();
    plan.visit_surface_program(|step| {
        if let Step::Operation {
            operation: Operation::TabBar(bar),
            ..
        } = step
        {
            result.push((center(bar.viewport()), bar.scroll_offset()));
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    result.sort_by(|(left, _), (right, _)| left.x.total_cmp(&right.x));
    result
}

fn menu(harness: &mut Harness) -> (Pos2, f64) {
    let mut frame = harness.dockspace.session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let mut result = None;
    plan.visit_surface_program(|step| {
        if let Step::PopupMenu { menu, .. } = step {
            assert!(
                menu.maximum_scroll_offset() > 0.0,
                "the popup fixture must contain more rows than its available viewport"
            );
            result = Some((center(menu.viewport()), menu.scroll_offset()));
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    result.expect("the fixture has opened its tab list")
}

fn open_menu(harness: &mut Harness) {
    let output = harness.run(WIDTH, Vec::new());
    let trigger = nodes(&output)
        .iter()
        .find_map(|(id, node)| (node.label() == Some("List tabs")).then_some(*id))
        .expect("the overflowing tab strip exposes its list trigger");
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
    for _ in 0..3 {
        harness.run(WIDTH, Vec::new());
    }
}

fn accesskit_scroll_event(
    target: egui::accesskit::NodeId,
    action: egui::accesskit::Action,
) -> Event {
    Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: target,
        data: None,
    })
}

fn accesskit_scroll_fixture(callback: bool) -> (Harness, egui::accesskit::NodeId) {
    let mut harness = setup();
    if callback {
        harness.local_style = Some((*harness.context.global_style()).clone());
    }
    open_menu(&mut harness);
    let output = harness.run(WIDTH, Vec::new());
    let target = nodes(&output)
        .iter()
        .find_map(|(id, node)| {
            (node.role() == Role::ScrollBar && node.label() == Some("Open tabs scroll position"))
                .then_some(*id)
        })
        .expect("the overflowing menu exposes its actual accessible scrollbar");
    assert_eq!(menu(&mut harness).1, 0.0);
    (harness, target)
}

#[test]
fn accesskit_tab_list_scroll_batch_preserves_steps_order_and_endpoint_clamping() {
    use egui::accesskit::Action::{Decrement, Increment};
    for callback in [false, true] {
        for discards in [0, 2] {
            for actions in [
                vec![Increment, Increment],
                vec![Increment, Decrement],
                vec![Decrement, Increment],
                vec![Decrement, Increment, Increment],
            ] {
                let (mut separate, target) = accesskit_scroll_fixture(callback);
                for &action in &actions {
                    separate.run(WIDTH, vec![accesskit_scroll_event(target, action)]);
                    for _ in 0..3 {
                        separate.run(WIDTH, Vec::new());
                    }
                }
                let expected = menu(&mut separate).1;

                let (mut batched, target) = accesskit_scroll_fixture(callback);
                batched.context.options_mut(|options| {
                    options.max_passes = (discards + 1).try_into().unwrap();
                });
                batched.context.add_plugin(LateWheelDiscard {
                    remaining: discards,
                });
                let output = batched.run(
                    WIDTH,
                    actions
                        .iter()
                        .map(|&action| accesskit_scroll_event(target, action))
                        .collect(),
                );
                assert_eq!(output.platform_output.num_completed_passes, discards + 1);
                for _ in 0..3 {
                    batched.run(WIDTH, Vec::new());
                }
                let actual = menu(&mut batched).1;
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "callback={callback}, discards={discards}, actions={actions:?}: {actual} != {expected}",
                );
                assert!(
                    batched
                        .dockspace
                        .view()
                        .item(ItemId::new(1))
                        .unwrap()
                        .is_active()
                );
                batched.run(WIDTH, Vec::new());
                assert_eq!(
                    menu(&mut batched).1,
                    actual,
                    "idle must not replay the batch"
                );
            }
        }
    }
}

fn run_with(
    harness: &mut Harness,
    events: Vec<Event>,
    mut paint: impl FnMut(&mut Ui, &mut Dockspace, &mut Panes),
) -> FullOutput {
    harness.frame += 1;
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
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
fn official_tab_strip_horizontal_wheel_scrolls_without_changing_selection() {
    let mut harness = setup();
    let (point, before) = strip(&mut harness);
    harness.run(WIDTH, vec![Event::PointerMoved(point)]);
    harness.run(WIDTH, vec![wheel(vec2(-90.0, 0.0))]);
    harness.run(WIDTH, Vec::new());
    let (_, after) = strip(&mut harness);
    assert!(
        after > before,
        "a horizontal wheel over the official-egui tab strip must reveal later tabs"
    );
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .is_active(),
        "browsing the tab strip must keep the current pane selected"
    );
}

#[test]
fn official_tab_list_vertical_wheel_scrolls_without_changing_selection() {
    let mut harness = setup();
    open_menu(&mut harness);
    let (point, before) = menu(&mut harness);
    let (_, strip_before) = strip(&mut harness);
    harness.run(WIDTH, vec![Event::PointerMoved(point)]);
    harness.run(WIDTH, vec![wheel(vec2(0.0, -90.0))]);
    harness.run(WIDTH, Vec::new());
    let (_, after) = menu(&mut harness);
    assert!(
        after > before,
        "a vertical wheel over the official-egui tab list must reveal later rows"
    );
    assert_eq!(
        strip(&mut harness).1,
        strip_before,
        "the popup owns its wheel independently of the underlying tab strip"
    );
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(1))
            .unwrap()
            .is_active()
    );
}

#[test]
fn official_tab_list_wheel_clamps_before_reversing_in_the_same_packet() {
    for separate in [false, true] {
        for (deltas, expected) in [([4.0, -4.0], 4.0), ([-4.0, 4.0], 0.0)] {
            let mut harness = setup();
            open_menu(&mut harness);
            let (point, before) = menu(&mut harness);
            assert_eq!(before, 0.0);
            harness.run(WIDTH, vec![Event::PointerMoved(point)]);
            let events = vec![
                wheel(vec2(0.0, deltas[0])),
                Event::PointerMoved(point + vec2(1.0, 0.0)),
                wheel(vec2(0.0, deltas[1])),
            ];
            if separate {
                for event in events {
                    harness.run(WIDTH, vec![event]);
                }
            } else {
                harness.run(WIDTH, events);
            }
            harness.run(WIDTH, Vec::new());
            assert_eq!(
                menu(&mut harness).1,
                expected,
                "separate={separate}, deltas={deltas:?}"
            );
        }
    }
}

#[test]
fn official_tab_wheel_respects_framework_units_shift_and_scroll_style() {
    for (unit, delta, modifiers, only_direction) in [
        (
            egui::MouseWheelUnit::Point,
            vec2(-4.0, 0.0),
            egui::Modifiers::NONE,
            false,
        ),
        (
            egui::MouseWheelUnit::Line,
            vec2(-1.0, 0.0),
            egui::Modifiers::NONE,
            false,
        ),
        (
            egui::MouseWheelUnit::Page,
            vec2(-0.1, 0.0),
            egui::Modifiers::NONE,
            false,
        ),
        (
            egui::MouseWheelUnit::Point,
            vec2(0.0, -4.0),
            egui::Modifiers::SHIFT,
            false,
        ),
        (
            egui::MouseWheelUnit::Point,
            vec2(0.0, -4.0),
            egui::Modifiers::NONE,
            true,
        ),
    ] {
        let mut harness = setup();
        let (point, before) = strip(&mut harness);
        run_with(
            &mut harness,
            vec![Event::PointerMoved(point)],
            |ui, dockspace, panes| {
                ui.style_mut().always_scroll_the_only_direction = only_direction;
                dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
            },
        );
        let event = Event::MouseWheel {
            unit,
            delta,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers,
        };
        let mut expected = 0.0;
        run_with(&mut harness, vec![event.clone()], |ui, dockspace, panes| {
            ui.style_mut().always_scroll_the_only_direction = only_direction;
            let options = ui.ctx().options(|options| options.input_options);
            let delta = ui.input(|input| {
                assert_eq!(input.smooth_scroll_delta(), egui::Vec2::ZERO);
                egui::InputState::default()
                    .begin_pass(
                        RawInput {
                            screen_rect: Some(input.viewport_rect()),
                            time: Some(input.time),
                            predicted_dt: input.stable_dt,
                            events: vec![event.clone()],
                            ..Default::default()
                        },
                        false,
                        input.pixels_per_point(),
                        options,
                    )
                    .smooth_scroll_delta()
            });
            expected = -f64::from(delta.x)
                - if only_direction {
                    f64::from(delta.y)
                } else {
                    0.0
                };
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
        });
        assert!(expected > 0.0);
        run_with(&mut harness, vec![], |ui, dockspace, panes| {
            ui.style_mut().always_scroll_the_only_direction = only_direction;
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
        });
        assert!(
            (strip(&mut harness).1 - before - expected).abs() < 1e-6,
            "tab scrolling must use egui's normalized units, modifiers and direction policy"
        );
    }
}

#[test]
fn official_tab_wheel_yields_to_same_layer_foreign_widgets() {
    let mut harness = setup();
    let (point, before) = strip(&mut harness);
    let hit = Rect::from_center_size(point, vec2(40.0, 16.0));
    let mut remaining = egui::Vec2::ZERO;
    let mut paint = |ui: &mut Ui, dockspace: &mut Dockspace, panes: &mut Panes| {
        dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
        remaining = ui.input(|input| input.smooth_scroll_delta());
        let _ = ui.interact(
            hit,
            ui.make_persistent_id("foreign-wheel-owner"),
            egui::Sense::click_and_drag(),
        );
    };
    for _ in 0..3 {
        run_with(&mut harness, vec![Event::PointerMoved(point)], &mut paint);
    }
    run_with(&mut harness, vec![wheel(vec2(-4.0, 0.0))], &mut paint);
    assert_eq!(
        remaining,
        vec2(-4.0, 0.0),
        "an unrelated widget keeps its unconsumed wheel"
    );
    harness.run(WIDTH, Vec::new());
    assert_eq!(strip(&mut harness).1, before);
}

#[test]
fn official_tab_strip_and_popup_wheel_are_blocked_by_modal() {
    for popup in [false, true] {
        let mut harness = setup();
        if popup {
            open_menu(&mut harness);
        }
        let (point, before) = if popup {
            menu(&mut harness)
        } else {
            strip(&mut harness)
        };
        let mut paint = |ui: &mut Ui, dockspace: &mut Dockspace, panes: &mut Panes| {
            egui::Modal::new(egui::Id::unique("wheel-blocking-modal")).show(ui.ctx(), |ui| {
                ui.label("Modal input owner");
            });
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
        };
        for _ in 0..3 {
            run_with(&mut harness, vec![Event::PointerMoved(point)], &mut paint);
        }
        let delta = if popup {
            vec2(0.0, -4.0)
        } else {
            vec2(-4.0, 0.0)
        };
        run_with(&mut harness, vec![wheel(delta)], &mut paint);
        run_with(&mut harness, Vec::new(), &mut paint);
        assert_eq!(
            if popup {
                menu(&mut harness).1
            } else {
                strip(&mut harness).1
            },
            before
        );
    }
}

#[test]
fn official_tab_list_bounds_and_backdrop_block_wheel_chaining() {
    for backdrop in [false, true] {
        let mut harness = setup();
        open_menu(&mut harness);
        let (point, before) = menu(&mut harness);
        let point = if backdrop {
            egui::pos2(10.0, 10.0)
        } else {
            point
        };
        harness.run(WIDTH, vec![Event::PointerMoved(point)]);
        let mut remaining = vec2(1.0, 1.0);
        run_with(
            &mut harness,
            vec![wheel(vec2(2.0, 4.0))],
            |ui, dockspace, panes| {
                dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
                remaining = ui.input(|input| input.smooth_scroll_delta());
            },
        );
        assert_eq!(
            remaining,
            egui::Vec2::ZERO,
            "the core popup owns the wheel even at its first row or outside its frame"
        );
        harness.run(WIDTH, Vec::new());
        assert_eq!(menu(&mut harness).1, before);
        assert_eq!(strip(&mut harness).1, 0.0);
    }
}

struct LateWheelDiscard {
    remaining: usize,
}

impl egui::plugin::Plugin for LateWheelDiscard {
    fn debug_name(&self) -> &'static str {
        "official-tab-wheel-late-discard"
    }

    fn output_hook(&mut self, _: &Context, output: &mut FullOutput) {
        if self.remaining > 0 {
            self.remaining -= 1;
            output
                .platform_output
                .request_discard_reasons
                .push(egui::RepaintCause::new_reason(
                    "retry after tab chrome consumed the wheel",
                ));
        }
    }
}

#[test]
fn official_tab_wheel_survives_late_discards_without_repeating_the_action() {
    for callback in [false, true] {
        let mut harness = setup();
        if callback {
            harness.local_style = Some((*harness.context.global_style()).clone());
        }
        let (point, before) = strip(&mut harness);
        harness.run(WIDTH, vec![Event::PointerMoved(point)]);
        harness
            .context
            .options_mut(|options| options.max_passes = 4.try_into().unwrap());
        harness
            .context
            .add_plugin(LateWheelDiscard { remaining: 2 });
        let output = harness.run(WIDTH, vec![wheel(vec2(-4.0, 0.0))]);
        assert_eq!(output.platform_output.num_completed_passes, 3);
        harness.run(WIDTH, Vec::new());
        assert_eq!(strip(&mut harness).1, before + 4.0);
        harness.run(WIDTH, vec![wheel(vec2(-4.0, 0.0))]);
        harness.run(WIDTH, Vec::new());
        assert_eq!(
            strip(&mut harness).1,
            before + 8.0,
            "identical wheel input in a new original run remains independent"
        );
    }
}

#[test]
fn official_pane_content_keeps_its_own_vertical_scroll() {
    #[derive(Default)]
    struct ScrollableContent {
        offset: f32,
    }

    impl PaneView for ScrollableContent {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Document {}", item.get()).into())
        }

        fn ui(&mut self, _: ItemId, ui: &mut Ui) -> PaneResponse {
            self.offset = egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    for row in 0..200 {
                        ui.label(format!("Document line {row}"));
                    }
                })
                .state
                .clamped_offset()
                .y;
            PaneResponse::default()
        }
    }

    let mut harness = setup();
    let mut content = ScrollableContent::default();
    let mut paint = |ui: &mut Ui, dockspace: &mut Dockspace, _: &mut Panes| {
        dockspace
            .show_single_surface(SURFACE, ui, &mut content)
            .unwrap();
    };
    for _ in 0..3 {
        run_with(
            &mut harness,
            vec![Event::PointerMoved(egui::pos2(40.0, 100.0))],
            &mut paint,
        );
    }
    run_with(&mut harness, vec![wheel(vec2(0.0, -4.0))], &mut paint);
    assert!(content.offset > 0.0);
    harness.run(WIDTH, Vec::new());
    assert_eq!(strip(&mut harness).1, 0.0);
}

#[test]
fn official_tab_list_owns_wheel_before_pane_content_runs() {
    #[derive(Default)]
    struct Document {
        bounds: Option<Rect>,
        offset: f32,
    }

    impl PaneView for Document {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Document {item}").into())
        }

        fn ui(&mut self, _: ItemId, ui: &mut Ui) -> PaneResponse {
            let area = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_min_height(3_200.0);
                    ui.label("Document behind the tab list")
                });
            self.bounds = Some(area.inner_rect);
            self.offset = area.state.clamped_offset().y;
            PaneResponse::default()
                .with_scroll_area(ui, &area)
                .with_content(&area.inner)
        }
    }

    let mut harness = setup();
    open_menu(&mut harness);
    let (point, before) = menu(&mut harness);
    let mut document = Document::default();
    let mut run = |harness: &mut Harness, events| {
        harness.frame += 1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            time: Some(f64::from(harness.frame) * 0.1),
            events,
            ..Default::default()
        };
        let (output, _) = harness
            .dockspace
            .run_single_surface(&harness.context, input, SURFACE, &mut document)
            .unwrap();
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        assert!(document.bounds.unwrap().contains(point));
        assert_eq!(document.offset, 0.0);
    };
    run(&mut harness, vec![Event::PointerMoved(point)]);
    run(&mut harness, vec![wheel(vec2(0.0, -120.0))]);
    for _ in 0..12 {
        run(&mut harness, vec![]);
    }
    assert!((menu(&mut harness).1 - before - 120.0).abs() < 0.001);
}

#[test]
fn opposite_wheel_events_reach_both_tab_strips_in_one_packet() {
    let mut harness = setup();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [1..=20, 21..=40].map(|items| DockspaceNode::tabs(items.map(ItemId::new))),
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("independent-wheel-strips", layout)
        .style(DockStyle {
            tab_min_width: 120.0,
            tab_max_width: 120.0,
            ..Default::default()
        })
        .build()
        .unwrap();
    for _ in 0..3 {
        harness.run(WIDTH, vec![]);
    }
    let positions = strips(&mut harness);
    let [(left, _), (right, _)] = positions[..] else {
        panic!("expected two overflowing tab strips")
    };
    harness.run(
        WIDTH,
        vec![
            Event::PointerMoved(left),
            wheel(vec2(-120.0, 0.0)),
            Event::PointerMoved(right),
            wheel(vec2(-120.0, 0.0)),
        ],
    );
    for _ in 0..12 {
        harness.run(WIDTH, vec![]);
    }
    let before = strips(&mut harness);
    assert!(before.iter().all(|(_, offset)| *offset > 60.0));
    harness.run(
        WIDTH,
        vec![
            Event::PointerMoved(left),
            wheel(vec2(60.0, 0.0)),
            Event::PointerMoved(right),
            wheel(vec2(-60.0, 0.0)),
        ],
    );
    for _ in 0..12 {
        harness.run(WIDTH, vec![]);
    }
    let after = strips(&mut harness);
    assert!((after[0].1 - before[0].1 + 60.0).abs() < 0.001);
    assert!((after[1].1 - before[1].1 - 60.0).abs() < 0.001);
}

#[test]
fn manual_pass_keeps_wheel_tail_and_trackpad_momentum_with_the_tab_list() {
    let mut harness = setup();
    let document = std::cell::Cell::new((Pos2::ZERO, 0.0));
    let run = |harness: &mut Harness, events| {
        harness.frame += 1;
        harness.context.begin_pass(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            time: Some(f64::from(harness.frame) * 0.1),
            events,
            ..Default::default()
        });
        harness.context.root_ui(|ui| {
            ui.columns(2, |columns| {
                let [dock, app] = columns else { unreachable!() };
                let area = egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(app, |ui| {
                        ui.set_min_height(3_200.0);
                        ui.label("Adjacent application document");
                    });
                document.set((area.inner_rect.center(), area.state.clamped_offset().y));
                dock.shrink_clip_rect(dock.available_rect_before_wrap());
                harness
                    .dockspace
                    .show_single_surface(SURFACE, dock, &mut harness.panes)
                    .unwrap();
            });
        });
        let mut output = harness.context.end_pass();
        output.textures_delta.clear();
        output
    };
    run(&mut harness, vec![]);
    let output = run(&mut harness, vec![]);
    let trigger = nodes(&output)
        .iter()
        .find_map(|(id, node)| (node.label() == Some("List tabs")).then_some(*id))
        .unwrap();
    run(
        &mut harness,
        vec![accesskit_scroll_event(
            trigger,
            egui::accesskit::Action::Click,
        )],
    );
    run(&mut harness, vec![]);
    run(&mut harness, vec![]);
    let (point, before) = menu(&mut harness);
    let app = document.get().0;
    run(
        &mut harness,
        vec![Event::PointerMoved(point), wheel(vec2(0.0, -120.0))],
    );
    run(
        &mut harness,
        vec![Event::PointerMoved(app), wheel(vec2(0.0, -4.0))],
    );
    for _ in 0..12 {
        run(&mut harness, vec![]);
    }
    assert!((menu(&mut harness).1 - before - 120.0).abs() < 0.001);
    assert_eq!(document.get().1, 4.0);

    use egui::{
        MouseWheelSource::{Momentum, Trackpad},
        TouchPhase::{End, Move, Start},
    };
    let phase = |source, phase, y| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, y),
        phase,
        modifiers: egui::Modifiers::NONE,
        source,
    };
    run(
        &mut harness,
        vec![Event::PointerMoved(point), phase(Trackpad, Start, 0.0)],
    );
    run(
        &mut harness,
        vec![
            Event::PointerMoved(point + vec2(1.0, 0.0)),
            phase(Trackpad, Move, -20.0),
        ],
    );
    run(
        &mut harness,
        vec![Event::PointerMoved(app), phase(Trackpad, Move, -30.0)],
    );
    run(&mut harness, vec![phase(Trackpad, End, 0.0)]);
    run(&mut harness, vec![phase(Momentum, Start, 0.0)]);
    run(&mut harness, vec![phase(Momentum, Move, -10.0)]);
    run(&mut harness, vec![phase(Momentum, End, 0.0)]);
    run(&mut harness, vec![]);
    assert!((menu(&mut harness).1 - before - 180.0).abs() < 0.001);
    assert_eq!(document.get().1, 4.0);
}

#[test]
fn official_tab_strip_does_not_acquire_pane_content_smoothing_tail() {
    #[derive(Default)]
    struct WideContent {
        offset: egui::Vec2,
    }

    impl PaneView for WideContent {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Document {}", item.get()).into())
        }

        fn ui(&mut self, _: ItemId, ui: &mut Ui) -> PaneResponse {
            self.offset = egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_size(vec2(4_000.0, 2_400.0));
                    ui.label("Wide application document");
                })
                .state
                .clamped_offset();
            PaneResponse::default()
        }
    }

    let mut harness = setup();
    let (header, before) = strip(&mut harness);
    let mut content = WideContent::default();
    let mut sample = egui::Vec2::ZERO;
    {
        let mut paint = |ui: &mut Ui, dockspace: &mut Dockspace, _: &mut Panes| {
            sample = ui.input(egui::InputState::smooth_scroll_delta);
            dockspace
                .show_single_surface(SURFACE, ui, &mut content)
                .unwrap();
        };
        for _ in 0..3 {
            run_with(
                &mut harness,
                vec![Event::PointerMoved(egui::pos2(40.0, 100.0))],
                &mut paint,
            );
        }
        run_with(
            &mut harness,
            vec![wheel(vec2(-2_400.0, -2_400.0))],
            &mut paint,
        );
        run_with(&mut harness, vec![Event::PointerMoved(header)], &mut paint);
    }
    assert!(content.offset.x > 0.0 && content.offset.y > 0.0);
    assert!(
        sample.x < 0.0,
        "the pointer enters the tab strip while egui still has a real horizontal smoothing tail"
    );
    for _ in 0..3 {
        run_with(&mut harness, Vec::new(), |ui, dockspace, _| {
            dockspace
                .show_single_surface(SURFACE, ui, &mut content)
                .unwrap();
        });
    }
    assert_eq!(
        strip(&mut harness).1,
        before,
        "moving the pointer must not retarget the document's pending wheel into its header"
    );

    harness.run(WIDTH, vec![wheel(vec2(-4.0, 0.0))]);
    harness.run(WIDTH, Vec::new());
    assert!(
        strip(&mut harness).1 > before,
        "a new original wheel over the header can acquire its own scroll receiver"
    );
}

#[test]
fn official_tab_strip_continues_its_own_smoothing_tail() {
    let mut harness = setup();
    let (point, before) = strip(&mut harness);
    harness.run(WIDTH, vec![Event::PointerMoved(point)]);
    run_with(
        &mut harness,
        vec![wheel(vec2(-90.0, 0.0))],
        |ui, dockspace, panes| {
            assert_eq!(ui.input(egui::InputState::smooth_scroll_delta).x, 0.0);
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
        },
    );
    run_with(&mut harness, Vec::new(), |ui, dockspace, panes| {
        assert_eq!(ui.input(egui::InputState::smooth_scroll_delta).x, 0.0);
        dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
    });
    let first = strip(&mut harness).1 - before;
    assert!(first > 0.0 && first < 90.0);
    harness.run(WIDTH, Vec::new());
    assert!(
        strip(&mut harness).1 - before > first,
        "a header's acquired wheel keeps its own subsequent smoothing sample"
    );
    for _ in 0..8 {
        harness.run(WIDTH, Vec::new());
    }
    assert!((strip(&mut harness).1 - before - 90.0).abs() < 1e-5);
}

#[test]
fn official_tab_strip_continuous_wheel_keeps_samples_as_tab_geometry_moves() {
    let mut harness = setup();
    let (point, before) = strip(&mut harness);
    harness.run(WIDTH, vec![Event::PointerMoved(point)]);
    for _ in 0..6 {
        harness.run(WIDTH, vec![wheel(vec2(-4.0, 0.0))]);
    }
    harness.run(WIDTH, Vec::new());
    assert_eq!(
        strip(&mut harness).1,
        before + 24.0,
        "the same header remains the wheel owner while its tabs scroll under the pointer"
    );
}

#[test]
fn official_tab_wheel_rejects_disabled_ui_and_framework_zoom() {
    for disabled in [false, true] {
        let mut harness = setup();
        let (point, before) = strip(&mut harness);
        harness.run(WIDTH, vec![Event::PointerMoved(point)]);
        let mut event = wheel(vec2(-4.0, 0.0));
        if !disabled && let Event::MouseWheel { modifiers, .. } = &mut event {
            *modifiers = egui::Modifiers::CTRL;
        }
        let mut zoom = 1.0;
        run_with(&mut harness, vec![event], |ui, dockspace, panes| {
            if disabled {
                ui.disable();
            }
            dockspace.show_single_surface(SURFACE, ui, panes).unwrap();
            zoom = ui.input(egui::InputState::zoom_delta);
        });
        harness.run(WIDTH, Vec::new());
        assert_eq!(strip(&mut harness).1, before);
        if !disabled {
            assert_ne!(zoom, 1.0, "Ctrl-wheel remains egui's zoom input");
        }
    }
}
