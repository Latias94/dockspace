//! Real pointer previews and commits for the shared_grid example's mixed cells.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use super::*;
use crate::{
    DockEdge, DockspaceAxis, DockspaceDropAction, DockspaceDropDirection, DockspaceDropEligibility,
    DockspaceDropTargetScope, DockspaceGridShape, DockspaceGuideScope, DockspaceTabView, DropGuide,
    DropPreview, NodeKey, Tab, TabKey, TabVisuals,
};

const ROOT: RootId = RootId::new(1);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);
const E: ItemId = ItemId::new(5);
const F: ItemId = ItemId::new(6);
const G: ItemId = ItemId::new(7);
const ITEMS: [ItemId; 7] = [A, B, C, D, E, F, G];

#[derive(Default)]
struct GridPanes {
    bodies: BTreeMap<ItemId, Rect>,
    tabs: BTreeMap<TabKey, Rect>,
    guides: Vec<DropGuide>,
    preview: Option<DropPreview>,
    text: String,
}

impl PaneView for GridPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(
            match item {
                A => "Bare A",
                B => "Document B",
                C => "Document C",
                D => "Bare D",
                E => "Document E",
                F => "Tool F",
                G => "Tool G",
                _ => panic!("layout tabs never use an application item as their title"),
            }
            .into(),
        )
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        self.bodies.insert(item, ui.available_rect_before_wrap());
        let response = ui.add(
            egui::TextEdit::multiline(&mut self.text)
                .id(egui::Id::unique(("shared-grid-preview-editor", item)))
                .desired_width(f32::INFINITY),
        );
        PaneResponse::from(&response)
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        self.tabs.insert(tab.key, tab.visible_rect);
        tab.paint(ui)
    }

    fn drop_guide_ui(&mut self, ui: &Ui, guide: &DropGuide) {
        self.guides.push(*guide);
        guide.paint(ui);
    }

    fn drop_preview_ui(&mut self, ui: &Ui, preview: &DropPreview) {
        self.preview = Some(preview.clone());
        preview.paint(ui);
    }
}

struct GridHarness {
    context: Context,
    dockspace: Dockspace,
    panes: GridPanes,
    frame: u32,
    grid: NodeKey,
    cells: [NodeKey; 4],
    tools: TabKey,
    tools_node: NodeKey,
    tool_children: [NodeKey; 2],
}

