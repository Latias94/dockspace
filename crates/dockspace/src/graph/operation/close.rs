//! Frozen close preparation against one checked workspace candidate.

use super::*;

/// Core-private frozen payload for an approved content close.
///
/// Unlike [`WorkspaceCommand`], this type cannot be constructed by callers and
/// always carries the complete item multiset and policy capabilities captured
/// when the close plan was opened.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PreparedContentClose {
    Item {
        source: ItemSource,
        expected_items: BTreeMap<ItemId, usize>,
        requirement: CloseItemRequirement,
    },
    Root {
        source: NodeSource,
        items: Vec<ItemId>,
        expected_items: BTreeMap<ItemId, usize>,
        requirements: Vec<CloseItemRequirement>,
    },
    Tabs {
        source: NodeSource,
        items: Vec<ItemId>,
        expected_items: BTreeMap<ItemId, usize>,
        requirements: Vec<CloseItemRequirement>,
    },
    Tab {
        source: TabSource,
        items: Vec<ItemId>,
        expected_items: BTreeMap<ItemId, usize>,
        requirements: Vec<CloseItemRequirement>,
    },
}

/// Core-private frozen payload for closing one complete surface's content.
///
/// Root entries are ordered as the optional main root followed by contained
/// roots in normative back-to-front roster order. Each entry carries the exact
/// root source and its deterministic subtree item order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedSurfaceContentClose {
    roster: SurfaceRosterSource,
    roots: Vec<(NodeSource, Vec<ItemId>)>,
    items: Vec<ItemId>,
    requirements: Vec<CloseItemRequirement>,
}

impl PreparedSurfaceContentClose {
    #[must_use]
    pub(crate) fn new(
        roster: SurfaceRosterSource,
        roots: Vec<(NodeSource, Vec<ItemId>)>,
        requirements: Vec<CloseItemRequirement>,
    ) -> Self {
        let items = roots
            .iter()
            .flat_map(|(_, items)| items.iter().copied())
            .collect();
        Self {
            roster,
            roots,
            items,
            requirements,
        }
    }

    pub(crate) const fn roster(&self) -> &SurfaceRosterSource {
        &self.roster
    }

    pub(crate) const fn surface(&self) -> SurfaceId {
        self.roster.surface()
    }

    pub(crate) fn roots(&self) -> &[(NodeSource, Vec<ItemId>)] {
        &self.roots
    }

    pub(crate) fn items(&self) -> &[ItemId] {
        &self.items
    }

    pub(crate) fn requirements(&self) -> &[CloseItemRequirement] {
        &self.requirements
    }
}

#[derive(Debug)]
pub(crate) struct PreparedContentCloseCommit {
    pub(crate) candidate: Workspace,
    pub(crate) outcome: CloseCommitOutcome,
}

/// Checks one exact layout-tab close before approval capture or graph mutation.
/// A protected central anchor belongs to its root and cannot be closed as a tab.
pub(crate) fn validate_layout_tab_close(
    workspace: &Workspace,
    source: &TabSource,
) -> Result<NodeId, CommandError> {
    validate_tab_source(workspace, source)?;
    let node = source.tab().layout().ok_or(CommandError::Invariant {
        stage: "validate layout-tab close for a direct content tab",
    })?;
    if workspace.layout_tab_contains_central(source.root(), source.tab()) {
        let central = workspace
            .root(source.root())
            .and_then(|record| record.central)
            .ok_or(CommandError::Invariant {
                stage: "locate protected layout-tab central anchor",
            })?;
        return Err(CommandError::CentralNodeWouldDetach {
            root: source.root(),
            source_node: node,
            central,
        });
    }
    Ok(node)
}

