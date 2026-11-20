use std::convert::Infallible;

use dockspace::geometry::{LogicalRect, LogicalSize};
use dockspace::model::{
    DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, RootId,
    SurfaceId,
};
use dockspace::runtime::{
    DeferredSurfaceAction, DockspaceHostFrame, DockspaceSession, HostInputOutcome,
    RetainedTabListMenu, SurfaceMeasurementAnswer as Answer, SurfaceMeasurementRequest as Question,
    SurfacePaintPlan, SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
    SurfacePresentationResult, SurfaceTabListInput as Input,
    SurfaceTabListNavigation as Navigation, TabListMenuMetrics, TabListMenuPaintRecord,
    TabListMenuRowId, TabStripControlKind, TabStripControlMetric, TabStripControlMetrics,
    TabStripControlPlacement, TabStripMetrics,
};

const SURFACE: SurfaceId = SurfaceId::new(4);
const FIRST: ItemId = ItemId::new(1);
const LAST: ItemId = ItemId::new(12);

fn layout() -> DockspaceLayout {
    let tabs = DockspaceNode::central_tabs((1..=12).map(ItemId::new));
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(9), tabs),
    )])
    .unwrap()
}

fn session() -> DockspaceSession {
    DockspaceSession::from_layout(layout(), Default::default()).unwrap()
}

fn measure(frame: &mut DockspaceHostFrame<'_>, popup_height: f64) {
    let bounds = LogicalRect::new(0.0, 0.0, 640.0, 600.0).unwrap();
    let controls = TabStripControlMetrics::new(0.0)
        .unwrap()
        .with_tab_list_menu(
            TabStripControlMetric::new(20.0, TabStripControlPlacement::ReservedTrailing).unwrap(),
        );
    let menu = TabListMenuMetrics::new(40.0, 0.0, 0.0, 0.0, popup_height, 0.0).unwrap();
    frame
        .measure_surface_with(SURFACE, |request| {
            Ok::<_, dockspace::runtime::DockspaceRuntimeError>(match request {
                Question::DockBounds { .. } | Question::PopupPlaneBounds { .. } => {
                    Answer::Bounds(bounds)
                }
                Question::PaneSizing { .. } => {
                    Answer::PaneSizing(LogicalSize::new(20.0, 30.0).unwrap().into())
                }
                Question::TabIntrinsic { .. } => Answer::TabIntrinsic(100.0),
                Question::TabStrip { .. } => Answer::TabStrip(
                    TabStripMetrics::new(0.0, 0.0)
                        .unwrap()
                        .with_controls(controls)
                        .with_tab_list_menu(menu),
                ),
            })
        })
        .unwrap();
}

fn present(
    session: &mut DockspaceSession,
    height: f64,
    action: Option<DeferredSurfaceAction>,
    mut visit: impl FnMut(SurfacePaintPlan<'_>, Step<'_>),
) {
    let mut frame = session.begin_host_frame().unwrap();
    if let Some(action) = action {
        frame.submit_deferred_surface_action(action).unwrap();
    }
    measure(&mut frame, height);
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let receipt = plan
        .execute_surface_program(|step| {
            visit(plan, step);
            Ok::<_, Infallible>(())
        })
        .unwrap();
    frame.confirm_surface_painted(receipt).unwrap();
    let mut report = frame.commit().unwrap();
    assert!(
        !report
            .inputs()
            .iter()
            .any(|input| matches!(input, HostInputOutcome::StaleRejected { .. })),
        "{report:?}",
    );
    let outputs = report.take_painted_outputs();
    assert_eq!(outputs.len(), 1);
    for output in outputs {
        session
            .report_surface_presentation(output, SurfacePresentationResult::Presented)
            .unwrap();
    }
    session.begin_host_frame().unwrap().commit().unwrap();
}

fn with_menu<T>(
    session: &mut DockspaceSession,
    height: f64,
    action: Option<DeferredSurfaceAction>,
    inspect: impl FnOnce(SurfacePaintPlan<'_>, TabListMenuPaintRecord<'_>) -> T,
) -> T {
    let mut inspect = Some(inspect);
    let mut result = None;
    present(session, height, action, |plan, step| {
        if let Step::PopupMenu { menu, .. } = step {
            result = Some(inspect.take().unwrap()(plan, menu));
        }
    });
    result.expect("an open menu must appear in the output")
}

fn open_action(session: &mut DockspaceSession, height: f64) -> DeferredSurfaceAction {
    let mut action = None;
    present(session, height, None, |plan, step| {
        if let Step::Operation {
            operation: Operation::TabStripControl(control),
            ..
        } = step
            && control.kind() == TabStripControlKind::TabListMenu
        {
            action = plan
                .prepare_tab_strip_control_activation(control)
                .map(|action| action.into_deferred());
        }
    });
    action.expect("the overflow control must be available")
}

fn retained_and_last(
    plan: SurfacePaintPlan<'_>,
    menu: TabListMenuPaintRecord<'_>,
) -> (RetainedTabListMenu, TabListMenuRowId) {
    (
        plan.retain_tab_list_menu(menu).unwrap(),
        menu.rows().last().unwrap().input_id(),
    )
}

