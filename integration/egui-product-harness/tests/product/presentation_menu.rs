use super::*;

#[test]
fn menu_floated_root_stays_floating_when_released_away_from_docking_guides() {
    for layout in [layout(), split_layout()] {
        let context = Context::default();
        context.enable_accesskit();
        let mut dockspace = Dockspace::builder("menu-float-empty-floor-drag", layout)
            .build()
            .unwrap();
        let original = (root_tabs(&dockspace), root_split(&dockspace));
        let mut panes = Panes;
        let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let (menu, menu_node) = accesskit_node(&stable.output, Role::Button, "Layout actions");
        assert_eq!(
            menu_node.description(),
            Some("Actions for the entire layout, including all split groups")
        );
        let _ = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![accesskit_action(menu, Action::Click)],
        );
        let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let (float, _) = accesskit_node(&opened.output, Role::MenuItem, "Float Layout");
        let _ = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![accesskit_action(float, Action::Click)],
        );
        let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let floating = dockspace.view().item(FIRST).unwrap().contained().unwrap();
        let source = node_center(&stable.output, Role::TitleBar, "First");
        let blank = Pos2::new(80.0, 450.0);
        for events in [
            vec![Event::PointerMoved(source)],
            vec![pointer_button(source, true)],
            vec![Event::PointerMoved(source + vec2(12.0, 8.0))],
        ] {
            let _ = run_frame(&context, &mut dockspace, &mut panes, events);
        }
        let preview = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![Event::PointerMoved(blank)],
        );
        let guide = node_rect(&preview.output, Role::Label, "Dockspace center drop guide");
        assert!(!guide.contains(blank));
        let _ = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![pointer_button(blank, false)],
        );
        let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        assert!(!dockspace.has_active_gesture());
        assert!(
            dockspace.view().contained(floating).is_some(),
            "releasing a floating root over blank canvas must keep it floating"
        );
        assert!(
            dockspace
                .view()
                .surface(SURFACE)
                .unwrap()
                .main_root()
                .is_none()
        );
        for item in [FIRST, SECOND] {
            let item = dockspace.view().item(item).unwrap();
            assert_eq!(item.root(), ROOT);
            assert_eq!(item.contained(), Some(floating));
        }

        for modifiers in [Modifiers::SHIFT, Modifiers::NONE] {
            let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
            let source = node_center(&stable.output, Role::TitleBar, "First");
            let button = |pressed| Event::PointerButton {
                pos: if pressed { source } else { guide.center() },
                button: PointerButton::Primary,
                pressed,
                modifiers,
            };
            for events in [
                vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(source),
                ],
                vec![button(true)],
                vec![Event::PointerMoved(source + vec2(12.0, 8.0))],
                vec![Event::PointerMoved(guide.center())],
            ] {
                let _ = run_frame(&context, &mut dockspace, &mut panes, events);
            }
            assert!(dockspace.has_active_gesture());
            let _ = run_frame(&context, &mut dockspace, &mut panes, vec![button(false)]);
            let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
            assert!(!dockspace.has_active_gesture());
            if modifiers.shift {
                assert!(
                    dockspace.view().contained(floating).is_some(),
                    "Shift must suppress docking even over the empty-surface guide"
                );
            } else {
                assert!(dockspace.view().contained(floating).is_none());
                assert_eq!(
                    dockspace
                        .view()
                        .surface(SURFACE)
                        .unwrap()
                        .main_root()
                        .unwrap()
                        .id(),
                    ROOT
                );
                assert_eq!(
                    (root_tabs(&dockspace), root_split(&dockspace)),
                    original,
                    "the explicit center drop restores the complete layout and stable identities"
                );
            }
        }
    }
}

