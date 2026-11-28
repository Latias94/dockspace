//! Practical IDE workflows on the complete-output `wgpu` runner.
//!
//! Covers high-frequency editor requirements in one runnable workspace:
//! - Dynamic `+` tab creation in `TabBarSlot::AfterTabs` (`open_item_current`).
//! - View menu visibility toggling (`set_item_visibility_current`) without closing slots.
//! - Fine-grained `DockPolicy`: `TabBarVisibility::AutoHideSingle`, protected
//!   non-closeable/non-undockable panes (`DockItemRule`), root resize axis lock
//!   (`set_allow_root_resize_axis`), `RootAllocation::PreferCentral`, and
//!   `PaneOverflowPolicy::Compress`.
//! - `DockInputConfig`: `Ctrl+Tab` / `Ctrl+Shift+Tab` cycling, configurable
//!   `DockingActivation`, middle-click close, and splitter double-click modes.
//! - Application `egui::DragAndDrop` (Assets -> Document) and OS `dropped_files`
//!   coexisting cleanly with docking gestures.
//! - Live `DockspaceEventKind` lifecycle log driven by `mutation().events()`.
//!
//! Run from the repository root:
//! `cargo run -p egui_dockspace --example ide_workflows --features wgpu --locked`

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{BTreeMap, VecDeque};

use dockspace::policy::{
    CloseCapability, DockItemRule, DockPolicy, TabBarInteraction, TabBarPolicy, TabBarVisibility,
};
use egui::{Context, Key, KeyboardShortcut, Modifiers, RawInput, Ui};
use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, DockAnchor, DockInputConfig, DockPlacement,
    DockingActivation, Dockspace, DockspaceActionStatus, DockspaceAxis, DockspaceEventKind,
    DockspaceLayout, DockspaceMutation, DockspaceNode, DockspaceOutput, DockspaceResponse,
    DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, PaneOverflowPolicy, PaneResponse,
    PaneSizing, PaneView, RootAllocation, RootId, SplitterDoubleClick, SurfaceId, TabBar, TabBarId,
    TabBarLayout, TabBarSlot,
    winit::{
        AppResult, FrameControl, NativeApp, NativeFrameReport, NativeOptions, backend, run_native,
    },
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const MAIN_ROOT: RootId = RootId::new(1);

const ASSETS: ItemId = ItemId::new(1);
const OVERVIEW: ItemId = ItemId::new(2);
const FIRST_DOC: ItemId = ItemId::new(3);
const ACTIVITY_LOG: ItemId = ItemId::new(4);
const FIRST_DYNAMIC_ID: u64 = 100;
const MAX_LOG_ENTRIES: usize = 64;

