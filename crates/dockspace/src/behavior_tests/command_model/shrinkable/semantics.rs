//! A flat semantic oracle: no split topology, runtime identities or solver state.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Group {
    central: bool,
    items: Vec<ItemId>,
    selected: Option<ItemId>,
    mru: Vec<ItemId>,
}

impl Group {
    fn new(items: Vec<ItemId>, central: bool) -> Self {
        Self {
            central,
            selected: items.first().copied(),
            mru: items.clone(),
            items,
        }
    }

    fn select(&mut self, item: ItemId) {
        self.selected = Some(item);
        self.mru.retain(|candidate| *candidate != item);
        self.mru.insert(0, item);
    }

    fn visible_selection(&self, hidden: &BTreeSet<ItemId>) -> Option<ItemId> {
        let selected = self.selected?;
        if !hidden.contains(&selected) {
            return Some(selected);
        }
        let index = self.items.iter().position(|item| *item == selected)?;
        self.items[index + 1..]
            .iter()
            .chain(self.items[..index].iter().rev())
            .copied()
            .find(|item| !hidden.contains(item))
    }

    fn repair_visibility(&mut self, hidden: &BTreeSet<ItemId>) {
        if let Some(item) = self.visible_selection(hidden) {
            self.select(item);
        }
    }

    fn remove(&mut self, item: ItemId, hidden: &BTreeSet<ItemId>) {
        self.items.retain(|candidate| *candidate != item);
        self.mru.retain(|candidate| *candidate != item);
        if self.selected == Some(item) {
            self.selected = self.mru.first().copied();
        }
        self.repair_visibility(hidden);
    }

