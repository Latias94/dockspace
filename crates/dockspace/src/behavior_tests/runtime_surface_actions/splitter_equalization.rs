//! Equalization follows exact solved extents and the ordinary atomic resize path.

use super::*;
use crate::model::DockPlacement;
use crate::runtime::DockPresentationConfig;

fn weighted(
    axis: DockspaceAxis,
    children: impl IntoIterator<Item = (DockspaceNode, f64)>,
) -> DockspaceNode {
    DockspaceNode::split(axis, children).unwrap()
}

fn measured(content: DockspaceNode, policy: DockPolicy) -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, policy).unwrap();
    install_ready_candidate(&mut session);
    session
}

fn equalize(session: &mut DockspaceSession, index: usize) -> HostFrameReport {
    equalize_with_metrics(session, index, metrics())
}

fn equalize_with_metrics(
    session: &mut DockspaceSession,
    index: usize,
    measurements: UniformSurfaceMetrics,
) -> HostFrameReport {
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let splitter = plan.splitters().nth(index).unwrap();
    let source_bounds = (splitter.before_bounds(), splitter.after_bounds());
    let action = plan
        .prepare_splitter_adjustment(splitter.visual_id(), SurfaceSplitterAdjustment::Equalize)
        .unwrap();
    frame.submit_surface_action(action).unwrap();
    frame
        .measure_surface(SURFACE, measurements)
        .unwrap_or_else(|error| {
            panic!("equalization measurement failed: {error:?}; measurements={measurements:?}; source_bounds={source_bounds:?}")
        });
    frame.commit().unwrap()
}

fn pane_bounds(session: &mut DockspaceSession) -> Vec<(ItemId, LogicalRect)> {
    inspect_surface(session, |plan| {
        plan.panes()
            .filter_map(|pane| pane.selected().map(|item| (item, pane.bounds())))
            .collect()
    })
}

fn bound(bounds: &[(ItemId, LogicalRect)], item: ItemId) -> LogicalRect {
    bounds
        .iter()
        .find(|(current, _)| *current == item)
        .unwrap()
        .1
}

fn extent(bounds: LogicalRect, axis: DockspaceAxis) -> f64 {
    match axis {
        DockspaceAxis::Horizontal => bounds.width(),
        DockspaceAxis::Vertical => bounds.height(),
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 0.0002,
        "{actual} != {expected}"
    );
}

fn measure(request: SurfaceMeasurementRequest) -> SurfaceMeasurementAnswer {
    match request {
        SurfaceMeasurementRequest::DockBounds { .. }
        | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
            SurfaceMeasurementAnswer::Bounds(LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap())
        }
        SurfaceMeasurementRequest::PaneMinimum { .. } => {
            SurfaceMeasurementAnswer::PaneMinimum(LogicalSize::new(64.0, 48.0).unwrap())
        }
        SurfaceMeasurementRequest::TabIntrinsic { .. } => {
            SurfaceMeasurementAnswer::TabIntrinsic(96.0)
        }
        SurfaceMeasurementRequest::TabStrip { .. } => {
            SurfaceMeasurementAnswer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
        }
    }
}

fn scaled_config(scale: f64) -> DockPresentationConfig {
    DockPresentationConfig::builder()
        .minimum_pane_size(80.0 * scale, 60.0 * scale)
        .tab_bar_height(28.0 * scale)
        .tab_group_grip_extent(28.0 * scale)
        .tab_close_extent(16.0 * scale)
        .tab_horizontal_padding(10.0 * scale)
        .tab_min_width(72.0 * scale)
        .tab_max_width(220.0 * scale)
        .splitter_thickness(scale)
        .splitter_hit_extent(6.0 * scale)
        .splitter_keyboard_step(1.0e-12 * scale)
        .build()
        .unwrap()
}

