//! Official-egui wheel input reuses the core's current tab scroll receivers.

use dockspace::geometry::LogicalPoint;
use dockspace::runtime::{
    PreparedSurfaceAction, SurfacePaintProgramOperation as Operation,
    SurfacePaintProgramStep as Step,
};
use egui::{
    Context, Event, Id, InputOptions, InputState, Pos2, RawInput, Ui, Vec2, ViewportId,
    ViewportIdMap,
};
use std::collections::hash_map::Entry;

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

/// Input ownership for one original egui run, without retaining scroll geometry.
#[derive(Clone)]
struct WheelClaim {
    first_pass: u64,
    last_pass: u64,
    owners: Vec<WheelOwner>,
}

#[derive(Clone, Copy, PartialEq)]
struct WheelOwner {
    receiver: Id,
    point: Pos2,
    axes: [bool; 2],
}

#[derive(Default)]
struct WheelRouting {
    viewports: ViewportIdMap<WheelPass>,
}

struct WheelPass {
    index: u64,
    cursor: Option<Pos2>,
    streams: Vec<WheelStream>,
    previous_owners: Vec<WheelOwner>,
    owners: Vec<WheelOwner>,
}

struct WheelStream {
    point: Pos2,
    input: InputState,
    fresh: bool,
    carried: Vec2,
}

impl WheelRouting {
    fn current(&mut self, ui: &Ui) -> &mut WheelPass {
        let context = ui.ctx();
        let index = context.cumulative_pass_nr();
        match self.viewports.entry(context.viewport_id()) {
            Entry::Occupied(mut entry) => {
                let pass = entry.get_mut();
                if pass.index != index {
                    let previous = (pass.index.checked_add(1) == Some(index)).then_some(pass);
                    let next = WheelPass::new(ui, index, previous);
                    entry.insert(next);
                }
                entry.into_mut()
            }
            Entry::Vacant(entry) => entry.insert(WheelPass::new(ui, index, None)),
        }
    }
}

impl egui::plugin::Plugin for WheelRouting {
    fn debug_name(&self) -> &'static str {
        "egui_dockspace::wheel_routing"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let state = self.current(ui);
        let mut owned = Vec2::ZERO;
        for stream in &state.streams {
            for axis in 0..2 {
                if state
                    .previous_owners
                    .iter()
                    .any(|owner| owner.point == stream.point && owner.axes[axis])
                {
                    owned[axis] += stream.carried[axis];
                }
            }
        }
        if owned == Vec2::ZERO {
            return;
        }
        // A ScrollArea gets the remainder after removing the signed motion
        // already owned by chrome. Opposing streams can cancel in egui's
        // aggregate, so its magnitude cannot bound that reservation.
        ui.input_mut(|input| {
            let available = input.smooth_scroll_delta;
            let remaining = available - owned;
            if remaining.is_finite() && (available != Vec2::ZERO || input.zoom_delta() == 1.0) {
                input.smooth_scroll_delta = remaining;
            }
        });
    }

    fn output_hook(&mut self, context: &Context, _: &mut egui::FullOutput) {
        self.viewports.retain(|&viewport, _| {
            viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(viewport) != 0
        });
    }
}

