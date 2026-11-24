use super::*;
use crate::graph::normalized_weight_sum;
use crate::model::DockspaceGridShares;

#[derive(Debug)]
enum DetachedPayload {
    Item {
        source_root: RootId,
        item: ItemId,
        standalone: bool,
    },
    Node {
        source_root: RootId,
        node: NodeId,
        items: Vec<ItemId>,
        selected: Option<TabKey>,
        mru: Option<Vec<TabKey>>,
        merge_tabs: bool,
        central: Option<NodeId>,
    },
}

impl DetachedPayload {
    fn source_root(&self) -> RootId {
        match self {
            Self::Item { source_root, .. } | Self::Node { source_root, .. } => *source_root,
        }
    }

    fn items(&self) -> Vec<ItemId> {
        match self {
            Self::Item { item, .. } => vec![*item],
            Self::Node { items, .. } => items.clone(),
        }
    }
}

/// Concrete topology changes produced by one completed move.
#[derive(Default)]
pub(super) struct MoveEffects {
    pub(super) inserted_split: Option<NodeId>,
    pub(super) retired_source_surface: Option<SurfaceId>,
}

pub(super) fn apply_command(
    workspace: &mut Workspace,
    authority: TransactionAuthority<'_>,
    command: &WorkspaceCommand,
) -> Result<(AppliedCommand, MoveEffects), CommandError> {
    if let TransactionAuthority::Policy(policy) = authority {
        if matches!(command, WorkspaceCommand::Move { .. }) {
            policy.check_arrangement_unlocked()?;
        } else {
            authorize_workspace_command(workspace, policy, command)?;
        }
    }
    let result = match command {
        WorkspaceCommand::Select { source } => select(workspace, source),
        WorkspaceCommand::SelectItem { source } => select_item(workspace, source),
        WorkspaceCommand::SetItemVisibility { source, visible } => {
            set_item_visibility(workspace, source, *visible)
        }
        WorkspaceCommand::Reorder {
            source,
            insertion_index,
        } => reorder(workspace, source, *insertion_index),
        WorkspaceCommand::Open { item, target } => open(workspace, *item, target),
        WorkspaceCommand::Move { payload, target } => {
            let policy = match authority {
                TransactionAuthority::Policy(policy) => Some(policy),
                TransactionAuthority::SurfaceRecovery => None,
            };
            return move_payload(workspace, policy, payload, target);
        }
        WorkspaceCommand::ResizeTracks { tracks } => resize_tracks(workspace, tracks),
        WorkspaceCommand::AttachEmptySurface { surface, root } => {
            attach_empty_surface(workspace, *surface, *root)
        }
        WorkspaceCommand::RetireEmptySurface { surface } => {
            retire_empty_surface(workspace, *surface)
        }
        WorkspaceCommand::CreateSurfaceRoot {
            surface,
            root,
            content,
        } => create_surface_root(workspace, *surface, *root, content),
        WorkspaceCommand::CreateContainedRoot {
            surface,
            root,
            floating,
            rect,
            position,
            content,
        } => create_contained_root(
            workspace, *surface, *root, *floating, *rect, *position, content,
        ),
        WorkspaceCommand::InstallMainRoot {
            surface,
            root,
            content,
        } => install_main_root(workspace, *surface, *root, content),
        WorkspaceCommand::RehomeRoot { source, target } => rehome_root(workspace, source, *target),
        WorkspaceCommand::PromoteContained {
            source,
            surface,
            floating,
        } => promote_contained(workspace, source, *surface, *floating),
        WorkspaceCommand::UpdateContainedRect {
            surface,
            root,
            floating,
            expected_rect,
            rect,
        } => update_contained_rect(workspace, *surface, *root, *floating, *expected_rect, *rect),
        WorkspaceCommand::UpdateContainedPresentation {
            source,
            floating,
            expected_rect,
            expected_roster,
            rect,
            position,
        } => update_contained_presentation(
            workspace,
            source,
            *floating,
            *expected_rect,
            expected_roster,
            *rect,
            *position,
        ),
        WorkspaceCommand::RaiseContained {
            source,
            floating,
            expected_roster,
        } => raise_contained(workspace, source, *floating, expected_roster),
        WorkspaceCommand::RemoveEmptyRoot { source } => remove_empty_root(workspace, source),
    };
    let applied = result?;
    if let ItemDelta::Open(item) = applied.delta {
        workspace.issue_open_item(item)?;
    }
    Ok((applied, MoveEffects::default()))
}

pub(super) fn select(
    workspace: &mut Workspace,
    source: &TabSource,
) -> Result<AppliedCommand, CommandError> {
    validate_tab_source(workspace, source)?;
    let tabs = source.tabs();
    let tab = source.tab();
    let changed = workspace.set_visible_tab_selection(tabs, tab)?;
    if changed {
        workspace.complete_selection_change(source.root(), tabs)?;
    }
    Ok(AppliedCommand {
        outcome: CommandOutcome::Selected { tab, tabs, changed },
        delta: ItemDelta::None,
    })
}

pub(super) fn select_item(
    workspace: &mut Workspace,
    source: &ItemSource,
) -> Result<AppliedCommand, CommandError> {
    validate_item_source(workspace, source)?;
    if workspace.item_is_hidden(source.item()) {
        return Err(CommandError::ItemHidden {
            item: source.item(),
        });
    }
    if let Some(tabs) = source.tabs()
        && workspace
            .root_structure(source.root())
            .is_some_and(|structure| !structure.has_composition())
    {
        let changed = workspace.set_visible_tab_selection(tabs, TabKey::for_item(source.item()))?;
        if changed {
            workspace.complete_selection_change(source.root(), tabs)?;
        }
        return Ok(AppliedCommand {
            outcome: CommandOutcome::ItemActivated {
                item: source.item(),
                changed,
            },
            delta: ItemDelta::None,
        });
    }
    let changed = activate_item_ancestry(workspace, source.root(), source.place(), source.item())?;
    if changed {
        workspace.complete_root_change(source.root())?;
    }
    Ok(AppliedCommand {
        outcome: CommandOutcome::ItemActivated {
            item: source.item(),
            changed,
        },
        delta: ItemDelta::None,
    })
}

/// Writes only the selections on a validated real item's current placement path.
/// Callers own source validation and complete the resulting root issuance once.
fn activate_item_ancestry(
    workspace: &mut Workspace,
    root: RootId,
    place: ItemPlace,
    item: ItemId,
) -> Result<bool, CommandError> {
    let mut choices = Vec::new();
    if let Some(tabs) = place.tabs() {
        choices.push((tabs, TabKey::for_item(item)));
    }
    let mut node = place.owner_node();
    while let Some(parent) = workspace.parent_link(root, node) {
        if matches!(workspace.node(parent.parent), Some(Node::Tabs { .. })) {
            choices.push((parent.parent, TabKey::for_layout(node)));
        }
        node = parent.parent;
    }
    let mut changed = false;
    for (tabs, tab) in choices {
        changed |= workspace.set_visible_tab_selection(tabs, tab)?;
    }
    Ok(changed)
}

fn set_item_visibility(
    workspace: &mut Workspace,
    source: &ItemSource,
    visible: bool,
) -> Result<AppliedCommand, CommandError> {
    validate_item_source(workspace, source)?;
    let item = source.item();
    let root = source.root();
    let changed = workspace.item_is_hidden(item) == visible;
    if changed {
        workspace.begin_root_edit(root);
        let hidden_items = Arc::make_mut(&mut workspace.storage.hidden_items);
        if visible {
            hidden_items.remove(&item);
        } else {
            hidden_items.insert(item);
        }
        let mut node = source.owner_node();
        loop {
            if matches!(workspace.node(node), Some(Node::Tabs { .. })) {
                workspace.repair_visible_tab_selection(node)?;
            }
            let Some(parent) = workspace.parent_link(root, node) else {
                break;
            };
            node = parent.parent;
        }
        workspace.complete_root_change(root)?;
        workspace.complete_root_structure(root)?;
    }
    Ok(AppliedCommand {
        outcome: CommandOutcome::ItemVisibilityChanged {
            item,
            root,
            visible,
            changed,
        },
        delta: ItemDelta::None,
    })
}

