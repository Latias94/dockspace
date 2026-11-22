//! Regression coverage for popup placement and scrolling through host actions.

use super::*;
use crate::runtime::{
    DockspaceHostFrame, DockspaceRuntimeError, SurfaceTabInput, SurfaceTabListNavigation,
};

fn measure(
    frame: &mut DockspaceHostFrame<'_>,
    surface: SurfaceId,
    bounds: LogicalRect,
    tab_intrinsic: SurfaceMeasurementAnswer,
    height: f64,
) {
    frame
        .measure_surface_with(surface, |request| {
            Ok::<_, DockspaceRuntimeError>(match request {
                SurfaceMeasurementRequest::DockBounds { .. }
                | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                    SurfaceMeasurementAnswer::Bounds(bounds)
                }
                SurfaceMeasurementRequest::PaneSizing { .. } => {
                    SurfaceMeasurementAnswer::PaneSizing(
                        LogicalSize::new(10.0, 10.0).unwrap().into(),
                    )
                }
                SurfaceMeasurementRequest::TabIntrinsic { .. } => tab_intrinsic,
                SurfaceMeasurementRequest::TabStrip { .. } => {
                    let controls = TabStripControlMetrics::new(0.0)
                        .unwrap()
                        .with_tab_list_menu(
                            TabStripControlMetric::new(
                                20.0,
                                TabStripControlPlacement::ReservedTrailing,
                            )
                            .unwrap(),
                        );
                    let menu = TabListMenuMetrics::new(44.0, 0.0, 0.0, 0.0, height, 0.0).unwrap();
                    SurfaceMeasurementAnswer::TabStrip(
                        TabStripMetrics::new(0.0, 0.0)
                            .unwrap()
                            .with_controls(controls)
                            .with_tab_list_menu(menu),
                    )
                }
            })
        })
        .unwrap();
}

fn open_menu(
    bounds: LogicalRect,
    tab_intrinsic: SurfaceMeasurementAnswer,
    height: f64,
) -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::tabs((1..=12).map(ItemId::new))),
    )])
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, SURFACE, bounds, tab_intrinsic, height);
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let control = plan
        .tab_strip_controls()
        .find(|control| control.kind() == TabStripControlKind::TabListMenu)
        .unwrap();
    let open = plan
        .prepare_tab_strip_control_activation(control)
        .unwrap()
        .into_deferred();
    frame.commit().unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_deferred_surface_action(open).unwrap();
    measure(&mut frame, SURFACE, bounds, tab_intrinsic, height);
    frame.commit().unwrap();
    session
}

#[test]
fn a_full_width_tab_list_fits_a_fractional_popup_plane_origin() {
    let extent = LogicalRect::new(0.3, 0.0, 600.0, 380.0).unwrap();
    let mut session = open_menu(extent, SurfaceMeasurementAnswer::TabIntrinsic(720.0), 200.0);
    inspect_surface(&mut session, |plan| {
        let popup = plan.tab_list_menus().next().unwrap().bounds();
        assert_eq!(popup.x(), extent.x());
        assert_eq!(popup.max().x(), extent.max().x());
    });
}

#[test]
fn tab_list_rows_use_their_own_intrinsic_width() {
    let extent = LogicalRect::new(0.0, 0.0, 260.0, 380.0).unwrap();
    let mut session = open_menu(
        extent,
        SurfaceMeasurementAnswer::TabIntrinsicWithMenuWidth {
            tab_width: 12.0,
            menu_width: 160.0,
        },
        180.0,
    );
    inspect_surface(&mut session, |plan| {
        let menu = plan.tab_list_menus().next().unwrap();
        assert_eq!(menu.viewport().width(), 160.0);
        assert!(menu.rows().all(|row| row.bounds().width() == 160.0));
    });
}

#[test]
fn oversized_menu_rows_stay_aligned_after_home_and_reveal() {
    let extent = LogicalRect::new(0.0, 0.3, 600.0, 380.0).unwrap();
    let intrinsic = SurfaceMeasurementAnswer::TabIntrinsic(110.0);
    let mut session = open_menu(extent, intrinsic, 25.0);
    let scrolled = 11.1;
    let apply = |session: &mut DockspaceSession, action| {
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(action).unwrap();
        measure(&mut frame, SURFACE, extent, intrinsic, 25.0);
        frame.commit().unwrap();
    };
    for explicit_reveal in [false, true] {
        let scroll = inspect_surface(&mut session, |plan| {
            let menu = plan.tab_list_menus().next().unwrap();
            assert!((menu.viewport().height() - 25.0).abs() < 1e-8);
            assert_eq!(menu.rows().next().unwrap().bounds().height(), 44.0);
            let center = plan
                .receiver_for_tab_list_menu_scroll(menu)
                .unwrap()
                .center();
            plan.prepare_tab_list_menu_scroll_at(menu, center, scrolled)
                .unwrap()
                .into_deferred()
        });
        apply(&mut session, scroll);
        for offset in [scrolled, 0.0, 0.0] {
            let reveal = inspect_surface(&mut session, |plan| {
                let menu = plan.tab_list_menus().next().unwrap();
                assert_eq!(menu.scroll_offset(), offset);
                if explicit_reveal {
                    plan.prepare_tab_list_menu_reveal(menu.rows().next().unwrap())
                } else {
                    plan.prepare_tab_list_menu_navigation(menu, SurfaceTabListNavigation::First)
                }
                .unwrap()
                .into_deferred()
            });
            apply(&mut session, reveal);
        }
    }
}

