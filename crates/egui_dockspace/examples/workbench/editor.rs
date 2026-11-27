//! Application content and docking layout. No window-system or renderer code.

use std::collections::BTreeMap;

use egui::{Color32, Context, FontId, Frame, Margin, TextStyle, Theme};
use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, DockMenu, DockMenuAction, DockMenuButton, DockStyle,
    Dockspace, DockspaceAxis, DockspaceContainedLayout, DockspaceError, DockspaceLayout,
    DockspaceNode, DockspaceOutput, DockspaceResponse, DockspaceRootLayout, DockspaceSurfaceLayout,
    DragPreview, DropGuide, FloatingPresentationId, ItemId, LogicalRect, PaneResponse, PaneSizing,
    PaneView, RootId, SurfaceId, Tab, TabBar, TabBarId, TabBarLayout, TabBarSlot, TabClose, TabKey,
    TabStylePreset, TabVisuals,
};

#[cfg(feature = "motion")]
mod motion;
mod sizing;
mod tabs;

pub(super) const SURFACE: SurfaceId = SurfaceId::new(1);
pub(super) const MAIN_ROOT: RootId = RootId::new(1);
const FLOATING_ROOT: RootId = RootId::new(2);
const FLOATING: FloatingPresentationId = FloatingPresentationId::new(1);

pub(super) const EDITOR: ItemId = ItemId::new(1);
pub(super) const PREVIEW: ItemId = ItemId::new(2);
pub(super) const OUTLINE: ItemId = ItemId::new(3);
pub(super) const INSPECTOR: ItemId = ItemId::new(4);
pub(super) const CONSOLE: ItemId = ItemId::new(5);
const HISTORY: ItemId = ItemId::new(6);
pub(super) const PROFILER: ItemId = ItemId::new(7);
pub(super) const ASSETS: ItemId = ItemId::new(8);

pub(super) fn new_context() -> Context {
    let context = Context::default();
    context.set_theme(Theme::Light);
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        for text in [TextStyle::Body, TextStyle::Button, TextStyle::Monospace] {
            style.text_styles.get_mut(&text).unwrap().size = 14.0;
        }
    });
    context
}

pub(super) fn dock_style() -> DockStyle {
    DockStyle::default()
}

fn toolbar(
    ui: &mut egui::Ui,
    showcase: bool,
    restore_layout: &mut bool,
    custom_tabs: &mut bool,
    tab_preset: &mut TabStylePreset,
    sizing_demo: &mut bool,
) {
    egui::Panel::top("workbench-toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Dockspace");
            ui.separator();
            *restore_layout |= ui
                .button("Restore layout")
                .on_hover_text("Reopen every pane and keep document edits.")
                .clicked();
            if !showcase {
                ui.menu_button("Help", |ui| {
                    ui.label("Drag a tab into the center of a pane to merge it.");
                    ui.label("Drop near an edge to split. Drag a divider to resize.");
                    ui.label("Hold Shift while dragging to float a tab without docking.");
                    ui.label("Drag a tab to move one pane; drag a bar's dotted grip to move its tab group.");
                    ui.label("The dotted window-title grip moves the entire floating window, including its splits.");
                    ui.label("A group’s three-dot menu floats or closes that group; the window menu acts on the entire layout.");
                    ui.label("Close panes freely; Restore layout keeps your edits.");
                });
                sizing::controls(ui, sizing_demo);
            }
            ui.checkbox(custom_tabs, "Custom tabs");
            egui::ComboBox::from_id_salt("workbench-tab-style")
                .selected_text(preset_name(*tab_preset))
                .width(90.0)
                .show_ui(ui, |ui| {
                    for preset in [TabStylePreset::Dockspace, TabStylePreset::Rounded, TabStylePreset::EguiTiles] {
                        ui.selectable_value(tab_preset, preset, preset_name(preset));
                    }
                })
                .response
                .labelled_by(ui.label("Tab style").id);
            if !showcase {
                ui.separator();
                ui.weak("Shift + drag to float").on_hover_text(
                    "Hold Shift to disable docking while dragging, then release inside this window to float the tab.",
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                if ui.selectable_label(dark, "Dark").clicked() {
                    ui.ctx().set_theme(Theme::Dark);
                }
                if ui.selectable_label(!dark, "Light").clicked() {
                    ui.ctx().set_theme(Theme::Light);
                }
            });
        });
    });
}

