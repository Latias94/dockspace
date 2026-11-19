//! Short, shrinkable sizing journeys with an independent two-pane oracle.

use super::*;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

#[derive(Clone, Copy, Debug)]
enum Profile {
    Flexible,
    Capped,
    Fixed,
}

impl Profile {
    fn minimum(self) -> f64 {
        match self {
            Self::Flexible | Self::Capped => 1.0,
            Self::Fixed => 90.0,
        }
    }

    fn maximum(self) -> Option<f64> {
        match self {
            Self::Flexible => None,
            Self::Capped => Some(180.0),
            Self::Fixed => Some(90.0),
        }
    }

    fn preferred(self) -> f64 {
        match self {
            Self::Flexible => 240.0,
            Self::Capped => 140.0,
            Self::Fixed => 90.0,
        }
    }

    fn sizing(self) -> PaneSizing {
        PaneSizing::new(
            AxisSizing::new(self.minimum(), self.maximum(), Some(self.preferred())).unwrap(),
            AxisSizing::default(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    HostWidth(u16),
    Profile { third: bool, profile: Profile },
    Select { third: bool },
    ThirdVisible(bool),
    Adjust(SurfaceSplitterAdjustment),
}

fn actions() -> impl Strategy<Value = Action> {
    prop_oneof![
        prop::sample::select(vec![300_u16, 600, 1_000]).prop_map(Action::HostWidth),
        (
            any::<bool>(),
            prop::sample::select(vec![Profile::Flexible, Profile::Capped, Profile::Fixed]),
        )
            .prop_map(|(third, profile)| Action::Profile { third, profile }),
        any::<bool>().prop_map(|third| Action::Select { third }),
        any::<bool>().prop_map(Action::ThirdVisible),
        prop::sample::select(vec![
            SurfaceSplitterAdjustment::Preferred,
            SurfaceSplitterAdjustment::Increment,
            SurfaceSplitterAdjustment::Decrement,
            SurfaceSplitterAdjustment::Equalize,
        ])
        .prop_map(Action::Adjust),
    ]
}

struct Expected {
    width: f64,
    fraction: f64,
    selected: ItemId,
    third_visible: bool,
    profiles: [Profile; 2],
}

impl Expected {
    fn initial() -> Self {
        Self {
            width: 600.0,
            fraction: 0.5,
            selected: FIRST,
            third_visible: true,
            profiles: [Profile::Flexible, Profile::Capped],
        }
    }

    fn profile(&self) -> Profile {
        self.profiles[usize::from(self.selected == THIRD)]
    }

    fn clamp(&self, width: f64) -> f64 {
        let profile = self.profile();
        width.clamp(
            profile.minimum(),
            profile.maximum().unwrap_or(self.width - 1.0),
        )
    }

    fn first_width(&self) -> f64 {
        // With one unconstrained sibling, the sole bound simply clamps its peer.
        // This oracle does not traverse or solve Dockspace's layout tree.
        self.clamp(self.width * self.fraction)
    }

    fn hints(&self) -> [(ItemId, PaneSizing); 2] {
        [
            (FIRST, self.profiles[0].sizing()),
            (THIRD, self.profiles[1].sizing()),
        ]
    }

    fn bounds(&self) -> LogicalRect {
        LogicalRect::new(0.0, 0.0, self.width + 1.0, 401.0).unwrap()
    }

    fn assert_session(&self, session: &mut DockspaceSession) {
        let weights = split_weights(session);
        assert_eq!(weights.len(), 2);
        assert_close(weights[0] / (weights[0] + weights[1]), self.fraction);
        for (item, hidden, selected) in [
            (FIRST, false, self.selected == FIRST),
            (THIRD, !self.third_visible, self.selected == THIRD),
            (SECOND, false, true),
        ] {
            let location = session
                .view()
                .item(item)
                .expect("all three items stay owned");
            assert_eq!(location.root(), ROOT);
            assert_eq!(location.surface(), SURFACE);
            assert_eq!(location.is_hidden(), hidden);
            assert_eq!(location.is_active(), selected);
            let members = location
                .tabs()
                .expect("fixture item belongs to a tabs container")
                .items()
                .collect::<Vec<_>>();
            assert_eq!(
                members,
                if item == SECOND {
                    &[SECOND][..]
                } else {
                    &[FIRST, THIRD][..]
                }
            );
        }
        inspect_surface(session, |plan| {
            let panes = plan.panes().collect::<Vec<_>>();
            assert_eq!(panes.len(), 2);
            let first = panes
                .iter()
                .find(|pane| pane.item() == self.selected)
                .unwrap();
            let second = panes.iter().find(|pane| pane.item() == SECOND).unwrap();
            assert_close(first.bounds().x(), 0.0);
            assert_close(first.content_bounds().width(), self.first_width());
            assert_close(second.bounds().x(), self.first_width() + 1.0);
            assert_close(
                second.content_bounds().width(),
                self.width - self.first_width(),
            );
            let splitter = outer_splitter(plan);
            assert!(
                splitter.operable(),
                "an exact two-leaf partition stays operable"
            );
            let range = plan.splitter_resize_range(splitter.visual_id()).unwrap();
            assert_close(range.value(), self.first_width() / self.width);
            assert_close(range.minimum(), self.profile().minimum() / self.width);
            assert_close(
                range.maximum(),
                self.profile().maximum().unwrap_or(self.width - 1.0) / self.width,
            );
        });
    }

    fn apply(&mut self, session: &mut DockspaceSession, action: Action) {
        let old_version = session.version();
        let old_semantics = (self.fraction, self.selected, self.third_visible);
        let mut frame = session.begin_host_frame().unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let splitter = outer_splitter(plan);
        let stale = plan
            .prepare_splitter_adjustment(splitter.visual_id(), SurfaceSplitterAdjustment::Increment)
            .expect("fresh exact partition grants the adjustment");
        match action {
            Action::HostWidth(width) => self.width = f64::from(width),
            Action::Profile { third, profile } => self.profiles[usize::from(third)] = profile,
            Action::Select { third } if !third || self.third_visible => {
                self.selected = if third { THIRD } else { FIRST };
                frame.select_item_current(self.selected).unwrap();
            }
            Action::Select { .. } => {}
            Action::ThirdVisible(visible) => {
                self.third_visible = visible;
                if !visible && self.selected == THIRD {
                    self.selected = FIRST;
                }
                frame.set_item_visibility_current(THIRD, visible).unwrap();
            }
            Action::Adjust(adjustment) => {
                let before = self.first_width();
                let target = match adjustment {
                    SurfaceSplitterAdjustment::Preferred => self.profile().preferred(),
                    SurfaceSplitterAdjustment::Equalize => self.width * 0.5,
                    SurfaceSplitterAdjustment::Increment => before + 16.0,
                    SurfaceSplitterAdjustment::Decrement => before - 16.0,
                    _ => unreachable!("the generator uses four bounded adjustments"),
                };
                let after = self.clamp(target);
                if (after - before).abs() > 1.0e-9 {
                    self.fraction = after / self.width;
                }
                let prepared = plan
                    .prepare_splitter_adjustment(splitter.visual_id(), adjustment)
                    .unwrap();
                frame.submit_surface_action(prepared).unwrap();
            }
        }
        measure_at(&mut frame, &self.hints(), self.bounds());
        let report = frame.commit().unwrap();
        assert_eq!(
            report.workspace_changed(),
            old_semantics != (self.fraction, self.selected, self.third_visible),
            "semantic outcome for {action:?}"
        );
        self.assert_session(session);
        if session.version() != old_version {
            let version = session.version();
            let mut frame = session.begin_host_frame().unwrap();
            frame.submit_surface_action(stale).unwrap();
            measure_at(&mut frame, &self.hints(), self.bounds());
            let report = frame.commit().unwrap();
            assert!(matches!(
                report.inputs(),
                [HostInputOutcome::StaleRejected { .. }]
            ));
            assert!(!report.workspace_changed());
            assert_eq!(session.version(), version);
            self.assert_session(session);
        }
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1.0e-8, "{actual} != {expected}");
}

fn journey(actions: &[Action]) {
    let mut expected = Expected::initial();
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::tabs([FIRST, THIRD]),
            DockspaceNode::tabs([SECOND]),
        ],
    )
    .unwrap();
    let mut session = measured(content, &expected.hints());
    // The constructive prefix guarantees a real preferred resize, selection,
    // hidden restoration, fixed cap, and host resize in every generated case.
    for action in [
        Action::Adjust(SurfaceSplitterAdjustment::Preferred),
        Action::Select { third: true },
        Action::ThirdVisible(false),
        Action::Profile {
            third: true,
            profile: Profile::Fixed,
        },
        Action::ThirdVisible(true),
        Action::Select { third: true },
        Action::HostWidth(1_000),
    ]
    .into_iter()
    .chain(actions.iter().copied())
    .chain([
        Action::ThirdVisible(true),
        Action::Profile {
            third: false,
            profile: Profile::Flexible,
        },
        Action::Select { third: false },
        Action::Adjust(SurfaceSplitterAdjustment::Preferred),
    ]) {
        expected.apply(&mut session, action);
    }
    assert_close(expected.first_width(), 240.0);
}

#[test]
fn increasing_a_splitter_at_its_cap_after_host_resize_keeps_the_saved_weights() {
    journey(&[
        Action::HostWidth(600),
        Action::Profile {
            third: true,
            profile: Profile::Capped,
        },
        Action::Adjust(SurfaceSplitterAdjustment::Preferred),
        Action::Adjust(SurfaceSplitterAdjustment::Decrement),
        Action::Adjust(SurfaceSplitterAdjustment::Decrement),
        Action::HostWidth(1_000),
        Action::Adjust(SurfaceSplitterAdjustment::Increment),
    ]);
}

#[test]
fn hiding_and_remeasuring_a_capped_descendant_preserves_piecewise_contacts() {
    let column = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([THIRD])],
    )
    .unwrap();
    let content = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [column, DockspaceNode::tabs([SECOND])],
    )
    .unwrap();
    let mut hints = [
        (FIRST, width_sizing(Some(100.0), Some(120.0))),
        (THIRD, width_sizing(Some(200.0), None)),
    ];
    let mut session = measured(content, &hints);
    let assert_outer = |session: &mut DockspaceSession, width: f64, operable: bool| {
        inspect_surface(session, |plan| {
            let splitter = outer_splitter(plan);
            assert_close(splitter.before_bounds().width(), width);
            assert_close(splitter.after_bounds().width(), 600.0 - width);
            assert_eq!(splitter.operable(), operable);
            assert_eq!(
                plan.splitter_resize_range(splitter.visual_id()).is_some(),
                operable
            );
            assert_eq!(
                plan.prepare_splitter_adjustment(
                    splitter.visual_id(),
                    SurfaceSplitterAdjustment::Preferred
                )
                .is_some(),
                operable
            );
            assert_eq!(plan.receiver_for_splitter(splitter).is_some(), operable);
        });
    };
    assert_outer(&mut session, 300.0, true);
    let weights = all_split_weights(&session);
    let mut hide = session.begin_host_frame().unwrap();
    hide.set_item_visibility_current(FIRST, false).unwrap();
    measure(&mut hide, &hints);
    hide.commit().unwrap();
    assert_eq!(all_split_weights(&session), weights);
    assert_outer(&mut session, 300.0, true);
    apply(
        &mut session,
        &hints,
        &[SurfaceSplitterAdjustment::Preferred],
    );
    assert_outer(&mut session, 200.0, true);

    for maximum in [400.0, 120.0, 400.0] {
        hints[0].1 = width_sizing(Some(100.0), Some(maximum));
        let weights = all_split_weights(&session);
        let mut show = session.begin_host_frame().unwrap();
        show.set_item_visibility_current(FIRST, true).unwrap();
        measure(&mut show, &hints);
        show.commit().unwrap();
        assert_eq!(all_split_weights(&session), weights);
        assert_outer(&mut session, 200.0, true);
        inspect_surface(&mut session, |plan| {
            let first = plan.panes().find(|pane| pane.item() == FIRST).unwrap();
            assert_close(first.content_bounds().width(), 200.0_f64.min(maximum));
        });
    }
}

