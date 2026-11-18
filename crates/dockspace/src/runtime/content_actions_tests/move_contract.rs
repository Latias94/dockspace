//! All programmatic move sources use the same destination and capture contract.

use super::*;
use crate::model::{
    DockMoveSource, DockMoveTarget, DockspaceAxis, DockspaceGridShape, TabGroupDockMode,
};

fn source(session: &DockspaceSession, kind: usize) -> DockMoveSource {
    match kind {
        0 => DockMoveSource::Item(FIRST),
        1 => DockMoveSource::Tab(session.capture_tab(FIRST.into()).unwrap()),
        2 => DockMoveSource::Tabs(session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap()),
        _ => DockMoveSource::Root(ROOT),
    }
}

fn dock_target(mode: TabGroupDockMode) -> DockMoveTarget {
    DockMoveTarget::Dock {
        placement: DockPlacement::Center(DockAnchor::Item(THIRD)),
        mode,
    }
}

fn measure(session: &mut DockspaceSession) {
    let mut frame = session.begin_host_frame().unwrap();
    for surface in [SURFACE, OTHER_SURFACE] {
        frame
            .measure_surface(
                surface,
                UniformSurfaceMetrics::new(
                    LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap(),
                    LogicalSize::new(40.0, 40.0).unwrap(),
                    70.0,
                )
                .unwrap(),
            )
            .unwrap();
    }
    commit(frame);
}

#[test]
fn move_sources_share_docking_and_contained_destinations_without_widening_scope() {
    for kind in 0..4 {
        for contained in [false, true] {
            let mut session = session();
            measure(&mut session);
            let target = if contained {
                DockMoveTarget::Contained {
                    surface: OTHER_SURFACE,
                }
            } else {
                dock_target(TabGroupDockMode::MergeTabs)
            };
            let action = session.prepare_move(source(&session, kind), target);
            assert!(
                apply_action(&mut session, action),
                "kind={kind}, {target:?}"
            );
            for item in if kind < 2 {
                &[FIRST][..]
            } else {
                &[FIRST, SECOND][..]
            } {
                let moved = session.view().item(*item).unwrap();
                assert_eq!(moved.surface(), OTHER_SURFACE);
                assert_eq!(moved.contained().is_some(), contained);
                if contained && kind >= 2 {
                    assert_eq!(moved.root(), ROOT);
                }
            }
            if kind < 2 {
                assert_eq!(session.view().item(SECOND).unwrap().surface(), SURFACE);
            }
            assert_eq!(session.view().item(THIRD).unwrap().root(), OTHER_ROOT);
            assert_eq!(session.view().item(FOURTH).unwrap().root(), OTHER_ROOT);
            session.workspace().validate().unwrap();
            assert_eq!(
                session.workspace().item_multiset().values().sum::<usize>(),
                4
            );
        }
    }
}

#[test]
fn move_of_a_tabs_group_or_tabs_root_respects_explicit_center_insertion() {
    for root in [false, true] {
        for mode in [
            TabGroupDockMode::MergeTabs,
            TabGroupDockMode::PreserveLayout,
        ] {
            let mut session = session();
            let content = session.view().root(ROOT).unwrap().content().unwrap().key();
            let moving = source(&session, if root { 3 } else { 2 });
            let action = session.prepare_move(moving, dock_target(mode));
            assert!(apply_action(&mut session, action));
            let target = session
                .view()
                .root(OTHER_ROOT)
                .unwrap()
                .content()
                .unwrap()
                .tabs()
                .unwrap();
            match mode {
                TabGroupDockMode::MergeTabs => {
                    assert_eq!(
                        target.tabs(),
                        [THIRD.into(), FOURTH.into(), FIRST.into(), SECOND.into()]
                    );
                }
                TabGroupDockMode::PreserveLayout => {
                    assert_eq!(
                        target.tabs(),
                        [THIRD.into(), FOURTH.into(), TabKey::for_layout(content.0)]
                    );
                    let nested = session.view().node(content).unwrap().tabs().unwrap();
                    assert_eq!(nested.tabs(), [FIRST.into(), SECOND.into()]);
                }
            }
        }
    }
}

