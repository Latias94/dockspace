use super::*;

use egui::{Id, ImeEvent, Key, KeyboardShortcut, Modifiers, ViewportId};

#[derive(Default)]
struct EditingDocuments {
    text: [String; 2],
    scroll: Option<f32>,
    editor: Option<Rect>,
    editor_layout: Option<egui::Layout>,
    opt_out: bool,
    grid_before_editor: bool,
    grid_after_editor: bool,
    select_all_on_focus: bool,
    expose_second_target_on_first_draw: bool,
    first_tab: Option<Id>,
}

fn editor_id(item: ItemId) -> Id {
    Id::unique(("focus-handoff-document", item))
}

impl PaneView for EditingDocuments {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(format!("Document {}", item.get()).into())
    }

    fn focus_target(&self, item: ItemId) -> Option<Id> {
        (!(self.expose_second_target_on_first_draw && item == SECOND && self.editor.is_none()))
            .then(|| editor_id(item))
    }

    fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
        !self.opt_out
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &crate::Tab<'_>) -> crate::TabVisuals {
        if tab.item() == Some(FIRST) {
            self.first_tab = Some(tab.response.id);
        }
        tab.paint(ui)
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
        if item == SECOND && self.grid_before_editor {
            egui::Grid::new("document-metadata").show(ui, |ui| {
                ui.label("Language");
                ui.label("English");
                ui.end_row();
            });
        }
        let target = editor_id(item);
        let mut show_edit = |ui: &mut Ui| {
            crate::pane_text_edit(
                ui,
                target,
                egui::TextEdit::singleline(&mut self.text[usize::from(item == SECOND)]),
            )
        };
        let response = match (item, self.scroll, self.editor_layout) {
            (SECOND, Some(offset), _) => {
                egui::ScrollArea::vertical()
                    .max_height(78.0)
                    .vertical_scroll_offset(offset)
                    .show(ui, |ui| {
                        let response = show_edit(ui);
                        ui.add_space(360.0);
                        response
                    })
                    .inner
            }
            (SECOND, None, Some(layout)) => {
                let bounds = Rect::from_min_size(ui.max_rect().min, egui::vec2(240.0, 96.0));
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(bounds).layout(layout),
                    |ui| {
                        ui.shrink_clip_rect(bounds);
                        show_edit(ui)
                    },
                )
                .inner
            }
            _ => show_edit(ui),
        };
        if item == SECOND {
            self.editor = Some(response.interact_rect);
            if self.select_all_on_focus && response.gained_focus() {
                let mut state = egui::TextEdit::load_state(ui.ctx(), target).unwrap();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(self.text[1].chars().count()),
                    )));
                state.store(ui.ctx(), target);
            }
            if self.grid_after_editor {
                egui::Grid::new("document-footer").show(ui, |ui| {
                    ui.label("Language");
                    ui.label("English");
                    ui.end_row();
                });
            }
        }
        (&response).into()
    }
}

fn editing_dock(tabs: bool) -> Dockspace {
    named_editing_dock("editor-handoff", tabs)
}

fn named_editing_dock(id: &str, tabs: bool) -> Dockspace {
    let content = if tabs {
        DockspaceNode::tabs([FIRST, SECOND])
    } else {
        DockspaceNode::equal_split(
            crate::DockspaceAxis::Horizontal,
            [FIRST, SECOND].map(|item| DockspaceNode::tabs([item])),
        )
        .unwrap()
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), content),
    )])
    .unwrap();
    Dockspace::builder(id, layout).build().unwrap()
}

fn select_document(dock: &mut Dockspace, item: ItemId) {
    assert!(matches!(
        dock.select_item_current(item).unwrap().status(),
        DockspaceActionStatus::Applied(_)
    ));
}

fn command(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }
}

#[test]
fn clicking_the_destination_editor_preserves_edit_and_save_ownership() {
    for owned in [true, false] {
        for queued_paste in [false, true] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                ..Default::default()
            };
            let file = tempfile::NamedTempFile::new().unwrap();
            let mut clipboard = Vec::new();
            let mut time = 0.0;
            let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
                time += 0.1;
                let output = run_editor_frame(dock, &context, input(time, events), docs, owned);
                clipboard.extend(clipboard_text(&output));
                output.drop_without_applying_deltas();
                if consume_save(&context) {
                    std::fs::write(file.path(), &docs.text[1]).unwrap();
                }
            };
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, vec![]);
            let pos = docs.editor.unwrap().center();
            paint(&mut dock, &mut docs, vec![Event::PointerMoved(pos)]);
            select_document(&mut dock, SECOND);
            let mut batch = vec![editor_button(pos, true)];
            if queued_paste {
                paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
                batch.push(editor_button(pos, false));
            } else {
                batch.extend([command(Key::A), Event::Cut]);
            }
            batch.push(command(Key::S));
            paint(&mut dock, &mut docs, batch);
            if !queued_paste {
                paint(&mut dock, &mut docs, vec![editor_button(pos, false)]);
            }
            for _ in 0..3 {
                paint(&mut dock, &mut docs, vec![]);
            }
            let (text, copied) = if queued_paste {
                ("bravoX", vec![])
            } else {
                ("", vec!["bravo"])
            };
            assert_eq!(
                docs.text,
                ["alpha", text],
                "owned={owned}, queued_paste={queued_paste}"
            );
            assert_eq!(clipboard, copied);
            assert_eq!(std::fs::read_to_string(file.path()).unwrap(), text);
        }
    }
}

fn editor_button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn consume_save(context: &Context) -> bool {
    context.input_mut(|input| {
        input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
    })
}

#[test]
fn deferred_paste_uses_the_editors_current_layout_slot() {
    use egui::{Align, Layout};

    for owned in [true, false] {
        for layout in [
            Layout::top_down(Align::Max),
            Layout::right_to_left(Align::Min),
            Layout::bottom_up(Align::Min),
        ] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                editor_layout: Some(layout),
                ..Default::default()
            };
            let mut time = 0.0;
            let mut saved = Vec::new();
            let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
                time += 0.1;
                paint_editor_frame(dock, &context, input(time, events), docs, owned);
                if consume_save(&context) {
                    saved.push(docs.text[1].clone());
                }
            };
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, SECOND);
            paint(
                &mut dock,
                &mut docs,
                vec![Event::Paste("X".into()), command(Key::S)],
            );
            paint(&mut dock, &mut docs, vec![]);
            assert_eq!(saved, ["bravoX"], "owned={owned}, layout={layout:?}");
            assert_eq!(docs.text, ["alpha", "bravoX"]);
        }
    }
}

