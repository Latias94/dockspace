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
    delta: f64,
) -> Vec<(ItemId, LogicalRect)> {
    present_pointer_surface(session);
    let split = bounds(session)
        .into_iter()
        .find(|(item, _)| *item == FIRST)
        .unwrap()
        .1;
    let start = LogicalPoint::new(split.max().x() + 2.0, y).unwrap();
    let end = LogicalPoint::new(start.x() + delta, y).unwrap();
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
fn capped_nested_split_keeps_both_resize_directions_at_a_frame_contact() {
    use SurfaceSplitterAdjustment::{Decrement, Increment};

    let offered = LogicalRect::new(0.0, 0.0, 204.0, 228.0).unwrap();
    let top = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(2), pane(3)]).unwrap();
    let left = DockspaceNode::equal_split(DockspaceAxis::Vertical, [top, pane(1)]).unwrap();
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [left, pane(4)]).unwrap();
    let config = DockPresentationConfig::builder()
        .minimum_pane_size(1.0, 1.0)
        .splitter_thickness(4.0)
        .splitter_keyboard_step(5.0)
        .build()
        .unwrap();
    for minimum in [45.0, 46.0, 47.0] {
        let hints = [
            (
                SECOND,
                PaneSizing::builder().max_width(90.0).build().unwrap(),
            ),
            (
                THIRD,
                PaneSizing::builder().max_width(60.0).build().unwrap(),
            ),
            (
                FOURTH,
                PaneSizing::builder().min_width(minimum).build().unwrap(),
            ),
        ];
        for (delta, adjustment) in [(-5.0, Decrement), (5.0, Increment)] {
            let mut pointer = session_with_config(content.clone(), offered, config.clone(), &hints);
            let dragged = drag_outer_splitter(&mut pointer, offered, &hints, 170.0, delta);
            let mut semantic =
                session_with_config(content.clone(), offered, config.clone(), &hints);
            let mut frame = semantic.begin_host_frame().unwrap();
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let outer = plan
                .splitters()
                .find(|splitter| {
                    splitter
                        .draw_bounds()
                        .contains(LogicalPoint::new(102.0, 170.0).unwrap())
                })
                .unwrap();
            let range = plan.splitter_resize_range(outer).unwrap();
            assert_eq!(
                (range.minimum(), range.value(), range.maximum()),
                (0.265, 0.5, 0.56)
            );
            let action = plan.prepare_splitter_adjustment(outer, adjustment).unwrap();
            frame.submit_surface_action(action).unwrap();
            measure(&mut frame, offered, &hints);
            assert!(frame.commit().unwrap().workspace_changed());
            assert_eq!(bounds(&mut semantic), dragged);
            for (item, expected) in [100.0 + delta, 48.0, 48.0 + delta, 100.0 - delta]
                .into_iter()
                .enumerate()
            {
                assert_eq!(pane_bounds(&dragged, item as u64 + 1).width(), expected);
            }
        }
    }
}

#[cfg(feature = "serde")]
fn persistent_session(
    content: DockspaceNode,
    item_count: u64,
    propagation: ResizePropagation,
) -> DockspaceSession {
    use crate::runtime::{DockspaceDocumentBootstrap, DockspaceDocumentId};

    let mut bootstrap = DockspaceDocumentBootstrap::new(DockspaceDocumentId::from_bytes([5; 16]));
    for id in 1..=item_count {
        assert_eq!(
            bootstrap.ensure_item(format!("pane-{id}")).unwrap(),
            ItemId::new(id)
        );
    }
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap();
    DockspaceSession::from_persistent_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        config(propagation),
        bootstrap,
    )
    .unwrap()
}

#[cfg(feature = "serde")]
fn reopen(session: &mut DockspaceSession, propagation: ResizePropagation) -> DockspaceSession {
    DockspaceSession::from_document_json_with_presentation_config(
        &session.save_document_json().unwrap(),
        DockPolicy::default(),
        config(propagation),
        |_, _| true,
    )
    .unwrap()
}

