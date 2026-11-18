//! Mapped surface changes preserve ordinary egui content identities and retained scroll state.

use super::*;
use crate::host_frame::NativeHostFrame;
use crate::mailbox::NativeHostBridge;
use crate::receiver::NativeReceiverStore;
use crate::viewport_map::NativeViewportMap;
use dockspace::geometry::{LogicalSize, PhysicalPoint, PhysicalRect, ScaleFactor};

const MOVED: ItemId = ItemId::new(1);
const ANCHOR: ItemId = ItemId::new(2);
const TARGET: ItemId = ItemId::new(3);
const OTHER: SurfaceId = SurfaceId::new(2);

#[derive(Default)]
struct ContentPanes {
    text: String,
    initialize_scroll: bool,
    observed: Option<(Id, Id, f32)>,
}

impl PaneView for ContentPanes {
    fn title(&self, _: ItemId) -> Option<WidgetText> {
        Some("Document".into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> egui_dockspace::PaneResponse {
        if item != MOVED {
            return Default::default();
        }
        let edit = ui.text_edit_singleline(&mut self.text);
        let mut scroll = egui::ScrollArea::vertical()
            .animated(false)
            .max_height(60.0);
        if self.initialize_scroll {
            scroll = scroll.vertical_scroll_offset(120.0);
        }
        let scroll = scroll.show(ui, |ui| {
            for index in 0..50 {
                ui.label(format!("line {index}"));
            }
        });
        self.observed = Some((edit.id, scroll.id, scroll.state.clamped_offset().y));
        Default::default()
    }
}

fn paint(
    session: &mut dockspace::runtime::DockspaceSession,
    context: &egui::Context,
    scope: NativePaintScope,
    surface: SurfaceId,
    viewport: ViewportId,
    panes: &mut ContentPanes,
) {
    let mut raw = egui::RawInput {
        viewport_id: viewport,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 480.0),
        )),
        ..Default::default()
    };
    if viewport != ViewportId::ROOT {
        raw.viewports.entry(viewport).or_default().parent = Some(ViewportId::ROOT);
    }
    let bridge = Arc::new(NativeHostBridge::new(NativeViewportMap::default()));
    let mut receivers = NativeReceiverStore::default();
    let mut frame = NativeHostFrame::new(
        session.begin_host_frame().unwrap(),
        bridge,
        &mut receivers,
        None,
    );
    let mut paint = None;
    let mut output = context.run_ui(raw, |ui| {
        let placement = NativeSurfacePlacement::new(
            surface,
            LogicalSize::new(600.0, 400.0).unwrap(),
            PhysicalPoint::new(12.0, 18.0).unwrap(),
            ScaleFactor::new(1.0).unwrap(),
            PhysicalRect::new(12.0, 18.0, 600.0, 400.0).unwrap(),
        );
        let mapping = NativePaintMapping::new(placement, 1.0, ui.clip_rect()).unwrap();
        let mut child = scope.child_ui(mapping, ui, surface);
        frame
            .measure_surface(
                surface,
                &child,
                child.max_rect(),
                child.clip_rect(),
                panes,
                &DockStyle::default(),
            )
            .unwrap();
        paint = Some(
            scope
                .paint_surface(
                    &mut frame,
                    surface,
                    &mut child,
                    panes,
                    &DockStyle::default(),
                )
                .unwrap(),
        );
    });
    output.textures_delta.clear();
    let mut paint = paint.unwrap();
    assert!(paint.had_ready_plan());
    assert!(paint.validate_final_output(&mut output));
    assert!(paint.confirm_into(&mut frame.frame).unwrap());
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let outputs = frame.commit().unwrap().take_painted_outputs();
    session
        .report_surface_presentations(outputs, SurfacePresentationResult::Presented)
        .unwrap();
    output.drop_without_applying_deltas();
}

#[test]
fn mapped_content_keeps_widget_ids_and_scroll_when_docking_across_surfaces() {
    let context = egui::Context::default();
    let scope = test_state().paint_scope;
    let style = DockStyle::default();
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), DockspaceNode::central_tabs([MOVED, ANCHOR])),
        ),
        DockspaceSurfaceLayout::new(
            OTHER,
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::central_tabs([TARGET])),
        ),
    ])
    .unwrap();
    let mut session = dockspace::runtime::DockspaceSession::from_layout_with_presentation_config(
        layout,
        Default::default(),
        style.presentation_config().unwrap(),
    )
    .unwrap();
    let mut panes = ContentPanes {
        text: "workspace draft".into(),
        initialize_scroll: true,
        ..Default::default()
    };
    for _ in 0..3 {
        paint(
            &mut session,
            &context,
            scope,
            SURFACE,
            ViewportId::ROOT,
            &mut panes,
        );
    }
    let before = panes.observed.unwrap();
    assert!((before.2 - 120.0).abs() < 1.0);
    panes.initialize_scroll = false;
    let child = ViewportId::from_hash_of("content-migration-child");
    for (target, surface, viewport) in [(TARGET, OTHER, child), (ANCHOR, SURFACE, ViewportId::ROOT)]
    {
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .dock_item_current(MOVED, DockPlacement::After(target))
            .unwrap();
        frame.select_item_current(MOVED).unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        frame.commit().unwrap();
        for _ in 0..3 {
            paint(&mut session, &context, scope, surface, viewport, &mut panes);
        }
        let after = panes.observed.unwrap();
        assert_eq!((after.0, after.1), (before.0, before.1));
        assert!(
            (after.2 - before.2).abs() < 1.0,
            "scroll must follow the item to {surface:?}"
        );
        assert_eq!(panes.text, "workspace draft");
    }
}
