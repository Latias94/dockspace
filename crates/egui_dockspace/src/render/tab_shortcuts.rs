//! Ordered tab keyboard input follows current widget focus and one core action.

use dockspace::runtime::{
    DockspaceVisualId, SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
    SurfaceTabInput, SurfaceTabNavigation,
};
use egui::{Event, FocusDirection, Id, Key, KeyboardShortcut};

use crate::PaneFocusState;

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

/// Returns whether a tab control can no longer receive the remaining keys this pass.
pub(super) fn prepare(context: &mut RenderContext<'_, '_, '_>) -> bool {
    if !can_cycle(context) {
        return false;
    }
    let traversals = context
        .ui
        .input(|input| tab_traversals(&input.events, context.input).collect::<Vec<_>>());
    if let Some(&(index, direction)) = traversals.first()
        && let Some(group) = focused_group(context, FocusPass::Previous)
        && group.control != FocusedControl::Group
    {
        let mut prefix = matching_events(context, true);
        let prefix_len = prefix.partition_point(|input| input.index < index);
        let tail_start = prefix.get(prefix_len).map(|input| input.index);
        prefix.truncate(prefix_len);
        if capture_keys(context, group, prefix) {
            return true;
        }
        context.ui.memory_mut(|memory| memory.move_focus(direction));
        let traversal_end = tail_start.map_or(traversals.len(), |tail| {
            traversals.partition_point(|(index, _)| *index < tail)
        });
        if traversal_end > 1 || tail_start.is_some() && direction == FocusDirection::Previous {
            // egui folds repeated traversal into one direction and schedules
            // reverse focus for the next pass. Apply the tab chrome stops in order.
            if !focus_through_tab_controls(context, &traversals[..traversal_end]) {
                return true;
            }
        }
        // A second traversal after a local action may depend on widgets created
        // by that action. Keep the remaining keys from activating the earlier control.
        return traversal_end < traversals.len();
    }
    if (context.input.next_tab.is_some() || context.input.previous_tab.is_some())
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
            .memory_mut(|memory| memory.move_focus(FocusDirection::None));
    }
    false
}

pub(super) fn capture(context: &mut RenderContext<'_, '_, '_>) {
    if !can_cycle(context) {
        return;
    }
    let mut events = matching_events(context, true);
    if events.is_empty() {
        return;
    }
    let Some(group) = focused_group(context, FocusPass::Current) else {
        return;
    };
    if let Some((index, _)) = context
        .ui
        .input(|input| tab_traversals(&input.events, context.input).last())
    {
        events.retain(|input| input.index > index);
    }
    capture_keys(context, group, events);
}

fn capture_keys(
    context: &mut RenderContext<'_, '_, '_>,
    group: FocusedGroup,
    mut events: Vec<TabKeyInput>,
) -> bool {
    if group.control.header().is_none() {
        let Some(start) = events.iter().position(|event| {
            event.selected_start
                || matches!(group.control, FocusedControl::Close(_))
                    && event.input == SurfaceTabInput::Activate
        }) else {
            return false;
        };
        events.drain(..start);
        if let FocusedControl::Close(tab) = group.control
            && !events[0].selected_start
        {
            let key = match events[0].event {
                Event::Key { key, .. } => key,
                _ => unreachable!("a tab action always originates from a key"),
            };
            if let Some(action) = context.plan.prepare_tab_close(tab)
                && consume_events(context.ui, &events[..1])
            {
                let origin = super::tab_close::id(context, tab).with(("keyboard-activate", key));
                context.push_local_action(origin, action);
            }
            return false;
        }
    }
    let activation = events
        .iter()
        .position(|event| matches!(event.input, SurfaceTabInput::Activate));
    if let Some(index) = activation {
        // Activation hands later keys to pane content rather than retaining
        // header ownership for the rest of this original input packet.
        events.truncate(index + 1);
    }
    let Some(action) = prepare_navigation(context, group, &events) else {
        return false;
    };
    let Some(target) = action.tab_focus_visual() else {
        return false;
    };
    if !consume_events(context.ui, &events) {
        return false;
    }
    context.ui.memory_mut(|memory| {
        memory.request_focus(super::tabs::tab_id_for_visual(
            context.ui,
            context.instance_id,
            target,
        ));
        memory.move_focus(FocusDirection::None);
    });
    let origin = context.ui.make_persistent_id((
        context.instance_id,
        "tab-keyboard-input",
        context.plan.surface(),
        group.visual,
    ));
    context.push_local_action(origin, action);
    activation.is_some()
}

fn consume_events(ui: &egui::Ui, events: &[TabKeyInput]) -> bool {
    ui.input_mut(|input| {
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
    })
}

