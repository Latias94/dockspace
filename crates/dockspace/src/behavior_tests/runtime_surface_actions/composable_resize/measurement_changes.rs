//! Fresh measurements must not reuse a direct Auto boundary after its cell map changes.

use super::*;
use crate::runtime::DockspaceManipulationCancelReason;

#[derive(Clone, Copy, Debug)]
enum ResizeDriver {
    Column,
    Junction,
}

impl ResizeDriver {
    fn delta(self, column: f64) -> (f64, f64) {
        match self {
            Self::Column => (column, 0.0),
            Self::Junction => (column, 10.0),
        }
    }
}

fn five_cells(shape: DockspaceGridShape, offered: LogicalRect) -> DockspaceSession {
    session(
        DockspaceNode::grid(
            shape,
            [FIRST, SECOND, THIRD, FOURTH, FIFTH].map(DockspaceNode::pane),
        )
        .expect("five bare cells fit the declared grid"),
        offered,
        ResizePropagation::Adjacent,
        &[],
    )
}

fn shares(session: &DockspaceSession) -> Option<DockspaceGridShares> {
    session
        .view()
        .root(ROOT)
        .and_then(|root| root.content())
        .and_then(|node| node.grid())
        .expect("the original grid remains the root content")
        .shares()
        .cloned()
}

fn assert_shape(session: &mut DockspaceSession, expected: (usize, usize)) {
    let panes = bounds(session);
    assert_eq!(
        panes.iter().map(|(item, _)| *item).collect::<Vec<_>>(),
        [FIRST, SECOND, THIRD, FOURTH, FIFTH]
    );
    let mut rows = panes.iter().map(|(_, rect)| rect.y()).collect::<Vec<_>>();
    let mut columns = panes.iter().map(|(_, rect)| rect.x()).collect::<Vec<_>>();
    rows.sort_by(f64::total_cmp);
    columns.sort_by(f64::total_cmp);
    rows.dedup();
    columns.dedup();
    assert_eq!((rows.len(), columns.len()), expected, "{panes:?}");
}

