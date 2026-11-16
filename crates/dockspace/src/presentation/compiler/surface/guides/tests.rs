use super::*;

fn rect(x1: f64, y1: f64, x2: f64, y2: f64) -> LogicalRect {
    LogicalRect::from_min_max(
        LogicalPoint::new(x1, y1).unwrap(),
        LogicalPoint::new(x2, y2).unwrap(),
    )
    .unwrap()
}

fn button(bounds: LogicalRect) -> GuideButtonGeometry {
    GuideButtonGeometry {
        draw: bounds,
        hit: bounds,
    }
}

#[test]
fn tiny_edge_previews_stay_representable_and_contained_at_any_origin() {
    for (x, y, extent) in [
        (0.0, 0.0, 800.0),
        (-400.0, -300.0, 800.0),
        (1e12, -1e12, 800.0),
        (1e-100, -1e-100, 1e-100),
    ] {
        let bounds = LogicalRect::new(x, y, extent, extent).unwrap();
        for fraction in [f64::from_bits(1), 1e-50, 0.3, 1.0_f64.next_down()] {
            for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                let preview =
                    edge_preview(bounds, edge, DockFraction::new(fraction).unwrap()).unwrap();
                assert!(rect_has_area(preview), "{bounds:?}, {fraction}, {edge:?}");
                assert!(
                    rect_contains(bounds, preview),
                    "{bounds:?}, {fraction}, {edge:?}"
                );
                match edge {
                    Edge::Left | Edge::Top => assert_eq!(preview.min(), bounds.min()),
                    Edge::Right | Edge::Bottom => assert_eq!(preview.max(), bounds.max()),
                }
            }
        }
    }
}

#[test]
fn inner_hit_cells_keep_shared_fractional_boundaries_exact() {
    let bounds = rect(5.6, 5.6, 60.0, 60.0);
    let center = button(rect(26.8, 26.8, 38.0, 38.0));
    let mut edges = GuideEdgeGeometry {
        left: button(rect(5.6, 26.8, 30.0, 38.0)),
        right: button(rect(35.0, 26.8, 60.0, 38.0)),
        top: button(rect(26.8, 5.6, 38.0, 30.0)),
        bottom: button(rect(26.8, 35.0, 38.0, 60.0)),
    };
    separate_inner_cross_hits(bounds, center, &mut edges).unwrap();
    let mut buttons = Vec::from(edges.buttons());
    buttons.push(center);
    assert!(!guide_hits_overlap(&buttons));
    assert_eq!(edges.left.hit.max().x(), center.hit.x());
    assert_eq!(edges.top.hit.max().y(), center.hit.y());
}

#[test]
fn outer_hit_cells_keep_shared_fractional_boundaries_exact() {
    let bounds = rect(5.6, 5.6, 60.0, 60.0);
    let mut edges = GuideEdgeGeometry {
        left: button(rect(5.6, 26.8, 30.0, 38.0)),
        right: button(rect(35.0, 26.8, 60.0, 38.0)),
        top: button(rect(26.8, 5.6, 38.0, 30.0)),
        bottom: button(rect(26.8, 35.0, 38.0, 60.0)),
    };
    separate_outer_cross_hits(bounds, &mut edges).unwrap();
    assert!(!guide_hits_overlap(&edges.buttons()));
    assert_eq!(edges.left.hit.max().x(), edges.top.hit.x());
    assert_eq!(edges.top.hit.max().y(), edges.left.hit.y());
}

#[test]
fn inner_guides_do_not_fail_the_scene_for_an_unrepresentable_occlusion_sliver() {
    use crate::graph::{Node, Workspace};
    use crate::ids::{FloatingPresentationId, ItemId};
    use crate::presentation::manifest::PolicyRevision;

    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let surface_bounds = rect(0.0, 0.0, 1280.0, 820.0);
    let node_bounds = rect(833.4, 595.6, 1280.0, 820.0);
    let content = rect(833.4, 619.6, 1280.0, 820.0);
    let config = DockPresentationConfig::default();
    let policy = crate::policy::DockPolicy::default().snapshot(PolicyRevision::new(1));
    let layer = SceneLayerKey::surface_base();
    let mut workspace = Workspace::builder();
    let tabs = workspace.insert_node(Node::tabs([ItemId::new(1), ItemId::new(2)]));

    for (occlusion_top, visible_guide) in [
        (None, true),
        (Some(content.y() + 24.0), true),
        (Some(content.y().next_up()), false),
    ] {
        let mut ready = PresentationPlan::new(surface, surface_bounds);
        if let Some(top) = occlusion_top {
            ready.push_drop_occlusion(DropOcclusionRecord::new(
                FloatingPresentationId::new(1),
                HitRegion::new(rect(content.x(), top, content.max().x(), content.max().y())),
                SceneLayerKey::contained(0).unwrap(),
            ));
        }
        let outer = outer_guide_geometry(
            &config,
            surface_bounds,
            DropGuideClusterId::outer(surface, root),
            layer,
            &ready,
        )
        .unwrap()
        .unwrap();
        let inner = compile_inner_targets_and_guide(
            &policy,
            &config,
            outer,
            surface,
            root,
            tabs,
            node_bounds,
            content,
            layer,
            DockFraction::new(0.5).unwrap(),
            false,
            &mut ready,
        )
        .expect("foreground occlusion must not turn representable pane bounds into a fatal scene");
        assert_eq!(
            inner.is_some(),
            visible_guide,
            "occlusion_top={occlusion_top:?}"
        );
        assert_eq!(
            ready.drop_targets().len(),
            if visible_guide { 5 } else { 0 },
            "an omitted guide must not leave a partially installed cluster",
        );
    }
}

