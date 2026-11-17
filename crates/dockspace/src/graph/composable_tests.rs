use std::collections::BTreeSet;
use std::sync::Arc;

use crate::close::plan::{CloseItemRequirement, ContentCloseScope, prepare_scoped_content_close};
use crate::graph::command::{
    CloseCommitOutcome, CommandOutcome, DockTarget, MovePayload, WorkspaceCommand,
};
use crate::graph::error::{CommandError, TransactionError};
use crate::graph::root_structure::RootNodeKind;
use crate::graph::transaction::WorkspaceTransaction;
use crate::graph::{Node, RootRecord, SurfacePresentation, TabLabel, Workspace};
use crate::ids::{ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::model::{
    DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    DockspaceTabView,
};
use crate::policy::DockPolicySnapshot;

mod edit_regressions;

const ROOT: RootId = RootId::new(1);
const SURFACE: SurfaceId = SurfaceId::new(1);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);

fn vertical_layout() -> DockspaceLayout {
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::pane(C),
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [DockspaceNode::tabs([A, B]), DockspaceNode::pane(D)],
                )
                .unwrap()
                .named("Tools"),
            ]),
        ),
    )])
    .unwrap()
}

fn activate(
    workspace: &mut Workspace,
    item: ItemId,
) -> crate::graph::transaction::TransactionReport {
    let source = workspace.capture_item_source_by_id(item).unwrap().unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::SelectItem { source }])
        .apply(workspace, &DockPolicySnapshot::default())
        .unwrap()
}

fn outer_and_tools(workspace: &Workspace) -> (NodeId, TabKey, NodeId) {
    let outer = workspace.root(ROOT).unwrap().node;
    let Node::Tabs { items, .. } = workspace.node(outer).unwrap() else {
        unreachable!()
    };
    let tools = items[1];
    (outer, tools, tools.layout().unwrap())
}

#[test]
fn first_vertical_fixture_keeps_real_ownership_bare_content_and_declared_title() {
    let layout = vertical_layout();
    let view = layout.view();
    assert_eq!(view.item(A).unwrap().tab_index(), Some(0));
    assert!(view.item(A).unwrap().tabs().is_some());
    assert_eq!(view.item(D).unwrap().tab_index(), None);
    assert!(view.item(D).unwrap().tabs().is_none());
    assert!(view.item(C).unwrap().is_active());
    assert!(!view.item(A).unwrap().is_active());
    assert!(!view.item(D).unwrap().is_active());
    assert_eq!(
        view.surface(SURFACE)
            .unwrap()
            .active_items()
            .collect::<Vec<_>>(),
        [C]
    );
    assert_eq!(
        view.root(ROOT).unwrap().active_items().collect::<Vec<_>>(),
        [C]
    );
    assert_eq!(
        view.root(ROOT).unwrap().items().collect::<Vec<_>>(),
        [C, A, B, D]
    );
    let root = view.root(ROOT).unwrap().content().unwrap();
    assert_eq!(root.items().collect::<Vec<_>>(), [C, A, B, D]);
    assert_eq!(root.tabs().unwrap().key(), root.key());
    let children: Vec<_> = root.tabs().unwrap().children().collect();
    assert!(matches!(children[0], DockspaceTabView::Item(C)));
    assert!(
        matches!(children[1], DockspaceTabView::Layout(node) if node.layout_title() == Some("Tools"))
    );
    let workspace = layout.into_workspace();
    let (outer, tools, split) = outer_and_tools(&workspace);
    assert_eq!(workspace.nodes().count(), 4);
    assert_eq!(workspace.first_active_item(outer), Some(C));
    assert_eq!(
        workspace.tab_label(outer, tools),
        Some(TabLabel::Layout { title: "Tools" })
    );
    assert_eq!(
        workspace
            .node(outer)
            .unwrap()
            .child_nodes()
            .collect::<Vec<_>>(),
        [(1, split)]
    );
    let structure = workspace.root_structure(ROOT).unwrap();
    assert!(structure.has_composition());
    assert_eq!(structure.nodes.len(), 4);
    assert!(
        matches!(&structure.nodes[0].kind, RootNodeKind::Tabs { children } if children.len() == 1)
    );
    assert!(
        structure
            .nodes
            .iter()
            .any(|row| matches!(row.kind, RootNodeKind::Pane { item: D }))
    );
    workspace.validate().unwrap();
}

