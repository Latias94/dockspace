//! Exact tab commands preserve composite ownership and native host protection.

use dockspace::close::ClosePlanTarget;
use dockspace::model::{DockspaceGridShape, DockspaceTabView, NodeKey, TabKey};
use dockspace::runtime::{DockspaceCloseOutcome, DockspaceCloseRequestRejection};

use super::*;

const TARGET: ItemId = ItemId::new(5);

fn tools_layout() -> DockspaceNode {
    DockspaceNode::grid(
        DockspaceGridShape::Auto,
        [
            DockspaceNode::tabs([SECOND, THIRD]),
            DockspaceNode::pane(CLOSED),
        ],
    )
    .unwrap()
    .named("Tools")
}

fn composed_state(companion: bool) -> NativeRuntimeState<Panes> {
    let mut surface = DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([DockspaceNode::pane(FIRST), tools_layout()]),
        ),
    );
    if companion {
        surface = surface.with_contained(DockspaceContainedLayout::new(
            FloatingPresentationId::new(1),
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([TARGET])),
            LogicalRect::new(80.0, 80.0, 320.0, 240.0).unwrap(),
        ));
    }
    state_with_layout(
        DockspaceLayout::new([surface])
            .unwrap()
            .with_hidden_items([FIRST, THIRD])
            .unwrap(),
    )
}

fn tools_key(state: &NativeRuntimeState<Panes>) -> (TabKey, NodeKey) {
    let tabs = state
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap()
        .tabs()
        .unwrap();
    let key = tabs.tabs()[1];
    let Some(DockspaceTabView::Layout(node)) = tabs.child(key) else {
        panic!("Tools must be one direct layout tab");
    };
    (key, node.key())
}

#[test]
fn docking_and_floating_a_layout_tab_preserve_its_grid_and_every_owned_descendant() {
    for float in [false, true] {
        let mut state = composed_state(true);
        present_geometry(
            &mut state,
            LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
        );
        let (key, node_key) = tools_key(&state);
        let tab = state.capture_tab(key).unwrap();
        assert!(state.capture_tab(CLOSED.into()).is_none());
        let command = if float {
            NativeCommand::Move {
                source: dockspace::model::DockMoveSource::Tab(tab),
                target: dockspace::model::DockMoveTarget::Contained { surface: SURFACE },
            }
        } else {
            NativeCommand::Move {
                source: dockspace::model::DockMoveSource::Tab(tab),
                target: dockspace::model::DockMoveTarget::Dock {
                    placement: DockPlacement::Center(DockAnchor::Item(TARGET)),
                    mode: dockspace::model::TabGroupDockMode::MergeTabs,
                },
            }
        };
        let state_receipt = state.request_command(command).unwrap();
        commit_queued(&mut state);
        let moved_root = match take(&state_receipt) {
            NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                DockspaceActionOutcome::TabDocked {
                    tab,
                    source_root: ROOT,
                    target_root,
                    items,
                    changed: true,
                },
            )) if !float => {
                assert_eq!(tab, key);
                assert_eq!(items, [SECOND, THIRD, CLOSED]);
                let tabs = state
                    .view()
                    .root(target_root)
                    .unwrap()
                    .content()
                    .unwrap()
                    .tabs()
                    .unwrap();
                assert_eq!(tabs.tabs(), [TARGET.into(), key]);
                assert_eq!(tabs.selected(), Some(key));
                target_root
            }
            NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                DockspaceActionOutcome::TabFloated {
                    tab,
                    source_root: ROOT,
                    root,
                    surface: SURFACE,
                    items,
                    changed: true,
                    ..
                },
            )) if float => {
                assert_eq!(tab, key);
                assert_eq!(items, [SECOND, THIRD, CLOSED]);
                assert_eq!(
                    state.view().root(root).unwrap().content().unwrap().key(),
                    node_key
                );
                root
            }
            outcome => panic!("the exact layout tab must produce one move result: {outcome:?}"),
        };
        let view = state.view();
        let tools = view.node(node_key).unwrap();
        assert_eq!(tools.root().unwrap().id(), moved_root);
        assert_eq!(tools.layout_title(), Some("Tools"));
        assert_eq!(tools.items().collect::<Vec<_>>(), [SECOND, THIRD, CLOSED]);
        let grid = tools
            .grid()
            .expect("moving the layout tab preserves the real Grid");
        assert_eq!(grid.shape(), DockspaceGridShape::Auto);
        assert_eq!(grid.child_count(), 2);
        assert!(grid.shares().is_none());
        assert_eq!(
            view.root(moved_root)
                .unwrap()
                .active_items()
                .collect::<Vec<_>>(),
            [SECOND, CLOSED]
        );
        assert_eq!(
            view.root(ROOT).unwrap().items().collect::<Vec<_>>(),
            [FIRST]
        );
        assert!(view.root(ROOT).unwrap().active_items().next().is_none());
        assert!(view.item(FIRST).unwrap().is_hidden());
        assert!(view.item(THIRD).unwrap().is_hidden());
        assert!(view.item(CLOSED).unwrap().tabs().is_none());
        assert!(state.error().is_none());
    }
}

