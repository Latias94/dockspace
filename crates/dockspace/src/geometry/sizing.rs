//! Validated application-owned pane content sizing.

use super::LogicalSize;

/// Invalid application sizing metadata for one pane content axis.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PaneSizingError {
    /// A minimum, maximum, or preferred extent was NaN or infinite.
    #[error("pane sizing `{field}` must be finite, got {value}")]
    NonFinite {
        /// Name of the rejected extent.
        field: &'static str,
        /// Rejected extent.
        value: f64,
    },
    /// A minimum, maximum, or preferred extent was negative.
    #[error("pane sizing `{field}` must be non-negative, got {value}")]
    Negative {
        /// Name of the rejected extent.
        field: &'static str,
        /// Rejected extent.
        value: f64,
    },
    /// An explicit maximum was smaller than the minimum.
    #[error("pane sizing minimum {minimum} exceeds maximum {maximum}")]
    MinimumExceedsMaximum {
        /// Rejected minimum.
        minimum: f64,
        /// Rejected maximum.
        maximum: f64,
    },
    /// A preferred extent was outside the application's declared range.
    #[error(
        "preferred pane extent {preferred} is outside minimum {minimum} and maximum {maximum:?}"
    )]
    PreferredOutOfRange {
        /// Rejected preferred extent.
        preferred: f64,
        /// Declared minimum.
        minimum: f64,
        /// Declared maximum, or no upper bound.
        maximum: Option<f64>,
    },
}

/// Validated minimum, optional maximum, and preferred pane content extent.
///
/// Values use renderer-independent logical points. `None` is an unbounded
/// maximum, not an infinite scalar. A preferred extent is an explicit reset
/// target; it does not continuously resize the pane or replace stored weights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxisSizing {
    minimum: f64,
    maximum: Option<f64>,
    preferred: Option<f64>,
}

impl AxisSizing {
    /// Validates one complete application content-axis declaration.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-finite or negative extent, an inverted range,
    /// or a preferred extent outside the declared range. Renderer style floors
    /// are applied later and do not affect validation of this declaration.
    pub fn new(
        minimum: f64,
        maximum: Option<f64>,
        preferred: Option<f64>,
    ) -> Result<Self, PaneSizingError> {
        for (field, value) in [
            ("minimum", Some(minimum)),
            ("maximum", maximum),
            ("preferred", preferred),
        ] {
            let Some(value) = value else {
                continue;
            };
            if !value.is_finite() {
                return Err(PaneSizingError::NonFinite { field, value });
            }
            if value < 0.0 {
                return Err(PaneSizingError::Negative { field, value });
            }
        }
        if let Some(maximum) = maximum
            && minimum > maximum
        {
            return Err(PaneSizingError::MinimumExceedsMaximum { minimum, maximum });
        }
        if let Some(preferred) = preferred
            && (preferred < minimum || maximum.is_some_and(|maximum| preferred > maximum))
        {
            return Err(PaneSizingError::PreferredOutOfRange {
                preferred,
                minimum,
                maximum,
            });
        }
        Ok(Self {
            minimum,
            maximum,
            preferred,
        })
    }

    /// Returns the minimum content extent.
    #[must_use]
    pub const fn minimum(self) -> f64 {
        self.minimum
    }

    /// Returns the maximum content extent, or no upper bound.
    #[must_use]
    pub const fn maximum(self) -> Option<f64> {
        self.maximum
    }

    /// Returns the optional explicit reset target.
    #[must_use]
    pub const fn preferred(self) -> Option<f64> {
        self.preferred
    }
}

/// Validated content sizing for the horizontal and vertical axes of one pane.
///
/// Tab bars, floating titles, and borders are added by core presentation.
/// Defaults impose no application minimum, maximum, or preferred extent;
/// renderer style supplies the ordinary minimum floor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneSizing {
    width: AxisSizing,
    height: AxisSizing,
}

