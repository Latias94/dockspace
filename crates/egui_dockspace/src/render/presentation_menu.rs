//! Exact tab, group and root commands rendered with shared egui menu primitives.

use dockspace::model::{DockspaceActionRejection, RootId, TabGroupDockMode, TabLabel};
use dockspace::runtime::{
    ContainedPaintRecord, DockspacePresentationCommandKind,
    DockspacePresentationCommandUnavailable, DockspacePresentationMenuAnchorKind,
    DockspaceRuntimeError, DockspaceTabsGroupCommandKind, DockspaceVisualId,
    DockspaceWorkspaceStamp, PanePaintRecord, PresentationCommandPaintRecord,
    PresentationMenuAnchorPaintRecord, SurfacePaintDisposition, SurfacePaintProgramOperation,
    SurfacePaintProgramStep, TabBarPaintRecord, TabPaintRecord,
};
use egui::accesskit::{Action, HasPopup, Role};
use egui::{Button, Key, PointerButton, Popup, Response, Sense, StrokeKind, UiBuilder};

use crate::touch_menu::take_touch_context_menu;

use super::RenderContext;
use super::actions::{effective_enabled, unambiguous_pointer_click};
use super::geometry::{accesskit_bounds, egui_rect};

mod keyboard;
mod popup;
pub(super) use popup::invalidate_stale;
pub(crate) use popup::prepare_layer;

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