const SAMPLE_ASSETS: [(&str, &str); 4] = [
    ("HeroRig", "spawn_entity(\"HeroRig\", vec3(0.0, 0.0, 0.0));"),
    ("SunLight", "spawn_light(\"SunLight\", intensity: 3.2);"),
    ("MainCamera", "set_camera(\"MainCamera\", fov: 60.0);"),
    ("GridShader", "bind_shader(\"shaders/grid.wgsl\");"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = NativeOptions {
        window: backend::window::WindowAttributes::default()
            .with_title("Dockspace — IDE Workflows")
            .with_inner_size(backend::dpi::LogicalSize::new(1240.0, 780.0)),
        ..Default::default()
    };
    let context = Context::default();
    context.set_theme(egui::Theme::Light);
    run_native(options, context, &mut IdeApp::new())?;
    Ok(())
}

#[derive(Clone, Debug)]
struct AssetDragPayload {
    name: &'static str,
    snippet: &'static str,
}

enum PendingAction {
    NewDocumentInGroup(ItemId),
    ToggleVisibility(ItemId, bool),
    ApplyPolicy,
    ApplyInput,
    ApplyStyle,
    ResetLayout,
}

struct IdeControls {
    auto_hide_single: bool,
    protect_overview: bool,
    lock_vertical_split: bool,
    compress_overflow: bool,
    docking_mode: DockingModeChoice,
    middle_click_close: bool,
    double_click_equalize: bool,
    pending_actions: Vec<PendingAction>,
    status: String,
}

struct IdeApp {
    dockspace: Dockspace,
    panes: IdePanes,
    controls: IdeControls,
    next_doc_id: u64,
    next_doc_number: usize,
    last_response: Option<DockspaceResponse>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DockingModeChoice {
    ShiftPauses,
    HoldAltToDock,
    Always,
}

impl DockingModeChoice {
    const fn to_activation(self) -> DockingActivation {
        match self {
            Self::ShiftPauses => DockingActivation::HoldToDisable(Modifiers::SHIFT),
            Self::HoldAltToDock => DockingActivation::HoldToEnable(Modifiers::ALT),
            Self::Always => DockingActivation::Always,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::ShiftPauses => "Shift pauses (default)",
            Self::HoldAltToDock => "Hold Alt to dock",
            Self::Always => "Always dock",
        }
    }
}

impl IdeApp {
    fn new() -> Self {
        let mut app = Self {
            dockspace: Dockspace::builder("ide-workflows", default_layout())
                .policy(build_policy(false, true, false))
                .input(build_input(
                    DockingModeChoice::ShiftPauses,
                    true,
                    false,
                ))
                .build()
                .expect("static IDE layout is valid"),
            panes: IdePanes::new(),
            controls: IdeControls {
                auto_hide_single: false,
                protect_overview: true,
                lock_vertical_split: false,
                compress_overflow: false,
                docking_mode: DockingModeChoice::ShiftPauses,
                middle_click_close: true,
                double_click_equalize: false,
                pending_actions: Vec::new(),
                status: "Click '+' on any tab bar to open a new tab, use Ctrl+Tab to cycle, or drag an Asset into a Document."
                    .to_owned(),
            },
            next_doc_id: FIRST_DYNAMIC_ID,
            next_doc_number: 2,
            last_response: None,
        };
        app.panes
            .push_log("Workspace ready (PreferCentral + Ctrl+Tab + protected Overview)".to_owned());
        app
    }

    fn record_mutation(&mut self, mutation: &DockspaceMutation) {
        for event in mutation.events() {
            let entry = match event.kind() {
                DockspaceEventKind::ItemOpened { item, root } => {
                    format!("ItemOpened({:?} in {:?})", self.panes.name(*item), root)
                }
                DockspaceEventKind::SelectionCommitted { tab } => {
                    let label = tab
                        .item()
                        .map_or_else(|| format!("{tab:?}"), |item| self.panes.name(item));
                    format!("SelectionCommitted({label})")
                }
                DockspaceEventKind::ItemActivated { item } => {
                    format!("ItemActivated({})", self.panes.name(*item))
                }
                DockspaceEventKind::ItemVisibilityChanged { item, visible } => {
                    format!(
                        "ItemVisibilityChanged({}, visible={visible})",
                        self.panes.name(*item)
                    )
                }
                DockspaceEventKind::TabReordered { tab, from, to } => {
                    let label = tab
                        .item()
                        .map_or_else(|| format!("{tab:?}"), |item| self.panes.name(item));
                    format!("TabReordered({label}: {from} -> {to})")
                }
                DockspaceEventKind::ItemsMoved {
                    items,
                    source_root,
                    target_root,
                } => {
                    format!(
                        "ItemsMoved({} items: {:?} -> {:?})",
                        items.len(),
                        source_root,
                        target_root
                    )
                }
                DockspaceEventKind::Closed(outcome) => format!("Closed({outcome:?})"),
                DockspaceEventKind::LayoutReset => "LayoutReset".to_owned(),
                other => format!("{other:?}"),
            };
            self.panes.push_log(entry);
        }
    }

    fn apply_pending_actions(&mut self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut changed = false;
        // Collect tab-bar '+' clicks queued during `PaneView::tab_bar_ui`.
        for anchor in self.panes.queued_new_tabs.drain(..) {
            self.controls
                .pending_actions
                .push(PendingAction::NewDocumentInGroup(anchor));
        }

        for action in std::mem::take(&mut self.controls.pending_actions) {
            match action {
                PendingAction::NewDocumentInGroup(anchor) => {
                    let item = ItemId::new(self.next_doc_id);
                    self.next_doc_id += 1;
                    let title = format!("Untitled-{}.rs", self.next_doc_number);
                    self.next_doc_number += 1;
                    self.panes.documents.insert(
                        item,
                        DocumentBuffer {
                            title: title.clone(),
                            text: format!(
                                "// {title}\n// Drop an Asset chip from the left pane or an OS file here.\n"
                            ),
                        },
                    );
                    let placement = if self.dockspace.view().item(anchor).is_some() {
                        DockPlacement::Center(DockAnchor::Item(anchor))
                    } else {
                        DockPlacement::Center(DockAnchor::Central(MAIN_ROOT))
                    };
                    let result = self.dockspace.open_item_current(item, placement)?;
                    self.record_mutation(result.mutation());
                    match result.status() {
                        DockspaceActionStatus::Applied(_) => {
                            self.controls.status = format!("Opened {title}");
                            changed = true;
                        }
                        status => {
                            self.panes.documents.remove(&item);
                            self.controls.status = format!("Could not open {title}: {status:?}");
                        }
                    }
                }
                PendingAction::ToggleVisibility(item, visible) => {
                    let result = self.dockspace.set_item_visibility_current(item, visible)?;
                    self.record_mutation(result.mutation());
                    self.controls.status = format!(
                        "{} {}",
                        if visible { "Shown" } else { "Hidden" },
                        self.panes.name(item)
                    );
                    changed = true;
                }
                PendingAction::ApplyPolicy => {
                    let policy = build_policy(
                        self.controls.auto_hide_single,
                        self.controls.protect_overview,
                        self.controls.lock_vertical_split,
                    );
                    let mutation = self.dockspace.set_policy(policy)?;
                    self.record_mutation(&mutation);
                    "Updated docking policy".clone_into(&mut self.controls.status);
                    changed = true;
                }
                PendingAction::ApplyInput => {
                    self.dockspace.set_input(build_input(
                        self.controls.docking_mode,
                        self.controls.middle_click_close,
                        self.controls.double_click_equalize,
                    ));
                    self.controls.status = format!(
                        "Input config updated ({}, Ctrl+Tab enabled)",
                        self.controls.docking_mode.label()
                    );
                    changed = true;
                }
                PendingAction::ApplyStyle => {
                    let mut style = self.dockspace.style().clone();
                    style.pane_overflow_policy = if self.controls.compress_overflow {
                        PaneOverflowPolicy::Compress
                    } else {
                        PaneOverflowPolicy::Clip
                    };
                    let mutation = self.dockspace.set_style(style)?;
                    self.record_mutation(&mutation);
                    self.controls.status = format!(
                        "Overflow policy: {:?}",
                        self.dockspace.style().pane_overflow_policy
                    );
                    changed = true;
                }
                PendingAction::ResetLayout => {
                    let mutation = self.dockspace.reset_layout(default_layout())?;
                    self.record_mutation(&mutation);
                    "Restored default IDE layout".clone_into(&mut self.controls.status);
                    changed = true;
                }
            }
        }

        if let Some(response) = self.last_response.take() {
            self.record_mutation(response.mutation());
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
                    let resolved = self.dockspace.resolve_close(
                        plan.request(),
                        item.token(),
                        CloseDecision::Allow,
                    )?;
                    self.record_mutation(resolved.mutation());
                    if resolved.mutation().workspace_changed() {
                        // Drop dynamic untitled buffers once closed; built-in panes stay reusable.
                        if item.item().get() >= FIRST_DYNAMIC_ID {
                            self.panes.documents.remove(&item.item());
                        }
                        changed = true;
                    }
                }
            }
        }
        Ok(changed)
    }

    fn handle_dropped_files(&mut self, input: &RawInput) {
        for file in &input.dropped_files {
            let name = file.path().display().to_string();
            let name = if name.is_empty() {
                "unnamed-file".to_owned()
            } else {
                name
            };
            self.panes
                .push_log(format!("OS dropped file received: {name}"));
            if let Some(doc) = self
                .panes
                .last_active_doc
                .and_then(|item| self.panes.documents.get_mut(&item))
            {
                doc.text.push_str(&format!("\n// Dropped file: {name}\n"));
            }
            self.controls.status = format!("Imported dropped file: {name}");
        }
    }
}

