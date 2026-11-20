use super::*;

fn pane(id: u64) -> DockspaceNode {
    DockspaceNode::pane(ItemId::new(id))
}

fn grid(
    shape: DockspaceGridShape,
    cells: impl IntoIterator<Item = DockspaceNode>,
    rows: impl IntoIterator<Item = f64>,
    columns: impl IntoIterator<Item = f64>,
) -> DockspaceNode {
    DockspaceNode::grid_with_shares(
        shape,
        cells,
        DockspaceGridShares::new(rows, columns).unwrap(),
    )
    .unwrap()
}

fn drag_outer_splitter(
    session: &mut DockspaceSession,
    offered: LogicalRect,
    hints: &[(ItemId, PaneSizing)],
    y: f64,
) -> Vec<(ItemId, LogicalRect)> {
    present_pointer_surface(session);
    let split = bounds(session)
        .into_iter()
        .find(|(item, _)| *item == FIRST)
        .unwrap()
        .1;
    let start = LogicalPoint::new(split.max().x() + 2.0, y).unwrap();
    let end = LogicalPoint::new(start.x() + 20.0, y).unwrap();
    for (event, point) in [
        (
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            start,
        ),
        (SurfacePointerEvent::Moved, end),
    ] {
        let report = submit_host_edges_with(session, &[(event, Some(point))], |frame| {
            measure(frame, offered, hints);
        });
        assert!(session.has_active_gesture());
        assert!(!report.workspace_changed());
    }
    let preview = measured_bounds(session, offered, hints);
    let report = submit_host_edges_with(
        session,
        &[(
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            Some(end),
        )],
        |frame| measure(frame, offered, hints),
    );
    assert!(report.workspace_changed());
    let committed = measured_bounds(session, offered, hints);
    assert_eq!(preview, committed);
    committed
}

fn pane_bounds(panes: &[(ItemId, LogicalRect)], item: u64) -> LogicalRect {
    panes
        .iter()
        .find(|(key, _)| *key == ItemId::new(item))
        .unwrap()
        .1
}

#[test]
fn dragging_an_outer_split_preserves_a_capped_far_grid_track() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 96.0).unwrap();
    let nested = grid(
        DockspaceGridShape::Auto,
        (3..=8).map(pane),
        [1.0, 2.0],
        [1.0, 2.0, 1.0],
    );
    let middle = grid(fixed(1, 2), [pane(2), nested], [1.0], [2.0, 1.0]);
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), middle]).unwrap();
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for minimum in [1.0, 70.0] {
            let hints = std::iter::once((
                SECOND,
                PaneSizing::builder().min_width(minimum).build().unwrap(),
            ))
            .chain((3..=8).map(|id| {
                (
                    ItemId::new(id),
                    PaneSizing::builder().fixed_width(40.0).build().unwrap(),
                )
            }))
            .collect::<Vec<_>>();
            let mut session = session(content.clone(), offered, propagation, &hints);
            let before = bounds(&mut session);
            let far = pane_bounds(&before, 3);
            assert_eq!(pane_bounds(&before, 2).width(), 112.0);
            assert_eq!(far.x(), 320.0);
            let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0);
            assert!((pane_bounds(&committed, 1).width() - 220.0).abs() < 1e-8);
            assert!((pane_bounds(&committed, 2).width() - 92.0).abs() < 1e-8);
            assert!((pane_bounds(&committed, 3).x() - far.x()).abs() < 1e-8);
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn dragging_an_outer_split_does_not_store_the_auto_grids_temporary_columns() {
    use crate::runtime::{DockspaceDocumentBootstrap, DockspaceDocumentId};

    let offered = LogicalRect::new(0.0, 0.0, 424.0, 228.0).unwrap();
    let restored_bounds = LogicalRect::new(0.0, 0.0, 424.0, 170.0).unwrap();
    let hints = [
        (THIRD, 30.0),
        (SIXTH, 30.0),
        (FOURTH, 40.0),
        (SEVENTH, 40.0),
    ]
    .map(|(item, width)| {
        (
            item,
            PaneSizing::builder().fixed_width(width).build().unwrap(),
        )
    });
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, six_pane_auto_grid([1.0, 3.0], [1.0, 2.0, 1.0])),
        )])
        .unwrap();
        let mut bootstrap =
            DockspaceDocumentBootstrap::new(DockspaceDocumentId::from_bytes([5; 16]));
        for id in 1..=7 {
            assert_eq!(
                bootstrap.ensure_item(format!("pane-{id}")).unwrap(),
                ItemId::new(id)
            );
        }
        let mut session = DockspaceSession::from_persistent_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            config(propagation),
            bootstrap,
        )
        .unwrap();
        let before = measured_bounds(&mut session, offered, &hints);
        assert_eq!(pane_bounds(&before, 2).y(), pane_bounds(&before, 4).y());
        let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0);
        assert_ne!(
            pane_bounds(&committed, 2).y(),
            pane_bounds(&committed, 4).y()
        );

        let document = session.save_document_json().unwrap();
        let mut reopened = DockspaceSession::from_document_json_with_presentation_config(
            &document,
            DockPolicy::default(),
            config(propagation),
            |_, _| true,
        )
        .unwrap();
        let panes = measured_bounds(&mut reopened, restored_bounds, &[]);
        for (id, expected) in [(2, 45.5), (3, 91.0), (4, 45.5)] {
            assert!(
                (pane_bounds(&panes, id).width() - expected).abs() < 1e-8,
                "{panes:?}"
            );
        }
    }
}

