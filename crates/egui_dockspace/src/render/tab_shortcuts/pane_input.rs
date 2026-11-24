//! Viewport-local keyboard input awaiting an exact pane focus handoff.

use std::{collections::VecDeque, time::Duration};

use dockspace::{
    model::{ItemId, TabKey},
    runtime::{DockspaceOpenItemId, PanePaintRecord},
};
use egui::{
    Context, Event, EventFilter, FocusDirection, Id, Key, RawInput, Rect, Ui, ViewportId,
    ViewportIdMap,
};

use super::super::RenderContext;

mod widget;
use widget::{FrameworkInput, WidgetInput, widget_input_key, with_keyboard_events};
pub use widget::{pane_text_edit, with_pane_keyboard_input, with_pane_keyboard_input_at};

#[derive(Clone, Copy)]
enum Handoff {
    Activation {
        header: Id,
        awaiting_publication: bool,
    },
    Continuation,
    FrameworkTraversal {
        pane: Id,
        settlement: TraversalSettlement,
    },
    RetryRegistration,
    RetryDelivery,
}

#[derive(Clone, Copy)]
enum TraversalSettlement {
    PassEnd(u64),
    NextPassStart,
}

impl Handoff {
    fn is_retry(self) -> bool {
        matches!(self, Self::RetryRegistration | Self::RetryDelivery)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaneInputTarget {
    slot: Id,
    item: ItemId,
    content: DockspaceOpenItemId,
    widget: Option<Id>,
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
    last_viewport: Option<ViewportId>,
}

#[derive(Default)]
struct ViewportKeyboard {
    handoff: Option<PendingInput>,
    later_packets: VecDeque<PaneInputPacket>,
    owner_seen: bool,
    last_focus: Option<Id>,
    host_keys_on_retry: egui::IdMap<Vec<Event>>,
}

impl ViewportKeyboard {
    fn settle_traversal(
        &mut self,
        context: &Context,
        viewport: ViewportId,
        pane: Id,
        focused: Option<Id>,
    ) {
        let Some(widget) = focused.filter(|&widget| {
            context.viewport_for(viewport, |viewport| {
                crate::focus::widget_belongs_to_pane(&viewport.prev_pass.widgets, widget, pane)
            })
        }) else {
            self.retire_input();
            return;
        };
        if let Some(pending) = self.handoff.as_mut() {
            let old = pending.packet.target;
            let target = PaneInputTarget {
                widget: Some(widget),
                ..old
            };
            pending.handoff = Handoff::Continuation;
            pending.packet.target = target;
            self.rebind_packets(old, target);
        }
    }

    fn rebind_packets(&mut self, old: PaneInputTarget, target: PaneInputTarget) {
        if old != target {
            for packet in &mut self.later_packets {
                if packet.target == old {
                    packet.target = target;
                }
            }
        }
    }

    fn retire_input(&mut self) {
        self.later_packets.clear();
        self.handoff = None;
    }
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
            self.viewports.remove(&viewport);
            return;
        }
        input.host_keys_on_retry.clear();
        if previous_viewport == Some(viewport) {
            input.last_focus = context.memory(egui::Memory::focused);
        }
        if let Some(pending) = &mut input.handoff {
            if pending.ready || pending.handoff.is_retry() {
                input.handoff = None;
            } else {
                pending.ready = true;
            }
        }
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let context = ui.ctx();
        let viewport = context.viewport_id();
        let Some(input) = self.viewports.get_mut(&viewport) else {
            return;
        };
        let Some(PendingInput {
            handoff:
                Handoff::FrameworkTraversal {
                    pane,
                    settlement: TraversalSettlement::NextPassStart,
                },
            packet,
            ..
        }) = &input.handoff
        else {
            return;
        };
        // egui applies reverse Tab before the UI can make another focus request.
        let focused = if has_pointer_or_accessibility_edges(context) {
            None
        } else {
            ui.memory(|memory| {
                packet
                    .target
                    .widget
                    .filter(|&widget| memory.had_focus_last_frame(widget))?;
                memory.focused()
            })
        };
        input.settle_traversal(context, viewport, *pane, focused);
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if let Some(input) = self.viewports.get_mut(&ui.ctx().viewport_id()) {
            input.last_focus = ui.memory(egui::Memory::focused);
            if input.handoff.as_ref().is_some_and(|pending| {
                matches!(
                    pending.handoff,
                    Handoff::FrameworkTraversal {
                        settlement: TraversalSettlement::PassEnd(_),
                        ..
                    }
                ) && pending.packet.target.widget != input.last_focus
            }) {
                input.retire_input();
            }
        }
    }

