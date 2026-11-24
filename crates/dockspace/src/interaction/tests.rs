use super::*;
use crate::graph::workspace::WorkspaceIndex;
use crate::graph::{ContainedFloating, Node, RootRecord, SurfacePresentation, Workspace};
use crate::ids::ItemId;
use crate::ids::{EngineAuthorityDomainId, ReducerTickId, WorkspaceRevision};
use crate::input::intent::Authority;
use crate::input::journal::{PointerInputLease, PointerProviderScope, PointerStreamId};
use crate::interaction::targeting::guide::{DropGuideClusterRecord, DropGuideTargetRecord};
use crate::interaction::targeting::resolver::{DropResolution, resolve_presented_drop};
use crate::interaction::targeting::targets::{
    DropTargetAvailability, DropTargetLocation, DropTargetRecord, DropVisual, SceneLayerKey,
};
use crate::platform::WindowCoordinateObservation;
use crate::platform::coordinates::CoordinateSnapshot;
use crate::platform::viewport::{
    CoordinateGeneration, CoordinateObservationGeneration, WindowIncarnation, WindowToken,
};
use crate::policy::DockPolicySnapshot;
use crate::presentation::config::PresentationConfigRevision;
use crate::presentation::hit::region::HitRegion;
use crate::presentation::hit::{PresentationHitManifest, PresentationHitRegionKind};
use crate::presentation::manifest::{SurfaceRequirementRevision, SurfaceSceneRevision};
use crate::presentation::observation::{
    PresentationLedger, PresentationOutputSerial, SurfacePresentationOutputTicket,
};
use crate::presentation::scene::{
    PresentationPlan, SurfaceProgramRootOperation, SurfaceProgramRootPresentation, TabBarSceneId,
    TabStripControlRecord,
};
use crate::presentation::tab_strip::{TabStripControlId, TabStripStateKey};

fn frozen_scene(surface: SurfaceId, epoch: WorkspaceEpoch) -> SurfaceSceneStamp {
    SurfaceSceneStamp::new(
        SurfaceMeasurementTicket::new(
            EngineAuthorityDomainId::new_for_test(90),
            epoch,
            PresentationConfigRevision::new(1),
            PolicyRevision::new(1),
            SurfaceRequirementRevision::new(1),
            surface,
        ),
        SurfaceSceneRevision::new(1),
    )
}

fn frozen_presentation(surface: SurfaceId, epoch: WorkspaceEpoch) -> FrozenPresentationAuthority {
    let output = SurfacePresentationOutputTicket::mint(
        EngineAuthorityDomainId::new_for_test(90),
        PresentationOutputSerial::new_for_test(1),
        frozen_scene(surface, epoch),
    );
    FrozenPresentationAuthority::new(
        PresentedSurfaceAuthority::mint_observed_for_test(output, CoordinateGeneration::new(1)),
        PopupRoutingRevision::default(),
    )
}

