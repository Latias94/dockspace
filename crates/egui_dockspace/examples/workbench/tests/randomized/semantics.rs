//! Test-local semantic checks independent of the adapter's action reports.

use std::collections::BTreeMap;

use egui_dockspace::{
    DockEdge, DockspaceAxis, DockspaceDropAction, DockspaceTabView, FloatingPresentationId, ItemId,
    LogicalRect, NodeKey, RootId, TabKey,
};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Group {
    key: NodeKey,
    root: RootId,
    floating: Option<FloatingPresentationId>,
    title: Option<String>,
    tabs: Vec<TabKey>,
    layout_nodes: Vec<Option<NodeKey>>,
    members: Vec<Vec<ItemId>>,
    items: Vec<ItemId>,
    selected: Option<TabKey>,
}

#[derive(Debug, Clone, PartialEq)]
struct Split {
    key: NodeKey,
    root: RootId,
    floating: Option<FloatingPresentationId>,
    title: Option<String>,
    axis: DockspaceAxis,
    child_keys: Vec<NodeKey>,
    children: Vec<Vec<ItemId>>,
    weights: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LayoutState {
    groups: Vec<Group>,
    splits: Vec<Split>,
    floating: BTreeMap<FloatingPresentationId, LogicalRect>,
    hidden: Vec<ItemId>,
}

impl LayoutState {
    pub(super) fn capture(replay: &Replay) -> Self {
        let mut state = Self {
            groups: Vec::new(),
            splits: Vec::new(),
            floating: BTreeMap::new(),
            hidden: replay.dock.view().hidden_items().collect(),
        };
        for surface in replay.dock.view().surfaces() {
            let mut roots = surface
                .main_root()
                .into_iter()
                .map(|root| (root, None))
                .collect::<Vec<_>>();
            for floating in surface.contained() {
                state.floating.insert(floating.id(), floating.rect());
                roots.push((floating.root().unwrap(), Some(floating.id())));
            }
            for (root, floating) in roots {
                let mut pending = vec![root.content().unwrap()];
                while let Some(node) = pending.pop() {
                    if let Some(tabs) = node.tabs() {
                        state.groups.push(Group {
                            key: tabs.key(),
                            root: root.id(),
                            floating,
                            title: node.layout_title().map(str::to_owned),
                            tabs: tabs.tabs().to_vec(),
                            layout_nodes: tabs
                                .children()
                                .map(|child| match child {
                                    DockspaceTabView::Item(_) => None,
                                    DockspaceTabView::Layout(node) => Some(node.key()),
                                })
                                .collect(),
                            members: tabs
                                .children()
                                .map(|child| match child {
                                    DockspaceTabView::Item(item) => vec![item],
                                    DockspaceTabView::Layout(node) => node.items().collect(),
                                })
                                .collect(),
                            items: tabs.items().collect::<Vec<_>>(),
                            selected: tabs.selected(),
                        });
                        pending.extend(tabs.children().rev().filter_map(|child| match child {
                            DockspaceTabView::Layout(node) => Some(node),
                            DockspaceTabView::Item(_) => None,
                        }));
                    } else if let Some(split) = node.split() {
                        let children = split.children().collect::<Vec<_>>();
                        state.splits.push(Split {
                            key: node.key(),
                            root: root.id(),
                            floating,
                            title: node.layout_title().map(str::to_owned),
                            axis: split.axis(),
                            child_keys: children.iter().map(|child| child.key()).collect(),
                            children: children.iter().copied().map(descendants).collect(),
                            weights: split.weights().collect(),
                        });
                        pending.extend(children.into_iter().rev());
                    } else if let Some(grid) = node.grid() {
                        pending.extend(grid.children().rev());
                    }
                }
            }
        }
        state.groups.sort_by(|left, right| {
            (&left.items, &left.members).cmp(&(&right.items, &right.members))
        });
        state
    }

    pub(super) fn assert_same_content(&self, after: &Self) {
        assert_eq!(after.groups.len(), self.groups.len());
        for (actual, expected) in after.groups.iter().zip(&self.groups) {
            // Rebuilding the same public layout allocates fresh node keys.
            // Compare every direct member's real content and selected position.
            let semantic_group = |group: &Group| {
                (
                    group.root,
                    group.floating,
                    group.title.clone(),
                    group.items.clone(),
                    group.members.clone(),
                    group
                        .tabs
                        .iter()
                        .position(|tab| Some(*tab) == group.selected),
                )
            };
            assert_eq!(
                semantic_group(actual),
                semantic_group(expected),
                "resizing or reconstruction must preserve every group and selection"
            );
        }
        assert_eq!(
            after.hidden, self.hidden,
            "resizing must preserve visibility"
        );
    }

