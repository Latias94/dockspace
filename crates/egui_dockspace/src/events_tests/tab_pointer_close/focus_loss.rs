//! Auxiliary clicks must not outlive a window-focus cancellation boundary.

use super::*;

#[test]
fn a_middle_tab_press_cannot_close_after_window_focus_is_lost() {
    for callback_mode in [false, true] {
        for separate in [false, true] {
            let mut harness = Harness::new(DockStyle::default());
            harness.callback_mode = callback_mode;
            let position = body(&harness, SECOND);
            harness.step(vec![Event::PointerMoved(position)]);
            harness.step(vec![middle(position, true)]);
            if separate {
                harness.step(vec![Event::WindowFocused(false)]);
                harness.step(vec![Event::WindowFocused(true), middle(position, false)]);
            } else {
                harness.step(vec![
                    Event::WindowFocused(false),
                    Event::WindowFocused(true),
                    middle(position, false),
                ]);
            }
            assert!(
                harness.close_requests.is_empty(),
                "callback={callback_mode}, separate={separate}: a cancelled middle press cannot request close"
            );
            assert!(collect_requests(&mut harness, Vec::new()).is_empty());
            let (_, requests) = click(&mut harness, position);
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].target(),
                crate::ClosePlanTarget::Item { item: SECOND }
            );
        }
    }
}

#[test]
fn a_fresh_press_after_a_cancelled_release_cannot_retroactively_validate_the_release() {
    for callback_mode in [false, true] {
        let mut harness = Harness::new(DockStyle::default());
        harness.callback_mode = callback_mode;
        let position = body(&harness, SECOND);
        harness.step(vec![Event::PointerMoved(position)]);
        harness.step(vec![middle(position, true)]);
        harness.step(vec![
            Event::WindowFocused(false),
            Event::WindowFocused(true),
            middle(position, false),
            middle(position, true),
        ]);
        assert!(collect_requests(&mut harness, Vec::new()).is_empty());
        let requests = collect_requests(&mut harness, vec![middle(position, false)]);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].target(),
            crate::ClosePlanTarget::Item { item: SECOND }
        );
    }
}

#[test]
fn middle_click_continuity_requires_a_focused_press_in_an_observed_pass() {
    for callback_mode in [false, true] {
        for missed_pass in [false, true] {
            let mut harness = Harness::new(DockStyle::default());
            harness.callback_mode = callback_mode;
            let position = body(&harness, SECOND);
            harness.step(vec![Event::PointerMoved(position)]);
            harness.step(vec![middle(position, true)]);
            if missed_pass {
                harness
                    .context
                    .run_ui(
                        RawInput {
                            events: vec![Event::WindowFocused(false)],
                            ..Default::default()
                        },
                        |_| {},
                    )
                    .drop_without_applying_deltas();
                harness.step(vec![Event::WindowFocused(true), middle(position, false)]);
            } else {
                harness.step(vec![
                    Event::WindowFocused(false),
                    middle(position, true),
                    Event::WindowFocused(true),
                    middle(position, false),
                ]);
            }
            assert!(collect_requests(&mut harness, Vec::new()).is_empty());
            let requests = collect_requests(
                &mut harness,
                vec![
                    Event::WindowFocused(false),
                    Event::WindowFocused(true),
                    Event::PointerMoved(position),
                    middle(position, true),
                    middle(position, false),
                ],
            );
            assert_eq!(
                requests.len(),
                1,
                "a genuinely new focused click remains usable"
            );
            assert_eq!(
                requests[0].target(),
                crate::ClosePlanTarget::Item { item: SECOND }
            );
        }
    }
}
