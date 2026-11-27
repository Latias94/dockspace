#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use egui_dockspace_native::NativeCommand;
use std::collections::BTreeMap;

use dockspace::geometry::{LogicalRect, PhysicalRect};
use dockspace::model::{
    DockMoveSource, DockMoveTarget, DockspaceActionOutcome, DockspaceAxis,
    DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, FloatingPresentationId, ItemId, NativeWindowPlacement, RootId,
    SurfaceId, TabKey,
};
use dockspace::policy::{CloseCapability, DockPolicy};
use dockspace::runtime::DockspaceSession;
#[cfg(feature = "serde")]
use dockspace::runtime::{DockspaceDocumentBootstrap, DockspaceDocumentId};
use eframe::egui;
use egui_dockspace::{
    DockDensity, DockStyle, DockspaceActionStatus, PaneResponse, PaneSizing, PaneView,
};
use egui_dockspace_native::{NativeActionReceipt, NativeActionStatus, NativeDockspaceApp};

const SURFACE: SurfaceId = SurfaceId::new(1);
const MAIN_ROOT: RootId = RootId::new(1);
const INSPECTOR_ROOT: RootId = RootId::new(2);
const INSPECTOR_FLOATING: FloatingPresentationId = FloatingPresentationId::new(1);
const OUTLINE: ItemId = ItemId::new(1);
const EDITOR: ItemId = ItemId::new(2);
const PREVIEW: ItemId = ItemId::new(3);
const INSPECTOR: ItemId = ItemId::new(4);
const CONSOLE: ItemId = ItemId::new(5);
const HISTORY: ItemId = ItemId::new(6);
const PROFILER: ItemId = ItemId::new(7);
const ASSETS: ItemId = ItemId::new(8);
#[cfg(feature = "serde")]
const DOCUMENT_ID: DockspaceDocumentId = DockspaceDocumentId::from_bytes(*b"native-editor-v1");

