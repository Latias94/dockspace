use super::*;

use crate::input::intent::{Authority, AuthorityUnavailableReason, PointerId};
use crate::input::journal::{
    DesktopRouteFact, PointerCaptureOwner, PointerEdge, PointerEdgeKind, PointerEdgeLocation,
    SurfaceLocalPointerEndpoint, SurfaceLocalPointerScope,
};
use crate::platform::effect::{DispatchFailureReason, EffectDispatchResult, EffectId};
use crate::platform::focus::{FocusObservationEnvelope, FocusObservationGeneration};
use crate::platform::provider::PlatformObservationAuthority;
use crate::platform::viewport::{
    CapabilityObservationGeneration, CloseObservationGeneration, InventoryObservationGeneration,
    PlatformSnapshotGeneration, WindowIncarnation, WindowToken, WorkAreaObservationGeneration,
};
use crate::platform::{
    CapabilityRosterObservation, CloseEffectAcknowledgement, WindowCloseObservation,
    WindowCloseState, WindowInventoryObservation, WorkAreaRosterObservation,
};
use crate::presentation::observation::PresentationLedger;
use crate::{ids::SurfaceId, ids::WorkspaceEpoch};

fn platform_provider(domain: EngineAuthorityDomainId) -> PlatformObservationLease {
    let mut authority = PlatformObservationAuthority::new(domain);
    authority
        .create()
        .expect("test platform provider must mint")
}

fn joined_pointer_provider(host: PresentationHostLease, incarnation: u64) -> PointerInputLease {
    PointerInputLease::new(
        host.authority_domain(),
        incarnation,
        PointerProviderScope::NativeJoined(host),
    )
}

fn presentation_host(domain: EngineAuthorityDomainId) -> PresentationHostLease {
    let mut ledger = PresentationLedger::new(domain);
    ledger
        .create_host()
        .expect("test presentation host must mint")
}

fn recorder(domain: u64) -> BackendIngressRecorder {
    let domain = EngineAuthorityDomainId::new_for_test(domain);
    let host = presentation_host(domain);
    BackendIngressRecorder::new(
        platform_provider(domain),
        joined_pointer_provider(host, 1),
        host,
        PointerEdgeSequence::new(0),
    )
    .expect("matching joined native providers must create a recorder")
}

fn binding(domain: u64, token: u64) -> ViewportBinding {
    ViewportBinding::new(
        EngineAuthorityDomainId::new_for_test(domain),
        WorkspaceEpoch::new(0),
        SurfaceId::new(token),
        WindowToken::new(token),
        WindowIncarnation::new(1),
    )
}

pub(super) fn authority_and_recorder(
    domain: u64,
) -> (BackendIngressAuthority, BackendIngressRecorder) {
    let domain = EngineAuthorityDomainId::new_for_test(domain);
    let host = presentation_host(domain);
    let mut authority = BackendIngressAuthority::new(domain);
    let recorder = authority
        .enroll(
            platform_provider(domain),
            joined_pointer_provider(host, 1),
            host,
            PointerEdgeSequence::new(0),
        )
        .expect("matching backend authority must enroll");
    (authority, recorder)
}

fn snapshot(generation: u64) -> PlatformSnapshot {
    snapshot_with_close_observations(generation, Vec::new())
}

fn snapshot_with_close_observations(
    generation: u64,
    close_observations: Vec<WindowCloseObservation>,
) -> PlatformSnapshot {
    let reason = AuthorityUnavailableReason::NotReported;
    PlatformSnapshot::new(
        PlatformSnapshotGeneration::new(generation),
        CapabilityRosterObservation::unknown(
            CapabilityObservationGeneration::new(generation),
            reason,
        ),
        FocusObservationEnvelope::new(
            FocusObservationGeneration::new(generation),
            Authority::Unknown(reason),
            Authority::Unknown(reason),
        ),
        WindowInventoryObservation::unknown(
            InventoryObservationGeneration::new(generation),
            reason,
        ),
        Vec::new(),
        close_observations,
        WorkAreaRosterObservation::unknown(WorkAreaObservationGeneration::new(generation), reason),
    )
    .expect("unknown platform facts form a canonical snapshot")
}

fn effect_result(effect: u64) -> EffectResult {
    EffectResult::new(
        EffectId::new(effect),
        WorkspaceEpoch::new(0),
        EffectDispatchResult::DispatchFailed(DispatchFailureReason::AdapterRejected),
    )
}

fn pointer_segment(previous: u64) -> PointerEdgeJournal {
    let previous = PointerEdgeSequence::new(previous);
    let through = previous
        .checked_next()
        .expect("test pointer sequence must advance");
    let reason = AuthorityUnavailableReason::NotReported;
    PointerEdgeJournal::new(
        previous,
        through,
        vec![PointerEdge::new(
            through,
            PointerId::new(1),
            PointerEdgeKind::Moved,
            PointerEdgeLocation::Desktop {
                route: DesktopRouteFact::unknown(Authority::Unknown(reason), reason),
            },
            Authority::Known(PointerCaptureOwner::None),
        )],
    )
    .expect("test pointer segment must be contiguous")
}

