//! Public resize actions and presented input share the composable track solver.

use std::num::NonZeroUsize;

use super::*;
use crate::geometry::PaneSizing;
use crate::model::{DockspaceGridShape, DockspaceGridShares};
use crate::runtime::{DockPresentationConfig, DockspaceHostFrame, ResizePropagation};

mod grid_commands;
mod measurement_changes;

fn config(propagation: ResizePropagation) -> DockPresentationConfig {
    DockPresentationConfig::builder()
        .minimum_pane_size(1.0, 1.0)
        .splitter_thickness(4.0)
        .splitter_keyboard_step(20.0)
        .resize_propagation(propagation)
        .build()
        .unwrap()
}

fn measure(
    frame: &mut DockspaceHostFrame<'_>,
    bounds: LogicalRect,
    hints: &[(ItemId, PaneSizing)],
) {
    frame
        .measure_surface_with(SURFACE, |request| {
            Ok::<_, crate::runtime::DockspaceRuntimeError>(match request {
                SurfaceMeasurementRequest::DockBounds { .. }
                | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                    SurfaceMeasurementAnswer::Bounds(bounds)
                }
                SurfaceMeasurementRequest::PaneSizing { item, .. } => {
                    SurfaceMeasurementAnswer::PaneSizing(
                        hints
                            .iter()
                            .find_map(|(id, sizing)| (Some(*id) == item).then_some(*sizing))
                            .unwrap_or_default(),
                    )
                }
                SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                    SurfaceMeasurementAnswer::TabIntrinsic(64.0)
                }
                SurfaceMeasurementRequest::TabStrip { .. } => {
                    SurfaceMeasurementAnswer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                }
            })
        })
        .unwrap();
}

fn session(
    content: DockspaceNode,
    bounds: LogicalRect,
    propagation: ResizePropagation,
    hints: &[(ItemId, PaneSizing)],
) -> DockspaceSession {
    session_with_config(content, bounds, config(propagation), hints)
}

fn session_with_config(
    content: DockspaceNode,
    bounds: LogicalRect,
    config: DockPresentationConfig,
    hints: &[(ItemId, PaneSizing)],
) -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap();
    let mut session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        config,
    )
    .unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, bounds, hints);
    frame.commit().unwrap();
    session
}

fn fixed(rows: usize, columns: usize) -> DockspaceGridShape {
    DockspaceGridShape::Fixed {
        rows: NonZeroUsize::new(rows).unwrap(),
        columns: NonZeroUsize::new(columns).unwrap(),
    }
}

fn bounds(session: &mut DockspaceSession) -> Vec<(ItemId, LogicalRect)> {
    inspect_surface(session, |plan| {
        plan.panes()
            .map(|pane| (pane.item(), pane.content_geometry().clip()))
            .collect()
    })
}

fn measured_bounds(
    session: &mut DockspaceSession,
    offered: LogicalRect,
    hints: &[(ItemId, PaneSizing)],
) -> Vec<(ItemId, LogicalRect)> {
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, offered, hints);
    let panes = frame
        .paint_plan(SURFACE)
        .unwrap()
        .unwrap()
        .panes()
        .map(|pane| (pane.item(), pane.content_geometry().clip()))
        .collect();
    frame.commit().unwrap();
    panes
}

fn assert_cells(session: &mut DockspaceSession, sizes: [(f64, f64); 4]) {
    let actual = bounds(session);
    for (item, (width, height)) in [FIRST, SECOND, THIRD, FOURTH].into_iter().zip(sizes) {
        let rect = actual.iter().find(|(id, _)| *id == item).unwrap().1;
        assert!(
            (rect.width() - width).abs() < 1e-8,
            "{item:?}: {rect:?}, width {width}"
        );
        assert!(
            (rect.height() - height).abs() < 1e-8,
            "{item:?}: {rect:?}, height {height}"
        );
    }
}

fn grid_content() -> DockspaceNode {
    DockspaceNode::composed_tabs([
        DockspaceNode::grid(
            fixed(2, 2),
            [FIRST, SECOND, THIRD, FOURTH].map(DockspaceNode::pane),
        )
        .unwrap()
        .named("Grid"),
        DockspaceNode::pane(FIFTH),
    ])
}

