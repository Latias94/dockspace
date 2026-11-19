//! Local semantic actions retaining the exact scene that exposed their targets.

use super::*;

impl DockEngine {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reduce_deferred_surface_action(
        &mut self,
        input: InputSequence,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        expected: WorkspaceVersion,
        witness: &crate::presentation::scene::SurfacePlanScene,
        action: &LocalSurfaceAction,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
        interaction_events: &mut Vec<InteractionEvent>,
    ) -> Result<InputOutcome, EngineError> {
        if expected.epoch() != self.version.epoch() {
            return Ok(InputOutcome::StaleRejected {
                expected,
                accepted_base: self.version,
            });
        }
        // Return destinations and native recovery surfaces can lie outside
        // this surface's frozen plan. Keep their durable destination lookup
        // version-bound until a complete destination witness is available.
        let external_destination = match action {
            LocalSurfaceAction::PresentationCommand {
                action: ProductAction::DockBackRoot { .. },
                ..
            } => true,
            LocalSurfaceAction::PresentationCommand {
                action: ProductAction::FloatRoot { surface, .. },
                ..
            } => *surface != witness.stamp().surface(),
            _ => false,
        };
        if external_destination && expected != self.version {
            return Ok(InputOutcome::StaleRejected {
                expected,
                accepted_base: self.version,
            });
        }
        if self
            .validate_deferred_surface_action(witness, action, policy)
            .is_err()
        {
            return Ok(InputOutcome::StaleRejected {
                expected,
                accepted_base: self.version,
            });
        }

        let outcome = match action {
            LocalSurfaceAction::SelectTab { selection, .. } => self.select_tab_from_candidate(
                input,
                focus_causal,
                witness,
                *selection,
                policy,
                events,
                interaction_events,
            ),
            LocalSurfaceAction::Close { target, .. } => {
                self.close_from_candidate(input, witness, *target, policy)
            }
            LocalSurfaceAction::CloseRoot { root, .. } => {
                self.close_root_from_candidate(input, witness, *root, policy)
            }
            LocalSurfaceAction::CloseTabs { group, .. } => {
                let target = ContentCloseTarget::TabsSource {
                    source: group.source.clone(),
                };
                self.request_deferred_content_close(input, target, policy)
            }
            LocalSurfaceAction::PresentationCommand { action, .. } => self
                .reduce_presentation_command_from_candidate(
                    input,
                    cause,
                    focus_causal,
                    witness,
                    action.clone(),
                    policy,
                    events,
                    interaction_events,
                ),
            LocalSurfaceAction::TabChrome { action, .. } => self.apply_tab_chrome_from_candidate(
                cause,
                focus_causal,
                witness,
                action,
                policy,
                events,
            ),
            LocalSurfaceAction::AdjustSplitter {
                splitter,
                adjustment,
                ..
            } => {
                let outcome = self.adjust_splitter_from_candidate(
                    input,
                    witness,
                    *splitter,
                    adjustment.clone(),
                    policy,
                    events,
                    interaction_events,
                )?;
                Ok(InputOutcome::InteractionProcessed {
                    outcome,
                    version: self.version,
                })
            }
            LocalSurfaceAction::AdjustSplitterJunction {
                junction,
                axis,
                adjustment,
                ..
            } => {
                let outcome = self.adjust_splitter_junction_from_candidate(
                    input,
                    witness,
                    *junction,
                    *axis,
                    adjustment.clone(),
                    policy,
                    events,
                    interaction_events,
                )?;
                Ok(InputOutcome::InteractionProcessed {
                    outcome,
                    version: self.version,
                })
            }
            LocalSurfaceAction::ResetSplitter {
                splitter, reset, ..
            } => {
                let outcome = self.reset_splitter_from_candidate(
                    input,
                    witness,
                    *splitter,
                    *reset,
                    policy,
                    events,
                    interaction_events,
                )?;
                Ok(InputOutcome::InteractionProcessed {
                    outcome,
                    version: self.version,
                })
            }
            LocalSurfaceAction::AdjustContainedResize {
                floating,
                direction,
                delta,
                ..
            } => self.resize_contained_from_candidate(
                input,
                witness,
                *floating,
                *direction,
                *delta,
                policy,
                events,
                interaction_events,
            ),
            LocalSurfaceAction::CancelGesture { delivery, .. } => {
                self.reduce_gesture_cancel_input(input, self.version, *delivery, interaction_events)
            }
        }?;
        Ok(match outcome {
            InputOutcome::CommandProcessed {
                changed, version, ..
            } if matches!(action, LocalSurfaceAction::SelectTab { .. }) => {
                let LocalSurfaceAction::SelectTab { selection, .. } = action else {
                    unreachable!("the guard selects the same action variant");
                };
                InputOutcome::ProductActionProcessed {
                    outcome: crate::model::DockspaceActionOutcome::Selected {
                        tab: selection.target().tab,
                        changed,
                    },
                    version,
                }
            }
            InputOutcome::InteractionProcessed {
                outcome: InteractionOutcome::Rejected(_),
                ..
            }
            | InputOutcome::CommandRejected { .. } => InputOutcome::StaleRejected {
                expected,
                accepted_base: self.version,
            },
            other => other,
        })
    }