fn preset_name(preset: TabStylePreset) -> &'static str {
    match preset {
        TabStylePreset::Dockspace => "Dockspace",
        TabStylePreset::Rounded => "Rounded",
        TabStylePreset::EguiTiles => "egui_tiles",
    }
}

pub(super) fn run_editor(
    dockspace: &mut Dockspace,
    context: &Context,
    input: egui::RawInput,
    panes: &mut DemoPanes,
    restore_layout: &mut bool,
) -> Result<(DockspaceOutput, Option<DockspaceResponse>), DockspaceError> {
    // Apply geometry between settled runs, never from inside a paint callback.
    if panes.tab_preset != dockspace.style().tab_preset {
        let mut style = dockspace.style().clone();
        style.apply_tab_preset(panes.tab_preset);
        dockspace.set_style(style)?;
    }
    let showcase = panes.showcase;
    let mut custom_tabs = panes.custom_tabs;
    let mut tab_preset = panes.tab_preset;
    let mut sizing_demo = panes.sizing_demo;
    #[cfg(feature = "motion")]
    let (mut motion_settings, mut motion_preference) =
        (*dockspace.motion_settings(), dockspace.motion_preference());
    if dockspace.view().surfaces().next().is_none() {
        let output = egui_dockspace::run_ui(context, input, |ui| {
            toolbar(
                ui,
                showcase,
                restore_layout,
                &mut custom_tabs,
                &mut tab_preset,
                &mut sizing_demo,
            );
            ui.centered_and_justified(|ui| {
                ui.label(
                    "All panes are closed. Restore layout to reopen them and keep your edits.",
                );
            });
        });
        panes.custom_tabs = custom_tabs;
        update_tab_preset(context, panes, tab_preset);
        sizing::apply_mode(context, panes, sizing_demo);
        return Ok((output, None));
    }
    let (output, response) =
        dockspace.run_single_surface_with_ui(context, input, SURFACE, panes, |ui| {
            toolbar(
                ui,
                showcase,
                restore_layout,
                &mut custom_tabs,
                &mut tab_preset,
                &mut sizing_demo,
            );
            #[cfg(feature = "motion")]
            if !showcase {
                motion::controls(ui, &mut motion_settings, &mut motion_preference);
            }
        })?;
    panes.custom_tabs = custom_tabs;
    update_tab_preset(context, panes, tab_preset);
    sizing::apply_mode(context, panes, sizing_demo);
    #[cfg(feature = "motion")]
    {
        if *dockspace.motion_settings() != motion_settings
            || dockspace.motion_preference() != motion_preference
        {
            context.request_repaint();
        }
        dockspace.set_motion_settings(motion_settings);
        dockspace.set_motion_preference(motion_preference);
    }
    Ok((output, Some(response)))
}

fn update_tab_preset(context: &Context, panes: &mut DemoPanes, preset: TabStylePreset) {
    if panes.tab_preset != preset {
        panes.tab_preset = preset;
        context.request_repaint();
    }
}

pub(super) fn apply_editor_commands(
    dockspace: &mut Dockspace,
    response: Option<&DockspaceResponse>,
    restore_layout: bool,
) -> Result<bool, DockspaceError> {
    if restore_layout {
        return dockspace
            .reset_layout(demo_layout())
            .map(|mutation| mutation.workspace_changed());
    }
    let mut changed = false;
    // Closing a sample pane retains its application data for Restore layout.
    for request in response
        .into_iter()
        .flat_map(DockspaceResponse::close_request_events)
        .filter(|request| !request.reused())
    {
        let plan = request.plan();
        for item in plan.items() {
            if item.state() == CloseItemDecisionState::Pending {
                changed |= dockspace
                    .resolve_close(plan.request(), item.token(), CloseDecision::Allow)?
                    .mutation()
                    .workspace_changed();
            }
        }
    }
    Ok(changed)
}

pub(super) fn demo_layout() -> DockspaceLayout {
    let top = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([OUTLINE]), 0.24),
            (DockspaceNode::tabs([EDITOR, PREVIEW]), 0.76),
        ],
    )
    .expect("the static top split is valid");
    let bottom = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([CONSOLE, HISTORY]), 0.65),
            (DockspaceNode::tabs([PROFILER, ASSETS]), 0.35),
        ],
    )
    .expect("the static bottom split is valid");
    let main = DockspaceNode::split(DockspaceAxis::Vertical, [(top, 0.7), (bottom, 0.3)])
        .expect("the editor rows are valid");
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(MAIN_ROOT, main),
    )
    .with_contained(DockspaceContainedLayout::new(
        FLOATING,
        DockspaceRootLayout::new(
            FLOATING_ROOT,
            DockspaceNode::tabs([INSPECTOR]).named("Inspector"),
        ),
        LogicalRect::new(940.0, 90.0, 300.0, 340.0).expect("the static Inspector bounds are valid"),
    ))])
    .expect("the workbench layout is valid")
}

