//! Optional real-seat source hover and press handshake for the Wayland driver.

use eframe::egui;

pub(super) struct SourcePress {
    source: egui::Pos2,
    hovered: bool,
    dragging: bool,
}

impl SourcePress {
    pub(super) fn from_env(physical: egui::Pos2, scale: f32) -> Option<Self> {
        if std::env::var("EGUI_DOCKSPACE_WAYLAND_INPUT").as_deref() != Ok("1") {
            return None;
        }
        eprintln!("editor-wayland-scale: {scale}");
        Some(Self {
            source: physical / scale,
            hovered: false,
            dragging: false,
        })
    }

    pub(super) fn advance(&mut self, context: &egui::Context) {
        if self.dragging {
            return;
        }
        context.input(|input| {
            if !self.hovered
                && input.focused
                && !input.pointer.any_down()
                && input
                    .pointer
                    .hover_pos()
                    .is_some_and(|point| point.distance(self.source) <= 1.0)
            {
                self.hovered = true;
                eprintln!("editor-wayland-press:");
            } else if self.hovered
                && input.pointer.primary_down()
                && input
                    .pointer
                    .press_origin()
                    .is_some_and(|point| point.distance(self.source) <= 1.0)
            {
                self.dragging = true;
                eprintln!("editor-wayland-drag:");
            }
        });
    }

    pub(super) fn dragging(&self) -> bool {
        self.dragging
    }
}
