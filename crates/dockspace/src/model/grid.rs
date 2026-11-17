use std::num::NonZeroUsize;

use thiserror::Error;

use super::DockspaceAxis;
use crate::graph::{
    InvalidSplitWeights, NORMALIZED_WEIGHT_TOLERANCE, SplitWeight, normalized_weight_sum,
};

/// Declared dimensions of a grid with shared row and column tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DockspaceGridShape {
    /// Uses explicit, nonzero row and column counts.
    Fixed {
        /// Number of shared row tracks.
        rows: NonZeroUsize,
        /// Number of shared column tracks.
        columns: NonZeroUsize,
    },
    /// Resolves dimensions from the child count and offered layout bounds.
    Auto,
}

impl DockspaceGridShape {
    pub(crate) fn validate_children(self, children: usize) -> Result<(), DockspaceGridError> {
        if children == 0 {
            return Err(DockspaceGridError::EmptyChildren);
        }
        if let Self::Fixed { rows, columns } = self {
            let capacity = rows.get().checked_mul(columns.get()).ok_or(
                DockspaceGridError::CapacityOverflow {
                    rows: rows.get(),
                    columns: columns.get(),
                },
            )?;
            if children > capacity {
                return Err(DockspaceGridError::TooManyChildren { children, capacity });
            }
            if children < columns.get() {
                return Err(DockspaceGridError::UnoccupiedTrack {
                    axis: DockspaceAxis::Horizontal,
                    index: children,
                });
            }
            let last_row_start = (rows.get() - 1) * columns.get();
            if children <= last_row_start {
                return Err(DockspaceGridError::UnoccupiedTrack {
                    axis: DockspaceAxis::Vertical,
                    index: children.div_ceil(columns.get()),
                });
            }
        }
        Ok(())
    }
}

/// Positive, finite track shares normalized independently for each grid axis.
///
/// The row and column counts are carried by their respective share sequences.
/// Matching these counts to a grid shape belongs to layout declaration validation.
#[derive(Debug, Clone, PartialEq)]
pub struct DockspaceGridShares {
    rows: Vec<SplitWeight>,
    columns: Vec<SplitWeight>,
}

impl DockspaceGridShares {
    /// Normalizes nonempty row and column shares using the core track weight validator.
    ///
    /// # Errors
    ///
    /// Returns [`DockspaceGridError::InvalidRowShares`] or
    /// [`DockspaceGridError::InvalidColumnShares`] when the corresponding axis is empty,
    /// contains a nonpositive or nonfinite value, or cannot be normalized to positive
    /// finite shares.
    pub fn new(
        rows: impl IntoIterator<Item = f64>,
        columns: impl IntoIterator<Item = f64>,
    ) -> Result<Self, DockspaceGridError> {
        let rows = SplitWeight::normalize(rows)
            .map_err(|source| DockspaceGridError::InvalidRowShares { source })?;
        let columns = SplitWeight::normalize(columns)
            .map_err(|source| DockspaceGridError::InvalidColumnShares { source })?;
        Ok(Self { rows, columns })
    }

    /// Visits normalized row shares from top to bottom.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = f64> + '_ {
        self.rows.iter().map(|weight| weight.get())
    }

    /// Visits normalized column shares from left to right.
    pub fn columns(&self) -> impl ExactSizeIterator<Item = f64> + '_ {
        self.columns.iter().map(|weight| weight.get())
    }

    pub(crate) fn row_weights(&self) -> &[SplitWeight] {
        &self.rows
    }

    pub(crate) fn column_weights(&self) -> &[SplitWeight] {
        &self.columns
    }

    pub(crate) fn from_normalized(
        rows: Vec<SplitWeight>,
        columns: Vec<SplitWeight>,
    ) -> Result<Self, DockspaceGridError> {
        for (axis, weights) in [
            (DockspaceAxis::Vertical, rows.as_slice()),
            (DockspaceAxis::Horizontal, columns.as_slice()),
        ] {
            let invalid = |source| match axis {
                DockspaceAxis::Vertical => DockspaceGridError::InvalidRowShares { source },
                DockspaceAxis::Horizontal => DockspaceGridError::InvalidColumnShares { source },
            };
            if weights.is_empty() {
                return Err(invalid(InvalidSplitWeights::Empty));
            }
            for (index, weight) in weights.iter().enumerate() {
                SplitWeight::new(weight.get())
                    .map_err(|source| invalid(InvalidSplitWeights::Invalid { index, source }))?;
            }
            let sum = normalized_weight_sum(weights.iter().map(|weight| weight.get()));
            if !sum.is_finite() || (sum - 1.0).abs() > NORMALIZED_WEIGHT_TOLERANCE {
                return Err(DockspaceGridError::SharesNotNormalized { axis, sum });
            }
        }
        Ok(Self { rows, columns })
    }

    pub(crate) fn validate_for(
        &self,
        shape: DockspaceGridShape,
        children: usize,
    ) -> Result<(), DockspaceGridError> {
        shape.validate_children(children)?;
        if let DockspaceGridShape::Fixed { rows, columns } = shape {
            if rows.get() != self.rows.len() || columns.get() != self.columns.len() {
                return Err(DockspaceGridError::ShareShapeMismatch {
                    expected_rows: rows.get(),
                    expected_columns: columns.get(),
                    rows: self.rows.len(),
                    columns: self.columns.len(),
                });
            }
        } else {
            let rows =
                NonZeroUsize::new(self.rows.len()).ok_or(DockspaceGridError::InvalidRowShares {
                    source: InvalidSplitWeights::Empty,
                })?;
            let columns = NonZeroUsize::new(self.columns.len()).ok_or(
                DockspaceGridError::InvalidColumnShares {
                    source: InvalidSplitWeights::Empty,
                },
            )?;
            DockspaceGridShape::Fixed { rows, columns }.validate_children(children)?;
        }
        Ok(())
    }
}

