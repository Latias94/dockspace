//! Nested guide hit precedence agrees with the resolver's active scope.

use super::*;
use crate::geometry::LogicalPoint;
use crate::graph::{Node, Workspace};
use crate::ids::{ItemId, NodeId};
use crate::interaction::targeting::guide::DropGuideTargetRecord;
use crate::interaction::targeting::targets::{
    DropTargetAvailability, DropTargetLocation, DropTargetRecord, DropVisual,
};

fn center_cluster(node: NodeId, depth: usize, layer: SceneLayerKey) -> DropGuideClusterRecord {
    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let bounds = LogicalRect::new(0.0, 0.0, 400.0, 300.0).unwrap();
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
}

#[test]
fn overlapping_nested_guide_hits_choose_depth_before_arena_identity() {
    let mut builder = Workspace::builder();
    let older = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let newer = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let point = LogicalPoint::new(100.0, 100.0).unwrap();
    let surface = SurfaceId::new(1);
    let root = RootId::new(1);

    for (outer, inner) in [(older, newer), (newer, older)] {
        for reverse in [false, true] {
            let mut regions = Vec::new();
            let mut clusters = [
                center_cluster(outer, 1, SceneLayerKey::new(1)),
                center_cluster(inner, 2, SceneLayerKey::new(1)),
            ];
            if reverse {
                clusters.reverse();
            }
            for cluster in &clusters {
                push_guide_regions(&mut |region| regions.push(region), surface, cluster);
            }
            assert_eq!(
                resolve_exclusive_regions(
                    &regions,
                    PresentationPointerLane::HoverDrop,
                    point,
                    DropSourceSuppression::default(),
                )
                .expect("nested guide scopes have deterministic precedence")
                .map(|region| region.id().kind()),
                Some(PresentationHitRegionKind::DropTarget(
                    DropTargetId::Center {
                        surface,
                        root,
                        tabs: inner,
                    }
                )),
            );
        }
    }
}

#[test]
fn nested_guide_depth_does_not_override_foreground_layers_or_unrelated_ambiguity() {
    let mut builder = Workspace::builder();
    let foreground = builder.insert_node(Node::tabs([ItemId::new(1)]));
    let background = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let surface = SurfaceId::new(1);
    let point = LogicalPoint::new(100.0, 100.0).unwrap();
    let mut regions = Vec::new();
    for cluster in [
        center_cluster(background, 9, SceneLayerKey::new(1)),
        center_cluster(foreground, 1, SceneLayerKey::new(2)),
    ] {
        push_guide_regions(&mut |region| regions.push(region), surface, &cluster);
    }
    let winner = resolve_exclusive_regions(
        &regions,
        PresentationPointerLane::HoverDrop,
        point,
        DropSourceSuppression::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(winner.stack().layer(), SceneLayerKey::new(2));

    regions.clear();
    for node in [foreground, background] {
        push_guide_regions(
            &mut |region| regions.push(region),
            surface,
            &center_cluster(node, 1, SceneLayerKey::new(1)),
        );
    }
    assert!(matches!(
        resolve_exclusive_regions(
            &regions,
            PresentationPointerLane::HoverDrop,
            point,
            DropSourceSuppression::default(),
        ),
        Err(PresentationHitResolutionError::Ambiguous { .. })
    ));
}
