//! Direct manipulation checks at move and release checkpoints.

use super::*;

impl Replay {
    pub(super) fn resize(&mut self, junction: bool) -> Attempt {
        self.settle();
        let (role, label) = if junction {
            (Role::Group, "Resize pane grid")
        } else {
            (Role::Splitter, "Resize panes")
        };
        let mut control = None;
        for _ in 0..16 {
            let choices = self.controls(role, label);
            if let Some((widget, bounds)) = self.pick(&choices) {
                let point = bounds.center();
                self.frame(vec![Event::PointerMoved(point)]);
                if self
                    .context
                    .viewport(|viewport| viewport.hits.drag.is_some_and(|hit| hit.id == widget))
                {
                    control = Some((widget, bounds));
                    break;
                }
            } else {
                self.frame(Vec::new());
            }
        }
        let Some((widget, bounds)) = control else {
            self.note(format!("no exposed {label}"));
            return self.record("resize", Attempt::Blocked);
        };
        let before = LayoutState::capture(self);
        let before_rects = self.selected_rects();
        let range = self.numeric_range(widget);
        let point = bounds.center();
        let distance = self.pick(&[-36.0, -20.0, 20.0, 36.0]).unwrap();
        let delta = if junction {
            egui::vec2(distance, -distance * 0.75)
        } else if bounds.width() < bounds.height() {
            egui::vec2(distance, 0.0)
        } else {
            egui::vec2(0.0, distance)
        };
        self.note(format!("drag {label} at {point:?} by {delta:?}"));
        self.frame(vec![Event::PointerMoved(point), button(point, true)]);
        assert!(
            self.dock.has_active_gesture(),
            "the exposed resize control must acquire its gesture"
        );
        self.frame(vec![Event::PointerMoved(point + delta)]);
        let during_rects = self.selected_rects();
        self.frame(vec![button(point + delta, false)]);
        self.frame(Vec::new());
        assert!(
            !self.dock.has_active_gesture(),
            "releasing a divider must finish direct manipulation"
        );
        let after = LayoutState::capture(self);
        before.assert_same_content(&after);
        for (item, rect) in self.selected_rects() {
            let during = during_rects[&item];
            for (actual, expected) in [rect.left(), rect.top(), rect.right(), rect.bottom()]
                .into_iter()
                .zip([during.left(), during.top(), during.right(), during.bottom()])
            {
                assert!(
                    (actual - expected).abs() < 1e-3,
                    "release at the last move point must preserve the sampled resize geometry"
                );
            }
        }
        if let Some((value, minimum, maximum)) = range {
            let (actual, _, _) = self
                .numeric_range(widget)
                .expect("the same splitter survives a resize");
            let displacement = if bounds.width() < bounds.height() {
                delta.x
            } else {
                delta.y
            };
            let has_room = if displacement > 0.0 {
                value < maximum - 1e-7
            } else {
                value > minimum + 1e-7
            };
            if has_room {
                assert!(
                    (actual - value) * f64::from(displacement) > 1e-7,
                    "a movable splitter must follow the pointer: {value} -> {actual}, delta={displacement}"
                );
            } else {
                assert!(
                    (actual - value).abs() < 1e-5,
                    "an outward resize at its limit must stay clamped"
                );
            }
            let horizontal = bounds.width() < bounds.height();
            for (item, rect) in self.selected_rects() {
                let old = before_rects[&item];
                let (actual, expected) = if horizontal {
                    ([rect.top(), rect.bottom()], [old.top(), old.bottom()])
                } else {
                    ([rect.left(), rect.right()], [old.left(), old.right()])
                };
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 1e-3,
                        "single-axis resize changed the perpendicular pane extent"
                    );
                }
            }
        }
        self.record("resize", Attempt::Performed)
    }

    pub(super) fn selected_rects(
        &self,
    ) -> std::collections::BTreeMap<egui_dockspace::ItemId, Rect> {
        self.panes
            .panes
            .iter()
            .filter_map(|(&item, pane)| {
                self.dock
                    .view()
                    .item(item)
                    .filter(|view| view.is_active())
                    .and_then(|_| pane.content.map(|rect| (item, rect)))
            })
            .collect()
    }
}

