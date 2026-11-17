//! Public-guide admission and semantic outcomes for actual docking gestures.

use egui_dockspace::{
    DockEdge, DockspaceDropAction, DockspaceDropDirection, DockspaceDropEligibility,
    DockspaceDropTargetScope, DockspaceGuideScope,
};

use super::semantics::MoveExpectation;
use super::*;

impl Replay {
    fn drop_observations(&self) -> Option<&editor::DemoDropObservations> {
        self.panes.drop_observations.as_ref().filter(|observed| {
            // egui increments its pass counter after completing the pass.
            // A final pass without callbacks must not reuse earlier guides.
            observed.pass + 1 == self.context.cumulative_pass_nr()
        })
    }

    fn covering_floating_roots(
        &self,
        point: Pos2,
        source_root: egui_dockspace::RootId,
    ) -> Vec<egui_dockspace::RootId> {
        self.dock
            .view()
            .surface(SURFACE)
            .unwrap()
            .contained()
            .filter_map(|floating| {
                let root = floating.root().unwrap().id();
                let rect = floating.rect();
                (root != source_root
                    && f64::from(point.x) >= rect.x()
                    && f64::from(point.x) <= rect.x() + rect.width()
                    && f64::from(point.y) >= rect.y()
                    && f64::from(point.y) <= rect.y() + rect.height())
                .then_some(root)
            })
            .collect()
    }

    fn separate_active_destination(
        &self,
        item: egui_dockspace::ItemId,
        target: egui_dockspace::ItemId,
    ) -> bool {
        let view = self.dock.view();
        let source = view
            .item(item)
            .unwrap()
            .tabs()
            .expect("item is an inline tab");
        // Pressing a source tab selects it. A destination in a sibling layout
        // tab then becomes inactive, even if its own Tabs selection is unchanged.
        view.item(target).unwrap().is_active()
            && !view
                .node(source.key())
                .unwrap()
                .items()
                .any(|owned| owned == target)
    }

    fn dock_at(
        &mut self,
        item: egui_dockspace::ItemId,
        target: egui_dockspace::ItemId,
        label: &str,
    ) -> Attempt {
        if !self.begin_drag(item, Modifiers::NONE) {
            return Attempt::Blocked;
        }
        self.dock_active_at(item, target, label)
    }

