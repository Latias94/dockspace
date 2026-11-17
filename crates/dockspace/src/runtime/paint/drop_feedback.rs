//! Read-only destination facts from an already resolved docking preview.

use crate::geometry::LogicalRect;
use crate::graph::command::{Edge, MovePayload};
use crate::graph::{Node, Workspace};
use crate::ids::{ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::interaction::targeting::targets::DropTargetId;
use crate::model::DockEdge;
use crate::presentation::scene::{PaneRecord, PaneSceneId, PresentationPlan, TabBarSceneId};

use super::{DockspaceVisualId, VisualIdentity};

#[cfg(test)]
mod tests;

/// Current presented region addressed by one resolved docking action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockspaceDropTargetScope {
    /// Merge with or split one presented Pane or Tabs region.
    Pane,
    /// Split the complete root at an outer edge.
    Root,
    /// Install a main root on an empty logical surface.
    Surface,
}

/// Semantic result of the already resolved target, without command authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockspaceDropAction {
    /// Merge the payload into existing Tabs or promote a bare Pane to Tabs.
    Merge,
    /// Insert at an exact gap in the target's persistent tab sequence.
    InsertTab {
        /// Durable gap index, including hidden members, before removing the source payload.
        index: usize,
        /// Nearest preceding visible target item after excluding the complete payload.
        previous: Option<TabKey>,
        /// Nearest following visible target item after excluding the complete payload.
        next: Option<TabKey>,
    },
    /// Split the target region; its scope distinguishes inner from outer docking.
    Split {
        /// Physical side receiving the payload.
        edge: DockEdge,
    },
    /// Install the payload as the main root of a rootless logical surface.
    InstallRoot,
}

/// Ephemeral destination feedback bound to one exact drag preview.
///
/// Bounds and selection describe the currently presented destination, before
/// source removal. The preview's separate visual describes the resulting layout.
/// This value carries no input, mutation or presentation acknowledgement authority.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockspaceDropTargetFeedback {
    target: DropTargetId,
    content: Option<TargetContent>,
    bounds: LogicalRect,
    action: DockspaceDropAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetContent {
    Tab(TabKey),
    Pane(ItemId),
}

impl DockspaceDropTargetFeedback {
    /// Returns the target's logical surface.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.target.surface()
    }

    /// Returns the existing destination root, absent for a rootless surface.
    #[must_use]
    pub const fn root(self) -> Option<RootId> {
        self.target.root()
    }

    /// Returns whether the destination is a pane, root or empty surface.
    #[must_use]
    pub const fn scope(self) -> DockspaceDropTargetScope {
        match self.target {
            DropTargetId::Center { .. }
            | DropTargetId::TabGap { .. }
            | DropTargetId::InnerEdge { .. } => DockspaceDropTargetScope::Pane,
            DropTargetId::OuterEdge { .. } => DockspaceDropTargetScope::Root,
            DropTargetId::SurfaceBackground { .. } => DockspaceDropTargetScope::Surface,
        }
    }

    /// Returns the stable opaque identity of the resolved destination.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::DropTarget(self.target))
    }

    /// Returns the selected direct tab of the presented Tabs target before the move.
    ///
    /// Bare Panes, empty central Tabs, complete-root edges and empty surfaces
    /// have no selected tab. A bare Pane does not borrow its ancestor's selection.
    #[must_use]
    pub const fn selected_tab(self) -> Option<TabKey> {
        match self.content {
            Some(TargetContent::Tab(tab)) => Some(tab),
            Some(TargetContent::Pane(_)) | None => None,
        }
    }

    /// Returns the bare Pane's item or the directly selected content tab's item.
    ///
    /// A selected layout tab has no representative descendant item. Empty Tabs,
    /// complete-root edges and empty surfaces also return `None`.
    #[must_use]
    pub const fn selected_item(self) -> Option<ItemId> {
        match self.content {
            Some(TargetContent::Pane(item)) => Some(item),
            Some(TargetContent::Tab(tab)) => tab.item(),
            None => None,
        }
    }

    /// Returns the current visible pane or root bounds, not the future preview rectangle.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.bounds
    }

    /// Returns the merge, insertion, split or root-install action selected by core.
    #[must_use]
    pub const fn action(self) -> DockspaceDropAction {
        self.action
    }
}

