//! Framework focus sampling and exact pending-observation ownership.

mod pending_observations;
mod tab_intent;

pub(crate) use pending_observations::PendingFocusObservations;
pub(crate) use tab_intent::DeferredTabFocus;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod input_tests;

use dockspace::model::{DockspaceSurfaceView, ItemId};
use dockspace::runtime::{
    DockspaceHostFocus, DockspacePaneFocusObservation, DockspacePaneFocusRequest,
};
use egui::{Context, Id, Rect, Ui};

use crate::pane::{PaneFocusState, PaneView};

#[derive(Clone, Copy)]
struct FrameworkFocusRequest {
    request: DockspacePaneFocusRequest,
    applied: bool,
    tab_target: Option<Id>,
    observation: Option<DockspacePaneFocusObservation>,
    wake_frame: Option<u64>,
}

fn with_framework_focus<T>(
    context: &Context,
    instance: Id,
    viewport: egui::ViewportId,
    request: DockspacePaneFocusRequest,
    update: impl FnOnce(&mut FrameworkFocusRequest) -> T,
) -> T {
    let key = Id::unique((instance, "applied-pane-focus", request.surface(), viewport));
    context.data_mut(|data| {
        let state = data.get_temp_mut_or_insert_with(key, || FrameworkFocusRequest {
            request,
            applied: false,
            tab_target: None,
            observation: None,
            wake_frame: None,
        });
        if state.request != request {
            *state = FrameworkFocusRequest {
                request,
                applied: false,
                tab_target: None,
                observation: None,
                wake_frame: None,
            };
        }
        update(state)
    })
}

/// A framework focus side effect is applied once, independently of renderer acknowledgement.
/// The real focus observation still accompanies each exact output until core settles it.
/// `focus_before_contents` preserves a later application focus choice made while
/// registering the target; a pointer click may legitimately clear the old focus.
/// An accepted accessibility focus in this input batch supersedes an unapplied request.
pub(crate) fn apply_pane_focus_request(
    context: &Context,
    instance: Id,
    request: DockspacePaneFocusRequest,
    target: Id,
    focus_before_contents: Option<Id>,
) -> bool {
    let focused = context.memory(egui::Memory::focused);
    let explicit_focus = explicitly_focused_elsewhere(context, focused, Some(target));
    let apply = with_framework_focus(context, instance, context.viewport_id(), request, |state| {
        !std::mem::replace(&mut state.applied, true)
            // egui applies explicit accessibility focus before this callback.
            // Consume the old side effect without replacing that newer choice.
            && !explicit_focus
            && (focused.is_none() || focused == focus_before_contents || focused == Some(target))
            && state
                .tab_target
                .take()
                .is_none_or(|tab| focus_before_contents == Some(tab))
    });
    if apply {
        context.memory_mut(|memory| {
            if !memory.has_focus(target) {
                memory.request_focus(target);
            }
        });
    }
    apply
}

pub(crate) fn pane_focus_handoff_pending(
    context: &Context,
    instance: Id,
    request: DockspacePaneFocusRequest,
    target: Option<Id>,
    focused: Option<Id>,
) -> bool {
    !explicitly_focused_elsewhere(context, focused, target)
        && with_framework_focus(context, instance, context.viewport_id(), request, |state| {
            !state.applied && state.tab_target.is_none_or(|tab| focused == Some(tab))
        })
}

/// Settles the framework side effect after the user has surrendered the source focus.
pub(crate) fn cancel_pane_focus_request(
    context: &Context,
    instance: Id,
    request: DockspacePaneFocusRequest,
) {
    with_framework_focus(context, instance, context.viewport_id(), request, |state| {
        state.applied = true;
    });
}

fn explicitly_focused_elsewhere(
    context: &Context,
    focused: Option<Id>,
    target: Option<Id>,
) -> bool {
    let key = Id::unique(("dockspace-explicit-accesskit-focus", context.viewport_id()));
    let frame = context.cumulative_frame_nr();
    let requested_in_raw = context.input(|input| {
        input.raw.events.iter().rev().find_map(|event| {
            if let egui::Event::AccessKitActionRequest(request) = event
                && request.action == egui::accesskit::Action::Focus
                && request.target_tree == egui::accesskit::TreeId::ROOT
                && request.data.is_none()
            {
                Some(request.target_node)
            } else {
                None
            }
        })
    });
    let active_request = if let Some(node) = requested_in_raw {
        context.data_mut(|data| data.insert_temp(key, (frame, node)));
        Some(node)
    } else {
        context.data(|data| {
            data.get_temp::<(u64, egui::accesskit::NodeId)>(key)
                .filter(|(recorded, _)| *recorded == frame)
                .map(|(_, node)| node)
        })
    };
    focused
        .filter(|focused| Some(*focused) != target)
        .is_some_and(|focused| active_request == Some(focused.accesskit_id()))
}