#[test]
fn content_activation_selects_every_real_ancestor_once_without_changing_layout_identity() {
    let mut workspace = vertical_layout().into_workspace();
    let (outer, tools, split) = outer_and_tools(&workspace);
    let structure = Arc::clone(workspace.root_structure(ROOT).unwrap());
    let before = workspace.root_reference(ROOT).unwrap().clone();
    let report = activate(&mut workspace, B);
    assert_eq!(
        report.outcomes(),
        [CommandOutcome::ItemActivated {
            item: B,
            changed: true
        }]
    );
    assert_ne!(workspace.root_reference(ROOT), Some(&before));
    assert_eq!(workspace.visible_tab_selection(outer), Some(tools));
    assert_eq!(workspace.first_active_item(split), Some(B));
    assert!(workspace.item_is_active(B));
    assert!(workspace.item_is_active(D));
    assert!(!workspace.item_is_active(A));
    assert!(!workspace.item_is_active(C));
    let view = crate::model::DockspaceView::new(&workspace);
    assert_eq!(
        view.surface(SURFACE)
            .unwrap()
            .active_items()
            .collect::<Vec<_>>(),
        [B, D]
    );
    assert_eq!(
        view.root(ROOT).unwrap().active_items().collect::<Vec<_>>(),
        [B, D]
    );
    assert_eq!(
        workspace.tab_label(outer, tools),
        Some(TabLabel::Layout { title: "Tools" })
    );
    assert!(Arc::ptr_eq(
        &structure,
        workspace.root_structure(ROOT).unwrap()
    ));
    activate(&mut workspace, C);
    assert!(!workspace.item_is_active(B));
    assert!(!workspace.item_is_active(D));
    assert_eq!(outer_and_tools(&workspace).1, tools);
}

#[test]
fn selecting_hidden_descendant_is_atomic_and_never_shows_it() {
    let mut workspace = vertical_layout()
        .with_hidden_items([B])
        .unwrap()
        .into_workspace();
    let root = crate::model::DockspaceView::new(&workspace)
        .root(ROOT)
        .unwrap();
    assert_eq!(root.items().collect::<Vec<_>>(), [C, A, B, D]);
    assert_eq!(
        root.content().unwrap().items().collect::<Vec<_>>(),
        [C, A, B, D]
    );
    let before = workspace.clone();
    let source = workspace.capture_item_source_by_id(B).unwrap().unwrap();
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::SelectItem { source }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Command {
            source: CommandError::ItemHidden { item: B },
            ..
        }
    ));
    assert_eq!(workspace, before);
    assert!(workspace.item_is_hidden(B));
    assert!(workspace.item_is_active(C));
}

#[test]
fn bare_root_pane_has_no_tabs_and_repeated_activation_has_no_graph_change() {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::pane(A).named("Ignored")),
    )])
    .unwrap();
    let root = layout.view().root(ROOT).unwrap();
    assert_eq!(root.items().collect::<Vec<_>>(), [A]);
    assert_eq!(root.content().unwrap().items().collect::<Vec<_>>(), [A]);
    let mut workspace = layout.into_workspace();
    let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
    assert_eq!(source.tabs(), None);
    assert_eq!(source.tab_index(), None);
    assert_eq!(source.as_tab_source(), None);
    let before = workspace.root_reference(ROOT).unwrap().clone();
    let report = activate(&mut workspace, A);
    assert_eq!(
        report.outcomes(),
        [CommandOutcome::ItemActivated {
            item: A,
            changed: false
        }]
    );
    assert_eq!(workspace.root_reference(ROOT), Some(&before));
    assert!(workspace.item_is_active(A));
    assert_eq!(
        workspace.node(source.owner_node()).unwrap().layout_title(),
        None
    );
}

#[test]
fn layout_child_replacement_repairs_selected_and_mru_at_the_original_roster_index() {
    let mut builder = Workspace::builder();
    let first = builder.insert_node(Node::tabs([A]));
    let second = builder.insert_node(Node::tabs([B]));
    let replacement = builder.insert_node(Node::tabs([D]));
    let old = TabKey::for_layout(first);
    let other = TabKey::for_layout(second);
    let mut node = Node::tabs_from_keys(vec![TabKey::for_item(C), old, other], Some(old));
    assert_eq!(
        node.child_nodes().collect::<Vec<_>>(),
        [(1, first), (2, second)]
    );
    assert_eq!(node.replace_child(1, replacement), Ok(first));
    let Node::Tabs { selected, mru, .. } = &node else {
        unreachable!()
    };
    assert_eq!(*selected, Some(TabKey::for_layout(replacement)));
    assert_eq!(mru[0], TabKey::for_layout(replacement));
    assert!(!mru.contains(&old));
    let before = node.clone();
    assert!(node.replace_child(0, first).is_err());
    assert_eq!(node, before);
}

