//! Menu rows derived from the current paint plan and edited by the application.

use std::collections::{HashMap, HashSet};

use dockspace::model::{DockspaceActionRejection, RootId, TabGroupDockMode, TabLabel};
use dockspace::runtime::{
    DockspacePresentationCommandKind, DockspacePresentationCommandUnavailable,
    DockspaceRuntimeError, DockspaceTabsGroupCommandKind, DockspaceVisualId, PanePaintRecord,
    PresentationCommandPaintRecord, SurfacePaintProgramOperation, SurfacePaintProgramStep,
    TabBarPaintRecord,
};
use egui::accesskit::{Action, HasPopup, Role};
use egui::{Button, Id, Response, Ui, UiBuilder};

use crate::{DockMenu, DockMenuContext, DockMenuItem, DockMenuItemKind, TabBarId, TabKey};

use super::{MenuAction, MenuPageKind, RenderContext, Scope};

const COMMANDS: [DockspacePresentationCommandKind; 8] = [
    DockspacePresentationCommandKind::Float,
    DockspacePresentationCommandKind::MoveToNewWindow,
    DockspacePresentationCommandKind::DockBack,
    DockspacePresentationCommandKind::DockLeft,
    DockspacePresentationCommandKind::DockRight,
    DockspacePresentationCommandKind::DockTop,
    DockspacePresentationCommandKind::DockBottom,
    DockspacePresentationCommandKind::DockCenter,
];

struct GroupDestination {
    visual: DockspaceVisualId,
    label: String,
    command: PresentationCommandPaintRecord,
}

pub(super) struct PreparedMenu {
    pub(super) menu: DockMenu,
    actions: HashMap<DockMenuItemKind, Option<MenuAction>>,
}

