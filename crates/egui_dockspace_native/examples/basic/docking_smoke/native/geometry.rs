//! Observe real client geometry after a resize command or UI zoom change.

use super::*;

#[derive(Debug)]
pub(super) struct ChangedGeometry {
    kind: Interruption,
    root_physical_size: egui::Vec2,
    child_physical_size: egui::Vec2,
    root_scale: f32,
    child_scale: f32,
    pass: u64,
}

impl ChangedGeometry {
    pub(super) fn request(
        kind: Interruption,
        context: &egui::Context,
        observed: &EditorObservations,
        child: egui::ViewportId,
    ) -> Self {
        let root = observed.root();
        let child = &observed.viewports[&child];
        let change = Self {
            kind,
            root_physical_size: root.client_rect.unwrap().size() * root.scale,
            child_physical_size: child.client_rect.unwrap().size() * child.scale,
            root_scale: root.scale,
            child_scale: child.scale,
            pass: context.cumulative_pass_nr(),
        };
        assert!((context.zoom_factor() - 1.0).abs() < 0.001);
        match kind {
            Interruption::Resize => {
                assert!(
                    !near(root.client_rect.unwrap().size(), egui::vec2(1100.0, 740.0)),
                    "the resize must change the actual source client"
                );
                context.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::InnerSize(egui::vec2(1100.0, 740.0)),
                );
            }
            Interruption::Zoom => context.set_zoom_factor(1.25),
            _ => unreachable!(),
        }
        change
    }

    pub(super) fn observed(
        &self,
        context: &egui::Context,
        observed: &EditorObservations,
        child: egui::ViewportId,
    ) -> bool {
        // The probe records scale in output_hook, after this journey advances.
        // Requiring a later pass and matching scale/size ensures the changed
        // input and at least one changed output have both reached the real UI.
        if context.cumulative_pass_nr() < self.pass + 2 {
            return false;
        }
        let root = observed.root();
        let child = &observed.viewports[&child];
        let (Some(root_rect), Some(child_rect)) = (root.client_rect, child.client_rect) else {
            return false;
        };
        match self.kind {
            Interruption::Resize => {
                near(root_rect.size(), egui::vec2(1100.0, 740.0))
                    && near(context.viewport_rect().size(), root_rect.size())
                    && (root.scale - self.root_scale).abs() < 0.001
                    && near(child_rect.size() * child.scale, self.child_physical_size)
            }
            Interruption::Zoom => {
                (context.zoom_factor() - 1.25).abs() < 0.001
                    && (root.scale - self.root_scale * 1.25).abs() < 0.001
                    && (child.scale - self.child_scale * 1.25).abs() < 0.001
                    && near(root_rect.size() * root.scale, self.root_physical_size)
                    && near(child_rect.size() * child.scale, self.child_physical_size)
                    && near(context.viewport_rect().size(), root_rect.size())
            }
            _ => unreachable!(),
        }
    }
}

fn near(actual: egui::Vec2, expected: egui::Vec2) -> bool {
    (actual - expected).abs().max_elem() <= 1.0
}
