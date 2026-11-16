use super::*;
use crate::graph::{RootRecord, SurfacePresentation};
use crate::ids::{ItemId, SurfaceId};
use crate::model::RootAllocation;

const ROOT: RootId = RootId::new(1);

fn project(workspace: &Workspace) -> LayoutProjection {
    project_root(
        workspace,
        ROOT,
        LogicalRect::new(11.0, 17.0, 1001.0, 600.0).unwrap(),
        |node| {
            assert!(workspace.node_has_visible_content(node));
            Some(
                Constraints::new(
                    LogicalSize::new(10.0, 10.0).unwrap(),
                    LogicalSize::new(2000.0, 2000.0).unwrap(),
                )
                .unwrap(),
            )
        },
        LayoutMetrics::new(1.0).unwrap(),
    )
    .unwrap()
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn hidden_middle_child_keeps_durable_indices_without_reserving_space_or_a_gap() {
    let mut builder = Workspace::builder();
    let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let c = builder.insert_node(Node::tabs([ItemId::new(3)]));
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, [a, b, c], [0.2, 0.3, 0.5]).unwrap());
    builder.set_root(ROOT, RootRecord::new(split));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    let mut workspace = builder.build().unwrap();
    let original = project(&workspace);
    workspace
        .replace_hidden_items([ItemId::new(2)].into())
        .unwrap();
    let hidden = project(&workspace);
    let projected = hidden.splits.get(&split).unwrap();
    assert_eq!(
        projected.visible_child_indices().collect::<Vec<_>>(),
        [0, 2]
    );
    assert_eq!(projected.following_child_index(0), Some(2));
    assert_eq!(projected.following_child_index(1), None);
    assert_eq!(projected.splitter_rects().count(), 1);
    assert!(!hidden.nodes[&b].visible);
    assert_eq!(hidden.nodes[&b].rect.width(), 0.0);
    assert_eq!(hidden.nodes[&b].rect.height(), 0.0);
    close(hidden.nodes[&a].rect.width(), 1000.0 * 2.0 / 7.0);
    close(hidden.nodes[&c].rect.width(), 1000.0 * 5.0 / 7.0);
    close(
        hidden.nodes[&a].rect.max().x() + 1.0,
        hidden.nodes[&c].rect.x(),
    );
    workspace.replace_hidden_items(Default::default()).unwrap();
    assert_eq!(project(&workspace), original);
}

#[test]
fn fully_hidden_subtrees_do_not_compose_constraints_or_nested_gaps() {
    let mut builder = Workspace::builder();
    let a = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let b = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let c = builder.insert_node(Node::tabs([ItemId::new(3)]));
    let nested = builder.insert_node(Node::split(Axis::Vertical, [a, b], [0.4, 0.6]).unwrap());
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, [nested, c], [0.7, 0.3]).unwrap());
    builder.set_root(ROOT, RootRecord::new(split));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    builder.set_hidden_items([ItemId::new(1), ItemId::new(2)].into());
    let workspace = builder.build().unwrap();
    let projected = project(&workspace);
    for node in [a, b, nested] {
        assert!(!projected.nodes[&node].visible);
        assert_eq!(
            projected.nodes[&node].constraints.min(),
            LogicalSize::new(0.0, 0.0).unwrap()
        );
        assert_eq!(projected.nodes[&node].rect.width(), 0.0);
        assert_eq!(projected.nodes[&node].rect.height(), 0.0);
    }
    assert_eq!(
        projected
            .splits
            .get(&nested)
            .unwrap()
            .splitter_rects()
            .count(),
        0
    );
    assert_eq!(
        projected
            .splits
            .get(&split)
            .unwrap()
            .splitter_rects()
            .count(),
        0
    );
    close(projected.nodes[&c].rect.width(), 1001.0);
    close(projected.nodes[&c].rect.height(), 600.0);
}

#[test]
fn hidden_central_ownership_does_not_participate_in_visible_allocation() {
    for allocation in [RootAllocation::Weighted, RootAllocation::PreferCentral] {
        let mut builder = Workspace::builder();
        let central = builder.insert_node(Node::tabs([ItemId::new(1)]));
        let a = builder.insert_node(Node::tabs([ItemId::new(2)]));
        let b = builder.insert_node(Node::tabs([ItemId::new(3)]));
        let split = builder
            .insert_node(Node::split(Axis::Horizontal, [central, a, b], [0.2, 0.3, 0.5]).unwrap());
        builder.set_root(
            ROOT,
            RootRecord::new(split)
                .with_central(central)
                .with_allocation(allocation),
        );
        builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
        builder.set_hidden_items([ItemId::new(1)].into());
        let mut workspace = builder.build().unwrap();
        let projected = project(&workspace);
        let split_projection = projected.splits.get(&split).unwrap();
        assert_eq!(split_projection.central_index, None);
        assert_eq!(split_projection.following_child_index(1), Some(2));
        close(projected.nodes[&a].rect.width(), 375.0);
        close(projected.nodes[&b].rect.width(), 625.0);
        assert_eq!(workspace.root(ROOT).unwrap().central, Some(central));
        workspace.replace_hidden_items(Default::default()).unwrap();
        let restored = project(&workspace);
        assert_eq!(
            restored.splits.get(&split).unwrap().central_index,
            (allocation == RootAllocation::PreferCentral).then_some(0)
        );
        close(restored.nodes[&central].rect.width(), 999.0 * 0.2);
        close(restored.nodes[&a].rect.width(), 999.0 * 0.3);
        close(restored.nodes[&b].rect.width(), 999.0 * 0.5);
    }
}

#[test]
fn empty_central_leaf_remains_a_visible_destination() {
    let mut builder = Workspace::builder();
    let central = builder.insert_node(Node::tabs([]));
    builder.set_root(ROOT, RootRecord::new(central).with_central(central));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    let workspace = builder.build().unwrap();
    let projected = project(&workspace);
    assert!(projected.nodes[&central].visible);
    close(projected.nodes[&central].rect.width(), 1001.0);
}