#[test]
fn an_inherited_grid_transfer_leaves_a_constrained_sibling_grids_shares_intact() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 280.0).unwrap();
    let hints = (3..=8)
        .map(|id| {
            (
                ItemId::new(id),
                PaneSizing::builder().min_width(40.0).build().unwrap(),
            )
        })
        .chain([(
            ItemId::new(12),
            PaneSizing::builder().fixed_width(40.0).build().unwrap(),
        )])
        .collect::<Vec<_>>();
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let nested = grid(
            DockspaceGridShape::Auto,
            (3..=8).map(pane),
            [1.0, 2.0],
            [1.0, 2.0, 1.0],
        );
        let upper = grid(
            fixed(2, 2),
            [pane(2), nested, pane(9), pane(10)],
            [1.0; 2],
            [2.0, 1.0],
        );
        let lower = grid(fixed(2, 2), (11..=14).map(pane), [1.0; 2], [1.0, 3.0]);
        let right = DockspaceNode::equal_split(DockspaceAxis::Vertical, [upper, lower]).unwrap();
        let content =
            DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), right]).unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap()
        .with_hidden_items([ItemId::new(14)])
        .unwrap();
        let mut session = DockspaceSession::from_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            config(propagation),
        )
        .unwrap();
        measured_bounds(&mut session, offered, &hints);
        let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0);
        assert!((pane_bounds(&committed, 2).width() - 48.0).abs() < 1e-8);

        let mut frame = session.begin_host_frame().unwrap();
        frame
            .set_item_visibility_current(ItemId::new(14), true)
            .unwrap();
        measure(&mut frame, offered, &hints);
        assert!(frame.commit().unwrap().workspace_changed());
        let shown = measured_bounds(&mut session, offered, &hints);
        assert!(
            (pane_bounds(&shown, 13).width() - 44.0).abs() < 1e-8,
            "{shown:?}"
        );
        assert!(
            (pane_bounds(&shown, 14).width() - 132.0).abs() < 1e-8,
            "{shown:?}"
        );
    }
}

#[test]
fn a_child_grid_retains_its_shares_when_the_parent_finishes_the_transfer() {
    let offered = LogicalRect::new(0.0, 0.0, 412.0, 160.0).unwrap();
    let hints = [(
        THIRD,
        PaneSizing::builder().fixed_width(50.0).build().unwrap(),
    )];
    let nested = grid(fixed(1, 2), [pane(2), pane(3)], [1.0], [1.0, 3.0]);
    let right = grid(fixed(1, 2), [nested, pane(4)], [1.0], [1.0, 1.0]);
    let left = DockspaceNode::grid(DockspaceGridShape::Auto, [pane(1)]).unwrap();
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [left, right]).unwrap();
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let mut session = session(content.clone(), offered, propagation, &hints);
        let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0);
        for (item, expected) in [(2, 26.0), (3, 50.0), (4, 100.0)] {
            assert!((pane_bounds(&committed, item).width() - expected).abs() < 1e-8);
        }

        let relaxed = measured_bounds(&mut session, offered, &[]);
        for (item, expected) in [(2, 19.0), (3, 57.0), (4, 100.0)] {
            assert!(
                (pane_bounds(&relaxed, item).width() - expected).abs() < 1e-8,
                "{relaxed:?}"
            );
        }
    }
}

