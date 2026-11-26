//! Receiver evidence from the matching egui pass and presented Dockspace output.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::time::Duration;

use dockspace::geometry::LogicalPoint;
use dockspace::model::{ItemId, SurfaceId, TabGroupDockMode};
use dockspace::runtime::{
    DockspaceHostFrame, DockspacePointerContinuation, DockspaceRuntimeError, HostPointerInput,
    HostPointerRoute, PointerDeviceKind, PointerEventMetadata, PresentedDockReceiver,
    PresentedDockspaceSurface, SurfacePointerButton, SurfacePointerCapture, SurfacePointerEvent,
    SurfacePointerId, SurfacePointerReceiverFacts, SurfacePresentationArtifact,
    SurfacePresentationCommit,
};
use egui::{AreaState, Context, Id, LayerId, Pos2, Rect, Ui, WidgetRect, WidgetRects};

use crate::focus::DeferredTabFocus;
use crate::pass_settlement::{InputPassId, ProductPointerPacket};
use crate::render::ProductReceiverBinding;

use super::stream::{PendingPointerRecovery, PointerSample, PointerStreams, ends_primary_gesture};

#[cfg(test)]
mod ownership_tests;

/// Framework receiver authority frozen before any application callback runs.
pub(crate) struct ProductPointerInputSnapshot {
    widgets: WidgetRects,
    interactive_layers: Vec<(LayerId, Rect, f32)>,
    interaction_radius: f32,
    transformed_layers: HashSet<LayerId>,
    layer_at: Box<dyn Fn(Pos2) -> Option<LayerId> + Send + Sync>,
    pub(crate) foreign_drag: bool,
}

/// Metadata pairing one committed egui output with its new receiver graph.
///
/// Preceding output observations settle before this pass's graph is queued.
/// Keeping that ordering in one private helper prevents the callback and
/// owned-run facades from drifting apart while their focus and repaint
/// lifecycles remain intentionally separate.
pub(crate) struct ProductPointerPresentation {
    layer: LayerId,
    input_pass: Option<InputPassId>,
    pointer_clip: Option<Rect>,
    pub(crate) receivers: Vec<ProductReceiverBinding>,
    pub(crate) focus_bindings: Vec<(ItemId, Id)>,
}

impl ProductPointerPresentation {
    pub(crate) fn new(ui: &Ui, input_pass: Option<InputPassId>) -> Self {
        let layer = ui.layer_id();
        let pointer_clip = ui
            .ctx()
            .layer_transform_to_global(layer)
            .is_none_or(|transform| transform == egui::emath::TSTransform::IDENTITY)
            .then(|| ui.clip_rect());
        Self {
            layer,
            input_pass,
            pointer_clip,
            receivers: Vec::new(),
            focus_bindings: Vec::new(),
        }
    }
}

fn pointer_clip_contains(clip: Rect, point: Pos2) -> bool {
    clip.is_positive() && clip.contains(point)
}

impl ProductPointerInputSnapshot {
    pub(crate) fn capture(context: &Context) -> Self {
        let widgets = context.viewport(|viewport| viewport.prev_pass.widgets.clone());
        let interaction_radius = context.global_style().interaction.interact_radius;
        // egui excludes click-through areas from its widget hit test even
        // when their children register interactive senses.
        let click_through_layers = widgets
            .layer_ids()
            .filter(|layer| {
                AreaState::load(context, layer.id).is_some_and(|area| !area.interactable)
            })
            .collect::<HashSet<_>>();
        let (transformed_layers, layer_at, interactive_layers) = context.memory(|memory| {
            let areas = memory.areas().clone();
            let transforms = memory.to_global.clone();
            let interactive_layers = widgets
                .layers()
                .filter(|(layer, _)| {
                    !click_through_layers.contains(layer) && memory.allows_interaction(**layer)
                })
                .flat_map(|(layer, widgets)| {
                    let transforms = &transforms;
                    widgets.iter().filter_map(move |widget| {
                        if !widget.enabled || !widget.sense.interactive() {
                            return None;
                        }
                        let rect = transforms
                            .get(layer)
                            .map_or(widget.interact_rect, |transform| {
                                *transform * widget.interact_rect
                            });
                        let radius = interaction_radius
                            * transforms
                                .get(layer)
                                .map_or(1.0, |transform| transform.scaling.abs());
                        (rect.is_finite() && rect.is_positive()).then_some((
                            *layer,
                            rect,
                            radius * radius,
                        ))
                    })
                })
                .collect();
            let transformed_layers = transforms
                .iter()
                .filter(|(_, transform)| **transform != egui::emath::TSTransform::IDENTITY)
                .map(|(layer, _)| *layer)
                .collect();
            let blocked = memory
                .layer_ids()
                .filter(|layer| !memory.is_above_modal_layer(*layer))
                .collect::<HashSet<_>>();
            let modal = memory.top_modal_layer();
            // Erase the private Areas type while using egui's own hit test.
            // Capturing Memory itself would retain unrelated user data and the run portal.
            let layer_at: Box<dyn Fn(Pos2) -> Option<LayerId> + Send + Sync> =
                Box::new(move |position| {
                    let layer = areas.layer_id_at(position, &transforms)?;
                    if blocked.contains(&layer) {
                        modal
                    } else {
                        Some(layer)
                    }
                });
            (transformed_layers, layer_at, interactive_layers)
        });
        Self {
            widgets,
            interactive_layers,
            interaction_radius,
            transformed_layers,
            layer_at,
            foreign_drag: has_foreign_drag(context),
        }
    }

