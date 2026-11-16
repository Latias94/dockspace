use super::*;

#[test]
fn accesskit_tab_focus_preserves_header_focus_until_explicit_activation() {
    let context = Context::default();
    context.enable_accesskit();
    let mut dockspace = Dockspace::builder("product-accesskit-tab-focus", layout())
        .build()
        .unwrap();
    let mut panes = FocusPanes::new("product-accesskit-tab-focus", true);
    let _ = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (stable, _) = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    let (second, _) = accesskit_node(&stable.output, Role::Tab, "Second");
    let _ = run_focus_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(second, Action::Focus)],
    );
    for _ in 0..4 {
        let _ = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    }
    assert_eq!(selected(&dockspace), Some(SECOND));
    assert_eq!(
        context
            .memory(egui::Memory::focused)
            .map(|id| id.accesskit_id()),
        Some(second),
        "focusing a tab must keep focus in the header for subsequent navigation"
    );
    assert!(!context.memory(|memory| memory.has_focus(panes.target)));

    let _ = run_focus_frame(
        &context,
        &mut dockspace,
        &mut panes,
        key_press(Key::ArrowLeft),
    );
    for _ in 0..3 {
        let _ = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    }
    assert_eq!(selected(&dockspace), Some(FIRST));

    let _ = run_focus_frame(
        &context,
        &mut dockspace,
        &mut panes,
        vec![accesskit_action(second, Action::Focus)],
    );
    for _ in 0..3 {
        let _ = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    }
    let _ = run_focus_frame(&context, &mut dockspace, &mut panes, key_press(Key::Enter));
    for _ in 0..4 {
        let _ = run_focus_frame(&context, &mut dockspace, &mut panes, Vec::new());
    }
    assert!(
        context.memory(|memory| memory.has_focus(panes.target)),
        "explicit activation must still enter the selected pane content"
    );
}