#[cfg(feature = "serde")]
#[test]
fn remapped_grid_keeps_a_child_divider_whose_slot_still_follows_the_drag() {
    let offered = LogicalRect::new(0.0, 0.0, 424.0, 228.0).unwrap();
    let return_to_three_columns = LogicalRect::new(0.0, 0.0, 424.0, 170.0).unwrap();
    let content = |outer, child| {
        let nested = grid(fixed(1, 2), [pane(2), pane(3)], [1.0], child);
        let sibling = grid(fixed(1, 2), [pane(6), pane(9)], [1.0], [2.0, 1.0]);
        let parent = grid(
            DockspaceGridShape::Auto,
            [nested, pane(4), pane(5), sibling, pane(7), pane(8)],
            [1.0, 1.0],
            [113.0, 44.5, 44.5],
        );
        let [left, right] = outer;
        DockspaceNode::split(
            DockspaceAxis::Horizontal,
            [(pane(1), left), (parent, right)],
        )
        .unwrap()
    };

    for mode in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for pointer in [true, false] {
            let mut session = persistent_session(content([1.0, 1.0], [1.0, 1.0]), 9, mode);
            let original = measured_bounds(&mut session, offered, &[]);
            assert_eq!(pane_bounds(&original, 2).x(), pane_bounds(&original, 6).x());
            assert_eq!(pane_bounds(&original, 3).x(), 272.5);
            let resized = if pointer {
                drag_outer_splitter(&mut session, offered, &[], 44.0, 20.0)
            } else {
                assert!(adjust_outer_splitter(
                    &mut session,
                    offered,
                    &[],
                    &[SurfaceSplitterAdjustment::Increment],
                ));
                measured_bounds(&mut session, offered, &[])
            };

            let mut reopened = reopen(&mut session, mode);
            let mut expected =
                super::session(content([230.0, 190.0], [34.5, 54.5]), offered, mode, &[]);
            for (actual, expected) in [
                (resized, bounds(&mut expected)),
                (
                    measured_bounds(&mut reopened, offered, &[]),
                    bounds(&mut expected),
                ),
                (
                    measured_bounds(&mut reopened, return_to_three_columns, &[]),
                    measured_bounds(&mut expected, return_to_three_columns, &[]),
                ),
            ] {
                for item in 1..=9 {
                    let actual = pane_bounds(&actual, item);
                    let expected = pane_bounds(&expected, item);
                    assert!(
                        (actual.x() - expected.x()).abs() < 1e-8
                            && (actual.y() - expected.y()).abs() < 1e-8
                            && (actual.width() - expected.width()).abs() < 1e-8
                            && (actual.height() - expected.height()).abs() < 1e-8,
                        "{mode:?}, pane {item}: expected {expected:?}, got {actual:?}"
                    );
                }
            }
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn resizing_a_grid_column_keeps_the_split_in_the_stationary_column() {
    let offered = LogicalRect::new(0.0, 0.0, 504.0, 78.0).unwrap();
    let auto = DockspaceNode::grid(DockspaceGridShape::Auto, (4..=9).map(pane)).unwrap();
    let far = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(3), auto]).unwrap();
    let content = grid(fixed(1, 3), [pane(1), pane(2), far], [1.0], [2.0, 2.0, 1.0]);
    let hints = [(3, 20.0), (4, 40.0), (7, 40.0)].map(|(id, width)| {
        (
            ItemId::new(id),
            PaneSizing::builder().min_width(width).build().unwrap(),
        )
    });
    for mode in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for pointer in [true, false] {
            let mut session = persistent_session(content.clone(), 9, mode);
            let original = measured_bounds(&mut session, offered, &hints);
            assert_eq!(pane_bounds(&original, 4).x(), 420.0);
            assert_eq!(pane_bounds(&original, 6).y(), pane_bounds(&original, 7).y());
            let resized = if pointer {
                drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0)
            } else {
                assert!(adjust_outer_splitter(
                    &mut session,
                    offered,
                    &hints,
                    &[SurfaceSplitterAdjustment::Increment],
                ));
                measured_bounds(&mut session, offered, &hints)
            };
            let mut reopened = reopen(&mut session, mode);
            for panes in [resized, measured_bounds(&mut reopened, offered, &hints)] {
                assert_ne!(pane_bounds(&panes, 6).y(), pane_bounds(&panes, 7).y());
                assert!((pane_bounds(&panes, 1).width() - 214.0).abs() < 1e-8);
                assert!((pane_bounds(&panes, 2).width() - 174.0).abs() < 1e-8);
                assert!((pane_bounds(&panes, 3).x() - 396.0).abs() < 1e-8);
                assert!(
                    (pane_bounds(&panes, 4).x() - 420.0).abs() < 1e-8,
                    "{mode:?}"
                );
            }
        }
    }
}

