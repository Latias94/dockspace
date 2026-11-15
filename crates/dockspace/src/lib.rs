//! Renderer-neutral docking layouts for Rust editor UIs.
//!
//! Build tabs, nested splits and floating groups while keeping pane content in
//! your application. Dockspace owns layout and interaction state; your host
//! supplies measurements, drawing, input and presentation results.
//!
//! # Start with a layout
//!
//! ```
//! use dockspace::model::{
//!     DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
//!     ItemId, RootId, SurfaceId,
//! };
//! use dockspace::policy::DockPolicy;
//! use dockspace::runtime::DockspaceSession;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
//!         SurfaceId::new(1),
//!         DockspaceRootLayout::new(
//!             RootId::new(1),
//!             DockspaceNode::central_tabs([ItemId::new(1), ItemId::new(2)]),
//!         ),
//!     )])?;
//!     let session = DockspaceSession::from_layout(layout, DockPolicy::default())?;
//!     assert!(session.view().item(ItemId::new(1)).is_some());
//!     Ok(())
//! }
//! ```
//!
//! # Connect your host
//!
//! - [`model`] defines stable item identities, layouts and read-only views.
//! - [`policy`] controls docking, closing and resizing permissions.
//! - [`runtime::DockspaceSession`] accepts host frames and application commands.
//! - [`close`] supports immediate decisions and deferred approval while saving.
//!
//! A host executes the returned surface program, commits the frame and reports
//! whether that exact output was presented or dropped. A logical surface need
//! not be an OS window. Native window operations are explicit host requests.
//! The core owns no renderer, event loop, UI framework or async runtime.
//!
//! The default `motion` feature provides interruptible presentation geometry;
//! custom hosts supply its clock and schedule frames. Enable `serde` for layout
//! documents with stable application keys. Pane data and file storage remain
//! application-owned.
//!
//! Start with the [layout commands example], then follow the [engine host guide]
//! for rendering, input, native windows and output settlement. For an existing
//! UI integration, see [`egui_dockspace`] or [`dockspace_winit`].
//!
//! [layout commands example]: https://github.com/Latias94/dockspace/blob/main/crates/dockspace/examples/layout_commands.rs
//! [engine host guide]: https://github.com/Latias94/dockspace/blob/main/crates/dockspace/docs/guide.md
//! [`egui_dockspace`]: https://docs.rs/egui_dockspace
//! [`dockspace_winit`]: https://docs.rs/dockspace_winit

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as dockspace;

pub mod close;
mod engine;
pub mod geometry;
mod graph;
mod ids;
mod input;
mod interaction;
pub mod model;
#[cfg(feature = "motion")]
pub mod motion;
#[cfg(any(feature = "serde", test))]
mod persistence;
mod platform;
pub mod policy;
mod presentation;
pub mod runtime;

#[cfg(test)]
mod behavior_tests;

#[doc(hidden)]
mod public_boundary_contracts {
    //! Raw reducer, scene, provider and ingress protocols remain private even
    //! when every crate feature is enabled.
    //!
    //! ```compile_fail
    //! use dockspace::engine::DockEngine;
    //! ```
    //!
    //! ```compile_fail
    //! use dockspace::graph::Workspace;
    //! ```
    //!
    //! ```compile_fail
    //! use dockspace::ids::NodeId;
    //! ```
    //!
    //! ```compile_fail
    //! use dockspace::presentation::scene::SurfaceSceneStamp;
    //! ```
    //!
    //! ```compile_fail
    //! use dockspace::platform::provider::PlatformObservationLease;
    //! ```
    //!
    //! ```compile_fail
    //! use dockspace::persistence::snapshot::WorkspaceSnapshot;
    //! ```
    //!
    //! ```compile_fail
    //! use dockspace::close::plan::ClosePlanPhase;
    //! ```
}