fn draw_toolbar(
    ui: &mut Ui,
    controls: &mut IdeControls,
    visibility: [(ItemId, &'static str, bool); 3],
    new_tab_anchor: ItemId,
) {
    egui::Panel::top("ide-workflows-bar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("IDE Workflows");
            ui.separator();

            ui.menu_button("View (Hide/Show)", |ui| {
                for (item, label, visible) in visibility {
                    let mut next = visible;
                    if ui.checkbox(&mut next, label).changed() {
                        controls
                            .pending_actions
                            .push(PendingAction::ToggleVisibility(item, next));
                    }
                }
            });

            ui.menu_button("Policy", |ui| {
                if ui
                    .checkbox(&mut controls.auto_hide_single, "AutoHideSingle tab bars")
                    .changed()
                {
                    controls.pending_actions.push(PendingAction::ApplyPolicy);
                }
                if ui
                    .checkbox(
                        &mut controls.protect_overview,
                        "Protect Overview (no close/undock/reorder)",
                    )
                    .changed()
                {
                    controls.pending_actions.push(PendingAction::ApplyPolicy);
                }
                if ui
                    .checkbox(
                        &mut controls.lock_vertical_split,
                        "Lock vertical split axis",
                    )
                    .changed()
                {
                    controls.pending_actions.push(PendingAction::ApplyPolicy);
                }
                if ui
                    .checkbox(&mut controls.compress_overflow, "Compress overflow panes")
                    .changed()
                {
                    controls.pending_actions.push(PendingAction::ApplyStyle);
                }
            });

            ui.menu_button("Input (Ctrl+Tab)", |ui| {
                for choice in [
                    DockingModeChoice::ShiftPauses,
                    DockingModeChoice::HoldAltToDock,
                    DockingModeChoice::Always,
                ] {
                    if ui
                        .selectable_value(&mut controls.docking_mode, choice, choice.label())
                        .changed()
                    {
                        controls.pending_actions.push(PendingAction::ApplyInput);
                    }
                }
                ui.separator();
                if ui
                    .checkbox(&mut controls.middle_click_close, "Middle-click closes tab")
                    .changed()
                {
                    controls.pending_actions.push(PendingAction::ApplyInput);
                }
                if ui
                    .checkbox(
                        &mut controls.double_click_equalize,
                        "Double-click splitter equalizes (vs Preferred)",
                    )
                    .changed()
                {
                    controls.pending_actions.push(PendingAction::ApplyInput);
                }
            });

            if ui.button("+ New tab").clicked() {
                controls
                    .pending_actions
                    .push(PendingAction::NewDocumentInGroup(new_tab_anchor));
            }
            if ui.button("Reset").clicked() {
                controls.pending_actions.push(PendingAction::ResetLayout);
            }
        });
        ui.small(&controls.status);
    });
}