#[derive(Clone, Copy)]
enum Scope<'plan> {
    Root(PresentationMenuAnchorPaintRecord<'plan>),
    Tabs(TabBarPaintRecord<'plan>),
    Tab(TabPaintRecord<'plan>),
    Pane(PanePaintRecord<'plan>),
}

#[derive(Clone, Copy)]
struct MenuPage {
    workspace: DockspaceWorkspaceStamp,
    kind: MenuPageKind,
    focus_first: bool,
}

#[derive(Clone, Copy)]
enum MenuPageKind {
    GroupTargets(TabGroupDockMode),
    ActivePanes,
    Pane(DockspaceVisualId),
}

struct GroupDestination {
    visual: DockspaceVisualId,
    label: String,
    command: PresentationCommandPaintRecord,
}

enum MenuAction {
    Command(PresentationCommandPaintRecord),
    Close,
    OpenPage(MenuPageKind),
    Back,
}

impl Scope<'_> {
    const fn label(self) -> &'static str {
        match self {
            Self::Root(_) => "Layout actions",
            Self::Tabs(_) => "Tab group actions",
            Self::Tab(tab) if tab.key().is_layout() => "Layout tab actions",
            Self::Tab(_) => "Tab actions",
            Self::Pane(_) => "Pane actions",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Root(_) => "Actions for the entire layout, or choose one of its active panes",
            Self::Tabs(_) => "Actions for this tab group; other split groups stay in place",
            Self::Tab(tab) if tab.key().is_layout() => {
                "Actions for this layout tab and its complete nested layout"
            }
            Self::Tab(_) => "Actions for this pane tab",
            Self::Pane(_) => "Actions for this application pane only",
        }
    }

    fn visual(self) -> dockspace::runtime::DockspaceVisualId {
        match self {
            Self::Root(anchor) => anchor.visual_id(),
            Self::Tabs(bar) => bar.visual_id(),
            Self::Tab(tab) => tab.visual_id(),
            Self::Pane(pane) => pane.visual_id(),
        }
    }
}

pub(super) fn paint(
    context: &mut RenderContext<'_, '_, '_>,
    anchor: PresentationMenuAnchorPaintRecord<'_>,
    contained: Option<ContainedPaintRecord<'_>>,
    disposition: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    if !disposition.is_source_gap()
        && let Some(reserved) = anchor.reserved_bounds().and_then(egui_rect)
    {
        context
            .ui
            .painter()
            .rect_filled(reserved, 0.0, context.visuals.tab_bar_fill);
        crate::tab::paint_bar_separator(
            context.ui.painter(),
            reserved,
            context.visuals.tab_bar_stroke,
        );
    }
    paint_scoped(context, Scope::Root(anchor), contained, disposition)
}

pub(super) fn paint_tabs(
    context: &mut RenderContext<'_, '_, '_>,
    bar: TabBarPaintRecord<'_>,
    disposition: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    paint_scoped(context, Scope::Tabs(bar), None, disposition)
}

pub(super) fn paint_tab(
    context: &mut RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    response: &Response,
    disposition: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    let popup_id = response.id.with("popup");
    let enabled = effective_enabled(context.ui, tab.operable() && response.enabled())
        && !disposition.is_source_gap()
        && context.plan.prepare_gesture_cancel().is_none()
        && popup::source_available(context.ui, context.instance_id, popup_id);
    if !enabled {
        Popup::close_id(context.ui.ctx(), popup_id);
        return Ok(());
    }
    let touch = take_touch_context_menu(
        context.ui,
        context.instance_id,
        context.plan.surface(),
        tab.root(),
        Some(tab.key()),
    )
    .is_some();
    let accessible = context
        .ui
        .input(|input| input.has_accesskit_action_request(response.id, Action::ShowContextMenu));
    let keyboard = response.has_focus()
        && context
            .ui
            .input(|input| input.focused && !input.pointer.any_down())
        && context.ui.input_mut(|input| {
            let event = input.events.iter().position(|event| {
                matches!(event,
                egui::Event::Key { key, modifiers, pressed: true, .. }
                    if (*key == Key::Menu && modifiers.is_none())
                        || (*key == Key::F10 && modifiers.shift_only()))
            });
            if let Some(event) = event {
                input.events.remove(event);
                true
            } else {
                false
            }
        });
    if touch
        || accessible
        || keyboard
        || unambiguous_pointer_click(context.ui, response, PointerButton::Secondary)
    {
        response.request_focus();
        Popup::open_id(context.ui.ctx(), popup_id);
    }
    let menu_open = Popup::is_id_open(context.ui.ctx(), popup_id);
    context
        .ui
        .ctx()
        .accesskit_node_builder(response.id, |node| {
            node.set_has_popup(HasPopup::Menu);
            node.set_expanded(menu_open);
            node.set_description(format!(
                "{}. Open the tab menu with Shift+F10.",
                Scope::Tab(tab).description()
            ));
            node.add_action(Action::ShowContextMenu);
        });
    if menu_open {
        paint_menu(context, Scope::Tab(tab), response, false)?;
    }
    Ok(())
}

fn paint_scoped(
    context: &mut RenderContext<'_, '_, '_>,
    scope: Scope<'_>,
    contained: Option<ContainedPaintRecord<'_>>,
    disposition: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    let (bounds, visual, operable, touch_requested) = match scope {
        Scope::Root(anchor) => (
            Some(anchor.bounds()),
            anchor.visual_id(),
            anchor.operable(),
            take_touch_context_menu(
                context.ui,
                context.instance_id,
                context.plan.surface(),
                anchor.root(),
                None,
            )
            .is_some(),
        ),
        Scope::Tabs(bar) => (
            bar.group_menu_bounds(),
            bar.visual_id(),
            context.plan.tabs_group_menu_operable(bar),
            false,
        ),
        Scope::Tab(_) | Scope::Pane(_) => return Ok(()),
    };
    let Some(bounds) = bounds.and_then(egui_rect) else {
        return Ok(());
    };
    let id = context
        .ui
        .make_persistent_id((context.instance_id, scope.label(), visual));
    let popup_id = id.with("popup");
    let ui_enabled = effective_enabled(context.ui, operable)
        && !disposition.is_source_gap()
        && context.plan.prepare_gesture_cancel().is_none()
        && popup::source_available(context.ui, context.instance_id, popup_id);
    let response = context
        .ui
        .interact(
            bounds,
            id,
            if ui_enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        )
        .on_hover_text(scope.description());
    if ui_enabled && touch_requested {
        response.request_focus();
        Popup::open_id(context.ui.ctx(), popup_id);
    }
    if ui_enabled && response.clicked() {
        response.request_focus();
    }
    if !ui_enabled && Popup::is_id_open(context.ui.ctx(), popup_id) {
        Popup::close_id(context.ui.ctx(), popup_id);
    }
    let menu_open = Popup::is_id_open(context.ui.ctx(), popup_id);
    if !menu_open {
        context.ui.ctx().data_mut(|data| {
            data.remove::<MenuPage>(popup_id.with("page"));
        });
    }
    if !disposition.is_source_gap() {
        match scope {
            Scope::Root(anchor) => paint_anchor_icon(context, anchor, contained, bounds, &response),
            Scope::Tabs(_) => {
                if RenderContext::response_emphasized(&response) {
                    context.ui.painter().rect_filled(
                        bounds,
                        2.0,
                        context.interaction_fill(&response),
                    );
                }
                let color = context
                    .interaction_stroke(&response, context.style.visuals.tab_text_color)
                    .color;
                super::chrome_icons::group_actions(context.ui, bounds, color);
            }
            Scope::Tab(_) | Scope::Pane(_) => {}
        }
        if ui_enabled && response.has_focus() {
            context.ui.painter_at(bounds).rect_stroke(
                bounds,
                2.0,
                context.visuals.tab_focus_stroke,
                StrokeKind::Inside,
            );
        }
    }
    context.ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(Role::Button);
        node.set_bounds(accesskit_bounds(bounds));
        node.set_label(scope.label());
        node.set_description(scope.description());
        node.set_has_popup(HasPopup::Menu);
        node.set_expanded(menu_open);
        if ui_enabled {
            node.add_action(Action::Click);
        } else {
            node.set_disabled();
            node.clear_actions();
        }
    });
    if !menu_open && (!ui_enabled || !response.clicked()) {
        return Ok(());
    }
    paint_menu(context, scope, &response, !touch_requested)
}

