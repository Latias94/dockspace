use std::convert::Infallible;

use dockspace::geometry::{LogicalPoint, LogicalRect, LogicalSize};
use dockspace::model::{
    DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, FloatingPresentationId, ItemId, RootId, SurfaceId,
};
use dockspace::runtime::{
    DeferredSurfaceAction, DockspaceHostFrame, DockspaceSession, HostInputOutcome,
    RetainedTabListMenu, SurfaceMeasurementAnswer as Answer, SurfaceMeasurementRequest as Question,
    SurfacePaintPlan, SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
    SurfacePresentationResult, SurfaceTabListInput as Input,
    SurfaceTabListNavigation as Navigation, TabGroupDragRegionKind, TabListMenuMetrics,
    TabListMenuPaintRecord, TabListMenuRowId, TabStripControlKind, TabStripControlMetric,
    TabStripControlMetrics, TabStripControlPaintRecord, TabStripControlPlacement, TabStripMetrics,
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
    measure_popup(frame, popup_height, None);
}

fn measure_popup(
    frame: &mut DockspaceHostFrame<'_>,
    popup_height: f64,
    popup_bounds: Option<LogicalRect>,
) {
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
                Question::DockBounds { .. } => Answer::Bounds(bounds),
                Question::PopupPlaneBounds { .. } => Answer::Bounds(popup_bounds.unwrap_or(bounds)),
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

fn opener_in(step: Step<'_>) -> Option<TabStripControlPaintRecord> {
    match step {
        Step::Operation {
            operation: Operation::TabStripControl(control),
            ..
        } if control.kind() == TabStripControlKind::TabListMenu => Some(control),
        _ => None,
    }
}

#[test]
fn opener_cannot_rebind_to_another_session_epoch_or_abandoned_output() {
    let mut original = session();
    let mut opener = None;
    present(&mut original, 80.0, None, |plan, step| {
        if let Some(control) = opener_in(step) {
            assert!(plan.receiver_for_tab_strip_control(control).is_some());
            assert!(plan.prepare_tab_strip_control_activation(control).is_some());
            opener = Some(control);
        }
    });
    let opener = opener.unwrap();
    present(&mut original, 80.0, None, |plan, step| {
        if let Some(control) = opener_in(step) {
            assert_eq!(control.input_id(), opener.input_id());
            assert!(plan.prepare_tab_strip_control_activation(opener).is_some());
        }
    });

    let reject = |plan: SurfacePaintPlan<'_>, control: TabStripControlPaintRecord| {
        assert!(plan.receiver_for_tab_strip_control(control).is_none());
        assert!(plan.prepare_tab_strip_control_activation(control).is_none());
    };

    let open = open_action(&mut original, 80.0);
    present(&mut original, 80.0, Some(open), |plan, step| {
        if let Some(control) = opener_in(step) {
            assert!(control.menu_open());
            assert_eq!(control.input_id(), opener.input_id());
            reject(plan, opener);
            assert!(plan.prepare_tab_strip_control_activation(control).is_some());
        }
    });

    let mut foreign = session();
    present(&mut foreign, 80.0, None, |plan, step| {
        if let Some(control) = opener_in(step) {
            assert_eq!(control.visual_id(), opener.visual_id());
            assert_ne!(control.input_id(), opener.input_id());
            reject(plan, opener);
        }
    });

    let reset = original.prepare_layout_reset(layout()).unwrap();
    original
        .begin_layout_reset_frame(&reset)
        .unwrap()
        .commit()
        .unwrap();
    let mut dismiss = None;
    present(&mut original, 80.0, None, |plan, step| {
        if let Some(control) = opener_in(step) {
            assert!(control.menu_open());
            assert_eq!(control.visual_id(), opener.visual_id());
            assert_ne!(control.input_id(), opener.input_id());
            reject(plan, opener);
            dismiss = plan
                .prepare_tab_strip_control_activation(control)
                .map(|action| action.into_deferred());
        }
        if let Step::PopupMenu { menu, .. } = step {
            assert_ne!(menu.opener_input_id(), opener.input_id());
        }
    });
    assert!(dismiss.is_some());
    present(&mut original, 80.0, dismiss, |_, step| {
        assert!(!matches!(step, Step::PopupMenu { .. }));
    });

    let mut uncommitted = session();
    let abandoned = {
        let mut frame = uncommitted.begin_host_frame().unwrap();
        measure(&mut frame, 80.0);
        let mut captured = None;
        let receipt = frame
            .paint_plan(SURFACE)
            .unwrap()
            .unwrap()
            .execute_surface_program(|step| {
                captured = captured.or_else(|| opener_in(step));
                Ok::<_, Infallible>(())
            })
            .unwrap();
        frame.confirm_surface_painted(receipt).unwrap();
        captured.unwrap()
    };
    present(&mut uncommitted, 80.0, None, |plan, step| {
        if let Some(control) = opener_in(step) {
            assert_eq!(control.input_id(), abandoned.input_id());
            reject(plan, abandoned);
            assert!(plan.prepare_tab_strip_control_activation(control).is_some());
        }
    });
}

