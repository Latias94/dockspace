use super::*;

#[test]
fn host_focus_facts_reject_hidden_targets_before_recording_focus_or_history() {
    let mut fixture = counter_fixture();
    publish_counter_scene(&mut fixture);
    let item = ItemId::new(1);
    let fact = capture_focus_fact(
        &fixture.engine,
        SOURCE_SURFACE,
        1,
        HostFocusState::Pane(item),
    );
    let expected = fixture.engine.version();
    let transition = submit_test_input(
        &mut fixture.engine,
        fact.host(),
        EngineInput::SetItemVisibility {
            expected,
            item,
            visible: false,
        },
    )
    .unwrap();
    assert!(matches!(
        transition.reduced_inputs()[0].outcome(),
        InputOutcome::ProductActionProcessed { .. }
    ));
    let before = fixture.engine.viewport_focus.panel_focus(SOURCE_SURFACE);
    let rejected = submit_focus_fact(&mut fixture.engine, fact);
    assert!(matches!(
        rejected.reduced_inputs()[0].outcome(),
        InputOutcome::PaneFocusObservationPublished {
            transition: PaneFocusObservationTransition::Rejected(
                PaneFocusObservationRejection::ItemUnavailable { item: rejected },
            ),
        } if *rejected == item
    ));
    assert_eq!(
        fixture.engine.viewport_focus.panel_focus(SOURCE_SURFACE),
        before
    );
    assert_eq!(
        fixture
            .engine
            .viewport_focus
            .current_pane_focus(SOURCE_SURFACE),
        None
    );
}