impl NativeApp for IdeApp {
    fn update(&mut self, context: &Context, input: RawInput) -> AppResult<DockspaceOutput> {
        self.handle_dropped_files(&input);
        let view = self.dockspace.view();
        let visibility = [
            (
                ASSETS,
                "Assets",
                view.item(ASSETS).is_some_and(|item| item.is_visible()),
            ),
            (
                OVERVIEW,
                "Overview (Protected)",
                view.item(OVERVIEW).is_some_and(|item| item.is_visible()),
            ),
            (
                ACTIVITY_LOG,
                "Activity Log",
                view.item(ACTIVITY_LOG)
                    .is_some_and(|item| item.is_visible()),
            ),
        ];
        let new_tab_anchor = self.panes.last_active_doc.unwrap_or(FIRST_DOC);
        let controls = &mut self.controls;
        let (output, response) = self.dockspace.run_single_surface_with_ui(
            context,
            input,
            SURFACE,
            &mut self.panes,
            |ui| {
                // Toolbar mutations are queued and applied in `frame_finished`.
                draw_toolbar(ui, controls, visibility, new_tab_anchor);
            },
        )?;
        self.last_response = Some(response);
        Ok(output)
    }

    fn frame_finished(&mut self, _: NativeFrameReport<'_>) -> AppResult<FrameControl> {
        let changed = self.apply_pending_actions()?;
        Ok(if changed {
            FrameControl::Repaint
        } else {
            FrameControl::Continue
        })
    }
}

