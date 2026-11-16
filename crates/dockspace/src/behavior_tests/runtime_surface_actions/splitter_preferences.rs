//! Explicit preferences share the durable, constrained splitter action path.

use super::*;
use crate::geometry::{AxisSizing, PaneSizing};
use crate::runtime::{DockPresentationConfig, DockspaceHostFrame, SplitterPaintRecord};

mod sequences;

fn width_sizing(preferred: Option<f64>, maximum: Option<f64>) -> PaneSizing {
    PaneSizing::new(
        AxisSizing::new(0.0, maximum, preferred).unwrap(),
        AxisSizing::default(),
    )
}

fn measure(frame: &mut DockspaceHostFrame<'_>, hints: &[(ItemId, PaneSizing)]) {
    measure_at(
        frame,
        hints,
        LogicalRect::new(0.0, 0.0, 601.0, 601.0).unwrap(),
    );
}

fn measure_at(
    frame: &mut DockspaceHostFrame<'_>,
    hints: &[(ItemId, PaneSizing)],
    bounds: LogicalRect,
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

fn measured(content: DockspaceNode, hints: &[(ItemId, PaneSizing)]) -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap();
    let config = DockPresentationConfig::builder()
        .minimum_pane_size(1.0, 1.0)
        .splitter_thickness(1.0)
        .splitter_keyboard_step(16.0)
        .build()
        .unwrap();
    let mut session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        config,
    )
    .unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    measure(&mut frame, hints);
    frame.commit().unwrap();
    session
}

fn pair_session(weights: [f64; 2], hints: &[(ItemId, PaneSizing)]) -> DockspaceSession {
    measured(
        DockspaceNode::split(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::tabs([FIRST]), weights[0]),
                (DockspaceNode::tabs([SECOND]), weights[1]),
            ],
        )
        .unwrap(),
        hints,
    )
}

fn nested_content() -> DockspaceNode {
    let inner = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
    )
    .unwrap();
    let column = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [inner, DockspaceNode::tabs([THIRD])],
    )
    .unwrap();
    DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [column, DockspaceNode::tabs([FOURTH])],
    )
    .unwrap()
}

fn outer_splitter(plan: SurfacePaintPlan<'_>) -> SplitterPaintRecord<'_> {
    let covered_area = |splitter: SplitterPaintRecord<'_>| {
        let before = splitter.before_bounds();
        let after = splitter.after_bounds();
        before.width() * before.height() + after.width() * after.height()
    };
    plan.splitters()
        .max_by(|first, second| covered_area(*first).total_cmp(&covered_area(*second)))
        .unwrap()
}

fn apply(
    session: &mut DockspaceSession,
    hints: &[(ItemId, PaneSizing)],
    steps: &[SurfaceSplitterAdjustment],
) {
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let splitter = outer_splitter(plan);
    let action = plan
        .prepare_splitter_adjustment_sequence(splitter.visual_id(), steps.iter().copied())
        .expect("the constrained splitter prepares the explicit preference sequence");
    frame.submit_surface_action(action).unwrap();
    measure(&mut frame, hints);
    frame.commit().unwrap();
}

fn pair_extents(session: &mut DockspaceSession) -> (f64, f64) {
    inspect_surface(session, |plan| {
        let splitter = outer_splitter(plan);
        let extent = |rect: LogicalRect| match splitter.axis() {
            DockspaceAxis::Horizontal => rect.width(),
            DockspaceAxis::Vertical => rect.height(),
        };
        (
            extent(splitter.before_bounds()),
            extent(splitter.after_bounds()),
        )
    })
}

fn assert_pair(session: &mut DockspaceSession, expected: (f64, f64)) {
    let actual = pair_extents(session);
    for (actual, expected) in [(actual.0, expected.0), (actual.1, expected.1)] {
        assert!((actual - expected).abs() < 1.0e-9, "{actual} != {expected}");
    }
}

