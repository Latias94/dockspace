//! Ordered tab keyboard input follows current widget focus and one core action.

use dockspace::runtime::{
    DockspaceVisualId, SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
    SurfaceTabInput, SurfaceTabNavigation,
};
use egui::{Event, Key, KeyboardShortcut};

use crate::PaneFocusState;

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

pub(super) fn prepare(context: &RenderContext<'_, '_, '_>) {
    if (context.input.next_tab.is_some() || context.input.previous_tab.is_some())
        && can_cycle(context)
        && let Some(input) = matching_events(context, false).first()
        && matches!(
            input.event,
            Event::Key {
                key: egui::Key::Tab,
                ..
            }
        )
        && let Some(group) = focused_group(context, FocusPass::Previous)
        && prepare_navigation(context, group, std::slice::from_ref(input)).is_some()
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
    let mut events = matching_events(context, true);
    if events.is_empty() {
        return;
    }
    if !can_cycle(context) {
        return;
    }
    let Some(group) = focused_group(context, FocusPass::Current) else {
        return;
    };
    if group.tab.is_none() {
        events = matching_events(context, false);
    }
    if let Some(index) = events
        .iter()
        .position(|event| matches!(event.input, SurfaceTabInput::Activate))
    {
        // Activation hands later keys to pane content rather than retaining
        // header ownership for the rest of this original input packet.
        events.truncate(index + 1);
    }
    let Some(action) = prepare_navigation(context, group, &events) else {
        return;
    };
    let Some(target) = action.tab_focus_visual() else {
        return;
    };
    if !context.ui.input_mut(|input| {
        if events
            .iter()
            .any(|event| input.events.get(event.index) != Some(&event.event))
        {
            return false;
        }
        for event in events.iter().rev() {
            input.events.remove(event.index);
        }
        true
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
        "tab-keyboard-input",
        context.plan.surface(),
        group.visual,
    ));
    context.push_local_action(origin, action);
}

fn can_cycle(context: &RenderContext<'_, '_, '_>) -> bool {
    effective_enabled(context.ui, true)
        && context.ui.input(|input| input.focused)
        && !context.plan.has_manipulation_preview()
        && context.plan.prepare_gesture_cancel().is_none()
        && context.ui.ctx().dragged_id().is_none()
        && !context.ui.input(|input| input.pointer.any_down())
        && !egui::Popup::is_any_open(context.ui.ctx())
        && !context.ui.ctx().any_popup_open()
}

struct TabKeyInput {
    index: usize,
    event: Event,
    input: SurfaceTabInput,
    selected_start: bool,
}

fn matching_events(context: &RenderContext<'_, '_, '_>, header: bool) -> Vec<TabKeyInput> {
    context.ui.input(|input| {
        input
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
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
                let selected_start =
                    matches(context.input.previous_tab) || matches(context.input.next_tab);
                let input = if matches(context.input.previous_tab) {
                    SurfaceTabInput::Navigate(SurfaceTabNavigation::Previous)
                } else if matches(context.input.next_tab) {
                    SurfaceTabInput::Navigate(SurfaceTabNavigation::Next)
                } else if header && modifiers.matches_logically(egui::Modifiers::NONE) {
                    match key {
                        Key::ArrowLeft => SurfaceTabInput::Navigate(SurfaceTabNavigation::Previous),
                        Key::ArrowRight => SurfaceTabInput::Navigate(SurfaceTabNavigation::Next),
                        Key::Home => SurfaceTabInput::Navigate(SurfaceTabNavigation::First),
                        Key::End => SurfaceTabInput::Navigate(SurfaceTabNavigation::Last),
                        Key::Enter | Key::Space => SurfaceTabInput::Activate,
                        _ => return None,
                    }
                } else {
                    return None;
                };
                Some(TabKeyInput {
                    index,
                    event: event.clone(),
                    input,
                    selected_start,
                })
            })
            .collect()
    })
}

#[derive(Clone, Copy)]
enum FocusPass {
    Previous,
    Current,
}

#[derive(Clone, Copy)]
struct FocusedGroup {
    visual: DockspaceVisualId,
    tab: Option<dockspace::model::ItemId>,
}

fn focused_group(context: &RenderContext<'_, '_, '_>, pass: FocusPass) -> Option<FocusedGroup> {
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
    let mut focused_tab = None;
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
                        if focused
                            == super::tabs::tab_id_for_visual(
                                context.ui,
                                context.instance_id,
                                tab.visual_id(),
                            )
                        {
                            focused_tab = Some(tab.item());
                        }
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
    group.map(|visual| FocusedGroup {
        visual,
        tab: focused_tab,
    })
}

fn prepare_navigation(
    context: &RenderContext<'_, '_, '_>,
    group: FocusedGroup,
    inputs: &[TabKeyInput],
) -> Option<dockspace::runtime::PreparedSurfaceAction> {
    let first = inputs.first()?;
    let mut source = group.tab;
    let mut selected = None;
    context
        .plan
        .visit_surface_program(|step| {
            if let Step::Operation {
                operation, paint, ..
            } = step
                && !paint.is_source_gap()
            {
                match operation {
                    Operation::Pane(pane) if pane.tab_bar_visual_id() == group.visual => {
                        selected = pane.selected();
                    }
                    Operation::Tab(tab)
                        if source.is_none()
                            && tab.tab_bar_visual_id() == group.visual
                            && tab.operable() =>
                    {
                        source = Some(tab.item());
                    }
                    _ => {}
                }
            }
            Ok::<(), ()>(())
        })
        .ok()?;
    let source = source?;
    let start = if first.selected_start {
        selected?
    } else {
        group.tab?
    };
    context
        .plan
        .prepare_tab_input_sequence(source, start, inputs.iter().map(|input| input.input))
}
