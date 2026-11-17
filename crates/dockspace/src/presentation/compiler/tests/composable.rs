//! Active composition geometry and receiver proofs share the exact graph choices.

use super::*;
use crate::graph::command::WorkspaceCommand;
use crate::graph::transaction::WorkspaceTransaction;
use crate::presentation::scene::PaneSceneId;

const A: ItemId = ItemId::new(101);
const B: ItemId = ItemId::new(102);
const C: ItemId = ItemId::new(103);
const D: ItemId = ItemId::new(104);
const SURFACE_HEIGHT: f64 = 500.0;
const TAB_BAR_HEIGHT: f64 = 20.0;
const SPLITTER_THICKNESS: f64 = 4.0;

fn fixture() -> (Workspace, SurfaceId, RootId, NodeId, NodeId, NodeId) {
    let surface = SurfaceId::new(91);
    let root = RootId::new(92);
    let mut builder = Workspace::builder();
    let inner = builder.insert_node(Node::tabs([A, B]));
    let bare = builder.insert_node(Node::pane(D));
    let mut tools = Node::equal_split(Axis::Horizontal, [inner, bare]).unwrap();
    tools.set_layout_title(Some(Arc::from("Tools")));
    let tools = builder.insert_node(tools);
    let outer = builder.insert_node(Node::tabs_from_keys(
        vec![TabKey::for_item(C), TabKey::for_layout(tools)],
        Some(TabKey::for_item(C)),
    ));
    builder.set_root(root, RootRecord::new(outer));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    (builder.build().unwrap(), surface, root, outer, inner, tools)
}

fn select(workspace: &mut Workspace, root: RootId, tabs: NodeId, key: TabKey) {
    let source = workspace.capture_tab_source(root, tabs, key).unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Select { source }])
        .apply(workspace, &policy_snapshot())
        .unwrap();
}

fn plan(workspace: &Workspace, surface: SurfaceId) -> PresentationPlan {
    let config = DockPresentationConfig::builder()
        .tab_bar_height(TAB_BAR_HEIGHT)
        .tab_group_grip_extent(TAB_BAR_HEIGHT)
        .minimum_pane_size(1.0, 1.0)
        .splitter_thickness(SPLITTER_THICKNESS)
        .build()
        .unwrap();
    let policy = policy_snapshot();
    let plan = compile_ready_plan_with_config(
        workspace,
        surface,
        LogicalRect::new(0.0, 0.0, 800.0, SURFACE_HEIGHT).unwrap(),
        false,
        &policy,
        &config,
    )
    .unwrap();
    PresentationPlanValidator::new(workspace, &policy)
        .unwrap()
        .validate_and_canonicalize(plan)
        .unwrap()
}

