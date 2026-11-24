use super::*;
use crate::graph::{Node, RootRecord, SurfacePresentation, Workspace};
use crate::interaction::targeting::targets::{DropTargetRecord, DropVisual, SurfaceBackground};
use crate::policy::DockPolicy;
use crate::presentation::hit::region::HitRegion;
use crate::presentation::scene::PaneSceneId;
use crate::runtime::{DockspaceSession, UniformSurfaceMetrics};

fn measured_surface_session() -> (DockspaceSession, SurfaceId) {
    let surface = SurfaceId::new(1);
    let root = RootId::new(2);
    let item = ItemId::new(3);
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([item]));
    builder.set_root(root, RootRecord::new(tabs).with_central(tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    let mut session = DockspaceSession::from_workspace_for_test(
        builder.build().expect("the test workspace validates"),
        DockPolicy::default(),
    )
    .expect("the runtime initializes");
    let metrics = UniformSurfaceMetrics::new(
        LogicalRect::new(0.0, 0.0, 640.0, 480.0).expect("test bounds validate"),
        LogicalSize::new(32.0, 24.0).expect("test minimum validates"),
        80.0,
    )
    .expect("test metrics validate");
    let mut measurement_frame = session
        .begin_host_frame()
        .expect("the measurement frame begins");
    measurement_frame
        .measure_surface(surface, metrics)
        .expect("the surface measurement stages");
    measurement_frame
        .commit()
        .expect("the surface measurement commits");
    (session, surface)
}

fn assert_preflight_matches_complete_resolution(plan: SurfacePaintPlan<'_>) {
    let menu_open_bars = plan.program_menu_open_bars();
    let fully_resolvable = plan
        .plan
        .visit_surface_program(|step| {
            plan.resolve_surface_program_step(step, &menu_open_bars, None)
                .map(|_| ())
        })
        .is_ok();
    assert_eq!(plan.validate_surface_program(), fully_resolvable);
    assert_eq!(plan.program_valid, fully_resolvable);
    let mut expected_resources = Vec::new();
    let full_visit = plan.visit_surface_program(|step| {
        match step {
            SurfacePaintProgramStep::RootBegin { root, .. } => {
                if let crate::graph::TabLabel::Item(item) = root.label() {
                    expected_resources.push((
                        DockspaceVisualId::from_pane_scene(PaneSceneId {
                            root: root.root(),
                            item,
                        }),
                        TabKey::for_item(item),
                    ));
                }
            }
            SurfacePaintProgramStep::Operation {
                operation: SurfacePaintProgramOperation::Tab(tab),
                ..
            } => expected_resources.push((tab.visual_id(), tab.key())),
            SurfacePaintProgramStep::PopupMenu { menu, .. } => {
                expected_resources.extend(menu.rows().map(|row| (row.tab_visual_id(), row.key())));
            }
            SurfacePaintProgramStep::DragDecoration(decoration) => {
                expected_resources.push((decoration.source_visual(), decoration.key()));
            }
            SurfacePaintProgramStep::DropTarget(target) => {
                let descriptor = target.receiver();
                assert_eq!(Some(descriptor), plan.receiver(descriptor.region.kind()));
            }
            _ => {}
        }
        Ok::<(), ()>(())
    });
    let mut actual_resources = Vec::new();
    let resource_visit = plan.visit_tab_resources(|visual, item| {
        actual_resources.push((visual, item));
    });
    assert_eq!(full_visit.is_ok(), resource_visit.is_ok());
    assert_eq!(actual_resources, expected_resources);
}

#[test]
fn linked_target_receivers_match_identity_resolution_without_paint_time_searches() {
    use crate::interaction::targeting::resolver::structural_work;

    let (mut session, surface) = measured_surface_session();
    for width in [640.0, 800.0, 640.0] {
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .measure_surface(
                surface,
                UniformSurfaceMetrics::new(
                    LogicalRect::new(0.0, 0.0, width, 480.0).unwrap(),
                    LogicalSize::new(32.0, 24.0).unwrap(),
                    80.0,
                )
                .unwrap(),
            )
            .unwrap();
        let plan = frame.paint_plan(surface).unwrap().unwrap();
        assert!(plan.program_link.is_some());
        let collect = |plan: SurfacePaintPlan<'_>| {
            let mut receivers = Vec::new();
            plan.visit_surface_program(|step| {
                if let SurfacePaintProgramStep::DropTarget(target) = step {
                    receivers.push(target.receiver());
                }
                Ok::<(), ()>(())
            })
            .unwrap();
            receivers
        };
        structural_work::reset();
        let linked = collect(plan);
        assert!(!linked.is_empty());
        assert_eq!(
            structural_work::snapshot().presentation_hit_lookup_comparisons,
            0,
            "executing the installed link must not search target identities"
        );
        let resolved = collect(SurfacePaintPlan {
            program_link: None,
            ..plan
        });
        assert_eq!(linked, resolved);
        assert!(structural_work::snapshot().presentation_hit_lookup_comparisons > 0);
        frame.commit().unwrap();
    }
}

