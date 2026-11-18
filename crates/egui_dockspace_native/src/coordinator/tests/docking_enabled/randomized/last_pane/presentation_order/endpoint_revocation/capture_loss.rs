use super::*;

#[test]
fn capture_loss_then_target_dpi_change_rejects_a_late_cross_window_release() {
    for presentation_first in [false, true] {
        for result in [
            SurfacePresentationResult::Presented,
            SurfacePresentationResult::Dropped,
        ] {
            let mut fixture = dragging(0xca97_11e);
            fixture
                .trace
                .event(("late release", presentation_first, result));
            let stale = hold_target_preview(&mut fixture.native, fixture.host);
            let version = fixture.native.session.version();
            let (preview, lost_capture) = apply(
                &mut fixture.native,
                fixture.child,
                WindowEvent::PointerCaptureChanged {
                    device_id: DeviceId::dummy(),
                    capture: PointerWindowRoute::None,
                },
                &fixture.receivers,
                true,
                &mut fixture.trace,
            );
            let [cancelled] = lost_capture.manipulation_feedback() else {
                panic!("{}: {lost_capture:?}", fixture.trace.replay());
            };
            assert_eq!(
                cancelled.status(),
                Status::Cancelled(CancelReason::CaptureLost)
            );
            assert!(preview.is_none());
            assert!(!lost_capture.workspace_changed());
            assert!(!fixture.native.session.has_active_gesture());
            assert_child_retained(&fixture.native, fixture.child);

            let change = WindowChange::NativeDpi;
            fixture
                .native
                .report_snapshot(
                    [
                        (fixture.host, change.facts(0.0)),
                        (fixture.child, ready_window_facts(720.0)),
                    ],
                    NativeWorkAreaRoster::Unknown,
                    crate::capabilities::capabilities_for_backend(
                        eframe::NativeWindowingBackend::X11,
                    ),
                )
                .unwrap();
            let mut dpi = commit_boundary(&mut fixture.native);
            fixture.native.settle_host_frame_report(&dpi).unwrap();
            change.assert_applied(&fixture.native, fixture.host);
            assert!(dpi.manipulation_feedback().is_empty());
            assert!(dpi.take_native_effects().is_empty());
            assert_eq!(fixture.native.session.version(), version);

            let late_presentation = if presentation_first {
                settle_revoked_preview(&mut fixture, stale, result, Endpoint::Target);
                None
            } else {
                Some(stale)
            };
            let mut late_release = release_last_pane(
                &mut fixture.native,
                fixture.child,
                fixture.target,
                &fixture.receivers,
                &mut fixture.trace,
            );
            assert!(late_release.manipulation_feedback().is_empty());
            assert!(late_release.take_native_effects().is_empty());
            assert!(!late_release.workspace_changed());
            if let Some(stale) = late_presentation {
                settle_revoked_preview(&mut fixture, stale, result, Endpoint::Target);
            }
            assert_eq!(fixture.native.session.version(), version);
            assert!(!fixture.native.session.has_pending_presentation());
            assert_child_retained(&fixture.native, fixture.child);

            let effects = retry_last_pane_after_cancellation(
                &mut fixture.native,
                fixture.host,
                fixture.child,
                *cancelled,
                &mut fixture.trace,
            );
            assert!(matches!(effects.as_slice(), [effect]
                if effect.operation() == &NativeEffectOperation::ReleaseChild { binding: fixture.child }));
            assert_terminal_stays_settled(&mut fixture.native);
            retire_child(
                &mut fixture.native,
                fixture.host,
                fixture.child,
                effects,
                &mut fixture.trace,
            );
        }
    }
}