#[test]
fn recorder_binds_exact_matching_native_joined_provider_incarnations() {
    let domain = EngineAuthorityDomainId::new_for_test(1);
    let host = presentation_host(domain);
    let recorder = BackendIngressRecorder::new(
        platform_provider(domain),
        joined_pointer_provider(host, 7),
        host,
        PointerEdgeSequence::new(3),
    )
    .expect("matching providers must be admitted");

    assert_eq!(recorder.lease().authority_domain(), domain);
    assert_eq!(recorder.lease().platform_incarnation(), 1);
    assert_eq!(recorder.lease().pointer_incarnation(), 7);
    assert_eq!(recorder.lease().presentation_host(), host);
    assert_eq!(
        recorder.lease().pointer_provider().scope(),
        PointerProviderScope::NativeJoined(host)
    );
}

#[test]
fn recorder_rejects_foreign_and_non_joined_pointer_providers() {
    let first = EngineAuthorityDomainId::new_for_test(1);
    let second = EngineAuthorityDomainId::new_for_test(2);
    let platform = platform_provider(first);
    let foreign = joined_pointer_provider(presentation_host(second), 1);
    assert_eq!(
        BackendIngressRecorder::new(
            platform,
            foreign,
            presentation_host(first),
            PointerEdgeSequence::new(0),
        )
        .expect_err("foreign pointer authority must fail"),
        BackendIngressError::AuthorityDomainMismatch {
            platform: first,
            pointer: second,
        }
    );

    let mut presentations = PresentationLedger::new(first);
    let host = presentations
        .create_host()
        .expect("test presentation host must mint");
    for scope in [
        PointerProviderScope::DesktopGlobal,
        PointerProviderScope::HostLocal(host),
        PointerProviderScope::SurfaceLocal(SurfaceLocalPointerScope::new(
            host,
            SurfaceLocalPointerEndpoint::Logical(SurfaceId::new(1)),
        )),
    ] {
        let local = PointerInputLease::new(first, 2, scope);
        assert_eq!(
            BackendIngressRecorder::new(platform, local, host, PointerEdgeSequence::new(0))
                .expect_err("non-joined pointer authority must fail"),
            BackendIngressError::PointerProviderNotNativeJoined { provider: local }
        );
    }
}

#[test]
fn recorder_rejects_foreign_or_inexact_joined_presentation_hosts() {
    let domain = EngineAuthorityDomainId::new_for_test(1);
    let platform = platform_provider(domain);
    let mut presentations = PresentationLedger::new(domain);
    let host = presentations.create_host().expect("bound host");
    let other_host = presentations.create_host().expect("other local host");
    let pointer = joined_pointer_provider(host, 1);
    assert_eq!(
        BackendIngressRecorder::new(platform, pointer, other_host, PointerEdgeSequence::new(0))
            .expect_err("another host in the same domain must fail"),
        BackendIngressError::PresentationHostLeaseMismatch {
            expected: host,
            submitted: other_host,
        }
    );
    let foreign_host = presentation_host(EngineAuthorityDomainId::new_for_test(2));
    assert_eq!(
        BackendIngressRecorder::new(platform, pointer, foreign_host, PointerEdgeSequence::new(0))
            .expect_err("foreign presentation authority must fail"),
        BackendIngressError::PresentationAuthorityDomainMismatch {
            backend: domain,
            presentation: foreign_host.authority_domain(),
        }
    );
    let foreign_scope =
        PointerInputLease::new(domain, 1, PointerProviderScope::NativeJoined(foreign_host));
    assert_eq!(
        BackendIngressRecorder::new(platform, foreign_scope, host, PointerEdgeSequence::new(0))
            .expect_err("a joined scope cannot smuggle a foreign host"),
        BackendIngressError::PresentationHostLeaseMismatch {
            expected: foreign_host,
            submitted: host,
        }
    );
}