#[test]
fn junction_batch_stops_when_its_orthogonal_grid_boundary_changes_cells() {
    use SurfaceSplitterAdjustment::Increment;

    let inner = grid(
        DockspaceGridShape::Auto,
        (2..=7).map(pane),
        [1.0, 3.0],
        [1.0, 2.0, 1.0],
    );
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), inner]).unwrap();
    for mode in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for width in [424.0, 524.0] {
            let offered = LogicalRect::new(0.0, 0.0, width, 228.0).unwrap();
            let first_tee = |session: &mut DockspaceSession| {
                inspect_surface(session, |plan| {
                    plan.splitter_junctions()
                        .filter(|junction| junction.arm_count() == 3)
                        .min_by(|a, b| a.hit_bounds().y().total_cmp(&b.hit_bounds().y()))
                        .unwrap()
                        .visual_id()
                })
            };
            let mut stepped = session(content.clone(), offered, mode, &[]);
            let id = first_tee(&mut stepped);
            let adjust = |session: &mut DockspaceSession, steps: &[SurfaceSplitterAdjustment]| {
                let mut frame = session.begin_host_frame().unwrap();
                let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                let junction = plan
                    .splitter_junctions()
                    .find(|junction| junction.visual_id() == id)
                    .unwrap();
                let action = plan
                    .prepare_splitter_junction_adjustment_sequence(
                        junction,
                        DockspaceAxis::Horizontal,
                        steps.iter().copied(),
                    )
                    .unwrap();
                frame.submit_surface_action(action).unwrap();
                measure(&mut frame, offered, &[]);
                assert!(frame.commit().unwrap().workspace_changed());
            };
            let original = bounds(&mut stepped);
            assert_eq!(pane_bounds(&original, 2).y(), pane_bounds(&original, 4).y());
            adjust(&mut stepped, &[Increment]);
            let first = bounds(&mut stepped);
            assert_eq!(first_tee(&mut stepped), id);
            let remapped = pane_bounds(&first, 2).y() != pane_bounds(&first, 4).y();
            assert_eq!(remapped, width == 424.0);
            if !remapped {
                adjust(&mut stepped, &[Increment]);
                adjust(&mut stepped, &[Increment]);
            }
            let mut batch = session(content.clone(), offered, mode, &[]);
            adjust(&mut batch, &[Increment; 3]);
            assert_eq!(bounds(&mut batch), bounds(&mut stepped), "{mode:?} {width}");
        }
    }
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
            let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0);
            assert!((pane_bounds(&committed, 1).width() - 220.0).abs() < 1e-8);
            assert!((pane_bounds(&committed, 2).width() - 92.0).abs() < 1e-8);
            assert!((pane_bounds(&committed, 3).x() - far.x()).abs() < 1e-8);
        }
    }
}

