use super::*;

fn fixture(policy: DockPolicy, selected: usize) -> TabStripReducerFixture {
    let mut fixture = tab_strip_reducer_fixture_with_selection(policy.clone(), selected);
    let config = DockPresentationConfig::builder()
        .tab_min_width(100.0)
        .tab_max_width(100.0)
        .build()
        .unwrap();
    fixture.engine = DockEngine::new_with_presentation_config(
        fixture.engine.workspace().clone(),
        policy,
        config,
    )
    .unwrap();
    fixture
}

fn set_visibility(
    fixture: &mut TabStripReducerFixture,
    host: PresentationHostLease,
    item: ItemId,
    visible: bool,
) {
    let expected = fixture.engine.version();
    let transition = submit_test_input(
        &mut fixture.engine,
        host,
        EngineInput::SetItemVisibility {
            expected,
            item,
            visible,
        },
    )
    .unwrap();
    assert!(matches!(transition.reduced_inputs()[0].outcome(),
        InputOutcome::ProductActionProcessed {
            outcome: crate::model::DockspaceActionOutcome::ItemVisibilityChanged {
                item: actual, visible: actual_visible, changed: true,
            }, ..
        } if *actual == item && *actual_visible == visible));
}

#[test]
fn open_menu_reconciles_hidden_members_and_focus_before_its_next_published_frame() {
    for hidden_index in [1, 3] {
        let mut fixture = fixture(DockPolicy::default(), 3);
        let surface = fixture.surface;
        let bar = fixture.bar;
        let host = fixture.engine.create_presentation_host().unwrap();
        publish_tab_strip_reducer_projection(&mut fixture, host, true);
        let opened = submit_prepared_interaction(&mut fixture.engine, host, |view| {
            view.prepare_tab_strip_control_activation(surface, TabStripControlId::TabListMenu(bar))
                .map(|prepared| EngineInput::ActivateTabStripControl { prepared })
        });
        let menu = match opened.reduced_inputs()[0].outcome() {
            InputOutcome::InteractionProcessed {
                outcome:
                    InteractionOutcome::TabStripControlActivated {
                        menu: Some(menu), ..
                    },
                ..
            } => *menu,
            outcome => panic!("menu must open before visibility changes: {outcome:?}"),
        };
        publish_tab_strip_reducer_projection(&mut fixture, host, true);
        let hidden = fixture.items[hidden_index];
        set_visibility(&mut fixture, host, hidden, false);
        let visible = fixture
            .items
            .into_iter()
            .filter(|item| *item != hidden)
            .map(crate::ids::TabKey::from)
            .collect::<Vec<_>>();
        let active = fixture
            .engine
            .presentation_authority
            .tab_strip_states
            .active_menu()
            .unwrap();
        assert_eq!(active.session(), menu);
        assert_eq!(active.tabs(), visible);
        assert!(visible.contains(&active.focus()));
        let focus = active.focus();
        publish_tab_strip_reducer_projection(&mut fixture, host, true);
        let plan = fixture
            .engine
            .interaction_projection(surface)
            .unwrap()
            .plan();
        let record = plan
            .tab_list_menu_records()
            .iter()
            .find(|record| record.session() == menu)
            .unwrap();
        assert_eq!(
            record
                .rows()
                .iter()
                .map(|row| row.tab().tab)
                .collect::<Vec<_>>(),
            visible
        );
        assert_eq!(
            record
                .rows()
                .iter()
                .find(|row| row.focused())
                .unwrap()
                .tab()
                .tab,
            focus
        );

        set_visibility(&mut fixture, host, hidden, true);
        publish_tab_strip_reducer_projection(&mut fixture, host, true);
        let active = fixture
            .engine
            .presentation_authority
            .tab_strip_states
            .active_menu()
            .unwrap();
        assert_eq!(active.session(), menu);
        assert_eq!(active.tabs(), fixture.items.map(crate::ids::TabKey::from));
        assert_eq!(
            active.focus(),
            focus,
            "showing a member must not steal menu focus"
        );
        let plan = fixture
            .engine
            .interaction_projection(surface)
            .unwrap()
            .plan();
        assert_eq!(
            plan.tab_list_menu_records()[0].rows().len(),
            fixture.items.len()
        );
        assert!(!fixture.engine.workspace().item_is_hidden(hidden));
    }
}