#[test]
fn recorder_mints_one_contiguous_order_across_all_payload_lanes() {
    let mut recorder = recorder(1);
    let platform = recorder
        .record_platform_snapshot(WorkspaceEpoch::new(0), snapshot(1))
        .expect("platform record must fit");
    let effect = recorder
        .record_platform_effect_result(effect_result(1))
        .expect("effect record must fit");
    let close = recorder
        .record_native_close_observation(
            WorkspaceEpoch::new(0),
            WindowCloseObservation::new(
                crate::platform::viewport::ViewportBinding::new(
                    EngineAuthorityDomainId::new_for_test(1),
                    WorkspaceEpoch::new(0),
                    SurfaceId::new(1),
                    WindowToken::new(1),
                    WindowIncarnation::new(1),
                ),
                CloseObservationGeneration::new(1),
                Authority::Known(WindowCloseState::LiveRequested),
                CloseEffectAcknowledgement::known(None),
            ),
        )
        .expect("close record must fit");
    let semantic = recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("semantic record must fit");
    let pointer = recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("pointer record must fit");

    assert_eq!(
        [
            platform.get(),
            effect.get(),
            close.get(),
            semantic.get(),
            pointer.get(),
        ],
        [1, 2, 3, 4, 5],
    );
    let batch = recorder
        .pending_batch()
        .expect("recorder-owned batch must validate");
    assert_eq!(batch.previous().get(), 0);
    assert_eq!(batch.through().get(), 5);
    assert_eq!(
        batch
            .records()
            .iter()
            .map(|record| record.ordinal().get())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert!(matches!(
        batch.records()[0].payload(),
        BackendIngressPayload::PlatformSnapshot { .. }
    ));
    assert!(matches!(
        batch.records()[1].payload(),
        BackendIngressPayload::PlatformEffectResult(_)
    ));
    assert!(matches!(
        batch.records()[2].payload(),
        BackendIngressPayload::NativeCloseObservation { .. }
    ));
    assert!(matches!(
        batch.records()[3].payload(),
        BackendIngressPayload::SemanticInput(input)
            if matches!(input.as_ref(), EngineInput::ValidateWorkspace)
    ));
    assert!(matches!(
        batch.records()[4].payload(),
        BackendIngressPayload::PointerSegment(_)
    ));
}

#[test]
fn draining_the_single_writer_freezes_its_final_backend_and_pointer_frontiers() {
    let mut recorder = recorder(1);
    let lease = recorder.lease();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("semantic record must fit");
    recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("pointer record must fit");

    let receipt = recorder.drain();

    assert_eq!(receipt.lease(), lease);
    assert_eq!(receipt.recorded_through(), BackendIngressOrdinal(2));
    assert_eq!(receipt.pointer_through(), PointerEdgeSequence::new(1));
}

#[test]
fn rejected_pointer_gap_does_not_consume_ingress_or_pointer_watermarks() {
    let mut recorder = recorder(1);
    assert_eq!(
        recorder
            .record_pointer_segment(pointer_segment(1))
            .expect_err("pointer gap must fail"),
        BackendIngressError::PointerSegmentPreviousMismatch {
            expected: PointerEdgeSequence::new(0),
            submitted: PointerEdgeSequence::new(1),
        }
    );
    assert_eq!(recorder.recorded_through().get(), 0);
    assert!(
        recorder
            .pending_batch()
            .expect("empty recorder-owned batch must validate")
            .is_empty()
    );

    assert_eq!(
        recorder
            .record_pointer_segment(pointer_segment(0))
            .expect("retry from the retained pointer watermark must work")
            .get(),
        1
    );
}

#[test]
fn savepoint_rolls_back_records_and_the_pointer_lane_atomically() {
    let mut recorder = recorder(1);
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the prefix must fit");
    let savepoint = recorder.savepoint();
    recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("the transaction pointer edge must fit");
    recorder
        .record_platform_effect_result(effect_result(1))
        .expect("the transaction effect must fit");

    recorder
        .rollback_to(savepoint)
        .expect("the unchanged recorder prefix is rollback-safe");

    let batch = recorder
        .pending_batch()
        .expect("the restored prefix remains a valid batch");
    assert_eq!(batch.through().get(), 1);
    assert_eq!(batch.len(), 1);
    assert_eq!(
        recorder
            .record_pointer_segment(pointer_segment(0))
            .expect("the pointer watermark must also roll back")
            .get(),
        2
    );
}

#[test]
fn stale_nested_savepoint_cannot_accept_a_rewritten_public_ordinal() {
    let mut recorder = recorder(1);
    let outer = recorder.savepoint();
    let abandoned = recorder
        .record_semantic_input_receipt(EngineInput::ValidateWorkspace)
        .expect("the first branch record must fit");
    let stale_nested = recorder.savepoint();

    recorder
        .rollback_to(outer)
        .expect("the outer boundary must abandon the first branch");
    assert!(!abandoned.is_live());
    let replacement = recorder
        .record_semantic_input_receipt(EngineInput::ValidateWorkspace)
        .expect("the replacement may reuse only the public ordinal");
    assert_eq!(abandoned.ordinal(), replacement.ordinal());

    assert_eq!(
        recorder
            .rollback_to(stale_nested)
            .expect_err("a stale nested boundary cannot authenticate the rewritten branch"),
        BackendIngressError::SavepointPrefixRewritten {
            boundary: BackendIngressOrdinal(1),
        }
    );
    assert!(replacement.is_live());
    assert_eq!(recorder.retained_record_count(), 1);
}

#[test]
fn immutable_batch_is_exactly_retryable_and_validates_commit_authority() {
    let mut recorder = recorder(1);
    recorder
        .record_platform_snapshot(WorkspaceEpoch::new(0), snapshot(1))
        .expect("platform record must fit");
    recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("pointer record must fit");

    let first = recorder
        .batch_after(BackendIngressOrdinal::ORIGIN)
        .expect("origin suffix must exist");
    let retry = recorder
        .batch_after(BackendIngressOrdinal::ORIGIN)
        .expect("retry must preserve the same batch");
    assert_eq!(retry, first);
    assert_eq!(
        first.validate_against(
            recorder.lease(),
            BackendIngressOrdinal::ORIGIN,
            BackendIngressRecordIdentity::ORIGIN,
        ),
        Ok(())
    );
    assert_eq!(
        first
            .validate_against(
                recorder.lease(),
                BackendIngressOrdinal(1),
                BackendIngressRecordIdentity::ORIGIN,
            )
            .expect_err("a different engine watermark must fail"),
        BackendIngressError::BatchPreviousMismatch {
            expected: BackendIngressOrdinal(1),
            submitted: BackendIngressOrdinal::ORIGIN,
        }
    );
}

#[test]
fn ingress_payload_fixed_storage_stays_within_the_snapshot_envelope() {
    assert!(
        size_of::<BackendIngressPayload>()
            <= size_of::<PlatformSnapshot>()
                + size_of::<WorkspaceEpoch>()
                + std::mem::align_of::<BackendIngressPayload>(),
        "semantic inputs must not inflate every pointer payload allocation",
    );
}

#[test]
fn transport_capacity_includes_spare_slots_and_releases_consumed_receipt_storage() {
    let mut recorder = recorder(1);
    assert!(recorder.retained_records().is_empty());
    assert_eq!(recorder.record_buffer_capacity_bytes(), 0);
    assert_eq!(recorder.shared_fixed_data_bytes(), size_of::<Mutex<()>>());
    recorder.records.reserve(8);
    recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("one pointer record must append");
    assert_eq!(recorder.retained_records().len(), 1);
    assert_eq!(
        recorder.retained_records().as_ptr(),
        recorder.records.as_ptr(),
    );
    assert!(recorder.records.capacity() > recorder.retained_records().len());
    assert_eq!(
        recorder.record_buffer_capacity_bytes(),
        recorder.records.capacity() * size_of::<BackendIngressRecord>(),
    );
    assert_eq!(
        recorder.shared_fixed_data_bytes(),
        size_of::<Mutex<()>>() + size_of::<BackendIngressPayload>() + size_of::<AtomicU8>(),
    );

    let mut batch = recorder.pending_batch().expect("the record must freeze");
    batch.records.reserve(8);
    assert!(batch.records.capacity() > batch.len());
    assert_eq!(
        batch.record_buffer_capacity_bytes(),
        batch.records.capacity() * size_of::<BackendIngressRecord>(),
    );
    let mut guard = batch.commit_guard();
    guard.records.reserve(8);
    assert!(guard.records.capacity() > guard.records.len());
    assert_eq!(
        guard.owned_capacity_bytes(),
        guard.records.capacity()
            * size_of::<(BackendIngressOrdinal, BackendIngressRecordLiveness)>(),
    );

    let mut bindings = Vec::with_capacity(8);
    bindings.push(binding(1, 7));
    let bytes = bindings.capacity() * size_of::<ViewportBinding>();
    let mut receipt = BackendIngressPrefixRetirementReceipt::new(
        recorder.lease(),
        recorder.recorded_through(),
        bindings,
    );
    assert!(bytes > size_of::<ViewportBinding>());
    assert_eq!(receipt.owned_capacity_bytes(), bytes);
    receipt.consume().expect("the receipt must consume once");
    assert_eq!(receipt.owned_capacity_bytes(), 0);
}

#[test]
fn payload_owner_is_shared_by_recorder_frozen_batches_and_retries() {
    let (authority, mut recorder) = authority_and_recorder(1);
    let platform = snapshot_with_close_observations(
        1,
        vec![WindowCloseObservation::new(
            binding(1, 1),
            CloseObservationGeneration::new(1),
            Authority::Known(WindowCloseState::Destroyed),
            CloseEffectAcknowledgement::known(None),
        )],
    );
    let close_observations = platform.close_observations().as_ptr();
    let pointer = pointer_segment(0);
    let edges = pointer.edges().as_ptr();
    let group = recorder.savepoint();
    recorder
        .record_platform_snapshot(WorkspaceEpoch::new(0), platform)
        .expect("the platform payload must append");
    recorder
        .record_pointer_segment(pointer)
        .expect("the pointer payload must append");
    recorder
        .finish_atomic_append(group)
        .expect("the captured group must finalize before freezing");

    let recorder_buffer = recorder.record_buffer_capacity_bytes();
    let recorder_shared = recorder.shared_fixed_data_bytes();
    let first = recorder.pending_batch().expect("the group must freeze");
    let second = recorder.pending_batch().expect("another batch must freeze");
    let cloned = first.clone();
    assert_ne!(first.records.as_ptr(), recorder.retained_records().as_ptr());
    assert_ne!(first.records.as_ptr(), cloned.records.as_ptr());
    assert_eq!(recorder.record_buffer_capacity_bytes(), recorder_buffer);
    assert_eq!(recorder.shared_fixed_data_bytes(), recorder_shared);
    assert_eq!(
        cloned
            .commit_guard()
            .publish_with(|| Err::<(), _>("rejected"))
            .expect("the shared payload must remain publication-safe"),
        Err("rejected"),
    );
    let retry = recorder
        .batch_after(BackendIngressOrdinal::ORIGIN)
        .expect("failed publication must remain retryable");
    let prefix = recorder
        .batch_prefix_after(BackendIngressOrdinal::ORIGIN, NonZeroUsize::new(2).unwrap())
        .expect("the complete group must fit the selected prefix");

    for batch in [&first, &second, &cloned, &retry, &prefix] {
        assert_eq!(batch, &first);
        assert_eq!(
            batch.record_buffer_capacity_bytes(),
            batch.records.capacity() * size_of::<BackendIngressRecord>(),
        );
        authority
            .validate_batch(batch)
            .expect("each shared batch must retain live publication authority");
        for (recorded, frozen) in recorder.records.iter().zip(batch.records()) {
            assert!(Arc::ptr_eq(&recorded.payload, &frozen.payload));
            assert!(Arc::ptr_eq(&recorded.liveness.0, &frozen.liveness.0));
            assert!(std::ptr::eq(recorded.payload(), frozen.payload()));
            assert_eq!(recorded.identity, frozen.identity);
            assert_eq!(recorded.liveness, frozen.liveness);
            assert_eq!(recorded.is_commit_boundary(), frozen.is_commit_boundary());
        }
        assert!(!batch.records()[0].is_commit_boundary());
        assert!(batch.records()[1].is_commit_boundary());
        let BackendIngressPayload::PlatformSnapshot { snapshot, .. } = batch.records()[0].payload()
        else {
            panic!("the first record must retain its platform payload");
        };
        assert_eq!(snapshot.close_observations().len(), 1);
        assert_eq!(snapshot.close_observations().as_ptr(), close_observations);
        let BackendIngressPayload::PointerSegment(journal) = batch.records()[1].payload() else {
            panic!("the second record must retain its pointer payload");
        };
        assert_eq!(journal.edges().len(), 1);
        assert_eq!(journal.edges().as_ptr(), edges);
    }
    assert_eq!(recorder.shared_fixed_data_bytes(), recorder_shared);
}

#[test]
fn payload_owner_survives_rollback_without_authenticating_reused_ordinals() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    let savepoint = recorder.savepoint();
    recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("the original pointer payload must append");
    let owner = Arc::downgrade(&recorder.records[0].payload);
    let frozen = recorder.pending_batch().expect("the original must freeze");
    let second = recorder
        .batch_after(BackendIngressOrdinal::ORIGIN)
        .expect("another original batch must freeze");
    let cloned = frozen.clone();

    // Advancing the candidate ledger alone does not publish record liveness.
    authority
        .commit_batch(&frozen)
        .expect("the candidate ledger must retain the original identity");
    let watermark = authority
        .commit_watermark()
        .expect("the identity must exist");
    recorder
        .rollback_to(savepoint)
        .expect("an unpublished branch must remain revocable");
    assert_eq!(recorder.retained_record_count(), 0);
    assert!(owner.upgrade().is_some());
    for batch in [&frozen, &second, &cloned] {
        assert!(Weak::ptr_eq(
            &owner,
            &Arc::downgrade(&batch.records()[0].payload),
        ));
        assert_eq!(
            batch.commit_guard().validate(),
            Err(BackendIngressError::RecordRevoked {
                ordinal: BackendIngressOrdinal(1),
            }),
        );
    }

    recorder
        .record_pointer_segment(pointer_segment(0))
        .expect("rollback must restore the pointer frontier for the same payload");
    assert_eq!(recorder.records[0].ordinal, frozen.records()[0].ordinal);
    assert_eq!(recorder.records[0].payload(), frozen.records()[0].payload());
    assert_ne!(recorder.records[0].identity, frozen.records()[0].identity);
    assert!(!Arc::ptr_eq(
        &recorder.records[0].payload,
        &frozen.records()[0].payload,
    ));
    recorder
        .record_pointer_segment(pointer_segment(1))
        .expect("a later record must follow the rewritten predecessor");
    let suffix = recorder
        .batch_after(watermark.through())
        .expect("the rewritten predecessor must expose its suffix");
    assert_eq!(
        authority.validate_batch(&suffix),
        Err(BackendIngressError::BatchPreviousIdentityMismatch {
            ordinal: BackendIngressOrdinal(1),
        }),
    );
    assert_eq!(
        recorder.retire_committed_prefix(watermark),
        Err(BackendIngressError::CommitWatermarkContentMismatch {
            ordinal: BackendIngressOrdinal(1),
        }),
    );

    drop(frozen);
    drop(second);
    assert!(owner.upgrade().is_some());
    let BackendIngressPayload::PointerSegment(journal) = cloned.records()[0].payload() else {
        panic!("the revoked clone must retain its readable pointer payload");
    };
    assert_eq!(journal, &pointer_segment(0));
    drop(cloned);
    assert!(owner.upgrade().is_none());
}

