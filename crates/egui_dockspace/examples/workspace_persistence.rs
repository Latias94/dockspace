//! Persistent workspace layouts, presets, and debounced autosave on the `wgpu` runner.
//!
//! Runs on the complete-output `wgpu` host so you can drag tabs, split panes,
//! resize dividers, or float groups, watch debounced autosave write the JSON
//! layout document via `mutation.notification().document_changed()`, and switch
//! named presets or restart while keeping pane content owned by the application.
//!
//! Run from the repository root:
//! `cargo run -p egui_dockspace --example workspace_persistence --features wgpu,serde --locked`

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Context, RawInput, ViewportCommand};
use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, DockAnchor, DockPlacement, Dockspace,
    DockspaceActionStatus, DockspaceAxis, DockspaceDocumentBootstrap, DockspaceDocumentId,
    DockspaceLayout, DockspaceMutation, DockspaceNode, DockspaceOutput, DockspaceResponse,
    DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
    winit::{
        AppResult, FrameControl, NativeApp, NativeFrameReport, NativeOptions, backend, run_native,
    },
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const PERSISTENCE_DOCUMENT_ID: DockspaceDocumentId =
    DockspaceDocumentId::from_bytes(*b"egui-example-doc");
const DEFAULT_PRESET: &str = "Default";
const MAX_USER_PRESETS: usize = 8;
const AUTOSAVE_DELAY: Duration = Duration::from_millis(500);
const PANES: [(&str, &str); 8] = [
    ("pane/document", "Document"),
    ("pane/console", "Console"),
    ("pane/scene", "Scene"),
    ("pane/outline", "Outline"),
    ("pane/inspector", "Inspector"),
    ("pane/history", "History"),
    ("pane/profiler", "Profiler"),
    ("pane/assets", "Assets"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let layout_file = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("dockspace-layout.json"), PathBuf::from);
    let options = NativeOptions {
        window: backend::window::WindowAttributes::default()
            .with_title("egui_dockspace editor layouts")
            .with_inner_size(backend::dpi::LogicalSize::new(1200.0, 780.0)),
        ..Default::default()
    };
    let context = Context::default();
    context.set_theme(egui::Theme::Light);
    let mut app = PersistenceApp::open(layout_file);
    run_native(options, context, &mut app)?;
    Ok(())
}

enum ToolbarCommand {
    SaveFile,
    ExitWithoutSaving,
    SetLocked(bool),
    SavePreset,
    ApplyCurrentPreset,
    ApplyPreset(String),
    RemovePreset(String),
    Reopen(ItemId),
}

struct PersistenceControls {
    presets: BTreeMap<String, Vec<u8>>,
    preset_name: String,
    status: String,
    auto_save: bool,
    close_save_failed: bool,
    commands: Vec<ToolbarCommand>,
}

struct PersistenceApp {
    dockspace: Dockspace,
    panes: PersistencePanes,
    default_layout: DockspaceLayout,
    controls: PersistenceControls,
    layout_file: Option<PathBuf>,
    save_on_close: bool,
    close_pending: bool,
    layout_dirty: bool,
    auto_save_at: Option<f64>,
    last_response: Option<DockspaceResponse>,
}

impl PersistenceApp {
    fn new() -> Self {
        let mut bootstrap = DockspaceDocumentBootstrap::new(PERSISTENCE_DOCUMENT_ID);
        let items: Vec<_> = PANES
            .iter()
            .map(|(key, _)| {
                bootstrap
                    .ensure_item(*key)
                    .expect("the bounded pane identities fit")
            })
            .collect();
        let default_layout = example_layout(&items);
        let mut dockspace = Dockspace::builder("persistence", default_layout.clone())
            .persistence(bootstrap)
            .build()
            .expect("the static editor layout is valid");
        let default = dockspace
            .save_document_json()
            .expect("the default layout encodes");
        Self {
            dockspace,
            panes: PersistencePanes::new(&items),
            default_layout,
            controls: PersistenceControls {
                presets: BTreeMap::from([(DEFAULT_PRESET.to_owned(), default)]),
                preset_name: "Working".to_owned(),
                status:
                    "Drag tabs or resize splitters; layouts persist independently of pane data."
                        .to_owned(),
                auto_save: true,
                close_save_failed: false,
                commands: Vec::new(),
            },
            layout_file: None,
            save_on_close: false,
            close_pending: false,
            layout_dirty: false,
            auto_save_at: None,
            last_response: None,
        }
    }

