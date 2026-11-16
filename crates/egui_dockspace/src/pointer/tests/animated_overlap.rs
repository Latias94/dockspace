//! Overlapping animated panes retain distinct click and drag receiver lanes.

use std::time::Duration;

use super::*;
use crate::motion::{Curve, MotionSettings, Transition};

fn frame(
    context: &Context,
    dockspace: &mut Dockspace,
    time: f64,
    events: Vec<Event>,
) -> (FullOutput, Vec<ItemId>) {
    let mut closed = Vec::new();
    let (mut output, receipt) = crate::run_ui(
        context,
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| {
            let response = dockspace
                .show_single_surface(SURFACE, ui, &mut Panes)
                .expect("overlapping independent pointer lanes must commit");
            closed.extend(
                response
                    .close_requests()
                    .flat_map(|plan| plan.items().iter().map(|item| item.item())),
            );
        },
    )
    .into_parts();
    receipt.report(crate::SurfacePresentationResult::Presented);
    output.textures_delta.clear();
    (output, closed)
}

fn control(output: &FullOutput, role: Role, label: &str) -> (egui::accesskit::NodeId, Rect) {
    let (id, node) = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .unwrap_or_else(|| panic!("the animated output exposes {role:?} {label}"));
    let bounds = node.bounds().unwrap();
    #[allow(clippy::cast_possible_truncation)]
    let rect = Rect::from_min_max(
        Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
        Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
    );
    (*id, rect)
}

