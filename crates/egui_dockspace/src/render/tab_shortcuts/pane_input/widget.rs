//! Exposes a deferred keyboard packet only during its editor callback.

use egui::{Align, Context, Direction, Event, EventFilter, FocusDirection, Id, Rect, Ui};

use super::{framework_traversal, is_keyboard_input, is_plain_escape};

#[derive(Clone)]
pub(super) enum FrameworkInput {
    Traverse(FocusDirection, Vec<Event>),
    Surrender(Vec<Event>),
}

#[derive(Clone, Default)]
pub(super) struct WidgetInput {
    pub(super) widget: Option<Id>,
    pub(super) events: Option<Vec<Event>>,
    pub(super) restore_focus: bool,
    pub(super) unconsumed: Option<Vec<Event>>,
    pub(super) framework_input: Option<FrameworkInput>,
}

pub(super) fn widget_input_key(context: &Context) -> Id {
    Id::unique(("dockspace-deferred-editor-input", context.viewport_id()))
}

/// Gives one focusable widget access to keyboard events deferred by Dockspace.
///
/// Call this from [`crate::PaneView::ui`] around the widget that owns `id`.
/// Pass its egui event filter, and keep other focusable widgets outside the
/// callback. When no deferred input belongs to the widget, the callback runs
/// with ordinary egui input.
pub fn with_pane_keyboard_input<R>(
    ui: &mut Ui,
    id: Id,
    filter: EventFilter,
    draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    with_widget_input(ui, id, filter, None, draw)
}

/// Gives the widget at `bounds` access to keyboard events deferred by Dockspace.
///
/// Use the widget's current allocated rectangle in the same coordinate space as
/// `ui.clip_rect()`. Draw only the widget that owns `id` inside `draw`. Event
/// delivery requires a visible intersection, a live focus binding and an
/// enabled widget. This form also supports explicitly positioned controls.
pub fn with_pane_keyboard_input_at<R>(
    ui: &mut Ui,
    id: Id,
    filter: EventFilter,
    bounds: Rect,
    draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    with_widget_input(ui, id, filter, Some(bounds), draw)
}

fn with_widget_input<R>(
    ui: &mut Ui,
    id: Id,
    filter: EventFilter,
    bounds: Option<Rect>,
    draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    let context = ui.ctx().clone();
    let key = widget_input_key(&context);
    let data_key = egui::util::id_type_map::RawKey::new::<WidgetInput>(key);
    if !owns_packet(&context, data_key, id) {
        return draw(ui);
    }
    let focused = ui.memory(egui::Memory::focused);
    let visible = ui.is_enabled()
        && ui.is_visible()
        && ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
        && context.viewport(|viewport| {
            viewport.prev_pass.widgets.get(id).is_some_and(|widget| {
                let bounds = bounds.unwrap_or_else(|| next_widget_bounds(ui, widget.rect.size()));
                widget.layer_id == ui.layer_id()
                    && bounds.is_finite()
                    && ui.clip_rect().intersect(bounds).is_positive()
            })
        });
    let delivery = visible
        .then(|| {
            context.data_mut(|data| {
                let input = data.get_temp_mut_or_default::<WidgetInput>(key);
                if input.widget != Some(id)
                    || !(focused == Some(id) || focused.is_none() && input.restore_focus)
                {
                    return None;
                }
                Some((input.events.take()?, input.restore_focus))
            })
        })
        .flatten();
    let Some((mut events, restore_focus)) = delivery else {
        return draw(ui);
    };
    if restore_focus && focused.is_none() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }

    // Editor keys stay in packet order. Give only an unlocked framework
    // traversal to egui's focus resolver, and hold any following text until
    // the resolver has identified the next widget in this same pane.
    let mut framework_input = None;
    if let Some(index) = events.iter().position(|event| {
        !filter.matches(event) && (framework_traversal(event).is_some() || is_plain_escape(event))
    }) {
        let mut tail = events.split_off(index);
        let navigation = tail.remove(0);
        if let Some(direction) = framework_traversal(&navigation) {
            ui.memory_mut(|memory| memory.move_focus(direction));
            framework_input = Some(FrameworkInput::Traverse(direction, tail));
        } else {
            framework_input = Some(FrameworkInput::Surrender(tail));
        }
    }
    let (response, unconsumed) = with_keyboard_events(&context, events, || draw(ui));
    if matches!(framework_input, Some(FrameworkInput::Surrender(_))) {
        ui.memory_mut(|memory| memory.surrender_focus(id));
    }
    context.data_mut(|data| {
        let input = data.get_temp_mut_or_default::<WidgetInput>(key);
        input.unconsumed = Some(unconsumed);
        input.framework_input = framework_input;
    });
    response
}

