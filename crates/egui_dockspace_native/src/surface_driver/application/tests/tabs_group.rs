//! Group chrome preserves physical-window ownership and close-plan scope.

use super::*;

fn group_move_state() -> NativeRuntimeState<Panes> {
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::central_tabs([FIRST]),
            DockspaceNode::tabs_with_selection([SECOND, THIRD], Some(THIRD)).unwrap(),
        ],
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, content),
    )])
    .unwrap()
    .with_hidden_items([SECOND])
    .unwrap();
    let mut state = state_with_layout(layout);
    present_geometry(
        &mut state,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
    );
    state
}

#[test]
fn native_group_insertion_mode_controls_center_and_ordered_tab_destinations() {
    use dockspace::model::DockspaceTabView;

    for mode in [
        TabGroupDockMode::MergeTabs,
        TabGroupDockMode::PreserveLayout,
    ] {
        for placement in [
            DockPlacement::Center(DockAnchor::Item(FIRST)),
            DockPlacement::Before(FIRST),
            DockPlacement::After(FIRST),
        ] {
            let mut state = group_move_state();
            let source = state.view().item(THIRD).unwrap().tabs().unwrap().key();
            let group = state.capture_tabs_group(DockAnchor::Item(THIRD)).unwrap();
            state
                .request_command(NativeCommand::DockTabs {
                    group,
                    placement,
                    mode,
                })
                .unwrap();
            commit_queued(&mut state);
            assert!(matches!(
                take(&mut state),
                NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                    DockspaceActionOutcome::TabsDocked { items, changed: true, .. }
                )) if items == [SECOND, THIRD]
            ));

            let view = state.view();
            let target = view.item(FIRST).unwrap().tabs().unwrap();
            let before = matches!(placement, DockPlacement::Before(_));
            match mode {
                TabGroupDockMode::MergeTabs => {
                    let expected = if before {
                        [SECOND, THIRD, FIRST]
                    } else {
                        [FIRST, SECOND, THIRD]
                    };
                    assert_eq!(target.tabs(), expected.map(Into::into));
                    assert_eq!(target.selected(), Some(THIRD.into()));
                }
                TabGroupDockMode::PreserveLayout => {
                    assert_eq!(target.tabs().len(), 2);
                    let index = usize::from(!before);
                    let key = target.tabs()[index];
                    let Some(DockspaceTabView::Layout(layout)) = target.child(key) else {
                        panic!("preserved groups must remain complete layout tabs");
                    };
                    assert_eq!(layout.key(), source);
                    assert_eq!(target.selected(), Some(key));
                    let nested = layout.tabs().unwrap();
                    assert_eq!(nested.tabs(), [SECOND.into(), THIRD.into()]);
                    assert_eq!(nested.selected(), Some(THIRD.into()));
                    assert_eq!(target.tabs()[1 - index], FIRST.into());
                }
            }
            assert!(view.item(SECOND).unwrap().is_hidden());
            assert!(view.item(THIRD).unwrap().is_active());
            assert_eq!(view.root(ROOT).unwrap().items().count(), 3);
            assert!(view.root(ROOT).unwrap().central().is_some());
            assert!(!state.coordinator.session().has_pending_presentation());
            assert!(state.error().is_none());
        }
    }
}

#[test]
fn native_group_insertion_modes_share_the_same_edge_split_behavior() {
    use dockspace::model::{DockEdge, DockFraction};

    for edge in [
        DockEdge::Left,
        DockEdge::Right,
        DockEdge::Top,
        DockEdge::Bottom,
    ] {
        let mut layouts = Vec::new();
        for mode in [
            TabGroupDockMode::MergeTabs,
            TabGroupDockMode::PreserveLayout,
        ] {
            let mut state = group_move_state();
            let group = state.capture_tabs_group(DockAnchor::Item(THIRD)).unwrap();
            state
                .request_command(NativeCommand::DockTabs {
                    group,
                    placement: DockPlacement::InnerEdge {
                        anchor: DockAnchor::Item(FIRST),
                        edge,
                        fraction: DockFraction::new(0.35).unwrap(),
                    },
                    mode,
                })
                .unwrap();
            commit_queued(&mut state);
            assert!(matches!(
                take(&mut state),
                NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                    DockspaceActionOutcome::TabsDocked { changed: true, .. }
                ))
            ));
            let view = state.view();
            assert!(view.item(SECOND).unwrap().is_hidden());
            assert_eq!(
                view.item(THIRD).unwrap().tabs().unwrap().selected(),
                Some(THIRD.into())
            );
            assert_eq!(view.root(ROOT).unwrap().items().count(), 3);
            let split = view.root(ROOT).unwrap().content().unwrap().split().unwrap();
            let children = split
                .children()
                .map(|node| (node.is_central(), node.items().collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            layouts.push((
                split.axis(),
                split.weights().map(f64::to_bits).collect::<Vec<_>>(),
                children,
            ));
        }
        assert_eq!(
            layouts[0], layouts[1],
            "edge insertion must ignore center mode"
        );
    }
}

