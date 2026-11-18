//! Fixed hosts inspect the real move candidate instead of predicting graph cleanup.

use super::*;
use crate::model::{DockMoveSource, DockMoveTarget, TabGroupDockMode};

#[test]
fn surface_retirement_preflight_matches_whole_and_partial_move_candidates() {
    for kind in 0..4 {
        let mut session = session();
        let source = match kind {
            0 => DockMoveSource::Item(FIRST),
            1 => DockMoveSource::Tab(session.capture_tab(FIRST.into()).unwrap()),
            2 => DockMoveSource::Tabs(session.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap()),
            _ => DockMoveSource::Root(ROOT),
        };
        let action = session.prepare_move(
            source,
            DockMoveTarget::Dock {
                placement: DockPlacement::Center(DockAnchor::Item(THIRD)),
                mode: TabGroupDockMode::PreserveLayout,
            },
        );
        let before = session.workspace().clone();
        let version = session.version();
        assert_eq!(
            session.action_removes_surface(&action, SURFACE).unwrap(),
            kind >= 2
        );
        assert!(
            !session
                .action_removes_surface(&action, OTHER_SURFACE)
                .unwrap()
        );
        assert_eq!(session.workspace(), &before);
        assert_eq!(session.version(), version);
        assert!(apply_action(&mut session, action));
        assert_eq!(session.view().surface(SURFACE).is_none(), kind >= 2);
        assert!(session.view().surface(OTHER_SURFACE).is_some());
    }
}

#[test]
fn surface_retirement_preflight_preserves_stale_foreign_and_policy_rejections() {
    let mut owner = session();
    let foreign = session();
    let target = DockMoveTarget::Dock {
        placement: DockPlacement::Center(DockAnchor::Item(THIRD)),
        mode: TabGroupDockMode::MergeTabs,
    };
    let foreign_action = owner.prepare_move(
        DockMoveSource::Tabs(foreign.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap()),
        target,
    );
    assert!(
        !owner
            .action_removes_surface(&foreign_action, SURFACE)
            .unwrap()
    );
    let stale = owner.prepare_move(DockMoveSource::Root(ROOT), target);
    let selection = owner.prepare_select_item(SECOND);
    assert!(apply_action(&mut owner, selection));
    assert!(!owner.action_removes_surface(&stale, SURFACE).unwrap());

    let mut policy = DockPolicy::default();
    policy.set_arrangement_locked(true);
    let mut frame = owner.begin_host_frame().unwrap();
    frame.replace_policy(policy).unwrap();
    commit(frame);
    let locked = owner.prepare_move(DockMoveSource::Root(ROOT), target);
    assert!(!owner.action_removes_surface(&locked, SURFACE).unwrap());
    let mut frame = owner.begin_host_frame().unwrap();
    let submitted = frame.submit_prepared_action(locked).unwrap();
    let report = commit(frame);
    assert!(matches!(
        report.submitted_action_outcome(submitted),
        Some(HostInputOutcome::ProductActionRejected(
            DockspaceActionRejection::PolicyDenied
        ))
    ));
    assert!(owner.view().surface(SURFACE).is_some());
}
