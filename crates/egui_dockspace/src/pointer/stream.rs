//! Ordered egui input and recovery after a rejected host frame.

use std::collections::BTreeSet;

use dockspace::model::TabGroupDockMode;
use dockspace::runtime::{
    DockspaceInteractionError, PointerDeviceKind, SurfacePointerButton, SurfacePointerCancelReason,
    SurfacePointerEvent, SurfacePointerId,
};
use egui::{Event, Modifiers, PointerButton, Pos2};

use crate::input::DockInputConfig;
use crate::pass_settlement::ProductPointerPacket;

use super::touch::TouchContacts;

pub(super) const GENERIC_POINTER: SurfacePointerId = SurfacePointerId::new(1);
const PLATFORM_CANCEL: SurfacePointerCancelReason =
    SurfacePointerCancelReason::ExplicitPlatformCancellation;

/// Streams and targeting policy admitted by a workspace frame.
#[derive(Clone, Default)]
pub(super) struct PointerStreams {
    pub(super) position: Option<Pos2>,
    pub(super) generic_phase: GenericPointerPhase,
    pub(super) docking_enabled: Option<bool>,
    pub(super) group_dock_mode: Option<TabGroupDockMode>,
    foreign_capture: bool,
    pub(super) touches: TouchContacts,
}

pub(super) struct PointerSamples {
    pub(super) events: Vec<PointerSample>,
    pub(super) focused: bool,
    pub(super) modifiers: Modifiers,
}

/// Effects of physical input consumed before the corresponding draft commits.
#[derive(Default)]
pub(super) struct PendingPointerRecovery {
    cancelled_streams: BTreeSet<SurfacePointerId>,
    generic_position_lost: bool,
    paused_docking: bool,
    changed_group_mode: Option<TabGroupDockMode>,
}

impl PendingPointerRecovery {
    pub(super) fn mark_gap(&mut self, streams: &PointerStreams) {
        if streams.generic_phase.is_live() {
            self.cancelled_streams.insert(GENERIC_POINTER);
        }
        self.cancelled_streams.extend(streams.touches.pointers());
    }

