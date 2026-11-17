use std::sync::Arc;

use super::*;
use crate::geometry::Constraints;
use crate::graph::{RootRecord, SurfacePresentation, WorkspaceBuilder};
use crate::ids::{EngineAuthorityDomainId, ItemId, SurfaceId, WorkspaceEpoch};
use crate::interaction::targeting::targets::SceneLayerKey;
use crate::policy::PolicyRevision;
use crate::presentation::config::{DockPresentationConfig, PresentationConfigRevision};
use crate::presentation::hit::region::HitRegion;
use crate::presentation::layout::{
    LayoutMetrics, TrackWeightOverride, project_root, project_root_with_overrides,
};
use crate::presentation::manifest::{
    Measurement, SurfaceMeasurementTicket, SurfaceMeasurements, SurfaceRequirementRevision,
    SurfaceRequirements,
};
use crate::presentation::scene::{RootLayoutFacts, SplitterSceneId};

const ROOT: RootId = RootId::new(1);
const SURFACE: SurfaceId = SurfaceId::new(1);

struct Fixture {
    workspace: Workspace,
    plan: PresentationPlan,
    projection: Arc<LayoutProjection>,
    constraints: BTreeMap<NodeId, Constraints>,
    bounds: LogicalRect,
    metrics: LayoutMetrics,
    capped: bool,
}

impl Fixture {
    fn new(
        workspace: Workspace,
        bounds: LogicalRect,
        constraints: BTreeMap<NodeId, Constraints>,
        thickness: f64,
    ) -> Self {
        Self::with_propagation(
            workspace,
            bounds,
            constraints,
            thickness,
            crate::presentation::config::ResizePropagation::Adjacent,
        )
    }

    fn with_propagation(
        workspace: Workspace,
        bounds: LogicalRect,
        constraints: BTreeMap<NodeId, Constraints>,
        thickness: f64,
        propagation: crate::presentation::config::ResizePropagation,
    ) -> Self {
        Self::with_mode(
            workspace,
            bounds,
            constraints,
            thickness,
            propagation,
            false,
        )
    }

    fn with_mode(
        workspace: Workspace,
        bounds: LogicalRect,
        constraints: BTreeMap<NodeId, Constraints>,
        thickness: f64,
        propagation: crate::presentation::config::ResizePropagation,
        capped: bool,
    ) -> Self {
        let metrics = LayoutMetrics::new(thickness).unwrap();
        let projection = Arc::new(if capped {
            crate::presentation::layout::measure_root_sizing(
                &workspace,
                ROOT,
                &pane_sizing(&constraints),
                metrics,
            )
            .unwrap()
            .project(bounds, &[])
            .unwrap()
        } else {
            project_root(
                &workspace,
                ROOT,
                bounds,
                |node| constraints.get(&node).copied(),
                metrics,
            )
            .unwrap()
        });
        let ticket = SurfaceMeasurementTicket::new(
            EngineAuthorityDomainId::new_for_test(1),
            WorkspaceEpoch::new(1),
            PresentationConfigRevision::new(1),
            PolicyRevision::new(1),
            SurfaceRequirementRevision::new(1),
            SURFACE,
        );
        let requirements = SurfaceRequirements::new(
            ticket,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let mut measurements = SurfaceMeasurements::new(ticket);
        measurements
            .set_bounds(requirements.bounds(), Measurement::Measured(bounds))
            .unwrap();
        let measurements = requirements
            .validate(&measurements)
            .unwrap()
            .require_authoritative()
            .unwrap();
        let config = DockPresentationConfig::builder()
            .splitter_thickness(thickness)
            .resize_propagation(propagation)
            .build()
            .unwrap();
        let mut plan = PresentationPlan::new(SURFACE, bounds);
        plan.install_layout_facts(config, measurements);
        plan.push_root_layout_facts(RootLayoutFacts::new(
            ROOT,
            bounds,
            BTreeMap::new(),
            Arc::clone(&projection),
        ));
        Self {
            workspace,
            plan,
            projection,
            constraints,
            bounds,
            metrics,
            capped,
        }
    }

    fn handle(&self, node: NodeId, index: usize) -> (NodeSource, SplitterRecord) {
        let Node::Split {
            axis,
            children,
            weights,
            ..
        } = self.workspace.node(node).unwrap()
        else {
            unreachable!();
        };
        let split = self.projection.splits.get(&node).unwrap();
        let after = split.following_child_index(index).unwrap();
        let draw = split.children[index].following_gap.unwrap();
        let record = SplitterRecord::new(
            SplitterSceneId::split(ROOT, node, index),
            draw,
            HitRegion::new(draw),
            *axis,
            self.projection.nodes[&children[index]].rect,
            self.projection.nodes[&children[after]].rect,
            split.children[index].minimum,
            split.children[after].minimum,
            split.children[index].maximum,
            split.children[after].maximum,
            split
                .children
                .iter()
                .map(|child| child.extent)
                .collect::<Vec<_>>(),
            split.central_index,
            weights.clone(),
            SceneLayerKey::surface_base(),
            true,
        )
        .with_after_index(after);
        (
            self.workspace.capture_node_source(ROOT, node).unwrap(),
            record,
        )
    }

    fn groups(&self, handles: &[(NodeId, usize)]) -> Vec<FrozenResizeAxisGroup> {
        prepare_resize_axis_groups(
            &self.workspace,
            &self.plan,
            handles
                .iter()
                .map(|&(node, index)| self.handle(node, index))
                .collect(),
        )
        .unwrap()
    }

    fn project_updates(&self, updates: &[TrackResize]) -> LayoutProjection {
        let overrides = updates
            .iter()
            .map(|update| TrackWeightOverride::new(update.source(), update.weights()))
            .collect::<Vec<_>>();
        if self.capped {
            return crate::presentation::layout::measure_root_sizing(
                &self.workspace,
                ROOT,
                &pane_sizing(&self.constraints),
                self.metrics,
            )
            .unwrap()
            .project(self.bounds, &overrides)
            .unwrap();
        }
        project_root_with_overrides(
            &self.workspace,
            ROOT,
            self.bounds,
            |node| self.constraints.get(&node).copied(),
            self.metrics,
            &overrides,
        )
        .unwrap()
    }
}

fn pane_sizing(
    constraints: &BTreeMap<NodeId, Constraints>,
) -> BTreeMap<NodeId, crate::geometry::PaneSizing> {
    constraints
        .iter()
        .map(|(&node, constraints)| {
            (
                node,
                crate::geometry::PaneSizing::builder()
                    .min_width(constraints.min().width())
                    .max_width(constraints.max().width())
                    .min_height(constraints.min().height())
                    .max_height(constraints.max().height())
                    .build()
                    .unwrap(),
            )
        })
        .collect()
}

fn rect(width: f64, height: f64) -> LogicalRect {
    LogicalRect::new(0.0, 0.0, width, height).unwrap()
}

fn constraints(minimum_width: f64, maximum_width: f64) -> Constraints {
    Constraints::new(
        LogicalSize::new(minimum_width, 20.0).unwrap(),
        LogicalSize::new(maximum_width, 2000.0).unwrap(),
    )
    .unwrap()
}

fn tabs(builder: &mut WorkspaceBuilder, item: u64) -> NodeId {
    builder.insert_node(Node::tabs([ItemId::new(item)]))
}

fn finish(mut builder: WorkspaceBuilder, node: NodeId, central: Option<NodeId>) -> Workspace {
    let record = match central {
        Some(central) => RootRecord::new(node)
            .with_central(central)
            .with_allocation(crate::model::RootAllocation::PreferCentral),
        None => RootRecord::new(node),
    };
    builder.set_root(ROOT, record);
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.build().unwrap()
}

fn nested(central: usize) -> (Fixture, [NodeId; 7]) {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let b = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let d = tabs(&mut builder, 4);
    let inner = builder.insert_node(Node::split(Axis::Horizontal, [c, d], [0.5, 0.5]).unwrap());
    let across = builder.insert_node(Node::split(Axis::Vertical, [b, inner], [0.5, 0.5]).unwrap());
    let outer =
        builder.insert_node(Node::split(Axis::Horizontal, [a, across], [0.5, 0.5]).unwrap());
    let central = match central {
        1 => Some(c),
        2 => Some(d),
        _ => None,
    };
    let workspace = finish(builder, outer, central);
    let fixture = Fixture::new(
        workspace,
        rect(1001.0, 601.0),
        BTreeMap::from([
            (a, constraints(100.0, 2000.0)),
            (b, constraints(100.0, 510.0)),
            (c, constraints(200.0, 2000.0)),
            (d, constraints(50.0, 2000.0)),
        ]),
        1.0,
    );
    (fixture, [a, b, c, d, inner, across, outer])
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 0.0002,
        "{actual} != {expected}"
    );
}

