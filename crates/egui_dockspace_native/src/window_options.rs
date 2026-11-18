//! Application-owned appearance for managed child OS windows.

use std::collections::BTreeMap;
use std::sync::Arc;

use dockspace::runtime::NativeWindowBinding;
use eframe::egui::{IconData, ViewportBuilder, ViewportId, WindowLevel};

/// Last declared icons and creation-only taskbar state for managed viewports.
///
/// egui's builder patch cannot clear an icon or remember an explicit clear.
pub(crate) type DeclaredWindowIcons = BTreeMap<ViewportId, DeclaredWindowIcon>;

pub(crate) struct DeclaredWindowIcon {
    pub(crate) window: NativeWindowBinding,
    pub(crate) icon: Option<Arc<IconData>>,
    // Creation-only state remains fixed for this viewport's native lifetime.
    pub(crate) taskbar: bool,
}

/// Appearance defaults or per-surface overrides for managed child OS windows.
///
/// The external Root window keeps its application's `eframe::NativeOptions`.
/// In a surface override, `None` inherits the corresponding global option. A
/// field left unset after inheritance uses Dockspace's defaults: decorations and
/// Windows taskbar presence are enabled, the window level is normal, and there
/// is no window icon. Live properties return to these defaults when an override
/// is removed; taskbar presence remains fixed for the current window lifetime.
/// The all-`None` [`Default`] remains suitable for an override
/// that inherits every global field. Platform support for decorations, taskbar
/// presence, icons and stacking may differ.
/// Initial and recovery placement, visibility, activation and pointer passthrough
/// remain owned by Dockspace's native lifecycle. Once a window is live, the
/// application may request ordinary window movement through its UI host; core
/// consumes the provider's fresh geometry observations rather than treating the
/// requested position as fact.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeChildWindowOptions {
    /// Whether the platform should draw window decorations; defaults to `true`.
    pub decorations: Option<bool>,
    /// Whether a new child should appear in the Windows taskbar; defaults to `true`.
    ///
    /// This is fixed at the first declaration of each native window lifetime.
    /// Changes apply to subsequently created or replaced windows. Egui can only
    /// change it by recreating the OS window, which must not bypass Dockspace's
    /// native retirement and first-live protocol.
    pub taskbar: Option<bool>,
    /// Requested stacking level, such as [`WindowLevel::AlwaysOnTop`].
    /// Defaults to [`WindowLevel::Normal`] after inheritance.
    /// This does not order peers within the same level or establish a native parent relationship.
    pub window_level: Option<WindowLevel>,
    /// Application icon for the configured managed child windows.
    ///
    /// In an override, `None` inherits the global icon. A final `None` clears a
    /// previously configured icon and suppresses eframe's parent-icon fallback.
    pub icon: Option<Arc<IconData>>,
}

impl NativeChildWindowOptions {
    pub(crate) fn with_overrides(&self, overrides: &Self) -> Self {
        Self {
            decorations: overrides.decorations.or(self.decorations),
            taskbar: overrides.taskbar.or(self.taskbar),
            window_level: overrides.window_level.or(self.window_level),
            icon: overrides.icon.as_ref().or(self.icon.as_ref()).cloned(),
        }
    }

    pub(crate) fn viewport_builder(&self) -> ViewportBuilder {
        ViewportBuilder {
            decorations: Some(self.decorations.unwrap_or(true)),
            taskbar: Some(self.taskbar.unwrap_or(true)),
            window_level: Some(self.window_level.unwrap_or(WindowLevel::Normal)),
            icon: self.icon.clone(),
            ..Default::default()
        }
    }
}
