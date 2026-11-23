//! Viewport-local keyboard input awaiting an exact pane focus handoff.

use std::{
    collections::{HashSet, VecDeque},
    time::Duration,
};

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
    RetryRegistration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaneInputTarget {
    slot: Id,
    item: ItemId,
    content: DockspaceContentId,
    widget: Option<Id>,
}

struct PaneInputPacket {
    target: PaneInputTarget,
    events: Vec<Event>,
    queued_at_ingress: bool,
    framework_keys: bool,
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
    last_viewport: Option<ViewportId>,
}

#[derive(Default)]
struct ViewportKeyboard {
    handoff: Option<PendingInput>,
    later_packets: VecDeque<PaneInputPacket>,
    incoming: Vec<Event>,
    owner_seen: bool,
    unclaimed_shortcuts: bool,
    physical_keys: Option<HashSet<Key>>,
    last_focus: Option<Id>,
}

impl egui::plugin::Plugin for PaneKeyboard {
    fn debug_name(&self) -> &'static str {
        "egui_dockspace::pane_keyboard"
    }

    fn input_hook(&mut self, context: &Context, raw: &mut RawInput) {
        let viewport = raw.viewport_id;
        let previous_viewport = self.last_viewport.replace(viewport);
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
        if !std::mem::take(&mut input.owner_seen) {
            // The previous input batch never reached the Dockspace slot that
            // was holding it. Release its fresh input and retire addressed edits.
            raw.events.splice(..0, std::mem::take(&mut input.incoming));
            self.viewports.remove(&viewport);
            return;
        }
        if let Some(pending) = &mut input.handoff {
            if pending.ready || matches!(pending.handoff, Handoff::RetryRegistration) {
                input.handoff = None;
            } else {
                pending.ready = true;
            }
        }
        input.unclaimed_shortcuts = false;
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
            input.physical_keys = physical_keys(context, raw);
            // A new standalone application chord remains available to panels
            // composed before docking. If no panel consumes it, the pane will
            // see it in its own packet, after the pending edit.
            input.unclaimed_shortcuts = input.handoff.as_ref().is_some_and(|pending| {
                pending
                    .packet
                    .events
                    .iter()
                    .all(|event| !is_key_event(event))
                    && input.later_packets.is_empty()
                    && raw
                        .events
                        .iter()
                        .filter(|event| is_keyboard_input(event))
                        .all(|event| {
                            matches!(event, Event::Key { modifiers, .. }
                            if (modifiers.command || modifiers.ctrl)
                                && egui::EventFilter::default().matches(event))
                        })
            });
            // The render path can see a newer focus request. Assign the incoming
            // events only after checking the owner of the earlier packet.
            if !input.unclaimed_shortcuts {
                input.incoming = raw
                    .events
                    .extract_if(.., |event| is_keyboard_input(event))
                    .collect();
            }
        }
        let previous_focus = if previous_viewport == Some(viewport) {
            context.memory(egui::Memory::focused)
        } else {
            input.last_focus
        };
        let next_packet = match &mut input.handoff {
            Some(PendingInput {
                handoff: Handoff::Continuation,
                ready: true,
                packet,
                ..
            }) => Some(packet),
            Some(_) => None,
            None => input.later_packets.front_mut(),
        };
        if let Some(packet) = next_packet
            && packet.queued_at_ingress
            && packet.target.widget.is_some()
            && packet.target.widget == previous_focus
            && !raw.events.iter().any(|event| {
                matches!(
                    event,
                    Event::PointerButton { .. } | Event::AccessKitActionRequest(_)
                )
            })
        {
            // Feed keys that inform egui's focus traversal into its normal
            // begin-pass processing. Keep the edit suffix intact so cursor
            // motion between text events stays in widget order.
            let filter = egui::EventFilter::default();
            let first_edit = packet
                .events
                .iter()
                .position(|event| !is_key_event(event))
                .unwrap_or(packet.events.len());
            let keys: Vec<_> = packet
                .events
                .extract_if(..first_edit, |event| {
                    matches!(event, Event::Key { pressed: true, .. }) && !filter.matches(event)
                })
                .collect();
            packet.framework_keys = !keys.is_empty();
            if packet.framework_keys && input.physical_keys.is_none() {
                input.physical_keys =
                    Some(context.input_for(viewport, |state| state.keys_down.clone()));
            }
            raw.events.splice(..0, keys);
        }
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        self.apply_physical_keys(ui.ctx());
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if let Some(input) = self.viewports.get_mut(&ui.ctx().viewport_id()) {
            input.last_focus = ui.memory(egui::Memory::focused);
        }
    }

    fn output_hook(&mut self, context: &Context, output: &mut egui::FullOutput) {
        self.viewports.retain(|viewport, input| {
            (input.handoff.is_some()
                || !input.later_packets.is_empty()
                || !input.incoming.is_empty())
                && (*viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(*viewport) != 0)
        });
        // egui has already popped the viewport which just ended. Wake the
        // viewports that actually hold input, including native children.
        for (&viewport, input) in &self.viewports {
            if !input.later_packets.is_empty() || !input.incoming.is_empty() {
                context.request_repaint_of(viewport);
                if let Some(output) = output.viewport_output.get_mut(&viewport) {
                    output.repaint_delay = Duration::ZERO;
                }
            }
        }
    }
}

