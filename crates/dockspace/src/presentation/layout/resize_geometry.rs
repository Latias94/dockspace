//! Feasible projected geometry used to publish splitter resize availability.

use std::collections::BTreeMap;

use super::{Axis, LayoutProjection, NodeId, NodeResult, RootNodeKind, SplitProjection};
use crate::geometry::LogicalRect;

#[derive(Clone, Copy, Debug, Default)]
struct NodeResizeFacts {
    visible: bool,
    feasible: bool,
    split_axis: Option<Axis>,
    partition_complete: bool,
}

/// Temporary eligibility facts derived from one exact root projection.
///
/// Capped content may occupy less than its offered slot: the frozen piecewise
/// evaluator follows that contact change. Overflow and incomplete partitions
/// remain unavailable. Policy, occlusion, numeric limits and prepared-action
/// authority retain their own checks.
pub(crate) struct ResizeGeometryFacts {
    nodes: BTreeMap<NodeId, NodeResizeFacts>,
    grid_tracks: BTreeMap<NodeId, [Vec<bool>; 2]>,
}

impl ResizeGeometryFacts {
    pub(crate) fn root_is_feasible(&self, node: NodeId) -> bool {
        self.nodes.get(&node).is_some_and(|facts| facts.feasible)
    }

    pub(crate) fn grid_splitter_is_operable(
        &self,
        grid: &super::GridProjection,
        axis: Axis,
        index: usize,
    ) -> bool {
        let Some(tracks) = self
            .grid_tracks
            .get(&grid.node)
            .map(|tracks| &tracks[axis_index(axis)])
        else {
            return false;
        };
        index + 1 < grid.axis(axis).tracks.len()
            && (tracks[index] || tracks[index + 1])
            && self.nodes.get(&grid.node).is_some_and(|node| node.feasible)
    }

    /// Checks the exact projected owner and its two visible incident children.
    pub(crate) fn splitter_is_operable(
        &self,
        split: NodeId,
        before: NodeId,
        after: NodeId,
    ) -> bool {
        self.nodes.get(&split).is_some_and(|split| {
            split.feasible && split.split_axis.is_some() && split.partition_complete
        }) && [before, after].into_iter().all(|child| {
            self.nodes
                .get(&child)
                .is_some_and(|child| child.visible && child.feasible)
        })
    }
}

/// Measures resize eligibility once, bottom-up through the active composition.
///
/// A smaller capped frame is valid; an overflowed or compressed frame is not.
/// A layout tab contributes only its active child. Shared Grid tracks and Split
/// partitions must cover their complete frame before a gesture can be frozen.
pub(crate) fn measure_resize_geometry(
    projection: &LayoutProjection,
    splitter_thickness: f64,
) -> ResizeGeometryFacts {
    let mut nodes = BTreeMap::<NodeId, NodeResizeFacts>::new();
    let mut grid_tracks = BTreeMap::new();
    let topology = &projection.nodes.topology;
    for (row, node) in topology.nodes.iter().enumerate().rev() {
        let result = &projection.nodes.values[row];
        let children = match &node.kind {
            RootNodeKind::Pane { .. } => &[][..],
            RootNodeKind::Tabs { children }
            | RootNodeKind::Split { children, .. }
            | RootNodeKind::Grid { children, .. } => &topology.children[children.clone()],
        };
        let mut facts = NodeResizeFacts {
            visible: result.visible,
            ..NodeResizeFacts::default()
        };
        if result.visible {
            let frame_valid = [Axis::Horizontal, Axis::Vertical].into_iter().all(|axis| {
                let extent = rect_extent(result.rect, axis);
                let minimum = match axis {
                    Axis::Horizontal => result.constraints.min().width(),
                    Axis::Vertical => result.constraints.min().height(),
                };
                extent + coordinate_tolerance(result.rect, axis) >= minimum
                    && extent
                        <= rect_extent(result.slot, axis) + coordinate_tolerance(result.slot, axis)
            });
            let descendants_feasible = children.iter().all(|&child| {
                nodes
                    .get(&topology.nodes[child].id)
                    .is_some_and(|child| !child.visible || child.feasible)
            });
            facts.feasible = frame_valid && descendants_feasible;
            match &node.kind {
                RootNodeKind::Pane { .. } | RootNodeKind::Tabs { .. } => {}
                RootNodeKind::Split { payload, .. } => {
                    facts.split_axis = Some(payload.axis());
                    facts.partition_complete =
                        projection.splits.get(&node.id).is_some_and(|split| {
                            partition_is_valid(
                                result,
                                &split,
                                children.iter().map(|&row| topology.nodes[row].id),
                                projection,
                                splitter_thickness,
                            )
                        });
                    facts.feasible &= facts.partition_complete;
                }
                RootNodeKind::Grid { .. } => {
                    facts.feasible &= projection.grids.get(&node.id).is_some_and(|grid| {
                        let mut visible = [
                            vec![false; grid.shape.columns],
                            vec![false; grid.shape.rows],
                        ];
                        for (cell, child) in grid.children.iter().enumerate() {
                            if nodes.get(child).is_some_and(|child| child.visible) {
                                visible[0][cell % grid.shape.columns] = true;
                                visible[1][cell / grid.shape.columns] = true;
                            }
                        }
                        grid_tracks.insert(node.id, visible);
                        [Axis::Horizontal, Axis::Vertical].into_iter().all(|axis| {
                            let tracks = grid.axis(axis);
                            tracks.overflow == 0.0
                                && tracks.unallocated <= coordinate_tolerance(result.rect, axis)
                        })
                    });
                }
            }
        }
        nodes.insert(node.id, facts);
    }
    ResizeGeometryFacts { nodes, grid_tracks }
}

