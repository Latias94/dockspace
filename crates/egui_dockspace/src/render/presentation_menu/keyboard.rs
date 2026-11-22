//! One keyboard packet walks the current enabled menu entries in input order.

use egui::{Event, EventFilter, Id, Key, Ui};

use crate::render::menu_input::{self, Intent, MenuKind, Navigation, live_accesskit_click_target};

pub(super) enum Outcome {
    Focus(Id),
    Activate { target: Id, retains_focus: bool },
    Dismiss,
}

pub(super) fn capture(
    ui: &mut Ui,
    id: Id,
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
    if !ui.is_sizing_pass()
        && let Some(routed) = menu_input::register(
            ui,
            id,
            entries.iter().copied(),
            MenuKind::Product {
                anchor,
                submenus: submenus.to_vec(),
            },
        )
    {
        let outcome = match routed.intent {
            Intent::Focus(target) if target == anchor || entries.contains(&target) => {
                Outcome::Focus(target)
            }
            Intent::Back(target) if target == anchor => Outcome::Focus(anchor),
            Intent::Click(target) | Intent::Activate(target) if entries.contains(&target) => {
                Outcome::Activate {
                    target,
                    retains_focus: routed.retains_focus,
                }
            }
            Intent::Dismiss(_) => Outcome::Dismiss,
            _ => return None,
        };
        return Some(apply(ui, anchor, outcome, routed.retains_focus));
    }
    if menu_input::awaits_application(ui, id)
        || menu_input::started_outside(ui, id, |initial| {
            initial == anchor || entries.contains(&initial)
        })
    {
        return None;
    }
    let (events, outcome) = ui.ctx().viewport(|viewport| {
        let mut events = Vec::new();
        let mut outcome = None;
        for (slot, event) in viewport.input.events.iter().enumerate() {
            if live_accesskit_click_target(event, &viewport.prev_pass.widgets).is_some() {
                break;
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
            // Tab hands ownership to framework traversal, including this
            // packet's suffix. The menu must not reuse its preceding cursor.
            if *key == Key::Tab && (modifiers.is_none() || modifiers.shift_only()) {
                break;
            }
            if !modifiers.matches_logically(egui::Modifiers::NONE) {
                continue;
            }
            match key {
                Key::ArrowDown | Key::ArrowUp | Key::Home | Key::End
                    if can_navigate && !entries.is_empty() =>
                {
                    let direction = match key {
                        Key::ArrowDown => Navigation::Next,
                        Key::ArrowUp => Navigation::Previous,
                        Key::Home => Navigation::First,
                        _ => Navigation::Last,
                    };
                    let index = direction.wrapping_destination(cursor, entries.len() - 1);
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
                    outcome = cursor.map(|index| Outcome::Activate {
                        target: entries[index],
                        retains_focus: true,
                    });
                    events.push((slot, event.clone()));
                    break;
                }
                Key::Enter | Key::Space if can_navigate && cursor.is_some() => {
                    outcome = cursor.map(|index| Outcome::Activate {
                        target: entries[index],
                        retains_focus: true,
                    });
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
    Some(apply(ui, anchor, outcome, can_dismiss || can_navigate))
}

fn apply(ui: &Ui, anchor: Id, outcome: Outcome, retains_focus: bool) -> Outcome {
    if retains_focus {
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    match outcome {
        Outcome::Focus(target) => {
            if retains_focus {
                ui.memory_mut(|memory| memory.request_focus(target));
            }
        }
        Outcome::Activate { .. } => {
            ui.close();
        }
        Outcome::Dismiss => {
            if retains_focus {
                ui.memory_mut(|memory| memory.request_focus(anchor));
            }
            ui.close();
        }
    }
    ui.request_repaint();
    outcome
}
