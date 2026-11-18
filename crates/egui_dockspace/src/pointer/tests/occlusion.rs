//! Real framework ordering must shield floating chrome from covered content.

use egui::accesskit::Role;
use egui::{Context, Event, FullOutput, Id, Modifiers, Pos2, RawInput, Rect, Sense, Ui, vec2};

use crate::{
    Dockspace, DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, FloatingPresentationId, ItemId, LogicalRect, PaneResponse, PaneView,
    RootId, SurfaceId, SurfacePresentationResult,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FLOATING: FloatingPresentationId = FloatingPresentationId::new(1);

struct UndeclaredBackground;

impl PaneView for UndeclaredBackground {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(
            if item.get() == 1 {
                "Canvas"
            } else {
                "Inspector"
            }
            .into(),
        )
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        if item.get() == 1 {
            ui.interact(
                ui.available_rect_before_wrap(),
                Id::unique("undeclared-background-canvas"),
                Sense::click_and_drag(),
            );
        } else {
            ui.label("Floating inspector content");
        }
        PaneResponse::default()
    }
}

struct Fixture {
    context: Context,
    dockspace: Dockspace,
    tick: u32,
}

#[derive(Clone, Copy, Debug)]
enum OverlayLayer {
    Shared,
    Foreground,
    ClickThroughArea,
}

impl Fixture {
    fn new() -> Self {
        let context = Context::default();
        context.enable_accesskit();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::central_tabs([ItemId::new(1)]),
            ),
        )
        .with_contained(DockspaceContainedLayout::new(
            FLOATING,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([ItemId::new(2)])),
            LogicalRect::new(160.0, 100.0, 320.0, 300.0).unwrap(),
        ))])
        .unwrap();
        let dockspace = Dockspace::builder("floating-chrome-occlusion", layout)
            .build()
            .unwrap();
        Self {
            context,
            dockspace,
            tick: 0,
        }
    }

    fn frame(&mut self, events: Vec<Event>, overlay: Option<(Rect, OverlayLayer)>) -> FullOutput {
        self.tick += 1;
        let (mut output, receipt) = crate::run_ui(
            &self.context,
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(f64::from(self.tick) * 0.1),
                events,
                ..Default::default()
            },
            |ui| {
                self.dockspace
                    .show_single_surface(SURFACE, ui, &mut UndeclaredBackground)
                    .unwrap();
                if let Some((rect, layer)) = overlay {
                    let show = |ui: &mut Ui| {
                        ui.interact(
                            rect,
                            Id::unique("undeclared-foreground-control"),
                            Sense::click_and_drag(),
                        );
                    };
                    match layer {
                        OverlayLayer::Shared => show(ui),
                        OverlayLayer::Foreground => {
                            ui.scope_builder(
                                egui::UiBuilder::new().layer_id(egui::LayerId::new(
                                    egui::Order::Foreground,
                                    Id::unique("overlay-without-an-area"),
                                )),
                                show,
                            );
                        }
                        OverlayLayer::ClickThroughArea => {
                            egui::Area::new(Id::unique("click-through-overlay"))
                                .order(egui::Order::Foreground)
                                .fixed_pos(rect.min)
                                .interactable(false)
                                .show(ui.ctx(), |ui| {
                                    ui.allocate_rect(rect, Sense::hover());
                                    show(ui);
                                });
                        }
                    }
                }
            },
        )
        .into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn bounds(&self) -> LogicalRect {
        self.dockspace.view().contained(FLOATING).unwrap().rect()
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::SHIFT,
    }
}

#[test]
fn covered_undeclared_content_does_not_block_foreground_floating_chrome() {
    for (role, label) in [
        (Role::TitleBar, "Tabs"),
        (Role::Splitter, "Resize floating right edge"),
    ] {
        for (layer, blocked) in [
            (None, false),
            (Some(OverlayLayer::Shared), true),
            (Some(OverlayLayer::Foreground), true),
            (Some(OverlayLayer::ClickThroughArea), false),
        ] {
            let mut fixture = Fixture::new();
            let mut output = fixture.frame(Vec::new(), None);
            for _ in 0..3 {
                output = fixture.frame(Vec::new(), None);
            }
            let (title_id, title) = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.role() == role && node.label() == Some(label))
                .map(|(id, node)| (*id, node.bounds().unwrap()))
                .unwrap();
            #[allow(clippy::cast_possible_truncation)]
            let point = Pos2::new(
                ((title.x0 + title.x1) * 0.5) as f32,
                ((title.y0 + title.y1) * 0.5) as f32,
            );
            let overlay =
                layer.map(|layer| (Rect::from_center_size(point, vec2(80.0, 20.0)), layer));
            fixture.frame(vec![Event::PointerMoved(point)], overlay);
            fixture.frame(vec![Event::PointerMoved(point)], overlay);
            fixture.context.viewport(|viewport| {
                if matches!(layer, Some(OverlayLayer::ClickThroughArea)) {
                    let widget = viewport
                        .prev_pass
                        .widgets
                        .get(Id::unique("undeclared-foreground-control"))
                        .unwrap();
                    assert!(widget.enabled && widget.sense.interactive());
                    assert!(widget.interact_rect.contains(point));
                }
                assert_eq!(
                    viewport.hits.drag.unwrap().id.value(),
                    if blocked {
                        Id::unique("undeclared-foreground-control").value()
                    } else {
                        title_id.0
                    },
                    "the real egui hit must follow layer interaction: {layer:?}"
                );
                if blocked || role == Role::TitleBar {
                    assert_eq!(
                        viewport.hits.click.unwrap().id.value(),
                        viewport.hits.drag.unwrap().id.value()
                    );
                }
            });
            let before = fixture.bounds();
            fixture.frame(vec![button(point, true)], overlay);
            assert_eq!(
                fixture.dockspace.has_active_gesture(),
                !blocked,
                "{layer:?}"
            );
            let moved = point + vec2(64.0, 32.0);
            fixture.frame(
                vec![
                    Event::ModifiersChanged(Modifiers::SHIFT),
                    Event::PointerMoved(moved),
                ],
                overlay,
            );
            fixture.frame(vec![button(moved, false)], overlay);
            fixture.frame(Vec::new(), overlay);
            let after = fixture.bounds();
            if blocked {
                assert_eq!(
                    after, before,
                    "an actual foreground widget must still block dragging"
                );
            } else if role == Role::TitleBar {
                assert_eq!(after.x() - before.x(), 64.0);
                assert_eq!(after.y() - before.y(), 32.0);
            } else {
                assert_eq!(after.x(), before.x());
                assert_eq!(after.y(), before.y());
                assert_eq!(after.width() - before.width(), 64.0);
                assert_eq!(after.height(), before.height());
            }
            assert!(!fixture.dockspace.has_active_gesture());
        }
    }
}
