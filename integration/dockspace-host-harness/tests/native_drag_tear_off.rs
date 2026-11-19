//! Native drag intent preserves real hover facts and exact first-live ownership.

use std::time::Duration;

use dockspace::geometry::{LogicalRect, LogicalSize, PhysicalPoint, PhysicalRect, ScaleFactor};
use dockspace::model::{
    DockMoveSource, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    ItemId, NativeWindowPlacement, RootId, SurfaceId, WorkspaceVersion,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockspaceHostFrame, DockspaceReceiverDescriptor, DockspaceSession, HostFrameReport,
    HostWindowToken, HostWorkAreaToken, NativeCreateEffectAcknowledgement, NativeCreateReadiness,
    NativeDesktopPointerLocation, NativeDesktopPosition, NativeDragTearOff,
    NativeEffectAcknowledgement, NativeEffectOperation, NativeEffectRequest,
    NativeHostCapabilities, NativeHostCapability, NativeLocalPointerFact,
    NativeLocalPointerLocation, NativeLocalToDesktopHandoff, NativePointerButton,
    NativePointerCancelReason, NativePointerEvent, NativePointerHover, NativePointerId,
    NativePointerInput, NativePointerOwner, NativePointerRoster, NativeReceiverAnswer,
    NativeReceiverPurpose, NativeReceiverQuery, NativeSurfaceBinding, NativeWindowFacts,
    NativeWindowHit, NativeWindowInputState, NativeWindowPresentationState, NativeWindows,
    NativeWorkAreaBinding, NativeWorkAreaFacts, NativeWorkAreaRoster, PaintedSurfaceOutput,
    PointerDeviceKind, PointerEventMetadata, SurfacePaintProgramOperation, SurfacePaintProgramStep,
    SurfacePresentationArtifact, SurfacePresentationResult, SurfaceUnavailableReason,
    UniformSurfaceMetrics,
};
use dockspace_host_harness::{OwnedRecord, RecordingRenderer};

const HOST: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const RETAINED: ItemId = ItemId::new(1);
const MOVED: ItemId = ItemId::new(2);
const POINTER: NativePointerId = NativePointerId::new(7);

#[derive(Clone, Copy)]
struct Scene {
    binding: NativeSurfaceBinding,
    artifact: SurfacePresentationArtifact,
    tab: DockspaceReceiverDescriptor,
}

struct Fixture {
    session: DockspaceSession,
    root: NativeSurfaceBinding,
    scene: Scene,
    clock: u64,
}

