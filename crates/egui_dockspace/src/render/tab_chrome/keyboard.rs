//! Ordered tab-list commands retain one cursor and one final core action.

use dockspace::runtime::{
    PreparedSurfaceAction, RetainedTabListOpener, TabListMenuPaintRecord,
    TabStripControlPaintRecord,
};
use egui::{Event, EventFilter, Id, IdMap, Key, PointerButton, Ui};

use super::super::menu_input::{
    self, Intent, MenuKey, MenuKind, accesskit_click_target, accesskit_focus_target,
    live_accesskit_click_target,
};
use super::super::{
    RenderContext,
    actions::{effective_enabled, focus_uninterrupted, unambiguous_pointer_click},
    geometry::egui_rect,
};
use super::{MenuRow, backdrop_id};

#[derive(Clone)]
struct PendingOpening {
    frame: u64,
    id: Id,
    opener: RetainedTabListOpener,
    keys: Vec<MenuKey>,
    outcome: Option<Outcome>,
    retains_focus: bool,
}

#[derive(Clone)]
struct PaintedOpener {
    pass: u64,
    opener: RetainedTabListOpener,
}

pub(super) enum OpeningInput {
    LocalClick,
    AwaitingPresentation {
        terminal: bool,
    },
    Ready {
        outcome: Option<Outcome>,
        retains_focus: bool,
    },
}

impl OpeningInput {
    pub(super) fn terminal(&self) -> bool {
        match self {
            Self::LocalClick => true,
            Self::AwaitingPresentation { terminal } => *terminal,
            Self::Ready { outcome, .. } => outcome.is_some_and(Outcome::is_terminal),
        }
    }

    pub(super) fn retains_focus(&self) -> bool {
        matches!(
            self,
            Self::Ready {
                retains_focus: true,
                ..
            }
        )
    }
}

fn opening_slot(context: &RenderContext<'_, '_, '_>) -> Id {
    context.instance_id.with((
        "tab-list-opening",
        context.plan.surface(),
        context.ui.ctx().viewport_id(),
    ))
}

fn pointer_openers_slot(context: &RenderContext<'_, '_, '_>) -> Id {
    context.instance_id.with((
        "tab-list-pointer-openers",
        context.plan.surface(),
        context.ui.ctx().viewport_id(),
    ))
}

pub(super) fn remember_pointer_opener(
    context: &RenderContext<'_, '_, '_>,
    control: TabStripControlPaintRecord,
    id: Id,
) {
    if context.pointer_authority.supports_preview_gestures()
        && let Some(opener) = context.plan.retain_tab_list_opener(control)
    {
        let pass = context.ui.ctx().cumulative_pass_nr();
        let slot = pointer_openers_slot(context);
        context.ui.ctx().data_mut(|data| {
            let openers = data.get_temp_mut_or_default::<IdMap<PaintedOpener>>(slot);
            openers.retain(|_, entry| entry.pass.saturating_add(1) >= pass);
            openers.insert(id, PaintedOpener { pass, opener });
        });
    }
}

pub(super) fn register_opening(
    context: &RenderContext<'_, '_, '_>,
    control: TabStripControlPaintRecord,
    id: Id,
    start: usize,
) {
    let Some(opener) = context.plan.retain_tab_list_opener(control) else {
        return;
    };
    retain_opening(context, opener, id, start);
}

fn retain_opening(
    context: &RenderContext<'_, '_, '_>,
    opener: RetainedTabListOpener,
    id: Id,
    start: usize,
) {
    let ui = &*context.ui;
    let focused = ui.memory(egui::Memory::focused);
    let click_keys = menu_input::take_tab_list_click_keys(ui, id, start);
    let other_opener = focused
        .filter(|&focused| focused != id && menu_input::tab_list_opener_has_focus(ui, focused));
    // An application widget already built in this pass may have seen Enter.
    // Preserve that confirmation while the new opener takes the next packet.
    let application_seen_key = focused
        .filter(|&focused| focused != id && other_opener != Some(focused))
        .is_some_and(|focused| {
            ui.ctx()
                .viewport(|viewport| viewport.this_pass.widgets.get(focused).is_some())
        });
    let application_took_focus = application_seen_key
        && focused.is_some_and(|focused| !ui.memory(|memory| memory.had_focus_last_frame(focused)));
    let click_boundary = ui.ctx().viewport(|viewport| {
        viewport
            .input
            .events
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(slot, event)| {
                live_accesskit_click_target(event, &viewport.prev_pass.widgets).map(|_| slot)
            })
    });
    let (keys, retains_focus) = ui.input_mut(|input| {
        let claims_click = click_keys.is_some();
        let mut retains_focus = claims_click || !application_took_focus;
        if let Some(other) = other_opener {
            // Two openers can accept separate confirmations in one packet.
            // Only the later click can take focus, independent of paint order.
            let last_click = input
                .raw
                .events
                .iter()
                .rev()
                .filter_map(accesskit_click_target)
                .find(|&target| target == other.accesskit_id() || target == id.accesskit_id());
            retains_focus &= last_click != Some(other.accesskit_id());
        }
        let mut keys = click_keys.unwrap_or_default();
        let mut slots = Vec::new();
        for (slot, event) in input.events.iter().enumerate().skip(start) {
            if accesskit_focus_target(event).is_some()
                || click_boundary == Some(slot)
                || matches!(event, Event::PointerButton { pressed: true, .. })
            {
                retains_focus = false;
                break;
            }
            if matches!(
                event,
                Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    ..
                }
            ) {
                retains_focus = false;
                break;
            }
            let Some(key) = MenuKey::from_event(event).filter(|key| key.is_tab_list()) else {
                continue;
            };
            if application_seen_key && !claims_click && matches!(key, MenuKey::Activate) {
                break;
            }
            keys.push(key);
            slots.push(slot);
            if key.is_terminal() {
                break;
            }
        }
        for slot in slots.into_iter().rev() {
            input.events.remove(slot);
        }
        (keys, retains_focus)
    });
    if retains_focus {
        menu_input::focus_tab_list_opener(
            ui,
            id,
            !keys.last().is_some_and(|key| key.is_terminal()),
        );
    }
    let slot = opening_slot(context);
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            slot,
            PendingOpening {
                frame,
                id,
                opener,
                keys,
                outcome: None,
                retains_focus,
            },
        );
    });
}

