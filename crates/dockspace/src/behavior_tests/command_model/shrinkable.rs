use super::*;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

#[derive(Debug, Clone, Copy)]
enum ModelAction {
    Select {
        item: ItemId,
    },
    Reorder {
        item: ItemId,
        insertion: usize,
    },
    CenterMove {
        item: ItemId,
        target_item: ItemId,
    },
    EdgeMove {
        item: ItemId,
        target_item: ItemId,
        edge: Edge,
        fraction: DockFraction,
    },
    Open {
        item: ItemId,
        target_item: ItemId,
    },
    ResizeContaining {
        item: ItemId,
    },
}

fn action_strategy() -> impl Strategy<Value = ModelAction> {
    let item = (1..=ITEM_COUNT).prop_map(ItemId::new);
    let max_insertion = usize::try_from(ITEM_COUNT).expect("model item count fits usize");
    prop_oneof![
        item.clone().prop_map(|item| ModelAction::Select { item }),
        (item.clone(), 0..=max_insertion)
            .prop_map(|(item, insertion)| ModelAction::Reorder { item, insertion }),
        (item.clone(), item.clone())
            .prop_map(|(item, target_item)| ModelAction::CenterMove { item, target_item }),
        (
            item.clone(),
            item.clone(),
            prop::sample::select(vec![Edge::Left, Edge::Right, Edge::Top, Edge::Bottom]),
            prop::sample::select(vec![0.25_f64, 0.5, 0.75]),
        )
            .prop_map(
                |(item, target_item, edge, fraction)| ModelAction::EdgeMove {
                    item,
                    target_item,
                    edge,
                    fraction: DockFraction::new(fraction).expect("generated fraction is valid"),
                }
            ),
        (item.clone(), item.clone())
            .prop_map(|(item, target_item)| ModelAction::Open { item, target_item }),
        item.prop_map(|item| ModelAction::ResizeContaining { item }),
    ]
}

fn resolve_action(
    action: ModelAction,
    workspace: &Workspace,
    open: &BTreeSet<ItemId>,
) -> Option<(WorkspaceCommand, ModelDelta)> {
    // Shrinking can remove a prerequisite. Never replace a missing item or target.
    Some(match action {
        ModelAction::Select { item } => {
            select_item_command(workspace, item_tabs(workspace, item)?, item)
        }
        ModelAction::Reorder { item, insertion } => {
            let tabs = item_tabs(workspace, item)?;
            let Node::Tabs { items, .. } = workspace.node(tabs)? else {
                unreachable!("item lookup must return tabs");
            };
            if insertion > items.len() {
                return None;
            }
            reorder_item_command(workspace, tabs, item, insertion)
        }
        ModelAction::CenterMove { item, target_item } => center_item_command(
            workspace,
            item_tabs(workspace, item)?,
            item,
            item_tabs(workspace, target_item)?,
        ),
        ModelAction::EdgeMove {
            item,
            target_item,
            edge,
            fraction,
        } => edge_item_command(
            workspace,
            item_tabs(workspace, item)?,
            item,
            item_tabs(workspace, target_item)?,
            edge,
            fraction,
        ),
        ModelAction::Open { item, target_item } => {
            if open.contains(&item) {
                return None;
            }
            open_item_command(workspace, item, item_tabs(workspace, target_item)?)
        }
        ModelAction::ResizeContaining { item } => {
            let tabs = item_tabs(workspace, item)?;
            let (split, child_count) = workspace.nodes().find_map(|(node, value)| match value {
                Node::Split { children, .. } if children.contains(&tabs) => {
                    Some((node, children.len()))
                }
                Node::Split { .. } | Node::Tabs { .. } => None,
            })?;
            resize_split_command(workspace, split, child_count)
        }
    })
}

fn apply_action(
    action: ModelAction,
    workspace: &mut Workspace,
    expected: &mut BTreeSet<ItemId>,
) -> bool {
    let Some((command, delta)) = resolve_action(action, workspace, expected) else {
        return false;
    };
    let before = workspace.clone();
    match WorkspaceTransaction::from_commands([command])
        .apply(workspace, &DockPolicySnapshot::default())
    {
        Err(_) => assert_eq!(*workspace, before, "rejected action: {action:?}"),
        Ok(report) => {
            assert_success_postcondition(&delta, &report, &before, workspace);
            if let ModelDelta::Open { item, .. } = delta {
                assert!(expected.insert(item), "duplicate open: {action:?}");
            }
        }
    }
    workspace
        .validate()
        .expect("every published action must remain strict");
    assert_eq!(
        workspace.item_multiset(),
        expected.iter().copied().map(|item| (item, 1)).collect(),
        "membership after {action:?}"
    );
    true
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
        ..ProptestConfig::with_cases(128)
    })]

    #[test]
    fn shrinkable_command_sequences_preserve_atomicity_membership_and_postconditions(
        actions in prop::collection::vec(action_strategy(), 1..65),
    ) {
        let mut workspace = initial_workspace();
        let mut expected = (1..=INITIAL_OPEN_ITEMS).map(ItemId::new).collect();
        for action in actions {
            apply_action(action, &mut workspace, &mut expected);
        }
    }
}

#[test]
fn removing_open_prerequisite_skips_the_original_target_instead_of_retargeting() {
    let open = ModelAction::Open {
        item: ItemId::new(9),
        target_item: ItemId::new(1),
    };
    let move_to_opened = ModelAction::EdgeMove {
        item: ItemId::new(3),
        target_item: ItemId::new(9),
        edge: Edge::Top,
        fraction: DockFraction::new(0.5).expect("valid model fraction"),
    };
    let initial = initial_workspace();
    let initial_members = (1..=INITIAL_OPEN_ITEMS)
        .map(ItemId::new)
        .collect::<BTreeSet<_>>();
    let mut complete = initial.clone();
    let mut complete_members = initial_members.clone();
    assert!(apply_action(open, &mut complete, &mut complete_members));
    assert!(apply_action(
        move_to_opened,
        &mut complete,
        &mut complete_members
    ));
    assert_ne!(
        item_tabs(&complete, ItemId::new(3)),
        item_tabs(&initial, ItemId::new(3))
    );

    let mut shortened = initial.clone();
    let mut shortened_members = initial_members.clone();
    assert!(!apply_action(
        move_to_opened,
        &mut shortened,
        &mut shortened_members
    ));
    assert_eq!(shortened, initial);
    assert_eq!(shortened_members, initial_members);
}