fn all_split_weights(session: &DockspaceSession) -> Vec<Vec<f64>> {
    let content = session
        .view()
        .surface(SURFACE)
        .unwrap()
        .main_root()
        .unwrap()
        .content()
        .unwrap();
    let mut pending = vec![content];
    let mut weights = Vec::new();
    while let Some(node) = pending.pop() {
        if let Some(split) = node.split() {
            weights.push(split.weights().collect());
            pending.extend(split.children());
        }
    }
    weights
}

fn assert_geometry_close(first: &mut DockspaceSession, second: &mut DockspaceSession) {
    let panes = |session: &mut DockspaceSession| {
        inspect_surface(session, |plan| {
            plan.panes()
                .map(|pane| (pane.selected(), pane.bounds()))
                .collect::<Vec<_>>()
        })
    };
    let first = panes(first);
    let second = panes(second);
    assert_eq!(first.len(), second.len());
    for ((first_item, first), (second_item, second)) in first.into_iter().zip(second) {
        assert_eq!(first_item, second_item);
        for (actual, expected) in [
            (first.x(), second.x()),
            (first.y(), second.y()),
            (first.width(), second.width()),
            (first.height(), second.height()),
        ] {
            assert!((actual - expected).abs() < 1.0e-8, "{actual} != {expected}");
        }
    }
}

fn assert_durable_layouts_close(
    batch: &mut DockspaceSession,
    separate: &mut DockspaceSession,
    hints: &[(ItemId, PaneSizing)],
) {
    let batch_weights = all_split_weights(batch);
    let separate_weights = all_split_weights(separate);
    assert_eq!(batch_weights.len(), separate_weights.len());
    for (batch, separate) in batch_weights.iter().zip(&separate_weights) {
        assert_eq!(batch.len(), separate.len());
        for (actual, expected) in batch.iter().zip(separate) {
            assert!(
                (actual - expected).abs() <= 1.0e-12,
                "{actual} != {expected}"
            );
        }
    }
    assert_geometry_close(batch, separate);
    for session in [&mut *batch, &mut *separate] {
        let mut frame = session.begin_host_frame().unwrap();
        measure_at(
            &mut frame,
            hints,
            LogicalRect::new(0.0, 0.0, 6001.0, 6001.0).unwrap(),
        );
        frame.commit().unwrap();
    }
    assert_geometry_close(batch, separate);
    assert_eq!(all_split_weights(batch), batch_weights);
    assert_eq!(all_split_weights(separate), separate_weights);
}

#[test]
fn one_preference_restores_an_absolute_extent_on_either_side() {
    for (item, expected) in [(FIRST, (200.0, 400.0)), (SECOND, (400.0, 200.0))] {
        let hints = [(item, width_sizing(Some(200.0), None))];
        let mut session = pair_session([1.0, 1.0], &hints);
        assert_pair(&mut session, (300.0, 300.0));
        apply(
            &mut session,
            &hints,
            &[SurfaceSplitterAdjustment::Preferred],
        );
        assert_pair(&mut session, expected);
    }
}

#[test]
fn two_preferences_divide_the_available_pair_proportionally() {
    for (first, second) in [(100.0, 200.0), (1000.0, 2000.0)] {
        let hints = [
            (FIRST, width_sizing(Some(first), None)),
            (SECOND, width_sizing(Some(second), None)),
        ];
        let mut session = pair_session([3.0, 1.0], &hints);
        assert_pair(&mut session, (450.0, 150.0));
        apply(
            &mut session,
            &hints,
            &[SurfaceSplitterAdjustment::Preferred],
        );
        assert_pair(&mut session, (200.0, 400.0));
    }
}

