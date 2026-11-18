//! Native target revocation terminates rehome without waiting for an output.

use super::*;

use crate::geometry::PhysicalPoint;
use crate::runtime::{
    NativeDesktopPointerLocation, NativeDesktopPosition, NativePointerButton,
    NativePointerCancelReason, NativePointerEvent, NativePointerHover, NativePointerId,
    NativePointerInput, NativePointerOwner, NativeReceiverPurpose, NativeReceiverQuery,
    NativeSurfacePlacement, NativeWindowHit, PaintedSurfaceOutput,
};

const TARGET_SURFACE: SurfaceId = SurfaceId::new(40);
const TARGET_ROOT: RootId = RootId::new(400);
const TARGET_WINDOW: HostWindowToken = HostWindowToken::new(4);

#[derive(Clone, Copy, Debug)]
enum TargetLoss {
    Destroyed,
    GeometryUnavailable,
    BindingReplaced,
}

fn three_native_windows() -> (
    DockspaceSession,
    NativeSurfaceBinding,
    NativeSurfaceBinding,
    NativeSurfaceBinding,
) {
    let (mut session, source) = live_managed_native_root();
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .attach_empty_surface_current(TARGET_SURFACE, TARGET_ROOT)
        .unwrap();
    commit(frame);
    session
        .register_native_root(TARGET_SURFACE, TARGET_WINDOW)
        .unwrap();
    let registered = commit_managed_native_frame(&mut session);
    let target = registered
        .native_bindings()
        .iter()
        .copied()
        .find(|binding| binding.surface() == TARGET_SURFACE)
        .unwrap_or_else(|| panic!("the independent target must register: {registered:?}"));
    let root = session
        .native_window_bindings()
        .flat_map(|window| session.native_surface_bindings_for_window(window))
        .find(|binding| binding.surface() == MAIN_SURFACE)
        .unwrap();
    session
        .report_managed_native_snapshot(
            [
                (root, native_facts()),
                (source, native_facts()),
                (target, native_facts()),
            ],
            managed_native_work_areas(),
        )
        .unwrap();
    commit_managed_native_frame(&mut session);
    present_surfaces(
        &mut session,
        &[MAIN_SURFACE, NATIVE_SOURCE_SURFACE, TARGET_SURFACE],
    );
    (session, root, source, target)
}

fn native_facts() -> NativeWindowFacts {
    managed_native_window_facts(
        PhysicalRect::new(0.0, 0.0, 800.0, 600.0).expect("the visible geometry validates"),
    )
}

fn emit_target(session: &mut DockspaceSession) -> PaintedSurfaceOutput {
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface(TARGET_SURFACE, presentation_metrics())
        .unwrap();
    let receipt = frame
        .paint_plan(TARGET_SURFACE)
        .unwrap()
        .unwrap()
        .execute_surface_program(|_| Ok::<(), ()>(()))
        .unwrap();
    frame.confirm_surface_painted(receipt).unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let mut report = frame.commit().unwrap();
    let mut outputs = report.take_painted_outputs();
    assert_eq!(outputs.len(), 1);
    outputs.pop().unwrap()
}

fn assert_source_ownership(session: &DockspaceSession) {
    for item in [NATIVE_FIRST, NATIVE_SECOND] {
        let pane = session.view().item(item).unwrap();
        assert_eq!(pane.surface(), NATIVE_SOURCE_SURFACE);
        assert_eq!(pane.root(), NATIVE_SOURCE_ROOT);
    }
}

