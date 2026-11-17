//! Core-owned renderer decoration for one active drag source.

use std::fmt;

use crate::geometry::LogicalRect;
use crate::graph::command::{FingerprintNode, MovePayload, NodeFingerprint, NodeSource};
use crate::ids::{FloatingPresentationId, ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::interaction::{ActiveDragView, DragPhase, DragSessionId};
use crate::presentation::scene::{
    ContainedLayoutOwner, PaneSceneId, PresentationPlan, SurfaceSceneSet, TabBarSceneId, TabSceneId,
};

use super::{DockspaceVisualId, VisualIdentity};

/// Renderer-facing semantic class of the content currently being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockspaceDragSourceKind {
    /// One tab item.
    Item,
    /// One complete tabs leaf, including its pane body.
    TabGroup,
    /// One proper structural subtree.
    Subtree,
    /// One complete root.
    Root,
}

/// Stable, renderer-neutral decoration facts for one core-owned active drag.
///
/// This value is intentionally independent from docking preview acknowledgement.
/// It exposes no graph node, scene stamp, preview token, or gesture session.
#[derive(Clone, Copy)]
pub struct DockspaceDragDecoration<'frame> {
    source_surface: SurfaceId,
    kind: DockspaceDragSourceKind,
    source_visual: DragSourceVisual,
    source_bounds: LogicalRect,
    projected_root_bounds: Option<LogicalRect>,
    source_is_projected: bool,
    item: Option<ItemId>,
    label: crate::graph::TabLabel<'frame>,
    label_key: TabKey,
    source: DragDecorationSource<'frame>,
}

impl fmt::Debug for DockspaceDragDecoration<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceDragDecoration")
            .field("source_surface", &self.source_surface)
            .field("kind", &self.kind)
            .field("source_visual", &self.source_visual())
            .field("source_bounds", &self.source_bounds)
            .field("source_is_projected", &self.source_is_projected)
            .field("item", &self.item)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

// A drag source needs only these four identities. Storing the general visual
// enum here makes every surface-program step pay for unrelated junction IDs.
#[derive(Clone, Copy)]
enum DragSourceVisual {
    Tab(TabSceneId),
    Pane(PaneSceneId),
    Tabs(TabBarSceneId),
    Contained(FloatingPresentationId),
    Subtree { root: RootId, node: NodeId },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragDecorationSource<'frame> {
    Item(TabSceneId),
    TabGroup {
        bar: TabBarSceneId,
        fingerprint: &'frame NodeFingerprint,
    },
    Subtree {
        root: crate::ids::RootId,
        node: NodeId,
        fingerprint: &'frame NodeFingerprint,
        complete_root: bool,
    },
}

impl<'frame> DockspaceDragDecoration<'frame> {
    /// Returns the surface which still presents the drag source.
    #[must_use]
    pub const fn source_surface(self) -> SurfaceId {
        self.source_surface
    }

    /// Returns the semantic class of the source payload.
    #[must_use]
    pub const fn kind(self) -> DockspaceDragSourceKind {
        self.kind
    }

    /// Returns an opaque stable identity for the decorated source visual.
    #[must_use]
    pub const fn source_visual(self) -> DockspaceVisualId {
        DockspaceVisualId(match self.source_visual {
            DragSourceVisual::Tab(tab) => VisualIdentity::Tab(tab),
            DragSourceVisual::Pane(pane) => VisualIdentity::Pane(pane),
            DragSourceVisual::Tabs(bar) => VisualIdentity::Tabs(bar),
            DragSourceVisual::Contained(floating) => VisualIdentity::Contained(floating),
            DragSourceVisual::Subtree { root, node } => VisualIdentity::DragSource { root, node },
        })
    }

    /// Returns the exact source-surface bounds represented by the decoration.
    #[must_use]
    pub const fn source_bounds(self) -> LogicalRect {
        self.source_bounds
    }

    /// Returns the full-window projection for a contained root drag.
    #[must_use]
    pub const fn projected_root_bounds(self) -> Option<LogicalRect> {
        self.projected_root_bounds
    }