#[test]
fn equalize_keeps_the_first_step_when_a_t_junction_becomes_a_cross() {
    use SurfaceSplitterAdjustment::Equalize;

    let offered = LogicalRect::new(0.0, 0.0, 604.0, 544.0).unwrap();
    let left = DockspaceNode::split(
        DockspaceAxis::Vertical,
        [
            (DockspaceNode::pane(FIRST), 15.0),
            (DockspaceNode::pane(SECOND), 17.0),
        ],
    )
    .unwrap();
    let right = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [THIRD, FOURTH].map(DockspaceNode::pane),
    )
    .unwrap();
    let content = DockspaceNode::equal_split(DockspaceAxis::Horizontal, [left, right]).unwrap();
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for steps in [&[Equalize][..], &[Equalize, Equalize][..]] {
            let mut session = session(content.clone(), offered, propagation, &[]);
            assert_cells(
                &mut session,
                [
                    (300.0, 240.0),
                    (300.0, 272.0),
                    (300.0, 256.0),
                    (300.0, 256.0),
                ],
            );
            let mut frame = session.begin_host_frame().unwrap();
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let tee = plan
                .splitter_junctions()
                .min_by(|a, b| a.hit_bounds().y().total_cmp(&b.hit_bounds().y()))
                .unwrap();
            assert_eq!(tee.arm_count(), 3);
            let id = tee.visual_id();
            let compiled = crate::presentation::compiler::surface_compilation_count();
            let action = plan
                .prepare_splitter_junction_adjustment_sequence(
                    id,
                    DockspaceAxis::Vertical,
                    steps.iter().copied(),
                )
                .expect("the accepted prefix still commits when it changes the junction");
            assert_eq!(
                crate::presentation::compiler::surface_compilation_count(),
                compiled
            );
            frame.submit_surface_action(action).unwrap();
            measure(&mut frame, offered, &[]);
            assert!(frame.commit().unwrap().workspace_changed());
            assert_cells(&mut session, [(300.0, 256.0); 4]);
            inspect_surface(&mut session, |plan| {
                assert_eq!(plan.splitter_junctions().next().unwrap().arm_count(), 4);
                assert!(
                    plan.prepare_splitter_junction_adjustment(
                        id,
                        DockspaceAxis::Vertical,
                        Equalize
                    )
                    .is_none()
                );
            });
        }
    }
}

#[test]
fn auto_grid_reversal_materializes_the_current_shape_before_returning_to_the_old_shape() {
    use SurfaceSplitterAdjustment::{Decrement, Increment};

    let content = DockspaceNode::grid_with_shares(
        DockspaceGridShape::Auto,
        [FIRST, SECOND, THIRD, FOURTH].map(DockspaceNode::pane),
        DockspaceGridShares::new([1.0, 3.0], [1.0, 3.0]).unwrap(),
    )
    .unwrap();
    let wide = LogicalRect::new(0.0, 0.0, 803.0, 100.0).unwrap();
    let square = LogicalRect::new(0.0, 0.0, 401.0, 401.0).unwrap();
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        let config = DockPresentationConfig::builder()
            .minimum_pane_size(1.0, 1.0)
            .splitter_thickness(1.0)
            .splitter_keyboard_step(16.0)
            .resize_propagation(propagation)
            .build()
            .unwrap();
        for batches in [
            &[&[Increment, Decrement][..]][..],
            &[&[Increment][..], &[Decrement][..]][..],
        ] {
            let mut session = session_with_config(content.clone(), wide, config.clone(), &[]);
            assert!(
                bounds(&mut session)
                    .iter()
                    .all(|(_, rect)| rect.y() == 28.0 && rect.width() == 200.0)
            );
            for steps in batches {
                let mut frame = session.begin_host_frame().unwrap();
                let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                let handle = plan
                    .splitters()
                    .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
                    .min_by(|a, b| a.draw_bounds().x().total_cmp(&b.draw_bounds().x()))
                    .unwrap();
                let action = plan
                    .prepare_splitter_adjustment_sequence(handle.visual_id(), steps.iter().copied())
                    .unwrap();
                frame.submit_surface_action(action).unwrap();
                measure(&mut frame, wide, &[]);
                frame.commit().unwrap();
            }
            let restored = measured_bounds(&mut session, square, &[]);
            assert!(
                restored
                    .iter()
                    .all(|(_, rect)| (rect.width() - 200.0).abs() < 1e-8),
                "{restored:?}"
            );
        }
    }
}

