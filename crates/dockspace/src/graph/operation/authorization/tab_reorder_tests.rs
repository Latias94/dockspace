use super::*;
use crate::graph::SurfacePresentation;
use crate::policy::{DockItemRule, DockPolicy, PolicyRejection, PolicyRevision};

const ROOT: RootId = RootId::new(1);
const OTHER_ROOT: RootId = RootId::new(2);
const BARRIER: ItemId = ItemId::new(3);

fn fixture() -> (Workspace, DockPolicySnapshot, NodeId, NodeId) {
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs((1..=5).map(ItemId::new)));
    let other = builder.insert_node(Node::tabs([ItemId::new(6)]));
    builder.set_root(ROOT, RootRecord::new(tabs));
    builder.set_root(OTHER_ROOT, RootRecord::new(other));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    builder.set_surface(
        SurfaceId::new(2),
        SurfacePresentation::with_main(OTHER_ROOT),
    );
    let mut policy = DockPolicy::default();
    let mut rule = DockItemRule::new();
    rule.set_allow_tab_reorder(false);
    policy.set_item_rule(BARRIER, rule);
    (
        builder.build().unwrap(),
        policy.snapshot(PolicyRevision::default()),
        tabs,
        other,
    )
}

#[test]
fn locked_tabs_and_crossing_moves_share_one_rule_for_commands_and_drop_eligibility() {
    let (workspace, policy, tabs, _) = fixture();
    let original = workspace.clone();
    for from in 0..5 {
        for insertion_index in 0..=5 {
            let item = ItemId::new(u64::try_from(from + 1).unwrap());
            let source = workspace.capture_item_source(ROOT, tabs, item).unwrap();
            let target = DockTarget::TabGap {
                target: workspace.capture_tab_target(ROOT, tabs).unwrap(),
                index: insertion_index,
            };
            let payload = MovePayload::Item(source.clone());
            // A tab stays on its original side of the locked tab; the locked
            // tab itself permits only the two gaps which keep its index.
            let allowed = match from {
                0 | 1 => insertion_index <= 2,
                2 => (2..=3).contains(&insertion_index),
                _ => insertion_index >= 3,
            };
            let eligibility = DropCommandEligibility::new_validated(&workspace, &policy, &payload)
                .unwrap()
                .check_validated_target(&target);
            assert_eq!(
                eligibility.is_ok(),
                allowed,
                "from={from}, gap={insertion_index}"
            );
            for command in [
                WorkspaceCommand::Reorder {
                    source: source
                        .as_tab_source()
                        .expect("fixture owns an inline item tab"),
                    insertion_index,
                },
                WorkspaceCommand::Move { payload, target },
            ] {
                let result = prepare_transaction(&workspace, &policy, &[command]);
                assert_eq!(
                    result.is_ok(),
                    allowed,
                    "from={from}, gap={insertion_index}"
                );
                if !allowed {
                    assert!(matches!(
                        result.unwrap_err(),
                        TransactionError::Command {
                            source: CommandError::Policy(PolicyRejection::TabReorderDisabled {
                                item: BARRIER
                            }),
                            ..
                        }
                    ));
                }
            }
        }
    }
    assert_eq!(
        workspace, original,
        "preparing or rejecting a command never publishes it"
    );
}

#[test]
fn reorder_constraints_allow_undocking_and_new_items_before_the_barrier() {
    let (workspace, policy, tabs, other) = fixture();
    let moved = WorkspaceCommand::Move {
        payload: MovePayload::Item(workspace.capture_item_source(ROOT, tabs, BARRIER).unwrap()),
        target: DockTarget::Center(workspace.capture_tab_target(OTHER_ROOT, other).unwrap()),
    };
    let (candidate, report) = prepare_transaction(&workspace, &policy, &[moved])
        .unwrap()
        .into_parts();
    assert!(report.changed());
    assert_eq!(
        candidate
            .capture_item_source_by_id(BARRIER)
            .unwrap()
            .unwrap()
            .root(),
        OTHER_ROOT
    );

    let opened = WorkspaceCommand::Open {
        item: ItemId::new(7),
        target: DockTarget::TabGap {
            target: workspace.capture_tab_target(ROOT, tabs).unwrap(),
            index: 0,
        },
    };
    let (candidate, report) = prepare_transaction(&workspace, &policy, &[opened])
        .unwrap()
        .into_parts();
    assert!(report.changed());
    let Node::Tabs { items, .. } = candidate.node(tabs).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        items,
        &[7, 1, 2, 3, 4, 5].map(ItemId::new).map(TabKey::for_item)
    );
}