#[path = "basic/command_smoke.rs"]
mod command_smoke;
#[path = "basic/docking_smoke/mod.rs"]
mod docking_smoke;
#[path = "basic/layout.rs"]
mod layout;
#[path = "basic/operation_log.rs"]
mod operation_log;
#[path = "basic/smoke.rs"]
mod smoke;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let operation_log = operation_log::OperationLog::from_args()
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let mut policy = DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    policy.set_close_capability(CloseCapability::DeferredAllowed);
    let command_smoke = std::env::args().any(|argument| argument == "--command-smoke");
    let command_smoke_completed =
        command_smoke.then(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let smoke = std::env::args().any(|argument| argument == "--smoke");
    let docking_smoke = std::env::args().any(|argument| argument == "--docking-smoke");
    let showcase = std::env::args().any(|argument| argument == "--showcase");
    // The finite journeys deliberately exercise their own initial geometry,
    // including an off-screen contained window for BringContainedIntoView.
    // The interactive example performs one initial fit after its real host
    // surface has been presented.
    let initial_action = if command_smoke || smoke || docking_smoke {
        ExampleActionState::Idle
    } else {
        ExampleActionState::AwaitingInspectorPlacement("Initial layout")
    };
    let mut panes = ExamplePanes::new();
    // The command journey verifies data retention without accepting desktop edits.
    // The interactive example and OS-input journey keep their editable content.
    panes.text_editable = !command_smoke;
    let default_layout = if command_smoke {
        command_smoke::layout()
    } else if docking_smoke
        && std::env::var("EGUI_DOCKSPACE_DOCKING_CASE").as_deref() == Ok("native-preview-first")
    {
        // Exercise automatic native retirement without a persistent central area.
        editor_layout_with_inspector(false, SURFACE, false)
    } else if showcase {
        showcase_layout()
    } else {
        example_layout()
    };
    let style = DockStyle::default();
    let presentation = style
        .presentation_config()
        .expect("the default chrome is valid");
    #[cfg(feature = "serde")]
    let session = {
        let mut bootstrap = DockspaceDocumentBootstrap::new(DOCUMENT_ID);
        // BTreeMap visits the original ItemIds in their allocation order, 1..=8.
        for (&item, pane) in &panes.panes {
            assert_eq!(
                bootstrap.ensure_item(pane.key).expect("the eight keys fit"),
                item,
            );
        }
        DockspaceSession::from_persistent_layout_with_presentation_config(
            default_layout.clone(),
            policy,
            presentation,
            bootstrap,
        )
        .expect("the static native document is valid")
    };
    #[cfg(not(feature = "serde"))]
    let session = DockspaceSession::from_layout_with_presentation_config(
        default_layout.clone(),
        policy,
        presentation,
    )
    .expect("the static native example layout is valid");
    let mut dockspace = NativeDockspaceApp::new(
        egui::Id::unique("native-dockspace-example"),
        session,
        SURFACE,
        panes,
        style,
    )
    .expect("the native coordinator initializes");
    if let Some(log) = &operation_log {
        log.record(
            "initial-layout",
            dockspace.with_view(|view| format!("{view:?}")),
        );
        let log = log.clone();
        dockspace = dockspace.with_notification_handler(move |notification, runtime| {
            log.record("commit", format!("{notification:?}"));
            log.record(
                "layout-current",
                runtime.with_view(|view| format!("{view:?}")),
            );
        });
    }
    let native_host = dockspace.native_host_handler();
    let app = NativeExampleApp {
        dockspace,
        default_layout,
        #[cfg(feature = "serde")]
        presets: BTreeMap::new(),
        #[cfg(feature = "serde")]
        preset_name: "Working".to_owned(),
        action: initial_action,
        action_receipt: None,
        font_size: 13.0,
        density: DockDensity::default(),
        showcase,
        status: None,
        smoke: smoke.then(smoke::EditorSmoke::new),
        docking_smoke: docking_smoke.then(docking_smoke::DockingSmoke::new),
        command_smoke: command_smoke_completed
            .as_ref()
            .map(|completed| command_smoke::CommandSmoke::new(std::sync::Arc::clone(completed))),
        operation_log,
    };
    let options = eframe::NativeOptions {
        viewport: {
            if showcase {
                egui::ViewportBuilder::default()
                    .with_inner_size([800.0, 560.0])
                    .with_position([30.0, 90.0])
            } else {
                let viewport = egui::ViewportBuilder::default().with_inner_size([1280.0, 820.0]);
                if docking_smoke
                    && std::env::var("EGUI_DOCKSPACE_DOCKING_CASE")
                        .is_ok_and(|case| case.starts_with("native-"))
                {
                    viewport.with_position([40.0, 80.0])
                } else {
                    viewport
                }
            }
        },
        native_host: Some(native_host),
        ..Default::default()
    };
    eframe::run_native(
        "Dockspace native editor",
        options,
        Box::new(move |creation| {
            creation.egui_ctx.set_theme(if showcase {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            if let Some(log) = &app.operation_log {
                log.install(&creation.egui_ctx);
            }
            if let Some(smoke) = &app.docking_smoke {
                smoke.install(&creation.egui_ctx);
                app.dockspace
                    .with_panes_mut(|panes| panes.probe = Some(smoke.probe()));
            }
            Ok(Box::new(app))
        }),
    )?;
    if command_smoke_completed
        .is_some_and(|completed| !completed.load(std::sync::atomic::Ordering::Relaxed))
    {
        return Err(std::io::Error::other(
            "native editor command smoke failed; see preceding diagnostics",
        )
        .into());
    }
    Ok(())
}

struct NativeExampleApp {
    dockspace: NativeDockspaceApp<ExamplePanes>,
    default_layout: DockspaceLayout,
    #[cfg(feature = "serde")]
    presets: BTreeMap<String, layout::NamedPreset>,
    #[cfg(feature = "serde")]
    preset_name: String,
    action: ExampleActionState,
    action_receipt: Option<NativeActionReceipt>,
    font_size: f32,
    density: DockDensity,
    showcase: bool,
    status: Option<String>,
    smoke: Option<smoke::EditorSmoke>,
    docking_smoke: Option<docking_smoke::DockingSmoke>,
    command_smoke: Option<command_smoke::CommandSmoke>,
    operation_log: Option<operation_log::OperationLog>,
}

#[derive(Clone, Copy, Default)]
enum ExampleActionState {
    #[default]
    Idle,
    AwaitingTearOffOutcome,
    AwaitingFirstPresentation(SurfaceId),
    AwaitingDockBackOutcome(RootId),
    AwaitingLayoutOutcome(&'static str),
    AwaitingFreshPresentation(&'static str),
    AwaitingInspectorPlacement(&'static str),
    AwaitingSmokeOutcome(&'static str),
}

impl ExampleActionState {
    const fn is_pending(self) -> bool {
        !matches!(self, Self::Idle)
    }
}

impl eframe::App for NativeExampleApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if matches!(
            self.action,
            ExampleActionState::AwaitingTearOffOutcome
                | ExampleActionState::AwaitingDockBackOutcome(_)
                | ExampleActionState::AwaitingLayoutOutcome(_)
        ) {
            self.collect_action_status();
        }
        egui::Panel::top("native-example-controls").show(ui, |ui| {
            self.show_controls(ui);
        });
        <NativeDockspaceApp<ExamplePanes> as eframe::App>::ui(&mut self.dockspace, ui, frame);
        if let Some(log) = &self.operation_log {
            log.observe(&self.dockspace);
        }
        if let Some(mut smoke) = self.smoke.take() {
            smoke.advance(self, ui.ctx());
            self.smoke = Some(smoke);
        }
        if let Some(mut smoke) = self.docking_smoke.take() {
            smoke.advance(self, ui.ctx());
            self.docking_smoke = Some(smoke);
        }
        if let Some(mut smoke) = self.command_smoke.take() {
            smoke.advance(&self.dockspace, ui.ctx());
            self.command_smoke = Some(smoke);
        }
    }
}

impl NativeExampleApp {
    fn show_controls(&mut self, ui: &mut egui::Ui) {
        self.finish_presentation_wait();
        let root_surface = self.dockspace.root_surface();
        let inspector = self.dockspace.with_view(|view| {
            view.item(INSPECTOR)
                .map(|item| InspectorLocation::new(item, root_surface))
        });
        let root_presented = self.dockspace.is_surface_presented(root_surface);

        ui.horizontal_wrapped(|ui| {
            ui.strong("Dockspace native editor");
            ui.separator();
            ui.add_enabled_ui(self.controls_available(), |ui| {
                egui::containers::menu::MenuButton::new("Appearance")
                    .config(
                        egui::containers::menu::MenuConfig::new()
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                    )
                    .ui(ui, |ui| {
                        let previous_theme = ui.ctx().options(|options| options.theme_preference);
                        let previous_density = self.density;
                        egui::widgets::global_theme_preference_buttons(ui);
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.density, DockDensity::Compact, "Compact");
                            ui.selectable_value(
                                &mut self.density,
                                DockDensity::Comfortable,
                                "Comfortable",
                            );
                        });
                        let font_changed = ui
                            .add(
                                egui::DragValue::new(&mut self.font_size)
                                    .range(10.0..=36.0)
                                    .prefix("Font "),
                            )
                            .changed();
                        if font_changed {
                            let font = egui::FontId::proportional(self.font_size);
                            ui.ctx().all_styles_mut(|style| {
                                style.override_font_id = Some(font.clone())
                            });
                        }
                        if font_changed
                            || self.density != previous_density
                            || ui.ctx().options(|options| options.theme_preference)
                                != previous_theme
                        {
                            ui.set_style(ui.ctx().global_style());
                            let result = self
                                .dockspace
                                .request_style(DockStyle::from_egui_with_density(ui, self.density));
                            self.begin_layout_action("Appearance", result);
                        }
                    });
            });
            if let Some(location) = inspector.filter(|location| location.is_native_child()) {
                if ui
                    .add_enabled(
                        self.controls_available() && !self.dockspace.arrangement_locked(),
                        egui::Button::new("Dock Inspector Back"),
                    )
                    .clicked()
                {
                    self.request_root_dock_back(location.root);
                }
            } else {
                let contained_root = inspector.filter(|location| location.is_contained());
                let enabled = self.controls_available()
                    && !self.dockspace.arrangement_locked()
                    && root_presented
                    && contained_root.is_some();
                if ui
                    .add_enabled(enabled, egui::Button::new("Open Inspector Window"))
                    .on_disabled_hover_text(
                        "Move the live contained Inspector and Preview group into an OS window.",
                    )
                    .clicked()
                    && let Some(location) = contained_root
                {
                    self.request_inspector_window(location.root);
                }
                if !root_presented {
                    ui.label("Waiting for the root output to be presented…");
                }
            }
            self.show_layout_bar_items(ui, inspector, root_presented);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                if ui.selectable_label(dark, "Dark").clicked() {
                    ui.ctx().set_theme(egui::Theme::Dark);
                }
                if ui.selectable_label(!dark, "Light").clicked() {
                    ui.ctx().set_theme(egui::Theme::Light);
                }
            });
        });
        if self.action.is_pending() {
            self.show_pending_controls(ui);
        }
        self.show_close_decisions(ui);
        if !self.showcase
            && let Some(status) = &self.status
        {
            ui.small(format!("Last action: {status}"));
        }
    }

    fn show_pending_controls(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spinner();
            match self.action {
                ExampleActionState::AwaitingDockBackOutcome(_) => {
                    ui.label("Returning the window group to the main dockspace…");
                }
                ExampleActionState::AwaitingLayoutOutcome(operation)
                | ExampleActionState::AwaitingSmokeOutcome(operation) => {
                    ui.label(format!("{operation}: waiting for the committed result…"));
                }
                ExampleActionState::AwaitingFreshPresentation(operation) => {
                    ui.label(format!(
                        "{operation}: layout restored; waiting for the main window and each child window to be ready for editing…"
                    ));
                }
                ExampleActionState::AwaitingInspectorPlacement(operation) => {
                    ui.label(format!(
                        "{operation}: waiting for the main window to present before fitting Inspector into view…"
                    ));
                }
                _ => {
                    ui.label("Creating and presenting the native child window…");
                }
            }
        });
    }

    fn show_capability_status(
        &self,
        ui: &mut egui::Ui,
        inspector: Option<InspectorLocation>,
        root_presented: bool,
    ) {
        let locked = self.dockspace.arrangement_locked();
        let local = if root_presented && locked {
            "selection ready; arrangement locked"
        } else if root_presented {
            "ready"
        } else {
            "waiting for the root surface"
        };
        let contained = if locked {
            "arrangement locked"
        } else if inspector.is_some_and(InspectorLocation::is_contained) {
            "ready"
        } else {
            "not currently presented"
        };
        let native = if self.dockspace.error_kind().is_some() {
            "unavailable (runtime stopped)"
        } else if locked {
            "arrangement locked"
        } else {
            "programmatic window hosting ready; physical input requires backend facts"
        };
        ui.small(format!(
            "Capability status — local select/reorder: {local}; contained move/resize: {contained}; native promotion: {native}."
        ));
    }

    fn show_presentation_state(
        &self,
        ui: &mut egui::Ui,
        inspector: Option<InspectorLocation>,
        root_presented: bool,
    ) {
        let state = match inspector {
            Some(location) if location.is_contained() && root_presented => {
                "Inspector presentation: contained floating window in the root egui surface."
            }
            Some(location) if location.is_contained() => {
                "Inspector presentation: contained, waiting for the root surface output."
            }
            Some(location) if location.is_native_child() => {
                if self.dockspace.is_surface_presented(location.surface) {
                    "Inspector presentation: live native child OS window; closing it recovers the previous presentation."
                } else {
                    "Inspector presentation: native child transition is not live yet."
                }
            }
            Some(_) if root_presented => "Inspector presentation: docked in the root surface.",
            Some(_) => "Inspector presentation: root surface output is not live yet.",
            None => "Inspector presentation: pane is closed.",
        };
        ui.small(state);
    }

    fn show_walkthrough(&self, ui: &mut egui::Ui) {
        ui.label(
            "Drag tabs to merge or split panes. Move the floating Inspector by its title grip.",
        );
        ui.label(
            "Open Inspector Window creates an OS window; Dock Inspector Back returns it. \
             Closing that window also returns its panes without losing edits.",
        );
        ui.label(
            "Use Pane inventory to reopen closed panes. Default layout restores the arrangement and keeps your data.",
        );
        ui.label(
            "Lock arrangement prevents layout changes while keeping pane selection and editing available.",
        );
        #[cfg(feature = "serde")]
        ui.label(
            "Named presets last for this session and include native windows. Restoring a preset keeps pane data.",
        );
        ui.small(
            "Cross-window dragging depends on the platform. Use window commands when dragging is unavailable.",
        );
    }

    fn request_inspector_window(&mut self, root: RootId) {
        let bounds = if self.showcase {
            PhysicalRect::new(852.0, 114.0, 460.0, 420.0)
        } else {
            PhysicalRect::new(760.0, 120.0, 440.0, 340.0)
        }
        .expect("the static child placement is valid");
        self.request_native_window(root, bounds);
    }

    fn request_native_window(&mut self, root: RootId, bounds: PhysicalRect) {
        let placement = NativeWindowPlacement::new(bounds);
        match self.dockspace.request_command(NativeCommand::Move {
            source: DockMoveSource::Root(root),
            target: DockMoveTarget::Native {
                placement: Some(placement),
            },
        }) {
            Ok(receipt) => {
                self.action_receipt = Some(receipt);
                self.action = ExampleActionState::AwaitingTearOffOutcome;
                self.status = None;
            }
            Err(error) => self.status = Some(format!("Could not open native window: {error}")),
        }
    }

    fn request_root_dock_back(&mut self, root: RootId) {
        match self
            .dockspace
            .request_command(NativeCommand::DockRootBack { root })
        {
            Ok(receipt) => {
                self.action_receipt = Some(receipt);
                self.action = ExampleActionState::AwaitingDockBackOutcome(root);
                self.status = None;
            }
            Err(error) => {
                self.status = Some(format!("Could not dock the window group back: {error}"))
            }
        }
    }

    fn collect_action_status(&mut self) {
        let Some(status) = poll_receipt(&mut self.action_receipt) else {
            return;
        };
        if let Some(log) = &self.operation_log {
            log.record("action-result", format!("{status:?}"));
        }
        match (self.action, status) {
            (
                ExampleActionState::AwaitingTearOffOutcome,
                Ok(NativeActionStatus::Dock(DockspaceActionStatus::Requested {
                    outcome:
                        DockspaceActionOutcome::NativeRootTearOffRequested { target_surface, .. },
                    transition: _,
                })),
            ) => {
                self.action = ExampleActionState::AwaitingFirstPresentation(target_surface);
                self.status = Some("Opening Inspector window…".to_owned());
            }
            (
                ExampleActionState::AwaitingDockBackOutcome(expected_root),
                Ok(NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                    DockspaceActionOutcome::RootDocked {
                        root,
                        changed: true,
                        ..
                    },
                ))),
            ) if root == expected_root => {
                self.action = ExampleActionState::Idle;
                self.status = Some("Panes returned to the main window.".to_owned());
            }
            (_, Ok(status)) => {
                self.record_action_status(&status);
            }
            (_, Err(error)) => {
                self.action = ExampleActionState::Idle;
                self.status = Some(format!("The native action was interrupted: {error}"));
            }
        }
    }

    fn record_action_status(&mut self, status: &NativeActionStatus) {
        self.action = match status {
            NativeActionStatus::StyleChanged { .. } => {
                ExampleActionState::AwaitingFreshPresentation("Appearance")
            }
            NativeActionStatus::LayoutReset { .. } => {
                // The scripted editor smoke owns its own Default-layout
                // barrier. The interactive example additionally fits the
                // contained Inspector once against the first real surface
                // bounds, so the toolbar height never becomes a hard-coded
                // placement assumption.
                match self.action {
                    ExampleActionState::AwaitingSmokeOutcome(_) => {
                        ExampleActionState::AwaitingFreshPresentation("Default layout")
                    }
                    _ => ExampleActionState::AwaitingInspectorPlacement("Default layout"),
                }
            }
            NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                DockspaceActionOutcome::ContainedBoundsUpdated { .. },
            )) if matches!(
                self.action,
                ExampleActionState::AwaitingLayoutOutcome("Inspector placement")
            ) =>
            {
                ExampleActionState::AwaitingFreshPresentation("Inspector placement")
            }
            #[cfg(feature = "serde")]
            NativeActionStatus::DocumentRestored { .. } => {
                ExampleActionState::AwaitingFreshPresentation("Document restore")
            }
            _ => ExampleActionState::Idle,
        };
        self.status = Some(
            self.dockspace
                .with_panes(|panes| layout::describe_status(status, panes)),
        );
    }

    fn finish_presentation_wait(&mut self) {
        match self.action {
            ExampleActionState::AwaitingFirstPresentation(surface)
                if self.dockspace.is_surface_presented(surface) =>
            {
                self.action = ExampleActionState::Idle;
                self.status = Some("Inspector window ready.".to_owned());
            }
            ExampleActionState::AwaitingFirstPresentation(surface)
                if self.dockspace.is_quiescent()
                    && self
                        .dockspace
                        .with_view(|view| view.surface(surface).is_none()) =>
            {
                self.action = ExampleActionState::Idle;
                self.status = Some(
                    "Native child is no longer available; its panes remain in the dockspace."
                        .to_owned(),
                );
            }
            ExampleActionState::AwaitingFreshPresentation(operation)
                if self.all_surfaces_presented() && self.dockspace.is_quiescent() =>
            {
                self.action = ExampleActionState::Idle;
                self.status = Some(format!("{operation} ready."));
            }
            ExampleActionState::AwaitingInspectorPlacement(operation)
                if self.all_surfaces_presented() && self.dockspace.is_quiescent() =>
            {
                let host = self.dockspace.root_surface();
                let inspector_is_contained = self.dockspace.with_view(|view| {
                    view.item(INSPECTOR)
                        .is_some_and(|item| item.surface() == host && item.contained().is_some())
                });
                self.action = ExampleActionState::Idle;
                self.status = Some(format!("{operation} ready."));
                if inspector_is_contained {
                    let request = self
                        .dockspace
                        .request_command(NativeCommand::BringContainedIntoView { item: INSPECTOR });
                    self.begin_layout_action("Inspector placement", request);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct InspectorLocation {
    root: RootId,
    surface: SurfaceId,
    host_surface: SurfaceId,
    contained: bool,
}

impl InspectorLocation {
    fn new(item: dockspace::model::DockspaceItemView<'_>, host_surface: SurfaceId) -> Self {
        Self {
            root: item.root(),
            surface: item.surface(),
            host_surface,
            contained: item.contained().is_some(),
        }
    }

    fn is_contained(self) -> bool {
        self.surface == self.host_surface && self.contained
    }

    fn is_native_child(self) -> bool {
        self.surface != self.host_surface
    }
}

fn example_layout() -> DockspaceLayout {
    editor_layout(false)
}

fn showcase_layout() -> DockspaceLayout {
    let center = DockspaceNode::split(
        DockspaceAxis::Vertical,
        [
            (
                DockspaceNode::central_tabs([EDITOR, HISTORY, PROFILER]),
                0.72,
            ),
            (DockspaceNode::tabs([CONSOLE]), 0.28),
        ],
    )
    .expect("the showcase editor and console split is valid");
    let root = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([OUTLINE, ASSETS]), 0.26),
            (center, 0.74),
        ],
    )
    .expect("the showcase split is valid");
    let inspector_tabs =
        DockspaceNode::central_tabs_with_selection([PREVIEW, INSPECTOR], Some(INSPECTOR))
            .expect("the Inspector starts selected");
    let inspector = DockspaceContainedLayout::new(
        INSPECTOR_FLOATING,
        DockspaceRootLayout::new(INSPECTOR_ROOT, inspector_tabs),
        LogicalRect::new(470.0, 110.0, 310.0, 330.0).expect("the showcase contained rect is valid"),
    );
    let surface = DockspaceSurfaceLayout::new(SURFACE, DockspaceRootLayout::new(MAIN_ROOT, root))
        .with_contained(inspector);
    DockspaceLayout::new([surface]).expect("the showcase surface is valid")
}

fn editor_layout(separate_assets: bool) -> DockspaceLayout {
    editor_layout_on_surface(separate_assets, SURFACE)
}

fn editor_layout_on_surface(separate_assets: bool, host_surface: SurfaceId) -> DockspaceLayout {
    editor_layout_with_inspector(separate_assets, host_surface, true)
}

fn editor_layout_with_inspector(
    separate_assets: bool,
    host_surface: SurfaceId,
    central_inspector: bool,
) -> DockspaceLayout {
    let center = DockspaceNode::split(
        DockspaceAxis::Vertical,
        [
            (
                DockspaceNode::central_tabs([EDITOR, HISTORY, PROFILER]),
                0.74,
            ),
            (DockspaceNode::tabs([CONSOLE]), 0.26),
        ],
    )
    .expect("the editor and console split is valid");
    let root = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (
                DockspaceNode::tabs(if separate_assets {
                    vec![OUTLINE]
                } else {
                    vec![OUTLINE, ASSETS]
                }),
                0.20,
            ),
            (center, 0.80),
        ],
    )
    .expect("the static split is valid");
    let inspector_tabs = if central_inspector {
        DockspaceNode::central_tabs_with_selection([PREVIEW, INSPECTOR], Some(INSPECTOR))
    } else {
        DockspaceNode::tabs_with_selection([PREVIEW, INSPECTOR], Some(INSPECTOR))
    }
    .expect("the Inspector starts selected");
    let inspector = DockspaceContainedLayout::new(
        INSPECTOR_FLOATING,
        DockspaceRootLayout::new(INSPECTOR_ROOT, inspector_tabs),
        LogicalRect::new(850.0, 80.0, 360.0, 410.0).expect("the static contained rect is valid"),
    );
    let mut surface =
        DockspaceSurfaceLayout::new(host_surface, DockspaceRootLayout::new(MAIN_ROOT, root))
            .with_contained(inspector);
    if separate_assets {
        surface = surface.with_contained(DockspaceContainedLayout::new(
            FloatingPresentationId::new(2),
            DockspaceRootLayout::new(RootId::new(3), DockspaceNode::tabs([ASSETS])),
            LogicalRect::new(120.0, 100.0, 360.0, 300.0)
                .expect("the static Assets placement is valid"),
        ));
    }
    DockspaceLayout::new([surface]).expect("the static surface is valid")
}

