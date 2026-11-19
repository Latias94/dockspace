use super::*;
use crate::geometry::{Constraints, LogicalSize};
use crate::graph::{Axis, Node, RootRecord, SurfacePresentation, Workspace};
use crate::ids::{EngineAuthorityDomainId, ItemId, WorkspaceEpoch};
use crate::interaction::targeting::targets::DropOcclusionRecord;
use crate::policy::{PolicyRevision, TabBarInteraction};
use crate::presentation::config::{DockPresentationConfig, PresentationConfigRevision};
use crate::presentation::layout::{LayoutMetrics, project_root};
use crate::presentation::manifest::{
    Measurement, SurfaceMeasurementTicket, SurfaceMeasurements, SurfaceRequirementRevision,
    SurfaceRequirements, SurfaceSceneRevision,
};
use crate::presentation::observation::PresentationOutputSerial;
use crate::presentation::scene::{
    PaneRecord, RootLayoutFacts, SplitterRecord, SurfaceProgramRootOperation,
    SurfaceProgramRootPresentation, SurfaceSceneStamp, TabBarRecord,
    TabListMenuGeometryAvailability, TabsRecord,
};

fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
    LogicalRect::new(x, y, width, height).unwrap()
}

fn ticket() -> SurfaceMeasurementTicket {
    SurfaceMeasurementTicket::new(
        EngineAuthorityDomainId::new_for_test(1),
        WorkspaceEpoch::new(1),
        PresentationConfigRevision::new(1),
        PolicyRevision::new(1),
        SurfaceRequirementRevision::new(1),
        SurfaceId::new(1),
    )
}

fn output(serial: u64) -> SurfacePresentationOutputTicket {
    SurfacePresentationOutputTicket::mint(
        ticket().authority_domain(),
        PresentationOutputSerial::new_for_test(serial),
        SurfaceSceneStamp::new(ticket(), SurfaceSceneRevision::new(serial)),
    )
}

#[derive(Clone)]
struct Fixture {
    bounds: LogicalRect,
    root_bounds: LogicalRect,
    pane_bounds: Option<LogicalRect>,
    draw_bounds: LogicalRect,
    hit_bounds: LogicalRect,
    axis: Axis,
    chrome_bounds: LogicalRect,
    selected: ItemId,
    sibling: Option<ItemId>,
    selection_context: bool,
    config: DockPresentationConfig,
    occlusions: Vec<DropOcclusionRecord>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            bounds: rect(0.0, 0.0, 600.0, 600.0),
            root_bounds: rect(40.0, 40.0, 520.0, 520.0),
            pane_bounds: Some(rect(0.0, 0.0, 600.0, 600.0)),
            draw_bounds: rect(298.0, 40.0, 4.0, 520.0),
            hit_bounds: rect(298.0, 40.0, 4.0, 520.0),
            axis: Axis::Horizontal,
            chrome_bounds: rect(280.0, 120.0, 40.0, 20.0),
            selected: ItemId::new(1),
            sibling: None,
            selection_context: true,
            config: DockPresentationConfig::default(),
            occlusions: Vec::new(),
        }
    }
}