pub(super) fn prepare(
    context: &RenderContext<'_, '_, '_>,
    scope: Scope<'_>,
    page: Option<MenuPageKind>,
) -> Result<Option<PreparedMenu>, DockspaceRuntimeError> {
    let (panes, root_is_pane) = if let Scope::Root(anchor) = scope {
        active_panes(context, anchor.root())?
    } else {
        (Vec::new(), false)
    };
    let menu_scope = match page {
        Some(MenuPageKind::Pane(visual)) => {
            let Some(pane) = panes.iter().find(|pane| pane.visual_id() == visual) else {
                return Ok(None);
            };
            Scope::Pane(*pane)
        }
        _ => scope,
    };
    let accessible_label = match page {
        Some(MenuPageKind::GroupTargets(mode)) => group_mode_label(mode),
        Some(MenuPageKind::ActivePanes) => "Active panes",
        _ => menu_scope.label(),
    };
    let heading = match (menu_scope, page) {
        (_, Some(MenuPageKind::GroupTargets(mode))) => group_mode_label(mode).to_owned(),
        (_, Some(MenuPageKind::ActivePanes)) => "Choose one active pane".to_owned(),
        (Scope::Root(_), _) => "Entire layout".to_owned(),
        (Scope::Tabs(bar), _) => context.plan.tabs_group_label(bar).map_or_else(
            || "Tab group".to_owned(),
            |label| format!("Tab group: {}", label_text(context, label, None)),
        ),
        (Scope::Tab(tab), _) => {
            let Some(resource) = context.resources.tab(tab.key()) else {
                return Ok(None);
            };
            let scope = if tab.key().is_layout() {
                "Layout tab"
            } else {
                "Pane tab"
            };
            format!("{scope}: {}", resource.title)
        }
        (Scope::Pane(pane), _) => {
            format!(
                "Pane: {}",
                label_text(context, TabLabel::Item(pane.item()), None)
            )
        }
    };
    let mut prepared = PreparedMenu {
        menu: DockMenu {
            context: DockMenuContext {
                surface: context.plan.surface(),
                root: menu_scope.root(),
                target: menu_scope.target(),
            },
            heading,
            accessible_label: accessible_label.to_owned(),
            items: Vec::new(),
        },
        actions: HashMap::new(),
    };

    if page.is_none_or(|page| matches!(page, MenuPageKind::Pane(_))) {
        for (label, command) in commands(context, menu_scope)? {
            prepared.command(
                DockMenuItemKind::Presentation(command.kind()),
                label,
                command,
            );
        }
    }
    match (scope, page) {
        (Scope::Tabs(bar), None) => {
            for mode in [
                TabGroupDockMode::MergeTabs,
                TabGroupDockMode::PreserveLayout,
            ] {
                let destinations = group_destinations(context, bar, mode)?;
                let available = destinations.iter().any(|entry| entry.command.is_ready());
                prepared.push(
                    DockMenuItemKind::GroupDestinations(mode),
                    group_mode_label(mode),
                    Some(if available {
                        group_mode_description(mode)
                    } else {
                        "No other eligible visible tab group is available."
                    }),
                    available.then_some(MenuAction::OpenPage(MenuPageKind::GroupTargets(mode))),
                );
            }
        }
        (Scope::Tabs(bar), Some(MenuPageKind::GroupTargets(mode))) => {
            for destination in group_destinations(context, bar, mode)? {
                prepared.command(
                    DockMenuItemKind::GroupDestination(mode, TabBarId(destination.visual)),
                    destination.label,
                    destination.command,
                );
            }
        }
        (Scope::Root(_), None) if !root_is_pane => {
            prepared.push(
                DockMenuItemKind::ActivePanes,
                "Active Panes…",
                Some("Choose a visible pane to move or close only that application's content."),
                (!panes.is_empty()).then_some(MenuAction::OpenPage(MenuPageKind::ActivePanes)),
            );
        }
        (Scope::Root(_), Some(MenuPageKind::ActivePanes)) => {
            for pane in panes {
                prepared.push(
                    DockMenuItemKind::ActivePane(pane.item()),
                    label_text(context, TabLabel::Item(pane.item()), None),
                    Some("Open actions for this application pane only."),
                    Some(MenuAction::OpenPage(MenuPageKind::Pane(pane.visual_id()))),
                );
            }
        }
        _ => {}
    }

    prepared.menu.items.push(DockMenuItem::separator());
    if page.is_none_or(|page| matches!(page, MenuPageKind::Pane(_))) {
        let (label, close) = match menu_scope {
            Scope::Root(anchor) => (
                "Close Layout",
                context.plan.prepare_root_close(anchor.root()),
            ),
            Scope::Tabs(bar) => ("Close Tab Group", context.plan.prepare_tabs_close(bar)),
            Scope::Tab(tab) => (
                if tab.key().is_layout() {
                    "Close Layout Tab"
                } else {
                    "Close Tab"
                },
                context.plan.prepare_tab_body_close(tab.key()),
            ),
            Scope::Pane(pane) => ("Close Pane", context.plan.prepare_pane_close(pane)),
        };
        prepared.push(
            DockMenuItemKind::Close,
            label,
            Some(menu_scope.description()),
            close.map(MenuAction::Close),
        );
    }
    if let Some(page) = page {
        let label = match page {
            MenuPageKind::GroupTargets(_) => "Back to Tab Group Actions",
            MenuPageKind::ActivePanes => "Back to Layout Actions",
            MenuPageKind::Pane(_) => "Back to Active Panes",
        };
        prepared.push(
            DockMenuItemKind::Back,
            label,
            Some("Return to the previous menu page."),
            Some(MenuAction::Back),
        );
    }
    Ok(Some(prepared))
}

impl PreparedMenu {
    fn command(
        &mut self,
        kind: DockMenuItemKind,
        label: impl Into<String>,
        command: PresentationCommandPaintRecord,
    ) {
        let description = command.unavailable_reason().map(unavailable_text);
        let action = command.is_ready().then_some(MenuAction::Command(command));
        self.push(kind, label, description, action);
    }

    fn push(
        &mut self,
        kind: DockMenuItemKind,
        label: impl Into<String>,
        description: Option<&str>,
        action: Option<MenuAction>,
    ) {
        self.menu.items.push(DockMenuItem {
            kind,
            label: label.into(),
            description: description.map(str::to_owned),
            enabled: action.is_some(),
        });
        self.actions.insert(kind, action);
    }

