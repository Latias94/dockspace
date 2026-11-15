use super::*;

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