pub(super) struct DemoPane {
    title: &'static str,
    pub(super) text: String,
    pub(super) content: Option<egui::Rect>,
    pub(super) editor: Option<egui::Rect>,
    pub(super) scroll_offset: f32,
    pub(super) scroll_bounds: Option<egui::Rect>,
}

pub(super) struct DemoPanes {
    #[allow(dead_code)]
    pub(super) showcase: bool,
    pub(super) custom_tabs: bool,
    pub(super) tab_preset: TabStylePreset,
    sizing_demo: bool,
    pub(super) tab_accent: usize,
    pub(super) panes: BTreeMap<ItemId, DemoPane>,
    selection: usize,
    objects: [SceneObject; 3],
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(super) drop_observations: Option<DemoDropObservations>,
    #[cfg(test)]
    pub(super) tab_widgets: BTreeMap<egui_dockspace::TabKey, (u64, egui::Id)>,
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct DemoDropObservations {
    pub(super) pass: u64,
    #[cfg(test)]
    pub(super) guides: Vec<DropGuide>,
    pub(super) target: Option<egui_dockspace::DockspaceDropTargetFeedback>,
}

struct SceneObject {
    name: &'static str,
    kind: &'static str,
    color: Color32,
    position: [f32; 3],
    visible: bool,
}

const OBJECTS: [&str; 3] = ["Camera", "Light", "Player"];
const OBJECT_KINDS: [&str; 3] = ["Perspective", "Directional", "Character"];
const OBJECT_COLORS: [Color32; 3] = [
    Color32::from_rgb(139, 92, 246),
    Color32::from_rgb(234, 179, 8),
    Color32::from_rgb(56, 152, 236),
];

const INITIAL_EDITOR_CODE: &str = "\
// src/scene/playground.rs

use crate::engine::{Entity, Scene, Vec3};

pub struct Playground {
    scene: Scene,
    elapsed: f32,
}

impl Playground {
    pub fn new() -> Self {
        let mut scene = Scene::new(\"demo\");
        scene.spawn(Entity::camera(
            Vec3::new(-140.0, 60.0, 0.0),
        ));
        scene.spawn(Entity::light(
            Vec3::new(140.0, 60.0, 0.0),
        ));
        scene.spawn(Entity::player(
            Vec3::new(0.0, -60.0, 0.0),
        ));
        Self { scene, elapsed: 0.0 }
    }

    pub fn update(&mut self, dt: f32) {
        self.elapsed += dt;
        self.scene.sync_transforms();
    }
}
";

impl DemoPanes {
    pub(super) fn new() -> Self {
        Self {
            showcase: false,
            custom_tabs: false,
            tab_preset: TabStylePreset::Rounded,
            sizing_demo: false,
            tab_accent: 0,
            panes: [
                (EDITOR, "Editor", INITIAL_EDITOR_CODE),
                (PREVIEW, "Preview", ""),
                (OUTLINE, "Outline", ""),
                (INSPECTOR, "Inspector", "Active player rig with orbit camera and key light."),
                (CONSOLE, "Console", ""),
                (HISTORY, "History", "Loaded playground.scene\nSpawned Camera, Light, Player\nAdjusted Player transform\nSaved scene snapshot"),
                (PROFILER, "Profiler", ""),
                (ASSETS, "Assets", "src/scene/playground.rs\nscenes/playground.scene\nmeshes/player.mesh\nshaders/grid.wgsl\ntextures/albedo.png"),
            ]
            .into_iter()
            .map(|(item, title, text)| (item, DemoPane {
                title,
                text: text.into(),
                content: None,
                editor: None,
                scroll_offset: 0.0,
                scroll_bounds: None,
            }))
            .collect(),
            selection: 2,
            objects: std::array::from_fn(|index| SceneObject {
                name: OBJECTS[index],
                kind: OBJECT_KINDS[index],
                color: OBJECT_COLORS[index],
                position: [[-140.0, 60.0, 0.0], [140.0, 60.0, 0.0], [0.0, -60.0, 0.0]][index],
                visible: true,
            }),
            #[cfg(any(test, target_arch = "wasm32"))]
            drop_observations: cfg!(test).then(DemoDropObservations::default),
            #[cfg(test)]
            tab_widgets: BTreeMap::new(),
        }
    }

