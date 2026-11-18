//! Native renderer journeys retain the shared motion and output contract.

use super::*;
use dockspace::motion::MotionMode;

mod harness;
use harness::{FIRST, Harness, MIDDLE};

#[test]
fn native_renderer_hides_immediately_and_animates_survivors_then_restores_the_partition() {
    let mut harness = Harness::new();
    let before = harness.paint(50, false).unwrap();
    let original = before.poses[0].panes.clone();
    harness.present(before);
    let harness_state_receipt = harness.visibility(false);
    let began = harness.paint(100, false).unwrap();
    assert!(harness.active());
    assert!(!began.poses[0].partition);
    assert_eq!(began.poses[0].panes[&FIRST], original[&FIRST]);
    assert!(!began.poses[0].panes.contains_key(&MIDDLE));
    assert!(!harness.state.panes.painted.contains(&MIDDLE));
    harness.present(began);
    harness_state_receipt.try_recv().ok().unwrap().unwrap();
    let midway = harness.paint(200, false).unwrap();
    assert!(midway.poses[0].panes[&FIRST].width() > original[&FIRST].width());
    let middle_width = midway.poses[0].panes[&FIRST].width();
    harness.present(midway);
    let settled = harness.paint(300, false).unwrap();
    assert!(!harness.active());
    assert!(settled.poses[0].partition);
    assert!(settled.poses[0].panes[&FIRST].width() > middle_width);
    assert!(!settled.decoration_active);
    harness.present(settled);

    let harness_state_receipt = harness.visibility(true);
    let shown = harness.paint(400, false).unwrap();
    assert!(harness.active());
    assert_eq!(shown.poses[0].panes[&MIDDLE], original[&MIDDLE]);
    assert!(harness.state.panes.painted.contains(&MIDDLE));
    harness.present(shown);
    harness_state_receipt.try_recv().ok().unwrap().unwrap();
    let restored = harness.paint(600, false).unwrap();
    assert!(!harness.active());
    assert_eq!(restored.poses[0].panes, original);
    harness.present(restored);
}

#[test]
fn native_repeated_passes_keep_one_clock_sample_even_when_the_host_clock_advances() {
    let mut harness = Harness::new();
    let harness_state_receipt = harness.visibility(false);
    let began = harness.paint(100, false).unwrap();
    harness.present(began);
    harness_state_receipt.try_recv().ok().unwrap().unwrap();
    let repeated = harness.paint(150, true).unwrap();
    assert_eq!(repeated.poses.len(), 2);
    assert_eq!(repeated.poses[0].motion, repeated.poses[1].motion);
    assert_eq!(repeated.poses[0].motion.now, Duration::from_millis(150));
    assert_eq!(repeated.poses[0].panes, repeated.poses[1].panes);
    harness.present(repeated);
}

#[test]
fn native_off_reduced_and_zero_egui_time_use_target_geometry_and_preserve_request_receipts() {
    for (off, reduced, zero_time) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut harness = Harness::new();
        let before_version = harness.state.coordinator.session().version();
        let before_config = harness
            .state
            .coordinator
            .session()
            .presentation_config()
            .clone();
        let configuration = NativeMotionConfiguration {
            settings: MotionSettings {
                mode: if off {
                    MotionMode::Off
                } else {
                    MotionMode::System
                },
                ..Default::default()
            },
            preference: if reduced {
                MotionPreference::Reduced
            } else {
                MotionPreference::Unknown
            },
        };
        let harness_state_receipt = harness.state.request_motion(configuration).unwrap();
        assert_eq!(
            harness.state.request_motion(configuration).map(|_| ()),
            Err(NativeActionRequestError::Busy)
        );
        let configured = harness.paint(50, false).unwrap();
        harness.present(configured);
        assert_eq!(harness.state.motion, configuration);
        assert_eq!(
            harness.state.coordinator.session().version(),
            before_version
        );
        assert_eq!(
            harness.state.coordinator.session().presentation_config(),
            &before_config
        );
        assert_eq!(
            harness_state_receipt.try_recv().ok().unwrap().unwrap(),
            NativeActionStatus::MotionChanged {
                changed: off || reduced
            }
        );
        if zero_time {
            harness
                .context
                .all_styles_mut(|style| style.animation_time = 0.0);
        }
        let harness_state_receipt = harness.visibility(false);
        let hidden = harness.paint(100, false).unwrap();
        assert!(!harness.active());
        assert!(hidden.poses[0].partition);
        assert!(!hidden.decoration_active);
        assert!(!hidden.poses[0].panes.contains_key(&MIDDLE));
        harness.present(hidden);
        harness_state_receipt.try_recv().unwrap().unwrap();
    }
}

#[test]
fn native_pending_output_does_not_paint_through_capacity_or_request_motion_repaints() {
    let mut harness = Harness::new();
    let harness_state_receipt = harness.visibility(false);
    let held = harness.paint(100, false).unwrap();
    assert!(harness.active());
    harness_state_receipt.try_recv().ok().unwrap().unwrap();
    for now in [120, 160, 220, 300] {
        assert!(harness.paint(now, false).is_none());
        assert!(!harness.context.has_requested_repaint());
        assert!(harness.state.error().is_none());
    }
    harness.present(held);
    let settled = harness.paint(400, false).unwrap();
    assert!(!harness.active());
    assert!(settled.poses[0].partition);
    assert!(!harness.context.has_requested_repaint());
    harness.present(settled);
}