impl Fixture {
    fn new() -> Self {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            HOST,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::central_tabs_with_selection([RETAINED, MOVED], Some(MOVED)).unwrap(),
            ),
        )])
        .unwrap();
        let mut policy = DockPolicy::default();
        policy.set_allow_native_surfaces(true);
        let mut session = DockspaceSession::from_layout(layout, policy).unwrap();
        session
            .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
            .unwrap();
        let capabilities = [
            NativeHostCapability::AuthoritativeInventory,
            NativeHostCapability::AuthoritativeButtonState,
            NativeHostCapability::PointerHitTestObservation,
            NativeHostCapability::DesktopPointerPosition,
            NativeHostCapability::HoveredWindow,
            NativeHostCapability::NativeWindowLifecycle,
            NativeHostCapability::GlobalWindowPlacement,
            NativeHostCapability::WorkArea,
        ]
        .into_iter()
        .fold(NativeHostCapabilities::none_supported(), |caps, cap| {
            caps.with(cap)
        });
        session
            .configure_managed_native_capabilities(capabilities)
            .unwrap();
        session
            .register_native_root(HOST, HostWindowToken::new(1))
            .unwrap();
        let registered = finish(frame(&mut session, None));
        let [root] = registered.native_bindings() else {
            panic!("the root must have exactly one native binding");
        };
        let root = *root;
        session
            .report_managed_native_snapshot([(root, window_facts(root_bounds()))], work_areas())
            .unwrap();
        finish(frame(&mut session, None));
        let (scene, output) = paint(&mut session, None, root, root_bounds());
        session
            .report_surface_presentation(output, SurfacePresentationResult::Presented)
            .unwrap();
        finish(frame(&mut session, Some(scene)));
        Self {
            session,
            root,
            scene,
            clock: 0,
        }
    }

    fn settle(&mut self) -> HostFrameReport {
        finish(frame(&mut self.session, Some(self.scene)))
    }

    fn work_area(&self) -> NativeWorkAreaBinding {
        self.session
            .native_work_area(HostWorkAreaToken::new(1))
            .unwrap()
    }

    fn input(
        &mut self,
        event: NativePointerEvent,
        position: NativeDesktopPosition,
        hover: NativePointerHover,
        capture: NativePointerOwner,
    ) -> NativePointerInput {
        self.clock += 10;
        let location = NativeDesktopPointerLocation::new(
            position,
            hover,
            (hover == NativePointerHover::OutsideAll).then(|| self.work_area()),
        );
        NativePointerInput::new(
            POINTER,
            event,
            location,
            NativePointerOwner::Native(self.root.window()),
            capture,
        )
        .with_metadata(PointerEventMetadata::new(
            PointerDeviceKind::Mouse,
            Duration::from_millis(self.clock),
        ))
    }

    fn press(&mut self) -> NativePointerInput {
        let point = self.scene.tab.center();
        self.input(
            NativePointerEvent::ButtonPressed(NativePointerButton::Primary),
            NativeDesktopPosition::Exact(
                PhysicalPoint::new(root_bounds().x() + point.x(), root_bounds().y() + point.y())
                    .unwrap(),
            ),
            NativePointerHover::Dock(self.root.window()),
            NativePointerOwner::Native(self.root.window()),
        )
    }

    fn outside(
        &mut self,
        event: NativePointerEvent,
        hover: NativePointerHover,
        capture: NativePointerOwner,
    ) -> NativePointerInput {
        self.input(
            event,
            NativeDesktopPosition::Exact(outside_point()),
            hover,
            capture,
        )
    }

    fn present_source(&mut self) -> HostFrameReport {
        let (scene, output) = paint(
            &mut self.session,
            Some(self.scene),
            self.root,
            root_bounds(),
        );
        self.scene = scene;
        self.session
            .report_surface_presentation(output, SurfacePresentationResult::Presented)
            .unwrap();
        self.settle()
    }

    fn start_drag(&mut self) -> NativeDragTearOff {
        let press = self.press();
        let moved = self.outside(
            NativePointerEvent::Moved,
            NativePointerHover::OutsideAll,
            NativePointerOwner::Native(self.root.window()),
        );
        self.session
            .record_native_pointer_batch([press, moved])
            .unwrap();
        assert!(self.session.native_drag_tear_off().is_none());
        assert!(self.present_source().take_native_effects().is_empty());
        let drag = self
            .session
            .native_drag_tear_off()
            .expect("the committed drag has an identity");
        let moved = self.outside(
            NativePointerEvent::Moved,
            NativePointerHover::OutsideAll,
            NativePointerOwner::Native(self.root.window()),
        );
        self.session.record_native_pointer(moved).unwrap();
        assert!(self.session.native_drag_tear_off().is_none());
        assert!(self.present_source().take_native_effects().is_empty());
        assert_eq!(self.session.native_drag_tear_off(), Some(drag));
        drag
    }

    fn foreign_release(&mut self, drag: Option<NativeDragTearOff>) -> NativePointerInput {
        self.outside(
            NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
            NativePointerHover::Foreign,
            NativePointerOwner::None,
        )
        .with_native_tear_off(self.work_area(), drag)
    }

    fn release_request(&mut self, release: NativePointerInput) -> NativeEffectRequest {
        let before = self.session.version();
        self.session.record_native_pointer(release).unwrap();
        assert!(self.session.native_drag_tear_off().is_none());
        let report = self.settle();
        self.take_request(report, before)
    }

    fn take_request(
        &mut self,
        mut report: HostFrameReport,
        before: WorkspaceVersion,
    ) -> NativeEffectRequest {
        for _ in 0..3 {
            self.assert_source(before);
            let effects = report.take_native_effects();
            if !effects.is_empty() {
                let [effect] = effects.try_into().unwrap();
                assert!(matches!(
                    effect.operation(),
                    NativeEffectOperation::CreateWindow { .. }
                ));
                return effect;
            }
            assert!(
                self.session.has_pending_presentation(),
                "release must await its exact preview"
            );
            report = self.present_source();
        }
        panic!("presenting the exact source preview must settle native release");
    }

    fn assert_source(&self, before: WorkspaceVersion) {
        assert_eq!(self.session.version(), before);
        let view = self.session.view();
        for item in [RETAINED, MOVED] {
            let location = view.item(item).unwrap();
            assert_eq!(
                (location.surface(), location.root(), location.contained()),
                (HOST, ROOT, None)
            );
        }
        assert_eq!(
            view.item(MOVED)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            &[RETAINED, MOVED]
        );
        assert_eq!(
            view.item(MOVED)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .selected()
                .and_then(|tab| tab.item()),
            Some(MOVED)
        );
    }

    fn observe(
        &mut self,
        child: NativeSurfaceBinding,
        placement: PhysicalRect,
        readiness: NativeCreateReadiness,
        acknowledgement: Option<NativeCreateEffectAcknowledgement>,
    ) {
        let presentation = if readiness == NativeCreateReadiness::Created {
            NativeWindowPresentationState::Hidden
        } else {
            NativeWindowPresentationState::Visible
        };
        self.session
            .report_managed_native_snapshot(
                [
                    (self.root, window_facts(root_bounds())),
                    (
                        child,
                        window_facts(placement)
                            .with_presentation(presentation)
                            .with_create_readiness(readiness, acknowledgement),
                    ),
                ],
                work_areas(),
            )
            .unwrap();
        let mut report = self.settle();
        assert!(report.native_admissions().is_empty());
        assert!(report.take_native_effects().is_empty());
    }
}