    fn dock_active_at(
        &mut self,
        item: egui_dockspace::ItemId,
        target: egui_dockspace::ItemId,
        label: &str,
    ) -> Attempt {
        assert!(
            self.dock.has_active_gesture(),
            "target admission requires an already acquired drag"
        );
        let (direction, action) = match label {
            "Dockspace center drop guide" => {
                (DockspaceDropDirection::Center, DockspaceDropAction::Merge)
            }
            "Dockspace left drop guide" => (
                DockspaceDropDirection::Left,
                DockspaceDropAction::Split {
                    edge: DockEdge::Left,
                },
            ),
            "Dockspace right drop guide" => (
                DockspaceDropDirection::Right,
                DockspaceDropAction::Split {
                    edge: DockEdge::Right,
                },
            ),
            "Dockspace top drop guide" => (
                DockspaceDropDirection::Top,
                DockspaceDropAction::Split {
                    edge: DockEdge::Top,
                },
            ),
            "Dockspace bottom drop guide" => (
                DockspaceDropDirection::Bottom,
                DockspaceDropAction::Split {
                    edge: DockEdge::Bottom,
                },
            ),
            _ => panic!("unknown docking guide {label}"),
        };
        let destination_view = self.dock.view().item(target).unwrap();
        if !destination_view.is_active() {
            self.note(format!(
                "blocked {item}->{target}: source activation hid the destination"
            ));
            self.escape();
            return Attempt::Blocked;
        }
        let root = destination_view.root();
        let selected = destination_view
            .tabs()
            .expect("item is an inline tab")
            .selected()
            .and_then(|tab| tab.item())
            .unwrap();
        let source_root = self.dock.view().item(item).unwrap().root();
        let mut destination = self.panes.panes[&selected].content.unwrap().center();
        for _ in 0..16 {
            destination = self.panes.panes[&selected].content.unwrap().center();
            self.frame(vec![Event::PointerMoved(destination)]);
            let guide = self.drop_observations().and_then(|observed| {
                observed
                    .guides
                    .iter()
                    .find(|guide| {
                        guide.root == Some(root)
                            && guide.scope == DockspaceGuideScope::Inner
                            && guide.direction == direction
                    })
                    .copied()
            });
            let covered = self.covering_floating_roots(destination, source_root);
            if let Some(guide) = guide {
                if guide.eligibility == DockspaceDropEligibility::Rejected {
                    self.note(format!(
                        "{label} for {item}->{target} is explicitly rejected"
                    ));
                    self.escape();
                    return Attempt::Blocked;
                }
                let point = guide.rect.center();
                // Keep completed hit geometry and motion at the same instant.
                // The release is a separate packet, after inspecting this hover.
                self.frame_at(
                    self.context.input(|input| input.time),
                    vec![Event::PointerMoved(point)],
                );
                let admitted = self.drop_observations().is_some_and(|observed| {
                    observed.guides.iter().any(|current| {
                        current.id == guide.id
                            && current.active
                            && current.eligibility == DockspaceDropEligibility::Eligible
                    }) && observed.target.is_some_and(|preview| {
                        preview.surface() == SURFACE
                            && preview.root() == Some(root)
                            && preview.scope() == DockspaceDropTargetScope::Pane
                            && preview.selected_item() == Some(selected)
                            && preview.action() == action
                    })
                });
                if admitted {
                    self.note(format!("admit {item}->{target}: root={root:?}, selected={selected}, action={action:?}"));
                    self.frame_at(
                        self.context.input(|input| input.time),
                        vec![button(point, false)],
                    );
                    assert!(
                        !self.dock.has_active_gesture(),
                        "the admitted drop must finish its gesture"
                    );
                    return Attempt::Performed;
                }
            }
            if !covered.is_empty() {
                self.note(format!(
                    "blocked {item}->{target} root={root:?} at {destination:?}: covered by {covered:?}; observed={:?}",
                    self.drop_observations().and_then(|observed| observed.target),
                ));
                self.escape();
                return Attempt::Blocked;
            }
        }
        panic!(
            "an unobscured target must expose its requested {label}: {item}->{target}, root={root:?}, selected={selected}, body={destination:?}, observed={:?}",
            self.drop_observations()
                .and_then(|observed| observed.target),
        );
    }

    pub(super) fn split(&mut self) -> Attempt {
        let items = self.main_items();
        let mut choices = Vec::new();
        for &item in &items {
            let source = self.dock.view().item(item).unwrap();
            for &target in &items {
                if source
                    .tabs()
                    .expect("item is an inline tab")
                    .items()
                    .collect::<Vec<_>>()
                    .len()
                    > 1
                    && self.separate_active_destination(item, target)
                {
                    choices.push((item, target));
                }
            }
        }
        let Some((item, target)) = self.pick(&choices) else {
            return self.record("split", Attempt::NotApplicable);
        };
        let expected = MoveExpectation::capture(self, item);
        let direction = self.pick(&["left", "right", "top", "bottom"]).unwrap();
        let edge = match direction {
            "left" => DockEdge::Left,
            "right" => DockEdge::Right,
            "top" => DockEdge::Top,
            "bottom" => DockEdge::Bottom,
            _ => unreachable!("the complete edge roster contains only physical directions"),
        };
        self.note(format!("split {item} from {target} at {direction}"));
        let label = format!("Dockspace {direction} drop guide");
        let result = self.dock_at(item, target, &label);
        if result.performed() {
            expected.assert_docked(self, target, DockspaceDropAction::Split { edge });
            self.splits += 1;
        }
        self.record("split", result)
    }

    pub(super) fn merge(&mut self) -> Attempt {
        let items = self.main_items();
        let mut choices = Vec::new();
        for &item in &items {
            for &target in &items {
                if self.separate_active_destination(item, target) {
                    choices.push((item, target));
                }
            }
        }
        let Some((item, target)) = self.pick(&choices) else {
            return self.record("merge", Attempt::NotApplicable);
        };
        let expected = MoveExpectation::capture(self, item);
        self.note(format!("merge {item} into {target}"));
        let result = self.dock_at(item, target, "Dockspace center drop guide");
        if result.performed() {
            expected.assert_docked(self, target, DockspaceDropAction::Merge);
            self.merges += 1;
        }
        self.record("merge", result)
    }

