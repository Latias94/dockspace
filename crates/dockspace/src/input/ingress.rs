//! Backend-captured total ordering across platform and pointer ingress lanes.
//!
//! This module owns only transport structure. It does not reduce records or
//! advance engine authority. A backend recorder binds one exact platform
//! provider incarnation to one exact joined native pointer provider and mints
//! opaque ordinals as facts are captured. Immutable batches can then be retried
//! until a host-frame commit advances the engine-owned watermark.

mod native_capacity;

mod authority;
mod batch;
mod recorder;

pub(crate) use authority::BackendIngressAuthority;
pub use batch::BackendIngressBatch;
pub(crate) use batch::BackendIngressCommitGuard;
pub use recorder::BackendIngressRecorder;

use std::mem::size_of;
use std::num::NonZeroUsize;
#[cfg(test)]
use std::sync::Weak;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use crate::engine::EngineInput;
use crate::engine::surface_recovery::{SurfaceRecoveryBootstrap, SurfaceRecoveryTarget};
use crate::engine::transition::WorkspaceVersion;
use crate::ids::SurfaceId;
use crate::ids::{EngineAuthorityDomainId, WorkspaceEpoch};
use crate::input::journal::{
    PointerEdgeJournal, PointerEdgeSequence, PointerInputLease, PointerProviderScope,
};
use crate::platform::effect::EffectResult;
use crate::platform::provider::PlatformObservationLease;
#[cfg(test)]
use crate::platform::provider::PlatformProviderReservation;
use crate::platform::viewport::{ViewportBinding, ViewportRole, WindowToken};
use crate::platform::{PlatformSnapshot, WindowCloseObservation};
use crate::presentation::observation::{HostPresentationObservationEntry, PresentationHostLease};

/// Opaque capture position in one backend ingress provider lifetime.
///
/// Ordinals are allocated only by [`BackendIngressRecorder`]. Callers may
/// retain and compare them, but cannot construct or advance them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct BackendIngressOrdinal(u64);

impl BackendIngressOrdinal {
    pub(crate) const ORIGIN: Self = Self(0);

    #[cfg(test)]
    pub(crate) const fn from_committed_source_sequence(value: u64) -> Self {
        Self(value)
    }

    const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the diagnostic protocol representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Non-reused identity of one recorder append, independent of rollback ordinals.
///
/// Rollback may reuse a public capture position, but it must never make a
/// different payload indistinguishable from the immutable batch which core
/// already committed at that position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BackendIngressRecordIdentity(u64);

impl BackendIngressRecordIdentity {
    const ORIGIN: Self = Self(0);

    const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Single-writer synchronization shared by one recorder branch and every
/// frozen batch derived from it. The lock spans the final engine swap so a
/// concurrent rollback cannot revoke a record between validation and publish.
#[derive(Clone, Debug)]
struct BackendIngressCoordination(Arc<Mutex<()>>);

impl BackendIngressCoordination {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(())))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl PartialEq for BackendIngressCoordination {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BackendIngressCoordination {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum BackendIngressRecordState {
    Live = 0,
    Publishing = 1,
    Committed = 2,
    Revoked = 3,
}

impl BackendIngressRecordState {
    fn load(state: &AtomicU8) -> Self {
        match state.load(Ordering::Acquire) {
            0 => Self::Live,
            1 => Self::Publishing,
            2 => Self::Committed,
            3 => Self::Revoked,
            value => unreachable!("invalid backend ingress record state {value}"),
        }
    }
}

/// Shared publication state for one exact recorder append.
///
/// Immutable batches and document receipts retain the same allocation. The
/// recorder and final host publication therefore agree on one linearized
/// `Live -> Publishing -> Committed` or `Live -> Revoked` transition even when
/// public ordinals are later reused by another append branch.
#[derive(Clone, Debug)]
struct BackendIngressRecordLiveness(Arc<AtomicU8>);

impl BackendIngressRecordLiveness {
    fn new() -> Self {
        Self(Arc::new(AtomicU8::new(
            BackendIngressRecordState::Live as u8,
        )))
    }

    fn state(&self) -> BackendIngressRecordState {
        BackendIngressRecordState::load(&self.0)
    }

    #[cfg(test)]
    fn is_live(&self) -> bool {
        self.state() == BackendIngressRecordState::Live
    }

    fn begin_publication(&self) {
        debug_assert_eq!(self.state(), BackendIngressRecordState::Live);
        self.0.store(
            BackendIngressRecordState::Publishing as u8,
            Ordering::Release,
        );
    }