fn click_start_fixture() -> (ClickStart, Arc<FrozenClickAction>) {
    let epoch = WorkspaceEpoch::new(9);
    let surface = SurfaceId::new(10);
    let root = RootId::new(11);
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([crate::ids::ItemId::new(12)]));
    let bar = TabBarSceneId { root, tabs };
    let bounds = LogicalRect::new(0.0, 0.0, 40.0, 20.0).expect("test control bounds must be valid");
    let record = TabStripControlRecord::new(
        TabStripControlId::TabListMenu(bar),
        bounds,
        true,
        SceneLayerKey::surface_base(),
    );
    let mut plan = PresentationPlan::new(surface, bounds);
    plan.push_tab_strip_control_record(record);
    plan.begin_surface_program_root(root, SurfaceProgramRootPresentation::Main)
        .expect("test control root opens");
    plan.push_surface_program_root_operation(SurfaceProgramRootOperation::TabStripControl(
        record.id(),
    ))
    .expect("test control enters the surface program");
    plan.end_surface_program_root(root)
        .expect("test control root closes");
    plan.finish_surface_program()
        .expect("test control program finishes");
    plan.link_surface_program()
        .expect("test control program links its actual record row");
    let authority = EngineAuthorityDomainId::new_for_test(13);
    let ticket = SurfaceMeasurementTicket::new(
        authority,
        epoch,
        PresentationConfigRevision::new(1),
        crate::policy::PolicyRevision::new(1),
        SurfaceRequirementRevision::new(1),
        surface,
    );
    let stamp = SurfaceSceneStamp::new(ticket, SurfaceSceneRevision::new(1));
    let output = SurfacePresentationOutputTicket::mint(
        authority,
        PresentationOutputSerial::new_for_test(1),
        stamp,
    );
    let manifest = PresentationHitManifest::compile(output, &plan)
        .expect("test plan should resolve through its surface program");
    let region = manifest
        .regions()
        .iter()
        .find(|region| {
            matches!(
                region.id().kind(),
                PresentationHitRegionKind::TabStripControl(TabStripControlId::TabListMenu(
                    actual
                )) if actual == bar
            )
        })
        .expect("compiled control must publish one click region")
        .id();
    let lease = PointerInputLease::new(authority, 1, PointerProviderScope::DesktopGlobal);
    let owner = GestureOwner::new(
        PointerStreamId::new(lease, PointerId::new(14), 1),
        GestureLane::Desktop,
    );
    let action = Arc::new(FrozenClickAction::TabStripControl(
        FrozenTabStripControlClick {
            key: TabStripStateKey::new(surface, bar),
            record,
            output: None,
        },
    ));
    (
        ClickStart {
            epoch,
            owner,
            capture_authority: Authority::Unknown(
                crate::input::intent::AuthorityUnavailableReason::ProviderUnavailable,
            ),
            button: PointerButton::Primary,
            region,
            measurement: ticket,
            coordinates: SurfaceCoordinateCapture::Headless {
                authority_generation: CoordinateGeneration::new(1),
            },
            presentation: frozen_presentation(surface, epoch),
            action: Arc::clone(&action),
        },
        action,
    )
}

