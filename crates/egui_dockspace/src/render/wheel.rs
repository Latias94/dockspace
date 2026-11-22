//! Route wheel events against the completed egui pass before egui smooths them.

use dockspace::geometry::LogicalPoint;
use dockspace::runtime::{
    DockspaceReceiverDescriptor, DockspaceReceiverRole, PreparedSurfaceAction,
    SurfacePaintProgramOperation as Operation, SurfacePaintProgramStep as Step,
};
use egui::{
    Context, Event, Id, IdMap, InputOptions, InputState, LayerId, Modifiers, MouseWheelSource,
    Pos2, RawInput, Rect, TouchPhase, Ui, Vec2, ViewportId, ViewportIdMap,
};

use super::{RenderContext, actions::effective_enabled, geometry::egui_rect};

#[derive(Default)]
struct WheelRouting {
    viewports: ViewportIdMap<WheelViewport>,
}

type LayerHit = Box<dyn Fn(Pos2) -> Option<(LayerId, Pos2)> + Send + Sync>;

#[derive(Default)]
struct WheelViewport {
    pass: u64,
    layer_hit: Option<LayerHit>,
    surfaces: Vec<WheelSurface>,
    gesture: Option<WheelGesture>,
    streams: Vec<WheelStream>,
    advanced_run: Option<u64>,
}

struct WheelSurface {
    id: Id,
    layer: LayerId,
    regions: Vec<WheelRegion>,
    widgets: IdMap<Rect>,
}

enum WheelRegion {
    ContainedFrame(Rect),
    Target(WheelTarget),
}

struct WheelTarget {
    id: Id,
    bounds: Rect,
    hit: Rect,
    axes: [bool; 2],
    ordered: bool,
}

#[derive(Clone, Copy)]
struct WheelRoute {
    surface: Id,
    target: Id,
    axes: [bool; 2],
    offset: Vec2,
    ordered: bool,
}

struct WheelGesture {
    route: Option<WheelRoute>,
    source: MouseWheelSource,
    modifiers: Modifiers,
    last_event: f64,
    ended: bool,
}

struct WheelStream {
    route: WheelRoute,
    input: InputState,
    events: Vec<Event>,
    samples: Vec<Vec2>,
}

impl egui::plugin::Plugin for WheelRouting {
    fn debug_name(&self) -> &'static str {
        "egui_dockspace::wheel_routing"
    }

    fn input_hook(&mut self, context: &Context, input: &mut RawInput) {
        let viewport = input.viewport_id;
        let Some(state) = self.viewports.get_mut(&viewport) else {
            return;
        };
        let pass = context.cumulative_pass_nr_for(viewport);
        if state.pass.checked_add(1) != Some(pass)
            || !input.focused
            || input
                .events
                .iter()
                .any(|event| matches!(event, Event::WindowFocused(false)))
        {
            *state = WheelViewport {
                pass,
                ..Default::default()
            };
            return;
        }

        state.pass = pass;
        let layer_hit = state.layer_hit.take();
        let surfaces = std::mem::take(&mut state.surfaces);
        for stream in &mut state.streams {
            stream.samples.clear();
        }
        let (mut cursor, previous_time) = context.input_for(viewport, |previous| {
            (previous.pointer.latest_pos(), previous.time)
        });
        let time = input
            .time
            .unwrap_or(previous_time + f64::from(input.predicted_dt));
        let options = context.options(|options| options.input_options);

        input.events.retain_mut(|event| match event {
            Event::PointerMoved(point) | Event::PointerButton { pos: point, .. } => {
                cursor = Some(*point);
                true
            }
            Event::PointerGone => {
                cursor = None;
                true
            }
            Event::MouseWheel {
                unit,
                delta,
                phase,
                source,
                modifiers,
            } => {
                let (route, effective_modifiers) =
                    state.route_gesture(*source, *phase, *modifiers, time, || {
                        let (layer, point) = layer_hit.as_ref()?(cursor?)?;
                        context.viewport_for(viewport, |viewport| {
                            let widgets = &viewport.prev_pass.widgets;
                            surfaces.iter().rev().find_map(|surface| {
                                (surface.layer == layer)
                                    .then(|| surface.route_at(widgets, point))
                                    .flatten()
                            })
                        })
                    });
                let Some(route) = route else {
                    return true;
                };
                if !delta.is_finite() {
                    return true;
                }
                *modifiers = effective_modifiers;
                if effective_modifiers.matches_any(options.zoom_modifier) {
                    return true;
                }

                let axes = raw_axes(route.axes, effective_modifiers, options);
                let owned = Vec2::new(
                    if axes[0] { delta.x } else { 0.0 },
                    if axes[1] { delta.y } else { 0.0 },
                );
                if owned != Vec2::ZERO || (*phase != TouchPhase::Move && axes.contains(&true)) {
                    state.push(
                        route,
                        Event::MouseWheel {
                            unit: *unit,
                            delta: owned,
                            phase: *phase,
                            source: *source,
                            modifiers: effective_modifiers,
                        },
                    );
                }
                *delta -= owned;
                *delta != Vec2::ZERO || (*phase != TouchPhase::Move && axes.contains(&false))
            }
            _ => true,
        });
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if let Some(state) = self.viewports.get_mut(&ui.ctx().viewport_id())
            && !state.surfaces.is_empty()
        {
            state.layer_hit = layer_hit(ui.ctx());
        }
    }

    fn output_hook(&mut self, context: &Context, _: &mut egui::FullOutput) {
        self.viewports.retain(|&viewport, _| {
            viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(viewport) != 0
        });
    }
}

