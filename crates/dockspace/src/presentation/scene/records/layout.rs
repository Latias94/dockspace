//! Retained renderer measurements needed for source-aware drop projection.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::geometry::{LogicalRect, PaneSizing};
use crate::ids::{FloatingPresentationId, NodeId, RootId, SurfaceId};
use crate::interaction::{ContainedTransformSessionId, DragSessionId};
use crate::policy::TabBarVisibility;
use crate::presentation::config::DockPresentationConfig;
use crate::presentation::layout::{LayoutProjection, NodeMap};
use crate::presentation::manifest::{
    AuthoritativeSurfaceMeasurements, PaneSizingKey, SurfaceMeasurements, TabIntrinsic,
    TabIntrinsicKey, TabStripKey, TabStripMetrics,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainedLayoutOwner {
    Drag(DragSessionId),
    Transform(ContainedTransformSessionId),
}

/// A transient root layout input, never a mutation of durable placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContainedLayoutProjection {
    pub(crate) owner: ContainedLayoutOwner,
    pub(crate) surface: SurfaceId,
    pub(crate) root: RootId,
    pub(crate) floating: FloatingPresentationId,
    pub(crate) source_rect: LogicalRect,
    pub(crate) rect: LogicalRect,
    /// Target geometry lineage preserved only by recompilation of identical measurements.
    pub(crate) target_basis: Option<super::super::SurfaceSceneStamp>,
}

impl ContainedLayoutProjection {
    pub(crate) fn drop_source_suppression(
        self,
    ) -> crate::interaction::targeting::targets::DropSourceSuppression {
        match self.owner {
            ContainedLayoutOwner::Drag(_) => {
                crate::interaction::targeting::targets::DropSourceSuppression::complete_root(
                    self.root,
                    Some(self.floating),
                )
            }
            ContainedLayoutOwner::Transform(_) => Default::default(),
        }
    }
}

/// Exact layout inputs retained with one compiled presentation plan.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PresentationLayoutFacts {
    config: DockPresentationConfig,
    roots: Arc<BTreeMap<RootId, RootLayoutFacts>>,
    measurements: Arc<SurfaceMeasurements>,
}

impl PresentationLayoutFacts {
    pub(crate) fn new(
        config: DockPresentationConfig,
        measurements: AuthoritativeSurfaceMeasurements<'_>,
    ) -> Self {
        Self {
            config,
            roots: Arc::new(BTreeMap::new()),
            measurements: Arc::new(measurements.snapshot()),
        }
    }

    pub(crate) const fn config(&self) -> &DockPresentationConfig {
        &self.config
    }

    pub(crate) fn root(&self, root: RootId) -> Option<&RootLayoutFacts> {
        self.roots.get(&root)
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = &RootLayoutFacts> {
        self.roots.values()
    }

    pub(crate) fn insert_root(&mut self, facts: RootLayoutFacts) -> bool {
        Arc::make_mut(&mut self.roots)
            .insert(facts.root(), facts)
            .is_none()
    }

    pub(crate) fn tab_intrinsic_for_tab(&self, tab: crate::ids::TabKey) -> Option<TabIntrinsic> {
        self.measurements.tab_intrinsic_for_tab(tab)
    }

    pub(crate) fn tab_intrinsic(&self, key: TabIntrinsicKey) -> Option<TabIntrinsic> {
        self.measurements.measured_tab_intrinsic(key)
    }

    pub(crate) fn tab_strip(&self, key: TabStripKey) -> Option<TabStripMetrics> {
        self.measurements.measured_tab_strip(key)
    }

    pub(crate) fn measurements(&self) -> &Arc<SurfaceMeasurements> {
        &self.measurements
    }

    pub(crate) fn rebind_measurements(&self, measurements: SurfaceMeasurements) -> Self {
        Self {
            config: self.config.clone(),
            roots: Arc::clone(&self.roots),
            measurements: Arc::new(measurements),
        }
    }

    pub(crate) fn same_inputs(&self, other: &Self) -> bool {
        self.config == other.config
            && (Arc::ptr_eq(&self.roots, &other.roots) || self.roots == other.roots)
            && (Arc::ptr_eq(&self.measurements, &other.measurements)
                || self.measurements == other.measurements)
    }
}

/// Surface-local bounds and raw pane sizing measured for one presented root.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RootLayoutFacts {
    root: RootId,
    bounds: LogicalRect,
    allocation: LogicalRect,
    pane_sizings: BTreeMap<PaneSizingKey, PaneSizing>,
    projection: Arc<LayoutProjection>,
    labels: Option<NodeMap<super::FrozenTabLabel>>,
    tab_bar_visibility: BTreeMap<NodeId, TabBarVisibility>,
}

impl RootLayoutFacts {
    pub(crate) fn new(
        root: RootId,
        bounds: LogicalRect,
        pane_sizings: BTreeMap<PaneSizingKey, PaneSizing>,
        projection: Arc<LayoutProjection>,
    ) -> Self {
        Self {
            root,
            bounds,
            allocation: bounds,
            pane_sizings,
            projection,
            labels: None,
            tab_bar_visibility: BTreeMap::new(),
        }
    }

    pub(crate) const fn with_allocation(mut self, allocation: LogicalRect) -> Self {
        self.allocation = allocation;
        self
    }

    pub(crate) fn with_node_labels(mut self, labels: NodeMap<super::FrozenTabLabel>) -> Self {
        self.labels = Some(labels);
        self
    }

    pub(crate) fn with_tab_bar_visibility(
        mut self,
        visibility: BTreeMap<NodeId, TabBarVisibility>,
    ) -> Self {
        self.tab_bar_visibility = visibility;
        self
    }

    /// Retains resolved chrome independently of whether clipping emitted a header.
    pub(crate) fn tab_bar_visibility(&self, tabs: NodeId) -> Option<TabBarVisibility> {
        self.tab_bar_visibility.get(&tabs).copied()
    }

    pub(crate) fn node_label(&self, node: NodeId) -> Option<crate::graph::TabLabel<'_>> {
        self.labels
            .as_ref()?
            .get(&node)
            .map(super::FrozenTabLabel::as_label)
    }

    pub(crate) const fn root(&self) -> RootId {
        self.root
    }

    pub(crate) const fn bounds(&self) -> LogicalRect {
        self.bounds
    }

    pub(crate) const fn allocation(&self) -> LogicalRect {
        self.allocation
    }

    /// Returns the actual root allocation after reserving independent controls.
    pub(crate) fn content_bounds(&self) -> LogicalRect {
        self.projection
            .nodes
            .iter_preorder()
            .next()
            .map_or(self.allocation, |(_, node)| node.slot)
    }

    pub(crate) fn projection(&self) -> &LayoutProjection {
        &self.projection
    }

    pub(crate) fn pane_sizings(&self) -> impl Iterator<Item = (PaneSizingKey, PaneSizing)> + '_ {
        self.pane_sizings
            .iter()
            .map(|(key, minimum)| (*key, *minimum))
    }

    pub(crate) fn pane_sizing(&self, key: PaneSizingKey) -> Option<PaneSizing> {
        self.pane_sizings.get(&key).copied()
    }
}
