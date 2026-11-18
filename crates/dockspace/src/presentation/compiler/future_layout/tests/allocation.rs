use super::*;
use crate::model::RootAllocation;

#[test]
fn a_fractional_contained_header_that_fills_its_body_matches_the_committed_preview() {
    let target_root = RootId::new(2);
    let floating = FloatingPresentationId::new(1);
    let retained = ItemId::new(2);
    let mut builder = Workspace::builder();
    let source = builder.insert_node(Node::tabs([MOVED]));
    let target = builder.insert_node(Node::tabs([retained]));
    builder.set_root(ROOT, RootRecord::new(source).with_central(source));
    builder.set_root(target_root, RootRecord::new(target));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_contained_floating(
        floating,
        ContainedFloating::new(
            target_root,
            LogicalRect::new(
                152.336_611_439_481_7,
                10.895_509_511_466_425,
                390.454_536_638_563_47,
                50.238_274_954_240_115,
            )
            .unwrap(),
        ),
    );
    builder.attach_contained(SURFACE, floating).unwrap();
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
            root: target_root,
            tabs: target,
            index: 1,
        },
    );
    WorkspaceTransaction::from_commands([resolved.command().clone()])
        .apply(&mut workspace, &policy)
        .unwrap();
    let committed = compile(&workspace, &policy, &config, strip, &mut states);
    let contained = committed.contained_record(floating).unwrap();
    let bar = target_bar(&committed, target);
    assert_eq!(bar.bounds(), contained.content_bounds());
    let tabs = committed
        .tabs_records()
        .iter()
        .find(|tabs| tabs.id().tabs == target)
        .unwrap();
    assert_eq!(
        tabs.content_bounds().max(),
        contained.content_bounds().max()
    );
    assert_eq!(tabs.content_bounds().height(), 0.0);
    assert_future_tabs_match_committed(&resolved, &committed, target, &[MOVED]);
}

#[test]
fn central_preview_uses_the_explicit_allocation_and_matches_committed_geometry() {
    for allocation in [RootAllocation::Weighted, RootAllocation::PreferCentral] {
        let source_surface = SurfaceId::new(2);
        let source_root = RootId::new(2);
        let left_item = ItemId::new(10);
        let mut builder = Workspace::builder();
        let left = builder.insert_node(Node::tabs([left_item]));
        let anchor = builder.insert_node(Node::tabs([ItemId::new(11)]));
        let right = builder.insert_node(Node::tabs([ItemId::new(12)]));
        let split = builder.insert_node(
            Node::split(Axis::Horizontal, [left, anchor, right], [1.0, 2.0, 1.0]).unwrap(),
        );
        let source = builder.insert_node(Node::tabs([MOVED]));
        builder.set_root(
            ROOT,
            RootRecord::new(split)
                .with_central(anchor)
                .with_allocation(allocation),
        );
        builder.set_root(source_root, RootRecord::new(source));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        builder.set_surface(source_surface, SurfacePresentation::with_main(source_root));
        let mut workspace = builder.build().unwrap();
        let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
        let config = DockPresentationConfig::default();
        let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
        let minimums = BTreeMap::from([(left_item, LogicalSize::new(300.0, 0.0).unwrap())]);
        let mut states = TabStripStateStore::default();
        let target_plan = compile_on_surface_with_minimums(
            &workspace,
            &policy,
            &config,
            strip,
            &mut states,
            SURFACE,
            &minimums,
        );
        let source_plan = compile_on_surface_with_minimums(
            &workspace,
            &policy,
            &config,
            strip,
            &mut states,
            source_surface,
            &minimums,
        );
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
                tabs: anchor,
            },
        );
        let available = 1000.0 - 2.0 * config.splitter_thickness();
        let delta = 300.0 - available * 0.25;
        let expected_width = available * 0.5
            - match allocation {
                RootAllocation::Weighted => delta * (2.0 / 3.0),
                RootAllocation::PreferCentral => delta,
            };
        assert!((resolved.visual().rect().width() - expected_width).abs() < 1.0e-6);
        WorkspaceTransaction::from_commands([resolved.command().clone()])
            .apply(&mut workspace, &policy)
            .unwrap();
        let committed = compile_on_surface_with_minimums(
            &workspace,
            &policy,
            &config,
            strip,
            &mut states,
            SURFACE,
            &minimums,
        );
        let pane = committed
            .pane_records()
            .iter()
            .find(|pane| pane.item() == MOVED)
            .unwrap();
        assert_eq!(resolved.visual().rect(), pane.content_bounds());
        assert_eq!(workspace.root(ROOT).unwrap().central, Some(anchor));
        assert_eq!(workspace.root(ROOT).unwrap().allocation, allocation);
        assert_future_tabs_match_committed(&resolved, &committed, anchor, &[MOVED]);
    }
}
