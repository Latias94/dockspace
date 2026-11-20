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
