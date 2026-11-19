//! Translation from current-pass egui responses into opaque core actions.

use egui::accesskit::Action;
use egui::{Event, Id, Key, PointerButton, Response, Ui};

use super::PointerActionAuthority;

#[derive(Clone, Copy)]
struct PointerClickScope {
    pass: u64,
    cancelled: [bool; egui::NUM_POINTER_BUTTONS],
    focus_uninterrupted: bool,
}

fn pointer_click_scope(ui: &Ui) -> PointerClickScope {
    let context = ui.ctx();
    let pass = context.cumulative_pass_nr();
    let id = Id::unique(("egui-dockspace-pointer-click-scope", context.viewport_id()));
    let previous = context.data(|data| data.get_temp::<PointerClickScope>(id));
    if let Some(scope) = previous.filter(|scope| scope.pass == pass) {
        return scope;
    }
    // This only vetoes egui clicks after deactivation or a missed paint pass.
    // It does not choose receivers, synthesize clicks, or authorize core input.
    let mut cancelled = previous
        .filter(|scope| scope.pass.checked_add(1) == Some(pass))
        .map_or([true; egui::NUM_POINTER_BUTTONS], |scope| scope.cancelled);
    let focus_uninterrupted = ui.input(|input| {
        let host_focused = input.raw.focused;
        let mut focused = host_focused;
        let mut uninterrupted = focused;
        if !focused {
            cancelled.fill(true);
        }
        for event in &input.raw.events {
            match event {
                Event::WindowFocused(value) => {
                    focused = host_focused && *value;
                    if !focused {
                        cancelled.fill(true);
                        uninterrupted = false;
                    }
                }
                Event::PointerButton {
                    button,
                    pressed: true,
                    ..
                } if focused => cancelled[*button as usize] = false,
                _ => {}
            }
        }
        uninterrupted && input.focused
    });
    let scope = PointerClickScope {
        pass,
        cancelled,
        focus_uninterrupted,
    };
    context.data_mut(|data| data.insert_temp(id, scope));
    scope
}

pub(super) fn observe_pointer_focus(ui: &Ui) {
    pointer_click_scope(ui);
}

pub(super) fn focus_uninterrupted(ui: &Ui) -> bool {
    pointer_click_scope(ui).focus_uninterrupted
}

pub(super) fn effective_enabled(ui: &Ui, enabled: bool) -> bool {
    enabled && ui.is_enabled() && ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
}

/// Keeps egui's widget routing while rejecting ambiguous per-pass button aggregates.
///
/// A response identifies only one clicked widget, whereas `clicked_by` reports any click of that
/// button in the pass. Mixed buttons, multiple releases, or a new press after
/// release cannot identify the same event reliably.
pub(super) fn unambiguous_pointer_click(
    ui: &Ui,
    response: &Response,
    button: PointerButton,
) -> bool {
    if pointer_click_scope(ui).cancelled[button as usize] || !response.clicked_by(button) {
        return false;
    }
    let Some(position) = response.interact_pointer_pos() else {
        return false;
    };
    let from_global = ui.ctx().layer_transform_from_global(response.layer_id);
    ui.input(|input| {
        if [
            PointerButton::Primary,
            PointerButton::Secondary,
            PointerButton::Middle,
            PointerButton::Extra1,
            PointerButton::Extra2,
        ]
        .into_iter()
        .any(|other| other != button && input.pointer.button_down(other))
        {
            return false;
        }
        let mut release = None;
        for event in &input.raw.events {
            if let Event::PointerButton {
                pos,
                button: source,
                pressed,
                ..
            } = event
                && (*source != button
                    || (*pressed && release.is_some())
                    || (!*pressed && release.replace(*pos).is_some()))
            {
                return false;
            }
        }
        release.is_some_and(|release| {
            let release = from_global.map_or(release, |transform| transform * release);
            release == position && response.interact_rect.contains(release)
        })
    })
}

pub(super) fn button_activation_origin(
    ui: &Ui,
    response: &Response,
    pointer_authority: PointerActionAuthority,
) -> Option<Id> {
    if !effective_enabled(ui, response.enabled()) {
        return None;
    }
    if response.has_focus()
        && let Some(key) = ui.input_mut(|input| {
            [Key::Enter, Key::Space]
                .into_iter()
                .find(|&key| input.consume_key(egui::Modifiers::NONE, key))
        })
    {
        return Some(response.id.with(("keyboard-activate", key)));
    }
    pointer_or_accesskit_activation_origin(ui, response, pointer_authority)
}

pub(super) fn pointer_or_accesskit_activation_origin(
    ui: &Ui,
    response: &Response,
    pointer_authority: PointerActionAuthority,
) -> Option<Id> {
    if !effective_enabled(ui, response.enabled()) {
        return None;
    }
    if ui.input(|input| input.has_accesskit_action_request(response.id, Action::Click)) {
        return Some(response.id.with("accesskit-click"));
    }
    (pointer_authority.accepts_local_pointer_actions()
        && unambiguous_pointer_click(ui, response, PointerButton::Primary))
    .then(|| response.id.with("pointer-click"))
}
