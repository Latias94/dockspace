//! Ergonomic single-window and multi-surface logical UI host driver.
//!
//! [`LogicalHost`] encapsulates the recurring frame, measurement, display-list
//! painting, two-step presentation settlement, and fail-closed pointer routing
//! choreography on top of [`DockspaceSession`] without weakening any core
//! authority or presentation invariant.

use std::collections::BTreeSet;
use std::fmt;

use thiserror::Error;

use crate::geometry::LogicalPoint;
use crate::model::{
    DockspaceLayout, DockspaceView, PreparedDockAction, SurfaceId, WorkspaceVersion,
};
use crate::policy::DockPolicy;
use crate::runtime::{
    DockPresentationConfig, DockspacePaneFocusRequest, DockspaceRuntimeError, DockspaceSession,
    DockspaceSubmittedAction, HostFrameReport, HostPointerInput, HostPointerRoute,
    PaintedSurfaceOutput, PointerDeviceKind, PointerEventMetadata, PreparedPaneFocusObservation,
    SurfaceMeasurementAnswer, SurfaceMeasurementRequest, SurfacePaintProgramVisitError,
    SurfacePointerButton, SurfacePointerCancelReason, SurfacePointerCapture, SurfacePointerEvent,
    SurfacePointerId, SurfacePointerReceiverFacts, SurfacePresentationBatchReportError,
    SurfacePresentationResult, SurfaceScrollDelta, SurfaceScrollDeviceId, SurfaceScrollEvent,
    SurfaceScrollModifiers, SurfaceScrollMomentum, SurfaceScrollPhase, SurfaceWork,
    SurfaceWorkDeferred,
};

use super::measure::SurfaceMeasurer;
use super::scene::{SurfaceDisplayList, SurfaceProgramVisitor};

/// Default primary mouse pointer identity used by [`LogicalPointerEvent::mouse`].
pub const DEFAULT_MOUSE_POINTER: SurfacePointerId = SurfacePointerId::new(1);

/// Default scroll device identity used by [`LogicalPointerEvent::wheel`].
pub const DEFAULT_SCROLL_DEVICE: SurfaceScrollDeviceId = SurfaceScrollDeviceId::new(1);

/// Framework occlusion proof for a logical pointer event.
///
/// Core hit geometry alone never proves whether a framework overlay, modal, or
/// custom widget covers a docking control. Hosts declare that proof explicitly
/// on every pointer event so occluded clicks fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerClearance {
    /// The host proved no framework widget or overlay obstructs docking at this point.
    Unobstructed,
    /// A framework widget or higher layer covers docking at this point.
    Blocked,
    /// The host cannot prove whether a framework widget obstructs this point.
    Unknown,
}

/// One host-ordered logical pointer event routed through [`LogicalHost`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalPointerEvent {
    /// Stable pointer stream identifier.
    pub pointer: SurfacePointerId,
    /// Logical surface currently under the pointer (or receiving the edge).
    pub surface: SurfaceId,
    /// Surface-local logical coordinates at event time, if known.
    pub position: Option<LogicalPoint>,
    /// Pointer button, motion, scroll, or cancellation edge.
    pub event: SurfacePointerEvent,
    /// Explicit framework occlusion proof at `position`.
    pub clearance: PointerClearance,
    /// Observed capture ownership within this host's input domain.
    pub capture: SurfacePointerCapture,
    /// Device kind, monotonic timestamp, and docking-enablement metadata.
    pub metadata: PointerEventMetadata,
}

impl LogicalPointerEvent {
    /// Creates an untimed mouse event on [`DEFAULT_MOUSE_POINTER`] with
    /// [`SurfacePointerCapture::None`].
    #[must_use]
    pub const fn mouse(
        surface: SurfaceId,
        position: Option<LogicalPoint>,
        event: SurfacePointerEvent,
        clearance: PointerClearance,
    ) -> Self {
        Self {
            pointer: DEFAULT_MOUSE_POINTER,
            surface,
            position,
            event,
            clearance,
            capture: SurfacePointerCapture::None,
            metadata: PointerEventMetadata::untimed(PointerDeviceKind::Mouse),
        }
    }