#[test]
fn ancestor_splitter_round_trip_preserves_auto_grid_shares_across_reflow() {
    use SurfaceSplitterAdjustment::{Decrement, Increment};

    let grid_items = [SECOND, THIRD, FOURTH, FIFTH];
    let grid = DockspaceNode::grid_with_shares(
        DockspaceGridShape::Auto,
        grid_items.map(DockspaceNode::pane),
        DockspaceGridShares::new([1.0, 3.0], [3.0, 1.0]).unwrap(),
    )
    .unwrap();
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::pane(FIRST), grid],
    )
    .unwrap();
    let offered = LogicalRect::new(0.0, 0.0, 401.0, 428.0).unwrap();
    let resize = |session: &mut DockspaceSession, steps: &[SurfaceSplitterAdjustment]| {
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let outer = plan
            .splitters()
            .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
            .min_by(|a, b| a.draw_bounds().x().total_cmp(&b.draw_bounds().x()))
            .unwrap();
        let action = plan
            .prepare_splitter_adjustment_sequence(outer.visual_id(), steps.iter().copied())
            .unwrap();
        frame.submit_surface_action(action).unwrap();
        measure(&mut frame, offered, &[]);
        frame.commit().unwrap();
    };
    let mut stepwise = session_with_config(
        content.clone(),
        offered,
        DockPresentationConfig::default(),
        &[],
    );
    resize(&mut stepwise, &[Increment]);
    assert!(
        bounds(&mut stepwise)
            .into_iter()
            .all(|(item, rect)| { item == FIRST || rect.width() == 184.0 })
    );
    resize(&mut stepwise, &[Decrement]);
    let expected = bounds(&mut stepwise);
    for (item, height) in grid_items.into_iter().zip([99.75, 99.75, 299.25, 299.25]) {
        assert_eq!(
            expected
                .iter()
                .find(|(id, _)| *id == item)
                .unwrap()
                .1
                .height(),
            height
        );
    }

    let mut batched = session_with_config(content, offered, DockPresentationConfig::default(), &[]);
    resize(&mut batched, &[Increment, Decrement]);
    assert_eq!(bounds(&mut batched), expected);
}

fn six_pane_auto_grid(rows: [f64; 2], columns: [f64; 3]) -> DockspaceNode {
    let grid = DockspaceNode::grid_with_shares(
        DockspaceGridShape::Auto,
        [SECOND, THIRD, FOURTH, FIFTH, SIXTH, SEVENTH].map(DockspaceNode::pane),
        DockspaceGridShares::new(rows, columns).unwrap(),
    )
    .unwrap();
    DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::pane(FIRST), grid],
    )
    .unwrap()
}

fn adjust_outer_splitter(
    session: &mut DockspaceSession,
    offered: LogicalRect,
    hints: &[(ItemId, PaneSizing)],
    keys: &[SurfaceSplitterAdjustment],
) -> bool {
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let outer = plan
        .splitters()
        .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
        .min_by(|a, b| a.draw_bounds().x().total_cmp(&b.draw_bounds().x()))
        .unwrap();
    if let Some(action) =
        plan.prepare_splitter_adjustment_sequence(outer.visual_id(), keys.iter().copied())
    {
        frame.submit_surface_action(action).unwrap();
    }
    measure(&mut frame, offered, hints);
    frame.commit().unwrap().workspace_changed()
}

