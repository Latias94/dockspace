//! Same-frame native lifetimes and physical placement persistence.

use super::*;

#[test]
fn a_root_registered_and_restored_in_one_frame_can_release_its_predecessor_and_create_children() {
    let bytes = saved_document(SavedHosting::Children);
    let layout = DockspaceLayout::new([host_surface(ROOTS[0])]).unwrap();
    let mut session =
        DockspaceSession::from_persistent_layout(layout, policy(), bootstrap(true)).unwrap();
    session
        .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
        .unwrap();
    session
        .configure_managed_native_capabilities(capabilities())
        .unwrap();
    let restore = session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    session
        .register_native_root(HOST, HostWindowToken::new(44))
        .unwrap();
    let mut frame = native_frame(&mut session);
    frame
        .submit_native_document_restore(&restore, HOST)
        .unwrap();
    let mut report = finish(frame);
    let [continuation] = report.native_root_continuations() else {
        panic!("the Root must retain its physical window")
    };
    let predecessor = continuation.predecessor();
    let successor = continuation.successor();
    assert!(
        session.recognizes_native_binding(predecessor),
        "a same-frame predecessor must retain the producer lane required for cleanup"
    );
    session
        .report_native_binding_quiescence(predecessor)
        .unwrap();
    idle(&mut session);
    // Producer retirement must settle before dependent creates can advance.
    idle(&mut session);
    session
        .report_managed_native_snapshot([(successor, root_facts())], work_areas())
        .unwrap();
    let mut frame = native_frame(&mut session);
    assert!(
        frame.advance_next_native_surface_create().unwrap(),
        "the first Child must become creatable after exact predecessor cleanup"
    );
    report = finish(frame);
    let child = assert_content(session.view()).children()[0];
    let _ = take_create(&mut report, child, 0);
}

#[test]
fn physical_child_placement_roundtrips_without_a_presentation_scale() {
    let mut session =
        DockspaceSession::from_persistent_layout(document_layout(), policy(), bootstrap(true))
            .unwrap();
    session
        .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
        .unwrap();
    session
        .configure_managed_native_capabilities(capabilities())
        .unwrap();
    session
        .register_native_root(HOST, HostWindowToken::new(91))
        .unwrap();
    let root = idle(&mut session).native_bindings()[0];
    let mut rows = vec![(root, root_facts())];
    for (index, surface) in CHILDREN.into_iter().enumerate() {
        session
            .register_owned_native_child(surface, HostWindowToken::new(20 + index as u64), HOST)
            .unwrap();
        let report = idle(&mut session);
        let child = *report
            .native_bindings()
            .iter()
            .find(|binding| binding.surface() == surface)
            .unwrap();
        let facts = NativeWindowFacts::live()
            .with_outer_bounds(placement(index))
            .with_content_bounds(placement(index))
            .with_native_scale_factor(ScaleFactor::new(2.0).unwrap())
            .with_presentation(NativeWindowPresentationState::Visible);
        rows.push((child, facts));
    }
    session
        .report_managed_native_snapshot(rows.clone(), work_areas())
        .unwrap();
    idle(&mut session);
    for binding in session
        .native_window_bindings()
        .flat_map(|window| session.native_surface_bindings_for_window(window))
        .filter(|binding| binding.surface() != HOST)
    {
        assert!(
            session.native_local_coordinates(binding).is_none(),
            "physical placement must not fabricate render coordinates"
        );
    }
    let bytes = session.save_document_json().unwrap();
    let restore = session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    restore.validate_native_surface_hosting(HOST).unwrap();
    for (index, surface) in CHILDREN.into_iter().enumerate() {
        let preference = restore
            .surface_presentation_preferences()
            .find(|preference| preference.surface() == surface)
            .unwrap();
        assert_eq!(preference.outer_rect(), Some(placement(index)));
        assert_eq!(
            preference.scale_factor_hint(),
            Some(ScaleFactor::new(2.0).unwrap())
        );
    }

    // Every window has already been observed and saved. Only this child's
    // physical facts change, so another window cannot supply the dirty signal.
    session
        .report_managed_native_snapshot(rows.clone(), work_areas())
        .unwrap();
    assert!(idle(&mut session).notification().is_empty());
    let version = session.version();
    let updated = PhysicalRect::new(-640.0, 160.0, 720.0, 520.0).unwrap();
    let scale = ScaleFactor::new(1.5).unwrap();
    rows[1].1 = NativeWindowFacts::live()
        .with_outer_bounds(updated)
        .with_content_bounds(updated)
        .with_native_scale_factor(scale)
        .with_presentation(NativeWindowPresentationState::Visible);
    session
        .report_managed_native_snapshot(rows.clone(), work_areas())
        .unwrap();
    let changed = idle(&mut session);
    assert!(
        changed.notification().native_window_placement_changed(),
        "a persistable physical-only update must notify applications that debounce saves"
    );
    assert!(changed.notification().document_changed());
    assert!(changed.notification().layout_change().is_none());
    assert_eq!(session.version(), version);
    assert!(session.native_local_coordinates(rows[1].0).is_none());
    let bytes = session.save_document_json().unwrap();
    let restore = session
        .prepare_document_restore_json(&bytes, recognize)
        .unwrap();
    let preference = restore
        .surface_presentation_preferences()
        .find(|preference| preference.surface() == CHILDREN[0])
        .unwrap();
    assert_eq!(preference.outer_rect(), Some(updated));
    assert_eq!(preference.inner_size_hint(), Some(updated.size()));
    assert_eq!(preference.scale_factor_hint(), Some(scale));

    session
        .report_managed_native_snapshot(rows, work_areas())
        .unwrap();
    assert!(idle(&mut session).notification().is_empty());
    assert!(idle(&mut session).notification().is_empty());
}
