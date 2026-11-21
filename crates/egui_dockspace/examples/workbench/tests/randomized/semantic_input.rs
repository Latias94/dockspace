//! Mixed semantic input interleaved with the existing pointer journey grammar.

use egui::accesskit::{Action, ActionRequest, Orientation, TreeId};

use super::*;

#[derive(Clone, Copy, Debug)]
struct ResizeControl {
    id: egui::Id,
    label: &'static str,
    orientation: Orientation,
    bounds: Rect,
}

impl Replay {
    fn resize_controls(&self, label: &'static str) -> Vec<ResizeControl> {
        let nodes = &self
            .output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let mut controls = self
            .controls(Role::Splitter, label)
            .into_iter()
            .filter_map(|(id, bounds)| {
                let (_, node) = nodes.iter().find(|(node, _)| *node == id.accesskit_id())?;
                Some(ResizeControl {
                    id,
                    label,
                    orientation: node.orientation()?,
                    bounds,
                })
            })
            .collect::<Vec<_>>();
        // Seeded choice follows position within each semantic control role.
        controls.sort_unstable_by(|left, right| {
            let a = left.bounds;
            let b = right.bounds;
            a.left()
                .total_cmp(&b.left())
                .then_with(|| a.top().total_cmp(&b.top()))
                .then_with(|| a.right().total_cmp(&b.right()))
                .then_with(|| a.bottom().total_cmp(&b.bottom()))
                .then_with(|| {
                    (left.orientation == Orientation::Vertical)
                        .cmp(&(right.orientation == Orientation::Vertical))
                })
        });
        controls
    }

    fn resize_counterpart(&self, target: ResizeControl, separate: &Self) -> ResizeControl {
        let mut matches = separate
            .resize_controls(target.label)
            .into_iter()
            .filter(|control| {
                let source = target.bounds;
                let candidate = control.bounds;
                control.orientation == target.orientation
                    && [source.left(), source.top(), source.right(), source.bottom()]
                        .into_iter()
                        .zip([
                            candidate.left(),
                            candidate.top(),
                            candidate.right(),
                            candidate.bottom(),
                        ])
                        .all(|(a, b)| (a - b).abs() < 1e-3)
            });
        let counterpart = matches
            .next()
            .unwrap_or_else(|| panic!("the rebuilt layout must expose {target:?}"));
        assert!(
            matches.next().is_none(),
            "the matching resize control must be unique: {target:?}"
        );
        match (
            self.numeric_range(target.id),
            separate.numeric_range(counterpart.id),
        ) {
            (Some((value, min, max)), Some((other_value, other_min, other_max))) => {
                for (a, b) in [(value, other_value), (min, other_min), (max, other_max)] {
                    assert!(
                        (a - b).abs() < 1e-8,
                        "initial resize range for {target:?}: {a} != {b}"
                    );
                }
            }
            (None, None) => {}
            (source, counterpart) => panic!(
                "initial resize range missing on one path for {target:?}: {source:?} vs {counterpart:?}"
            ),
        }
        counterpart
    }

    fn focus_resize_control(&mut self, control: ResizeControl) {
        self.frame(vec![accesskit(control.id, Action::Focus)]);
        assert_eq!(
            self.context.memory(egui::Memory::focused),
            Some(control.id),
            "the registered resize control {control:?} must accept accessibility focus"
        );
    }

    fn assert_same_materialized_panes(&self, separate: &Self) {
        let expected = self.selected_rects();
        let actual = separate.selected_rects();
        assert_eq!(
            actual.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>()
        );
        for (item, rect) in actual {
            let expected = expected[&item];
            for (actual, expected) in [rect.left(), rect.top(), rect.right(), rect.bottom()]
                .into_iter()
                .zip([
                    expected.left(),
                    expected.top(),
                    expected.right(),
                    expected.bottom(),
                ])
            {
                assert!(
                    (actual - expected).abs() < 1e-3,
                    "counterpart pane {item} differs at the compared checkpoint: {rect:?} versus {expected:?}"
                );
            }
        }
    }

