//! Viewport-local keyboard input awaiting an exact pane focus handoff.

use std::{collections::VecDeque, time::Duration};

use dockspace::{
    model::{ItemId, TabKey},
    runtime::{DockspaceContentId, PanePaintRecord},
};
use egui::{
    Context, Event, FocusDirection, Id, Key, RawInput, Rect, Ui, ViewportId, ViewportIdMap,
};

use super::super::RenderContext;

#[derive(Clone, Copy)]
enum Handoff {
    Activation {
        header: Id,
        awaiting_publication: bool,
    },
    Continuation {
        widget: Id,
    },
    RetryRegistration {
        widget: Id,
    },
}

struct PendingInput {
    ready: bool,
    item: ItemId,
    content: DockspaceContentId,
    previous_request: Option<dockspace::runtime::DockspacePaneFocusRequest>,
    handoff: Handoff,
    events: Vec<Event>,
}

#[derive(Default)]
struct PaneKeyboard {
    viewports: ViewportIdMap<ViewportKeyboard>,
}

#[derive(Default)]
struct ViewportKeyboard {
    handoff: Option<(Id, PendingInput)>,
    later_packets: VecDeque<Vec<Event>>,
}

impl egui::plugin::Plugin for PaneKeyboard {
    fn debug_name(&self) -> &'static str {
        "egui_dockspace::pane_keyboard"
    }

    fn input_hook(&mut self, context: &Context, raw: &mut RawInput) {
        let viewport = raw.viewport_id;
        if !raw.focused
            || raw.events.contains(&Event::WindowFocused(false))
            || (viewport != ViewportId::ROOT && context.cumulative_pass_nr_for(viewport) == 0)
        {
            self.viewports.remove(&viewport);
            return;
        }
        let Some(input) = self.viewports.get_mut(&viewport) else {
            return;
        };
        if context.viewport_for(viewport, |state| state.output.num_completed_passes != 0) {
            return;
        }
        // Count original input passes at ingress. egui's frame number is only
        // advanced by run_ui, and a sizing pass does not deliver another packet.
        if let Some((_, pending)) = &mut input.handoff {
            if pending.ready || matches!(pending.handoff, Handoff::RetryRegistration { .. }) {
                input.handoff = None;
            } else {
                pending.ready = true;
            }
        }
        // Pure edits can share an already validated widget. A packet containing
        // command keys needs its own pass so application callbacks observe its
        // original place in the input sequence.
        let waits_for_handoff = input.handoff.as_ref().is_some_and(|(_, pending)| {
            !pending.events.is_empty()
                && (!input.later_packets.is_empty()
                    || pending.events.iter().any(is_key_event)
                    || raw.events.iter().any(is_key_event))
        });
        if waits_for_handoff || !input.later_packets.is_empty() {
            // Commands can change focus or read the document. Dispatch the next
            // keyboard packet only after the older packet has reached the pane
            // and its remaining shortcuts have reached the application.
            let next: Vec<_> = raw
                .events
                .extract_if(.., |event| is_keyboard_input(event))
                .collect();
            if !next.is_empty() {
                input.later_packets.push_back(next);
            }
            if !waits_for_handoff && let Some(packet) = input.later_packets.pop_front() {
                drop(raw.events.splice(..0, packet));
            }
        }
    }

    fn output_hook(&mut self, context: &Context, output: &mut egui::FullOutput) {
        self.viewports.retain(|viewport, input| {
            (input.handoff.is_some() || !input.later_packets.is_empty())
                && (*viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(*viewport) != 0)
        });
        // egui has already popped the viewport which just ended. Wake the
        // viewports that actually hold input, including native children.
        for (&viewport, input) in &self.viewports {
            if !input.later_packets.is_empty() {
                context.request_repaint_of(viewport);
                if let Some(output) = output.viewport_output.get_mut(&viewport) {
                    output.repaint_delay = Duration::ZERO;
                }
            }
        }
    }
}

pub(in crate::render) struct PaneInputDelivery {
    item: ItemId,
    widget: Id,
    events: Vec<Event>,
    phase: DeliveryPhase,
}

#[derive(Clone, Copy)]
enum DeliveryPhase {
    AwaitingRegistration,
    Registered,
}

pub(super) fn activate(
    context: &RenderContext<'_, '_, '_>,
    start: usize,
    key: Key,
    tab: TabKey,
    header: Id,
) {
    let Some(item) = context.plan.tab_content_focus_target(tab) else {
        return;
    };
    let mut events = take_keys_after(context.ui, start);
    if key == Key::Space && matches!(events.first(), Some(Event::Text(text)) if text == " ") {
        // egui-winit emits a text event immediately after the activating Space.
        events.remove(0);
    }
    defer(
        context,
        item,
        Handoff::Activation {
            header,
            awaiting_publication: false,
        },
        events,
    );
}

pub(super) fn continue_at(
    context: &RenderContext<'_, '_, '_>,
    item: ItemId,
    widget: Id,
    events: Vec<Event>,
) {
    defer(context, item, Handoff::Continuation { widget }, events);
}

