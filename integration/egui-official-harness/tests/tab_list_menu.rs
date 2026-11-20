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

#[derive(Clone, Copy, Default)]
enum Titles {
    #[default]
    Plain,
    BakedElision,
    CenterAligned,
    RightAligned,
    TightLines,
    Blank,
}

#[derive(Default)]
struct Panes {
    context: Context,
    header_width: Option<f32>,
    titles: Titles,
    title_prefix: Option<&'static str>,
}

impl Panes {
    fn label(&self, item: ItemId) -> String {
        match self.titles {
            Titles::TightLines => format!("Ágyp {:02}", item.get()),
            Titles::Blank => String::new(),
            _ => format!(
                "{} document name with a unique suffix {:02}",
                self.title_prefix.unwrap_or("A long"),
                item.get(),
            ),
        }
    }
}

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<WidgetText> {
        let label = self.label(item);
        let alignment = match self.titles {
            Titles::Plain | Titles::Blank => return Some(label.into()),
            Titles::CenterAligned => egui::Align::Center,
            Titles::RightAligned => egui::Align::RIGHT,
            Titles::BakedElision | Titles::TightLines => egui::Align::LEFT,
        };
        let tight = matches!(self.titles, Titles::TightLines);
        let mut job = egui::text::LayoutJob::simple_format(
            label,
            egui::TextFormat {
                font_id: egui::FontId::proportional(if tight { 54.0 } else { 14.0 }),
                line_height: tight.then_some(12.0),
                color: egui::Color32::WHITE,
                ..Default::default()
            },
        );
        job.halign = alignment;
        Some(if matches!(self.titles, Titles::BakedElision) {
            job.wrap = egui::text::TextWrapping::truncate_at_width(48.0);
            self.context.fonts_mut(|fonts| fonts.layout_job(job)).into()
        } else {
            job.into()
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
    fn new(mut panes: Panes) -> Self {
        let context = Context::default();
        context.enable_accesskit();
        context.all_styles_mut(|style| style.spacing.scroll = egui::style::ScrollStyle::solid());
        panes.context = context.clone();
        Self {
            context,
            dock: make_dock(),
            panes,
            tick: 0,
        }
    }

    fn input(&mut self, events: Vec<Event>, height: f32) -> RawInput {
        self.tick += 1;
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, height))),
            time: Some(f64::from(self.tick) * 0.1),
            events,
            ..Default::default()
        }
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        let input = self.input(events, 200.0);
        let (presented, _) = self
            .dock
            .run_single_surface(&self.context, input, SURFACE, &mut self.panes)
            .unwrap();
        let (mut output, receipt) = presented.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn run_embedded(&mut self, events: Vec<Event>) -> FullOutput {
        let input = self.input(events, 540.0);
        let (mut output, receipt) = egui_dockspace::run_ui(&self.context, input, |ui| {
            egui::Panel::top("embedded-dock")
                .exact_size(96.0)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    self.dock
                        .show_single_surface(SURFACE, ui, &mut self.panes)
                        .unwrap();
                });
        })
        .into_parts();
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
        if nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Menu)
        {
            return output;
        }
        let (trigger, _) = node(&output, Role::Button, "List tabs");
        self.run(vec![accesskit(*trigger, Action::Click)]);
        self.settle()
    }
}

fn layout() -> DockspaceLayout {
    let tabs = DockspaceNode::central_tabs_with_selection(
        (1..=LAST.get()).map(ItemId::new),
        Some(SELECTED),
    )
    .unwrap();
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), tabs),
    )])
    .unwrap()
}

fn make_dock() -> Dockspace {
    Dockspace::builder("tab-list-menu-regressions", layout())
        .style(DockStyle {
            tab_min_width: 160.0,
            tab_max_width: 160.0,
            ..DockStyle::default()
        })
        .build()
        .unwrap()
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

#[allow(
    clippy::cast_possible_truncation,
    reason = "The trigger is inside a 640×200 surface."
)]
fn trigger_center(output: &FullOutput) -> Pos2 {
    let bounds = node(output, Role::Button, "List tabs").1.bounds().unwrap();
    egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    )
}

