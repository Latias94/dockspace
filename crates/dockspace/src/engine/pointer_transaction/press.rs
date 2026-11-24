//! Press admission, source gestures and pane activation.

use super::*;

impl DockEngine {
    pub(super) fn reduce_journal_primary_press(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        owner: GestureOwner,
        capture_authority: Authority<PointerCaptureOwner>,
        edge: &PointerEdge,
        receipt: &ValidatedPointerReceiverReceipt,
        snapshot: &JournalPresentationSnapshot,
        desktop_delivery_route: Option<DesktopRouteValidation>,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
    ) -> Result<Option<InteractionOutcome>, EngineError> {
        if self.interaction.status() != InteractionStatus::Idle {
            return Ok(Some(InteractionOutcome::Rejected(
                InteractionRejection::GestureBusy {
                    status: self.interaction.status(),
                },
            )));
        }
        let Some(point) = self.journal_logical_point(edge, desktop_delivery_route) else {
            return Ok(None);
        };
        let stream = owner.stream();
        let expected_delivery =
            match stream.lease().scope() {
                PointerProviderScope::SurfaceLocal(_) | PointerProviderScope::HostLocal(_) => {
                    PointerEventDeliveryOwner::ProviderEndpoint
                }
                PointerProviderScope::DesktopGlobal => {
                    let binding = desktop_delivery_route
                        .and_then(DesktopRouteValidation::dock_route)
                        .map(|route| route.binding())
                        .ok_or(EngineError::ReductionCauseInvariant {
                            detail: "desktop primary press has no exact hovered source binding",
                        })?;
                    PointerEventDeliveryOwner::Native(binding.window())
                }
                PointerProviderScope::NativeJoined(_) => {
                    let binding = match owner.lane() {
                    GestureLane::NativeLocal { binding, .. } => binding,
                    GestureLane::Desktop => desktop_delivery_route
                        .and_then(DesktopRouteValidation::dock_route)
                        .map(|route| route.binding().window())
                        .ok_or(EngineError::ReductionCauseInvariant {
                            detail: "joined desktop primary press has no native coordinate route",
                        })?,
                    _ => return Err(EngineError::ReductionCauseInvariant {
                        detail: "joined primary press has a non-native lane",
                    }),
                };
                    PointerEventDeliveryOwner::Native(binding)
                }
            };
        if let Some(rejection) =
            self.journal_press_delivery_rejection(edge.delivery_owner(), expected_delivery)?
        {
            return Ok(Some(InteractionOutcome::Rejected(rejection)));
        }
        if let Some(outcome) = self.dismiss_popup_from_other_surface_press(
            cause,
            owner,
            capture_authority,
            receipt,
            snapshot,
        )? {
            return Ok(Some(outcome));
        }
        let Some((region, presentation)) =
            Self::journal_semantic_press_delivery(cause, receipt, snapshot)?
        else {
            return Ok(None);
        };
        if let Some(rejection) =
            self.journal_press_capture_rejection(owner, capture_authority, presentation)?
        {
            return Ok(Some(InteractionOutcome::Rejected(rejection)));
        }
        if matches!(
            region.kind(),
            PresentationHitRegionKind::TabClose(_)
                | PresentationHitRegionKind::ContainedClose(_)
                | PresentationHitRegionKind::TabStripControl(_)
                | PresentationHitRegionKind::TabListMenuRow { .. }
                | PresentationHitRegionKind::TabListMenuBlocker(_)
                | PresentationHitRegionKind::TabListMenuBackdrop(_)
        ) {
            let outcome = self.arm_journal_click(
                cause,
                owner,
                capture_authority,
                presentation,
                region,
                point,
                policy,
            )?;
            if edge.metadata().device() == crate::input::journal::PointerDeviceKind::Touch
                && Self::pointer_timing_supported(owner)
                && let InteractionStatus::Pressed { session } = self.interaction.status()
            {
                let threshold_origin =
                    self.journal_drag_threshold_origin(edge, desktop_delivery_route)?;
                self.interaction
                    .active_click_mut(session)
                    .map_err(|error| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!("{error:?}"),
                    })?
                    .touch = Some(TouchPressState {
                    initial_pointer: point,
                    current_pointer: point,
                    threshold_origin,
                    request: None,
                    armed: false,
                });
            }
            return Ok(outcome);
        }
        let threshold_origin = self.journal_drag_threshold_origin(edge, desktop_delivery_route)?;
        if edge.metadata().device() == crate::input::journal::PointerDeviceKind::Touch
            && Self::pointer_timing_supported(owner)
            && Self::touch_wait_region(region.kind())
        {
            return self.arm_journal_touch_press(
                cause,
                owner,
                capture_authority,
                presentation,
                region,
                point,
                threshold_origin,
                edge.metadata(),
            );
        }
        self.activate_journal_source_press(
            cause,
            focus_causal,
            SourcePressIntent::Activate,
            owner,
            capture_authority,
            threshold_origin,
            presentation,
            region,
            point,
            self.journal_pointer_hit_device(edge),
            policy,
            events,
        )
    }

    pub(in crate::engine) fn activate_journal_source_press(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        intent: SourcePressIntent,
        owner: GestureOwner,
        capture_authority: Authority<PointerCaptureOwner>,
        threshold_origin: JournalDragThresholdOrigin,
        presentation: &JournalSurfacePresentation,
        region: PresentationHitRegionId,
        point: LogicalPoint,
        device: crate::input::journal::PointerDeviceKind,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
    ) -> Result<Option<InteractionOutcome>, EngineError> {
        if !self.presented_receiver_is_current(presentation.plan(), region) {
            return Ok(Some(InteractionOutcome::Rejected(
                InteractionRejection::StaleScene,
            )));
        }
        match region.kind() {
            PresentationHitRegionKind::TabBody(tab) => {
                let source = TabGestureSource::Tab(tab);
                let prepared = match self.prepare_journal_tab_gesture(presentation, source, point) {
                    Ok(prepared) => prepared,
                    Err(rejection) => return Ok(Some(InteractionOutcome::Rejected(rejection))),
                };
                self.activate_prepared_journal_tab_gesture(
                    cause,
                    focus_causal,
                    intent,
                    owner,
                    capture_authority,
                    threshold_origin,
                    prepared,
                    policy,
                    events,
                )
                .map(Some)
            }
            PresentationHitRegionKind::TabGroupDrag { bar: group, .. } => {
                let source = TabGestureSource::Group(group);
                let prepared = match self.prepare_journal_tab_gesture(presentation, source, point) {
                    Ok(prepared) => prepared,
                    Err(rejection) => return Ok(Some(InteractionOutcome::Rejected(rejection))),
                };
                self.activate_prepared_journal_tab_gesture(
                    cause,
                    focus_causal,
                    intent,
                    owner,
                    capture_authority,
                    threshold_origin,
                    prepared,
                    policy,
                    events,
                )
                .map(Some)
            }
            PresentationHitRegionKind::SplitterHandle(_)
            | PresentationHitRegionKind::SplitterJunction(_) => {
                if let Some(rejection) = self.pending_presentation_transition_gesture_rejection() {
                    return Ok(Some(InteractionOutcome::Rejected(rejection)));
                }
                let start = match self.prepare_journal_splitter_gesture(
                    presentation,
                    region,
                    owner,
                    capture_authority,
                    point,
                    device,
                    policy,
                ) {
                    Ok(start) => start,
                    Err(rejection) => return Ok(Some(InteractionOutcome::Rejected(rejection))),
                };
                let (session, replaced) = self
                    .interaction
                    .begin_resize(self.version.epoch(), start)
                    .map_err(|source| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: source.to_string(),
                    })?;
                if replaced.is_some() {
                    return Err(EngineError::ReductionCauseInvariant {
                        detail: "journal splitter press replaced an active gesture after idle validation",
                    });
                }
                Ok(Some(InteractionOutcome::ResizeBegan { session, replaced }))
            }
            PresentationHitRegionKind::ContainedTitle(floating) => {
                let prepared = match self.prepare_journal_contained_gesture(
                    presentation,
                    floating,
                    ContainedGestureKind::TitleDrag,
                    point,
                    device,
                ) {
                    Ok(prepared) => prepared,
                    Err(rejection) => return Ok(Some(InteractionOutcome::Rejected(rejection))),
                };
                self.activate_prepared_journal_contained_gesture(
                    cause,
                    focus_causal,
                    intent,
                    owner,
                    capture_authority,
                    threshold_origin,
                    prepared,
                    policy,
                    events,
                )
                .map(Some)
            }
            PresentationHitRegionKind::ContainedResize {
                floating,
                direction,
            } => {
                let prepared = match self.prepare_journal_contained_gesture(
                    presentation,
                    floating,
                    ContainedGestureKind::Resize(direction),
                    point,
                    device,
                ) {
                    Ok(prepared) => prepared,
                    Err(rejection) => return Ok(Some(InteractionOutcome::Rejected(rejection))),
                };
                self.activate_prepared_journal_contained_gesture(
                    cause,
                    focus_causal,
                    intent,
                    owner,
                    capture_authority,
                    threshold_origin,
                    prepared,
                    policy,
                    events,
                )
                .map(Some)
            }
            PresentationHitRegionKind::PaneBody(_)
            | PresentationHitRegionKind::ContainedFrameBlocker(_) => self
                .activate_journal_contained_content(
                    cause,
                    focus_causal,
                    presentation,
                    region,
                    point,
                    policy,
                    events,
                ),
            _ => Ok(None),
        }
    }

    fn activate_journal_contained_content(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        presentation: &JournalSurfacePresentation,
        region: PresentationHitRegionId,
        point: LogicalPoint,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
    ) -> Result<Option<InteractionOutcome>, EngineError> {
        let plan = presentation.plan();
        let (contained, focus_item) = match region.kind() {
            PresentationHitRegionKind::PaneBody(pane) => (
                plan.contained_records()
                    .iter()
                    .find(|record| record.root() == pane.root),
                plan.pane_records()
                    .iter()
                    .find(|record| record.id() == pane)
                    .map(crate::presentation::scene::PaneRecord::item),
            ),
            PresentationHitRegionKind::ContainedFrameBlocker(floating) => {
                (plan.contained_record(floating), None)
            }
            _ => return Ok(None),
        };
        let Some(contained) = contained else {
            return Ok(None);
        };
        if contained.is_frontmost() {
            return Ok(None);
        }
        if !contained.outer_bounds().contains(point)
            || !plan.point_is_on_authoritative_layer(point, contained.layer())
        {
            return Ok(Some(InteractionOutcome::Rejected(
                InteractionRejection::ActionReceiverUnavailable {
                    target: region.kind(),
                },
            )));
        }
        let surface = presentation.surface();
        let floating = contained.floating();
        let root = contained.root();
        if self.root_has_pending_presentation_transition(root) {
            return Ok(Some(InteractionOutcome::Rejected(
                InteractionRejection::PresentationTransitionPending,
            )));
        }
        if self.workspace.presentation_for_root(root)
            != Some(crate::graph::workspace::RootPresentationOwner::Contained { surface, floating })
        {
            return Ok(Some(InteractionOutcome::Rejected(
                InteractionRejection::StaleScene,
            )));
        }
        let command = self
            .workspace
            .root(root)
            .ok_or(CommandError::MissingRoot { root })
            .and_then(|record| self.workspace.capture_node_source(root, record.node))
            .and_then(|source| {
                self.workspace
                    .capture_contained_roster(surface)
                    .map(|expected_roster| WorkspaceCommand::RaiseContained {
                        source,
                        floating,
                        expected_roster,
                    })
            });
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                return Ok(Some(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(error),
                )));
            }
        };
        let (_, changed) =
            match self.apply_journal_workspace_command(cause, &command, policy, events)? {
                Ok(result) => result,
                Err(error) => {
                    return Ok(Some(InteractionOutcome::Rejected(
                        InteractionRejection::CommandRejected(error),
                    )));
                }
            };
        if let Some(item) = focus_item {
            let native_guard = self.viewport_focus_binding(surface);
            self.viewport_focus
                .request_local_pane_focus(surface, item, native_guard, focus_causal)
                .map_err(|source| EngineError::CausedViewportFocus { cause, source })?;
        }
        Ok(Some(InteractionOutcome::ProductActionApplied(
            crate::model::DockspaceActionOutcome::ContainedRaised {
                root,
                surface,
                floating,
                changed,
            },
        )))
    }

    pub(in crate::engine) fn prepare_journal_tab_gesture(
        &self,
        presentation: &JournalSurfacePresentation,
        source: TabGestureSource,
        point: crate::geometry::LogicalPoint,
    ) -> Result<PreparedTabGesture, InteractionRejection> {
        let surface = presentation.surface();
        let plan = presentation.plan();
        self.prepare_tab_gesture(
            surface,
            presentation.scene(),
            plan,
            source,
            point,
            Self::freeze_journal_presentation(presentation),
        )
    }

    fn prepare_tab_gesture(
        &self,
        surface: SurfaceId,
        scene: SurfaceSceneStamp,
        plan: &crate::presentation::scene::PresentationPlan,
        source: TabGestureSource,
        point: crate::geometry::LogicalPoint,
        presentation: FrozenPresentationAuthority,
    ) -> Result<PreparedTabGesture, InteractionRejection> {
        if scene.requirement().workspace_epoch() != self.version.epoch() {
            return Err(InteractionRejection::StaleScene);
        }
        if !plan.bounds().contains(point) {
            return Err(InteractionRejection::TabGesturePointerOutsideSurface { surface });
        }
        let (root, tabs, layer) = match source {
            TabGestureSource::Tab(source) => {
                let tab = plan
                    .tab_records()
                    .iter()
                    .find(|record| *record.id() == source)
                    .ok_or(InteractionRejection::TabGestureSourceUnavailable {
                        source: TabGestureSource::Tab(source),
                    })?;
                if !tab.drag_hit().contains(point)
                    || tab
                        .close_bounds()
                        .is_some_and(|close| close.contains(point))
                {
                    return Err(InteractionRejection::TabGestureHitMismatch {
                        source: TabGestureSource::Tab(source),
                    });
                }
                (source.root, source.tabs, tab.layer())
            }
            TabGestureSource::Group(source) => {
                let bar = plan
                    .tab_bar_records()
                    .iter()
                    .find(|record| *record.id() == source)
                    .ok_or(InteractionRejection::TabGestureSourceUnavailable {
                        source: TabGestureSource::Group(source),
                    })?;
                let group =
                    bar.group_drag()
                        .ok_or(InteractionRejection::TabGestureSourceUnavailable {
                            source: TabGestureSource::Group(source),
                        })?;
                if !group.contains(point) {
                    return Err(InteractionRejection::TabGestureHitMismatch {
                        source: TabGestureSource::Group(source),
                    });
                }
                (source.root, source.tabs, bar.layer())
            }
            TabGestureSource::ContainedTitle { root, floating } => {
                let contained = plan
                    .contained_record(floating)
                    .filter(|record| record.root() == root)
                    .ok_or(InteractionRejection::TabGestureSourceUnavailable { source })?;
                if !contained.title_drag_hit().contains(point) {
                    return Err(InteractionRejection::TabGestureHitMismatch { source });
                }
                let node = self
                    .workspace
                    .root(root)
                    .map(|record| record.node)
                    .ok_or(InteractionRejection::TabGestureSourceUnavailable { source })?;
                (root, node, contained.layer())
            }
        };
        if let Some(occluding) = plan
            .drop_occlusions()
            .iter()
            .filter(|occlusion| occlusion.layer() > layer && occlusion.region().contains(point))
            .max_by_key(|occlusion| occlusion.layer())
        {
            return Err(InteractionRejection::TabGestureOccluded {
                source,
                occluding: occluding.floating(),
            });
        }
        let source_node = self
            .workspace
            .capture_node_source(root, tabs)
            .map_err(InteractionRejection::CommandRejected)?;
        let contained = match self.workspace.presentation_for_root(root) {
            Some(crate::graph::workspace::RootPresentationOwner::Main {
                surface: owner_surface,
            }) if owner_surface == surface => None,
            Some(crate::graph::workspace::RootPresentationOwner::Contained {
                surface: owner_surface,
                floating,
            }) if owner_surface == surface => {
                let presented = plan
                    .contained_record(floating)
                    .filter(|record| record.root() == root && record.layer() == layer);
                let durable = self
                    .workspace
                    .contained_floating(floating)
                    .filter(|record| record.root == root);
                let (Some(presented), Some(durable)) = (presented, durable) else {
                    return Err(InteractionRejection::TabGestureSourceUnavailable { source });
                };
                if plan.unclipped_contained_bounds(floating) != Some(durable.rect) {
                    return Err(InteractionRejection::TabGestureSourceUnavailable { source });
                }
                let root_node = self
                    .workspace
                    .root(root)
                    .map(|record| record.node)
                    .ok_or(InteractionRejection::TabGestureSourceUnavailable { source })?;
                Some(PreparedContainedTabOrigin {
                    surface,
                    floating,
                    root_source: self
                        .workspace
                        .capture_node_source(root, root_node)
                        .map_err(InteractionRejection::CommandRejected)?,
                    source_rect: durable.rect,
                    minimum_size: presented.minimum_size(),
                    expected_roster: self
                        .workspace
                        .capture_contained_roster(surface)
                        .map_err(InteractionRejection::CommandRejected)?,
                })
            }
            _ => return Err(InteractionRejection::TabGestureSourceUnavailable { source }),
        };
        let unavailable = || InteractionRejection::TabGestureSourceUnavailable { source };
        let source_geometry = match source {
            TabGestureSource::ContainedTitle { .. } => {
                let contained = contained.as_ref().ok_or_else(unavailable)?;
                JournalDragSourceGeometry {
                    source_rect: contained.source_rect,
                    minimum_size: contained.minimum_size,
                }
            }
            TabGestureSource::Tab(_) | TabGestureSource::Group(_) => {
                let facts = plan.layout_facts().ok_or_else(unavailable)?;
                let root_facts = facts.root(root).ok_or_else(unavailable)?;
                let source_node = match source {
                    TabGestureSource::Tab(tab) => tab.tab.layout().unwrap_or(tabs),
                    _ => tabs,
                };
                let projected = root_facts
                    .projection()
                    .nodes
                    .get(&source_node)
                    .ok_or_else(unavailable)?;
                let pane_sizing = match source {
                    TabGestureSource::Tab(tab) if tab.tab.item().is_some() => {
                        let measured = root_facts
                            .pane_sizing(PaneSizingKey::new(
                                root,
                                tab.tab.item().ok_or_else(unavailable)?,
                            ))
                            .ok_or_else(unavailable)?;
                        let visibility =
                            self.presentation_requirements()
                                .surface(surface)
                                .and_then(|requirements| {
                                    requirements.tab_bar(
                                        crate::presentation::scene::TabBarSceneId { root, tabs },
                                    )
                                })
                                .map(|bar| bar.policy().resolved(1, false).visibility())
                                .ok_or_else(unavailable)?;
                        pane_outer_sizing(measured, facts.config(), visibility)
                            .map_err(|_| unavailable())?
                            .minimum_size()
                    }
                    TabGestureSource::Tab(_) => projected.constraints.min(),
                    TabGestureSource::Group(_) => projected.constraints.min(),
                    TabGestureSource::ContainedTitle { .. } => return Err(unavailable()),
                };
                JournalDragSourceGeometry {
                    source_rect: projected.rect,
                    minimum_size: contained_minimum_size(facts.config(), pane_sizing)
                        .map_err(|_| unavailable())?,
                }
            }
        };
        Ok(PreparedTabGesture {
            source,
            surface,
            root,
            tabs,
            source_node,
            source_geometry,
            contained,
            initial_pointer: point,
            presentation,
            source_layout_facts: plan.layout_facts().cloned().map(std::sync::Arc::new),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine) fn activate_prepared_journal_tab_gesture(
        &mut self,
        cause: ReductionCause,
        focus_causal: FocusCausalStamp,
        intent: SourcePressIntent,
        owner: GestureOwner,
        capture_authority: Authority<PointerCaptureOwner>,
        threshold_origin: JournalDragThresholdOrigin,
        prepared: PreparedTabGesture,
        policy: &DockPolicySnapshot,
        events: &mut Vec<WorkspaceEvent>,
    ) -> Result<InteractionOutcome, EngineError> {
        if let Some(rejection) = self.pending_presentation_transition_gesture_rejection() {
            return Ok(InteractionOutcome::Rejected(rejection));
        }
        let source_payload = match prepared.source {
            TabGestureSource::Tab(source) => self
                .workspace
                .verify_reference(
                    source.root,
                    source.tabs,
                    &prepared.source_node.reference,
                    ReferenceRole::Source,
                )
                .and_then(|()| self.tab_drag_payload(source)),
            TabGestureSource::Group(_) => {
                if matches!(self.workspace.node(prepared.tabs), Some(Node::Tabs { .. })) {
                    Ok(MovePayload::Tabs(prepared.source_node.clone()))
                } else {
                    Err(CommandError::NodeIsNotTabs {
                        node: prepared.tabs,
                    })
                }
            }
            TabGestureSource::ContainedTitle { .. } => {
                Ok(MovePayload::Subtree(prepared.source_node.clone()))
            }
        };
        let source_payload = match source_payload {
            Ok(payload) => payload,
            Err(source) => {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(source),
                ));
            }
        };
        let can_arm = match authorize_captured_drag_source(&self.workspace, policy, &source_payload)
        {
            Ok(()) => true,
            // Selecting a tab and focusing its content do not require docking permission.
            Err(CommandError::Policy(_))
                if intent == SourcePressIntent::Activate
                    && matches!(prepared.source, TabGestureSource::Tab(_)) =>
            {
                false
            }
            Err(source) => {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(source),
                ));
            }
        };
        let mut candidate_events = Vec::new();
        let mut commands = Vec::with_capacity(2);
        let mut selection_changed = false;
        if let Some(contained) = &prepared.contained
            && contained.expected_roster.contained().last() != Some(&contained.floating)
        {
            let raise = WorkspaceCommand::RaiseContained {
                source: contained.root_source.clone(),
                floating: contained.floating,
                expected_roster: contained.expected_roster.clone(),
            };
            if intent == SourcePressIntent::Activate {
                commands.push(raise);
            } else if let Err(source) = crate::graph::operation::authorize_workspace_command(
                &self.workspace,
                policy,
                &raise,
            ) {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(source),
                ));
            }
        }
        if intent == SourcePressIntent::Activate
            && let TabGestureSource::Tab(source) = prepared.source
        {
            let already_selected = matches!(
                self.workspace.node(source.tabs),
                Some(Node::Tabs {
                    selected: Some(selected),
                    ..
                }) if *selected == source.tab
            );
            if !already_selected {
                selection_changed = true;
                let selection =
                    match self
                        .workspace
                        .capture_tab_source(source.root, source.tabs, source.tab)
                    {
                        Ok(source) => source,
                        Err(error) => {
                            return Ok(InteractionOutcome::Rejected(
                                InteractionRejection::CommandRejected(error),
                            ));
                        }
                    };
                commands.push(WorkspaceCommand::Select { source: selection });
            }
        }
        if !commands.is_empty() {
            let prepared = match self.workspace.prepare_transaction(policy, &commands) {
                Ok(prepared) => prepared,
                Err(TransactionError::Command { source, .. }) if source.is_expected_rejection() => {
                    return Ok(InteractionOutcome::Rejected(
                        InteractionRejection::CommandRejected(source),
                    ));
                }
                Err(source) => {
                    return Err(EngineError::PointerInteractionInvariant {
                        cause,
                        detail: source.to_string(),
                    });
                }
            };
            let (workspace, report) = prepared.into_snapshot_parts();
            if let Some(surface) = self.first_pending_presentation_source_mismatch(
                &workspace,
                WorkspacePublicationAuthority::Ordinary,
            ) {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(
                        crate::graph::error::CommandError::SurfaceLifecycleFrozen { surface },
                    ),
                ));
            }
            if let Some(surface) = self.first_workspace_publication_mismatch(&workspace, None, None)
            {
                return Ok(InteractionOutcome::Rejected(
                    InteractionRejection::CommandRejected(
                        crate::graph::error::CommandError::SurfaceLifecycleFrozen { surface },
                    ),
                ));
            }
            let changed = report.changed();
            let outcomes = report.into_outcomes();
            let publication = match self.stage_workspace_publication(workspace, policy) {
                Ok(publication) => publication,
                Err(source) if source.is_expected_rejection() => {
                    return Ok(InteractionOutcome::Rejected(
                        InteractionRejection::CommandRejected(source),
                    ));
                }
                Err(source) => {
                    return Err(EngineError::PointerInteractionInvariant {
                        cause,
                        detail: source.to_string(),
                    });
                }
            };
            self.publish_workspace(publication);
            self.reconcile_viewport_focus_authority();
            if changed {
                if let [WorkspaceCommand::Select { source }] = commands.as_slice() {
                    self.advance_selection_revision_caused(cause, source)?;
                } else {
                    self.advance_revision_caused(cause)?;
                }
                candidate_events.extend(outcomes.into_iter().map(|outcome| {
                    WorkspaceEvent::new_caused(
                        cause,
                        self.version,
                        WorkspaceEventKind::CommandCommitted(outcome),
                    )
                }));
            }
        }
        if intent == SourcePressIntent::Activate {
            if let Some(item) = self.pointer_payload_focus_item(&source_payload) {
                self.request_pointer_pane_focus(
                    prepared.surface,
                    item,
                    focus_causal,
                    &mut candidate_events,
                )?;
            }
            if !can_arm && let TabGestureSource::Tab(source) = prepared.source {
                events.extend(candidate_events);
                return Ok(InteractionOutcome::ProductActionApplied(
                    crate::model::DockspaceActionOutcome::Selected {
                        tab: source.tab,
                        changed: selection_changed,
                    },
                ));
            }
        }
        let payload = match prepared.source {
            TabGestureSource::Tab(source) => self.tab_drag_payload(source),
            TabGestureSource::Group(_) => self
                .workspace
                .capture_node_source(prepared.root, prepared.tabs)
                .map(MovePayload::Tabs),
            TabGestureSource::ContainedTitle { .. } => self
                .workspace
                .capture_node_source(prepared.root, prepared.tabs)
                .map(MovePayload::Subtree),
        };
        let payload = payload.map_err(|source| EngineError::PointerInteractionInvariant {
            cause,
            detail: format!("staged tab gesture lost its payload source: {source}"),
        })?;
        let drag_source = self.prepare_drag_source(&payload).map_err(|source| {
            EngineError::PointerInteractionInvariant {
                cause,
                detail: format!("staged tab gesture lost drag authority: {source:?}"),
            }
        })?;
        let origin = if drag_source.complete_root.is_some() {
            if let Some(contained) = &prepared.contained {
                let source_roster = self
                    .workspace
                    .capture_contained_roster(contained.surface)
                    .map_err(|source| EngineError::PointerInteractionInvariant {
                        cause,
                        detail: format!(
                            "staged contained tab gesture lost its source roster: {source}"
                        ),
                    })?;
                FrozenDragOrigin::Contained(FrozenContainedDragOrigin {
                    surface: contained.surface,
                    root: prepared.root,
                    floating: contained.floating,
                    intent: match prepared.source {
                        TabGestureSource::ContainedTitle { .. } => ContainedDragIntent::MoveWindow,
                        TabGestureSource::Tab(_) | TabGestureSource::Group(_) => {
                            ContainedDragIntent::DockContent
                        }
                    },
                    source_rect: contained.source_rect,
                    source_roster,
                    initial_pointer: prepared.initial_pointer,
                    minimum_size: contained.minimum_size,
                })
            } else {
                FrozenDragOrigin::Workspace
            }
        } else {
            FrozenDragOrigin::Workspace
        };
        let continuation = self.scene_gesture_continuation_draft(
            cause,
            owner,
            prepared.presentation,
            SceneGestureContinuationSource::Drag {
                payload: payload.clone(),
                source_surface: drag_source.source_surface,
                complete_root: drag_source.complete_root.clone(),
                origin: origin.clone(),
                coordinates: self.capture_surface_coordinates(drag_source.source_surface),
            },
        );
        let (session, replaced) = self
            .interaction
            .arm_drag(DragArmStart {
                epoch: self.version.epoch(),
                owner,
                journal_capture_authority: capture_authority,
                button: PointerButton::Primary,
                payload,
                source_surface: drag_source.source_surface,
                initial_pointer: Some(prepared.initial_pointer),
                journal_threshold_origin: threshold_origin,
                complete_root: drag_source.complete_root,
                partial_detachable: drag_source.partial_detachable,
                origin,
                source_validated_at: self.version,
                presentation: prepared.presentation,
                source_layout_facts: prepared.source_layout_facts,
                journal_source_geometry: Some(prepared.source_geometry),
                continuation,
            })
            .map_err(|source| EngineError::PointerInteractionInvariant {
                cause,
                detail: source.to_string(),
            })?;
        if replaced.is_some() {
            return Err(EngineError::ReductionCauseInvariant {
                detail: "tab gesture replaced an active gesture after busy validation",
            });
        }
        events.extend(candidate_events);
        Ok(InteractionOutcome::DragArmed { session, replaced })
    }

    pub(in crate::engine) fn request_pointer_pane_focus(
        &mut self,
        surface: SurfaceId,
        item: ItemId,
        causal: FocusCausalStamp,
        events: &mut Vec<WorkspaceEvent>,
    ) -> Result<(), EngineError> {
        if let Some(binding) = self
            .viewport
            .viewport(surface)
            .filter(|record| record.can_accept_activation())
            .map(crate::platform::registry::ViewportRecord::binding)
        {
            self.start_viewport_activation(
                ViewportActivationRequest::pointer_tab_gesture(binding, PanelFocus::Item(item)),
                causal,
                events,
            )?;
        } else {
            let native_guard = self.viewport_focus_binding(surface);
            self.viewport_focus
                .request_local_pane_focus(surface, item, native_guard, causal)
                .map_err(|source| EngineError::CausedViewportFocus {
                    cause: causal.cause(),
                    source,
                })?;
        }
        Ok(())
    }
}
