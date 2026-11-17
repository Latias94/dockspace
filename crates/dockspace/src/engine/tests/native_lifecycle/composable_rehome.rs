//! Native source retirement waits for the exact target output for every tab payload.

use super::*;

use crate::model::{DockAnchor, TabGroupDockMode};
use crate::platform::registry::{ViewportAdmission, ViewportOwnership};
use crate::platform::viewport::PresentationObservationGeneration;
use crate::presentation::observation::HostPresentationOutput;

mod pending_lifecycle;
mod same_surface;

const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);
const TARGET: ItemId = ItemId::new(3);
const HIDDEN: ItemId = ItemId::new(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceRemainder {
    None,
    HiddenSibling,
    CentralAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RehomeTarget {
    Tabs,
    Float,
    Main,
}

impl RehomeTarget {
    fn placement(self) -> DockPlacement {
        match self {
            Self::Tabs => DockPlacement::Center(DockAnchor::Item(TARGET)),
            Self::Main => DockPlacement::Main(TARGET_SURFACE),
            Self::Float => unreachable!("contained floating uses its own product action"),
        }
    }
}

fn native_tab_source(
    nested: bool,
    remainder: SourceRemainder,
) -> (BackgroundFixture, ViewportBinding) {
    let mut builder = Workspace::builder();
    let mut inner = Node::tabs(if nested {
        vec![FIRST, SECOND]
    } else {
        vec![FIRST]
    });
    if nested {
        inner.set_layout_title(Some(Arc::from("Tools")));
    }
    let source_tabs = builder.insert_node(inner);
    let source_node = if nested {
        let tab = TabKey::for_layout(source_tabs);
        let mut tabs = vec![tab];
        if remainder == SourceRemainder::HiddenSibling {
            tabs.push(TabKey::for_item(HIDDEN));
        }
        builder.insert_node(Node::tabs_from_keys(tabs, Some(tab)))
    } else {
        source_tabs
    };
    let target_tabs = builder.insert_node(Node::tabs([TARGET]));
    let floating = FloatingPresentationId::new(2);
    let source_root = if remainder == SourceRemainder::CentralAnchor {
        RootRecord::new(source_node).with_central(source_node)
    } else {
        RootRecord::new(source_node)
    };
    builder.set_root(SOURCE_ROOT, source_root);
    builder.set_root(TARGET_ROOT, RootRecord::new(target_tabs));
    builder.set_surface(SOURCE_SURFACE, SurfacePresentation::with_main(SOURCE_ROOT));
    builder.set_surface(TARGET_SURFACE, SurfacePresentation::rootless());
    builder.set_contained_floating(
        floating,
        ContainedFloating::new(TARGET_ROOT, background_contained_rect()),
    );
    builder
        .attach_contained(TARGET_SURFACE, floating)
        .expect("the surviving target root must have a presentation owner");
    let mut policy = DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    let mut engine = DockEngine::new(
        builder
            .build()
            .expect("the native source layout must be valid"),
        policy,
    )
    .expect("the native source engine must initialize");
    let presentation_host = engine
        .create_presentation_host()
        .expect("the existing lifecycle host must mint");
    let mut fixture = BackgroundFixture {
        engine,
        presentation_host,
        source_tabs,
    };
    if remainder == SourceRemainder::HiddenSibling {
        let expected = fixture.engine.version();
        let hidden = submit_test_input(
            &mut fixture.engine,
            presentation_host,
            EngineInput::SetItemVisibility {
                expected,
                item: HIDDEN,
                visible: false,
            },
        )
        .expect("the outer sibling must hide through the ordinary product input");
        assert!(matches!(
            hidden.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed { .. }
        ));
        assert!(fixture.engine.workspace().item_is_hidden(HIDDEN));
    }
    let provider = test_platform_provider(&mut fixture.engine);
    let expected = fixture.engine.version();
    let host = submit_test_input(
        &mut fixture.engine,
        presentation_host,
        EngineInput::RegisterViewport {
            provider,
            expected,
            surface: TARGET_SURFACE,
            token: WindowToken::new(2),
            role: ViewportRole::Root,
            recovery_target: None,
        },
    )
    .expect("the surviving native root must register");
    assert!(matches!(
        host.reduced_inputs()[0].outcome(),
        InputOutcome::ViewportRegistered { .. }
    ));
    let expected = fixture.engine.version();
    let child = submit_test_input(
        &mut fixture.engine,
        presentation_host,
        EngineInput::BootstrapChildViewport {
            provider,
            expected,
            surface: SOURCE_SURFACE,
            token: WindowToken::new(1),
            recovery: SurfaceRecoveryBootstrap::new(TARGET_SURFACE),
        },
    )
    .expect("the real child must register through the existing bootstrap input");
    assert!(matches!(
        child.reduced_inputs()[0].outcome(),
        InputOutcome::ViewportRegistered { .. }
    ));
    let source = background_source_binding(&fixture);
    let host = fixture
        .engine
        .viewport()
        .viewport(TARGET_SURFACE)
        .expect("the recovery host must remain registered")
        .binding();
    let scale = ScaleFactor::new(1.0).unwrap();
    let windows = [host, source]
        .into_iter()
        .enumerate()
        .map(|(index, binding)| {
            let x = if index == 0 { 0.0 } else { 500.0 };
            ObservedWindow::new(binding)
                .with_coordinate_observation(WindowCoordinateObservation::new(
                    binding,
                    CoordinateObservationGeneration::new(1),
                    Authority::Known(PhysicalRect::new(x, 0.0, 400.0, 300.0).unwrap()),
                    Authority::Known(PhysicalRect::new(x - 8.0, -30.0, 416.0, 338.0).unwrap()),
                    Authority::Known(scale),
                    Authority::Known(scale),
                ))
                .with_presentation_observation(WindowPresentationObservation::new(
                    binding,
                    PresentationObservationGeneration::new(1),
                    Authority::Known(WindowPresentationState::Visible),
                ))
        })
        .collect();
    let mut capabilities = PlatformCapabilities::default();
    capabilities.set_native_window_lifecycle(PlatformCapability::Supported);
    capabilities.set_authoritative_inventory(PlatformCapability::Supported);
    capabilities.set_global_window_placement(PlatformCapability::Supported);
    capabilities.set_work_area(PlatformCapability::Supported);
    capabilities.set_global_focus_observation(PlatformCapability::Supported);
    let snapshot = test_platform_snapshot(
        capabilities,
        FocusObservationEnvelope::new(
            FocusObservationGeneration::new(1),
            Authority::Known(GlobalFocusedWindow::Foreign),
            Authority::Known(None),
        ),
        windows,
        Vec::new(),
        known_work_area_observation(
            1,
            vec![ObservedWorkArea::new(
                WorkAreaToken::new(1),
                PhysicalRect::new(0.0, 0.0, 1920.0, 1080.0).unwrap(),
                scale,
            )],
        ),
    )
    .expect("the two real native window observations must form a valid snapshot");
    let expected_epoch = fixture.engine.version().epoch();
    submit_test_input(
        &mut fixture.engine,
        presentation_host,
        EngineInput::PublishPlatformSnapshot {
            provider,
            expected_epoch,
            snapshot,
        },
    )
    .expect("the complete native snapshot must reduce");
    publish_background_scene(&mut fixture);
    let record = fixture.engine.viewport().viewport(SOURCE_SURFACE).unwrap();
    assert_eq!(record.binding(), source);
    assert_eq!(record.role(), ViewportRole::Child);
    assert_eq!(record.ownership(), ViewportOwnership::RuntimeOwned);
    assert_eq!(record.admission(), ViewportAdmission::Admitted);
    assert_eq!(
        fixture
            .engine
            .interaction_authority(SOURCE_SURFACE)
            .and_then(|authority| authority.binding()),
        Some(source)
    );
    assert!(
        fixture
            .engine
            .bound_surface_recoveries
            .contains_key(&SOURCE_SURFACE)
    );
    let _ = fixture.engine.viewport.take_new_effects();
    (fixture, source)
}

fn assert_no_source_release(transition: &EngineTransition, source: ViewportBinding) {
    assert!(transition.platform_effects().iter().all(|effect| {
        !matches!(effect.effect(), PlatformEffect::ReleaseChild { binding } if *binding == source)
    }));
}

fn assert_source_retained(
    fixture: &BackgroundFixture,
    before: &Workspace,
    version: WorkspaceVersion,
    source: ViewportBinding,
) {
    assert_eq!(
        fixture.engine.workspace(),
        before,
        "an unpresented target cannot transfer any source topology or item ownership"
    );
    assert_eq!(fixture.engine.version(), version);
    assert_eq!(
        fixture
            .engine
            .workspace()
            .presentation_for_root(SOURCE_ROOT),
        Some(crate::graph::workspace::RootPresentationOwner::Main {
            surface: SOURCE_SURFACE,
        })
    );
    assert_eq!(
        fixture
            .engine
            .viewport()
            .viewport(SOURCE_SURFACE)
            .map(|record| { (record.binding(), record.admission()) }),
        Some((source, ViewportAdmission::Admitted))
    );
    assert!(
        fixture
            .engine
            .bound_surface_recoveries
            .contains_key(&SOURCE_SURFACE)
    );
}

fn emit_target_preview(fixture: &mut BackgroundFixture) -> EngineTransition {
    let contribution =
        begin_surface_measurement(&fixture.engine, TARGET_SURFACE, background_bounds());
    submit_surface_measurement(&mut fixture.engine, fixture.presentation_host, contribution);
    let mut frame = begin_test_host_frame(&fixture.engine, fixture.presentation_host);
    for obligation in frame.issue_presentation_obligations().unwrap() {
        if obligation.slot().surface() == TARGET_SURFACE {
            let token = frame
                .view()
                .begin_surface_contribution(TARGET_SURFACE)
                .unwrap();
            let interaction = frame
                .view()
                .presentation_interaction(TARGET_SURFACE)
                .expect("the pending target retains its exact preview identity");
            assert!(interaction.drag_preview().is_some());
            frame
                .record_painted_surface_contribution(obligation, token, interaction)
                .expect("the target must emit its actual pending preview");
        } else {
            frame
                .settle_presentation_obligation(
                    obligation,
                    HostPresentationDisposition::Unavailable(
                        HostPresentationUnavailableReason::OutputNotProduced,
                    ),
                )
                .unwrap();
        }
    }
    complete_surface_contribution_roster(&mut frame);
    frame
        .finish(&mut fixture.engine)
        .expect("painting the exact target preview must not itself transfer ownership")
}

fn observe_target_preview(
    fixture: &mut BackgroundFixture,
    output: HostPresentationOutput,
    presented: bool,
) -> EngineTransition {
    let mut prelude = fixture
        .engine
        .begin_host_frame(fixture.presentation_host)
        .unwrap();
    let streams = prelude.pending_presentation_streams().collect::<Vec<_>>();
    assert!(
        streams.contains(&output.stream()),
        "a first renderer terminal must observe an actually pending output"
    );
    prelude
        .submit_presentation_observation(HostPresentationObservation::Batch(
            streams
                .into_iter()
                .map(|stream| {
                    let observation = if stream == output.stream() {
                        HostPresentationStreamObservation::Captured {
                            generation: HostPresentationCaptureGeneration::new(
                                output.key().ordinal_for_test(),
                            ),
                            progress: HostPresentationProgress::Retired {
                                settled_through: output.key(),
                                presented: Authority::Known(presented.then_some(output.key())),
                            },
                        }
                    } else {
                        HostPresentationStreamObservation::NoUpdate
                    };
                    HostPresentationObservationEntry::new(stream, observation)
                })
                .collect(),
        ))
        .expect("the exact renderer terminal must retain its original output identity");
    let mut frame = prelude.seal(&fixture.engine).unwrap();
    complete_host_frame_with_explicit_surface_roster(&fixture.engine, &mut frame);
    frame
        .finish(&mut fixture.engine)
        .expect("a target renderer terminal must settle through the ordinary lifecycle")
}

fn reject_retired_output(
    fixture: &mut BackgroundFixture,
    output: HostPresentationOutput,
) -> EngineTransition {
    let mut prelude = fixture
        .engine
        .begin_host_frame(fixture.presentation_host)
        .unwrap();
    let streams = prelude.pending_presentation_streams().collect::<Vec<_>>();
    assert!(
        !streams.contains(&output.stream()),
        "the renderer already settled every output on this stream"
    );
    let mut entries = streams
        .into_iter()
        .map(|stream| {
            HostPresentationObservationEntry::new(
                stream,
                HostPresentationStreamObservation::NoUpdate,
            )
        })
        .collect::<Vec<_>>();
    entries.push(HostPresentationObservationEntry::new(
        output.stream(),
        HostPresentationStreamObservation::Captured {
            generation: HostPresentationCaptureGeneration::new(output.key().ordinal_for_test()),
            progress: HostPresentationProgress::Retired {
                settled_through: output.key(),
                presented: Authority::Known(Some(output.key())),
            },
        },
    ));
    assert!(matches!(
        prelude.submit_presentation_observation(HostPresentationObservation::Batch(entries)),
        Err(CoreHostFrameError::PresentationObservationStreamOutsideScope { stream })
            if stream == output.stream()
    ));
    drop(prelude);
    // A rejected stale batch cannot poison the next independently valid host frame.
    let mut frame = begin_test_host_frame(&fixture.engine, fixture.presentation_host);
    complete_host_frame_with_explicit_surface_roster(&fixture.engine, &mut frame);
    frame.finish(&mut fixture.engine).unwrap()
}

fn prepared_tab_rehome(
    fixture: &BackgroundFixture,
    nested: bool,
    target: RehomeTarget,
) -> EngineInput {
    let prepared = if nested {
        let group = fixture
            .engine
            .capture_tabs_group(DockAnchor::Item(FIRST))
            .unwrap();
        assert_eq!(group.source.node(), fixture.source_tabs);
        if target == RehomeTarget::Float {
            fixture.engine.prepare_float_tabs(group, TARGET_SURFACE)
        } else {
            fixture.engine.prepare_dock_tabs_with_mode(
                group,
                target.placement(),
                TabGroupDockMode::MergeTabs,
            )
        }
    } else {
        let tab = fixture
            .engine
            .capture_tab(TabKey::for_item(FIRST))
            .expect("the singleton inline tab must capture as a real PreparedTab");
        if target == RehomeTarget::Float {
            fixture.engine.prepare_float_tab(tab, TARGET_SURFACE)
        } else {
            fixture.engine.prepare_dock_tab(tab, target.placement())
        }
    };
    fixture.engine.accept_prepared_action(prepared).unwrap()
}

fn check_tab_rehome(nested: bool, destination: RehomeTarget, presented: bool) {
    let (mut fixture, source) = native_tab_source(nested, SourceRemainder::None);
    let before = fixture.engine.workspace().clone();
    let version = fixture.engine.version();
    let payload = if nested {
        assert_ne!(before.root(SOURCE_ROOT).unwrap().node, fixture.source_tabs);
        MovePayload::Tabs(
            before
                .capture_node_source(SOURCE_ROOT, fixture.source_tabs)
                .unwrap(),
        )
    } else {
        MovePayload::Item(
            before
                .capture_item_source(SOURCE_ROOT, fixture.source_tabs, FIRST)
                .unwrap(),
        )
    };
    let items = before.collect_items_in_subtree(fixture.source_tabs);
    assert_eq!(
        items,
        if nested {
            vec![FIRST, SECOND]
        } else {
            vec![FIRST]
        }
    );
    assert!(before.root(SOURCE_ROOT).unwrap().central.is_none());
    let target = before
        .capture_tab_target(TARGET_ROOT, before.root(TARGET_ROOT).unwrap().node)
        .unwrap();
    // Prove source exhaustion independently of native admission and product action routing.
    let mut graph_candidate = before.clone();
    WorkspaceTransaction::from_commands([WorkspaceCommand::Move {
        payload,
        target: crate::graph::command::DockTarget::Center(target),
    }])
    .apply(&mut graph_candidate, fixture.engine.policy_snapshot())
    .expect("moving the exact payload is a legal graph transaction");
    assert!(graph_candidate.root(SOURCE_ROOT).is_none());
    assert!(graph_candidate.surface(SOURCE_SURFACE).is_none());
    assert_eq!(graph_candidate.item_multiset(), before.item_multiset());
    for item in &items {
        assert_eq!(
            graph_candidate.locate_item_by_id(*item).unwrap().root,
            TARGET_ROOT
        );
    }

    let input = prepared_tab_rehome(&fixture, nested, destination);
    let staged = submit_test_input(&mut fixture.engine, fixture.presentation_host, input)
        .expect("the exact tab action must produce a normal semantic outcome");
    assert!(
        matches!(
            staged.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed { .. }
        ),
        "a legal native rehome cannot be rejected: {:?}",
        staged.reduced_inputs()[0].outcome()
    );
    assert_source_retained(&fixture, &before, version, source);
    assert_no_source_release(&staged, source);
    let pending = fixture
        .engine
        .pending_mutation
        .as_ref()
        .and_then(PendingMutation::rehome)
        .expect("source exhaustion must create one target presentation obligation");
    assert_eq!(pending.source_binding, source);
    assert_eq!(pending.target_surface, TARGET_SURFACE);
    assert_eq!(pending.root, SOURCE_ROOT);

    let mut deferred = begin_test_host_frame(&fixture.engine, fixture.presentation_host);
    complete_host_frame_with_explicit_surface_roster(&fixture.engine, &mut deferred);
    let deferred = deferred.finish(&mut fixture.engine).unwrap();
    assert_source_retained(&fixture, &before, version, source);
    assert_no_source_release(&deferred, source);
    assert!(fixture.engine.pending_mutation.is_some());

    let emitted = emit_target_preview(&mut fixture);
    let outputs = emitted
        .presentation_emissions()
        .iter()
        .map(|emission| emission.output())
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        panic!("only the exact pending target preview may emit: {outputs:?}");
    };
    assert_eq!(output.surface(), TARGET_SURFACE);
    assert_source_retained(&fixture, &before, version, source);
    assert_no_source_release(&emitted, source);

    let settled = observe_target_preview(&mut fixture, *output, presented);
    assert!(fixture.engine.pending_mutation.is_none());
    assert_eq!(
        fixture.engine.workspace().item_multiset(),
        before.item_multiset()
    );
    if presented {
        assert_eq!(fixture.engine.version().epoch(), version.epoch());
        assert_eq!(
            fixture.engine.version().revision(),
            version.revision().checked_next().unwrap()
        );
        for item in &items {
            let location = fixture.engine.workspace().locate_item_by_id(*item).unwrap();
            let owner = fixture
                .engine
                .workspace()
                .presentation_for_root(location.root)
                .unwrap();
            assert_eq!(owner.surface(), TARGET_SURFACE);
            match destination {
                RehomeTarget::Float => assert!(matches!(
                    owner,
                    crate::graph::workspace::RootPresentationOwner::Contained { .. }
                )),
                RehomeTarget::Main => {
                    assert_eq!(
                        owner,
                        crate::graph::workspace::RootPresentationOwner::Main {
                            surface: TARGET_SURFACE,
                        }
                    );
                    assert_eq!(
                        fixture
                            .engine
                            .workspace()
                            .surface(TARGET_SURFACE)
                            .unwrap()
                            .main_root,
                        Some(location.root)
                    );
                }
                RehomeTarget::Tabs => assert_eq!(location.root, TARGET_ROOT),
            }
            if nested && destination != RehomeTarget::Tabs {
                assert_eq!(
                    fixture.engine.workspace().root(location.root).unwrap().node,
                    fixture.source_tabs,
                    "placing the inner group cannot widen the payload to its outer tabs"
                );
            }
        }
        assert!(fixture.engine.workspace().surface(SOURCE_SURFACE).is_none());
        assert_eq!(
            settled
                .platform_effects()
                .iter()
                .filter(|effect| matches!(
                    effect.effect(),
                    PlatformEffect::ReleaseChild { binding } if *binding == source
                ))
                .count(),
            1,
            "the presented target must retire the exact native source once"
        );
        assert_eq!(
            settled
                .events()
                .iter()
                .filter(|event| matches!(
                    event.kind(),
                    WorkspaceEventKind::PresentationRehomeSettled {
                        result: crate::engine::event::PresentationRehomeResult::Applied,
                        ..
                    }
                ))
                .count(),
            1
        );
    } else {
        assert_source_retained(&fixture, &before, version, source);
        assert_no_source_release(&settled, source);
        assert!(settled.events().iter().any(|event| matches!(
            event.kind(),
            WorkspaceEventKind::PresentationRehomeSettled {
                result: crate::engine::event::PresentationRehomeResult::TargetNotPresented,
                outcome: None,
                ..
            }
        )));
    }

    let after = fixture.engine.workspace().clone();
    let after_version = fixture.engine.version();
    let replayed = reject_retired_output(&mut fixture, *output);
    assert_eq!(fixture.engine.workspace(), &after);
    assert_eq!(fixture.engine.version(), after_version);
    assert_no_source_release(&replayed, source);
    assert!(replayed.events().is_empty());
    assert!(fixture.engine.pending_mutation.is_none());
}

#[test]
fn inline_native_tab_rehome_defers_ownership_until_target_is_presented() {
    for target in [RehomeTarget::Tabs, RehomeTarget::Float, RehomeTarget::Main] {
        check_tab_rehome(false, target, true);
    }
}

#[test]
fn inline_native_tab_rehome_dropped_target_preserves_source_ownership() {
    for target in [RehomeTarget::Tabs, RehomeTarget::Float, RehomeTarget::Main] {
        check_tab_rehome(false, target, false);
    }
}

#[test]
fn nested_sole_layout_tab_rehome_defers_ownership_until_target_is_presented() {
    for target in [RehomeTarget::Tabs, RehomeTarget::Float, RehomeTarget::Main] {
        check_tab_rehome(true, target, true);
    }
}

#[test]
fn nested_sole_layout_tab_rehome_dropped_target_preserves_source_ownership() {
    for target in [RehomeTarget::Tabs, RehomeTarget::Float, RehomeTarget::Main] {
        check_tab_rehome(true, target, false);
    }
}

fn check_nonvacant_source(remainder: SourceRemainder) {
    for target in [RehomeTarget::Tabs, RehomeTarget::Float, RehomeTarget::Main] {
        let (mut fixture, source) = native_tab_source(true, remainder);
        let before_items = fixture.engine.workspace().item_multiset();
        let version = fixture.engine.version();
        let input = prepared_tab_rehome(&fixture, true, target);
        let moved = submit_test_input(&mut fixture.engine, fixture.presentation_host, input)
            .expect("a payload leaving a real source remainder must move normally");
        assert!(
            matches!(
                moved.reduced_inputs()[0].outcome(),
                InputOutcome::ProductActionProcessed { .. }
            ),
            "{remainder:?} to {target:?}: {:?}",
            moved.reduced_inputs()[0].outcome()
        );
        assert!(fixture.engine.pending_mutation.is_none());
        assert_eq!(
            fixture.engine.version().revision(),
            version.revision().checked_next().unwrap()
        );
        assert_eq!(fixture.engine.workspace().item_multiset(), before_items);
        assert_eq!(
            fixture
                .engine
                .workspace()
                .presentation_for_root(SOURCE_ROOT),
            Some(crate::graph::workspace::RootPresentationOwner::Main {
                surface: SOURCE_SURFACE,
            })
        );
        assert_eq!(
            fixture
                .engine
                .viewport()
                .viewport(SOURCE_SURFACE)
                .map(|record| record.binding()),
            Some(source)
        );
        assert!(
            fixture
                .engine
                .bound_surface_recoveries
                .contains_key(&SOURCE_SURFACE)
        );
        assert_no_source_release(&moved, source);
        for item in [FIRST, SECOND] {
            let location = fixture.engine.workspace().locate_item_by_id(item).unwrap();
            assert_eq!(
                fixture
                    .engine
                    .workspace()
                    .presentation_for_root(location.root)
                    .unwrap()
                    .surface(),
                TARGET_SURFACE
            );
        }
        let expected = fixture.engine.version();
        let follow_up = match remainder {
            SourceRemainder::HiddenSibling => {
                assert!(fixture.engine.workspace().item_is_hidden(HIDDEN));
                assert_eq!(
                    fixture.engine.workspace().collect_items_in_subtree(
                        fixture.engine.workspace().root(SOURCE_ROOT).unwrap().node
                    ),
                    [HIDDEN]
                );
                EngineInput::SetItemVisibility {
                    expected,
                    item: HIDDEN,
                    visible: true,
                }
            }
            SourceRemainder::CentralAnchor => {
                let root = fixture.engine.workspace().root(SOURCE_ROOT).unwrap();
                assert_eq!(root.central, Some(root.node));
                assert!(matches!(
                    fixture.engine.workspace().node(root.node),
                    Some(Node::Tabs { items, .. }) if items.is_empty()
                ));
                EngineInput::OpenItem {
                    expected,
                    item: ItemId::new(5),
                    placement: DockPlacement::Center(DockAnchor::Central(SOURCE_ROOT)),
                }
            }
            SourceRemainder::None => unreachable!("this control must retain a real remainder"),
        };
        let continued =
            submit_test_input(&mut fixture.engine, fixture.presentation_host, follow_up)
                .expect("source preservation cannot leave subsequent legal input blocked");
        assert!(matches!(
            continued.reduced_inputs()[0].outcome(),
            InputOutcome::ProductActionProcessed { .. }
        ));
        assert!(fixture.engine.pending_mutation.is_none());
        assert_no_source_release(&continued, source);
    }
}

#[test]
fn hidden_outer_sibling_keeps_native_source_live_without_a_rehome_barrier() {
    check_nonvacant_source(SourceRemainder::HiddenSibling);
}

#[test]
fn empty_central_anchor_keeps_native_source_live_without_a_rehome_barrier() {
    check_nonvacant_source(SourceRemainder::CentralAnchor);
}
