//! egui input consumption can outlive both a Dockspace callback and its candidate output.

use super::*;
use egui::{RawInput, TouchDeviceId, TouchId, TouchPhase, ViewportBuilder, ViewportId};
use egui_dockspace::{
    DockspaceErrorKind, DockspaceManipulationCancelReason, DockspaceManipulationStatus,
    DockspaceTouchEvent,
};

#[derive(Clone, Copy)]
enum RunMode {
    Callback,
    Owned,
}

struct DragHarness {
    context: Context,
    dockspace: Dockspace,
    viewport: ViewportId,
    mode: RunMode,
}

struct InputFrame {
    output: FullOutput,
    feedback: Vec<DockspaceManipulationStatus>,
    close_requests: usize,
    touch_menu_requested: bool,
}

enum RepaintInput {
    Event(Event),
    ForeignCapture,
}

struct RepaintInputPlugin {
    completed_passes: u64,
    input: Option<RepaintInput>,
    foreign_capture: bool,
}

impl egui::plugin::Plugin for RepaintInputPlugin {
    fn debug_name(&self) -> &'static str {
        "product-input-continuity::repaint-input"
    }

    fn input_hook(&mut self, context: &Context, input: &mut RawInput) {
        if context.cumulative_pass_nr_for(input.viewport_id) != self.completed_passes {
            return;
        }
        match self.input.take() {
            Some(RepaintInput::Event(event)) => input.events.push(event),
            Some(RepaintInput::ForeignCapture) => {
                egui::DragAndDrop::set_payload(context, 1_u8);
                self.foreign_capture = true;
            }
            None => {}
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if std::mem::take(&mut self.foreign_capture) {
            assert_eq!(
                egui::DragAndDrop::take_payload::<u8>(ui.ctx()).as_deref(),
                Some(&1)
            );
        }
    }
}

impl DragHarness {
    fn new(viewport: ViewportId, mode: RunMode) -> Self {
        let context = Context::default();
        context.enable_accesskit();
        let dockspace = Dockspace::builder("product-input-continuity", layout())
            .style(instrumented_style())
            .build()
            .unwrap();
        Self {
            context,
            dockspace,
            viewport,
            mode,
        }
    }

    fn input(&self, events: Vec<Event>) -> RawInput {
        let mut raw = input(events);
        raw.viewport_id = self.viewport;
        raw.viewports.entry(self.viewport).or_default();
        raw
    }

    fn frame(&mut self, events: Vec<Event>) -> InputFrame {
        self.frame_at(None, events)
    }

    fn frame_at(&mut self, time: Option<f64>, events: Vec<Event>) -> InputFrame {
        let mut raw = self.input(events);
        raw.time = time;
        let mut feedback = Vec::new();
        let mut close_requests = 0;
        let mut touch_menu_requested = false;
        let mut observe = |response: egui_dockspace::DockspaceResponse| {
            feedback.extend(
                response
                    .manipulation_feedback()
                    .iter()
                    .map(|item| item.status()),
            );
            close_requests += response.close_request_events().len();
            touch_menu_requested |= response
                .touch_events()
                .iter()
                .any(|event| matches!(event, DockspaceTouchEvent::ContextMenuRequested(_)));
        };
        let mut output = match self.mode {
            RunMode::Callback => run_headless(&self.context, raw, |ui| {
                let response = self
                    .dockspace
                    .show_single_surface(SURFACE, ui, &mut Panes)
                    .unwrap();
                observe(response);
            }),
            RunMode::Owned => {
                let (output, response) = self
                    .dockspace
                    .run_single_surface(&self.context, raw, SURFACE, &mut Panes)
                    .unwrap();
                observe(response);
                let (output, receipt) = output.into_parts();
                receipt.report(SurfacePresentationResult::Presented);
                output
            }
        };
        output.textures_delta.clear();
        InputFrame {
            output,
            feedback,
            close_requests,
            touch_menu_requested,
        }
    }

    fn start_reorder(&mut self) -> Pos2 {
        let _ = self.frame(Vec::new());
        let stable = self.frame(Vec::new());
        let source = tab_center(&stable.output, "Second");
        let target = tab_center(&stable.output, "First");
        let _ = self.frame(vec![
            Event::PointerMoved(source),
            pointer_button(source, true),
        ]);
        let dragging = self.frame(vec![Event::PointerMoved(target)]);
        assert!(self.dockspace.has_active_gesture());
        assert!(
            rect_fill_count(
                &dragging.output,
                self.dockspace.style().visuals.ghost_fill.unwrap()
            ) > 0
        );
        target
    }