#[test]
fn escape_in_a_deferred_packet_leaves_save_and_key_up_for_the_host() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(false);
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            ..Default::default()
        };
        let mut time = 0.0;
        let mut saved = Vec::new();
        let mut release_count = 0;
        let release = key_event(Key::F8, Modifiers::NONE, false);
        let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
            time += 0.1;
            paint_editor_frame(dock, &context, input(time, events), docs, owned);
            release_count += context.input(|input| {
                input
                    .events
                    .iter()
                    .filter(|event| **event == release)
                    .count()
            });
            if consume_save(&context) {
                saved.push(docs.text[1].clone());
            }
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(
            &mut dock,
            &mut docs,
            vec![key_event(Key::F8, Modifiers::NONE, true)],
        );
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
        paint(
            &mut dock,
            &mut docs,
            vec![
                Event::Text("P".into()),
                key_event(Key::Escape, Modifiers::NONE, true),
                release.clone(),
                command(Key::S),
            ],
        );
        for _ in 0..4 {
            paint(&mut dock, &mut docs, vec![]);
        }
        assert_eq!(saved, ["bravoXP"], "owned={owned}");
        assert_eq!(release_count, 1);
        assert!(context.memory(egui::Memory::focused).is_none());
    }
}

#[test]
fn header_activation_in_a_plain_pane_leaves_save_for_the_host() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(true);
        let mut docs = EditingDocuments {
            opt_out: true,
            ..Default::default()
        };
        paint_editor_frame(&mut dock, &context, input(0.1, vec![]), &mut docs, owned);
        context.memory_mut(|memory| memory.request_focus(docs.first_tab.unwrap()));
        paint_editor_frame(&mut dock, &context, input(0.2, vec![]), &mut docs, owned);
        paint_editor_frame(
            &mut dock,
            &context,
            input(
                0.3,
                vec![
                    key_event(Key::ArrowRight, Modifiers::NONE, true),
                    key_event(Key::Enter, Modifiers::NONE, true),
                    command(Key::S),
                ],
            ),
            &mut docs,
            owned,
        );
        assert!(consume_save(&context), "owned={owned}");
        paint_editor_frame(&mut dock, &context, input(0.4, vec![]), &mut docs, owned);
        assert!(dock.view().item(SECOND).unwrap().is_active());
    }
}

#[test]
fn an_old_edit_cannot_reach_a_reopened_item_with_the_same_id() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(true);
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            ..Default::default()
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut time = 0.0;
        let mut clipboard = Vec::new();
        let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
            time += 0.1;
            let output = run_editor_frame(dock, &context, input(time, events), docs, owned);
            clipboard.extend(clipboard_text(&output));
            output.drop_without_applying_deltas();
            if consume_save(&context) {
                std::fs::write(file.path(), &docs.text[1]).unwrap();
            }
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
        paint(
            &mut dock,
            &mut docs,
            vec![command(Key::A), Event::Cut, command(Key::S)],
        );
        assert_eq!(docs.text, ["alpha", "bravoX"]);
        let request = dock.request_close_item_current(SECOND).unwrap();
        let crate::DockspaceCloseRequestStatus::Requested(request) = request.status() else {
            panic!("the editor should have a close plan");
        };
        let plan = request.plan();
        let _ = dock
            .resolve_close(
                plan.request(),
                plan.items()[0].token(),
                crate::CloseDecision::Allow,
            )
            .unwrap();
        assert!(dock.view().item(SECOND).is_none());
        let opened = dock
            .open_item_current(
                SECOND,
                crate::DockPlacement::Center(crate::DockAnchor::Item(FIRST)),
            )
            .unwrap();
        assert!(matches!(opened.status(), DockspaceActionStatus::Applied(_)));
        std::fs::write(file.path(), "reloaded from disk").unwrap();
        docs.text[1] = std::fs::read_to_string(file.path()).unwrap();
        for _ in 0..4 {
            paint(&mut dock, &mut docs, vec![]);
        }
        assert_eq!(docs.text, ["alpha", "reloaded from disk"], "owned={owned}");
        assert!(clipboard.is_empty());
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "reloaded from disk"
        );
    }
}

#[test]
fn unlocked_arrow_navigation_rebinds_the_following_packet_to_the_search_field() {
    struct SideBySide {
        body: EditingDocuments,
        search: String,
    }

    impl PaneView for SideBySide {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.body.title(item)
        }

        fn focus_target(&self, item: ItemId) -> Option<Id> {
            self.body.focus_target(item)
        }

        fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
            true
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            if item == FIRST {
                return self.body.ui(item, ui);
            }
            ui.horizontal(|ui| {
                let body = crate::pane_text_edit(
                    ui,
                    editor_id(item),
                    egui::TextEdit::singleline(&mut self.body.text[1])
                        .desired_width(70.0)
                        .event_filter(egui::EventFilter::default()),
                );
                let search = crate::pane_text_edit(
                    ui,
                    DocumentsWithSearch::search_id(),
                    egui::TextEdit::singleline(&mut self.search).desired_width(70.0),
                );
                crate::PaneResponse::from(&body).with_content(&search)
            })
            .inner
        }
    }

    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(false);
        let mut docs = SideBySide {
            body: EditingDocuments {
                text: ["alpha".into(), "B".into()],
                ..Default::default()
            },
            search: "F".into(),
        };
        let mut time = 0.0;
        let mut saved = Vec::new();
        let mut paint = |dock: &mut Dockspace, docs: &mut SideBySide, events| {
            time += 0.1;
            paint_editor_frame(dock, &context, input(time, events), docs, owned);
            if consume_save(&context) {
                saved.push((docs.body.text[1].clone(), docs.search.clone()));
            }
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
        paint(
            &mut dock,
            &mut docs,
            vec![
                key_event(Key::ArrowRight, Modifiers::NONE, true),
                key_event(Key::ArrowRight, Modifiers::NONE, false),
            ],
        );
        paint(
            &mut dock,
            &mut docs,
            vec![Event::Text("N".into()), command(Key::S)],
        );
        for _ in 0..4 {
            paint(&mut dock, &mut docs, vec![]);
        }
        assert_eq!(docs.body.text, ["alpha", "BX"], "owned={owned}");
        assert_eq!(docs.search, "FN");
        assert_eq!(saved, [("BX".into(), "FN".into())]);
        assert_eq!(
            context.memory(egui::Memory::focused),
            Some(DocumentsWithSearch::search_id())
        );
    }
}

