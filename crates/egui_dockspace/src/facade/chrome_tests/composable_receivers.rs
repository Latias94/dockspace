//! Composite layouts keep menu, header and splitter receivers in their actual scope.

use std::num::NonZeroUsize;

use super::*;
use crate::{
    DockspaceAxis, DockspaceContainedLayout, DockspaceGridShape, DockspaceTabView,
    FloatingPresentationId, LogicalRect, NodeKey, TabKey,
};
use egui::accesskit::{Action, ActionRequest, Orientation, TreeId};

const WIDTH: f32 = 900.0;
const ROOT: RootId = RootId::new(1);
const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);

fn setup() -> Harness {
    let mut harness = Harness::new(
        ["Editor A", "Editor B", "Document C", "Bare D"]
            .map(str::to_owned)
            .to_vec(),
        A,
        DockStyle::default(),
    );
    let tools = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [
            DockspaceNode::tabs([A, B]).named("Documents"),
            DockspaceNode::pane(D),
        ],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs_with_selection([DockspaceNode::pane(C), tools], Some(1))
                .unwrap(),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("composable-receiver-scopes", layout)
        .build()
        .unwrap();
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness) -> FullOutput {
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new());
    harness.run(WIDTH, Vec::new())
}

fn node<'a>(output: &'a FullOutput, role: Role, label: &str) -> (NodeId, &'a Node) {
    nodes(output)
        .iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .map(|(id, node)| (*id, node))
        .unwrap_or_else(|| panic!("missing accessible {role:?}: {label}"))
}

#[allow(clippy::cast_possible_truncation)]
fn bounds(node: &Node) -> Rect {
    let rect = node.bounds().unwrap();
    Rect::from_min_max(
        egui::pos2(rect.x0 as f32, rect.y0 as f32),
        egui::pos2(rect.x1 as f32, rect.y1 as f32),
    )
}

fn button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click_named(harness: &mut Harness, output: &FullOutput, role: Role, label: &str) -> FullOutput {
    let (_, target) = node(output, role, label);
    assert!(!target.is_disabled(), "{label} must be enabled");
    click_point(harness, bounds(target).center())
}

fn click_point(harness: &mut Harness, point: Pos2) -> FullOutput {
    harness.run(WIDTH, vec![Event::PointerMoved(point)]);
    harness.run(
        WIDTH,
        vec![button(point, egui::PointerButton::Primary, true)],
    );
    harness.run(
        WIDTH,
        vec![button(point, egui::PointerButton::Primary, false)],
    );
    settle(harness)
}

fn action(target: NodeId, action: Action) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    })
}

