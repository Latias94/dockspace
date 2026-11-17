//! Descriptive OS captions derived from the committed product view.

use dockspace::model::{DockspaceView, SurfaceId};
use egui_dockspace::PaneView;

pub(crate) fn window_title(
    view: DockspaceView<'_>,
    panes: &impl PaneView,
    surface: SurfaceId,
) -> String {
    let mut title = String::new();
    if let Some(surface) = view.surface(surface) {
        for item in surface.active_items() {
            if !title.is_empty() {
                title.push_str(" · ");
            }
            if let Some(caption) = panes
                .title(item)
                .filter(|caption| !caption.text().trim().is_empty())
            {
                title.push_str(caption.text().trim());
            } else {
                use std::fmt::Write as _;
                let _ = write!(title, "Pane {}", item.get());
            }
        }
    }
    if title.is_empty() {
        format!("Panel {}", surface.get())
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dockspace::model::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
        ItemId, RootId,
    };
    use dockspace::policy::DockPolicy;
    use dockspace::runtime::{DockspaceSession, SurfaceUnavailableReason};

    const SURFACE: SurfaceId = SurfaceId::new(2);
    const FIRST: ItemId = ItemId::new(1);
    const SECOND: ItemId = ItemId::new(2);

    struct Panes;

    impl PaneView for Panes {
        fn title(&self, item: ItemId) -> Option<eframe::egui::WidgetText> {
            match item {
                FIRST => Some("Inspector".into()),
                SECOND => Some("预览".into()),
                _ => None,
            }
        }

        fn ui(&mut self, _: ItemId, _: &mut eframe::egui::Ui) -> egui_dockspace::PaneResponse {
            egui_dockspace::PaneResponse::default()
        }
    }

    fn session(content: DockspaceNode) -> DockspaceSession {
        DockspaceSession::from_layout(
            DockspaceLayout::new([DockspaceSurfaceLayout::new(
                SURFACE,
                DockspaceRootLayout::new(RootId::new(2), content),
            )])
            .unwrap(),
            DockPolicy::default(),
        )
        .unwrap()
    }

    #[test]
    fn selecting_another_pane_changes_the_caption_without_replacing_its_surface() {
        let mut session = session(DockspaceNode::tabs([FIRST, SECOND]));
        assert_eq!(window_title(session.view(), &Panes, SURFACE), "Inspector");
        let action = session.prepare_select_item(SECOND);
        let mut frame = session.begin_host_frame().unwrap();
        frame.submit_prepared_action(action).unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        frame.commit().unwrap();
        assert_eq!(window_title(session.view(), &Panes, SURFACE), "预览");
        assert_eq!(session.view().surface(SURFACE).unwrap().id(), SURFACE);
    }

    #[test]
    fn composed_caption_includes_active_bare_content_and_omits_inactive_descendants() {
        let bare = ItemId::new(9);
        let alternate = ItemId::new(10);
        let tools = DockspaceNode::equal_split(
            DockspaceAxis::Horizontal,
            [
                DockspaceNode::tabs([FIRST, SECOND]),
                DockspaceNode::pane(bare),
            ],
        )
        .unwrap()
        .named("Tools");
        let mut session = session(DockspaceNode::composed_tabs([
            DockspaceNode::pane(alternate),
            tools,
        ]));
        assert_eq!(window_title(session.view(), &Panes, SURFACE), "Pane 10");

        let mut frame = session.begin_host_frame().unwrap();
        frame.select_item_current(bare).unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        frame.commit().unwrap();
        assert_eq!(
            window_title(session.view(), &Panes, SURFACE),
            "Inspector · Pane 9"
        );

        let mut frame = session.begin_host_frame().unwrap();
        frame.select_item_current(alternate).unwrap();
        frame
            .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
            .unwrap();
        frame.commit().unwrap();
        assert_eq!(window_title(session.view(), &Panes, SURFACE), "Pane 10");
    }

    #[test]
    fn split_captions_include_selected_leaves_and_missing_content_has_a_fallback() {
        let session = session(
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [
                    DockspaceNode::tabs([FIRST, SECOND]),
                    DockspaceNode::tabs([ItemId::new(9)]),
                ],
            )
            .unwrap(),
        );
        assert_eq!(
            window_title(session.view(), &Panes, SURFACE),
            "Inspector · Pane 9"
        );
        assert_eq!(
            window_title(session.view(), &Panes, SurfaceId::new(8)),
            "Panel 8"
        );
    }

    #[test]
    fn hidden_leaves_leave_the_caption_and_restore_without_replacing_identity() {
        let mut session = session(
            DockspaceNode::equal_split(
                DockspaceAxis::Horizontal,
                [DockspaceNode::tabs([FIRST]), DockspaceNode::tabs([SECOND])],
            )
            .unwrap(),
        );
        let root = session.view().item(FIRST).unwrap().root();
        assert_eq!(
            window_title(session.view(), &Panes, SURFACE),
            "Inspector · 预览"
        );

        for (item, visible, expected) in [
            (FIRST, false, "预览"),
            (SECOND, false, "Panel 2"),
            (FIRST, true, "Inspector"),
            (SECOND, true, "Inspector · 预览"),
        ] {
            let mut frame = session.begin_host_frame().unwrap();
            frame.set_item_visibility_current(item, visible).unwrap();
            frame
                .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
                .unwrap();
            frame.commit().unwrap();

            assert_eq!(window_title(session.view(), &Panes, SURFACE), expected);
            let surface = session.view().surface(SURFACE).unwrap();
            assert_eq!(surface.id(), SURFACE);
            assert_eq!(surface.main_root().unwrap().id(), root);
            for item in [FIRST, SECOND] {
                let pane = session.view().item(item).unwrap();
                assert_eq!(pane.surface(), SURFACE);
                assert_eq!(pane.root(), root);
            }
        }
    }

    #[test]
    fn deep_window_caption_preserves_leaf_order_without_call_stack_traversal() {
        const TEST: &str = "window_title::tests::deep_window_caption_preserves_leaf_order_without_call_stack_traversal";
        const CHILD: &str = "DOCKSPACE_DEEP_CAPTION_SUBPROCESS";
        if std::env::var(CHILD).as_deref() != Ok(TEST) {
            // A recursive traversal can abort the process on stack overflow.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, TEST)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{TEST} failed: {}\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        let mut node = DockspaceNode::central_tabs([FIRST]);
        let mut expected = "Inspector".to_owned();
        for depth in 0_u64..10_000 {
            let axis = if depth.is_multiple_of(2) {
                DockspaceAxis::Horizontal
            } else {
                DockspaceAxis::Vertical
            };
            let item = depth + 3;
            node =
                DockspaceNode::equal_split(axis, [node, DockspaceNode::tabs([ItemId::new(item)])])
                    .unwrap();
            use std::fmt::Write as _;
            write!(expected, " · Pane {item}").unwrap();
        }
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(2), node),
        )])
        .unwrap();
        assert_eq!(window_title(layout.view(), &Panes, SURFACE), expected);
    }
}
