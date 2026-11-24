//! Qualification of input receivers from the exact semantic output.

use super::*;

/// Semantic receiver class independent of renderer widget identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockspaceReceiverRole {
    PaneBody,
    /// Empty Tabs container chrome; never application Pane content.
    TabsBackground,
    TabBody,
    TabClose,
    TabStripControl,
    TabStripScroll,
    TabListMenuRow,
    TabListMenuScroll,
    TabListMenuBlocker,
    TabListMenuBackdrop,
    /// Dedicated whole-group grip before the first tab.
    TabGroupGrip,
    /// Unoccupied strip space after the final tab.
    TabGroupTrailingEmpty,
    Splitter,
    SplitterJunction,
    ContainedFrame,
    ContainedTitle,
    ContainedClose,
    ContainedResize,
    DropTarget,
}

/// Opaque receiver descriptor captured while painting one semantic output.
///
/// A descriptor is not input authority. It must be rebound after a concrete
/// output is finally presented before an event may name it.
#[derive(Clone, Copy, PartialEq)]
pub struct DockspaceReceiverDescriptor {
    pub(in crate::runtime) output: SurfacePresentationOutputTicket,
    pub(in crate::runtime) region: PresentationHitRegionId,
    pub(super) lanes: PresentationPointerLanes,
    pub(super) role: DockspaceReceiverRole,
    pub(super) bounds: LogicalRect,
    pub(super) center: LogicalPoint,
}

impl fmt::Debug for DockspaceReceiverDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockspaceReceiverDescriptor")
            .field("role", &self.role)
            .field("bounds", &self.bounds)
            .field("center", &self.center)
            .finish()
    }
}

impl DockspaceReceiverDescriptor {
    pub(in crate::runtime) fn from_projection_region(
        output: SurfacePresentationOutputTicket,
        region: PresentationHitRegionId,
        lanes: PresentationPointerLanes,
        bounds: LogicalRect,
    ) -> Option<Self> {
        Some(Self {
            output,
            region,
            lanes,
            role: receiver_role(region.kind())?,
            bounds,
            center: crate::presentation::scene::paint_receiver_center(bounds)?,
        })
    }

    /// Returns a stable visual identity for renderer widget keys.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        DockspaceVisualId(VisualIdentity::Receiver(self.region))
    }

    /// Returns the semantic receiver role.
    #[must_use]
    pub const fn role(self) -> DockspaceReceiverRole {
        self.role
    }

    /// Returns the base receiver rectangle supplied to the renderer.
    ///
    /// An expanded touch target can survive clipping that leaves this empty.
    /// Resolve device-specific input with [`super::DockspaceSession::host_pointer_hit_for_device`]
    /// rather than treating these bounds as the complete touch footprint.
    #[must_use]
    pub const fn bounds(self) -> LogicalRect {
        self.bounds
    }

    /// Returns the center of the exact half-open receiver rectangle.
    #[must_use]
    pub const fn center(self) -> LogicalPoint {
        self.center
    }

    pub(in crate::runtime) const fn supports_lane(self, lane: PresentationPointerLane) -> bool {
        self.lanes.contains(lane)
    }
}