    fn identity_layer(&self, layer: LayerId) -> bool {
        !self.transformed_layers.contains(&layer)
    }

    /// Joins exact registrations without discarding their framework paint order.
    fn ownership(&self, scene: &PointerScene) -> PacketPointerOwnership {
        #[cfg(test)]
        ownership_tests::record_join(
            scene.receivers.len(),
            self.widgets
                .get_layer(scene.layer)
                .filter(|widget| widget.enabled && widget.sense.interactive())
                .count(),
        );

        let owned = scene
            .receivers
            .iter()
            .filter(|binding| {
                binding.interactive
                    && binding.layer_id == scene.layer
                    && self.widgets.get(binding.widget_id).is_some_and(|widget| {
                        widget.enabled
                            && widget.sense.interactive()
                            && widget.layer_id == binding.layer_id
                            && widget.interact_rect == binding.interact_rect
                    })
            })
            .map(|binding| binding.widget_id)
            .collect::<HashSet<_>>();
        PacketPointerOwnership {
            interaction_radius_sq: self.interaction_radius * self.interaction_radius,
            foreign_layers: self
                .interactive_layers
                .iter()
                .filter(|(layer, _, _)| *layer != scene.layer)
                .map(|(_, rect, radius_sq)| (*rect, *radius_sq))
                .collect(),
            widgets: self
                .widgets
                .get_layer(scene.layer)
                .filter(|widget| widget.enabled && widget.sense.interactive())
                .map(|widget| (*widget, owned.contains(&widget.id)))
                .collect(),
        }
    }
}

struct PacketPointerOwnership {
    interaction_radius_sq: f32,
    foreign_layers: Vec<(Rect, f32)>,
    widgets: Vec<(WidgetRect, bool)>,
}

impl PacketPointerOwnership {
    fn is_clear_at(&self, position: Pos2) -> bool {
        // A custom Ui can draw on another layer without registering an Area.
        // The layer-at-point check alone cannot prove that layer is absent.
        if self
            .foreign_layers
            .iter()
            .any(|(rect, radius_sq)| rect.distance_sq_to_pos(position) <= *radius_sq)
        {
            return false;
        }
        let mut top_owned_click = None;
        let mut top_owned_drag = None;
        let mut nearby_unowned = Vec::new();
        for (widget, owned) in self.widgets.iter().rev() {
            if widget.interact_rect.distance_sq_to_pos(position) > self.interaction_radius_sq {
                continue;
            }
            if !owned {
                if widget.interact_rect.contains(position) {
                    return false;
                }
                nearby_unowned.push(*widget);
                continue;
            }
            // A direct owner of both lanes shields the widgets behind it.
            if widget.interact_rect.contains(position) {
                if widget.sense.senses_click() && widget.sense.senses_drag() {
                    return true;
                }
                if widget.sense.senses_click() && top_owned_click.is_none() {
                    top_owned_click = Some(*widget);
                }
                if widget.sense.senses_drag() && top_owned_drag.is_none() {
                    top_owned_drag = Some(*widget);
                }
            }
        }
        if nearby_unowned.is_empty() {
            return true;
        }
        match (top_owned_click, top_owned_drag) {
            (Some(_), _) => true,
            (None, Some(hit_drag)) => !nearby_unowned
                .iter()
                .any(|unowned| hit_drag.interact_rect.contains_rect(unowned.interact_rect)),
            (None, None) => false,
        }
    }
}

struct PointerScene {
    artifact: SurfacePresentationArtifact,
    layer: LayerId,
    input_pass: Option<InputPassId>,
    pointer_clip: Option<Rect>,
    receivers: Vec<ProductReceiverBinding>,
    focus_bindings: Vec<(ItemId, Id)>,
}

