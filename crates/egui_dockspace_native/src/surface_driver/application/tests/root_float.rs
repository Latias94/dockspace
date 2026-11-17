//! Complete-root floating through the same native command slot as individual panes.

use super::*;

fn assert_same_content(
    before: dockspace::model::DockspaceNodeView<'_>,
    after: dockspace::model::DockspaceNodeView<'_>,
) {
    let mut pending = vec![(before, after)];
    while let Some((before, after)) = pending.pop() {
        assert_eq!(before.is_central(), after.is_central());
        if let Some(tabs) = before.tabs() {
            let actual = after.tabs().expect("floating preserves tabs leaves");
            assert_eq!(actual.tabs(), tabs.tabs());
            assert_eq!(actual.selected(), tabs.selected());
        } else {
            let split = before.split().unwrap();
            let actual = after.split().expect("floating preserves split nodes");
            assert_eq!(actual.axis(), split.axis());
            assert_eq!(actual.child_count(), split.child_count());
            assert_eq!(
                actual.weights().collect::<Vec<_>>(),
                split.weights().collect::<Vec<_>>()
            );
            pending.extend(split.children().zip(actual.children()));
        }
    }
}

#[test]
fn native_float_root_command_preserves_nested_layout_and_uses_the_result_slot() {
    let last = ItemId::new(5);
    let content = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::central_tabs([FIRST]), 0.3),
            (
                DockspaceNode::split(
                    DockspaceAxis::Vertical,
                    [
                        (
                            DockspaceNode::tabs_with_selection([SECOND, THIRD], Some(THIRD))
                                .unwrap(),
                            0.6,
                        ),
                        (DockspaceNode::tabs([last]), 0.4),
                    ],
                )
                .unwrap(),
                0.7,
            ),
        ],
    )
    .unwrap();
    let mut state = state_with_content(content, false);
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    present_geometry(&mut state, bounds);
    let before = state.view().to_layout();
    let version = state.coordinator.session().version();
    state
        .request_command(NativeCommand::FloatRoot {
            root: ROOT,
            surface: SURFACE,
        })
        .unwrap();
    assert_eq!(state.coordinator.session().version(), version);
    assert!(state.view().surface(SURFACE).unwrap().main_root().is_some());
    assert_eq!(
        state
            .request_command(NativeCommand::SelectItem { item: FIRST })
            .unwrap_err()
            .reason(),
        NativeActionRequestError::Busy
    );
    commit_queued(&mut state);
    assert_eq!(
        state
            .request_command(NativeCommand::SelectItem { item: FIRST })
            .unwrap_err()
            .reason(),
        NativeActionRequestError::Busy,
        "the unconsumed root action result retains the command slot"
    );
    let NativeActionStatus::Dock(DockspaceActionStatus::Applied(
        DockspaceActionOutcome::RootFloated {
            root: ROOT,
            surface: SURFACE,
            floating,
            items,
            changed: true,
        },
    )) = take(&mut state)
    else {
        panic!("FloatRoot must produce the core's complete-root outcome");
    };
    assert_eq!(items, [FIRST, SECOND, THIRD, last]);
    let view = state.view();
    assert!(view.surface(SURFACE).unwrap().is_rootless());
    assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 1);
    assert_eq!(view.contained(floating).unwrap().root().unwrap().id(), ROOT);
    for item in [FIRST, SECOND, THIRD, last] {
        let item = view.item(item).unwrap();
        assert_eq!(item.root(), ROOT);
        assert_eq!(item.contained(), Some(floating));
    }
    assert_same_content(
        before.view().root(ROOT).unwrap().content().unwrap(),
        view.root(ROOT).unwrap().content().unwrap(),
    );
    assert_eq!(state.panes.0, "application-owned content");
    assert!(state.error().is_none());

    present_geometry(&mut state, bounds);
    state
        .request_command(NativeCommand::DockRootBack { root: ROOT })
        .unwrap();
    commit_queued(&mut state);
    assert!(matches!(
        take(&mut state),
        NativeActionStatus::Dock(DockspaceActionStatus::Applied(
            DockspaceActionOutcome::RootDocked {
                root: ROOT,
                target_root: ROOT,
                changed: true,
                ..
            }
        ))
    ));
    assert!(state.view().contained(floating).is_none());
    assert_eq!(
        state
            .view()
            .surface(SURFACE)
            .unwrap()
            .main_root()
            .unwrap()
            .id(),
        ROOT
    );
    assert_same_content(
        before.view().root(ROOT).unwrap().content().unwrap(),
        state.view().root(ROOT).unwrap().content().unwrap(),
    );
}

#[test]
fn native_float_root_command_preserves_core_unavailable_statuses() {
    let mut state = state();
    state
        .request_command(NativeCommand::FloatRoot {
            root: ROOT,
            surface: SURFACE,
        })
        .unwrap();
    commit_queued(&mut state);
    assert_eq!(
        take(&mut state),
        NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
            DockspaceActionRejection::PresentationUnavailable { surface: SURFACE }
        ))
    );
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    present_geometry(&mut state, bounds);
    for (root, surface, expected) in [
        (
            RootId::new(99),
            SURFACE,
            DockspaceActionRejection::RootUnavailable {
                root: RootId::new(99),
            },
        ),
        (
            ROOT,
            SurfaceId::new(99),
            DockspaceActionRejection::SurfaceUnavailable {
                surface: SurfaceId::new(99),
            },
        ),
    ] {
        state
            .request_command(NativeCommand::FloatRoot { root, surface })
            .unwrap();
        commit_queued(&mut state);
        assert_eq!(
            take(&mut state),
            NativeActionStatus::Dock(DockspaceActionStatus::Rejected(expected))
        );
        assert!(state.view().item(FIRST).unwrap().contained().is_none());
        assert!(state.error().is_none());
    }
    state
        .request_command(NativeCommand::SetArrangementLocked { locked: true })
        .unwrap();
    commit_queued(&mut state);
    let _ = take(&mut state);
    present_geometry(&mut state, bounds);
    state
        .request_command(NativeCommand::FloatRoot {
            root: ROOT,
            surface: SURFACE,
        })
        .unwrap();
    commit_queued(&mut state);
    assert_eq!(
        take(&mut state),
        NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
            DockspaceActionRejection::PolicyDenied
        ))
    );
    assert!(state.view().surface(SURFACE).unwrap().main_root().is_some());
    assert!(state.error().is_none());
}

#[test]
fn native_float_root_command_cannot_remove_the_fixed_host_surface() {
    let target_surface = SurfaceId::new(2);
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([FIRST, SECOND])),
        ),
        DockspaceSurfaceLayout::new(
            target_surface,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([THIRD])),
        ),
    ])
    .unwrap();
    let mut state = state_with_layout(layout);
    let version = state.coordinator.session().version();
    let command = NativeCommand::FloatRoot {
        root: ROOT,
        surface: target_surface,
    };
    let error = state.request_command(command.clone()).unwrap_err();
    assert_eq!(
        error.reason(),
        NativeActionRequestError::HostSurfaceRequired { surface: SURFACE }
    );
    assert_eq!(error.into_command(), command);
    assert!(!state.application_actions.is_occupied());
    assert_eq!(state.coordinator.session().version(), version);
    assert!(state.view().surface(SURFACE).unwrap().main_root().is_some());
}
