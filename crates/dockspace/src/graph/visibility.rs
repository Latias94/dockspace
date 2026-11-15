//! Persistent item visibility and the visible projection of unchanged tab membership.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::error::{CommandError, ReferenceRole};
use super::{Node, Workspace, WorkspaceBuilder};
use crate::ids::{ItemId, NodeId};

#[cfg(test)]
mod tests;

impl Workspace {
    /// Returns whether an owned item is hidden. Unknown items are not hidden.
    #[must_use]
    pub fn item_is_hidden(&self, item: ItemId) -> bool {
        self.storage.hidden_items.contains(&item)
    }

    /// Iterates hidden owned items in stable identity order.
    pub fn hidden_items(&self) -> impl Iterator<Item = ItemId> + '_ {
        self.storage.hidden_items.iter().copied()
    }

    /// Returns visible members of a tabs leaf in their durable tab order.
    ///
    /// Fully visible tabs borrow their roster without allocating. Missing nodes
    /// and split nodes have no visible tab members.
    #[must_use]
    pub fn visible_tab_items(&self, tabs: NodeId) -> Cow<'_, [ItemId]> {
        let Some(Node::Tabs { items, .. }) = self.node(tabs) else {
            return Cow::Borrowed(&[]);
        };
        if self.storage.hidden_items.is_empty()
            || !items.iter().any(|item| self.item_is_hidden(*item))
        {
            Cow::Borrowed(items)
        } else {
            Cow::Owned(
                items
                    .iter()
                    .copied()
                    .filter(|item| !self.item_is_hidden(*item))
                    .collect(),
            )
        }
    }

    /// Returns the visible selected item, or `None` when every member is hidden.
    ///
    /// The logical selection remains valid even in an entirely hidden stack.
    /// When repairing an edit, a hidden selection yields to the next visible
    /// member, then the preceding visible member in reverse visual order.
    #[must_use]
    pub fn visible_tab_selection(&self, tabs: NodeId) -> Option<ItemId> {
        let Node::Tabs {
            items, selected, ..
        } = self.node(tabs)?
        else {
            return None;
        };
        if let Some(item) = selected
            && !self.item_is_hidden(*item)
        {
            return Some(*item);
        }
        let selected_index =
            selected.and_then(|item| items.iter().position(|entry| *entry == item));
        match selected_index {
            Some(index) => items[index + 1..]
                .iter()
                .chain(items[..index].iter().rev())
                .copied()
                .find(|item| !self.item_is_hidden(*item)),
            None => items
                .iter()
                .copied()
                .find(|item| !self.item_is_hidden(*item)),
        }
    }

    /// Returns whether a subtree contributes visible layout content.
    ///
    /// An empty declared central leaf remains a usable docking target. A
    /// populated leaf whose members are all hidden contributes no rectangle.
    #[must_use]
    pub fn node_has_visible_content(&self, node: NodeId) -> bool {
        let tabs_have_content = |node, items: &[ItemId]| {
            if items.is_empty() {
                self.node_root(node)
                    .and_then(|root| self.root(root))
                    .is_some_and(|record| record.central == Some(node))
            } else {
                self.storage.hidden_items.is_empty()
                    || items.iter().any(|item| !self.item_is_hidden(*item))
            }
        };
        let mut pending = match self.node(node) {
            Some(Node::Tabs { items, .. }) => return tabs_have_content(node, items),
            // Canonical nonempty split trees always contain an owned item or
            // the declared empty central target. No descendant scan is needed
            // for the default, fully visible workspace.
            Some(Node::Split { .. }) if self.storage.hidden_items.is_empty() => return true,
            Some(Node::Split { children, .. }) => children.clone(),
            None => return false,
        };
        while let Some(node) = pending.pop() {
            match self.node(node) {
                Some(Node::Tabs { items, .. }) => {
                    if tabs_have_content(node, items) {
                        return true;
                    }
                }
                Some(Node::Split { children, .. }) => pending.extend(children.iter().copied()),
                None => {}
            }
        }
        false
    }

    pub(crate) fn replace_hidden_items(
        &mut self,
        hidden_items: BTreeSet<ItemId>,
    ) -> Result<(), CommandError> {
        if *self.storage.hidden_items == hidden_items {
            return Ok(());
        }
        self.storage.hidden_items = Arc::new(hidden_items);
        let tabs: Vec<_> = self
            .nodes()
            .filter_map(|(node, value)| matches!(value, Node::Tabs { .. }).then_some(node))
            .collect();
        for tabs in tabs {
            self.repair_visible_tab_selection(tabs)?;
        }
        let roots: Vec<_> = self.roots().map(|(root, _)| root).collect();
        for root in roots {
            self.begin_root_edit(root);
            self.complete_root_change(root)?;
            self.complete_root_structure(root)?;
        }
        Ok(())
    }

    pub(super) fn repair_visible_tab_selection(
        &mut self,
        tabs: NodeId,
    ) -> Result<bool, CommandError> {
        match self.visible_tab_selection(tabs) {
            Some(item) => self.set_visible_tab_selection(tabs, item),
            None => Ok(false),
        }
    }

    pub(super) fn set_visible_tab_selection(
        &mut self,
        tabs: NodeId,
        item: ItemId,
    ) -> Result<bool, CommandError> {
        if self.item_is_hidden(item) {
            return Err(CommandError::ItemHidden { item });
        }
        if matches!(self.node(tabs), Some(Node::Tabs { selected, .. }) if *selected == Some(item)) {
            return Ok(false);
        }
        let node = self
            .storage
            .nodes
            .get_mut(tabs)
            .ok_or(CommandError::MissingNode {
                role: ReferenceRole::Source,
                node: tabs,
            })?;
        let Node::Tabs { selected, mru, .. } = node else {
            return Err(CommandError::NodeIsNotTabs { node: tabs });
        };
        let index = mru
            .iter()
            .position(|candidate| *candidate == item)
            .ok_or(CommandError::ItemNotInTabs { tabs, item })?;
        let changed = *selected != Some(item);
        *selected = Some(item);
        if index != 0 {
            mru.remove(index);
            mru.insert(0, item);
        }
        Ok(changed)
    }
}

impl WorkspaceBuilder {
    /// Replaces the hidden item set in an unvalidated storage draft.
    ///
    /// The completed graph must own every hidden item; selection is validated
    /// separately and is never silently rewritten while decoding a snapshot.
    #[cfg(any(feature = "serde", test))]
    pub(crate) fn set_hidden_items(&mut self, hidden_items: BTreeSet<ItemId>) {
        self.workspace.hidden_items = Arc::new(hidden_items);
    }
}
