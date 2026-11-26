//! Scroll phases retain sequence identity independently of mouse buttons.

use dockspace::geometry::PhysicalPoint;
use dockspace::runtime::{
    NativePointerEvent, NativeScrollCancelReason, NativeScrollDelta, NativeScrollEvent,
    NativeScrollModifiers, NativeScrollMomentum, NativeScrollPhase, NativeScrollSequenceId,
    NativeWindowBinding, NativeWorkAreaBinding,
};
use winit::event::{MouseScrollDelta, TouchPhase, WindowEvent};

use crate::event::{NativePointerRoutes, NativeWindowEventRecord};

use super::device::{NativeInputDeviceKey, NativePointerDeviceEntry};
use super::{
    NativePointerTranslation, NativePointerTranslator, exact_delivery_binding, native_input,
};

#[derive(Debug, Clone, Copy, Default)]
pub(super) enum NativeScrollState {
    #[default]
    Idle,
    /// One winit phaseful sequence still maps to a live core sequence.
    Live {
        sequence: NativeScrollSequenceId,
        delivery: Option<NativeWindowBinding>,
    },
    /// Core closed the provider tail; ignore late phases from that binding.
    Retired {
        delivery: Option<NativeWindowBinding>,
    },
}

impl NativePointerTranslator {
    pub(super) fn scroll(
        &mut self,
        device: NativeInputDeviceKey,
        delta: NativeScrollDelta,
        phase: TouchPhase,
        record: &NativeWindowEventRecord,
        routes: NativePointerRoutes,
        resolve_work_area: &mut impl FnMut(PhysicalPoint) -> Option<NativeWorkAreaBinding>,
    ) -> NativePointerTranslation {
        if self.device_mut(device).is_none() {
            return NativePointerTranslation::Ignored;
        }
        let Self {
            devices,
            next_scroll_sequence,
            ..
        } = self;
        let Some(NativePointerDeviceEntry::Live(device)) = devices.get_mut(&device) else {
            unreachable!("a successfully enrolled device remains live in the adapter map");
        };
        let modifiers = match record.event() {
            WindowEvent::MouseWheel { facts, .. } => native_modifiers(facts),
            WindowEvent::PanGesture { .. } => NativeScrollModifiers::Unknown,
            _ => unreachable!("scroll translation only receives wheel or pan records"),
        };
        let delivery = exact_delivery_binding(routes.delivery());
        if let NativeScrollState::Live {
            sequence: active_sequence,
            delivery: Some(active),
        } = device.scroll
            && delivery != Some(active)
        {
            // A brand-new Started phase on another live window recovers when an
            // earlier phaseful sequence lost its Ended edge (e.g. macOS/Wayland
            // shared DeviceId). Non-start phases from another binding remain
            // ignored so delayed predecessor tails cannot mutate or end the live sequence.
            if phase != TouchPhase::Started {
                return NativePointerTranslation::Ignored;
            }
            let Some(next_sequence) = allocate_scroll_sequence(next_scroll_sequence) else {
                return NativePointerTranslation::Ignored;
            };
            device.scroll = NativeScrollState::Live {
                sequence: next_sequence,
                delivery,
            };
            let cancelled = NativeScrollEvent::new(
                device.scroll_device,
                NativeScrollPhase::Cancel {
                    sequence: active_sequence,
                    reason: NativeScrollCancelReason::PlatformCancelled,
                },
                NativeScrollMomentum::Unknown,
                NativeScrollModifiers::Unknown,
            );
            let started = NativeScrollEvent::new(
                device.scroll_device,
                NativeScrollPhase::Begin {
                    sequence: next_sequence,
                    delta: Some(delta),
                },
                NativeScrollMomentum::Unknown,
                modifiers,
            );
            return NativePointerTranslation::Restart {
                cancelled: native_input(
                    device.pointer,
                    NativePointerEvent::Scrolled(cancelled),
                    record,
                    routes,
                    resolve_work_area,
                ),
                pressed: native_input(
                    device.pointer,
                    NativePointerEvent::Scrolled(started),
                    record,
                    routes,
                    resolve_work_area,
                ),
            };
        }
        let Some((phase, next_state)) =
            translate_scroll_phase(device.scroll, phase, delta, delivery, next_scroll_sequence)
        else {
            return NativePointerTranslation::Ignored;
        };
        device.scroll = next_state;

        let event = NativeScrollEvent::new(
            device.scroll_device,
            phase,
            NativeScrollMomentum::Unknown,
            modifiers,
        );
        NativePointerTranslation::Input(native_input(
            device.pointer,
            NativePointerEvent::Scrolled(event),
            record,
            routes,
            resolve_work_area,
        ))
    }
}