struct DocumentBuffer {
    title: String,
    text: String,
}

struct IdePanes {
    documents: BTreeMap<ItemId, DocumentBuffer>,
    last_active_doc: Option<ItemId>,
    queued_new_tabs: Vec<ItemId>,
    events: VecDeque<String>,
}

impl IdePanes {
    fn new() -> Self {
        Self {
            documents: BTreeMap::from([(
                FIRST_DOC,
                DocumentBuffer {
                    title: "main.rs".to_owned(),
                    text: "// Drag an Asset chip from the Assets pane onto this editor.\n// Press Ctrl+Tab / Ctrl+Shift+Tab while focused to cycle tabs.\nfn main() {}\n"
                        .to_owned(),
                },
            )]),
            last_active_doc: Some(FIRST_DOC),
            queued_new_tabs: Vec::new(),
            events: VecDeque::new(),
        }
    }

    fn name(&self, item: ItemId) -> String {
        match item {
            ASSETS => "Assets".to_owned(),
            OVERVIEW => "Overview".to_owned(),
            ACTIVITY_LOG => "Activity Log".to_owned(),
            other => self
                .documents
                .get(&other)
                .map_or_else(|| format!("Pane {}", other.get()), |doc| doc.title.clone()),
        }
    }

    fn push_log(&mut self, entry: String) {
        if self.events.len() >= MAX_LOG_ENTRIES {
            self.events.pop_front();
        }
        self.events.push_back(entry);
    }
}

impl PaneView for IdePanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        match item {
            ASSETS => Some("Assets".into()),
            OVERVIEW => Some("Overview".into()),
            ACTIVITY_LOG => Some("Activity Log".into()),
            other => self
                .documents
                .get(&other)
                .map(|doc| doc.title.as_str().into()),
        }
    }

    fn sizing(&self, item: ItemId) -> PaneSizing {
        let builder = PaneSizing::builder().min_width(150.0).min_height(100.0);
        let builder = match item {
            ASSETS => builder.preferred_width(220.0),
            ACTIVITY_LOG => builder.preferred_height(150.0),
            _ => builder,
        };
        builder.build().expect("static pane sizing is valid")
    }

    fn tab_bar_layout(&self, _bar: TabBarId, _ui: &Ui) -> TabBarLayout {
        TabBarLayout {
            after_tabs_width: 24.0,
            ..Default::default()
        }
    }

    fn tab_bar_ui(&mut self, ui: &mut Ui, bar: TabBar<'_>, slot: TabBarSlot) {
        if slot == TabBarSlot::AfterTabs
            && let Some(anchor) = bar.items().last()
            && ui
                .small_button("+")
                .on_hover_text("Open a new document tab in this group")
                .clicked()
        {
            self.queued_new_tabs.push(anchor);
        }
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        match item {
            ASSETS => {
                ui.strong("Draggable Assets");
                ui.small("Drag a chip into any document pane — docking stays out of the way.");
                ui.add_space(6.0);
                let mut pane_response = PaneResponse::default();
                for (name, snippet) in SAMPLE_ASSETS {
                    let chip_id = egui::Id::unique(("asset-chip", name));
                    let response = ui
                        .dnd_drag_source(chip_id, AssetDragPayload { name, snippet }, |ui| {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.label(format!(":: {name}"));
                            });
                        })
                        .response;
                    pane_response = pane_response.with_content(&response);
                }
                pane_response
            }
            OVERVIEW => {
                ui.heading("Protected Anchor Pane");
                ui.label(
                    "When 'Protect Overview' is checked in Policy, this pane uses DockItemRule to disable closing, undocking, and tab reordering.",
                );
                ui.separator();
                ui.label("Main root uses RootAllocation::PreferCentral so the editor area absorbs resize deltas first.");
                PaneResponse::default()
            }
            ACTIVITY_LOG => {
                let scroll = egui::ScrollArea::vertical()
                    .id_salt("ide-activity-log")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for entry in &self.events {
                            ui.monospace(entry);
                        }
                    });
                PaneResponse::default().with_scroll_area(ui, &scroll)
            }
            other => {
                let Some(doc) = self.documents.get_mut(&other) else {
                    return PaneResponse::default();
                };
                self.last_active_doc = Some(other);
                let editor_id = doc_focus_id(other);
                let mut dropped_asset = None;
                let (zone_response, dropped) =
                    ui.dnd_drop_zone::<AssetDragPayload, _>(egui::Frame::NONE, |ui| {
                        let scroll = egui::ScrollArea::both()
                            .id_salt(("doc-scroll", other))
                            .show(ui, |ui| {
                                egui_dockspace::pane_text_edit(
                                    ui,
                                    editor_id,
                                    egui::TextEdit::multiline(&mut doc.text)
                                        .code_editor()
                                        .desired_width(f32::INFINITY),
                                )
                            });
                        let inner = scroll.inner.clone();
                        (scroll, inner)
                    });
                if let Some(payload) = dropped {
                    doc.text.push_str(&format!("\n{}\n", payload.snippet));
                    dropped_asset = Some(payload.name);
                }
                let (scroll, editor_response) = zone_response.inner;
                let response = PaneResponse::default()
                    .with_scroll_area(ui, &scroll)
                    .with_content(&editor_response)
                    .with_content(&zone_response.response);
                if let Some(name) = dropped_asset {
                    self.push_log(format!("Dropped asset '{name}' into {}", self.name(other)));
                }
                response
            }
        }
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        self.documents
            .contains_key(&item)
            .then(|| doc_focus_id(item))
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        self.focus_target(item).is_some()
    }
}

