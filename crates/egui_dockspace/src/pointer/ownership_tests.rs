//! Shared output tables and packet-scoped framework ownership joins.

use std::cell::Cell;
use std::sync::Arc;

use dockspace::geometry::{LogicalRect, LogicalSize};
use dockspace::model::{
    DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, RootId,
    SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockspaceReceiverDescriptor, DockspaceSession, PaintedSurfaceOutput, SurfacePresentationResult,
    UniformSurfaceMetrics,
};
use egui::{Context, Event, Id, LayerId, Pos2, RawInput, Rect, Sense};

use super::{PointerScene, ProductPointer, ProductPointerInputSnapshot};
use crate::render::ProductReceiverBinding;

const SURFACE: SurfaceId = SurfaceId::new(1);
const ITEM: ItemId = ItemId::new(1);

thread_local! {
    static JOIN_WORK: Cell<(usize, usize, usize)> = const { Cell::new((0, 0, 0)) };
}

pub(super) fn record_join(bindings: usize, rectangles: usize) {
    JOIN_WORK.with(|work| {
        let (joins, previous_bindings, previous_rectangles) = work.get();
        work.set((
            joins + 1,
            previous_bindings + bindings,
            previous_rectangles + rectangles,
        ));
    });
}

fn take_join_work() -> (usize, usize, usize) {
    JOIN_WORK.with(|work| work.replace((0, 0, 0)))
}

fn session() -> DockspaceSession {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), DockspaceNode::tabs([ITEM])),
    )])
    .unwrap();
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    session.enable_host_pointer().unwrap();
    session
}

fn paint(session: &mut DockspaceSession) -> (PaintedSurfaceOutput, DockspaceReceiverDescriptor) {
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface(
            SURFACE,
            UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
                LogicalSize::new(64.0, 48.0).unwrap(),
                80.0,
            )
            .unwrap(),
        )
        .unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let receiver = plan.tab_receiver(ITEM).unwrap();
    let receipt = plan.execute_surface_program(|_| Ok::<(), ()>(())).unwrap();
    frame.confirm_surface_painted(receipt).unwrap();
    let output = frame
        .commit()
        .unwrap()
        .take_painted_outputs()
        .pop()
        .unwrap();
    (output, receiver)
}

fn settle(session: &mut DockspaceSession) -> Vec<dockspace::runtime::SurfacePresentationCommit> {
    session
        .begin_host_frame()
        .unwrap()
        .commit()
        .unwrap()
        .surface_presentation_commits()
        .to_vec()
}

fn emit(
    pointer: &mut ProductPointer,
    output: &PaintedSurfaceOutput,
    receiver: DockspaceReceiverDescriptor,
    count: usize,
) {
    let layer = LayerId::new(egui::Order::Background, Id::unique("output-layer"));
    pointer.emitted(
        output.presentation_artifact(),
        layer,
        1,
        true,
        (0..count)
            .map(|index| ProductReceiverBinding {
                interact_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0)),
                interactive: true,
                widget_id: Id::unique(("owned-table", index)),
                layer_id: layer,
                receiver,
            })
            .collect(),
        vec![(ITEM, Id::unique("pane-focus"))],
    );
}

#[test]
fn pointer_drafts_share_output_tables_and_discard_only_private_changes() {
    for count in [1, 16, 128] {
        let mut session = session();
        let mut pointer = ProductPointer::default();
        let mut outputs = Vec::new();
        for _ in 0..4 {
            let (output, receiver) = paint(&mut session);
            emit(&mut pointer, &output, receiver, count);
            outputs.push(output);
        }
        pointer.position = Some(Pos2::new(12.0, 34.0));
        let mut draft = pointer.clone();
        for (original, copied) in pointer.pending.iter().zip(&draft.pending) {
            assert_eq!(original.receivers.len(), count);
            assert!(Arc::ptr_eq(&original.receivers, &copied.receivers));
            assert!(Arc::ptr_eq(
                &original.focus_bindings,
                &copied.focus_bindings
            ));
        }
        draft.position = Some(Pos2::new(56.0, 78.0));
        draft.pending.clear();
        drop(draft);
        assert_eq!(pointer.position, Some(Pos2::new(12.0, 34.0)));
        assert_eq!(pointer.pending.len(), outputs.len());
        assert!(
            pointer
                .pending
                .iter()
                .all(|scene| scene.receivers.len() == count)
        );
    }
}