#[test]
fn a_complete_native_group_menu_does_not_offer_another_window_for_the_same_payload() {
    use dockspace::runtime::{
        DockspacePresentationCommandUnavailable, DockspaceTabsGroupCommandKind, HostWindowToken,
        NativePointerRoster,
    };

    let child_surface = SurfaceId::new(2);
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs([THIRD])),
        ),
        DockspaceSurfaceLayout::new(
            child_surface,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([FIRST, SECOND])),
        ),
    ])
    .unwrap();
    let mut policy = DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    let style = DockStyle::default();
    let mut session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        policy,
        style.presentation_config().unwrap(),
    )
    .unwrap();
    session
        .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
        .unwrap();
    session
        .configure_managed_native_capabilities(crate::capabilities::capabilities_for_backend(
            eframe::NativeWindowingBackend::Windows,
        ))
        .unwrap();
    session
        .register_native_root(SURFACE, HostWindowToken::new(11))
        .unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit().unwrap();
    let host = report
        .native_bindings()
        .iter()
        .copied()
        .find(|binding| binding.surface() == SURFACE)
        .unwrap();
    session
        .register_owned_native_child(child_surface, HostWindowToken::new(22), SURFACE)
        .unwrap();
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit().unwrap();
    let child = report
        .native_bindings()
        .iter()
        .copied()
        .find(|binding| binding.surface() == child_surface)
        .expect("the existing owned child receives a real recovery binding");
    let live_facts = |x| {
        let bounds = PhysicalRect::new(x, 0.0, 800.0, 600.0).unwrap();
        let scale = ScaleFactor::new(1.0).unwrap();
        NativeWindowFacts::live()
            .with_content_bounds(bounds)
            .with_outer_bounds(bounds)
            .with_native_scale_factor(scale)
            .with_presentation_scale_factor(scale)
            .with_input(NativeWindowInputState::ReceivesInput, None)
            .with_presentation(NativeWindowPresentationState::Visible)
            .with_close(NativeCloseState::Clear, None)
    };
    session
        .report_managed_native_snapshot(
            [(host, live_facts(0.0)), (child, live_facts(900.0))],
            NativeWorkAreaRoster::Exact(vec![NativeWorkAreaFacts::new(
                HostWorkAreaToken::new(1),
                PhysicalRect::new(0.0, 0.0, 1920.0, 1080.0).unwrap(),
                ScaleFactor::new(1.0).unwrap(),
            )]),
        )
        .unwrap();
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    let mut frame = session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .measure_surface(
            child_surface,
            UniformSurfaceMetrics::new(bounds, LogicalSize::new(64.0, 48.0).unwrap(), 96.0)
                .unwrap(),
        )
        .unwrap();
    let plan = frame.paint_plan(child_surface).unwrap().unwrap();
    let mut bar = None;
    plan.visit_surface_program(|step| {
        if let dockspace::runtime::SurfacePaintProgramStep::Operation {
            operation: dockspace::runtime::SurfacePaintProgramOperation::TabBar(record),
            ..
        } = step
        {
            bar = Some(record);
        }
        Ok::<(), ()>(())
    })
    .unwrap();
    let bar = bar.expect("the complete native group has one visible tab bar");
    let command = plan
        .tabs_group_command(bar, DockspaceTabsGroupCommandKind::MoveToNewWindow)
        .unwrap()
        .expect("native window creation is supported");
    assert_eq!(
        command.unavailable_reason(),
        Some(DockspacePresentationCommandUnavailable::AlreadyNative)
    );
}

#[test]
fn closing_one_group_keeps_sibling_content_and_the_physical_host_alive() {
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::tabs([FIRST, SECOND]),
            DockspaceNode::tabs([THIRD]),
        ],
    )
    .unwrap();
    let mut state = state_with_content(content, false);
    let binding = state
        .coordinator
        .viewport_binding(egui::ViewportId::ROOT)
        .unwrap();
    let group = state.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let mut frame = state
        .coordinator
        .begin_host_frame(|_| NativeReceiverAnswer::Unknown)
        .unwrap();
    let submitted = frame.frame.request_close_tabs_current(group).unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let report = frame.commit().unwrap();
    let plan = match report.submitted_action_outcome(submitted) {
        Some(HostInputOutcome::CloseRequested { plan, .. }) => plan.clone(),
        outcome => panic!("group close must open one core plan, got {outcome:?}"),
    };
    state.coordinator.settle_host_frame_report(&report).unwrap();
    for item in plan.items() {
        state
            .request_command(NativeCommand::DecideClose {
                request: plan.request(),
                token: item.token(),
                decision: CloseDecision::Allow,
            })
            .unwrap();
        commit_queued(&mut state);
        take(&mut state);
    }
    assert!(state.view().item(FIRST).is_none());
    assert!(state.view().item(SECOND).is_none());
    assert_eq!(state.view().item(THIRD).unwrap().root(), ROOT);
    assert_eq!(
        state.coordinator.viewport_binding(egui::ViewportId::ROOT),
        Some(binding)
    );
    assert!(state.shutdown.is_none());
    assert!(state.error().is_none());
}
