//! Shared six-dot affordance for dragging a window or a tab group.

use egui::{Color32, Rect, Ui, vec2};

pub(super) fn paint(ui: &Ui, bounds: Rect, color: Color32) {
    if !bounds.is_positive() {
        return;
    }
    let spacing = 3.5_f32.min(bounds.width() / 2.0).min(bounds.height() / 3.0);
    let radius = 1.0_f32.min(spacing * 0.3);
    let painter = ui.painter_at(bounds);
    for x in [-0.5, 0.5] {
        for y in [-1.0, 0.0, 1.0] {
            painter.circle_filled(
                bounds.center() + vec2(x * spacing, y * spacing),
                radius,
                color,
            );
        }
    }
}
