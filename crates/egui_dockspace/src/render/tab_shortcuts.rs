//! Routes ordered keyboard input through tab chrome and hands content input off at focus changes.

mod pane_input;

use dockspace::model::{ItemId, TabKey};
use dockspace::runtime::{
    DockspaceVisualId, PanePaintRecord, SurfacePaintProgramOperation as Operation,
    SurfacePaintProgramStep as Step, SurfaceTabInput, SurfaceTabNavigation,
};
use egui::{Event, FocusDirection, Id, Key, KeyboardShortcut};

use crate::PaneFocusState;

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

pub(super) use pane_input::PaneInputDelivery;

pub(super) struct PaneCycle {
    item: ItemId,
    focused: Id,
    shortcut: Event,
    tail: Vec<Event>,
}

pub(super) enum TabKeyboard {
    Done,
    AfterWidgets,
    PaneShortcut(PaneCycle),
}

impl TabKeyboard {
    pub(super) fn prepare(context: &mut RenderContext<'_, '_, '_>) -> Self {
        match pane_input::prepare(context) {
            pane_input::PaneInputState::Ready(delivery) => context.pane_input = Some(delivery),
            pane_input::PaneInputState::Waiting => {}
            pane_input::PaneInputState::None => {
                if can_capture(context) {
                    return Self::prepare_keys(context);
                }
            }
        }
        Self::Done
    }

    fn prepare_keys(context: &mut RenderContext<'_, '_, '_>) -> Self {
        let keys = matching_events(context);
        let Some((focused, group)) = (!keys.is_empty())
            .then(|| focused_group(context, FocusPass::Previous))
            .flatten()
        else {
            return Self::AfterWidgets;
        };
        if group.control.is_tab_control() {
            route(context, focused, group, &keys, FocusPass::Previous);
            return Self::Done;
        }
        prepare_pane_cycle(context, focused, group, &keys)
            .map_or(Self::AfterWidgets, Self::PaneShortcut)
    }

    pub(super) fn for_replayed_pane(
        context: &RenderContext<'_, '_, '_>,
        pane: PanePaintRecord<'_>,
    ) -> Self {
        let cycle = context
            .ui
            .memory(egui::Memory::focused)
            .and_then(|focused| {
                prepare_pane_cycle(
                    context,
                    focused,
                    FocusedGroup {
                        visual: pane.tab_bar_visual_id()?,
                        control: FocusedControl::Pane(pane.item()),
                    },
                    &matching_events(context),
                )
            });
        cycle.map_or(Self::AfterWidgets, Self::PaneShortcut)
    }

    pub(super) fn capture(self, context: &mut RenderContext<'_, '_, '_>) {
        let routed_from_pane = matches!(self, Self::PaneShortcut(_));
        if matches!(self, Self::Done) {
            return;
        }
        if let Self::PaneShortcut(cycle) = self {
            if context.ui.memory(egui::Memory::focused) != Some(cycle.focused) {
                return;
            }
            if !context
                .ui
                .input(|input| input.events.contains(&cycle.shortcut))
            {
                // A pane that consumes its configured shortcut keeps the following
                // input. The callback has run, so deliver it on its next pass.
                pane_input::continue_at(context, cycle.item, cycle.focused, cycle.tail);
                return;
            }
            context
                .ui
                .input_mut(|input| input.events.extend(cycle.tail));
        }
        if !can_capture(context) {
            return;
        }
        let keys = matching_events(context);
        let keys = if routed_from_pane {
            keys.as_slice()
        } else {
            // egui has already applied traversal while registering widgets.
            // Only interpret commands that follow that handoff.
            let after_traversal = keys
                .iter()
                .rposition(|key| key.traversal.is_some())
                .map_or(0, |index| index + 1);
            &keys[after_traversal..]
        };
        if !keys.is_empty()
            && let Some((focused, group)) = focused_group(context, FocusPass::Current)
        {
            route(context, focused, group, keys, FocusPass::Current);
        }
    }
}

fn prepare_pane_cycle(
    context: &RenderContext<'_, '_, '_>,
    focused: Id,
    group: FocusedGroup,
    keys: &[TabKeyInput],
) -> Option<PaneCycle> {
    let shortcut = keys.iter().find(|key| key.selected_start)?;
    let _ = prepare_navigation(context, group, std::slice::from_ref(shortcut), None)?;
    if shortcut.key == Key::Tab {
        // Preserve configured Ctrl+Shift+Tab for the pane's shortcut handler.
        context
            .ui
            .memory_mut(|memory| memory.move_focus(FocusDirection::None));
    }
    let FocusedControl::Pane(item) = group.control else {
        return None;
    };
    let tail = pane_input::take_keys_after(context.ui, shortcut.index + 1);
    (!tail.is_empty()).then(|| PaneCycle {
        item,
        focused,
        shortcut: shortcut.event.clone(),
        tail,
    })
}

