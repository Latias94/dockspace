//! Close-button painting over adapter-owned input and core close requests.

use dockspace::runtime::TabPaintRecord;
use egui::accesskit::{Action, Role};
use egui::{PointerButton, Sense, UiBuilder, vec2};

use crate::{TabClose, TabCloseVisibility, TabVisuals};

use super::RenderContext;
use super::actions::{
    effective_enabled, pointer_or_accesskit_activation_origin, unambiguous_pointer_click,
};
use super::geometry::{accesskit_bounds, egui_rect};
use super::measurement::TabPaintResource;

pub(super) fn capture_body_click(
    context: &mut RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    response: &egui::Response,
) {
    if let Some(origin) = middle_click_origin(context, tab, response)
        && let Some(action) = context.plan.prepare_tab_body_close(tab.key())
    {
        context.push_local_action(origin, action);
    }
}

fn middle_click_origin(
    context: &RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    response: &egui::Response,
) -> Option<egui::Id> {
    // The core journal handles primary-button gestures. Auxiliary clicks keep
    // egui's completed widget routing, including application child controls.
    (context.input.middle_click_closes_tab
        && effective_enabled(context.ui, response.enabled())
        && unambiguous_pointer_click(context.ui, response, PointerButton::Middle))
    .then(|| {
        context
            .ui
            .make_persistent_id((context.instance_id, "middle-tab-close", tab.key()))
    })
}

pub(super) fn paint(
    context: &mut RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    resource: &TabPaintResource,
    tab_response: &egui::Response,
    tab_visuals: Option<TabVisuals>,
) {
    let Some(rect) = tab.close_bounds().and_then(egui_rect) else {
        return;
    };
    let id = id(context, tab);
    let receiver = context.plan.receiver_for_tab_close(tab);
    let enabled = effective_enabled(context.ui, receiver.is_some());
    let close_label = if tab.key().is_layout() {
        format!("Close Layout Tab {}", resource.title)
    } else {
        format!("Close {}", resource.title)
    };
    let response = context
        .interact_receiver(
            rect,
            id,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
            enabled.then_some(receiver).flatten(),
        )
        .on_hover_text(close_label.clone());
    let hovered = tab_response.hovered() || response.hovered();
    let visible = tab_response.has_focus()
        || response.has_focus()
        || match context.style.tab_close_visibility {
            TabCloseVisibility::Always => true,
            TabCloseVisibility::SelectedOrHovered => tab.selected() || hovered,
            TabCloseVisibility::Hovered => hovered,
        };
    if visible && let Some(tab_visuals) = tab_visuals {
        let mut stroke =
            context.interaction_stroke(&response, context.style.visuals.tab_text_color);
        stroke.color = tab_visuals.close_color.unwrap_or(stroke.color);
        let size = context
            .style
            .tab_close_icon_size
            .min(rect.width().min(rect.height()));
        let close = TabClose {
            key: tab.key(),
            rect,
            icon_rect: egui::Rect::from_center_size(rect.center(), vec2(size, size)),
            response: &response,
            selected: tab.selected(),
            enabled,
            stroke,
        };
        let mut child = context
            .ui
            .new_child(UiBuilder::new().id_salt((id, "paint")).max_rect(rect));
        child.set_clip_rect(context.ui.clip_rect().intersect(rect));
        if !enabled && child.is_enabled() {
            child.disable();
        }
        if resource.missing {
            close.paint(&child);
        } else {
            context.panes.tab_close_ui(&child, &close);
        }
    }
    context.ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(Role::Button);
        node.set_bounds(accesskit_bounds(rect));
        node.set_label(close_label);
        if tab.key().is_layout() {
            node.set_description("Closes this layout tab and all of its content together");
        }
        if enabled {
            node.add_action(Action::Click);
        } else {
            node.set_disabled();
        }
    });
    if enabled
        && let Some(origin) =
            pointer_or_accesskit_activation_origin(context.ui, &response, context.pointer_authority)
                .or_else(|| middle_click_origin(context, tab, &response))
        && let Some(action) = context.plan.prepare_tab_close(tab.key())
    {
        context.push_local_action(origin, action);
    }
}

pub(super) fn id(context: &RenderContext<'_, '_, '_>, tab: TabPaintRecord<'_>) -> egui::Id {
    context
        .ui
        .make_persistent_id((context.instance_id, tab.close_input_id()))
}