#[test]
fn presentation_menu_floats_and_docks_back_with_truthful_native_availability() {
    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace = Dockspace::builder("product-presentation-menu", layout())
        .build()
        .expect("the presentation-menu facade initializes");
    let mut panes = Panes;

    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (menu, menu_node) = accesskit_node(&stable.output, Role::Button, "Layout actions");
    assert!(!menu_node.is_disabled());

    let sizing = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(menu, Action::Click)],
    );
    assert!(sizing.close_requests.is_empty());
    let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(opened.close_requests.is_empty());
    let (float, float_node) = accesskit_node(&opened.output, Role::MenuItem, "Float Layout");
    assert!(
        !opened
            .output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem
                && node.label() == Some("Move Layout to New Window"))
    );
    let (_, dock_back_node) = accesskit_node(&opened.output, Role::MenuItem, "Dock Back");
    assert!(
        !float_node.is_disabled(),
        "{float_node:#?}; description={:?}",
        float_node.description()
    );
    assert!(dock_back_node.is_disabled());
    assert_eq!(
        surface_ids(&dockspace),
        [SURFACE],
        "the product starts with one same-context logical surface"
    );

    let floated = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(float, Action::Click)],
    );
    assert!(floated.close_requests.is_empty());
    assert!(dockspace.view().item(SECOND).unwrap().contained().is_none());
    let floated = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(floated.close_requests.is_empty());
    let floating_id = dockspace
        .view()
        .item(SECOND)
        .and_then(|item| item.contained())
        .expect("Float must move the complete root into an egui-native contained window");
    for item in [FIRST, SECOND] {
        let location = dockspace
            .view()
            .item(item)
            .expect("Float preserves every item");
        assert_eq!(location.contained(), Some(floating_id));
        assert_eq!(
            location.tabs().items().collect::<Vec<_>>(),
            &[FIRST, SECOND]
        );
        assert_eq!(
            location.tabs().selected().and_then(|tab| tab.item()),
            Some(FIRST)
        );
    }
    assert_eq!(
        surface_ids(&dockspace),
        [SURFACE],
        "same-context Float must not synthesize a native logical surface or binding"
    );

    let floating = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(floating.close_requests.is_empty());
    let (menu, _) = accesskit_node(&floating.output, Role::Button, "Layout actions");
    let sizing = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(menu, Action::Click)],
    );
    assert!(sizing.close_requests.is_empty());
    let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(opened.close_requests.is_empty());
    let (_, float_node) = accesskit_node(&opened.output, Role::MenuItem, "Float Layout");
    assert!(
        !opened
            .output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem
                && node.label() == Some("Move Layout to New Window")),
        "contained ownership cannot add a native command to this host"
    );
    let (dock_back, dock_back_node) = accesskit_node(&opened.output, Role::MenuItem, "Dock Back");
    assert!(float_node.is_disabled());
    assert!(!dock_back_node.is_disabled());

    let docked = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(dock_back, Action::Click)],
    );
    assert!(
        docked.close_requests.is_empty(),
        "Dock Back must move the root without requesting permission to destroy its content",
    );
    assert!(dockspace.view().contained(floating_id).is_some());
    let docked = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(docked.close_requests.is_empty());
    assert!(dockspace.view().contained(floating_id).is_none());
    let main = dockspace
        .view()
        .surface(SURFACE)
        .and_then(|surface| surface.main_root())
        .expect("Dock Back recovers the complete root as the main presentation");
    for item in [FIRST, SECOND] {
        let location = dockspace
            .view()
            .item(item)
            .expect("Dock Back preserves every item");
        assert_eq!(location.root(), main.id());
        assert_eq!(location.surface(), SURFACE);
        assert_eq!(location.contained(), None);
        assert_eq!(
            location.tabs().items().collect::<Vec<_>>(),
            &[FIRST, SECOND]
        );
        assert_eq!(
            location.tabs().selected().and_then(|tab| tab.item()),
            Some(FIRST)
        );
    }
    assert_eq!(
        surface_ids(&dockspace),
        [SURFACE],
        "same-context Dock Back must preserve the sole logical surface"
    );
    let settled = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(
        settled.close_requests.is_empty(),
        "Dock Back must not publish a delayed close plan"
    );
}

