//! Optional customization of the adapter's presentation menus.

use egui::{Color32, CornerRadius, Id, Rect, Response, Ui};

use crate::{ItemId, RootId, SurfaceId, TabBarId, TabGroupDockMode, TabKey};

pub use dockspace::runtime::DockspacePresentationCommandKind;

mod icons;

/// The menu opened by an adapter-owned button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DockMenuButtonKind {
    /// Actions for the complete layout, including a contained floating layout.
    Layout,
    /// Actions for one tab group.
    TabGroup,
    /// The overflow list of one group's open tabs.
    TabList,
}

/// Paint-only customization of an adapter-owned menu button.
///
/// The adapter retains hit testing, popup placement, focus and accessibility.
/// Paint within the supplied Ui's clip without registering another control.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DockMenuButton<'a> {
    /// Which menu this control opens.
    pub kind: DockMenuButtonKind,
    /// Logical surface, root and subject of the menu.
    pub context: DockMenuContext,
    /// Core-owned drawing allocation; hit bounds are in [`Self::response`].
    pub rect: Rect,
    /// Adapter-owned interaction, including hover, press and keyboard focus.
    pub response: &'a Response,
    /// Whether the adapter currently allows this control to accept input.
    pub enabled: bool,
    /// Whether this button's menu is open in the current pass.
    pub open: bool,
    /// Resolved background color; idle action buttons are transparent.
    pub fill: Color32,
    /// Resolved icon color, before the supplied Ui's opacity.
    pub icon_color: Color32,
    /// Default background rounding, independent of the hit bounds.
    pub corner_radius: CornerRadius,
}

impl DockMenuButton<'_> {
    /// Paints the default background and scope-specific icon.
    pub fn paint(&self, ui: &Ui) {
        self.paint_background(ui);
        self.paint_icon(ui);
    }

    /// Paints the resolved background, for use with an application icon.
    pub fn paint_background(&self, ui: &Ui) {
        ui.painter_at(self.rect)
            .rect_filled(self.rect, self.corner_radius, self.fill);
    }

    /// Paints the default icon, for use with an application background.
    pub fn paint_icon(&self, ui: &Ui) {
        let painter = ui.painter_at(self.rect);
        match self.kind {
            DockMenuButtonKind::Layout => {
                icons::layout_actions(&painter, self.rect, self.icon_color)
            }
            DockMenuButtonKind::TabGroup => {
                icons::group_actions(&painter, self.rect, self.icon_color)
            }
            DockMenuButtonKind::TabList => icons::tab_list(&painter, self.rect, self.icon_color),
        }
    }
}

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