    pub(super) fn remember(&mut self, streams: &PointerStreams, samples: &[PointerSample]) {
        for sample in samples {
            match sample.event {
                SurfacePointerEvent::Moved
                    if sample.pointer == GENERIC_POINTER
                        && sample.position.is_none_or(|point| !point.is_finite()) =>
                {
                    self.generic_position_lost = true;
                }
                SurfacePointerEvent::DockingEnabledChanged { enabled: false } => {
                    self.paused_docking = true;
                }
                SurfacePointerEvent::GroupDockModeChanged { mode }
                    if streams.group_dock_mode != Some(mode) =>
                {
                    self.changed_group_mode = Some(mode);
                }
                event
                    if ends_primary_gesture(event)
                        || event == SurfacePointerEvent::CaptureChanged =>
                {
                    let enrolled = if sample.pointer == GENERIC_POINTER {
                        if matches!(event, SurfacePointerEvent::ButtonReleased(_)) {
                            streams.generic_phase == GenericPointerPhase::Pressed
                        } else {
                            streams.generic_phase.is_live()
                        }
                    } else {
                        streams
                            .touches
                            .pointers()
                            .any(|pointer| pointer == sample.pointer)
                    };
                    if enrolled {
                        self.cancelled_streams.insert(sample.pointer);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn retained_position_is_current(&self, pointer: SurfacePointerId) -> bool {
        !self.cancelled_streams.contains(&pointer)
            && (pointer != GENERIC_POINTER || !self.generic_position_lost)
    }
}

/// Phase of the synthesized generic pointer stream.
///
/// A hover stream remains live after a release so ordinary motion and focus
/// observations keep their existing ownership semantics. The explicit
/// pressed phase is needed to recover when a host loses a release and later
/// delivers another primary press; a second press cannot be sent to the core
/// as though the first one had ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum GenericPointerPhase {
    #[default]
    Idle,
    Hover,
    Pressed,
}

impl GenericPointerPhase {
    const fn is_live(self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[derive(Clone, Copy)]
pub(super) struct PointerSample {
    pub(super) pointer: SurfacePointerId,
    pub(super) device: PointerDeviceKind,
    pub(super) event: SurfacePointerEvent,
    pub(super) position: Option<Pos2>,
    pub(super) docking_enabled: Option<bool>,
    pub(super) group_dock_mode: Option<TabGroupDockMode>,
}

impl PointerSample {
    pub(super) const fn generic(
        event: SurfacePointerEvent,
        position: Option<Pos2>,
        docking_enabled: Option<bool>,
        group_dock_mode: Option<TabGroupDockMode>,
    ) -> Self {
        Self {
            pointer: GENERIC_POINTER,
            device: PointerDeviceKind::Unknown,
            event,
            position,
            docking_enabled,
            group_dock_mode,
        }
    }
}

impl PointerStreams {
    pub(super) fn clear(&mut self) {
        let mut touches = std::mem::take(&mut self.touches);
        touches.clear();
        *self = Self {
            touches,
            ..Self::default()
        };
    }

    pub(super) fn sample(
        &self,
        pointer: SurfacePointerId,
        event: SurfacePointerEvent,
    ) -> Option<PointerSample> {
        if pointer == GENERIC_POINTER {
            Some(self.generic_sample(event))
        } else {
            self.touches
                .continuation(pointer, event, self.docking_enabled, self.group_dock_mode)
        }
    }

    pub(super) fn normalize(
        &mut self,
        packet: &ProductPointerPacket,
        config: DockInputConfig,
        modifiers: Modifiers,
        recovery: &PendingPointerRecovery,
        coordinates_supported: bool,
        foreign_drag: bool,
    ) -> Result<PointerSamples, DockspaceInteractionError> {
        let host_focused = packet.focused();
        let mut focused = host_focused;
        let mut modifiers = if focused { modifiers } else { Modifiers::NONE };
        let mut samples = Vec::new();
        self.recover(
            recovery,
            !coordinates_supported || !focused,
            foreign_drag,
            &mut samples,
        );
        self.apply_modifiers(config, modifiers, &mut samples);

        let mut touch_interval =
            !self.touches.is_empty() || focus_interval_has_touch(packet.events());
        for (index, event) in packet.events().iter().enumerate() {
            if let Event::WindowFocused(window_focused) = event {
                // A false host snapshot denies the packet. Ordered focus changes
                // delimit the delivery intervals in an otherwise focused packet.
                let next_focused = host_focused && *window_focused;
                if next_focused && !focused {
                    touch_interval = !self.touches.is_empty()
                        || focus_interval_has_touch(&packet.events()[index + 1..]);
                }
                focused = next_focused;
            }
            let pointer_event = matches!(
                event,
                Event::Touch { .. }
                    | Event::PointerMoved(_)
                    | Event::PointerButton { .. }
                    | Event::PointerGone
            );
            if !focused && (pointer_event || matches!(event, Event::Key { .. })) {
                // A queued key release can carry modifiers from before blur.
                continue;
            }
            if let Some(observed) = event_modifiers(event) {
                modifiers = observed;
                self.apply_modifiers(config, modifiers, &mut samples);
            }
            if touch_interval && pointer_event {
                // Retire the generic stream at the boundary where synthesized
                // input begins. The suppressed tail can include its terminal.
                self.cancel_generic(PLATFORM_CANCEL, &mut samples);
                if !matches!(event, Event::Touch { .. }) {
                    continue;
                }
            }
            self.normalize_event(event, config, coordinates_supported, &mut samples)?;
        }
        Ok(PointerSamples {
            events: samples,
            focused,
            modifiers,
        })
    }

    fn recover(
        &mut self,
        recovery: &PendingPointerRecovery,
        cancel_all: bool,
        foreign_drag: bool,
        samples: &mut Vec<PointerSample>,
    ) {
        if recovery.generic_position_lost {
            self.position = None;
        }
        let should_cancel = |pointer| cancel_all || recovery.cancelled_streams.contains(&pointer);
        if should_cancel(GENERIC_POINTER) {
            self.cancel_generic(PLATFORM_CANCEL, samples);
        }
        if cancel_all || !recovery.cancelled_streams.is_empty() {
            samples.extend(self.touches.cancel_where(
                should_cancel,
                PLATFORM_CANCEL,
                self.docking_enabled,
                self.group_dock_mode,
            ));
        }
        if foreign_drag && !self.foreign_capture {
            if self.generic_phase.is_live() {
                samples.push(self.generic_sample(SurfacePointerEvent::CaptureChanged));
            }
            samples.extend(
                self.touches
                    .capture_changed(self.docking_enabled, self.group_dock_mode),
            );
        }
        self.foreign_capture = foreign_drag;
        if recovery.generic_position_lost && self.generic_phase.is_live() {
            // Revoke the consumed position, keeping the stream available for reentry.
            samples.push(self.generic_sample(SurfacePointerEvent::Moved));
        }
        // Preserve modifier round trips that invalidated an old preview while
        // the final key state stayed the same.
        if recovery.paused_docking && self.docking_enabled == Some(true) {
            self.docking_enabled = Some(false);
            samples.extend(docking_change_samples(
                &self.touches,
                self.generic_phase.is_live(),
                false,
            ));
        }
        if let Some(mode) = recovery
            .changed_group_mode
            .filter(|&mode| self.group_dock_mode != Some(mode))
        {
            self.group_dock_mode = Some(mode);
            samples.extend(group_dock_change_samples(
                &self.touches,
                self.generic_phase.is_live(),
                mode,
            ));
        }
    }

    fn apply_modifiers(
        &mut self,
        config: DockInputConfig,
        modifiers: Modifiers,
        samples: &mut Vec<PointerSample>,
    ) {
        let generic_live = self.generic_phase.is_live();
        if let Some(enabled) = update_docking_enabled(&mut self.docking_enabled, config, modifiers)
        {
            samples.extend(docking_change_samples(&self.touches, generic_live, enabled));
        }
        if let Some(mode) = update_group_dock_mode(&mut self.group_dock_mode, config, modifiers) {
            samples.extend(group_dock_change_samples(&self.touches, generic_live, mode));
        }
    }

    fn normalize_event(
        &mut self,
        event: &Event,
        config: DockInputConfig,
        coordinates_supported: bool,
        samples: &mut Vec<PointerSample>,
    ) -> Result<(), DockspaceInteractionError> {
        let event = match *event {
            Event::Touch {
                device_id,
                id,
                phase,
                pos,
                ..
            } if coordinates_supported => {
                samples.extend(self.touches.observe(
                    device_id,
                    id,
                    phase,
                    pos,
                    self.docking_enabled,
                    self.group_dock_mode,
                )?);
                return Ok(());
            }
            Event::PointerMoved(point) => {
                self.position = Some(point);
                SurfacePointerEvent::Moved
            }
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                ..
            } => {
                self.position = Some(pos);
                if pressed {
                    if self.generic_phase == GenericPointerPhase::Pressed {
                        // A second press ends the stream whose release was lost.
                        self.cancel_generic(PLATFORM_CANCEL, samples);
                    }
                    self.generic_phase = GenericPointerPhase::Pressed;
                    SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary)
                } else {
                    self.generic_phase = GenericPointerPhase::Hover;
                    SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary)
                }
            }
            Event::PointerGone => {
                self.position = None;
                SurfacePointerEvent::Moved
            }
            Event::WindowFocused(false) => {
                self.cancel_all(PLATFORM_CANCEL, samples);
                return Ok(());
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if config.cancel_matches(key, modifiers) => {
                self.cancel_all(SurfacePointerCancelReason::ExplicitCancellation, samples);
                return Ok(());
            }
            _ => return Ok(()),
        };
        if self.generic_phase == GenericPointerPhase::Idle {
            self.generic_phase = GenericPointerPhase::Hover;
        }
        samples.push(self.generic_sample(event));
        Ok(())
    }

    fn cancel_generic(
        &mut self,
        reason: SurfacePointerCancelReason,
        samples: &mut Vec<PointerSample>,
    ) {
        if self.generic_phase.is_live() {
            samples.push(self.generic_sample(SurfacePointerEvent::StreamCancelled(reason)));
            self.generic_phase = GenericPointerPhase::Idle;
        }
    }

    fn cancel_all(&mut self, reason: SurfacePointerCancelReason, samples: &mut Vec<PointerSample>) {
        samples.extend(
            self.touches
                .cancel_all(reason, self.docking_enabled, self.group_dock_mode),
        );
        self.cancel_generic(reason, samples);
    }

    fn generic_sample(&self, event: SurfacePointerEvent) -> PointerSample {
        PointerSample::generic(
            event,
            self.position,
            self.docking_enabled,
            self.group_dock_mode,
        )
    }
}

pub(super) fn ends_primary_gesture(event: SurfacePointerEvent) -> bool {
    matches!(
        event,
        SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary)
            | SurfacePointerEvent::ContactEnded(SurfacePointerButton::Primary)
            | SurfacePointerEvent::StreamCancelled(_)
            | SurfacePointerEvent::StreamEnded
    )
}

fn focus_interval_has_touch(events: &[Event]) -> bool {
    events
        .iter()
        .take_while(|event| !matches!(event, Event::WindowFocused(false)))
        .any(|event| matches!(event, Event::Touch { .. }))
}

pub(crate) fn event_modifiers(event: &Event) -> Option<Modifiers> {
    match event {
        Event::ModifiersChanged(modifiers)
        | Event::Key { modifiers, .. }
        | Event::PointerButton { modifiers, .. } => Some(*modifiers),
        Event::WindowFocused(false) => Some(Modifiers::NONE),
        _ => None,
    }
}

pub(super) fn update_docking_enabled(
    current: &mut Option<bool>,
    config: DockInputConfig,
    modifiers: Modifiers,
) -> Option<bool> {
    let enabled = config.docking_enabled(modifiers);
    let changed = *current != Some(enabled);
    *current = Some(enabled);
    changed.then_some(enabled)
}

pub(super) fn docking_change_samples(
    touches: &TouchContacts,
    generic_live: bool,
    enabled: bool,
) -> impl Iterator<Item = PointerSample> {
    generic_live
        .then(|| {
            PointerSample::generic(
                SurfacePointerEvent::DockingEnabledChanged { enabled },
                None,
                Some(enabled),
                None,
            )
        })
        .into_iter()
        .chain(touches.docking_enabled_changed(enabled))
}

pub(super) fn update_group_dock_mode(
    current: &mut Option<TabGroupDockMode>,
    config: DockInputConfig,
    modifiers: Modifiers,
) -> Option<TabGroupDockMode> {
    let mode = config.tab_group_dock_mode(modifiers);
    let changed = *current != Some(mode);
    *current = Some(mode);
    changed.then_some(mode)
}

pub(super) fn group_dock_change_samples(
    touches: &TouchContacts,
    generic_live: bool,
    mode: TabGroupDockMode,
) -> impl Iterator<Item = PointerSample> {
    generic_live
        .then(|| {
            PointerSample::generic(
                SurfacePointerEvent::GroupDockModeChanged { mode },
                None,
                None,
                Some(mode),
            )
        })
        .into_iter()
        .chain(touches.group_dock_mode_changed(mode))
}