fn assert_active(harness: &Harness, expected: &[ItemId]) {
    assert_eq!(
        [A, B, C, D]
            .into_iter()
            .filter(|item| harness.dockspace.view().item(*item).unwrap().is_active())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn a_tab_in_the_main_layout_does_not_offer_center_docking_as_dock_back() {
    let mut harness = setup();
    harness.focus_tab(WIDTH, "Editor A");
    harness.run(WIDTH, vec![key(Key::Menu)]);
    let output = settle(&mut harness);
    node(&output, Role::Menu, "Tab actions");
    let (_, center) = node(&output, Role::MenuItem, "Dock Center");
    assert!(
        !center.is_disabled(),
        "moving a tab to another group remains available"
    );
    assert!(
        nodes(&output).iter().all(|(_, node)| {
            node.role() != Role::MenuItem || node.label() != Some("Dock Back") || node.is_disabled()
        }),
        "a tab already in the main layout has no floating presentation to dock back; center docking has its own command"
    );
    assert_active(&harness, &[A, D]);
}

#[test]
fn a_main_bare_pane_menu_does_not_offer_center_docking_as_dock_back() {
    let mut harness = setup();
    let info = egui::Id::unique("pane-information");
    harness.panes.customize_menu = Some(|menu| {
        if menu.context().target == crate::DockMenuTarget::Pane(D) {
            menu.items.insert(
                0,
                crate::DockMenuItem::custom(
                    egui::Id::unique("pane-information"),
                    "Pane information",
                ),
            );
        }
    });
    let output = settle(&mut harness);
    let output = click_named(&mut harness, &output, Role::Button, "Layout actions");
    let output = click_named(&mut harness, &output, Role::MenuItem, "Active Panes…");
    let output = click_named(&mut harness, &output, Role::MenuItem, "Bare D");
    node(&output, Role::Menu, "Pane actions");
    let (_, center) = node(&output, Role::MenuItem, "Dock Center");
    assert!(
        !center.is_disabled(),
        "an explicit pane center move remains available"
    );
    assert!(
        nodes(&output).iter().all(|(_, node)| {
            node.role() != Role::MenuItem || node.label() != Some("Dock Back") || node.is_disabled()
        }),
        "a bare pane already in the main layout must not relabel an unrelated group move as Dock Back"
    );
    assert!(harness.dockspace.view().item(D).unwrap().tabs().is_none());
    assert_active(&harness, &[A, D]);
    click_named(&mut harness, &output, Role::MenuItem, "Pane information");
    assert_eq!(
        harness.panes.menu_actions,
        [crate::DockMenuAction {
            context: crate::DockMenuContext {
                surface: SURFACE,
                root: ROOT,
                target: crate::DockMenuTarget::Pane(D)
            },
            id: info,
        }]
    );
}

const E: ItemId = ItemId::new(5);
const FLOATING_ROOT: RootId = RootId::new(2);
const FLOATING: FloatingPresentationId = FloatingPresentationId::new(1);

#[derive(Clone, Copy, Debug)]
enum FloatingScope {
    Tab,
    LayoutTab,
    BarePane,
}

fn floating_composition() -> Harness {
    let mut harness = Harness::new(
        ["Editor A", "Editor B", "Document C", "Bare D", "Document E"]
            .map(str::to_owned)
            .to_vec(),
        C,
        DockStyle::default(),
    );
    let tools = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([A, B]), DockspaceNode::pane(D)],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, DockspaceNode::tabs([C])),
    )
    .with_contained(DockspaceContainedLayout::new(
        FLOATING,
        DockspaceRootLayout::new(
            FLOATING_ROOT,
            DockspaceNode::composed_tabs_with_selection([DockspaceNode::pane(E), tools], Some(1))
                .unwrap()
                .named("Floating workspace"),
        ),
        LogicalRect::new(180.0, 100.0, 620.0, 420.0).unwrap(),
    ))])
    .unwrap();
    harness.dockspace = Dockspace::builder("floating-exact-scope-dock-back", layout)
        .build()
        .unwrap();
    settle(&mut harness);
    harness
}

fn open_floating_scope(harness: &mut Harness, scope: FloatingScope) -> FullOutput {
    let output = settle(harness);
    match scope {
        FloatingScope::Tab | FloatingScope::LayoutTab => {
            let title = match scope {
                FloatingScope::Tab => "Editor A",
                _ => "Tools",
            };
            let tab = node(&output, Role::Tab, title).0;
            harness.run(WIDTH, vec![action(tab, Action::ShowContextMenu)]);
            settle(harness)
        }
        FloatingScope::BarePane => {
            let title = bounds(node(&output, Role::TitleBar, "Floating workspace").1);
            let anchor = nodes(&output)
                .iter()
                .find(|(_, node)| {
                    node.role() == Role::Button
                        && node.label() == Some("Layout actions")
                        && (title.top()..=title.bottom()).contains(&bounds(node).center().y)
                })
                .expect("the floating root owns its title-bar layout menu");
            let point = bounds(&anchor.1).center();
            let output = click_point(harness, point);
            let output = click_named(harness, &output, Role::MenuItem, "Active Panes…");
            click_named(harness, &output, Role::MenuItem, "Bare D")
        }
    }
}

