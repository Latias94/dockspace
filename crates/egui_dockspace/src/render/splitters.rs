//! Splitter rendering and semantic keyboard/accessibility actions.

use dockspace::model::DockspaceAxis;
use dockspace::runtime::{
    PreparedSurfaceAction, SplitterJunctionPaintRecord, SurfacePaintDisposition, SurfacePaintPlan,
    SurfaceSplitterAdjustment,
};
use egui::accesskit::{Action, Orientation, Role};
use egui::{Color32, CursorIcon, Event, EventFilter, Key, PointerButton, Sense, UiBuilder};

use crate::{Splitter, SplitterDoubleClick, SplitterKind};

use super::RenderContext;
use super::actions::{effective_enabled, unambiguous_pointer_click};
use super::geometry::{accesskit_bounds, egui_rect, inset_ui_rect};
pub(crate) fn paint_splitter(
    context: &mut RenderContext<'_, '_, '_>,
    splitter: dockspace::runtime::SplitterPaintRecord<'_>,
    paint: SurfacePaintDisposition,
) {
    if !context.plan.partition_available(splitter.root()) {
        return;
    }
    let Some(draw) = egui_rect(splitter.draw_bounds()) else {
        return;
    };
    let Some(hit) = egui_rect(splitter.hit_bounds()) else {
        return;
    };
    let id = context
        .ui
        .make_persistent_id((context.instance_id, splitter.input_id()));
    let enabled = effective_enabled(context.ui, splitter.operable());
    let drag_enabled = enabled && context.pointer_authority.supports_preview_gestures();
    let double_click = enabled
        .then_some(context.input.splitter_double_click)
        .flatten();
    let sense = if drag_enabled {
        if double_click.is_some() {
            Sense::click_and_drag()
        } else {
            Sense::drag()
        }
    } else if double_click.is_some() {
        Sense::click()
    } else if enabled {
        Sense::focusable_noninteractive()
    } else {
        Sense::hover()
    };
    let receiver = drag_enabled
        .then(|| context.plan.receiver_for_splitter(splitter))
        .flatten();
    let response = context.interact_receiver(hit, id, sense, receiver);
    let response = response.on_hover_text(match (drag_enabled, double_click) {
        (true, Some(SplitterDoubleClick::Equalize)) => {
            "Drag to resize panes; double-click to equalize adjacent panes. Use Tab to focus, then arrows or Home/End."
        }
        (true, Some(SplitterDoubleClick::Preferred)) => {
            "Drag to resize panes; double-click to restore preferred sizes, or equalize when none are declared. Use Tab to focus, then arrows or Home/End."
        }
        (false, Some(SplitterDoubleClick::Equalize)) => {
            "Double-click to equalize adjacent panes. Use Tab to focus, then arrows or Home/End. Drag resizing is unavailable in this host."
        }
        (false, Some(SplitterDoubleClick::Preferred)) => {
            "Double-click to restore preferred sizes, or equalize when none are declared. Use Tab to focus, then arrows or Home/End. Drag resizing is unavailable in this host."
        }
        (true, None) => {
            "Drag to resize panes. Use Tab to focus, then arrows to adjust or Home/End for limits."
        }
        (false, None) if enabled => {
            "Use Tab to focus, then arrows or Home/End to resize panes. Drag resizing is unavailable in this host."
        }
        (false, None) => "Pane resizing is unavailable.",
    });
    configure_accessibility(context.ui, context.plan, &response, splitter, enabled);
    let focused = enabled && response.has_focus();
    if !paint.is_source_gap() {
        let hovered = response.hovered();
        let dragging = context.plan.active_splitter_resize() == Some(splitter.visual_id());
        let emphasized = focused || dragging || (drag_enabled && hovered);
        context.panes.splitter_ui(
            &context.ui.painter_at(draw.union(hit)),
            &Splitter {
                id,
                surface: context.plan.surface(),
                root: splitter.root(),
                kind: SplitterKind::Divider(splitter.axis()),
                rect: draw,
                hit_rect: response.interact_rect,
                enabled,
                drag_enabled,
                hovered,
                focused,
                dragging,
                fill: context.splitter_fill(&response, emphasized),
            },
        );
    }
    if !enabled {
        return;
    }
    let horizontal = splitter.axis() == DockspaceAxis::Horizontal;
    if focused {
        context.ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                EventFilter {
                    horizontal_arrows: horizontal,
                    vertical_arrows: !horizontal,
                    ..Default::default()
                },
            );
        });
    }
    let pointer_reset = double_click
        .filter(|_| {
            response.double_clicked_by(PointerButton::Primary)
                && unambiguous_pointer_click(context.ui, &response, PointerButton::Primary)
        })
        .map(|action| {
            let adjustment = match action {
                SplitterDoubleClick::Equalize => SurfaceSplitterAdjustment::Equalize,
                SplitterDoubleClick::Preferred => SurfaceSplitterAdjustment::Preferred,
            };
            (id.with("pointer-size-reset"), adjustment)
        });
    let adjustment = if let Some((origin, adjustment)) = pointer_reset {
        context
            .plan
            .prepare_splitter_adjustment(splitter.visual_id(), adjustment)
            .map(|action| (origin, action))
    } else {
        splitter_adjustment(context.ui, id, horizontal, focused, |adjustments| {
            context.plan.prepare_splitter_adjustment_sequence(
                splitter.visual_id(),
                adjustments.iter().copied(),
            )
        })
    };
    if let Some((origin, action)) = adjustment {
        context.push_local_action(origin, action);
    }
    if drag_enabled && response.hovered() {
        context.ui.ctx().set_cursor_icon(match splitter.axis() {
            DockspaceAxis::Horizontal => CursorIcon::ResizeHorizontal,
            DockspaceAxis::Vertical => CursorIcon::ResizeVertical,
        });
    }
}

