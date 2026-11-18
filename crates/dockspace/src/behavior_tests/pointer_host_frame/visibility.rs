use super::*;
use dockspace::engine::transition::EngineTransition;
use dockspace::model::DockspaceActionOutcome;

const VISIBILITY_SOURCE: StableInputSourceId = StableInputSourceId::new(0xB17);

fn set_visibility_after(
    engine: &mut DockEngine,
    host: &mut TestPresentationHost,
    provider: &SurfaceLocalPointerProvider,
    watermark: u64,
    item: ItemId,
    visible: bool,
) -> EngineTransition {
    let expected = engine.version();
    let mut frame = host.begin(engine);
    frame
        .submit_surface_pointer_journal(provider, empty_journal(watermark))
        .expect("visibility changes preserve the gesture's committed pointer prefix");
    frame
        .submit_pointer_receiver_receipts(
            PointerReceiverReceiptBatch::new(Vec::<PointerReceiverReceipt>::new()).unwrap(),
        )
        .expect("an empty pointer prefix requires no receiver facts");
    support::TestInputStream::resume(engine, VISIBILITY_SOURCE)
        .append(
            &mut frame,
            EngineInput::ProductAction {
                expected,
                action: crate::model::ProductAction::SetItemVisibility { item, visible },
                required_surfaces: Vec::new(),
            },
        )
        .expect("visibility reduces after the exact pointer prefix");
    complete(engine, &mut frame);
    let transition = host.finish(frame, engine);
    assert!(matches!(
        transition.reduced_inputs()[0].outcome(),
        InputOutcome::ProductActionProcessed {
            outcome: DockspaceActionOutcome::ItemVisibilityChanged {
                item: actual_item, visible: actual_visible, changed: true,
            }, ..
        } if *actual_item == item && *actual_visible == visible
    ));
    transition
}

fn assert_workspace_cancelled_once(transition: &EngineTransition, previous: InteractionStatus) {
    let cancelled = transition
        .interaction_events()
        .iter()
        .filter_map(|event| match event.kind() {
            InteractionEventKind::Cancelled { status, reason } => Some((*status, *reason)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        cancelled,
        [(previous, InteractionCancelReason::WorkspaceChanged)]
    );
}

fn submit_unprobed_edges_after(
    engine: &mut DockEngine,
    host: &mut TestPresentationHost,
    provider: &SurfaceLocalPointerProvider,
    mut previous: u64,
    point: LogicalPoint,
    kinds: impl IntoIterator<Item = PointerEdgeKind>,
) -> EngineTransition {
    let mut frame = host.begin(engine);
    for kind in kinds {
        frame
            .submit_surface_pointer_journal(
                provider,
                pointer_edge_journal(
                    previous,
                    kind,
                    PointerEdgeLocation::SurfaceLocal {
                        position: Authority::Known(point),
                    },
                    Authority::Known(PointerCaptureOwner::None),
                ),
            )
            .expect("late pointer edges continue the original provider stream");
        let candidate = frame.pointer_receiver_candidates().unwrap().candidates()[0].clone();
        assert_eq!(
            candidate.probes(),
            PointerReceiverProbeRequest::NotApplicable
        );
        frame
            .submit_pointer_receiver_receipts(
                PointerReceiverReceiptBatch::new([
                    candidate.receipt(PointerReceiverObservation::NotApplicable)
                ])
                .unwrap(),
            )
            .expect("unowned edges must not ask the host to replay stale receiver facts");
        previous += 1;
    }
    complete(engine, &mut frame);
    host.finish(frame, engine)
}

#[test]
fn hide_pressed_close_discards_its_old_receiver_and_allows_a_fresh_close_press_after_show() {
    let mut engine = DockEngine::new(workspace(), DockPolicy::default()).unwrap();
    let mut host = TestPresentationHost::new(&mut engine);
    publish_surface(&mut engine, &mut host, SURFACE, bounds());
    let provider = create_local_provider(&mut engine, &host);
    let item = ItemId::new(1);
    let (old_close, point) = point_inside_tab_close(interaction(&engine), item);
    submit_dock_press_after(&mut engine, &mut host, &provider, old_close, point, 0);
    let pressed = engine.interaction().status();
    assert!(matches!(pressed, InteractionStatus::Pressed { .. }));

    let hidden = set_visibility_after(&mut engine, &mut host, &provider, 1, item, false);
    assert_workspace_cancelled_once(&hidden, pressed);
    assert_eq!(engine.interaction().status(), InteractionStatus::Idle);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 1);
    assert!(
        PointerReceiverDelivery::new(
            interaction(&engine),
            PointerReceiverDeliveryDisposition::Dock(old_close),
        )
        .is_err(),
        "a hidden close receiver cannot be rebound to the newly painted output"
    );
    let before_release = engine.workspace().clone();
    let released = submit_unprobed_edges_after(
        &mut engine,
        &mut host,
        &provider,
        1,
        point,
        [PointerEdgeKind::ButtonReleased(PointerButton::Primary)],
    );
    assert!(
        released.reduced_pointer_edges()[0]
            .interaction_outcomes()
            .is_empty()
    );
    assert_eq!(engine.workspace(), &before_release);
    assert_eq!(engine.active_close_plans().count(), 0);

    set_visibility_after(&mut engine, &mut host, &provider, 2, item, true);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 2);
    let (close, point) = point_inside_tab_close(interaction(&engine), item);
    submit_dock_press_after(&mut engine, &mut host, &provider, close, point, 2);
    assert!(matches!(
        engine.interaction().status(),
        InteractionStatus::Pressed { .. }
    ));
    assert_ne!(engine.interaction().status(), pressed);
}

