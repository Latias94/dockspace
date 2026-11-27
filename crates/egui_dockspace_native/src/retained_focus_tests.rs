//! Real output tokens exercise the retention entry used by deferred viewports.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dockspace::model::{
    DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, RootId,
};
use dockspace::runtime::DockspaceSession;
use egui_dockspace::{DockStyle, PaneResponse};
use winit::platform::x11::EventLoopBuilderExtX11 as _;

use super::*;

const SURFACE: SurfaceId = SurfaceId::new(1);
const EDITOR: ItemId = ItemId::new(1);
const NEXT_EDITOR: ItemId = ItemId::new(2);

struct Editor {
    target: egui::Id,
    text: [String; 2],
    first_header: Option<egui::Id>,
    calls: Arc<AtomicUsize>,
}

impl PaneView for Editor {
    fn title(&self, _: ItemId) -> Option<egui::WidgetText> {
        Some("Retained editor".into())
    }

    fn tab_ui(
        &mut self,
        ui: &mut egui::Ui,
        tab: &egui_dockspace::Tab<'_>,
    ) -> egui_dockspace::TabVisuals {
        if tab.item() == Some(EDITOR) {
            self.first_header = Some(tab.response.id);
        }
        tab.paint(ui)
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        let target = self.focus_target(item).unwrap();
        let response = egui_dockspace::pane_text_edit(
            ui,
            target,
            egui::TextEdit::singleline(&mut self.text[usize::from(item == NEXT_EDITOR)]),
        );
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            response.request_focus();
        }
        PaneResponse::default().with_content(&response)
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(if item == EDITOR {
            self.target
        } else {
            self.target.with(item)
        })
    }

    fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
        true
    }
}

struct RetainApp {
    app: NativeDockspaceApp<Editor>,
    phase: Phase,
    header: Option<egui::Id>,
    save_values: Vec<String>,
    calls: Arc<AtomicUsize>,
    skipped: Arc<AtomicUsize>,
    started: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initial,
    Activate,
    Retain,
    Resume,
    Done,
}

impl eframe::App for RetainApp {
    fn raw_input_hook(&mut self, _: &egui::Context, raw: &mut egui::RawInput) {
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        };
        match self.phase {
            Phase::Activate => raw.events.extend([
                key(egui::Key::ArrowRight, egui::Modifiers::NONE),
                key(egui::Key::Enter, egui::Modifiers::NONE),
                egui::Event::Paste("X".into()),
                key(egui::Key::S, egui::Modifiers::COMMAND),
            ]),
            Phase::Retain if self.skipped.load(Ordering::SeqCst) == 0 => {
                raw.events.push(egui::Event::Text("Y".into()))
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(15));
        if self.phase == Phase::Retain {
            let header = self.header.unwrap();
            assert!(ui.memory(|memory| memory.has_focus(header)));
            let calls = self.calls.load(Ordering::SeqCst);
            let token = eframe::current_native_output_token().unwrap();
            let mut state = lock_state(&self.app.state);
            state
                .stage_retain_previous(ui.ctx(), token, SURFACE)
                .unwrap();
            assert_eq!(self.calls.load(Ordering::SeqCst), calls);
            assert!(
                ui.ctx()
                    .viewport(|viewport| viewport.this_pass.widgets.get(header).is_none())
            );
            if self.skipped.fetch_add(1, Ordering::SeqCst) == 1 {
                self.phase = Phase::Resume;
            }
        } else {
            eframe::App::ui(&mut self.app, ui, frame);
            match self.phase {
                Phase::Initial if self.app.is_surface_presented(SURFACE) => {
                    let header = self.app.with_panes(|panes| panes.first_header.unwrap());
                    ui.memory_mut(|memory| memory.request_focus(header));
                    self.phase = Phase::Activate;
                }
                Phase::Activate => {
                    self.header = ui.memory(egui::Memory::focused);
                    assert!(self.header.is_some());
                    self.phase = Phase::Retain;
                }
                Phase::Resume => {
                    if ui.input_mut(|input| {
                        input.consume_shortcut(&egui::KeyboardShortcut::new(
                            egui::Modifiers::COMMAND,
                            egui::Key::S,
                        ))
                    }) {
                        self.save_values
                            .push(self.app.with_panes(|panes| panes.text[1].clone()));
                    }
                    let finished = self.app.with_panes(|panes| panes.text[1] == "bXY");
                    if finished {
                        self.app.with_panes(|panes| {
                            assert_eq!(panes.text, ["a", "bXY"]);
                            assert_eq!(
                                ui.memory(egui::Memory::focused),
                                panes.focus_target(NEXT_EDITOR)
                            );
                        });
                        assert_eq!(self.save_values, ["bX"]);
                        self.phase = Phase::Done;
                        self.skipped.fetch_add(1, Ordering::SeqCst);
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                _ => {}
            }
        }
        if self.phase != Phase::Done {
            ui.ctx().request_repaint_after(Duration::from_millis(5));
        }
    }
}

#[test]
fn real_eframe_retained_output_preserves_activation_and_editor_packets() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!(
            "SKIP retained editor focus: DISPLAY is unavailable; run the X11 qualification gate"
        );
        return;
    }
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::central_tabs([EDITOR, NEXT_EDITOR]),
        ),
    )])
    .unwrap();
    let target = egui::Id::unique("real-retained-editor");
    let calls = Arc::new(AtomicUsize::new(0));
    let skipped = Arc::new(AtomicUsize::new(0));
    let app = NativeDockspaceApp::new_with_close_policy(
        egui::Id::unique("real-retained-focus"),
        DockspaceSession::from_layout_with_presentation_config(
            layout,
            Default::default(),
            egui_dockspace::DockStyle::default()
                .presentation_config()
                .unwrap(),
        )
        .unwrap(),
        SURFACE,
        Editor {
            target,
            text: ["a".into(), "b".into()],
            first_header: None,
            calls: Arc::clone(&calls),
        },
        DockStyle::default(),
        NativeWindowClosePolicy::RetainLayout,
    )
    .unwrap();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        native_host: Some(app.native_host_handler()),
        renderer: eframe::Renderer::Glow,
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_x11().with_any_thread(true);
        })),
        ..Default::default()
    };
    let completed = Arc::clone(&skipped);
    eframe::run_native(
        "Retained editor focus regression",
        options,
        Box::new(move |_| {
            Ok(Box::new(RetainApp {
                app,
                phase: Phase::Initial,
                header: None,
                save_values: Vec::new(),
                calls,
                skipped,
                started: Instant::now(),
            }))
        }),
    )
    .unwrap();
    assert_eq!(completed.load(Ordering::SeqCst), 3);
}