#[test]
fn missing_preferences_use_the_existing_equalization_result() {
    let mut preferred = pair_session([1.0, 5.0], &[]);
    let mut equalized = pair_session([1.0, 5.0], &[]);
    apply(&mut preferred, &[], &[SurfaceSplitterAdjustment::Preferred]);
    apply(&mut equalized, &[], &[SurfaceSplitterAdjustment::Equalize]);
    assert_pair(&mut preferred, (300.0, 300.0));
    assert_eq!(split_weights(&preferred), split_weights(&equalized));
}

#[test]
fn full_layout_maximum_limits_the_preferred_destination() {
    let hints = [
        (FIRST, width_sizing(None, Some(220.0))),
        (SECOND, width_sizing(Some(100.0), None)),
    ];
    let mut session = pair_session([1.0, 5.0], &hints);
    assert_pair(&mut session, (100.0, 500.0));
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_pair(&mut session, (220.0, 380.0));
}

#[test]
fn ordinary_adjustments_and_later_layout_keep_the_committed_preference_weights() {
    let hints = [(FIRST, width_sizing(Some(200.0), None))];
    let mut session = pair_session([1.0, 1.0], &hints);
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_pair(&mut session, (200.0, 400.0));
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Increment],
    );
    assert_pair(&mut session, (216.0, 384.0));
    let weights = split_weights(&session);
    let mut idle = session.begin_host_frame().unwrap();
    measure(&mut idle, &hints);
    assert!(!idle.commit().unwrap().workspace_changed());
    assert_pair(&mut session, (216.0, 384.0));
    assert_eq!(split_weights(&session), weights);
}

#[test]
fn preference_batches_and_separate_steps_preserve_durable_resize_behavior() {
    use SurfaceSplitterAdjustment::{Decrement, Increment, Maximum, Preferred};
    let hints = [(FIRST, width_sizing(Some(200.0), None))];
    for (steps, expected) in [
        (vec![Preferred, Increment], (216.0, 384.0)),
        (vec![Increment, Preferred, Decrement], (184.0, 416.0)),
        (vec![Preferred, Maximum, Preferred], (200.0, 400.0)),
        (vec![Preferred, Increment, Decrement], (200.0, 400.0)),
    ] {
        let mut separate = pair_session([1.0, 1.0], &hints);
        for step in &steps {
            apply(&mut separate, &hints, &[*step]);
        }
        let mut batch = pair_session([1.0, 1.0], &hints);
        apply(&mut batch, &hints, &steps);
        assert_pair(&mut batch, expected);
        assert_pair(&mut separate, expected);
        assert_durable_layouts_close(&mut batch, &mut separate, &hints);
        assert_pair(&mut batch, (expected.0 * 10.0, expected.1 * 10.0));
        assert_pair(&mut separate, (expected.0 * 10.0, expected.1 * 10.0));
    }
}

#[test]
fn only_the_selected_tab_supplies_the_explicit_preference() {
    let hints = [
        (FIRST, width_sizing(Some(200.0), None)),
        (THIRD, width_sizing(Some(450.0), None)),
    ];
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::tabs([FIRST, THIRD]),
            DockspaceNode::tabs([SECOND]),
        ],
    )
    .unwrap();
    let mut session = measured(content, &hints);
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_pair(&mut session, (200.0, 400.0));
    let mut select = session.begin_host_frame().unwrap();
    select.select_item_current(THIRD).unwrap();
    measure(&mut select, &hints);
    select.commit().unwrap();
    assert_pair(&mut session, (200.0, 400.0));
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_pair(&mut session, (450.0, 150.0));
}

#[test]
fn vertical_preferences_add_visible_tab_chrome_exactly_once() {
    let hints = [(
        FIRST,
        PaneSizing::new(
            AxisSizing::default(),
            AxisSizing::new(0.0, None, Some(172.0)).unwrap(),
        ),
    )];
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
    )
    .unwrap();
    let mut session = measured(content, &hints);
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_pair(&mut session, (200.0, 400.0));
}