#[test]
fn a_parent_grid_settles_before_measuring_its_nested_grid_transfer() {
    let offered = LogicalRect::new(0.0, 0.0, 812.0, 96.0).unwrap();
    let auto = grid(
        DockspaceGridShape::Auto,
        (3..=8).map(pane),
        [1.0, 2.0],
        [1.0, 2.0, 1.0],
    );
    let child = grid(fixed(1, 2), [pane(2), auto], [1.0], [2.0, 1.0]);
    let parent = grid(fixed(1, 2), [child, pane(9)], [1.0], [1.0, 1.0]);
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(1), parent]).unwrap();
    let hints = (3..=8)
        .map(|id| {
            (
                ItemId::new(id),
                PaneSizing::builder().fixed_width(40.0).build().unwrap(),
            )
        })
        .collect::<Vec<_>>();

    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let mut session = session(content.clone(), offered, propagation, &hints);
        let before = bounds(&mut session);
        assert_eq!(pane_bounds(&before, 1).width(), 404.0);
        assert_eq!(pane_bounds(&before, 3).x(), 524.0);
        assert_eq!(pane_bounds(&before, 9).x(), 612.0);

        let resized = drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0);
        assert_eq!(pane_bounds(&resized, 1).width(), 424.0);
        assert_eq!(pane_bounds(&resized, 2).width(), 92.0);
        assert_eq!(pane_bounds(&resized, 3).x(), 524.0);
        assert_eq!(pane_bounds(&resized, 9).x(), 612.0);
    }
}

#[test]
fn moving_a_divider_accepts_rounding_in_small_stationary_tracks() {
    let offered = LogicalRect::new(0.0, 0.0, 1024.0, 128.0).unwrap();
    for auto_cells in [1, 2] {
        let left =
            DockspaceNode::grid(DockspaceGridShape::Auto, (1..=auto_cells).map(pane)).unwrap();
        let right =
            DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(3), pane(4)]).unwrap();
        let content = grid(
            fixed(1, 3),
            [left, pane(5), right],
            [1.0],
            [49.0, 49.0, 1.0],
        );
        let mut session = session(content, offered, ResizePropagation::Adjacent, &[]);
        let original = bounds(&mut session);
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let divider = plan
            .splitters()
            .find(|splitter| {
                splitter.axis() == DockspaceAxis::Horizontal
                    && (splitter.draw_bounds().max().x() - pane_bounds(&original, 5).x()).abs()
                        < 1e-9
            })
            .unwrap();
        let action = plan
            .prepare_splitter_adjustment(divider, SurfaceSplitterAdjustment::Increment)
            .unwrap();
        frame.submit_surface_action(action).unwrap();
        measure(&mut frame, offered, &[]);
        assert!(frame.commit().unwrap().workspace_changed());
        let resized = measured_bounds(&mut session, offered, &[]);
        for item in [3, 4] {
            let before = pane_bounds(&original, item);
            let after = pane_bounds(&resized, item);
            assert!((before.x() - after.x()).abs() < 1e-9);
            assert!((before.width() - after.width()).abs() < 1e-9);
        }
        // The left grid can have two cells, so check the middle pane's boundary.
        assert!((pane_bounds(&resized, 5).x() - pane_bounds(&original, 5).x() - 20.0).abs() < 1e-9);
    }
}

#[test]
fn capped_auto_grid_keeps_its_far_edge_when_a_parent_split_transfers_space() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 96.0).unwrap();
    let hints = (3..=8)
        .map(|id| {
            (
                ItemId::new(id),
                PaneSizing::builder().fixed_width(16.0).build().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for weights in [[1.0, 1.0], [5.0, 3.0]] {
            let auto = grid(
                DockspaceGridShape::Auto,
                (3..=8).map(pane),
                [1.0, 1.0],
                [1.0, 1.0, 1.0],
            );
            let near = DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [(pane(2), weights[0]), (auto, weights[1])],
            )
            .unwrap();
            let content = grid(fixed(1, 2), [pane(1), near], [1.0], [1.0, 1.0]);
            let mut session = session(content, offered, propagation, &hints);
            let initial = bounds(&mut session);
            assert!(
                adjust_outer_splitter(
                    &mut session,
                    offered,
                    &hints,
                    &[SurfaceSplitterAdjustment::Increment]
                ),
                "{propagation:?}, {weights:?}"
            );
            let resized = measured_bounds(&mut session, offered, &hints);
            assert!((pane_bounds(&resized, 1).width() - 220.0).abs() < 1e-9);
            assert!((pane_bounds(&resized, 2).width() - 120.0).abs() < 1e-9);
            for item in 3..=8 {
                let before = pane_bounds(&initial, item);
                let after = pane_bounds(&resized, item);
                assert!((before.x() - after.x()).abs() < 1e-9);
                assert_eq!(before.y(), after.y());
            }
        }
    }
}