    fn output_hook(&mut self, context: &Context, output: &mut egui::FullOutput) {
        for (&viewport, input) in &mut self.viewports {
            if let Some(Handoff::FrameworkTraversal {
                pane,
                settlement: TraversalSettlement::PassEnd(pass),
            }) = input.handoff.as_ref().map(|pending| pending.handoff)
                && context.cumulative_pass_nr_for(viewport) == pass + 1
            {
                // Spatial navigation settles in egui's end_pass, before the
                // host can refocus a widget between runs.
                let focused = terminal_viewport_focus(context, viewport, output);
                input.settle_traversal(context, viewport, pane, focused);
            }
        }
        self.viewports.retain(|viewport, input| {
            (input.handoff.is_some()
                || !input.later_packets.is_empty()
                || !input.host_keys_on_retry.is_empty())
                && (*viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(*viewport) != 0)
        });
        // egui has already popped the viewport which just ended. Wake the
        // viewports that actually hold input, including native children.
        for (&viewport, input) in &self.viewports {
            if input.owner_seen && (input.handoff.is_some() || !input.later_packets.is_empty()) {
                context.request_repaint_of(viewport);
                if let Some(output) = output.viewport_output.get_mut(&viewport) {
                    output.repaint_delay = Duration::ZERO;
                }
            }
        }
    }
}

fn terminal_viewport_focus(
    context: &Context,
    viewport: ViewportId,
    output: &egui::FullOutput,
) -> Option<Id> {
    if viewport == ViewportId::ROOT
        || crate::pass_settlement::owns_viewport_run(context, viewport)
        || output
            .viewport_output
            .get(&viewport)
            .is_some_and(|viewport| {
                viewport.class == egui::ViewportClass::Deferred && viewport.viewport_ui_cb.is_some()
            })
    {
        return context.memory(egui::Memory::focused);
    }
    // A nested viewport's output hook can run with the parent's focus memory.
    let focus = output.platform_output.accesskit_update.as_ref()?.focus;
    context.viewport_for(viewport, |viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets)
            .find(|widget| widget.id.accesskit_id() == focus)
            .map(|widget| widget.id)
    })
}

pub(in crate::render) struct PaneInputDelivery {
    packet: PaneInputPacket,
    phase: DeliveryPhase,
    focus_cleared_by_escape: bool,
}

#[derive(Clone, Copy)]
enum DeliveryPhase {
    AwaitingRegistration,
    Registered { retry: bool },
}