    fn start_touch_reorder(&mut self) -> Pos2 {
        self.context
            .options_mut(|options| options.max_passes = 1.try_into().unwrap());
        let _ = self.frame_at(Some(0.0), Vec::new());
        let ready = self.frame_at(Some(0.1), Vec::new());
        let source = tab_center(&ready.output, "Second");
        let target = tab_center(&ready.output, "First");
        let _ = self.frame_at(Some(1.0), vec![contact(1, TouchPhase::Start, source)]);
        let _ = self.frame_at(Some(1.35), Vec::new());
        let _ = self.frame_at(Some(1.4), vec![contact(1, TouchPhase::Move, target)]);
        assert!(
            self.dockspace.has_active_gesture(),
            "armed touch must begin a drag"
        );
        target
    }

    fn repaint_with_extra_input(&mut self, input: RepaintInput) -> InputFrame {
        self.context
            .options_mut(|options| options.max_passes = 2.try_into().unwrap());
        self.context.add_plugin(RepaintInputPlugin {
            completed_passes: self.context.cumulative_pass_nr_for(self.viewport) + 1,
            input: Some(input),
            foreign_capture: false,
        });
        self.context
            .plugin_or_default::<LateDiscardPlugin>()
            .lock()
            .remaining = 1;
        let frame = self.frame(Vec::new());
        assert_eq!(frame.output.platform_output.num_completed_passes, 2);
        assert!(frame.feedback.is_empty());
        frame
    }

    fn reject_owned_frame(&mut self, events: Vec<Event>) {
        self.reject_owned_input(self.input(events));
    }

    fn reject_owned_input(&mut self, input: RawInput) {
        self.context
            .plugin_or_default::<LateDiscardPlugin>()
            .lock()
            .strip_callback_compounds_once = true;
        let error = self
            .dockspace
            .run_single_surface(&self.context, input, SURFACE, &mut Panes)
            .err()
            .expect("the output hook must reject the candidate");
        assert_eq!(error.kind(), DockspaceErrorKind::OperationConflict);
    }

    fn assert_cancelled(&self, frame: &InputFrame) {
        self.assert_cancelled_with(
            frame,
            DockspaceManipulationCancelReason::PointerStreamCancelled,
        );
    }

    fn assert_cancelled_with(&self, frame: &InputFrame, reason: DockspaceManipulationCancelReason) {
        assert!(!self.dockspace.has_active_gesture());
        assert_eq!(
            rect_fill_count(
                &frame.output,
                self.dockspace.style().visuals.ghost_fill.unwrap()
            ),
            0
        );
        assert_eq!(
            frame.feedback,
            [DockspaceManipulationStatus::Cancelled(reason)]
        );
        assert_eq!(root_tabs(&self.dockspace), Some(vec![FIRST, SECOND]));
    }

    fn reorder_after_recovery(&mut self) {
        let ready = self.frame(Vec::new());
        let source = tab_center(&ready.output, "Second");
        let target = tab_center(&ready.output, "First");
        let _ = self.frame(vec![
            Event::PointerMoved(source),
            pointer_button(source, true),
            Event::PointerMoved(target),
            pointer_button(target, false),
        ]);
        let _ = self.frame(Vec::new());
        assert!(!self.dockspace.has_active_gesture());
        assert_eq!(root_tabs(&self.dockspace), Some(vec![SECOND, FIRST]));
    }

    fn show_child_from_parent(&self, visible: bool) {
        let mut output = run_headless(&self.context, input(Vec::new()), |ui| {
            if visible {
                ui.ctx().show_viewport_deferred(
                    self.viewport,
                    ViewportBuilder::default(),
                    |_, _| {},
                );
            }
        });
        output.textures_delta.clear();
        assert_eq!(output.viewport_output.contains_key(&self.viewport), visible);
    }
}

fn contact(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch {
        device_id: TouchDeviceId(1),
        id: TouchId(id),
        phase,
        pos,
        force: None,
    }
}

fn click(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        pointer_button(pos, true),
        pointer_button(pos, false),
    ]
}