fn grid_fixture(seed: u64) -> Replay {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    };
    let column = |top, bottom| {
        DockspaceNode::equal_split(
            DockspaceAxis::Vertical,
            [DockspaceNode::tabs([top]), DockspaceNode::tabs([bottom])],
        )
        .unwrap()
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [column(EDITOR, PREVIEW), column(CONSOLE, PROFILER)],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut replay = Replay::from_layout(seed, layout);
    replay.settle();
    replay
}

fn floating_fixture(seed: u64) -> Replay {
    use egui_dockspace::{
        DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
        DockspaceSurfaceLayout, FloatingPresentationId, LogicalRect, RootId,
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(MAIN_ROOT, DockspaceNode::tabs([EDITOR])),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([INSPECTOR])),
        LogicalRect::new(300.0, 240.0, 400.0, 320.0).unwrap(),
    ))])
    .unwrap();
    let mut replay = Replay::from_layout(seed, layout);
    replay.settle();
    replay
}

fn axis_changes(
    before: &std::collections::BTreeMap<egui_dockspace::ItemId, Rect>,
    after: &std::collections::BTreeMap<egui_dockspace::ItemId, Rect>,
) -> (bool, bool) {
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    let mut changed = (false, false);
    for (item, actual) in after {
        let expected = before[item];
        changed.0 |= (actual.left() - expected.left()).abs() > 1e-3
            || (actual.right() - expected.right()).abs() > 1e-3;
        changed.1 |= (actual.top() - expected.top()).abs() > 1e-3
            || (actual.bottom() - expected.bottom()).abs() > 1e-3;
    }
    changed
}

#[test]
fn required_divider_and_junction_drags_change_their_admitted_geometry() {
    for junction in [false, true] {
        let mut replay = grid_fixture(0xd1a1_de00 + u64::from(junction));
        let before = replay.selected_rects();
        assert_eq!(
            replay.resize(junction),
            Attempt::Performed,
            "the roomy unobscured grid must expose its required resize entry"
        );
        let changed = axis_changes(&before, &replay.selected_rects());
        if junction {
            assert_eq!(
                changed,
                (true, true),
                "the admitted corner drag must move both axes"
            );
        } else {
            assert_ne!(
                changed.0, changed.1,
                "a divider drag must move exactly one axis"
            );
        }
    }
}

#[test]
fn required_junction_semantic_axes_change_only_the_requested_axis() {
    use super::semantic_input::{accesskit, key};
    for (horizontal, label, arrow) in [
        (true, "Resize pane grid horizontally", egui::Key::ArrowRight),
        (false, "Resize pane grid vertically", egui::Key::ArrowDown),
    ] {
        let mut replay = grid_fixture(0xa715_0000 + u64::from(horizontal));
        let (target, _) = replay
            .controls(Role::Splitter, label)
            .into_iter()
            .next()
            .expect("the required junction axis control must be registered");
        let before = replay.selected_rects();
        let content = LayoutState::capture(&replay);
        replay.frame(vec![accesskit(target, egui::accesskit::Action::Focus)]);
        assert_eq!(replay.context.memory(egui::Memory::focused), Some(target));
        replay.frame(vec![
            accesskit(target, egui::accesskit::Action::Increment),
            key(arrow),
        ]);
        replay.settle();
        content.assert_same_content(&LayoutState::capture(&replay));
        assert_eq!(
            axis_changes(&before, &replay.selected_rects()),
            (horizontal, !horizontal),
            "the roomy junction must apply both producers on exactly the requested axis"
        );
        let after = replay.selected_rects();
        let displacement = if horizontal {
            after[&EDITOR].width() - before[&EDITOR].width()
        } else {
            after[&EDITOR].height() - before[&EDITOR].height()
        };
        let expected = 2.0 * replay.dock.style().splitter_keyboard_step;
        assert!(
            (displacement - expected).abs() < 1e-3,
            "the junction must apply both admitted producers: expected {expected}, got {displacement}"
        );
    }
}

