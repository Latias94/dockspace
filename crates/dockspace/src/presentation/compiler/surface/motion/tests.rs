use super::*;
use crate::geometry::{LogicalPoint, LogicalSize};
use crate::graph::{Axis, ContainedFloating, RootRecord, SurfacePresentation};
use crate::ids::ItemId;
use crate::policy::DockPolicy;
use crate::presentation::manifest::{Measurement, PolicyRevision, TabIntrinsic, TabStripMetrics};
use crate::presentation::scene::SurfaceProgramBlock;
use proptest::prelude::*;

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const FLOATING_ROOT: RootId = RootId::new(2);

#[test]
fn animating_a_split_keeps_a_clipped_floating_panes_layout_bounds() {
    let outer = LogicalRect::new(-80.0, -43.0, 280.0, 240.0).unwrap();
    let (workspace, policy, manifest, target) = fixture_with_contained_bounds(outer);
    let pane = moving_pane(&target);
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(
            pane.id(),
            LogicalRect::new(80.0, 40.0, 300.0, 240.0).unwrap(),
        )]),
        &BTreeMap::new(),
    )
    .unwrap();
    let floating_pane = |plan: &PresentationPlan| {
        plan.pane_records()
            .iter()
            .find(|pane| pane.id().root == FLOATING_ROOT)
            .map(|pane| (pane.layout_bounds(), pane.clip()))
            .unwrap()
    };
    let (layout, clip) = floating_pane(&target);
    assert!(layout.x() < clip.x());
    assert_eq!(floating_pane(&sampled), (layout, clip));
}

fn fixture() -> (
    Workspace,
    DockPolicySnapshot,
    SceneRequirementManifest,
    PresentationPlan,
) {
    fixture_with_contained_bounds(LogicalRect::new(500.0, 320.0, 280.0, 240.0).unwrap())
}

fn fixture_with_contained_bounds(
    outer: LogicalRect,
) -> (
    Workspace,
    DockPolicySnapshot,
    SceneRequirementManifest,
    PresentationPlan,
) {
    let mut builder = Workspace::builder();
    let left = builder.insert_node(Node::tabs([ItemId::new(3), ItemId::new(1)]));
    let right = builder.insert_node(Node::tabs([ItemId::new(4), ItemId::new(2)]));
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, [left, right], [1.0, 1.0]).unwrap());
    builder.set_root(ROOT, RootRecord::new(split).with_central(left));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    let floating_tabs = builder.insert_node(Node::tabs([ItemId::new(5), ItemId::new(6)]));
    builder.set_root(FLOATING_ROOT, RootRecord::new(floating_tabs));
    let floating = FloatingPresentationId::new(1);
    builder.set_contained_floating(floating, ContainedFloating::new(FLOATING_ROOT, outer));
    builder.attach_contained(SURFACE, floating).unwrap();
    let workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::default();
    let draft = derive_scene_requirement_draft(
        EngineAuthorityDomainId::new_for_test(1),
        &workspace,
        WorkspaceVersion::initial(),
        PresentationConfigRevision::new(1),
        &policy,
        RequirementRevision::new(1),
        &BTreeMap::from([(SURFACE, SurfaceRequirementRevision::new(1))]),
    )
    .unwrap();
    let requirements = draft.surface(SURFACE).unwrap();
    let mut measurements = SurfaceMeasurements::new(requirements.ticket());
    measurements
        .set_bounds(
            requirements.bounds(),
            Measurement::Measured(LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap()),
        )
        .unwrap();
    for key in requirements.pane_sizings() {
        measurements
            .insert_pane_sizing(
                key,
                Measurement::Measured((LogicalSize::new(0.0, 0.0).unwrap()).into()),
            )
            .unwrap();
    }
    for key in requirements.tab_intrinsics() {
        measurements
            .insert_tab_intrinsic(key, Measurement::Measured(TabIntrinsic::new(32.0).unwrap()))
            .unwrap();
    }
    for key in requirements.tab_strips() {
        measurements
            .insert_tab_strip(
                key,
                Measurement::Measured(TabStripMetrics::new(0.0, 0.0).unwrap()),
            )
            .unwrap();
    }
    let manifest = draft.finalize(PopupPlaneRequirement::default()).unwrap();
    let target = compile_surface_plan(
        &workspace,
        WorkspaceVersion::initial(),
        &policy,
        &config,
        &manifest,
        &measurements,
        &TabStripStateStore::default(),
        &[],
        None,
    )
    .unwrap()
    .finalize()
    .unwrap();
    (workspace, policy, manifest, target)
}

