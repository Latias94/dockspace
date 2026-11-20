//! Tab-strip controls and popup paint records.

use std::sync::Arc;

use crate::engine::LocalTabChromeAction;
use crate::geometry::LogicalRect;
use crate::ids::{ItemId, RootId, SurfaceId, TabKey};
use crate::interaction::FrozenTabListMenuRowClick;
use crate::model::WorkspaceVersion;
use crate::presentation::scene::{
    SurfacePlanScene, TabListMenuBackdropRecord, TabListMenuRecord, TabListMenuRowRecord,
    TabStripControlRecord,
};
use crate::presentation::tab_strip::{TabListMenuSessionId, TabStripControlId};
use crate::runtime::{DeferredSurfaceAction, PreparedSurfaceAction, SurfaceTabListInput};

use super::{DockspacePaintLayer, DockspaceVisualId, SurfacePaintPlan, VisualIdentity};

/// Shared keyboard authority for one tab-list menu from an exact output.
///
/// Keep one copy with the rendered menu. Resolve a key only after the host
/// delivers it to that output. Core revalidates the deferred action on submit.
#[derive(Debug, Clone)]
pub struct RetainedTabListMenu {
    witness: Arc<SurfacePlanScene>,
    version: WorkspaceVersion,
    record: TabListMenuRecord,
    backdrop: TabListMenuBackdropRecord,
}

impl RetainedTabListMenu {
    /// Tests whether a later presented menu can continue this menu's keyboard input.
    ///
    /// The popup session, ordered roster and keyboard authority must match.
    /// The host still validates the expected workspace version and focus claim.
    #[must_use]
    pub fn can_continue_from(&self, previous: &Self) -> bool {
        self.record.session() == previous.record.session()
            && super::tab_input::same_keyboard_authority(&self.witness, &previous.witness)
            && self.record.rows().iter().map(|row| row.tab()).eq(previous
                .record
                .rows()
                .iter()
                .map(|row| row.tab()))
    }

    /// Resolves one input and returns its resulting row identity and action.
    ///
    /// The host supplies the current cursor or the destination of the previous
    /// key in a batch. The row may be scrolled out of view. Row identities from
    /// a different menu session are rejected.
    #[must_use]
    pub fn resolve(
        &self,
        row: DockspaceVisualId,
        input: SurfaceTabListInput,
    ) -> Option<(DockspaceVisualId, DeferredSurfaceAction)> {
        let session = self.record.session();
        let VisualIdentity::TabListMenuRow {
            session: owner,
            tab,
        } = row.0
        else {
            return None;
        };
        if owner != session {
            return None;
        }
        let rows = self.record.rows();
        let start = rows.iter().find(|row| row.tab() == tab)?;
        let target = match input {
            SurfaceTabListInput::Navigate(navigation) => {
                &rows[navigation.destination(start.ordinal(), rows.len() - 1)]
            }
            _ => start,
        };
        let revision = self.backdrop.revision();
        let action = match input {
            SurfaceTabListInput::Focus | SurfaceTabListInput::Navigate(_) => {
                LocalTabChromeAction::NavigateMenu {
                    session,
                    revision,
                    record: self.record.clone(),
                    target: target.tab().tab,
                }
            }
            SurfaceTabListInput::Activate => {
                LocalTabChromeAction::ActivateMenuRow(FrozenTabListMenuRowClick {
                    session,
                    record: *target,
                    revision,
                })
            }
            SurfaceTabListInput::Dismiss => LocalTabChromeAction::DismissMenu {
                session,
                revision,
                record: self.record.clone(),
                backdrop: self.backdrop,
            },
        };
        let stamp = self.witness.stamp();
        let action = PreparedSurfaceAction::tab_chrome(
            stamp.requirement().authority_domain(),
            self.version,
            stamp,
            Arc::clone(&self.witness),
            action,
        );
        Some((
            DockspaceVisualId(VisualIdentity::TabListMenuRow {
                session,
                tab: target.tab(),
            }),
            action.into_deferred(),
        ))
    }
}

impl SurfacePaintPlan<'_> {
    /// Captures one menu for retained keyboard and accessibility delivery.
    #[must_use]
    pub fn retain_tab_list_menu(
        self,
        menu: TabListMenuPaintRecord<'_>,
    ) -> Option<RetainedTabListMenu> {
        if !std::ptr::eq(self.plan, menu.plan) {
            return None;
        }
        Some(RetainedTabListMenu {
            witness: self.frozen_action_witness()?,
            version: self.version,
            record: menu.record.clone(),
            backdrop: menu.backdrop,
        })
    }
}

/// Product-facing class of one core-owned tab-strip control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TabStripControlKind {
    /// Scroll toward the beginning of the stable tab order.
    ScrollBackward,
    /// Scroll toward the end of the stable tab order.
    ScrollForward,
    /// Open or close the complete tab-list menu.
    TabListMenu,
    /// Reveal an automatically hidden single-member tab bar.
    RevealTabBar,
}

