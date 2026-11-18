//! Per-request completion ownership is independent from execution capacity.

use std::sync::mpsc::TryRecvError;

use dockspace::model::{DockMoveSource, DockMoveTarget};

use super::*;

fn visibility_status(item: ItemId) -> NativeActionStatus {
    NativeActionStatus::Dock(DockspaceActionStatus::Applied(
        DockspaceActionOutcome::ItemVisibilityChanged {
            item,
            visible: false,
            changed: true,
        },
    ))
}

#[test]
fn independent_receipts_can_be_read_in_reverse_completion_order() {
    let mut state = state();
    let first = state
        .request_command(NativeCommand::SetItemVisibility {
            item: SECOND,
            visible: false,
        })
        .unwrap();
    assert_eq!(first.try_recv().unwrap_err(), TryRecvError::Empty);
    commit_queued(&mut state);
    assert!(!state.application_actions.is_occupied());
    assert!(
        !state
            .pending_work()
            .contains(crate::NativeWaitReason::ApplicationAction)
    );

    let second = state
        .request_command(NativeCommand::SetItemVisibility {
            item: THIRD,
            visible: false,
        })
        .unwrap();
    commit_queued(&mut state);
    assert!(!state.application_actions.is_occupied());
    assert_eq!(
        second.try_recv().unwrap().unwrap(),
        visibility_status(THIRD)
    );
    assert_eq!(
        first.try_recv().unwrap().unwrap(),
        visibility_status(SECOND)
    );
    assert_eq!(first.try_recv().unwrap_err(), TryRecvError::Disconnected);
    assert_eq!(second.try_recv().unwrap_err(), TryRecvError::Disconnected);
    assert!(state.view().item(SECOND).unwrap().is_hidden());
    assert!(state.view().item(THIRD).unwrap().is_hidden());
}

#[test]
fn discarding_a_receipt_does_not_cancel_the_accepted_request() {
    let mut state = state();
    let receipt = state
        .request_command(NativeCommand::SetItemVisibility {
            item: SECOND,
            visible: false,
        })
        .unwrap();
    drop(receipt);
    assert!(state.application_actions.is_occupied());
    commit_queued(&mut state);
    assert!(state.view().item(SECOND).unwrap().is_hidden());
    assert!(!state.application_actions.is_occupied());

    let next = state
        .request_command(NativeCommand::SetItemVisibility {
            item: THIRD,
            visible: false,
        })
        .unwrap();
    commit_queued(&mut state);
    assert_eq!(next.try_recv().unwrap().unwrap(), visibility_status(THIRD));
    assert!(state.error().is_none());
}

#[test]
fn a_final_native_record_yields_to_the_queued_command_at_the_budget_boundary() {
    use dockspace::runtime::{NativeInputBudget, NativeMonotonicClock};

    let mut state = state();
    let receipt = state
        .request_command(NativeCommand::SetItemVisibility {
            item: SECOND,
            visible: false,
        })
        .unwrap();
    let binding = state
        .coordinator
        .viewport_binding(egui::ViewportId::ROOT)
        .unwrap();
    state
        .coordinator
        .publish_close(binding, NativeCloseState::Clear, None)
        .unwrap();
    let context = egui::Context::default();
    let clock = NativeMonotonicClock::new(|| std::time::Duration::ZERO);
    let mut budget = NativeInputBudget::new(&clock);
    budget.complete_unsequenced_records(255);
    assert!(
        !state
            .service_pending_prefixes(&context, None, &mut budget)
            .unwrap(),
        "the accepted command requires another service turn after the last native record"
    );
    assert!(budget.is_exhausted());
    assert!(!state.coordinator.session().has_pending_native_input());
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Empty);
    assert!(state.view().item(SECOND).unwrap().is_visible());

    let mut budget = NativeInputBudget::new(&clock);
    assert!(
        state
            .service_pending_prefixes(&context, None, &mut budget)
            .unwrap()
    );
    assert_eq!(
        receipt.try_recv().unwrap().unwrap(),
        visibility_status(SECOND)
    );
    assert!(state.view().item(SECOND).unwrap().is_hidden());
    assert!(state.error().is_none());
}

