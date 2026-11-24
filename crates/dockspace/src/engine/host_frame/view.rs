//! Read-only queries against the current host-frame candidate.

use super::*;

impl<'frame> HostFrameView<'frame> {
    pub(crate) fn tab_list_opening_output(
        &self,
        session: TabListMenuSessionId,
        revision: PopupRoutingRevision,
    ) -> Option<SurfacePresentationOutputTicket> {
        let states = &self.engine.presentation_authority.tab_strip_states;
        let active = states.active_menu_for(session.key())?;
        (active.session() == session && states.popup_requirement().revision() == revision)
            .then_some(())?;
        active.opening_output(revision)
    }

    pub(crate) fn tab_content_focus_target(
        &self,
        tab: crate::presentation::scene::TabSceneId,
    ) -> Option<ItemId> {
        self.engine.tab_content_focus_target(tab)
    }

    pub(crate) fn checked_splitter_resize_range(
        &self,
        plan: &PresentationPlan,
        splitter: crate::presentation::scene::SplitterSceneId,
    ) -> Option<crate::interaction::ResizeDeltaInterval> {
        self.engine.checked_splitter_resize_range(plan, splitter)
    }

    pub(crate) fn prepare_splitter_adjustments(
        &self,
        plan: &PresentationPlan,
        target: crate::presentation::scene::SplitterResizeTarget,
        axis: Axis,
        step_size: f64,
        steps: impl IntoIterator<Item = crate::runtime::SurfaceSplitterAdjustment>,
    ) -> Option<crate::interaction::ResizeAdjustment> {
        self.engine
            .prepare_splitter_adjustments(plan, target, axis, step_size, steps)
    }