#[derive(Default)]
pub(super) struct PointerScenes {
    pending: Vec<PointerScene>,
    presented: Option<PointerScene>,
}

impl PointerScenes {
    pub(super) fn emitted(
        &mut self,
        artifact: SurfacePresentationArtifact,
        presentation: ProductPointerPresentation,
    ) {
        self.pending.push(PointerScene {
            artifact,
            layer: presentation.layer,
            input_pass: presentation.input_pass,
            pointer_clip: presentation.pointer_clip,
            receivers: presentation.receivers,
            focus_bindings: presentation.focus_bindings,
        });
    }

    pub(super) fn observe(&mut self, commits: &[SurfacePresentationCommit]) {
        for scene in self.pending.extract_if(.., |scene| {
            commits.iter().any(|commit| commit.settles(scene.artifact))
        }) {
            if commits.iter().any(|commit| commit.presents(scene.artifact)) {
                self.presented = Some(scene);
            }
        }
    }

    pub(super) fn focus_bindings(
        &self,
        current: Option<PresentedDockspaceSurface>,
    ) -> &[(ItemId, Id)] {
        self.for_output(current)
            .map_or(&[], |scene| &scene.focus_bindings)
    }

    fn for_output(&self, current: Option<PresentedDockspaceSurface>) -> Option<&PointerScene> {
        let current = current?;
        // The host frame can see a newly accepted artifact before the public report settles it.
        self.presented
            .iter()
            .chain(&self.pending)
            .find(|scene| current.matches_presentation_artifact(scene.artifact))
    }
}

/// Exact binding and framework evidence for all edges in one original run.
pub(super) struct PointerReceivers<'a> {
    packet: &'a ProductPointerPacket,
    current: Option<PresentedDockspaceSurface>,
    scene: Option<&'a PointerScene>,
    input_scene: Option<&'a PointerScene>,
    ownership: OnceCell<PacketPointerOwnership>,
    ui_enabled: bool,
    coordinates_supported: bool,
    foreign_drag: bool,
}

impl<'a> PointerReceivers<'a> {
    pub(super) fn new(
        scenes: &'a PointerScenes,
        current: Option<PresentedDockspaceSurface>,
        ui: &Ui,
        packet: &'a ProductPointerPacket,
    ) -> Self {
        let scene = scenes.for_output(current);
        let snapshot = packet.receivers();
        let identity_layer = |layer| {
            snapshot.identity_layer(layer)
                && ui
                    .ctx()
                    .layer_transform_to_global(layer)
                    .is_none_or(|transform| transform == egui::emath::TSTransform::IDENTITY)
        };
        let coordinates_supported = identity_layer(ui.layer_id())
            && scene
                .is_none_or(|scene| scene.pointer_clip.is_some() && identity_layer(scene.layer));
        let input_scene = scene.filter(|scene| {
            coordinates_supported
                && scene.input_pass.is_some()
                && scene.input_pass == packet.preceding_input_pass()
        });
        // Keep the original ownership even if application code consumes a
        // foreign payload. Also honor a source started before this call.
        let foreign_drag = snapshot.foreign_drag || has_foreign_drag(ui.ctx());
        Self {
            packet,
            current,
            scene,
            input_scene,
            ownership: OnceCell::new(),
            ui_enabled: ui.is_enabled(),
            coordinates_supported,
            foreign_drag,
        }
    }

    pub(super) const fn coordinates_supported(&self) -> bool {
        self.coordinates_supported
    }

    pub(super) const fn foreign_drag(&self) -> bool {
        self.foreign_drag
    }

    fn is_clear_at(&self, scene: &PointerScene, position: Pos2) -> bool {
        let snapshot = self.packet.receivers();
        self.ui_enabled
            && scene
                .pointer_clip
                .is_some_and(|clip| pointer_clip_contains(clip, position))
            && (snapshot.layer_at)(position) == Some(scene.layer)
            && self
                .ownership
                .get_or_init(|| snapshot.ownership(scene))
                .is_clear_at(position)
    }

