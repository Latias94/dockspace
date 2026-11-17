use super::*;
use crate::graph::command::RootPresentationTarget;
use crate::policy::{DockPolicyRequest, DockResizePolicyRequest, DockSurfaceRule, PolicyDecision};

const CONTAINED_ROOT: RootId = RootId::new(2);
const CONTAINED: FloatingPresentationId = FloatingPresentationId::new(1);

fn contained_workspace() -> (Workspace, NodeId, NodeId, NodeId) {
    let mut builder = Workspace::builder();
    let mut tabs = (1..=5).map(|item| builder.insert_node(Node::tabs([ItemId::new(item)])));
    let [left, right, floating_left, top, bottom] = std::array::from_fn(|_| tabs.next().unwrap());
    let main = builder.insert_node(Node::equal_split(Axis::Horizontal, [left, right]).unwrap());
    let vertical = builder.insert_node(Node::equal_split(Axis::Vertical, [top, bottom]).unwrap());
    let horizontal = builder
        .insert_node(Node::equal_split(Axis::Horizontal, [floating_left, vertical]).unwrap());
    builder.set_root(ROOT, RootRecord::new(main));
    builder.set_root(CONTAINED_ROOT, RootRecord::new(horizontal));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_contained_floating(
        CONTAINED,
        ContainedFloating::new(
            CONTAINED_ROOT,
            LogicalRect::new(480.0, 250.0, 300.0, 300.0).unwrap(),
        ),
    );
    builder.attach_contained(SURFACE, CONTAINED).unwrap();
    (builder.build().unwrap(), main, horizontal, vertical)
}

#[test]
fn root_axis_rules_intersect_other_scopes_and_restore_the_default_policy() {
    let mut policy = DockPolicy::default();
    let original = policy.clone();
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Horizontal, false);
    let frozen = policy.snapshot(PolicyRevision::new(1));
    let request = |root, axis, surface| {
        DockPolicyRequest::Resize(DockResizePolicyRequest::new(root, axis, surface))
    };
    for surface in [None, Some(SURFACE)] {
        assert_eq!(
            frozen.evaluate(&request(ROOT, Axis::Horizontal, surface)),
            PolicyDecision::Reject(PolicyRejection::RootResizeAxisDisabled {
                root: ROOT,
                axis: Axis::Horizontal,
            })
        );
        assert_eq!(
            frozen.evaluate(&request(ROOT, Axis::Vertical, surface)),
            PolicyDecision::Allow
        );
        assert_eq!(
            frozen.evaluate(&request(CONTAINED_ROOT, Axis::Horizontal, surface)),
            PolicyDecision::Allow
        );
    }
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Horizontal, true);
    assert_eq!(policy, original, "unlocking leaves no redundant root rule");
    assert!(
        !frozen
            .policy()
            .allows_root_resize_axis(ROOT, DockspaceAxis::Horizontal)
    );

    policy.set_allow_resize_axis(DockspaceAxis::Horizontal, false);
    policy.set_allow_root_resize_axis(ROOT, DockspaceAxis::Horizontal, true);
    assert_eq!(
        policy.snapshot(PolicyRevision::new(2)).evaluate(&request(
            ROOT,
            Axis::Horizontal,
            Some(SURFACE)
        )),
        PolicyDecision::Reject(PolicyRejection::ResizeAxisDisabled {
            axis: Axis::Horizontal
        })
    );
    let mut surface = DockSurfaceRule::default();
    surface.set_resize_axis(DockspaceAxis::Vertical, false);
    policy.set_surface_rule(SURFACE, surface);
    assert_eq!(
        policy.snapshot(PolicyRevision::new(3)).evaluate(&request(
            ROOT,
            Axis::Vertical,
            Some(SURFACE)
        )),
        PolicyDecision::Reject(PolicyRejection::SurfaceResizeAxisDisabled {
            surface: SURFACE,
            axis: Axis::Vertical
        })
    );
}