struct ExamplePane {
    key: &'static str,
    title: &'static str,
    text: String,
    loaded: bool,
    missing_painted: bool,
    zoom: f32,
    pan: egui::Vec2,
    input: Option<(egui::ViewportId, egui::Rect, bool, f32)>,
    content: Option<(egui::Rect, f32)>,
    window_id: Option<u64>,
    text_color: egui::Color32,
}

impl ExamplePane {
    fn set_loaded(&mut self, loaded: bool) {
        self.loaded = loaded;
        self.missing_painted = false;
        self.input = None;
        self.content = None;
        self.window_id = None;
    }
}

struct ExamplePanes {
    panes: BTreeMap<ItemId, ExamplePane>,
    text_editable: bool,
    probe: Option<docking_smoke::EditorProbeHandle>,
}

impl ExamplePanes {
    fn new() -> Self {
        Self {
            text_editable: true,
            probe: None,
            panes: [
                (OUTLINE, "pane/outline", "Outline", "Scene\n  Camera [Perspective]\n  Directional Light\n  Player [Character]\n    Transform\n    MeshRenderer\n    Collider"),
                (EDITOR, "pane/document", "Document", "// src/scene.rs\n\npub fn update(scene: &mut Scene, dt: f32) {\n    let speed = 4.5 * dt;\n    scene.player.translate([speed, 0.0, 0.0]);\n    scene.camera.track(&scene.player);\n    scene.sync_viewport();\n}\n"),
                (PREVIEW, "pane/preview", "Preview", "Application-drawn scene"),
                (INSPECTOR, "pane/inspector", "Inspector", "Object:   Player\nLayer:    Characters\nPosition: [0.0, -60.0, 0.0]\nRotation: [0.0, 15.0, 0.0]\nMesh:     meshes/player.mesh\nMaterial: materials/default.mat\n"),
                (CONSOLE, "pane/console", "Console", "Live workspace log (scroll to inspect earlier entries):"),
                (HISTORY, "pane/history", "History", "Loaded scenes/main.scene\nSelected Player\nMoved Player to [0, -60, 0]\nUpdated directional light intensity\nSaved scene snapshot"),
                (PROFILER, "pane/profiler", "Profiler", "Frame budget · 8.8 / 16.7 ms (60 FPS)"),
                (ASSETS, "pane/assets", "Assets", "scenes/main.scene\nmeshes/player.mesh\nmaterials/default.mat\nshaders/grid.wgsl\ntextures/grid.png"),
            ]
            .into_iter()
            .map(|(item, key, title, text)| (item, ExamplePane {
                key,
                title,
                text: text.to_owned(),
                loaded: true,
                missing_painted: false,
                zoom: 1.0,
                pan: egui::Vec2::ZERO,
                input: None,
                content: None,
                window_id: None,
                text_color: egui::Color32::TRANSPARENT,
            }))
            .collect(),
        }
    }
}

