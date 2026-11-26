//! Ergonomic surface measurement helpers for third-party UI frameworks.
//!
//! Implement [`SurfaceMeasurer`] or configure [`SurfaceMetrics`] to answer core
//! [`SurfaceMeasurementRequest`] queries without hand-matching every request variant.

use crate::geometry::{LogicalRect, LogicalSize, PaneSizing};
use crate::model::{ItemId, SurfaceId, TabKey, TabLabel};
use crate::policy::TabBarVisibility;
use crate::runtime::{
    DockspaceInteractionError, DockspaceRuntimeError, DockspaceVisualId, SurfaceMeasurementAnswer,
    SurfaceMeasurementRequest, TabStripMetrics,
};

/// Default minimum pane extent used by [`SurfaceMetrics::new`].
const DEFAULT_MIN_PANE_WIDTH: f64 = 64.0;
const DEFAULT_MIN_PANE_HEIGHT: f64 = 48.0;
/// Default tab content width used by [`SurfaceMetrics::new`].
const DEFAULT_TAB_CONTENT_WIDTH: f64 = 96.0;

/// Reusable logical measurement defaults for one surface.
///
/// Start with [`Self::new`] for quick bring-up, then customize tab-title shaping
/// with [`Self::with_tab_width`] or implement [`SurfaceMeasurer`] when pane
/// sizing, popup planes, or tab-strip chrome vary dynamically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceMetrics {
    bounds: LogicalRect,
    popup_bounds: Option<LogicalRect>,
    pane_sizing: PaneSizing,
    tab_content_width: f64,
    tab_strip: TabStripMetrics,
}

impl SurfaceMetrics {
    /// Creates logical metrics for `bounds` with sensible fallback defaults
    /// (`64x48` minimum pane size, `96.0` tab content width, zero extra strip chrome).
    #[must_use]
    pub fn new(bounds: LogicalRect) -> Self {
        let min_size = LogicalSize::new(DEFAULT_MIN_PANE_WIDTH, DEFAULT_MIN_PANE_HEIGHT)
            .expect("positive finite default minimum pane size");
        let tab_strip = TabStripMetrics::new(0.0, 0.0).expect("zero tab strip chrome is valid");
        Self {
            bounds,
            popup_bounds: None,
            pane_sizing: min_size.into(),
            tab_content_width: DEFAULT_TAB_CONTENT_WIDTH,
            tab_strip,
        }
    }

    /// Sets an independent popup plane rectangle (defaults to `bounds`).
    #[must_use]
    pub const fn with_popup_bounds(mut self, bounds: LogicalRect) -> Self {
        self.popup_bounds = Some(bounds);
        self
    }

    /// Sets the default pane sizing declaration returned for pane sizing queries.
    #[must_use]
    pub fn with_pane_sizing(mut self, sizing: impl Into<PaneSizing>) -> Self {
        self.pane_sizing = sizing.into();
        self
    }

    /// Sets the default tab content width before core padding.
    ///
    /// # Errors
    ///
    /// Returns [`DockspaceInteractionError::InvalidMeasurementProfile`] when
    /// `width` is negative or non-finite.
    pub fn with_default_tab_width(mut self, width: f64) -> Result<Self, DockspaceInteractionError> {
        if !width.is_finite() || width < 0.0 {
            return Err(DockspaceInteractionError::InvalidMeasurementProfile);
        }
        self.tab_content_width = width;
        Ok(self)
    }

    /// Sets renderer-owned leading/trailing chrome metrics for tab strips.
    #[must_use]
    pub const fn with_tab_strip(mut self, metrics: TabStripMetrics) -> Self {
        self.tab_strip = metrics;
        self
    }

    /// Returns the configured dock layout bounds.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.bounds
    }

    /// Returns the popup plane bounds (falling back to [`Self::bounds`]).
    #[must_use]
    pub const fn popup_bounds(self) -> LogicalRect {
        match self.popup_bounds {
            Some(bounds) => bounds,
            None => self.bounds,
        }
    }

    /// Returns a closure suitable for [`crate::runtime::DockspaceHostFrame::prepare_surface_with`]
    /// or [`crate::host::LogicalHost::paint_display_list`].
    #[must_use]
    pub fn resolver(
        mut self,
    ) -> impl FnMut(
        SurfaceMeasurementRequest<'_>,
    ) -> Result<SurfaceMeasurementAnswer, DockspaceRuntimeError> {
        move |request| self.measure_request(request)
    }

    /// Returns a resolver closure that delegates tab title width measurement to
    /// `measure_tab` while answering bounds, pane sizing, and strip chrome from `self`.
    #[must_use]
    pub fn with_tab_width<F>(
        mut self,
        mut measure_tab: F,
    ) -> impl FnMut(
        SurfaceMeasurementRequest<'_>,
    ) -> Result<SurfaceMeasurementAnswer, DockspaceRuntimeError>
    where
        F: FnMut(TabLabel<'_>) -> f64,
    {
        move |request| match request {
            SurfaceMeasurementRequest::TabIntrinsic { label, .. } => {
                Ok(SurfaceMeasurementAnswer::TabIntrinsic(measure_tab(label)))
            }
            other => self.measure_request(other),
        }
    }
}