/// Drains an entire tabs scope while retaining only a protected central path.
/// Complete descendant subtrees are destroyed without transient empty track owners.
fn clear_tabs_content(
    workspace: &mut Workspace,
    root: RootId,
    tabs: NodeId,
) -> Result<(), CommandError> {
    let central = workspace
        .root(root)
        .and_then(|record| record.central)
        .filter(|central| workspace.subtree_contains(tabs, *central));
    let path = if let Some(central) = central {
        let path = workspace
            .path_to_node(root, central)
            .ok_or(CommandError::Invariant {
                stage: "locate close-tabs protected central path",
            })?;
        let start = path
            .iter()
            .position(|node| *node == tabs)
            .ok_or(CommandError::Invariant {
                stage: "locate tabs scope on protected central path",
            })?;
        path[start..].to_vec()
    } else {
        vec![tabs]
    };
    workspace.begin_root_edit(root);
    for (index, node) in path.iter().copied().enumerate() {
        let next = path.get(index + 1).copied();
        let current = workspace.node(node).ok_or(CommandError::MissingNode {
            role: ReferenceRole::Source,
            node,
        })?;
        let removed: Vec<_> = current
            .child_nodes()
            .map(|(_, child)| child)
            .filter(|child| Some(*child) != next)
            .collect();
        let replacement = match current {
            Node::Tabs { layout_title, .. } => {
                let selected = next.map(TabKey::for_layout);
                let mut replacement =
                    Node::tabs_from_keys(selected.into_iter().collect(), selected);
                replacement.set_layout_title(layout_title.clone());
                replacement
            }
            Node::Split {
                axis, layout_title, ..
            } => Node::Split {
                axis: *axis,
                children: vec![next.ok_or(CommandError::Invariant {
                    stage: "retain split child on protected central path",
                })?],
                weights: vec![SplitWeight::new(1.0).map_err(|_| CommandError::Invariant {
                    stage: "retain central path split share",
                })?],
                layout_title: layout_title.clone(),
            },
            Node::Grid {
                shape,
                layout_title,
                ..
            } => Node::Grid {
                shape: match shape {
                    crate::model::DockspaceGridShape::Fixed { .. } => {
                        crate::model::DockspaceGridShape::Fixed {
                            rows: std::num::NonZeroUsize::MIN,
                            columns: std::num::NonZeroUsize::MIN,
                        }
                    }
                    crate::model::DockspaceGridShape::Auto => {
                        crate::model::DockspaceGridShape::Auto
                    }
                },
                children: vec![next.ok_or(CommandError::Invariant {
                    stage: "retain grid child on protected central path",
                })?],
                shares: None,
                layout_title: layout_title.clone(),
            },
            Node::Pane { .. } => {
                return Err(CommandError::Invariant {
                    stage: "close-tabs central path contains a bare pane",
                });
            }
        };
        workspace
            .storage
            .nodes
            .replace(node, replacement)
            .ok_or(CommandError::MissingNode {
                role: ReferenceRole::Source,
                node,
            })?;
        workspace.refresh_child_placements(node);
        for child in removed {
            remove_subtree_nodes(workspace, child)?;
        }
    }
    Ok(())
}

/// Prepares one approved complete-surface content close against an isolated workspace candidate.
///
/// No topology is removed until every frozen roster, root, item, and policy fact
/// has been revalidated against the same candidate.
pub(crate) fn prepare_surface_content_close(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    prepared: &PreparedSurfaceContentClose,
) -> Result<PreparedContentCloseCommit, TransactionError> {
    let (candidate, mut outcomes) = prepare_surface_content_closes(workspace, policy, &[prepared])?;
    Ok(PreparedContentCloseCommit {
        candidate,
        outcome: outcomes.remove(0),
    })
}

/// Applies one complete native-window content roster to a single isolated draft.
/// A rejected member discards the entire draft and every sibling mutation.
pub(crate) fn prepare_surface_content_closes(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    prepared: &[&PreparedSurfaceContentClose],
) -> Result<(Workspace, Vec<CloseCommitOutcome>), TransactionError> {
    if prepared.iter().all(|prepared| prepared.items().is_empty()) {
        return Err(close_invariant("validate frozen close-surface item order"));
    }
    let mut surfaces = BTreeSet::new();
    for prepared in prepared {
        if !surfaces.insert(prepared.surface()) {
            return Err(close_invariant("validate duplicate close-window surface"));
        }
        validate_surface_content_close(workspace, policy, prepared)?;
    }
    let mut candidate = clone_workspace_draft(workspace);
    let mut expected_after = workspace.item_multiset();
    let mut outcomes = Vec::with_capacity(prepared.len());
    for prepared in prepared {
        for item in prepared.items() {
            subtract_closed_item(&mut expected_after, *item)?;
        }
        for (source, _) in prepared.roots() {
            retire_root(&mut candidate, source.root()).map_err(close_command_error)?;
        }
        outcomes.push(CloseCommitOutcome::SurfaceClosed {
            surface: prepared.surface(),
            items: prepared.items().to_vec(),
        });
    }
    prune_vacant_surfaces(&mut candidate);
    canonicalize_draft(&mut candidate).map_err(TransactionError::from)?;
    let actual_after = candidate.item_multiset();
    if actual_after != expected_after {
        return Err(TransactionError::ItemReconciliation {
            expected: expected_after,
            actual: actual_after,
        });
    }
    Ok((candidate, outcomes))
}