mod piecewise_tests;

#[test]
fn frontier_rejects_a_reordered_live_tree_with_equal_child_extents() {
    let (mut fixture, [_, _, c, d, inner, _, outer]) = nested(0);
    assert_eq!(
        fixture.projection.nodes[&c].rect.width(),
        fixture.projection.nodes[&d].rect.width()
    );
    let mut replacement = fixture.workspace.node(inner).unwrap().clone();
    let Node::Split { children, .. } = &mut replacement else {
        unreachable!();
    };
    children.swap(0, 1);
    let mut builder = fixture.workspace.clone().into_builder();
    builder.replace_node(inner, replacement).unwrap();
    fixture.workspace = builder.build().unwrap();
    assert!(
        prepare_resize_axis_groups(
            &fixture.workspace,
            &fixture.plan,
            vec![fixture.handle(outer, 0)],
        )
        .is_none()
    );
}

#[test]
fn nested_touching_frontier_uses_descendant_limits_and_keeps_the_far_side_fixed() {
    for central in 0..=2 {
        let (fixture, [a, b, c, d, inner, across, outer]) = nested(central);
        let groups = fixture.groups(&[(outer, 0)]);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .sources()
                .map(NodeSource::node)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([outer, across, inner])
        );
        assert_eq!(
            groups[0]
                .resized_sources()
                .map(NodeSource::node)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([outer, inner])
        );
        close(groups[0].common_delta.minimum, -10.0);
        close(groups[0].common_delta.maximum, 49.5);
        let sources = groups[0].sources().collect::<Vec<_>>();
        assert!(
            sources
                .iter()
                .all(|source| sources[0].reference() == source.reference())
        );
        for (requested, effective) in [(1e6, 49.5), (-1e6, -10.0), (18.0, 18.0)] {
            let updates = resize_axis_group_updates(&groups, [requested, 0.0]).unwrap();
            assert_eq!(updates.len(), 2);
            let projected = fixture.project_updates(&updates);
            close(projected.nodes[&a].rect.width(), 500.0 + effective);
            close(projected.nodes[&b].rect.width(), 500.0 - effective);
            close(projected.nodes[&c].rect.width(), 249.5 - effective);
            close(projected.nodes[&d].rect.width(), 249.5);
            close(
                projected.nodes[&d].rect.x(),
                fixture.projection.nodes[&d].rect.x(),
            );
        }
    }
}

#[test]
fn two_axes_keep_orthogonal_sources_but_emit_each_split_once() {
    let (fixture, [_, b, _, d, inner, across, outer]) = nested(0);
    let groups = fixture.groups(&[(outer, 0), (across, 0)]);
    assert_eq!(groups.len(), 2);
    for group in &groups {
        assert!(group.sources().any(|source| source.node() == inner));
        assert!(group.resized_sources().all(|source| {
            matches!(fixture.workspace.node(source.node()), Some(Node::Split { axis, .. }) if *axis == group.axis)
        }));
    }
    let updates = resize_axis_group_updates(&groups, [18.0, 24.0]).unwrap();
    assert_eq!(updates.len(), 3);
    assert_eq!(
        updates
            .iter()
            .map(|update| update.source().node())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([outer, across, inner])
    );
    let projected = fixture.project_updates(&updates);
    close(projected.nodes[&b].rect.width(), 482.0);
    close(projected.nodes[&b].rect.height(), 324.0);
    close(projected.nodes[&d].rect.width(), 249.5);
    close(projected.nodes[&d].rect.height(), 276.0);
    close(
        projected.nodes[&d].rect.x(),
        fixture.projection.nodes[&d].rect.x(),
    );
}