#[test]
fn closing_a_layout_tab_approves_hidden_descendants_atomically_and_keeps_hidden_host_content() {
    let mut state = composed_state(false);
    let (key, _) = tools_key(&state);
    let binding = state
        .coordinator
        .viewport_binding(egui::ViewportId::ROOT)
        .unwrap();
    for veto in [true, false] {
        let state_receipt = state
            .request_command(NativeCommand::CloseTab {
                tab: state.capture_tab(key).unwrap(),
            })
            .unwrap();
        commit_queued(&mut state);
        let NativeActionStatus::CloseRequested {
            plan,
            reused: false,
        } = take(&state_receipt)
        else {
            panic!("the layout tab must expose its own atomic close plan");
        };
        assert_eq!(
            plan.target(),
            ClosePlanTarget::Tab {
                root: ROOT,
                tab: key
            }
        );
        assert_eq!(
            plan.items()
                .iter()
                .map(|item| item.item())
                .collect::<Vec<_>>(),
            [SECOND, THIRD, CLOSED]
        );
        for (index, item) in plan.items().iter().enumerate() {
            let decision = if veto && index == 1 {
                CloseDecision::Veto
            } else {
                CloseDecision::Allow
            };
            let state_receipt = state
                .request_command(NativeCommand::DecideClose {
                    request: plan.request(),
                    token: item.token(),
                    decision,
                })
                .unwrap();
            commit_queued(&mut state);
            let outcome = take(&state_receipt);
            if veto || index + 1 < plan.items().len() {
                assert!(matches!(
                    outcome,
                    NativeActionStatus::CloseDecisionProcessed { changed: false, .. }
                ));
                assert_eq!(
                    state.view().root(ROOT).unwrap().items().collect::<Vec<_>>(),
                    [FIRST, SECOND, THIRD, CLOSED]
                );
            } else {
                assert!(matches!(outcome,
                    NativeActionStatus::CloseDecisionProcessed {
                        application: Some(Ok(DockspaceCloseOutcome::TabClosed { root: ROOT, tab, items })),
                        changed: true, ..
                    } if tab == key && items == [SECOND, THIRD, CLOSED]
                ));
            }
            if decision == CloseDecision::Veto {
                break;
            }
        }
        assert!(state.pending_content_close_plans().is_empty());
    }
    assert_eq!(
        state.view().root(ROOT).unwrap().items().collect::<Vec<_>>(),
        [FIRST]
    );
    assert!(state.view().item(FIRST).unwrap().is_hidden());
    assert!(
        state
            .view()
            .root(ROOT)
            .unwrap()
            .active_items()
            .next()
            .is_none()
    );
    assert!(state.capture_tab(key).is_none());
    for item in [SECOND, THIRD, CLOSED] {
        assert!(state.view().item(item).is_none());
    }
    assert_eq!(
        state.coordinator.viewport_binding(egui::ViewportId::ROOT),
        Some(binding)
    );
    assert!(state.error().is_none());
}

#[test]
fn a_complete_layout_tab_cannot_remove_the_fixed_native_host() {
    let target = SurfaceId::new(2);
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::composed_tabs([tools_layout()])),
        ),
        DockspaceSurfaceLayout::new(
            target,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::central_tabs([FIRST])),
        ),
    ])
    .unwrap()
    .with_hidden_items([THIRD])
    .unwrap();
    let mut state = state_with_layout(layout);
    present_geometry(
        &mut state,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
    );
    let key = state
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap()
        .tabs()
        .unwrap()
        .tabs()[0];
    let tab = state.capture_tab(key).unwrap();
    let version = state.coordinator.session().version();
    for command in [
        NativeCommand::Move {
            source: dockspace::model::DockMoveSource::Tab(tab.clone()),
            target: dockspace::model::DockMoveTarget::Dock {
                placement: DockPlacement::Center(DockAnchor::Item(FIRST)),
                mode: dockspace::model::TabGroupDockMode::MergeTabs,
            },
        },
        NativeCommand::Move {
            source: dockspace::model::DockMoveSource::Tab(tab.clone()),
            target: dockspace::model::DockMoveTarget::Contained { surface: target },
        },
        NativeCommand::CloseTab { tab },
    ] {
        let error = state.request_command(command.clone()).unwrap_err();
        assert_eq!(
            error.reason(),
            NativeActionRequestError::HostSurfaceRequired { surface: SURFACE }
        );
        assert_eq!(error.into_command(), command);
        assert!(!state.application_actions.is_occupied());
        assert_eq!(state.coordinator.session().version(), version);
        assert_eq!(
            state.view().root(ROOT).unwrap().items().collect::<Vec<_>>(),
            [SECOND, THIRD, CLOSED]
        );
    }
}

