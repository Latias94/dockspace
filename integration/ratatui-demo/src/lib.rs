//! Third-party UI framework integration demo pairing `dockspace::host` with `ratatui`.
//!
//! This crate intentionally uses ONLY the public `dockspace::host` integration kit
//! (plus `dockspace::{geometry, model, policy, runtime}` value types) to prove that
//! an external framework with a completely different rendering paradigm (character-cell
//! terminal buffers) can integrate full docking, splitting, resizing, tab scrolling,
//! popup menus, contained floating windows, and fail-closed occlusion in a compact
//! adapter with zero `unsafe` code.

use std::collections::BTreeMap;

use dockspace::geometry::{LogicalPoint, LogicalRect, LogicalSize};
use dockspace::host::{
    DEFAULT_MOUSE_POINTER, LogicalHost, LogicalPaintOutcome, LogicalPointerEvent,
    LogicalPresentationError, LogicalPresentationToken, OwnedContainedFrame, OwnedDragPreview,
    OwnedDropGuide, OwnedPopupMenu, OwnedTabLabel, PointerClearance, SurfaceDisplayList,
    SurfaceDisplayStep, SurfaceMetrics,
};
use dockspace::model::{
    DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    ItemId, RootId, SurfaceId, TabLabel,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockPresentationConfig, DockspaceDropDirection, DockspaceRuntimeError, SurfacePointerButton,
    SurfacePointerCancelReason, SurfacePointerCapture, SurfacePresentationResult,
    SurfaceScrollDelta, TabListMenuMetrics, TabStripControlKind, TabStripControlMetric,
    TabStripControlMetrics, TabStripControlPlacement, TabStripMetrics,
};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// Logical width allocated per terminal column.
pub const CELL_WIDTH: f64 = 20.0;
/// Logical height allocated per terminal row (matches `CELL_WIDTH` so 1-cell
/// borders, splitters, title bars, and tab bars align identically on both axes).
pub const CELL_HEIGHT: f64 = 20.0;

/// Primary logical surface hosted by [`RatatuiDockApp`].
pub const MAIN_SURFACE: SurfaceId = SurfaceId::new(1);

/// Demo application item descriptors rendered inside docked panes.
#[derive(Debug, Clone)]
pub struct DemoPane {
    /// Tab title shown in tab strips, popup menus, and drag previews.
    pub title: String,
    /// Multi-line body text rendered inside the pane content slot.
    pub body: String,
    /// Accent color used for the pane's active header/content badge.
    pub accent: Color,
}

/// Terminal-aligned `dockspace` presentation metrics where every chrome band
/// (tab bar, floating title bar, borders, splitters, drop guides) is an exact
/// integer multiple of one terminal cell (`20.0 x 20.0`).
#[must_use]
pub fn terminal_presentation_config() -> DockPresentationConfig {
    DockPresentationConfig::builder()
        .tab_bar_height(CELL_HEIGHT)
        .tab_group_grip_extent(CELL_WIDTH)
        .tab_horizontal_padding(CELL_WIDTH)
        .tab_min_width(CELL_WIDTH * 7.0)
        .tab_max_width(CELL_WIDTH * 24.0)
        .tab_close_extent(CELL_WIDTH)
        .splitter_thickness(CELL_WIDTH)
        .splitter_hit_extent(CELL_WIDTH)
        .pointer_drag_start_distance(CELL_WIDTH)
        .guide_extent(CELL_WIDTH * 3.0)
        .guide_gap(CELL_WIDTH)
        .guide_hit_padding(0.0)
        .guide_outer_inset(CELL_WIDTH * 3.0)
        .floating_title_height(CELL_HEIGHT)
        .floating_border_width(CELL_WIDTH)
        .floating_resize_extent(CELL_WIDTH)
        .minimum_pane_size(CELL_WIDTH * 8.0, CELL_HEIGHT * 4.0)
        .minimum_floating_size(CELL_WIDTH * 14.0, CELL_HEIGHT * 6.0)
        .build()
        .expect("terminal presentation config is valid")
}

