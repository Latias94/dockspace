use crate::ids::TabKey;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::close::plan::CloseItemRequirement;
use crate::graph::command::{
    CommandOutcome, DockTarget, MovePayload, RootPresentationTarget, WorkspaceCommand,
};
use crate::graph::error::{CommandError, TransactionError};
use crate::graph::operation::{PreparedContentClose, WorkspaceSnapshot, prepare_content_close};
use crate::graph::transaction::WorkspaceTransaction;
use crate::graph::validation::WorkspaceValidationError;
use crate::graph::{Axis, Node, RootRecord, SurfacePresentation, Workspace};
use crate::ids::{ItemId, NodeId, RootId, SurfaceId};
use crate::policy::{DockPolicy, DockPolicySnapshot, PolicyRejection, PolicyRevision};

const ROOT: RootId = RootId::new(1);
const OTHER: RootId = RootId::new(2);

fn fixture() -> (Workspace, [NodeId; 3]) {
    let mut builder = Workspace::builder();
    let tabs = [
        builder.insert_node(Node::tabs((1..=5).map(ItemId::new))),
        builder.insert_node(Node::tabs([ItemId::new(6), ItemId::new(7)])),
        builder.insert_node(Node::tabs([])),
    ];
    let split = builder.insert_node(Node::equal_split(Axis::Horizontal, tabs).unwrap());
    let other = builder.insert_node(Node::tabs([ItemId::new(8), ItemId::new(9)]));
    builder.set_root(ROOT, RootRecord::new(split).with_central(tabs[2]));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    builder.set_root(OTHER, RootRecord::new(other));
    builder.set_surface(SurfaceId::new(2), SurfacePresentation::with_main(OTHER));
    (builder.build().unwrap(), tabs)
}

fn apply(workspace: &mut Workspace, command: WorkspaceCommand) -> CommandOutcome {
    let report = WorkspaceTransaction::from_commands([command])
        .apply(workspace, &DockPolicySnapshot::default())
        .unwrap();
    workspace.validate().unwrap();
    report.into_outcomes().remove(0)
}

fn visibility(workspace: &mut Workspace, item: u64, visible: bool) -> CommandOutcome {
    let source = workspace
        .capture_item_source_by_id(ItemId::new(item))
        .unwrap()
        .unwrap();
    apply(
        workspace,
        WorkspaceCommand::SetItemVisibility { source, visible },
    )
}

fn select(workspace: &mut Workspace, item: u64) {
    let source = workspace
        .capture_item_source_by_id(ItemId::new(item))
        .unwrap()
        .unwrap();
    apply(
        workspace,
        WorkspaceCommand::Select {
            source: source
                .as_tab_source()
                .expect("fixture owns an inline item tab"),
        },
    );
}

#[test]
fn hiding_retains_rosters_weights_and_chooses_visible_successors_without_showing_focus_theft() {
    let (mut workspace, tabs) = fixture();
    let root = *workspace.root(ROOT).unwrap();
    let split = workspace.node(root.node).unwrap().clone();
    let ownership = workspace.item_multiset();
    assert!(matches!(
        workspace.visible_tab_keys(tabs[0]),
        Cow::Borrowed(_)
    ));
    select(&mut workspace, 3);
    for item in [2, 4, 3] {
        visibility(&mut workspace, item, false);
    }
    assert_eq!(
        workspace.visible_tab_selection(tabs[0]),
        Some(TabKey::for_item(ItemId::new(5)))
    );
    visibility(&mut workspace, 5, false);
    assert_eq!(
        workspace.visible_tab_selection(tabs[0]),
        Some(TabKey::for_item(ItemId::new(1)))
    );
    visibility(&mut workspace, 1, false);
    assert_eq!(workspace.visible_tab_selection(tabs[0]), None);
    assert!(workspace.visible_tab_keys(tabs[0]).is_empty());
    assert!(!workspace.node_has_visible_content(tabs[0]));
    assert!(workspace.node_has_visible_content(tabs[2]));
    assert!(matches!(
        workspace.node(tabs[0]),
        Some(Node::Tabs {
            selected: Some(_),
            ..
        })
    ));

    visibility(&mut workspace, 4, true);
    visibility(&mut workspace, 2, true);
    assert_eq!(
        workspace.visible_tab_selection(tabs[0]),
        Some(TabKey::for_item(ItemId::new(4)))
    );
    assert_eq!(
        workspace.visible_tab_keys(tabs[0]).as_ref(),
        [
            TabKey::for_item(ItemId::new(2)),
            TabKey::for_item(ItemId::new(4))
        ]
    );
    assert_eq!(*workspace.root(ROOT).unwrap(), root);
    assert_eq!(workspace.node(root.node).unwrap(), &split);
    assert_eq!(workspace.item_multiset(), ownership);
    assert!(
        matches!(workspace.node(tabs[0]), Some(Node::Tabs { items, .. })
        if items == &(1..=5).map(ItemId::new).map(TabKey::for_item).collect::<Vec<_>>())
    );
}