#[test]
fn rejected_focus_loss_and_foreign_capture_end_the_previous_drag() {
    for foreign_capture in [false, true] {
        let mut harness = DragHarness::new(ViewportId::ROOT, RunMode::Owned);
        harness.start_reorder();
        let version = harness.dockspace.version();
        let mut raw = harness.input(Vec::new());
        if foreign_capture {
            raw.hovered_files.push(egui::HoveredFile::default());
        } else {
            raw.focused = false;
            raw.events.push(Event::WindowFocused(false));
        }
        harness.reject_owned_input(raw);
        assert!(harness.dockspace.has_active_gesture());
        assert_eq!(harness.dockspace.version(), version);

        let shown = harness.frame(Vec::new());
        harness.assert_cancelled(&shown);
        assert_eq!(harness.dockspace.version(), version);
        harness.reorder_after_recovery();
    }
}

#[test]
fn remounting_across_an_egui_input_gap_cancels_the_old_drag() {
    for (manual_pass, released_while_hidden) in [(false, false), (false, true), (true, true)] {
        let mut harness = DragHarness::new(ViewportId::ROOT, RunMode::Callback);
        let target = harness.start_reorder();
        let version = harness.dockspace.version();
        let release = || vec![pointer_button(target, false)];
        let raw = harness.input(if released_while_hidden {
            release()
        } else {
            Vec::new()
        });
        let mut hidden = if manual_pass {
            harness.context.begin_pass(raw);
            harness.context.end_pass()
        } else {
            run_headless(&harness.context, raw, |ui| {
                ui.label("Another workspace");
            })
        };
        hidden.textures_delta.clear();
        assert!(try_node_rect(&hidden, Role::Tab, "Second").is_none());
        assert_eq!(
            harness.context.input(|input| input.pointer.primary_down()),
            !released_while_hidden
        );

        let shown = harness.frame(Vec::new());
        harness.assert_cancelled(&shown);
        if !released_while_hidden {
            let terminal = harness.frame(release());
            assert!(
                terminal.feedback.is_empty(),
                "the old release has no remaining gesture"
            );
        }
        assert_eq!(harness.dockspace.version(), version);
        harness.reorder_after_recovery();
    }
}

#[test]
fn rolled_back_frames_cannot_lose_an_existing_drag_terminal() {
    for missed_input in [false, true] {
        let mut harness = DragHarness::new(ViewportId::ROOT, RunMode::Owned);
        let target = harness.start_reorder();
        let version = harness.dockspace.version();
        // One case misses the release entirely. The other observes it in a
        // candidate that the output hook rejects before it can commit.
        let mut release = vec![pointer_button(target, false)];
        if missed_input {
            let mut hidden = run_headless(
                &harness.context,
                harness.input(std::mem::take(&mut release)),
                |_| {},
            );
            hidden.textures_delta.clear();
        }
        harness.reject_owned_frame(release);
        assert!(
            harness.dockspace.has_active_gesture(),
            "candidate rollback is atomic"
        );
        assert_eq!(harness.dockspace.version(), version);
        assert!(!harness.context.input(|input| input.pointer.primary_down()));

        let shown = harness.frame(Vec::new());
        harness.assert_cancelled(&shown);
        assert_eq!(harness.dockspace.version(), version);
        harness.reorder_after_recovery();
    }
}

#[test]
fn a_release_injected_only_on_an_egui_repaint_pass_cancels_the_drag() {
    for mode in [RunMode::Callback, RunMode::Owned] {
        let mut harness = DragHarness::new(ViewportId::ROOT, mode);
        let target = harness.start_reorder();
        let version = harness.dockspace.version();
        let _ =
            harness.repaint_with_extra_input(RepaintInput::Event(pointer_button(target, false)));
        assert!(!harness.context.input(|state| state.pointer.primary_down()));
        assert_eq!(harness.dockspace.version(), version);

        let recovered = harness.frame(Vec::new());
        harness.assert_cancelled(&recovered);
        assert_eq!(harness.dockspace.version(), version);
        assert!(harness.frame(Vec::new()).feedback.is_empty());
        harness.reorder_after_recovery();
    }
}

#[test]
fn modifier_sources_injected_only_on_repaint_mark_the_pointer_input_gap() {
    for mode in [RunMode::Callback, RunMode::Owned] {
        for event in [
            Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: false,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            },
            Event::PointerButton {
                pos: Pos2::new(200.0, 200.0),
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: Modifiers::SHIFT,
            },
        ] {
            let mut harness = DragHarness::new(ViewportId::ROOT, mode);
            harness.start_reorder();
            let version = harness.dockspace.version();
            let _ = harness.repaint_with_extra_input(RepaintInput::Event(event));
            assert!(harness.dockspace.has_active_gesture());

            let recovered = harness.frame(Vec::new());
            harness.assert_cancelled(&recovered);
            assert_eq!(harness.dockspace.version(), version);
            harness.reorder_after_recovery();
        }
    }
}