#[test]
fn fresh_measurements_revoke_old_adjustments_without_changing_the_workspace_version() {
    for (width, maximum, expected_width) in [(601.0, Some(220.0), 220.0), (901.0, None, 450.0)] {
        let original = [(FIRST, width_sizing(Some(200.0), None))];
        let mut session = pair_session([1.0, 1.0], &original);
        let stale = inspect_surface(&mut session, |plan| {
            plan.prepare_splitter_adjustment(
                outer_splitter(plan).visual_id(),
                SurfaceSplitterAdjustment::Preferred,
            )
            .unwrap()
        });
        let version = session.version();
        let weights = split_weights(&session);
        let hints = [(FIRST, width_sizing(Some(200.0), maximum))];
        let bounds = LogicalRect::new(0.0, 0.0, width, 601.0).unwrap();
        let mut update = session.begin_host_frame().unwrap();
        measure_at(&mut update, &hints, bounds);
        assert!(!update.commit().unwrap().workspace_changed());
        assert_eq!(session.version(), version);

        let mut replay = session.begin_host_frame().unwrap();
        replay.submit_surface_action(stale).unwrap();
        measure_at(&mut replay, &hints, bounds);
        let report = replay.commit().unwrap();
        assert!(!report.workspace_changed());
        assert!(report.manipulation_feedback().iter().all(|feedback| {
            !matches!(feedback.status(), DockspaceManipulationStatus::Delivered(_))
        }));
        assert_eq!(session.version(), version);
        assert_eq!(split_weights(&session), weights);
        assert_pair(&mut session, (expected_width, width - 1.0 - expected_width));
        assert!(!session.has_active_gesture());

        let mut fresh = session.begin_host_frame().unwrap();
        let plan = fresh.paint_plan(SURFACE).unwrap().unwrap();
        let action = plan
            .prepare_splitter_adjustment(
                outer_splitter(plan).visual_id(),
                SurfaceSplitterAdjustment::Preferred,
            )
            .expect("fresh measurements restore adjustment authority");
        fresh.submit_surface_action(action).unwrap();
        measure_at(&mut fresh, &hints, bounds);
        assert!(fresh.commit().unwrap().workspace_changed());
        assert_pair(&mut session, (200.0, width - 201.0));
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions"))),
        ..ProptestConfig::with_cases(128)
    })]

    #[test]
    fn short_sizing_journeys_preserve_geometry_ownership_and_action_authority(
        trace in prop::collection::vec(actions(), 0..25),
    ) {
        journey(&trace);
    }
}