    pub(super) fn stationary_blocker(
        &self,
        frame: &DockspaceHostFrame<'_>,
        surface: SurfaceId,
        streams: &PointerStreams,
        recovery: &PendingPointerRecovery,
        events: &[PointerSample],
    ) -> Option<PointerSample> {
        let scene = self.input_scene?;
        if !self.packet.focused() || self.foreign_drag {
            return None;
        }
        let (pointer, retained_point) = frame.host_pointer_hover_position(surface)?;
        if !recovery.retained_position_is_current(pointer)
            || events.iter().any(|sample| {
                sample.pointer == pointer
                    && (sample.event == SurfacePointerEvent::Moved
                        || ends_primary_gesture(sample.event))
            })
        {
            return None;
        }
        let sample = streams.sample(pointer, SurfacePointerEvent::Moved)?;
        let position = sample.position?;
        // Revoke only from matching, newly observed blocking evidence.
        (logical_point(position) == Some(retained_point) && !self.is_clear_at(scene, position))
            .then_some(sample)
    }

    pub(super) fn submit(
        &self,
        frame: &mut DockspaceHostFrame<'_>,
        surface: SurfaceId,
        samples: impl IntoIterator<Item = PointerSample>,
        streams: &PointerStreams,
        focused: bool,
    ) -> Result<DeferredTabFocus, DockspaceRuntimeError> {
        let observed_at = self.packet.observed_at();
        let prepared = self.prepare(frame, surface, samples, streams, focused);
        if !prepared.edges.is_empty() {
            frame.submit_host_pointer_batch(
                prepared
                    .edges
                    .iter()
                    .map(|edge| pointer_input(edge, observed_at)),
            )?;
        }
        if let Some(probe) = prepared.continuation
            && let Some(request) = frame.pointer_continuation()
            && request.pointer() == probe.request.pointer()
            && request.surface() == probe.request.surface()
            && request.deadline() == probe.request.deadline()
            && request.position() == probe.request.position()
            && probe.edge.surface == frame.presented_surface(request.surface())
        {
            // Raw input can revoke or refresh the timer. Reuse only the exact
            // pre-input proof at the same point and output, with the current token.
            let edge = Edge {
                event: request.event(),
                ..probe.edge
            };
            frame.submit_host_pointer(pointer_input(&edge, observed_at))?;
        }
        Ok(prepared.deferred_focus)
    }

    fn prepare(
        &self,
        frame: &DockspaceHostFrame<'_>,
        surface: SurfaceId,
        samples: impl IntoIterator<Item = PointerSample>,
        streams: &PointerStreams,
        focused: bool,
    ) -> PreparedPointerEdges {
        let mut retain_frame_owner = true;
        let mut edge_at =
            |sample: PointerSample| self.edge_at(frame, surface, sample, &mut retain_frame_owner);
        let edges = samples.into_iter().map(&mut edge_at).collect::<Vec<_>>();
        let continuation = frame.pointer_continuation().and_then(|request| {
            if !focused
                || request.surface() != surface
                || !self
                    .packet
                    .observed_at()
                    .is_some_and(|now| now >= request.deadline())
            {
                return None;
            }
            let edge = edge_at(streams.sample(request.pointer(), request.event())?);
            (!edge.blocked
                && edge.point == Some(request.position())
                && edge.surface.is_some()
                && edge.surface == self.current)
                .then_some(QualifiedPointerContinuation { request, edge })
        });
        let mut deferred_focus = DeferredTabFocus::default();
        if let Some(scene) = self.scene {
            for edge in &edges {
                if !matches!(
                    edge.event,
                    SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary)
                ) {
                    continue;
                }
                let target = if edge.device == PointerDeviceKind::Unknown
                    && let Some(receiver) = edge.delivery
                    && receiver.descriptor().role()
                        == dockspace::runtime::DockspaceReceiverRole::TabBody
                    && let Some(binding) = scene
                        .receivers
                        .iter()
                        .find(|binding| binding.receiver == receiver.descriptor())
                {
                    Some(binding.widget_id)
                } else {
                    None
                };
                deferred_focus.observe_primary_press(target);
            }
        }
        PreparedPointerEdges {
            edges,
            continuation,
            deferred_focus,
        }
    }

    fn edge_at(
        &self,
        frame: &DockspaceHostFrame<'_>,
        surface: SurfaceId,
        sample: PointerSample,
        retain_frame_owner: &mut bool,
    ) -> Edge {
        let capture = if self.foreign_drag {
            SurfacePointerCapture::Foreign
        } else {
            SurfacePointerCapture::None
        };
        let mut edge = Edge {
            pointer: sample.pointer,
            device: sample.device,
            event: sample.event,
            point: None,
            surface: None,
            manipulation_surface: None,
            delivery: None,
            drag_delivery: None,
            blocked: false,
            docking_enabled: sample.docking_enabled,
            group_dock_mode: sample.group_dock_mode,
            capture,
        };
        if matches!(
            sample.event,
            SurfacePointerEvent::DockingEnabledChanged { .. }
                | SurfacePointerEvent::GroupDockModeChanged { .. }
        ) {
            // Modifier markers carry no coordinate or receiver observation.
            return edge;
        }
        edge.point = sample
            .position
            .filter(|_| self.coordinates_supported)
            .and_then(logical_point);
        let hit = edge
            .point
            .and_then(|point| frame.host_pointer_hit_for_device(surface, point, sample.device));
        let frame_owner = frame
            .host_pointer_manipulation_surface(sample.pointer)
            .filter(|source| self.coordinates_supported && source.surface() == surface);
        if frame_owner.is_some()
            && sample.event == SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary)
        {
            *retain_frame_owner = false;
        }
        edge.manipulation_surface = frame_owner
            .filter(|_| *retain_frame_owner)
            .or(self.current.filter(|_| self.coordinates_supported));
        if frame_owner.is_some() && ends_primary_gesture(sample.event) {
            // Its terminal ends the source's old coordinate proof within this packet.
            *retain_frame_owner = false;
        }
        edge.surface = hit.map(|hit| hit.surface());
        edge.blocked = self.foreign_drag
            || self
                .input_scene
                .zip(sample.position)
                .is_none_or(|(scene, position)| !self.is_clear_at(scene, position));
        if !edge.blocked {
            edge.delivery = hit.and_then(|hit| hit.delivery());
            edge.drag_delivery = hit.and_then(|hit| hit.drag_delivery());
        }
        edge
    }
}