#[test]
fn a_recovery_frame_accepts_mouse_input_after_a_touch_terminal_on_repaint() {
    for mode in [RunMode::Callback, RunMode::Owned] {
        let mut harness = DragHarness::new(ViewportId::ROOT, mode);
        let target = harness.start_touch_reorder();
        let version = harness.dockspace.version();
        let repainted = harness.repaint_with_extra_input(RepaintInput::Event(contact(
            1,
            TouchPhase::End,
            target,
        )));
        assert!(harness.dockspace.has_active_gesture());
        let close = node_center(&repainted.output, Role::Button, "Close First");

        let recovered = harness.frame(click(close));
        harness.assert_cancelled(&recovered);
        assert_eq!(
            recovered.close_requests, 1,
            "the expired contact must not suppress the new mouse click"
        );
        assert_eq!(harness.dockspace.version(), version);
    }
}

#[test]
fn transient_foreign_capture_on_an_egui_repaint_pass_cancels_the_drag() {
    for mode in [RunMode::Callback, RunMode::Owned] {
        let mut harness = DragHarness::new(ViewportId::ROOT, mode);
        harness.start_reorder();
        let version = harness.dockspace.version();
        let _ = harness.repaint_with_extra_input(RepaintInput::ForeignCapture);
        assert!(!egui::DragAndDrop::has_any_payload(&harness.context));
        let recovered = harness.frame(Vec::new());
        harness.assert_cancelled(&recovered);
        assert_eq!(harness.dockspace.version(), version);
    }
}

#[test]
fn rejected_touch_terminal_retires_only_the_affected_contact() {
    for end_dragging_contact in [true, false] {
        let mut harness = DragHarness::new(ViewportId::ROOT, RunMode::Owned);
        let target = harness.start_touch_reorder();
        // A second contact stays enrolled away from the tab strip.
        let other_point = Pos2::new(200.0, 200.0);
        let _ = harness.frame_at(
            Some(1.5),
            vec![
                contact(1, TouchPhase::Move, target),
                contact(2, TouchPhase::Start, other_point),
            ],
        );
        let version = harness.dockspace.version();

        let terminal = if end_dragging_contact {
            contact(1, TouchPhase::End, target)
        } else {
            contact(2, TouchPhase::Cancel, other_point)
        };
        harness.reject_owned_frame(vec![terminal, Event::PointerGone]);
        assert!(
            harness.dockspace.has_active_gesture(),
            "the rejected update is atomic"
        );
        assert_eq!(harness.dockspace.version(), version);

        let recovered = harness.frame(Vec::new());
        if end_dragging_contact {
            harness.assert_cancelled(&recovered);
            // Deliver the other contact's real terminal before checking mouse recovery.
            let _ = harness.frame(vec![contact(2, TouchPhase::End, other_point)]);
            harness.reorder_after_recovery();
        } else {
            assert!(recovered.feedback.is_empty());
            assert!(harness.dockspace.has_active_gesture());
            let _ = harness.frame(vec![contact(1, TouchPhase::End, target)]);
            assert_eq!(root_tabs(&harness.dockspace), Some(vec![SECOND, FIRST]));
            assert!(!harness.dockspace.has_active_gesture());
        }
    }
}

#[test]
fn a_live_egui_viewport_keeps_its_drag_but_recreating_the_same_viewport_cancels_it() {
    for reuse_pass_number in [false, true] {
        let child = ViewportId::from_hash_of("product-recreated-viewport");
        let mut harness = DragHarness::new(child, RunMode::Callback);
        harness.context.set_embed_viewports(false);
        harness.context.options_mut(|options| {
            options.max_passes = 1.try_into().unwrap();
        });
        harness.show_child_from_parent(true);
        harness.start_reorder();
        let version = harness.dockspace.version();
        let last_child_pass = harness.context.cumulative_pass_nr_for(child);

        harness.show_child_from_parent(true);
        assert_eq!(
            harness.context.cumulative_pass_nr_for(child),
            last_child_pass
        );
        assert!(harness.frame(Vec::new()).feedback.is_empty());
        assert!(harness.dockspace.has_active_gesture());
        let last_child_pass = harness.context.cumulative_pass_nr_for(child);

        harness.show_child_from_parent(false);
        harness.show_child_from_parent(true);
        assert_eq!(harness.context.cumulative_pass_nr_for(child), 0);
        if reuse_pass_number {
            for _ in 0..last_child_pass {
                let mut output = harness.context.run_ui(harness.input(Vec::new()), |_| {});
                output.textures_delta.clear();
            }
            assert_eq!(
                harness.context.cumulative_pass_nr_for(child),
                last_child_pass
            );
        }
        let shown = harness.frame(Vec::new());
        harness.assert_cancelled_with(
            &shown,
            DockspaceManipulationCancelReason::PointerProviderRetired,
        );
        assert_eq!(harness.dockspace.version(), version);
        harness.reorder_after_recovery();
    }
}