    fn validate_deferred_surface_action(
        &self,
        witness: &crate::presentation::scene::SurfacePlanScene,
        action: &LocalSurfaceAction,
        policy: &DockPolicySnapshot,
    ) -> Result<(), InteractionRejection> {
        let stamp = witness.stamp();
        let ticket = stamp.requirement();
        let surface = stamp.surface();
        let plan = witness.plan();
        if ticket.authority_domain() != self.authority_domain
            || ticket.workspace_epoch() != self.version.epoch()
            || ticket.policy() != policy.revision()
            || ticket.config() != self.presentation_authority.presentation_config_revision
            || plan.measurement_ticket() != Some(ticket)
            || plan.surface() != surface
            || action.scene().is_some_and(|scene| scene != stamp)
        {
            return Err(InteractionRejection::StaleScene);
        }
        if self.workspace.surface(surface).is_none()
            || self
                .presentation_authority
                .presentation_requirements
                .surface(surface)
                .is_none()
            || plan.popup()
                != self
                    .presentation_authority
                    .tab_strip_states
                    .popup_requirement()
            || !self.presented_layering_is_current(plan)
            || if matches!(action, LocalSurfaceAction::ResetSplitter { .. }) {
                !self.presented_contained_geometry_is_current(plan)
            } else {
                !self.presented_transient_geometry_is_current(plan)
            }
        {
            return Err(InteractionRejection::TargetAuthorityInvalid);
        }
        if !Self::coordinate_capture_matches_current(
            witness.coordinate_capture(),
            self.viewport.viewport(surface),
            self.viewport.surface_coordinate_authority(surface),
        ) {
            return Err(InteractionRejection::TargetAuthorityInvalid);
        }
        // A sibling floating can move over a frozen receiver without changing
        // z-order. Its current rectangle is part of the receiver's authority.
        if !plan.contained_records().iter().all(|record| {
            self.workspace.presentation_for_root(record.root())
                == Some(crate::graph::workspace::RootPresentationOwner::Contained {
                    surface,
                    floating: record.floating(),
                })
                && plan.unclipped_contained_bounds(record.floating())
                    == self
                        .contained_layout_projection(surface)
                        .filter(|projection| projection.floating == record.floating())
                        .map(|projection| projection.rect)
                        .or_else(|| {
                            self.workspace
                                .contained_floating(record.floating())
                                .map(|current| current.rect)
                        })
        }) {
            return Err(InteractionRejection::TargetAuthorityInvalid);
        }

        let root_is_current = |root| self.presented_root_structure_is_current(plan, root);
        let item_receiver_is_current = |kind| {
            witness
                .hit_manifest()
                .region_for_kind(kind)
                .is_some_and(|region| self.presented_receiver_is_current(plan, region.id()))
        };
        let tabs_group_menu_is_current = |group: &crate::model::PreparedTabsGroup| {
            group.expected == self.version
                && self.capture_product_tabs(group).is_ok()
                && plan.tab_bar_records().iter().any(|bar| {
                    bar.id().root == group.source.root()
                        && bar.id().tabs == group.source.node()
                        && plan.receiver_active_path_is_current(
                            PresentationHitRegionKind::TabStripScroll(*bar.id()),
                            &self.workspace,
                        )
                        && bar
                            .group_menu_bounds()
                            .is_some_and(|bounds| plan.region_is_operable(bounds, bar.layer()))
                })
        };
        let current = match action {
            LocalSurfaceAction::SelectTab { selection, .. } => {
                item_receiver_is_current(PresentationHitRegionKind::TabBody(selection.receiver()))
                    && match selection {
                        LocalTabSelection::Activate(_) => true,
                        LocalTabSelection::Navigate { source, .. }
                        | LocalTabSelection::NavigateAndActivate { source, .. } => {
                            root_is_current(source.root)
                                && plan
                                    .tab_bar_records()
                                    .iter()
                                    .find(|bar| {
                                        bar.id().root == source.root && bar.id().tabs == source.tabs
                                    })
                                    .is_some_and(|bar| {
                                        self.presentation_authority
                                            .tab_strip_states
                                            .state(TabStripStateKey::new(surface, *bar.id()))
                                            .is_some_and(|state| {
                                                state.scroll_offset().is_none_or(|offset| {
                                                    offset == bar.scroll_offset()
                                                }) && state.resolved_viewport_width().is_none_or(
                                                    |width| width == bar.layout_viewport().width(),
                                                )
                                            })
                                    })
                        }
                    }
            }
            LocalSurfaceAction::Close {
                target: CloseSceneTarget::Tab(tab),
                ..
            } => item_receiver_is_current(PresentationHitRegionKind::TabClose(*tab)),
            LocalSurfaceAction::Close {
                target: CloseSceneTarget::TabBody(tab),
                ..
            } => item_receiver_is_current(PresentationHitRegionKind::TabBody(*tab)),
            LocalSurfaceAction::Close {
                target: CloseSceneTarget::Pane(pane),
                ..
            } => item_receiver_is_current(PresentationHitRegionKind::PaneBody(*pane)),
            LocalSurfaceAction::Close {
                target: CloseSceneTarget::Contained(floating),
                ..
            } => plan
                .contained_record(*floating)
                .is_some_and(|record| root_is_current(record.root())),
            LocalSurfaceAction::AdjustContainedResize { floating, .. } => {
                plan.contained_record(*floating)
                    .is_some_and(|record| root_is_current(record.root()))
                    && self.deferred_resize_geometry_is_current(witness, action)
            }
            LocalSurfaceAction::CloseRoot { root, .. } => root_is_current(*root),
            LocalSurfaceAction::CloseTabs { group, .. } => tabs_group_menu_is_current(group),
            LocalSurfaceAction::PresentationCommand { action, .. } => {
                self.product_presentation_source_root(action)
                    .is_some_and(root_is_current)
                    && match action {
                        ProductAction::DockItem { item, .. }
                        | ProductAction::FloatItem { item, .. }
                        | ProductAction::TearOffNative {
                            source: crate::model::DockMoveSource::Item(item),
                            ..
                        } => plan
                            .pane_records()
                            .iter()
                            .find(|pane| pane.item() == *item)
                            .is_some_and(|pane| {
                                item_receiver_is_current(PresentationHitRegionKind::PaneBody(
                                    pane.id(),
                                ))
                            }),
                        ProductAction::DockTab { tab, .. }
                        | ProductAction::FloatTab { tab, .. }
                        | ProductAction::TearOffNative {
                            source: crate::model::DockMoveSource::Tab(tab),
                            ..
                        } => {
                            self.capture_product_tab(tab).is_ok()
                                && item_receiver_is_current(PresentationHitRegionKind::TabBody(
                                    crate::presentation::scene::TabSceneId {
                                        root: tab.source.root(),
                                        tabs: tab.source.tabs(),
                                        tab: tab.key(),
                                    },
                                ))
                        }
                        ProductAction::FloatTabs { group, .. }
                        | ProductAction::DockTabs { group, .. }
                        | ProductAction::TearOffNative {
                            source: crate::model::DockMoveSource::Tabs(group),
                            ..
                        } => tabs_group_menu_is_current(group),
                        _ => true,
                    }
                    && self.deferred_presentation_destination_is_current(plan, action)
            }
            LocalSurfaceAction::TabChrome { action, .. } => {
                self.deferred_tab_chrome_is_current(plan, action)
            }
            LocalSurfaceAction::AdjustSplitter { splitter, .. } => {
                root_is_current(splitter.root)
                    && self.deferred_resize_geometry_is_current(witness, action)
            }
            LocalSurfaceAction::ResetSplitter { splitter, .. } => {
                self.deferred_splitter_reset_is_current(witness, *splitter)
            }
            LocalSurfaceAction::AdjustSplitterJunction { junction, .. } => {
                junction
                    .splitters()
                    .iter()
                    .all(|splitter| root_is_current(splitter.root))
                    && self.deferred_resize_geometry_is_current(witness, action)
            }
            LocalSurfaceAction::CancelGesture { status, .. } => {
                *status == self.interaction.status() && *status != InteractionStatus::Idle
            }
        };
        current
            .then_some(())
            .ok_or(InteractionRejection::TargetAuthorityInvalid)
    }

