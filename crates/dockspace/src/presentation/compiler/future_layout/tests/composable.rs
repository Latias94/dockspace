//! Bare destinations gain real Tabs ownership only in the checked candidate.

use super::*;
use crate::model::{
    DockspaceAxis, DockspaceGridShape, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout,
};
use std::num::NonZeroUsize;

#[test]
fn center_drop_into_bare_content_projects_the_new_tabs_without_old_strip_measurements() {
    let retained = ItemId::new(10);
    let remainder = ItemId::new(2);
    for grid in [
        None,
        Some(DockspaceGridShape::Auto),
        Some(DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(2).unwrap(),
            columns: NonZeroUsize::new(2).unwrap(),
        }),
    ] {
        let cells = [
            DockspaceNode::pane(retained),
            DockspaceNode::tabs([MOVED, remainder]),
            DockspaceNode::pane(ItemId::new(11)),
            DockspaceNode::pane(ItemId::new(12)),
        ];
        let content = match grid {
            Some(shape) => DockspaceNode::grid(shape, cells).unwrap(),
            None => DockspaceNode::equal_split(DockspaceAxis::Horizontal, cells).unwrap(),
        };
        let mut workspace = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap()
        .into_workspace();
        let source = workspace.capture_item_source_by_id(MOVED).unwrap().unwrap();
        let target = workspace
            .capture_item_source_by_id(retained)
            .unwrap()
            .unwrap()
            .owner_node();
        let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
        let config = DockPresentationConfig::default();
        let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
        let mut states = TabStripStateStore::default();
        let presented = compile(&workspace, &policy, &config, strip, &mut states);
        assert!(matches!(workspace.node(target), Some(Node::Pane { .. })));
        assert!(
            !presented
                .tab_bar_records()
                .iter()
                .any(|bar| bar.id().tabs == target)
        );

        let resolved = resolve_target(
            &workspace,
            &policy,
            &presented,
            &presented,
            MovePayload::Item(source),
            DropTargetId::Center {
                surface: SURFACE,
                root: ROOT,
                tabs: target,
            },
        );
        assert!(
            resolved.preview_tab_strip().is_none(),
            "an unmeasured new strip must not invent host chrome"
        );
        WorkspaceTransaction::from_commands([resolved.command().clone()])
            .apply(&mut workspace, &policy)
            .unwrap();
        workspace.validate().unwrap();
        assert_eq!(
            workspace.visible_tab_keys(target).as_ref(),
            [retained, MOVED].map(TabKey::for_item)
        );
        assert_eq!(workspace.visible_tab_selection(target), Some(MOVED.into()));
        assert!(workspace.item_is_active(MOVED));
        assert!(!workspace.item_is_active(retained));
        assert!(workspace.item_is_active(remainder));
        let committed = compile(&workspace, &policy, &config, strip, &mut states);
        let pane = committed
            .pane_records()
            .iter()
            .find(|pane| pane.item() == MOVED)
            .unwrap();
        assert_eq!(pane.owner_node(), target);
        assert_eq!(resolved.visual().rect(), pane.content_bounds());
    }
}