    /// Creates an untimed mouse move event on [`DEFAULT_MOUSE_POINTER`].
    #[must_use]
    pub const fn moved(
        surface: SurfaceId,
        position: LogicalPoint,
        clearance: PointerClearance,
    ) -> Self {
        Self::mouse(
            surface,
            Some(position),
            SurfacePointerEvent::Moved,
            clearance,
        )
    }

    /// Creates an untimed mouse button-press event on [`DEFAULT_MOUSE_POINTER`].
    #[must_use]
    pub const fn pressed(
        surface: SurfaceId,
        position: LogicalPoint,
        button: SurfacePointerButton,
        clearance: PointerClearance,
    ) -> Self {
        Self::mouse(
            surface,
            Some(position),
            SurfacePointerEvent::ButtonPressed(button),
            clearance,
        )
    }

    /// Creates an untimed mouse button-release event on [`DEFAULT_MOUSE_POINTER`].
    #[must_use]
    pub const fn released(
        surface: SurfaceId,
        position: LogicalPoint,
        button: SurfacePointerButton,
        clearance: PointerClearance,
    ) -> Self {
        Self::mouse(
            surface,
            Some(position),
            SurfacePointerEvent::ButtonReleased(button),
            clearance,
        )
    }

    /// Creates a discrete mouse wheel scroll event on [`DEFAULT_MOUSE_POINTER`]
    /// and [`DEFAULT_SCROLL_DEVICE`].
    #[must_use]
    pub const fn wheel(
        surface: SurfaceId,
        position: LogicalPoint,
        delta: SurfaceScrollDelta,
        clearance: PointerClearance,
    ) -> Self {
        Self::scroll(
            surface,
            Some(position),
            SurfaceScrollEvent::new(
                DEFAULT_SCROLL_DEVICE,
                SurfaceScrollPhase::Discrete { delta },
                SurfaceScrollMomentum::Direct,
                SurfaceScrollModifiers::new(false, false, false, false),
            ),
            clearance,
        )
    }

    /// Creates a scroll event carrying an explicit lossless [`SurfaceScrollEvent`].
    #[must_use]
    pub const fn scroll(
        surface: SurfaceId,
        position: Option<LogicalPoint>,
        scroll: SurfaceScrollEvent,
        clearance: PointerClearance,
    ) -> Self {
        Self::mouse(
            surface,
            position,
            SurfacePointerEvent::Scrolled(scroll),
            clearance,
        )
    }

    /// Overrides the pointer stream identity.
    #[must_use]
    pub const fn with_pointer(mut self, pointer: SurfacePointerId) -> Self {
        self.pointer = pointer;
        self
    }

    /// Overrides capture ownership for this edge.
    #[must_use]
    pub const fn with_capture(mut self, capture: SurfacePointerCapture) -> Self {
        self.capture = capture;
        self
    }

    /// Overrides device/timing metadata for this edge.
    #[must_use]
    pub const fn with_metadata(mut self, metadata: PointerEventMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Toggles docking enablement (for example holding Shift to float) on metadata.
    #[must_use]
    pub const fn with_docking_enabled(mut self, enabled: bool) -> Self {
        self.metadata = self.metadata.with_docking_enabled(enabled);
        self
    }
}

/// Affine token holding the painted surface outputs awaiting renderer settlement.
///
/// After the UI framework actually presents (or drops/cancels) the frame, pass
/// this token to [`LogicalHost::finish_presentation`] so the core records the
/// outcome and commits the settlement frame before subsequent pointer routing.
#[must_use = "report presentation outcome via LogicalHost::finish_presentation or deliberately drop to retire"]
pub struct LogicalPresentationToken {
    outputs: Vec<PaintedSurfaceOutput>,
}

impl LogicalPresentationToken {
    /// Returns the number of painted outputs awaiting presentation settlement.
    #[must_use]
    pub fn len(&self) -> usize {
        self.outputs.len()
    }