fn root_bounds() -> PhysicalRect {
    PhysicalRect::new(100.0, 80.0, 640.0, 480.0).unwrap()
}

fn outside_point() -> PhysicalPoint {
    PhysicalPoint::new(1_100.0, 700.0).unwrap()
}

fn window_facts(bounds: PhysicalRect) -> NativeWindowFacts {
    let scale = ScaleFactor::new(1.0).unwrap();
    NativeWindowFacts::live()
        .with_outer_bounds(bounds)
        .with_content_bounds(bounds)
        .with_native_scale_factor(scale)
        .with_presentation_scale_factor(scale)
        .with_input(NativeWindowInputState::ReceivesInput, None)
        .with_presentation(NativeWindowPresentationState::Visible)
}

fn work_areas() -> NativeWorkAreaRoster {
    NativeWorkAreaRoster::Exact(vec![NativeWorkAreaFacts::new(
        HostWorkAreaToken::new(1),
        PhysicalRect::new(0.0, 0.0, 1_920.0, 1_080.0).unwrap(),
        ScaleFactor::new(1.0).unwrap(),
    )])
}

fn frame(session: &mut DockspaceSession, scene: Option<Scene>) -> DockspaceHostFrame<'_> {
    let frame = session
        .begin_native_host_frame(move |query| match (scene, query) {
            (Some(scene), NativeReceiverQuery::WindowHit(query))
                if query.window() == scene.binding.window() =>
            {
                NativeReceiverAnswer::WindowHit(NativeWindowHit::Surface(scene.artifact))
            }
            (Some(scene), NativeReceiverQuery::Surface(query))
                if query.matches_native_binding(scene.binding) =>
            {
                if matches!(query.purpose(), NativeReceiverPurpose::HoverHit) {
                    query.clear_hover_lane(&scene.tab).map_or(
                        NativeReceiverAnswer::Unknown,
                        NativeReceiverAnswer::ClearHoverLane,
                    )
                } else {
                    query
                        .bind_receiver(&scene.tab)
                        .map_or(NativeReceiverAnswer::Unknown, NativeReceiverAnswer::Dock)
                }
            }
            _ => NativeReceiverAnswer::Unknown,
        })
        .unwrap();
    #[cfg(feature = "motion")]
    let frame = {
        let mut frame = frame;
        frame
            .set_motion(dockspace::motion::MotionFrame {
                now: Duration::from_secs(1),
                settings: dockspace::motion::MotionSettings::default(),
                preference: dockspace::motion::MotionPreference::Reduced,
            })
            .unwrap();
        frame
    };
    frame
}

fn finish(mut frame: DockspaceHostFrame<'_>) -> HostFrameReport {
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap()
}

