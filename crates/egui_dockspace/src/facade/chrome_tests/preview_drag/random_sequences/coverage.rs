//! Count actual drag phases and require admitted entries after generated edits.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragAttempt {
    Unavailable,
    CancelledBeforePreview,
    Finished(Finish),
}

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    requested: usize,
    unavailable: usize,
    acquired: usize,
    previewed: usize,
    delivered: usize,
    cancelled: usize,
}

impl Counts {
    fn record(&mut self, attempt: DragAttempt) {
        self.requested += 1;
        match attempt {
            DragAttempt::Unavailable => self.unavailable += 1,
            DragAttempt::CancelledBeforePreview => {
                self.acquired += 1;
                self.cancelled += 1;
            }
            DragAttempt::Finished(finish) => {
                self.acquired += 1;
                self.previewed += 1;
                if finish == Finish::Release {
                    self.delivered += 1;
                } else {
                    self.cancelled += 1;
                }
            }
        }
    }
}

fn direction_index(direction: DockspaceDropDirection) -> usize {
    match direction {
        DockspaceDropDirection::Center => 0,
        DockspaceDropDirection::Left => 1,
        DockspaceDropDirection::Right => 2,
        DockspaceDropDirection::Top => 3,
        DockspaceDropDirection::Bottom => 4,
    }
}

#[derive(Debug, Default)]
pub(super) struct DragCoverage {
    guides: [Counts; 5],
    gaps: Counts,
}

impl DragCoverage {
    pub(super) fn guide(&mut self, direction: DockspaceDropDirection, attempt: DragAttempt) {
        self.guides[direction_index(direction)].record(attempt);
    }

    pub(super) fn gap(&mut self, attempt: DragAttempt) {
        self.gaps.record(attempt);
    }

    pub(super) fn assert_requests(&self, actions: &[Action]) {
        let mut guides = [0; 5];
        let mut gaps = 0;
        for action in actions {
            match action {
                Action::Guide { direction, .. } => guides[direction_index(*direction)] += 1,
                Action::Gap { .. } => gaps += 1,
                _ => {}
            }
        }
        for (observed, expected) in self.guides.iter().zip(guides).chain([(&self.gaps, gaps)]) {
            assert_eq!(observed.requested, expected);
            assert_eq!(observed.unavailable + observed.acquired, expected);
            assert_eq!(observed.delivered + observed.cancelled, observed.acquired);
            assert!(observed.delivered <= observed.previewed);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        max_shrink_iters: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn an_admitted_drag_entry_must_complete_after_generated_visibility_and_selection_edits(
        prefix in proptest::collection::vec((any::<bool>(), any::<u8>()), 0..8),
    ) {
        // Run every entry for every prefix, rather than hoping a random selector chooses it.
        for entry in 0..6 {
            let mut harness = GridHarness::new();
            let mut trace = vec![format!("prefix={prefix:?}, required entry={entry}")];
            for &(visibility, selector) in &prefix {
                if visibility {
                    harness.random_visibility(selector, &mut trace);
                } else {
                    harness.random_click(TabSelector { prefer_layout: true, index: selector }, &mut trace);
                }
                harness.assert_random_invariants();
            }
            // Restore the declared prerequisite without replacing the edited layout.
            for selector in 0..5 {
                let item = [B, C, E, F, G][usize::from(selector)];
                if harness.expected_hidden.contains(&item) {
                    harness.random_visibility(selector, &mut trace);
                }
            }
            let selected = harness.dockspace.select_item_current(E).unwrap();
            assert!(matches!(selected.status(), DockspaceActionStatus::Applied(_)));
            harness.settle();
            let source = harness.tab_slots().into_iter().find(|slot| slot.key == E.into())
                .expect("the declared source is a direct real-item tab");
            let control = harness.panes.tab_controls[&source.key];
            prop_assert_eq!(harness.pointer_hits(control.drag.center()), (Some(control.id), Some(control.id)));
            let attempt = if entry == 5 {
                harness.random_gap(&source, 1, true, Finish::Release, &mut trace)
            } else {
                let direction = [
                    DockspaceDropDirection::Center,
                    DockspaceDropDirection::Left,
                    DockspaceDropDirection::Right,
                    DockspaceDropDirection::Top,
                    DockspaceDropDirection::Bottom,
                ][entry];
                harness.random_guide(&source, 0, direction, Finish::Release, &mut trace)
            };
            prop_assert_eq!(attempt, DragAttempt::Finished(Finish::Release), "{}", trace.join("\n"));
            harness.random_click(TabSelector { prefer_layout: false, index: 0 }, &mut trace);
            harness.assert_random_invariants();
        }
    }
}
