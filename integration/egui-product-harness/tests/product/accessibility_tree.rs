//! UI semantics for separately embedded Dockspaces with the same layout IDs.

use super::*;
use egui_kittest::kittest::NodeT as _;

#[test]
fn embedded_dockspaces_keep_tabs_and_selected_content_under_their_own_host_panes() {
    let editors = ["Left editor", "Right editor"]
        .map(|label| Dockspace::builder(label, layout()).build().unwrap());
    let mut harness = Harness::builder()
        .with_size(vec2(800.0, 600.0))
        .build_ui_state(
            |ui, editors: &mut [Dockspace; 2]| {
                ui.columns(2, |columns| {
                    for ((column, editor), label) in columns
                        .iter_mut()
                        .zip(editors)
                        .zip(["Left editor", "Right editor"])
                    {
                        column.scope_builder(
                            egui::UiBuilder::new()
                                .accessibility_label(label)
                                .accessibility_role(Role::Pane),
                            |ui| {
                                editor.show_single_surface(SURFACE, ui, &mut Panes).unwrap();
                            },
                        );
                    }
                });
            },
            editors,
        );
    harness.run_steps(2);

    let left = harness.get_by_role_and_label(Role::Pane, "Left editor");
    let right = harness.get_by_role_and_label(Role::Pane, "Right editor");
    let left_tab = left.get_by_role_and_label(Role::Tab, "Second");
    let right_tab = right.get_by_role_and_label(Role::Tab, "Second");
    let tab_ids = [
        left_tab.accesskit_node().id(),
        right_tab.accesskit_node().id(),
    ];
    assert_ne!(tab_ids[0], tab_ids[1]);
    for editor in [left, right] {
        editor.get_by_label("Pane 1");
        assert!(editor.query_by_label("Pane 2").is_none());
    }

    left_tab.click_accesskit();
    harness.run_steps(2);
    assert_eq!(
        harness.state().each_ref().map(selected),
        [Some(SECOND), Some(FIRST)]
    );

    let left = harness.get_by_role_and_label(Role::Pane, "Left editor");
    let right = harness.get_by_role_and_label(Role::Pane, "Right editor");
    left.get_by_label("Pane 2");
    right.get_by_label("Pane 1");
    assert!(left.query_by_label("Pane 1").is_none());
    assert!(right.query_by_label("Pane 2").is_none());
    for (editor, expected_tab) in [left, right].into_iter().zip(tab_ids) {
        let tab = editor.get_by_role_and_label(Role::Tab, "Second");
        assert_eq!(tab.accesskit_node().id(), expected_tab);
    }
}