#[test]
fn layout_tab_projects_only_its_active_children_and_subtracts_each_header_once() {
    let (mut workspace, surface, root, outer, inner, tools) = fixture();
    let first = plan(&workspace, surface);
    assert_eq!(first.pane_records().len(), 1);
    assert_eq!(first.pane_records()[0].item(), C);
    assert_eq!(
        first.pane_records()[0].bounds().height(),
        SURFACE_HEIGHT - TAB_BAR_HEIGHT
    );
    assert_eq!(first.tabs_records().len(), 1);
    assert_eq!(first.tab_bar_records().len(), 1);
    assert!(first.splitter_records().is_empty());
    assert_eq!(
        first
            .layout_facts()
            .unwrap()
            .root(root)
            .unwrap()
            .pane_sizings()
            .count(),
        4
    );

    select(&mut workspace, root, outer, TabKey::for_layout(tools));
    let composed = plan(&workspace, surface);
    assert_eq!(
        composed
            .pane_records()
            .iter()
            .map(|pane| pane.item())
            .collect::<Vec<_>>(),
        [A, D]
    );
    assert_eq!(composed.tab_bar_records().len(), 2);
    let pane = |item| {
        composed
            .pane_records()
            .iter()
            .find(|pane| pane.item() == item)
            .unwrap()
    };
    assert_eq!(
        pane(A).bounds().height(),
        SURFACE_HEIGHT - 2.0 * TAB_BAR_HEIGHT
    );
    assert_eq!(pane(D).bounds().height(), SURFACE_HEIGHT - TAB_BAR_HEIGHT);
    let child_width = (800.0 - SPLITTER_THICKNESS) * 0.5;
    assert_eq!(pane(A).slot().width(), child_width);
    assert_eq!(pane(D).slot().width(), child_width);
    assert_eq!(composed.splitter_records().len(), 1);
    let divider = &composed.splitter_records()[0];
    assert_eq!(divider.id().node(), tools);
    assert_eq!(
        divider.draw_bounds(),
        LogicalRect::new(
            child_width,
            TAB_BAR_HEIGHT,
            SPLITTER_THICKNESS,
            SURFACE_HEIGHT - TAB_BAR_HEIGHT,
        )
        .unwrap()
    );
    assert_eq!(
        pane(D).containing_tab_bar(),
        Some(TabBarSceneId { root, tabs: outer })
    );
    assert_eq!(
        composed.tab_label(TabSceneId {
            root,
            tabs: outer,
            tab: TabKey::for_layout(tools)
        }),
        Some(crate::graph::TabLabel::Layout { title: "Tools" })
    );

    let old_inner_header = PresentationHitRegionKind::TabBody(TabSceneId {
        root,
        tabs: inner,
        tab: TabKey::for_item(B),
    });
    let old_inner_body = PresentationHitRegionKind::PaneBody(PaneSceneId { root, item: A });
    let old_inner_divider = PresentationHitRegionKind::SplitterHandle(*divider.id());
    assert!(composed.receiver_active_path_is_current(old_inner_header, &workspace));
    assert!(composed.receiver_active_path_is_current(old_inner_body, &workspace));
    assert!(composed.receiver_active_path_is_current(old_inner_divider, &workspace));
    select(&mut workspace, root, inner, TabKey::for_item(B));
    assert!(composed.receiver_active_path_is_current(old_inner_header, &workspace));
    assert!(!composed.receiver_active_path_is_current(old_inner_body, &workspace));
    let switched = plan(&workspace, surface);
    assert_eq!(
        switched.selected_in_tabs(outer),
        Some(TabKey::for_layout(tools))
    );
    assert_eq!(
        switched.tab_label(TabSceneId {
            root,
            tabs: outer,
            tab: TabKey::for_layout(tools)
        }),
        Some(crate::graph::TabLabel::Layout { title: "Tools" })
    );
    assert!(switched.pane_records().iter().any(|pane| pane.item() == B));
    select(&mut workspace, root, outer, TabKey::for_item(C));
    assert!(!composed.receiver_active_path_is_current(old_inner_header, &workspace));
    assert!(!composed.receiver_active_path_is_current(old_inner_body, &workspace));
    assert!(!composed.receiver_active_path_is_current(old_inner_divider, &workspace));
    let restored = plan(&workspace, surface);
    assert_eq!(restored.pane_records().len(), 1);
    assert_eq!(restored.tab_bar_records().len(), 1);
}