impl GridHarness {
    fn new() -> Self {
        // Keep the example's real bare panes, named containers and direct tab scopes.
        let tools = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::pane(F), DockspaceNode::pane(G)],
        )
        .unwrap()
        .named("Tools");
        let grid = DockspaceNode::grid(
            DockspaceGridShape::Fixed {
                rows: NonZeroUsize::new(2).unwrap(),
                columns: NonZeroUsize::new(2).unwrap(),
            },
            [
                DockspaceNode::pane(A),
                DockspaceNode::tabs([B, C]).named("Documents"),
                DockspaceNode::pane(D),
                DockspaceNode::composed_tabs([DockspaceNode::pane(E), tools]),
            ],
        )
        .unwrap()
        .named("Workspace");
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, grid),
        )])
        .unwrap();
        let dockspace = Dockspace::builder("shared-grid-pointer-preview", layout)
            .build()
            .unwrap();
        let root = dockspace.view().root(ROOT).unwrap().content().unwrap();
        let grid = root.key();
        let children = root.grid().unwrap().children().collect::<Vec<_>>();
        let cells = std::array::from_fn(|index| children[index].key());
        let outer = children[3].tabs().unwrap();
        let tools = outer.tabs()[1];
        let Some(DockspaceTabView::Layout(tools_view)) = outer.child(tools) else {
            panic!("Tools is a real layout tab, independent of F and G");
        };
        let tools_node = tools_view.key();
        let tool_children = tools_view
            .split()
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
            grid,
            cells,
            tools,
            tools_node,
            tool_children,
        };
        harness.settle();
        harness.assert_active(&[A, B, D, E]);
        harness
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        self.frame += 1;
        self.panes.bodies.clear();
        self.panes.tabs.clear();
        self.panes.guides.clear();
        self.panes.preview = None;
        let (output, _) = self
            .dockspace
            .run_single_surface(
                &self.context,
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
                    time: Some(f64::from(self.frame) * 0.1),
                    events,
                    ..Default::default()
                },
                SURFACE,
                &mut self.panes,
            )
            .unwrap_or_else(|error| panic!("shared_grid pointer frame {}: {error}", self.frame));
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self) {
        self.run(Vec::new());
        self.run(Vec::new());
    }

    fn begin_drag(&mut self, key: TabKey) {
        let source = self.panes.tabs[&key].center();
        self.run(vec![Event::PointerMoved(source)]);
        self.run(vec![button(source, true)]);
        self.run(vec![Event::PointerMoved(source + vec2(20.0, 8.0))]);
        assert!(self.dockspace.has_active_gesture(), "dragging {key:?}");
    }

    fn hover_guide(&mut self, item: ItemId, direction: DockspaceDropDirection) -> Pos2 {
        let body = self.panes.bodies[&item].center();
        self.run(vec![Event::PointerMoved(body)]);
        self.run(vec![Event::PointerMoved(body)]);
        let guide = self
            .panes
            .guides
            .iter()
            .filter(|guide| {
                guide.root == Some(ROOT)
                    && guide.scope == DockspaceGuideScope::Inner
                    && guide.direction == direction
            })
            .min_by(|left, right| {
                left.rect
                    .center()
                    .distance_sq(body)
                    .total_cmp(&right.rect.center().distance_sq(body))
            })
            .unwrap_or_else(|| panic!("{item:?} exposes its own {direction:?} guide"));
        assert_eq!(guide.eligibility, DockspaceDropEligibility::Eligible);
        let point = guide.rect.center();
        self.run(vec![Event::PointerMoved(point)]);
        point
    }

    fn hover_gap_before(&mut self, key: TabKey) -> Pos2 {
        let tab = self.panes.tabs[&key];
        let point = egui::pos2(tab.left() + 1.0, tab.center().y);
        self.run(vec![Event::PointerMoved(point)]);
        self.run(vec![Event::PointerMoved(point)]);
        point
    }

    fn assert_preview(
        &self,
        point: Pos2,
        action: DockspaceDropAction,
        future_tabs: &[TabKey],
        selected: Option<TabKey>,
        payload: &[TabKey],
    ) {
        let preview = self
            .panes
            .preview
            .as_ref()
            .expect("an eligible real target paints its future result before release");
        assert_eq!(preview.target.surface(), SURFACE);
        assert_eq!(preview.target.root(), Some(ROOT));
        assert_eq!(preview.target.scope(), DockspaceDropTargetScope::Pane);
        assert_eq!(preview.target.action(), action);
        assert!(preview.target_rect.contains(point));
        assert!(preview.rect.is_positive() && preview.rect.is_finite());
        assert_eq!(
            preview.tabs.iter().map(|tab| tab.key).collect::<Vec<_>>(),
            future_tabs
        );
        assert_eq!(
            preview
                .tabs
                .iter()
                .filter(|tab| tab.selected)
                .map(|tab| tab.key)
                .collect::<Vec<_>>(),
            selected.into_iter().collect::<Vec<_>>()
        );
        assert_eq!(
            preview
                .tabs
                .iter()
                .filter(|tab| tab.is_payload)
                .map(|tab| tab.key)
                .collect::<Vec<_>>(),
            payload
        );
        for tab in &preview.tabs {
            if tab.key == self.tools {
                assert_eq!(tab.title.job.text, "Tools");
            }
        }
    }

    fn release(&mut self, point: Pos2) {
        self.run(vec![button(point, false)]);
        self.settle();
        assert!(!self.dockspace.has_active_gesture());
        assert!(self.panes.preview.is_none());
    }

    fn click_tab(&mut self, key: TabKey) {
        let point = self.panes.tabs[&key].center();
        self.run(vec![Event::PointerMoved(point)]);
        self.run(vec![button(point, true)]);
        self.run(vec![button(point, false)]);
        self.settle();
        assert!(!self.dockspace.has_active_gesture());
    }

    fn assert_grid(&self, expected_cells: [NodeKey; 4]) {
        let view = self.dockspace.view();
        assert_eq!(view.surfaces().count(), 1);
        assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 0);
        let root = view.root(ROOT).unwrap().content().unwrap();
        assert_eq!(root.key(), self.grid);
        assert_eq!(root.layout_title(), Some("Workspace"));
        let grid = root.grid().unwrap();
        assert_eq!(
            grid.shape(),
            DockspaceGridShape::Fixed {
                rows: NonZeroUsize::new(2).unwrap(),
                columns: NonZeroUsize::new(2).unwrap(),
            }
        );
        assert!(grid.shares().is_none());
        assert_eq!(
            grid.children().map(|child| child.key()).collect::<Vec<_>>(),
            expected_cells
        );
        let mut owned = root.items().collect::<Vec<_>>();
        owned.sort_unstable();
        assert_eq!(owned, ITEMS, "every real pane stays owned exactly once");
        for item in ITEMS {
            let location = view.item(item).unwrap();
            assert_eq!(location.root(), ROOT);
            assert_eq!(location.surface(), SURFACE);
            assert!(location.contained().is_none());
        }
        assert_eq!(view.node(self.cells[2]).unwrap().item(), Some(D));
        let documents = view.node(self.cells[1]).unwrap();
        assert_eq!(documents.layout_title(), Some("Documents"));
        assert!(documents.tabs().is_some());
        let tools = view.node(self.tools_node).unwrap();
        assert_eq!(tools.layout_title(), Some("Tools"));
        let split = tools.split().unwrap();
        assert_eq!(split.axis(), DockspaceAxis::Horizontal);
        assert_eq!(split.weights().collect::<Vec<_>>(), [0.5, 0.5]);
        let children = split.children().collect::<Vec<_>>();
        assert_eq!(
            children.iter().map(|child| child.key()).collect::<Vec<_>>(),
            self.tool_children
        );
        assert_eq!(children[0].item(), Some(F));
        assert_eq!(children[1].item(), Some(G));
        assert!(view.item(F).unwrap().tabs().is_none());
        assert!(view.item(G).unwrap().tabs().is_none());
    }

    fn assert_tabs(&self, node: NodeKey, tabs: &[TabKey], selected: TabKey) {
        let tabs_view = self.dockspace.view().node(node).unwrap().tabs().unwrap();
        assert_eq!(tabs_view.tabs(), tabs);
        assert_eq!(tabs_view.selected(), Some(selected));
    }

    fn assert_active(&self, expected: &[ItemId]) {
        assert_eq!(
            ITEMS
                .into_iter()
                .filter(|item| self.dockspace.view().item(*item).unwrap().is_active())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            self.panes.bodies.keys().copied().collect::<Vec<_>>(),
            expected,
            "only the selected real content receives application UI"
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
fn shared_grid_document_tab_previews_and_commits_into_a_bare_pane_center() {
    let mut harness = GridHarness::new();
    harness.begin_drag(C.into());
    let point = harness.hover_guide(A, DockspaceDropDirection::Center);
    harness.assert_preview(point, DockspaceDropAction::Merge, &[], None, &[]);
    let target = harness.panes.preview.as_ref().unwrap().target;
    assert_eq!(target.selected_tab(), None);
    assert_eq!(target.selected_item(), Some(A));
    assert!(
        harness
            .panes
            .preview
            .as_ref()
            .unwrap()
            .tab_bar_rect
            .is_none()
    );
    assert_eq!(
        harness
            .dockspace
            .view()
            .node(harness.cells[0])
            .unwrap()
            .item(),
        Some(A)
    );
    harness.release(point);
    harness.assert_grid(harness.cells);
    harness.assert_tabs(harness.cells[0], &[A.into(), C.into()], C.into());
    harness.assert_tabs(harness.cells[1], &[B.into()], B.into());
    harness.assert_tabs(harness.cells[3], &[E.into(), harness.tools], E.into());
    harness.assert_active(&[B, C, D, E]);
    harness.click_tab(A.into());
    harness.assert_active(&[A, B, D, E]);
    harness.click_tab(C.into());
    harness.assert_active(&[B, C, D, E]);
    harness.assert_grid(harness.cells);
}

#[test]
fn shared_grid_layout_tab_previews_and_commits_its_subtree_into_a_bare_pane_center() {
    let mut harness = GridHarness::new();
    harness.begin_drag(harness.tools);
    let point = harness.hover_guide(A, DockspaceDropDirection::Center);
    harness.assert_preview(point, DockspaceDropAction::Merge, &[], None, &[]);
    let target = harness.panes.preview.as_ref().unwrap().target;
    assert_eq!(target.selected_tab(), None);
    assert_eq!(target.selected_item(), Some(A));
    // A bare destination has no measured strip; preview its real result without invented tabs.
    assert!(
        harness
            .panes
            .preview
            .as_ref()
            .unwrap()
            .tab_bar_rect
            .is_none()
    );
    harness.release(point);
    harness.assert_grid(harness.cells);
    harness.assert_tabs(harness.cells[0], &[A.into(), harness.tools], harness.tools);
    harness.assert_tabs(harness.cells[1], &[B.into(), C.into()], B.into());
    harness.assert_tabs(harness.cells[3], &[E.into()], E.into());
    harness.assert_active(&[B, D, E, F, G]);
    harness.click_tab(A.into());
    harness.assert_active(&[A, B, D, E]);
    harness.click_tab(harness.tools);
    harness.assert_active(&[B, D, E, F, G]);
    harness.assert_grid(harness.cells);
}

#[test]
fn shared_grid_layout_tab_reorders_at_a_same_parent_gap_without_unpacking_its_panes() {
    let mut harness = GridHarness::new();
    harness.begin_drag(harness.tools);
    let point = harness.hover_gap_before(E.into());
    harness.assert_preview(
        point,
        DockspaceDropAction::InsertTab {
            index: 0,
            previous: None,
            next: Some(E.into()),
        },
        &[harness.tools, E.into()],
        Some(harness.tools),
        &[harness.tools],
    );
    harness.release(point);
    harness.assert_grid(harness.cells);
    harness.assert_tabs(harness.cells[3], &[harness.tools, E.into()], harness.tools);
    harness.assert_active(&[A, B, D, F, G]);
    harness.click_tab(E.into());
    harness.assert_active(&[A, B, D, E]);
    harness.click_tab(harness.tools);
    harness.assert_active(&[A, B, D, F, G]);
}

#[test]
fn shared_grid_layout_tab_moves_to_an_exact_gap_in_another_cell() {
    let mut harness = GridHarness::new();
    harness.begin_drag(harness.tools);
    let point = harness.hover_gap_before(C.into());
    harness.assert_preview(
        point,
        DockspaceDropAction::InsertTab {
            index: 1,
            previous: Some(B.into()),
            next: Some(C.into()),
        },
        &[B.into(), harness.tools, C.into()],
        Some(harness.tools),
        &[harness.tools],
    );
    harness.release(point);
    harness.assert_grid(harness.cells);
    harness.assert_tabs(
        harness.cells[1],
        &[B.into(), harness.tools, C.into()],
        harness.tools,
    );
    harness.assert_tabs(harness.cells[3], &[E.into()], E.into());
    harness.assert_active(&[A, D, E, F, G]);
    harness.click_tab(C.into());
    harness.assert_active(&[A, C, D, E]);
    harness.click_tab(harness.tools);
    harness.assert_active(&[A, D, E, F, G]);
}

#[test]
fn shared_grid_document_tab_splits_only_the_target_cell_at_its_inner_edge() {
    let mut harness = GridHarness::new();
    harness.begin_drag(C.into());
    let point = harness.hover_guide(A, DockspaceDropDirection::Right);
    harness.assert_preview(
        point,
        DockspaceDropAction::Split {
            edge: DockEdge::Right,
        },
        &[],
        None,
        &[],
    );
    let target = harness.panes.preview.as_ref().unwrap().target;
    assert_eq!(target.selected_tab(), None);
    assert_eq!(target.selected_item(), Some(A));
    harness.release(point);
    let root = harness
        .dockspace
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap();
    let cell = root.grid().unwrap().children().next().unwrap();
    assert_ne!(cell.key(), harness.cells[0]);
    let split = cell.split().expect("the inner edge wraps only bare A");
    assert_eq!(split.axis(), DockspaceAxis::Horizontal);
    let children = split.children().collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].key(), harness.cells[0]);
    assert_eq!(children[0].item(), Some(A));
    let payload = children[1].key();
    harness.assert_tabs(payload, &[C.into()], C.into());
    let mut expected_cells = harness.cells;
    expected_cells[0] = cell.key();
    harness.assert_grid(expected_cells);
    harness.assert_tabs(harness.cells[1], &[B.into()], B.into());
    harness.assert_active(&[A, B, C, D, E]);
    harness.begin_drag(C.into());
    harness.run(vec![key(Key::Escape)]);
    harness.settle();
    assert!(!harness.dockspace.has_active_gesture());
    harness.assert_tabs(payload, &[C.into()], C.into());
    harness.assert_grid(expected_cells);
    harness.assert_active(&[A, B, C, D, E]);
}