fn tab_traversals<'a>(
    events: &'a [Event],
    input: &crate::DockInputConfig,
) -> impl Iterator<Item = (usize, FocusDirection)> + 'a {
    let shortcuts = [input.previous_tab, input.next_tab];
    events.iter().enumerate().filter_map(move |(index, event)| {
        let Event::Key {
            key: Key::Tab,
            modifiers,
            pressed: true,
            ..
        } = event
        else {
            return None;
        };
        if shortcuts.into_iter().flatten().any(|shortcut| {
            shortcut.logical_key == Key::Tab && modifiers.matches_exact(shortcut.modifiers)
        }) {
            return None;
        }
        if modifiers.is_none() {
            Some((index, FocusDirection::Next))
        } else if modifiers.shift_only() {
            Some((index, FocusDirection::Previous))
        } else {
            None
        }
    })
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
    control: FocusedControl,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusedControl {
    Group,
    Header(dockspace::model::TabKey),
    Close(dockspace::model::TabKey),
}

impl FocusedControl {
    fn header(self) -> Option<dockspace::model::TabKey> {
        match self {
            Self::Header(tab) => Some(tab),
            Self::Group | Self::Close(_) => None,
        }
    }
}

fn focused_group(context: &RenderContext<'_, '_, '_>, pass: FocusPass) -> Option<FocusedGroup> {
    let focused = context.ui.memory(egui::Memory::focused)?;
    group_for_widget(context, focused, pass)
        .filter(|_| context.ui.memory(egui::Memory::focused) == Some(focused))
}

fn focus_through_tab_controls(
    context: &RenderContext<'_, '_, '_>,
    traversals: &[(usize, FocusDirection)],
) -> bool {
    let Some(focused) = context.ui.memory(egui::Memory::focused) else {
        return false;
    };
    let mut target = focused;
    for &(_, direction) in traversals {
        let next = context.ui.ctx().viewport(|viewport| {
            let widgets = &viewport.prev_pass.widgets;
            let (layer, index) = widgets.order(target)?;
            let is_focusable =
                |widget: &&egui::WidgetRect| widget.enabled && widget.sense.is_focusable();
            let widget = match direction {
                FocusDirection::Previous => widgets
                    .get_layer(layer)
                    .take(index)
                    .filter(is_focusable)
                    .last(),
                FocusDirection::Next => widgets.get_layer(layer).skip(index + 1).find(is_focusable),
                _ => return None,
            }?;
            Some(widget.id)
        });
        let Some(next) = next.filter(|&id| {
            group_for_widget(context, id, FocusPass::Previous)
                .is_some_and(|group| group.control != FocusedControl::Group)
        }) else {
            return false;
        };
        target = next;
    }
    context.ui.memory_mut(|memory| {
        if memory.focused() != Some(focused) {
            return false;
        }
        memory.move_focus(FocusDirection::None);
        memory.request_focus(target);
        true
    })
}

fn group_for_widget(
    context: &RenderContext<'_, '_, '_>,
    focused: Id,
    pass: FocusPass,
) -> Option<FocusedGroup> {
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
    let mut control = FocusedControl::Group;
    let mut blocked = false;
    let result = context.plan.visit_surface_program(|step| {
        match step {
            Step::Operation {
                operation, paint, ..
            } if !paint.is_source_gap() => {
                let candidate = match operation {
                    Operation::Pane(pane) => {
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
                        (context.panes.focus_state(pane.item(), context.ui.ctx())
                            == PaneFocusState::Focused
                            && bounds
                                .is_some_and(|bounds| bounds.contains_rect(widget.interact_rect)))
                        .then(|| pane.tab_bar_visual_id())
                        .flatten()
                    }
                    Operation::Tab(tab) => {
                        if focused
                            == super::tabs::tab_id_for_visual(
                                context.ui,
                                context.instance_id,
                                tab.visual_id(),
                            )
                        {
                            control = FocusedControl::Header(tab.key());
                            Some(tab.tab_bar_visual_id())
                        } else if focused == super::tab_close::id(context, tab.key()) {
                            control = FocusedControl::Close(tab.key());
                            Some(tab.tab_bar_visual_id())
                        } else {
                            None
                        }
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
    if result.is_err() || blocked {
        return None;
    }
    group.map(|visual| FocusedGroup { visual, control })
}

fn prepare_navigation(
    context: &RenderContext<'_, '_, '_>,
    group: FocusedGroup,
    inputs: &[TabKeyInput],
) -> Option<dockspace::runtime::PreparedSurfaceAction> {
    let first = inputs.first()?;
    let mut source = group.control.header();
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
                    Operation::TabBar(bar) if bar.visual_id() == group.visual => {
                        selected = bar.selected();
                    }
                    Operation::Tab(tab) if tab.tab_bar_visual_id() == group.visual => {
                        if source.is_none() && tab.operable() {
                            source = Some(tab.key());
                        }
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
        group.control.header()?
    };
    context
        .plan
        .prepare_tab_input_sequence(source, start, inputs.iter().map(|input| input.input))
}