#[test]
fn adjacent_incident_gaps_merge_before_solving_an_nary_split() {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let b = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let outer =
        builder.insert_node(Node::split(Axis::Horizontal, [a, b, c], [0.25, 0.5, 0.25]).unwrap());
    let fixture = Fixture::new(
        finish(builder, outer, None),
        rect(1002.0, 300.0),
        [a, b, c]
            .into_iter()
            .map(|node| (node, constraints(20.0, 2000.0)))
            .collect(),
        1.0,
    );
    let groups = fixture.groups(&[(outer, 0), (outer, 1), (outer, 0)]);
    let updates = resize_axis_group_updates(&groups, [20.0, 0.0]).unwrap();
    assert_eq!(updates.len(), 1);
    let projected = fixture.project_updates(&updates);
    close(projected.nodes[&a].rect.width(), 270.0);
    close(projected.nodes[&b].rect.width(), 500.0);
    close(projected.nodes[&b].rect.x(), 271.0);
    close(projected.nodes[&c].rect.width(), 230.0);
    let mut edge = Some(1.0);
    assert!(merge_edge(&mut edge, -1.0).is_none());
}

#[test]
fn resize_across_hidden_siblings_preserves_their_weight_and_restoration_share() {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let b = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, [a, b, c], [0.2, 0.3, 0.5]).unwrap());
    builder.set_hidden_items([ItemId::new(2)].into());
    let fixture = Fixture::new(
        finish(builder, split, None),
        rect(1001.0, 300.0),
        [a, b, c]
            .into_iter()
            .map(|node| (node, constraints(20.0, 2000.0)))
            .collect(),
        1.0,
    );
    let groups = fixture.groups(&[(split, 0)]);
    let frozen = groups[0]
        .splits
        .iter()
        .find(|value| value.source.node() == split)
        .unwrap();
    assert_eq!(frozen.child_coefficients, [1.0, 0.0, -1.0]);
    let updates = resize_axis_group_updates(&groups, [50.0, 0.0]).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0].split_weights().unwrap()[1].get().to_bits(),
        0.3_f64.to_bits()
    );
    close(updates[0].split_weights().unwrap()[0].get(), 0.235);
    close(updates[0].split_weights().unwrap()[2].get(), 0.465);
    let projected = fixture.project_updates(&updates);
    close(
        projected.nodes[&a].rect.width(),
        fixture.projection.nodes[&a].rect.width() + 50.0,
    );
    close(
        projected.nodes[&c].rect.width(),
        fixture.projection.nodes[&c].rect.width() - 50.0,
    );
    assert!(!projected.nodes[&b].visible);
    let unchanged = resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap();
    assert_eq!(
        unchanged[0].split_weights().unwrap(),
        frozen.weights.as_slice()
    );
}

#[test]
fn hidden_constraints_do_not_limit_the_visible_frontier_but_visible_maxima_still_do() {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let hidden = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let split = builder
        .insert_node(Node::split(Axis::Horizontal, [a, hidden, c], [0.2, 0.3, 0.5]).unwrap());
    builder.set_hidden_items([ItemId::new(2)].into());
    let fixture = Fixture::new(
        finish(builder, split, None),
        rect(1001.0, 300.0),
        BTreeMap::from([
            (a, constraints(20.0, 300.0)),
            (hidden, constraints(10000.0, 20000.0)),
            (c, constraints(20.0, 2000.0)),
        ]),
        1.0,
    );
    let groups = fixture.groups(&[(split, 0)]);
    close(groups[0].common_delta.maximum, 300.0 - 1000.0 * 2.0 / 7.0);
    let updates = resize_axis_group_updates(&groups, [1000.0, 0.0]).unwrap();
    let projected = fixture.project_updates(&updates);
    close(projected.nodes[&a].rect.width(), 300.0);
    close(projected.nodes[&c].rect.width(), 700.0);
    assert_eq!(
        updates[0].split_weights().unwrap()[1].get().to_bits(),
        0.3_f64.to_bits()
    );
}

#[test]
fn resize_limits_keep_subnormal_visible_weights_representable_when_a_hidden_sibling_dominates() {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let hidden = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let split = builder
        .insert_node(Node::split(Axis::Horizontal, [a, hidden, c], [1e-310, 1.0, 1e-310]).unwrap());
    builder.set_hidden_items([ItemId::new(2)].into());
    let fixture = Fixture::new(
        finish(builder, split, None),
        rect(1001.0, 300.0),
        [a, hidden, c]
            .into_iter()
            .map(|node| (node, constraints(0.0, 2000.0)))
            .collect(),
        1.0,
    );
    let groups = fixture.groups(&[(split, 0)]);
    for delta in [-1e6, 1e6] {
        let updates = resize_axis_group_updates(&groups, [delta, 0.0])
            .expect("every advertised endpoint must keep positive durable weights");
        assert_eq!(
            updates[0].split_weights().unwrap()[1].get().to_bits(),
            1.0_f64.to_bits()
        );
        assert!(updates[0].split_weights().unwrap()[0].get() > 0.0);
        assert!(updates[0].split_weights().unwrap()[2].get() > 0.0);
        let projected = fixture.project_updates(&updates);
        assert!(projected.nodes[&a].rect.width() > 0.0);
        assert!(projected.nodes[&c].rect.width() > 0.0);
    }
}

