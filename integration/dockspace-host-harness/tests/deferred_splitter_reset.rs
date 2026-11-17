//! Absolute splitter resets retain their original authority across weight-only jitter.

use dockspace::geometry::{
    AxisSizing, LogicalPoint, LogicalRect, PaneSizing, PhysicalSize, ScaleFactor,
};
use dockspace::model::{
    DockPlacement, DockspaceAxis, DockspaceContainedLayout, DockspaceLayout, DockspaceNode,
    DockspaceRootLayout, DockspaceSurfaceLayout, FloatingPresentationId, ItemId, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    ContainedResizeDirection, DeferredSurfaceAction, DockPresentationConfig, DockspaceHostFrame,
    DockspaceReceiverDescriptor, DockspaceSession, HostFrameReport, HostInputOutcome,
    HostPointerInput, HostPointerRoute, HostWindowToken, NativeHostCapabilities,
    NativeHostCapability, NativePointerRoster, NativeReceiverAnswer, NativeWindowFacts,
    NativeWindowInputState, NativeWindowPresentationState, NativeWorkAreaRoster,
    PreparedSurfaceAction, PresentedDockReceiver, PresentedDockspaceSurface, SplitterPaintRecord,
    SurfaceContainedResizeAdjustment, SurfaceMeasurementAnswer as Answer,
    SurfaceMeasurementRequest as Question, SurfacePaintPlan,
    SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
    SurfacePointerButton as Button, SurfacePointerCapture, SurfacePointerEvent as Event,
    SurfacePointerId, SurfacePointerReceiverFacts, SurfacePresentationResult,
    SurfaceSplitterAdjustment as Adjustment, SurfaceUnavailableReason, TabStripMetrics,
};
use dockspace_host_harness::RecordingRenderer;

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);
const THIRD: ItemId = ItemId::new(3);
const FLOAT: FloatingPresentationId = FloatingPresentationId::new(1);

fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
    LogicalRect::new(x, y, width, height).unwrap()
}

fn layout() -> DockspaceLayout {
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::tabs([FIRST, THIRD]), 1.0),
                    (DockspaceNode::tabs([SECOND]), 3.0),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap()
}

fn session() -> DockspaceSession {
    session_with_layout(layout())
}

fn session_with_layout(layout: DockspaceLayout) -> DockspaceSession {
    DockspaceSession::from_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        DockPresentationConfig::builder()
            .minimum_pane_size(1.0, 1.0)
            .splitter_thickness(1.0)
            .build()
            .unwrap(),
    )
    .unwrap()
}

fn sizing(preferred: Option<f64>, maximum: Option<f64>) -> PaneSizing {
    PaneSizing::new(
        AxisSizing::new(32.0, maximum, preferred).unwrap(),
        AxisSizing::new(24.0, None, None).unwrap(),
    )
}

fn measure(frame: &mut DockspaceHostFrame<'_>, width: f64, first: PaneSizing, second: PaneSizing) {
    frame
        .measure_surface_with(SURFACE, |question| {
            Ok::<_, dockspace::runtime::DockspaceRuntimeError>(match question {
                Question::DockBounds { .. } | Question::PopupPlaneBounds { .. } => {
                    Answer::Bounds(rect(0.0, 0.0, width, 600.0))
                }
                Question::PaneSizing { item, .. } => {
                    Answer::PaneSizing(if item == Some(FIRST) { first } else { second })
                }
                Question::TabIntrinsic { .. } => Answer::TabIntrinsic(80.0),
                Question::TabStrip { .. } => {
                    Answer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                }
            })
        })
        .unwrap();
}

fn commit(mut frame: DockspaceHostFrame<'_>) -> HostFrameReport {
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap()
}

fn splitter(plan: SurfacePaintPlan<'_>) -> SplitterPaintRecord<'_> {
    let mut splitter = None;
    plan.visit_surface_program(|step| {
        if let Step::Operation {
            operation: Operation::Splitter(record),
            ..
        } = step
        {
            splitter.get_or_insert(record);
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    splitter.unwrap()
}

fn capture(
    session: &mut DockspaceSession,
    adjustment: Adjustment,
    first: PaneSizing,
    second: PaneSizing,
) -> DeferredSurfaceAction {
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, 601.0, first, second);
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    plan.prepare_splitter_adjustment(splitter(plan).visual_id(), adjustment)
        .unwrap()
        .into_deferred()
}

fn submit(session: &mut DockspaceSession, action: DeferredSurfaceAction) -> HostFrameReport {
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_deferred_surface_action(action).unwrap();
    commit(frame)
}

fn assert_stale(report: &HostFrameReport) {
    assert!(!report.workspace_changed(), "{report:?}");
    assert!(
        matches!(report.inputs(), [HostInputOutcome::StaleRejected { .. }]),
        "{report:?}"
    );
}

fn assert_pair(
    session: &mut DockspaceSession,
    first: PaneSizing,
    second: PaneSizing,
    expected: [f64; 2],
) {
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, 601.0, first, second);
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let splitter = splitter(plan);
    let actual = [
        splitter.before_bounds().width(),
        splitter.after_bounds().width(),
    ];
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1.0e-9, "{actual} != {expected}");
    }
    commit(frame);
}