pub(super) fn activate(
    context: &mut RenderContext<'_, '_, '_>,
    start: usize,
    key: Key,
    tab: TabKey,
    header: Id,
) {
    let Some(item) = context.plan.tab_content_focus_target(tab) else {
        return;
    };
    let widget = context.panes.focus_target(item);
    if !context.panes.accepts_deferred_keyboard_input(item) {
        // This pane leaves keyboard delivery with egui. Keep host commands
        // available after rendering, when the old editor can no longer see them.
        context.pane_shortcuts.extend(
            take_keys_after(context.ui, start)
                .into_iter()
                .filter(is_host_key),
        );
        return;
    }
    let Some(target) = PaneInputTarget::capture(context, item, widget) else {
        return;
    };
    let mut events = take_keys_after(context.ui, start);
    if key == Key::Space && matches!(events.first(), Some(Event::Text(text)) if text == " ") {
        // egui-winit emits a text event immediately after the activating Space.
        events.remove(0);
    }
    defer(
        context,
        PaneInputPacket { target, events },
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
    if !context.panes.accepts_deferred_keyboard_input(item) {
        return;
    }
    let Some(target) = PaneInputTarget::capture(context, item, Some(widget)) else {
        return;
    };
    defer(
        context,
        PaneInputPacket { target, events },
        Handoff::Continuation,
    );
}

impl PaneInputTarget {
    fn capture(
        context: &RenderContext<'_, '_, '_>,
        item: ItemId,
        widget: Option<Id>,
    ) -> Option<Self> {
        Some(Self {
            slot: slot_id(context),
            item,
            content: context.plan.open_item_identity(item)?,
            widget,
        })
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
    if context.ui.ctx().current_pass_index() != 0 && !layout_retry(context) {
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
    let current_content = Some(content) == context.plan.open_item_identity(item);
    let focus_moves = context
        .pane_focus_request
        .is_some_and(|request| request.item() != Some(item));
    if !current_content
        || !context.panes.accepts_deferred_keyboard_input(item)
        || (matches!(
            pending.handoff,
            Handoff::Continuation | Handoff::FrameworkTraversal { .. }
        ) && focus_moves)
    {
        retire_input(context);
        return PaneInputState::None;
    }
    if let Handoff::RetryRegistration = pending.handoff {
        return if context.pane_focus_request == pending.previous_request {
            PaneInputState::Ready(PaneInputDelivery {
                packet: pending.packet,
                phase: DeliveryPhase::AwaitingRegistration,
                focus_cleared_by_escape: false,
            })
        } else {
            retire_input(context);
            PaneInputState::None
        };
    }
    if let Handoff::RetryDelivery = pending.handoff {
        return if context.pane_focus_request == pending.previous_request
            && super::can_route_keyboard(context)
        {
            PaneInputState::Ready(PaneInputDelivery {
                packet: pending.packet,
                phase: DeliveryPhase::Registered { retry: true },
                focus_cleared_by_escape: false,
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
    let focus_cleared_by_escape = focused.is_none()
        && widget.is_some_and(|widget| focus_before_live_escape(context.ui.ctx(), widget));
    let phase = match pending.handoff {
        Handoff::Continuation
            if widget.is_some_and(|widget| {
                (focused == Some(widget) || focus_cleared_by_escape)
                    && pointer_keeps_focus(context.ui.ctx(), widget)
            }) && super::can_route_keyboard(context) =>
        {
            stage_live_input(context, &mut pending.packet);
            DeliveryPhase::Registered { retry: false }
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
                    if !has_pointer_or_accessibility_edges(context.ui.ctx()) {
                        stage_live_input(context, &mut pending.packet);
                    }
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
            if !has_pointer_or_accessibility_edges(context.ui.ctx()) {
                stage_live_input(context, &mut pending.packet);
            }
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
        focus_cleared_by_escape,
    })
}

fn pointer_keeps_focus(context: &Context, widget: Id) -> bool {
    if context.input(|input| {
        input
            .raw
            .events
            .iter()
            .any(|event| matches!(event, Event::AccessKitActionRequest(_)))
    }) {
        return false;
    }
    if !context.input(|input| input.pointer.any_down())
        && !has_pointer_or_accessibility_edges(context)
    {
        return true;
    }
    // egui resolves clicks from the previous pass. A click on the same editor
    // keeps its queued text, even with the default click-to-surrender policy.
    let clicked = context.interaction_snapshot(|snapshot| snapshot.clicked == Some(widget));
    let hit = context.viewport(|viewport| viewport.hits.click.is_some_and(|hit| hit.id == widget));
    if !clicked && !hit {
        return false;
    }
    let Some(response) = context.read_response(widget) else {
        return false;
    };
    let from_global = context.layer_transform_from_global(response.layer_id);
    context.input(|input| {
        input.raw.events.iter().all(|event| match event {
            Event::PointerButton { pos, button, .. } => {
                *button == egui::PointerButton::Primary
                    && response
                        .interact_rect
                        .contains(from_global.map_or(*pos, |transform| transform * *pos))
            }
            _ => true,
        })
    })
}

fn has_pointer_or_accessibility_edges(context: &Context) -> bool {
    context.input(|input| {
        input.raw.events.iter().any(|event| {
            matches!(
                event,
                Event::PointerButton { .. } | Event::AccessKitActionRequest(_)
            )
        })
    })
}

fn focus_before_live_escape(context: &Context, widget: Id) -> bool {
    !has_pointer_or_accessibility_edges(context)
        && context.input(|input| input.events.iter().any(is_plain_escape))
        && context.plugin_opt::<PaneKeyboard>().is_some_and(|plugin| {
            plugin
                .lock()
                .viewports
                .get(&context.viewport_id())
                .is_some_and(|input| input.last_focus == Some(widget))
        })
}

fn is_plain_escape(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Escape, pressed: true, modifiers, .. }
        if modifiers.is_none())
}

fn framework_traversal(event: &Event) -> Option<FocusDirection> {
    match event {
        Event::Key {
            key: Key::Tab,
            pressed: true,
            modifiers,
            ..
        } if modifiers.is_none() => Some(FocusDirection::Next),
        Event::Key {
            key: Key::Tab,
            pressed: true,
            modifiers,
            ..
        } if modifiers.shift_only() => Some(FocusDirection::Previous),
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if modifiers.is_none() => match key {
            Key::ArrowLeft => Some(FocusDirection::Left),
            Key::ArrowRight => Some(FocusDirection::Right),
            Key::ArrowUp => Some(FocusDirection::Up),
            Key::ArrowDown => Some(FocusDirection::Down),
            _ => None,
        },
        _ => None,
    }
}

fn store(context: &RenderContext<'_, '_, '_>, pending: PendingInput) {
    let ctx = context.ui.ctx();
    let retry = pending.handoff.is_retry();
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

fn layout_retry(context: &RenderContext<'_, '_, '_>) -> bool {
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
                    .is_some_and(|pending| pending.handoff.is_retry())
            })
    })
}

fn can_append_live(context: &RenderContext<'_, '_, '_>, packet: &PaneInputPacket) -> bool {
    let ctx = context.ui.ctx();
    !ctx.plugin_opt::<PaneKeyboard>().is_some_and(|plugin| {
        plugin
            .lock()
            .viewports
            .get(&ctx.viewport_id())
            .is_some_and(|input| !input.later_packets.is_empty())
    }) && (packet.events.is_empty()
        || (!packet.events.iter().any(is_key_event)
            && packet
                .events
                .iter()
                .any(|event| matches!(event, Event::Ime(egui::ImeEvent::Preedit { .. })))
            && ctx.input(|input| {
                // Native IMEs can clear the marked text immediately before
                // committing its replacement. Keep that pair with the preedit.
                let first = input.events.iter().filter(|event| is_keyboard_input(event))
                    .find(|event| !matches!(event, Event::Ime(egui::ImeEvent::Preedit { text, .. }) if text.is_empty()));
                matches!(first, Some(Event::Ime(egui::ImeEvent::Commit(_))))
            })))
}

fn stage_live_input(context: &RenderContext<'_, '_, '_>, packet: &mut PaneInputPacket) {
    let ctx = context.ui.ctx();
    let append = can_append_live(context, packet);
    let mut events = take_keys_after(context.ui, 0);
    if events.is_empty() {
        return;
    }
    // egui may already have selected a focus direction. Hold that decision
    // until the exact editor can apply its filter and process its packet.
    let navigation_held = events
        .iter()
        .any(|event| !EventFilter::default().matches(event));
    if navigation_held {
        ctx.memory_mut(|memory| memory.move_focus(FocusDirection::None));
    }
    if append {
        packet.events.append(&mut events);
    } else if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        input.later_packets.push_back(PaneInputPacket {
            target: packet.target,
            events,
        });
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
        input.rebind_packets(old, target);
    }
    target
}

fn retire_input(context: &RenderContext<'_, '_, '_>) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        input.retire_input();
    }
}