    fn commit_publication(&self) {
        debug_assert_eq!(self.state(), BackendIngressRecordState::Publishing);
        self.0.store(
            BackendIngressRecordState::Committed as u8,
            Ordering::Release,
        );
    }

    fn abort_publication(&self) {
        debug_assert_eq!(self.state(), BackendIngressRecordState::Publishing);
        self.0
            .store(BackendIngressRecordState::Live as u8, Ordering::Release);
    }

    fn revoke_live(&self) {
        debug_assert_eq!(self.state(), BackendIngressRecordState::Live);
        self.0
            .store(BackendIngressRecordState::Revoked as u8, Ordering::Release);
    }
}

impl PartialEq for BackendIngressRecordLiveness {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BackendIngressRecordLiveness {}

/// Test proof of one exact recorder append across savepoint rollback.
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct BackendIngressRecordReceipt {
    lease: BackendIngressLease,
    ordinal: BackendIngressOrdinal,
    identity: BackendIngressRecordIdentity,
    liveness: BackendIngressRecordLiveness,
}

#[cfg(test)]
impl BackendIngressRecordReceipt {
    pub(crate) const fn ordinal(&self) -> BackendIngressOrdinal {
        self.ordinal
    }

    pub(crate) fn is_live(&self) -> bool {
        self.liveness.is_live()
    }
}

#[cfg(test)]
impl PartialEq for BackendIngressRecordReceipt {
    fn eq(&self, other: &Self) -> bool {
        self.lease == other.lease
            && self.ordinal == other.ordinal
            && self.identity == other.identity
            && self.liveness == other.liveness
    }
}

#[cfg(test)]
impl Eq for BackendIngressRecordReceipt {}

/// Exact backend lifetime joining platform and desktop-global pointer ingress.
///
/// The fields and constructor are private. A runtime may retain this lease but
/// cannot rebind either provider incarnation or move it to another engine
/// authority domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendIngressLease {
    platform: PlatformObservationLease,
    pointer: PointerInputLease,
    presentation_host: PresentationHostLease,
}

/// Affine proof that one backend producer has stopped minting ingress records.
///
/// The receipt is created only by consuming the sole non-cloneable
/// [`BackendIngressRecorder`]. Records captured after the core's committed prefix are explicitly
/// abandoned at this boundary; immutable batches already handed out remain harmless because the
/// provider replacement revokes their exact lease before a successor is activated.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a drained backend producer must be retired or replaced"]
#[cfg(test)]
pub struct BackendIngressDrainReceipt {
    lease: BackendIngressLease,
    recorded_through: BackendIngressOrdinal,
    pointer_through: PointerEdgeSequence,
    consumed: bool,
}

#[cfg(test)]
impl BackendIngressDrainReceipt {
    /// Returns the exact joined provider lifetime whose producer stopped.
    #[must_use]
    pub const fn lease(&self) -> BackendIngressLease {
        self.lease
    }

    /// Returns the final backend ordinal minted before producer shutdown.
    #[must_use]
    pub const fn recorded_through(&self) -> BackendIngressOrdinal {
        self.recorded_through
    }

    /// Returns the final physical pointer sequence observed by the predecessor producer.
    #[must_use]
    pub const fn pointer_through(&self) -> PointerEdgeSequence {
        self.pointer_through
    }

    /// Returns whether this affine proof was transferred into a replacement ticket.
    #[must_use]
    pub const fn is_consumed(&self) -> bool {
        self.consumed
    }

    pub(crate) const fn validate_active(&self) -> Result<(), BackendIngressError> {
        if self.consumed {
            Err(BackendIngressError::DrainReceiptConsumed)
        } else {
            Ok(())
        }
    }

    pub(crate) fn transfer(&mut self) -> Result<Self, BackendIngressError> {
        self.validate_active()?;
        self.consumed = true;
        Ok(Self {
            lease: self.lease,
            recorded_through: self.recorded_through,
            pointer_through: self.pointer_through,
            consumed: false,
        })
    }