    /// Returns whether no painted outputs remain in this token.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    /// Consumes this token and returns the underlying affine [`PaintedSurfaceOutput`] handles.
    #[must_use]
    pub fn into_outputs(self) -> Vec<PaintedSurfaceOutput> {
        self.outputs
    }
}

impl fmt::Debug for LogicalPresentationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LogicalPresentationToken")
            .field("output_count", &self.outputs.len())
            .finish()
    }
}

/// Result of preparing and painting one surface through [`LogicalHost`].
#[derive(Debug)]
pub enum LogicalPaintOutcome<T> {
    /// The surface program was accepted and committed; report actual presentation
    /// using `presentation` once the renderer finishes presenting or dropping it.
    Ready {
        /// Renderer output (such as a [`SurfaceDisplayList`] or `()`).
        payload: T,
        /// Affine presentation token to pass to [`LogicalHost::finish_presentation`].
        presentation: LogicalPresentationToken,
        /// Committed report from the paint frame.
        report: HostFrameReport,
    },
    /// Painting was deferred (for example bounded output backpressure or unavailable measurements).
    Deferred {
        /// Core reason painting was deferred.
        reason: SurfaceWorkDeferred,
        /// Committed report from the frame.
        report: HostFrameReport,
    },
}

impl<T> LogicalPaintOutcome<T> {
    /// Returns the committed [`HostFrameReport`] regardless of ready/deferred state.
    #[must_use]
    pub const fn report(&self) -> &HostFrameReport {
        match self {
            Self::Ready { report, .. } | Self::Deferred { report, .. } => report,
        }
    }
}

