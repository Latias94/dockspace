//! Physical Root/Child round trips, followed by retirement of the drained Child.

use dockspace::geometry::PhysicalRect;
use dockspace::model::{DockspaceActionOutcome, SurfaceId};
use dockspace::runtime::NativeWindowPlacement;
use egui_dockspace_native::{NativeActionStatus, NativeCommand};
use std::io::{self, BufRead as _};
use std::sync::mpsc::{self, Receiver};

use egui_dockspace::DockspaceActionStatus;

use super::super::{INSPECTOR_FLOATING, INSPECTOR_ROOT, MAIN_ROOT};
use super::*;

mod geometry;
mod interrupt;
use interrupt::InterruptedDrag;
pub(super) use interrupt::Interruption;

pub(super) struct NativeSmoke {
    started: Instant,
    order: [ItemId; 2],
    created: bool,
    child: Option<(egui::ViewportId, u64, SurfaceId)>,
    step: usize,
    announced: bool,
    target_announced: bool,
    expected: Vec<ItemId>,
    remaining: Vec<ItemId>,
    text: Option<BTreeMap<ItemId, String>>,
    setup_completed: bool,
    outside_observed: bool,
    geometry_continued: bool,
    retains_empty_central: bool,
    close_announced: bool,
    retirement_announced: bool,
    external_observation: Receiver<String>,
    interruption: Option<InterruptedDrag>,
    retired_child: Option<(egui::ViewportId, u64, SurfaceId)>,
}

