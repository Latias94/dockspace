//! Exact tab, group and root commands rendered with shared egui menu primitives.

use dockspace::model::{RootId, TabGroupDockMode};
use dockspace::runtime::{
    ContainedPaintRecord, DockspacePresentationMenuAnchorKind, DockspaceRuntimeError,
    DockspaceVisualId, DockspaceWorkspaceStamp, PanePaintRecord, PreparedSurfaceAction,
    PresentationCommandPaintRecord, PresentationMenuAnchorPaintRecord, SurfacePaintDisposition,
    TabBarPaintRecord, TabPaintRecord,
};
use egui::accesskit::{Action, HasPopup, Role};
use egui::{Color32, CornerRadius, Id, Key, PointerButton, Popup, Response, Sense};

use crate::touch_menu::take_touch_context_menu;
use crate::{
    DockMenuAction, DockMenuButton, DockMenuButtonKind, DockMenuContext, DockMenuTarget, TabBarId,
};

use super::RenderContext;
use super::actions::{effective_enabled, focus_uninterrupted, unambiguous_pointer_click};
use super::geometry::{accesskit_bounds, egui_rect};

mod entries;
mod keyboard;
mod popup;
pub(super) use popup::invalidate_stale;
pub(crate) use popup::prepare_layer;

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

#[derive(Clone, Copy, Debug, Hash)]
enum MenuPageKind {
    GroupTargets(TabGroupDockMode),
    ActivePanes,
    Pane(DockspaceVisualId),
}

enum MenuAction {
    Command(PresentationCommandPaintRecord),
    Close(PreparedSurfaceAction),
    OpenPage(MenuPageKind),
    Back,
    Custom(Id),
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

    fn visual(self) -> DockspaceVisualId {
        match self {
            Self::Root(anchor) => anchor.visual_id(),
            Self::Tabs(bar) => bar.visual_id(),
            Self::Tab(tab) => tab.visual_id(),
            Self::Pane(pane) => pane.visual_id(),
        }
    }

    fn root(self) -> RootId {
        match self {
            Self::Root(anchor) => anchor.root(),
            Self::Tabs(bar) => bar.root(),
            Self::Tab(tab) => tab.root(),
            Self::Pane(pane) => pane.root(),
        }
    }

    fn target(self) -> DockMenuTarget {
        match self {
            Self::Root(_) => DockMenuTarget::Root,
            Self::Tabs(bar) => DockMenuTarget::TabGroup(TabBarId(bar.visual_id())),
            Self::Tab(tab) => DockMenuTarget::Tab(tab.key()),
            Self::Pane(pane) => DockMenuTarget::Pane(pane.item()),
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
        && focus_uninterrupted(context.ui)
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
    // Keep the button registered while unfocused: the first new press can
    // focus the host and hit its previously presented widget in one batch.
    let can_activate = ui_enabled && focus_uninterrupted(context.ui);
    if can_activate && touch_requested {
        response.request_focus();
        Popup::open_id(context.ui.ctx(), popup_id);
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
    if menu_open || (can_activate && response.clicked()) {
        paint_menu(context, scope, &response, !touch_requested)?;
    }
    let menu_open = Popup::is_id_open(context.ui.ctx(), popup_id);
    if !disposition.is_source_gap() {
        let (kind, emphasized_fill, icon_color) = match scope {
            Scope::Root(anchor) => {
                let (fill, color) = anchor_colors(context, anchor, contained, &response);
                (DockMenuButtonKind::Layout, fill, color)
            }
            Scope::Tabs(_) => (
                DockMenuButtonKind::TabGroup,
                context.interaction_fill(&response),
                context
                    .interaction_stroke(&response, context.style.visuals.tab_text_color)
                    .color,
            ),
            Scope::Tab(_) | Scope::Pane(_) => return Ok(()),
        };
        context.paint_menu_button(&DockMenuButton {
            kind,
            context: DockMenuContext {
                surface: context.plan.surface(),
                root: scope.root(),
                target: scope.target(),
            },
            rect: bounds,
            response: &response,
            enabled: ui_enabled,
            open: menu_open,
            fill: if RenderContext::response_emphasized(&response) {
                emphasized_fill
            } else {
                Color32::TRANSPARENT
            },
            icon_color,
            corner_radius: CornerRadius::same(2),
        });
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
    Ok(())
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
    let Some(mut prepared) = entries::prepare(context, scope, page.map(|page| page.kind))? else {
        Popup::close_id(context.ui.ctx(), popup_id);
        context
            .ui
            .ctx()
            .data_mut(|data| data.remove::<MenuPage>(page_id));
        return Ok(());
    };
    let menu_context = prepared.menu.context();
    let mut entries = Vec::with_capacity(prepared.menu.items.len());
    let mut page_focus_handled = false;
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
            context.panes.customize_menu(&mut prepared.menu);
            prepared.paint(
                ui,
                popup_id.with(page.map(|page| page.kind)),
                ui_enabled,
                &mut entries,
            );
            if page.is_some_and(|page| page.focus_first) && !ui.is_sizing_pass() && ui.is_enabled()
            {
                // Page entry is deferred to its first visible pass. A newer
                // application or accessibility focus choice supersedes it.
                page_focus_handled = true;
                if ui.input(|input| input.focused)
                    && ui.memory(|memory| {
                        memory.focused() == Some(response.id)
                            || (memory.focused().is_none()
                                && memory.had_focus_last_frame(response.id))
                    })
                    && let Some((first, _)) = entries.first()
                {
                    first.request_focus();
                }
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
    if page_focus_handled && let Some(mut page) = page {
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
            MenuAction::Close(action) => {
                Popup::close_id(context.ui.ctx(), popup_id);
                context.push_local_action(origin, action);
            }
            MenuAction::Custom(id) => {
                Popup::close_id(context.ui.ctx(), popup_id);
                response.request_focus();
                context.menu_action.get_or_insert(DockMenuAction {
                    context: menu_context,
                    id,
                });
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

fn anchor_colors(
    context: &RenderContext<'_, '_, '_>,
    anchor: PresentationMenuAnchorPaintRecord<'_>,
    contained: Option<ContainedPaintRecord<'_>>,
    response: &egui::Response,
) -> (Color32, Color32) {
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
    let color = context.interaction_stroke(response, idle_color).color;
    (emphasized_fill, color)
}
