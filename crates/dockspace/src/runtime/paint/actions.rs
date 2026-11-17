//! Opaque framework gesture preparation over one exact paint plan.

use crate::geometry::LogicalPoint;
use crate::ids::{FloatingPresentationId, ItemId};
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
        let record = self
            .plan
            .tab_strip_control_records()
            .iter()
            .copied()
            .find(|record| *record == control.record && record.enabled())?;
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
        let record = self
            .plan
            .tab_bar_records()
            .iter()
            .find(|record| *record == bar.record)
            .cloned()?;
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
        let menu = self
            .plan
            .tab_list_menu_records()
            .iter()
            .find(|menu| menu.session() == row.session)?;
        let record = menu
            .rows()
            .iter()
            .copied()
            .find(|record| *record == row.record)?;
        Some(PreparedSurfaceAction::tab_chrome(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            crate::engine::LocalTabChromeAction::ActivateMenuRow(FrozenTabListMenuRowClick {
                session: row.session,
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
        let projected = self
            .plan
            .tab_list_menu_backdrop_records()
            .iter()
            .copied()
            .find(|record| *record == backdrop.record)?;
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
    /// delta is applied. A round trip remains a handled action even when its
    /// final offset is unchanged. Empty sequences and non-finite deltas return
    /// `None` without preparing a partial action.
    #[must_use]
    pub fn prepare_tab_list_menu_scroll_sequence(
        self,
        menu: TabListMenuPaintRecord<'_>,
        deltas: impl IntoIterator<Item = f64>,
    ) -> Option<PreparedSurfaceAction> {
        let mut deltas = deltas.into_iter().peekable();
        deltas.peek()?;
        let target = deltas.try_fold(menu.scroll_offset(), |offset, delta| {
            delta
                .is_finite()
                .then(|| (offset + delta).clamp(0.0, menu.maximum_scroll_offset()))
        })?;
        let adjustment =
            crate::engine::TabScrollAdjustment::scroll_by(target - menu.scroll_offset()).ok()?;
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
        let menu = self
            .tab_list_menus()
            .find(|menu| menu.record.session() == row.session)?;
        self.prepare_tab_list_menu_scroll(
            menu,
            crate::engine::TabScrollAdjustment::reveal_item(row.item()),
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
        let target = match navigation {
            SurfaceTabListNavigation::Previous => rows[current.saturating_sub(1)].tab().item,
            SurfaceTabListNavigation::Next => rows[current.saturating_add(1).min(last)].tab().item,
            SurfaceTabListNavigation::First => rows[0].tab().item,
            SurfaceTabListNavigation::Last => rows[last].tab().item,
        };
        self.prepare_tab_list_menu_focus(menu, target)
    }

    /// Prepares focus of one exact item in an open tab-list menu.
    #[must_use]
    pub fn prepare_tab_list_menu_focus(
        self,
        menu: TabListMenuPaintRecord<'_>,
        item: ItemId,
    ) -> Option<PreparedSurfaceAction> {
        let record = self.current_tab_list_menu(menu)?;
        record
            .rows()
            .iter()
            .any(|row| row.tab().item == item)
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
        item: ItemId,
        navigation: SurfaceTabNavigation,
    ) -> Option<PreparedSurfaceAction> {
        let tab = self.tab_record(item)?;
        let start = self.plan.selected_in_tabs(tab.id().tabs).unwrap_or(item);
        self.prepare_tab_input_sequence(item, start, [SurfaceTabInput::Navigate(navigation)])
    }

    /// Selects this exact tab while preserving framework focus in its header.
    ///
    /// Use this for an accessibility focus request. Unlike activation, focusing
    /// a tab supersedes older content-focus requests without entering its pane.
    #[must_use]
    pub fn prepare_tab_focus(self, item: ItemId) -> Option<PreparedSurfaceAction> {
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

    /// Folds ordered tab inputs into one action against this exact candidate.
    ///
    /// `item` authorizes the visible receiver; `start` identifies the initial
    /// member in the same strip, which may be clipped. Hosts can use the focused
    /// tab for header keys and the selected tab for document-cycling shortcuts.
    /// Each later input uses the preceding destination.
    /// The last input decides whether header focus is preserved or pane content
    /// is activated. No intermediate selection is published.
    ///
    /// A round trip is handled even when it returns to the initial tab. An empty
    /// sequence or ineffectual navigation returns `None`. Hidden members
    /// remain valid destinations while `item` must be an operable receiver.
    #[must_use]
    pub fn prepare_tab_input_sequence(
        self,
        item: ItemId,
        start: ItemId,
        inputs: impl IntoIterator<Item = SurfaceTabInput>,
    ) -> Option<PreparedSurfaceAction> {
        let current = self
            .plan
            .tab_records()
            .iter()
            .find(|record| record.id().item == item)?;
        self.tab_is_operable(current).then_some(())?;
        let bar =
            self.plan.tab_bar_records().iter().find(|bar| {
                bar.id().root == current.id().root && bar.id().tabs == current.id().tabs
            })?;
        let members = bar.members();
        let mut inputs = inputs.into_iter().peekable();
        inputs.peek()?;
        let mut cursor = members
            .iter()
            .position(|member| member.tab().item == start)?;
        let mut handled = false;
        let mut activate = false;
        for input in inputs {
            match input {
                SurfaceTabInput::Navigate(navigation) => {
                    let next = navigation.into_core().destination(cursor, members.len());
                    handled |= next != cursor;
                    cursor = next;
                    activate = false;
                }
                SurfaceTabInput::Activate => {
                    handled = true;
                    activate = true;
                }
            }
        }
        let target = members[cursor].tab();
        // Focus can be on a visible tab while the selected tab is scrolled out.
        // Home/End must still reveal and focus that selection without changing it.
        (handled || target != *current.id()).then_some(())?;
        let selection = if activate {
            crate::engine::LocalTabSelection::Activate {
                source: *current.id(),
                target,
            }
        } else {
            crate::engine::LocalTabSelection::Navigate {
                source: *current.id(),
                target,
            }
        };
        Some(PreparedSurfaceAction::select_tab(
            self.authority_domain,
            self.version,
            self.scene,
            self.frozen_action_witness()?,
            selection,
        ))
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
    /// Each step clamps to the same touching-layout solver limits as a direct
    /// resize. An endpoint followed by an outward and inward step therefore
    /// finishes one step inside that endpoint. Effective intermediate movement
    /// commits visible proportions even when the final displacement is zero.
    /// Empty sequences return `None`.
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
    /// Every step uses the common reachable displacement of all incident pairs
    /// on this axis. The other axis remains unchanged. Equalizing still requires
    /// a common constrained destination. Effective intermediate movement commits
    /// visible proportions; empty sequences return `None`.
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
            self.surface,
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
        self.plan
            .tab_list_menu_records()
            .iter()
            .find(|record| *record == menu.record)
            .cloned()
    }
}

#[cfg(test)]
mod scroll_tests;
