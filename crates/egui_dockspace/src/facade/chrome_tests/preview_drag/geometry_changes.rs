//! Coordinate changes cancel old Grid input without stranding fresh receivers.

use super::*;

#[derive(Clone, Copy, Debug)]
enum GeometryChange {
    NativeDpi,
    UiZoom,
    NarrowWindow,
}

impl GeometryChange {
    fn apply(self, harness: &mut GridHarness) -> f32 {
        let physical_size = vec2(1100.0, 760.0);
        let (size, pixels_per_point) = match self {
            Self::NativeDpi => {
                harness.native_pixels_per_point = Some(1.25);
                (physical_size / 1.25, 1.25)
            }
            Self::UiZoom => {
                harness.context.set_zoom_factor(1.5);
                (physical_size / 1.5, 1.5)
            }
            Self::NarrowWindow => (vec2(640.0, 760.0), 1.0),
        };
        harness.screen_rect = Rect::from_min_size(Pos2::ZERO, size);
        pixels_per_point
    }

    fn restore(self, harness: &mut GridHarness) {
        match self {
            Self::NativeDpi => harness.native_pixels_per_point = Some(1.0),
            Self::UiZoom => harness.context.set_zoom_factor(1.0),
            Self::NarrowWindow => {}
        }
        harness.screen_rect = Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0));
        let output = harness.run(Vec::new());
        assert!((output.pixels_per_point - 1.0).abs() < f32::EPSILON);
        harness.assert_geometry_ownership_and_activity();
    }
}

#[derive(Clone, Copy, Debug)]
enum Cancel {
    Escape,
    FocusLoss,
}

impl Cancel {
    fn event(self) -> Event {
        match self {
            Self::Escape => key(Key::Escape),
            Self::FocusLoss => Event::WindowFocused(false),
        }
    }
}

impl GridHarness {
    fn assert_geometry_ownership_and_activity(&self) {
        let view = self.dockspace.view();
        let surface = view.surface(SURFACE).unwrap();
        assert_eq!(view.surfaces().count(), 1);
        assert_eq!(surface.contained_count(), 0);
        let root = view.root(ROOT).unwrap().content().unwrap();
        assert_eq!(root.key(), self.grid);
        assert_eq!(
            root.grid()
                .unwrap()
                .children()
                .map(|child| child.key())
                .collect::<Vec<_>>(),
            self.cells
        );
        let mut owned = root.items().collect::<Vec<_>>();
        owned.sort_unstable();
        assert_eq!(owned, ITEMS, "all seven real panes remain uniquely owned");
        let active = ITEMS
            .into_iter()
            .filter(|item| view.item(*item).unwrap().is_active())
            .collect::<Vec<_>>();
        self.assert_active(&active);
        let mut callbacks = self.panes.body_calls.clone();
        callbacks.sort_unstable();
        callbacks.dedup();
        assert_eq!(callbacks, active);
    }

    fn assert_equal_grid_partition(&self) {
        let screen = self.screen_rect;
        let gap = self.dockspace.style().splitter_thickness;
        let size = (screen.size() - vec2(gap, gap)) * 0.5;
        let top_left = Rect::from_min_size(screen.min, size);
        let top_right = Rect::from_min_size(screen.min + vec2(size.x + gap, 0.0), size);
        let bottom_left = Rect::from_min_size(screen.min + vec2(0.0, size.y + gap), size);
        let mut bottom_right = Rect::from_min_size(screen.min + size + vec2(gap, gap), size);
        assert_rect_close(self.panes.bodies[&A], top_left, &self.context);
        assert_rect_close(self.panes.bodies[&D], bottom_left, &self.context);
        let document = if self.dockspace.view().item(B).unwrap().is_active() {
            B
        } else {
            C
        };
        let mut document_content = top_right;
        document_content.min.y += self.dockspace.style().tab_bar_height;
        assert_rect_close(
            self.panes.bodies[&document],
            document_content,
            &self.context,
        );
        bottom_right.min.y += self.dockspace.style().tab_bar_height;
        if self.dockspace.view().item(E).unwrap().is_active() {
            assert_rect_close(self.panes.bodies[&E], bottom_right, &self.context);
        } else {
            let tool_width = (bottom_right.width() - gap) * 0.5;
            let leading =
                Rect::from_min_size(bottom_right.min, vec2(tool_width, bottom_right.height()));
            let trailing = Rect::from_min_max(
                bottom_right.min + vec2(tool_width + gap, 0.0),
                bottom_right.max,
            );
            assert_rect_close(self.panes.bodies[&F], leading, &self.context);
            assert_rect_close(self.panes.bodies[&G], trailing, &self.context);
        }
    }