#[cfg(feature = "serde")]
#[test]
fn capped_auto_grid_keeps_latent_shares_after_ancestor_resize_and_reopen() {
    use crate::runtime::{DockspaceDocumentBootstrap, DockspaceDocumentId};
    use SurfaceSplitterAdjustment::{Decrement, Increment};

    let offered = LogicalRect::new(0.0, 0.0, 604.0, 508.0).unwrap();
    let short = LogicalRect::new(0.0, 0.0, 604.0, 228.0).unwrap();
    let hints = [THIRD, FIFTH, SEVENTH].map(|item| {
        (
            item,
            PaneSizing::builder().fixed_width(80.0).build().unwrap(),
        )
    });
    for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
        for batches in [
            &[&[Increment, Decrement][..]][..],
            &[&[Increment][..], &[Decrement][..]][..],
        ] {
            let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(ROOT, six_pane_auto_grid([1.0, 3.0], [1.0, 2.0, 1.0])),
            )])
            .unwrap();
            let mut bootstrap =
                DockspaceDocumentBootstrap::new(DockspaceDocumentId::from_bytes([4; 16]));
            for index in 1..=7 {
                assert_eq!(
                    bootstrap.ensure_item(format!("pane-{index}")).unwrap(),
                    ItemId::new(index)
                );
            }
            let config = config(propagation);
            let mut session = DockspaceSession::from_persistent_layout_with_presentation_config(
                layout,
                DockPolicy::default(),
                config.clone(),
                bootstrap,
            )
            .unwrap();
            let initial = measured_bounds(&mut session, offered, &hints);
            for batch in batches {
                adjust_outer_splitter(&mut session, offered, &hints, batch);
            }
            assert_eq!(bounds(&mut session), initial);
            let document = session.save_document_json().unwrap();
            let mut reopened = DockspaceSession::from_document_json_with_presentation_config(
                &document,
                DockPolicy::default(),
                config,
                |_, _| true,
            )
            .unwrap();
            let unconstrained = measured_bounds(&mut reopened, offered, &[]);
            assert_eq!(unconstrained[1].1.width(), 148.0);
            assert_eq!(unconstrained[2].1.width(), 148.0);
            let original_shape = measured_bounds(&mut reopened, short, &[]);
            assert_eq!(original_shape[1].1.height(), 49.0);
            assert_eq!(original_shape[4].1.height(), 147.0);
        }
    }
}

#[test]
fn a_capped_reflow_preserves_the_accepted_splitter_batch_prefix() {
    use SurfaceSplitterAdjustment::{Decrement, Increment};

    let offered = LogicalRect::new(0.0, 0.0, 424.0, 228.0).unwrap();
    let hints = [SECOND, FOURTH, SIXTH].map(|item| {
        (
            item,
            PaneSizing::builder().fixed_width(40.0).build().unwrap(),
        )
    });
    let content = six_pane_auto_grid([1.0; 2], [1.0; 3]);
    for last in [Increment, Decrement] {
        let mut separate = session(
            content.clone(),
            offered,
            ResizePropagation::Adjacent,
            &hints,
        );
        assert!(adjust_outer_splitter(
            &mut separate,
            offered,
            &hints,
            &[Increment]
        ));
        assert!(!adjust_outer_splitter(
            &mut separate,
            offered,
            &hints,
            &[last]
        ));
        let expected = bounds(&mut separate);
        assert!((expected[0].1.width() - 230.0).abs() < 1e-8);

        let mut batched = session(
            content.clone(),
            offered,
            ResizePropagation::Adjacent,
            &hints,
        );
        assert!(adjust_outer_splitter(
            &mut batched,
            offered,
            &hints,
            &[Increment, last]
        ));
        assert_eq!(bounds(&mut batched), expected);
    }
}

