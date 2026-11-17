//! Product-level surface measurement seam.

pub use crate::presentation::manifest::{
    MeasurementValueError, TabListMenuMetrics, TabStripControlMetric, TabStripControlMetrics,
    TabStripControlPlacement, TabStripMetrics,
};

use crate::geometry::{LogicalRect, PaneSizing};
use crate::ids::{ItemId, SurfaceId, TabKey};
use crate::policy::TabBarVisibility;
use crate::presentation::manifest::{Measurement, SurfaceMeasurements, TabIntrinsic};
use crate::presentation::scene::{PaneSceneId, SurfaceScene};

use super::{
    DockspaceHostFrame, DockspaceInteractionError, DockspaceRuntimeError, DockspaceVisualId,
    SurfaceUnavailableReason, SurfaceWork, SurfaceWorkDeferred,
};

/// Uniform measurements for a renderer whose panes and tabs share one metric.
///
/// Rich adapters should use [`DockspaceHostFrame::measure_surface_with`] so
/// pane, tab, popup, and tab-strip values can vary independently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UniformSurfaceMetrics {
    pub(super) bounds: LogicalRect,
    pub(super) pane_sizing: PaneSizing,
    pub(super) tab_intrinsic: TabIntrinsic,
    pub(super) tab_strip: TabStripMetrics,
}

impl UniformSurfaceMetrics {
    /// Validates one complete uniform measurement profile.
    ///
    /// # Errors
    ///
    /// Returns an error when the tab content width is negative or non-finite.
    pub fn new(
        bounds: LogicalRect,
        pane_sizing: impl Into<PaneSizing>,
        tab_content_width: f64,
    ) -> Result<Self, DockspaceInteractionError> {
        let tab_intrinsic = TabIntrinsic::new(tab_content_width)
            .map_err(|_| DockspaceInteractionError::InvalidMeasurementProfile)?;
        let tab_strip = TabStripMetrics::new(0.0, 0.0)
            .map_err(|_| DockspaceInteractionError::InvalidMeasurementProfile)?;
        Ok(Self {
            bounds,
            pane_sizing: pane_sizing.into(),
            tab_intrinsic,
            tab_strip,
        })
    }
}

/// One core-derived product measurement question.
///
/// Structural node identities and exact-set manifest keys remain private. The
/// adapter sees only stable product items, opaque visual identities, and the
/// policy fact needed to measure renderer chrome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceMeasurementRequest<'a> {
    /// Measure the logical rectangle occupied by the dock layout.
    DockBounds {
        /// Logical surface being measured.
        surface: SurfaceId,
    },
    /// Measure the logical rectangle in which transient popup chrome may appear.
    PopupPlaneBounds {
        /// Logical surface being measured.
        surface: SurfaceId,
    },
    /// Measure one pane's complete content sizing declaration.
    ///
    /// Report the smallest useful viewport, not the full document extent.
    /// Hosts own content scrolling, wrapping and canvas navigation. A surface
    /// smaller than the combined minima can still clip panes out of view.
    PaneSizing {
        /// Stable renderer identity of the real content placement.
        visual: DockspaceVisualId,
        /// Real pane item, including content under an inactive layout tab.
        ///
        /// Every visible item needs sizing metadata independently of its
        /// content being loaded. Hidden items are omitted until shown again.
        /// Core uses these values for exact potential drag layouts; an unavailable
        /// declaration defers the complete surface contribution.
        item: Option<ItemId>,
    },
    /// Measure one tab's text-and-icon content width before core padding.
    TabIntrinsic {
        /// Stable renderer identity of the tab.
        visual: DockspaceVisualId,
        /// Stable renderer identity of the tab strip owning this tab.
        bar: DockspaceVisualId,
        /// Direct member or prospective whole-group layout key.
        /// A prospective key requests metrics without declaring a visible header.
        key: TabKey,
        /// Core-frozen application or structural-container label.
        label: crate::graph::TabLabel<'a>,
    },
    /// Measure renderer-owned chrome around one tab strip.
    TabStrip {
        /// Stable renderer identity of the tab strip.
        visual: DockspaceVisualId,
        /// Visibility of the strip being measured, including potential chrome
        /// for an automatically hidden bar. This is not a request to paint it.
        visibility: TabBarVisibility,
    },
}

