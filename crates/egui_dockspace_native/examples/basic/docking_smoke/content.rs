//! Interrupt ordinary editor widgets through OS input, then verify a fresh drag.

use std::io::{self, BufRead as _};
use std::sync::mpsc::{self, Receiver};

use egui_dockspace_native::NativeCommand;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Control {
    Canvas,
    Zoom,
    Text,
    Scroll,
}

impl Control {
    fn points(self, rect: egui::Rect) -> [egui::Pos2; 2] {
        let source = match self {
            Self::Canvas => rect.center(),
            Self::Zoom => rect.left_center() + egui::vec2(24.0, 0.0),
            Self::Text => rect.left_top() + egui::vec2(8.0, 8.0),
            Self::Scroll => rect.center_top() + egui::vec2(0.0, 8.0),
        };
        let delta = match self {
            Self::Canvas => egui::vec2(30.0, 20.0),
            Self::Zoom => egui::vec2(40.0, 0.0),
            Self::Text => egui::vec2(100.0, 0.0),
            Self::Scroll => egui::vec2(0.0, 35.0),
        };
        [source, source + delta]
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Pan(egui::Vec2),
    Zoom(f32),
    Selection(Option<egui::text::CCursorRange>),
    Scroll(f32),
}

#[derive(Clone, Debug)]
pub(super) struct Sample {
    rect: egui::Rect,
    held: bool,
    dragged: bool,
    primary_down: bool,
    value: Value,
}

impl EditorProbeHandle {
    fn observe_control(
        &self,
        ui: &egui::Ui,
        control: Control,
        response: &egui::Response,
        value: Value,
    ) {
        self.0
            .lock()
            .unwrap()
            .viewports
            .entry(ui.ctx().viewport_id())
            .or_default()
            .controls
            .insert(
                control,
                Sample {
                    rect: response.rect.intersect(ui.clip_rect()) * ui.ctx().pixels_per_point(),
                    held: response.is_pointer_button_down_on(),
                    dragged: response.dragged_by(egui::PointerButton::Primary),
                    primary_down: ui.input(|input| input.pointer.primary_down()),
                    value,
                },
            );
    }

    pub(in super::super) fn observe_canvas(
        &self,
        ui: &egui::Ui,
        canvas: &egui::Response,
        pan: egui::Vec2,
        slider: &egui::Response,
        zoom: f32,
    ) {
        self.observe_control(ui, Control::Canvas, canvas, Value::Pan(pan));
        self.observe_control(ui, Control::Zoom, slider, Value::Zoom(zoom));
    }

    pub(in super::super) fn observe_text(&self, ui: &egui::Ui, response: &egui::Response) {
        let selection = egui::TextEdit::load_state(ui.ctx(), response.id)
            .and_then(|state| state.cursor.char_range());
        self.observe_control(ui, Control::Text, response, Value::Selection(selection));
    }

