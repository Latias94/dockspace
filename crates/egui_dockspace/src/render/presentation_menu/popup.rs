//! Product menus retain their source coordinates inside a viewport-wide overlay.

use dockspace::runtime::{
    DockspaceVisualId, SurfacePaintPlan, SurfacePaintProgramOperation as Operation,
    SurfacePaintProgramStep as Step,
};
use egui::containers::menu::{MenuConfig, menu_style};
use egui::emath::TSTransform;
use egui::{
    Area, AreaState, Id, InnerResponse, LayerId, Layout, Order, Popup, RectAlign, Response,
    ScrollArea, Sense, Ui, UiKind, UiStackInfo, ViewportId,
};

use super::keyboard;

#[derive(Clone, Copy)]
struct MenuRun {
    instance: Id,
    popup: Id,
    workspace: dockspace::runtime::DockspaceWorkspaceStamp,
    surface: dockspace::model::SurfaceId,
    visual: DockspaceVisualId,
    toggled_in: Option<(ViewportId, u64)>,
    visible_pass: u64,
    press_started_while_open: bool,
}

fn run_id() -> Id {
    // egui owns one open popup per context; this slot retains no popup history.
    Id::unique("egui-dockspace-presentation-menu-run")
}

fn layer(instance: Id, surface: dockspace::model::SurfaceId) -> LayerId {
    LayerId::new(
        Order::Foreground,
        instance.with(("presentation-menu-layer", surface)),
    )
}

pub(crate) fn prepare_layer(ui: &Ui, instance: Id, surface: dockspace::model::SurfaceId) {
    let transform = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or(TSTransform::IDENTITY);
    ui.ctx()
        .set_transform_layer(layer(instance, surface), transform);
}

pub(super) fn source_available(ui: &Ui, instance: Id, popup: Id) -> bool {
    if ui.ctx().any_popup_open() {
        return false;
    }
    if !Popup::is_any_open(ui.ctx()) || Popup::is_id_open(ui.ctx(), popup) {
        return true;
    }
    ui.ctx()
        .data(|data| data.get_temp::<MenuRun>(run_id()))
        .is_some_and(|run| run.instance == instance && Popup::is_id_open(ui.ctx(), run.popup))
}

pub(crate) fn invalidate_stale(ui: &Ui, instance: Id, plan: SurfacePaintPlan<'_>) {
    let Some(run) = ui
        .ctx()
        .data(|data| data.get_temp::<MenuRun>(run_id()))
        .filter(|run| run.instance == instance && run.surface == plan.surface())
    else {
        return;
    };
    if run.workspace != plan.workspace_stamp()
        || !super::effective_enabled(ui, true)
        || !source_available(ui, instance, run.popup)
    {
        Popup::close_id(ui.ctx(), run.popup);
        ui.ctx().data_mut(|data| data.remove::<MenuRun>(run_id()));
        return;
    }
    if !Popup::is_id_open(ui.ctx(), run.popup) {
        return;
    }
    let mut live = false;
    let valid_program = plan
        .visit_surface_program(|step| {
            if let Step::Operation {
                operation, paint, ..
            } = step
                && operation.visual_id() == run.visual
                && !paint.is_source_gap()
            {
                live = match operation {
                    Operation::Tab(tab) => tab.operable(),
                    Operation::TabBar(bar) => plan.tabs_group_menu_operable(bar),
                    Operation::PresentationMenuAnchor(anchor) => anchor.operable(),
                    _ => false,
                };
            }
            Ok::<(), std::convert::Infallible>(())
        })
        .is_ok();
    if !valid_program || !live {
        Popup::close_id(ui.ctx(), run.popup);
        ui.ctx().data_mut(|data| data.remove::<MenuRun>(run_id()));
    }
}

