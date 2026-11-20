//! Direct-tab actions reuse the same product preflight and affine submission as root menus.

use crate::ids::NodeKey;
use crate::model::{
    DockAnchor, DockEdge, DockFraction, DockMoveSource, DockPlacement, DockspaceActionRejection,
    PreparedTab, ProductAction,
};

use super::{
    DockspacePresentationCommandKind as Kind, DockspacePresentationCommandUnavailable,
    DockspaceRuntimeError, PresentationCommandPaintRecord, SurfacePaintPlan, TabPaintRecord,
};

impl SurfacePaintPlan<'_> {
    /// Preflights an action on this exact visible direct tab.
    ///
    /// A layout tab moves its complete subtree. This never substitutes one of
    /// its descendants or widens the action to the containing tabs group.
    /// Submit a ready result through [`Self::prepare_presentation_command`].
    ///
    /// # Errors
    ///
    /// Returns an internal error if core product preflight violates an invariant.
    pub fn tab_command(
        self,
        tab: TabPaintRecord<'_>,
        kind: Kind,
    ) -> Result<Option<PresentationCommandPaintRecord>, DockspaceRuntimeError> {
        if !tab.belongs_to(&self) || !tab.operable() {
            return Ok(None);
        }
        let id = *tab.record.id();
        let workspace = self.candidate.workspace();
        let Ok(source) = workspace.capture_tab_source(id.root, id.tabs, id.tab) else {
            return Ok(None);
        };
        let captured = PreparedTab {
            authority_domain: self.authority_domain,
            expected: self.version,
            source,
        };
        let unavailable = |reason| {
            PresentationCommandPaintRecord::unavailable(
                self.authority_domain,
                self.version,
                self.scene,
                id.root,
                kind,
                DockspacePresentationCommandUnavailable::Action(reason),
            )
        };
        let recovery_surface = match self.candidate.native_main_recovery_surface(id.root) {
            Ok(surface) => surface,
            Err(reason) => return Ok(Some(unavailable(reason))),
        };
        if kind == Kind::DockBack
            && recovery_surface.is_none()
            && !matches!(
                workspace.presentation_for_root(id.root),
                Some(crate::graph::workspace::RootPresentationOwner::Contained { .. })
            )
        {
            return Ok(Some(unavailable(
                DockspaceActionRejection::DockBackUnavailable { root: id.root },
            )));
        }
        let surface = recovery_surface.unwrap_or(self.surface);
        let action = match kind {
            Kind::Float => ProductAction::FloatTab {
                tab: captured,
                surface,
            },
            Kind::MoveToNewWindow => ProductAction::TearOffNative {
                source: DockMoveSource::Tab(captured),
                placement: None,
            },
            _ => {
                let edge = match kind {
                    Kind::DockLeft => Some(DockEdge::Left),
                    Kind::DockRight => Some(DockEdge::Right),
                    Kind::DockTop => Some(DockEdge::Top),
                    Kind::DockBottom => Some(DockEdge::Bottom),
                    _ => None,
                };
                let placement = match self.direct_tab_dock_placement(&captured, surface, edge) {
                    Some(placement) => placement,
                    None => return Ok(Some(unavailable(DockspaceActionRejection::Conflict))),
                };
                ProductAction::DockTab {
                    tab: captured,
                    placement,
                }
            }
        };
        Ok(Some(
            match self.candidate.prepare_product_action_if_available(action)? {
                Ok(prepared) => {
                    let (domain, version, action, _) = prepared.into_parts();
                    PresentationCommandPaintRecord::ready(
                        domain, version, self.scene, id.root, kind, action,
                    )
                }
                Err(reason) => unavailable(reason),
            },
        ))
    }

    fn direct_tab_dock_placement(
        self,
        tab: &PreparedTab,
        surface: crate::ids::SurfaceId,
        edge: Option<DockEdge>,
    ) -> Option<DockPlacement> {
        let workspace = self.candidate.workspace();
        let root = workspace.surface(surface)?.main_root;
        let Some(root) = root else {
            return Some(DockPlacement::Main(surface));
        };
        let target = workspace.root(root)?;
        if let Some(edge) = edge {
            let only_payload = root == tab.source.root()
                && workspace
                    .collect_items_in_subtree(target.node)
                    .iter()
                    .all(|item| {
                        tab.key().item() == Some(*item)
                            || tab.key().layout().is_some_and(|node| {
                                workspace.locate_item_by_id(*item).is_some_and(|place| {
                                    workspace.subtree_contains(node, place.owner_node())
                                })
                            })
                    });
            if only_payload {
                return None;
            }
            return Some(DockPlacement::OuterEdge {
                root,
                edge,
                fraction: DockFraction::new(self.plan.layout_facts()?.config().dock_fraction())
                    .ok()?,
            });
        }
        let eligible = |node| {
            node != tab.source.tabs()
                && !tab
                    .key()
                    .layout()
                    .is_some_and(|source| workspace.subtree_contains(source, node))
        };
        self.plan
            .tab_bar_records()
            .iter()
            .filter(|bar| bar.id().root == root && eligible(bar.id().tabs))
            .map(|bar| bar.id().tabs)
            .chain(
                self.plan
                    .pane_records()
                    .iter()
                    .filter(|pane| pane.id().root == root && eligible(pane.owner_node()))
                    .map(|pane| pane.owner_node()),
            )
            .next()
            .map(|node| DockPlacement::Center(DockAnchor::Node(NodeKey(node))))
    }
}
