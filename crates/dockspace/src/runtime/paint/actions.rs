//! Opaque framework gesture preparation over one exact paint plan.

use crate::geometry::LogicalPoint;
use crate::ids::{FloatingPresentationId, ItemId, TabKey};
use crate::interaction::{
    FrozenTabListMenuBackdropClick, FrozenTabListMenuRowClick, FrozenTabStripControlClick,
};
use crate::presentation::scene::ContainedResizeDirection as CoreContainedResizeDirection;
use crate::presentation::tab_strip::TabStripStateKey;

use super::super::{
    PreparedSurfaceAction, SurfaceContainedResizeAdjustment, SurfaceSplitterAdjustment,
    SurfaceTabInput, SurfaceTabListNavigation, SurfaceTabNavigation,
};
use super::{
    DockspaceVisualId, SurfacePaintPlan, TabBarPaintRecord, TabListMenuBackdropPaintRecord,
    TabListMenuPaintRecord, TabListMenuRowPaintRecord, TabStripControlPaintRecord, VisualIdentity,
};

/// Product-facing direction of one contained-floating resize handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContainedResizeDirection {
    /// Top edge.
    North,
    /// Top-right corner.
    NorthEast,
    /// Right edge.
    East,
    /// Bottom-right corner.
    SouthEast,
    /// Bottom edge.
    South,
    /// Bottom-left corner.
    SouthWest,
    /// Left edge.
    West,
    /// Top-left corner.
    NorthWest,
}

impl ContainedResizeDirection {
    pub(super) const fn from_core(direction: CoreContainedResizeDirection) -> Self {
        match direction {
            CoreContainedResizeDirection::North => Self::North,
            CoreContainedResizeDirection::NorthEast => Self::NorthEast,
            CoreContainedResizeDirection::East => Self::East,
            CoreContainedResizeDirection::SouthEast => Self::SouthEast,
            CoreContainedResizeDirection::South => Self::South,
            CoreContainedResizeDirection::SouthWest => Self::SouthWest,
            CoreContainedResizeDirection::West => Self::West,
            CoreContainedResizeDirection::NorthWest => Self::NorthWest,
        }
    }

    const fn into_core(self) -> CoreContainedResizeDirection {
        match self {
            Self::North => CoreContainedResizeDirection::North,
            Self::NorthEast => CoreContainedResizeDirection::NorthEast,
            Self::East => CoreContainedResizeDirection::East,
            Self::SouthEast => CoreContainedResizeDirection::SouthEast,
            Self::South => CoreContainedResizeDirection::South,
            Self::SouthWest => CoreContainedResizeDirection::SouthWest,
            Self::West => CoreContainedResizeDirection::West,
            Self::NorthWest => CoreContainedResizeDirection::NorthWest,
        }
    }
}