#[test]
fn a_service_deadline_never_leaves_a_ready_command_waiting_without_a_wake() {
    use dockspace::runtime::{NativeInputBudget, NativeMonotonicClock};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    for cutoff in 1..32 {
        let mut state = state();
        let receipt = state
            .request_command(NativeCommand::SetItemVisibility {
                item: SECOND,
                visible: false,
            })
            .unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let clock = NativeMonotonicClock::new(move || {
            if reads.fetch_add(1, Ordering::SeqCst) < cutoff {
                Duration::ZERO
            } else {
                Duration::from_millis(3)
            }
        });
        let mut budget = NativeInputBudget::new(&clock);
        let idle = state
            .service_pending_prefixes(&egui::Context::default(), None, &mut budget)
            .unwrap();
        if idle {
            assert_eq!(
                receipt.try_recv().unwrap_or_else(|error| panic!(
                    "cutoff={cutoff}, idle service lost its wake: {error}"
                )),
                Ok(visibility_status(SECOND)),
            );
        } else {
            let clock = NativeMonotonicClock::new(|| Duration::ZERO);
            let mut budget = NativeInputBudget::new(&clock);
            assert!(
                state
                    .service_pending_prefixes(&egui::Context::default(), None, &mut budget)
                    .unwrap()
            );
            assert_eq!(
                receipt.try_recv().unwrap().unwrap(),
                visibility_status(SECOND)
            );
        }
        assert!(state.view().item(SECOND).unwrap().is_hidden());
        assert!(state.error().is_none());
    }
}

#[test]
fn configuration_failure_after_core_commit_reports_a_committed_interruption() {
    use crate::event::NativeWindowEventRecord;
    use crate::mailbox::HostRecord;
    use dockspace::runtime::{
        NativeInputBudget, NativeInputStorageLimit, NativeMonotonicClock,
        NativeProducerStorageLimit, NativeReportStorageLimit,
    };

    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs([FIRST, SECOND])),
    )])
    .unwrap();
    let slots = |count| std::num::NonZeroUsize::new(count).unwrap();
    let input = NativeInputStorageLimit::new(slots(4), 512 * 1024)
        .with_producer_storage_limit(NativeProducerStorageLimit::new(
            slots(8),
            slots(1),
            128 * 1024,
        ))
        .with_report_storage_limit(NativeReportStorageLimit::new(
            slots(64),
            slots(8),
            256 * 1024,
        ));
    let storage = crate::NativeStorageLimit::new(input, 4 * 1024 * 1024, 2, 2).unwrap();
    let mut state =
        state_with_layout_policy_and_storage(layout, DockPolicy::default(), Some(storage));
    let binding = state
        .coordinator
        .viewport_binding(egui::ViewportId::ROOT)
        .unwrap();
    let bridge = state.coordinator.dispatch_bridge();
    bridge.push_record(HostRecord::WindowEvent(NativeWindowEventRecord::for_test(
        1,
        winit::window::WindowId::from(1),
        Some(egui::ViewportId::ROOT),
        Some(binding),
        winit::event::WindowEvent::ModifiersChanged(winit::keyboard::ModifiersState::ALT.into()),
    )));
    let context = egui::Context::default();
    let clock = NativeMonotonicClock::new(|| std::time::Duration::ZERO);
    state
        .coordinator
        .reduce_callback_prefix(&context, &mut NativeInputBudget::new(&clock))
        .unwrap();
    commit_native_command_observations(&mut state);
    let configured = egui_dockspace::DockInputConfig {
        group_preserve_layout: None,
        ..state.input()
    };
    let receipt = state.request_input(configured).unwrap();
    let action = state.application_actions.take().unwrap();
    let published = state.coordinator.session().version();
    let mut frame = state
        .coordinator
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let submitted = action
        .submit(&mut frame.frame, published)
        .unwrap()
        .finish(&mut frame.frame)
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit().unwrap();
    for _ in 0..4 {
        state
            .coordinator
            .publish_close(binding, NativeCloseState::Clear, None)
            .unwrap();
    }
    let error = state
        .publish_application_configuration(&context, &mut Some(submitted), &report)
        .unwrap_err();
    assert_eq!(
        state.input(),
        configured,
        "adapter configuration was already published"
    );
    let kind = error.kind();
    state.stop_current_output(error);
    let failure = receipt.try_recv().unwrap().unwrap_err();
    assert_eq!(failure.kind(), kind);
    assert_eq!(
        failure.interruption(),
        crate::NativeActionInterruption::CommittedOutcomeUnavailable
    );
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Disconnected);
}

