//! Application edits must not let an old gesture commit against a new owner.

use super::*;
use crate::{DockAnchor, DockPlacement};

#[test]
fn hiding_a_drag_source_or_destination_revokes_the_old_drop_without_losing_content() {
    for hidden in [A, C] {
        let mut harness = GridHarness::new();
        harness.begin_drag(C.into());
        let point = harness.hover_guide(A, DockspaceDropDirection::Center);
        let result = harness
            .dockspace
            .set_item_visibility_current(hidden, false)
            .unwrap();
        assert!(matches!(result.status(), DockspaceActionStatus::Applied(_)));
        harness.expected_hidden.insert(hidden);
        let version = harness.dockspace.version();
        harness.run(Vec::new());
        harness.run(vec![button(point, false)]);
        harness.settle();
        assert_eq!(harness.dockspace.version(), version);
        harness.assert_random_invariants();
        assert_eq!(
            harness
                .dockspace
                .view()
                .node(harness.cells[0])
                .unwrap()
                .item(),
            Some(A)
        );
        assert_eq!(
            harness
                .dockspace
                .view()
                .item(C)
                .unwrap()
                .tabs()
                .unwrap()
                .key(),
            harness.cells[1]
        );
        let result = harness
            .dockspace
            .set_item_visibility_current(hidden, true)
            .unwrap();
        assert!(matches!(result.status(), DockspaceActionStatus::Applied(_)));
        harness.expected_hidden.remove(&hidden);
        harness.settle();
        harness.begin_drag(C.into());
        let point = harness.hover_guide(A, DockspaceDropDirection::Center);
        harness.release(point);
        harness.assert_tabs(harness.cells[0], &[A.into(), C.into()], C.into());
        harness.assert_random_invariants();
    }
}

#[test]
fn an_application_move_revokes_the_old_drag_source_before_a_late_release() {
    let mut harness = GridHarness::new();
    harness.begin_drag(C.into());
    let point = harness.hover_guide(A, DockspaceDropDirection::Center);
    let result = harness
        .dockspace
        .dock_item_current(C, DockPlacement::Center(DockAnchor::Item(E)))
        .unwrap();
    assert!(matches!(result.status(), DockspaceActionStatus::Applied(_)));
    let version = harness.dockspace.version();
    harness.run(Vec::new());
    harness.run(vec![button(point, false)]);
    harness.settle();
    assert_eq!(harness.dockspace.version(), version);
    harness.assert_tabs(harness.cells[1], &[B.into()], B.into());
    harness.assert_tabs(
        harness.cells[3],
        &[E.into(), harness.tools, C.into()],
        C.into(),
    );
    harness.assert_random_invariants();
    harness.begin_drag(C.into());
    let point = harness.hover_guide(A, DockspaceDropDirection::Center);
    harness.release(point);
    harness.assert_tabs(harness.cells[0], &[A.into(), C.into()], C.into());
    harness.assert_random_invariants();
}

#[test]
fn hiding_the_last_visible_layout_descendant_cancels_the_whole_drag_and_keeps_hidden_ownership() {
    let mut harness = GridHarness::new();
    let mut trace = Vec::new();
    harness.random_visibility(3, &mut trace);
    assert_eq!(harness.expected_hidden, BTreeSet::from([F]));
    harness.begin_drag(harness.tools);
    let point = harness.hover_guide(A, DockspaceDropDirection::Center);
    harness.random_visibility(4, &mut trace);
    assert_eq!(harness.expected_hidden, BTreeSet::from([F, G]));
    let version = harness.dockspace.version();
    harness.run(vec![button(point, false)]);
    harness.settle();
    assert_eq!(harness.dockspace.version(), version);
    harness.assert_tabs(harness.cells[3], &[E.into(), harness.tools], E.into());
    assert_eq!(
        harness
            .dockspace
            .view()
            .node(harness.cells[0])
            .unwrap()
            .item(),
        Some(A)
    );
    harness.assert_random_invariants();
    harness.random_visibility(4, &mut trace);
    harness.begin_drag(harness.tools);
    let point = harness.hover_guide(A, DockspaceDropDirection::Center);
    harness.release(point);
    harness.assert_tabs(harness.cells[0], &[A.into(), harness.tools], harness.tools);
    harness.assert_tabs(harness.cells[3], &[E.into()], E.into());
    assert_eq!(harness.expected_hidden, BTreeSet::from([F]));
    harness.assert_random_invariants();
}

#[test]
fn switching_a_layout_branch_during_grid_resize_discards_transient_shares_and_old_release() {
    for orientation in [Orientation::Horizontal, Orientation::Vertical] {
        let mut harness = GridHarness::new();
        let (_, point) = harness.begin_grid_resize(orientation);
        let delta = match orientation {
            Orientation::Horizontal => vec2(0.0, 24.0),
            Orientation::Vertical => vec2(24.0, 0.0),
        };
        let moved = point + delta;
        harness.run(vec![Event::PointerMoved(moved)]);
        assert!(harness.dockspace.has_active_gesture());
        let result = harness.dockspace.select_item_current(F).unwrap();
        assert!(matches!(result.status(), DockspaceActionStatus::Applied(_)));
        let version = harness.dockspace.version();
        harness.run(Vec::new());
        harness.run(vec![button(moved, false)]);
        harness.settle();
        assert_eq!(harness.dockspace.version(), version);
        assert!(
            harness
                .dockspace
                .view()
                .node(harness.grid)
                .unwrap()
                .grid()
                .unwrap()
                .shares()
                .is_none()
        );
        harness.assert_tabs(harness.cells[3], &[E.into(), harness.tools], harness.tools);
        harness.assert_random_invariants();
        let (_, point) = harness.begin_grid_resize(orientation);
        harness.run(vec![Event::PointerMoved(point + delta)]);
        harness.release(point + delta);
        assert!(
            harness
                .dockspace
                .view()
                .node(harness.grid)
                .unwrap()
                .grid()
                .unwrap()
                .shares()
                .is_some()
        );
        assert_ne!(harness.dockspace.version(), version);
        harness.assert_random_invariants();
    }
}

#[test]
fn a_layout_reset_during_docking_or_resize_rejects_the_old_release_and_accepts_new_input() {
    for resize in [false, true] {
        let mut harness = GridHarness::new();
        let layout = harness.dockspace.view().to_layout();
        let point = if resize {
            let (_, point) = harness.begin_grid_resize(Orientation::Vertical);
            let moved = point + vec2(24.0, 0.0);
            harness.run(vec![Event::PointerMoved(moved)]);
            moved
        } else {
            harness.begin_drag(C.into());
            harness.hover_guide(A, DockspaceDropDirection::Center)
        };
        assert!(harness.dockspace.has_active_gesture());
        harness.dockspace.reset_layout(layout).unwrap();
        let version = harness.dockspace.version();
        harness.run(Vec::new());
        harness.run(vec![button(point, false)]);
        harness.settle();
        assert_eq!(harness.dockspace.version(), version);
        harness.assert_random_invariants();
        harness.click_tab(C.into());
        harness.begin_drag(C.into());
        let point = harness.hover_guide(A, DockspaceDropDirection::Center);
        harness.release(point);
        let tabs = harness.dockspace.view().item(C).unwrap().tabs().unwrap();
        assert_eq!(tabs.tabs(), &[A.into(), C.into()]);
        assert_eq!(tabs.selected(), Some(C.into()));
        harness.assert_random_invariants();
    }
}