#[test]
fn independent_grid_transfers_share_projection_passes() {
    use crate::presentation::layout::auto_reflow_projection_count;

    let sample = |count: u64| {
        let grids = (0..count).map(|index| {
            let first = 2 + 3 * index;
            let inner = grid(
                fixed(1, 2),
                [pane(first), pane(first + 1)],
                [1.0],
                [1.0, 3.0],
            );
            grid(fixed(1, 2), [inner, pane(first + 2)], [1.0], [1.0, 1.0])
        });
        let left =
            DockspaceNode::grid(DockspaceGridShape::Auto, [pane(1), pane(2 + 3 * count)]).unwrap();
        let right = DockspaceNode::equal_split(DockspaceAxis::Vertical, grids).unwrap();
        let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [left, right]).unwrap();
        let fixed_width = PaneSizing::builder().fixed_width(50.0).build().unwrap();
        let hints = (0..count)
            .map(|index| (ItemId::new(3 + 3 * index), fixed_width))
            .collect::<Vec<_>>();
        let offered = LogicalRect::new(0.0, 0.0, 412.0, 28.0 + count as f64 * 64.0).unwrap();
        let mut session = session(content, offered, ResizePropagation::Adjacent, &hints);

        inspect_surface(&mut session, |plan| {
            let divider = plan
                .splitters()
                .filter(|divider| divider.axis() == DockspaceAxis::Horizontal)
                .min_by(|a, b| a.draw_bounds().x().total_cmp(&b.draw_bounds().x()))
                .expect("outer split is the first horizontal divider");
            let before = auto_reflow_projection_count();
            assert!(
                plan.prepare_splitter_adjustment(divider, SurfaceSplitterAdjustment::Increment)
                    .is_some()
            );
            auto_reflow_projection_count() - before
        })
    };
    let narrow = sample(4);
    let wide = sample(32);
    assert_eq!(narrow, wide);
    assert!(
        wide <= 3,
        "{wide} full-root projections for two nested tiers"
    );
}

#[test]
fn one_cell_auto_grid_does_not_reproject_ancestor_transfers() {
    use crate::presentation::layout::auto_reflow_projection_count;

    for depth in [4, 8, 16] {
        let mut branch = pane(2);
        let mut span = 96.0;
        for item in 3..3 + depth {
            let split = DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [(branch, span), (pane(item), 32.0)],
            )
            .unwrap();
            // Keep the nested split from being flattened into its parent.
            branch = DockspaceNode::grid(fixed(1, 1), [split]).unwrap();
            span += 36.0;
        }
        let auto = DockspaceNode::grid(DockspaceGridShape::Auto, [pane(1)]).unwrap();
        let content =
            DockspaceNode::split(DockspaceAxis::Horizontal, [(auto, 120.0), (branch, span)])
                .unwrap();
        let offered = LogicalRect::new(0.0, 0.0, span + 124.0, 130.0).unwrap();
        let mut session = session(content, offered, ResizePropagation::Adjacent, &[]);
        let original = bounds(&mut session);
        assert!((pane_bounds(&original, 1).width() - 120.0).abs() < 1e-9);
        let before = auto_reflow_projection_count();
        assert!(adjust_outer_splitter(
            &mut session,
            offered,
            &[],
            &[SurfaceSplitterAdjustment::Increment]
        ));
        assert_eq!(auto_reflow_projection_count(), before, "depth {depth}");
        let resized = measured_bounds(&mut session, offered, &[]);
        assert!((pane_bounds(&resized, 1).width() - 140.0).abs() < 1e-9);
        for item in 3..3 + depth {
            assert!(
                (pane_bounds(&resized, item).x() - pane_bounds(&original, item).x()).abs() < 1e-9
            );
        }
    }
}