#[test]
fn subpixel_surface_translation_keeps_nested_grid_resize_operable() {
    let hints = [
        (
            SIXTH,
            PaneSizing::builder().fixed_height(120.0).build().unwrap(),
        ),
        (
            SEVENTH,
            PaneSizing::builder().min_height(35.0).build().unwrap(),
        ),
    ];
    let inner = grid(
        DockspaceGridShape::Auto,
        (2..=7).map(pane),
        [1.0, 2.0],
        [1.0, 2.0, 1.0],
    );
    let middle = grid(
        fixed(4, 1),
        [pane(8), inner, pane(9), pane(10)],
        [1.0, 2.0, 1.0, 3.0],
        [1.0],
    );
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), middle]).unwrap();
    let resize = |y| {
        let offered = LogicalRect::new(0.0, y, 403.3, 251.4).unwrap();
        let mut session = session(content.clone(), offered, ResizePropagation::Outward, &hints);
        assert!(adjust_outer_splitter(
            &mut session,
            offered,
            &hints,
            &[SurfaceSplitterAdjustment::Increment]
        ));
        measured_bounds(&mut session, offered, &hints)
    };
    let original = resize(0.0);
    let translated = resize(0.3);
    assert_eq!(original.len(), translated.len());
    for ((item, first), (same_item, second)) in original.into_iter().zip(translated) {
        assert_eq!(item, same_item);
        assert!((first.x() - second.x()).abs() < 1e-8);
        assert!((first.y() - (second.y() - 0.3)).abs() < 1e-8);
        assert!((first.width() - second.width()).abs() < 1e-8);
        assert!((first.height() - second.height()).abs() < 1e-8);
    }
}

#[test]
fn outward_endpoint_with_an_empty_grid_track_still_compiles_after_commit() {
    let offered = LogicalRect::new(0.0, 0.0, 804.0, 340.0).unwrap();
    let hints = [
        (
            SIXTH,
            PaneSizing::builder().fixed_height(120.0).build().unwrap(),
        ),
        (
            SEVENTH,
            PaneSizing::builder().min_height(35.0).build().unwrap(),
        ),
    ];
    let nested = grid(
        DockspaceGridShape::Auto,
        (2..=7).map(pane),
        [1.0, 2.0],
        [1.0, 2.0, 1.0],
    );
    let middle = grid(
        fixed(4, 1),
        [pane(8), nested, pane(9), pane(10)],
        [1.0, 2.0, 1.0, 3.0],
        [1.0],
    );
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), middle]).unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap()
    .with_hidden_items([THIRD, ItemId::new(10)])
    .unwrap();
    let mut session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        config(ResizePropagation::Outward),
    )
    .unwrap();
    measured_bounds(&mut session, offered, &hints);

    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let outer = plan
        .splitters()
        .find(|splitter| {
            splitter.axis() == DockspaceAxis::Horizontal && splitter.draw_bounds().x() == 400.0
        })
        .unwrap();
    let action = plan
        .prepare_splitter_adjustment(outer.visual_id(), SurfaceSplitterAdjustment::Maximum)
        .unwrap();
    frame.submit_surface_action(action).unwrap();
    assert!(frame.commit().unwrap().workspace_changed());

    let after = measured_bounds(&mut session, offered, &hints);
    assert!(
        after
            .iter()
            .find(|(item, _)| *item == FIRST)
            .unwrap()
            .1
            .width()
            > 400.0
    );
}