#[test]
fn contained_presentation_menu_exposes_and_executes_directional_commands() {
    let cases = [
        (
            "Dock Left",
            Some((DockspaceAxis::Horizontal, vec![vec![SECOND], vec![FIRST]])),
            None,
        ),
        (
            "Dock Right",
            Some((DockspaceAxis::Horizontal, vec![vec![FIRST], vec![SECOND]])),
            None,
        ),
        (
            "Dock Top",
            Some((DockspaceAxis::Vertical, vec![vec![SECOND], vec![FIRST]])),
            None,
        ),
        (
            "Dock Bottom",
            Some((DockspaceAxis::Vertical, vec![vec![FIRST], vec![SECOND]])),
            None,
        ),
        ("Dock Center", None, Some(vec![FIRST, SECOND])),
    ];

    for (label, expected_split, expected_tabs) in cases {
        let context = Context::default();
        context.enable_accesskit();
        let mut dockspace = Dockspace::builder(
            ("product-directional-presentation-menu", label),
            contained_layout(),
        )
        .build()
        .expect("the directional presentation-menu facade initializes");
        let mut panes = Panes;

        let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        let contained_rect = dockspace
            .view()
            .contained(FLOATING)
            .expect("the contained presentation remains available")
            .rect();
        let tree = stable
            .output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit is enabled");
        let (menu, menu_node) = tree
            .nodes
            .iter()
            .find_map(|(id, node)| {
                let bounds = node.bounds()?;
                let center_x = (bounds.x0 + bounds.x1) * 0.5;
                let center_y = (bounds.y0 + bounds.y1) * 0.5;
                (node.role() == Role::Button
                    && node.label() == Some("Layout actions")
                    && center_x >= contained_rect.min().x()
                    && center_x <= contained_rect.max().x()
                    && center_y >= contained_rect.min().y()
                    && center_y <= contained_rect.max().y())
                .then_some((*id, node))
            })
            .expect("the contained title exposes its presentation menu");
        assert!(!menu_node.is_disabled());

        let _ = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![accesskit_action(menu, Action::Click)],
        );
        let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        for command_label in [
            "Dock Left",
            "Dock Right",
            "Dock Top",
            "Dock Bottom",
            "Dock Center",
        ] {
            let (_, node) = accesskit_node(&opened.output, Role::MenuItem, command_label);
            assert!(!node.is_disabled(), "{command_label} must be executable",);
            assert!(
                node.supports_action(Action::Click),
                "{command_label} must expose an AccessKit click action",
            );
        }
        let (command, _) = accesskit_node(&opened.output, Role::MenuItem, label);

        let _ = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![accesskit_action(command, Action::Click)],
        );
        assert!(dockspace.view().contained(FLOATING).is_some());
        let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        assert!(
            dockspace.view().contained(FLOATING).is_none(),
            "{label} must consume the contained presentation",
        );
        assert_eq!(
            root_split(&dockspace),
            expected_split,
            "{label} must use the core-derived split target",
        );
        assert_eq!(
            root_tabs(&dockspace),
            expected_tabs,
            "{label} must use the core-derived central target",
        );
    }
}

#[test]
fn open_presentation_menu_closes_immediately_when_ui_becomes_disabled() {
    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace = Dockspace::builder("product-open-disabled-presentation-menu", layout())
        .build()
        .expect("the presentation-menu facade initializes");
    let mut panes = Panes;

    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (menu, _) = accesskit_node(&stable.output, Role::Button, "Layout actions");
    let _ = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(menu, Action::Click)],
    );
    let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (_, opened_anchor) = accesskit_node(&opened.output, Role::Button, "Layout actions");
    assert_eq!(opened_anchor.is_expanded(), Some(true));
    let _ = accesskit_node(&opened.output, Role::MenuItem, "Float Layout");

    let disabled = run_disabled_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (_, disabled_anchor) = accesskit_node(&disabled.output, Role::Button, "Layout actions");
    assert!(disabled_anchor.is_disabled());
    assert_eq!(
        disabled_anchor.is_expanded(),
        Some(false),
        "the disabled anchor must synchronously report its popup as collapsed",
    );
    let tree = disabled
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled");
    assert!(
        tree.nodes
            .iter()
            .all(|(_, node)| node.role() != Role::MenuItem),
        "the popup must stop rendering as soon as its anchor becomes disabled",
    );
}

