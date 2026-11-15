use super::*;
use proptest::prelude::*;

fn overflowing_plan(bounds: LogicalRect, controls: TabStripControlMetrics) -> PresentationPlan {
    let policy = policy_snapshot();
    let (workspace, manifest, measurements, states, _, _, _) =
        overflowing_tab_strip_fixture_with_geometry(&policy, None, false, controls, bounds, bounds);
    let plan = compile_surface_measurements(
        &workspace,
        version(),
        &policy,
        &DockPresentationConfig::default(),
        &manifest,
        &measurements,
        &states,
        &[],
        None,
    )
    .expect("fractional overflow geometry should compile");
    PresentationPlanValidator::new(&workspace, &policy)
        .unwrap()
        .validate_and_canonicalize(plan)
        .expect("overflow controls must share exact chrome boundaries")
}

#[test]
fn fractional_overflow_control_shares_the_presentation_menu_edge() {
    let bounds =
        LogicalRect::new(15.813_516_505_747_248, 20.0, 187.497_371_322_747_74, 180.0).unwrap();
    let controls = TabStripControlMetrics::new(0.0)
        .unwrap()
        .with_tab_list_menu(
            TabStripControlMetric::new(20.0, TabStripControlPlacement::ReservedTrailing).unwrap(),
        );
    let plan = overflowing_plan(bounds, controls);
    let [control] = plan.tab_strip_control_records() else {
        panic!("the overflowing strip retains its menu control")
    };
    let bar = &plan.tab_bar_records()[0];
    let anchor = plan.presentation_menu_anchor_records()[0]
        .ready_bounds()
        .unwrap();
    assert_eq!(bar.viewport().max().x(), control.bounds().x());
    assert_eq!(control.bounds().max().x(), anchor.x());
}

#[test]
fn touching_overlay_controls_share_an_ordered_boundary() {
    let content = LogicalRect::new(5.669_495_304_401_262, 20.0, 36.0, 24.0).unwrap();
    let controls = TabStripControlMetrics::new(0.0)
        .unwrap()
        .with_scroll_backward(
            TabStripControlMetric::new(18.0, TabStripControlPlacement::OverlayLeading).unwrap(),
        )
        .with_scroll_forward(
            TabStripControlMetric::new(18.0, TabStripControlPlacement::OverlayTrailing).unwrap(),
        );
    let layout = tab_strip_control_layout(content, controls)
        .unwrap()
        .unwrap();
    let leading = layout.rects[0].unwrap();
    let trailing = layout.rects[1].unwrap();
    assert_eq!(layout.viewport, content);
    assert_eq!(leading.min(), content.min());
    assert_eq!(trailing.max(), content.max());
    assert_eq!(leading.max().x(), trailing.x());
}

fn placement(index: usize) -> TabStripControlPlacement {
    [
        TabStripControlPlacement::ReservedLeading,
        TabStripControlPlacement::ReservedTrailing,
        TabStripControlPlacement::OverlayLeading,
        TabStripControlPlacement::OverlayTrailing,
    ][index]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn fractional_overflow_chrome_accepts_all_control_placements(
        x in 0.0_f64..200.0,
        y in 0.0_f64..100.0,
        width in 80.0_f64..500.0,
        placements in prop::array::uniform3(0_usize..4),
        spacing in prop_oneof![Just(0.0), 0.0_f64..8.0],
    ) {
        let bounds = LogicalRect::new(x, y, width, 180.0).unwrap();
        let controls = TabStripControlMetrics::new(spacing).unwrap()
            .with_scroll_backward(TabStripControlMetric::new(18.0, placement(placements[0])).unwrap())
            .with_scroll_forward(TabStripControlMetric::new(18.0, placement(placements[1])).unwrap())
            .with_tab_list_menu(TabStripControlMetric::new(20.0, placement(placements[2])).unwrap());
        let plan = overflowing_plan(bounds, controls);
        prop_assert!(plan.tab_bar_records()[0].viewport().max().x() <= bounds.max().x());
    }
}
