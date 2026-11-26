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

mod handoff;
mod queue;
mod replay;
mod widget;

pub(in crate::render) use handoff::PaneInputState;
pub(super) use handoff::{activate, continue_at, prepare};
use handoff::{
    defer, framework_traversal, has_pointer_or_accessibility_edges, is_plain_escape,
    pointer_keeps_focus,
};
pub(crate) use queue::release_host_keys;
pub(crate) use queue::retain_skipped_pane_keyboard;
use queue::{
    defer_framework_traversal, focus_before_live_escape, foreign_pending_input, layout_retry,
    observe_surface_keyboard, rebind_packets, retire_input, return_queued_host_keys,
    settle_framework_keys, stage_live_input, store, take,
};
use replay::DeliveryPhase;
pub(in crate::render) use replay::PaneInputDelivery;
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
    RetryRegistration {
        focus_cleared_by_escape: bool,
    },
    RetryDelivery {
        focus_cleared_by_escape: bool,
    },
}

#[derive(Clone, Copy)]
enum TraversalSettlement {
    PassEnd(u64),
    Forward { pass: u64, wrap: bool },
    NextPassStart,
}

impl Handoff {
    fn is_retry(self) -> bool {
        matches!(
            self,
            Self::RetryRegistration { .. } | Self::RetryDelivery { .. }
        )
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

fn is_key_event(event: &Event) -> bool {
    matches!(event, Event::Key { .. })
}

fn is_host_key(event: &Event) -> bool {
    matches!(event, Event::Key { pressed: false, .. })
        || matches!(event, Event::Key { modifiers, .. }
            if modifiers.command || modifiers.ctrl || modifiers.alt)
}

fn slot_id(context: &RenderContext<'_, '_, '_>) -> Id {
    keyboard_slot(context.instance_id, context.plan.surface())
}

fn keyboard_slot(instance: Id, surface: dockspace::model::SurfaceId) -> Id {
    instance.with(("pane-keyboard-handoff", surface))
}

/// Takes the keyboard portion of a suffix while leaving pointer and system events in place.
pub(super) fn take_keys_after(ui: &Ui, start: usize) -> Vec<Event> {
    take_keyboard_suffix(ui.ctx(), start)
}

fn take_keyboard_suffix(context: &Context, start: usize) -> Vec<Event> {
    context.input_mut(|input| {
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