#[test]
fn native_tab_commands_preserve_stale_and_foreign_captures_without_refreshing_them() {
    for foreign in [false, true] {
        for kind in 0..3 {
            let mut source = composed_state(false);
            let (key, _) = tools_key(&source);
            let tab = source.capture_tab(key).unwrap();
            let captured = tab.expected_version();
            if !foreign {
                let source_receipt = source
                    .request_command(NativeCommand::SetItemVisibility {
                        item: FIRST,
                        visible: true,
                    })
                    .unwrap();
                commit_queued(&mut source);
                take(&source_receipt);
            }
            let mut state = if foreign {
                composed_state(false)
            } else {
                source
            };
            let accepted = state.coordinator.session().version();
            let command = match kind {
                0 => NativeCommand::Move {
                    source: dockspace::model::DockMoveSource::Tab(tab),
                    target: dockspace::model::DockMoveTarget::Dock {
                        placement: DockPlacement::Center(DockAnchor::Item(FIRST)),
                        mode: dockspace::model::TabGroupDockMode::MergeTabs,
                    },
                },
                1 => NativeCommand::Move {
                    source: dockspace::model::DockMoveSource::Tab(tab),
                    target: dockspace::model::DockMoveTarget::Contained { surface: SURFACE },
                },
                _ => NativeCommand::CloseTab { tab },
            };
            let state_receipt = state.request_command(command).unwrap();
            let report = commit_queued_report(&mut state);
            assert_eq!(report.before(), report.after());
            let expected = match (foreign, kind) {
                (true, 2) => {
                    NativeActionStatus::CloseRejected(DockspaceCloseRequestRejection::Conflict {
                        target: ClosePlanTarget::Tab {
                            root: ROOT,
                            tab: key,
                        },
                    })
                }
                (true, _) => NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
                    DockspaceActionRejection::Conflict,
                )),
                (false, 2) => NativeActionStatus::Stale {
                    expected: captured,
                    accepted,
                },
                (false, _) => NativeActionStatus::Dock(DockspaceActionStatus::Stale {
                    expected: captured,
                    accepted,
                }),
            };
            assert_eq!(take(&state_receipt), expected);
            assert_eq!(
                state.view().root(ROOT).unwrap().items().collect::<Vec<_>>(),
                [FIRST, SECOND, THIRD, CLOSED]
            );
            assert!(state.pending_content_close_plans().is_empty());
            assert!(!state.application_actions.is_occupied());
            assert!(state.error().is_none());
        }
    }
}

#[test]
fn stale_close_of_the_only_host_tab_reports_stale_before_retention() {
    let target = SurfaceId::new(2);
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([FIRST])),
        ),
        DockspaceSurfaceLayout::new(
            target,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::central_tabs([SECOND, THIRD])),
        ),
    ])
    .unwrap();
    let mut state = state_with_layout(layout);
    let tab = state.capture_tab(TabKey::for_item(FIRST)).unwrap();
    let expected = tab.expected_version();
    let selection = state
        .request_command(NativeCommand::SelectItem { item: THIRD })
        .unwrap();
    commit_queued(&mut state);
    take(&selection);
    assert!(state.view().item(THIRD).unwrap().is_active());
    let accepted = state.coordinator.session().version();
    assert_ne!(expected, accepted);

    let close = state
        .request_command(NativeCommand::CloseTab { tab })
        .unwrap();
    let report = commit_queued_report(&mut state);
    assert_eq!(
        take(&close),
        NativeActionStatus::Stale { expected, accepted }
    );
    assert!(!report.workspace_changed());
    assert!(state.view().item(FIRST).is_some());
}
