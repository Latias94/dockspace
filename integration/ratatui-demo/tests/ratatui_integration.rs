//! Headless visual & mouse-driven integration tests for `dockspace-ratatui-demo`
//! running on `ratatui::backend::TestBackend`.

use std::collections::BTreeMap;

use dockspace::geometry::{LogicalPoint, LogicalRect};
use dockspace::host::{DEFAULT_MOUSE_POINTER, SurfaceDisplayList};
use dockspace::model::{
    DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, FloatingPresentationId, ItemId, RootId,
};
use dockspace::runtime::{DockspaceDropDirection, SurfacePresentationResult, TabStripControlKind};
use dockspace_ratatui_demo::{
    DemoPane, MAIN_SURFACE, RatatuiDockApp, logical_point_to_cell, split_status_bar,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

fn rect_center(rect: LogicalRect) -> LogicalPoint {
    LogicalPoint::new(
        rect.x() + rect.width() * 0.5,
        rect.y() + rect.height() * 0.5,
    )
    .unwrap()
}

fn mouse(kind: MouseEventKind, point: LogicalPoint, modifiers: KeyModifiers) -> MouseEvent {
    let (column, row) = logical_point_to_cell(point);
    MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }
}

fn render_and_settle(
    terminal: &mut Terminal<TestBackend>,
    app: &mut RatatuiDockApp,
) -> (SurfaceDisplayList, String) {
    let area = terminal.get_frame().area();
    let (dock_area, _) = split_status_bar(area);
    let (display_list, token) = app
        .prepare_frame(dock_area)
        .unwrap()
        .expect("surface is paintable");
    terminal
        .draw(|frame| app.draw(frame, &display_list))
        .unwrap();
    app.finish_frame(token, SurfacePresentationResult::Presented)
        .unwrap();
    let buf = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            text.push_str(buf[(x, y)].symbol());
        }
        text.push('\n');
    }
    (display_list, text)
}

#[test]
fn ratatui_renders_workspace_switches_tabs_and_blocks_modal_clicks() {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut app = RatatuiDockApp::demo().unwrap();

    let (dl, screen) = render_and_settle(&mut terminal, &mut app);
    assert!(screen.contains("editor.rs"));
    assert!(screen.contains("Cargo.toml"));
    assert!(screen.contains("Terminal"));
    assert!(
        screen.contains('│'),
        "horizontal splitter draws vertical bar"
    );
    assert!(screen.contains("dockspace + ratatui"));
    assert!(
        !screen.contains("[↓]") && !screen.contains("[●]"),
        "idle screen must not render drop guides until an active drag begins"
    );

    // Locate Cargo.toml tab from the owned display list.
    let manifest_tab = dl
        .tabs()
        .find(|tab| tab.item == Some(ItemId::new(2)))
        .expect("Cargo.toml tab visible");
    let tab_pt = rect_center(manifest_tab.visible_bounds);

    // Open modal overlay covering top-center; a click inside modal must fail closed.
    app.toggle_modal();
    let (_, modal_screen) = render_and_settle(&mut terminal, &mut app);
    assert!(modal_screen.contains("Modal (PointerClearance::Blocked)"));

    // Click inside modal region: dockspace must ignore it.
    let modal_rect = app
        .modal_rect(split_status_bar(terminal.get_frame().area()).0)
        .unwrap();
    let covered_pt = dockspace_ratatui_demo::cell_to_logical_point(modal_rect.x + 2, modal_rect.y);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        covered_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        covered_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    assert!(app.host().view().item(ItemId::new(1)).unwrap().is_active());

    // Close modal and click Cargo.toml tab via real crossterm MouseEvent.
    app.toggle_modal();
    let _ = render_and_settle(&mut terminal, &mut app);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        tab_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        tab_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();

    let (_, after_screen) = render_and_settle(&mut terminal, &mut app);
    assert!(app.host().view().item(ItemId::new(2)).unwrap().is_active());
    assert!(
        after_screen.contains("Zero unsafe, zero egui/gpui assumptions"),
        "rendered terminal buffer must switch to Cargo.toml body"
    );
}

