//! Public tab callbacks preserve core geometry, accessibility and input ownership.

use dockspace::policy::{DockPolicy, TabBarInteraction, TabBarPolicy, TabBarVisibility};
use egui::accesskit::Role;
use egui::{Color32, Context, Event, FullOutput, Pos2, RawInput, Rect, Ui, vec2};
use egui_dockspace::{
    DockStyle, Dockspace, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, DragPreview, DropPreviewTab, ItemId, PaneResponse, PaneView, RootId,
    SurfaceId, SurfacePresentationResult, Tab, TabBar, TabBarId, TabBarLayout, TabBarSlot,
    TabClose, TabCloseVisibility, TabStylePreset, TabVisuals,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);
const CUSTOM_FILL: Color32 = Color32::from_rgb(22, 140, 120);
const PREVIEW_FILL: Color32 = Color32::from_rgb(151, 61, 123);

struct CloseSnapshot {
    item: ItemId,
    rect: Rect,
    icon: Rect,
    color: Color32,
}

#[derive(Clone)]
struct TabSnapshot {
    item: ItemId,
    id: egui::Id,
    bar: TabBarId,
    rect: Rect,
    background: Rect,
    visible: Rect,
    content: Rect,
    enabled: bool,
    selected: bool,
    pane_focused: bool,
    visuals: TabVisuals,
}

#[derive(Clone)]
struct SlotSnapshot {
    bar: TabBarId,
    slot: TabBarSlot,
    rect: Rect,
    clip: Rect,
    enabled: bool,
    items: Vec<ItemId>,
}

struct Panes {
    extra_width: f32,
    layout: TabBarLayout,
    tabs: Vec<TabSnapshot>,
    slots: Vec<SlotSnapshot>,
    tab_button: Option<Rect>,
    slot_button: Option<Rect>,
    tab_clicks: usize,
    slot_clicks: usize,
    slot_menu: bool,
    missing: bool,
    custom_visuals: bool,
    closes: Vec<CloseSnapshot>,
    after_button: Option<Rect>,
    after_clicks: usize,
    previews: Vec<Rect>,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            extra_width: 40.0,
            layout: TabBarLayout {
                leading_width: 36.0,
                trailing_width: 64.0,
                ..Default::default()
            },
            tabs: Vec::new(),
            slots: Vec::new(),
            tab_button: None,
            slot_button: None,
            tab_clicks: 0,
            slot_clicks: 0,
            slot_menu: false,
            missing: false,
            custom_visuals: false,
            closes: Vec::new(),
            after_button: None,
            after_clicks: 0,
            previews: Vec::new(),
        }
    }
}

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        (!self.missing).then(|| if item == FIRST { "First" } else { "Second" }.into())
    }

    fn ui(&mut self, _item: ItemId, _ui: &mut Ui) -> PaneResponse {
        PaneResponse::default()
    }

    fn tab_content_width(
        &self,
        _key: egui_dockspace::TabKey,
        _ui: &Ui,
        title: &egui::Galley,
    ) -> f32 {
        title.size().x + self.extra_width
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        let Some(item) = tab.item() else {
            return tab.paint(ui);
        };
        self.tabs.push(TabSnapshot {
            item: item,
            id: tab.response.id,
            bar: tab.bar,
            rect: tab.rect,
            background: tab.background_rect(),
            visible: tab.visible_rect,
            content: tab.content_rect,
            enabled: tab.enabled && ui.is_enabled(),
            selected: tab.selected,
            pane_focused: tab.pane_focused,
            visuals: tab.visuals,
        });
        let mut custom = tab.clone();
        custom.visuals.fill = CUSTOM_FILL;
        if self.custom_visuals {
            custom.visuals.corner_radius = egui::CornerRadius::same(9);
            custom.visuals.background_inset = vec2(6.0, 4.0);
            custom.visuals.focus_stroke = egui::Stroke::new(3.0, Color32::MAGENTA);
            custom.visuals.text_color_override = Some(Color32::WHITE);
            custom.visuals.close_color = Some(Color32::WHITE);
        }
        custom.paint_background(ui.painter());
        let button = Rect::from_min_max(
            egui::pos2(tab.content_rect.right() - 24.0, tab.content_rect.top()),
            tab.content_rect.right_bottom(),
        );
        let title_rect = Rect::from_min_max(tab.content_rect.min, button.left_bottom());
        custom.paint_title(ui, title_rect);
        if item == SECOND {
            let response = ui.put(button, egui::Button::new("!"));
            self.tab_button = Some(response.rect);
            self.tab_clicks += usize::from(response.clicked());
        }
        custom.visuals
    }

    fn tab_close_ui(&mut self, ui: &Ui, close: &TabClose<'_>) {
        let Some(item) = close.item() else {
            close.paint(ui);
            return;
        };
        assert!(close.rect.contains_rect(ui.clip_rect()));
        self.closes.push(CloseSnapshot {
            item: item,
            rect: close.rect,
            icon: close.icon_rect,
            color: close.stroke.color,
        });
        close.paint(ui);
    }

    fn drag_preview_ui(&mut self, ui: &Ui, preview: &DragPreview) {
        assert!(preview.rect.contains_rect(ui.clip_rect()));
        self.previews.push(preview.rect);
        let mut preview = preview.clone();
        preview.fill = PREVIEW_FILL;
        preview.paint(ui);
    }

    fn tab_bar_layout(&self, _bar: TabBarId, _ui: &Ui) -> TabBarLayout {
        self.layout
    }

    fn tab_bar_ui(&mut self, ui: &mut Ui, bar: TabBar<'_>, slot: TabBarSlot) {
        self.slots.push(SlotSnapshot {
            bar: bar.id(),
            slot,
            rect: ui.max_rect(),
            clip: ui.clip_rect(),
            enabled: ui.is_enabled(),
            items: bar.items().collect(),
        });
        if slot == TabBarSlot::Trailing {
            let response = if self.slot_menu {
                ui.menu_button("Action", |ui| ui.label("Custom action"))
                    .response
            } else {
                ui.button("Action")
            };
            self.slot_button = Some(response.rect);
            self.slot_clicks += usize::from(response.clicked());
        } else if slot == TabBarSlot::AfterTabs {
            let response = ui.button("+");
            self.after_button = Some(response.rect.intersect(ui.clip_rect()));
            self.after_clicks += usize::from(response.clicked());
        } else {
            ui.label("2");
        }
    }
}