pub(super) fn show(
    source: &Ui,
    instance: Id,
    surface: dockspace::model::SurfaceId,
    workspace: dockspace::runtime::DockspaceWorkspaceStamp,
    visual: DockspaceVisualId,
    popup: Id,
    anchor: &Response,
    toggle_on_click: bool,
    style: &crate::DockStyle,
    content: impl FnOnce(&mut Ui) -> (Vec<Id>, Vec<Id>),
) -> Option<InnerResponse<Option<keyboard::Outcome>>> {
    let context = source.ctx();
    if !super::effective_enabled(source, anchor.enabled())
        || !source_available(source, instance, popup)
    {
        Popup::close_id(context, popup);
        return None;
    }
    let frame = (
        context.viewport_id(),
        context.cumulative_frame_nr_for(context.viewport_id()),
    );
    let pass = context.cumulative_pass_nr();
    let mut run = context
        .data(|data| data.get_temp::<MenuRun>(run_id()))
        .filter(|run| run.popup == popup)
        .unwrap_or(MenuRun {
            instance,
            popup,
            workspace,
            surface,
            visual,
            toggled_in: None,
            visible_pass: 0,
            press_started_while_open: false,
        });
    if run.workspace != workspace {
        Popup::close_id(context, popup);
        context.data_mut(|data| data.remove::<MenuRun>(run_id()));
        return None;
    }
    // While open, Enter/Space belong to the ordered menu keyboard handler.
    // egui also marks the focused anchor as clicked for those same keys.
    let anchor_toggle = !Popup::is_id_open(context, popup)
        || source.input(|input| {
            input.has_accesskit_action_request(anchor.id, egui::accesskit::Action::Click)
        })
        || super::unambiguous_pointer_click(source, anchor, egui::PointerButton::Primary);
    if toggle_on_click && anchor.clicked() && anchor_toggle && run.toggled_in != Some(frame) {
        Popup::toggle_id(context, popup);
        run.toggled_in = Some(frame);
    }
    if !Popup::is_id_open(context, popup) {
        context.data_mut(|data| data.insert_temp(run_id(), run));
        return None;
    }
    Popup::open_id(context, popup);

    let transform = context
        .layer_transform_to_global(source.layer_id())
        .unwrap_or(TSTransform::IDENTITY);
    let viewport = transform.inverse() * context.content_rect();
    if !viewport.is_finite() || !viewport.size().is_finite() || !viewport.is_positive() {
        Popup::close_id(context, popup);
        return None;
    }
    let menu_layer = layer(instance, surface);
    prepare_layer(source, instance, surface);
    let was_open = run.visible_pass.checked_add(1) == Some(pass);
    let (pressed, clicked) =
        source.input(|input| (input.pointer.any_pressed(), input.pointer.any_click()));
    if !was_open {
        run.press_started_while_open = false;
    } else if pressed {
        run.press_started_while_open = true;
    }
    let expected = AreaState::load(context, menu_layer.id)
        .and_then(|state| state.size)
        .unwrap_or_else(|| egui::vec2(source.spacing().menu_width, 0.0))
        .min(viewport.size());
    let align = RectAlign::find_best_align(
        std::iter::once(RectAlign::BOTTOM_START)
            .chain(RectAlign::BOTTOM_START.symmetries())
            .chain(RectAlign::MENU_ALIGNS.iter().copied()),
        viewport,
        anchor.interact_rect,
        0.0,
        expected,
    )
    .unwrap_or(RectAlign::BOTTOM_START);
    let (pivot, position) = align.pivot_pos(&anchor.interact_rect, 0.0);
    let mut output = Area::new(menu_layer.id)
        .order(menu_layer.order)
        .fixed_pos(position)
        .pivot(pivot)
        .default_width(source.spacing().menu_width)
        .constrain_to(viewport)
        .enabled(source.is_enabled())
        .movable(false)
        .fade_in(false)
        .sizing_pass(!was_open)
        .sense(Sense::click())
        .layout(Layout::top_down_justified(egui::Align::Min))
        .accessibility_parent(anchor.id)
        .info(
            UiStackInfo::new(UiKind::Menu)
                .with_tag_value(MenuConfig::MENU_CONFIG_TAG, MenuConfig::new()),
        )
        .show(context, |ui| {
            ui.set_style(std::sync::Arc::clone(source.style()));
            menu_style(ui.style_mut());
            let frame = style.resolved_popup_frame(ui.style());
            let margins = frame.total_margin();
            let available = (viewport.size() - margins.sum()).max(egui::Vec2::ZERO);
            ui.set_max_width(ui.max_rect().width().min(available.x));
            frame
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .id_salt("presentation-menu-scroll")
                        .max_height(available.y)
                        .show(ui, |ui| {
                            let (entries, submenus) = content(ui);
                            let outcome = keyboard::capture(
                                ui,
                                anchor.id,
                                &entries,
                                &submenus,
                                source.is_enabled(),
                            );
                            if let Some(focused) = ui.memory(egui::Memory::focused)
                                && entries.contains(&focused)
                                && let Some(response) = ui.ctx().read_response(focused)
                                && !ui.clip_rect().contains_rect(response.rect)
                            {
                                response.scroll_to_me_animation(
                                    None,
                                    egui::style::ScrollAnimation::none(),
                                );
                            }
                            outcome
                        })
                        .inner
                })
                .inner
        });
    if output.response.should_close() || (run.press_started_while_open && clicked) {
        Popup::close_id(context, popup);
        output.response.set_close();
        run.press_started_while_open = false;
    }
    run.visible_pass = pass;
    context.data_mut(|data| data.insert_temp(run_id(), run));
    Some(output)
}
