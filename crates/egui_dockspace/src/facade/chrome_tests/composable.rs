//! Nested layout tabs keep labels, content callbacks and header focus distinct.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use super::*;
use crate::{
    DockInputConfig, DockspaceActionOutcome, DockspaceActionStatus, DockspaceGridShape, Tab,
    TabKey, TabVisuals,
};

const A: ItemId = ItemId::new(1);
const B: ItemId = ItemId::new(2);
const C: ItemId = ItemId::new(3);
const D: ItemId = ItemId::new(4);

#[derive(Default)]
struct NestedPanes {
    calls: BTreeMap<ItemId, Rect>,
    tabs: BTreeMap<TabKey, String>,
    text: String,
}

impl PaneView for NestedPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(match item {
            A => "Editor A".into(),
            B => "Editor B".into(),
            C => "Document C".into(),
            D => "Bare D".into(),
            _ => panic!("layout labels must not call application ItemId callbacks"),
        })
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        self.calls.insert(item, ui.available_rect_before_wrap());
        let target = self.focus_target(item).unwrap();
        let response = ui.add(egui::TextEdit::singleline(&mut self.text).id(target));
        PaneResponse::from(&response)
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(egui::Id::unique(("nested-pane-editor", item)))
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        self.tabs.insert(tab.key, tab.title.job.text.clone());
        tab.paint(ui)
    }
}

fn setup() -> (Context, Dockspace, NestedPanes) {
    let context = Context::default();
    context.enable_accesskit();
    context.all_styles_mut(|style| style.animation_time = 0.0);
    let tools = DockspaceNode::equal_split(
        crate::DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([A, B]), DockspaceNode::pane(D)],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::composed_tabs([DockspaceNode::pane(C), tools]),
        ),
    )])
    .unwrap();
    let dockspace = Dockspace::builder("composable-chrome", layout)
        .style(DockStyle {
            splitter_thickness: 4.0,
            ..DockStyle::default()
        })
        .build()
        .unwrap();
    (context, dockspace, NestedPanes::default())
}

fn paint(
    context: &Context,
    dockspace: &mut Dockspace,
    panes: &mut NestedPanes,
    events: Vec<Event>,
) -> FullOutput {
    panes.calls.clear();
    panes.tabs.clear();
    let (output, _) = dockspace
        .run_single_surface(
            context,
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
                events,
                ..Default::default()
            },
            SURFACE,
            panes,
        )
        .unwrap();
    let (mut output, receipt) = output.into_parts();
    receipt.report(SurfacePresentationResult::Presented);
    output.textures_delta.clear();
    output
}

fn activate(dockspace: &mut Dockspace, item: ItemId) {
    let activated = dockspace.select_item_current(item).unwrap();
    assert!(matches!(
        activated.status(),
        DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemActivated {
            item: activated,
            ..
        }) if *activated == item
    ));
}

#[test]
fn layout_tab_retains_its_title_and_only_active_real_panes_receive_ui() {
    let (context, mut dockspace, mut panes) = setup();
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [C]);
    let c_rect = panes.calls[&C];
    let layout_key = *panes
        .tabs
        .iter()
        .find(|(_, title)| *title == "Tools")
        .unwrap()
        .0;
    assert!(layout_key.is_layout());
    assert_eq!(panes.tabs.len(), 2);

    let activated = dockspace.select_item_current(A).unwrap();
    assert!(matches!(
        activated.status(),
        DockspaceActionStatus::Applied(DockspaceActionOutcome::ItemActivated {
            item: A,
            changed: true
        })
    ));
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [A, D]);
    assert_eq!(panes.tabs[&layout_key], "Tools");
    assert_eq!(panes.tabs.len(), 4, "bare D has no implicit tab header");
    let a_rect = panes.calls[&A];
    let d_rect = panes.calls[&D];
    assert!((d_rect.top() - c_rect.top()).abs() <= 1.0);
    assert!((a_rect.top() - d_rect.top() - dockspace.style().tab_bar_height).abs() <= 1.0);
    assert!((d_rect.left() - a_rect.right() - dockspace.style().splitter_thickness).abs() <= 1.0);
    assert!(
        (a_rect.width() + d_rect.width() + dockspace.style().splitter_thickness - c_rect.width())
            .abs()
            <= 1.0
    );
    assert!(dockspace.view().item(D).unwrap().tabs().is_none());
    assert!(dockspace.view().item(A).unwrap().is_active());
    assert!(dockspace.view().item(D).unwrap().is_active());

    activate(&mut dockspace, B);
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [B, D]);
    assert_eq!(panes.tabs[&layout_key], "Tools");
    activate(&mut dockspace, C);
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [C]);
    assert_eq!(panes.tabs.len(), 2);
    assert!(!dockspace.view().item(B).unwrap().is_active());
    assert!(!dockspace.view().item(D).unwrap().is_active());
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Tab
                && matches!(node.label(), Some("Editor A" | "Editor B" | "Bare D")))
    );
}