pub(crate) fn paint_junction(
    context: &mut RenderContext<'_, '_, '_>,
    junction: SplitterJunctionPaintRecord<'_>,
    paint: SurfacePaintDisposition,
) {
    if !context.plan.partition_available(junction.root()) {
        return;
    }
    let Some(hit) = egui_rect(junction.hit_bounds()) else {
        return;
    };
    let id = context
        .ui
        .make_persistent_id((context.instance_id, junction.input_id()));
    let core_operable = context.plan.splitter_junction_operable(junction);
    let operable = effective_enabled(context.ui, core_operable);
    let drag_operable = operable && context.pointer_authority.supports_preview_gestures();
    let axis_operability = [DockspaceAxis::Horizontal, DockspaceAxis::Vertical].map(|axis| {
        let present = junction.has_axis(axis);
        let core_operable = present && context.plan.splitter_junction_axis_operable(junction, axis);
        (
            axis,
            present,
            core_operable,
            effective_enabled(context.ui, core_operable),
        )
    });
    let receiver = drag_operable
        .then(|| context.plan.receiver_for_splitter_junction(junction))
        .flatten();
    let response = context.interact_receiver(
        hit,
        id,
        if drag_operable {
            Sense::DRAG
        } else {
            Sense::hover()
        },
        receiver,
    );
    context
        .ui
        .ctx()
        .accesskit_node_builder(response.id, |node| {
            let horizontal_operable = axis_operability[0].3;
            let vertical_operable = axis_operability[1].3;
            let any_axis_operable = horizontal_operable || vertical_operable;
            node.set_role(Role::Group);
            node.set_label("Resize pane grid");
            node.set_description(junction_description(
                drag_operable,
                horizontal_operable,
                vertical_operable,
            ));
            node.set_bounds(accesskit_bounds(hit));
            if !drag_operable && !any_axis_operable {
                node.set_disabled();
            }
        });
    let mut focused = false;
    for (axis, present, _core_axis_operable, axis_operable) in axis_operability {
        if !present {
            continue;
        }
        let Some(axis_response) =
            junction_axis_response(context.ui, hit, response.id, id, axis, axis_operable)
        else {
            continue;
        };
        if !axis_operable {
            continue;
        }
        let horizontal = axis == DockspaceAxis::Horizontal;
        let axis_focused = axis_response.has_focus();
        if axis_focused {
            focused = true;
            context.ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    axis_response.id,
                    EventFilter {
                        horizontal_arrows: horizontal,
                        vertical_arrows: !horizontal,
                        ..Default::default()
                    },
                );
            });
        }
        if let Some((origin, action)) = splitter_adjustment(
            context.ui,
            axis_response.id,
            horizontal,
            axis_focused,
            |adjustments| {
                context.plan.prepare_splitter_junction_adjustment_sequence(
                    junction.visual_id(),
                    axis,
                    adjustments.iter().copied(),
                )
            },
        ) {
            context.push_local_action(origin, action);
        }
    }
    if !paint.is_source_gap() {
        let hovered = response.hovered();
        let dragging = context.plan.active_splitter_resize() == Some(junction.visual_id());
        if drag_operable && hovered {
            context.ui.ctx().set_cursor_icon(CursorIcon::AllScroll);
        }
        let fill = if focused || dragging || (drag_operable && hovered) {
            context.splitter_fill(&response, true)
        } else {
            Color32::TRANSPARENT
        };
        context.panes.splitter_ui(
            &context.ui.painter_at(hit),
            &Splitter {
                id,
                surface: context.plan.surface(),
                root: junction.root(),
                kind: SplitterKind::Junction,
                rect: hit,
                hit_rect: response.interact_rect,
                enabled: drag_operable || axis_operability.iter().any(|axis| axis.3),
                drag_enabled: drag_operable,
                hovered,
                focused,
                dragging,
                fill,
            },
        );
    }
}