fn native_local_drag_start_fixture(
    contained_intent: Option<ContainedDragIntent>,
) -> (DragArmStart, Workspace) {
    let epoch = WorkspaceEpoch::new(23);
    let surface = SurfaceId::new(21);
    let root = RootId::new(22);
    let target_root = RootId::new(24);
    let target_surface = SurfaceId::new(25);
    let floating = FloatingPresentationId::new(26);
    let source_rect =
        LogicalRect::new(0.0, 0.0, 480.0, 360.0).expect("test source bounds must be valid");
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([ItemId::new(1), ItemId::new(2)]));
    let target_tabs = builder.insert_node(Node::tabs([ItemId::new(3)]));
    builder.set_root(root, RootRecord::new(tabs));
    builder.set_root(target_root, RootRecord::new(target_tabs));
    if contained_intent.is_some() {
        builder.set_surface(surface, SurfacePresentation::rootless());
        builder.set_contained_floating(floating, ContainedFloating::new(root, source_rect));
        builder
            .attach_contained(surface, floating)
            .expect("test contained source surface must exist");
    } else {
        builder.set_surface(surface, SurfacePresentation::with_main(root));
    }
    builder.set_surface(target_surface, SurfacePresentation::with_main(target_root));
    let workspace = builder.build().expect("test drag workspace must be valid");
    let source = workspace
        .capture_node_source(root, tabs)
        .expect("test drag source must be current");
    let presentation = frozen_presentation(surface, epoch);
    let domain = EngineAuthorityDomainId::new_for_test(90);
    let host = PresentationLedger::new(domain)
        .create_host()
        .expect("test joined host must mint");
    let lease = PointerInputLease::new(domain, 1, PointerProviderScope::NativeJoined(host));
    let binding = ViewportBinding::new(
        domain,
        epoch,
        surface,
        WindowToken::new(1),
        WindowIncarnation::new(1),
    );
    let generation = CoordinateGeneration::new(1);
    let owner = GestureOwner::new(
        PointerStreamId::new(lease, PointerId::new(1), 1),
        GestureLane::NativeLocal {
            binding: binding.window(),
            coordinate_generation: generation,
        },
    );
    let coordinates = SurfaceCoordinateCapture::NativeReady {
        coordinates: CoordinateSnapshot::from_observation(
            binding,
            generation,
            WindowCoordinateObservation::new(
                binding,
                CoordinateObservationGeneration::new(1),
                Authority::Known(
                    PhysicalRect::new(400.0, -300.0, 1200.0, 900.0)
                        .expect("test content bounds must be valid"),
                ),
                Authority::Unknown(crate::input::intent::AuthorityUnavailableReason::NotReported),
                Authority::Known(ScaleFactor::new(2.0).expect("test native scale is valid")),
                Authority::Known(ScaleFactor::new(2.5).expect("test presentation scale is valid")),
            ),
        )
        .expect("test source coordinates must be complete"),
        authority_generation: generation,
    };
    let payload = MovePayload::Tabs(source.clone());
    let initial_pointer = LogicalPoint::new(4.0, 3.0).expect("test pointer is finite");
    let version = WorkspaceVersion::new(epoch, WorkspaceRevision::new(7));
    let minimum_size = LogicalSize::new(120.0, 90.0).expect("test source minimum must be valid");
    let origin = match contained_intent {
        Some(intent) => FrozenDragOrigin::Contained(FrozenContainedDragOrigin {
            surface,
            root,
            floating,
            intent,
            source_rect,
            source_roster: workspace
                .capture_contained_roster(surface)
                .expect("test contained roster must be current"),
            initial_pointer,
            minimum_size,
        }),
        None => FrozenDragOrigin::Workspace,
    };
    let continuation = SceneGestureContinuationDraft {
        activation: ReductionCause::SurfaceContribution {
            tick: ReducerTickId::new(9),
            surface,
            base: frozen_scene(surface, epoch),
        },
        owner,
        origin: presentation,
        workspace: version,
        policy: PolicyRevision::new(1),
        config: PresentationConfigRevision::new(1),
        requirements: RequirementRevision::new(4),
        popup_routing: PopupRoutingRevision::new_for_test(5),
        source: SceneGestureContinuationSource::Drag {
            payload: payload.clone(),
            source_surface: surface,
            complete_root: Some(source.clone()),
            origin: origin.clone(),
            coordinates,
        },
    };
    (
        DragArmStart {
            epoch,
            owner,
            journal_capture_authority: Authority::Known(PointerCaptureOwner::Native(
                binding.window(),
            )),
            button: PointerButton::Primary,
            payload,
            source_surface: surface,
            initial_pointer: Some(initial_pointer),
            journal_threshold_origin: JournalDragThresholdOrigin::NativeLocal {
                binding,
                coordinate_generation: generation,
                position: initial_pointer,
            },
            complete_root: Some(source),
            partial_detachable: false,
            origin,
            source_validated_at: version,
            presentation,
            source_layout_facts: None,
            journal_source_geometry: Some(JournalDragSourceGeometry {
                source_rect,
                minimum_size,
            }),
            continuation: Some(continuation),
        },
        workspace,
    )
}

fn active_native_local_drag_fixture(
    contained_intent: Option<ContainedDragIntent>,
) -> (InteractionState, DragSessionId, Workspace) {
    let (start, workspace) = native_local_drag_start_fixture(contained_intent);
    let owner = start.owner;
    let mut state = InteractionState::default();
    let (session, replaced) = state.arm_drag(start).expect("test drag must arm");
    assert_eq!(replaced, None);
    state
        .begin_drag(session, owner, PointerButton::Primary)
        .expect("test drag must begin");
    (state, session, workspace)
}

#[test]
fn click_generation_exhaustion_leaves_state_atomic() {
    let (start, _) = click_start_fixture();
    let mut state = InteractionState {
        last_click_generation: ClickGeneration::new(u64::MAX),
        ..InteractionState::default()
    };
    let before = state.clone();

    assert_eq!(
        state.begin_click(start),
        Err(InteractionCounterError::ClickGenerationExhausted)
    );
    assert_eq!(state, before);
}

#[test]
fn active_click_clones_share_the_frozen_action() {
    let (start, action) = click_start_fixture();
    let mut state = InteractionState::default();
    state.begin_click(start).expect("test click must begin");
    assert_eq!(Arc::strong_count(&action), 2);

    let clone = state.clone();
    assert_eq!(Arc::strong_count(&action), 3);
    drop(clone);
    assert_eq!(Arc::strong_count(&action), 2);
}