fn translate_scroll_phase(
    state: NativeScrollState,
    phase: TouchPhase,
    delta: NativeScrollDelta,
    event_delivery: Option<NativeWindowBinding>,
    next_scroll_sequence: &mut u64,
) -> Option<(NativeScrollPhase, NativeScrollState)> {
    match (state, phase) {
        (NativeScrollState::Idle | NativeScrollState::Retired { .. }, TouchPhase::Started) => {
            let sequence = allocate_scroll_sequence(next_scroll_sequence)?;
            Some((
                NativeScrollPhase::Begin {
                    sequence,
                    delta: Some(delta),
                },
                NativeScrollState::Live {
                    sequence,
                    delivery: event_delivery,
                },
            ))
        }
        (
            NativeScrollState::Live { sequence, delivery },
            TouchPhase::Started | TouchPhase::Moved,
        ) => Some((
            NativeScrollPhase::Update { sequence, delta },
            NativeScrollState::Live { sequence, delivery },
        )),
        (NativeScrollState::Live { sequence, .. }, TouchPhase::Ended) => Some((
            NativeScrollPhase::End {
                sequence,
                delta: Some(delta),
            },
            NativeScrollState::Idle,
        )),
        (NativeScrollState::Live { sequence, .. }, TouchPhase::Cancelled) => Some((
            NativeScrollPhase::Cancel {
                sequence,
                reason: NativeScrollCancelReason::PlatformCancelled,
            },
            NativeScrollState::Idle,
        )),
        (
            NativeScrollState::Retired {
                delivery: Some(retired),
            },
            TouchPhase::Moved,
        ) if event_delivery.is_some() && event_delivery != Some(retired) => Some((
            NativeScrollPhase::Discrete { delta },
            NativeScrollState::Idle,
        )),
        (NativeScrollState::Retired { .. }, _) => None,
        (NativeScrollState::Idle, TouchPhase::Moved) => Some((
            NativeScrollPhase::Discrete { delta },
            NativeScrollState::Idle,
        )),
        (NativeScrollState::Idle, TouchPhase::Ended) => (!is_zero_delta(delta)).then_some((
            NativeScrollPhase::Discrete { delta },
            NativeScrollState::Idle,
        )),
        (NativeScrollState::Idle, TouchPhase::Cancelled) => None,
    }
}

fn allocate_scroll_sequence(next_scroll_sequence: &mut u64) -> Option<NativeScrollSequenceId> {
    let next = next_scroll_sequence.checked_add(1)?;
    *next_scroll_sequence = next;
    Some(NativeScrollSequenceId::new(next))
}

pub(super) fn delta_value(delta: &MouseScrollDelta) -> Option<NativeScrollDelta> {
    let delta = match delta {
        MouseScrollDelta::LineDelta(x, y) => NativeScrollDelta::Lines {
            x: f64::from(*x),
            y: f64::from(*y),
        },
        MouseScrollDelta::PixelDelta(position) => NativeScrollDelta::PhysicalPixels {
            x: position.x,
            y: position.y,
        },
    };
    scroll_delta_is_finite(delta).then_some(delta)
}

pub(super) fn scroll_delta_is_finite(delta: NativeScrollDelta) -> bool {
    match delta {
        NativeScrollDelta::PhysicalPixels { x, y }
        | NativeScrollDelta::LogicalPoints { x, y }
        | NativeScrollDelta::Lines { x, y }
        | NativeScrollDelta::Pages { x, y } => x.is_finite() && y.is_finite(),
    }
}

fn is_zero_delta(delta: NativeScrollDelta) -> bool {
    match delta {
        NativeScrollDelta::PhysicalPixels { x, y }
        | NativeScrollDelta::LogicalPoints { x, y }
        | NativeScrollDelta::Lines { x, y }
        | NativeScrollDelta::Pages { x, y } => x == 0.0 && y == 0.0,
    }
}

fn native_modifiers(facts: &winit::event::PointerEventFacts) -> NativeScrollModifiers {
    facts
        .modifiers
        .map_or(NativeScrollModifiers::Unknown, |modifiers| {
            NativeScrollModifiers::Exact {
                shift: modifiers.shift_key(),
                control: modifiers.control_key(),
                alt: modifiers.alt_key(),
                command: modifiers.super_key(),
            }
        })
}