fn two_roots() -> (Workspace, NodeId, NodeId, RootId) {
    let destination = RootId::new(2);
    let mut builder = Workspace::builder();
    let source = builder.insert_node(Node::tabs([A, B]));
    let target = builder.insert_node(Node::tabs([C]));
    builder.set_root(ROOT, RootRecord::new(source));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_root(destination, RootRecord::new(target));
    builder.set_surface(
        SurfaceId::new(2),
        SurfacePresentation::with_main(destination),
    );
    (builder.build().unwrap(), source, target, destination)
}

#[test]
fn subtree_center_preserves_a_tabs_boundary_while_explicit_tabs_payload_merges_direct_children() {
    for merge in [false, true] {
        let (mut workspace, source, target, destination) = two_roots();
        let node = workspace.capture_node_source(ROOT, source).unwrap();
        let payload = if merge {
            MovePayload::Tabs(node)
        } else {
            MovePayload::Subtree(node)
        };
        let target_ref = workspace.capture_tab_target(destination, target).unwrap();
        WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
            payload,
            target: DockTarget::Center(target_ref),
        }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap();
        let Node::Tabs { items, .. } = workspace.node(target).unwrap() else {
            unreachable!()
        };
        if merge {
            assert_eq!(items, &[C, A, B].map(TabKey::for_item));
            assert_eq!(workspace.nodes().count(), 1);
        } else {
            assert_eq!(items, &[TabKey::for_item(C), TabKey::for_layout(source)]);
            assert_eq!(workspace.nodes().count(), 2);
            assert_eq!(workspace.parent_link(destination, source).unwrap().index, 1);
        }
        assert!(workspace.root(ROOT).is_none());
        assert!(workspace.surface(SURFACE).is_none());
        assert_eq!(workspace.collect_items_in_subtree(target).len(), 3);
        workspace.validate().unwrap();
    }
}

#[test]
fn bare_pane_center_and_before_after_gaps_create_real_tabs_in_one_graph_owner() {
    for index in [None, Some(0), Some(1)] {
        let mut builder = Workspace::builder();
        let pane = builder.insert_node(Node::pane(A));
        builder.set_root(ROOT, RootRecord::new(pane));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let mut workspace = builder.build().unwrap();
        let reference = workspace.capture_tab_target(ROOT, pane).unwrap();
        let target = match index {
            None => DockTarget::Center(reference),
            Some(index) => DockTarget::TabGap {
                target: reference,
                index,
            },
        };
        WorkspaceTransaction::from_commands([WorkspaceCommand::Open { item: B, target }])
            .apply(&mut workspace, &DockPolicySnapshot::default())
            .unwrap();
        let expected = if index == Some(0) { [B, A] } else { [A, B] };
        let Node::Tabs { items, .. } = workspace.node(pane).unwrap() else {
            unreachable!()
        };
        assert_eq!(items, &expected.map(TabKey::for_item));
        assert_eq!(workspace.nodes().count(), 1);
        workspace.validate().unwrap();
    }
}

#[test]
fn layout_tab_close_covers_inactive_and_hidden_real_descendants_and_keeps_sibling_content() {
    let workspace = vertical_layout()
        .with_hidden_items([B, D])
        .unwrap()
        .into_workspace();
    let (tabs, tab, node) = outer_and_tools(&workspace);
    let policy = DockPolicySnapshot::default();
    let items = workspace.collect_items_in_subtree(node);
    assert_eq!(
        items.iter().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([A, B, D])
    );
    let requirements = items
        .iter()
        .map(|item| CloseItemRequirement::new(*item, policy.pane_close_capability(*item)))
        .collect();
    let scope = ContentCloseScope::Tab {
        root: ROOT,
        tabs,
        tab,
        node,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: items.iter().copied().collect(),
        requirements,
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabClosed {
            root: ROOT,
            tab,
            items
        }
    );
    assert_eq!(
        commit.candidate.item_multiset(),
        std::collections::BTreeMap::from([(C, 1)])
    );
    assert_eq!(commit.candidate.nodes().count(), 1);
    assert_eq!(commit.candidate.hidden_items().count(), 0);
    assert!(commit.candidate.item_is_active(C));
    commit.candidate.validate().unwrap();
}

