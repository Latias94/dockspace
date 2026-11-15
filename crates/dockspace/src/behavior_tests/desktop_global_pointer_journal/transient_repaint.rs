//! Gesture visuals invalidate only their old and new physical presentation owners.

use super::*;
use crate::engine::transition::EngineTransition;
use crate::interaction::GestureCancelDelivery;

const UNRELATED_SURFACE: SurfaceId = SurfaceId::new(3);

fn fixture() -> ActiveNativeDragFixture {
    let mut builder = Workspace::builder();
    let source = builder.insert_node(Node::tabs([ItemId::new(1), ItemId::new(3)]));
    let target_tabs = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let unrelated = builder.insert_node(Node::tabs([ItemId::new(4)]));
    for (surface, root, node) in [
        (SOURCE_SURFACE, SOURCE_ROOT, source),
        (TARGET_SURFACE, TARGET_ROOT, target_tabs),
        (UNRELATED_SURFACE, RootId::new(3), unrelated),
    ] {
        builder.set_root(root, RootRecord::new(node));
        builder.set_surface(surface, SurfacePresentation::with_main(root));
    }
    let DesktopNativeFixture {
        mut engine,
        mut host,
        provider,
        source_binding,
        target_binding,
        target_tabs,
    } = desktop_native_fixture_from_workspace(builder.build().unwrap(), target_tabs, 1.0);
    let target_point = arm_native_drag(
        &mut engine,
        &mut host,
        provider,
        source_binding,
        target_binding,
        target_tabs,
        0,
    );
    assert_eq!(
        engine.presentation_preview().unwrap().visual().surface(),
        TARGET_SURFACE
    );
    assert_eq!(
        engine.interaction().drop_affordance().unwrap().surface(),
        TARGET_SURFACE
    );
    ActiveNativeDragFixture {
        engine,
        host,
        provider,
        source_binding,
        target_binding,
        target_tabs,
        target_point,
    }
}

fn repaint_surfaces(transition: &EngineTransition) -> Vec<SurfaceId> {
    transition
        .surface_scene_deltas()
        .iter()
        .copied()
        .filter(|delta| delta.requires_repaint())
        .map(|delta| delta.surface())
        .collect()
}

