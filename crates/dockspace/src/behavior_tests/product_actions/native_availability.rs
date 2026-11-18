//! Native availability must account for the recovery host after canonical detachment.

use super::*;
use crate::model::DockMoveSource;

const HIDDEN_OUTER: ItemId = ItemId::new(33);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceRemainder {
    None,
    HiddenSibling,
    CentralAnchor,
}

#[derive(Clone, Copy, Debug)]
enum NestedSource {
    Tabs,
    LayoutTab,
}

impl NestedSource {
    fn capture(self, session: &DockspaceSession) -> DockMoveSource {
        match self {
            Self::Tabs => DockMoveSource::Tabs(
                session
                    .capture_tabs_group(DockAnchor::Item(NATIVE_FIRST))
                    .expect("the inner tabs group is capturable"),
            ),
            Self::LayoutTab => {
                let tab = session
                    .view()
                    .root(NATIVE_SOURCE_ROOT)
                    .and_then(|root| root.content())
                    .and_then(|node| node.tabs())
                    .expect("the external native root contains the outer tabs")
                    .tabs()[0];
                assert!(tab.layout().is_some());
                DockMoveSource::Tab(
                    session
                        .capture_tab(tab)
                        .expect("the outer layout tab is capturable"),
                )
            }
        }
    }
}

fn placement() -> NativeWindowPlacement {
    NativeWindowPlacement::new(
        PhysicalRect::new(920.0, 120.0, 420.0, 320.0)
            .expect("the explicit native placement validates"),
    )
}

fn live_external_nested_source(
    remainder: SourceRemainder,
) -> (DockspaceSession, NativeSurfaceBinding) {
    let inner =
        DockspaceNode::tabs_with_selection([NATIVE_FIRST, NATIVE_SECOND], Some(NATIVE_SECOND))
            .expect("the inner selection validates")
            .named("Tools");
    let mut children = vec![inner];
    if remainder == SourceRemainder::HiddenSibling {
        children.push(DockspaceNode::pane(HIDDEN_OUTER));
    }
    let outer = if remainder == SourceRemainder::CentralAnchor {
        DockspaceNode::central_composed_tabs(children)
    } else {
        DockspaceNode::composed_tabs(children)
    };
    let mut layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        NATIVE_SOURCE_SURFACE,
        DockspaceRootLayout::new(NATIVE_SOURCE_ROOT, outer),
    )])
    .expect("the nested external native layout validates");
    if remainder == SourceRemainder::HiddenSibling {
        layout = layout
            .with_hidden_items([HIDDEN_OUTER])
            .expect("the outer sibling remains owned while hidden");
    }
    let mut policy = crate::policy::DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    let mut session = DockspaceSession::from_layout(layout, policy)
        .expect("the nested external native session initializes");
    session
        .enable_managed_native_host(NativePointerRoster::Exact(Vec::new()))
        .expect("the managed native host enrolls");
    session
        .configure_managed_native_capabilities(managed_native_capabilities())
        .expect("the managed native capabilities configure");
    session
        .register_native_root(NATIVE_SOURCE_SURFACE, NATIVE_ROOT_WINDOW)
        .expect("the source registers as an external native root");
    let registration = commit_managed_native_frame(&mut session);
    let [HostInputOutcome::NativeSurfaceRegistered { binding }] = registration.inputs() else {
        panic!(
            "unexpected external native registration: {:?}",
            registration.inputs()
        );
    };
    let binding = *binding;
    session
        .report_managed_native_snapshot(
            [(
                binding,
                managed_native_window_facts(
                    PhysicalRect::new(80.0, 60.0, 800.0, 600.0)
                        .expect("the external source bounds validate"),
                ),
            )],
            managed_native_work_areas(),
        )
        .expect("the external source reports exact live native facts");
    commit_managed_native_frame(&mut session);
    present_surfaces(&mut session, &[NATIVE_SOURCE_SURFACE]);
    commit_managed_native_frame(&mut session);

    assert!(session.is_current_native_binding(binding));
    assert!(!session.has_pending_native_create());
    assert!(!session.has_pending_presentation());
    let root = session
        .view()
        .root(NATIVE_SOURCE_ROOT)
        .expect("the external source root exists");
    assert_eq!(
        root.items().collect::<Vec<_>>(),
        if remainder == SourceRemainder::HiddenSibling {
            vec![NATIVE_FIRST, NATIVE_SECOND, HIDDEN_OUTER]
        } else {
            vec![NATIVE_FIRST, NATIVE_SECOND]
        }
    );
    assert_eq!(
        root.central().is_some(),
        remainder == SourceRemainder::CentralAnchor
    );
    assert_eq!(
        session.view().hidden_items().collect::<Vec<_>>(),
        if remainder == SourceRemainder::HiddenSibling {
            vec![HIDDEN_OUTER]
        } else {
            Vec::new()
        }
    );
    let surface = session
        .view()
        .surface(NATIVE_SOURCE_SURFACE)
        .expect("the source is the only logical recovery surface");
    assert_eq!(
        surface.main_root().map(|root| root.id()),
        Some(NATIVE_SOURCE_ROOT)
    );
    assert_eq!(surface.contained().count(), 0);
    assert_eq!(session.view().surfaces().count(), 1);
    (session, binding)
}