#[test]
fn payload_owner_survives_committed_prefix_retirement_until_last_batch_drops() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    let pointer = pointer_segment(0);
    let edges = pointer.edges().as_ptr();
    recorder
        .record_pointer_segment(pointer)
        .expect("the retained pointer payload must append");
    let owner = Arc::downgrade(&recorder.records[0].payload);
    recorder
        .record_pointer_segment(pointer_segment(1))
        .expect("the independent suffix must append");
    let recorder_buffer = recorder.record_buffer_capacity_bytes();
    let recorder_shared = recorder.shared_fixed_data_bytes();
    let frozen = recorder.pending_batch().expect("both records must freeze");
    let prefix = recorder
        .batch_prefix_after(BackendIngressOrdinal::ORIGIN, NonZeroUsize::MIN)
        .expect("only the committed prefix must be selected");
    let cloned = prefix.clone();
    authority
        .commit_batch(&prefix)
        .expect("the candidate prefix identity must commit");
    prefix
        .commit_guard()
        .publish_with(|| Ok::<(), ()>(()))
        .expect("the exact prefix must remain publishable")
        .expect("the prepared prefix must publish");
    let watermark = authority
        .commit_watermark()
        .expect("the identity must exist");
    assert!(
        recorder
            .retire_committed_prefix(watermark)
            .expect("the exact committed prefix must retire")
            .is_none()
    );
    assert_eq!(recorder.retained_record_count(), 1);
    assert_eq!(recorder.record_buffer_capacity_bytes(), recorder_buffer);
    assert_eq!(
        recorder.shared_fixed_data_bytes(),
        recorder_shared - size_of::<BackendIngressPayload>() - size_of::<AtomicU8>(),
    );
    assert!(owner.upgrade().is_some());
    for batch in [&frozen, &prefix, &cloned] {
        assert!(Weak::ptr_eq(
            &owner,
            &Arc::downgrade(&batch.records()[0].payload),
        ));
        assert_eq!(batch.records()[0].identity, watermark.record_identity);
        assert_eq!(
            batch.commit_guard().validate(),
            Err(BackendIngressError::RecordAlreadyCommitted {
                ordinal: watermark.through(),
            }),
        );
    }
    let suffix = recorder
        .batch_after(watermark.through())
        .expect("the unretired suffix must retain its predecessor identity");
    authority
        .validate_batch(&suffix)
        .expect("the prefix retirement must preserve suffix replay authority");
    assert!(Arc::ptr_eq(
        &recorder.records[0].payload,
        &suffix.records()[0].payload,
    ));
    assert_eq!(recorder.record_buffer_capacity_bytes(), recorder_buffer);

    drop(frozen);
    drop(prefix);
    assert!(owner.upgrade().is_some());
    let BackendIngressPayload::PointerSegment(journal) = cloned.records()[0].payload() else {
        panic!("the committed clone must retain its readable pointer payload");
    };
    assert_eq!(journal.edges().len(), 1);
    assert_eq!(journal.edges().as_ptr(), edges);
    assert_eq!(journal, &pointer_segment(0));
    drop(cloned);
    assert!(owner.upgrade().is_none());
}