impl<'frame> SurfacePaintPlan<'frame> {
    /// Returns every active semantic receiver in deterministic hit-stack order.
    #[cfg(test)]
    pub(crate) fn receivers(self) -> impl Iterator<Item = DockspaceReceiverDescriptor> + 'frame {
        self.hit_manifest
            .regions()
            .iter()
            .copied()
            .filter(|region| !region.is_passive())
            .filter_map(move |region| {
                DockspaceReceiverDescriptor::from_projection_region(
                    self.output,
                    region.id(),
                    region.lanes(),
                    region.hit().rect(),
                )
            })
    }

    #[must_use]
    pub fn tab_receiver(&self, item: impl Into<TabKey>) -> Option<DockspaceReceiverDescriptor> {
        let tab = self.tab_record(item.into())?;
        self.tab_is_operable(tab).then_some(())?;
        self.receiver(PresentationHitRegionKind::TabBody(*tab.id()))
    }

    /// Returns an empty Tabs container's background receiver from this output.
    #[must_use]
    pub fn receiver_for_tabs(
        &self,
        tabs: TabsPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (tabs.output == self.semantic_output()).then_some(())?;
        let id = tabs.record.id();
        if self.plan.selected_in_tabs(id.tabs).is_some() {
            return None;
        }
        self.receiver(PresentationHitRegionKind::TabsBackground(id))
    }

    /// Returns the exact receiver for a pane body from this output.
    #[must_use]
    pub fn receiver_for_pane(
        &self,
        pane: PanePaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (pane.output == self.semantic_output()).then_some(())?;
        let pane_id = pane.record.id();
        self.receiver(PresentationHitRegionKind::PaneBody(pane_id))
    }

    /// Returns the exact receiver for a tab body from this output.
    #[must_use]
    pub fn receiver_for_tab_body(
        &self,
        tab: TabPaintRecord<'_>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (tab.belongs_to(self) && tab.operable()).then_some(())?;
        let tab_id = *tab.record.id();
        self.receiver(PresentationHitRegionKind::TabBody(tab_id))
    }

    /// Returns the exact receiver for a tab close control from this output.
    #[must_use]
    pub fn receiver_for_tab_close(
        &self,
        tab: TabPaintRecord<'_>,
    ) -> Option<DockspaceReceiverDescriptor> {
        tab.belongs_to(self).then_some(())?;
        let close_bounds = tab.record.close_bounds()?;
        self.plan
            .region_is_operable(close_bounds, tab.record.layer())
            .then_some(())?;
        let tab_id = *tab.record.id();
        self.receiver(PresentationHitRegionKind::TabClose(tab_id))
    }

    /// Returns the exact receiver for a group-drag region from this output.
    #[must_use]
    pub fn receiver_for_tab_group_drag_region(
        &self,
        region: TabGroupDragRegionPaintRecord,
    ) -> Option<DockspaceReceiverDescriptor> {
        region.belongs_to(self).then_some(())?;
        self.receiver(PresentationHitRegionKind::TabGroupDrag {
            bar: region.bar,
            region: region.record.kind(),
        })
    }

    /// Returns the scroll receiver for an overflowing tab strip from this output.
    #[must_use]
    pub fn receiver_for_tab_strip_scroll(
        &self,
        bar: TabBarPaintRecord<'_>,
    ) -> Option<DockspaceReceiverDescriptor> {
        bar.belongs_to(self).then_some(())?;
        let bar_id = *bar.record.id();
        self.receiver(PresentationHitRegionKind::TabStripScroll(bar_id))
    }

    /// Returns the exact receiver for a tab-strip control from this output.
    #[must_use]
    pub fn receiver_for_tab_strip_control(
        &self,
        control: TabStripControlPaintRecord,
    ) -> Option<DockspaceReceiverDescriptor> {
        control.belongs_to(self).then_some(())?;
        let control_id = control.record.id();
        self.receiver(PresentationHitRegionKind::TabStripControl(control_id))
    }

    /// Returns the exact receiver for one visible tab-list row from this output.
    #[must_use]
    pub fn receiver_for_tab_list_menu_row(
        &self,
        row: TabListMenuRowPaintRecord,
    ) -> Option<DockspaceReceiverDescriptor> {
        row.belongs_to(self).then_some(())?;
        let session = row.session();
        let tab = row.record.tab();
        self.receiver(PresentationHitRegionKind::TabListMenuRow { menu: session, tab })
    }

    /// Returns the blocker receiver for an open menu frame from this output.
    #[must_use]
    pub fn receiver_for_tab_list_menu_frame(
        &self,
        menu: TabListMenuPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        menu.backdrop().belongs_to(self).then_some(())?;
        let session = menu.record.session();
        self.receiver(PresentationHitRegionKind::TabListMenuBlocker(session))
    }

    /// Returns the scroll receiver for an open menu from this output.
    #[must_use]
    pub fn receiver_for_tab_list_menu_scroll(
        &self,
        menu: TabListMenuPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        menu.backdrop().belongs_to(self).then_some(())?;
        let session = menu.record.session();
        self.receiver(PresentationHitRegionKind::TabListMenuScroll(session))
    }

    /// Returns the full-surface backdrop receiver for a menu from this output.
    #[must_use]
    pub fn receiver_for_tab_list_menu_backdrop(
        &self,
        backdrop: TabListMenuBackdropPaintRecord,
    ) -> Option<DockspaceReceiverDescriptor> {
        backdrop.belongs_to(self).then_some(())?;
        let session = backdrop.record.session();
        self.receiver(PresentationHitRegionKind::TabListMenuBackdrop(session))
    }

    /// Returns the exact receiver for one painted splitter handle.
    #[must_use]
    pub fn receiver_for_splitter(
        &self,
        splitter: SplitterPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (splitter.output == self.semantic_output()).then_some(())?;
        let splitter_id = *splitter.record.id();
        self.receiver(PresentationHitRegionKind::SplitterHandle(splitter_id))
    }

    /// Returns the exact receiver for one atomic splitter junction.
    #[must_use]
    pub fn receiver_for_splitter_junction(
        &self,
        junction: SplitterJunctionPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (junction.output == self.semantic_output()).then_some(())?;
        self.receiver(PresentationHitRegionKind::SplitterJunction(
            junction.record.id(),
        ))
    }

    /// Returns whether every incident handle accepts one atomic resize gesture.
    #[must_use]
    pub fn splitter_junction_operable(
        &self,
        junction: SplitterJunctionPaintRecord<'frame>,
    ) -> bool {
        junction.output == self.semantic_output()
            && self.plan.partition_available(junction.record.id().root())
            && junction.record.operable()
    }

    /// Returns whether every incident splitter on one junction axis accepts a
    /// scene-bound keyboard or accessibility adjustment.
    #[must_use]
    pub fn splitter_junction_axis_operable(
        &self,
        junction: SplitterJunctionPaintRecord<'frame>,
        axis: DockspaceAxis,
    ) -> bool {
        if junction.output != self.semantic_output()
            || !self.plan.partition_available(junction.record.id().root())
        {
            return false;
        }
        let axis: Axis = axis.into();
        // Runtime plans borrow the validator's canonical, unique record arrays.
        let junctions = self.plan.splitter_junction_records();
        let Ok(index) =
            junctions.binary_search_by_key(&junction.record.id(), SplitterJunctionRecord::id)
        else {
            return false;
        };
        let record = &junctions[index];
        let directions = match axis {
            Axis::Horizontal => [
                SplitterJunctionDirection::North,
                SplitterJunctionDirection::South,
            ],
            Axis::Vertical => [
                SplitterJunctionDirection::East,
                SplitterJunctionDirection::West,
            ],
        };
        let [first, second] = directions.map(|direction| record.id().arm(direction));
        let Some(first) = first.or(second) else {
            return false;
        };
        let splitters = self.plan.splitter_records();
        let is_operable = |id| {
            splitters
                .binary_search_by_key(&id, |splitter| *splitter.id())
                .ok()
                .is_some_and(|index| {
                    let splitter = &splitters[index];
                    splitter.axis() == axis && splitter.operable()
                })
        };
        is_operable(first) && second.filter(|id| *id != first).is_none_or(is_operable)
    }

    /// Returns the exact blocker receiver for one contained floating surface.
    #[must_use]
    pub fn receiver_for_contained_frame(
        &self,
        contained: ContainedPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (contained.output == self.semantic_output()).then_some(())?;
        let floating = contained.record.floating();
        self.receiver(PresentationHitRegionKind::ContainedFrameBlocker(floating))
    }

    /// Returns the exact title-drag receiver for one contained floating surface.
    #[must_use]
    pub fn receiver_for_contained_title(
        &self,
        contained: ContainedPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (contained.output == self.semantic_output() && contained.title_operable()).then_some(())?;
        let floating = contained.record.floating();
        self.receiver(PresentationHitRegionKind::ContainedTitle(floating))
    }

    /// Returns the exact close receiver for one contained floating surface.
    #[must_use]
    pub fn receiver_for_contained_close(
        &self,
        contained: ContainedPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (contained.output == self.semantic_output() && contained.close_operable()).then_some(())?;
        let floating = contained.record.floating();
        self.receiver(PresentationHitRegionKind::ContainedClose(floating))
    }

    /// Returns the exact receiver for this output's contained floating edge.
    #[must_use]
    pub fn receiver_for_contained_resize(
        &self,
        resize: ContainedResizePaintRecord,
    ) -> Option<DockspaceReceiverDescriptor> {
        (resize.output == self.semantic_output() && resize.operable()).then_some(())?;
        let direction = resize.record.direction();
        self.receiver(PresentationHitRegionKind::ContainedResize {
            floating: resize.floating,
            direction,
        })
    }

    /// Returns the exact receiver for a core-selected drop-guide target from this output.
    #[must_use]
    pub fn receiver_for_drop_target(
        &self,
        target: DropAffordanceTargetPaintRecord<'frame>,
    ) -> Option<DockspaceReceiverDescriptor> {
        (target.output == self.semantic_output()).then_some(())?;
        let target_id = target.target_id();
        self.receiver(PresentationHitRegionKind::DropTarget(target_id))
    }

    #[must_use]
    pub fn center_drop_receiver_for_item(
        &self,
        item: ItemId,
    ) -> Option<DockspaceReceiverDescriptor> {
        let tab = self.tab_record(item.into())?;
        let tab = *tab.id();
        self.receiver(PresentationHitRegionKind::DropTarget(
            DropTargetId::Center {
                surface: self.output.surface(),
                root: tab.root,
                tabs: tab.tabs,
            },
        ))
    }

    pub(super) fn receiver(
        &self,
        kind: PresentationHitRegionKind,
    ) -> Option<DockspaceReceiverDescriptor> {
        let region = *self.hit_manifest.region_for_kind(kind)?;
        if region.is_passive() {
            return None;
        }
        DockspaceReceiverDescriptor::from_projection_region(
            self.output,
            region.id(),
            region.lanes(),
            region.hit().rect(),
        )
    }
}

