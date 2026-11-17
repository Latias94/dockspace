//! Stable pane identities and committed layout commands without a renderer.

use dockspace::model::{
    DockAnchor, DockPlacement, DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockspaceHostFrame, DockspaceSession, DockspaceSubmittedAction, HostFrameReport,
    HostInputOutcome,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const OUTLINE: ItemId = ItemId::new(1);
const EDITOR: ItemId = ItemId::new(2);
const PREVIEW: ItemId = ItemId::new(3);
const INSPECTOR: ItemId = ItemId::new(4);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let split = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([OUTLINE]), 0.25),
            (DockspaceNode::central_tabs([EDITOR, PREVIEW]), 0.75),
        ],
    )?;
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, split),
    )])?;
    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default())?;

    let mut frame = session.begin_host_frame()?;
    let selected = frame.select_item_current(PREVIEW)?;
    commit_action(frame, selected)?;
    assert_eq!(
        session
            .view()
            .item(EDITOR)
            .unwrap()
            .tabs()
            .expect("the example declares an explicit tabs group")
            .selected()
            .and_then(|tab| tab.item()),
        Some(PREVIEW)
    );

    let mut frame = session.begin_host_frame()?;
    let opened =
        frame.open_item_current(INSPECTOR, DockPlacement::Center(DockAnchor::Item(EDITOR)))?;
    commit_action(frame, opened)?;
    assert_eq!(
        session
            .view()
            .item(EDITOR)
            .unwrap()
            .tabs()
            .expect("the example declares an explicit tabs group")
            .items()
            .collect::<Vec<_>>(),
        [EDITOR, PREVIEW, INSPECTOR],
    );

    let mut frame = session.begin_host_frame()?;
    let moved =
        frame.dock_item_current(INSPECTOR, DockPlacement::Center(DockAnchor::Item(OUTLINE)))?;
    commit_action(frame, moved)?;
    assert_eq!(
        session
            .view()
            .item(OUTLINE)
            .unwrap()
            .tabs()
            .expect("the example declares an explicit tabs group")
            .items()
            .collect::<Vec<_>>(),
        [OUTLINE, INSPECTOR],
    );

    // Dropping an uncommitted frame publishes none of its proposed actions.
    let before = session.version();
    let mut abandoned = session.begin_host_frame()?;
    abandoned.select_item_current(OUTLINE)?;
    drop(abandoned);
    assert_eq!(session.version(), before);
    assert_eq!(
        session
            .view()
            .item(OUTLINE)
            .unwrap()
            .tabs()
            .expect("the example declares an explicit tabs group")
            .selected()
            .and_then(|tab| tab.item()),
        Some(INSPECTOR)
    );

    println!("Inspector moved into Outline's group; an abandoned edit left it selected.");

    let mut frame = session.begin_host_frame()?;
    let hidden = frame.set_item_visibility_current(INSPECTOR, false)?;
    commit_action(frame, hidden)?;
    assert!(session.view().item(INSPECTOR).unwrap().is_hidden());
    assert_eq!(
        session
            .view()
            .item(OUTLINE)
            .unwrap()
            .tabs()
            .expect("the example declares an explicit tabs group")
            .visible_items()
            .collect::<Vec<_>>(),
        [OUTLINE],
    );
    let mut frame = session.begin_host_frame()?;
    let shown = frame.set_item_visibility_current(INSPECTOR, true)?;
    commit_action(frame, shown)?;
    assert_eq!(
        session
            .view()
            .item(OUTLINE)
            .unwrap()
            .tabs()
            .expect("the example declares an explicit tabs group")
            .items()
            .collect::<Vec<_>>(),
        [OUTLINE, INSPECTOR],
    );
    assert!(session.view().item(OUTLINE).unwrap().is_active());
    println!("Inspector was hidden and restored in place without stealing selection.");
    Ok(())
}

fn commit_action(
    frame: DockspaceHostFrame<'_>,
    action: DockspaceSubmittedAction,
) -> Result<HostFrameReport, Box<dyn std::error::Error>> {
    // This example changes layout only. A renderer host instead measures,
    // executes the surface program and reports its exact output result.
    let report = frame.commit()?;
    match report.submitted_action_outcome(action) {
        Some(HostInputOutcome::ProductActionApplied(outcome)) => println!("Applied: {outcome:?}"),
        outcome => return Err(format!("The layout command was not applied: {outcome:?}").into()),
    }
    Ok(report)
}