#[test]
fn rejected_editor_handoff_does_not_cut_a_scrolled_out_document() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(false);
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            scroll: Some(0.0),
            ..Default::default()
        };
        let mut time = 0.0;
        let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
            time += 0.1;
            let raw = input(time, events);
            let mut capability = crate::DockspaceCapability::Supported;
            let output = if owned {
                let (output, response) = dock
                    .run_single_surface(&context, raw, SURFACE, docs)
                    .unwrap();
                capability = response.pane_focus_capability();
                output
            } else {
                crate::run_ui(&context, raw, |ui| {
                    capability = dock
                        .show_single_surface(SURFACE, ui, docs)
                        .unwrap()
                        .pane_focus_capability();
                })
            };
            let (output, receipt) = output.into_parts();
            let copied = output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::CopyText(_)));
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
            (capability, copied)
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        assert!(docs.editor.unwrap().is_positive());
        assert_eq!(
            context.memory(egui::Memory::focused),
            docs.focus_target(FIRST)
        );

        select_document(&mut dock, SECOND);
        docs.scroll = Some(160.0);
        let (capability, copied) = paint(&mut dock, &mut docs, vec![command(Key::A), Event::Cut]);
        assert!(!docs.editor.unwrap().is_positive());
        assert_eq!(
            capability,
            crate::DockspaceCapability::Unavailable(
                crate::DockspaceUnavailableReason::PaneFocusBindingUnavailable
            )
        );
        assert_eq!(
            context.memory(egui::Memory::focused),
            docs.focus_target(FIRST)
        );
        assert_eq!(docs.text, ["alpha", "bravo"], "owned={owned}");
        assert!(
            !copied,
            "no clipboard side effect from the rejected focus transfer"
        );
        paint(&mut dock, &mut docs, vec![]);
        assert_eq!(docs.text, ["alpha", "bravo"]);
    }
}

#[test]
fn clicking_or_reselecting_the_same_editor_keeps_its_pending_input() {
    for owned in [true, false] {
        for click in [true, false] {
            for (batch, expected) in [
                (vec![Event::Text("X".into())], "bX?"),
                (vec![Event::Paste("paste".into())], "bpaste?"),
                (
                    vec![
                        Event::Ime(ImeEvent::Preedit {
                            text: "ni".into(),
                            active_range_chars: Some(0..2),
                        }),
                        Event::Ime(ImeEvent::Commit("你".into())),
                    ],
                    "b你?",
                ),
            ] {
                let context = Context::default();
                let mut dock = editing_dock(true);
                let mut docs = EditingDocuments {
                    text: ["a".into(), "b".into()],
                    ..Default::default()
                };
                let mut time = 0.0;
                let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
                    time += 0.1;
                    let raw = input(time, events);
                    let output = if owned {
                        dock.run_single_surface(&context, raw, SURFACE, docs)
                            .unwrap()
                            .0
                    } else {
                        crate::run_ui(&context, raw, |ui| {
                            dock.show_single_surface(SURFACE, ui, docs).unwrap();
                        })
                    };
                    let (output, receipt) = output.into_parts();
                    output.drop_without_applying_deltas();
                    receipt.report(SurfacePresentationResult::Presented);
                };
                paint(&mut dock, &mut docs, vec![]);
                select_document(&mut dock, FIRST);
                paint(&mut dock, &mut docs, vec![]);
                select_document(&mut dock, SECOND);
                paint(&mut dock, &mut docs, batch);
                assert_eq!(
                    context.memory(egui::Memory::focused),
                    docs.focus_target(SECOND)
                );
                let mut events = Vec::new();
                if click {
                    let pos = docs.editor.unwrap().center();
                    events.push(Event::PointerMoved(pos));
                    events.extend([true, false].map(|pressed| Event::PointerButton {
                        pos,
                        pressed,
                        button: PointerButton::Primary,
                        modifiers: Modifiers::NONE,
                    }));
                } else {
                    select_document(&mut dock, SECOND);
                }
                events.push(Event::Text("?".into()));
                paint(&mut dock, &mut docs, events);
                assert_eq!(docs.text[1], expected.trim_end_matches('?'));
                paint(&mut dock, &mut docs, vec![]);
                assert_eq!(docs.text, ["a", expected], "owned={owned}, click={click}");
            }
        }
    }
}

#[test]
fn application_save_observes_the_paste_from_the_editor_handoff() {
    for (first, next, saved_text, final_text) in [
        (
            vec![Event::Paste("X".into()), command(Key::S)],
            vec![Event::Text("Y".into())],
            "bX",
            "bXY",
        ),
        (
            vec![command(Key::S)],
            vec![command(Key::A), Event::Cut],
            "b",
            "",
        ),
    ] {
        for idle_between in [false, true] {
            for viewport in [ViewportId::ROOT, ViewportId::from_hash_of("editor window")] {
                assert_save_before_next_input(
                    viewport,
                    &first,
                    &next,
                    idle_between,
                    false,
                    saved_text,
                    final_text,
                );
            }
        }
    }
}

#[test]
fn a_discarded_grid_measurement_keeps_the_pending_paste_and_save() {
    assert_save_before_next_input(
        ViewportId::ROOT,
        &[Event::Paste("X".into()), command(Key::S)],
        &[],
        false,
        true,
        "bX",
        "bX",
    );
}

#[test]
fn a_layout_retry_cannot_edit_the_document_after_a_save_in_the_same_run() {
    let context = Context::default();
    let mut dock = editing_dock(false);
    let mut docs = EditingDocuments {
        text: ["alpha".into(), "bravo".into()],
        ..Default::default()
    };
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut time = 0.0;
    let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
        time += 0.1;
        let mut save = false;
        let (out, receipt) = crate::run_ui(&context, input(time, events), |ui| {
            dock.show_single_surface(SURFACE, ui, docs).unwrap();
            save |= ui.input_mut(|input| {
                input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
            });
        })
        .into_parts();
        let passes = out.platform_output.num_completed_passes;
        out.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
        if save {
            std::fs::write(file.path(), &docs.text[1]).unwrap();
        }
        passes
    };
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, FIRST);
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, SECOND);
    paint(
        &mut dock,
        &mut docs,
        vec![Event::Paste("X".into()), command(Key::S)],
    );
    docs.grid_after_editor = true;
    assert_eq!(
        paint(&mut dock, &mut docs, vec![command(Key::A), Event::Cut]),
        2
    );
    assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "bravoX");
    assert_eq!(docs.text, ["alpha", "bravoX"]);
    paint(&mut dock, &mut docs, vec![]);
    assert_eq!(docs.text, ["alpha", ""]);
}

struct DocumentsWithSearch {
    docs: EditingDocuments,
    search: String,
}

impl DocumentsWithSearch {
    fn search_id() -> Id {
        Id::unique("search-document")
    }
}

impl PaneView for DocumentsWithSearch {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        self.docs.title(item)
    }

    fn focus_target(&self, item: ItemId) -> Option<Id> {
        self.docs.focus_target(item)
    }

    fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
        true
    }

    fn focus_state(&self, item: ItemId, ctx: &Context) -> crate::PaneFocusState {
        if item == SECOND && ctx.memory(|m| m.has_focus(Self::search_id())) {
            crate::PaneFocusState::Focused
        } else {
            self.docs.focus_state(item, ctx)
        }
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
        let body = self.docs.ui(item, ui);
        if item == SECOND {
            if ui.memory(|m| m.has_focus(Self::search_id())) {
                ui.input_mut(|input| {
                    input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Tab));
                });
            }
            body.with_content(&crate::pane_text_edit(
                ui,
                Self::search_id(),
                egui::TextEdit::singleline(&mut self.search),
            ))
        } else {
            body
        }
    }
}

