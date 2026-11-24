//! Prepared keyboard and menu actions tied to the current paint plan.

use super::*;

impl<'frame> SurfacePaintPlan<'frame> {
    /// Returns one root-level presentation command frozen from this exact candidate.
    ///
    /// A root owned by another surface is not part of this plan and returns `None`.
    ///
    /// # Errors
    ///
    /// Returns an internal runtime error only when core command preflight violates an invariant.
    pub fn presentation_command(
        self,
        root: RootId,
        kind: DockspacePresentationCommandKind,
    ) -> Result<Option<PresentationCommandPaintRecord>, DockspaceRuntimeError> {
        let owner = match self.candidate.workspace().presentation_for_root(root) {
            Some(owner) => owner,
            None => return Ok(None),
        };
        let owner_surface = match owner {
            crate::graph::workspace::RootPresentationOwner::Main { surface }
            | crate::graph::workspace::RootPresentationOwner::Contained { surface, .. } => surface,
        };
        if owner_surface != self.surface {
            return Ok(None);
        }

        let unavailable =
            |reason| PresentationCommandPaintRecord::unavailable(self, root, kind, reason);
        if kind == DockspacePresentationCommandKind::Float
            && matches!(
                owner,
                crate::graph::workspace::RootPresentationOwner::Contained { .. }
            )
        {
            return Ok(Some(unavailable(
                DockspacePresentationCommandUnavailable::AlreadyContained,
            )));
        }
        if kind == DockspacePresentationCommandKind::MoveToNewWindow
            && self.candidate.root_is_bound_native_main(root)
        {
            return Ok(Some(unavailable(
                DockspacePresentationCommandUnavailable::AlreadyNative,
            )));
        }

        let action = match kind {
            DockspacePresentationCommandKind::Float => {
                let target_surface = match self.candidate.native_main_recovery_surface(root) {
                    Ok(Some(surface)) => surface,
                    Ok(None) => self.surface,
                    Err(reason) => {
                        return Ok(Some(unavailable(
                            DockspacePresentationCommandUnavailable::Action(reason),
                        )));
                    }
                };
                let rect = if target_surface == self.surface {
                    match self.candidate.default_contained_float_rect(
                        root,
                        target_surface,
                        self.plan,
                    ) {
                        Ok(rect) => Some(rect),
                        Err(reason) => {
                            return Ok(Some(unavailable(
                                DockspacePresentationCommandUnavailable::Action(reason),
                            )));
                        }
                    }
                } else {
                    None
                };
                ProductAction::FloatRoot {
                    root,
                    surface: target_surface,
                    rect,
                }
            }
            DockspacePresentationCommandKind::DockBack => ProductAction::DockBackRoot { root },
            DockspacePresentationCommandKind::DockLeft
            | DockspacePresentationCommandKind::DockRight
            | DockspacePresentationCommandKind::DockTop
            | DockspacePresentationCommandKind::DockBottom
            | DockspacePresentationCommandKind::DockCenter => {
                let edge = match kind {
                    DockspacePresentationCommandKind::DockLeft => {
                        Some(crate::model::DockEdge::Left)
                    }
                    DockspacePresentationCommandKind::DockRight => {
                        Some(crate::model::DockEdge::Right)
                    }
                    DockspacePresentationCommandKind::DockTop => Some(crate::model::DockEdge::Top),
                    DockspacePresentationCommandKind::DockBottom => {
                        Some(crate::model::DockEdge::Bottom)
                    }
                    DockspacePresentationCommandKind::DockCenter => None,
                    DockspacePresentationCommandKind::Float
                    | DockspacePresentationCommandKind::MoveToNewWindow
                    | DockspacePresentationCommandKind::DockBack => unreachable!(
                        "directional presentation commands are matched by the outer branch"
                    ),
                };
                let placement = match self.candidate.directional_dock_placement(root, edge) {
                    Ok(placement) => placement,
                    Err(reason) => {
                        return Ok(Some(unavailable(
                            DockspacePresentationCommandUnavailable::Action(reason),
                        )));
                    }
                };
                ProductAction::DockRoot {
                    root,
                    placement,
                    mode: crate::model::TabGroupDockMode::MergeTabs,
                }
            }
            DockspacePresentationCommandKind::MoveToNewWindow => ProductAction::TearOffNative {
                source: crate::model::DockMoveSource::Root(root),
                placement: None,
            },
        };
        let record = match self.candidate.prepare_product_action_if_available(action)? {
            Ok(prepared) => {
                let (authority_domain, version, action, _) = prepared.into_parts();
                debug_assert_eq!(authority_domain, self.authority_domain);
                debug_assert_eq!(version, self.version);
                PresentationCommandPaintRecord::ready(self, root, kind, action)
            }
            Err(reason) => unavailable(DockspacePresentationCommandUnavailable::Action(reason)),
        };
        Ok(Some(record))
    }