    fn deferred_splitter_reset_is_current(
        &self,
        witness: &crate::presentation::scene::SurfacePlanScene,
        splitter: SplitterSceneId,
    ) -> bool {
        let plan = witness.plan();
        if !self.presented_root_topology_is_current(plan, splitter.root)
            || !plan.partition_available(splitter.root)
            || !plan.splitter_record(splitter).is_some_and(|record| {
                record.operable() && plan.region_is_operable(record.hit().rect(), record.layer())
            })
            || !plan.receiver_active_path_is_current(
                PresentationHitRegionKind::SplitterHandle(splitter),
                &self.workspace,
            )
            || !plan
                .pane_records()
                .iter()
                .filter(|pane| pane.id().root == splitter.root)
                .all(|pane| {
                    self.workspace.item_is_active(pane.item())
                        && self
                            .workspace
                            .locate_item_by_id(pane.item())
                            .is_some_and(|location| {
                                location.root == pane.id().root
                                    && location.owner_node() == pane.owner_node()
                            })
                })
        {
            return false;
        }
        let Some(frozen) = plan.layout_facts() else {
            return false;
        };
        self.scene()
            .measurements_are_current(witness.stamp().surface(), frozen.measurements())
    }

    fn deferred_resize_geometry_is_current(
        &self,
        witness: &crate::presentation::scene::SurfacePlanScene,
        action: &LocalSurfaceAction,
    ) -> bool {
        let Some(current) = self.scene().surface(witness.stamp().surface()) else {
            return false;
        };
        if current.stamp().requirement() != witness.stamp().requirement() {
            return false;
        }
        match current {
            SurfaceScene::Ready(ready) => match action {
                LocalSurfaceAction::AdjustContainedResize { floating, .. } => {
                    ready.plan().bounds() == witness.plan().bounds()
                        && ready
                            .plan()
                            .contained_record(*floating)
                            .zip(witness.plan().contained_record(*floating))
                            .is_some_and(|(current, frozen)| {
                                current.minimum_size() == frozen.minimum_size()
                            })
                }
                LocalSurfaceAction::AdjustSplitter { .. }
                | LocalSurfaceAction::AdjustSplitterJunction { .. } => ready
                    .plan()
                    .layout_facts()
                    .zip(witness.plan().layout_facts())
                    .is_some_and(|(current, frozen)| {
                        current.measurements() == frozen.measurements()
                    }),
                _ => false,
            },
            // A discarded UI pass can supply the first exact geometry for the
            // current requirements. An explicit failed observation cannot.
            SurfaceScene::Bootstrap(scene) => matches!(
                scene.reason(),
                crate::presentation::scene::BootstrapSurfaceSceneReason::AwaitingContribution
                    | crate::presentation::scene::BootstrapSurfaceSceneReason::RequirementsChanged
                    | crate::presentation::scene::BootstrapSurfaceSceneReason::WorkspaceReplaced
            ),
            SurfaceScene::Stale(scene) => matches!(
                scene.reason(),
                crate::presentation::scene::StaleSurfaceSceneReason::RequirementsChanged
            ),
        }
    }