fn begin_resize(
    session: &mut DockspaceSession,
    offered: LogicalRect,
    driver: ResizeDriver,
) -> LogicalPoint {
    present_pointer_surface(session);
    let press = inspect_surface(session, |plan| match driver {
        ResizeDriver::Column => {
            let splitter = plan
                .splitters()
                .find(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
                .expect("the grid has an operable column boundary");
            let boundary = plan
                .receiver_for_splitter(splitter)
                .expect("the direct column boundary has an exact receiver");
            let first = plan.panes().find(|pane| pane.item() == FIRST).unwrap();
            // Stay inside the first row, away from every row/column junction.
            LogicalPoint::new(
                boundary.center().x(),
                first.bounds().y() + first.bounds().height() * 0.5,
            )
            .unwrap()
        }
        ResizeDriver::Junction => {
            let junction = plan
                .splitter_junctions()
                .next()
                .expect("both shared grid axes meet at a junction");
            assert!(plan.splitter_junction_operable(junction));
            plan.receiver_for_splitter_junction(junction)
                .expect("the shared junction has one atomic receiver")
                .center()
        }
    });
    submit_host_edges_with(
        session,
        &[(
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            Some(press),
        )],
        |frame| measure(frame, offered, &[]),
    );
    assert!(session.has_active_gesture());
    press
}

fn move_resize(
    session: &mut DockspaceSession,
    press: LogicalPoint,
    delta: (f64, f64),
    offered: LogicalRect,
) -> LogicalPoint {
    let moved = LogicalPoint::new(press.x() + delta.0, press.y() + delta.1).unwrap();
    submit_host_edges_with(
        session,
        &[(SurfacePointerEvent::Moved, Some(moved))],
        |frame| measure(frame, offered, &[]),
    );
    assert!(session.has_active_gesture());
    moved
}

fn replace_measurement(session: &mut DockspaceSession, offered: LogicalRect) -> HostFrameReport {
    // Explicit measurement is required: the pointer helper preserves an existing Ready plan.
    // No platform coordinate facts or workspace references change in this frame.
    let mut frame = session
        .begin_host_frame()
        .expect("the fresh bounds measurement frame begins");
    measure(&mut frame, offered, &[]);
    frame
        .commit()
        .expect("the fresh bounds measurement commits")
}

fn assert_new_map_revokes_resize(driver: ResizeDriver, update_before_measurement: bool) {
    let wide = LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap();
    let tall = LogicalRect::new(0.0, 0.0, 204.0, 404.0).unwrap();
    let mut session = five_cells(DockspaceGridShape::Auto, wide);
    assert_shape(&mut session, (2, 3));
    let original = session.workspace().clone();
    let version = session.version();
    assert_eq!(shares(&session), None);

    let press = begin_resize(&mut session, wide, driver);
    let initial = measured_bounds(&mut session, wide, &[]);
    let moved = if update_before_measurement {
        let moved = move_resize(&mut session, press, driver.delta(16.0), wide);
        let preview = measured_bounds(&mut session, wide, &[]);
        let first = |panes: &[(ItemId, LogicalRect)]| {
            panes.iter().find(|(item, _)| *item == FIRST).unwrap().1
        };
        assert_ne!(first(&preview).width(), first(&initial).width());
        if matches!(driver, ResizeDriver::Junction) {
            assert_ne!(first(&preview).height(), first(&initial).height());
        }
        moved
    } else {
        press
    };
    assert_eq!(session.workspace(), &original);
    assert_eq!(session.version(), version);

    let measured = replace_measurement(&mut session, tall);
    assert!(!measured.workspace_changed());
    assert_eq!(session.workspace(), &original);
    assert_eq!(session.version(), version);
    assert_eq!(shares(&session), None);
    assert!(
        !session.has_active_gesture(),
        "{driver:?}, updated={update_before_measurement}: a direct driver cannot retain its old map"
    );
    let [cancelled] = measured.manipulation_feedback() else {
        panic!("the entire {driver:?} gesture must cancel exactly once: {measured:?}");
    };
    assert_eq!(
        cancelled.status(),
        DockspaceManipulationStatus::Cancelled(DockspaceManipulationCancelReason::SceneUnavailable)
    );
    let mut clean = five_cells(DockspaceGridShape::Auto, tall);
    let clean_geometry = bounds(&mut clean);
    assert_eq!(
        bounds(&mut session),
        clean_geometry,
        "revocation removes every old resize override from the newly measured plan"
    );
    assert_shape(&mut session, (3, 2));

    let released = submit_host_edges_with(
        &mut session,
        &[(
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            Some(moved),
        )],
        |frame| measure(frame, tall, &[]),
    );
    assert!(!released.workspace_changed());
    assert!(
        released.manipulation_feedback().iter().all(|feedback| {
            !matches!(feedback.status(), DockspaceManipulationStatus::Cancelled(_))
        }),
        "the late release must not publish a second cancellation"
    );
    assert!(!session.has_active_gesture());
    assert_eq!(session.workspace(), &original);
    assert_eq!(session.version(), version);
    assert_eq!(shares(&session), None);
    assert_eq!(
        bounds(&mut session),
        clean_geometry,
        "a late release cannot revive the old map or its last accepted sample"
    );
    assert_shape(&mut session, (3, 2));

    let fresh = begin_resize(&mut session, tall, driver);
    let moved = move_resize(&mut session, fresh, driver.delta(12.0), tall);
    let released = submit_host_edges_with(
        &mut session,
        &[(
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            Some(moved),
        )],
        |frame| measure(frame, tall, &[]),
    );
    assert!(released.workspace_changed(), "{released:?}");
    assert!(!session.has_active_gesture());
    assert_eq!(session.version().epoch(), version.epoch());
    assert_eq!(
        session.version().revision().get(),
        version.revision().get() + 1
    );
    let shares = shares(&session).expect("the new gesture commits shares for its current map");
    assert_eq!((shares.rows().len(), shares.columns().len()), (3, 2));
    assert_eq!(
        session.workspace().item_multiset(),
        original.item_multiset()
    );
    assert_shape(&mut session, (3, 2));
}

#[test]
fn measuring_a_new_auto_cell_map_revokes_the_direct_resize_and_late_release_cannot_commit() {
    for driver in [ResizeDriver::Column, ResizeDriver::Junction] {
        for update_before_measurement in [false, true] {
            assert_new_map_revokes_resize(driver, update_before_measurement);
        }
    }
}

#[test]
fn abandoning_a_new_auto_map_measurement_preserves_the_committed_resize() {
    let wide = LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap();
    let tall = LogicalRect::new(0.0, 0.0, 204.0, 404.0).unwrap();
    let mut clean = five_cells(DockspaceGridShape::Auto, tall);
    let clean_geometry = bounds(&mut clean);
    for update_before_measurement in [false, true] {
        let mut session = five_cells(DockspaceGridShape::Auto, wide);
        let original = session.workspace().clone();
        let version = session.version();
        let press = begin_resize(&mut session, wide, ResizeDriver::Column);
        if update_before_measurement {
            move_resize(&mut session, press, (16.0, 0.0), wide);
        }
        let committed_geometry = measured_bounds(&mut session, wide, &[]);

        let mut abandoned = session
            .begin_host_frame()
            .expect("the abandoned measurement frame begins");
        measure(&mut abandoned, tall, &[]);
        let candidate_geometry = abandoned
            .paint_plan(SURFACE)
            .unwrap()
            .expect("the invalidated resize exposes a clean candidate")
            .panes()
            .map(|pane| (pane.item(), pane.bounds()))
            .collect::<Vec<_>>();
        assert_eq!(candidate_geometry, clean_geometry);
        assert_eq!(abandoned.version(), version);
        drop(abandoned);

        assert!(session.has_active_gesture());
        assert_eq!(session.workspace(), &original);
        assert_eq!(session.version(), version);
        assert_eq!(shares(&session), None);
        assert_eq!(
            bounds(&mut session),
            committed_geometry,
            "discarding a measurement must preserve the committed bounds and resize sample"
        );
        let retained = replace_measurement(&mut session, wide);
        assert!(retained.manipulation_feedback().is_empty());
        assert!(session.has_active_gesture());
        assert_eq!(bounds(&mut session), committed_geometry);

        let moved = move_resize(&mut session, press, (24.0, 0.0), wide);
        let released = submit_host_edges_with(
            &mut session,
            &[(
                SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
                Some(moved),
            )],
            |frame| measure(frame, wide, &[]),
        );
        assert!(released.workspace_changed(), "{released:?}");
        let [delivered] = released.manipulation_feedback() else {
            panic!("the retained resize delivers exactly once: {released:?}");
        };
        assert_eq!(
            delivered.status(),
            DockspaceManipulationStatus::Delivered(DockspaceManipulationDelivery::SplitterResize)
        );
        assert!(!session.has_active_gesture());
        assert_eq!(session.version().epoch(), version.epoch());
        assert_eq!(
            session.version().revision().get(),
            version.revision().get() + 1
        );
        let shares = shares(&session).expect("the retained gesture commits its original map");
        assert_eq!((shares.rows().len(), shares.columns().len()), (2, 3));
        assert_eq!(
            session.workspace().item_multiset(),
            original.item_multiset()
        );
        assert_shape(&mut session, (2, 3));
    }
}

fn assert_measurement_preserves_resize(
    shape: DockspaceGridShape,
    wide: LogicalRect,
    changed: LogicalRect,
) {
    let mut session = five_cells(shape, wide);
    assert_shape(&mut session, (2, 3));
    let original = session.workspace().clone();
    let version = session.version();
    let press = begin_resize(&mut session, wide, ResizeDriver::Column);
    move_resize(&mut session, press, (16.0, 0.0), wide);

    let measured = replace_measurement(&mut session, changed);
    assert!(!measured.workspace_changed());
    assert_eq!(session.workspace(), &original);
    assert_eq!(session.version(), version);
    assert_eq!(shares(&session), None);
    assert!(
        session.has_active_gesture(),
        "fresh bounds preserving the direct cell map must permit continuation"
    );
    assert_shape(&mut session, (2, 3));

    let moved = move_resize(&mut session, press, (24.0, 0.0), changed);
    let released = submit_host_edges_with(
        &mut session,
        &[(
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            Some(moved),
        )],
        |frame| measure(frame, changed, &[]),
    );
    assert!(released.workspace_changed(), "{released:?}");
    assert!(!session.has_active_gesture());
    assert_eq!(session.version().epoch(), version.epoch());
    assert_eq!(
        session.version().revision().get(),
        version.revision().get() + 1
    );
    let shares = shares(&session).expect("the still-current direct map accepts track shares");
    assert_eq!((shares.rows().len(), shares.columns().len()), (2, 3));
    assert_eq!(
        session.workspace().item_multiset(),
        original.item_multiset()
    );
    assert_shape(&mut session, (2, 3));
}

#[test]
fn measuring_new_bounds_with_the_same_auto_map_preserves_the_direct_resize() {
    assert_measurement_preserves_resize(
        DockspaceGridShape::Auto,
        LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
        LogicalRect::new(0.0, 0.0, 424.0, 204.0).unwrap(),
    );
}

#[test]
fn measuring_a_new_aspect_preserves_a_fixed_grid_direct_resize() {
    assert_measurement_preserves_resize(
        fixed(2, 3),
        LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap(),
        LogicalRect::new(0.0, 0.0, 204.0, 404.0).unwrap(),
    );
}
