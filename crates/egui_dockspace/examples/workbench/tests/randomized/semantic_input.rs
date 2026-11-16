//! Mixed semantic input interleaved with the existing pointer journey grammar.

use egui::accesskit::{Action, ActionRequest, Orientation, TreeId};

use super::*;

impl Replay {
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
            for (widget, _) in self.controls(Role::Splitter, label) {
                let orientation = self
                    .output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == widget.accesskit_id())
                    .and_then(|(_, node)| node.orientation());
                if let Some(orientation) = orientation {
                    controls.push((widget, orientation));
                }
            }
        }
        let Some((target, orientation)) = self.pick(&controls) else {
            self.record("mixed adjustment", Attempt::NotApplicable);
            return;
        };
        let before = LayoutState::capture(self);
        let mut separate = self.at_current_layout();
        before.assert_equivalent_geometry(&LayoutState::capture(&separate));
        self.assert_same_materialized_panes(&separate);
        assert!(
            separate
                .output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|(id, _)| *id == target.accesskit_id()),
            "rebuilding the same logical layout must register the selected semantic control"
        );
        self.frame(vec![accesskit(target, Action::Focus)]);
        separate.frame(vec![accesskit(target, Action::Focus)]);
        assert_eq!(
            self.context.memory(egui::Memory::focused),
            Some(target),
            "the registered resize control must retain its accepted accessibility focus",
        );
        let (increment, decrement) = match orientation {
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
                    target,
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
        for event in events.iter().cloned() {
            separate.frame(vec![event]);
            separate.frame(Vec::new());
        }
        self.frame(events);
        assert!(self.context.input(|input| input.events.iter().all(|event| {
            !matches!(event, Event::Key { key, pressed: true, .. } if *key == increment || *key == decrement)
                && !matches!(event, Event::AccessKitActionRequest(request)
                    if request.target_node == target.accesskit_id()
                        && matches!(request.action, Action::Increment | Action::Decrement))
        })), "the actual focused control must consume every mixed adjustment");
        assert_eq!(self.context.memory(egui::Memory::focused), Some(target));
        assert!(!self.dock.has_active_gesture());
        self.settle();
        separate.settle();
        let after = LayoutState::capture(self);
        before.assert_same_content(&after);
        self.assert_same_materialized_panes(&separate);
        LayoutState::capture(&separate).assert_equivalent_geometry(&after);

        // Equivalence alone would accept two paths that both consume and ignore
        // input. A numeric control with room must also make an actual step.
        if let Some((value, minimum, maximum)) = self.numeric_range(target)
            && maximum - minimum > 1e-6
        {
            let increase = value < (minimum + maximum) * 0.5;
            self.frame(vec![
                accesskit(
                    target,
                    if increase {
                        Action::Increment
                    } else {
                        Action::Decrement
                    },
                ),
                key(if increase { increment } else { decrement }),
            ]);
            self.frame(Vec::new());
            let (actual, _, _) = self.numeric_range(target).unwrap();
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

fn clamped_splitter_fixture() -> (Replay, egui::Id) {
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
    let (target, _) = replay
        .controls(Role::Splitter, "Resize panes")
        .into_iter()
        .next()
        .expect("the clamped two-pane fixture must register its divider");
    let (value, minimum, maximum) = replay.numeric_range(target).unwrap();
    assert!(
        (value - minimum).abs() < 1e-8 && value > 0.1 && value < maximum,
        "the latent 10% share must be projected up to the content minimum"
    );
    replay.frame(vec![accesskit(target, Action::Focus)]);
    assert_eq!(replay.context.memory(egui::Memory::focused), Some(target));
    (replay, target)
}

#[test]
fn clamped_mixed_resize_round_trip_preserves_frame_independent_durable_shares() {
    let (mut batched, target) = clamped_splitter_fixture();
    let initial = LayoutState::capture(&batched);
    let mut separate = batched.at_current_layout();
    initial.assert_equivalent_geometry(&LayoutState::capture(&separate));
    batched.assert_same_materialized_panes(&separate);
    separate.frame(vec![accesskit(target, Action::Focus)]);
    let events = [
        accesskit(target, Action::Increment),
        key(egui::Key::ArrowLeft),
    ];
    for event in events.iter().cloned() {
        separate.frame(vec![event]);
        separate.settle();
    }
    batched.frame(events.into_iter().collect());
    batched.settle();
    batched.assert_same_materialized_panes(&separate);
    let final_state = LayoutState::capture(&batched);
    LayoutState::capture(&separate).assert_equivalent_geometry(&final_state);
    assert_ne!(
        final_state, initial,
        "an effective discrete round trip must record the measured shares, even at its original visible minimum"
    );
    let (value, _, _) = batched.numeric_range(target).unwrap();
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
        accesskit(target, Action::Decrement),
        key(egui::Key::ArrowLeft),
    ]);
    replay.settle();
    before.assert_equivalent_geometry(&LayoutState::capture(&replay));
}