#[test]
fn ratatui_mouse_drag_renders_drop_guides_and_splits_workspace() {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut app = RatatuiDockApp::demo().unwrap();
    let (initial, _) = render_and_settle(&mut terminal, &mut app);
    assert_eq!(initial.splitters().count(), 1);

    // Drag Cargo.toml (Item 2) into the left pane and drop onto Bottom guide to create a new split.
    let tab_2 = initial
        .tabs()
        .find(|tab| tab.item == Some(ItemId::new(2)))
        .unwrap();
    let left_pane = initial
        .panes()
        .find(|pane| pane.item == ItemId::new(1))
        .unwrap();
    let start = rect_center(tab_2.drag_bounds);
    let pane_mid = rect_center(left_pane.content_geometry.layout_bounds());

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        start,
        KeyModifiers::NONE,
    ))
    .unwrap();
    // Like the real event loop, redraw after press settles tab selection onto the new scene.
    let _ = render_and_settle(&mut terminal, &mut app);
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        pane_mid,
        KeyModifiers::NONE,
    ))
    .unwrap();
    assert!(app.host().has_active_manipulation(DEFAULT_MOUSE_POINTER));

    let (during_drag, drag_screen) = render_and_settle(&mut terminal, &mut app);
    assert!(drag_screen.contains("[DRAGGING]"));
    assert!(
        drag_screen.contains("[↓]") && drag_screen.contains("[←]") && drag_screen.contains("[→]"),
        "active drag must render live drop guide badges into ratatui buffer\nscreen:\n{drag_screen}"
    );
    let bottom_target = during_drag
        .drop_guides()
        .flat_map(|guide| guide.active_targets().iter())
        .find(|target| target.direction == DockspaceDropDirection::Bottom)
        .expect("live Bottom drop guide affordance target");
    let drop_pt = rect_center(bottom_target.draw_bounds);

    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        drop_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    let _ = render_and_settle(&mut terminal, &mut app);
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        drop_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    assert!(!app.host().has_active_manipulation(DEFAULT_MOUSE_POINTER));

    let (after_drop, final_screen) = render_and_settle(&mut terminal, &mut app);
    assert_eq!(after_drop.panes().count(), 3);
    assert_eq!(after_drop.splitters().count(), 2);
    assert!(
        final_screen.contains('─'),
        "vertical split draws horizontal splitter"
    );
    assert!(final_screen.contains("dockspace + ratatui"));
    assert!(final_screen.contains("Zero unsafe, zero egui/gpui assumptions"));
}

#[test]
fn ratatui_splitter_drag_resizes_panes_in_terminal_buffer() {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut app = RatatuiDockApp::demo().unwrap();
    let (before, _) = render_and_settle(&mut terminal, &mut app);

    let splitter = before.splitters().next().unwrap();
    let left_before = before
        .panes()
        .find(|pane| pane.item == ItemId::new(1))
        .unwrap()
        .content_geometry
        .layout_bounds()
        .width();
    let start = rect_center(splitter.hit_bounds);
    let moved = LogicalPoint::new(start.x() - 200.0, start.y()).unwrap();

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        start,
        KeyModifiers::NONE,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        moved,
        KeyModifiers::NONE,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        moved,
        KeyModifiers::NONE,
    ))
    .unwrap();

    let (after, _) = render_and_settle(&mut terminal, &mut app);
    let left_after = after
        .panes()
        .find(|pane| pane.item == ItemId::new(1))
        .unwrap()
        .content_geometry
        .layout_bounds()
        .width();
    assert!(
        left_after < left_before - 100.0,
        "left pane must shrink after dragging splitter left ({left_after} vs {left_before})"
    );
}