#[test]
fn an_animated_close_over_another_tab_body_does_not_fail_the_pointer_protocol() {
    let context = Context::default();
    context.enable_accesskit();
    context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [
                    DockspaceNode::tabs([FIRST]),
                    DockspaceNode::tabs([SECOND]),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut dockspace = Dockspace::builder("animated-close-over-another-tab", layout)
        .build()
        .unwrap();
    dockspace.set_motion_settings(MotionSettings {
        layout: Transition::Tween {
            duration: Duration::from_secs(1),
            curve: Curve::Linear,
        },
        ..Default::default()
    });
    for time in [0.0, 0.1, 0.2] {
        frame(&context, &mut dockspace, time, Vec::new());
    }
    let moved = dockspace
        .dock_item_current(
            FIRST,
            crate::DockPlacement::InnerEdge {
                anchor: crate::DockAnchor::Item(SECOND),
                edge: crate::DockEdge::Right,
                fraction: crate::DockFraction::new(0.5).unwrap(),
            },
        )
        .unwrap();
    assert!(matches!(
        moved.status(),
        crate::DockspaceActionStatus::Applied(crate::DockspaceActionOutcome::Docked {
            item: FIRST,
            changed: true,
            ..
        })
    ));
    for _ in 0..3 {
        frame(&context, &mut dockspace, 1.0, Vec::new());
    }
    let (output, _) = frame(&context, &mut dockspace, 1.45, Vec::new());
    let (close_id, close) = control(&output, Role::Button, "Close First");
    let (body_id, body) = control(&output, Role::Tab, "Second");
    let overlap = close.intersect(body);
    assert!(
        overlap.is_positive(),
        "the travelling front pane's close button overlaps the other tab body: {close:?}, {body:?}",
    );
    let point = overlap.center();
    frame(
        &context,
        &mut dockspace,
        1.45,
        vec![Event::PointerMoved(point)],
    );
    context.viewport(|viewport| {
        assert_eq!(
            viewport.hits.click.map(|hit| hit.id.accesskit_id()),
            Some(close_id),
            "the actual front close control owns the click lane",
        );
        assert_eq!(
            viewport.hits.drag.map(|hit| hit.id.accesskit_id()),
            Some(body_id),
            "the lower tab still owns egui's independent drag lane",
        );
    });
    let target = point + egui::vec2(120.0, 80.0);
    let version = dockspace.version();
    let (_, closed) = frame(
        &context,
        &mut dockspace,
        1.45,
        vec![
            Event::ModifiersChanged(Modifiers::SHIFT),
            Event::PointerMoved(point),
            button(point, true),
            Event::PointerMoved(target),
            button(target, false),
        ],
    );
    assert!(
        !dockspace.has_active_gesture(),
        "the terminal must leave the real pointer input usable",
    );
    assert!(closed.is_empty(), "moving out cancels the foreground close");
    assert_eq!(
        dockspace.version(),
        version,
        "the lower tab must not change topology or selection",
    );
    for item in [FIRST, SECOND] {
        assert!(
            dockspace.view().item(item).unwrap().contained().is_none(),
            "the background drag lane must not float item {item}",
        );
    }

    let (_, closed) = frame(
        &context,
        &mut dockspace,
        1.45,
        vec![
            Event::PointerMoved(point),
            button(point, true),
            button(point, false),
        ],
    );
    assert_eq!(
        closed,
        [FIRST],
        "an actual click still requests closing the foreground tab only",
    );
    let (settled, _) = frame(&context, &mut dockspace, 2.1, Vec::new());
    let (_, body) = control(&settled, Role::Tab, "Second");
    let source = body.center();
    let target = source + egui::vec2(96.0, 80.0);
    frame(
        &context,
        &mut dockspace,
        2.1,
        vec![
            Event::PointerButton {
                pos: source,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::SHIFT,
            },
            Event::PointerMoved(target),
            Event::PointerButton {
                pos: target,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::SHIFT,
            },
        ],
    );
    assert!(
        dockspace.view().item(SECOND).unwrap().contained().is_some(),
        "another real tab must remain draggable after resolving the overlap",
    );
    assert!(!dockspace.has_active_gesture());
}

#[test]
fn a_foreground_drag_lane_over_a_background_close_resizes_without_closing() {
    let context = Context::default();
    context.enable_accesskit();
    context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [
                    DockspaceNode::tabs([FIRST]),
                    DockspaceNode::tabs([SECOND]),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut dockspace = Dockspace::builder("foreground-splitter-over-background-close", layout)
        .style(crate::DockStyle {
            splitter_hit_extent: 800.0,
            ..Default::default()
        })
        .build()
        .unwrap();
    dockspace.set_input(crate::DockInputConfig {
        double_click_equalizes_splitter: false,
        ..Default::default()
    });
    let mut output = frame(&context, &mut dockspace, 0.0, Vec::new()).0;
    for time in [0.1, 0.2, 0.3] {
        output = frame(&context, &mut dockspace, time, Vec::new()).0;
    }
    let (_, close) = control(&output, Role::Button, "Close First");
    let (splitter, hit) = control(&output, Role::Splitter, "Resize panes");
    let value = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(id, _)| *id == splitter)
        .unwrap()
        .1
        .numeric_value()
        .unwrap();
    let point = close.intersect(hit).center();
    assert!(close.intersect(hit).is_positive());
    frame(
        &context,
        &mut dockspace,
        0.4,
        vec![Event::PointerMoved(point)],
    );
    context.viewport(|viewport| {
        assert_eq!(
            viewport.hits.drag.map(|hit| hit.id.accesskit_id()),
            Some(splitter),
            "the foreground drag-only splitter owns the actual egui gesture",
        );
        assert!(
            viewport.hits.click.is_none(),
            "egui suppresses the background click below a foreground drag-only widget",
        );
    });
    let target = point + egui::vec2(32.0, 0.0);
    let (_, closed) = frame(
        &context,
        &mut dockspace,
        0.5,
        vec![
            Event::PointerMoved(point),
            button(point, true),
            Event::PointerMoved(target),
            button(target, false),
        ],
    );
    assert!(closed.is_empty(), "the lower close button must not activate");
    assert!(!dockspace.has_active_gesture());
    let (output, closed) = frame(&context, &mut dockspace, 0.6, Vec::new());
    assert!(closed.is_empty());
    let after = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(id, _)| *id == splitter)
        .unwrap()
        .1
        .numeric_value()
        .unwrap();
    let extent = f64::from(hit.width() - dockspace.style().splitter_thickness);
    assert!(
        ((after - value) * extent - 32.0).abs() < 0.01,
        "the foreground splitter must commit its requested 32-point resize: {value} -> {after}",
    );
    for item in [FIRST, SECOND] {
        assert!(dockspace.view().item(item).is_some());
    }
}