/// One answer to a [`SurfaceMeasurementRequest`].
///
/// The answer variant must match the request variant. Core performs that join,
/// validates scalar values, builds the exact measurement roster, and rejects
/// the complete contribution on any mismatch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceMeasurementAnswer {
    /// Logical bounds for either a dock layout or popup plane request.
    Bounds(LogicalRect),
    /// Logical content bounds and optional reset preferences for one pane request.
    PaneSizing(PaneSizing),
    /// Text-and-icon content width for one tab request.
    TabIntrinsic(f64),
    /// Renderer-owned tab-strip chrome measurements.
    TabStrip(TabStripMetrics),
    /// The adapter participated but could not authoritatively measure the value.
    Unavailable(SurfaceUnavailableReason),
}

impl SurfaceMeasurementAnswer {
    fn into_bounds(self) -> Result<Measurement<LogicalRect>, DockspaceInteractionError> {
        match self {
            Self::Bounds(value) => Ok(Measurement::Measured(value)),
            Self::Unavailable(reason) => Ok(Measurement::Unavailable(reason.into())),
            Self::PaneSizing(_) | Self::TabIntrinsic(_) | Self::TabStrip(_) => {
                Err(DockspaceInteractionError::MeasurementAnswerMismatch)
            }
        }
    }

    fn into_pane_sizing(self) -> Result<Measurement<PaneSizing>, DockspaceInteractionError> {
        match self {
            Self::PaneSizing(value) => Ok(Measurement::Measured(value)),
            Self::Unavailable(reason) => Ok(Measurement::Unavailable(reason.into())),
            Self::Bounds(_) | Self::TabIntrinsic(_) | Self::TabStrip(_) => {
                Err(DockspaceInteractionError::MeasurementAnswerMismatch)
            }
        }
    }

    fn into_tab_intrinsic(self) -> Result<Measurement<TabIntrinsic>, DockspaceInteractionError> {
        match self {
            Self::TabIntrinsic(content_width) => TabIntrinsic::new(content_width)
                .map(Measurement::Measured)
                .map_err(|_| DockspaceInteractionError::InvalidMeasurementProfile),
            Self::Unavailable(reason) => Ok(Measurement::Unavailable(reason.into())),
            Self::Bounds(_) | Self::PaneSizing(_) | Self::TabStrip(_) => {
                Err(DockspaceInteractionError::MeasurementAnswerMismatch)
            }
        }
    }

    fn into_tab_strip(self) -> Result<Measurement<TabStripMetrics>, DockspaceInteractionError> {
        match self {
            Self::TabStrip(value) => Ok(Measurement::Measured(value)),
            Self::Unavailable(reason) => Ok(Measurement::Unavailable(reason.into())),
            Self::Bounds(_) | Self::PaneSizing(_) | Self::TabIntrinsic(_) => {
                Err(DockspaceInteractionError::MeasurementAnswerMismatch)
            }
        }
    }
}

