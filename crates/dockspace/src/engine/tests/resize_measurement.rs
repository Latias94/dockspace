//! Resize-invalidating measurements retain the exact gesture and sample they checked.

use super::*;
use crate::input::journal::{PointerDeviceKind, SurfaceLocalPointerProvider};
use crate::interaction::ResizeSessionId;
use crate::model::DockspaceGridShape;

struct Fixture {
    engine: DockEngine,
    host: PresentationHostLease,
    provider: SurfaceLocalPointerProvider,
    owner: GestureOwner,
}

fn wide() -> LogicalRect {
    LogicalRect::new(0.0, 0.0, 404.0, 204.0).unwrap()
}

fn tall() -> LogicalRect {
    LogicalRect::new(0.0, 0.0, 204.0, 404.0).unwrap()
}

fn fixture() -> Fixture {
    let mut builder = Workspace::builder();
    let children = (1..=5)
        .map(|item| builder.insert_node(Node::pane(ItemId::new(item))))
        .collect();
    let grid = builder.insert_node(Node::Grid {
        shape: DockspaceGridShape::Auto,
        children,
        shares: None,
        layout_title: None,
    });
    builder.set_root(SOURCE_ROOT, RootRecord::new(grid));
    builder.set_surface(SOURCE_SURFACE, SurfacePresentation::with_main(SOURCE_ROOT));
    let config = DockPresentationConfig::builder()
        .minimum_pane_size(1.0, 1.0)
        .splitter_thickness(4.0)
        .build()
        .unwrap();
    let mut engine = DockEngine::new_with_presentation_config(
        builder.build().unwrap(),
        DockPolicy::default(),
        config,
    )
    .unwrap();
    let host = engine.create_presentation_host().unwrap();
    publish_surface_projection(&mut engine, host, SOURCE_SURFACE, wide());
    let projection = engine
        .interaction_projection(SOURCE_SURFACE)
        .expect("the fixture must publish real pointer authority");
    let grid = projection
        .plan()
        .layout_facts()
        .unwrap()
        .root(SOURCE_ROOT)
        .unwrap()
        .projection()
        .grids
        .get(&grid)
        .unwrap();
    assert_eq!((grid.shape.rows, grid.shape.columns), (2, 3));
    let provider = engine
        .create_surface_local_pointer_provider(
            SurfaceLocalPointerScope::new(
                host,
                SurfaceLocalPointerEndpoint::Logical(SOURCE_SURFACE),
            ),
            PointerEdgeSequence::new(0),
        )
        .unwrap();
    let owner = GestureOwner::new(
        PointerStreamId::new(provider.lease(), TEST_POINTER, 1),
        GestureLane::SurfaceLocal,
    );
    Fixture {
        engine,
        host,
        provider,
        owner,
    }
}

fn begin_resize(engine: &mut DockEngine, owner: GestureOwner) -> ResizeSessionId {
    let presentation = JournalSurfacePresentation::from_interaction(
        engine.interaction_projection(SOURCE_SURFACE).unwrap(),
    );
    let record = presentation
        .plan()
        .splitter_records()
        .iter()
        .find(|record| record.axis() == Axis::Horizontal && record.operable())
        .unwrap();
    let draw = record.draw_bounds();
    let point = LogicalPoint::new(
        draw.x() + draw.width() * 0.5,
        draw.y() + draw.height() * 0.25,
    )
    .unwrap();
    let region = presentation
        .hit_manifest()
        .region_for_kind(PresentationHitRegionKind::SplitterHandle(*record.id()))
        .unwrap()
        .id();
    let start = engine
        .prepare_journal_splitter_gesture(
            &presentation,
            region,
            owner,
            Authority::Known(PointerCaptureOwner::ProviderEndpoint),
            point,
            PointerDeviceKind::Mouse,
            &engine.policy,
        )
        .expect("a real presented column handle must freeze its resize frontier");
    let (session, replaced) = engine
        .interaction
        .begin_resize(engine.version.epoch(), start)
        .unwrap();
    assert_eq!(replaced, None);
    session
}

fn set_sample(engine: &mut DockEngine, session: ResizeSessionId, delta: f64) {
    let resize = engine.interaction.active_resize(session).unwrap();
    let updates = super::super::splitter_geometry::resize_axis_group_updates(
        &resize.axis_groups,
        [delta, 0.0],
    )
    .expect("the sample must be feasible in the frozen presented grid");
    crate::graph::operation::validate_track_resizes(&engine.workspace, &engine.policy, &updates)
        .expect("the replacement sample must pass command authority and policy");
    let point = LogicalPoint::new(
        resize.initial_pointer.x() + delta,
        resize.initial_pointer.y(),
    )
    .unwrap();
    engine
        .interaction
        .set_resize_updates(session, point, updates)
        .unwrap();
}