#[test]
fn reversed_output_reports_keep_shared_tables_bound_to_the_exact_artifact() {
    let mut session = session();
    let mut pointer = ProductPointer::default();
    let mut outputs = Vec::new();
    for count in [1, 16, 128] {
        let (output, receiver) = paint(&mut session);
        emit(&mut pointer, &output, receiver, count);
        outputs.push(output);
    }
    let final_scene = pointer.pending.last().unwrap().clone();
    session
        .report_surface_presentation(outputs.pop().unwrap(), SurfacePresentationResult::Presented)
        .unwrap();
    pointer.observe(&settle(&mut session));
    assert!(pointer.presented.is_none());
    assert_eq!(pointer.pending.len(), 3);
    session
        .report_surface_presentation(outputs.pop().unwrap(), SurfacePresentationResult::Dropped)
        .unwrap();
    pointer.observe(&settle(&mut session));
    assert!(pointer.presented.is_none());
    assert_eq!(pointer.pending.len(), 3);
    session
        .report_surface_presentation(outputs.pop().unwrap(), SurfacePresentationResult::Presented)
        .unwrap();
    let commits = settle(&mut session);
    assert!(
        commits
            .iter()
            .any(|commit| commit.presents(final_scene.artifact))
    );

    // A draft can synchronize the committed presentation without consuming the
    // original's still-pending tables. Abandoning it preserves that original.
    let mut draft = pointer.clone();
    let frame = session.begin_host_frame().unwrap();
    draft.sync_presented(&frame, SURFACE);
    let presented = draft.presented.as_ref().unwrap();
    assert_eq!(presented.artifact, final_scene.artifact);
    assert!(Arc::ptr_eq(&presented.receivers, &final_scene.receivers));
    assert!(Arc::ptr_eq(
        &presented.focus_bindings,
        &final_scene.focus_bindings
    ));
    drop(frame);
    drop(draft);
    assert!(pointer.presented.is_none());
    assert_eq!(pointer.pending.len(), 3);

    pointer.observe(&commits);
    assert!(pointer.pending.is_empty());
    let presented = pointer.presented.as_ref().unwrap();
    assert_eq!(presented.artifact, final_scene.artifact);
    assert!(Arc::ptr_eq(&presented.receivers, &final_scene.receivers));
    assert!(Arc::ptr_eq(
        &presented.focus_bindings,
        &final_scene.focus_bindings
    ));
}

fn widgets(
    context: &Context,
    receiver: DockspaceReceiverDescriptor,
    count: usize,
) -> Vec<ProductReceiverBinding> {
    let mut bindings = Vec::new();
    for _ in 0..2 {
        bindings.clear();
        context
            .run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0))),
                    ..RawInput::default()
                },
                |ui| {
                    egui::Area::new(Id::unique("ownership-test-layer"))
                        .fixed_pos(Pos2::ZERO)
                        .movable(false)
                        // The area routes the layer; only content widgets are interactive.
                        .sense(Sense::hover())
                        .show(ui.ctx(), |ui| {
                            let bounds =
                                Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(100.0, 120.0));
                            ui.allocate_rect(bounds, Sense::hover());
                            for index in 0..count {
                                let response = ui.interact(
                                    bounds,
                                    Id::unique(("content", index)),
                                    Sense::click(),
                                );
                                bindings.push(ProductReceiverBinding {
                                    interact_rect: response.interact_rect,
                                    interactive: true,
                                    widget_id: response.id,
                                    layer_id: response.layer_id,
                                    receiver,
                                });
                            }
                        });
                },
            )
            .drop_without_applying_deltas();
    }
    bindings
}

