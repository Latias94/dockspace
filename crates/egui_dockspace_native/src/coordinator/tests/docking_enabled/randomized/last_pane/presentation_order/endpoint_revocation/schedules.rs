//! Bounded schedules preserve each producer's order while varying their interleaving.
//!
//! These controlled snapshots and recorded receiver answers exercise the native
//! reducer, not OS callback delivery or actual GPU presentation.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    ChangeWindow,
    RestoreWindow,
    TailTerminal,
    HeadTerminal,
    DuplicateRelease,
}

const LANES: [&[Step]; 3] = [
    &[Step::ChangeWindow, Step::RestoreWindow],
    &[Step::TailTerminal, Step::HeadTerminal],
    &[Step::DuplicateRelease],
];

fn schedules(prefix: &mut Vec<Step>, positions: [usize; 3], output: &mut Vec<Vec<Step>>) {
    if prefix.len() == 5 {
        output.push(prefix.clone());
        return;
    }
    for (lane, steps) in LANES.iter().enumerate() {
        let Some(&step) = steps.get(positions[lane]) else {
            continue;
        };
        // The head protects the pending release until revocation commits.
        // Delivery before revocation is covered by the ordinary delivery tests.
        if step == Step::HeadTerminal && !prefix.contains(&Step::ChangeWindow) {
            continue;
        }
        let mut next = positions;
        next[lane] += 1;
        prefix.push(step);
        schedules(prefix, next, output);
        prefix.pop();
    }
}