    pub(super) fn float(&mut self) -> Attempt {
        let items = self.main_items();
        let Some(item) = self.pick(&items) else {
            return self.record("float", Attempt::NotApplicable);
        };
        self.float_item(item)
    }

    fn float_item(&mut self, item: egui_dockspace::ItemId) -> Attempt {
        let expected = MoveExpectation::capture(self, item);
        self.note(format!("Shift-drag {item} to float"));
        if !self.begin_drag(item, Modifiers::SHIFT) {
            return self.record("float", Attempt::Blocked);
        }
        let body = self.panes.panes[&item].content.unwrap();
        let point = body.min + body.size() * 0.65;
        self.frame(vec![Event::PointerMoved(point)]);
        self.frame(vec![Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::SHIFT,
        }]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        expected.assert_floated(self);
        assert!(!self.dock.has_active_gesture());
        self.floats += 1;
        self.record("float", Attempt::Performed)
    }

    pub(super) fn redock(&mut self) -> Attempt {
        let floating: Vec<_> = self
            .panes
            .panes
            .keys()
            .copied()
            .filter(|item| self.dock.view().item(*item).unwrap().contained().is_some())
            .collect();
        let Some(item) = self.pick(&floating) else {
            return self.record("redock", Attempt::NotApplicable);
        };
        let destinations: Vec<_> = self
            .main_items()
            .into_iter()
            .filter(|target| self.dock.view().item(*target).unwrap().is_active())
            .collect();
        let Some(target) = self.pick(&destinations) else {
            return self.record("redock", Attempt::NotApplicable);
        };
        let expected = MoveExpectation::capture(self, item);
        self.note(format!("redock floating {item} into {target}"));
        let result = self.dock_at(item, target, "Dockspace center drop guide");
        if result.performed() {
            expected.assert_docked(self, target, DockspaceDropAction::Merge);
        }
        self.record("redock", result)
    }
}

#[test]
fn required_gesture_sources_use_exposed_tabs_beside_a_covering_floating_window() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceContainedLayout, DockspaceLayout, DockspaceNode,
        DockspaceRootLayout, DockspaceSurfaceLayout, FloatingPresentationId, LogicalRect, RootId,
    };

    let floating_root = RootId::new(2);
    let fixture = |floating_rect| {
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                MAIN_ROOT,
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [
                        DockspaceNode::tabs([PROFILER]),
                        DockspaceNode::tabs([EDITOR]),
                    ],
                )
                .unwrap(),
            ),
        )
        .with_contained(DockspaceContainedLayout::new(
            FloatingPresentationId::new(1),
            DockspaceRootLayout::new(floating_root, DockspaceNode::tabs([INSPECTOR])),
            floating_rect,
        ))])
        .unwrap()
    };
    let mut replay = Replay::from_layout(
        0x0cc1_deca,
        fixture(LogicalRect::new(700.0, 250.0, 500.0, 400.0).unwrap()),
    );
    replay.settle();
    let (_, point) = replay.source(PROFILER).expect("Profiler starts exposed");
    // Keep part of the drag body authoritative so it remains an enabled
    // candidate, while the driver's center sample is behind the floating frame.
    let changed = replay
        .dock
        .reset_layout(fixture(
            LogicalRect::new(
                f64::from(point.x) - 4.0,
                f64::from(point.y) - 12.0,
                500.0,
                400.0,
            )
            .unwrap(),
        ))
        .unwrap();
    assert!(changed.workspace_changed());
    replay.settle();
    assert!(
        replay.main_items().contains(&PROFILER),
        "a partially covered tab remains an enabled candidate: source={:?}, widgets={:?}",
        replay.source(PROFILER),
        replay.panes.tab_widgets,
    );
    let (profiler_widget, covered) = replay.source(PROFILER).unwrap();
    assert_eq!(
        replay.covering_floating_roots(covered, MAIN_ROOT),
        [floating_root],
    );
    let bounds = replay
        .controls(Role::Tab, "Profiler")
        .into_iter()
        .find(|(widget, _)| *widget == profiler_widget)
        .unwrap()
        .1;
    let exposed_edge = egui::pos2(bounds.left() + 2.0, bounds.center().y);
    assert!(
        replay
            .covering_floating_roots(exposed_edge, MAIN_ROOT)
            .is_empty()
    );
    replay.frame_at(
        replay.context.input(|input| input.time),
        vec![Event::PointerMoved(exposed_edge)],
    );
    assert!(replay.context.viewport(|viewport| {
        viewport
            .hits
            .click
            .is_some_and(|hit| hit.id == profiler_widget)
            && viewport
                .hits
                .drag
                .is_some_and(|hit| hit.id == profiler_widget)
    }));
    let before = LayoutState::capture(&replay);
    assert!(replay.exposed_source(PROFILER).is_none());
    assert_eq!(LayoutState::capture(&replay), before);
    assert!(!replay.dock.has_active_gesture());

    let (item, source) = replay.exposed_main_source();
    assert_eq!(item, EDITOR);
    let widget = replay.source(item).unwrap().0;
    assert!(replay.context.viewport(|viewport| {
        viewport.hits.click.is_some_and(|hit| hit.id == widget)
            && viewport.hits.drag.is_some_and(|hit| hit.id == widget)
    }));
    replay.begin_drag_at(item, source, Modifiers::NONE);
    assert!(replay.dock.has_active_gesture());
    replay.escape();
    assert_eq!(LayoutState::capture(&replay), before);
}