fn present_and_capture(
    session: &mut DockspaceSession,
    adjustment: Adjustment,
    first: PaneSizing,
    second: PaneSizing,
) -> (DockspaceReceiverDescriptor, PreparedSurfaceAction) {
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, 601.0, first, second);
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let splitter = splitter(plan);
    let receiver = plan.receiver_for_splitter(splitter).unwrap();
    let action = plan
        .prepare_splitter_adjustment(splitter.visual_id(), adjustment)
        .unwrap();
    let (_, receipt) = RecordingRenderer::execute(plan).unwrap();
    frame.confirm_surface_painted(receipt).unwrap();
    let mut outputs = commit(frame).take_painted_outputs();
    assert_eq!(outputs.len(), 1);
    session
        .report_surface_presentation(outputs.pop().unwrap(), SurfacePresentationResult::Presented)
        .unwrap();
    commit(session.begin_host_frame().unwrap());
    (receiver, action)
}

fn pointer_jitter<'receiver>(
    receiver: &'receiver PresentedDockReceiver,
    surface: &'receiver PresentedDockspaceSurface,
) -> [HostPointerInput<'receiver>; 3] {
    let point = receiver.center();
    let start = HostPointerRoute::new(*surface, point);
    let moved = HostPointerRoute::new(
        *surface,
        LogicalPoint::new(point.x() + 1.0, point.y()).unwrap(),
    );
    let facts = SurfacePointerReceiverFacts::delivery(receiver)
        .with_drag_delivery(receiver)
        .with_clear_hover_lane(surface);
    [
        (
            Event::ButtonPressed(Button::Primary),
            start,
            SurfacePointerCapture::ProviderEndpoint,
        ),
        (Event::Moved, moved, SurfacePointerCapture::ProviderEndpoint),
        (
            Event::ButtonReleased(Button::Primary),
            moved,
            SurfacePointerCapture::None,
        ),
    ]
    .map(|(event, route, capture)| {
        HostPointerInput::new(
            SurfacePointerId::new(1),
            event,
            route,
            route,
            capture,
            facts,
        )
    })
}

#[test]
fn deferred_absolute_reset_uses_the_original_witness_after_one_pixel_jitter_in_the_same_candidate()
{
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        let mut session = session();
        session.enable_host_pointer().unwrap();
        let first = sizing(Some(200.0), None);
        let second = sizing(None, None);
        let (receiver, reset) = present_and_capture(&mut session, adjustment, first, second);
        let receiver = session.bind_presented_receiver(&receiver).unwrap();
        let surface = session.presented_surface(SURFACE).unwrap();
        let jitter = pointer_jitter(&receiver, &surface);
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_host_pointer_batch(jitter).unwrap();
        frame
            .submit_deferred_surface_action(reset.into_deferred())
            .unwrap();
        let report = commit(frame);
        assert!(report.workspace_changed(), "{report:?}");
        assert!(
            !report
                .inputs()
                .iter()
                .any(|outcome| matches!(outcome, HostInputOutcome::StaleRejected { .. })),
            "{report:?}"
        );
        let expected = if adjustment == Adjustment::Preferred {
            [200.0, 400.0]
        } else {
            [300.0, 300.0]
        };
        assert_pair(&mut session, first, second, expected);
    }
}

