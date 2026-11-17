//! A root may retire while its native surface keeps the same contained payload.

use super::*;

#[test]
fn same_surface_floating_of_a_sole_nested_tab_preserves_binding_without_a_rehome_barrier() {
    for exact_layout_tab in [false, true] {
        let (mut fixture, source) = native_tab_source(true, SourceRemainder::None);
        let before = fixture.engine.workspace().clone();
        let version = fixture.engine.version();
        let inner = fixture.source_tabs;
        let layout_tab = TabKey::for_layout(inner);
        assert_ne!(before.root(SOURCE_ROOT).unwrap().node, inner);
        let prepared = if exact_layout_tab {
            let tab = fixture
                .engine
                .capture_tab(layout_tab)
                .expect("the sole direct layout tab must retain its own subtree identity");
            fixture.engine.prepare_float_tab(tab, SOURCE_SURFACE)
        } else {
            let group = fixture
                .engine
                .capture_tabs_group(DockAnchor::Item(FIRST))
                .unwrap();
            assert_eq!(group.source.node(), inner);
            fixture.engine.prepare_float_tabs(group, SOURCE_SURFACE)
        };
        let input = fixture.engine.accept_prepared_action(prepared).unwrap();
        let floated = submit_test_input(&mut fixture.engine, fixture.presentation_host, input)
            .expect("same-surface floating must produce a normal product outcome");
        let (root, floating) = match floated.reduced_inputs()[0].outcome() {
            InputOutcome::ProductActionProcessed {
                outcome:
                    DockspaceActionOutcome::TabsFloated {
                        source_root: SOURCE_ROOT,
                        root,
                        surface: SOURCE_SURFACE,
                        floating,
                        items,
                        changed: true,
                    },
                ..
            } if !exact_layout_tab && items == &[FIRST, SECOND] => (*root, *floating),
            InputOutcome::ProductActionProcessed {
                outcome:
                    DockspaceActionOutcome::TabFloated {
                        tab,
                        source_root: SOURCE_ROOT,
                        root,
                        surface: SOURCE_SURFACE,
                        floating,
                        items,
                        changed: true,
                    },
                ..
            } if exact_layout_tab && *tab == layout_tab && items == &[FIRST, SECOND] => {
                (*root, *floating)
            }
            outcome => panic!(
                "the native surface survives its old root; exact_layout_tab={exact_layout_tab}: {outcome:?}"
            ),
        };
        assert!(fixture.engine.pending_mutation.is_none());
        assert_no_source_release(&floated, source);
        assert!(floated.events().iter().all(|event| !matches!(
            event.kind(),
            WorkspaceEventKind::PresentationRehomeSettled { .. }
        )));
        assert_eq!(
            fixture.engine.version().revision(),
            version.revision().checked_next().unwrap()
        );
        assert_eq!(fixture.engine.version().epoch(), version.epoch());
        let workspace = fixture.engine.workspace();
        assert!(workspace.root(SOURCE_ROOT).is_none());
        assert_eq!(workspace.root(root).unwrap().node, inner);
        assert_eq!(
            workspace.presentation_for_root(root),
            Some(crate::graph::workspace::RootPresentationOwner::Contained {
                surface: SOURCE_SURFACE,
                floating,
            })
        );
        let surface = workspace.surface(SOURCE_SURFACE).unwrap();
        assert_eq!(surface.main_root, None);
        assert_eq!(surface.contained, [floating]);
        assert_eq!(workspace.item_multiset(), before.item_multiset());
        assert_eq!(workspace.collect_items_in_subtree(inner), [FIRST, SECOND]);
        assert_eq!(workspace.node(inner).unwrap().layout_title(), Some("Tools"));
        assert_eq!(workspace.node(inner), before.node(inner));
        assert_eq!(
            fixture
                .engine
                .viewport()
                .viewport(SOURCE_SURFACE)
                .map(|record| { (record.binding(), record.admission()) }),
            Some((source, ViewportAdmission::Admitted))
        );
        assert_eq!(
            fixture
                .engine
                .bound_surface_recoveries
                .get(&SOURCE_SURFACE)
                .map(|bound| bound.binding),
            Some(source)
        );
    }
}