#[test]
fn open_main_presentation_menu_closes_immediately_when_anchor_becomes_occluded() {
    let context = Context::default();
    context.enable_accesskit();
    let initial_rect = LogicalRect::new(40.0, 0.0, 180.0, 180.0)
        .expect("the initial contained rectangle is valid");
    let mut dockspace = Dockspace::builder(
        "product-open-occluded-presentation-menu",
        contained_layout_at(initial_rect),
    )
    .build()
    .expect("the presentation-menu occlusion facade initializes");
    let mut panes = Panes;

    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable_tree = stable
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled");
    let (main_menu, main_anchor) = stable_tree
        .nodes
        .iter()
        .find_map(|(id, node)| {
            let is_main_anchor = node.role() == Role::Button
                && node.label() == Some("Layout actions")
                && node.bounds().is_some_and(|bounds| bounds.x0 > 600.0);
            is_main_anchor.then_some((*id, node))
        })
        .expect("the unobscured main root exposes its presentation menu");
    assert!(!main_anchor.is_disabled());
    let (right_resize, _) =
        accesskit_node(&stable.output, Role::Splitter, "Resize floating right edge");

    let _ = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(main_menu, Action::Click)],
    );
    let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let opened_tree = opened
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled");
    let opened_anchor = opened_tree
        .nodes
        .iter()
        .find_map(|(id, node)| (*id == main_menu).then_some(node))
        .expect("the opened main presentation anchor remains stable");
    assert_eq!(opened_anchor.is_expanded(), Some(true));
    let _ = accesskit_node(&opened.output, Role::MenuItem, "Float Layout");

    let mut occluded = None;
    for _ in 0..64 {
        let output = run_frame(
            &context,
            &mut dockspace,
            &mut panes,
            vec![accesskit_action(right_resize, Action::Increment)],
        );
        let tree = output
            .output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit is enabled");
        let anchor_disabled = tree
            .nodes
            .iter()
            .find_map(|(id, node)| (*id == main_menu).then_some(node.is_disabled()))
            .expect("the main presentation anchor remains described");
        if anchor_disabled {
            occluded = Some(output);
            break;
        }
    }
    let occluded = occluded.expect("semantic resizing eventually occludes the main menu anchor");
    let occluded_tree = occluded
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled");
    let occluded_anchor = occluded_tree
        .nodes
        .iter()
        .find_map(|(id, node)| (*id == main_menu).then_some(node))
        .expect("the occluded main presentation anchor remains described");
    assert!(occluded_anchor.is_disabled());
    assert_eq!(
        occluded_anchor.is_expanded(),
        Some(false),
        "the occluded anchor must synchronously report its popup as collapsed",
    );
    assert!(
        occluded_tree
            .nodes
            .iter()
            .all(|(_, node)| node.role() != Role::MenuItem),
        "the stale popup must not remain above or intercept the front contained window",
    );
    assert!(
        occluded_tree.nodes.iter().any(|(id, node)| {
            *id != main_menu
                && node.role() == Role::Button
                && node.label() == Some("Layout actions")
                && !node.is_disabled()
        }),
        "the front contained window keeps its own presentation anchor operable",
    );
}

