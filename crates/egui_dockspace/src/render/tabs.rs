//! Pane and tab rendering over opaque product paint records.

use dockspace::runtime::{
    DockspaceControlId, DockspaceRuntimeError, PanePaintRecord, SurfacePaintDisposition,
    TabGroupDragRegionKind, TabPaintRecord, TabsPaintRecord,
};
use egui::accesskit::{Action, Role};
use egui::{
    Align, CursorIcon, EventFilter, Id, Layout, PointerButton, Sense, StrokeKind, Ui, UiBuilder,
    pos2,
};

use crate::DockInputConfig;

use super::actions::effective_enabled;
use super::geometry::{accesskit_bounds, egui_rect, inset_ui_rect};
use super::measurement::TabPaintResource;
use super::{ProductReceiverBinding, RenderContext};

#[derive(Clone, Copy)]
pub(crate) enum RootVisualContext {
    Main,
    Contained { pane_fill: egui::Color32 },
}

impl RootVisualContext {
    const fn pane_fill(self, workspace_fill: egui::Color32) -> egui::Color32 {
        match self {
            Self::Main => workspace_fill,
            Self::Contained { pane_fill, .. } => pane_fill,
        }
    }
}

pub(crate) fn paint_tabs(
    context: &mut RenderContext<'_, '_, '_>,
    tabs: TabsPaintRecord<'_>,
    visual_context: RootVisualContext,
    paint: SurfacePaintDisposition,
) {
    if let Some(receiver) = context.plan.receiver_for_tabs(tabs)
        && let Some(bounds) = egui_rect(receiver.bounds())
    {
        let id = context.ui.make_persistent_id((
            context.instance_id,
            "tabs-background",
            tabs.visual_id(),
        ));
        context.interact_receiver(bounds, id, Sense::click(), Some(receiver));
    }
    if paint.is_source_gap() {
        return;
    }
    let (Some(bounds), Some(clip)) = (egui_rect(tabs.bounds()), egui_rect(tabs.clip())) else {
        return;
    };
    context
        .ui
        .painter_at(context.ui.clip_rect().intersect(clip))
        .rect_filled(
            bounds,
            0.0,
            visual_context.pane_fill(context.visuals.workspace_fill),
        );
}