#[test]
fn batch_validation_rejects_reordering_and_cross_lease_splicing() {
    let mut first_recorder = recorder(1);
    first_recorder
        .record_platform_snapshot(WorkspaceEpoch::new(0), snapshot(1))
        .expect("first record must fit");
    first_recorder
        .record_platform_effect_result(effect_result(1))
        .expect("second record must fit");
    let first = first_recorder
        .pending_batch()
        .expect("first recorder-owned batch must validate");

    let mut reordered = first.records.clone();
    reordered.swap(0, 1);
    assert_eq!(
        BackendIngressBatch::from_records(
            first.coordination.clone(),
            first.lease,
            first.previous,
            first.previous_record_identity,
            first.through,
            reordered,
        )
        .expect_err("record reordering must fail"),
        BackendIngressError::RecordOrdinalGap {
            expected: BackendIngressOrdinal(1),
            submitted: BackendIngressOrdinal(2),
        }
    );

    let mut second_recorder = recorder(2);
    second_recorder
        .record_platform_snapshot(WorkspaceEpoch::new(0), snapshot(1))
        .expect("foreign record must fit its own recorder");
    let mut spliced = first.records.clone();
    spliced[0] = second_recorder
        .pending_batch()
        .expect("second recorder-owned batch must validate")
        .records[0]
        .clone();
    assert!(matches!(
        BackendIngressBatch::from_records(
            first.coordination.clone(),
            first.lease,
            first.previous,
            first.previous_record_identity,
            first.through,
            spliced,
        ),
        Err(BackendIngressError::RecordLeaseMismatch { ordinal, .. })
            if ordinal == BackendIngressOrdinal(1)
    ));
}

