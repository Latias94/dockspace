//! Numeric reprojection for ordered resizing and Auto shape validation.

use super::*;

#[cfg(test)]
thread_local! {
    static AUTO_REFLOW_PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn auto_reflow_projection_count() -> usize {
    AUTO_REFLOW_PROJECTIONS.get()
}

/// Immutable numeric inputs, without workspace or gesture authority.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenRootLayout {
    input: Arc<RootLayoutInput>,
    weights: BTreeMap<NodeId, TrackWeights>,
}

#[derive(Debug, PartialEq)]
struct RootLayoutInput {
    nodes: NodeMap<NodeResult>,
    bounds: LogicalRect,
    basis: ProjectionBasis,
    minimums_only: bool,
    leaf_sizing: Option<BTreeMap<NodeId, PaneSizing>>,
}

impl FrozenRootLayout {
    pub(crate) fn topology(&self) -> &RootStructure {
        &self.input.nodes.topology
    }

    pub(crate) fn splitter_thickness(&self) -> f64 {
        self.input.basis.metrics.splitter_thickness()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture(
        topology: &Arc<RootStructure>,
        values: &[NodeResult],
        bounds: LogicalRect,
        basis: &ProjectionBasis,
        minimums_only: bool,
        leaf_sizing: Option<&BTreeMap<NodeId, PaneSizing>>,
        weights: &BTreeMap<NodeId, TrackWeights>,
    ) -> Option<Arc<Self>> {
        topology
            .grid_rows
            .iter()
            .any(|&row| {
                matches!(
                    &topology.nodes[row].kind,
                    RootNodeKind::Grid { payload, .. }
                        if payload.shape() == crate::model::DockspaceGridShape::Auto
                )
            })
            .then(|| {
                Arc::new(Self {
                    input: Arc::new(RootLayoutInput {
                        nodes: NodeMap::from_values(Arc::clone(topology), values.to_vec()),
                        bounds,
                        basis: basis.clone(),
                        minimums_only,
                        leaf_sizing: leaf_sizing.cloned(),
                    }),
                    weights: weights.clone(),
                })
            })
    }

    /// Reuses the exact compiler after both resize axes have been merged.
    /// Sources are checked by the resize owner before and after this numeric step.
    pub(crate) fn project(
        &self,
        overrides: &BTreeMap<NodeId, TrackWeights>,
    ) -> Result<LayoutProjection, LayoutError> {
        #[cfg(test)]
        AUTO_REFLOW_PROJECTIONS.set(AUTO_REFLOW_PROJECTIONS.get() + 1);
        let mut weights = self.weights.clone();
        weights.extend(
            overrides
                .iter()
                .map(|(&node, weights)| (node, weights.clone())),
        );
        self.project_weights(&weights)
    }

    fn project_weights(
        &self,
        weights: &BTreeMap<NodeId, TrackWeights>,
    ) -> Result<LayoutProjection, LayoutError> {
        let input = &self.input;
        project_measurements_inner(
            Arc::clone(&input.nodes.topology),
            input.nodes.values.clone(),
            input.bounds,
            &input.basis,
            input.minimums_only,
            input.leaf_sizing.as_ref(),
            weights,
        )
    }
}

impl LayoutProjection {
    /// Reprojects this root using only numeric layout facts. The original
    /// surface supplies clipping and input authority separately.
    pub(crate) fn project_resize_step(
        &self,
        weights: &BTreeMap<NodeId, TrackWeights>,
    ) -> Result<Self, LayoutError> {
        if let Some(auto) = self.auto_reflow() {
            let mut current = (**auto).clone();
            current.weights.extend(
                weights
                    .iter()
                    .map(|(&node, weights)| (node, weights.clone())),
            );
            let mut projection = current.project_weights(&current.weights)?;
            projection.auto_reflow = Some(Arc::new(current));
            return Ok(projection);
        }

        // Fixed grids and splits keep their measured constraints and child map.
        // Inline tabs occupy one row regardless of how many items they contain.
        let mut projected = Self {
            nodes: self.nodes.clone(),
            splits: SplitMap::new(Arc::clone(&self.nodes.topology)),
            grids: GridMap::default(),
            auto_reflow: None,
            basis: self.basis.clone(),
        };
        self.basis
            .project_into(self.nodes.at_row(0).slot, weights, &mut projected, None)?;
        Ok(projected)
    }
}
