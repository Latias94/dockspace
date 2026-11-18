//! On-demand resize limits from the same touching-frontier solver as gestures.

use crate::engine::SplitterReset;
use crate::graph::Axis;
use crate::interaction::{ResizeAdjustment, ResizeDeltaInterval};
use crate::presentation::scene::SplitterResizeTarget;

use super::super::SurfaceSplitterAdjustment;
use super::{DockspaceVisualId, SurfacePaintPlan, VisualIdentity};

/// Current and reachable position of a boundary within its track owner.
///
/// Values are fractions of the owner's combined visible track extents, excluding
/// separators. This remains meaningful when outward propagation moves more than
/// the adjacent pair. Limits include all affected nested content. They describe
/// this exact candidate and grant no input or presentation authority.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSplitterRange {
    value: f64,
    minimum: f64,
    maximum: f64,
}

impl SurfaceSplitterRange {
    /// Returns the current fraction preceding this boundary.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    /// Returns the smallest proven fraction within the frozen resize interval.
    #[must_use]
    pub const fn minimum(self) -> f64 {
        self.minimum
    }

    /// Returns the largest proven fraction within the frozen resize interval.
    #[must_use]
    pub const fn maximum(self) -> f64 {
        self.maximum
    }
}

impl SurfacePaintPlan<'_> {
    /// Returns the exact divider or junction that owns a pointer resize on this surface.
    #[must_use]
    pub fn active_splitter_resize(self) -> Option<DockspaceVisualId> {
        let resize = self.candidate.interaction().active_resize_view()?;
        if resize.surface() != self.surface {
            return None;
        }
        let visual = match resize.target() {
            SplitterResizeTarget::Handle(id) => VisualIdentity::Splitter(id),
            SplitterResizeTarget::Junction(id) => VisualIdentity::SplitterJunction(id),
        };
        Some(DockspaceVisualId(visual))
    }

    /// Resolves a proven resize interval for a splitter in this candidate.
    ///
    /// Returns `None` when resizing is unavailable or the layout cannot prove
    /// its limits. Auto endpoints must also preserve exact final geometry; an
    /// unprovable endpoint returns `None` without searching for another bound.
    /// This walks the root's touching layout and allocates temporary
    /// solver state. Query it when numeric accessibility values or application
    /// controls need the range, rather than for ordinary splitter drawing.
    #[must_use]
    pub fn splitter_resize_range(
        self,
        splitter: DockspaceVisualId,
    ) -> Option<SurfaceSplitterRange> {
        let VisualIdentity::Splitter(id) = splitter.0 else {
            return None;
        };
        let record = self
            .plan
            .splitter_record(id)
            .filter(|record| record.operable())?;
        let limits = self
            .candidate
            .checked_splitter_resize_range(self.plan, id)?;
        let before = record.child_extents().get(..=id.index)?.iter().sum::<f64>();
        let total = record.child_extents().iter().sum::<f64>();
        if !total.is_finite() || total <= 0.0 {
            return None;
        }
        Some(SurfaceSplitterRange {
            value: before / total,
            minimum: ((before + limits.minimum) / total).clamp(0.0, 1.0),
            maximum: ((before + limits.maximum) / total).clamp(0.0, 1.0),
        })
    }

    pub(super) fn splitter_adjustment_sequence(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        adjustments: impl IntoIterator<Item = SurfaceSplitterAdjustment>,
    ) -> Option<ResizeAdjustment> {
        let mut adjustments = adjustments.into_iter();
        let first = adjustments.next()?;
        let root = match target {
            SplitterResizeTarget::Handle(id) => id.root,
            SplitterResizeTarget::Junction(id) => id.root(),
        };
        let auto = self
            .plan
            .layout_facts()?
            .root(root)?
            .projection()
            .auto_reflow()
            .is_some();
        let Some(second) = adjustments.next() else {
            let adjustment = self
                .splitter_adjustment_delta(target, axis, first)
                .map(ResizeAdjustment::new)?;
            if auto
                && !self.candidate.splitter_adjustment_geometry_is_available(
                    self.plan, target, axis, adjustment,
                )?
            {
                return None;
            }
            return Some(adjustment);
        };
        let limits = self
            .candidate
            .splitter_resize_delta_interval(self.plan, target, axis)?;
        let mut sequence = ResizeAdjustment::new(0.0);
        for adjustment in [first, second].into_iter().chain(adjustments) {
            let delta = match adjustment {
                SurfaceSplitterAdjustment::Decrement => {
                    sequence.delta() - self.splitter_keyboard_step
                }
                SurfaceSplitterAdjustment::Increment => {
                    sequence.delta() + self.splitter_keyboard_step
                }
                SurfaceSplitterAdjustment::Minimum => limits.minimum,
                SurfaceSplitterAdjustment::Maximum => limits.maximum,
                SurfaceSplitterAdjustment::Equalize | SurfaceSplitterAdjustment::Preferred => self
                    .splitter_restoration_delta(
                        target,
                        axis,
                        limits,
                        adjustment,
                        sequence.delta(),
                    )?,
            }
            .clamp(limits.minimum, limits.maximum);
            let mut trial = sequence;
            trial.record(delta);
            if !auto
                || self
                    .candidate
                    .splitter_adjustment_geometry_is_available(self.plan, target, axis, trial)?
            {
                sequence = trial;
            }
        }
        Some(sequence)
    }

    fn splitter_adjustment_delta(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        adjustment: SurfaceSplitterAdjustment,
    ) -> Option<f64> {
        match adjustment {
            SurfaceSplitterAdjustment::Decrement => Some(-self.splitter_keyboard_step),
            SurfaceSplitterAdjustment::Increment => Some(self.splitter_keyboard_step),
            SurfaceSplitterAdjustment::Minimum
            | SurfaceSplitterAdjustment::Maximum
            | SurfaceSplitterAdjustment::Equalize
            | SurfaceSplitterAdjustment::Preferred => {
                let limits = self
                    .candidate
                    .splitter_resize_delta_interval(self.plan, target, axis)?;
                if adjustment == SurfaceSplitterAdjustment::Minimum {
                    Some(limits.minimum)
                } else if adjustment == SurfaceSplitterAdjustment::Maximum {
                    Some(limits.maximum)
                } else {
                    self.splitter_restoration_delta(target, axis, limits, adjustment, 0.0)
                }
            }
        }
    }

    fn splitter_restoration_delta(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        limits: ResizeDeltaInterval,
        adjustment: SurfaceSplitterAdjustment,
        cumulative_delta: f64,
    ) -> Option<f64> {
        let reset = match adjustment {
            SurfaceSplitterAdjustment::Equalize => SplitterReset::Equalize,
            SurfaceSplitterAdjustment::Preferred => SplitterReset::Preferred,
            _ => return None,
        };
        self.candidate.splitter_restoration_delta(
            self.plan,
            target,
            axis,
            limits,
            reset,
            cumulative_delta,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::geometry::{LogicalRect, PaneSizing};
    use crate::ids::{ItemId, RootId, SurfaceId};
    use crate::model::{
        DockspaceAxis, DockspaceGridShape, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
        DockspaceSurfaceLayout,
    };
    use crate::policy::DockPolicy;
    use crate::runtime::{
        DockPresentationConfig, DockspaceHostFrame, DockspaceRuntimeError, DockspaceSession,
        ResizePropagation, SurfaceMeasurementAnswer, SurfaceMeasurementRequest, TabStripMetrics,
    };

    const ROOT: RootId = RootId::new(1201);
    const SURFACE: SurfaceId = SurfaceId::new(1202);
    const PEER: ItemId = ItemId::new(1);

    fn measure(frame: &mut DockspaceHostFrame<'_>, minimum_height: f64) {
        frame
            .measure_surface_with(SURFACE, |request| {
                Ok::<_, DockspaceRuntimeError>(match request {
                    SurfaceMeasurementRequest::DockBounds { .. }
                    | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                        SurfaceMeasurementAnswer::Bounds(
                            LogicalRect::new(0.0, 0.0, 1004.0, 224.0).unwrap(),
                        )
                    }
                    SurfaceMeasurementRequest::PaneSizing { item, .. } => {
                        SurfaceMeasurementAnswer::PaneSizing(if item == Some(PEER) {
                            PaneSizing::default()
                        } else {
                            PaneSizing::builder()
                                .min_width(20.0)
                                .min_height(minimum_height)
                                .build()
                                .unwrap()
                        })
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
        shape: DockspaceGridShape,
        minimum_height: f64,
        propagation: ResizePropagation,
    ) -> DockspaceSession {
        let grid = DockspaceNode::grid(
            shape,
            (2..=5).map(|item| DockspaceNode::pane(ItemId::new(item))),
        )
        .unwrap();
        let content = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::pane(PEER), grid],
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap();
        let config = DockPresentationConfig::builder()
            .minimum_pane_size(1.0, 1.0)
            .tab_bar_height(20.0)
            .tab_group_grip_extent(20.0)
            .splitter_thickness(4.0)
            .splitter_keyboard_step(20.0)
            .resize_propagation(propagation)
            .build()
            .unwrap();
        let mut session = DockspaceSession::from_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            config,
        )
        .unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        measure(&mut frame, minimum_height);
        frame.commit().unwrap();
        session
    }

    #[test]
    fn auto_range_omits_an_endpoint_that_exact_reflow_cannot_reach() {
        for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
            let mut session = session(DockspaceGridShape::Auto, 120.0, propagation);
            let mut frame = session.begin_host_frame().unwrap();
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let outer = plan
                .splitters()
                .find(|splitter| splitter.before_bounds().width() == 500.0)
                .unwrap();
            assert!(outer.operable());
            assert!(plan.receiver_for_splitter(outer).is_some());
            assert!(plan.splitter_resize_range(outer.visual_id()).is_none());
            let action = plan
                .prepare_splitter_adjustment(
                    outer.visual_id(),
                    SurfaceSplitterAdjustment::Increment,
                )
                .unwrap();
            frame.submit_surface_action(action).unwrap();
            measure(&mut frame, 120.0);
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            for pane in plan.panes() {
                let expected = match pane.item().get() {
                    1 => 520.0,
                    2 => 102.0,
                    _ => 122.0,
                };
                assert!((pane.bounds().width() - expected).abs() < 1.0e-9);
            }
            assert!(frame.commit().unwrap().workspace_changed());
        }
    }

    #[test]
    fn auto_and_fixed_ranges_keep_proven_endpoints_without_searching_new_bounds() {
        let fixed = DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(1).unwrap(),
            columns: NonZeroUsize::new(4).unwrap(),
        };
        for (shape, minimum_height) in [(DockspaceGridShape::Auto, 20.0), (fixed, 120.0)] {
            for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
                let maximum = match propagation {
                    ResizePropagation::Adjacent => 0.602,
                    ResizePropagation::Outward => 0.908,
                };
                let mut session = session(shape, minimum_height, propagation);
                let mut frame = session.begin_host_frame().unwrap();
                let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                let outer = plan
                    .splitters()
                    .find(|splitter| splitter.before_bounds().width() == 500.0)
                    .unwrap();
                let range = plan.splitter_resize_range(outer.visual_id()).unwrap();
                assert_eq!(range.value(), 0.5);
                assert!((range.minimum() - 0.001).abs() < 1.0e-10);
                assert!((range.maximum() - maximum).abs() < 1.0e-10);
                let action = plan
                    .prepare_splitter_adjustment(
                        outer.visual_id(),
                        SurfaceSplitterAdjustment::Maximum,
                    )
                    .unwrap();
                frame.submit_surface_action(action).unwrap();
                measure(&mut frame, minimum_height);
                let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                let peer = plan.panes().find(|pane| pane.item() == PEER).unwrap();
                assert!((peer.bounds().width() - maximum * 1000.0).abs() < 1.0e-9);
                assert!(frame.commit().unwrap().workspace_changed());
            }
        }
    }

    #[test]
    fn auto_adjustment_batches_skip_infeasible_atoms_and_reverse_from_the_last_accepted_delta() {
        let sequence = [
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Decrement,
        ];
        let mut expected: Option<Vec<(ItemId, LogicalRect)>> = None;
        for batched in [false, true] {
            let mut session = session(DockspaceGridShape::Auto, 120.0, ResizePropagation::Adjacent);
            let batches = if batched {
                vec![sequence.as_slice()]
            } else {
                sequence.iter().map(std::slice::from_ref).collect()
            };
            for (index, adjustments) in batches.into_iter().enumerate() {
                let mut frame = session.begin_host_frame().unwrap();
                let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                let outer = plan
                    .splitters()
                    .filter(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
                    .max_by(|left, right| {
                        left.before_bounds()
                            .width()
                            .total_cmp(&right.before_bounds().width())
                    })
                    .unwrap();
                let action = plan.prepare_splitter_adjustment_sequence(
                    outer.visual_id(),
                    adjustments.iter().copied(),
                );
                if let Some(action) = action {
                    frame.submit_surface_action(action).unwrap();
                    measure(&mut frame, 120.0);
                    assert!(frame.commit().unwrap().workspace_changed());
                } else {
                    assert!(
                        !batched && index == 4,
                        "only the fifth increment crosses the shape cut"
                    );
                }
            }
            let mut frame = session.begin_host_frame().unwrap();
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            let actual = plan
                .panes()
                .map(|pane| (pane.item(), pane.bounds()))
                .collect::<Vec<_>>();
            for (item, bounds) in &actual {
                let width = match item.get() {
                    1 => 560.0,
                    2 => 62.0,
                    _ => 122.0,
                };
                assert!(
                    (bounds.width() - width).abs() < 1.0e-9,
                    "batched={batched}: {bounds:?}"
                );
                assert_eq!(bounds.height(), 204.0);
            }
            if let Some(expected) = &expected {
                for ((item, actual), (expected_item, expected_bounds)) in
                    actual.iter().zip(expected)
                {
                    assert_eq!(item, expected_item);
                    assert!((actual.x() - expected_bounds.x()).abs() < 1.0e-9);
                    assert!((actual.width() - expected_bounds.width()).abs() < 1.0e-9);
                    assert_eq!(actual.y(), expected_bounds.y());
                    assert_eq!(actual.height(), expected_bounds.height());
                }
            } else {
                expected = Some(actual);
            }
        }
    }
}