#[test]
fn a_pane_source_with_the_same_title_as_its_layout_tab_merges_only_the_real_pane() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    };

    let outer = DockspaceNode::composed_tabs_with_selection(
        [
            DockspaceNode::pane(CONSOLE),
            DockspaceNode::tabs([INSPECTOR]).named("Inspector"),
        ],
        Some(1),
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [outer, DockspaceNode::tabs([EDITOR, PREVIEW])],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut replay = Replay::from_layout(0x1abe_1ed4, layout);
    replay.settle();
    let layout_tab = replay
        .dock
        .view()
        .item(CONSOLE)
        .unwrap()
        .tabs()
        .unwrap()
        .selected()
        .unwrap();
    assert!(layout_tab.is_layout());
    let layout_widget = replay.panes.tab_widgets[&layout_tab].1;
    let pane_widget = replay.panes.tab_widgets[&INSPECTOR.into()].1;
    let matching_titles = replay.controls(Role::Tab, "Inspector");
    assert_eq!(matching_titles.len(), 2);
    assert!(matching_titles.iter().any(|(id, _)| *id == layout_widget));
    assert!(matching_titles.iter().any(|(id, _)| *id == pane_widget));
    assert_ne!(layout_widget, pane_widget);
    assert_eq!(replay.source(INSPECTOR).unwrap().0, pane_widget);

    let expected = MoveExpectation::capture(&replay, INSPECTOR);
    assert_eq!(
        replay.dock_at(INSPECTOR, EDITOR, "Dockspace center drop guide"),
        Attempt::Performed,
    );
    expected.assert_docked(&replay, EDITOR, DockspaceDropAction::Merge);
    assert_eq!(
        replay
            .dock
            .view()
            .item(EDITOR)
            .unwrap()
            .tabs()
            .unwrap()
            .tabs(),
        [EDITOR.into(), PREVIEW.into(), INSPECTOR.into()],
    );
}

#[test]
fn source_activation_cannot_reuse_an_inactive_nested_panes_cached_guides() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    };

    let outer = DockspaceNode::composed_tabs_with_selection(
        [
            DockspaceNode::pane(PROFILER),
            DockspaceNode::pane(editor::ASSETS),
            DockspaceNode::tabs([INSPECTOR]).named("Inspector"),
        ],
        Some(2),
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [DockspaceNode::tabs([EDITOR, PREVIEW]), outer],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut replay = Replay::from_layout(0x1aac_71ae, layout);
    replay.settle();
    assert!(replay.dock.view().item(INSPECTOR).unwrap().is_active());
    assert!(!replay.separate_active_destination(editor::ASSETS, INSPECTOR));
    assert!(replay.separate_active_destination(editor::ASSETS, EDITOR));
    let cached_content = replay.panes.panes[&INSPECTOR].content.unwrap();

    assert!(replay.begin_drag(editor::ASSETS, Modifiers::NONE));
    assert!(replay.dock.view().item(editor::ASSETS).unwrap().is_active());
    let inspector = replay.dock.view().item(INSPECTOR).unwrap();
    assert!(!inspector.is_active());
    assert_eq!(inspector.tabs().unwrap().selected(), Some(INSPECTOR.into()));
    assert_eq!(replay.panes.panes[&INSPECTOR].content, Some(cached_content));

    // The cached content now overlaps the selected ancestor's real body.
    // Its same-direction guide belongs to Assets, not the inactive Inspector.
    replay.frame(vec![Event::PointerMoved(cached_content.center())]);
    let mut guides = replay
        .drop_observations()
        .unwrap()
        .guides
        .iter()
        .filter(|guide| {
            guide.root == Some(MAIN_ROOT)
                && guide.scope == DockspaceGuideScope::Inner
                && guide.direction == DockspaceDropDirection::Right
        });
    let guide = *guides
        .next()
        .expect("the active ancestor has a right guide");
    assert!(guides.next().is_none(), "only one Inner cluster is exposed");
    replay.frame_at(
        replay.context.input(|input| input.time),
        vec![Event::PointerMoved(guide.rect.center())],
    );
    assert_eq!(
        replay
            .drop_observations()
            .unwrap()
            .target
            .unwrap()
            .selected_item(),
        Some(editor::ASSETS),
    );
    let before = LayoutState::capture(&replay);
    assert_eq!(
        replay.dock_active_at(editor::ASSETS, INSPECTOR, "Dockspace right drop guide"),
        Attempt::Blocked,
        "an inactive nested target cannot authorize its ancestor's same-label guide",
    );
    assert_eq!(LayoutState::capture(&replay), before);
    assert!(!replay.dock.has_active_gesture());
}