fn paint_menu(
    context: &mut RenderContext<'_, '_, '_>,
    scope: Scope<'_>,
    response: &Response,
    toggle_on_click: bool,
) -> Result<(), DockspaceRuntimeError> {
    let popup_id = response.id.with("popup");
    let page_id = popup_id.with("page");
    let ui_enabled = effective_enabled(context.ui, response.enabled());
    let workspace = context.plan.workspace_stamp();
    let page = context
        .ui
        .ctx()
        .data(|data| data.get_temp::<MenuPage>(page_id))
        .filter(|page| {
            page.workspace == workspace
                && matches!(
                    (scope, page.kind),
                    (Scope::Tabs(_), MenuPageKind::GroupTargets(_))
                        | (
                            Scope::Root(_),
                            MenuPageKind::ActivePanes | MenuPageKind::Pane(_)
                        )
                )
        });
    let (active_panes, root_is_pane) = if let Scope::Root(anchor) = scope {
        active_panes(context, anchor.root())?
    } else {
        (Vec::new(), false)
    };
    let menu_scope = if let Some(MenuPage {
        kind: MenuPageKind::Pane(visual),
        ..
    }) = page
    {
        let Some(pane) = active_panes.iter().find(|pane| pane.visual_id() == visual) else {
            Popup::close_id(context.ui.ctx(), popup_id);
            context
                .ui
                .ctx()
                .data_mut(|data| data.remove::<MenuPage>(page_id));
            return Ok(());
        };
        Scope::Pane(*pane)
    } else {
        scope
    };
    let heading = match menu_scope {
        Scope::Root(_) => "Entire layout".to_owned(),
        Scope::Tabs(bar) => context.plan.tabs_group_label(bar).map_or_else(
            || "Tab group".to_owned(),
            |label| format!("Tab group: {}", label_text(context, label)),
        ),
        Scope::Tab(tab) => {
            let Some(resource) = context.resources.tab(tab.key()) else {
                return Ok(());
            };
            format!(
                "{}: {}",
                if tab.key().is_layout() {
                    "Layout tab"
                } else {
                    "Pane tab"
                },
                resource.title
            )
        }
        Scope::Pane(pane) => format!("Pane: {}", label_text(context, TabLabel::Item(pane.item()))),
    };
    let commands = if page.is_none_or(|page| matches!(page.kind, MenuPageKind::Pane(_))) {
        commands(context, menu_scope)?
    } else {
        Vec::new()
    };
    let mut group_targets = Vec::new();
    if let Scope::Tabs(bar) = scope {
        for mode in [
            TabGroupDockMode::MergeTabs,
            TabGroupDockMode::PreserveLayout,
        ] {
            if page.is_none_or(
                |page| matches!(page.kind, MenuPageKind::GroupTargets(current) if current == mode),
            ) {
                group_targets.push((mode, group_destinations(context, bar, mode)?));
            }
        }
    }
    let mut entries = Vec::with_capacity(commands.len() + active_panes.len() + 4);
    let mut focused_entry = false;
    let mut close = match menu_scope {
        Scope::Root(anchor) => context.plan.prepare_root_close(anchor.root()),
        Scope::Tabs(bar) => context.plan.prepare_tabs_close(bar),
        Scope::Tab(tab) => context.plan.prepare_tab_body_close(tab.key()),
        Scope::Pane(pane) => context.plan.prepare_pane_close(pane),
    };
    let popup = popup::show(
        context.ui,
        context.instance_id,
        context.plan.surface(),
        workspace,
        scope.visual(),
        popup_id,
        response,
        toggle_on_click,
        context.style,
        |ui| {
            ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                node.set_role(Role::Menu);
                node.set_label(match page.map(|page| page.kind) {
                    Some(MenuPageKind::GroupTargets(mode)) => group_mode_label(mode),
                    Some(MenuPageKind::ActivePanes) => "Active panes",
                    _ => menu_scope.label(),
                });
            });
            ui.weak(heading);
            ui.separator();
            for (label, command) in commands {
                let response = command_entry(
                    ui,
                    popup_id.with(("command", menu_scope.visual(), command.kind())),
                    label,
                    &command,
                    ui_enabled,
                );
                if ui_enabled && command.is_ready() {
                    entries.push((response, MenuAction::Command(command)));
                }
            }
            for (mode, destinations) in group_targets {
                if page.is_some() {
                    ui.weak(group_mode_label(mode));
                    for destination in destinations {
                        let response = command_entry(
                            ui,
                            popup_id.with(("group-center", mode, destination.visual)),
                            &destination.label,
                            &destination.command,
                            ui_enabled,
                        );
                        if ui_enabled && destination.command.is_ready() {
                            entries.push((response, MenuAction::Command(destination.command)));
                        }
                    }
                } else {
                    let enabled =
                        ui_enabled && destinations.iter().any(|entry| entry.command.is_ready());
                    let response = page_entry(
                        ui,
                        popup_id.with(("group-mode", mode)),
                        group_mode_label(mode),
                        group_mode_description(mode),
                        enabled,
                    )
                    .on_disabled_hover_text("No other eligible visible tab group is available.");
                    if enabled {
                        entries.push((
                            response,
                            MenuAction::OpenPage(MenuPageKind::GroupTargets(mode)),
                        ));
                    }
                }
            }
            if matches!(scope, Scope::Root(_)) && !root_is_pane {
                if page.is_none() {
                    let enabled = ui_enabled && !active_panes.is_empty();
                    let response = page_entry(
                        ui,
                        popup_id.with("active-panes"),
                        "Active Panes…",
                        "Choose a visible pane to move or close only that application's content.",
                        enabled,
                    );
                    if enabled {
                        entries.push((response, MenuAction::OpenPage(MenuPageKind::ActivePanes)));
                    }
                } else if matches!(page.map(|page| page.kind), Some(MenuPageKind::ActivePanes)) {
                    ui.weak("Choose one active pane");
                    for pane in &active_panes {
                        let title = label_text(context, TabLabel::Item(pane.item()));
                        let response = page_entry(
                            ui,
                            popup_id.with(("pane", pane.visual_id())),
                            &title,
                            "Open actions for this application pane only.",
                            ui_enabled,
                        );
                        if response.enabled() {
                            entries.push((
                                response,
                                MenuAction::OpenPage(MenuPageKind::Pane(pane.visual_id())),
                            ));
                        }
                    }
                }
            }
            ui.separator();
            if page.is_none_or(|page| matches!(page.kind, MenuPageKind::Pane(_))) {
                let label = match menu_scope {
                    Scope::Root(_) => "Close Layout",
                    Scope::Tabs(_) => "Close Tab Group",
                    Scope::Tab(tab) if tab.key().is_layout() => "Close Layout Tab",
                    Scope::Tab(_) => "Close Tab",
                    Scope::Pane(_) => "Close Pane",
                };
                let response = plain_entry(
                    ui,
                    popup_id.with((menu_scope.visual(), label)),
                    label,
                    menu_scope.description(),
                    ui_enabled && close.is_some(),
                );
                if response.enabled() {
                    entries.push((response, MenuAction::Close));
                }
            }
            if let Some(page) = page {
                let label = match page.kind {
                    MenuPageKind::GroupTargets(_) => "Back to Tab Group Actions",
                    MenuPageKind::ActivePanes => "Back to Layout Actions",
                    MenuPageKind::Pane(_) => "Back to Active Panes",
                };
                let response = plain_entry(
                    ui,
                    popup_id.with(label),
                    label,
                    "Return to the previous menu page.",
                    ui_enabled,
                );
                if response.enabled() {
                    entries.push((response, MenuAction::Back));
                }
            }
            if page.is_some_and(|page| page.focus_first)
                && !ui.is_sizing_pass()
                && ui.is_enabled()
                && let Some((response, _)) = entries.first()
            {
                response.request_focus();
                focused_entry = true;
            }
            (
                entries.iter().map(|(response, _)| response.id).collect(),
                entries
                    .iter()
                    .filter_map(|(response, action)| {
                        matches!(action, MenuAction::OpenPage(_)).then_some(response.id)
                    })
                    .collect(),
            )
        },
    );
    if focused_entry && let Some(mut page) = page {
        page.focus_first = false;
        context
            .ui
            .ctx()
            .data_mut(|data| data.insert_temp(page_id, page));
    }
    let mut go_back = false;
    let keyboard_activation = match popup.and_then(|popup| popup.inner) {
        Some(keyboard::Outcome::Activate(target)) => Some(target),
        Some(keyboard::Outcome::Dismiss) => {
            context
                .ui
                .ctx()
                .data_mut(|data| data.remove::<MenuPage>(page_id));
            return Ok(());
        }
        Some(keyboard::Outcome::Focus(target)) if page.is_some() && target == response.id => {
            go_back = true;
            None
        }
        Some(keyboard::Outcome::Focus(_)) | None => None,
    };
    let activated = if go_back {
        Some((response.id.with("back"), MenuAction::Back))
    } else {
        entries.into_iter().find_map(|(response, action)| {
            let origin = if keyboard_activation == Some(response.id) {
                Some(response.id.with("keyboard-activate"))
            } else if keyboard_activation.is_none()
                && response.clicked()
                && context
                    .ui
                    .input(|input| input.has_accesskit_action_request(response.id, Action::Click))
            {
                Some(response.id.with("accesskit-click"))
            } else if keyboard_activation.is_none()
                && unambiguous_pointer_click(context.ui, &response, PointerButton::Primary)
            {
                Some(response.id.with("pointer-click"))
            } else {
                None
            }?;
            Some((origin, action))
        })
    };
    if let Some((origin, action)) = activated {
        match action {
            MenuAction::Command(command) => {
                if let Some(action) = context.plan.prepare_presentation_command(command) {
                    Popup::close_id(context.ui.ctx(), popup_id);
                    context.push_local_action(origin, action);
                }
            }
            MenuAction::Close => {
                if let Some(action) = close.take() {
                    Popup::close_id(context.ui.ctx(), popup_id);
                    context.push_local_action(origin, action);
                }
            }
            MenuAction::OpenPage(kind) => {
                context.ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        page_id,
                        MenuPage {
                            workspace,
                            kind,
                            focus_first: true,
                        },
                    );
                });
                Popup::open_id(context.ui.ctx(), popup_id);
                response.request_focus();
                context.ui.ctx().request_repaint();
            }
            MenuAction::Back => {
                context.ui.ctx().data_mut(|data| {
                    if matches!(page.map(|page| page.kind), Some(MenuPageKind::Pane(_))) {
                        data.insert_temp(
                            page_id,
                            MenuPage {
                                workspace,
                                kind: MenuPageKind::ActivePanes,
                                focus_first: true,
                            },
                        );
                    } else {
                        data.remove::<MenuPage>(page_id);
                    }
                });
                Popup::open_id(context.ui.ctx(), popup_id);
                response.request_focus();
                context.ui.ctx().request_repaint();
            }
        }
    }
    if !Popup::is_id_open(context.ui.ctx(), popup_id) {
        context
            .ui
            .ctx()
            .data_mut(|data| data.remove::<MenuPage>(page_id));
    }
    Ok(())
}