    pub(super) fn paint(
        mut self,
        ui: &mut Ui,
        popup_id: Id,
        ui_enabled: bool,
        entries: &mut Vec<(Response, MenuAction)>,
    ) {
        ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
            node.set_role(Role::Menu);
            node.set_label(self.menu.accessible_label);
        });
        if !self.menu.heading.is_empty() {
            ui.weak(self.menu.heading);
            ui.separator();
        }
        let mut custom_ids = HashSet::new();
        let mut had_row = false;
        let mut separator_pending = false;
        for item in self.menu.items {
            let action = match item.kind() {
                DockMenuItemKind::Separator => {
                    separator_pending = had_row;
                    continue;
                }
                DockMenuItemKind::Custom(id) if custom_ids.insert(id) => {
                    Some(MenuAction::Custom(id))
                }
                // Only rows issued for this exact page can resolve to built-in actions.
                kind => {
                    let Some(action) = self.actions.remove(&kind) else {
                        continue;
                    };
                    action
                }
            };
            if std::mem::take(&mut separator_pending) {
                ui.separator();
            }
            let response = paint_item(
                ui,
                popup_id.with(item.kind()),
                &item,
                action.as_ref(),
                ui_enabled,
            );
            if response.enabled()
                && let Some(action) = action
            {
                entries.push((response, action));
            }
            had_row = true;
        }
    }
}

fn paint_item(
    ui: &mut Ui,
    id: Id,
    item: &DockMenuItem,
    action: Option<&MenuAction>,
    ui_enabled: bool,
) -> Response {
    let submenu = matches!(
        item.kind(),
        DockMenuItemKind::GroupDestinations(_)
            | DockMenuItemKind::ActivePanes
            | DockMenuItemKind::ActivePane(_)
    );
    let response = ui
        .scope_builder(UiBuilder::new().scope_id(id), |ui| {
            let mut button = Button::new(item.label.as_str()).wrap();
            if submenu {
                button = button.right_text("›");
            }
            ui.add_enabled(ui_enabled && item.enabled && action.is_some(), button)
        })
        .inner;
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::MenuItem);
        node.set_label(item.label.as_str());
        if let Some(description) = &item.description {
            node.set_description(description.as_str());
        }
        if submenu {
            node.set_has_popup(HasPopup::Menu);
            node.set_expanded(false);
        }
        node.clear_actions();
        if response.enabled() {
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
        } else {
            node.set_disabled();
        }
    });
    match &item.description {
        Some(description) if response.enabled() => response.on_hover_text(description),
        Some(description) => response.on_disabled_hover_text(description),
        None => response,
    }
}

fn label_text(
    context: &RenderContext<'_, '_, '_>,
    label: TabLabel<'_>,
    key: Option<TabKey>,
) -> String {
    match label {
        TabLabel::Item(item) => context.resources.tab(item).map_or_else(
            || {
                context.panes.title(item).map_or_else(
                    || format!("Missing pane {}", item.get()),
                    |title| title.text().to_owned(),
                )
            },
            |resource| resource.title.clone(),
        ),
        TabLabel::Layout { title } => key.map_or_else(
            || title.to_owned(),
            |key| context.panes.layout_tab_title(key, title).text().to_owned(),
        ),
    }
}

fn active_panes<'plan>(
    context: &RenderContext<'_, 'plan, '_>,
    root_id: RootId,
) -> Result<(Vec<PanePaintRecord<'plan>>, bool), DockspaceRuntimeError> {
    let mut panes = Vec::new();
    let mut root_is_pane = false;
    context
        .plan
        .visit_surface_program(|step| {
            match step {
                SurfacePaintProgramStep::RootBegin { root, .. } if root.root() == root_id => {
                    root_is_pane = matches!(root.label(), TabLabel::Item(_));
                }
                SurfacePaintProgramStep::Operation {
                    operation: SurfacePaintProgramOperation::Pane(pane),
                    paint,
                    ..
                } if pane.root() == root_id && !paint.is_source_gap() => {
                    // The core owns visibility, ancestry and occlusion admission.
                    if context
                        .plan
                        .pane_command(pane, DockspacePresentationCommandKind::Float)?
                        .is_some()
                    {
                        panes.push(pane);
                    }
                }
                _ => {}
            }
            Ok::<(), DockspaceRuntimeError>(())
        })
        .map_err(|error| error.into_runtime_error())?;
    Ok((panes, root_is_pane))
}

