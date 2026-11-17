//! Visibility crosses the real renderer, keyboard navigation and accessible resize controls.

use super::*;
use crate::{DockspaceActionOutcome, DockspaceActionStatus};

fn settle(harness: &mut Harness) -> FullOutput {
    let mut output = harness.run(800.0, Vec::new());
    for _ in 0..3 {
        output = harness.run(800.0, Vec::new());
    }
    output
}

#[test]
fn tab_cycling_and_accessibility_skip_hidden_tabs_until_restored() {
    let mut harness = Harness::tabs(1);
    harness.dockspace.set_input(crate::DockInputConfig {
        next_tab: Some(egui::KeyboardShortcut::new(egui::Modifiers::CTRL, Key::Tab)),
        ..Default::default()
    });
    for item in [2, 4, 5, 6] {
        let hidden = harness
            .dockspace
            .set_item_visibility_current(ItemId::new(item), false)
            .unwrap();
        assert!(matches!(
            hidden.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemVisibilityChanged {
                item: hidden_item,
                visible: false,
                changed: true,
            }) if *hidden_item == ItemId::new(item)
        ));
    }
    let output = settle(&mut harness);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Tab && node.label() == Some("Document 2"))
    );
    assert!(
        !texts(&output)
            .iter()
            .any(|(_, text)| text.galley.text() == "Document 2")
    );
    harness.focus_tab(800.0, "Document 1");
    for expected in [3, 1, 3] {
        let mut event = key(Key::Tab);
        if let Event::Key { modifiers, .. } = &mut event {
            *modifiers = egui::Modifiers::CTRL;
        }
        harness.run(800.0, vec![event]);
        settle(&mut harness);
        assert!(
            harness
                .dockspace
                .view()
                .item(ItemId::new(expected))
                .unwrap()
                .is_active()
        );
    }
    let shown = harness
        .dockspace
        .set_item_visibility_current(ItemId::new(2), true)
        .unwrap();
    assert!(matches!(
        shown.status(),
        DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemVisibilityChanged {
            item,
            visible: true,
            changed: true,
        }) if *item == ItemId::new(2)
    ));
    let output = settle(&mut harness);
    assert!(
        nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Tab && node.label() == Some("Document 2"))
    );
    assert!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(3))
            .unwrap()
            .is_active()
    );
    assert_eq!(
        harness
            .dockspace
            .view()
            .item(ItemId::new(2))
            .unwrap()
            .tab_index(),
        Some(1)
    );
}

#[test]
fn keyboard_splitter_ranges_and_equalization_preserve_hidden_pane_shares() {
    for axis in [
        crate::DockspaceAxis::Horizontal,
        crate::DockspaceAxis::Vertical,
    ] {
        let mut harness = Harness::tabs(1);
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::split(
                    axis,
                    [
                        (DockspaceNode::tabs([ItemId::new(1)]), 0.2),
                        (DockspaceNode::tabs([ItemId::new(2)]), 0.3),
                        (DockspaceNode::tabs([ItemId::new(3)]), 0.5),
                    ],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        harness.dockspace = Dockspace::builder(("hidden-splitter", axis), layout)
            .build()
            .unwrap();
        let hidden = harness
            .dockspace
            .set_item_visibility_current(ItemId::new(2), false)
            .unwrap();
        assert!(matches!(
            hidden.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemVisibilityChanged {
                item,
                visible: false,
                changed: true,
            }) if *item == ItemId::new(2)
        ));
        let output = settle(&mut harness);
        let splitters: Vec<_> = nodes(&output)
            .iter()
            .filter(|(_, node)| node.role() == Role::Splitter)
            .collect();
        assert_eq!(splitters.len(), 1);
        let value = splitters[0].1.numeric_value().unwrap();
        assert!((value - 0.2 / 0.7).abs() < 1e-5, "{axis:?}: {value}");
        let endpoint = splitters[0].1.max_numeric_value().unwrap();
        harness.focus(800.0, Role::Splitter, "Resize panes");
        harness.run(800.0, vec![key(Key::End)]);
        let output = settle(&mut harness);
        let (_, splitter) = nodes(&output)
            .iter()
            .find(|(_, node)| node.role() == Role::Splitter)
            .unwrap();
        assert!((splitter.numeric_value().unwrap() - endpoint).abs() < 1e-5);

        let bounds = splitter.bounds().unwrap();
        let point = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        harness.run(800.0, vec![Event::PointerMoved(point)]);
        for _ in 0..2 {
            for pressed in [true, false] {
                harness.run(
                    800.0,
                    vec![Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        }
        let output = settle(&mut harness);
        let (_, splitter) = nodes(&output)
            .iter()
            .find(|(_, node)| node.role() == Role::Splitter)
            .unwrap();
        assert!((splitter.numeric_value().unwrap() - 0.5).abs() < 1e-5);
        let weights = || {
            harness
                .dockspace
                .view()
                .root(RootId::new(1))
                .unwrap()
                .content()
                .unwrap()
                .split()
                .unwrap()
                .weights()
                .collect::<Vec<_>>()
        };
        let after = weights();
        assert_eq!(after[1], 0.3);
        assert!((after[0] - 0.35).abs() < 1e-5);
        assert!((after[2] - 0.35).abs() < 1e-5);
        let shown = harness
            .dockspace
            .set_item_visibility_current(ItemId::new(2), true)
            .unwrap();
        assert!(matches!(
            shown.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemVisibilityChanged {
                item,
                visible: true,
                changed: true,
            }) if *item == ItemId::new(2)
        ));
        let output = settle(&mut harness);
        assert_eq!(
            nodes(&output)
                .iter()
                .filter(|(_, node)| node.role() == Role::Splitter)
                .count(),
            2
        );
        let restored = harness
            .dockspace
            .view()
            .root(RootId::new(1))
            .unwrap()
            .content()
            .unwrap()
            .split()
            .unwrap()
            .weights()
            .collect::<Vec<_>>();
        assert_eq!(restored, after);
    }
}
