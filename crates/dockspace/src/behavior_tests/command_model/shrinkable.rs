use super::*;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

mod semantics;
use semantics::{Prediction, Semantics};

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
    Visibility {
        item: ItemId,
        visible: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Skipped,
    Rejected,
    Applied { changed: bool },
}

#[derive(Default)]
struct Coverage {
    skipped: usize,
    rejected: usize,
    applied: usize,
    changed: usize,
}

impl Coverage {
    fn record(&mut self, step: Step) {
        match step {
            Step::Skipped => self.skipped += 1,
            Step::Rejected => self.rejected += 1,
            Step::Applied { changed } => {
                self.applied += 1;
                self.changed += usize::from(changed);
            }
        }
    }
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
        item.clone()
            .prop_map(|item| ModelAction::ResizeContaining { item }),
        (item, any::<bool>()).prop_map(|(item, visible)| ModelAction::Visibility { item, visible }),
    ]
}

fn resolve_action(
    action: ModelAction,
    workspace: &Workspace,
    expected: &Semantics,
) -> (WorkspaceCommand, Option<ModelDelta>) {
    // Only the independent model decides applicability. Missing SUT authority is a failure.
    let tabs_for = |item| item_tabs(workspace, item).expect("model-owned item must resolve in SUT");
    let resolved = match action {
        ModelAction::Select { item } => select_item_command(workspace, tabs_for(item), item),
        ModelAction::Reorder { item, insertion } => {
            let tabs = tabs_for(item);
            let Node::Tabs { items, .. } = workspace.node(tabs).expect("item tabs remain present")
            else {
                unreachable!("item lookup must return tabs");
            };
            if insertion > items.len() {
                return (
                    WorkspaceCommand::Reorder {
                        source: workspace
                            .capture_item_source(ROOT, tabs, item)
                            .expect("fresh source"),
                        insertion_index: insertion,
                    },
                    None,
                );
            }
            reorder_item_command(workspace, tabs, item, insertion)
        }
        ModelAction::CenterMove { item, target_item } => {
            center_item_command(workspace, tabs_for(item), item, tabs_for(target_item))
        }
        ModelAction::EdgeMove {
            item,
            target_item,
            edge,
            fraction,
        } => edge_item_command(
            workspace,
            tabs_for(item),
            item,
            tabs_for(target_item),
            edge,
            fraction,
        ),
        ModelAction::Open { item, target_item } => {
            open_item_command(workspace, item, tabs_for(target_item))
        }
        ModelAction::ResizeContaining { item } => {
            assert!(expected.can_resize(item));
            let tabs = tabs_for(item);
            let (split, child_count) = workspace
                .nodes()
                .find_map(|(node, value)| match value {
                    Node::Split { children, .. } if children.contains(&tabs) => {
                        Some((node, children.len()))
                    }
                    Node::Split { .. } | Node::Tabs { .. } => None,
                })
                .expect("multiple modeled groups require a containing split");
            resize_split_command(workspace, split, child_count)
        }
        ModelAction::Visibility { item, visible } => {
            return (
                WorkspaceCommand::SetItemVisibility {
                    source: workspace
                        .capture_item_source(ROOT, tabs_for(item), item)
                        .expect("fresh visibility source"),
                    visible,
                },
                None,
            );
        }
    };
    (resolved.0, Some(resolved.1))
}

