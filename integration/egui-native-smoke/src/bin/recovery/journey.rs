//! Finite application-command journeys over real native viewports.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dockspace::geometry::PhysicalRect;
use dockspace::model::{
    DockMoveSource, DockMoveTarget, DockspaceActionOutcome, DockspaceActionRejection,
    NativeWindowPlacement, TabGroupDockMode,
};
use eframe::egui::{self, ViewportCommand, ViewportId};
use egui_dockspace::DockspaceActionStatus;
use egui_dockspace_native::{NativeActionStatus, NativeCommand};

use super::{ITEMS, PreparedTabsGroup, ROOT, SharedApp, lock};

#[derive(Debug)]
enum DockBackProgress {
    Ready,
    AwaitingResult,
    Settling,
}

#[derive(Debug)]
enum Phase {
    Root,
    RejectStale,
    Creating {
        index: usize,
        target: Option<dockspace::model::SurfaceId>,
    },
    Resize,
    Zoom,
    RestoreZoom,
    Fail,
    AwaitFatal,
    Drain,
    DockBack {
        index: usize,
        progress: DockBackProgress,
    },
    Done,
}

pub(super) struct Journey {
    app: SharedApp,
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
    phase: Phase,
    receipt: Option<egui_dockspace_native::NativeActionReceipt>,
    fail: bool,
    stale: Option<PreparedTabsGroup>,
    deadline: Instant,
}

impl Journey {
    pub(super) fn new(
        app: SharedApp,
        outcome: Arc<Mutex<Option<Result<(), String>>>>,
        fail: bool,
        stale: Option<PreparedTabsGroup>,
    ) -> Self {
        Self {
            app,
            outcome,
            phase: Phase::Root,
            receipt: None,
            fail,
            stale,
            deadline: Instant::now() + Duration::from_secs(25),
        }
    }