#[test]
fn cycling_from_bare_pane_focus_uses_the_nearest_active_ancestor_bar() {
    let (context, mut dockspace, mut panes) = setup();
    dockspace.set_input(DockInputConfig {
        next_tab: Some(egui::KeyboardShortcut::new(egui::Modifiers::CTRL, Key::Tab)),
        ..Default::default()
    });
    activate(&mut dockspace, D);
    for _ in 0..3 {
        paint(&context, &mut dockspace, &mut panes, Vec::new());
    }
    context.memory_mut(|memory| memory.request_focus(panes.focus_target(D).unwrap()));
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    );
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(dockspace.view().item(C).unwrap().is_active());
    assert!(!dockspace.view().item(D).unwrap().is_active());
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [C]);
}

#[test]
fn an_active_nested_pane_accepts_a_real_tab_drop_from_its_sibling_group() {
    let context = Context::default();
    context.enable_accesskit();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::equal_split(
                crate::DockspaceAxis::Horizontal,
                [
                    DockspaceNode::tabs([A, B]),
                    DockspaceNode::composed_tabs([
                        DockspaceNode::pane(C),
                        DockspaceNode::tabs([D]).named("Nested"),
                    ]),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap();
    let mut dockspace = Dockspace::builder("nested-pointer-destination", layout)
        .build()
        .unwrap();
    let mut panes = NestedPanes::default();
    activate(&mut dockspace, D);
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    let (_, source) = nodes(&output)
        .iter()
        .find(|(_, node)| node.role() == Role::Tab && node.label() == Some("Editor B"))
        .unwrap();
    let bounds = source.bounds().unwrap();
    #[allow(clippy::cast_possible_truncation)]
    let source = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::PointerMoved(source)],
    );
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![button(source, true)],
    );
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::PointerMoved(source + vec2(20.0, 8.0))],
    );
    assert!(dockspace.has_active_gesture());
    let destination = panes.calls[&D].center();
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::PointerMoved(destination)],
    );
    let output = paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::PointerMoved(destination)],
    );
    let guide = nodes(&output)
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Label && node.label() == Some("Dockspace bottom drop guide")
        })
        .filter_map(|(_, node)| node.bounds())
        .min_by(|left, right| {
            let distance = |rect: &egui::accesskit::Rect| {
                ((rect.x0 + rect.x1) * 0.5 - f64::from(destination.x)).powi(2)
                    + ((rect.y0 + rect.y1) * 0.5 - f64::from(destination.y)).powi(2)
            };
            distance(left).total_cmp(&distance(right))
        })
        .unwrap_or_else(|| {
            panic!(
                "an active nested content destination exposes its guide; active_gesture={}",
                dockspace.has_active_gesture()
            )
        });
    #[allow(clippy::cast_possible_truncation)]
    let target = egui::pos2(
        ((guide.x0 + guide.x1) * 0.5) as f32,
        ((guide.y0 + guide.y1) * 0.5) as f32,
    );
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::PointerMoved(target)],
    );
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![button(target, false)],
    );
    assert!(!dockspace.has_active_gesture());
    let view = dockspace.view();
    assert!(view.item(B).unwrap().contained().is_none());
    assert_eq!(
        view.item(B)
            .unwrap()
            .tabs()
            .unwrap()
            .items()
            .collect::<Vec<_>>(),
        [B]
    );
    assert!(view.item(B).unwrap().is_active());
    assert!(view.item(D).unwrap().is_active());
    assert!(!view.item(C).unwrap().is_active());
    let outer = view.item(C).unwrap().tabs().unwrap();
    let Some(crate::DockspaceTabView::Layout(nested)) = outer.child(outer.selected().unwrap())
    else {
        panic!("the drop stays inside the selected nested layout");
    };
    assert_eq!(
        nested.split().unwrap().axis(),
        crate::DockspaceAxis::Vertical
    );
    assert_eq!(nested.items().collect::<Vec<_>>(), [D, B]);
}

