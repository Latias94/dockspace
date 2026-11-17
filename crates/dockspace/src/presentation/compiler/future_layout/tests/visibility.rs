use super::*;

#[test]
fn hidden_target_member_gap_preview_matches_the_durable_insertion_and_committed_strip() {
    let retained = [ItemId::new(10), ItemId::new(11), ItemId::new(12)];
    let mut builder = Workspace::builder();
    let source = builder.insert_node(Node::tabs([MOVED]));
    let target = builder.insert_node(Node::tabs(retained));
    let split = builder.insert_node(Node::equal_split(Axis::Horizontal, [source, target]).unwrap());
    builder.set_root(ROOT, RootRecord::new(split).with_central(target));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_hidden_items([retained[1]].into());
    let mut workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::default();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    let mut states = TabStripStateStore::default();
    let plan = compile(&workspace, &policy, &config, strip, &mut states);
    let resolved = resolve_target(
        &workspace,
        &policy,
        &plan,
        &plan,
        MovePayload::Item(workspace.capture_item_source(ROOT, source, MOVED).unwrap()),
        DropTargetId::TabGap {
            surface: SURFACE,
            root: ROOT,
            tabs: target,
            index: 2,
        },
    );
    WorkspaceTransaction::from_commands([resolved.command().clone()])
        .apply(&mut workspace, &policy)
        .unwrap();
    let Some(Node::Tabs { items, .. }) = workspace.node(target) else {
        unreachable!()
    };
    assert_eq!(
        items,
        &[retained[0], retained[1], MOVED, retained[2]].map(TabKey::for_item)
    );
    assert_eq!(
        workspace.visible_tab_keys(target).as_ref(),
        &[retained[0], MOVED, retained[2]].map(TabKey::for_item)
    );
    assert!(workspace.item_is_hidden(retained[1]));
    let committed = compile(&workspace, &policy, &config, strip, &mut states);
    assert_future_tabs_match_committed(&resolved, &committed, target, &[MOVED]);
}

#[test]
fn all_hidden_main_outer_drop_previews_the_same_full_space_as_the_committed_visible_pane() {
    let other_surface = SurfaceId::new(2);
    let other_root = RootId::new(2);
    let hidden_item = ItemId::new(10);
    let mut builder = Workspace::builder();
    let target = builder.insert_node(Node::tabs([hidden_item]));
    let source = builder.insert_node(Node::tabs([MOVED]));
    builder.set_root(ROOT, RootRecord::new(target));
    builder.set_root(other_root, RootRecord::new(source));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_surface(other_surface, SurfacePresentation::with_main(other_root));
    builder.set_hidden_items([hidden_item].into());
    let mut workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::default();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    let mut states = TabStripStateStore::default();
    let target_plan = compile(&workspace, &policy, &config, strip, &mut states);
    let source_plan = compile_on_surface(
        &workspace,
        &policy,
        &config,
        strip,
        &mut states,
        other_surface,
    );
    let resolved = resolve_target(
        &workspace,
        &policy,
        &target_plan,
        &source_plan,
        MovePayload::Item(
            workspace
                .capture_item_source(other_root, source, MOVED)
                .unwrap(),
        ),
        DropTargetId::OuterEdge {
            surface: SURFACE,
            root: ROOT,
            node: target,
            edge: crate::graph::command::Edge::Right,
        },
    );
    WorkspaceTransaction::from_commands([resolved.command().clone()])
        .apply(&mut workspace, &policy)
        .unwrap();
    let committed = compile(&workspace, &policy, &config, strip, &mut states);
    let pane = committed
        .pane_records()
        .iter()
        .find(|pane| pane.item() == MOVED)
        .unwrap();
    assert_eq!(resolved.visual().rect(), pane.bounds());
    assert_eq!(
        pane.bounds(),
        LogicalRect::new(
            0.0,
            config.tab_bar_height(),
            1000.0,
            600.0 - config.tab_bar_height(),
        )
        .unwrap()
    );
    assert_eq!(
        committed
            .tabs_records()
            .iter()
            .find(|tabs| tabs.id().root == ROOT)
            .unwrap()
            .bounds(),
        committed.bounds()
    );
    assert!(workspace.item_is_hidden(hidden_item));
    assert!(workspace.node(target).is_some());
    assert!(
        committed
            .tab_records()
            .iter()
            .all(|tab| tab.id().tab != hidden_item.into())
    );
}
