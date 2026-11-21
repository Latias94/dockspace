//! Ordered tab-list commands retain one cursor and one final core action.

use dockspace::runtime::{
    RetainedTabListOpener, TabListMenuPaintRecord, TabStripControlPaintRecord,
};
use egui::accesskit::{Action, ActionRequest, NodeId, TreeId};
use egui::{Event, EventFilter, Id, Key, PointerButton, Ui};

use super::super::{RenderContext, actions::effective_enabled};
use super::MenuRow;

mod routing;

use routing::{Intent, MenuKey};

#[derive(Clone)]
struct PendingOpening {
    frame: u64,
    id: Id,
    opener: RetainedTabListOpener,
    input: OpeningInput,
    prior_focus: Option<Id>,
}

#[derive(Clone)]
pub(super) struct OpeningInput {
    keys: Vec<MenuKey>,
    pub(super) retains_focus: bool,
}

fn opening_slot(context: &RenderContext<'_, '_, '_>) -> Id {
    context.instance_id.with((
        "tab-list-opening",
        context.plan.surface(),
        context.ui.ctx().viewport_id(),
    ))
}

pub(super) fn register_opener(
    context: &RenderContext<'_, '_, '_>,
    control: TabStripControlPaintRecord,
    id: Id,
    activated: bool,
) {
    if let Some(input) = routing::register_closed_opener(context.ui, id)
        && activated
        && let Some(opener) = context.plan.retain_tab_list_opener(control)
    {
        let context_ui = context.ui.ctx();
        let pending = PendingOpening {
            frame: context_ui.cumulative_frame_nr(),
            id,
            opener,
            input,
            prior_focus: context_ui.memory(egui::Memory::focused),
        };
        let slot = opening_slot(context);
        context_ui.data_mut(|data| data.insert_temp(slot, pending));
    }
}

pub(super) fn take_opening_input(
    context: &RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    opener: Id,
) -> Option<OpeningInput> {
    let ui = &*context.ui;
    let frame = ui.ctx().cumulative_frame_nr();
    let slot = opening_slot(context);
    let pending = ui.ctx().data_mut(|data| {
        let pending = data.get_temp::<PendingOpening>(slot)?;
        if pending.frame == frame {
            return None;
        }
        data.remove::<PendingOpening>(slot);
        Some(pending)
    })?;
    if pending.frame.checked_add(1) != Some(frame)
        || pending.id != opener
        || !context
            .plan
            .retain_tab_list_menu(menu)?
            .can_continue_opening(&pending.opener)
    {
        return None;
    }
    let mut input = pending.input;
    input.retains_focus &= ui.memory(egui::Memory::focused) == pending.prior_focus;
    Some(input)
}