#[test]
fn hidden_boundary_children_do_not_break_nested_touching_frontier() {
    let mut builder = Workspace::builder();
    let hidden_leading = tabs(&mut builder, 1);
    let left = tabs(&mut builder, 2);
    let hidden_trailing = tabs(&mut builder, 3);
    let right = tabs(&mut builder, 4);
    let hidden_lower = tabs(&mut builder, 5);
    let nested = builder.insert_node(
        Node::split(
            Axis::Horizontal,
            [hidden_leading, left, hidden_trailing],
            [0.2, 0.5, 0.3],
        )
        .unwrap(),
    );
    // A perpendicular level keeps canonicalization from merging both horizontal
    // splits, while its hidden sibling leaves the nested frontier fully exposed.
    let across = builder
        .insert_node(Node::split(Axis::Vertical, [nested, hidden_lower], [0.5, 0.5]).unwrap());
    let outer =
        builder.insert_node(Node::split(Axis::Horizontal, [across, right], [0.5, 0.5]).unwrap());
    builder.set_hidden_items([ItemId::new(1), ItemId::new(3), ItemId::new(5)].into());
    let fixture = Fixture::new(
        finish(builder, outer, None),
        rect(1001.0, 300.0),
        [hidden_leading, left, hidden_trailing, right, hidden_lower]
            .into_iter()
            .map(|node| (node, constraints(20.0, 2000.0)))
            .collect(),
        1.0,
    );
    assert!(matches!(
        fixture.workspace.node(nested),
        Some(Node::Split { .. })
    ));
    let groups = fixture.groups(&[(outer, 0)]);
    let updates = resize_axis_group_updates(&groups, [40.0, 0.0]).unwrap();
    let nested_update = updates
        .iter()
        .find(|update| update.source().node() == nested)
        .unwrap();
    assert_eq!(
        nested_update
            .split_weights()
            .unwrap()
            .iter()
            .map(|weight| weight.get())
            .collect::<Vec<_>>(),
        [0.2, 0.5, 0.3]
    );
    let projected = fixture.project_updates(&updates);
    close(projected.nodes[&left].rect.width(), 540.0);
    close(projected.nodes[&right].rect.width(), 460.0);
}

#[test]
fn nonadjacent_four_arm_junction_keeps_both_axes_and_central_routes_consistent() {
    for central_choice in 0..5 {
        let mut builder = Workspace::builder();
        let leaves = std::array::from_fn::<_, 7, _>(|index| {
            tabs(&mut builder, u64::try_from(index + 1).unwrap())
        });
        let north = builder.insert_node(
            Node::split(
                Axis::Horizontal,
                [leaves[0], leaves[1], leaves[2]],
                [0.25, 0.5, 0.25],
            )
            .unwrap(),
        );
        let south = builder.insert_node(
            Node::split(
                Axis::Horizontal,
                [leaves[3], leaves[4], leaves[5]],
                [0.25, 0.5, 0.25],
            )
            .unwrap(),
        );
        let outer = builder.insert_node(
            Node::split(Axis::Vertical, [north, leaves[6], south], [0.25, 0.5, 0.25]).unwrap(),
        );
        builder.set_hidden_items([ItemId::new(2), ItemId::new(5), ItemId::new(7)].into());
        let central = match central_choice {
            1 => Some(leaves[0]),
            2 => Some(leaves[5]),
            3 => Some(leaves[6]),
            4 => Some(leaves[1]),
            _ => None,
        };
        let fixture = Fixture::new(
            finish(builder, outer, central),
            rect(1001.0, 701.0),
            leaves
                .into_iter()
                .map(|node| (node, constraints(20.0, 2000.0)))
                .collect(),
            1.0,
        );
        let handles = [north, south, outer].map(|node| fixture.handle(node, 0));
        let junctions =
            crate::presentation::scene::junction_index::derive_splitter_junction_candidates(
                handles.iter().map(|(_, record)| record),
            )
            .unwrap();
        assert_eq!(junctions.len(), 1);
        assert_eq!(junctions[0].id().arm_count(), 4);
        assert_eq!(junctions[0].id().splitters().len(), 3);
        assert!(handles.iter().all(|(_, record)| record.after_index() == 2));
        let groups = fixture.groups(&[(north, 0), (south, 0), (outer, 0)]);
        assert_eq!(groups.len(), 2);
        let updates = resize_axis_group_updates(&groups, [30.0, 40.0]).unwrap();
        assert_eq!(updates.len(), 3);
        for update in &updates {
            assert_eq!(
                update.split_weights().unwrap()[1].get().to_bits(),
                0.5_f64.to_bits()
            );
        }
        let projected = fixture.project_updates(&updates);
        for (leaf, dx, dy) in [
            (leaves[0], 30.0, 40.0),
            (leaves[2], -30.0, 40.0),
            (leaves[3], 30.0, -40.0),
            (leaves[5], -30.0, -40.0),
        ] {
            close(
                projected.nodes[&leaf].rect.width(),
                fixture.projection.nodes[&leaf].rect.width() + dx,
            );
            close(
                projected.nodes[&leaf].rect.height(),
                fixture.projection.nodes[&leaf].rect.height() + dy,
            );
        }
    }
}