#[test]
fn visibility_refreshes_root_authority_and_fingerprints_but_noop_and_peers_share_their_state() {
    let (mut workspace, tabs) = fixture();
    let original = workspace.clone();
    let before = original.capture_node_source(ROOT, tabs[0]).unwrap();
    let fingerprint = original.root_fingerprint(ROOT).unwrap();
    let structure = Arc::clone(original.root_structure(ROOT).unwrap());
    let other = original.root(OTHER).unwrap().node;
    let peer = original.capture_node_source(OTHER, other).unwrap();
    visibility(&mut workspace, 2, false);
    let after = workspace.capture_node_source(ROOT, tabs[0]).unwrap();
    let hidden_fingerprint = workspace.root_fingerprint(ROOT).unwrap();
    assert_ne!(before.reference(), after.reference());
    assert!(!fingerprint.presentation_structure_eq(&hidden_fingerprint));
    assert!(!fingerprint.presentation_topology_eq(&hidden_fingerprint));
    assert!(!Arc::ptr_eq(
        &structure,
        workspace.root_structure(ROOT).unwrap()
    ));
    assert_eq!(
        workspace
            .capture_node_source(OTHER, other)
            .unwrap()
            .reference(),
        peer.reference()
    );
    assert!(!original.item_is_hidden(ItemId::new(2)));
    assert!(std::ptr::eq(
        original.node(tabs[0]).unwrap(),
        workspace.node(tabs[0]).unwrap()
    ));
    assert_eq!(
        visibility(&mut workspace, 2, false),
        CommandOutcome::ItemVisibilityChanged {
            item: ItemId::new(2),
            root: ROOT,
            visible: false,
            changed: false,
        }
    );
    assert_eq!(
        workspace
            .capture_node_source(ROOT, tabs[0])
            .unwrap()
            .reference(),
        after.reference()
    );
}

#[test]
fn an_all_hidden_selected_item_is_rejected_even_by_the_unchanged_selection_fast_path() {
    let (mut workspace, tabs) = fixture();
    for item in 1..=5 {
        visibility(&mut workspace, item, false);
    }
    let Node::Tabs { selected, .. } = workspace.node(tabs[0]).unwrap() else {
        panic!()
    };
    let item = selected.unwrap().item().unwrap();
    let source = workspace.capture_item_source(ROOT, tabs[0], item).unwrap();
    let snapshot = WorkspaceSnapshot::from(workspace.clone());
    let error = snapshot
        .prepare_transaction(
            &DockPolicySnapshot::default(),
            &[WorkspaceCommand::Select {
                source: source
                    .as_tab_source()
                    .expect("fixture owns an inline item tab"),
            }],
        )
        .unwrap_err();
    assert!(matches!(error, TransactionError::Command {
        source: CommandError::ItemHidden { item: actual }, ..
    } if actual == item));
    assert_eq!(&*snapshot, &workspace);
}