fn assert_fresh_drag_on_survivor(session: &mut DockspaceSession, root: NativeSurfaceBinding) {
    // Stay in the enrolled native provider. Starting a second host-local
    // provider would test a different input lifetime and is not permitted.
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let plan = frame.paint_plan(MAIN_SURFACE).unwrap().unwrap();
    let tab = plan
        .tabs()
        .find(|tab| tab.item() == Some(TARGET_FIRST))
        .unwrap();
    let receiver = plan.receiver_for_tab_body(tab).unwrap();
    let position = receiver.center();
    let receipt = plan.execute_surface_program(|_| Ok::<(), ()>(())).unwrap();
    frame.confirm_surface_painted(receipt).unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let mut report = frame.commit().unwrap();
    let mut outputs = report.take_painted_outputs();
    assert_eq!(outputs.len(), 1);
    let output = outputs.pop().unwrap();
    let artifact = output.presentation_artifact();
    session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    commit_managed_native_frame(session);
    let moved = PhysicalPoint::new(position.x() + 35.0, position.y() + 45.0).unwrap();
    let pointer = NativePointerId::new(1);
    for (event, position) in [
        (
            NativePointerEvent::ButtonPressed(NativePointerButton::Primary),
            PhysicalPoint::new(position.x(), position.y()).unwrap(),
        ),
        (NativePointerEvent::Moved, moved),
    ] {
        session
            .record_native_pointer(NativePointerInput::new(
                pointer,
                event,
                NativeDesktopPointerLocation::new(
                    NativeDesktopPosition::Exact(position),
                    NativePointerHover::Dock(root.window()),
                    None,
                ),
                NativePointerOwner::Native(root.window()),
                NativePointerOwner::Native(root.window()),
            ))
            .unwrap();
    }
    let mut frame = session
        .begin_native_host_frame(|query| match query {
            NativeReceiverQuery::WindowHit(query) => {
                NativeReceiverAnswer::WindowHit(if query.window() == root.window() {
                    NativeWindowHit::Surface(artifact)
                } else {
                    NativeWindowHit::Unknown
                })
            }
            NativeReceiverQuery::Surface(query) => match query.purpose() {
                NativeReceiverPurpose::ClickDelivery | NativeReceiverPurpose::DragDelivery => query
                    .bind_receiver(&receiver)
                    .map_or(NativeReceiverAnswer::Unknown, NativeReceiverAnswer::Dock),
                NativeReceiverPurpose::HoverHit => {
                    NativeReceiverAnswer::ClearHoverLane(query.presented_surface())
                }
                _ => NativeReceiverAnswer::Unknown,
            },
        })
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();
    assert!(
        session.has_active_gesture(),
        "the cancelled transfer cannot block another window's next native drag"
    );
    session
        .record_native_pointer(NativePointerInput::new(
            pointer,
            NativePointerEvent::StreamCancelled(
                NativePointerCancelReason::ExplicitPlatformCancellation,
            ),
            NativeDesktopPointerLocation::new(
                NativeDesktopPosition::Unknown,
                NativePointerHover::Unknown,
                None,
            ),
            NativePointerOwner::Native(root.window()),
            NativePointerOwner::None,
        ))
        .unwrap();
    commit_managed_native_frame(session);
    assert!(!session.has_active_gesture());
}

#[test]
fn native_rehome_target_revocation_terminates_without_any_preview_output() {
    for loss in [
        TargetLoss::Destroyed,
        TargetLoss::GeometryUnavailable,
        TargetLoss::BindingReplaced,
    ] {
        check_target_loss(loss, None);
    }
}

#[test]
fn native_rehome_target_revocation_cannot_be_reversed_by_a_late_preview_terminal() {
    for loss in [
        TargetLoss::Destroyed,
        TargetLoss::GeometryUnavailable,
        TargetLoss::BindingReplaced,
    ] {
        for terminal in [
            SurfacePresentationResult::Presented,
            SurfacePresentationResult::Dropped,
        ] {
            check_target_loss(loss, Some(terminal));
        }
    }
}

