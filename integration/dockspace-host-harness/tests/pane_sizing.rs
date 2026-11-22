//! Complete sizing survives the public measurement and selection seams.

use dockspace::geometry::{LogicalRect, PaneSizing};
use dockspace::model::{
    DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    ItemId, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockspaceRuntimeError, DockspaceSession, SurfaceMeasurementAnswer as Answer,
    SurfaceMeasurementRequest as Request, SurfacePaintProgramOperation as Operation,
    SurfacePaintProgramStep as Step, SurfaceSplitterAdjustment, SurfaceUnavailableReason,
    TabStripMetrics,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);
const THIRD: ItemId = ItemId::new(3);

fn session(content: DockspaceNode) -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), content),
    )])
    .unwrap();
    DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap()
}

fn measure(
    session: &mut DockspaceSession,
    width: f64,
    selected: Option<ItemId>,
    sizing: impl Fn(ItemId) -> PaneSizing,
) -> Vec<(ItemId, LogicalRect)> {
    let mut frame = session.begin_host_frame().unwrap();
    if let Some(selected) = selected {
        frame.select_item_current(selected).unwrap();
    }
    let plan = frame
        .prepare_surface_with(SURFACE, |request| {
            Ok::<_, DockspaceRuntimeError>(answer(request, width, &sizing))
        })
        .unwrap()
        .ready()
        .expect("valid complete measurements produce a paintable surface");
    let mut panes = Vec::new();
    plan.visit_surface_program(|step| {
        if let Step::Operation {
            operation: Operation::Pane(pane),
            ..
        } = step
        {
            panes.push((pane.item(), pane.content_geometry().clip()));
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();
    panes
}

fn answer(request: Request, width: f64, sizing: &impl Fn(ItemId) -> PaneSizing) -> Answer {
    match request {
        Request::DockBounds { .. } | Request::PopupPlaneBounds { .. } => {
            Answer::Bounds(LogicalRect::new(0.0, 0.0, width, 400.0).unwrap())
        }
        Request::PaneSizing { item, .. } => {
            Answer::PaneSizing(item.map_or_else(PaneSizing::default, sizing))
        }
        Request::TabIntrinsic { .. } => Answer::TabIntrinsic(72.0),
        Request::TabStrip { .. } => Answer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap()),
    }
}

#[test]
fn a_fixed_content_viewport_does_not_expand_to_its_host_or_style_floor() {
    let mut session = session(DockspaceNode::tabs([FIRST]));
    for width in [600.0, 900.0, 300.0] {
        let panes = measure(&mut session, width, None, |_| {
            PaneSizing::builder()
                .fixed_width(20.0)
                .fixed_height(30.0)
                .build()
                .unwrap()
        });
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].0, FIRST);
        assert_eq!(panes[0].1.width(), 20.0);
        assert_eq!(panes[0].1.height(), 30.0);
    }
}

#[test]
fn inactive_constraints_are_measured_without_intersecting_the_selected_tab() {
    let mut session = session(DockspaceNode::tabs([FIRST, SECOND]));
    for (selected, expected) in [(FIRST, 180.0), (SECOND, 420.0), (FIRST, 180.0)] {
        let panes = measure(&mut session, 800.0, Some(selected), |item| {
            PaneSizing::builder()
                .fixed_width(if item == FIRST { 180.0 } else { 420.0 })
                .build()
                .unwrap()
        });
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].0, selected);
        assert_eq!(panes[0].1.width(), expected);
    }
}

#[test]
fn updated_maxima_replace_the_old_measurement_without_mutating_membership() {
    let mut session = session(DockspaceNode::tabs([FIRST]));
    let original = session.version();
    for cap in [360.0, 140.0, 280.0] {
        let panes = measure(&mut session, 800.0, None, |_| {
            PaneSizing::builder().max_width(cap).build().unwrap()
        });
        assert_eq!(panes[0].1.width(), cap);
        assert_eq!(session.version(), original);
    }
}

