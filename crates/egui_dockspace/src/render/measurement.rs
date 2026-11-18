//! Egui-owned measurement facts for one renderer-neutral host frame.

#[cfg(feature = "native-render-support")]
use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use dockspace::geometry::PaneSizing;
use dockspace::model::{ItemId, SurfaceId, TabKey, TabLabel};
use dockspace::policy::TabBarVisibility;
use dockspace::runtime::{
    DockspaceHostFrame, DockspaceRuntimeError, DockspaceVisualId, SurfaceMeasurementAnswer,
    SurfaceMeasurementRequest, SurfaceWork, TabListMenuMetrics, TabStripControlMetric,
    TabStripControlMetrics, TabStripControlPlacement, TabStripMetrics,
};
use egui::{FontSelection, Galley, TextStyle, TextWrapMode, Ui};

use crate::error::DockspaceError;
use crate::error_detail::DockspaceErrorSource;
use crate::pane::PaneView;
use crate::style::DockStyle;
use crate::tab::{TabBarId, galley_for_width};

use super::geometry::logical_rect;

#[derive(Clone)]
pub(crate) struct TabPaintResource {
    pub(crate) title: String,
    pub(crate) galley: Arc<Galley>,
    pub(crate) missing: bool,
    pub(crate) content_width: f32,
}

impl TabPaintResource {
    pub(crate) fn galley_for_width(&self, ui: &Ui, width: f32) -> Arc<Galley> {
        galley_for_width(ui, &self.galley, width)
    }
}

#[derive(Default)]
pub(crate) struct PaintResources {
    #[cfg(feature = "native-render-support")]
    tab_visuals: OnceCell<HashMap<TabKey, DockspaceVisualId>>,
    tabs: HashMap<TabKey, TabPaintResource>,
    missing: BTreeSet<ItemId>,
}

impl PaintResources {
    pub(crate) fn prepare_for_plan(
        &mut self,
        plan: dockspace::runtime::SurfacePaintPlan<'_>,
        ui: &Ui,
        panes: &dyn PaneView,
    ) -> Result<(), DockspaceRuntimeError> {
        plan.visit_tab_resources(|_, key| {
            if let Some(label) = plan.tab_label(key) {
                self.get_or_shape(key, label, ui, panes);
            }
        })?;
        plan.visit_surface_program(|step| {
            if let dockspace::runtime::SurfacePaintProgramStep::Operation {
                operation: dockspace::runtime::SurfacePaintProgramOperation::Pane(pane),
                ..
            } = step
            {
                self.get_or_shape(pane.item().into(), TabLabel::Item(pane.item()), ui, panes);
            }
            Ok::<(), DockspaceRuntimeError>(())
        })
        .map_err(|error| error.into_runtime_error())?;
        Ok(())
    }

    fn get_or_shape(
        &mut self,
        key: TabKey,
        label: TabLabel<'_>,
        ui: &Ui,
        panes: &dyn PaneView,
    ) -> &TabPaintResource {
        self.tabs.entry(key).or_insert_with(|| {
            let resource = shape_tab(ui, panes, key, label);
            if resource.missing
                && let Some(item) = key.item()
            {
                self.missing.insert(item);
            }
            resource
        })
    }

    #[cfg(feature = "native-render-support")]
    pub(crate) fn tab_visual(
        &self,
        plan: dockspace::runtime::SurfacePaintPlan<'_>,
        key: TabKey,
    ) -> Option<DockspaceVisualId> {
        if self.tab_visuals.get().is_none() {
            // Only native diagnostics materialize this exact program map.
            let mut visuals = HashMap::new();
            plan.visit_tab_resources(|visual, key| {
                if visual.kind() == dockspace::runtime::DockspaceVisualKind::Tab {
                    visuals.entry(key).or_insert(visual);
                }
            })
            .ok()?;
            let _ = self.tab_visuals.set(visuals);
        }
        self.tab_visuals.get()?.get(&key).copied()
    }

    pub(crate) fn missing_items(&self) -> impl Iterator<Item = ItemId> + '_ {
        self.missing.iter().copied()
    }

    pub(crate) fn is_missing(&self, item: ItemId) -> bool {
        self.missing.contains(&item)
    }

    pub(crate) fn tab(&self, key: impl Into<TabKey>) -> Option<&TabPaintResource> {
        self.tabs.get(&key.into())
    }
}