/// Pointer chrome may clear a target during its own widget registration.
/// Ordinary keyboard and IME frames must receive focus before content runs.
pub(crate) fn input_may_surrender_focus(context: &Context) -> bool {
    let surrender = context.options(|options| options.input_options.surrender_focus_on);
    context.input(|input| match surrender {
        egui::SurrenderFocusOn::Presses => input.pointer.any_pressed(),
        egui::SurrenderFocusOn::Clicks => input.pointer.any_click(),
        egui::SurrenderFocusOn::Never => false,
    })
}

pub(crate) fn pane_registered_focus_target(
    ui: &Ui,
    clip: Rect,
    target: Id,
    previous: bool,
) -> bool {
    ui.ctx().viewport(|viewport| {
        let widgets = if previous {
            &viewport.prev_pass.widgets
        } else {
            &viewport.this_pass.widgets
        };
        let Some(widget) = widgets.get(target) else {
            return false;
        };
        if !widget.enabled
            || !widget.visible
            || !widget.sense.is_focusable()
            || widget.layer_id != ui.layer_id()
            || !widget.rect.is_finite()
            || !widget.rect.is_positive()
            || !widget.interact_rect.is_finite()
            || !widget.interact_rect.is_positive()
            || !clip.contains_rect(widget.interact_rect)
        {
            return false;
        }

        widget_belongs_to_pane(widgets, target, ui.unique_id())
    })
}

pub(crate) fn widget_belongs_to_pane(widgets: &egui::WidgetRects, mut id: Id, pane: Id) -> bool {
    // Parents register first. This proves ancestry even inside a clipped
    // scroll area, and an ID collision cannot form a cycle.
    while id != pane {
        let (Some(widget), Some((layer, order))) = (widgets.get(id), widgets.order(id)) else {
            return false;
        };
        let Some((parent_layer, parent_order)) = widgets.order(widget.parent_id) else {
            return false;
        };
        if layer != parent_layer || parent_order >= order {
            return false;
        }
        id = widget.parent_id;
    }
    true
}

impl DeferredTabFocus {
    /// Hands a newly committed pane intent off from this run's ordered tab input.
    ///
    /// Content may take focus only while the original tab still owns framework
    /// focus. This neither applies focus nor acknowledges renderer presentation.
    pub(crate) fn handoff_after_commit(
        &self,
        context: &Context,
        instance: Id,
        viewport: egui::ViewportId,
        previous: Option<DockspacePaneFocusRequest>,
        committed: Option<DockspacePaneFocusRequest>,
    ) {
        let Some(request) = committed.filter(|request| Some(*request) != previous) else {
            return;
        };
        if !self.handled() {
            return;
        }
        with_framework_focus(context, instance, viewport, request, |state| {
            state.applied = self.target().is_none();
            state.tab_target = self.target();
        });
    }
}

/// Repeated observations remain true facts without creating an idle repaint loop.
pub(crate) fn focus_observation_needs_repaint(
    context: &Context,
    instance: Id,
    request: DockspacePaneFocusRequest,
    observation: DockspacePaneFocusObservation,
) -> bool {
    let frame = context.cumulative_frame_nr();
    with_framework_focus(context, instance, context.viewport_id(), request, |state| {
        if state.observation != Some(observation) {
            state.observation = Some(observation);
            state.wake_frame = Some(frame);
        }
        state.wake_frame == Some(frame)
    })
}

#[derive(Default)]
struct PaneFocusEvidence {
    focused: Option<ItemId>,
    ambiguous: bool,
}

/// Samples actual focus in the current viewport without consulting focus history.
///
/// Call once in the matching viewport's input phase, using the current surface
/// before close/reset actions or encoding. The caller must validate the native
/// route and presentation scope; widget IDs alone do not establish that scope.
/// Submit pending exact acknowledgements, report this sampled fact, then submit
/// UI/application actions. This function never requests or consumes focus.
///
/// Exact-target claims require an enabled, visible, focusable widget in this
/// viewport's previous-pass registry. Composite `PaneView::focus_state` overrides
/// remain authoritative when they claim focus without that exact target.
/// Unknown pane states do not invent a claim or invalidate another pane's
/// uniquely proven claim. Multiple or unprovable positive claims stay Unknown.
/// `retained_focus` may contain only registrations owned by the exact output
/// currently presented at this viewport's input endpoint. It grants no focus
/// without the live memory focus and current pane ownership checks.
///
/// `workspace_id` must match the ID used by the surface renderer. `dock_bounds`
/// must be expressed in viewport-global coordinates and enclose the workspace.
/// Only a registered focused widget fully outside these bounds proves External;
/// unknown widgets within the workspace stay Unknown. Layer transforms are
/// applied before comparing widget bounds.
#[doc(hidden)]
pub fn sample_host_focus(
    context: &Context,
    surface: DockspaceSurfaceView<'_>,
    panes: &dyn PaneView,
    workspace_id: Id,
    dock_bounds: Rect,
    retained_focus: &[(ItemId, Id)],
) -> DockspaceHostFocus {
    if !dock_bounds.is_finite()
        || !dock_bounds.is_positive()
        || !context.input(|input| input.focused)
    {
        return DockspaceHostFocus::Unknown;
    }

    let focused_id = context.memory(|memory| memory.focused());
    let mut evidence = PaneFocusEvidence::default();
    for item in surface.active_items() {
        collect_pane_focus(
            item,
            context,
            panes,
            focused_id,
            retained_focus,
            &mut evidence,
        );
    }

    if evidence.ambiguous {
        return DockspaceHostFocus::Unknown;
    }

    if focused_id == Some(workspace_id) {
        if evidence.focused.is_some() {
            return DockspaceHostFocus::Unknown;
        }
        return registered_widget_bounds(context, workspace_id)
            .map_or(DockspaceHostFocus::Unknown, |_| {
                DockspaceHostFocus::Workspace
            });
    }

    if let Some(item) = evidence.focused {
        return DockspaceHostFocus::Pane(item);
    }

    if let Some(id) = focused_id {
        let Some(bounds) = registered_widget_bounds(context, id) else {
            return DockspaceHostFocus::Unknown;
        };
        return if !bounds.intersects(dock_bounds) {
            DockspaceHostFocus::External
        } else {
            DockspaceHostFocus::Unknown
        };
    }

    DockspaceHostFocus::None
}

