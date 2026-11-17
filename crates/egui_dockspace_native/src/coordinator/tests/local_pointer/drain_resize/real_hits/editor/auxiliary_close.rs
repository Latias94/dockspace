//! Controlled native callbacks and actual renderer responses share one middle click.
//!
//! The fixture supplies both input lanes normally fed by the window integration;
//! it does not emulate the OS conversion or mint native renderer output tokens.

use dockspace::close::{CloseDecision, ClosePlanTarget, DeferredCloseDecision};
use dockspace::policy::CloseCapability;
use dockspace::runtime::{
    DockspaceCloseOutcome, DockspaceClosePlan, DockspaceCloseResolution, HostCloseRequestOrigin,
};
use egui_dockspace::native_support::EguiSurfaceAction;

use super::*;

fn present_events(editor: &mut Editor, events: Vec<egui::Event>) -> Vec<EguiSurfaceAction> {
    let EncodedScene {
        output,
        mut paint,
        pixels_per_point,
        ..
    } = editor.encode_with_events(events);
    let actions = paint.take_local_actions();
    present_completed_output(
        &mut editor.native.receivers,
        &editor.context,
        pixels_per_point,
        editor.binding,
        &output,
        &paint,
    )
    .unwrap();
    editor
        .native
        .session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    let report = editor.commit(false);
    assert!(report.inputs().is_empty(), "{report:?}");
    assert!(!report.workspace_changed());
    actions
}

fn middle_click(editor: &mut Editor, point: LogicalPoint) -> Vec<EguiSurfaceAction> {
    #[allow(clippy::cast_possible_truncation)]
    let position = egui::pos2(point.x() as f32, point.y() as f32);
    let version = editor.native.session.version();
    for pressed in [true, false] {
        let recorded = editor.native.session.native_input_recorded_count();
        let report = editor.pointer_button(
            point,
            Some(if pressed {
                ElementState::Pressed
            } else {
                ElementState::Released
            }),
            MouseButton::Middle,
        );
        assert!(
            editor.native.session.native_input_recorded_count() > recorded,
            "the native middle-button callback must enter the input journal"
        );
        assert!(report.inputs().is_empty(), "{report:?}");
        assert!(report.manipulation_feedback().is_empty(), "{report:?}");
        assert!(!report.workspace_changed());
        assert_eq!(editor.native.session.version(), version);
        assert!(
            editor
                .native
                .session
                .view()
                .item(INSPECTOR)
                .unwrap()
                .is_active()
        );
        assert!(
            !editor
                .native
                .session
                .view()
                .item(THIRD)
                .unwrap()
                .is_active()
        );
        let actions = present_events(
            editor,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Middle,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                },
            ],
        );
        assert_eq!(editor.native.session.version(), version);
        assert_eq!(
            editor
                .native
                .session
                .view()
                .item(THIRD)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .selected(),
            Some(INSPECTOR.into()),
            "the shared renderer must not select the middle-clicked tab"
        );
        if !pressed {
            return actions;
        }
        assert!(actions.is_empty(), "a press must not close before release");
    }
    unreachable!()
}

fn request_close(editor: &mut Editor, action: EguiSurfaceAction) -> DockspaceClosePlan {
    let deferred = action.into_deferred();
    let version = editor.native.session.version();
    let mut abandoned = editor
        .native
        .poll_resolved_host_frame(&editor.context, &mut test_budget())
        .unwrap()
        .unwrap();
    abandoned
        .frame
        .submit_deferred_surface_action(deferred.clone())
        .unwrap();
    drop(abandoned);
    assert_eq!(editor.native.session.version(), version);
    assert_eq!(
        editor.native.session.pending_content_close_plans().count(),
        0
    );

    let mut frame = editor
        .native
        .poll_resolved_host_frame(&editor.context, &mut test_budget())
        .unwrap()
        .unwrap();
    frame
        .frame
        .submit_deferred_surface_action(deferred)
        .unwrap();
    let report = finish(frame);
    editor.native.settle_host_frame_report(&report).unwrap();
    let [
        HostInputOutcome::CloseRequested {
            plan,
            reused: false,
            origin: HostCloseRequestOrigin::Interaction,
        },
    ] = report.inputs()
    else {
        panic!("one completed middle click must request one close: {report:?}");
    };
    assert_eq!(plan.target(), ClosePlanTarget::Item { item: THIRD });
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.items()[0].item(), THIRD);
    assert_eq!(editor.native.session.version(), version);
    assert!(
        editor
            .native
            .session
            .view()
            .item(INSPECTOR)
            .unwrap()
            .is_active()
    );
    plan.clone()
}

