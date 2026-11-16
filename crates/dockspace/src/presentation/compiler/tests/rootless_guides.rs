use super::*;
use crate::graph::command::{MovePayload, WorkspaceCommand};
use crate::interaction::targeting::guide::{DropGuideScope, DropGuideSlot};
use crate::interaction::targeting::resolver::{
    DropResolution, DropTargeting, resolve_presented_drop_with_targeting,
};
use crate::interaction::targeting::targets::{DropTargetKind, DropTargetRecord};
use crate::presentation::scene::ContainedLayoutOwner;

const SURFACE: SurfaceId = SurfaceId::new(1);
const SOURCE_ROOT: RootId = RootId::new(1);
const OTHER_ROOT: RootId = RootId::new(2);
const SOURCE_FLOATING: FloatingPresentationId = FloatingPresentationId::new(1);
const OTHER_FLOATING: FloatingPresentationId = FloatingPresentationId::new(2);

fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
    LogicalRect::new(x, y, width, height).unwrap()
}

fn fixture(other_rect: LogicalRect) -> Workspace {
    let mut builder = Workspace::builder();
    for (root, item, floating, bounds) in [
        (OTHER_ROOT, ItemId::new(2), OTHER_FLOATING, other_rect),
        (
            SOURCE_ROOT,
            ItemId::new(1),
            SOURCE_FLOATING,
            rect(80.0, 50.0, 600.0, 450.0),
        ),
    ] {
        let tabs = builder.insert_node(Node::tabs([item]));
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_contained_floating(floating, ContainedFloating::new(root, bounds));
    }
    builder.set_surface(SURFACE, SurfacePresentation::rootless());
    builder.attach_contained(SURFACE, OTHER_FLOATING).unwrap();
    builder.attach_contained(SURFACE, SOURCE_FLOATING).unwrap();
    builder.build().unwrap()
}

fn surface_guide(
    plan: &PresentationPlan,
) -> &crate::interaction::targeting::guide::DropGuideTargetRecord {
    let cluster = plan
        .drop_guide_clusters()
        .iter()
        .find(|cluster| cluster.id().scope() == DropGuideScope::Surface)
        .unwrap();
    assert_eq!(cluster.id().root(), None);
    assert_eq!(cluster.targets().count(), 1);
    cluster.target(DropGuideSlot::Center).unwrap()
}