    pub(super) fn adjustment_batch(&mut self) {
        self.settle();
        let mut controls = Vec::new();
        for label in [
            "Resize panes",
            "Resize pane grid horizontally",
            "Resize pane grid vertically",
            "Resize floating right edge",
            "Resize floating left edge",
            "Resize floating top edge",
            "Resize floating bottom edge",
        ] {
            controls.extend(self.resize_controls(label));
        }
        let Some(target) = self.pick(&controls) else {
            self.record("mixed adjustment", Attempt::NotApplicable);
            return;
        };
        let before = LayoutState::capture(self);
        let mut separate = self.at_current_layout();
        before.assert_equivalent_geometry(&LayoutState::capture(&separate));
        self.assert_same_materialized_panes(&separate);
        let counterpart = self.resize_counterpart(target, &separate);
        self.focus_resize_control(target);
        separate.focus_resize_control(counterpart);
        let (increment, decrement) = match target.orientation {
            Orientation::Vertical => (egui::Key::ArrowRight, egui::Key::ArrowLeft),
            Orientation::Horizontal => (egui::Key::ArrowDown, egui::Key::ArrowUp),
        };
        let count = self.pick(&[2, 3, 4, 5]).unwrap();
        let mut events = Vec::with_capacity(count + 1);
        // Always include both producers; subsequent steps vary repetitions,
        // reversals and which producer starts the packet.
        for index in 0..count {
            let increase = self.pick(&[false, true]).unwrap();
            let event = if index % 2 == 0 {
                accesskit(
                    target.id,
                    if increase {
                        Action::Increment
                    } else {
                        Action::Decrement
                    },
                )
            } else {
                key(if increase { increment } else { decrement })
            };
            events.push(event);
        }
        if self.pick(&[false, true]).unwrap() {
            events.reverse();
        }
        self.note(format!(
            "mixed resize batch target={target:?}, events={events:?}"
        ));
        for mut event in events.iter().cloned() {
            if let Event::AccessKitActionRequest(request) = &mut event {
                request.target_node = counterpart.id.accesskit_id();
            }
            separate.frame(vec![event]);
            separate.frame(Vec::new());
            assert_eq!(
                separate.context.memory(egui::Memory::focused),
                Some(counterpart.id)
            );
        }
        self.frame(events);
        assert!(self.context.input(|input| input.events.iter().all(|event| {
            !matches!(event, Event::Key { key, pressed: true, .. } if *key == increment || *key == decrement)
                && !matches!(event, Event::AccessKitActionRequest(request)
                    if request.target_node == target.id.accesskit_id()
                        && matches!(request.action, Action::Increment | Action::Decrement))
        })), "the actual focused control must consume every mixed adjustment");
        assert_eq!(self.context.memory(egui::Memory::focused), Some(target.id));
        assert!(!self.dock.has_active_gesture());
        self.settle();
        separate.settle();
        let after = LayoutState::capture(self);
        before.assert_same_content(&after);
        self.assert_same_materialized_panes(&separate);
        LayoutState::capture(&separate).assert_equivalent_geometry(&after);

        // Equivalence alone would accept two paths that both consume and ignore
        // input. A numeric control with room must also make an actual step.
        if let Some((value, minimum, maximum)) = self.numeric_range(target.id)
            && maximum - minimum > 1e-6
        {
            let increase = value < (minimum + maximum) * 0.5;
            self.frame(vec![
                accesskit(
                    target.id,
                    if increase {
                        Action::Increment
                    } else {
                        Action::Decrement
                    },
                ),
                key(if increase { increment } else { decrement }),
            ]);
            self.frame(Vec::new());
            let (actual, _, _) = self.numeric_range(target.id).unwrap();
            assert!(
                (actual - value) * if increase { 1.0 } else { -1.0 } > 1e-7,
                "a numeric control with room must apply its admitted adjustments: {value} -> {actual}"
            );
            after.assert_same_content(&LayoutState::capture(self));
        }
        self.adjustment_batches += 1;
        self.record("mixed adjustment", Attempt::Performed);
        self.frame(Vec::new());
    }
}

pub(super) fn accesskit(target: egui::Id, action: Action) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target.accesskit_id(),
        data: None,
    })
}

pub(super) fn key(key: egui::Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn clamped_splitter_fixture() -> (Replay, ResizeControl) {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::tabs([EDITOR]), 0.1),
                    (DockspaceNode::tabs([PREVIEW]), 0.9),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut replay = Replay::from_layout(0xc1a0_0ed, layout);
    replay.settle();
    let target = replay
        .resize_controls("Resize panes")
        .into_iter()
        .next()
        .expect("the clamped two-pane fixture must register its divider");
    let (value, minimum, maximum) = replay.numeric_range(target.id).unwrap();
    assert!(
        (value - minimum).abs() < 1e-8 && value > 0.1 && value < maximum,
        "the latent 10% share must be projected up to the content minimum"
    );
    replay.focus_resize_control(target);
    (replay, target)
}