#[test]
fn root_axis_rule_isolates_contained_splitters_and_keeps_the_other_axis_usable() {
    let (workspace, main, horizontal, vertical) = contained_workspace();
    let mut policy = DockPolicy::default();
    policy.set_allow_root_resize_axis(CONTAINED_ROOT, DockspaceAxis::Horizontal, false);
    let mut engine = DockEngine::new(workspace, policy).unwrap();
    let mut host = support::TestPresentationHost::new(&mut engine);
    support::publish_surface(&mut engine, &mut host, SURFACE, rect(800.0, 600.0));

    let (_, main_record) = splitter(&engine, main);
    let (scene, blocked) = splitter(&engine, horizontal);
    let (_, vertical_record) = splitter(&engine, vertical);
    assert!(main_record.operable());
    assert!(!blocked.operable());
    assert!(vertical_record.operable());
    let ready = engine.scene().ready_surface(SURFACE).unwrap();
    assert!(
        ready
            .hit_manifest()
            .region_for_kind(PresentationHitRegionKind::SplitterHandle(*blocked.id()))
            .is_none()
    );
    let junction = ready
        .plan()
        .splitter_junction_records()
        .iter()
        .find(|record| record.id().root() == CONTAINED_ROOT)
        .unwrap();
    assert!(
        !junction.operable(),
        "a corner gesture cannot bypass a denied axis"
    );
    let before = engine.workspace().clone();
    assert!(matches!(
        adjust_splitter(&mut engine, &mut host, scene, *blocked.id(), 24.0),
        InteractionOutcome::Rejected(InteractionRejection::SplitterGestureHitUnavailable {
            surface: SURFACE
        })
    ));
    assert_eq!(engine.workspace(), &before);

    let (scene, record) = splitter(&engine, main);
    assert!(matches!(
        adjust_splitter(&mut engine, &mut host, scene, *record.id(), 24.0),
        InteractionOutcome::SplitterAdjusted { changed: true, .. }
    ));
    assert_ne!(
        node_weights(engine.workspace(), main),
        node_weights(&before, main)
    );
    support::publish_surface(&mut engine, &mut host, SURFACE, rect(800.0, 600.0));

    let mut pointer = create_pointer_stream(&mut engine, &host);
    let (_, press) = begin_handle_resize(&mut engine, &mut host, &mut pointer, vertical);
    let moved = LogicalPoint::new(press.x(), press.y() + 24.0).unwrap();
    let delivered = submit_pointer_edge(
        &mut engine,
        &mut host,
        &mut pointer,
        PointerEdgeKind::ButtonReleased(PointerButton::Primary),
        moved,
        None,
    );
    assert!(matches!(
        pointer_interaction_outcome(&delivered),
        InteractionOutcome::ResizeDelivered { changed: true, .. }
    ));
    assert_ne!(
        node_weights(engine.workspace(), vertical),
        node_weights(&before, vertical)
    );
    assert_eq!(
        node_weights(engine.workspace(), horizontal),
        node_weights(&before, horizontal)
    );
}

#[test]
fn root_resize_rule_rejects_atomic_batches_before_and_after_surface_rehome() {
    let (mut workspace, main, horizontal, vertical) = contained_workspace();
    let mut policy = DockPolicy::default();
    policy.set_allow_root_resize_axis(CONTAINED_ROOT, DockspaceAxis::Horizontal, false);
    let policy = policy.snapshot(PolicyRevision::new(1));
    let assert_denied = |workspace: &mut Workspace| {
        let main = workspace.capture_node_source(ROOT, main).unwrap();
        let blocked = workspace
            .capture_node_source(CONTAINED_ROOT, horizontal)
            .unwrap();
        let before = workspace.clone();
        let error = WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
            tracks: vec![
                TrackResize::split(main, weights(0.25, 0.75)),
                TrackResize::split(blocked, weights(0.25, 0.75)),
            ],
        }])
        .apply(workspace, &policy)
        .unwrap_err();
        assert!(matches!(
            error,
            TransactionError::Command {
                source: CommandError::Policy(PolicyRejection::RootResizeAxisDisabled {
                    root: CONTAINED_ROOT,
                    axis: Axis::Horizontal
                }),
                ..
            }
        ));
        assert_eq!(
            *workspace, before,
            "a denied root rejects the complete mixed-root resize"
        );
    };
    assert_denied(&mut workspace);
    let source = workspace
        .capture_node_source(CONTAINED_ROOT, horizontal)
        .unwrap();
    let mut native_policy = policy.to_policy();
    native_policy.set_allow_native_surfaces(true);
    WorkspaceTransaction::from_commands([WorkspaceCommand::RehomeRoot {
        source,
        target: RootPresentationTarget::NewSurface {
            surface: SurfaceId::new(2),
        },
    }])
    .apply(
        &mut workspace,
        &native_policy.snapshot(PolicyRevision::new(2)),
    )
    .unwrap();
    assert_eq!(
        workspace.surface(SurfaceId::new(2)).unwrap().main_root,
        Some(CONTAINED_ROOT)
    );
    assert_denied(&mut workspace);
    let vertical_source = workspace
        .capture_node_source(CONTAINED_ROOT, vertical)
        .unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::ResizeTracks {
        tracks: vec![TrackResize::split(vertical_source, weights(0.25, 0.75))],
    }])
    .apply(&mut workspace, &policy)
    .unwrap();
    assert_eq!(node_weights(&workspace, vertical), weights(0.25, 0.75));
}