fn paint_editor_frame(
    dock: &mut Dockspace,
    context: &Context,
    raw: RawInput,
    panes: &mut dyn PaneView,
    owned: bool,
) {
    run_editor_frame(dock, context, raw, panes, owned).drop_without_applying_deltas();
}

fn run_editor_frame(
    dock: &mut Dockspace,
    context: &Context,
    raw: RawInput,
    panes: &mut dyn PaneView,
    owned: bool,
) -> FullOutput {
    let output = if owned {
        dock.run_single_surface(context, raw, SURFACE, panes)
            .unwrap()
            .0
    } else {
        crate::run_ui(context, raw, |ui| {
            dock.show_single_surface(SURFACE, ui, panes).unwrap();
        })
    };
    let (output, receipt) = output.into_parts();
    receipt.report(SurfacePresentationResult::Presented);
    output
}

fn clipboard_text(output: &FullOutput) -> impl Iterator<Item = String> + '_ {
    output
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
}

fn key_event(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn selecting_an_unbound_pane_does_not_take_another_docks_paste() {
    struct Unbound;
    impl PaneView for Unbound {
        fn title(&self, _: ItemId) -> Option<egui::WidgetText> {
            Some("Unbound".into())
        }

        fn ui(&mut self, _: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            ui.label("No focus binding");
            crate::PaneResponse::default()
        }
    }

    for left_first in [true, false] {
        let context = Context::default();
        let mut left = named_editing_dock("unbound-dock", true);
        let mut right = editing_dock(false);
        let mut docs = EditingDocuments {
            text: ["draft:".into(), "other".into()],
            ..Default::default()
        };
        let paint = |left: &mut Dockspace,
                     right: &mut Dockspace,
                     docs: &mut EditingDocuments,
                     time,
                     events| {
            let (output, receipt) = crate::run_ui(&context, input(time, events), |ui| {
                ui.columns(2, |cols| {
                    for index in if left_first { [0, 1] } else { [1, 0] } {
                        if index == 0 {
                            left.show_single_surface(SURFACE, &mut cols[0], &mut Unbound)
                                .unwrap();
                        } else {
                            right
                                .show_single_surface(SURFACE, &mut cols[1], docs)
                                .unwrap();
                        }
                    }
                });
            })
            .into_parts();
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
        };
        paint(&mut left, &mut right, &mut docs, 0.1, vec![]);
        select_document(&mut right, FIRST);
        paint(&mut left, &mut right, &mut docs, 0.2, vec![]);
        paint(&mut left, &mut right, &mut docs, 0.3, vec![]);
        assert_eq!(
            context.memory(egui::Memory::focused),
            Some(editor_id(FIRST))
        );
        select_document(&mut left, SECOND);
        paint(
            &mut left,
            &mut right,
            &mut docs,
            0.4,
            vec![Event::Paste("invoice".into())],
        );
        assert_eq!(
            docs.text,
            ["draft:invoice", "other"],
            "left_first={left_first}"
        );
    }
}

#[test]
fn replacement_search_does_not_inherit_the_documents_pending_edit() {
    struct Replaced {
        docs: EditingDocuments,
        replacement: Option<String>,
    }
    impl PaneView for Replaced {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.docs.title(item)
        }

        fn focus_target(&self, item: ItemId) -> Option<Id> {
            self.docs.focus_target(item)
        }

        fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
            true
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            if item == SECOND
                && let Some(search) = &mut self.replacement
            {
                let id = Id::unique("replacement-search");
                ui.memory_mut(|memory| memory.request_focus(id));
                return (&ui.add(egui::TextEdit::singleline(search).id(id))).into();
            }
            self.docs.ui(item, ui)
        }
    }

    for owned in [true, false] {
        for edit in [
            vec![command(Key::A), Event::Cut],
            vec![Event::Ime(ImeEvent::Commit("你".into()))],
        ] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = Replaced {
                docs: EditingDocuments {
                    text: ["alpha".into(), "bravo".into()],
                    ..Default::default()
                },
                replacement: None,
            };
            let mut clipboard = Vec::new();
            let mut paint = |dock: &mut Dockspace, docs: &mut Replaced, time, events| {
                let output = run_editor_frame(dock, &context, input(time, events), docs, owned);
                clipboard.extend(clipboard_text(&output));
                output.drop_without_applying_deltas();
            };
            paint(&mut dock, &mut docs, 0.1, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, 0.2, vec![]);
            select_document(&mut dock, SECOND);
            paint(&mut dock, &mut docs, 0.3, edit.clone());
            docs.replacement = Some("find".into());
            paint(&mut dock, &mut docs, 0.4, vec![]);
            paint(&mut dock, &mut docs, 0.5, vec![]);
            assert_eq!(
                docs.replacement.as_deref(),
                Some("find"),
                "owned={owned}, edit={edit:?}"
            );
            assert_eq!(docs.docs.text, ["alpha", "bravo"]);
            assert!(clipboard.is_empty(), "owned={owned}, edit={edit:?}");
        }
    }
}

#[test]
fn a_click_cut_and_save_packet_cannot_be_replayed_into_a_third_document() {
    const THIRD: ItemId = ItemId::new(3);
    #[derive(Default)]
    struct ThreeDocuments {
        docs: EditingDocuments,
        third: String,
        first_editor: Option<Rect>,
    }
    impl PaneView for ThreeDocuments {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.docs.title(item)
        }

        fn focus_target(&self, item: ItemId) -> Option<Id> {
            Some(editor_id(item))
        }

        fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
            true
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            if item == THIRD {
                return (&crate::pane_text_edit(
                    ui,
                    editor_id(item),
                    egui::TextEdit::singleline(&mut self.third),
                ))
                    .into();
            }
            let response = self.docs.ui(item, ui);
            if item == FIRST {
                self.first_editor = ui.ctx().viewport(|viewport| {
                    viewport
                        .this_pass
                        .widgets
                        .get(editor_id(FIRST))
                        .map(|widget| widget.interact_rect)
                });
            }
            response
        }
    }

    let content = DockspaceNode::equal_split(
        crate::DockspaceAxis::Horizontal,
        [FIRST, SECOND, THIRD].map(|item| DockspaceNode::tabs([item])),
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), content),
    )])
    .unwrap();
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = Dockspace::builder("three edit targets", layout.clone())
            .build()
            .unwrap();
        let mut docs = ThreeDocuments {
            docs: EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                ..Default::default()
            },
            third: "charlie".into(),
            ..Default::default()
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &docs.third).unwrap();
        let mut clipboard = Vec::new();
        let mut paint = |dock: &mut Dockspace, docs: &mut ThreeDocuments, time, events| {
            let raw = input(time, events);
            let save_for_third = |context: &Context| {
                context.input_mut(|input| {
                    input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
                }) && context.memory(|memory| memory.has_focus(editor_id(THIRD)))
            };
            let (output, save_third) = if owned {
                let output = run_editor_frame(dock, &context, raw, docs, true);
                (output, save_for_third(&context))
            } else {
                let mut save_third = false;
                let (output, receipt) = crate::run_ui(&context, raw, |ui| {
                    dock.show_single_surface(SURFACE, ui, docs).unwrap();
                    save_third |= save_for_third(ui.ctx());
                })
                .into_parts();
                receipt.report(SurfacePresentationResult::Presented);
                (output, save_third)
            };
            clipboard.extend(clipboard_text(&output));
            output.drop_without_applying_deltas();
            if save_third {
                std::fs::write(file.path(), &docs.third).unwrap();
            }
        };
        paint(&mut dock, &mut docs, 0.1, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, 0.2, vec![]);
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, 0.3, vec![Event::Paste("X".into())]);
        let pos = docs.first_editor.unwrap().center();
        let mut events = vec![Event::PointerMoved(pos)];
        events.extend([true, false].map(|pressed| Event::PointerButton {
            pos,
            pressed,
            button: PointerButton::Primary,
            modifiers: Modifiers::NONE,
        }));
        events.extend([command(Key::A), Event::Cut, command(Key::S)]);
        paint(&mut dock, &mut docs, 0.4, events);
        select_document(&mut dock, THIRD);
        for time in [0.5, 0.6, 0.7] {
            paint(&mut dock, &mut docs, time, vec![]);
        }
        assert_eq!(docs.third, "charlie", "owned={owned}");
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "charlie");
        assert!(!clipboard.iter().any(|text| text == "charlie"));
    }
}