fn validate_surface_content_close(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    prepared: &PreparedSurfaceContentClose,
) -> Result<(), TransactionError> {
    if !workspace.matches_surface_roster_source(prepared.roster()) {
        return Err(TransactionError::Precondition {
            index: 0,
            source: TransactionPreconditionError::StaleSurfaceRoster {
                surface: prepared.surface(),
            },
        });
    }
    evaluate_policy(
        policy,
        &DockPolicyRequest::CloseSurface {
            surface: prepared.surface(),
        },
    )
    .map_err(close_command_error)?;

    let roster_sources = prepared
        .roster()
        .main_source()
        .into_iter()
        .chain(
            prepared
                .roster()
                .contained()
                .iter()
                .map(crate::graph::command::ContainedRootSource::source),
        )
        .collect::<Vec<_>>();
    if roster_sources.len() != prepared.roots().len()
        || roster_sources
            .iter()
            .zip(prepared.roots())
            .any(|(roster, (frozen, _))| *roster != frozen)
    {
        return Err(close_invariant("validate frozen close-surface root order"));
    }

    let roster_items = prepared
        .roots()
        .iter()
        .flat_map(|(_, items)| items.iter().copied())
        .collect::<Vec<_>>();
    if roster_items.as_slice() != prepared.items() {
        return Err(close_invariant("validate frozen close-surface item order"));
    }

    let requirement_items = prepared
        .requirements()
        .iter()
        .map(|requirement| requirement.item())
        .collect::<Vec<_>>();
    if requirement_items.as_slice() != prepared.items() {
        return Err(close_invariant(
            "validate frozen close-surface item requirements",
        ));
    }

    let mut frozen_roots = BTreeSet::new();
    let mut frozen_items = BTreeSet::new();
    for (source, items) in prepared.roots() {
        if !frozen_roots.insert(source.root()) {
            return Err(close_invariant(
                "validate duplicate frozen close-surface root",
            ));
        }
        for item in items {
            if !frozen_items.insert(*item) {
                return Err(close_invariant(
                    "validate duplicate frozen close-surface item",
                ));
            }
        }
    }
    for requirement in prepared.requirements() {
        validate_close_requirement(policy, *requirement)?;
    }
    for (source, frozen_items) in prepared.roots() {
        let (_, root_node) =
            validate_root_source(workspace, source).map_err(close_command_error)?;
        if workspace.collect_items_in_subtree(root_node) != *frozen_items {
            return Err(close_invariant(
                "validate frozen close-surface root item order",
            ));
        }
    }

    // The baseline is deliberately captured at apply time. A close plan owns
    // only its frozen source roster; unrelated surfaces may change while a
    // native close confirmation is pending.
    let before_items = workspace.item_multiset();
    for item in prepared.items() {
        if before_items.get(item) != Some(&1) {
            return Err(close_invariant(
                "validate unique frozen close-surface item ownership",
            ));
        }
    }
    Ok(())
}