fn group_destinations(
    context: &RenderContext<'_, '_, '_>,
    source: TabBarPaintRecord<'_>,
    mode: TabGroupDockMode,
) -> Result<Vec<GroupDestination>, DockspaceRuntimeError> {
    let mut destinations = Vec::new();
    context
        .plan
        .visit_surface_program(|step| {
            if let SurfacePaintProgramStep::Operation {
                operation: SurfacePaintProgramOperation::TabBar(target),
                paint,
                ..
            } = step
                && !paint.is_source_gap()
                && let Some(command) = context
                    .plan
                    .tabs_group_center_command(source, target, mode)?
                && let Some(label) = context.plan.tabs_group_label(target)
            {
                destinations.push(GroupDestination {
                    visual: target.visual_id(),
                    label: group_destination_label(context, target, label),
                    command,
                });
            }
            Ok::<(), DockspaceRuntimeError>(())
        })
        .map_err(|error| error.into_runtime_error())?;
    Ok(destinations)
}

fn group_destination_label(
    context: &RenderContext<'_, '_, '_>,
    bar: TabBarPaintRecord<'_>,
    label: TabLabel<'_>,
) -> String {
    let member_title = |member: dockspace::runtime::TabStripMemberPaintRecord<'_>| {
        let title = context.resources.tab(member.key()).map_or_else(
            || label_text(context, member.label(), Some(member.key())),
            |resource| resource.title.clone(),
        );
        let mut chars = title
            .chars()
            .map(|ch| if ch.is_whitespace() { ' ' } else { ch });
        let mut title = chars.by_ref().take(24).collect::<String>();
        if chars.next().is_some() {
            title.push('…');
        }
        title
    };
    // Only direct strip members identify a destination; a layout member keeps
    // its own title rather than borrowing names from its selected descendants.
    let count = bar.members().len();
    let mut titles = bar.members().take(3).map(member_title).collect::<Vec<_>>();
    if count > titles.len() {
        titles.push(format!("+{} more", count - titles.len()));
    }
    let members = if titles.is_empty() {
        "Empty group".to_owned()
    } else {
        titles.join(", ")
    };
    let selection = bar
        .selected()
        .and_then(|selected| bar.members().find(|member| member.key() == selected))
        .map_or_else(
            || "No selected tab".to_owned(),
            |member| format!("Selected: {}", member_title(member)),
        );
    format!(
        "{}\n{members} · {selection}",
        label_text(context, label, None)
    )
}

const fn group_mode_label(mode: TabGroupDockMode) -> &'static str {
    match mode {
        TabGroupDockMode::MergeTabs => "Merge Tab Group Into…",
        TabGroupDockMode::PreserveLayout => "Dock Tab Group as Layout Tab…",
    }
}

const fn group_mode_description(mode: TabGroupDockMode) -> &'static str {
    match mode {
        TabGroupDockMode::MergeTabs => "Append this group's direct tabs to the destination group.",
        TabGroupDockMode::PreserveLayout => {
            "Insert this complete group as one layout tab in the destination group."
        }
    }
}