#[test]
fn a_deferred_grid_reset_recomputes_current_weights_only_for_the_captured_cell_map() {
    use SurfaceSplitterAdjustment::{Equalize, Increment, Preferred};

    for (width, reflowed) in [(424.0, true), (524.0, false)] {
        let offered = LogicalRect::new(0.0, 0.0, width, 228.0).unwrap();
        for reset in [Equalize, Preferred] {
            let mut session = session(
                six_pane_auto_grid([1.0, 3.0], [1.0, 2.0, 1.0]),
                offered,
                ResizePropagation::Adjacent,
                &[],
            );
            let action = inspect_surface(&mut session, |plan| {
                let mut dividers = plan
                    .splitters()
                    .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
                    .collect::<Vec<_>>();
                dividers.sort_by(|a, b| a.draw_bounds().x().total_cmp(&b.draw_bounds().x()));
                plan.prepare_splitter_adjustment(dividers[1].visual_id(), reset)
                    .unwrap()
                    .into_deferred()
            });
            assert!(adjust_outer_splitter(
                &mut session,
                offered,
                &[],
                &[Increment, Increment]
            ));
            let before = bounds(&mut session);
            let mut frame = session.begin_host_frame().unwrap();
            frame.submit_deferred_surface_action(action).unwrap();
            measure(&mut frame, offered, &[]);
            let report = frame.commit().unwrap();
            assert_eq!(report.workspace_changed(), !reflowed);
            let after = bounds(&mut session);
            if reflowed {
                assert_eq!(after, before);
            } else {
                assert!((after[1].1.width() - after[2].1.width()).abs() < 1e-8);
            }
        }
    }
}

#[test]
fn presented_grid_junction_commits_both_shared_axes_and_cancel_preserves_the_graph() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 232.0).unwrap();
    for cancel in [false, true] {
        let mut session = session(grid_content(), offered, ResizePropagation::Adjacent, &[]);
        assert_cells(&mut session, [(200.0, 100.0); 4]);
        let original = session.version();
        present_pointer_surface(&mut session);
        let press = inspect_surface(&mut session, |plan| {
            let junction = plan.splitter_junctions().next().unwrap();
            assert!(plan.splitter_junction_operable(junction));
            plan.receiver_for_splitter_junction(junction)
                .unwrap()
                .center()
        });
        let moved = LogicalPoint::new(press.x() + 20.0, press.y() + 10.0).unwrap();
        submit_host_edges_with(
            &mut session,
            &[(
                SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
                Some(press),
            )],
            |frame| measure(frame, offered, &[]),
        );
        assert!(session.has_active_gesture());
        submit_host_edges_with(
            &mut session,
            &[(SurfacePointerEvent::Moved, Some(moved))],
            |frame| measure(frame, offered, &[]),
        );
        assert_eq!(
            session.version(),
            original,
            "live geometry is a candidate, not an early graph mutation"
        );
        assert!(
            session.has_active_gesture(),
            "a shared-grid update must keep its frozen session"
        );
        let terminal = if cancel {
            SurfacePointerEvent::StreamCancelled(
                SurfacePointerCancelReason::ExplicitPlatformCancellation,
            )
        } else {
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary)
        };
        let terminal_report =
            submit_host_edges_with(&mut session, &[(terminal, Some(moved))], |frame| {
                measure(frame, offered, &[])
            });
        assert!(!session.has_active_gesture());
        if cancel {
            assert_eq!(session.version(), original);
            assert_cells(&mut session, [(200.0, 100.0); 4]);
        } else {
            assert_eq!(
                session.version().revision().get(),
                original.revision().get() + 1,
                "{terminal_report:?}"
            );
            assert_cells(
                &mut session,
                [(220.0, 110.0), (180.0, 110.0), (220.0, 90.0), (180.0, 90.0)],
            );
        }
    }
}