#[test]
fn layout_title_rename_does_not_invalidate_a_content_close_scope() {
    let workspace = vertical_layout().into_workspace();
    let (tabs, tab, node) = outer_and_tools(&workspace);
    let policy = DockPolicySnapshot::default();
    let items = workspace.collect_items_in_subtree(node);
    let requirements = items
        .iter()
        .map(|item| CloseItemRequirement::new(*item, policy.pane_close_capability(*item)))
        .collect();
    let scope = ContentCloseScope::Tab {
        root: ROOT,
        tabs,
        tab,
        node,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: items.into_iter().collect(),
        requirements,
    };
    let mut builder = workspace.into_builder();
    builder
        .draft_mut()
        .nodes
        .get_mut(node)
        .unwrap()
        .set_layout_title(Some(Arc::from("Renamed")));
    let renamed = builder.build().unwrap();
    assert!(scope.matches(&renamed, &policy));
    assert_eq!(
        renamed.tab_label(tabs, tab),
        Some(TabLabel::Layout { title: "Renamed" })
    );
}

#[cfg(feature = "serde")]
#[test]
fn strict_v1_snapshot_round_trips_real_panes_layout_edges_and_titles() {
    use crate::persistence::snapshot::{SnapshotNode, WorkspaceSnapshot};
    let workspace = vertical_layout()
        .with_hidden_items([B])
        .unwrap()
        .into_workspace();
    let snapshot = WorkspaceSnapshot::capture(&workspace).unwrap();
    assert!(
        snapshot
            .nodes
            .iter()
            .any(|record| matches!(record.node, SnapshotNode::Pane { item: 4 }))
    );
    let json = serde_json::to_value(&snapshot).unwrap();
    let decoded: WorkspaceSnapshot = serde_json::from_value(json).unwrap();
    let restored = decoded.build_candidate(|_| true).unwrap();
    assert_eq!(WorkspaceSnapshot::capture(&restored).unwrap(), snapshot);
    let (tabs, tab, _) = outer_and_tools(&restored);
    assert_eq!(
        restored.tab_label(tabs, tab),
        Some(TabLabel::Layout { title: "Tools" })
    );
    assert!(
        restored
            .capture_item_source_by_id(D)
            .unwrap()
            .unwrap()
            .tabs()
            .is_none()
    );
}

