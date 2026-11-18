//! Application viewport creation stays outside managed lifetime admission.

use super::*;

#[test]
fn unrelated_deferred_viewport_creation_is_not_deferred_by_docking() {
    let binding = binding();
    let managed = crate::deferred_viewport::viewport_id_for(binding);
    let unrelated = ViewportId::from_hash_of("application-owned-preferences");
    let mut routes = NativeViewportMap::default();
    routes.reserve(managed, binding).unwrap();
    let (bridge, _) = bridge_with_clock(routes);
    for quarantine in [false, true] {
        if quarantine {
            bridge.enter_quarantine();
        }
        let state = bridge.lock();
        assert_eq!(
            state.unreserved_viewport_create_admission(unrelated),
            NativeViewportCreateAdmission::Proceed {
                requested_outer_rect: None,
            },
            "a Dockspace failure must not capture an application's independent viewport"
        );
        assert_eq!(
            state.unreserved_viewport_create_admission(managed),
            NativeViewportCreateAdmission::Defer,
            "a managed viewport still needs its original reserved create"
        );
    }
    assert!(bridge.lock().journal.is_empty());
}

#[test]
fn a_retired_managed_viewport_does_not_gain_unrelated_creation_permission() {
    let binding = binding();
    let managed = crate::deferred_viewport::viewport_id_for(binding);
    let mut routes = NativeViewportMap::default();
    routes.reserve(managed, binding).unwrap();
    assert!(routes.admit_test_create(managed, binding));
    let (bridge, _) = bridge_with_clock(routes);
    let failure = NativeViewportCreateFailureRecord::new(
        managed,
        binding,
        NativeViewportCreateFailureKind::WindowUnavailable,
    );
    assert!(bridge.lock().record_viewport_create_failure(failure));
    assert!(bridge.acknowledge_viewport_create_failure(failure, true));
    assert!(bridge.lock().retirements.has_quiescence_owner(binding));
    assert_eq!(
        bridge.lock().unreserved_viewport_create_admission(managed),
        NativeViewportCreateAdmission::Defer,
        "moving an old route into retirement must not admit a new physical lifetime"
    );
}

#[test]
fn replacement_create_failure_keeps_the_actual_viewport_managed_until_quiescence() {
    let predecessor = binding();
    let replacement = binding();
    let managed = crate::deferred_viewport::viewport_id_for(predecessor);
    let unused = crate::deferred_viewport::viewport_id_for(replacement);
    assert_ne!(managed, unused);
    let mut routes = NativeViewportMap::default();
    routes.reserve(managed, replacement).unwrap();
    assert!(routes.admit_test_create(managed, replacement));
    let (bridge, _) = bridge_with_clock(routes);
    let failure = NativeViewportCreateFailureRecord::new(
        managed,
        replacement,
        NativeViewportCreateFailureKind::WindowUnavailable,
    );
    assert!(bridge.lock().record_viewport_create_failure(failure));
    assert!(bridge.acknowledge_viewport_create_failure(failure, true));
    assert_eq!(
        bridge.with_viewports(|routes| routes.binding(managed)),
        None
    );
    assert!(bridge.lock().retirements.has_quiescence_owner(replacement));
    for quarantine in [false, true] {
        if quarantine {
            bridge.enter_quarantine();
        }
        let records = bridge.lock();
        assert_eq!(
            records.unreserved_viewport_create_admission(managed),
            NativeViewportCreateAdmission::Defer,
            "replacement failure must retain the predecessor's actual viewport"
        );
        assert_eq!(
            records.unreserved_viewport_create_admission(unused),
            NativeViewportCreateAdmission::Proceed {
                requested_outer_rect: None,
            },
            "a binding hash does not claim an undeclared viewport"
        );
    }
    bridge.commit_frame_boundary(false);
    assert_eq!(bridge.retirement_quiescence_candidates(), [replacement]);
    assert!(bridge.retirement_finish_quiescence(replacement));
    assert!(!bridge.manages_viewport(managed));
}

#[test]
fn cancelled_unstarted_replacement_keeps_its_actual_viewport_managed() {
    let replacement = binding();
    let managed = ViewportId::from_hash_of("cancelled-replacement-viewport");
    assert_ne!(
        managed,
        crate::deferred_viewport::viewport_id_for(replacement)
    );
    let (bridge, _) = bridge_with_clock(NativeViewportMap::default());
    bridge.reserve_viewport_route(managed, replacement).unwrap();
    assert!(bridge.retire_unstarted_viewport(managed, replacement));
    assert_eq!(
        bridge.with_viewports(|routes| routes.binding(managed)),
        None
    );
    assert_eq!(
        bridge.lock().unreserved_viewport_create_admission(managed),
        NativeViewportCreateAdmission::Defer,
        "retiring an unstarted replacement must not authorize a delayed create"
    );
    assert!(bridge.retirement_finish_quiescence(replacement));
    assert!(!bridge.manages_viewport(managed));
}

#[test]
fn retained_window_resources_keep_the_actual_viewport_managed_without_a_live_route() {
    let binding = binding();
    let managed = ViewportId::from_hash_of("retained-replacement-output");
    let unused = crate::deferred_viewport::viewport_id_for(binding);
    assert_ne!(managed, unused);
    let (bridge, _) = bridge_with_clock(NativeViewportMap::default());
    let mut records = bridge.lock();
    records
        .window_resources
        .prepare_host_reference(managed, binding)
        .unwrap();
    records
        .window_resources
        .commit_host_reference(managed, binding);
    records.window_resources.retire(managed, binding);
    assert_eq!(records.viewports.binding(managed), None);
    assert!(!records.retirements.has_pending_work());
    assert_eq!(
        records.unreserved_viewport_create_admission(managed),
        NativeViewportCreateAdmission::Defer,
        "an old output pin retains its original viewport, including after retirement"
    );
    assert_eq!(
        records.unreserved_viewport_create_admission(unused),
        NativeViewportCreateAdmission::Proceed {
            requested_outer_rect: None,
        }
    );
    records
        .window_resources
        .refresh_host_references(|_, _| false);
    assert!(!records.manages_viewport(managed));
}