#[test]
fn retained_menu_rows_expire_across_session_epoch_roster_and_popup_changes() {
    let mut original = session();
    let open = open_action(&mut original, 80.0);
    let (old, old_row) = with_menu(&mut original, 80.0, Some(open), retained_and_last);

    let mut foreign = session();
    let open = open_action(&mut foreign, 80.0);
    let (other, other_row) = with_menu(&mut foreign, 80.0, Some(open), retained_and_last);
    assert_eq!(old_row.visual_id(), other_row.visual_id());
    assert!(!other.can_continue_from(&old));
    assert!(other.resolve(old_row, Input::Activate).is_none());
    assert!(foreign.view().item(FIRST).unwrap().is_active());
    with_menu(&mut original, 80.0, None, |_, menu| {
        let row = menu.rows().next().unwrap();
        with_menu(&mut foreign, 80.0, None, |plan, same_menu| {
            assert_eq!(
                row.visual_id(),
                same_menu.rows().next().unwrap().visual_id()
            );
            assert!(plan.receiver_for_tab_list_menu_row(row).is_none());
            assert!(plan.prepare_tab_list_menu_row_activation(row).is_none());
            assert!(plan.prepare_tab_list_menu_reveal(row).is_none());
        });
    });

    let reset = original.prepare_layout_reset(layout()).unwrap();
    original
        .begin_layout_reset_frame(&reset)
        .unwrap()
        .commit()
        .unwrap();
    let (reset_menu, reset_row) = with_menu(&mut original, 80.0, None, retained_and_last);
    assert_eq!(old_row.visual_id(), reset_row.visual_id());
    assert!(!reset_menu.can_continue_from(&old));
    assert!(reset_menu.resolve(old_row, Input::Activate).is_none());

    let mut frame = original.begin_host_frame().unwrap();
    frame.set_item_visibility_current(FIRST, false).unwrap();
    frame.commit().unwrap();
    let (changed_menu, changed_row) = with_menu(&mut original, 80.0, None, retained_and_last);
    assert_eq!(reset_row.visual_id(), changed_row.visual_id());
    assert!(!changed_menu.can_continue_from(&reset_menu));
    assert!(changed_menu.resolve(reset_row, Input::Activate).is_none());

    let (_, dismiss) = changed_menu.resolve(changed_row, Input::Dismiss).unwrap();
    present(&mut original, 80.0, Some(dismiss), |_, _| {});
    let open = open_action(&mut original, 80.0);
    let (reopened, reopened_row) = with_menu(&mut original, 80.0, Some(open), retained_and_last);
    assert!(!reopened.can_continue_from(&changed_menu));
    assert!(reopened.resolve(changed_row, Input::Activate).is_none());
    let (_, activate) = reopened.resolve(reopened_row, Input::Activate).unwrap();
    present(&mut original, 80.0, Some(activate), |_, _| {});
    assert!(original.view().item(LAST).unwrap().is_active());
}

#[test]
fn menu_keyboard_reveal_uses_the_actual_viewport_and_preserves_manual_scroll() {
    for split_frames in [false, true] {
        let mut session = session();
        let open = open_action(&mut session, 480.0);
        let (retained, first, last) = with_menu(&mut session, 480.0, Some(open), |plan, menu| {
            (
                plan.retain_tab_list_menu(menu).unwrap(),
                menu.rows().next().unwrap().input_id(),
                menu.rows().last().unwrap().input_id(),
            )
        });
        let (target, end) = retained
            .resolve(first, Input::Navigate(Navigation::Last))
            .unwrap();
        assert_eq!(target, last);
        let mut end = Some(end);
        if split_frames {
            present(&mut session, 480.0, end.take(), |_, _| {});
        }

        let scroll = with_menu(&mut session, 80.0, end, |plan, menu| {
            let row = menu.rows().last().unwrap();
            assert_eq!(menu.scroll_offset(), 400.0);
            assert!(row.focused());
            assert!(row.interaction_bounds().is_some());
            assert!(
                plan.retain_tab_list_menu(menu)
                    .unwrap()
                    .can_continue_from(&retained)
            );
            assert_eq!(row.input_id(), last);
            plan.prepare_tab_list_menu_scroll_by(menu, -400.0)
                .unwrap()
                .into_deferred()
        });
        with_menu(&mut session, 80.0, Some(scroll), |_, menu| {
            assert_eq!(menu.scroll_offset(), 0.0);
            let row = menu.rows().last().unwrap();
            assert!(row.focused());
            assert!(row.interaction_bounds().is_none());
        });
        with_menu(&mut session, 120.0, None, |_, menu| {
            assert_eq!(menu.scroll_offset(), 0.0);
        });
        assert!(session.view().item(FIRST).unwrap().is_active());
    }
}

#[test]
fn explicit_row_reveal_uses_the_actual_viewport_without_moving_focus() {
    let mut session = session();
    let open = open_action(&mut session, 480.0);
    let reveal = with_menu(&mut session, 480.0, Some(open), |plan, menu| {
        plan.prepare_tab_list_menu_reveal(menu.rows().last().unwrap())
            .unwrap()
            .into_deferred()
    });
    with_menu(&mut session, 80.0, Some(reveal), |_, menu| {
        assert_eq!(menu.scroll_offset(), 400.0);
        assert!(menu.rows().next().unwrap().focused());
        let row = menu.rows().last().unwrap();
        assert!(row.interaction_bounds().is_some());
        assert!(!row.focused());
    });
}
