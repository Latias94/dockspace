//! Application overlays must block docking without stranding the next drag.

use super::*;
use crate::{DockspaceAxis, DockspaceContainedLayout, FloatingPresentationId, LogicalRect};

struct Harness {
    context: Context,
    dockspace: Dockspace,
    panes: RunPanes,
}

impl Harness {
    fn frame(&mut self, events: Vec<Event>, modal: bool) -> FullOutput {
        let time = self.context.input(|input| input.time) + 0.1;
        let (output, _) = self
            .dockspace
            .run_single_surface_with_ui(
                &self.context,
                input(time, events),
                SURFACE,
                &mut self.panes,
                |ui| {
                    if modal {
                        egui::Modal::new(egui::Id::unique("empty-surface-docking-modal")).show(
                            ui.ctx(),
                            |ui| {
                                ui.set_min_size(egui::vec2(240.0, 160.0));
                                ui.label("Application modal");
                            },
                        );
                    }
                },
            )
            .expect("application overlays must preserve a valid owned docking run");
        let (mut output, receipt) = output.into_parts();
        output.textures_delta.clear();
        receipt.report(SurfacePresentationResult::Presented);
        output
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "actual AccessKit bounds are finite coordinates within the test viewport"
)]
fn center(output: &FullOutput, role: Role, label: &str) -> Pos2 {
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.role() == role && node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap_or_else(|| panic!("the current output must expose {role:?} {label}"));
    egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    )
}

fn release(pos: Pos2) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the regression follows one real title drag across modal coverage, release and a fresh successful drag"
)]
fn an_application_modal_blocks_empty_surface_docking_and_a_fresh_drag_recovers() {
    let context = Context::default();
    context.enable_accesskit();
    context.all_styles_mut(|style| style.animation_time = 0.0);
    let root = RootId::new(1);
    let floating = FloatingPresentationId::new(1);
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::rootless(SURFACE).with_contained(
        DockspaceContainedLayout::new(
            floating,
            DockspaceRootLayout::new(
                root,
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [
                        DockspaceNode::central_tabs([FIRST]),
                        DockspaceNode::tabs([SECOND]),
                    ],
                )
                .unwrap(),
            ),
            LogicalRect::new(40.0, 40.0, 320.0, 240.0).unwrap(),
        ),
    )])
    .unwrap();
    let dockspace = Dockspace::builder("empty-surface-docking-modal", layout)
        .build()
        .unwrap();
    let mut harness = Harness {
        context,
        dockspace,
        panes: RunPanes::default(),
    };
    let _ = harness.frame(Vec::new(), false);
    let stable = harness.frame(Vec::new(), false);
    let source = center(&stable, Role::TitleBar, "First");
    let blank = Pos2::new(90.0, 360.0);
    for events in [
        press(source),
        vec![Event::PointerMoved(source + egui::vec2(16.0, 8.0))],
        vec![Event::PointerMoved(blank)],
    ] {
        let _ = harness.frame(events, false);
    }
    assert!(harness.dockspace.has_active_gesture());
    let preview = harness.frame(Vec::new(), false);
    let guide = center(&preview, Role::Label, "Dockspace center drop guide");
    let _ = harness.frame(vec![Event::PointerMoved(guide)], false);

    // Publish the modal before its covering layer qualifies the release.
    for _ in 0..3 {
        let _ = harness.frame(Vec::new(), true);
    }
    let modal_layer = harness
        .context
        .memory(egui::Memory::top_modal_layer)
        .unwrap();
    assert_eq!(harness.context.layer_id_at(guide), Some(modal_layer));
    let _ = harness.frame(vec![release(guide)], true);
    let _ = harness.frame(Vec::new(), true);
    assert!(!harness.dockspace.has_active_gesture());
    assert!(
        harness
            .dockspace
            .view()
            .surface(SURFACE)
            .unwrap()
            .main_root()
            .is_none()
    );
    assert!(harness.dockspace.view().contained(floating).is_some());
    for item in [FIRST, SECOND] {
        let location = harness.dockspace.view().item(item).unwrap();
        assert_eq!(location.root(), root);
        assert_eq!(location.contained(), Some(floating));
    }

    // Expire the application's former layer before acquiring a fresh gesture.
    let mut stable = harness.frame(Vec::new(), false);
    for _ in 0..2 {
        stable = harness.frame(Vec::new(), false);
    }
    assert!(
        harness
            .context
            .memory(egui::Memory::top_modal_layer)
            .is_none()
    );
    let source = center(&stable, Role::TitleBar, "First");
    for events in [
        press(source),
        vec![Event::PointerMoved(source + egui::vec2(16.0, 8.0))],
        vec![Event::PointerMoved(blank)],
    ] {
        let _ = harness.frame(events, false);
    }
    assert!(harness.dockspace.has_active_gesture());
    let preview = harness.frame(Vec::new(), false);
    let guide = center(&preview, Role::Label, "Dockspace center drop guide");
    for events in [
        vec![Event::PointerMoved(guide)],
        vec![release(guide)],
        Vec::new(),
    ] {
        let _ = harness.frame(events, false);
    }
    assert!(!harness.dockspace.has_active_gesture());
    assert!(harness.dockspace.view().contained(floating).is_none());
    let main = harness
        .dockspace
        .view()
        .surface(SURFACE)
        .unwrap()
        .main_root()
        .unwrap();
    assert_eq!(main.id(), root);
    let split = main.content().unwrap().split().unwrap();
    assert_eq!(split.axis(), DockspaceAxis::Horizontal);
    assert_eq!(
        split
            .children()
            .map(|node| node.tabs().unwrap().items().collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [vec![FIRST], vec![SECOND]],
        "recovering input must dock the complete root without merging its tab groups"
    );
}