    /// Returns whether the complete source tree is already painted at its projection.
    ///
    /// Adapters retain cursor feedback but must not paint a duplicate source ghost.
    #[must_use]
    pub const fn source_is_projected(self) -> bool {
        self.source_is_projected
    }

    /// Returns the dragged item when the source is one tab.
    #[must_use]
    pub const fn item(self) -> Option<ItemId> {
        self.item
    }

    /// Returns the source's own frozen label, independent of descendant selection.
    #[must_use]
    pub const fn label(self) -> crate::graph::TabLabel<'frame> {
        self.label
    }

    /// Returns the exact content or layout key used for label resources.
    #[must_use]
    pub const fn key(self) -> TabKey {
        self.label_key
    }

    /// Returns whether the active source decoration omits one opaque visual.
    ///
    /// Adapters retain the dock visual's response, receiver, and adapter-owned
    /// accessibility registration; only its paint is omitted. Pane adapters may
    /// render application content through a transparent painter to preserve widget
    /// identity while leaving a real visual gap. Structural membership remains
    /// core-owned and no graph identity crosses this boundary.
    #[must_use]
    pub(super) fn omits_visual(self, visual: DockspaceVisualId) -> bool {
        if self.source_is_projected {
            return false;
        }
        match self.source {
            DragDecorationSource::Item(tab) => {
                matches!(visual.0, VisualIdentity::Tab(candidate) if candidate == tab)
            }
            DragDecorationSource::TabGroup { bar, fingerprint } => {
                visual_belongs_to_subtree(visual, bar.root, bar.tabs, fingerprint, false)
            }
            DragDecorationSource::Subtree {
                root,
                node,
                fingerprint,
                complete_root,
            } => {
                visual == self.source_visual()
                    || visual_belongs_to_subtree(visual, root, node, fingerprint, complete_root)
            }
        }
    }
}

pub(super) fn resolve<'frame>(
    scenes: &'frame SurfaceSceneSet,
    drag: ActiveDragView<'frame>,
) -> Option<DockspaceDragDecoration<'frame>> {
    if drag.phase() != DragPhase::Dragging {
        return None;
    }
    let payload = drag.payload();
    let root = match payload {
        MovePayload::Item(source) => source.root(),
        MovePayload::Tabs(source) | MovePayload::Subtree(source) => source.root(),
    };
    let (source_surface, plan) = scenes.surfaces().find_map(|(surface, scene)| {
        let plan = scene.ready()?.candidate().plan();
        plan.command_basis(root).map(|_| (*surface, plan))
    })?;
    match payload {
        MovePayload::Item(source) => {
            let Some(tabs) = source.tabs() else {
                let pane = plan.pane_records().iter().find(|pane| {
                    pane.id()
                        == PaneSceneId {
                            root: source.root(),
                            item: source.item(),
                        }
                })?;
                return Some(DockspaceDragDecoration {
                    source_surface,
                    kind: DockspaceDragSourceKind::Item,
                    source_visual: DragSourceVisual::Pane(pane.id()),
                    source_bounds: pane.bounds(),
                    projected_root_bounds: None,
                    source_is_projected: false,
                    item: Some(source.item()),
                    label: crate::graph::TabLabel::Item(source.item()),
                    label_key: TabKey::for_item(source.item()),
                    source: DragDecorationSource::Subtree {
                        root: source.root(),
                        node: source.owner_node(),
                        fingerprint: plan.command_basis(source.root())?,
                        complete_root: false,
                    },
                });
            };
            let bar = plan.tab_bar_records().iter().find(|record| {
                let id = record.id();
                id.root == source.root() && id.tabs == tabs
            })?;
            // Strip members retain exact source geometry even after scrolling clips its paint.
            let member = bar
                .members()
                .iter()
                .find(|member| member.tab().tab.item() == Some(source.item()))?;
            let tab = member.tab();
            let projection = source_projection(plan, source_surface, source.root(), drag.session());
            Some(DockspaceDragDecoration {
                source_surface,
                kind: DockspaceDragSourceKind::Item,
                source_visual: DragSourceVisual::Tab(tab),
                source_bounds: member.full_bounds(),
                projected_root_bounds: projection,
                source_is_projected: projection.is_some(),
                item: Some(source.item()),
                label: crate::graph::TabLabel::Item(source.item()),
                label_key: TabKey::for_item(source.item()),
                source: DragDecorationSource::Item(tab),
            })
        }
        MovePayload::Tabs(source) => {
            let pane = plan.tabs_records().iter().find(|record| {
                let id = record.id();
                id.root == source.root() && id.tabs == source.node()
            })?;
            let projection = (source.node() == plan.command_basis(source.root())?.0.root_node)
                .then(|| source_projection(plan, source_surface, source.root(), drag.session()))
                .flatten();
            Some(DockspaceDragDecoration {
                source_surface,
                kind: DockspaceDragSourceKind::TabGroup,
                source_visual: DragSourceVisual::Tabs(pane.id()),
                source_bounds: pane.bounds(),
                projected_root_bounds: projection,
                source_is_projected: projection.is_some(),
                item: None,
                label: plan.node_label(source.root(), source.node())?,
                label_key: TabKey::for_layout(source.node()),
                source: DragDecorationSource::TabGroup {
                    bar: pane.id(),
                    fingerprint: plan.command_basis(source.root())?,
                },
            })
        }
        MovePayload::Subtree(source) => resolve_subtree(
            plan,
            source_surface,
            source,
            drag.session(),
            drag.contained_projection_rect(),
        ),
    }
}

