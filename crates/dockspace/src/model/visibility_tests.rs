use super::{
    DockspaceLayout, DockspaceLayoutError, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, RootId, SurfaceId,
};
use crate::ids::TabKey;

fn fixture() -> DockspaceLayout {
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SurfaceId::new(1),
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::tabs_with_selection((1..=3).map(ItemId::new), Some(ItemId::new(2)))
                .unwrap(),
        ),
    )])
    .unwrap()
}

#[test]
fn layout_visibility_is_queryable_and_survives_copying_without_exposing_runtime_nodes() {
    let layout = fixture()
        .with_hidden_items([ItemId::new(2), ItemId::new(3)])
        .unwrap();
    let view = layout.view();
    let hidden = view.item(ItemId::new(2)).unwrap();
    assert!(hidden.is_hidden());
    assert!(!hidden.is_visible());
    assert!(!hidden.is_selected());
    assert!(view.item(ItemId::new(1)).unwrap().is_selected());
    assert_eq!(
        hidden.tabs().visible_items().collect::<Vec<_>>(),
        [ItemId::new(1)]
    );
    assert_eq!(
        hidden.tabs().items().collect::<Vec<_>>(),
        [ItemId::new(1), ItemId::new(2), ItemId::new(3)]
    );
    assert_eq!(
        hidden.tabs().visible_selected(),
        Some(TabKey::for_item(ItemId::new(1)))
    );
    assert_eq!(
        view.hidden_items().collect::<Vec<_>>(),
        [ItemId::new(2), ItemId::new(3)]
    );
    let copy = view.to_layout();
    assert_eq!(
        copy.view().hidden_items().collect::<Vec<_>>(),
        view.hidden_items().collect::<Vec<_>>()
    );
    assert_eq!(
        copy.view().item(ItemId::new(2)).unwrap().tabs().selected(),
        Some(TabKey::for_item(ItemId::new(1)))
    );
}

#[test]
fn all_hidden_layout_keeps_logical_selection_and_replacing_the_hidden_set_shows_content() {
    let layout = fixture()
        .with_hidden_items((1..=3).map(ItemId::new))
        .unwrap();
    let item = layout.view().item(ItemId::new(2)).unwrap();
    assert_eq!(
        item.tabs().selected(),
        Some(TabKey::for_item(ItemId::new(2)))
    );
    assert_eq!(item.tabs().visible_selected(), None);
    assert!(item.tabs().visible_items().next().is_none());
    assert!(!item.is_selected());
    let layout = layout
        .with_hidden_items([ItemId::new(1), ItemId::new(2)])
        .unwrap();
    assert_eq!(
        layout
            .view()
            .item(ItemId::new(3))
            .unwrap()
            .tabs()
            .visible_selected(),
        Some(TabKey::for_item(ItemId::new(3)))
    );
    let layout = layout.with_hidden_items([]).unwrap();
    assert!(layout.view().hidden_items().next().is_none());
    assert_eq!(
        layout
            .view()
            .item(ItemId::new(3))
            .unwrap()
            .tabs()
            .visible_selected(),
        Some(TabKey::for_item(ItemId::new(3)))
    );
}

#[test]
fn initial_hidden_set_rejects_duplicate_or_unknown_item_identities() {
    assert_eq!(
        fixture()
            .with_hidden_items([ItemId::new(2), ItemId::new(2)])
            .unwrap_err(),
        DockspaceLayoutError::DuplicateHiddenItem {
            item: ItemId::new(2)
        }
    );
    assert_eq!(
        fixture().with_hidden_items([ItemId::new(99)]).unwrap_err(),
        DockspaceLayoutError::UnknownHiddenItem {
            item: ItemId::new(99)
        }
    );
}
