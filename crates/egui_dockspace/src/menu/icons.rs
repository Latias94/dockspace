//! Distinct scope cues for tab selection, group actions and complete layouts.

use egui::{Color32, Painter, Rect, Stroke, StrokeKind, vec2};

pub(super) fn group_actions(painter: &Painter, bounds: Rect, color: Color32) {
    let spacing = bounds.width().min(bounds.height()) * 0.17;
    for row in [-1.0, 0.0, 1.0] {
        painter.circle_filled(bounds.center() + vec2(0.0, row * spacing), 1.2, color);
    }
}

pub(super) fn layout_actions(painter: &Painter, bounds: Rect, color: Color32) {
    let icon = Rect::from_center_size(bounds.center(), bounds.size() * 0.57);
    painter.rect_stroke(icon, 2.0, Stroke::new(1.0, color), StrokeKind::Inside);
    chevron(painter, icon.shrink(3.0), color);
}

pub(super) fn tab_list(painter: &Painter, bounds: Rect, color: Color32) {
    let center = bounds.center();
    let extent = bounds.width().min(bounds.height()) * 0.23;
    for offset in [-0.8, 0.0, 0.8] {
        let y = center.y + offset * extent;
        painter.line_segment(
            [egui::pos2(center.x - extent, y), egui::pos2(center.x, y)],
            Stroke::new(1.2, color),
        );
    }
    let cue = Rect::from_center_size(center + vec2(extent, 0.0), vec2(extent, extent));
    chevron(painter, cue, color);
}

fn chevron(painter: &Painter, bounds: Rect, color: Color32) {
    let center = bounds.center();
    let half = bounds.width().min(bounds.height()) * 0.4;
    painter.line_segment(
        [
            center + vec2(-half, -half * 0.5),
            center + vec2(0.0, half * 0.5),
        ],
        Stroke::new(1.2, color),
    );
    painter.line_segment(
        [
            center + vec2(0.0, half * 0.5),
            center + vec2(half, -half * 0.5),
        ],
        Stroke::new(1.2, color),
    );
}