    fn open(path: PathBuf) -> Self {
        let mut app = Self::new();
        match std::fs::read(&path) {
            Ok(bytes) => match Self::decode(&bytes) {
                Ok((dockspace, items)) => {
                    app.dockspace = dockspace;
                    app.default_layout = example_layout(&items);
                    app.panes = PersistencePanes::new(&items);
                    app.controls.status = format!("Restored {}", path.display());
                    app.save_on_close = true;
                }
                Err(error) => {
                    app.controls.status = format!(
                        "Could not restore {}: {error}. Save layout file to replace it.",
                        path.display()
                    );
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                app.controls.status =
                    format!("Layout autosaves to {} after changes", path.display());
                app.save_on_close = true;
            }
            Err(error) => {
                app.controls.status = format!("Could not read {}: {error}", path.display());
            }
        }
        app.layout_file = Some(path);
        app
    }

    fn decode(bytes: &[u8]) -> Result<(Dockspace, Vec<ItemId>), Box<dyn std::error::Error>> {
        let dockspace = Dockspace::from_document_json(
            "persistence",
            bytes,
            Default::default(),
            Default::default(),
            |document, key| {
                document == PERSISTENCE_DOCUMENT_ID && PANES.iter().any(|(known, _)| *known == key)
            },
        )?;
        // Reconnect the application registry through stable keys, not saved numeric IDs.
        let items = PANES
            .iter()
            .map(|(key, _)| {
                dockspace.item_id_for_external_key(key).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("missing pane key: {key}"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((dockspace, items))
    }

    fn write_layout(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = self.dockspace.save_document_json()?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }

    fn save_layout_file(&mut self) {
        let Some(path) = self.layout_file.clone() else {
            return;
        };
        self.auto_save_at = None;
        self.layout_dirty = false;
        match self.write_layout(&path) {
            Ok(()) => {
                self.controls.status = format!("Saved {}", path.display());
                self.save_on_close = true;
                self.controls.close_save_failed = false;
            }
            Err(error) => {
                self.controls.status = format!("Could not save {}: {error}", path.display());
            }
        }
    }

    fn observe_mutation(&mut self, mutation: &DockspaceMutation) {
        self.layout_dirty |= mutation.notification().document_changed();
    }

    fn autosave(&mut self, context: &Context) {
        // An unreadable file requires an explicit save before automatic replacement.
        if !self.controls.auto_save || !self.save_on_close || self.layout_file.is_none() {
            // Cancelling a deadline must preserve its unsaved layout change.
            self.layout_dirty |= self.auto_save_at.take().is_some();
            return;
        }
        let now = context.input(|input| input.time);
        if std::mem::take(&mut self.layout_dirty) {
            self.auto_save_at = Some(now + AUTOSAVE_DELAY.as_secs_f64());
        }
        if let Some(deadline) = self.auto_save_at {
            if now >= deadline {
                self.save_layout_file();
            } else {
                context.request_repaint_after(Duration::from_secs_f64(deadline - now));
            }
        }
    }

    fn save(&mut self) {
        let name = self.controls.preset_name.trim().to_owned();
        if name.is_empty() || name == DEFAULT_PRESET {
            "Choose a non-empty name other than Default".clone_into(&mut self.controls.status);
            return;
        }
        if !self.controls.presets.contains_key(&name)
            && self.controls.presets.len() > MAX_USER_PRESETS
        {
            "The eight user layout slots are full; replace or remove one"
                .clone_into(&mut self.controls.status);
            return;
        }
        match self.dockspace.save_document_json() {
            Ok(snapshot) => {
                self.controls.presets.insert(name.clone(), snapshot);
                self.controls.status = format!("Saved layout: {name}");
            }
            Err(error) => self.controls.status = error.to_string(),
        }
    }

    fn restore(&mut self) {
        let name = self.controls.preset_name.trim().to_owned();
        self.restore_named(&name);
    }

    fn restore_named(&mut self, name: &str) {
        if name == DEFAULT_PRESET {
            // Defaults are ordinary layouts; applying one does not need JSON or document authority.
            match self.dockspace.reset_layout(self.default_layout.clone()) {
                Ok(mutation) => {
                    self.observe_mutation(&mutation);
                    self.controls.status = format!("Applied layout: {name}");
                }
                Err(error) => self.controls.status = error.to_string(),
            }
            return;
        }
        let Some(snapshot) = self.controls.presets.get(name).cloned() else {
            "Choose a saved layout".clone_into(&mut self.controls.status);
            return;
        };
        match self
            .dockspace
            .restore_document_json(&snapshot, |document_id, key| {
                document_id == PERSISTENCE_DOCUMENT_ID
                    && PANES.iter().any(|(known, _)| *known == key)
            }) {
            Ok(mutation) => {
                self.observe_mutation(&mutation);
                self.controls.status = format!("Applied layout: {name}");
            }
            Err(error) => self.controls.status = error.to_string(),
        }
    }

    fn reopen(&mut self, item: ItemId) {
        if !self.panes.items.contains_key(&item) {
            return;
        }
        // Resolve the current presentation: restoring an old preset can remap retired IDs.
        let view = self.dockspace.view();
        let Some(surface) = view.surfaces().next() else {
            "No logical surface is available".clone_into(&mut self.controls.status);
            return;
        };
        let placement = match surface.main_root() {
            None => DockPlacement::Main(surface.id()),
            Some(root) if root.central().is_some() => {
                DockPlacement::Center(DockAnchor::Central(root.id()))
            }
            Some(root) => {
                let anchor = self.panes.items.keys().copied().find(|item| {
                    view.item(*item)
                        .is_some_and(|open| open.root() == root.id())
                });
                let Some(anchor) = anchor else {
                    "The current layout has no available docking anchor"
                        .clone_into(&mut self.controls.status);
                    return;
                };
                DockPlacement::Center(DockAnchor::Item(anchor))
            }
        };
        match self.dockspace.open_item_current(item, placement) {
            Ok(result) => {
                self.observe_mutation(result.mutation());
                match result.status() {
                    DockspaceActionStatus::Applied(_) => {
                        "Pane reopened".clone_into(&mut self.controls.status);
                    }
                    status => self.controls.status = format!("Open pane: {status:?}"),
                }
            }
            Err(error) => self.controls.status = error.to_string(),
        }
    }

    fn set_locked(&mut self, locked: bool) {
        let mut policy = self.dockspace.policy().clone();
        policy.set_arrangement_locked(locked);
        match self.dockspace.set_policy(policy) {
            Ok(_) => {
                self.controls.status = if locked {
                    "Layout locked"
                } else {
                    "Layout unlocked"
                }
                .to_owned();
            }
            Err(error) => self.controls.status = error.to_string(),
        }
    }

    fn process_post_frame(&mut self, context: &Context) -> FrameControl {
        let mut repaint = false;
        for command in std::mem::take(&mut self.controls.commands) {
            repaint = true;
            match command {
                ToolbarCommand::SaveFile => self.save_layout_file(),
                ToolbarCommand::ExitWithoutSaving => {
                    self.save_on_close = false;
                    self.close_pending = false;
                    context.send_viewport_cmd(ViewportCommand::Close);
                }
                ToolbarCommand::SetLocked(locked) => self.set_locked(locked),
                ToolbarCommand::SavePreset => self.save(),
                ToolbarCommand::ApplyCurrentPreset => self.restore(),
                ToolbarCommand::ApplyPreset(name) => {
                    self.controls.preset_name = name.clone();
                    self.restore_named(&name);
                }
                ToolbarCommand::RemovePreset(name) => {
                    self.controls.presets.remove(&name);
                }
                ToolbarCommand::Reopen(item) => self.reopen(item),
            }
        }

        if let Some(response) = self.last_response.take() {
            self.observe_mutation(response.mutation());
            for request in response
                .close_request_events()
                .iter()
                .filter(|request| !request.reused())
            {
                let plan = request.plan();
                for item in plan
                    .items()
                    .iter()
                    .filter(|item| item.state() == CloseItemDecisionState::Pending)
                {
                    // Only layout ownership closes. Application content remains in the catalog.
                    match self.dockspace.resolve_close(
                        plan.request(),
                        item.token(),
                        CloseDecision::Allow,
                    ) {
                        Ok(result) => {
                            self.observe_mutation(result.mutation());
                            repaint |= result.mutation().workspace_changed();
                        }
                        Err(error) => self.controls.status = error.to_string(),
                    }
                }
            }
        }

        if self.close_pending && self.save_on_close {
            self.close_pending = false;
            if let Some(path) = self.layout_file.clone() {
                match self.write_layout(&path) {
                    Ok(()) => {
                        self.save_on_close = false;
                        self.controls.close_save_failed = false;
                        context.send_viewport_cmd(ViewportCommand::Close);
                    }
                    Err(error) => {
                        self.controls.status =
                            format!("Could not save {}: {error}", path.display());
                        self.controls.close_save_failed = true;
                        context.send_viewport_cmd(ViewportCommand::Minimized(false));
                        context.send_viewport_cmd(ViewportCommand::Visible(true));
                    }
                }
                repaint = true;
            }
        } else {
            self.autosave(context);
        }

        if repaint {
            FrameControl::Repaint
        } else {
            FrameControl::Continue
        }
    }
}

fn draw_controls(
    ui: &mut egui::Ui,
    controls: &mut PersistenceControls,
    locked: bool,
    closed_panes: &[(ItemId, &'static str)],
) {
    egui::Panel::top("layout-controls").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Editor layouts");
            if ui.button("Save layout file").clicked() {
                controls.commands.push(ToolbarCommand::SaveFile);
            }
            ui.checkbox(&mut controls.auto_save, "Autosave layout");
            if controls.close_save_failed && ui.button("Exit without saving").clicked() {
                controls.commands.push(ToolbarCommand::ExitWithoutSaving);
            }
            let mut lock_toggle = locked;
            if ui.checkbox(&mut lock_toggle, "Lock layout").changed() {
                controls
                    .commands
                    .push(ToolbarCommand::SetLocked(lock_toggle));
            }
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut controls.preset_name).desired_width(130.0));
            if ui.button("Save preset").clicked() {
                controls.commands.push(ToolbarCommand::SavePreset);
            }
            if ui
                .add_enabled(
                    !locked && controls.presets.contains_key(controls.preset_name.trim()),
                    egui::Button::new("Apply preset"),
                )
                .clicked()
            {
                controls.commands.push(ToolbarCommand::ApplyCurrentPreset);
            }
            if ui
                .add_enabled(!locked, egui::Button::new("Reset to default"))
                .clicked()
            {
                controls
                    .commands
                    .push(ToolbarCommand::ApplyPreset(DEFAULT_PRESET.to_owned()));
            }
            ui.menu_button("Saved layouts", |ui| {
                let names: Vec<_> = controls.presets.keys().cloned().collect();
                for name in names {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!locked, egui::Button::new(&name)).clicked() {
                            controls
                                .commands
                                .push(ToolbarCommand::ApplyPreset(name.clone()));
                            ui.close();
                        }
                        if name != DEFAULT_PRESET && ui.small_button("Remove").clicked() {
                            controls.commands.push(ToolbarCommand::RemovePreset(name));
                        }
                    });
                }
            });
            ui.menu_button("Closed panes", |ui| {
                if closed_panes.is_empty() {
                    ui.weak("Every pane is open");
                }
                for &(item, title) in closed_panes {
                    if ui
                        .add_enabled(!locked, egui::Button::new(format!("Reopen {title}")))
                        .clicked()
                    {
                        controls.commands.push(ToolbarCommand::Reopen(item));
                        ui.close();
                    }
                }
            });
        });
        ui.label(&controls.status);
    });
}