fn invalidating_measurement(engine: &DockEngine) -> PreparedSurfaceContribution {
    let contribution = begin_surface_measurement(engine, SOURCE_SURFACE, tall());
    let invalidated = contribution
        .invalidated_resize
        .as_ref()
        .expect("fresh 3x2 bounds must invalidate the frozen 2x3 driver");
    let resize = engine
        .interaction
        .active_resize(invalidated.session)
        .unwrap();
    assert_eq!(invalidated.owner, resize.owner);
    assert_eq!(invalidated.updates, resize.updates);
    let PreparedSurfaceContributionState::Ready { plan } = &contribution.state else {
        panic!("the invalidating measurements must still produce a paintable clean plan");
    };
    assert!(plan.resize_projection().is_empty());
    assert!(engine.invalidated_resize_contribution_is_current(&contribution));
    contribution
}

fn reject_without_effects(
    engine: &mut DockEngine,
    contribution: PreparedSurfaceContribution,
) -> SurfaceContributionRejection {
    let workspace = engine.workspace.clone();
    let version = engine.version();
    let interaction = engine.interaction.clone();
    let scene = engine.scene().clone();
    let serial = engine
        .presentation_authority
        .last_presentation_output_serial;
    let mut events = Vec::new();
    let outcome = engine
        .reduce_surface_contribution_with_interaction_events(
            ReducerTickId::new(900),
            contribution,
            engine.policy.revision(),
            &mut events,
        )
        .unwrap();
    let SurfaceContributionOutcome::Rejected { surface, reason } = outcome else {
        panic!("a stale compile result must be rejected: {outcome:?}");
    };
    assert_eq!(surface, SOURCE_SURFACE);
    assert_eq!(engine.workspace(), &*workspace);
    assert_eq!(engine.version(), version);
    assert_eq!(engine.interaction, interaction);
    assert_eq!(engine.scene(), &scene);
    assert_eq!(
        engine
            .presentation_authority
            .last_presentation_output_serial,
        serial
    );
    assert!(events.is_empty(), "rejected measurements must not cancel");
    reason
}

fn empty_pointer_frame(fixture: &Fixture) -> CoreHostFrame {
    let mut frame = begin_test_host_frame(&fixture.engine, fixture.host);
    let watermark = fixture.provider.committed_through();
    frame
        .submit_surface_pointer_journal(
            &fixture.provider,
            PointerEdgeJournal::new(watermark, watermark, Vec::new()).unwrap(),
        )
        .unwrap();
    frame
        .submit_pointer_receiver_receipts(PointerReceiverReceiptBatch::new([]).unwrap())
        .unwrap();
    frame
}

#[test]
fn invalidated_measurement_rejects_a_later_sample_of_the_same_resize() {
    let mut fixture = fixture();
    let session = begin_resize(&mut fixture.engine, fixture.owner);
    set_sample(&mut fixture.engine, session, 16.0);
    let contribution = invalidating_measurement(&fixture.engine);
    set_sample(&mut fixture.engine, session, 24.0);
    assert!(
        !fixture
            .engine
            .invalidated_resize_contribution_is_current(&contribution)
    );
    assert!(matches!(
        reject_without_effects(&mut fixture.engine, contribution),
        SurfaceContributionRejection::ResizeProjectionChanged { surface } if surface == SOURCE_SURFACE
    ));
}

#[test]
fn invalidated_measurement_cannot_cancel_a_new_session_with_the_same_owner_and_sample() {
    let mut fixture = fixture();
    let first = begin_resize(&mut fixture.engine, fixture.owner);
    set_sample(&mut fixture.engine, first, 16.0);
    let contribution = invalidating_measurement(&fixture.engine);
    assert_eq!(
        fixture.engine.interaction.cancel_owner(fixture.owner),
        Some(InteractionStatus::Resizing { session: first })
    );
    let second = begin_resize(&mut fixture.engine, fixture.owner);
    assert_ne!(first, second);
    set_sample(&mut fixture.engine, second, 16.0);
    assert_eq!(
        contribution.invalidated_resize.as_ref().unwrap().updates,
        fixture
            .engine
            .interaction
            .active_resize(second)
            .unwrap()
            .updates,
        "only the session identity distinguishes this replacement"
    );
    assert!(matches!(
        reject_without_effects(&mut fixture.engine, contribution),
        SurfaceContributionRejection::ResizeProjectionChanged { .. }
    ));
}

#[test]
fn invalidated_measurement_still_checks_its_original_surface_base() {
    let mut fixture = fixture();
    let session = begin_resize(&mut fixture.engine, fixture.owner);
    set_sample(&mut fixture.engine, session, 16.0);
    let contribution = invalidating_measurement(&fixture.engine);
    let replacement = begin_surface_measurement(&fixture.engine, SOURCE_SURFACE, wide());
    assert!(replacement.invalidated_resize.is_none());
    let mut events = Vec::new();
    let outcome = fixture
        .engine
        .reduce_surface_contribution_with_interaction_events(
            ReducerTickId::new(899),
            replacement,
            fixture.engine.policy.revision(),
            &mut events,
        )
        .unwrap();
    assert!(matches!(outcome, SurfaceContributionOutcome::Ready { .. }));
    assert!(events.is_empty());
    assert!(matches!(
        reject_without_effects(&mut fixture.engine, contribution),
        SurfaceContributionRejection::StaleBase { .. }
    ));
}