#[test]
fn prepared_absolute_reset_keeps_its_strict_contract_after_an_earlier_weight_update() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        let mut session = session();
        let first = sizing(Some(200.0), None);
        let second = sizing(None, None);
        let (reset, increment) = {
            let mut frame = session.begin_host_frame().unwrap();
            measure(&mut frame, 601.0, first, second);
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let visual = splitter(plan).visual_id();
            let reset = plan
                .prepare_splitter_adjustment(visual, adjustment)
                .unwrap();
            let increment = plan
                .prepare_splitter_adjustment(visual, Adjustment::Increment)
                .unwrap()
                .into_deferred();
            commit(frame);
            (reset, increment)
        };
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(increment).unwrap();
        frame.submit_surface_action(reset).unwrap();
        let report = commit(frame);
        assert!(
            report.workspace_changed(),
            "the increment must execute: {report:?}"
        );
        assert!(
            matches!(report.inputs(), [HostInputOutcome::StaleRejected { .. }]),
            "ordinary prepared submission must preserve its original version: {report:?}"
        );
        assert_pair(&mut session, first, second, [166.0, 434.0]);
    }
}

#[test]
fn deferred_reset_intent_ignores_the_precomputed_displacement() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        let mut session = session();
        let first = sizing(Some(200.0), None);
        let second = sizing(None, None);
        let original = capture(&mut session, adjustment, first, second);
        let increment = capture(&mut session, Adjustment::Increment, first, second);
        assert!(submit(&mut session, increment).workspace_changed());
        let current = capture(&mut session, adjustment, first, second);
        assert_ne!(original.expected_version(), current.expected_version());
        assert!(original.same_intent(&current));
        assert!(submit(&mut session, original).workspace_changed());
    }
}

#[test]
fn deferred_absolute_reset_survives_cross_frame_jitter_with_or_without_a_replacement_plan() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        for remeasure in [false, true] {
            let mut session = session();
            session.enable_host_pointer().unwrap();
            let first = sizing(Some(200.0), None);
            let second = sizing(None, None);
            let (receiver, reset) = present_and_capture(&mut session, adjustment, first, second);
            let expected_version = reset.expected_version();
            let receiver = session.bind_presented_receiver(&receiver).unwrap();
            let surface = session.presented_surface(SURFACE).unwrap();
            let jitter = pointer_jitter(&receiver, &surface);
            let mut frame = session.begin_host_frame().unwrap();
            frame.submit_host_pointer_batch(jitter).unwrap();
            assert!(commit(frame).workspace_changed());
            assert_ne!(session.version(), expected_version);
            if remeasure {
                assert_pair(&mut session, first, second, [151.0, 449.0]);
            }
            let report = submit(&mut session, reset.into_deferred());
            assert!(report.workspace_changed(), "{report:?}");
            assert!(
                !report
                    .inputs()
                    .iter()
                    .any(|outcome| matches!(outcome, HostInputOutcome::StaleRejected { .. })),
                "{report:?}"
            );
            let expected = if adjustment == Adjustment::Preferred {
                [200.0, 400.0]
            } else {
                [300.0, 300.0]
            };
            assert_pair(&mut session, first, second, expected);
        }
    }
}

#[test]
fn preferred_reset_after_jitter_uses_current_geometry_and_the_original_hard_limits() {
    let mut session = session();
    session.enable_host_pointer().unwrap();
    let first = sizing(None, Some(220.0));
    let second = sizing(Some(100.0), None);
    let (receiver, reset) = present_and_capture(&mut session, Adjustment::Preferred, first, second);
    let receiver = session.bind_presented_receiver(&receiver).unwrap();
    let surface = session.presented_surface(SURFACE).unwrap();
    let jitter = pointer_jitter(&receiver, &surface);
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_host_pointer_batch(jitter).unwrap();
    assert!(commit(frame).workspace_changed());
    assert!(submit(&mut session, reset.into_deferred()).workspace_changed());
    assert_pair(&mut session, first, second, [220.0, 380.0]);
}

#[test]
fn newer_unpainted_measurements_cannot_be_lost_when_weights_change_before_an_old_reset() {
    for retain_painted_fallback in [false, true] {
        let mut session = session();
        let second = sizing(None, None);
        let reset = {
            let mut frame = session.begin_host_frame().unwrap();
            measure(&mut frame, 601.0, sizing(Some(200.0), None), second);
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let reset = plan
                .prepare_splitter_adjustment(splitter(plan).visual_id(), Adjustment::Preferred)
                .unwrap()
                .into_deferred();
            if retain_painted_fallback {
                let (_, receipt) = RecordingRenderer::execute(plan).unwrap();
                frame.confirm_surface_painted(receipt).unwrap();
                let mut outputs = commit(frame).take_painted_outputs();
                assert_eq!(outputs.len(), 1);
                session
                    .report_surface_presentation(
                        outputs.pop().unwrap(),
                        SurfacePresentationResult::Presented,
                    )
                    .unwrap();
                commit(session.begin_host_frame().unwrap());
            } else {
                assert!(commit(frame).take_painted_outputs().is_empty());
            }
            reset
        };
        let increment = {
            let mut frame = session.begin_host_frame().unwrap();
            measure(&mut frame, 601.0, sizing(Some(210.0), None), second);
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let increment = plan
                .prepare_splitter_adjustment(splitter(plan).visual_id(), Adjustment::Increment)
                .unwrap()
                .into_deferred();
            assert!(commit(frame).take_painted_outputs().is_empty());
            increment
        };
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(increment).unwrap();
        frame.submit_deferred_surface_action(reset).unwrap();
        let report = commit(frame);
        assert!(
            report.workspace_changed(),
            "the increment must execute: {report:?}"
        );
        assert!(
            matches!(report.inputs(), [HostInputOutcome::StaleRejected { .. }]),
            "the following reset must reject its superseded measurement witness, painted fallback: {retain_painted_fallback}: {report:?}"
        );
        assert_pair(
            &mut session,
            sizing(Some(210.0), None),
            second,
            [166.0, 434.0],
        );
    }
}