#[test]
fn only_execution_mints_a_receipt_bound_to_its_exact_host_frame() {
    let (mut session, surface) = measured_surface_session();

    let mut inspection_frame = session
        .begin_host_frame()
        .expect("the inspection frame begins");
    let plan = inspection_frame
        .paint_plan(surface)
        .expect("the inspection plan resolves")
        .expect("the measured surface is paintable");
    assert_preflight_matches_complete_resolution(plan);
    let inspected: () = plan
        .visit_surface_program(|_| Ok::<(), ()>(()))
        .expect("inspection traverses the complete program without minting authority");
    assert_eq!(inspected, ());
    let rejected = plan.execute_surface_program(|_| Err::<(), _>("sink rejected"));
    assert!(matches!(
        rejected,
        Err(SurfacePaintProgramVisitError::Sink("sink rejected"))
    ));
    let receipt = plan
        .execute_surface_program(|_| Ok::<(), ()>(()))
        .expect("the renderer accepts the complete program");
    drop(inspection_frame);

    let mut successor = session
        .begin_host_frame()
        .expect("the successor frame begins");
    let error = successor
        .confirm_surface_painted(receipt)
        .expect_err("a receipt from an aborted host frame must fail closed");
    assert!(matches!(
        error.interaction_error(),
        Some(DockspaceInteractionError::SurfaceProgramExecutionMismatch {
            surface: rejected
        }) if *rejected == surface
    ));
}

