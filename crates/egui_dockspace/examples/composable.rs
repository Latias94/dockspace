//! Named layout tabs with nested tabs and real content without implicit chrome.

use std::collections::BTreeMap;

use eframe::egui;
use egui_dockspace::{
    CloseDecision, CloseItemDecisionState, Dockspace, DockspaceActionStatus, DockspaceAxis,
    DockspaceCloseOutcome, DockspaceCloseResolution, DockspaceLayout, DockspaceNode,
    DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);

fn main() -> eframe::Result {
    eframe::run_native(
        "Composable dockspace",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 680.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(Example::new()))),
    )
}

struct Example {
    dockspace: Dockspace,
    panes: Documents,
    status: Option<String>,
}

impl Example {
    fn new() -> Self {
        let tools = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [DockspaceNode::tabs([A, B]), DockspaceNode::pane(D)],
        )
        .expect("the tools layout has two children")
        .named("Tools");
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::composed_tabs([DockspaceNode::pane(C), tools]),
            ),
        )])
        .expect("each application item appears once");
        Self {
            dockspace: Dockspace::builder("composable-example", layout)
                .build()
                .expect("the static composed layout is valid"),
            panes: Documents {
                text: [A, B, C, D]
                    .map(|item| (item, format!("Editable content for {}", title(item))))
                    .into_iter()
                    .collect(),
            },
            status: None,
        }
    }
}

impl eframe::App for Example {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::Panel::top("example-help").show(ui, |ui| {
            ui.label("Open Tools to show its nested tab group and Bare D side by side.");
            ui.label("Tools keeps its own title when you switch between Editor A and Editor B.");
            ui.horizontal(|ui| {
                ui.label("Activate content:");
                for item in [C, A, B, D] {
                    if ui
                        .add_enabled(
                            self.dockspace.view().item(item).is_some(),
                            egui::Button::new(title(item)),
                        )
                        .clicked()
                    {
                        let activated = self
                            .dockspace
                            .select_item_current(item)
                            .expect("the example content remains owned");
                        self.status = match activated.status() {
                            DockspaceActionStatus::Applied(_) => None,
                            status => {
                                Some(format!("Could not activate {}: {status:?}", title(item)))
                            }
                        };
                        ui.ctx().request_repaint();
                    }
                }
            });
            if let Some(status) = &self.status {
                ui.label(status);
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.dockspace.view().surface(SURFACE).is_none() {
                ui.label("All content is closed.");
                return;
            }
            let response = self
                .dockspace
                .show_single_surface(SURFACE, ui, &mut self.panes)
                .expect("the example surface is live");
            for request in response
                .close_request_events()
                .iter()
                .filter(|request| !request.reused())
            {
                let plan = request.plan();
                for item in plan
                    .items()
                    .iter()
                    .filter(|item| item.state() == CloseItemDecisionState::Pending)
                {
                    let resolved = self
                        .dockspace
                        .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
                        .expect("the example content has no pending save");
                    self.status = match resolved.outcome() {
                        DockspaceCloseOutcome::Processed {
                            resolution:
                                DockspaceCloseResolution::Recorded { .. }
                                | DockspaceCloseResolution::Approved { .. },
                            application: None | Some(Ok(_)),
                            ..
                        } => None,
                        outcome => Some(format!(
                            "Could not close {}: {outcome:?}",
                            title(item.item())
                        )),
                    };
                    ui.ctx().request_repaint();
                }
            }
        });
    }
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
        let response = egui_dockspace::pane_text_edit(
            ui,
            editor(item),
            egui::TextEdit::multiline(self.text.get_mut(&item).expect("each pane has content"))
                .desired_width(f32::INFINITY),
        );
        PaneResponse::from(&response)
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        self.focus_target(item).is_some()
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(editor(item))
    }
}

fn editor(item: ItemId) -> egui::Id {
    egui::Id::unique(("composable-example-editor", item))
}

fn title(item: ItemId) -> &'static str {
    match item {
        A => "Editor A",
        B => "Editor B",
        C => "Document C",
        D => "Bare D",
        _ => "Document",
    }
}