fn assert_floating_dock_back_scope(
    harness: &Harness,
    scope: FloatingScope,
    main: NodeKey,
    outer_key: NodeKey,
    tools: TabKey,
    tools_key: NodeKey,
) {
    let view = harness.dockspace.view();
    let destination = view.item(C).unwrap().tabs().unwrap();
    assert_eq!(destination.key(), main);
    let expected = match scope {
        FloatingScope::Tab => TabKey::for_item(A),
        FloatingScope::LayoutTab => tools,
        FloatingScope::BarePane => TabKey::for_item(D),
    };
    assert_eq!(destination.tabs(), &[C.into(), expected], "{scope:?}");
    assert_eq!(destination.selected(), Some(expected));
    let source = view.root(FLOATING_ROOT).unwrap().content().unwrap();
    assert_eq!(source.key(), outer_key);
    assert_eq!(source.layout_title(), Some("Floating workspace"));
    assert_eq!(view.item(E).unwrap().contained(), Some(FLOATING));
    let moved = match scope {
        FloatingScope::Tab => vec![A],
        FloatingScope::LayoutTab => vec![A, B, D],
        FloatingScope::BarePane => vec![D],
    };
    for item in [A, B, D] {
        let location = view.item(item).unwrap();
        assert_eq!(
            location.root(),
            if moved.contains(&item) {
                ROOT
            } else {
                FLOATING_ROOT
            }
        );
        assert_eq!(
            location.contained(),
            (!moved.contains(&item)).then_some(FLOATING)
        );
    }
    if matches!(scope, FloatingScope::LayoutTab) {
        assert_eq!(source.tabs().unwrap().tabs(), &[E.into()]);
        assert_eq!(source.tabs().unwrap().selected(), Some(E.into()));
        let layout = view.node(tools_key).unwrap();
        assert_eq!(layout.layout_title(), Some("Tools"));
        assert_eq!(layout.items().collect::<Vec<_>>(), [A, B, D]);
        assert!(view.item(D).unwrap().tabs().is_none());
        assert_eq!(
            view.item(A).unwrap().tabs().unwrap().selected(),
            Some(B.into())
        );
    } else {
        assert_eq!(source.tabs().unwrap().tabs(), &[E.into(), tools]);
        assert_eq!(source.tabs().unwrap().selected(), Some(tools));
    }
    let mut owned = view
        .root(ROOT)
        .unwrap()
        .items()
        .chain(source.items())
        .collect::<Vec<_>>();
    owned.sort_unstable();
    assert_eq!(owned, [A, B, C, D, E]);
}

#[test]
fn floating_dock_back_keeps_exact_tab_layout_and_bare_pane_scopes_and_expires_old_commands() {
    for scope in [
        FloatingScope::Tab,
        FloatingScope::LayoutTab,
        FloatingScope::BarePane,
    ] {
        let mut harness = floating_composition();
        let outer = harness
            .dockspace
            .view()
            .root(FLOATING_ROOT)
            .unwrap()
            .content()
            .unwrap();
        let outer_key = outer.key();
        let tools = outer.tabs().unwrap().tabs()[1];
        let Some(DockspaceTabView::Layout(layout)) = outer.tabs().unwrap().child(tools) else {
            panic!("Tools retains its independent layout identity");
        };
        let tools_key = layout.key();
        let main = harness
            .dockspace
            .view()
            .item(C)
            .unwrap()
            .tabs()
            .unwrap()
            .key();
        let output = open_floating_scope(&mut harness, scope);
        let (stale, command) = node(&output, Role::MenuItem, "Dock Back");
        assert!(!command.is_disabled(), "floating {scope:?} can dock back");

        let activated = harness.dockspace.select_item_current(B).unwrap();
        assert!(matches!(
            activated.status(),
            crate::DockspaceActionStatus::Applied(crate::DockspaceActionOutcome::ItemActivated {
                item: B,
                changed: true
            })
        ));
        let version = harness.dockspace.version();
        let output = settle(&mut harness);
        assert!(
            !nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::MenuItem)
        );
        harness.run(WIDTH, vec![action(stale, Action::Click)]);
        settle(&mut harness);
        assert_eq!(
            harness.dockspace.version(),
            version,
            "old {scope:?} command is inert"
        );
        for item in [A, B, D, E] {
            assert_eq!(
                harness.dockspace.view().item(item).unwrap().contained(),
                Some(FLOATING)
            );
        }
        assert_eq!(
            harness
                .dockspace
                .view()
                .item(C)
                .unwrap()
                .tabs()
                .unwrap()
                .tabs(),
            &[C.into()]
        );

        let output = open_floating_scope(&mut harness, scope);
        click_named(&mut harness, &output, Role::MenuItem, "Dock Back");
        assert_floating_dock_back_scope(&harness, scope, main, outer_key, tools, tools_key);
    }
}

