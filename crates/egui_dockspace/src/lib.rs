//! Docking layouts for egui, backed by the renderer-neutral [`dockspace`] core.
//!
//! Add tabs, splitters, docking guides and floating groups in one OS window or
//! browser canvas. Your application keeps pane data and implements [`PaneView`];
//! the adapter supplies egui rendering, styling, input and close requests.
//!
//! # Define your panes
//!
//! ```
//! use egui_dockspace::{
//!     Dockspace, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
//!     DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
//! };
//!
//! struct Documents {
//!     text: String,
//! }
//!
//! impl PaneView for Documents {
//!     fn title(&self, _item: ItemId) -> Option<egui::WidgetText> {
//!         Some("Document".into())
//!     }
//!
//!     fn ui(&mut self, _item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
//!         PaneResponse::from(&ui.text_edit_multiline(&mut self.text))
//!     }
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let item = ItemId::new(1);
//!     let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
//!         SurfaceId::new(1),
//!         DockspaceRootLayout::new(RootId::new(1), DockspaceNode::central_tabs([item])),
//!     )])?;
//!     let dockspace = Dockspace::builder("editor", layout).build()?;
//!     let _panes = Documents {
//!         text: "Hello, Dockspace".into(),
//!     };
//!     assert!(dockspace.view().item(item).is_some());
//!     Ok(())
//! }
//! ```
//!
//! # Choose a host
//!
//! For drag docking, the host must own the complete egui run and final output.
//! Enable `wgpu` to use the optional single-window winit runner, or integrate
//! [`Dockspace::run_single_surface`] with your own renderer. Split the returned
//! [`DockspaceOutput`] at the renderer boundary and report its actual presentation
//! result through the receipt. The [small winit editor] shows a complete app.
//!
//! An ordinary `eframe::App::ui` callback can use
//! [`Dockspace::show_single_surface`] for rendering, selection, close, menus and
//! keyboard resizing. Drag docking and pane-focus confirmation require the
//! complete-output host. Multiple native OS windows use a separate experimental
//! adapter; see [host choices].
//!
//! # Customize
//!
//! [`DockStyle`] follows egui's theme and offers tab presets, density and visual
//! overrides. [`PaneView`] callbacks customize tabs, controls, guides and previews
//! while the adapter retains docking input and accessibility. [`DockInputConfig`]
//! configures gestures and shortcuts.
//!
//! Motion is enabled by default. Set `motion::MotionSettings::mode` to
//! `motion::MotionMode::Off` to disable it at runtime, or use
//! `default-features = false` to omit motion support. Enable `serde` for layout
//! save/restore. See the [integration guide] for motion preferences, pane focus,
//! deferred close decisions and persistence.
//!
//! Pane callbacks are synchronous and may repeat across egui passes. Keep I/O in
//! application-owned background tasks and return completed data to the UI thread.
//!
//! [small winit editor]: https://github.com/Latias94/dockspace/blob/main/crates/egui_dockspace/examples/winit.rs
//! [host choices]: https://github.com/Latias94/dockspace/blob/main/crates/egui_dockspace/README.md#host-choices
//! [integration guide]: https://github.com/Latias94/dockspace/blob/main/crates/egui_dockspace/docs/guide.md

#![forbid(unsafe_code)]

pub mod drag_preview;
pub mod drop_guide;
pub mod drop_preview;
pub mod pane;
pub mod style;
pub mod tab;
#[cfg(feature = "winit")]
pub mod winit;

mod builder;
mod error;
mod error_detail;
mod facade;
mod focus;
mod input;
#[cfg(feature = "native-render-support")]
#[doc(hidden)]
pub mod native_support;
mod output_proof;
mod pass_settlement;
mod pointer;
mod render;
mod response;
mod touch_menu;