impl NativeSmoke {
    pub(super) fn new(order: [ItemId; 2], retains_empty_central: bool) -> Self {
        let (send, external_observation) = mpsc::channel();
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
            order,
            created: false,
            child: None,
            step: 0,
            announced: false,
            target_announced: false,
            expected: Vec::new(),
            remaining: Vec::new(),
            text: None,
            setup_completed: false,
            outside_observed: false,
            geometry_continued: false,
            retains_empty_central,
            close_announced: false,
            retirement_announced: false,
            external_observation,
            interruption: None,
            retired_child: None,
        }
    }

    pub(super) fn with_interruption(mut self, kind: Interruption) -> Self {
        self.interruption = Some(InterruptedDrag::new(kind));
        self
    }

    pub(super) fn advance(
        &mut self,
        app: &NativeExampleApp,
        context: &egui::Context,
        observations: &Arc<Mutex<EditorObservations>>,
    ) -> bool {
        assert!(app.dockspace.error_kind().is_none());
        assert!(
            self.started.elapsed() < Duration::from_secs(45),
            "native physical journey timed out at step {}; pending={:?}; view={:?}; interruption={:?}; observations={:?}",
            self.step,
            app.dockspace.pending_work(),
            app.dockspace.with_view(|view| format!("{view:?}")),
            self.interruption,
            observations
                .lock()
                .unwrap()
                .viewports
                .iter()
                .map(|(id, sample)| (
                    id,
                    sample.window,
                    sample.preview_tabs.as_ref(),
                    sample.escape_presses,
                    sample.focus_losses,
                    sample.releases
                ))
                .collect::<Vec<_>>()
        );
        if let Some(result) = app.dockspace.take_action_status() {
            assert!(!self.setup_completed);
            assert!(
                matches!(
                    result,
                    Ok(NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                        DockspaceActionOutcome::NativeRootTearOffRequested { .. }
                    )))
                ),
                "unexpected setup result: {result:?}"
            );
            self.setup_completed = true;
        }
        if self.announced && self.step == 0 && self.interruption.is_none() && !self.outside_observed
        {
            let observed = observations.lock().unwrap();
            if observed.root().primary_down && observed.root().held_below_client {
                self.outside_observed = true;
                eprintln!("editor-native-outside-observed: original-primary-held=true");
            }
        }
        self.text.get_or_insert_with(|| text(app));
        if !app.dockspace.is_quiescent() || !app.all_surfaces_presented() {
            return false;
        }
        if !self.created {
            // Setup uses the same public command as the toolbar. Every move
            // under test below is submitted exclusively by real pointer input.
            app.dockspace
                .request_command(NativeCommand::MoveRootToNewWindow {
                    root: INSPECTOR_ROOT,
                    placement: NativeWindowPlacement::new(
                        PhysicalRect::new(1400.0, 180.0, 440.0, 340.0).unwrap(),
                    ),
                })
                .unwrap();
            self.created = true;
            return false;
        }
        if self.child.is_none() {
            if !self.setup_completed {
                return false;
            }
            let child_surface = app
                .dockspace
                .with_view(|view| view.item(INSPECTOR).unwrap().surface());
            if child_surface == SURFACE {
                return false;
            }
            let observations = observations.lock().unwrap();
            let Some((&viewport, child)) = observations.viewports.iter().find(|(id, sample)| {
                **id != egui::ViewportId::ROOT
                    && self
                        .retired_child
                        .is_none_or(|(retired, _, _)| **id != retired)
                    && sample.tabs.contains_key("Inspector")
                    && sample.window.is_some()
            }) else {
                return false;
            };
            let window = child.window.unwrap();
            if let Some((old_viewport, old_window, old_surface)) = self.retired_child {
                assert_ne!(
                    viewport, old_viewport,
                    "a recreated Child has a fresh viewport"
                );
                assert_ne!(
                    child_surface, old_surface,
                    "the retired surface cannot be reused"
                );
                eprintln!(
                    "editor-native-interrupt-recreated: {old_viewport:?} {old_window} {viewport:?} {window}"
                );
            }
            eprintln!("editor-native-child: {viewport:?} {window}");
            self.child = Some((viewport, window, child_surface));
        }
        if self.step == 4 {
            let (viewport, window, surface) = self.child.unwrap();
            let retained = app.dockspace.with_view(|view| {
                if let Some(surface) = view.surface(surface) {
                    assert!(self.retains_empty_central);
                    let root = surface.main_root().unwrap();
                    assert_eq!(root.id(), INSPECTOR_ROOT);
                    assert!(root.retains_empty_central_root());
                    assert!(root.content().unwrap().tabs().unwrap().tabs().is_empty());
                    assert_eq!(surface.contained_count(), 0);
                    true
                } else {
                    false
                }
            });
            if retained {
                if !self.close_announced {
                    eprintln!("editor-native-empty-central: {viewport:?} {window}");
                    self.close_announced = true;
                }
                return false;
            }
            assert_eq!(self.close_announced, self.retains_empty_central);
            app.dockspace.with_view(|view| {
                assert!(view.surface(surface).is_none());
                if self.retains_empty_central {
                    // RecoverPresentation restores the complete root, including
                    // its deliberate empty central docking target.
                    let recovered = view.contained(INSPECTOR_FLOATING).unwrap();
                    let root = recovered.root().unwrap();
                    assert_eq!(root.id(), INSPECTOR_ROOT);
                    assert!(!root.retains_empty_central_root());
                    let content = root.content().unwrap();
                    assert!(content.is_central());
                    assert!(content.tabs().unwrap().tabs().is_empty());
                    assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 1);
                } else {
                    assert!(view.root(INSPECTOR_ROOT).is_none());
                    assert!(view.contained(INSPECTOR_FLOATING).is_none());
                    assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 0);
                }
                assert_eq!(view.surfaces().count(), 1);
                for item in [
                    OUTLINE, EDITOR, PREVIEW, INSPECTOR, CONSOLE, HISTORY, PROFILER, ASSETS,
                ] {
                    let pane = view
                        .item(item)
                        .expect("all eight panes remain in the model");
                    assert_eq!(pane.surface(), SURFACE);
                    assert_eq!(pane.root(), MAIN_ROOT);
                    assert_eq!(pane.contained(), None);
                }
                for item in [OUTLINE, ASSETS, HISTORY, self.order[0], self.order[1]] {
                    let pane = view.item(item).unwrap();
                    assert_eq!(pane.root(), MAIN_ROOT);
                    assert_eq!(
                        pane.tabs().items().collect::<Vec<_>>(),
                        [OUTLINE, ASSETS, HISTORY, self.order[0], self.order[1]]
                    );
                }
            });
            assert_eq!(self.text.as_ref().unwrap(), &text(app));
            assert_eq!(self.text.as_ref().unwrap().len(), 8);
            assert!(self.outside_observed || self.geometry_continued);
            if !self.retirement_announced {
                eprintln!(
                    "editor-native-retired: {} registry=8 text=preserved quiescent=true",
                    self.child.unwrap().1
                );
                self.retirement_announced = true;
            }
            return match self.external_observation.try_recv() {
                Ok(line) => {
                    assert_eq!(line.trim(), "native-retirement-observed");
                    true
                }
                Err(mpsc::TryRecvError::Empty) => false,
                Err(error) => panic!("the external OS-window observer disconnected: {error}"),
            };
        }
        let (source, target) = match self.step {
            0 => (HISTORY, INSPECTOR),
            1 => (HISTORY, OUTLINE),
            2 => (self.order[0], OUTLINE),
            3 => (self.order[1], OUTLINE),
            _ => unreachable!(),
        };
        if self.announced {
            if let Some(interruption) = &mut self.interruption {
                if !interruption.advance(
                    app,
                    context,
                    observations,
                    self.child.unwrap(),
                    &self.expected,
                    &self.external_observation,
                ) {
                    return false;
                }
                let continued = interruption.continues_drag();
                if interruption.recreates_child() {
                    self.retired_child = self.child.take();
                    let retired = self.retired_child.unwrap().0;
                    observations.lock().unwrap().viewports.remove(&retired);
                    self.created = false;
                    self.setup_completed = false;
                }
                assert_eq!(self.text.as_ref().unwrap(), &text(app));
                self.interruption = None;
                if !continued {
                    self.announced = false;
                    self.expected.clear();
                    self.remaining.clear();
                    return false;
                }
                // A geometry refresh keeps the original held drag. Its real
                // release is step zero and goes through the usual completion
                // checks instead of minting a replacement press.
                self.geometry_continued = true;
            }
            let completed = app.dockspace.with_view(|view| {
                let source = view.item(source).unwrap();
                let target = view.item(target).unwrap();
                source.tabs().items().eq(self.expected.iter().copied())
                    && target.tabs().items().eq(self.expected.iter().copied())
                    && source.surface() == target.surface()
                    && source.is_selected()
                    && self.remaining.first().is_none_or(|item| {
                        view.item(*item)
                            .unwrap()
                            .tabs()
                            .items()
                            .eq(self.remaining.iter().copied())
                    })
            });
            if completed {
                assert_eq!(self.text.as_ref().unwrap(), &text(app));
                if self.step < 3 {
                    assert!(
                        app.dockspace
                            .with_view(|view| view.surface(self.child.unwrap().2).is_some())
                    );
                }
                eprintln!(
                    "editor-native-moved: step={} source={source:?} target={target:?}",
                    self.step
                );
                self.step += 1;
                self.announced = false;
                self.target_announced = false;
                self.expected.clear();
                self.remaining.clear();
            } else if !self.target_announced {
                let target_viewport = if self.step == 0 {
                    self.child.unwrap().0
                } else {
                    egui::ViewportId::ROOT
                };
                let Some(selected) = app.dockspace.with_view(|view| {
                    view.item(target)
                        .unwrap()
                        .tabs()
                        .selected()
                        .and_then(|tab| tab.item())
                }) else {
                    return false;
                };
                if let Some((bounds, scale)) = app
                    .dockspace
                    .with_panes(|panes| panes.panes[&selected].content)
                    && let Some(sample) =
                        observations.lock().unwrap().viewports.get(&target_viewport)
                    && let Some(guide) = sample
                        .center_guides
                        .iter()
                        .find(|guide| bounds.contains(guide.center()))
                {
                    // Small panes may move their guide cluster to avoid root
                    // edge targets. Drive the rendered merge button itself.
                    let point = guide.center() * scale;
                    eprintln!(
                        "editor-native-target: {target_viewport:?} {} {} {}",
                        sample.window.unwrap(),
                        point.x,
                        point.y
                    );
                    self.target_announced = true;
                }
            }
            return false;
        }
        let source_viewport = if self.step == 0 {
            egui::ViewportId::ROOT
        } else {
            self.child.unwrap().0
        };
        let target_viewport = if self.step == 0 {
            self.child.unwrap().0
        } else {
            egui::ViewportId::ROOT
        };
        let source_title = app.dockspace.with_panes(|panes| panes.panes[&source].title);
        let observations = observations.lock().unwrap();
        let Some(source_sample) = observations.viewports.get(&source_viewport) else {
            return false;
        };
        let Some(target_sample) = observations.viewports.get(&target_viewport) else {
            return false;
        };
        let Some(source_bounds) = source_sample.tabs.get(source_title).copied() else {
            return false;
        };
        let Some(selected) = app.dockspace.with_view(|view| {
            view.item(target)
                .unwrap()
                .tabs()
                .selected()
                .and_then(|tab| tab.item())
        }) else {
            return false;
        };
        let Some((target_bounds, scale)) = app
            .dockspace
            .with_panes(|panes| panes.panes[&selected].content)
        else {
            return false;
        };
        let source_point = (source_bounds.min + source_bounds.size() * 0.35) * source_sample.scale;
        let target_point = target_bounds.center() * scale;
        let (mut expected, remaining) = app.dockspace.with_view(|view| {
            (
                view.item(target)
                    .unwrap()
                    .tabs()
                    .items()
                    .collect::<Vec<_>>(),
                view.item(source)
                    .unwrap()
                    .tabs()
                    .items()
                    .filter(|item| *item != source)
                    .collect::<Vec<_>>(),
            )
        });
        assert!(!expected.contains(&source));
        expected.push(source);
        self.expected = expected;
        self.remaining = remaining;
        eprintln!(
            "editor-native-drag: {source_viewport:?} {} {} {} {target_viewport:?} {} {} {}",
            source_sample.window.unwrap(),
            source_point.x.round(),
            source_point.y.round(),
            target_sample.window.unwrap(),
            target_point.x.round(),
            target_point.y.round()
        );
        self.announced = true;
        false
    }
}

fn text(app: &NativeExampleApp) -> BTreeMap<ItemId, String> {
    app.dockspace.with_panes(|panes| {
        panes
            .panes
            .iter()
            .map(|(&id, pane)| (id, pane.text.clone()))
            .collect()
    })
}