#[test]
fn equalization_balances_only_adjacent_nary_children_with_any_central_position() {
    for axis in [DockspaceAxis::Horizontal, DockspaceAxis::Vertical] {
        for central in [None, Some(FIRST), Some(SECOND), Some(THIRD)] {
            let content = weighted(
                axis,
                [FIRST, SECOND, THIRD]
                    .into_iter()
                    .zip([1.0, 3.0, 4.0])
                    .map(|(item, weight)| {
                        let child = if central == Some(item) {
                            DockspaceNode::central_tabs([item])
                        } else {
                            DockspaceNode::tabs([item])
                        };
                        (child, weight)
                    }),
            );
            let mut session = measured(content, DockPolicy::default());
            let before = pane_bounds(&mut session);
            assert!(extent(bound(&before, FIRST), axis) < extent(bound(&before, SECOND), axis));
            let version = session.version();
            let report = equalize(&mut session, 0);
            assert!(report.workspace_changed());
            assert_eq!(
                session.version().revision().get(),
                version.revision().get() + 1
            );
            let after = pane_bounds(&mut session);
            close(
                extent(bound(&after, FIRST), axis),
                extent(bound(&after, SECOND), axis),
            );
            let original_far = bound(&before, THIRD);
            let adjusted_far = bound(&after, THIRD);
            // Normalized shares can round an untouched child's solved boundary.
            for (actual, expected) in [
                (adjusted_far.min().x(), original_far.min().x()),
                (adjusted_far.min().y(), original_far.min().y()),
                (adjusted_far.max().x(), original_far.max().x()),
                (adjusted_far.max().y(), original_far.max().y()),
            ] {
                close(actual, expected);
            }
            assert_eq!(after.len(), 3);

            let weights = split_weights(&session);
            let version = session.version();
            assert!(!equalize(&mut session, 0).workspace_changed());
            assert_eq!(session.version(), version);
            assert_eq!(split_weights(&session), weights);
            assert!(!session.has_active_gesture());
        }
    }
}

#[test]
fn equalization_has_a_scale_relative_f64_endpoint_without_suppressing_small_steps() {
    let mut saw_nonzero_residual = false;
    for scale in [1.0e-6, 1.0, 1.0e6] {
        let content = weighted(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::central_tabs([FIRST]), 1.0),
                (DockspaceNode::tabs([SECOND]), 3.0),
                (DockspaceNode::tabs([THIRD]), 7.0),
            ],
        );
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap();
        let mut session = DockspaceSession::from_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            scaled_config(scale),
        )
        .unwrap();
        let measurements = UniformSurfaceMetrics::new(
            LogicalRect::new(0.0, 0.0, 1002.0 * scale, 600.0 * scale).unwrap(),
            LogicalSize::new(64.0 * scale, 48.0 * scale).unwrap(),
            96.0 * scale,
        )
        .unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        frame.measure_surface(SURFACE, measurements).unwrap();
        frame.commit().unwrap();
        let before = session.version();
        assert!(equalize_with_metrics(&mut session, 0, measurements).workspace_changed());
        assert_eq!(
            session.version().revision().get(),
            before.revision().get() + 1
        );

        // At unit scale the central remainder and its neighbor differ by ULPs
        // after balancing 1000 / 11 and 3000 / 11. The second command must still
        // preserve the exact committed graph, not normalize its weights again.
        let (before_extent, after_extent) = inspect_surface(&mut session, |plan| {
            let splitter = plan.splitters().next().unwrap();
            (
                splitter.before_bounds().width(),
                splitter.after_bounds().width(),
            )
        });
        let residual = (before_extent - after_extent).abs();
        saw_nonzero_residual |= residual != 0.0;
        assert!(
            residual <= 1000.0 * scale * f64::EPSILON * 64.0,
            "scale={scale}: residual={residual}"
        );
        let version = session.version();
        let weights = split_weights(&session);
        let committed = session.workspace().clone();
        for _ in 0..3 {
            let report = equalize_with_metrics(&mut session, 0, measurements);
            assert!(!report.workspace_changed(), "scale={scale}: {report:?}");
            assert_eq!(session.version(), version);
            assert_eq!(split_weights(&session), weights);
            assert_eq!(session.workspace(), &committed);
        }

        // This representable displacement is smaller than the equalization
        // endpoint budget. Increment remains a precise ordinary resize action.
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let action = plan
            .prepare_splitter_adjustment(
                plan.splitters().next().unwrap().visual_id(),
                SurfaceSplitterAdjustment::Increment,
            )
            .unwrap();
        frame.submit_surface_action(action).unwrap();
        frame.measure_surface(SURFACE, measurements).unwrap();
        assert!(frame.commit().unwrap().workspace_changed(), "scale={scale}");
        assert_ne!(split_weights(&session), weights);
        assert_eq!(
            session.version().revision().get(),
            version.revision().get() + 1
        );
    }
    assert!(
        saw_nonzero_residual,
        "the fixture must exercise real f64 rounding"
    );
}