    fn deferred_presentation_destination_is_current(
        &self,
        plan: &PresentationPlan,
        action: &ProductAction,
    ) -> bool {
        use crate::model::{DockAnchor, DockPlacement};
        let placement = match action {
            ProductAction::DockRoot { placement, .. }
            | ProductAction::DockTabs { placement, .. }
            | ProductAction::DockTab { placement, .. } => placement,
            _ => return true,
        };
        let anchor = match *placement {
            DockPlacement::Center(anchor) | DockPlacement::InnerEdge { anchor, .. } => anchor,
            DockPlacement::Before(item) | DockPlacement::After(item) => DockAnchor::Item(item),
            DockPlacement::OuterEdge { root, .. } => DockAnchor::Central(root),
            DockPlacement::Main(surface) => {
                return surface == plan.surface()
                    && self
                        .workspace
                        .surface(surface)
                        .is_some_and(|current| current.main_root.is_none())
                    && plan.layout_facts().is_some_and(|facts| {
                        facts.roots().all(|root| {
                            plan.contained_records()
                                .iter()
                                .any(|record| record.root() == root.root())
                        })
                    });
            }
        };
        let root = match anchor {
            DockAnchor::Node(key) => {
                let Some(root) = self.workspace.node_root(key.0) else {
                    return false;
                };
                if !plan
                    .layout_facts()
                    .and_then(|facts| facts.root(root))
                    .and_then(|facts| facts.projection().nodes.get(&key.0))
                    .is_some_and(|result| result.visible)
                {
                    return false;
                }
                let mut node = key.0;
                while let Some(parent) = self.workspace.parent_link(root, node) {
                    if matches!(self.workspace.node(parent.parent), Some(Node::Tabs { .. }))
                        && self.workspace.visible_tab_selection(parent.parent)
                            != Some(TabKey::for_layout(node))
                    {
                        return false;
                    }
                    node = parent.parent;
                }
                root
            }
            DockAnchor::Central(root) => root,
            DockAnchor::Item(item) => {
                let Some(tab) = plan
                    .tab_records()
                    .iter()
                    .find(|record| record.id().tab.item() == Some(item))
                else {
                    return false;
                };
                let tab = tab.id();
                if !self
                    .workspace
                    .locate_item_by_id(item)
                    .is_some_and(|location| {
                        location.root == tab.root && location.tabs() == Some(tab.tabs)
                    })
                {
                    return false;
                }
                tab.root
            }
        };
        self.presented_root_structure_is_current(plan, root)
    }