impl WheelViewport {
    fn route_gesture(
        &mut self,
        source: MouseWheelSource,
        phase: TouchPhase,
        modifiers: Modifiers,
        time: f64,
        hit: impl FnOnce() -> Option<WheelRoute>,
    ) -> (Option<WheelRoute>, Modifiers) {
        // Match egui's idle fallback for hosts that omit a terminal phase.
        if self.gesture.as_ref().is_some_and(|gesture| {
            time - gesture.last_event
                > if gesture.source == MouseWheelSource::Trackpad {
                    1.0
                } else {
                    0.15
                }
        }) {
            self.gesture = None;
        }
        if phase == TouchPhase::Start {
            let route = if source == MouseWheelSource::Momentum {
                self.gesture
                    .as_ref()
                    .map_or_else(hit, |gesture| gesture.route)
            } else {
                hit()
            };
            self.gesture = Some(WheelGesture {
                route,
                source,
                modifiers,
                last_event: time,
                ended: false,
            });
            return (route, modifiers);
        }
        let Some(gesture) = self.gesture.as_mut().filter(|gesture| {
            (gesture.source == source && !gesture.ended)
                || (gesture.source == MouseWheelSource::Trackpad
                    && source == MouseWheelSource::Momentum)
        }) else {
            return ((phase == TouchPhase::Move).then(hit).flatten(), modifiers);
        };
        gesture.source = source;
        gesture.modifiers |= modifiers;
        gesture.last_event = time;
        let routed = (gesture.route, gesture.modifiers);
        match phase {
            TouchPhase::End if source == MouseWheelSource::Trackpad => gesture.ended = true,
            TouchPhase::End | TouchPhase::Cancel => self.gesture = None,
            _ => {}
        }
        routed
    }

    fn push(&mut self, route: WheelRoute, event: Event) {
        if let Some(stream) = self.streams.iter_mut().find(|stream| {
            stream.route.surface == route.surface && stream.route.target == route.target
        }) {
            stream.route = route;
            stream.events.push(event);
        } else {
            self.streams.push(WheelStream {
                route,
                input: InputState::default(),
                events: vec![event],
                samples: Vec::new(),
            });
        }
    }

    fn advance(&mut self, ui: &Ui, pass: u64) {
        let first =
            pass.saturating_sub(u64::try_from(ui.ctx().current_pass_index()).unwrap_or(u64::MAX));
        if self.advanced_run == Some(first) {
            return;
        }
        self.advanced_run = Some(first);
        let options = ui.ctx().options(|options| options.input_options);
        ui.input(|input| {
            for stream in &mut self.streams {
                stream.advance(input, options);
            }
        });
        self.streams
            .retain(|stream| stream.input.is_scrolling() || !stream.samples.is_empty());
    }
}

impl WheelStream {
    fn advance(&mut self, frame: &InputState, options: InputOptions) {
        let mut events = std::mem::take(&mut self.events);
        let mut advance = |events, predicted_dt| {
            self.input = std::mem::take(&mut self.input).begin_pass(
                RawInput {
                    screen_rect: Some(frame.viewport_rect()),
                    time: Some(frame.time),
                    predicted_dt,
                    events,
                    ..Default::default()
                },
                false,
                frame.pixels_per_point(),
                options,
            );
            let sample = self.input.smooth_scroll_delta();
            if sample != Vec2::ZERO {
                self.samples.push(sample);
            }
        };
        if self.route.ordered && events.len() > 1 {
            // Keep directly delivered motion in order at the menu's endpoints.
            // Let egui carry pending smoothing through the same input state and
            // advance it once with the actual frame duration.
            for event in events.drain(..) {
                advance(vec![event], 0.0);
            }
        }
        advance(events, frame.stable_dt);
    }
}