#[test]
fn a_gesture_freezes_its_lane_but_the_next_gesture_can_reuse_the_stream() {
    let (mut start, _) = click_start_fixture();
    let domain = start.owner.lease().authority_domain();
    let host = PresentationLedger::new(domain)
        .create_host()
        .expect("test presentation host must be available");
    let lease = PointerInputLease::new(domain, 1, PointerProviderScope::NativeJoined(host));
    let stream = PointerStreamId::new(lease, start.owner.pointer(), 1);
    let binding = ViewportBinding::new(
        domain,
        start.epoch,
        start.presentation.surface(),
        WindowToken::new(1),
        WindowIncarnation::new(1),
    );
    let owner = GestureOwner::new(
        stream,
        GestureLane::NativeLocal {
            binding: binding.window(),
            coordinate_generation: CoordinateGeneration::new(1),
        },
    );
    start.owner = owner;
    let mut state = InteractionState::default();
    let (session, _) = state
        .begin_click(start.clone())
        .expect("test native-local click must begin");
    let frozen = state.clone();

    for incompatible in [
        GestureOwner::new(stream, GestureLane::Desktop),
        GestureOwner::new(
            stream,
            GestureLane::NativeLocal {
                binding: binding.window(),
                coordinate_generation: CoordinateGeneration::new(2),
            },
        ),
    ] {
        assert_eq!(state.cancel_owner(incompatible), None);
        assert_eq!(
            state.take_click_for_release(session, incompatible, PointerButton::Primary),
            Err(InteractionRejection::PointerStreamMismatch {
                expected: stream,
                actual: stream,
            })
        );
        assert_eq!(state, frozen);
    }
    assert_eq!(state.active_owner(), Some(owner));
    assert_eq!(state.active_stream(), Some(stream));
    assert_eq!(
        state
            .take_click_for_release(session, owner, PointerButton::Primary)
            .expect("the exact owner can release")
            .owner,
        owner
    );
    assert_eq!(state.active_stream(), None);

    let desktop_owner = GestureOwner::new(stream, GestureLane::Desktop);
    start.owner = desktop_owner;
    let (next_session, _) = state
        .begin_click(start)
        .expect("the next gesture can choose the desktop lane");
    assert_ne!(next_session, session);
    assert_eq!(state.active_stream(), Some(stream));
    assert_eq!(state.active_owner(), Some(desktop_owner));
    assert_eq!(state.cancel_owner(owner), None);
    assert_eq!(state.active_owner(), Some(desktop_owner));
    state
        .take_click_for_release(next_session, desktop_owner, PointerButton::Primary)
        .expect("the new exact owner can release");
    assert_eq!(state.status(), InteractionStatus::Idle);
}

