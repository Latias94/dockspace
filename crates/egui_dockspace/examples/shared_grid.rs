//! Real shared grid tracks with bare content and a named nested layout tab.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, Dockspace, DockspaceAxis, DockspaceGridShape,
    DockspaceLayout, DockspaceNode, DockspaceOutput, DockspaceResponse, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
    winit::{
        AppResult, FrameControl, NativeApp, NativeFrameReport, NativeOptions, backend, run_native,
    },
};

const SURFACE: SurfaceId = SurfaceId::new(1);
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
        ui.label("Use Layout actions > Active Panes for bare content. Tools is one layout tab with two independent panes.");
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
