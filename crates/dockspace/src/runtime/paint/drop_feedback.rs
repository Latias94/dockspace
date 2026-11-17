//! Read-only destination facts from an already resolved docking preview.

use crate::geometry::LogicalRect;
use crate::graph::command::{Edge, MovePayload};
use crate::graph::{Node, Workspace};
use crate::ids::{ItemId, RootId, SurfaceId, TabKey};
use crate::interaction::targeting::targets::DropTargetId;
use crate::model::DockEdge;
use crate::presentation::scene::{PresentationPlan, TabBarSceneId};

use super::{DockspaceVisualId, VisualIdentity};

#[cfg(test)]
mod tests;

/// Current presented region addressed by one resolved docking action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockspaceDropTargetScope {
    /// Merge with or split one tabs leaf.
    Pane,
    /// Split the complete root at an outer edge.
    Root,
    /// Install a main root on an empty logical surface.
    Surface,
}

/// Semantic result of the already resolved target, without command authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockspaceDropAction {
    /// Merge the payload into the target tabs stack.
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
    selected: Option<TabKey>,
    bounds: LogicalRect,
    action: DockspaceDropAction,
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

    /// Returns the item representing the presented pane, before the payload is selected.
    ///
    /// Empty central panes, complete-root edges and empty surfaces have no unique item.
    #[must_use]
    pub const fn selected_tab(self) -> Option<TabKey> {
        self.selected
    }

    /// Returns content only when the selected tab directly contains one item.
    #[must_use]
    pub const fn selected_item(self) -> Option<ItemId> {
        match self.selected {
            Some(tab) => tab.item(),
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
    let (pane, action) = match target {
        DropTargetId::Center { root, tabs, .. } => (
            Some(TabBarSceneId { root, tabs }),
            DockspaceDropAction::Merge,
        ),
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
                    // Successful tab-gap resolution rejects a target inside a split
                    // payload. Only a same-leaf group can overlap its destination.
                    source.root() == root && source.node() == tabs
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
                Some(TabBarSceneId { root, tabs }),
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
            Some(TabBarSceneId { root, tabs: node }),
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
    let (selected, bounds) = match pane {
        Some(id) => {
            let panes = plan.tabs_records();
            let pane = &panes[panes.binary_search_by_key(&id, |pane| pane.id()).ok()?];
            (plan.selected_in_tabs(id.tabs), pane.bounds())
        }
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
        selected,
        bounds,
        action,
    })
}

const fn public_edge(edge: Edge) -> DockEdge {
    match edge {
        Edge::Left => DockEdge::Left,
        Edge::Right => DockEdge::Right,
        Edge::Top => DockEdge::Top,
        Edge::Bottom => DockEdge::Bottom,
    }
}