pub(super) const fn receiver_role(
    kind: PresentationHitRegionKind,
) -> Option<DockspaceReceiverRole> {
    Some(match kind {
        PresentationHitRegionKind::PaneBody(_) => DockspaceReceiverRole::PaneBody,
        PresentationHitRegionKind::TabsBackground(_) => DockspaceReceiverRole::TabsBackground,
        PresentationHitRegionKind::TabBody(_) => DockspaceReceiverRole::TabBody,
        PresentationHitRegionKind::TabClose(_) => DockspaceReceiverRole::TabClose,
        PresentationHitRegionKind::TabStripControl(_) => DockspaceReceiverRole::TabStripControl,
        PresentationHitRegionKind::TabStripScroll(_) => DockspaceReceiverRole::TabStripScroll,
        PresentationHitRegionKind::TabListMenuRow { .. } => DockspaceReceiverRole::TabListMenuRow,
        PresentationHitRegionKind::TabListMenuScroll(_) => DockspaceReceiverRole::TabListMenuScroll,
        PresentationHitRegionKind::TabListMenuBlocker(_) => {
            DockspaceReceiverRole::TabListMenuBlocker
        }
        PresentationHitRegionKind::TabListMenuBackdrop(_) => {
            DockspaceReceiverRole::TabListMenuBackdrop
        }
        PresentationHitRegionKind::TabGroupDrag {
            region: TabGroupDragRegionKind::LeadingGrip,
            ..
        } => DockspaceReceiverRole::TabGroupGrip,
        PresentationHitRegionKind::TabGroupDrag {
            region: TabGroupDragRegionKind::TrailingEmpty,
            ..
        } => DockspaceReceiverRole::TabGroupTrailingEmpty,
        PresentationHitRegionKind::SplitterHandle(_) => DockspaceReceiverRole::Splitter,
        PresentationHitRegionKind::SplitterJunction(_) => DockspaceReceiverRole::SplitterJunction,
        PresentationHitRegionKind::ContainedFrameBlocker(_) => {
            DockspaceReceiverRole::ContainedFrame
        }
        PresentationHitRegionKind::ContainedTitle(_) => DockspaceReceiverRole::ContainedTitle,
        PresentationHitRegionKind::ContainedClose(_) => DockspaceReceiverRole::ContainedClose,
        PresentationHitRegionKind::ContainedResize { .. } => DockspaceReceiverRole::ContainedResize,
        PresentationHitRegionKind::DropGuideActivation(_) => return None,
        PresentationHitRegionKind::DropTarget(_) => DockspaceReceiverRole::DropTarget,
    })
}