#[test]
fn deferred_tab_chrome_actions_use_the_visible_roster_across_real_publications() {
    let mut fixture = fixture(DockPolicy::default(), 0);
    let surface = fixture.surface;
    let bar = fixture.bar;
    let key = TabStripStateKey::new(surface, bar);
    let host = fixture.engine.create_presentation_host().unwrap();
    let hidden = fixture.items[1];
    set_visibility(&mut fixture, host, hidden, false);
    publish_tab_strip_reducer_projection(&mut fixture, host, true);
    let witness = fixture
        .engine
        .scene()
        .surface(surface)
        .unwrap()
        .ready()
        .unwrap()
        .candidate()
        .frozen_witness();
    assert!(witness.plan().tab_bar_records()[0].maximum_scroll_offset() > 0.0);
    let action = LocalSurfaceAction::TabChrome {
        scene: witness.stamp(),
        action: LocalTabChromeAction::ScrollStrip {
            key,
            record: witness.plan().tab_bar_records()[0].clone(),
            adjustment: TabScrollAdjustment::scroll_by(1_000_000.0).unwrap(),
        },
    };
    let expected = fixture.engine.version();
    let scrolled = submit_test_input(
        &mut fixture.engine,
        host,
        EngineInput::ApplyDeferredSurfaceAction {
            expected,
            witness,
            action,
        },
    )
    .unwrap();
    assert!(matches!(scrolled.reduced_inputs()[0].outcome(),
        InputOutcome::InteractionProcessed {
            outcome: InteractionOutcome::TabStripScrolled { offset, changed: true, .. }, ..
        } if *offset > 0.0));
    publish_tab_strip_reducer_projection(&mut fixture, host, true);
    let witness = fixture
        .engine
        .scene()
        .surface(surface)
        .unwrap()
        .ready()
        .unwrap()
        .candidate()
        .frozen_witness();
    let record = &witness.plan().tab_bar_records()[0];
    assert_eq!(record.scroll_offset(), record.maximum_scroll_offset());
    let action = LocalSurfaceAction::TabChrome {
        scene: witness.stamp(),
        action: LocalTabChromeAction::ActivateControl(frozen_control(
            witness.plan(),
            key,
            TabStripControlId::TabListMenu(bar),
        )),
    };
    let expected = fixture.engine.version();
    let opened = submit_test_input(
        &mut fixture.engine,
        host,
        EngineInput::ApplyDeferredSurfaceAction {
            expected,
            witness,
            action,
        },
    )
    .unwrap();
    let menu = match opened.reduced_inputs()[0].outcome() {
        InputOutcome::InteractionProcessed {
            outcome:
                InteractionOutcome::TabStripControlActivated {
                    menu: Some(menu), ..
                },
            ..
        } => *menu,
        outcome => {
            panic!("deferred menu activation must accept the current visible roster: {outcome:?}")
        }
    };
    publish_tab_strip_reducer_projection(&mut fixture, host, true);
    let witness = fixture
        .engine
        .scene()
        .surface(surface)
        .unwrap()
        .ready()
        .unwrap()
        .candidate()
        .frozen_witness();
    let action = LocalSurfaceAction::TabChrome {
        scene: witness.stamp(),
        action: LocalTabChromeAction::NavigateMenu {
            session: menu,
            revision: witness.plan().popup().revision(),
            record: witness.plan().tab_list_menu_records()[0].clone(),
            target: fixture.items[3].into(),
        },
    };
    let expected = fixture.engine.version();
    let navigated = submit_test_input(
        &mut fixture.engine,
        host,
        EngineInput::ApplyDeferredSurfaceAction {
            expected,
            witness,
            action,
        },
    )
    .unwrap();
    assert!(matches!(navigated.reduced_inputs()[0].outcome(),
        InputOutcome::InteractionProcessed {
            outcome: InteractionOutcome::TabListMenuFocusMoved { tab, changed: true, .. }, ..
        } if *tab == fixture.items[3].into()));
    publish_tab_strip_reducer_projection(&mut fixture, host, true);
    let active = fixture
        .engine
        .presentation_authority
        .tab_strip_states
        .active_menu()
        .unwrap();
    assert_eq!(
        active.tabs(),
        [fixture.items[0], fixture.items[2], fixture.items[3]].map(crate::ids::TabKey::from)
    );
    assert_eq!(
        fixture.engine.workspace().visible_tab_selection(bar.tabs),
        Some(fixture.items[0].into())
    );
    assert!(fixture.engine.workspace().item_is_hidden(hidden));
    assert!(
        matches!(fixture.engine.workspace().node(bar.tabs), Some(Node::Tabs { items, .. }) if items.as_slice() == fixture.items.map(crate::ids::TabKey::from))
    );
}