pub(super) fn capture(
    context: &mut RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    id: Id,
    opener: Id,
    rows: &[MenuRow<'_>],
    opening: bool,
    opening_input: Option<OpeningInput>,
) {
    if !effective_enabled(context.ui, true) {
        return;
    }
    let focused = context.ui.memory(egui::Memory::focused);
    let focused_index = rows.iter().position(|row| Some(row.id) == focused);
    let routed = routing::register(context.ui, id, rows);
    if let Some(row) = focused_index.and_then(|index| rows.get(index)) {
        context.ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                row.id,
                EventFilter {
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                },
            );
        });
    }
    if let Some(routed) = routed {
        let outcome = match routed.intent {
            Intent::Focus(row) => rows
                .iter()
                .position(|candidate| candidate.id == row)
                .map(Outcome::Navigate),
            Intent::Activate(row) => rows
                .iter()
                .position(|candidate| candidate.id == row)
                .map(Outcome::Activate),
            Intent::Dismiss(row) => rows
                .iter()
                .any(|candidate| candidate.id == row)
                .then_some(Outcome::Dismiss),
        };
        if let Some(outcome) = outcome {
            apply(
                context,
                menu,
                id,
                opener,
                rows,
                outcome,
                routed.retains_focus,
                None,
            );
        }
        return;
    }
    if let Some(input) = opening_input {
        let mut cursor = rows.iter().position(|row| row.record.focused());
        let mut outcome = None;
        for key in input.keys {
            if let Some(next) = step(&mut cursor, rows.len().checked_sub(1), key) {
                outcome = Some(next);
            }
        }
        if let Some(outcome) = outcome {
            apply(
                context,
                menu,
                id,
                opener,
                rows,
                outcome,
                input.retains_focus,
                None,
            );
        }
        return;
    }
    let start = if opening {
        let Some(start) = after_opening_click(context.ui, opener) else {
            return;
        };
        start
    } else {
        0
    };
    let initial = if !opening && passed_focus_change_to_egui(context.ui, focused) {
        context.ui.memory(|memory| {
            rows.iter()
                .position(|row| memory.had_focus_last_frame(row.id))
        })
    } else {
        focused_index
    };
    if let Some(batch) = keyboard_batch(context.ui, rows, initial, focused, start) {
        apply(
            context,
            menu,
            id,
            opener,
            rows,
            batch.outcome,
            batch.retains_focus || (focused.is_none() && matches!(batch.outcome, Outcome::Dismiss)),
            Some(batch.events),
        );
        return;
    }
    if let Some(row) = focused_index.and_then(|index| rows.get(index))
        && !row.record.focused()
        && let Some(action) = context
            .plan
            .prepare_tab_list_menu_focus(menu, row.record.key())
    {
        context.push_local_action(row.id.with("framework-focus"), action);
    }
}

#[allow(clippy::too_many_arguments)]
fn apply(
    context: &mut RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    id: Id,
    opener: Id,
    rows: &[MenuRow<'_>],
    outcome: Outcome,
    retains_focus: bool,
    events: Option<Vec<(usize, Event)>>,
) {
    let action = match outcome {
        Outcome::Navigate(index) => rows.get(index).and_then(|row| {
            context
                .plan
                .prepare_tab_list_menu_focus(menu, row.record.key())
        }),
        Outcome::Activate(index) => rows.get(index).and_then(|row| {
            context
                .plan
                .prepare_tab_list_menu_row_activation(row.record)
        }),
        Outcome::Dismiss => context.plan.prepare_tab_list_menu_dismiss(menu.backdrop()),
    };
    let Some(action) = action else {
        return;
    };
    if let Some(events) = events {
        let consumed = context.ui.input_mut(|input| {
            if events
                .iter()
                .any(|(index, event)| input.events.get(*index) != Some(event))
            {
                return false;
            }
            for (index, _) in events.iter().rev() {
                input.events.remove(*index);
            }
            true
        });
        if !consumed {
            return;
        }
    }
    match outcome {
        Outcome::Navigate(index) if retains_focus => {
            context.ui.memory_mut(|memory| {
                memory.move_focus(egui::FocusDirection::None);
                if !memory.has_focus(rows[index].id) {
                    memory.request_focus(rows[index].id);
                }
            });
        }
        Outcome::Dismiss if retains_focus => restore_trigger_focus(context.ui, opener),
        Outcome::Navigate(_) | Outcome::Dismiss | Outcome::Activate(_) => {}
    }
    let origin = id.with("keyboard-input");
    if matches!(outcome, Outcome::Activate(_)) {
        context.push_tab_list_activation(origin, action, retains_focus);
    } else {
        context.push_local_action(origin, action);
    }
}

fn passed_focus_change_to_egui(ui: &Ui, focused: Option<Id>) -> bool {
    ui.input(|input| {
        input.events.iter().any(|event| match focused {
            // egui applies AccessKit focus before widget registration. Rebuild
            // the earlier owner so an Enter before that transfer keeps its target.
            Some(id) => accesskit_focus_target(event) == Some(id.accesskit_id()),
            // Escape can clear focus before a newly focused row has installed its filter.
            None => matches!(event, Event::Key {
                key: Key::Escape, pressed: true, modifiers, ..
            } if modifiers.matches_logically(egui::Modifiers::NONE)),
        })
    })
}

fn after_opening_click(ui: &Ui, opener: Id) -> Option<usize> {
    let Some(trigger) = ui.ctx().read_response(opener) else {
        return Some(0);
    };
    // Use the opener registered for input. The response's aggregate pointer
    // position can include a movement that occurred after the release.
    let hit = ui.ctx().viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .get(opener)
            .map_or(trigger.interact_rect, |widget| widget.interact_rect)
    });
    let from_global = ui.ctx().layer_transform_from_global(trigger.layer_id);
    let clicked = trigger.clicked_by(PointerButton::Primary);
    ui.input(|input| {
        let mut releases = input.events.iter().enumerate().filter_map(|(slot, event)| {
            let Event::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                pos,
                ..
            } = event
            else {
                return None;
            };
            let pos = from_global.map_or(*pos, |transform| transform * *pos);
            hit.contains(pos).then_some(slot)
        });
        match releases.next() {
            Some(release) => releases.next().is_none().then_some(release + 1),
            None if clicked => None,
            None => Some(0),
        }
    })
}

