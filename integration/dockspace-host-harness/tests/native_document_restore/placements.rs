use super::*;
use dockspace::runtime::DockMoveSource;

fn saved_root_bounds(session: &mut DockspaceSession) -> PhysicalRect {
    let bytes = session.save_document_json().unwrap();
    let document = session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    document
        .surface_presentation_preferences()
        .find(|preference| preference.surface() == HOST)
        .unwrap()
        .outer_rect()
        .unwrap()
}

#[test]
fn restored_root_does_not_reuse_previous_binding_placement_before_fresh_observation() {
    let mut host = NativeDocumentHost::new(4, true);
    let saved = PhysicalRect::new(100.0, 80.0, 640.0, 480.0).unwrap();
    let moved = PhysicalRect::new(1100.0, 80.0, 640.0, 480.0).unwrap();
    assert_eq!(saved_root_bounds(&mut host.session), saved);
    let bytes = host.session.save_document_json().unwrap();
    host.session
        .report_managed_native_snapshot([(host.root, window_facts(moved))], work_areas())
        .unwrap();
    assert!(
        idle(&mut host.session)
            .notification()
            .native_window_placement_changed()
    );
    assert_eq!(saved_root_bounds(&mut host.session), moved);

    let restore = host
        .session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    let predecessor = host.root;
    let generation = host.session.next_document_generation();
    let mut rollback = native_frame(&mut host.session);
    rollback
        .submit_native_document_restore(&restore, HOST)
        .unwrap();
    rollback
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    drop(rollback);
    assert_eq!(host.session.next_document_generation(), generation);
    assert!(host.session.is_current_native_binding(predecessor));
    assert_eq!(saved_root_bounds(&mut host.session), moved);

    // The placement probe saves and advances document sidecars. Prepare a
    // fresh restore after that capture rather than reusing stale save authority.
    let restore = host
        .session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    let mut frame = native_frame(&mut host.session);
    frame
        .submit_native_document_restore(&restore, HOST)
        .unwrap();
    let mut report = finish(frame);
    assert_no_effects(&mut report);
    let [successor] = report.native_bindings() else {
        panic!("the restored root must receive one fresh native binding");
    };
    host.root = *successor;
    assert_ne!(host.root, predecessor);
    assert_eq!(saved_root_bounds(&mut host.session), saved);
    host.quiesce(predecessor);
    assert_eq!(saved_root_bounds(&mut host.session), saved);
    let version = host.session.version();

    host.session
        .report_managed_native_snapshot([(host.root, window_facts(moved))], work_areas())
        .unwrap();
    let mut rollback = native_frame(&mut host.session);
    rollback
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    drop(rollback);
    assert_eq!(saved_root_bounds(&mut host.session), saved);
    let report = idle(&mut host.session);
    assert!(report.notification().native_window_placement_changed());
    assert!(report.notification().document_changed());
    assert!(report.notification().layout_change().is_none());
    assert_eq!(host.session.version(), version);
    assert_eq!(saved_root_bounds(&mut host.session), moved);

    for state in [
        NativeWindowPresentationState::Minimized,
        NativeWindowPresentationState::Visible,
    ] {
        let bounds = if state == NativeWindowPresentationState::Minimized {
            PhysicalRect::new(5.0, 5.0, 160.0, 20.0).unwrap()
        } else {
            moved
        };
        host.session
            .report_managed_native_snapshot(
                [(host.root, window_facts(bounds).with_presentation(state))],
                work_areas(),
            )
            .unwrap();
        assert!(
            !idle(&mut host.session)
                .notification()
                .native_window_placement_changed()
        );
        assert_eq!(saved_root_bounds(&mut host.session), moved);
    }
}

