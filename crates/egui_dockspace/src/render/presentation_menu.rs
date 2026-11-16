//! Scoped group and root commands rendered with egui menu primitives.

use dockspace::model::DockspaceActionRejection;
use dockspace::runtime::{
    ContainedPaintRecord, DockspacePresentationCommandKind,
    DockspacePresentationCommandUnavailable, DockspacePresentationMenuAnchorKind,
    DockspaceRuntimeError, DockspaceTabsGroupCommandKind, PresentationCommandPaintRecord,
    PresentationMenuAnchorPaintRecord, SurfacePaintDisposition, TabBarPaintRecord,
};
use egui::accesskit::{Action, HasPopup, Role};
use egui::{Button, PointerButton, Popup, Sense, StrokeKind, UiBuilder};

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
}

impl Scope<'_> {
    const fn label(self) -> &'static str {
        match self {
            Self::Root(_) => "Layout actions",
            Self::Tabs(_) => "Tab group actions",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Root(_) => "Actions for the entire layout, including all split groups",
            Self::Tabs(_) => "Actions for this tab group; other split groups stay in place",
        }
    }
}

pub(super) fn paint(
    context: &mut RenderContext<'_, '_, '_>,
    anchor: PresentationMenuAnchorPaintRecord<'_>,
    contained: Option<ContainedPaintRecord<'_>>,
    disposition: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    paint_scoped(context, Scope::Root(anchor), contained, disposition)
}

pub(super) fn paint_tabs(
    context: &mut RenderContext<'_, '_, '_>,
    bar: TabBarPaintRecord<'_>,
    disposition: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    paint_scoped(context, Scope::Tabs(bar), None, disposition)
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
            )
            .is_some(),
        ),
        Scope::Tabs(bar) => (
            bar.group_menu_bounds(),
            bar.visual_id(),
            context.plan.tabs_group_menu_operable(bar),
            false,
        ),
    };
    let Some(bounds) = bounds.and_then(egui_rect) else {
        return Ok(());
    };
    let id = context
        .ui
        .make_persistent_id((context.instance_id, scope.label(), visual));
    let popup_id = id.with("popup");
    let ui_enabled = effective_enabled(context.ui, operable);
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
        Popup::open_id(context.ui.ctx(), popup_id);
    }
    if !ui_enabled && Popup::is_id_open(context.ui.ctx(), popup_id) {
        Popup::close_id(context.ui.ctx(), popup_id);
    }
    let menu_open = Popup::is_id_open(context.ui.ctx(), popup_id);
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

    let commands = commands(context, scope)?;
    let mut entries = Vec::with_capacity(commands.len() + 1);
    let mut close = match scope {
        Scope::Root(anchor) => context.plan.prepare_root_close(anchor.root()),
        Scope::Tabs(bar) => context.plan.prepare_tabs_close(bar),
    };
    let popup = popup::show(
        context.ui,
        context.instance_id,
        context.plan.surface(),
        context.plan.workspace_stamp(),
        popup_id,
        &response,
        touch_requested,
        context.style,
        |ui| {
            ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                node.set_role(Role::Menu);
                node.set_label(scope.label());
            });
            ui.weak(match scope {
                Scope::Root(_) => "Entire layout",
                Scope::Tabs(_) => "Tab group",
            });
            ui.separator();
            for (label, command) in commands {
                let enabled = ui_enabled && command.is_ready();
                let response = ui
                    .scope_builder(
                        UiBuilder::new().scope_id(popup_id.with(("command", command.kind()))),
                        |ui| ui.add_enabled(enabled, Button::new(label)),
                    )
                    .inner;
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_role(Role::MenuItem);
                    node.set_label(label);
                    node.clear_actions();
                    if enabled {
                        node.add_action(Action::Click);
                        node.add_action(Action::Focus);
                    } else {
                        node.set_disabled();
                        if let Some(reason) = command.unavailable_reason() {
                            node.set_description(unavailable_text(reason));
                        }
                    }
                });
                let response = if let Some(reason) = command.unavailable_reason() {
                    response.on_disabled_hover_text(unavailable_text(reason))
                } else {
                    response
                };
                if enabled {
                    entries.push((response, Some(command)));
                }
            }
            ui.separator();
            let close_label = match scope {
                Scope::Root(_) => "Close Layout",
                Scope::Tabs(_) => "Close Tab Group",
            };
            let response = ui
                .scope_builder(UiBuilder::new().scope_id(popup_id.with("close")), |ui| {
                    ui.add_enabled(ui_enabled && close.is_some(), Button::new(close_label))
                })
                .inner;
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_role(Role::MenuItem);
                node.set_label(close_label);
                node.clear_actions();
                if response.enabled() {
                    node.add_action(Action::Click);
                    node.add_action(Action::Focus);
                } else {
                    node.set_disabled();
                }
            });
            if response.enabled() {
                entries.push((response, None));
            }
            entries.iter().map(|(response, _)| response.id).collect()
        },
    );
    let keyboard_activation = match popup.and_then(|popup| popup.inner) {
        Some(keyboard::Outcome::Activate(target)) => Some(target),
        Some(keyboard::Outcome::Dismiss) => return Ok(()),
        Some(keyboard::Outcome::Focus(_)) | None => None,
    };
    let activated = entries.into_iter().find_map(|(response, command)| {
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
        let action = match command {
            Some(command) => context.plan.prepare_presentation_command(command),
            None => close.take(),
        }?;
        Some((origin, action))
    });
    if let Some((origin, action)) = activated {
        Popup::close_id(context.ui.ctx(), popup_id);
        context.push_local_action(origin, action);
    }
    Ok(())
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
        DockspacePresentationMenuAnchorKind::TabBar => (
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