impl PaneInputDelivery {
    pub(super) fn for_focus_request(context: &RenderContext<'_, '_, '_>) -> Option<Self> {
        if !context.pointer_authority.supports_preview_gestures()
            || !super::can_route_keyboard(context)
            || !context
                .ui
                .input(|input| input.events.iter().any(is_keyboard_input))
        {
            return None;
        }
        let request = context.pane_focus_request?;
        let item = request.item()?;
        if !context.panes.accepts_deferred_keyboard_input(item) {
            return None;
        }
        let widget = context.panes.focus_target(item);
        let target = PaneInputTarget::capture(context, item, widget)?;
        if has_pointer_or_accessibility_edges(context.ui.ctx())
            && !widget.is_some_and(|widget| pointer_keeps_focus(context.ui.ctx(), widget))
        {
            return None;
        }
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
            packet: PaneInputPacket {
                target,
                events: take_keys_after(context.ui, 0),
            },
            phase: DeliveryPhase::AwaitingRegistration,
            focus_cleared_by_escape: false,
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
        let DeliveryPhase::Registered { retry } = self.phase else {
            return false;
        };
        self.packet.target.widget.is_some_and(|widget| {
            ui.memory(|memory| {
                memory.has_focus(widget)
                    || self.focus_cleared_by_escape && memory.focused().is_none()
            }) && (retry || crate::focus::pane_registered_focus_target(ui, clip, widget, true))
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
        retire_input(context);
    }

    pub(in crate::render) fn replay(
        self,
        context: &RenderContext<'_, '_, '_>,
        pane: PanePaintRecord<'_>,
    ) -> ScopedPaneInput {
        let ctx = context.ui.ctx();
        let (keyboard, events) = with_keyboard_events(ctx, self.packet.events, || {
            super::TabKeyboard::for_replayed_pane(context, pane)
        });
        let key = widget_input_key(ctx);
        let previous = ctx.data_mut(|data| {
            let previous = data.remove_temp(key);
            data.insert_temp(
                key,
                WidgetInput {
                    widget: self.packet.target.widget,
                    events: Some(events),
                    restore_focus: self.focus_cleared_by_escape,
                    ..WidgetInput::default()
                },
            );
            previous
        });
        ScopedPaneInput {
            context: ctx.clone(),
            key: Some(key),
            previous,
            target: self.packet.target,
            keyboard,
        }
    }
}

pub(in crate::render) struct ScopedPaneInput {
    context: Context,
    key: Option<Id>,
    previous: Option<WidgetInput>,
    target: PaneInputTarget,
    keyboard: super::TabKeyboard,
}

impl ScopedPaneInput {
    pub(in crate::render) fn finish(
        mut self,
        context: &mut RenderContext<'_, '_, '_>,
        ui: &Ui,
        clip: Rect,
    ) {
        let input = self.restore_scope();
        let Some(events) = input.unconsumed else {
            if self.context.will_discard() {
                if let Some(mut events) = input.events {
                    if let super::TabKeyboard::PaneShortcut(cycle) =
                        std::mem::replace(&mut self.keyboard, super::TabKeyboard::Done)
                    {
                        events.extend(cycle.tail);
                    }
                    defer(
                        context,
                        PaneInputPacket {
                            target: self.target,
                            events,
                        },
                        Handoff::RetryDelivery,
                    );
                }
            } else {
                retire_input(context);
            }
            return;
        };
        let keyboard = std::mem::replace(&mut self.keyboard, super::TabKeyboard::Done);
        let (_, unconsumed) =
            with_keyboard_events(&self.context, events, || keyboard.capture(context));
        // These keys can be observed by host callbacks only after the exact
        // widget processed the packet.
        context
            .pane_shortcuts
            .extend(unconsumed.into_iter().filter(is_key_event));
        match input.framework_input {
            Some(FrameworkInput::Surrender(tail)) => {
                context
                    .pane_shortcuts
                    .extend(tail.into_iter().filter(is_key_event));
                retire_input(context);
            }
            Some(FrameworkInput::Traverse(direction, tail)) => {
                let focused = ui.memory(egui::Memory::focused).filter(|&widget| {
                    crate::focus::pane_registered_focus_target(ui, clip, widget, false)
                });
                if direction == FocusDirection::Next {
                    settle_framework_keys(context, self.target, focused, tail);
                } else {
                    defer_framework_traversal(
                        context,
                        self.target,
                        ui.unique_id(),
                        direction,
                        tail,
                    );
                }
            }
            None => {}
        }
    }

    fn restore_scope(&mut self) -> WidgetInput {
        let Some(key) = self.key.take() else {
            return WidgetInput::default();
        };
        self.context.data_mut(|data| {
            let input = data.remove_temp(key).unwrap_or_default();
            if let Some(previous) = self.previous.take() {
                data.insert_temp(key, previous);
            }
            input
        })
    }
}

impl Drop for ScopedPaneInput {
    fn drop(&mut self) {
        self.restore_scope();
    }
}

fn is_key_event(event: &Event) -> bool {
    matches!(event, Event::Key { .. })
}

fn is_host_key(event: &Event) -> bool {
    matches!(event, Event::Key { pressed: false, .. })
        || matches!(event, Event::Key { modifiers, .. }
            if modifiers.command || modifiers.ctrl || modifiers.alt)
}

fn settle_framework_keys(
    context: &RenderContext<'_, '_, '_>,
    target: PaneInputTarget,
    focused: Option<Id>,
    tail: Vec<Event>,
) {
    if let Some(widget) = focused {
        let target = rebind_packets(context, target, widget);
        let ctx = context.ui.ctx();
        if !tail.is_empty()
            && let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
            && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
        {
            if !tail
                .iter()
                .any(|event| matches!(event, Event::Key { pressed: true, .. }))
                && let Some(next) = input
                    .later_packets
                    .front_mut()
                    .filter(|next| next.target == target)
            {
                next.events.splice(..0, tail);
            } else {
                input.later_packets.push_front(PaneInputPacket {
                    target,
                    events: tail,
                });
            }
        }
    } else {
        // A framework traversal may leave the pane. The following fresh input
        // resumes ordinary egui routing on the next batch.
        retire_input(context);
    }
}

fn defer_framework_traversal(
    context: &RenderContext<'_, '_, '_>,
    target: PaneInputTarget,
    pane: Id,
    direction: FocusDirection,
    tail: Vec<Event>,
) {
    let ctx = context.ui.ctx();
    let packet = if tail.is_empty() {
        ctx.plugin_opt::<PaneKeyboard>().and_then(|plugin| {
            plugin
                .lock()
                .viewports
                .get_mut(&ctx.viewport_id())?
                .later_packets
                .pop_front()
        })
    } else {
        Some(PaneInputPacket {
            target,
            events: tail,
        })
    };
    if let Some(packet) = packet {
        let settlement = if direction == FocusDirection::Previous {
            TraversalSettlement::NextPassStart
        } else {
            TraversalSettlement::PassEnd(ctx.cumulative_pass_nr())
        };
        defer(
            context,
            packet,
            Handoff::FrameworkTraversal { pane, settlement },
        );
    }
}

fn slot_id(context: &RenderContext<'_, '_, '_>) -> Id {
    keyboard_slot(context.instance_id, context.plan.surface())
}

fn keyboard_slot(instance: Id, surface: dockspace::model::SurfaceId) -> Id {
    instance.with(("pane-keyboard-handoff", surface))
}

pub(crate) fn release_host_keys(
    ui: &Ui,
    instance: Id,
    surface: dockspace::model::SurfaceId,
    mut keys: Vec<Event>,
) {
    let ctx = ui.ctx();
    let slot = keyboard_slot(instance, surface);
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
        && let Some(mut previous) = input.host_keys_on_retry.remove(&slot)
    {
        previous.append(&mut keys);
        keys = previous;
    }
    if keys.is_empty() {
        return;
    }
    if ctx.will_discard() {
        ctx.plugin_or_default::<PaneKeyboard>()
            .lock()
            .viewports
            .entry(ctx.viewport_id())
            .or_default()
            .host_keys_on_retry
            .insert(slot, keys);
    } else {
        ui.input_mut(|input| drop(input.events.splice(..0, keys)));
    }
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