fn defer_then_veto(editor: &mut Editor, plan: &DockspaceClosePlan) {
    let mut frame = editor
        .native
        .poll_resolved_host_frame(&editor.context, &mut test_budget())
        .unwrap()
        .unwrap();
    let submitted = frame
        .frame
        .resolve_close(
            plan.request(),
            plan.items()[0].token(),
            CloseDecision::Deferred,
        )
        .unwrap();
    let report = finish(frame);
    editor.native.settle_host_frame_report(&report).unwrap();
    let Some(HostInputOutcome::CloseDecisionProcessed {
        resolution: DockspaceCloseResolution::Deferred { continuation, .. },
        application: None,
        changed: false,
        ..
    }) = report.submitted_action_outcome(submitted)
    else {
        panic!("native UI close must retain the ordinary deferred token: {report:?}");
    };
    let continuation = *continuation;
    assert!(editor.native.session.view().item(THIRD).is_some());
    assert!(present_events(editor, Vec::new()).is_empty());
    assert_eq!(
        editor.native.session.pending_content_close_plans().count(),
        1
    );

    let mut frame = editor
        .native
        .poll_resolved_host_frame(&editor.context, &mut test_budget())
        .unwrap()
        .unwrap();
    frame
        .frame
        .continue_deferred_close(plan.request(), continuation, DeferredCloseDecision::Veto)
        .unwrap();
    let report = finish(frame);
    editor.native.settle_host_frame_report(&report).unwrap();
    assert!(!report.workspace_changed());
    assert_eq!(
        editor.native.session.pending_content_close_plans().count(),
        0
    );
}

#[test]
fn native_middle_body_close_keeps_selection_and_the_deferred_decision_workflow() {
    for desktop in [false, true] {
        let mut editor = Editor::new(desktop);
        let mut policy = editor.native.session.policy().clone();
        policy.set_close_capability(CloseCapability::DeferredAllowed);
        let mut frame = editor
            .native
            .poll_resolved_host_frame(&editor.context, &mut test_budget())
            .unwrap()
            .unwrap();
        frame.frame.replace_policy(policy).unwrap();
        let report = finish(frame);
        editor.native.settle_host_frame_report(&report).unwrap();
        let scene = editor.present();
        let [action]: [_; 1] = middle_click(&mut editor, scene.tab_point(THIRD))
            .try_into()
            .unwrap();
        let origin = action.origin();
        let plan = request_close(&mut editor, action);
        defer_then_veto(&mut editor, &plan);
        let scene = editor.present();

        let [action]: [_; 1] = middle_click(&mut editor, scene.tab_point(THIRD))
            .try_into()
            .unwrap();
        assert_eq!(
            action.origin(),
            origin,
            "the widget event slot stays stable"
        );
        let plan = request_close(&mut editor, action);
        let mut frame = editor
            .native
            .poll_resolved_host_frame(&editor.context, &mut test_budget())
            .unwrap()
            .unwrap();
        let submitted = frame
            .frame
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        let report = finish(frame);
        editor.native.settle_host_frame_report(&report).unwrap();
        assert!(matches!(
            report.submitted_action_outcome(submitted),
            Some(HostInputOutcome::CloseDecisionProcessed {
                application: Some(Ok(DockspaceCloseOutcome::ItemClosed {
                    item: THIRD,
                    root: INSPECTOR_ROOT,
                })),
                changed: true,
                ..
            })
        ));
        assert!(report.workspace_changed());
        assert!(editor.native.session.view().item(THIRD).is_none());
        assert!(
            editor
                .native
                .session
                .view()
                .item(INSPECTOR)
                .unwrap()
                .is_active()
        );
        for id in (1..=8).filter(|id| *id != THIRD.get()) {
            assert!(editor.native.session.view().item(ItemId::new(id)).is_some());
        }
        assert_eq!(
            editor.native.session.pending_content_close_plans().count(),
            0
        );
        assert!(present_events(&mut editor, Vec::new()).is_empty());
        assert!(!editor.native.session.has_active_gesture());
        editor.assert_quiescent();
    }
}

#[test]
fn disabling_native_middle_tab_close_keeps_raw_button_delivery_without_ui_actions() {
    for desktop in [false, true] {
        let mut editor = Editor::new(desktop);
        editor.native.set_input(egui_dockspace::DockInputConfig {
            middle_click_closes_tab: false,
            ..Default::default()
        });
        let scene = editor.present();
        assert!(middle_click(&mut editor, scene.tab_point(THIRD)).is_empty());
        assert_eq!(
            editor.native.session.pending_content_close_plans().count(),
            0
        );
        assert!(present_events(&mut editor, Vec::new()).is_empty());
        for id in 1..=8 {
            assert!(editor.native.session.view().item(ItemId::new(id)).is_some());
        }
        assert!(!editor.native.session.has_active_gesture());
        editor.assert_quiescent();
    }
}
