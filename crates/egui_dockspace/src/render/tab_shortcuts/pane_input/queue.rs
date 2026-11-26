//! Per-viewport input ownership, retained passes and traversal settlement.

use super::*;

#[derive(Default)]
struct PaneKeyboard {
    viewports: ViewportIdMap<ViewportKeyboard>,
    last_viewport: Option<ViewportId>,
}

#[derive(Default)]
struct ViewportKeyboard {
    handoff: Option<PendingInput>,
    later_packets: VecDeque<PaneInputPacket>,
    pass: Option<KeyboardPass>,
    retained: bool,
    last_focus: Option<Id>,
    host_keys_on_retry: egui::IdMap<Vec<Event>>,
    host_keys_after_traversal: Option<(Id, Vec<Event>)>,
}

struct KeyboardPass {
    number: u64,
    surface_seen: bool,
    retained: bool,
}

impl ViewportKeyboard {
    fn begin_pass(&mut self, number: u64) {
        self.pass = Some(KeyboardPass {
            number,
            surface_seen: false,
            retained: false,
        });
    }

    fn owner_packet(&self) -> Option<&PaneInputPacket> {
        self.handoff
            .as_ref()
            .map(|pending| &pending.packet)
            .or_else(|| self.later_packets.front())
    }

    fn observe_surface(&mut self, context: &Context) {
        let number = context.cumulative_pass_nr();
        if self.pass.as_ref().is_none_or(|pass| pass.number != number) {
            self.begin_pass(number);
        }
        if let Some(pass) = &mut self.pass {
            pass.surface_seen = true;
        }
    }

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
            self.retire_to_host();
            return;
        };
        self.last_focus = Some(widget);
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

    fn finish_forward_traversal(
        &mut self,
        context: &Context,
        viewport: ViewportId,
        pane: Id,
        focused: Option<Id>,
    ) {
        if focused.is_some_and(|widget| {
            context.viewport_for(viewport, |viewport| {
                crate::focus::widget_belongs_to_pane(&viewport.prev_pass.widgets, widget, pane)
            })
        }) {
            self.settle_traversal(context, viewport, pane, focused);
            return;
        }
        self.retire_to_host();
    }

    fn await_forward_wrap(&mut self, pane: Id, completed: u64) {
        if let Some(pending) = &mut self.handoff {
            pending.handoff = Handoff::FrameworkTraversal {
                pane,
                settlement: TraversalSettlement::Forward {
                    pass: completed,
                    wrap: true,
                },
            };
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
        self.retained = false;
    }

    fn retire_to_host(&mut self) {
        let Some(slot) = self.owner_packet().map(|packet| packet.target.slot) else {
            return;
        };
        let keys = self.take_host_keys();
        if !keys.is_empty() {
            self.host_keys_after_traversal = Some((slot, keys));
        }
    }

    fn take_host_keys(&mut self) -> Vec<Event> {
        self.retained = false;
        self.handoff
            .take()
            .map(|pending| pending.packet)
            .into_iter()
            .chain(self.later_packets.drain(..))
            .flat_map(|packet| packet.events)
            .filter(is_host_key)
            .collect()
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
        input.begin_pass(context.cumulative_pass_nr_for(viewport));
        if context.viewport_for(viewport, |state| state.output.num_completed_passes != 0) {
            return;
        }
        input.host_keys_on_retry.clear();
        if previous_viewport == Some(viewport) {
            input.last_focus = context.memory(egui::Memory::focused);
        }
        if let Some(pending) = &mut input.handoff {
            pending.ready = true;
        }
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let context = ui.ctx();
        let viewport = context.viewport_id();
        let Some(input) = self.viewports.get_mut(&viewport) else {
            return;
        };
        let Some(PendingInput {
            handoff: Handoff::FrameworkTraversal { pane, settlement },
            packet,
            ..
        }) = &input.handoff
        else {
            return;
        };
        if has_pointer_or_accessibility_edges(context) {
            input.retire_to_host();
            return;
        }
        let current = context.cumulative_pass_nr();
        let pane = *pane;
        match *settlement {
            TraversalSettlement::NextPassStart => {
                // egui applies reverse Tab at the start of the next pass.
                let focused = ui.memory(|memory| {
                    packet
                        .target
                        .widget
                        .filter(|&widget| memory.had_focus_last_frame(widget))?;
                    memory.focused()
                });
                input.settle_traversal(context, viewport, pane, focused);
            }
            TraversalSettlement::PassEnd(pass) if current == pass + 1 => {
                // An immediate child without AccessKit can read its terminal
                // focus again only after egui re-enters that viewport.
                let focused = focus_from_previous_pass(ui);
                input.last_focus = focused;
                input.settle_traversal(context, viewport, pane, focused);
            }
            TraversalSettlement::Forward { pass, wrap }
                if current > pass && context.current_pass_index() == 0 && !input.retained =>
            {
                let focused = focus_from_previous_pass(ui);
                if focused.is_none()
                    && !wrap
                    && ui.memory(|memory| memory.focused().is_none())
                    && !ui.input(|input| input.events.iter().any(is_plain_escape))
                {
                    input.await_forward_wrap(pane, current);
                } else if focused.is_some() {
                    input.finish_forward_traversal(context, viewport, pane, focused);
                }
            }
            _ => {}
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if let Some(input) = self.viewports.get_mut(&ui.ctx().viewport_id()) {
            input.last_focus = ui.memory(egui::Memory::focused);
            if input.pass.as_ref().is_none_or(|pass| !pass.retained)
                && input.handoff.as_ref().is_some_and(|pending| {
                    matches!(
                        pending.handoff,
                        Handoff::FrameworkTraversal {
                            settlement: TraversalSettlement::PassEnd(_),
                            ..
                        }
                    ) && pending.packet.target.widget != input.last_focus
                })
            {
                input.retire_to_host();
            }
        }
    }

    fn output_hook(&mut self, context: &Context, output: &mut egui::FullOutput) {
        let discarded = output.platform_output.requested_discard()
            && output.platform_output.num_completed_passes
                < context.options(|options| options.max_passes.get());
        for (&viewport, input) in &mut self.viewports {
            if !input
                .pass
                .as_ref()
                .is_some_and(|pass| pass.number + 1 == context.cumulative_pass_nr_for(viewport))
            {
                continue;
            }
            let pass = input
                .pass
                .take()
                .expect("the current keyboard pass was checked");
            if !pass.surface_seen {
                if !discarded {
                    input.retire_input();
                    input.host_keys_on_retry.clear();
                    input.host_keys_after_traversal = None;
                }
                continue;
            }
            if pass.retained {
                continue;
            }
            if let Some(Handoff::FrameworkTraversal { pane, settlement }) =
                input.handoff.as_ref().map(|pending| pending.handoff)
            {
                let completed = context.cumulative_pass_nr_for(viewport);
                // egui resolves traversal after on_end_pass. Use this exact
                // completed pass before host code can refocus between runs.
                let Some(focused) =
                    crate::pass_settlement::terminal_viewport_focus(context, viewport, output)
                else {
                    continue;
                };
                match settlement {
                    TraversalSettlement::PassEnd(pass) if completed == pass + 1 => {
                        input.settle_traversal(context, viewport, pane, focused);
                    }
                    TraversalSettlement::Forward { pass, wrap }
                        if completed > pass && !discarded =>
                    {
                        if focused.is_none() && !wrap {
                            // egui gives focus to the first interested widget
                            // of the next pass when Tab reaches the end.
                            input.await_forward_wrap(pane, completed);
                        } else {
                            input.finish_forward_traversal(context, viewport, pane, focused);
                        }
                    }
                    _ => {}
                }
            }
        }
        self.viewports.retain(|viewport, input| {
            (input.handoff.is_some()
                || !input.later_packets.is_empty()
                || !input.host_keys_on_retry.is_empty()
                || input.host_keys_after_traversal.is_some())
                && (*viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(*viewport) != 0)
        });
        // egui has already popped the viewport which just ended. Wake the
        // viewports that actually hold input, including native children.
        for (&viewport, input) in &self.viewports {
            if input.handoff.is_some()
                || !input.later_packets.is_empty()
                || input.host_keys_after_traversal.is_some()
            {
                context.request_repaint_of(viewport);
                if let Some(output) = output.viewport_output.get_mut(&viewport) {
                    output.repaint_delay = Duration::ZERO;
                }
            }
        }
    }
}

fn focus_from_previous_pass(ui: &Ui) -> Option<Id> {
    ui.memory(|memory| {
        memory
            .focused()
            .filter(|&widget| memory.had_focus_last_frame(widget))
    })
}

/// Keeps the exact surface's queued input and focus alive through a retained pass.
/// Returns whether the existing focus was kept alive by the queue.
pub(crate) fn retain_skipped_pane_keyboard(
    context: &Context,
    instance: Id,
    surface: dockspace::model::SurfaceId,
) -> bool {
    let slot = keyboard_slot(instance, surface);
    let Some(plugin) = context.plugin_opt::<PaneKeyboard>() else {
        return false;
    };
    let mut keyboard = plugin.lock();
    let Some(input) = keyboard.viewports.get_mut(&context.viewport_id()) else {
        return false;
    };
    let Some(packet) = input
        .owner_packet()
        .filter(|packet| packet.target.slot == slot)
    else {
        if input
            .host_keys_after_traversal
            .as_ref()
            .is_some_and(|(owner, _)| *owner == slot)
        {
            input.observe_surface(context);
        }
        return false;
    };
    let target = packet.target;
    let expected_focus = match input.handoff.as_ref().map(|pending| pending.handoff) {
        Some(Handoff::Activation { header, .. }) => Some(header),
        _ => target.widget,
    };
    let focus = context.memory(egui::Memory::focused);
    let no_pointer_edge = !has_pointer_or_accessibility_edges(context);
    let escaped = focus.is_none()
        && expected_focus.is_some()
        && input.last_focus == expected_focus
        && no_pointer_edge
        && context.input(|input| input.events.iter().any(is_plain_escape));
    let owned_focus = expected_focus.filter(|&id| focus == Some(id) || escaped);
    if let Some(id) = owned_focus {
        if escaped {
            context.memory_mut(|memory| memory.request_focus(id));
        }
        context.check_for_id_clash(id, Rect::ZERO, "retained dockspace keyboard focus");
    }
    input.observe_surface(context);
    let pass = input
        .pass
        .as_mut()
        .expect("observing the surface starts a keyboard pass");
    let first_retention = !std::mem::replace(&mut pass.retained, true);
    input.retained = true;
    if first_retention
        && no_pointer_edge
        && context.current_pass_index() == 0
        && (owned_focus.is_some()
            || input.handoff.as_ref().is_some_and(|pending| {
                matches!(pending.handoff, Handoff::FrameworkTraversal { .. })
            }))
    {
        let events = take_live_keyboard(context);
        if !events.is_empty() {
            input
                .later_packets
                .push_back(PaneInputPacket { target, events });
        }
    }
    owned_focus.is_some()
}

pub(super) fn observe_surface_keyboard(context: &Context, slot: Id) {
    let Some(plugin) = context.plugin_opt::<PaneKeyboard>() else {
        return;
    };
    let mut keyboard = plugin.lock();
    let Some(input) = keyboard.viewports.get_mut(&context.viewport_id()) else {
        return;
    };
    if input
        .owner_packet()
        .is_some_and(|packet| packet.target.slot == slot)
        || input
            .host_keys_after_traversal
            .as_ref()
            .is_some_and(|(owner, _)| *owner == slot)
    {
        input.observe_surface(context);
    }
}

pub(super) fn foreign_pending_input(context: &RenderContext<'_, '_, '_>) -> bool {
    let ctx = context.ui.ctx();
    let slot = slot_id(context);
    ctx.plugin_opt::<PaneKeyboard>().is_some_and(|plugin| {
        plugin
            .lock()
            .viewports
            .get(&ctx.viewport_id())
            .is_some_and(|input| {
                input
                    .owner_packet()
                    .is_some_and(|packet| packet.target.slot != slot)
            })
    })
}

pub(super) fn focus_before_live_escape(context: &Context, widget: Id) -> bool {
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

pub(super) fn store(context: &RenderContext<'_, '_, '_>, pending: PendingInput) {
    let ctx = context.ui.ctx();
    let retry = pending.handoff.is_retry();
    let plugin = ctx.plugin_or_default::<PaneKeyboard>();
    let mut keyboard = plugin.lock();
    let input = keyboard.viewports.entry(ctx.viewport_id()).or_default();
    input.observe_surface(ctx);
    input.last_focus = ctx.memory(egui::Memory::focused);
    input.handoff = Some(pending);
    if !retry {
        ctx.request_repaint();
    }
}

pub(super) fn take(context: &RenderContext<'_, '_, '_>) -> Option<(PendingInput, bool)> {
    let ctx = context.ui.ctx();
    let plugin = ctx.plugin_opt::<PaneKeyboard>()?;
    let mut keyboard = plugin.lock();
    let input = keyboard.viewports.get_mut(&ctx.viewport_id())?;
    let slot = slot_id(context);
    let pending = if input.handoff.is_some() {
        input
            .handoff
            .take_if(|pending| pending.packet.target.slot == slot)?
    } else {
        input
            .later_packets
            .front()
            .filter(|packet| packet.target.slot == slot)?;
        PendingInput {
            ready: true,
            previous_request: None,
            handoff: Handoff::Continuation,
            packet: input.later_packets.pop_front()?,
        }
    };
    input.observe_surface(ctx);
    Some((pending, std::mem::take(&mut input.retained)))
}

pub(super) fn layout_retry(context: &RenderContext<'_, '_, '_>) -> bool {
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

pub(super) fn stage_live_input(context: &RenderContext<'_, '_, '_>, packet: &mut PaneInputPacket) {
    let ctx = context.ui.ctx();
    let append = can_append_live(context, packet);
    let mut events = take_live_keyboard(ctx);
    if events.is_empty() {
        return;
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

fn take_live_keyboard(context: &Context) -> Vec<Event> {
    let events = take_keyboard_suffix(context, 0);
    // egui may already have selected a focus direction. Hold that decision
    // until the exact editor can apply its filter and process its packet.
    if events
        .iter()
        .any(|event| !EventFilter::default().matches(event))
    {
        context.memory_mut(|memory| memory.move_focus(FocusDirection::None));
    }
    events
}

pub(super) fn rebind_packets(
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

pub(super) fn retire_input(context: &RenderContext<'_, '_, '_>) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
    {
        input.retire_input();
    }
}

pub(super) fn return_queued_host_keys(context: &mut RenderContext<'_, '_, '_>) {
    let ctx = context.ui.ctx();
    if let Some(plugin) = ctx.plugin_opt::<PaneKeyboard>()
        && let Some(input) = plugin.lock().viewports.get_mut(&ctx.viewport_id())
        && input
            .owner_packet()
            .is_none_or(|packet| packet.target.slot == slot_id(context))
    {
        context.pane_shortcuts.extend(input.take_host_keys());
    }
}

pub(super) fn settle_framework_keys(
    context: &RenderContext<'_, '_, '_>,
    target: PaneInputTarget,
    widget: Id,
    tail: Vec<Event>,
) {
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
}

pub(super) fn defer_framework_traversal(
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
        let settlement = match direction {
            FocusDirection::Previous => TraversalSettlement::NextPassStart,
            FocusDirection::Next => TraversalSettlement::Forward {
                pass: ctx.cumulative_pass_nr(),
                wrap: false,
            },
            _ => TraversalSettlement::PassEnd(ctx.cumulative_pass_nr()),
        };
        defer(
            context,
            packet,
            Handoff::FrameworkTraversal { pane, settlement },
        );
    }
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
    {
        if let Some(mut previous) = input.host_keys_on_retry.remove(&slot) {
            previous.append(&mut keys);
            keys = previous;
        }
        if let Some((_, mut previous)) = input
            .host_keys_after_traversal
            .take_if(|(owner, _)| *owner == slot)
        {
            previous.append(&mut keys);
            keys = previous;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use dockspace::{
        model::{
            DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, RootId,
            SurfaceId,
        },
        runtime::DockspaceSession,
    };

    #[test]
    fn forward_wrap_waits_through_retained_passes_for_current_widgets() {
        let context = Context::default();
        let instance = Id::unique("forward wrap");
        let surface = SurfaceId::new(1);
        let item = ItemId::new(1);
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            surface,
            DockspaceRootLayout::new(RootId::new(1), DockspaceNode::tabs([item])),
        )])
        .unwrap();
        let session = DockspaceSession::from_layout(layout, Default::default()).unwrap();
        let widget = Id::unique("last editor");
        let slot = keyboard_slot(instance, surface);
        let plugin = context.plugin_or_default::<PaneKeyboard>();
        let save = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::S);
        let mut text = String::new();
        context
            .run_ui(RawInput::default(), |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(widget))
                    .request_focus();
            })
            .drop_without_applying_deltas();
        context
            .run_ui(RawInput::default(), |ui| {
                let mut keyboard = plugin.lock();
                let input = keyboard.viewports.entry(ViewportId::ROOT).or_default();
                input.observe_surface(&context);
                let pass = context.cumulative_pass_nr();
                let pane = Id::unique("editor pane");
                input.handoff = Some(PendingInput {
                    ready: false,
                    previous_request: None,
                    handoff: Handoff::FrameworkTraversal {
                        pane,
                        settlement: TraversalSettlement::Forward { pass, wrap: false },
                    },
                    packet: PaneInputPacket {
                        target: PaneInputTarget {
                            slot,
                            item,
                            content: session.open_item_identity(item).unwrap(),
                            widget: Some(widget),
                        },
                        events: vec![Event::Key {
                            key: Key::S,
                            physical_key: Some(Key::S),
                            pressed: true,
                            repeat: false,
                            modifiers: save.modifiers,
                        }],
                    },
                });
                drop(keyboard);
                ui.memory_mut(|memory| memory.move_focus(FocusDirection::Next));
                ui.add(egui::TextEdit::singleline(&mut text).id(widget));
            })
            .drop_without_applying_deltas();
        assert_eq!(context.memory(egui::Memory::focused), None);
        for _ in 0..2 {
            context
                .run_ui(RawInput::default(), |ui| {
                    retain_skipped_pane_keyboard(ui.ctx(), instance, surface);
                    release_host_keys(ui, instance, surface, Vec::new());
                    assert!(!ui.input_mut(|input| input.consume_shortcut(&save)));
                })
                .drop_without_applying_deltas();
        }
        context
            .run_ui(RawInput::default(), |ui| {
                let toolbar = ui.button("Toolbar");
                assert!(toolbar.has_focus());
                observe_surface_keyboard(ui.ctx(), slot);
            })
            .drop_without_applying_deltas();
        context
            .run_ui(RawInput::default(), |ui| {
                let _ = ui.button("Toolbar");
                release_host_keys(ui, instance, surface, Vec::new());
                assert!(ui.input_mut(|input| input.consume_shortcut(&save)));
                assert!(!ui.input_mut(|input| input.consume_shortcut(&save)));
            })
            .drop_without_applying_deltas();
    }
}