fn paint(
    session: &mut DockspaceSession,
    scene: Option<Scene>,
    binding: NativeSurfaceBinding,
    bounds: PhysicalRect,
) -> (Scene, PaintedSurfaceOutput) {
    let mut frame = frame(session, scene);
    frame
        .measure_surface(
            binding.surface(),
            UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, bounds.width(), bounds.height()).unwrap(),
                LogicalSize::new(32.0, 24.0).unwrap(),
                80.0,
            )
            .unwrap(),
        )
        .unwrap();
    let plan = frame.paint_plan(binding.surface()).unwrap().unwrap();
    let mut tab = None;
    plan.visit_surface_program(|step| {
        if let SurfacePaintProgramStep::Operation {
            operation: SurfacePaintProgramOperation::Tab(record),
            ..
        } = step
            && record.item() == Some(MOVED)
        {
            tab = plan.receiver_for_tab_body(record);
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    let tab = tab.expect("the moved pane has a presented tab receiver");
    let (recording, receipt) = RecordingRenderer::execute(plan).unwrap();
    assert!(recording.records.contains(&OwnedRecord::Receiver(tab)));
    frame.confirm_surface_painted(receipt).unwrap();
    let mut report = finish(frame);
    assert!(report.native_admissions().is_empty());
    assert!(report.take_native_effects().is_empty());
    let [output] = report.take_painted_outputs().try_into().unwrap();
    assert!(output.matches_native_binding(binding));
    (
        Scene {
            binding,
            artifact: output.presentation_artifact(),
            tab,
        },
        output,
    )
}

fn requested_window(effect: &NativeEffectRequest) -> (NativeSurfaceBinding, PhysicalRect) {
    match effect.operation() {
        NativeEffectOperation::CreateWindow {
            binding, placement, ..
        } => (*binding, *placement),
        operation => panic!("expected native creation, got {operation:?}"),
    }
}

fn accept(effect: NativeEffectRequest) -> NativeCreateEffectAcknowledgement {
    let Some(NativeEffectAcknowledgement::Create(acknowledgement)) = effect.accepted() else {
        panic!("native creation must retain its exact acknowledgement");
    };
    acknowledgement
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct WindowKey {
    handle: u64,
    generation: u64,
}

#[test]
fn foreign_intent_adopts_the_same_resource_and_waits_for_exact_first_live() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let key = WindowKey {
        handle: 42,
        generation: 3,
    };
    let resource = Box::new(42_u64);
    let allocation = std::ptr::from_ref(resource.as_ref());
    let mut provisional = Some((drag, key, resource));
    let mut windows = NativeWindows::new();
    let moved = fixture
        .outside(
            NativePointerEvent::Moved,
            NativePointerHover::Foreign,
            NativePointerOwner::Native(fixture.root.window()),
        )
        .with_native_tear_off(fixture.work_area(), Some(drag));
    fixture.session.record_native_pointer(moved).unwrap();
    assert!(fixture.present_source().take_native_effects().is_empty());
    assert_eq!(fixture.session.native_drag_tear_off(), Some(drag));
    assert_eq!(
        fixture
            .session
            .native_pointer_capture_request()
            .unwrap()
            .source_binding(),
        fixture.root
    );
    let release = fixture.foreign_release(Some(drag));
    let effect = fixture.release_request(release);
    assert_eq!(effect.drag_tear_off(), Some(drag));
    let (child, placement) = requested_window(&effect);
    assert!(fixture.session.native_drag_tear_off().is_none());
    assert!(fixture.session.has_pending_native_create());
    let dispatch = effect.begin_dispatch();
    assert_eq!(dispatch.drag_tear_off(), Some(drag));
    let (owner, key, resource) = provisional.take().unwrap();
    assert_eq!(dispatch.drag_tear_off(), Some(owner));
    windows.insert(child, key, resource).unwrap();
    assert_eq!(windows.binding(key), Some(child.window()));
    assert_eq!(
        std::ptr::from_ref(windows.get(child).unwrap().as_ref()),
        allocation
    );
    let awaiting = dispatch.await_observation();
    assert_eq!(awaiting.drag_tear_off(), Some(drag));
    fixture.observe(
        child,
        placement,
        NativeCreateReadiness::Created,
        Some(accept(awaiting)),
    );
    fixture.assert_source(before);
    fixture.observe(child, placement, NativeCreateReadiness::Prepared, None);
    fixture.assert_source(before);
    let (_, output) = paint(&mut fixture.session, Some(fixture.scene), child, placement);
    fixture.assert_source(before);
    fixture
        .session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    let mut admitted = fixture.settle();
    assert_eq!(admitted.native_admissions(), &[child]);
    let [feedback] = admitted.manipulation_feedback() else {
        panic!("first-live admission must finish its originating manipulation");
    };
    assert_eq!(
        feedback.status(),
        dockspace::runtime::DockspaceManipulationStatus::Delivered(
            dockspace::runtime::DockspaceManipulationDelivery::Native,
        ),
    );
    assert!(!fixture.session.has_pending_native_create());
    assert_eq!(
        fixture.session.view().item(MOVED).unwrap().surface(),
        child.surface()
    );
    assert_eq!(
        fixture.session.view().item(RETAINED).unwrap().surface(),
        HOST
    );
    assert_eq!(windows.iter().count(), 1);
    assert_eq!(
        std::ptr::from_ref(windows.get(child).unwrap().as_ref()),
        allocation
    );
    for effect in admitted.take_native_effects() {
        assert!(
            matches!(effect.operation(), NativeEffectOperation::RetainChild { binding } if *binding == child)
        );
        assert!(effect.drag_tear_off().is_none());
        assert!(effect.accepted().is_none());
    }
    assert!(fixture.settle().native_admissions().is_empty());
}

#[test]
fn foreign_release_without_fresh_intent_cancels_even_after_an_authorized_move() {
    for authorize_move in [false, true] {
        let mut fixture = Fixture::new();
        let before = fixture.session.version();
        let drag = fixture.start_drag();
        if authorize_move {
            let moved = fixture
                .outside(
                    NativePointerEvent::Moved,
                    NativePointerHover::Foreign,
                    NativePointerOwner::Native(fixture.root.window()),
                )
                .with_native_tear_off(fixture.work_area(), Some(drag));
            fixture.session.record_native_pointer(moved).unwrap();
            assert!(fixture.present_source().take_native_effects().is_empty());
        }
        let release = fixture.outside(
            NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
            NativePointerHover::Foreign,
            NativePointerOwner::None,
        );
        fixture.session.record_native_pointer(release).unwrap();
        let mut report = fixture.settle();
        assert!(report.take_native_effects().is_empty());
        assert!(!fixture.session.has_active_gesture());
        assert!(!fixture.session.has_pending_native_create());
        assert!(
            report
                .manipulation_feedback()
                .iter()
                .any(|feedback| matches!(
                    feedback.status(),
                    dockspace::runtime::DockspaceManipulationStatus::Cancelled(
                        dockspace::runtime::DockspaceManipulationCancelReason::OpaquePointerBlocker,
                    )
                ))
        );
        fixture.assert_source(before);
    }
}

#[test]
fn foreign_hover_over_a_dock_window_never_queries_the_covered_receiver() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let release = fixture
        .input(
            NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
            NativeDesktopPosition::Exact(PhysicalPoint::new(420.0, 320.0).unwrap()),
            NativePointerHover::Foreign,
            NativePointerOwner::None,
        )
        .with_native_tear_off(fixture.work_area(), Some(drag));
    fixture.session.record_native_pointer(release).unwrap();
    let frame = fixture
        .session
        .begin_native_host_frame(|query| {
            panic!("Foreign must retain its point-scoped barrier over the source window: {query:?}")
        })
        .unwrap();
    let report = finish(frame);
    let effect = fixture.take_request(report, before);
    assert_eq!(effect.drag_tear_off(), Some(drag));
    fixture.assert_source(before);
    accept(effect);
}

#[test]
fn work_area_revoked_before_the_release_ordinal_cannot_authorize_foreign_placement() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let release = fixture.foreign_release(Some(drag));
    fixture
        .session
        .report_managed_native_snapshot(
            [(fixture.root, window_facts(root_bounds()))],
            NativeWorkAreaRoster::Unknown,
        )
        .unwrap();
    // Recording validates committed bindings; the reducer sees the earlier
    // revocation at this release's ordinal and must reject its old authority.
    fixture.session.record_native_pointer(release).unwrap();
    let mut report = fixture.settle();
    assert!(report.take_native_effects().is_empty());
    assert!(!fixture.session.has_pending_native_create());
    assert!(!fixture.session.has_pending_presentation());
    assert!(!fixture.session.has_active_gesture());
    fixture.assert_source(before);
}