fn reorder(
    workspace: &mut Workspace,
    source: &TabSource,
    insertion_index: usize,
) -> Result<AppliedCommand, CommandError> {
    validate_tab_source(workspace, source)?;
    let tabs = source.tabs();
    let tab = source.tab();
    let (from, to, changed) = reorder_tab(workspace, tabs, tab, insertion_index)?;
    if changed {
        workspace.complete_root_change(source.root())?;
    }
    Ok(AppliedCommand {
        outcome: CommandOutcome::Reordered {
            tab,
            tabs,
            from,
            to,
            changed,
        },
        delta: ItemDelta::None,
    })
}

fn open(
    workspace: &mut Workspace,
    item: ItemId,
    target: &DockTarget,
) -> Result<AppliedCommand, CommandError> {
    validate_target(workspace, target)?;
    let root = target_root(target);
    let payload = DetachedPayload::Item {
        source_root: root,
        item,
        standalone: false,
    };
    let _ = insert_payload(workspace, payload, target)?;
    let location = workspace
        .locate_item_by_id(item)
        .ok_or(CommandError::Invariant {
            stage: "locate newly opened item for ancestor activation",
        })?;
    activate_item_ancestry(workspace, root, location.place, item)?;
    workspace.complete_root_change(root)?;
    workspace.complete_root_structure(root)?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::Opened { item, root },
        delta: ItemDelta::Open(item),
    })
}

fn move_payload(
    workspace: &mut Workspace,
    policy: Option<&DockPolicySnapshot>,
    payload: &MovePayload,
    target: &DockTarget,
) -> Result<(AppliedCommand, MoveEffects), CommandError> {
    // This private owner checks and executes the exact command against its
    // exclusive candidate. No checked token or index can cross a mutation.
    check_move(workspace, policy, payload, target)?;

    let in_place = if let Some((tabs, tab)) =
        super::authorization::same_tabs_move_source(workspace, payload, target)
    {
        match target {
            DockTarget::Center(_) => Some(false),
            DockTarget::TabGap { index, .. } => Some(reorder_tab(workspace, tabs, tab, *index)?.2),
            DockTarget::InnerEdge(_) | DockTarget::OuterEdge(_) => None,
        }
    } else if let MovePayload::Item(source) = payload {
        item_move_is_noop(workspace, source, target).then_some(false)
    } else {
        None
    };
    if let Some(changed) = in_place {
        let source_root = payload_root(payload);
        if changed {
            workspace.complete_root_change(source_root)?;
        }
        let items = match payload {
            MovePayload::Item(source) => vec![source.item()],
            MovePayload::Tabs(source) | MovePayload::Subtree(source) => {
                workspace.collect_items_in_subtree(source.node())
            }
        };
        return Ok((
            AppliedCommand {
                outcome: CommandOutcome::Moved {
                    items,
                    source_root,
                    target_root: target_root(target),
                    changed,
                },
                delta: ItemDelta::None,
            },
            MoveEffects::default(),
        ));
    }

    if let Some(source_node) = payload_node(payload)
        && source_node == target_node(target)
        && target_is_tabs(target)
    {
        return Ok((
            AppliedCommand {
                outcome: CommandOutcome::Moved {
                    items: workspace.collect_items_in_subtree(source_node),
                    source_root: payload_root(payload),
                    target_root: target_root(target),
                    changed: false,
                },
                delta: ItemDelta::None,
            },
            MoveEffects::default(),
        ));
    }

    let exact_noop_source = match (payload, target) {
        (
            MovePayload::Tabs(source) | MovePayload::Subtree(source),
            DockTarget::InnerEdge(target),
        ) if node_move_is_exact_inner_edge_noop(workspace, source, target) => Some(source),
        _ => None,
    };
    if let Some(source) = exact_noop_source {
        return Ok((
            AppliedCommand {
                outcome: CommandOutcome::Moved {
                    items: workspace.collect_items_in_subtree(source.node()),
                    source_root: source.root(),
                    target_root: target_root(target),
                    changed: false,
                },
                delta: ItemDelta::None,
            },
            MoveEffects::default(),
        ));
    }

    // Source removal may prune a layout child from an ancestor destination.
    // Only this candidate's insertion gap changes; captured authority was checked above.
    let reveal_item = workspace
        .root_structure(target_root(target))
        .filter(|structure| structure.has_composition())
        .and_then(|_| match payload {
            MovePayload::Item(source) => {
                (!workspace.item_is_hidden(source.item())).then_some(source.item())
            }
            MovePayload::Tabs(source) | MovePayload::Subtree(source) => {
                workspace.first_active_item(source.node())
            }
        });
    let mut insertion_target = target.clone();
    let detached = detach_payload(workspace, payload, Some(&mut insertion_target))?;
    let source_root = detached.source_root();
    let items = detached.items();
    let target_root = target_root(target);
    let inserted_split = insert_payload(workspace, detached, &insertion_target)?;
    if let Some(item) = reveal_item {
        let location = workspace
            .locate_item_by_id(item)
            .ok_or(CommandError::Invariant {
                stage: "locate moved content for destination activation",
            })?;
        activate_item_ancestry(workspace, target_root, location.place, item)?;
    }
    let retired_source_surface = cleanup_empty_source_root(workspace, source_root)?;
    workspace.complete_root_change(source_root)?;
    if target_root != source_root {
        workspace.complete_root_change(target_root)?;
    }
    Ok((
        AppliedCommand {
            outcome: CommandOutcome::Moved {
                items,
                source_root,
                target_root,
                changed: true,
            },
            delta: ItemDelta::None,
        },
        MoveEffects {
            inserted_split,
            retired_source_surface,
        },
    ))
}

fn node_move_is_exact_inner_edge_noop(
    workspace: &Workspace,
    source: &NodeSource,
    target: &EdgeTarget,
) -> bool {
    if source.root() != target.root() {
        return false;
    }
    let Some(root) = workspace.storage.roots.get(&source.root()) else {
        return false;
    };
    if root
        .central
        .is_some_and(|central| workspace.subtree_contains(source.node(), central))
    {
        return false;
    }
    let Some(source_parent) = workspace.parent_link(source.root(), source.node()) else {
        return false;
    };
    let Some(target_parent) = workspace.parent_link(target.root(), target.node()) else {
        return false;
    };
    if source_parent.parent != target_parent.parent {
        return false;
    }
    let Some(Node::Split {
        axis,
        children,
        weights,
        ..
    }) = workspace.storage.nodes.get(source_parent.parent)
    else {
        return false;
    };
    if children.len() != weights.len() {
        return false;
    }
    let (target_axis, insert_before) = edge_axis_and_order(target.edge());
    if *axis != target_axis {
        return false;
    }

    let mut simulated_children = children.clone();
    let mut simulated_weights = weights.clone();
    if remove_split_child_from_roster(
        &mut simulated_children,
        &mut simulated_weights,
        source_parent.index,
    )
    .is_err()
    {
        return false;
    }
    if split_existing_branch_roster(
        &mut simulated_children,
        &mut simulated_weights,
        target.node(),
        source.node(),
        insert_before,
        target.fraction().get(),
        target_axis,
    )
    .is_err()
    {
        return false;
    }

    simulated_children == *children && simulated_weights == *weights
}

fn axis_is_equal_default(weights: &[SplitWeight]) -> bool {
    weights
        .first()
        .is_some_and(|first| weights.iter().all(|weight| weight == first))
}

fn grid_resize_changed_axes(
    resize: &TrackResize,
    current: Option<&DockspaceGridShares>,
    proposed: &DockspaceGridShares,
) -> [bool; 2] {
    let effective = current.filter(|shares| {
        resize.grid_basis().is_none_or(|basis| {
            shares.row_weights().len() == basis.rows()
                && shares.column_weights().len() == basis.columns()
        })
    });
    let changed_axis = |axis, current: Option<&[SplitWeight]>, proposed: &[SplitWeight]| {
        current.map_or_else(
            || !axis_is_equal_default(proposed) || resize.materialized_axis() == Some(axis),
            |current| current != proposed,
        )
    };
    [
        changed_axis(
            Axis::Horizontal,
            effective.map(DockspaceGridShares::column_weights),
            proposed.column_weights(),
        ),
        changed_axis(
            Axis::Vertical,
            effective.map(DockspaceGridShares::row_weights),
            proposed.row_weights(),
        ),
    ]
}

