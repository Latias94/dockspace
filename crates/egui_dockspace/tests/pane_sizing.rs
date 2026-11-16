//! Application sizing and splitter reset behavior through the public egui facade.

use egui::accesskit::Role;
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, Ui};
use egui_dockspace::{
    DockInputConfig, DockStyle, Dockspace, DockspaceAxis, DockspaceLayout, DockspaceNode,
    DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, PaneResponse, PaneSizing, PaneView,
    RootId, SurfaceId, SurfacePresentationResult,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);

struct Panes {
    first_sizing: PaneSizing,
    rects: [Option<Rect>; 2],
}

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(if item == FIRST { "First" } else { "Second" }.into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        self.rects[usize::from(item != FIRST)] = Some(ui.max_rect());
        PaneResponse::default()
    }

    fn sizing(&self, item: ItemId) -> PaneSizing {
        if item == FIRST {
            self.first_sizing
        } else {
            PaneSizing::default()
        }
    }
}

struct Harness {
    context: Context,
    dock: Dockspace,
    panes: Panes,
    owned: bool,
    time: f64,
}

impl Harness {
    fn new(owned: bool, first_sizing: PaneSizing, input: DockInputConfig) -> Self {
        let context = Context::default();
        context.enable_accesskit();
        context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::split(
                    DockspaceAxis::Horizontal,
                    [
                        (DockspaceNode::tabs([FIRST]), 0.25),
                        (DockspaceNode::tabs([SECOND]), 0.75),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        Self {
            context,
            dock: Dockspace::builder(("pane-sizing-contract", owned), layout)
                .style(DockStyle {
                    minimum_pane_size: egui::vec2(128.0, 64.0),
                    ..Default::default()
                })
                .input(input)
                .build()
                .unwrap(),
            panes: Panes {
                first_sizing,
                rects: [None, None],
            },
            owned,
            time: 0.0,
        }
    }

    fn frame(&mut self, width: f32, events: Vec<Event>) -> FullOutput {
        self.time += 0.05;
        self.panes.rects = [None, None];
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.0))),
            focused: true,
            time: Some(self.time),
            events,
            ..RawInput::default()
        };
        let output = if self.owned {
            self.dock
                .run_single_surface(&self.context, input, SURFACE, &mut self.panes)
                .unwrap()
                .0
        } else {
            egui_dockspace::run_ui(&self.context, input, |ui| {
                self.dock
                    .show_single_surface(SURFACE, ui, &mut self.panes)
                    .unwrap();
            })
        };
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self, width: f32) -> FullOutput {
        let mut output = self.frame(width, Vec::new());
        for _ in 0..3 {
            output = self.frame(width, Vec::new());
        }
        output
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "accessible splitter bounds fit the finite test viewport"
    )]
    fn double_click(&mut self, width: f32) {
        let output = self.settle(width);
        let bounds = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::Splitter)
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let position = Pos2::new(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        self.frame(width, vec![Event::PointerMoved(position)]);
        for _ in 0..2 {
            for pressed in [true, false] {
                self.frame(
                    width,
                    vec![Event::PointerButton {
                        pos: position,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    }],
                );
            }
        }
        self.settle(width);
    }

    fn widths(&self) -> [f32; 2] {
        self.panes.rects.map(|rect| rect.unwrap().width())
    }
}

#[test]
fn explicit_fixed_and_maximum_widths_can_be_smaller_than_the_style_floor() {
    for owned in [true, false] {
        for (sizing, expected) in [
            (
                PaneSizing::builder().fixed_width(40.0).build().unwrap(),
                40.0,
            ),
            (PaneSizing::builder().max_width(76.0).build().unwrap(), 76.0),
        ] {
            let mut harness = Harness::new(owned, sizing, DockInputConfig::default());
            harness.settle(800.0);
            let [first, second] = harness.widths();
            assert!((first - expected).abs() <= 1.0, "{first} != {expected}");
            harness.settle(1000.0);
            let [resized_first, resized_second] = harness.widths();
            assert!((resized_first - expected).abs() <= 1.0);
            assert!(resized_second > second + 150.0);
        }
    }
}

#[test]
fn preferred_width_applies_only_to_explicit_double_click_reset() {
    for owned in [true, false] {
        let sizing = PaneSizing::builder()
            .min_width(64.0)
            .max_width(300.0)
            .preferred_width(180.0)
            .build()
            .unwrap();
        let mut harness = Harness::new(owned, sizing, DockInputConfig::default());
        harness.settle(800.0);
        assert!((harness.widths()[0] - 180.0).abs() > 10.0);

        harness.double_click(800.0);
        assert!((harness.widths()[0] - 180.0).abs() <= 1.0);

        harness.settle(1000.0);
        assert!(
            harness.widths()[0] > 190.0,
            "stored proportions survive resizing"
        );
        harness.double_click(1000.0);
        assert!((harness.widths()[0] - 180.0).abs() <= 1.0);
    }
}

#[test]
fn default_double_click_without_preferences_equalizes_and_can_be_disabled() {
    for owned in [true, false] {
        for enabled in [true, false] {
            let mut input = DockInputConfig::default();
            if !enabled {
                input.splitter_double_click = None;
            }
            let mut harness = Harness::new(owned, PaneSizing::default(), input);
            harness.settle(800.0);
            let before = harness.widths();
            assert!((before[0] - before[1]).abs() > 100.0);
            harness.double_click(800.0);
            let after = harness.widths();
            if enabled {
                assert!((after[0] - after[1]).abs() <= 1.0);
            } else {
                assert!((after[0] - before[0]).abs() <= 1.0);
                assert!((after[1] - before[1]).abs() <= 1.0);
            }
        }
    }
}
