//! Linear edge contacts used to publish splitter resize availability.

use std::collections::BTreeMap;

use super::{Axis, LayoutProjection, NodeId, NodeResult, SplitProjection};
use crate::geometry::LogicalRect;
use crate::graph::{Node, Workspace};

#[derive(Clone, Copy, Debug, Default)]
struct EdgeContacts {
    start: bool,
    end: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct NodeContacts {
    axes: [EdgeContacts; 2],
    split_axis: Option<Axis>,
    partition_complete: bool,
}

/// Temporary edge facts derived from one exact root projection.
///
/// These facts describe contact topology only. Policy, occlusion, numeric
/// displacement limits, and prepared-action authority retain their own checks.
pub(crate) struct ResizeContactFacts {
    nodes: BTreeMap<NodeId, NodeContacts>,
}

impl ResizeContactFacts {
    /// Checks the two incident children of an exact projected splitter.
    pub(crate) fn splitter_is_operable(
        &self,
        split: NodeId,
        before: NodeId,
        after: NodeId,
    ) -> bool {
        let Some(split) = self.nodes.get(&split) else {
            return false;
        };
        let Some(axis) = split.split_axis else {
            return false;
        };
        let Some(before) = self.nodes.get(&before) else {
            return false;
        };
        let Some(after) = self.nodes.get(&after) else {
            return false;
        };
        let axis = axis_index(axis);
        split.partition_complete && before.axes[axis].end && after.axes[axis].start
    }
}

/// Measures each node's independent start/end edge contacts once, bottom-up.
///
/// A capped frame cannot follow an independently moving slot edge. A split
/// along the requested axis needs a complete partition and its first/last
/// visible child's corresponding contact. A perpendicular split needs that
/// contact from every visible child, without constraining the unrelated axis.
/// Missing tree or projection facts leave the affected contacts unavailable.
pub(crate) fn measure_resize_contacts(
    workspace: &Workspace,
    projection: &LayoutProjection,
    splitter_thickness: f64,
) -> ResizeContactFacts {
    let mut nodes = BTreeMap::<NodeId, NodeContacts>::new();
    for (&node, result) in projection.nodes.iter_preorder().rev() {
        let mut contacts = NodeContacts::default();
        if result.visible {
            match workspace.node(node) {
                Some(Node::Tabs { .. }) => {
                    for axis in [Axis::Horizontal, Axis::Vertical] {
                        let aligned = frame_follows_slot(result, axis);
                        contacts.axes[axis_index(axis)] = EdgeContacts {
                            start: aligned,
                            end: aligned,
                        };
                    }
                }
                Some(Node::Split { axis, children, .. }) => {
                    contacts.split_axis = Some(*axis);
                    if let Some(split) = projection
                        .splits
                        .get(&node)
                        .filter(|split| split.axis == *axis)
                    {
                        let geometry_valid = partition_is_valid(
                            result,
                            &split,
                            children,
                            projection,
                            splitter_thickness,
                        );
                        contacts.partition_complete = geometry_valid
                            && split.unallocated <= coordinate_tolerance(result.rect, *axis);
                        for requested_axis in [Axis::Horizontal, Axis::Vertical] {
                            if !geometry_valid || !frame_follows_slot(result, requested_axis) {
                                continue;
                            }
                            let mut first = None;
                            let mut last = None;
                            let mut all = EdgeContacts {
                                start: true,
                                end: true,
                            };
                            let mut complete = true;
                            for child in children {
                                let Some(result) = projection.nodes.get(child) else {
                                    complete = false;
                                    break;
                                };
                                if !result.visible {
                                    continue;
                                }
                                let Some(child) = nodes.get(child) else {
                                    complete = false;
                                    break;
                                };
                                let edges = child.axes[axis_index(requested_axis)];
                                first.get_or_insert(edges);
                                last = Some(edges);
                                all.start &= edges.start;
                                all.end &= edges.end;
                            }
                            let (Some(first), Some(last)) = (first, last) else {
                                continue;
                            };
                            if !complete {
                                continue;
                            }
                            contacts.axes[axis_index(requested_axis)] = if *axis == requested_axis {
                                EdgeContacts {
                                    start: contacts.partition_complete && first.start,
                                    end: contacts.partition_complete && last.end,
                                }
                            } else {
                                all
                            };
                        }
                    }
                }
                None => {}
            }
        }
        nodes.insert(node, contacts);
    }
    ResizeContactFacts { nodes }
}

fn frame_follows_slot(result: &NodeResult, axis: Axis) -> bool {
    (rect_extent(result.slot, axis) - rect_extent(result.rect, axis)).abs()
        <= coordinate_tolerance(result.slot, axis)
}

fn partition_is_valid(
    result: &NodeResult,
    split: &SplitProjection<'_>,
    children: &[NodeId],
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
        let Some(projected) = projection.nodes.get(node) else {
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
            || child.extent <= 0.0
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
    use crate::geometry::PaneSizing;
    use crate::graph::{RootRecord, SurfacePresentation};
    use crate::ids::{ItemId, RootId, SurfaceId};
    use crate::model::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    };
    use crate::policy::DockPolicy;
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
    fn a_capped_descendant_breaks_its_column_contact_until_hidden() {
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
        let contacts = measure_resize_contacts(&workspace, &projection, 1.0);
        assert!(!contacts.splitter_is_operable(root, column, right));
        assert!(contacts.splitter_is_operable(column, top, bottom));

        workspace.replace_hidden_items([FIRST].into()).unwrap();
        let projection = measure_root_sizing(&workspace, ROOT, &sizing, metrics)
            .unwrap()
            .project(bounds, &[])
            .unwrap();
        assert!(!projection.nodes[&top].visible);
        let contacts = measure_resize_contacts(&workspace, &projection, 1.0);
        assert!(contacts.splitter_is_operable(root, column, right));
    }

    #[test]
    fn a_capped_frontier_publishes_no_pointer_or_semantic_resize_receiver() {
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
        assert!(!outer.operable());
        assert!(plan.receiver_for_splitter(outer).is_none());
        assert!(plan.splitter_resize_range(outer.visual_id()).is_none());
        for adjustment in [
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Equalize,
            SurfaceSplitterAdjustment::Preferred,
        ] {
            assert!(
                plan.prepare_splitter_adjustment(outer.visual_id(), adjustment)
                    .is_none()
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
                let pane = plan
                    .panes()
                    .find(|pane| pane.selected() == Some(FIRST))
                    .unwrap();
                let extent = match axis {
                    DockspaceAxis::Horizontal => pane.bounds().height(),
                    DockspaceAxis::Vertical => pane.bounds().width(),
                };
                let expected = match policy {
                    PaneOverflowPolicy::Clip => 80.0,
                    PaneOverflowPolicy::Compress => 49.5,
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