#[test]
fn an_edge_drop_wraps_a_named_layout_in_place_and_preserves_the_original_target() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    };

    let outer = DockspaceNode::composed_tabs_with_selection(
        [
            DockspaceNode::pane(PROFILER),
            DockspaceNode::tabs([INSPECTOR]).named("Inspector"),
            DockspaceNode::pane(editor::ASSETS),
        ],
        Some(1),
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [DockspaceNode::tabs([EDITOR, PREVIEW]), outer],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut replay = Replay::from_layout(0x1a70_07ab, layout);
    replay.settle();
    let expected = MoveExpectation::capture(&replay, PREVIEW);
    assert_eq!(
        replay.dock_at(PREVIEW, INSPECTOR, "Dockspace bottom drop guide"),
        Attempt::Performed,
    );
    expected.assert_docked(
        &replay,
        INSPECTOR,
        DockspaceDropAction::Split {
            edge: DockEdge::Bottom,
        },
    );
}

#[test]
fn emptying_a_layout_split_branch_retains_one_named_boundary_group() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
        DockspaceTabView,
    };

    let nested = DockspaceNode::equal_split(
        DockspaceAxis::Vertical,
        [
            DockspaceNode::tabs([INSPECTOR]),
            DockspaceNode::tabs([PREVIEW]),
        ],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::composed_tabs_with_selection(
                [
                    DockspaceNode::pane(PROFILER),
                    nested,
                    DockspaceNode::pane(editor::ASSETS),
                ],
                Some(1),
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut replay = Replay::from_layout(0xb0ad_da12, layout);
    replay.settle();
    let outer = replay.dock.view().item(PROFILER).unwrap().tabs().unwrap();
    let before_tabs = outer.tabs().to_vec();
    let boundary = outer.selected().unwrap();
    let Some(DockspaceTabView::Layout(node)) = outer.child(boundary) else {
        panic!("Tools starts as an independent layout tab");
    };
    let boundary_key = node.key();
    assert!(node.split().is_some());
    let expected = MoveExpectation::capture(&replay, INSPECTOR);
    assert_eq!(
        replay.dock_at(INSPECTOR, PREVIEW, "Dockspace center drop guide"),
        Attempt::Performed,
    );
    expected.assert_docked(&replay, PREVIEW, DockspaceDropAction::Merge);
    let outer = replay.dock.view().item(PROFILER).unwrap().tabs().unwrap();
    assert_eq!(outer.tabs(), before_tabs);
    assert_eq!(outer.selected(), Some(boundary));
    let Some(DockspaceTabView::Layout(node)) = outer.child(boundary) else {
        panic!("the same Tools tab keeps its complete remaining layout");
    };
    assert_eq!(node.key(), boundary_key);
    assert_eq!(node.layout_title(), Some("Tools"));
    assert_eq!(node.tabs().unwrap().tabs().len(), 1);
    assert_eq!(node.items().collect::<Vec<_>>(), [PREVIEW, INSPECTOR]);
}

#[test]
fn required_workbench_drops_preserve_payload_order_selection_and_unrelated_groups() {
    let mut replay = Replay::new(0x5e6a_471c);
    for (item, target, label, action) in [
        (
            INSPECTOR,
            EDITOR,
            "Dockspace center drop guide",
            DockspaceDropAction::Merge,
        ),
        (
            PREVIEW,
            OUTLINE,
            "Dockspace left drop guide",
            DockspaceDropAction::Split {
                edge: DockEdge::Left,
            },
        ),
    ] {
        replay.settle();
        let expected = MoveExpectation::capture(&replay, item);
        assert_eq!(
            replay.dock_at(item, target, label),
            Attempt::Performed,
            "the fixed unobscured fixture must expose its required docking guide"
        );
        expected.assert_docked(&replay, target, action);
    }
    replay.settle();
    assert_eq!(
        replay.float_item(PREVIEW),
        Attempt::Performed,
        "the required detached pane must complete an actual Shift-drag"
    );
    replay.settle();
    let editor_title = replay.panes.title(EDITOR).unwrap().text().to_owned();
    let editor_tab = replay.controls(Role::Tab, &editor_title)[0].0;
    replay.frame(vec![Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: editor_tab.accesskit_id(),
            data: None,
        },
    )]);
    replay.settle();
    assert!(replay.dock.view().item(EDITOR).unwrap().is_active());
    let expected = MoveExpectation::capture(&replay, PREVIEW);
    assert_eq!(
        replay.dock_at(PREVIEW, EDITOR, "Dockspace center drop guide"),
        Attempt::Performed
    );
    expected.assert_docked(&replay, EDITOR, DockspaceDropAction::Merge);
    replay.assert_items();
}