pub(crate) fn validate_track_resizes(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    tracks: &[TrackResize],
) -> Result<Vec<(NodeId, bool)>, CommandError> {
    let validated = validate_track_resizes_structure(workspace, tracks)?;
    for (resize, (_, changed)) in tracks.iter().zip(&validated) {
        if !changed {
            continue;
        }
        let source = resize.source();
        let surface = workspace
            .presentation_for_root(source.root())
            .map(|owner| match owner {
                RootPresentationOwner::Main { surface }
                | RootPresentationOwner::Contained { surface, .. } => surface,
            });
        let axes = match (workspace.node(source.node()), resize.weights()) {
            (Some(Node::Split { axis, .. }), TrackWeights::Split(_)) => [Some(*axis), None],
            (
                Some(Node::Grid {
                    shares: current, ..
                }),
                TrackWeights::Grid(shares),
            ) => {
                let [horizontal, vertical] =
                    grid_resize_changed_axes(resize, current.as_ref(), shares);
                [
                    horizontal.then_some(Axis::Horizontal),
                    vertical.then_some(Axis::Vertical),
                ]
            }
            _ => {
                return Err(CommandError::Invariant {
                    stage: "authorize structurally validated track resize",
                });
            }
        };
        for axis in axes.into_iter().flatten() {
            evaluate_policy(
                policy,
                &DockPolicyRequest::Resize(DockResizePolicyRequest::new(
                    source.root(),
                    axis,
                    surface,
                )),
            )?;
        }
    }
    Ok(validated)
}

pub(super) fn validate_track_resizes_structure(
    workspace: &Workspace,
    tracks: &[TrackResize],
) -> Result<Vec<(NodeId, bool)>, CommandError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut validated = Vec::with_capacity(tracks.len());
    for resize in tracks {
        let source = resize.source();
        validate_node_source(workspace, source)?;
        let node = source.node();
        if !seen.insert(node) {
            return Err(CommandError::DuplicateTrackResize { node });
        }
        let current = workspace.node(node).ok_or(CommandError::MissingNode {
            role: ReferenceRole::Source,
            node,
        })?;
        let changed = match (current, resize.weights()) {
            (
                Node::Split {
                    children,
                    weights: current,
                    ..
                },
                TrackWeights::Split(weights),
            ) => {
                if children.len() != weights.len() {
                    return Err(CommandError::SplitWeightCountMismatch {
                        split: node,
                        children: children.len(),
                        weights: weights.len(),
                    });
                }
                for (index, weight) in weights.iter().enumerate() {
                    SplitWeight::new(weight.get()).map_err(|source| {
                        CommandError::InvalidTrackWeight {
                            node,
                            index,
                            source,
                        }
                    })?;
                }
                let sum = normalized_weight_sum(weights.iter().map(|weight| weight.get()));
                if !sum.is_finite() || (sum - 1.0).abs() > NORMALIZED_WEIGHT_TOLERANCE {
                    return Err(CommandError::SplitWeightsNotNormalized { split: node, sum });
                }
                current != weights
            }
            (
                Node::Grid {
                    shape,
                    children,
                    shares: current,
                    ..
                },
                TrackWeights::Grid(shares),
            ) => {
                shares
                    .validate_for(*shape, children.len())
                    .map_err(|source| CommandError::InvalidGrid { node, source })?;
                if let Some(basis) = resize.grid_basis() {
                    if basis.children() != children.as_slice() {
                        return Err(CommandError::GridResizeBasisMismatch { node });
                    }
                    if basis.rows() != shares.row_weights().len()
                        || basis.columns() != shares.column_weights().len()
                    {
                        return Err(CommandError::InvalidGrid {
                            node,
                            source: crate::model::DockspaceGridError::ShareShapeMismatch {
                                expected_rows: basis.rows(),
                                expected_columns: basis.columns(),
                                rows: shares.row_weights().len(),
                                columns: shares.column_weights().len(),
                            },
                        });
                    }
                }
                grid_resize_changed_axes(resize, current.as_ref(), shares)
                    .into_iter()
                    .any(|changed| changed)
            }
            _ => return Err(CommandError::TrackOwnerKindMismatch { node }),
        };
        validated.push((node, changed));
    }
    Ok(validated)
}

fn resize_tracks(
    workspace: &mut Workspace,
    tracks: &[TrackResize],
) -> Result<AppliedCommand, CommandError> {
    let validated = validate_track_resizes_structure(workspace, tracks)?;
    for (resize, (_, changed)) in tracks.iter().zip(&validated) {
        if !changed {
            continue;
        }
        workspace.begin_root_edit(resize.source().root());
        match (
            workspace.storage.nodes.get_mut(resize.source().node()),
            resize.weights(),
        ) {
            (
                Some(Node::Split {
                    weights: current, ..
                }),
                TrackWeights::Split(weights),
            ) => current.clone_from(weights),
            (
                Some(Node::Grid {
                    shares: current, ..
                }),
                TrackWeights::Grid(shares),
            ) => *current = Some(shares.clone()),
            _ => {
                return Err(CommandError::Invariant {
                    stage: "apply validated track resize batch",
                });
            }
        }
    }
    let changed_roots = tracks
        .iter()
        .zip(&validated)
        .filter_map(|(resize, (_, changed))| changed.then_some(resize.source().root()))
        .collect::<BTreeSet<_>>();
    for root in changed_roots {
        workspace.complete_root_change(root)?;
        workspace.complete_root_structure(root)?;
    }
    Ok(AppliedCommand {
        outcome: CommandOutcome::TracksResized {
            tracks: validated.iter().map(|(node, _)| *node).collect(),
            changed: validated.iter().any(|(_, changed)| *changed),
        },
        delta: ItemDelta::None,
    })
}

fn attach_empty_surface(
    workspace: &mut Workspace,
    surface: SurfaceId,
    root: RootId,
) -> Result<AppliedCommand, CommandError> {
    ensure_root_ids_available(workspace, root)?;
    if workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::SurfaceIdCollision { surface });
    }
    let central = workspace.insert_runtime_node(Node::tabs([]));
    Arc::make_mut(&mut workspace.storage.roots)
        .insert(root, RootRecord::new(central).with_central(central));
    workspace
        .storage
        .surfaces
        .insert(surface, SurfacePresentation::with_main(root));
    workspace.complete_root_birth(root, RootPresentationOwner::Main { surface })?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::EmptySurfaceAttached { surface, root },
        delta: ItemDelta::None,
    })
}

fn retire_empty_surface(
    workspace: &mut Workspace,
    surface: SurfaceId,
) -> Result<AppliedCommand, CommandError> {
    let presentation = workspace
        .storage
        .surfaces
        .get(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    let mut roots = Vec::with_capacity(presentation.contained.len().saturating_add(1));
    roots.extend(presentation.main_root);
    for floating in &presentation.contained {
        let record = workspace.storage.contained_floatings.get(floating).ok_or(
            CommandError::MissingFloating {
                floating: *floating,
            },
        )?;
        roots.push(record.root);
    }
    let mut items = 0;
    for root in &roots {
        let record = workspace
            .storage
            .roots
            .get(root)
            .ok_or(CommandError::MissingRoot { root: *root })?;
        items += workspace.collect_items_in_subtree(record.node).len();
    }
    if items != 0 {
        return Err(CommandError::SurfaceNotEmpty { surface, items });
    }
    for root in roots {
        retire_root(workspace, root)?;
    }
    workspace.storage.surfaces.remove(&surface);
    Ok(AppliedCommand {
        outcome: CommandOutcome::EmptySurfaceRetired { surface },
        delta: ItemDelta::None,
    })
}

fn create_surface_root(
    workspace: &mut Workspace,
    surface: SurfaceId,
    root: RootId,
    content: &RootContent,
) -> Result<AppliedCommand, CommandError> {
    ensure_root_ids_available(workspace, root)?;
    if workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::SurfaceIdCollision { surface });
    }
    validate_root_content(workspace, content)?;
    let (node, central, items, delta) = detach_root_content(workspace, content)?;
    Arc::make_mut(&mut workspace.storage.roots).insert(
        root,
        RootRecord {
            central,
            ..RootRecord::new(node)
        },
    );
    workspace
        .storage
        .surfaces
        .insert(surface, SurfacePresentation::with_main(root));
    workspace.complete_root_birth(root, RootPresentationOwner::Main { surface })?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::SurfaceRootCreated {
            surface,
            root,
            items,
        },
        delta,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_contained_root(
    workspace: &mut Workspace,
    surface: SurfaceId,
    root: RootId,
    floating: FloatingPresentationId,
    rect: crate::geometry::LogicalRect,
    position: ContainedPosition,
    content: &RootContent,
) -> Result<AppliedCommand, CommandError> {
    ensure_root_ids_available(workspace, root)?;
    if workspace
        .storage
        .contained_floatings
        .contains_key(&floating)
    {
        return Err(CommandError::FloatingIdCollision { floating });
    }
    if !workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::MissingSurface { surface });
    }
    contained_insertion_index(workspace, surface, floating, position)?;
    validate_root_content(workspace, content)?;
    let (node, central, items, delta) = detach_root_content(workspace, content)?;
    if !workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::MissingSurface { surface });
    }
    let insertion_index = contained_insertion_index(workspace, surface, floating, position)?;
    Arc::make_mut(&mut workspace.storage.roots).insert(
        root,
        RootRecord {
            central,
            ..RootRecord::new(node)
        },
    );
    workspace
        .storage
        .contained_floatings
        .insert(floating, ContainedFloating::new(root, rect));
    let presentation = workspace
        .storage
        .surfaces
        .get_mut(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if insertion_index > presentation.contained.len() {
        return Err(CommandError::Invariant {
            stage: "insert contained at validated roster position",
        });
    }
    presentation.contained.insert(insertion_index, floating);
    workspace.complete_root_birth(root, RootPresentationOwner::Contained { surface, floating })?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::ContainedRootCreated {
            surface,
            root,
            floating,
            items,
        },
        delta,
    })
}