fn run(
    schedule: &[Step],
    endpoint: Endpoint,
    change: WindowChange,
    result: SurfacePresentationResult,
) {
    let mut fixture = dragging(0x5c4e_d01e);
    fixture
        .trace
        .event(("schedule", schedule, endpoint, change, result));
    let mut head = Some(hold_target_preview(&mut fixture.native, fixture.host));
    let mut tail = Some(hold_target_preview(&mut fixture.native, fixture.host));
    let head_artifact = head.as_ref().unwrap().presentation_artifact();
    let tail_artifact = tail.as_ref().unwrap().presentation_artifact();
    assert_ne!(head_artifact, tail_artifact);
    let pending = release(&mut fixture);
    let version = fixture.native.session.version();
    let mut cancelled = None;
    let mut settlements = [0; 2];
    let mut revocation_committed = false;
    let mut head_reported = false;

    for &step in schedule {
        fixture.trace.event(step);
        match step {
            Step::ChangeWindow | Step::RestoreWindow => {
                let mut windows = [
                    (fixture.host, ready_window_facts(0.0)),
                    (fixture.child, ready_window_facts(720.0)),
                ];
                if step == Step::ChangeWindow {
                    let binding = windows[endpoint.index()].0;
                    windows[endpoint.index()].1 = change.facts(x(binding.surface()));
                }
                fixture
                    .native
                    .report_snapshot(
                        windows,
                        NativeWorkAreaRoster::Unknown,
                        crate::capabilities::capabilities_for_backend(
                            eframe::NativeWindowingBackend::X11,
                        ),
                    )
                    .unwrap();
            }
            Step::TailTerminal | Step::HeadTerminal => {
                head_reported |= step == Step::HeadTerminal;
                let (output, terminal) = if step == Step::TailTerminal {
                    (tail.take().unwrap(), result)
                } else {
                    (head.take().unwrap(), SurfacePresentationResult::Dropped)
                };
                fixture
                    .native
                    .session
                    .report_surface_presentation(output, terminal)
                    .unwrap();
            }
            Step::DuplicateRelease => {
                let event = mouse(
                    fixture.child,
                    fixture.target,
                    Some(ElementState::Released),
                    FIRST_SURFACE,
                    PointerWindowRoute::None,
                    None,
                );
                let ordinal = fixture.trace.event(&event);
                let record = captured_record(&fixture.native, fixture.child, ordinal, event);
                fixture
                    .native
                    .bridge
                    .push_record(HostRecord::WindowEvent(record));
                fixture
                    .native
                    .reduce_callback_prefix(&egui::Context::default(), &mut test_budget())
                    .unwrap();
            }
        }
        drop(
            fixture
                .native
                .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
                .unwrap(),
        );
        assert_eq!(
            fixture.native.session.version(),
            version,
            "{}",
            fixture.trace.replay()
        );
        assert_eq!(
            fixture.native.session.has_pending_presentation(),
            !revocation_committed
        );
        assert_child_retained(&fixture.native, fixture.child);

        let mut report = commit_boundary(&mut fixture.native);
        fixture.native.settle_host_frame_report(&report).unwrap();
        if step == Step::ChangeWindow {
            assert_eq!(
                report.manipulation_feedback().len(),
                1,
                "{}",
                fixture.trace.replay()
            );
            revocation_committed = true;
        } else {
            assert_eq!(
                report.manipulation_feedback(),
                [],
                "{}",
                fixture.trace.replay()
            );
        }
        assert_eq!(
            fixture.native.session.has_pending_presentation(),
            !revocation_committed
        );
        for feedback in report.manipulation_feedback() {
            assert_eq!(feedback.manipulation_id(), pending.manipulation_id());
            assert!(
                matches!(feedback.status(), Status::Cancelled(_)),
                "{}: {report:?}",
                fixture.trace.replay()
            );
            assert!(
                cancelled.replace(*feedback).is_none(),
                "{}: cancellation is affine",
                fixture.trace.replay()
            );
        }
        for (count, artifact) in settlements.iter_mut().zip([head_artifact, tail_artifact]) {
            *count += report
                .surface_presentation_commits()
                .iter()
                .filter(|commit| commit.settles(artifact))
                .count();
            assert!(
                *count <= 1,
                "{}: exact outputs settle once",
                fixture.trace.replay()
            );
        }
        for commit in report.surface_presentation_commits() {
            if commit.settles(head_artifact) {
                assert!(commit.retires(head_artifact));
            }
            if commit.settles(tail_artifact)
                && (result == SurfacePresentationResult::Dropped || endpoint == Endpoint::Target)
            {
                assert!(
                    commit.retires(tail_artifact),
                    "{}: old target geometry stays revoked after restoration",
                    fixture.trace.replay()
                );
            }
        }
        if !head_reported {
            assert_eq!(
                settlements,
                [0, 0],
                "a tail cannot bypass its unsettled head"
            );
        }
        assert_eq!(report.take_native_effects(), []);
        assert!(!report.workspace_changed());
        assert_eq!(fixture.native.session.version(), version);
        assert_child_retained(&fixture.native, fixture.child);
    }

    assert_eq!(settlements, [1, 1], "{}", fixture.trace.replay());
    let cancelled = cancelled.expect("every schedule commits window revocation");
    assert!(!fixture.native.session.has_pending_presentation());
    assert!(!fixture.native.session.has_active_gesture());
    assert_terminal_stays_settled(&mut fixture.native);
    let effects = retry_last_pane_after_cancellation(
        &mut fixture.native,
        fixture.host,
        fixture.child,
        cancelled,
        &mut fixture.trace,
    );
    retire_child(
        &mut fixture.native,
        fixture.host,
        fixture.child,
        effects,
        &mut fixture.trace,
    );
}

#[test]
fn bounded_window_output_and_release_schedules_cancel_once_then_dock_again() {
    let mut all = Vec::new();
    schedules(&mut Vec::new(), [0; 3], &mut all);
    assert_eq!(all.len(), 25);
    for schedule in &all {
        for endpoint in [Endpoint::Source, Endpoint::Target] {
            for change in [
                WindowChange::Hidden,
                WindowChange::NativeDpi,
                WindowChange::PresentationScale,
                WindowChange::ClientResize,
                WindowChange::NegativeOrigin,
                WindowChange::PositiveOrigin,
            ] {
                for result in [
                    SurfacePresentationResult::Presented,
                    SurfacePresentationResult::Dropped,
                ] {
                    run(schedule, endpoint, change, result);
                }
            }
        }
    }
}
