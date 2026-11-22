//! Unowned callbacks must retire tab-list clicks at window-focus boundaries.

use egui::accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Ui, vec2};

use crate::{
    DockStyle, Dockspace, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, PaneResponse, PaneView, RootId, SurfaceId,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);

struct Panes;

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Some(if item == FIRST { "A" } else { "B" }.into())
    }

    fn ui(&mut self, _: ItemId, _: &mut Ui) -> PaneResponse {
        PaneResponse::default()
    }
}

struct Harness {
    context: Context,
    dockspace: Dockspace,
    nodes: Vec<(NodeId, Node)>,
    frame: u32,
}

impl Harness {
    fn open() -> Self {
        let context = Context::default();
        context.enable_accesskit();
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::central_tabs_with_selection([FIRST, SECOND], Some(SECOND)).unwrap(),
            ),
        )])
        .unwrap();
        let mut harness = Self {
            context,
            dockspace: Dockspace::builder("unowned-tab-list-focus", layout)
                .style(DockStyle {
                    tab_min_width: 400.0,
                    tab_max_width: 400.0,
                    ..Default::default()
                })
                .build()
                .unwrap(),
            nodes: Vec::new(),
            frame: 0,
        };
        harness.settle();
        let trigger = harness.node(Role::Button, "List tabs").0;
        harness.step(vec![Event::AccessKitActionRequest(ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: trigger,
            data: None,
        })]);
        harness.settle();
        harness.node(Role::MenuItem, "A");
        harness.node(Role::MenuItem, "B");
        assert!(harness.dockspace.view().item(SECOND).unwrap().is_active());
        harness
    }

    fn frame(&mut self, focused: bool, events: Vec<Event>) {
        self.frame += 1;
        // This deliberately uses egui's ordinary callback without a Dockspace
        // terminal-run owner, exercising the LocalResponses fallback.
        let mut output = self.context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))),
                time: Some(f64::from(self.frame) * 0.05),
                focused,
                events,
                ..Default::default()
            },
            |ui| {
                let response = self
                    .dockspace
                    .show_single_surface(SURFACE, ui, &mut Panes)
                    .unwrap();
                if self.frame > 3 {
                    let capabilities = response.interaction_capabilities();
                    assert!(capabilities.local_actions_current());
                    assert!(!capabilities.pointer_receivers_current());
                }
            },
        );
        output.textures_delta.clear();
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
    }

    fn step(&mut self, events: Vec<Event>) {
        self.frame(true, events);
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.step(Vec::new());
        }
    }

    fn node(&self, role: Role, label: &str) -> &(NodeId, Node) {
        self.nodes
            .iter()
            .find(|(_, node)| node.role() == role && node.label() == Some(label))
            .unwrap_or_else(|| panic!("missing {role:?} {label:?}"))
    }

    fn row_center(&self, label: &str) -> Pos2 {
        let bounds = self.node(Role::MenuItem, label).1.bounds().unwrap();
        egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        )
    }

    fn click(&mut self, position: Pos2) {
        self.step(vec![Event::PointerMoved(position)]);
        self.step(vec![primary(position, true)]);
        self.step(vec![primary(position, false)]);
        self.settle();
    }
}

fn primary(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    SamePassRegain,
    SeparatePassRegain,
    UnfocusedSnapshot,
}

impl Boundary {
    fn release(self, harness: &mut Harness, position: Pos2) {
        match self {
            Self::SamePassRegain => harness.step(vec![
                Event::WindowFocused(false),
                Event::WindowFocused(true),
                primary(position, false),
            ]),
            Self::SeparatePassRegain => {
                harness.frame(false, vec![Event::WindowFocused(false)]);
                harness.step(vec![Event::WindowFocused(true), primary(position, false)]);
            }
            Self::UnfocusedSnapshot => harness.frame(false, vec![primary(position, false)]),
        }
        harness.settle();
    }
}

#[test]
fn unowned_tab_list_row_rejects_a_release_after_window_focus_loss() {
    for boundary in [
        Boundary::SamePassRegain,
        Boundary::SeparatePassRegain,
        Boundary::UnfocusedSnapshot,
    ] {
        let mut harness = Harness::open();
        let position = harness.row_center("A");
        harness.step(vec![Event::PointerMoved(position)]);
        harness.step(vec![primary(position, true)]);
        boundary.release(&mut harness, position);
        assert!(
            harness.dockspace.view().item(SECOND).unwrap().is_active(),
            "{boundary:?}: a cancelled press must not activate a menu row"
        );
        harness.node(Role::MenuItem, "A");
        harness.click(position);
        assert!(harness.dockspace.view().item(FIRST).unwrap().is_active());
    }
}

#[test]
fn unowned_tab_list_backdrop_rejects_a_release_after_window_focus_loss() {
    for boundary in [
        Boundary::SamePassRegain,
        Boundary::SeparatePassRegain,
        Boundary::UnfocusedSnapshot,
    ] {
        let mut harness = Harness::open();
        let position = egui::pos2(620.0, 460.0);
        harness.step(vec![Event::PointerMoved(position)]);
        harness.step(vec![primary(position, true)]);
        boundary.release(&mut harness, position);
        harness.node(Role::MenuItem, "A");
        harness.click(position);
        assert!(
            harness
                .nodes
                .iter()
                .all(|(_, node)| node.role() != Role::MenuItem),
            "{boundary:?}: a fresh backdrop click must still dismiss the menu"
        );
    }
}

#[test]
fn unowned_tab_list_uses_the_first_confirmation_in_pointer_and_keyboard_packets() {
    for backdrop in [false, true] {
        for pointer_first in [false, true] {
            for batched in [false, true] {
                let mut harness = Harness::open();
                let position = if backdrop {
                    egui::pos2(620.0, 460.0)
                } else {
                    harness.row_center("B")
                };
                harness.step(vec![Event::PointerMoved(position)]);
                harness.step(vec![primary(position, true)]);
                let keyboard = vec![
                    Event::AccessKitActionRequest(ActionRequest {
                        action: Action::Focus,
                        target_tree: TreeId::ROOT,
                        target_node: harness.node(Role::MenuItem, "A").0,
                        data: None,
                    }),
                    Event::Key {
                        key: Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    },
                ];
                let mut packets = [vec![primary(position, false)], keyboard];
                if !pointer_first {
                    packets.reverse();
                }
                if batched {
                    harness.step(packets.concat());
                } else {
                    for packet in packets {
                        harness.step(packet);
                    }
                }
                harness.settle();
                let expected = if pointer_first { SECOND } else { FIRST };
                let case = (backdrop, pointer_first, batched);
                assert!(
                    harness.dockspace.view().item(expected).unwrap().is_active(),
                    "{case:?}"
                );
                assert!(
                    harness
                        .nodes
                        .iter()
                        .all(|(_, node)| node.role() != Role::MenuItem),
                    "{case:?}"
                );
            }
        }
    }
}
