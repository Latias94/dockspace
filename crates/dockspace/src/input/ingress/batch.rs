//! Retryable ingress prefixes and commit-time liveness validation.

use super::*;

/// Commit-time proof that every record reduced by one prepared frame still
/// belongs to the recorder branch which produced it.
#[derive(Debug)]
pub(crate) struct BackendIngressCommitGuard {
    coordination: BackendIngressCoordination,
    pub(super) records: Vec<(BackendIngressOrdinal, BackendIngressRecordLiveness)>,
}

impl BackendIngressCommitGuard {
    /// Returns the owned record buffer capacity without expanding shared liveness.
    #[must_use]
    pub(crate) fn owned_capacity_bytes(&self) -> usize {
        self.records.capacity() * size_of::<(BackendIngressOrdinal, BackendIngressRecordLiveness)>()
    }

    pub(crate) fn validate(&self) -> Result<(), BackendIngressError> {
        let _coordination = self.coordination.lock();
        self.validate_locked()
    }

    pub(crate) fn publish_with<T, E>(
        self,
        publish: impl FnOnce() -> Result<T, E>,
    ) -> Result<Result<T, E>, BackendIngressError> {
        let _coordination = self.coordination.lock();
        self.validate_locked()?;
        for (_, liveness) in &self.records {
            liveness.begin_publication();
        }
        match publish() {
            Ok(value) => {
                for (_, liveness) in &self.records {
                    liveness.commit_publication();
                }
                Ok(Ok(value))
            }
            Err(error) => {
                for (_, liveness) in &self.records {
                    liveness.abort_publication();
                }
                Ok(Err(error))
            }
        }
    }

    fn validate_locked(&self) -> Result<(), BackendIngressError> {
        for (ordinal, liveness) in &self.records {
            match liveness.state() {
                BackendIngressRecordState::Live => {}
                BackendIngressRecordState::Publishing => {
                    return Err(BackendIngressError::RecordPublicationInProgress {
                        ordinal: *ordinal,
                    });
                }
                BackendIngressRecordState::Committed => {
                    return Err(BackendIngressError::RecordAlreadyCommitted { ordinal: *ordinal });
                }
                BackendIngressRecordState::Revoked => {
                    return Err(BackendIngressError::RecordRevoked { ordinal: *ordinal });
                }
            }
        }
        Ok(())
    }
}

/// Immutable complete interval of one backend ingress stream.
///
/// Construction is private and validates exact lease equality plus a contiguous
/// `(previous, through]` ordinal interval. The value is cloneable and contains
/// no consumptive cursor, so a failed host-frame attempt may retry the same batch.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendIngressBatch {
    pub(super) coordination: BackendIngressCoordination,
    pub(super) lease: BackendIngressLease,
    pub(super) previous: BackendIngressOrdinal,
    pub(super) previous_record_identity: BackendIngressRecordIdentity,
    pub(super) through: BackendIngressOrdinal,
    pub(super) records: Vec<BackendIngressRecord>,
}

impl BackendIngressBatch {
    pub(super) fn from_records(
        coordination: BackendIngressCoordination,
        lease: BackendIngressLease,
        previous: BackendIngressOrdinal,
        previous_record_identity: BackendIngressRecordIdentity,
        through: BackendIngressOrdinal,
        records: Vec<BackendIngressRecord>,
    ) -> Result<Self, BackendIngressError> {
        let batch = Self {
            coordination,
            lease,
            previous,
            previous_record_identity,
            through,
            records,
        };
        batch.validate_shape()?;
        Ok(batch)
    }

    /// Returns the exact joined provider lifetime which captured this batch.
    #[must_use]
    #[cfg(test)]
    pub const fn lease(&self) -> BackendIngressLease {
        self.lease
    }

    /// Returns the exclusive lower ingress watermark.
    #[must_use]
    #[cfg(test)]
    pub const fn previous(&self) -> BackendIngressOrdinal {
        self.previous
    }

    /// Returns the inclusive upper ingress watermark.
    #[must_use]
    pub const fn through(&self) -> BackendIngressOrdinal {
        self.through
    }

