//! Fixed hosts inspect the real move candidate instead of predicting graph cleanup.

use super::*;
use crate::model::{DockMoveSource, DockMoveTarget, DockspaceActionAvailability, TabGroupDockMode};

fn removes(session: &DockspaceSession, action: &PreparedDockAction, surface: SurfaceId) -> bool {
    match session.action_availability(action).unwrap() {
        DockspaceActionAvailability::Ready {
            retired_surfaces, ..
        } => retired_surfaces.contains(&surface),
        DockspaceActionAvailability::Unavailable(_) => false,
    }
}

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
        assert_eq!(removes(&session, &action, SURFACE), kind >= 2);
        assert!(!removes(&session, &action, OTHER_SURFACE));
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
    assert!(!removes(&owner, &foreign_action, SURFACE));
    let stale = owner.prepare_move(DockMoveSource::Root(ROOT), target);
    let selection = owner.prepare_select_item(SECOND);
    assert!(apply_action(&mut owner, selection));
    assert!(!removes(&owner, &stale, SURFACE));

    let mut policy = DockPolicy::default();
    policy.set_arrangement_locked(true);
    let mut frame = owner.begin_host_frame().unwrap();
    frame.replace_policy(policy).unwrap();
    commit(frame);
    let locked = owner.prepare_move(DockMoveSource::Root(ROOT), target);
    assert!(!removes(&owner, &locked, SURFACE));
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

#[test]
fn required_surfaces_are_enforced_by_the_submitted_candidate_for_every_move_source() {
    for (kind, host_lifetime) in (0..4).flat_map(|kind| [(kind, false), (kind, true)]) {
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
        let action = if host_lifetime {
            // Already prepared actions must observe a later host declaration.
            session.retain_host_surface(SURFACE).unwrap();
            session.retain_host_surface(OTHER_SURFACE).unwrap();
            action
        } else {
            action.preserving_surfaces([SURFACE, OTHER_SURFACE])
        };
        let before = session.workspace().clone();
        let version = session.version();
        let availability = session.action_availability(&action).unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        let submitted = frame.submit_prepared_action(action).unwrap();
        let report = commit(frame);
        if kind >= 2 {
            assert_eq!(
                availability,
                DockspaceActionAvailability::Unavailable(
                    DockspaceActionRejection::SurfaceRequired { surface: SURFACE },
                )
            );
            assert!(matches!(
                report.submitted_action_outcome(submitted),
                Some(HostInputOutcome::ProductActionRejected(
                    DockspaceActionRejection::SurfaceRequired { surface: SURFACE }
                ))
            ));
            assert_eq!(session.workspace(), &before);
            assert_eq!(session.version(), version);
        } else {
            assert!(matches!(
                availability,
                DockspaceActionAvailability::Ready {
                    requires_presentation: false,
                    ..
                }
            ));
            assert!(matches!(
                report.submitted_action_outcome(submitted),
                Some(HostInputOutcome::ProductActionApplied(_))
            ));
        }
        assert!(session.view().surface(SURFACE).is_some());
        assert!(session.view().surface(OTHER_SURFACE).is_some());
    }
}

#[test]
fn host_surface_replacement_requires_retention_and_failed_reset_is_atomic() {
    let mut session = session();
    session.retain_host_surface(SURFACE).unwrap();
    let before = session.workspace().clone();
    let version = session.version();
    let without_host = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        OTHER_SURFACE,
        DockspaceRootLayout::new(
            OTHER_ROOT,
            DockspaceNode::tabs([FIRST, SECOND, THIRD, FOURTH]),
        ),
    )])
    .unwrap();
    let reset = session.prepare_layout_reset(without_host).unwrap();
    assert_eq!(
        session
            .begin_layout_reset_frame(&reset)
            .err()
            .unwrap()
            .kind(),
        DockspaceRuntimeErrorKind::OperationConflict
    );
    assert_eq!(session.workspace(), &before);
    assert_eq!(session.version(), version);

    let with_host = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([FIRST, SECOND, THIRD, FOURTH])),
    )])
    .unwrap();
    let reset = session.prepare_layout_reset(with_host).unwrap();
    commit(session.begin_layout_reset_frame(&reset).unwrap());
    assert!(session.view().surface(SURFACE).is_some());
    assert!(session.view().surface(OTHER_SURFACE).is_none());
    assert_ne!(session.version().epoch(), version.epoch());
    session.retain_host_surface(SURFACE).unwrap();
}

#[test]
fn host_surface_declaration_respects_suspended_frame_ownership() {
    let mut session = session();
    assert_eq!(
        session
            .retain_host_surface(SurfaceId::new(99))
            .unwrap_err()
            .kind(),
        DockspaceRuntimeErrorKind::OperationConflict
    );
    let update = session.begin_update().unwrap();
    assert_eq!(
        session.retain_host_surface(SURFACE).unwrap_err().kind(),
        DockspaceRuntimeErrorKind::UpdateInProgress
    );
    drop(update);
    session.retain_host_surface(SURFACE).unwrap();
    session.retain_host_surface(SURFACE).unwrap();
    let action = session.prepare_move(
        DockMoveSource::Root(ROOT),
        DockMoveTarget::Dock {
            placement: DockPlacement::Center(DockAnchor::Item(THIRD)),
            mode: TabGroupDockMode::MergeTabs,
        },
    );
    assert_eq!(
        session.action_availability(&action).unwrap(),
        DockspaceActionAvailability::Unavailable(DockspaceActionRejection::SurfaceRequired {
            surface: SURFACE,
        })
    );
}