impl DockspaceHostFrame<'_> {
    /// Supplies one complete uniform measurement answer for a frozen surface.
    ///
    /// # Errors
    ///
    /// Returns an error when the surface is outside the frozen roster, already
    /// answered, or the measurements cannot satisfy the core manifest.
    pub fn measure_surface(
        &mut self,
        surface: SurfaceId,
        metrics: UniformSurfaceMetrics,
    ) -> Result<(), DockspaceRuntimeError> {
        self.measure_surface_with(surface, |request| {
            Ok::<_, DockspaceRuntimeError>(match request {
                SurfaceMeasurementRequest::DockBounds { .. }
                | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                    SurfaceMeasurementAnswer::Bounds(metrics.bounds)
                }
                SurfaceMeasurementRequest::PaneSizing { .. } => {
                    SurfaceMeasurementAnswer::PaneSizing(metrics.pane_sizing)
                }
                SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                    SurfaceMeasurementAnswer::TabIntrinsic(metrics.tab_intrinsic.content_width())
                }
                SurfaceMeasurementRequest::TabStrip { .. } => {
                    SurfaceMeasurementAnswer::TabStrip(metrics.tab_strip)
                }
            })
        })
    }

    /// Resolves every exact surface measurement through product-level questions.
    ///
    /// Core owns deterministic traversal, structural identities, exact-set
    /// assembly, and validation. The resolver may return an explicit
    /// [`SurfaceMeasurementAnswer::Unavailable`] for any individual value, or an
    /// adapter error to stop measurement without compiling a partial contribution.
    /// This entry point does not reserve output capacity or begin an encoding pass.
    ///
    /// # Errors
    ///
    /// Returns an error when the surface is outside the frozen roster, already
    /// answered, an answer variant does not match its request, or the resulting
    /// exact contribution is invalid. Core errors are converted into `E`. The first
    /// resolver error is returned unchanged; earlier answers from that invocation
    /// are discarded. Dropping the host frame rolls the whole candidate back.
    pub fn measure_surface_with<E>(
        &mut self,
        surface: SurfaceId,
        mut resolve: impl FnMut(SurfaceMeasurementRequest) -> Result<SurfaceMeasurementAnswer, E>,
    ) -> Result<(), E>
    where
        E: From<DockspaceRuntimeError>,
    {
        let core_error = |error: DockspaceRuntimeError| E::from(error);
        self.complete_pointer_journal()
            .map_err(|error| core_error(error.into()))?;
        if self.surface_answered(surface) {
            return Err(core_error(
                DockspaceInteractionError::SurfaceAlreadyAnswered { surface }.into(),
            ));
        }
        self.frame
            .refresh_presentation_layout_except(Some(surface))
            .map_err(|error| core_error(error.into()))?;
        let requirements = self
            .frame
            .view()
            .presentation_requirements()
            .surface(surface)
            .ok_or(DockspaceInteractionError::SurfaceOutsideRoster { surface })
            .map_err(|error| core_error(error.into()))?;
        let mut measurements = SurfaceMeasurements::new(requirements.ticket());
        measurements
            .set_bounds(
                requirements.bounds(),
                resolve(SurfaceMeasurementRequest::DockBounds { surface })?
                    .into_bounds()
                    .map_err(|error| core_error(error.into()))?,
            )
            .map_err(|_| {
                core_error(DockspaceInteractionError::MeasurementRosterInvariant.into())
            })?;
        if let Some(key) = requirements.popup_plane_bounds() {
            measurements
                .set_popup_plane_bounds(
                    key,
                    resolve(SurfaceMeasurementRequest::PopupPlaneBounds { surface })?
                        .into_bounds()
                        .map_err(|error| core_error(error.into()))?,
                )
                .map_err(|_| {
                    core_error(DockspaceInteractionError::MeasurementRosterInvariant.into())
                })?;
        }
        for key in requirements.pane_sizings() {
            let visual = DockspaceVisualId::from_pane_scene(PaneSceneId {
                root: key.root(),
                item: key.item(),
            });
            measurements
                .insert_pane_sizing(
                    key,
                    resolve(SurfaceMeasurementRequest::PaneSizing {
                        visual,
                        item: Some(key.item()),
                    })?
                    .into_pane_sizing()
                    .map_err(|error| core_error(error.into()))?,
                )
                .map_err(|_| {
                    core_error(DockspaceInteractionError::MeasurementRosterInvariant.into())
                })?;
        }
        for key in requirements.tab_intrinsics() {
            let tab = key.tab();
            measurements
                .insert_tab_intrinsic(
                    key,
                    resolve(SurfaceMeasurementRequest::TabIntrinsic {
                        visual: DockspaceVisualId::from_tab_scene(tab),
                        bar: DockspaceVisualId::from_tab_bar_scene(
                            crate::presentation::scene::TabBarSceneId {
                                root: tab.root,
                                tabs: tab.tabs,
                            },
                        ),
                        key: tab.tab,
                        label: requirements
                            .tab_label(tab)
                            .ok_or(DockspaceInteractionError::MeasurementRosterInvariant)
                            .map_err(|error| core_error(error.into()))?,
                    })?
                    .into_tab_intrinsic()
                    .map_err(|error| core_error(error.into()))?,
                )
                .map_err(|_| {
                    core_error(DockspaceInteractionError::MeasurementRosterInvariant.into())
                })?;
        }
        for key in requirements.tab_strips() {
            let bar = key.bar();
            let visibility = requirements
                .tab_bar(bar)
                .ok_or(DockspaceInteractionError::MeasurementRosterInvariant)
                .map_err(|error| core_error(error.into()))?
                .policy()
                .visibility();
            let visibility = if visibility == TabBarVisibility::AutoHideSingle {
                TabBarVisibility::Visible
            } else {
                visibility
            };
            measurements
                .insert_tab_strip(
                    key,
                    resolve(SurfaceMeasurementRequest::TabStrip {
                        visual: DockspaceVisualId::from_tab_bar_scene(bar),
                        visibility,
                    })?
                    .into_tab_strip()
                    .map_err(|error| core_error(error.into()))?,
                )
                .map_err(|_| {
                    core_error(DockspaceInteractionError::MeasurementRosterInvariant.into())
                })?;
        }
        let token = self
            .frame
            .view()
            .begin_surface_contribution(surface)
            .map_err(|error| core_error(error.into()))?;
        let contribution = self
            .frame
            .view()
            .prepare_surface_contribution(token, measurements)
            .map_err(|error| core_error(error.into()))?;
        self.frame
            .compile_surface_contribution(contribution)
            .map_err(|error| core_error(error.into()))?;
        Ok(())
    }

    /// Prepares one surface for measurement and painting in the current encoding pass.
    ///
    /// This convenience entry point owns the repeated host decision of whether a
    /// surface may refresh its measurements. It first admits the surface's output,
    /// then measures only an unanswered unmeasured surface or a ready surface whose
    /// [`SurfaceMeasurementRefresh`](super::SurfaceMeasurementRefresh) is
    /// [`Allowed`](super::SurfaceMeasurementRefresh::Allowed).
    /// A Ready plan with a retained gesture or preview keeps its measured geometry.
    /// An Unmeasured surface still requests measurements; an active resize projects
    /// its frozen track shares into the newly offered bounds when its direct Grid
    /// cell maps remain compatible. A changed direct map cancels the complete resize
    /// and uses durable shares. Output backpressure
    /// returns [`SurfaceWorkDeferred::OutputCapacity`] without invoking the resolver.
    /// A surface answered earlier in the same pass is also retained; its resolver
    /// is not called twice.
    ///
    /// To emit an output, execute the returned ready plan's complete program and
    /// call [`DockspaceHostFrame::confirm_surface_painted`] before committing.
    /// Frames that retain their existing output may commit without painting.
    /// This method seals input and starts a retry checkpoint; use
    /// [`Self::restart_encoding_pass`] to retry encoding without replaying input.
    ///
    /// # Errors
    ///
    /// Returns the adapter error from `resolve`, or a converted core runtime error.
    /// Dropping the host frame rolls back any measurement contribution staged here.
    pub fn prepare_surface_with<E>(
        &mut self,
        surface: SurfaceId,
        mut resolve: impl FnMut(SurfaceMeasurementRequest) -> Result<SurfaceMeasurementAnswer, E>,
    ) -> Result<SurfaceWork<'_>, E>
    where
        E: From<DockspaceRuntimeError>,
    {
        self.begin_encoding_pass().map_err(E::from)?;
        let should_measure = match self.surface_work(surface).map_err(E::from)? {
            SurfaceWork::Ready(plan) => {
                let allowed = plan.measurement_refresh().is_allowed();
                !self.surface_answered(surface) && allowed
            }
            SurfaceWork::Deferred(SurfaceWorkDeferred::Unmeasured) => {
                !self.surface_answered(surface)
            }
            SurfaceWork::Deferred(SurfaceWorkDeferred::OutputCapacity) => {
                return Ok(SurfaceWork::Deferred(SurfaceWorkDeferred::OutputCapacity));
            }
        };
        if should_measure {
            self.measure_surface_with(surface, &mut resolve)?;
        }
        self.surface_work(surface).map_err(E::from)
    }

    /// Explicitly settles every unanswered surface by retaining a current
    /// Ready candidate or reporting the supplied unavailability reason.
    ///
    /// With [`SurfaceUnavailableReason::Deferred`], this only completes pending
    /// pointer input. [`Self::commit`] already performs that step; an explicit
    /// call is useful only when input must finish before further frame work.
    ///
    /// # Errors
    ///
    /// Returns an error when any contribution is stale, duplicated, or cannot
    /// satisfy the frozen surface roster.
    pub fn complete_unpainted_surfaces(
        &mut self,
        reason: SurfaceUnavailableReason,
    ) -> Result<(), DockspaceRuntimeError> {
        self.complete_pointer_input()?;
        if reason == SurfaceUnavailableReason::Deferred {
            return Ok(());
        }
        let surfaces = self.frame.surfaces().collect::<Vec<_>>();
        for surface in surfaces {
            if self.surface_answered(surface) {
                continue;
            }
            let token = self.frame.view().begin_surface_contribution(surface)?;
            let contribution = if matches!(
                self.frame.view().scene().surface(surface),
                Some(SurfaceScene::Ready(_))
            ) {
                self.frame
                    .view()
                    .prepare_surface_retained_contribution(token)?
            } else {
                self.frame
                    .view()
                    .prepare_surface_unavailable_contribution(token, reason.into())?
            };
            self.frame.push_surface_contribution(contribution)?;
        }
        Ok(())
    }
}