#[test]
fn hide_armed_tab_ignores_late_movement_and_release_then_accepts_a_new_press_after_show() {
    let mut engine = DockEngine::new(workspace(), DockPolicy::default()).unwrap();
    let mut host = TestPresentationHost::new(&mut engine);
    publish_surface(&mut engine, &mut host, SURFACE, bounds());
    let provider = create_local_provider(&mut engine, &host);
    let item = ItemId::new(1);
    let (old_tab, point) = point_inside_tab(interaction(&engine), item);
    submit_dock_press_after(&mut engine, &mut host, &provider, old_tab, point, 0);
    let armed = engine.interaction().status();
    assert!(matches!(armed, InteractionStatus::Armed { .. }));

    let hidden = set_visibility_after(&mut engine, &mut host, &provider, 1, item, false);
    assert_workspace_cancelled_once(&hidden, armed);
    assert_eq!(engine.interaction().status(), InteractionStatus::Idle);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 1);
    assert!(
        PointerReceiverDelivery::new(
            interaction(&engine),
            PointerReceiverDeliveryDisposition::Dock(old_tab),
        )
        .is_err()
    );
    let before_tail = engine.workspace().clone();
    let tail = submit_unprobed_edges_after(
        &mut engine,
        &mut host,
        &provider,
        1,
        point,
        [
            PointerEdgeKind::Moved,
            PointerEdgeKind::ButtonReleased(PointerButton::Primary),
        ],
    );
    assert!(
        tail.reduced_pointer_edges()
            .iter()
            .all(|edge| edge.interaction_outcomes().is_empty())
    );
    assert_eq!(engine.workspace(), &before_tail);

    set_visibility_after(&mut engine, &mut host, &provider, 3, item, true);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 3);
    let (tab, point) = point_inside_tab(interaction(&engine), item);
    let next = submit_dock_press_after(&mut engine, &mut host, &provider, tab, point, 3);
    assert!(matches!(
        next.reduced_pointer_edges()[0].interaction_outcomes(),
        [InteractionOutcome::DragArmed { .. }]
    ));
    assert!(matches!(
        engine.interaction().status(),
        InteractionStatus::Armed { .. }
    ));
    assert_ne!(engine.interaction().status(), armed);
}

