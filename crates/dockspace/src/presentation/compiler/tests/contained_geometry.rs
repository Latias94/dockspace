use super::*;
use proptest::prelude::*;

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const FLOATING: FloatingPresentationId = FloatingPresentationId::new(1);

fn contained_plan(bounds: LogicalRect) -> PresentationPlan {
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([ItemId::new(1)]));
    builder.set_root(ROOT, RootRecord::new(tabs));
    builder.set_surface(SURFACE, SurfacePresentation::rootless());
    builder.set_contained_floating(FLOATING, ContainedFloating::new(ROOT, bounds));
    builder.attach_contained(SURFACE, FLOATING).unwrap();
    let workspace = builder.build().unwrap();
    compile_default_ready_plan(
        &workspace,
        SURFACE,
        LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
    )
}

#[test]
fn fractional_contained_chrome_preserves_the_exact_inner_edges() {
    let bounds = LogicalRect::new(
        152.336_611_439_481_7,
        10.895_509_511_466_425,
        390.454_536_638_563_47,
        352.244_908_506_001_8,
    )
    .unwrap();
    let plan = contained_plan(bounds);
    let contained = plan.contained_record(FLOATING).unwrap();
    assert_eq!(contained.outer_bounds(), bounds);
    assert_eq!(
        contained.content_bounds().max(),
        contained.inner_bounds().max()
    );
    assert_eq!(
        contained.title_bounds().min(),
        contained.inner_bounds().min()
    );
    assert_eq!(
        contained.title_bounds().max().y(),
        contained.content_bounds().y()
    );
}

#[test]
fn zero_width_title_drag_keeps_the_exact_title_bottom() {
    let bounds = LogicalRect::new(
        0.0,
        2.500_354_948_395_537_4,
        27.407_292_323_815_3,
        28.592_092_695_424_59,
    )
    .unwrap();
    let plan = contained_plan(bounds);
    let contained = plan.contained_record(FLOATING).unwrap();
    let drag = contained.title_drag_hit().rect();
    assert_eq!(drag.width(), 0.0);
    assert_eq!(drag.max().y(), contained.title_bounds().max().y());
}

#[test]
fn narrow_fractional_contained_bar_keeps_its_exact_viewport_edges() {
    let bounds = LogicalRect::new(
        3.500_635_177_675_262_4,
        0.0,
        25.761_005_312_236_758,
        448.450_760_973_025_3,
    )
    .unwrap();
    let plan = contained_plan(bounds);
    let contained = plan.contained_record(FLOATING).unwrap();
    let [bar] = plan.tab_bar_records() else {
        panic!("the contained Tabs root must retain its real header")
    };
    assert_eq!(bar.bounds().min(), contained.content_bounds().min());
    assert_eq!(bar.bounds().max().x(), contained.content_bounds().max().x());
    assert_eq!(bar.viewport(), bar.bounds());
    assert!(bar.group_grip_bounds().is_none());
    assert!(bar.group_menu_bounds().is_none());
    assert!(plan.tab_strip_control_records().is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn fractional_and_thin_contained_roots_keep_a_valid_chrome_partition(
        x in 0.0_f64..200.0,
        y in 0.0_f64..100.0,
        width in prop_oneof![1.0_f64..40.0, 40.0_f64..500.0],
        height in prop_oneof![1.0_f64..30.0, 30.0_f64..450.0],
    ) {
        let bounds = LogicalRect::new(x, y, width, height).unwrap();
        let plan = contained_plan(bounds);
        let contained = plan.contained_record(FLOATING).unwrap();
        prop_assert_eq!(contained.outer_bounds(), bounds);
        prop_assert_eq!(contained.title_bounds().min(), contained.inner_bounds().min());
        prop_assert_eq!(contained.content_bounds().max(), contained.inner_bounds().max());
        prop_assert_eq!(contained.title_bounds().max().y(), contained.content_bounds().y());
    }
}