struct Harness {
    context: Context,
    dock: Dockspace,
    // Exercise forwarding of every new callback through Box<PaneView>.
    panes: Box<Panes>,
    close_requests: usize,
    width: f32,
}

impl Harness {
    fn new(policy: DockPolicy) -> Self {
        let context = Context::default();
        context.enable_accesskit();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), DockspaceNode::tabs([FIRST, SECOND])),
        )])
        .unwrap();
        let mut style = DockStyle {
            tab_corner_radius: egui::CornerRadius::same(6),
            ..Default::default()
        };
        style.visuals.tab_stroke = Some(egui::Stroke::new(1.0, Color32::RED));
        Self {
            context,
            dock: Dockspace::builder("custom-tab-test", layout)
                .policy(policy)
                .style(style)
                .build()
                .unwrap(),
            panes: Box::default(),
            close_requests: 0,
            width: 800.0,
        }
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        self.panes.tabs.clear();
        self.panes.slots.clear();
        self.panes.closes.clear();
        self.panes.after_button = None;
        self.panes.previews.clear();
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 400.0))),
            time: Some(self.context.input(|input| input.time) + 0.1),
            events,
            ..Default::default()
        };
        let (output, response) = self
            .dock
            .run_single_surface(&self.context, input, SURFACE, &mut self.panes)
            .unwrap();
        assert_eq!(
            response.presentation_settlement_capability(),
            egui_dockspace::DockspaceCapability::Supported
        );
        self.close_requests += response
            .close_request_events()
            .iter()
            .filter(|event| !event.reused())
            .count();
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new()).drop_without_applying_deltas();
        }
    }

    fn click(&mut self, point: Pos2) {
        self.run(vec![Event::PointerMoved(point)])
            .drop_without_applying_deltas();
        for pressed in [true, false] {
            self.run(vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }])
            .drop_without_applying_deltas();
        }
        self.settle();
    }

    fn tab(&self, item: ItemId) -> &TabSnapshot {
        self.panes.tabs.iter().find(|tab| tab.item == item).unwrap()
    }

    fn accesskit_action(&mut self, label: &str, action: egui::accesskit::Action) {
        let output = self.run(Vec::new());
        let target = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some(label)).then_some(*id))
            .unwrap();
        self.run(vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: target,
                data: None,
            },
        )])
        .drop_without_applying_deltas();
        self.settle();
    }
}