#[test]
fn ratatui_overflow_strip_wheel_scrolls_and_popup_menu_activates_hidden_tab() {
    let items: Vec<ItemId> = (1..=8).map(ItemId::new).collect();
    let last_item = *items.last().unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        MAIN_SURFACE,
        DockspaceRootLayout::new(RootId::new(1), DockspaceNode::tabs(items.clone())),
    )])
    .unwrap();
    let mut panes = BTreeMap::new();
    for item in items {
        panes.insert(
            item,
            DemoPane {
                title: format!("file_{}.rs", item.get()),
                body: format!("Content of file_{}.rs", item.get()),
                accent: Color::Cyan,
            },
        );
    }

    let mut terminal = Terminal::new(TestBackend::new(48, 22)).unwrap();
    let mut app = RatatuiDockApp::from_layout(layout, panes).unwrap();
    let (initial, screen) = render_and_settle(&mut terminal, &mut app);
    assert!(
        screen.contains('▼'),
        "overflowing strip renders TabListMenu button\nscreen:\n{screen}"
    );
    assert!(initial.tab_bars().next().unwrap().maximum_scroll_offset > 0.0);

    // Wheel scroll directly over the first tab advances horizontal scroll offset.
    let first_tab_pt = rect_center(initial.tabs().next().unwrap().visible_bounds);
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        first_tab_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    let (scrolled, _) = render_and_settle(&mut terminal, &mut app);
    assert!(scrolled.tab_bars().next().unwrap().scroll_offset > 0.0);

    // Click the ▼ button to open the tab-list popup menu in the terminal buffer.
    let menu_btn = scrolled
        .tab_strip_controls()
        .find(|control| control.kind == TabStripControlKind::TabListMenu)
        .unwrap();
    let menu_pt = rect_center(menu_btn.hit_bounds);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        menu_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        menu_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();

    let (with_popup, popup_screen) = render_and_settle(&mut terminal, &mut app);
    assert!(popup_screen.contains("Tabs"), "popup menu border renders");
    let menu = with_popup.popup_menus().next().expect("popup menu open");
    let row =
        menu.rows
            .iter()
            .find(|row| row.item == Some(last_item) && row.interaction_bounds.is_some())
            .or_else(|| {
                menu.rows.iter().rev().find(|row| {
                    row.item != Some(ItemId::new(1)) && row.interaction_bounds.is_some()
                })
            })
            .expect("clickable popup row");
    let chosen = row.item.unwrap();
    let row_pt = rect_center(row.interaction_bounds.unwrap());
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        row_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        row_pt,
        KeyModifiers::NONE,
    ))
    .unwrap();

    let (_, after_pick) = render_and_settle(&mut terminal, &mut app);
    assert!(app.host().view().item(chosen).unwrap().is_active());
    assert!(after_pick.contains(&format!("Content of file_{}.rs", chosen.get())));
}

#[test]
fn ratatui_contained_floating_window_renders_and_drags_with_shift() {
    let initial_rect = LogicalRect::new(200.0, 60.0, 520.0, 200.0).unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        MAIN_SURFACE,
        DockspaceRootLayout::new(RootId::new(1), DockspaceNode::pane(ItemId::new(1))),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(RootId::new(2), DockspaceNode::tabs([ItemId::new(2)])),
        initial_rect,
    ))])
    .unwrap();
    let mut panes = BTreeMap::new();
    panes.insert(
        ItemId::new(1),
        DemoPane {
            title: "Main".into(),
            body: "Background workspace".into(),
            accent: Color::Cyan,
        },
    );
    panes.insert(
        ItemId::new(2),
        DemoPane {
            title: "Scratch".into(),
            body: "Floating scratchpad".into(),
            accent: Color::Yellow,
        },
    );

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut app = RatatuiDockApp::from_layout(layout, panes).unwrap();
    let (before, screen) = render_and_settle(&mut terminal, &mut app);
    assert!(screen.contains("Scratch"));
    assert!(screen.contains("Floating scratchpad"));

    let contained = before.contained_frames().next().unwrap();
    let start = rect_center(contained.title_drag_bounds);
    let moved = LogicalPoint::new(start.x() + 150.0, start.y() + 60.0).unwrap();

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        start,
        KeyModifiers::SHIFT,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        moved,
        KeyModifiers::SHIFT,
    ))
    .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        moved,
        KeyModifiers::SHIFT,
    ))
    .unwrap();

    let (after, _) = render_and_settle(&mut terminal, &mut app);
    let moved_frame = after.contained_frames().next().unwrap();
    assert!(
        moved_frame.outer_bounds.x() > initial_rect.x() + 80.0,
        "contained floating window must move right in terminal coordinates"
    );
}