fn install_main_root(
    workspace: &mut Workspace,
    surface: SurfaceId,
    root: RootId,
    content: &RootContent,
) -> Result<AppliedCommand, CommandError> {
    ensure_root_ids_available(workspace, root)?;
    require_rootless_surface(workspace, surface)?;
    validate_root_content(workspace, content)?;
    let (node, central, items, delta) = detach_root_content(workspace, content)?;
    let presentation = workspace
        .storage
        .surfaces
        .get_mut(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if let Some(existing) = presentation.main_root {
        return Err(CommandError::SurfaceMainOccupied {
            surface,
            root: existing,
        });
    }
    Arc::make_mut(&mut workspace.storage.roots).insert(
        root,
        RootRecord {
            central,
            ..RootRecord::new(node)
        },
    );
    presentation.main_root = Some(root);
    workspace.complete_root_birth(root, RootPresentationOwner::Main { surface })?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::MainRootInstalled {
            surface,
            root,
            items,
        },
        delta,
    })
}

fn rehome_root(
    workspace: &mut Workspace,
    source: &NodeSource,
    target: RootPresentationTarget,
) -> Result<AppliedCommand, CommandError> {
    validate_node_source(workspace, source)?;
    let root = source.root();
    let root_node = workspace
        .storage
        .roots
        .get(&root)
        .ok_or(CommandError::MissingRoot { root })?
        .node;
    if source.node() != root_node {
        return Err(CommandError::NodeIsNotRoot {
            root,
            node: source.node(),
        });
    }

    let current = workspace
        .presentation_for_root(root)
        .ok_or(CommandError::Invariant {
            stage: "locate root presentation before rehome",
        })?;
    let outcome = match target {
        RootPresentationTarget::NewSurface { surface } => {
            rehome_root_to_new_surface(workspace, root, current, surface)
        }
        RootPresentationTarget::Main { surface } => {
            rehome_root_to_main(workspace, root, current, surface)
        }
        RootPresentationTarget::Contained {
            surface,
            floating,
            rect,
            position,
        } => rehome_root_to_contained(
            workspace,
            root,
            current,
            ContainedRehomeTarget {
                surface,
                floating,
                rect,
                position,
            },
        ),
    }?;
    let presentation = match target {
        RootPresentationTarget::NewSurface { surface }
        | RootPresentationTarget::Main { surface } => RootPresentationOwner::Main { surface },
        RootPresentationTarget::Contained {
            surface, floating, ..
        } => RootPresentationOwner::Contained { surface, floating },
    };
    workspace.complete_root_rehome(root, presentation)?;
    Ok(outcome)
}

fn rehome_root_to_new_surface(
    workspace: &mut Workspace,
    root: RootId,
    current: RootPresentationOwner,
    surface: SurfaceId,
) -> Result<AppliedCommand, CommandError> {
    if workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::SurfaceIdCollision { surface });
    }
    detach_root_presentation(workspace, root, current)?;
    workspace
        .storage
        .surfaces
        .insert(surface, SurfacePresentation::with_main(root));
    Ok(rehome_outcome(root, surface, None, true))
}

fn rehome_root_to_main(
    workspace: &mut Workspace,
    root: RootId,
    current: RootPresentationOwner,
    surface: SurfaceId,
) -> Result<AppliedCommand, CommandError> {
    if current == (RootPresentationOwner::Main { surface }) {
        return Ok(rehome_outcome(root, surface, None, false));
    }
    require_rootless_surface(workspace, surface)?;
    if let RootPresentationOwner::Contained {
        surface: current_surface,
        floating,
    } = current
        && current_surface == surface
    {
        return Err(CommandError::ContainedPromotionRequiresDedicatedCommand { surface, floating });
    }
    detach_root_presentation(workspace, root, current)?;
    let presentation = workspace
        .storage
        .surfaces
        .get_mut(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if let Some(existing) = presentation.main_root {
        return Err(CommandError::SurfaceMainOccupied {
            surface,
            root: existing,
        });
    }
    presentation.main_root = Some(root);
    Ok(rehome_outcome(root, surface, None, true))
}

#[derive(Debug, Clone, Copy)]
struct ContainedRehomeTarget {
    surface: SurfaceId,
    floating: FloatingPresentationId,
    rect: crate::geometry::LogicalRect,
    position: ContainedPosition,
}

fn rehome_root_to_contained(
    workspace: &mut Workspace,
    root: RootId,
    current: RootPresentationOwner,
    target: ContainedRehomeTarget,
) -> Result<AppliedCommand, CommandError> {
    let ContainedRehomeTarget {
        surface,
        floating,
        rect,
        position,
    } = target;
    if !workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::MissingSurface { surface });
    }
    if let RootPresentationOwner::Contained {
        surface: current_surface,
        floating: current_floating,
    } = current
    {
        if floating != current_floating {
            return Err(CommandError::FloatingIdentityWouldChange {
                root,
                existing: current_floating,
                requested: floating,
            });
        }
        if current_surface == surface {
            return Err(CommandError::RehomeMetadataRequiresDedicatedCommand { floating });
        }
    } else if workspace
        .storage
        .contained_floatings
        .contains_key(&floating)
    {
        return Err(CommandError::FloatingIdCollision { floating });
    }

    contained_insertion_index(workspace, surface, floating, position)?;
    detach_root_presentation(workspace, root, current)?;
    if !workspace.storage.surfaces.contains_key(&surface) {
        return Err(CommandError::MissingSurface { surface });
    }
    let insertion_index = contained_insertion_index(workspace, surface, floating, position)?;
    workspace
        .storage
        .contained_floatings
        .insert(floating, ContainedFloating::new(root, rect));
    let presentation = workspace
        .storage
        .surfaces
        .get_mut(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if insertion_index > presentation.contained.len() {
        return Err(CommandError::Invariant {
            stage: "rehome contained at validated roster position",
        });
    }
    presentation.contained.insert(insertion_index, floating);
    Ok(rehome_outcome(root, surface, Some(floating), true))
}

fn promote_contained(
    workspace: &mut Workspace,
    source: &NodeSource,
    surface: SurfaceId,
    floating: FloatingPresentationId,
) -> Result<AppliedCommand, CommandError> {
    let (root, _) = validate_root_source(workspace, source)?;
    require_rootless_surface(workspace, surface)?;
    let current = workspace
        .presentation_for_root(root)
        .ok_or(CommandError::Invariant {
            stage: "locate contained presentation before promotion",
        })?;
    if current != (RootPresentationOwner::Contained { surface, floating }) {
        return floating_presentation_mismatch(workspace, floating, root, surface);
    }

    detach_root_presentation(workspace, root, current)?;
    let presentation = workspace
        .storage
        .surfaces
        .get_mut(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if let Some(existing) = presentation.main_root {
        return Err(CommandError::SurfaceMainOccupied {
            surface,
            root: existing,
        });
    }
    presentation.main_root = Some(root);
    workspace.complete_root_rehome(root, RootPresentationOwner::Main { surface })?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::ContainedPromoted {
            surface,
            root,
            floating,
        },
        delta: ItemDelta::None,
    })
}

