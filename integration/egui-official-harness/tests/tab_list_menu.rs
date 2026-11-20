use egui::accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use egui::{Context, Event, FullOutput, Key, Pos2, RawInput, Rect, Ui, WidgetText, vec2};
use egui_dockspace::{
    DockStyle, Dockspace, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
    SurfacePresentationResult, TabKey,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const SELECTED: ItemId = ItemId::new(36);
const LAST: ItemId = ItemId::new(40);

#[derive(Default)]
struct Panes {
    header_width: Option<f32>,
    rich_titles: bool,
}

impl Panes {
    fn label(&self, item: ItemId) -> String {
        if self.rich_titles {
            format!("Document {:02}", item.get())
        } else {
            format!(
                "A long document name with a unique suffix {:02}",
                item.get()
            )
        }
    }
}

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<WidgetText> {
        let label = self.label(item);
        Some(if self.rich_titles {
            egui::text::LayoutJob::simple_format(
                label,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(54.0),
                    line_height: Some(12.0),
                    color: egui::Color32::WHITE,
                    ..Default::default()
                },
            )
            .into()
        } else {
            label.into()
        })
    }

    fn tab_content_width(&self, _: TabKey, _: &Ui, title: &egui::Galley) -> f32 {
        self.header_width.unwrap_or(title.size().x)
    }

    fn ui(&mut self, _: ItemId, _: &mut Ui) -> PaneResponse {
        PaneResponse::default()
    }
}

struct Fixture {
    context: Context,
    dock: Dockspace,
    panes: Panes,
    tick: u32,
}

impl Fixture {
    fn new(panes: Panes) -> Self {
        let context = Context::default();
        context.enable_accesskit();
        context.all_styles_mut(|style| style.spacing.scroll = egui::style::ScrollStyle::solid());
        let tabs = DockspaceNode::central_tabs_with_selection(
            (1..=LAST.get()).map(ItemId::new),
            Some(SELECTED),
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), tabs),
        )])
        .unwrap();
        let dock = Dockspace::builder("tab-list-menu-regressions", layout)
            .style(DockStyle {
                tab_min_width: 160.0,
                tab_max_width: 160.0,
                ..DockStyle::default()
            })
            .build()
            .unwrap();
        Self {
            context,
            dock,
            panes,
            tick: 0,
        }
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        self.tick += 1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 200.0))),
            time: Some(f64::from(self.tick) * 0.1),
            events,
            ..Default::default()
        };
        let (presented, _) = self
            .dock
            .run_single_surface(&self.context, input, SURFACE, &mut self.panes)
            .unwrap();
        let (mut output, receipt) = presented.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self) -> FullOutput {
        self.run(Vec::new());
        self.run(Vec::new());
        self.run(Vec::new())
    }

    fn open_accessibly(&mut self) -> FullOutput {
        let output = self.settle();
        let (trigger, _) = node(&output, Role::Button, "List tabs");
        self.run(vec![accesskit(*trigger, Action::Click)]);
        self.settle()
    }
}

fn accesskit(target_node: NodeId, action: Action) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        target_node,
        target_tree: TreeId::ROOT,
        data: None,
    })
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn nodes(output: &FullOutput) -> &[(NodeId, Node)] {
    &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
}