#[test]
fn tab_strip_scroll_and_group_drag_receivers_are_bound_to_their_output() {
    let mut original = session();
    let mut foreign = session();
    let mut checked = false;
    present(&mut original, 80.0, None, |plan, step| {
        if let Step::Operation {
            operation: Operation::TabBar(bar),
            ..
        } = step
        {
            let grip = bar
                .group_drag_region(TabGroupDragRegionKind::LeadingGrip)
                .unwrap();
            assert!(plan.receiver_for_tab_group_drag_region(grip).is_some());
            let receiver = plan.receiver_for_tab_strip_scroll(bar).unwrap();
            let bounds = receiver.bounds();
            let center = LogicalPoint::new(
                bounds.x() + bounds.width() / 2.0,
                bounds.y() + bounds.height() / 2.0,
            )
            .unwrap();
            assert!(
                plan.prepare_tab_strip_scroll_at(bar, center, 20.0)
                    .is_some()
            );
            present(&mut foreign, 80.0, None, |other_plan, step| {
                if let Step::Operation {
                    operation: Operation::TabBar(other_bar),
                    ..
                } = step
                {
                    assert_eq!(bar.visual_id(), other_bar.visual_id());
                    assert!(
                        other_plan
                            .receiver_for_tab_group_drag_region(grip)
                            .is_none()
                    );
                    assert!(other_plan.receiver_for_tab_strip_scroll(bar).is_none());
                    assert!(
                        other_plan
                            .prepare_tab_strip_scroll_at(bar, center, 20.0)
                            .is_none()
                    );
                    assert!(other_plan.retain_tab_strip(bar).is_none());
                    checked = true;
                }
            });
        }
    });
    assert!(checked);
}

#[test]
fn tab_body_and_close_receivers_are_bound_to_their_output() {
    let mut original = session();
    let mut foreign = session();
    let mut checked = false;
    present(&mut original, 80.0, None, |plan, step| {
        if let Step::Operation {
            operation: Operation::Tab(tab),
            ..
        } = step
            && tab.item() == Some(FIRST)
        {
            assert!(plan.receiver_for_tab_body(tab).is_some());
            assert!(plan.receiver_for_tab_close(tab).is_some());
            present(&mut foreign, 80.0, None, |other_plan, step| {
                if let Step::Operation {
                    operation: Operation::Tab(other_tab),
                    ..
                } = step
                    && other_tab.item() == Some(FIRST)
                {
                    assert_eq!(tab.visual_id(), other_tab.visual_id());
                    assert!(other_plan.receiver_for_tab_body(tab).is_none());
                    assert!(other_plan.receiver_for_tab_close(tab).is_none());
                    checked = true;
                }
            });
        }
    });
    assert!(checked);
}