#[test]
fn move_capture_keeps_its_original_session_and_revision_for_every_destination() {
    for kind in [1, 2] {
        for target in [
            dock_target(TabGroupDockMode::PreserveLayout),
            DockMoveTarget::Contained {
                surface: OTHER_SURFACE,
            },
            DockMoveTarget::Native { placement: None },
        ] {
            let mut session = session();
            let foreign = self::session();
            let captured = source(&session, kind);
            let original = session.version();
            let select = session.prepare_select_item(SECOND);
            assert!(apply_action(&mut session, select));
            let action = session.prepare_move(captured, target);
            assert_eq!(action.expected_version(), original);
            let current = session.version();
            let mut frame = session.begin_host_frame().unwrap();
            let submitted = frame.submit_prepared_action(action).unwrap();
            let report = commit(frame);
            assert_eq!(
                report.submitted_action_outcome(submitted),
                Some(&HostInputOutcome::StaleRejected {
                    expected: original,
                    accepted: current,
                })
            );
            let action = session.prepare_move(source(&foreign, kind), target);
            let mut frame = session.begin_host_frame().unwrap();
            let error = frame.submit_prepared_action(action).unwrap_err();
            assert!(error.is_foreign_prepared_action());
            drop(frame);
            assert_eq!(session.version(), current);
            assert_eq!(session.view().item(FIRST).unwrap().root(), ROOT);
        }
    }
}

#[test]
fn moving_a_complete_root_retains_bare_and_composed_content_identity() {
    for content in [
        DockspaceNode::pane(FIRST),
        DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::pane(FIRST), DockspaceNode::pane(SECOND)],
        )
        .unwrap(),
        DockspaceNode::grid(
            DockspaceGridShape::Auto,
            [DockspaceNode::pane(FIRST), DockspaceNode::tabs([SECOND])],
        )
        .unwrap(),
    ] {
        for contained in [false, true] {
            let layout = DockspaceLayout::new([
                DockspaceSurfaceLayout::new(
                    SURFACE,
                    DockspaceRootLayout::new(ROOT, content.clone()),
                ),
                DockspaceSurfaceLayout::new(
                    OTHER_SURFACE,
                    DockspaceRootLayout::new(OTHER_ROOT, DockspaceNode::tabs([THIRD])),
                ),
            ])
            .unwrap();
            let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
            measure(&mut session);
            let key = session.view().root(ROOT).unwrap().content().unwrap().key();
            let items = session
                .view()
                .root(ROOT)
                .unwrap()
                .items()
                .collect::<Vec<_>>();
            let bare = items.len() == 1;
            let action = session.prepare_move(
                DockMoveSource::Root(ROOT),
                if contained {
                    DockMoveTarget::Contained {
                        surface: OTHER_SURFACE,
                    }
                } else {
                    dock_target(TabGroupDockMode::PreserveLayout)
                },
            );
            assert!(apply_action(&mut session, action));
            if contained || !bare {
                assert_eq!(
                    session
                        .view()
                        .node(key)
                        .unwrap()
                        .items()
                        .collect::<Vec<_>>(),
                    items
                );
            }
            if contained {
                let moved = session.view().node(key).unwrap();
                assert_eq!(moved.root().unwrap().id(), ROOT);
                assert!(session.view().item(FIRST).unwrap().contained().is_some());
            } else {
                assert_eq!(session.view().item(FIRST).unwrap().root(), OTHER_ROOT);
                let target = session
                    .view()
                    .root(OTHER_ROOT)
                    .unwrap()
                    .content()
                    .unwrap()
                    .tabs()
                    .unwrap();
                assert_eq!(
                    target.tabs(),
                    [
                        THIRD.into(),
                        if bare {
                            FIRST.into()
                        } else {
                            TabKey::for_layout(key.0)
                        }
                    ]
                );
            }
            session.workspace().validate().unwrap();
        }
    }
}
