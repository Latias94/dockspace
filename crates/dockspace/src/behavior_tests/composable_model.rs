//! A labelled membership oracle, independent of arena topology and geometry.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

use crate::graph::Workspace;
use crate::graph::command::{DockTarget, MovePayload, WorkspaceCommand};
use crate::graph::transaction::WorkspaceTransaction;
use crate::ids::{ItemId, NodeId, RootId, SurfaceId, TabKey};
use crate::model::{
    DockspaceAxis, DockspaceGridShape, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, DockspaceView,
};
use crate::policy::DockPolicySnapshot;

const ITEMS: [ItemId; 7] = [
    ItemId::new(1),
    ItemId::new(2),
    ItemId::new(3),
    ItemId::new(4),
    ItemId::new(5),
    ItemId::new(6),
    ItemId::new(7),
];
const ROOTS: [RootId; 2] = [RootId::new(1), RootId::new(2)];

#[derive(Clone, Copy, Debug)]
enum Action {
    Activate(ItemId),
    TransferTools,
    ReorderTools(usize),
    SetBareVisibility(bool),
}

fn actions() -> impl Strategy<Value = Action> {
    prop_oneof![
        prop::sample::select(ITEMS.to_vec()).prop_map(Action::Activate),
        Just(Action::TransferTools),
        (0..=2_usize).prop_map(Action::ReorderTools),
        any::<bool>().prop_map(Action::SetBareVisibility),
    ]
}

struct Labels {
    owner: usize,
    // Each root has one permanent document and optionally the Tools tab.
    rosters: [Vec<bool>; 2],
    active_tools: [bool; 2],
    left: ItemId,
    right: ItemId,
    bare_visible: bool,
}

impl Labels {
    fn initial() -> Self {
        Self {
            owner: 0,
            rosters: [vec![false, true], vec![false]],
            active_tools: [false, false],
            left: ITEMS[0],
            right: ITEMS[4],
            bare_visible: true,
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Activate(item) if item == ITEMS[2] => self.active_tools[0] = false,
            Action::Activate(item) if item == ITEMS[6] => self.active_tools[1] = false,
            Action::Activate(item) => {
                self.active_tools[self.owner] = true;
                if [ITEMS[0], ITEMS[1]].contains(&item) {
                    self.left = item;
                } else if [ITEMS[4], ITEMS[5]].contains(&item) {
                    self.right = item;
                }
            }
            Action::TransferTools => {
                self.rosters[self.owner].retain(|tools| !tools);
                self.active_tools[self.owner] = false;
                self.owner = 1 - self.owner;
                self.rosters[self.owner].push(true);
                self.active_tools[self.owner] = true;
            }
            Action::ReorderTools(gap) => {
                let roster = &mut self.rosters[self.owner];
                let previous = roster.iter().position(|tools| *tools).unwrap();
                roster.remove(previous);
                roster.insert(gap - usize::from(gap > previous), true);
            }
            Action::SetBareVisibility(visible) => self.bare_visible = visible,
        }
    }