fn partition_is_valid(
    result: &NodeResult,
    split: &SplitProjection<'_>,
    children: impl ExactSizeIterator<Item = NodeId>,
    projection: &LayoutProjection,
    splitter_thickness: f64,
) -> bool {
    if split.children.len() != children.len()
        || !splitter_thickness.is_finite()
        || splitter_thickness < 0.0
    {
        return false;
    }
    let tolerance = coordinate_tolerance(result.rect, split.axis);
    if !split.overflow.is_finite()
        || split.overflow < 0.0
        || split.overflow > tolerance
        || !split.unallocated.is_finite()
        || split.unallocated < 0.0
    {
        return false;
    }
    let mut remaining = split.visible_child_indices().count();
    if remaining == 0 {
        return false;
    }
    let mut extent = 0.0;
    for (child, node) in split.children.iter().zip(children) {
        let Some(projected) = projection.nodes.get(&node) else {
            return false;
        };
        if child.visible != projected.visible {
            return false;
        }
        if !child.visible {
            if child.following_gap.is_some() {
                return false;
            }
            continue;
        }
        if !child.extent.is_finite()
            || child.extent < 0.0
            || (child.extent - rect_extent(projected.rect, split.axis)).abs() > tolerance
        {
            return false;
        }
        remaining -= 1;
        extent += child.extent;
        match (remaining, child.following_gap) {
            (0, None) => {}
            (0, Some(_)) | (_, None) => return false,
            (_, Some(gap)) => {
                let gap_extent = rect_extent(gap, split.axis);
                let gap_tolerance = coordinate_tolerance(gap, split.axis)
                    .max(splitter_thickness.max(1.0) * f64::EPSILON * 64.0);
                if (gap_extent - splitter_thickness).abs() > gap_tolerance {
                    return false;
                }
                extent += gap_extent;
            }
        }
    }
    // unallocated also includes the parent's slot outside this capped frame.
    // The internal track sum proves completeness without treating that background
    // as a missing edge on an unrelated resize axis.
    extent.is_finite() && (extent - rect_extent(result.rect, split.axis)).abs() <= tolerance
}

fn axis_index(axis: Axis) -> usize {
    match axis {
        Axis::Horizontal => 0,
        Axis::Vertical => 1,
    }
}

fn rect_extent(rect: LogicalRect, axis: Axis) -> f64 {
    match axis {
        Axis::Horizontal => rect.width(),
        Axis::Vertical => rect.height(),
    }
}