#[test]
fn clamped_mixed_resize_round_trip_preserves_frame_independent_durable_shares() {
    let (mut batched, target) = clamped_splitter_fixture();
    let initial = LayoutState::capture(&batched);
    let mut separate = batched.at_current_layout();
    initial.assert_equivalent_geometry(&LayoutState::capture(&separate));
    batched.assert_same_materialized_panes(&separate);
    let counterpart = batched.resize_counterpart(target, &separate);
    separate.focus_resize_control(counterpart);
    let events = |target| {
        [
            accesskit(target, Action::Increment),
            key(egui::Key::ArrowLeft),
        ]
    };
    for event in events(counterpart.id) {
        separate.frame(vec![event]);
        separate.settle();
    }
    batched.frame(events(target.id).into());
    batched.settle();
    batched.assert_same_materialized_panes(&separate);
    let final_state = LayoutState::capture(&batched);
    LayoutState::capture(&separate).assert_equivalent_geometry(&final_state);
    assert_ne!(
        final_state, initial,
        "an effective discrete round trip must record the measured shares, even at its original visible minimum"
    );
    let (value, _, _) = batched.numeric_range(target.id).unwrap();
    let first_share = batched
        .dock
        .view()
        .root(MAIN_ROOT)
        .unwrap()
        .content()
        .unwrap()
        .split()
        .unwrap()
        .weights()
        .next()
        .unwrap();
    assert!(
        (first_share - value).abs() < 1e-8,
        "the first effective resize fixes durable intent to the actually reached extent"
    );
}

#[test]
fn outward_resize_at_a_clamped_minimum_preserves_latent_shares() {
    let (mut replay, target) = clamped_splitter_fixture();
    let before = LayoutState::capture(&replay);
    replay.frame(vec![
        accesskit(target.id, Action::Decrement),
        key(egui::Key::ArrowLeft),
    ]);
    replay.settle();
    before.assert_equivalent_geometry(&LayoutState::capture(&replay));
}

#[test]
fn junction_axis_identity_survives_tab_overflow_and_opposite_adjustments() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
        ItemId,
    };

    let columns = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (
                DockspaceNode::tabs_with_selection([ItemId::new(6), PREVIEW], Some(PREVIEW))
                    .unwrap(),
                0.6374902267396404,
            ),
            (
                DockspaceNode::tabs_with_selection(
                    [PROFILER, ItemId::new(8), INSPECTOR, EDITOR],
                    Some(EDITOR),
                )
                .unwrap(),
                0.18125488663017983,
            ),
            (DockspaceNode::tabs([OUTLINE]), 0.18125488663017983),
        ],
    )
    .unwrap();
    let rows = DockspaceNode::split(
        DockspaceAxis::Vertical,
        [
            (DockspaceNode::tabs([CONSOLE]), 0.6786381842456609),
            (columns, 0.32136181575433914),
        ],
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(MAIN_ROOT, rows),
    )])
    .unwrap();
    let mut batched = Replay::from_layout(0x5eed, layout);
    batched.settle();
    let target = batched
        .resize_controls("Resize pane grid horizontally")
        .into_iter()
        .max_by(|a, b| a.bounds.center().x.total_cmp(&b.bounds.center().x))
        .expect("the row and three columns must expose their right junction");
    let mut separate = batched.at_current_layout();
    let initial = LayoutState::capture(&batched);
    initial.assert_equivalent_geometry(&LayoutState::capture(&separate));
    batched.assert_same_materialized_panes(&separate);
    let counterpart = batched.resize_counterpart(target, &separate);
    batched.focus_resize_control(target);
    separate.focus_resize_control(counterpart);
    // The first step crosses a tab overflow threshold. A sibling chrome change
    // must not replace the junction axis's input or accessibility identity.
    let events = |target| {
        [
            key(egui::Key::ArrowLeft),
            accesskit(target, Action::Increment),
        ]
    };
    for event in events(counterpart.id) {
        separate.frame(vec![event]);
        assert!(
            separate.context.input(|input| input.events.is_empty()),
            "the focused junction must consume each adjustment after its sibling chrome changes"
        );
        separate.frame(Vec::new());
        assert!(
            separate
                .resize_controls(counterpart.label)
                .iter()
                .any(|widget| widget.id == counterpart.id),
            "tab overflow changes must preserve the junction axis identity"
        );
        assert_eq!(
            separate.context.memory(egui::Memory::focused),
            Some(counterpart.id)
        );
    }
    batched.frame(events(target.id).into());
    assert_eq!(
        batched.context.memory(egui::Memory::focused),
        Some(target.id)
    );
    batched.settle();
    separate.settle();
    batched.assert_same_materialized_panes(&separate);
    initial.assert_equivalent_geometry(&LayoutState::capture(&batched));
    initial.assert_equivalent_geometry(&LayoutState::capture(&separate));
}