    /// Returns the post-observation workspace visible to this frame.
    #[must_use]
    pub fn workspace(&self) -> &'frame Workspace {
        self.engine.workspace()
    }

    /// Returns the post-observation workspace version visible to this frame.
    #[must_use]
    pub const fn version(&self) -> WorkspaceVersion {
        self.engine.version()
    }

    pub(crate) const fn authority_domain(&self) -> EngineAuthorityDomainId {
        self.engine.authority_domain()
    }

    #[cfg(feature = "serde")]
    pub(crate) const fn presentation_identity_frontier(&self) -> PresentationIdentityFrontier {
        self.engine.presentation_identity_frontier()
    }

    /// Returns the exact policy used by this frame's candidate.
    #[must_use]
    #[cfg(test)]
    pub const fn policy(&self) -> &'frame DockPolicy {
        self.engine.policy()
    }

    /// Returns the validated presentation configuration used by this frame.
    #[must_use]
    pub const fn presentation_config(&self) -> &'frame DockPresentationConfig {
        self.engine.presentation_config()
    }

    pub(crate) fn prepare_product_action_if_available(
        &self,
        action: crate::model::ProductAction,
    ) -> Result<
        Result<crate::model::PreparedDockAction, crate::model::DockspaceActionRejection>,
        EngineError,
    > {
        self.engine.prepare_product_action_if_available(action)
    }

    pub(crate) fn root_close_is_available(&self, root: RootId) -> bool {
        self.engine
            .capture_content_close(
                crate::graph::command::ContentCloseTarget::Root(root),
                self.engine.policy_snapshot(),
            )
            .is_ok()
    }

    pub(crate) fn tabs_close_is_available(&self, group: &crate::model::PreparedTabsGroup) -> bool {
        self.engine.capture_product_tabs(group).is_ok()
            && self
                .engine
                .capture_content_close(
                    crate::graph::command::ContentCloseTarget::TabsSource {
                        source: group.source.clone(),
                    },
                    self.engine.policy_snapshot(),
                )
                .is_ok()
    }

    pub(crate) fn tab_close_is_available(
        &self,
        tab: crate::presentation::scene::TabSceneId,
    ) -> bool {
        self.engine
            .workspace
            .capture_tab_source(tab.root, tab.tabs, tab.tab)
            .ok()
            .is_some_and(|source| {
                self.engine
                    .capture_content_close(
                        ContentCloseTarget::Tab { source },
                        self.engine.policy_snapshot(),
                    )
                    .is_ok()
            })
    }

    pub(crate) fn item_close_is_available(&self, item: ItemId) -> bool {
        self.engine
            .capture_content_close(
                crate::graph::command::ContentCloseTarget::Item(item),
                self.engine.policy_snapshot(),
            )
            .is_ok()
    }

    pub(crate) fn default_contained_float_rect(
        &self,
        root: RootId,
        surface: SurfaceId,
        plan: &crate::presentation::scene::PresentationPlan,
    ) -> Result<crate::geometry::LogicalRect, crate::model::DockspaceActionRejection> {
        self.engine
            .derive_product_root_float_rect_from_plan(root, surface, plan)
    }

    pub(crate) fn root_is_bound_native_main(&self, root: RootId) -> bool {
        self.engine.root_is_bound_native_main(root)
    }

    pub(crate) fn native_main_recovery_surface(
        &self,
        root: RootId,
    ) -> Result<Option<SurfaceId>, crate::model::DockspaceActionRejection> {
        self.engine.native_main_float_target(root)
    }

    pub(crate) fn directional_dock_placement(
        &self,
        root: RootId,
        edge: Option<crate::model::DockEdge>,
    ) -> Result<crate::model::DockPlacement, crate::model::DockspaceActionRejection> {
        self.engine
            .derive_product_directional_dock_placement(root, edge)
    }

    /// Returns the exact semantic requirement manifest visible to this frame.
    #[must_use]
    pub const fn presentation_requirements(&self) -> &'frame SceneRequirementManifest {
        self.engine.presentation_requirements()
    }

    /// Returns the exact immutable source resources retained for native admission.
    ///
    /// This roster is independent of output slots. Resources stay pinned until
    /// exact first-live ownership transfer or the completion of native cleanup.
    #[cfg(test)]
    pub fn retained_native_staging_resources(
        &self,
    ) -> impl ExactSizeIterator<Item = &NativeStagingResourceDescriptor> + '_ {
        self.engine.viewport.retained_native_staging_resources()
    }

    /// Returns the post-observation authoritative scene state.
    #[must_use]
    pub const fn scene(&self) -> &'frame SurfaceSceneSet {
        self.engine.scene()
    }

    /// Returns the post-observation interaction state.
    #[must_use]
    pub const fn interaction(&self) -> &'frame InteractionState {
        self.engine.interaction()
    }

    /// Returns the transient interaction tokens frozen for one surface paint.
    #[must_use]
    pub fn presentation_interaction(
        &self,
        surface: SurfaceId,
    ) -> Option<HostInteractionPresentation> {
        self.presentation_roster
            .surface_output(surface)
            .and_then(|output| output.payload().interaction())
    }

    /// Returns the exact frozen drag preview geometry which this surface must paint.
    #[must_use]
    pub fn presentation_drag_preview(
        &self,
        surface: SurfaceId,
    ) -> Option<&'frame crate::interaction::InteractionPreview> {
        let expected = self.presentation_interaction(surface)?.drag_preview()?;
        self.engine
            .presentation_preview()
            .filter(|preview| preview.token() == expected && preview.visual().surface() == surface)
    }

    /// Reads the presented target and captured payload of an existing preview.
    /// No candidate transaction or source-membership reconstruction is performed.
    pub(crate) fn presentation_dock_target_context(
        &self,
        preview: &crate::interaction::InteractionPreview,
    ) -> Option<(
        &'frame PresentationPlan,
        crate::interaction::targeting::targets::DropTargetId,
        Option<&'frame MovePayload>,
    )> {
        let PreviewVisual::Dock {
            target, surface, ..
        } = *preview.visual()
        else {
            return None;
        };
        let token = preview.token();
        let drag = self
            .engine
            .interaction
            .active_drag(token.session())
            .ok()
            .or_else(|| {
                self.engine
                    .pending_mutation
                    .as_ref()
                    .and_then(PendingMutation::drag_release)
                    .filter(|pending| pending.preview == token)
                    .map(|pending| &pending.drag)
            });
        let (published, payload) = match drag {
            Some(drag) => (drag.preview()?, Some(&drag.payload)),
            None => {
                let pending = self.engine.pending_mutation.as_ref()?.rehome()?;
                let payload = match pending.preview.proof() {
                    PreviewProof::PresentationRehome {
                        command: WorkspaceCommand::Move { payload, .. },
                    } => Some(payload),
                    _ => None,
                };
                (&pending.preview, payload)
            }
        };
        if published.public().token() != token {
            return None;
        }
        let presented = self.engine.scene().interaction_projection(surface)?;
        let plan = presented.plan();
        if presented.output().stamp() != token.scene() {
            let projection = plan.contained_projection()?;
            if projection.owner
                != crate::presentation::scene::ContainedLayoutOwner::Drag(token.session())
                || projection.target_basis.is_none()
                || projection.target_basis != published.target_layout_basis()
                || target.root() == Some(projection.root)
            {
                return None;
            }
        }
        Some((plan, target, payload))
    }

    /// Returns the exact frozen contained-transform preview which this surface must paint.
    #[must_use]
    pub fn presentation_contained_transform_preview(
        &self,
        surface: SurfaceId,
    ) -> Option<&'frame ContainedTransformPreview> {
        let expected = self
            .presentation_interaction(surface)?
            .contained_transform_preview()?;
        self.engine
            .presentation_contained_transform_preview()
            .filter(|preview| preview.token() == expected && preview.surface() == surface)
    }

    /// Returns the current receiver-authoritative projection for one surface.
    #[must_use]
    pub fn interaction_projection(
        &self,
        surface: SurfaceId,
    ) -> Option<SurfaceInteractionProjection<'frame>> {
        self.receiver_presentations
            .get(&surface)
            .map(JournalSurfacePresentation::projection)
    }

    /// Returns the exact prepared-action projection for the test host roster.
    /// host-frame roster. Unlike pointer receiver authority, semantic actions
    /// are not scoped to the surface that supplied a local pointer provider.
    #[must_use]
    #[cfg(test)]
    pub fn prepared_action_projection(
        &self,
        surface: SurfaceId,
    ) -> Option<SurfaceInteractionProjection<'frame>> {
        self.prepared_presentations
            .get(&surface)
            .map(JournalSurfacePresentation::projection)
    }

    /// Resolves the unique core-owned HoverDrop receiver at one logical point.
    ///
    /// The returned receipt fact owns its winner or `EmptyLane` disposition
    /// and is atomically bound to this sealed frame's exact output, presented
    /// authority, and point. Framework occlusion remains an adapter fact: call
    /// this only after the framework authoritatively proves that no foreign
    /// receiver blocks docking at the point.
    pub fn resolve_hover_drop_receiver(
        &self,
        surface: SurfaceId,
        point: crate::geometry::LogicalPoint,
    ) -> Result<PointerReceiverHoverHit, HostFrameHoverDropResolutionError> {
        let projection = self.interaction_projection(surface).ok_or(
            HostFrameHoverDropResolutionError::InteractionProjectionUnavailable { surface },
        )?;
        let disposition = match projection
            .hit_manifest()
            .resolve_exclusive_with_suppression(
                PresentationPointerLane::HoverDrop,
                point,
                self.engine.hover_drop_suppression(),
            ) {
            Ok(Some(region)) => PointerReceiverHoverHitDisposition::Dock(region.id()),
            Ok(None) => PointerReceiverHoverHitDisposition::EmptyLane,
            Err(PresentationHitResolutionError::Ambiguous { first, second, .. }) => {
                return Err(HostFrameHoverDropResolutionError::Ambiguous { first, second });
            }
        };
        PointerReceiverHoverHit::new(projection, point, disposition)
            .map_err(|source| HostFrameHoverDropResolutionError::ObservationBinding { source })
    }

    /// Prepares one keyboard, accessibility, or other non-pointer tab-strip control activation.
    ///
    /// The control must exist and be enabled in this sealed frame's exact
    /// receiver-authoritative output. The resulting value is opaque and must be
    /// submitted as [`EngineInput::ActivateTabStripControl`]; reduction rechecks
    /// the same presented authority, popup revision, and control record.
    #[cfg(test)]
    pub fn prepare_tab_strip_control_activation(
        &self,
        surface: SurfaceId,
        control: TabStripControlId,
    ) -> Result<PreparedTabStripControlActivation, InteractionRejection> {
        let key = TabStripStateKey::new(surface, control.bar());
        let projection = self
            .prepared_action_projection(surface)
            .ok_or(InteractionRejection::TabStripSourceUnavailable { key })?;
        self.engine
            .prepare_presented_tab_strip_control(projection, control)
    }

    /// Prepares one keyboard or accessibility activation of an exact active menu row.
    ///
    /// The row is resolved against the sole active popup session in this sealed
    /// output. The opaque result cannot be retargeted to another item, session,
    /// popup revision, or presented scene before reduction.
    #[cfg(test)]
    pub fn prepare_tab_list_menu_row_activation(
        &self,
        surface: SurfaceId,
        session: crate::presentation::tab_strip::TabListMenuSessionId,
        tab: crate::presentation::scene::TabSceneId,
    ) -> Result<PreparedTabListMenuRowActivation, InteractionRejection> {
        let projection = self
            .prepared_action_projection(surface)
            .ok_or(InteractionRejection::TabListMenuSessionUnavailable { session })?;
        self.engine
            .prepare_presented_tab_list_menu_row(projection, session, tab)
    }

    /// Prepares one Escape, keyboard, or programmatic dismissal of the exact
    /// active tab-list menu.
    #[cfg(test)]
    pub fn prepare_tab_list_menu_dismiss(
        &self,
        surface: SurfaceId,
        session: crate::presentation::tab_strip::TabListMenuSessionId,
    ) -> Result<PreparedTabListMenuDismiss, InteractionRejection> {
        let projection = self
            .prepared_action_projection(surface)
            .ok_or(InteractionRejection::TabListMenuSessionUnavailable { session })?;
        let plan = projection.plan();
        let revision = plan.popup().revision();
        self.engine
            .validate_tab_list_menu_popup(plan, session, revision)?;
        let record = plan
            .tab_list_menu_records()
            .iter()
            .find(|record| record.session() == session)
            .cloned()
            .ok_or(InteractionRejection::TabListMenuSessionUnavailable { session })?;
        let backdrop = plan
            .tab_list_menu_backdrop_records()
            .iter()
            .copied()
            .find(|record| record.session() == session && record.revision() == revision)
            .ok_or(InteractionRejection::TabListMenuBackdropUnavailable { session })?;
        if !self
            .engine
            .presentation_authority
            .tab_strip_states
            .active_menu_for(session.key())
            .is_some_and(|active| active.session() == session)
        {
            return Err(InteractionRejection::TabListMenuSessionUnavailable { session });
        }
        Ok(PreparedTabListMenuDismiss {
            presentation: DockEngine::freeze_interaction_projection(projection),
            session,
            revision,
            record,
            backdrop,
        })
    }

    /// Prepares one exact tab-strip wheel, edge-drag, or reveal adjustment.
    ///
    /// The bar must belong to `surface`, remain interaction-enabled, and exist
    /// in this sealed frame's receiver-authoritative projection.
    #[cfg(test)]
    pub fn prepare_tab_strip_scroll(
        &self,
        surface: SurfaceId,
        bar: crate::presentation::scene::TabBarSceneId,
        adjustment: TabScrollAdjustment,
    ) -> Result<PreparedTabStripScroll, InteractionRejection> {
        let key = TabStripStateKey::new(surface, bar);
        let projection = self
            .prepared_action_projection(surface)
            .ok_or(InteractionRejection::TabStripSourceUnavailable { key })?;
        let record = projection
            .plan()
            .tab_bar_records()
            .iter()
            .find(|record| *record.id() == bar)
            .cloned()
            .ok_or(InteractionRejection::TabStripSourceUnavailable { key })?;
        if record.interaction() != TabBarInteraction::Enabled {
            return Err(InteractionRejection::TabStripScrollDisabled { key });
        }
        if self
            .engine
            .presentation_authority
            .tab_strip_states
            .state(key)
            .is_none()
        {
            return Err(InteractionRejection::TabStripSourceUnavailable { key });
        }
        let requested_items: &[TabKey] = match &adjustment.0 {
            TabScrollAdjustmentKind::ScrollByPreserving { keep_visible, .. } => keep_visible,
            TabScrollAdjustmentKind::ScrollTo { .. } => &[],
            TabScrollAdjustmentKind::RevealTab(item) => std::slice::from_ref(item),
        };
        if let Some(item) = requested_items.iter().copied().find(|item| {
            !record
                .members()
                .iter()
                .any(|member| member.tab().tab == *item)
        }) {
            return Err(InteractionRejection::TabStripScrollItemUnavailable { key, item });
        }
        Ok(PreparedTabStripScroll {
            presentation: DockEngine::freeze_interaction_projection(projection),
            key,
            record,
            adjustment,
        })
    }

    /// Prepares one exact active tab-list menu wheel or reveal adjustment.
    ///
    /// The session, popup routing revision, complete menu geometry, and target
    /// item membership are all frozen from this sealed frame.
    #[cfg(test)]
    pub fn prepare_tab_list_menu_scroll(
        &self,
        surface: SurfaceId,
        session: crate::presentation::tab_strip::TabListMenuSessionId,
        adjustment: TabScrollAdjustment,
    ) -> Result<PreparedTabListMenuScroll, InteractionRejection> {
        let projection = self
            .prepared_action_projection(surface)
            .ok_or(InteractionRejection::TabListMenuSessionUnavailable { session })?;
        self.engine
            .prepare_presented_tab_list_menu_scroll(projection, session, adjustment)
    }

    /// Prepares one exact keyboard or accessibility focus move in the active menu.
    ///
    /// Relative moves are resolved against the sealed ordered roster and clamp
    /// at its boundaries. The prepared proof binds the resolved target to the
    /// exact popup revision and complete menu geometry used for reveal.
    #[cfg(test)]
    pub fn prepare_tab_list_menu_navigation(
        &self,
        surface: SurfaceId,
        session: crate::presentation::tab_strip::TabListMenuSessionId,
        navigation: TabListMenuNavigation,
    ) -> Result<PreparedTabListMenuNavigation, InteractionRejection> {
        let projection = self
            .prepared_action_projection(surface)
            .ok_or(InteractionRejection::TabListMenuSessionUnavailable { session })?;
        self.engine
            .prepare_presented_tab_list_menu_navigation(projection, session, navigation)
    }

    /// Returns this frame's read-only viewport coordinator.
    #[must_use]
    pub const fn viewport(&self) -> &'frame ViewportCoordinator {
        self.engine.viewport()
    }

    /// Returns this frame's adapter-neutral viewport focus state.
    #[must_use]
    pub const fn viewport_focus(&self) -> &'frame ViewportFocusCoordinator {
        self.engine.viewport_focus()
    }

    pub(crate) fn close_plan(&self, request: CloseRequestId) -> Option<&'frame ClosePlan> {
        self.engine.close_plan(request)
    }

    /// Returns the pane-focus request admitted at this committed boundary.
    ///
    /// The request becomes observable to an adapter only after the boundary
    /// that publishes its obligation.
    #[must_use]
    pub fn published_pane_focus_intent(&self) -> Option<PaneFocusIntent> {
        self.engine.viewport_focus.published_pane_intent()
    }

    /// Freezes one contribution token against this frame's exact authority.
    pub fn begin_surface_contribution(
        &self,
        surface: SurfaceId,
    ) -> Result<SurfaceContributionToken, SurfaceContributionBeginError> {
        if !self.presentation_roster.contains_surface(surface) {
            return Err(SurfaceContributionBeginError::SurfaceOutsideRoster { surface });
        }
        self.engine.begin_surface_contribution(surface)
    }

    /// Compiles one exact measurement answer against this frame's authority.
    pub fn prepare_surface_contribution(
        &self,
        token: SurfaceContributionToken,
        measurements: SurfaceMeasurements,
    ) -> Result<PreparedSurfaceContribution, SurfaceContributionPrepareError> {
        if !self.presentation_roster.contains_surface(token.surface()) {
            return Err(SurfaceContributionPrepareError::SurfaceOutsideRoster {
                surface: token.surface(),
            });
        }
        self.engine
            .prepare_surface_contribution(token, measurements)
    }

    /// Prepares one explicit unavailable answer against this frame's authority.
    pub fn prepare_surface_unavailable_contribution(
        &self,
        token: SurfaceContributionToken,
        reason: MeasurementUnavailableReason,
    ) -> Result<PreparedSurfaceContribution, SurfaceContributionPrepareError> {
        if !self.presentation_roster.contains_surface(token.surface()) {
            return Err(SurfaceContributionPrepareError::SurfaceOutsideRoster {
                surface: token.surface(),
            });
        }
        self.engine
            .prepare_surface_unavailable_contribution(token, reason)
    }

    /// Retains one exact Ready candidate visible to this frame.
    pub fn prepare_surface_retained_contribution(
        &self,
        token: SurfaceContributionToken,
    ) -> Result<PreparedSurfaceContribution, SurfaceContributionPrepareError> {
        if !self.presentation_roster.contains_surface(token.surface()) {
            return Err(SurfaceContributionPrepareError::SurfaceOutsideRoster {
                surface: token.surface(),
            });
        }
        self.engine.prepare_surface_retained_contribution(token)
    }
}