fn next_widget_bounds(ui: &Ui, size: egui::Vec2) -> Rect {
    let layout = ui.layout();
    let anchor = match layout.main_dir() {
        Direction::LeftToRight => egui::Align2::new(Align::Min, layout.vertical_align()),
        Direction::RightToLeft => egui::Align2::new(Align::Max, layout.vertical_align()),
        Direction::TopDown => egui::Align2::new(layout.horizontal_align(), Align::Min),
        Direction::BottomUp => egui::Align2::new(layout.horizontal_align(), Align::Max),
    };
    anchor.anchor_size(ui.next_widget_position(), size)
}

/// Draws a TextEdit with its exact deferred Dockspace keyboard input, if any.
pub fn pane_text_edit(ui: &mut Ui, id: Id, edit: egui::TextEdit<'_>) -> egui::Response {
    let ctx = ui.ctx();
    let key = egui::util::id_type_map::RawKey::new::<WidgetInput>(widget_input_key(ctx));
    if !owns_packet(ctx, key, id) {
        return ui.add(edit.id(id));
    }
    let Some(size) = ctx.viewport(|viewport| {
        let widget = viewport.prev_pass.widgets.get(id)?;
        (widget.rect.is_finite() && widget.rect.is_positive()).then(|| widget.rect.size())
    }) else {
        return ui.add(edit.id(id));
    };
    let filter = edit.get_event_filter();
    let layout = ui
        .layout()
        .with_main_wrap(false)
        .with_main_justify(true)
        .with_cross_justify(true);
    // Let egui place the editor in the current layout before opening the
    // input scope. The child fills that allocation and inherits its clip.
    ui.allocate_ui_with_layout(size, layout, |ui| {
        let bounds = ui.max_rect();
        ui.shrink_clip_rect(bounds);
        with_pane_keyboard_input_at(ui, id, filter, bounds, |ui| ui.add(edit.id(id)))
    })
    .inner
}

fn owns_packet(context: &Context, key: egui::util::id_type_map::RawKey, widget: Id) -> bool {
    context.data(|data| {
        data.get_temp_raw(key)
            .and_then(|input| input.downcast_ref::<WidgetInput>())
            .is_some_and(|input| input.widget == Some(widget) && input.events.is_some())
    })
}

struct ScopedKeyboardEvents<'a> {
    context: &'a Context,
    hidden: Vec<Event>,
    active: bool,
}

impl ScopedKeyboardEvents<'_> {
    fn restore(&mut self) -> Vec<Event> {
        self.active = false;
        self.context.input_mut(|input| {
            let unconsumed = input
                .events
                .extract_if(.., |event| is_keyboard_input(event))
                .collect();
            input.events.splice(..0, std::mem::take(&mut self.hidden));
            unconsumed
        })
    }
}

impl Drop for ScopedKeyboardEvents<'_> {
    fn drop(&mut self) {
        if self.active {
            self.restore();
        }
    }
}

pub(super) fn with_keyboard_events<R>(
    context: &Context,
    events: Vec<Event>,
    draw: impl FnOnce() -> R,
) -> (R, Vec<Event>) {
    let hidden = context.input_mut(|input| {
        let hidden = input
            .events
            .extract_if(.., |event| is_keyboard_input(event))
            .collect();
        input.events.splice(..0, events);
        hidden
    });
    let mut guard = ScopedKeyboardEvents {
        context,
        hidden,
        active: true,
    };
    let result = draw();
    (result, guard.restore())
}