    pub(super) fn assert_equivalent_geometry(&self, after: &Self) {
        self.assert_same_content(after);
        assert_eq!(after.splits.len(), self.splits.len());
        for (actual, expected) in after.splits.iter().zip(&self.splits) {
            assert_eq!(actual.axis, expected.axis);
            assert_eq!(actual.children, expected.children);
            assert_eq!(actual.weights.len(), expected.weights.len());
            for (actual_share, expected_share) in actual.weights.iter().zip(&expected.weights) {
                assert!(
                    (actual_share - expected_share).abs() < 1e-8,
                    "split share {actual_share} != {expected_share}; axis={:?}, children={:?}",
                    actual.axis,
                    actual.children,
                );
            }
        }
        assert_eq!(
            after.floating.keys().collect::<Vec<_>>(),
            self.floating.keys().collect::<Vec<_>>()
        );
        for (id, actual) in &after.floating {
            let expected = self.floating[id];
            for (actual, expected) in [actual.x(), actual.y(), actual.width(), actual.height()]
                .into_iter()
                .zip([
                    expected.x(),
                    expected.y(),
                    expected.width(),
                    expected.height(),
                ])
            {
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "floating coordinate {actual} != {expected}"
                );
            }
        }
    }
}

fn descendants(node: egui_dockspace::DockspaceNodeView<'_>) -> Vec<ItemId> {
    node.items().collect()
}

pub(super) struct MoveExpectation {
    before: LayoutState,
    source: Group,
    payload: Vec<ItemId>,
    selected: ItemId,
}

impl MoveExpectation {
    pub(super) fn capture(replay: &Replay, item: ItemId) -> Self {
        let before = LayoutState::capture(replay);
        let source = before
            .groups
            .iter()
            .find(|group| group.items.contains(&item))
            .unwrap()
            .clone();
        let (payload, selected) = if source.floating.is_some() {
            assert_eq!(
                before
                    .groups
                    .iter()
                    .filter(|group| group.root == source.root)
                    .count(),
                1,
                "this journey's floating title fixture contains one tabs group"
            );
            (
                source.items.clone(),
                source.selected.and_then(TabKey::item).unwrap(),
            )
        } else {
            (vec![item], item)
        };
        Self {
            before,
            source,
            payload,
            selected,
        }
    }

    pub(super) fn assert_docked(
        &self,
        replay: &Replay,
        target: ItemId,
        action: DockspaceDropAction,
    ) {
        let merged = action == DockspaceDropAction::Merge;
        let destination = self
            .before
            .groups
            .iter()
            .find(|group| group.items.contains(&target))
            .unwrap();
        assert_ne!(destination.items, self.source.items);
        let after = LayoutState::capture(replay);
        let moved = replay.dock.view().item(self.selected).unwrap();
        assert!(
            moved.contained().is_none(),
            "the admitted drop must return its payload to the main root"
        );
        assert_eq!(moved.root(), destination.root);
        let preserves_layout = merged && self.source.floating.is_some();
        let mut expected = destination.items.clone();
        if preserves_layout {
            expected.clone_from(&self.payload);
            let target_tabs = replay.dock.view().item(target).unwrap().tabs().unwrap();
            assert_eq!(
                &target_tabs.tabs()[..destination.tabs.len()],
                destination.tabs.as_slice()
            );
            assert_eq!(target_tabs.tabs().len(), destination.tabs.len() + 1);
            let layout = *target_tabs.tabs().last().unwrap();
            let Some(DockspaceTabView::Layout(layout_view)) = target_tabs.child(layout) else {
                panic!("a center drop from a floating title preserves the complete root layout");
            };
            assert_eq!(layout_view.items().collect::<Vec<_>>(), self.payload);
            assert_eq!(
                layout_view.tabs().unwrap().key(),
                moved.tabs().unwrap().key()
            );
            assert_eq!(layout_view.key(), self.source.key);
            assert_eq!(layout_view.layout_title(), self.source.title.as_deref());
            assert_eq!(target_tabs.selected(), Some(layout));
        } else if merged {
            expected.extend_from_slice(&self.payload);
            let mut expected_tabs = destination
                .tabs
                .iter()
                .copied()
                .zip(&destination.members)
                .filter_map(|(tab, items)| {
                    (!items.iter().all(|item| self.payload.contains(item))).then_some(tab)
                })
                .collect::<Vec<_>>();
            expected_tabs.extend(self.payload.iter().copied().map(TabKey::from));
            assert_eq!(
                moved.tabs().unwrap().tabs(),
                expected_tabs,
                "a pane merge preserves every unrelated layout tab in durable order"
            );
            assert_eq!(moved.tabs().unwrap().key(), destination.key);
            assert_eq!(
                replay
                    .dock
                    .view()
                    .node(destination.key)
                    .unwrap()
                    .layout_title(),
                destination.title.as_deref(),
            );
        } else {
            expected.clone_from(&self.payload);
            assert!(
                after.groups.contains(destination),
                "an edge drop preserves the target group"
            );
        }
        assert_eq!(
            moved
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            expected,
            "an admitted drop must move exactly its payload in order"
        );
        assert_eq!(
            moved.tabs().expect("item is an inline tab").selected(),
            Some(self.selected.into()),
            "the destination must activate the moved selection"
        );
        self.assert_remainder_and_unaffected(replay, &after, Some(destination), Some(action));
    }

