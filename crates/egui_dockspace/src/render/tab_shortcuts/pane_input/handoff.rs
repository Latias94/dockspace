//! Admission of pane editing input during focus handoffs.

use super::*;

pub(in crate::render::tab_shortcuts) fn activate(
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

pub(in crate::render::tab_shortcuts) fn continue_at(
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
    pub(super) fn capture(
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

pub(super) fn defer(
    context: &RenderContext<'_, '_, '_>,
    packet: PaneInputPacket,
    handoff: Handoff,
) {
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

pub(in crate::render::tab_shortcuts) fn prepare(
    context: &mut RenderContext<'_, '_, '_>,
) -> PaneInputState {
    observe_surface_keyboard(context.ui.ctx(), slot_id(context));
    if foreign_pending_input(context) {
        return PaneInputState::Waiting;
    }
    if context.ui.ctx().current_pass_index() != 0 && !layout_retry(context) {
        // egui calls the UI again with the same input to resolve layout. The
        // following packet is eligible at the next input hook, never here.
        return PaneInputState::Waiting;
    }
    let Some((mut pending, retained)) = take(context) else {
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
    if !current_content {
        retire_input(context);
        return PaneInputState::None;
    }
    if !context.panes.accepts_deferred_keyboard_input(item)
        || (matches!(
            pending.handoff,
            Handoff::Continuation | Handoff::FrameworkTraversal { .. }
        ) && focus_moves)
    {
        return_host_keys(context, pending.packet.events);
        return PaneInputState::None;
    }
    if matches!(pending.handoff, Handoff::FrameworkTraversal { .. }) {
        if context.ui.ctx().current_pass_index() == 0
            && !has_pointer_or_accessibility_edges(context.ui.ctx())
            && crate::render::tab_shortcuts::can_route_keyboard(context)
        {
            stage_live_input(context, &mut pending.packet);
        }
        store(context, pending);
        return PaneInputState::Waiting;
    }
    if let Handoff::RetryRegistration {
        focus_cleared_by_escape,
    } = pending.handoff
    {
        return if context.pane_focus_request == pending.previous_request {
            PaneInputState::Ready(PaneInputDelivery {
                packet: pending.packet,
                phase: DeliveryPhase::AwaitingRegistration,
                focus_cleared_by_escape,
            })
        } else {
            return_host_keys(context, pending.packet.events);
            PaneInputState::None
        };
    }
    if let Handoff::RetryDelivery {
        focus_cleared_by_escape,
    } = pending.handoff
    {
        return if context.pane_focus_request == pending.previous_request
            && crate::render::tab_shortcuts::can_route_keyboard(context)
        {
            PaneInputState::Ready(PaneInputDelivery {
                packet: pending.packet,
                phase: DeliveryPhase::Registered { retry: true },
                focus_cleared_by_escape,
            })
        } else {
            return_host_keys(context, pending.packet.events);
            PaneInputState::None
        };
    }
    if !pending.ready {
        // A layout retry has no new input and must not reapply the handoff.
        store(context, pending);
        return PaneInputState::Waiting;
    }
    let focused = context.ui.memory(egui::Memory::focused);
    let activation_escaped = matches!(pending.handoff, Handoff::Activation { header, .. }
        if focused.is_none() && focus_before_live_escape(context.ui.ctx(), header));
    if activation_escaped || matches!(pending.handoff, Handoff::CancelledActivation) {
        // Escape still belongs to the tab. The editor has yet to receive focus.
        let request = context
            .pane_focus_request
            .filter(|request| Some(*request) != pending.previous_request);
        return_host_keys(context, std::mem::take(&mut pending.packet.events));
        if let Some(request) = request {
            if request.item() == Some(item) {
                crate::focus::cancel_pane_focus_request(
                    context.ui.ctx(),
                    context.instance_id,
                    request,
                );
            }
        } else if matches!(
            pending.handoff,
            Handoff::Activation {
                awaiting_publication: false,
                ..
            }
        ) {
            // The activation's selection may commit in this pass. Remember
            // the cancellation until its focus request can be observed.
            pending.ready = false;
            pending.handoff = Handoff::CancelledActivation;
            store(context, pending);
        }
        return PaneInputState::Waiting;
    }
    let focus_cleared_by_escape = focused.is_none()
        && widget.is_some_and(|widget| focus_before_live_escape(context.ui.ctx(), widget));
    let phase = match pending.handoff {
        Handoff::Continuation
            if widget.is_some_and(|widget| {
                (focused == Some(widget) || focus_cleared_by_escape)
                    && pointer_keeps_focus(context.ui.ctx(), widget)
            }) && crate::render::tab_shortcuts::can_route_keyboard(context) =>
        {
            stage_live_input(context, &mut pending.packet);
            if retained {
                // A retained output keeps focus alive without registering a new
                // widget. Validate the editor again before replaying its packet.
                DeliveryPhase::AwaitingRegistration
            } else {
                DeliveryPhase::Registered { retry: false }
            }
        }
        Handoff::Activation {
            header,
            awaiting_publication,
        } if focused == Some(header)
            && crate::render::tab_shortcuts::can_capture(context)
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
                return_host_keys(context, pending.packet.events);
                return PaneInputState::None;
            }
            if request.and_then(|request| request.item()) != Some(item) {
                return_host_keys(context, pending.packet.events);
                return PaneInputState::None;
            }
            if !has_pointer_or_accessibility_edges(context.ui.ctx()) {
                stage_live_input(context, &mut pending.packet);
            }
            DeliveryPhase::AwaitingRegistration
        }
        _ => {
            return_host_keys(context, pending.packet.events);
            return PaneInputState::None;
        }
    };
    PaneInputState::Ready(PaneInputDelivery {
        packet: pending.packet,
        phase,
        focus_cleared_by_escape,
    })
}

pub(super) fn pointer_keeps_focus(context: &Context, widget: Id) -> bool {
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

pub(super) fn has_pointer_or_accessibility_edges(context: &Context) -> bool {
    context.input(|input| {
        input.raw.events.iter().any(|event| {
            matches!(
                event,
                Event::PointerButton { .. } | Event::AccessKitActionRequest(_)
            )
        })
    })
}

pub(super) fn is_plain_escape(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Escape, pressed: true, modifiers, .. }
        if modifiers.is_none())
}

pub(super) fn framework_traversal(event: &Event) -> Option<FocusDirection> {
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