fn pointer_button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn pointer_open_keeps_menu_focus_for_the_next_key_packet() {
    let mut fixture = Fixture::new(Panes::default());
    let point = trigger_center(&fixture.settle());
    fixture.run(vec![Event::PointerMoved(point)]);
    for pressed in [true, false] {
        fixture.run(vec![pointer_button(point, pressed)]);
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
fn menu_keys_follow_the_opening_release_in_batched_and_split_input() {
    for keys_before_release in [true, false] {
        for batched in [false, true] {
            let mut fixture = Fixture::new(Panes::default());
            let point = trigger_center(&fixture.settle());
            fixture.run(vec![Event::PointerMoved(point)]);
            fixture.run(vec![pointer_button(point, true)]);
            let release = vec![pointer_button(point, false)];
            let keys = vec![key(Key::End), key(Key::Enter)];
            let packets = if keys_before_release {
                [keys, release]
            } else {
                [release, keys]
            };
            if batched {
                fixture.run(packets.into_iter().flatten().collect());
            } else {
                for events in packets {
                    fixture.run(events);
                }
            }
            let output = fixture.settle();
            let expected = if keys_before_release { SELECTED } else { LAST };
            assert!(fixture.dock.view().item(expected).unwrap().is_active());
            assert_eq!(
                nodes(&output)
                    .iter()
                    .any(|(_, node)| node.role() == Role::Menu),
                keys_before_release,
            );
        }
    }
}

#[test]
fn stale_menu_accesskit_rows_cannot_activate_a_replacement_layout() {
    for replacement in ["epoch", "roster", "session"] {
        let mut fixture = Fixture::new(Panes::default());
        let output = fixture.open_accessibly();
        let target = ItemId::new(SELECTED.get() - 1);
        let (stale, row) = node(&output, Role::MenuItem, &fixture.panes.label(target));
        assert!(row.supports_action(Action::Click));
        match replacement {
            "epoch" => {
                fixture.dock.reset_layout(layout()).unwrap();
            }
            "roster" => {
                let _ = fixture
                    .dock
                    .set_item_visibility_current(ItemId::new(2), false)
                    .unwrap();
            }
            "session" => fixture.dock = make_dock(),
            _ => unreachable!(),
        }
        fixture.panes.title_prefix = Some("A different");
        let replacement = fixture.open_accessibly();
        let (fresh, row) = node(&replacement, Role::MenuItem, &fixture.panes.label(target));
        assert!(row.supports_action(Action::Click));
        fixture.run(vec![accesskit(*stale, Action::Click)]);
        let after = fixture.settle();
        node(&after, Role::Menu, "Open tabs");
        assert!(fixture.dock.view().item(SELECTED).unwrap().is_active());
        assert_ne!(stale, fresh);

        fixture.run(vec![accesskit(*fresh, Action::Click)]);
        fixture.settle();
        assert!(fixture.dock.view().item(target).unwrap().is_active());
    }
}

#[test]
fn embedded_menu_reveals_and_activates_its_last_row_inside_the_parent_clip() {
    let mut fixture = Fixture::new(Panes::default());
    let output = fixture.run_embedded(Vec::new());
    let (trigger, _) = node(&output, Role::Button, "List tabs");
    fixture.run_embedded(vec![accesskit(*trigger, Action::Click)]);
    let output = fixture.run_embedded(Vec::new());
    let label = fixture.panes.label(LAST);
    let (target, _) = node(&output, Role::MenuItem, &label);
    fixture.run_embedded(vec![accesskit(*target, Action::ScrollIntoView)]);
    let output = fixture.run_embedded(Vec::new());
    let (_, row) = node(&output, Role::MenuItem, &label);
    assert!(row.bounds().unwrap().y1 <= 96.0);
    assert!(row.supports_action(Action::Click));
    fixture.run_embedded(vec![accesskit(*target, Action::Click)]);
    fixture.run_embedded(Vec::new());
    assert!(fixture.dock.view().item(LAST).unwrap().is_active());
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
        titles: Titles::TightLines,
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

#[test]
fn custom_menu_title_allocation_contains_its_unelided_glyphs() {
    for titles in [
        Titles::BakedElision,
        Titles::CenterAligned,
        Titles::RightAligned,
        Titles::TightLines,
    ] {
        let mut fixture = Fixture::new(Panes {
            titles,
            ..Panes::default()
        });
        let output = fixture.open_accessibly();
        let label = fixture.panes.label(SELECTED);
        let menu_top = node(&output, Role::Menu, "Open tabs")
            .1
            .bounds()
            .unwrap()
            .y0;
        let (clip, title) = output
            .shapes
            .iter()
            .find_map(|shape| {
                let egui::Shape::Text(text) = &shape.shape else {
                    return None;
                };
                (f64::from(shape.clip_rect.top()) > menu_top && text.galley.text() == label)
                    .then_some((shape.clip_rect, text))
            })
            .expect("the selected menu row must paint its title");
        assert!(!title.galley.elided);
        let glyphs = title.visual_bounding_rect();
        assert!(
            clip.expand(1.0).contains_rect(glyphs),
            "row clip {clip:?} misses glyphs {glyphs:?}"
        );
    }
}

#[test]
fn a_blank_title_can_still_be_selected_in_a_scrollable_menu() {
    let mut fixture = Fixture::new(Panes {
        titles: Titles::Blank,
        ..Panes::default()
    });
    let output = fixture.open_accessibly();
    node(&output, Role::Menu, "Open tabs");
    fixture.run(vec![key(Key::End), key(Key::Enter)]);
    fixture.settle();
    assert!(fixture.dock.view().item(LAST).unwrap().is_active());
}