fn check_target_loss(loss: TargetLoss, late: Option<SurfacePresentationResult>) {
    let (mut session, root, source, target) = three_native_windows();
    let before = session.version();
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .dock_root_current(
            NATIVE_SOURCE_ROOT,
            DockPlacement::Center(DockAnchor::Central(TARGET_ROOT)),
        )
        .unwrap();
    let staged = commit(frame);
    let [HostInputOutcome::ProductPresentationActionRequested { transition, .. }] = staged.inputs()
    else {
        panic!(
            "the complete native root must stage one rehome: {:?}",
            staged.inputs()
        );
    };
    let transition = *transition;
    assert!(session.has_pending_presentation());
    let output = late.map(|_| emit_target(&mut session));
    let mut current_target = target;
    let mut cancelled = if matches!(loss, TargetLoss::BindingReplaced) {
        let mut frame = session
            .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap();
        frame
            .set_native_surface_placements(
                target.window(),
                &[NativeSurfacePlacement::new(
                    TARGET_SURFACE,
                    LogicalSize::new(780.0, 600.0).unwrap(),
                    PhysicalPoint::new(20.0, 0.0).unwrap(),
                    ScaleFactor::new(1.0).unwrap(),
                    PhysicalRect::new(20.0, 0.0, 780.0, 600.0).unwrap(),
                )],
            )
            .unwrap();
        let report = commit(frame);
        current_target = session
            .native_surface_bindings_for_window(target.window())
            .find(|binding| binding.surface() == TARGET_SURFACE)
            .unwrap();
        assert_ne!(current_target, target);
        assert!(!session.is_current_native_binding(target));
        assert!(session.is_current_native_binding(current_target));
        report
    } else {
        let mut facts = vec![(root, native_facts()), (source, native_facts())];
        match loss {
            TargetLoss::Destroyed => facts.push((target, NativeWindowFacts::destroyed())),
            TargetLoss::GeometryUnavailable => facts.push((target, NativeWindowFacts::live())),
            TargetLoss::BindingReplaced => {
                unreachable!("replacement uses its exact placement transaction")
            }
        }
        session
            .report_managed_native_snapshot(facts, managed_native_work_areas())
            .unwrap();
        commit_managed_native_frame(&mut session)
    };
    assert!(
        !session.has_pending_presentation(),
        "{loss:?}/{late:?}: native target authority loss must not wait for a nonexistent Dropped output"
    );
    assert!(matches!(
        cancelled.presentation_transitions(),
        [terminal] if terminal.id() == transition
            && terminal.result() == DockspacePresentationTransitionResult::TargetNotPresented
            && terminal.outcome().is_none()
    ));
    assert!(cancelled.take_native_effects().is_empty());
    assert_eq!(session.version(), before);
    assert!(session.is_current_native_binding(source));
    assert_source_ownership(&session);
    if let (Some(output), Some(terminal)) = (output, late) {
        session
            .report_surface_presentation(output, terminal)
            .unwrap();
        let settled = commit_managed_native_frame(&mut session);
        assert!(settled.presentation_transitions().is_empty());
        assert!(!session.has_pending_presentation());
        assert_source_ownership(&session);
    }
    if !matches!(loss, TargetLoss::Destroyed) {
        session
            .report_managed_native_snapshot(
                [
                    (root, native_facts()),
                    (source, native_facts()),
                    (current_target, native_facts()),
                ],
                managed_native_work_areas(),
            )
            .unwrap();
        assert!(
            commit_managed_native_frame(&mut session)
                .presentation_transitions()
                .is_empty()
        );
        let restored = present_surfaces_with_report(
            &mut session,
            &[MAIN_SURFACE, NATIVE_SOURCE_SURFACE, TARGET_SURFACE],
        );
        assert!(restored.presentation_transitions().is_empty());
        assert!(!session.has_pending_presentation());
        assert_source_ownership(&session);
    }
    let painted =
        present_surfaces_with_report(&mut session, &[MAIN_SURFACE, NATIVE_SOURCE_SURFACE]);
    assert!(painted.presentation_transitions().is_empty());
    assert_source_ownership(&session);
    assert_fresh_drag_on_survivor(&mut session, root);
}