    pub(super) fn assert_floated(&self, replay: &Replay) {
        let after = LayoutState::capture(replay);
        let moved = replay.dock.view().item(self.selected).unwrap();
        assert!(
            moved.contained().is_some(),
            "an admitted Shift-drag must float its payload"
        );
        assert_eq!(
            moved
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            self.payload
        );
        assert_eq!(
            moved.tabs().expect("item is an inline tab").selected(),
            Some(self.selected.into())
        );
        self.assert_remainder_and_unaffected(replay, &after, None, None);
    }

    fn assert_remainder_and_unaffected(
        &self,
        replay: &Replay,
        after: &LayoutState,
        destination: Option<&Group>,
        action: Option<DockspaceDropAction>,
    ) {
        assert_eq!(
            after.hidden, self.before.hidden,
            "moving content must preserve visibility"
        );
        let remainder = self
            .source
            .items
            .iter()
            .copied()
            .filter(|item| !self.payload.contains(item))
            .collect::<Vec<_>>();
        if let Some(&item) = remainder.first() {
            let source = replay.dock.view().item(item).unwrap();
            assert_eq!(
                source
                    .tabs()
                    .expect("item is an inline tab")
                    .items()
                    .collect::<Vec<_>>(),
                remainder,
                "the donor preserves the remaining tab order"
            );
            assert_eq!(source.root(), self.source.root);
            assert_eq!(source.contained(), self.source.floating);
            if self.source.selected.is_some_and(|selected| {
                selected
                    .item()
                    .is_none_or(|item| !self.payload.contains(&item))
            }) {
                assert_eq!(
                    source.tabs().expect("item is an inline tab").selected(),
                    self.source.selected,
                    "pressing then removing another tab restores the previous selection"
                );
            }
        } else if let Some(floating) = self.source.floating {
            assert!(
                replay.dock.view().contained(floating).is_none(),
                "an emptied floating source must retire"
            );
        }
        if self.source.floating.is_none() {
            let retained_tabs = self
                .source
                .tabs
                .iter()
                .copied()
                .filter(|tab| tab.item().is_none_or(|item| !self.payload.contains(&item)))
                .collect::<Vec<_>>();
            if !retained_tabs.is_empty() {
                let donor = after
                    .groups
                    .iter()
                    .find(|group| group.key == self.source.key)
                    .expect("a donor with retained content or layout tabs must survive");
                assert_eq!(donor.tabs, retained_tabs);
                assert_eq!(donor.items, remainder);
                assert_eq!(donor.root, self.source.root);
                assert_eq!(donor.floating, self.source.floating);
                if self
                    .source
                    .selected
                    .is_some_and(|tab| donor.tabs.contains(&tab))
                {
                    assert_eq!(donor.selected, self.source.selected);
                }
            }
        }
        let mut removed_ancestors = Vec::new();
        for group in &self.before.groups {
            if group == &self.source || destination.is_some_and(|target| group == target) {
                continue;
            }
            let expected_members = group
                .tabs
                .iter()
                .copied()
                .zip(&group.members)
                .filter(|(_, items)| {
                    !items.iter().all(|item| self.payload.contains(item))
                        || destination.is_some_and(|target| {
                            target.items.iter().any(|item| items.contains(item))
                        })
                })
                .collect::<Vec<_>>();
            let actual = after.groups.iter().find(|actual| actual.key == group.key);
            if expected_members.is_empty() {
                assert!(actual.is_none(), "an emptied ancestor group must retire");
                removed_ancestors.push(group.key);
                continue;
            }
            let actual = actual.unwrap_or_else(|| panic!("unrelated group vanished: {group:?}"));
            assert_eq!(actual.root, group.root);
            assert_eq!(actual.floating, group.floating);
            assert_eq!(actual.title, group.title);
            assert_eq!(actual.items, group.items);
            assert_eq!(
                actual.tabs.len(),
                expected_members.len(),
                "only emptied layout members may disappear from an ancestor",
            );
            let mut selected = None;
            for (index, (original_tab, _)) in expected_members.iter().enumerate() {
                let actual_tab = actual.tabs[index];
                if actual_tab != *original_tab {
                    let Some(DockspaceDropAction::Split { edge }) = action else {
                        panic!("only an edge drop may wrap an ancestor's retained layout member");
                    };
                    let target = destination.expect("an edge drop has an exact target group");
                    let original_index = group
                        .tabs
                        .iter()
                        .position(|tab| tab == original_tab)
                        .unwrap();
                    assert_eq!(
                        group.layout_nodes[original_index],
                        Some(target.key),
                        "only the edge target itself may be replaced by a split wrapper",
                    );
                    self.assert_target_wrapper(replay, group.key, actual_tab, target, edge);
                }
                if group.selected == Some(*original_tab) {
                    selected = Some(actual_tab);
                }
            }
            if let Some(selected) = selected {
                assert_eq!(
                    actual.selected,
                    Some(selected),
                    "a wrapped selected layout stays selected at the same surviving tab position",
                );
            } else {
                assert!(
                    actual
                        .selected
                        .is_some_and(|tab| actual.tabs.contains(&tab))
                );
            }
            for (actual_items, (_, original_items)) in actual.members.iter().zip(expected_members) {
                let retains_payload = destination.is_some_and(|target| {
                    target
                        .items
                        .iter()
                        .any(|item| original_items.contains(item))
                });
                let without_payload = |items: &[ItemId]| {
                    items
                        .iter()
                        .copied()
                        .filter(|item| !self.payload.contains(item))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    without_payload(actual_items),
                    without_payload(original_items),
                    "moving a descendant preserves all other ancestor content in order"
                );
                assert_eq!(
                    actual_items
                        .iter()
                        .copied()
                        .filter(|item| self.payload.contains(item))
                        .collect::<Vec<_>>(),
                    if retains_payload {
                        self.payload.clone()
                    } else {
                        Vec::new()
                    },
                    "ancestor membership must follow the exact payload destination"
                );
            }
        }
        let merged = action == Some(DockspaceDropAction::Merge);
        let preserves_layout = merged && self.source.floating.is_some();
        let source_emptied = self
            .source
            .tabs
            .iter()
            .all(|tab| tab.item().is_some_and(|item| self.payload.contains(&item)));
        let mut expected_groups = self
            .before
            .groups
            .iter()
            .filter(|group| {
                !removed_ancestors.contains(&group.key)
                    && !(source_emptied && !preserves_layout && group.key == self.source.key)
            })
            .map(|group| group.key)
            .collect::<Vec<_>>();
        if !merged {
            let payload_group = replay
                .dock
                .view()
                .item(self.selected)
                .unwrap()
                .tabs()
                .unwrap()
                .key();
            assert!(
                !self
                    .before
                    .groups
                    .iter()
                    .any(|group| group.key == payload_group),
                "an inline pane edge/float move allocates its own payload group",
            );
            expected_groups.push(payload_group);
        }
        expected_groups.extend(self.assert_retained_layout_boundaries(
            replay,
            after,
            destination,
            action,
        ));
        expected_groups.sort_unstable();
        let mut actual_groups = after
            .groups
            .iter()
            .map(|group| group.key)
            .collect::<Vec<_>>();
        actual_groups.sort_unstable();
        assert_eq!(
            actual_groups, expected_groups,
            "only the exact payload group, retired ancestors and retained layout boundaries may change the group inventory",
        );
        for (id, rect) in &self.before.floating {
            if Some(*id) != self.source.floating {
                assert_eq!(
                    after.floating.get(id),
                    Some(rect),
                    "unrelated floating geometry changed"
                );
            }
        }
    }