proptest::proptest! {
    #[test]
    fn visible_resize_frontiers_preserve_hidden_shares_for_arbitrary_masks(
        count in 2_usize..10,
        hidden_mask in 0_u16..1024,
        delta in -100_i16..100,
        central in proptest::prelude::any::<bool>(),
    ) {
        let mut builder = Workspace::builder();
        let children = (0..count).map(|index| {
            tabs(&mut builder, u64::try_from(index + 1).unwrap())
        }).collect::<Vec<_>>();
        let visible = (0..count).filter(|&index| hidden_mask & (1 << index) == 0).collect::<Vec<_>>();
        proptest::prop_assume!(visible.len() >= 2);
        let split = builder.insert_node(Node::equal_split(Axis::Horizontal, children.iter().copied()).unwrap());
        builder.set_hidden_items((0..count)
            .filter(|&index| hidden_mask & (1 << index) != 0)
            .map(|index| ItemId::new(u64::try_from(index + 1).unwrap())).collect());
        let central_leaf = central.then_some(children[count / 2]);
        let fixture = Fixture::new(
            finish(builder, split, central_leaf),
            rect(1000.0 + f64::from(u16::try_from(visible.len() - 1).unwrap()), 300.0),
            children.iter().copied().map(|node| (node, constraints(20.0, 2000.0))).collect(),
            1.0,
        );
        let handles = visible[..visible.len() - 1].iter().copied().map(|index| (split, index)).collect::<Vec<_>>();
        let groups = fixture.groups(&handles);
        let updates = resize_axis_group_updates(&groups, [f64::from(delta), 0.0]).unwrap();
        proptest::prop_assert_eq!(updates.len(), 1);
        let Node::Split { weights, .. } = fixture.workspace.node(split).unwrap() else { unreachable!() };
        for index in 0..count {
            if hidden_mask & (1 << index) != 0 {
                proptest::prop_assert_eq!(updates[0].split_weights().unwrap()[index].get().to_bits(), weights[index].get().to_bits());
            }
        }
        let projected = fixture.project_updates(&updates);
        let requested = f64::from(delta).clamp(groups[0].common_delta.minimum, groups[0].common_delta.maximum);
        for (ordinal, &index) in visible.iter().enumerate() {
            let old = fixture.projection.nodes[&children[index]].rect.width();
            let change = if ordinal == 0 { requested } else if ordinal + 1 == visible.len() { -requested } else { 0.0 };
            proptest::prop_assert!((projected.nodes[&children[index]].rect.width() - old - change).abs() < 1e-8);
        }
    }
}

fn constrained() -> (Fixture, NodeId) {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let b = tabs(&mut builder, 2);
    let split = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.25, 0.75]).unwrap());
    (
        Fixture::new(
            finish(builder, split, None),
            rect(401.0, 300.0),
            BTreeMap::from([
                (a, constraints(200.0, 400.0)),
                (b, constraints(100.0, 400.0)),
            ]),
            1.0,
        ),
        split,
    )
}

#[test]
fn zero_effective_motion_preserves_constrained_split_intent_bitwise() {
    let (fixture, split) = constrained();
    let groups = fixture.groups(&[(split, 0)]);
    assert_eq!(
        fixture
            .projection
            .splits
            .get(&split)
            .unwrap()
            .children
            .iter()
            .map(|child| child.extent)
            .collect::<Vec<_>>(),
        [200.0, 200.0]
    );
    for requested in [0.0, -48.0] {
        let updates = resize_axis_group_updates(&groups, [requested, 0.0]).unwrap();
        assert_eq!(
            updates[0]
                .split_weights()
                .unwrap()
                .iter()
                .map(|weight| weight.get().to_bits())
                .collect::<Vec<_>>(),
            [0.25_f64.to_bits(), 0.75_f64.to_bits()]
        );
    }
}

#[test]
fn returning_to_the_press_boundary_restores_the_original_intent() {
    let (fixture, split) = constrained();
    let groups = fixture.groups(&[(split, 0)]);
    let moved = resize_axis_group_updates(&groups, [50.0, 0.0]).unwrap();
    assert_eq!(
        moved[0]
            .split_weights()
            .unwrap()
            .iter()
            .map(|weight| weight.get())
            .collect::<Vec<_>>(),
        [0.625, 0.375]
    );
    let returned = resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap();
    assert_eq!(
        returned[0].split_weights().unwrap(),
        groups[0].splits[0].weights.as_slice()
    );
}

#[test]
fn discrete_round_trip_commits_visible_proportions() {
    let (fixture, split) = constrained();
    let groups = fixture.groups(&[(split, 0)]);
    let mut adjustment = ResizeAdjustment::new(50.0);
    adjustment.record(0.0);
    let updates =
        resize_axis_group_adjustment_updates(&groups, Axis::Horizontal, adjustment).unwrap();
    assert_eq!(
        updates[0]
            .split_weights()
            .unwrap()
            .iter()
            .map(|weight| weight.get())
            .collect::<Vec<_>>(),
        [0.5, 0.5]
    );
    let projected = fixture.project_updates(&updates);
    for (&node, before) in fixture.projection.nodes.iter_preorder() {
        assert_eq!(projected.nodes[&node].rect, before.rect);
    }
}

#[test]
fn discrete_clamped_adjustments_preserve_latent_shares() {
    let (fixture, split) = constrained();
    let groups = fixture.groups(&[(split, 0)]);
    for delta in [0.0, -48.0] {
        let mut adjustment = ResizeAdjustment::new(delta);
        adjustment.record(0.0);
        let updates =
            resize_axis_group_adjustment_updates(&groups, Axis::Horizontal, adjustment).unwrap();
        assert_eq!(
            updates[0]
                .split_weights()
                .unwrap()
                .iter()
                .map(|weight| weight.get().to_bits())
                .collect::<Vec<_>>(),
            [0.25_f64.to_bits(), 0.75_f64.to_bits()]
        );
    }
}

#[test]
fn discrete_round_trip_preserves_unmoved_and_sub_ulp_splits() {
    let mut builder = Workspace::builder();
    let a = tabs(&mut builder, 1);
    let b = tabs(&mut builder, 2);
    let c = tabs(&mut builder, 3);
    let d = tabs(&mut builder, 4);
    let moving = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.25, 0.75]).unwrap());
    let stationary =
        builder.insert_node(Node::split(Axis::Horizontal, [c, d], [0.25, 0.75]).unwrap());
    let outer =
        builder.insert_node(Node::split(Axis::Vertical, [moving, stationary], [0.5, 0.5]).unwrap());
    let fixture = Fixture::new(
        finish(builder, outer, None),
        rect(401.0, 601.0),
        BTreeMap::from([
            (a, constraints(200.0, 400.0)),
            (b, constraints(100.0, 400.0)),
            (c, constraints(200.0, 400.0)),
            (d, constraints(100.0, 400.0)),
        ]),
        1.0,
    );
    let frozen = fixture.groups(&[(moving, 0), (stationary, 0)]);
    let mut adjustment = ResizeAdjustment::new(50.0);
    adjustment.record(0.0);
    for coefficient in [0.0, f64::EPSILON * 0.01] {
        let mut groups = frozen.clone();
        let stationary_split = groups[0]
            .splits
            .iter_mut()
            .find(|split| split.source.node() == stationary)
            .unwrap();
        // Isolate the solver's effective-motion boundary, including nonzero
        // coefficients whose changes cannot be represented at this extent.
        stationary_split.child_coefficients = vec![coefficient, -coefficient];
        let updates =
            resize_axis_group_adjustment_updates(&groups, Axis::Horizontal, adjustment).unwrap();
        assert_eq!(updates.len(), 2);
        let moved = updates
            .iter()
            .find(|update| update.source().node() == moving)
            .unwrap();
        assert_eq!(
            moved
                .split_weights()
                .unwrap()
                .iter()
                .map(|weight| weight.get())
                .collect::<Vec<_>>(),
            [0.5, 0.5]
        );
        let untouched = updates
            .iter()
            .find(|update| update.source().node() == stationary)
            .unwrap();
        assert_eq!(
            untouched
                .split_weights()
                .unwrap()
                .iter()
                .map(|weight| weight.get().to_bits())
                .collect::<Vec<_>>(),
            [0.25_f64.to_bits(), 0.75_f64.to_bits()],
            "unmoved split normalized for coefficient {coefficient}"
        );
    }
}

