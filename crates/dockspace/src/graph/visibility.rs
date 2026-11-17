//! Persistent item visibility and the visible projection of unchanged tab membership.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::error::{CommandError, ReferenceRole};
use super::{Node, Workspace, WorkspaceBuilder};
use crate::ids::{ItemId, NodeId, TabKey};

#[cfg(test)]
mod tests;

impl Workspace {
    /// Returns whether an owned item is hidden. Unknown items are not hidden.
    #[must_use]
    pub fn item_is_hidden(&self, item: ItemId) -> bool {
        self.storage.hidden_items.contains(&item)
    }

    /// Returns whether a supported direct tab child contributes visible content.
    #[must_use]
    pub(crate) fn tab_is_visible(&self, tab: TabKey) -> bool {
        match (tab.item(), tab.layout()) {
            (Some(item), _) => !self.item_is_hidden(item),
            (_, Some(node)) => self.node_has_visible_content(node),
            _ => false,
        }
    }

    /// Iterates hidden owned items in stable identity order.
    pub fn hidden_items(&self) -> impl Iterator<Item = ItemId> + '_ {
        self.storage.hidden_items.iter().copied()
    }

    /// Returns visible direct children of a Tabs container in durable tab order.
    ///
    /// Fully visible tabs borrow their roster without allocating. Missing nodes
    /// and split nodes have no visible tab members.
    #[must_use]
    pub fn visible_tab_keys(&self, tabs: NodeId) -> Cow<'_, [TabKey]> {
        let Some(Node::Tabs { items, .. }) = self.node(tabs) else {
            return Cow::Borrowed(&[]);
        };
        if items.iter().all(|tab| self.tab_is_visible(*tab)) {
            Cow::Borrowed(items)
        } else {
            Cow::Owned(
                items
                    .iter()
                    .copied()
                    .filter(|tab| self.tab_is_visible(*tab))
                    .collect(),
            )
        }
    }

    /// Returns the visible selected tab, or `None` when every child is hidden.
    ///
    /// The logical selection remains valid even in an entirely hidden stack.
    /// When repairing an edit, a hidden selection yields to the next visible
    /// member, then the preceding visible member in reverse visual order.
    #[must_use]
    pub fn visible_tab_selection(&self, tabs: NodeId) -> Option<TabKey> {
        let Node::Tabs {
            items, selected, ..
        } = self.node(tabs)?
        else {
            return None;
        };
        if let Some(item) = selected
            && self.tab_is_visible(*item)
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
                .find(|item| self.tab_is_visible(*item)),
            None => items
                .iter()
                .copied()
                .find(|item| self.tab_is_visible(*item)),
        }
    }

    /// Returns whether a subtree contributes visible layout content.
    ///
    /// An empty declared central leaf remains a usable docking target. A
    /// populated leaf whose members are all hidden contributes no rectangle.
    #[must_use]
    pub fn node_has_visible_content(&self, node: NodeId) -> bool {
        let mut current = Some(node);
        let mut pending = Vec::new();
        while let Some(id) = current {
            if let Some(value) = self.node(id) {
                match value {
                    Node::Pane { item } if !self.item_is_hidden(*item) => return true,
                    Node::Tabs { items, .. } => {
                        if items
                            .iter()
                            .filter_map(|tab| tab.item())
                            .any(|item| !self.item_is_hidden(item))
                        {
                            return true;
                        }
                        if items.is_empty()
                            && self
                                .node_root(id)
                                .and_then(|root| self.root(root))
                                .is_some_and(|root| root.central == Some(id))
                        {
                            return true;
                        }
                    }
                    _ => {}
                }
                pending.extend(value.child_nodes().map(|(_, child)| child));
            }
            current = pending.pop();
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
        item: TabKey,
    ) -> Result<bool, CommandError> {
        if let Some(content) = item.item()
            && self.item_is_hidden(content)
        {
            return Err(CommandError::ItemHidden { item: content });
        }
        if !self.tab_is_visible(item) {
            return Err(CommandError::TabNotInTabs { tabs, tab: item });
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
            .ok_or(CommandError::TabNotInTabs { tabs, tab: item })?;
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
