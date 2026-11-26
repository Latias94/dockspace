//! Ergonomic integration layer for connecting any Rust UI framework to `dockspace`.
//!
//! While [`crate::runtime`] exposes the complete low-level protocol surface
//! (~150+ types covering immediate/retained frames, bounded storage, and native
//! windowing), most UI framework integrations only need a small, structured
//! subset organized into three tiers:
//!
//! 1. **Tier 1 — Owned display lists & typed visitors** ([`SurfaceDisplayList`],
//!    [`SurfaceProgramVisitor`], [`SurfaceMetrics`], [`SurfaceMeasurer`]):
//!    Compile any [`crate::runtime::SurfacePaintPlan`] into an owned, `'static`
//!    display list for retained UIs and deferred GPU render queues, or implement
//!    only the chrome methods you draw on [`SurfaceProgramVisitor`].
//! 2. **Tier 2 — Logical UI host driver** ([`LogicalHost`], [`LogicalPointerEvent`],
//!    [`PointerClearance`], [`LogicalPresentationToken`]):
//!    Eliminates single-window and multi-surface logical boilerplate (measurement,
//!    paint confirmation, mandatory two-step presentation settlement, and
//!    fail-closed pointer routing) while preserving every core authority invariant.
//! 3. **Tier 3 — Native multiview helpers** ([`NativeReportDigest`],
//!    [`NativeWindowRetirementState`], plus re-exported [`NativeWindows`],
//!    [`NativeEffectOutbox`], [`compile_native_window_facts`],
//!    [`resolve_region_window_hit`]):
//!    Synchronizes committed reports, deduplicates dirty OS windows, and tracks
//!    exact native destruction, output retirement, and callback quiescence.
//!
//! # Quickstart: integrate a custom UI framework
//!
//! ```
//! use dockspace::geometry::{LogicalPoint, LogicalRect};
//! use dockspace::host::{
//!     LogicalHost, LogicalPaintOutcome, LogicalPointerEvent, PointerClearance,
//!     SurfaceDisplayStep, SurfaceMetrics,
//! };
//! use dockspace::model::{
//!     DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, ItemId,
//!     RootId, SurfaceId,
//! };
//! use dockspace::policy::DockPolicy;
//! use dockspace::runtime::{
//!     SurfacePointerButton, SurfacePointerEvent, SurfacePresentationResult,
//! };
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let surface = SurfaceId::new(1);
//!     let first = ItemId::new(1);
//!     let second = ItemId::new(2);
//!     let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
//!         surface,
//!         DockspaceRootLayout::new(RootId::new(1), DockspaceNode::tabs([first, second])),
//!     )])?;
//!
//!     let mut host = LogicalHost::from_layout(layout, DockPolicy::default())?;
//!     let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0)?;
//!     let mut metrics = SurfaceMetrics::new(bounds);
//!
//!     // 1. Measure & compile into an owned `'static` display list.
//!     let (display_list, token) = match host.paint_display_list(surface, &mut metrics)? {
//!         LogicalPaintOutcome::Ready {
//!             payload,
//!             presentation,
//!             ..
//!         } => (payload, presentation),
//!         LogicalPaintOutcome::Deferred { .. } => unreachable!("fresh host has capacity"),
//!     };
//!
//!     // 2. Render panes/tabs/splitters in your UI framework without borrowing `host`.
//!     let second_tab_center = display_list
//!         .tabs()
//!         .find(|tab| tab.item == Some(second))
//!         .map(|tab| {
//!             let rect = tab.visible_bounds;
//!             LogicalPoint::new(rect.x() + rect.width() * 0.5, rect.y() + rect.height() * 0.5)
//!                 .unwrap()
//!         })
//!         .expect("second tab is visible");
//!     assert!(display_list
//!         .steps()
//!         .iter()
//!         .any(|step| matches!(step, SurfaceDisplayStep::Pane(pane) if pane.item == first)));
//!
//!     // 3. Report actual presentation and settle receiver authority in one call.
//!     host.finish_presentation(token, SurfacePresentationResult::Presented)?;
//!
//!     // 4. Route pointer input with explicit framework clearance.
//!     host.submit_pointer_batch([
//!         LogicalPointerEvent::mouse(
//!             surface,
//!             Some(second_tab_center),
//!             SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
//!             PointerClearance::Unobstructed,
//!         ),
//!         LogicalPointerEvent::mouse(
//!             surface,
//!             Some(second_tab_center),
//!             SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
//!             PointerClearance::Unobstructed,
//!         ),
//!     ])?;
//!     assert!(host.view().item(second).unwrap().is_active());
//!     Ok(())
//! }
//! ```

mod logical;
mod measure;
mod native;
mod scene;

#[cfg(test)]
mod tests;

pub use logical::{
    DEFAULT_MOUSE_POINTER, DEFAULT_SCROLL_DEVICE, LogicalHost, LogicalPaintOutcome,
    LogicalPointerEvent, LogicalPresentationError, LogicalPresentationToken,
    LogicalVisitorPaintError, PointerClearance,
};
pub use measure::{SurfaceMeasurer, SurfaceMetrics};
pub use native::{NativeReportDigest, NativeWindowRetirementState};
pub use scene::{
    OwnedContainedFrame, OwnedContainedResize, OwnedContainedTransformPreview, OwnedDragDecoration,
    OwnedDragPreview, OwnedDropAffordanceCluster, OwnedDropAffordanceTarget, OwnedDropGuide,
    OwnedDropGuideTarget, OwnedPane, OwnedPopupBackdrop, OwnedPopupMenu, OwnedPopupMenuRow,
    OwnedPresentationMenuAnchor, OwnedPreviewTab, OwnedPreviewTabBar, OwnedRootKind, OwnedSplitter,
    OwnedSplitterJunction, OwnedSurfaceRoot, OwnedTab, OwnedTabBar, OwnedTabGroupDragRegion,
    OwnedTabLabel, OwnedTabStripControl, OwnedTabStripMember, OwnedTabs, SurfaceDisplayList,
    SurfaceDisplayStep, SurfaceProgramVisitor, compile_display_list, execute_with_visitor,
    visit_with_visitor,
};

// Curated native-integration re-exports so Tier-3 hosts find every shared helper in one place.
pub use crate::runtime::{
    NativeEffectOutbox, NativeRegionWindowHit, NativeWindowClientGeometry, NativeWindowFactsSample,
    NativeWindowInsertError, NativeWindowObservationOutbox, NativeWindowRegistryError,
    NativeWindows, compile_native_window_facts, resolve_region_window_hit,
};
