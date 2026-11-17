//! Graph-level activation guarantees for edits inside inactive layout branches.

use super::*;

#[test]
fn graph_open_activates_every_ancestor_without_showing_hidden_existing_content() {
    const OPENED: ItemId = ItemId::new(5);

    for target_item in [A, D] {
        let mut workspace = vertical_layout()
            .with_hidden_items([A, B, D])
            .unwrap()
            .into_workspace();
        let (outer, tools, split) = outer_and_tools(&workspace);
        let source = workspace
            .capture_item_source_by_id(target_item)
            .unwrap()
            .unwrap();
        let target_owner = source.owner_node();
        let target = workspace.capture_tab_target(ROOT, target_owner).unwrap();
        let report = WorkspaceTransaction::from_commands([WorkspaceCommand::Open {
            item: OPENED,
            target: DockTarget::Center(target),
        }])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap();
        assert_eq!(
            report.outcomes(),
            [CommandOutcome::Opened {
                item: OPENED,
                root: ROOT
            }]
        );
        assert!(workspace.item_is_active(OPENED));
        assert!(!workspace.item_is_active(C));
        assert_eq!(workspace.visible_tab_selection(outer), Some(tools));
        assert_eq!(workspace.first_active_item(split), Some(OPENED));
        assert_eq!(workspace.hidden_items().collect::<Vec<_>>(), [A, B, D]);
        assert_eq!(
            workspace.locate_item_by_id(OPENED).unwrap().owner_node(),
            target_owner
        );
        assert_eq!(
            workspace.tab_label(outer, tools),
            Some(TabLabel::Layout { title: "Tools" })
        );
        let expected = if target_item == A {
            vec![C, A, B, OPENED, D]
        } else {
            vec![C, A, B, D, OPENED]
        };
        assert_eq!(workspace.collect_items_in_subtree(outer), expected);
        workspace.validate().unwrap();
    }
}

#[test]
fn graph_open_activation_rolls_back_when_a_later_command_uses_a_stale_scope() {
    const OPENED: ItemId = ItemId::new(5);

    for target_item in [A, D] {
        let mut workspace = vertical_layout()
            .with_hidden_items([A, B, D])
            .unwrap()
            .into_workspace();
        let source = workspace
            .capture_item_source_by_id(target_item)
            .unwrap()
            .unwrap();
        let target = workspace
            .capture_tab_target(ROOT, source.owner_node())
            .unwrap();
        let stale = workspace.capture_item_source_by_id(C).unwrap().unwrap();
        let before = workspace.clone();
        let before_reference = workspace.root_reference(ROOT).unwrap().clone();
        let error = WorkspaceTransaction::from_commands([
            WorkspaceCommand::Open {
                item: OPENED,
                target: DockTarget::Center(target),
            },
            WorkspaceCommand::SelectItem { source: stale },
        ])
        .apply(&mut workspace, &DockPolicySnapshot::default())
        .unwrap_err();
        assert!(matches!(
            error,
            TransactionError::Command {
                index: 1,
                source: CommandError::StaleNode { .. },
            }
        ));
        assert_eq!(workspace, before);
        assert_eq!(workspace.root_reference(ROOT), Some(&before_reference));
        assert!(workspace.item_is_active(C));
        assert!(workspace.locate_item_by_id(OPENED).is_none());
    }
}