impl PaneKeyboard {
    fn apply_physical_keys(&mut self, context: &Context) {
        if let Some(input) = self.viewports.get_mut(&context.viewport_id())
            && let Some(keys) = input.physical_keys.take()
        {
            context.input_mut(|state| state.keys_down = keys);
        }
    }
}

fn physical_keys(context: &Context, raw: &RawInput) -> Option<HashSet<Key>> {
    // Packet replay changes when widgets see key events. Polling must still
    // reflect the platform's latest press and release observations.
    let mut edges = raw
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Key { key, pressed, .. } => Some((*key, *pressed)),
            _ => None,
        })
        .peekable();
    edges.peek()?;
    let mut keys = context.input_for(raw.viewport_id, |input| input.keys_down.clone());
    for (key, pressed) in edges {
        if pressed {
            keys.insert(key);
        } else {
            keys.remove(&key);
        }
    }
    Some(keys)
}

pub(in crate::render) struct PaneInputDelivery {
    packet: PaneInputPacket,
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
    let widget = context.panes.focus_target(item);
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
        PaneInputPacket::new(context, item, Some(widget), events),
        Handoff::Continuation,
    );
}

impl PaneInputPacket {
    fn new(
        context: &RenderContext<'_, '_, '_>,
        item: ItemId,
        widget: Option<Id>,
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
            queued_at_ingress: false,
            framework_keys: false,
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
    if let Some(plugin) = context.ui.ctx().plugin_opt::<PaneKeyboard>() {
        // Manual begin_pass/end_pass hosts do not run egui's on_begin_pass.
        plugin.lock().apply_physical_keys(context.ui.ctx());
    }
    if context.ui.ctx().current_pass_index() != 0 && !registration_retry(context) {
        // egui calls the UI again with the same input to resolve layout. The
        // following packet is eligible at the next input hook, never here.
        return PaneInputState::Waiting;
    }
    let Some(mut pending) = take(context) else {
        return PaneInputState::None;
    };
    let PaneInputTarget {
        item,
        content,
        widget,
        ..
    } = pending.packet.target;
    let current_content = content == context.plan.content_identity(item);
    let focus_moves = context
        .pane_focus_request
        .is_some_and(|request| request.item() != Some(item));
    if !current_content || (matches!(pending.handoff, Handoff::Continuation) && focus_moves) {
        retire_replaced_input(context);
        return PaneInputState::None;
    }
    if let Handoff::RetryRegistration = pending.handoff {
        return if context.pane_focus_request == pending.previous_request {
            PaneInputState::Ready(PaneInputDelivery {
                packet: pending.packet,
                phase: DeliveryPhase::AwaitingRegistration,
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
            if widget.is_some_and(|widget| {
                (focused == Some(widget) || pending.packet.framework_keys)
                    && pointer_keeps_focus(context.ui.ctx(), widget)
            }) && super::can_route_keyboard(context) =>
        {
            if !pending.packet.framework_keys {
                stage_unclaimed_shortcuts(context);
                pending.packet.events.extend(take_keys_after(context.ui, 0));
            }
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
                    stage_unclaimed_shortcuts(context);
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
            stage_unclaimed_shortcuts(context);
            pending.packet.events.extend(take_keys_after(context.ui, 0));
            queue_incoming(context, pending.packet.target);
            DeliveryPhase::AwaitingRegistration
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
    let retry = matches!(pending.handoff, Handoff::RetryRegistration);
    let plugin = ctx.plugin_or_default::<PaneKeyboard>();
    let mut keyboard = plugin.lock();
    let input = keyboard.viewports.entry(ctx.viewport_id()).or_default();
    input.owner_seen = true;
    input.last_focus = ctx.memory(egui::Memory::focused);
    input.handoff = Some(pending);
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
        let pending = input
            .handoff
            .take_if(|pending| pending.packet.target.slot == slot);
        input.owner_seen |= pending.is_some();
        return pending;
    }
    input
        .later_packets
        .front()
        .filter(|packet| packet.target.slot == slot)?;
    input.owner_seen = true;
    Some(PendingInput {
        ready: true,
        previous_request: None,
        handoff: Handoff::Continuation,
        packet: input.later_packets.pop_front()?,
    })
}

fn registration_retry(context: &RenderContext<'_, '_, '_>) -> bool {
    let ctx = context.ui.ctx();
    ctx.plugin_opt::<PaneKeyboard>().is_some_and(|plugin| {
        plugin
            .lock()
            .viewports
            .get(&ctx.viewport_id())
            .is_some_and(|input| {
                input
                    .handoff
                    .as_ref()
                    .is_some_and(|pending| matches!(pending.handoff, Handoff::RetryRegistration))
            })
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
            queued_at_ingress: true,
            framework_keys: false,
        });
    }
}

fn stage_unclaimed_shortcuts(context: &RenderContext<'_, '_, '_>) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
        && std::mem::take(&mut input.unclaimed_shortcuts)
    {
        input.incoming = take_keys_after(context.ui, 0);
    }
}

fn rebind_packets(
    context: &RenderContext<'_, '_, '_>,
    old: PaneInputTarget,
    widget: Id,
) -> PaneInputTarget {
    let target = PaneInputTarget {
        widget: Some(widget),
        ..old
    };
    let ctx = context.ui.ctx();
    if target != old
        && let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        for packet in &mut input.later_packets {
            if packet.target == old {
                packet.target = target;
            }
        }
    }
    target
}

fn retire_input(context: &RenderContext<'_, '_, '_>) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        input.later_packets.clear();
        // The incoming packet has never been delivered. Return it through the
        // next raw input hook so egui can process its focus and key state.
        input.owner_seen = false;
    }
}

fn retire_replaced_input(context: &RenderContext<'_, '_, '_>) {
    retire_input(context);
    let ctx = context.ui.ctx();
    if context
        .pane_focus_request
        .and_then(|request| request.item())
        .is_some()
        && let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        // The current plan explicitly requests a different focus target.
        // The new handoff may take this unaddressed packet now.
        ctx.input_mut(|state| {
            state
                .events
                .splice(..0, std::mem::take(&mut input.incoming));
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
        let widget = context.panes.focus_target(item);
        let prior_focus = context.ui.memory(egui::Memory::focused);
        if (widget.is_some() && prior_focus == widget)
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
            phase: DeliveryPhase::AwaitingRegistration,
        })
    }

    pub(in crate::render) fn targets(&self, item: ItemId) -> bool {
        self.packet.target.item == item
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
            && self.packet.target.widget.is_some_and(|widget| {
                ui.memory(|memory| memory.has_focus(widget))
                    && crate::focus::pane_registered_focus_target(ui, clip, widget, true)
            })
    }

    pub(in crate::render) fn settle_registration(
        mut self,
        context: &mut RenderContext<'_, '_, '_>,
        target: Option<Id>,
    ) {
        if self.awaits_registration() && context.ui.ctx().will_discard() {
            defer(context, self.packet, Handoff::RetryRegistration);
        } else if self.awaits_registration()
            && let Some(target) = target.filter(|target| {
                self.packet
                    .target
                    .widget
                    .is_none_or(|expected| *target == expected)
            })
            && context.ui.memory(|memory| memory.has_focus(target))
        {
            self.packet.target = rebind_packets(context, self.packet.target, target);
            defer(context, self.packet, Handoff::Continuation);
        } else {
            self.retire(context);
        }
    }

    pub(in crate::render) fn retire(self, context: &mut RenderContext<'_, '_, '_>) {
        if self.awaits_registration() {
            context
                .pane_shortcuts
                .extend(self.packet.events.into_iter().filter(is_key_event));
        }
        retire_input(context);
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
            target: self.packet.target,
            framework_keys: self.packet.framework_keys,
        }
    }
}