#[test]
fn explicit_native_intent_cannot_bypass_an_in_application_window_blocker() {
    for hit in [
        NativeWindowHit::Blocked,
        NativeWindowHit::Unknown,
        NativeWindowHit::WindowBackground,
    ] {
        let mut fixture = Fixture::new();
        let before = fixture.session.version();
        let drag = fixture.start_drag();
        let release = fixture
            .input(
                NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
                NativeDesktopPosition::Exact(PhysicalPoint::new(420.0, 320.0).unwrap()),
                NativePointerHover::Dock(fixture.root.window()),
                NativePointerOwner::None,
            )
            .with_native_tear_off(fixture.work_area(), Some(drag));
        fixture.session.record_native_pointer(release).unwrap();
        let frame = fixture
            .session
            .begin_native_host_frame(|query| match query {
                NativeReceiverQuery::WindowHit(_) => NativeReceiverAnswer::WindowHit(hit),
                NativeReceiverQuery::Surface(_) => {
                    panic!("an unresolved hit cannot select covered content")
                }
            })
            .unwrap();
        let mut report = finish(frame);
        assert!(report.take_native_effects().is_empty());
        assert!(!fixture.session.has_pending_native_create());
        assert!(!fixture.session.has_pending_presentation());
        assert!(!fixture.session.has_active_gesture());
        fixture.assert_source(before);
    }
}

#[test]
fn native_intent_does_not_promote_unknown_facts_or_foreign_capture() {
    for (position, hover, capture) in [
        (
            NativeDesktopPosition::Unknown,
            NativePointerHover::Foreign,
            NativePointerOwner::None,
        ),
        (
            NativeDesktopPosition::Exact(outside_point()),
            NativePointerHover::Unknown,
            NativePointerOwner::None,
        ),
        (
            NativeDesktopPosition::Exact(outside_point()),
            NativePointerHover::Foreign,
            NativePointerOwner::Foreign,
        ),
    ] {
        let mut fixture = Fixture::new();
        let before = fixture.session.version();
        let drag = fixture.start_drag();
        let release = fixture
            .input(
                NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
                position,
                hover,
                capture,
            )
            .with_native_tear_off(fixture.work_area(), Some(drag));
        fixture.session.record_native_pointer(release).unwrap();
        let mut report = fixture.settle();
        assert!(
            report.take_native_effects().is_empty(),
            "{position:?}, {hover:?}, {capture:?}"
        );
        assert!(!fixture.session.has_pending_native_create());
        fixture.assert_source(before);
        assert!(
            fixture.present_source().take_native_effects().is_empty(),
            "{position:?}, {hover:?}, {capture:?}"
        );
        assert!(!fixture.session.has_pending_native_create());
        fixture.assert_source(before);
    }
}

