//! Monotonic gesture identities and their frozen pointer owners.

use super::*;

macro_rules! interaction_counter {
    ($constructor_visibility:vis $name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Creates a counter value from its runtime representation.
            #[must_use]
            $constructor_visibility const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the runtime representation.
            #[must_use]
            #[allow(dead_code, reason = "Internal identity macros provide inspection for protocol tests; production uses only each domain's required operations")]
            pub const fn get(self) -> u64 {
                self.0
            }

            pub(crate) const fn checked_next(self) -> Option<Self> {
                match self.0.checked_add(1) {
                    Some(value) => Some(Self::new(value)),
                    None => None,
                }
            }
        }
    };
}

interaction_counter!(pub DragGeneration, "Monotonic generation of drag sessions.");
interaction_counter!(
    pub(crate) ClickGeneration,
    "Monotonic generation of click sessions."
);
interaction_counter!(
    pub ResizeGeneration,
    "Monotonic generation of splitter-resize sessions."
);
interaction_counter!(
    pub(crate) ScrollSessionId,
    "Core-owned identity of one admitted smooth-scroll session."
);

/// Identity of one press/release click session within a workspace epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClickSessionId {
    epoch: WorkspaceEpoch,
    generation: ClickGeneration,
}

impl ClickSessionId {
    pub(crate) const fn new(epoch: WorkspaceEpoch, generation: ClickGeneration) -> Self {
        Self { epoch, generation }
    }

    /// Returns the workspace epoch in which the click was pressed.
    #[must_use]
    #[cfg(test)]
    pub const fn epoch(self) -> WorkspaceEpoch {
        self.epoch
    }
}
interaction_counter!(
    pub ContainedTransformGeneration,
    "Monotonic generation of contained transform sessions."
);
interaction_counter!(
    pub PreviewSequence,
    "Monotonic identity for previews published by one engine."
);

/// Identity of one drag session within a workspace epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DragSessionId {
    epoch: WorkspaceEpoch,
    generation: DragGeneration,
}

impl DragSessionId {
    /// Creates a typed drag identity.
    #[must_use]
    pub const fn new(epoch: WorkspaceEpoch, generation: DragGeneration) -> Self {
        Self { epoch, generation }
    }

    /// Returns the workspace epoch in which the drag was armed.
    #[must_use]
    pub const fn epoch(self) -> WorkspaceEpoch {
        self.epoch
    }

    /// Returns the drag generation.
    #[must_use]
    pub const fn generation(self) -> DragGeneration {
        self.generation
    }
}

/// Identity of one resize session within a workspace epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResizeSessionId {
    epoch: WorkspaceEpoch,
    generation: ResizeGeneration,
}

impl ResizeSessionId {
    /// Creates a typed resize identity.
    #[must_use]
    pub const fn new(epoch: WorkspaceEpoch, generation: ResizeGeneration) -> Self {
        Self { epoch, generation }
    }
}

/// Identity of one contained transform session within a workspace epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContainedTransformSessionId {
    epoch: WorkspaceEpoch,
    generation: ContainedTransformGeneration,
}

impl ContainedTransformSessionId {
    /// Creates a typed contained transform identity.
    #[must_use]
    pub const fn new(epoch: WorkspaceEpoch, generation: ContainedTransformGeneration) -> Self {
        Self { epoch, generation }
    }

    /// Returns the workspace epoch in which the transform began.
    #[must_use]
    pub const fn epoch(self) -> WorkspaceEpoch {
        self.epoch
    }

    /// Returns the contained transform generation.
    #[must_use]
    pub const fn generation(self) -> ContainedTransformGeneration {
        self.generation
    }
}

/// Coordinate lane frozen by a validated press for one transient gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum GestureLane {
    Desktop,
    NativeLocal {
        binding: crate::platform::viewport::WindowBinding,
        coordinate_generation: CoordinateGeneration,
    },
    HostLocal,
    SurfaceLocal,
}

/// Exact journal stream and coordinate lane owning one transient gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct GestureOwner {
    stream: PointerStreamId,
    lane: GestureLane,
}

impl GestureOwner {
    pub(crate) const fn new(stream: PointerStreamId, lane: GestureLane) -> Self {
        Self { stream, lane }
    }

    pub(crate) const fn stream(self) -> PointerStreamId {
        self.stream
    }

    pub(crate) const fn lane(self) -> GestureLane {
        self.lane
    }

    pub(crate) const fn pointer(self) -> PointerId {
        self.stream.pointer()
    }

    pub(crate) const fn lease(self) -> PointerInputLease {
        self.stream.lease()
    }

    pub(crate) const fn is_desktop(self) -> bool {
        matches!(self.lane, GestureLane::Desktop)
    }
}
