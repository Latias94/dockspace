//! Mixed semantic input interleaved with the existing pointer journey grammar.

use egui::accesskit::{Action, ActionRequest, Orientation, TreeId};

use super::*;

impl Replay {
    pub(super) fn adjustment_batch(&mut self) {
        let mut controls = Vec::new();
        for label in [
            "Resize panes",
            "Resize pane grid horizontally",
            "Resize pane grid vertically",
            "Resize floating right edge",
            "Resize floating left edge",
            "Resize floating top edge",
            "Resize floating bottom edge",
        ] {
            for (widget, _) in self.controls(Role::Splitter, label) {
                let orientation = self
                    .output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == widget.accesskit_id())
                    .and_then(|(_, node)| node.orientation());
                if let Some(orientation) = orientation {
                    controls.push((widget, orientation));
                }
            }
        }
        let Some((target, orientation)) = self.pick(&controls) else {
            return;
        };
        self.frame(vec![accesskit(target, Action::Focus)]);
        assert_eq!(
            self.context.memory(egui::Memory::focused),
            Some(target),
            "the registered resize control must retain its accepted accessibility focus",
        );
        let (increment, decrement) = match orientation {
            Orientation::Vertical => (egui::Key::ArrowRight, egui::Key::ArrowLeft),
            Orientation::Horizontal => (egui::Key::ArrowDown, egui::Key::ArrowUp),
        };
        let count = self.pick(&[2, 3, 4, 5]).unwrap();
        let mut events = Vec::with_capacity(count + 1);
        // Always include both producers; subsequent steps vary repetitions,
        // reversals and which producer starts the packet.
        for index in 0..count {
            let increase = self.pick(&[false, true]).unwrap();
            let event = if index % 2 == 0 {
                accesskit(
                    target,
                    if increase {
                        Action::Increment
                    } else {
                        Action::Decrement
                    },
                )
            } else {
                key(if increase { increment } else { decrement })
            };
            events.push(event);
        }
        if self.pick(&[false, true]).unwrap() {
            events.reverse();
        }
        self.note(format!(
            "mixed resize batch target={target:?}, events={events:?}"
        ));
        self.frame(events);
        assert!(self.context.input(|input| input.events.iter().all(|event| {
            !matches!(event, Event::Key { key, pressed: true, .. } if *key == increment || *key == decrement)
                && !matches!(event, Event::AccessKitActionRequest(request)
                    if request.target_node == target.accesskit_id()
                        && matches!(request.action, Action::Increment | Action::Decrement))
        })), "the actual focused control must consume every mixed adjustment");
        assert_eq!(self.context.memory(egui::Memory::focused), Some(target));
        assert!(!self.dock.has_active_gesture());
        self.adjustment_batches += 1;
        self.frame(Vec::new());
    }
}

fn accesskit(target: egui::Id, action: Action) -> Event {
    Event::AccessKitActionRequest(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target.accesskit_id(),
        data: None,
    })
}

fn key(key: egui::Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}
