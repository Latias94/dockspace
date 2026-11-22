use super::*;
use crate::{PaneResponse, Tab, TabBar, TabBarId, TabBarLayout, TabBarSlot, TabClose, TabVisuals};

#[test]
fn a_neighbors_close_button_does_not_interrupt_custom_header_editing() {
    #[derive(Default)]
    struct Headers {
        text: String,
        first_tab: Option<Pos2>,
        editor: Option<egui::Response>,
        first_close_drawn: bool,
    }

    impl PaneView for Headers {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Tab {}", item.get()).into())
        }

        fn ui(&mut self, _: ItemId, _: &mut Ui) -> PaneResponse {
            PaneResponse::default()
        }

        fn tab_content_width(&self, _: crate::TabKey, _: &Ui, _: &egui::Galley) -> f32 {
            124.0
        }

        fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
            if tab.item() == Some(FIRST) {
                self.first_tab = Some(tab.content_rect.center());
                return tab.paint(ui);
            }
            self.editor =
                Some(ui.add(egui::TextEdit::singleline(&mut self.text).desired_width(70.0)));
            tab.visuals
        }

        fn tab_close_ui(&mut self, ui: &Ui, close: &TabClose<'_>) {
            self.first_close_drawn |= close.item() == Some(FIRST);
            close.paint(ui);
        }
    }

    let (context, mut dock, _, _) = presented_tabs("editable-tab-header");
    let selected = dock.select_item_current(SECOND).unwrap();
    assert!(matches!(
        selected.status(),
        DockspaceActionStatus::Applied(_)
    ));
    let mut headers = Headers::default();
    let mut time = 1.0;
    let mut paint = |headers: &mut Headers, events| {
        time += 0.1;
        headers.first_close_drawn = false;
        let (output, _) = dock
            .run_single_surface(&context, input(time, events), SURFACE, headers)
            .unwrap();
        let (output, receipt) = output.into_parts();
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
    };
    paint(&mut headers, vec![]);
    paint(&mut headers, vec![]);
    let editor = headers.editor.as_ref().unwrap().clone();
    let pos = editor.interact_rect.center();
    paint(&mut headers, vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        paint(
            &mut headers,
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    paint(&mut headers, vec![Event::Text("x".into())]);
    assert_eq!(headers.text, "x");
    assert!(!headers.first_close_drawn);
    let neighbor = headers.first_tab.unwrap();
    paint(&mut headers, vec![Event::PointerMoved(neighbor)]);
    assert!(headers.first_close_drawn);
    paint(&mut headers, vec![Event::Text("y".into())]);
    assert_eq!(headers.text, "xy");
    assert_eq!(headers.editor.unwrap().id, editor.id);
}

#[test]
fn custom_bar_slots_keep_same_group_focus_and_drop_replaced_groups_undo_history() {
    struct Fields {
        leading: bool,
        old: String,
        replacement: String,
        trailing: Option<egui::Response>,
    }

    impl PaneView for Fields {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Document {}", item.get()).into())
        }

        fn ui(&mut self, _: ItemId, _: &mut Ui) -> PaneResponse {
            PaneResponse::default()
        }

        fn tab_bar_layout(&self, _: TabBarId, _: &Ui) -> TabBarLayout {
            TabBarLayout {
                leading_width: if self.leading { 76.0 } else { 0.0 },
                trailing_width: 120.0,
                ..Default::default()
            }
        }

        fn tab_bar_ui(&mut self, ui: &mut Ui, bar: TabBar<'_>, slot: TabBarSlot) {
            match slot {
                TabBarSlot::Leading => {
                    ui.label("Search");
                }
                TabBarSlot::Trailing => {
                    let text = if bar.items().any(|item| item == FIRST) {
                        &mut self.old
                    } else {
                        &mut self.replacement
                    };
                    self.trailing =
                        Some(ui.add(egui::TextEdit::singleline(text).desired_width(90.0)));
                }
                TabBarSlot::AfterTabs => {}
            }
        }
    }

    let context = Context::default();
    let layout = |items| {
        DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(RootId::new(1), DockspaceNode::central_tabs(items)),
        )])
        .unwrap()
    };
    let mut dock = Dockspace::builder("tab-slot-history", layout([FIRST, SECOND]))
        .build()
        .unwrap();
    let mut fields = Fields {
        leading: true,
        old: String::new(),
        replacement: String::new(),
        trailing: None,
    };
    let mut time = 0.0;
    let mut paint = |dock: &mut Dockspace, fields: &mut Fields, events| {
        time += 0.1;
        let (output, _) = dock
            .run_single_surface(&context, input(time, events), SURFACE, fields)
            .unwrap();
        let (output, receipt) = output.into_parts();
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
    };
    paint(&mut dock, &mut fields, vec![]);
    let old_id = fields.trailing.as_ref().unwrap().id;
    context.memory_mut(|memory| memory.request_focus(old_id));
    paint(&mut dock, &mut fields, vec![Event::Text("OLD".into())]);
    for _ in 0..16 {
        paint(&mut dock, &mut fields, vec![]);
    }
    assert_eq!(fields.old, "OLD");
    fields.leading = false;
    paint(&mut dock, &mut fields, vec![]);
    assert_eq!(fields.trailing.as_ref().unwrap().id, old_id);
    assert_eq!(context.memory(egui::Memory::focused), Some(old_id));

    dock.reset_layout(layout([ItemId::new(3), ItemId::new(4)]))
        .unwrap();
    paint(&mut dock, &mut fields, vec![]);
    let replacement_id = fields.trailing.as_ref().unwrap().id;
    assert_ne!(replacement_id, old_id);
    context.memory_mut(|memory| memory.request_focus(replacement_id));
    paint(&mut dock, &mut fields, vec![Event::Text("K".into())]);
    assert_eq!(fields.replacement, "K");
    paint(
        &mut dock,
        &mut fields,
        vec![Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    assert_eq!(fields.replacement, "");
    assert_eq!(fields.old, "OLD");
}