    /// Returns records in immutable backend capture order.
    #[must_use]
    pub fn records(&self) -> &[BackendIngressRecord] {
        &self.records
    }

    /// Returns the record buffer capacity without expanding shared record data.
    #[must_use]
    pub(crate) fn record_buffer_capacity_bytes(&self) -> usize {
        self.records.capacity() * size_of::<BackendIngressRecord>()
    }

    /// Returns the number of captured records in this interval.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns whether this interval advances no ingress facts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(crate) fn into_prefix(
        mut self,
        completed_records: usize,
    ) -> Result<Self, BackendIngressError> {
        debug_assert!(completed_records <= self.records.len());
        self.records.truncate(completed_records);
        self.through = self
            .records
            .last()
            .map_or(self.previous, |record| record.ordinal);
        self.validate_commit_boundary()?;
        Ok(self)
    }

    /// Validates this batch against the engine-owned provider and committed watermark.
    pub(super) fn validate_against(
        &self,
        expected_lease: BackendIngressLease,
        committed_through: BackendIngressOrdinal,
        committed_record_identity: BackendIngressRecordIdentity,
    ) -> Result<(), BackendIngressError> {
        if self.lease != expected_lease {
            return Err(BackendIngressError::BatchLeaseMismatch {
                expected: expected_lease,
                submitted: self.lease,
            });
        }
        if self.previous != committed_through {
            return Err(BackendIngressError::BatchPreviousMismatch {
                expected: committed_through,
                submitted: self.previous,
            });
        }
        if self.previous_record_identity != committed_record_identity {
            return Err(BackendIngressError::BatchPreviousIdentityMismatch {
                ordinal: committed_through,
            });
        }
        self.validate_shape()?;
        self.validate_publishable()
    }

    fn validate_shape(&self) -> Result<(), BackendIngressError> {
        let mut cursor = self.previous;
        for record in &self.records {
            if record.lease != self.lease {
                return Err(BackendIngressError::RecordLeaseMismatch {
                    ordinal: record.ordinal,
                    expected: self.lease,
                    submitted: record.lease,
                });
            }
            let expected = cursor
                .checked_next()
                .ok_or(BackendIngressError::OrdinalExhausted)?;
            if record.ordinal != expected {
                return Err(BackendIngressError::RecordOrdinalGap {
                    expected,
                    submitted: record.ordinal,
                });
            }
            cursor = record.ordinal;
        }
        if cursor != self.through {
            return Err(BackendIngressError::BatchThroughMismatch {
                expected: cursor,
                submitted: self.through,
            });
        }
        self.validate_commit_boundary()
    }

    fn validate_commit_boundary(&self) -> Result<(), BackendIngressError> {
        if self
            .records
            .last()
            .is_some_and(|record| !record.is_commit_boundary())
        {
            return Err(BackendIngressError::AtomicGroupIncomplete {
                through: self.through,
            });
        }
        Ok(())
    }

    fn validate_publishable(&self) -> Result<(), BackendIngressError> {
        for record in &self.records {
            match record.liveness.state() {
                BackendIngressRecordState::Live => {}
                BackendIngressRecordState::Publishing => {
                    return Err(BackendIngressError::RecordPublicationInProgress {
                        ordinal: record.ordinal,
                    });
                }
                BackendIngressRecordState::Committed => {
                    return Err(BackendIngressError::RecordAlreadyCommitted {
                        ordinal: record.ordinal,
                    });
                }
                BackendIngressRecordState::Revoked => {
                    return Err(BackendIngressError::RecordRevoked {
                        ordinal: record.ordinal,
                    });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn commit_guard(&self) -> BackendIngressCommitGuard {
        let mut records = Vec::with_capacity(self.records.len());
        records.extend(
            self.records
                .iter()
                .map(|record| (record.ordinal, record.liveness.clone())),
        );
        BackendIngressCommitGuard {
            coordination: self.coordination.clone(),
            records,
        }
    }
}
