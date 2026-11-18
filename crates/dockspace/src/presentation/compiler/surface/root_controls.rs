//! Independent root controls reserve space before allocating application content.

use super::*;
use crate::graph::workspace::RootPresentationOwner;

pub(crate) struct RootControlsLayout {
    pub(crate) reserved: LogicalRect,
    pub(crate) content: LogicalRect,
    pub(crate) menu: Option<LogicalRect>,
}

/// Main roots without eligible Tabs use an independent band for layout actions.
/// Contained roots already provide the same actions in their title chrome.
pub(crate) fn root_controls_layout(
    workspace: &Workspace,
    root: RootId,
    bounds: LogicalRect,
    config: &DockPresentationConfig,
) -> Result<Option<RootControlsLayout>, SceneCompilationError> {
    if !matches!(
        workspace.presentation_for_root(root),
        Some(RootPresentationOwner::Main { .. })
    ) {
        return Ok(None);
    }
    let topology = workspace
        .root_structure(root)
        .ok_or(crate::presentation::layout::LayoutError::UnfinishedRoot { root })?;
    if topology.nodes.iter().any(|node| {
        matches!(workspace.node(node.id), Some(Node::Tabs { .. }))
            && workspace.node_has_visible_content(node.id)
    }) {
        return Ok(None);
    }
    if bounds.height() <= 0.0 || bounds.width() <= 0.0 {
        return Ok(None);
    }
    let content = bounds.inset_top(config.tab_bar_height())?;
    let bottom = content.y();
    let reserved =
        LogicalRect::from_min_max(bounds.min(), LogicalPoint::new(bounds.max().x(), bottom)?)?;
    let extent = (reserved.height() - 2.0).max(0.0).min(reserved.width());
    let menu = if extent > 0.0 && extent >= config.tab_close_extent() {
        Some(LogicalRect::new(
            reserved.max().x() - extent,
            reserved.y() + (reserved.height() - extent) * 0.5,
            extent,
            extent,
        )?)
    } else {
        None
    };
    Ok(Some(RootControlsLayout {
        reserved,
        content,
        menu,
    }))
}
