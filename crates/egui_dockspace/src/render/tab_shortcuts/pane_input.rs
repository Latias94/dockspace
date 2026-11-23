//! Viewport-local keyboard input awaiting an exact pane focus handoff.

use dockspace::{
    model::{ItemId, TabKey},
    runtime::{DockspaceContentId, PanePaintRecord},
};
use egui::{Context, Event, FocusDirection, Id, Key, Rect, Ui};

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
}

#[derive(Clone)]
struct PendingInput {
    frame: u64,
    item: ItemId,
    content: DockspaceContentId,
    previous_request: Option<dockspace::runtime::DockspacePaneFocusRequest>,
    handoff: Handoff,
    events: Vec<Event>,
}

#[derive(Clone, Default)]
struct InputSlot(Option<PendingInput>);

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
        frame: context.ui.ctx().cumulative_frame_nr(),
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
    let id = slot_id(context);
    let Some(mut pending) = context
        .ui
        .ctx()
        .data_mut(|data| data.remove_temp::<InputSlot>(id).and_then(|slot| slot.0))
    else {
        return PaneInputState::None;
    };
    let frame = context.ui.ctx().cumulative_frame_nr();
    if pending.content != context.plan.content_identity(pending.item) {
        return PaneInputState::None;
    }
    if frame == pending.frame {
        // A layout retry has no new input and must not reapply the handoff.
        context
            .ui
            .ctx()
            .data_mut(|data| data.insert_temp(id, InputSlot(Some(pending))));
        return PaneInputState::Waiting;
    }
    if pending.frame.checked_add(1) != Some(frame) {
        return PaneInputState::None;
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
                    pending.frame = frame;
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
    let id = slot_id(context);
    context
        .ui
        .ctx()
        .data_mut(|data| data.insert_temp(id, InputSlot(Some(pending))));
    context.ui.ctx().request_repaint();
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
        if matches!(self.phase, DeliveryPhase::AwaitingRegistration)
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
    context.instance_id.with((
        "pane-keyboard-handoff",
        context.ui.ctx().viewport_id(),
        context.plan.surface(),
    ))
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
