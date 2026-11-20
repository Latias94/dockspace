//! Ordered tab-list commands retain one cursor and one final core action.

use dockspace::runtime::TabListMenuPaintRecord;
use egui::accesskit::{Action, ActionRequest, NodeId, TreeId};
use egui::{Event, EventFilter, Id, Key, PointerButton, Ui};

use super::super::{RenderContext, actions::effective_enabled};
use super::MenuRow;

pub(super) fn capture(
    context: &mut RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    id: Id,
    opener: Id,
    rows: &[MenuRow<'_>],
    opening: bool,
) {
    if !effective_enabled(context.ui, true) {
        return;
    }
    let focused = context.ui.memory(egui::Memory::focused);
    let focused_index = rows.iter().position(|row| Some(row.id) == focused);
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
        let action = match batch.outcome {
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
        let consumed = context.ui.input_mut(|input| {
            if batch
                .events
                .iter()
                .any(|(index, event)| input.events.get(*index) != Some(event))
            {
                return false;
            }
            for (index, _) in batch.events.iter().rev() {
                input.events.remove(*index);
            }
            true
        });
        if !consumed {
            return;
        }
        match batch.outcome {
            Outcome::Navigate(index) if batch.retains_focus => {
                if let Some(row) = rows.get(index) {
                    context.ui.memory_mut(|memory| {
                        memory.move_focus(egui::FocusDirection::None);
                        if !memory.has_focus(row.id) {
                            memory.request_focus(row.id);
                        }
                    });
                }
            }
            Outcome::Dismiss if batch.retains_focus || focused.is_none() => {
                restore_trigger_focus(context.ui, opener);
            }
            Outcome::Navigate(_) | Outcome::Dismiss | Outcome::Activate(_) => {}
        }
        context.push_local_action(id.with("keyboard-input"), action);
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
            let Event::Key {
                key,
                modifiers,
                pressed: true,
                ..
            } = event
            else {
                continue;
            };
            if !modifiers.matches_logically(egui::Modifiers::NONE) {
                continue;
            }
            if let (Some(index), Some(last)) = (cursor, last) {
                let next = match key {
                    Key::ArrowUp => Some(index.saturating_sub(1)),
                    Key::ArrowDown => Some(index.saturating_add(1).min(last)),
                    Key::Home => Some(0),
                    Key::End => Some(last),
                    _ => None,
                };
                if let Some(next) = next {
                    cursor = Some(next);
                    outcome = Some(Outcome::Navigate(next));
                    events.push((slot, event.clone()));
                    continue;
                }
                if matches!(key, Key::Enter | Key::Space) {
                    outcome = Some(Outcome::Activate(index));
                    events.push((slot, event.clone()));
                    break;
                }
            }
            if *key == Key::Escape {
                outcome = Some(Outcome::Dismiss);
                events.push((slot, event.clone()));
                break;
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
