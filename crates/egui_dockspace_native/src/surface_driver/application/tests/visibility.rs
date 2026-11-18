use super::*;

#[test]
fn hiding_the_last_host_pane_uses_normal_command_results_and_keeps_its_window() {
    let mut state = state_with_content(DockspaceNode::tabs([FIRST]), false);
    let bindings = state
        .coordinator
        .session()
        .native_window_bindings()
        .collect::<Vec<_>>();
    let content = state.panes.0.clone();
    for visible in [false, true] {
        let state_receipt = state
            .request_command(NativeCommand::SetItemVisibility {
                item: FIRST,
                visible,
            })
            .unwrap();
        commit_queued(&mut state);
        assert_eq!(
            take(&state_receipt),
            NativeActionStatus::Dock(DockspaceActionStatus::Applied(
                DockspaceActionOutcome::ItemVisibilityChanged {
                    item: FIRST,
                    visible,
                    changed: true,
                }
            )),
        );
        assert_eq!(state.view().item(FIRST).unwrap().is_visible(), visible);
        assert_eq!(state.view().item(FIRST).unwrap().root(), ROOT);
        assert_eq!(
            state
                .coordinator
                .session()
                .native_window_bindings()
                .collect::<Vec<_>>(),
            bindings,
        );
        assert_eq!(state.panes.0, content);
        assert!(state.error().is_none());

        if !visible {
            let state_receipt = state
                .request_command(NativeCommand::SelectItem { item: FIRST })
                .unwrap();
            commit_queued(&mut state);
            assert_eq!(
                take(&state_receipt),
                NativeActionStatus::Dock(DockspaceActionStatus::Rejected(
                    DockspaceActionRejection::ItemHidden { item: FIRST },
                )),
            );
            assert!(state.error().is_none());
        }
    }
}