#[test]
fn absolute_reset_rejects_changed_order_visibility_selection_owner_and_epoch() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        for change in [
            "topology",
            "order",
            "visibility",
            "selection",
            "owner",
            "epoch",
        ] {
            let mut session = session();
            let first = sizing(Some(200.0), None);
            let second = sizing(None, None);
            let reset = capture(&mut session, adjustment, first, second);
            if change == "epoch" {
                let replacement = session.prepare_layout_reset(layout()).unwrap();
                commit(session.begin_layout_reset_frame(&replacement).unwrap());
            } else if change == "owner" {
                let mut frame = session.begin_host_frame().unwrap();
                measure(&mut frame, 601.0, first, second);
                commit(frame);
                let action = session.prepare_float_root(ROOT, SURFACE);
                let mut frame = session.begin_host_frame().unwrap();
                frame.submit_prepared_action(action).unwrap();
                assert!(commit(frame).workspace_changed());
            } else {
                let mut frame = session.begin_host_frame().unwrap();
                match change {
                    "topology" => {
                        frame
                            .dock_item_current(SECOND, DockPlacement::Before(FIRST))
                            .unwrap();
                    }
                    "order" => {
                        frame
                            .dock_item_current(THIRD, DockPlacement::Before(FIRST))
                            .unwrap();
                    }
                    "visibility" => {
                        frame.set_item_visibility_current(SECOND, false).unwrap();
                    }
                    "selection" => {
                        frame.select_item_current(THIRD).unwrap();
                    }
                    _ => unreachable!(),
                }
                assert!(commit(frame).workspace_changed());
            }
            assert_stale(&submit(&mut session, reset));
        }
    }
}

#[test]
fn absolute_reset_rejects_changed_measurements_bounds_policy_and_configuration() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        for change in [
            "measurements",
            "metadata_unavailable",
            "bounds",
            "policy",
            "configuration",
        ] {
            let mut session = session();
            let first = sizing(Some(200.0), None);
            let second = sizing(None, None);
            let reset = capture(&mut session, adjustment, first, second);
            let mut frame = session.begin_host_frame().unwrap();
            match change {
                "measurements" => measure(&mut frame, 601.0, sizing(Some(210.0), None), second),
                "metadata_unavailable" => {
                    frame
                        .measure_surface_with(SURFACE, |question| {
                            Ok::<_, dockspace::runtime::DockspaceRuntimeError>(match question {
                                Question::PaneSizing { .. } => Answer::Unavailable(
                                    SurfaceUnavailableReason::ContentUnavailable,
                                ),
                                Question::DockBounds { .. } | Question::PopupPlaneBounds { .. } => {
                                    Answer::Bounds(rect(0.0, 0.0, 601.0, 600.0))
                                }
                                Question::TabIntrinsic { .. } => Answer::TabIntrinsic(80.0),
                                Question::TabStrip { .. } => {
                                    Answer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                                }
                            })
                        })
                        .unwrap();
                    assert!(frame.paint_plan(SURFACE).unwrap().is_none());
                }
                "bounds" => measure(&mut frame, 701.0, first, second),
                "policy" => {
                    let mut policy = DockPolicy::default();
                    policy.set_arrangement_locked(true);
                    frame.replace_policy(policy).unwrap();
                }
                "configuration" => {
                    frame
                        .replace_presentation_config(
                            DockPresentationConfig::builder()
                                .splitter_thickness(2.0)
                                .build()
                                .unwrap(),
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            commit(frame);
            assert_stale(&submit(&mut session, reset));
        }
    }
}

#[test]
fn absolute_reset_rejects_a_moved_contained_occluder_with_unchanged_topology_and_order() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        let content = DockspaceNode::split(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::tabs([FIRST, THIRD]), 1.0),
                (DockspaceNode::tabs([SECOND]), 3.0),
            ],
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )
        .with_contained(DockspaceContainedLayout::new(
            FLOAT,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([ItemId::new(4)])),
            rect(170.0, 0.0, 400.0, 600.0),
        ))])
        .unwrap();
        let mut session = session_with_layout(layout);
        let first = sizing(Some(200.0), None);
        let second = sizing(None, None);
        let reset = capture(&mut session, adjustment, first, second);
        let resize = {
            let mut frame = session.begin_host_frame().unwrap();
            measure(&mut frame, 601.0, first, second);
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            plan.prepare_contained_resize_adjustment_sequence(
                FLOAT,
                ContainedResizeDirection::West,
                [SurfaceContainedResizeAdjustment::Decrement; 2],
            )
            .unwrap()
            .into_deferred()
        };
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(resize).unwrap();
        assert!(commit(frame).workspace_changed());
        assert_eq!(session.view().item(THIRD).unwrap().root(), ROOT);
        assert!(session.view().contained(FLOAT).unwrap().rect().x() < 150.0);
        assert_stale(&submit(&mut session, reset));
    }
}