    #[cfg(any(test, target_arch = "wasm32"))]
    fn observe_drop_pass(&mut self, ui: &egui::Ui) -> Option<&mut DemoDropObservations> {
        let observation = self.drop_observations.as_mut()?;
        let pass = ui.ctx().cumulative_pass_nr();
        if observation.pass != pass {
            *observation = DemoDropObservations {
                pass,
                ..Default::default()
            };
        }
        Some(observation)
    }
}

impl PaneView for DemoPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        self.panes.get(&item).map(|pane| pane.title.into())
    }

    fn tab_content_width(&self, _key: TabKey, _ui: &egui::Ui, title: &egui::Galley) -> f32 {
        title.size().x + if self.custom_tabs { 18.0 } else { 0.0 }
    }

    fn tab_ui(&mut self, ui: &mut egui::Ui, tab: &Tab<'_>) -> TabVisuals {
        #[cfg(test)]
        self.tab_widgets
            .insert(tab.key, (ui.ctx().cumulative_pass_nr(), tab.response.id));
        tabs::tab_ui(self, ui, tab)
    }

    fn tab_close_ui(&mut self, ui: &egui::Ui, close: &TabClose<'_>) {
        tabs::tab_close_ui(self, ui, close);
    }

    fn menu_button_ui(&mut self, ui: &egui::Ui, button: &DockMenuButton<'_>) {
        tabs::menu_button_ui(self, ui, button);
    }

    fn customize_menu(&self, menu: &mut DockMenu) {
        tabs::customize_menu(self, menu);
    }

    fn on_menu_action(&mut self, action: DockMenuAction) {
        tabs::on_menu_action(self, action);
    }

    fn drag_preview_ui(&mut self, ui: &egui::Ui, preview: &DragPreview) {
        tabs::drag_preview_ui(self, ui, preview);
    }

    fn drop_guide_ui(&mut self, ui: &egui::Ui, guide: &DropGuide) {
        #[cfg(test)]
        self.observe_drop_pass(ui).unwrap().guides.push(*guide);
        tabs::drop_guide_ui(self, ui, guide);
    }

    #[cfg(any(test, target_arch = "wasm32"))]
    fn drop_preview_ui(&mut self, ui: &egui::Ui, preview: &egui_dockspace::DropPreview) {
        if let Some(observation) = self.observe_drop_pass(ui) {
            observation.target = Some(preview.target);
        }
        preview.paint(ui);
    }

    fn tab_bar_layout(&self, _bar: TabBarId, _ui: &egui::Ui) -> TabBarLayout {
        if self.custom_tabs {
            TabBarLayout {
                leading_width: 24.0,
                trailing_width: 44.0,
                after_tabs_width: 24.0,
            }
        } else {
            TabBarLayout::default()
        }
    }

