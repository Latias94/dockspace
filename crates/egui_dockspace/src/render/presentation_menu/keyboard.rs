//! One keyboard packet walks the current enabled menu entries in input order.

use egui::{Event, EventFilter, Id, Key, Ui};

pub(super) enum Outcome {
    Focus(Id),
    Activate(Id),
    Dismiss,
}

pub(super) fn capture(
    ui: &mut Ui,
    anchor: Id,
    entries: &[Id],
    submenus: &[Id],
    source_enabled: bool,
) -> Option<Outcome> {
    if !source_enabled
        || !ui.input(|input| input.focused)
        || !ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
    {
        return None;
    }
    let focused = ui.memory(egui::Memory::focused);
    let mut cursor = entries.iter().position(|entry| Some(*entry) == focused);
    if cursor.is_none()
        && focused.is_none()
        && ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(event, Event::Key {
                    key: Key::Escape, pressed: true, modifiers, ..
                } if modifiers.matches_logically(egui::Modifiers::NONE))
            })
        })
    {
        cursor = entries
            .iter()
            .position(|entry| ui.memory(|memory| memory.had_focus_last_frame(*entry)));
    }
    // Sizing disables the popup widgets. Framework focus released by Escape
    // can still belong to this menu's previous scope; application focus cannot.
    let can_navigate = !ui.is_sizing_pass()
        && ui.is_enabled()
        && (cursor.is_some() || focused.is_none() || focused == Some(anchor));
    let had_scope_focus = ui.memory(|memory| {
        memory.had_focus_last_frame(anchor)
            || entries
                .iter()
                .any(|entry| memory.had_focus_last_frame(*entry))
    });
    let can_dismiss =
        cursor.is_some() || focused == Some(anchor) || (focused.is_none() && had_scope_focus);
    if can_navigate
        && let Some(target) = cursor
            .map(|index| entries[index])
            .or_else(|| (focused == Some(anchor)).then_some(anchor))
    {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                target,
                EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: true,
                    escape: true,
                    ..Default::default()
                },
            );
        });
    }
    let (events, outcome) = ui.input(|input| {
        let mut events = Vec::new();
        let mut outcome = None;
        for (slot, event) in input.events.iter().enumerate() {
            let Event::Key {
                key,
                modifiers,
                pressed: true,
                ..
            } = event
            else {
                continue;
            };
            // Tab hands ownership to framework traversal, including this
            // packet's suffix. The menu must not reuse its preceding cursor.
            if *key == Key::Tab && (modifiers.is_none() || modifiers.shift_only()) {
                break;
            }
            if !modifiers.matches_logically(egui::Modifiers::NONE) {
                continue;
            }
            match key {
                Key::ArrowDown | Key::ArrowUp if can_navigate && !entries.is_empty() => {
                    let index = if *key == Key::ArrowDown {
                        cursor
                            .and_then(|index| index.checked_add(1))
                            .filter(|index| *index < entries.len())
                            .unwrap_or(0)
                    } else {
                        cursor
                            .and_then(|index| index.checked_sub(1))
                            .unwrap_or(entries.len() - 1)
                    };
                    cursor = Some(index);
                    outcome = Some(Outcome::Focus(entries[index]));
                }
                Key::ArrowLeft if can_navigate => {
                    outcome = Some(Outcome::Focus(anchor));
                    events.push((slot, event.clone()));
                    break;
                }
                Key::ArrowRight
                    if can_navigate
                        && cursor.is_some_and(|index| submenus.contains(&entries[index])) =>
                {
                    outcome = cursor.map(|index| Outcome::Activate(entries[index]));
                    events.push((slot, event.clone()));
                    break;
                }
                Key::Enter | Key::Space if can_navigate && cursor.is_some() => {
                    outcome = cursor.map(|index| Outcome::Activate(entries[index]));
                    events.push((slot, event.clone()));
                    break;
                }
                Key::Escape if can_dismiss => {
                    outcome = Some(Outcome::Dismiss);
                    events.push((slot, event.clone()));
                    break;
                }
                _ => continue,
            }
            events.push((slot, event.clone()));
        }
        (events, outcome)
    });
    let outcome = outcome?;
    if !ui.input_mut(|input| {
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
    }) {
        return None;
    }
    ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    match outcome {
        Outcome::Focus(target) => {
            ui.memory_mut(|memory| memory.request_focus(target));
        }
        Outcome::Activate(_) => {
            ui.close();
        }
        Outcome::Dismiss => {
            if focused.is_none()
                || focused == Some(anchor)
                || focused.is_some_and(|focused| entries.contains(&focused))
            {
                ui.memory_mut(|memory| memory.request_focus(anchor));
            }
            ui.close();
        }
    }
    ui.request_repaint();
    Some(outcome)
}