#[test]
fn invalidated_measurement_still_checks_its_original_policy_revision() {
    let mut fixture = fixture();
    let session = begin_resize(&mut fixture.engine, fixture.owner);
    set_sample(&mut fixture.engine, session, 16.0);
    let contribution = invalidating_measurement(&fixture.engine);
    let submitted = fixture.engine.policy.revision();
    let current = submitted.checked_next().unwrap();
    fixture.engine.policy = DockPolicy::default().snapshot(current);
    assert!(matches!(
        reject_without_effects(&mut fixture.engine, contribution),
        SurfaceContributionRejection::PolicyAuthorityChanged {
            submitted: actual_submitted, current: actual_current
        } if actual_submitted == submitted && actual_current == current
    ));
}

#[test]
fn ordinary_clean_measurement_does_not_bypass_a_new_resize_projection() {
    let mut fixture = fixture();
    let mut frame = empty_pointer_frame(&fixture);
    let contribution = begin_surface_measurement(frame.candidate_engine(), SOURCE_SURFACE, tall());
    assert!(contribution.invalidated_resize.is_none());
    let before = frame.candidate.interaction.clone();
    let session = begin_resize(&mut frame.candidate, fixture.owner);
    set_sample(&mut frame.candidate, session, 16.0);
    assert!(!frame.candidate.prepared_surface_matches_resize_projection(
        &before,
        SOURCE_SURFACE,
        &contribution,
    ));
    frame.push_surface_contribution(contribution).unwrap();
    complete_host_frame_with_explicit_surface_roster(&fixture.engine, &mut frame);
    let transition = frame.finish(&mut fixture.engine).unwrap();
    assert!(matches!(
        transition.surface_contributions(),
        [SurfaceContributionOutcome::Rejected {
            reason: SurfaceContributionRejection::ResizeProjectionChanged { .. },
            ..
        }]
    ));
    assert_eq!(
        fixture.engine.interaction.status(),
        InteractionStatus::Resizing { session }
    );
    assert!(transition.interaction_events().is_empty());
}

#[test]
fn only_adopted_measurements_cancel_inline_and_deferred_resizes() {
    for compile_inline in [false, true] {
        for commit in [false, true] {
            for delta in [0.0, 16.0] {
                let mut fixture = fixture();
                let session = begin_resize(&mut fixture.engine, fixture.owner);
                set_sample(&mut fixture.engine, session, delta);
                assert_eq!(
                    fixture
                        .engine
                        .interaction
                        .active_resize(session)
                        .unwrap()
                        .updates
                        .is_empty(),
                    delta == 0.0
                );
                let workspace = fixture.engine.workspace.clone();
                let version = fixture.engine.version();
                let interaction = fixture.engine.interaction.clone();
                let scene = fixture.engine.scene().clone();
                let mut frame = empty_pointer_frame(&fixture);
                let contribution = invalidating_measurement(frame.candidate_engine());
                if compile_inline {
                    frame.compile_surface_contribution(contribution).unwrap();
                    assert_eq!(frame.view().interaction().status(), InteractionStatus::Idle);
                } else {
                    frame.push_surface_contribution(contribution).unwrap();
                    assert_eq!(frame.view().interaction().status(), interaction.status());
                }
                if !commit {
                    drop(frame);
                    assert_eq!(fixture.engine.interaction, interaction);
                    assert_eq!(fixture.engine.scene(), &scene);
                    assert_eq!(fixture.engine.workspace(), &*workspace);
                    assert_eq!(fixture.engine.version(), version);
                    continue;
                }
                if compile_inline {
                    complete_host_frame_with_current_outputs(&fixture.engine, &mut frame);
                } else {
                    complete_host_frame_with_explicit_surface_roster(&fixture.engine, &mut frame);
                }
                let transition = frame.finish(&mut fixture.engine).unwrap();
                assert!(matches!(
                    transition.surface_contributions(),
                    [SurfaceContributionOutcome::Ready { .. }]
                ));
                assert!(matches!(
                    transition.interaction_events(),
                    [event] if matches!(event.kind(),
                        InteractionEventKind::Cancelled {
                            status: InteractionStatus::Resizing { session: actual },
                            reason: InteractionCancelReason::SceneUnavailable,
                        } if *actual == session)
                ));
                assert_eq!(fixture.engine.interaction.status(), InteractionStatus::Idle);
                assert_eq!(fixture.engine.workspace(), &*workspace);
                assert_eq!(fixture.engine.version(), version);
                assert!(
                    fixture
                        .engine
                        .scene()
                        .ready_candidate(SOURCE_SURFACE)
                        .unwrap()
                        .plan()
                        .resize_projection()
                        .is_empty()
                );
            }
        }
    }
}
