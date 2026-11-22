use super::*;

#[test]
fn activating_a_partially_visible_editor_delivers_the_first_character() {
    struct Documents {
        text: [String; 2],
        long_editor: Option<egui::Response>,
    }

    impl PaneView for Documents {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Document {}", item.get()).into())
        }

        fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
            Some(egui::Id::unique(("clipped-document-editor", item)))
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            let target = self.focus_target(item).unwrap();
            let text = &mut self.text[usize::from(item == SECOND)];
            let scroll = egui::ScrollArea::vertical()
                .max_height(130.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .id(target)
                            .desired_width(160.0),
                    )
                });
            if item == SECOND {
                self.long_editor = Some(scroll.inner.clone());
            }
            // Focus registration is independent of declaring docking hover receivers.
            crate::PaneResponse::default()
        }
    }

    let long_text = "Long document\n".repeat(40);
    let third = ItemId::new(3);
    let destination = ItemId::new(4);
    for move_document in [false, true] {
        let context = Context::default();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::equal_split(
                    crate::DockspaceAxis::Horizontal,
                    [
                        DockspaceNode::tabs([FIRST]),
                        DockspaceNode::tabs([SECOND, third]),
                        DockspaceNode::tabs([destination]),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        let mut dock = Dockspace::builder("clipped-editors", layout)
            .build()
            .unwrap();
        let mut panes = Documents {
            text: ["first".into(), long_text.clone()],
            long_editor: None,
        };
        let mut time = 0.0;
        let mut paint = |dock: &mut Dockspace, panes: &mut Documents, events| {
            time += 0.1;
            let (output, _) = dock
                .run_single_surface(&context, input(time, events), SURFACE, panes)
                .unwrap();
            let (output, receipt) = output.into_parts();
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
        };
        paint(&mut dock, &mut panes, vec![]);
        let selected = dock.select_item_current(FIRST).unwrap();
        assert!(matches!(
            selected.status(),
            DockspaceActionStatus::Applied(_)
        ));
        paint(&mut dock, &mut panes, vec![]);
        assert_eq!(
            context.memory(egui::Memory::focused),
            panes.focus_target(FIRST)
        );
        let activation = if move_document {
            dock.dock_item_current(SECOND, crate::DockPlacement::After(destination))
        } else {
            for visible in [false, true] {
                let change = dock.set_item_visibility_current(SECOND, visible).unwrap();
                assert!(matches!(change.status(), DockspaceActionStatus::Applied(_)));
                paint(&mut dock, &mut panes, vec![]);
            }
            dock.select_item_current(SECOND)
        };
        assert!(matches!(
            activation.unwrap().status(),
            DockspaceActionStatus::Applied(_)
        ));
        assert!(dock.view().item(SECOND).unwrap().is_active());
        paint(&mut dock, &mut panes, vec![]);
        let editor = panes.long_editor.as_ref().unwrap();
        assert!(editor.interact_rect.is_positive());
        assert!(editor.interact_rect.height() < editor.rect.height());
        paint(&mut dock, &mut panes, vec![Event::Text("!".into())]);
        assert_eq!(panes.text[0], "first", "moved: {move_document}");
        assert_eq!(
            panes.text[1].matches('!').count(),
            1,
            "moved: {move_document}"
        );
        assert_eq!(
            context.memory(egui::Memory::focused),
            panes.focus_target(SECOND)
        );
    }
}
