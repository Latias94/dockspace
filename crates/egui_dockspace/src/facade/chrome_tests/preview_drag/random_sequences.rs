//! Bounded generated input journeys reuse the shared Grid pointer harness.

use proptest::prelude::*;

use super::*;
use crate::{DockspaceActionOutcome, DockspaceActionStatus, DockspaceNodeView};

mod coverage;
mod external_edits;

use coverage::{DragAttempt, DragCoverage};

#[derive(Clone, Debug)]
struct TabSlot {
    key: TabKey,
    owner: NodeKey,
    root: RootId,
    roster: Vec<TabKey>,
    items: Vec<ItemId>,
    owner_items: Vec<ItemId>,
}

fn collect_tab_slots(node: DockspaceNodeView<'_>, slots: &mut Vec<TabSlot>) {
    if let Some(tabs) = node.tabs() {
        for key in tabs.tabs() {
            let child = tabs.child(*key).unwrap();
            let items = match child {
                DockspaceTabView::Item(item) => vec![item],
                DockspaceTabView::Layout(layout) => layout.items().collect(),
            };
            slots.push(TabSlot {
                key: *key,
                owner: node.key(),
                root: node.root().unwrap().id(),
                roster: tabs.tabs().to_vec(),
                items,
                owner_items: node.items().collect(),
            });
            if let DockspaceTabView::Layout(layout) = child {
                collect_tab_slots(layout, slots);
            }
        }
    } else if let Some(split) = node.split() {
        for child in split.children() {
            collect_tab_slots(child, slots);
        }
    } else if let Some(grid) = node.grid() {
        for child in grid.children() {
            collect_tab_slots(child, slots);
        }
    }
}