impl SurfacePaintPlan<'_> {
    pub(super) fn frozen_action_witness(
        self,
    ) -> Option<std::sync::Arc<crate::presentation::scene::SurfacePlanScene>> {
        self.candidate
            .scene()
            .ready_candidate(self.surface)
            .filter(|candidate| candidate.stamp() == self.scene)
            .map(crate::presentation::scene::SurfacePlanScene::frozen_witness)
    }

    /// Prepares activation of one exact current-frame tab-strip control.
    #[must_use]
    pub fn prepare_tab_strip_control_activation(
        self,
        control: TabStripControlPaintRecord,
    ) -> Option<PreparedSurfaceAction> {
        (control.belongs_to(&self) && control.enabled()).then_some(())?;
        let record = control.record;
        let key = TabStripStateKey::new(self.surface, record.id().bar());
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabChromeAction::ActivateControl(FrozenTabStripControlClick {
                key,
                record,
            }),
        ))
    }

    /// Prepares a finite wheel adjustment for the sole authoritative tab-strip
    /// scroll receiver at `point`.
    ///
    /// The point is a framework input fact. Core resolves popup coverage and
    /// contained-floating occlusion before binding the opaque action to the
    /// current bar record and scene.
    #[must_use]
    pub fn prepare_tab_strip_scroll_at(
        self,
        bar: TabBarPaintRecord<'_>,
        point: LogicalPoint,
        delta: f64,
    ) -> Option<PreparedSurfaceAction> {
        let receiver = self.receiver_for_tab_strip_scroll(bar)?;
        if !receiver.bounds().contains(point)
            || self.plan.tab_scroll_owner_at(point)? != *bar.record.id()
        {
            return None;
        }
        let record = bar.record.clone();
        let adjustment = crate::engine::TabScrollAdjustment::scroll_by(delta).ok()?;
        let target = (record.scroll_offset() + delta).clamp(0.0, record.maximum_scroll_offset());
        if target == record.scroll_offset() {
            return None;
        }
        let key = TabStripStateKey::new(self.surface, *record.id());
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabChromeAction::ScrollStrip {
                key,
                record,
                adjustment,
            },
        ))
    }

    /// Prepares activation of one exact current-frame tab-list row.
    #[must_use]
    pub fn prepare_tab_list_menu_row_activation(
        self,
        row: TabListMenuRowPaintRecord,
    ) -> Option<PreparedSurfaceAction> {
        row.belongs_to(&self).then_some(())?;
        let session = row.session();
        let menu = self
            .plan
            .tab_list_menu_records()
            .iter()
            .find(|menu| menu.session() == session)?;
        let record = *menu
            .rows()
            .get(row.record.ordinal())
            .filter(|&record| *record == row.record)?;
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabChromeAction::ActivateMenuRow(FrozenTabListMenuRowClick {
                session,
                record,
                revision: self.plan.popup().revision(),
            }),
        ))
    }

    /// Prepares dismissal of the exact open tab-list menu covered by `backdrop`.
    #[must_use]
    pub fn prepare_tab_list_menu_dismiss(
        self,
        backdrop: TabListMenuBackdropPaintRecord,
    ) -> Option<PreparedSurfaceAction> {
        backdrop.belongs_to(&self).then_some(())?;
        let projected = backdrop.record;
        let action = self
            .plan
            .tab_list_menu_records()
            .iter()
            .find(|record| record.session() == projected.session())
            .cloned()
            .map_or_else(
                || {
                    crate::engine::LocalTabChromeAction::DismissMenuFromBackdrop(
                        FrozenTabListMenuBackdropClick {
                            session: projected.session(),
                            record: projected,
                            revision: projected.revision(),
                        },
                    )
                },
                |record| crate::engine::LocalTabChromeAction::DismissMenu {
                    session: projected.session(),
                    revision: projected.revision(),
                    record,
                    backdrop: projected,
                },
            );
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            action,
        ))
    }

    /// Prepares a finite logical scroll adjustment for one exact open tab-list menu.
    #[must_use]
    pub fn prepare_tab_list_menu_scroll_by(
        self,
        menu: TabListMenuPaintRecord<'_>,
        delta: f64,
    ) -> Option<PreparedSurfaceAction> {
        self.prepare_tab_list_menu_scroll_sequence(menu, [delta])
    }

    /// Folds ordered logical scroll deltas into one exact tab-list menu action.
    ///
    /// Every step clamps to this menu's current scroll range before the next
    /// delta is applied. Moving away from the current offset releases row
    /// reveal even if a later step returns to that offset. Empty sequences
    /// and non-finite deltas return `None` without preparing a partial action.
    #[must_use]
    pub fn prepare_tab_list_menu_scroll_sequence(
        self,
        menu: TabListMenuPaintRecord<'_>,
        deltas: impl IntoIterator<Item = f64>,
    ) -> Option<PreparedSurfaceAction> {
        let adjustment = crate::engine::TabScrollAdjustment::menu_scroll_sequence(
            menu.scroll_offset(),
            menu.maximum_scroll_offset(),
            deltas,
        )?;
        self.prepare_tab_list_menu_scroll(menu, adjustment)
    }

    /// Prepares a finite wheel adjustment only when `point` is inside the exact
    /// current tab-list menu scroll receiver.
    #[must_use]
    pub fn prepare_tab_list_menu_scroll_at(
        self,
        menu: TabListMenuPaintRecord<'_>,
        point: LogicalPoint,
        delta: f64,
    ) -> Option<PreparedSurfaceAction> {
        let receiver = self.receiver_for_tab_list_menu_scroll(menu)?;
        receiver.bounds().contains(point).then_some(())?;
        if !delta.is_finite() {
            return None;
        }
        let target = (menu.scroll_offset() + delta).clamp(0.0, menu.maximum_scroll_offset());
        if target == menu.scroll_offset() {
            return None;
        }
        self.prepare_tab_list_menu_scroll_by(menu, delta)
    }

    /// Prepares the minimum scroll needed to reveal one exact menu row.
    #[must_use]
    pub fn prepare_tab_list_menu_reveal(
        self,
        row: TabListMenuRowPaintRecord,
    ) -> Option<PreparedSurfaceAction> {
        row.belongs_to(&self).then_some(())?;
        let menu = self
            .tab_list_menus()
            .find(|menu| menu.record.session() == row.session())?;
        self.prepare_tab_list_menu_scroll(
            menu,
            crate::engine::TabScrollAdjustment::reveal_tab(row.key()),
        )
    }

    /// Prepares one clamped focus move within an exact open tab-list menu.
    #[must_use]
    pub fn prepare_tab_list_menu_navigation(
        self,
        menu: TabListMenuPaintRecord<'_>,
        navigation: SurfaceTabListNavigation,
    ) -> Option<PreparedSurfaceAction> {
        let record = self.current_tab_list_menu(menu)?;
        let rows = record.rows();
        let current = rows.iter().position(|row| row.focused())?;
        let last = rows.len().checked_sub(1)?;
        let target = rows[navigation.destination(current, last)].tab().tab;
        self.prepare_tab_list_menu_focus(menu, target)
    }

    /// Prepares focus of one exact item in an open tab-list menu.
    #[must_use]
    pub fn prepare_tab_list_menu_focus(
        self,
        menu: TabListMenuPaintRecord<'_>,
        item: impl Into<TabKey>,
    ) -> Option<PreparedSurfaceAction> {
        let item = item.into();
        let record = self.current_tab_list_menu(menu)?;
        record
            .rows()
            .iter()
            .any(|row| row.tab().tab == item)
            .then_some(())?;
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabChromeAction::NavigateMenu {
                session: record.session(),
                revision: self.plan.popup().revision(),
                record,
                target: item,
            },
        ))
    }

    /// Prepares navigation within the exact tab strip which owns `item`.
    ///
    /// Navigation preserves framework tab focus through deferred selection. It
    /// supersedes older pane-focus requests without requesting content focus.
    #[must_use]
    pub fn prepare_tab_navigation(
        self,
        item: impl Into<TabKey>,
        navigation: SurfaceTabNavigation,
    ) -> Option<PreparedSurfaceAction> {
        let item = item.into();
        let tab = self.tab_record(item)?;
        let start = self.plan.selected_in_tabs(tab.id().tabs).unwrap_or(item);
        self.prepare_tab_input_sequence(item, start, [SurfaceTabInput::Navigate(navigation)])
    }

    /// Selects this exact tab while preserving framework focus in its header.
    ///
    /// Use this for an accessibility focus request. Unlike activation, focusing
    /// a tab supersedes older content-focus requests without entering its pane.
    #[must_use]
    pub fn prepare_tab_focus(self, item: impl Into<TabKey>) -> Option<PreparedSurfaceAction> {
        let item = item.into();
        let current = self.tab_record(item)?;
        self.tab_is_operable(current).then_some(())?;
        Some(PreparedSurfaceAction::select_tab(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabSelection::Navigate {
                source: *current.id(),
                target: *current.id(),
            },
        ))
    }

    /// Returns the pane a tab activation would ask the host to focus.
    ///
    /// Hosts can retain keyboard input for this pane until they receive a matching
    /// focus request. This lookup does not select the tab or authorize delivery.
    /// A layout tab resolves to its first active pane, including when the tab is
    /// currently unselected or clipped.
    #[must_use]
    pub fn tab_content_focus_target(self, key: TabKey) -> Option<ItemId> {
        let tab = self
            .plan
            .tab_bar_records()
            .iter()
            .flat_map(|bar| bar.members())
            .find(|member| member.tab().tab == key)?;
        self.candidate.tab_content_focus_target(tab.tab())
    }

    /// Prepares one scene-bound semantic adjustment for an exact splitter.
    #[must_use]
    pub fn prepare_splitter_adjustment(
        self,
        splitter: DockspaceVisualId,
        adjustment: SurfaceSplitterAdjustment,
    ) -> Option<PreparedSurfaceAction> {
        self.prepare_splitter_adjustment_sequence(splitter, [adjustment])
    }

    /// Folds ordered adjustments into one scene-bound splitter action.
    ///
    /// Each step uses the track allocation left by the previous accepted step
    /// and the same limits as a direct resize. Only the final track weights
    /// are published. A sequence can redistribute space even if the divider
    /// returns to its starting point.
    /// Auto steps are checked in order: an infeasible step retains the last
    /// accepted allocation. Failed source or policy checks abort preparation.
    /// An empty sequence or a single infeasible Auto step returns `None`.
    #[must_use]
    pub fn prepare_splitter_adjustment_sequence(
        self,
        splitter: DockspaceVisualId,
        adjustments: impl IntoIterator<Item = SurfaceSplitterAdjustment>,
    ) -> Option<PreparedSurfaceAction> {
        let VisualIdentity::Splitter(id) = splitter.0 else {
            return None;
        };
        let record = self
            .plan
            .splitter_record(id)
            .filter(|record| record.operable() && self.plan.partition_available(id.root))?;
        let mut adjustments = adjustments.into_iter().peekable();
        let first = adjustments.next()?;
        let reset = if adjustments.peek().is_none() {
            match first {
                SurfaceSplitterAdjustment::Equalize => Some(crate::engine::SplitterReset::Equalize),
                SurfaceSplitterAdjustment::Preferred => {
                    Some(crate::engine::SplitterReset::Preferred)
                }
                _ => None,
            }
        } else {
            None
        };
        let adjustment = self.splitter_adjustment_sequence(
            crate::presentation::scene::SplitterResizeTarget::Handle(id),
            record.axis(),
            std::iter::once(first).chain(adjustments),
        )?;
        if let Some(reset) = reset {
            return Some(PreparedSurfaceAction::reset_splitter(
                self.authority_domain,
                self.version,
                self.scene,
                self.frozen_action_witness()?,
                id,
                reset,
                adjustment,
            ));
        }
        Some(PreparedSurfaceAction::adjust_splitter(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            id,
            adjustment,
        ))
    }

    /// Prepares one scene-bound semantic adjustment for a single junction axis.
    ///
    /// Equalizing requires a common constrained displacement for every incident
    /// pair on that axis. Incompatible pairs return `None` instead of moving
    /// their shared junction independently.
    #[must_use]
    pub fn prepare_splitter_junction_adjustment(
        self,
        junction: DockspaceVisualId,
        axis: crate::model::DockspaceAxis,
        adjustment: SurfaceSplitterAdjustment,
    ) -> Option<PreparedSurfaceAction> {
        self.prepare_splitter_junction_adjustment_sequence(junction, axis, [adjustment])
    }

    /// Folds ordered adjustments for one junction axis into one action.
    ///
    /// Each step uses the previous accepted track allocation and the common
    /// reachable displacement of all incident pairs on this axis. Equalizing
    /// requires a common constrained destination. The final track weights are
    /// published together. An infeasible Auto step keeps the last accepted
    /// allocation. Failed source or policy checks abort preparation. An empty
    /// sequence or a single infeasible Auto step returns `None`.
    #[must_use]
    pub fn prepare_splitter_junction_adjustment_sequence(
        self,
        junction: DockspaceVisualId,
        axis: crate::model::DockspaceAxis,
        adjustments: impl IntoIterator<Item = SurfaceSplitterAdjustment>,
    ) -> Option<PreparedSurfaceAction> {
        let VisualIdentity::SplitterJunction(id) = junction.0 else {
            return None;
        };
        let record = self
            .splitter_junctions()
            .find(|record| record.visual_id() == junction)?;
        self.splitter_junction_axis_operable(record, axis)
            .then_some(())?;
        let adjustment = self.splitter_adjustment_sequence(
            crate::presentation::scene::SplitterResizeTarget::Junction(id),
            axis.into(),
            adjustments,
        )?;
        Some(PreparedSurfaceAction::adjust_splitter_junction(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            id,
            axis,
            adjustment,
        ))
    }

    /// Prepares one keyboard or accessibility adjustment for an exact cardinal
    /// contained-floating resize edge.
    #[must_use]
    pub fn prepare_contained_resize_adjustment(
        self,
        floating: FloatingPresentationId,
        direction: ContainedResizeDirection,
        adjustment: SurfaceContainedResizeAdjustment,
    ) -> Option<PreparedSurfaceAction> {
        self.prepare_contained_resize_adjustment_sequence(floating, direction, [adjustment])
    }

    /// Folds ordered cardinal-edge adjustments into one scene-bound action.
    ///
    /// Every step uses the same surface bounds and minimum-size constraints as
    /// direct resizing. An outward step at a limit followed by an inward step
    /// therefore ends inside that limit. Empty sequences return `None`.
    #[must_use]
    pub fn prepare_contained_resize_adjustment_sequence(
        self,
        floating: FloatingPresentationId,
        direction: ContainedResizeDirection,
        adjustments: impl IntoIterator<Item = SurfaceContainedResizeAdjustment>,
    ) -> Option<PreparedSurfaceAction> {
        let direction = direction.into_core();
        match direction {
            CoreContainedResizeDirection::North
            | CoreContainedResizeDirection::East
            | CoreContainedResizeDirection::South
            | CoreContainedResizeDirection::West => {}
            CoreContainedResizeDirection::NorthEast
            | CoreContainedResizeDirection::SouthEast
            | CoreContainedResizeDirection::SouthWest
            | CoreContainedResizeDirection::NorthWest => return None,
        }
        let contained = self.plan.contained_records().iter().find(|record| {
            record.floating() == floating
                && record.transform_operable()
                && record.resize().iter().any(|resize| {
                    resize.direction() == direction
                        && self
                            .plan
                            .region_is_operable(resize.hit().rect(), record.layer())
                })
        })?;
        let source = self
            .candidate
            .workspace()
            .contained_floating(floating)
            .filter(|record| record.root == contained.root())?;
        let delta = crate::engine::DockEngine::cardinal_contained_resize_sequence_delta(
            source.rect,
            self.plan.bounds(),
            contained.minimum_size(),
            direction,
            adjustments
                .into_iter()
                .map(|adjustment| self.splitter_keyboard_step * adjustment.direction()),
        )?;
        Some(PreparedSurfaceAction::adjust_contained_resize(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            floating,
            direction,
            delta,
        ))
    }

    /// Prepares cancellation of the exact active gesture exposed by this callback.
    ///
    /// Local surfaces may cancel their own gesture. A native surface carries its
    /// exact current window binding and may cancel a gesture from another window.
    #[must_use]
    pub fn prepare_gesture_cancel(self) -> Option<PreparedSurfaceAction> {
        let delivery = self.gesture_cancel_delivery?;
        Some(PreparedSurfaceAction::cancel_gesture(
            self.authority_domain,
            self.version,
            self.surface,
            self.frozen_action_witness()?,
            self.candidate.interaction().status(),
            delivery,
        ))
    }

    fn prepare_tab_list_menu_scroll(
        self,
        menu: TabListMenuPaintRecord<'_>,
        adjustment: crate::engine::TabScrollAdjustment,
    ) -> Option<PreparedSurfaceAction> {
        let record = self.current_tab_list_menu(menu)?;
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabChromeAction::ScrollMenu {
                session: record.session(),
                revision: self.plan.popup().revision(),
                record,
                adjustment,
            },
        ))
    }

    fn current_tab_list_menu(
        self,
        menu: TabListMenuPaintRecord<'_>,
    ) -> Option<crate::presentation::scene::TabListMenuRecord> {
        menu.backdrop()
            .belongs_to(&self)
            .then(|| menu.record.clone())
    }
}

#[cfg(test)]
mod scroll_tests;