/// Trait-based surface measurement adapter for UI frameworks.
///
/// Only [`Self::dock_bounds`] is required; every other method has a sensible
/// default so integrators can start minimal and specialize text shaping,
/// per-pane minimum sizes, and tab-strip chrome incrementally.
pub trait SurfaceMeasurer {
    /// Returns the logical dock layout bounds for `surface`.
    fn dock_bounds(&mut self, surface: SurfaceId) -> LogicalRect;

    /// Returns the popup plane bounds for `surface` (defaults to [`Self::dock_bounds`]).
    fn popup_bounds(&mut self, surface: SurfaceId) -> LogicalRect {
        self.dock_bounds(surface)
    }

    /// Returns the pane sizing declaration for one pane visual/item.
    fn pane_sizing(&mut self, _visual: DockspaceVisualId, _item: Option<ItemId>) -> PaneSizing {
        LogicalSize::new(DEFAULT_MIN_PANE_WIDTH, DEFAULT_MIN_PANE_HEIGHT)
            .expect("positive finite default minimum pane size")
            .into()
    }

    /// Returns the tab content width answer (or independent header/menu widths)
    /// for one tab label.
    fn tab_intrinsic(
        &mut self,
        _visual: DockspaceVisualId,
        _bar: DockspaceVisualId,
        _key: TabKey,
        _label: TabLabel<'_>,
    ) -> SurfaceMeasurementAnswer {
        SurfaceMeasurementAnswer::TabIntrinsic(DEFAULT_TAB_CONTENT_WIDTH)
    }

    /// Returns renderer-owned tab-strip chrome metrics for one bar.
    fn tab_strip(
        &mut self,
        _visual: DockspaceVisualId,
        _visibility: TabBarVisibility,
    ) -> TabStripMetrics {
        TabStripMetrics::new(0.0, 0.0).expect("zero tab strip chrome is valid")
    }

    /// Answers a single core [`SurfaceMeasurementRequest`] through this measurer.
    ///
    /// # Errors
    ///
    /// Never fails by default; returns `Result` so it plugs directly into
    /// [`crate::runtime::DockspaceHostFrame::prepare_surface_with`].
    fn measure_request(
        &mut self,
        request: SurfaceMeasurementRequest<'_>,
    ) -> Result<SurfaceMeasurementAnswer, DockspaceRuntimeError> {
        Ok(match request {
            SurfaceMeasurementRequest::DockBounds { surface } => {
                SurfaceMeasurementAnswer::Bounds(self.dock_bounds(surface))
            }
            SurfaceMeasurementRequest::PopupPlaneBounds { surface } => {
                SurfaceMeasurementAnswer::Bounds(self.popup_bounds(surface))
            }
            SurfaceMeasurementRequest::PaneSizing { visual, item } => {
                SurfaceMeasurementAnswer::PaneSizing(self.pane_sizing(visual, item))
            }
            SurfaceMeasurementRequest::TabIntrinsic {
                visual,
                bar,
                key,
                label,
            } => self.tab_intrinsic(visual, bar, key, label),
            SurfaceMeasurementRequest::TabStrip { visual, visibility } => {
                SurfaceMeasurementAnswer::TabStrip(self.tab_strip(visual, visibility))
            }
        })
    }
}

impl SurfaceMeasurer for SurfaceMetrics {
    fn dock_bounds(&mut self, _surface: SurfaceId) -> LogicalRect {
        self.bounds
    }

    fn popup_bounds(&mut self, _surface: SurfaceId) -> LogicalRect {
        Self::popup_bounds(*self)
    }

    fn pane_sizing(&mut self, _visual: DockspaceVisualId, _item: Option<ItemId>) -> PaneSizing {
        self.pane_sizing
    }

    fn tab_intrinsic(
        &mut self,
        _visual: DockspaceVisualId,
        _bar: DockspaceVisualId,
        _key: TabKey,
        _label: TabLabel<'_>,
    ) -> SurfaceMeasurementAnswer {
        SurfaceMeasurementAnswer::TabIntrinsic(self.tab_content_width)
    }

    fn tab_strip(
        &mut self,
        _visual: DockspaceVisualId,
        _visibility: TabBarVisibility,
    ) -> TabStripMetrics {
        self.tab_strip
    }
}

impl<T: SurfaceMeasurer + ?Sized> SurfaceMeasurer for &mut T {
    fn dock_bounds(&mut self, surface: SurfaceId) -> LogicalRect {
        (**self).dock_bounds(surface)
    }

    fn popup_bounds(&mut self, surface: SurfaceId) -> LogicalRect {
        (**self).popup_bounds(surface)
    }

    fn pane_sizing(&mut self, visual: DockspaceVisualId, item: Option<ItemId>) -> PaneSizing {
        (**self).pane_sizing(visual, item)
    }

    fn tab_intrinsic(
        &mut self,
        visual: DockspaceVisualId,
        bar: DockspaceVisualId,
        key: TabKey,
        label: TabLabel<'_>,
    ) -> SurfaceMeasurementAnswer {
        (**self).tab_intrinsic(visual, bar, key, label)
    }

    fn tab_strip(
        &mut self,
        visual: DockspaceVisualId,
        visibility: TabBarVisibility,
    ) -> TabStripMetrics {
        (**self).tab_strip(visual, visibility)
    }
}