fn moving_pane(target: &PresentationPlan) -> &crate::presentation::scene::PaneRecord {
    target
        .pane_records()
        .iter()
        .find(|pane| pane.id().root == ROOT)
        .unwrap()
}

#[test]
fn sampled_leaf_reflows_without_scaling_text_and_keeps_target_layout_shared() {
    let (workspace, policy, manifest, target) = fixture();
    reset_surface_compilation_count();
    let original = target.clone();
    let pane = moving_pane(&target);
    let rect = LogicalRect::new(80.0, 40.0, 300.0, 240.0).unwrap();
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(pane.id(), rect)]),
        &BTreeMap::from([(pane.id(), 1)]),
    )
    .unwrap();
    assert_eq!(surface_compilation_count(), 0);
    let sampled_pane = sampled
        .pane_records()
        .iter()
        .find(|record| record.id() == pane.id())
        .unwrap();
    assert_eq!(sampled_pane.bounds(), rect);
    assert_eq!(sampled_pane.item(), pane.item());
    let bar = sampled
        .tab_bar_records()
        .iter()
        .find(|bar| {
            bar.id().tabs
                == pane
                    .containing_tab_bar()
                    .expect("flat fixture has a containing bar")
                    .tabs
        })
        .unwrap();
    assert_eq!(
        bar.bounds().height(),
        target.layout_facts().unwrap().config().tab_bar_height()
    );
    let old_tab = target
        .tab_records()
        .iter()
        .find(|tab| {
            tab.id().tabs
                == pane
                    .containing_tab_bar()
                    .expect("flat fixture has a containing bar")
                    .tabs
        })
        .unwrap();
    let new_tab = sampled
        .tab_records()
        .iter()
        .find(|tab| tab.id() == old_tab.id())
        .unwrap();
    assert_eq!(
        old_tab.text_layout_bounds().width(),
        new_tab.text_layout_bounds().width()
    );
    assert_eq!(
        old_tab.text_layout_bounds().height(),
        new_tab.text_layout_bounds().height()
    );
    let old_layout = target.layout_facts().unwrap();
    let new_layout = sampled.layout_facts().unwrap();
    assert!(Arc::ptr_eq(
        old_layout.measurements(),
        new_layout.measurements()
    ));
    assert!(std::ptr::eq(
        old_layout.root(ROOT).unwrap().projection(),
        new_layout.root(ROOT).unwrap().projection(),
    ));
    assert_eq!(sampled.splitter_records(), target.splitter_records());
    assert_eq!(target, original);
}