    fn advance(&mut self, app: &mut super::App, context: &egui::Context) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            return Err(format!(
                "phase {:?} timed out; pending={:?}, backlog={:?}",
                self.phase,
                app.pending_work(),
                app.output_backlog()
            ));
        }
        if !matches!(self.phase, Phase::AwaitFatal | Phase::Drain | Phase::Done) {
            app.with_errors(|primary, cleanup| {
                if primary.is_some() || !cleanup.is_empty() {
                    Err(format!(
                        "unexpected runtime failure: {primary:?}; cleanup={cleanup:?}"
                    ))
                } else {
                    Ok(())
                }
            })?;
        }
        match &mut self.phase {
            Phase::Root if app.is_surface_presented(ROOT) => {
                if let Some(group) = self.stale.take() {
                    self.receipt = Some(
                        app.request_command(NativeCommand::Move {
                            source: DockMoveSource::Tabs(group),
                            target: DockMoveTarget::Dock {
                                placement: super::DockPlacement::After(ITEMS[0]),
                                mode: TabGroupDockMode::MergeTabs,
                            },
                        })
                        .map_err(|error| error.to_string())?,
                    );
                    self.phase = Phase::RejectStale;
                } else {
                    self.create(app, 1)?;
                }
            }
            Phase::RejectStale => {
                if let Some(status) = egui_native_smoke::poll_action_receipt(&mut self.receipt) {
                    if status
                        != Ok(NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
                            DockspaceActionRejection::Conflict,
                        )))
                    {
                        return Err(format!("foreign capability returned {status:?}"));
                    }
                    self.create(app, 1)?;
                }
            }
            Phase::Creating { index, target } => {
                if target.is_none() {
                    if let Some(status) = egui_native_smoke::poll_action_receipt(&mut self.receipt)
                    {
                        match status {
                            Ok(NativeActionStatus::Dock(DockspaceActionStatus::Requested {
                                outcome:
                                    DockspaceActionOutcome::NativeRootTearOffRequested {
                                        target_surface,
                                        items,
                                        ..
                                    },
                                transition: _,
                            })) if items == [ITEMS[*index]] => *target = Some(target_surface),
                            status => return Err(format!("native creation returned {status:?}")),
                        }
                    }
                }
                if let Some(surface) = target {
                    let item = ITEMS[*index];
                    if app.is_surface_presented(*surface)
                        && app.with_view(|view| view.item(item).unwrap().surface() == *surface)
                        && app.is_quiescent()
                    {
                        if *index == 1 {
                            self.create(app, 2)?;
                        } else {
                            context.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(
                                680.0, 480.0,
                            )));
                            self.phase = Phase::Resize;
                        }
                    }
                }
            }
            Phase::Resize => {
                if context.input(|input| {
                    input.viewport().inner_rect.is_some_and(|rect| {
                        (rect.width() - 680.0).abs() < 2.0 && (rect.height() - 480.0).abs() < 2.0
                    })
                }) {
                    context.set_zoom_factor(1.25);
                    self.phase = Phase::Zoom;
                }
            }
            Phase::Zoom if (context.zoom_factor() - 1.25).abs() < 0.001 && app.is_quiescent() => {
                context.set_zoom_factor(1.0);
                self.phase = Phase::RestoreZoom;
            }
            Phase::RestoreZoom
                if (context.zoom_factor() - 1.0).abs() < 0.001 && app.is_quiescent() =>
            {
                if self.fail {
                    self.phase = Phase::Fail;
                } else {
                    self.phase = Phase::DockBack {
                        index: 1,
                        progress: DockBackProgress::Ready,
                    };
                }
            }
            Phase::AwaitFatal => {
                app.with_errors(|primary, cleanup| {
                    if !cleanup.is_empty() {
                        return Err(format!("fatal cleanup failed: {cleanup:?}"));
                    }
                    if let Some(primary) = primary {
                        let detail = format!("{primary:?}");
                        if !detail.contains("InvalidWindowSnapshot") {
                            return Err(format!("unexpected injected failure: {detail}"));
                        }
                        eprintln!("recovery-smoke: observed deliberate invalid roster rejection");
                        self.phase = Phase::Drain;
                    }
                    Ok(())
                })?;
            }
            Phase::Drain => {
                app.with_errors(|primary, cleanup| {
                    if primary.is_none() || !cleanup.is_empty() {
                        Err(format!(
                            "fatal injection or cleanup failed: {primary:?}; {cleanup:?}"
                        ))
                    } else {
                        Ok(())
                    }
                })?;
                let children: Vec<_> = context.input(|input| {
                    input
                        .raw
                        .viewports
                        .keys()
                        .copied()
                        .filter(|id| *id != ViewportId::ROOT)
                        .collect()
                });
                for child in &children {
                    context.send_viewport_cmd_to(*child, ViewportCommand::Close);
                }
                // Close is programmatic here. It qualifies native destruction,
                // not delivery of a window-manager CloseRequested event.
                if app.is_shutdown_drained() {
                    eprintln!(
                        "recovery-smoke: semantic cleanup drained; remaining viewport declarations={}",
                        children.len()
                    );
                    self.finish(context, Ok(()));
                }
            }
            Phase::DockBack { index, progress } => {
                let item = ITEMS[*index];
                match progress {
                    DockBackProgress::Ready => {
                        self.receipt = Some(
                            app.request_command(NativeCommand::Move {
                                source: DockMoveSource::Item(item),
                                target: DockMoveTarget::Dock {
                                    placement: super::DockPlacement::After(ITEMS[0]),
                                    mode: TabGroupDockMode::MergeTabs,
                                },
                            })
                            .map_err(|error| error.to_string())?,
                        );
                        *progress = DockBackProgress::AwaitingResult;
                    }
                    DockBackProgress::AwaitingResult => {
                        if let Some(status) =
                            egui_native_smoke::poll_action_receipt(&mut self.receipt)
                        {
                            if !matches!(status, Ok(NativeActionStatus::Dock(DockspaceActionStatus::Applied(DockspaceActionOutcome::Docked { item: delivered, .. }))) if delivered == item)
                            {
                                return Err(format!("fresh docking returned {status:?}"));
                            }
                            *progress = DockBackProgress::Settling;
                        }
                    }
                    DockBackProgress::Settling => {}
                }
                if matches!(progress, DockBackProgress::Settling)
                    && app.is_quiescent()
                    && app.with_view(|view| view.item(item).unwrap().surface() == ROOT)
                {
                    if *index == 1 {
                        self.phase = Phase::DockBack {
                            index: 2,
                            progress: DockBackProgress::Ready,
                        };
                    } else {
                        self.finish(context, Ok(()));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn create(&mut self, app: &super::App, index: usize) -> Result<(), String> {
        let placement = NativeWindowPlacement::new(
            PhysicalRect::new(120.0 + index as f64 * 80.0, 120.0, 360.0, 260.0).unwrap(),
        );
        self.receipt = Some(
            app.request_command(NativeCommand::Move {
                source: DockMoveSource::Item(ITEMS[index]),
                target: DockMoveTarget::Native {
                    placement: Some(placement),
                },
            })
            .map_err(|error| error.to_string())?,
        );
        self.phase = Phase::Creating {
            index,
            target: None,
        };
        Ok(())
    }

    fn finish(&mut self, context: &egui::Context, result: Result<(), String>) {
        if !matches!(self.phase, Phase::Done) {
            *lock(&self.outcome) = Some(result);
            self.phase = Phase::Done;
        }
        context.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    }
}

impl eframe::App for Journey {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let shared = Arc::clone(&self.app);
        let mut app = lock(&shared);
        if matches!(self.phase, Phase::Fail) {
            // An invalid composition is a deterministic public protocol failure.
            // All output tokens and lifecycle callbacks still come from eframe.
            app.show_root_surfaces(ui, &[]);
            self.phase = Phase::AwaitFatal;
        } else {
            eframe::App::ui(&mut *app, ui, frame);
        }
        if let Err(error) = self.advance(&mut app, ui.ctx()) {
            self.finish(ui.ctx(), Err(error));
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}