fn plain_entry(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    description: &str,
    enabled: bool,
) -> Response {
    let response = ui
        .scope_builder(UiBuilder::new().scope_id(id), |ui| {
            ui.add_enabled(enabled, Button::new(label))
        })
        .inner;
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::MenuItem);
        node.set_label(label);
        node.set_description(description);
        node.clear_actions();
        if response.enabled() {
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
        } else {
            node.set_disabled();
        }
    });
    response
}

fn page_entry(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    description: &str,
    enabled: bool,
) -> Response {
    let response = ui
        .scope_builder(UiBuilder::new().scope_id(id), |ui| {
            ui.add_enabled(enabled, Button::new(label).right_text("›"))
        })
        .inner;
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::MenuItem);
        node.set_label(label);
        node.set_description(description);
        node.set_has_popup(HasPopup::Menu);
        node.set_expanded(false);
        node.clear_actions();
        if response.enabled() {
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
        } else {
            node.set_disabled();
        }
    });
    response.on_hover_text(description)
}

fn command_entry(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    command: &PresentationCommandPaintRecord,
    ui_enabled: bool,
) -> Response {
    let enabled = ui_enabled && command.is_ready();
    let response = ui
        .scope_builder(UiBuilder::new().scope_id(id), |ui| {
            ui.add_enabled(enabled, Button::new(label).wrap())
        })
        .inner;
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::MenuItem);
        node.set_label(label);
        node.clear_actions();
        if response.enabled() {
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
        } else {
            node.set_disabled();
            if let Some(reason) = command.unavailable_reason() {
                node.set_description(unavailable_text(reason));
            }
        }
    });
    if let Some(reason) = command.unavailable_reason() {
        response.on_disabled_hover_text(unavailable_text(reason))
    } else {
        response
    }
}