    fn deferred_tab_chrome_is_current(
        &self,
        plan: &PresentationPlan,
        action: &LocalTabChromeAction,
    ) -> bool {
        if let LocalTabChromeAction::DismissMenuFromBackdrop(backdrop) = action {
            return self
                .validate_tab_list_menu_popup(plan, backdrop.session, backdrop.revision)
                .is_ok()
                && backdrop.record.session() == backdrop.session
                && backdrop.record.revision() == backdrop.revision
                && plan
                    .tab_list_menu_backdrop_records()
                    .contains(&backdrop.record);
        }
        let key = match action {
            LocalTabChromeAction::ActivateControl(control) => control.key,
            LocalTabChromeAction::ScrollStrip { key, .. } => *key,
            LocalTabChromeAction::ActivateMenuRow(row) => row.session.key(),
            LocalTabChromeAction::DismissMenu { session, .. }
            | LocalTabChromeAction::ScrollMenu { session, .. }
            | LocalTabChromeAction::NavigateMenu { session, .. } => session.key(),
            LocalTabChromeAction::DismissMenuFromBackdrop(backdrop) => backdrop.session.key(),
        };
        let owner_is_current = match self.workspace.presentation_for_root(key.bar().root) {
            Some(
                crate::graph::workspace::RootPresentationOwner::Main { surface }
                | crate::graph::workspace::RootPresentationOwner::Contained { surface, .. },
            ) => surface == key.surface(),
            None => false,
        };
        if key.surface() != plan.surface() || !owner_is_current {
            return false;
        }
        let Some(bar) = plan
            .tab_bar_records()
            .iter()
            .find(|record| *record.id() == key.bar())
        else {
            return false;
        };
        if !matches!(self.workspace.node(key.bar().tabs), Some(Node::Tabs { .. }))
            || !self
                .workspace
                .visible_tab_keys(key.bar().tabs)
                .iter()
                .copied()
                .eq(bar.members().iter().map(|member| member.tab().tab))
        {
            return false;
        }
        match action {
            LocalTabChromeAction::ActivateControl(_) | LocalTabChromeAction::ScrollStrip { .. } => {
                self.presentation_authority
                    .tab_strip_states
                    .state(key)
                    .is_some_and(|state| {
                        state
                            .scroll_offset()
                            .is_none_or(|offset| offset == bar.scroll_offset())
                    })
            }
            _ => self
                .presentation_authority
                .tab_strip_states
                .active_menu_for(key)
                .is_some_and(|active| {
                    plan.tab_list_menu_records()
                        .iter()
                        .find(|record| record.session() == active.session())
                        .is_some_and(|record| {
                            record.scroll_offset() == active.scroll_offset()
                                && record
                                    .rows()
                                    .iter()
                                    .find(|row| row.focused())
                                    .map(|row| row.tab().tab)
                                    == Some(active.focus())
                                && record
                                    .rows()
                                    .iter()
                                    .map(|row| row.tab().tab)
                                    .eq(active.tabs().iter().copied())
                        })
                }),
        }
    }

