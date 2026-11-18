//! Replacing menu-button paint preserves the public interaction contract.

use dockspace::policy::{DockPolicy, TabBarInteraction, TabBarPolicy, TabBarVisibility};
use egui::accesskit::{Action, ActionRequest, HasPopup, Node, NodeId, Role, TreeId};
use egui::{Color32, Context, Event, FullOutput, Key, Pos2, RawInput, Rect, Ui, vec2};
use egui_dockspace::{
    DockMenuButton, DockMenuButtonKind, DockMenuContext, DockMenuTarget, DockStyle, Dockspace,
    DockspaceAxis, DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, FloatingPresentationId, ItemId, LogicalRect, PaneResponse, PaneView,
    RootId, SurfaceId, SurfacePresentationResult,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const MAIN: RootId = RootId::new(1);
const FLOATING: RootId = RootId::new(2);
const WIDTH: f32 = 900.0;
const CUSTOM_FILL: Color32 = Color32::from_rgb(21, 131, 117);
const FOCUS: Color32 = Color32::from_rgb(219, 34, 177);

#[derive(Clone, Debug)]
struct Snapshot {
    id: egui::Id,
    kind: DockMenuButtonKind,
    context: DockMenuContext,
    rect: Rect,
    hit: Rect,
    clip: Rect,
    enabled: bool,
    open: bool,
    hovered: bool,
    pressed: bool,
    focused: bool,
    ui_enabled: bool,
    icon_color: Color32,
    opacity: f32,
}

#[derive(Default)]
struct Buttons(Vec<Snapshot>);

impl PaneView for Buttons {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(format!("Document {}", item.get()).into())
    }

    fn ui(&mut self, _: ItemId, _: &mut Ui) -> PaneResponse {
        PaneResponse::default()
    }

    fn menu_button_ui(&mut self, ui: &Ui, button: &DockMenuButton<'_>) {
        self.0.push(Snapshot {
            id: button.response.id,
            kind: button.kind,
            context: button.context,
            rect: button.rect,
            hit: button.response.interact_rect,
            clip: ui.clip_rect(),
            enabled: button.enabled,
            open: button.open,
            hovered: button.response.hovered(),
            pressed: button.response.is_pointer_button_down_on(),
            focused: button.response.has_focus(),
            ui_enabled: ui.is_enabled(),
            icon_color: button.icon_color,
            opacity: ui.opacity(),
        });
        // Deliberate overdraw checks the callback's actual painter clip.
        ui.painter()
            .rect_filled(button.rect.expand(100.0), 9.0, CUSTOM_FILL);
        let mut appearance = *button;
        appearance.icon_color = Color32::WHITE;
        appearance.paint_icon(ui);
    }
}

struct Harness {
    context: Context,
    dockspace: Dockspace,
    panes: Box<Buttons>,
    callback: bool,
    enabled: bool,
    frame: u32,
}