#[test]
fn runtime_drop_disconnects_queued_and_uncommitted_receipts() {
    for submitted in [false, true] {
        let mut state = state();
        let receipt = state
            .request_command(NativeCommand::SetItemVisibility {
                item: SECOND,
                visible: false,
            })
            .unwrap();
        if submitted {
            let published = state.coordinator.session().version();
            let action = state.application_actions.take().unwrap();
            let mut frame = state
                .coordinator
                .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
                .unwrap();
            let submission = action.submit(&mut frame.frame, published).unwrap();
            let submitted = submission.finish(&mut frame.frame).unwrap();
            drop(submitted);
            drop(frame);
        }
        assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Empty);
        assert!(state.view().item(SECOND).unwrap().is_visible());
        drop(state);
        assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Disconnected);
    }
}

#[test]
fn committed_result_outlives_its_runtime_without_becoming_a_disconnect() {
    let mut state = state();
    let receipt = state
        .request_command(NativeCommand::SetItemVisibility {
            item: SECOND,
            visible: false,
        })
        .unwrap();
    commit_queued(&mut state);
    drop(state);
    assert_eq!(
        receipt.try_recv().unwrap().unwrap(),
        visibility_status(SECOND)
    );
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Disconnected);
}

fn awaiting_native_return() -> (NativeRuntimeState<Panes>, crate::NativeActionReceipt) {
    let mut state = state_for_native_commands();
    let group = state.capture_tabs_group(DockAnchor::Item(SECOND)).unwrap();
    let before = state.coordinator.session().version();
    let creation = state
        .request_command(NativeCommand::Move {
            source: DockMoveSource::Tabs(group),
            target: DockMoveTarget::Native {
                placement: Some(NativeWindowPlacement::new(
                    PhysicalRect::new(900.0, 100.0, 480.0, 320.0).unwrap(),
                )),
            },
        })
        .unwrap();
    let mut report = commit_queued_report(&mut state);
    assert!(matches!(
        take(&creation),
        NativeActionStatus::Dock(DockspaceActionStatus::Applied(
            DockspaceActionOutcome::NativeRootTearOffRequested { .. }
        ))
    ));
    let (binding, bounds) =
        present_created_native_payload(&mut state, &mut report, before, &[SECOND, THIRD]);
    present_group_return_endpoints(&mut state, binding, bounds);
    let group = state.capture_tabs_group(DockAnchor::Item(SECOND)).unwrap();
    let receipt = state
        .request_command(NativeCommand::Move {
            source: DockMoveSource::Tabs(group),
            target: DockMoveTarget::Dock {
                placement: DockPlacement::After(FIRST),
                mode: TabGroupDockMode::MergeTabs,
            },
        })
        .unwrap();
    let report = commit_queued_report(&mut state);
    assert!(matches!(
        report.inputs(),
        [HostInputOutcome::ProductPresentationActionRequested { .. }]
    ));
    assert!(state.coordinator.session().has_pending_presentation());
    assert!(state.application_actions.is_occupied());
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Empty);
    assert_eq!(
        state.view().item(SECOND).unwrap().surface(),
        binding.surface()
    );
    (state, receipt)
}

#[test]
fn service_updates_an_existing_child_without_a_root_paint() {
    let mut state = state_for_native_commands();
    let before = state.coordinator.session().version();
    let creation = state
        .request_command(NativeCommand::Move {
            source: DockMoveSource::Item(SECOND),
            target: DockMoveTarget::Native {
                placement: Some(NativeWindowPlacement::new(
                    PhysicalRect::new(900.0, 100.0, 480.0, 320.0).unwrap(),
                )),
            },
        })
        .unwrap();
    let mut report = commit_queued_report(&mut state);
    let (binding, _) = present_created_native_payload(&mut state, &mut report, before, &[SECOND]);
    let context = egui::Context::default();
    let pass = context.cumulative_pass_nr_for(egui::ViewportId::ROOT);
    let hidden = state
        .request_command(NativeCommand::SetItemVisibility {
            item: SECOND,
            visible: false,
        })
        .unwrap();
    service_until_wait(&mut state, &context);
    assert_eq!(
        hidden.try_recv().unwrap().unwrap(),
        visibility_status(SECOND)
    );
    assert!(state.view().item(SECOND).unwrap().is_hidden());
    assert_eq!(
        state.view().item(SECOND).unwrap().surface(),
        binding.surface()
    );
    assert!(state.view().item(FIRST).unwrap().is_visible());
    assert_eq!(context.cumulative_pass_nr_for(egui::ViewportId::ROOT), pass);
    assert!(matches!(
        creation.try_recv(),
        Ok(Ok(NativeActionStatus::Dock(
            DockspaceActionStatus::Applied(
                DockspaceActionOutcome::NativeRootTearOffRequested { .. }
            )
        )))
    ));
    assert!(!state.application_actions.is_occupied());
    assert!(state.error().is_none());
}