#[test]
fn a_pointer_packet_joins_content_ownership_once_for_all_edges() {
    for count in [1, 16, 128] {
        for unknown in [0, 1] {
            let context = Context::default();
            context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
            let mut session = session();
            let (output, receiver) = paint(&mut session);
            let mut bindings = widgets(&context, receiver, count + unknown);
            let point = bindings[0].interact_rect.center();
            bindings.truncate(count);
            bindings.extend_from_within(..);
            let scene = PointerScene {
                artifact: output.presentation_artifact(),
                layer: bindings[0].layer_id,
                completed_pass: context.cumulative_pass_nr(),
                coordinates_supported: true,
                receivers: bindings.into(),
                focus_bindings: Arc::from([]),
            };
            session
                .report_surface_presentation(output, SurfacePresentationResult::Presented)
                .unwrap();
            settle(&mut session);
            let mut pointer = ProductPointer {
                presented: Some(scene),
                ..ProductPointer::default()
            };
            take_join_work();
            let output = crate::run_ui(
                &context,
                RawInput {
                    screen_rect: Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0))),
                    events: vec![Event::PointerMoved(point); 128],
                    ..RawInput::default()
                },
                |ui| {
                    let packet = super::pointer_packet(ui).unwrap();
                    let scene = pointer.presented.as_ref().unwrap();
                    let snapshot = packet.receivers();
                    assert_eq!(snapshot.completed_pass, scene.completed_pass);
                    assert_eq!((snapshot.layer_at)(point), Some(scene.layer));
                    assert_eq!(
                        snapshot
                            .widgets
                            .get_layer(scene.layer)
                            .filter(|widget| widget.enabled && widget.sense.interactive())
                            .count(),
                        count + unknown
                    );
                    egui::Area::new(Id::unique("ownership-test-layer"))
                        .fixed_pos(Pos2::ZERO)
                        .movable(false)
                        .sense(Sense::hover())
                        .show(ui.ctx(), |ui| {
                            let frame = session.begin_host_frame().unwrap();
                            let prepared = pointer
                                .prepare_packet(&frame, SURFACE, ui, &packet)
                                .unwrap();
                            assert_eq!(prepared.raw.len(), 128);
                            assert!(
                                prepared
                                    .raw
                                    .iter()
                                    .all(|edge| edge.blocked == (unknown == 1))
                            );
                        });
                },
            );
            let (output, receipt) = output.into_parts();
            output.drop_without_applying_deltas();
            drop(receipt);
            assert_eq!(take_join_work(), (1, count * 2, count + unknown));
        }
    }
}

#[test]
fn packet_join_keeps_equal_rectangles_distinct_without_repeating_per_point() {
    for count in [1, 16, 128] {
        for unknown in [0, 1] {
            let context = Context::default();
            context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
            let (output, receiver) = paint(&mut session());
            let mut bindings = widgets(&context, receiver, count + unknown);
            let bounds = bindings[0].interact_rect;
            bindings.truncate(count);
            // Repeated declarations must not consume the undeclared widget's
            // identical rectangle; only distinct live widget IDs get credit.
            bindings.extend_from_within(..);
            let scene = PointerScene {
                artifact: output.presentation_artifact(),
                layer: bindings[0].layer_id,
                completed_pass: context.cumulative_pass_nr(),
                coordinates_supported: true,
                receivers: bindings.into(),
                focus_bindings: Arc::from([]),
            };
            let snapshot = ProductPointerInputSnapshot::capture(&context);
            take_join_work();
            let ownership = snapshot.ownership(&scene);
            assert_eq!(
                ownership.widgets.iter().filter(|(_, owned)| !owned).count(),
                unknown
            );
            for _ in 0..128 {
                assert_eq!(ownership.is_clear_at(bounds.center()), unknown == 0);
                assert!(ownership.is_clear_at(Pos2::new(200.0, 200.0)));
            }
            assert_eq!(take_join_work(), (1, count * 2, count + unknown));
        }
    }
}

#[test]
fn ownership_join_keeps_unproven_or_noninteractive_widget_rectangles_blocked() {
    let context = Context::default();
    let (output, receiver) = paint(&mut session());
    let binding = widgets(&context, receiver, 1)[0];
    for changed in [
        ProductReceiverBinding {
            interactive: false,
            ..binding
        },
        ProductReceiverBinding {
            widget_id: Id::unique("not-registered"),
            ..binding
        },
        ProductReceiverBinding {
            layer_id: LayerId::new(egui::Order::Foreground, Id::unique("foreign")),
            ..binding
        },
        ProductReceiverBinding {
            interact_rect: binding.interact_rect.shrink(1.0),
            ..binding
        },
    ] {
        let scene = PointerScene {
            artifact: output.presentation_artifact(),
            layer: binding.layer_id,
            completed_pass: context.cumulative_pass_nr(),
            coordinates_supported: true,
            receivers: Arc::from([changed]),
            focus_bindings: Arc::from([]),
        };
        let snapshot = ProductPointerInputSnapshot::capture(&context);
        assert!(
            !snapshot
                .ownership(&scene)
                .is_clear_at(binding.interact_rect.center())
        );
    }
}