/// Prepare one approved close against a fresh candidate workspace.
///
/// This is intentionally separate from the public command transaction. A
/// close changes the item multiset and is therefore only reachable through a
/// ClosePlan-approved payload.
pub(crate) fn prepare_content_close(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    prepared: &PreparedContentClose,
) -> Result<PreparedContentCloseCommit, TransactionError> {
    let expected_items = match prepared {
        PreparedContentClose::Item { expected_items, .. }
        | PreparedContentClose::Root { expected_items, .. }
        | PreparedContentClose::Tabs { expected_items, .. }
        | PreparedContentClose::Tab { expected_items, .. } => expected_items,
    };
    let actual_items = workspace.item_multiset();
    if &actual_items != expected_items {
        return Err(TransactionError::ItemReconciliation {
            expected: expected_items.clone(),
            actual: actual_items,
        });
    }

    let mut candidate = clone_workspace_draft(workspace);
    let outcome = match prepared {
        PreparedContentClose::Item {
            source,
            requirement,
            ..
        } => {
            validate_close_requirement(policy, *requirement)?;
            validate_item_source(&candidate, source).map_err(close_command_error)?;
            let root = source.root();
            let item = source.item();
            let successors = selected_ancestor_close_successors(
                &candidate,
                root,
                source.owner_node(),
                Some(TabKey::for_item(item)),
            );
            remove_item(&mut candidate, source.owner_node(), item, None)
                .map_err(close_command_error)?;
            Arc::make_mut(&mut candidate.storage.hidden_items).remove(&item);
            apply_selected_ancestor_close_successors(&mut candidate, &successors)
                .map_err(close_command_error)?;
            cleanup_empty_source_root(&mut candidate, root).map_err(close_command_error)?;
            candidate
                .complete_root_change(root)
                .map_err(close_command_error)?;
            CloseCommitOutcome::ItemClosed { item, root }
        }
        PreparedContentClose::Tabs {
            source,
            items,
            requirements,
            ..
        } => {
            for requirement in requirements {
                validate_close_requirement(policy, *requirement)?;
            }
            validate_node_source(&candidate, source).map_err(close_command_error)?;
            let root = source.root();
            if !matches!(candidate.node(source.node()), Some(Node::Tabs { .. })) {
                return Err(close_invariant("validate frozen close-tabs node"));
            }
            let actual = candidate.collect_items_in_subtree(source.node());
            if actual.len() != items.len() || !actual.iter().all(|item| items.contains(item)) {
                return Err(close_invariant("validate frozen close-tabs membership"));
            }
            let successors =
                selected_ancestor_close_successors(&candidate, root, source.node(), None);
            clear_tabs_content(&mut candidate, root, source.node()).map_err(close_command_error)?;
            for item in items {
                Arc::make_mut(&mut candidate.storage.hidden_items).remove(item);
            }
            apply_selected_ancestor_close_successors(&mut candidate, &successors)
                .map_err(close_command_error)?;
            cleanup_empty_source_root(&mut candidate, root).map_err(close_command_error)?;
            candidate
                .complete_root_change(root)
                .map_err(close_command_error)?;
            CloseCommitOutcome::TabsClosed {
                root,
                items: items.clone(),
            }
        }
        PreparedContentClose::Tab {
            source,
            items,
            requirements,
            ..
        } => {
            let node =
                validate_layout_tab_close(&candidate, source).map_err(close_command_error)?;
            let actual = candidate.collect_items_in_subtree(node);
            if actual.len() != items.len() || !actual.iter().all(|item| items.contains(item)) {
                return Err(close_invariant("validate frozen layout-tab membership"));
            }
            for requirement in requirements {
                validate_close_requirement(policy, *requirement)?;
            }
            let root = source.root();
            let successors = selected_ancestor_close_successors(
                &candidate,
                root,
                source.tabs(),
                Some(source.tab()),
            );
            detach_node(&mut candidate, root, node, None).map_err(close_command_error)?;
            remove_subtree_nodes(&mut candidate, node).map_err(close_command_error)?;
            apply_selected_ancestor_close_successors(&mut candidate, &successors)
                .map_err(close_command_error)?;
            cleanup_empty_source_root(&mut candidate, root).map_err(close_command_error)?;
            candidate
                .complete_root_change(root)
                .map_err(close_command_error)?;
            CloseCommitOutcome::TabClosed {
                root,
                tab: source.tab(),
                items: items.clone(),
            }
        }
        PreparedContentClose::Root {
            source,
            items,
            requirements,
            ..
        } => {
            for requirement in requirements {
                validate_close_requirement(policy, *requirement)?;
            }
            let (root, root_node) =
                validate_root_source(&candidate, source).map_err(close_command_error)?;
            let current_items = candidate.collect_items_in_subtree(root_node);
            if current_items != *items {
                return Err(TransactionError::Command {
                    index: 0,
                    source: CommandError::Invariant {
                        stage: "validate frozen close-root item order",
                    },
                });
            }
            if items.is_empty() {
                return Err(TransactionError::Command {
                    index: 0,
                    source: CommandError::RootEmpty { root },
                });
            }
            retire_root(&mut candidate, root).map_err(close_command_error)?;
            CloseCommitOutcome::RootClosed {
                root,
                items: items.clone(),
            }
        }
    };

    let mut expected_after = expected_items.clone();
    match &outcome {
        CloseCommitOutcome::ItemClosed { item, .. } => {
            expected_after.remove(item);
        }
        CloseCommitOutcome::RootClosed { items, .. }
        | CloseCommitOutcome::TabsClosed { items, .. }
        | CloseCommitOutcome::TabClosed { items, .. } => {
            for item in items {
                if expected_after.remove(item).is_none() {
                    return Err(TransactionError::Command {
                        index: 0,
                        source: CommandError::Invariant {
                            stage: "reconcile frozen close-root item multiset",
                        },
                    });
                }
            }
        }
        CloseCommitOutcome::SurfaceClosed { .. } => {
            return Err(close_invariant(
                "reject surface outcome from single-content close",
            ));
        }
    }
    prune_vacant_surfaces(&mut candidate);
    canonicalize_draft(&mut candidate).map_err(TransactionError::from)?;
    let actual_after = candidate.item_multiset();
    if actual_after != expected_after {
        return Err(TransactionError::ItemReconciliation {
            expected: expected_after,
            actual: actual_after,
        });
    }
    Ok(PreparedContentCloseCommit { candidate, outcome })
}

