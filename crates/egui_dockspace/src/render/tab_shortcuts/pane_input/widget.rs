//! Exposes a deferred keyboard packet only during its editor callback.

use egui::{Context, Event, EventFilter, Id, Rect, Ui};

use super::{is_keyboard_input, is_plain_escape, tab_traversal};

#[derive(Clone, Default)]
pub(super) struct WidgetInput {
    pub(super) widget: Option<Id>,
    pub(super) events: Option<Vec<Event>>,
    pub(super) restore_focus: bool,
    pub(super) unconsumed: Option<Vec<Event>>,
    pub(super) traversal_tail: Option<Vec<Event>>,
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
    let context = ui.ctx().clone();
    let key = widget_input_key(&context);
    let data_key = egui::util::id_type_map::RawKey::new::<WidgetInput>(key);
    if !context.data(|data| {
        data.get_temp_raw(data_key)
            .and_then(|input| input.downcast_ref::<WidgetInput>())
            .is_some_and(|input| input.widget == Some(id))
    }) {
        return draw(ui);
    }
    let focused = ui.memory(egui::Memory::focused);
    let visible = ui.is_enabled()
        && ui.is_visible()
        && context.viewport(|viewport| {
            viewport.prev_pass.widgets.get(id).is_some_and(|widget| {
                widget.layer_id == ui.layer_id()
                    && ui
                        .clip_rect()
                        .intersect(Rect::from_min_size(
                            ui.next_widget_position(),
                            widget.rect.size(),
                        ))
                        .is_positive()
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
    let mut escape = false;
    let mut traversal_tail = None;
    if let Some(index) = events.iter().position(|event| {
        !filter.matches(event) && (tab_traversal(event).is_some() || is_plain_escape(event))
    }) {
        let mut tail = events.split_off(index);
        let navigation = tail.remove(0);
        if let Some(direction) = tab_traversal(&navigation) {
            ui.memory_mut(|memory| memory.move_focus(direction));
            traversal_tail = Some(tail);
        } else {
            escape = true;
            traversal_tail = Some(Vec::new());
        }
    }
    let (response, unconsumed) = with_keyboard_events(&context, events, || draw(ui));
    if escape {
        ui.memory_mut(|memory| memory.surrender_focus(id));
    }
    context.data_mut(|data| {
        let input = data.get_temp_mut_or_default::<WidgetInput>(key);
        input.unconsumed = Some(unconsumed);
        input.traversal_tail = traversal_tail;
    });
    response
}

/// Draws a TextEdit with its exact deferred Dockspace keyboard input, if any.
pub fn pane_text_edit(ui: &mut Ui, id: Id, edit: egui::TextEdit<'_>) -> egui::Response {
    with_pane_keyboard_input(ui, id, edit.get_event_filter(), |ui| ui.add(edit.id(id)))
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
