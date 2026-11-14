//! Last-roster capture uses committed lifetime evidence, never host detachment.

use super::*;
use crate::work_area::FrozenWorkAreaRoster;

fn commit_baseline(bridge: &NativeHostBridge) {
    bridge.push_viewport_roster(
        NativeViewportRosterRecord::for_test_with_backend_and_work_areas(
            [],
            FrozenWorkAreaRoster::Unknown,
            eframe::NativeWindowingBackend::MacOs,
        ),
    );
    let roster = bridge.front_viewport_roster().unwrap();
    assert!(bridge.acknowledge_viewport_roster(&roster));
    bridge.commit_frame_boundary(false);
    assert!(bridge.lock().settle_viewport_roster(&roster, true));
}

fn observe_destroyed(bridge: &NativeHostBridge, binding: NativeSurfaceBinding, ordinal: u64) {
    let window = WindowId::from(11);
    bridge.push_record(destroyed(ordinal, window));
    assert_eq!(
        bridge.retirement_observe_destroyed(ViewportId::ROOT, binding, window, ordinal),
        DestroyedObservation::Recorded
    );
    assert!(bridge.acknowledge_event(ordinal));
}

#[test]
fn attached_host_can_capture_last_roster_only_after_full_destroyed_commit() {
    let binding = binding();
    let mut routes = NativeViewportMap::default();
    routes
        .bind(ViewportId::ROOT, WindowId::from(11), binding)
        .unwrap();
    let (bridge, _) = bridge_with_clock(routes);
    bridge.enable_observation_attachment_for_test();
    commit_baseline(&bridge);
    bridge.enter_quarantine();
    bridge.require_terminal_roster();
    assert!(
        !bridge.capture_destroyed_window_roster(),
        "a live window cannot disappear from the inventory"
    );
    bridge.push_record(destroyed(100, WindowId::from(11)));
    assert!(
        !bridge.capture_destroyed_window_roster(),
        "Received is not committed destruction"
    );
    assert_eq!(
        bridge.retirement_observe_destroyed(ViewportId::ROOT, binding, WindowId::from(11), 100),
        DestroyedObservation::Recorded
    );
    assert!(bridge.acknowledge_event(100));
    assert!(
        !bridge.capture_destroyed_window_roster(),
        "transferred ingress retains its obligation"
    );
    bridge.commit_frame_boundary(true);
    assert!(!bridge.capture_destroyed_window_roster());
    bridge.commit_frame_boundary(false);
    let reserved = bridge
        .lock()
        .terminal_roster
        .reserved_payload_capacity_bytes();
    assert_eq!(bridge.lock().attachment, HostAttachmentState::Attached);
    assert!(bridge.capture_destroyed_window_roster());
    assert!(
        !bridge.capture_destroyed_window_roster(),
        "one requirement owns one terminal receipt"
    );
    let roster = bridge.front_viewport_roster().unwrap();
    assert!(roster.roster().observations().is_empty());
    assert_eq!(
        roster.roster().backend(),
        eframe::NativeWindowingBackend::MacOs
    );
    assert_eq!(
        roster.roster().work_areas().unwrap(),
        &FrozenWorkAreaRoster::Unknown
    );
    assert_eq!(
        bridge.retirement_destroyed_observations(),
        [(binding, NativeWindowFacts::destroyed())]
    );
    assert!(bridge.acknowledge_viewport_roster(&roster));
    assert!(bridge.lock().terminal_roster.has_pending());
    bridge.commit_frame_boundary(false);
    assert!(
        bridge.lock().terminal_roster.has_pending(),
        "raw commit alone does not settle the native snapshot"
    );
    assert!(bridge.lock().settle_viewport_roster(&roster, false));
    assert!(
        bridge.capture_destroyed_window_roster(),
        "an abandoned candidate must recapture"
    );
    let retry = bridge.front_viewport_roster().unwrap();
    assert!(bridge.acknowledge_viewport_roster(&retry));
    assert!(bridge.lock().settle_viewport_roster(&retry, true));
    assert!(
        bridge.lock().terminal_roster.has_pending(),
        "settlement alone does not commit the ingress"
    );
    bridge.commit_frame_boundary(false);
    assert!(!bridge.lock().terminal_roster.has_pending());
    assert_eq!(
        bridge
            .lock()
            .terminal_roster
            .reserved_payload_capacity_bytes(),
        reserved
    );
}