#[test]
fn a_late_output_cannot_authorize_a_close_in_a_recreated_egui_viewport() {
    let child = ViewportId::from_hash_of("product-recreated-viewport-receiver");
    let mut harness = DragHarness::new(child, RunMode::Callback);
    harness.context.set_embed_viewports(false);
    harness.context.options_mut(|options| {
        options.max_passes = 1.try_into().unwrap();
    });
    harness.show_child_from_parent(true);
    let _ = harness.frame(Vec::new());
    let (mut old, receipt) = run_ui(&harness.context, harness.input(Vec::new()), |ui| {
        harness
            .dockspace
            .show_single_surface(SURFACE, ui, &mut Panes)
            .unwrap();
    })
    .into_parts();
    old.textures_delta.clear();
    let close_position = node_center(&old, Role::Button, "Close First");
    let old_pass = harness.context.cumulative_pass_nr_for(child);

    harness.show_child_from_parent(false);
    harness.show_child_from_parent(true);
    assert_eq!(harness.context.cumulative_pass_nr_for(child), 0);
    for _ in 0..old_pass {
        let mut output = harness.context.run_ui(harness.input(Vec::new()), |_| {});
        output.textures_delta.clear();
    }
    assert_eq!(harness.context.cumulative_pass_nr_for(child), old_pass);
    receipt.report(SurfacePresentationResult::Presented);

    let mut click_close = || {
        let frame = harness.frame(click(close_position));
        assert_eq!(
            node_center(&frame.output, Role::Button, "Close First"),
            close_position
        );
        frame.close_requests
    };
    assert_eq!(
        click_close(),
        0,
        "the old output belongs to the retired viewport"
    );
    assert_eq!(
        click_close(),
        1,
        "the freshly presented output owns its close control"
    );
}

#[test]
fn a_retired_viewport_cannot_acknowledge_pane_focus_in_its_replacement() {
    for (mode, originally_available, interrupted_parent_output) in [
        (RunMode::Callback, false, false),
        (RunMode::Callback, true, false),
        (RunMode::Owned, false, false),
        (RunMode::Owned, true, false),
        (RunMode::Callback, false, true),
        (RunMode::Owned, false, true),
    ] {
        let child = ViewportId::from_hash_of("product-recreated-viewport-pane-focus");
        let mut harness = DragHarness::new(child, mode);
        harness.context.set_embed_viewports(false);
        harness.context.options_mut(|options| {
            options.max_passes = 1.try_into().unwrap();
        });
        let parent_output = harness.context.plugin_or_default::<LateDiscardPlugin>();
        harness.show_child_from_parent(true);
        let _ = harness.frame(Vec::new());
        let _ = harness.frame(Vec::new());
        let selected = harness.dockspace.select_item_current(SECOND).unwrap();
        assert!(matches!(
            selected.status(),
            DockspaceActionStatus::Applied(_)
        ));
        let mut panes = FocusPanes::new("replacement-pane-focus", originally_available);
        let run_with_focus_pane = |harness: &mut DragHarness, panes: &mut FocusPanes| {
            let raw = harness.input(Vec::new());
            match mode {
                RunMode::Callback => {
                    let mut capability = None;
                    let output = run_ui(&harness.context, raw, |ui| {
                        let response = harness
                            .dockspace
                            .show_single_surface(SURFACE, ui, panes)
                            .unwrap();
                        capability = Some(response.pane_focus_capability());
                    });
                    (output, capability.unwrap())
                }
                RunMode::Owned => {
                    let (output, response) = harness
                        .dockspace
                        .run_single_surface(&harness.context, raw, SURFACE, panes)
                        .unwrap();
                    (output, response.pane_focus_capability())
                }
            }
        };
        let (old, capability) = run_with_focus_pane(&mut harness, &mut panes);
        let expected = if originally_available {
            DockspaceCapability::Supported
        } else {
            DockspaceCapability::Unavailable(
                DockspaceUnavailableReason::PaneFocusBindingUnavailable,
            )
        };
        assert_eq!(capability, expected);
        assert_eq!(
            harness
                .context
                .memory(|memory| memory.has_focus(panes.target)),
            originally_available,
        );
        let (output, receipt) = old.into_parts();
        output.drop_without_applying_deltas();
        let late_receipt = if originally_available {
            receipt.report(SurfacePresentationResult::Presented);
            None
        } else {
            Some(receipt)
        };
        if interrupted_parent_output {
            // The root has already retired the child when its first output hook panics.
            parent_output.lock().panic_once = true;
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    harness.show_child_from_parent(false);
                }))
                .is_err()
            );
        } else {
            harness.show_child_from_parent(false);
            harness.show_child_from_parent(true);
        }
        assert_eq!(harness.context.cumulative_pass_nr_for(child), 0);
        if let Some(receipt) = late_receipt {
            receipt.report(SurfacePresentationResult::Presented);
        }
        assert!(
            !harness
                .context
                .memory(|memory| memory.has_focus(panes.target))
        );

        panes.expose_target = true;
        let (new, capability) = run_with_focus_pane(&mut harness, &mut panes);
        assert_eq!(capability, DockspaceCapability::Supported);
        let (output, receipt) = new.into_parts();
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
        assert!(
            harness
                .context
                .memory(|memory| memory.has_focus(panes.target)),
            "the replacement must receive the pending focus request (old binding available: {originally_available}, interrupted parent output: {interrupted_parent_output})"
        );
    }
}