/// Converts a terminal rectangle into `dockspace` logical coordinates.
#[must_use]
pub fn rect_to_logical(rect: Rect) -> LogicalRect {
    let width = (f64::from(rect.width.max(1))) * CELL_WIDTH;
    let height = (f64::from(rect.height.max(1))) * CELL_HEIGHT;
    LogicalRect::new(
        f64::from(rect.x) * CELL_WIDTH,
        f64::from(rect.y) * CELL_HEIGHT,
        width,
        height,
    )
    .expect("non-empty logical rect")
}

/// Converts a `dockspace` logical rectangle into a clamped terminal cell `Rect`.
#[must_use]
pub fn logical_to_rect(logical: LogicalRect, clip: Rect) -> Option<Rect> {
    let x0 = (logical.x() / CELL_WIDTH).round().max(f64::from(clip.x));
    let y0 = (logical.y() / CELL_HEIGHT).round().max(f64::from(clip.y));
    let x1 = ((logical.x() + logical.width()) / CELL_WIDTH)
        .round()
        .min(f64::from(clip.x.saturating_add(clip.width)));
    let y1 = ((logical.y() + logical.height()) / CELL_HEIGHT)
        .round()
        .min(f64::from(clip.y.saturating_add(clip.height)));
    let width = (x1 - x0).floor();
    let height = (y1 - y0).floor();
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Rect::new(x0 as u16, y0 as u16, width as u16, height as u16))
}

/// Converts a terminal cell coordinate `(column, row)` into the logical center of that cell.
#[must_use]
pub fn cell_to_logical_point(column: u16, row: u16) -> LogicalPoint {
    LogicalPoint::new(
        (f64::from(column) + 0.5) * CELL_WIDTH,
        (f64::from(row) + 0.5) * CELL_HEIGHT,
    )
    .expect("finite terminal cell center")
}

/// Converts a logical point back into the nearest terminal cell `(column, row)`.
#[must_use]
pub fn logical_point_to_cell(point: LogicalPoint) -> (u16, u16) {
    let col = (point.x() / CELL_WIDTH).floor().max(0.0) as u16;
    let row = (point.y() / CELL_HEIGHT).floor().max(0.0) as u16;
    (col, row)
}

/// Complete `ratatui` docking application powered by [`LogicalHost`].
pub struct RatatuiDockApp {
    host: LogicalHost,
    panes: BTreeMap<ItemId, DemoPane>,
    last_display_list: Option<SurfaceDisplayList>,
    last_dock_area: Rect,
    modal_open: bool,
    status_note: String,
}