#[test]
fn adjacent_boundary_grows_an_uncapped_band_past_its_capped_peer() {
    use crate::presentation::layout::auto_reflow_projection_count;

    let offered = LogicalRect::new(0.0, 0.0, 308.0, 228.0).unwrap();
    let hints = [(
        FIRST,
        PaneSizing::builder().max_width(100.0).build().unwrap(),
    )];
    let left = DockspaceNode::equal_split(DockspaceAxis::Vertical, [pane(1), pane(2)]).unwrap();
    let right = DockspaceNode::grid(DockspaceGridShape::Auto, [pane(4)]).unwrap();
    let content =
        DockspaceNode::equal_split(DockspaceAxis::Horizontal, [left, pane(3), right]).unwrap();
    for pointer in [false, true] {
        let mut session = session(
            content.clone(),
            offered,
            ResizePropagation::Adjacent,
            &hints,
        );
        let count = auto_reflow_projection_count();
        inspect_surface(&mut session, |plan| {
            let divider = plan
                .splitters()
                .find(|divider| divider.draw_bounds().x() == 100.0)
                .unwrap();
            let range = plan.splitter_resize_range(divider).unwrap();
            assert!((range.maximum() * 300.0 - 199.0).abs() < 1e-8);
        });
        let resized = if pointer {
            drag_outer_splitter(&mut session, offered, &hints, 65.0, 20.0)
        } else {
            assert!(adjust_outer_splitter(
                &mut session,
                offered,
                &hints,
                &[SurfaceSplitterAdjustment::Increment],
            ));
            measured_bounds(&mut session, offered, &hints)
        };
        for (item, x, width) in [
            (1, 0.0, 100.0),
            (2, 0.0, 120.0),
            (3, 124.0, 80.0),
            (4, 208.0, 100.0),
        ] {
            let rect = pane_bounds(&resized, item);
            assert!(
                (rect.x() - x).abs() < 1e-8 && (rect.width() - width).abs() < 1e-8,
                "{resized:?}"
            );
        }
        assert_eq!(auto_reflow_projection_count(), count);
    }
}

#[test]
fn widening_a_sparse_grid_preserves_the_capacity_of_its_empty_column() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 96.0).unwrap();
    let left = DockspaceNode::grid(DockspaceGridShape::Auto, [pane(1)]).unwrap();
    let right = grid(
        fixed(2, 2),
        [pane(2), pane(3), pane(4)],
        [1.0; 2],
        [3.0, 1.0],
    );
    let content = grid(fixed(1, 2), [left, right], [1.0], [3.0, 1.0]);
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap()
    .with_hidden_items([SECOND, FOURTH])
    .unwrap();
    let config = config(ResizePropagation::Outward)
        .to_builder()
        .splitter_keyboard_step(64.0)
        .build()
        .unwrap();
    for pointer in [false, true] {
        let mut session = DockspaceSession::from_layout_with_presentation_config(
            layout.clone(),
            DockPolicy::default(),
            config.clone(),
        )
        .unwrap();
        let before = measured_bounds(&mut session, offered, &[]);
        assert_eq!(pane_bounds(&before, 1).width(), 300.0);
        assert_eq!(pane_bounds(&before, 3).width(), 24.0);
        let after = if pointer {
            drag_outer_splitter(&mut session, offered, &[], 44.0, -64.0)
        } else {
            assert!(adjust_outer_splitter(
                &mut session,
                offered,
                &[],
                &[SurfaceSplitterAdjustment::Decrement],
            ));
            measured_bounds(&mut session, offered, &[])
        };
        assert!((pane_bounds(&after, 1).width() - 236.0).abs() < 1e-8);
        assert!(
            (pane_bounds(&after, 3).x() - 344.0).abs() < 1e-8,
            "{after:?}"
        );
        assert!(
            (pane_bounds(&after, 3).width() - 60.0).abs() < 1e-8,
            "{after:?}"
        );
    }
}