#[test]
fn missing_facts_overflow_unallocated_and_compressed_gaps_are_unavailable() {
    let (fixture, split) = constrained();
    let plan = PresentationPlan::new(SURFACE, fixture.bounds);
    assert!(
        prepare_resize_axis_groups(&fixture.workspace, &plan, vec![fixture.handle(split, 0)])
            .is_none()
    );
    for (width, thickness, minimum, maximum) in [
        (101.0, 1.0, 100.0, 1000.0),
        (401.0, 1.0, 0.0, 100.0),
        (0.5, 1.0, 0.0, 1000.0),
    ] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let b = tabs(&mut builder, 2);
        let split = builder.insert_node(Node::split(Axis::Horizontal, [a, b], [0.5, 0.5]).unwrap());
        let fixture = Fixture::new(
            finish(builder, split, None),
            rect(width, 300.0),
            [a, b]
                .into_iter()
                .map(|node| (node, constraints(minimum, maximum)))
                .collect(),
            thickness,
        );
        assert!(
            prepare_resize_axis_groups(
                &fixture.workspace,
                &fixture.plan,
                vec![fixture.handle(split, 0)]
            )
            .is_none()
        );
    }
}

#[test]
fn invalid_displacements_and_duplicate_axis_groups_are_unavailable() {
    let (fixture, split) = constrained();
    let groups = fixture.groups(&[(split, 0)]);
    assert!(resize_axis_group_updates(&groups, [f64::NAN, 0.0]).is_none());
    assert!(resize_axis_group_updates(&groups, [0.0, f64::INFINITY]).is_none());
    assert!(
        resize_axis_group_updates(&[groups[0].clone(), groups[0].clone()], [1.0, 0.0]).is_none()
    );
}

#[test]
fn fractional_origins_bounds_and_gap_thickness_keep_valid_geometry_available() {
    let mut rounded_gap = false;
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for (x, y) in [(127.3, 91.7), (-127.3, -91.7), (1_000_000.3, 1_000_000.7)] {
            for thickness in [1.0, 0.3, 1.25] {
                let mut builder = Workspace::builder();
                let a = tabs(&mut builder, 1);
                let b = tabs(&mut builder, 2);
                let split = builder.insert_node(Node::split(axis, [a, b], [0.25, 0.75]).unwrap());
                let fixture = Fixture::new(
                    finish(builder, split, None),
                    LogicalRect::new(x, y, 401.25, 303.375).unwrap(),
                    [a, b]
                        .into_iter()
                        .map(|node| (node, constraints(20.0, 2000.0)))
                        .collect(),
                    thickness,
                );
                let initial = fixture.projection.splits.get(&split).unwrap();
                rounded_gap |=
                    rect_extent(initial.children[0].following_gap.unwrap(), axis) != thickness;
                let groups = fixture.groups(&[(split, 0)]);
                let delta = match axis {
                    Axis::Horizontal => [17.125, 0.0],
                    Axis::Vertical => [0.0, 17.125],
                };
                let updates = resize_axis_group_updates(&groups, delta).unwrap();
                let projected = fixture.project_updates(&updates);
                close(
                    rect_extent(projected.nodes[&a].rect, axis),
                    initial.children[0].extent + 17.125,
                );
                close(
                    rect_extent(projected.nodes[&b].rect, axis),
                    initial.children[1].extent - 17.125,
                );
            }
        }
    }
    assert!(
        rounded_gap,
        "the regression must exercise rounded max-minus-min gap extents"
    );
}

fn constraints_along(axis: Axis, minimum: f64, maximum: f64) -> Constraints {
    match axis {
        Axis::Horizontal => constraints(minimum, maximum),
        Axis::Vertical => Constraints::new(
            LogicalSize::new(20.0, minimum).unwrap(),
            LogicalSize::new(2000.0, maximum).unwrap(),
        )
        .unwrap(),
    }
}

fn bounds_along(axis: Axis, origin: f64, extent: f64) -> LogicalRect {
    match axis {
        Axis::Horizontal => LogicalRect::new(origin, 0.0, extent, 300.0).unwrap(),
        Axis::Vertical => LogicalRect::new(0.0, origin, 300.0, extent).unwrap(),
    }
}

fn delta_along(axis: Axis, delta: f64) -> [f64; 2] {
    match axis {
        Axis::Horizontal => [delta, 0.0],
        Axis::Vertical => [0.0, delta],
    }
}