fn commands(
    context: &RenderContext<'_, '_, '_>,
    scope: Scope<'_>,
) -> Result<Vec<(&'static str, PresentationCommandPaintRecord)>, DockspaceRuntimeError> {
    let mut entries = Vec::new();
    match scope {
        Scope::Root(anchor) => {
            for kind in COMMANDS {
                if let Some(command) = context.plan.presentation_command(anchor.root(), kind)? {
                    entries.push((command_label(kind), command));
                }
            }
        }
        Scope::Tabs(bar) => {
            for (kind, label) in [
                (DockspaceTabsGroupCommandKind::Float, "Float Tab Group"),
                (
                    DockspaceTabsGroupCommandKind::MoveToNewWindow,
                    "Move Tab Group to New Window",
                ),
            ] {
                if let Some(command) = context.plan.tabs_group_command(bar, kind)? {
                    entries.push((label, command));
                }
            }
        }
        Scope::Tab(tab) => {
            for kind in COMMANDS {
                if let Some(command) = context.plan.tab_command(tab, kind)? {
                    let label = match kind {
                        DockspacePresentationCommandKind::Float if tab.key().is_layout() => {
                            "Float Layout Tab"
                        }
                        DockspacePresentationCommandKind::Float => "Float Tab",
                        DockspacePresentationCommandKind::MoveToNewWindow
                            if tab.key().is_layout() =>
                        {
                            "Move Layout Tab to New Window"
                        }
                        DockspacePresentationCommandKind::MoveToNewWindow => {
                            "Move Tab to New Window"
                        }
                        _ => command_label(kind),
                    };
                    entries.push((label, command));
                }
            }
        }
        Scope::Pane(pane) => {
            for kind in COMMANDS {
                if let Some(command) = context.plan.pane_command(pane, kind)? {
                    let label = match kind {
                        DockspacePresentationCommandKind::Float => "Float Pane",
                        DockspacePresentationCommandKind::MoveToNewWindow => {
                            "Move Pane to New Window"
                        }
                        _ => command_label(kind),
                    };
                    entries.push((label, command));
                }
            }
        }
    }
    entries.retain(|(_, command)| {
        command.unavailable_reason()
            != Some(DockspacePresentationCommandUnavailable::Action(
                DockspaceActionRejection::NativeUnavailable,
            ))
    });
    Ok(entries)
}

const fn command_label(kind: DockspacePresentationCommandKind) -> &'static str {
    match kind {
        DockspacePresentationCommandKind::Float => "Float Layout",
        DockspacePresentationCommandKind::MoveToNewWindow => "Move Layout to New Window",
        DockspacePresentationCommandKind::DockBack => "Dock Back",
        DockspacePresentationCommandKind::DockLeft => "Dock Left",
        DockspacePresentationCommandKind::DockRight => "Dock Right",
        DockspacePresentationCommandKind::DockTop => "Dock Top",
        DockspacePresentationCommandKind::DockBottom => "Dock Bottom",
        DockspacePresentationCommandKind::DockCenter => "Dock Center",
    }
}

const fn unavailable_text(reason: DockspacePresentationCommandUnavailable) -> &'static str {
    match reason {
        DockspacePresentationCommandUnavailable::AlreadyContained => {
            "This root is already in a floating egui window."
        }
        DockspacePresentationCommandUnavailable::AlreadyNative => {
            "This root is already in a native child window."
        }
        DockspacePresentationCommandUnavailable::Action(reason) => action_unavailable_text(reason),
    }
}

const fn action_unavailable_text(reason: DockspaceActionRejection) -> &'static str {
    match reason {
        DockspaceActionRejection::ItemHidden { .. } => "Show the pane before selecting it.",
        DockspaceActionRejection::PolicyDenied => {
            "This command is disabled by the current docking policy."
        }
        DockspaceActionRejection::SurfaceRequired { .. } => {
            "This command would remove a required host surface."
        }
        DockspaceActionRejection::Conflict => {
            "Another docking transition is currently in progress."
        }
        DockspaceActionRejection::IdentityExhausted => {
            "No presentation identity is currently available."
        }
        DockspaceActionRejection::NativeUnavailable => {
            "Native child windows are unavailable for this host."
        }
        DockspaceActionRejection::ItemUnavailable { .. }
        | DockspaceActionRejection::ItemNotContained { .. }
        | DockspaceActionRejection::AnchorUnavailable { .. }
        | DockspaceActionRejection::RootUnavailable { .. }
        | DockspaceActionRejection::DockBackUnavailable { .. }
        | DockspaceActionRejection::MainSurfaceUnavailable { .. }
        | DockspaceActionRejection::SurfaceUnavailable { .. }
        | DockspaceActionRejection::PresentationUnavailable { .. } => {
            "The current layout cannot perform this command."
        }
    }
}