#[test]
fn a_large_far_sibling_cannot_suppress_equalization_of_a_small_adjacent_pair() {
    for scale in [1.0e-12, 1.0e-6, 1.0] {
        for far_central in [false, true] {
            let far = if far_central {
                DockspaceNode::central_tabs([THIRD])
            } else {
                DockspaceNode::tabs([THIRD])
            };
            let content = weighted(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::tabs([FIRST]), 1.0),
                    (DockspaceNode::tabs([SECOND]), 2.0),
                    (far, 1.0e14),
                ],
            );
            let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(ROOT, content),
            )])
            .unwrap();
            let mut session = DockspaceSession::from_layout_with_presentation_config(
                layout,
                DockPolicy::default(),
                scaled_config(scale),
            )
            .unwrap();
            let width = 1.0e16 * scale;
            let measurements = UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, width, 600.0 * scale).unwrap(),
                LogicalSize::new(64.0 * scale, 48.0 * scale).unwrap(),
                96.0 * scale,
            )
            .unwrap();
            let mut frame = session.begin_host_frame().unwrap();
            frame.measure_surface(SURFACE, measurements).unwrap();
            frame.commit().unwrap();
            let before = pane_bounds(&mut session);
            let first = bound(&before, FIRST).width();
            let second = bound(&before, SECOND).width();
            let requested = (second - first) * 0.5;
            assert!(requested > 0.0 && requested < width * f64::EPSILON * 64.0);
            let version = session.version();
            assert!(
                equalize_with_metrics(&mut session, 0, measurements).workspace_changed(),
                "scale={scale}, far_central={far_central}"
            );
            assert_eq!(
                session.version().revision().get(),
                version.revision().get() + 1
            );
            let after = pane_bounds(&mut session);
            let expected = (first + second) * 0.5;
            let precision = (first + second) * f64::EPSILON * 64.0;
            for item in [FIRST, SECOND] {
                assert!(
                    (bound(&after, item).width() - expected).abs() <= precision,
                    "scale={scale}, far_central={far_central}, item={item}"
                );
            }
            assert!(
                (bound(&after, THIRD).min().x() - bound(&before, THIRD).min().x()).abs()
                    <= precision,
                "scale={scale}, far_central={far_central}"
            );
            assert_eq!(after.len(), 3);
            let weights = split_weights(&session);
            let version = session.version();
            let committed = session.workspace().clone();
            assert!(!equalize_with_metrics(&mut session, 0, measurements).workspace_changed());
            assert_eq!(session.version(), version);
            assert_eq!(split_weights(&session), weights);
            assert_eq!(session.workspace(), &committed);
        }
    }
}

#[test]
fn equalization_uses_actual_equal_geometry_without_rewriting_constrained_weights() {
    let mut session = measured(
        weighted(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::tabs([FIRST]), 1.0),
                (DockspaceNode::tabs([SECOND]), 3.0),
            ],
        ),
        DockPolicy::default(),
    );
    let mut frame = session.begin_host_frame().unwrap();
    let half = (800.0 - frame.presentation_config().splitter_thickness()) * 0.5;
    frame
        .measure_surface_with(SURFACE, |request| match request {
            SurfaceMeasurementRequest::PaneMinimum {
                item: Some(FIRST), ..
            } => SurfaceMeasurementAnswer::PaneMinimum(LogicalSize::new(half, 48.0).unwrap()),
            request => measure(request),
        })
        .unwrap();
    frame.commit().unwrap();
    let before = pane_bounds(&mut session);
    close(
        bound(&before, FIRST).width(),
        bound(&before, SECOND).width(),
    );
    let weights = split_weights(&session);
    assert_ne!(weights[0], weights[1]);
    let version = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let action = plan
        .prepare_splitter_adjustment(
            plan.splitters().next().unwrap().visual_id(),
            SurfaceSplitterAdjustment::Equalize,
        )
        .unwrap();
    frame.submit_surface_action(action).unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    assert!(!frame.commit().unwrap().workspace_changed());
    assert_eq!(session.version(), version);
    assert_eq!(split_weights(&session), weights);
}

