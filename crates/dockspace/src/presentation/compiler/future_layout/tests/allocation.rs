use super::*;
use crate::model::RootAllocation;

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
            .find(|pane| committed.pane_selected(pane) == Some(MOVED))
            .unwrap();
        assert_eq!(resolved.visual().rect(), pane.content_bounds());
        assert_eq!(workspace.root(ROOT).unwrap().central, Some(anchor));
        assert_eq!(workspace.root(ROOT).unwrap().allocation, allocation);
        assert_future_tabs_match_committed(&resolved, &committed, anchor, &[MOVED]);
    }
}