#[test]
fn exact_source_release_delivery_preserves_the_existing_unknown_capture_contract() {
    let mut fixture = Fixture::new();
    let drag = fixture.start_drag();
    let release = fixture
        .outside(
            NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
            NativePointerHover::Foreign,
            NativePointerOwner::Unknown,
        )
        .with_native_tear_off(fixture.work_area(), Some(drag));
    let effect = fixture.release_request(release);
    assert_eq!(effect.drag_tear_off(), Some(drag));
    accept(effect);
}

#[test]
fn missing_release_delivery_and_capture_cannot_authorize_native_placement() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let release = NativePointerInput::new(
        POINTER,
        NativePointerEvent::ButtonReleased(NativePointerButton::Primary),
        NativeDesktopPointerLocation::new(
            NativeDesktopPosition::Exact(outside_point()),
            NativePointerHover::Foreign,
            None,
        ),
        NativePointerOwner::Unknown,
        NativePointerOwner::Unknown,
    )
    .with_native_tear_off(fixture.work_area(), Some(drag));
    fixture.session.record_native_pointer(release).unwrap();
    assert!(fixture.settle().take_native_effects().is_empty());
    assert!(!fixture.session.has_pending_native_create());
    assert!(!fixture.session.has_pending_presentation());
    assert!(fixture.present_source().take_native_effects().is_empty());
    fixture.assert_source(before);
}

#[test]
fn atomic_press_move_release_correlates_the_original_drag_without_a_queried_token() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    assert!(fixture.session.native_drag_tear_off().is_none());
    let press = fixture.press();
    let moved = fixture
        .outside(
            NativePointerEvent::Moved,
            NativePointerHover::Foreign,
            NativePointerOwner::Native(fixture.root.window()),
        )
        .with_native_tear_off(fixture.work_area(), None);
    let release = fixture.foreign_release(None);
    fixture
        .session
        .record_native_pointer_batch([press, moved, release])
        .unwrap();
    assert!(fixture.session.native_drag_tear_off().is_none());
    let report = fixture.settle();
    let effect = fixture.take_request(report, before);
    let origin = effect
        .drag_tear_off()
        .expect("the reduced drag supplies its original identity");
    let (child, placement) = requested_window(&effect);
    assert!(fixture.session.native_drag_tear_off().is_none());
    assert!(!fixture.session.has_active_gesture());
    let dispatch = effect.begin_dispatch();
    assert_eq!(dispatch.drag_tear_off(), Some(origin));
    let request = dispatch.await_observation();
    assert_eq!(request.drag_tear_off(), Some(origin));
    fixture.observe(
        child,
        placement,
        NativeCreateReadiness::Prepared,
        Some(accept(request)),
    );
    fixture.assert_source(before);
}

#[test]
fn local_to_desktop_handoff_preserves_foreign_native_intent_in_one_batch() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let coordinates = fixture
        .session
        .native_local_coordinates(fixture.root.window())
        .unwrap();
    let geometry = coordinates.geometry();
    let capture = |point| {
        coordinates
            .capture(
                fixture.root.window(),
                NativeLocalPointerFact::new(
                    point,
                    geometry.content_size(),
                    geometry.native_scale_factor(),
                ),
                geometry,
            )
            .unwrap()
    };
    let start = fixture.scene.tab.center();
    let press = NativePointerInput::new_local(
        POINTER,
        NativePointerEvent::ButtonPressed(NativePointerButton::Primary),
        NativeLocalPointerLocation::Exact(capture(
            PhysicalPoint::new(start.x(), start.y()).unwrap(),
        )),
        NativePointerOwner::Native(fixture.root.window()),
        NativePointerOwner::Native(fixture.root.window()),
    );
    let local_moved = NativePointerInput::new_local(
        POINTER,
        NativePointerEvent::Moved,
        NativeLocalPointerLocation::Exact(capture(
            PhysicalPoint::new(start.x() + 24.0, start.y()).unwrap(),
        )),
        NativePointerOwner::Native(fixture.root.window()),
        NativePointerOwner::Native(fixture.root.window()),
    );
    let point = outside_point();
    let handoff = NativePointerInput::new_handoff(
        POINTER,
        NativePointerEvent::Moved,
        NativeLocalToDesktopHandoff::new(
            capture(
                PhysicalPoint::new(point.x() - root_bounds().x(), point.y() - root_bounds().y())
                    .unwrap(),
            ),
            NativeDesktopPointerLocation::new(
                NativeDesktopPosition::Exact(point),
                NativePointerHover::Foreign,
                None,
            ),
        ),
        NativePointerOwner::Native(fixture.root.window()),
        NativePointerOwner::Native(fixture.root.window()),
    )
    .with_native_tear_off(fixture.work_area(), None);
    let release = fixture.foreign_release(None);
    fixture
        .session
        .record_native_pointer_batch([press, local_moved, handoff, release])
        .unwrap();
    let report = fixture.settle();
    let effect = fixture.take_request(report, before);
    assert!(effect.drag_tear_off().is_some());
    fixture.assert_source(before);
    accept(effect);
}