fn node<'output>(output: &'output FullOutput, role: Role, label: &str) -> &'output (NodeId, Node) {
    nodes(output)
        .iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .unwrap_or_else(|| panic!("missing accessible {role:?}: {label}"))
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    reason = "The trigger is inside a 640×200 surface."
)]
fn pointer_open_keeps_menu_focus_for_the_next_key_packet() {
    let mut fixture = Fixture::new(Panes::default());
    let output = fixture.settle();
    let bounds = node(&output, Role::Button, "List tabs").1.bounds().unwrap();
    let point = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    fixture.run(vec![Event::PointerMoved(point)]);
    for pressed in [true, false] {
        fixture.run(vec![Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    fixture.run(vec![key(Key::ArrowUp), key(Key::Enter)]);
    let output = fixture.settle();
    let previous = ItemId::new(SELECTED.get() - 1);
    assert!(fixture.dock.view().item(previous).unwrap().is_active());
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Menu)
    );
}

#[test]
fn offscreen_menu_rows_allow_reveal_and_focus_without_spatial_activation() {
    let mut fixture = Fixture::new(Panes::default());
    let output = fixture.open_accessibly();
    let first = fixture.panes.label(ItemId::new(1));
    let (first_id, row) = node(&output, Role::MenuItem, &first);
    let first_id = *first_id;
    assert!(row.bounds().is_none());
    assert!(!row.supports_action(Action::Click));
    assert!(row.supports_action(Action::Focus));
    assert!(row.supports_action(Action::ScrollIntoView));
    let menu = node(&output, Role::Menu, "Open tabs").1.bounds().unwrap();
    for (_, row) in nodes(&output)
        .iter()
        .filter(|(_, node)| node.role() == Role::MenuItem)
    {
        if let Some(bounds) = row.bounds() {
            assert!(bounds.y0 >= menu.y0 && bounds.y1 <= menu.y1);
        }
    }

    fixture.run(vec![accesskit(first_id, Action::Click)]);
    let output = fixture.settle();
    node(&output, Role::Menu, "Open tabs");
    assert!(fixture.dock.view().item(SELECTED).unwrap().is_active());

    fixture.run(vec![accesskit(first_id, Action::ScrollIntoView)]);
    let output = fixture.settle();
    let (revealed_id, row) = node(&output, Role::MenuItem, &first);
    assert_eq!(*revealed_id, first_id);
    assert!(row.bounds().is_some());
    assert!(row.supports_action(Action::Click));

    let last = fixture.panes.label(LAST);
    let (last_id, row) = node(&output, Role::MenuItem, &last);
    assert!(row.bounds().is_none());
    fixture.run(vec![accesskit(*last_id, Action::Focus)]);
    let output = fixture.settle();
    assert!(node(&output, Role::MenuItem, &last).1.bounds().is_some());
    assert_eq!(
        fixture
            .context
            .memory(egui::Memory::focused)
            .map(|id| id.value()),
        Some(last_id.0)
    );
    assert!(fixture.dock.view().item(SELECTED).unwrap().is_active());
}

#[test]
fn compact_headers_leave_room_for_the_full_menu_title() {
    let mut reference = Fixture::new(Panes::default());
    let output = reference.open_accessibly();
    let label = reference.panes.label(SELECTED);
    let normal = node(&output, Role::MenuItem, &label)
        .1
        .bounds()
        .unwrap()
        .width();
    assert!(normal > 200.0, "the fixture must have a wide menu title");

    for width in [0.0, 16.0] {
        let mut fixture = Fixture::new(Panes {
            header_width: Some(width),
            ..Panes::default()
        });
        let output = fixture.open_accessibly();
        let width = node(&output, Role::MenuItem, &label)
            .1
            .bounds()
            .unwrap()
            .width();
        assert_eq!(width, normal);
    }
}

#[test]
fn rich_menu_titles_are_drawn_only_within_the_visible_row() {
    let mut fixture = Fixture::new(Panes {
        rich_titles: true,
        ..Panes::default()
    });
    let output = fixture.open_accessibly();
    let menu = node(&output, Role::Menu, "Open tabs").1.bounds().unwrap();
    let mut painted = 0;
    for shape in &output.shapes {
        let egui::Shape::Text(text) = &shape.shape else {
            continue;
        };
        if f64::from(shape.clip_rect.min.y) < menu.y0 {
            continue;
        }
        let (_, row) = node(&output, Role::MenuItem, text.galley.text());
        let visible = row.bounds().expect("an offscreen title must not be drawn");
        assert!(f64::from(shape.clip_rect.min.y) >= visible.y0);
        assert!(f64::from(shape.clip_rect.max.y) <= visible.y1);
        painted += 1;
    }
    assert!(
        painted > 1,
        "the output must contain actual menu title shapes"
    );
}