fn bare_root() -> (Context, Dockspace, NestedPanes) {
    let context = Context::default();
    context.enable_accesskit();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), DockspaceNode::pane(D)),
    )])
    .unwrap();
    let dockspace = Dockspace::builder("bare-root-controls", layout)
        .build()
        .unwrap();
    (context, dockspace, NestedPanes::default())
}

#[test]
fn bare_root_has_a_visible_layout_menu_outside_application_content() {
    let (context, mut dockspace, mut panes) = bare_root();
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(
        panes.tabs.is_empty(),
        "bare content must not acquire a tab header"
    );
    let (anchor, node) = nodes(&output)
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Layout actions"))
        .expect("a bare main root has a visible default layout entry");
    let bounds = node.bounds().unwrap();
    assert!(bounds.y1 <= f64::from(panes.calls[&D].top()));
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: *anchor,
                data: None,
            },
        )],
    );
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(
        nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem
                && node.label() == Some("Float Layout")
                && !node.is_disabled())
    );
    assert!(
        nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem && node.label() == Some("Close Layout"))
    );
    paint(&context, &mut dockspace, &mut panes, vec![key(Key::Escape)]);
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [D]);
    assert!(dockspace.view().item(D).unwrap().tabs().is_none());
}

fn invoke(
    context: &Context,
    dockspace: &mut Dockspace,
    panes: &mut NestedPanes,
    output: &FullOutput,
    role: Role,
    label: &str,
) -> FullOutput {
    let target = nodes(output)
        .iter()
        .find_map(|(id, node)| {
            (node.role() == role && node.label() == Some(label) && !node.is_disabled())
                .then_some(*id)
        })
        .unwrap_or_else(|| panic!("missing enabled {role:?}: {label}"));
    paint(
        context,
        dockspace,
        panes,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: target,
                data: None,
            },
        )],
    );
    paint(context, dockspace, panes, Vec::new());
    paint(context, dockspace, panes, Vec::new())
}

#[test]
fn bare_root_default_layout_menu_floats_docks_back_and_requests_ordinary_content_close() {
    let (context, mut dockspace, mut panes) = bare_root();
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::Button,
        "Layout actions",
    );
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Float Layout",
    );
    assert!(dockspace.view().item(D).unwrap().contained().is_some());
    assert!(dockspace.view().item(D).unwrap().tabs().is_none());
    assert!(
        panes.tabs.is_empty(),
        "floating real bare content preserves its lack of a tab bar"
    );
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::Button,
        "Layout actions",
    );
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Dock Back",
    );
    assert!(dockspace.view().item(D).unwrap().contained().is_none());
    assert!(dockspace.view().item(D).unwrap().tabs().is_none());
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::Button,
        "Layout actions",
    );
    invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Close Layout",
    );
    let plans = dockspace
        .session
        .pending_content_close_plans()
        .collect::<Vec<_>>();
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0]
            .items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [D]
    );
    assert!(
        dockspace.view().item(D).is_some(),
        "the application still decides actual content closure"
    );
}

fn bare_composition(grid: bool) -> (Context, Dockspace, NestedPanes) {
    let context = Context::default();
    context.enable_accesskit();
    context.all_styles_mut(|style| style.animation_time = 0.0);
    let content = if grid {
        DockspaceNode::grid(
            DockspaceGridShape::Fixed {
                rows: NonZeroUsize::new(2).unwrap(),
                columns: NonZeroUsize::new(2).unwrap(),
            },
            [A, B, C, D].map(DockspaceNode::pane),
        )
        .unwrap()
    } else {
        DockspaceNode::equal_split(
            crate::DockspaceAxis::Horizontal,
            [A, D].map(DockspaceNode::pane),
        )
        .unwrap()
    };
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), content),
    )])
    .unwrap();
    let dockspace = Dockspace::builder("bare-composition-menu", layout)
        .build()
        .unwrap();
    (context, dockspace, NestedPanes::default())
}

fn open_pane_actions(
    context: &Context,
    dockspace: &mut Dockspace,
    panes: &mut NestedPanes,
    output: &FullOutput,
    title: &str,
) -> FullOutput {
    let output = invoke(
        context,
        dockspace,
        panes,
        output,
        Role::Button,
        "Layout actions",
    );
    let output = invoke(
        context,
        dockspace,
        panes,
        &output,
        Role::MenuItem,
        "Active Panes…",
    );
    invoke(context, dockspace, panes, &output, Role::MenuItem, title)
}