#[test]
fn a_delayed_focus_key_is_processed_before_later_text() {
    for owned in [true, false] {
        for key in [Key::Tab, Key::Escape] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = DocumentsWithSearch {
                docs: EditingDocuments {
                    text: ["alpha".into(), "bravo".into()],
                    ..Default::default()
                },
                search: "find".into(),
            };
            let mut time = 0.0;
            let mut paint = |dock: &mut Dockspace, docs: &mut DocumentsWithSearch, events| {
                time += 0.1;
                paint_editor_frame(dock, &context, input(time, events), docs, owned);
            };
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, FIRST);
            paint(
                &mut dock,
                &mut docs,
                vec![key_event(Key::F6, Modifiers::NONE, true)],
            );
            assert!(context.input(|input| input.key_down(Key::F6)));
            select_document(&mut dock, SECOND);
            paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
            paint(
                &mut dock,
                &mut docs,
                vec![
                    key_event(key, Modifiers::NONE, true),
                    key_event(Key::F6, Modifiers::NONE, false),
                ],
            );
            assert!(!context.input(|input| input.key_down(Key::F6)));
            assert!(context.input(|input| input.key_down(key)));
            paint(
                &mut dock,
                &mut docs,
                vec![
                    key_event(key, Modifiers::NONE, false),
                    Event::Text("Y".into()),
                ],
            );
            paint(&mut dock, &mut docs, vec![]);
            assert_eq!(
                docs.docs.text,
                ["alpha", "bravoX"],
                "owned={owned}, key={key:?}"
            );
            let (focus, search) = if key == Key::Tab {
                (Some(DocumentsWithSearch::search_id()), "findY")
            } else {
                (None, "find")
            };
            assert_eq!(context.memory(egui::Memory::focused), focus);
            assert_eq!(docs.search, search);
            assert!(!context.input(|input| input.key_down(key)));
        }
    }
}

#[test]
fn queued_cursor_keys_keep_their_order_with_text_and_clipboard_events() {
    for queued in [false, true] {
        for (modifiers, edit, text, copied) in [
            (Modifiers::NONE, Event::Text("Y".into()), "bravoYX", None),
            (Modifiers::SHIFT, Event::Cut, "bravo", Some("X")),
        ] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                ..Default::default()
            };
            let mut clipboard = Vec::new();
            let mut time = 0.0;
            let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
                time += 0.1;
                let output = run_editor_frame(dock, &context, input(time, events), docs, false);
                clipboard.extend(clipboard_text(&output));
                output.drop_without_applying_deltas();
            };
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, SECOND);
            paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
            if !queued {
                paint(&mut dock, &mut docs, vec![]);
            }
            paint(
                &mut dock,
                &mut docs,
                vec![key_event(Key::ArrowLeft, modifiers, true), edit],
            );
            paint(&mut dock, &mut docs, vec![]);
            paint(&mut dock, &mut docs, vec![]);
            assert_eq!(docs.text, ["alpha", text], "queued={queued}, {modifiers:?}");
            assert_eq!(clipboard, copied.into_iter().collect::<Vec<_>>());
        }
    }
}

#[test]
fn buffered_tab_cycling_keeps_its_text_suffix_out_of_the_departing_document() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(true);
        dock.set_input(crate::DockInputConfig {
            next_tab: Some(KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)),
            ..Default::default()
        });
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            ..Default::default()
        };
        let paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, time, events| {
            paint_editor_frame(dock, &context, input(time, events), docs, owned);
        };
        paint(&mut dock, &mut docs, 0.1, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, 0.2, vec![]);
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, 0.3, vec![Event::Paste("X".into())]);
        paint(
            &mut dock,
            &mut docs,
            0.4,
            vec![
                key_event(Key::Tab, Modifiers::CTRL, true),
                Event::Text("Y".into()),
            ],
        );
        for time in [0.5, 0.6, 0.7] {
            paint(&mut dock, &mut docs, time, vec![]);
        }
        assert!(dock.view().item(FIRST).unwrap().is_active());
        assert_eq!(docs.text, ["alpha", "bravoX"], "owned={owned}");
    }
}

#[test]
fn ime_commit_and_save_finish_before_a_grid_retry_and_tab_change() {
    for grid_before_editor in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(true);
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            ..Default::default()
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut time = 0.0;
        let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
            time += 0.1;
            let mut save = false;
            let (output, receipt) = crate::run_ui(&context, input(time, events), |ui| {
                dock.show_single_surface(SURFACE, ui, docs).unwrap();
                save |= ui.input_mut(|input| {
                    input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
                });
            })
            .into_parts();
            let passes = output.platform_output.num_completed_passes;
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
            if save {
                std::fs::write(file.path(), &docs.text[1]).unwrap();
            }
            (passes, save)
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, SECOND);
        paint(
            &mut dock,
            &mut docs,
            vec![Event::Ime(ImeEvent::Preedit {
                text: "ni".into(),
                active_range_chars: Some(0..2),
            })],
        );
        docs.grid_before_editor = grid_before_editor;
        docs.grid_after_editor = !grid_before_editor;
        let (passes, saved) = paint(
            &mut dock,
            &mut docs,
            vec![
                Event::Ime(ImeEvent::Preedit {
                    text: String::new(),
                    active_range_chars: None,
                }),
                Event::Ime(ImeEvent::Commit("你".into())),
                command(Key::S),
            ],
        );
        assert_eq!(passes, 2);
        assert!(saved, "grid_before_editor={grid_before_editor}");
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        assert_eq!(docs.text, ["alpha", "bravo你"]);
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "bravo你");
    }
}

