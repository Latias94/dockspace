//! Nested chrome keeps pointer ownership and Auto Grid sizing in its own scope.

use std::collections::BTreeMap;

use super::*;
use crate::{DockspaceAxis, DockspaceGridShape, PaneSizing, Splitter, Tab, TabKey, TabVisuals};

const ROOT: RootId = RootId::new(1);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);

#[derive(Default)]
struct GeometryPanes {
    panes: BTreeMap<ItemId, Rect>,
    tabs: BTreeMap<TabKey, (egui::Id, Rect)>,
    splitters: Vec<Splitter>,
    sizing: PaneSizing,
}

impl PaneView for GeometryPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(format!("Pane {}", item.get()).into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        self.panes.insert(item, ui.available_rect_before_wrap());
        PaneResponse::default()
    }

    fn sizing(&self, _: ItemId) -> PaneSizing {
        self.sizing
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        self.tabs
            .insert(tab.key, (tab.response.id, tab.visible_rect));
        tab.paint(ui)
    }

    fn splitter_ui(&mut self, painter: &egui::Painter, splitter: &Splitter) {
        self.splitters.push(*splitter);
        splitter.paint(painter);
    }
}

struct GeometryHarness {
    context: Context,
    dockspace: Dockspace,
    panes: GeometryPanes,
    bounds: Rect,
    frame: u32,
}

impl GeometryHarness {
    fn new(content: DockspaceNode, size: egui::Vec2, sizing: PaneSizing) -> Self {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap();
        Self::with_layout(layout, size, sizing)
    }

    fn with_layout(layout: DockspaceLayout, size: egui::Vec2, sizing: PaneSizing) -> Self {
        let context = Context::default();
        context.all_styles_mut(|style| style.animation_time = 0.0);
        Self {
            context,
            dockspace: Dockspace::builder("nested-geometry", layout)
                .style(DockStyle {
                    tab_bar_height: 24.0,
                    splitter_thickness: 4.0,
                    splitter_hit_extent: 32.0,
                    minimum_pane_size: vec2(1.0, 1.0),
                    ..Default::default()
                })
                .build()
                .unwrap(),
            panes: GeometryPanes {
                sizing,
                ..Default::default()
            },
            bounds: Rect::from_min_size(egui::pos2(40.0, 60.0), size),
            frame: 0,
        }
    }

    fn run(&mut self, events: Vec<Event>) {
        self.frame += 1;
        self.panes.panes.clear();
        self.panes.tabs.clear();
        self.panes.splitters.clear();
        let (output, _) = self
            .dockspace
            .run_single_surface(
                &self.context,
                RawInput {
                    screen_rect: Some(self.bounds),
                    time: Some(f64::from(self.frame) * 0.1),
                    events,
                    ..Default::default()
                },
                SURFACE,
                &mut self.panes,
            )
            .unwrap();
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }
}