    /// Reconstructs the core-owned copy of a proof after the original adapter
    /// ticket was dropped. This is safe only for metadata already persisted in
    /// [`BackendIngressAuthority::pending_replacement`]; callers cannot supply
    /// this constructor data through the public API.
    fn from_replacement_state(
        lease: BackendIngressLease,
        recorded_through: BackendIngressOrdinal,
        pointer_through: PointerEdgeSequence,
    ) -> Self {
        Self {
            lease,
            recorded_through,
            pointer_through,
            consumed: false,
        }
    }
}

/// Affine proof that one recorder prefix has been reclaimed after core commit.
///
/// Binding quiescence facts inside this prefix are safe to apply only while the
/// engine remains at the receipt's opaque commit boundary. The receipt remains reusable
/// after a failed settlement and becomes consumed only after the engine publishes
/// every associated retention update atomically. Prefixes without binding
/// quiescence remain allocation-free and do not require engine settlement.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a reclaimed backend prefix must be settled with its core authority"]
pub struct BackendIngressPrefixRetirementReceipt {
    authority: Option<BackendIngressPrefixRetirementAuthority>,
}

#[derive(Debug, PartialEq, Eq)]
struct BackendIngressPrefixRetirementAuthority {
    lease: BackendIngressLease,
    through: BackendIngressOrdinal,
    binding_quiescences: Vec<ViewportBinding>,
}

impl BackendIngressPrefixRetirementReceipt {
    fn new(
        lease: BackendIngressLease,
        through: BackendIngressOrdinal,
        binding_quiescences: Vec<ViewportBinding>,
    ) -> Self {
        Self {
            authority: Some(BackendIngressPrefixRetirementAuthority {
                lease,
                through,
                binding_quiescences,
            }),
        }
    }