#[test]
fn desktop_handoff_rebinds_the_source_and_discards_old_target_facts_atomically() {
    let (mut state, session, workspace) = active_native_local_drag_fixture(None);
    let drag = state.active_drag(session).expect("test drag is active");
    let owner = drag.owner;
    let source = drag
        .complete_root
        .as_ref()
        .expect("test source is complete");
    let geometry = drag
        .journal_source_geometry
        .expect("test source has geometry");
    let source_surface = drag.source_surface;
    let source_root = source.root();
    let anchor = SurfacePointer::new(
        source_surface,
        drag.initial_pointer.expect("test source has a pointer"),
    );
    state
        .freeze_journal_presentation_reservation(
            session,
            owner,
            source_surface,
            source_root,
            FloatingPresentationId::new(27),
            anchor,
            geometry.source_rect,
            geometry.minimum_size,
            false,
        )
        .expect("test source reservation must freeze");

    let target_surface = SurfaceId::new(25);
    let target_root = RootId::new(24);
    let target_tabs = workspace
        .root(target_root)
        .expect("test target root exists")
        .node;
    let presentation = frozen_presentation(target_surface, session.epoch());
    let scene = frozen_scene(target_surface, session.epoch());
    let bounds =
        LogicalRect::new(0.0, 0.0, 300.0, 200.0).expect("test target bounds must be valid");
    let point = LogicalPoint::new(100.0, 100.0).expect("test target point is finite");
    let target = Arc::new(DropTargetRecord::new(
        DropTargetLocation::Center {
            surface: target_surface,
            root: target_root,
            tabs: target_tabs,
        },
        DropTargetAvailability::Available,
        HitRegion::new(bounds),
        SceneLayerKey::surface_base(),
        DropVisual::new(bounds),
    ));
    let mut plan = PresentationPlan::new(target_surface, bounds);
    plan.bind_command_basis(
        target_root,
        workspace
            .root_fingerprint(target_root)
            .expect("test target root is current"),
    );
    plan.push_drop_target(Arc::clone(&target));
    plan.push_drop_guide_cluster(DropGuideClusterRecord::inner_center(
        target_surface,
        target_root,
        target_tabs,
        HitRegion::new(bounds),
        SceneLayerKey::surface_base(),
        DropGuideTargetRecord::new(target, bounds),
    ));
    let drag = state.active_drag(session).expect("test drag is active");
    let index = WorkspaceIndex::build(&workspace, drag.source_validated_at)
        .expect("test workspace index must build");
    let (resolution, affordance, observation) = resolve_presented_drop(
        scene,
        &plan,
        None,
        &workspace,
        drag.source_validated_at,
        &index,
        &DockPolicySnapshot::default(),
        session,
        drag.payload.clone(),
        None,
        point,
    )
    .expect("test target must resolve")
    .into_evaluation_parts();
    let DropResolution::Resolved(resolved) = resolution else {
        panic!("test center target must accept the source");
    };
    let acquisition = observation.acquisition().expect("test target is acquired");
    let (preview, _) = state
        .publish_preview(
            session,
            scene,
            PreviewVisual::Dock {
                surface: target_surface,
                target: resolved.target_id(),
                rect: resolved.visual().rect(),
            },
            PreviewProof::Dock {
                target: resolved.target_id(),
                command: resolved.command().clone(),
            },
            Some(acquisition),
            resolved.preview_tab_strip(),
        )
        .expect("test preview must publish");
    state
        .bind_preview_layout_basis(session, Some(scene))
        .expect("test preview must retain its layout basis");
    assert!(state.observe_presented_preview(target_surface, preview.token()));
    state
        .set_drop_affordance(session, Some(affordance.expect("test guides are visible")))
        .expect("test guide affordance must install");
    let GestureLane::NativeLocal {
        binding,
        coordinate_generation,
    } = owner.lane()
    else {
        panic!("test owner must be native-local");
    };
    let local_point = LogicalPoint::new(42.0, 18.0).expect("test local point is finite");
    let route = crate::input::journal::NativeSurfacePointerRoute::new(
        ViewportBinding::from_window(binding, state.active_surface().unwrap()),
        coordinate_generation,
        PhysicalPoint::new(105.0, 45.0).expect("test client point is finite"),
        local_point,
    );
    state
        .set_drag_position(
            session,
            ManipulationPositionKey::JournalNativeLocal {
                manipulation: Some(route),
                hover: Some(route),
            },
        )
        .expect("test local position must advance");
    let token = state
        .mint_continuation(
            owner,
            PointerContinuationSession::Drag(session),
            std::time::Duration::from_millis(100),
        )
        .expect("test timed continuation must mint");
    let drag = state.active_drag_mut(session).expect("test drag is active");
    drag.timed = Some(DragTimedContinuation::HoverSelection(HoverSelectionState {
        request: PointerContinuationRequest {
            token,
            surface: target_surface,
            point,
        },
        target: TabSceneId {
            root: target_root,
            tabs: target_tabs,
            tab: ItemId::new(3).into(),
        },
        authority: presentation.presented,
        workspace: drag.source_validated_at,
        started_at: std::time::Duration::ZERO,
    }));
    assert_eq!(drag.current_pointer(), Some(local_point));
    let no_candidate = TargetObservation::surface(scene, SceneLayerKey::surface_base(), None);
    assert_eq!(
        state.reconcile_drag_target(session, no_candidate, Some(point), 8.0),
        Ok(TargetLatchDecision::RetainCurrent)
    );

    let desktop_owner = GestureOwner::new(owner.stream(), GestureLane::Desktop);
    let mut expected = state.clone();
    let drag = expected
        .active_drag_mut(session)
        .expect("test drag is active");
    drag.owner = desktop_owner;
    drag.continuation
        .as_mut()
        .expect("test source has a continuation")
        .draft
        .owner = desktop_owner;
    drag.timed = None;
    drag.target.last_advanced = None;
    drag.target.last_logical_point = None;
    drag.target.affordance = None;
    drag.target.preview = DragPreviewState::Unresolved {
        status: PreviewResolutionStatus::Unavailable,
    };

    assert_eq!(state.upgrade_drag_to_desktop(session, owner), Ok(()));
    assert_eq!(state, expected);
    assert_eq!(state.active_stream(), Some(owner.stream()));
    let drag = state
        .active_drag(session)
        .expect("same drag remains active");
    assert_eq!(drag.owner.lease(), owner.lease());
    assert_eq!(drag.owner.pointer(), owner.pointer());
    assert_eq!(drag.target_acquisition(), None);
    assert_eq!(drag.target.current_pointer(None), None);
    assert_eq!(drag.current_pointer(), drag.initial_pointer);
    assert!(!state.observe_presented_preview(target_surface, preview.token()));
    assert_eq!(
        state.reconcile_drag_target(session, no_candidate, Some(point), 8.0),
        Ok(TargetLatchDecision::AcceptCandidate)
    );
    let released = state
        .take_drag_for_release(session, desktop_owner, PointerButton::Primary)
        .expect("upgraded owner must release the same session");
    assert_eq!(released.session, session);
    assert_eq!(released.owner.stream(), owner.stream());
    assert_eq!(state.status(), InteractionStatus::Idle);
}

