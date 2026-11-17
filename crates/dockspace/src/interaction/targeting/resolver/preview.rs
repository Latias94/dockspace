//! Post-transaction edge preview projection.

use std::sync::Arc;

use thiserror::Error;

use crate::geometry::{GeometryError, LogicalRect};
use crate::graph::command::{DockTarget, MovePayload, NodeSource, WorkspaceCommand};
use crate::graph::error::CommandError;
use crate::graph::{Node, Workspace};
use crate::ids::{NodeId, RootId};
use crate::interaction::targeting::targets::{DropTargetKind, DropTargetRecord, DropVisual};
use crate::policy::DockPolicySnapshot;
use crate::presentation::compiler::{
    FutureLayoutProjectionError, FutureTabStrip, candidate_tab_bar_policy, project_future_root,
    project_future_tab_strip,
};
use crate::presentation::layout::{NodeMap, NodeResult};
use crate::presentation::scene::{PresentationLayoutFacts, PresentationPlan};

pub(super) struct ResolvedPreview {
    pub(super) visual: DropVisual,
    pub(super) tab_strip: Option<Arc<FutureTabStrip>>,
}

impl From<DropVisual> for ResolvedPreview {
    fn from(visual: DropVisual) -> Self {
        Self {
            visual,
            tab_strip: None,
        }
    }
}

pub(super) fn resolved_visual(
    original: &Workspace,
    candidate: &Workspace,
    policy: &DockPolicySnapshot,
    target_plan: &PresentationPlan,
    source_layout_facts: Option<&PresentationLayoutFacts>,
    target_record: &DropTargetRecord,
    command: &WorkspaceCommand,
) -> Result<ResolvedPreview, DropPreviewProjectionError> {
    if !matches!(
        target_record.id().kind(),
        DropTargetKind::TabGap
            | DropTargetKind::Center
            | DropTargetKind::InnerEdge
            | DropTargetKind::OuterEdge
    ) {
        return Ok(target_record.visual().into());
    }
    let WorkspaceCommand::Move { payload, target } = command else {
        return Ok(target_record.visual().into());
    };
    if let MovePayload::Subtree(source) = payload
        && let DockTarget::Center(destination)
        | DockTarget::TabGap {
            target: destination,
            ..
        } = target
        && source.root() == destination.root()
        && source.node() == destination.tabs()
        && matches!(original.node(source.node()), Some(Node::Tabs { .. }))
    {
        // Preflight proves that returning a whole group to itself is a no-op.
        // Central targets remain queryable, but no nested layout tab is created.
        return Ok(target_record.visual().into());
    }
    let root = target_root(target);
    let Some(projection) = project_future_root(
        original,
        candidate,
        policy,
        target_plan,
        source_layout_facts,
        root,
    )?
    else {
        // Synthetic plans exist only in crate-local resolver tests. Production
        // plans always retain their measured layout facts.
        return Ok(target_record.visual().into());
    };
    let tab_strip = match target {
        DockTarget::Center(target) | DockTarget::TabGap { target, .. } => {
            match project_future_tab_strip(
                original,
                candidate,
                policy,
                target_plan,
                source_layout_facts,
                target,
                payload,
                &projection,
            ) {
                Ok(projected) => Some(projected),
                // A previously hidden strip has no host measurements. Keep the
                // legal content preview, rather than inventing its tab chrome.
                Err(
                    FutureLayoutProjectionError::TabStripMeasurementUnavailable { .. }
                    | FutureLayoutProjectionError::TabIntrinsicUnavailable { .. },
                ) => None,
                Err(error) => return Err(error.into()),
            }
        }
        DockTarget::InnerEdge(_) | DockTarget::OuterEdge(_) => None,
    };
    let rect = match target {
        DockTarget::Center(target) => projected_center_rect(
            original,
            candidate,
            policy,
            target_plan,
            target,
            &projection.nodes,
        )?,
        DockTarget::InnerEdge(_) | DockTarget::OuterEdge(_) => match payload {
            MovePayload::Item(source) => {
                let location = candidate.locate_item_by_id(source.item()).ok_or(
                    DropPreviewProjectionError::PayloadItemMissing {
                        item: source.item(),
                    },
                )?;
                if let Some(tabs) = location.tabs() {
                    let target = candidate.capture_tab_target(location.root, tabs)?;
                    projected_center_rect(
                        original,
                        candidate,
                        policy,
                        target_plan,
                        &target,
                        &projection.nodes,
                    )?
                } else {
                    projection
                        .nodes
                        .get(&location.owner_node())
                        .filter(|result| result.visible)
                        .map(|result| result.rect)
                        .ok_or(DropPreviewProjectionError::PayloadProjectionMissing {
                            root,
                            node: location.owner_node(),
                        })?
                }
            }
            MovePayload::Tabs(source) => {
                projected_tabs_rect(original, candidate, root, source, &projection.nodes)?
            }
            MovePayload::Subtree(source) => {
                projected_subtree_rect(original, candidate, root, source, &projection.nodes)?
            }
        },
        DockTarget::TabGap { target, .. } => match tab_strip.as_ref() {
            Some(projected) => projected.insertion,
            None => projected_center_rect(
                original,
                candidate,
                policy,
                target_plan,
                target,
                &projection.nodes,
            )?,
        },
    };
    let rect = match target {
        DockTarget::InnerEdge(_) | DockTarget::OuterEdge(_) => {
            clip_to_visible_root(target_plan, root, rect)?
        }
        DockTarget::Center(_) | DockTarget::TabGap { .. } => rect,
    };
    Ok(ResolvedPreview {
        visual: DropVisual::new(rect),
        tab_strip: tab_strip.and_then(|projected| projected.strip),
    })
}