fn rehome_outcome(
    root: RootId,
    surface: SurfaceId,
    floating: Option<FloatingPresentationId>,
    changed: bool,
) -> AppliedCommand {
    AppliedCommand {
        outcome: CommandOutcome::RootRehomed {
            root,
            surface,
            floating,
            changed,
        },
        delta: ItemDelta::None,
    }
}

fn update_contained_rect(
    workspace: &mut Workspace,
    surface: SurfaceId,
    root: RootId,
    floating: FloatingPresentationId,
    expected_rect: crate::geometry::LogicalRect,
    rect: crate::geometry::LogicalRect,
) -> Result<AppliedCommand, CommandError> {
    validate_contained(workspace, surface, root, floating)?;
    let record = workspace
        .storage
        .contained_floatings
        .get_mut(&floating)
        .ok_or(CommandError::MissingFloating { floating })?;
    if record.rect != expected_rect {
        return Err(CommandError::StaleContainedRect {
            floating,
            expected: expected_rect,
            actual: record.rect,
        });
    }
    let changed = record.rect != rect;
    record.rect = rect;
    Ok(AppliedCommand {
        outcome: CommandOutcome::ContainedRectUpdated { floating, changed },
        delta: ItemDelta::None,
    })
}

#[allow(clippy::too_many_arguments)]
fn update_contained_presentation(
    workspace: &mut Workspace,
    source: &NodeSource,
    floating: FloatingPresentationId,
    expected_rect: crate::geometry::LogicalRect,
    expected_roster: &ContainedRosterSource,
    rect: crate::geometry::LogicalRect,
    position: ContainedPosition,
) -> Result<AppliedCommand, CommandError> {
    let (root, _) = validate_root_source(workspace, source)?;
    let surface = expected_roster.surface();
    let owner = workspace
        .presentation_for_root(root)
        .ok_or(CommandError::Invariant {
            stage: "locate contained presentation before update",
        })?;
    if owner != (RootPresentationOwner::Contained { surface, floating }) {
        return floating_presentation_mismatch(workspace, floating, root, surface);
    }
    validate_contained(workspace, surface, root, floating)?;

    let record = workspace
        .storage
        .contained_floatings
        .get(&floating)
        .ok_or(CommandError::MissingFloating { floating })?;
    if record.rect != expected_rect {
        return Err(CommandError::StaleContainedRect {
            floating,
            expected: expected_rect,
            actual: record.rect,
        });
    }

    let presentation = workspace
        .storage
        .surfaces
        .get(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if presentation.contained.as_slice() != expected_roster.contained() {
        return Err(CommandError::StaleContainedRoster {
            surface,
            expected: expected_roster.contained().to_vec(),
            actual: presentation.contained.clone(),
        });
    }
    let from = presentation
        .contained
        .iter()
        .position(|entry| *entry == floating)
        .ok_or(CommandError::MissingFloating { floating })?;
    let to = contained_reinsertion_index(workspace, surface, floating, from, position)?;
    let rect_changed = expected_rect != rect;
    let order_changed = from != to;

    if rect_changed {
        workspace
            .storage
            .contained_floatings
            .get_mut(&floating)
            .ok_or(CommandError::MissingFloating { floating })?
            .rect = rect;
    }
    if order_changed {
        let presentation = workspace
            .storage
            .surfaces
            .get_mut(&surface)
            .ok_or(CommandError::MissingSurface { surface })?;
        presentation.contained.remove(from);
        presentation.contained.insert(to, floating);
    }

    Ok(AppliedCommand {
        outcome: CommandOutcome::ContainedPresentationUpdated {
            floating,
            from,
            to,
            rect_changed,
            order_changed,
        },
        delta: ItemDelta::None,
    })
}

fn raise_contained(
    workspace: &mut Workspace,
    source: &NodeSource,
    floating: FloatingPresentationId,
    expected_roster: &ContainedRosterSource,
) -> Result<AppliedCommand, CommandError> {
    let (root, _) = validate_root_source(workspace, source)?;
    let owner = workspace
        .presentation_for_root(root)
        .ok_or(CommandError::Invariant {
            stage: "locate contained presentation before raise",
        })?;
    let RootPresentationOwner::Contained {
        surface,
        floating: id,
    } = owner
    else {
        return floating_presentation_mismatch(
            workspace,
            floating,
            root,
            expected_roster.surface(),
        );
    };
    if id != floating {
        return floating_presentation_mismatch(workspace, floating, root, surface);
    }
    if expected_roster.surface() != surface {
        return Err(CommandError::ContainedRosterSurfaceMismatch {
            captured_surface: expected_roster.surface(),
            actual_surface: surface,
        });
    }
    validate_contained(workspace, surface, root, floating)?;
    let presentation = workspace
        .storage
        .surfaces
        .get_mut(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if presentation.contained.as_slice() != expected_roster.contained() {
        return Err(CommandError::StaleContainedRoster {
            surface,
            expected: expected_roster.contained().to_vec(),
            actual: presentation.contained.clone(),
        });
    }
    let from = presentation
        .contained
        .iter()
        .position(|entry| *entry == floating)
        .ok_or(CommandError::MissingFloating { floating })?;
    let to = presentation
        .contained
        .len()
        .checked_sub(1)
        .ok_or(CommandError::Invariant {
            stage: "raise contained in non-empty roster",
        })?;
    if from == to {
        return Ok(AppliedCommand {
            outcome: CommandOutcome::ContainedRaised {
                floating,
                from,
                to,
                changed: false,
            },
            delta: ItemDelta::None,
        });
    }
    presentation.contained.remove(from);
    presentation.contained.push(floating);
    Ok(AppliedCommand {
        outcome: CommandOutcome::ContainedRaised {
            floating,
            from,
            to,
            changed: true,
        },
        delta: ItemDelta::None,
    })
}

fn remove_empty_root(
    workspace: &mut Workspace,
    source: &NodeSource,
) -> Result<AppliedCommand, CommandError> {
    let (root, root_node) = validate_root_source(workspace, source)?;
    let item_count = workspace.collect_items_in_subtree(root_node).len();
    if item_count != 0 {
        return Err(CommandError::RootNotEmpty {
            root,
            items: item_count,
        });
    }
    retire_root(workspace, root)?;
    Ok(AppliedCommand {
        outcome: CommandOutcome::EmptyRootRemoved { root },
        delta: ItemDelta::None,
    })
}

fn ensure_root_ids_available(workspace: &Workspace, root: RootId) -> Result<(), CommandError> {
    if workspace.storage.roots.contains_key(&root) {
        Err(CommandError::RootIdCollision { root })
    } else {
        Ok(())
    }
}

fn detach_root_content(
    workspace: &mut Workspace,
    content: &RootContent,
) -> Result<(NodeId, Option<NodeId>, Vec<ItemId>, ItemDelta), CommandError> {
    match content {
        RootContent::OpenItem(item) => {
            let node = workspace.insert_runtime_node(Node::tabs([*item]));
            Ok((node, None, vec![*item], ItemDelta::Open(*item)))
        }
        RootContent::Move(payload) => {
            let detached = detach_payload(workspace, payload, None)?;
            let items = detached.items();
            cleanup_empty_source_root(workspace, detached.source_root())?;
            // RootContent::Move always removes actual content from its donor.
            // This completion precedes the next command's capture/check.
            workspace.complete_root_change(detached.source_root())?;
            workspace.complete_root_structure(detached.source_root())?;
            match detached {
                DetachedPayload::Item {
                    item, standalone, ..
                } => {
                    let value = if standalone {
                        Node::pane(item)
                    } else {
                        Node::tabs([item])
                    };
                    let node = workspace.insert_runtime_node(value);
                    Ok((node, None, items, ItemDelta::None))
                }
                DetachedPayload::Node { node, central, .. } => {
                    Ok((node, central, items, ItemDelta::None))
                }
            }
        }
    }
}

fn detach_payload(
    workspace: &mut Workspace,
    payload: &MovePayload,
    destination: Option<&mut DockTarget>,
) -> Result<DetachedPayload, CommandError> {
    match payload {
        MovePayload::Item(source) => {
            remove_item(workspace, source.owner_node(), source.item(), destination)?;
            Ok(DetachedPayload::Item {
                source_root: source.root(),
                item: source.item(),
                standalone: matches!(source.place(), ItemPlace::Pane { .. }),
            })
        }
        MovePayload::Tabs(source) | MovePayload::Subtree(source) => {
            let mut node = source.node();
            let source_root = source.root();
            let items = workspace.collect_items_in_subtree(node);
            let (selected, mru, is_tabs) = match workspace.storage.nodes.get(node) {
                Some(Node::Tabs { selected, mru, .. }) => (*selected, Some(mru.clone()), true),
                Some(Node::Pane { .. } | Node::Split { .. } | Node::Grid { .. }) => {
                    (None, None, false)
                }
                None => {
                    return Err(CommandError::MissingNode {
                        role: ReferenceRole::Source,
                        node,
                    });
                }
            };
            let central = if moves_central_tab_contents(workspace, payload) {
                // A central container belongs to its root. Moving its tabs
                // transfers their state while leaving that container empty.
                workspace.begin_root_edit(source_root);
                let source =
                    workspace
                        .storage
                        .nodes
                        .get_mut(node)
                        .ok_or(CommandError::MissingNode {
                            role: ReferenceRole::Source,
                            node,
                        })?;
                let title = match source {
                    Node::Tabs { layout_title, .. } => layout_title.clone(),
                    _ => None,
                };
                let mut empty = Node::tabs([]);
                empty.set_layout_title(title);
                let mut contents = std::mem::replace(source, empty);
                contents.set_layout_title(None);
                node = workspace.insert_runtime_node(contents);
                None
            } else {
                detach_node(workspace, source_root, node, destination)?
            };
            Ok(DetachedPayload::Node {
                source_root,
                node,
                items,
                selected,
                mru,
                merge_tabs: is_tabs && matches!(payload, MovePayload::Tabs(_)),
                central,
            })
        }
    }
}

pub(super) fn moves_central_tab_contents(workspace: &Workspace, payload: &MovePayload) -> bool {
    matches!(payload, MovePayload::Tabs(source) if workspace.root(source.root()).is_some_and(
        |record| record.central == Some(source.node()) && record.node != source.node()
    ))
}

pub(super) fn detach_node(
    workspace: &mut Workspace,
    root: RootId,
    node: NodeId,
    destination: Option<&mut DockTarget>,
) -> Result<Option<NodeId>, CommandError> {
    let record = *workspace
        .storage
        .roots
        .get(&root)
        .ok_or(CommandError::MissingRoot { root })?;
    if record.node == node {
        detach_root(workspace, root)?;
        return Ok(record.central);
    }
    if let Some(central) = record.central
        && workspace.subtree_contains(node, central)
    {
        return Err(CommandError::CentralNodeWouldDetach {
            root,
            source_node: node,
            central,
        });
    }
    let parent = workspace
        .parent_link(root, node)
        .ok_or(CommandError::NodeOutsideRoot {
            role: ReferenceRole::Source,
            root,
            node,
        })?;
    remove_child(workspace, parent.parent, parent.index, destination)?;
    Ok(None)
}

fn remove_child(
    workspace: &mut Workspace,
    mut parent: NodeId,
    mut index: usize,
    mut destination: Option<&mut DockTarget>,
) -> Result<(), CommandError> {
    let root = workspace.node_root(parent).ok_or(CommandError::Invariant {
        stage: "locate structural child removal root",
    })?;
    let removed = workspace
        .node(parent)
        .and_then(|value| value.child_nodes().find(|(slot, _)| *slot == index))
        .map(|(_, child)| child)
        .ok_or(CommandError::Invariant {
            stage: "detach indexed structural child",
        })?;
    workspace.begin_root_edit(root);
    workspace.unroot_subtree(removed);
    loop {
        let ancestor = workspace.parent_link(root, parent);
        let central = workspace.root(root).and_then(|record| record.central);
        let current = workspace.node(parent).ok_or(CommandError::MissingNode {
            role: ReferenceRole::Source,
            node: parent,
        })?;
        let empty = match current {
            Node::Tabs { .. } => {
                let retain_empty = destination.as_deref().is_some_and(|target| {
                    matches!(target, DockTarget::Center(target) | DockTarget::TabGap { target, .. }
                        if target.tabs() == parent)
                });
                let child = current
                    .child_nodes()
                    .find(|(slot, _)| *slot == index)
                    .map(|(_, child)| child)
                    .ok_or(CommandError::Invariant {
                        stage: "remove actual layout-tab roster entry",
                    })?;
                remove_tab(workspace, parent, TabKey::for_layout(child))?;
                if let Some(DockTarget::TabGap { target, index: gap }) = destination.as_deref_mut()
                    && target.tabs() == parent
                    && index < *gap
                {
                    *gap -= 1;
                }
                matches!(workspace.node(parent), Some(Node::Tabs { items, .. }) if items.is_empty())
                    && central != Some(parent)
                    && !retain_empty
            }
            Node::Split { .. } => {
                let Some(Node::Split {
                    children, weights, ..
                }) = workspace.storage.nodes.get_mut(parent)
                else {
                    unreachable!()
                };
                remove_split_child_from_roster(children, weights, index)?;
                children.is_empty()
            }
            Node::Grid { .. } => {
                let value =
                    workspace
                        .storage
                        .nodes
                        .get_mut(parent)
                        .ok_or(CommandError::MissingNode {
                            role: ReferenceRole::Source,
                            node: parent,
                        })?;
                let Node::Grid { children, .. } = value else {
                    unreachable!()
                };
                if index >= children.len() {
                    return Err(CommandError::Invariant {
                        stage: "remove actual grid roster entry",
                    });
                }
                children.remove(index);
                let empty = children.is_empty();
                if !empty {
                    value
                        .repair_grid_tracks()
                        .map_err(|source| CommandError::InvalidGrid {
                            node: parent,
                            source,
                        })?;
                }
                empty
            }
            Node::Pane { .. } => {
                return Err(CommandError::Invariant {
                    stage: "remove structural child from a bare pane",
                });
            }
        };
        if !empty {
            workspace.refresh_child_placements(parent);
            return Ok(());
        }
        if let Some(ancestor) = ancestor {
            workspace.remove_runtime_node(parent);
            parent = ancestor.parent;
            index = ancestor.index;
        } else {
            detach_root(workspace, root)?;
            workspace.remove_runtime_node(parent);
            return Ok(());
        }
    }
}

fn remove_split_child_from_roster(
    children: &mut Vec<NodeId>,
    weights: &mut Vec<SplitWeight>,
    index: usize,
) -> Result<(), CommandError> {
    if index >= children.len() || index >= weights.len() {
        return Err(CommandError::Invariant {
            stage: "detach indexed split child",
        });
    }
    children.remove(index);
    weights.remove(index);
    if weights.len() == 1 {
        weights[0] = SplitWeight::new(1.0).map_err(|_| CommandError::Invariant {
            stage: "assign singleton split weight",
        })?;
    } else if !weights.is_empty() {
        *weights =
            SplitWeight::normalize(weights.iter().map(|weight| weight.get())).map_err(|_| {
                CommandError::Invariant {
                    stage: "normalize weights after detaching child",
                }
            })?;
    }
    Ok(())
}

fn insert_payload(
    workspace: &mut Workspace,
    payload: DetachedPayload,
    target: &DockTarget,
) -> Result<Option<NodeId>, CommandError> {
    match target {
        DockTarget::Center(target) => {
            let index = tabs_len(workspace, target.tabs())?;
            merge_payload_into_tabs(workspace, payload, target.tabs(), index).map(|()| None)
        }
        DockTarget::TabGap { target, index } => {
            merge_payload_into_tabs(workspace, payload, target.tabs(), *index).map(|()| None)
        }
        DockTarget::InnerEdge(target) | DockTarget::OuterEdge(target) => {
            insert_payload_at_edge(workspace, &payload, target).map(Some)
        }
    }
}

fn merge_payload_into_tabs(
    workspace: &mut Workspace,
    payload: DetachedPayload,
    target: NodeId,
    index: usize,
) -> Result<(), CommandError> {
    if matches!(payload, DetachedPayload::Node { .. })
        && let Some(root) = workspace.node_root(target)
    {
        workspace.begin_root_edit(root);
    }
    if let Some(Node::Pane { item }) = workspace.node(target) {
        let item = *item;
        if let Some(root) = workspace.node_root(target) {
            workspace.begin_root_edit(root);
        }
        workspace.storage.nodes.replace(target, Node::tabs([item]));
    }
    let (items, selected, mru) = match payload {
        DetachedPayload::Item { item, .. } => (
            vec![TabKey::for_item(item)],
            Some(TabKey::for_item(item)),
            None,
        ),
        DetachedPayload::Node {
            node,
            merge_tabs: true,
            selected,
            mru,
            ..
        } => {
            let source =
                workspace
                    .storage
                    .nodes
                    .get_mut(node)
                    .ok_or(CommandError::MissingNode {
                        role: ReferenceRole::Source,
                        node,
                    })?;
            let Node::Tabs {
                items,
                selected: source_selected,
                mru: source_mru,
                ..
            } = source
            else {
                return Err(CommandError::NodeIsNotTabs { node });
            };
            let items = std::mem::take(items);
            *source_selected = None;
            source_mru.clear();
            workspace.remove_runtime_node(node);
            (items, selected, mru)
        }
        DetachedPayload::Node { node, .. } => {
            let tab = match workspace.node(node) {
                Some(Node::Pane { item }) => {
                    let tab = TabKey::for_item(*item);
                    workspace.remove_runtime_node(node);
                    tab
                }
                Some(_) => TabKey::for_layout(node),
                None => {
                    return Err(CommandError::MissingNode {
                        role: ReferenceRole::Source,
                        node,
                    });
                }
            };
            (vec![tab], Some(tab), None)
        }
    };
    insert_tabs(workspace, target, index, &items, selected, mru.as_deref())
}

fn insert_tabs(
    workspace: &mut Workspace,
    tabs: NodeId,
    index: usize,
    inserted: &[TabKey],
    selected_item: Option<TabKey>,
    source_mru: Option<&[TabKey]>,
) -> Result<(), CommandError> {
    // A hidden payload retains its flag and does not displace visible focus in
    // its destination. An entirely hidden destination keeps a legal selection.
    let selected_item = selected_item
        .filter(|tab| workspace.tab_is_visible(*tab))
        .or_else(|| workspace.visible_tab_selection(tabs))
        .or_else(|| {
            inserted
                .iter()
                .copied()
                .find(|tab| workspace.tab_is_visible(*tab))
        })
        .or(selected_item);
    let node = workspace
        .storage
        .nodes
        .get_mut(tabs)
        .ok_or(CommandError::MissingNode {
            role: ReferenceRole::Target,
            node: tabs,
        })?;
    let Node::Tabs {
        items,
        selected,
        mru: target_mru,
        ..
    } = node
    else {
        return Err(CommandError::NodeIsNotTabs { node: tabs });
    };
    if index > items.len() {
        return Err(CommandError::TabIndexOutOfBounds {
            tabs,
            index,
            len: items.len(),
        });
    }
    let target_was_empty = items.is_empty();
    items.splice(index..index, inserted.iter().copied());
    if let Some(item) = selected_item {
        *selected = Some(item);
    }
    if target_was_empty && let Some(source_mru) = source_mru {
        *target_mru = source_mru.to_vec();
        if let Some(item) = selected_item {
            target_mru.retain(|entry| *entry != item);
            target_mru.insert(0, item);
        }
    } else {
        if let Some(item) = selected_item {
            target_mru.retain(|candidate| *candidate != item);
            target_mru.insert(0, item);
        }
        for &item in inserted {
            if Some(item) != selected_item {
                target_mru.push(item);
            }
        }
    }
    workspace.repair_visible_tab_selection(tabs)?;
    workspace.refresh_child_placements(tabs);
    if let Some(root) = workspace.node_root(tabs) {
        for tab in inserted {
            if let Some(child) = tab.layout() {
                workspace.assign_subtree(root, child, workspace.parent_link(root, child));
            }
        }
    }
    Ok(())
}

fn insert_payload_at_edge(
    workspace: &mut Workspace,
    payload: &DetachedPayload,
    target: &EdgeTarget,
) -> Result<NodeId, CommandError> {
    workspace.begin_root_edit(target.root());
    let payload_node = match payload {
        DetachedPayload::Item {
            item, standalone, ..
        } => {
            let value = if *standalone {
                Node::pane(*item)
            } else {
                Node::tabs([*item])
            };
            let node = workspace.insert_runtime_node(value);
            workspace.place_node(target.root(), node, None);
            node
        }
        DetachedPayload::Node { node, .. } => {
            workspace.assign_subtree(target.root(), *node, None);
            *node
        }
    };
    let (axis, before) = edge_axis_and_order(target.edge());
    if let Some(parent) = workspace.parent_link(target.root(), target.node())
        && matches!(workspace.node(parent.parent), Some(Node::Split { axis: parent_axis, .. }) if *parent_axis == axis)
    {
        split_existing_branch(
            workspace,
            parent.parent,
            target.node(),
            payload_node,
            before,
            target.fraction().get(),
            axis,
        )?;
        return Ok(parent.parent);
    }
    wrap_target_branch(
        workspace,
        target.root(),
        target.node(),
        payload_node,
        axis,
        before,
        target.fraction().get(),
    )
}

#[allow(clippy::too_many_arguments)]
fn split_existing_branch(
    workspace: &mut Workspace,
    parent: NodeId,
    target: NodeId,
    payload: NodeId,
    before: bool,
    fraction: f64,
    axis: Axis,
) -> Result<(), CommandError> {
    let node = workspace
        .storage
        .nodes
        .get_mut(parent)
        .ok_or(CommandError::MissingNode {
            role: ReferenceRole::Target,
            node: parent,
        })?;
    let Node::Split {
        children, weights, ..
    } = node
    else {
        return Err(CommandError::NodeIsNotSplit { node: parent });
    };
    split_existing_branch_roster(children, weights, target, payload, before, fraction, axis)?;
    workspace.refresh_child_placements(parent);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn split_existing_branch_roster(
    children: &mut Vec<NodeId>,
    weights: &mut Vec<SplitWeight>,
    target: NodeId,
    payload: NodeId,
    before: bool,
    fraction: f64,
    axis: Axis,
) -> Result<(), CommandError> {
    let index =
        children
            .iter()
            .position(|child| *child == target)
            .ok_or(CommandError::Invariant {
                stage: "locate edge target in same-axis parent",
            })?;
    let branch = weights
        .get(index)
        .copied()
        .ok_or(CommandError::Invariant {
            stage: "locate edge target branch weight",
        })?
        .get();
    let payload_weight = SplitWeight::new(branch * fraction)
        .map_err(|_| CommandError::InvalidEdgeWeight { axis })?;
    let target_weight = SplitWeight::new(branch - payload_weight.get())
        .map_err(|_| CommandError::InvalidEdgeWeight { axis })?;
    weights[index] = target_weight;
    let insertion = if before { index } else { index + 1 };
    children.insert(insertion, payload);
    weights.insert(insertion, payload_weight);
    Ok(())
}

fn wrap_target_branch(
    workspace: &mut Workspace,
    root: RootId,
    target: NodeId,
    payload: NodeId,
    axis: Axis,
    before: bool,
    fraction: f64,
) -> Result<NodeId, CommandError> {
    let payload_weight =
        SplitWeight::new(fraction).map_err(|_| CommandError::InvalidEdgeWeight { axis })?;
    let target_weight =
        SplitWeight::new(1.0 - fraction).map_err(|_| CommandError::InvalidEdgeWeight { axis })?;
    let (children, weights) = if before {
        (vec![payload, target], vec![payload_weight, target_weight])
    } else {
        (vec![target, payload], vec![target_weight, payload_weight])
    };
    let previous_parent = workspace.parent_link(root, target);
    let wrapper = workspace.insert_runtime_node(Node::Split {
        axis,
        children,
        weights,
        layout_title: None,
    });
    if let Some(parent) = previous_parent {
        let node =
            workspace
                .storage
                .nodes
                .get_mut(parent.parent)
                .ok_or(CommandError::MissingNode {
                    role: ReferenceRole::Target,
                    node: parent.parent,
                })?;
        node.replace_child(parent.index, wrapper)
            .map_err(|_| CommandError::Invariant {
                stage: "replace wrapped target branch",
            })?;
    } else {
        let mut record = workspace
            .storage
            .roots
            .get(&root)
            .copied()
            .ok_or(CommandError::MissingRoot { root })?;
        if record.node != target {
            return Err(CommandError::NodeOutsideRoot {
                role: ReferenceRole::Target,
                root,
                node: target,
            });
        }
        record.node = wrapper;
        Arc::make_mut(&mut workspace.storage.roots).insert(root, record);
    }
    workspace.place_node(root, wrapper, previous_parent);
    workspace.refresh_child_placements(wrapper);
    Ok(wrapper)
}

fn item_move_is_noop(workspace: &Workspace, source: &ItemSource, target: &DockTarget) -> bool {
    if source.tabs().is_none() && source.owner_node() == target_node(target) {
        return true;
    }
    match target {
        DockTarget::InnerEdge(target) | DockTarget::OuterEdge(target)
            if Some(target.node()) == source.tabs() =>
        {
            matches!(
                workspace.storage.nodes.get(source.owner_node()),
                Some(Node::Tabs { items, .. }) if items.len() == 1
            ) && workspace
                .storage
                .roots
                .get(&source.root())
                .is_some_and(|record| record.central != source.tabs())
        }
        DockTarget::Center(_)
        | DockTarget::TabGap { .. }
        | DockTarget::InnerEdge(_)
        | DockTarget::OuterEdge(_) => false,
    }
}

fn reorder_tab(
    workspace: &mut Workspace,
    tabs: NodeId,
    item: TabKey,
    insertion_index: usize,
) -> Result<(usize, usize, bool), CommandError> {
    let has_layout = workspace
        .node(tabs)
        .is_some_and(|node| node.child_nodes().next().is_some());
    if has_layout && let Some(root) = workspace.node_root(tabs) {
        workspace.begin_root_edit(root);
    }
    let node = workspace
        .storage
        .nodes
        .get_mut(tabs)
        .ok_or(CommandError::MissingNode {
            role: ReferenceRole::Source,
            node: tabs,
        })?;
    let Node::Tabs { items, .. } = node else {
        return Err(CommandError::NodeIsNotTabs { node: tabs });
    };
    if insertion_index > items.len() {
        return Err(CommandError::TabIndexOutOfBounds {
            tabs,
            index: insertion_index,
            len: items.len(),
        });
    }
    let from = items
        .iter()
        .position(|entry| *entry == item)
        .ok_or(CommandError::TabNotInTabs { tabs, tab: item })?;
    let to = if insertion_index > from {
        insertion_index - 1
    } else {
        insertion_index
    };
    if from == to {
        return Ok((from, to, false));
    }
    items.remove(from);
    items.insert(to, item);
    if has_layout {
        workspace.refresh_child_placements(tabs);
    }
    Ok((from, to, true))
}

pub(super) fn remove_item(
    workspace: &mut Workspace,
    owner: NodeId,
    item: ItemId,
    destination: Option<&mut DockTarget>,
) -> Result<(), CommandError> {
    if let Some(Node::Pane { item: actual }) = workspace.node(owner) {
        if *actual != item {
            return Err(CommandError::ItemNotInTabs { tabs: owner, item });
        }
        let root = workspace.node_root(owner).ok_or(CommandError::Invariant {
            stage: "locate removed pane root",
        })?;
        detach_node(workspace, root, owner, destination)?;
        workspace.remove_runtime_node(owner);
        return Ok(());
    }
    remove_tab(workspace, owner, TabKey::for_item(item))
}

fn remove_tab(workspace: &mut Workspace, tabs: NodeId, tab: TabKey) -> Result<(), CommandError> {
    let node = workspace
        .storage
        .nodes
        .get_mut(tabs)
        .ok_or(CommandError::MissingNode {
            role: ReferenceRole::Source,
            node: tabs,
        })?;
    let Node::Tabs {
        items,
        selected,
        mru,
        ..
    } = node
    else {
        return Err(CommandError::NodeIsNotTabs { node: tabs });
    };
    let index = items
        .iter()
        .position(|entry| *entry == tab)
        .ok_or(CommandError::TabNotInTabs { tabs, tab })?;
    items.remove(index);
    mru.retain(|entry| *entry != tab);
    if *selected == Some(tab) {
        *selected = mru.first().copied();
    }
    workspace.repair_visible_tab_selection(tabs)?;
    workspace.refresh_child_placements(tabs);
    Ok(())
}

pub(super) fn validate_contained(
    workspace: &Workspace,
    surface: SurfaceId,
    root: RootId,
    floating: FloatingPresentationId,
) -> Result<(), CommandError> {
    let record = workspace
        .storage
        .contained_floatings
        .get(&floating)
        .ok_or(CommandError::MissingFloating { floating })?;
    if record.root != root || contained_surface(workspace, floating) != Some(surface) {
        return floating_presentation_mismatch(workspace, floating, root, surface);
    }
    let presentation = workspace
        .storage
        .surfaces
        .get(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if !presentation.contained.contains(&floating) {
        return Err(CommandError::Invariant {
            stage: "validate contained presentation backlink",
        });
    }
    Ok(())
}

fn require_rootless_surface(workspace: &Workspace, surface: SurfaceId) -> Result<(), CommandError> {
    let presentation = workspace
        .storage
        .surfaces
        .get(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    if let Some(root) = presentation.main_root {
        Err(CommandError::SurfaceMainOccupied { surface, root })
    } else {
        Ok(())
    }
}

fn contained_insertion_index(
    workspace: &Workspace,
    surface: SurfaceId,
    floating: FloatingPresentationId,
    position: ContainedPosition,
) -> Result<usize, CommandError> {
    let presentation = workspace
        .storage
        .surfaces
        .get(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    let anchor = match position {
        ContainedPosition::Front => return Ok(presentation.contained.len()),
        ContainedPosition::Before(anchor) | ContainedPosition::After(anchor) => anchor,
    };
    if anchor == floating {
        return Err(CommandError::ContainedAnchorIsSelf { floating });
    }
    if let Some(index) = presentation
        .contained
        .iter()
        .position(|entry| *entry == anchor)
    {
        let after = usize::from(matches!(position, ContainedPosition::After(_)));
        return index.checked_add(after).ok_or(CommandError::Invariant {
            stage: "calculate contained roster insertion index",
        });
    }
    if let Some(actual_surface) = contained_surface(workspace, anchor) {
        return Err(CommandError::ContainedAnchorOnDifferentSurface {
            anchor,
            expected_surface: surface,
            actual_surface,
        });
    }
    Err(CommandError::MissingContainedAnchor { surface, anchor })
}

fn contained_reinsertion_index(
    workspace: &Workspace,
    surface: SurfaceId,
    floating: FloatingPresentationId,
    from: usize,
    position: ContainedPosition,
) -> Result<usize, CommandError> {
    let presentation = workspace
        .storage
        .surfaces
        .get(&surface)
        .ok_or(CommandError::MissingSurface { surface })?;
    let anchor =
        match position {
            ContainedPosition::Front => {
                return presentation.contained.len().checked_sub(1).ok_or(
                    CommandError::Invariant {
                        stage: "reinsert contained in non-empty roster",
                    },
                );
            }
            ContainedPosition::Before(anchor) | ContainedPosition::After(anchor) => anchor,
        };
    if anchor == floating {
        return Err(CommandError::ContainedAnchorIsSelf { floating });
    }
    if let Some(anchor_index) = presentation
        .contained
        .iter()
        .position(|entry| *entry == anchor)
    {
        let anchor_after_removal = anchor_index - usize::from(anchor_index > from);
        return anchor_after_removal
            .checked_add(usize::from(matches!(position, ContainedPosition::After(_))))
            .ok_or(CommandError::Invariant {
                stage: "calculate contained reinsertion index",
            });
    }
    if let Some(actual_surface) = contained_surface(workspace, anchor) {
        return Err(CommandError::ContainedAnchorOnDifferentSurface {
            anchor,
            expected_surface: surface,
            actual_surface,
        });
    }
    Err(CommandError::MissingContainedAnchor { surface, anchor })
}

fn contained_surface(workspace: &Workspace, floating: FloatingPresentationId) -> Option<SurfaceId> {
    workspace
        .storage
        .surfaces
        .iter()
        .find_map(|(surface, presentation)| {
            presentation
                .contained
                .contains(&floating)
                .then_some(*surface)
        })
}

pub(super) fn floating_presentation_mismatch<T>(
    workspace: &Workspace,
    floating: FloatingPresentationId,
    expected_root: RootId,
    expected_surface: SurfaceId,
) -> Result<T, CommandError> {
    let record = workspace
        .storage
        .contained_floatings
        .get(&floating)
        .ok_or(CommandError::MissingFloating { floating })?;
    let actual_surface = contained_surface(workspace, floating).ok_or(CommandError::Invariant {
        stage: "locate contained roster owner",
    })?;
    Err(CommandError::FloatingPresentationMismatch {
        floating,
        expected_root,
        expected_surface,
        actual_root: record.root,
        actual_surface,
    })
}