#[test]
fn an_unmounted_dock_does_not_send_queued_navigation_to_the_replacement_page() {
    for viewport in [
        ViewportId::ROOT,
        ViewportId::from_hash_of("settings viewport"),
    ] {
        for key in [Key::Tab, Key::Escape] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                ..Default::default()
            };
            let viewport_input = |time, events| {
                let mut raw = input(time, events);
                raw.viewport_id = viewport;
                raw.viewports.entry(viewport).or_default();
                raw
            };
            let paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, time, events| {
                paint_editor_frame(dock, &context, viewport_input(time, events), docs, false);
            };
            paint(&mut dock, &mut docs, 0.1, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, 0.2, vec![]);
            select_document(&mut dock, SECOND);
            paint(&mut dock, &mut docs, 0.3, vec![Event::Paste("X".into())]);
            paint(
                &mut dock,
                &mut docs,
                0.4,
                vec![key_event(key, Modifiers::NONE, true)],
            );

            let mut title = String::from("Title");
            let mut note = String::from("Note");
            let title_id = Id::unique("settings-title");
            for (time, events) in [(0.5, vec![]), (0.6, vec![Event::Text("Y".into())])] {
                context
                    .run_ui(viewport_input(time, events), |ui| {
                        assert!(!ui.input(|input| input.key_pressed(key)));
                        if time == 0.5 {
                            ui.memory_mut(|memory| memory.request_focus(title_id));
                        }
                        ui.add(egui::TextEdit::singleline(&mut title).id(title_id));
                        ui.add(
                            egui::TextEdit::singleline(&mut note).id(Id::unique("settings-note")),
                        );
                    })
                    .drop_without_applying_deltas();
            }
            assert_eq!(
                [&*title, &*note],
                ["TitleY", "Note"],
                "{viewport:?}, {key:?}"
            );
        }
    }
}

#[test]
fn keyboard_activation_can_address_an_editor_registered_during_its_first_draw() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(true);
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            expose_second_target_on_first_draw: true,
            ..Default::default()
        };
        let mut time = 0.0;
        let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
            time += 0.1;
            paint_editor_frame(dock, &context, input(time, events), docs, owned);
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        let tab = docs.first_tab.unwrap();
        context.memory_mut(|memory| memory.request_focus(tab));
        paint(&mut dock, &mut docs, vec![]);
        assert!(docs.focus_target(SECOND).is_none());
        paint(
            &mut dock,
            &mut docs,
            vec![
                key_event(Key::ArrowRight, Modifiers::NONE, true),
                key_event(Key::Enter, Modifiers::NONE, true),
                command(Key::A),
                Event::Cut,
            ],
        );
        for _ in 0..3 {
            paint(&mut dock, &mut docs, vec![]);
        }
        assert_eq!(
            context.memory(egui::Memory::focused),
            Some(editor_id(SECOND))
        );
        assert_eq!(docs.text, ["alpha", ""], "owned={owned}");
    }
}

#[test]
fn an_independent_save_shortcut_reaches_panels_during_an_owned_run() {
    let context = Context::default();
    let mut dock = editing_dock(false);
    let mut docs = EditingDocuments {
        text: ["alpha".into(), "bravo".into()],
        ..Default::default()
    };
    let mut time = 0.0;
    let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
        time += 0.1;
        let mut save = false;
        let (output, _) = dock
            .run_single_surface_with_ui(&context, input(time, events), SURFACE, docs, |ui| {
                save |= ui.input_mut(|input| {
                    input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
                });
            })
            .unwrap();
        let (output, receipt) = output.into_parts();
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
        save
    };
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, FIRST);
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, SECOND);
    paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
    assert!(paint(&mut dock, &mut docs, vec![command(Key::S)]));
    assert_eq!(docs.text, ["alpha", "bravoX"]);
    assert!(!paint(&mut dock, &mut docs, vec![]));
}

#[test]
fn a_pane_shortcut_continues_at_its_focused_subfield() {
    for owned in [true, false] {
        let context = Context::default();
        let mut dock = editing_dock(true);
        dock.set_input(crate::DockInputConfig {
            next_tab: Some(KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)),
            ..Default::default()
        });
        let mut docs = DocumentsWithSearch {
            docs: EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                ..Default::default()
            },
            search: "find".into(),
        };
        let paint = |dock: &mut Dockspace, docs: &mut DocumentsWithSearch, time, events| {
            paint_editor_frame(dock, &context, input(time, events), docs, owned);
        };
        paint(&mut dock, &mut docs, 0.1, vec![]);
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, 0.2, vec![]);
        context.memory_mut(|memory| memory.request_focus(DocumentsWithSearch::search_id()));
        paint(&mut dock, &mut docs, 0.3, vec![]);
        paint(
            &mut dock,
            &mut docs,
            0.4,
            vec![
                key_event(Key::Tab, Modifiers::CTRL, true),
                Event::Text("X".into()),
            ],
        );
        paint(&mut dock, &mut docs, 0.5, vec![Event::Text("Y".into())]);
        assert_eq!(docs.search, "findX", "owned={owned}");
        paint(&mut dock, &mut docs, 0.6, vec![]);
        assert_eq!(docs.search, "findXY", "owned={owned}");
        assert_eq!(docs.docs.text, ["alpha", "bravo"]);
    }
}

#[test]
fn unmounting_dockspace_releases_the_viewport_keyboard() {
    let context = Context::default();
    let mut dock = editing_dock(false);
    let mut docs = EditingDocuments {
        text: ["alpha".into(), "bravo".into()],
        ..Default::default()
    };
    let paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, time, events| {
        paint_editor_frame(dock, &context, input(time, events), docs, false);
    };
    paint(&mut dock, &mut docs, 0.1, vec![]);
    select_document(&mut dock, FIRST);
    paint(&mut dock, &mut docs, 0.2, vec![]);
    select_document(&mut dock, SECOND);
    paint(&mut dock, &mut docs, 0.3, vec![Event::Paste("X".into())]);
    paint(&mut dock, &mut docs, 0.4, vec![command(Key::A), Event::Cut]);
    assert_eq!(docs.text, ["alpha", "bravoX"]);
    let mut draft = String::from("draft");
    let editor = Id::unique("independent-editor");
    let mut saved = false;
    let mut repaint_delay = Duration::ZERO;
    for (time, events) in [
        (0.5, vec![Event::Text("Y".into())]),
        (0.6, vec![Event::Paste("Z".into()), command(Key::S)]),
        (0.7, vec![]),
        (0.8, vec![]),
        (0.9, vec![]),
    ] {
        let output = context.run_ui(input(time, events), |ui| {
            if !ui.memory(|m| m.has_focus(editor)) {
                ui.memory_mut(|m| m.request_focus(editor));
            }
            ui.add(egui::TextEdit::singleline(&mut draft).id(editor));
            saved |= ui.input_mut(|input| {
                input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
            });
        });
        repaint_delay = output.viewport_output[&ViewportId::ROOT].repaint_delay;
        output.drop_without_applying_deltas();
    }
    assert_eq!(draft, "draftYZ");
    assert!(saved);
    assert_eq!(docs.text, ["alpha", "bravoX"]);
    assert!(
        !repaint_delay.is_zero(),
        "unmounting must end keyboard polling"
    );
}