    fn assert_retained_layout_boundaries(
        &self,
        replay: &Replay,
        after: &LayoutState,
        destination: Option<&Group>,
        action: Option<DockspaceDropAction>,
    ) -> Vec<NodeKey> {
        let mut converted = Vec::new();
        for split in &self.before.splits {
            let boundary_parent = self
                .before
                .groups
                .iter()
                .find(|group| group.layout_nodes.contains(&Some(split.key)));
            if boundary_parent.is_none() {
                continue;
            }
            let surviving = split
                .children
                .iter()
                .enumerate()
                .filter(|(_, items)| {
                    items.iter().any(|item| !self.payload.contains(item))
                        || destination.is_some_and(|target| {
                            target.items.iter().any(|item| items.contains(item))
                        })
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let [survivor] = surviving.as_slice() else {
                continue;
            };
            // A same-axis edge inserts another direct split child, so removing
            // its donor does not collapse this boundary to one remaining branch.
            if let Some(DockspaceDropAction::Split { edge }) = action
                && destination.is_some_and(|target| split.child_keys.contains(&target.key))
                && matches!(
                    (split.axis, edge),
                    (DockspaceAxis::Horizontal, DockEdge::Left | DockEdge::Right)
                        | (DockspaceAxis::Vertical, DockEdge::Top | DockEdge::Bottom)
                )
            {
                continue;
            }
            let group = after
                .groups
                .iter()
                .find(|group| group.key == split.key)
                .expect("a one-branch layout-tab split retains its identity as a Tabs boundary");
            assert_eq!(group.root, split.root);
            assert_eq!(group.floating, split.floating);
            assert_eq!(group.title, split.title);
            assert_eq!(group.tabs.len(), 1);
            assert_eq!(group.selected, group.tabs.first().copied());
            let retained = replay.dock.view().node(split.key).unwrap();
            let Some(DockspaceTabView::Layout(child)) =
                retained.tabs().unwrap().child(group.tabs[0])
            else {
                panic!("this workbench boundary retains its original structural branch");
            };
            assert_eq!(child.root().unwrap().id(), split.root);
            if child.key() != split.child_keys[*survivor] {
                let Some(DockspaceDropAction::Split { edge }) = action else {
                    panic!(
                        "a surviving branch can change identity only through exact edge wrapping"
                    );
                };
                let target = destination.unwrap();
                assert_eq!(split.child_keys[*survivor], target.key);
                self.assert_target_wrapper(replay, split.key, group.tabs[0], target, edge);
            }
            converted.push(split.key);
        }
        converted
    }

    fn assert_target_wrapper(
        &self,
        replay: &Replay,
        parent: NodeKey,
        wrapped_tab: TabKey,
        target: &Group,
        edge: DockEdge,
    ) {
        let view = replay.dock.view();
        let parent = view.node(parent).unwrap();
        let Some(DockspaceTabView::Layout(wrapper)) = parent.tabs().unwrap().child(wrapped_tab)
        else {
            panic!("an edge wrapper remains one direct layout tab");
        };
        assert_eq!(wrapper.root().unwrap().id(), target.root);
        assert_ne!(wrapper.key(), target.key);
        let split = wrapper
            .split()
            .expect("an edge drop introduces a split wrapper");
        let (axis, target_index) = match edge {
            DockEdge::Left => (DockspaceAxis::Horizontal, 1),
            DockEdge::Right => (DockspaceAxis::Horizontal, 0),
            DockEdge::Top => (DockspaceAxis::Vertical, 1),
            DockEdge::Bottom => (DockspaceAxis::Vertical, 0),
        };
        assert_eq!(split.axis(), axis);
        let children = split.children().collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        let original = children[target_index];
        assert_eq!(
            original.key(),
            target.key,
            "the original target node is retained"
        );
        assert_eq!(original.layout_title(), target.title.as_deref());
        let original_tabs = original
            .tabs()
            .expect("the original target stays a tabs container");
        assert_eq!(original_tabs.tabs(), target.tabs);
        assert_eq!(original_tabs.selected(), target.selected);
        let payload = children[1 - target_index];
        assert_eq!(payload.items().collect::<Vec<_>>(), self.payload);
        assert_eq!(
            payload.key(),
            view.item(self.selected).unwrap().tabs().unwrap().key(),
            "the other wrapper child owns exactly the moved payload group",
        );
        assert_eq!(
            split.weights().collect::<Vec<_>>(),
            [0.5, 0.5],
            "the workbench's admitted edge drop requests an equal share",
        );
    }
}