fn defer(context: &RenderContext<'_, '_, '_>, item: ItemId, handoff: Handoff, events: Vec<Event>) {
    let pending = PendingInput {
        ready: false,
        item,
        content: context.plan.content_identity(item),
        previous_request: context.pane_focus_request,
        handoff,
        events,
    };
    store(context, pending);
}

pub(in crate::render) enum PaneInputState {
    None,
    Waiting,
    Ready(PaneInputDelivery),
}

pub(super) fn prepare(context: &RenderContext<'_, '_, '_>) -> PaneInputState {
    let Some(mut pending) = take(context) else {
        return PaneInputState::None;
    };
    if pending.content != context.plan.content_identity(pending.item) {
        return PaneInputState::None;
    }
    if let Handoff::RetryRegistration { widget } = pending.handoff {
        return if context.pane_focus_request == pending.previous_request {
            PaneInputState::Ready(PaneInputDelivery {
                item: pending.item,
                widget,
                events: pending.events,
                phase: DeliveryPhase::AwaitingRegistration,
            })
        } else {
            PaneInputState::None
        };
    }
    if !pending.ready {
        // A layout retry has no new input and must not reapply the handoff.
        store(context, pending);
        return PaneInputState::Waiting;
    }
    let focused = context.ui.memory(egui::Memory::focused);
    let (widget, phase) = match pending.handoff {
        Handoff::Continuation { widget }
            if focused == Some(widget)
                && context
                    .pane_focus_request
                    .is_none_or(|request| request.item() == Some(pending.item))
                && super::can_route_keyboard(context)
                && pointer_keeps_focus(context.ui.ctx(), widget) =>
        {
            pending.events.extend(take_keys_after(context.ui, 0));
            (widget, DeliveryPhase::Registered)
        }
        Handoff::Activation {
            header,
            awaiting_publication,
        } if focused == Some(header)
            && super::can_capture(context)
            && !crate::focus::input_may_surrender_focus(context.ui.ctx()) =>
        {
            let request = context.pane_focus_request;
            pending.events.extend(take_keys_after(context.ui, 0));
            context
                .ui
                .memory_mut(|memory| memory.move_focus(FocusDirection::None));
            if request == pending.previous_request || request.is_none() {
                if !awaiting_publication {
                    // An activation reduced in this frame becomes an admitted
                    // request only upon publication. Save its input for one run.
                    pending.ready = false;
                    pending.handoff = Handoff::Activation {
                        header,
                        awaiting_publication: true,
                    };
                    store(context, pending);
                    return PaneInputState::Waiting;
                }
                return PaneInputState::None;
            }
            if request.and_then(|request| request.item()) != Some(pending.item) {
                return PaneInputState::None;
            }
            let Some(target) = context.panes.focus_target(pending.item) else {
                return PaneInputState::None;
            };
            (target, DeliveryPhase::AwaitingRegistration)
        }
        _ => return PaneInputState::None,
    };
    PaneInputState::Ready(PaneInputDelivery {
        item: pending.item,
        widget,
        events: pending.events,
        phase,
    })
}

fn pointer_keeps_focus(context: &Context, widget: Id) -> bool {
    let (pressed, down) =
        context.input(|input| (input.pointer.any_pressed(), input.pointer.any_down()));
    if !pressed && !down && !crate::focus::input_may_surrender_focus(context) {
        return true;
    }
    // egui resolves clicks from the previous pass. A click on the same editor
    // keeps its queued text, even with the default click-to-surrender policy.
    context.interaction_snapshot(|snapshot| snapshot.clicked == Some(widget))
        || (down
            && context.viewport(|viewport| viewport.hits.click.is_some_and(|hit| hit.id == widget)))
}

fn store(context: &RenderContext<'_, '_, '_>, pending: PendingInput) {
    let ctx = context.ui.ctx();
    let retry = matches!(pending.handoff, Handoff::RetryRegistration { .. });
    ctx.plugin_or_default::<PaneKeyboard>()
        .lock()
        .viewports
        .entry(ctx.viewport_id())
        .or_default()
        .handoff = Some((slot_id(context), pending));
    if !retry {
        ctx.request_repaint();
    }
}

fn take(context: &RenderContext<'_, '_, '_>) -> Option<PendingInput> {
    let ctx = context.ui.ctx();
    ctx.plugin_opt::<PaneKeyboard>()?
        .lock()
        .viewports
        .get_mut(&ctx.viewport_id())?
        .handoff
        .take_if(|(id, _)| *id == slot_id(context))
        .map(|(_, pending)| pending)
}