impl NativeApp for PersistenceApp {
    fn update(&mut self, context: &Context, input: RawInput) -> AppResult<DockspaceOutput> {
        let close_requested = input.viewport().close_requested();
        let locked = self.dockspace.policy().arrangement_locked();
        let view = self.dockspace.view();
        let surface = view.surfaces().next().map(|surface| surface.id());
        let closed_panes: Vec<_> = self
            .panes
            .items
            .iter()
            .filter_map(|(item, pane)| view.item(*item).is_none().then_some((*item, pane.title)))
            .collect();

        if close_requested && self.save_on_close {
            self.close_pending = true;
        }
        let cancel_close = close_requested && self.save_on_close;
        let controls = &mut self.controls;

        let Some(surface) = surface else {
            self.last_response = None;
            return Ok(egui_dockspace::run_ui(context, input, |ui| {
                if cancel_close {
                    ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                }
                draw_controls(ui, controls, locked, &closed_panes);
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.weak("No logical surface is available");
                });
            }));
        };

        let (output, response) = self.dockspace.run_single_surface_with_ui(
            context,
            input,
            surface,
            &mut self.panes,
            |ui| {
                if cancel_close {
                    ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                }
                draw_controls(ui, controls, locked, &closed_panes);
            },
        )?;
        self.last_response = Some(response);
        Ok(output)
    }

    fn frame_finished(&mut self, frame: NativeFrameReport<'_>) -> AppResult<FrameControl> {
        Ok(self.process_post_frame(frame.context))
    }
}