#[test]
fn committed_prefix_retirement_mints_one_affine_binding_quiescence_proof() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    let retired_binding = binding(1, 7);
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("prefix semantic input must fit");
    recorder
        .record_platform_binding_quiescence(retired_binding)
        .expect("binding quiescence must fit");
    let before_reclaim = recorder.savepoint();
    let batch = recorder.pending_batch().expect("prefix must freeze");
    authority
        .commit_batch(&batch)
        .expect("core authority must commit the exact prefix");
    let watermark = authority
        .commit_watermark()
        .expect("the committed authority must expose its exact watermark");
    let mut receipt = recorder
        .retire_committed_prefix(watermark)
        .expect("committed prefix must be reclaimable")
        .expect("an advancing prefix must mint a receipt");

    assert_eq!(receipt.through(), Some(BackendIngressOrdinal(2)));
    assert_eq!(receipt.binding_quiescences(), &[retired_binding]);
    assert_eq!(
        recorder
            .rollback_to(before_reclaim)
            .expect_err("a reclaimed prefix invalidates older savepoints"),
        BackendIngressError::SavepointPrefixReclaimed {
            saved: BackendIngressOrdinal::ORIGIN,
            current: BackendIngressOrdinal(2),
        }
    );

    authority
        .commit_prefix_retirement(&receipt)
        .expect("the same core authority must accept its reclaimed prefix");
    receipt.consume().expect("the proof is affine");
    assert_eq!(
        authority
            .commit_prefix_retirement(&receipt)
            .expect_err("a consumed proof cannot replay"),
        BackendIngressError::PrefixRetirementReceiptConsumed,
    );
}

