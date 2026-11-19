use std::convert::Infallible;

use dockspace::{
    geometry::{LogicalRect, ScaleFactor},
    model::DockspaceActionOutcome,
    runtime::{
        DockspacePresentationTransitionResult, HostInputOutcome, HostWindowToken,
        NativeEffectOperation, NativeSurfaceRole, NativeWindowFacts, NativeWindowInputState,
        NativeWindowPresentationState, NativeWorkAreaRoster, SurfacePresentationResult,
    },
};

use super::*;

#[test]
fn detach_accepts_requested_transfer_and_retains_source_until_first_live() {
    let mut session = session().unwrap();
    session
        .register_native_root(MAIN, HostWindowToken::new(1))
        .unwrap();
    let registered = idle(&mut session).unwrap();
    let [binding] = registered.native_bindings() else {
        panic!("the Root must register before measuring its content");
    };
    let bounds = PhysicalRect::new(100.0, 80.0, 640.0, 480.0).unwrap();
    let scale = ScaleFactor::new(1.0).unwrap();
    let facts = NativeWindowFacts::live()
        .with_outer_bounds(bounds)
        .with_content_bounds(bounds)
        .with_native_scale_factor(scale)
        .with_presentation_scale_factor(scale)
        .with_input(NativeWindowInputState::ReceivesInput, None)
        .with_presentation(NativeWindowPresentationState::Visible);
    session
        .report_managed_native_snapshot([(binding.window(), facts)], NativeWorkAreaRoster::Unknown)
        .unwrap();
    idle(&mut session).unwrap();
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame.begin_encoding_pass().unwrap();
    super::super::paint::measure(
        &mut frame,
        MAIN,
        LogicalRect::new(0.0, 0.0, 640.0, 480.0).unwrap(),
    )
    .unwrap();
    let receipt = frame
        .paint_plan(MAIN)
        .unwrap()
        .unwrap()
        .execute_surface_program(|_| Ok::<(), Infallible>(()))
        .unwrap();
    frame.confirm_surface_painted(receipt).unwrap();
    let mut painted = frame.commit().unwrap();
    let [output] = painted.take_painted_outputs().try_into().unwrap();
    session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    idle(&mut session).unwrap();

    let version = session.version();
    let placement = PhysicalRect::new(900.0, 120.0, 420.0, 320.0).unwrap();
    let mut report = detach(&mut session, placement).unwrap();
    let [
        HostInputOutcome::ProductPresentationActionRequested {
            outcome:
                DockspaceActionOutcome::NativeRootTearOffRequested {
                    root,
                    source_surface: MAIN,
                    target_surface,
                    items,
                },
            transition,
        },
    ] = report.inputs()
    else {
        panic!(
            "detach must return the accepted request: {:?}",
            report.inputs()
        );
    };
    let transition = *transition;
    let target_surface = *target_surface;
    assert_ne!(*root, RootId::new(1));
    assert_eq!(items, &[INSPECTOR]);
    assert_eq!(session.version(), version);
    assert_eq!(session.view().item(INSPECTOR).unwrap().surface(), MAIN);
    assert_eq!(
        session.view().item(INSPECTOR).unwrap().root(),
        RootId::new(1)
    );
    assert!(session.view().surface(target_surface).is_none());
    assert!(session.has_pending_native_create());
    assert!(report.presentation_transitions().is_empty());
    let [create] = report.take_native_effects().try_into().unwrap();
    assert!(
        matches!(create.operation(), NativeEffectOperation::CreateWindow {
        binding, role: NativeSurfaceRole::Child, placement: actual,
    } if binding.surface() == target_surface && *actual == placement)
    );
    let _ = create.accepted();

    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    assert!(frame.cancel_next_native_create().unwrap());
    let cancelled = frame.commit().unwrap();
    let [terminal] = cancelled.presentation_transitions() else {
        panic!("cancellation must settle the identity returned by detach");
    };
    assert_eq!(terminal.id(), transition);
    assert_eq!(
        terminal.result(),
        DockspacePresentationTransitionResult::Cancelled
    );
}
