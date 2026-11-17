//! Exact compiler inputs retained for shape-changing Auto resize validation.

use super::*;

/// Immutable numeric inputs, without workspace or gesture authority.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenRootLayout {
    nodes: NodeMap<NodeResult>,
    central: CentralRoute,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
    minimums_only: bool,
    leaf_sizing: Option<BTreeMap<NodeId, PaneSizing>>,
    weights: BTreeMap<NodeId, TrackWeights>,
    tabs: BTreeMap<usize, (Option<usize>, f64)>,
}

impl FrozenRootLayout {
    pub(crate) fn splitter_thickness(&self) -> f64 {
        self.metrics.splitter_thickness()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture(
        topology: &Arc<RootStructure>,
        values: &[NodeResult],
        central: &CentralRoute,
        bounds: LogicalRect,
        metrics: LayoutMetrics,
        minimums_only: bool,
        leaf_sizing: Option<&BTreeMap<NodeId, PaneSizing>>,
        weights: &BTreeMap<NodeId, TrackWeights>,
        tabs: &BTreeMap<usize, (Option<usize>, f64)>,
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
                    nodes: NodeMap::from_values(Arc::clone(topology), values.to_vec()),
                    central: central.clone(),
                    bounds,
                    metrics,
                    minimums_only,
                    leaf_sizing: leaf_sizing.cloned(),
                    weights: weights.clone(),
                    tabs: tabs.clone(),
                })
            })
    }

    /// Reuses the exact compiler after both resize axes have been merged.
    /// Sources are checked by the resize owner before and after this numeric step.
    pub(crate) fn project(
        &self,
        overrides: &BTreeMap<NodeId, TrackWeights>,
    ) -> Result<LayoutProjection, LayoutError> {
        let mut weights = self.weights.clone();
        weights.extend(
            overrides
                .iter()
                .map(|(&node, weights)| (node, weights.clone())),
        );
        project_measurements_inner(
            Arc::clone(&self.nodes.topology),
            self.nodes.values.clone(),
            self.central.clone(),
            self.bounds,
            self.metrics,
            self.minimums_only,
            self.leaf_sizing.as_ref(),
            &weights,
            &self.tabs,
        )
    }
}