#[test]
fn auto_hidden_bar_reveals_when_its_other_logical_members_are_hidden() {
    let mut policy = DockPolicy::default();
    policy.set_tab_bar(crate::policy::TabBarPolicy::new(
        crate::policy::TabBarVisibility::AutoHideSingle,
        TabBarInteraction::Enabled,
    ));
    let mut fixture = fixture(policy, 0);
    let surface = fixture.surface;
    let bar = fixture.bar;
    let host = fixture.engine.create_presentation_host().unwrap();
    for item in fixture.items.into_iter().skip(1) {
        set_visibility(&mut fixture, host, item, false);
    }
    publish_tab_strip_reducer_projection(&mut fixture, host, false);
    let hidden = fixture
        .engine
        .interaction_projection(surface)
        .unwrap()
        .plan();
    assert!(hidden.tab_bar_records().is_empty());
    let control = TabStripControlId::RevealTabBar(bar);
    let region = compiled_control_region(&fixture, hidden, control);
    assert!(fixture.engine.presented_receiver_is_current(hidden, region));
    let revealed = submit_prepared_interaction(&mut fixture.engine, host, |view| {
        view.prepare_tab_strip_control_activation(surface, control)
            .map(|prepared| EngineInput::ActivateTabStripControl { prepared })
    });
    assert!(matches!(
        revealed.reduced_inputs()[0].outcome(),
        InputOutcome::InteractionProcessed {
            outcome: InteractionOutcome::TabStripControlActivated { changed: true, .. },
            ..
        }
    ));
    publish_tab_strip_reducer_projection(&mut fixture, host, false);
    let plan = fixture
        .engine
        .interaction_projection(surface)
        .unwrap()
        .plan();
    assert_eq!(plan.tab_bar_records().len(), 1);
    assert_eq!(
        plan.tab_bar_records()[0].members()[0].tab().tab,
        fixture.items[0].into()
    );
    assert_eq!(plan.tab_bar_records()[0].members().len(), 1);
    assert!(
        plan.tab_strip_control_records()
            .iter()
            .all(|record| record.id() != control)
    );
    assert!(
        matches!(fixture.engine.workspace().node(bar.tabs), Some(Node::Tabs { items, .. }) if items.as_slice() == fixture.items.map(crate::ids::TabKey::from))
    );
    assert_eq!(
        fixture
            .engine
            .workspace()
            .hidden_items()
            .collect::<Vec<_>>(),
        fixture.items[1..]
    );
}