impl PaneSizing {
    /// Combines two already validated content axes.
    #[must_use]
    pub const fn new(width: AxisSizing, height: AxisSizing) -> Self {
        Self { width, height }
    }

    /// Starts a content sizing declaration with both axes unconstrained.
    #[must_use]
    pub fn builder() -> PaneSizingBuilder {
        PaneSizingBuilder::default()
    }

    /// Declares content minimums without maximums or preferred reset targets.
    #[must_use]
    pub const fn from_minimum(minimum: LogicalSize) -> Self {
        Self {
            width: AxisSizing {
                minimum: minimum.width(),
                maximum: None,
                preferred: None,
            },
            height: AxisSizing {
                minimum: minimum.height(),
                maximum: None,
                preferred: None,
            },
        }
    }

    /// Returns horizontal content sizing.
    #[must_use]
    pub const fn width(self) -> AxisSizing {
        self.width
    }

    /// Returns vertical content sizing.
    #[must_use]
    pub const fn height(self) -> AxisSizing {
        self.height
    }

    /// Returns the already validated application content minimums.
    #[must_use]
    pub const fn minimum_size(self) -> LogicalSize {
        LogicalSize {
            width: self.width.minimum(),
            height: self.height.minimum(),
        }
    }
}

impl From<LogicalSize> for PaneSizing {
    fn from(minimum: LogicalSize) -> Self {
        Self::from_minimum(minimum)
    }
}

/// Builds pane content sizing, validating the final declaration in [`Self::build`].
///
/// Each setter replaces its named value. A fixed setter replaces both minimum
/// and maximum, preserving any preferred value. Later setters can change either
/// bound again; only the resulting declaration is validated.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneSizingBuilder {
    min_width: f64,
    max_width: Option<f64>,
    preferred_width: Option<f64>,
    min_height: f64,
    max_height: Option<f64>,
    preferred_height: Option<f64>,
}

impl PaneSizingBuilder {
    /// Sets the minimum content width in logical points.
    #[must_use]
    pub const fn min_width(mut self, minimum: f64) -> Self {
        self.min_width = minimum;
        self
    }

    /// Sets the maximum content width in logical points.
    #[must_use]
    pub const fn max_width(mut self, maximum: f64) -> Self {
        self.max_width = Some(maximum);
        self
    }

    /// Sets the content width used by an explicit preferred-size reset.
    #[must_use]
    pub const fn preferred_width(mut self, preferred: f64) -> Self {
        self.preferred_width = Some(preferred);
        self
    }

    /// Sets both content width bounds to the same extent.
    #[must_use]
    pub const fn fixed_width(mut self, extent: f64) -> Self {
        self.min_width = extent;
        self.max_width = Some(extent);
        self
    }

    /// Sets the minimum content height in logical points.
    #[must_use]
    pub const fn min_height(mut self, minimum: f64) -> Self {
        self.min_height = minimum;
        self
    }

    /// Sets the maximum content height in logical points.
    #[must_use]
    pub const fn max_height(mut self, maximum: f64) -> Self {
        self.max_height = Some(maximum);
        self
    }

    /// Sets the content height used by an explicit preferred-size reset.
    #[must_use]
    pub const fn preferred_height(mut self, preferred: f64) -> Self {
        self.preferred_height = Some(preferred);
        self
    }

    /// Sets both content height bounds to the same extent.
    #[must_use]
    pub const fn fixed_height(mut self, extent: f64) -> Self {
        self.min_height = extent;
        self.max_height = Some(extent);
        self
    }