#[test]
fn tab_presets_follow_theme_hover_and_selection_while_custom_callbacks_still_win() {
    for preset in [
        TabStylePreset::Dockspace,
        TabStylePreset::Rounded,
        TabStylePreset::EguiTiles,
    ] {
        let mut harness = Harness::new(DockPolicy::default());
        let mut style = harness.dock.style().clone();
        style.apply_tab_preset(preset);
        style.tab_bar_height = 31.0;
        harness.dock.set_style(style).unwrap();
        for theme in [egui::Visuals::light(), egui::Visuals::dark()] {
            harness.context.set_visuals(theme.clone());
            harness
                .run(vec![Event::PointerGone])
                .drop_without_applying_deltas();
            harness.settle();
            let first = harness.tab(FIRST);
            assert!(first.selected && !first.pane_focused);
            assert_eq!(
                first.rect.height(),
                31.0,
                "rendering must retain manual metric edits"
            );
            assert_eq!(
                first.visuals.connect_to_pane,
                preset == TabStylePreset::EguiTiles
            );
            assert_eq!(first.visuals.overline_stroke, egui::Stroke::NONE);
            if preset == TabStylePreset::Rounded {
                assert_eq!(first.visuals.corner_radius, egui::CornerRadius::same(6));
                assert_eq!(first.visuals.background_inset, vec2(2.0, 2.0));
                assert_eq!(first.visuals.stroke.width, 1.0);
                assert_ne!(first.visuals.fill, theme.panel_fill);
                assert_eq!(first.visuals.text_color, theme.strong_text_color());
            } else {
                assert_eq!(first.visuals.fill, theme.panel_fill);
                assert_eq!(first.visuals.background_inset, egui::Vec2::ZERO);
            }
            let idle = harness.tab(SECOND).visuals.fill;
            let target = harness.tab(SECOND).content.left_center() + vec2(2.0, 0.0);
            let output = harness.run(vec![Event::PointerMoved(target)]);
            harness.settle();
            if preset == TabStylePreset::EguiTiles {
                assert_eq!(harness.tab(SECOND).visuals.fill, idle);
            } else {
                assert_ne!(harness.tab(SECOND).visuals.fill, idle);
            }
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(rect) if rect.fill == CUSTOM_FILL
                        && rect.rect.contains(harness.tab(SECOND).rect.center())
                )),
                "the application callback can override every recipe"
            );
        }
    }
}

#[test]
fn rounded_tab_custom_overline_follows_the_public_tabs_rounded_background() {
    let mut harness = Harness::new(DockPolicy::default());
    let overline = egui::Stroke::new(2.0, Color32::from_rgb(231, 15, 172));
    let radius = egui::CornerRadius {
        nw: 5,
        ne: 5,
        sw: 0,
        se: 0,
    };
    let mut style = harness.dock.style().clone();
    style.apply_tab_preset(TabStylePreset::Rounded);
    style.tab_corner_radius = radius;
    style.tab_background_inset = egui::Vec2::ZERO;
    style.visuals.tab_selected_overline = Some(overline);
    harness.dock.set_style(style).unwrap();
    harness.settle();

    let output = harness.run(vec![Event::PointerGone]);
    let tab = harness.tab(FIRST);
    assert_eq!(tab.visuals.corner_radius, radius);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(background)
            if background.fill == CUSTOM_FILL
                && background.rect == tab.rect
                && background.corner_radius == radius
    )));
    let (clip, path) = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.stroke == overline.into() => {
                Some((shape.clip_rect, path))
            }
            _ => None,
        })
        .expect("the selected overline must follow both rounded corners as one open path");
    assert!(!path.closed);
    assert_eq!(path.fill, Color32::TRANSPARENT);
    assert!(tab.visible.contains_rect(clip));
    let top = tab.rect.top() + overline.width * 0.5;
    assert!(
        path.points
            .iter()
            .any(|point| (point.y - top).abs() < 0.001)
    );
    for side in [path.points.first().unwrap(), path.points.last().unwrap()] {
        assert!(
            side.y > top + 1.0,
            "both ends must turn down the rounded corners instead of stopping on the flat top"
        );
    }
    assert!(
        path.points.windows(2).all(|pair| pair[0] != pair[1]),
        "duplicate vertices can produce stroke artifacts"
    );
    let (preview, preview_background) = paint_preview_tab_background(
        tab.rect,
        tab.visible,
        clip,
        radius,
        egui::Vec2::ZERO,
        overline,
    );
    assert_eq!(preview_background, tab.background);
    let (preview_clip, preview_path) = painted_overline(&preview, overline);
    assert_eq!(preview_clip, clip);
    assert_eq!(
        preview_path, path,
        "live and future tabs must share their contour"
    );
}

fn paint_preview_tab_background(
    rect: Rect,
    visible: Rect,
    clip: Rect,
    radius: egui::CornerRadius,
    background_inset: egui::Vec2,
    overline: egui::Stroke,
) -> (FullOutput, Rect) {
    let mut background = Rect::NOTHING;
    let mut output = Context::default().run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 768.0))),
            ..Default::default()
        },
        |ui| {
            ui.set_clip_rect(clip);
            let tab = DropPreviewTab {
                key: FIRST.into(),
                ordinal: 0,
                selected: true,
                is_payload: true,
                rect,
                visible_rect: Some(visible),
                title_rect: None,
                title_clip_rect: None,
                title: ui.painter().layout_no_wrap(
                    "Future tab".into(),
                    egui::FontId::default(),
                    Color32::WHITE,
                ),
                fill: CUSTOM_FILL,
                stroke: egui::Stroke::NONE,
                overline_stroke: overline,
                connect_to_pane: false,
                corner_radius: radius,
                background_inset,
                text_color: Color32::WHITE,
                text_color_override: None,
            };
            background = tab.background_rect();
            tab.paint_background(ui);
        },
    );
    output.textures_delta.clear();
    (output, background)
}