    fn assert_workspace(&self, workspace: &Workspace, tools: NodeId) {
        let view = DockspaceView::new(workspace);
        assert_eq!(
            workspace.item_multiset(),
            BTreeMap::from(ITEMS.map(|item| (item, 1)))
        );
        for (index, root) in ROOTS.into_iter().enumerate() {
            let document = if index == 0 { ITEMS[2] } else { ITEMS[6] };
            let expected_owned = self.rosters[index]
                .iter()
                .flat_map(|tools| {
                    if *tools {
                        vec![ITEMS[0], ITEMS[1], ITEMS[3], ITEMS[4], ITEMS[5]]
                    } else {
                        vec![document]
                    }
                })
                .collect::<Vec<_>>();
            let expected_active = if self.active_tools[index] {
                [
                    Some(self.left),
                    self.bare_visible.then_some(ITEMS[3]),
                    Some(self.right),
                ]
                .into_iter()
                .flatten()
                .collect()
            } else {
                vec![document]
            };
            let actual = view.root(root).unwrap();
            assert_eq!(actual.items().collect::<Vec<_>>(), expected_owned);
            assert_eq!(actual.active_items().collect::<Vec<_>>(), expected_active);
            let tabs = actual.content().unwrap().tabs().unwrap();
            let expected_tabs = self.rosters[index]
                .iter()
                .map(|is_tools| {
                    if *is_tools {
                        TabKey::for_layout(tools)
                    } else {
                        TabKey::for_item(document)
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(tabs.tabs(), expected_tabs);
            for item in expected_owned {
                let actual = view.item(item).unwrap();
                assert_eq!(actual.root(), root);
                assert_eq!(actual.is_active(), expected_active.contains(&item));
            }
        }
        assert_eq!(workspace.node(tools).unwrap().layout_title(), Some("Tools"));
        assert!(view.item(ITEMS[3]).unwrap().tabs().is_none());
        assert_eq!(view.item(ITEMS[3]).unwrap().is_visible(), self.bare_visible);
        workspace.validate().unwrap();
    }
}

fn fixture() -> (Workspace, NodeId) {
    let [a, b, c, d, e, f, g] = ITEMS;
    let grid = DockspaceNode::grid(
        DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(1).unwrap(),
            columns: NonZeroUsize::new(2).unwrap(),
        },
        [DockspaceNode::pane(d), DockspaceNode::tabs([e, f])],
    )
    .unwrap();
    let tools = DockspaceNode::equal_split(
        DockspaceAxis::Horizontal,
        [DockspaceNode::tabs([a, b]), grid],
    )
    .unwrap()
    .named("Tools");
    let workspace = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SurfaceId::new(1),
            DockspaceRootLayout::new(
                ROOTS[0],
                DockspaceNode::composed_tabs([DockspaceNode::pane(c), tools]),
            ),
        ),
        DockspaceSurfaceLayout::new(
            SurfaceId::new(2),
            DockspaceRootLayout::new(ROOTS[1], DockspaceNode::tabs([g])),
        ),
    ])
    .unwrap()
    .into_workspace();
    let tools = workspace
        .nodes()
        .find_map(|(id, node)| (node.layout_title() == Some("Tools")).then_some(id))
        .unwrap();
    (workspace, tools)
}

fn command(workspace: &Workspace, tools: NodeId, owner: usize, action: Action) -> WorkspaceCommand {
    let root = ROOTS[owner];
    match action {
        Action::Activate(item) => WorkspaceCommand::SelectItem {
            source: workspace.capture_item_source_by_id(item).unwrap().unwrap(),
        },
        Action::TransferTools => {
            let destination = ROOTS[1 - owner];
            WorkspaceCommand::Move {
                payload: MovePayload::Subtree(workspace.capture_node_source(root, tools).unwrap()),
                target: DockTarget::Center(
                    workspace
                        .capture_tab_target(destination, workspace.root(destination).unwrap().node)
                        .unwrap(),
                ),
            }
        }
        Action::ReorderTools(insertion_index) => WorkspaceCommand::Reorder {
            source: workspace
                .capture_tab_source(
                    root,
                    workspace.root(root).unwrap().node,
                    TabKey::for_layout(tools),
                )
                .unwrap(),
            insertion_index,
        },
        Action::SetBareVisibility(visible) => WorkspaceCommand::SetItemVisibility {
            source: workspace
                .capture_item_source_by_id(ITEMS[3])
                .unwrap()
                .unwrap(),
            visible,
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct("proptest-regressions/composable_model.txt"))),
        ..ProptestConfig::with_cases(128)
    })]

    #[test]
    fn composable_sequences_preserve_labelled_membership_and_activation(
        actions in prop::collection::vec(actions(), 1..65),
    ) {
        let (mut workspace, tools) = fixture();
        let mut expected = Labels::initial();
        expected.assert_workspace(&workspace, tools);
        for action in actions {
            let command = command(&workspace, tools, expected.owner, action);
            let before = workspace.clone();
            let result = WorkspaceTransaction::from_commands([command])
                .apply(&mut workspace, &DockPolicySnapshot::default());
            if matches!(action, Action::Activate(item) if item == ITEMS[3] && !expected.bare_visible) {
                prop_assert!(result.is_err(), "hidden content cannot be activated");
                prop_assert_eq!(&workspace, &before, "rejected activation must be atomic");
            } else {
                result.unwrap_or_else(|error| panic!("model-valid {action:?}: {error:?}"));
                expected.apply(action);
            }
            expected.assert_workspace(&workspace, tools);
        }
    }
}