impl Fixture {
    fn plan(&self) -> PresentationPlan {
        let requirements = SurfaceRequirements::new(
            ticket(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let mut measurements = SurfaceMeasurements::new(ticket());
        measurements
            .set_bounds(requirements.bounds(), Measurement::Measured(self.bounds))
            .unwrap();
        let measured = requirements
            .validate(&measurements)
            .unwrap()
            .require_authoritative()
            .unwrap();
        let root = RootId::new(1);
        let mut builder = Workspace::builder();
        let tabs = builder.insert_node(Node::tabs_with_selection(
            [ItemId::new(1), ItemId::new(2), ItemId::new(9)],
            Some(self.selected),
        ));
        let sibling = self
            .sibling
            .map(|item| builder.insert_node(Node::tabs([item])));
        let root_node = sibling.map_or(tabs, |sibling| {
            builder.insert_node(Node::equal_split(Axis::Horizontal, [tabs, sibling]).unwrap())
        });
        builder.set_root(root, RootRecord::new(root_node));
        builder.set_surface(ticket().surface(), SurfacePresentation::with_main(root));
        let workspace = builder.build().unwrap();
        let constraints =
            Constraints::new(LogicalSize::new(0.0, 0.0).unwrap(), self.root_bounds.size()).unwrap();
        let mut leaf_constraints = std::collections::BTreeMap::from([(tabs, constraints)]);
        if let Some(sibling) = sibling {
            leaf_constraints.insert(sibling, constraints);
        }
        let projection = project_root(
            &workspace,
            root,
            self.root_bounds,
            |node| leaf_constraints.get(&node).copied(),
            LayoutMetrics::new(self.config.splitter_thickness()).unwrap(),
        )
        .unwrap();
        let mut plan = PresentationPlan::new(ticket().surface(), self.bounds);
        plan.install_layout_facts(self.config.clone(), measured);
        plan.push_root_layout_facts(RootLayoutFacts::new(
            root,
            self.root_bounds,
            Default::default(),
            Arc::new(projection),
        ));
        plan.begin_surface_program_root(root, SurfaceProgramRootPresentation::Main)
            .unwrap();
        let bar = TabBarSceneId { root, tabs };
        let ancestry = self.selection_context.then(|| {
            plan.push_tabs_record(
                TabsRecord::new(
                    bar,
                    self.root_bounds,
                    self.root_bounds,
                    SceneLayerKey::surface_base(),
                    None,
                ),
                Some(self.selected.into()),
            );
            plan.push_active_tab_choice(None, tabs, self.selected.into())
        });
        let mut panes = Vec::new();
        if let Some(bounds) = self.pane_bounds {
            let pane = PaneSceneId {
                root,
                item: self.selected,
            };
            panes.push(
                PaneRecord::new(pane, tabs, bounds, bounds, SceneLayerKey::surface_base())
                    .with_context(bounds, self.selection_context.then_some(bar), ancestry),
            );
            plan.push_surface_program_pane_operation(pane).unwrap();
        }
        if let Some((tabs, item)) = sibling.zip(self.sibling) {
            let bar = TabBarSceneId { root, tabs };
            let bounds = rect(400.0, 400.0, 120.0, 120.0);
            plan.push_tabs_record(
                TabsRecord::new(bar, bounds, bounds, SceneLayerKey::surface_base(), None),
                Some(item.into()),
            );
            let ancestry = Some(plan.push_active_tab_choice(None, tabs, item.into()));
            let pane = PaneSceneId { root, item };
            panes.push(
                PaneRecord::new(pane, tabs, bounds, bounds, SceneLayerKey::surface_base())
                    .with_context(bounds, Some(bar), ancestry),
            );
            plan.push_surface_program_pane_operation(pane).unwrap();
        }
        panes.sort_unstable_by_key(PaneRecord::id);
        for pane in panes {
            plan.push_pane_record(pane);
        }
        plan.push_tab_bar_record(TabBarRecord::new(
            bar,
            self.chrome_bounds,
            self.chrome_bounds,
            self.chrome_bounds,
            0.0,
            0.0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            TabBarInteraction::Disabled,
            TabListMenuGeometryAvailability::Unavailable,
            SceneLayerKey::surface_base(),
        ));
        plan.push_surface_program_root_operation(SurfaceProgramRootOperation::TabBar(bar))
            .unwrap();
        let splitter = SplitterSceneId {
            root,
            owner: crate::presentation::scene::SplitterOwner::Split(tabs),
            index: 0,
        };
        plan.push_splitter_record(SplitterRecord::new(
            splitter,
            self.draw_bounds,
            HitRegion::new(self.hit_bounds),
            self.axis,
            self.root_bounds,
            self.root_bounds,
            0.0,
            0.0,
            520.0,
            520.0,
            vec![200.0, 200.0],
            None,
            Vec::new(),
            SceneLayerKey::surface_base(),
            true,
        ));
        plan.push_surface_program_root_operation(SurfaceProgramRootOperation::Splitter(splitter))
            .unwrap();
        for &occlusion in &self.occlusions {
            plan.push_drop_occlusion(occlusion);
        }
        plan.end_surface_program_root(root).unwrap();
        plan.finish_surface_program().unwrap();
        plan.link_surface_program().unwrap();
        plan
    }
}

fn assert_full_compile_matches(
    reused: &PresentationHitManifest,
    plan: &PresentationPlan,
) -> PresentationHitManifest {
    let full = PresentationHitManifest::compile(reused.output(), plan).unwrap();
    assert_eq!(*reused, full);
    for (left, right) in reused.regions().iter().zip(full.regions()) {
        assert!(same_region(left, right));
    }
    assert_eq!(
        reused
            .geometry
            .touch_regions
            .iter()
            .map(|(row, rect)| (*row, rect_bits(*rect)))
            .collect::<Vec<_>>(),
        full.geometry
            .touch_regions
            .iter()
            .map(|(row, rect)| (*row, rect_bits(*rect)))
            .collect::<Vec<_>>()
    );
    assert_eq!(reused.geometry.emission_rows, full.geometry.emission_rows);
    full
}

#[test]
fn content_identity_change_rebuilds_exact_receivers_at_the_same_geometry() {
    let fixture = Fixture::default();
    let previous_plan = fixture.plan();
    let previous = PresentationHitManifest::compile(output(1), &previous_plan).unwrap();
    assert!(!previous.geometry.touch_regions.is_empty());
    let unchanged_plan = fixture.plan();
    let compilations = touch::compilation_count();
    let unchanged = PresentationHitManifest::compile_reusing(
        output(2),
        &unchanged_plan,
        Some((&previous_plan, &previous)),
    )
    .unwrap();
    assert_eq!(touch::compilation_count(), compilations);
    assert!(Arc::ptr_eq(&previous.geometry, &unchanged.geometry));
    assert_eq!(previous.regions(), unchanged.regions());
    assert_eq!(unchanged.output(), output(2));
    assert_ne!(previous.output(), unchanged.output());
    assert_full_compile_matches(&unchanged, &unchanged_plan);
    let changed_plan = Fixture {
        selected: ItemId::new(2),
        ..fixture
    }
    .plan();
    assert_ne!(previous_plan, changed_plan);
    let compilations = touch::compilation_count();
    let changed = PresentationHitManifest::compile_reusing(
        output(3),
        &changed_plan,
        Some((&previous_plan, &previous)),
    )
    .unwrap();
    assert_eq!(touch::compilation_count(), compilations);
    assert!(!Arc::ptr_eq(&previous.geometry, &changed.geometry));
    let pane_region = |manifest: &PresentationHitManifest| {
        manifest
            .regions()
            .iter()
            .copied()
            .find(|region| matches!(region.id().kind(), PresentationHitRegionKind::PaneBody(_)))
            .unwrap()
    };
    let old_pane = pane_region(&previous);
    let new_pane = pane_region(&changed);
    assert_eq!(old_pane.hit(), new_pane.hit());
    assert_ne!(old_pane.id(), new_pane.id());
    assert_eq!(
        new_pane.id().kind(),
        PresentationHitRegionKind::PaneBody(PaneSceneId {
            root: RootId::new(1),
            item: ItemId::new(2),
        })
    );
    assert!(
        changed
            .regions()
            .iter()
            .all(|region| region.id() != old_pane.id())
    );
    assert!(changed.region(old_pane.id()).is_none());
    assert_eq!(changed.region(new_pane.id()), Some(&new_pane));
    assert_ne!(previous.output(), changed.output());
    assert_eq!(changed.output(), output(3));
    let full = assert_full_compile_matches(&changed, &changed_plan);
    assert_eq!(changed.geometry.touch_order, full.geometry.touch_order);
}

#[test]
fn selection_rebinding_refreshes_canonical_rows_in_both_directions_without_touch_compilation() {
    let fixture = Fixture {
        sibling: Some(ItemId::new(5)),
        ..Fixture::default()
    };
    let old_plan = fixture.plan();
    let old = PresentationHitManifest::compile(output(1), &old_plan).unwrap();
    assert!(!old.geometry.touch_regions.is_empty());
    let new_plan = Fixture {
        selected: ItemId::new(9),
        ..fixture
    }
    .plan();
    let compilations = touch::compilation_count();
    let new =
        PresentationHitManifest::compile_reusing(output(2), &new_plan, Some((&old_plan, &old)))
            .unwrap();
    assert_eq!(touch::compilation_count(), compilations);
    assert_ne!(new.geometry.emission_rows, old.geometry.emission_rows);
    assert_eq!(new.geometry.touch_regions, old.geometry.touch_regions);
    assert_eq!(new.geometry.touch_order, old.geometry.touch_order);
    let pane_id = |item| {
        PresentationHitRegionId::new(
            new_plan.surface(),
            PresentationHitRegionKind::PaneBody(PaneSceneId {
                root: RootId::new(1),
                item: ItemId::new(item),
            }),
        )
    };
    assert!(new.region(pane_id(1)).is_none());
    assert!(new.region(pane_id(9)).is_some());
    let full = assert_full_compile_matches(&new, &new_plan);
    assert_eq!(new.geometry.touch_order, full.geometry.touch_order);

    let compilations = touch::compilation_count();
    let unchanged =
        PresentationHitManifest::compile_reusing(output(3), &new_plan, Some((&new_plan, &new)))
            .unwrap();
    assert_eq!(touch::compilation_count(), compilations);
    assert!(Arc::ptr_eq(&unchanged.geometry, &new.geometry));
    let full = assert_full_compile_matches(&unchanged, &new_plan);
    assert_eq!(unchanged.geometry.touch_order, full.geometry.touch_order);

    let compilations = touch::compilation_count();
    let restored =
        PresentationHitManifest::compile_reusing(output(4), &old_plan, Some((&new_plan, &new)))
            .unwrap();
    assert_eq!(touch::compilation_count(), compilations);
    assert!(restored.region(pane_id(9)).is_none());
    assert!(restored.region(pane_id(1)).is_some());
    assert_eq!(*restored.geometry, *old.geometry);
    assert_eq!(restored.geometry.emission_rows, old.geometry.emission_rows);
    let full = assert_full_compile_matches(&restored, &old_plan);
    assert_eq!(restored.geometry.touch_order, full.geometry.touch_order);
}

#[test]
fn content_identity_without_a_direct_selection_proof_recompiles_touch_geometry() {
    let fixture = Fixture {
        selection_context: false,
        ..Fixture::default()
    };
    let old_plan = fixture.plan();
    let old = PresentationHitManifest::compile(output(1), &old_plan).unwrap();
    let new_plan = Fixture {
        selected: ItemId::new(2),
        ..fixture
    }
    .plan();
    assert!(touch::same_context(&old_plan, &new_plan));
    let compilations = touch::compilation_count();
    let new =
        PresentationHitManifest::compile_reusing(output(2), &new_plan, Some((&old_plan, &old)))
            .unwrap();
    assert_eq!(touch::compilation_count(), compilations + 1);
    assert_full_compile_matches(&new, &new_plan);
}

#[test]
fn touch_context_changes_reject_reuse_even_when_receiver_regions_are_unchanged() {
    let fixture = Fixture::default();
    let previous_plan = fixture.plan();
    let previous = PresentationHitManifest::compile(output(1), &previous_plan).unwrap();
    let mut cases = Vec::new();
    let mut changed = fixture.clone();
    changed.chrome_bounds = rect(280.0, 220.0, 40.0, 20.0);
    cases.push(changed);
    let mut changed = fixture.clone();
    changed.root_bounds = rect(60.0, 60.0, 480.0, 480.0);
    cases.push(changed);
    for config in [
        DockPresentationConfig::builder()
            .touch_resize_hit_extent(60.0)
            .build()
            .unwrap(),
        DockPresentationConfig::builder()
            .splitter_hit_extent(24.0)
            .build()
            .unwrap(),
        DockPresentationConfig::builder()
            .floating_resize_extent(10.0)
            .build()
            .unwrap(),
    ] {
        cases.push(Fixture {
            config,
            ..fixture.clone()
        });
    }
    let mut changed = fixture.clone();
    changed.occlusions.push(DropOcclusionRecord::new(
        FloatingPresentationId::new(7),
        HitRegion::new(rect(280.0, 320.0, 40.0, 30.0)),
        SceneLayerKey::new(1),
    ));
    cases.push(changed);
    for changed in cases {
        let plan = changed.plan();
        let mut emitted = Vec::new();
        emit_regions(&plan, &mut |region| emitted.push(region)).unwrap();
        let mut old_emitted = Vec::new();
        emit_regions(&previous_plan, &mut |region| old_emitted.push(region)).unwrap();
        assert_eq!(emitted.len(), old_emitted.len());
        assert!(
            emitted
                .iter()
                .zip(&old_emitted)
                .all(|(a, b)| same_region(a, b))
        );
        assert!(!touch::same_context(&plan, &previous_plan));
        let compilations = touch::compilation_count();
        let actual = PresentationHitManifest::compile_reusing(
            output(2),
            &plan,
            Some((&previous_plan, &previous)),
        )
        .unwrap();
        assert_eq!(touch::compilation_count(), compilations + 1);
        assert!(!Arc::ptr_eq(&actual.geometry, &previous.geometry));
        assert_full_compile_matches(&actual, &plan);
    }
}

#[test]
fn draw_axis_surface_bounds_and_emission_count_changes_use_full_compilation() {
    let fixture = Fixture::default();
    let previous_plan = fixture.plan();
    let previous = PresentationHitManifest::compile(output(1), &previous_plan).unwrap();
    let cases = [
        Fixture {
            draw_bounds: rect(300.0, 40.0, 4.0, 520.0),
            ..fixture.clone()
        },
        Fixture {
            axis: Axis::Vertical,
            ..fixture.clone()
        },
        Fixture {
            bounds: rect(0.0, 0.0, 620.0, 600.0),
            ..fixture.clone()
        },
        Fixture {
            pane_bounds: None,
            ..fixture
        },
    ];
    for changed in cases {
        let plan = changed.plan();
        let compilations = touch::compilation_count();
        let actual = PresentationHitManifest::compile_reusing(
            output(2),
            &plan,
            Some((&previous_plan, &previous)),
        )
        .unwrap();
        assert_eq!(touch::compilation_count(), compilations + 1);
        assert!(!Arc::ptr_eq(&actual.geometry, &previous.geometry));
        assert_full_compile_matches(&actual, &plan);
    }
}

#[test]
fn raw_signed_zero_changes_never_reuse_numerically_equal_receiver_geometry() {
    let previous_plan = Fixture::default().plan();
    let previous = PresentationHitManifest::compile(output(1), &previous_plan).unwrap();
    let plan = Fixture {
        pane_bounds: Some(rect(-0.0, -0.0, 600.0, 600.0)),
        ..Fixture::default()
    }
    .plan();
    assert_eq!(previous_plan.pane_records(), plan.pane_records());
    let compilations = touch::compilation_count();
    let actual = PresentationHitManifest::compile_reusing(
        output(2),
        &plan,
        Some((&previous_plan, &previous)),
    )
    .unwrap();
    assert_eq!(touch::compilation_count(), compilations + 1);
    assert!(!Arc::ptr_eq(&actual.geometry, &previous.geometry));
    assert_eq!(
        rect_bits(actual.regions()[0].hit.rect())[0],
        (-0.0_f64).to_bits()
    );
    assert_full_compile_matches(&actual, &plan);
}

#[test]
fn any_compacted_row_disables_reuse_and_cache_hints_do_not_change_equality() {
    let plan = Fixture {
        draw_bounds: rect(1_000.0, 1_000.0, 0.0, 0.0),
        hit_bounds: rect(1_000.0, 1_000.0, 0.0, 0.0),
        ..Fixture::default()
    }
    .plan();
    let previous = PresentationHitManifest::compile(output(1), &plan).unwrap();
    assert_eq!(previous.regions().len(), 1);
    assert!(previous.geometry.emission_rows.is_none());
    let actual =
        PresentationHitManifest::compile_reusing(output(2), &plan, Some((&plan, &previous)))
            .unwrap();
    assert!(!Arc::ptr_eq(&actual.geometry, &previous.geometry));
    assert_full_compile_matches(&actual, &plan);

    let complete = PresentationHitManifest::compile(output(1), &Fixture::default().plan()).unwrap();
    assert!(complete.geometry.emission_rows.is_some());
    let without_hint = PresentationHitManifest {
        output: complete.output,
        geometry: Arc::new(HitGeometry {
            touch_order: complete.geometry.touch_order.clone(),
            regions: complete.geometry.regions.clone(),
            touch_regions: complete.geometry.touch_regions.clone(),
            emission_rows: None,
        }),
    };
    assert_eq!(complete, without_hint);
}