#[test]
fn header_keys_enter_the_layouts_real_content_and_nested_keys_keep_their_own_strip() {
    let mut harness = setup();
    let output = settle(&mut harness);
    let stale_inner_tab = node(&output, Role::Tab, "Editor B").0;
    let root = harness
        .dockspace
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap();
    let tools = root.tabs().unwrap().tabs()[1];
    harness.focus_tab(WIDTH, "Tools");
    harness.run(WIDTH, vec![key(Key::Home), key(Key::Enter)]);
    settle(&mut harness);
    assert_active(&harness, &[C]);
    assert_eq!(
        harness.context.memory(egui::Memory::focused),
        harness.panes.focus_target(C)
    );
    let version = harness.dockspace.version();
    harness.run(WIDTH, vec![action(stale_inner_tab, Action::Click)]);
    let output = settle(&mut harness);
    assert_eq!(harness.dockspace.version(), version);
    assert_active(&harness, &[C]);
    assert!(!nodes(&output).iter().any(|(_, node)| {
        node.role() == Role::Tab && matches!(node.label(), Some("Editor A" | "Editor B"))
    }));

    harness.focus_tab(WIDTH, "Document C");
    harness.run(WIDTH, vec![key(Key::ArrowRight), key(Key::Enter)]);
    settle(&mut harness);
    assert_active(&harness, &[A, D]);
    assert_eq!(
        harness.context.memory(egui::Memory::focused),
        harness.panes.focus_target(A),
        "activating Tools enters its first active real pane, without a fake layout focus target"
    );
    harness.focus_tab(WIDTH, "Editor A");
    harness.run(WIDTH, vec![key(Key::End), key(Key::Enter)]);
    settle(&mut harness);
    assert_active(&harness, &[B, D]);
    let root = harness
        .dockspace
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap();
    assert_eq!(root.tabs().unwrap().selected(), Some(tools));
    assert_eq!(
        harness
            .dockspace
            .view()
            .item(A)
            .unwrap()
            .tabs()
            .unwrap()
            .selected(),
        Some(B.into())
    );
    assert_eq!(
        harness.context.memory(egui::Memory::focused),
        harness.panes.focus_target(B)
    );
}

fn grid_harness() -> Harness {
    let mut harness = Harness::new(
        (1..=6).map(|item| format!("Document {item}")).collect(),
        B,
        DockStyle::default(),
    );
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::grid(
                DockspaceGridShape::Fixed {
                    rows: NonZeroUsize::new(2).unwrap(),
                    columns: NonZeroUsize::new(2).unwrap(),
                },
                [
                    DockspaceNode::pane(A),
                    DockspaceNode::tabs([B, C]),
                    DockspaceNode::pane(D),
                    DockspaceNode::tabs([ItemId::new(5), ItemId::new(6)]),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    harness.dockspace = Dockspace::builder("composable-drop-then-resize", layout)
        .build()
        .unwrap();
    settle(&mut harness);
    harness
}

fn local_vertical_splitter(output: &FullOutput) -> (NodeId, &Node) {
    nodes(output)
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Splitter
                && node.label() == Some("Resize panes")
                && node.orientation() == Some(Orientation::Vertical)
        })
        .min_by(|(_, left), (_, right)| {
            bounds(left).center().x.total_cmp(&bounds(right).center().x)
        })
        .map(|(id, node)| (*id, node))
        .expect("the newly inserted local splitter is exposed")
}

fn drag_pointer(harness: &mut Harness, source: Pos2, destination: Pos2) {
    harness.run(WIDTH, vec![Event::PointerMoved(source)]);
    harness.run(
        WIDTH,
        vec![button(source, egui::PointerButton::Primary, true)],
    );
    harness.run(WIDTH, vec![Event::PointerMoved(destination)]);
    harness.run(
        WIDTH,
        vec![button(destination, egui::PointerButton::Primary, false)],
    );
}