#[test]
fn desktop_handoff_also_accepts_a_drag_without_a_source_continuation() {
    let (mut start, _) = native_local_drag_start_fixture(None);
    start.continuation = None;
    let owner = start.owner;
    let mut state = InteractionState::default();
    let (session, _) = state.arm_drag(start).expect("test drag must arm");
    state
        .begin_drag(session, owner, PointerButton::Primary)
        .expect("test drag must begin");
    assert_eq!(state.upgrade_drag_to_desktop(session, owner), Ok(()));
    assert_eq!(state.active_scene_gesture_continuation(), None);
    assert_eq!(state.active_stream(), Some(owner.stream()));
    assert_eq!(
        state.active_owner(),
        Some(GestureOwner::new(owner.stream(), GestureLane::Desktop))
    );
    let before_retry = state.clone();
    assert_eq!(
        state.upgrade_drag_to_desktop(session, owner),
        Err(InteractionRejection::SessionMismatch)
    );
    assert_eq!(state, before_retry);
}

#[test]
fn desktop_handoff_rejects_wrong_session_owner_or_lane_without_mutation() {
    let (state, session, _) = active_native_local_drag_fixture(None);
    let owner = state.active_owner().expect("test drag has an owner");
    let GestureLane::NativeLocal {
        binding,
        coordinate_generation,
    } = owner.lane()
    else {
        panic!("test owner must be native-local");
    };
    let wrong_session = DragSessionId::new(session.epoch(), DragGeneration::new(2));
    let wrong_stream = PointerStreamId::new(owner.lease(), PointerId::new(2), 1);
    let wrong_provider =
        PointerInputLease::new(owner.lease().authority_domain(), 2, owner.lease().scope());
    for (requested_session, requested_owner) in [
        (wrong_session, owner),
        (
            session,
            GestureOwner::new(owner.stream(), GestureLane::Desktop),
        ),
        (
            session,
            GestureOwner::new(
                owner.stream(),
                GestureLane::NativeLocal {
                    binding,
                    coordinate_generation: coordinate_generation
                        .checked_next()
                        .expect("test coordinate generation must advance"),
                },
            ),
        ),
        (session, GestureOwner::new(wrong_stream, owner.lane())),
        (
            session,
            GestureOwner::new(
                PointerStreamId::new(wrong_provider, owner.pointer(), 1),
                owner.lane(),
            ),
        ),
    ] {
        let mut candidate = state.clone();
        assert_eq!(
            candidate.upgrade_drag_to_desktop(requested_session, requested_owner),
            Err(InteractionRejection::SessionMismatch)
        );
        assert_eq!(candidate, state);
    }
    for lane in [
        GestureLane::Desktop,
        GestureLane::HostLocal,
        GestureLane::SurfaceLocal,
    ] {
        let mut candidate = state.clone();
        let nonlocal_owner = GestureOwner::new(owner.stream(), lane);
        let drag = candidate
            .active_drag_mut(session)
            .expect("test drag is active");
        drag.owner = nonlocal_owner;
        drag.continuation
            .as_mut()
            .expect("test source has a continuation")
            .draft
            .owner = nonlocal_owner;
        let before = candidate.clone();
        assert_eq!(
            candidate.upgrade_drag_to_desktop(session, nonlocal_owner),
            Err(InteractionRejection::SessionMismatch)
        );
        assert_eq!(candidate, before);
    }
}