#[test]
fn same_surface_move_to_a_contained_peer_preserves_binding_and_exact_payload_scope() {
    const PEER: ItemId = ItemId::new(9);

    for exact_layout_tab in [false, true] {
        let (mut fixture, source) = native_tab_source(true, SourceRemainder::None);
        let expected = fixture.engine.version();
        let opened = submit_test_input(
            &mut fixture.engine,
            fixture.presentation_host,
            EngineInput::OpenItem {
                expected,
                item: PEER,
                placement: DockPlacement::Center(DockAnchor::Item(TARGET)),
            },
        )
        .expect("the existing target must retain a peer while a new item opens");
        assert!(matches!(
            opened.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed { .. }
        ));
        let expected = fixture.engine.version();
        let floated = submit_test_input(
            &mut fixture.engine,
            fixture.presentation_host,
            EngineInput::FloatItem {
                expected,
                item: PEER,
                surface: SOURCE_SURFACE,
            },
        )
        .expect("an ordinary partial item float must establish the same-surface peer");
        assert!(matches!(
            floated.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed {
                outcome: DockspaceActionOutcome::Floated { item: PEER, .. },
                ..
            }
        ));
        assert_no_source_release(&opened, source);
        assert_no_source_release(&floated, source);
        assert!(fixture.engine.pending_mutation.is_none());

        let before = fixture.engine.workspace().clone();
        let version = fixture.engine.version();
        let inner = fixture.source_tabs;
        let layout_tab = TabKey::for_layout(inner);
        let peer_root = before.locate_item_by_id(PEER).unwrap().root;
        let peer_owner = before.presentation_for_root(peer_root).unwrap();
        let crate::graph::workspace::RootPresentationOwner::Contained {
            surface: SOURCE_SURFACE,
            floating,
        } = peer_owner
        else {
            panic!("the peer must already belong to this exact native surface");
        };
        let peer_tabs = before.root(peer_root).unwrap().node;
        assert_ne!(before.root(SOURCE_ROOT).unwrap().node, inner);
        let placement = DockPlacement::Center(DockAnchor::Item(PEER));
        let prepared = if exact_layout_tab {
            let tab = fixture.engine.capture_tab(layout_tab).unwrap();
            fixture.engine.prepare_dock_tab(tab, placement)
        } else {
            let group = fixture
                .engine
                .capture_tabs_group(DockAnchor::Item(FIRST))
                .unwrap();
            fixture.engine.prepare_dock_tabs(group, placement)
        };
        let input = fixture.engine.accept_prepared_action(prepared).unwrap();
        let moved = submit_test_input(&mut fixture.engine, fixture.presentation_host, input)
            .expect("moving to a same-surface contained peer must be an ordinary product action");
        match moved.reduced_inputs()[0].outcome() {
            InputOutcome::ProductActionProcessed {
                outcome:
                    DockspaceActionOutcome::TabsDocked {
                        source_root: SOURCE_ROOT,
                        target_root,
                        items,
                        changed: true,
                    },
                ..
            } if !exact_layout_tab && *target_root == peer_root && items == &[FIRST, SECOND] => {}
            InputOutcome::ProductActionProcessed {
                outcome:
                    DockspaceActionOutcome::TabDocked {
                        tab,
                        source_root: SOURCE_ROOT,
                        target_root,
                        items,
                        changed: true,
                    },
                ..
            } if exact_layout_tab
                && *tab == layout_tab
                && *target_root == peer_root
                && items == &[FIRST, SECOND] => {}
            outcome => panic!(
                "same-surface Move must retain its real payload; exact_layout_tab={exact_layout_tab}: {outcome:?}"
            ),
        }
        assert!(fixture.engine.pending_mutation.is_none());
        assert_no_source_release(&moved, source);
        assert_eq!(
            fixture.engine.version().revision(),
            version.revision().checked_next().unwrap()
        );
        let workspace = fixture.engine.workspace();
        assert!(workspace.root(SOURCE_ROOT).is_none());
        assert_eq!(workspace.presentation_for_root(peer_root), Some(peer_owner));
        let surface = workspace.surface(SOURCE_SURFACE).unwrap();
        assert_eq!(surface.main_root, None);
        assert_eq!(surface.contained, [floating]);
        assert_eq!(workspace.item_multiset(), before.item_multiset());
        assert_eq!(
            workspace.collect_items_in_subtree(peer_tabs),
            [PEER, FIRST, SECOND]
        );
        for item in [PEER, FIRST, SECOND] {
            assert_eq!(workspace.locate_item_by_id(item).unwrap().root, peer_root);
        }
        let Some(Node::Tabs { items, .. }) = workspace.node(peer_tabs) else {
            panic!("center docking must preserve the peer's existing tabs owner");
        };
        if exact_layout_tab {
            assert_eq!(items, &[TabKey::for_item(PEER), layout_tab]);
            assert_eq!(workspace.node(inner), before.node(inner));
        } else {
            assert_eq!(
                items,
                &[
                    TabKey::for_item(PEER),
                    TabKey::for_item(FIRST),
                    TabKey::for_item(SECOND)
                ]
            );
        }
        assert_eq!(
            fixture
                .engine
                .viewport()
                .viewport(SOURCE_SURFACE)
                .map(|record| { (record.binding(), record.admission()) }),
            Some((source, ViewportAdmission::Admitted))
        );
        assert_eq!(
            fixture
                .engine
                .bound_surface_recoveries
                .get(&SOURCE_SURFACE)
                .map(|bound| bound.binding),
            Some(source)
        );
    }
}
