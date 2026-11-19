//! Document placement and its notification share physical, visible facts.

use super::*;
use crate::geometry::PhysicalSize;

fn fixture() -> (ViewportRegistry, ViewportBinding) {
    let mut registry = ViewportRegistry::default();
    let binding = registry
        .register_existing(
            WorkspaceEpoch::new(0),
            SurfaceId::new(1),
            WindowToken::new(10),
            ViewportRole::Root,
        )
        .unwrap();
    (registry, binding)
}

fn physical_window(
    binding: ViewportBinding,
    generation: u64,
    state: WindowPresentationState,
    outer: PhysicalRect,
    size: PhysicalSize,
    scale: ScaleFactor,
) -> ObservedWindow {
    ObservedWindow::new(binding)
        .with_coordinate_observation(
            WindowCoordinateObservation::new(
                binding,
                CoordinateObservationGeneration::new(generation),
                Authority::Unknown(AuthorityUnavailableReason::NotReported),
                Authority::Known(outer),
                Authority::Known(scale),
                Authority::Unknown(AuthorityUnavailableReason::NotReported),
            )
            .with_content_size(Authority::Known(size)),
        )
        .with_presentation_observation(presentation_observation(
            binding,
            generation,
            Authority::Known(state),
        ))
}

#[test]
fn physical_placement_changes_notify_without_granting_render_authority() {
    let (mut registry, binding) = fixture();
    let outer = PhysicalRect::new(10.0, 20.0, 820.0, 640.0).unwrap();
    let moved = PhysicalRect::new(30.0, 40.0, 820.0, 640.0).unwrap();
    let size = PhysicalSize::new(800.0, 600.0).unwrap();
    let resized = PhysicalSize::new(780.0, 580.0).unwrap();
    let scale = ScaleFactor::new(1.0).unwrap();
    let rescaled = ScaleFactor::new(2.0).unwrap();
    for (index, (outer, size, scale, changed)) in [
        (outer, size, scale, true),
        (outer, size, scale, false),
        (outer, resized, scale, true),
        (outer, resized, rescaled, true),
        (moved, resized, rescaled, true),
        (moved, resized, rescaled, false),
    ]
    .into_iter()
    .enumerate()
    {
        let generation = index as u64 + 1;
        let transition = registry
            .apply_snapshot_for_test(&snapshot(
                generation,
                vec![physical_window(
                    binding,
                    generation,
                    WindowPresentationState::Visible,
                    outer,
                    size,
                    scale,
                )],
            ))
            .unwrap();
        assert_eq!(
            transition.native_window_placement_changed(),
            changed,
            "physical change {index} must ignore transport generations"
        );
        let observation = registry
            .window(binding.window())
            .unwrap()
            .last_visible_placement()
            .unwrap();
        assert_eq!(observation.outer_rect, outer);
        assert_eq!(observation.inner_size, Some(size));
        assert_eq!(observation.scale_factor, Some(scale));
        let record = registry.record(binding.surface()).unwrap();
        assert!(record.coordinates().is_none());
        assert!(!record.has_coordinate_authority());
        assert!(!record.is_routeable());
    }
}

#[test]
fn physical_placement_retains_the_last_visible_bounds_without_notifying_again() {
    let (mut registry, binding) = fixture();
    let initial = PhysicalRect::new(10.0, 20.0, 820.0, 640.0).unwrap();
    let moved = PhysicalRect::new(30.0, 40.0, 820.0, 640.0).unwrap();
    let size = PhysicalSize::new(800.0, 600.0).unwrap();
    let scale = ScaleFactor::new(1.0).unwrap();
    let windows = [
        Some((1, WindowPresentationState::Visible, initial)),
        Some((2, WindowPresentationState::Hidden, moved)),
        Some((3, WindowPresentationState::Minimized, moved)),
        Some((4, WindowPresentationState::Visible, moved)),
        None,
        Some((5, WindowPresentationState::Visible, moved)),
        Some((6, WindowPresentationState::Visible, moved)),
    ];
    for (index, (window, (changed, bounds))) in windows
        .into_iter()
        .zip([
            (true, initial),
            (false, initial),
            (false, initial),
            (true, moved),
            (false, moved),
            (false, moved),
            (false, moved),
        ])
        .enumerate()
    {
        let observation = window.map(|(generation, state, outer)| {
            physical_window(binding, generation, state, outer, size, scale)
        });
        let transition = registry
            .apply_snapshot_for_test(&snapshot(
                index as u64 + 1,
                observation.into_iter().collect(),
            ))
            .unwrap();
        assert_eq!(transition.native_window_placement_changed(), changed);
        assert_eq!(
            registry
                .window(binding.window())
                .unwrap()
                .last_visible_placement()
                .map(|placement| placement.outer_rect),
            Some(bounds)
        );
    }

    registry.mark_awaiting_destroyed(binding).unwrap();
    let transition = registry
        .apply_snapshot_for_test(&snapshot(
            8,
            vec![physical_window(
                binding,
                7,
                WindowPresentationState::Visible,
                initial,
                size,
                scale,
            )],
        ))
        .unwrap();
    assert!(!transition.native_window_placement_changed());
    assert!(
        registry
            .window(binding.window())
            .unwrap()
            .last_visible_placement()
            .is_none()
    );
}