#[test]
fn a_presented_sibling_tab_stays_operable_while_another_surface_toggles_its_list() {
    let bounds = LogicalRect::new(0.0, 0.0, 420.0, 380.0).unwrap();
    let intrinsic = SurfaceMeasurementAnswer::TabIntrinsic(110.0);
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs((1..=12).map(ItemId::new))),
        ),
        DockspaceSurfaceLayout::new(
            TARGET_SURFACE,
            DockspaceRootLayout::new(
                TARGET_ROOT,
                DockspaceNode::central_tabs((13..=24).map(ItemId::new)),
            ),
        ),
    ])
    .unwrap();

    let paint = |session: &mut DockspaceSession, surface, target, action| {
        let mut frame = session.begin_host_frame().unwrap();
        if let Some(action) = action {
            frame.submit_deferred_surface_action(action).unwrap();
        }
        measure(&mut frame, surface, bounds, intrinsic, 100.0);
        let plan = frame.paint_plan(surface).unwrap().unwrap();
        let opener = plan
            .tab_strip_controls()
            .find(|control| control.kind() == TabStripControlKind::TabListMenu)
            .unwrap();
        let toggle = plan
            .prepare_tab_strip_control_activation(opener)
            .unwrap()
            .into_deferred();
        let select = plan
            .prepare_tab_select(target)
            .map(PreparedSurfaceAction::into_deferred);
        let strip = plan.retain_tab_strip(plan.tab_bars().next().unwrap());
        let receipt = plan.execute_surface_program(|_| Ok::<_, ()>(())).unwrap();
        frame.confirm_surface_painted(receipt).unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        let mut report = frame.commit().unwrap();
        for output in report.take_painted_outputs() {
            session
                .report_surface_presentation(output, SurfacePresentationResult::Presented)
                .unwrap();
        }
        session.begin_host_frame().unwrap().commit().unwrap();
        (toggle, select, opener.menu_open(), strip)
    };

    for (opener_surface, close_menu) in [(SURFACE, false), (SURFACE, true), (TARGET_SURFACE, false)]
    {
        let mut session =
            DockspaceSession::from_layout(layout.clone(), DockPolicy::default()).unwrap();
        let sibling_tab = ItemId::new(14);
        let (open_a, _, a_open, _) = paint(&mut session, SURFACE, SECOND, None);
        let (open_b, select_b, b_open, original_strip) =
            paint(&mut session, TARGET_SURFACE, sibling_tab, None);
        assert!(!a_open && !b_open);
        let presented_b = session.presented_surface(TARGET_SURFACE).unwrap();
        let (opener, item) = if opener_surface == SURFACE {
            (open_a, SECOND)
        } else {
            (open_b, sibling_tab)
        };
        let (dismiss, _, opened, _) = paint(&mut session, opener_surface, item, Some(opener));
        assert!(opened);
        if close_menu {
            let (_, _, opened, _) = paint(&mut session, opener_surface, item, Some(dismiss));
            assert!(!opened);
        }

        let unchanged = opener_surface != TARGET_SURFACE;
        assert_eq!(
            session.presented_surface(TARGET_SURFACE) == Some(presented_b),
            unchanged
        );
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .submit_deferred_surface_action(select_b.unwrap())
            .unwrap();
        let report = frame.commit().unwrap();
        assert_eq!(report.workspace_changed(), unchanged, "{report:?}");
        assert_eq!(
            session.view().item(sibling_tab).unwrap().is_active(),
            unchanged
        );
        let (_, _, opened, current_strip) = paint(&mut session, TARGET_SURFACE, sibling_tab, None);
        if unchanged {
            assert!(!opened);
            let current = current_strip.unwrap();
            let last = sibling_tab.into();
            let receiver = current
                .continuation_receiver(&original_strip.unwrap(), last)
                .expect("another surface's popup leaves the committed strip operable");
            let first = ItemId::new(13);
            let activation = current
                .resolve(
                    receiver,
                    last,
                    [
                        SurfaceTabInput::Navigate(SurfaceTabNavigation::First),
                        SurfaceTabInput::Activate,
                    ],
                )
                .unwrap();
            assert_eq!(activation.tab_focus_target(), Some(first.into()));
            let mut frame = session.begin_host_frame().unwrap();
            frame.submit_deferred_surface_action(activation).unwrap();
            assert!(frame.commit().unwrap().workspace_changed());
            assert!(session.view().item(first).unwrap().is_active());
        } else {
            assert!(matches!(
                report.inputs(),
                [HostInputOutcome::StaleRejected { .. }]
            ));
            assert!(
                !current_strip
                    .unwrap()
                    .same_navigation_scope(&original_strip.unwrap())
            );
            let _ = paint(&mut session, SURFACE, SECOND, None);
            session.enable_surface_pointer(SURFACE).unwrap();
            let mut frame = session.begin_host_frame().unwrap();
            let plan = frame.paint_plan(TARGET_SURFACE).unwrap().unwrap();
            let menu = plan.tab_list_menus().next().unwrap();
            assert!(plan.tab_list_menu_is_presented(menu));
            let row = menu
                .rows()
                .find(|row| row.interaction_bounds().is_some())
                .unwrap();
            assert!(plan.tab_list_menu_row_is_presented(row));
        }
    }
}
