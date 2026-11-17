//! A new Auto cell map retires an active resize before painting or late release.

use super::*;

const AUTO_ITEMS: [ItemId; 5] = [A, B, C, D, E];

struct AutoGridHarness {
    context: Context,
    dockspace: Dockspace,
    panes: GridPanes,
    frame: u32,
    screen_rect: Rect,
    grid: NodeKey,
    cells: [NodeKey; 5],
    splitters: Vec<SplitterControl>,
}

impl AutoGridHarness {
    fn new() -> Self {
        let content = DockspaceNode::grid(
            DockspaceGridShape::Auto,
            AUTO_ITEMS.map(DockspaceNode::pane),
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap();
        let style = DockStyle {
            splitter_thickness: 4.0,
            minimum_pane_size: vec2(12.0, 12.0),
            ..DockStyle::default()
        };
        let dockspace = Dockspace::builder("auto-grid-measurement-change", layout)
            .style(style)
            .build()
            .unwrap();
        let root = dockspace.view().root(ROOT).unwrap().content().unwrap();
        let grid = root.key();
        let cells = root
            .grid()
            .unwrap()
            .children()
            .map(|child| child.key())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let context = Context::default();
        context.enable_accesskit();
        let mut harness = Self {
            context,
            dockspace,
            panes: GridPanes::default(),
            frame: 0,
            screen_rect: Rect::from_min_size(Pos2::ZERO, vec2(404.0, 204.0)),
            grid,
            cells,
            splitters: Vec::new(),
        };
        harness.settle();
        harness.assert_partition(&[0.5, 0.5], &[1.0 / 3.0; 3]);
        harness
    }

    fn run(&mut self, events: Vec<Event>) {
        self.frame += 1;
        self.panes.clear_frame_observations();
        let (output, _) = self
            .dockspace
            .run_single_surface(
                &self.context,
                RawInput {
                    screen_rect: Some(self.screen_rect),
                    time: Some(f64::from(self.frame) * 0.1),
                    events,
                    ..Default::default()
                },
                SURFACE,
                &mut self.panes,
            )
            .unwrap_or_else(|error| panic!("Auto Grid pointer frame {}: {error}", self.frame));
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        self.splitters = splitter_controls(&output);
        output.textures_delta.clear();
        self.assert_ownership_and_callbacks();
    }

    fn settle(&mut self) {
        self.run(Vec::new());
        self.run(Vec::new());
    }

    fn assert_ownership_and_callbacks(&self) {
        let view = self.dockspace.view();
        assert_eq!(view.surfaces().count(), 1);
        assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 0);
        let root = view.root(ROOT).unwrap().content().unwrap();
        assert_eq!(root.key(), self.grid);
        let grid = root.grid().unwrap();
        assert_eq!(grid.shape(), DockspaceGridShape::Auto);
        assert_eq!(
            grid.children().map(|child| child.key()).collect::<Vec<_>>(),
            self.cells
        );
        assert_eq!(root.items().collect::<Vec<_>>(), AUTO_ITEMS);
        for (item, cell) in AUTO_ITEMS.into_iter().zip(self.cells) {
            let location = view.item(item).unwrap();
            assert_eq!(location.root(), ROOT);
            assert!(location.is_active());
            assert!(location.tabs().is_none());
            assert_eq!(view.node(cell).unwrap().item(), Some(item));
        }
        let mut callbacks = self.panes.body_calls.clone();
        callbacks.sort_unstable();
        assert_eq!(callbacks, AUTO_ITEMS);
        assert_eq!(
            self.panes.bodies.keys().copied().collect::<Vec<_>>(),
            AUTO_ITEMS
        );
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "the fixture dimensions never exceed three tracks"
    )]
    fn assert_partition(&self, rows: &[f32], columns: &[f32]) {
        let style = self.dockspace.style();
        let gap = style.splitter_thickness;
        // Five bare cells leave the default layout-actions band above the Grid.
        let mut content = self.screen_rect;
        content.min.y += style.tab_bar_height;
        let width = content.width() - gap * (columns.len() - 1) as f32;
        let height = content.height() - gap * (rows.len() - 1) as f32;
        for (index, item) in AUTO_ITEMS.into_iter().enumerate() {
            let row = index / columns.len();
            let column = index % columns.len();
            let origin = content.min
                + vec2(
                    width * columns[..column].iter().sum::<f32>() + gap * column as f32,
                    height * rows[..row].iter().sum::<f32>() + gap * row as f32,
                );
            let expected =
                Rect::from_min_size(origin, vec2(width * columns[column], height * rows[row]));
            let actual = self.panes.bodies[&item];
            for (actual, expected) in [
                (actual.left(), expected.left()),
                (actual.top(), expected.top()),
                (actual.right(), expected.right()),
                (actual.bottom(), expected.bottom()),
            ] {
                assert!(
                    (actual - expected).abs() < 0.01,
                    "{item:?}, {}x{}: expected {expected}, got {actual}",
                    rows.len(),
                    columns.len(),
                );
            }
        }
    }

    fn begin_column_resize(&mut self) -> (SplitterControl, Pos2) {
        let control = self
            .splitters
            .iter()
            .filter(|control| control.orientation == Orientation::Vertical)
            .min_by(|a, b| a.rect.left().total_cmp(&b.rect.left()))
            .copied()
            .expect("the current Auto map exposes its first column boundary");
        let point = egui::pos2(
            control.rect.center().x,
            control.rect.top() + control.rect.height() * 0.25,
        );
        self.run(vec![Event::PointerMoved(point)]);
        assert_eq!(
            self.context
                .viewport(|viewport| viewport.hits.drag.map(|hit| hit.id.accesskit_id())),
            Some(control.id),
            "the current column handle must win the real hit graph before pressing"
        );
        self.run(vec![button(point, true)]);
        assert!(self.dockspace.has_active_gesture());
        (control, point)
    }

    fn assert_no_persistent_shares(&self) {
        assert!(
            self.dockspace
                .view()
                .node(self.grid)
                .unwrap()
                .grid()
                .unwrap()
                .shares()
                .is_none()
        );
    }
}