fn settle_pointer_frame(
    fixture: &mut ActiveNativeDragFixture,
    through: u64,
    cancel_from: Option<ViewportBinding>,
) -> EngineTransition {
    let mut frame = fixture.host.begin(&fixture.engine);
    frame
        .submit_pointer_journal(
            fixture.provider,
            PointerEdgeJournal::new(
                PointerEdgeSequence::new(through),
                PointerEdgeSequence::new(through),
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();
    frame
        .submit_pointer_receiver_receipts(
            PointerReceiverReceiptBatch::new(Vec::<PointerReceiverReceipt>::new()).unwrap(),
        )
        .unwrap();
    if let Some(delivery) = cancel_from {
        support::TestInputStream::resume(&fixture.engine, INPUT_SOURCE)
            .append(
                &mut frame,
                EngineInput::CancelActiveInteraction {
                    expected: fixture.engine.version(),
                    delivery: GestureCancelDelivery::NativeBinding(delivery),
                },
            )
            .unwrap();
    }
    complete(&fixture.engine, &mut frame);
    fixture.host.finish(frame, &mut fixture.engine)
}

#[test]
fn cancelling_native_drag_repaints_source_and_old_preview_without_waking_unrelated_surface() {
    for from_target in [false, true] {
        let mut fixture = fixture();
        let before = fixture.engine.version();
        let source_scene = fixture
            .engine
            .scene()
            .surface(SOURCE_SURFACE)
            .unwrap()
            .stamp();
        let target_scene = fixture
            .engine
            .scene()
            .surface(TARGET_SURFACE)
            .unwrap()
            .stamp();
        let delivery = if from_target {
            fixture.target_binding
        } else {
            fixture.source_binding
        };
        let transition =
            settle_pointer_frame(&mut fixture, ACTIVE_DRAG_POINTER_THROUGH, Some(delivery));
        assert_eq!(fixture.engine.version(), before);
        assert_eq!(
            fixture.engine.interaction().status(),
            InteractionStatus::Idle
        );
        assert!(fixture.engine.presentation_preview().is_none());
        assert!(fixture.engine.interaction().drop_affordance().is_none());
        assert_eq!(
            fixture
                .engine
                .scene()
                .surface(SOURCE_SURFACE)
                .unwrap()
                .stamp(),
            source_scene,
            "source decoration disappears without changing the layout scene"
        );
        assert_eq!(
            fixture
                .engine
                .scene()
                .surface(TARGET_SURFACE)
                .unwrap()
                .stamp(),
            target_scene,
            "target preview and guides disappear without changing the layout scene"
        );
        assert_eq!(
            repaint_surfaces(&transition),
            [SOURCE_SURFACE, TARGET_SURFACE]
        );
        assert!(
            !transition
                .affected_surfaces()
                .any(|surface| surface == UNRELATED_SURFACE)
        );

        let idle = settle_pointer_frame(&mut fixture, ACTIVE_DRAG_POINTER_THROUGH, None);
        assert!(
            repaint_surfaces(&idle).is_empty(),
            "settled cancellation must not keep either native window repainting"
        );
    }
}

#[test]
fn capture_loss_repaints_the_source_decoration_and_the_remote_preview() {
    let mut fixture = fixture();
    let before = fixture.engine.version();
    let transition = submit_active_drag_edge(
        &mut fixture.engine,
        &mut fixture.host,
        fixture.provider,
        fixture.source_binding,
        fixture.target_binding,
        fixture.target_tabs,
        fixture.target_point,
        PointerEdgeKind::CaptureChanged,
        Authority::Known(PointerCaptureOwner::None),
    );
    assert_eq!(fixture.engine.version(), before);
    assert_eq!(
        fixture.engine.interaction().status(),
        InteractionStatus::Idle
    );
    assert_eq!(
        repaint_surfaces(&transition),
        [SOURCE_SURFACE, TARGET_SURFACE]
    );
}

#[test]
fn cancelling_over_a_blocked_destination_repaints_its_ghost_without_preview_or_guides() {
    let mut fixture = fixture();
    let point =
        LogicalPoint::new(fixture.target_point.x(), fixture.target_point.y() + 1.0).unwrap();
    let projection = interaction(&fixture.engine, TARGET_SURFACE);
    let route = desktop_route(
        fixture.target_binding,
        projection,
        TARGET_ORIGIN_X,
        TARGET_SCALE,
        point,
    );
    let mut frame = fixture.host.begin(&fixture.engine);
    frame
        .submit_pointer_journal(
            fixture.provider,
            pointer_edge_journal_with_delivery(
                ACTIVE_DRAG_POINTER_THROUGH,
                PointerEdgeKind::Moved,
                PointerEdgeLocation::Desktop { route },
                Authority::Known(PointerEventDeliveryOwner::Native(
                    fixture.source_binding.window(),
                )),
                Authority::Known(PointerCaptureOwner::Native(fixture.source_binding.window())),
            ),
        )
        .unwrap();
    let candidate = frame.pointer_receiver_candidates().unwrap().candidates()[0].clone();
    assert_eq!(candidate.probes(), PointerReceiverProbeRequest::HoverHit);
    let projection = frame.view().interaction_projection(TARGET_SURFACE).unwrap();
    let blocked = PointerReceiverHoverHit::new(
        projection,
        point,
        PointerReceiverHoverHitDisposition::Blocked,
    )
    .unwrap();
    frame
        .submit_pointer_receiver_receipts(
            PointerReceiverReceiptBatch::new([candidate.receipt(
                PointerReceiverObservation::Presented(
                    PresentedPointerReceiverObservation::new([
                        PointerReceiverProbeReceipt::HoverHit(blocked),
                    ])
                    .unwrap(),
                ),
            )])
            .unwrap(),
        )
        .unwrap();
    complete(&fixture.engine, &mut frame);
    fixture.host.finish(frame, &mut fixture.engine);
    assert!(matches!(
        fixture.engine.interaction().status(),
        InteractionStatus::Dragging { .. }
    ));
    assert!(fixture.engine.presentation_preview().is_none());
    assert!(fixture.engine.interaction().drop_affordance().is_none());

    let source = fixture.source_binding;
    let cancelled =
        settle_pointer_frame(&mut fixture, ACTIVE_DRAG_POINTER_THROUGH + 1, Some(source));
    assert_eq!(
        repaint_surfaces(&cancelled),
        [SOURCE_SURFACE, TARGET_SURFACE],
        "a blocked target can still paint a pointer ghost without a drop preview"
    );
}

#[test]
fn an_unchanged_live_drag_does_not_repaint_from_presentation_provenance_alone() {
    let mut fixture = fixture();
    let preview = fixture.engine.presentation_preview().cloned();
    let transition = settle_pointer_frame(&mut fixture, ACTIVE_DRAG_POINTER_THROUGH, None);
    assert!(matches!(
        fixture.engine.interaction().status(),
        InteractionStatus::Dragging { .. }
    ));
    assert_eq!(fixture.engine.presentation_preview(), preview.as_ref());
    assert!(
        repaint_surfaces(&transition).is_empty(),
        "re-presenting unchanged drag visuals must not create a repaint loop"
    );
}