#[test]
fn fractional_sample_keeps_content_inside_the_exact_pane_edges() {
    let (workspace, policy, manifest, target) = fixture();
    let pane = moving_pane(&target);
    let bounds = LogicalRect::from_min_max(
        LogicalPoint::new(80.0, 242.858_136_646_816_4).unwrap(),
        LogicalPoint::new(380.0, 433.674_076_586_488_87).unwrap(),
    )
    .unwrap();
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(pane.id(), bounds)]),
        &BTreeMap::from([(pane.id(), 1)]),
    )
    .unwrap();
    let pane = sampled
        .pane_records()
        .iter()
        .find(|record| record.id() == pane.id())
        .unwrap();
    let bar = sampled
        .tab_bar_records()
        .iter()
        .find(|bar| {
            bar.id().tabs
                == pane
                    .containing_tab_bar()
                    .expect("flat fixture has a containing bar")
                    .tabs
        })
        .unwrap();
    assert_eq!(pane.bounds(), bounds);
    assert_eq!(pane.content_bounds().max(), bounds.max());
    assert_eq!(bar.bounds().x(), bounds.x());
    assert_eq!(
        bar.bounds().y(),
        bounds.y() - target.layout_facts().unwrap().config().tab_bar_height()
    );
    assert_eq!(bar.bounds().max().y(), pane.content_bounds().y());
    assert!(pane.content_bounds().y() <= bounds.max().y());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn sampled_fractional_and_thin_panes_preserve_shared_edges(
        x in 0.0_f64..100.0,
        y in 0.0_f64..100.0,
        width in prop_oneof![Just(0.0), 0.0_f64..600.0],
        height in prop_oneof![Just(0.0), Just(28.0), 0.0_f64..28.0, 28.0_f64..480.0],
    ) {
        let (workspace, policy, manifest, target) = fixture();
        let pane = moving_pane(&target);
        let bounds = LogicalRect::new(x, y, width, height).unwrap();
        let sampled = materialize_presentation_sample(
            &workspace,
            &policy,
            &manifest,
            &TabStripStateStore::default(),
            &target,
            &BTreeMap::from([(pane.id(), bounds)]),
            &BTreeMap::from([(pane.id(), 1)]),
        ).unwrap();
        let pane = sampled.pane_records().iter()
            .find(|record| record.id() == pane.id()).unwrap();
        prop_assert_eq!(pane.bounds(), bounds);
        prop_assert_eq!(pane.content_bounds().x(), bounds.x());
        prop_assert_eq!(pane.content_bounds().max(), bounds.max());
        prop_assert!(pane.content_bounds().y() >= bounds.y());
        prop_assert!(pane.content_bounds().y() <= bounds.max().y());
        if let Some(bar) = sampled.tab_bar_records().iter()
            .find(|bar| bar.id().tabs == pane.containing_tab_bar().expect("flat fixture has a containing bar").tabs)
        {
            prop_assert_eq!(bar.bounds().x(), bounds.x());
            prop_assert_eq!(
                bar.bounds().y(),
                (bounds.y() - target.layout_facts().unwrap().config().tab_bar_height()).max(0.0),
            );
            prop_assert_eq!(bar.bounds().max().x(), bounds.max().x());
            prop_assert_eq!(bar.bounds().max().y(), pane.content_bounds().y());
        }
    }
}

#[test]
fn moving_root_has_no_structural_drop_authority_and_other_roots_keep_theirs() {
    let (workspace, policy, manifest, target) = fixture();
    let pane = moving_pane(&target);
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(
            pane.id(),
            LogicalRect::new(40.0, 0.0, 400.0, 600.0).unwrap(),
        )]),
        &BTreeMap::from([(pane.id(), 1)]),
    )
    .unwrap();
    assert!(!sampled.partition_available(ROOT));
    assert!(sampled.partition_available(FLOATING_ROOT));
    assert!(
        sampled
            .drop_targets()
            .iter()
            .all(|target| target.id().root() != Some(ROOT))
    );
    assert!(
        sampled
            .drop_guide_clusters()
            .iter()
            .all(|cluster| cluster.id().root() != Some(ROOT))
    );
    let retained = |plan: &PresentationPlan| {
        plan.drop_targets()
            .iter()
            .filter(|target| target.id().root() == Some(FLOATING_ROOT))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(!retained(&target).is_empty());
    assert_eq!(retained(&sampled), retained(&target));
    assert!(
        sampled
            .magnetic_target_at(
                LogicalPoint::new(100.0, 300.0).unwrap(),
                SceneLayerKey::surface_base(),
                |_| true,
            )
            .is_none()
    );
    assert!(
        sampled
            .surface_program()
            .blocks()
            .iter()
            .all(|block| match block {
                SurfaceProgramBlock::DropTarget(id) => id.root() != Some(ROOT),
                SurfaceProgramBlock::DropGuide(id) => id.root() != Some(ROOT),
                _ => true,
            })
    );
}

