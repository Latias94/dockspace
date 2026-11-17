//! Test-local semantic checks independent of the adapter's action reports.

use std::collections::BTreeMap;

use egui_dockspace::{DockspaceAxis, FloatingPresentationId, ItemId, LogicalRect, RootId};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Group {
    root: RootId,
    floating: Option<FloatingPresentationId>,
    items: Vec<ItemId>,
    selected: Option<ItemId>,
}

#[derive(Debug, Clone, PartialEq)]
struct Split {
    axis: DockspaceAxis,
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
                            root: root.id(),
                            floating,
                            items: tabs.items().collect::<Vec<_>>(),
                            selected: tabs.selected().and_then(|tab| tab.item()),
                        });
                    } else if let Some(split) = node.split() {
                        let children = split.children().collect::<Vec<_>>();
                        state.splits.push(Split {
                            axis: split.axis(),
                            children: children.iter().copied().map(descendants).collect(),
                            weights: split.weights().collect(),
                        });
                        pending.extend(children.into_iter().rev());
                    }
                }
            }
        }
        state
            .groups
            .sort_by(|left, right| left.items.cmp(&right.items));
        state
    }

    pub(super) fn assert_same_content(&self, after: &Self) {
        assert_eq!(
            after.groups, self.groups,
            "resizing must preserve every group and selection"
        );
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
    let mut pending = vec![node];
    let mut items = Vec::new();
    while let Some(node) = pending.pop() {
        if let Some(tabs) = node.tabs() {
            items.extend(tabs.items());
        } else if let Some(split) = node.split() {
            pending.extend(split.children().collect::<Vec<_>>().into_iter().rev());
        }
    }
    items
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
            (source.items.clone(), source.selected.unwrap())
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

    pub(super) fn assert_docked(&self, replay: &Replay, target: ItemId, merged: bool) {
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
        let mut expected = destination.items.clone();
        if merged {
            expected.extend_from_slice(&self.payload);
        } else {
            expected.clone_from(&self.payload);
            assert!(
                after.groups.contains(destination),
                "an edge drop preserves the target group"
            );
        }
        assert_eq!(
            moved.tabs().items().collect::<Vec<_>>(),
            expected,
            "an admitted drop must move exactly its payload in order"
        );
        assert_eq!(
            moved.tabs().selected().and_then(|tab| tab.item()),
            Some(self.selected),
            "the destination must activate the moved selection"
        );
        self.assert_remainder_and_unaffected(replay, &after, Some(destination), merged);
    }

    pub(super) fn assert_floated(&self, replay: &Replay) {
        let after = LayoutState::capture(replay);
        let moved = replay.dock.view().item(self.selected).unwrap();
        assert!(
            moved.contained().is_some(),
            "an admitted Shift-drag must float its payload"
        );
        assert_eq!(moved.tabs().items().collect::<Vec<_>>(), self.payload);
        assert_eq!(
            moved.tabs().selected().and_then(|tab| tab.item()),
            Some(self.selected)
        );
        self.assert_remainder_and_unaffected(replay, &after, None, false);
    }

    fn assert_remainder_and_unaffected(
        &self,
        replay: &Replay,
        after: &LayoutState,
        destination: Option<&Group>,
        merged: bool,
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
                source.tabs().items().collect::<Vec<_>>(),
                remainder,
                "the donor preserves the remaining tab order"
            );
            assert_eq!(source.root(), self.source.root);
            assert_eq!(source.contained(), self.source.floating);
            if self
                .source
                .selected
                .is_some_and(|selected| !self.payload.contains(&selected))
            {
                assert_eq!(
                    source.tabs().selected().and_then(|tab| tab.item()),
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
        for group in &self.before.groups {
            if group == &self.source || destination.is_some_and(|target| group == target) {
                continue;
            }
            assert!(
                after.groups.contains(group),
                "unrelated group changed: {group:?}"
            );
        }
        let expected_groups =
            self.before.groups.len() + usize::from(!merged) - usize::from(remainder.is_empty());
        assert_eq!(
            after.groups.len(),
            expected_groups,
            "only the predicted groups may be created or removed"
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
}
