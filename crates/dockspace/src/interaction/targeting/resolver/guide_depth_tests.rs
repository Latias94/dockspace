//! Nested guide scope follows frozen structural depth instead of arena identity.

use super::*;
use crate::geometry::LogicalPoint;
use crate::graph::Node;
use crate::ids::{ItemId, NodeId};
use crate::interaction::targeting::guide::DropGuideTargetRecord;

#[test]
fn overlapping_inner_guides_choose_the_deepest_scope_before_node_identity() {
    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let bounds = LogicalRect::new(0.0, 0.0, 400.0, 300.0).unwrap();
    let mut builder = Workspace::builder();
    let older = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let newer = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let cluster = |node: NodeId, depth, layer| {
        let target = Arc::new(DropTargetRecord::new(
            DropTargetLocation::Center {
                surface,
                root,
                tabs: node,
            },
            DropTargetAvailability::Available,
            HitRegion::new(bounds),
            layer,
            DropVisual::new(bounds),
        ));
        DropGuideClusterRecord::inner_center(
            surface,
            root,
            node,
            HitRegion::new(bounds),
            layer,
            DropGuideTargetRecord::new(target, bounds),
        )
        .with_depth(depth)
    };
    for (outer, inner) in [(older, newer), (newer, older)] {
        let mut plan = PresentationPlan::new(surface, bounds);
        plan.push_drop_guide_cluster(cluster(outer, 1, SceneLayerKey::new(1)));
        plan.push_drop_guide_cluster(cluster(inner, 2, SceneLayerKey::new(1)));
        let selected = select_guide_clusters(
            &plan,
            LogicalPoint::new(100.0, 100.0).unwrap(),
            None,
            DropSourceSuppression::default(),
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].id(),
            DropGuideClusterId::inner(surface, root, inner)
        );
    }

    let mut plan = PresentationPlan::new(surface, bounds);
    plan.push_drop_guide_cluster(cluster(older, 1, SceneLayerKey::new(2)));
    plan.push_drop_guide_cluster(cluster(newer, 9, SceneLayerKey::new(1)));
    let selected = select_guide_clusters(
        &plan,
        LogicalPoint::new(100.0, 100.0).unwrap(),
        None,
        DropSourceSuppression::default(),
    );
    assert_eq!(
        selected[0].id(),
        DropGuideClusterId::inner(surface, root, older)
    );
}