fn source_projection(
    plan: &PresentationPlan,
    surface: SurfaceId,
    root: RootId,
    session: DragSessionId,
) -> Option<LogicalRect> {
    let projection = plan.contained_projection()?;
    (projection.owner == ContainedLayoutOwner::Drag(session)
        && projection.surface == surface
        && projection.root == root
        && plan
            .contained_records()
            .iter()
            .any(|record| record.root() == root && record.floating() == projection.floating))
    .then_some(projection.rect)
}

fn resolve_subtree<'frame>(
    plan: &'frame PresentationPlan,
    source_surface: SurfaceId,
    source: &'frame NodeSource,
    session: DragSessionId,
    pointer_projection: Option<LogicalRect>,
) -> Option<DockspaceDragDecoration<'frame>> {
    let complete_root = source.node() == plan.command_basis(source.root())?.0.root_node;
    let source_identity = DragDecorationSource::Subtree {
        root: source.root(),
        node: source.node(),
        fingerprint: plan.command_basis(source.root())?,
        complete_root,
    };
    let label = plan.node_label(source.root(), source.node())?;
    let label_key = match label {
        crate::graph::TabLabel::Item(item) => TabKey::for_item(item),
        crate::graph::TabLabel::Layout { .. } => TabKey::for_layout(source.node()),
    };
    if complete_root
        && let Some(contained) = plan
            .contained_records()
            .iter()
            .find(|record| record.root() == source.root())
    {
        let projection = source_projection(plan, source_surface, source.root(), session);
        return Some(DockspaceDragDecoration {
            source_surface,
            kind: DockspaceDragSourceKind::Root,
            source_visual: DragSourceVisual::Contained(contained.floating()),
            source_bounds: contained.outer_bounds(),
            projected_root_bounds: projection.or(pointer_projection),
            source_is_projected: projection.is_some(),
            item: None,
            label,
            label_key,
            source: source_identity,
        });
    }

    Some(DockspaceDragDecoration {
        source_surface,
        kind: if complete_root {
            DockspaceDragSourceKind::Root
        } else {
            DockspaceDragSourceKind::Subtree
        },
        source_visual: DragSourceVisual::Subtree {
            root: source.root(),
            node: source.node(),
        },
        source_bounds: subtree_bounds(plan, source)?,
        projected_root_bounds: None,
        source_is_projected: false,
        item: None,
        label,
        label_key,
        source: source_identity,
    })
}