#[test]
fn clipping_a_translated_contained_grid_preserves_its_layout_and_junction_hit() {
    use std::num::NonZeroUsize;

    use crate::{DockspaceContainedLayout, FloatingPresentationId, SplitterKind};

    let grid = DockspaceNode::grid(
        DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(2).unwrap(),
            columns: NonZeroUsize::new(2).unwrap(),
        },
        (1..=4).map(|item| DockspaceNode::pane(ItemId::new(item))),
    )
    .unwrap();
    let mut baseline = None;
    for left in [0.0, -80.0] {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([ItemId::new(50)])),
        )
        .with_contained(DockspaceContainedLayout::new(
            FloatingPresentationId::new(1),
            DockspaceRootLayout::new(RootId::new(2), grid.clone()),
            crate::LogicalRect::new(left, 100.0, 404.0, 304.0).unwrap(),
        ))])
        .unwrap();
        let size = vec2(904.0, 524.0);
        let mut harness = GeometryHarness::with_layout(layout, size, PaneSizing::default());
        harness.bounds = Rect::from_min_size(Pos2::ZERO, size);
        harness.settle();
        let junction = *harness
            .panes
            .splitters
            .iter()
            .find(|splitter| splitter.kind == SplitterKind::Junction)
            .unwrap();
        let right_cell = harness.panes.panes[&B];
        let (center, cell) = *baseline.get_or_insert((junction.rect.center(), right_cell));
        let translation = vec2(left as f32, 0.0);
        assert_eq!(junction.rect.center(), center + translation);
        assert_eq!(right_cell, cell.translate(translation));
        harness.run(vec![Event::PointerMoved(center + translation)]);
        assert_eq!(
            harness
                .context
                .viewport(|viewport| viewport.hits.drag.map(|hit| hit.id)),
            Some(junction.id)
        );
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn nested_splitter_hit_preserves_parent_tab_click_and_drag_ownership() {
    for drag in [false, true] {
        let tools = DockspaceNode::split(
            DockspaceAxis::Vertical,
            [
                (DockspaceNode::pane(A), 1.0),
                (DockspaceNode::pane(B), 49.0),
            ],
        )
        .unwrap()
        .named("Tools");
        let mut harness = GeometryHarness::new(
            DockspaceNode::composed_tabs([tools, DockspaceNode::pane(C)]),
            vec2(500.0, 228.0),
            PaneSizing::default(),
        );
        harness.settle();
        let divider = harness.panes.splitters[0];
        let (tab_id, tab_bounds) = harness.panes.tabs[&TabKey::for_item(C)];
        assert!(divider.hit_rect.top() >= tab_bounds.bottom());
        let point = egui::pos2(174.0, tab_bounds.bottom() - 3.0);
        assert!(tab_bounds.contains(point));
        harness.run(vec![Event::PointerMoved(point)]);
        harness.context.viewport(|viewport| {
            assert_eq!(viewport.hits.click.map(|hit| hit.id), Some(tab_id));
            assert_eq!(viewport.hits.drag.map(|hit| hit.id), Some(tab_id));
        });
        harness.run(vec![button(point, true)]);
        let end = if drag { point + vec2(0.0, 40.0) } else { point };
        if drag {
            harness.run(vec![Event::PointerMoved(end)]);
            harness.run(vec![key(Key::Escape)]);
        }
        harness.run(vec![button(end, false)]);
        harness.settle();
        if !drag {
            assert!(harness.dockspace.view().item(C).unwrap().is_active());
        }
        let _ = harness.dockspace.select_item_current(A).unwrap();
        harness.settle();
        assert_eq!(harness.panes.splitters[0].rect, divider.rect);

        let start = divider.rect.center();
        harness.run(vec![Event::PointerMoved(start)]);
        harness.run(vec![button(start, true)]);
        let end = start + vec2(0.0, 40.0);
        harness.run(vec![Event::PointerMoved(end)]);
        harness.run(vec![button(end, false)]);
        harness.settle();
        assert!(harness.panes.splitters[0].rect.top() > divider.rect.top() + 20.0);
    }
}

#[test]
fn auto_grid_shape_uses_offered_space_before_cell_maxima_through_layout_tabs() {
    for horizontal in [true, false] {
        for maximum in [None, Some(150.0), Some(100.0)] {
            for depth in 0..=2 {
                let mut content = DockspaceNode::grid(
                    DockspaceGridShape::Auto,
                    (1..=4).map(|item| DockspaceNode::pane(ItemId::new(item))),
                )
                .unwrap();
                for _ in 0..depth {
                    content = DockspaceNode::composed_tabs([content]);
                }
                let sizing = maximum.map_or_else(PaneSizing::default, |maximum| {
                    let builder = PaneSizing::builder();
                    if horizontal {
                        builder.max_width(maximum)
                    } else {
                        builder.max_height(maximum)
                    }
                    .build()
                    .unwrap()
                });
                let header = 24.0 * depth.max(1) as f32;
                let size = if horizontal {
                    vec2(604.0, 148.0 + header)
                } else {
                    vec2(148.0, 604.0 + header)
                };
                let mut harness = GeometryHarness::new(content, size, sizing);
                harness.settle();
                let extent = maximum.unwrap_or(148.0).min(148.0) as f32;
                assert_eq!(harness.panes.panes.len(), 4);
                for index in 0..4 {
                    let pane = harness.panes.panes[&ItemId::new(index + 1)];
                    let expected = if horizontal {
                        Rect::from_min_size(
                            egui::pos2(40.0 + index as f32 * (extent + 4.0), 60.0 + header),
                            vec2(extent, 148.0),
                        )
                    } else {
                        Rect::from_min_size(
                            egui::pos2(40.0, 60.0 + header + index as f32 * (extent + 4.0)),
                            vec2(148.0, extent),
                        )
                    };
                    assert_eq!(
                        pane, expected,
                        "horizontal={horizontal}, maximum={maximum:?}, depth={depth}"
                    );
                }
            }
        }
    }
}
