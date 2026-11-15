//! Optional tab cycling follows current widget focus and the ordinary navigation action.

use dockspace::runtime::{
    DockspaceVisualId, SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
    SurfaceTabNavigation,
};
use egui::{Event, KeyboardShortcut};

use crate::PaneFocusState;

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

pub(super) fn prepare(context: &RenderContext<'_, '_, '_>) {
    if can_cycle(context)
        && let Some((_, event, navigation)) = matching_event(context)
        && matches!(
            event,
            Event::Key {
                key: egui::Key::Tab,
                ..
            }
        )
        && let Some(group) = focused_group(context, FocusPass::Previous)
        && prepare_navigation(context, group, navigation).is_some()
    {
        // egui treats Ctrl+Shift+Tab as reverse traversal. Cancel that before
        // focus registration can queue a delayed destination. Content still
        // receives the untouched event before capture decides whether to cycle.
        context
            .ui
            .memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
}

pub(super) fn capture(context: &mut RenderContext<'_, '_, '_>) {
    if !can_cycle(context) {
        return;
    }
    let Some((event_index, event, navigation)) = matching_event(context) else {
        return;
    };
    let Some(group) = focused_group(context, FocusPass::Current) else {
        return;
    };
    let Some(action) = prepare_navigation(context, group, navigation) else {
        return;
    };
    let Some(target) = action.tab_focus_visual() else {
        return;
    };
    if !context.ui.input_mut(|input| {
        if input.events.get(event_index) == Some(&event) {
            input.events.remove(event_index);
            true
        } else {
            false
        }
    }) {
        return;
    }
    context.ui.memory_mut(|memory| {
        memory.move_focus(egui::FocusDirection::None);
        memory.request_focus(super::tabs::tab_id_for_visual(
            context.ui,
            context.instance_id,
            target,
        ));
    });
    let origin = context.ui.make_persistent_id((
        context.instance_id,
        "configured-tab-navigation",
        context.plan.surface(),
        navigation,
    ));
    context.push_local_action(origin, action);
}

fn can_cycle(context: &RenderContext<'_, '_, '_>) -> bool {
    (context.input.next_tab.is_some() || context.input.previous_tab.is_some())
        && effective_enabled(context.ui, true)
        && context.ui.input(|input| input.focused)
        && !context.plan.has_manipulation_preview()
        && context.plan.prepare_gesture_cancel().is_none()
        && context.ui.ctx().dragged_id().is_none()
        && !context.ui.input(|input| input.pointer.any_down())
        && !egui::Popup::is_any_open(context.ui.ctx())
        && !context.ui.ctx().any_popup_open()
}

fn matching_event(
    context: &RenderContext<'_, '_, '_>,
) -> Option<(usize, Event, SurfaceTabNavigation)> {
    context.ui.input(|input| {
        input.events.iter().enumerate().find_map(|(index, event)| {
            let Event::Key {
                key,
                modifiers,
                pressed: true,
                ..
            } = event
            else {
                return None;
            };
            let matches = |shortcut: Option<KeyboardShortcut>| {
                shortcut.is_some_and(|shortcut| {
                    *key == shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers)
                })
            };
            if matches(context.input.previous_tab) {
                Some((index, event.clone(), SurfaceTabNavigation::Previous))
            } else if matches(context.input.next_tab) {
                Some((index, event.clone(), SurfaceTabNavigation::Next))
            } else {
                None
            }
        })
    })
}

#[derive(Clone, Copy)]
enum FocusPass {
    Previous,
    Current,
}

fn focused_group(
    context: &RenderContext<'_, '_, '_>,
    pass: FocusPass,
) -> Option<DockspaceVisualId> {
    let focused = context.ui.memory(egui::Memory::focused)?;
    let widget = context.ui.ctx().viewport(|viewport| {
        let state = match pass {
            FocusPass::Previous => &viewport.prev_pass,
            FocusPass::Current => &viewport.this_pass,
        };
        state.widgets.get(focused).copied().filter(|widget| {
            widget.enabled
                && widget.visible
                && widget.sense.is_focusable()
                && widget.layer_id == context.ui.layer_id()
                && widget.interact_rect.is_finite()
                && widget.interact_rect.is_positive()
        })
    })?;

    let mut group = None;
    let mut blocked = false;
    let result = context.plan.visit_surface_program(|step| {
        match step {
            Step::Operation {
                operation, paint, ..
            } if !paint.is_source_gap() => {
                let candidate = match operation {
                    Operation::Pane(pane) => pane.selected().and_then(|item| {
                        let bounds = match pass {
                            FocusPass::Current => egui_rect(pane.content_bounds()),
                            FocusPass::Previous => {
                                let id = context.ui.make_persistent_id((
                                    context.instance_id,
                                    "pane-body",
                                    pane.visual_id(),
                                ));
                                context.ui.ctx().viewport(|viewport| {
                                    viewport
                                        .prev_pass
                                        .widgets
                                        .get(id)
                                        .map(|pane| pane.interact_rect)
                                })
                            }
                        };
                        (context.panes.focus_state(item, context.ui.ctx())
                            == PaneFocusState::Focused
                            && bounds
                                .is_some_and(|bounds| bounds.contains_rect(widget.interact_rect)))
                        .then(|| pane.tab_bar_visual_id())
                    }),
                    Operation::Tab(tab)
                        if focused
                            == super::tabs::tab_id_for_visual(
                                context.ui,
                                context.instance_id,
                                tab.visual_id(),
                            )
                            || focused
                                == context.ui.make_persistent_id((
                                    context.instance_id,
                                    "tab-close",
                                    tab.item(),
                                )) =>
                    {
                        Some(tab.tab_bar_visual_id())
                    }
                    Operation::TabStripControl(control)
                        if focused == super::tab_chrome::control_id(context, control) =>
                    {
                        Some(control.tab_bar_visual_id())
                    }
                    _ => None,
                };
                if let Some(candidate) = candidate {
                    blocked |= group.is_some_and(|previous| previous != candidate);
                    group = Some(candidate);
                }
            }
            Step::PopupBackdrop(_) | Step::PopupMenu { .. } => blocked = true,
            _ => {}
        }
        Ok::<(), ()>(())
    });
    if result.is_err() || blocked || context.ui.memory(egui::Memory::focused) != Some(focused) {
        return None;
    }
    group
}

fn prepare_navigation(
    context: &RenderContext<'_, '_, '_>,
    group: DockspaceVisualId,
    navigation: SurfaceTabNavigation,
) -> Option<dockspace::runtime::PreparedSurfaceAction> {
    let mut action = None;
    // Core navigates from the selected item; the source only locates its strip.
    // Keep that source deterministic across passes, including when navigation
    // rejects a source which is already the destination.
    context
        .plan
        .visit_surface_program(|step| {
            if action.is_none()
                && let Step::Operation {
                    operation: Operation::Tab(tab),
                    paint,
                    ..
                } = step
                && !paint.is_source_gap()
                && tab.tab_bar_visual_id() == group
                && tab.operable()
            {
                action = context.plan.prepare_tab_navigation(tab.item(), navigation);
            }
            Ok::<(), ()>(())
        })
        .ok()?;
    action
}
