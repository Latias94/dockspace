//! Tab-strip controls and popup paint records.

use std::sync::Arc;

use crate::engine::LocalTabChromeAction;
use crate::geometry::LogicalRect;
use crate::ids::{EngineAuthorityDomainId, ItemId, RootId, TabKey, WorkspaceEpoch};
use crate::interaction::FrozenTabListMenuRowClick;
use crate::model::WorkspaceVersion;
use crate::presentation::scene::{
    SurfacePlanScene, TabBarSceneId, TabListMenuBackdropRecord, TabListMenuRecord,
    TabListMenuRowRecord, TabSceneId, TabStripControlRecord,
};
use crate::presentation::tab_strip::{
    PopupRoutingRevision, TabListMenuSessionId, TabStripControlId,
};
use crate::runtime::{DeferredSurfaceAction, PreparedSurfaceAction, SurfaceTabListInput};

use super::{
    DockspaceControlId, DockspacePaintLayer, DockspaceSemanticOutput, DockspaceVisualId,
    SurfacePaintPlan, VisualIdentity,
};

/// Input identity of one row in a core-issued tab-list menu instance.
///
/// Stable across geometry changes to the same menu. It expires after a session
/// or layout replacement, reopening the menu, or changing its ordered roster.
/// Use [`Self::visual_id`] for rendering identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabListMenuRowId {
    scope: MenuInputScope,
    key: TabKey,
    ordinal: usize,
}