fn painted_overline(output: &FullOutput, stroke: egui::Stroke) -> (Rect, &egui::epaint::PathShape) {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.stroke == stroke.into() => {
                Some((shape.clip_rect, path))
            }
            _ => None,
        })
        .expect("a rounded overline is one open path")
}

#[test]
fn rounded_preview_overline_clamps_large_radii_in_a_narrow_tab() {
    let rect = Rect::from_min_size(egui::pos2(20.0, 30.0), vec2(6.0, 24.0));
    let stroke = egui::Stroke::new(2.0, Color32::MAGENTA);
    let (output, _) = paint_preview_tab_background(
        rect,
        rect,
        rect,
        egui::CornerRadius {
            nw: u8::MAX,
            ne: u8::MAX,
            sw: 0,
            se: 0,
        },
        egui::Vec2::ZERO,
        stroke,
    );
    let (clip, path) = painted_overline(&output, stroke);
    assert_eq!(clip, rect);
    assert!(!path.closed);
    assert_eq!(path.fill, Color32::TRANSPARENT);
    assert!(path.points.len() > 2);
    assert!(
        path.points
            .iter()
            .all(|point| point.is_finite() && rect.shrink(1.0).contains(*point)),
        "the accent must stay within the tab even when both corners fill its width"
    );
    assert!(
        path.points
            .windows(2)
            .all(|pair| pair[0] != pair[1] && pair[0].x <= pair[1].x),
        "the joined arcs must not double back or repeat their shared vertex"
    );
}

#[test]
fn rounded_preview_overline_retains_asymmetric_and_square_corners() {
    let rect = Rect::from_min_size(egui::pos2(20.0, 30.0), vec2(90.0, 24.0));
    let stroke = egui::Stroke::new(2.0, Color32::MAGENTA);
    for (nw, ne) in [(0, 9), (u8::MAX, 3)] {
        let (output, _) = paint_preview_tab_background(
            rect,
            rect,
            rect,
            egui::CornerRadius {
                nw,
                ne,
                sw: 0,
                se: 0,
            },
            egui::Vec2::ZERO,
            stroke,
        );
        let (_, path) = painted_overline(&output, stroke);
        let first = path.points.first().unwrap();
        let last = path.points.last().unwrap();
        assert!((first.x - (rect.left() + 1.0)).abs() < 0.001);
        assert!((last.x - (rect.right() - 1.0)).abs() < 0.001);
        if nw == 0 {
            assert!((first.y - (rect.top() + 1.0)).abs() < 0.001);
            assert!(
                last.y > first.y + 3.0,
                "only the rounded side turns downward"
            );
        } else {
            assert!(first.y > last.y + 3.0);
            assert!(
                first.y <= rect.center().y,
                "oversized custom rounding must also respect the tab height"
            );
        }
    }
}

#[test]
fn rounded_preview_overline_preserves_the_contour_behind_both_clips() {
    let rect = Rect::from_min_size(egui::pos2(20.0, 30.0), vec2(90.0, 24.0));
    let visible = Rect::from_min_max(egui::pos2(40.0, 30.0), rect.right_bottom());
    let clip = Rect::from_min_max(Pos2::ZERO, egui::pos2(80.0, 100.0));
    let stroke = egui::Stroke::new(2.0, Color32::MAGENTA);
    let (output, _) = paint_preview_tab_background(
        rect,
        visible,
        clip,
        egui::CornerRadius {
            nw: 5,
            ne: 5,
            sw: 0,
            se: 0,
        },
        egui::Vec2::ZERO,
        stroke,
    );
    let (actual_clip, path) = painted_overline(&output, stroke);
    assert_eq!(actual_clip, visible.intersect(clip));
    assert!(path.points.first().unwrap().x < actual_clip.left());
    assert!(path.points.last().unwrap().x > actual_clip.right());
    assert!(
        path.points.windows(2).any(|pair| {
            pair[0].x < actual_clip.left()
                && pair[1].x > actual_clip.right()
                && pair
                    .iter()
                    .all(|point| (point.y - (rect.top() + 1.0)).abs() < 0.001)
        }),
        "scrolling clips the full contour instead of rounding the visible fragment"
    );
}

