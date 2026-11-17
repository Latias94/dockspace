//! Corner-preserving conversion at the egui/product geometry boundary.

use dockspace::geometry::{LogicalPoint, LogicalRect};
use egui::{Rect, emath::GuiRounding as _, pos2};

pub(crate) fn logical_rect(rect: Rect) -> Result<LogicalRect, ()> {
    if !rect.is_finite() || !rect.size().is_finite() {
        return Err(());
    }
    // Preserve shared corners without reconstructing them from a rounded f32 span.
    LogicalRect::from_min_max(
        LogicalPoint::new(f64::from(rect.min.x), f64::from(rect.min.y)).map_err(|_| ())?,
        LogicalPoint::new(f64::from(rect.max.x), f64::from(rect.max.y)).map_err(|_| ())?,
    )
    .map_err(|_| ())
}

pub(crate) fn egui_rect(rect: LogicalRect) -> Option<Rect> {
    let max = rect.max();
    // Round shared corners directly instead of adding a separately rounded size.
    let rect = Rect::from_min_max(
        pos2(finite_f32(rect.x())?, finite_f32(rect.y())?),
        pos2(finite_f32(max.x())?, finite_f32(max.y())?),
    );
    (rect.is_finite() && rect.size().is_finite()).then_some(rect)
}

/// Fits child UI layout to egui's grid without expanding its semantic region.
/// Paint and receiver bounds retain the core's continuous coordinates.
pub(crate) fn inset_ui_rect(rect: Rect) -> Option<Rect> {
    let min = pos2(-(-rect.min.x).floor_ui(), -(-rect.min.y).floor_ui());
    let aligned = Rect::from_min_max(min, rect.max.floor_ui());
    (aligned.is_finite() && aligned.is_positive()).then_some(aligned)
}

fn finite_f32(value: f64) -> Option<f32> {
    if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
        return None;
    }
    // Egui's coordinate domain is f32. The range check above makes the only
    // unavoidable loss here a precision reduction at the adapter boundary.
    #[allow(clippy::cast_possible_truncation)]
    Some(value as f32)
}