#[test]
fn first_save_after_minimizing_a_torn_off_child_remains_restorable() {
    let mut host = NativeDocumentHost::new(8, true);
    let requested_bounds = placement(0);
    let action = host.session.prepare_move_to_new_window(
        DockMoveSource::Item(HOST_DETAIL),
        NativeWindowPlacement::new(requested_bounds),
    );
    let mut frame = native_frame(&mut host.session);
    let submitted = frame.submit_prepared_action(action).unwrap();
    let mut report = finish(frame);
    assert!(matches!(
        report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::ProductPresentationActionRequested { .. })
    ));
    let [create] = report.take_native_effects().try_into().unwrap();
    let NativeEffectOperation::CreateWindow { binding, .. } = *create.operation() else {
        panic!("the tear-off must issue a child create effect");
    };
    let Some(NativeEffectAcknowledgement::Create(ack)) = create.accepted() else {
        panic!("the child must retain its readiness acknowledgement");
    };
    let observed = PhysicalRect::new(-880.0, 140.0, 620.0, 460.0).unwrap();
    let facts = window_facts(observed).with_native_scale_factor(ScaleFactor::new(2.0).unwrap());
    host.session
        .report_managed_native_snapshot(
            [
                (host.root, root_facts()),
                (
                    binding,
                    facts
                        .clone()
                        .with_create_readiness(NativeCreateReadiness::Prepared, Some(ack)),
                ),
            ],
            work_areas(),
        )
        .unwrap();
    idle(&mut host.session);
    let (output, _) = paint(&mut host.session, binding);
    host.session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    assert_eq!(idle(&mut host.session).native_admissions(), &[binding]);

    let minimized = NativeWindowFacts::live()
        .with_outer_bounds(PhysicalRect::new(5.0, 5.0, 160.0, 20.0).unwrap())
        .with_presentation(NativeWindowPresentationState::Minimized);
    host.session
        .report_managed_native_snapshot(
            [(host.root, root_facts()), (binding, minimized)],
            work_areas(),
        )
        .unwrap();
    assert!(
        !idle(&mut host.session)
            .notification()
            .native_window_placement_changed()
    );
    assert!(host.session.native_local_coordinates(binding).is_none());

    let bytes = host.session.save_document_json().unwrap();
    let fresh = NativeDocumentHost::new(8, true);
    let restore = fresh
        .session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    let saved = restore
        .surface_presentation_preferences()
        .find(|preference| preference.surface() == binding.surface())
        .unwrap();
    assert_eq!(saved.outer_rect(), Some(observed));
    assert_eq!(
        saved.scale_factor_hint(),
        Some(ScaleFactor::new(2.0).unwrap())
    );
    restore.validate_native_surface_hosting(HOST).unwrap();
}

#[test]
fn restore_relocates_only_children_detached_from_every_known_work_area() {
    let bytes = saved_document(SavedHosting::Children);
    let scale = ScaleFactor::new(1.0).unwrap();
    let primary = NativeWorkAreaFacts::new(
        HostWorkAreaToken::new(1),
        PhysicalRect::new(0.0, 0.0, 1920.0, 1080.0).unwrap(),
        scale,
    );
    let overlap = NativeWorkAreaFacts::new(
        HostWorkAreaToken::new(2),
        PhysicalRect::new(-300.0, 0.0, 300.0, 1080.0).unwrap(),
        ScaleFactor::new(2.0).unwrap(),
    );
    let relocated = PhysicalRect::new(0.0, 120.0, 640.0, 480.0).unwrap();
    for (roster, expected) in [
        (NativeWorkAreaRoster::Exact(vec![primary]), relocated),
        (NativeWorkAreaRoster::Exact(vec![overlap]), placement(0)),
        (NativeWorkAreaRoster::Unknown, placement(0)),
    ] {
        let mut host = NativeDocumentHost::new(8, true);
        let predecessor = host.root;
        let (ids, _) = host.restore(&bytes);
        host.quiesce(predecessor);
        host.session
            .report_managed_native_snapshot([(host.root, root_facts())], roster)
            .unwrap();
        idle(&mut host.session);
        let (output, _) = paint(&mut host.session, host.root);
        host.session
            .report_surface_presentation(output, SurfacePresentationResult::Presented)
            .unwrap();
        idle(&mut host.session);

        let mut frame = native_frame(&mut host.session);
        assert!(frame.advance_next_native_surface_create().unwrap());
        let mut report = finish(frame);
        let [create] = report.take_native_effects().try_into().unwrap();
        let NativeEffectOperation::CreateWindow {
            binding, placement, ..
        } = *create.operation()
        else {
            panic!("expected a restored child request");
        };
        assert_eq!(binding.surface(), ids.children()[0]);
        assert_eq!(placement, expected);
        let _ = create.accepted();
    }
}