#[test]
fn invalid_surface_program_emits_no_paint_steps() {
    let surface = SurfaceId::new(1);
    let root = RootId::new(2);
    let item = ItemId::new(3);
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([item]));
    builder.set_root(root, RootRecord::new(tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    let mut session = DockspaceSession::from_workspace_for_test(
        builder.build().expect("the test workspace validates"),
        DockPolicy::default(),
    )
    .expect("the runtime initializes");
    let metrics = UniformSurfaceMetrics::new(
        LogicalRect::new(0.0, 0.0, 640.0, 480.0).expect("test bounds validate"),
        LogicalSize::new(32.0, 24.0).expect("test minimum validates"),
        80.0,
    )
    .expect("test metrics validate");
    let mut measurement_frame = session
        .begin_host_frame()
        .expect("the measurement frame begins");
    measurement_frame
        .measure_surface(surface, metrics)
        .expect("the surface measurement stages");
    measurement_frame
        .commit()
        .expect("the surface measurement commits");

    let mut paint_frame = session.begin_host_frame().expect("the paint frame begins");
    let valid = paint_frame
        .paint_plan(surface)
        .expect("the paint plan lookup succeeds")
        .expect("the measured surface is paintable");
    assert!(
        valid.program_valid,
        "ready plans must preflight their program"
    );
    assert_preflight_matches_complete_resolution(valid);
    let mut malformed = PresentationPlan::new(surface, valid.bounds());
    malformed
        .begin_surface_program_root(root, SurfaceProgramRootPresentation::Main)
        .expect("the malformed program root opens");
    malformed
        .push_surface_program_root_operation(SurfaceProgramRootOperation::Pane(PaneSceneId {
            root,
            item,
        }))
        .expect("the missing record identity enters the program");
    malformed
        .end_surface_program_root(root)
        .expect("the malformed program root closes");
    malformed
        .finish_surface_program()
        .expect("the malformed program seals structurally");
    assert!(matches!(
        malformed.link_surface_program(),
        Err(crate::presentation::scene::SceneBuildError::InvalidSurfaceProgram { .. })
    ));
    let malformed = SurfacePaintPlan {
        plan: &malformed,
        program_valid: false,
        ..valid
    };
    let malformed = SurfacePaintPlan {
        program_valid: malformed.validate_surface_program(),
        ..malformed
    };
    assert_preflight_matches_complete_resolution(malformed);

    assert!(
        !malformed.program_valid,
        "the preflight must reject a program referencing a missing pane record"
    );

    let mut emitted_steps = 0;
    let result = malformed.visit_surface_program(|_| {
        emitted_steps += 1;
        Ok::<(), ()>(())
    });

    assert!(matches!(
        result,
        Err(SurfacePaintProgramVisitError::InvalidProgram {
            surface: rejected
        }) if rejected == surface
    ));
    assert_eq!(
        emitted_steps, 0,
        "an invalid program must fail before the renderer can paint a partial output"
    );
}

#[test]
fn drop_target_without_receiver_invalidates_the_whole_surface_program() {
    let surface = SurfaceId::new(1);
    let root = RootId::new(2);
    let item = ItemId::new(3);
    let mut builder = Workspace::builder();
    let tabs = builder.insert_node(Node::tabs([item]));
    builder.set_root(root, RootRecord::new(tabs));
    builder.set_surface(surface, SurfacePresentation::with_main(root));
    let mut session = DockspaceSession::from_workspace_for_test(
        builder.build().expect("the test workspace validates"),
        DockPolicy::default(),
    )
    .expect("the runtime initializes");
    let bounds = LogicalRect::new(0.0, 0.0, 640.0, 480.0).expect("test bounds validate");
    let metrics = UniformSurfaceMetrics::new(
        bounds,
        LogicalSize::new(32.0, 24.0).expect("test minimum validates"),
        80.0,
    )
    .expect("test metrics validate");
    let mut measurement_frame = session
        .begin_host_frame()
        .expect("the measurement frame begins");
    measurement_frame
        .measure_surface(surface, metrics)
        .expect("the surface measurement stages");
    measurement_frame
        .commit()
        .expect("the surface measurement commits");

    let mut paint_frame = session.begin_host_frame().expect("the paint frame begins");
    let valid = paint_frame
        .paint_plan(surface)
        .expect("the paint plan lookup succeeds")
        .expect("the measured surface is paintable");
    assert_preflight_matches_complete_resolution(valid);
    let mut malformed = PresentationPlan::new(surface, bounds);
    malformed
        .set_surface_background(DropTargetRecord::surface_background(
            SurfaceBackground::new(surface),
            HitRegion::new(bounds),
            DropVisual::new(bounds),
        ))
        .expect("the background target stages");
    malformed
        .finish_surface_program()
        .expect("the target program seals structurally");
    malformed
        .link_surface_program()
        .expect("the target row links before testing dynamic receiver validation");
    let malformed = SurfacePaintPlan {
        plan: &malformed,
        program_valid: false,
        ..valid
    };
    let malformed = SurfacePaintPlan {
        program_valid: malformed.validate_surface_program(),
        ..malformed
    };
    assert_preflight_matches_complete_resolution(malformed);

    assert!(
        !malformed.program_valid,
        "a program drop target without the exact receiver must fail preflight"
    );
    let mut emitted_steps = 0;
    let result = malformed.visit_surface_program(|_| {
        emitted_steps += 1;
        Ok::<(), ()>(())
    });

    assert!(matches!(
        result,
        Err(SurfacePaintProgramVisitError::InvalidProgram {
            surface: rejected
        }) if rejected == surface
    ));
    assert_eq!(
        emitted_steps, 0,
        "the missing receiver must not degrade into a partially painted program"
    );
}
