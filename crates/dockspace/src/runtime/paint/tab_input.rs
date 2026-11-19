//! Tab navigation shared by immediate and retained hosts.

use std::sync::Arc;

use crate::engine::LocalTabSelection;
use crate::ids::TabKey;
use crate::model::WorkspaceVersion;
use crate::policy::TabBarInteraction;
use crate::presentation::scene::{
    PresentationPlan, SurfacePlanScene, TabBarRecord, TabBarSceneId, TabRecord, TabSceneId,
};
use crate::runtime::{DeferredSurfaceAction, PreparedSurfaceAction, SurfaceTabInput};

use super::{DockspaceVisualId, SurfacePaintPlan, TabBarPaintRecord, VisualIdentity};

/// An owned tab strip from one exact output for hosts that dispatch input after painting.
///
/// Retain this with the corresponding controls and use it only for events
/// delivered to that output. Core revalidates the resulting deferred action
/// against current receiver authority and layout when it is submitted.
#[derive(Debug, Clone)]
pub struct RetainedTabStrip {
    witness: Arc<SurfacePlanScene>,
    version: WorkspaceVersion,
    bar: TabBarSceneId,
}

impl RetainedTabStrip {
    /// Returns the visual identity of the strip these inputs address.
    #[must_use]
    pub const fn visual_id(&self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::TabBar(self.bar))
    }

    /// Resolves inputs from an operable `receiver` in the original output.
    ///
    /// `start` can name a different member of the same strip, including one
    /// scrolled out of view. See [`SurfacePaintPlan::prepare_tab_input_sequence`]
    /// for how the sequence is folded.
    #[must_use]
    pub fn resolve(
        &self,
        receiver: impl Into<TabKey>,
        start: impl Into<TabKey>,
        inputs: impl IntoIterator<Item = SurfaceTabInput>,
    ) -> Option<DeferredSurfaceAction> {
        let plan = self.witness.plan();
        let source = TabSceneId {
            root: self.bar.root,
            tabs: self.bar.tabs,
            tab: receiver.into(),
        };
        let tab = plan.tab_records().iter().find(|tab| *tab.id() == source)?;
        let bar = tab_bar(plan, self.bar)?;
        let selection = resolve(plan, bar, tab, start.into(), inputs)?;
        let scene = self.witness.stamp();
        Some(
            PreparedSurfaceAction::select_tab(
                scene.requirement().authority_domain(),
                self.version,
                scene,
                Arc::clone(&self.witness),
                selection,
            )
            .into_deferred(),
        )
    }
}

impl SurfacePaintPlan<'_> {
    /// Captures this strip once so a later callback can resolve its tab input.
    ///
    /// Returns `None` for a foreign record or a strip with disabled interaction.
    #[must_use]
    pub fn retain_tab_strip(self, bar: TabBarPaintRecord<'_>) -> Option<RetainedTabStrip> {
        if !std::ptr::eq(bar.plan, self.plan)
            || bar.record.interaction() != TabBarInteraction::Enabled
        {
            return None;
        }
        Some(RetainedTabStrip {
            witness: self.frozen_action_witness()?,
            version: self.version,
            bar: *bar.record.id(),
        })
    }

    /// Folds ordered tab inputs into one action against this exact candidate.
    ///
    /// `receiver` must be operable. `start` identifies the initial member in
    /// the same strip, which may be clipped. Use the focused tab for header
    /// keys and the selected tab for document-cycling shortcuts.
    /// Each later input uses the preceding destination. The last input decides
    /// whether to preserve header focus or activate pane content.
    ///
    /// A round trip is handled even if it returns to the initial tab. Empty
    /// sequences and ineffectual navigation return `None`.
    #[must_use]
    pub fn prepare_tab_input_sequence(
        self,
        receiver: impl Into<TabKey>,
        start: impl Into<TabKey>,
        inputs: impl IntoIterator<Item = SurfaceTabInput>,
    ) -> Option<PreparedSurfaceAction> {
        let tab = self.tab_record(receiver.into())?;
        let bar = tab_bar(self.plan, owner(tab))?;
        let selection = resolve(self.plan, bar, tab, start.into(), inputs)?;
        Some(PreparedSurfaceAction::select_tab(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            selection,
        ))
    }

    pub(super) fn tab_is_operable(&self, tab: &TabRecord) -> bool {
        tab_bar(self.plan, owner(tab)).is_some_and(|bar| operable(self.plan, bar, tab))
    }
}

fn owner(tab: &TabRecord) -> TabBarSceneId {
    TabBarSceneId {
        root: tab.id().root,
        tabs: tab.id().tabs,
    }
}

fn tab_bar(plan: &PresentationPlan, id: TabBarSceneId) -> Option<&TabBarRecord> {
    let bars = plan.tab_bar_records();
    let index = bars.binary_search_by_key(&id, |bar| *bar.id()).ok()?;
    Some(&bars[index])
}

fn operable(plan: &PresentationPlan, bar: &TabBarRecord, tab: &TabRecord) -> bool {
    let bounds = tab.drag_hit().rect();
    bounds.width() > 0.0
        && bounds.height() > 0.0
        && bar.interaction() == TabBarInteraction::Enabled
        && plan.region_is_operable(bounds, tab.layer())
}

fn resolve(
    plan: &PresentationPlan,
    bar: &TabBarRecord,
    tab: &TabRecord,
    start: TabKey,
    inputs: impl IntoIterator<Item = SurfaceTabInput>,
) -> Option<LocalTabSelection> {
    operable(plan, bar, tab).then_some(())?;
    let members = bar.members();
    let mut cursor = members
        .iter()
        .position(|member| member.tab().tab == start)?;
    let mut inputs = inputs.into_iter().peekable();
    inputs.peek()?;
    let mut handled = false;
    let mut activate = false;
    let mut navigated = false;
    for input in inputs {
        match input {
            SurfaceTabInput::Navigate(navigation) => {
                let next = navigation.into_core().destination(cursor, members.len());
                handled |= next != cursor;
                cursor = next;
                activate = false;
                navigated = true;
            }
            SurfaceTabInput::Activate => {
                handled = true;
                activate = true;
            }
        }
    }
    let source = *tab.id();
    let target = members[cursor].tab();
    // Header focus and selection can differ. A clipped selected target can
    // also require revealing even when Home/End leaves the cursor unchanged.
    (handled || target != source || !plan.tab_selected(tab)).then_some(())?;
    Some(if activate {
        if navigated || target != source {
            LocalTabSelection::NavigateAndActivate { source, target }
        } else {
            LocalTabSelection::Activate(source)
        }
    } else {
        LocalTabSelection::Navigate { source, target }
    })
}