fn can_capture(context: &RenderContext<'_, '_, '_>) -> bool {
    effective_enabled(context.ui, true)
        && super::actions::focus_uninterrupted(context.ui)
        && !context.plan.has_manipulation_preview()
        && context.plan.prepare_gesture_cancel().is_none()
        && context.ui.ctx().dragged_id().is_none()
        && !context.ui.input(|input| input.pointer.any_down())
        && !egui::Popup::is_any_open(context.ui.ctx())
        && !context.ui.ctx().any_popup_open()
}

/// Routes through known tab controls. At a destination owned by egui, give it
/// the traversal and stop interpreting later keys as tab commands in this pass.
fn route(
    context: &mut RenderContext<'_, '_, '_>,
    mut focused: Id,
    mut group: FocusedGroup,
    keys: &[TabKeyInput],
    pass: FocusPass,
) {
    let mut selection = None;
    let mut consumed = Vec::new();
    let mut focus = None;
    let mut activation = None;
    let mut remaining = keys;
    while !remaining.is_empty() {
        let end = remaining
            .iter()
            .position(|key| key.traversal.is_some())
            .unwrap_or(remaining.len());
        let (chunk, tail) = remaining.split_at(end);
        if let Some((action, accepted)) = navigation_chunk(context, group, chunk, selection) {
            let first = &accepted[0];
            let last = &accepted[accepted.len() - 1];
            if let Some((key, visual)) = action.tab_focus_target().zip(action.tab_focus_visual()) {
                focused = super::tabs::tab_id_for_visual(context.ui, context.instance_id, visual);
                focus = Some((focused, FocusDirection::None));
                selection = Some((group.visual, key));
                if last.input == Some(SurfaceTabInput::Activate) {
                    activation = Some((last.index, last.key, key, focused));
                }
            }
            consumed.extend(accepted.iter().map(|key| key.index));
            let origin = context.ui.make_persistent_id((
                context.instance_id,
                "tab-keyboard-input",
                context.plan.surface(),
                group.visual,
                first.index,
            ));
            context.push_local_action(origin, action);

            if activation.is_some() {
                break;
            }
        }
        let Some((traversal, rest)) = tail.split_first() else {
            break;
        };
        let direction = traversal.traversal.expect("the chunk ends at a traversal");
        if focus.is_none() && !group.control.is_tab_control() {
            // The pane or application owns regular Tab traversal from here.
            break;
        }
        let next = adjacent_widget(context, focused, direction, pass).and_then(|next| {
            let group = group_for_widget(context, next, pass)?;
            group.control.is_tab_control().then_some((next, group))
        });
        let Some((next, destination)) = next else {
            if matches!(pass, FocusPass::Previous) {
                focus = Some((focused, direction));
            }
            break;
        };
        focused = next;
        group = destination;
        focus = Some((focused, FocusDirection::None));
        consumed.push(traversal.index);
        remaining = rest;
    }

    if let Some((index, key, tab, header)) = activation {
        pane_input::activate(context, index + 1, key, tab, header);
    }
    if !consumed.is_empty() {
        context.ui.input_mut(|input| {
            for index in consumed.into_iter().rev() {
                input.events.remove(index);
            }
        });
    }
    if let Some((target, direction)) = focus {
        context.ui.memory_mut(|memory| {
            if !memory.has_focus(target) {
                memory.request_focus(target);
            }
            memory.move_focus(direction);
        });
    }
}

fn navigation_chunk<'a>(
    context: &RenderContext<'_, '_, '_>,
    group: FocusedGroup,
    keys: &'a [TabKeyInput],
    selection: Option<(DockspaceVisualId, TabKey)>,
) -> Option<(dockspace::runtime::PreparedSurfaceAction, &'a [TabKeyInput])> {
    let keys = if group.control.header().is_some() {
        keys
    } else {
        let first = keys.iter().position(|key| {
            key.selected_start
                || matches!(group.control, FocusedControl::Close(_))
                    && key.input == Some(SurfaceTabInput::Activate)
        })?;
        &keys[first..]
    };
    let activation = keys
        .iter()
        .position(|key| key.input == Some(SurfaceTabInput::Activate));
    let keys = activation.map_or(keys, |activation| &keys[..=activation]);
    let first = keys.first()?;
    if let FocusedControl::Close(tab) = group.control
        && !first.selected_start
    {
        return context
            .plan
            .prepare_tab_close(tab)
            .map(|action| (action, &keys[..1]));
    }
    prepare_navigation(context, group, keys, selection).map(|action| (action, keys))
}