    /// Validates both final content-axis declarations.
    ///
    /// # Errors
    ///
    /// Returns the first invalid width or height declaration according to
    /// [`AxisSizing::new`]. No partial sizing value is produced.
    pub fn build(self) -> Result<PaneSizing, PaneSizingError> {
        Ok(PaneSizing::new(
            AxisSizing::new(self.min_width, self.max_width, self.preferred_width)?,
            AxisSizing::new(self.min_height, self.max_height, self.preferred_height)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_minimum_only_sizing_keep_unbounded_axes() {
        let defaults = PaneSizing::default();
        assert_eq!(PaneSizing::builder().build().unwrap(), defaults);
        for axis in [defaults.width(), defaults.height()] {
            assert_eq!(axis.minimum().to_bits(), 0.0_f64.to_bits());
            assert_eq!(axis.maximum(), None);
            assert_eq!(axis.preferred(), None);
        }
        let minimum = LogicalSize::new(160.0, 90.0).unwrap();
        let sizing = PaneSizing::from_minimum(minimum);
        assert_eq!(PaneSizing::from(minimum), sizing);
        assert_eq!(sizing.minimum_size(), minimum);
        assert_eq!(sizing.width(), AxisSizing::new(160.0, None, None).unwrap());
        assert_eq!(sizing.height(), AxisSizing::new(90.0, None, None).unwrap());
    }

    #[test]
    fn builder_combines_independent_bounds_fixed_axes_and_preferred_targets() {
        let sizing = PaneSizing::builder()
            .min_width(160.0)
            .max_width(420.0)
            .preferred_width(280.0)
            .fixed_height(72.0)
            .preferred_height(72.0)
            .build()
            .unwrap();
        assert_eq!(
            sizing,
            PaneSizing::new(
                AxisSizing::new(160.0, Some(420.0), Some(280.0)).unwrap(),
                AxisSizing::new(72.0, Some(72.0), Some(72.0)).unwrap(),
            )
        );
    }

    #[test]
    fn later_builder_setters_replace_bounds_without_a_separate_fixed_mode() {
        let sizing = PaneSizing::builder()
            .fixed_width(120.0)
            .min_width(80.0)
            .preferred_width(100.0)
            .min_height(500.0)
            .max_height(10.0)
            .fixed_height(60.0)
            .max_height(90.0)
            .build()
            .unwrap();
        assert_eq!(
            sizing.width(),
            AxisSizing::new(80.0, Some(120.0), Some(100.0)).unwrap()
        );
        assert_eq!(
            sizing.height(),
            AxisSizing::new(60.0, Some(90.0), None).unwrap()
        );
        assert!(matches!(
            PaneSizing::builder()
                .preferred_width(100.0)
                .fixed_width(80.0)
                .build(),
            Err(PaneSizingError::PreferredOutOfRange { .. })
        ));
    }

    #[test]
    fn every_axis_scalar_rejects_non_finite_and_negative_values() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            for result in [
                AxisSizing::new(value, None, None),
                AxisSizing::new(0.0, Some(value), None),
                AxisSizing::new(0.0, None, Some(value)),
            ] {
                assert!(matches!(
                    result,
                    Err(PaneSizingError::NonFinite { .. } | PaneSizingError::Negative { .. })
                ));
            }
        }
    }

    #[test]
    fn declared_ranges_and_preferred_targets_are_validated_before_style() {
        assert!(matches!(
            PaneSizing::builder()
                .min_width(90.0)
                .max_width(80.0)
                .build(),
            Err(PaneSizingError::MinimumExceedsMaximum { .. })
        ));
        for preferred in [39.0, 81.0] {
            assert!(matches!(
                PaneSizing::builder()
                    .min_height(40.0)
                    .max_height(80.0)
                    .preferred_height(preferred)
                    .build(),
                Err(PaneSizingError::PreferredOutOfRange { .. })
            ));
        }
        for preferred in [40.0, 80.0] {
            assert!(AxisSizing::new(40.0, Some(80.0), Some(preferred)).is_ok());
        }
        assert!(AxisSizing::new(40.0, None, Some(200.0)).is_ok());
        assert!(
            PaneSizing::builder()
                .fixed_width(0.0)
                .fixed_height(0.0)
                .build()
                .is_ok()
        );
    }
}