fn clip_to_visible_root(
    target_plan: &PresentationPlan,
    root: RootId,
    rect: LogicalRect,
) -> Result<LogicalRect, DropPreviewProjectionError> {
    let facts = target_plan
        .layout_facts()
        .ok_or(FutureLayoutProjectionError::TargetLayoutFactsMissing)?;
    let root_bounds = facts
        .root(root)
        .ok_or(FutureLayoutProjectionError::TargetRootNotPresented { root })?
        .bounds();
    let rect = clip_rect(rect, root_bounds)?;
    Ok(clip_rect(rect, target_plan.bounds())?)
}

fn clip_rect(rect: LogicalRect, clip: LogicalRect) -> Result<LogicalRect, GeometryError> {
    let min_x = rect.x().max(clip.x()).min(clip.max().x());
    let min_y = rect.y().max(clip.y()).min(clip.max().y());
    let max_x = rect.max().x().min(clip.max().x()).max(min_x);
    let max_y = rect.max().y().min(clip.max().y()).max(min_y);
    LogicalRect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn projected_center_rect(
    original: &Workspace,
    candidate: &Workspace,
    policy: &DockPolicySnapshot,
    target_plan: &PresentationPlan,
    target: &crate::graph::command::TabTarget,
    node_results: &NodeMap<NodeResult>,
) -> Result<LogicalRect, DropPreviewProjectionError> {
    let bounds = node_results
        .get(&target.tabs())
        .map(|result| result.rect)
        .ok_or(DropPreviewProjectionError::TargetProjectionMissing {
            root: target.root(),
            node: target.tabs(),
        })?;
    let crate::graph::Node::Tabs { .. } = candidate.node(target.tabs()).ok_or(
        DropPreviewProjectionError::TargetProjectionMissing {
            root: target.root(),
            node: target.tabs(),
        },
    )?
    else {
        return Err(DropPreviewProjectionError::ProjectedCenterIsNotTabs {
            root: target.root(),
            node: target.tabs(),
        });
    };
    let facts = target_plan
        .layout_facts()
        .ok_or(DropPreviewProjectionError::TargetLayoutFactsMissing)?;
    let root_bounds = facts
        .root(target.root())
        .ok_or(FutureLayoutProjectionError::TargetRootNotPresented {
            root: target.root(),
        })?
        .bounds();
    let bounds = clip_rect(bounds, root_bounds)?;
    let tab_bar = candidate_tab_bar_policy(
        original,
        candidate,
        policy,
        target_plan,
        target.root(),
        target.tabs(),
    )?;
    let tab_height = if tab_bar.visibility() == crate::policy::TabBarVisibility::Visible {
        facts.config().tab_bar_height().min(bounds.height())
    } else {
        0.0
    };
    Ok(LogicalRect::new(
        bounds.x(),
        bounds.y() + tab_height,
        bounds.width(),
        (bounds.height() - tab_height).max(0.0),
    )?)
}

fn projected_tabs_rect(
    original: &Workspace,
    candidate: &Workspace,
    target_root: RootId,
    source: &NodeSource,
    node_results: &NodeMap<NodeResult>,
) -> Result<LogicalRect, DropPreviewProjectionError> {
    let missing = || DropPreviewProjectionError::PayloadGroupMissing {
        root: target_root,
        source_node: source.node(),
    };
    let Some(Node::Tabs { items, .. }) = original.node(source.node()) else {
        return Err(missing());
    };
    // A central anchor stays in its original root while its complete roster
    // moves to a new carrier. Direct tab identity locates that actual carrier;
    // descendant pane geometry would omit the group's own tab bar.
    let first = items.first().copied().ok_or_else(missing)?;
    let carrier = if let Some(item) = first.item() {
        let location = candidate.locate_item_by_id(item).ok_or_else(missing)?;
        if location.root != target_root {
            return Err(missing());
        }
        location.tabs().ok_or_else(missing)?
    } else {
        candidate
            .parent_link(target_root, first.layout().ok_or_else(missing)?)
            .ok_or_else(missing)?
            .parent
    };
    if !matches!(candidate.node(carrier), Some(Node::Tabs { items: actual, .. }) if actual == items)
    {
        return Err(missing());
    }
    projected_node_rect(target_root, carrier, node_results)
}

fn projected_node_rect(
    root: RootId,
    node: NodeId,
    node_results: &NodeMap<NodeResult>,
) -> Result<LogicalRect, DropPreviewProjectionError> {
    node_results
        .get(&node)
        .filter(|result| result.visible)
        .map(|result| result.rect)
        .ok_or(DropPreviewProjectionError::PayloadProjectionMissing { root, node })
}

fn projected_subtree_rect(
    original: &Workspace,
    candidate: &Workspace,
    target_root: RootId,
    source: &NodeSource,
    node_results: &NodeMap<NodeResult>,
) -> Result<LogicalRect, DropPreviewProjectionError> {
    if candidate.node_root(source.node()) == Some(target_root) {
        return projected_node_rect(target_root, source.node(), node_results);
    }
    let mut pending = vec![source.node()];
    let mut bounds = None;
    while let Some(node) = pending.pop() {
        if candidate.node_root(node) != Some(target_root) {
            // Canonicalization can flatten an unnamed same-axis Split into
            // its destination. Its surviving child containers still carry
            // the complete geometry, including any nested tab bars.
            let Some(Node::Split { children, .. }) = original.node(node) else {
                return Err(DropPreviewProjectionError::PayloadProjectionMissing {
                    root: target_root,
                    node,
                });
            };
            pending.extend(children.iter().rev().copied());
            continue;
        }
        let result = node_results.get(&node).ok_or(
            DropPreviewProjectionError::PayloadProjectionMissing {
                root: target_root,
                node,
            },
        )?;
        if !result.visible {
            continue;
        }
        let rect = result.rect;
        bounds = Some(match bounds {
            None => rect,
            Some(previous) => union_rect(previous, rect)?,
        });
    }
    bounds.ok_or(DropPreviewProjectionError::PayloadProjectionMissing {
        root: target_root,
        node: source.node(),
    })
}

fn union_rect(left: LogicalRect, right: LogicalRect) -> Result<LogicalRect, GeometryError> {
    LogicalRect::new(
        left.x().min(right.x()),
        left.y().min(right.y()),
        left.max().x().max(right.max().x()) - left.x().min(right.x()),
        left.max().y().max(right.max().y()) - left.y().min(right.y()),
    )
}

const fn target_root(target: &DockTarget) -> RootId {
    match target {
        DockTarget::Center(target) | DockTarget::TabGap { target, .. } => target.root(),
        DockTarget::InnerEdge(target) | DockTarget::OuterEdge(target) => target.root(),
    }
}

#[derive(Debug, Error)]
pub(super) enum DropPreviewProjectionError {
    #[error(transparent)]
    FutureLayout(#[from] FutureLayoutProjectionError),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("drag payload item {item} vanished from the transaction candidate")]
    PayloadItemMissing { item: crate::ids::ItemId },
    #[error(
        "candidate root {root} omitted the complete roster of drag payload group {source_node:?}"
    )]
    PayloadGroupMissing { root: RootId, source_node: NodeId },
    #[error("candidate projection omitted drag payload node {root}/{node:?}")]
    PayloadProjectionMissing { root: RootId, node: NodeId },
    #[error("candidate projection omitted center target node {root}/{node:?}")]
    TargetProjectionMissing { root: RootId, node: NodeId },
    #[error("candidate center target {root}/{node:?} is not a Tabs container")]
    ProjectedCenterIsNotTabs { root: RootId, node: NodeId },
    #[error("target presentation omitted retained layout facts")]
    TargetLayoutFactsMissing,
    #[error(transparent)]
    Geometry(#[from] GeometryError),
}