#[test]
fn desktop_handoff_rejects_a_mismatched_source_continuation_without_mutation() {
    let (state, session, _) = active_native_local_drag_fixture(None);
    let owner = state.active_owner().expect("test drag has an owner");
    for mismatch in 0..3 {
        let mut candidate = state.clone();
        let continuation = candidate
            .active_drag_mut(session)
            .expect("test drag is active")
            .continuation
            .as_mut()
            .expect("test source has a continuation");
        match mismatch {
            0 => {
                continuation.session = SceneGestureSession::Drag(DragSessionId::new(
                    session.epoch(),
                    DragGeneration::new(2),
                ));
            }
            1 => {
                continuation.session =
                    SceneGestureSession::ContainedTransform(ContainedTransformSessionId::new(
                        session.epoch(),
                        ContainedTransformGeneration::new(1),
                    ));
            }
            _ => {
                continuation.draft.owner = GestureOwner::new(owner.stream(), GestureLane::Desktop);
            }
        }
        let before = candidate.clone();
        assert_eq!(
            candidate.upgrade_drag_to_desktop(session, owner),
            Err(InteractionRejection::SessionMismatch)
        );
        assert_eq!(candidate, before);
    }
}

#[test]
fn desktop_handoff_requires_an_already_active_drag() {
    let (start, _) = native_local_drag_start_fixture(None);
    let owner = start.owner;
    let mut armed = InteractionState::default();
    let (session, _) = armed.arm_drag(start).expect("test drag must arm");
    let (click, _) = click_start_fixture();
    let mut pressed = InteractionState::default();
    pressed.begin_click(click).expect("test click must begin");
    for mut candidate in [InteractionState::default(), armed, pressed] {
        let before = candidate.clone();
        assert_eq!(
            candidate.upgrade_drag_to_desktop(session, owner),
            Err(InteractionRejection::SessionMismatch)
        );
        assert_eq!(candidate, before);
    }
}

#[test]
fn desktop_handoff_preserves_contained_drag_intent_inside_and_outside_source() {
    for intent in [
        ContainedDragIntent::DockContent,
        ContainedDragIntent::MoveWindow,
    ] {
        for point in [
            LogicalPoint::new(42.0, 18.0).expect("test source point is finite"),
            LogicalPoint::new(500.0, 400.0).expect("test outside point is finite"),
        ] {
            let (mut state, session, _) = active_native_local_drag_fixture(Some(intent));
            let owner = state.active_owner().expect("test drag has an owner");
            let GestureLane::NativeLocal {
                binding,
                coordinate_generation,
            } = owner.lane()
            else {
                panic!("test owner must be native-local");
            };
            let route = crate::input::journal::NativeSurfacePointerRoute::new(
                ViewportBinding::from_window(binding, state.active_surface().unwrap()),
                coordinate_generation,
                PhysicalPoint::new(point.x() * 2.5, point.y() * 2.5)
                    .expect("test client point is finite"),
                point,
            );
            let position = ManipulationPositionKey::JournalNativeLocal {
                manipulation: Some(route),
                hover: Some(route),
            };
            let allows_magnetic = intent == ContainedDragIntent::DockContent;
            assert_eq!(
                state
                    .active_drag(session)
                    .expect("test drag is active")
                    .magnetic_targeting_allowed(),
                allows_magnetic,
                "source intent determines magnetism before any source-rectangle exit",
            );
            state
                .set_drag_position(session, position)
                .expect("test drag must observe the local point");
            let before_source = state
                .active_scene_gesture_continuation()
                .expect("test source has a continuation")
                .draft
                .source
                .clone();
            assert_eq!(state.upgrade_drag_to_desktop(session, owner), Ok(()));
            let drag = state
                .active_drag(session)
                .expect("same contained drag remains active");
            assert_eq!(drag.target.last_advanced, None);
            assert_eq!(drag.target.last_logical_point, None);
            assert_eq!(
                drag.continuation
                    .as_ref()
                    .expect("source continuation survives")
                    .draft
                    .source,
                before_source
            );
            assert_eq!(drag.magnetic_targeting_allowed(), allows_magnetic);
        }
    }
}