#[test]
fn unrelated_and_neutral_rules_do_not_restrict_the_current_tabs_group() {
    let (workspace, _, tabs, _) = fixture();
    let mut policy = DockPolicy::default();
    for id in 1..=4096 {
        policy.set_item_rule(ItemId::new(id), DockItemRule::new());
    }
    let mut barrier = DockItemRule::new();
    barrier.set_allow_tab_reorder(false);
    // This barrier is open in another group, rather than being a stale rule.
    policy.set_item_rule(ItemId::new(6), barrier);
    let policy = policy.snapshot(PolicyRevision::default());
    for from in 1..=5 {
        let source = workspace
            .capture_item_source(ROOT, tabs, ItemId::new(from))
            .unwrap();
        let payload = MovePayload::Item(source);
        let eligibility =
            DropCommandEligibility::new_validated(&workspace, &policy, &payload).unwrap();
        for index in 0..=5 {
            let target = DockTarget::TabGap {
                target: workspace.capture_tab_target(ROOT, tabs).unwrap(),
                index,
            };
            eligibility.check_validated_target(&target).unwrap();
            prepare_transaction(
                &workspace,
                &policy,
                &[WorkspaceCommand::Move {
                    payload: payload.clone(),
                    target,
                }],
            )
            .unwrap();
        }
    }
}

#[test]
fn frozen_barriers_match_command_rejections_for_every_source_gap_and_rule_subset() {
    let (workspace, _, tabs, _) = fixture();
    let target = workspace.capture_tab_target(ROOT, tabs).unwrap();
    for mask in 0..32_u32 {
        let mut policy = DockPolicy::default();
        for index in 0..5 {
            if mask & (1 << index) != 0 {
                let mut rule = DockItemRule::new();
                rule.set_allow_tab_reorder(false);
                policy.set_item_rule(ItemId::new(index + 1), rule);
            }
        }
        let policy = policy.snapshot(PolicyRevision::default());
        for item in (1..=5).map(ItemId::new) {
            let source = workspace.capture_item_source(ROOT, tabs, item).unwrap();
            let payload = MovePayload::Item(source.clone());
            let eligibility =
                DropCommandEligibility::new_validated(&workspace, &policy, &payload).unwrap();
            for index in 0..=5 {
                let preview = eligibility.check_validated_target(&DockTarget::TabGap {
                    target: target.clone(),
                    index,
                });
                let command = authorize_workspace_command(
                    &workspace,
                    &policy,
                    &WorkspaceCommand::Reorder {
                        source: source
                            .clone()
                            .as_tab_source()
                            .expect("fixture owns an inline item tab"),
                        insertion_index: index,
                    },
                );
                assert_eq!(preview, command, "mask={mask}, item={item:?}, gap={index}");
            }
        }
    }
}

fn mixed_fixture(central: bool) -> (Workspace, NodeId, NodeId) {
    let mut builder = Workspace::builder();
    let mut tools = Node::tabs([ItemId::new(2), ItemId::new(4)]);
    tools.set_layout_title(Some(Arc::from("Tools")));
    let layout = builder.insert_node(tools);
    let first = TabKey::for_item(ItemId::new(1));
    let tabs = builder.insert_node(Node::tabs_from_keys(
        vec![
            first,
            TabKey::for_layout(layout),
            TabKey::for_item(ItemId::new(3)),
        ],
        Some(first),
    ));
    let record = RootRecord::new(tabs);
    builder.set_root(
        ROOT,
        if central {
            record.with_central(layout)
        } else {
            record
        },
    );
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    (builder.build().unwrap(), tabs, layout)
}