impl RatatuiDockApp {
    /// Builds a default 4-pane IDE-style workspace (two left tabs + right split).
    ///
    /// # Errors
    ///
    /// Returns any `dockspace` layout or host initialization error.
    pub fn demo() -> Result<Self, DockspaceRuntimeError> {
        let editor = ItemId::new(1);
        let manifest = ItemId::new(2);
        let terminal = ItemId::new(3);
        let outline = ItemId::new(4);

        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            MAIN_SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::split(
                    DockspaceAxis::Horizontal,
                    [
                        (DockspaceNode::tabs([editor, manifest]), 2.0),
                        (DockspaceNode::tabs([terminal, outline]), 1.0),
                    ],
                )
                .expect("valid demo split"),
            ),
        )])
        .expect("valid demo layout");

        let mut panes = BTreeMap::new();
        panes.insert(
            editor,
            DemoPane {
                title: "editor.rs".into(),
                body: "fn main() {\n    println!(\"dockspace + ratatui\");\n}\n\nDrag any tab onto a pane to split or merge.".into(),
                accent: Color::Cyan,
            },
        );
        panes.insert(
            manifest,
            DemoPane {
                title: "Cargo.toml".into(),
                body: "[dependencies]\ndockspace = \"0.1\"\nratatui = \"0.30\"\n\nZero unsafe, zero egui/gpui assumptions.".into(),
                accent: Color::Yellow,
            },
        );
        panes.insert(
            terminal,
            DemoPane {
                title: "Terminal".into(),
                body: "$ cargo test\ntest result: ok.\n\nTry dragging the vertical splitter!"
                    .into(),
                accent: Color::Green,
            },
        );
        panes.insert(
            outline,
            DemoPane {
                title: "Outline".into(),
                body: "- RatatuiDockApp\n  - prepare_frame()\n  - render_display_list()\n  - handle_mouse()".into(),
                accent: Color::Magenta,
            },
        );

        Self::from_layout(layout, panes)
    }

    /// Creates a `ratatui` dock app from any custom layout and item registry.
    ///
    /// # Errors
    ///
    /// Returns any `dockspace` session initialization error.
    pub fn from_layout(
        layout: DockspaceLayout,
        panes: BTreeMap<ItemId, DemoPane>,
    ) -> Result<Self, DockspaceRuntimeError> {
        let host = LogicalHost::from_layout_with_presentation_config(
            layout,
            DockPolicy::default(),
            terminal_presentation_config(),
        )?;
        Ok(Self {
            host,
            panes,
            last_display_list: None,
            last_dock_area: Rect::default(),
            modal_open: false,
            status_note: "Click/drag tabs, drag splitters | [m] modal [r] reset [q/Ctrl+C] quit"
                .into(),
        })
    }

    /// Borrows the underlying [`LogicalHost`].
    #[must_use]
    pub const fn host(&self) -> &LogicalHost {
        &self.host
    }

    /// Mutably borrows the underlying [`LogicalHost`].
    #[must_use]
    pub fn host_mut(&mut self) -> &mut LogicalHost {
        &mut self.host
    }

    /// Returns the most recently painted owned [`SurfaceDisplayList`].
    #[must_use]
    pub fn last_display_list(&self) -> Option<&SurfaceDisplayList> {
        self.last_display_list.as_ref()
    }

    /// Returns whether the demo modal overlay is currently covering the center of the dock.
    #[must_use]
    pub const fn is_modal_open(&self) -> bool {
        self.modal_open
    }

    /// Toggles the demo modal overlay used to demonstrate `PointerClearance::Blocked`.
    pub fn toggle_modal(&mut self) {
        self.modal_open = !self.modal_open;
        self.status_note = if self.modal_open {
            "Modal OPEN: clicks inside modal pass PointerClearance::Blocked (fail-closed)".into()
        } else {
            "Modal closed: unobstructed pointer input resumed".into()
        };
    }

    /// Computes the centered modal rectangle when [`Self::is_modal_open`] is true.
    #[must_use]
    pub fn modal_rect(&self, dock_area: Rect) -> Option<Rect> {
        if !self.modal_open {
            return None;
        }
        let width = dock_area.width.min(44);
        let height = dock_area.height.min(7);
        if width == 0 || height == 0 {
            return None;
        }
        let x = dock_area.x + dock_area.width.saturating_sub(width) / 2;
        let y = dock_area.y;
        Some(Rect::new(x, y, width, height))
    }

    /// Measures and compiles the next owned [`SurfaceDisplayList`] for `dock_area`.
    ///
    /// Pass the returned [`LogicalPresentationToken`] to [`Self::finish_frame`]
    /// immediately after `terminal.draw(...)` presents (or drops) the frame.
    ///
    /// # Errors
    ///
    /// Returns any `dockspace` measurement or host-frame error.
    pub fn prepare_frame(
        &mut self,
        dock_area: Rect,
    ) -> Result<Option<(SurfaceDisplayList, LogicalPresentationToken)>, DockspaceRuntimeError> {
        self.last_dock_area = dock_area;
        let logical_bounds = rect_to_logical(dock_area);
        let button = TabStripControlMetric::new(
            CELL_WIDTH * 2.0,
            TabStripControlPlacement::ReservedTrailing,
        )
        .expect("positive cell button width");
        let controls = TabStripControlMetrics::new(0.0)
            .expect("zero control spacing")
            .with_scroll_backward(button)
            .with_scroll_forward(button)
            .with_tab_list_menu(button);
        let menu = TabListMenuMetrics::new(
            CELL_HEIGHT,
            CELL_WIDTH,
            CELL_HEIGHT,
            0.0,
            CELL_HEIGHT * 10.0,
            CELL_WIDTH,
        )
        .expect("cell-aligned tab list menu metrics");
        let tab_strip = TabStripMetrics::new(0.0, 0.0)
            .expect("zero strip reservation")
            .with_controls(controls)
            .with_tab_list_menu(menu);
        let metrics = SurfaceMetrics::new(logical_bounds)
            .with_pane_sizing(
                LogicalSize::new(CELL_WIDTH * 8.0, CELL_HEIGHT * 4.0)
                    .expect("valid terminal pane sizing"),
            )
            .with_tab_strip(tab_strip);

        let titles = &self.panes;
        match self.host.paint_display_list_with(
            MAIN_SURFACE,
            metrics.with_tab_width(|label| {
                let text = resolve_borrowed_label(titles, label);
                (text.chars().count().max(3) as f64) * CELL_WIDTH
            }),
        )? {
            LogicalPaintOutcome::Ready {
                payload,
                presentation,
                ..
            } => {
                self.last_display_list = Some(payload.clone());
                Ok(Some((payload, presentation)))
            }
            LogicalPaintOutcome::Deferred { .. } => Ok(None),
        }
    }

    /// Reports the actual terminal draw outcome and settles core receiver authority.
    ///
    /// # Errors
    ///
    /// Returns any presentation report or settlement commit error.
    pub fn finish_frame(
        &mut self,
        token: LogicalPresentationToken,
        result: SurfacePresentationResult,
    ) -> Result<(), LogicalPresentationError> {
        self.host.finish_presentation(token, result)?;
        Ok(())
    }

    /// Renders `display_list` plus optional modal and bottom status line into a `ratatui::Frame`.
    pub fn draw(&self, frame: &mut Frame<'_>, display_list: &SurfaceDisplayList) {
        let full = frame.area();
        let (dock_area, status_area) = split_status_bar(full);
        self.render_display_list(frame, dock_area, display_list);
        if let Some(modal) = self.modal_rect(dock_area) {
            frame.render_widget(Clear, modal);
            let modal_widget = Paragraph::new(
                "Framework Modal Overlay\nClicks here are blocked from dockspace.\nPress [m] to close.",
            )
            .block(
                Block::default()
                    .title(" Modal (PointerClearance::Blocked) ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightRed)),
            )
            .wrap(Wrap { trim: false });
            frame.render_widget(modal_widget, modal);
        }
        if let Some(status_area) = status_area {
            let drag_tag = if self.host.has_active_manipulation(DEFAULT_MOUSE_POINTER) {
                "[DRAGGING] "
            } else {
                ""
            };
            let line = format!("{drag_tag}{}", self.status_note);
            let bar = Paragraph::new(line).style(Style::default().fg(Color::Black).bg(Color::Gray));
            frame.render_widget(bar, status_area);
        }
    }

    /// Translates one `crossterm` mouse event into `dockspace::host` pointer/wheel input.
    ///
    /// # Errors
    ///
    /// Returns any `dockspace` pointer submission error.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> Result<(), DockspaceRuntimeError> {
        let dock_area = self.last_dock_area;
        let inside_dock = event.column >= dock_area.x
            && event.column < dock_area.x.saturating_add(dock_area.width)
            && event.row >= dock_area.y
            && event.row < dock_area.y.saturating_add(dock_area.height);
        let occluded_by_modal = self.modal_rect(dock_area).is_some_and(|modal| {
            event.column >= modal.x
                && event.column < modal.x.saturating_add(modal.width)
                && event.row >= modal.y
                && event.row < modal.y.saturating_add(modal.height)
        });
        let clearance = if occluded_by_modal {
            PointerClearance::Blocked
        } else if inside_dock {
            PointerClearance::Unobstructed
        } else {
            PointerClearance::Unknown
        };
        let point = cell_to_logical_point(event.column, event.row);
        let docking_enabled = !event.modifiers.contains(KeyModifiers::SHIFT);

        match event.kind {
            MouseEventKind::Down(button) => {
                if let Some(btn) = map_mouse_button(button) {
                    self.host.submit_pointer(
                        LogicalPointerEvent::pressed(MAIN_SURFACE, point, btn, clearance)
                            .with_capture(SurfacePointerCapture::ProviderEndpoint)
                            .with_docking_enabled(docking_enabled),
                    )?;
                }
            }
            MouseEventKind::Drag(_) => {
                self.host.submit_pointer(
                    LogicalPointerEvent::moved(MAIN_SURFACE, point, clearance)
                        .with_capture(SurfacePointerCapture::ProviderEndpoint)
                        .with_docking_enabled(docking_enabled),
                )?;
            }
            MouseEventKind::Up(button) => {
                if let Some(btn) = map_mouse_button(button) {
                    self.host.submit_pointer(
                        LogicalPointerEvent::released(MAIN_SURFACE, point, btn, clearance)
                            .with_docking_enabled(docking_enabled),
                    )?;
                }
            }
            MouseEventKind::Moved => {
                self.host.submit_pointer(
                    LogicalPointerEvent::moved(MAIN_SURFACE, point, clearance)
                        .with_docking_enabled(docking_enabled),
                )?;
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollRight => {
                self.host.submit_wheel(
                    MAIN_SURFACE,
                    point,
                    SurfaceScrollDelta::LogicalPoints {
                        x: -CELL_WIDTH * 4.0,
                        y: 0.0,
                    },
                    clearance,
                )?;
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft => {
                self.host.submit_wheel(
                    MAIN_SURFACE,
                    point,
                    SurfaceScrollDelta::LogicalPoints {
                        x: CELL_WIDTH * 4.0,
                        y: 0.0,
                    },
                    clearance,
                )?;
            }
        }
        Ok(())
    }

    /// Cancels any active pointer manipulation (for example when the user presses `Esc`).
    ///
    /// # Errors
    ///
    /// Returns any `dockspace` cancellation error.
    pub fn cancel_interaction(&mut self) -> Result<(), DockspaceRuntimeError> {
        self.host.cancel_pointer(
            DEFAULT_MOUSE_POINTER,
            MAIN_SURFACE,
            SurfacePointerCancelReason::ExplicitCancellation,
        )?;
        Ok(())
    }

    fn render_display_list(
        &self,
        frame: &mut Frame<'_>,
        dock_area: Rect,
        display_list: &SurfaceDisplayList,
    ) {
        for step in display_list.steps() {
            match step {
                SurfaceDisplayStep::Pane(pane) => {
                    if let Some(rect) = logical_to_rect(pane.content_geometry.clip(), dock_area) {
                        let (title, body, accent) = self.panes.get(&pane.item).map_or_else(
                            || {
                                (
                                    format!("Pane {}", pane.item.get()),
                                    String::new(),
                                    Color::White,
                                )
                            },
                            |demo| (demo.title.clone(), demo.body.clone(), demo.accent),
                        );
                        frame.render_widget(Clear, rect);
                        let widget = Paragraph::new(body)
                            .block(
                                Block::default()
                                    .title(format!(" {title} "))
                                    .borders(Borders::ALL)
                                    .border_style(Style::default().fg(accent)),
                            )
                            .wrap(Wrap { trim: false });
                        frame.render_widget(widget, rect);
                    }
                }
                SurfaceDisplayStep::TabBar(bar) => {
                    if let Some(rect) = logical_to_rect(bar.bounds, dock_area) {
                        fill_rect(
                            frame,
                            rect,
                            ' ',
                            Style::default().bg(Color::DarkGray).fg(Color::White),
                        );
                        if let Some(grip) = bar.group_grip_bounds
                            && let Some(grip_rect) = logical_to_rect(grip, rect)
                        {
                            write_clipped(
                                frame,
                                grip_rect,
                                "::",
                                Style::default().bg(Color::DarkGray).fg(Color::Gray),
                            );
                        }
                    }
                }
                SurfaceDisplayStep::Tab(tab) => {
                    if let Some(rect) = logical_to_rect(tab.visible_bounds, dock_area) {
                        let label = self.resolve_owned_label(&tab.label);
                        let style = if tab.selected {
                            Style::default()
                                .bg(Color::Blue)
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().bg(Color::Black).fg(Color::Gray)
                        };
                        fill_rect(frame, rect, ' ', style);
                        let text = if tab.close_visual_bounds.is_some() && rect.width >= 4 {
                            format!(" {label} ×")
                        } else {
                            format!(" {label}")
                        };
                        write_clipped(frame, rect, &text, style);
                    }
                }
                SurfaceDisplayStep::Splitter(splitter) => {
                    if let Some(rect) = logical_to_rect(splitter.draw_bounds, dock_area) {
                        let glyph = match splitter.axis {
                            DockspaceAxis::Horizontal => '│',
                            DockspaceAxis::Vertical => '─',
                        };
                        fill_rect(frame, rect, glyph, Style::default().fg(Color::Yellow));
                    }
                }
                SurfaceDisplayStep::TabStripControl(control) => {
                    if let Some(rect) = logical_to_rect(control.bounds, dock_area) {
                        let glyph = match control.kind {
                            TabStripControlKind::ScrollBackward => "<",
                            TabStripControlKind::ScrollForward => ">",
                            TabStripControlKind::TabListMenu => "▼",
                            TabStripControlKind::RevealTabBar => "≡",
                        };
                        let style = if control.enabled {
                            Style::default()
                                .bg(Color::Black)
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().bg(Color::DarkGray).fg(Color::Black)
                        };
                        fill_rect(frame, rect, ' ', style);
                        write_clipped(frame, rect, glyph, style);
                    }
                }
                SurfaceDisplayStep::ContainedControls {
                    frame: contained, ..
                } => {
                    self.render_contained_frame(frame, dock_area, contained);
                }
                SurfaceDisplayStep::PopupBackdrop(backdrop) => {
                    if let Some(rect) = logical_to_rect(backdrop.bounds, dock_area) {
                        let buf = frame.buffer_mut();
                        for y in rect.y..rect.y.saturating_add(rect.height) {
                            for x in rect.x..rect.x.saturating_add(rect.width) {
                                buf[(x, y)].set_style(Style::default().fg(Color::DarkGray));
                            }
                        }
                    }
                }
                SurfaceDisplayStep::PopupMenu(menu) => {
                    self.render_popup_menu(frame, dock_area, menu);
                }
                SurfaceDisplayStep::DragPreview(preview) => {
                    self.render_drag_preview(frame, dock_area, preview);
                }
                SurfaceDisplayStep::DropGuide(guide) => {
                    self.render_drop_guide(frame, dock_area, guide);
                }
                SurfaceDisplayStep::ContainedTransformPreview(preview) => {
                    if let Some(rect) = logical_to_rect(preview.rect, dock_area) {
                        frame.render_widget(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_style(Style::default().fg(Color::LightCyan)),
                            rect,
                        );
                    }
                }
                SurfaceDisplayStep::DragDecoration(_)
                | SurfaceDisplayStep::RootBegin { .. }
                | SurfaceDisplayStep::RootEnd(_)
                | SurfaceDisplayStep::Tabs(_)
                | SurfaceDisplayStep::SplitterJunction(_)
                | SurfaceDisplayStep::PresentationMenuAnchor(_)
                | SurfaceDisplayStep::DropTarget(_)
                | SurfaceDisplayStep::InteractionFeedback => {}
            }
        }
    }

    fn render_contained_frame(
        &self,
        frame: &mut Frame<'_>,
        dock_area: Rect,
        contained: &OwnedContainedFrame,
    ) {
        if let Some(outer) = logical_to_rect(contained.outer_bounds, dock_area) {
            let border_color = if contained.is_frontmost {
                Color::LightYellow
            } else {
                Color::Gray
            };
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
                outer,
            );
        }
        if let Some(title_rect) = logical_to_rect(contained.title_bounds, dock_area) {
            let title = self.resolve_owned_label(&contained.label);
            let style = Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD);
            fill_rect(frame, title_rect, ' ', style);
            write_clipped(frame, title_rect, &format!(" {title}"), style);
        }
    }

    fn render_popup_menu(&self, frame: &mut Frame<'_>, dock_area: Rect, menu: &OwnedPopupMenu) {
        let Some(menu_rect) = logical_to_rect(menu.bounds, dock_area) else {
            return;
        };
        frame.render_widget(Clear, menu_rect);
        frame.render_widget(
            Block::default()
                .title(" Tabs ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
            menu_rect,
        );
        for row in &menu.rows {
            let Some(row_bounds) = row.interaction_bounds else {
                continue;
            };
            let Some(row_rect) = logical_to_rect(row_bounds, menu_rect) else {
                continue;
            };
            let label = self.resolve_owned_label(&row.label);
            let prefix = if row.selected { "*" } else { " " };
            let style = if row.focused || row.selected {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Color::Black).fg(Color::White)
            };
            fill_rect(frame, row_rect, ' ', style);
            write_clipped(frame, row_rect, &format!("{prefix}{label}"), style);
        }
    }

    fn render_drag_preview(
        &self,
        frame: &mut Frame<'_>,
        dock_area: Rect,
        preview: &OwnedDragPreview,
    ) {
        if let Some((_, logical)) = preview.logical_rect()
            && let Some(rect) = logical_to_rect(logical, dock_area)
        {
            frame.render_widget(
                Block::default()
                    .title(" Drop Preview ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightGreen)),
                rect,
            );
        }
    }

    fn render_drop_guide(&self, frame: &mut Frame<'_>, dock_area: Rect, guide: &OwnedDropGuide) {
        for target in guide.active_targets() {
            let Some(rect) = logical_to_rect(target.draw_bounds, dock_area) else {
                continue;
            };
            let badge = match target.direction {
                DockspaceDropDirection::Left => "[←]",
                DockspaceDropDirection::Right => "[→]",
                DockspaceDropDirection::Top => "[↑]",
                DockspaceDropDirection::Bottom => "[↓]",
                DockspaceDropDirection::Center => "[●]",
            };
            let style = if target.is_active {
                Style::default()
                    .bg(Color::Green)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Color::DarkGray).fg(Color::LightCyan)
            };
            fill_rect(frame, rect, ' ', style);
            let label_row = Rect::new(rect.x, rect.y + rect.height / 2, rect.width, 1);
            write_clipped(frame, label_row, badge, style);
        }
    }

    fn resolve_owned_label(&self, label: &OwnedTabLabel) -> String {
        resolve_borrowed_label(&self.panes, label.as_borrowed())
    }
}

