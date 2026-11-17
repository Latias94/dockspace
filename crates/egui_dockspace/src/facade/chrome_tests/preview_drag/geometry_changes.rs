//! Grid gestures retain measurement-only changes and recover after explicit cancellation.

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
        self.assert_grid_partition(self.screen_rect, 0.5, 0.5);
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the expected pane partition is finite and bounded by the test viewport"
    )]
    fn assert_grid_partition(&self, screen: Rect, row_share: f64, column_share: f64) {
        let gap = self.dockspace.style().splitter_thickness;
        let content = screen.size() - vec2(gap, gap);
        let leading = vec2(
            (f64::from(content.x) * column_share) as f32,
            (f64::from(content.y) * row_share) as f32,
        );
        let mut top_left = Rect::from_min_size(screen.min, leading);
        if self.dockspace.view().item(A).unwrap().tabs().is_some() {
            top_left.min.y += self.dockspace.style().tab_bar_height;
        }
        let top_right = Rect::from_min_max(
            screen.min + vec2(leading.x + gap, 0.0),
            egui::pos2(screen.right(), screen.top() + leading.y),
        );
        let bottom_left = Rect::from_min_max(
            screen.min + vec2(0.0, leading.y + gap),
            egui::pos2(screen.left() + leading.x, screen.bottom()),
        );
        let mut bottom_right =
            Rect::from_min_max(screen.min + leading + vec2(gap, gap), screen.max);
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

#[test]
fn shared_grid_geometry_changes_during_resize_do_not_require_an_explicit_cancel() {
    // RawInput changes local measurements without replacing native coordinate
    // authority. A changed resize projection can require new measurements;
    // its frozen solver shares overlay the new bounds until release commits them.
    for change in [
        GeometryChange::NativeDpi,
        GeometryChange::UiZoom,
        GeometryChange::NarrowWindow,
    ] {
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let mut harness = GridHarness::new();
            let frozen_screen = harness.screen_rect;
            let gap = harness.dockspace.style().splitter_thickness;
            let (row_share, column_share) = match orientation {
                Orientation::Horizontal => {
                    (0.5 + 24.0 / f64::from(frozen_screen.height() - gap), 0.5)
                }
                Orientation::Vertical => (0.5, 0.5 + 24.0 / f64::from(frozen_screen.width() - gap)),
            };
            let (_, point) = harness.begin_grid_resize(orientation);
            let end = point
                + match orientation {
                    Orientation::Horizontal => vec2(0.0, 24.0),
                    Orientation::Vertical => vec2(24.0, 0.0),
                };
            harness.run(vec![Event::PointerMoved(end)]);
            assert!(harness.dockspace.has_active_gesture());
            harness.assert_grid_partition(frozen_screen, row_share, column_share);
            let version = harness.dockspace.version();
            let pixels_per_point = change.apply(&mut harness);
            let output = harness.run(Vec::new());
            assert!(
                (output.pixels_per_point - pixels_per_point).abs() < f32::EPSILON,
                "{change:?}/{orientation:?}"
            );
            assert_eq!(harness.dockspace.version(), version);
            assert!(
                harness.dockspace.has_active_gesture(),
                "measurement-only {change:?}/{orientation:?} must retain the frozen resize",
            );
            assert!(
                harness
                    .dockspace
                    .view()
                    .node(harness.grid)
                    .unwrap()
                    .grid()
                    .unwrap()
                    .shares()
                    .is_none(),
                "the retained preview cannot publish persistent shares"
            );
            harness.assert_grid_partition(harness.screen_rect, row_share, column_share);
            harness.assert_tabs(harness.cells[1], &[B.into(), C.into()], B.into());
            harness.assert_tabs(harness.cells[3], &[E.into(), harness.tools], E.into());
            harness.assert_active(&[A, B, D, E]);
            harness.assert_geometry_ownership_and_activity();
            harness.run(vec![button(end, false)]);
            assert!(!harness.dockspace.has_active_gesture());
            assert!(harness.panes.preview.is_none());
            assert_ne!(harness.dockspace.version(), version);
            let shares = harness
                .dockspace
                .view()
                .node(harness.grid)
                .unwrap()
                .grid()
                .unwrap()
                .shares()
                .expect("release commits the pointer displacement in the frozen track space");
            for (actual, expected) in shares.rows().chain(shares.columns()).zip([
                row_share,
                1.0 - row_share,
                column_share,
                1.0 - column_share,
            ]) {
                assert!(
                    (actual - expected).abs() < 1e-12,
                    "{change:?}/{orientation:?}: expected frozen share {expected}, got {actual}"
                );
            }
            harness.assert_grid_partition(harness.screen_rect, row_share, column_share);
            harness.assert_tabs(harness.cells[1], &[B.into(), C.into()], B.into());
            harness.assert_tabs(harness.cells[3], &[E.into(), harness.tools], E.into());
            harness.assert_active(&[A, B, D, E]);
            harness.assert_geometry_ownership_and_activity();
            harness.settle();
            harness.assert_grid_partition(harness.screen_rect, row_share, column_share);
            harness.assert_tabs(harness.cells[1], &[B.into(), C.into()], B.into());
            harness.assert_tabs(harness.cells[3], &[E.into(), harness.tools], E.into());
            harness.assert_active(&[A, B, D, E]);
            harness.click_tab(C.into());
            harness.click_tab(B.into());
            harness.resize_current_grid(orientation, -12.0);
            change.restore(&mut harness);
            harness.assert_geometry_ownership_and_activity();
        }
    }
}