#[test]
fn a_cancelled_drag_token_cannot_authorize_a_new_drag_of_the_same_pointer() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let old = fixture.start_drag();
    let cancelled = fixture.outside(
        NativePointerEvent::StreamCancelled(
            NativePointerCancelReason::ExplicitPlatformCancellation,
        ),
        NativePointerHover::Foreign,
        NativePointerOwner::None,
    );
    fixture.session.record_native_pointer(cancelled).unwrap();
    assert!(fixture.settle().take_native_effects().is_empty());
    assert!(!fixture.session.has_active_gesture());
    assert!(fixture.session.native_drag_tear_off().is_none());
    assert!(fixture.present_source().take_native_effects().is_empty());
    let current = fixture.start_drag();
    assert_ne!(current, old);
    let release = fixture.foreign_release(Some(old));
    fixture.session.record_native_pointer(release).unwrap();
    assert!(fixture.settle().take_native_effects().is_empty());
    assert!(!fixture.session.has_active_gesture());
    assert!(!fixture.session.has_pending_native_create());
    fixture.assert_source(before);
    assert!(fixture.present_source().take_native_effects().is_empty());
    let fresh = fixture.start_drag();
    assert_ne!(fresh, current);
    let release = fixture.foreign_release(Some(fresh));
    let effect = fixture.release_request(release);
    assert_eq!(effect.drag_tear_off(), Some(fresh));
    accept(effect);
    fixture.assert_source(before);
}

#[test]
fn discarded_release_candidate_retries_one_correlated_create() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let release = fixture.foreign_release(Some(drag));
    fixture.session.record_native_pointer(release).unwrap();
    {
        let mut candidate = frame(&mut fixture.session, Some(fixture.scene));
        candidate
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
    }
    assert!(fixture.session.has_pending_native_input());
    assert!(!fixture.session.has_pending_native_create());
    assert!(fixture.session.native_drag_tear_off().is_none());
    fixture.assert_source(before);
    let report = fixture.settle();
    let effect = fixture.take_request(report, before);
    let (child, placement) = requested_window(&effect);
    assert_eq!(effect.drag_tear_off(), Some(drag));
    assert!(fixture.session.has_pending_native_create());
    assert!(!fixture.session.has_active_gesture());
    fixture.assert_source(before);
    fixture.observe(
        child,
        placement,
        NativeCreateReadiness::Prepared,
        Some(accept(effect)),
    );
    fixture.assert_source(before);
}

#[test]
fn permanently_rejected_pointer_head_preserves_create_ownership_and_cannot_replay() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let release = fixture.foreign_release(Some(drag));
    let effect = fixture.release_request(release);
    let (child, _) = requested_window(&effect);
    assert_eq!(effect.drag_tear_off(), Some(drag));
    fixture.session.record_native_pointer(release).unwrap();
    for _ in 0..2 {
        let error = fixture
            .session
            .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
            .err()
            .expect("a duplicate raw release permanently contradicts the button journal");
        assert_eq!(
            error.kind(),
            dockspace::runtime::DockspaceRuntimeErrorKind::HostProtocol
        );
        assert!(fixture.session.has_pending_native_input());
        assert!(fixture.session.has_pending_native_create());
        fixture.assert_source(before);
    }
    let result = effect
        .begin_dispatch()
        .indeterminate(dockspace::runtime::NativeIndeterminateReason::AcknowledgementLost);
    let mut reconstructed = Fixture::new();
    let failure = reconstructed
        .session
        .report_native_effect_result(result)
        .expect_err("a fresh session cannot take custody of an old affine terminal");
    let (_, result) = failure.into_parts();
    fixture.session.report_native_effect_result(result).unwrap();
    fixture
        .session
        .report_native_input_binding_retired(fixture.root)
        .unwrap();
    fixture
        .session
        .report_managed_native_snapshot(
            [
                (fixture.root, NativeWindowFacts::destroyed()),
                (child, NativeWindowFacts::destroyed()),
            ],
            NativeWorkAreaRoster::Unknown,
        )
        .unwrap();
    assert!(fixture.session.has_pending_native_input());
    assert!(fixture.session.has_pending_native_create());
    assert!(fixture.session.recognizes_native_binding(fixture.root));
    fixture.assert_source(before);
    assert!(
        fixture
            .session
            .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
            .is_err()
    );
    assert!(reconstructed.session.native_drag_tear_off().is_none());
    let fresh = reconstructed.start_drag();
    assert_ne!(fresh, drag);
    let release = reconstructed.foreign_release(Some(drag));
    let error = reconstructed
        .session
        .record_native_pointer(release)
        .expect_err("the failed generation's drag cannot name a new provider");
    assert_eq!(
        error.native_kind(),
        Some(dockspace::runtime::NativeHostErrorKind::InvalidFacts)
    );
    assert!(!reconstructed.session.has_pending_native_input());
    assert_eq!(reconstructed.session.native_drag_tear_off(), Some(fresh));
}