fn pane_focus_id(item: ItemId) -> egui::Id {
    egui::Id::unique(("native-editor-content", item.get()))
}

fn paint_canvas(
    pane: &mut ExamplePane,
    ui: &mut egui::Ui,
    content: PaneResponse,
    probe: Option<&docking_smoke::EditorProbeHandle>,
) -> PaneResponse {
    let controls = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 140.0).clamp(80.0, 128.0);
        let reset = ui.button("Reset view");
        if reset.clicked() {
            pane.pan = egui::Vec2::ZERO;
            pane.zoom = 1.0;
        }
        let zoom = ui.add(egui::Slider::new(&mut pane.zoom, 0.5..=2.0).text("Zoom"));
        (reset, zoom)
    });
    let (rect, background) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let response = ui.interact(rect, pane_focus_id(PREVIEW), egui::Sense::click_and_drag());
    if response.dragged() {
        pane.pan += response.drag_delta();
    }
    if let Some(probe) = probe {
        probe.observe_canvas(ui, &response, pane.pan, &controls.inner.1, pane.zoom);
    }
    let dark = ui.visuals().dark_mode;
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let canvas_fill = if dark {
        egui::Color32::from_rgb(18, 21, 28)
    } else {
        egui::Color32::from_rgb(244, 246, 250)
    };
    painter.rect_filled(rect, 4.0, canvas_fill);
    let spacing = 32.0 * pane.zoom;
    let origin = rect.center() + pane.pan;
    let minor = egui::Stroke::new(
        1.0,
        if dark {
            egui::Color32::from_rgba_unmultiplied(148, 163, 184, 24)
        } else {
            egui::Color32::from_rgba_unmultiplied(100, 116, 139, 34)
        },
    );
    let x_axis = egui::Stroke::new(
        1.2,
        egui::Color32::from_rgba_unmultiplied(248, 113, 113, if dark { 100 } else { 130 }),
    );
    let y_axis = egui::Stroke::new(
        1.2,
        egui::Color32::from_rgba_unmultiplied(74, 222, 128, if dark { 100 } else { 130 }),
    );
    for index in -32..=32 {
        let offset = index as f32 * spacing;
        painter.line_segment(
            [
                egui::pos2(origin.x + offset, rect.top()),
                egui::pos2(origin.x + offset, rect.bottom()),
            ],
            if index == 0 { y_axis } else { minor },
        );
        painter.line_segment(
            [
                egui::pos2(rect.left(), origin.y + offset),
                egui::pos2(rect.right(), origin.y + offset),
            ],
            if index == 0 { x_axis } else { minor },
        );
    }
    let accent = egui::Color32::from_rgb(56, 152, 236);
    painter.circle_stroke(
        origin,
        34.0 * pane.zoom,
        egui::Stroke::new(2.0, accent.gamma_multiply(0.35)),
    );
    painter.circle_filled(origin, 28.0 * pane.zoom, accent);
    painter.text(
        origin,
        egui::Align2::CENTER_CENTER,
        "Player",
        egui::FontId::proportional((13.0 * pane.zoom).clamp(10.0, 20.0)),
        egui::Color32::WHITE,
    );
    painter.text(
        rect.center_bottom() - egui::vec2(0.0, 16.0),
        egui::Align2::CENTER_BOTTOM,
        "Application canvas · drag to pan",
        egui::FontId::proportional(13.0),
        if dark {
            egui::Color32::from_rgb(148, 163, 184)
        } else {
            egui::Color32::from_rgb(71, 85, 105)
        },
    );
    content
        .with_content(&controls.inner.0)
        .with_content(&controls.inner.1)
        .with_content(&controls.response)
        .with_content(&background)
        .with_content(&response)
}