fn junction_description(
    drag_operable: bool,
    horizontal_operable: bool,
    vertical_operable: bool,
) -> &'static str {
    match (drag_operable, horizontal_operable, vertical_operable) {
        (true, true, true) => {
            "Drag to resize rows and columns, or use the horizontal and vertical splitter controls"
        }
        (true, true, false) => {
            "Drag to resize rows and columns, or use the horizontal splitter control"
        }
        (true, false, true) => {
            "Drag to resize rows and columns, or use the vertical splitter control"
        }
        (true, false, false) => "Drag to resize rows and columns",
        (false, true, true) => {
            "Use the horizontal and vertical splitter controls to resize rows and columns"
        }
        (false, true, false) => "Use the horizontal splitter control to resize columns",
        (false, false, true) => "Use the vertical splitter control to resize rows",
        (false, false, false) => "Pane grid resizing is unavailable",
    }
}

fn junction_axis_response(
    ui: &mut egui::Ui,
    hit: egui::Rect,
    parent_id: egui::Id,
    junction_id: egui::Id,
    axis: DockspaceAxis,
    enabled: bool,
) -> Option<egui::Response> {
    let layout_rect = inset_ui_rect(hit)?;
    let mut axis_ui = ui.new_child(
        UiBuilder::new()
            // Tab overflow can change the parent's auto-ID counter between
            // steps. Axis focus and accessibility must keep the junction ID.
            .scope_id(junction_id.with(("axis", axis)))
            .max_rect(layout_rect)
            .sense(if enabled {
                Sense::focusable_noninteractive()
            } else {
                Sense::hover()
            })
            .accessibility_parent(parent_id),
    );
    axis_ui.expand_to_include_rect(layout_rect);
    let response = axis_ui.response();
    axis_ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Splitter);
        node.set_bounds(accesskit_bounds(hit));
        match axis {
            DockspaceAxis::Horizontal => {
                node.set_label("Resize pane grid horizontally");
                node.set_description("Use Left/Right Arrow to resize columns, Home/End for limits");
                node.set_orientation(Orientation::Vertical);
            }
            DockspaceAxis::Vertical => {
                node.set_label("Resize pane grid vertically");
                node.set_description("Use Up/Down Arrow to resize rows, Home/End for limits");
                node.set_orientation(Orientation::Horizontal);
            }
        }
        if enabled {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        } else {
            node.set_disabled();
        }
    });
    drop(axis_ui);
    Some(response)
}