#[test]
fn native_local_hover_never_falls_back_to_manipulation_or_outside_all() {
    let surface = SurfaceId::new(21);
    let binding = ViewportBinding::new(
        EngineAuthorityDomainId::new_for_test(90),
        WorkspaceEpoch::new(23),
        surface,
        WindowToken::new(1),
        WindowIncarnation::new(1),
    );
    let route = |x, y| {
        crate::input::journal::NativeSurfacePointerRoute::new(
            binding,
            CoordinateGeneration::new(1),
            PhysicalPoint::new(x, y).expect("test client position must be finite"),
            LogicalPoint::new(x, y).expect("test surface position must be finite"),
        )
    };
    let manipulation = route(100.0, 20.0);
    let hover = route(20.0, 10.0);
    let position = ManipulationPositionKey::JournalNativeLocal {
        manipulation: Some(manipulation),
        hover: Some(hover),
    };
    assert_eq!(position.surface(), Some(surface));
    assert_eq!(position.hover_surface(), Some(surface));
    assert_eq!(
        position.hover_probe(),
        Some(SurfacePointer::new(surface, hover.surface_position()))
    );
    assert_eq!(position.logical_point(), Some(hover.surface_position()));

    let unavailable_hover = ManipulationPositionKey::JournalNativeLocal {
        manipulation: Some(manipulation),
        hover: None,
    };
    assert_eq!(unavailable_hover.surface(), None);
    assert_eq!(unavailable_hover.hover_surface(), None);
    assert_eq!(unavailable_hover.hover_probe(), None);
    assert_eq!(unavailable_hover.logical_point(), None);
    assert!(unavailable_hover.hover_probe_requested());

    let swapped_lanes = ManipulationPositionKey::JournalNativeLocal {
        manipulation: Some(hover),
        hover: Some(manipulation),
    };
    assert_eq!(swapped_lanes.surface(), Some(surface));
    assert_eq!(swapped_lanes.hover_surface(), Some(surface));
    assert_eq!(
        swapped_lanes.hover_probe(),
        Some(SurfacePointer::new(
            surface,
            manipulation.surface_position()
        ))
    );
    assert_eq!(
        swapped_lanes.logical_point(),
        Some(manipulation.surface_position())
    );
}

#[test]
fn invalidating_position_preserves_latest_logical_point_for_revalidation() {
    let surface = SurfaceId::new(21);
    let presentation = frozen_presentation(surface, WorkspaceEpoch::new(23));
    let latest = LogicalPoint::new(42.0, 18.0).expect("test point must be finite");
    let initial = LogicalPoint::new(4.0, 3.0).expect("test point must be finite");
    let mut target = DragTargetState::new(&FrozenDragOrigin::Workspace);

    let route = HostLocalPointerRoute::new(presentation.presented, latest);
    target.set_last_advanced(ManipulationPositionKey::JournalHost {
        manipulation: Some(route),
        hover: Some(route),
    });
    target.invalidate_last_advanced();

    assert_eq!(target.last_advanced(), None);
    assert_eq!(target.current_pointer(Some(initial)), Some(latest));
}
