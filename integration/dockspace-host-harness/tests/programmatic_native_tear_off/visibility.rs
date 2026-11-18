//! Visibility changes in an admitted native tear-off.

use super::*;

#[test]
fn hiding_and_showing_all_child_tabs_keeps_the_original_native_window() {
    let mut fixture = Fixture::new();
    let group = fixture
        .session
        .capture_tabs_group(DockAnchor::Item(A))
        .unwrap();
    let action = fixture
        .session
        .native_window_command(DockMoveSource::Tabs(group), placement())
        .unwrap()
        .into_prepared()
        .unwrap();
    let mut request = fixture.request(action);
    let acknowledgement = request.accept();
    fixture.observe(
        &request,
        NativeCreateReadiness::Created,
        Some(acknowledgement),
    );
    fixture.observe(&request, NativeCreateReadiness::Prepared, None);
    let session = &mut fixture.session;
    let child = request.binding;
    let surface = child.surface();
    let output = paint(session, child, request.placement);
    session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    assert_eq!(idle(session).native_admissions(), &[child]);
    let original_placement = session.native_surface_placement(child).unwrap();
    let metrics = UniformSurfaceMetrics::new(
        LogicalRect::new(
            0.0,
            0.0,
            request.placement.width(),
            request.placement.height(),
        )
        .unwrap(),
        LogicalSize::new(32.0, 24.0).unwrap(),
        80.0,
    )
    .unwrap();

    let mut previous_receivers = [None; 2];
    for (change, visible, selected) in [
        (None, [true, true], Some(B)),
        (Some((B, false)), [true, false], Some(A)),
        (Some((A, false)), [false, false], None),
        (Some((A, true)), [true, false], Some(A)),
        (Some((B, true)), [true, true], Some(A)),
    ] {
        let mut frame = native_frame(session);
        let action = change.map(|(item, visible)| {
            (
                frame.set_item_visibility_current(item, visible).unwrap(),
                item,
                visible,
            )
        });
        let mut report = finish(frame);
        if let Some((action, item, visible)) = action {
            assert!(matches!(
                report.submitted_action_outcome(action),
                Some(HostInputOutcome::ProductActionApplied(
                    DockspaceActionOutcome::ItemVisibilityChanged {
                        item: changed, visible: shown, changed: true,
                    },
                )) if (*changed, *shown) == (item, visible)
            ));
            if !visible {
                let index = usize::from(item == B);
                let old = previous_receivers[index].as_ref().unwrap();
                assert!(
                    session.bind_presented_receiver(old).is_none(),
                    "the previous output must immediately lose authority for hidden {item:?}"
                );
            }
        }
        assert!(report.native_admissions().is_empty());
        assert!(report.take_native_effects().is_empty());

        let mut frame = native_frame(session);
        frame.measure_surface(surface, metrics).unwrap();
        let plan = frame.paint_plan(surface).unwrap().unwrap();
        let receivers = [A, B].map(|item| plan.tab_receiver(item));
        assert_eq!(receivers.map(|receiver| receiver.is_some()), visible);
        let (recording, receipt) = RecordingRenderer::execute(plan).unwrap();
        let painted = recording.records.iter().filter_map(|record| match record {
            OwnedRecord::PaneContent { item, .. } => Some(*item),
            _ => None,
        });
        assert!(painted.eq(selected));
        frame.confirm_surface_painted(receipt).unwrap();
        let mut report = finish(frame);
        assert!(report.take_native_effects().is_empty());
        let [output] = report.take_painted_outputs().try_into().unwrap();
        assert!(output.matches_native_binding(child));
        let artifact = output.presentation_artifact();
        session
            .report_surface_presentation(output, SurfacePresentationResult::Presented)
            .unwrap();
        let mut settled = idle(session);
        assert!(
            settled
                .surface_presentation_commits()
                .iter()
                .any(|commit| commit.presents(artifact))
        );
        assert!(settled.take_native_effects().is_empty());
        assert!(settled.native_admissions().is_empty());
        assert!(session.is_current_native_binding(child));
        assert_eq!(
            session.native_surface_placement(child),
            Some(original_placement)
        );
        assert_eq!(session.native_window_bindings().count(), 2);
        assert!(
            session
                .native_surface_bindings_for_window(child.window())
                .eq([child])
        );

        let view = session.view();
        for (item, visible) in [A, B].into_iter().zip(visible) {
            let location = view.item(item).unwrap();
            assert_eq!(
                (location.root(), location.surface()),
                (request.root, surface)
            );
            assert_eq!(location.is_visible(), visible);
        }
        let tabs = view.item(A).unwrap().tabs().unwrap();
        assert!(tabs.items().eq([A, B]));
        assert_eq!(tabs.visible_selected().and_then(|tab| tab.item()), selected);
        for receiver in receivers.iter().flatten() {
            assert!(session.bind_presented_receiver(receiver).is_some());
        }
        for receiver in previous_receivers.iter().flatten() {
            assert!(session.bind_presented_receiver(receiver).is_none());
        }
        previous_receivers = receivers;
    }
}
