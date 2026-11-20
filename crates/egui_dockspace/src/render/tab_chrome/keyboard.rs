//! Ordered tab-list commands retain one cursor and one final core action.

use dockspace::runtime::{TabListMenuPaintRecord, TabListMenuRowPaintRecord};
use egui::{Event, EventFilter, Id, Key, PointerButton, Ui};

use super::super::{
    RenderContext,
    actions::{effective_enabled, pointer_click_event},
};
use super::{tab_list_menu_row_id, tab_list_menu_trigger_id};

pub(super) fn capture(
    context: &mut RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    id: Id,
    rows: &[TabListMenuRowPaintRecord],
    opening: bool,
) {
    if !effective_enabled(context.ui, true) {
        return;
    }
    let focused = context.ui.memory(egui::Memory::focused);
    let focused_index = rows
        .iter()
        .position(|row| Some(tab_list_menu_row_id(context, *row)) == focused)
        .or_else(|| {
            // A newly focused row may not have installed its filter before
            // egui processes Escape. Recover that packet's original owner only
            // when no application widget has since taken focus.
            (focused.is_none()
                && context.ui.input(|input| {
                    input.events.iter().any(|event| {
                        matches!(event, Event::Key {
                            key: Key::Escape, pressed: true, modifiers, ..
                        } if modifiers.matches_logically(egui::Modifiers::NONE))
                    })
                }))
            .then(|| {
                rows.iter().position(|row| {
                    context.ui.memory(|memory| {
                        memory.had_focus_last_frame(tab_list_menu_row_id(context, *row))
                    })
                })
            })
            .flatten()
        });
    if let Some(row) = focused_index.and_then(|index| rows.get(index)) {
        context.ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                tab_list_menu_row_id(context, *row),
                EventFilter {
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                },
            );
        });
    }
    let start = if opening {
        let Some(start) = after_opening_click(context, menu) else {
            return;
        };
        start
    } else {
        0
    };
    if let Some(batch) = keyboard_batch(context.ui, focused_index, rows.len(), start) {
        let action = match batch.outcome {
            Outcome::Navigate(index) => rows
                .get(index)
                .and_then(|row| context.plan.prepare_tab_list_menu_focus(menu, row.key())),
            Outcome::Activate(index) => rows
                .get(index)
                .and_then(|row| context.plan.prepare_tab_list_menu_row_activation(*row)),
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
            Outcome::Navigate(index) => {
                if let Some(row) = rows.get(index) {
                    let target = tab_list_menu_row_id(context, *row);
                    context.ui.memory_mut(|memory| {
                        memory.move_focus(egui::FocusDirection::None);
                        memory.request_focus(target);
                    });
                }
            }
            Outcome::Dismiss => restore_trigger_focus(context, menu),
            Outcome::Activate(_) => {}
        }
        context.push_local_action(id.with("keyboard-input"), action);
        return;
    }
    if let Some(row) = focused_index.and_then(|index| rows.get(index))
        && !row.focused()
        && let Some(action) = context.plan.prepare_tab_list_menu_focus(menu, row.key())
    {
        let origin = tab_list_menu_row_id(context, *row).with("framework-focus");
        context.push_local_action(origin, action);
    }
}

fn after_opening_click(
    context: &RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
) -> Option<usize> {
    let target = tab_list_menu_trigger_id(context, menu.tab_bar_visual_id());
    let Some(trigger) = context
        .ui
        .ctx()
        .read_response(target)
        .filter(|trigger| trigger.clicked_by(PointerButton::Primary))
    else {
        return Some(0);
    };
    // The core has already applied this packet's pointer edges. A newly opened
    // menu receives only the keys after its opener's release. If that release
    // cannot be identified in the remaining input, retain the current row.
    let release = pointer_click_event(context.ui, &trigger, PointerButton::Primary)?;
    context.ui.input(|input| {
        input
            .events
            .iter()
            .position(|event| *event == release)
            .map(|index| index + 1)
    })
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
}

fn keyboard_batch(
    ui: &Ui,
    mut cursor: Option<usize>,
    row_count: usize,
    start: usize,
) -> Option<KeyboardBatch> {
    ui.input(|input| {
        // Paint records retain the initial focus for the whole packet. Fold
        // commands over one cursor before publishing its final semantic action.
        let last = row_count.checked_sub(1);
        let mut outcome = None;
        let mut events = Vec::new();
        for (slot, event) in input.events.iter().enumerate().skip(start) {
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
        outcome.map(|outcome| KeyboardBatch { outcome, events })
    })
}

fn restore_trigger_focus(context: &RenderContext<'_, '_, '_>, menu: TabListMenuPaintRecord<'_>) {
    let target = tab_list_menu_trigger_id(context, menu.tab_bar_visual_id());
    if context.ui.ctx().viewport(|viewport| {
        viewport
            .this_pass
            .widgets
            .get(target)
            .is_some_and(|widget| widget.enabled && widget.visible && widget.sense.is_focusable())
    }) {
        context.ui.memory_mut(|memory| {
            memory.move_focus(egui::FocusDirection::None);
            memory.request_focus(target);
        });
    }
}
