//! Frozen input and synchronous callback scope for one official egui run.

use std::cell::RefCell;
use std::sync::{Arc, PoisonError};

use egui::{Context, Event, Modifiers, RawInput, Ui, ViewportId, ViewportIdMap};

use crate::pointer::{ProductPointerInputSnapshot, event_modifiers, has_foreign_drag};

use super::{RunIdentity, active_run_portal};

pub(crate) const RUN_SCOPE_TAG: &str = "egui_dockspace::official_run_scope";

pub(super) struct RunInput {
    events: Vec<Event>,
    focused: bool,
    receivers: ProductPointerInputSnapshot,
}

/// Identity of a real egui input pass, unique within one Context.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputPassId(u64);

#[derive(Default)]
pub(super) struct RunInputSnapshot {
    last_id: u64,
    viewports: ViewportIdMap<ViewportHistory>,
}

#[derive(Default)]
struct ViewportHistory {
    lifetime: Arc<ViewportLifetime>,
    last_owned_pass: Option<InputPassId>,
}

#[derive(Default)]
pub(super) struct ViewportLifetime;

impl egui::plugin::Plugin for RunInputSnapshot {
    fn debug_name(&self) -> &'static str {
        "egui_dockspace::run_input_snapshot"
    }

    fn input_hook(&mut self, context: &Context, input: &mut RawInput) {
        let viewport = input.viewport_id;
        let Some(portal) = active_run_portal(context) else {
            // Direct egui passes can consume input without opening a docking run.
            self.break_continuity(viewport);
            return;
        };
        let mut portal = portal.lock().unwrap_or_else(PoisonError::into_inner);
        // egui can reject recursive root borrowing before on_begin_pass runs.
        if in_run_callback(context) {
            portal.poisoned = true;
            self.break_continuity(viewport);
            return;
        }
        if portal.viewport != viewport || portal.poisoned {
            self.break_continuity(viewport);
            return;
        }
        self.last_id = self
            .last_id
            .checked_add(1)
            .expect("egui input pass identity exhausted");
        let pass = InputPassId(self.last_id);
        let history = self.viewports.entry(viewport).or_default();
        let previous = history.last_owned_pass.replace(pass);
        if portal.last_input_pass.replace(pass).is_none() {
            portal.viewport_lifetime = Some(Arc::downgrade(&history.lifetime));
            // The previous InputState already includes events from passes that
            // may have run while this Dockspace was absent.
            portal.initial_modifiers = context.input_for(viewport, |previous| {
                if previous.focused {
                    previous.modifiers
                } else {
                    Modifiers::NONE
                }
            });
            // egui can reuse a viewport ID after destroying the old viewport.
            portal.preceding_input_pass = previous.filter(|_| {
                viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(viewport) != 0
            });
        }
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let Some(portal) = active_run_portal(ui.ctx()) else {
            return;
        };
        let mut portal = portal.lock().unwrap_or_else(PoisonError::into_inner);
        if in_run_callback(ui.ctx()) {
            // A raw recursive Context::run_ui has no inherited Ui stack tag.
            portal.poisoned = true;
            return;
        }
        if portal.poisoned || portal.unobserved_input || portal.viewport != ui.ctx().viewport_id() {
            return;
        }
        if let Some(first) = &portal.input {
            // egui re-runs input hooks on discard. A plugin may add new edges
            // after the packet we froze. That suffix cannot share its receivers.
            portal.unobserved_input = (!first.receivers.foreign_drag && has_foreign_drag(ui.ctx()))
                || ui.ctx().input(|input| {
                    (!input.raw.focused && first.focused)
                        || input.raw.events.iter().any(affects_pointer_stream)
                });
        } else {
            let (events, focused) = ui
                .ctx()
                .input(|input| (input.raw.events.clone(), input.raw.focused));
            portal.input = Some(Arc::new(RunInput {
                events,
                focused,
                receivers: ProductPointerInputSnapshot::capture(ui.ctx()),
            }));
        }
        if !portal.unobserved_input {
            portal.observed_input_pass = portal.last_input_pass;
        }
    }

    fn output_hook(&mut self, context: &Context, _output: &mut egui::FullOutput) {
        self.viewports.retain(|&viewport, _| {
            viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(viewport) != 0
        });
    }
}

impl RunInputSnapshot {
    fn break_continuity(&mut self, viewport: ViewportId) {
        if let Some(history) = self.viewports.get_mut(&viewport) {
            history.last_owned_pass = None;
        }
    }
}

fn affects_pointer_stream(event: &Event) -> bool {
    event_modifiers(event).is_some()
        || matches!(
            event,
            Event::PointerMoved(_)
                | Event::PointerGone
                | Event::Touch { .. }
                | Event::WindowFocused(_)
        )
}