#[test]
fn an_inner_edge_tab_drop_leaves_the_new_splitter_ready_for_pointer_and_key_resize() {
    let mut harness = grid_harness();
    let output = settle(&mut harness);
    let grid = harness
        .dockspace
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap()
        .key();
    let source = bounds(node(&output, Role::Tab, "Document 3").1).center();
    let target = egui::pos2(200.0, 160.0);
    harness.run(WIDTH, vec![Event::PointerMoved(source)]);
    harness.run(
        WIDTH,
        vec![button(source, egui::PointerButton::Primary, true)],
    );
    harness.run(WIDTH, vec![Event::PointerMoved(source + vec2(20.0, 8.0))]);
    assert!(harness.dockspace.has_active_gesture());
    harness.run(WIDTH, vec![Event::PointerMoved(target)]);
    let output = harness.run(WIDTH, vec![Event::PointerMoved(target)]);
    let guide = nodes(&output)
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Label && node.label() == Some("Dockspace right drop guide")
        })
        .map(|(_, node)| bounds(node).center())
        .min_by(|left, right| {
            left.distance_sq(target)
                .total_cmp(&right.distance_sq(target))
        })
        .expect("bare A exposes its own inner right guide");
    harness.run(WIDTH, vec![Event::PointerMoved(guide)]);
    harness.run(
        WIDTH,
        vec![button(guide, egui::PointerButton::Primary, false)],
    );
    let output = settle(&mut harness);
    assert!(!harness.dockspace.has_active_gesture());
    let root = harness
        .dockspace
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap();
    assert_eq!(root.key(), grid);
    let cell = root.grid().unwrap().children().next().unwrap();
    let split = cell
        .split()
        .expect("the drop splits A's cell, rather than the whole grid");
    assert_eq!(split.axis(), DockspaceAxis::Horizontal);
    assert_eq!(cell.items().collect::<Vec<_>>(), [A, C]);
    assert_eq!(split.children().next().unwrap().item(), Some(A));
    let source_group = harness.dockspace.view().item(B).unwrap().tabs().unwrap();
    assert_eq!(source_group.tabs(), &[B.into()]);
    let cell_key = cell.key();
    let before = split.weights().collect::<Vec<_>>();
    let (_, splitter) = local_vertical_splitter(&output);
    let point = bounds(splitter).center();
    let moved = point + vec2(32.0, 0.0);
    drag_pointer(&mut harness, point, moved);
    let output = settle(&mut harness);
    let after = harness
        .dockspace
        .view()
        .node(cell_key)
        .unwrap()
        .split()
        .unwrap()
        .weights()
        .collect::<Vec<_>>();
    assert!(after[0] > before[0] && after[1] < before[1]);
    assert!(!harness.dockspace.has_active_gesture());
    let (splitter_id, splitter) = local_vertical_splitter(&output);
    assert!((bounds(splitter).center().x - moved.x).abs() < 1.0);
    harness.run(WIDTH, vec![action(splitter_id, Action::Focus)]);
    harness.run(WIDTH, vec![key(Key::ArrowLeft)]);
    settle(&mut harness);
    let final_view = harness.dockspace.view();
    let weights = final_view
        .node(cell_key)
        .unwrap()
        .split()
        .unwrap()
        .weights()
        .collect::<Vec<_>>();
    assert!(weights[0] < after[0] && weights[1] > after[1]);
    let root = final_view.root(ROOT).unwrap().content().unwrap();
    assert_eq!(root.key(), grid);
    assert!(root.grid().unwrap().shares().is_none());
    let mut items = root.items().collect::<Vec<_>>();
    items.sort_unstable();
    assert_eq!(items, (1..=6).map(ItemId::new).collect::<Vec<_>>());
    assert_eq!(
        final_view.item(C).unwrap().tabs().unwrap().selected(),
        Some(C.into())
    );
    assert_eq!(
        final_view.item(B).unwrap().tabs().unwrap().selected(),
        Some(B.into())
    );
}