fn example_layout(items: &[ItemId]) -> DockspaceLayout {
    let center = DockspaceNode::split(
        DockspaceAxis::Vertical,
        [
            (
                DockspaceNode::central_tabs([items[0], items[2], items[5], items[6], items[7]]),
                0.75,
            ),
            (DockspaceNode::tabs([items[1]]), 0.25),
        ],
    )
    .expect("the center split is valid");
    let root = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([items[3]]), 0.18),
            (center, 0.62),
            (DockspaceNode::tabs([items[4]]), 0.20),
        ],
    )
    .expect("the editor split is valid");
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, root),
    )])
    .expect("the editor layout is valid")
}

struct EditorPane {
    title: &'static str,
    text: String,
    canvas: bool,
}

struct PersistencePanes {
    items: BTreeMap<ItemId, EditorPane>,
}

impl PersistencePanes {
    fn new(items: &[ItemId]) -> Self {
        Self {
            items: items
                .iter()
                .copied()
                .zip(PANES)
                .map(|(item, (key, title))| {
                    (
                        item,
                        EditorPane {
                            title,
                            text: format!(
                                "{title} application data\n{key}\n\nEdits survive closing, reopening and layout presets."
                            ),
                            canvas: key == "pane/scene",
                        },
                    )
                })
                .collect(),
        }
    }
}