#[test]
fn discarded_presentation_menu_activation_commits_exactly_once() {
    let context = Context::default();
    context.enable_accesskit();
    context.options_mut(|options| {
        options.max_passes = 4.try_into().expect("four is non-zero");
    });
    let mut dockspace = Dockspace::builder("product-discarded-presentation-menu", layout())
        .build()
        .expect("the multipass presentation-menu facade initializes");
    let mut panes = Panes;

    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (menu, _) = accesskit_node(&stable.output, Role::Button, "Layout actions");
    let _ = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(menu, Action::Click)],
    );
    let opened = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (float, float_node) = accesskit_node(&opened.output, Role::MenuItem, "Float Layout");
    assert!(!float_node.is_disabled());

    context
        .plugin_or_default::<LateDiscardPlugin>()
        .lock()
        .remaining = 2;
    let passes = run_two_discarded_dockspace_passes(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(float, Action::Click)],
    );
    assert_eq!(passes, 3, "the terminal pass intentionally omits dockspace");
    assert_eq!(
        dockspace
            .view()
            .surface(SURFACE)
            .unwrap()
            .contained()
            .count(),
        0
    );
    let delivered = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(delivered.close_requests.is_empty());
    let surface = dockspace
        .view()
        .surface(SURFACE)
        .expect("the logical surface remains present");
    assert_eq!(
        surface.contained().count(),
        1,
        "discarded passes must retain the local command once without replaying it"
    );
    assert!(surface.main_root().is_none());
    let committed_version = dockspace.version();
    for _ in 0..2 {
        run_frame(&context, &mut dockspace, &mut panes, Vec::new());
        assert_eq!(dockspace.version(), committed_version);
        assert_eq!(
            dockspace
                .view()
                .surface(SURFACE)
                .unwrap()
                .contained()
                .count(),
            1
        );
    }
}

#[test]
fn disabled_ui_does_not_open_or_activate_presentation_commands() {
    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace = Dockspace::builder("product-disabled-presentation-menu", layout())
        .build()
        .expect("the disabled presentation-menu facade initializes");
    let mut panes = Panes;

    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (menu, _) = accesskit_node(&stable.output, Role::Button, "Layout actions");
    let disabled = run_disabled_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(menu, Action::Click)],
    );
    let (_, node) = accesskit_node(&disabled.output, Role::Button, "Layout actions");
    assert!(node.is_disabled());
    assert!(!node.supports_action(Action::Click));
    assert!(
        dockspace
            .view()
            .item(SECOND)
            .and_then(|item| item.contained())
            .is_none(),
        "a delayed accessibility click must not float content through a disabled Ui",
    );

    let after = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let tree = after
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled");
    assert!(
        tree.nodes
            .iter()
            .all(|(_, node)| node.role() != Role::MenuItem),
        "the disabled anchor must not leave its popup open",
    );
}

#[test]
fn fully_occluded_main_presentation_menu_is_accessibility_disabled() {
    let context = Context::default();
    context.enable_accesskit();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::central_tabs([FIRST])),
    )
    .with_contained(DockspaceContainedLayout::new(
        FLOATING,
        DockspaceRootLayout::new(FLOATING_ROOT, DockspaceNode::tabs([SECOND])),
        LogicalRect::new(620.0, 0.0, 180.0, 180.0)
            .expect("the occluding contained rectangle is valid"),
    ))])
    .expect("the presentation-menu occlusion layout is valid");
    let mut dockspace = Dockspace::builder("product-occluded-presentation-menu", layout)
        .build()
        .expect("the occluded presentation-menu facade initializes");
    let mut panes = Panes;

    let _ = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let stable = run_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let tree = stable
        .output
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit is enabled");
    let anchors = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some("Layout actions"))
        .collect::<Vec<_>>();
    assert_eq!(anchors.len(), 2);
    let (occluded_id, occluded) = anchors
        .iter()
        .copied()
        .find(|(_, node)| node.is_disabled())
        .expect("the main anchor under the front contained window is disabled");
    assert!(!occluded.supports_action(Action::Click));
    assert!(
        anchors.iter().any(|(_, node)| !node.is_disabled()),
        "the front contained title keeps its own command menu operable",
    );

    let _ = run_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(*occluded_id, Action::Click)],
    );
    assert!(
        dockspace
            .view()
            .item(FIRST)
            .and_then(|item| item.contained())
            .is_none(),
        "an occluded main command anchor must not execute through the front window",
    );
}