    pub(crate) const fn lease(&self) -> Option<BackendIngressLease> {
        match &self.authority {
            Some(authority) => Some(authority.lease),
            None => None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn through(&self) -> Option<BackendIngressOrdinal> {
        match &self.authority {
            Some(authority) => Some(authority.through),
            None => None,
        }
    }

    pub(crate) fn binding_quiescences(&self) -> &[ViewportBinding] {
        self.authority
            .as_ref()
            .map_or(&[], |authority| &authority.binding_quiescences)
    }

    /// Returns the owned binding buffer capacity, excluding inline receipt data.
    #[must_use]
    pub(crate) fn owned_capacity_bytes(&self) -> usize {
        self.authority.as_ref().map_or(0, |authority| {
            authority.binding_quiescences.capacity() * size_of::<ViewportBinding>()
        })
    }

    fn authority(&self) -> Result<&BackendIngressPrefixRetirementAuthority, BackendIngressError> {
        self.authority
            .as_ref()
            .ok_or(BackendIngressError::PrefixRetirementReceiptConsumed)
    }

    pub(crate) fn consume(&mut self) -> Result<Vec<ViewportBinding>, BackendIngressError> {
        self.authority
            .take()
            .map(|authority| authority.binding_quiescences)
            .ok_or(BackendIngressError::PrefixRetirementReceiptConsumed)
    }
}

/// Affine authority to finish one exact joined backend-provider handoff.
///
/// This type deliberately does not implement `Clone` or `Copy`, and it never
/// exposes the core-owned platform reservation. A joined replacement can therefore
/// activate the platform, desktop-pointer, and backend-order lanes only through
/// the engine-owned joined replacement path.
#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
pub struct BackendIngressProviderReplacementTicket {
    authority: Option<BackendIngressProviderReplacementAuthority>,
}

#[derive(Debug)]
#[cfg(test)]
struct BackendIngressProviderReplacementAuthority {
    authority_domain: EngineAuthorityDomainId,
    handoff: BackendIngressReplacementId,
    generation: u64,
    _monitor: Arc<()>,
}

#[cfg(test)]
impl PartialEq for BackendIngressProviderReplacementAuthority {
    fn eq(&self, other: &Self) -> bool {
        self.authority_domain == other.authority_domain
            && self.handoff == other.handoff
            && self.generation == other.generation
            && Arc::ptr_eq(&self._monitor, &other._monitor)
    }
}

#[cfg(test)]
impl Eq for BackendIngressProviderReplacementAuthority {}

/// Core-owned identity of one pending joined-provider handoff.
///
/// The identity is deliberately separate from the platform ticket. A dropped
/// adapter ticket does not discard the quiescence proof retained by core, and a
/// later reissued ticket still has to name this exact pending handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg(test)]
pub(crate) struct BackendIngressReplacementId(u64);

#[cfg(test)]
impl BackendIngressReplacementId {
    const ORIGIN: Self = Self(0);

    const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[cfg(test)]
impl BackendIngressProviderReplacementTicket {
    pub(crate) const fn new(
        authority_domain: EngineAuthorityDomainId,
        handoff: BackendIngressReplacementId,
        generation: u64,
        monitor: Arc<()>,
    ) -> Self {
        Self {
            authority: Some(BackendIngressProviderReplacementAuthority {
                authority_domain,
                handoff,
                generation,
                _monitor: monitor,
            }),
        }
    }

    pub(crate) fn authority(
        &self,
    ) -> Option<(
        EngineAuthorityDomainId,
        BackendIngressReplacementId,
        u64,
        &Arc<()>,
    )> {
        match &self.authority {
            Some(authority) => Some((
                authority.authority_domain,
                authority.handoff,
                authority.generation,
                &authority._monitor,
            )),
            None => None,
        }
    }

    pub(crate) fn consume(&mut self) -> bool {
        self.authority.take().is_some()
    }

    /// Returns whether this handoff authority was already committed.
    #[must_use]
    pub const fn is_consumed(&self) -> bool {
        self.authority.is_none()
    }
}

/// Core-minted proof that one exact joined provider committed an ingress prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendIngressCommitWatermark {
    lease: BackendIngressLease,
    through: BackendIngressOrdinal,
    record_identity: BackendIngressRecordIdentity,
}

impl BackendIngressCommitWatermark {
    const fn new(
        lease: BackendIngressLease,
        through: BackendIngressOrdinal,
        record_identity: BackendIngressRecordIdentity,
    ) -> Self {
        Self {
            lease,
            through,
            record_identity,
        }
    }

    /// Returns the inclusive committed ingress position.
    #[must_use]
    #[cfg(test)]
    pub const fn through(self) -> BackendIngressOrdinal {
        self.through
    }
}

impl BackendIngressLease {
    pub(crate) fn new(
        platform: PlatformObservationLease,
        pointer: PointerInputLease,
        presentation_host: PresentationHostLease,
    ) -> Result<Self, BackendIngressError> {
        let platform_domain = platform.authority_domain();
        let pointer_domain = pointer.authority_domain();
        if platform_domain != pointer_domain {
            return Err(BackendIngressError::AuthorityDomainMismatch {
                platform: platform_domain,
                pointer: pointer_domain,
            });
        }
        let presentation_domain = presentation_host.authority_domain();
        if presentation_domain != platform_domain {
            return Err(BackendIngressError::PresentationAuthorityDomainMismatch {
                backend: platform_domain,
                presentation: presentation_domain,
            });
        }
        let PointerProviderScope::NativeJoined(pointer_host) = pointer.scope() else {
            return Err(BackendIngressError::PointerProviderNotNativeJoined { provider: pointer });
        };
        if pointer_host != presentation_host {
            return Err(BackendIngressError::PresentationHostLeaseMismatch {
                expected: pointer_host,
                submitted: presentation_host,
            });
        }
        Ok(Self {
            platform,
            pointer,
            presentation_host,
        })
    }

    /// Returns the engine authority domain shared by both provider incarnations.
    #[must_use]
    pub const fn authority_domain(self) -> EngineAuthorityDomainId {
        self.platform.authority_domain()
    }

    /// Returns the platform-provider incarnation for diagnostics.
    #[must_use]
    pub const fn platform_incarnation(self) -> u64 {
        self.platform.incarnation()
    }

    /// Returns the pointer-provider incarnation for diagnostics.
    #[must_use]
    pub const fn pointer_incarnation(self) -> u64 {
        self.pointer.incarnation()
    }

    /// Returns the exact presentation host bound to this backend lifetime.
    #[must_use]
    pub const fn presentation_host(self) -> PresentationHostLease {
        self.presentation_host
    }

    pub(crate) const fn platform_provider(self) -> PlatformObservationLease {
        self.platform
    }

    pub(crate) const fn pointer_provider(self) -> PointerInputLease {
        self.pointer
    }
}

/// Payload captured at one exact backend ingress ordinal.
///
/// Lane-local generations remain part of each payload. The enclosing ingress
/// ordinal adds only the total order between those independent lanes.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendIngressPayload {
    /// One complete atomic platform observation batch.
    PlatformSnapshot {
        /// Workspace epoch whose binding roster the backend observed.
        expected_epoch: WorkspaceEpoch,
        /// Complete platform fact envelope captured at this ingress position.
        snapshot: PlatformSnapshot,
    },
    /// The producer has permanently closed every ingress lane for one exact binding.
    ///
    /// This record does not itself release core retention. Reclamation becomes
    /// authoritative only when the recorder retires a core-committed prefix and
    /// returns a [`BackendIngressPrefixRetirementReceipt`].
    PlatformBindingQuiesced {
        /// Exact core-minted binding which no future producer fact may name.
        binding: ViewportBinding,
    },
    /// One exact binding-scoped native-close observation.
    NativeCloseObservation {
        /// Workspace epoch whose binding incarnation the backend observed.
        expected_epoch: WorkspaceEpoch,
        /// Exact close fact captured at this ingress position.
        observation: WindowCloseObservation,
    },
    /// One globally consistent native-focus observation.
    GlobalFocusObservation {
        /// Workspace epoch whose binding incarnation the backend observed.
        expected_epoch: WorkspaceEpoch,
        /// Provider-owned focus fact captured at this ingress position.
        observation: crate::platform::focus::FocusObservationEnvelope,
    },
    /// One non-observational platform-effect dispatch result.
    PlatformEffectResult(EffectResult),
    /// Removal of native input devices resets this provider's live input streams.
    NativeInputDeviceRemoval,
    /// One exact native binding can no longer deliver input or provider tails.
    NativeInputBindingRetired {
        /// The complete binding incarnation whose dependent live input ends.
        binding: ViewportBinding,
    },
    /// One narrow non-pointer semantic action captured by the backend event loop.
    SemanticInput(Box<EngineInput>),
    /// One presentation-stream fact captured at this exact backend position.
    PresentationObservation {
        /// Core-minted host bound when this backend provider was enrolled.
        host: PresentationHostLease,
        /// One stream observation; a no-update entry still consumes this ordinal.
        entry: HostPresentationObservationEntry,
    },
    /// One edgewise pointer journal segment, including an optional authority checkpoint.
    PointerSegment(PointerEdgeJournal),
}

/// One immutable provider-bound record in a backend ingress stream.
///
/// The recorder and every frozen copy share one payload allocation, including
/// any vectors owned by its platform snapshot or pointer journal.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendIngressRecord {
    lease: BackendIngressLease,
    ordinal: BackendIngressOrdinal,
    identity: BackendIngressRecordIdentity,
    payload: Arc<BackendIngressPayload>,
    liveness: BackendIngressRecordLiveness,
    commit_boundary_after: bool,
}

impl BackendIngressRecord {
    /// Returns this record's opaque capture position.
    #[must_use]
    pub const fn ordinal(&self) -> BackendIngressOrdinal {
        self.ordinal
    }