impl PaneView for ExamplePanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        self.panes.get(&item).map(|pane| pane.title.into())
    }

    fn drop_preview_ui(&mut self, ui: &egui::Ui, preview: &egui_dockspace::DropPreview) {
        preview.paint(ui);
        if let Some(probe) = &self.probe {
            probe.observe_preview(ui, preview);
        }
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        let Some(pane) = self.panes.get_mut(&item) else {
            ui.label("Missing pane");
            return PaneResponse::default();
        };
        pane.input = None;
        pane.text_color = ui.visuals().text_color();
        pane.content = Some((ui.max_rect(), ui.ctx().pixels_per_point()));
        pane.window_id = None;
        pane.missing_painted = false;
        let mut content = PaneResponse::default();
        if !pane.loaded {
            let placeholder = ui.label("Recoverable missing content — temporarily unloaded.");
            pane.missing_painted = ui.is_rect_visible(placeholder.rect);
            let retained = ui.small(format!(
                "{} · title and application data retained",
                pane.key
            ));
            let reload = ui.button("Reload content");
            if reload.clicked() {
                pane.set_loaded(true);
                if let Some(target) = self.focus_target(item) {
                    ui.memory_mut(|memory| memory.request_focus(target));
                }
                ui.ctx().request_repaint();
            }
            return content
                .with_content(&placeholder)
                .with_content(&retained)
                .with_content(&reload);
        }
        if item == PREVIEW {
            content = paint_canvas(pane, ui, content, self.probe.as_ref());
        } else if item == CONSOLE {
            let description = ui.label(&pane.text);
            content = content.with_content(&description);
            let log = egui::ScrollArea::vertical()
                .id_salt(("native-editor-log", item.get()))
                .show(ui, |ui| {
                    let mut content = content.with_content_ids([ui.unique_id()]);
                    let messages = [
                        "[INFO ] Native host ready · Root surface presented",
                        "[SCENE] Loaded scenes/main.scene (3 entities)",
                        "[DOCK ] Floating Inspector group ready for native tear-off",
                        "[GPU  ] Frame pacing steady at 60 FPS (8.8 ms)",
                    ];
                    for line in 0..128 {
                        let entry = ui.monospace(format!(
                            "10:42:{:02} {}",
                            line % 60,
                            messages[line % messages.len()]
                        ));
                        content = content.with_content(&entry);
                    }
                    content
                });
            content = log.inner;
            if let Some(probe) = &self.probe {
                probe.observe_scroll(ui, log.id, log.state.clamped_offset().y);
            }
            // Only declare scroll controls actually registered in this pass.
            for id in [log.id.with("area"), log.id.with(1_usize)] {
                if ui
                    .ctx()
                    .viewport(|viewport| viewport.this_pass.widgets.get(id).is_some())
                {
                    content = content.with_content_ids([id]);
                }
            }
        } else if item == PROFILER {
            let description = ui.label(&pane.text);
            content = content.with_content(&description);
            for (stage, milliseconds) in [("Simulation", 2.6), ("Rendering", 5.4), ("UI", 0.8)] {
                let label = ui.label(format!("{stage}: {milliseconds:.1} ms"));
                content = content.with_content(&label);
                let progress = ui
                    .add(egui::ProgressBar::new(milliseconds / 16.67).desired_width(f32::INFINITY));
                content = content.with_content(&progress);
            }
        } else {
            let id = pane_focus_id(item);
            let dark = ui.visuals().dark_mode;
            let mut rust_layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                let job = highlight_native_rust(ui, buf.as_str(), wrap_width, dark);
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            let mut editor = egui::TextEdit::multiline(&mut pane.text)
                .id(id)
                .interactive(self.text_editable)
                .frame(egui::Frame::new().inner_margin(egui::Margin::same(8)))
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace);
            if item == EDITOR {
                editor = editor.layouter(&mut rust_layouter);
            }
            let bounds = ui.available_rect_before_wrap();
            let response = egui_dockspace::with_pane_keyboard_input_at(
                ui,
                id,
                editor.get_event_filter(),
                bounds,
                |ui| ui.put(bounds, editor),
            );
            content = content.with_content(&response);
            if item == EDITOR
                && let Some(probe) = &self.probe
            {
                probe.observe_text(ui, &response);
            }
            pane.input = Some((
                ui.ctx().viewport_id(),
                response.rect,
                response.has_focus(),
                ui.ctx().pixels_per_point(),
            ));
            pane.window_id =
                eframe::current_native_output_token().map(|token| u64::from(token.window_id()));
        }
        content
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        self.panes
            .get(&item)
            .filter(|pane| {
                self.text_editable && pane.loaded && ![PREVIEW, CONSOLE, PROFILER].contains(&item)
            })
            .map(|_| pane_focus_id(item))
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        self.focus_target(item).is_some()
    }

    fn sizing(&self, item: ItemId) -> PaneSizing {
        let (width, height) = if item == PREVIEW {
            (220.0, 160.0)
        } else {
            (120.0, 72.0)
        };
        PaneSizing::builder()
            .min_width(width)
            .min_height(height)
            .build()
            .expect("the native editor content minimums are valid")
    }
}

/// Polls only the receipt owned by this example operation.
fn poll_receipt(
    receipt: &mut Option<NativeActionReceipt>,
) -> Option<Result<NativeActionStatus, String>> {
    let result = match receipt.as_ref()?.try_recv() {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(std::sync::mpsc::TryRecvError::Empty) => return None,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Err("the native runtime ended before this action's terminal result".to_owned())
        }
    };
    *receipt = None;
    Some(result)
}

fn highlight_native_rust(
    ui: &egui::Ui,
    text: &str,
    wrap_width: f32,
    dark: bool,
) -> egui::text::LayoutJob {
    use egui::Color32;

    let font = egui::TextStyle::Monospace.resolve(ui.style());
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
                | "for" | "in" | "match" | "return" | "const" | "self" | "true" | "false" => kw_col,
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