pub(crate) fn paint_pane(
    context: &mut RenderContext<'_, '_, '_>,
    pane: PanePaintRecord<'_>,
    visual_context: RootVisualContext,
    paint: SurfacePaintDisposition,
) {
    let pane_fill = visual_context.pane_fill(context.visuals.workspace_fill);
    let paint_omitted = paint.is_source_gap();
    let geometry = pane.content_geometry();
    let Some(pane_rect) = egui_rect(geometry.clip()) else {
        return;
    };
    let clip = context.ui.clip_rect().intersect(pane_rect);
    if !paint_omitted {
        context
            .ui
            .painter_at(clip)
            .rect_filled(pane_rect, 0.0, pane_fill);
    }
    let pane_id =
        context
            .ui
            .make_persistent_id((context.instance_id, "pane-body", pane.visual_id()));
    let pane_receiver = context.plan.receiver_for_pane(pane);
    let _response =
        context.interact_receiver(pane_rect, pane_id, Sense::click_and_drag(), pane_receiver);
    let item = pane.item();
    if !pane_rect.is_positive() {
        return;
    }
    let Some(content_rect) = egui_rect(geometry.layout_bounds()).and_then(inset_ui_rect) else {
        return;
    };
    // Content follows the item when docking changes its slot or parent Ui.
    // The pane body above retains the slot's separate receiver identity.
    let mut child = context.ui.new_child(
        UiBuilder::new()
            .scope_id(
                context
                    .instance_id
                    .with(("pane-content", context.plan.content_identity(item))),
            )
            .max_rect(content_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    if paint_omitted {
        // Keep pane widget identities alive so focus and application-owned
        // accessibility state survive the temporary source gap.
        child.set_opacity(0.0);
    }
    child.set_clip_rect(clip);
    let mut paint_audit = context.paint_audit.take();
    let painter = child.painter().clone();
    // Keep ownership outside the callback so an unwind releases the append
    // checkpoint before Ui::drop, while normal finalization remains audited.
    let mut child = Some(child);
    let mut paint_contents = || {
        let child_ui = child.as_mut().expect("the pane child is still open");
        if context.resources.is_missing(item) {
            child_ui.centered_and_justified(|ui| {
                ui.label(format!("Missing pane {}", item.get()));
            });
        } else {
            let focus_request = context
                .pane_focus_request
                .filter(|request| request.item() == Some(item))
                .filter(|_| {
                    (context.pointer_authority.supports_preview_gestures()
                        || context
                            .pane_input
                            .as_ref()
                            .is_some_and(|input| input.targets(item)))
                        && effective_enabled(child_ui, true)
                        && !*context.pane_focus_target_requested
                });
            let mut delivery = super::tab_shortcuts::PaneInputDelivery::take_for(
                &mut context.pane_input,
                child_ui,
                pane,
            );
            let focus_before_contents =
                focus_request.and_then(|_| child_ui.memory(egui::Memory::focused));
            let focus_target = focus_request.and_then(|_| context.panes.focus_target(item));
            let preapplied = match (focus_request, focus_target) {
                (Some(request), Some(target))
                    if !child_ui.ctx().will_discard()
                        && !crate::focus::input_may_surrender_focus(child_ui.ctx())
                        && !super::tab_shortcuts::has_live_keyboard_input(child_ui.ctx())
                        && (crate::focus::pane_registered_focus_target(
                            child_ui, clip, target, true,
                        ) || delivery
                            .as_ref()
                            .is_some_and(|input| input.awaits_registration())) =>
                {
                    // Let the widget initialize its selection while holding
                    // the focus claim until actual post-UI registration.
                    let pending = crate::focus::pane_focus_handoff_pending(
                        child_ui.ctx(),
                        context.instance_id,
                        request,
                        Some(target),
                        focus_before_contents,
                    );
                    if pending {
                        child_ui.memory_mut(|memory| memory.request_focus(target));
                    }
                    pending
                }
                _ => false,
            };
            let replay = if delivery
                .as_ref()
                .is_some_and(|delivery| delivery.can_replay(child_ui, clip))
            {
                delivery
                    .take()
                    .map(|delivery| delivery.replay(context, pane))
            } else {
                None
            };
            let pane_response = context.panes.ui(item, child_ui);
            let retry = delivery
                .as_ref()
                .is_some_and(|input| input.awaits_registration())
                && child_ui.ctx().will_discard();
            let registered_target = context.panes.focus_target(item).filter(|target| {
                !retry && crate::focus::pane_registered_focus_target(child_ui, clip, *target, false)
            });
            if preapplied
                && focus_target != registered_target
                && let Some(target) = focus_target
                && child_ui.memory(|memory| memory.has_focus(target))
            {
                child_ui.memory_mut(|memory| {
                    memory.surrender_focus(target);
                    if let Some(previous) =
                        focus_before_contents.filter(|previous| *previous != target)
                    {
                        memory.request_focus(previous);
                    }
                });
            }
            if let Some(target) = registered_target {
                context.pane_focus_bindings.push((item, target));
                if let Some(request) = focus_request {
                    // Register content first: egui surrenders a target's focus
                    // when a title/tab-group click lands outside that widget.
                    crate::focus::apply_pane_focus_request(
                        context.ui.ctx(),
                        context.instance_id,
                        request,
                        target,
                        focus_before_contents,
                    );
                    *context.pane_focus_target_requested = true;
                    *context.pane_focus_target = Some(target);
                }
            }
            if let Some(delivery) = delivery {
                let target = delivery
                    .registration_target(registered_target)
                    .filter(|target| {
                        !retry
                            && crate::focus::pane_registered_focus_target(
                                child_ui, clip, *target, false,
                            )
                    });
                delivery.settle_registration(context, target);
            }
            if let Some(replay) = replay {
                replay.finish(context, child_ui, clip);
            }
            if let Some(receiver) = pane_receiver
                && pane_rect.is_finite()
                && pane_rect.is_positive()
            {
                for widget_id in pane_response.content_widgets() {
                    let Some(widget) = child_ui
                        .ctx()
                        .viewport(|viewport| viewport.this_pass.widgets.get(widget_id).copied())
                    else {
                        continue;
                    };
                    if widget.enabled
                        && widget.layer_id == child_ui.layer_id()
                        && widget.interact_rect.is_finite()
                        && widget.interact_rect.is_positive()
                        && pane_rect.contains_rect(widget.interact_rect)
                    {
                        context.receivers.push(ProductReceiverBinding {
                            interact_rect: widget.interact_rect,
                            interactive: widget.sense.interactive(),
                            widget_id: widget.id,
                            layer_id: widget.layer_id,
                            receiver,
                        });
                    }
                }
            }
        }
        drop(child.take());
    };
    if let Some(audit) = paint_audit.as_mut() {
        audit.check_append(&painter, paint_contents);
    } else {
        paint_contents();
    }
    context.paint_audit = paint_audit;
}

pub(crate) fn paint_tab_bar(
    context: &mut RenderContext<'_, '_, '_>,
    bar: dockspace::runtime::TabBarPaintRecord<'_>,
    paint: SurfacePaintDisposition,
) {
    let paint_omitted = paint.is_source_gap();
    if !paint_omitted && let Some(bounds) = egui_rect(bar.bounds()) {
        context
            .ui
            .painter()
            .rect_filled(bounds, 0.0, context.visuals.tab_bar_fill);
        crate::tab::paint_bar_separator(
            context.ui.painter(),
            bounds,
            context.visuals.tab_bar_stroke,
        );
    }
    let id =
        context
            .ui
            .make_persistent_id((context.instance_id, "tab-strip-scroll", bar.visual_id()));
    // Preserve scroll geometry so native hit testing can prove modal occlusion.
    let receiver = context
        .ui
        .is_enabled()
        .then(|| context.plan.receiver_for_tab_strip_scroll(bar))
        .flatten();
    context.register_scroll_receiver(id, receiver);
    paint_group_drag_regions(context, bar, paint);
    super::tab_customization::paint_bar_slots(context, bar, paint);
    #[cfg(feature = "native-render-support")]
    publish_native_tab_strip_diagnostics(context, bar);
}

#[cfg(feature = "native-render-support")]
fn publish_native_tab_strip_diagnostics(
    context: &RenderContext<'_, '_, '_>,
    bar: dockspace::runtime::TabBarPaintRecord<'_>,
) {
    use dockspace::runtime::TabStripMemberVisibility;

    if !super::native_drag_preview_diagnostic_enabled(context.ui.ctx()) {
        return;
    }
    let Some(viewport) = egui_rect(bar.viewport()) else {
        return;
    };
    for member in bar.members() {
        let visibility = match member.visibility() {
            TabStripMemberVisibility::Visible => "fully-visible",
            TabStripMemberVisibility::PartiallyVisible => "partially-visible",
            TabStripMemberVisibility::Hidden => continue,
        };
        if context.resources.tab_visual(context.plan, member.key()) != Some(member.visual_id()) {
            continue;
        }
        let Some(resource) = context.resources.tab(member.key()) else {
            continue;
        };
        let Some(full) = egui_rect(member.full_bounds()) else {
            continue;
        };
        let title = resource.title.as_str();
        let id = context.ui.make_persistent_id((
            context.instance_id,
            "native-tab-diagnostic",
            member.visual_id(),
        ));
        context
            .ui
            .ctx()
            .accesskit_node_builder(id.with("native-tab-full-bounds"), |node| {
                node.set_role(Role::Label);
                node.set_label(format!("Dockspace tab full bounds {title}"));
                node.set_value(visibility);
                node.set_bounds(accesskit_bounds(full));
            });
        context
            .ui
            .ctx()
            .accesskit_node_builder(id.with("native-tab-strip-viewport"), |node| {
                node.set_role(Role::Label);
                node.set_label(format!("Dockspace tab strip viewport {title}"));
                node.set_value(format!(
                    "offset={};maximum={}",
                    bar.scroll_offset(),
                    bar.maximum_scroll_offset(),
                ));
                node.set_bounds(accesskit_bounds(viewport));
            });
    }
}

fn paint_group_drag_regions(
    context: &mut RenderContext<'_, '_, '_>,
    bar: dockspace::runtime::TabBarPaintRecord<'_>,
    paint: SurfacePaintDisposition,
) {
    let bar_hovered =
        egui_rect(bar.bounds()).is_some_and(|rect| context.ui.rect_contains_pointer(rect));
    for region in bar.group_drag_regions() {
        let Some(bounds) = egui_rect(region.bounds()) else {
            continue;
        };
        let id = context.ui.make_persistent_id((
            context.instance_id,
            "tab-group-drag-region",
            region.visual_id(),
        ));
        let receiver = context.plan.receiver_for_tab_group_drag_region(region);
        let enabled = effective_enabled(
            context.ui,
            receiver.is_some() && context.pointer_authority.supports_preview_gestures(),
        );
        let response = context.interact_receiver(
            bounds,
            id,
            if enabled {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
            enabled.then_some(receiver).flatten(),
        );
        let description = group_drag_description(context.ui.ctx(), context.input);
        context.ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(Role::GenericContainer);
            node.set_label("Drag tab group");
            node.set_description(description.clone());
            node.set_bounds(accesskit_bounds(bounds));
            if !enabled {
                node.set_disabled();
            }
        });
        let response = if enabled {
            response.on_hover_text(description)
        } else {
            response
        };
        let source_gap = paint.is_source_gap();
        if enabled && (response.hovered() || source_gap) {
            context.ui.ctx().set_cursor_icon(if source_gap {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
        }
        if region.kind() == TabGroupDragRegionKind::LeadingGrip {
            let paint_omitted = paint.is_source_gap();
            if !paint_omitted
                && enabled
                && (context.style.tab_group_grip_visibility
                    == crate::TabGroupGripVisibility::Always
                    || bar_hovered)
            {
                super::drag_grip::paint(
                    context.ui,
                    bounds,
                    context
                        .interaction_stroke(&response, context.style.visuals.tab_text_color)
                        .color,
                );
            }
        }
    }
}

fn group_drag_description(context: &egui::Context, input: &DockInputConfig) -> String {
    let mut description =
        "Drag to move or dock this tab group. Other tab groups stay in place.".to_owned();
    match input.group_preserve_layout {
        Some(modifiers) if modifiers.is_none() => {
            description.push_str(" Center drops preserve this group as one layout tab.");
        }
        binding => {
            description.push_str(" Center drops merge its direct tabs.");
            if let Some(modifiers) = binding {
                description.push_str(&format!(
                    " Hold {} to keep this group as one layout tab instead.",
                    context.format_modifiers(modifiers)
                ));
            }
        }
    }
    description.push_str(" Open Tab group actions for commands.");
    description
}

pub(crate) fn paint_tab_record(
    context: &mut RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    paint: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    let resources = context.resources;
    let Some(resource) = resources.tab(tab.key()) else {
        return Ok(());
    };
    paint_tab(context, tab, resource, paint)
}

pub(crate) fn paint_tab(
    context: &mut RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    resource: &TabPaintResource,
    paint: SurfacePaintDisposition,
) -> Result<(), DockspaceRuntimeError> {
    let Some(visible) = egui_rect(tab.visible_bounds()) else {
        return Ok(());
    };
    let Some(drag) = egui_rect(tab.drag_bounds()) else {
        return Ok(());
    };
    let id = tab_id(context.ui, context.instance_id, tab.input_id());
    let operable = effective_enabled(context.ui, tab.operable());
    // Policy-disabled tabs retain their visible hover and accessibility footprint.
    let response_bounds = if operable { drag } else { visible };
    let preview_gestures_available = context.pointer_authority.supports_preview_gestures();
    let receiver = (operable && preview_gestures_available)
        .then(|| context.plan.receiver_for_tab_body(tab))
        .flatten();
    let response = context.interact_receiver(
        response_bounds,
        id,
        if operable && preview_gestures_available {
            Sense::click_and_drag()
        } else if operable {
            Sense::click()
        } else {
            Sense::hover()
        },
        receiver,
    );
    let scope_hint = if tab.key().is_layout() {
        "Actions affect this layout tab and its complete nested layout."
    } else {
        "Actions affect this pane tab."
    };
    let response = response.on_hover_ui(|ui| {
        ui.label(&resource.title);
        ui.weak(scope_hint);
        if operable {
            ui.label("Right-click or press Shift+F10 for tab actions.");
        }
    });
    let source_gap = paint.is_source_gap();
    if operable && preview_gestures_available && (response.hovered() || source_gap) {
        context.ui.ctx().set_cursor_icon(if source_gap {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        });
    }
    let selected = tab.selected();
    let focused = response.has_focus();
    let pane_focused = selected
        && operable
        && tab.item().is_some()
        && context.plan.focused_pane() == tab.item()
        && context.ui.input(|input| input.focused);
    let visuals = (!source_gap).then(|| {
        super::tab_customization::paint_tab(context, tab, resource, &response, pane_focused)
    });
    if let Some(visuals) = visuals {
        if pane_focused
            && visuals.pane_focus_stroke.width > 0.0
            && visuals.pane_focus_stroke.color.a() > 0
        {
            let underline = egui::Rect::from_min_max(
                pos2(
                    visible.left(),
                    (visible.bottom() - visuals.pane_focus_stroke.width).max(visible.top()),
                ),
                visible.right_bottom(),
            );
            #[cfg(feature = "motion")]
            let underline =
                super::motion::sample(context, super::motion::Effect::PointerFocus, underline);
            context
                .ui
                .painter()
                .rect_filled(underline, 0.0, visuals.pane_focus_stroke.color);
        }
        if focused {
            context.ui.painter_at(visible).rect_stroke(
                egui_rect(tab.full_bounds())
                    .map(|rect| crate::tab::background_rect(rect, visuals.background_inset))
                    .unwrap_or(visible)
                    .shrink(visuals.focus_stroke.width),
                visuals.corner_radius,
                visuals.focus_stroke,
                StrokeKind::Inside,
            );
        }
    }
    configure_tab_accessibility(
        context.ui,
        id,
        response_bounds,
        resource.title.as_str(),
        selected,
        operable,
        tab.key().is_layout(),
    );
    #[cfg(feature = "native-render-support")]
    publish_native_tab_bounds_diagnostic(context.ui, id, visible, resource.title.as_str());
    if operable {
        capture_tab_actions(context, tab, &response, focused);
        super::tab_close::capture_body_click(context, tab, &response);
        super::tab_close::paint(context, tab, resource, &response, visuals);
    }
    super::presentation_menu::paint_tab(context, tab, &response, paint)
}

#[cfg(feature = "native-render-support")]
fn publish_native_tab_bounds_diagnostic(ui: &Ui, tab: Id, visible: egui::Rect, title: &str) {
    if !super::native_drag_preview_diagnostic_enabled(ui.ctx()) {
        return;
    }
    ui.ctx()
        .accesskit_node_builder(tab.with("native-tab-bounds"), |node| {
            node.set_role(Role::Label);
            node.set_label(format!("Dockspace tab bounds {title}"));
            node.set_bounds(accesskit_bounds(visible));
        });
}

pub(super) fn tab_id(ui: &Ui, instance_id: Id, input: DockspaceControlId) -> Id {
    ui.make_persistent_id((instance_id, "tab", input))
}

fn capture_tab_actions(
    context: &mut RenderContext<'_, '_, '_>,
    tab: TabPaintRecord<'_>,
    response: &egui::Response,
    focused: bool,
) {
    let id = response.id;
    let accesskit_activation = context
        .ui
        .input(|input| input.has_accesskit_action_request(id, Action::Click));
    let accesskit_focus = context
        .ui
        .input(|input| input.has_accesskit_action_request(id, Action::Focus));
    let pointer_activation = context.pointer_authority.accepts_local_pointer_actions()
        && super::actions::unambiguous_pointer_click(context.ui, response, PointerButton::Primary);
    if accesskit_activation || pointer_activation {
        response.request_focus();
        if (!tab.selected() || accesskit_activation)
            && let Some(action) = context.plan.prepare_tab_select(tab.key())
        {
            context.push_local_action(
                id.with(if accesskit_activation {
                    "accesskit-click"
                } else {
                    "pointer-click"
                }),
                action,
            );
        }
    } else if accesskit_focus {
        response.request_focus();
        if let Some(action) = context.plan.prepare_tab_focus(tab.key()) {
            context.push_local_action(id.with("accesskit-focus"), action);
        }
    }
    if focused {
        context.ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
}

fn configure_tab_accessibility(
    ui: &Ui,
    id: Id,
    rect: egui::Rect,
    title: &str,
    selected: bool,
    operable: bool,
    layout: bool,
) {
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(Role::Tab);
        node.set_bounds(accesskit_bounds(rect));
        node.set_label(title);
        if layout {
            node.set_description("Layout tab containing a complete nested layout");
        }
        if operable {
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
        } else {
            node.set_disabled();
        }
        if selected {
            node.set_selected(true);
        } else {
            node.clear_selected();
        }
    });
}

#[cfg(all(test, feature = "native-render-support"))]
mod tests {
    use super::*;

    #[test]
    fn native_tab_diagnostic_is_opt_in_and_passive_without_changing_drag_semantics() {
        for enabled in [false, true] {
            let context = egui::Context::default();
            context.enable_accesskit();
            if enabled {
                super::super::enable_native_drag_preview_diagnostic(&context);
            }
            let full = egui::Rect::from_min_max(pos2(28.0, 0.0), pos2(110.0, 28.0));
            let drag = egui::Rect::from_min_max(pos2(28.0, 0.0), pos2(76.0, 28.0));
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                let id = ui.make_persistent_id("native-tab-diagnostic-test");
                configure_tab_accessibility(ui, id, drag, "Main", true, true, false);
                publish_native_tab_bounds_diagnostic(ui, id, full, "Main");
            });
            let nodes = &output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes;
            let tab = nodes
                .iter()
                .find(|(_, node)| node.role() == Role::Tab && node.label() == Some("Main"))
                .unwrap();
            assert_eq!(tab.1.bounds(), Some(accesskit_bounds(drag)));
            let diagnostic: Vec<_> = nodes
                .iter()
                .filter(|(_, node)| node.label() == Some("Dockspace tab bounds Main"))
                .collect();
            assert_eq!(diagnostic.len(), usize::from(enabled));
            if let [(_, node)] = diagnostic.as_slice() {
                assert_eq!(node.role(), Role::Label);
                assert_eq!(node.bounds(), Some(accesskit_bounds(full)));
                assert!(!node.supports_action(Action::Click));
                assert!(!node.supports_action(Action::Focus));
            }
            output.drop_without_applying_deltas();
        }
    }
}