impl CoreHostFrame {
    pub(crate) fn candidate_engine(&self) -> &DockEngine {
        &self.candidate
    }

    /// Returns the sole post-observation authority from which adapters may
    /// prepare measurements, output, and receiver facts for this frame.
    #[must_use]
    pub const fn view(&self) -> HostFrameView<'_> {
        HostFrameView {
            engine: &self.candidate,
            presentation_roster: &self.frozen_presentation_roster,
            receiver_presentations: &self.frozen_pointer_presentations,
            #[cfg(test)]
            prepared_presentations: &self.frozen_prepared_presentations,
        }
    }

    /// Returns the exact post-observation surface roster frozen at seal.
    #[must_use]
    pub fn surfaces(&self) -> impl ExactSizeIterator<Item = SurfaceId> + '_ {
        self.frozen_presentation_roster.surfaces()
    }

    /// Returns whether the private input prefix changed any presentation fact
    /// after receiver authority was sealed for this frame.
    #[must_use]
    #[cfg(test)]
    pub const fn presentation_changed_since_seal(&self) -> bool {
        self.presentation_snapshot_changed
    }

    /// Returns whether input changed scene, coordinate, or surface projection
    /// authority rather than only transient interaction visuals.
    #[must_use]
    #[cfg(test)]
    pub const fn presentation_projection_changed_since_seal(&self) -> bool {
        self.presentation_projection_changed
    }

    /// Returns the exact joined backend provider frozen for this frame.
    #[must_use]
    pub const fn backend_ingress_provider(&self) -> Option<BackendIngressLease> {
        self.backend_ingress
    }

    pub(crate) const fn input_terminated_for_overload(&self) -> bool {
        self.input_terminated_for_overload
    }

    pub(crate) const fn backend_ingress_through(&self) -> BackendIngressOrdinal {
        self.candidate.backend_ingress.committed_through()
    }

    /// Reserves only inline ingress cursor and commit-guard transport data.
    pub(crate) const fn backend_ingress_inline_capacity_bytes() -> usize {
        size_of::<HostBackendIngressCursor>() + size_of::<BackendIngressCommitGuard>()
    }

    /// Counts this frame's ingress buffers, whose payloads remain in its recorder.
    pub(crate) fn backend_ingress_buffer_capacity_bytes(&self) -> usize {
        let cursor = self.pending_backend_ingress.as_ref().map_or(0, |cursor| {
            size_of::<HostBackendIngressCursor>()
                .saturating_add(cursor.record_buffer_capacity_bytes())
        });
        let guard = self
            .backend_ingress_commit_guard
            .as_ref()
            .map_or(0, |guard| {
                size_of::<BackendIngressCommitGuard>().saturating_add(guard.owned_capacity_bytes())
            });
        cursor.saturating_add(guard)
    }

    /// Returns the typed reduction failure retained after
    /// [`CoreHostFrameError::InputPrefixReductionFailed`].
    ///
    /// The value is diagnostic-only: the host frame remains poisoned and must
    /// be dropped or finished through its normal rollback path.
    #[must_use]
    #[cfg(test)]
    pub fn input_prefix_error(&self) -> Option<&EngineError> {
        match &self.poison {
            Some(CoreHostFrameError::InputPrefixReductionFailed { source }) => Some(source),
            _ => None,
        }
    }

    pub(crate) fn proposed_workspace_root_rebounds(
        &self,
    ) -> impl Iterator<
        Item = (
            crate::platform::viewport::ViewportBinding,
            crate::platform::viewport::ViewportBinding,
        ),
    > + '_ {
        self.reduced_inputs
            .iter()
            .filter_map(|reduced| match reduced.outcome() {
                crate::engine::transition::InputOutcome::WorkspaceReplaced {
                    reconciliation,
                    ..
                } => Some(reconciliation.rebound()),
                _ => None,
            })
            .flat_map(|pairs| pairs.iter().copied())
    }

    pub(crate) fn has_native_placement_submission(&self) -> bool {
        self.reduced_inputs.iter().any(|reduced| {
            matches!(
                reduced.outcome(),
                crate::engine::transition::InputOutcome::NativeSurfacePlacementsChanged {
                    changed: true,
                    ..
                }
            )
        })
    }
}
