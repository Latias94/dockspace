//! Input, scheduling and optional native-window helpers for winit-based hosts.
//!
//! Use these helpers with your own UI framework and renderer. The host keeps its
//! event loop and docking session; this crate adds no UI or GPU dependency.
//!
//! - [`SurfaceCoordinates`] converts window-client physical pixels into surface
//!   logical points, including application zoom in the supplied scale.
//! - [`MouseInput`] samples per-window mouse events and invalidates stale positions
//!   after geometry changes, cursor departure and DPI changes.
//! - [`RepaintScheduler`] tracks repaint deadlines without a polling loop.
//! - [`backend`] re-exports the matching winit version.
//!
//! # Map a surface's coordinates
//!
//! ```
//! use dockspace::geometry::{PhysicalPoint, ScaleFactor};
//! use dockspace_winit::{SurfaceCoordinates, backend::dpi::PhysicalPosition};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let coordinates = SurfaceCoordinates::new(
//!         PhysicalPoint::new(100.0, 50.0)?,
//!         ScaleFactor::new(2.0)?,
//!     );
//!     let point = coordinates.to_surface(PhysicalPosition::new(140.0, 90.0))?;
//!     assert_eq!((point.x(), point.y()), (20.0, 20.0));
//!     Ok(())
//! }
//! ```
//!
//! The origin is a position inside the window client area, not on the desktop.
//! The scale must match the renderer's pixels per logical point, including zoom.
//!
//! # Multiple windows
//!
//! Keep mouse and repaint state per OS window and coordinate mappings per logical
//! surface. Route events by winit window ID; one docking session owns the shared
//! workspace and the host supplies an ordered input stream across windows.
//!
//! Enable `native-windows` for an exact-binding resource registry, or
//! `native-geometry` for window observations and physical placement helpers. The
//! `native` feature enables both. These helpers do not own a native state machine.
//!
//! The host supplies receiver delivery, pointer capture and actual presentation
//! results. Mouse samples have no original OS timestamp and do not manufacture
//! timed metadata. For egui, prefer its adapter's `winit` or `wgpu` feature so each
//! event enters the docking session once.
//!
//! See the [crate guide] for event-loop integration and runnable software-rendered
//! single-window and multiview examples.
//!
//! [crate guide]: https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/README.md

mod coordinates;
mod mouse;
mod repaint;

#[cfg(any(feature = "native-windows", feature = "native-geometry"))]
pub mod native;

pub use coordinates::SurfaceCoordinates;
pub use mouse::{MouseInput, MouseSample, map_mouse_button};
pub use repaint::RepaintScheduler;
pub use winit as backend;