#[test]
fn cancelled_first_live_cannot_transfer_source_through_a_late_presented_output() {
    use dockspace::runtime::{
        DockspaceManipulationCancelReason, DockspaceManipulationStatus,
        DockspacePresentationTransitionResult,
    };

    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let release = fixture.foreign_release(Some(drag));
    fixture.session.record_native_pointer(release).unwrap();
    let release_report = fixture.settle();
    let [pending] = release_report.manipulation_feedback() else {
        panic!("the release must wait for the exact preview to be presented");
    };
    assert!(matches!(
        pending.status(),
        DockspaceManipulationStatus::Pending(_)
    ));
    let manipulation = pending.manipulation_id();
    let effect = fixture.take_request(release_report, before);
    let (child, placement) = requested_window(&effect);
    assert_eq!(effect.drag_tear_off(), Some(drag));
    fixture.observe(
        child,
        placement,
        NativeCreateReadiness::Prepared,
        Some(accept(effect)),
    );
    let (_, output) = paint(&mut fixture.session, Some(fixture.scene), child, placement);
    fixture.assert_source(before);
    let mut cancellation = frame(&mut fixture.session, Some(fixture.scene));
    assert!(cancellation.cancel_next_native_create().unwrap());
    let mut cancelled = finish(cancellation);
    let [terminal] = cancelled.presentation_transitions() else {
        panic!("the accepted transfer terminates exactly once");
    };
    assert_eq!(
        terminal.result(),
        DockspacePresentationTransitionResult::Cancelled
    );
    let [feedback] = cancelled.manipulation_feedback() else {
        panic!("the transfer terminal must also finish its originating manipulation");
    };
    assert_eq!(feedback.manipulation_id(), manipulation);
    assert_eq!(
        feedback.status(),
        DockspaceManipulationStatus::Cancelled(
            DockspaceManipulationCancelReason::ExplicitCancellation
        )
    );
    let [close] = cancelled.take_native_effects().try_into().unwrap();
    assert!(
        matches!(close.operation(), NativeEffectOperation::CompensatingClose { binding } if *binding == child)
    );
    assert!(close.drag_tear_off().is_none());
    assert!(matches!(
        close.accepted(),
        Some(NativeEffectAcknowledgement::Close(_))
    ));
    assert!(!fixture.session.has_pending_native_create());
    fixture
        .session
        .report_surface_presentation(output, SurfacePresentationResult::Presented)
        .unwrap();
    let mut late = fixture.settle();
    assert!(late.manipulation_feedback().is_empty());
    assert!(late.native_admissions().is_empty());
    assert!(late.take_native_effects().is_empty());
    fixture.assert_source(before);
}

#[test]
fn programmatic_creation_does_not_borrow_a_previous_pointer_drag_identity() {
    let mut fixture = Fixture::new();
    let before = fixture.session.version();
    let drag = fixture.start_drag();
    let cancelled = fixture.outside(
        NativePointerEvent::StreamCancelled(
            NativePointerCancelReason::ExplicitPlatformCancellation,
        ),
        NativePointerHover::Foreign,
        NativePointerOwner::None,
    );
    fixture.session.record_native_pointer(cancelled).unwrap();
    assert!(fixture.settle().take_native_effects().is_empty());
    assert!(fixture.present_source().take_native_effects().is_empty());
    let placement = PhysicalRect::new(900.0, 120.0, 420.0, 320.0).unwrap();
    let action = fixture.session.prepare_move_to_new_window(
        DockMoveSource::Item(MOVED),
        NativeWindowPlacement::new(placement),
    );
    let mut command = frame(&mut fixture.session, Some(fixture.scene));
    command.submit_prepared_action(action).unwrap();
    let mut report = finish(command);
    let [effect] = report.take_native_effects().try_into().unwrap();
    let (_, requested) = requested_window(&effect);
    assert_eq!(requested, placement);
    assert_ne!(effect.drag_tear_off(), Some(drag));
    assert_eq!(effect.drag_tear_off(), None);
    let dispatch = effect.begin_dispatch();
    assert_eq!(dispatch.drag_tear_off(), None);
    accept(dispatch.await_observation());
    fixture.assert_source(before);
}