pub(in crate::render) struct ScopedPaneInput {
    context: Context,
    active: bool,
    target: PaneInputTarget,
    framework_keys: bool,
}

impl ScopedPaneInput {
    pub(in crate::render) fn finish(
        mut self,
        context: &mut RenderContext<'_, '_, '_>,
        ui: &Ui,
        clip: Rect,
    ) {
        self.active = false;
        // TextEdit reads text and clipboard events without removing them from
        // egui's list. Explicitly consumed keys have already been removed.
        // Return the remaining keys after all panes have painted so that host
        // shortcuts can observe the effect of the editor's earlier input.
        self.context.input_mut(|input| {
            context.pane_shortcuts.extend(
                input
                    .events
                    .extract_if(.., |event| is_keyboard_input(event))
                    .filter(is_key_event),
            );
        });
        if self.framework_keys {
            let focused = ui.memory(egui::Memory::focused).filter(|&widget| {
                crate::focus::pane_registered_focus_target(ui, clip, widget, false)
            });
            settle_framework_keys(context, self.target, focused);
        } else {
            queue_incoming(context, self.target);
        }
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

fn settle_framework_keys(
    context: &RenderContext<'_, '_, '_>,
    target: PaneInputTarget,
    focused: Option<Id>,
) {
    if let Some(widget) = focused {
        queue_incoming(context, rebind_packets(context, target, widget));
    } else {
        // A framework traversal may leave the pane. The following fresh input
        // resumes ordinary egui routing on the next batch.
        retire_input(context);
    }
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