#[test]
fn shared_grid_geometry_changes_during_docking_keep_fresh_targets_operable() {
    // The same host pointer stream continues against its retained logical scene;
    // a physical-coordinate epoch change belongs to native-host tests.
    for change in [
        GeometryChange::NativeDpi,
        GeometryChange::UiZoom,
        GeometryChange::NarrowWindow,
    ] {
        for layout_tab in [false, true] {
            let mut harness = GridHarness::new();
            let frozen_screen = harness.screen_rect;
            let source = if layout_tab { harness.tools } else { C.into() };
            harness.begin_drag(source);
            let frozen_point = harness.hover_guide(A, DockspaceDropDirection::Center);
            harness.assert_preview(frozen_point, DockspaceDropAction::Merge, &[], None, &[]);
            let frozen_target = harness.panes.preview.as_ref().unwrap().target_rect;
            let frozen_preview = harness.panes.preview.as_ref().unwrap().rect;
            let before = harness.layout_signature();
            let version = harness.dockspace.version();
            let pixels_per_point = change.apply(&mut harness);
            let output = harness.run(Vec::new());
            assert!(
                (output.pixels_per_point - pixels_per_point).abs() < f32::EPSILON,
                "{change:?}, layout_tab={layout_tab}"
            );
            assert_eq!(harness.dockspace.version(), version);
            assert_eq!(harness.layout_signature(), before);
            assert!(
                harness.dockspace.has_active_gesture(),
                "measurement-only {change:?}, layout_tab={layout_tab} must retain the source",
            );
            harness.assert_preview(frozen_point, DockspaceDropAction::Merge, &[], None, &[]);
            assert_eq!(
                harness.panes.preview.as_ref().unwrap().target_rect,
                frozen_target
            );
            assert_eq!(harness.panes.preview.as_ref().unwrap().rect, frozen_preview);
            harness.assert_grid_partition(frozen_screen, 0.5, 0.5);
            harness.assert_geometry_ownership_and_activity();
            let point = harness.hover_guide(A, DockspaceDropDirection::Center);
            assert_eq!(
                point, frozen_point,
                "the retained drag resolves the same exact logical guide before measurement refresh"
            );
            harness.assert_preview(point, DockspaceDropAction::Merge, &[], None, &[]);
            harness.release(point);
            harness.assert_tabs(harness.cells[0], &[A.into(), source], source);
            let expected_documents: Vec<TabKey> = if layout_tab {
                vec![B.into(), C.into()]
            } else {
                vec![B.into()]
            };
            let expected_outer: Vec<TabKey> = if layout_tab {
                vec![E.into()]
            } else {
                vec![E.into(), harness.tools]
            };
            harness.assert_tabs(harness.cells[1], &expected_documents, B.into());
            harness.assert_tabs(harness.cells[3], &expected_outer, E.into());
            assert_ne!(harness.dockspace.version(), version);
            harness.assert_geometry_ownership_and_activity();
            harness.click_tab(A.into());
            harness.assert_equal_grid_partition();
            change.restore(&mut harness);
            harness.assert_equal_grid_partition();
            harness.assert_geometry_ownership_and_activity();
        }
    }
}