#[test]
fn prefix_retirement_receipt_cannot_cross_a_later_committed_fact() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    recorder
        .record_platform_binding_quiescence(binding(1, 1))
        .expect("first quiescence must fit");
    let first_batch = recorder.pending_batch().expect("first prefix must freeze");
    authority
        .commit_batch(&first_batch)
        .expect("first prefix must commit");
    let first_watermark = authority
        .commit_watermark()
        .expect("the first commit must expose its exact watermark");
    let first = recorder
        .retire_committed_prefix(first_watermark)
        .expect("first prefix must reclaim")
        .expect("first prefix must mint a receipt");

    recorder
        .record_platform_binding_quiescence(binding(1, 2))
        .expect("second quiescence must fit");
    let second_batch = recorder.pending_batch().expect("second prefix must freeze");
    authority
        .commit_batch(&second_batch)
        .expect("second prefix must commit");
    let second_watermark = authority
        .commit_watermark()
        .expect("the second commit must expose its exact watermark");
    let second = recorder
        .retire_committed_prefix(second_watermark)
        .expect("second prefix must reclaim")
        .expect("second prefix must mint a receipt");

    assert_eq!(
        authority
            .commit_prefix_retirement(&first)
            .expect_err("a later committed fact must invalidate the older barrier"),
        BackendIngressError::PrefixRetirementWatermarkMismatch {
            committed: BackendIngressOrdinal(2),
            submitted: BackendIngressOrdinal(1),
        },
    );
    authority
        .commit_prefix_retirement(&second)
        .expect("the proof ending at the exact current boundary remains valid");
}

#[test]
fn committed_watermark_rejects_rollback_rewrite_at_the_same_ordinal() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    let savepoint = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the original record must fit");
    let committed = recorder
        .pending_batch()
        .expect("the original immutable batch must freeze");
    authority
        .commit_batch(&committed)
        .expect("core commits the original record identity");
    let watermark = authority
        .commit_watermark()
        .expect("the original commit must expose its exact identity");

    recorder
        .rollback_to(savepoint)
        .expect("the recorder may abandon its unretired branch");
    recorder
        .record_platform_binding_quiescence(binding(1, 9))
        .expect("rollback may reuse the public ordinal with a new record identity");

    assert_eq!(
        recorder
            .retire_committed_prefix(watermark)
            .expect_err("an old commit cannot authenticate rewritten quiescence"),
        BackendIngressError::CommitWatermarkContentMismatch {
            ordinal: BackendIngressOrdinal(1),
        },
    );
    assert_eq!(recorder.retained_record_count(), 1);
}

#[test]
fn recorder_rollback_revokes_every_previously_frozen_batch() {
    let (authority, mut recorder) = authority_and_recorder(1);
    let savepoint = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the original record must fit");
    let frozen = recorder
        .pending_batch()
        .expect("the original immutable batch must freeze");

    recorder
        .rollback_to(savepoint)
        .expect("recorder rollback must revoke the abandoned append branch");

    assert_eq!(
        authority
            .validate_batch(&frozen)
            .expect_err("a frozen batch cannot outlive recorder rollback"),
        BackendIngressError::RecordRevoked {
            ordinal: BackendIngressOrdinal::from_committed_source_sequence(1),
        },
    );
}

#[test]
fn rollback_preserves_a_frozen_prefix_and_revokes_its_frozen_suffix() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the retained prefix record must fit");
    let prefix = recorder
        .pending_batch()
        .expect("the retained prefix must freeze");
    let savepoint = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the abandoned suffix record must fit");
    let full = recorder
        .pending_batch()
        .expect("the complete branch must freeze before rollback");

    recorder
        .rollback_to(savepoint)
        .expect("rollback must preserve the exact frozen prefix");

    authority
        .validate_batch(&prefix)
        .expect("the frozen retained prefix must remain publishable");
    assert_eq!(
        authority
            .validate_batch(&full)
            .expect_err("the frozen full batch must retain revoked suffix identity"),
        BackendIngressError::RecordRevoked {
            ordinal: BackendIngressOrdinal::from_committed_source_sequence(2),
        }
    );
    authority
        .commit_batch(&prefix)
        .expect("the retained prefix must still commit exactly once");
}

