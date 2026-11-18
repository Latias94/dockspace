//! Optional customization of the adapter's presentation menus.

use egui::Id;

use crate::{ItemId, RootId, SurfaceId, TabBarId, TabGroupDockMode, TabKey};

pub use dockspace::runtime::DockspacePresentationCommandKind;

/// The exact subject of the open menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockMenuTarget {
    /// The complete root identified by [`DockMenuContext::root`].
    Root,
    /// One tab group, including its direct layout tabs.
    TabGroup(TabBarId),
    /// One application or layout tab.
    Tab(TabKey),
    /// One active application pane chosen from the root menu.
    Pane(ItemId),
}

/// Location and subject of a menu. Group and layout-tab IDs are session-local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DockMenuContext {
    /// Logical surface containing the source, including managed native surfaces.
    pub surface: SurfaceId,
    /// Docking root containing the source.
    pub root: RootId,
    /// Scope of the open menu page.
    pub target: DockMenuTarget,
}

/// A row's identity within the current presentation menu page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DockMenuItemKind {
    /// A built-in float, native move or directional dock action.
    Presentation(DockspacePresentationCommandKind),
    /// Opens the destination list for docking a tab group.
    GroupDestinations(TabGroupDockMode),
    /// Docks this tab group into a specific target group.
    GroupDestination(TabGroupDockMode, TabBarId),
    /// Opens the root's list of active panes.
    ActivePanes,
    /// Opens one active pane's commands.
    ActivePane(ItemId),
    /// Requests a close through the regular close workflow.
    Close,
    /// Returns to the preceding menu page.
    Back,
    /// Application action with an ID unique within the page.
    Custom(Id),
    /// Non-interactive divider between rows.
    Separator,
}

/// A presentation menu row. Built-in rows describe commands for the current page.
#[derive(Clone, Debug)]
pub struct DockMenuItem {
    pub(crate) kind: DockMenuItemKind,
    /// Visible and accessible row label.
    pub label: String,
    /// Optional tooltip and accessible description.
    pub description: Option<String>,
    /// Whether the application allows activation. Core availability also applies.
    pub enabled: bool,
}

impl DockMenuItem {
    /// Creates an enabled row delivered to [`crate::PaneView::on_menu_action`].
    ///
    /// Use an ID independent of the label and its position in the menu. If a
    /// page contains the same ID more than once, only the first row is shown.
    pub fn custom(id: Id, label: impl Into<String>) -> Self {
        Self {
            kind: DockMenuItemKind::Custom(id),
            label: label.into(),
            description: None,
            enabled: true,
        }
    }

    /// Creates a separator which is displayed between visible rows.
    #[must_use]
    pub const fn separator() -> Self {
        Self {
            kind: DockMenuItemKind::Separator,
            label: String::new(),
            description: None,
            enabled: false,
        }
    }

    /// Returns the row's identity, independent of its current label.
    #[must_use]
    pub const fn kind(&self) -> DockMenuItemKind {
        self.kind
    }
}

/// Menu title and rows passed to [`crate::PaneView::customize_menu`].
#[derive(Debug)]
#[non_exhaustive]
pub struct DockMenu {
    pub(crate) context: DockMenuContext,
    /// Visible heading. An empty string hides the heading.
    pub heading: String,
    /// Name read by assistive technology when the menu opens.
    pub accessible_label: String,
    /// Built-in and application rows in display order.
    pub items: Vec<DockMenuItem>,
}

impl DockMenu {
    /// Returns the logical location and exact subject of the open menu page.
    #[must_use]
    pub const fn context(&self) -> DockMenuContext {
        self.context
    }
}

/// An application action selected from a presentation menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockMenuAction {
    /// Source location and subject at the time of activation.
    pub context: DockMenuContext,
    /// ID supplied to [`DockMenuItem::custom`].
    pub id: Id,
}