#[test]
fn sole_nested_native_payload_is_unavailable_before_its_recovery_host_is_pruned() {
    for source_kind in [NestedSource::Tabs, NestedSource::LayoutTab] {
        let (session, binding) = live_external_nested_source(SourceRemainder::None);
        let before = session.workspace().clone();
        let version = session.version();
        let source = source_kind.capture(&session);

        let command = session
            .native_window_command(source, placement())
            .expect("the nested native availability query is nonfatal");

        assert_eq!(session.workspace(), &before);
        assert_eq!(session.version(), version);
        assert!(session.is_current_native_binding(binding));
        assert!(!session.has_pending_native_create());
        assert!(!session.has_pending_presentation());
        assert_eq!(
            command.unavailable_reason(),
            Some(DockspaceActionRejection::NativeUnavailable),
            "{source_kind:?} would remove the external root which is its only recovery host"
        );
    }
}

#[test]
fn sole_nested_native_payload_submission_rejects_without_starting_native_creation() {
    for source_kind in [NestedSource::Tabs, NestedSource::LayoutTab] {
        for prepared in [false, true] {
            let (mut session, binding) = live_external_nested_source(SourceRemainder::None);
            let before = session.workspace().clone();
            let version = session.version();
            let source = source_kind.capture(&session);
            let action = session.prepare_move_to_new_window(source.clone(), placement());
            let mut frame = session
                .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
                .expect("the nested native submission frame begins");
            let submitted = if prepared {
                frame
                    .submit_prepared_action(action)
                    .expect("the prepared nested tear-off stages")
            } else {
                frame
                    .move_to_new_window_current(source, placement())
                    .expect("the current nested tear-off stages")
            };
            let mut report = commit(frame);

            assert_eq!(report.before(), version);
            assert_eq!(report.after(), version);
            assert_eq!(
                report.inputs(),
                &[HostInputOutcome::ProductActionRejected(
                    DockspaceActionRejection::NativeUnavailable,
                )],
                "{source_kind:?}, prepared={prepared}"
            );
            assert_eq!(
                report.submitted_action_outcome(submitted),
                report.inputs().first()
            );
            assert!(report.events().is_empty());
            assert!(report.presentation_transitions().is_empty());
            assert!(report.native_admissions().is_empty());
            assert_eq!(report.native_bindings(), &[binding]);
            assert!(report.take_native_effects().is_empty());
            assert_eq!(session.workspace(), &before);
            assert_eq!(session.version(), version);
            assert!(session.is_current_native_binding(binding));
            assert!(!session.has_pending_native_create());
            assert!(!session.has_pending_presentation());
        }
    }
}

#[test]
fn retained_native_source_keeps_nested_tear_off_available() {
    for remainder in [
        SourceRemainder::HiddenSibling,
        SourceRemainder::CentralAnchor,
    ] {
        for source_kind in [NestedSource::Tabs, NestedSource::LayoutTab] {
            let (mut session, binding) = live_external_nested_source(remainder);
            let before = session.workspace().clone();
            let version = session.version();
            let source = source_kind.capture(&session);
            let command = session
                .native_window_command(source, placement())
                .expect("the retained native source has queryable availability");
            let action = command.into_prepared().unwrap_or_else(|reason| {
                panic!("{remainder:?}, {source_kind:?} must remain ready: {reason:?}")
            });
            assert_eq!(session.workspace(), &before);
            assert_eq!(session.version(), version);
            assert!(!session.has_pending_native_create());

            let mut frame = session
                .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)
                .expect("the retained native source submission frame begins");
            frame
                .submit_prepared_action(action)
                .expect("the available native tear-off stages");
            let mut report = commit(frame);
            assert!(matches!(
                report.inputs(),
                [HostInputOutcome::ProductActionApplied(
                    DockspaceActionOutcome::NativeRootTearOffRequested {
                        source_surface: NATIVE_SOURCE_SURFACE,
                        items,
                        ..
                    }
                )] if items == &[NATIVE_FIRST, NATIVE_SECOND]
            ));
            let effects = report.take_native_effects();
            assert!(matches!(
                effects.as_slice(),
                [effect] if matches!(
                    effect.operation(),
                    NativeEffectOperation::CreateWindow { placement: requested, .. }
                        if *requested == placement().outer_rect()
                )
            ));
            assert_eq!(report.before(), version);
            assert_eq!(report.after(), version);
            assert_eq!(session.workspace(), &before);
            assert!(session.is_current_native_binding(binding));
            assert!(session.has_pending_native_create());
            assert!(!session.has_pending_presentation());
        }
    }
}