impl WheelPass {
    fn new(ui: &Ui, index: u64, previous: Option<&mut Self>) -> Self {
        let options = ui.ctx().options(|options| options.input_options);
        ui.input(|input| {
            let focus_intact = input.focused
                && !input
                    .raw
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::WindowFocused(false)));
            let mut previous = previous.filter(|_| focus_intact);
            let mut cursor = previous.as_ref().and_then(|pass| pass.cursor);
            let mut samples = Vec::<(Pos2, Vec<Event>)>::new();
            if focus_intact {
                for event in &input.raw.events {
                    match event {
                        Event::PointerMoved(point) | Event::PointerButton { pos: point, .. } => {
                            cursor = Some(*point)
                        }
                        Event::PointerGone => cursor = None,
                        Event::MouseWheel {
                            phase: egui::TouchPhase::End | egui::TouchPhase::Cancel,
                            ..
                        } => {
                            // egui clears the viewport's smoothing accumulator
                            // on either phase, including earlier events in this pass.
                            previous = None;
                            samples.clear();
                        }
                        Event::MouseWheel { .. } => {
                            if let Some(point) = cursor {
                                if let Some((_, events)) =
                                    samples.iter_mut().find(|(at, _)| *at == point)
                                {
                                    events.push(event.clone());
                                } else {
                                    samples.push((point, vec![event.clone()]));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            let advance = |point, state: Option<InputState>, events: Vec<Event>| {
                let fresh = !events.is_empty();
                let prior = state.as_ref().filter(|_| fresh).map(|state| {
                    advance_wheel(input, options, state.clone(), Vec::new()).smooth_scroll_delta()
                });
                let input = advance_wheel(input, options, state.unwrap_or_default(), events);
                let carried = if fresh {
                    prior.unwrap_or_default()
                } else {
                    input.smooth_scroll_delta()
                };
                WheelStream {
                    point,
                    input,
                    fresh,
                    carried,
                }
            };
            // Use the framework's wheel conversion and smoothing at each input point.
            let mut streams = Vec::new();
            let mut previous_owners = Vec::new();
            if let Some(previous) = previous {
                previous_owners = std::mem::take(&mut previous.owners);
                for stream in std::mem::take(&mut previous.streams) {
                    let events = samples
                        .iter()
                        .position(|(at, _)| *at == stream.point)
                        .map_or_else(Vec::new, |index| samples.remove(index).1);
                    streams.push(advance(stream.point, Some(stream.input), events));
                }
            }
            streams.extend(
                samples
                    .into_iter()
                    .map(|(point, events)| advance(point, None, events)),
            );
            streams.retain(|stream| {
                stream.input.is_scrolling() || stream.input.smooth_scroll_delta() != Vec2::ZERO
            });
            Self {
                index,
                cursor: input.pointer.latest_pos(),
                streams,
                previous_owners,
                owners: Vec::new(),
            }
        })
    }
}

fn advance_wheel(
    input: &InputState,
    options: InputOptions,
    state: InputState,
    events: Vec<Event>,
) -> InputState {
    state.begin_pass(
        RawInput {
            screen_rect: Some(input.viewport_rect()),
            time: Some(input.time),
            predicted_dt: input.stable_dt,
            events,
            ..Default::default()
        },
        false,
        input.pixels_per_point(),
        options,
    )
}

pub(super) fn capture(context: &mut RenderContext<'_, '_, '_>) {
    // Native backends resolve the original wheel through their external journal.
    #[cfg(feature = "native-render-support")]
    if context.pointer_authority == super::PointerActionAuthority::ExternalJournal
        || context.scroll_registrar.is_some()
    {
        return;
    }

    let id = context.ui.make_persistent_id((
        context.instance_id,
        "tab-chrome-wheel",
        context.plan.surface(),
    ));
    let pass = context.ui.ctx().cumulative_pass_nr();
    let routing = context.ui.ctx().plugin_or_default::<WheelRouting>();
    let mut routing = routing.lock();
    let wheel = routing.current(context.ui);
    let first_pass = pass
        .saturating_sub(u64::try_from(context.ui.ctx().current_pass_index()).unwrap_or(u64::MAX));
    let previous = context
        .ui
        .ctx()
        .data(|data| data.get_temp::<WheelClaim>(id));
    if let Some(claim) = previous
        .as_ref()
        .filter(|claim| claim.first_pass == first_pass)
    {
        // egui may advance wheel smoothing during a layout retry. The original
        // run already owns its action, including when a later callback discards it.
        for stream in &wheel.streams {
            let delta = stream.input.smooth_scroll_delta();
            if delta == Vec2::ZERO {
                // A sizing retry can have the same timestamp as its first
                // pass. Keep ownership of the remaining motion in that case.
                wheel.owners.extend(
                    claim
                        .owners
                        .iter()
                        .copied()
                        .filter(|owner| owner.point == stream.point),
                );
                continue;
            }
            let Some(local) = point_in_surface(context, stream.point, stream.fresh) else {
                continue;
            };
            if let Some((origin, _, axes)) = prepare(context, id, local, delta) {
                let owner = WheelOwner {
                    receiver: origin,
                    point: stream.point,
                    axes,
                };
                if !claim.owners.contains(&owner) {
                    continue;
                }
                take_stream(context.ui, stream, owner, &wheel.previous_owners);
                wheel.owners.push(owner);
            }
        }
        context.ui.ctx().data_mut(|data| {
            data.insert_temp(
                id,
                WheelClaim {
                    last_pass: pass,
                    ..claim.clone()
                },
            );
        });
        return;
    }
    let previous_owners = previous
        .filter(|claim| claim.last_pass.checked_add(1) == Some(first_pass))
        .map_or_else(Vec::new, |claim| claim.owners);
    if wheel.streams.is_empty() && previous_owners.is_empty() {
        return;
    }
    // A fresh run starts without a claim. Even when application content owns
    // this wheel, its later smoothing tail must not reacquire old chrome.
    context.ui.ctx().data_mut(|data| {
        data.insert_temp(
            id,
            WheelClaim {
                first_pass,
                last_pass: pass,
                owners: Vec::new(),
            },
        );
    });

    if !effective_enabled(context.ui, true)
        || !context.ui.input(|input| input.focused)
        || context.plan.has_manipulation_preview()
        || context.plan.prepare_gesture_cancel().is_some()
        || context.ui.ctx().dragged_id().is_some()
        || egui::Popup::is_any_open(context.ui.ctx())
        || context.ui.ctx().any_popup_open()
    {
        return;
    }
    let mut owners = Vec::new();
    let mut commands = Vec::<(Id, LogicalPoint, Vec2, Option<PreparedSurfaceAction>)>::new();
    for stream in &wheel.streams {
        let delta = stream.input.smooth_scroll_delta();
        if !delta.is_finite() {
            continue;
        }
        if delta == Vec2::ZERO {
            owners.extend(
                previous_owners
                    .iter()
                    .copied()
                    .filter(|owner| owner.point == stream.point),
            );
            continue;
        }
        let Some(point) = point_in_surface(context, stream.point, stream.fresh) else {
            continue;
        };
        let Some((origin, action, ownership)) = prepare(context, id, point, delta) else {
            continue;
        };
        let owner = WheelOwner {
            receiver: origin,
            point: stream.point,
            axes: ownership,
        };
        if !stream.fresh && !previous_owners.contains(&owner) {
            continue;
        }
        let captured = take_stream(context.ui, stream, owner, &wheel.previous_owners);
        if captured == Vec2::ZERO {
            continue;
        }
        owners.push(owner);
        if action.is_some() {
            if let Some((_, _, previous, prepared)) =
                commands.iter_mut().find(|(id, _, _, _)| *id == origin)
            {
                *previous += captured;
                *prepared = None;
            } else {
                commands.push((
                    origin,
                    point,
                    captured,
                    action.filter(|_| captured == delta),
                ));
            }
        }
    }
    wheel.owners.extend(owners.iter().copied());
    drop(routing);
    context.ui.ctx().data_mut(|data| {
        data.insert_temp(
            id,
            WheelClaim {
                first_pass,
                last_pass: pass,
                owners,
            },
        );
    });
    for (origin, point, delta, prepared) in commands {
        if let Some(action) = prepared.or_else(|| prepare(context, id, point, delta)?.1) {
            context.push_local_action(origin, action);
            context.ui.ctx().request_repaint();
        }
    }
}

fn point_in_surface(
    context: &RenderContext<'_, '_, '_>,
    global: Pos2,
    fresh: bool,
) -> Option<LogicalPoint> {
    let layer = context.ui.layer_id();
    let local = context
        .ui
        .ctx()
        .layer_transform_from_global(layer)
        .map_or(global, |transform| transform * global);
    let clip = context
        .ui
        .clip_rect()
        .intersect(egui_rect(context.plan.bounds())?);
    if !clip.is_positive() || !clip.contains(local) {
        return None;
    }
    // A tail stays with the receiver that already accepted this sample. Its
    // new paint record still has to identify the same scroll target.
    if fresh
        && (context.ui.ctx().layer_id_at(global) != Some(layer)
            || !owns_framework_hits(context, global, local))
    {
        return None;
    }
    LogicalPoint::new(f64::from(local.x), f64::from(local.y)).ok()
}

fn prepare(
    context: &RenderContext<'_, '_, '_>,
    id: Id,
    point: LogicalPoint,
    delta: Vec2,
) -> Option<(Id, Option<PreparedSurfaceAction>, [bool; 2])> {
    let single_direction = context.ui.style().always_scroll_the_only_direction;
    let mut prepared = None;
    let mut popup_owner = None;
    context
        .plan
        .visit_surface_program(|step| {
            if let Step::PopupBackdrop(backdrop) = step {
                if context
                    .plan
                    .receiver_for_tab_list_menu_backdrop(backdrop)
                    .is_some_and(|receiver| receiver.bounds().contains(point))
                {
                    popup_owner = Some(id.with(backdrop.visual_id()));
                }
            } else if prepared.is_none() {
                match step {
                    Step::Operation {
                        operation: Operation::TabBar(bar),
                        paint,
                        ..
                    } if !paint.is_source_gap() => {
                        let amount = scroll_amount(delta, 0, single_direction);
                        if let Some(action) =
                            context.plan.prepare_tab_strip_scroll_at(bar, point, amount)
                        {
                            prepared = Some((
                                id.with(bar.group_menu_input_id()),
                                action,
                                consumed_axes(0, single_direction),
                            ));
                        }
                    }
                    Step::PopupMenu { menu, paint } if !paint.is_source_gap() => {
                        let amount = scroll_amount(delta, 1, single_direction);
                        if let Some(action) = context
                            .plan
                            .prepare_tab_list_menu_scroll_at(menu, point, amount)
                        {
                            prepared = Some((
                                id.with(menu.input_id()),
                                action,
                                consumed_axes(1, single_direction),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            Ok::<(), ()>(())
        })
        .ok()?;
    if let Some((origin, action, axes)) = prepared {
        Some((
            origin,
            Some(action),
            if popup_owner.is_some() {
                [true; 2]
            } else {
                axes
            },
        ))
    } else {
        popup_owner.map(|owner| (owner, None, [true; 2]))
    }
}

fn owns_framework_hits(context: &RenderContext<'_, '_, '_>, global: Pos2, local: Pos2) -> bool {
    let current = context.ui.input(|input| input.pointer.interact_pos());
    let layer = context.ui.layer_id();
    context.ui.ctx().viewport(|viewport| {
        let hits = if current == Some(global) {
            [viewport.hits.click, viewport.hits.drag]
        } else {
            // The cached hit result belongs to the last pointer position. Use
            // the registered click and drag lanes at the wheel's actual point.
            let mut hits = [None, None];
            for widget in viewport.prev_pass.widgets.get_layer(layer) {
                if !widget.enabled || !widget.interact_rect.contains(local) {
                    continue;
                }
                if widget.sense.senses_click() {
                    hits[0] = Some(*widget);
                }
                if widget.sense.senses_drag() {
                    hits[1] = Some(*widget);
                }
            }
            hits
        };
        hits.into_iter()
            .flatten()
            .filter(|hit| hit.interact_rect.contains(local))
            .all(|hit| {
                context.receivers.iter().any(|binding| {
                    binding.interactive
                        && binding.widget_id == hit.id
                        && binding.layer_id == hit.layer_id
                })
            })
    })
}

fn scroll_amount(delta: Vec2, axis: usize, single_direction: bool) -> f64 {
    if single_direction {
        -f64::from(delta.x) - f64::from(delta.y)
    } else {
        -f64::from(delta[axis])
    }
}

fn consumed_axes(axis: usize, single_direction: bool) -> [bool; 2] {
    [single_direction || axis == 0, single_direction || axis == 1]
}

fn owned_axes(mut delta: Vec2, axes: [bool; 2]) -> Vec2 {
    for (axis, owned) in axes.into_iter().enumerate() {
        if !owned {
            delta[axis] = 0.0;
        }
    }
    delta
}

fn take_stream(ui: &Ui, stream: &WheelStream, owner: WheelOwner, previous: &[WheelOwner]) -> Vec2 {
    let carried = if previous.contains(&owner) {
        owned_axes(stream.carried, owner.axes)
    } else {
        Vec2::ZERO
    };
    ui.input_mut(|input| {
        let requested = owned_axes(stream.input.smooth_scroll_delta(), owner.axes) - carried;
        let taken = bounded_scroll(input.smooth_scroll_delta, requested);
        input.smooth_scroll_delta -= taken;
        carried + taken
    })
}

fn bounded_scroll(available: Vec2, requested: Vec2) -> Vec2 {
    let mut taken = Vec2::ZERO;
    for axis in 0..2 {
        if available[axis].is_finite() && requested[axis].signum() == available[axis].signum() {
            taken[axis] = requested[axis].clamp(-available[axis].abs(), available[axis].abs());
        }
    }
    taken
}