#[test]
fn bare_main_root_reserves_layout_actions_outside_real_content() {
    let surface = SurfaceId::new(93);
    let root = RootId::new(94);
    let mut builder = Workspace::builder();
    let owner = builder.insert_node(Node::pane(A));
    builder.set_root(root, RootRecord::new(owner));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    let workspace = builder.build().unwrap();
    let projected = plan(&workspace, surface);
    assert!(projected.tabs_records().is_empty());
    assert!(projected.tab_bar_records().is_empty());
    assert!(projected.tab_records().is_empty());
    assert_eq!(projected.pane_records().len(), 1);
    let pane = &projected.pane_records()[0];
    assert_eq!(pane.item(), A);
    assert_eq!(pane.owner_node(), owner);
    assert_eq!(pane.containing_tab_bar(), None);
    assert_eq!(
        pane.bounds(),
        LogicalRect::new(0.0, TAB_BAR_HEIGHT, 800.0, SURFACE_HEIGHT - TAB_BAR_HEIGHT).unwrap()
    );
    let anchor = projected
        .presentation_menu_anchor_records()
        .first()
        .unwrap();
    assert_eq!(anchor.host(), PresentationMenuAnchorHost::RootControls);
    let reserved = anchor.reserved_bounds().unwrap();
    assert_eq!(
        reserved,
        LogicalRect::new(0.0, 0.0, 800.0, TAB_BAR_HEIGHT).unwrap()
    );
    assert!(
        anchor
            .ready_bounds()
            .is_some_and(|menu| !positive_rect_overlap(menu, pane.bounds()))
    );
    let facts = projected.layout_facts().unwrap().root(root).unwrap();
    assert_eq!(
        facts.bounds(),
        LogicalRect::new(0.0, 0.0, 800.0, SURFACE_HEIGHT).unwrap()
    );
    assert_eq!(facts.content_bounds(), pane.slot());
    assert!(projected.surface_program().blocks().iter().any(|block| matches!(
        block,
        crate::presentation::scene::SurfaceProgramBlock::RootOperation(SurfaceProgramRootOperation::PresentationMenuAnchor(control_root)) if *control_root == root
    )));
}

#[test]
fn bare_split_root_keeps_splitters_below_independent_controls() {
    let surface = SurfaceId::new(95);
    let root = RootId::new(96);
    let mut builder = Workspace::builder();
    let left = builder.insert_node(Node::pane(A));
    let right = builder.insert_node(Node::pane(D));
    let split = builder.insert_node(Node::equal_split(Axis::Horizontal, [left, right]).unwrap());
    builder.set_root(root, RootRecord::new(split));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    let workspace = builder.build().unwrap();
    let config = DockPresentationConfig::builder()
        .tab_bar_height(TAB_BAR_HEIGHT)
        .tab_group_grip_extent(TAB_BAR_HEIGHT)
        .minimum_pane_size(1.0, 1.0)
        .splitter_thickness(SPLITTER_THICKNESS)
        .build()
        .unwrap();
    let policy = policy_snapshot();
    let projected = compile_ready_plan_with_config(
        &workspace,
        surface,
        LogicalRect::new(0.0, 0.0, 800.0, SURFACE_HEIGHT).unwrap(),
        false,
        &policy,
        &config,
    )
    .unwrap();
    let projected = PresentationPlanValidator::new(&workspace, &policy)
        .unwrap()
        .validate_and_canonicalize(projected)
        .unwrap();
    let reserved = projected.presentation_menu_anchor_records()[0]
        .reserved_bounds()
        .unwrap();
    assert_eq!(projected.splitter_records().len(), 1);
    assert!(
        projected
            .pane_records()
            .iter()
            .all(|pane| pane.containing_tab_bar().is_none()
                && !positive_rect_overlap(reserved, pane.bounds()))
    );
    assert!(
        projected
            .splitter_records()
            .iter()
            .all(
                |splitter| !positive_rect_overlap(reserved, splitter.hit().rect())
                    && !positive_rect_overlap(reserved, splitter.draw_bounds())
            )
    );
}

#[test]
fn layout_tab_containing_the_central_anchor_has_no_close_chrome() {
    let (workspace, surface, root, outer, inner, tools) = fixture();
    let mut builder = workspace.into_builder();
    builder.set_root(root, RootRecord::new(outer).with_central(inner));
    let workspace = builder.build().unwrap();
    let projected = plan(&workspace, surface);
    let tab = projected
        .tab_records()
        .iter()
        .find(|tab| tab.id().tab == TabKey::for_layout(tools))
        .unwrap();
    assert_eq!(tab.close_visual_bounds(), None);
    assert_eq!(tab.close_bounds(), None);
    assert!(
        projected
            .tab_records()
            .iter()
            .find(|tab| tab.id().tab == TabKey::for_item(C))
            .unwrap()
            .close_visual_bounds()
            .is_some()
    );
}

