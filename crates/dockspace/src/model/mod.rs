//! Product-facing docking layout and read-only query types.
//!
//! This module deliberately hides runtime graph identities. Applications describe layouts with
//! stable item, root, surface, and contained-floating identities; the core compiles that
//! declaration into its validated runtime graph.

mod action;
mod grid;
mod layout;
mod revision;
mod view;
#[cfg(test)]
mod visibility_tests;

pub use crate::graph::{InvalidSplitWeight, InvalidSplitWeights, TabLabel};
pub use crate::ids::{FloatingPresentationId, ItemId, NodeKey, RootId, SurfaceId, TabKey};
pub use action::{
    DockAnchor, DockEdge, DockFraction, DockMoveSource, DockMoveTarget, DockPlacement,
    DockspaceActionOutcome, DockspaceActionRejection, InvalidDockFraction, LogicalSurfaceRejection,
    NativeWindowPlacement, PreparedDockAction, PreparedTab, PreparedTabsGroup, TabGroupDockMode,
};
pub(crate) use action::{PreparedDockActionAuthorityMismatch, ProductAction};
pub use grid::{DockspaceGridError, DockspaceGridShape, DockspaceGridShares};
pub use layout::{
    DockspaceAxis, DockspaceContainedLayout, DockspaceLayout, DockspaceLayoutError, DockspaceNode,
    DockspaceRootLayout, DockspaceSurfaceLayout, RootAllocation,
};
pub use revision::WorkspaceVersion;
pub use view::{
    DockspaceContainedView, DockspaceGridView, DockspaceItemView, DockspaceNodeView,
    DockspaceRootView, DockspaceSplitView, DockspaceSurfaceView, DockspaceTabView,
    DockspaceTabsView, DockspaceView,
};