    /// Prepares activation of a ready presentation command from this exact output.
    #[must_use]
    pub fn prepare_presentation_command(
        self,
        record: PresentationCommandPaintRecord,
    ) -> Option<crate::runtime::PreparedSurfaceAction> {
        (record.output == self.semantic_output() && record.version == self.version).then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::presentation_command(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            record.action?,
        ))
    }

    /// Prepares a complete-root close request from an operable command-menu anchor.
    ///
    /// Returns `None` when the root is occluded, absent or disallows closing.
    #[must_use]
    pub fn prepare_root_close(self, root: RootId) -> Option<crate::runtime::PreparedSurfaceAction> {
        self.plan
            .presentation_menu_anchor_records()
            .iter()
            .find(|anchor| {
                anchor.root() == root
                    && anchor
                        .ready_bounds()
                        .is_some_and(|bounds| self.plan.region_is_operable(bounds, anchor.layer()))
            })?;
        self.candidate.root_close_is_available(root).then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::close_root(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            root,
        ))
    }

    /// Prepares a current-candidate tab selection without exposing scene identity.
    ///
    /// Activation requests the selected pane's content focus. Use
    /// [`Self::prepare_tab_navigation`] to preserve framework tab focus instead.
    #[must_use]
    pub fn prepare_tab_select(
        self,
        item: impl Into<TabKey>,
    ) -> Option<crate::runtime::PreparedSurfaceAction> {
        let tab = self.tab_record(item.into())?;
        self.tab_is_operable(tab).then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::select_tab(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabSelection::Activate(*tab.id()),
        ))
    }

    /// Prepares a close request for one visible tab close control.
    #[must_use]
    pub fn prepare_tab_close(
        self,
        item: impl Into<TabKey>,
    ) -> Option<crate::runtime::PreparedSurfaceAction> {
        let tab = self.tab_record(item.into())?;
        let close_bounds = tab.close_bounds()?;
        self.plan
            .region_is_operable(close_bounds, tab.layer())
            .then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::close(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::input::intent::CloseSceneTarget::Tab(*tab.id()),
        ))
    }

    /// Prepares a close request from an independent activation of a visible tab body.
    ///
    /// Hosts can use this for middle-click closing, including partially clipped
    /// tabs whose close button is unavailable. The body must remain operable and
    /// the item's close policy must allow the ordinary close-decision workflow.
    /// This does not grant authority to activate a missing close button.
    #[must_use]
    pub fn prepare_tab_body_close(
        self,
        item: impl Into<TabKey>,
    ) -> Option<crate::runtime::PreparedSurfaceAction> {
        let tab = self.tab_record(item.into())?;
        self.tab_is_operable(tab).then_some(())?;
        self.candidate
            .tab_close_is_available(*tab.id())
            .then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::close(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::input::intent::CloseSceneTarget::TabBody(*tab.id()),
        ))
    }

    /// Prepares a close request for one visible contained-floating close control.
    #[must_use]
    pub fn prepare_contained_close(
        self,
        floating: FloatingPresentationId,
    ) -> Option<crate::runtime::PreparedSurfaceAction> {
        let record = self
            .plan
            .contained_records()
            .iter()
            .find(|record| record.floating() == floating)?;
        let close_bounds = record.close_bounds()?;
        self.plan
            .region_is_operable(close_bounds, record.layer())
            .then_some(())?;
        self.candidate
            .root_close_is_available(record.root())
            .then_some(())?;
        Some(crate::runtime::PreparedSurfaceAction::close(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::input::intent::CloseSceneTarget::Contained(floating),
        ))
    }
}