impl PaneInputDelivery {
    pub(super) fn for_focus_request(context: &RenderContext<'_, '_, '_>) -> Option<Self> {
        if !context.pointer_authority.supports_preview_gestures()
            || !super::can_route_keyboard(context)
            || crate::focus::input_may_surrender_focus(context.ui.ctx())
            || !context
                .ui
                .input(|input| input.events.iter().any(is_keyboard_input))
        {
            return None;
        }
        let request = context.pane_focus_request?;
        let item = request.item()?;
        let widget = context.panes.focus_target(item)?;
        let prior_focus = context.ui.memory(egui::Memory::focused);
        if prior_focus == Some(widget)
            || prior_focus.is_some_and(|id| {
                // An application widget composed before this surface may have
                // already read the unaddressed keyboard events this pass.
                context
                    .ui
                    .ctx()
                    .viewport(|viewport| viewport.this_pass.widgets.get(id).is_some())
            })
        {
            return None;
        }
        crate::focus::pane_focus_handoff_pending(
            context.ui.ctx(),
            context.instance_id,
            request,
            widget,
            prior_focus,
        )
        .then(|| Self {
            item,
            widget,
            events: take_keys_after(context.ui, 0),
            phase: DeliveryPhase::AwaitingRegistration,
        })
    }

    pub(in crate::render) fn targets(&self, item: ItemId) -> bool {
        self.item == item
    }

    pub(in crate::render) fn awaits_registration(&self) -> bool {
        matches!(self.phase, DeliveryPhase::AwaitingRegistration)
    }

    pub(in crate::render) fn take_for(
        delivery: &mut Option<Self>,
        ui: &Ui,
        pane: PanePaintRecord<'_>,
    ) -> Option<Self> {
        let expected = delivery.as_ref()?;
        (expected.targets(pane.item()) && super::super::actions::effective_enabled(ui, true))
            .then(|| delivery.take())
            .flatten()
    }

    pub(in crate::render) fn can_replay(&self, ui: &Ui, clip: Rect) -> bool {
        matches!(self.phase, DeliveryPhase::Registered)
            && ui.memory(|memory| memory.has_focus(self.widget))
            && crate::focus::pane_registered_focus_target(ui, clip, self.widget, true)
    }

    pub(in crate::render) fn settle_registration(
        self,
        context: &mut RenderContext<'_, '_, '_>,
        target: Option<Id>,
    ) {
        if self.awaits_registration() && context.ui.ctx().will_discard() {
            defer(
                context,
                self.item,
                Handoff::RetryRegistration {
                    widget: self.widget,
                },
                self.events,
            );
        } else if self.awaits_registration()
            && target == Some(self.widget)
            && context.ui.memory(|memory| memory.has_focus(self.widget))
        {
            continue_at(context, self.item, self.widget, self.events);
        } else {
            self.release_shortcuts(context.pane_shortcuts);
        }
    }

    pub(in crate::render) fn release_shortcuts(self, shortcuts: &mut Vec<Event>) {
        shortcuts.extend(self.events.into_iter().filter(is_key_event));
    }

    pub(in crate::render) fn replay(self, context: &Context) -> ScopedPaneInput {
        // The ordinary widget API exposes an unaddressed event list. Keep the
        // replay local to this pane, including when another pane paints later.
        let mut events = self.events;
        context.input_mut(|input| {
            events.append(&mut input.events);
            input.events = events;
        });
        ScopedPaneInput {
            context: context.clone(),
            active: true,
        }
    }
}

pub(in crate::render) struct ScopedPaneInput {
    context: Context,
    active: bool,
}

impl ScopedPaneInput {
    pub(in crate::render) fn finish(mut self, shortcuts: &mut Vec<Event>) {
        self.active = false;
        // TextEdit reads text and clipboard events without removing them from
        // egui's list. Explicitly consumed keys have already been removed.
        // Return the remaining keys after all panes have painted so that host
        // shortcuts can observe the effect of the editor's earlier input.
        self.context.input_mut(|input| {
            shortcuts.extend(
                input
                    .events
                    .extract_if(.., |event| is_keyboard_input(event))
                    .filter(is_key_event),
            );
        });
    }
}

impl Drop for ScopedPaneInput {
    fn drop(&mut self) {
        if self.active {
            self.context.input_mut(|input| {
                input.events.retain(|event| !is_keyboard_input(event));
            });
        }
    }
}

fn is_key_event(event: &Event) -> bool {
    matches!(event, Event::Key { .. })
}

fn slot_id(context: &RenderContext<'_, '_, '_>) -> Id {
    context
        .instance_id
        .with(("pane-keyboard-handoff", context.plan.surface()))
}

/// Takes the keyboard portion of a suffix while leaving pointer and system events in place.
pub(super) fn take_keys_after(ui: &Ui, start: usize) -> Vec<Event> {
    ui.input_mut(|input| {
        let mut index = 0;
        input
            .events
            .extract_if(.., |event| {
                let take = index >= start && is_keyboard_input(event);
                index += 1;
                take
            })
            .collect()
    })
}

pub(super) fn is_keyboard_input(event: &Event) -> bool {
    matches!(
        event,
        Event::Key { .. }
            | Event::Text(_)
            | Event::Ime(_)
            | Event::Paste(_)
            | Event::Cut
            | Event::Copy
    )
}
