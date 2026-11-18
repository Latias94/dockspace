//! Window deactivation retires both built-in and custom menu presses.

use super::*;

#[derive(Clone, Copy, Debug)]
enum Boundary {
    Snapshot,
    UnfocusedRelease,
    LaterRegain,
    RegainBeforeRelease,
    RegainAfterRelease,
}

impl Boundary {
    fn deliver(self, harness: &mut Harness, position: Pos2) -> usize {
        let mut run = |focused, events| {
            harness.frame += 1;
            harness
                .run_input(RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
                    time: Some(f64::from(harness.frame) * 0.1),
                    focused,
                    events,
                    ..Default::default()
                })
                .platform_output
                .num_completed_passes
        };
        match self {
            Self::Snapshot => run(false, vec![button(position, false)]),
            Self::UnfocusedRelease => run(
                false,
                vec![Event::WindowFocused(false), button(position, false)],
            ),
            Self::LaterRegain => {
                let passes = run(false, vec![Event::WindowFocused(false)]);
                run(
                    true,
                    vec![Event::WindowFocused(true), button(position, false)],
                );
                passes
            }
            Self::RegainBeforeRelease => run(
                true,
                vec![
                    Event::WindowFocused(false),
                    Event::WindowFocused(true),
                    button(position, false),
                ],
            ),
            Self::RegainAfterRelease => run(
                true,
                vec![
                    Event::WindowFocused(false),
                    button(position, false),
                    Event::WindowFocused(true),
                ],
            ),
        }
    }
}

struct RepeatPasses(usize);

impl egui::plugin::Plugin for RepeatPasses {
    fn debug_name(&self) -> &'static str {
        "menu-focus-loss-repeated-passes"
    }

    fn output_hook(&mut self, _: &Context, output: &mut FullOutput) {
        if self.0 > 0 {
            self.0 -= 1;
            output
                .platform_output
                .request_discard_reasons
                .push(egui::RepaintCause::new_reason(
                    "repeat the menu focus-loss boundary",
                ));
        }
    }
}

#[test]
fn a_menu_press_is_cancelled_by_focus_loss_even_when_focus_returns_before_release() {
    for callback in [false, true] {
        for built_in in [false, true] {
            for discards in [0, 2] {
                for boundary in [
                    Boundary::Snapshot,
                    Boundary::UnfocusedRelease,
                    Boundary::LaterRegain,
                    Boundary::RegainBeforeRelease,
                    Boundary::RegainAfterRelease,
                ] {
                    let mut harness = setup(callback);
                    if built_in {
                        harness.panes.customize_menu = None;
                    }
                    let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
                    let old = menu_entry(&output, if built_in { "Float Tab" } else { "Tool 1" });
                    let position = center(&output, old);
                    let version = harness.dockspace.version();
                    harness.run(WIDTH, vec![Event::PointerMoved(position)]);
                    harness.run(WIDTH, vec![button(position, true)]);
                    harness.context.options_mut(|options| {
                        options.max_passes = (discards + 1).try_into().unwrap();
                    });
                    harness.context.add_plugin(RepeatPasses(discards));
                    assert_eq!(boundary.deliver(&mut harness, position), discards + 1);
                    assert!(
                        harness.panes.menu_actions.is_empty(),
                        "{callback}/{built_in}/{discards}/{boundary:?}: a pre-focus-loss press cannot activate on a late release"
                    );
                    assert_eq!(harness.dockspace.version(), version);
                    assert!(!egui::Popup::is_any_open(&harness.context));

                    harness.run(WIDTH, vec![Event::WindowFocused(true)]);
                    harness.panes.customize_menu = Some(tools);
                    let (output, _) = open(&mut harness, "Document 2", Open::Accessible);
                    let fresh = center(&output, menu_entry(&output, "Tool 2"));
                    harness.run(WIDTH, vec![Event::PointerMoved(fresh)]);
                    harness.run(WIDTH, vec![button(fresh, true)]);
                    harness.run(WIDTH, vec![button(fresh, false)]);
                    assert_one_action(&harness, 2);
                    assert_eq!(harness.dockspace.version(), version);
                }
            }
        }
    }
}