    fn close_from_candidate(
        &mut self,
        input: InputSequence,
        candidate: &crate::presentation::scene::SurfacePlanScene,
        target: CloseSceneTarget,
        policy: &DockPolicySnapshot,
    ) -> Result<InputOutcome, EngineError> {
        let plan = candidate.plan();
        let (bounds, layer, content) = match self.scene_close_region(plan, target) {
            Ok(region) => region,
            Err(error) => return Ok(self.candidate_action_rejection(error)),
        };
        if !plan.region_is_operable(bounds, layer) {
            return Ok(self.candidate_action_rejection(
                InteractionRejection::CloseActivationOccluded { target },
            ));
        }
        self.request_deferred_content_close(input, content, policy)
    }

    fn close_root_from_candidate(
        &mut self,
        input: InputSequence,
        candidate: &crate::presentation::scene::SurfacePlanScene,
        root: RootId,
        policy: &DockPolicySnapshot,
    ) -> Result<InputOutcome, EngineError> {
        let plan = candidate.plan();
        if !plan
            .presentation_menu_anchor_records()
            .iter()
            .any(|anchor| {
                anchor.root() == root
                    && anchor
                        .ready_bounds()
                        .is_some_and(|bounds| plan.region_is_operable(bounds, anchor.layer()))
            })
        {
            return Ok(InputOutcome::ContentCloseRejected {
                target: ContentCloseTarget::Root(root),
                reason: ContentCloseRequestRejection::RootUnavailable { root },
                version: self.version,
            });
        }
        self.request_deferred_content_close(input, ContentCloseTarget::Root(root), policy)
    }

    fn request_deferred_content_close(
        &mut self,
        input: InputSequence,
        target: ContentCloseTarget,
        policy: &DockPolicySnapshot,
    ) -> Result<InputOutcome, EngineError> {
        let outcome = self.reduce_content_close_request(input, self.version, target, policy)?;
        Ok(match outcome {
            InputOutcome::ContentCloseRequested {
                plan,
                reused,
                version,
                ..
            } => InputOutcome::InteractionProcessed {
                outcome: InteractionOutcome::CloseRequested { plan, reused },
                version,
            },
            other => other,
        })
    }
}