#[test]
fn tab_presets_keep_selection_precedence_for_keyboard_focus_and_pointer_feedback() {
    const ACTIVE: Color32 = Color32::from_rgb(13, 73, 131);
    const HOVERED: Color32 = Color32::from_rgb(211, 167, 23);
    let focus_stroke = egui::Stroke::new(3.0, Color32::MAGENTA);
    for preset in [
        TabStylePreset::Dockspace,
        TabStylePreset::Rounded,
        TabStylePreset::EguiTiles,
    ] {
        for theme in [egui::Visuals::light(), egui::Visuals::dark()] {
            let mut harness = Harness::new(DockPolicy::default());
            harness.context.set_visuals(theme);
            let mut style = harness.dock.style().clone();
            style.apply_tab_preset(preset);
            style.visuals.tab_active_fill = Some(ACTIVE);
            style.visuals.tab_hover_fill = Some(HOVERED);
            style.visuals.tab_focus_stroke = Some(focus_stroke);
            harness.dock.set_style(style).unwrap();
            harness
                .run(vec![Event::PointerGone])
                .drop_without_applying_deltas();
            harness.settle();

            harness.accesskit_action("First", egui::accesskit::Action::Focus);
            let output = harness.run(Vec::new());
            let tab = harness.tab(FIRST);
            assert!(tab.selected);
            assert!(harness.context.memory(|memory| memory.has_focus(tab.id)));
            assert_eq!(tab.visuals.fill, ACTIVE);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.stroke == focus_stroke
                    && rect.rect.contains(tab.rect.center())
            )));

            let point = tab.content.left_center() + vec2(2.0, 0.0);
            harness
                .run(vec![Event::PointerMoved(point)])
                .drop_without_applying_deltas();
            assert_eq!(harness.tab(FIRST).visuals.fill, ACTIVE);
            for pressed in [true, false] {
                harness
                    .run(vec![Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }])
                    .drop_without_applying_deltas();
                assert_eq!(harness.tab(FIRST).visuals.fill, ACTIVE);
            }
            harness
                .run(vec![Event::PointerGone])
                .drop_without_applying_deltas();
            harness.settle();
            assert!(
                harness
                    .context
                    .memory(|memory| memory.has_focus(harness.tab(FIRST).id))
            );
            assert_eq!(harness.tab(FIRST).visuals.fill, ACTIVE);
        }
    }
}

#[test]
fn rounded_keyboard_navigation_does_not_hover_an_unselected_tab() {
    let mut harness = Harness::new(DockPolicy::default());
    let mut style = harness.dock.style().clone();
    style.apply_tab_preset(TabStylePreset::Rounded);
    style.visuals.tab_fill = Some(Color32::BLUE);
    style.visuals.tab_hover_fill = Some(Color32::YELLOW);
    harness.dock.set_style(style).unwrap();
    harness
        .run(vec![Event::PointerGone])
        .drop_without_applying_deltas();
    harness.settle();
    let id = harness.tab(SECOND).id;
    harness
        .context
        .memory_mut(|memory| memory.request_focus(id));
    harness.settle();
    assert!(harness.context.memory(|memory| memory.has_focus(id)));
    assert!(!harness.tab(SECOND).selected);
    assert_eq!(harness.tab(SECOND).visuals.fill, Color32::BLUE);
}

#[test]
fn rounded_tab_background_insets_match_live_preview_and_keyboard_focus() {
    let mut harness = Harness::new(DockPolicy::default());
    let focus_stroke = egui::Stroke::new(1.0, Color32::MAGENTA);
    let mut style = harness.dock.style().clone();
    style.apply_tab_preset(TabStylePreset::Rounded);
    style.visuals.tab_focus_stroke = Some(focus_stroke);
    harness.dock.set_style(style).unwrap();
    harness.settle();
    harness.accesskit_action("First", egui::accesskit::Action::Focus);
    let output = harness.run(Vec::new());
    let tab = harness.tab(FIRST);
    assert_eq!(tab.visuals.background_inset, vec2(2.0, 2.0));
    assert_eq!(tab.background, tab.rect.shrink2(vec2(2.0, 2.0)));
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(background)
            if background.fill == CUSTOM_FILL
                && background.rect == tab.background
                && background.corner_radius == egui::CornerRadius::same(6)
    )));
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(focus)
                if focus.stroke == focus_stroke
                    && tab.background.contains_rect(focus.rect)
                    && focus.corner_radius == egui::CornerRadius::same(6)
        )),
        "keyboard focus must follow the inset background instead of the full hit allocation"
    );
    let (preview, background) = paint_preview_tab_background(
        tab.rect,
        tab.visible,
        tab.visible,
        tab.visuals.corner_radius,
        tab.visuals.background_inset,
        egui::Stroke::NONE,
    );
    assert_eq!(background, tab.background);
    assert!(preview.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(painted)
            if painted.fill == CUSTOM_FILL
                && painted.rect == background
                && painted.corner_radius == egui::CornerRadius::same(6)
    )));

    let narrow = Rect::from_min_size(egui::pos2(20.0, 30.0), vec2(6.0, 24.0));
    let (_, collapsed) = paint_preview_tab_background(
        narrow,
        narrow,
        narrow,
        tab.visuals.corner_radius,
        vec2(1000.0, 1000.0),
        egui::Stroke::NONE,
    );
    assert_eq!(
        collapsed,
        Rect::from_center_size(narrow.center(), egui::Vec2::ZERO)
    );
}

