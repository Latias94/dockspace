//! On-demand resize limits from the same touching-frontier solver as gestures.

use crate::graph::Axis;
use crate::interaction::{ResizeAdjustment, ResizeDeltaInterval};
use crate::presentation::scene::{SplitterResizeTarget, SplitterSceneId};

use super::super::SurfaceSplitterAdjustment;
use super::{DockspaceVisualId, SurfacePaintPlan, VisualIdentity};

/// Current and reachable sizes of the pane immediately before a splitter.
///
/// Values are fractions of the two adjacent panes' combined extent, excluding
/// their separator. Limits include all nested panes touching the moving edge,
/// not just the adjacent panes' own minimum sizes. They describe this exact
/// candidate and grant no input or presentation authority.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceSplitterRange {
    value: f64,
    minimum: f64,
    maximum: f64,
}

impl SurfaceSplitterRange {
    /// Returns the current fraction occupied by the preceding pane.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    /// Returns the smallest reachable fraction under the full layout constraints.
    #[must_use]
    pub const fn minimum(self) -> f64 {
        self.minimum
    }

    /// Returns the largest reachable fraction under the full layout constraints.
    #[must_use]
    pub const fn maximum(self) -> f64 {
        self.maximum
    }
}

impl SurfacePaintPlan<'_> {
    /// Resolves the exact reachable range for a splitter in this candidate.
    ///
    /// Returns `None` when resizing is unavailable or the layout cannot prove
    /// its limits. This walks the root's touching layout and allocates temporary
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
        let limits = self.candidate.splitter_resize_delta_interval(
            self.plan,
            SplitterResizeTarget::Handle(id),
            record.axis(),
        )?;
        let before = *record.child_extents().get(id.index)?;
        let after = *record.child_extents().get(record.after_index())?;
        let total = before + after;
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
        let Some(second) = adjustments.next() else {
            return self
                .splitter_adjustment_delta(target, axis, first)
                .map(ResizeAdjustment::new);
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
                SurfaceSplitterAdjustment::Equalize => {
                    self.splitter_equalization_delta(target, axis, limits)?
                }
            }
            .clamp(limits.minimum, limits.maximum);
            sequence.record(delta);
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
            | SurfaceSplitterAdjustment::Equalize => {
                let limits = self
                    .candidate
                    .splitter_resize_delta_interval(self.plan, target, axis)?;
                if adjustment == SurfaceSplitterAdjustment::Minimum {
                    Some(limits.minimum)
                } else if adjustment == SurfaceSplitterAdjustment::Maximum {
                    Some(limits.maximum)
                } else {
                    self.splitter_equalization_delta(target, axis, limits)
                }
            }
        }
    }

    fn splitter_equalization_delta(
        self,
        target: SplitterResizeTarget,
        axis: Axis,
        limits: ResizeDeltaInterval,
    ) -> Option<f64> {
        let equalize = |id: SplitterSceneId| {
            let record = self.plan.splitter_record(id)?;
            let before = *record.child_extents().get(id.index)?;
            let after = *record.child_extents().get(record.after_index())?;
            let delta = (after - before) * 0.5;
            let scale = if record
                .central_index()
                .is_some_and(|central| central == id.index || central == record.after_index())
            {
                record.child_extents().iter().sum::<f64>()
            } else {
                before + after
            };
            if !delta.is_finite() || !scale.is_finite() {
                return None;
            }
            let delta = delta.clamp(limits.minimum, limits.maximum);
            // Only a central child in this pair depends on the whole split's
            // remainder. Unrelated large siblings must not suppress a locally
            // representable adjustment. Ordinary displacements use no endpoint.
            let precision = scale * f64::EPSILON * 64.0;
            Some(if delta.abs() <= precision { 0.0 } else { delta })
        };
        match target {
            SplitterResizeTarget::Handle(id) => equalize(id),
            SplitterResizeTarget::Junction(junction) => {
                let mut displacement = None;
                for id in junction.splitters() {
                    if self.plan.splitter_record(id)?.axis() != axis {
                        continue;
                    }
                    let delta = equalize(id)?;
                    if displacement.is_some_and(|previous| previous != delta) {
                        return None;
                    }
                    displacement = Some(delta);
                }
                displacement
            }
        }
    }
}