/// Keeps an already focused, exactly presented target alive during a skipped UI.
///
/// This registers no hit receiver and makes no focus request. Call only when
/// retaining that output without invoking its pane callbacks in this pass.
#[doc(hidden)]
pub fn retain_presented_pane_focus(context: &Context, retained_focus: &[(ItemId, Id)]) {
    let Some(id) = context.memory(|memory| memory.focused()) else {
        return;
    };
    if retained_focus.iter().any(|(_, target)| *target == id) {
        // egui's focus dead-man switch uses state IDs, independently of widget
        // hit registration. The original output still owns this state identity.
        context.check_for_id_clash(id, Rect::ZERO, "retained dockspace pane focus");
    }
}

/// Keeps an active pane's existing focus identity alive while its UI is skipped.
///
/// The caller must validate the current surface's viewport and native lifetime.
/// This neither requests focus nor supplies presentation evidence to focus sampling.
#[doc(hidden)]
#[cfg(any(test, feature = "native-render-support"))]
pub fn retain_skipped_pane_focus(
    context: &Context,
    surface: DockspaceSurfaceView<'_>,
    panes: &dyn PaneView,
) {
    let Some(id) = context.memory(|memory| memory.focused()) else {
        return;
    };
    if surface
        .active_items()
        .any(|item| panes.focus_target(item) == Some(id))
    {
        context.check_for_id_clash(id, Rect::ZERO, "skipped dockspace pane focus");
    }
}

/// Keeps existing focus and queued editor input while the exact surface skips painting.
#[doc(hidden)]
#[cfg(any(test, feature = "native-render-support"))]
pub fn retain_skipped_surface_input_state(
    context: &Context,
    instance: Id,
    surface: DockspaceSurfaceView<'_>,
    panes: &dyn PaneView,
) {
    if !crate::render::retain_skipped_pane_keyboard(context, instance, surface.id()) {
        retain_skipped_pane_focus(context, surface, panes);
    }
}

pub(crate) fn retain_presented_surface_input_state(
    context: &Context,
    instance: Id,
    surface: dockspace::model::SurfaceId,
    retained_focus: &[(ItemId, Id)],
) {
    if !crate::render::retain_skipped_pane_keyboard(context, instance, surface) {
        retain_presented_pane_focus(context, retained_focus);
    }
}

fn collect_pane_focus(
    item: ItemId,
    context: &Context,
    panes: &dyn PaneView,
    focused_id: Option<Id>,
    retained_focus: &[(ItemId, Id)],
    evidence: &mut PaneFocusEvidence,
) {
    if panes.focus_state(item, context) == PaneFocusState::Focused {
        if panes.focus_target(item).is_some_and(|target| {
            focused_id == Some(target)
                && registered_widget_bounds(context, target).is_none()
                && !retained_focus.contains(&(item, target))
        }) {
            evidence.ambiguous = true;
        } else if evidence.focused.replace(item).is_some() {
            evidence.ambiguous = true;
        }
    }
}

fn registered_widget_bounds(context: &Context, id: Id) -> Option<Rect> {
    let widget = context.viewport(|viewport| viewport.prev_pass.widgets.get(id).copied())?;
    if !widget.enabled
        || !widget.visible
        || !widget.sense.is_focusable()
        || !widget.rect.is_finite()
        || !widget.rect.is_positive()
    {
        return None;
    }

    let bounds = context
        .layer_transform_to_global(widget.layer_id)
        .map_or(widget.rect, |transform| transform * widget.rect);
    (bounds.is_finite() && bounds.is_positive()).then_some(bounds)
}
