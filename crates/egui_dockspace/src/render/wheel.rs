//! Official-egui wheel input reuses the core's current tab scroll receivers.

use dockspace::geometry::LogicalPoint;
use dockspace::runtime::{
    PreparedSurfaceAction, SurfacePaintProgramOperation as Operation,
    SurfacePaintProgramStep as Step,
};
use egui::{Event, Id, Pos2, Ui, Vec2};

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

/// Input ownership for one original egui run, without retaining scroll geometry.
#[derive(Clone, Copy)]
struct WheelClaim {
    first_pass: u64,
    last_pass: u64,
    axes: [bool; 2],
    owner: Option<(Id, Pos2)>,
}

#[derive(Clone, Copy)]
struct WheelLocation {
    pass: u64,
    cursor: Option<Pos2>,
    sample: WheelSample,
}

#[derive(Clone, Copy)]
enum WheelSample {
    None,
    At(Pos2),
    Ambiguous,
}

impl WheelLocation {
    fn for_pass(ui: &Ui) -> Self {
        let context = ui.ctx();
        let pass = context.cumulative_pass_nr();
        let id = Id::unique(("dockspace-wheel-location", context.viewport_id()));
        let previous = context.data(|data| data.get_temp::<Self>(id));
        if let Some(location) = previous.filter(|location| location.pass == pass) {
            return location;
        }
        let location = ui.input(|input| {
            let mut point = previous
                .filter(|location| location.pass.checked_add(1) == Some(pass))
                .and_then(|location| location.cursor);
            let mut sample = WheelSample::None;
            for event in &input.raw.events {
                match event {
                    Event::PointerMoved(position) | Event::PointerButton { pos: position, .. } => {
                        point = Some(*position);
                    }
                    Event::PointerGone => point = None,
                    Event::MouseWheel { .. } => {
                        // egui provides one smoothed delta for the pass. We can
                        // attribute it only when all raw samples share one point.
                        sample = match (sample, point) {
                            (WheelSample::None, Some(point)) => WheelSample::At(point),
                            (WheelSample::At(previous), Some(point)) if previous == point => sample,
                            _ => WheelSample::Ambiguous,
                        };
                    }
                    _ => {}
                }
            }
            Self {
                pass,
                cursor: input.pointer.latest_pos(),
                sample,
            }
        });
        context.data_mut(|data| data.insert_temp(id, location));
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
    let first_pass = pass
        .saturating_sub(u64::try_from(context.ui.ctx().current_pass_index()).unwrap_or(u64::MAX));
    let previous = context
        .ui
        .ctx()
        .data(|data| data.get_temp::<WheelClaim>(id));
    if let Some(claim) = previous.filter(|claim| claim.first_pass == first_pass) {
        // egui may advance wheel smoothing during a layout retry. The original
        // run already owns its action, including when a later callback discards it.
        consume(context, claim.axes);
        context.ui.ctx().data_mut(|data| {
            data.insert_temp(
                id,
                WheelClaim {
                    last_pass: pass,
                    ..claim
                },
            );
        });
        return;
    }
    let location = WheelLocation::for_pass(context.ui);
    let previous_owner = previous
        .filter(|claim| claim.last_pass.checked_add(1) == Some(first_pass))
        .and_then(|claim| claim.owner);
    let smoothing_tail = matches!(location.sample, WheelSample::None);
    if smoothing_tail && previous_owner.is_none() {
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
                axes: [false; 2],
                owner: None,
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
    let Some(bounds) = egui_rect(context.plan.bounds()) else {
        return;
    };
    let Some(global_point) = (match location.sample {
        WheelSample::None => previous_owner.map(|(_, at)| at),
        WheelSample::At(point) => Some(point),
        WheelSample::Ambiguous => None,
    }) else {
        return;
    };
    let point = context
        .ui
        .ctx()
        .layer_transform_from_global(context.ui.layer_id())
        .map_or(global_point, |transform| transform * global_point);
    let clip = context.ui.clip_rect().intersect(bounds);
    if !clip.is_positive()
        || !clip.contains(point)
        || context.ui.ctx().layer_id_at(global_point) != Some(context.ui.layer_id())
        || !owns_framework_hits(context, global_point, point)
    {
        return;
    }
    let Ok(logical_point) = LogicalPoint::new(f64::from(point.x), f64::from(point.y)) else {
        return;
    };
    let delta = context.ui.input(|input| input.smooth_scroll_delta());
    if !delta.is_finite() || delta == Vec2::ZERO {
        return;
    }
    let Some((origin, action, axes)) = prepare(context, id, logical_point, delta) else {
        return;
    };
    let owner = (origin, global_point);
    if smoothing_tail && previous_owner != Some(owner) {
        return;
    }
    consume(context, axes);
    context.ui.ctx().data_mut(|data| {
        data.insert_temp(
            id,
            WheelClaim {
                first_pass,
                last_pass: pass,
                axes,
                owner: Some(owner),
            },
        );
    });
    if let Some(action) = action {
        context.push_local_action(origin, action);
        context.ui.ctx().request_repaint();
    }
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

fn consume(context: &RenderContext<'_, '_, '_>, axes: [bool; 2]) {
    context.ui.input_mut(|input| {
        for (axis, consumed) in axes.into_iter().enumerate() {
            if consumed {
                input.smooth_scroll_delta[axis] = 0.0;
            }
        }
    });
}
