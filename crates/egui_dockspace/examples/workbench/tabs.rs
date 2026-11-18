//! Optional application chrome. Dockspace still owns docking and close input.

use egui::{Color32, CornerRadius, Id, Rect, Stroke, Ui, pos2, vec2};
use egui_dockspace::{
    DockMenu, DockMenuAction, DockMenuButton, DockMenuButtonKind, DockMenuItem, DockMenuTarget,
    DockspaceDropEligibility, DragPreview, DropGuide, Tab, TabBar, TabBarSlot, TabClose,
    TabVisuals,
};

use super::DemoPanes;

const ACCENTS: [Color32; 3] = [
    Color32::from_rgb(48, 112, 172),
    Color32::from_rgb(35, 125, 105),
    Color32::from_rgb(130, 78, 160),
];

fn tint_action() -> Id {
    Id::unique("workbench-cycle-tab-accent")
}

pub(super) fn customize_menu(panes: &DemoPanes, menu: &mut DockMenu) {
    if panes.custom_tabs
        && matches!(
            menu.context().target,
            DockMenuTarget::Tab(_) | DockMenuTarget::TabGroup(_)
        )
    {
        menu.items
            .insert(0, DockMenuItem::custom(tint_action(), "Cycle tab accent"));
        menu.items.insert(1, DockMenuItem::separator());
    }
}

pub(super) fn on_menu_action(panes: &mut DemoPanes, action: DockMenuAction) {
    if action.id == tint_action() {
        cycle_accent(panes);
    }
}

fn cycle_accent(panes: &mut DemoPanes) {
    panes.tab_accent = (panes.tab_accent + 1) % ACCENTS.len();
}

pub(super) fn menu_button_ui(panes: &DemoPanes, ui: &Ui, button: &DockMenuButton<'_>) {
    if !panes.custom_tabs {
        button.paint(ui);
        return;
    }
    let accent = ACCENTS[panes.tab_accent];
    let mut appearance = *button;
    appearance.corner_radius = CornerRadius::same(5);
    if button.enabled {
        appearance.icon_color = accent;
        if button.open {
            appearance.fill = ui.visuals().window_fill().lerp_to_gamma(accent, 0.18);
        }
    }
    appearance.paint_background(ui);
    if button.kind == DockMenuButtonKind::TabGroup {
        let spacing = button.rect.width().min(button.rect.height()) * 0.18;
        for column in [-1.0, 0.0, 1.0] {
            ui.painter().circle_filled(
                button.rect.center() + vec2(column * spacing, 0.0),
                1.2,
                appearance.icon_color,
            );
        }
    } else {
        appearance.paint_icon(ui);
    }
}

pub(super) fn tab_ui(panes: &DemoPanes, ui: &Ui, tab: &Tab<'_>) -> TabVisuals {
    if !panes.custom_tabs {
        return tab.paint(ui);
    }
    let accent = ACCENTS[panes.tab_accent];
    let mut custom = tab.clone();
    custom.visuals.corner_radius = CornerRadius {
        nw: 6,
        ne: 6,
        sw: 0,
        se: 0,
    };
    if tab.selected {
        custom.visuals.stroke = Stroke::new(1.0, accent);
    }
    custom.paint_background(ui.painter());
    paint_icon(ui, tab.content_rect, tab.content_clip_rect, accent);
    custom.paint_title(
        ui,
        Rect::from_min_max(tab.content_rect.min + vec2(18.0, 0.0), tab.content_rect.max),
    );
    custom.visuals
}

pub(super) fn tab_close_ui(panes: &DemoPanes, ui: &Ui, close: &TabClose<'_>) {
    if panes.custom_tabs && close.response.hovered() {
        ui.painter()
            .rect_filled(close.rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    close.paint(ui);
}

pub(super) fn drag_preview_ui(panes: &DemoPanes, ui: &Ui, preview: &DragPreview) {
    if !panes.custom_tabs || preview.title.is_none() {
        preview.paint(ui);
        return;
    }
    preview.paint_background(ui);
    paint_icon(
        ui,
        preview.title_rect,
        ui.clip_rect(),
        ACCENTS[panes.tab_accent],
    );
    preview.paint_title(
        ui,
        Rect::from_min_max(
            preview.title_rect.min + vec2(18.0, 0.0),
            preview.title_rect.max,
        ),
    );
}

pub(super) fn drop_guide_ui(panes: &DemoPanes, ui: &Ui, guide: &DropGuide) {
    if !panes.custom_tabs || guide.eligibility == DockspaceDropEligibility::Rejected {
        guide.paint(ui);
        return;
    }
    let emphasis = ui.ctx().animate_bool_with_time_and_easing(
        guide.id.with("accent"),
        guide.active,
        ui.style().animation_time.min(0.12),
        egui::emath::easing::cubic_in_out,
    );
    let accent = ACCENTS[panes.tab_accent];
    let mut appearance = *guide;
    appearance.fill = ui
        .visuals()
        .window_fill()
        .lerp_to_gamma(accent, 0.12 + 0.22 * emphasis);
    // Selection feedback is immediate; only the decorative fill transitions.
    appearance.stroke = Stroke::new(if guide.active { 2.0 } else { 1.0 }, accent);
    appearance.cue_color = accent;
    // The circular background replaces the default shape; hit geometry stays fixed.
    ui.painter().circle(
        guide.rect.center(),
        guide.rect.width().min(guide.rect.height()) * 0.5 - 1.0,
        appearance.fill,
        appearance.stroke,
    );
    appearance.paint_cue(ui);
}

fn paint_icon(ui: &Ui, content: Rect, clip: Rect, color: Color32) {
    let icon = Rect::from_center_size(
        pos2(content.left() + 5.0, content.center().y),
        vec2(8.0, 10.0),
    );
    ui.painter()
        .with_clip_rect(clip)
        .rect_filled(icon, 2.0, color);
}

pub(super) fn tab_bar_ui(panes: &mut DemoPanes, ui: &mut Ui, bar: TabBar<'_>, slot: TabBarSlot) {
    ui.spacing_mut().button_padding = vec2(4.0, 2.0);
    ui.add_space(2.0);
    match slot {
        TabBarSlot::Leading => {
            ui.small(bar.tabs().len().to_string());
        }
        TabBarSlot::Trailing => {
            if ui.small_button("Tint").clicked() {
                cycle_accent(panes);
            }
        }
        TabBarSlot::AfterTabs => {
            ui.menu_button("?", |ui| {
                ui.label("Drag tabs to split or merge panes.");
                ui.label("Hold Shift while dragging to float a tab without docking.");
                ui.label("Use Tint to change the tab accent.");
            });
        }
    }
}
