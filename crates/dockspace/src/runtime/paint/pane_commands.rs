//! Exact visible-content menu actions share product and close transactions.

use crate::ids::NodeKey;
use crate::model::{
    DockAnchor, DockEdge, DockFraction, DockPlacement, DockspaceActionRejection,
    NativeTearOffSource, ProductAction,
};

use super::{
    DockspacePresentationCommandKind as Kind, DockspacePresentationCommandUnavailable,
    DockspaceRuntimeError, PanePaintRecord, PresentationCommandPaintRecord, SurfacePaintPlan,
};

impl SurfacePaintPlan<'_> {
    /// Preflights an action for one exact visible application content placement.
    ///
    /// This addresses only the real item, including a bare Pane inside a Split
    /// or Grid. It never widens the action to an enclosing layout tab or root.
    /// Submit a ready result through [`Self::prepare_presentation_command`].
    ///
    /// # Errors
    ///
    /// Returns an internal error when product preflight violates an invariant.
    pub fn pane_command(
        self,
        pane: PanePaintRecord<'_>,
        kind: Kind,
    ) -> Result<Option<PresentationCommandPaintRecord>, DockspaceRuntimeError> {
        if !self.pane_menu_is_operable(pane) {
            return Ok(None);
        }
        let item = pane.item();
        let root = pane.root();
        let unavailable = |reason| {
            PresentationCommandPaintRecord::unavailable(
                self.authority_domain,
                self.version,
                self.scene,
                root,
                kind,
                DockspacePresentationCommandUnavailable::Action(reason),
            )
        };
        let recovery_surface = match self.candidate.native_main_recovery_surface(root) {
            Ok(surface) => surface,
            Err(reason) => return Ok(Some(unavailable(reason))),
        };
        if kind == Kind::DockBack
            && recovery_surface.is_none()
            && !matches!(
                self.candidate.workspace().presentation_for_root(root),
                Some(crate::graph::workspace::RootPresentationOwner::Contained { .. })
            )
        {
            return Ok(Some(unavailable(
                DockspaceActionRejection::DockBackUnavailable { root },
            )));
        }
        let surface = recovery_surface.unwrap_or(self.surface);
        let action = match kind {
            Kind::Float => ProductAction::FloatItem { item, surface },
            Kind::MoveToNewWindow => ProductAction::TearOffNative {
                source: NativeTearOffSource::Item(item),
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
                let Some(placement) = self.pane_dock_placement(pane, surface, edge) else {
                    return Ok(Some(unavailable(DockspaceActionRejection::Conflict)));
                };
                ProductAction::DockItem { item, placement }
            }
        };
        Ok(Some(
            match self.candidate.prepare_product_action_if_available(action)? {
                Ok(prepared) => {
                    let (domain, version, action) = prepared.into_parts();
                    PresentationCommandPaintRecord::ready(
                        domain, version, self.scene, root, kind, action,
                    )
                }
                Err(reason) => unavailable(reason),
            },
        ))
    }

    /// Prepares the ordinary close-decision workflow for this exact content.
    ///
    /// Returns `None` for inactive, occluded, foreign-plan or nonclosable content.
    #[must_use]
    pub fn prepare_pane_close(
        self,
        pane: PanePaintRecord<'_>,
    ) -> Option<crate::runtime::PreparedSurfaceAction> {
        self.pane_menu_is_operable(pane).then_some(())?;
        self.candidate
            .item_close_is_available(pane.item())
            .then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::close(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::input::intent::CloseSceneTarget::Pane(pane.record.id()),
        ))
    }

    fn pane_menu_is_operable(self, pane: PanePaintRecord<'_>) -> bool {
        self.plan
            .pane_records()
            .iter()
            .any(|row| std::ptr::eq(row, pane.record))
            && pane.content_bounds().width() > 0.0
            && pane.content_bounds().height() > 0.0
            && self
                .plan
                .region_is_operable(pane.content_bounds(), pane.record.layer())
            && self.plan.receiver_active_path_is_current(
                crate::presentation::hit::PresentationHitRegionKind::PaneBody(pane.record.id()),
                self.candidate.workspace(),
            )
    }

    fn pane_dock_placement(
        self,
        pane: PanePaintRecord<'_>,
        surface: crate::ids::SurfaceId,
        edge: Option<DockEdge>,
    ) -> Option<DockPlacement> {
        let workspace = self.candidate.workspace();
        let Some(root) = workspace.surface(surface)?.main_root else {
            return Some(DockPlacement::Main(surface));
        };
        if let Some(edge) = edge {
            let target = workspace.root(root)?;
            if root == pane.root()
                && workspace
                    .items_in_subtree(target.node)
                    .all(|item| item == pane.item())
            {
                return None;
            }
            return Some(DockPlacement::OuterEdge {
                root,
                edge,
                fraction: DockFraction::new(self.plan.layout_facts()?.config().dock_fraction())
                    .ok()?,
            });
        }
        self.plan
            .tab_bar_records()
            .iter()
            .filter(|bar| bar.id().root == root && bar.id().tabs != pane.record.owner_node())
            .map(|bar| bar.id().tabs)
            .chain(
                self.plan
                    .pane_records()
                    .iter()
                    .filter(|row| {
                        row.id().root == root && row.owner_node() != pane.record.owner_node()
                    })
                    .map(|row| row.owner_node()),
            )
            .next()
            .map(|node| DockPlacement::Center(DockAnchor::Node(NodeKey(node))))
    }
}