struct SurfaceMeasurementContext<'a> {
    ui: &'a Ui,
    panes: &'a dyn PaneView,
    style: &'a DockStyle,
    dock_bounds: dockspace::geometry::LogicalRect,
    popup_bounds: dockspace::geometry::LogicalRect,
    row_heights: HashMap<DockspaceVisualId, f64>,
    resources: Option<&'a mut PaintResources>,
    missing: BTreeSet<ItemId>,
    horizontal_padding: f32,
    vertical_padding: f32,
    row_spacing: f32,
    scrollbar_extent: f32,
}

impl<'a> SurfaceMeasurementContext<'a> {
    fn new(
        ui: &'a Ui,
        dock_rect: egui::Rect,
        popup_rect: egui::Rect,
        panes: &'a dyn PaneView,
        style: &'a DockStyle,
        resources: Option<&'a mut PaintResources>,
    ) -> Result<Self, DockspaceError> {
        let dock_bounds =
            logical_rect(dock_rect).map_err(|()| DockspaceErrorSource::InvalidMeasurement {
                what: "dock bounds",
            })?;
        let popup_bounds =
            logical_rect(popup_rect).map_err(|()| DockspaceErrorSource::InvalidMeasurement {
                what: "popup plane bounds",
            })?;
        let popup_style = ui.style();
        let popup_spacing = &popup_style.spacing;
        let popup_frame = style.resolved_popup_frame(popup_style);
        let popup_stroke = popup_frame.stroke.width.max(0.0);
        let horizontal_padding = f32::from(
            i8::max(
                popup_frame.inner_margin.left,
                popup_frame.inner_margin.right,
            )
            .max(0),
        ) + popup_stroke;
        let vertical_padding = f32::from(
            i8::max(
                popup_frame.inner_margin.top,
                popup_frame.inner_margin.bottom,
            )
            .max(0),
        ) + popup_stroke;
        Ok(Self {
            ui,
            panes,
            style,
            dock_bounds,
            popup_bounds,
            row_heights: HashMap::new(),
            resources,
            missing: BTreeSet::new(),
            horizontal_padding,
            vertical_padding,
            row_spacing: popup_spacing.item_spacing.y.max(0.0),
            scrollbar_extent: popup_spacing.scroll.allocated_width().max(0.0),
        })
    }

    fn resolve(
        &mut self,
        request: SurfaceMeasurementRequest,
    ) -> Result<SurfaceMeasurementAnswer, DockspaceError> {
        let Self {
            ui,
            panes,
            style,
            dock_bounds,
            popup_bounds,
            row_heights,
            resources,
            missing,
            horizontal_padding,
            vertical_padding,
            row_spacing,
            scrollbar_extent,
        } = self;
        let (ui, panes, style) = (*ui, *panes, *style);
        let resources = resources.as_deref_mut();
        let (dock_bounds, popup_bounds) = (*dock_bounds, *popup_bounds);
        let (horizontal_padding, vertical_padding) = (*horizontal_padding, *vertical_padding);
        let (row_spacing, scrollbar_extent) = (*row_spacing, *scrollbar_extent);
        match request {
            SurfaceMeasurementRequest::DockBounds { .. } => {
                Ok(SurfaceMeasurementAnswer::Bounds(dock_bounds))
            }
            SurfaceMeasurementRequest::PopupPlaneBounds { .. } => {
                Ok(SurfaceMeasurementAnswer::Bounds(popup_bounds))
            }
            SurfaceMeasurementRequest::PaneSizing { item, .. } => {
                let sizing = item.map_or_else(PaneSizing::default, |item| panes.sizing(item));
                Ok(SurfaceMeasurementAnswer::PaneSizing(sizing))
            }
            SurfaceMeasurementRequest::TabIntrinsic {
                visual: _,
                bar,
                key,
                label,
            } => {
                let uncached;
                let resource = if let Some(resources) = resources {
                    resources.get_or_shape(key, label, ui, panes)
                } else {
                    // Independent native measurement has no same-pass paint owner.
                    uncached = shape_tab(ui, panes, key, label);
                    &uncached
                };
                if resource.missing
                    && let Some(item) = key.item()
                {
                    missing.insert(item);
                }
                let row_height = f64::from(
                    ui.spacing()
                        .interact_size
                        .y
                        .max(resource.galley.size().y + ui.spacing().button_padding.y * 2.0),
                );
                row_heights
                    .entry(bar)
                    .and_modify(|current| *current = current.max(row_height))
                    .or_insert(row_height);
                Ok(SurfaceMeasurementAnswer::TabIntrinsic(f64::from(
                    resource.content_width,
                )))
            }
            SurfaceMeasurementRequest::TabStrip { visual, visibility } => {
                let layout = if visibility == TabBarVisibility::Visible {
                    panes.tab_bar_layout(TabBarId(visual), ui)
                } else {
                    crate::TabBarLayout::default()
                };
                let mut metrics = TabStripMetrics::new(
                    f64::from(layout.leading_width),
                    f64::from(layout.trailing_width),
                )
                .and_then(|metrics| {
                    metrics.with_after_tabs_width(f64::from(layout.after_tabs_width))
                })
                .map_err(|_| DockspaceErrorSource::InvalidMeasurement { what: "tab strip" })?;
                if visibility == TabBarVisibility::Visible {
                    let extent = f64::from(style.tab_bar_height);
                    let controls = TabStripControlMetrics::new(0.0)
                        .and_then(|controls| {
                            Ok(controls
                                .with_scroll_backward(TabStripControlMetric::new(
                                    extent,
                                    TabStripControlPlacement::OverlayLeading,
                                )?)
                                .with_scroll_forward(TabStripControlMetric::new(
                                    extent,
                                    TabStripControlPlacement::OverlayTrailing,
                                )?)
                                .with_tab_list_menu(TabStripControlMetric::new(
                                    extent,
                                    TabStripControlPlacement::ReservedTrailing,
                                )?))
                        })
                        .map_err(|_| DockspaceErrorSource::InvalidMeasurement {
                            what: "tab strip controls",
                        })?;
                    let menu = TabListMenuMetrics::new(
                        row_heights.get(&visual).copied().unwrap_or_default(),
                        f64::from(horizontal_padding),
                        f64::from(vertical_padding),
                        f64::from(row_spacing),
                        popup_bounds.height(),
                        f64::from(scrollbar_extent),
                    )
                    .map_err(|_| DockspaceErrorSource::InvalidMeasurement {
                        what: "tab list menu",
                    })?;
                    metrics = metrics.with_controls(controls).with_tab_list_menu(menu);
                }
                Ok(SurfaceMeasurementAnswer::TabStrip(metrics))
            }
        }
    }