#[test]
fn bare_split_and_grid_panes_have_default_float_and_exact_close_actions() {
    for grid in [false, true] {
        for close in [false, true] {
            let (context, mut dockspace, mut panes) = bare_composition(grid);
            paint(&context, &mut dockspace, &mut panes, Vec::new());
            let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
            assert!(panes.tabs.is_empty());
            let anchor = nodes(&output)
                .iter()
                .find(|(_, node)| {
                    node.role() == Role::Button && node.label() == Some("Layout actions")
                })
                .unwrap()
                .1
                .bounds()
                .unwrap();
            assert!(
                panes
                    .calls
                    .values()
                    .all(|bounds| anchor.y1 <= f64::from(bounds.top()))
            );
            let output = open_pane_actions(&context, &mut dockspace, &mut panes, &output, "Bare D");
            assert!(
                nodes(&output)
                    .iter()
                    .any(|(_, node)| node.role() == Role::Menu
                        && node.label() == Some("Pane actions"))
            );
            let output = invoke(
                &context,
                &mut dockspace,
                &mut panes,
                &output,
                Role::MenuItem,
                if close { "Close Pane" } else { "Float Pane" },
            );
            assert!(dockspace.view().item(D).unwrap().tabs().is_none());
            assert!(dockspace.view().item(A).unwrap().contained().is_none());
            if grid {
                assert!(dockspace.view().item(B).unwrap().contained().is_none());
                assert!(dockspace.view().item(C).unwrap().contained().is_none());
            }
            if close {
                let plans = dockspace
                    .session
                    .pending_content_close_plans()
                    .collect::<Vec<_>>();
                assert_eq!(plans.len(), 1);
                assert_eq!(
                    plans[0]
                        .items()
                        .iter()
                        .map(|item| item.item())
                        .collect::<Vec<_>>(),
                    [D]
                );
                assert!(dockspace.view().item(D).unwrap().contained().is_none());
            } else {
                assert!(dockspace.view().item(D).unwrap().contained().is_some());
                assert!(
                    dockspace
                        .session
                        .pending_content_close_plans()
                        .next()
                        .is_none()
                );
            }
            assert!(
                !nodes(&output)
                    .iter()
                    .any(|(_, node)| node.role() == Role::MenuItem)
            );
        }
    }
}

#[test]
fn active_pane_pages_exclude_inactive_descendants_and_reject_old_actions_after_ancestor_switch() {
    let (context, mut dockspace, mut panes) = setup();
    activate(&mut dockspace, B);
    activate(&mut dockspace, D);
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::Button,
        "Layout actions",
    );
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Active Panes…",
    );
    for title in ["Editor A", "Document C"] {
        assert!(
            !nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::MenuItem && node.label() == Some(title))
        );
    }
    for title in ["Editor B", "Bare D"] {
        assert!(
            nodes(&output)
                .iter()
                .any(|(_, node)| node.role() == Role::MenuItem && node.label() == Some(title))
        );
    }
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Bare D",
    );
    let stale = nodes(&output)
        .iter()
        .find(|(_, node)| node.role() == Role::MenuItem && node.label() == Some("Float Pane"))
        .unwrap()
        .0;
    activate(&mut dockspace, C);
    let version = dockspace.version();
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: stale,
                data: None,
            },
        )],
    );
    assert_eq!(dockspace.version(), version);
    assert!(dockspace.view().item(D).unwrap().contained().is_none());
    assert_eq!(panes.calls.keys().copied().collect::<Vec<_>>(), [C]);
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
}

#[test]
fn active_pane_menu_keyboard_returns_through_pages_and_escape_restores_root_anchor() {
    let (context, mut dockspace, mut panes) = bare_composition(true);
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    let anchor = nodes(&output)
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Layout actions"))
        .unwrap()
        .0;
    open_pane_actions(&context, &mut dockspace, &mut panes, &output, "Bare D");
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![key(Key::ArrowLeft)],
    );
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(
        nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem && node.label() == Some("Bare D"))
    );
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem && node.label() == Some("Float Pane"))
    );
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![key(Key::ArrowRight)],
    );
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert!(
        nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::Menu && node.label() == Some("Pane actions"))
    );
    paint(&context, &mut dockspace, &mut panes, vec![key(Key::Escape)]);
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert_eq!(
        context.memory(egui::Memory::focused).unwrap().value(),
        anchor.0
    );
    assert!(
        !nodes(&output)
            .iter()
            .any(|(_, node)| node.role() == Role::MenuItem)
    );
}

