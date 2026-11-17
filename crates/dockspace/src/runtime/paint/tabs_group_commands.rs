//! Exact tabs-group commands attached to the existing surface action pipeline.

use crate::model::{NativeTearOffSource, PreparedTabsGroup, ProductAction};
use crate::policy::TabBarInteraction;

use super::{
    DockspacePresentationCommandKind, DockspacePresentationCommandUnavailable,
    DockspaceRuntimeError, PresentationCommandPaintRecord, SurfacePaintPlan, TabBarPaintRecord,
};

/// Presentation actions scoped to one captured complete tabs group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DockspaceTabsGroupCommandKind {
    /// Float this group inside a logical surface, leaving sibling groups in place.
    Float,
    /// Move this group into a managed native child through first-live admission.
    MoveToNewWindow,
}

impl<'frame> SurfacePaintPlan<'frame> {
    /// Returns whether this exact group-menu anchor has authoritative visible area.
    #[must_use]
    pub fn tabs_group_menu_operable(self, bar: TabBarPaintRecord<'_>) -> bool {
        self.plan.tab_bar_records().iter().any(|record| {
            std::ptr::eq(record, bar.record)
                && !record.members().is_empty()
                && record.interaction() == TabBarInteraction::Enabled
                && record
                    .group_menu_bounds()
                    .is_some_and(|bounds| self.plan.region_is_operable(bounds, record.layer()))
        })
    }

    /// Prepares one atomic close request for this exact captured group.
    ///
    /// Membership, ownership and close policy are checked again by the shared
    /// close workflow. A veto preserves every member of the group.
    #[must_use]
    pub fn prepare_tabs_close(
        self,
        bar: TabBarPaintRecord<'_>,
    ) -> Option<super::super::PreparedSurfaceAction> {
        let group = self.tabs_group_for_menu(bar)?;
        self.candidate
            .tabs_close_is_available(&group)
            .then_some(())?;
        Some(super::super::PreparedSurfaceAction::close_tabs(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            group,
        ))
    }

    pub(super) fn tabs_group_for_menu(
        self,
        bar: TabBarPaintRecord<'_>,
    ) -> Option<PreparedTabsGroup> {
        self.tabs_group_menu_operable(bar).then_some(())?;
        let id = *bar.record.id();
        let source = self
            .candidate
            .workspace()
            .capture_node_source(id.root, id.tabs)
            .ok()?;
        Some(PreparedTabsGroup {
            authority_domain: self.authority_domain,
            expected: self.version,
            source,
        })
    }

