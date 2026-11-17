//! Browser entry point for the same editor used by the desktop workbench.

#![cfg(target_arch = "wasm32")]

#[cfg(feature = "motion")]
use std::{cell::Cell, rc::Rc};

use egui::{Context, RawInput};
#[cfg(feature = "motion")]
use egui_dockspace::motion::MotionPreference;
use egui_dockspace::{
    Dockspace, DockspaceOutput, DockspaceResponse, PaneView, SurfacePresentationResult,
    winit::{
        AppResult, FrameControl, NativeApp, NativeFrameReport, WebOptions, WebRenderer, run_web,
    },
};
use wasm_bindgen::{JsCast, prelude::*};

#[path = "../../../crates/egui_dockspace/examples/workbench/editor.rs"]
mod editor;

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|panic| {
        status("error", &format!("The demo stopped: {panic}"));
    }));
    status("loading", "Initializing the docking demo…");
    match launch() {
        Ok(()) => Ok(()),
        Err(error) => {
            status(
                "error",
                error
                    .as_string()
                    .as_deref()
                    .unwrap_or("The demo could not start."),
            );
            Err(error)
        }
    }
}

fn launch() -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("The browser document is unavailable."))?;
    let canvas = document
        .get_element_by_id("dockspace-canvas")
        .ok_or_else(|| JsValue::from_str("The docking canvas is missing."))?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("dockspace-canvas must be a canvas element."))?;
    let query = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let parameters = || query.trim_start_matches('?').split('&');
    let test_mode = parameters().any(|part| part == "test=1");
    let mut options = WebOptions::new(canvas.clone());
    if parameters().any(|part| part == "renderer=webgl") {
        options.renderer = WebRenderer::WebGl;
    }
    let context = editor::new_context();
    if test_mode {
        context.enable_accesskit();
    }
    #[cfg(feature = "motion")]
    let system_motion = SystemMotion::observe(&context)?;
    let app = Workbench {
        dockspace: Dockspace::builder("web-workbench", editor::demo_layout())
            .style(editor::dock_style())
            .build()
            .map_err(|error| JsValue::from_str(&error.to_string()))?,
        panes: editor::DemoPanes::new(),
        response: None,
        restore_layout: false,
        ready: false,
        test_canvas: test_mode.then(|| canvas.clone()),
        presented_frames: 0,
        #[cfg(feature = "motion")]
        system_motion,
    };
    run_web(options, context, app, |error| {
        status("error", &format!("The demo stopped: {error}"));
    })
    .map_err(|error| JsValue::from_str(&error.to_string()))
}

fn status(state: &str, message: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("demo-status"))
    else {
        return;
    };
    element.set_text_content(Some(message));
    let _ = element.set_attribute("data-state", state);
}

struct Workbench {
    dockspace: Dockspace,
    panes: editor::DemoPanes,
    response: Option<DockspaceResponse>,
    restore_layout: bool,
    ready: bool,
    test_canvas: Option<web_sys::HtmlCanvasElement>,
    presented_frames: u64,
    #[cfg(feature = "motion")]
    system_motion: Option<SystemMotion>,
}

#[cfg(feature = "motion")]
struct SystemMotion {
    query: web_sys::MediaQueryList,
    listener: Closure<dyn FnMut(web_sys::MediaQueryListEvent)>,
    pending: Rc<Cell<Option<bool>>>,
}

#[cfg(feature = "motion")]
impl SystemMotion {
    fn observe(context: &Context) -> Result<Option<Self>, JsValue> {
        let Some(window) = web_sys::window() else {
            return Ok(None);
        };
        let Some(query) = window.match_media("(prefers-reduced-motion: reduce)")? else {
            return Ok(None);
        };
        let pending = Rc::new(Cell::new(None));
        let changes = Rc::clone(&pending);
        let context = context.clone();
        let listener = Closure::<dyn FnMut(web_sys::MediaQueryListEvent)>::new(
            move |event: web_sys::MediaQueryListEvent| {
                changes.set(Some(event.matches()));
                context.request_repaint();
            },
        );
        query.add_event_listener_with_callback("change", listener.as_ref().unchecked_ref())?;
        pending.set(Some(query.matches()));
        Ok(Some(Self {
            query,
            listener,
            pending,
        }))
    }

    fn take_change(&self) -> Option<MotionPreference> {
        self.pending.take().map(|reduced| {
            if reduced {
                MotionPreference::Reduced
            } else {
                MotionPreference::Full
            }
        })
    }
}

#[cfg(feature = "motion")]
impl Drop for SystemMotion {
    fn drop(&mut self) {
        let _ = self
            .query
            .remove_event_listener_with_callback("change", self.listener.as_ref().unchecked_ref());
    }
}

