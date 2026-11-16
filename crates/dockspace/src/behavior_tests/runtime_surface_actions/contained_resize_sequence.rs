//! Cardinal-edge batches reuse the same constraints as separate semantic actions.

use super::*;

fn apply(
    session: &mut DockspaceSession,
    direction: ContainedResizeDirection,
    steps: &[SurfaceContainedResizeAdjustment],
    deferred: bool,
) {
    let mut frame = session.begin_host_frame().unwrap();
    let action = frame
        .paint_plan(SURFACE)
        .unwrap()
        .unwrap()
        .prepare_contained_resize_adjustment_sequence(FLOATING, direction, steps.iter().copied())
        .expect("the exact cardinal edge prepares one sequence action");
    if deferred {
        frame
            .submit_deferred_surface_action(action.into_deferred())
            .unwrap();
    } else {
        frame.submit_surface_action(action).unwrap();
    }
    frame.measure_surface(SURFACE, metrics()).unwrap();
    frame.commit().unwrap();
}

#[test]
fn contained_resize_sequence_preserves_repetition_round_trips_and_endpoint_order() {
    use SurfaceContainedResizeAdjustment::{Decrement, Increment};
    let sequences = [
        vec![Increment, Increment],
        vec![Increment, Decrement],
        std::iter::repeat_n(Increment, 64)
            .chain([Decrement])
            .collect(),
        std::iter::repeat_n(Decrement, 64)
            .chain([Increment])
            .collect(),
    ];
    for direction in [
        ContainedResizeDirection::North,
        ContainedResizeDirection::East,
        ContainedResizeDirection::South,
        ContainedResizeDirection::West,
    ] {
        for deferred in [false, true] {
            for sequence in &sequences {
                let mut separate = contained_session(DockPolicy::default());
                install_ready_candidate(&mut separate);
                for step in sequence {
                    apply(&mut separate, direction, &[*step], deferred);
                }
                let expected = contained_rect(&separate);
                let mut batched = contained_session(DockPolicy::default());
                install_ready_candidate(&mut batched);
                apply(&mut batched, direction, sequence, deferred);
                assert_eq!(
                    contained_rect(&batched),
                    expected,
                    "{direction:?}, deferred={deferred}, sequence={sequence:?}"
                );
            }
        }
    }
}

#[test]
fn contained_resize_sequence_keeps_placement_after_normalizing_an_offscreen_source() {
    let fixture = || {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs([FIRST])),
        )
        .with_contained(DockspaceContainedLayout::new(
            FLOATING,
            DockspaceRootLayout::new(FLOATING_ROOT, DockspaceNode::tabs([SECOND])),
            LogicalRect::new(160.0, 500.0, 320.0, 200.0).unwrap(),
        ))])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        install_ready_candidate(&mut session);
        session
    };
    let steps = [SurfaceContainedResizeAdjustment::Increment; 2];
    for deferred in [false, true] {
        let mut separate = fixture();
        for step in steps {
            apply(
                &mut separate,
                ContainedResizeDirection::North,
                &[step],
                deferred,
            );
        }
        let expected = contained_rect(&separate);
        assert_eq!(
            expected,
            LogicalRect::new(160.0, 432.0, 320.0, 168.0).unwrap()
        );
        let mut batched = fixture();
        apply(
            &mut batched,
            ContainedResizeDirection::North,
            &steps,
            deferred,
        );
        assert_eq!(contained_rect(&batched), expected);
    }
}

#[test]
fn contained_resize_sequence_rejects_empty_diagonal_and_policy_denied_actions() {
    for enabled in [false, true] {
        let mut policy = DockPolicy::default();
        policy.set_allow_contained_transform(enabled);
        let mut session = contained_session(policy);
        install_ready_candidate(&mut session);
        inspect_surface(&mut session, |plan| {
            assert!(
                plan.prepare_contained_resize_adjustment_sequence(
                    FLOATING,
                    ContainedResizeDirection::East,
                    [],
                )
                .is_none()
            );
            assert!(
                plan.prepare_contained_resize_adjustment_sequence(
                    FLOATING,
                    ContainedResizeDirection::NorthEast,
                    [SurfaceContainedResizeAdjustment::Increment; 2],
                )
                .is_none()
            );
            assert_eq!(
                plan.prepare_contained_resize_adjustment_sequence(
                    FLOATING,
                    ContainedResizeDirection::East,
                    [SurfaceContainedResizeAdjustment::Increment; 2],
                )
                .is_some(),
                enabled
            );
        });
    }
}