#[test]
fn a_queued_cut_cannot_reach_a_different_editor() {
    for owned in [false, true] {
        for replace_workspace in [false, true] {
            let context = Context::default();
            let mut dock = editing_dock(false);
            let mut docs = EditingDocuments {
                text: ["alpha".into(), "bravo".into()],
                ..Default::default()
            };
            let mut time = 0.0;
            let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
                time += 0.1;
                let raw = input(time, events);
                let output = if owned {
                    dock.run_single_surface(&context, raw, SURFACE, docs)
                        .unwrap()
                        .0
                } else {
                    crate::run_ui(&context, raw, |ui| {
                        dock.show_single_surface(SURFACE, ui, docs).unwrap();
                    })
                };
                let (output, receipt) = output.into_parts();
                let copied = output
                    .platform_output
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::OutputCommand::CopyText(_)));
                output.drop_without_applying_deltas();
                receipt.report(SurfacePresentationResult::Presented);
                copied
            };
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, SECOND);
            paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
            assert!(!paint(
                &mut dock,
                &mut docs,
                vec![command(Key::A), Event::Cut]
            ));
            assert_eq!(docs.text, ["alpha", "bravoX"]);

            if replace_workspace {
                dock = editing_dock(false);
                docs.text = ["replacement A".into(), "replacement B".into()];
                // The new workspace deliberately reuses the original widget IDs.
                assert!(!paint(&mut dock, &mut docs, vec![]));
                assert_eq!(docs.text, ["replacement A", "replacement B"]);
            } else {
                select_document(&mut dock, FIRST);
                assert!(!paint(&mut dock, &mut docs, vec![Event::Text("?".into())]));
                assert_eq!(docs.text, ["alpha", "bravoX"]);
                assert_eq!(
                    context.memory(egui::Memory::focused),
                    docs.focus_target(FIRST)
                );
                assert!(!paint(&mut dock, &mut docs, vec![]));
                assert_eq!(docs.text, ["alpha?", "bravoX"]);
            }
        }
    }
}

#[test]
fn an_editor_observes_focus_before_its_pending_paste() {
    for (tabs, grid_before_editor) in [(false, false), (false, true), (true, false), (true, true)] {
        let context = Context::default();
        let mut dock = editing_dock(tabs);
        let mut docs = EditingDocuments {
            text: ["alpha".into(), "bravo".into()],
            select_all_on_focus: true,
            ..Default::default()
        };
        let mut time = 0.0;
        let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
            time += 0.1;
            let (output, _) = dock
                .run_single_surface(&context, input(time, events), SURFACE, docs)
                .unwrap();
            let (output, receipt) = output.into_parts();
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
        };
        paint(&mut dock, &mut docs, vec![]);
        select_document(&mut dock, FIRST);
        paint(&mut dock, &mut docs, vec![]);
        docs.grid_before_editor = grid_before_editor;
        select_document(&mut dock, SECOND);
        paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
        assert_eq!(docs.text, ["alpha", "bravo"]);
        paint(&mut dock, &mut docs, vec![]);
        assert_eq!(
            docs.text,
            ["alpha", "X"],
            "tabs={tabs}, Grid before editor={grid_before_editor}"
        );
    }
}

fn assert_save_before_next_input(
    viewport: ViewportId,
    first: &[Event],
    next: &[Event],
    idle_between: bool,
    grid_before_editor: bool,
    saved_text: &str,
    final_text: &str,
) {
    let context = Context::default();
    let mut dock = editing_dock(false);
    let mut docs = EditingDocuments {
        text: ["a".into(), "b".into()],
        ..Default::default()
    };
    let mut saved = Vec::new();
    let saved_file = tempfile::NamedTempFile::new().unwrap();
    let mut time = 0.0;
    let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
        time += 0.1;
        let mut raw = input(time, events);
        raw.viewport_id = viewport;
        raw.viewports.entry(viewport).or_default();
        let output = crate::run_ui(&context, raw, |ui| {
            dock.show_single_surface(SURFACE, ui, docs).unwrap();
            if ui.input_mut(|input| {
                input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))
            }) {
                saved.push(docs.text[1].clone());
                std::fs::write(saved_file.path(), &docs.text[1]).unwrap();
            }
        });
        let (output, receipt) = output.into_parts();
        let passes = output.platform_output.num_completed_passes;
        let repaint_delay = output.viewport_output[&viewport].repaint_delay;
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
        (passes, repaint_delay)
    };
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, FIRST);
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, SECOND);
    docs.grid_before_editor = grid_before_editor;
    let (passes, _) = paint(&mut dock, &mut docs, first.to_vec());
    assert_eq!(passes, if grid_before_editor { 2 } else { 1 });
    if idle_between {
        paint(&mut dock, &mut docs, vec![]);
    }
    let (_, repaint_delay) = paint(&mut dock, &mut docs, next.to_vec());
    if !idle_between && !next.is_empty() {
        assert!(
            repaint_delay.is_zero(),
            "queued input must wake {viewport:?}"
        );
    }
    paint(&mut dock, &mut docs, vec![]);
    assert_eq!(docs.text, ["a", final_text]);
    assert_eq!(saved, [saved_text], "idle_between={idle_between}");
    assert_eq!(
        std::fs::read_to_string(saved_file.path()).unwrap(),
        saved_text
    );
}