    fn tab_bar_ui(&mut self, ui: &mut egui::Ui, bar: TabBar<'_>, slot: TabBarSlot) {
        tabs::tab_bar_ui(self, ui, bar, slot);
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        let Some(pane) = self.panes.get_mut(&item) else {
            return PaneResponse::default();
        };
        pane.content = Some(ui.available_rect_before_wrap());
        pane.editor = None;
        let frame = Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let mut content = PaneResponse::default();
                match item {
                    OUTLINE => {
                        let wide = ui.available_width() >= 185.0;
                        let header = ui.horizontal(|ui| {
                            let mut h = PaneResponse::default();
                            h = h.with_content(&ui.strong("Playground"));
                            if wide {
                                let right = ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.weak(format!("{} nodes", self.objects.len())),
                                );
                                h = h.with_content(&right.inner).with_content(&right.response);
                            }
                            h
                        });
                        content = content
                            .with_content_ids(header.inner.content_widgets())
                            .with_content(&header.response);
                        content = content.with_content(&ui.separator());
                        for (index, name) in OBJECTS.iter().enumerate() {
                            let selected = self.selection == index;
                            let object = &self.objects[index];
                            let row = ui.horizontal(|ui| {
                                let (dot, dot_resp) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 10.0),
                                    egui::Sense::hover(),
                                );
                                let color = if object.visible {
                                    object.color
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                ui.painter().circle_filled(dot.center(), 4.0, color);
                                let label = ui.selectable_label(selected, *name);
                                let clicked = label.clicked();
                                let mut resp = PaneResponse::default()
                                    .with_content(&dot_resp)
                                    .with_content(&label);
                                if wide {
                                    let right = ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| ui.weak(object.kind),
                                    );
                                    resp = resp
                                        .with_content(&right.inner)
                                        .with_content(&right.response);
                                }
                                (clicked, resp)
                            });
                            if row.inner.0 {
                                self.selection = index;
                            }
                            content = content
                                .with_content_ids(row.inner.1.content_widgets())
                                .with_content(&row.response);
                            if selected {
                                let children = ui.indent(("outline-children", index), |ui| {
                                    let mut nested = PaneResponse::default();
                                    for component in ["Transform", "Renderer", "Bounds"] {
                                        nested =
                                            nested.with_content(&ui.weak(format!("· {component}")));
                                    }
                                    nested
                                });
                                content = content
                                    .with_content_ids(children.inner.content_widgets())
                                    .with_content(&children.response);
                            }
                        }
                    }
                    PREVIEW => {
                        let dark = ui.visuals().dark_mode;
                        let (rect, response) =
                            ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                        let canvas_fill = if dark {
                            Color32::from_rgb(18, 21, 28)
                        } else {
                            Color32::from_rgb(244, 246, 250)
                        };
                        let minor_grid = if dark {
                            Color32::from_rgba_unmultiplied(148, 163, 184, 22)
                        } else {
                            Color32::from_rgba_unmultiplied(100, 116, 139, 32)
                        };
                        let major_grid = if dark {
                            Color32::from_rgba_unmultiplied(148, 163, 184, 48)
                        } else {
                            Color32::from_rgba_unmultiplied(100, 116, 139, 64)
                        };
                        painter.rect_filled(rect, 6.0, canvas_fill);
                        let center_origin = rect.center();
                        let step = 28.0_f32;
                        let cols = (rect.width() * 0.5 / step).ceil() as i32 + 1;
                        let rows = (rect.height() * 0.5 / step).ceil() as i32 + 1;
                        for ix in -cols..=cols {
                            let x = center_origin.x + ix as f32 * step;
                            let stroke = if ix == 0 {
                                egui::Stroke::new(
                                    1.2,
                                    Color32::from_rgba_unmultiplied(
                                        74,
                                        222,
                                        128,
                                        if dark { 95 } else { 130 },
                                    ),
                                )
                            } else if ix % 4 == 0 {
                                egui::Stroke::new(1.0, major_grid)
                            } else {
                                egui::Stroke::new(1.0, minor_grid)
                            };
                            painter.line_segment(
                                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                                stroke,
                            );
                        }
                        for iy in -rows..=rows {
                            let y = center_origin.y + iy as f32 * step;
                            let stroke = if iy == 0 {
                                egui::Stroke::new(
                                    1.2,
                                    Color32::from_rgba_unmultiplied(
                                        248,
                                        113,
                                        113,
                                        if dark { 95 } else { 130 },
                                    ),
                                )
                            } else if iy % 4 == 0 {
                                egui::Stroke::new(1.0, major_grid)
                            } else {
                                egui::Stroke::new(1.0, minor_grid)
                            };
                            painter.line_segment(
                                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                                stroke,
                            );
                        }
                        for (index, object) in self
                            .objects
                            .iter()
                            .enumerate()
                            .filter(|(_, object)| object.visible)
                        {
                            let selected = index == self.selection;
                            let center =
                                center_origin + egui::vec2(object.position[0], -object.position[1]);
                            let bounds =
                                egui::Rect::from_center_size(center, egui::vec2(108.0, 62.0));
                            let shadow = bounds.translate(egui::vec2(0.0, 4.0));
                            painter.rect_filled(
                                shadow,
                                9.0,
                                Color32::from_rgba_unmultiplied(
                                    0,
                                    0,
                                    0,
                                    if dark { 90 } else { 32 },
                                ),
                            );
                            let card_fill = if dark {
                                Color32::from_rgb(29, 35, 48)
                                    .lerp_to_gamma(object.color, if selected { 0.36 } else { 0.16 })
                            } else {
                                Color32::WHITE
                                    .lerp_to_gamma(object.color, if selected { 0.24 } else { 0.10 })
                            };
                            let border = if selected {
                                egui::Stroke::new(2.0, object.color)
                            } else {
                                egui::Stroke::new(
                                    1.0,
                                    object.color.gamma_multiply(if dark { 0.65 } else { 0.5 }),
                                )
                            };
                            if selected {
                                painter.rect_stroke(
                                    bounds.expand(3.5),
                                    11.0,
                                    egui::Stroke::new(2.0, object.color.gamma_multiply(0.32)),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            painter.rect(bounds, 8.0, card_fill, border, egui::StrokeKind::Inside);
                            painter.circle_filled(
                                bounds.left_top() + egui::vec2(14.0, 16.0),
                                4.0,
                                object.color,
                            );
                            let title_color = if dark {
                                Color32::from_rgb(241, 245, 249)
                            } else {
                                Color32::from_rgb(15, 23, 42)
                            };
                            let sub_color = if dark {
                                Color32::from_rgb(148, 163, 184)
                            } else {
                                Color32::from_rgb(71, 85, 105)
                            };
                            painter.text(
                                bounds.left_top() + egui::vec2(24.0, 16.0),
                                egui::Align2::LEFT_CENTER,
                                object.name,
                                FontId::proportional(14.0),
                                title_color,
                            );
                            painter.text(
                                bounds.left_bottom() + egui::vec2(12.0, -14.0),
                                egui::Align2::LEFT_CENTER,
                                format!(
                                    "{:.0}, {:.0}, {:.0}",
                                    object.position[0], object.position[1], object.position[2]
                                ),
                                FontId::monospace(11.0),
                                sub_color,
                            );
                        }
                        let hud_rect = egui::Rect::from_min_size(
                            rect.left_top() + egui::vec2(10.0, 10.0),
                            egui::vec2(174.0, 24.0),
                        );
                        painter.rect_filled(
                            hud_rect,
                            6.0,
                            if dark {
                                Color32::from_rgba_unmultiplied(15, 23, 42, 200)
                            } else {
                                Color32::from_rgba_unmultiplied(255, 255, 255, 215)
                            },
                        );
                        painter.text(
                            hud_rect.left_center() + egui::vec2(10.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            "Viewport · 2D Ortho · 60 FPS",
                            FontId::proportional(11.5),
                            if dark {
                                Color32::from_rgb(148, 163, 184)
                            } else {
                                Color32::from_rgb(71, 85, 105)
                            },
                        );
                        painter.rect_stroke(
                            rect,
                            6.0,
                            ui.visuals().widgets.noninteractive.bg_stroke,
                            egui::StrokeKind::Inside,
                        );
                        content = content.with_content(&response);
                    }
                    INSPECTOR => {
                        let object = &mut self.objects[self.selection];
                        let header = ui.horizontal(|ui| {
                            let (dot, dot_resp) = ui
                                .allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.5, object.color);
                            let name = ui.strong(object.name);
                            let right = ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| ui.weak(object.kind),
                            );
                            PaneResponse::default()
                                .with_content(&dot_resp)
                                .with_content(&name)
                                .with_content(&right.inner)
                                .with_content(&right.response)
                        });
                        content = content
                            .with_content_ids(header.inner.content_widgets())
                            .with_content(&header.response);
                        content = content.with_content(&ui.separator());
                        content = content.with_content(&ui.label("Position"));
                        let row = ui.horizontal_wrapped(|ui| {
                            let mut fields = PaneResponse::default();
                            for (axis, value) in
                                ["X ", "Y ", "Z "].into_iter().zip(&mut object.position)
                            {
                                fields = fields.with_content(
                                    &ui.add(egui::DragValue::new(value).speed(1.0).prefix(axis)),
                                );
                            }
                            fields
                        });
                        content = content
                            .with_content_ids(row.inner.content_widgets())
                            .with_content(&row.response);
                        content = content
                            .with_content(&ui.checkbox(&mut object.visible, "Visible in Preview"));
                        ui.add_space(4.0);
                        content = content.with_content(&ui.label("Scene notes"));
                        content = content.with_content(
                            &ui.add(
                                egui::TextEdit::multiline(&mut pane.text)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            ),
                        );
                    }
                    CONSOLE => {
                        let dark = ui.visuals().dark_mode;
                        let time_color = ui.visuals().weak_text_color();
                        let text_color = ui.visuals().text_color();
                        let mono = FontId::monospace(12.5);
                        let entries: [(&str, Color32, &str); 6] = [
                            (
                                "INFO ",
                                if dark {
                                    Color32::from_rgb(125, 196, 228)
                                } else {
                                    Color32::from_rgb(2, 132, 199)
                                },
                                "Loaded playground.scene (3 entities)",
                            ),
                            (
                                "GPU  ",
                                if dark {
                                    Color32::from_rgb(166, 218, 149)
                                } else {
                                    Color32::from_rgb(21, 128, 61)
                                },
                                "Pipeline ready · WGPU Metal · 60 FPS",
                            ),
                            (
                                "DOCK ",
                                if dark {
                                    Color32::from_rgb(198, 160, 246)
                                } else {
                                    Color32::from_rgb(126, 34, 206)
                                },
                                "Surface 1 layout committed (8 panes)",
                            ),
                            (
                                "SCENE",
                                if dark {
                                    Color32::from_rgb(238, 212, 159)
                                } else {
                                    Color32::from_rgb(180, 83, 9)
                                },
                                "Player transform synced to viewport",
                            ),
                            (
                                "INFO ",
                                if dark {
                                    Color32::from_rgb(125, 196, 228)
                                } else {
                                    Color32::from_rgb(2, 132, 199)
                                },
                                "Shader cache warm (grid.wgsl, 0.4 ms)",
                            ),
                            (
                                "DOCK ",
                                if dark {
                                    Color32::from_rgb(198, 160, 246)
                                } else {
                                    Color32::from_rgb(126, 34, 206)
                                },
                                "Snapshot verified · pane content retained",
                            ),
                        ];
                        let log = egui::ScrollArea::vertical()
                            .id_salt("workbench-console")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let mut rows =
                                    PaneResponse::default().with_content_ids([ui.unique_id()]);
                                for index in 0..96 {
                                    let (tag, color, message) = entries[index % entries.len()];
                                    let mut job = egui::text::LayoutJob::default();
                                    job.append(
                                        &format!("10:42:{:02}  ", index % 60),
                                        0.0,
                                        egui::TextFormat::simple(mono.clone(), time_color),
                                    );
                                    job.append(
                                        tag,
                                        0.0,
                                        egui::TextFormat::simple(mono.clone(), color),
                                    );
                                    job.append(
                                        &format!("  {message}"),
                                        0.0,
                                        egui::TextFormat::simple(mono.clone(), text_color),
                                    );
                                    rows = rows.with_content(&ui.label(job));
                                }
                                rows
                            });
                        pane.scroll_offset = log.state.clamped_offset().y;
                        pane.scroll_bounds = Some(log.inner_rect);
                        let scrolling = PaneResponse::default().with_scroll_area(ui, &log);
                        content = log.inner.with_content_ids(scrolling.content_widgets());
                    }
                    PROFILER => {
                        let avail = ui.available_width();
                        let summary = ui.horizontal(|ui| {
                            let label = ui.strong("Frame Budget");
                            let mut resp = PaneResponse::default().with_content(&label);
                            if avail >= 195.0 {
                                let text = if avail >= 265.0 {
                                    "9.4 / 16.7 ms · 60 FPS"
                                } else {
                                    "9.4 ms · 60 FPS"
                                };
                                let right = ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.weak(text),
                                );
                                resp = resp
                                    .with_content(&right.inner)
                                    .with_content(&right.response);
                            }
                            resp
                        });
                        content = content
                            .with_content_ids(summary.inner.content_widgets())
                            .with_content(&summary.response);
                        content = content.with_content(&ui.separator());
                        for (label, milliseconds, fill) in [
                            ("Simulation", 2.6_f32, Color32::from_rgb(56, 152, 236)),
                            ("Rendering", 5.4_f32, Color32::from_rgb(139, 92, 246)),
                            ("Layout", 0.6_f32, Color32::from_rgb(34, 197, 94)),
                            ("UI", 0.8_f32, Color32::from_rgb(234, 179, 8)),
                        ] {
                            content = content.with_content(
                                &ui.add(
                                    egui::ProgressBar::new(milliseconds / 16.67)
                                        .fill(fill.gamma_multiply(0.75))
                                        .text(format!("{label}   {milliseconds:.1} ms")),
                                ),
                            );
                        }
                    }
                    HISTORY | ASSETS => {
                        let prefix = if item == HISTORY { "• " } else { "· " };
                        for line in pane.text.lines() {
                            content =
                                content.with_content(&ui.monospace(format!("{prefix}{line}")));
                        }
                    }
                    _ => {
                        let id = editor_id(item);
                        let dark = ui.visuals().dark_mode;
                        let mut layouter =
                            |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                                let job = highlight_rust_code(ui, buf.as_str(), wrap_width, dark);
                                ui.fonts_mut(|fonts| fonts.layout_job(job))
                            };
                        let editor = egui::TextEdit::multiline(&mut pane.text)
                            .id(id)
                            .code_editor()
                            .frame(Frame::new())
                            .layouter(&mut layouter)
                            .desired_width(f32::INFINITY);
                        let bounds = ui.available_rect_before_wrap();
                        let edit = egui_dockspace::with_pane_keyboard_input_at(
                            ui,
                            id,
                            editor.get_event_filter(),
                            bounds,
                            |ui| ui.put(bounds, editor),
                        );
                        pane.editor = Some(edit.rect);
                        content = content.with_content(&edit);
                    }
                }
                content
            });
        frame.inner.with_content(&frame.response)
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        (item == EDITOR).then(|| editor_id(item))
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        item == EDITOR
    }

    fn sizing(&self, item: ItemId) -> PaneSizing {
        sizing::pane_sizing(item, self.sizing_demo)
    }
}