#[test]
fn hide_dragging_payload_cancels_the_resolved_preview_and_late_release_cannot_dock_it() {
    let (workspace, target_tabs) = split_workspace();
    let mut engine = DockEngine::new(workspace, DockPolicy::default()).unwrap();
    let mut host = TestPresentationHost::new(&mut engine);
    publish_surface(&mut engine, &mut host, SURFACE, bounds());
    let provider = create_local_provider(&mut engine, &host);
    let item = ItemId::new(1);
    let projection = interaction(&engine);
    let (old_tab, press) = point_inside_tab(projection, item);
    let (drop_region, target) = point_inside_center_target(projection, target_tabs);
    begin_journal_drag(
        &mut engine,
        &mut host,
        &provider,
        old_tab,
        press,
        target,
        PointerReceiverHoverHitDisposition::Dock(drop_region),
    );
    let dragging = engine.interaction().status();
    assert!(matches!(dragging, InteractionStatus::Dragging { .. }));
    assert!(engine.interaction().preview().is_some());

    let hidden = set_visibility_after(&mut engine, &mut host, &provider, 2, item, false);
    assert_workspace_cancelled_once(&hidden, dragging);
    assert_eq!(engine.interaction().status(), InteractionStatus::Idle);
    assert!(engine.interaction().active_drag_view().is_none());
    assert_eq!(engine.pending_release_preview(), None);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 2);
    let before_tail = engine.workspace().clone();
    let tail = submit_unprobed_edges_after(
        &mut engine,
        &mut host,
        &provider,
        2,
        target,
        [
            PointerEdgeKind::Moved,
            PointerEdgeKind::ButtonReleased(PointerButton::Primary),
        ],
    );
    assert!(
        tail.reduced_pointer_edges()
            .iter()
            .all(|edge| edge.interaction_outcomes().is_empty())
    );
    assert_eq!(engine.workspace(), &before_tail);

    set_visibility_after(&mut engine, &mut host, &provider, 4, item, true);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 4);
    let (tab, point) = point_inside_tab(interaction(&engine), item);
    let next = submit_dock_press_after(&mut engine, &mut host, &provider, tab, point, 4);
    assert!(matches!(
        next.reduced_pointer_edges()[0].interaction_outcomes(),
        [InteractionOutcome::DragArmed { .. }]
    ));
    assert_ne!(engine.interaction().status(), dragging);
}

#[test]
fn hide_resizing_child_discards_transient_weights_and_restored_split_accepts_a_new_resize() {
    let (workspace, split) = splitter_workspace();
    let mut engine = DockEngine::new(workspace, DockPolicy::default()).unwrap();
    let mut host = TestPresentationHost::new(&mut engine);
    publish_surface(&mut engine, &mut host, SURFACE, bounds());
    let provider = create_local_provider(&mut engine, &host);
    let (old_splitter, press) = point_inside_splitter(interaction(&engine), split);
    let Node::Split { weights, .. } = engine.workspace().node(split).unwrap() else {
        unreachable!()
    };
    let weights = weights.clone();
    submit_dock_press_after(&mut engine, &mut host, &provider, old_splitter, press, 0);
    let resizing = engine.interaction().status();
    assert!(matches!(resizing, InteractionStatus::Resizing { .. }));
    let moved = LogicalPoint::new(press.x() + 24.0, press.y()).unwrap();
    let moving = submit_unprobed_edges_after(
        &mut engine,
        &mut host,
        &provider,
        1,
        moved,
        [PointerEdgeKind::Moved],
    );
    assert!(matches!(
        moving.reduced_pointer_edges()[0].interaction_outcomes(),
        [InteractionOutcome::ResizeUpdated { .. }]
    ));

    let hidden = set_visibility_after(&mut engine, &mut host, &provider, 2, ItemId::new(2), false);
    assert_workspace_cancelled_once(&hidden, resizing);
    assert_eq!(engine.interaction().status(), InteractionStatus::Idle);
    assert!(
        matches!(engine.workspace().node(split), Some(Node::Split { weights: current, .. }) if current == &weights)
    );
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 2);
    assert!(
        PointerReceiverDelivery::new(
            interaction(&engine),
            PointerReceiverDeliveryDisposition::Dock(old_splitter),
        )
        .is_err(),
        "a collapsed split exposes no receiver for the old resize handle"
    );
    let before_tail = engine.workspace().clone();
    let tail = submit_unprobed_edges_after(
        &mut engine,
        &mut host,
        &provider,
        2,
        moved,
        [
            PointerEdgeKind::Moved,
            PointerEdgeKind::ButtonReleased(PointerButton::Primary),
        ],
    );
    assert!(
        tail.reduced_pointer_edges()
            .iter()
            .all(|edge| edge.interaction_outcomes().is_empty())
    );
    assert_eq!(engine.workspace(), &before_tail);

    set_visibility_after(&mut engine, &mut host, &provider, 4, ItemId::new(2), true);
    republish_surface_with_active_provider(&mut engine, &mut host, &provider, 4);
    let (splitter, point) = point_inside_splitter(interaction(&engine), split);
    let next = submit_dock_press_after(&mut engine, &mut host, &provider, splitter, point, 4);
    assert!(matches!(
        next.reduced_pointer_edges()[0].interaction_outcomes(),
        [InteractionOutcome::ResizeBegan { .. }]
    ));
    assert!(matches!(
        engine.interaction().status(),
        InteractionStatus::Resizing { .. }
    ));
    assert_ne!(engine.interaction().status(), resizing);
}

