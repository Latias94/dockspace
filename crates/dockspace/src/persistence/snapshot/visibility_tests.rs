use std::cell::RefCell;
use std::collections::BTreeSet;

use super::{SnapshotNode, SnapshotRestoreError, SnapshotTabKey, WorkspaceSnapshot};
use crate::graph::validation::WorkspaceValidationError;
use crate::graph::{Axis, Node, RootRecord, SurfacePresentation, Workspace};
use crate::ids::{ItemId, RootId, SurfaceId};

fn fixture() -> Workspace {
    let mut builder = Workspace::builder();
    let left = builder.insert_node(Node::tabs((1..=3).map(ItemId::new)));
    let right = builder.insert_node(Node::tabs([ItemId::new(4)]));
    let root = builder.insert_node(Node::equal_split(Axis::Horizontal, [left, right]).unwrap());
    builder.set_root(RootId::new(1), RootRecord::new(root));
    builder.set_surface(
        SurfaceId::new(1),
        SurfacePresentation::with_main(RootId::new(1)),
    );
    builder.set_hidden_items(BTreeSet::from([ItemId::new(2), ItemId::new(4)]));
    builder.build().unwrap()
}

#[test]
fn direct_tab_snapshot_keys_are_tagged_and_reject_the_old_numeric_shape() {
    let snapshot = WorkspaceSnapshot::capture(&fixture()).unwrap();
    let encoded = serde_json::to_value(&snapshot).unwrap();
    let tabs = encoded["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["node"]["kind"] == "tabs")
        .unwrap();
    assert_eq!(
        tabs["node"]["items"][0],
        serde_json::json!({"kind": "item", "id": 1})
    );

    let mut old = encoded.clone();
    for record in old["nodes"].as_array_mut().unwrap() {
        if record["node"]["kind"] == "tabs" {
            record["node"]["items"][0] = serde_json::json!(1);
            break;
        }
    }
    assert!(serde_json::from_value::<WorkspaceSnapshot>(old).is_err());

    let mut layout = snapshot;
    layout.hidden_items.clear();
    let child = layout.nodes.last().unwrap().id;
    let record = layout
        .nodes
        .iter_mut()
        .find(|record| matches!(record.node, SnapshotNode::Tabs { .. }))
        .unwrap();
    let SnapshotNode::Tabs {
        items,
        selected,
        mru,
        ..
    } = &mut record.node
    else {
        unreachable!()
    };
    *items = vec![SnapshotTabKey::Layout(child)];
    *selected = Some(SnapshotTabKey::Layout(child));
    *mru = vec![SnapshotTabKey::Layout(child)];
    assert!(matches!(
        layout.build_candidate(|_| true),
        Err(SnapshotRestoreError::InvalidWorkspace(errors))
            if errors.errors().iter().any(|error| matches!(error, WorkspaceValidationError::Cycle { .. }))
    ));
}

#[test]
fn json_round_trip_retains_hidden_members_and_queries_the_registry_once_per_owned_item() {
    let snapshot = WorkspaceSnapshot::capture(&fixture()).unwrap();
    assert_eq!(snapshot.hidden_items, [2, 4]);
    let json = serde_json::to_vec(&snapshot).unwrap();
    let decoded: WorkspaceSnapshot = serde_json::from_slice(&json).unwrap();
    let queried = RefCell::new(Vec::new());
    let restored = decoded
        .build_candidate(|item| {
            queried.borrow_mut().push(item);
            true
        })
        .unwrap();
    assert_eq!(
        *queried.borrow(),
        (1..=4).map(ItemId::new).collect::<Vec<_>>()
    );
    assert_eq!(WorkspaceSnapshot::capture(&restored).unwrap(), snapshot);
    let tabs = restored
        .locate_item_by_id(ItemId::new(4))
        .unwrap()
        .tabs()
        .unwrap();
    assert!(restored.item_is_hidden(ItemId::new(4)));
    assert_eq!(restored.visible_tab_selection(tabs), None);
    assert!(
        matches!(restored.node(tabs), Some(Node::Tabs { selected: Some(item), .. })
        if item.item() == Some(ItemId::new(4)))
    );
}

#[test]
fn duplicate_and_unowned_hidden_ids_reject_before_application_registry_queries() {
    let snapshot = WorkspaceSnapshot::capture(&fixture()).unwrap();
    let mut duplicate = snapshot.clone();
    duplicate.hidden_items.push(2);
    assert_eq!(
        duplicate
            .build_candidate(|_| panic!("invalid visibility must fail before registry lookup")),
        Err(SnapshotRestoreError::DuplicateHiddenItem {
            item: ItemId::new(2)
        })
    );
    let mut unknown = snapshot;
    unknown.hidden_items.push(99);
    assert_eq!(
        unknown.build_candidate(|_| panic!("invalid visibility must fail before registry lookup")),
        Err(SnapshotRestoreError::UnknownHiddenItem {
            item: ItemId::new(99)
        })
    );
}

#[test]
fn hidden_selection_with_visible_siblings_is_rejected_and_all_hidden_selection_remains_legal() {
    let mut snapshot = WorkspaceSnapshot::capture(&fixture()).unwrap();
    snapshot.hidden_items.push(1);
    let error = snapshot
        .build_candidate(|_| panic!("invalid selection must fail before registry lookup"))
        .unwrap_err();
    assert!(
        matches!(error, SnapshotRestoreError::InvalidWorkspace(errors)
        if errors.errors().iter().any(|error| matches!(error,
            WorkspaceValidationError::HiddenSelection { item, .. } if *item == ItemId::new(1))))
    );
    snapshot.hidden_items.push(3);
    let restored = snapshot.build_candidate(|_| true).unwrap();
    for item in 1..=4 {
        let tabs = restored
            .locate_item_by_id(ItemId::new(item))
            .unwrap()
            .tabs()
            .unwrap();
        assert!(restored.item_is_hidden(ItemId::new(item)));
        assert_eq!(restored.visible_tab_selection(tabs), None);
    }
    assert_eq!(
        WorkspaceSnapshot::capture(&restored).unwrap().hidden_items,
        [1, 2, 3, 4]
    );
}

#[test]
fn an_omitted_hidden_set_has_the_default_all_visible_semantics() {
    let snapshot = WorkspaceSnapshot::capture(&fixture()).unwrap();
    let mut value = serde_json::to_value(snapshot).unwrap();
    value.as_object_mut().unwrap().remove("hidden_items");
    let decoded: WorkspaceSnapshot = serde_json::from_value(value).unwrap();
    assert!(decoded.hidden_items.is_empty());
    assert!(
        decoded
            .build_candidate(|_| true)
            .unwrap()
            .hidden_items()
            .next()
            .is_none()
    );
}