#[test]
fn required_floating_edges_move_for_pointer_and_semantic_input() {
    use super::semantic_input::{accesskit, key};
    use egui_dockspace::FloatingPresentationId;
    for (index, label, delta, arrow) in [
        (
            0,
            "Resize floating right edge",
            egui::vec2(24.0, 0.0),
            egui::Key::ArrowRight,
        ),
        (
            1,
            "Resize floating left edge",
            egui::vec2(-24.0, 0.0),
            egui::Key::ArrowLeft,
        ),
        (
            2,
            "Resize floating top edge",
            egui::vec2(0.0, -24.0),
            egui::Key::ArrowUp,
        ),
        (
            3,
            "Resize floating bottom edge",
            egui::vec2(0.0, 24.0),
            egui::Key::ArrowDown,
        ),
    ] {
        for pointer in [false, true] {
            let mut replay = floating_fixture(0xed9e_0000 + index);
            let floating = FloatingPresentationId::new(1);
            let before = replay.dock.view().contained(floating).unwrap().rect();
            let content = LayoutState::capture(&replay);
            let main = replay.panes.panes[&EDITOR].content.unwrap();
            let (target, bounds) = replay
                .controls(Role::Splitter, label)
                .into_iter()
                .next()
                .expect("the roomy floating fixture must expose every cardinal resize edge");
            if pointer {
                let point = bounds.center();
                replay.frame(vec![Event::PointerMoved(point)]);
                assert!(
                    replay.context.viewport(|viewport| viewport
                        .hits
                        .drag
                        .is_some_and(|hit| hit.id == target)),
                    "the required floating edge must be the actual exposed drag receiver"
                );
                replay.frame(vec![button(point, true)]);
                assert!(replay.dock.has_active_gesture());
                replay.frame(vec![Event::PointerMoved(point + delta)]);
                replay.frame(vec![button(point + delta, false)]);
            } else {
                replay.frame(vec![accesskit(target, egui::accesskit::Action::Focus)]);
                assert_eq!(replay.context.memory(egui::Memory::focused), Some(target));
                let action = if delta.x + delta.y > 0.0 {
                    egui::accesskit::Action::Increment
                } else {
                    egui::accesskit::Action::Decrement
                };
                replay.frame(vec![accesskit(target, action), key(arrow)]);
            }
            replay.settle();
            let after = replay.dock.view().contained(floating).unwrap().rect();
            let horizontal = delta.x != 0.0;
            let (changed, unchanged) = if horizontal {
                (
                    after.width() - before.width(),
                    after.height() - before.height(),
                )
            } else {
                (
                    after.height() - before.height(),
                    after.width() - before.width(),
                )
            };
            let distance = if pointer {
                24.0
            } else {
                2.0 * f64::from(replay.dock.style().splitter_keyboard_step)
            };
            assert!(
                (changed - distance).abs() < 1e-4,
                "{label}, pointer={pointer}: the edge must grow by {distance}, got {before:?} -> {after:?}"
            );
            assert!(
                unchanged.abs() < 1e-9,
                "a cardinal resize preserves the other extent"
            );
            let (expected_x, expected_y) = match label {
                "Resize floating left edge" => (before.x() - distance, before.y()),
                "Resize floating top edge" => (before.x(), before.y() - distance),
                _ => (before.x(), before.y()),
            };
            assert!(
                (after.x() - expected_x).abs() < 1e-4 && (after.y() - expected_y).abs() < 1e-4,
                "{label} must preserve its opposite anchor: {before:?} -> {after:?}"
            );
            assert_eq!(
                replay.panes.panes[&EDITOR].content,
                Some(main),
                "resizing a floating root must not move main content"
            );
            content.assert_same_content(&LayoutState::capture(&replay));
            assert!(!replay.dock.has_active_gesture());
        }
    }
}
