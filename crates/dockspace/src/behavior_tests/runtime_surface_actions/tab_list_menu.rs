//! Regression coverage for popup placement and scrolling through host actions.

use super::*;
use crate::runtime::{DockspaceHostFrame, DockspaceRuntimeError, SurfaceTabListNavigation};

fn measure(frame: &mut DockspaceHostFrame<'_>, bounds: LogicalRect, label_width: f64, height: f64) {
    frame
        .measure_surface_with(SURFACE, |request| {
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
                SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                    SurfaceMeasurementAnswer::TabIntrinsic(label_width)
                }
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

fn open_menu(bounds: LogicalRect, label_width: f64, height: f64) -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::tabs((1..=12).map(ItemId::new))),
    )])
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, bounds, label_width, height);
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
    measure(&mut frame, bounds, label_width, height);
    frame.commit().unwrap();
    session
}

#[test]
fn a_full_width_tab_list_fits_a_fractional_popup_plane_origin() {
    let extent = LogicalRect::new(0.3, 0.0, 600.0, 380.0).unwrap();
    let mut session = open_menu(extent, 720.0, 200.0);
    inspect_surface(&mut session, |plan| {
        let popup = plan.tab_list_menus().next().unwrap().bounds();
        assert_eq!(popup.x(), extent.x());
        assert_eq!(popup.max().x(), extent.max().x());
    });
}

#[test]
fn oversized_menu_rows_stay_aligned_after_home_and_reveal() {
    let extent = LogicalRect::new(0.0, 0.3, 600.0, 380.0).unwrap();
    let mut session = open_menu(extent, 110.0, 25.0);
    let scrolled = 11.1;
    let apply = |session: &mut DockspaceSession, action| {
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(action).unwrap();
        measure(&mut frame, extent, 110.0, 25.0);
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