#[test]
fn rootless_surface_guide_ignores_moving_source_but_respects_other_floating_occlusion() {
    let bounds = rect(0.0, 0.0, 800.0, 600.0);
    let policy = policy_snapshot();
    let config = DockPresentationConfig::default();
    let session = DragSessionId::new(WorkspaceEpoch::new(3), DragGeneration::new(1));
    for other_rect in [rect(330.0, 240.0, 140.0, 120.0), bounds] {
        let workspace = fixture(other_rect);
        let baseline = compile_default_ready_plan(&workspace, SURFACE, bounds);
        let manifest = derive_scene_requirement_draft(
            authority_domain(),
            &workspace,
            version(),
            PresentationConfigRevision::new(7),
            &policy,
            RequirementRevision::new(9),
            &surface_revisions([SURFACE]),
        )
        .unwrap()
        .finalize(PopupPlaneRequirement::default())
        .unwrap();
        let root = workspace.root(SOURCE_ROOT).unwrap();
        let source = workspace
            .capture_node_source(SOURCE_ROOT, root.node)
            .unwrap();
        let source_rect = workspace.contained_floating(SOURCE_FLOATING).unwrap().rect;
        let mut guide_geometry = None;
        for (revision, source_position) in [
            rect(40.0, 30.0, 600.0, 450.0),
            rect(100.0, 90.0, 600.0, 450.0),
            rect(200.0, 120.0, 600.0, 450.0),
        ]
        .into_iter()
        .enumerate()
        {
            let plan = compile_surface_plan(
                &workspace,
                version(),
                &policy,
                &config,
                &manifest,
                baseline.layout_facts().unwrap().measurements(),
                &TabStripStateStore::default(),
                &[],
                Some(ContainedLayoutProjection {
                    owner: ContainedLayoutOwner::Drag(session),
                    surface: SURFACE,
                    root: SOURCE_ROOT,
                    floating: SOURCE_FLOATING,
                    source_rect,
                    rect: source_position,
                    target_basis: None,
                }),
            )
            .unwrap()
            .finalize()
            .unwrap();
            let guide = surface_guide(&plan);
            let geometry = (guide.draw(), guide.hit());
            assert_eq!(*guide_geometry.get_or_insert(geometry), geometry);
            let hit = guide.hit().rect();
            let point =
                LogicalPoint::new(hit.x() + hit.width() * 0.5, hit.y() + hit.height() * 0.5)
                    .unwrap();
            assert!(
                source_position.contains(point),
                "the source covers its own destination guide"
            );
            let query = resolve_presented_drop_with_targeting(
                SurfaceSceneStamp::new(
                    plan.measurement_ticket().unwrap(),
                    SurfaceSceneRevision::new(u64::try_from(revision + 1).unwrap()),
                ),
                &plan,
                baseline.layout_facts(),
                &workspace,
                version(),
                manifest.workspace_index(),
                &policy,
                session,
                MovePayload::Tabs(source.clone()),
                None,
                point,
                DropTargeting::Explicit,
            )
            .unwrap();
            if other_rect == bounds {
                assert!(other_rect.contains(point));
                assert!(query.affordance().is_some_and(|affordance| {
                    affordance
                        .clusters()
                        .iter()
                        .all(|cluster| cluster.id().root() == Some(OTHER_ROOT))
                }));
                assert!(
                    !matches!(query.resolution(), DropResolution::Resolved(drop)
                    if drop.target_id().kind() == DropTargetKind::SurfaceBackground),
                    "a non-source floating window must block the latent background guide"
                );
            } else {
                assert!(
                    !other_rect.contains(point),
                    "the guide avoids the non-source window"
                );
                let DropResolution::Resolved(drop) = query.resolution() else {
                    panic!("the moving source must not occlude its surface guide: {query:?}");
                };
                assert!(matches!(
                    drop.command(),
                    WorkspaceCommand::PromoteContained {
                        surface: SURFACE,
                        floating: SOURCE_FLOATING,
                        ..
                    }
                ));
            }
        }
    }
}

#[test]
fn rootless_surface_guide_keeps_its_canonical_target_when_other_guides_are_rebound() {
    let workspace = fixture(rect(500.0, 350.0, 200.0, 150.0));
    let original = compile_default_ready_plan(&workspace, SURFACE, rect(0.0, 0.0, 800.0, 600.0));
    let frozen = original.clone();
    let mut disabled = DockPolicy::default();
    disabled.set_allow_edge_split(false);
    let disabled = disabled.snapshot(PolicyRevision::new(9));
    let current = PresentationPlanValidator::new(&workspace, &disabled)
        .unwrap()
        .validate_and_canonicalize(original.clone())
        .unwrap();

    let mut rebound = 0;
    for target in current.drop_targets() {
        let previous = original
            .drop_targets()
            .iter()
            .find(|old| old.id() == target.id())
            .unwrap();
        if matches!(
            target.id().kind(),
            DropTargetKind::InnerEdge | DropTargetKind::OuterEdge
        ) {
            rebound += 1;
            assert!(matches!(
                target.availability(),
                DropTargetAvailability::Unavailable(_)
            ));
            assert_eq!(previous.availability(), DropTargetAvailability::Available);
            assert!(!Arc::ptr_eq(target, previous));
        }
    }
    assert!(
        rebound > 0,
        "policy changes must exercise canonical guide rebinding"
    );
    for plan in [&original, &current] {
        assert!(std::ptr::eq(
            surface_guide(plan).target(),
            plan.surface_background().unwrap()
        ));
        for cluster in plan.drop_guide_clusters() {
            for (_, guide) in cluster.targets() {
                let canonical: &DropTargetRecord = plan.drop_target(guide.id()).unwrap();
                assert!(std::ptr::eq(guide.target(), canonical));
            }
        }
    }
    assert!(std::ptr::eq(
        original.surface_background().unwrap(),
        current.surface_background().unwrap(),
    ));
    assert_eq!(
        original, frozen,
        "rebinding must leave the previous output unchanged"
    );
}
