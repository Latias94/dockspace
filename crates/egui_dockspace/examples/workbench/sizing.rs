use egui::Context;
use egui_dockspace::{ItemId, PaneSizing};

use super::{CONSOLE, DemoPanes, INSPECTOR, OUTLINE};

const INSPECTOR_MAX_WIDTH: f64 = 240.0;
const CONSOLE_HEIGHT: f64 = 120.0;
const OUTLINE_PREFERRED_WIDTH: f64 = 220.0;

pub(super) fn controls(ui: &mut egui::Ui, enabled: &mut bool) {
    ui.menu_button("Sizing", |ui| {
        if ui.checkbox(enabled, "Pane sizing demo").changed() {
            ui.close();
        }
        ui.separator();
        ui.label(format!(
            "Inspector: maximum content width {INSPECTOR_MAX_WIDTH} pt."
        ));
        ui.label(format!(
            "Console: fixed content height {CONSOLE_HEIGHT} pt."
        ));
        ui.label("Select History to compare normal tool sizing.");
        ui.separator();
        ui.label(format!(
            "Outline: preferred content width {OUTLINE_PREFERRED_WIDTH} pt."
        ));
        ui.label("Double-click the Outline / Editor divider to restore Preferred.");
        ui.weak("Turn the demo off to return to normal pane sizing.");
    });
}

pub(super) fn apply_mode(context: &Context, panes: &mut DemoPanes, enabled: bool) {
    if panes.sizing_demo != enabled {
        panes.sizing_demo = enabled;
        context.request_repaint();
    }
}

pub(super) fn pane_sizing(item: ItemId, enabled: bool) -> PaneSizing {
    let sizing = PaneSizing::builder().min_width(160.0).min_height(100.0);
    let sizing = if enabled {
        match item {
            INSPECTOR => sizing.max_width(INSPECTOR_MAX_WIDTH),
            CONSOLE => sizing.fixed_height(CONSOLE_HEIGHT),
            OUTLINE => sizing.preferred_width(OUTLINE_PREFERRED_WIDTH),
            _ => sizing,
        }
    } else {
        sizing
    };
    sizing
        .build()
        .expect("the workbench content sizes are valid")
}