impl Harness {
    fn new(callback: bool) -> Self {
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                MAIN,
                DockspaceNode::equal_split(
                    DockspaceAxis::Horizontal,
                    [
                        DockspaceNode::tabs((1..=5).map(ItemId::new)),
                        DockspaceNode::pane(ItemId::new(6)),
                    ],
                )
                .unwrap(),
            ),
        )
        .with_contained(DockspaceContainedLayout::new(
            FloatingPresentationId::new(1),
            DockspaceRootLayout::new(
                FLOATING,
                DockspaceNode::tabs([ItemId::new(7), ItemId::new(8)]),
            ),
            LogicalRect::new(500.0, 180.0, 320.0, 300.0).unwrap(),
        ))])
        .unwrap();
        let mut style = DockStyle {
            tab_min_width: 120.0,
            ..DockStyle::default()
        };
        style.visuals.tab_focus_stroke = Some(egui::Stroke::new(2.0, FOCUS));
        let context = Context::default();
        context.enable_accesskit();
        context.all_styles_mut(|style| style.animation_time = 0.0);
        Self {
            context,
            dockspace: Dockspace::builder("custom-menu-buttons", layout)
                .style(style)
                .build()
                .unwrap(),
            panes: Box::default(),
            callback,
            enabled: true,
            frame: 0,
        }
    }

    fn run(&mut self, events: Vec<Event>) -> FullOutput {
        self.panes.0.clear();
        self.frame += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 600.0))),
            time: Some(f64::from(self.frame) * 0.1),
            events,
            ..Default::default()
        };
        let output = if self.callback {
            egui_dockspace::run_ui(&self.context, raw, |ui| {
                if !self.enabled {
                    ui.disable();
                }
                self.dockspace
                    .show_single_surface(SURFACE, ui, &mut self.panes)
                    .unwrap();
            })
        } else {
            self.dockspace
                .run_single_surface(&self.context, raw, SURFACE, &mut self.panes)
                .unwrap()
                .0
        };
        let (mut output, receipt) = output.into_parts();
        receipt.report(SurfacePresentationResult::Presented);
        output.textures_delta.clear();
        output
    }

    fn settle(&mut self) -> FullOutput {
        self.run(Vec::new());
        self.run(Vec::new());
        self.run(Vec::new())
    }

    fn button(&self, kind: DockMenuButtonKind, root: RootId) -> Snapshot {
        self.panes
            .0
            .iter()
            .find(|button| button.kind == kind && button.context.root == root)
            .unwrap_or_else(|| panic!("missing {kind:?} in {root:?}: {:?}", self.panes.0))
            .clone()
    }
}

fn nodes(output: &FullOutput) -> &[(NodeId, Node)] {
    &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
}

fn action(id: egui::Id, action: Action) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: NodeId(id.value()),
        data: None,
    })
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn assert_expanded(output: &FullOutput, button: &Snapshot, expanded: bool) {
    let node = &nodes(output)
        .iter()
        .find(|(id, _)| id.0 == button.id.value())
        .expect("the existing accessible button remains registered")
        .1;
    assert_eq!(node.role(), Role::Button);
    assert_eq!(node.has_popup(), Some(HasPopup::Menu));
    assert_eq!(node.is_expanded(), Some(expanded));
    assert_eq!(button.open, expanded);
}

#[test]
fn custom_menu_buttons_keep_scope_clip_and_pointer_routing() {
    for callback in [false, true] {
        for (kind, root) in [
            (DockMenuButtonKind::Layout, MAIN),
            (DockMenuButtonKind::Layout, FLOATING),
            (DockMenuButtonKind::TabGroup, MAIN),
            (DockMenuButtonKind::TabList, MAIN),
        ] {
            let mut harness = Harness::new(callback);
            let output = harness.settle();
            let before = harness.button(kind, root);
            assert_eq!(before.context.surface, SURFACE);
            assert!(before.enabled && before.ui_enabled);
            assert!(before.rect.contains_rect(before.clip));
            match kind {
                DockMenuButtonKind::Layout => {
                    assert_eq!(before.context.target, DockMenuTarget::Root)
                }
                _ => assert!(matches!(before.context.target, DockMenuTarget::TabGroup(_))),
            }
            assert_expanded(&output, &before, false);
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == CUSTOM_FILL
                    && rect.rect == before.rect.expand(100.0))
                        && shape.clip_rect == before.clip
                }),
                "the replacement paint must use the supplied clipped painter"
            );

            let pos = before.hit.center();
            harness.run(vec![Event::PointerMoved(pos)]);
            assert!(harness.button(kind, root).hovered);
            for pressed in [true, false] {
                harness.run(vec![Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
                if pressed {
                    assert!(harness.button(kind, root).pressed);
                }
            }
            let output = harness.settle();
            let opened = harness.button(kind, root);
            assert_expanded(&output, &opened, true);
            assert_eq!(opened.id, before.id);
            assert_eq!(opened.context, before.context);
            assert_eq!(opened.rect, before.rect);
            assert_eq!(opened.hit, before.hit);
            assert!(!harness.dockspace.has_active_gesture());
            harness.run(vec![key(Key::Escape)]);
            let output = harness.settle();
            let closed = harness.button(kind, root);
            assert_expanded(&output, &closed, false);
            assert!(
                closed.focused,
                "Escape returns focus to the original button"
            );
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == closed.rect
                    && rect.stroke.color == FOCUS)
                }),
                "the adapter must retain a focus outline over replacement paint"
            );
        }
    }
}

