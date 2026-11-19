//! Viewport-local keyboard input awaiting an exact pane focus handoff.

use dockspace::{
    model::{ItemId, TabKey},
    runtime::{DockspaceContentId, PanePaintRecord},
};
use egui::{Context, Event, FocusDirection, Id, Key, Ui};

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
    if pending.content != context.plan.content_identity(pending.item)
        || !super::can_capture(context)
        || crate::focus::input_may_surrender_focus(context.ui.ctx())
    {
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
    let widget = match pending.handoff {
        Handoff::Continuation { widget } if focused == Some(widget) => {
            pending.events.extend(take_keys_after(context.ui, 0));
            widget
        }
        Handoff::Activation {
            header,
            awaiting_publication,
        } if focused == Some(header) => {
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
            target
        }
        _ => return PaneInputState::None,
    };
    PaneInputState::Ready(PaneInputDelivery {
        item: pending.item,
        widget,
        events: pending.events,
    })
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
    pub(in crate::render) fn targets(&self, item: ItemId) -> bool {
        self.item == item
    }

    pub(in crate::render) fn take_for(
        delivery: &mut Option<Self>,
        ui: &Ui,
        pane: PanePaintRecord<'_>,
    ) -> Option<Self> {
        let expected = delivery.as_ref()?;
        (expected.targets(pane.item())
            && super::super::actions::effective_enabled(ui, true)
            && ui.memory(|memory| memory.has_focus(expected.widget)))
        .then(|| delivery.take())
        .flatten()
    }

    pub(in crate::render) fn replay(self, context: &Context) -> ScopedPaneInput {
        // The ordinary widget API exposes an unaddressed event list. Keep the
        // replay local to this pane, including when another pane paints later.
        let mut events = self.events;
        let replayed = !events.is_empty();
        context.input_mut(|input| {
            events.append(&mut input.events);
            input.events = events;
        });
        ScopedPaneInput {
            context: context.clone(),
            replayed,
        }
    }
}

pub(in crate::render) struct ScopedPaneInput {
    context: Context,
    replayed: bool,
}

impl Drop for ScopedPaneInput {
    fn drop(&mut self) {
        if self.replayed {
            self.context.input_mut(|input| {
                input.events.retain(|event| !is_keyboard_input(event));
            });
        }
    }
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

fn is_keyboard_input(event: &Event) -> bool {
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
