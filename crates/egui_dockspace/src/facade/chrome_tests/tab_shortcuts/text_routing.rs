use super::*;

struct Editors {
    text: [String; 2],
    consume_cycle: bool,
}

impl PaneView for Editors {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(format!("Document {}", item.get()).into())
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(egui::Id::unique(("tab-routing-editor", item)))
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        let id = self.focus_target(item).unwrap();
        if self.consume_cycle && ui.memory(|memory| memory.has_focus(id)) {
            ui.input_mut(|input| input.consume_shortcut(&bindings().next_tab.unwrap()));
        }
        let response = ui.add(
            egui::TextEdit::multiline(&mut self.text[usize::try_from(item.get()).unwrap() - 1])
                .id(id),
        );
        PaneResponse::from(&response)
    }
}

struct Fixture {
    harness: Harness,
    editors: Editors,
    entry: Entry,
}

#[derive(Clone, Copy, Debug)]
enum Entry {
    Owned,
    Composed,
    Callback,
}

const ENTRIES: [Entry; 3] = [Entry::Owned, Entry::Composed, Entry::Callback];

impl Fixture {
    fn new(entry: Entry) -> Self {
        let mut harness = Harness::new(
            ["Document 1".into(), "Document 2".into()].into(),
            ItemId::new(1),
            DockStyle::default(),
        );
        harness.dockspace.set_input(bindings());
        let mut fixture = Self {
            harness,
            editors: Editors {
                text: Default::default(),
                consume_cycle: false,
            },
            entry,
        };
        fixture.settle();
        fixture
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        let harness = &mut self.harness;
        harness.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(f64::from(harness.frame) * 0.1),
            events,
            ..Default::default()
        };
        let paint = |ui: &mut Ui| {
            harness
                .dockspace
                .show_single_surface(SURFACE, ui, &mut self.editors)
                .unwrap();
        };
        let output = match self.entry {
            Entry::Owned => {
                harness
                    .dockspace
                    .run_single_surface(&harness.context, raw, SURFACE, &mut self.editors)
                    .unwrap()
                    .0
            }
            Entry::Composed => crate::run_ui(&harness.context, raw, paint),
            Entry::Callback => {
                let mut output = harness.context.run_ui(raw, paint);
                output.textures_delta.clear();
                return output;
            }
        };
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self) -> FullOutput {
        self.run(Vec::new());
        self.run(Vec::new());
        self.run(Vec::new())
    }

    fn focus(&mut self, id: egui::Id) {
        self.harness
            .context
            .memory_mut(|memory| memory.request_focus(id));
        self.run(Vec::new());
        assert_eq!(self.harness.context.memory(egui::Memory::focused), Some(id));
    }

    fn focus_header(&mut self, label: &str) {
        let output = self.run(Vec::new());
        let (header, _) = nodes(&output)
            .iter()
            .find(|(_, node)| node.role() == Role::Tab && node.label() == Some(label))
            .unwrap();
        self.run(vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Focus,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: *header,
                data: None,
            },
        )]);
        assert_eq!(
            self.harness
                .context
                .memory(egui::Memory::focused)
                .map(|id| id.accesskit_id()),
            Some(*header),
        );
    }
}

#[test]
fn cycling_does_not_write_trailing_input_into_the_old_document() {
    for entry in ENTRIES {
        for batched in [false, true] {
            let mut fixture = Fixture::new(entry);
            fixture.focus(fixture.editors.focus_target(ItemId::new(1)).unwrap());
            let mut events = vec![
                Event::Text("before".into()),
                shortcut(bindings().next_tab.unwrap()),
            ];
            let after = [key(Key::Backspace), Event::Text("after".into())];
            if batched {
                events.extend(after.clone());
            }
            fixture.run(events);
            if !batched {
                fixture.run(after.into());
            }
            fixture.settle();
            assert_selected(&fixture.harness, 2);
            assert_eq!(
                fixture.editors.text,
                ["before", ""],
                "{entry:?}, batched={batched}"
            );
        }
    }
}

#[test]
fn header_activation_delivers_the_first_text_to_the_new_document_once() {
    for entry in ENTRIES {
        for batched in [false, true] {
            for activation in [Key::Enter, Key::Space] {
                let mut fixture = Fixture::new(entry);
                fixture.focus_header("Document 2");
                let mut events = vec![key(activation)];
                if activation == Key::Space {
                    events.push(Event::Text(" ".into()));
                }
                let edits = [
                    Event::Text("first".into()),
                    key(Key::ArrowLeft),
                    Event::Text("-".into()),
                ];
                if batched {
                    events.extend(edits.clone());
                }
                fixture.run(events);
                if !batched {
                    fixture.run(edits.into());
                }
                fixture.settle();
                assert_selected(&fixture.harness, 2);
                assert_eq!(
                    fixture.editors.text,
                    ["", "firs-t"],
                    "{entry:?}, batched={batched}, key={activation:?}"
                );
            }
        }
    }
}

#[test]
fn an_activation_followed_by_another_cycle_routes_the_replayed_suffix_in_order() {
    for entry in ENTRIES {
        for batched in [false, true] {
            let mut fixture = Fixture::new(entry);
            fixture.focus_header("Document 2");
            let events = [
                key(Key::Enter),
                Event::Text("first".into()),
                shortcut(bindings().next_tab.unwrap()),
                Event::Text("after".into()),
            ];
            if batched {
                fixture.run(events.into());
            } else {
                for event in events {
                    fixture.run(vec![event]);
                }
            }
            fixture.settle();
            fixture.run(Vec::new());
            assert!(
                fixture
                    .harness
                    .dockspace
                    .view()
                    .item(ItemId::new(1))
                    .unwrap()
                    .is_active(),
                "{entry:?}, batched={batched}, text={:?}",
                fixture.editors.text
            );
            assert_eq!(
                fixture.editors.text,
                ["", "first"],
                "{entry:?}, batched={batched}"
            );
        }
    }
}

#[test]
fn an_editor_that_consumes_cycling_keeps_its_trailing_input() {
    for entry in ENTRIES {
        let mut fixture = Fixture::new(entry);
        fixture.editors.consume_cycle = true;
        fixture.focus(fixture.editors.focus_target(ItemId::new(1)).unwrap());
        fixture.run(vec![
            shortcut(bindings().next_tab.unwrap()),
            Event::Text("first".into()),
        ]);
        fixture.run(vec![Event::Text("second".into())]);
        fixture.settle();
        assert_selected(&fixture.harness, 1);
        assert_eq!(fixture.editors.text, ["firstsecond", ""], "{entry:?}");
    }
}
