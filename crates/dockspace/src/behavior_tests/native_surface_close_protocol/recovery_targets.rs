//! Native close approval must preserve every surviving child's recovery host.

use super::*;

use dockspace::engine::surface_recovery::{ConvertedMainRecovery, SurfaceRecoveryTarget};
use dockspace::geometry::LogicalSize;

fn with_recovering_child() -> (Harness, ViewportBinding, ViewportBinding) {
    let mut fixture = Harness::new(rehome_workspace());
    let host = fixture.register_source_viewport();
    let target = SurfaceRecoveryTarget::with_converted_main(
        fixture
            .engine
            .root_recovery_anchor(SOURCE)
            .expect("the registered host must own its recovery anchor"),
        ConvertedMainRecovery::new(
            TARGET_ROOT,
            REHOMED_MAIN,
            LogicalSize::new(0.0, 0.0).unwrap(),
        ),
    );
    let provider = fixture.presentation_host.platform_provider();
    let expected = fixture.engine.version();
    let registered = fixture.submit(EngineInput::RegisterViewport {
        provider,
        expected,
        surface: TARGET,
        token: TARGET_TOKEN,
        role: ViewportRole::Child,
        recovery_target: Some(target),
    });
    let child = match only_outcome(&registered) {
        InputOutcome::ViewportRegistered { binding } => *binding,
        outcome => panic!("the child must register with the live host anchor: {outcome:?}"),
    };
    (fixture, host, child)
}

#[test]
fn closing_a_native_recovery_host_is_cancelled_before_accept() {
    let (mut fixture, host, child) = with_recovering_child();
    let before = fixture.engine.workspace().clone();
    let anchor = fixture.engine.root_recovery_anchor(SOURCE);
    let edge = fixture
        .open_native_close_edge_with_windows(host, vec![live_window(host), live_window(child)]);
    let (plan, mut approved) = fixture.begin_surface_close(edge, SurfaceCloseRequest::CloseContent);
    assert_eq!(plan.items().len(), 3);
    for item in plan.items() {
        approved = fixture.submit(EngineInput::ResolveClose {
            request: plan.request(),
            token: item.token(),
            decision: CloseDecision::Allow,
        });
    }

    assert!(
        !approved.platform_effects().iter().any(|effect| matches!(
            effect.effect(),
            PlatformEffect::ResolveNativeClose {
                edge: actual,
                resolution: NativeCloseResolution::Accept,
                ..
            } if *actual == edge
        )),
        "all Allow votes cannot authorize destruction of a surviving child's recovery host"
    );
    assert_eq!(fixture.engine.workspace(), &before);
    assert_eq!(fixture.engine.root_recovery_anchor(SOURCE), anchor);
    assert_eq!(
        fixture
            .engine
            .close_plan(plan.request())
            .map(ClosePlan::phase),
        Some(ClosePlanPhase::CancelRequested),
        "an unpublishable close candidate must retain its exact cancellation obligation"
    );
    for binding in [host, child] {
        assert_eq!(
            fixture
                .engine
                .viewport()
                .viewport(binding.surface())
                .map(|record| record.binding()),
            Some(binding)
        );
    }

    let cancelling = fixture.submit(EngineInput::ValidateWorkspace);
    let cancellation = native_close_effect(&cancelling, host, NativeCloseResolution::Cancel);
    fixture.publish_close_with_windows(
        host,
        3,
        WindowCloseState::LiveClear,
        Some(cancellation),
        vec![live_window(host), live_window(child)],
    );
    assert_eq!(
        fixture
            .engine
            .close_plan(plan.request())
            .map(ClosePlan::phase),
        Some(ClosePlanPhase::Cancelled),
        "only the exact cancellation acknowledgement settles the rejected close"
    );
    assert_eq!(fixture.engine.workspace(), &before);
    assert_eq!(fixture.engine.root_recovery_anchor(SOURCE), anchor);
    assert!(
        fixture
            .engine
            .viewport()
            .viewport(SOURCE)
            .unwrap()
            .is_routeable()
    );
    assert!(
        fixture
            .engine
            .viewport()
            .viewport(TARGET)
            .unwrap()
            .is_routeable()
    );

    let expected = fixture.engine.version();
    let selected = fixture.submit(EngineInput::SelectItem {
        expected,
        item: ItemId::new(2),
    });
    assert!(matches!(
        only_outcome(&selected),
        InputOutcome::ProductActionProcessed {
            outcome: dockspace::model::DockspaceActionOutcome::ItemActivated {
                item,
                changed: true,
            },
            ..
        } if *item == ItemId::new(2)
    ));
    assert_eq!(
        fixture.engine.workspace().item_multiset(),
        before.item_multiset()
    );
    assert!(selected.platform_effects().is_empty());
}