#[test]
fn a_nested_touching_minimum_stops_equalization_before_the_remote_pane_moves() {
    let inner = weighted(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([THIRD]), 1.0),
            (DockspaceNode::central_tabs([FOURTH]), 1.0),
        ],
    );
    let right = weighted(
        DockspaceAxis::Vertical,
        [(DockspaceNode::tabs([SECOND]), 1.0), (inner, 1.0)],
    );
    let mut session = measured(
        weighted(
            DockspaceAxis::Horizontal,
            [(DockspaceNode::tabs([FIRST]), 1.0), (right, 3.0)],
        ),
        DockPolicy::default(),
    );
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface_with(SURFACE, |request| match request {
            SurfaceMeasurementRequest::PaneMinimum {
                item: Some(THIRD), ..
            } => SurfaceMeasurementAnswer::PaneMinimum(LogicalSize::new(240.0, 48.0).unwrap()),
            request => measure(request),
        })
        .unwrap();
    frame.commit().unwrap();
    let before = pane_bounds(&mut session);
    let version = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let outer = plan
        .splitters()
        .find(|splitter| {
            splitter.axis() == DockspaceAxis::Horizontal
                && splitter.before_bounds() == bound(&before, FIRST)
        })
        .unwrap();
    let range = plan.splitter_resize_range(outer.visual_id()).unwrap();
    assert!(range.maximum() < 0.5);
    let action = plan
        .prepare_splitter_adjustment(outer.visual_id(), SurfaceSplitterAdjustment::Equalize)
        .unwrap();
    frame.submit_surface_action(action).unwrap();
    frame
        .measure_surface_with(SURFACE, |request| match request {
            SurfaceMeasurementRequest::PaneMinimum {
                item: Some(THIRD), ..
            } => SurfaceMeasurementAnswer::PaneMinimum(LogicalSize::new(240.0, 48.0).unwrap()),
            request => measure(request),
        })
        .unwrap();
    assert!(frame.commit().unwrap().workspace_changed());
    assert_eq!(
        session.version().revision().get(),
        version.revision().get() + 1
    );
    let after = pane_bounds(&mut session);
    close(bound(&after, THIRD).width(), 240.0);
    close(
        bound(&after, FOURTH).width(),
        bound(&before, FOURTH).width(),
    );
    close(
        bound(&after, FOURTH).min().x(),
        bound(&before, FOURTH).min().x(),
    );
    assert!(bound(&after, FIRST).width() > bound(&before, FIRST).width());
    assert!(bound(&after, FIRST).width() < bound(&after, SECOND).width());
}

#[test]
fn junction_equalization_changes_only_one_axis_and_one_workspace_revision() {
    let column = |first, second| {
        weighted(
            DockspaceAxis::Vertical,
            [
                (DockspaceNode::tabs([first]), 1.0),
                (DockspaceNode::tabs([second]), 3.0),
            ],
        )
    };
    let mut session = measured(
        weighted(
            DockspaceAxis::Horizontal,
            [(column(FIRST, SECOND), 1.0), (column(THIRD, FOURTH), 3.0)],
        ),
        DockPolicy::default(),
    );
    for axis in [DockspaceAxis::Horizontal, DockspaceAxis::Vertical] {
        let before = pane_bounds(&mut session);
        let version = session.version();
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let junction = plan.splitter_junctions().next().unwrap();
        let action = plan
            .prepare_splitter_junction_adjustment(
                junction.visual_id(),
                axis,
                SurfaceSplitterAdjustment::Equalize,
            )
            .unwrap();
        frame.submit_surface_action(action).unwrap();
        frame.measure_surface(SURFACE, metrics()).unwrap();
        assert!(frame.commit().unwrap().workspace_changed());
        assert_eq!(
            session.version().revision().get(),
            version.revision().get() + 1
        );
        let after = pane_bounds(&mut session);
        let (pair, unaffected) = match axis {
            DockspaceAxis::Horizontal => ([FIRST, THIRD], DockspaceAxis::Vertical),
            DockspaceAxis::Vertical => ([FIRST, SECOND], DockspaceAxis::Horizontal),
        };
        close(
            extent(bound(&after, pair[0]), axis),
            extent(bound(&after, pair[1]), axis),
        );
        for item in [FIRST, SECOND, THIRD, FOURTH] {
            close(
                extent(bound(&after, item), unaffected),
                extent(bound(&before, item), unaffected),
            );
        }
    }
}