impl PaneView for PersistencePanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        self.items.get(&item).map(|pane| pane.title.into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        let Some(pane) = self.items.get_mut(&item) else {
            return PaneResponse::default();
        };
        if pane.canvas {
            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click());
            let rect = response.rect;
            painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
            for index in 1..12 {
                let x = egui::lerp(rect.x_range(), index as f32 / 12.0);
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
            }
            painter.circle_filled(rect.center(), 32.0, ui.visuals().selection.bg_fill);
            painter.text(
                rect.center() + egui::vec2(0.0, 52.0),
                egui::Align2::CENTER_CENTER,
                "Application-drawn scene",
                egui::FontId::proportional(16.0),
                ui.visuals().text_color(),
            );
            PaneResponse::from(&response)
        } else {
            let scroll = egui::ScrollArea::both()
                .id_salt(("editor-pane-scroll", item.get()))
                .show(ui, |ui| {
                    let editor = egui_dockspace::pane_text_edit(
                        ui,
                        egui::Id::unique(("editor-pane-focus", item.get())),
                        egui::TextEdit::multiline(&mut pane.text).desired_width(f32::INFINITY),
                    );
                    if pane.title == "Console" {
                        for line in 0..128 {
                            ui.monospace(format!("[{line:03}] Application event log"));
                        }
                    }
                    editor
                });
            PaneResponse::default()
                .with_scroll_area(ui, &scroll)
                .with_content(&scroll.inner)
        }
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        self.items
            .get(&item)
            .filter(|pane| !pane.canvas)
            .map(|_| egui::Id::unique(("editor-pane-focus", item.get())))
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        self.focus_target(item).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Action, ActionRequest, Role, TreeId};
    use egui_dockspace::{
        DockspaceActionOutcome, DockspaceCloseOutcome, DockspaceCloseRequestStatus,
        DockspaceCloseResolution, SurfacePresentationResult,
    };

    fn run_frame_raw(
        context: &Context,
        app: &mut PersistenceApp,
        raw: RawInput,
    ) -> egui::FullOutput {
        let output = app
            .update(context, raw)
            .expect("persistence frame succeeds");
        let (mut full_output, receipt) = output.into_parts();
        full_output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);
        let _ = app.process_post_frame(context);
        full_output
    }

    fn run_frame(
        context: &Context,
        app: &mut PersistenceApp,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        run_frame_at(context, app, events, None)
    }

    fn run_frame_at(
        context: &Context,
        app: &mut PersistenceApp,
        events: Vec<egui::Event>,
        time: Option<f64>,
    ) -> egui::FullOutput {
        run_frame_raw(
            context,
            app,
            RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 780.0),
                )),
                events,
                time,
                ..Default::default()
            },
        )
    }

    fn close_input() -> RawInput {
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 780.0),
            )),
            ..Default::default()
        };
        raw.viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        raw
    }

    #[test]
    fn layout_file_round_trip_preserves_tab_order_and_reconnects_application_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.json");
        let mut app = PersistenceApp::open(path.clone());
        let document = app
            .dockspace
            .item_id_for_external_key("pane/document")
            .unwrap();
        let inspector = app
            .dockspace
            .item_id_for_external_key("pane/inspector")
            .unwrap();
        let moved = app
            .dockspace
            .dock_item_current(document, DockPlacement::Center(DockAnchor::Item(inspector)))
            .unwrap();
        assert!(matches!(
            moved.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::Docked {
                item,
                changed: true,
                ..
            }) if *item == document
        ));
        app.save_layout_file();
        assert!(path.is_file());
        let restored = PersistenceApp::open(path.clone());
        assert_eq!(
            restored
                .dockspace
                .view()
                .item(document)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            &[inspector, document]
        );
        assert_eq!(restored.panes.items[&document].title, "Document");
        assert_eq!(
            restored.controls.status,
            format!("Restored {}", path.display())
        );

        let moved = app
            .dockspace
            .dock_item_current(inspector, DockPlacement::Center(DockAnchor::Item(document)))
            .unwrap();
        assert!(matches!(
            moved.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::Docked {
                item,
                ..
            }) if *item == inspector
        ));
        app.save_layout_file();
        assert!(app.controls.status.starts_with("Saved "));
        assert!(PersistenceApp::open(path).save_on_close);
    }

    #[test]
    fn invalid_layout_file_is_kept_until_an_explicit_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.json");
        let original = b"an invalid or newer document";
        std::fs::write(&path, original).unwrap();
        let mut app = PersistenceApp::open(path.clone());
        assert!(!app.save_on_close);
        assert!(app.controls.status.starts_with("Could not restore "));
        let output = run_frame_raw(&Context::default(), &mut app, close_input());
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&ViewportCommand::CancelClose)
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.save_layout_file();
        assert!(app.save_on_close);
        assert!(PersistenceApp::decode(&std::fs::read(path).unwrap()).is_ok());
    }

    #[test]
    fn close_save_failure_cancels_exit_and_a_retry_saves_the_layout() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.json");
        let mut app = PersistenceApp::open(path.clone());
        // A directory cannot be replaced by the atomic file writer on any platform.
        std::fs::create_dir(&path).unwrap();
        let context = Context::default();
        let failed = run_frame_raw(&context, &mut app, close_input());
        assert!(
            failed.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&ViewportCommand::CancelClose)
        );
        assert!(app.controls.close_save_failed);
        assert!(app.controls.status.starts_with("Could not save "));
        assert!(app.save_on_close);
        std::fs::remove_dir(&path).unwrap();

        let _ = run_frame_raw(&context, &mut app, close_input());
        // The follow-up frame emits the queued ViewportCommand::Close.
        let closing = run_frame(&context, &mut app, Vec::new());
        assert!(
            closing.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&ViewportCommand::Close)
        );
        assert!(path.is_file());
        assert!(!app.controls.close_save_failed);
        assert!(!app.save_on_close);
        assert!(PersistenceApp::decode(&std::fs::read(path).unwrap()).is_ok());
    }

    #[test]
    fn splitter_changes_autosave_after_idle_and_idle_frames_do_not_resave() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.json");
        let mut app = PersistenceApp::open(path.clone());
        let context = Context::default();
        context.enable_accesskit();
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(0.0));
        let ready = run_frame_at(&context, &mut app, Vec::new(), Some(0.1));
        let target = ready
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.role() == Role::Splitter && node.supports_action(Action::Increment))
                    .then_some(*id)
            })
            .expect("the real editor split supports accessible resizing");
        assert!(
            !path.exists(),
            "initial measurement does not dirty the layout"
        );
        let before = app.dockspace.version();
        let _ = run_frame_at(
            &context,
            &mut app,
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                action: Action::Increment,
                target_tree: TreeId::ROOT,
                target_node: target,
                data: None,
            })],
            Some(0.2),
        );
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(0.3));
        assert_ne!(app.dockspace.version(), before);
        assert!(app.auto_save_at.is_some());
        assert!(!path.exists(), "resizing waits for the debounce interval");
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(0.9));
        let saved = std::fs::read(&path).unwrap();
        assert!(PersistenceApp::decode(&saved).is_ok());
        assert!(app.auto_save_at.is_none());
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(2.0));
        assert_eq!(std::fs::read(path).unwrap(), saved);
    }

    #[test]
    fn autosave_preserves_an_unreadable_file_until_explicit_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.json");
        let original = b"unreadable application document";
        std::fs::write(&path, original).unwrap();
        let mut app = PersistenceApp::open(path.clone());
        let context = Context::default();
        app.restore_named(DEFAULT_PRESET);
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(0.0));
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(2.0));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.save_layout_file();
        assert!(PersistenceApp::decode(&std::fs::read(path).unwrap()).is_ok());
    }

    #[test]
    fn reenabling_autosave_resumes_a_change_scheduled_before_it_was_paused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("layout.json");
        let mut app = PersistenceApp::open(path.clone());
        let context = Context::default();
        context.enable_accesskit();
        app.restore_named(DEFAULT_PRESET);
        let ready = run_frame_at(&context, &mut app, Vec::new(), Some(0.0));
        let checkbox = ready
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.role() == Role::CheckBox && node.label() == Some("Autosave layout"))
                    .then_some(*id)
            })
            .expect("the example exposes its autosave toggle");
        let toggle = || {
            egui::Event::AccessKitActionRequest(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: checkbox,
                data: None,
            })
        };
        let _ = run_frame_at(&context, &mut app, vec![toggle()], Some(0.1));
        assert!(!app.controls.auto_save);
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(1.0));
        assert!(
            !path.exists(),
            "paused autosave must not write the pending layout"
        );
        let _ = run_frame_at(&context, &mut app, vec![toggle()], Some(1.1));
        assert!(app.controls.auto_save);
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(1.7));
        let saved =
            std::fs::read(&path).expect("reenabling autosave must persist the pending change");
        assert!(PersistenceApp::decode(&saved).is_ok());
        let _ = run_frame_at(&context, &mut app, Vec::new(), Some(3.0));
        assert_eq!(std::fs::read(path).unwrap(), saved);
    }

    #[test]
    fn closed_inventory_reopens_stable_items_and_presets_preserve_application_data() {
        let context = Context::default();
        context.enable_accesskit();
        let mut app = PersistenceApp::new();
        let document = app
            .dockspace
            .item_id_for_external_key("pane/document")
            .expect("the document has a persistent identity");
        let text = "Unsaved application text survives closing and layout restore";
        text.clone_into(&mut app.panes.items.get_mut(&document).expect("pane data").text);
        app.save();
        assert_eq!(app.controls.status, "Saved layout: Working");
        let _ = run_frame(&context, &mut app, Vec::new());
        let ready = run_frame(&context, &mut app, Vec::new());
        let close = ready
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("the example exposes its controls")
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.role() == Role::Button
                    && node.label() == Some("Close Document")
                    && node.supports_action(Action::Click))
                .then_some(*id)
            })
            .expect("the document has an enabled close button");
        assert!(app.dockspace.view().item(document).is_some());

        let _ = run_frame(
            &context,
            &mut app,
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: close,
                data: None,
            })],
        );
        assert!(app.dockspace.view().item(document).is_some());
        let _ = run_frame(&context, &mut app, Vec::new());
        assert!(app.dockspace.view().item(document).is_none());
        assert_eq!(app.panes.items[&document].text, text);

        "Closed document".clone_into(&mut app.controls.preset_name);
        app.save();
        app.reopen(document);
        assert!(app.dockspace.view().item(document).unwrap().is_active());
        assert_eq!(app.panes.items[&document].text, text);
        let _ = run_frame(&context, &mut app, Vec::new());
        let reopened = app.dockspace.version();
        app.reopen(document);
        assert_eq!(
            app.dockspace.version(),
            reopened,
            "duplicate reopen is a no-op"
        );

        app.restore();
        assert_eq!(app.controls.status, "Applied layout: Closed document");
        assert!(app.dockspace.view().item(document).is_none());
        app.restore_named(DEFAULT_PRESET);
        assert!(app.dockspace.view().item(document).is_some());
        assert_eq!(app.panes.items[&document].text, text);
        "Working".clone_into(&mut app.controls.preset_name);
        app.restore();
        assert_eq!(app.controls.status, "Applied layout: Working");
        assert!(app.dockspace.view().item(document).is_some());
        assert_eq!(app.panes.items[&document].text, text);
        let restored = run_frame(&context, &mut app, Vec::new());
        assert!(
            restored
                .platform_output
                .accesskit_update
                .as_ref()
                .expect("the restored example exposes its panes")
                .nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Tab && node.label() == Some("Document")),
            "the restored layout renders the retained application pane"
        );
    }

    #[test]
    fn named_layout_storage_is_bounded_and_default_is_immutable() {
        let mut app = PersistenceApp::new();
        let default = app.controls.presets[DEFAULT_PRESET].clone();
        for index in 0..MAX_USER_PRESETS {
            app.controls.preset_name = format!("Layout {index}");
            app.save();
        }
        assert_eq!(app.controls.presets.len(), MAX_USER_PRESETS + 1);
        let generation = app.dockspace.next_document_generation();
        "Extra".clone_into(&mut app.controls.preset_name);
        app.save();
        assert_eq!(app.controls.presets.len(), MAX_USER_PRESETS + 1);
        assert_eq!(app.dockspace.next_document_generation(), generation);
        "Layout 0".clone_into(&mut app.controls.preset_name);
        app.save();
        assert_eq!(app.controls.status, "Saved layout: Layout 0");
        assert_eq!(app.controls.presets.len(), MAX_USER_PRESETS + 1);
        DEFAULT_PRESET.clone_into(&mut app.controls.preset_name);
        app.save();
        assert_eq!(app.controls.presets[DEFAULT_PRESET], default);
        app.set_locked(true);
        let version = app.dockspace.version();
        app.restore_named(DEFAULT_PRESET);
        assert!(app.dockspace.policy().arrangement_locked());
        assert_eq!(app.dockspace.version(), version);
        app.set_locked(false);
        app.restore_named(DEFAULT_PRESET);
        assert_eq!(app.controls.status, "Applied layout: Default");
    }

    #[test]
    fn root_retirement_and_default_restore_use_current_presentation_identities() {
        let context = Context::default();
        context.enable_accesskit();
        let mut app = PersistenceApp::new();
        let _ = run_frame(&context, &mut app, Vec::new());
        let document = app
            .dockspace
            .item_id_for_external_key("pane/document")
            .unwrap();
        let close = app.dockspace.request_close_root_current(ROOT).unwrap();
        let DockspaceCloseRequestStatus::Requested(request) = close.status() else {
            panic!("the editor root must offer an ordinary application close plan");
        };
        let plan = request.plan();
        for (index, item) in plan.items().iter().enumerate() {
            let closed = app
                .dockspace
                .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
                .unwrap();
            if index + 1 == plan.items().len() {
                assert!(matches!(
                    closed.outcome(),
                    DockspaceCloseOutcome::Processed {
                        resolution: DockspaceCloseResolution::Approved { .. },
                        application: Some(Ok(egui_dockspace::DockspaceAppliedClose::RootClosed {
                            root: ROOT,
                            ..
                        })),
                        ..
                    }
                ));
            }
        }
        assert!(app.dockspace.view().root(ROOT).is_none());
        assert!(app.dockspace.view().surfaces().next().is_none());
        app.reopen(document);
        assert_eq!(app.controls.status, "No logical surface is available");

        app.restore_named(DEFAULT_PRESET);
        assert_eq!(app.controls.status, "Applied layout: Default");
        let surface = app.dockspace.view().surfaces().next().unwrap();
        assert_eq!(surface.id(), SURFACE);
        let root = surface.main_root().unwrap().id();
        assert_eq!(root, ROOT);
        let close = app.dockspace.request_close_item_current(document).unwrap();
        let DockspaceCloseRequestStatus::Requested(request) = close.status() else {
            panic!("the restored document must remain closeable");
        };
        let plan = request.plan();
        let closed = app
            .dockspace
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                CloseDecision::Allow,
            )
            .unwrap();
        assert!(matches!(
            closed.outcome(),
            DockspaceCloseOutcome::Processed {
                resolution: DockspaceCloseResolution::Approved { .. },
                application: Some(Ok(egui_dockspace::DockspaceAppliedClose::ItemClosed {
                    item,
                    ..
                })),
                ..
            } if *item == document
        ));
        app.reopen(document);
        assert_eq!(app.controls.status, "Pane reopened");
        assert_eq!(app.dockspace.view().item(document).unwrap().root(), root);
        let output = run_frame(&context, &mut app, Vec::new());
        assert!(
            output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Tab && node.label() == Some("Document"))
        );
    }
}