impl WheelSurface {
    fn route_at(&self, widgets: &egui::WidgetRects, point: Pos2) -> Option<WheelRoute> {
        let region = self.regions.iter().rev().find(|region| {
            let bounds = match region {
                WheelRegion::ContainedFrame(bounds) => *bounds,
                WheelRegion::Target(target) => target.hit,
            };
            contains(bounds, point)
        })?;
        let WheelRegion::Target(target) = region else {
            // A foreground root can cover a rear strip without owning a strip itself.
            return None;
        };
        // Wheel events need direct hits. Check both framework widget lanes so
        // that newer application overlays keep their input on the same layer.
        let mut click = None;
        let mut drag = None;
        for widget in widgets
            .get_layer(self.layer)
            .filter(|widget| widget.enabled && widget.interact_rect.contains(point))
        {
            if widget.sense.senses_click() {
                click = Some(widget);
            }
            if widget.sense.senses_drag() {
                drag = Some(widget);
            }
        }
        if [click, drag]
            .into_iter()
            .flatten()
            .any(|widget| self.widgets.get(&widget.id) != Some(&widget.interact_rect))
        {
            return None;
        }
        Some(WheelRoute {
            surface: self.id,
            target: target.id,
            axes: target.axes,
            offset: point - target.bounds.min,
            ordered: target.ordered,
        })
    }
}

fn contains(rect: Rect, point: Pos2) -> bool {
    rect.contains(point) && point.x < rect.max.x && point.y < rect.max.y
}

fn layer_hit(context: &Context) -> Option<LayerHit> {
    if egui::Popup::is_any_open(context)
        || context.any_popup_open()
        || context.dragged_id().is_some()
    {
        return None;
    }
    Some(context.memory(|memory| -> LayerHit {
        let areas = memory.areas().clone();
        let transforms = memory.to_global.clone();
        let blocked = memory
            .layer_ids()
            .filter(|layer| !memory.allows_interaction(*layer))
            .collect::<Vec<_>>();
        Box::new(move |position| {
            let layer = areas.layer_id_at(position, &transforms)?;
            if blocked.contains(&layer) {
                return None;
            }
            let local = transforms
                .get(&layer)
                .map_or(position, |transform| transform.inverse() * position);
            Some((layer, local))
        })
    }))
}

fn raw_axes(axes: [bool; 2], modifiers: Modifiers, options: InputOptions) -> [bool; 2] {
    // egui can remap both raw components onto a single axis before smoothing.
    match (
        modifiers.matches_any(options.horizontal_scroll_modifier),
        modifiers.matches_any(options.vertical_scroll_modifier),
    ) {
        (true, false) => [axes[0]; 2],
        (false, true) => [axes[1]; 2],
        _ => axes,
    }
}