pub(crate) fn accesskit_bounds(rect: Rect) -> egui::accesskit::Rect {
    egui::accesskit::Rect {
        x0: f64::from(rect.min.x),
        y0: f64::from(rect.min.y),
        x1: f64::from(rect.max.x),
        y1: f64::from(rect.max.y),
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;

    use super::*;

    #[test]
    fn logical_rect_preserves_original_corners_and_exact_egui_roundtrips() {
        for rect in [
            Rect::from_min_max(pos2(-16_777_216.0, -16_777_216.0), pos2(1.0, 3.0)),
            Rect::from_min_max(pos2(0.1, -0.3), pos2(0.3, 0.1)),
            Rect::from_min_max(
                pos2(f32::MAX * 0.5, -f32::MAX),
                pos2(f32::MAX, -f32::MAX * 0.5),
            ),
            Rect::from_min_max(pos2(0.25, 0.5), pos2(0.25, 1.5)),
            Rect::ZERO,
        ] {
            let converted = logical_rect(rect).unwrap();
            assert_eq!(
                converted.min(),
                LogicalPoint::new(f64::from(rect.min.x), f64::from(rect.min.y)).unwrap(),
            );
            assert_eq!(
                converted.max(),
                LogicalPoint::new(f64::from(rect.max.x), f64::from(rect.max.y)).unwrap(),
                "converting the f32 span first must not move the far corner: {rect:?}",
            );
            assert_eq!(egui_rect(converted), Some(rect));
        }
    }

    #[test]
    fn logical_rect_preserves_adjacent_shared_edges_on_both_axes() {
        let first = Rect::from_min_max(pos2(-16_777_216.0, -16_777_216.0), pos2(1.0, 3.0));
        let right = Rect::from_min_max(pos2(first.max.x, first.min.y), pos2(5.0, first.max.y));
        let below = Rect::from_min_max(pos2(first.min.x, first.max.y), pos2(first.max.x, 7.0));
        let [first, right, below] = [first, right, below].map(|rect| logical_rect(rect).unwrap());
        assert_eq!(first.max().x(), right.min().x());
        assert_eq!(first.max().y(), below.min().y());
    }

    #[test]
    fn logical_rect_rejects_nonfinite_inverted_and_f32_extent_overflow() {
        for rect in [
            Rect::NOTHING,
            Rect::EVERYTHING,
            Rect::from_min_max(pos2(f32::NAN, 0.0), pos2(1.0, 1.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, f32::NAN)),
            Rect::from_min_max(pos2(2.0, 0.0), pos2(1.0, 1.0)),
            Rect::from_min_max(pos2(0.0, 2.0), pos2(1.0, 1.0)),
            Rect::from_min_max(pos2(-f32::MAX, 0.0), pos2(f32::MAX, 1.0)),
            Rect::from_min_max(pos2(0.0, -f32::MAX), pos2(1.0, f32::MAX)),
        ] {
            assert_eq!(logical_rect(rect), Err(()), "{rect:?}");
        }
    }

    #[test]
    fn egui_rect_rejects_f32_endpoint_and_extent_overflow() {
        let limit = f64::from(f32::MAX);
        for rect in [
            LogicalRect::new(limit, 0.0, limit, 1.0).unwrap(),
            LogicalRect::new(0.0, limit, 1.0, limit).unwrap(),
            LogicalRect::new(-limit, 0.0, limit * 2.0, 1.0).unwrap(),
            LogicalRect::new(0.0, -limit, 1.0, limit * 2.0).unwrap(),
        ] {
            assert_eq!(
                egui_rect(rect),
                None,
                "finite f64 corners and extents must not produce an overflowing egui rectangle: {rect:?}",
            );
        }
    }

    #[test]
    fn egui_rect_rounds_original_corners_once_and_preserves_shared_edges() {
        let min = LogicalPoint::new(16_777_217.0, -16_777_218.0).unwrap();
        let shared = LogicalPoint::new(16_777_218.0, -16_777_217.0).unwrap();
        let max = LogicalPoint::new(16_777_219.0, -16_777_216.0).unwrap();
        let first = egui_rect(LogicalRect::from_min_max(min, shared).unwrap()).unwrap();
        let second = egui_rect(LogicalRect::from_min_max(shared, max).unwrap()).unwrap();
        assert_eq!(first.min, pos2(16_777_216.0, -16_777_218.0));
        assert_eq!(first.max, pos2(16_777_218.0, -16_777_216.0));
        assert_eq!(second.min, first.max);
        assert_eq!(second.max, pos2(16_777_220.0, -16_777_216.0));
        assert_eq!(first.size(), vec2(2.0, 2.0));
        assert_eq!(second.size(), vec2(2.0, 0.0));
    }

    #[test]
    fn egui_rect_preserves_finite_boundaries_and_degenerate_rectangles() {
        let limit = f64::from(f32::MAX);
        for (rect, expected) in [
            (
                LogicalRect::new(limit * 0.5, limit * 0.5, limit * 0.5, limit * 0.5).unwrap(),
                Rect::from_min_max(
                    pos2(f32::MAX * 0.5, f32::MAX * 0.5),
                    pos2(f32::MAX, f32::MAX),
                ),
            ),
            (
                LogicalRect::new(-limit, -limit, limit * 0.5, limit * 0.5).unwrap(),
                Rect::from_min_max(
                    pos2(-f32::MAX, -f32::MAX),
                    pos2(-f32::MAX * 0.5, -f32::MAX * 0.5),
                ),
            ),
            (
                LogicalRect::new(0.25, 0.5, 0.0, 1.0).unwrap(),
                Rect::from_min_max(pos2(0.25, 0.5), pos2(0.25, 1.5)),
            ),
            (LogicalRect::new(0.0, 0.0, 0.0, 0.0).unwrap(), Rect::ZERO),
        ] {
            let converted = egui_rect(rect).unwrap();
            assert_eq!(converted, expected);
            assert!(converted.is_finite());
            assert!(converted.size().is_finite());
        }
    }

    #[test]
    fn child_layout_stays_inside_its_semantic_region_or_is_omitted() {
        for rect in [
            Rect::from_min_max(pos2(-100.1, -80.3), pos2(221.6, 167.6)),
            Rect::from_min_max(pos2(0.031, 0.001), pos2(0.064, 0.063)),
        ] {
            let aligned = inset_ui_rect(rect).unwrap();
            assert!(rect.contains_rect(aligned));
            assert_eq!(aligned, aligned.round_ui());
            assert!(aligned.min.x - rect.min.x < egui::emath::GUI_ROUNDING);
            assert!(aligned.min.y - rect.min.y < egui::emath::GUI_ROUNDING);
            assert!(rect.max.x - aligned.max.x < egui::emath::GUI_ROUNDING);
            assert!(rect.max.y - aligned.max.y < egui::emath::GUI_ROUNDING);
        }

        for rect in [
            Rect::from_min_max(pos2(0.001, 0.001), pos2(0.02, 0.02)),
            Rect::from_min_max(pos2(0.032, 0.001), pos2(0.063, 0.064)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(f32::INFINITY, 1.0)),
        ] {
            assert_eq!(inset_ui_rect(rect), None);
        }
    }
}