#[test]
fn pane_menu_commands_do_not_retarget_old_accessibility_actions_to_another_pane_page() {
    let (context, mut dockspace, mut panes) = bare_composition(false);
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = open_pane_actions(&context, &mut dockspace, &mut panes, &output, "Bare D");
    let old_commands = ["Float Pane", "Close Pane"].map(|label| {
        nodes(&output)
            .iter()
            .find(|(_, node)| {
                node.role() == Role::MenuItem && node.label() == Some(label) && !node.is_disabled()
            })
            .unwrap()
            .0
    });
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Back to Active Panes",
    );
    let output = invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Editor A",
    );
    for (label, old) in ["Float Pane", "Close Pane"].into_iter().zip(old_commands) {
        let current = nodes(&output)
            .iter()
            .find(|(_, node)| node.role() == Role::MenuItem && node.label() == Some(label))
            .unwrap()
            .0;
        assert_ne!(
            current, old,
            "each command widget retains its real pane scope"
        );
    }
    let version = dockspace.version();
    let events = old_commands.map(|target_node| {
        Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node,
            data: None,
        })
    });
    paint(&context, &mut dockspace, &mut panes, events.into());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    assert_eq!(dockspace.version(), version);
    assert!(
        dockspace
            .session
            .pending_content_close_plans()
            .next()
            .is_none()
    );
    for item in [A, D] {
        assert!(dockspace.view().item(item).unwrap().contained().is_none());
    }
    invoke(
        &context,
        &mut dockspace,
        &mut panes,
        &output,
        Role::MenuItem,
        "Close Pane",
    );
    let plans = dockspace
        .session
        .pending_content_close_plans()
        .collect::<Vec<_>>();
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0]
            .items()
            .iter()
            .map(|item| item.item())
            .collect::<Vec<_>>(),
        [A]
    );
}

#[test]
fn real_grid_keyboard_and_accessibility_adjust_one_shared_track_across_every_cell() {
    use egui::accesskit::{Action, ActionRequest, Orientation, TreeId};

    let (context, mut dockspace, mut panes) = bare_composition(true);
    dockspace
        .set_style(DockStyle {
            splitter_thickness: 4.0,
            splitter_keyboard_step: 10.0,
            ..Default::default()
        })
        .unwrap();
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    let columns = nodes(&output)
        .iter()
        .filter(|(_, node)| node.role() == Role::Splitter && node.label() == Some("Resize panes"))
        .collect::<Vec<_>>();
    assert_eq!(
        columns.len(),
        2,
        "one boundary per shared axis, rather than one per cell"
    );
    let column = columns
        .iter()
        .find(|(_, node)| node.orientation() == Some(Orientation::Vertical))
        .unwrap()
        .0;
    let initial = panes.calls.clone();
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::AccessKitActionRequest(ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: column,
            data: None,
        })],
    );
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![key(Key::ArrowRight), key(Key::ArrowRight)],
    );
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    let output = paint(&context, &mut dockspace, &mut panes, Vec::new());
    for item in [A, C] {
        assert!((panes.calls[&item].width() - initial[&item].width() - 20.0).abs() < 1.0);
    }
    for item in [B, D] {
        assert!((panes.calls[&item].width() - initial[&item].width() + 20.0).abs() < 1.0);
    }
    let after_columns = panes.calls.clone();
    let row = nodes(&output)
        .iter()
        .find(|(_, node)| {
            node.role() == Role::Splitter
                && node.label() == Some("Resize panes")
                && node.orientation() == Some(Orientation::Horizontal)
        })
        .unwrap()
        .0;
    paint(
        &context,
        &mut dockspace,
        &mut panes,
        vec![Event::AccessKitActionRequest(ActionRequest {
            action: Action::Increment,
            target_tree: TreeId::ROOT,
            target_node: row,
            data: None,
        })],
    );
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    paint(&context, &mut dockspace, &mut panes, Vec::new());
    for item in [A, B] {
        assert!((panes.calls[&item].height() - initial[&item].height() - 10.0).abs() < 1.0);
    }
    for item in [C, D] {
        assert!((panes.calls[&item].height() - initial[&item].height() + 10.0).abs() < 1.0);
    }
    for item in [A, B, C, D] {
        assert!((panes.calls[&item].width() - after_columns[&item].width()).abs() < 1.0);
        assert!(dockspace.view().item(item).unwrap().tabs().is_none());
    }
}