/// Reserves the bottom row for the status bar when height >= 2.
#[must_use]
pub fn split_status_bar(area: Rect) -> (Rect, Option<Rect>) {
    if area.height <= 1 {
        return (area, None);
    }
    let dock = Rect::new(area.x, area.y, area.width, area.height - 1);
    let status = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    (dock, Some(status))
}

fn resolve_borrowed_label(panes: &BTreeMap<ItemId, DemoPane>, label: TabLabel<'_>) -> String {
    match label {
        TabLabel::Item(item) => panes
            .get(&item)
            .map_or_else(|| format!("Tab {}", item.get()), |pane| pane.title.clone()),
        TabLabel::Layout { title } => title.to_owned(),
    }
}

fn map_mouse_button(button: MouseButton) -> Option<SurfacePointerButton> {
    match button {
        MouseButton::Left => Some(SurfacePointerButton::Primary),
        MouseButton::Right => Some(SurfacePointerButton::Secondary),
        MouseButton::Middle => Some(SurfacePointerButton::Middle),
    }
}

fn fill_rect(frame: &mut Frame<'_>, rect: Rect, symbol: char, style: Style) {
    let buf = frame.buffer_mut();
    let mut utf8 = [0u8; 4];
    let text = symbol.encode_utf8(&mut utf8);
    for y in rect.y..rect.y.saturating_add(rect.height) {
        for x in rect.x..rect.x.saturating_add(rect.width) {
            buf[(x, y)].set_symbol(text).set_style(style);
        }
    }
}

fn write_clipped(frame: &mut Frame<'_>, rect: Rect, text: &str, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let y = rect.y;
    for (offset, ch) in text.chars().take(usize::from(rect.width)).enumerate() {
        let x = rect.x + offset as u16;
        let mut utf8 = [0u8; 4];
        buf[(x, y)]
            .set_symbol(ch.encode_utf8(&mut utf8))
            .set_style(style);
    }
}
