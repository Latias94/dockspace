use super::*;
use crate::{PaneResponse, Tab, TabClose, TabVisuals};

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