fn visual_belongs_to_subtree(
    visual: DockspaceVisualId,
    root: crate::ids::RootId,
    source: NodeId,
    fingerprint: &NodeFingerprint,
    complete_root: bool,
) -> bool {
    match visual.0 {
        VisualIdentity::Pane(candidate) => {
            candidate.root == root
                && (complete_root || subtree_contains_item(fingerprint, source, candidate.item))
        }
        VisualIdentity::Tab(candidate) => {
            candidate.root == root
                && (complete_root || subtree_contains_node(fingerprint, source, candidate.tabs))
        }
        VisualIdentity::Tabs(candidate) | VisualIdentity::TabBar(candidate) => {
            candidate.root == root
                && (complete_root || subtree_contains_node(fingerprint, source, candidate.tabs))
        }
        VisualIdentity::TabGroupDragRegion { bar, .. } => {
            bar.root == root
                && (complete_root || subtree_contains_node(fingerprint, source, bar.tabs))
        }
        VisualIdentity::TabStripControl(control) => {
            let bar = control.bar();
            bar.root == root
                && (complete_root || subtree_contains_node(fingerprint, source, bar.tabs))
        }
        VisualIdentity::TabListMenu(session)
        | VisualIdentity::TabListMenuRow { session, .. }
        | VisualIdentity::TabListMenuBackdrop { session, .. } => {
            let bar = session.key().bar();
            bar.root == root
                && (complete_root || subtree_contains_node(fingerprint, source, bar.tabs))
        }
        VisualIdentity::PresentationMenuAnchor(candidate) => candidate == root && complete_root,
        VisualIdentity::Splitter(splitter) => {
            splitter.root == root
                && (complete_root || subtree_contains_node(fingerprint, source, splitter.node()))
        }
        VisualIdentity::SplitterJunction(junction) => {
            junction.root() == root
                && (complete_root
                    || junction.splitters().into_iter().all(|splitter| {
                        subtree_contains_node(fingerprint, source, splitter.node())
                    }))
        }
        VisualIdentity::DragSource {
            root: candidate,
            node,
        } => {
            candidate == root && (complete_root || subtree_contains_node(fingerprint, source, node))
        }
        VisualIdentity::Contained(_)
        | VisualIdentity::DropGuide(_)
        | VisualIdentity::DropTarget(_)
        | VisualIdentity::Receiver(_) => false,
    }
}

fn subtree_contains_node(fingerprint: &NodeFingerprint, source: NodeId, target: NodeId) -> bool {
    if source == target {
        return true;
    }
    let Some(record) = fingerprint
        .0
        .nodes
        .iter()
        .find(|record| record.id == source)
    else {
        return false;
    };
    match &record.node {
        FingerprintNode::Pane { .. } => false,
        FingerprintNode::Tabs { items, .. } => items
            .iter()
            .filter_map(|tab| tab.layout())
            .any(|child| subtree_contains_node(fingerprint, child, target)),
        FingerprintNode::Split { children, .. } | FingerprintNode::Grid { children, .. } => {
            children
                .iter()
                .copied()
                .any(|child| subtree_contains_node(fingerprint, child, target))
        }
    }
}

fn subtree_contains_item(fingerprint: &NodeFingerprint, source: NodeId, item: ItemId) -> bool {
    let Some(record) = fingerprint
        .0
        .nodes
        .iter()
        .find(|record| record.id == source)
    else {
        return false;
    };
    match &record.node {
        FingerprintNode::Pane { item: child, .. } => *child == item,
        FingerprintNode::Tabs { items, .. } => items.iter().any(|tab| {
            tab.item() == Some(item)
                || tab
                    .layout()
                    .is_some_and(|child| subtree_contains_item(fingerprint, child, item))
        }),
        FingerprintNode::Split { children, .. } | FingerprintNode::Grid { children, .. } => {
            children
                .iter()
                .any(|child| subtree_contains_item(fingerprint, *child, item))
        }
    }
}

fn subtree_bounds(plan: &PresentationPlan, source: &NodeSource) -> Option<LogicalRect> {
    plan.layout_facts()?
        .root(source.root())?
        .projection()
        .nodes
        .get(&source.node())
        .filter(|node| node.visible)
        .map(|node| node.rect)
}