pub(super) fn take_menu_input(
    context: &mut RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    id: Id,
    opener: Id,
    rows: &[MenuRow<'_>],
) -> Option<OpeningInput> {
    let ui = &*context.ui;
    let presented = !context.pointer_authority.supports_preview_gestures()
        || context.plan.tab_list_menu_is_presented(menu);
    // Expose the targets for the next input packet. A command addressed to a
    // row needs a presented menu at delivery; only opener keys can wait for it.
    let routed = menu_input::register(ui, id, rows.iter().map(|row| row.id), MenuKind::TabList)
        .filter(|_| presented);
    if routed
        .as_ref()
        .is_some_and(|input| input.follows_primary_release)
        && let Some((origin, action)) = earlier_local_menu_click(context, menu, rows)
    {
        context.push_local_action(origin, action);
        return Some(OpeningInput::LocalClick);
    }
    let routed = routed.and_then(|routed| {
        let row_index = |id| rows.iter().position(|row| row.id == id);
        let outcome = match routed.intent {
            Intent::Focus(row) => row_index(row).map(Outcome::Navigate),
            Intent::Click(row) => row_index(row)
                .filter(|&index| row_click_is_presented(context, rows[index].record))
                .map(Outcome::Activate),
            Intent::Activate(row) => row_index(row).map(Outcome::Activate),
            Intent::Dismiss(row) => row_index(row).map(|_| Outcome::Dismiss),
            Intent::Back(_) => None,
        }?;
        Some((outcome, routed.retains_focus))
    });
    let ready = || {
        routed
            .map(|(outcome, retains_focus)| OpeningInput::Ready {
                outcome: Some(outcome),
                retains_focus,
            })
            .or_else(|| {
                (!presented).then_some(OpeningInput::AwaitingPresentation { terminal: false })
            })
    };
    let frame = ui.ctx().cumulative_frame_nr();
    let slot = opening_slot(context);
    if !presented
        && !ui
            .ctx()
            .data(|data| data.get_temp::<PendingOpening>(slot).is_some())
        && let Some(start) = after_opening_click(ui, opener).filter(|start| *start > 0)
        && let Some(previous) = ui.ctx().data(|data| {
            data.get_temp::<IdMap<PaintedOpener>>(pointer_openers_slot(context))
                .and_then(|openers| openers.get(&opener).cloned())
        })
        && previous.pass.checked_add(1) == Some(ui.ctx().cumulative_pass_nr())
        && context
            .plan
            .retain_tab_list_menu(menu)
            .is_some_and(|list| list.can_continue_opening(&previous.opener))
    {
        // Pointer ingress may open the menu before egui builds the surface.
        // Retain only the keys after the witnessed release.
        retain_opening(context, previous.opener, opener, start);
    }
    let Some(mut pending) = ui.ctx().data(|data| data.get_temp::<PendingOpening>(slot)) else {
        return ready();
    };
    let consecutive = pending.frame == frame || pending.frame.checked_add(1) == Some(frame);
    if !consecutive
        || pending.id != opener
        || !focus_uninterrupted(ui)
        || context
            .plan
            .retain_tab_list_menu(menu)
            .is_none_or(|menu| !menu.can_continue_opening(&pending.opener))
    {
        ui.ctx()
            .data_mut(|data| data.remove::<PendingOpening>(slot));
        return ready();
    }
    if pending.frame != frame {
        let (keys, retains_focus) = menu_input::take_opening_keys(ui, opener);
        if !pending.outcome.is_some_and(Outcome::is_terminal) {
            pending.keys.extend(keys);
        }
        pending.retains_focus &= retains_focus;
        pending.frame = frame;
    }
    let mut cursor = match pending.outcome {
        Some(Outcome::Navigate(index)) => Some(index),
        _ => menu.rows().position(|row| row.focused()),
    };
    let last = menu.rows().len().checked_sub(1);
    for key in pending.keys.drain(..) {
        if let Some(outcome) = step(&mut cursor, last, key) {
            pending.outcome = Some(outcome);
            if outcome.is_terminal() {
                break;
            }
        }
    }
    if !pending.outcome.is_some_and(Outcome::is_terminal)
        && let Some((outcome, retains_focus)) = routed
    {
        pending.outcome = Some(outcome);
        pending.retains_focus = retains_focus;
    }
    if !presented {
        // The open intent owns these keys until a renderer accepts its menu.
        let terminal = pending.outcome.is_some_and(Outcome::is_terminal);
        if pending.retains_focus && !terminal {
            menu_input::expect_tab_list(ui, opener);
        }
        ui.ctx().data_mut(|data| data.insert_temp(slot, pending));
        ui.ctx().request_repaint();
        return Some(OpeningInput::AwaitingPresentation { terminal });
    }
    ui.ctx().data_mut(|data| {
        data.remove::<PendingOpening>(slot);
    });
    Some(OpeningInput::Ready {
        outcome: pending.outcome,
        retains_focus: pending.retains_focus,
    })
}

fn earlier_local_menu_click(
    context: &RenderContext<'_, '_, '_>,
    menu: TabListMenuPaintRecord<'_>,
    rows: &[MenuRow<'_>],
) -> Option<(Id, PreparedSurfaceAction)> {
    if !context.pointer_authority.accepts_local_pointer_actions() {
        return None;
    }
    let ui = &*context.ui;
    // Use egui's resolved click from the preceding widget registrations and
    // recheck the hit against the menu's current bounds and prepared action.
    let id = ui.ctx().interaction_snapshot(|snapshot| snapshot.clicked)?;
    let mut response = ui.ctx().read_response(id)?;
    let backdrop = menu.backdrop();
    let (hit, action) = if id == backdrop_id(context, backdrop) {
        (
            backdrop.bounds(),
            context.plan.prepare_tab_list_menu_dismiss(backdrop),
        )
    } else {
        let row = rows.iter().find(|row| row.id == id)?.record;
        if context.plan.receiver_for_tab_list_menu_row(row).is_none() {
            return None;
        }
        (
            row.interaction_bounds()?,
            context.plan.prepare_tab_list_menu_row_activation(row),
        )
    };
    let hit = egui_rect(hit)?;
    response.interact_rect = response
        .interact_rect
        .intersect(hit)
        .intersect(ui.clip_rect());
    unambiguous_pointer_click(ui, &response, PointerButton::Primary)
        .then_some((id.with("pointer-click"), action?))
}

pub(super) fn row_click_is_presented(
    context: &RenderContext<'_, '_, '_>,
    row: dockspace::runtime::TabListMenuRowPaintRecord<'_>,
) -> bool {
    !context.pointer_authority.supports_preview_gestures()
        || context.plan.tab_list_menu_row_is_presented(row)
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
    if !effective_enabled(context.ui, true)
        || matches!(opening_input, Some(OpeningInput::LocalClick))
    {
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
    if let Some(OpeningInput::Ready {
        outcome: Some(outcome),
        retains_focus,
    }) = opening_input
    {
        apply(
            context,
            menu,
            id,
            opener,
            rows,
            outcome,
            retains_focus,
            None,
        );
        return;
    }
    if menu_input::awaits_application(context.ui, id)
        || (!opening
            && menu_input::started_outside(context.ui, id, |initial| {
                initial == opener || rows.iter().any(|row| row.id == initial)
            }))
    {
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

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Navigate(usize),
    Activate(usize),
    Dismiss,
}

impl Outcome {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Activate(_) | Self::Dismiss)
    }
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
        (MenuKey::Dismiss, Some(_), _) => Some(Outcome::Dismiss),
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
    ui.ctx().viewport(|viewport| {
        // Paint records retain the initial focus for the whole packet. Fold
        // commands over one cursor before publishing its final semantic action.
        let last = rows.len().checked_sub(1);
        let mut outcome = None;
        let mut events = Vec::new();
        for (slot, event) in viewport.input.events.iter().enumerate().skip(start) {
            if live_accesskit_click_target(event, &viewport.prev_pass.widgets).is_some() {
                cursor = None;
                break;
            }
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
            let Some(key) = MenuKey::from_event(event).filter(|key| key.is_tab_list()) else {
                continue;
            };
            if let Some(next) = step(&mut cursor, last, key) {
                outcome = Some(next);
                events.push((slot, event.clone()));
                if next.is_terminal() {
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