#[test]
fn preserve_group_previews_use_the_exact_prospective_layout_intrinsic() {
    let surface = SurfaceId::new(97);
    let root = RootId::new(98);
    let mut builder = Workspace::builder();
    let mut source = Node::tabs([A, B]);
    source.set_layout_title(Some(Arc::from("Tool group")));
    let source = builder.insert_node(source);
    let target = builder.insert_node(Node::tabs([C]));
    let split = builder.insert_node(Node::equal_split(Axis::Horizontal, [source, target]).unwrap());
    builder.set_root(root, RootRecord::new(split).with_central(target));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    let workspace = builder.build().unwrap();
    let policy = policy_snapshot();
    let config = DockPresentationConfig::default();
    let compile = |workspace: &Workspace| {
        let manifest = derive_scene_requirement_draft(
            authority_domain(),
            workspace,
            version(),
            PresentationConfigRevision::new(7),
            &policy,
            RequirementRevision::new(9),
            &surface_revisions([surface]),
        )
        .unwrap()
        .finalize(PopupPlaneRequirement::default())
        .unwrap();
        let requirements = manifest.surface(surface).unwrap();
        let mut measurements = SurfaceMeasurements::new(requirements.ticket());
        measurements
            .set_bounds(
                requirements.bounds(),
                Measurement::Measured(LogicalRect::new(0.0, 0.0, 800.0, 500.0).unwrap()),
            )
            .unwrap();
        for key in requirements.pane_sizings() {
            measurements
                .insert_pane_sizing(
                    key,
                    Measurement::Measured(crate::geometry::PaneSizing::default()),
                )
                .unwrap();
        }
        for key in requirements.tab_intrinsics() {
            let label = requirements.tab_label(key.tab()).unwrap();
            let width = if key.tab().tab == TabKey::for_layout(source) {
                assert_eq!(
                    label,
                    crate::graph::TabLabel::Layout {
                        title: "Tool group"
                    }
                );
                117.25
            } else {
                35.0
            };
            measurements
                .insert_tab_intrinsic(
                    key,
                    Measurement::Measured(TabIntrinsic::new(width).unwrap()),
                )
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
        let plan = compile_surface_measurements_without_guides(
            workspace,
            version(),
            &policy,
            &config,
            &manifest,
            &measurements,
            &TabStripStateStore::default(),
            &[],
            None,
        )
        .unwrap();
        PresentationPlanValidator::new(workspace, &policy)
            .unwrap()
            .validate_and_canonicalize(plan)
            .unwrap()
    };
    let presented = compile(&workspace);
    let prospective = TabSceneId {
        root,
        tabs: source,
        tab: TabKey::for_layout(source),
    };
    assert_eq!(
        presented.tab_label(prospective),
        Some(crate::graph::TabLabel::Layout {
            title: "Tool group"
        })
    );
    assert!(
        !presented
            .tab_records()
            .iter()
            .any(|tab| *tab.id() == prospective)
    );
    assert!(!presented.tab_bar_records().iter().any(|bar| {
        bar.members()
            .iter()
            .any(|member| member.tab() == prospective)
    }));
    let payload = crate::graph::command::MovePayload::Subtree(
        workspace.capture_node_source(root, source).unwrap(),
    );
    let target_proof = workspace.capture_tab_target(root, target).unwrap();
    let command = WorkspaceCommand::Move {
        payload: payload.clone(),
        target: crate::graph::command::DockTarget::Center(target_proof.clone()),
    };
    let mut candidate = workspace.clone();
    WorkspaceTransaction::from_commands([command])
        .apply(&mut candidate, &policy)
        .unwrap();
    let projection = project_future_root(&workspace, &candidate, &policy, &presented, None, root)
        .unwrap()
        .unwrap();
    let future = project_future_tab_strip(
        &workspace,
        &candidate,
        &policy,
        &presented,
        None,
        &target_proof,
        &payload,
        &projection,
    )
    .unwrap()
    .strip
    .unwrap();
    let incoming = future.tabs.iter().find(|tab| tab.payload).unwrap();
    assert_eq!(incoming.key, TabKey::for_layout(source));
    assert_eq!(incoming.layout_title.as_deref(), Some("Tool group"));
    let expected_width = super::super::tab_strip_layout::desired_tab_width(117.25, true, &config);
    assert_eq!(incoming.full_bounds.width(), expected_width);
    let committed = compile(&candidate);
    let actual = committed
        .tab_records()
        .iter()
        .find(|tab| tab.id().tab == incoming.key)
        .unwrap();
    assert_eq!(actual.full_bounds(), incoming.full_bounds);
    assert_eq!(
        committed.node_label(root, source),
        Some(crate::graph::TabLabel::Layout {
            title: "Tool group"
        })
    );
}

#[test]
fn frozen_root_and_subtree_labels_do_not_follow_selected_descendants() {
    let (mut workspace, surface, root, outer, inner, tools) = fixture();
    select(&mut workspace, root, outer, TabKey::for_layout(tools));
    let first = plan(&workspace, surface);
    assert_eq!(
        first.root_label(root),
        Some(crate::graph::TabLabel::Layout { title: "Tabs" })
    );
    assert_eq!(
        first.node_label(root, tools),
        Some(crate::graph::TabLabel::Layout { title: "Tools" })
    );
    select(&mut workspace, root, inner, TabKey::for_item(B));
    let second = plan(&workspace, surface);
    assert_eq!(first.root_label(root), second.root_label(root));
    assert_eq!(
        first.node_label(root, tools),
        second.node_label(root, tools)
    );
    let mut changed = workspace.into_builder();
    changed
        .draft_mut()
        .nodes
        .get_mut(tools)
        .unwrap()
        .set_layout_title(Some(Arc::from("Renamed tools")));
    let changed = changed.build().unwrap();
    let renamed = plan(&changed, surface);
    assert_eq!(
        renamed.node_label(root, tools),
        Some(crate::graph::TabLabel::Layout {
            title: "Renamed tools"
        })
    );
    assert_eq!(
        first.node_label(root, tools),
        Some(crate::graph::TabLabel::Layout { title: "Tools" })
    );
    assert_eq!(
        second.command_basis(root),
        renamed.command_basis(root),
        "a presentation rename cannot change structural close ownership"
    );
}

#[test]
fn contained_title_keeps_its_declared_label_when_content_is_fully_clipped() {
    let surface = SurfaceId::new(99);
    let root = RootId::new(100);
    let floating = FloatingPresentationId::new(101);
    let mut builder = Workspace::builder();
    let mut tabs = Node::tabs([A, B]);
    tabs.set_layout_title(Some(Arc::from("Compact tools")));
    let tabs = builder.insert_node(tabs);
    builder.set_root(root, RootRecord::new(tabs));
    builder.set_surface(surface, SurfacePresentation::rootless());
    builder.set_contained_floating(
        floating,
        ContainedFloating::new(root, LogicalRect::new(50.0, 50.0, 240.0, 20.0).unwrap()),
    );
    builder.attach_contained(surface, floating).unwrap();
    let workspace = builder.build().unwrap();
    let projected = compile_default_ready_plan(
        &workspace,
        surface,
        LogicalRect::new(0.0, 0.0, 800.0, 500.0).unwrap(),
    );
    assert!(projected.pane_records().is_empty());
    assert!(projected.tab_bar_records().is_empty());
    let contained = projected.contained_record(floating).unwrap();
    assert_eq!(contained.content_bounds().height(), 0.0);
    assert_eq!(
        contained.label(),
        Some(crate::graph::TabLabel::Layout {
            title: "Compact tools"
        })
    );
    assert_eq!(projected.root_label(root), contained.label());
    let mut root_label = None;
    projected
        .visit_surface_program(|step| {
            if let crate::presentation::scene::ResolvedSurfaceProgramStep::RootBegin(root) = step {
                root_label = root.label();
            }
            Ok::<(), ()>(())
        })
        .unwrap();
    assert_eq!(root_label, contained.label());
}