impl Workbench {
    fn publish_test_state(&mut self, frame: &NativeFrameReport<'_>) {
        let Some(canvas) = &self.test_canvas else {
            return;
        };
        if frame.presentation != SurfacePresentationResult::Presented {
            return;
        }
        self.presented_frames += 1;
        let Some(update) = frame.accesskit_update else {
            return;
        };
        let mut tabs = Vec::new();
        let mut guides = Vec::new();
        let mut splitters = Vec::new();
        let mut controls = Vec::new();
        for (_, node) in &update.nodes {
            let Some(bounds) = node.bounds() else {
                continue;
            };
            let record = serde_json::json!({
                "label": node.label(),
                "rect": [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
                "role": format!("{:?}", node.role()),
                "value": node.numeric_value(),
                "text": node.value(),
            });
            match (node.role(), node.label()) {
                (egui::accesskit::Role::Tab, _) => tabs.push(record),
                (egui::accesskit::Role::Label, Some("Dockspace center drop guide")) => {
                    guides.push(record);
                }
                (egui::accesskit::Role::Splitter, _) if !node.is_disabled() => {
                    splitters.push(record);
                }
                (egui::accesskit::Role::Slider | egui::accesskit::Role::ComboBox, _)
                | (_, Some(_)) => controls.push(record),
                _ => {}
            }
        }
        let view = self.dockspace.view();
        let rect = |rect: egui::Rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
        let panes: Vec<_> = self
            .panes
            .panes
            .keys()
            .filter_map(|&id| {
                let item = view.item(id)?;
                Some(serde_json::json!({
                    "item": id.get(),
                    "root": item.root().get(),
                    "group": item.tabs().items().iter().map(|id| id.get()).collect::<Vec<_>>(),
                    "selected": item.tabs().selected().map(|id| id.get()),
                    "contentRect": self.panes.panes.get(&id).and_then(|pane| pane.content).map(rect),
                }))
            })
            .collect();
        #[cfg(feature = "motion")]
        let motion = serde_json::json!({
            "settings": format!("{:?}", self.dockspace.motion_settings()),
            "preference": format!("{:?}", self.dockspace.motion_preference()),
        });
        #[cfg(not(feature = "motion"))]
        let motion = serde_json::json!({ "enabled": false });
        let state = serde_json::json!({
            "version": format!("{:?}", self.dockspace.version()),
            "frames": self.presented_frames,
            "activeGesture": self.dockspace.has_active_gesture(),
            "pixelsPerPoint": frame.context.pixels_per_point(),
            "zoomFactor": frame.context.zoom_factor(),
            "input": frame.context.input(|input| serde_json::json!({
                "pointer": input.pointer.latest_pos().map(|point| [point.x, point.y]),
                "primaryDown": input.pointer.primary_down(),
                "shift": input.modifiers.shift,
            })),
            "tabs": tabs,
            "centerGuides": guides,
            "splitters": splitters,
            "controls": controls,
            "panes": panes,
            "editorText": self.panes.panes.get(&editor::EDITOR).map(|pane| &pane.text),
            "editorFocused": self.panes.focus_target(editor::EDITOR)
                .is_some_and(|id| frame.context.memory(|memory| memory.has_focus(id))),
            "editorRect": self.panes.panes.get(&editor::EDITOR).and_then(|pane| pane.editor).map(rect),
            "consoleOffset": self.panes.panes.get(&editor::CONSOLE).map(|pane| pane.scroll_offset),
            "consoleRect": self.panes.panes.get(&editor::CONSOLE).and_then(|pane| pane.scroll_bounds).map(rect),
            "motion": motion,
            "tabPreset": format!("{:?}", self.dockspace.style().tab_preset),
            "darkMode": frame.context.global_style().visuals.dark_mode,
        });
        let _ = canvas.set_attribute("data-dockspace-state", &state.to_string());
    }
}

impl NativeApp for Workbench {
    fn update(&mut self, context: &Context, input: RawInput) -> AppResult<DockspaceOutput> {
        #[cfg(feature = "motion")]
        if let Some(preference) = self
            .system_motion
            .as_ref()
            .and_then(SystemMotion::take_change)
        {
            self.dockspace.set_motion_preference(preference);
        }
        self.restore_layout = false;
        let (output, response) = editor::run_editor(
            &mut self.dockspace,
            context,
            input,
            &mut self.panes,
            &mut self.restore_layout,
        )?;
        self.response = response;
        Ok(output)
    }

    fn frame_finished(&mut self, frame: NativeFrameReport<'_>) -> AppResult<FrameControl> {
        // Observe the rendered state before any post-frame close or restore command.
        self.publish_test_state(&frame);
        let changed = editor::apply_editor_commands(
            &mut self.dockspace,
            self.response.as_ref(),
            self.restore_layout,
        )?;
        if !self.ready && frame.presentation == SurfacePresentationResult::Presented {
            self.ready = true;
            status(
                "ready",
                "Ready — drag tabs to dock; Shift + drag to float inside the canvas.",
            );
        }
        Ok(if changed {
            FrameControl::Repaint
        } else {
            FrameControl::Continue
        })
    }
}