#[test]
fn equalization_respects_axis_policy_and_discarded_or_stale_actions() {
    let content = || {
        weighted(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::tabs([FIRST]), 1.0),
                (DockspaceNode::tabs([SECOND]), 3.0),
            ],
        )
    };
    let mut policy = DockPolicy::default();
    policy.set_allow_resize_axis(DockspaceAxis::Horizontal, false);
    let mut disabled = measured(content(), policy);
    inspect_surface(&mut disabled, |plan| {
        let splitter = plan.splitters().next().unwrap();
        assert!(
            plan.prepare_splitter_adjustment(
                splitter.visual_id(),
                SurfaceSplitterAdjustment::Equalize
            )
            .is_none()
        );
    });

    let mut session = measured(content(), DockPolicy::default());
    let before = session.workspace().clone();
    let version = session.version();
    {
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let action = plan
            .prepare_splitter_adjustment(
                plan.splitters().next().unwrap().visual_id(),
                SurfaceSplitterAdjustment::Equalize,
            )
            .unwrap();
        frame.submit_surface_action(action).unwrap();
    }
    assert_eq!(session.workspace(), &before);
    assert_eq!(session.version(), version);

    let stale = inspect_surface(&mut session, |plan| {
        plan.prepare_splitter_adjustment(
            plan.splitters().next().unwrap().visual_id(),
            SurfaceSplitterAdjustment::Equalize,
        )
        .unwrap()
    });
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .dock_item_current(SECOND, DockPlacement::After(FIRST))
        .unwrap();
    frame.measure_surface(SURFACE, metrics()).unwrap();
    frame.commit().unwrap();
    let after_merge = session.workspace().clone();
    let version = session.version();
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_surface_action(stale).unwrap();
    frame.measure_surface(SURFACE, metrics()).unwrap();
    let report = frame.commit().unwrap();
    assert!(matches!(
        report.inputs(),
        [HostInputOutcome::StaleRejected { .. }]
    ));
    assert_eq!(session.workspace(), &after_merge);
    assert_eq!(session.version(), version);
    assert_eq!(
        session.view().item(FIRST).unwrap().tabs().items(),
        [FIRST, SECOND]
    );
}

#[test]
fn equalization_after_a_pointer_release_does_not_leave_a_resize_session() {
    let mut session = measured(
        weighted(
            DockspaceAxis::Horizontal,
            [
                (DockspaceNode::tabs([FIRST]), 1.0),
                (DockspaceNode::tabs([SECOND]), 3.0),
            ],
        ),
        DockPolicy::default(),
    );
    present_pointer_surface(&mut session);
    let point = inspect_surface(&mut session, |plan| {
        plan.receiver_for_splitter(plan.splitters().next().unwrap())
            .unwrap()
            .center()
    });
    submit_host_edges(
        &mut session,
        &[
            (
                SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
                Some(point),
            ),
            (
                SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
                Some(point),
            ),
        ],
    );
    assert!(!session.has_active_gesture());
    assert!(equalize(&mut session, 0).workspace_changed());
    assert!(!session.has_active_gesture());
    let after = pane_bounds(&mut session);
    close(bound(&after, FIRST).width(), bound(&after, SECOND).width());
}