#[test]
fn discarded_root_policy_change_preserves_resize_until_a_committed_change_cancels_it() {
    let (workspace, split) = simple_workspace();
    let mut engine = DockEngine::new(workspace, DockPolicy::default()).unwrap();
    let mut host = support::TestPresentationHost::new(&mut engine);
    publish(
        &mut engine,
        &mut host,
        support::MeasurementProfile::default(),
    );
    let (old_scene, old_splitter) = splitter(&engine, split);
    let mut pointer = create_pointer_stream(&mut engine, &host);
    let (session, press) = begin_handle_resize(&mut engine, &mut host, &mut pointer, split);
    let before = engine.workspace().clone();
    let version = engine.version();
    let mut replacement = engine.policy().clone();
    replacement.set_allow_root_resize_axis(ROOT, DockspaceAxis::Horizontal, false);
    {
        let mut frame = host.begin(&engine);
        frame
            .submit_surface_pointer_journal(&pointer.provider, empty_pointer_journal(&pointer))
            .unwrap();
        frame
            .submit_pointer_receiver_receipts(
                PointerReceiverReceiptBatch::new(Vec::<PointerReceiverReceipt>::new()).unwrap(),
            )
            .unwrap();
        support::TestInputStream::resume(&engine, INPUT_SOURCE)
            .append(
                &mut frame,
                EngineInput::ReplacePolicy {
                    expected: version,
                    policy: replacement.clone(),
                },
            )
            .unwrap();
        assert!(
            !frame
                .view()
                .policy()
                .allows_root_resize_axis(ROOT, DockspaceAxis::Horizontal)
        );
        assert_eq!(frame.view().interaction().status(), InteractionStatus::Idle);
    }
    assert_eq!(engine.workspace(), &before);
    assert_eq!(engine.version(), version);
    assert!(
        engine
            .policy()
            .allows_root_resize_axis(ROOT, DockspaceAxis::Horizontal)
    );
    assert_eq!(
        engine.interaction().status(),
        InteractionStatus::Resizing { session }
    );

    let changed = submit_input_with_idle_pointer(
        &mut engine,
        &mut host,
        &mut pointer,
        EngineInput::ReplacePolicy {
            expected: version,
            policy: replacement,
        },
    );
    assert!(changed.interaction_events().iter().any(|event| matches!(
        event.kind(),
        crate::interaction::InteractionEventKind::Cancelled {
            status: InteractionStatus::Resizing { session: actual },
            reason: InteractionCancelReason::PolicyChanged,
        } if *actual == session
    )));
    assert_eq!(engine.interaction().status(), InteractionStatus::Idle);
    assert_eq!(engine.workspace(), &before);

    let expected = engine.version();
    let stale = submit_input_with_idle_pointer(
        &mut engine,
        &mut host,
        &mut pointer,
        EngineInput::AdjustSplitterResize {
            expected: version,
            scene: old_scene,
            splitter: *old_splitter.id(),
            delta: 24.0,
        },
    );
    assert!(matches!(
        stale.reduced_inputs()[0].outcome(),
        InputOutcome::StaleRejected { .. }
    ));
    submit_pointer_edge(
        &mut engine,
        &mut host,
        &mut pointer,
        PointerEdgeKind::ButtonReleased(PointerButton::Primary),
        press,
        None,
    );
    assert_eq!(engine.workspace(), &before);
    assert_eq!(engine.version(), expected);
    let mut receipt = pointer.provider.drain().unwrap();
    let retired = engine
        .retire_quiesced_surface_local_pointer_provider(&mut receipt)
        .unwrap();
    assert!(!retired.interaction_changed());
    publish(
        &mut engine,
        &mut host,
        support::MeasurementProfile::default(),
    );
    assert!(!splitter(&engine, split).1.operable());
}