fn retained_items_policy() -> DockPolicy {
    let mut policy = DockPolicy::default();
    for item in (1..=4).map(ItemId::new) {
        let mut rule = DockItemRule::new();
        rule.set_allow_undocking(false);
        policy.set_item_rule(item, rule);
    }
    policy
}

fn direct_payload(workspace: &Workspace, tabs: NodeId, tab: TabKey) -> MovePayload {
    match tab.item() {
        Some(item) => MovePayload::Item(workspace.capture_item_source(ROOT, tabs, item).unwrap()),
        None => MovePayload::Subtree(
            workspace
                .capture_node_source(ROOT, tab.layout().unwrap())
                .unwrap(),
        ),
    }
}

#[test]
fn mixed_direct_child_moves_use_original_gaps_without_undocking_or_replacing_layouts() {
    let cases = [
        (0, 0, [0, 1, 2]),
        (0, 1, [0, 1, 2]),
        (0, 2, [1, 0, 2]),
        (0, 3, [1, 2, 0]),
        (1, 0, [1, 0, 2]),
        (1, 1, [0, 1, 2]),
        (1, 2, [0, 1, 2]),
        (1, 3, [0, 2, 1]),
        (2, 0, [2, 0, 1]),
        (2, 1, [0, 2, 1]),
        (2, 2, [0, 1, 2]),
        (2, 3, [0, 1, 2]),
    ];
    for central in [false, true] {
        let (workspace, tabs, layout) = mixed_fixture(central);
        let original = workspace.clone();
        let keys = [
            TabKey::for_item(ItemId::new(1)),
            TabKey::for_layout(layout),
            TabKey::for_item(ItemId::new(3)),
        ];
        let policy = retained_items_policy().snapshot(PolicyRevision::default());
        let target = workspace.capture_tab_target(ROOT, tabs).unwrap();
        for (moving, gap, order) in cases {
            let expected = order.map(|index| keys[index]);
            let command = WorkspaceCommand::Move {
                payload: direct_payload(&workspace, tabs, keys[moving]),
                target: DockTarget::TabGap {
                    target: target.clone(),
                    index: gap,
                },
            };
            let (candidate, report) = prepare_transaction(&workspace, &policy, &[command])
                .unwrap()
                .into_parts();
            let Some(Node::Tabs { items, .. }) = candidate.node(tabs) else {
                unreachable!()
            };
            assert_eq!(
                items, &expected,
                "moving={moving}, gap={gap}, central={central}"
            );
            assert_eq!(report.changed(), expected != keys);
            assert_eq!(candidate.node(layout), workspace.node(layout));
            assert_eq!(candidate.root(ROOT), workspace.root(ROOT));
            assert_eq!(candidate.tab_mru(tabs), workspace.tab_mru(tabs));
            assert_eq!(candidate.item_multiset(), workspace.item_multiset());
            assert_eq!(candidate.nodes().count(), workspace.nodes().count());
            candidate.validate().unwrap();
        }
        for tab in keys {
            let command = WorkspaceCommand::Move {
                payload: direct_payload(&workspace, tabs, tab),
                target: DockTarget::Center(target.clone()),
            };
            let (candidate, report) = prepare_transaction(&workspace, &policy, &[command])
                .unwrap()
                .into_parts();
            assert!(!report.changed());
            assert_eq!(candidate, workspace);
        }
        assert_eq!(workspace, original);
    }
}