/// Failure while reporting presentation outcome or committing its settlement frame.
#[derive(Debug, Error)]
pub enum LogicalPresentationError {
    /// One or more outputs in the token were foreign, stale, or already retired.
    #[error(transparent)]
    Report(#[from] SurfacePresentationBatchReportError),
    /// Reporting succeeded, but the settlement commit frame returned an error.
    #[error(transparent)]
    Settlement(#[from] DockspaceRuntimeError),
}

/// Failure when painting through a fallible [`SurfaceProgramVisitor`] on [`LogicalHost`].
#[derive(Debug, Error)]
pub enum LogicalVisitorPaintError<E> {
    /// Core host-frame or measurement failure.
    #[error(transparent)]
    Runtime(#[from] DockspaceRuntimeError),
    /// The sealed program or visitor failed during paint traversal.
    #[error(transparent)]
    Visit(#[from] SurfacePaintProgramVisitError<E>),
}

/// High-level driver for single-window and multi-surface logical UI frameworks.
///
/// Wraps a [`DockspaceSession`] with host-local pointer authority enabled and
/// provides ergonomic, invariant-preserving helpers for:
/// - painting into an owned [`SurfaceDisplayList`] or [`SurfaceProgramVisitor`],
/// - two-step presentation settlement via [`LogicalPresentationToken`],
/// - fail-closed pointer/scroll/cancellation submission via [`LogicalPointerEvent`],
/// - application command dispatch and focus handoffs.
///
/// Use [`Self::session`] and [`Self::session_mut`] whenever you need full access
/// to lower-level [`DockspaceSession`] APIs (such as close plans, persistence,
/// suspended updates, or custom batching).
#[derive(Debug)]
pub struct LogicalHost {
    session: DockspaceSession,
}

impl LogicalHost {
    /// Wraps an existing [`DockspaceSession`] and enables host-local pointer input.
    ///
    /// # Errors
    ///
    /// Returns an error if another incompatible pointer provider (such as a
    /// managed native host) is already active on `session`.
    pub fn new(mut session: DockspaceSession) -> Result<Self, DockspaceRuntimeError> {
        session.enable_host_pointer()?;
        Ok(Self { session })
    }

    /// Creates a new logical host from a validated layout and policy.
    ///
    /// # Errors
    ///
    /// Returns any session initialization error.
    pub fn from_layout(
        layout: DockspaceLayout,
        policy: DockPolicy,
    ) -> Result<Self, DockspaceRuntimeError> {
        Self::new(DockspaceSession::from_layout(layout, policy)?)
    }

    /// Creates a new logical host with explicit renderer-neutral presentation config.
    ///
    /// # Errors
    ///
    /// Returns any session initialization error.
    pub fn from_layout_with_presentation_config(
        layout: DockspaceLayout,
        policy: DockPolicy,
        config: DockPresentationConfig,
    ) -> Result<Self, DockspaceRuntimeError> {
        Self::new(DockspaceSession::from_layout_with_presentation_config(
            layout, policy, config,
        )?)
    }

    /// Borrows the underlying [`DockspaceSession`].
    #[must_use]
    pub const fn session(&self) -> &DockspaceSession {
        &self.session
    }

    /// Mutably borrows the underlying [`DockspaceSession`] for advanced workflows.
    #[must_use]
    pub fn session_mut(&mut self) -> &mut DockspaceSession {
        &mut self.session
    }

    /// Unwraps the underlying [`DockspaceSession`].
    #[must_use]
    pub fn into_session(self) -> DockspaceSession {
        self.session
    }

    /// Returns the read-only product view of the published workspace.
    #[must_use]
    pub fn view(&self) -> DockspaceView<'_> {
        self.session.view()
    }

    /// Returns the current published workspace version.
    #[must_use]
    pub const fn version(&self) -> WorkspaceVersion {
        self.session.version()
    }

    /// Submits a single prepared application action in one atomic host frame.
    ///
    /// # Errors
    ///
    /// Returns any frame prelude, submission, or commit error.
    pub fn dispatch_action(
        &mut self,
        action: PreparedDockAction,
    ) -> Result<(DockspaceSubmittedAction, HostFrameReport), DockspaceRuntimeError> {
        let mut frame = self.session.begin_host_frame()?;
        let submitted = frame.submit_prepared_action(action)?;
        let report = frame.commit()?;
        Ok((submitted, report))
    }

    /// Submits one logical pointer event with explicit framework clearance.
    ///
    /// Automatically resolves `host_pointer_hit_for_device`, routes active
    /// capture manipulations back to their originating surface, applies
    /// fail-closed receiver facts for [`PointerClearance::Blocked`] and
    /// [`PointerClearance::Unknown`], and commits the frame.
    ///
    /// # Errors
    ///
    /// Returns any host-frame or pointer submission error.
    pub fn submit_pointer(
        &mut self,
        event: LogicalPointerEvent,
    ) -> Result<HostFrameReport, DockspaceRuntimeError> {
        self.submit_pointer_batch([event])
    }

    /// Submits an ordered batch of logical pointer events in one atomic host frame.
    ///
    /// # Errors
    ///
    /// Returns any host-frame or pointer submission error.
    pub fn submit_pointer_batch(
        &mut self,
        events: impl IntoIterator<Item = LogicalPointerEvent>,
    ) -> Result<HostFrameReport, DockspaceRuntimeError> {
        let mut frame = self.session.begin_host_frame()?;
        for item in events {
            let hit = item.position.and_then(|point| {
                frame.host_pointer_hit_for_device(item.surface, point, item.metadata.device())
            });
            let blocked_surface = match (hit.as_ref(), item.clearance) {
                (Some(hit), PointerClearance::Blocked) => Some(hit.surface()),
                _ => None,
            };
            let facts = match (hit.as_ref(), blocked_surface.as_ref(), item.clearance) {
                (Some(hit), _, PointerClearance::Unobstructed) => hit.unobstructed_receiver_facts(),
                (_, Some(surface), PointerClearance::Blocked) => {
                    SurfacePointerReceiverFacts::blocked_hover(surface)
                        .with_blocked_delivery(surface)
                }
                _ => SurfacePointerReceiverFacts::unknown(),
            };
            let hover_route = hit
                .zip(item.position)
                .map_or(HostPointerRoute::Unknown, |(hit, point)| {
                    HostPointerRoute::new(hit.surface(), point)
                });
            let manipulation_route = item
                .position
                .zip(frame.host_pointer_manipulation_surface(item.pointer))
                .map_or(hover_route, |(point, source)| {
                    HostPointerRoute::new(source, point)
                });
            frame.submit_host_pointer(
                HostPointerInput::new(
                    item.pointer,
                    item.event,
                    manipulation_route,
                    hover_route,
                    item.capture,
                    facts,
                )
                .with_metadata(item.metadata),
            )?;
        }
        frame.commit()
    }

    /// Returns the surface currently owning an active manipulation (such as a
    /// tab drag, splitter drag, or contained-window drag/resize) for `pointer`.
    ///
    /// UI frameworks can use this to maintain pointer capture and avoid letting
    /// unrelated widgets steal drag input while docking owns the gesture.
    #[must_use]
    pub fn active_manipulation_surface(&self, pointer: SurfacePointerId) -> Option<SurfaceId> {
        self.session
            .host_pointer_manipulation_surface(pointer)
            .map(|surface| surface.surface())
    }

    /// Returns whether `pointer` currently owns an active docking manipulation.
    #[must_use]
    pub fn has_active_manipulation(&self, pointer: SurfacePointerId) -> bool {
        self.active_manipulation_surface(pointer).is_some()
    }

    /// Submits one discrete mouse wheel scroll event with explicit clearance.
    ///
    /// Uses [`DEFAULT_MOUSE_POINTER`] and [`DEFAULT_SCROLL_DEVICE`]. For
    /// trackpad phaseful sequences or modifier-aware scroll events, pass
    /// [`LogicalPointerEvent::scroll`] to [`Self::submit_pointer`].
    ///
    /// # Errors
    ///
    /// Returns any host-frame or scroll validation error.
    pub fn submit_wheel(
        &mut self,
        surface: SurfaceId,
        position: LogicalPoint,
        delta: SurfaceScrollDelta,
        clearance: PointerClearance,
    ) -> Result<HostFrameReport, DockspaceRuntimeError> {
        self.submit_pointer(LogicalPointerEvent::wheel(
            surface, position, delta, clearance,
        ))
    }

    /// Cancels an active logical pointer stream (for example on `Escape` or window blur).
    ///
    /// # Errors
    ///
    /// Returns any host-frame or pointer submission error.
    pub fn cancel_pointer(
        &mut self,
        pointer: SurfacePointerId,
        surface: SurfaceId,
        reason: SurfacePointerCancelReason,
    ) -> Result<HostFrameReport, DockspaceRuntimeError> {
        self.submit_pointer(
            LogicalPointerEvent::mouse(
                surface,
                None,
                SurfacePointerEvent::StreamCancelled(reason),
                PointerClearance::Unknown,
            )
            .with_pointer(pointer),
        )
    }

    /// Measures `surface` via `measurer`, compiles its paint program into an
    /// owned `'static` [`SurfaceDisplayList`], confirms paint acceptance, and
    /// commits the frame.
    ///
    /// If [`LogicalPaintOutcome::Ready`] is returned, pass its
    /// [`LogicalPresentationToken`] to [`Self::finish_presentation`] after the
    /// renderer actually presents or drops the output.
    ///
    /// # Errors
    ///
    /// Returns any core measurement, program validation, or frame commit error.
    pub fn paint_display_list<M: SurfaceMeasurer>(
        &mut self,
        surface: SurfaceId,
        measurer: &mut M,
    ) -> Result<LogicalPaintOutcome<SurfaceDisplayList>, DockspaceRuntimeError> {
        self.paint_display_list_with(surface, |request| measurer.measure_request(request))
    }

    /// Closure-based variant of [`Self::paint_display_list`] accepting any
    /// custom measurement resolver (such as [`super::SurfaceMetrics::with_tab_width`]).
    ///
    /// # Errors
    ///
    /// Returns any measurement resolver or core host-frame error.
    pub fn paint_display_list_with<F, E>(
        &mut self,
        surface: SurfaceId,
        measure: F,
    ) -> Result<LogicalPaintOutcome<SurfaceDisplayList>, E>
    where
        F: FnMut(SurfaceMeasurementRequest<'_>) -> Result<SurfaceMeasurementAnswer, E>,
        E: From<DockspaceRuntimeError>,
    {
        let mut frame = self.session.begin_host_frame().map_err(E::from)?;
        match frame.prepare_surface_with(surface, measure)? {
            SurfaceWork::Ready(plan) => {
                let (display_list, receipt) = plan.compile_display_list().map_err(E::from)?;
                frame.confirm_surface_painted(receipt).map_err(E::from)?;
                let mut report = frame.commit().map_err(E::from)?;
                let presentation = LogicalPresentationToken {
                    outputs: report.take_painted_outputs(),
                };
                Ok(LogicalPaintOutcome::Ready {
                    payload: display_list,
                    presentation,
                    report,
                })
            }
            SurfaceWork::Deferred(reason) => {
                let report = frame.commit().map_err(E::from)?;
                Ok(LogicalPaintOutcome::Deferred { reason, report })
            }
        }
    }

    /// Measures `surface` via `measurer`, executes the canonical surface program
    /// directly through an immediate-mode [`SurfaceProgramVisitor`], confirms
    /// paint acceptance, and commits the frame.
    ///
    /// # Errors
    ///
    /// Returns [`LogicalVisitorPaintError::Runtime`] on core/frame errors or
    /// [`LogicalVisitorPaintError::Visit`] if the visitor fails.
    pub fn paint_with_visitor<M, V>(
        &mut self,
        surface: SurfaceId,
        measurer: &mut M,
        visitor: &mut V,
    ) -> Result<LogicalPaintOutcome<()>, LogicalVisitorPaintError<V::Error>>
    where
        M: SurfaceMeasurer,
        V: SurfaceProgramVisitor,
    {
        let mut frame = self.session.begin_host_frame()?;
        match frame.prepare_surface_with(surface, |request| measurer.measure_request(request))? {
            SurfaceWork::Ready(plan) => {
                let receipt = plan.execute_visitor(visitor)?;
                frame.confirm_surface_painted(receipt)?;
                let mut report = frame.commit()?;
                let presentation = LogicalPresentationToken {
                    outputs: report.take_painted_outputs(),
                };
                Ok(LogicalPaintOutcome::Ready {
                    payload: (),
                    presentation,
                    report,
                })
            }
            SurfaceWork::Deferred(reason) => {
                let report = frame.commit()?;
                Ok(LogicalPaintOutcome::Deferred { reason, report })
            }
        }
    }

    /// Reports the actual renderer presentation result for `token` and executes
    /// the mandatory settlement commit so presented receiver authority updates
    /// immediately before subsequent pointer input.
    ///
    /// # Errors
    ///
    /// Returns any presentation report or settlement commit error.
    pub fn finish_presentation(
        &mut self,
        token: LogicalPresentationToken,
        result: SurfacePresentationResult,
    ) -> Result<HostFrameReport, LogicalPresentationError> {
        self.session
            .report_surface_presentations(token.outputs, result)?;
        Ok(self.session.begin_host_frame()?.commit()?)
    }

    /// Queries the currently pending pane focus request for `surface`, if any.
    ///
    /// # Errors
    ///
    /// Returns any host-frame prelude error.
    pub fn pane_focus_request(
        &mut self,
        surface: SurfaceId,
    ) -> Result<Option<DockspacePaneFocusRequest>, DockspaceRuntimeError> {
        let frame = self.session.begin_host_frame()?;
        let request = frame.pane_focus_request(surface);
        frame.commit()?;
        Ok(request)
    }

    /// Reports the host's observed focus result for a previously queried
    /// [`DockspacePaneFocusRequest`] and commits it.
    ///
    /// # Errors
    ///
    /// Returns any host-frame or focus-observation submission error.
    pub fn submit_pane_focus_observation(
        &mut self,
        observation: PreparedPaneFocusObservation,
    ) -> Result<HostFrameReport, DockspaceRuntimeError> {
        let mut frame = self.session.begin_host_frame()?;
        frame.submit_pane_focus_observation(observation)?;
        frame.commit()
    }

    /// Merges and deduplicates [`HostFrameReport::repaint_surfaces`] across
    /// multiple reports (for example a paint report and its settlement report).
    #[must_use]
    pub fn merge_repaint_surfaces(reports: &[&HostFrameReport]) -> Vec<SurfaceId> {
        let mut merged = BTreeSet::new();
        for report in reports {
            merged.extend(report.repaint_surfaces().iter().copied());
        }
        merged.into_iter().collect()
    }
}