/// Saves close-specific visual gaps before pruning can discard their rosters.
/// The direct scope and each structural ancestor use the same successor rule.
fn selected_ancestor_close_successors(
    workspace: &Workspace,
    root: RootId,
    mut node: NodeId,
    mut closing: Option<TabKey>,
) -> Vec<(NodeId, TabKey, TabKey)> {
    let mut successors = Vec::new();
    loop {
        if let Some(closing) = closing
            && let Some(successor) = selected_tab_close_successor(workspace, node, closing)
        {
            successors.push((node, closing, successor));
        }
        let Some(parent) = workspace.parent_link(root, node) else {
            break;
        };
        closing = Some(TabKey::for_layout(node));
        node = parent.parent;
    }
    successors
}

/// Applies a saved gap only when closing exhausted the original selected branch.
/// Hidden owned siblings retain their slots; an empty central target stays visible.
fn apply_selected_ancestor_close_successors(
    workspace: &mut Workspace,
    successors: &[(NodeId, TabKey, TabKey)],
) -> Result<(), CommandError> {
    for &(tabs, closing, successor) in successors {
        let Some(node) = workspace.node(tabs) else {
            continue;
        };
        let Node::Tabs { items, .. } = node else {
            return Err(CommandError::NodeIsNotTabs { node: tabs });
        };
        if items.contains(&closing) && workspace.tab_is_visible(closing) {
            continue;
        }
        workspace.set_visible_tab_selection(tabs, successor)?;
    }
    Ok(())
}

/// Chooses the same visual successor for direct content and complete layout tabs.
fn selected_tab_close_successor(
    workspace: &Workspace,
    tabs: NodeId,
    closing: TabKey,
) -> Option<TabKey> {
    let Node::Tabs {
        items, selected, ..
    } = workspace.node(tabs)?
    else {
        return None;
    };
    if *selected != Some(closing) {
        return None;
    }
    let index = items.iter().position(|tab| *tab == closing)?;
    items[index + 1..]
        .iter()
        .chain(items[..index].iter().rev())
        .copied()
        .find(|tab| workspace.tab_is_visible(*tab))
}

fn close_command_error(error: CommandError) -> TransactionError {
    TransactionError::Command {
        index: 0,
        source: error,
    }
}

fn close_invariant(stage: &'static str) -> TransactionError {
    close_command_error(CommandError::Invariant { stage })
}

fn subtract_closed_item(
    items: &mut BTreeMap<ItemId, usize>,
    item: ItemId,
) -> Result<(), TransactionError> {
    let Some(count) = items.get_mut(&item) else {
        return Err(close_invariant(
            "reconcile frozen close-surface item multiset",
        ));
    };
    if *count == 0 {
        return Err(close_invariant(
            "reconcile zero-count close-surface item multiset",
        ));
    }
    if *count == 1 {
        items.remove(&item);
    } else {
        *count -= 1;
    }
    Ok(())
}

fn validate_close_requirement(
    policy: &DockPolicySnapshot,
    requirement: CloseItemRequirement,
) -> Result<(), TransactionError> {
    evaluate_policy(
        policy,
        &DockPolicyRequest::ClosePane {
            item: requirement.item(),
            deferred: false,
        },
    )
    .map_err(close_command_error)?;
    let actual = policy.pane_close_capability(requirement.item());
    if actual != requirement.capability() {
        return Err(close_command_error(CommandError::CloseCapabilityChanged {
            item: requirement.item(),
            expected: requirement.capability(),
            actual,
        }));
    }
    if !actual.allows_close() {
        return Err(TransactionError::Command {
            index: 0,
            source: CommandError::Invariant {
                stage: "close policy disabled after plan capture",
            },
        });
    }
    Ok(())
}