#[test]
fn ordered_ownership_requires_both_capture_lanes_to_shield_unknown_widgets() {
    for (foreign_sense, owned_sense, framework_clear, qualified) in [
        (Sense::CLICK, Sense::CLICK, true, false),
        (Sense::DRAG, Sense::CLICK, false, false),
        (Sense::click_and_drag(), Sense::CLICK, false, false),
        (Sense::CLICK, Sense::DRAG, true, false),
        (Sense::DRAG, Sense::DRAG, false, false),
        (Sense::CLICK, Sense::click_and_drag(), true, true),
        (Sense::DRAG, Sense::click_and_drag(), true, true),
        (Sense::click_and_drag(), Sense::click_and_drag(), true, true),
    ] {
        let context = Context::default();
        context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        let (output, receiver) = paint(&mut session());
        let bounds = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(100.0, 120.0));
        let mut binding = None;
        for _ in 0..4 {
            context
                .run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0))),
                        events: vec![Event::PointerMoved(bounds.center())],
                        ..RawInput::default()
                    },
                    |ui| {
                        egui::Area::new(Id::unique("ordered-ownership-layer"))
                            .fixed_pos(Pos2::ZERO)
                            .movable(false)
                            .sense(Sense::hover())
                            .show(ui.ctx(), |ui| {
                                ui.allocate_rect(bounds, Sense::hover());
                                ui.interact(
                                    bounds,
                                    Id::unique("foreign-background"),
                                    foreign_sense,
                                );
                                let response = ui.interact(
                                    bounds,
                                    Id::unique("owned-foreground"),
                                    owned_sense,
                                );
                                binding = Some(ProductReceiverBinding {
                                    interact_rect: response.interact_rect,
                                    interactive: true,
                                    widget_id: response.id,
                                    layer_id: response.layer_id,
                                    receiver,
                                });
                            });
                    },
                )
                .drop_without_applying_deltas();
        }
        let binding = binding.unwrap();
        let scene = PointerScene {
            artifact: output.presentation_artifact(),
            layer: binding.layer_id,
            completed_pass: context.cumulative_pass_nr(),
            coordinates_supported: true,
            receivers: Arc::from([binding]),
            focus_bindings: Arc::from([]),
        };
        let actual_clear = context.viewport(|viewport| {
            assert!(
                viewport.hits.click.is_some() || viewport.hits.drag.is_some(),
                "the real framework must hit one of the overlapping widgets"
            );
            [viewport.hits.click, viewport.hits.drag]
                .into_iter()
                .flatten()
                .all(|hit| hit.id == binding.widget_id)
        });
        assert_eq!(
            actual_clear, framework_clear,
            "the fixture must agree with real egui routing: foreign={foreign_sense:?}, owned={owned_sense:?}"
        );
        assert_eq!(
            ProductPointerInputSnapshot::capture(&context)
                .ownership(&scene)
                .is_clear_at(bounds.center()),
            qualified,
            "foreign={foreign_sense:?}, owned={owned_sense:?}"
        );
    }
}

#[test]
fn nearby_undeclared_widget_blocks_a_single_lane_owned_drag() {
    for owned_sense in [Sense::click_and_drag(), Sense::DRAG] {
        for unknown_sense in [Sense::DRAG, Sense::click_and_drag()] {
            for unknown_before in [true, false] {
                let context = Context::default();
                context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
                let (output, receiver) = paint(&mut session());
                let bounds = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(100.0, 120.0));
                let nearby = Rect::from_min_max(Pos2::new(54.0, 72.0), Pos2::new(56.0, 74.0));
                let point = Pos2::new(55.0, 70.0);
                let owned_id = Id::unique("nearby-owned-control");
                let unknown_id = Id::unique("nearby-undeclared-control");
                assert!(bounds.contains(point));
                assert!(bounds.contains_rect(nearby));
                assert!(!nearby.contains(point));
                assert!(
                    nearby.distance_sq_to_pos(point)
                        < context.global_style().interaction.interact_radius.powi(2)
                );

                let mut binding = None;
                for _ in 0..4 {
                    context
                        .run_ui(
                            RawInput {
                                screen_rect: Some(Rect::from_min_max(
                                    Pos2::ZERO,
                                    Pos2::new(800.0, 600.0),
                                )),
                                events: vec![Event::PointerMoved(point)],
                                ..RawInput::default()
                            },
                            |ui| {
                                egui::Area::new(Id::unique("nearby-ownership-layer"))
                                    .fixed_pos(Pos2::ZERO)
                                    .movable(false)
                                    .sense(Sense::hover())
                                    .show(ui.ctx(), |ui| {
                                        ui.allocate_rect(bounds, Sense::hover());
                                        if unknown_before {
                                            ui.interact(nearby, unknown_id, unknown_sense);
                                        }
                                        let response = ui.interact(bounds, owned_id, owned_sense);
                                        binding = Some(ProductReceiverBinding {
                                            interact_rect: response.interact_rect,
                                            interactive: true,
                                            widget_id: response.id,
                                            layer_id: response.layer_id,
                                            receiver,
                                        });
                                        if !unknown_before {
                                            ui.interact(nearby, unknown_id, unknown_sense);
                                        }
                                    });
                            },
                        )
                        .drop_without_applying_deltas();
                }
                let binding = binding.unwrap();
                let both_lanes_owned = owned_sense.senses_click();
                context.viewport(|viewport| {
                    assert_eq!(
                        viewport.hits.drag.map(|hit| hit.id),
                        Some(if both_lanes_owned { owned_id } else { unknown_id }),
                        "real egui routing must distinguish direct dual-lane ownership from nearby drag assistance: owned={owned_sense:?}, unknown={unknown_sense:?}, unknown_before={unknown_before}",
                    );
                    assert_eq!(
                        viewport.hits.click.map(|hit| hit.id),
                        if both_lanes_owned {
                            Some(owned_id)
                        } else {
                            unknown_sense.senses_click().then_some(unknown_id)
                        },
                    );
                });
                let scene = PointerScene {
                    artifact: output.presentation_artifact(),
                    layer: binding.layer_id,
                    completed_pass: context.cumulative_pass_nr(),
                    coordinates_supported: true,
                    receivers: Arc::from([binding]),
                    focus_bindings: Arc::from([]),
                };
                let snapshot = ProductPointerInputSnapshot::capture(&context);
                assert_eq!((snapshot.layer_at)(point), Some(scene.layer));
                assert_eq!(
                    snapshot.ownership(&scene).is_clear_at(point),
                    both_lanes_owned,
                    "nearby undeclared capture must block a single-lane owner without obstructing a direct dual-lane owner: owned={owned_sense:?}, unknown={unknown_sense:?}, unknown_before={unknown_before}",
                );
            }
        }
    }
}

