//! Prepaid backing for stores whose owners enforce exact slot admission.

use std::collections::VecDeque;

use crate::error::NativeHostProtocolError;

const UNAVAILABLE: NativeHostProtocolError =
    NativeHostProtocolError::NativeReportStorageUnavailable;

pub(crate) struct BoundedVec<T>(Vec<T>);

impl<T> BoundedVec<T> {
    pub(crate) fn with_slots(slots: Option<usize>) -> Result<Self, NativeHostProtocolError> {
        let mut storage = Vec::new();
        if let Some(slots) = slots {
            Self::reserve(&mut storage, slots)?;
        }
        Ok(Self(storage))
    }

    pub(crate) fn reserve(
        storage: &mut Vec<T>,
        slots: usize,
    ) -> Result<(), NativeHostProtocolError> {
        if storage.capacity() > slots {
            return Err(UNAVAILABLE);
        }
        storage
            .try_reserve_exact(slots - storage.len())
            .map_err(|_| UNAVAILABLE)?;
        (storage.capacity() <= slots)
            .then_some(())
            .ok_or(UNAVAILABLE)
    }

    pub(crate) fn into_inner(self) -> Vec<T> {
        self.0
    }
}

pub(crate) struct BoundedVecDeque<T>(VecDeque<T>);

impl<T> BoundedVecDeque<T> {
    pub(crate) fn with_slots(slots: Option<usize>) -> Result<Self, NativeHostProtocolError> {
        let mut storage = VecDeque::new();
        if let Some(slots) = slots {
            storage.try_reserve_exact(slots).map_err(|_| UNAVAILABLE)?;
            if storage.capacity() > slots {
                return Err(UNAVAILABLE);
            }
        }
        Ok(Self(storage))
    }

    pub(crate) fn into_inner(self) -> VecDeque<T> {
        self.0
    }
}
