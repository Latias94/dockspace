use dockspace::{
    model::{
        DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
        DockspaceSurfaceLayout, FloatingPresentationId, RootId,
    },
    policy::DockPolicy,
    runtime::{DockspaceSession, SurfacePresentationResult},
};

use super::*;

#[test]
fn clipped_pane_text_is_a_crop_of_the_same_unclipped_layout() {
    let surface = SurfaceId::new(1);
    let item = ItemId::new(5);
    let mut images = Vec::new();
    for top in [80.0, -80.0] {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            surface,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::central_tabs([ItemId::new(1)]),
            ),
        )
        .with_contained(DockspaceContainedLayout::new(
            FloatingPresentationId::new(1),
            DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([item])),
            LogicalRect::new(100.0, top, 260.0, 240.0).unwrap(),
        ))])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        frame.begin_encoding_pass().unwrap();
        let bounds = LogicalRect::new(0.0, 0.0, 640.0, 480.0).unwrap();
        measure(&mut frame, surface, bounds).unwrap();
        let mut pixels = vec![BACKGROUND; 640 * 480];
        let mut paint = Pixels {
            pixels: &mut pixels,
            size: [640, 480],
            scale: 1.0,
            clip: rect(bounds),
        };
        let receipt = frame
            .paint_plan(surface)
            .unwrap()
            .unwrap()
            .execute_surface_program(|step| {
                if let Step::Operation {
                    operation: Operation::Pane(pane),
                    ..
                } = step
                    && pane.item() == item
                {
                    if top < 0.0 {
                        assert!(pane.content_geometry().layout_bounds().y() < 0.0);
                        assert_eq!(pane.content_geometry().clip().y(), 0.0);
                    } else {
                        let geometry = pane.content_geometry();
                        assert_eq!(geometry.layout_bounds(), geometry.clip());
                    }
                    paint_pane(&mut paint, pane);
                }
                Ok::<(), Infallible>(())
            })
            .unwrap();
        frame.confirm_surface_painted(receipt).unwrap();
        let mut report = frame.commit().unwrap();
        for output in report.take_painted_outputs() {
            session
                .report_surface_presentation(output, SurfacePresentationResult::Dropped)
                .unwrap();
        }
        images.push(pixels);
    }
    for y in 0..159 {
        for x in 101..359 {
            assert_eq!(
                images[1][y * 640 + x],
                images[0][(y + 160) * 640 + x],
                "pane content drifted at ({x}, {y})"
            );
        }
    }
}