fn editor_id(item: ItemId) -> egui::Id {
    egui::Id::unique(("workbench-editor", item))
}

fn highlight_rust_code(
    ui: &egui::Ui,
    text: &str,
    wrap_width: f32,
    dark: bool,
) -> egui::text::LayoutJob {
    let font = TextStyle::Monospace.resolve(ui.style());
    let (kw_col, ty_col, str_col, cmt_col, num_col, punct_col, plain_col) = if dark {
        (
            Color32::from_rgb(198, 160, 246),
            Color32::from_rgb(125, 196, 228),
            Color32::from_rgb(166, 218, 149),
            Color32::from_rgb(110, 115, 141),
            Color32::from_rgb(245, 169, 127),
            Color32::from_rgb(147, 154, 183),
            Color32::from_rgb(202, 211, 245),
        )
    } else {
        (
            Color32::from_rgb(136, 57, 239),
            Color32::from_rgb(2, 132, 199),
            Color32::from_rgb(21, 128, 61),
            Color32::from_rgb(100, 116, 139),
            Color32::from_rgb(194, 65, 12),
            Color32::from_rgb(71, 85, 105),
            Color32::from_rgb(30, 41, 59),
        )
    };
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: wrap_width,
            ..Default::default()
        },
        ..Default::default()
    };
    let push = |job: &mut egui::text::LayoutJob, slice: &str, color: Color32| {
        if !slice.is_empty() {
            job.append(
                slice,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                },
            );
        }
    };
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &text[index..];
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(bytes.len(), |offset| index + offset);
            push(&mut job, &text[index..end], cmt_col);
            index = end;
        } else if bytes[index] == b'"' {
            let mut end = index + 1;
            let mut escaped = false;
            while end < bytes.len() {
                let ch = bytes[end];
                end += 1;
                if escaped {
                    escaped = false;
                } else if ch == b'\\' {
                    escaped = true;
                } else if ch == b'"' {
                    break;
                }
            }
            push(&mut job, &text[index..end], str_col);
            index = end;
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let mut end = index + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let ident = &text[index..end];
            let color = match ident {
                "fn" | "pub" | "let" | "mut" | "struct" | "impl" | "use" | "if" | "else"
                | "for" | "in" | "match" | "return" | "const" | "self" | "crate" | "true"
                | "false" => kw_col,
                "f32" | "f64" | "usize" | "u32" | "i32" | "bool" | "str" | "Self" | "Some"
                | "None" | "Option" | "Result" => ty_col,
                _ if ident.as_bytes()[0].is_ascii_uppercase() => ty_col,
                _ => plain_col,
            };
            push(&mut job, ident, color);
            index = end;
        } else if bytes[index].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit() || bytes[end] == b'.' || bytes[end] == b'_')
            {
                end += 1;
            }
            push(&mut job, &text[index..end], num_col);
            index = end;
        } else if bytes[index].is_ascii_whitespace() {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            push(&mut job, &text[index..end], plain_col);
            index = end;
        } else {
            let next = rest
                .chars()
                .next()
                .map_or(bytes.len(), |ch| index + ch.len_utf8());
            push(&mut job, &text[index..next], punct_col);
            index = next;
        }
    }
    job
}
