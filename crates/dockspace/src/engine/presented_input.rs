//! Semantic admission of receivers from the output still visible to the host.

use super::*;
use crate::ids::NodeId;

impl DockEngine {
    pub(crate) fn presented_layering_is_current(&self, plan: &PresentationPlan) -> bool {
        self.workspace
            .surface(plan.surface())
            .is_some_and(|surface| {
                plan.contained_order()
                    .is_none_or(|order| order == surface.contained)
            })
    }

    pub(super) fn presented_track_weights_are_current(
        &self,
        root: RootId,
        node: NodeId,
        expected: &crate::graph::command::TrackWeights,
    ) -> bool {
        if let Some(update) = self.interaction.active_resize_view().and_then(|resize| {
            resize
                .updates()
                .iter()
                .find(|update| update.source().root() == root && update.source().node() == node)
        }) {
            return update.weights() == expected;
        }
        match (self.workspace.node(node), expected) {
            (
                Some(Node::Split { weights, .. }),
                crate::graph::command::TrackWeights::Split(expected),
            ) => weights == expected,
            (
                Some(Node::Grid {
                    shares: Some(shares),
                    ..
                }),
                crate::graph::command::TrackWeights::Grid(expected),
            ) => shares == expected,
            _ => false,
        }
    }

    pub(crate) fn presented_transient_geometry_is_current(&self, plan: &PresentationPlan) -> bool {
        self.presented_contained_geometry_is_current(plan)
            && plan.resize_projection().iter().all(|update| {
                self.presented_track_weights_are_current(
                    update.source().root(),
                    update.source().node(),
                    update.weights(),
                )
            })
    }

    pub(super) fn presented_contained_geometry_is_current(&self, plan: &PresentationPlan) -> bool {
        plan.contained_projection().is_none_or(|projection| {
            self.current_presented_contained_rect(projection.surface, projection.floating)
                == Some(projection.rect)
        })
    }

    fn current_presented_contained_rect(
        &self,
        surface: SurfaceId,
        floating: FloatingPresentationId,
    ) -> Option<LogicalRect> {
        self.contained_layout_projection(surface)
            .filter(|projection| projection.floating == floating)
            .map(|projection| projection.rect)
            .or_else(|| {
                self.workspace
                    .contained_floating(floating)
                    .map(|record| record.rect)
            })
    }
    pub(crate) fn presented_root_structure_is_current(
        &self,
        plan: &PresentationPlan,
        root: RootId,
    ) -> bool {
        if !self.presented_root_owner_is_current(plan, root) {
            return false;
        }
        plan.command_basis(root).is_some_and(|frozen| {
            self.workspace
                .root_fingerprint(root)
                .is_ok_and(|current| frozen.presentation_topology_eq(&current))
                && frozen.0.nodes.iter().all(|node| {
                    if let Some(update) = plan.resize_projection().iter().find(|update| {
                        update.source().root() == root && update.source().node() == node.id
                    }) {
                        return self.presented_track_weights_are_current(
                            root,
                            node.id,
                            update.weights(),
                        );
                    }
                    let active = self.interaction.active_resize_view().and_then(|resize| {
                        resize.updates().iter().find(|update| {
                            update.source().root() == root && update.source().node() == node.id
                        })
                    });
                    match &node.node {
                        crate::graph::command::FingerprintNode::Split { weight_bits, .. } => {
                            let weights =
                                active
                                    .and_then(TrackResize::split_weights)
                                    .or_else(|| match self.workspace.node(node.id) {
                                        Some(Node::Split { weights, .. }) => {
                                            Some(weights.as_slice())
                                        }
                                        _ => None,
                                    });
                            weights.is_some_and(|weights| {
                                weights
                                    .iter()
                                    .map(|weight| weight.get().to_bits())
                                    .eq(weight_bits.iter().copied())
                            })
                        }
                        crate::graph::command::FingerprintNode::Grid { share_bits, .. } => {
                            let shares = active.and_then(TrackResize::grid_shares).or_else(|| {
                                match self.workspace.node(node.id) {
                                    Some(Node::Grid { shares, .. }) => shares.as_ref(),
                                    _ => None,
                                }
                            });
                            match (share_bits, shares) {
                                (None, None) => true,
                                (Some((rows, columns)), Some(shares)) => {
                                    shares.rows().map(f64::to_bits).eq(rows.iter().copied())
                                        && shares
                                            .columns()
                                            .map(f64::to_bits)
                                            .eq(columns.iter().copied())
                                }
                                _ => false,
                            }
                        }
                        _ => true,
                    }
                })
        })
    }

    pub(super) fn presented_root_topology_is_current(
        &self,
        plan: &PresentationPlan,
        root: RootId,
    ) -> bool {
        self.presented_root_owner_is_current(plan, root)
            && plan.command_basis(root).is_some_and(|frozen| {
                self.workspace
                    .root_fingerprint(root)
                    .is_ok_and(|current| frozen.presentation_topology_eq(&current))
            })
    }

    fn presented_root_owner_is_current(&self, plan: &PresentationPlan, root: RootId) -> bool {
        match self.workspace.presentation_for_root(root) {
            Some(crate::graph::workspace::RootPresentationOwner::Main { surface }) => {
                surface == plan.surface()
                    && !plan
                        .contained_records()
                        .iter()
                        .any(|record| record.root() == root)
            }
            Some(crate::graph::workspace::RootPresentationOwner::Contained {
                surface,
                floating,
            }) => {
                surface == plan.surface()
                    && plan
                        .contained_record(floating)
                        .is_some_and(|record| record.root() == root)
                    && plan.unclipped_contained_bounds(floating)
                        == self.current_presented_contained_rect(surface, floating)
            }
            None => false,
        }
    }