#[test]
fn an_occluded_main_target_cannot_release_into_a_floating_roots_guide() {
    use egui_dockspace::{
        DockspaceAxis, DockspaceContainedLayout, DockspaceLayout, DockspaceNode,
        DockspaceRootLayout, DockspaceSurfaceLayout, FloatingPresentationId, LogicalRect, RootId,
    };

    let floating_root = RootId::new(2);
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            MAIN_ROOT,
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (
                        DockspaceNode::tabs_with_selection([EDITOR, PREVIEW], Some(PREVIEW))
                            .unwrap(),
                        0.35,
                    ),
                    (DockspaceNode::tabs([OUTLINE]), 0.65),
                ],
            )
            .unwrap(),
        ),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(floating_root, DockspaceNode::tabs([INSPECTOR])),
        LogicalRect::new(640.0, 210.0, 500.0, 420.0).unwrap(),
    ))])
    .unwrap();
    let mut replay = Replay::from_layout(0x0cc1_0ded, layout);
    replay.settle();
    let point = replay.panes.panes[&OUTLINE].content.unwrap().center();
    assert_eq!(
        replay.covering_floating_roots(point, MAIN_ROOT),
        [floating_root]
    );

    // Establish that the identical visible label really belongs to the
    // foreground root, rather than merely testing a missing-guide path.
    assert!(replay.begin_drag(PREVIEW, Modifiers::NONE));
    replay.frame(vec![Event::PointerMoved(point)]);
    let observed = replay
        .drop_observations()
        .expect("the covered point must expose foreground docking feedback");
    assert!(observed.guides.iter().any(|guide| {
        guide.root == Some(floating_root)
            && guide.scope == DockspaceGuideScope::Inner
            && guide.direction == DockspaceDropDirection::Center
            && guide.eligibility == DockspaceDropEligibility::Eligible
    }));
    assert_eq!(observed.target.unwrap().root(), Some(floating_root));
    replay.escape();
    let before = LayoutState::capture(&replay);

    assert!(
        replay.begin_drag(PREVIEW, Modifiers::NONE),
        "the occlusion check must begin the required drag before testing target admission"
    );
    assert_eq!(
        replay.dock_active_at(PREVIEW, OUTLINE, "Dockspace center drop guide"),
        Attempt::Blocked,
        "the foreground root's same-label guide must never authorize the requested main-pane release",
    );
    before.assert_equivalent_geometry(&LayoutState::capture(&replay));
    assert!(!replay.dock.has_active_gesture());
    assert!(
        replay
            .dock
            .view()
            .item(PREVIEW)
            .unwrap()
            .contained()
            .is_none()
    );
}