#[test]
fn small_far_central_keeps_nary_resize_available_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let b = tabs(&mut builder, 2);
        let central = tabs(&mut builder, 3);
        let split = builder.insert_node(
            Node::split(axis, [a, b, central], [0.5 - 0.5e-10, 0.5 - 0.5e-10, 1e-10]).unwrap(),
        );
        let fixture = Fixture::new(
            finish(builder, split, Some(central)),
            bounds_along(axis, 0.0, 500.0),
            BTreeMap::from([
                (a, constraints_along(axis, 20.0, 2000.0)),
                (b, constraints_along(axis, 20.0, 2000.0)),
                (central, constraints_along(axis, 1e-6, 2000.0)),
            ]),
            1.0,
        );
        let initial = fixture.projection.splits.get(&split).unwrap();
        assert_eq!(initial.central_index, Some(2));
        assert_eq!(initial.children[2].extent, 1e-6);
        let original_far = fixture.projection.nodes[&central].rect;
        let groups = fixture.groups(&[(split, 0)]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].splits.len(), 1);
        assert_eq!(groups[0].splits[0].child_coefficients, [1.0, -1.0, 0.0]);
        assert!(groups[0].common_delta.minimum <= -64.0);
        assert!(groups[0].common_delta.maximum >= 64.0);
        let original_bits = groups[0].splits[0]
            .weights
            .iter()
            .map(|weight| weight.get().to_bits())
            .collect::<Vec<_>>();
        assert_eq!(
            original_bits,
            [
                (0.5_f64 - 0.5e-10).to_bits(),
                (0.5_f64 - 0.5e-10).to_bits(),
                1e-10_f64.to_bits(),
            ]
        );

        for delta in [0.0, 64.0, -64.0, 0.0] {
            let updates = resize_axis_group_updates(&groups, delta_along(axis, delta))
                .unwrap_or_else(|| panic!("{axis:?}: delta={delta} must remain available"));
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].source().node(), split);
            let weights = updates[0].split_weights().unwrap();
            assert!(
                weights
                    .iter()
                    .all(|weight| weight.get().is_finite() && weight.get() > 0.0)
            );
            let sum = weights.iter().map(|weight| weight.get()).sum::<f64>();
            assert!((sum - 1.0).abs() <= crate::graph::NORMALIZED_WEIGHT_TOLERANCE);
            if delta == 0.0 {
                assert_eq!(
                    weights
                        .iter()
                        .map(|weight| weight.get().to_bits())
                        .collect::<Vec<_>>(),
                    original_bits,
                );
            }

            let projected = fixture.project_updates(&updates);
            let resized = projected.splits.get(&split).unwrap();
            assert!(
                validate_split_projection(
                    &projected,
                    &resized,
                    axis,
                    fixture.bounds,
                    &[a, b, central],
                    weights,
                    1.0,
                    true,
                )
                .is_some()
            );
            close(
                rect_extent(projected.nodes[&a].rect, axis),
                initial.children[0].extent + delta,
            );
            close(
                rect_extent(projected.nodes[&b].rect, axis),
                initial.children[1].extent - delta,
            );
            assert_eq!(resized.children[2].extent, 1e-6);
            let far = projected.nodes[&central].rect;
            for (actual, expected) in [
                (far.min().x(), original_far.min().x()),
                (far.min().y(), original_far.min().y()),
                (far.max().x(), original_far.max().x()),
                (far.max().y(), original_far.max().y()),
            ] {
                assert!((actual - expected).abs() <= tolerance(actual, expected));
            }
        }
    }
}

#[test]
fn rounded_nested_minimum_overflow_allows_reverse_expansion_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let c = tabs(&mut builder, 2);
        let d = tabs(&mut builder, 3);
        let b = tabs(&mut builder, 4);
        let inner = builder.insert_node(Node::split(axis, [c, d], [0.5, 0.5]).unwrap());
        let orthogonal = match axis {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
        let across = builder.insert_node(Node::split(orthogonal, [b, inner], [0.5, 0.5]).unwrap());
        // Pin valid complementary shares so rectangle rounding produces a positive
        // nested minimum residual independently of constructor normalization.
        let outer = builder.insert_node(Node::Split {
            layout_title: None,
            axis,
            children: vec![a, across],
            weights: vec![
                SplitWeight::new(0.9).unwrap(),
                SplitWeight::new(1.0_f64 - 0.9_f64).unwrap(),
            ],
        });
        let fixture = Fixture::new(
            finish(builder, outer, None),
            bounds_along(axis, 0.0, 1001.0),
            BTreeMap::from([
                (a, constraints_along(axis, 20.0, 2000.0)),
                (b, constraints_along(axis, 20.0, 2000.0)),
                (c, constraints_along(axis, 200.3, 2000.0)),
                (d, constraints_along(axis, 50.0, 2000.0)),
            ]),
            1.0,
        );
        close(
            rect_extent(fixture.projection.nodes[&inner].rect, axis),
            251.3,
        );
        let residual = fixture.projection.splits.get(&inner).unwrap().overflow;
        assert!(residual > 0.0 && residual < 1e-10, "{axis:?}: {residual}");
        let groups = fixture.groups(&[(outer, 0)]);
        let updates = resize_axis_group_updates(&groups, delta_along(axis, -10.0)).unwrap();
        assert_eq!(updates.len(), 2);
        let projected = fixture.project_updates(&updates);
        close(rect_extent(projected.nodes[&c].rect, axis), 210.3);
        close(rect_extent(projected.nodes[&d].rect, axis), 50.0);
        let far = projected.nodes[&d].rect;
        let original_far = fixture.projection.nodes[&d].rect;
        close(far.x(), original_far.x());
        close(far.y(), original_far.y());
        for update in resize_axis_group_updates(&groups, delta_along(axis, 10.0)).unwrap() {
            let Node::Split { weights, .. } =
                fixture.workspace.node(update.source().node()).unwrap()
            else {
                unreachable!();
            };
            assert_eq!(update.split_weights().unwrap(), weights.as_slice());
        }
    }
}

