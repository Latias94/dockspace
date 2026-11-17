use std::collections::BTreeSet;

use crate::graph::Workspace;
use crate::graph::command::CloseCommitOutcome;
use crate::graph::error::{CommandError, TransactionError};
use crate::graph::operation::{
    PreparedContentClose, PreparedContentCloseCommit, prepare_content_close,
};
use crate::graph::workspace::RootPresentationOwner;
use crate::ids::{ItemId, NodeId, RootId, TabKey};
use crate::policy::DockPolicySnapshot;

use super::CloseItemRequirement;

/// Semantic facts covered by an ordinary content-close approval.
///
/// Selection, MRU, measurements, scroll, split geometry and unrelated content
/// are deliberately absent. The coordinator separately guards the engine domain
/// and workspace epoch, and the engine reconciles this scope after each change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContentCloseScope {
    Item {
        root: RootId,
        owner_node: NodeId,
        item: ItemId,
        owner: RootPresentationOwner,
        requirement: CloseItemRequirement,
    },
    Root {
        root: RootId,
        owner: RootPresentationOwner,
        items: BTreeSet<ItemId>,
        requirements: Vec<CloseItemRequirement>,
    },
    Tabs {
        root: RootId,
        tabs: NodeId,
        owner: RootPresentationOwner,
        items: BTreeSet<ItemId>,
        requirements: Vec<CloseItemRequirement>,
    },
    Tab {
        root: RootId,
        tabs: NodeId,
        tab: TabKey,
        node: NodeId,
        owner: RootPresentationOwner,
        items: BTreeSet<ItemId>,
        requirements: Vec<CloseItemRequirement>,
    },
}

impl ContentCloseScope {
    pub(crate) const fn root(&self) -> RootId {
        match self {
            Self::Item { root, .. }
            | Self::Root { root, .. }
            | Self::Tabs { root, .. }
            | Self::Tab { root, .. } => *root,
        }
    }

    pub(crate) fn matches(&self, workspace: &Workspace, policy: &DockPolicySnapshot) -> bool {
        match self {
            Self::Item {
                root,
                owner_node,
                item,
                owner,
                requirement,
            } => {
                workspace.presentation_for_root(*root) == Some(*owner)
                    && workspace.locate_item_by_id(*item).is_some_and(|location| {
                        location.root == *root && location.owner_node() == *owner_node
                    })
                    && requirement_matches(*requirement, policy)
            }
            Self::Root {
                root,
                owner,
                items,
                requirements,
            } => {
                workspace.presentation_for_root(*root) == Some(*owner)
                    && workspace.root(*root).is_some_and(|record| {
                        let actual = workspace.collect_items_in_subtree(record.node);
                        actual.len() == items.len()
                            && actual.iter().all(|item| items.contains(item))
                    })
                    && requirements
                        .iter()
                        .all(|requirement| requirement_matches(*requirement, policy))
            }
            Self::Tabs {
                root,
                tabs,
                owner,
                items,
                requirements,
            } => {
                workspace.presentation_for_root(*root) == Some(*owner)
                    && workspace.node_root(*tabs) == Some(*root)
                    && matches!(workspace.node(*tabs), Some(crate::graph::Node::Tabs { .. }))
                    && subtree_items_match(workspace, *tabs, items)
                    && requirements
                        .iter()
                        .all(|requirement| requirement_matches(*requirement, policy))
            }
            Self::Tab {
                root,
                tabs,
                tab,
                node,
                owner,
                items,
                requirements,
            } => {
                workspace.presentation_for_root(*root) == Some(*owner)
                    && workspace.node_root(*tabs) == Some(*root)
                    && tab.layout() == Some(*node)
                    && !workspace.layout_tab_contains_central(*root, *tab)
                    && matches!(workspace.node(*tabs), Some(crate::graph::Node::Tabs { items, .. }) if items.contains(tab))
                    && subtree_items_match(workspace, *node, items)
                    && requirements
                        .iter()
                        .all(|requirement| requirement_matches(*requirement, policy))
            }
        }
    }
}

fn subtree_items_match(workspace: &Workspace, node: NodeId, expected: &BTreeSet<ItemId>) -> bool {
    let actual = workspace.collect_items_in_subtree(node);
    actual.len() == expected.len() && actual.iter().all(|item| expected.contains(item))
}

fn requirement_matches(requirement: CloseItemRequirement, policy: &DockPolicySnapshot) -> bool {
    let capability = policy.pane_close_capability(requirement.item());
    capability.allows_close() && capability == requirement.capability()
}

/// Lowers a still-current approval into exact command material for this draft.
///
/// The global item multiset below is a conservation check for this one mutation,
/// not an asynchronous approval dependency. Exact structural sources are also
/// captured only after the target scope was revalidated; UI state never becomes
/// part of the stored approval through a general root fingerprint.
pub(crate) fn prepare_scoped_content_close(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    scope: &ContentCloseScope,
) -> Result<PreparedContentCloseCommit, TransactionError> {
    if !scope.matches(workspace, policy) {
        // Public decision paths reconcile scopes before consuming tokens. A
        // mismatch here means a caller bypassed that private proof boundary.
        return Err(command_error(CommandError::Invariant {
            stage: "prepare content close without a current target scope",
        }));
    }
    let expected_items = workspace.item_multiset();
    let prepared = match scope {
        ContentCloseScope::Item {
            root,
            owner_node,
            item,
            requirement,
            ..
        } => PreparedContentClose::Item {
            source: workspace
                .capture_item_source(*root, *owner_node, *item)
                .map_err(command_error)?,
            expected_items,
            requirement: *requirement,
        },
        ContentCloseScope::Root {
            root, requirements, ..
        } => {
            let record = workspace
                .root(*root)
                .ok_or_else(|| command_error(CommandError::MissingRoot { root: *root }))?;
            PreparedContentClose::Root {
                source: workspace
                    .capture_node_source(*root, record.node)
                    .map_err(command_error)?,
                items: workspace.collect_items_in_subtree(record.node),
                expected_items,
                requirements: requirements.clone(),
            }
        }
        ContentCloseScope::Tabs {
            root,
            tabs,
            requirements,
            ..
        } => PreparedContentClose::Tabs {
            source: workspace
                .capture_node_source(*root, *tabs)
                .map_err(command_error)?,
            items: requirements
                .iter()
                .map(|requirement| requirement.item())
                .collect(),
            expected_items,
            requirements: requirements.clone(),
        },
        ContentCloseScope::Tab {
            root,
            tabs,
            tab,
            requirements,
            ..
        } => PreparedContentClose::Tab {
            source: workspace
                .capture_tab_source(*root, *tabs, *tab)
                .map_err(command_error)?,
            items: requirements
                .iter()
                .map(|requirement| requirement.item())
                .collect(),
            expected_items,
            requirements: requirements.clone(),
        },
    };
    let mut commit = prepare_content_close(workspace, policy, &prepared)?;
    if let ContentCloseScope::Root {
        root, requirements, ..
    } = scope
    {
        // Reports preserve the approval roster even when tabs were reordered.
        commit.outcome = CloseCommitOutcome::RootClosed {
            root: *root,
            items: requirements
                .iter()
                .map(|requirement| requirement.item())
                .collect(),
        };
    }
    Ok(commit)
}

fn command_error(source: CommandError) -> TransactionError {
    TransactionError::Command { index: 0, source }
}