#[test]
fn runtime_drop_disconnects_an_awaiting_presentation_receipt() {
    let (state, receipt) = awaiting_native_return();
    drop(state);
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Disconnected);
}

#[test]
fn a_new_source_render_does_not_cancel_a_programmatic_native_return() {
    let (mut state, receipt) = awaiting_native_return();
    let source = state.view().item(SECOND).unwrap().surface();
    let before = state.coordinator.session().version();
    let mut repaint = |surface, bounds, tab_width| {
        let mut frame = state
            .coordinator
            .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap();
        frame
            .frame
            .measure_surface(
                surface,
                UniformSurfaceMetrics::new(
                    bounds,
                    LogicalSize::new(64.0, 48.0).unwrap(),
                    tab_width,
                )
                .unwrap(),
            )
            .unwrap();
        let painted = frame
            .frame
            .paint_plan(surface)
            .unwrap()
            .unwrap()
            .execute_surface_program(|_| Ok::<(), ()>(()))
            .unwrap();
        frame.frame.confirm_surface_painted(painted).unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        let mut report = frame.commit().unwrap();
        state.coordinator.settle_host_frame_report(&report).unwrap();
        let [output] = report.take_painted_outputs().try_into().unwrap();
        state
            .coordinator
            .report_model_presentation_for_test(output, SurfacePresentationResult::Presented)
            .unwrap();
        let report = commit_native_command_observations(&mut state);
        state
            .application_actions
            .settle_presentation_transitions(report.presentation_transitions())
            .unwrap();
        if surface == source {
            assert!(state.coordinator.session().has_pending_presentation());
            assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Empty);
            assert_eq!(state.coordinator.session().version(), before);
            assert_eq!(state.view().item(SECOND).unwrap().surface(), source);
        }
    };
    repaint(
        source,
        LogicalRect::new(0.0, 0.0, 480.0, 320.0).unwrap(),
        132.0,
    );
    repaint(
        SURFACE,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
        96.0,
    );
    assert!(matches!(receipt.try_recv(), Ok(Ok(NativeActionStatus::Dock(
        DockspaceActionStatus::Applied(DockspaceActionOutcome::TabsDocked {
            source_root: _, target_root: ROOT, items, changed: true,
        })
    ))) if items == [SECOND, THIRD]));
    assert_eq!(state.view().item(SECOND).unwrap().surface(), SURFACE);
    assert_eq!(state.view().item(THIRD).unwrap().surface(), SURFACE);
    assert!(!state.application_actions.is_occupied());
    assert!(state.error().is_none());
}

#[test]
fn fatal_stop_keeps_a_committed_return_pending_without_a_real_terminal() {
    let (mut state, receipt) = awaiting_native_return();
    let ownership = |state: &NativeRuntimeState<Panes>| {
        [FIRST, SECOND, THIRD].map(|item| {
            let location = state.view().item(item).unwrap();
            (location.surface(), location.root(), location.is_hidden())
        })
    };
    let before = ownership(&state);
    let version = state.coordinator.session().version();
    state.stop_current_output(
        crate::error::NativeHostProtocolError::OutputRouteAttachmentFailed.into(),
    );
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Empty);
    assert!(state.application_actions.is_occupied());
    assert!(!state.is_shutdown_drained());
    assert!(
        !state
            .application_actions
            .settle_presentation_transitions(&[])
            .unwrap()
    );
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Empty);
    assert_eq!(ownership(&state), before);
    assert_eq!(state.coordinator.session().version(), version);
    drop(state);
    assert_eq!(receipt.try_recv().unwrap_err(), TryRecvError::Disconnected);
}