#[test]
fn custom_menu_buttons_keep_keyboard_and_accessibility_activation() {
    for callback in [false, true] {
        for kind in [
            DockMenuButtonKind::Layout,
            DockMenuButtonKind::TabGroup,
            DockMenuButtonKind::TabList,
        ] {
            for keyboard in [false, true] {
                let mut harness = Harness::new(callback);
                harness.settle();
                let source = harness.button(kind, MAIN);
                if keyboard {
                    for _ in 0..100 {
                        if harness.context.memory(|memory| memory.has_focus(source.id)) {
                            break;
                        }
                        harness.run(vec![key(Key::Tab)]);
                    }
                    assert!(harness.context.memory(|memory| memory.has_focus(source.id)));
                    harness.run(vec![key(Key::Space)]);
                } else {
                    harness.run(vec![action(source.id, Action::Click)]);
                }
                let output = harness.settle();
                assert_expanded(&output, &harness.button(kind, MAIN), true);
                harness.run(vec![key(Key::Escape)]);
                let output = harness.settle();
                assert_expanded(&output, &harness.button(kind, MAIN), false);
            }
        }
    }
}

#[test]
fn disabled_menu_buttons_keep_custom_paint_without_accepting_activation() {
    let mut harness = Harness::new(true);
    harness.settle();
    harness.enabled = false;
    let output = harness.settle();
    for button in &harness.panes.0 {
        assert!(!button.enabled);
        assert!(!button.ui_enabled);
        assert_expanded(&output, button, false);
        let node = &nodes(&output)
            .iter()
            .find(|(id, _)| id.0 == button.id.value())
            .unwrap()
            .1;
        assert!(node.is_disabled());
        assert!(!node.supports_action(Action::Click));
    }
    let source = harness.button(DockMenuButtonKind::Layout, MAIN);
    harness.run(vec![action(source.id, Action::Click)]);
    assert!(harness.panes.0.iter().all(|button| !button.open));
    harness.enabled = true;
    harness.settle();
    harness.run(vec![action(source.id, Action::Click)]);
    let output = harness.settle();
    assert_expanded(
        &output,
        &harness.button(DockMenuButtonKind::Layout, MAIN),
        true,
    );
}

#[test]
fn a_core_disabled_tab_list_applies_disabled_opacity_exactly_once() {
    for callback in [false, true] {
        let mut harness = Harness::new(callback);
        let color = Color32::from_rgb(80, 150, 210);
        let mut style = harness.dockspace.style().clone();
        style.visuals.tab_text_color = Some(color);
        harness.dockspace.set_style(style).unwrap();
        let mut policy = DockPolicy::default();
        policy.set_tab_bar(TabBarPolicy::new(
            TabBarVisibility::Visible,
            TabBarInteraction::Disabled,
        ));
        harness.dockspace.set_policy(policy).unwrap();
        harness.settle();
        let button = harness.button(DockMenuButtonKind::TabList, MAIN);
        assert!(!button.enabled);
        assert!(!button.ui_enabled);
        assert_eq!(
            button.icon_color, color,
            "descriptor colors precede the child Ui's opacity"
        );
        assert_eq!(
            button.opacity,
            harness.context.global_style().visuals.disabled_alpha()
        );
        harness.run(vec![action(button.id, Action::Click)]);
        harness.settle();
        assert!(!harness.button(DockMenuButtonKind::TabList, MAIN).open);
    }
}