#[test]
fn subtree_preferences_combine_current_extents_actual_gaps_and_cross_axis_maxima() {
    let hints = [
        (FIRST, width_sizing(Some(100.0), None)),
        (THIRD, width_sizing(Some(200.0), None)),
    ];
    let mut session = measured(nested_content(), &hints);
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    // max(100 preferred + 149.5 current + 1 separator, 200 preferred).
    assert_pair(&mut session, (250.5, 349.5));
}

#[test]
fn unbounded_subtree_preferences_keep_ratios_above_the_current_host_extent() {
    let hints = [
        (FIRST, width_sizing(Some(1000.0), None)),
        (SECOND, width_sizing(Some(2000.0), None)),
        (THIRD, width_sizing(Some(1000.0), None)),
    ];
    let column = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
    )
    .unwrap();
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [column, DockspaceNode::tabs([THIRD])],
    )
    .unwrap();
    let mut session = measured(content, &hints);
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_pair(&mut session, (400.0, 200.0));
}

#[test]
fn nested_preference_batches_consume_the_geometry_after_each_prefix() {
    use SurfaceSplitterAdjustment::{Increment, Preferred};
    let hints = [
        (FIRST, width_sizing(Some(100.0), None)),
        (THIRD, width_sizing(Some(200.0), None)),
    ];
    for (steps, expected) in [
        ([Increment, Preferred], (266.5, 333.5)),
        ([Preferred, Preferred], (201.0, 399.0)),
    ] {
        let mut separate = measured(nested_content(), &hints);
        for step in steps {
            apply(&mut separate, &hints, &[step]);
        }
        assert_pair(&mut separate, expected);
        let mut batch = measured(nested_content(), &hints);
        apply(&mut batch, &hints, &steps);
        assert_pair(&mut batch, expected);
        assert_durable_layouts_close(&mut batch, &mut separate, &hints);
        assert_pair(&mut batch, (expected.0 * 10.0, expected.1 * 10.0));
        assert_pair(&mut separate, (expected.0 * 10.0, expected.1 * 10.0));
    }
}

#[test]
fn junction_preferences_require_one_common_incident_displacement() {
    for (right_content_height, compatible) in [(172.0, true), (272.0, false)] {
        let height_sizing = |preferred| {
            PaneSizing::new(
                AxisSizing::default(),
                AxisSizing::new(0.0, None, Some(preferred)).unwrap(),
            )
        };
        let hints = [
            (FIRST, height_sizing(172.0)),
            (THIRD, height_sizing(right_content_height)),
        ];
        let column = |top, bottom| {
            DockspaceNode::equal_split(
                DockspaceAxis::Vertical,
                [DockspaceNode::tabs([top]), DockspaceNode::tabs([bottom])],
            )
            .unwrap()
        };
        let content = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [column(FIRST, SECOND), column(THIRD, FOURTH)],
        )
        .unwrap();
        let mut session = measured(content, &hints);
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let junction = plan.splitter_junctions().next().unwrap();
        let action = plan.prepare_splitter_junction_adjustment(
            junction.visual_id(),
            DockspaceAxis::Vertical,
            SurfaceSplitterAdjustment::Preferred,
        );
        assert_eq!(action.is_some(), compatible);
        if let Some(action) = action {
            frame.submit_surface_action(action).unwrap();
            measure(&mut frame, &hints);
        } else {
            frame
                .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
                .unwrap();
        }
        assert_eq!(frame.commit().unwrap().workspace_changed(), compatible);
        if compatible {
            inspect_surface(&mut session, |plan| {
                let columns = plan
                    .splitters()
                    .filter(|splitter| splitter.axis() == DockspaceAxis::Vertical)
                    .collect::<Vec<_>>();
                assert_eq!(columns.len(), 2);
                for splitter in columns {
                    assert!((splitter.before_bounds().height() - 200.0).abs() < 1.0e-9);
                    assert!((splitter.after_bounds().height() - 400.0).abs() < 1.0e-9);
                }
            });
        }
    }
}
