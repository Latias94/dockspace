//! Grid scenes retain real cells and shared boundary proof under selected Tabs.

use super::*;
use crate::graph::SplitWeight;
use crate::presentation::scene::{PaneSceneId, SplitterOwner};
use std::num::NonZeroUsize;

const ROOT: RootId = RootId::new(8101);
const SURFACE: SurfaceId = SurfaceId::new(8102);

fn fixture(nested: bool, hidden: &[ItemId]) -> (Workspace, NodeId, Option<NodeId>) {
    let mut builder = Workspace::builder();
    let children = (1..=4)
        .map(|index| builder.insert_node(Node::pane(ItemId::new(index))))
        .collect();
    let grid = builder.insert_node(Node::Grid {
        shape: crate::model::DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(2).unwrap(),
            columns: NonZeroUsize::new(2).unwrap(),
        },
        children,
        shares: None,
        layout_title: Some(Arc::from("Palette")),
    });
    let outer = nested.then(|| {
        builder.insert_node(Node::tabs_from_keys(
            vec![TabKey::for_item(ItemId::new(10)), TabKey::for_layout(grid)],
            Some(TabKey::for_layout(grid)),
        ))
    });
    builder.set_root(ROOT, RootRecord::new(outer.unwrap_or(grid)));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_hidden_items(hidden.iter().copied().collect());
    (builder.build().unwrap(), grid, outer)
}

fn plan(workspace: &Workspace) -> PresentationPlan {
    let config = DockPresentationConfig::builder()
        .tab_bar_height(20.0)
        .tab_group_grip_extent(20.0)
        .splitter_thickness(4.0)
        .minimum_pane_size(1.0, 1.0)
        .build()
        .unwrap();
    let policy = policy_snapshot();
    let plan = compile_ready_plan_with_config(
        workspace,
        SURFACE,
        LogicalRect::new(0.0, 0.0, 404.0, 224.0).unwrap(),
        true,
        &policy,
        &config,
    )
    .unwrap();
    PresentationPlanValidator::new(workspace, &policy)
        .unwrap()
        .validate_and_canonicalize(plan)
        .unwrap()
}

#[test]
fn fixed_grid_compiles_real_content_shared_boundaries_and_one_junction() {
    let (workspace, grid, _) = fixture(false, &[]);
    let plan = plan(&workspace);
    assert!(plan.tab_records().is_empty());
    assert!(plan.tabs_records().is_empty());
    assert_eq!(plan.pane_records().len(), 4);
    assert_eq!(plan.splitter_records().len(), 2);
    assert_eq!(plan.splitter_junction_records().len(), 1);
    for pane in plan.pane_records() {
        assert_eq!(pane.bounds().width(), 200.0);
        assert_eq!(pane.bounds().height(), 100.0);
        assert!(pane.containing_tab_bar().is_none());
        let center = DropTargetId::Center {
            surface: SURFACE,
            root: ROOT,
            tabs: pane.owner_node(),
        };
        assert!(plan.drop_target(center).is_some());
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            assert!(
                plan.drop_target(DropTargetId::InnerEdge {
                    surface: SURFACE,
                    root: ROOT,
                    node: pane.owner_node(),
                    edge,
                })
                .is_some()
            );
        }
    }
    for record in plan.splitter_records() {
        assert_eq!(record.id().node(), grid);
        assert!(matches!(record.id().owner,SplitterOwner::Grid{node,..} if node==grid));
        let proof = record.grid().unwrap();
        assert_eq!((proof.rows(), proof.columns()), (2, 2));
        assert_eq!(proof.children().len(), 4);
        assert_eq!(record.child_extents().len(), 2);
        assert_eq!(
            record.weights(),
            SplitWeight::normalize([1.0, 1.0]).unwrap()
        );
        assert!(record.operable());
    }
}

#[test]
fn hidden_cells_keep_grid_slots_without_pane_callbacks_or_fake_tabs() {
    let (workspace, grid, _) = fixture(false, &[ItemId::new(1)]);
    let plan = plan(&workspace);
    assert_eq!(plan.pane_records().len(), 3);
    assert!(
        !plan
            .pane_records()
            .iter()
            .any(|pane| pane.item() == ItemId::new(1))
    );
    let second = plan
        .pane_records()
        .iter()
        .find(|pane| pane.item() == ItemId::new(2))
        .unwrap();
    assert_eq!(
        second.bounds(),
        LogicalRect::new(204.0, 20.0, 200.0, 100.0).unwrap()
    );
    assert_eq!(
        plan.layout_facts()
            .unwrap()
            .root(ROOT)
            .unwrap()
            .projection()
            .grids
            .get(&grid)
            .unwrap()
            .children
            .len(),
        4
    );
    assert!(plan.tab_records().is_empty());
}

#[test]
fn switching_away_from_a_grid_revokes_its_body_header_and_shared_junction_paths() {
    let (mut workspace, grid, outer) = fixture(true, &[]);
    let outer = outer.unwrap();
    let old = plan(&workspace);
    assert_eq!(old.tab_bar_records().len(), 1);
    assert_eq!(old.pane_records().len(), 4);
    let body = PresentationHitRegionKind::PaneBody(PaneSceneId {
        root: ROOT,
        item: ItemId::new(1),
    });
    let boundary = PresentationHitRegionKind::SplitterHandle(*old.splitter_records()[0].id());
    let junction =
        PresentationHitRegionKind::SplitterJunction(old.splitter_junction_records()[0].id());
    let pane = old
        .pane_records()
        .iter()
        .find(|pane| pane.item() == ItemId::new(1))
        .unwrap();
    let center = PresentationHitRegionKind::DropTarget(DropTargetId::Center {
        surface: SURFACE,
        root: ROOT,
        tabs: pane.owner_node(),
    });
    let edge = PresentationHitRegionKind::DropTarget(DropTargetId::InnerEdge {
        surface: SURFACE,
        root: ROOT,
        node: pane.owner_node(),
        edge: Edge::Left,
    });
    let guide = old
        .drop_guide_clusters()
        .iter()
        .find(|cluster| cluster.id() == DropGuideClusterId::inner(SURFACE, ROOT, pane.owner_node()))
        .map(|cluster| PresentationHitRegionKind::DropGuideActivation(cluster.id()))
        .expect("the visible Grid cell has a local docking guide");
    let receivers = [body, boundary, junction, center, edge, guide];
    for receiver in receivers {
        assert!(old.receiver_active_path_is_current(receiver, &workspace));
    }
    let source = workspace
        .capture_tab_source(ROOT, outer, TabKey::for_item(ItemId::new(10)))
        .unwrap();
    crate::graph::transaction::WorkspaceTransaction::from_commands([
        crate::graph::command::WorkspaceCommand::Select { source },
    ])
    .apply(&mut workspace, &policy_snapshot())
    .unwrap();
    for receiver in receivers {
        assert!(!old.receiver_active_path_is_current(receiver, &workspace));
    }
    let current = plan(&workspace);
    assert_eq!(current.pane_records().len(), 1);
    assert!(current.splitter_records().is_empty());
    assert!(current.splitter_junction_records().is_empty());
    assert_eq!(
        old.node_label(ROOT, grid),
        Some(crate::graph::TabLabel::Layout { title: "Palette" })
    );
    assert_eq!(
        current
            .layout_facts()
            .unwrap()
            .root(ROOT)
            .unwrap()
            .pane_sizings()
            .count(),
        5
    );
}