#[test]
fn rounded_tab_background_can_open_into_the_pane_with_vertical_insets() {
    let mut harness = Harness::new(DockPolicy::default());
    let mut style = harness.dock.style().clone();
    style.apply_tab_preset(TabStylePreset::Rounded);
    style.tab_connect_to_pane = true;
    harness.dock.set_style(style).unwrap();
    harness.settle();

    let output = harness.run(vec![Event::PointerGone]);
    let tab = harness.tab(FIRST);
    assert!(tab.visuals.connect_to_pane);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(background)
            if background.fill == CUSTOM_FILL
                && background.rect.min == tab.background.min
                && background.rect.right() == tab.background.right()
                && background.rect.bottom() == tab.rect.bottom() + tab.visuals.stroke.width
                && shape.clip_rect.bottom() <= tab.rect.bottom()
    )));
}

#[test]
fn returned_tab_visuals_coordinate_close_icons_and_rounded_keyboard_focus() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.panes.custom_visuals = true;
    let mut style = harness.dock.style().clone();
    style.apply_tab_preset(TabStylePreset::Rounded);
    style.tab_close_visibility = TabCloseVisibility::Always;
    style.tab_close_icon_size = 6.0;
    harness.dock.set_style(style).unwrap();
    harness.settle();
    assert_eq!(harness.panes.closes.len(), 2);
    for close in &harness.panes.closes {
        assert_eq!(close.icon.size(), vec2(6.0, 6.0));
        assert_eq!(close.color, Color32::WHITE);
        assert!(close.rect.width() > close.icon.width());
    }
    harness.accesskit_action("First", egui::accesskit::Action::Focus);
    let output = harness.run(Vec::new());
    let background = harness.tab(FIRST).rect.shrink2(vec2(6.0, 4.0));
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(rect) if rect.fill == CUSTOM_FILL
            && rect.rect == background
            && rect.corner_radius == egui::CornerRadius::same(9)
    )));
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(rect) if rect.stroke == egui::Stroke::new(3.0, Color32::MAGENTA)
            && rect.corner_radius == egui::CornerRadius::same(9)
            && background.contains_rect(rect.rect)
    )));
    let close = harness
        .panes
        .closes
        .iter()
        .find(|close| close.item == FIRST)
        .unwrap();
    harness.click(close.rect.left_center() + vec2(1.0, 0.0));
    assert_eq!(
        harness.close_requests, 1,
        "the full close target remains clickable outside the small icon"
    );
}

#[test]
fn hover_only_close_icons_keep_layout_stable_and_reveal_for_keyboard_focus() {
    let mut harness = Harness::new(DockPolicy::default());
    let mut style = harness.dock.style().clone();
    style.tab_close_visibility = TabCloseVisibility::Hovered;
    harness.dock.set_style(style).unwrap();
    harness.settle();
    let width = harness.tab(FIRST).rect.width();
    assert!(harness.panes.closes.is_empty());
    let point = harness.tab(FIRST).content.left_center() + vec2(2.0, 0.0);
    harness
        .run(vec![Event::PointerMoved(point)])
        .drop_without_applying_deltas();
    assert!(harness.panes.closes.iter().any(|close| close.item == FIRST));
    assert_eq!(harness.tab(FIRST).rect.width(), width);
    harness.accesskit_action("Second", egui::accesskit::Action::Focus);
    assert!(
        harness
            .panes
            .closes
            .iter()
            .any(|close| close.item == SECOND)
    );
}

#[test]
fn after_tabs_button_scrolls_with_tabs_and_keeps_its_own_input() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.panes.layout.after_tabs_width = 40.0;
    harness.settle();
    let slot = harness
        .panes
        .slots
        .iter()
        .find(|slot| slot.slot == TabBarSlot::AfterTabs)
        .unwrap();
    assert_eq!(slot.rect.left(), harness.tab(SECOND).rect.right());
    harness.width = 380.0;
    let mut style = harness.dock.style().clone();
    style.tab_min_width = 160.0;
    style.tab_max_width = 160.0;
    harness.dock.set_style(style).unwrap();
    harness.settle();
    let pinned = harness
        .panes
        .slots
        .iter()
        .filter(|slot| slot.slot != TabBarSlot::AfterTabs)
        .cloned()
        .collect::<Vec<_>>();
    for _ in 0..8 {
        if harness
            .panes
            .slots
            .iter()
            .any(|slot| slot.slot == TabBarSlot::AfterTabs && slot.clip.width() >= 40.0)
        {
            break;
        }
        harness.accesskit_action("Scroll tabs forward", egui::accesskit::Action::Click);
    }
    let slot = harness
        .panes
        .slots
        .iter()
        .find(|slot| slot.slot == TabBarSlot::AfterTabs)
        .unwrap();
    assert_eq!(slot.rect.left(), harness.tab(SECOND).rect.right());
    assert_eq!(slot.clip.width(), 40.0);
    for before in pinned {
        let after = harness
            .panes
            .slots
            .iter()
            .find(|slot| slot.slot == before.slot)
            .unwrap();
        assert_eq!(after.rect, before.rect);
    }
    harness.click(harness.panes.after_button.unwrap().center());
    assert_eq!(harness.panes.after_clicks, 1);
    assert_eq!(harness.close_requests, 0);
    assert!(harness.dock.view().item(FIRST).unwrap().is_active());
}

