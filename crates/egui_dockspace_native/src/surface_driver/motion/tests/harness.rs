//! Shared-clock native frames exercised through the actual egui renderer.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dockspace::geometry::{LogicalRect, PhysicalRect, ScaleFactor};
use dockspace::model::{
    DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    ItemId, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockspaceSession, HostInputOutcome, NativeCloseState, NativeMonotonicClock,
    NativeReceiverAnswer, NativeWindowFacts, NativeWindowInputState, NativeWindowPresentationState,
    NativeWorkAreaRoster, PaintedSurfaceOutput, SurfacePaintProgramOperation,
    SurfacePaintProgramStep, SurfacePresentationResult, SurfaceUnavailableReason, SurfaceWork,
    SurfaceWorkDeferred,
};
use egui_dockspace::{DockStyle, PaneResponse};

use super::super::*;
use crate::coordinator::NativeCoordinator;
use crate::{NativeCommand, NativeWindowClosePolicy};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
pub(super) const FIRST: ItemId = ItemId::new(1);
pub(super) const MIDDLE: ItemId = ItemId::new(2);
const LAST: ItemId = ItemId::new(3);

#[derive(Default)]
pub(super) struct Panes {
    pub(super) painted: Vec<ItemId>,
}

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(format!("Native motion {item}").into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        self.painted.push(item);
        PaneResponse::from(&ui.label("Content is laid out at its sampled size."))
    }
}

#[derive(Debug)]
pub(super) struct Pose {
    pub(super) panes: BTreeMap<ItemId, LogicalRect>,
    pub(super) partition: bool,
    pub(super) motion: MotionFrame,
}

pub(super) struct Painted {
    pub(super) poses: Vec<Pose>,
    output: PaintedSurfaceOutput,
    pub(super) decoration_active: bool,
}

pub(super) struct Harness {
    pub(super) state: NativeRuntimeState<Panes>,
    pub(super) context: egui::Context,
    millis: Arc<AtomicU64>,
}