#[test]
fn detachment_does_not_prove_destruction_and_missing_backend_stays_unknown() {
    for baseline in [false, true] {
        let binding = binding();
        let mut routes = NativeViewportMap::default();
        routes
            .bind(ViewportId::ROOT, WindowId::from(11), binding)
            .unwrap();
        let (bridge, _) = bridge_with_clock(routes);
        if baseline {
            commit_baseline(&bridge);
        }
        bridge.enable_observation_attachment_for_test();
        bridge.lock().attachment.detach();
        bridge.enter_quarantine();
        bridge.require_terminal_roster();
        assert!(!bridge.capture_destroyed_window_roster());
        observe_destroyed(&bridge, binding, 100);
        bridge.commit_frame_boundary(false);
        assert_eq!(bridge.capture_destroyed_window_roster(), baseline);
        assert!(bridge.lock().terminal_roster.has_pending());
    }
}

#[test]
fn empty_window_inventory_waits_for_unmaterialized_create_failure_commit() {
    let binding = binding();
    let mut routes = NativeViewportMap::default();
    routes.reserve(ViewportId::ROOT, binding).unwrap();
    assert!(routes.admit_test_create(ViewportId::ROOT, binding));
    let (bridge, _) = bridge_with_clock(routes);
    commit_baseline(&bridge);
    bridge.enter_quarantine();
    bridge.require_terminal_roster();
    assert!(
        !bridge.capture_destroyed_window_roster(),
        "an admitted create can still materialize"
    );
    let failure = NativeViewportCreateFailureRecord::new(
        ViewportId::ROOT,
        binding,
        NativeViewportCreateFailureKind::WindowUnavailable,
    );
    assert!(bridge.lock().record_viewport_create_failure(failure));
    assert!(!bridge.capture_destroyed_window_roster());
    assert!(bridge.acknowledge_viewport_create_failure(failure, true));
    assert!(!bridge.capture_destroyed_window_roster());
    bridge.commit_frame_boundary(true);
    assert!(!bridge.capture_destroyed_window_roster());
    bridge.commit_frame_boundary(false);
    assert!(bridge.capture_destroyed_window_roster());
}

#[test]
fn last_roster_waits_for_a_free_prepaid_backing_and_retries_without_growth() {
    let binding = binding();
    let mut routes = NativeViewportMap::default();
    routes
        .bind(ViewportId::ROOT, WindowId::from(11), binding)
        .unwrap();
    let (bridge, _) = bridge_with_clock(routes);
    // Both existing payloads can be held by an old baseline and a dropped
    // candidate's caller. Recovery must not allocate a third payload.
    let first = {
        let mut records = bridge.lock();
        records.terminal_roster.require();
        let first = records
            .terminal_roster
            .capture_empty(eframe::NativeWindowingBackend::MacOs)
            .unwrap();
        assert!(records.record_terminal_viewport_roster(Arc::clone(&first)));
        first
    };
    let baseline = bridge.front_viewport_roster().unwrap();
    assert!(bridge.acknowledge_viewport_roster(&baseline));
    bridge.commit_frame_boundary(false);
    assert!(bridge.lock().settle_viewport_roster(&baseline, true));
    let second = {
        let mut records = bridge.lock();
        records.terminal_roster.require();
        records
            .terminal_roster
            .capture_empty(eframe::NativeWindowingBackend::MacOs)
            .unwrap()
    };
    let second_identity = Arc::as_ptr(&second);
    let reserved = bridge
        .lock()
        .terminal_roster
        .reserved_payload_capacity_bytes();
    bridge.enter_quarantine();
    observe_destroyed(&bridge, binding, 100);
    bridge.commit_frame_boundary(false);
    let captured = bridge.lock().journal.captured_through();
    assert!(!bridge.capture_destroyed_window_roster());
    assert_eq!(bridge.lock().journal.captured_through(), captured);
    assert_eq!(
        bridge
            .lock()
            .terminal_roster
            .reserved_payload_capacity_bytes(),
        reserved
    );
    assert!(bridge.lock().terminal_roster.needs_capture());
    drop(second);
    assert!(bridge.capture_destroyed_window_roster());
    assert!(!bridge.capture_destroyed_window_roster());
    let terminal = bridge.front_viewport_roster().unwrap();
    assert_eq!(Arc::as_ptr(terminal.shared_roster()), second_identity);
    assert!(!Arc::ptr_eq(terminal.shared_roster(), &first));
    assert_eq!(
        bridge
            .lock()
            .terminal_roster
            .reserved_payload_capacity_bytes(),
        reserved
    );
}