fn splitter_adjustment(
    ui: &egui::Ui,
    id: egui::Id,
    horizontal: bool,
    focused: bool,
    prepare: impl FnOnce(&[SurfaceSplitterAdjustment]) -> Option<PreparedSurfaceAction>,
) -> Option<(egui::Id, PreparedSurfaceAction)> {
    if !effective_enabled(ui, true) {
        return None;
    }
    let adjustments: Vec<_> = ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| input_adjustment(event, id, horizontal, focused))
            .collect()
    });
    if adjustments.is_empty() {
        return None;
    }
    let action = prepare(&adjustments)?;
    let mut keyboard = false;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if input_adjustment(event, id, horizontal, focused).is_some() {
                keyboard |= matches!(event, Event::Key { .. });
                false
            } else {
                true
            }
        });
    });
    if keyboard {
        // A newly focused control may not have installed its arrow filter
        // before begin_pass scheduled spatial traversal for this batch.
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    Some((id.with("ordered-adjust"), action))
}

fn input_adjustment(
    event: &Event,
    id: egui::Id,
    horizontal: bool,
    focused: bool,
) -> Option<SurfaceSplitterAdjustment> {
    if let Event::AccessKitActionRequest(request) = event {
        if request.target_node != id.accesskit_id()
            || request.target_tree != egui::accesskit::TreeId::ROOT
        {
            return None;
        }
        return match request.action {
            Action::Decrement => Some(SurfaceSplitterAdjustment::Decrement),
            Action::Increment => Some(SurfaceSplitterAdjustment::Increment),
            _ => None,
        };
    }
    focused
        .then(|| keyboard_adjustment(event, horizontal))
        .flatten()
}

fn keyboard_adjustment(event: &Event, horizontal: bool) -> Option<SurfaceSplitterAdjustment> {
    let Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if !modifiers.matches_logically(egui::Modifiers::NONE) {
        return None;
    }
    match key {
        Key::ArrowLeft if horizontal => Some(SurfaceSplitterAdjustment::Decrement),
        Key::ArrowUp if !horizontal => Some(SurfaceSplitterAdjustment::Decrement),
        Key::ArrowRight if horizontal => Some(SurfaceSplitterAdjustment::Increment),
        Key::ArrowDown if !horizontal => Some(SurfaceSplitterAdjustment::Increment),
        Key::Home => Some(SurfaceSplitterAdjustment::Minimum),
        Key::End => Some(SurfaceSplitterAdjustment::Maximum),
        _ => None,
    }
}

fn configure_accessibility(
    ui: &egui::Ui,
    plan: SurfacePaintPlan<'_>,
    response: &egui::Response,
    splitter: dockspace::runtime::SplitterPaintRecord<'_>,
    enabled: bool,
) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Splitter);
        node.set_label("Resize panes");
        node.set_orientation(match splitter.axis() {
            DockspaceAxis::Horizontal => Orientation::Vertical,
            DockspaceAxis::Vertical => Orientation::Horizontal,
        });
        if enabled {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        } else {
            node.set_disabled();
        }
        if enabled && let Some(range) = plan.splitter_resize_range(splitter.visual_id()) {
            node.set_numeric_value(range.value());
            node.set_min_numeric_value(range.minimum());
            node.set_max_numeric_value(range.maximum());
        }
    });
}
