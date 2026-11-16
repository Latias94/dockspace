//! Shared optional chrome allocation for live and candidate tab strips.

use super::*;
use crate::presentation::manifest::TabStripMetrics;

#[derive(Debug, Clone, Copy)]
pub(in crate::presentation::compiler) struct TabStripOptionalChromeLayout {
    pub(super) group_extent: f64,
    pub(super) leading_reserved_bounds: Option<LogicalRect>,
    pub(super) trailing_reserved_bounds: Option<LogicalRect>,
    pub(in crate::presentation::compiler) presentation_menu_bounds: Option<LogicalRect>,
    pub(super) group_menu_bounds: Option<LogicalRect>,
    pub(in crate::presentation::compiler) viewport: LogicalRect,
    pub(in crate::presentation::compiler) controls: Option<TabStripControlLayout>,
}

pub(in crate::presentation::compiler) fn tab_strip_optional_chrome_layout(
    bar: LogicalRect,
    item_count: usize,
    strip: TabStripMetrics,
    config: &DockPresentationConfig,
    include_presentation_menu: bool,
) -> Result<TabStripOptionalChromeLayout, SceneCompilationError> {
    let grip = if item_count == 0 {
        0.0
    } else {
        config.tab_group_grip_extent().min(bar.height())
    };
    // Drop optional chrome in reverse priority. The final pass preserves a
    // complete list target even when no minimum-width tab can fit beside it.
    for (group_extent, root_menu, group_menu, arrows, minimum_tab) in [
        (grip, include_presentation_menu, true, true, true),
        (grip, include_presentation_menu, true, false, true),
        (0.0, include_presentation_menu, true, false, true),
        (0.0, include_presentation_menu, false, false, true),
        (0.0, false, false, false, true),
        (0.0, include_presentation_menu, false, false, false),
        (0.0, false, false, false, false),
    ] {
        if let Some(layout) = allocate(
            bar,
            item_count,
            strip,
            config,
            group_extent,
            root_menu,
            group_menu && item_count > 0,
            arrows,
            minimum_tab,
        )? {
            return Ok(layout);
        }
    }
    Ok(TabStripOptionalChromeLayout {
        group_extent: 0.0,
        leading_reserved_bounds: None,
        trailing_reserved_bounds: None,
        presentation_menu_bounds: None,
        group_menu_bounds: None,
        viewport: LogicalRect::new(bar.x(), bar.y(), bar.width(), bar.height())?,
        controls: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn allocate(
    bar: LogicalRect,
    item_count: usize,
    strip: TabStripMetrics,
    config: &DockPresentationConfig,
    group_extent: f64,
    root_menu: bool,
    group_menu: bool,
    arrows: bool,
    require_tab: bool,
) -> Result<Option<TabStripOptionalChromeLayout>, SceneCompilationError> {
    let menu_extent = (bar.height() - 2.0).max(0.0);
    if (root_menu || group_menu) && (menu_extent < config.tab_close_extent() || menu_extent == 0.0)
    {
        return Ok(None);
    }
    let menu_count = u32::from(root_menu) + u32::from(group_menu);
    let menu_width = menu_extent * f64::from(menu_count);
    let right = bar.max().x() - menu_width;
    let left = (bar.x() + group_extent).min(bar.max().x());
    if right < left {
        return Ok(None);
    }
    let content = LogicalRect::from_min_max(
        LogicalPoint::new(left, bar.y())?,
        LogicalPoint::new(right, bar.max().y())?,
    )?;
    let count =
        u32::try_from(item_count).map_err(|_| SceneCompilationError::GeometryCountOverflow)?;
    let minimum_total = checked_sum(
        [
            config.tab_min_width() * f64::from(count),
            strip.after_tabs_width(),
        ],
        "minimum tab flow width",
    )?;
    let overflowing = item_count > 0
        && minimum_total + strip.leading_reserved() + strip.trailing_reserved() > content.width();
    let metrics = strip.controls().filter(|metrics| !metrics.is_empty());
    let metrics = if arrows {
        metrics
    } else {
        metrics.and_then(|metrics| {
            metrics.tab_list_menu().map(|menu| {
                TabStripControlMetrics::new(0.0)
                    .expect("zero spacing is valid")
                    .with_tab_list_menu(menu)
            })
        })
    };
    let controls = if overflowing && let Some(metrics) = metrics {
        let Some(controls) = tab_strip_control_layout(content, metrics)? else {
            return Ok(None);
        };
        Some(controls)
    } else {
        None
    };
    if !require_tab && item_count > 0 && controls.is_none_or(|controls| controls.rects[2].is_none())
    {
        return Ok(None);
    }
    let control_viewport = controls.map_or(content, |layout| layout.viewport);
    let minimum_tab = if require_tab && item_count > 0 {
        config.tab_min_width()
    } else {
        0.0
    };
    if control_viewport.width() < minimum_tab {
        return Ok(None);
    }
    // Application slots receive the remainder after required selection and
    // action entrances. They cannot displace a full list or menu target.
    let reservable = (control_viewport.width() - minimum_tab).max(0.0);
    let leading = strip.leading_reserved().min(reservable);
    let trailing = strip
        .trailing_reserved()
        .min((reservable - leading).max(0.0));
    let viewport = LogicalRect::from_min_max(
        LogicalPoint::new(control_viewport.x() + leading, bar.y())?,
        LogicalPoint::new(
            (control_viewport.max().x() - trailing).max(control_viewport.x() + leading),
            bar.max().y(),
        )?,
    )?;
    let bounds = |left, right| {
        LogicalRect::from_min_max(
            LogicalPoint::new(left, bar.y())?,
            LogicalPoint::new(right, bar.max().y())?,
        )
    };
    let menu_y = bar.y() + (bar.height() - menu_extent) * 0.5;
    let menu_bottom = (menu_y + menu_extent).min(bar.max().y());
    let root_menu_left = bar.max().x() - menu_extent;
    let menu_bounds = |left, right| {
        LogicalRect::from_min_max(
            LogicalPoint::new(left, menu_y)?,
            LogicalPoint::new(right, menu_bottom)?,
        )
    };
    Ok(Some(TabStripOptionalChromeLayout {
        group_extent,
        leading_reserved_bounds: (leading > 0.0)
            .then(|| bounds(control_viewport.x(), viewport.x()))
            .transpose()?,
        trailing_reserved_bounds: (trailing > 0.0)
            .then(|| bounds(viewport.max().x(), control_viewport.max().x()))
            .transpose()?,
        presentation_menu_bounds: root_menu
            .then(|| menu_bounds(root_menu_left, bar.max().x()))
            .transpose()?,
        group_menu_bounds: group_menu
            .then(|| {
                menu_bounds(
                    right,
                    if root_menu {
                        root_menu_left
                    } else {
                        bar.max().x()
                    },
                )
            })
            .transpose()?,
        viewport,
        controls,
    }))
}