#[test]
fn grid_keyboard_adjustment_shares_the_column_and_inactive_ancestor_revokes_an_old_action() {
    let offered = LogicalRect::new(0.0, 0.0, 404.0, 232.0).unwrap();
    let mut session = session(grid_content(), offered, ResizePropagation::Adjacent, &[]);
    let stale = {
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let splitter = plan
            .splitters()
            .find(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
            .unwrap();
        let action = plan
            .prepare_splitter_adjustment(splitter.visual_id(), SurfaceSplitterAdjustment::Increment)
            .unwrap();
        let stale = plan
            .prepare_splitter_adjustment(splitter.visual_id(), SurfaceSplitterAdjustment::Increment)
            .unwrap();
        frame.submit_surface_action(action).unwrap();
        measure(&mut frame, offered, &[]);
        frame.commit().unwrap();
        stale
    };
    assert_cells(
        &mut session,
        [
            (220.0, 100.0),
            (180.0, 100.0),
            (220.0, 100.0),
            (180.0, 100.0),
        ],
    );
    let mut frame = session.begin_host_frame().unwrap();
    frame.select_item_current(FIFTH).unwrap();
    measure(&mut frame, offered, &[]);
    frame.commit().unwrap();
    let version = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_surface_action(stale).unwrap();
    measure(&mut frame, offered, &[]);
    assert!(!frame.commit().unwrap().workspace_changed());
    assert_eq!(session.version(), version);
    assert_eq!(
        bounds(&mut session)
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        [FIFTH]
    );
}

#[test]
fn outward_accessibility_range_reports_the_whole_owner_boundary_and_reaches_its_limit() {
    let content = DockspaceNode::grid_with_shares(
        fixed(1, 3),
        [FIRST, SECOND, THIRD].map(DockspaceNode::pane),
        DockspaceGridShares::new([1.0], [1.0, 1.0, 1.0]).unwrap(),
    )
    .unwrap();
    let offered = LogicalRect::new(0.0, 0.0, 308.0, 228.0).unwrap();
    let hints = [
        (
            FIRST,
            PaneSizing::builder().min_width(20.0).build().unwrap(),
        ),
        (
            SECOND,
            PaneSizing::builder().min_width(80.0).build().unwrap(),
        ),
        (
            THIRD,
            PaneSizing::builder().min_width(20.0).build().unwrap(),
        ),
    ];
    let mut session = session(content, offered, ResizePropagation::Outward, &hints);
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let splitter = plan.splitters().next().unwrap();
    let range = plan.splitter_resize_range(splitter.visual_id()).unwrap();
    assert!((range.value() - 1.0 / 3.0).abs() < 1e-10);
    assert!((range.minimum() - 20.0 / 300.0).abs() < 1e-10);
    assert!((range.maximum() - 200.0 / 300.0).abs() < 1e-10);
    let action = plan
        .prepare_splitter_adjustment(splitter.visual_id(), SurfaceSplitterAdjustment::Maximum)
        .unwrap();
    frame.submit_surface_action(action).unwrap();
    measure(&mut frame, offered, &hints);
    frame.commit().unwrap();
    let actual = bounds(&mut session);
    for (item, expected) in [(FIRST, 200.0), (SECOND, 80.0), (THIRD, 20.0)] {
        assert!(
            (actual.iter().find(|(id, _)| *id == item).unwrap().1.width() - expected).abs() < 1e-8
        );
    }
}

#[test]
fn ancestor_resize_reflows_auto_grid_with_the_same_candidate_and_committed_geometry() {
    let grid = DockspaceNode::grid(
        DockspaceGridShape::Auto,
        [SECOND, THIRD, FOURTH, FIFTH].map(DockspaceNode::pane),
    )
    .unwrap();
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::pane(FIRST), grid],
    )
    .unwrap();
    let offered = LogicalRect::new(0.0, 0.0, 1004.0, 232.0).unwrap();
    let mut separate = session(content.clone(), offered, ResizePropagation::Adjacent, &[]);
    let mut batch = session(content, offered, ResizePropagation::Adjacent, &[]);
    let apply = |session: &mut DockspaceSession, adjustments: &[SurfaceSplitterAdjustment]| {
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let outer = plan
            .splitters()
            .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
            .max_by(|a, b| {
                a.before_bounds()
                    .width()
                    .total_cmp(&b.before_bounds().width())
            })
            .unwrap();
        let action = plan
            .prepare_splitter_adjustment_sequence(outer.visual_id(), adjustments.iter().copied())
            .unwrap();
        frame.submit_surface_action(action).unwrap();
        measure(&mut frame, offered, &[]);
        let preview = frame
            .paint_plan(SURFACE)
            .unwrap()
            .unwrap()
            .panes()
            .map(|pane| (pane.item(), pane.content_geometry().clip()))
            .collect::<Vec<_>>();
        assert!(frame.commit().unwrap().workspace_changed());
        assert_eq!(bounds(session), preview);
        preview
    };
    for (adjustment, expected_widths, expected_height) in [
        (SurfaceSplitterAdjustment::Increment, [198.0; 4], 100.0),
        (
            SurfaceSplitterAdjustment::Decrement,
            [182.0, 102.0, 102.0, 102.0],
            204.0,
        ),
    ] {
        for _ in 0..5 {
            apply(&mut separate, &[adjustment]);
        }
        let preview = apply(&mut batch, &[adjustment; 5]);
        assert_eq!(bounds(&mut separate), preview);
        for (item, expected_width) in [SECOND, THIRD, FOURTH, FIFTH]
            .into_iter()
            .zip(expected_widths)
        {
            let rect = preview.iter().find(|(id, _)| *id == item).unwrap().1;
            assert!(
                (rect.width() - expected_width).abs() < 1e-8,
                "{item:?}: {rect:?}"
            );
            assert!(
                (rect.height() - expected_height).abs() < 1e-8,
                "{item:?}: {rect:?}"
            );
        }
    }
}

