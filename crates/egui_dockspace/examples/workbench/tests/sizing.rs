//! Optional pane sizing through the workbench's actual toolbar and owned run.

use super::*;
use egui::Context;
use egui::accesskit::Role;
use egui_dockspace::{CloseDecision, DockspaceCloseRequestStatus};

fn workbench() -> (Context, Dockspace, DemoPanes) {
    let context = editor::new_context();
    context.enable_accesskit();
    context.all_styles_mut(|style| style.animation_time = 0.0);
    let dock = Dockspace::builder("workbench-sizing-menu", demo_layout())
        .style(editor::dock_style())
        .build()
        .unwrap();
    (context, dock, DemoPanes::new())
}

fn settle(context: &Context, dock: &mut Dockspace, panes: &mut DemoPanes) -> egui::FullOutput {
    let _ = workbench_frame(context, dock, panes, Vec::new());
    workbench_frame_at(
        context,
        dock,
        panes,
        context.input(|input| input.time) + 1.0,
        Vec::new(),
    )
}

fn click(
    context: &Context,
    dock: &mut Dockspace,
    panes: &mut DemoPanes,
    position: Pos2,
) -> egui::FullOutput {
    for pressed in [true, false] {
        let _ = workbench_frame(
            context,
            dock,
            panes,
            vec![Event::PointerMoved(position), button(position, pressed)],
        );
    }
    settle(context, dock, panes)
}

fn toggle_sizing(
    context: &Context,
    dock: &mut Dockspace,
    panes: &mut DemoPanes,
) -> egui::FullOutput {
    let output = settle(context, dock, panes);
    let menu = semantic_rect(&output, Role::Button, "Sizing").center();
    let output = click(context, dock, panes, menu);
    let checkbox = semantic_rect(&output, Role::CheckBox, "Pane sizing demo").center();
    click(context, dock, panes, checkbox)
}

fn assert_extent(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.1,
        "the painted content extent is {actual}, expected {expected}",
    );
}

#[test]
fn the_default_workbench_keeps_its_original_pane_dimensions() {
    let (context, mut dock, mut panes) = workbench();
    let _ = settle(&context, &mut dock, &mut panes);

    assert!(panes.panes[&INSPECTOR].content.unwrap().width() > 260.0);
    assert!(panes.panes[&CONSOLE].content.unwrap().height() > 160.0);
    assert!(panes.panes[&OUTLINE].content.unwrap().width() > 250.0);
    assert_eq!(
        dock.view()
            .item(EDITOR)
            .unwrap()
            .tabs()
            .items()
            .collect::<Vec<_>>(),
        &[EDITOR, PREVIEW],
    );
    assert!(dock.view().item(INSPECTOR).unwrap().contained().is_some());
}

#[test]
fn the_sizing_menu_caps_real_panes_tracks_tab_selection_and_restores_normal_geometry() {
    let (context, mut dock, mut panes) = workbench();
    let _ = settle(&context, &mut dock, &mut panes);
    let items = [INSPECTOR, CONSOLE, OUTLINE, EDITOR];
    let original = items.map(|item| panes.panes[&item].content.unwrap());

    let output = toggle_sizing(&context, &mut dock, &mut panes);
    assert_extent(panes.panes[&INSPECTOR].content.unwrap().width(), 240.0);
    assert_extent(panes.panes[&CONSOLE].content.unwrap().height(), 120.0);
    assert_extent(
        panes.panes[&OUTLINE].content.unwrap().width(),
        original[2].width(),
    );

    let history = semantic_rect(&output, Role::Tab, "History").center();
    let output = click(&context, &mut dock, &mut panes, history);
    let selected = dock
        .view()
        .item(CONSOLE)
        .unwrap()
        .tabs()
        .selected()
        .and_then(|tab| tab.item())
        .unwrap();
    assert_ne!(selected, CONSOLE);
    assert!(
        panes.panes[&selected].content.unwrap().height() > 160.0,
        "selecting History replaces Console's fixed content height",
    );
    assert_extent(panes.panes[&INSPECTOR].content.unwrap().width(), 240.0);

    let console = semantic_rect(&output, Role::Tab, "Console").center();
    let _ = click(&context, &mut dock, &mut panes, console);
    assert!(dock.view().item(CONSOLE).unwrap().is_selected());
    assert_extent(panes.panes[&CONSOLE].content.unwrap().height(), 120.0);

    let _ = toggle_sizing(&context, &mut dock, &mut panes);
    for (item, expected) in items.into_iter().zip(original) {
        let actual = panes.panes[&item].content.unwrap();
        assert_extent(actual.left(), expected.left());
        assert_extent(actual.top(), expected.top());
        assert_extent(actual.width(), expected.width());
        assert_extent(actual.height(), expected.height());
    }
}

#[test]
fn double_clicking_the_outline_divider_restores_its_declared_preferred_width() {
    let (context, mut dock, mut panes) = workbench();
    let output = toggle_sizing(&context, &mut dock, &mut panes);
    let outline = panes.panes[&OUTLINE].content.unwrap();
    let editor = panes.panes[&EDITOR].content.unwrap();
    assert!((outline.width() - 220.0).abs() > 20.0);
    let position = egui::pos2((outline.right() + editor.left()) * 0.5, outline.center().y);
    assert!(
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| {
                node.role() == Role::Splitter
                    && !node.is_disabled()
                    && node.bounds().is_some_and(|bounds| {
                        bounds.x0 <= f64::from(position.x)
                            && f64::from(position.x) <= bounds.x1
                            && bounds.y0 <= f64::from(position.y)
                            && f64::from(position.y) <= bounds.y1
                    })
            }),
        "the demo's Outline divider must publish an operable real splitter",
    );

    let _ = workbench_frame(
        &context,
        &mut dock,
        &mut panes,
        vec![Event::PointerMoved(position)],
    );
    for _ in 0..2 {
        for pressed in [true, false] {
            let _ = workbench_frame(
                &context,
                &mut dock,
                &mut panes,
                vec![button(position, pressed)],
            );
        }
    }
    let _ = settle(&context, &mut dock, &mut panes);
    assert_extent(panes.panes[&OUTLINE].content.unwrap().width(), 220.0);
}

#[test]
fn enabling_sizing_in_the_empty_editor_applies_after_restoring_all_panes() {
    let (context, mut dock, mut panes) = workbench();
    let _ = settle(&context, &mut dock, &mut panes);
    panes
        .panes
        .get_mut(&EDITOR)
        .unwrap()
        .text
        .push_str("\n// retained");
    let edited = panes.panes[&EDITOR].text.clone();
    for item in panes.panes.keys() {
        let request = dock.request_close_item_current(*item).unwrap();
        let DockspaceCloseRequestStatus::Requested(request) = request.status() else {
            panic!("closing an open demo pane produces a close plan");
        };
        let plan = request.plan();
        for item in plan.items() {
            let _ = dock
                .resolve_close(plan.request(), item.token(), CloseDecision::Allow)
                .unwrap();
        }
    }
    assert!(dock.view().surfaces().next().is_none());

    let output = toggle_sizing(&context, &mut dock, &mut panes);
    let restore = semantic_rect(&output, Role::Button, "Restore layout").center();
    let _ = click(&context, &mut dock, &mut panes, restore);
    assert!(
        panes
            .panes
            .keys()
            .all(|item| dock.view().item(*item).is_some())
    );
    assert_eq!(panes.panes[&EDITOR].text, edited);
    assert_extent(panes.panes[&INSPECTOR].content.unwrap().width(), 240.0);
    assert_extent(panes.panes[&CONSOLE].content.unwrap().height(), 120.0);
}