fn apply_action(action: ModelAction, workspace: &mut Workspace, expected: &mut Semantics) -> Step {
    let Some(prediction) = expected.predict(action) else {
        expected.assert_workspace(workspace, action);
        return Step::Skipped;
    };
    expected.assert_workspace(workspace, action);
    let (command, delta) = resolve_action(action, workspace, expected);
    let before = workspace.clone();
    let result = WorkspaceTransaction::from_commands([command])
        .apply(workspace, &DockPolicySnapshot::default());
    let step = match prediction {
        Prediction::Rejected(rejection) => {
            let error = result.expect_err("the independently predicted rejection must occur");
            assert!(
                rejection.matches(&error),
                "wrong rejection for {action:?}: {error:?}"
            );
            assert_eq!(*workspace, before, "rejected action: {action:?}");
            Step::Rejected
        }
        Prediction::Success { next, changed } => {
            let report = result
                .unwrap_or_else(|error| panic!("model-valid {action:?} was rejected: {error:?}"));
            assert_eq!(
                report.changed(),
                changed,
                "expected state change for {action:?}"
            );
            assert_eq!(report.changed(), before != *workspace, "{action:?}");
            if let Some(delta) = delta {
                assert_success_postcondition(&delta, &report, &before, workspace);
            }
            *expected = next;
            Step::Applied { changed }
        }
    };
    workspace
        .validate()
        .expect("every published action must remain strict");
    expected.assert_workspace(workspace, action);
    step
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
        ..ProptestConfig::with_cases(128)
    })]

    #[test]
    fn shrinkable_command_sequences_match_independent_semantics(
        actions in prop::collection::vec(action_strategy(), 1..65),
    ) {
        let mut workspace = initial_workspace();
        let mut expected = Semantics::initial();
        let mut coverage = Coverage::default();
        let requested = actions.len();
        for action in actions {
            coverage.record(apply_action(action, &mut workspace, &mut expected));
        }
        prop_assert_eq!(coverage.skipped + coverage.rejected + coverage.applied, requested);
        prop_assert!(coverage.changed <= coverage.applied);
    }

    #[test]
    fn constructive_sequences_require_legal_center_moves(
        prefix in prop::collection::vec(action_strategy(), 0..32),
    ) {
        let mut workspace = initial_workspace();
        let mut expected = Semantics::initial();
        let center = ModelAction::CenterMove { item: ItemId::new(3), target_item: ItemId::new(1) };
        prop_assert_eq!(apply_action(center, &mut workspace, &mut expected), Step::Applied { changed: true });
        for action in prefix {
            apply_action(action, &mut workspace, &mut expected);
        }
        prop_assert!(matches!(apply_action(center, &mut workspace, &mut expected), Step::Applied { .. }), "the always-owned source and destination must remain operable");
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
    let initial_members = Semantics::initial();
    let mut complete = initial.clone();
    let mut complete_members = initial_members.clone();
    assert_eq!(
        apply_action(open, &mut complete, &mut complete_members),
        Step::Applied { changed: true }
    );
    assert_eq!(
        apply_action(move_to_opened, &mut complete, &mut complete_members),
        Step::Applied { changed: true }
    );
    assert_ne!(
        item_tabs(&complete, ItemId::new(3)),
        item_tabs(&initial, ItemId::new(3))
    );

    let mut shortened = initial.clone();
    let mut shortened_members = initial_members.clone();
    assert_eq!(
        apply_action(move_to_opened, &mut shortened, &mut shortened_members),
        Step::Skipped
    );
    assert_eq!(shortened, initial);
    assert_eq!(shortened_members, initial_members);
}

#[test]
fn inapplicable_sequences_are_skipped_without_fabricating_success() {
    let mut workspace = initial_workspace();
    let before = workspace.clone();
    let mut expected = Semantics::initial();
    for action in [
        ModelAction::Select {
            item: ItemId::new(9),
        },
        ModelAction::CenterMove {
            item: ItemId::new(3),
            target_item: ItemId::new(9),
        },
    ] {
        assert_eq!(
            apply_action(action, &mut workspace, &mut expected),
            Step::Skipped
        );
    }
    assert_eq!(workspace, before);
}

#[test]
fn visibility_rejection_and_recovery_preserve_order_and_local_history() {
    let mut workspace = initial_workspace();
    let mut expected = Semantics::initial();
    let hidden = ItemId::new(1);
    for visible in [false, true] {
        assert_eq!(
            apply_action(
                ModelAction::Visibility {
                    item: hidden,
                    visible
                },
                &mut workspace,
                &mut expected
            ),
            Step::Applied { changed: true }
        );
        assert_eq!(
            apply_action(
                ModelAction::Select { item: hidden },
                &mut workspace,
                &mut expected
            ),
            if visible {
                Step::Applied { changed: true }
            } else {
                Step::Rejected
            }
        );
    }
    assert_eq!(
        apply_action(
            ModelAction::Open {
                item: hidden,
                target_item: ItemId::new(3)
            },
            &mut workspace,
            &mut expected
        ),
        Step::Rejected
    );
    assert_eq!(
        apply_action(
            ModelAction::Reorder {
                item: hidden,
                insertion: 3
            },
            &mut workspace,
            &mut expected
        ),
        Step::Rejected
    );
}
