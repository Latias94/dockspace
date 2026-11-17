//! Explicit accessible chrome focus supersedes an unapplied pane request.

use super::*;

#[test]
fn an_unapplied_pane_focus_request_yields_to_new_accesskit_focus() {
    for owned in [true, false] {
        let context = Context::default();
        context.enable_accesskit();
        context.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(
                RootId::new(1),
                DockspaceNode::equal_split(
                    crate::DockspaceAxis::Horizontal,
                    [DockspaceNode::tabs([EDITOR]), DockspaceNode::tabs([OTHER])],
                )
                .unwrap(),
            ),
        )])
        .unwrap();
        let mut dockspace = Dockspace::builder(("explicit-chrome-focus", owned), layout)
            .build()
            .unwrap();
        let mut pane = EditorPane {
            target: Id::unique(("explicit-chrome-focus-editor", owned)),
            text: String::new(),
            bounds: None,
            focus_trace: Vec::new(),
            focus_gains: 0,
        };
        let mut splitter = None;
        for pass in 0..4 {
            let (mut output, receipt) = run(
                owned,
                &context,
                &mut dockspace,
                &mut pane,
                0.4 + f64::from(pass) * 0.1,
                Vec::new(),
            )
            .into_parts();
            let splitter_node = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| {
                    node.role() == Role::Splitter && node.label() == Some("Resize panes")
                });
            splitter = splitter_node.and_then(|(node_id, node)| {
                assert!(node.supports_action(egui::accesskit::Action::Focus));
                assert!(!node.is_disabled());
                context.viewport(|viewport| {
                    viewport
                        .prev_pass
                        .widgets
                        .layers()
                        .flat_map(|(_, widgets)| widgets)
                        .find(|widget| {
                            widget.id.accesskit_id() == *node_id
                                && widget.enabled
                                && widget.visible
                                && widget.sense.is_focusable()
                        })
                        .map(|widget| widget.id)
                })
            });
            output.textures_delta.clear();
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
        }
        let splitter = splitter.expect("the presented separator owns its real framework widget");
        let selected = dockspace.select_item_current(EDITOR).unwrap();
        assert!(matches!(
            selected.status(),
            DockspaceActionStatus::Applied(DockspaceActionOutcome::Selected { tab: selected_tab, .. }) if selected_tab.item() == Some(EDITOR)));

        let mut focus_before_docking = None;
        let (mut output, receipt) = run_with_ui(
            owned,
            &context,
            &mut dockspace,
            &mut pane,
            1.0,
            vec![Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Focus,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: splitter.accesskit_id(),
                    data: None,
                },
            )],
            |ui| {
                focus_before_docking = ui.memory(egui::Memory::focused);
            },
        )
        .into_parts();
        assert_eq!(
            focus_before_docking,
            Some(splitter),
            "egui must accept the explicit focus before the adapter renders, owned={owned}",
        );
        assert_eq!(
            context.memory(egui::Memory::focused),
            Some(splitter),
            "the earlier unapplied pane request must not replace the newer accessible separator focus, owned={owned}, trace={:?}",
            pane.focus_trace,
        );
        output.textures_delta.clear();
        output.drop_without_applying_deltas();
        receipt.report(SurfacePresentationResult::Presented);
    }
}

#[test]
fn an_unaccepted_accesskit_focus_does_not_cancel_a_new_pane_focus_request() {
    for owned in [true, false] {
        for registered in [false, true] {
            let target = Id::unique("nonfocusable-accessibility-target");
            let register = |ui: &mut Ui| {
                if registered {
                    ui.interact(
                        Rect::from_min_size(Pos2::ZERO, egui::vec2(12.0, 12.0)),
                        target,
                        egui::Sense::hover(),
                    );
                }
            };
            let (context, mut dockspace, mut pane, _) = presented_editor_with_ui(owned, register);
            assert_eq!(context.memory(egui::Memory::focused), None);
            let selected = dockspace.select_item_current(EDITOR).unwrap();
            assert!(matches!(
                selected.status(),
                DockspaceActionStatus::Applied(DockspaceActionOutcome::Selected {
                    tab: selected_tab,
                    ..
                }) if selected_tab.item() == Some(EDITOR)));
            let mut focus_before_docking = None;
            let (mut output, receipt) = run_with_ui(
                owned,
                &context,
                &mut dockspace,
                &mut pane,
                1.0,
                vec![Event::AccessKitActionRequest(
                    egui::accesskit::ActionRequest {
                        action: egui::accesskit::Action::Focus,
                        target_tree: egui::accesskit::TreeId::ROOT,
                        target_node: target.accesskit_id(),
                        data: None,
                    },
                )],
                |ui| {
                    focus_before_docking = ui.memory(egui::Memory::focused);
                    register(ui);
                },
            )
            .into_parts();
            assert_eq!(
                focus_before_docking, None,
                "egui must reject an unavailable or nonfocusable target, owned={owned}, registered={registered}",
            );
            assert_eq!(
                context.memory(egui::Memory::focused),
                Some(pane.target),
                "an unaccepted focus event must not suppress the real pane request, owned={owned}, registered={registered}",
            );
            output.textures_delta.clear();
            output.drop_without_applying_deltas();
            receipt.report(SurfacePresentationResult::Presented);
        }
    }
}