#[test]
fn a_recreated_viewport_does_not_open_the_retired_viewports_touch_menu() {
    for mode in [RunMode::Callback, RunMode::Owned] {
        let child = ViewportId::from_hash_of("product-recreated-viewport-touch-menu");
        let mut harness = DragHarness::new(child, mode);
        harness.context.set_embed_viewports(false);
        harness.context.options_mut(|options| {
            options.max_passes = 1.try_into().unwrap();
        });
        harness.show_child_from_parent(true);
        let has_menu = |frame: &InputFrame| {
            frame
                .output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Menu && node.label() == Some("Tab actions"))
        };
        let mut time = 0.1;
        for recreated in [false, true] {
            let _ = harness.frame_at(Some(time), Vec::new());
            time += 0.1;
            let ready = harness.frame_at(Some(time), Vec::new());
            assert!(!has_menu(&ready));
            let point = tab_center(&ready.output, "Second");
            let _ = harness.frame_at(Some(time + 0.1), vec![contact(1, TouchPhase::Start, point)]);
            let _ = harness.frame_at(Some(time + 0.5), Vec::new());
            let released =
                harness.frame_at(Some(time + 0.6), vec![contact(1, TouchPhase::End, point)]);
            assert!(released.touch_menu_requested);
            assert!(!has_menu(&released));
            time += 0.8;
            if !recreated {
                harness.show_child_from_parent(false);
                harness.show_child_from_parent(true);
                assert_eq!(harness.context.cumulative_pass_nr_for(child), 0);
            }
            let mut appeared = false;
            for _ in 0..3 {
                appeared |= has_menu(&harness.frame_at(Some(time), Vec::new()));
                time += 0.05;
            }
            assert_eq!(appeared, recreated);
        }
    }
}

#[test]
fn a_recreated_viewport_does_not_inherit_the_retired_viewports_output_error() {
    let child = ViewportId::from_hash_of("product-recreated-viewport-output-error");
    let mut harness = DragHarness::new(child, RunMode::Callback);
    harness.context.set_embed_viewports(false);
    harness.context.options_mut(|options| {
        options.max_passes = 1.try_into().unwrap();
    });
    harness.show_child_from_parent(true);
    let _ = harness.frame(Vec::new());
    let _ = harness.frame(Vec::new());
    harness
        .context
        .plugin_or_default::<LateDiscardPlugin>()
        .lock()
        .strip_callback_compounds_once = true;
    let _ = harness.frame(Vec::new());
    harness.show_child_from_parent(false);
    harness.show_child_from_parent(true);

    let raw = harness.input(Vec::new());
    run_headless(&harness.context, raw, |ui| {
        let response = harness
            .dockspace
            .show_single_surface(SURFACE, ui, &mut Panes)
            .unwrap();
        assert_eq!(
            response.presentation_settlement_capability(),
            DockspaceCapability::Supported,
        );
    })
    .drop_without_applying_deltas();
}