    pub(in super::super) fn observe_scroll(&self, ui: &egui::Ui, id: egui::Id, offset: f32) {
        if let Some(response) = ui.ctx().read_response(id.with(1_usize)) {
            self.observe_control(ui, Control::Scroll, &response, Value::Scroll(offset));
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Interruption {
    Deactivate,
    OutsideRelease,
}

#[derive(Debug)]
enum Phase {
    Prepare,
    Drag {
        before: Value,
        cancellations: usize,
        releases: usize,
    },
    Interrupt {
        cancellations: usize,
        releases: usize,
    },
    Tail {
        value: Value,
        releases: usize,
        moves: usize,
        acknowledged: bool,
    },
    Fresh {
        before: Value,
        releases: usize,
        dragged: bool,
    },
}

pub(super) struct ContentSmoke {
    started: Instant,
    interruption: Interruption,
    index: usize,
    selected: bool,
    phase: Phase,
    external: Receiver<String>,
    layout: Option<String>,
    text: Option<BTreeMap<ItemId, String>>,
}

impl ContentSmoke {
    pub(super) fn new(interruption: Interruption) -> Self {
        let (send, external) = mpsc::channel();
        std::thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            started: Instant::now(),
            interruption,
            index: 0,
            selected: false,
            phase: Phase::Prepare,
            external,
            layout: None,
            text: None,
        }
    }

    pub(super) fn advance(
        &mut self,
        app: &NativeExampleApp,
        observations: &Arc<Mutex<EditorObservations>>,
    ) -> bool {
        assert!(app.dockspace.error_kind().is_none());
        assert!(
            self.started.elapsed() < Duration::from_secs(65),
            "content journey timed out: control={} phase={:?}; samples={:?}",
            self.index,
            self.phase,
            observations.lock().unwrap().root().controls
        );
        while let Some(result) = app.dockspace.take_action_status() {
            assert!(result.is_ok(), "content setup failed: {result:?}");
        }
        if !self.selected {
            app.dockspace
                .request_command(NativeCommand::SelectItem { item: PREVIEW })
                .unwrap();
            self.selected = true;
            return false;
        }
        if !app.dockspace.is_quiescent() || !app.all_surfaces_presented() {
            return false;
        }
        let layout = app
            .dockspace
            .with_view(|view| format!("{:?}", view.to_layout()));
        self.layout.get_or_insert_with(|| layout.clone());
        self.text.get_or_insert_with(|| drain::text(app));
        assert_eq!(self.layout.as_ref(), Some(&layout));
        assert_eq!(self.text.as_ref(), Some(&drain::text(app)));
        let controls = [
            Control::Canvas,
            Control::Zoom,
            Control::Text,
            Control::Scroll,
        ];
        let Some(&control) = controls.get(self.index) else {
            return true;
        };
        let observed = observations.lock().unwrap();
        let root = observed.root();
        let Some(sample) = root.controls.get(&control) else {
            return false;
        };
        match &mut self.phase {
            Phase::Prepare if !sample.primary_down => {
                announce("start", control, sample);
                self.phase = Phase::Drag {
                    before: sample.value.clone(),
                    cancellations: root.cancellations,
                    releases: root.releases,
                };
            }
            Phase::Drag {
                before,
                cancellations,
                releases,
            } if sample.held && sample.dragged && sample.value != *before => {
                eprintln!(
                    "editor-content-interrupt: {:?} {control:?}",
                    self.interruption
                );
                self.phase = Phase::Interrupt {
                    cancellations: *cancellations,
                    releases: *releases,
                };
            }
            Phase::Interrupt {
                cancellations,
                releases,
            } if !sample.held
                && !sample.dragged
                && !sample.primary_down
                && match self.interruption {
                    Interruption::Deactivate => root.cancellations > *cancellations,
                    Interruption::OutsideRelease => root.releases > *releases,
                } =>
            {
                if matches!(self.interruption, Interruption::Deactivate) {
                    assert_eq!(root.releases, *releases, "cancel must precede real up");
                }
                announce("cancelled", control, sample);
                self.phase = Phase::Tail {
                    value: sample.value.clone(),
                    releases: *releases,
                    moves: root.moves,
                    acknowledged: false,
                };
            }
            Phase::Tail {
                value,
                releases,
                moves,
                acknowledged,
            } => {
                assert!(!sample.held && !sample.dragged && !sample.primary_down);
                assert_eq!(sample.value, *value, "late input changed {control:?}");
                if !*acknowledged {
                    match self.external.try_recv() {
                        Ok(line) => {
                            assert_eq!(line.trim(), "content-tail-sent");
                            *acknowledged = true;
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(error) => panic!("content input driver disconnected: {error}"),
                    }
                }
                if *acknowledged && root.releases > *releases && root.moves >= *moves + 2 {
                    announce("fresh", control, sample);
                    self.phase = Phase::Fresh {
                        before: value.clone(),
                        releases: root.releases,
                        dragged: false,
                    };
                }
            }
            Phase::Fresh {
                before,
                releases,
                dragged,
            } => {
                *dragged |= sample.held && sample.dragged && sample.primary_down;
                if root.releases > *releases && !sample.primary_down && sample.value != *before {
                    assert!(*dragged, "fresh {control:?} must receive a real drag");
                    assert!(!sample.held && !sample.dragged);
                    eprintln!(
                        "editor-content-verified: {control:?} interrupted-tail=inert fresh-drag=changed"
                    );
                    self.index += 1;
                    self.phase = Phase::Prepare;
                }
            }
            _ => {}
        }
        false
    }
}

fn announce(stage: &str, control: Control, sample: &Sample) {
    let [mut source, mut target] = control.points(sample.rect);
    if stage == "fresh" {
        std::mem::swap(&mut source, &mut target);
    }
    eprintln!(
        "editor-content-{stage}: {control:?} {} {} {} {}",
        source.x, source.y, target.x, target.y
    );
}
