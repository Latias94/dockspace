//! Shared L08 visibility semantics through the public core-host contract.

use super::*;
use crate::scenario::{
    ItemAlias, JourneyId, LOCAL_PANE_VISIBILITY, ScenarioAdmission, ScenarioAxis,
    ScenarioCapability, ScenarioId, SemanticAction, SemanticObservation, SemanticSelector,
};

fn item(alias: ItemAlias) -> ItemId {
    match alias {
        ItemAlias::Primary => A,
        ItemAlias::Secondary => B,
        ItemAlias::Contained => X,
    }
}

fn alias(item: ItemId) -> ItemAlias {
    match item {
        A => ItemAlias::Primary,
        B => ItemAlias::Secondary,
        X => ItemAlias::Contained,
        other => panic!("the visibility scenario observed an unnamed item: {other:?}"),
    }
}

fn weights(view: DockspaceView<'_>) -> Vec<f64> {
    view.root(ROOT)
        .unwrap()
        .content()
        .unwrap()
        .split()
        .unwrap()
        .weights()
        .collect()
}

fn publish_visible_surface(host: &mut DeterministicHost) {
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    let report = host.run(|frame| {
        let view = frame.view();
        let visible = [A, B, C, X]
            .into_iter()
            .filter(|item| view.item(*item).unwrap().is_visible())
            .collect::<Vec<_>>();
        let selected = [A, C, X]
            .into_iter()
            .filter_map(|item| view.item(item).unwrap().tabs().visible_selected())
            .collect::<Vec<_>>();
        let contained_visible = view.item(X).unwrap().is_visible();
        let mut minimum_items = Vec::new();
        let mut intrinsic_items = Vec::new();
        frame
            .measure_surface_with(SURFACE, |request| {
                Ok::<_, dockspace::runtime::DockspaceRuntimeError>(match request {
                    SurfaceMeasurementRequest::DockBounds { .. }
                    | SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                        SurfaceMeasurementAnswer::Bounds(bounds)
                    }
                    SurfaceMeasurementRequest::PaneMinimum { item, .. } => {
                        let item = item.expect("hidden groups do not become empty logical leaves");
                        assert!(visible.contains(&item), "hidden content needs no metadata");
                        minimum_items.push(item);
                        SurfaceMeasurementAnswer::PaneMinimum(LogicalSize::new(32.0, 24.0).unwrap())
                    }
                    SurfaceMeasurementRequest::TabIntrinsic { item, .. } => {
                        assert!(visible.contains(&item), "hidden tabs need no metadata");
                        intrinsic_items.push(item);
                        SurfaceMeasurementAnswer::TabIntrinsic(72.0)
                    }
                    SurfaceMeasurementRequest::TabStrip { .. } => {
                        SurfaceMeasurementAnswer::TabStrip(TabStripMetrics::new(0.0, 0.0).unwrap())
                    }
                })
            })
            .unwrap();
        minimum_items.sort_unstable();
        intrinsic_items.sort_unstable();
        assert_eq!(minimum_items, visible);
        assert_eq!(intrinsic_items, visible);

        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let records = program_records(plan);
        let mut painted_items = records
            .tabs
            .iter()
            .map(|tab| tab.item())
            .collect::<Vec<_>>();
        painted_items.sort_unstable();
        assert_eq!(painted_items, visible);
        let mut painted_selection = records
            .panes
            .iter()
            .map(|pane| {
                pane.selected()
                    .expect("an entirely hidden group has no pane")
            })
            .collect::<Vec<_>>();
        painted_selection.sort_unstable();
        let mut selected = selected;
        selected.sort_unstable();
        assert_eq!(painted_selection, selected);
        for item in [A, B, C, X] {
            assert_eq!(plan.tab_receiver(item).is_some(), visible.contains(&item));
            assert_eq!(
                plan.prepare_tab_select(item).is_some(),
                visible.contains(&item),
                "only visible content exposes a current tab action"
            );
        }
        assert_eq!(records.contained.len(), usize::from(contained_visible));
        if visible.iter().all(|item| *item != A && *item != B) {
            assert!(records.splitters.is_empty());
            let remaining = records
                .panes
                .iter()
                .find(|pane| pane.selected() == Some(C))
                .unwrap();
            assert_eq!(remaining.bounds(), bounds);
        }
        frame.confirm_surface_program(SURFACE).unwrap();
    });
    host.observe_painted_outputs(report);
}

