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
    Continuation,
    RetryRegistration {
        qualified_focus: bool,
    },
}

#[derive(Clone, Copy)]
struct PaneInputTarget {
    slot: Id,
    item: ItemId,
    content: DockspaceContentId,
    widget: Id,
}

struct PaneInputPacket {
    target: PaneInputTarget,
    events: Vec<Event>,
}

struct PendingInput {
    ready: bool,
    previous_request: Option<dockspace::runtime::DockspacePaneFocusRequest>,
    handoff: Handoff,
    packet: PaneInputPacket,
}

#[derive(Default)]
struct PaneKeyboard {
    viewports: ViewportIdMap<ViewportKeyboard>,
}

#[derive(Default)]
struct ViewportKeyboard {
    handoff: Option<PendingInput>,
    later_packets: VecDeque<PaneInputPacket>,
    incoming: Vec<Event>,
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
        if let Some(pending) = &mut input.handoff {
            if pending.ready || matches!(pending.handoff, Handoff::RetryRegistration { .. }) {
                input.handoff = None;
            } else {
                pending.ready = true;
            }
        }
        input.incoming.clear();
        // Pure edits can share an already validated widget. A packet containing
        // command keys needs its own pass so application callbacks observe its
        // original place in the input sequence.
        let waits_for_handoff = input.handoff.as_ref().is_some_and(|pending| {
            !pending.packet.events.is_empty()
                && (!input.later_packets.is_empty()
                    || pending.packet.events.iter().any(is_key_event)
                    || raw.events.iter().any(is_key_event))
        });
        if waits_for_handoff || !input.later_packets.is_empty() {
            // The render path can see a newer focus request. Assign the incoming
            // events only after checking the owner of the earlier packet.
            input.incoming = raw
                .events
                .extract_if(.., |event| is_keyboard_input(event))
                .collect();
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
    packet: PaneInputPacket,
    phase: DeliveryPhase,
}

#[derive(Clone, Copy)]
enum DeliveryPhase {
    AwaitingRegistration { qualified_focus: bool },
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
    let Some(widget) = context.panes.focus_target(item) else {
        return;
    };
    let mut events = take_keys_after(context.ui, start);
    if key == Key::Space && matches!(events.first(), Some(Event::Text(text)) if text == " ") {
        // egui-winit emits a text event immediately after the activating Space.
        events.remove(0);
    }
    defer(
        context,
        PaneInputPacket::new(context, item, widget, events),
        Handoff::Activation {
            header,
            awaiting_publication: false,
        },
    );
}

pub(super) fn continue_at(
    context: &RenderContext<'_, '_, '_>,
    item: ItemId,
    widget: Id,
    events: Vec<Event>,
) {
    defer(
        context,
        PaneInputPacket::new(context, item, widget, events),
        Handoff::Continuation,
    );
}

impl PaneInputPacket {
    fn new(
        context: &RenderContext<'_, '_, '_>,
        item: ItemId,
        widget: Id,
        events: Vec<Event>,
    ) -> Self {
        Self {
            target: PaneInputTarget {
                slot: slot_id(context),
                item,
                content: context.plan.content_identity(item),
                widget,
            },
            events,
        }
    }
}

fn defer(context: &RenderContext<'_, '_, '_>, packet: PaneInputPacket, handoff: Handoff) {
    let pending = PendingInput {
        ready: false,
        previous_request: context.pane_focus_request,
        handoff,
        packet,
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
    let PaneInputTarget {
        item,
        content,
        widget,
        ..
    } = pending.packet.target;
    if content != context.plan.content_identity(item)
        || context.panes.focus_target(item) != Some(widget)
    {
        retire_input(context);
        return PaneInputState::None;
    }
    if let Handoff::RetryRegistration { qualified_focus } = pending.handoff {
        return if context.pane_focus_request == pending.previous_request {
            PaneInputState::Ready(PaneInputDelivery {
                packet: pending.packet,
                phase: DeliveryPhase::AwaitingRegistration { qualified_focus },
            })
        } else {
            retire_input(context);
            PaneInputState::None
        };
    }
    if !pending.ready {
        // A layout retry has no new input and must not reapply the handoff.
        store(context, pending);
        return PaneInputState::Waiting;
    }
    let focused = context.ui.memory(egui::Memory::focused);
    let phase = match pending.handoff {
        Handoff::Continuation
            if focused == Some(widget)
                && context
                    .pane_focus_request
                    .is_none_or(|request| request.item() == Some(item))
                && super::can_route_keyboard(context)
                && pointer_keeps_focus(context.ui.ctx(), widget) =>
        {
            queue_incoming(context, pending.packet.target);
            pending.packet.events.extend(take_keys_after(context.ui, 0));
            DeliveryPhase::Registered
        }
        Handoff::Activation {
            header,
            awaiting_publication,
        } if focused == Some(header)
            && super::can_capture(context)
            && !crate::focus::input_may_surrender_focus(context.ui.ctx()) =>
        {
            let request = context.pane_focus_request;
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
                    pending.packet.events.extend(take_keys_after(context.ui, 0));
                    queue_incoming(context, pending.packet.target);
                    store(context, pending);
                    return PaneInputState::Waiting;
                }
                retire_input(context);
                return PaneInputState::None;
            }
            if request.and_then(|request| request.item()) != Some(item) {
                retire_input(context);
                return PaneInputState::None;
            }
            pending.packet.events.extend(take_keys_after(context.ui, 0));
            queue_incoming(context, pending.packet.target);
            DeliveryPhase::AwaitingRegistration {
                qualified_focus: false,
            }
        }
        _ => {
            retire_input(context);
            return PaneInputState::None;
        }
    };
    PaneInputState::Ready(PaneInputDelivery {
        packet: pending.packet,
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
        .handoff = Some(pending);
    if !retry {
        ctx.request_repaint();
    }
}

fn take(context: &RenderContext<'_, '_, '_>) -> Option<PendingInput> {
    let ctx = context.ui.ctx();
    let plugin = ctx.plugin_opt::<PaneKeyboard>()?;
    let mut keyboard = plugin.lock();
    let input = keyboard.viewports.get_mut(&ctx.viewport_id())?;
    let slot = slot_id(context);
    if input.handoff.is_some() {
        return input
            .handoff
            .take_if(|pending| pending.packet.target.slot == slot);
    }
    input
        .later_packets
        .front()
        .filter(|packet| packet.target.slot == slot)?;
    Some(PendingInput {
        ready: true,
        previous_request: None,
        handoff: Handoff::Continuation,
        packet: input.later_packets.pop_front()?,
    })
}

fn queue_incoming(context: &RenderContext<'_, '_, '_>, target: PaneInputTarget) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
        && !input.incoming.is_empty()
    {
        input.later_packets.push_back(PaneInputPacket {
            target,
            events: std::mem::take(&mut input.incoming),
        });
    }
}

fn retire_input(context: &RenderContext<'_, '_, '_>) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        input.later_packets.clear();
        // The incoming packet has never been delivered. Let this pass route it.
        ctx.input_mut(|state| {
            drop(
                state
                    .events
                    .splice(..0, std::mem::take(&mut input.incoming)),
            );
        });
    }
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
            packet: PaneInputPacket::new(context, item, widget, take_keys_after(context.ui, 0)),
            phase: DeliveryPhase::AwaitingRegistration {
                qualified_focus: false,
            },
        })
    }

    pub(in crate::render) fn targets(&self, item: ItemId) -> bool {
        self.packet.target.item == item
    }

    pub(in crate::render) fn awaits_registration(&self) -> bool {
        matches!(self.phase, DeliveryPhase::AwaitingRegistration { .. })
    }

    pub(in crate::render) fn qualified_focus_in_discarded_pass(&self) -> bool {
        matches!(
            self.phase,
            DeliveryPhase::AwaitingRegistration {
                qualified_focus: true
            }
        )
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
            && ui.memory(|memory| memory.has_focus(self.packet.target.widget))
            && crate::focus::pane_registered_focus_target(ui, clip, self.packet.target.widget, true)
    }

    pub(in crate::render) fn settle_registration(
        self,
        context: &mut RenderContext<'_, '_, '_>,
        target: Option<Id>,
        preapplied: bool,
    ) {
        if let DeliveryPhase::AwaitingRegistration { qualified_focus } = self.phase
            && context.ui.ctx().will_discard()
        {
            defer(
                context,
                self.packet,
                Handoff::RetryRegistration {
                    qualified_focus: qualified_focus || preapplied,
                },
            );
        } else if self.awaits_registration()
            && target == Some(self.packet.target.widget)
            && context
                .ui
                .memory(|memory| memory.has_focus(self.packet.target.widget))
        {
            defer(context, self.packet, Handoff::Continuation);
        } else {
            self.release_shortcuts(context.pane_shortcuts);
        }
    }

    pub(in crate::render) fn release_shortcuts(self, shortcuts: &mut Vec<Event>) {
        if self.awaits_registration() {
            shortcuts.extend(self.packet.events.into_iter().filter(is_key_event));
        }
    }

    pub(in crate::render) fn replay(self, context: &Context) -> ScopedPaneInput {
        // The ordinary widget API exposes an unaddressed event list. Keep the
        // replay local to this pane, including when another pane paints later.
        let mut events = self.packet.events;
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
