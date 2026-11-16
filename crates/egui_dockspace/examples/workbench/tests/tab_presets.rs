//! Real toolbar selection applies complete tab recipes between presented runs.

use super::*;
use egui::accesskit::Role;
use egui_dockspace::TabStylePreset;

#[test]
fn selecting_tab_presets_updates_painted_corners_and_retains_application_overrides() {
    let context = editor::new_context();
    context.enable_accesskit();
    let fill = egui::Color32::from_rgb(221, 83, 171);
    let stroke = egui::Stroke::new(2.0, egui::Color32::RED);
    let mut style = editor::dock_style();
    style.visuals.tab_active_fill = Some(fill);
    style.visuals.tab_active_stroke = Some(stroke);
    style.splitter_thickness = 3.0;
    let mut dock = Dockspace::builder("workbench-tab-presets", demo_layout())
        .style(style)
        .build()
        .unwrap();
    let mut panes = DemoPanes::new();
    let _ = workbench_frame(&context, &mut dock, &mut panes, Vec::new());
    let mut output = workbench_frame(&context, &mut dock, &mut panes, Vec::new());

    for (preset, label, radius, inset) in [
        (
            TabStylePreset::EguiTiles,
            "egui_tiles",
            egui::CornerRadius::ZERO,
            egui::Vec2::ZERO,
        ),
        (
            TabStylePreset::Rounded,
            "Rounded",
            egui::CornerRadius::same(6),
            egui::vec2(2.0, 2.0),
        ),
        (
            TabStylePreset::Dockspace,
            "Dockspace",
            egui::CornerRadius::ZERO,
            egui::Vec2::ZERO,
        ),
    ] {
        let bounds = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.role() == Role::ComboBox)
                    .then(|| node.bounds())
                    .flatten()
            })
            .expect("the real toolbar exposes its preset selector");
        let selector = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        for pressed in [true, false] {
            let _ = workbench_frame(
                &context,
                &mut dock,
                &mut panes,
                vec![Event::PointerMoved(selector), button(selector, pressed)],
            );
        }
        let menu = workbench_frame(&context, &mut dock, &mut panes, Vec::new());
        let option = semantic_rect(&menu, Role::Button, label).center();
        for pressed in [true, false] {
            let _ = workbench_frame(
                &context,
                &mut dock,
                &mut panes,
                vec![Event::PointerMoved(option), button(option, pressed)],
            );
        }
        assert_eq!(panes.tab_preset, preset);
        let _ = workbench_frame(&context, &mut dock, &mut panes, Vec::new());
        output = workbench_frame(&context, &mut dock, &mut panes, Vec::new());

        assert_eq!(dock.style().tab_preset, preset);
        assert_eq!(dock.style().tab_corner_radius, radius);
        assert_eq!(dock.style().tab_background_inset, inset);
        assert_eq!(dock.style().visuals.tab_active_fill, Some(fill));
        assert_eq!(dock.style().visuals.tab_active_stroke, Some(stroke));
        assert_eq!(dock.style().splitter_thickness, 3.0);
        let tab = semantic_rect(&output, Role::Tab, "Editor");
        assert!(
            output.shapes.iter().any(|shape| {
                let egui::Shape::Rect(background) = &shape.shape else {
                    return false;
                };
                let visible = background.rect.intersect(shape.clip_rect);
                background.fill == fill
                    && visible.contains(tab.center())
                    && background.corner_radius == radius
                    && background.stroke == stroke
                    && visible.height() == dock.style().tab_bar_height - inset.y * 2.0
            }),
            "{preset:?} must change the actual painted tab, retaining explicit visual overrides",
        );
    }
}