impl Harness {
    pub(super) fn new() -> Self {
        let content = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [FIRST, MIDDLE, LAST].map(|item| DockspaceNode::tabs([item])),
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap();
        let session = DockspaceSession::from_layout_with_presentation_config(
            layout.clone(),
            DockPolicy::default(),
            egui_dockspace::DockStyle::default()
                .presentation_config()
                .unwrap(),
        )
        .unwrap();
        let mut state = NativeRuntimeState::new(
            egui::Id::unique("native-motion-renderer"),
            session,
            SURFACE,
            Panes::default(),
            DockStyle::default(),
            NativeWindowClosePolicy::Cancel,
        )
        .unwrap();
        let millis = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&millis);
        let clock = NativeMonotonicClock::new(move || {
            Duration::from_millis(observed.load(Ordering::SeqCst))
        });
        let mut session = DockspaceSession::from_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            egui_dockspace::DockStyle::default()
                .presentation_config()
                .unwrap(),
        )
        .unwrap();
        session
            .configure_output_capacity(NonZeroUsize::new(1).unwrap())
            .unwrap();
        state.coordinator = NativeCoordinator::new_with_clock(session, clock).unwrap();
        state
            .coordinator
            .register_native_root(SURFACE, crate::surface_driver::ROOT_WINDOW_TOKEN)
            .unwrap();
        let mut frame = state
            .coordinator
            .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        let report = frame.commit().unwrap();
        let binding = report
            .inputs()
            .iter()
            .find_map(|input| match input {
                HostInputOutcome::NativeSurfaceRegistered { binding } => Some(*binding),
                _ => None,
            })
            .unwrap();
        state
            .coordinator
            .bind_viewport(
                egui::ViewportId::ROOT,
                winit::window::WindowId::from(1),
                binding,
            )
            .unwrap();
        state.coordinator.settle_host_frame_report(&report).unwrap();
        let bounds = PhysicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
        let scale = ScaleFactor::new(1.0).unwrap();
        state
            .coordinator
            .report_snapshot(
                [(
                    binding,
                    NativeWindowFacts::live()
                        .with_content_bounds(bounds)
                        .with_outer_bounds(bounds)
                        .with_native_scale_factor(scale)
                        .with_presentation_scale_factor(scale)
                        .with_input(NativeWindowInputState::ReceivesInput, None)
                        .with_presentation(NativeWindowPresentationState::Visible)
                        .with_close(NativeCloseState::Clear, None),
                )],
                NativeWorkAreaRoster::Unknown,
                crate::capabilities::capabilities_for_backend(
                    eframe::NativeWindowingBackend::Windows,
                ),
            )
            .unwrap();
        let context = egui::Context::default();
        context.options_mut(|options| options.max_passes = NonZeroUsize::new(2).unwrap());
        let mut harness = Self {
            state,
            context,
            millis,
        };
        for now in [0, 16, 32] {
            let painted = harness.paint(now, false).unwrap();
            assert!(!harness.active());
            harness.present(painted);
        }
        harness
    }

    pub(super) fn active(&self) -> bool {
        self.state
            .coordinator
            .session()
            .surface_motion_is_active(SURFACE)
    }

    pub(super) fn visibility(&mut self, visible: bool) -> crate::NativeActionReceipt {
        self.state
            .request_command(NativeCommand::SetItemVisibility {
                item: MIDDLE,
                visible,
            })
            .unwrap()
    }

    pub(super) fn present(&mut self, painted: Painted) {
        self.state
            .coordinator
            .report_model_presentation_for_test(
                painted.output,
                SurfacePresentationResult::Presented,
            )
            .unwrap();
    }

    pub(super) fn paint(&mut self, now: u64, repeat: bool) -> Option<Painted> {
        self.millis.store(now, Ordering::SeqCst);
        self.state.panes.painted.clear();
        let action = self.state.application_actions.take();
        let published = self.state.coordinator.session().version();
        let bridge = self.state.coordinator.dispatch_bridge();
        let mut frame = self
            .state
            .coordinator
            .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
            .unwrap();
        let mut submission =
            action.map(|action| action.submit(&mut frame.frame, published).unwrap());
        let configuration = submission
            .as_ref()
            .and_then(NativeApplicationSubmission::encoding_motion)
            .unwrap_or(self.state.motion);
        let mut submitted = None;
        let mut poses = Vec::new();
        let mut paint = None;
        let mut blocked = false;
        let mut passes = 0;
        let mut output = self.context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                time: Some(Duration::from_millis(now).as_secs_f64()),
                ..Default::default()
            },
            |ui| {
                if passes == 0 {
                    frame
                        .frame
                        .set_motion(configuration.frame(bridge.observation_time(), ui))
                        .unwrap();
                    submitted = submission
                        .take()
                        .map(|submission| submission.finish(&mut frame.frame).unwrap());
                    frame.frame.begin_encoding_pass().unwrap();
                } else {
                    frame.frame.restart_encoding_pass().unwrap();
                }
                passes += 1;
                let bounds = ui.available_rect_before_wrap();
                frame
                    .measure_surface(
                        SURFACE,
                        ui,
                        bounds,
                        bounds,
                        &self.state.panes,
                        &self.state.style,
                    )
                    .unwrap();
                let plan = match frame.frame.surface_work(SURFACE).unwrap() {
                    SurfaceWork::Ready(plan) => plan,
                    SurfaceWork::Deferred(SurfaceWorkDeferred::OutputCapacity) => {
                        blocked = true;
                        return;
                    }
                    SurfaceWork::Deferred(reason) => {
                        panic!("measured native motion surface must be ready: {reason:?}")
                    }
                };
                let mut panes = BTreeMap::new();
                plan.visit_surface_program(|step| {
                    if let SurfacePaintProgramStep::Operation {
                        operation: SurfacePaintProgramOperation::Pane(pane),
                        ..
                    } = step
                    {
                        panes.insert(pane.item(), pane.bounds());
                    }
                    Ok::<(), ()>(())
                })
                .unwrap();
                poses.push(Pose {
                    panes,
                    partition: plan.partition_available(ROOT),
                    motion: plan.motion_frame().unwrap(),
                });
                paint = Some(
                    self.state
                        .paint_scope
                        .paint_surface(
                            &mut frame,
                            SURFACE,
                            ui,
                            &mut self.state.panes,
                            &self.state.style,
                        )
                        .unwrap(),
                );
                if repeat && poses.len() == 1 {
                    self.millis.fetch_add(73, Ordering::SeqCst);
                    ui.ctx()
                        .request_discard("the native run must retain one frozen motion sample");
                }
            },
        );
        let decoration_active = if let Some(paint) = &mut paint {
            assert!(paint.validate_final_output(&mut output));
            assert!(paint.confirm_into(&mut frame.frame).unwrap());
            paint.motion_needs_repaint()
        } else {
            false
        };
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        let mut report = frame.commit().unwrap();
        self.state
            .publish_application_configuration(&self.context, &mut submitted, &report)
            .unwrap();
        self.state
            .application_actions
            .settle_submission(submitted, &report)
            .unwrap();
        self.state
            .coordinator
            .settle_host_frame_report(&report)
            .unwrap();
        output.textures_delta.clear();
        let mut outputs = report.take_painted_outputs();
        if blocked {
            assert!(outputs.is_empty());
            assert!(self.state.panes.painted.is_empty());
            None
        } else {
            assert_eq!(outputs.len(), 1);
            Some(Painted {
                poses,
                output: outputs.remove(0),
                decoration_active,
            })
        }
    }
}
