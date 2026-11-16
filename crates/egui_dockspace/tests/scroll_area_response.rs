//! Scroll-container registrations stay exact and local to the current pass.

use egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use egui::{Context, Id, Pos2, RawInput, Rect, ScrollArea, Ui, vec2};
use egui_dockspace::PaneResponse;

fn frame(context: &Context, time: f64, show: impl FnMut(&mut Ui)) {
    let mut output = context.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(360.0, 260.0))),
            time: Some(time),
            ..RawInput::default()
        },
        show,
    );
    output.textures_delta.clear();
}

fn context() -> Context {
    let context = Context::default();
    context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    context.all_styles_mut(|style| style.animation_time = 0.0);
    context
}

#[test]
fn scroll_area_response_registers_only_its_current_scroll_controls() {
    for axes in [[true, false], [false, true], [true, true]] {
        let context = context();
        for pass in 0..3 {
            frame(&context, f64::from(pass), |ui| {
                let sibling = ui.button("Unrelated control");
                let scroll = ScrollArea::new(axes)
                    .id_salt("document")
                    .max_width(280.0)
                    .max_height(180.0)
                    .auto_shrink([false, false])
                    .scroll_source(ScrollSource::ALL)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        let button = ui.button("Explicit content control");
                        ui.set_min_size(vec2(900.0, 700.0));
                        button
                    });
                if pass == 0 {
                    return;
                }

                let content = PaneResponse::default().with_scroll_area(ui, &scroll);
                let registered: Vec<_> = content.content_widgets().collect();
                let mut expected = vec![scroll.id.with("area")];
                for (axis, enabled) in axes.into_iter().enumerate() {
                    if enabled {
                        expected.push(scroll.id.with(axis));
                    }
                }
                assert_eq!(registered.len(), expected.len(), "axes: {axes:?}");
                for id in expected {
                    assert!(registered.contains(&id), "missing {id:?}, axes: {axes:?}");
                }
                assert!(!registered.contains(&sibling.id));
                assert!(!registered.contains(&scroll.inner.id));

                let content = content.with_content(&scroll.inner);
                assert!(content.content_widgets().any(|id| id == scroll.inner.id));
            });
        }
    }
}

#[test]
fn scroll_area_response_excludes_scroll_controls_from_previous_pass() {
    let context = context();
    let mut scroll_id = Id::NULL;
    for pass in 0..2 {
        frame(&context, f64::from(pass), |ui| {
            let scroll = ScrollArea::both()
                .id_salt("document")
                .auto_shrink([false, false])
                .scroll_source(ScrollSource::ALL)
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| ui.set_min_size(vec2(900.0, 700.0)));
            scroll_id = scroll.id;
        });
    }

    frame(&context, 2.0, |ui| {
        let scroll = ScrollArea::both()
            .id_salt("document")
            .auto_shrink([false, false])
            .scroll_source(ScrollSource::NONE)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| ui.set_min_size(vec2(900.0, 700.0)));
        assert_eq!(scroll.id, scroll_id);
        for id in [
            scroll.id.with("area"),
            scroll.id.with(0_usize),
            scroll.id.with(1_usize),
        ] {
            assert!(
                ui.ctx().read_response(id).is_some(),
                "previous-pass fixture must contain {id:?}"
            );
            assert!(
                ui.ctx()
                    .viewport(|viewport| viewport.this_pass.widgets.get(id).is_none())
            );
        }

        let content = PaneResponse::default().with_scroll_area(ui, &scroll);
        assert_eq!(content.content_widgets().count(), 0);
    });
}
