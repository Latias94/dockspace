use super::*;

#[test]
fn compact_chrome_prioritizes_selection_and_clips_custom_slots_at_narrow_widths() {
    let config = DockPresentationConfig::builder()
        .tab_bar_height(24.0)
        .tab_min_width(56.0)
        .tab_close_extent(18.0)
        .tab_group_grip_extent(22.0)
        .build()
        .unwrap();
    let controls = TabStripControlMetrics::new(0.0)
        .unwrap()
        .with_tab_list_menu(
            crate::presentation::manifest::TabStripControlMetric::new(
                24.0,
                TabStripControlPlacement::ReservedTrailing,
            )
            .unwrap(),
        );
    for width in [0.0, 16.0, 24.0, 40.0, 80.0, 160.0] {
        for reserved in [0.0, 128.0] {
            let bar = LogicalRect::new(13.25, 7.5, width, 24.0).unwrap();
            let strip = crate::presentation::manifest::TabStripMetrics::new(reserved, reserved)
                .unwrap()
                .with_after_tabs_width(200.0)
                .unwrap()
                .with_controls(controls);
            let chrome = tab_strip_optional_chrome_layout(bar, 6, strip, &config, true).unwrap();
            let list = chrome.controls.and_then(|controls| controls.rects[2]);
            assert_eq!(
                list.is_some(),
                width >= 24.0,
                "width={width}, reserved={reserved}"
            );
            if width >= 80.0 {
                assert!(chrome.viewport.width() >= 56.0);
            }
            if width == 80.0 {
                assert!(chrome.presentation_menu_bounds.is_none());
                assert!(chrome.group_menu_bounds.is_none());
            }
            let regions = [
                Some(chrome.viewport),
                chrome.leading_reserved_bounds,
                chrome.trailing_reserved_bounds,
                chrome.presentation_menu_bounds,
                chrome.group_menu_bounds,
                list,
            ];
            for (index, bounds) in regions.iter().enumerate() {
                let Some(bounds) = bounds.filter(|bounds| rect_has_area(*bounds)) else {
                    continue;
                };
                assert!(rect_contains(bar, bounds));
                for other in regions[index + 1..].iter().flatten().copied() {
                    assert!(intersect_rect(bounds, other).unwrap().is_none());
                }
            }
        }
    }
}

#[test]
fn narrow_strip_keeps_the_tab_list_when_only_its_target_fits() {
    let metrics = TabStripControlMetrics::new(0.0)
        .unwrap()
        .with_tab_list_menu(
            crate::presentation::manifest::TabStripControlMetric::new(
                24.0,
                TabStripControlPlacement::ReservedTrailing,
            )
            .unwrap(),
        );
    let layout = tab_strip_control_layout(LogicalRect::new(0.0, 0.0, 24.0, 24.0).unwrap(), metrics)
        .unwrap()
        .expect("a tab list can remain usable without a visible tab");
    assert_eq!(layout.viewport.width(), 0.0);
    assert_eq!(layout.rects[2].unwrap().width(), 24.0);
}

#[test]
fn tab_gap_markers_round_outward_within_large_coordinate_viewports() {
    for scale in [1e-12, 1e-6, 1.0] {
        let viewport = LogicalRect::from_min_max(
            LogicalPoint::new(300.0 * scale, 0.0).unwrap(),
            LogicalPoint::new(1e16 * scale, 28.0 * scale).unwrap(),
        )
        .unwrap();
        let tab = LogicalRect::new(300.0 * scale, 0.0, 96.0 * scale, 28.0 * scale).unwrap();
        let gaps = tab_gap_geometries(viewport, &[tab], scale).unwrap();
        assert_eq!(gaps.len(), 2);
        for gap in &gaps {
            assert!(rect_has_area(gap.hit));
            assert!(rect_has_area(gap.visual));
            assert!(rect_contains(viewport, gap.visual));
        }
        assert_eq!(gaps[1].visual.max(), viewport.max());
    }
}

#[test]
fn rounded_tab_chrome_keeps_the_close_and_drag_boundary_identical() {
    for scale in [1.0, 1e6, 1e12] {
        let full = LogicalRect::from_min_max(
            LogicalPoint::new(2.8e-11 * scale, 0.0).unwrap(),
            LogicalPoint::new(1.219_999_999_999_954_7e-10 * scale, 2.8e-11 * scale).unwrap(),
        )
        .unwrap();
        let config = DockPresentationConfig::builder()
            .tab_close_extent(1.6e-11 * scale)
            .tab_horizontal_padding(1e-11 * scale)
            .build()
            .unwrap();
        let chrome = operable_tab_chrome(full, full, true, &config)
            .unwrap()
            .unwrap();
        let close = chrome.close.unwrap();
        assert_eq!(chrome.drag.max().x().to_bits(), close.x().to_bits());
        assert!(rect_has_area(chrome.drag));
        assert!(rect_contains(full, chrome.drag));
        assert!(rect_contains(full, close));
        assert!(rect_contains(full, chrome.text_layout));
        assert!(rect_contains(chrome.visible, chrome.text));
    }
}