#[test]
fn showing_inactive_content_preserves_outer_choice_until_explicit_activation() {
    let mut workspace = vertical_layout()
        .with_hidden_items([A, B, D])
        .unwrap()
        .into_workspace();
    let (outer, tools, _) = outer_and_tools(&workspace);
    assert_eq!(
        workspace.visible_tab_keys(outer).as_ref(),
        [TabKey::for_item(C)]
    );
    let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::SetItemVisibility {
        source,
        visible: true,
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    assert_eq!(
        workspace.visible_tab_keys(outer).as_ref(),
        [TabKey::for_item(C), tools]
    );
    assert!(workspace.item_is_active(C));
    assert!(!workspace.item_is_active(A));
    activate(&mut workspace, A);
    assert!(workspace.item_is_active(A));
    let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
    WorkspaceTransaction::from_commands([WorkspaceCommand::SetItemVisibility {
        source,
        visible: false,
    }])
    .apply(&mut workspace, &DockPolicySnapshot::default())
    .unwrap();
    assert!(workspace.item_is_active(C));
    assert_eq!(
        workspace.visible_tab_selection(outer),
        Some(TabKey::for_item(C))
    );
}

#[test]
fn extracting_real_content_keeps_bare_and_inline_source_structure_distinct() {
    use crate::graph::command::{DockFraction, Edge, RootContent};
    let mut policy = crate::policy::DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    let policy = policy.snapshot(crate::policy::PolicyRevision::default());
    for bare in [false, true] {
        for edge in [false, true] {
            let mut builder = Workspace::builder();
            let source = builder.insert_node(if bare { Node::pane(A) } else { Node::tabs([A]) });
            let remainder = builder.insert_node(Node::tabs([D]));
            let source_root = builder.insert_node(
                Node::equal_split(crate::graph::Axis::Horizontal, [source, remainder]).unwrap(),
            );
            builder.set_root(ROOT, RootRecord::new(source_root));
            builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
            let target_root = RootId::new(2);
            let target = builder.insert_node(Node::tabs([C]));
            builder.set_root(target_root, RootRecord::new(target));
            builder.set_surface(
                SurfaceId::new(2),
                SurfacePresentation::with_main(target_root),
            );
            let mut workspace = builder.build().unwrap();
            let payload =
                MovePayload::Item(workspace.capture_item_source_by_id(A).unwrap().unwrap());
            let command = if edge {
                WorkspaceCommand::Move {
                    payload,
                    target: DockTarget::InnerEdge(
                        workspace
                            .capture_inner_edge_target(
                                target_root,
                                target,
                                Edge::Left,
                                DockFraction::new(0.5).unwrap(),
                            )
                            .unwrap(),
                    ),
                }
            } else {
                WorkspaceCommand::CreateSurfaceRoot {
                    surface: SurfaceId::new(3),
                    root: RootId::new(3),
                    content: RootContent::Move(payload),
                }
            };
            WorkspaceTransaction::from_commands([command])
                .apply(&mut workspace, &policy)
                .unwrap();
            let actual = workspace.locate_item_by_id(A).unwrap();
            assert_eq!(actual.tabs().is_none(), bare);
            assert!(matches!(workspace.node(actual.owner_node()), Some(Node::Pane { .. })) == bare);
            assert_eq!(
                workspace
                    .nodes()
                    .filter(|(_, node)| matches!(node, Node::Tabs { .. }))
                    .count(),
                if bare { 2 } else { 3 }
            );
            assert_eq!(workspace.root(ROOT).unwrap().node, remainder);
            assert_eq!(workspace.locate_item_by_id(D).unwrap().root, ROOT);
            workspace.validate().unwrap();
        }
    }
}

#[test]
fn complete_bare_and_inline_roots_require_rehome_and_keep_their_root_identity() {
    use crate::graph::command::{RootContent, RootPresentationTarget};
    let mut policy = crate::policy::DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    let policy = policy.snapshot(crate::policy::PolicyRevision::default());
    for bare in [false, true] {
        let mut builder = Workspace::builder();
        let node = builder.insert_node(if bare { Node::pane(A) } else { Node::tabs([A]) });
        builder.set_root(ROOT, RootRecord::new(node));
        builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
        let mut workspace = builder.build().unwrap();
        let before = workspace.clone();
        let source = workspace.capture_item_source_by_id(A).unwrap().unwrap();
        let error = WorkspaceTransaction::from_commands([WorkspaceCommand::CreateSurfaceRoot {
            surface: SurfaceId::new(2),
            root: RootId::new(2),
            content: RootContent::Move(MovePayload::Item(source)),
        }])
        .apply(&mut workspace, &policy)
        .unwrap_err();
        assert_eq!(
            error,
            TransactionError::Command {
                index: 0,
                source: CommandError::WholeRootRequiresRehome { root: ROOT }
            }
        );
        assert_eq!(workspace, before);
        let source = workspace.capture_node_source(ROOT, node).unwrap();
        WorkspaceTransaction::from_commands([WorkspaceCommand::RehomeRoot {
            source,
            target: RootPresentationTarget::NewSurface {
                surface: SurfaceId::new(2),
            },
        }])
        .apply(&mut workspace, &policy)
        .unwrap();
        assert_eq!(workspace.root(ROOT), before.root(ROOT));
        assert_eq!(workspace.node(node), before.node(node));
        assert_eq!(
            workspace.surface(SurfaceId::new(2)).unwrap().main_root,
            Some(ROOT)
        );
        assert_eq!(workspace.locate_item_by_id(A).unwrap().root, ROOT);
        assert!(workspace.root(RootId::new(2)).is_none());
        workspace.validate().unwrap();
    }
}

#[test]
fn closing_bare_root_pane_uses_its_real_owner_and_retires_the_empty_surface() {
    let mut builder = Workspace::builder();
    let node = builder.insert_node(Node::pane(A));
    builder.set_root(ROOT, RootRecord::new(node));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    let workspace = builder.build().unwrap();
    let policy = DockPolicySnapshot::default();
    let scope = ContentCloseScope::Item {
        root: ROOT,
        owner_node: node,
        item: A,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        requirement: CloseItemRequirement::new(A, policy.pane_close_capability(A)),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::ItemClosed {
            item: A,
            root: ROOT
        }
    );
    assert_eq!(commit.candidate, Workspace::new());
}

#[test]
fn empty_title_uses_actual_container_kind_and_never_a_descendant_application_title() {
    let workspace = vertical_layout().into_workspace();
    let (tabs, tab, node) = outer_and_tools(&workspace);
    let mut builder = workspace.into_builder();
    builder
        .draft_mut()
        .nodes
        .get_mut(node)
        .unwrap()
        .set_layout_title(Some(Arc::from("")));
    let workspace = builder.build().unwrap();
    assert_eq!(
        workspace.tab_label(tabs, tab),
        Some(TabLabel::Layout { title: "Split" })
    );
    assert_eq!(workspace.tab_label(tabs, TabKey::for_item(A)), None);
}

#[test]
fn flat_content_activation_keeps_selection_cost_local_and_reuses_unchanged_snapshots() {
    use crate::graph::operation::WorkspaceSnapshot;
    use crate::interaction::targeting::resolver::structural_work;

    let (workspace, _, _, _) = two_roots();
    let mut snapshot = WorkspaceSnapshot::validated(workspace).unwrap();
    for (item, changed) in [(A, false), (B, true), (B, false)] {
        let source = snapshot.capture_item_source_by_id(item).unwrap().unwrap();
        let previous = source.reference().clone();
        let owner = source.owner_node();
        let command = WorkspaceCommand::SelectItem { source };
        structural_work::reset();
        let (actual, report) = snapshot
            .prepare_transaction(&DockPolicySnapshot::default(), &[command])
            .unwrap()
            .into_snapshot_parts();
        let work = structural_work::snapshot();
        assert_eq!(
            report.outcomes(),
            [CommandOutcome::ItemActivated { item, changed }]
        );
        assert_eq!(work.item_multiset_scans, 0);
        assert_eq!(work.canonical_full_passes, 0);
        assert_eq!(work.workspace_full_validations, 0);
        assert_eq!(
            work.workspace_deep_clones.transaction_candidates.calls,
            usize::from(changed)
        );
        assert_eq!(Arc::ptr_eq(actual.as_arc(), snapshot.as_arc()), !changed);
        if changed {
            assert!(
                actual
                    .root_reference(ROOT)
                    .unwrap()
                    .follows_selection(&previous, owner)
            );
        }
        snapshot = actual;
    }
}

#[test]
fn closing_an_all_bare_split_layout_tab_removes_its_complete_subtree() {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::pane(C),
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [DockspaceNode::pane(A), DockspaceNode::pane(B)],
                )
                .unwrap()
                .named("Tools"),
            ]),
        ),
    )])
    .unwrap()
    .with_hidden_items([B])
    .unwrap();
    let workspace = layout.into_workspace();
    let (tabs, tab, node) = outer_and_tools(&workspace);
    let policy = DockPolicySnapshot::default();
    let scope = ContentCloseScope::Tab {
        root: ROOT,
        tabs,
        tab,
        node,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B]),
        requirements: [A, B]
            .into_iter()
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .collect(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabClosed {
            root: ROOT,
            tab,
            items: vec![A, B]
        }
    );
    assert_eq!(commit.candidate.collect_items_in_subtree(tabs), [C]);
    assert_eq!(commit.candidate.nodes().count(), 1);
    assert!(commit.candidate.hidden_items().next().is_none());
    commit.candidate.validate().unwrap();
    assert_eq!(workspace.collect_items_in_subtree(node), [A, B]);
}

