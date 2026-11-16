use super::*;

fn group_drag_point(output: &FullOutput, label: &str, trailing: bool) -> Pos2 {
    let tab = node_rect(output, Role::Tab, label);
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled")
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::GenericContainer && node.label() == Some("Drag tab group")
        })
        .map(|(_, node)| accesskit_node_rect(node))
        .filter(|rect| {
            rect.min.y <= tab.center().y
                && rect.max.y >= tab.center().y
                && if trailing {
                    rect.min.x >= tab.max.x
                } else {
                    rect.max.x <= tab.min.x
                }
        })
        .min_by(|left, right| {
            left.center()
                .distance_sq(tab.center())
                .total_cmp(&right.center().distance_sq(tab.center()))
        })
        .expect("the source tab group exposes the requested drag region")
        .center()
}

fn shift_tear_off(
    context: &Context,
    dockspace: &mut Dockspace,
    panes: &mut dyn PaneView,
    source: Pos2,
) {
    let destination = Pos2::new(60.0, 520.0);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::SHIFT,
    };
    for events in [
        vec![
            Event::ModifiersChanged(Modifiers::SHIFT),
            Event::PointerMoved(source),
        ],
        vec![button(source, true)],
        vec![Event::PointerMoved(source + vec2(16.0, 12.0))],
        vec![Event::PointerMoved(destination)],
        vec![button(destination, false)],
        Vec::new(),
    ] {
        let _ = run_pane_frame(context, dockspace, panes, events);
    }
    assert!(!dockspace.has_active_gesture());
}

#[test]
fn nested_central_group_shift_drag_detaches_contents_and_preserves_empty_central() {
    let context = Context::default();
    context.enable_accesskit();
    let node = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::central_tabs([FIRST, SECOND]),
            DockspaceNode::tabs([THIRD]),
        ],
    )
    .unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, node),
    )])
    .unwrap();
    let mut dockspace = Dockspace::builder("nested-central-group-tear-off", layout)
        .build()
        .unwrap();
    let mut panes = Panes;
    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let source = group_drag_point(&stable.output, "First", false);
    shift_tear_off(&context, &mut dockspace, &mut panes, source);

    let first = dockspace.view().item(FIRST).unwrap();
    assert!(
        first.contained().is_some(),
        "dragging a central tab group moves its contents into a floating root"
    );
    assert_eq!(first.tabs().items(), &[FIRST, SECOND]);
    assert_eq!(dockspace.view().item(SECOND).unwrap().root(), first.root());
    assert_eq!(dockspace.view().item(THIRD).unwrap().root(), ROOT);
    assert!(
        dockspace
            .view()
            .root(ROOT)
            .unwrap()
            .central()
            .unwrap()
            .tabs()
            .unwrap()
            .items()
            .is_empty(),
        "the central slot remains available in the main layout"
    );
}

#[derive(Default)]
struct PartialFloatPanes {
    projection: ProjectionPanes,
}

impl PaneView for PartialFloatPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Panes.title(item)
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        self.projection.ui(item, ui)
    }

    fn sizing(&self, item: ItemId) -> egui_dockspace::PaneSizing {
        let (width, height) = match item {
            SECOND => (100.0, 90.0),
            THIRD => (160.0, 110.0),
            FOURTH => (320.0, 200.0),
            _ => (0.0, 0.0),
        };
        egui_dockspace::PaneSizing::builder()
            .min_width(width)
            .min_height(height)
            .build()
            .unwrap()
    }
}

fn split_contained_layout() -> DockspaceLayout {
    let floating_node = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::tabs([SECOND, THIRD]),
            DockspaceNode::tabs([FOURTH]),
        ],
    )
    .unwrap();
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs([FIRST])),
    )
    .with_contained(DockspaceContainedLayout::new(
        FLOATING,
        DockspaceRootLayout::new(FLOATING_ROOT, floating_node),
        LogicalRect::new(120.0, 100.0, 600.0, 360.0).unwrap(),
    ))])
    .unwrap()
}

fn assert_partial_contained_tear_off_uses_its_pane_geometry(group: bool) {
    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace = Dockspace::builder(
        ("partial-contained-tear-off", group),
        split_contained_layout(),
    )
    .build()
    .unwrap();
    let mut panes = PartialFloatPanes::default();
    let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let source_bounds = panes.projection.latest(SECOND).bounds;
    let source = if group {
        group_drag_point(&stable.output, "Second", false)
    } else {
        tab_center(&stable.output, "Second")
    };
    shift_tear_off(&context, &mut dockspace, &mut panes, source);

    let moved = dockspace.view().item(SECOND).unwrap();
    let floating = moved.contained().expect("the partial payload was floated");
    assert_ne!(floating, FLOATING);
    assert_eq!(
        moved.tabs().items(),
        if group {
            &[SECOND, THIRD][..]
        } else {
            &[SECOND][..]
        }
    );
    assert_eq!(
        dockspace.view().item(FOURTH).unwrap().contained(),
        Some(FLOATING)
    );
    assert_eq!(
        dockspace.view().item(THIRD).unwrap().contained(),
        Some(if group { floating } else { FLOATING })
    );
    let bounds = dockspace.view().contained(floating).unwrap().rect();
    assert!(
        bounds.width() <= f64::from(source_bounds.width() + 32.0),
        "a partial tear-off must use its pane's width and minimum, without inheriting its sibling's constraints: pane={source_bounds:?}, floating={bounds:?}"
    );
    assert!(
        bounds.height() <= f64::from(source_bounds.height() + 64.0),
        "a partial tear-off must only add its own chrome to the source pane"
    );
}

#[test]
fn contained_item_shift_drag_uses_source_pane_geometry() {
    assert_partial_contained_tear_off_uses_its_pane_geometry(false);
}