#[test]
fn deferred_tab_close_rejects_hidden_receivers_after_a_new_projection() {
    for close_body in [false, true] {
        for hide_target in [false, true] {
            let mut fixture = fixture(DockPolicy::default(), 0);
            let surface = fixture.surface;
            let target_item = fixture.items[0];
            let hidden_item = fixture.items[usize::from(!hide_target)];
            let host = fixture.engine.create_presentation_host().unwrap();
            publish_tab_strip_reducer_projection(&mut fixture, host, true);
            let witness = fixture
                .engine
                .scene()
                .surface(surface)
                .unwrap()
                .ready()
                .unwrap()
                .candidate()
                .frozen_witness();
            let tab = *witness
                .plan()
                .tab_records()
                .iter()
                .find(|record| record.id().tab == target_item.into())
                .unwrap()
                .id();
            let (target, kind) = if close_body {
                (
                    CloseSceneTarget::TabBody(tab),
                    PresentationHitRegionKind::TabBody(tab),
                )
            } else {
                (
                    CloseSceneTarget::Tab(tab),
                    PresentationHitRegionKind::TabClose(tab),
                )
            };
            let receiver = witness.hit_manifest().region_for_kind(kind).unwrap().id();
            assert!(
                fixture
                    .engine
                    .presented_receiver_is_current(witness.plan(), receiver)
            );
            let expected = fixture.engine.version();
            let action = LocalSurfaceAction::Close {
                scene: witness.stamp(),
                target,
            };

            set_visibility(&mut fixture, host, hidden_item, false);
            publish_tab_strip_reducer_projection(&mut fixture, host, true);
            let current = fixture
                .engine
                .scene()
                .surface(surface)
                .unwrap()
                .ready()
                .unwrap()
                .candidate();
            assert_ne!(current.stamp().requirement(), witness.stamp().requirement());
            assert_eq!(fixture.engine.version().epoch(), expected.epoch());
            assert_eq!(
                current
                    .plan()
                    .tab_records()
                    .iter()
                    .any(|record| record.id().tab == target_item.into()),
                !hide_target
            );
            let before = fixture.engine.workspace().clone();
            let transition = submit_test_input(
                &mut fixture.engine,
                host,
                EngineInput::ApplyDeferredSurfaceAction {
                    expected,
                    witness,
                    action,
                },
            )
            .unwrap();
            assert_eq!(fixture.engine.workspace(), &before);
            let plan = if hide_target {
                assert!(matches!(
                    transition.reduced_inputs()[0].outcome(),
                    InputOutcome::StaleRejected { .. }
                ));
                assert_eq!(fixture.engine.close.active_plans().count(), 0);
                let expected = fixture.engine.version();
                let requested = submit_test_input(
                    &mut fixture.engine,
                    host,
                    EngineInput::RequestContentClose {
                        expected,
                        target: ContentCloseTarget::Item(target_item),
                    },
                )
                .unwrap();
                match requested.reduced_inputs()[0].outcome() {
                    InputOutcome::ContentCloseRequested { plan, .. } => plan.clone(),
                    outcome => panic!(
                        "an explicit application request may still close a hidden item: {outcome:?}"
                    ),
                }
            } else {
                match transition.reduced_inputs()[0].outcome() {
                    InputOutcome::InteractionProcessed {
                        outcome: InteractionOutcome::CloseRequested { plan, .. },
                        ..
                    } => plan.clone(),
                    outcome => panic!(
                        "hiding a sibling must preserve the visible close receiver: {outcome:?}"
                    ),
                }
            };
            assert_eq!(plan.items().len(), 1);
            assert_eq!(plan.items()[0].item(), target_item);
            let closed = submit_test_input(
                &mut fixture.engine,
                host,
                EngineInput::ResolveClose {
                    request: plan.request(),
                    token: plan.items()[0].token(),
                    decision: CloseDecision::Allow,
                },
            )
            .unwrap();
            assert!(matches!(closed.reduced_inputs()[0].outcome(),
                InputOutcome::CloseDecisionProcessed {
                    application: Some(Ok(CloseCommitOutcome::ItemClosed { item, .. })),
                    changed: true, ..
                } if *item == target_item));
            assert!(
                fixture
                    .engine
                    .workspace()
                    .locate_item_by_id(target_item)
                    .is_none()
            );
            assert!(!fixture.engine.workspace().item_is_hidden(target_item));
        }
    }
}