    fn append(&mut self, item: ItemId, hidden: &BTreeSet<ItemId>) {
        let selection = if hidden.contains(&item) {
            self.visible_selection(hidden).unwrap_or(item)
        } else {
            item
        };
        self.items.push(item);
        self.select(selection);
        if selection != item {
            self.mru.push(item);
        }
        self.repair_visibility(hidden);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Semantics {
    groups: Vec<Group>,
    hidden: BTreeSet<ItemId>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Rejection {
    HiddenSelection(ItemId),
    DuplicateOpen(ItemId),
    GapOutOfBounds { insertion: usize, len: usize },
}

impl Rejection {
    pub(super) fn matches(self, error: &dockspace::graph::error::TransactionError) -> bool {
        use dockspace::graph::error::{CommandError, TransactionError};

        let TransactionError::Command { index: 0, source } = error else {
            return false;
        };
        match (self, source) {
            (Self::HiddenSelection(expected), CommandError::ItemHidden { item })
            | (Self::DuplicateOpen(expected), CommandError::ItemAlreadyOpen { item }) => {
                expected == *item
            }
            (
                Self::GapOutOfBounds { insertion, len },
                CommandError::TabIndexOutOfBounds {
                    index, len: actual, ..
                },
            ) => insertion == *index && len == *actual,
            _ => false,
        }
    }
}

pub(super) enum Prediction {
    Success { next: Semantics, changed: bool },
    Rejected(Rejection),
}

impl Semantics {
    pub(super) fn initial() -> Self {
        Self {
            groups: vec![
                Group::new(vec![ItemId::new(1), ItemId::new(2)], true),
                Group::new((3..=INITIAL_OPEN_ITEMS).map(ItemId::new).collect(), false),
            ],
            hidden: BTreeSet::new(),
        }
    }

    fn group(&self, item: ItemId) -> Option<usize> {
        self.groups
            .iter()
            .position(|group| group.items.contains(&item))
    }

    pub(super) fn can_resize(&self, item: ItemId) -> bool {
        self.group(item).is_some() && self.groups.len() > 1
    }

    pub(super) fn items_to_close(&self, item: ItemId, group: bool) -> Vec<ItemId> {
        if group {
            self.groups[self.group(item).expect("modeled close group exists")]
                .items
                .clone()
        } else {
            vec![item]
        }
    }

    pub(super) fn predict(&self, action: ModelAction) -> Option<Prediction> {
        let mut next = self.clone();
        let changed = match action {
            ModelAction::Select { item } => {
                let group = next.group(item)?;
                if self.hidden.contains(&item) {
                    return Some(Prediction::Rejected(Rejection::HiddenSelection(item)));
                }
                let changed = next.groups[group].selected != Some(item);
                next.groups[group].select(item);
                changed
            }
            ModelAction::Reorder { item, insertion } => {
                let group = &mut next.groups[self.group(item)?];
                if insertion > group.items.len() {
                    return Some(Prediction::Rejected(Rejection::GapOutOfBounds {
                        insertion,
                        len: group.items.len(),
                    }));
                }
                let from = group
                    .items
                    .iter()
                    .position(|candidate| *candidate == item)?;
                let to = insertion - usize::from(insertion > from);
                group.items.remove(from);
                group.items.insert(to, item);
                from != to
            }
            ModelAction::CenterMove { item, target_item } => {
                let source = self.group(item)?;
                let target = self.group(target_item)?;
                if source != target {
                    next.groups[source].remove(item, &self.hidden);
                    next.groups[target].append(item, &self.hidden);
                }
                source != target
            }
            ModelAction::EdgeMove {
                item, target_item, ..
            } => {
                let source = self.group(item)?;
                let target = self.group(target_item)?;
                let group = &self.groups[source];
                let unchanged = source == target && group.items.len() == 1 && !group.central;
                if !unchanged {
                    next.groups[source].remove(item, &self.hidden);
                    next.groups.push(Group::new(vec![item], false));
                }
                !unchanged
            }
            ModelAction::Open { item, target_item } => {
                let target = self.group(target_item)?;
                if self.group(item).is_some() {
                    return Some(Prediction::Rejected(Rejection::DuplicateOpen(item)));
                }
                next.groups[target].append(item, &self.hidden);
                true
            }
            ModelAction::ResizeContaining { item } => {
                if !self.can_resize(item) {
                    return None;
                }
                true
            }
            ModelAction::Visibility { item, visible } => {
                let group = self.group(item)?;
                let changed = if visible {
                    next.hidden.remove(&item)
                } else {
                    next.hidden.insert(item)
                };
                if changed {
                    next.groups[group].repair_visibility(&next.hidden);
                }
                changed
            }
            ModelAction::Close { item, group } => {
                let index = self.group(item)?;
                let source = &mut next.groups[index];
                if group {
                    for item in source.items.drain(..) {
                        next.hidden.remove(&item);
                    }
                    source.selected = None;
                    source.mru.clear();
                } else {
                    let successor = (source.selected == Some(item))
                        .then(|| {
                            let index = source
                                .items
                                .iter()
                                .position(|candidate| *candidate == item)?;
                            source.items[index + 1..]
                                .iter()
                                .chain(source.items[..index].iter().rev())
                                .copied()
                                .find(|candidate| !self.hidden.contains(candidate))
                        })
                        .flatten();
                    source.remove(item, &self.hidden);
                    next.hidden.remove(&item);
                    if let Some(successor) = successor {
                        source.select(successor);
                    }
                }
                true
            }
        };
        next.groups
            .retain(|group| group.central || !group.items.is_empty());
        Some(Prediction::Success { next, changed })
    }

    pub(super) fn assert_workspace(&self, workspace: &Workspace, action: ModelAction) {
        let mut expected = self.groups.clone();
        expected.sort();
        let mut actual = workspace
            .nodes()
            .filter_map(|(node, value)| match value {
                Node::Tabs {
                    items,
                    selected,
                    mru,
                } => Some(Group {
                    central: workspace
                        .root(ROOT)
                        .expect("model root remains live")
                        .central
                        == Some(node),
                    items: items.clone(),
                    selected: *selected,
                    mru: mru.clone(),
                }),
                Node::Split { .. } => None,
            })
            .collect::<Vec<_>>();
        actual.sort();
        assert_eq!(actual, expected, "complete ordered groups after {action:?}");
        assert_eq!(
            workspace.hidden_items().collect::<BTreeSet<_>>(),
            self.hidden,
            "visibility after {action:?}"
        );
        assert_eq!(
            workspace.roots().map(|(root, _)| root).collect::<Vec<_>>(),
            [ROOT]
        );
        assert_eq!(
            workspace
                .surfaces()
                .map(|(surface, _)| surface)
                .collect::<Vec<_>>(),
            [SURFACE]
        );
        for (node, value) in workspace.nodes() {
            let Node::Tabs { items, .. } = value else {
                continue;
            };
            let expected = expected
                .iter()
                .find(|group| group.items == *items)
                .expect("the complete group was compared");
            assert_eq!(
                workspace.visible_tab_selection(node),
                expected.visible_selection(&self.hidden)
            );
        }
    }
}