#[test]
fn commit_guard_linearizes_publication_before_rollback() {
    let mut recorder = recorder(1);
    let savepoint = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the record must fit");
    let batch = recorder
        .pending_batch()
        .expect("the batch must freeze before publication");
    let coordination = batch.coordination.clone();

    let published = batch
        .commit_guard()
        .publish_with(|| {
            assert!(matches!(
                coordination.0.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            Ok::<_, ()>(())
        })
        .expect("the guard must publish while holding coordination");
    assert_eq!(published, Ok(()));
    assert!(coordination.0.try_lock().is_ok());

    assert_eq!(
        recorder
            .rollback_to(savepoint)
            .expect_err("rollback must not erase a published record"),
        BackendIngressError::RollbackCrossesPublishedRecord {
            ordinal: BackendIngressOrdinal::from_committed_source_sequence(1),
        }
    );
    assert_eq!(recorder.retained_record_count(), 1);

    let committed_prefix = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("a live suffix may follow the committed prefix");
    recorder
        .rollback_to(committed_prefix)
        .expect("rollback may still remove only the later live suffix");
    assert_eq!(recorder.retained_record_count(), 1);
}

#[test]
fn failed_publication_restores_record_rollback_authority() {
    let mut recorder = recorder(1);
    let savepoint = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the record must fit");
    let batch = recorder
        .pending_batch()
        .expect("the batch must freeze before publication");

    let published = batch
        .commit_guard()
        .publish_with(|| Err::<(), _>("rejected"))
        .expect("the guard itself must remain valid");
    assert_eq!(published, Err("rejected"));
    recorder
        .rollback_to(savepoint)
        .expect("failed publication must restore rollback authority");
    assert_eq!(recorder.retained_record_count(), 0);
}

#[test]
fn committed_predecessor_identity_rejects_a_rewritten_prefix_hidden_by_a_later_record() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    let savepoint = recorder.savepoint();
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("the original record must fit");
    let original = recorder
        .pending_batch()
        .expect("the original immutable batch must freeze");
    let original_identity = original.records[0].identity;
    authority
        .commit_batch(&original)
        .expect("core commits the original predecessor identity");

    recorder
        .rollback_to(savepoint)
        .expect("the recorder may abandon its unretired branch");
    recorder
        .record_platform_binding_quiescence(binding(1, 9))
        .expect("the rewritten predecessor must fit");
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("a later record must fit after the rewritten predecessor");
    let rewritten_suffix = recorder
        .batch_after(BackendIngressOrdinal(1))
        .expect("the recorder can expose the rewritten ordinal suffix");

    assert_ne!(rewritten_suffix.previous_record_identity, original_identity);
    assert_eq!(
        authority
            .validate_batch(&rewritten_suffix)
            .expect_err("the later record cannot hide a rewritten committed predecessor"),
        BackendIngressError::BatchPreviousIdentityMismatch {
            ordinal: BackendIngressOrdinal(1),
        },
    );
    let watermark = authority
        .commit_watermark()
        .expect("the rejected suffix must leave the original watermark intact");
    assert_eq!(watermark.through(), BackendIngressOrdinal(1));
    assert_eq!(
        recorder
            .retire_committed_prefix(watermark)
            .expect_err("rewritten quiescence must not mint a retirement receipt"),
        BackendIngressError::CommitWatermarkContentMismatch {
            ordinal: BackendIngressOrdinal(1),
        },
    );
    assert_eq!(recorder.retained_record_count(), 2);
}

#[test]
fn ordinary_prefix_reclamation_needs_no_engine_retirement_receipt() {
    let (mut authority, mut recorder) = authority_and_recorder(1);
    recorder
        .record_semantic_input(EngineInput::ValidateWorkspace)
        .expect("ordinary ingress must fit");
    let batch = recorder
        .pending_batch()
        .expect("ordinary prefix must freeze");
    authority
        .commit_batch(&batch)
        .expect("ordinary prefix must commit");
    let watermark = authority
        .commit_watermark()
        .expect("the ordinary commit must expose its exact watermark");

    assert!(
        recorder
            .retire_committed_prefix(watermark)
            .expect("ordinary prefix must reclaim")
            .is_none(),
        "prefixes without binding closure must not force an engine candidate clone",
    );
    assert_eq!(recorder.retained_record_count(), 0);
}

#[test]
fn binding_quiescence_rejects_foreign_authority_and_unretired_duplicates() {
    let mut recorder = recorder(1);
    assert_eq!(
        recorder
            .record_platform_binding_quiescence(binding(2, 1))
            .expect_err("foreign binding authority must fail"),
        BackendIngressError::BindingQuiescenceAuthorityMismatch {
            expected: EngineAuthorityDomainId::new_for_test(1),
            submitted: EngineAuthorityDomainId::new_for_test(2),
        }
    );
    let local = binding(1, 1);
    recorder
        .record_platform_binding_quiescence(local)
        .expect("local binding quiescence must fit");
    assert_eq!(
        recorder
            .record_platform_binding_quiescence(local)
            .expect_err("one retained prefix cannot repeat a lane closure"),
        BackendIngressError::DuplicateBindingQuiescence { binding: local },
    );
}
