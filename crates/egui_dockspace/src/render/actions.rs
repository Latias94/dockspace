//! Translation from current-pass egui responses into opaque core actions.

use egui::accesskit::Action;
use egui::{Event, Id, Key, PointerButton, Response, Ui};

use super::PointerActionAuthority;
pub(super) fn effective_enabled(ui: &Ui, enabled: bool) -> bool {
    enabled && ui.is_enabled() && ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
}

/// Keeps egui's widget routing while rejecting ambiguous per-pass button aggregates.
///
/// A response identifies only one clicked widget, whereas `clicked_by` reports any click of that
/// button in the pass. Mixed buttons and multiple releases cannot identify the same event reliably.
pub(super) fn unambiguous_pointer_click(
    ui: &Ui,
    response: &Response,
    button: PointerButton,
) -> bool {
    if !response.clicked_by(button) {
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
                && (*source != button || (!*pressed && release.replace(*pos).is_some()))
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
    if ui.input(|input| input.has_accesskit_action_request(response.id, Action::Click)) {
        return Some(response.id.with("accesskit-click"));
    }
    (pointer_authority.accepts_local_pointer_actions()
        && unambiguous_pointer_click(ui, response, PointerButton::Primary))
    .then(|| response.id.with("pointer-click"))
}
