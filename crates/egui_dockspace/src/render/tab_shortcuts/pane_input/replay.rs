//! Delivery to the verified editor and return of unconsumed host input.

use super::widget::{FrameworkInput, WidgetInput, widget_input_key, with_keyboard_events};
use super::*;

pub(in crate::render) struct PaneInputDelivery {
    pub(super) packet: PaneInputPacket,
    pub(super) phase: DeliveryPhase,
    pub(super) focus_cleared_by_escape: bool,
}

#[derive(Clone, Copy)]
pub(super) enum DeliveryPhase {
    AwaitingRegistration,
    Registered { retry: bool },
}

impl PaneInputDelivery {
    pub(in crate::render::tab_shortcuts) fn for_focus_request(
        context: &RenderContext<'_, '_, '_>,
    ) -> Option<Self> {
        if !context.pointer_authority.supports_preview_gestures()
            || !crate::render::tab_shortcuts::can_route_keyboard(context)
            || !context
                .ui
                .input(|input| input.events.iter().any(is_keyboard_input))
        {
            return None;
        }
        let request = context.pane_focus_request?;
        let item = request.item()?;
        if !context.panes.accepts_deferred_keyboard_input(item) {
            return None;
        }
        let widget = context.panes.focus_target(item);
        let target = PaneInputTarget::capture(context, item, widget)?;
        if has_pointer_or_accessibility_edges(context.ui.ctx())
            && !widget.is_some_and(|widget| pointer_keeps_focus(context.ui.ctx(), widget))
        {
            return None;
        }
        let prior_focus = context.ui.memory(egui::Memory::focused);
        if (widget.is_some() && prior_focus == widget)
            || prior_focus.is_some_and(|id| {
                // An application widget composed before this surface may have
                // already read the unaddressed keyboard events this pass.
                context
                    .ui
                    .ctx()
                    .viewport(|viewport| viewport.this_pass.widgets.get(id).is_some())
            })
        {
            return None;
        }
        crate::focus::pane_focus_handoff_pending(
            context.ui.ctx(),
            context.instance_id,
            request,
            widget,
            prior_focus,
        )
        .then(|| Self {
            packet: PaneInputPacket {
                target,
                events: take_keys_after(context.ui, 0),
            },
            phase: DeliveryPhase::AwaitingRegistration,
            focus_cleared_by_escape: false,
        })
    }

    pub(in crate::render) fn targets(&self, item: ItemId) -> bool {
        self.packet.target.item == item
    }

    pub(in crate::render) fn awaits_registration(&self) -> bool {
        matches!(self.phase, DeliveryPhase::AwaitingRegistration)
    }

    pub(in crate::render) fn registration_target(&self, default: Option<Id>) -> Option<Id> {
        self.packet.target.widget.or(default)
    }

    pub(in crate::render) fn take_for(
        delivery: &mut Option<Self>,
        ui: &Ui,
        pane: PanePaintRecord<'_>,
    ) -> Option<Self> {
        let expected = delivery.as_ref()?;
        (expected.targets(pane.item()) && crate::render::actions::effective_enabled(ui, true))
            .then(|| delivery.take())
            .flatten()
    }

    pub(in crate::render) fn can_replay(&self, ui: &Ui, clip: Rect) -> bool {
        let DeliveryPhase::Registered { retry } = self.phase else {
            return false;
        };
        self.packet.target.widget.is_some_and(|widget| {
            ui.memory(|memory| {
                memory.has_focus(widget)
                    || self.focus_cleared_by_escape && memory.focused().is_none()
            }) && (retry || crate::focus::pane_registered_focus_target(ui, clip, widget, true))
        })
    }

    pub(in crate::render) fn settle_registration(
        mut self,
        context: &mut RenderContext<'_, '_, '_>,
        target: Option<Id>,
    ) {
        if self.awaits_registration() && context.ui.ctx().will_discard() {
            defer(context, self.packet, Handoff::RetryRegistration);
        } else if self.awaits_registration()
            && let Some(target) = target.filter(|target| {
                self.packet
                    .target
                    .widget
                    .is_none_or(|expected| *target == expected)
            })
            && context.ui.memory(|memory| memory.has_focus(target))
        {
            self.packet.target = rebind_packets(context, self.packet.target, target);
            defer(context, self.packet, Handoff::Continuation);
        } else {
            self.retire(context);
        }
    }