#[test]
fn ancestor_resize_keeps_the_far_grid_track_when_nested_auto_constraints_change() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 152.0).unwrap();
    for (inner_minimum, bottom_minimum) in [(1.0, 1.0), (26.0, 60.0)] {
        let hints = (3..=8)
            .map(|id| {
                (
                    ItemId::new(id),
                    PaneSizing::builder()
                        .min_width(40.0)
                        .min_height(inner_minimum)
                        .build()
                        .unwrap(),
                )
            })
            .chain([(
                ItemId::new(10),
                PaneSizing::builder()
                    .min_height(bottom_minimum)
                    .build()
                    .unwrap(),
            )])
            .collect::<Vec<_>>();
        let nested = grid(
            DockspaceGridShape::Auto,
            (3..=8).map(pane),
            [1.0, 2.0],
            [1.0, 2.0, 1.0],
        );
        let middle = grid(
            fixed(2, 2),
            [pane(2), nested, pane(9), pane(10)],
            [1.0; 2],
            [2.0, 1.0],
        );
        let content =
            DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), middle]).unwrap();
        let mut session = session(content, offered, ResizePropagation::Adjacent, &hints);
        assert!(adjust_outer_splitter(
            &mut session,
            offered,
            &hints,
            &[SurfaceSplitterAdjustment::Increment]
        ));

        let after = bounds(&mut session);
        for (id, expected) in [(1, 220.0), (2, 48.0), (10, 128.0)] {
            let width = after
                .iter()
                .find(|(item, _)| *item == ItemId::new(id))
                .unwrap()
                .1
                .width();
            assert!(
                (width - expected).abs() < 1e-8,
                "pane {id}: {width}, expected {expected}"
            );
        }
    }
}

#[test]
fn deferred_grid_reset_rejects_a_new_active_pane_in_a_previously_empty_cell() {
    let offered = LogicalRect::new(0.0, 0.0, 604.0, 420.0).unwrap();
    let first = DockspaceNode::composed_tabs([DockspaceNode::central_tabs([]), pane(5)]);
    let content = grid(
        fixed(2, 2),
        [first, pane(2), pane(3), pane(4)],
        [1.0, 3.0],
        [1.0, 3.0],
    );
    for step in [
        SurfaceSplitterAdjustment::Equalize,
        SurfaceSplitterAdjustment::Preferred,
    ] {
        let mut session = session(content.clone(), offered, ResizePropagation::Adjacent, &[]);
        let action = inspect_surface(&mut session, |plan| {
            let boundary = plan
                .splitters()
                .find(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
                .unwrap();
            plan.prepare_splitter_adjustment(boundary.visual_id(), step)
                .unwrap()
                .into_deferred()
        });
        let mut frame = session.begin_host_frame().unwrap();
        frame.select_item_current(FIFTH).unwrap();
        assert!(frame.commit().unwrap().workspace_changed());
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_deferred_surface_action(action).unwrap();
        let report = frame.commit().unwrap();
        assert!(
            matches!(report.inputs(), [HostInputOutcome::StaleRejected { .. }]),
            "{report:?}"
        );
        assert!(!report.workspace_changed());
    }
}

#[test]
fn fixed_grid_batch_continues_after_an_equalize_without_feasible_geometry() {
    use SurfaceSplitterAdjustment::{Decrement, Equalize, Increment, Preferred};

    let offered = LogicalRect::new(0.0, 0.0, 804.0, 212.0).unwrap();
    let hints = [(
        ItemId::new(10),
        PaneSizing::builder().fixed_height(10.0).build().unwrap(),
    )];
    let middle = grid(
        fixed(4, 1),
        [pane(8), pane(2), pane(9), pane(10)],
        [1.0, 2.0, 1.0, 3.0],
        [1.0],
    );
    let content = DockspaceNode::equal_split(DockspaceAxis::Vertical, [pane(1), middle]).unwrap();
    for restoration in [Equalize, Preferred] {
        let mut batched = session(
            content.clone(),
            offered,
            ResizePropagation::Adjacent,
            &hints,
        );
        let mut stepwise = session(
            content.clone(),
            offered,
            ResizePropagation::Adjacent,
            &hints,
        );
        let resize = |session: &mut DockspaceSession, steps: &[SurfaceSplitterAdjustment]| {
            let mut frame = session.begin_host_frame().unwrap();
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let boundary = plan
                .splitters()
                .min_by(|a, b| a.draw_bounds().y().total_cmp(&b.draw_bounds().y()))
                .unwrap();
            if let Some(action) = plan
                .prepare_splitter_adjustment_sequence(boundary.visual_id(), steps.iter().copied())
            {
                frame.submit_surface_action(action).unwrap();
            }
            measure(&mut frame, offered, &hints);
            frame.commit().unwrap();
        };
        let steps = [Decrement, Increment, restoration, Increment];
        resize(&mut batched, &steps);
        for step in steps {
            resize(&mut stepwise, &[step]);
        }
        let expected = bounds(&mut stepwise);
        assert_eq!(
            expected
                .iter()
                .find(|(item, _)| *item == FIRST)
                .unwrap()
                .1
                .height(),
            106.0
        );
        assert_eq!(bounds(&mut batched), expected);
    }
}