    /// Borrows the group's own title, independently of its selected content.
    #[must_use]
    pub fn tabs_group_label(
        self,
        bar: TabBarPaintRecord<'_>,
    ) -> Option<crate::model::TabLabel<'frame>> {
        self.plan
            .tab_bar_records()
            .iter()
            .any(|record| std::ptr::eq(record, bar.record))
            .then_some(())?;
        self.plan
            .node_label(bar.record.id().root, bar.record.id().tabs)
    }

    /// Preflights center docking between two exact groups exposed by this output.
    ///
    /// Both menu modes use the same graph evaluator as pointer preview and commit.
    ///
    /// # Errors
    ///
    /// Returns an internal error only if core product preflight violates an invariant.
    pub fn tabs_group_center_command(
        self,
        source: TabBarPaintRecord<'_>,
        target: TabBarPaintRecord<'_>,
        mode: crate::model::TabGroupDockMode,
    ) -> Result<Option<PresentationCommandPaintRecord>, DockspaceRuntimeError> {
        let Some(group) = self.tabs_group_for_menu(source) else {
            return Ok(None);
        };
        if source.visual_id() == target.visual_id()
            || !self
                .plan
                .tab_bar_records()
                .iter()
                .any(|record| std::ptr::eq(record, target.record))
            || target.record.interaction() != TabBarInteraction::Enabled
            || !self
                .plan
                .region_is_operable(target.record.viewport(), target.record.layer())
        {
            return Ok(None);
        }
        let root = group.source.root();
        let action = ProductAction::DockTabs {
            group,
            mode,
            placement: crate::model::DockPlacement::Center(crate::model::DockAnchor::Node(
                crate::ids::NodeKey(target.record.id().tabs),
            )),
        };
        Ok(Some(
            match self.candidate.prepare_product_action_if_available(action)? {
                Ok(prepared) => {
                    let (domain, version, action) = prepared.into_parts();
                    PresentationCommandPaintRecord::ready(
                        domain,
                        version,
                        self.scene,
                        root,
                        DockspacePresentationCommandKind::DockCenter,
                        action,
                    )
                }
                Err(reason) => PresentationCommandPaintRecord::unavailable(
                    self.authority_domain,
                    self.version,
                    self.scene,
                    root,
                    DockspacePresentationCommandKind::DockCenter,
                    DockspacePresentationCommandUnavailable::Action(reason),
                ),
            },
        ))
    }

    /// Preflights one command for this exact current-frame group anchor.
    ///
    /// The returned record freezes the complete payload and workspace version.
    /// Submit through [`Self::prepare_presentation_command`]; never refresh an
    /// already displayed popup against a replacement group.
    ///
    /// # Errors
    ///
    /// Returns an internal error only if command preflight violates a core invariant.
    pub fn tabs_group_command(
        self,
        bar: TabBarPaintRecord<'_>,
        kind: DockspaceTabsGroupCommandKind,
    ) -> Result<Option<PresentationCommandPaintRecord>, DockspaceRuntimeError> {
        let Some(group) = self.tabs_group_for_menu(bar) else {
            return Ok(None);
        };
        let root = bar.root();
        let record_kind = match kind {
            DockspaceTabsGroupCommandKind::Float => DockspacePresentationCommandKind::Float,
            DockspaceTabsGroupCommandKind::MoveToNewWindow => {
                DockspacePresentationCommandKind::MoveToNewWindow
            }
        };
        let unavailable = |reason| {
            PresentationCommandPaintRecord::unavailable(
                self.authority_domain,
                self.version,
                self.scene,
                root,
                record_kind,
                DockspacePresentationCommandUnavailable::Action(reason),
            )
        };
        let action = match kind {
            DockspaceTabsGroupCommandKind::Float => {
                if matches!(
                    self.candidate.workspace().presentation_for_root(root),
                    Some(crate::graph::workspace::RootPresentationOwner::Contained { .. })
                ) && self
                    .candidate
                    .workspace()
                    .root(root)
                    .is_some_and(|record| record.node == bar.record.id().tabs)
                {
                    return Ok(Some(PresentationCommandPaintRecord::unavailable(
                        self.authority_domain,
                        self.version,
                        self.scene,
                        root,
                        record_kind,
                        DockspacePresentationCommandUnavailable::AlreadyContained,
                    )));
                }
                let surface = match self.candidate.native_main_recovery_surface(root) {
                    Ok(surface) => surface.unwrap_or(self.surface),
                    Err(reason) => return Ok(Some(unavailable(reason))),
                };
                ProductAction::FloatTabs { group, surface }
            }
            DockspaceTabsGroupCommandKind::MoveToNewWindow => {
                if self.candidate.root_is_bound_native_main(root)
                    && self
                        .candidate
                        .workspace()
                        .root(root)
                        .is_some_and(|record| record.node == bar.record.id().tabs)
                {
                    return Ok(Some(PresentationCommandPaintRecord::unavailable(
                        self.authority_domain,
                        self.version,
                        self.scene,
                        root,
                        record_kind,
                        DockspacePresentationCommandUnavailable::AlreadyNative,
                    )));
                }
                ProductAction::TearOffNative {
                    source: NativeTearOffSource::Tabs(group),
                    placement: None,
                }
            }
        };
        Ok(Some(
            match self.candidate.prepare_product_action_if_available(action)? {
                Ok(prepared) => {
                    let (domain, version, action) = prepared.into_parts();
                    PresentationCommandPaintRecord::ready(
                        domain,
                        version,
                        self.scene,
                        root,
                        record_kind,
                        action,
                    )
                }
                Err(reason) => unavailable(reason),
            },
        ))
    }
}