/// Failure to construct checked grid track values.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DockspaceGridError {
    /// A grid requires at least one real structural child.
    #[error("a grid requires at least one child")]
    EmptyChildren,
    /// The declared rectangle cannot be represented on this platform.
    #[error("grid capacity overflows for {rows} rows and {columns} columns")]
    CapacityOverflow {
        /// Declared row count.
        rows: usize,
        /// Declared column count.
        columns: usize,
    },
    /// The durable roster does not fit the declared rectangle.
    #[error("grid has {children} children but capacity is {capacity}")]
    TooManyChildren {
        /// Supplied real child count.
        children: usize,
        /// Number of cells in the fixed rectangle.
        capacity: usize,
    },
    /// A declared track has no durable child slot.
    #[error("grid {axis:?} track {index} has no child")]
    UnoccupiedTrack {
        /// Horizontal columns or vertical rows.
        axis: DockspaceAxis,
        /// First track without a durable child slot.
        index: usize,
    },
    /// Track counts disagree with fixed dimensions.
    #[error("grid shares are {rows}x{columns}, expected {expected_rows}x{expected_columns}")]
    ShareShapeMismatch {
        /// Declared fixed row count.
        expected_rows: usize,
        /// Declared fixed column count.
        expected_columns: usize,
        /// Supplied row share count.
        rows: usize,
        /// Supplied column share count.
        columns: usize,
    },
    /// Strict persistent shares were not normalized before decoding.
    #[error("grid {axis:?} shares must sum to one, got {sum}")]
    SharesNotNormalized {
        /// Axis whose original persistent shares failed validation.
        axis: DockspaceAxis,
        /// Compensated sum of the original shares.
        sum: f64,
    },
    /// The row axis could not be normalized.
    #[error("invalid grid row shares: {source}")]
    InvalidRowShares {
        /// Failure reported by the shared track weight validator.
        source: InvalidSplitWeights,
    },
    /// The column axis could not be normalized.
    #[error("invalid grid column shares: {source}")]
    InvalidColumnShares {
        /// Failure reported by the shared track weight validator.
        source: InvalidSplitWeights,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InvalidSplitWeight;

    #[test]
    fn row_and_column_shares_normalize_independently_and_keep_their_counts() {
        let shares = DockspaceGridShares::new([1.0, 3.0], [2.0, 3.0, 5.0]).unwrap();
        assert_eq!(shares.rows().len(), 2);
        assert_eq!(shares.columns().len(), 3);
        assert_eq!(shares.rows().collect::<Vec<_>>(), [0.25, 0.75]);
        assert_eq!(shares.columns().collect::<Vec<_>>(), [0.2, 0.3, 0.5]);
        let independent = DockspaceGridShares::new([10.0, 30.0], [0.2, 0.3, 0.5]).unwrap();
        assert_eq!(shares, independent);
    }

    #[test]
    fn invalid_shares_report_their_axis_and_original_validation_failure() {
        assert_eq!(
            DockspaceGridShares::new([], [1.0]),
            Err(DockspaceGridError::InvalidRowShares {
                source: InvalidSplitWeights::Empty,
            })
        );
        assert_eq!(
            DockspaceGridShares::new([1.0], []),
            Err(DockspaceGridError::InvalidColumnShares {
                source: InvalidSplitWeights::Empty,
            })
        );
        assert_eq!(
            DockspaceGridShares::new([1.0], [1.0, -2.0]),
            Err(DockspaceGridError::InvalidColumnShares {
                source: InvalidSplitWeights::Invalid {
                    index: 1,
                    source: InvalidSplitWeight::NonPositive { value: -2.0 },
                },
            })
        );
        assert_eq!(
            DockspaceGridShares::new([f64::MAX, f64::MAX], [1.0]),
            Err(DockspaceGridError::InvalidRowShares {
                source: InvalidSplitWeights::InvalidSum,
            })
        );
        assert_eq!(
            DockspaceGridShares::new([1.0], [f64::from_bits(1), 4.0]),
            Err(DockspaceGridError::InvalidColumnShares {
                source: InvalidSplitWeights::Invalid {
                    index: 0,
                    source: InvalidSplitWeight::NonPositive { value: 0.0 },
                },
            })
        );
        let invalid = DockspaceGridShares::new([f64::INFINITY], [1.0]).unwrap_err();
        assert!(matches!(
            invalid,
            DockspaceGridError::InvalidRowShares {
                source: InvalidSplitWeights::Invalid {
                    index: 0,
                    source: InvalidSplitWeight::NonFinite {
                        value: f64::INFINITY
                    },
                },
            }
        ));
        assert!(std::error::Error::source(&invalid).is_some());
    }
}