#[test]
fn pane_rank_groups_front_content_after_all_rear_pane_chrome() {
    let (workspace, policy, manifest, target) = fixture();
    let front = moving_pane(&target);
    let rear = target
        .pane_records()
        .iter()
        .find(|pane| pane.id().root == ROOT && pane.id() != front.id())
        .unwrap();
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(front.id(), rear.bounds())]),
        &BTreeMap::from([(front.id(), usize::MAX)]),
    )
    .unwrap();
    assert_eq!(
        sampled.pane_rank(
            front.id().root,
            front
                .containing_tab_bar()
                .expect("flat fixture has a containing bar")
                .tabs
        ),
        usize::MAX
    );
    assert_eq!(
        sampled.pane_rank(
            rear.id().root,
            rear.containing_tab_bar()
                .expect("flat fixture has a containing bar")
                .tabs
        ),
        0
    );
    let blocks = sampled.surface_program().blocks();
    let front_start = blocks.iter().position(|block| matches!(
        block,
        SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::Pane(id)) if *id == front.id()
    )).unwrap();
    let rear_end = blocks
        .iter()
        .rposition(|block| match block {
            SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::Pane(id)) => {
                *id == rear.id()
            }
            SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::TabBar(id)) => {
                id.tabs
                    == rear
                        .containing_tab_bar()
                        .expect("flat fixture has a containing bar")
                        .tabs
            }
            SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::Tab(id)) => {
                id.tabs
                    == rear
                        .containing_tab_bar()
                        .expect("flat fixture has a containing bar")
                        .tabs
            }
            SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::TabStripControl(
                id,
            )) => {
                id.bar().tabs
                    == rear
                        .containing_tab_bar()
                        .expect("flat fixture has a containing bar")
                        .tabs
            }
            _ => false,
        })
        .unwrap();
    assert!(rear_end < front_start);
    assert!(!blocks.iter().any(|block| matches!(
        block,
        SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::Splitter(id)) if id.root == ROOT
    )), "an unavailable partition must not ask any renderer to paint its handles");
}

#[test]
fn settled_sample_reuses_target_facts_and_restores_partition_and_targets() {
    let (workspace, policy, manifest, target) = fixture();
    let rects = target
        .pane_records()
        .iter()
        .map(|pane| (pane.id(), pane.bounds()))
        .collect();
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &rects,
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(sampled.shares_facts(&target));
    assert!(sampled.partition_available(ROOT));
    assert_eq!(sampled.drop_targets(), target.drop_targets());
    assert_eq!(sampled, target);
}

#[test]
fn sampled_geometry_stays_inside_the_target_root_clip() {
    let (workspace, policy, manifest, target) = fixture();
    let pane = moving_pane(&target);
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(
            pane.id(),
            LogicalRect::new(-20.0, -40.0, 900.0, 700.0).unwrap(),
        )]),
        &BTreeMap::from([(pane.id(), 1)]),
    )
    .unwrap();
    let projected = sampled
        .pane_records()
        .iter()
        .find(|record| record.id() == pane.id())
        .unwrap();
    assert_eq!(
        projected.bounds(),
        target.layout_facts().unwrap().root(ROOT).unwrap().bounds()
    );
    assert!(rect_contains(
        projected.bounds(),
        projected.content_bounds()
    ));
    let unaffected = |plan: &PresentationPlan| {
        plan.pane_records()
            .iter()
            .filter(|record| record.id().root == FLOATING_ROOT)
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(unaffected(&sampled), unaffected(&target));
}

