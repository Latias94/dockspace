//! Theme-relative tab recipes, separate from application visual overrides.

use egui::{Color32, CornerRadius, Stroke, Visuals};

use super::{DockStyle, TabCloseVisibility, TabGroupGripVisibility};

/// Theme-relative tab appearance with compact reference geometry.
///
/// These recipes keep dockspace's interaction and focus semantics. They do not
/// replace the egui theme or reproduce another library's complete styling API.
/// Use [`DockStyle::apply_tab_preset`] to apply the complete recipe, including
/// rounding and tab height. Assigning [`DockStyle::tab_preset`] changes the
/// palette while retaining the current geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabStylePreset {
    /// Flat tabs with egui interaction colors and no default outlines.
    Dockspace,
    /// Inset rounded tabs with a quiet fill and a fine selected outline.
    #[default]
    Rounded,
    /// Square tabs whose selected outline opens into the pane below.
    EguiTiles,
}

impl DockStyle {
    /// Applies a tab recipe's appearance and compact reference geometry.
    ///
    /// Only tab metrics, background insets, corners and close/grip visibility change.
    /// Explicit [`super::DockVisualOverrides`] and non-tab settings are retained.
    /// Colors continue to follow the current Ui's theme. Metrics are applied
    /// once: subsequent field edits are retained until this is called again.
    /// Use `tab_preset` directly to change only the palette, preserving metrics.
    pub fn apply_tab_preset(&mut self, preset: TabStylePreset) {
        let defaults = Self::default();
        self.tab_preset = preset;
        self.tab_close_visibility = TabCloseVisibility::SelectedOrHovered;
        self.tab_group_grip_visibility = TabGroupGripVisibility::Always;
        self.tab_connect_to_pane = preset == TabStylePreset::EguiTiles;
        self.tab_corner_radius = if preset == TabStylePreset::Rounded {
            CornerRadius::same(6)
        } else {
            CornerRadius::ZERO
        };
        self.tab_background_inset = if preset == TabStylePreset::Rounded {
            egui::vec2(2.0, 2.0)
        } else {
            egui::Vec2::ZERO
        };
        self.tab_bar_height = defaults.tab_bar_height;
        self.tab_group_grip_extent = defaults.tab_group_grip_extent;
        self.tab_horizontal_padding = defaults.tab_horizontal_padding;
        self.tab_min_width = defaults.tab_min_width;
        self.tab_close_size = defaults.tab_close_size;
        self.tab_close_icon_size = defaults.tab_close_icon_size;
        self.tab_max_width = self.tab_max_width.max(self.tab_min_width);
    }
}

pub(super) struct TabPalette {
    pub fill: Color32,
    pub hover_fill: Color32,
    pub active_fill: Color32,
    pub active_stroke: Stroke,
    pub bar_stroke: Stroke,
    pub text_color: Color32,
    pub active_text_color: Color32,
}

impl TabStylePreset {
    pub(super) fn palette(self, visuals: &Visuals, bar_fill: Color32) -> TabPalette {
        let mut palette = TabPalette {
            fill: Color32::TRANSPARENT,
            hover_fill: visuals.widgets.hovered.weak_bg_fill,
            active_fill: visuals.panel_fill,
            active_stroke: Stroke::NONE,
            bar_stroke: Stroke::NONE,
            text_color: visuals.widgets.inactive.fg_stroke.color,
            active_text_color: visuals.widgets.active.fg_stroke.color,
        };
        match self {
            Self::Dockspace => {}
            Self::Rounded => {
                let accent = visuals.selection.bg_fill;
                palette.active_fill = visuals.panel_fill.lerp_to_gamma(accent, 0.2);
                palette.hover_fill = bar_fill
                    .lerp_to_gamma(visuals.panel_fill, 0.6)
                    .lerp_to_gamma(accent, 0.08);
                palette.active_stroke = Stroke::new(
                    1.0,
                    accent.lerp_to_gamma(visuals.selection.stroke.color, 0.55),
                );
                palette.active_text_color = visuals.strong_text_color();
            }
            Self::EguiTiles => {
                palette.hover_fill = Color32::TRANSPARENT;
                palette.active_stroke = visuals.selection.stroke;
                palette.bar_stroke =
                    Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color);
                palette.text_color = visuals.widgets.noninteractive.fg_stroke.color;
            }
        }
        palette
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DockFrameVisuals;

    #[test]
    fn switching_tab_recipes_retains_application_overrides_and_non_tab_configuration() {
        let mut style = DockStyle {
            splitter_thickness: 3.0,
            minimum_pane_size: egui::vec2(200.0, 100.0),
            floating_title_height: 36.0,
            ..Default::default()
        };
        style.visuals.tab_active_fill = Some(Color32::MAGENTA);
        style.visuals.tab_selected_overline = Some(Stroke::NONE);
        style.visuals.tab_bar_stroke = Some(Stroke::NONE);
        style.visuals.floating = DockFrameVisuals {
            fill: Some(Color32::RED),
            ..Default::default()
        };
        let overrides = style.visuals.clone();
        let original = style.clone();
        for preset in [
            TabStylePreset::Rounded,
            TabStylePreset::EguiTiles,
            TabStylePreset::Dockspace,
        ] {
            style.apply_tab_preset(preset);
            assert_eq!(style.validate(), Ok(()));
            assert_eq!(style.visuals, overrides);
            assert_eq!(style.splitter_thickness, original.splitter_thickness);
            assert_eq!(style.minimum_pane_size, original.minimum_pane_size);
            assert_eq!(style.floating_title_height, original.floating_title_height);
            for theme in [Visuals::light(), Visuals::dark()] {
                let resolved = style.resolved_visuals(&theme);
                assert_eq!(resolved.tab_active_fill, Color32::MAGENTA);
                assert_eq!(resolved.tab_selected_overline, Stroke::NONE);
                assert_eq!(resolved.tab_bar_stroke, Stroke::NONE);
                assert_eq!(
                    resolved.splitter_color,
                    DockStyle::default().resolved_visuals(&theme).splitter_color
                );
            }
        }
    }

    #[test]
    fn palette_only_selection_retains_font_derived_geometry() {
        let mut style = DockStyle {
            tab_preset: TabStylePreset::Dockspace,
            tab_bar_height: 42.0,
            tab_horizontal_padding: 12.0,
            ..Default::default()
        };
        let geometry = style.presentation_config().unwrap();
        style.tab_preset = TabStylePreset::Rounded;
        assert_eq!(style.presentation_config().unwrap(), geometry);
        let light = style.resolved_visuals(&Visuals::light());
        let dark = style.resolved_visuals(&Visuals::dark());
        assert_ne!(light.tab_active_fill, dark.tab_active_fill);
        assert_eq!(style.tab_bar_height, 42.0);
        assert_eq!(style.tab_horizontal_padding, 12.0);
    }
}