#[derive(Clone)]
pub(crate) struct ProductPointerPacket {
    run: Arc<RunIdentity>,
    input: Arc<RunInput>,
    initial_modifiers: Modifiers,
    preceding_input_pass: Option<InputPassId>,
    observed_input_pass: InputPassId,
    observed_at: Option<std::time::Duration>,
}

impl ProductPointerPacket {
    pub(crate) fn same_run(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.run, &other.run)
    }

    pub(crate) fn events(&self) -> &[egui::Event] {
        &self.input.events
    }

    pub(crate) fn focused(&self) -> bool {
        self.input.focused
    }

    pub(crate) const fn initial_modifiers(&self) -> Modifiers {
        self.initial_modifiers
    }

    pub(crate) fn receivers(&self) -> &ProductPointerInputSnapshot {
        &self.input.receivers
    }

    pub(crate) const fn preceding_input_pass(&self) -> Option<InputPassId> {
        self.preceding_input_pass
    }

    pub(crate) const fn observed_input_pass(&self) -> InputPassId {
        self.observed_input_pass
    }

    pub(crate) const fn observed_at(&self) -> Option<std::time::Duration> {
        self.observed_at
    }
}

pub(crate) fn pointer_packet(ui: &Ui) -> Option<ProductPointerPacket> {
    let portal = active_run_portal(ui.ctx())?;
    let portal = portal.lock().unwrap_or_else(PoisonError::into_inner);
    let scope = ui_run_scope(ui)?;
    if portal.poisoned
        || portal.current_pass != Some(scope.pass)
        || portal.viewport != ui.ctx().viewport_id()
        || !Arc::ptr_eq(&scope.run, &portal.run)
    {
        return None;
    }
    let input = Arc::clone(portal.input.as_ref()?);
    Some(ProductPointerPacket {
        run: Arc::clone(&portal.run),
        input,
        initial_modifiers: portal.initial_modifiers,
        preceding_input_pass: portal.preceding_input_pass,
        observed_input_pass: portal.observed_input_pass?,
        observed_at: portal.observed_at,
    })
}

#[derive(Clone)]
pub(crate) struct RunScope {
    pub(super) run: Arc<RunIdentity>,
    pub(super) pass: InputPassId,
}

thread_local! {
    static RUN_CALLBACKS: RefCell<Vec<RunCallback>> = const { RefCell::new(Vec::new()) };
}

struct RunCallback {
    context: Context,
    viewport: ViewportId,
    frame: u64,
    completed_pass: u64,
    pass_index: usize,
    scope: RunScope,
}

fn in_run_callback(context: &Context) -> bool {
    RUN_CALLBACKS.with(|callbacks| {
        callbacks
            .borrow()
            .iter()
            .any(|callback| callback.context == *context)
    })
}

#[must_use = "keep the guard alive until the synchronous application callback returns"]
pub(crate) struct RunCallbackGuard;

impl RunScope {
    pub(crate) fn enter(&self, ui: &Ui) -> RunCallbackGuard {
        RUN_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().push(RunCallback {
                context: ui.ctx().clone(),
                viewport: ui.ctx().viewport_id(),
                frame: ui.ctx().cumulative_frame_nr(),
                completed_pass: ui.ctx().cumulative_pass_nr(),
                pass_index: ui.ctx().current_pass_index(),
                scope: self.clone(),
            });
        });
        RunCallbackGuard
    }
}

impl Drop for RunCallbackGuard {
    fn drop(&mut self) {
        RUN_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().pop();
        });
    }
}

pub(super) fn ui_run_scope(ui: &Ui) -> Option<RunScope> {
    let tagged = ui.stack().iter().find_map(|stack| {
        stack
            .tags()
            .get_downcast::<RunScope>(RUN_SCOPE_TAG)
            .cloned()
    });
    RUN_CALLBACKS.with(|callbacks| {
        let callbacks = callbacks.borrow();
        let callback = callbacks.last()?;
        let source_matches = match tagged {
            Some(scope) => {
                scope.pass == callback.scope.pass && Arc::ptr_eq(&scope.run, &callback.scope.run)
            }
            None => {
                // Areas create independent root Uis. Limit the callback-local
                // fallback to their declared area root and matching layer ID.
                ui.stack()
                    .iter()
                    .last()
                    .is_some_and(|root| root.is_area_ui() && root.unique_id == ui.layer_id().id)
            }
        };
        (callback.context == *ui.ctx()
            && callback.viewport == ui.ctx().viewport_id()
            && callback.frame == ui.ctx().cumulative_frame_nr()
            && callback.completed_pass == ui.ctx().cumulative_pass_nr()
            && callback.pass_index == ui.ctx().current_pass_index()
            && source_matches)
            .then(|| callback.scope.clone())
    })
}