#[test]
fn saturated_siblings_leave_unowned_background_instead_of_filling_the_surface() {
    let mut session = session(
        DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
        )
        .unwrap(),
    );
    let gap = session.presentation_config().splitter_thickness();
    let panes = measure(&mut session, 600.0 + gap, None, |item| {
        PaneSizing::builder()
            .min_width(100.0)
            .max_width(if item == FIRST { 200.0 } else { 250.0 })
            .build()
            .unwrap()
    });
    let first = panes.iter().find(|(item, _)| *item == FIRST).unwrap().1;
    let second = panes.iter().find(|(item, _)| *item == SECOND).unwrap().1;
    assert_eq!(first.x(), 0.0);
    assert_eq!(first.width(), 200.0);
    assert_eq!(second.x(), 200.0 + gap);
    assert_eq!(second.width(), 250.0);
    assert_eq!(600.0 + gap - second.max().x(), 150.0);
    assert!(panes.iter().all(|(_, bounds)| bounds.max().x() < 550.0));
}

#[test]
fn hidden_content_supplies_fresh_sizing_when_shown_again() {
    let mut session = session(DockspaceNode::tabs([FIRST, SECOND]));
    let old = measure(&mut session, 800.0, Some(SECOND), |item| {
        PaneSizing::builder()
            .fixed_width(if item == FIRST { 180.0 } else { 420.0 })
            .build()
            .unwrap()
    });
    assert_eq!(old[0].1.width(), 420.0);
    let mut hide = session.begin_host_frame().unwrap();
    hide.set_item_visibility_current(SECOND, false).unwrap();
    hide.commit().unwrap();
    let hidden = measure(&mut session, 800.0, None, |item| {
        assert_eq!(
            item, FIRST,
            "hidden content must leave the measurement roster"
        );
        PaneSizing::builder().fixed_width(180.0).build().unwrap()
    });
    assert_eq!(hidden[0].0, FIRST);
    let mut show = session.begin_host_frame().unwrap();
    show.set_item_visibility_current(SECOND, true).unwrap();
    show.commit().unwrap();
    let restored = measure(&mut session, 800.0, Some(SECOND), |item| {
        PaneSizing::builder()
            .fixed_width(if item == FIRST { 180.0 } else { 260.0 })
            .build()
            .unwrap()
    });
    assert_eq!(restored[0].0, SECOND);
    assert_eq!(restored[0].1.width(), 260.0);
}

#[test]
fn cross_axis_slack_preserves_a_complete_horizontal_resize_frontier() {
    let left = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
    )
    .unwrap();
    let mut session = session(
        DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [left, DockspaceNode::tabs([THIRD])],
        )
        .unwrap(),
    );
    let width = 600.0 + session.presentation_config().splitter_thickness();
    let step = session.presentation_config().splitter_keyboard_step();
    let sizing = |item| {
        if item == THIRD {
            PaneSizing::default()
        } else {
            PaneSizing::builder().max_height(72.0).build().unwrap()
        }
    };
    let initial = measure(&mut session, width, None, sizing);
    for item in [FIRST, SECOND, THIRD] {
        assert_eq!(
            initial
                .iter()
                .find(|(id, _)| *id == item)
                .unwrap()
                .1
                .width(),
            300.0
        );
    }
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let mut action = None;
    plan.visit_surface_program(|step| {
        if let Step::Operation {
            operation: Operation::Splitter(splitter),
            ..
        } = step
            && splitter.axis() == DockspaceAxis::Horizontal
        {
            assert!(splitter.operable());
            action =
                plan.prepare_splitter_adjustment(splitter, SurfaceSplitterAdjustment::Increment);
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    let action =
        action.expect("vertical background must not revoke a complete horizontal frontier");
    frame.submit_surface_action(action).unwrap();
    frame
        .measure_surface_with(SURFACE, |request| {
            Ok::<_, DockspaceRuntimeError>(answer(request, width, &sizing))
        })
        .unwrap();
    frame.commit().unwrap();
    let resized = measure(&mut session, width, None, sizing);
    for (item, expected) in [
        (FIRST, 300.0 + step),
        (SECOND, 300.0 + step),
        (THIRD, 300.0 - step),
    ] {
        let actual = resized
            .iter()
            .find(|(id, _)| *id == item)
            .unwrap()
            .1
            .width();
        assert!(
            (actual - expected).abs() < 1.0e-9,
            "{item}: {actual} != {expected}"
        );
    }
}