#[test]
fn guide_placement_keeps_a_representable_cell_when_a_larger_scale_collapses() {
    let bounds = rect(0.0, 0.0, 1e12, 80.0);
    let obstacle = rect(1e-11, 1e-10, bounds.max().x(), bounds.max().y());
    let config = DockPresentationConfig::default();
    let metrics =
        GuideMetrics::compact(&config, bounds, config.inner_guide_reference_span()).unwrap();
    let half_span = metrics.half_extent + metrics.hit_padding + metrics.inner_offset;
    let (cell, geometry) = guide_placement(
        bounds,
        half_span,
        half_span,
        LogicalPoint::new(5e11, 40.0).unwrap(),
        [obstacle],
        |placement| {
            let geometry = inner_guide_geometry(
                placement.cell,
                placement.center_x,
                placement.center_y,
                metrics.scaled(placement.scale),
                false,
            )?;
            Ok(geometry
                .is_complete(bounds)
                .then_some((placement.cell, geometry)))
        },
    )
    .unwrap()
    .expect("the narrow cell near the origin still represents all five buttons");
    assert_eq!(cell, rect(0.0, 0.0, obstacle.x(), bounds.max().y()));
    assert!(geometry.is_complete(cell));
    assert!(!rects_overlap(cell, obstacle));

    // The wide top strip has a larger scale, but its center button's width is
    // smaller than one x-coordinate ULP. It must not replace the valid cell.
    let top = rect(0.0, 0.0, bounds.max().x(), obstacle.y());
    let scale = top.height() / (2.0 * half_span);
    let geometry =
        inner_guide_geometry(top, 5e11, top.height() * 0.5, metrics.scaled(scale), false).unwrap();
    assert!(!geometry.is_complete(top));
}

#[test]
fn intrinsic_inner_guide_failure_stays_typed_without_installing_partial_targets() {
    use crate::graph::{Node, Workspace};
    use crate::ids::{FloatingPresentationId, ItemId};
    use crate::presentation::manifest::PolicyRevision;

    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let bounds = rect(833.4, 619.6, 1280.0, 820.0);
    let policy = crate::policy::DockPolicy::default().snapshot(PolicyRevision::new(1));
    let mut workspace = Workspace::builder();
    let tabs = workspace.insert_node(Node::tabs([ItemId::new(1)]));
    let config = DockPresentationConfig::builder()
        .guide_extent(1e-300)
        .guide_gap(1e-300)
        .guide_hit_padding(0.0)
        .guide_outer_inset(1e-300)
        .build()
        .unwrap();
    for covered in [false, true] {
        let mut ready = PresentationPlan::new(surface, bounds);
        let outer = outer_guide_geometry(
            &DockPresentationConfig::default(),
            bounds,
            DropGuideClusterId::outer(surface, root),
            SceneLayerKey::surface_base(),
            &ready,
        )
        .unwrap()
        .unwrap();
        if covered {
            ready.push_drop_occlusion(DropOcclusionRecord::new(
                FloatingPresentationId::new(1),
                HitRegion::new(bounds),
                SceneLayerKey::contained(0).unwrap(),
            ));
        }
        let result = compile_inner_targets_and_guide(
            &policy,
            &config,
            outer,
            surface,
            root,
            tabs,
            bounds,
            bounds,
            SceneLayerKey::surface_base(),
            DockFraction::new(0.5).unwrap(),
            false,
            &mut ready,
        );
        assert!(matches!(
            result,
            Err(SceneCompilationError::UnrepresentableGuideGeometry {
                cluster,
                bounds: failed,
            }) if cluster == DropGuideClusterId::inner(surface, root, tabs) && failed == bounds
        ));
        assert!(ready.drop_targets().is_empty());
    }
}

#[test]
fn occlusion_slivers_keep_latent_surface_targets_and_omit_unplaceable_outer_guides() {
    use crate::ids::FloatingPresentationId;

    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let bounds = rect(833.4, 619.6, 1280.0, 820.0);
    let config = DockPresentationConfig::default();
    let mut ready = PresentationPlan::new(surface, bounds);
    ready.push_drop_occlusion(DropOcclusionRecord::new(
        FloatingPresentationId::new(1),
        HitRegion::new(rect(
            bounds.x(),
            bounds.y().next_up(),
            bounds.max().x(),
            bounds.max().y(),
        )),
        SceneLayerKey::contained(0).unwrap(),
    ));
    assert!(
        outer_guide_geometry(
            &config,
            bounds,
            DropGuideClusterId::outer(surface, root),
            SceneLayerKey::surface_base(),
            &ready,
        )
        .unwrap()
        .is_none(),
    );
    compile_surface_background(&config, surface, bounds, true, &mut ready).unwrap();
    let target = ready.surface_background().unwrap();
    assert!(rect_has_area(target.region().rect()));
    assert!(rect_contains(bounds, target.region().rect()));
    assert_eq!(ready.drop_guide_clusters().len(), 1);
}