fn doc_focus_id(item: ItemId) -> egui::Id {
    egui::Id::unique(("ide-doc-editor", item))
}

fn default_layout() -> DockspaceLayout {
    let center = DockspaceNode::split(
        DockspaceAxis::Vertical,
        [
            (DockspaceNode::central_tabs([OVERVIEW, FIRST_DOC]), 0.72),
            (DockspaceNode::tabs([ACTIVITY_LOG]), 0.28),
        ],
    )
    .expect("vertical center split is valid");
    let root = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [(DockspaceNode::tabs([ASSETS]), 0.22), (center, 0.78)],
    )
    .expect("horizontal main split is valid");
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(MAIN_ROOT, root).allocation(RootAllocation::PreferCentral),
    )])
    .expect("static IDE layout is valid")
}

fn build_policy(
    auto_hide_single: bool,
    protect_overview: bool,
    lock_vertical_split: bool,
) -> DockPolicy {
    let mut policy = DockPolicy::default();
    policy.set_tab_bar(TabBarPolicy::new(
        if auto_hide_single {
            TabBarVisibility::AutoHideSingle
        } else {
            TabBarVisibility::Visible
        },
        TabBarInteraction::Enabled,
    ));
    if protect_overview {
        let mut overview = DockItemRule::new();
        overview.set_close_capability(Some(CloseCapability::Disabled));
        overview.set_allow_undocking(false);
        overview.set_allow_tab_reorder(false);
        policy.set_item_rule(OVERVIEW, overview);
    }
    policy.set_allow_root_resize_axis(MAIN_ROOT, DockspaceAxis::Vertical, !lock_vertical_split);
    policy
}

