//! Application-configurable docking gestures and tab/splitter shortcuts.

use egui::{Key, KeyboardShortcut, Modifiers};

/// Modifier policy for acquiring docking targets during a drag.
///
/// Modifier requirements allow additional held modifiers. Use
/// [`Modifiers::COMMAND`] for the platform's primary shortcut modifier, or
/// [`Modifiers::CTRL`] and [`Modifiers::MAC_CMD`] for physical keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockingActivation {
    /// Acquires docking targets without a modifier requirement.
    Always,
    /// Acquires docking targets only while every required modifier is held.
    HoldToEnable(Modifiers),
    /// Pauses docking while every required modifier is held.
    ///
    /// The drag can still move or float its source while docking is paused.
    HoldToDisable(Modifiers),
}

impl Default for DockingActivation {
    fn default() -> Self {
        Self::HoldToDisable(Modifiers::SHIFT)
    }
}

/// Action requested by a primary-button double click on an internal splitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitterDoubleClick {
    /// Equalizes adjacent pane extents within their current size constraints.
    Equalize,
    /// Restores declared preferred extents, equalizing when no preference exists.
    ///
    /// Current size constraints still apply. Preferred extents do not otherwise
    /// replace the user's stored split proportions during ordinary layout.
    Preferred,
}

/// Input bindings owned by one docking adapter instance.
///
/// Ordinary egui control navigation and application document shortcuts retain
/// their own input owners. Defaults pause docking while Shift is held, cancel
/// a drag with Escape, close tabs with a middle click, and restore preferred pane
/// extents with a primary-button double click on their splitter. Without a
/// preferred extent, double clicking equalizes the adjacent panes. Tab cycling is
/// opt-in through [`Self::next_tab`] and [`Self::previous_tab`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockInputConfig {
    /// Modifier policy shared by pointer edges and modifier-only observations.
    pub docking: DockingActivation,
    /// Shortcut which cancels an active docking gesture; `None` disables it.
    ///
    /// Additional modifiers are allowed, so cancellation remains available
    /// while a docking activation modifier is held.
    pub cancel_drag: Option<KeyboardShortcut>,
    /// Requests closure of the clicked tab with the middle mouse button.
    ///
    /// Uses the normal close policy and application decision workflow. It does
    /// not select an inactive tab or require its close icon to be visible.
    pub middle_click_closes_tab: bool,
    /// Action requested when a splitter is double-clicked; `None` disables it.
    ///
    /// Defaults to [`SplitterDoubleClick::Preferred`]. Core clamps the result
    /// to both panes' current size constraints.
    pub splitter_double_click: Option<SplitterDoubleClick>,
    /// Cycles forward in the group whose pane, tab, or built-in strip control owns focus.
    ///
    /// Disabled by default so application document shortcuts keep their input.
    /// Unlike gesture modifiers, Shift and Alt must match this shortcut exactly.
    /// Navigation wraps in visual order and keeps keyboard focus on the destination
    /// tab. Press Enter or Space to activate its content. Pane widgets may consume
    /// the shortcut first; popups, foreign focus, and pointer gestures prevent it.
    pub next_tab: Option<KeyboardShortcut>,
    /// Cycles backward using the same ownership and modifier rules as [`Self::next_tab`].
    pub previous_tab: Option<KeyboardShortcut>,
}

impl Default for DockInputConfig {
    fn default() -> Self {
        Self {
            docking: DockingActivation::default(),
            cancel_drag: Some(KeyboardShortcut::new(Modifiers::NONE, Key::Escape)),
            middle_click_closes_tab: true,
            splitter_double_click: Some(SplitterDoubleClick::Preferred),
            next_tab: None,
            previous_tab: None,
        }
    }
}

impl DockInputConfig {
    /// Returns whether the observed modifiers permit acquiring docking targets.
    ///
    /// Every requested modifier must be held; additional modifiers are allowed.
    /// Input must follow egui's modifier convention: `command` accompanies Ctrl
    /// on Windows/Linux and Command on macOS. Physical Ctrl and macOS Command
    /// remain distinct when explicitly requested.
    #[must_use]
    pub fn docking_enabled(&self, modifiers: Modifiers) -> bool {
        match self.docking {
            DockingActivation::Always => true,
            DockingActivation::HoldToEnable(required) => modifiers_include(modifiers, required),
            DockingActivation::HoldToDisable(required) => !modifiers_include(modifiers, required),
        }
    }

    /// Matches a logical key and its event-time modifiers against cancellation.
    ///
    /// Additional modifiers are allowed. This method only matches a binding;
    /// the adapter decides whether the key press owns an active gesture.
    #[must_use]
    pub fn cancel_matches(&self, key: Key, modifiers: Modifiers) -> bool {
        self.cancel_drag.is_some_and(|shortcut| {
            shortcut.logical_key == key && modifiers_include(modifiers, shortcut.modifiers)
        })
    }
}

