use super::*;
use crate::ids::FloatingPresentationId;

#[test]
fn hidden_members_do_not_publish_chrome_and_visible_gaps_keep_durable_insertion_positions() {
    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let items = [ItemId::new(1), ItemId::new(2), ItemId::new(3)];
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs(items));
    builder.set_root(root, RootRecord::new(tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    builder.set_hidden_items([items[1]].into());
    let workspace = builder.build().unwrap();
    let plan = compile_default_ready_plan(
        &workspace,
        surface,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
    );
    assert_eq!(
        plan.tab_records()
            .iter()
            .map(|record| record.id().tab)
            .collect::<BTreeSet<_>>(),
        [items[0], items[2]].map(TabKey::for_item).into()
    );
    let bar = &plan.tab_bar_records()[0];
    assert_eq!(
        bar.members()
            .iter()
            .map(|member| member.tab().tab)
            .collect::<Vec<_>>(),
        [items[0], items[2]].map(TabKey::for_item)
    );
    assert_eq!(
        bar.members()
            .iter()
            .map(|member| member.ordinal())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let gaps = plan
        .drop_targets()
        .iter()
        .filter_map(|target| match target.location() {
            DropTargetLocation::TabGap { index, .. } => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(gaps, [0, 2, 3]);
}

#[test]
fn tab_gap_positions_survive_hidden_roster_members_and_scrolling_to_the_end() {
    let cases: [(&str, &[u64], &[usize]); 5] = [
        ("all visible", &[], &[0, 1, 2, 3, 4, 5, 6, 7, 8]),
        ("hidden prefix", &[1, 2], &[2, 3, 4, 5, 6, 7, 8]),
        ("hidden middle", &[3, 4], &[0, 1, 4, 5, 6, 7, 8]),
        ("hidden suffix", &[7, 8], &[0, 1, 2, 3, 4, 5, 8]),
        (
            "hidden at multiple positions",
            &[1, 4, 8],
            &[1, 2, 4, 5, 6, 8],
        ),
    ];
    for (case, hidden, expected) in cases {
        let surface = SurfaceId::new(1);
        let root = RootId::new(1);
        let mut builder = Workspace::builder();
        let tabs = builder.insert_node(Node::tabs((1..=8).map(ItemId::new)));
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_surface(surface, SurfacePresentation::with_main(root));
        let mut workspace = builder.build().unwrap();
        workspace
            .replace_hidden_items(hidden.iter().copied().map(ItemId::new).collect())
            .unwrap();
        let gaps = |plan: &PresentationPlan| {
            plan.drop_targets()
                .iter()
                .filter_map(|target| match target.location() {
                    DropTargetLocation::TabGap {
                        tabs: owner, index, ..
                    } if owner == tabs => Some(index),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let wide = compile_default_ready_plan(
            &workspace,
            surface,
            LogicalRect::new(0.0, 0.0, 1600.0, 600.0).unwrap(),
        );
        assert_eq!(
            wide.tab_bar_records()[0].maximum_scroll_offset(),
            0.0,
            "{case}"
        );
        assert_eq!(
            gaps(&wide),
            expected,
            "{case}: all visible gaps use durable positions"
        );

        let policy = policy_snapshot();
        let config = DockPresentationConfig::builder()
            .tab_bar_height(24.0)
            .tab_group_grip_extent(0.0)
            .tab_min_width(100.0)
            .tab_max_width(100.0)
            .build()
            .unwrap();
        let initial = compile_ready_plan_with_config(
            &workspace,
            surface,
            LogicalRect::new(0.0, 0.0, 240.0, 600.0).unwrap(),
            true,
            &policy,
            &config,
        )
        .unwrap();
        let bar = &initial.tab_bar_records()[0];
        assert_eq!(bar.scroll_offset(), 0.0, "{case}");
        assert!(bar.maximum_scroll_offset() > 0.0, "{case}");
        // Between 1.5 and 2.5 fixed-width tabs leave exactly three gaps at
        // the end: before each of the last two members and after the roster.
        assert!(
            150.0 < bar.viewport().width() && bar.viewport().width() < 250.0,
            "{case}"
        );
        let bar_id = *bar.id();
        let key = TabStripStateKey::new(surface, bar_id);
        let mut states = TabStripStateStore::default();
        states
            .reconcile_exact_for_surface_roster(
                [(key, workspace.visible_tab_keys(tabs).as_ref())],
                false,
            )
            .unwrap();
        states
            .adopt_resolved_scroll_offsets(
                surface,
                [(
                    bar_id,
                    bar.scroll_offset(),
                    workspace.visible_tab_selection(tabs),
                    bar.viewport().width(),
                )],
            )
            .unwrap();
        states.set_tab_strip_scroll_offset(key, f64::MAX).unwrap();
        let manifest = derive_scene_requirement_draft(
            authority_domain(),
            &workspace,
            version(),
            PresentationConfigRevision::new(7),
            &policy,
            RequirementRevision::new(9),
            &surface_revisions([surface]),
        )
        .unwrap()
        .finalize(states.popup_requirement())
        .unwrap();
        let scrolled = compile_surface_measurements(
            &workspace,
            version(),
            &policy,
            &config,
            &manifest,
            initial.layout_facts().unwrap().measurements(),
            &states,
            &[],
            None,
        )
        .unwrap();
        let scrolled = PresentationPlanValidator::new(&workspace, &policy)
            .unwrap()
            .validate_and_canonicalize(scrolled)
            .unwrap();
        let bar = &scrolled.tab_bar_records()[0];
        assert_eq!(bar.scroll_offset(), bar.maximum_scroll_offset(), "{case}");
        assert_eq!(
            gaps(&scrolled),
            &expected[expected.len() - 3..],
            "{case}: scrolling clips gaps without renumbering durable insertion positions"
        );
        assert_eq!(
            gaps(&scrolled).last(),
            Some(&8),
            "{case}: the final gap remains after hidden trailing members"
        );
    }
}

#[test]
fn hidden_contained_root_keeps_ownership_without_frame_occlusion_or_drop_targets() {
    let surface = SurfaceId::new(1);
    let main = RootId::new(1);
    let contained = RootId::new(2);
    let floating = FloatingPresentationId::new(1);
    let main_item = ItemId::new(1);
    let contained_item = ItemId::new(2);
    let mut builder = Workspace::builder();
    let main_tabs = builder.insert_node(Node::tabs([main_item]));
    let contained_tabs = builder.insert_node(Node::tabs([contained_item]));
    builder.set_root(main, RootRecord::new(main_tabs));
    builder.set_root(contained, RootRecord::new(contained_tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(main));
    builder.set_contained_floating(
        floating,
        ContainedFloating::new(
            contained,
            LogicalRect::new(30.0, 40.0, 350.0, 240.0).unwrap(),
        ),
    );
    builder.attach_contained(surface, floating).unwrap();
    let mut workspace = builder.build().unwrap();
    workspace
        .replace_hidden_items([contained_item].into())
        .unwrap();
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    let plan = compile_default_ready_plan(&workspace, surface, bounds);
    assert!(plan.contained_records().is_empty());
    assert_eq!(plan.contained_minimums().len(), 1);
    assert_eq!(plan.contained_minimums()[0].floating(), floating);
    assert!(plan.drop_occlusions().is_empty());
    assert!(
        plan.pane_records()
            .iter()
            .all(|pane| pane.id().root == main)
    );
    assert!(
        plan.drop_targets()
            .iter()
            .all(|target| target.location().root() == Some(main))
    );
    assert!(workspace.contained_floating(floating).is_some());
    assert_eq!(workspace.surface(surface).unwrap().contained, [floating]);
    workspace.replace_hidden_items(Default::default()).unwrap();
    let restored = compile_default_ready_plan(&workspace, surface, bounds);
    assert_eq!(restored.contained_records().len(), 1);
    assert_eq!(restored.drop_occlusions().len(), 1);
    assert!(
        restored
            .tab_records()
            .iter()
            .any(|tab| tab.id().tab == contained_item.into())
    );
}

#[test]
fn fully_hidden_main_root_keeps_only_its_outer_docking_destinations() {
    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([ItemId::new(1)]));
    builder.set_root(root, RootRecord::new(tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    builder.set_hidden_items([ItemId::new(1)].into());
    let workspace = builder.build().unwrap();
    let plan = compile_default_ready_plan(
        &workspace,
        surface,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
    );
    assert!(plan.pane_records().is_empty());
    assert!(plan.tab_bar_records().is_empty());
    assert!(plan.tab_records().is_empty());
    assert!(plan.splitter_records().is_empty());
    assert!(plan.surface_background().is_none());
    assert_eq!(plan.drop_targets().len(), 4);
    assert!(plan.drop_targets().iter().all(|target| matches!(target.location(), DropTargetLocation::OuterEdge { root: owner, .. } if owner == root)));
    assert_eq!(workspace.root(root).unwrap().node, tabs);
}

#[test]
fn hidden_middle_pane_publishes_one_resize_handle_between_the_visible_siblings() {
    let surface = SurfaceId::new(1);
    let root = RootId::new(1);
    let mut builder = Workspace::builder();
    let leaves = [ItemId::new(1), ItemId::new(2), ItemId::new(3)]
        .map(|item| builder.insert_node(Node::tabs([item])));
    let split =
        builder.insert_node(Node::split(Axis::Horizontal, leaves, [0.2, 0.3, 0.5]).unwrap());
    builder.set_root(root, RootRecord::new(split));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    builder.set_hidden_items([ItemId::new(2)].into());
    let workspace = builder.build().unwrap();
    let plan = compile_default_ready_plan(
        &workspace,
        surface,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
    );
    assert_eq!(plan.pane_records().len(), 2);
    assert_eq!(plan.splitter_records().len(), 1);
    let handle = &plan.splitter_records()[0];
    assert_eq!(handle.id().index, 0);
    assert_eq!(handle.after_index(), 2);
    assert_eq!(handle.child_extents()[1], 0.0);
    assert!(
        plan.drop_targets()
            .iter()
            .all(|target| match target.location() {
                DropTargetLocation::Center { tabs, .. }
                | DropTargetLocation::TabGap { tabs, .. } => tabs != leaves[1],
                DropTargetLocation::InnerEdge { node, .. } => node != leaves[1],
                _ => true,
            })
    );
}

#[test]
fn hidden_contained_roster_holes_preserve_program_order_layers_and_minimum_inventory() {
    let surface = SurfaceId::new(1);
    let main = RootId::new(1);
    let mut builder = Workspace::builder();
    let main_tabs = builder.insert_node(Node::tabs([ItemId::new(1)]));
    builder.set_root(main, RootRecord::new(main_tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(main));
    let floatings = [1_u64, 2, 3].map(FloatingPresentationId::new);
    for (index, floating) in floatings.into_iter().enumerate() {
        let item = ItemId::new(u64::try_from(index + 2).unwrap());
        let root = RootId::new(u64::try_from(index + 2).unwrap());
        let tabs = builder.insert_node(Node::tabs([item]));
        builder.set_root(root, RootRecord::new(tabs));
        builder.set_contained_floating(
            floating,
            ContainedFloating::new(
                root,
                LogicalRect::new(
                    40.0 + 250.0 * f64::from(u16::try_from(index).unwrap()),
                    100.0,
                    220.0,
                    280.0,
                )
                .unwrap(),
            ),
        );
        builder.attach_contained(surface, floating).unwrap();
    }
    let mut workspace = builder.build().unwrap();
    for hide_frontmost in [false, true] {
        let mut hidden = BTreeSet::from([ItemId::new(3)]);
        if hide_frontmost {
            hidden.insert(ItemId::new(4));
        }
        workspace.replace_hidden_items(hidden).unwrap();
        let plan = compile_default_ready_plan(
            &workspace,
            surface,
            LogicalRect::new(0.0, 0.0, 1000.0, 600.0).unwrap(),
        );
        let expected = if hide_frontmost {
            vec![(floatings[0], 0)]
        } else {
            vec![(floatings[0], 0), (floatings[2], 2)]
        };
        assert_eq!(
            plan.contained_records()
                .iter()
                .map(|record| (record.floating(), record.ordinal()))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(plan.contained_records().last().unwrap().is_frontmost());
        assert!(
            plan.contained_records()
                .iter()
                .rev()
                .skip(1)
                .all(|record| !record.is_frontmost())
        );
        assert_eq!(
            plan.contained_minimums()
                .iter()
                .map(|measurement| measurement.floating())
                .collect::<Vec<_>>(),
            floatings
        );
        assert_eq!(plan.drop_occlusions().len(), expected.len());
        let roots = plan
            .surface_program()
            .blocks()
            .iter()
            .filter_map(|block| match block {
                crate::presentation::scene::SurfaceProgramBlock::RootBegin {
                    presentation: SurfaceProgramRootPresentation::Contained(floating),
                    ..
                } => Some(*floating),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            roots,
            expected
                .iter()
                .map(|&(floating, _)| floating)
                .collect::<Vec<_>>()
        );
    }
}
