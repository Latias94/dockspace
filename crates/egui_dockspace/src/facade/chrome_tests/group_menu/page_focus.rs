//! A deferred menu-page focus request must not override newer application focus.

use super::*;

fn editor_frame(
    harness: &mut Harness,
    editor: egui::Id,
    text: &mut String,
    mut request_focus: Option<egui::Id>,
    events: Vec<Event>,
) -> FullOutput {
    harness.frame += 1;
    let output = crate::run_ui(
        &harness.context,
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            time: Some(f64::from(harness.frame) * 0.1),
            events,
            ..Default::default()
        },
        |ui| {
            if let Some(target) = request_focus.take() {
                ui.memory_mut(|memory| memory.request_focus(target));
            }
            ui.add(egui::TextEdit::singleline(text).id(editor));
            harness
                .dockspace
                .show_single_surface(SURFACE, ui, &mut harness.panes)
                .unwrap();
        },
    );
    let (mut output, receipt) = output.into_parts();
    receipt.report(SurfacePresentationResult::Presented);
    output.textures_delta.clear();
    output
}

#[derive(Clone, Copy, Debug)]
enum PageEntry {
    GroupTargets,
    ActivePanes,
    Pane,
    BackToPanes,
}

impl PageEntry {
    fn queue(self, harness: &mut Harness) -> &'static str {
        if matches!(self, Self::GroupTargets) {
            let output = open_first_group(harness);
            let mode = entry(&output, "Dock Tab Group as Layout Tab…");
            harness.run(WIDTH, vec![action(mode, egui::accesskit::Action::Click)]);
            return TOOLS_DESTINATION;
        }
        let output = settle(harness);
        let anchor = nodes(&output)
            .iter()
            .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Layout actions"))
            .unwrap()
            .0;
        harness.run(WIDTH, vec![action(anchor, egui::accesskit::Action::Click)]);
        let output = settle(harness);
        let active = entry(&output, "Active Panes…");
        harness.run(WIDTH, vec![action(active, egui::accesskit::Action::Click)]);
        if matches!(self, Self::ActivePanes) {
            return "Editor";
        }
        let output = settle(harness);
        let pane = entry(&output, "Editor");
        harness.run(WIDTH, vec![action(pane, egui::accesskit::Action::Click)]);
        if matches!(self, Self::Pane) {
            return "Float Pane";
        }
        let output = settle(harness);
        let back = entry(&output, "Back to Active Panes");
        harness.run(WIDTH, vec![action(back, egui::accesskit::Action::Click)]);
        "Editor"
    }
}

struct RepeatPasses(usize);

impl egui::plugin::Plugin for RepeatPasses {
    fn debug_name(&self) -> &'static str {
        "menu-page-focus-repeated-passes"
    }

    fn output_hook(&mut self, _: &Context, output: &mut FullOutput) {
        if self.0 > 0 {
            self.0 -= 1;
            output
                .platform_output
                .request_discard_reasons
                .push(egui::RepaintCause::new_reason(
                    "repeat the pending menu-page focus request",
                ));
        }
    }
}

#[test]
fn entering_a_menu_page_preserves_a_newer_application_editors_focus() {
    for discards in [0, 2] {
        for transition in [
            PageEntry::GroupTargets,
            PageEntry::ActivePanes,
            PageEntry::Pane,
            PageEntry::BackToPanes,
        ] {
            let mut harness = setup();
            let expected = transition.queue(&mut harness);
            let anchor = harness.context.memory(egui::Memory::focused).unwrap();
            let version = harness.dockspace.version();
            harness.context.options_mut(|options| {
                options.max_passes = (discards + 1).try_into().unwrap();
            });
            harness.context.add_plugin(RepeatPasses(discards));
            let editor = egui::Id::unique("application-editor-during-menu-page-change");
            let mut text = String::new();
            let output = editor_frame(
                &mut harness,
                editor,
                &mut text,
                Some(editor),
                vec![Event::Text("first".into())],
            );
            assert_eq!(output.platform_output.num_completed_passes, discards + 1);
            entry(&output, expected);
            assert_eq!(text, "first");
            assert_eq!(
                harness.context.memory(egui::Memory::focused),
                Some(editor),
                "{transition:?}/{discards}: entering the delayed menu page must not reclaim a newer application focus"
            );
            editor_frame(
                &mut harness,
                editor,
                &mut text,
                None,
                vec![Event::Text(" second".into())],
            );
            assert_eq!(text, "first second");
            assert_eq!(harness.context.memory(egui::Memory::focused), Some(editor));
            editor_frame(&mut harness, editor, &mut text, Some(anchor), Vec::new());
            assert_eq!(
                harness.context.memory(egui::Memory::focused),
                Some(anchor),
                "a superseded page-entry request must not be deferred until the anchor regains focus"
            );
            assert_eq!(harness.dockspace.version(), version);
        }
    }
}