#[test]
fn an_infeasible_auto_resize_release_keeps_the_last_verified_preview_and_respects_revocation() {
    let offered = LogicalRect::new(0.0, 0.0, 1004.0, 232.0).unwrap();
    let hints = [SECOND, THIRD, FOURTH, FIFTH].map(|item| {
        (
            item,
            PaneSizing::builder().min_height(120.0).build().unwrap(),
        )
    });
    for revoke in [false, true] {
        let grid = DockspaceNode::grid(
            DockspaceGridShape::Auto,
            [SECOND, THIRD, FOURTH, FIFTH].map(DockspaceNode::pane),
        )
        .unwrap();
        let content = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::pane(FIRST), grid],
        )
        .unwrap();
        let mut session = session(content, offered, ResizePropagation::Adjacent, &hints);
        let original = bounds(&mut session);
        let version = session.version();
        present_pointer_surface(&mut session);
        let press = inspect_surface(&mut session, |plan| {
            let outer = plan
                .splitters()
                .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
                .max_by(|a, b| {
                    a.before_bounds()
                        .width()
                        .total_cmp(&b.before_bounds().width())
                })
                .unwrap();
            plan.receiver_for_splitter(outer).unwrap().center()
        });
        let accepted = LogicalPoint::new(press.x() + 20.0, press.y()).unwrap();
        let infeasible = LogicalPoint::new(press.x() + 400.0, press.y()).unwrap();
        submit_host_edges_with(
            &mut session,
            &[(
                SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
                Some(press),
            )],
            |frame| measure(frame, offered, &hints),
        );
        assert!(session.has_active_gesture());
        submit_host_edges_with(
            &mut session,
            &[(SurfacePointerEvent::Moved, Some(accepted))],
            |frame| measure(frame, offered, &hints),
        );
        let preview = measured_bounds(&mut session, offered, &hints);
        assert_ne!(preview, original);
        assert_eq!(session.version(), version);
        submit_host_edges_with(
            &mut session,
            &[(SurfacePointerEvent::Moved, Some(infeasible))],
            |frame| measure(frame, offered, &hints),
        );
        assert!(session.has_active_gesture());
        assert_eq!(
            measured_bounds(&mut session, offered, &hints),
            preview,
            "an infeasible move retains the verified geometry"
        );
        if revoke {
            let mut policy = session.policy().clone();
            policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Horizontal, false);
            let mut frame = session.begin_host_frame().unwrap();
            frame.replace_policy(policy).unwrap();
            measure(&mut frame, offered, &hints);
            frame.commit().unwrap();
        }
        let release = submit_host_edges_with(
            &mut session,
            &[(
                SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
                Some(infeasible),
            )],
            |frame| measure(frame, offered, &hints),
        );
        assert!(!session.has_active_gesture());
        if revoke {
            assert!(!release.workspace_changed());
            assert_eq!(
                measured_bounds(&mut session, offered, &hints),
                original,
                "a saved sample cannot restore revoked resize permission"
            );
        } else {
            assert!(release.workspace_changed(), "{release:?}");
            assert_eq!(
                measured_bounds(&mut session, offered, &hints),
                preview,
                "release commits the same geometry the user saw"
            );
        }
    }
}