pub use builder::DockspaceBuilder;
pub use dockspace::close::{
    CloseDecision, CloseDecisionToken, CloseItemDecisionState, ClosePlanTarget,
    ClosePlanTargetKind, CloseRequestId, DeferredCloseDecision, DeferredCloseToken,
    SurfaceCloseDisposition,
};
pub use dockspace::geometry::{
    AxisSizing, LogicalRect, PaneSizing, PaneSizingBuilder, PaneSizingError,
};
pub use dockspace::model::{
    DockAnchor, DockEdge, DockFraction, DockPlacement, DockspaceActionOutcome,
    DockspaceActionRejection, DockspaceAxis, DockspaceContainedLayout, DockspaceContainedView,
    DockspaceLayout, DockspaceLayoutError, DockspaceNode, DockspaceNodeView, DockspaceRootLayout,
    DockspaceRootView, DockspaceSplitView, DockspaceSurfaceLayout, DockspaceSurfaceView,
    DockspaceTabsView, DockspaceView, FloatingPresentationId, InvalidDockFraction, ItemId,
    PreparedDockAction, PreparedTabsGroup, RootAllocation, RootId, SurfaceId, TabKey,
    WorkspaceVersion,
};
#[cfg(feature = "motion")]
pub use dockspace::motion;
pub use dockspace::runtime::{
    DockspaceCloseInertReason, DockspaceCloseOutcome as DockspaceAppliedClose, DockspaceClosePhase,
    DockspaceCloseRejection as DockspaceCloseApplicationRejection, DockspaceCloseRequestRejection,
    DockspaceCloseResolution, DockspaceContextMenuRequest, DockspaceEvent, DockspaceEventId,
    DockspaceEventKind, DockspaceFocusTarget, DockspaceLayoutChange,
    DockspaceManipulationCancelReason, DockspaceManipulationDelivery,
    DockspaceManipulationFeedback, DockspaceManipulationFeedbackId, DockspaceManipulationId,
    DockspaceManipulationPendingReason, DockspaceManipulationRejection,
    DockspaceManipulationStatus, DockspaceNotification, DockspaceTouchEvent, PreparedCloseRequest,
};
#[cfg(feature = "serde")]
pub use dockspace::runtime::{
    DockspaceDocumentBootstrap, DockspaceDocumentId, DockspacePersistenceError,
    DockspacePersistenceErrorKind,
};
pub use dockspace::runtime::{PaneOverflowPolicy, SurfacePresentationResult};
pub use drag_preview::{DockspaceDragSourceKind, DragPreview};
pub use drop_guide::{
    DockspaceDropDirection, DockspaceDropEligibility, DockspaceGuideScope, DropGuide,
};
pub use drop_preview::{
    DockspaceDropAction, DockspaceDropTargetFeedback, DockspaceDropTargetScope, DropPreview,
    DropPreviewTab,
};
pub use error::{DockspaceError, DockspaceErrorKind};
pub use facade::Dockspace;
pub use input::{DockInputConfig, DockingActivation, SplitterDoubleClick};
pub use pane::{PaneFocusState, PaneResponse, PaneView};
pub use pass_settlement::{DockspaceOutput, DockspaceOutputReceipt, run_ui};
pub use response::{
    DockspaceActionResult, DockspaceActionStatus, DockspaceCapability, DockspaceCloseItem,
    DockspaceCloseOutcome, DockspaceClosePlan, DockspaceCloseRequest, DockspaceCloseRequestResult,
    DockspaceCloseRequestStatus, DockspaceCloseResult, DockspaceInteractionCapabilities,
    DockspaceMutation, DockspaceResponse, DockspaceSurfaceCommitStatus, DockspaceSurfaceStatus,
    DockspaceUnavailableReason,
};
pub use style::{
    DockDensity, DockFrameVisuals, DockStyle, DockStyleError, DockVisualOverrides,
    TabCloseVisibility, TabGroupGripVisibility, TabStylePreset,
};
pub use tab::{Tab, TabBar, TabBarId, TabBarLayout, TabBarSlot, TabClose, TabVisuals};

#[cfg(test)]
mod events_tests;

#[doc(hidden)]
mod public_boundary_contracts {
    //! Core-owned close lifecycle phases are intentionally not re-exported.
    //!
    //! ```compile_fail
    //! use egui_dockspace::ClosePlanPhase;
    //! ```
}