impl TabListMenuRowId {
    /// Returns the row's visual identity for the host's retained controls.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        let bar = self.scope.session.key().bar();
        DockspaceVisualId(VisualIdentity::TabListMenuRow {
            session: self.scope.session,
            tab: TabSceneId {
                root: bar.root,
                tabs: bar.tabs,
                tab: self.key,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MenuInputScope {
    domain: EngineAuthorityDomainId,
    epoch: WorkspaceEpoch,
    session: TabListMenuSessionId,
    revision: PopupRoutingRevision,
}

impl MenuInputScope {
    fn from_plan(plan: &SurfacePaintPlan<'_>, backdrop: TabListMenuBackdropRecord) -> Self {
        Self {
            domain: plan.authority_domain,
            epoch: plan.version.epoch(),
            session: backdrop.session(),
            revision: backdrop.revision(),
        }
    }

    const fn row(self, record: TabListMenuRowRecord) -> TabListMenuRowId {
        TabListMenuRowId {
            scope: self,
            key: record.tab().tab,
            ordinal: record.ordinal(),
        }
    }
}

/// Shared keyboard authority for one tab-list menu from an exact output.
///
/// Keep one copy with the rendered menu. Resolve a key only after the host
/// delivers it to that output. Core revalidates the deferred action on submit.
#[derive(Debug, Clone)]
pub struct RetainedTabListMenu {
    witness: Arc<SurfacePlanScene>,
    version: WorkspaceVersion,
    scope: MenuInputScope,
    record: TabListMenuRecord,
    backdrop: TabListMenuBackdropRecord,
}

/// The closed tab-list control that received an opening input.
///
/// Retain this while waiting for its menu. The host checks that the opening
/// input was accepted and that focus still belongs to the menu.
#[derive(Debug, Clone)]
pub struct RetainedTabListOpener {
    witness: Arc<SurfacePlanScene>,
    version: WorkspaceVersion,
    bar: TabBarSceneId,
}

impl RetainedTabListMenu {
    /// Tests whether this is the first menu opened by the witnessed control.
    ///
    /// Reopening, roster changes, and changes to surface input authority expire
    /// the transition. Resolve queued input only after the host accepts this menu.
    #[must_use]
    pub fn can_continue_opening(&self, opener: &RetainedTabListOpener) -> bool {
        self.scope.session.key().bar() == opener.bar
            && self.version == opener.version
            && super::tab_input::same_surface_keyboard_authority(&self.witness, &opener.witness)
            && opener.witness.plan().popup().revision().checked_next() == Some(self.scope.revision)
    }

    /// Tests whether a later presented menu can continue this menu's keyboard input.
    ///
    /// The popup session, ordered roster and keyboard authority must match.
    /// The host still validates the expected workspace version and focus claim.
    #[must_use]
    pub fn can_continue_from(&self, previous: &Self) -> bool {
        self.scope == previous.scope
            && super::tab_input::same_keyboard_authority(&self.witness, &previous.witness)
    }

    /// Resolves one input and returns its resulting row identity and action.
    ///
    /// Start with [`TabListMenuRowPaintRecord::input_id`] or the destination of
    /// the previous key in a batch. The row may be scrolled out of view. Use
    /// [`Self::can_continue_from`] when resuming pending keys on a later output.
    #[must_use]
    pub fn resolve(
        &self,
        row: TabListMenuRowId,
        input: SurfaceTabListInput,
    ) -> Option<(TabListMenuRowId, DeferredSurfaceAction)> {
        if row.scope != self.scope {
            return None;
        }
        let rows = self.record.rows();
        let start = rows
            .get(row.ordinal)
            .filter(|start| start.tab().tab == row.key)?;
        let target = match input {
            SurfaceTabListInput::Navigate(navigation) => {
                &rows[navigation.destination(start.ordinal(), rows.len() - 1)]
            }
            _ => start,
        };
        let session = self.scope.session;
        let revision = self.scope.revision;
        let action = match input {
            SurfaceTabListInput::Focus | SurfaceTabListInput::Navigate(_) => {
                LocalTabChromeAction::NavigateMenu {
                    session,
                    revision,
                    record: self.record.clone(),
                    target: target.tab().tab,
                }
            }
            SurfaceTabListInput::Activate => LocalTabChromeAction::ActivateMenuRow {
                row: FrozenTabListMenuRowClick {
                    session,
                    record: *target,
                    revision,
                },
                focus_content: true,
            },
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
        Some((self.scope.row(*target), action.into_deferred()))
    }
}

impl SurfacePaintPlan<'_> {
    /// Retains the exact closed control for input received after an accepted open.
    #[must_use]
    pub fn retain_tab_list_opener(
        self,
        control: TabStripControlPaintRecord,
    ) -> Option<RetainedTabListOpener> {
        (control.belongs_to(&self)
            && control.enabled()
            && control.kind() == TabStripControlKind::TabListMenu
            && !control.menu_open())
        .then_some(())?;
        Some(RetainedTabListOpener {
            witness: self.frozen_action_witness()?,
            version: self.version,
            bar: control.record.id().bar(),
        })
    }

    /// Captures one menu for retained keyboard and accessibility delivery.
    #[must_use]
    pub fn retain_tab_list_menu(
        self,
        menu: TabListMenuPaintRecord<'_>,
    ) -> Option<RetainedTabListMenu> {
        if !menu.backdrop.belongs_to(&self) {
            return None;
        }
        Some(RetainedTabListMenu {
            witness: self.frozen_action_witness()?,
            version: self.version,
            scope: menu.scope,
            record: menu.record.clone(),
            backdrop: menu.backdrop.record,
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

/// Read-only geometry and state for one tab-strip control from an exact output.
#[derive(Debug, Clone, Copy)]
pub struct TabStripControlPaintRecord {
    pub(super) record: TabStripControlRecord,
    output: DockspaceSemanticOutput,
    menu_open: bool,
}

impl TabStripControlPaintRecord {
    pub(super) const fn new(
        plan: &SurfacePaintPlan<'_>,
        record: TabStripControlRecord,
        menu_open: bool,
    ) -> Self {
        Self {
            record,
            output: plan.semantic_output(),
            menu_open,
        }
    }

    pub(super) fn belongs_to(self, plan: &SurfacePaintPlan<'_>) -> bool {
        self.output == plan.semantic_output()
    }

    /// Returns the input identity for widget and accessibility registration.
    #[must_use]
    pub const fn input_id(self) -> DockspaceControlId {
        self.output.control_id(self.visual_id())
    }

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
    plan: &'plan crate::presentation::scene::PresentationPlan,
    output: DockspaceSemanticOutput,
    scope: MenuInputScope,
    pub(super) record: TabListMenuRowRecord,
}

impl<'plan> TabListMenuRowPaintRecord<'plan> {
    pub(super) fn belongs_to(self, plan: &SurfacePaintPlan<'_>) -> bool {
        self.output == plan.semantic_output()
    }

    /// Returns the row's identity for use with [`RetainedTabListMenu::resolve`].
    #[must_use]
    pub const fn input_id(self) -> TabListMenuRowId {
        self.scope.row(self.record)
    }

    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        self.input_id().visual_id()
    }

    pub(super) const fn session(self) -> TabListMenuSessionId {
        self.scope.session
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
    plan: &'plan crate::presentation::scene::PresentationPlan,
    scope: MenuInputScope,
    backdrop: TabListMenuBackdropPaintRecord,
}

impl<'plan> TabListMenuPaintRecord<'plan> {
    pub(super) fn new(
        plan: &SurfacePaintPlan<'plan>,
        record: &'plan TabListMenuRecord,
        backdrop: TabListMenuBackdropRecord,
    ) -> Self {
        Self {
            record,
            plan: plan.plan,
            scope: MenuInputScope::from_plan(plan, backdrop),
            backdrop: TabListMenuBackdropPaintRecord::new(plan, backdrop),
        }
    }

    /// Returns the input identity shared by this popup's frame and scrollbar.
    #[must_use]
    pub const fn input_id(self) -> DockspaceControlId {
        self.backdrop.output.control_id(self.backdrop.visual_id())
    }

    /// Returns the same input identity used by the list button.
    #[must_use]
    pub const fn opener_input_id(self) -> DockspaceControlId {
        let opener = TabStripControlId::TabListMenu(self.record.bar());
        self.backdrop
            .output
            .control_id(DockspaceVisualId(VisualIdentity::TabStripControl(opener)))
    }

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
        self.record
            .rows()
            .iter()
            .copied()
            .map(move |record| TabListMenuRowPaintRecord {
                scope: self.scope,
                record,
                plan: self.plan,
                output: self.backdrop.output,
            })
    }

    #[must_use]
    pub const fn layer(self) -> DockspacePaintLayer {
        DockspacePaintLayer::from_core(self.record.layer())
    }

    /// Returns the exact full-surface backdrop paired with this menu.
    #[must_use]
    pub const fn backdrop(self) -> TabListMenuBackdropPaintRecord {
        self.backdrop
    }
}

/// Read-only full-surface popup backdrop geometry from one exact paint output.
#[derive(Debug, Clone, Copy)]
pub struct TabListMenuBackdropPaintRecord {
    output: DockspaceSemanticOutput,
    pub(super) record: TabListMenuBackdropRecord,
}

impl TabListMenuBackdropPaintRecord {
    pub(super) const fn new(
        plan: &SurfacePaintPlan<'_>,
        record: TabListMenuBackdropRecord,
    ) -> Self {
        Self {
            output: plan.semantic_output(),
            record,
        }
    }

    pub(super) fn belongs_to(self, plan: &SurfacePaintPlan<'_>) -> bool {
        self.output == plan.semantic_output()
    }

    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabListMenuBackdrop {
            surface: self.output.surface(),
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