fn build_input(
    docking_mode: DockingModeChoice,
    middle_click_closes_tab: bool,
    double_click_equalize: bool,
) -> DockInputConfig {
    DockInputConfig {
        docking: docking_mode.to_activation(),
        middle_click_closes_tab,
        splitter_double_click: Some(if double_click_equalize {
            SplitterDoubleClick::Equalize
        } else {
            SplitterDoubleClick::Preferred
        }),
        next_tab: Some(KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)),
        previous_tab: Some(KeyboardShortcut::new(
            Modifiers::CTRL | Modifiers::SHIFT,
            Key::Tab,
        )),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_dockspace::{DockspaceCloseRequestStatus, SurfacePresentationResult};

    fn run_presented_frame(context: &Context, app: &mut IdeApp, events: Vec<egui::Event>) {
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 780.0),
            )),
            events,
            focused: true,
            ..Default::default()
        };
        let output = app.update(context, input).expect("IDE frame succeeds");
        let (mut full_output, receipt) = output.into_parts();
        full_output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);
        let _ = app.apply_pending_actions().expect("pending actions commit");
    }

    #[test]
    fn dynamic_new_tab_opens_and_records_lifecycle_event() {
        let context = Context::default();
        let mut app = IdeApp::new();
        run_presented_frame(&context, &mut app, Vec::new());

        app.panes.queued_new_tabs.push(FIRST_DOC);
        assert!(app.apply_pending_actions().unwrap());

        let new_item = ItemId::new(FIRST_DYNAMIC_ID);
        assert!(app.dockspace.view().item(new_item).is_some());
        assert!(
            app.panes
                .events
                .iter()
                .any(|entry| entry.contains("ItemOpened"))
        );
    }

    #[test]
    fn visibility_toggle_hides_and_restores_without_discarding_pane_state() {
        let mut app = IdeApp::new();
        app.controls
            .pending_actions
            .push(PendingAction::ToggleVisibility(ASSETS, false));
        assert!(app.apply_pending_actions().unwrap());
        assert!(!app.dockspace.view().item(ASSETS).unwrap().is_visible());

        app.controls
            .pending_actions
            .push(PendingAction::ToggleVisibility(ASSETS, true));
        assert!(app.apply_pending_actions().unwrap());
        assert!(app.dockspace.view().item(ASSETS).unwrap().is_visible());
    }

    #[test]
    fn protected_overview_rejects_close_and_auto_hide_single_applies() {
        let mut app = IdeApp::new();
        let close = app
            .dockspace
            .request_close_item_current(OVERVIEW)
            .expect("close request evaluates");
        assert!(matches!(
            close.status(),
            DockspaceCloseRequestStatus::Rejected(_)
        ));

        app.controls.auto_hide_single = true;
        app.controls
            .pending_actions
            .push(PendingAction::ApplyPolicy);
        assert!(app.apply_pending_actions().unwrap());
        assert_eq!(
            app.dockspace.policy().tab_bar().visibility(),
            TabBarVisibility::AutoHideSingle
        );
    }

    #[test]
    fn ctrl_tab_cycles_tabs_in_focused_document_group() {
        let context = Context::default();
        let mut app = IdeApp::new();
        let _ = app.dockspace.select_item_current(FIRST_DOC).unwrap();
        run_presented_frame(&context, &mut app, Vec::new());
        run_presented_frame(&context, &mut app, Vec::new());
        context.memory_mut(|memory| memory.request_focus(doc_focus_id(FIRST_DOC)));
        run_presented_frame(&context, &mut app, Vec::new());

        run_presented_frame(
            &context,
            &mut app,
            vec![egui::Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::CTRL,
            }],
        );
        run_presented_frame(&context, &mut app, Vec::new());

        let selected = app
            .dockspace
            .view()
            .item(OVERVIEW)
            .unwrap()
            .tabs()
            .unwrap()
            .selected()
            .and_then(|tab| tab.item());
        assert_eq!(selected, Some(OVERVIEW));
    }

    #[test]
    fn dropped_os_files_record_and_insert_into_active_document() {
        #[derive(Debug)]
        struct TestDroppedFile(std::path::PathBuf);

        impl egui::DroppedFile for TestDroppedFile {
            fn path(&self) -> &std::path::Path {
                &self.0
            }

            fn bytes(&self) -> Result<Vec<u8>, String> {
                Ok(Vec::new())
            }
        }

        let context = Context::default();
        let mut app = IdeApp::new();
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 780.0),
            )),
            ..Default::default()
        };
        raw.dropped_files.push(std::sync::Arc::new(TestDroppedFile(
            std::path::PathBuf::from("scene.gltf"),
        )));
        let output = app.update(&context, raw).unwrap();
        let (mut full_output, receipt) = output.into_parts();
        full_output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);

        assert!(
            app.panes.documents[&FIRST_DOC]
                .text
                .contains("Dropped file: scene.gltf")
        );
    }
}