#[test]
fn closing_a_composed_tabs_group_destroys_whole_all_bare_layout_descendants() {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::pane(C),
                DockspaceNode::composed_tabs([DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [DockspaceNode::pane(A), DockspaceNode::pane(B)],
                )
                .unwrap()])
                .named("Tools"),
            ]),
        ),
    )])
    .unwrap();
    let workspace = layout.into_workspace();
    let (outer, _, tabs) = outer_and_tools(&workspace);
    let policy = DockPolicySnapshot::default();
    let scope = ContentCloseScope::Tabs {
        root: ROOT,
        tabs,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B]),
        requirements: [A, B]
            .into_iter()
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .collect(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabsClosed {
            root: ROOT,
            items: vec![A, B]
        }
    );
    assert_eq!(commit.candidate.collect_items_in_subtree(outer), [C]);
    assert_eq!(commit.candidate.nodes().count(), 1);
    commit.candidate.validate().unwrap();
}

#[test]
fn a_layout_tab_containing_central_rejects_close_while_whole_tabs_close_keeps_the_anchor() {
    use crate::graph::operation::{
        PreparedContentClose, prepare_content_close, validate_layout_tab_close,
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::pane(C),
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [DockspaceNode::central_tabs([A]), DockspaceNode::pane(B)],
                )
                .unwrap()
                .named("Tools"),
            ]),
        ),
    )])
    .unwrap();
    let workspace = layout.into_workspace();
    let (tabs, tab, node) = outer_and_tools(&workspace);
    let central = workspace.root(ROOT).unwrap().central.unwrap();
    let source = workspace.capture_tab_source(ROOT, tabs, tab).unwrap();
    let expected = CommandError::CentralNodeWouldDetach {
        root: ROOT,
        source_node: node,
        central,
    };
    assert!(workspace.layout_tab_contains_central(ROOT, tab));
    assert_eq!(
        validate_layout_tab_close(&workspace, &source),
        Err(expected.clone())
    );
    let policy = DockPolicySnapshot::default();
    let requirements: Vec<_> = [A, B]
        .into_iter()
        .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
        .collect();
    let scope = ContentCloseScope::Tab {
        root: ROOT,
        tabs,
        tab,
        node,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B]),
        requirements: requirements.clone(),
    };
    assert!(!scope.matches(&workspace, &policy));
    let error = prepare_content_close(
        &workspace,
        &policy,
        &PreparedContentClose::Tab {
            source,
            items: vec![A, B],
            expected_items: workspace.item_multiset(),
            requirements,
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        TransactionError::Command {
            index: 0,
            source: expected
        }
    );
    let items = workspace.collect_items_in_subtree(tabs);
    let group = ContentCloseScope::Tabs {
        root: ROOT,
        tabs,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: items.iter().copied().collect(),
        requirements: items
            .iter()
            .map(|item| CloseItemRequirement::new(*item, policy.pane_close_capability(*item)))
            .collect(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &group).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabsClosed { root: ROOT, items }
    );
    assert!(commit.candidate.item_multiset().is_empty());
    assert_eq!(commit.candidate.root(ROOT).unwrap().central, Some(central));
    assert!(
        matches!(commit.candidate.node(central), Some(Node::Tabs { items, selected: None, .. }) if items.is_empty())
    );
    assert!(commit.candidate.surface(SURFACE).is_some());
    commit.candidate.validate().unwrap();
}

