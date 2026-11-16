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
        let root = destination_view.root();
        let selected = destination_view.tabs().selected().unwrap();
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
                let destination = self.dock.view().item(target).unwrap();
                if source.tabs().items().len() > 1
                    && destination.is_selected()
                    && !source.tabs().items().contains(&target)
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
        self.note(format!("split {item} from {target} at {direction}"));
        let label = format!("Dockspace {direction} drop guide");
        let result = self.dock_at(item, target, &label);
        if result.performed() {
            expected.assert_docked(self, target, false);
            self.splits += 1;
        }
        self.record("split", result)
    }

    pub(super) fn merge(&mut self) -> Attempt {
        let items = self.main_items();
        let mut choices = Vec::new();
        for &item in &items {
            for &target in &items {
                let source = self.dock.view().item(item).unwrap();
                let destination = self.dock.view().item(target).unwrap();
                if destination.is_selected() && !source.tabs().items().contains(&target) {
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
            expected.assert_docked(self, target, true);
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
            .filter(|target| self.dock.view().item(*target).unwrap().is_selected())
            .collect();
        let Some(target) = self.pick(&destinations) else {
            return self.record("redock", Attempt::NotApplicable);
        };
        let expected = MoveExpectation::capture(self, item);
        self.note(format!("redock floating {item} into {target}"));
        let result = self.dock_at(item, target, "Dockspace center drop guide");
        if result.performed() {
            expected.assert_docked(self, target, true);
        }
        self.record("redock", result)
    }
}

#[test]
fn required_workbench_drops_preserve_payload_order_selection_and_unrelated_groups() {
    let mut replay = Replay::new(0x5e6a_471c);
    for (item, target, label, merged) in [
        (INSPECTOR, EDITOR, "Dockspace center drop guide", true),
        (PREVIEW, OUTLINE, "Dockspace left drop guide", false),
    ] {
        replay.settle();
        let expected = MoveExpectation::capture(&replay, item);
        assert_eq!(
            replay.dock_at(item, target, label),
            Attempt::Performed,
            "the fixed unobscured fixture must expose its required docking guide"
        );
        expected.assert_docked(&replay, target, merged);
    }
    replay.settle();
    assert_eq!(
        replay.float_item(PREVIEW),
        Attempt::Performed,
        "the required detached pane must complete an actual Shift-drag"
    );
    replay.settle();
    let expected = MoveExpectation::capture(&replay, PREVIEW);
    assert_eq!(
        replay.dock_at(PREVIEW, EDITOR, "Dockspace center drop guide"),
        Attempt::Performed
    );
    expected.assert_docked(&replay, EDITOR, true);
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