#[test]
fn closing_the_child_preserves_its_host_and_applies_after_exact_destruction() {
    let (mut fixture, host, child) = with_recovering_child();
    let before = fixture.engine.workspace().clone();
    let anchor = fixture.engine.root_recovery_anchor(SOURCE);
    let edge = fixture
        .open_native_close_edge_with_windows(child, vec![live_window(host), live_window(child)]);
    let (plan, accepted) = fixture.request_surface_close(edge, SurfaceCloseRequest::CloseContent);
    assert_eq!(fixture.engine.workspace(), &before);
    assert_eq!(
        fixture
            .engine
            .close_plan(plan.request())
            .map(ClosePlan::phase),
        Some(ClosePlanPhase::EffectEmitted)
    );

    fixture.publish_close_with_windows(
        child,
        3,
        WindowCloseState::Destroyed,
        Some(accepted),
        vec![live_window(host)],
    );
    let workspace = fixture.engine.workspace();
    assert!(workspace.surface(TARGET).is_none());
    assert!(workspace.root(TARGET_ROOT).is_none());
    assert_eq!(workspace.surface(SOURCE), before.surface(SOURCE));
    assert_eq!(workspace.root(SOURCE_ROOT), before.root(SOURCE_ROOT));
    assert_eq!(
        workspace.root(SOURCE_CONTAINED_ROOT),
        before.root(SOURCE_CONTAINED_ROOT)
    );
    assert_eq!(fixture.engine.root_recovery_anchor(SOURCE), anchor);
    assert_eq!(
        fixture
            .engine
            .close_plan(plan.request())
            .map(ClosePlan::phase),
        Some(ClosePlanPhase::Applied)
    );
    assert!(fixture.engine.viewport().viewport(TARGET).is_none());
    assert_eq!(fixture.engine.surface_recovery_target(TARGET), None);
    assert_eq!(
        fixture
            .engine
            .viewport()
            .viewport(SOURCE)
            .map(|record| record.binding()),
        Some(host)
    );
}

#[test]
fn an_exact_retained_child_close_allows_the_same_surface_to_register_again() {
    let (mut fixture, host, child) = with_recovering_child();
    let before = fixture.engine.workspace().clone();
    let target = fixture.engine.surface_recovery_target(TARGET).unwrap();
    let anchor = fixture.engine.root_recovery_anchor(SOURCE);
    let edge = fixture
        .open_native_close_edge_with_windows(child, vec![live_window(host), live_window(child)]);
    let (plan, accepted) = fixture.request_surface_close(edge, SurfaceCloseRequest::RetainLayout);

    fixture.publish_close_with_windows(
        child,
        3,
        WindowCloseState::Destroyed,
        Some(accepted),
        vec![live_window(host)],
    );
    assert_eq!(fixture.engine.workspace(), &before);
    assert_eq!(
        fixture
            .engine
            .close_plan(plan.request())
            .map(ClosePlan::phase),
        Some(ClosePlanPhase::Applied)
    );
    assert!(fixture.engine.viewport().viewport(TARGET).is_none());
    assert!(fixture.engine.viewport().recovery_pending(TARGET).is_none());
    assert!(
        fixture
            .engine
            .viewport()
            .binding_retirements()
            .all(|(binding, _)| binding != child),
        "exact destruction must finish the retired child's physical cleanup"
    );

    let expected = fixture.engine.version();
    let provider = fixture.presentation_host.platform_provider();
    let registered = fixture.submit(EngineInput::RegisterViewport {
        provider,
        expected,
        surface: TARGET,
        token: WindowToken::new(102),
        role: ViewportRole::Child,
        recovery_target: Some(target),
    });
    let replacement = match only_outcome(&registered) {
        InputOutcome::ViewportRegistered { binding } => *binding,
        outcome => panic!(
            "an exactly completed RetainLayout close must release the old recovery obligation: {outcome:?}"
        ),
    };
    assert_ne!(replacement.window(), child.window());
    assert_eq!(replacement.surface(), child.surface());
    assert_eq!(fixture.engine.surface_recovery_target(TARGET), Some(target));
    assert_eq!(fixture.engine.root_recovery_anchor(SOURCE), anchor);
    assert_eq!(fixture.engine.workspace(), &before);

    fixture.publish_close_with_windows(
        replacement,
        1,
        WindowCloseState::LiveClear,
        None,
        vec![live_window(host), live_window(replacement)],
    );
    assert!(
        fixture
            .engine
            .viewport()
            .viewport(TARGET)
            .unwrap()
            .is_routeable(),
        "the replacement must acquire its own fresh live authority"
    );
}

#[test]
fn unplanned_child_destruction_retains_its_unfulfilled_recovery_obligation() {
    let (mut fixture, host, child) = with_recovering_child();
    let before = fixture.engine.workspace().clone();
    let target = fixture.engine.surface_recovery_target(TARGET).unwrap();
    fixture.publish_close_with_windows(
        child,
        1,
        WindowCloseState::LiveClear,
        None,
        vec![live_window(host), live_window(child)],
    );
    fixture.publish_close_with_windows(
        child,
        2,
        WindowCloseState::Destroyed,
        None,
        vec![live_window(host)],
    );

    assert_eq!(fixture.engine.workspace(), &before);
    assert_eq!(fixture.engine.surface_recovery_target(TARGET), Some(target));
    assert_eq!(
        fixture
            .engine
            .viewport()
            .recovery_pending(TARGET)
            .map(|pending| pending.source_binding()),
        Some(child),
        "unplanned destruction still owes recovery while no host scene has been presented"
    );
}