fn label_text(context: &RenderContext<'_, '_, '_>, label: TabLabel<'_>) -> String {
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
        TabLabel::Layout { title } => title.to_owned(),
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
            || label_text(context, member.label()),
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
    format!("{}\n{members} · {selection}", label_text(context, label))
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

fn paint_anchor_icon(
    context: &RenderContext<'_, '_, '_>,
    anchor: PresentationMenuAnchorPaintRecord<'_>,
    contained: Option<ContainedPaintRecord<'_>>,
    bounds: egui::Rect,
    response: &egui::Response,
) {
    let (emphasized_fill, idle_color) = match anchor.kind() {
        DockspacePresentationMenuAnchorKind::TabBar
        | DockspacePresentationMenuAnchorKind::RootControls => (
            context.interaction_fill(response),
            context.style.visuals.tab_text_color,
        ),
        DockspacePresentationMenuAnchorKind::ContainedTitle => {
            let title_text_color = contained
                .filter(|contained| contained.root() == anchor.root())
                .map(|contained| {
                    context
                        .style
                        .resolved_contained_window(context.ui.style(), contained.is_frontmost())
                        .title_text_color
                })
                .unwrap_or(context.ui.visuals().widgets.noninteractive.fg_stroke.color);
            (
                context.ui.style().interact(response).weak_bg_fill,
                Some(title_text_color),
            )
        }
    };
    if RenderContext::response_emphasized(response) {
        context
            .ui
            .painter()
            .rect_filled(bounds, 2.0, emphasized_fill);
    }
    let color = context.interaction_stroke(response, idle_color).color;
    super::chrome_icons::layout_actions(context.ui, bounds, color);
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