struct TabKeyInput {
    index: usize,
    key: Key,
    event: Event,
    input: Option<SurfaceTabInput>,
    traversal: Option<FocusDirection>,
    selected_start: bool,
}

fn matching_events(context: &RenderContext<'_, '_, '_>) -> Vec<TabKeyInput> {
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
                } = *event
                else {
                    return None;
                };
                let matches = |shortcut: Option<KeyboardShortcut>| {
                    shortcut.is_some_and(|shortcut| {
                        key == shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers)
                    })
                };
                let (input, traversal, selected_start) = if matches(context.input.previous_tab) {
                    (
                        Some(SurfaceTabInput::Navigate(SurfaceTabNavigation::Previous)),
                        None,
                        true,
                    )
                } else if matches(context.input.next_tab) {
                    (
                        Some(SurfaceTabInput::Navigate(SurfaceTabNavigation::Next)),
                        None,
                        true,
                    )
                } else if key == Key::Tab && (modifiers.is_none() || modifiers.shift_only()) {
                    let direction = if modifiers.is_none() {
                        FocusDirection::Next
                    } else {
                        FocusDirection::Previous
                    };
                    (None, Some(direction), false)
                } else if modifiers.matches_logically(egui::Modifiers::NONE) {
                    let input = match key {
                        Key::ArrowLeft => SurfaceTabInput::Navigate(SurfaceTabNavigation::Previous),
                        Key::ArrowRight => SurfaceTabInput::Navigate(SurfaceTabNavigation::Next),
                        Key::Home => SurfaceTabInput::Navigate(SurfaceTabNavigation::First),
                        Key::End => SurfaceTabInput::Navigate(SurfaceTabNavigation::Last),
                        Key::Enter | Key::Space => SurfaceTabInput::Activate,
                        _ => return None,
                    };
                    (Some(input), None, false)
                } else {
                    return None;
                };
                Some(TabKeyInput {
                    index,
                    key,
                    event: event.clone(),
                    input,
                    traversal,
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

#[derive(Clone, Copy)]
enum FocusedControl {
    Group,
    Pane(ItemId),
    Header(TabKey),
    Close(TabKey),
}

impl FocusedControl {
    fn header(self) -> Option<TabKey> {
        match self {
            Self::Header(tab) => Some(tab),
            _ => None,
        }
    }

    fn is_tab_control(self) -> bool {
        matches!(self, Self::Header(_) | Self::Close(_))
    }
}

fn focused_group(
    context: &RenderContext<'_, '_, '_>,
    pass: FocusPass,
) -> Option<(Id, FocusedGroup)> {
    let focused = context.ui.memory(egui::Memory::focused)?;
    group_for_widget(context, focused, pass)
        .filter(|_| context.ui.memory(egui::Memory::focused) == Some(focused))
        .map(|group| (focused, group))
}

fn adjacent_widget(
    context: &RenderContext<'_, '_, '_>,
    current: Id,
    direction: FocusDirection,
    pass: FocusPass,
) -> Option<Id> {
    context.ui.ctx().viewport(|viewport| {
        let widgets = match pass {
            FocusPass::Previous => &viewport.prev_pass.widgets,
            FocusPass::Current => &viewport.this_pass.widgets,
        };
        let (layer, index) = widgets.order(current)?;
        let focusable = |widget: &&egui::WidgetRect| widget.enabled && widget.sense.is_focusable();
        let adjacent = match direction {
            FocusDirection::Previous => widgets
                .get_layer(layer)
                .take(index)
                .filter(focusable)
                .last(),
            FocusDirection::Next => widgets.get_layer(layer).skip(index + 1).find(focusable),
            _ => return None,
        }?;
        Some(adjacent.id)
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
                        (context.ui.memory(egui::Memory::focused) == Some(focused)
                            && context.panes.focus_state(pane.item(), context.ui.ctx())
                                == PaneFocusState::Focused
                            && bounds
                                .is_some_and(|bounds| bounds.contains_rect(widget.interact_rect)))
                        .then(|| pane.tab_bar_visual_id())
                        .flatten()
                        .inspect(|_| control = FocusedControl::Pane(pane.item()))
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
    selection: Option<(DockspaceVisualId, TabKey)>,
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
        selection
            .filter(|(visual, _)| *visual == group.visual)
            .map(|(_, tab)| tab)
            .or(selected)?
    } else {
        group.control.header()?
    };
    context.plan.prepare_tab_input_sequence(
        source,
        start,
        inputs.iter().filter_map(|input| input.input),
    )
}