#[test]
fn application_drag_preview_paints_inside_the_core_owned_gesture() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.settle();
    let start = harness.tab(FIRST).content.left_center() + vec2(3.0, 0.0);
    harness
        .run(vec![
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ])
        .drop_without_applying_deltas();
    let target = start + vec2(40.0, 80.0);
    harness
        .run(vec![Event::PointerMoved(target)])
        .drop_without_applying_deltas();
    let output = harness.run(Vec::new());
    assert!(!harness.panes.previews.is_empty());
    assert!(
        output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == PREVIEW_FILL)
        )
    );
    harness
        .run(vec![Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }])
        .drop_without_applying_deltas();
    harness.settle();
    assert!(harness.panes.previews.is_empty());
    assert!(harness.dock.view().item(FIRST).is_some());
    assert_eq!(harness.close_requests, 0);
}

#[test]
fn hidden_grip_reclaims_space_and_background_still_starts_a_group_drag() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.settle();
    let original_left = harness.tab(FIRST).rect.left();
    let mut style = harness.dock.style().clone();
    let grip_width = style.tab_group_grip_extent;
    style.tab_group_grip_visibility = egui_dockspace::TabGroupGripVisibility::Hidden;
    style.tab_bar_height = 20.0;
    harness.dock.set_style(style).unwrap();
    harness.settle();
    assert_eq!(original_left - harness.tab(FIRST).rect.left(), grip_width);
    let start = harness.tab(SECOND).rect.right_center() + vec2(20.0, 0.0);
    harness
        .run(vec![
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ])
        .drop_without_applying_deltas();
    harness
        .run(vec![Event::PointerMoved(start + vec2(30.0, 60.0))])
        .drop_without_applying_deltas();
    harness.run(Vec::new()).drop_without_applying_deltas();
    assert!(
        !harness.panes.previews.is_empty(),
        "the trailing empty region still owns the group gesture"
    );
    harness
        .run(vec![Event::PointerGone])
        .drop_without_applying_deltas();
}

#[test]
fn custom_width_and_fixed_slots_share_the_core_allocation() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.settle();
    let leading = harness
        .panes
        .slots
        .iter()
        .find(|slot| slot.slot == TabBarSlot::Leading)
        .unwrap();
    let trailing = harness
        .panes
        .slots
        .iter()
        .find(|slot| slot.slot == TabBarSlot::Trailing)
        .unwrap();
    assert_eq!(leading.rect.width(), 36.0);
    assert_eq!(trailing.rect.width(), 64.0);
    for slot in &harness.panes.slots {
        assert!(slot.rect.contains_rect(slot.clip));
        assert_eq!(slot.items, [FIRST, SECOND]);
        assert!(slot.enabled);
        for tab in &harness.panes.tabs {
            assert_eq!(tab.bar, slot.bar);
            assert!(!tab.visible.intersect(slot.rect).is_positive());
        }
    }
    let original = harness.tab(FIRST).rect.width();
    harness.panes.extra_width += 32.0;
    harness.settle();
    assert_eq!(harness.tab(FIRST).rect.width(), original + 32.0);
}

#[test]
fn custom_paint_keeps_default_selection_accessibility_and_close_controls() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.settle();
    let output = harness.run(Vec::new());
    assert!(output.shapes.iter().any(|shape| matches!(
        &shape.shape, egui::Shape::Rect(rect)
            if rect.fill == CUSTOM_FILL && rect.corner_radius == egui::CornerRadius::same(6)
    )));
    let point = harness.tab(SECOND).content.left_center() + vec2(4.0, 0.0);
    harness.click(point);
    assert!(harness.dock.view().item(SECOND).unwrap().is_active());
    let output = harness.run(Vec::new());
    let nodes = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    assert!(
        nodes
            .iter()
            .any(|(_, node)| node.role() == Role::Tab && node.label() == Some("Second"))
    );
    let close = nodes
        .iter()
        .find_map(|(_, node)| {
            (node.role() == Role::Button && node.label() == Some("Close Second"))
                .then(|| node.bounds().unwrap())
        })
        .unwrap();
    #[allow(clippy::cast_possible_truncation)]
    let point = egui::pos2(
        ((close.x0 + close.x1) * 0.5) as f32,
        ((close.y0 + close.y1) * 0.5) as f32,
    );
    harness.click(point);
    assert_eq!(harness.close_requests, 1);
}