fn accesskit_focus_target(event: &Event) -> Option<NodeId> {
    match event {
        Event::AccessKitActionRequest(ActionRequest {
            action: Action::Focus,
            target_node,
            target_tree: TreeId::ROOT,
            data: None,
        }) => Some(*target_node),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Navigate(usize),
    Activate(usize),
    Dismiss,
}

struct KeyboardBatch {
    outcome: Outcome,
    events: Vec<(usize, Event)>,
    retains_focus: bool,
}

fn step(cursor: &mut Option<usize>, last: Option<usize>, key: MenuKey) -> Option<Outcome> {
    match (key, *cursor, last) {
        (MenuKey::Navigate(navigation), Some(index), Some(last)) => {
            let next = navigation.destination(index, last);
            *cursor = Some(next);
            Some(Outcome::Navigate(next))
        }
        (MenuKey::Activate, Some(index), _) => Some(Outcome::Activate(index)),
        (MenuKey::Dismiss, _, _) => Some(Outcome::Dismiss),
        _ => None,
    }
}

fn keyboard_batch(
    ui: &Ui,
    rows: &[MenuRow<'_>],
    mut cursor: Option<usize>,
    focused: Option<Id>,
    start: usize,
) -> Option<KeyboardBatch> {
    ui.input(|input| {
        // Paint records retain the initial focus for the whole packet. Fold
        // commands over one cursor before publishing its final semantic action.
        let last = rows.len().checked_sub(1);
        let mut outcome = None;
        let mut events = Vec::new();
        for (slot, event) in input.events.iter().enumerate().skip(start) {
            if let Some(target) = accesskit_focus_target(event) {
                if let Some(index) = rows.iter().position(|row| row.id.accesskit_id() == target) {
                    cursor = Some(index);
                    outcome = Some(Outcome::Navigate(index));
                    events.push((slot, event.clone()));
                } else if focused.is_some_and(|id| id.accesskit_id() == target) {
                    // egui recognized a later transfer to an application widget.
                    cursor = None;
                }
                continue;
            }
            let Some(key) = MenuKey::from_event(event) else {
                continue;
            };
            if let Some(next) = step(&mut cursor, last, key) {
                outcome = Some(next);
                events.push((slot, event.clone()));
                if matches!(next, Outcome::Activate(_) | Outcome::Dismiss) {
                    break;
                }
            }
        }
        outcome.map(|outcome| KeyboardBatch {
            outcome,
            events,
            retains_focus: cursor.is_some(),
        })
    })
}

fn restore_trigger_focus(ui: &Ui, target: Id) {
    if ui.ctx().viewport(|viewport| {
        viewport
            .this_pass
            .widgets
            .get(target)
            .is_some_and(|widget| widget.enabled && widget.visible && widget.sense.is_focusable())
    }) {
        ui.memory_mut(|memory| {
            memory.move_focus(egui::FocusDirection::None);
            memory.request_focus(target);
        });
    }
}