#[test]
fn auto_grid_viewport_transpose_cancels_the_old_column_resize_and_accepts_a_fresh_one() {
    let mut harness = AutoGridHarness::new();
    let version = harness.dockspace.version();
    let (_, point) = harness.begin_column_resize();
    let old_release = point + vec2(16.0, 0.0);
    harness.run(vec![Event::PointerMoved(old_release)]);
    assert!(harness.dockspace.has_active_gesture());
    harness.assert_no_persistent_shares();
    harness.assert_partition(&[0.5, 0.5], &[148.0 / 396.0, 116.0 / 396.0, 1.0 / 3.0]);

    harness.screen_rect = Rect::from_min_size(Pos2::ZERO, vec2(204.0, 404.0));
    harness.run(Vec::new());
    assert!(
        !harness.dockspace.has_active_gesture(),
        "new measurements change the direct Auto driver from 2x3 to 3x2"
    );
    assert_eq!(harness.dockspace.version(), version);
    harness.assert_no_persistent_shares();
    harness.assert_partition(&[1.0 / 3.0; 3], &[0.5, 0.5]);

    harness.run(vec![button(old_release, false)]);
    harness.settle();
    assert!(!harness.dockspace.has_active_gesture());
    assert!(!harness.context.input(|input| input.pointer.primary_down()));
    assert_eq!(harness.dockspace.version(), version);
    harness.assert_no_persistent_shares();
    harness.assert_partition(&[1.0 / 3.0; 3], &[0.5, 0.5]);

    let (_, point) = harness.begin_column_resize();
    let fresh_release = point + vec2(16.0, 0.0);
    harness.run(vec![Event::PointerMoved(fresh_release)]);
    assert!(harness.dockspace.has_active_gesture());
    harness.run(vec![button(fresh_release, false)]);
    harness.settle();
    assert!(!harness.dockspace.has_active_gesture());
    assert_ne!(harness.dockspace.version(), version);
    let shares = harness
        .dockspace
        .view()
        .node(harness.grid)
        .unwrap()
        .grid()
        .unwrap()
        .shares()
        .expect("a fresh receiver commits shares for the new 3x2 map");
    assert_eq!(shares.rows().len(), 3);
    assert_eq!(shares.columns().len(), 2);
    for (actual, expected) in
        shares
            .rows()
            .chain(shares.columns())
            .zip([1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.58, 0.42])
    {
        assert!((actual - expected).abs() < 1e-12);
    }
    harness.assert_partition(&[1.0 / 3.0; 3], &[0.58, 0.42]);
}