#[test]
fn mixed_child_barriers_agree_for_reorder_move_and_frozen_drop_eligibility() {
    let cases = [
        (1, 0, 1, false),
        (1, 3, 3, false),
        (0, 3, 2, false),
        (2, 0, 4, false),
        (1, 1, 2, true),
        (1, 2, 4, true),
        (0, 1, 1, true),
        (2, 2, 3, true),
        (1, 0, 3, true),
    ];
    for central in [false, true] {
        let (workspace, tabs, layout) = mixed_fixture(central);
        let original = workspace.clone();
        let keys = [
            TabKey::for_item(ItemId::new(1)),
            TabKey::for_layout(layout),
            TabKey::for_item(ItemId::new(3)),
        ];
        for (moving, gap, blocked, allowed) in cases {
            let blocked = ItemId::new(blocked);
            let mut policy = retained_items_policy();
            let mut rule = DockItemRule::new();
            rule.set_allow_undocking(false);
            rule.set_allow_tab_reorder(false);
            policy.set_item_rule(blocked, rule);
            let policy = policy.snapshot(PolicyRevision::default());
            let payload = direct_payload(&workspace, tabs, keys[moving]);
            let target = DockTarget::TabGap {
                target: workspace.capture_tab_target(ROOT, tabs).unwrap(),
                index: gap,
            };
            let expected = if allowed {
                Ok(())
            } else {
                Err(CommandError::Policy(PolicyRejection::TabReorderDisabled {
                    item: blocked,
                }))
            };
            let eligibility =
                DropCommandEligibility::new_validated(&workspace, &policy, &payload).unwrap();
            assert_eq!(
                eligibility.check_target_with(ReferenceAuthority::Live, &target),
                expected,
                "moving={moving}, gap={gap}, blocked={blocked}, central={central}"
            );
            let commands = [
                WorkspaceCommand::Reorder {
                    source: workspace
                        .capture_tab_source(ROOT, tabs, keys[moving])
                        .unwrap(),
                    insertion_index: gap,
                },
                WorkspaceCommand::Move { payload, target },
            ];
            for command in commands {
                assert_eq!(
                    authorize_workspace_command(&workspace, &policy, &command),
                    expected
                );
                match prepare_transaction(&workspace, &policy, &[command]) {
                    Ok(prepared) => {
                        assert!(allowed);
                        assert_eq!(
                            prepared.candidate().item_multiset(),
                            workspace.item_multiset()
                        );
                        prepared.candidate().validate().unwrap();
                    }
                    Err(TransactionError::Command { source, .. }) => {
                        assert_eq!(Err(source), expected);
                    }
                    Err(error) => panic!("unexpected transaction error: {error:?}"),
                }
            }
        }
        assert_eq!(workspace, original);
    }
}

#[test]
fn same_parent_layout_move_rejects_stale_scope_before_reordering() {
    let (workspace, tabs, layout) = mixed_fixture(true);
    let stale = direct_payload(&workspace, tabs, TabKey::for_layout(layout));
    let stale_target = workspace.capture_tab_target(ROOT, tabs).unwrap();
    let policy = retained_items_policy().snapshot(PolicyRevision::default());
    let command = WorkspaceCommand::Reorder {
        source: workspace
            .capture_tab_source(ROOT, tabs, TabKey::for_item(ItemId::new(1)))
            .unwrap(),
        insertion_index: 3,
    };
    let (candidate, _) = prepare_transaction(&workspace, &policy, &[command])
        .unwrap()
        .into_parts();
    let before = candidate.clone();
    for (payload, target, stale_role) in [
        (
            stale,
            candidate.capture_tab_target(ROOT, tabs).unwrap(),
            ReferenceRole::Source,
        ),
        (
            direct_payload(&candidate, tabs, TabKey::for_layout(layout)),
            stale_target,
            ReferenceRole::Target,
        ),
    ] {
        let command = WorkspaceCommand::Move {
            payload,
            target: DockTarget::TabGap { target, index: 3 },
        };
        assert!(matches!(
            prepare_transaction(&candidate, &policy, &[command]),
            Err(TransactionError::Command {
                source: CommandError::StaleNode { role, .. },
                ..
            }) if role == stale_role
        ));
    }
    assert_eq!(candidate, before);
}
