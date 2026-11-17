//! Menu scroll sequences preserve ordered boundaries without partial actions.

use super::*;
use crate::geometry::{LogicalRect, LogicalSize};
use crate::ids::ItemId;
use crate::model::{
    DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, RootId, SurfaceId,
};
use crate::policy::DockPolicy;
use crate::runtime::{
    DockspaceRuntimeError, DockspaceSession, SurfaceMeasurementAnswer, SurfaceMeasurementRequest,
    TabListMenuMetrics, TabStripControlKind, TabStripControlMetric, TabStripControlMetrics,
    TabStripControlPlacement, TabStripMetrics,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FIRST: ItemId = ItemId::new(1);

fn inspect<T>(
    session: &mut DockspaceSession,
    prepare: impl FnOnce(SurfacePaintPlan<'_>) -> T,
) -> T {
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface_with(SURFACE, |request| {
            Ok::<_, DockspaceRuntimeError>(match request {
                SurfaceMeasurementRequest::DockBounds { .. }
                | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                    SurfaceMeasurementAnswer::Bounds(
                        LogicalRect::new(0.0, 0.0, 640.0, 480.0).unwrap(),
                    )
                }
                SurfaceMeasurementRequest::PaneSizing { .. } => {
                    SurfaceMeasurementAnswer::PaneSizing(
                        (LogicalSize::new(32.0, 24.0).unwrap()).into(),
                    )
                }
                SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                    SurfaceMeasurementAnswer::TabIntrinsic(180.0)
                }
                SurfaceMeasurementRequest::TabStrip { .. } => SurfaceMeasurementAnswer::TabStrip(
                    TabStripMetrics::new(0.0, 0.0)
                        .unwrap()
                        .with_controls(
                            TabStripControlMetrics::new(4.0)
                                .unwrap()
                                .with_tab_list_menu(
                                    TabStripControlMetric::new(
                                        20.0,
                                        TabStripControlPlacement::ReservedTrailing,
                                    )
                                    .unwrap(),
                                ),
                        )
                        .with_tab_list_menu(
                            TabListMenuMetrics::new(24.0, 8.0, 8.0, 2.0, 140.0, 10.0).unwrap(),
                        ),
                ),
            })
        })
        .unwrap();
    let result = prepare(frame.paint_plan(SURFACE).unwrap().unwrap());
    frame.commit().unwrap();
    result
}

fn apply(session: &mut DockspaceSession, action: PreparedSurfaceAction) {
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_surface_action(action).unwrap();
    frame.commit().unwrap();
}

fn session() -> DockspaceSession {
    let mut session = DockspaceSession::from_layout(
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::central_tabs((1..=40).map(ItemId::new)),
            ),
        )])
        .unwrap(),
        DockPolicy::default(),
    )
    .unwrap();
    let open = inspect(&mut session, |plan| {
        let control = plan
            .tab_strip_controls()
            .find(|control| control.kind() == TabStripControlKind::TabListMenu)
            .unwrap();
        plan.prepare_tab_strip_control_activation(control).unwrap()
    });
    apply(&mut session, open);
    session
}

fn menu_state(session: &mut DockspaceSession) -> (f64, f64) {
    inspect(session, |plan| {
        let menu = plan.tab_list_menus().next().unwrap();
        (menu.scroll_offset(), menu.maximum_scroll_offset())
    })
}

fn scroll(session: &mut DockspaceSession, deltas: &[f64]) {
    let action = inspect(session, |plan| {
        let menu = plan.tab_list_menus().next().unwrap();
        plan.prepare_tab_list_menu_scroll_sequence(menu, deltas.iter().copied())
            .expect("a finite nonempty sequence prepares one action, including round trips")
    });
    apply(session, action);
}

#[test]
fn menu_scroll_sequence_clamps_each_delta_before_reversing() {
    let mut session = session();
    let (initial, maximum) = menu_state(&mut session);
    assert_eq!(initial, 0.0);
    assert!(maximum > 80.0);

    scroll(&mut session, &[20.0, 20.0]);
    assert_eq!(menu_state(&mut session).0, 40.0);

    scroll(&mut session, &[f64::MAX, 20.0, -20.0]);
    assert_eq!(menu_state(&mut session).0, maximum - 20.0);

    scroll(&mut session, &[-f64::MAX, -20.0, 20.0]);
    assert_eq!(menu_state(&mut session).0, 20.0);

    scroll(&mut session, &[20.0, -20.0]);
    assert_eq!(menu_state(&mut session).0, 20.0);
    assert!(session.view().item(FIRST).unwrap().is_selected());
}

#[test]
fn menu_scroll_sequence_rejects_empty_or_nonfinite_input_without_partial_mutation() {
    let mut session = session();
    let before = session.version();
    for deltas in [
        &[][..],
        &[f64::NAN][..],
        &[20.0, f64::NAN][..],
        &[f64::INFINITY][..],
        &[f64::NEG_INFINITY][..],
    ] {
        inspect(&mut session, |plan| {
            let menu = plan.tab_list_menus().next().unwrap();
            assert!(
                plan.prepare_tab_list_menu_scroll_sequence(menu, deltas.iter().copied())
                    .is_none(),
                "invalid input must not prepare a valid prefix: {deltas:?}",
            );
        });
        assert_eq!(session.version(), before);
        assert_eq!(menu_state(&mut session).0, 0.0);
    }
}
