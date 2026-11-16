//! Exact tabs-group commands attached to the existing surface action pipeline.

use crate::model::{DockAnchor, NativeTearOffSource, PreparedTabsGroup, ProductAction};
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

impl SurfacePaintPlan<'_> {
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
        let item = bar.record.members().first()?.tab().item;
        self.candidate
            .capture_tabs_group(DockAnchor::Item(item))
            .ok()
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