#[test]
fn failed_batches_and_locked_visibility_leave_the_live_workspace_unchanged() {
    let (mut workspace, tabs) = fixture();
    let original = workspace.clone();
    let source = workspace
        .capture_item_source(ROOT, tabs[0], ItemId::new(2))
        .unwrap();
    let target = workspace
        .capture_tab_target(OTHER, workspace.root(OTHER).unwrap().node)
        .unwrap();
    let transaction = WorkspaceTransaction::from_commands([
        WorkspaceCommand::SetItemVisibility {
            source: source.clone(),
            visible: false,
        },
        WorkspaceCommand::Open {
            item: ItemId::new(8),
            target: DockTarget::Center(target),
        },
    ]);
    assert!(
        transaction
            .apply(&mut workspace, &DockPolicySnapshot::default())
            .is_err()
    );
    assert_eq!(workspace, original);

    let mut policy = DockPolicy::new();
    policy.set_arrangement_locked(true);
    let error = WorkspaceTransaction::from_commands([WorkspaceCommand::SetItemVisibility {
        source,
        visible: false,
    }])
    .apply(&mut workspace, &policy.snapshot(PolicyRevision::new(1)))
    .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::Command {
            source: CommandError::Policy(PolicyRejection::ArrangementLocked),
            ..
        }
    ));
    assert_eq!(workspace, original);
}

#[test]
fn moving_hidden_items_preserves_the_flag_and_keeps_both_source_and_destination_selection_visible()
{
    let (mut workspace, tabs) = fixture();
    visibility(&mut workspace, 2, false);
    let source = workspace
        .capture_item_source(ROOT, tabs[0], ItemId::new(2))
        .unwrap();
    let target = workspace.capture_tab_target(ROOT, tabs[1]).unwrap();
    apply(
        &mut workspace,
        WorkspaceCommand::Move {
            payload: MovePayload::Item(source),
            target: DockTarget::Center(target),
        },
    );
    assert!(workspace.item_is_hidden(ItemId::new(2)));
    assert_eq!(
        workspace.visible_tab_selection(tabs[1]),
        Some(TabKey::for_item(ItemId::new(6)))
    );
    assert_eq!(
        workspace.visible_tab_keys(tabs[1]).as_ref(),
        [
            TabKey::for_item(ItemId::new(6)),
            TabKey::for_item(ItemId::new(7))
        ]
    );

    visibility(&mut workspace, 3, false);
    let source = workspace
        .capture_item_source(ROOT, tabs[0], ItemId::new(1))
        .unwrap();
    let target = workspace.capture_tab_target(ROOT, tabs[1]).unwrap();
    apply(
        &mut workspace,
        WorkspaceCommand::Move {
            payload: MovePayload::Item(source),
            target: DockTarget::Center(target),
        },
    );
    assert_eq!(
        workspace.visible_tab_selection(tabs[0]),
        Some(TabKey::for_item(ItemId::new(4)))
    );
}

#[test]
fn merging_hidden_tabs_retains_the_destination_selection_and_all_hidden_members() {
    let (mut workspace, tabs) = fixture();
    for item in 1..=5 {
        visibility(&mut workspace, item, false);
    }
    let source = workspace.capture_node_source(ROOT, tabs[0]).unwrap();
    let destination = workspace.root(OTHER).unwrap().node;
    let target = workspace.capture_tab_target(OTHER, destination).unwrap();
    apply(
        &mut workspace,
        WorkspaceCommand::Move {
            payload: MovePayload::Tabs(source),
            target: DockTarget::Center(target),
        },
    );
    assert_eq!(
        workspace.visible_tab_selection(destination),
        Some(TabKey::for_item(ItemId::new(8)))
    );
    assert_eq!(
        workspace.visible_tab_keys(destination).as_ref(),
        [
            TabKey::for_item(ItemId::new(8)),
            TabKey::for_item(ItemId::new(9))
        ]
    );
    for item in 1..=5 {
        assert!(workspace.item_is_hidden(ItemId::new(item)));
        assert_eq!(
            workspace.locate_item_by_id(ItemId::new(item)).unwrap().root,
            OTHER
        );
    }
}