#[test]
fn multi_step_restoration_sequence_remains_bound_to_its_original_weights() {
    let mut session = session();
    let first = sizing(Some(200.0), None);
    let second = sizing(None, None);
    let stale = {
        let mut frame = session.begin_host_frame().unwrap();
        measure(&mut frame, 601.0, first, second);
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        plan.prepare_splitter_adjustment_sequence(
            splitter(plan).visual_id(),
            [Adjustment::Preferred, Adjustment::Increment],
        )
        .unwrap()
        .into_deferred()
    };
    let increment = capture(&mut session, Adjustment::Increment, first, second);
    assert!(submit(&mut session, increment).workspace_changed());
    assert_stale(&submit(&mut session, stale));
}

#[test]
fn absolute_reset_rejects_changed_native_coordinates_even_with_identical_measurements() {
    for adjustment in [Adjustment::Equalize, Adjustment::Preferred] {
        let mut session = session();
        let mut policy = DockPolicy::default();
        policy.set_allow_native_surfaces(true);
        let mut frame = session.begin_host_frame().unwrap();
        frame.replace_policy(policy).unwrap();
        commit(frame);
        session
            .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
            .unwrap();
        session
            .configure_managed_native_capabilities(
                NativeHostCapabilities::none_supported()
                    .with(NativeHostCapability::AuthoritativeInventory)
                    .with(NativeHostCapability::AuthoritativeButtonState)
                    .with(NativeHostCapability::PointerHitTestObservation),
            )
            .unwrap();
        session
            .register_native_root(SURFACE, HostWindowToken::new(1))
            .unwrap();
        let registration = commit(
            session
                .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
                .unwrap(),
        );
        let binding = registration.native_bindings()[0];
        let native_facts = |presentation_scale| {
            NativeWindowFacts::live()
                .with_content_size(PhysicalSize::new(1202.0, 1200.0).unwrap())
                .with_native_scale_factor(ScaleFactor::new(2.0).unwrap())
                .with_presentation_scale_factor(ScaleFactor::new(presentation_scale).unwrap())
                .with_input(NativeWindowInputState::ReceivesInput, None)
                .with_presentation(NativeWindowPresentationState::Visible)
        };
        session
            .report_managed_native_snapshot(
                [(binding, native_facts(2.0))],
                NativeWorkAreaRoster::Unknown,
            )
            .unwrap();
        let first = sizing(Some(200.0), None);
        let second = sizing(None, None);
        let reset = {
            let mut frame = session
                .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
                .unwrap();
            measure(&mut frame, 601.0, first, second);
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let action = plan
                .prepare_splitter_adjustment(splitter(plan).visual_id(), adjustment)
                .unwrap()
                .into_deferred();
            commit(frame);
            action
        };
        session
            .report_managed_native_snapshot(
                [(binding, native_facts(1.5))],
                NativeWorkAreaRoster::Unknown,
            )
            .unwrap();
        commit(
            session
                .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
                .unwrap(),
        );
        let mut frame = session
            .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap();
        frame.submit_deferred_surface_action(reset).unwrap();
        assert_stale(&commit(frame));
    }
}
