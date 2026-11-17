//! Public paint capabilities keep menu payloads independent of root actions.

use super::*;

fn split_session() -> DockspaceSession {
    DockspaceSession::from_layout(
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                ROOT,
                DockspaceNode::equal_split(
                    crate::model::DockspaceAxis::Horizontal,
                    [
                        DockspaceNode::central_tabs([FIRST, SECOND]),
                        DockspaceNode::tabs([THIRD, FOURTH]),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap(),
        DockPolicy::default(),
    )
    .unwrap()
}

fn menu_action(session: &mut DockspaceSession, whole_root: bool) -> PreparedSurfaceAction {
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface(
            SURFACE,
            UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, 900.0, 600.0).unwrap(),
                LogicalSize::new(40.0, 40.0).unwrap(),
                70.0,
            )
            .unwrap(),
        )
        .unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let command = if whole_root {
        plan.presentation_command(ROOT, DockspacePresentationCommandKind::Float)
            .unwrap()
            .unwrap()
    } else {
        let bar = plan
            .tab_bars()
            .find(|bar| bar.members().any(|member| member.item() == Some(FIRST)))
            .unwrap();
        assert!(plan.tabs_group_menu_operable(bar));
        plan.tabs_group_command(bar, DockspaceTabsGroupCommandKind::Float)
            .unwrap()
            .unwrap()
    };
    assert!(command.is_ready());
    let action = plan.prepare_presentation_command(command).unwrap();
    commit(frame);
    action
}

#[test]
fn group_and_root_menu_float_commands_preserve_their_distinct_payloads() {
    for whole_root in [false, true] {
        let mut session = split_session();
        let action = menu_action(&mut session, whole_root);
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_surface_action(action).unwrap();
        let report = commit(frame);
        assert!(report.workspace_changed(), "{report:?}");
        let view = session.view();
        let first = view.item(FIRST).unwrap();
        assert!(first.contained().is_some());
        assert_eq!(first.contained(), view.item(SECOND).unwrap().contained());
        assert_eq!(view.item(THIRD).unwrap().contained().is_some(), whole_root);
        assert_eq!(view.item(FOURTH).unwrap().contained().is_some(), whole_root);
        assert_eq!(
            first
                .tabs()
                .expect("fixture item is an inline tab")
                .items()
                .collect::<Vec<_>>(),
            &[FIRST, SECOND]
        );
    }
}

#[test]
fn a_group_menu_capability_does_not_refresh_after_an_unrelated_selection() {
    let mut session = split_session();
    let action = menu_action(&mut session, false);
    let mut frame = session.begin_host_frame().unwrap();
    frame.select_item_current(FOURTH).unwrap();
    commit(frame);
    let mut frame = session.begin_host_frame().unwrap();
    frame.submit_surface_action(action).unwrap();
    let report = commit(frame);
    assert!(!report.workspace_changed());
    assert!(session.view().item(FIRST).unwrap().contained().is_none());
    assert!(
        report
            .inputs()
            .iter()
            .any(|input| matches!(input, HostInputOutcome::StaleRejected { .. }))
    );
}
