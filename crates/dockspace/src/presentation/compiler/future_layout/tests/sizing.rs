use super::*;

#[test]
fn an_inactive_items_complete_sizing_follows_it_into_a_center_preview_and_commit() {
    let source_surface = SurfaceId::new(2);
    let source_root = RootId::new(2);
    let selected_source = ItemId::new(10);
    let target_item = ItemId::new(11);
    let mut builder = Workspace::builder();
    let target = builder.insert_node(Node::tabs([target_item]));
    let source = builder.insert_node(Node::tabs([selected_source, MOVED]));
    builder.set_root(ROOT, RootRecord::new(target));
    builder.set_root(source_root, RootRecord::new(source));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_surface(source_surface, SurfacePresentation::with_main(source_root));
    let mut workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::default();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    let sizing = BTreeMap::from([
        (
            Some(MOVED),
            PaneSizing::builder()
                .fixed_width(180.0)
                .fixed_height(120.0)
                .build()
                .unwrap(),
        ),
        (
            Some(selected_source),
            PaneSizing::builder().fixed_width(400.0).build().unwrap(),
        ),
        (
            Some(target_item),
            PaneSizing::builder().fixed_width(360.0).build().unwrap(),
        ),
    ]);
    let mut states = TabStripStateStore::default();
    let target_plan = compile_on_surface_with_sizing(
        &workspace,
        &policy,
        &config,
        strip,
        &mut states,
        SURFACE,
        &sizing,
    );
    let source_plan = compile_on_surface_with_sizing(
        &workspace,
        &policy,
        &config,
        strip,
        &mut states,
        source_surface,
        &sizing,
    );
    assert_eq!(
        workspace.visible_tab_selection(source),
        Some(selected_source.into())
    );
    assert_eq!(source_plan.pane_records()[0].bounds().width(), 400.0);
    let resolved = resolve_target(
        &workspace,
        &policy,
        &target_plan,
        &source_plan,
        MovePayload::Item(
            workspace
                .capture_item_source(source_root, source, MOVED)
                .unwrap(),
        ),
        DropTargetId::Center {
            surface: SURFACE,
            root: ROOT,
            tabs: target,
        },
    );
    assert_eq!(resolved.visual().rect().width(), 180.0);
    assert_eq!(resolved.visual().rect().height(), 120.0);
    WorkspaceTransaction::from_commands([resolved.command().clone()])
        .apply(&mut workspace, &policy)
        .unwrap();
    let committed = compile_on_surface_with_sizing(
        &workspace,
        &policy,
        &config,
        strip,
        &mut states,
        SURFACE,
        &sizing,
    );
    let pane = committed
        .pane_records()
        .iter()
        .find(|pane| pane.item() == MOVED)
        .unwrap();
    assert_eq!(pane.content_bounds(), resolved.visual().rect());
    assert_eq!(
        workspace.visible_tab_keys(source).as_ref(),
        [selected_source.into()]
    );
}

#[test]
fn an_empty_central_anchor_uses_core_baseline_without_an_application_measurement() {
    let mut builder = Workspace::builder();
    let anchor = builder.insert_node(Node::tabs([]));
    let second = builder.insert_node(Node::tabs([ItemId::new(2)]));
    let third = builder.insert_node(Node::tabs([ItemId::new(3)]));
    let split = builder.insert_node(
        Node::split(Axis::Horizontal, [anchor, second, third], [2.0, 1.0, 1.0]).unwrap(),
    );
    builder.set_root(ROOT, RootRecord::new(split).with_central(anchor));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    let workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::default();
    let sizing = BTreeMap::from([(
        None,
        PaneSizing::builder().fixed_width(200.0).build().unwrap(),
    )]);
    let plan = compile_on_surface_with_sizing(
        &workspace,
        &policy,
        &config,
        TabStripMetrics::new(0.0, 0.0).unwrap(),
        &mut TabStripStateStore::default(),
        SURFACE,
        &sizing,
    );
    let future =
        super::super::project_future_root(&workspace, &workspace, &policy, &plan, None, ROOT)
            .unwrap()
            .unwrap();
    let available = 1000.0 - 2.0 * config.splitter_thickness();
    let sibling = available * 0.25;
    assert_eq!(future.nodes[&anchor].rect.width(), available * 0.5);
    assert_eq!(future.nodes[&second].rect.width(), sibling);
    assert_eq!(future.nodes[&third].rect.width(), sibling);
    assert_eq!(
        plan.layout_facts()
            .unwrap()
            .root(ROOT)
            .unwrap()
            .pane_sizings()
            .count(),
        2
    );
    assert!(
        !plan
            .pane_records()
            .iter()
            .any(|pane| pane.owner_node() == anchor)
    );
    assert!(
        plan.tabs_records()
            .iter()
            .any(|tabs| tabs.id().tabs == anchor)
    );
    assert_eq!(
        &future,
        plan.layout_facts()
            .unwrap()
            .root(ROOT)
            .unwrap()
            .projection()
    );
}