pub(super) fn resolve(
    plan: &PresentationPlan,
    target: DropTargetId,
    payload: Option<&MovePayload>,
    workspace: &Workspace,
) -> Option<DockspaceDropTargetFeedback> {
    plan.drop_target(target)?;
    let (node, action) = match target {
        DropTargetId::Center { root, tabs, .. } => (Some((root, tabs)), DockspaceDropAction::Merge),
        DropTargetId::TabGap {
            root, tabs, index, ..
        } => {
            let id = TabBarSceneId { root, tabs };
            let bars = plan.tab_bar_records();
            bars.binary_search_by_key(&id, |bar| *bar.id()).ok()?;
            let Node::Tabs { items, .. } = workspace.node(tabs)? else {
                return None;
            };
            let excludes = |item: TabKey| match payload {
                Some(MovePayload::Item(source)) => item.item() == Some(source.item()),
                Some(MovePayload::Tabs(source) | MovePayload::Subtree(source)) => {
                    // A composed payload can also be one direct Layout tab
                    // in this destination's roster, including an in-place reorder.
                    source.root() == root
                        && (source.node() == tabs || item.layout() == Some(source.node()))
                }
                None => false,
            };
            // Scene members omit hidden items, while gap identities retain
            // their pre-removal position in the full persistent roster.
            let previous = items
                .get(..index)?
                .iter()
                .rev()
                .copied()
                .find(|item| workspace.tab_is_visible(*item) && !excludes(*item));
            let next = items
                .get(index..)?
                .iter()
                .copied()
                .find(|item| workspace.tab_is_visible(*item) && !excludes(*item));
            (
                Some((root, tabs)),
                DockspaceDropAction::InsertTab {
                    index,
                    previous,
                    next,
                },
            )
        }
        DropTargetId::InnerEdge {
            root, node, edge, ..
        } => (
            Some((root, node)),
            DockspaceDropAction::Split {
                edge: public_edge(edge),
            },
        ),
        DropTargetId::OuterEdge { edge, .. } => (
            None,
            DockspaceDropAction::Split {
                edge: public_edge(edge),
            },
        ),
        DropTargetId::SurfaceBackground { .. } => (None, DockspaceDropAction::InstallRoot),
    };
    let (content, bounds) = match node {
        Some((root, node)) => presented_content(plan, root, node)?,
        None => (
            None,
            match target.root() {
                Some(root) => plan.layout_facts()?.root(root)?.bounds(),
                None => plan.bounds(),
            },
        ),
    };
    Some(DockspaceDropTargetFeedback {
        target,
        content,
        bounds,
        action,
    })
}

fn presented_content(
    plan: &PresentationPlan,
    root: RootId,
    node: NodeId,
) -> Option<(Option<TargetContent>, LogicalRect)> {
    // Use the presented projection's owner, not the current workspace or a containing bar.
    let item = plan
        .layout_facts()
        .and_then(|facts| facts.root(root)?.projection().nodes.pane_item(&node));
    if let Some(item) = item {
        let id = PaneSceneId { root, item };
        let panes = plan.pane_records();
        let pane = &panes[panes.binary_search_by_key(&id, PaneRecord::id).ok()?];
        return (pane.owner_node() == node)
            .then_some((Some(TargetContent::Pane(item)), pane.bounds()));
    }
    let id = TabBarSceneId { root, tabs: node };
    let tabs = plan.tabs_records();
    let tabs = &tabs[tabs.binary_search_by_key(&id, |tabs| tabs.id()).ok()?];
    Some((
        plan.selected_in_tabs(node).map(TargetContent::Tab),
        tabs.bounds(),
    ))
}

const fn public_edge(edge: Edge) -> DockEdge {
    match edge {
        Edge::Left => DockEdge::Left,
        Edge::Right => DockEdge::Right,
        Edge::Top => DockEdge::Top,
        Edge::Bottom => DockEdge::Bottom,
    }
}
