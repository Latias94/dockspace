use super::*;
use crate::graph::command::Edge;
use crate::presentation::tab_strip::TabStripControlId;

#[test]
fn clipped_revealed_single_sibling_keeps_preview_and_commit_aligned_across_surfaces() {
    for cross_surface in [false, true] {
        let source_surface = if cross_surface {
            SurfaceId::new(2)
        } else {
            SURFACE
        };
        let target_root = RootId::new(3);
        let sibling_item = ItemId::new(10);
        let target_item = ItemId::new(11);
        let floating = FloatingPresentationId::new(1);
        let mut builder = Workspace::builder();
        let source = builder.insert_node(Node::tabs([MOVED, ItemId::new(2)]));
        let sibling = builder.insert_node(Node::tabs([sibling_item]));
        let target = builder.insert_node(Node::tabs([target_item]));
        let split =
            builder.insert_node(Node::equal_split(Axis::Vertical, [sibling, target]).unwrap());
        builder.set_root(ROOT, RootRecord::new(source).with_central(source));
        builder.set_surface(source_surface, SurfacePresentation::with_main(ROOT));
        if cross_surface {
            builder.set_surface(SURFACE, SurfacePresentation::rootless());
        }
        builder.set_root(target_root, RootRecord::new(split));
        builder.set_contained_floating(
            floating,
            ContainedFloating::new(
                target_root,
                LogicalRect::new(200.0, -60.0, 400.0, 700.0).unwrap(),
            ),
        );
        builder.attach_contained(SURFACE, floating).unwrap();
        let mut workspace = builder.build().unwrap();
        let mut policy = DockPolicy::default();
        policy.set_tab_bar(TabBarPolicy::new(
            TabBarVisibility::AutoHideSingle,
            TabBarInteraction::Enabled,
        ));
        let policy = policy.snapshot(PolicyRevision::new(1));
        let config = DockPresentationConfig::default();
        let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
        let minimums = BTreeMap::from([(sibling_item, LogicalSize::new(100.0, 400.0).unwrap())]);
        let mut states = TabStripStateStore::default();
        compile_on_surface_with_minimums(
            &workspace,
            &policy,
            &config,
            strip,
            &mut states,
            SURFACE,
            &minimums,
        );
        let key = TabStripStateKey::new(
            SURFACE,
            TabBarSceneId {
                root: target_root,
                tabs: sibling,
            },
        );
        states.reveal_single_tab_bar(key).unwrap();
        let target_plan = compile_on_surface_with_minimums(
            &workspace,
            &policy,
            &config,
            strip,
            &mut states,
            SURFACE,
            &minimums,
        );
        assert!(
            target_plan
                .tab_bar_records()
                .iter()
                .all(|bar| bar.id().tabs != sibling)
        );
        assert_eq!(
            target_plan
                .layout_facts()
                .unwrap()
                .root(target_root)
                .unwrap()
                .tab_bar_visibility(sibling),
            Some(TabBarVisibility::Visible)
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
            MovePayload::Item(workspace.capture_item_source(ROOT, source, MOVED).unwrap()),
            DropTargetId::InnerEdge {
                surface: SURFACE,
                root: target_root,
                node: target,
                edge: Edge::Bottom,
            },
        );
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
        assert_eq!(
            resolved.visual().rect(),
            pane.content_bounds(),
            "cross_surface={cross_surface}"
        );
        assert_eq!(states.revealed_single_tab(key), Some(sibling_item.into()));
    }
}

#[test]
fn a_visible_single_tab_reveal_control_always_reveals_a_visible_header() {
    let floating = FloatingPresentationId::new(1);
    let mut policy = DockPolicy::default();
    policy.set_tab_bar(TabBarPolicy::new(
        TabBarVisibility::AutoHideSingle,
        TabBarInteraction::Enabled,
    ));
    let policy = policy.snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::default();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    for top in [-60.0, -41.0, -35.0, -29.0, 100.0] {
        let mut builder = Workspace::builder();
        let tabs = builder.insert_node(Node::tabs([MOVED]));
        builder.set_root(ROOT, RootRecord::new(tabs));
        builder.set_surface(SURFACE, SurfacePresentation::rootless());
        builder.set_contained_floating(
            floating,
            ContainedFloating::new(ROOT, LogicalRect::new(200.0, top, 400.0, 300.0).unwrap()),
        );
        builder.attach_contained(SURFACE, floating).unwrap();
        let workspace = builder.build().unwrap();
        let mut states = TabStripStateStore::default();
        let hidden = compile(&workspace, &policy, &config, strip, &mut states);
        let bar = TabBarSceneId { root: ROOT, tabs };
        let control = hidden
            .tab_strip_control_records()
            .iter()
            .find(|control| control.id() == TabStripControlId::RevealTabBar(bar));
        if top == -60.0 {
            assert!(
                control.is_none(),
                "a fully clipped header cannot offer a relocated reveal control"
            );
        } else {
            assert!(
                control.is_some(),
                "a visible header anchor must remain operable at y={top}"
            );
        }
        states
            .reveal_single_tab_bar(TabStripStateKey::new(SURFACE, bar))
            .unwrap();
        let revealed = compile(&workspace, &policy, &config, strip, &mut states);
        if control.is_some() {
            assert!(
                revealed
                    .tab_bar_records()
                    .iter()
                    .any(|record| *record.id() == bar),
                "revealing at y={top} must produce a visible bar"
            );
        }
    }
}
