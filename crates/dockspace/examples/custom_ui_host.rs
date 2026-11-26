//! Minimal custom UI framework integration using `dockspace::host`.
//!
//! Demonstrates both integration styles with zero UI or windowing dependencies:
//! 1. Retained UI / deferred render thread via [`SurfaceDisplayList`].
//! 2. Immediate-mode renderer via [`SurfaceProgramVisitor`].
//! 3. Dynamic text width measurement via [`SurfaceMetrics::with_tab_width`].
//! 4. Mandatory presentation settlement via [`LogicalHost::finish_presentation`].
//! 5. Fail-closed pointer routing via [`PointerClearance`].
//!
//! Run from the repository root:
//! `cargo run -p dockspace --example custom_ui_host --locked`

use std::convert::Infallible;

use dockspace::geometry::{LogicalPoint, LogicalRect, LogicalSize};
use dockspace::host::{
    LogicalHost, LogicalPaintOutcome, LogicalPointerEvent, PointerClearance, SurfaceDisplayList,
    SurfaceDisplayStep, SurfaceMetrics, SurfaceProgramVisitor,
};
use dockspace::model::{
    DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    ItemId, RootId, SurfaceId, TabLabel,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    PanePaintRecord, SplitterPaintRecord, SurfacePaintDisposition, SurfacePaintProgramRoot,
    SurfacePointerButton, SurfacePointerEvent, SurfacePresentationResult, TabPaintRecord,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const EDITOR: ItemId = ItemId::new(1);
const TERMINAL: ItemId = ItemId::new(2);
const INSPECTOR: ItemId = ItemId::new(3);

fn title_for(label: TabLabel<'_>) -> String {
    match label {
        TabLabel::Item(EDITOR) => "Editor.rs".into(),
        TabLabel::Item(TERMINAL) => "Terminal".into(),
        TabLabel::Item(INSPECTOR) => "Inspector".into(),
        TabLabel::Item(other) => format!("Pane {}", other.get()),
        TabLabel::Layout { title } => title.to_owned(),
    }
}

fn rect_center(rect: LogicalRect) -> LogicalPoint {
    LogicalPoint::new(
        rect.x() + rect.width() * 0.5,
        rect.y() + rect.height() * 0.5,
    )
    .expect("finite rect center")
}

/// Immediate-mode renderer implementing only the primitives it draws.
#[derive(Default)]
struct ImmediatePrintRenderer {
    panes_drawn: usize,
    tabs_drawn: usize,
    splitters_drawn: usize,
}

impl SurfaceProgramVisitor for ImmediatePrintRenderer {
    type Error = Infallible;

    fn pane(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        pane: PanePaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        if !paint.is_source_gap() {
            self.panes_drawn += 1;
            println!(
                "  [immediate] pane {:?} clip={:?}",
                pane.item(),
                pane.content_geometry().clip()
            );
        }
        Ok(())
    }

    fn tab(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        tab: TabPaintRecord<'_>,
        paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        if !paint.is_source_gap() {
            self.tabs_drawn += 1;
            println!(
                "  [immediate] tab {:?} selected={} bounds={:?}",
                title_for(tab.label()),
                tab.selected(),
                tab.visible_bounds()
            );
        }
        Ok(())
    }

    fn splitter(
        &mut self,
        _root: SurfacePaintProgramRoot<'_>,
        splitter: SplitterPaintRecord<'_>,
        _paint: SurfacePaintDisposition,
    ) -> Result<(), Self::Error> {
        self.splitters_drawn += 1;
        println!(
            "  [immediate] splitter {:?} bounds={:?}",
            splitter.axis(),
            splitter.draw_bounds()
        );
        Ok(())
    }
}

fn paint_retained(
    host: &mut LogicalHost,
    bounds: LogicalRect,
) -> Result<SurfaceDisplayList, Box<dyn std::error::Error>> {
    let metrics = SurfaceMetrics::new(bounds)
        .with_pane_sizing(LogicalSize::new(80.0, 60.0)?)
        .with_default_tab_width(96.0)?;

    // Measure tabs using your framework's text shaper and obtain an owned `'static` display list.
    let (display_list, token) = match host.paint_display_list_with(
        SURFACE,
        metrics.with_tab_width(|label| title_for(label).len() as f64 * 9.0 + 16.0),
    )? {
        LogicalPaintOutcome::Ready {
            payload,
            presentation,
            ..
        } => (payload, presentation),
        LogicalPaintOutcome::Deferred { reason, .. } => {
            return Err(format!("surface paint deferred: {reason:?}").into());
        }
    };

    // Mount retained widgets or enqueue GPU commands using `display_list` with no host borrow.
    for step in display_list.steps() {
        if let SurfaceDisplayStep::Pane(pane) = step {
            println!(
                "  [retained] mount pane {:?} in {:?}",
                pane.item,
                pane.content_geometry.layout_bounds()
            );
        }
    }

    // Report actual presentation and commit settlement so pointer routing is authorized.
    host.finish_presentation(token, SurfacePresentationResult::Presented)?;
    Ok(display_list)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::central_tabs([EDITOR, TERMINAL]), 2.0),
                    (DockspaceNode::pane(INSPECTOR), 1.0),
                ],
            )?,
        ),
    )])?;

    let mut host = LogicalHost::from_layout(layout, DockPolicy::default())?;
    let bounds = LogicalRect::new(0.0, 0.0, 1024.0, 640.0)?;

    println!("1. Retained UI frame (owned SurfaceDisplayList):");
    let display_list = paint_retained(&mut host, bounds)?;
    let terminal_tab_center = display_list
        .tabs()
        .find(|tab| tab.item == Some(TERMINAL))
        .map(|tab| rect_center(tab.visible_bounds))
        .expect("Terminal tab is visible");

    println!("2. Blocked click (framework modal/overlay covers tab -> fails closed):");
    host.submit_pointer_batch([
        LogicalPointerEvent::mouse(
            SURFACE,
            Some(terminal_tab_center),
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            PointerClearance::Blocked,
        ),
        LogicalPointerEvent::mouse(
            SURFACE,
            Some(terminal_tab_center),
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            PointerClearance::Blocked,
        ),
    ])?;
    assert!(host.view().item(EDITOR).unwrap().is_active());

    println!("3. Unobstructed click activates Terminal tab:");
    host.submit_pointer_batch([
        LogicalPointerEvent::mouse(
            SURFACE,
            Some(terminal_tab_center),
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::mouse(
            SURFACE,
            Some(terminal_tab_center),
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
    ])?;
    assert!(host.view().item(TERMINAL).unwrap().is_active());

    println!("4. Immediate-mode frame (typed SurfaceProgramVisitor):");
    let mut metrics = SurfaceMetrics::new(bounds);
    let mut renderer = ImmediatePrintRenderer::default();
    let token = match host.paint_with_visitor(SURFACE, &mut metrics, &mut renderer)? {
        LogicalPaintOutcome::Ready { presentation, .. } => presentation,
        LogicalPaintOutcome::Deferred { reason, .. } => {
            return Err(format!("surface paint deferred: {reason:?}").into());
        }
    };
    host.finish_presentation(token, SurfacePresentationResult::Presented)?;
    assert_eq!(renderer.panes_drawn, 2);
    assert_eq!(renderer.tabs_drawn, 2);
    assert_eq!(renderer.splitters_drawn, 1);

    println!("custom_ui_host example completed successfully.");
    Ok(())
}