#[test]
fn application_buttons_own_clicks_without_selecting_the_underlying_tab() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.settle();
    harness.click(harness.panes.tab_button.unwrap().center());
    assert_eq!(harness.panes.tab_clicks, 1);
    assert!(harness.dock.view().item(FIRST).unwrap().is_active());
    harness.click(harness.panes.slot_button.unwrap().center());
    assert_eq!(harness.panes.slot_clicks, 1);
    assert!(harness.dock.view().item(FIRST).unwrap().is_active());
    assert_eq!(harness.close_requests, 0);
}

#[test]
fn ordinary_egui_menus_in_slots_keep_complete_output_settlement() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.panes.slot_menu = true;
    harness.settle();
    harness.click(harness.panes.slot_button.unwrap().center());
    let output = harness.run(Vec::new());
    let texts = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(texts.contains(&"Custom action"), "painted text: {texts:?}");
    assert!(harness.dock.view().item(FIRST).unwrap().is_active());
}

#[test]
fn oversubscribed_slots_shrink_without_overlapping_operable_tabs() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.width = 200.0;
    harness.panes.layout = TabBarLayout {
        leading_width: 5000.0,
        trailing_width: 5000.0,
        ..Default::default()
    };
    harness.settle();
    assert!(!harness.panes.slots.is_empty());
    for slot in &harness.panes.slots {
        assert!(slot.rect.left() >= 0.0 && slot.rect.right() <= harness.width);
        for tab in &harness.panes.tabs {
            assert!(!tab.visible.intersect(slot.rect).is_positive());
        }
    }
    assert!(harness.tab(FIRST).visible.width() >= harness.dock.style().tab_min_width);
    harness.width = 800.0;
    harness.panes.layout = TabBarLayout::default();
    harness.settle();
    assert!(harness.panes.slots.is_empty());
    assert_eq!(harness.panes.tabs.len(), 2);
}

#[test]
fn disabled_and_hidden_bars_apply_to_application_chrome() {
    let mut policy = DockPolicy::default();
    policy.set_tab_bar(TabBarPolicy::new(
        TabBarVisibility::Visible,
        TabBarInteraction::Disabled,
    ));
    let mut harness = Harness::new(policy);
    harness.panes.layout.after_tabs_width = 32.0;
    harness.settle();
    assert_eq!(harness.panes.tabs.len(), 2);
    assert!(harness.panes.tabs.iter().all(|tab| !tab.enabled));
    assert!(harness.panes.slots.iter().all(|slot| !slot.enabled));
    harness.click(harness.panes.slot_button.unwrap().center());
    assert_eq!(harness.panes.slot_clicks, 0);
    let mut policy = DockPolicy::default();
    policy.set_tab_bar(TabBarPolicy::new(
        TabBarVisibility::Hidden,
        TabBarInteraction::Disabled,
    ));
    harness.dock.set_policy(policy).unwrap();
    harness.settle();
    assert!(harness.panes.tabs.is_empty());
    assert!(harness.panes.slots.is_empty());
}

#[test]
fn unloaded_panes_use_default_missing_tabs_without_application_callbacks() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.panes.missing = true;
    harness.settle();
    assert!(harness.panes.tabs.is_empty());
    let output = harness.run(Vec::new());
    assert!(harness.panes.closes.is_empty());
    assert!(
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| {
                node.role() == Role::Tab && node.label() == Some("Missing pane 1")
            })
    );
}

#[test]
fn fixed_slots_keep_their_identity_and_position_while_tabs_scroll() {
    let mut harness = Harness::new(DockPolicy::default());
    harness.width = 380.0;
    let mut style = harness.dock.style().clone();
    style.tab_min_width = 160.0;
    style.tab_max_width = 160.0;
    harness.dock.set_style(style).unwrap();
    harness.settle();
    let before = harness.panes.slots.clone();
    let second_before = harness.tab(SECOND).rect.left();
    let output = harness.run(Vec::new());
    let target = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(id, node)| (node.label() == Some("Scroll tabs forward")).then_some(*id))
        .unwrap();
    harness
        .run(vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: target,
                data: None,
            },
        )])
        .drop_without_applying_deltas();
    harness.settle();
    assert!(harness.tab(SECOND).rect.left() < second_before);
    for (before, after) in before.iter().zip(&harness.panes.slots) {
        assert_eq!(before.bar, after.bar);
        assert_eq!(before.slot, after.slot);
        assert_eq!(before.rect, after.rect);
    }
}

#[test]
fn invalid_application_measurements_are_rejected_at_the_host_boundary() {
    for (extra_width, leading_width) in [(f32::NAN, 0.0), (0.0, -1.0), (0.0, f32::INFINITY)] {
        let mut harness = Harness::new(DockPolicy::default());
        harness.panes.extra_width = extra_width;
        harness.panes.layout.leading_width = leading_width;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))),
            ..Default::default()
        };
        assert!(
            harness
                .dock
                .run_single_surface(&harness.context, input, SURFACE, &mut harness.panes)
                .is_err()
        );
    }
}