pub(super) fn capture(context: &mut RenderContext<'_, '_, '_>) {
    // Native backends already determine wheel authority in their input journal.
    #[cfg(feature = "native-render-support")]
    if context.pointer_authority == super::PointerActionAuthority::ExternalJournal
        || context.scroll_registrar.is_some()
    {
        return;
    }

    let ui = &*context.ui;
    let id = ui.make_persistent_id((
        context.instance_id,
        "tab-chrome-wheel",
        context.plan.surface(),
    ));
    let pass = ui.ctx().cumulative_pass_nr();
    let routing = ui.ctx().plugin_or_default::<WheelRouting>();
    let mut routing = routing.lock();
    let state = routing.viewports.entry(ui.ctx().viewport_id()).or_default();
    if state.pass != pass {
        *state = WheelViewport {
            pass,
            ..Default::default()
        };
    }
    state.advance(ui, pass);

    let mut regions = Vec::new();
    let mut commands = Vec::new();
    let clip = ui.clip_rect();
    if effective_enabled(ui, true)
        && !context.plan.has_manipulation_preview()
        && context.plan.prepare_gesture_cancel().is_none()
    {
        let only_direction = ui.style().always_scroll_the_only_direction;
        let _ = context.plan.visit_surface_program(|step| {
            let (target_id, receiver, axes) = match step {
                Step::RootBegin { root, .. } => {
                    if let Some(frame) = root.contained()
                        && let Some(bounds) = egui_rect(frame.outer_bounds())
                    {
                        regions.push(WheelRegion::ContainedFrame(bounds.intersect(clip)));
                    }
                    return Ok::<_, ()>(());
                }
                Step::PopupBackdrop(backdrop) => (
                    id.with(backdrop.visual_id()),
                    context.plan.receiver_for_tab_list_menu_backdrop(backdrop),
                    [true; 2],
                ),
                Step::PopupMenu { menu, paint } if !paint.is_source_gap() => (
                    id.with(menu.input_id()),
                    context.plan.receiver_for_tab_list_menu_scroll(menu),
                    [true; 2],
                ),
                Step::Operation {
                    operation: Operation::TabBar(bar),
                    paint,
                    ..
                } if !paint.is_source_gap() => (
                    id.with(bar.group_menu_input_id()),
                    context.plan.receiver_for_tab_strip_scroll(bar),
                    [true, only_direction],
                ),
                _ => return Ok::<_, ()>(()),
            };
            let Some(target) = wheel_target(target_id, receiver, axes, clip) else {
                return Ok(());
            };
            for stream in state
                .streams
                .iter_mut()
                .filter(|stream| stream.route.surface == id && stream.route.target == target.id)
            {
                if let Some(action) = prepare(context, step, &target, stream, only_direction) {
                    commands.push((target.id, action));
                }
            }
            regions.push(WheelRegion::Target(target));
            Ok(())
        });
    }
    state.surfaces.retain(|surface| surface.id != id);
    if regions
        .iter()
        .any(|region| matches!(region, WheelRegion::Target(_)))
    {
        if state.layer_hit.is_none() {
            // Direct begin_pass/end_pass also uses this registration.
            state.layer_hit = layer_hit(ui.ctx());
        }
        state.surfaces.push(WheelSurface {
            id,
            layer: ui.layer_id(),
            regions,
            widgets: context
                .receivers
                .iter()
                .filter(|binding| binding.interactive)
                .map(|binding| (binding.widget_id, binding.interact_rect))
                .collect(),
        });
    }
    let repaint = state
        .streams
        .iter()
        .any(|stream| stream.route.surface == id && stream.input.is_scrolling());
    drop(routing);
    if repaint || !commands.is_empty() {
        context.ui.ctx().request_repaint();
    }
    for (origin, action) in commands {
        context.push_local_action(origin, action);
    }
}

fn wheel_target(
    id: Id,
    receiver: Option<DockspaceReceiverDescriptor>,
    axes: [bool; 2],
    clip: Rect,
) -> Option<WheelTarget> {
    let receiver = receiver?;
    let bounds = egui_rect(receiver.bounds())?;
    let hit = bounds.intersect(clip);
    hit.is_positive().then_some(WheelTarget {
        id,
        bounds,
        hit,
        axes,
        ordered: receiver.role() == DockspaceReceiverRole::TabListMenuScroll,
    })
}

fn prepare(
    context: &RenderContext<'_, '_, '_>,
    step: Step<'_>,
    target: &WheelTarget,
    stream: &mut WheelStream,
    only_direction: bool,
) -> Option<PreparedSurfaceAction> {
    let mut samples = stream.samples.drain(..);
    if samples.as_slice().is_empty() || samples.as_slice().iter().any(|delta| !delta.is_finite()) {
        return None;
    }
    let point = target.bounds.min + stream.route.offset;
    if !contains(target.hit, point) {
        return None;
    }
    let point = LogicalPoint::new(f64::from(point.x), f64::from(point.y)).ok()?;
    let amount = |delta: Vec2, axis| {
        if only_direction {
            -f64::from(delta.x) - f64::from(delta.y)
        } else {
            -f64::from(delta[axis])
        }
    };
    match step {
        Step::PopupMenu { menu, .. } if samples.len() > 1 => {
            let receiver = context.plan.receiver_for_tab_list_menu_scroll(menu)?;
            receiver.bounds().contains(point).then_some(())?;
            context
                .plan
                .prepare_tab_list_menu_scroll_sequence(menu, samples.map(|delta| amount(delta, 1)))
        }
        Step::PopupMenu { menu, .. } => {
            context
                .plan
                .prepare_tab_list_menu_scroll_at(menu, point, amount(samples.next()?, 1))
        }
        Step::Operation {
            operation: Operation::TabBar(bar),
            ..
        } => context
            .plan
            .prepare_tab_strip_scroll_at(bar, point, amount(samples.next()?, 0)),
        _ => None,
    }
}