#[test]
fn front_pane_body_blocks_rear_tabs_and_moving_partition_has_no_resize_receivers() {
    use crate::presentation::hit::{
        PresentationHitManifest, PresentationHitRegionKind as Kind, PresentationPointerLane,
    };
    use crate::presentation::manifest::SurfaceSceneRevision;
    use crate::presentation::observation::PresentationOutputSerial;
    use crate::presentation::observation::SurfacePresentationOutputTicket;
    use crate::presentation::scene::SurfaceSceneStamp;
    let (workspace, policy, manifest, target) = fixture();
    let front = moving_pane(&target);
    let rear = target
        .pane_records()
        .iter()
        .find(|pane| pane.id().root == ROOT && pane.id() != front.id())
        .unwrap();
    let rear_bounds =
        LogicalRect::new(rear.bounds().x(), 100.0, rear.bounds().width(), 500.0).unwrap();
    let sampled = materialize_presentation_sample(
        &workspace,
        &policy,
        &manifest,
        &TabStripStateStore::default(),
        &target,
        &BTreeMap::from([(front.id(), rear.bounds()), (rear.id(), rear_bounds)]),
        &BTreeMap::from([(front.id(), 10)]),
    )
    .unwrap();
    let ticket = target.measurement_ticket().unwrap();
    let output = SurfacePresentationOutputTicket::mint(
        ticket.authority_domain(),
        PresentationOutputSerial::new_for_test(1),
        SurfaceSceneStamp::new(ticket, SurfaceSceneRevision::new(1)),
    );
    let hits = PresentationHitManifest::compile(output, &sampled).unwrap();
    assert!(
        !hits.regions().iter().any(
            |region| matches!(region.id().kind(), Kind::SplitterHandle(id) if id.root == ROOT)
        )
    );
    let rear_tab = sampled
        .tab_records()
        .iter()
        .find(|tab| {
            tab.id().tabs
                == rear
                    .containing_tab_bar()
                    .expect("flat fixture has a containing bar")
                    .tabs
        })
        .unwrap();
    let point = LogicalPoint::new(
        rear_tab.drag_hit().rect().x() + 2.0,
        rear_tab.drag_hit().rect().y() + 2.0,
    )
    .unwrap();
    assert_eq!(
        hits.resolve_exclusive(PresentationPointerLane::Click, point)
            .unwrap()
            .unwrap()
            .id()
            .kind(),
        Kind::PaneBody(front.id())
    );
}

#[test]
fn contained_background_stays_behind_its_tabs_and_content_during_any_root_motion() {
    use crate::presentation::hit::{
        PresentationHitManifest, PresentationHitRegionKind as Kind, PresentationPointerLane,
    };
    use crate::presentation::manifest::SurfaceSceneRevision;
    use crate::presentation::observation::{
        PresentationOutputSerial, SurfacePresentationOutputTicket,
    };
    use crate::presentation::scene::SurfaceSceneStamp;

    for moving_root in [ROOT, FLOATING_ROOT] {
        let (workspace, policy, manifest, target) = fixture();
        let moving = target
            .pane_records()
            .iter()
            .find(|pane| pane.id().root == moving_root)
            .unwrap();
        let ranks = target
            .pane_records()
            .iter()
            .enumerate()
            .map(|(rank, pane)| (pane.id(), rank + 1))
            .collect();
        let bounds = moving.bounds();
        let rect = LogicalRect::new(
            bounds.x() + 5.0,
            bounds.y() + 5.0,
            bounds.width() - 10.0,
            bounds.height() - 10.0,
        )
        .unwrap();
        let sampled = materialize_presentation_sample(
            &workspace,
            &policy,
            &manifest,
            &TabStripStateStore::default(),
            &target,
            &BTreeMap::from([(moving.id(), rect)]),
            &ranks,
        )
        .unwrap();
        let ticket = target.measurement_ticket().unwrap();
        let output = SurfacePresentationOutputTicket::mint(
            ticket.authority_domain(),
            PresentationOutputSerial::new_for_test(1),
            SurfaceSceneStamp::new(ticket, SurfaceSceneRevision::new(1)),
        );
        let hits = PresentationHitManifest::compile(output, &sampled).unwrap();
        let tab = sampled
            .tab_records()
            .iter()
            .find(|tab| tab.id().root == FLOATING_ROOT)
            .unwrap();
        let tab_rect = tab.drag_hit().rect();
        let point = LogicalPoint::new(tab_rect.x() + 2.0, tab_rect.y() + 2.0).unwrap();
        assert_eq!(
            hits.resolve_exclusive(PresentationPointerLane::Click, point)
                .unwrap()
                .unwrap()
                .id()
                .kind(),
            Kind::TabBody(*tab.id())
        );
        let pane = sampled
            .pane_records()
            .iter()
            .find(|pane| pane.id().root == FLOATING_ROOT)
            .unwrap();
        let body = pane.content_bounds();
        let point = LogicalPoint::new(body.x() + 2.0, body.y() + 2.0).unwrap();
        assert_eq!(
            hits.resolve_exclusive(PresentationPointerLane::Click, point)
                .unwrap()
                .unwrap()
                .id()
                .kind(),
            Kind::PaneBody(pane.id())
        );
    }
}
