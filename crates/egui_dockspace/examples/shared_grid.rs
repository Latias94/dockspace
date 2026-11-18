//! Real shared grid tracks with bare content and a named nested layout tab.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use egui::{Color32, Galley, Painter, Rect, Ui, WidgetText, vec2};
use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, Dockspace, DockspaceAxis, DockspaceGridShape,
    DockspaceLayout, DockspaceNode, DockspaceOutput, DockspaceResponse, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, Splitter, SplitterKind,
    SurfaceId, Tab, TabKey, TabVisuals,
    winit::{
        AppResult, FrameControl, NativeApp, NativeFrameReport, NativeOptions, backend, run_native,
    },
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const LAYOUT_ICON_SPACE: f32 = 16.0;
const GRID_ACCENT: Color32 = Color32::from_rgb(80, 139, 196);
const ITEMS: [ItemId; 7] = [
    ItemId::new(1),
    ItemId::new(2),
    ItemId::new(3),
    ItemId::new(4),
    ItemId::new(5),
    ItemId::new(6),
    ItemId::new(7),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = NativeOptions {
        window: backend::window::WindowAttributes::default()
            .with_title("Shared grid dockspace")
            .with_inner_size(backend::dpi::LogicalSize::new(1100.0, 760.0)),
        ..Default::default()
    };
    run_native(options, egui::Context::default(), &mut Example::new())?;
    Ok(())
}

fn layout() -> DockspaceLayout {
    let [a, b, c, d, e, f, g] = ITEMS;
    let tools = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::pane(f), DockspaceNode::pane(g)],
    )
    .expect("the tools layout has two real children")
    .named("Tools");
    let grid = DockspaceNode::grid(
        DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(2).expect("two rows are nonzero"),
            columns: NonZeroUsize::new(2).expect("two columns are nonzero"),
        },
        [
            DockspaceNode::pane(a),
            DockspaceNode::tabs([b, c]).named("Documents"),
            DockspaceNode::pane(d),
            DockspaceNode::composed_tabs([DockspaceNode::pane(e), tools]),
        ],
    )
    .expect("four row-major cells fill the two-by-two grid")
    .named("Workspace");
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), grid),
    )])
    .expect("each application item appears once")
}

struct Example {
    dockspace: Dockspace,
    panes: Documents,
    response: Option<DockspaceResponse>,
    reset: bool,
}

impl Example {
    fn new() -> Self {
        Self {
            dockspace: Dockspace::builder("shared-grid-example", layout())
                .build()
                .expect("the static grid layout is valid"),
            panes: Documents {
                text: ITEMS
                    .map(|item| (item, format!("Editable content for {}", title(item))))
                    .into_iter()
                    .collect(),
            },
            response: None,
            reset: false,
        }
    }
}

impl NativeApp for Example {
    fn update(
        &mut self,
        context: &egui::Context,
        input: egui::RawInput,
    ) -> AppResult<DockspaceOutput> {
        self.reset = false;
        if self.dockspace.view().surface(SURFACE).is_none() {
            self.response = None;
            return Ok(egui_dockspace::run_ui(context, input, |ui| {
                toolbar(ui, &mut self.reset)
            }));
        }
        let (output, response) = self.dockspace.run_single_surface_with_ui(
            context,
            input,
            SURFACE,
            &mut self.panes,
            |ui| toolbar(ui, &mut self.reset),
        )?;
        self.response = Some(response);
        Ok(output)
    }

    fn frame_finished(&mut self, _: NativeFrameReport<'_>) -> AppResult<FrameControl> {
        if self.reset {
            self.dockspace.reset_layout(layout())?;
            return Ok(FrameControl::Repaint);
        }
        let mut changed = false;
        for request in self
            .response
            .as_ref()
            .into_iter()
            .flat_map(DockspaceResponse::close_request_events)
            .filter(|request| !request.reused())
        {
            let plan = request.plan();
            for item in plan
                .items()
                .iter()
                .filter(|item| item.state() == CloseItemDecisionState::Pending)
            {
                changed |= self
                    .dockspace
                    .resolve_close(plan.request(), item.token(), CloseDecision::Allow)?
                    .mutation()
                    .workspace_changed();
            }
        }
        Ok(if changed {
            FrameControl::Repaint
        } else {
            FrameControl::Continue
        })
    }
}

fn toolbar(ui: &mut egui::Ui, reset: &mut bool) {
    egui::Panel::top("grid-help").show(ui, |ui| {
        ui.label("Resize a column across both rows, a row across both columns, or their shared intersection.");
        ui.label("Use Layout actions > Active Panes for bare content. Tool pair opens two independent panes.");
        *reset |= ui.button("Reset layout").clicked();
    });
}

struct Documents {
    text: BTreeMap<ItemId, String>,
}

impl PaneView for Documents {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        self.text.contains_key(&item).then(|| title(item).into())
    }

    fn layout_tab_title(&self, _key: TabKey, default_title: &str) -> WidgetText {
        if default_title == "Tools" {
            "Tool pair".into()
        } else {
            default_title.into()
        }
    }

    fn tab_content_width(&self, key: TabKey, _ui: &Ui, title: &Galley) -> f32 {
        title.size().x
            + if key.is_layout() {
                LAYOUT_ICON_SPACE
            } else {
                0.0
            }
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        if !tab.key.is_layout() {
            return tab.paint(ui);
        }
        tab.paint_background(ui.painter());
        let painter = ui.painter_at(tab.content_clip_rect);
        let icon_origin = egui::pos2(tab.content_rect.left(), tab.content_rect.center().y - 4.0);
        for x in [0.0, 6.0] {
            painter.rect_filled(
                Rect::from_min_size(icon_origin + vec2(x, 0.0), vec2(4.0, 8.0)),
                1.0,
                GRID_ACCENT,
            );
        }
        let title_rect = Rect::from_min_max(
            tab.content_rect.min + vec2(LAYOUT_ICON_SPACE, 0.0),
            tab.content_rect.max,
        );
        tab.paint_title(ui, title_rect);
        tab.visuals
    }

    fn splitter_ui(&mut self, painter: &Painter, splitter: &Splitter) {
        let emphasized =
            splitter.dragging || splitter.focused || (splitter.drag_enabled && splitter.hovered);
        match splitter.kind {
            SplitterKind::Divider(_) if emphasized => {
                painter.rect_filled(splitter.rect, 0.0, GRID_ACCENT);
            }
            SplitterKind::Divider(_) => splitter.paint(painter),
            SplitterKind::Junction => {
                let color = if splitter.enabled {
                    GRID_ACCENT
                } else {
                    Color32::GRAY
                };
                let radius = if emphasized { 5.0 } else { 3.0 };
                painter.circle_filled(splitter.rect.center(), radius, color);
            }
        }
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        ui.heading(title(item));
        let response = ui.add(
            egui::TextEdit::multiline(self.text.get_mut(&item).expect("each pane has content"))
                .id(editor(item))
                .desired_width(f32::INFINITY),
        );
        PaneResponse::from(&response)
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(editor(item))
    }
}

fn editor(item: ItemId) -> egui::Id {
    egui::Id::unique(("shared-grid-editor", item))
}

fn title(item: ItemId) -> &'static str {
    match item.get() {
        1 => "Bare A",
        2 => "Document B",
        3 => "Document C",
        4 => "Bare D",
        5 => "Document E",
        6 => "Tool F",
        7 => "Tool G",
        _ => "Document",
    }
}