#[test]
fn contained_tab_group_shift_drag_uses_source_pane_geometry() {
    assert_partial_contained_tear_off_uses_its_pane_geometry(true);
}

#[test]
fn contained_split_title_shift_drag_preserves_complete_window_geometry() {
    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace =
        Dockspace::builder("complete-contained-title-move", split_contained_layout())
            .build()
            .unwrap();
    let mut panes = PartialFloatPanes::default();
    let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let original = contained_rect(&dockspace);
    let source = contained_title_point(&dockspace);
    shift_tear_off(&context, &mut dockspace, &mut panes, source);

    let moved = dockspace.view().contained(FLOATING).unwrap();
    assert_eq!(moved.rect().size(), original.size());
    assert_ne!(moved.rect().min(), original.min());
    assert_eq!(
        dockspace.view().surface(SURFACE).unwrap().contained_count(),
        1,
        "moving a complete floating title keeps the existing presentation"
    );
    for item in [SECOND, THIRD, FOURTH] {
        let location = dockspace.view().item(item).unwrap();
        assert_eq!(location.root(), FLOATING_ROOT);
        assert_eq!(location.contained(), Some(FLOATING));
    }
    assert!(moved.root().unwrap().content().unwrap().split().is_some());
}

fn assert_chrome_click_focuses_selected_content(title: bool) {
    let context = Context::default();
    context.enable_accesskit();
    let layout = if title {
        contained_layout()
    } else {
        split_layout()
    };
    let mut dockspace = Dockspace::builder(("chrome-click-pane-focus", title), layout)
        .build()
        .unwrap();
    let mut panes = FocusPanes::new("chrome-click-pane-focus", true);
    let _ = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (stable, _) = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(!context.memory(|memory| memory.has_focus(panes.target)));
    let source = if title {
        contained_title_point(&dockspace)
    } else {
        group_drag_point(&stable.output, "Second", true)
    };
    for events in [
        vec![Event::PointerMoved(source)],
        vec![pointer_button(source, true)],
        vec![pointer_button(source, false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ] {
        let _ = run_focus_frame(&context, &mut dockspace, &mut panes, events);
    }
    assert!(!dockspace.has_active_gesture());
    assert!(
        context.memory(|memory| memory.has_focus(panes.target)),
        "clicking {} must focus its selected pane's content",
        if title {
            "the floating title"
        } else {
            "the tab group's trailing blank space"
        }
    );
}

#[test]
fn contained_title_click_focuses_selected_pane_content() {
    assert_chrome_click_focuses_selected_content(true);
}

#[test]
fn tab_group_trailing_click_focuses_selected_pane_content() {
    assert_chrome_click_focuses_selected_content(false);
}

#[test]
fn a_pane_callbacks_later_focus_choice_supersedes_the_chrome_click() {
    struct RedirectingPanes {
        panes: FocusPanes,
        alternate: egui::Id,
        redirect: bool,
    }

    impl PaneView for RedirectingPanes {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            self.panes.title(item)
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
            let response = self.panes.ui(item, ui);
            if item == SECOND {
                let alternate = ui.interact(
                    self.panes.target_rect().translate(vec2(0.0, 32.0)),
                    self.alternate,
                    egui::Sense::click_and_drag(),
                );
                if std::mem::take(&mut self.redirect) {
                    alternate.request_focus();
                }
                response.with_content(&alternate)
            } else {
                response
            }
        }

        fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
            self.panes.focus_target(item)
        }

        fn focus_state(&self, item: ItemId, context: &Context) -> PaneFocusState {
            self.panes.focus_state(item, context)
        }
    }

    for title in [false, true] {
        let context = Context::default();
        context.enable_accesskit();
        let layout = if title {
            contained_layout()
        } else {
            split_layout()
        };
        let mut dockspace = Dockspace::builder(("chrome-callback-focus-priority", title), layout)
            .build()
            .unwrap();
        let mut panes = RedirectingPanes {
            panes: FocusPanes::new("chrome-callback-focus-priority", true),
            alternate: egui::Id::unique(("chrome-callback-alternate", title)),
            redirect: false,
        };
        let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let stable = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let source = if title {
            contained_title_point(&dockspace)
        } else {
            group_drag_point(&stable.output, "Second", true)
        };
        for events in [
            vec![Event::PointerMoved(source)],
            vec![pointer_button(source, true)],
        ] {
            let _ = run_pane_frame(&context, &mut dockspace, &mut panes, events);
        }
        panes.redirect = true;
        let _ = run_pane_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![pointer_button(source, false)],
        );
        assert!(context.memory(|memory| memory.has_focus(panes.alternate)));
        for _ in 0..3 {
            let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
            assert!(
                context.memory(|memory| memory.has_focus(panes.alternate)),
                "the pending chrome intent must not replay over the callback's later focus choice"
            );
        }
    }
}

#[test]
fn an_unregistered_pane_focus_target_is_not_requested() {
    struct UnregisteredFocusPanes(egui::Id);

    impl PaneView for UnregisteredFocusPanes {
        fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
            Panes.title(item)
        }

        fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
            Panes.ui(item, ui)
        }

        fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
            (item == SECOND).then_some(self.0)
        }
    }

    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace = Dockspace::builder("unregistered-pane-focus", layout())
        .build()
        .unwrap();
    let mut panes = UnregisteredFocusPanes(egui::Id::unique("unregistered-pane-focus-target"));
    let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let _ = dockspace.select_item_current(SECOND).unwrap();
    for _ in 0..3 {
        let _ = run_pane_frame(&context, &mut dockspace, &mut panes, Vec::new());
        assert!(
            !context.memory(|memory| memory.has_focus(panes.0)),
            "declaring a focus ID must not grant focus before its current pane registers a valid widget"
        );
    }
}