#[test]
fn complete_content_collection_and_close_rosters_follow_mixed_tab_visual_order() {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [DockspaceNode::pane(A), DockspaceNode::tabs([B])],
                )
                .unwrap(),
                DockspaceNode::pane(C),
                DockspaceNode::pane(D),
            ]),
        ),
    )])
    .unwrap();
    let workspace = layout.into_workspace();
    let tabs = workspace.root(ROOT).unwrap().node;
    assert_eq!(workspace.collect_items_in_subtree(tabs), [A, B, C, D]);
    let root = crate::model::DockspaceView::new(&workspace)
        .root(ROOT)
        .unwrap();
    assert_eq!(root.items().collect::<Vec<_>>(), [A, B, C, D]);
    assert_eq!(
        root.content().unwrap().items().collect::<Vec<_>>(),
        [A, B, C, D]
    );
    let policy = DockPolicySnapshot::default();
    let scope = ContentCloseScope::Tabs {
        root: ROOT,
        tabs,
        owner: workspace.presentation_for_root(ROOT).unwrap(),
        items: BTreeSet::from([A, B, C, D]),
        requirements: [A, B, C, D]
            .into_iter()
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .collect(),
    };
    let commit = prepare_scoped_content_close(&workspace, &policy, &scope).unwrap();
    assert_eq!(
        commit.outcome,
        CloseCommitOutcome::TabsClosed {
            root: ROOT,
            items: vec![A, B, C, D]
        }
    );
    assert_eq!(commit.candidate, Workspace::new());
}