fn coordinate_tolerance(rect: LogicalRect, axis: Axis) -> f64 {
    let (minimum, maximum) = match axis {
        Axis::Horizontal => (rect.min().x(), rect.max().x()),
        Axis::Vertical => (rect.min().y(), rect.max().y()),
    };
    minimum.abs().max(maximum.abs()).max(1.0) * f64::EPSILON * 64.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;

    use crate::geometry::PaneSizing;
    use crate::graph::{Node, RootRecord, SurfacePresentation, Workspace};
    use crate::ids::{ItemId, RootId, SurfaceId};
    use crate::model::{
        DockspaceAxis, DockspaceGridShape, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
        DockspaceSurfaceLayout,
    };
    use crate::policy::DockPolicy;
    use crate::presentation::config::ResizePropagation;
    use crate::presentation::layout::{LayoutMetrics, measure_root_sizing};
    use crate::runtime::{
        DockPresentationConfig, DockspaceSession, SurfaceMeasurementAnswer,
        SurfaceMeasurementRequest, SurfaceSplitterAdjustment, TabStripMetrics,
    };

    const SURFACE: SurfaceId = SurfaceId::new(1);
    const ROOT: RootId = RootId::new(1);
    const FIRST: ItemId = ItemId::new(1);
    const SECOND: ItemId = ItemId::new(2);
    const THIRD: ItemId = ItemId::new(3);

    fn first_sizing() -> PaneSizing {
        PaneSizing::builder().max_width(120.0).build().unwrap()
    }

    #[test]
    fn a_capped_descendant_preserves_piecewise_resize_eligibility() {
        let mut builder = Workspace::builder();
        let top = builder.insert_node(Node::tabs([FIRST]));
        let bottom = builder.insert_node(Node::tabs([SECOND]));
        let right = builder.insert_node(Node::tabs([THIRD]));
        let column = builder.insert_node(Node::equal_split(Axis::Vertical, [top, bottom]).unwrap());
        let root =
            builder.insert_node(Node::equal_split(Axis::Horizontal, [column, right]).unwrap());
        builder.set_root(ROOT, RootRecord::new(root));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let mut workspace = builder.build().unwrap();
        let sizing = BTreeMap::from([
            (top, first_sizing()),
            (bottom, PaneSizing::default()),
            (right, PaneSizing::default()),
        ]);
        let bounds = LogicalRect::new(0.0, 0.0, 601.0, 601.0).unwrap();
        let metrics = LayoutMetrics::new(1.0).unwrap();
        let projection = measure_root_sizing(&workspace, ROOT, &sizing, metrics)
            .unwrap()
            .project(bounds, &[])
            .unwrap();
        assert!((projection.nodes[&column].rect.width() - 300.0).abs() < 1.0e-9);
        assert!((projection.nodes[&top].slot.width() - 300.0).abs() < 1.0e-9);
        assert!((projection.nodes[&top].rect.width() - 120.0).abs() < 1.0e-9);
        let contacts = measure_resize_geometry(&projection, 1.0);
        assert!(contacts.splitter_is_operable(root, column, right));
        assert!(contacts.splitter_is_operable(column, top, bottom));

        workspace.replace_hidden_items([FIRST].into()).unwrap();
        let projection = measure_root_sizing(&workspace, ROOT, &sizing, metrics)
            .unwrap()
            .project(bounds, &[])
            .unwrap();
        assert!(!projection.nodes[&top].visible);
        let contacts = measure_resize_geometry(&projection, 1.0);
        assert!(contacts.splitter_is_operable(root, column, right));
    }

    #[test]
    fn a_capped_cross_axis_partition_keeps_external_slack_outside_its_complete_frame() {
        let mut builder = Workspace::builder();
        let top = builder.insert_node(Node::tabs([FIRST]));
        let bottom = builder.insert_node(Node::tabs([SECOND]));
        let right = builder.insert_node(Node::tabs([THIRD]));
        let column = builder.insert_node(Node::equal_split(Axis::Vertical, [top, bottom]).unwrap());
        let root =
            builder.insert_node(Node::equal_split(Axis::Horizontal, [column, right]).unwrap());
        builder.set_root(ROOT, RootRecord::new(root));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let workspace = builder.build().unwrap();
        let capped = PaneSizing::builder()
            .min_width(80.0)
            .min_height(88.0)
            .max_height(100.0)
            .build()
            .unwrap();
        let sizing = BTreeMap::from([
            (top, capped),
            (bottom, capped),
            (
                right,
                PaneSizing::builder()
                    .min_width(80.0)
                    .min_height(88.0)
                    .build()
                    .unwrap(),
            ),
        ]);
        let projection =
            measure_root_sizing(&workspace, ROOT, &sizing, LayoutMetrics::new(1.0).unwrap())
                .unwrap()
                .project(LogicalRect::new(0.0, 0.0, 601.0, 400.0).unwrap(), &[])
                .unwrap();
        assert_eq!(projection.nodes[&column].slot.height(), 400.0);
        assert_eq!(projection.nodes[&column].rect.height(), 201.0);
        assert_eq!(projection.splits.get(&column).unwrap().unallocated, 199.0);
        for child in [top, bottom] {
            assert_eq!(projection.nodes[&child].rect.width(), 300.0);
            assert_eq!(projection.nodes[&child].rect.height(), 100.0);
        }
        let geometry = measure_resize_geometry(&projection, 1.0);
        assert!(geometry.splitter_is_operable(root, column, right));
        assert!(geometry.splitter_is_operable(column, top, bottom));
    }

    #[test]
    fn a_visible_fixed_zero_sibling_keeps_the_split_partition_complete() {
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let mut builder = Workspace::builder();
            let children = [FIRST, SECOND, THIRD].map(|item| builder.insert_node(Node::pane(item)));
            let root = builder.insert_node(Node::equal_split(axis, children).unwrap());
            builder.set_root(ROOT, RootRecord::new(root));
            builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
            let workspace = builder.build().unwrap();
            let fixed = match axis {
                Axis::Horizontal => PaneSizing::builder().fixed_width(0.0),
                Axis::Vertical => PaneSizing::builder().fixed_height(0.0),
            }
            .build()
            .unwrap();
            let sizing = BTreeMap::from([
                (children[0], fixed),
                (children[1], PaneSizing::default()),
                (children[2], PaneSizing::default()),
            ]);
            let projection =
                measure_root_sizing(&workspace, ROOT, &sizing, LayoutMetrics::new(4.0).unwrap())
                    .unwrap()
                    .project(LogicalRect::new(0.0, 0.0, 604.0, 604.0).unwrap(), &[])
                    .unwrap();
            for (child, expected) in children.into_iter().zip([0.0, 298.0, 298.0]) {
                let result = projection.nodes[&child];
                assert!(result.visible);
                assert_eq!(rect_extent(result.rect, axis), expected);
            }
            let geometry = measure_resize_geometry(&projection, 4.0);
            assert!(geometry.splitter_is_operable(root, children[1], children[2]));
        }
    }

    #[test]
    fn fixed_zero_siblings_allow_public_split_and_grid_resize_actions() {
        for axis in [DockspaceAxis::Horizontal, DockspaceAxis::Vertical] {
            for propagation in [ResizePropagation::Adjacent, ResizePropagation::Outward] {
                for grid in [false, true] {
                    let children = [FIRST, SECOND, THIRD].map(DockspaceNode::pane);
                    let root = if grid {
                        let (rows, columns) = match axis {
                            DockspaceAxis::Horizontal => (1, 3),
                            DockspaceAxis::Vertical => (3, 1),
                        };
                        DockspaceNode::grid(
                            DockspaceGridShape::Fixed {
                                rows: NonZeroUsize::new(rows).unwrap(),
                                columns: NonZeroUsize::new(columns).unwrap(),
                            },
                            children,
                        )
                        .unwrap()
                    } else {
                        DockspaceNode::equal_split(axis, children).unwrap()
                    };
                    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                        SURFACE,
                        DockspaceRootLayout::new(ROOT, root),
                    )])
                    .unwrap();
                    let config = DockPresentationConfig::builder()
                        .minimum_pane_size(1.0, 1.0)
                        .tab_bar_height(20.0)
                        .tab_group_grip_extent(20.0)
                        .splitter_thickness(4.0)
                        .splitter_keyboard_step(20.0)
                        .resize_propagation(propagation)
                        .build()
                        .unwrap();
                    let fixed = match axis {
                        DockspaceAxis::Horizontal => PaneSizing::builder().fixed_width(0.0),
                        DockspaceAxis::Vertical => PaneSizing::builder().fixed_height(0.0),
                    }
                    .build()
                    .unwrap();
                    let mut session = DockspaceSession::from_layout_with_presentation_config(
                        layout,
                        DockPolicy::default(),
                        config,
                    )
                    .unwrap();
                    for (adjustment, expected) in [
                        (None, [0.0, 298.0, 298.0]),
                        (
                            Some(SurfaceSplitterAdjustment::Increment),
                            [0.0, 318.0, 278.0],
                        ),
                        (
                            Some(SurfaceSplitterAdjustment::Decrement),
                            [0.0, 298.0, 298.0],
                        ),
                    ] {
                        let mut frame = session.begin_host_frame().unwrap();
                        if let Some(adjustment) = adjustment {
                            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                            let splitter = plan
                                .splitters()
                                .find(|splitter| {
                                    splitter.axis() == axis
                                        && match axis {
                                            DockspaceAxis::Horizontal => {
                                                splitter.before_bounds().width() > 0.0
                                            }
                                            DockspaceAxis::Vertical => {
                                                splitter.before_bounds().height() > 0.0
                                            }
                                        }
                                })
                                .unwrap();
                            assert!(splitter.operable());
                            assert!(plan.receiver_for_splitter(splitter).is_some());
                            assert!(plan.splitter_resize_range(splitter.visual_id()).is_some());
                            let action = plan
                                .prepare_splitter_adjustment(splitter.visual_id(), adjustment)
                                .unwrap();
                            frame.submit_surface_action(action).unwrap();
                        }
                        frame
                            .measure_surface_with(SURFACE, |request| {
                                Ok::<_, crate::runtime::DockspaceRuntimeError>(match request {
                                    SurfaceMeasurementRequest::DockBounds { .. }
                                    | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                                        SurfaceMeasurementAnswer::Bounds(
                                            LogicalRect::new(0.0, 0.0, 604.0, 624.0).unwrap(),
                                        )
                                    }
                                    SurfaceMeasurementRequest::PaneSizing {
                                        item: Some(FIRST),
                                        ..
                                    } => SurfaceMeasurementAnswer::PaneSizing(fixed),
                                    SurfaceMeasurementRequest::PaneSizing { .. } => {
                                        SurfaceMeasurementAnswer::PaneSizing(PaneSizing::default())
                                    }
                                    SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                                        SurfaceMeasurementAnswer::TabIntrinsic(64.0)
                                    }
                                    SurfaceMeasurementRequest::TabStrip { .. } => {
                                        SurfaceMeasurementAnswer::TabStrip(
                                            TabStripMetrics::new(0.0, 0.0).unwrap(),
                                        )
                                    }
                                })
                            })
                            .unwrap();
                        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                        assert_eq!(plan.panes().count(), 3);
                        assert_eq!(plan.tab_bars().count(), 0);
                        for (item, expected) in [FIRST, SECOND, THIRD].into_iter().zip(expected) {
                            let pane = plan.panes().find(|pane| pane.item() == item).unwrap();
                            let bounds = pane.content_geometry().clip();
                            let actual = match axis {
                                DockspaceAxis::Horizontal => bounds.width(),
                                DockspaceAxis::Vertical => bounds.height(),
                            };
                            assert!(
                                (actual - expected).abs() < 1.0e-9,
                                "{axis:?}/{propagation:?}/grid={grid}/{item:?}: {actual} != {expected}"
                            );
                        }
                        let report = frame.commit().unwrap();
                        assert_eq!(
                            report.workspace_changed(),
                            adjustment.is_some(),
                            "{report:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_capped_frontier_publishes_pointer_and_semantic_resize_receivers() {
        let column = DockspaceNode::equal_split(
            DockspaceAxis::Vertical,
            [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
        )
        .unwrap();
        let root = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [column, DockspaceNode::tabs([THIRD])],
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, root),
        )])
        .unwrap();
        let config = DockPresentationConfig::builder()
            .minimum_pane_size(1.0, 1.0)
            .splitter_thickness(1.0)
            .build()
            .unwrap();
        let mut session = DockspaceSession::from_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            config,
        )
        .unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .measure_surface_with(SURFACE, |request| {
                Ok::<_, crate::runtime::DockspaceRuntimeError>(match request {
                    SurfaceMeasurementRequest::DockBounds { .. }
                    | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                        SurfaceMeasurementAnswer::Bounds(
                            LogicalRect::new(0.0, 0.0, 601.0, 601.0).unwrap(),
                        )
                    }
                    SurfaceMeasurementRequest::PaneSizing {
                        item: Some(FIRST), ..
                    } => SurfaceMeasurementAnswer::PaneSizing(first_sizing()),
                    SurfaceMeasurementRequest::PaneSizing { .. } => {
                        SurfaceMeasurementAnswer::PaneSizing(PaneSizing::default())
                    }
                    SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                        SurfaceMeasurementAnswer::TabIntrinsic(64.0)
                    }
                    SurfaceMeasurementRequest::TabStrip { .. } => {
                        SurfaceMeasurementAnswer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                    }
                })
            })
            .unwrap();
        frame.commit().unwrap();

        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let outer = plan
            .splitters()
            .find(|splitter| splitter.axis() == DockspaceAxis::Horizontal)
            .unwrap();
        assert!(outer.operable());
        assert!(plan.receiver_for_splitter(outer).is_some());
        assert!(plan.splitter_resize_range(outer.visual_id()).is_some());
        for adjustment in [
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Equalize,
            SurfaceSplitterAdjustment::Preferred,
        ] {
            assert!(
                plan.prepare_splitter_adjustment(outer.visual_id(), adjustment)
                    .is_some()
            );
        }
        let inner = plan
            .splitters()
            .find(|splitter| splitter.axis() == DockspaceAxis::Vertical)
            .unwrap();
        assert!(inner.operable());
        assert!(plan.receiver_for_splitter(inner).is_some());
    }

    #[test]
    fn orthogonal_overflow_publishes_no_unusable_resize_receiver() {
        use crate::presentation::config::PaneOverflowPolicy;

        for axis in [DockspaceAxis::Horizontal, DockspaceAxis::Vertical] {
            for policy in [PaneOverflowPolicy::Clip, PaneOverflowPolicy::Compress] {
                let (cross, width, height, sizing) = match axis {
                    DockspaceAxis::Horizontal => (
                        DockspaceAxis::Vertical,
                        601.0,
                        100.0,
                        PaneSizing::builder().min_height(52.0).build().unwrap(),
                    ),
                    DockspaceAxis::Vertical => (
                        DockspaceAxis::Horizontal,
                        100.0,
                        601.0,
                        PaneSizing::builder().min_width(80.0).build().unwrap(),
                    ),
                };
                let inner = DockspaceNode::equal_split(
                    cross,
                    [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
                )
                .unwrap();
                let content =
                    DockspaceNode::equal_split(axis, [inner, DockspaceNode::tabs([THIRD])])
                        .unwrap();
                let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                    SURFACE,
                    DockspaceRootLayout::new(ROOT, content),
                )])
                .unwrap();
                let config = DockPresentationConfig::builder()
                    .minimum_pane_size(1.0, 1.0)
                    .pane_overflow_policy(policy)
                    .build()
                    .unwrap();
                let tab_bar_height = config.tab_bar_height();
                let mut session = DockspaceSession::from_layout_with_presentation_config(
                    layout,
                    DockPolicy::default(),
                    config,
                )
                .unwrap();
                let mut frame = session.begin_host_frame().unwrap();
                frame
                    .measure_surface_with(SURFACE, |request| {
                        Ok::<_, crate::runtime::DockspaceRuntimeError>(match request {
                            SurfaceMeasurementRequest::DockBounds { .. }
                            | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                                SurfaceMeasurementAnswer::Bounds(
                                    LogicalRect::new(0.0, 0.0, width, height).unwrap(),
                                )
                            }
                            SurfaceMeasurementRequest::PaneSizing { item, .. } => {
                                SurfaceMeasurementAnswer::PaneSizing(if item == Some(THIRD) {
                                    PaneSizing::default()
                                } else {
                                    sizing
                                })
                            }
                            SurfaceMeasurementRequest::TabIntrinsic { .. } => {
                                SurfaceMeasurementAnswer::TabIntrinsic(64.0)
                            }
                            SurfaceMeasurementRequest::TabStrip { .. } => {
                                SurfaceMeasurementAnswer::TabStrip(
                                    TabStripMetrics::new(0.0, 0.0).unwrap(),
                                )
                            }
                        })
                    })
                    .unwrap();
                let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                let pane = plan.panes().find(|pane| pane.item() == FIRST).unwrap();
                let bounds = pane.content_geometry().clip();
                let extent = match axis {
                    DockspaceAxis::Horizontal => bounds.height(),
                    DockspaceAxis::Vertical => bounds.width(),
                };
                let chrome = match axis {
                    DockspaceAxis::Horizontal => tab_bar_height,
                    DockspaceAxis::Vertical => 0.0,
                };
                let expected = match policy {
                    PaneOverflowPolicy::Clip => 80.0 - chrome,
                    PaneOverflowPolicy::Compress => 49.5 - chrome,
                };
                assert!((extent - expected).abs() < 1.0e-9, "{axis:?}/{policy:?}");
                let outer = plan
                    .splitters()
                    .find(|splitter| splitter.axis() == axis)
                    .unwrap();
                assert!(!outer.operable(), "{axis:?}/{policy:?}");
                assert!(plan.receiver_for_splitter(outer).is_none());
                assert!(plan.splitter_resize_range(outer.visual_id()).is_none());
                assert!(
                    plan.prepare_splitter_adjustment(
                        outer.visual_id(),
                        SurfaceSplitterAdjustment::Increment,
                    )
                    .is_none()
                );
            }
        }
    }
}