    pub(crate) fn presented_receiver_is_current(
        &self,
        plan: &PresentationPlan,
        region: PresentationHitRegionId,
    ) -> bool {
        if !self.presented_layering_is_current(plan)
            || !plan.receiver_active_path_is_current(region.kind(), &self.workspace)
        {
            return false;
        }
        let tab_is_current = |tab: crate::presentation::scene::TabSceneId| {
            self.workspace.visible_tab_keys(tab.tabs).contains(&tab.tab)
                && self
                    .workspace
                    .capture_tab_source(tab.root, tab.tabs, tab.tab)
                    .is_ok()
                && self.presented_root_owner_is_current(plan, tab.root)
        };
        let bar_is_current = |bar: crate::presentation::scene::TabBarSceneId| {
            let Some(record) = plan
                .tab_bar_records()
                .iter()
                .find(|record| *record.id() == bar)
            else {
                return false;
            };
            matches!(self.workspace.node(bar.tabs), Some(Node::Tabs { .. }))
                && self
                    .workspace
                    .visible_tab_keys(bar.tabs)
                    .iter()
                    .copied()
                    .eq(record.members().iter().map(|member| member.tab().tab))
                && self.presented_root_owner_is_current(plan, bar.root)
        };
        match region.kind() {
            PresentationHitRegionKind::TabBody(tab)
            | PresentationHitRegionKind::TabClose(tab)
            | PresentationHitRegionKind::TabListMenuRow { tab, .. } => tab_is_current(tab),
            PresentationHitRegionKind::TabsBackground(bar)
            | PresentationHitRegionKind::TabGroupDrag { bar, .. }
            | PresentationHitRegionKind::TabStripScroll(bar) => bar_is_current(bar),
            PresentationHitRegionKind::PaneBody(pane) => {
                self.presented_root_owner_is_current(plan, pane.root)
                    && plan
                        .pane_records()
                        .iter()
                        .find(|record| record.id() == pane)
                        .is_some_and(|record| {
                            self.workspace.item_is_active(pane.item)
                                && self.workspace.locate_item_by_id(pane.item).is_some_and(
                                    |location| {
                                        location.root == pane.root
                                            && location.owner_node() == record.owner_node()
                                    },
                                )
                        })
            }
            PresentationHitRegionKind::SplitterHandle(splitter) => {
                plan.partition_available(splitter.root)
                    && self.presented_root_owner_is_current(plan, splitter.root)
                    && self.presented_root_structure_is_current(plan, splitter.root)
            }
            PresentationHitRegionKind::SplitterJunction(junction) => {
                junction.splitters().iter().all(|splitter| {
                    plan.partition_available(splitter.root)
                        && plan.receiver_active_path_is_current(
                            PresentationHitRegionKind::SplitterHandle(*splitter),
                            &self.workspace,
                        )
                        && self.presented_root_owner_is_current(plan, splitter.root)
                        && self.presented_root_structure_is_current(plan, splitter.root)
                })
            }
            PresentationHitRegionKind::ContainedTitle(floating)
            | PresentationHitRegionKind::ContainedClose(floating)
            | PresentationHitRegionKind::ContainedResize { floating, .. }
            | PresentationHitRegionKind::ContainedFrameBlocker(floating) => {
                plan.contained_record(floating).is_some_and(|record| {
                    self.presented_root_owner_is_current(plan, record.root())
                        && self.presented_root_structure_is_current(plan, record.root())
                })
            }
            PresentationHitRegionKind::DropGuideActivation(cluster) => cluster.root().map_or_else(
                || {
                    self.workspace
                        .surface(cluster.surface_id())
                        .is_some_and(|presentation| presentation.main_root.is_none())
                },
                |root| self.presented_root_structure_is_current(plan, root),
            ),
            PresentationHitRegionKind::DropTarget(target) => match target {
                crate::interaction::targeting::targets::DropTargetId::SurfaceBackground {
                    surface,
                } => self
                    .workspace
                    .surface(surface)
                    .is_some_and(|presentation| presentation.main_root.is_none()),
                crate::interaction::targeting::targets::DropTargetId::TabGap { root, .. }
                | crate::interaction::targeting::targets::DropTargetId::Center { root, .. }
                | crate::interaction::targeting::targets::DropTargetId::InnerEdge {
                    root, ..
                }
                | crate::interaction::targeting::targets::DropTargetId::OuterEdge {
                    root, ..
                } => self.presented_root_structure_is_current(plan, root),
            },
            PresentationHitRegionKind::TabStripControl(TabStripControlId::RevealTabBar(bar)) => {
                self.presented_root_owner_is_current(plan, bar.root)
                    && self.workspace.visible_tab_keys(bar.tabs).len() == 1
                    && self.workspace.visible_tab_selection(bar.tabs)
                        == plan.selected_in_tabs(bar.tabs)
            }
            PresentationHitRegionKind::TabStripControl(control) => bar_is_current(control.bar()),
            PresentationHitRegionKind::TabListMenuScroll(_)
            | PresentationHitRegionKind::TabListMenuBlocker(_)
            | PresentationHitRegionKind::TabListMenuBackdrop(_) => true,
        }
    }
}