#[test]
fn a_hide_show_batch_refreshes_authority_even_when_durable_values_are_restored() {
    let (mut workspace, tabs) = fixture();
    let original = workspace.clone();
    let source = original
        .capture_item_source(ROOT, tabs[0], ItemId::new(2))
        .unwrap();
    let report =
        WorkspaceTransaction::apply_with(&mut workspace, &DockPolicySnapshot::default(), |stage| {
            for visible in [false, true] {
                stage.push(|capture| {
                    Ok(WorkspaceCommand::SetItemVisibility {
                        source: capture.capture_item_source(ROOT, tabs[0], ItemId::new(2))?,
                        visible,
                    })
                })?;
            }
            Ok(())
        })
        .unwrap();
    assert!(!report.changed());
    assert_eq!(workspace, original);
    assert_ne!(
        workspace
            .capture_item_source(ROOT, tabs[0], ItemId::new(2))
            .unwrap()
            .reference(),
        source.reference()
    );
    assert!(report.outcomes().iter().all(|outcome| matches!(
        outcome,
        CommandOutcome::ItemVisibilityChanged { changed: true, .. }
    )));
}

#[test]
fn whole_root_rehoming_retains_hidden_content_and_stable_identity() {
    let (mut workspace, _) = fixture();
    visibility(&mut workspace, 8, false);
    visibility(&mut workspace, 9, false);
    let node = workspace.root(OTHER).unwrap().node;
    let source = workspace.capture_node_source(OTHER, node).unwrap();
    let mut policy = DockPolicy::new();
    policy.set_allow_native_surfaces(true);
    WorkspaceTransaction::from_commands([WorkspaceCommand::RehomeRoot {
        source,
        target: RootPresentationTarget::NewSurface {
            surface: SurfaceId::new(3),
        },
    }])
    .apply(&mut workspace, &policy.snapshot(PolicyRevision::new(1)))
    .unwrap();
    workspace.validate().unwrap();
    assert_eq!(workspace.root(OTHER).unwrap().node, node);
    assert_eq!(
        workspace.surface(SurfaceId::new(3)).unwrap().main_root,
        Some(OTHER)
    );
    assert_eq!(
        workspace.hidden_items().collect::<Vec<_>>(),
        [ItemId::new(8), ItemId::new(9)]
    );
}

#[test]
fn closing_hidden_content_and_retiring_a_root_clear_only_the_closed_flags() {
    let (mut workspace, tabs) = fixture();
    for item in [2, 8, 9] {
        visibility(&mut workspace, item, false);
    }
    let policy = DockPolicySnapshot::default();
    let item = ItemId::new(2);
    let prepared = PreparedContentClose::Item {
        source: workspace.capture_item_source(ROOT, tabs[0], item).unwrap(),
        expected_items: workspace.item_multiset(),
        requirement: CloseItemRequirement::new(item, policy.pane_close_capability(item)),
    };
    workspace = prepare_content_close(&workspace, &policy, &prepared)
        .unwrap()
        .candidate;
    assert!(!workspace.item_is_hidden(item));
    assert!(workspace.locate_item_by_id(item).is_none());
    let node = workspace.root(OTHER).unwrap().node;
    let items = workspace.collect_items_in_subtree(node);
    let prepared = PreparedContentClose::Root {
        source: workspace.capture_node_source(OTHER, node).unwrap(),
        requirements: items
            .iter()
            .copied()
            .map(|item| CloseItemRequirement::new(item, policy.pane_close_capability(item)))
            .collect(),
        items,
        expected_items: workspace.item_multiset(),
    };
    workspace = prepare_content_close(&workspace, &policy, &prepared)
        .unwrap()
        .candidate;
    assert!(workspace.hidden_items().next().is_none());
    assert!(workspace.root(OTHER).is_none());
    workspace.validate().unwrap();
}

#[test]
fn builder_rejects_unowned_hidden_items_and_visible_siblings_of_a_hidden_selection() {
    let (workspace, _) = fixture();
    let mut builder = workspace.clone().into_builder();
    builder.set_hidden_items(BTreeSet::from([ItemId::new(99)]));
    assert!(builder.validate().unwrap_err().errors().iter().any(|error|
        matches!(error, WorkspaceValidationError::HiddenItemNotOwned { item } if *item == ItemId::new(99))));
    let mut builder = workspace.into_builder();
    builder.set_hidden_items(BTreeSet::from([ItemId::new(1)]));
    assert!(builder.validate().unwrap_err().errors().iter().any(|error|
        matches!(error, WorkspaceValidationError::HiddenSelection { item, .. } if *item == ItemId::new(1))));
}