#[test]
fn a_partially_clipped_list_button_can_open_its_menu() {
    let surface = DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(9),
            DockspaceNode::central_tabs([] as [ItemId; 0]),
        ),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(
            RootId::new(10),
            DockspaceNode::tabs((1..=12).map(ItemId::new)),
        ),
        LogicalRect::new(60.0, -40.0, 280.0, 300.0).unwrap(),
    ));
    let layout = DockspaceLayout::new([surface]).unwrap();
    let mut session = DockspaceSession::from_layout(layout, Default::default()).unwrap();
    let open = open_action(&mut session, 80.0);
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_deferred_surface_action(open).unwrap();
    // The popup plane clips the button further while leaving room below it.
    let plane = LogicalRect::new(0.0, 12.0, 640.0, 588.0).unwrap();
    measure_popup(&mut frame, 80.0, Some(plane));
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let mut opener_bounds = None;
    let mut menu_bounds = None;
    plan.visit_surface_program(|step| {
        if let Some(control) = opener_in(step) {
            assert!(control.menu_open());
            assert!(control.enabled());
            assert!(plan.receiver_for_tab_strip_control(control).is_some());
            opener_bounds = Some(control.bounds());
        }
        if let Step::PopupMenu { menu, .. } = step {
            menu_bounds = Some(menu.bounds());
        }
        Ok::<_, Infallible>(())
    })
    .unwrap();
    let opener = opener_bounds.unwrap();
    let menu = menu_bounds.unwrap();
    assert_eq!(opener.y(), 0.0);
    assert!(opener.y() < plane.y());
    assert!(opener.max().y() > plane.y());
    assert_eq!(menu.y(), opener.max().y());
    assert!(plane.contains(menu.min()));
    assert!(menu.max().x() <= plane.max().x());
    assert!(menu.max().y() <= plane.max().y());
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
fn queued_opening_input_expires_when_the_menu_is_replaced() {
    let mut original = session();
    let mut opener = None;
    present(&mut original, 80.0, None, |plan, step| {
        if let Some(control) = opener_in(step) {
            opener = plan.retain_tab_list_opener(control);
        }
    });
    let opener = opener.unwrap();
    let open = open_action(&mut original, 80.0);
    let (menu, row) = with_menu(&mut original, 80.0, Some(open), retained_and_last);
    assert!(menu.can_continue_opening(&opener));

    let mut foreign = session();
    let open = open_action(&mut foreign, 80.0);
    let (foreign_menu, _) = with_menu(&mut foreign, 80.0, Some(open), retained_and_last);
    assert!(!foreign_menu.can_continue_opening(&opener));

    let (_, dismiss) = menu.resolve(row, Input::Dismiss).unwrap();
    present(&mut original, 80.0, Some(dismiss), |_, _| {});
    let open = open_action(&mut original, 80.0);
    let (reopened, _) = with_menu(&mut original, 80.0, Some(open), retained_and_last);
    assert!(!reopened.can_continue_opening(&opener));
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
fn a_displayed_backdrop_can_dismiss_its_popup_after_the_focused_row_changes() {
    let mut session = session();
    let open = open_action(&mut session, 80.0);
    let (focus, dismiss) = with_menu(&mut session, 80.0, Some(open), |plan, menu| {
        let row = menu.rows().find(|row| row.focused()).unwrap().input_id();
        let (_, focus) = plan
            .retain_tab_list_menu(menu)
            .unwrap()
            .resolve(row, Input::Navigate(Navigation::Last))
            .unwrap();
        let dismiss = plan
            .prepare_tab_list_menu_dismiss(menu.backdrop())
            .unwrap()
            .into_deferred();
        (focus, dismiss)
    });
    with_menu(&mut session, 80.0, Some(focus), |_, menu| {
        let focused = menu.rows().find(|row| row.focused()).unwrap();
        assert_eq!(focused.item(), Some(LAST));
    });
    let mut has_popup = false;
    present(&mut session, 80.0, Some(dismiss), |_, step| {
        has_popup |= matches!(step, Step::PopupMenu { .. });
    });
    assert!(!has_popup);
    assert!(session.view().item(FIRST).unwrap().is_active());
}

#[test]
fn menu_records_cannot_be_rebound_to_another_output() {
    let mut original = session();
    let mut foreign = session();
    let foreign_open = open_action(&mut foreign, 80.0);
    with_menu(&mut foreign, 80.0, Some(foreign_open), |_, _| {});
    let open = open_action(&mut original, 80.0);
    let backdrop = with_menu(
        &mut original,
        80.0,
        Some(open),
        |original_plan, old_menu| {
            let old_backdrop = old_menu.backdrop();
            assert!(
                original_plan
                    .receiver_for_tab_list_menu_frame(old_menu)
                    .is_some()
            );
            assert!(
                original_plan
                    .receiver_for_tab_list_menu_scroll(old_menu)
                    .is_some()
            );
            assert!(
                original_plan
                    .receiver_for_tab_list_menu_backdrop(old_backdrop)
                    .is_some()
            );
            with_menu(&mut foreign, 80.0, None, |plan, menu| {
                assert_eq!(old_menu.visual_id(), menu.visual_id());
                assert_eq!(old_backdrop.visual_id(), menu.backdrop().visual_id());
                assert!(plan.receiver_for_tab_list_menu_frame(old_menu).is_none());
                assert!(plan.receiver_for_tab_list_menu_scroll(old_menu).is_none());
                assert!(
                    plan.receiver_for_tab_list_menu_backdrop(old_backdrop)
                        .is_none()
                );
                assert!(plan.prepare_tab_list_menu_dismiss(old_backdrop).is_none());
            });
            old_backdrop
        },
    );

    let discarded_backdrop = {
        let mut frame = original.begin_host_frame().unwrap();
        measure(&mut frame, 40.0);
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let mut backdrop = None;
        plan.visit_surface_program(|step| {
            if let Step::PopupBackdrop(record) = step {
                backdrop = Some(record);
            }
            Ok::<_, Infallible>(())
        })
        .unwrap();
        backdrop.unwrap()
    };
    with_menu(&mut original, 40.0, None, |plan, menu| {
        assert_eq!(menu.backdrop().visual_id(), backdrop.visual_id());
        assert_eq!(menu.backdrop().visual_id(), discarded_backdrop.visual_id());
        assert!(plan.receiver_for_tab_list_menu_backdrop(backdrop).is_none());
        assert!(plan.prepare_tab_list_menu_dismiss(backdrop).is_none());
        assert!(
            plan.prepare_tab_list_menu_dismiss(discarded_backdrop)
                .is_none()
        );
        assert!(
            plan.prepare_tab_list_menu_dismiss(menu.backdrop())
                .is_some()
        );
    });

    let reset = original.prepare_layout_reset(layout()).unwrap();
    original
        .begin_layout_reset_frame(&reset)
        .unwrap()
        .commit()
        .unwrap();
    with_menu(&mut original, 80.0, None, |plan, menu| {
        assert_eq!(menu.backdrop().visual_id(), backdrop.visual_id());
        assert!(plan.receiver_for_tab_list_menu_backdrop(backdrop).is_none());
        assert!(plan.prepare_tab_list_menu_dismiss(backdrop).is_none());
        assert!(
            plan.prepare_tab_list_menu_dismiss(menu.backdrop())
                .is_some()
        );
    });
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
fn scrolling_away_and_back_releases_keyboard_reveal_in_any_batching() {
    for (batches, expected_offset) in [
        (&[][..], 440.0),
        (&[&[0.0][..]][..], 440.0),
        (&[&[40.0][..]][..], 440.0),
        (&[&[-40.0, 40.0][..]][..], 400.0),
        (&[&[-40.0][..], &[40.0][..]][..], 400.0),
    ] {
        let mut session = session();
        let open = open_action(&mut session, 80.0);
        let mut action = Some(with_menu(&mut session, 80.0, Some(open), |plan, menu| {
            plan.prepare_tab_list_menu_navigation(menu, Navigation::Last)
                .unwrap()
                .into_deferred()
        }));
        for batch in batches {
            action = Some(with_menu(
                &mut session,
                80.0,
                action.take(),
                |plan, menu| {
                    plan.prepare_tab_list_menu_scroll_sequence(menu, batch.iter().copied())
                        .unwrap()
                        .into_deferred()
                },
            ));
        }
        with_menu(&mut session, 80.0, action, |_, menu| {
            assert_eq!(menu.scroll_offset(), 400.0, "{batches:?}");
        });
        with_menu(&mut session, 40.0, None, |_, menu| {
            assert_eq!(menu.scroll_offset(), expected_offset, "{batches:?}");
            let last = menu.rows().last().unwrap();
            assert!(last.focused());
            assert_eq!(
                last.interaction_bounds().is_some(),
                expected_offset == 440.0
            );
        });
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
