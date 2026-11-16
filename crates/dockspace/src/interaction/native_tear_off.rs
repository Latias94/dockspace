//! Stable correlation between a native drag and its host-owned provisional resources.

use crate::input::journal::PointerStreamId;

use super::DragSessionId;

/// Core-issued identity of one native drag and its provisional host resources.
///
/// The identity includes the session, pointer provider, pointer stream, and drag
/// incarnation. It grants no hover, capture, placement, or presentation facts.
/// Hosts may use it as a key for an unregistered provisional window, then adopt
/// that resource only when a committed native create request returns this key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeDragTearOff {
    stream: PointerStreamId,
    session: DragSessionId,
}

impl NativeDragTearOff {
    pub(crate) const fn new(stream: PointerStreamId, session: DragSessionId) -> Self {
        Self { stream, session }
    }

    pub(crate) const fn stream(self) -> PointerStreamId {
        self.stream
    }

    pub(crate) const fn session(self) -> DragSessionId {
        self.session
    }
}

impl std::fmt::Debug for NativeDragTearOff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NativeDragTearOff(..)")
    }
}