fn modifiers_include(observed: Modifiers, required: Modifiers) -> bool {
    (!required.alt || observed.alt)
        && (!required.ctrl || observed.ctrl)
        && (!required.shift || observed.shift)
        && (!required.mac_cmd || observed.mac_cmd)
        && (!required.command || observed.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shift_policy_allows_extra_modifiers_and_keeps_escape_available() {
        let input = DockInputConfig::default();
        for (modifiers, enabled) in [
            (Modifiers::NONE, true),
            (Modifiers::SHIFT, false),
            (Modifiers::CTRL | Modifiers::COMMAND, true),
            (
                Modifiers::SHIFT | Modifiers::CTRL | Modifiers::COMMAND,
                false,
            ),
            (Modifiers::ALT, true),
            (Modifiers::ALT | Modifiers::SHIFT, false),
        ] {
            assert_eq!(input.docking_enabled(modifiers), enabled, "{modifiers:?}");
            assert!(input.cancel_matches(Key::Escape, modifiers));
            assert!(!input.cancel_matches(Key::Enter, modifiers));
        }
    }

    #[test]
    fn activation_can_require_a_chord_or_remove_the_modifier_gate() {
        let required = Modifiers::ALT | Modifiers::SHIFT;
        let input = DockInputConfig {
            docking: DockingActivation::HoldToEnable(required),
            ..Default::default()
        };
        for (modifiers, enabled) in [
            (Modifiers::NONE, false),
            (Modifiers::ALT, false),
            (Modifiers::SHIFT, false),
            (required, true),
            (required | Modifiers::CTRL | Modifiers::COMMAND, true),
        ] {
            assert_eq!(input.docking_enabled(modifiers), enabled, "{modifiers:?}");
        }
        let input = DockInputConfig {
            docking: DockingActivation::Always,
            ..input
        };
        assert!(input.docking_enabled(Modifiers::NONE));
        assert!(input.docking_enabled(required | Modifiers::CTRL | Modifiers::COMMAND));
    }

    #[test]
    fn hold_to_disable_can_use_alt_without_retaining_the_default_shift_binding() {
        let input = DockInputConfig {
            docking: DockingActivation::HoldToDisable(Modifiers::ALT),
            ..Default::default()
        };
        assert!(input.docking_enabled(Modifiers::SHIFT));
        assert!(!input.docking_enabled(Modifiers::ALT));
        assert!(!input.docking_enabled(Modifiers::ALT | Modifiers::SHIFT));
    }

    #[test]
    fn command_alias_uses_the_observed_platform_mapping_and_preserves_physical_keys() {
        let windows_control = Modifiers::CTRL | Modifiers::COMMAND;
        let mac_command = Modifiers::MAC_CMD | Modifiers::COMMAND;
        let mac_control = Modifiers::CTRL;
        for (required, observed, enabled) in [
            (Modifiers::COMMAND, windows_control, true),
            (Modifiers::COMMAND, mac_command, true),
            (Modifiers::COMMAND, mac_control, false),
            (Modifiers::MAC_CMD, mac_command, true),
            (Modifiers::MAC_CMD, windows_control, false),
            (Modifiers::CTRL, windows_control, true),
            (Modifiers::CTRL, mac_command, false),
        ] {
            let input = DockInputConfig {
                docking: DockingActivation::HoldToEnable(required),
                ..Default::default()
            };
            assert_eq!(
                input.docking_enabled(observed),
                enabled,
                "required={required:?}, observed={observed:?}",
            );
        }
    }

    #[test]
    fn cancellation_can_require_a_chord_or_be_disabled() {
        let input = DockInputConfig {
            cancel_drag: Some(KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Q,
            )),
            ..Default::default()
        };
        for modifiers in [
            Modifiers::CTRL | Modifiers::COMMAND,
            Modifiers::MAC_CMD | Modifiers::COMMAND,
        ] {
            assert!(!input.cancel_matches(Key::Q, modifiers));
            assert!(input.cancel_matches(Key::Q, modifiers | Modifiers::SHIFT));
            assert!(input.cancel_matches(Key::Q, modifiers | Modifiers::SHIFT | Modifiers::ALT));
            assert!(!input.cancel_matches(Key::W, modifiers | Modifiers::SHIFT));
        }
        assert!(!input.cancel_matches(Key::Q, Modifiers::CTRL | Modifiers::SHIFT));
        let input = DockInputConfig {
            cancel_drag: None,
            ..input
        };
        assert!(!input.cancel_matches(Key::Escape, Modifiers::NONE));
        assert!(!input.cancel_matches(Key::Q, Modifiers::COMMAND | Modifiers::SHIFT));
    }
}
