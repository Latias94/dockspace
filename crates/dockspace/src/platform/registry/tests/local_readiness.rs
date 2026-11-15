use super::*;
use crate::platform::NativeLocalReadiness;

fn fixture() -> (ViewportRegistry, ViewportBinding) {
    let mut registry = ViewportRegistry::default();
    let binding = registry
        .register_existing(
            WorkspaceEpoch::new(0),
            SurfaceId::new(1),
            WindowToken::new(3),
            ViewportRole::Root,
        )
        .unwrap();
    (registry, binding)
}

fn local_observation(
    binding: ViewportBinding,
    generation: u64,
    readiness: Authority<NativeLocalReadiness>,
) -> WindowPresentationObservation {
    presentation_observation(
        binding,
        generation,
        Authority::Unknown(AuthorityUnavailableReason::NotReported),
    )
    .with_local_readiness(readiness)
}

#[test]
fn local_readiness_does_not_claim_os_visibility_or_desktop_authority() {
    let (mut registry, binding) = fixture();
    let observation = local_observation(binding, 1, Authority::Known(NativeLocalReadiness::Ready));
    registry
        .apply_snapshot_for_test(&snapshot(
            1,
            vec![ready_window(binding, 1).with_presentation_observation(observation)],
        ))
        .unwrap();
    let record = registry.record(binding.surface()).unwrap();
    assert_eq!(
        record.presentation_observation().unwrap().known_state(),
        None
    );
    assert!(record.has_local_coordinate_authority());
    assert!(record.is_locally_routeable());
    assert!(!record.has_coordinate_authority());
    assert!(!record.is_routeable());
    assert!(!record.can_accept_activation());
    assert!(
        record.can_observe_focus(),
        "real focus callbacks remain valid without native visibility"
    );
    registry
        .set_surface_placements(binding.window(), &shared_placements(), |_| false)
        .unwrap();
    for (_, sibling) in registry.records() {
        assert!(sibling.is_locally_routeable());
        assert!(!sibling.is_routeable());
        assert!(!sibling.has_coordinate_authority());
    }
}

#[test]
fn local_readiness_withdrawal_invalidates_old_coordinates_even_when_geometry_is_unchanged() {
    let (mut registry, binding) = fixture();
    let mut previous = None;
    for (generation, readiness) in [
        (1, Some(NativeLocalReadiness::Ready)),
        (2, Some(NativeLocalReadiness::Ready)),
        (3, None),
        (4, Some(NativeLocalReadiness::Ready)),
        (5, Some(NativeLocalReadiness::Unavailable)),
        (6, Some(NativeLocalReadiness::Ready)),
    ] {
        let readiness = readiness.map_or(
            Authority::Unknown(AuthorityUnavailableReason::NotReported),
            Authority::Known,
        );
        let observation = local_observation(binding, generation, readiness);
        registry
            .apply_snapshot_for_test(&snapshot(
                generation,
                vec![ready_window(binding, generation).with_presentation_observation(observation)],
            ))
            .unwrap();
        let record = registry.record(binding.surface()).unwrap();
        assert_eq!(
            record.is_locally_routeable(),
            matches!(readiness, Authority::Known(NativeLocalReadiness::Ready))
        );
        let window = registry
            .window_coordinates(binding.window())
            .unwrap()
            .coordinate_generation();
        let current = (record.coordinate_generation(), window);
        if let Some(previous) = previous {
            if generation == 2 {
                assert_eq!(
                    current, previous,
                    "an equivalent snapshot retains its authority"
                );
            } else {
                assert_ne!(
                    current.0, previous.0,
                    "local admission changes revoke every placement"
                );
                assert_ne!(
                    current.1, previous.1,
                    "local admission changes revoke the window tuple"
                );
            }
        }
        previous = Some(current);
    }
}

#[test]
fn explicit_negative_facts_override_a_ready_local_host() {
    for state in [
        WindowPresentationState::Hidden,
        WindowPresentationState::Minimized,
    ] {
        let (mut registry, binding) = fixture();
        let observation = presentation_observation(binding, 1, Authority::Known(state))
            .with_local_readiness(Authority::Known(NativeLocalReadiness::Ready));
        registry
            .apply_snapshot_for_test(&snapshot(
                1,
                vec![ready_window(binding, 1).with_presentation_observation(observation)],
            ))
            .unwrap();
        let record = registry.record(binding.surface()).unwrap();
        assert!(!record.has_local_coordinate_authority());
        assert!(!record.is_locally_routeable());
        assert!(!record.is_routeable());
        assert!(!record.can_observe_focus());
    }
    let (mut registry, binding) = fixture();
    let observation = presentation_observation(
        binding,
        1,
        Authority::Known(WindowPresentationState::Visible),
    )
    .with_local_readiness(Authority::Known(NativeLocalReadiness::Unavailable));
    registry
        .apply_snapshot_for_test(&snapshot(
            1,
            vec![ready_window(binding, 1).with_presentation_observation(observation)],
        ))
        .unwrap();
    let record = registry.record(binding.surface()).unwrap();
    assert!(
        record.has_coordinate_authority(),
        "OS geometry remains available for persistence and cleanup"
    );
    assert!(!record.has_local_coordinate_authority());
    assert!(!record.is_locally_routeable());
    assert!(!record.is_routeable());
    assert!(!record.can_observe_focus());
}

#[test]
fn local_readiness_conflicts_require_a_complete_unknown_tombstone() {
    let (_, binding) = fixture();
    let mut stream = WindowPresentationObservationStream::default();
    let ready = |generation| {
        local_observation(
            binding,
            generation,
            Authority::Known(NativeLocalReadiness::Ready),
        )
    };
    stream.observe(binding, Some(ready(1)));
    assert!(stream.current().unwrap().allows_local_work());
    stream.observe(
        binding,
        Some(local_observation(
            binding,
            1,
            Authority::Known(NativeLocalReadiness::Unavailable),
        )),
    );
    assert!(stream.current().is_none());
    stream.observe(binding, Some(ready(0)));
    assert!(
        stream.current().is_none(),
        "an older envelope cannot repair a conflict"
    );
    stream.observe(binding, Some(ready(2)));
    assert!(
        stream.current().is_none(),
        "unknown visibility alone is not a local tombstone"
    );
    stream.observe(
        binding,
        Some(local_observation(
            binding,
            3,
            Authority::Unknown(AuthorityUnavailableReason::NotReported),
        )),
    );
    stream.observe(binding, Some(ready(4)));
    assert!(stream.current().unwrap().allows_local_work());
    assert_eq!(stream.current().unwrap().known_state(), None);
}