#[test]
fn scaled_foreign_layer_nearby_widget_blocks_a_single_lane_owned_drag() {
    let context = Context::default();
    context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let (output, receiver) = paint(&mut session());
    let bounds = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(100.0, 120.0));
    let nearby = Rect::from_min_max(Pos2::new(27.0, 38.0), Pos2::new(28.0, 39.0));
    let point = Pos2::new(55.0, 70.0);
    let owned_id = Id::unique("scaled-nearby-owned-control");
    let unknown_id = Id::unique("scaled-nearby-undeclared-control");
    let foreign_layer = LayerId::new(
        egui::Order::Foreground,
        Id::unique("scaled-foreign-layer-without-area"),
    );
    let transform = egui::emath::TSTransform::from_scaling(2.0);
    context.set_transform_layer(foreign_layer, transform);
    let global_nearby = transform * nearby;
    let radius_sq = context.global_style().interaction.interact_radius.powi(2);
    assert!(bounds.contains(point));
    assert!(bounds.contains_rect(global_nearby));
    assert!(global_nearby.distance_sq_to_pos(point) > radius_sq);
    assert!(nearby.distance_sq_to_pos(transform.inverse() * point) < radius_sq);

    let mut binding = None;
    for _ in 0..4 {
        context
            .run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0))),
                    events: vec![Event::PointerMoved(point)],
                    ..RawInput::default()
                },
                |ui| {
                    egui::Area::new(Id::unique("scaled-nearby-ownership-layer"))
                        .fixed_pos(Pos2::ZERO)
                        .movable(false)
                        .sense(Sense::hover())
                        .show(ui.ctx(), |ui| {
                            ui.allocate_rect(bounds, Sense::hover());
                            let response = ui.interact(bounds, owned_id, Sense::DRAG);
                            binding = Some(ProductReceiverBinding {
                                interact_rect: response.interact_rect,
                                interactive: true,
                                widget_id: response.id,
                                layer_id: response.layer_id,
                                receiver,
                            });
                        });
                    ui.scope_builder(egui::UiBuilder::new().layer_id(foreign_layer), |ui| {
                        ui.interact(nearby, unknown_id, Sense::DRAG);
                    });
                },
            )
            .drop_without_applying_deltas();
    }
    context.viewport(|viewport| {
        assert_eq!(
            viewport.hits.drag.map(|hit| hit.id),
            Some(unknown_id),
            "egui's local-space search radius must admit the scaled foreign drag control",
        );
        assert!(viewport.hits.click.is_none());
    });
    let binding = binding.unwrap();
    let scene = PointerScene {
        artifact: output.presentation_artifact(),
        layer: binding.layer_id,
        completed_pass: context.cumulative_pass_nr(),
        coordinates_supported: true,
        receivers: Arc::from([binding]),
        focus_bindings: Arc::from([]),
    };
    let snapshot = ProductPointerInputSnapshot::capture(&context);
    assert_eq!((snapshot.layer_at)(point), Some(scene.layer));
    assert!(
        !snapshot.ownership(&scene).is_clear_at(point),
        "foreign drag assistance must block ownership even beyond the unscaled global radius",
    );
}