/// Read-only geometry and state for one tab-strip control.
#[derive(Debug, Clone, Copy)]
pub struct TabStripControlPaintRecord {
    pub(super) record: TabStripControlRecord,
    pub(super) menu_open: bool,
}

impl TabStripControlPaintRecord {
    #[must_use]
    pub const fn root(self) -> RootId {
        self.record.id().bar().root
    }

    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabStripControl(self.record.id()))
    }

    #[must_use]
    pub const fn tab_bar_visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabBar(self.record.id().bar()))
    }

    #[must_use]
    pub const fn kind(self) -> TabStripControlKind {
        match self.record.id() {
            TabStripControlId::ScrollBackward(_) => TabStripControlKind::ScrollBackward,
            TabStripControlId::ScrollForward(_) => TabStripControlKind::ScrollForward,
            TabStripControlId::TabListMenu(_) => TabStripControlKind::TabListMenu,
            TabStripControlId::RevealTabBar(_) => TabStripControlKind::RevealTabBar,
        }
    }

    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }

    #[must_use]
    pub const fn hit_bounds(self) -> LogicalRect {
        self.record.hit().rect()
    }

    #[must_use]
    pub const fn enabled(self) -> bool {
        self.record.enabled()
    }

    /// Returns whether this control owns the currently open tab-list menu.
    #[must_use]
    pub const fn menu_open(self) -> bool {
        self.menu_open
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }
}

/// Read-only row within one core-owned tab-list popup.
#[derive(Debug, Clone, Copy)]
pub struct TabListMenuRowPaintRecord<'plan> {
    pub(super) plan: &'plan crate::presentation::scene::PresentationPlan,
    pub(super) session: TabListMenuSessionId,
    pub(super) record: TabListMenuRowRecord,
}

impl<'plan> TabListMenuRowPaintRecord<'plan> {
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabListMenuRow {
            session: self.session,
            tab: self.record.tab(),
        })
    }

    #[must_use]
    pub const fn tab_visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Tab(self.record.tab()))
    }

    #[must_use]
    pub const fn key(self) -> TabKey {
        self.record.tab().tab
    }

    #[must_use]
    pub const fn item(self) -> Option<ItemId> {
        self.key().item()
    }

    /// Returns the same frozen label used by the exact tab and preview.
    #[must_use]
    pub fn label(self) -> crate::graph::TabLabel<'plan> {
        self.plan
            .tab_label(self.record.tab())
            .expect("compiled menu rows retain exact labels")
    }

    #[must_use]
    pub const fn ordinal(self) -> usize {
        self.record.ordinal()
    }

    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }

    #[must_use]
    pub const fn interaction_bounds(self) -> Option<LogicalRect> {
        match self.record.hit() {
            Some(hit) => Some(hit.rect()),
            None => None,
        }
    }

    #[must_use]
    pub const fn selected(self) -> bool {
        self.record.selected()
    }

    #[must_use]
    pub const fn focused(self) -> bool {
        self.record.focused()
    }
}

/// Read-only geometry for one open tab-list popup.
#[derive(Debug, Clone, Copy)]
pub struct TabListMenuPaintRecord<'plan> {
    pub(super) record: &'plan TabListMenuRecord,
    pub(super) plan: &'plan crate::presentation::scene::PresentationPlan,
    pub(super) surface: SurfaceId,
    pub(super) backdrop: TabListMenuBackdropRecord,
}

impl<'plan> TabListMenuPaintRecord<'plan> {
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabListMenu(self.record.session()))
    }

    #[must_use]
    pub const fn tab_bar_visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabBar(self.record.bar()))
    }

    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }

    #[must_use]
    pub const fn viewport(self) -> LogicalRect {
        self.record.viewport()
    }

    #[must_use]
    pub const fn scroll_offset(self) -> f64 {
        self.record.scroll_offset()
    }

    #[must_use]
    pub const fn maximum_scroll_offset(self) -> f64 {
        self.record.maximum_scroll_offset()
    }

    pub fn rows(self) -> impl ExactSizeIterator<Item = TabListMenuRowPaintRecord<'plan>> + 'plan {
        let session = self.record.session();
        self.record
            .rows()
            .iter()
            .copied()
            .map(move |record| TabListMenuRowPaintRecord {
                session,
                record,
                plan: self.plan,
            })
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }

    /// Returns the exact full-surface backdrop paired with this menu.
    #[must_use]
    pub const fn backdrop(self) -> TabListMenuBackdropPaintRecord {
        TabListMenuBackdropPaintRecord {
            surface: self.surface,
            record: self.backdrop,
        }
    }
}

/// Read-only full-surface popup backdrop geometry.
#[derive(Debug, Clone, Copy)]
pub struct TabListMenuBackdropPaintRecord {
    pub(super) surface: SurfaceId,
    pub(super) record: TabListMenuBackdropRecord,
}

impl TabListMenuBackdropPaintRecord {
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabListMenuBackdrop {
            surface: self.surface,
            session: self.record.session(),
            revision: self.record.revision(),
        })
    }

    #[must_use]
    pub const fn menu_visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabListMenu(self.record.session()))
    }

    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.record.bounds()
    }
}