#[test]
fn shared_l08_pane_visibility_preserves_durable_layout_and_current_public_controls() {
    let supported = [
        ScenarioCapability::LocalTabSelection,
        ScenarioCapability::LocalItemVisibility,
        ScenarioCapability::LocalSplitterAdjustment,
        ScenarioCapability::FullContainedProjection,
    ];
    assert_eq!(LOCAL_PANE_VISIBILITY.id(), ScenarioId::LocalPaneVisibility);
    assert_eq!(LOCAL_PANE_VISIBILITY.journey(), JourneyId::L08);
    assert_eq!(
        LOCAL_PANE_VISIBILITY.admission(&supported),
        ScenarioAdmission::Ready
    );
    assert_eq!(
        LOCAL_PANE_VISIBILITY.admission(&[ScenarioCapability::LocalTabSelection]),
        ScenarioAdmission::Unsupported {
            missing: ScenarioCapability::LocalItemVisibility,
        }
    );
    let mut host = DeterministicHost::new(shared_semantic_layout());
    let original_rect = host.view().contained(FLOATING).unwrap().rect();
    publish_visible_surface(&mut host);
    let expected_selection = [B, A, A, B, B, B, B, B];
    let mut observations = Vec::new();
    for (index, step) in LOCAL_PANE_VISIBILITY.steps().iter().enumerate() {
        let before_weights = weights(host.view());
        let observed = match (step.selector(), step.action()) {
            (SemanticSelector::Tab(target), SemanticAction::Activate) => {
                let report = host.run(|frame| {
                    frame.select_item_current(item(target)).unwrap();
                });
                let [
                    HostInputOutcome::ProductActionApplied(DockspaceActionOutcome::Selected {
                        item: selected,
                        changed: true,
                    }),
                ] = report.inputs()
                else {
                    panic!("selection must commit through the public action: {report:?}");
                };
                let selected_view = host.view().item(*selected).unwrap();
                assert!(selected_view.is_selected());
                SemanticObservation::ItemSelected(alias(selected_view.item()))
            }
            (SemanticSelector::Item(target), SemanticAction::SetVisible(visible)) => {
                let report = host.run(|frame| {
                    frame
                        .set_item_visibility_current(item(target), visible)
                        .unwrap();
                });
                let [
                    HostInputOutcome::ProductActionApplied(
                        DockspaceActionOutcome::ItemVisibilityChanged {
                            item: changed,
                            visible: committed_visible,
                            changed: true,
                        },
                    ),
                ] = report.inputs()
                else {
                    panic!("visibility must commit through its public action: {report:?}");
                };
                let current = host.view().item(*changed).unwrap();
                assert_eq!(current.is_visible(), *committed_visible);
                assert_eq!(weights(host.view()), before_weights);
                SemanticObservation::ItemVisibilityChanged {
                    item: alias(current.item()),
                    visible: current.is_visible(),
                }
            }
            (SemanticSelector::Splitter(ScenarioAxis::Horizontal), SemanticAction::Increment) => {
                host.run(|frame| {
                    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
                    let splitter = program_records(plan).splitters[0];
                    let action = plan
                        .prepare_splitter_adjustment(
                            splitter.visual_id(),
                            SurfaceSplitterAdjustment::Increment,
                        )
                        .unwrap();
                    frame.submit_surface_action(action).unwrap();
                });
                let after = weights(host.view());
                assert!(after[0] > before_weights[0]);
                assert!(after[1] < before_weights[1]);
                let axis = host
                    .view()
                    .root(ROOT)
                    .unwrap()
                    .content()
                    .unwrap()
                    .split()
                    .unwrap()
                    .axis();
                SemanticObservation::SplitterAdjusted(match axis {
                    DockspaceAxis::Horizontal => ScenarioAxis::Horizontal,
                    DockspaceAxis::Vertical => ScenarioAxis::Vertical,
                })
            }
            unsupported => panic!("unsupported visibility scenario step: {unsupported:?}"),
        };
        observations.push(observed);
        let tabs = host.view().item(A).unwrap().tabs();
        assert_eq!(tabs.items(), [A, B]);
        assert_eq!(tabs.selected(), Some(expected_selection[index]));
        assert_eq!(
            tabs.visible_items().collect::<Vec<_>>(),
            [A, B]
                .into_iter()
                .filter(|item| host.view().item(*item).unwrap().is_visible())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            tabs.visible_selected(),
            (index != 2).then_some(expected_selection[index])
        );
        assert_eq!(host.view().item(C).unwrap().tabs().items(), [C]);
        assert_eq!(host.view().item(X).unwrap().tabs().items(), [X]);
        assert_eq!(
            host.view().contained(FLOATING).unwrap().rect(),
            original_rect
        );
        assert!(host.session.pending_content_close_plans().next().is_none());
        assert!(!host.session.has_active_gesture());
        publish_visible_surface(&mut host);
    }
    assert_eq!(observations, LOCAL_PANE_VISIBILITY.expected_observations());
    assert!(host.view().hidden_items().next().is_none());
    host.run(|frame| {
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let action = plan.prepare_tab_select(A).unwrap();
        frame.submit_surface_action(action).unwrap();
    });
    assert!(host.view().item(A).unwrap().is_selected());
    publish_visible_surface(&mut host);
}