#[test]
fn a_later_find_shortcut_cannot_redirect_a_pending_cut_or_ime_commit() {
    struct FindDocuments {
        docs: EditingDocuments,
        query: String,
    }
    fn find_id() -> Id {
        Id::unique("find-in-document")
    }
    impl PaneView for FindDocuments {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.docs.title(item)
        }

        fn focus_target(&self, item: ItemId) -> Option<Id> {
            self.docs.focus_target(item)
        }

        fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
            true
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            if item == SECOND {
                let document = crate::with_pane_keyboard_input(
                    ui,
                    editor_id(item),
                    egui::EventFilter::default(),
                    |ui| {
                        if ui.input_mut(|input| {
                            input.consume_shortcut(&KeyboardShortcut::new(
                                Modifiers::COMMAND,
                                Key::F,
                            ))
                        }) {
                            ui.memory_mut(|memory| memory.request_focus(find_id()));
                        }
                        self.docs.ui(item, ui)
                    },
                );
                document.with_content(
                    &ui.add(egui::TextEdit::singleline(&mut self.query).id(find_id())),
                )
            } else {
                self.docs.ui(item, ui)
            }
        }
    }

    for (events, document, copied) in [
        (vec![command(Key::A), Event::Cut], "", Some("bravo")),
        (
            vec![Event::Ime(ImeEvent::Commit("你".into()))],
            "bravo你",
            None,
        ),
    ] {
        for (owned, tabs) in [(true, true), (true, false), (false, true), (false, false)] {
            let context = Context::default();
            let mut dock = editing_dock(tabs);
            let mut docs = FindDocuments {
                docs: EditingDocuments {
                    text: ["alpha".into(), "bravo".into()],
                    ..Default::default()
                },
                query: "find".into(),
            };
            let mut clipboard = Vec::new();
            let mut time = 0.0;
            let mut paint = |dock: &mut Dockspace, docs: &mut FindDocuments, events| {
                time += 0.1;
                let raw = input(time, events);
                let output = if owned {
                    dock.run_single_surface(&context, raw, SURFACE, docs)
                        .unwrap()
                        .0
                } else {
                    crate::run_ui(&context, raw, |ui| {
                        dock.show_single_surface(SURFACE, ui, docs).unwrap();
                    })
                };
                let (output, receipt) = output.into_parts();
                clipboard.extend(output.platform_output.commands.iter().filter_map(
                    |cmd| match cmd {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    },
                ));
                output.drop_without_applying_deltas();
                receipt.report(SurfacePresentationResult::Presented);
            };
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, FIRST);
            paint(&mut dock, &mut docs, vec![]);
            select_document(&mut dock, SECOND);
            paint(&mut dock, &mut docs, events.clone());
            paint(&mut dock, &mut docs, vec![command(Key::F)]);
            paint(&mut dock, &mut docs, vec![]);
            assert_eq!(
                docs.docs.text,
                ["alpha", document],
                "owned={owned}, tabs={tabs}"
            );
            assert_eq!(docs.query, "find");
            assert_eq!(clipboard, copied.into_iter().collect::<Vec<_>>());
            assert_eq!(context.memory(egui::Memory::focused), Some(find_id()));
        }
    }
}

#[test]
fn an_editor_handoff_does_not_replay_text_already_delivered_to_an_application_editor() {
    let context = Context::default();
    let mut dock = editing_dock(false);
    let mut docs = EditingDocuments {
        text: ["a".into(), "b".into()],
        ..Default::default()
    };
    let search_id = Id::unique("before-docking-search");
    let mut search = String::new();
    let mut time = 0.0;
    let mut paint = |dock: &mut Dockspace, docs: &mut EditingDocuments, events| {
        time += 0.1;
        let (output, _) = dock
            .run_single_surface_with_ui(&context, input(time, events), SURFACE, docs, |ui| {
                egui::Panel::top("search-panel").show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut search).id(search_id));
                });
            })
            .unwrap();
        let (output, receipt) = output.into_parts();
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
    };
    paint(&mut dock, &mut docs, vec![]);
    context.memory_mut(|memory| memory.request_focus(search_id));
    paint(&mut dock, &mut docs, vec![]);
    select_document(&mut dock, SECOND);
    paint(&mut dock, &mut docs, vec![Event::Paste("X".into())]);
    paint(&mut dock, &mut docs, vec![Event::Text("?".into())]);
    assert_eq!(docs.text, ["a", "b?"]);
    assert_eq!(search, "X");
}

#[test]
fn each_text_packet_goes_to_the_selected_editor_once() {
    struct Editors {
        text: [String; 2],
        footer: bool,
    }

    impl PaneView for Editors {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Some(format!("Document {}", item.get()).into())
        }

        fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
            Some(egui::Id::unique(("document-editor", item)))
        }

        fn accepts_deferred_keyboard_input(&self, _: ItemId) -> bool {
            true
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> crate::PaneResponse {
            let id = self.focus_target(item).unwrap();
            let text = &mut self.text[usize::from(item == SECOND)];
            let response =
                crate::pane_text_edit(ui, id, egui::TextEdit::multiline(text).desired_width(160.0));
            if item == SECOND && self.footer {
                let clip = ui.clip_rect();
                let footer =
                    Rect::from_min_max(clip.left_bottom() - egui::vec2(0.0, 24.0), clip.max);
                ui.shrink_clip_rect(footer);
                assert!(!ui.clip_rect().intersects(response.interact_rect));
                ui.painter().line_segment(
                    [footer.left_top(), footer.right_top()],
                    (1.0, Color32::WHITE),
                );
            }
            (&response).into()
        }
    }

    for owned in [true, false] {
        let split = |children: [ItemId; 2]| {
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                children.map(|item| DockspaceNode::tabs([item])),
            )
            .unwrap()
        };
        for content in [
            split([FIRST, SECOND]),
            split([SECOND, FIRST]),
            DockspaceNode::tabs([FIRST, SECOND]),
        ] {
            for footer in [false, true] {
                let context = Context::default();
                let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
                    SURFACE,
                    DockspaceRootLayout::new(RootId::new(1), content.clone()),
                )])
                .unwrap();
                let mut dock = Dockspace::builder("editor-input-owner", layout)
                    .build()
                    .unwrap();
                let mut editors = Editors {
                    text: ["first".into(), "second".into()],
                    footer,
                };
                let mut time = 0.0;
                let mut paint = |dock: &mut Dockspace, editors: &mut Editors, events| {
                    time += 0.1;
                    let raw = input(time, events);
                    let output = if owned {
                        dock.run_single_surface(&context, raw, SURFACE, editors)
                            .unwrap()
                            .0
                    } else {
                        crate::run_ui(&context, raw, |ui| {
                            dock.show_single_surface(SURFACE, ui, editors).unwrap();
                        })
                    };
                    let (output, receipt) = output.into_parts();
                    output.drop_without_applying_deltas();
                    receipt.report(SurfacePresentationResult::Presented);
                };
                paint(&mut dock, &mut editors, vec![]);
                assert!(matches!(
                    dock.select_item_current(FIRST).unwrap().status(),
                    DockspaceActionStatus::Applied(_)
                ));
                paint(&mut dock, &mut editors, vec![]);
                assert_eq!(
                    context.memory(egui::Memory::focused),
                    editors.focus_target(FIRST)
                );
                assert!(matches!(
                    dock.select_item_current(SECOND).unwrap().status(),
                    DockspaceActionStatus::Applied(_)
                ));
                paint(&mut dock, &mut editors, vec![Event::Text("!".into())]);
                assert_eq!(
                    editors.text,
                    ["first", "second"],
                    "editing waits for the handoff frame's registration, owned={owned}, footer={footer}"
                );
                paint(&mut dock, &mut editors, vec![Event::Text("?".into())]);
                assert_eq!(editors.text, ["first", "second!"]);
                paint(&mut dock, &mut editors, vec![]);
                assert_eq!(editors.text, ["first", "second!?"]);
                assert_eq!(
                    context.memory(egui::Memory::focused),
                    editors.focus_target(SECOND)
                );
            }
        }
    }
}

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