fn structural_children(node: DockspaceNodeView<'_>) -> Vec<DockspaceNodeView<'_>> {
    if let Some(tabs) = node.tabs() {
        tabs.children()
            .filter_map(|child| match child {
                DockspaceTabView::Item(_) => None,
                DockspaceTabView::Layout(layout) => Some(layout),
            })
            .collect()
    } else if let Some(split) = node.split() {
        split.children().collect()
    } else if let Some(grid) = node.grid() {
        grid.children().collect()
    } else {
        Vec::new()
    }
}

fn item_owner_path(node: DockspaceNodeView<'_>, item: ItemId, path: &mut Vec<NodeKey>) -> bool {
    path.push(node.key());
    if node.item() == Some(item)
        || node
            .tabs()
            .is_some_and(|tabs| tabs.items().any(|value| value == item))
    {
        return true;
    }
    for child in structural_children(node) {
        if item_owner_path(child, item, path) {
            return true;
        }
    }
    path.pop();
    false
}

fn append_layout_signature(node: DockspaceNodeView<'_>, signature: &mut Vec<String>) {
    signature.push(format!("{:?}: {:?}", node.key(), node.layout_title()));
    if let Some(item) = node.item() {
        signature.push(format!("pane {item:?}"));
    } else if let Some(tabs) = node.tabs() {
        signature.push(format!(
            "tabs {:?}, selected {:?}",
            tabs.tabs(),
            tabs.selected()
        ));
        for child in tabs.children() {
            if let DockspaceTabView::Layout(child) = child {
                append_layout_signature(child, signature);
            }
        }
    } else if let Some(split) = node.split() {
        signature.push(format!(
            "split {:?} {:?}",
            split.axis(),
            split.weights().collect::<Vec<_>>()
        ));
        for child in split.children() {
            append_layout_signature(child, signature);
        }
    } else if let Some(grid) = node.grid() {
        signature.push(format!("grid {:?} {:?}", grid.shape(), grid.shares()));
        for child in grid.children() {
            append_layout_signature(child, signature);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TabSelector {
    prefer_layout: bool,
    index: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finish {
    Release,
    Escape,
    PointerGone,
    OutsideRelease,
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Click(TabSelector),
    ToggleVisibility(u8),
    Guide {
        source: TabSelector,
        target: u8,
        direction: DockspaceDropDirection,
        finish: Finish,
    },
    Gap {
        source: TabSelector,
        target: u8,
        after: bool,
        finish: Finish,
    },
    Resize {
        boundary: u8,
        positive: bool,
    },
}

fn tab_gap_point(control: TabControl, after: bool) -> Option<Pos2> {
    // Full tab tails can lie under close or scroll controls. Stay within the
    // requested insertion half and let the completed hit graph prove exposure.
    let mut half = control.rect;
    if after {
        half.min.x = control.rect.center().x;
    } else {
        half.max.x = control.rect.center().x;
    }
    let exposed = half.intersect(control.drag);
    exposed.is_positive().then(|| {
        egui::pos2(
            exposed.left() + 1.0_f32.min(exposed.width() * 0.5),
            exposed.center().y,
        )
    })
}

fn guide_hit_bounds(guide: &DropGuide, style: &DockStyle) -> Rect {
    let scale = guide.rect.width() / style.drop_guide_extent;
    guide.rect.expand(style.drop_guide_hit_padding * scale)
}

fn random_action() -> impl Strategy<Value = Action> {
    let selector = || {
        (any::<bool>(), any::<u8>()).prop_map(|(prefer_layout, index)| TabSelector {
            prefer_layout,
            index,
        })
    };
    let finish = || {
        prop_oneof![
            6 => Just(Finish::Release),
            1 => Just(Finish::Escape),
            1 => Just(Finish::PointerGone),
            1 => Just(Finish::OutsideRelease),
        ]
    };
    let direction = prop_oneof![
        Just(DockspaceDropDirection::Center),
        Just(DockspaceDropDirection::Left),
        Just(DockspaceDropDirection::Right),
        Just(DockspaceDropDirection::Top),
        Just(DockspaceDropDirection::Bottom),
    ];
    prop_oneof![
        2 => selector().prop_map(Action::Click),
        2 => any::<u8>().prop_map(Action::ToggleVisibility),
        5 => (selector(), any::<u8>(), direction, finish()).prop_map(
            |(source, target, direction, finish)| Action::Guide {
                source, target, direction, finish,
            }
        ),
        3 => (selector(), any::<u8>(), any::<bool>(), finish()).prop_map(
            |(source, target, after, finish)| Action::Gap {
                source, target, after, finish,
            }
        ),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(boundary, positive)| {
            Action::Resize { boundary, positive }
        }),
    ]
}

impl GridHarness {
    fn tab_slots(&self) -> Vec<TabSlot> {
        let surface = self.dockspace.view().surface(SURFACE).unwrap();
        let mut slots = Vec::new();
        for root in surface
            .main_root()
            .into_iter()
            .chain(surface.contained().map(|floating| floating.root().unwrap()))
        {
            if let Some(content) = root.content() {
                collect_tab_slots(content, &mut slots);
            }
        }
        slots
    }

    fn exposed_tabs(&self, movable: bool) -> Vec<TabSlot> {
        let mut slots = self.tab_slots();
        slots.retain(|slot| {
            // Resident anchors keep distinct destinations available without resetting the fixture.
            (!movable || (!slot.items.contains(&A) && !slot.items.contains(&D)))
                && self
                    .panes
                    .tab_controls
                    .get(&slot.key)
                    .is_some_and(|control| {
                        control.drag.width() >= 8.0
                            && control.drag.height() >= 8.0
                            && self.context.layer_id_at(control.drag.center())
                                == Some(control.layer)
                    })
        });
        slots.sort_by_key(|slot| slot.key);
        slots
    }

    fn pointer_hits(&mut self, point: Pos2) -> (Option<egui::Id>, Option<egui::Id>) {
        // Inspect the completed geometry at its own time before sending a press.
        let time = self.context.input(|input| input.time);
        self.run_at(time, vec![Event::PointerMoved(point)]);
        self.context.viewport(|viewport| {
            (
                viewport.hits.click.map(|hit| hit.id),
                viewport.hits.drag.map(|hit| hit.id),
            )
        })
    }

    fn pick_tab(&mut self, selector: TabSelector, movable: bool) -> Option<TabSlot> {
        let all = self.exposed_tabs(movable);
        let preferred = all
            .iter()
            .filter(|slot| slot.key.is_layout() == selector.prefer_layout)
            .collect::<Vec<_>>();
        if all.is_empty() {
            return None;
        }
        let first = if preferred.is_empty() {
            &all[usize::from(selector.index) % all.len()]
        } else {
            preferred[usize::from(selector.index) % preferred.len()]
        };
        for slot in std::iter::once(first).chain(all.iter().filter(|slot| slot.key != first.key)) {
            let control = self.panes.tab_controls[&slot.key];
            if self.pointer_hits(control.drag.center()) == (Some(control.id), Some(control.id)) {
                return Some(slot.clone());
            }
        }
        None
    }

    fn guide_targets(&self, source: &TabSlot) -> Vec<ItemId> {
        self.panes
            .bodies
            .iter()
            .filter(|(item, bounds)| {
                !source.owner_items.contains(item)
                    && bounds.width() >= 120.0
                    && bounds.height() >= 120.0
                    && self.context.layer_id_at(bounds.center())
                        == self.panes.body_layers.get(item).copied()
                    && self
                        .dockspace
                        .view()
                        .item(**item)
                        .unwrap()
                        .tabs()
                        .is_none_or(|tabs| {
                            // Avoid self and ancestor targets; these actions promise a real cross-scope move.
                            tabs.key() != source.owner
                                && !self
                                    .dockspace
                                    .view()
                                    .node(tabs.key())
                                    .unwrap()
                                    .items()
                                    .any(|item| source.items.contains(&item))
                        })
            })
            .map(|(item, _)| *item)
            .collect()
    }

    fn random_guide(
        &mut self,
        source: &TabSlot,
        target: u8,
        direction: DockspaceDropDirection,
        finish: Finish,
        trace: &mut Vec<String>,
    ) -> DragAttempt {
        let targets = self.guide_targets(source);
        if targets.is_empty() {
            trace.push("no exposed guide target with room for its buttons".to_owned());
            return DragAttempt::Unavailable;
        }
        let target = targets[usize::from(target) % targets.len()];
        let location = self.dockspace.view().item(target).unwrap();
        let target_root = location.root();
        let selected = location.tabs().and_then(|tabs| tabs.selected());
        let mut target_path = Vec::new();
        assert!(item_owner_path(
            self.dockspace
                .view()
                .root(target_root)
                .unwrap()
                .content()
                .unwrap(),
            target,
            &mut target_path,
        ));
        let target_owner = *target_path.last().unwrap();
        let target_items = self
            .dockspace
            .view()
            .node(target_owner)
            .unwrap()
            .items()
            .collect::<Vec<_>>();
        let source_point = self.panes.tab_controls[&source.key].drag.center();
        trace.push(format!(
            "guide: {source:?} -> {target:?} in {target_root:?}, {direction:?}, {finish:?}"
        ));
        self.begin_drag(source.key);
        let before = self.layout_signature();
        let body = self.panes.bodies[&target].center();
        self.run(vec![Event::PointerMoved(body)]);
        self.run(vec![Event::PointerMoved(body)]);
        let guide = self
            .panes
            .guides
            .iter()
            .filter(|guide| {
                guide.root == Some(target_root)
                    && guide.scope == DockspaceGuideScope::Inner
                    && guide.direction == direction
            })
            .min_by(|left, right| {
                left.rect
                    .center()
                    .distance_sq(body)
                    .total_cmp(&right.rect.center().distance_sq(body))
            })
            .unwrap_or_else(|| {
                panic!("the chosen active pane {target:?} must expose {direction:?}")
            });
        assert_eq!(guide.eligibility, DockspaceDropEligibility::Eligible);
        let point = guide.rect.center();
        self.run(vec![Event::PointerMoved(point)]);
        let preview = self
            .panes
            .preview
            .as_ref()
            .expect("eligible guide paints its preview");
        assert_eq!(preview.target.root(), Some(target_root));
        assert_eq!(preview.target.scope(), DockspaceDropTargetScope::Pane);
        assert_eq!(preview.target.selected_tab(), selected);
        assert_eq!(preview.target.selected_item(), Some(target));
        let action = match direction {
            DockspaceDropDirection::Center => DockspaceDropAction::Merge,
            DockspaceDropDirection::Left => DockspaceDropAction::Split {
                edge: DockEdge::Left,
            },
            DockspaceDropDirection::Right => DockspaceDropAction::Split {
                edge: DockEdge::Right,
            },
            DockspaceDropDirection::Top => DockspaceDropAction::Split {
                edge: DockEdge::Top,
            },
            DockspaceDropDirection::Bottom => DockspaceDropAction::Split {
                edge: DockEdge::Bottom,
            },
        };
        assert_eq!(preview.target.action(), action);
        assert!(preview.target_rect.contains(body));
        assert!(preview.rect.is_positive() && preview.rect.is_finite());
        if self.finish_random_drag(source_point, point, finish, &before) {
            for item in &source.items {
                assert_eq!(
                    self.dockspace.view().item(*item).unwrap().root(),
                    target_root
                );
            }
            if direction == DockspaceDropDirection::Center {
                let slots = self.tab_slots();
                let inserted = slots.iter().find(|slot| slot.key == source.key).unwrap();
                assert_eq!(inserted.root, target_root);
                assert_eq!(
                    self.dockspace
                        .view()
                        .node(inserted.owner)
                        .unwrap()
                        .tabs()
                        .unwrap()
                        .selected(),
                    Some(source.key),
                );
                assert!(inserted.owner_items.contains(&target));
            } else {
                assert_eq!(
                    self.dockspace
                        .view()
                        .node(target_owner)
                        .unwrap()
                        .tabs()
                        .and_then(|tabs| tabs.selected()),
                    selected,
                );
                self.assert_edge_commit(source, target, target_owner, &target_items, direction);
            }
        }
        DragAttempt::Finished(finish)
    }

    fn assert_edge_commit(
        &self,
        source: &TabSlot,
        target: ItemId,
        target_owner: NodeKey,
        target_items: &[ItemId],
        direction: DockspaceDropDirection,
    ) {
        let view = self.dockspace.view();
        let root = view.item(target).unwrap().root();
        let mut path = Vec::new();
        assert!(item_owner_path(
            view.root(root).unwrap().content().unwrap(),
            target,
            &mut path
        ));
        assert_eq!(
            path.last(),
            Some(&target_owner),
            "an inner edge preserves the actual destination scope"
        );
        assert_eq!(
            view.node(target_owner).unwrap().items().collect::<Vec<_>>(),
            target_items
        );
        let expected_axis = match direction {
            DockspaceDropDirection::Left | DockspaceDropDirection::Right => {
                DockspaceAxis::Horizontal
            }
            DockspaceDropDirection::Top | DockspaceDropDirection::Bottom => DockspaceAxis::Vertical,
            DockspaceDropDirection::Center => unreachable!("center has a direct-tabs contract"),
        };
        let mut found = false;
        for ancestor in path.iter().rev().skip(1) {
            let Some(split) = view.node(*ancestor).unwrap().split() else {
                continue;
            };
            let children = split.children().collect::<Vec<_>>();
            let target_index = children
                .iter()
                .position(|child| path.contains(&child.key()))
                .unwrap();
            let payload_indices = children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| {
                    child
                        .items()
                        .any(|item| source.items.contains(&item))
                        .then_some(index)
                })
                .collect::<Vec<_>>();
            if payload_indices.is_empty() || payload_indices.contains(&target_index) {
                continue;
            }
            assert_eq!(split.axis(), expected_axis);
            assert_eq!(
                children[target_index].items().collect::<Vec<_>>(),
                target_items,
                "the split must keep the exact destination, rather than an ancestor with sibling cells",
            );
            let mut actual = payload_indices
                .iter()
                .flat_map(|index| children[*index].items())
                .collect::<Vec<_>>();
            let mut expected = source.items.clone();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(
                actual, expected,
                "the new branch contains precisely the typed tab payload"
            );
            let first = payload_indices[0];
            let last = *payload_indices.last().unwrap();
            assert_eq!(last + 1 - first, payload_indices.len());
            if matches!(
                direction,
                DockspaceDropDirection::Left | DockspaceDropDirection::Top
            ) {
                assert_eq!(last + 1, target_index);
            } else {
                assert_eq!(target_index + 1, first);
            }
            found = true;
            break;
        }
        assert!(
            found,
            "a completed inner-edge action must leave the payload beside its exact destination"
        );
        let payload_bounds = source
            .items
            .iter()
            .filter_map(|item| self.panes.bodies.get(item).copied())
            .reduce(|left, right| left.union(right))
            .expect("the inserted payload contributes active application content");
        let target_bounds = self.panes.bodies[&target];
        let ordered = match direction {
            DockspaceDropDirection::Left => payload_bounds.right() <= target_bounds.left() + 0.5,
            DockspaceDropDirection::Right => payload_bounds.left() + 0.5 >= target_bounds.right(),
            DockspaceDropDirection::Top => payload_bounds.bottom() <= target_bounds.top() + 0.5,
            DockspaceDropDirection::Bottom => payload_bounds.top() + 0.5 >= target_bounds.bottom(),
            DockspaceDropDirection::Center => unreachable!(),
        };
        assert!(
            ordered,
            "{direction:?}: payload {payload_bounds:?}, destination {target_bounds:?}"
        );
    }

    fn random_gap(
        &mut self,
        source: &TabSlot,
        target: u8,
        after: bool,
        finish: Finish,
        trace: &mut Vec<String>,
    ) -> DragAttempt {
        let choices = self
            .exposed_tabs(false)
            .into_iter()
            .filter(|slot| {
                slot.key != source.key
                    && !slot.items.iter().any(|item| source.items.contains(item))
                    && (slot.owner == source.owner
                        || (!slot
                            .owner_items
                            .iter()
                            .any(|item| source.items.contains(item))
                            && !slot
                                .items
                                .iter()
                                .any(|item| source.owner_items.contains(item))))
            })
            .filter_map(|slot| {
                let control = self.panes.tab_controls[&slot.key];
                let point = tab_gap_point(control, after)?;
                (self.context.layer_id_at(point) == Some(control.layer))
                    .then_some((slot, point, control.id))
            })
            .collect::<Vec<_>>();
        let selected = if choices.is_empty() {
            None
        } else {
            let first = usize::from(target) % choices.len();
            (0..choices.len())
                .map(|offset| (first + offset) % choices.len())
                .find(|index| {
                    let (_, point, id) = &choices[*index];
                    self.pointer_hits(*point) == (Some(*id), Some(*id))
                })
        };
        let Some(selected) = selected else {
            // Availability is decided from the pre-gesture view, never from a rejected drop.
            trace.push("no exposed tab gap".to_owned());
            return DragAttempt::Unavailable;
        };
        let (destination, point, _) = &choices[selected];
        let visible = self
            .tab_slots()
            .into_iter()
            .filter(|slot| {
                slot.items
                    .iter()
                    .any(|item| !self.expected_hidden.contains(item))
            })
            .map(|slot| slot.key)
            .collect::<BTreeSet<_>>();
        let position = destination
            .roster
            .iter()
            .position(|key| *key == destination.key)
            .unwrap();
        // A visible gap is before its next visible tab, or after the complete roster.
        let index = if after {
            destination
                .roster
                .iter()
                .enumerate()
                .skip(position + 1)
                .find_map(|(index, key)| visible.contains(key).then_some(index))
                .unwrap_or(destination.roster.len())
        } else {
            position
        };
        let previous = destination.roster[..index]
            .iter()
            .rev()
            .copied()
            .find(|key| *key != source.key && visible.contains(key));
        let next = destination.roster[index..]
            .iter()
            .copied()
            .find(|key| *key != source.key && visible.contains(key));
        let mut expected = destination.roster.clone();
        let offset = usize::from(expected[..index].contains(&source.key));
        expected.retain(|key| *key != source.key);
        expected.insert(index - offset, source.key);
        trace.push(format!(
            "gap: {source:?} -> {destination:?}, index={index}, after={after}, point={point:?}, {finish:?}"
        ));
        let source_point = self.panes.tab_controls[&source.key].drag.center();
        self.begin_drag(source.key);
        let before = self.layout_signature();
        let current_gap = self
            .panes
            .tab_controls
            .get(&destination.key)
            .is_some_and(|control| {
                control.drag.contains(*point)
                    && (point.x >= control.rect.center().x) == after
                    && self.context.layer_id_at(*point) == Some(control.layer)
            });
        let guide_overlap = self
            .panes
            .guides
            .iter()
            .any(|guide| guide_hit_bounds(guide, &self.dockspace.style).contains(*point));
        if !current_gap || guide_overlap {
            trace.push(format!(
                "gap target not selected: current_gap={current_gap}, guide_overlap={guide_overlap}; cancel before target motion"
            ));
            self.finish_random_drag(source_point, source_point, Finish::Escape, &before);
            return DragAttempt::CancelledBeforePreview;
        }
        self.run(vec![Event::PointerMoved(*point)]);
        let preview = self
            .panes
            .preview
            .as_ref()
            .expect("the exact exposed gap paints a preview");
        assert_eq!(preview.target.root(), Some(destination.root));
        assert_eq!(preview.target.scope(), DockspaceDropTargetScope::Pane);
        assert_eq!(
            preview.target.action(),
            DockspaceDropAction::InsertTab {
                index,
                previous,
                next
            },
        );
        assert_eq!(
            preview.tabs.iter().map(|tab| tab.key).collect::<Vec<_>>(),
            expected
                .iter()
                .copied()
                .filter(|key| visible.contains(key))
                .collect::<Vec<_>>()
        );
        if matches!(finish, Finish::Release) {
            // Exact insertion uses the current strip before an edge-scroll deadline can move it.
            let time = self.context.input(|input| input.time);
            self.release_at(time, *point);
            self.assert_random_invariants();
            self.assert_tabs(destination.owner, &expected, source.key);
        } else {
            self.finish_random_drag(source_point, *point, finish, &before);
        }
        DragAttempt::Finished(finish)
    }

    fn finish_random_drag(
        &mut self,
        source: Pos2,
        point: Pos2,
        finish: Finish,
        before: &[String],
    ) -> bool {
        match finish {
            Finish::Release => self.release(point),
            Finish::OutsideRelease => {
                let outside = egui::pos2(-80.0, -80.0);
                self.run(vec![Event::PointerMoved(outside), Event::PointerGone]);
                self.run(vec![button(outside, false), Event::PointerGone]);
                self.run(vec![Event::PointerMoved(egui::pos2(1050.0, 720.0))]);
                self.settle();
                assert_eq!(self.layout_signature().as_slice(), before);
            }
            Finish::Escape | Finish::PointerGone => {
                if matches!(finish, Finish::PointerGone) {
                    self.run(vec![Event::PointerGone]);
                    self.run(vec![Event::PointerMoved(source)]);
                    self.run(vec![button(source, true)]);
                    self.run(vec![Event::PointerMoved(source + vec2(20.0, 8.0))]);
                    assert!(
                        self.dockspace.has_active_gesture(),
                        "a fresh press recovers the lost release"
                    );
                }
                self.run(vec![key(Key::Escape)]);
                self.run(vec![
                    button(source, false),
                    Event::Key {
                        key: Key::Escape,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                self.settle();
                assert_eq!(self.layout_signature().as_slice(), before);
            }
        }
        self.assert_random_invariants();
        matches!(finish, Finish::Release)
    }

    fn random_resize(&mut self, boundary: u8, positive: bool, trace: &mut Vec<String>) {
        let choices = self
            .splitters
            .iter()
            .filter(|control| {
                control.value - control.minimum > 0.02 && control.maximum - control.value > 0.02
            })
            .copied()
            .collect::<Vec<_>>();
        if choices.is_empty() {
            trace.push("no unconstrained splitter; use a real tab click".to_owned());
            self.random_click(
                TabSelector {
                    prefer_layout: false,
                    index: boundary,
                },
                trace,
            );
            return;
        }
        let offset = usize::from(boundary) % choices.len();
        let mut exposed = None;
        'controls: for control in choices.iter().cycle().skip(offset).take(choices.len()) {
            let horizontal = control.orientation == Orientation::Horizontal;
            for fraction in [0.25, 0.75, 0.125, 0.875, 0.5] {
                let point = if horizontal {
                    egui::pos2(
                        control.rect.left() + control.rect.width() * fraction,
                        control.rect.center().y,
                    )
                } else {
                    egui::pos2(
                        control.rect.center().x,
                        control.rect.top() + control.rect.height() * fraction,
                    )
                };
                if self.pointer_hits(point).1.map(|id| id.accesskit_id()) == Some(control.id) {
                    exposed = Some((*control, point));
                    break 'controls;
                }
            }
        }
        let Some((control, point)) = exposed else {
            trace.push("no actual splitter receiver is exposed; use a real tab click".to_owned());
            self.random_click(
                TabSelector {
                    prefer_layout: false,
                    index: boundary,
                },
                trace,
            );
            return;
        };
        let horizontal = control.orientation == Orientation::Horizontal;
        let delta = if positive { 12.0 } else { -12.0 };
        let destination = point
            + if horizontal {
                vec2(0.0, delta)
            } else {
                vec2(delta, 0.0)
            };
        trace.push(format!("resize: {control:?}, {point:?} -> {destination:?}"));
        self.run(vec![button(point, true)]);
        self.run(vec![Event::PointerMoved(destination)]);
        assert!(self.dockspace.has_active_gesture());
        self.release(destination);
        let updated = self
            .splitters
            .iter()
            .find(|splitter| splitter.id == control.id)
            .unwrap_or_else(|| {
                panic!(
                    "the resized splitter keeps an operable numeric range: {control:?} -> {destination:?}\n{}",
                    trace.join("\n"),
                )
            });
        assert!(if positive {
            updated.value > control.value
        } else {
            updated.value < control.value
        });
    }

    fn random_click(&mut self, selector: TabSelector, trace: &mut Vec<String>) {
        let Some(slot) = self.pick_tab(selector, false) else {
            let editors = self
                .panes
                .editors
                .iter()
                .filter(|(_, (_, rect, layer))| {
                    rect.width() >= 8.0
                        && rect.height() >= 8.0
                        && self.context.layer_id_at(rect.center()) == Some(*layer)
                })
                .collect::<Vec<_>>();
            assert!(
                !editors.is_empty(),
                "a visible layout must expose real application content"
            );
            let (item, &(id, rect, _)) = editors[usize::from(selector.index) % editors.len()];
            trace.push(format!(
                "no exposed tab; focus the real content editor {item:?}"
            ));
            let point = rect.center();
            self.run(vec![Event::PointerMoved(point)]);
            self.run(vec![button(point, true)]);
            self.run(vec![button(point, false)]);
            self.settle();
            assert_eq!(self.context.memory(egui::Memory::focused), Some(id));
            return;
        };
        trace.push(format!("click: {:?} in {:?}", slot.key, slot.owner));
        self.click_tab(slot.key);
        assert_eq!(
            self.dockspace
                .view()
                .node(slot.owner)
                .unwrap()
                .tabs()
                .unwrap()
                .selected(),
            Some(slot.key),
        );
    }

    fn random_visibility(&mut self, selector: u8, trace: &mut Vec<String>) {
        // A and D remain visible resident anchors, even when every tabbed branch is hidden.
        let candidates = [B, C, E, F, G];
        let item = candidates[usize::from(selector) % candidates.len()];
        let visible = self.expected_hidden.contains(&item);
        trace.push(format!("visibility: {item:?} -> {visible}"));
        let result = self
            .dockspace
            .set_item_visibility_current(item, visible)
            .unwrap();
        assert!(matches!(
            result.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemVisibilityChanged {
                item: changed_item, visible: changed_visibility, changed: true,
            }) if *changed_item == item && *changed_visibility == visible
        ));
        if visible {
            self.expected_hidden.remove(&item);
        } else {
            self.expected_hidden.insert(item);
        }
        self.settle();
    }

    fn assert_random_invariants(&self) {
        let view = self.dockspace.view();
        assert_eq!(view.surfaces().count(), 1);
        let surface = view.surface(SURFACE).unwrap();
        let mut owned = Vec::new();
        for root in surface
            .main_root()
            .into_iter()
            .chain(surface.contained().map(|floating| floating.root().unwrap()))
        {
            for item in root.items() {
                let location = view.item(item).unwrap();
                assert_eq!(location.root(), root.id());
                assert_eq!(location.surface(), SURFACE);
                owned.push(item);
            }
        }
        owned.sort_unstable();
        assert_eq!(
            owned, ITEMS,
            "all seven panes must stay owned exactly once across every root"
        );
        assert_eq!(
            view.hidden_items().collect::<BTreeSet<_>>(),
            self.expected_hidden
        );
        for item in &self.expected_hidden {
            assert!(!view.item(*item).unwrap().is_active());
            assert!(!self.panes.bodies.contains_key(item));
        }
        let active = ITEMS
            .into_iter()
            .filter(|item| view.item(*item).unwrap().is_active())
            .collect::<Vec<_>>();
        let mut surface_active = surface.active_items().collect::<Vec<_>>();
        surface_active.sort_unstable();
        assert_eq!(surface_active, active);
        let mut calls = self.panes.body_calls.clone();
        calls.sort_unstable();
        assert_eq!(
            calls, active,
            "active content receives exactly one real application UI callback"
        );
        assert_eq!(
            self.panes.bodies.keys().copied().collect::<Vec<_>>(),
            active
        );
        assert!(!self.dockspace.has_active_gesture());
        assert!(self.panes.preview.is_none());
        assert!(!self.context.input(|input| input.pointer.primary_down()));
    }

    pub(super) fn layout_signature(&self) -> Vec<String> {
        let surface = self.dockspace.view().surface(SURFACE).unwrap();
        let mut signature = Vec::new();
        if let Some(root) = surface.main_root() {
            signature.push(format!("main {:?}", root.id()));
            if let Some(content) = root.content() {
                append_layout_signature(content, &mut signature);
            }
        }
        for floating in surface.contained() {
            let root = floating.root().unwrap();
            signature.push(format!(
                "floating {:?}, {:?}, {:?}",
                floating.id(),
                floating.rect(),
                root.id()
            ));
            append_layout_signature(root.content().unwrap(), &mut signature);
        }
        signature
    }
}

#[test]
fn grid_resize_uses_an_exposed_handle_after_a_bare_cell_edge_drop() {
    let mut harness = GridHarness::new();
    let mut trace = Vec::new();
    let source = harness
        .pick_tab(
            TabSelector {
                prefer_layout: false,
                index: 29,
            },
            true,
        )
        .unwrap();
    assert_eq!(source.key, E.into());
    harness.random_guide(
        &source,
        29,
        DockspaceDropDirection::Left,
        Finish::Release,
        &mut trace,
    );
    harness.random_resize(217, false, &mut trace);
    harness.assert_random_invariants();
    harness.random_click(
        TabSelector {
            prefer_layout: true,
            index: 0,
        },
        &mut trace,
    );
    harness.assert_random_invariants();
}

#[test]
fn shared_grid_row_resize_range_remains_available_after_consecutive_resizes() {
    let mut harness = GridHarness::new();
    let mut trace = Vec::new();
    let source = harness
        .pick_tab(
            TabSelector {
                prefer_layout: false,
                index: 0,
            },
            true,
        )
        .unwrap();
    assert_eq!(source.key, B.into());
    harness.random_guide(
        &source,
        52,
        DockspaceDropDirection::Top,
        Finish::Release,
        &mut trace,
    );
    harness.assert_random_invariants();
    harness.random_click(
        TabSelector {
            prefer_layout: true,
            index: 0,
        },
        &mut trace,
    );
    harness.assert_random_invariants();
    let row = harness
        .splitters
        .iter()
        .filter(|control| control.orientation == Orientation::Horizontal)
        .max_by(|left, right| left.rect.width().total_cmp(&right.rect.width()))
        .copied()
        .expect("the shared Grid exposes its full-width row boundary");
    let mut previous = row.value;
    for (boundary, positive, prefer_layout) in
        [(181, true, false), (25, true, true), (25, false, false)]
    {
        harness.random_resize(boundary, positive, &mut trace);
        harness.assert_random_invariants();
        let updated = harness
            .splitters
            .iter()
            .find(|control| control.id == row.id)
            .expect("the shared Grid row retains an operable numeric range after each resize");
        assert!(
            updated.minimum.is_finite() && updated.value.is_finite() && updated.maximum.is_finite()
        );
        assert!(updated.minimum < updated.value && updated.value < updated.maximum);
        assert!(if positive {
            updated.value > previous
        } else {
            updated.value < previous
        });
        previous = updated.value;
        harness.random_click(
            TabSelector {
                prefer_layout,
                index: 0,
            },
            &mut trace,
        );
        harness.assert_random_invariants();
    }
}

#[test]
fn nested_shared_grid_tail_gap_after_repeated_inner_moves_uses_the_current_strip() {
    let mut harness = GridHarness::new();
    let mut trace = Vec::new();
    let moves = [
        (true, 0, 2, DockspaceDropDirection::Left, harness.tools),
        (false, 0, 198, DockspaceDropDirection::Top, B.into()),
        (false, 220, 64, DockspaceDropDirection::Center, C.into()),
    ];
    for (step, (prefer_layout, index, target, direction, expected)) in moves.into_iter().enumerate()
    {
        let source = harness
            .pick_tab(
                TabSelector {
                    prefer_layout,
                    index,
                },
                true,
            )
            .unwrap();
        assert_eq!(source.key, expected);
        harness.random_guide(&source, target, direction, Finish::Release, &mut trace);
        harness.assert_random_invariants();
        harness.random_click(
            TabSelector {
                prefer_layout: step % 2 == 0,
                index: 0,
            },
            &mut trace,
        );
        harness.assert_random_invariants();
    }
    let source = harness
        .pick_tab(
            TabSelector {
                prefer_layout: false,
                index: 38,
            },
            true,
        )
        .unwrap();
    assert_eq!(source.key, E.into());
    let target = harness.dockspace.view().item(C).unwrap().tabs().unwrap();
    assert_eq!(target.tabs(), &[F.into(), C.into()]);
    let owner = target.key();
    let control = harness.panes.tab_controls[&C.into()];
    let covered = egui::pos2(control.rect.right() - 1.0, control.drag.center().y);
    let time = harness.context.input(|input| input.time);
    let output = harness.run_at(time, vec![Event::PointerMoved(covered)]);
    let covered_hit = harness
        .context
        .viewport(|viewport| viewport.hits.drag.unwrap().id);
    assert!(
        nodes(&output).iter().any(|(id, node)| {
            *id == covered_hit.accesskit_id()
                && node.role() == Role::Button
                && node.label() == Some("Scroll tabs forward")
        }),
        "the last tab's allocated tail is covered by the real scroll-forward control",
    );
    let exposed = tab_gap_point(control, true).unwrap();
    assert!(exposed.x > control.rect.center().x);
    assert!(exposed.x < control.drag.right());
    assert_eq!(
        harness.pointer_hits(exposed),
        (Some(control.id), Some(control.id)),
        "the gap beside the scroll button remains exposed through the tab body",
    );
    harness.random_gap(&source, 111, true, Finish::Release, &mut trace);
    harness.assert_tabs(owner, &[F.into(), C.into(), E.into()], E.into());
    harness.assert_random_invariants();
    harness.random_click(
        TabSelector {
            prefer_layout: false,
            index: 0,
        },
        &mut trace,
    );
    harness.assert_random_invariants();
}

fn start_nested_overflow_gap_drag() -> (GridHarness, NodeKey, Pos2) {
    let mut harness = GridHarness::new();
    let mut trace = Vec::new();
    harness.click_tab(harness.tools);
    let moves = [
        (false, 39, 20, DockspaceDropDirection::Left, B.into()),
        (true, 173, 49, DockspaceDropDirection::Left, harness.tools),
        (true, 19, 229, DockspaceDropDirection::Center, C.into()),
    ];
    for (step, (prefer_layout, index, target, direction, expected)) in moves.into_iter().enumerate()
    {
        let source = harness
            .pick_tab(
                TabSelector {
                    prefer_layout,
                    index,
                },
                true,
            )
            .unwrap();
        assert_eq!(source.key, expected);
        harness.random_guide(&source, target, direction, Finish::Release, &mut trace);
        harness.assert_random_invariants();
        harness.random_click(
            TabSelector {
                prefer_layout: step % 2 == 1,
                index: 0,
            },
            &mut trace,
        );
        harness.assert_random_invariants();
    }
    let source = harness
        .pick_tab(
            TabSelector {
                prefer_layout: false,
                index: 203,
            },
            true,
        )
        .unwrap();
    assert_eq!(source.key, E.into());
    let destination = harness.dockspace.view().item(C).unwrap().tabs().unwrap();
    assert_eq!(destination.tabs(), &[B.into(), C.into()]);
    assert_eq!(destination.selected(), Some(C.into()));
    let owner = destination.key();
    let control = harness.panes.tab_controls[&C.into()];
    let point = tab_gap_point(control, false).unwrap();
    assert_eq!(
        harness.pointer_hits(point),
        (Some(control.id), Some(control.id)),
        "the destination tab must be the exposed receiver before the source press",
    );
    harness.begin_drag(E.into());
    assert!(harness.panes.tab_controls[&C.into()].drag.contains(point));
    assert!(point.x < harness.panes.tab_controls[&C.into()].rect.center().x);
    assert!(
        harness
            .panes
            .guides
            .iter()
            .all(|guide| !guide_hit_bounds(guide, &harness.dockspace.style).contains(point))
    );
    (harness, owner, point)
}

#[test]
fn shared_grid_gap_before_a_selected_tab_commits_before_the_edge_scroll_deadline() {
    let (mut harness, owner, point) = start_nested_overflow_gap_drag();
    harness.run(vec![Event::PointerMoved(point)]);
    harness.assert_preview(
        point,
        DockspaceDropAction::InsertTab {
            index: 1,
            previous: Some(B.into()),
            next: Some(C.into()),
        },
        &[B.into(), E.into(), C.into()],
        Some(E.into()),
        &[E.into()],
    );
    let time = harness.context.input(|input| input.time);
    let deadline = harness
        .dockspace
        .session
        .pointer_continuation()
        .expect("the insertion point also starts backward edge scrolling")
        .deadline()
        .as_secs_f64();
    assert!(time < deadline);
    harness.release_at(time, point);
    harness.assert_tabs(owner, &[B.into(), E.into(), C.into()], E.into());
    harness.assert_random_invariants();
    harness.click_tab(E.into());
    harness.assert_random_invariants();
}

#[test]
fn shared_grid_edge_scroll_revokes_an_old_gap_and_accepts_fresh_presented_geometry() {
    let (mut harness, owner, point) = start_nested_overflow_gap_drag();
    let before = harness.layout_signature();
    let version = harness.dockspace.version();
    harness.run(vec![Event::PointerMoved(point)]);
    harness.assert_preview(
        point,
        DockspaceDropAction::InsertTab {
            index: 1,
            previous: Some(B.into()),
            next: Some(C.into()),
        },
        &[B.into(), E.into(), C.into()],
        Some(E.into()),
        &[E.into()],
    );
    let deadline = harness
        .dockspace
        .session
        .pointer_continuation()
        .expect("the exposed C gap also starts backward edge scrolling")
        .deadline()
        .as_secs_f64();
    harness.run_at(deadline + 0.001, Vec::new());
    assert!(harness.dockspace.has_active_gesture());
    assert!(harness.panes.preview.is_none());
    assert!(!harness.panes.tab_controls.contains_key(&C.into()));
    assert!(harness.panes.tab_controls.contains_key(&B.into()));
    assert_eq!(harness.layout_signature(), before);
    assert_eq!(harness.dockspace.version(), version);
    harness.assert_tabs(owner, &[B.into(), C.into()], C.into());

    // Admit the newly Presented strip at the same time, then use only its exposed gap.
    let time = harness.context.input(|input| input.time);
    harness.run_at(time, Vec::new());
    let current = harness.panes.tab_controls[&B.into()];
    // The clipped tab's left edge lies inside the adjacent splitter's hit slop.
    let leading_half = Rect::from_min_max(
        current.rect.min,
        egui::pos2(current.rect.center().x, current.rect.bottom()),
    );
    let current_gap = leading_half.intersect(current.drag).center();
    assert!(
        harness
            .panes
            .guides
            .iter()
            .all(|guide| !guide_hit_bounds(guide, &harness.dockspace.style).contains(current_gap))
    );
    assert_eq!(
        harness.pointer_hits(current_gap),
        (Some(current.id), Some(current.id)),
        "the replacement gap belongs to B in the newly admitted strip",
    );
    harness.assert_preview(
        current_gap,
        DockspaceDropAction::InsertTab {
            index: 0,
            previous: None,
            next: Some(B.into()),
        },
        &[E.into(), B.into(), C.into()],
        Some(E.into()),
        &[E.into()],
    );
    harness.release_at(time, current_gap);
    harness.assert_tabs(owner, &[E.into(), B.into(), C.into()], E.into());
    harness.assert_random_invariants();
    harness.click_tab(E.into());
    harness.assert_random_invariants();
}

#[test]
fn shared_grid_outer_guide_overrides_an_overlapping_gap_and_escape_preserves_the_layout() {
    let mut harness = GridHarness::new();
    let mut trace = Vec::new();
    let moves = [(220, 0, C.into()), (39, 2, B.into())];
    for (step, (index, target, expected)) in moves.into_iter().enumerate() {
        let source = harness
            .pick_tab(
                TabSelector {
                    prefer_layout: false,
                    index,
                },
                true,
            )
            .unwrap();
        assert_eq!(source.key, expected);
        harness.random_guide(
            &source,
            target,
            DockspaceDropDirection::Center,
            Finish::Release,
            &mut trace,
        );
        harness.assert_random_invariants();
        harness.random_click(
            TabSelector {
                prefer_layout: step % 2 == 0,
                index: 0,
            },
            &mut trace,
        );
        harness.assert_random_invariants();
    }
    let source = harness
        .pick_tab(
            TabSelector {
                prefer_layout: true,
                index: 0,
            },
            true,
        )
        .unwrap();
    assert_eq!(source.key, harness.tools);
    assert_eq!(source.roster, [E.into(), harness.tools]);
    let control = harness.panes.tab_controls[&E.into()];
    let overlapping = egui::pos2(
        control.rect.left() + control.rect.width() * 0.25,
        control.drag.center().y,
    );
    let source_point = harness.panes.tab_controls[&harness.tools].drag.center();
    harness.begin_drag(harness.tools);
    let before = harness.layout_signature();
    assert!(
        harness.panes.guides.iter().any(|guide| {
            guide.scope == DockspaceGuideScope::Outer
                && guide.direction == DockspaceDropDirection::Left
                && guide_hit_bounds(guide, &harness.dockspace.style).contains(overlapping)
        }),
        "the first tab's insertion half intersects the visible root-left guide",
    );
    harness.run(vec![Event::PointerMoved(overlapping)]);
    harness.run(vec![Event::PointerMoved(overlapping)]);
    let preview = harness.panes.preview.as_ref().unwrap();
    assert_eq!(preview.target.root(), Some(ROOT));
    assert_eq!(preview.target.scope(), DockspaceDropTargetScope::Root);
    assert_eq!(
        preview.target.action(),
        DockspaceDropAction::Split {
            edge: DockEdge::Left
        },
    );
    assert_eq!(preview.target.selected_tab(), None);
    assert_eq!(preview.target.selected_item(), None);
    assert_eq!(
        preview.target_rect,
        Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))
    );
    assert_eq!(
        preview.rect,
        Rect::from_min_size(Pos2::ZERO, vec2(549.5, 760.0))
    );
    assert!(harness.panes.guides.iter().any(|guide| {
        guide.active
            && guide.scope == DockspaceGuideScope::Outer
            && guide.direction == DockspaceDropDirection::Left
    }),);
    harness.finish_random_drag(source_point, overlapping, Finish::Escape, &before);
    harness.assert_tabs(source.owner, &[E.into(), harness.tools], harness.tools);
    harness.assert_random_invariants();

    harness.click_tab(E.into());
    harness.click_tab(harness.tools);
    let control = harness.panes.tab_controls[&E.into()];
    let exposed = tab_gap_point(control, false).unwrap();
    assert_eq!(exposed.x, 23.0);
    assert_eq!(
        harness.pointer_hits(exposed),
        (Some(control.id), Some(control.id)),
    );
    harness.begin_drag(harness.tools);
    assert!(
        harness
            .panes
            .guides
            .iter()
            .all(|guide| !guide_hit_bounds(guide, &harness.dockspace.style).contains(exposed))
    );
    harness.run(vec![Event::PointerMoved(exposed)]);
    harness.run(vec![Event::PointerMoved(exposed)]);
    harness.assert_preview(
        exposed,
        DockspaceDropAction::InsertTab {
            index: 0,
            previous: None,
            next: Some(E.into()),
        },
        &[harness.tools, E.into()],
        Some(harness.tools),
        &[harness.tools],
    );
    harness.release(exposed);
    harness.assert_tabs(source.owner, &[harness.tools, E.into()], harness.tools);
    harness.assert_active(&[A, B, D, G]);
    harness.assert_random_invariants();
    harness.click_tab(E.into());
    harness.click_tab(harness.tools);
    harness.assert_random_invariants();
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        max_shrink_iters: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn shared_grid_random_pointer_sequences_preserve_ownership_and_recover(
        geometry in 0u8..4,
        hidden in 0u8..32,
        actions in proptest::collection::vec(random_action(), 1..17),
    ) {
        let mut trace = vec![format!("initial geometry={geometry}, hidden={hidden:#07b}")];
        let mut coverage = DragCoverage::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut harness = GridHarness::new();
            let physical_size = vec2(1100.0, 760.0);
            let geometry_name = match geometry {
                0 => "default",
                1 => {
                    harness.native_pixels_per_point = Some(1.25);
                    harness.screen_rect = Rect::from_min_size(Pos2::ZERO, physical_size / 1.25);
                    "native DPI 1.25"
                }
                2 => {
                    harness.context.set_zoom_factor(1.5);
                    harness.screen_rect = Rect::from_min_size(Pos2::ZERO, physical_size / 1.5);
                    "UI zoom 1.5"
                }
                3 => {
                    harness.screen_rect = Rect::from_min_size(Pos2::ZERO, vec2(880.0, 643.5));
                    "narrow window with fractional height"
                }
                _ => unreachable!("the generated initial geometry is in 0..4"),
            };
            harness.settle();
            for selector in 0..5 {
                if hidden & (1 << selector) != 0 {
                    harness.random_visibility(selector, &mut trace);
                }
            }
            trace.push(format!(
                "geometry {geometry}: {geometry_name}; viewport={:?}, pixels_per_point={}, zoom={}",
                harness.context.content_rect(),
                harness.context.pixels_per_point(),
                harness.context.zoom_factor(),
            ));
            harness.assert_random_invariants();
            for (step, action) in actions.iter().copied().enumerate() {
                trace.push(format!("step {step}: {action:?}"));
                match action {
                    Action::Click(selector) => harness.random_click(selector, &mut trace),
                    Action::ToggleVisibility(selector) => harness.random_visibility(selector, &mut trace),
                    Action::Guide { source, target, direction, finish } => {
                        let attempt = if let Some(source) = harness.pick_tab(source, true) {
                            harness.random_guide(&source, target, direction, finish, &mut trace)
                        } else {
                            trace.push("no exposed movable header".to_owned());
                            DragAttempt::Unavailable
                        };
                        coverage.guide(direction, attempt);
                    }
                    Action::Gap { source, target, after, finish } => {
                        let attempt = if let Some(source) = harness.pick_tab(source, true) {
                            harness.random_gap(&source, target, after, finish, &mut trace)
                        } else {
                            trace.push("no exposed movable header".to_owned());
                            DragAttempt::Unavailable
                        };
                        coverage.gap(attempt);
                    }
                    Action::Resize { boundary, positive } => {
                        harness.random_resize(boundary, positive, &mut trace);
                    }
                }
                harness.assert_random_invariants();
                // A real subsequent click must work after commits, cancellations and resizes.
                harness.random_click(TabSelector { prefer_layout: step % 2 == 0, index: 0 }, &mut trace);
                harness.assert_random_invariants();
            }
            coverage.assert_requests(&actions);
        }));
        if let Err(error) = result {
            let reason = error.downcast_ref::<String>().map(String::as_str)
                .or_else(|| error.downcast_ref::<&str>().copied()).unwrap_or("non-string panic");
            prop_assert!(false, "{reason}\n{coverage:#?}\nresolved trace:\n{}", trace.join("\n"));
        }
    }
}