    pub(in crate::render) fn retire(self, context: &mut RenderContext<'_, '_, '_>) {
        retire_input(context);
    }

    pub(in crate::render) fn replay(
        self,
        context: &RenderContext<'_, '_, '_>,
        pane: PanePaintRecord<'_>,
    ) -> ScopedPaneInput {
        let ctx = context.ui.ctx();
        let (keyboard, events) = with_keyboard_events(ctx, self.packet.events, || {
            crate::render::tab_shortcuts::TabKeyboard::for_replayed_pane(context, pane)
        });
        let key = widget_input_key(ctx);
        let previous = ctx.data_mut(|data| {
            let previous = data.remove_temp(key);
            data.insert_temp(
                key,
                WidgetInput {
                    widget: self.packet.target.widget,
                    events: Some(events),
                    restore_focus: self.focus_cleared_by_escape,
                    ..WidgetInput::default()
                },
            );
            previous
        });
        ScopedPaneInput {
            context: ctx.clone(),
            key: Some(key),
            previous,
            target: self.packet.target,
            keyboard,
        }
    }
}

pub(in crate::render) struct ScopedPaneInput {
    context: Context,
    key: Option<Id>,
    previous: Option<WidgetInput>,
    target: PaneInputTarget,
    keyboard: crate::render::tab_shortcuts::TabKeyboard,
}

impl ScopedPaneInput {
    pub(in crate::render) fn finish(
        mut self,
        context: &mut RenderContext<'_, '_, '_>,
        ui: &Ui,
        clip: Rect,
    ) {
        let input = self.restore_scope();
        let Some(events) = input.unconsumed else {
            if self.context.will_discard() {
                if let Some(mut events) = input.events {
                    if let crate::render::tab_shortcuts::TabKeyboard::PaneShortcut(cycle) =
                        std::mem::replace(
                            &mut self.keyboard,
                            crate::render::tab_shortcuts::TabKeyboard::Done,
                        )
                    {
                        events.extend(cycle.tail);
                    }
                    defer(
                        context,
                        PaneInputPacket {
                            target: self.target,
                            events,
                        },
                        Handoff::RetryDelivery,
                    );
                }
            } else {
                retire_input(context);
            }
            return;
        };
        let keyboard = std::mem::replace(
            &mut self.keyboard,
            crate::render::tab_shortcuts::TabKeyboard::Done,
        );
        let (_, unconsumed) =
            with_keyboard_events(&self.context, events, || keyboard.capture(context));
        // These keys can be observed by host callbacks only after the exact
        // widget processed the packet.
        context
            .pane_shortcuts
            .extend(unconsumed.into_iter().filter(is_key_event));
        match input.framework_input {
            Some(FrameworkInput::Surrender(tail)) => {
                context.pane_shortcuts.extend(tail);
                retire_input(context);
            }
            Some(FrameworkInput::Traverse(direction, tail)) => {
                let focused = ui.memory(egui::Memory::focused).filter(|&widget| {
                    crate::focus::pane_registered_focus_target(ui, clip, widget, false)
                });
                if direction == FocusDirection::Next
                    && focused.is_some()
                    && focused != self.target.widget
                {
                    settle_framework_keys(context, self.target, focused, tail);
                } else {
                    defer_framework_traversal(
                        context,
                        self.target,
                        ui.unique_id(),
                        direction,
                        tail,
                    );
                }
            }
            None => {}
        }
    }

    fn restore_scope(&mut self) -> WidgetInput {
        let Some(key) = self.key.take() else {
            return WidgetInput::default();
        };
        self.context.data_mut(|data| {
            let input = data.remove_temp(key).unwrap_or_default();
            if let Some(previous) = self.previous.take() {
                data.insert_temp(key, previous);
            }
            input
        })
    }
}

impl Drop for ScopedPaneInput {
    fn drop(&mut self) {
        self.restore_scope();
    }
}