    /// Returns the typed fact captured at this position.
    #[must_use]
    pub fn payload(&self) -> &BackendIngressPayload {
        &self.payload
    }

    /// Returns whether a host frame may publish a prefix ending at this record.
    ///
    /// Records inside one causally indivisible append group can be reduced
    /// cooperatively, but only the group's final record is a commit boundary.
    #[must_use]
    pub const fn is_commit_boundary(&self) -> bool {
        self.commit_boundary_after
    }
}

/// Structural rejection while capturing or validating backend ingress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BackendIngressError {
    /// A native recorder cannot retain another record within its configured budget.
    #[error("native transport requires capacity {required}, exceeding limit {limit}")]
    NativeTransportCapacityExceeded {
        /// Configured record-slot or byte limit for the rejected resource.
        limit: usize,
        /// Required record slots or bytes, saturated when the calculation overflows.
        required: usize,
    },
    /// A payload is outside the native facade's finite transport contract.
    #[error("backend ingress payload has no supported native capacity accounting")]
    UnsupportedNativePayloadCapacity,
    /// A fixed transport buffer could not be allocated.
    #[error("backend ingress transport capacity allocation failed")]
    CapacityAllocationFailed,
    /// A proposed prefix would publish only part of an indivisible append group.
    #[error("backend ingress prefix through {through:?} ends inside an atomic group")]
    AtomicGroupIncomplete {
        /// Last record of the rejected prefix.
        through: BackendIngressOrdinal,
    },
    /// No complete atomic group fits in a requested nonempty prefix.
    #[error("backend ingress atomic group at {first:?} exceeds prefix limit {max_records}")]
    AtomicGroupExceedsPrefixLimit {
        /// First record of the pending atomic group.
        first: BackendIngressOrdinal,
        /// Maximum number of records permitted in the selection.
        max_records: NonZeroUsize,
    },
    /// An internal append group was finalized after its records ceased to be live.
    #[error("backend ingress atomic append record {ordinal:?} is no longer live")]
    AtomicAppendNotLive {
        /// Exact record which can no longer change its commit grouping.
        ordinal: BackendIngressOrdinal,
    },
    /// A joined replacement attempted to reuse an already transferred drain proof.
    #[error("backend ingress drain receipt was already consumed")]
    DrainReceiptConsumed,
    /// One affine prefix-retirement proof was submitted after successful settlement.
    #[error("backend ingress prefix-retirement receipt was already consumed")]
    PrefixRetirementReceiptConsumed,
    /// A successfully committed provider handoff ticket was submitted again.
    #[error("backend ingress provider replacement ticket was already consumed")]
    ProviderReplacementTicketConsumed,
    /// A joined provider replacement is already pending and must be finished,
    /// reissued, or explicitly reaped before another handoff can begin.
    #[error("backend ingress provider replacement is already pending")]
    ProviderReplacementPending,
    /// No joined provider replacement is currently pending.
    #[error("backend ingress provider replacement is not pending")]
    ProviderReplacementNotPending,
    /// A replacement ticket does not name the exact core-owned handoff.
    #[error("backend ingress provider replacement ticket is not the pending handoff")]
    UnknownReplacementTicket,
    /// The core-owned replacement identity counter cannot advance.
    #[error("backend ingress provider replacement identity counter is exhausted")]
    ReplacementIdentityExhausted,
    /// The ticket generation for one pending handoff cannot advance.
    #[error("backend ingress provider replacement ticket generation is exhausted")]
    ReplacementTicketGenerationExhausted,
    /// Another joined backend provider is already active.
    #[error("backend ingress provider {active:?} is already active")]
    ProviderAlreadyActive {
        /// Exact currently active provider pair.
        active: BackendIngressLease,
    },
    /// No joined backend provider currently owns ingress authority.
    #[error("backend ingress provider is unavailable")]
    ProviderUnavailable,
    /// A binding-quiescence record belongs to another engine authority domain.
    #[error("binding quiescence belongs to authority domain {submitted:?}, expected {expected:?}")]
    BindingQuiescenceAuthorityMismatch {
        /// Engine authority owned by the backend recorder.
        expected: EngineAuthorityDomainId,
        /// Engine authority carried by the binding.
        submitted: EngineAuthorityDomainId,
    },
    /// One unretired recorder prefix repeated a binding-quiescence assertion.
    #[error("backend ingress already recorded quiescence for binding {binding:?}")]
    DuplicateBindingQuiescence {
        /// Exact binding repeated by the producer.
        binding: ViewportBinding,
    },
    /// An operation named a provider other than the exact active pair.
    #[error("backend ingress provider {submitted:?} does not match active provider {expected:?}")]
    ProviderLeaseMismatch {
        /// Exact currently active provider pair.
        expected: BackendIngressLease,
        /// Provider pair supplied by the caller.
        submitted: BackendIngressLease,
    },
    /// A recorder rollback boundary belongs to another joined provider.
    #[error("backend ingress savepoint lease {submitted:?} does not match {expected:?}")]
    SavepointLeaseMismatch {
        /// Recorder-owned exact provider pair.
        expected: BackendIngressLease,
        /// Provider pair carried by the savepoint.
        submitted: BackendIngressLease,
    },
    /// A recorder prefix was reclaimed after the rollback boundary was minted.
    #[error("backend ingress savepoint prefix {saved:?} was reclaimed through {current:?}")]
    SavepointPrefixReclaimed {
        /// Reclaimed watermark captured by the savepoint.
        saved: BackendIngressOrdinal,
        /// Recorder's current reclaimed watermark.
        current: BackendIngressOrdinal,
    },
    /// Recorder state no longer contains the savepoint's complete prefix.
    #[error("backend ingress savepoint {saved:?} is ahead of current recorder state {current:?}")]
    SavepointAhead {
        /// Last ordinal captured by the savepoint.
        saved: BackendIngressOrdinal,
        /// Recorder's current last ordinal.
        current: BackendIngressOrdinal,
    },
    /// Public ordinals were reused after an outer rollback, so this savepoint no longer
    /// authenticates the exact recorder prefix it originally observed.
    #[error("backend ingress savepoint prefix through {boundary:?} was rewritten")]
    SavepointPrefixRewritten {
        /// Public ordinal at the stale savepoint boundary.
        boundary: BackendIngressOrdinal,
    },
    /// A presentation record named a host other than the one bound to this backend lifetime.
    #[error("backend presentation host {submitted:?} does not match bound host {expected:?}")]
    PresentationHostLeaseMismatch {
        /// Host frozen when the joined backend provider was enrolled.
        expected: PresentationHostLease,
        /// Host carried by the rejected record.
        submitted: PresentationHostLease,
    },
    /// The joined provider pair belongs to another engine authority domain.
    #[error("backend ingress belongs to authority domain {submitted:?}, expected {expected:?}")]
    ForeignAuthorityDomain {
        /// Engine authority which owns this ledger.
        expected: EngineAuthorityDomainId,
        /// Authority embedded in the submitted pair.
        submitted: EngineAuthorityDomainId,
    },
    /// The paired providers belong to different engine authority domains.
    #[error("platform ingress belongs to {platform:?}, but pointer ingress belongs to {pointer:?}")]
    AuthorityDomainMismatch {
        /// Platform provider authority domain.
        platform: EngineAuthorityDomainId,
        /// Pointer provider authority domain.
        pointer: EngineAuthorityDomainId,
    },
    /// The bound presentation host belongs to another engine authority domain.
    #[error(
        "presentation host belongs to {presentation:?}, but backend ingress belongs to {backend:?}"
    )]
    PresentationAuthorityDomainMismatch {
        /// Authority domain carried by the joined platform and pointer providers.
        backend: EngineAuthorityDomainId,
        /// Authority domain carried by the presentation host.
        presentation: EngineAuthorityDomainId,
    },
    /// Unified backend ingress requires a joined native pointer provider.
    #[error("backend ingress pointer provider {provider:?} is not native-joined")]
    PointerProviderNotNativeJoined {
        /// Rejected exact pointer-provider incarnation.
        provider: PointerInputLease,
    },
    /// The backend capture ordinal cannot advance without wrapping.
    #[error("backend ingress ordinal is exhausted")]
    OrdinalExhausted,
    /// The non-reused append identity cannot advance without wrapping.
    #[error("backend ingress record identity is exhausted")]
    RecordIdentityExhausted,
    /// Recorder rollback or drain revoked a record retained by an older batch.
    #[error("backend ingress record {ordinal:?} was revoked before core publication")]
    RecordRevoked {
        /// Exact provider-local position whose append branch was abandoned.
        ordinal: BackendIngressOrdinal,
    },
    /// Another publication currently owns the exact record under the recorder lock.
    #[error("backend ingress record {ordinal:?} is already being published")]
    RecordPublicationInProgress {
        /// Exact provider-local position whose publication has started.
        ordinal: BackendIngressOrdinal,
    },
    /// The record was already published by a successful host-frame commit.
    #[error("backend ingress record {ordinal:?} was already committed")]
    RecordAlreadyCommitted {
        /// Exact provider-local position already owned by the committed engine.
        ordinal: BackendIngressOrdinal,
    },
    /// Recorder rollback cannot erase input already published to the engine.
    #[error("backend rollback would cross published record {ordinal:?}")]
    RollbackCrossesPublishedRecord {
        /// First suffix record whose publication started or completed.
        ordinal: BackendIngressOrdinal,
    },
    /// A pointer segment contained more than one receiver-bearing edge.
    #[error("backend ingress pointer segment contains {edges} edges; at most one is allowed")]
    PointerSegmentMustBeEdgewise {
        /// Number of edges in the rejected segment.
        edges: usize,
    },
    /// A pointer segment did not continue from the recorder's exact lane watermark.
    #[error(
        "backend ingress pointer segment starts at {submitted}, expected committed watermark {expected}"
    )]
    PointerSegmentPreviousMismatch {
        /// Recorder-owned pointer watermark.
        expected: PointerEdgeSequence,
        /// Segment's exclusive lower pointer watermark.
        submitted: PointerEdgeSequence,
    },
    /// A platform snapshot or effect result was submitted through the semantic lane.
    #[error("backend platform facts must use their typed ingress lanes")]
    SemanticInputIsBackendFact,
    /// A terminal configuration update was submitted inside the causal input prefix.
    #[error("backend configuration input belongs to the terminal configuration phase")]
    SemanticInputIsConfiguration,
    /// Overload cancellation must remain available outside recorder capacity.
    #[error("terminal host control cannot enter the backend ingress recorder")]
    SemanticInputIsHostControl,
    /// A requested suffix starts after the latest captured record.
    #[error(
        "backend ingress committed watermark {committed:?} is ahead of recorded watermark {recorded:?}"
    )]
    CommittedWatermarkAhead {
        /// Watermark supplied by the caller.
        committed: BackendIngressOrdinal,
        /// Latest recorder-owned ordinal.
        recorded: BackendIngressOrdinal,
    },
    /// A requested retry prefix has already been reclaimed from this recorder.
    #[error(
        "backend ingress committed watermark {committed:?} predates reclaimed watermark {retired:?}"
    )]
    CommittedWatermarkRetired {
        /// Requested historical lower bound.
        committed: BackendIngressOrdinal,
        /// Oldest lower bound still retained by the recorder.
        retired: BackendIngressOrdinal,
    },
    /// A commit proof belongs to another joined provider incarnation.
    #[error("backend commit watermark lease {submitted:?} does not match {expected:?}")]
    CommitWatermarkLeaseMismatch {
        /// Recorder-owned exact provider pair.
        expected: BackendIngressLease,
        /// Provider pair carried by the core proof.
        submitted: BackendIngressLease,
    },
    /// Recorder contents at a committed ordinal differ from the immutable batch core accepted.
    #[error("backend commit watermark content no longer matches record {ordinal:?}")]
    CommitWatermarkContentMismatch {
        /// Public capture position whose non-reused identity changed after rollback.
        ordinal: BackendIngressOrdinal,
    },
    /// Prefix reclamation attempted to move behind an already reclaimed position.
    #[error("backend retirement watermark {submitted:?} predates reclaimed watermark {retired:?}")]
    RetirementWatermarkRegressed {
        /// Current reclaimed lower bound.
        retired: BackendIngressOrdinal,
        /// Regressive proof position.
        submitted: BackendIngressOrdinal,
    },
    /// A prefix-retirement proof belongs to another active joined provider.
    #[error("prefix-retirement lease {submitted:?} does not match active lease {expected:?}")]
    PrefixRetirementLeaseMismatch {
        /// Current core-owned backend lease.
        expected: BackendIngressLease,
        /// Recorder lease carried by the proof.
        submitted: BackendIngressLease,
    },
    /// A prefix-retirement proof does not end at the current core commit boundary.
    ///
    /// Exact equality is the causal barrier between permanent producer
    /// quiescence and guard reclamation. It prevents a later committed fact from
    /// crossing an older closure proof before that proof is settled.
    #[error(
        "prefix-retirement proof reaches {submitted:?}, but core is committed through {committed:?}"
    )]
    PrefixRetirementWatermarkMismatch {
        /// Last backend ordinal committed by a complete host frame.
        committed: BackendIngressOrdinal,
        /// Inclusive upper watermark carried by the proof.
        submitted: BackendIngressOrdinal,
    },
    /// The batch belongs to a different exact provider pair.
    #[error("backend ingress batch lease {submitted:?} does not match {expected:?}")]
    BatchLeaseMismatch {
        /// Engine-owned current lease.
        expected: BackendIngressLease,
        /// Lease carried by the batch.
        submitted: BackendIngressLease,
    },
    /// The batch does not begin at the engine's committed ingress watermark.
    #[error(
        "backend ingress batch starts at {submitted:?}, expected committed watermark {expected:?}"
    )]
    BatchPreviousMismatch {
        /// Engine-owned committed watermark.
        expected: BackendIngressOrdinal,
        /// Batch's exclusive lower watermark.
        submitted: BackendIngressOrdinal,
    },
    /// The batch follows a rewritten record at the public committed ordinal.
    #[error(
        "backend ingress batch predecessor identity does not match committed record {ordinal:?}"
    )]
    BatchPreviousIdentityMismatch {
        /// Public capture position whose predecessor identity diverged.
        ordinal: BackendIngressOrdinal,
    },
    /// One record was spliced from another provider pair.
    #[error(
        "backend ingress record {ordinal:?} carries lease {submitted:?}, expected {expected:?}"
    )]
    RecordLeaseMismatch {
        /// Position of the rejected record.
        ordinal: BackendIngressOrdinal,
        /// Batch-owned exact provider pair.
        expected: BackendIngressLease,
        /// Record-owned exact provider pair.
        submitted: BackendIngressLease,
    },
    /// Records are duplicated, missing, or reordered.
    #[error("backend ingress record ordinal {submitted:?} is not contiguous after {expected:?}")]
    RecordOrdinalGap {
        /// Sole acceptable next ordinal.
        expected: BackendIngressOrdinal,
        /// Rejected record ordinal.
        submitted: BackendIngressOrdinal,
    },
    /// The batch header does not name its last contiguous record.
    #[error("backend ingress batch through {submitted:?} does not match final record {expected:?}")]
    BatchThroughMismatch {
        /// Last contiguous record, or the lower watermark for an empty batch.
        expected: BackendIngressOrdinal,
        /// Inclusive upper watermark supplied by the batch header.
        submitted: BackendIngressOrdinal,
    },
}

#[cfg(test)]
mod bounded_prefix_tests;

#[cfg(test)]
mod tests;