    fn missing(self) -> BTreeSet<ItemId> {
        self.missing
    }
}

#[cfg(feature = "native-render-support")]
pub(crate) fn measure_surface(
    frame: &mut DockspaceHostFrame<'_>,
    surface: SurfaceId,
    ui: &Ui,
    dock_rect: egui::Rect,
    popup_rect: egui::Rect,
    panes: &dyn PaneView,
    style: &DockStyle,
    resources: Option<&mut PaintResources>,
) -> Result<BTreeSet<ItemId>, DockspaceError> {
    let mut measurement =
        SurfaceMeasurementContext::new(ui, dock_rect, popup_rect, panes, style, resources)?;
    frame.measure_surface_with(surface, |request| measurement.resolve(request))?;
    Ok(measurement.missing())
}

pub(crate) fn prepare_surface<'frame>(
    frame: &'frame mut DockspaceHostFrame<'_>,
    surface: SurfaceId,
    ui: &Ui,
    dock_rect: egui::Rect,
    popup_rect: egui::Rect,
    panes: &dyn PaneView,
    style: &DockStyle,
    resources: Option<&mut PaintResources>,
) -> Result<(SurfaceWork<'frame>, BTreeSet<ItemId>), DockspaceError> {
    let mut measurement = None;
    let mut resources = resources;
    let work = frame.prepare_surface_with(surface, |request| {
        let measurement = match measurement.as_mut() {
            Some(measurement) => measurement,
            None => measurement.insert(SurfaceMeasurementContext::new(
                ui,
                dock_rect,
                popup_rect,
                panes,
                style,
                resources.take(),
            )?),
        };
        measurement.resolve(request)
    })?;
    Ok((
        work,
        measurement.map_or_else(BTreeSet::new, SurfaceMeasurementContext::missing),
    ))
}

fn shape_tab(ui: &Ui, panes: &dyn PaneView, key: TabKey, label: TabLabel<'_>) -> TabPaintResource {
    let (widget_text, missing) = match label {
        TabLabel::Item(item) => match panes.title(item) {
            Some(title) => (title, false),
            None => (format!("Missing pane {}", item.get()).into(), true),
        },
        TabLabel::Layout { title } => (panes.layout_tab_title(key, title), false),
    };
    let title = widget_text.text().to_owned();
    let galley = widget_text.into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        f32::INFINITY,
        FontSelection::Style(TextStyle::Button),
    );
    TabPaintResource {
        content_width: if missing {
            galley.size().x
        } else {
            panes.tab_content_width(key, ui, &galley)
        },
        title,
        galley,
        missing,
    }
}