#[cfg(feature = "serde")]
#[test]
fn capped_inherited_split_keeps_its_shares_after_document_restore() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 96.0).unwrap();
    let hints = [(
        THIRD,
        PaneSizing::builder().max_width(40.0).build().unwrap(),
    )];
    let right = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [pane(2), pane(3)]).unwrap();
    let left = DockspaceNode::grid(DockspaceGridShape::Auto, [pane(1)]).unwrap();
    let content = grid(fixed(1, 2), [left, right], [1.0], [1.0; 2]);
    for mode in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for pointer in [false, true] {
            let mut session = persistent_session(content.clone(), 3, mode);
            let before = measured_bounds(&mut session, offered, &hints);
            assert_eq!(pane_bounds(&before, 2).width(), 156.0);
            let resized = if pointer {
                drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0)
            } else {
                assert!(adjust_outer_splitter(
                    &mut session,
                    offered,
                    &hints,
                    &[SurfaceSplitterAdjustment::Increment],
                ));
                measured_bounds(&mut session, offered, &hints)
            };
            assert!(
                (pane_bounds(&resized, 2).width() - 136.0).abs() < 1e-8,
                "{resized:?}"
            );
            assert!(
                (pane_bounds(&resized, 3).width() - 40.0).abs() < 1e-8,
                "{resized:?}"
            );

            let mut reopened = reopen(&mut session, mode);
            assert_eq!(measured_bounds(&mut reopened, offered, &hints), resized);
            let relaxed = measured_bounds(&mut reopened, offered, &[]);
            for id in [2, 3] {
                assert!(
                    (pane_bounds(&relaxed, id).width() - 88.0).abs() < 1e-8,
                    "{mode:?}: {relaxed:?}"
                );
            }
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn resizing_a_neighbor_keeps_stationary_grid_tracks_after_reopening() {
    let offered = LogicalRect::new(0.0, 0.0, 504.0, 78.0).unwrap();
    let auto = DockspaceNode::grid(DockspaceGridShape::Auto, (4..=9).map(pane)).unwrap();
    let far = grid(fixed(1, 2), [pane(3), auto], [1.0], [1.0, 1.0]);
    let content = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [(pane(1), 2.0), (pane(2), 2.0), (far, 1.0)],
    )
    .unwrap();
    let hints = [(3, 20.0), (4, 40.0), (7, 40.0)].map(|(id, width)| {
        (
            ItemId::new(id),
            PaneSizing::builder().min_width(width).build().unwrap(),
        )
    });
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let mut session = persistent_session(content.clone(), 9, propagation);
        let before = measured_bounds(&mut session, offered, &hints);
        assert_eq!(pane_bounds(&before, 4).x(), 420.0);

        let resized = drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0);
        let mut reopened = reopen(&mut session, propagation);
        for bounds in [resized, measured_bounds(&mut reopened, offered, &hints)] {
            for (item, width) in [(1, 214.0), (2, 174.0), (3, 20.0)] {
                let pane = pane_bounds(&bounds, item);
                assert!(
                    (pane.width() - width).abs() < 1e-8,
                    "{propagation:?}: pane {item} expected width {width}, got {pane:?}"
                );
            }
            assert!((pane_bounds(&bounds, 4).x() - 420.0).abs() < 1e-8);
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn dragging_an_outer_split_does_not_store_the_auto_grids_temporary_columns() {
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
        let content = six_pane_auto_grid([1.0, 3.0], [1.0, 2.0, 1.0]);
        let mut session = persistent_session(content, 7, propagation);
        let before = measured_bounds(&mut session, offered, &hints);
        assert_eq!(pane_bounds(&before, 2).y(), pane_bounds(&before, 4).y());
        let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0);
        assert_ne!(
            pane_bounds(&committed, 2).y(),
            pane_bounds(&committed, 4).y()
        );

        let mut reopened = reopen(&mut session, propagation);
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
        let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0);
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
        let committed = drag_outer_splitter(&mut session, offered, &hints, 44.0, 20.0);
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
        .prepare_splitter_adjustment(outer, SurfaceSplitterAdjustment::Maximum)
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
            plan.prepare_splitter_adjustment(boundary, step)
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
            if let Some(action) =
                plan.prepare_splitter_adjustment_sequence(boundary, steps.iter().copied())
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