#[test]
fn rounded_nested_maximum_unallocated_allows_reverse_shrink_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let c = tabs(&mut builder, 2);
        let d = tabs(&mut builder, 3);
        let b = tabs(&mut builder, 4);
        let inner = builder.insert_node(Node::split(axis, [c, d], [0.5, 0.5]).unwrap());
        let orthogonal = match axis {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
        let across = builder.insert_node(Node::split(orthogonal, [b, inner], [0.5, 0.5]).unwrap());
        let outer = builder.insert_node(Node::split(axis, [a, across], [0.5, 0.5]).unwrap());
        let fixture = Fixture::new(
            finish(builder, outer, None),
            bounds_along(axis, 0.0, 1001.0),
            BTreeMap::from([
                (a, constraints_along(axis, 20.0, 2000.0)),
                (b, constraints_along(axis, 20.0, 2000.0)),
                (c, constraints_along(axis, 20.0, 200.1)),
                (d, constraints_along(axis, 20.0, 50.0)),
            ]),
            1.0,
        );
        close(
            rect_extent(fixture.projection.nodes[&inner].rect, axis),
            251.1,
        );
        let residual = fixture.projection.splits.get(&inner).unwrap().unallocated;
        assert!(residual > 0.0 && residual < 1e-10, "{axis:?}: {residual}");
        let groups = fixture.groups(&[(outer, 0)]);
        let updates = resize_axis_group_updates(&groups, delta_along(axis, 10.0)).unwrap();
        assert_eq!(updates.len(), 2);
        let projected = fixture.project_updates(&updates);
        close(rect_extent(projected.nodes[&c].rect, axis), 190.1);
        close(rect_extent(projected.nodes[&d].rect, axis), 50.0);
        let far = projected.nodes[&d].rect;
        let original_far = fixture.projection.nodes[&d].rect;
        close(far.x(), original_far.x());
        close(far.y(), original_far.y());
        for update in resize_axis_group_updates(&groups, delta_along(axis, -10.0)).unwrap() {
            let Node::Split { weights, .. } =
                fixture.workspace.node(update.source().node()).unwrap()
            else {
                unreachable!();
            };
            assert_eq!(update.split_weights().unwrap(), weights.as_slice());
        }
    }
}

#[test]
fn large_origin_rounded_minimum_allows_reverse_expansion_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let b = tabs(&mut builder, 2);
        let split = builder.insert_node(Node::split(axis, [a, b], [0.1, 0.9]).unwrap());
        let fixture = Fixture::new(
            finish(builder, split, None),
            bounds_along(axis, 1_000_000.3, 1001.0),
            BTreeMap::from([
                (a, constraints_along(axis, 200.1, 2000.0)),
                (b, constraints_along(axis, 20.0, 2000.0)),
            ]),
            1.0,
        );
        let actual = rect_extent(fixture.projection.nodes[&a].rect, axis);
        assert!(
            actual < 200.1 && 200.1 - actual < 1e-7,
            "{axis:?}: {actual}"
        );
        let groups = fixture.groups(&[(split, 0)]);
        let updates = resize_axis_group_updates(&groups, delta_along(axis, 10.0)).unwrap();
        let projected = fixture.project_updates(&updates);
        close(rect_extent(projected.nodes[&a].rect, axis), 210.1);
        close(rect_extent(projected.nodes[&b].rect, axis), 789.9);
        let blocked = resize_axis_group_updates(&groups, delta_along(axis, -10.0)).unwrap();
        assert_eq!(
            blocked[0].split_weights().unwrap(),
            groups[0].splits[0].weights.as_slice()
        );
    }
}

#[test]
fn large_origin_rounded_maximum_allows_reverse_shrink_on_both_axes() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let b = tabs(&mut builder, 2);
        let split = builder.insert_node(Node::split(axis, [a, b], [0.5, 0.5]).unwrap());
        let fixture = Fixture::new(
            finish(builder, split, None),
            bounds_along(axis, 1_000_000.3, 1001.0),
            BTreeMap::from([
                (a, constraints_along(axis, 20.0, 200.3)),
                (b, constraints_along(axis, 20.0, 2000.0)),
            ]),
            1.0,
        );
        let actual = rect_extent(fixture.projection.nodes[&a].rect, axis);
        assert!(
            actual > 200.3 && actual - 200.3 < 1e-7,
            "{axis:?}: {actual}"
        );
        let groups = fixture.groups(&[(split, 0)]);
        let updates = resize_axis_group_updates(&groups, delta_along(axis, -10.0)).unwrap();
        let projected = fixture.project_updates(&updates);
        close(rect_extent(projected.nodes[&a].rect, axis), 190.3);
        close(rect_extent(projected.nodes[&b].rect, axis), 809.7);
        let blocked = resize_axis_group_updates(&groups, delta_along(axis, 10.0)).unwrap();
        assert_eq!(
            blocked[0].split_weights().unwrap(),
            groups[0].splits[0].weights.as_slice()
        );
    }
}

#[test]
fn large_origin_real_overflow_unallocated_and_compressed_gaps_remain_unavailable() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for (extent, minimum, maximum) in [
            (401.0, 200.1, 2000.0),
            (401.0, 20.0, 199.9),
            (0.5, 0.0, 2000.0),
        ] {
            let mut builder = Workspace::builder();
            let a = tabs(&mut builder, 1);
            let b = tabs(&mut builder, 2);
            let split = builder.insert_node(Node::split(axis, [a, b], [0.5, 0.5]).unwrap());
            let fixture = Fixture::new(
                finish(builder, split, None),
                bounds_along(axis, 1_000_000.3, extent),
                [a, b]
                    .into_iter()
                    .map(|node| (node, constraints_along(axis, minimum, maximum)))
                    .collect(),
                1.0,
            );
            let projected = fixture.projection.splits.get(&split).unwrap();
            if extent == 0.5 {
                close(
                    rect_extent(projected.children[0].following_gap.unwrap(), axis),
                    0.5,
                );
            } else if minimum > 200.0 {
                assert!(projected.overflow > 0.1);
            } else {
                assert!(projected.unallocated > 0.1);
            }
            assert!(
                prepare_resize_axis_groups(
                    &fixture.workspace,
                    &fixture.plan,
                    vec![fixture.handle(split, 0)],
                )
                .is_none(),
                "{axis:?}: extent={extent}, min={minimum}, max={maximum}"
            );
        }
    }
}
