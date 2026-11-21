//! Official-egui wheel input reuses the core's current tab scroll receivers.

use dockspace::geometry::LogicalPoint;
use dockspace::runtime::{
    PreparedSurfaceAction, SurfacePaintProgramOperation as Operation,
    SurfacePaintProgramStep as Step,
};
use std::sync::Arc;

use egui::{Event, Id, InputState, Pos2, RawInput, Ui, Vec2};

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

/// Input ownership for one original egui run, without retaining scroll geometry.
#[derive(Clone)]
struct WheelClaim {
    first_pass: u64,
    last_pass: u64,
    owners: Vec<(Id, Pos2)>,
}

struct WheelLocation {
    pass: u64,
    cursor: Option<Pos2>,
    streams: Vec<WheelStream>,
}

struct WheelStream {
    point: Pos2,
    input: InputState,
    fresh: bool,
}

impl WheelLocation {
    fn for_pass(ui: &Ui) -> Arc<Self> {
        let context = ui.ctx();
        let pass = context.cumulative_pass_nr();
        let id = Id::unique(("dockspace-wheel-location", context.viewport_id()));
        let previous = context.data(|data| data.get_temp::<Arc<Self>>(id));
        if let Some(location) = previous.as_ref().filter(|location| location.pass == pass) {
            return Arc::clone(location);
        }
        let options = context.options(|options| options.input_options);
        let previous = previous
            .as_deref()
            .filter(|location| location.pass.checked_add(1) == Some(pass));
        let location = Arc::new(ui.input(|input| {
            let mut cursor = previous.and_then(|location| location.cursor);
            let focus_intact = input.focused
                && !input
                    .raw
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::WindowFocused(false)));
            let mut samples = Vec::<(Pos2, Vec<Event>)>::new();
            if focus_intact {
                for event in &input.raw.events {
                    match event {
                        Event::PointerMoved(point) | Event::PointerButton { pos: point, .. } => {
                            cursor = Some(*point)
                        }
                        Event::PointerGone => cursor = None,
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
            let advance = |point, state: InputState, events: Vec<Event>| {
                let fresh = !events.is_empty();
                let input = state.begin_pass(
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
                );
                WheelStream {
                    point,
                    input,
                    fresh,
                }
            };
            // Use egui's own wheel conversion and smoothing at each input point.
            // The next wheel event must not move the tail from another receiver.
            let mut streams = Vec::new();
            if let Some(previous) = previous.filter(|_| focus_intact) {
                for stream in &previous.streams {
                    let events = samples
                        .iter()
                        .position(|(at, _)| *at == stream.point)
                        .map_or_else(Vec::new, |index| samples.remove(index).1);
                    streams.push(advance(stream.point, stream.input.clone(), events));
                }
            }
            streams.extend(
                samples
                    .into_iter()
                    .map(|(point, events)| advance(point, InputState::default(), events)),
            );
            streams.retain(|stream| {
                stream.input.is_scrolling() || stream.input.smooth_scroll_delta() != Vec2::ZERO
            });
            Self {
                pass,
                cursor: input.pointer.latest_pos(),
                streams,
            }
        }));
        context.data_mut(|data| data.insert_temp(id, Arc::clone(&location)));
        location
    }
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
    let location = WheelLocation::for_pass(context.ui);
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
        let mut captured = Vec2::ZERO;
        for stream in &location.streams {
            let delta = stream.input.smooth_scroll_delta();
            let Some(local) = point_in_surface(context, stream.point, stream.fresh) else {
                continue;
            };
            if let Some((origin, _, axes)) = prepare(context, id, local, delta)
                && claim.owners.contains(&(origin, stream.point))
            {
                add_captured(&mut captured, delta, axes);
            }
        }
        consume(context, captured);
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
    if location.streams.is_empty() && previous_owners.is_empty() {
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
    let mut captured = Vec2::ZERO;
    let mut commands = Vec::<(Id, LogicalPoint, Vec2, Option<PreparedSurfaceAction>)>::new();
    for stream in &location.streams {
        let delta = stream.input.smooth_scroll_delta();
        if !delta.is_finite() {
            continue;
        }
        if delta == Vec2::ZERO {
            owners.extend(
                previous_owners
                    .iter()
                    .copied()
                    .filter(|(_, point)| *point == stream.point),
            );
            continue;
        }
        let Some(point) = point_in_surface(context, stream.point, stream.fresh) else {
            continue;
        };
        let Some((origin, action, ownership)) = prepare(context, id, point, delta) else {
            continue;
        };
        let owner = (origin, stream.point);
        if !stream.fresh && !previous_owners.contains(&owner) {
            continue;
        }
        owners.push(owner);
        add_captured(&mut captured, delta, ownership);
        if let Some(action) = action {
            if let Some((_, _, previous, prepared)) =
                commands.iter_mut().find(|(id, _, _, _)| *id == origin)
            {
                *previous += delta;
                *prepared = None;
            } else {
                commands.push((origin, point, delta, Some(action)));
            }
        }
    }
    consume(context, captured);
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

fn add_captured(captured: &mut Vec2, delta: Vec2, axes: [bool; 2]) {
    for (axis, owned) in axes.into_iter().enumerate() {
        if owned {
            captured[axis] += delta[axis];
        }
    }
}

fn consume(context: &RenderContext<'_, '_, '_>, captured: Vec2) {
    context.ui.input_mut(|input| {
        for axis in 0..2 {
            let available = input.smooth_scroll_delta[axis];
            if captured[axis].signum() == available.signum() {
                input.smooth_scroll_delta[axis] -=
                    captured[axis].clamp(-available.abs(), available.abs());
            }
        }
    });
}