#[derive(Clone, Copy)]
struct Edge {
    pointer: SurfacePointerId,
    device: PointerDeviceKind,
    event: SurfacePointerEvent,
    point: Option<LogicalPoint>,
    surface: Option<PresentedDockspaceSurface>,
    manipulation_surface: Option<PresentedDockspaceSurface>,
    delivery: Option<PresentedDockReceiver>,
    drag_delivery: Option<PresentedDockReceiver>,
    blocked: bool,
    docking_enabled: Option<bool>,
    group_dock_mode: Option<TabGroupDockMode>,
    capture: SurfacePointerCapture,
}

struct PreparedPointerEdges {
    edges: Vec<Edge>,
    continuation: Option<QualifiedPointerContinuation>,
    deferred_focus: DeferredTabFocus,
}

struct QualifiedPointerContinuation {
    request: DockspacePointerContinuation,
    edge: Edge,
}

fn logical_point(position: Pos2) -> Option<LogicalPoint> {
    LogicalPoint::new(f64::from(position.x), f64::from(position.y)).ok()
}

pub(crate) fn has_foreign_drag(context: &Context) -> bool {
    egui::DragAndDrop::has_any_payload(context)
        || context.input(|input| {
            !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty()
        })
}

fn pointer_input(edge: &Edge, observed_at: Option<Duration>) -> HostPointerInput<'_> {
    let route = edge
        .surface
        .zip(edge.point)
        .map_or(HostPointerRoute::Unknown, |(surface, point)| {
            HostPointerRoute::new(surface, point)
        });
    let manipulation = edge
        .manipulation_surface
        .zip(edge.point)
        .map_or(HostPointerRoute::Unknown, |(surface, point)| {
            HostPointerRoute::new(surface, point)
        });
    let mut facts = SurfacePointerReceiverFacts::unknown();
    if let Some(surface) = edge.surface.as_ref() {
        if edge.blocked {
            facts = facts
                .with_blocked_delivery(surface)
                .with_blocked_hover(surface);
        } else {
            facts = edge.delivery.as_ref().map_or_else(
                || facts.with_empty_delivery_lane(surface),
                |receiver| facts.with_delivery(receiver),
            );
            facts = edge.drag_delivery.as_ref().map_or_else(
                || facts.with_empty_drag_delivery_lane(surface),
                |receiver| facts.with_drag_delivery(receiver),
            );
            facts = facts.with_clear_hover_lane(surface);
        }
    }
    let mut metadata = observed_at.map_or_else(
        || PointerEventMetadata::untimed(edge.device),
        |time| PointerEventMetadata::new(edge.device, time),
    );
    if let Some(enabled) = edge.docking_enabled {
        metadata = metadata.with_docking_enabled(enabled);
    }
    if let Some(mode) = edge.group_dock_mode {
        metadata = metadata.with_group_dock_mode(mode);
    }
    HostPointerInput::new(
        edge.pointer,
        edge.event,
        manipulation,
        route,
        edge.capture,
        facts,
    )
    .with_metadata(metadata)
}