#[test]
fn contained_group_edge_drag_previews_only_visible_members_but_moves_every_owned_item() {
    for split_payload in [false, true] {
        let visible = ItemId::new(1);
        let hidden = ItemId::new(2);
        let retained = ItemId::new(3);
        let mut builder = Workspace::builder();
        let main_tabs = builder.insert_node(Node::tabs([retained]));
        let visible_tabs = builder.insert_node(Node::tabs(if split_payload {
            vec![visible]
        } else {
            vec![visible, hidden]
        }));
        let hidden_tabs = if split_payload {
            builder.insert_node(Node::tabs([hidden]))
        } else {
            visible_tabs
        };
        let source_node = if split_payload {
            // The horizontal edge move canonicalizes this source split into the
            // destination. The hidden trailing leaf must not extend its visual
            // payload bounds back to the destination root's origin.
            builder.insert_node(
                Node::equal_split(Axis::Horizontal, [visible_tabs, hidden_tabs]).unwrap(),
            )
        } else {
            visible_tabs
        };
        builder.set_root(ROOT, RootRecord::new(main_tabs).with_central(main_tabs));
        builder.set_root(CONTAINED_ROOT, RootRecord::new(source_node));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        builder.set_contained_floating(
            CONTAINED_FLOATING,
            ContainedFloating::new(
                CONTAINED_ROOT,
                LogicalRect::new(400.0, 300.0, 180.0, 130.0).unwrap(),
            ),
        );
        builder
            .attach_contained(SURFACE, CONTAINED_FLOATING)
            .unwrap();
        builder.set_hidden_items([hidden].into());
        let mut engine = DockEngine::new(builder.build().unwrap(), DockPolicy::default()).unwrap();
        let mut host = TestPresentationHost::new(&mut engine);
        publish_surface(&mut engine, &mut host, SURFACE, bounds());
        let provider = create_local_provider(&mut engine, &host);
        let projection = interaction(&engine);
        let (source, press) = if split_payload {
            point_inside_contained_title(projection, CONTAINED_FLOATING)
        } else {
            point_inside_tab_group(projection, visible)
        };
        let target_id = DropTargetId::OuterEdge {
            surface: SURFACE,
            root: ROOT,
            node: main_tabs,
            edge: dockspace::graph::command::Edge::Right,
        };
        let target_region = projection
            .hit_manifest()
            .regions()
            .iter()
            .find(|region| region.id().kind() == PresentationHitRegionKind::DropTarget(target_id))
            .expect("the main root publishes its right-edge drop receiver");
        let hit = target_region.hit().rect();
        let target =
            LogicalPoint::new(hit.x() + hit.width() * 0.5, hit.y() + hit.height() * 0.5).unwrap();
        let target_region = target_region.id();
        let before_items = engine.workspace().item_multiset();
        let dragged = begin_journal_drag(
            &mut engine,
            &mut host,
            &provider,
            source,
            press,
            target,
            PointerReceiverHoverHitDisposition::Dock(target_region),
        );
        assert!(
            matches!(
                dragged.reduced_pointer_edges()[0].interaction_outcomes(),
                [InteractionOutcome::DragArmed { .. }]
            ),
            "hidden-member group press must arm (split_payload={split_payload}): outcomes={:?}, final_status={:?}",
            dragged.reduced_pointer_edges()[0].interaction_outcomes(),
            engine.interaction().status(),
        );
        assert!(
            dragged.reduced_pointer_edges()[1]
                .interaction_outcomes()
                .iter()
                .any(|outcome| matches!(
                    outcome,
                    InteractionOutcome::PreviewUpdated {
                        preview: Some(_),
                        status: PreviewResolutionStatus::Resolved,
                        ..
                    }
                )),
            "hidden-member group move must resolve (split_payload={split_payload}): outcomes={:?}, status={:?}",
            dragged.reduced_pointer_edges()[1].interaction_outcomes(),
            engine.interaction().status(),
        );
        let drag = engine.interaction().active_drag_view().unwrap();
        assert!(matches!(
            (split_payload, drag.payload()),
            (false, dockspace::graph::command::MovePayload::Tabs(_))
                | (true, dockspace::graph::command::MovePayload::Subtree(_))
        ));
        let preview = match engine.interaction().preview().unwrap().visual() {
            PreviewVisual::Dock { target, rect, .. } if *target == target_id => *rect,
            visual => {
                panic!("the complete group must preview the main root's right edge: {visual:?}")
            }
        };
        assert!(preview.x() > bounds().x());
        assert!(preview.width() < bounds().width());
        assert!(engine.workspace().root(CONTAINED_ROOT).is_some());
        paint_active_journal_drag(&mut engine, &mut host, &provider);
        let released = release_journal_drag_after(
            &mut engine,
            &mut host,
            &provider,
            2,
            target,
            PointerReceiverHoverHitDisposition::Dock(target_region),
        );
        assert!(
            released.reduced_pointer_edges()[0]
                .interaction_outcomes()
                .iter()
                .any(|outcome| matches!(
                    outcome,
                    InteractionOutcome::DragDelivered {
                        delivery: dockspace::interaction::InteractionDelivery::Workspace {
                            changed: true,
                            ..
                        },
                        ..
                    }
                ))
        );
        assert_eq!(engine.interaction().status(), InteractionStatus::Idle);
        assert_eq!(engine.workspace().item_multiset(), before_items);
        assert!(
            engine
                .workspace()
                .capture_item_source(ROOT, visible_tabs, visible)
                .is_ok()
        );
        assert!(
            engine
                .workspace()
                .capture_item_source(ROOT, hidden_tabs, hidden)
                .is_ok()
        );
        assert!(engine.workspace().item_is_hidden(hidden));
        assert!(engine.workspace().root(CONTAINED_ROOT).is_none());
        assert!(
            engine
                .workspace()
                .contained_floating(CONTAINED_FLOATING)
                .is_none()
        );
        republish_surface_with_active_provider(&mut engine, &mut host, &provider, 3);
        let committed = interaction(&engine);
        let pane = committed
            .plan()
            .tabs_records()
            .iter()
            .find(|pane| pane.id().tabs == visible_tabs)
            .unwrap();
        assert_eq!(preview, pane.bounds());
        assert!(
            committed
                .plan()
                .tab_records()
                .iter()
                .all(|tab| tab.id().tab.item() != Some(hidden))
        );
        engine.workspace().validate().unwrap();
    }
}