    fn cancel_with_geometry_change(
        &mut self,
        change: GeometryChange,
        cancel: Cancel,
        stale_release: Pos2,
    ) {
        let version = self.dockspace.version();
        let active = ITEMS
            .into_iter()
            .filter(|item| self.dockspace.view().item(*item).unwrap().is_active())
            .collect::<Vec<_>>();
        let pixels_per_point = change.apply(self);
        let output = self.run(vec![cancel.event()]);
        assert!(
            (output.pixels_per_point - pixels_per_point).abs() < f32::EPSILON,
            "{change:?}/{cancel:?} must render at the new scale"
        );
        assert!(
            !self.dockspace.has_active_gesture(),
            "{change:?}/{cancel:?}"
        );
        assert!(self.panes.preview.is_none());
        assert_eq!(self.dockspace.version(), version);
        self.assert_grid(self.cells);
        self.assert_active(&active);
        self.assert_equal_grid_partition();
        self.assert_geometry_ownership_and_activity();

        self.run(vec![
            Event::WindowFocused(true),
            button(stale_release, false),
            Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(!self.dockspace.has_active_gesture());
        assert!(self.panes.preview.is_none());
        assert!(!self.context.input(|input| input.pointer.primary_down()));
        assert_eq!(
            self.dockspace.version(),
            version,
            "the old coordinate's release cannot finish a cancelled gesture"
        );
        self.assert_geometry_ownership_and_activity();
    }

    fn begin_grid_resize(&mut self, orientation: Orientation) -> (SplitterControl, Pos2) {
        let control = self
            .splitters
            .iter()
            .filter(|control| control.orientation == orientation)
            .max_by(|left, right| {
                let span = |control: &SplitterControl| match orientation {
                    Orientation::Horizontal => control.rect.width(),
                    Orientation::Vertical => control.rect.height(),
                };
                span(left).total_cmp(&span(right))
            })
            .copied()
            .expect("the two-track Grid exposes its operable shared boundary");
        let point = match orientation {
            Orientation::Horizontal => egui::pos2(
                control.rect.left() + control.rect.width() * 0.25,
                control.rect.center().y,
            ),
            Orientation::Vertical => egui::pos2(
                control.rect.center().x,
                control.rect.top() + control.rect.height() * 0.25,
            ),
        };
        self.run(vec![Event::PointerMoved(point)]);
        assert_eq!(
            self.context
                .viewport(|viewport| { viewport.hits.drag.map(|hit| hit.id.accesskit_id()) }),
            Some(control.id),
            "the shared boundary must win the actual hit graph before pressing"
        );
        self.run(vec![button(point, true)]);
        assert!(self.dockspace.has_active_gesture());
        (control, point)
    }

    fn resize_current_grid(&mut self, orientation: Orientation, delta: f32) {
        let before = self.panes.bodies.clone();
        let version = self.dockspace.version();
        let (control, point) = self.begin_grid_resize(orientation);
        let end = point
            + match orientation {
                Orientation::Horizontal => vec2(0.0, delta),
                Orientation::Vertical => vec2(delta, 0.0),
            };
        self.run(vec![Event::PointerMoved(end)]);
        assert!(self.dockspace.has_active_gesture());
        self.release(end);
        assert_ne!(self.dockspace.version(), version);
        let updated = self
            .splitters
            .iter()
            .find(|splitter| splitter.id == control.id)
            .expect("the released Grid boundary retains its numeric range");
        assert!(updated.minimum < updated.value && updated.value < updated.maximum);
        assert!((updated.value - control.value) * f64::from(delta) > 0.0);
        for (item, mut expected) in before {
            match orientation {
                Orientation::Horizontal if matches!(item, A | B | C) => {
                    expected.max.y += delta;
                }
                Orientation::Horizontal if matches!(item, D | E) => {
                    expected.min.y += delta;
                }
                Orientation::Vertical if matches!(item, A | D) => {
                    expected.max.x += delta;
                }
                Orientation::Vertical if matches!(item, B | C | E) => {
                    expected.min.x += delta;
                }
                _ => panic!("the resize fixture uses four visible cell contents"),
            }
            assert_rect_close(self.panes.bodies[&item], expected, &self.context);
        }
        self.assert_geometry_ownership_and_activity();
    }
}

fn assert_rect_close(actual: Rect, expected: Rect, context: &Context) {
    let tolerance = 1.0 / context.pixels_per_point() + 0.001;
    for (actual, expected) in [
        (actual.left(), expected.left()),
        (actual.top(), expected.top()),
        (actual.right(), expected.right()),
        (actual.bottom(), expected.bottom()),
    ] {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected coordinate {expected}, got {actual} with tolerance {tolerance}"
        );
    }
}

#[test]
fn shared_grid_geometry_changes_cancel_docking_and_restore_fresh_receivers() {
    for change in [
        GeometryChange::NativeDpi,
        GeometryChange::UiZoom,
        GeometryChange::NarrowWindow,
    ] {
        for layout_tab in [false, true] {
            let mut harness = GridHarness::new();
            let source = if layout_tab { harness.tools } else { C.into() };
            harness.begin_drag(source);
            let point = harness.hover_guide(A, DockspaceDropDirection::Center);
            harness.assert_preview(point, DockspaceDropAction::Merge, &[], None, &[]);
            assert_eq!(
                harness
                    .panes
                    .preview
                    .as_ref()
                    .unwrap()
                    .target
                    .selected_item(),
                Some(A)
            );
            let cancel = if layout_tab {
                Cancel::FocusLoss
            } else {
                Cancel::Escape
            };
            harness.cancel_with_geometry_change(change, cancel, point);

            harness.click_tab(B.into());
            harness.click_tab(E.into());
            harness.assert_active(&[A, B, D, E]);
            harness.resize_current_grid(Orientation::Horizontal, 12.0);
            let shares = harness
                .dockspace
                .view()
                .node(harness.grid)
                .unwrap()
                .grid()
                .unwrap()
                .shares()
                .cloned();
            assert!(shares.is_some());
            change.restore(&mut harness);
            assert_eq!(
                harness
                    .dockspace
                    .view()
                    .node(harness.grid)
                    .unwrap()
                    .grid()
                    .unwrap()
                    .shares(),
                shares.as_ref(),
                "restoring the viewport preserves committed user shares"
            );
            harness.click_tab(C.into());
            harness.assert_active(&[A, C, D, E]);
            harness.begin_drag(harness.tools);
            let point = harness.hover_guide(A, DockspaceDropDirection::Center);
            harness.assert_preview(point, DockspaceDropAction::Merge, &[], None, &[]);
            harness.release(point);
            harness.assert_tabs(harness.cells[0], &[A.into(), harness.tools], harness.tools);
            harness.assert_tabs(harness.cells[1], &[B.into(), C.into()], C.into());
            harness.assert_tabs(harness.cells[3], &[E.into()], E.into());
            harness.assert_active(&[C, D, E, F, G]);
            harness.assert_geometry_ownership_and_activity();
        }
    }
}

#[test]
fn shared_grid_geometry_changes_cancel_resize_and_preserve_shared_tracks() {
    for change in [
        GeometryChange::NativeDpi,
        GeometryChange::UiZoom,
        GeometryChange::NarrowWindow,
    ] {
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let mut harness = GridHarness::new();
            let before = harness.panes.bodies.clone();
            let (_, point) = harness.begin_grid_resize(orientation);
            let end = point
                + match orientation {
                    Orientation::Horizontal => vec2(0.0, 24.0),
                    Orientation::Vertical => vec2(24.0, 0.0),
                };
            harness.run(vec![Event::PointerMoved(end)]);
            assert!(harness.dockspace.has_active_gesture());
            assert_ne!(harness.panes.bodies, before);
            let cancel = match orientation {
                Orientation::Horizontal => Cancel::Escape,
                Orientation::Vertical => Cancel::FocusLoss,
            };
            harness.cancel_with_geometry_change(change, cancel, end);
            harness.assert_active(&[A, B, D, E]);
            harness.click_tab(C.into());
            harness.assert_active(&[A, C, D, E]);
            harness.resize_current_grid(orientation, -12.0);
            let shares = harness
                .dockspace
                .view()
                .node(harness.grid)
                .unwrap()
                .grid()
                .unwrap()
                .shares()
                .cloned();
            assert!(shares.is_some());
            change.restore(&mut harness);
            assert_eq!(
                harness
                    .dockspace
                    .view()
                    .node(harness.grid)
                    .unwrap()
                    .grid()
                    .unwrap()
                    .shares(),
                shares.as_ref()
            );
            harness.click_tab(B.into());
            harness.assert_active(&[A, B, D, E]);
            harness.assert_geometry_ownership_and_activity();
        }
    }
}
