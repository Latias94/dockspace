//! Small generated moves checked against graph ownership and fresh measured output.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::Arc;

use super::label_rebinding::{check_boundary_label_rebinding, compile_label_measured};
use super::*;
use crate::graph::TabLabel;
use crate::graph::command::{CommandOutcome, DockFraction, DockTarget, Edge, WorkspaceCommand};
use crate::model::DockspaceGridShape;
use proptest::prelude::*;

const SECOND: ItemId = ItemId::new(2);
const HIDDEN: ItemId = ItemId::new(3);
const FALLBACK: ItemId = ItemId::new(4);

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_composable_preview_matches_commit_and_analytic_partitions(
        action in 0_u8..11,
        depth in 0_u8..3,
        container in 0_u8..3,
        target_container in 0_u8..3,
        bare_target in any::<bool>(),
        horizontal in any::<bool>(),
        reverse in any::<bool>(),
        named in any::<bool>(),
        hidden in any::<bool>(),
        inactive in any::<bool>(),
        empty_central in any::<bool>(),
    ) {
        if action == 0 {
            // The same canonical rewrite changes anonymous fallback text but
            // preserves a declared title. Both exact and source-facts fallback
            // must honor the frozen descriptor, independently of stable IDs.
            check_boundary_label_rebinding(named.then_some("Tools"));
        } else {
            check_move(action, depth, container, target_container, bare_target, horizontal, reverse, named, hidden, inactive, empty_central);
        }
    }
}

#[test]
fn named_group_edge_control_keeps_two_headers_and_the_exact_half_allocation() {
    let (candidate, plan, resolved) =
        check_move(6, 0, 0, 0, false, true, false, true, false, false, false);
    let body = plan
        .pane_records()
        .iter()
        .find(|pane| pane.item() == MOVED)
        .unwrap();
    assert_eq!(resolved.visual().rect(), rect(566.25, 153.0, 433.75, 447.0));
    assert_eq!(body.bounds(), rect(566.25, 209.0, 433.75, 391.0));
    assert!(candidate.item_is_active(MOVED));
    assert!(!candidate.item_is_active(SECOND));
    assert!(!candidate.item_is_active(FALLBACK));
}

#[test]
fn empty_central_sibling_is_excluded_from_the_payload_preview_scope() {
    let (_, _, resolved) = check_move(6, 0, 0, 0, false, true, false, false, false, false, true);
    assert_eq!(resolved.visual().rect(), rect(566.25, 153.0, 433.75, 447.0));
}

#[test]
fn nested_auto_grid_edge_drop_validates_its_own_clipped_tracks() {
    check_move(7, 1, 2, 1, false, true, false, false, false, false, false);
}

#[test]
fn bare_grid_cell_center_drop_matches_declared_two_by_two_tracks() {
    let (candidate, plan, resolved) =
        check_move(2, 0, 0, 1, true, true, false, true, false, false, false);
    let owner = candidate.locate_item_by_id(MOVED).unwrap().owner_node();
    let projection = plan
        .layout_facts()
        .unwrap()
        .root(ROOT)
        .unwrap()
        .projection();
    let grid = projection
        .grids
        .iter()
        .find_map(|(_, grid)| grid.children.contains(&owner).then_some(grid))
        .unwrap();
    let parent = rect(566.25, 153.0, 433.75, 447.0);
    assert_eq!(projection.nodes[&grid.node].rect, parent);
    assert_eq!((grid.shape.rows, grid.shape.columns), (2, 2));
    assert_eq!(grid.children.len(), 4);
    // Declared equal 2x2 tracks consume one four-unit gap per axis.
    // These constants do not read the solver's projected track extents.
    let width = (parent.width() - 4.0) / 2.0;
    let height = (parent.height() - 4.0) / 2.0;
    assert_eq!((width, height), (214.875, 221.5));
    for (index, child) in grid.children.iter().enumerate() {
        let x = parent.x() + (index % 2) as f64 * (width + 4.0);
        let y = parent.y() + (index / 2) as f64 * (height + 4.0);
        assert_eq!(projection.nodes[child].slot, rect(x, y, width, height));
    }
    for index in 0..2 {
        assert_eq!(grid.columns.tracks[index].extent, width);
        assert_eq!(
            grid.columns.tracks[index].bounds,
            rect(
                parent.x() + index as f64 * (width + 4.0),
                parent.y(),
                width,
                parent.height(),
            )
        );
        assert_eq!(grid.rows.tracks[index].extent, height);
        assert_eq!(
            grid.rows.tracks[index].bounds,
            rect(
                parent.x(),
                parent.y() + index as f64 * (height + 4.0),
                parent.width(),
                height,
            )
        );
    }
    assert_eq!(grid.children[0], owner);
    assert_eq!(
        resolved.visual().rect(),
        rect(566.25, 181.0, 214.875, 193.5)
    );
    assert!(resolved.preview_tab_strip().is_none());
}

#[test]
fn bare_grid_cell_inner_edge_drop_keeps_the_declared_cell_partition() {
    let (_, plan, resolved) = check_move(5, 0, 0, 1, true, true, false, false, false, false, false);
    // The 214.875-wide cell reserves four units, then gives each branch half.
    assert_eq!(
        resolved.visual().rect(),
        rect(675.6875, 181.0, 105.4375, 193.5)
    );
    let body = plan
        .pane_records()
        .iter()
        .find(|pane| pane.item() == MOVED)
        .unwrap();
    assert_eq!(body.bounds(), resolved.visual().rect());
    assert!(resolved.preview_tab_strip().is_none());
}

#[test]
fn complete_groups_and_subtrees_merge_into_bare_fixed_and_auto_grid_cells() {
    for target_container in [1, 2] {
        for action in [3, 10] {
            check_move(
                action,
                1,
                0,
                target_container,
                true,
                true,
                false,
                true,
                true,
                true,
                false,
            );
        }
    }
}

#[test]
fn every_payload_scope_can_split_a_bare_grid_cell_at_each_inner_edge() {
    for action in [5, 6, 7] {
        for horizontal in [true, false] {
            for reverse in [true, false] {
                check_move(
                    action, 1, 0, 1, true, horizontal, reverse, true, true, true, false,
                );
            }
        }
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
    LogicalRect::new(x, y, width, height).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn check_move(
    action: u8,
    depth: u8,
    container: u8,
    target_container: u8,
    bare_target: bool,
    horizontal: bool,
    reverse: bool,
    named: bool,
    hidden: bool,
    inactive: bool,
    empty_central: bool,
) -> (Workspace, PresentationPlan, ResolvedDrop) {
    let mut builder = Workspace::builder();
    let axis = if horizontal {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    let mut hidden_items = BTreeSet::new();
    if hidden {
        hidden_items.insert(HIDDEN);
    }
    let leaf = builder.insert_node(Node::tabs_with_selection(
        [MOVED, SECOND, HIDDEN],
        Some(if inactive { SECOND } else { MOVED }),
    ));
    let mut tree = leaf;
    for level in 0..depth {
        let sibling = builder.insert_node(Node::pane(ItemId::new(100 + u64::from(level) * 2)));
        let mut node = if container == 0 {
            let along = if level % 2 == 0 {
                axis
            } else if horizontal {
                Axis::Vertical
            } else {
                Axis::Horizontal
            };
            Node::equal_split(along, [tree, sibling]).unwrap()
        } else {
            let item = ItemId::new(101 + u64::from(level) * 2);
            let tail = builder.insert_node(Node::pane(item));
            if hidden {
                hidden_items.insert(item);
            }
            Node::Grid {
                shape: if container == 1 {
                    DockspaceGridShape::Fixed {
                        rows: NonZeroUsize::new(2).unwrap(),
                        columns: NonZeroUsize::new(2).unwrap(),
                    }
                } else {
                    DockspaceGridShape::Auto
                },
                children: vec![tree, sibling, tail],
                shares: None,
                layout_title: None,
            }
        };
        if named {
            node.set_layout_title(Some(Arc::from("Tools")));
        }
        tree = builder.insert_node(node);
    }
    let layout = TabKey::for_layout(tree);
    let mut group = Node::tabs_from_keys(
        vec![layout, FALLBACK.into()],
        Some(if inactive { FALLBACK.into() } else { layout }),
    );
    if named {
        group.set_layout_title(Some(Arc::from("Tools")));
    }
    let group = builder.insert_node(group);
    // Every center/edge payload can target bare content; gaps require Tabs.
    let bare = action == 2 || (bare_target && action != 4);
    // A pointer gap needs a presented roster; empty central targets expose
    // center docking instead and are covered by the other generated actions.
    let target = builder.insert_node(if bare {
        Node::pane(ItemId::new(10))
    } else if empty_central && action != 4 {
        Node::tabs([])
    } else {
        Node::tabs([ItemId::new(10), ItemId::new(11)])
    });
    let destination_branch = if target_container == 0 {
        target
    } else {
        let neighbors = [20, 21, 22].map(|item| builder.insert_node(Node::pane(ItemId::new(item))));
        if hidden {
            hidden_items.insert(ItemId::new(22));
        }
        let children = if reverse {
            vec![neighbors[0], neighbors[1], neighbors[2], target]
        } else {
            vec![target, neighbors[0], neighbors[1], neighbors[2]]
        };
        builder.insert_node(Node::Grid {
            shape: if target_container == 1 {
                DockspaceGridShape::Fixed {
                    rows: NonZeroUsize::new(2).unwrap(),
                    columns: NonZeroUsize::new(2).unwrap(),
                }
            } else {
                DockspaceGridShape::Auto
            },
            children,
            shares: None,
            layout_title: named.then(|| Arc::from("Tools")),
        })
    };
    let branches = if reverse {
        [destination_branch, group]
    } else {
        [group, destination_branch]
    };
    let lane = builder.insert_node(Node::equal_split(axis, branches).unwrap());
    // Fixed dyadic margins keep direct gaps outside the root's magnetic band.
    let top = builder.insert_node(Node::tabs([ItemId::new(900)]));
    let body = builder.insert_node(Node::split(Axis::Vertical, [top, lane], [1.0, 3.0]).unwrap());
    let left = builder.insert_node(Node::tabs([ItemId::new(901)]));
    let root =
        builder.insert_node(Node::split(Axis::Horizontal, [left, body], [1.0, 7.0]).unwrap());
    let record = if bare {
        RootRecord::new(root)
    } else {
        RootRecord::new(root).with_central(target)
    };
    builder.set_root(ROOT, record);
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_hidden_items(hidden_items);
    let workspace = builder.build().unwrap();
    if target_container != 0 {
        assert_eq!(
            workspace.parent_link(ROOT, target).unwrap().parent,
            destination_branch,
            "the generated destination must be an actual Grid cell",
        );
    }
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::builder()
        .splitter_thickness(4.0)
        .minimum_pane_size(8.0, 8.0)
        .build()
        .unwrap();
    let presented = compile_label_measured(&workspace, &policy, &config, SURFACE);
    check_partitions(&workspace, &presented, &config);
    let payload = match action {
        1 | 2 | 5 => MovePayload::Item(workspace.capture_item_source(ROOT, leaf, MOVED).unwrap()),
        3 | 6 | 8 => MovePayload::Tabs(workspace.capture_node_source(ROOT, group).unwrap()),
        4 | 7 | 9 | 10 => MovePayload::Subtree(workspace.capture_node_source(ROOT, tree).unwrap()),
        _ => unreachable!("the property handles the canonical label action separately"),
    };
    let members = match &payload {
        MovePayload::Item(source) => vec![source.item()],
        MovePayload::Tabs(source) | MovePayload::Subtree(source) => {
            workspace.collect_items_in_subtree(source.node())
        }
    };
    let payload_tabs = match &payload {
        MovePayload::Item(source) => vec![source.item().into()],
        MovePayload::Tabs(source) => workspace.visible_tab_keys(source.node()).into_owned(),
        MovePayload::Subtree(source) => vec![TabKey::for_layout(source.node())],
    };
    let mut payload_nodes = BTreeSet::new();
    if let MovePayload::Tabs(source) | MovePayload::Subtree(source) = &payload {
        let mut pending = vec![source.node()];
        while let Some(node) = pending.pop() {
            assert!(payload_nodes.insert(node));
            pending.extend(
                workspace
                    .node(node)
                    .unwrap()
                    .child_nodes()
                    .map(|(_, child)| child),
            );
        }
    }
    let edge = match (horizontal, reverse) {
        (true, false) => Edge::Right,
        (true, true) => Edge::Left,
        (false, false) => Edge::Bottom,
        (false, true) => Edge::Top,
    };
    let fraction = DockFraction::new(0.5).unwrap();
    let gap = usize::from(!empty_central);
    let (destination, target_id) = match action {
        1..=3 | 10 => (
            DockTarget::Center(workspace.capture_tab_target(ROOT, target).unwrap()),
            DropTargetId::Center {
                surface: SURFACE,
                root: ROOT,
                tabs: target,
            },
        ),
        4 => (
            DockTarget::TabGap {
                target: workspace.capture_tab_target(ROOT, target).unwrap(),
                index: gap,
            },
            DropTargetId::TabGap {
                surface: SURFACE,
                root: ROOT,
                tabs: target,
                index: gap,
            },
        ),
        5..=7 => (
            DockTarget::InnerEdge(
                workspace
                    .capture_inner_edge_target(ROOT, target, edge, fraction)
                    .unwrap(),
            ),
            DropTargetId::InnerEdge {
                surface: SURFACE,
                root: ROOT,
                node: target,
                edge,
            },
        ),
        8 | 9 => (
            DockTarget::OuterEdge(
                workspace
                    .capture_outer_edge_target(ROOT, edge, fraction)
                    .unwrap(),
            ),
            DropTargetId::OuterEdge {
                surface: SURFACE,
                root: ROOT,
                node: root,
                edge,
            },
        ),
        _ => unreachable!(),
    };
    let command = WorkspaceCommand::Move {
        payload: payload.clone(),
        target: destination,
    };
    let mut candidate = workspace.clone();
    let report = WorkspaceTransaction::from_commands([command.clone()])
        .apply(&mut candidate, &policy)
        .expect("generated commands must be legal independently of the resolver");
    assert!(
        matches!(report.outcomes(), [CommandOutcome::Moved { items, .. }] if items == &members)
    );
    candidate.validate().unwrap();
    assert_eq!(candidate.item_multiset(), workspace.item_multiset());
    for (item, count) in candidate.item_multiset() {
        assert_eq!(count, 1);
        assert_eq!(
            candidate.item_is_hidden(item),
            workspace.item_is_hidden(item)
        );
        assert!(candidate.capture_item_source_by_id(item).unwrap().is_some());
    }
    let committed = compile_label_measured(&candidate, &policy, &config, SURFACE);
    check_partitions(&candidate, &committed, &config);
    let resolved = resolve_target(
        &workspace, &policy, &presented, &presented, payload, target_id,
    );
    assert_eq!(resolved.command(), &command);
    if action <= 4 || action == 10 {
        let tabs = committed
            .tabs_records()
            .iter()
            .find(|record| record.id().tabs == target)
            .unwrap();
        if bare {
            assert!(
                resolved.preview_tab_strip().is_none(),
                "a promoted bare Pane has no retained strip measurement"
            );
        } else {
            check_future_strip(&resolved, &committed, &candidate, target, &payload_tabs);
        }
        let expected = if action == 4 {
            let bar = target_bar(&committed, target);
            let inserted = bar
                .members()
                .iter()
                .find(|member| member.tab().tab == payload_tabs[0])
                .unwrap()
                .full_bounds();
            let width = config.splitter_thickness().min(bar.viewport().width());
            let x = (inserted.x() - width * 0.5).clamp(
                bar.viewport().x(),
                (bar.viewport().max().x() - width).max(bar.viewport().x()),
            );
            rect(x, bar.viewport().y(), width, bar.viewport().height())
        } else {
            tabs.content_bounds()
        };
        assert_eq!(resolved.visual().rect(), expected);
    } else {
        assert!(resolved.preview_tab_strip().is_none());
        let expected = if action == 5 {
            committed
                .pane_records()
                .iter()
                .find(|pane| pane.item() == MOVED)
                .unwrap()
                .bounds()
        } else {
            // A payload's old structural roster excludes unrelated empty
            // destinations even when their ancestor has the same item set.
            // Surviving descendants also cover a canonically flattened Split.
            let mut bounds = None;
            for (node, result) in committed
                .layout_facts()
                .unwrap()
                .root(ROOT)
                .unwrap()
                .projection()
                .nodes
                .iter_preorder()
            {
                if result.visible && payload_nodes.contains(node) {
                    bounds = Some(match bounds {
                        None => result.rect,
                        Some(previous) => union(previous, result.rect),
                    });
                }
            }
            bounds.expect("a visible moved payload has a complete candidate scope")
        };
        let actual = resolved.visual().rect();
        // Reconstructing a union's extent can round its far edge once more
        // than borrowing the original rectangle. Keep subpixel solver errors
        // visible while allowing that final floating-point addition.
        for (actual, expected) in [
            (actual.x(), expected.x()),
            (actual.y(), expected.y()),
            (actual.max().x(), expected.max().x()),
            (actual.max().y(), expected.max().y()),
        ] {
            assert!((actual - expected).abs() < 1.0e-9, "{actual} != {expected}");
        }
    }
    (candidate, committed, resolved)
}

fn check_future_strip(
    resolved: &ResolvedDrop,
    committed: &PresentationPlan,
    workspace: &Workspace,
    target: NodeId,
    payload: &[TabKey],
) {
    let future = resolved
        .preview_tab_strip()
        .expect("this measured destination retains compatible labels and strip metrics");
    let bar = target_bar(committed, target);
    assert_eq!(future.bounds, bar.bounds());
    assert_eq!(future.viewport, bar.viewport());
    assert_eq!(future.scroll_offset, bar.scroll_offset());
    assert_eq!(future.maximum_scroll_offset, bar.maximum_scroll_offset());
    assert_eq!(
        future
            .tabs
            .iter()
            .filter(|tab| tab.payload)
            .map(|tab| tab.key)
            .collect::<Vec<_>>(),
        payload
    );
    assert_eq!(future.tabs.len(), bar.members().len());
    for (predicted, actual) in future.tabs.iter().zip(bar.members()) {
        assert_eq!(predicted.key, actual.tab().tab);
        assert_eq!(predicted.ordinal, actual.ordinal());
        assert_eq!(predicted.full_bounds, actual.full_bounds());
        assert_eq!(predicted.clip_bounds, bar.viewport());
        assert_eq!(
            predicted.selected,
            committed.selected_in_tabs(target) == Some(predicted.key)
        );
        let label = workspace.tab_label(target, predicted.key).unwrap();
        assert_eq!(committed.tab_label(actual.tab()), Some(label));
        assert_eq!(
            predicted.layout_title.as_deref(),
            match label {
                TabLabel::Item(_) => None,
                TabLabel::Layout { title } => Some(title),
            }
        );
        let visible = committed
            .tab_records()
            .iter()
            .find(|tab| *tab.id() == actual.tab());
        assert_eq!(
            predicted.visible_bounds,
            visible.map(|tab| tab.visible_bounds())
        );
        assert_eq!(
            predicted.text_layout_bounds,
            visible.map(|tab| tab.text_layout_bounds())
        );
        assert_eq!(predicted.text_bounds, visible.map(|tab| tab.text_bounds()));
        assert_eq!(
            predicted.close_visual_bounds,
            visible.and_then(|tab| tab.close_visual_bounds())
        );
    }
}

fn check_partitions(
    workspace: &Workspace,
    plan: &PresentationPlan,
    config: &DockPresentationConfig,
) {
    let facts = plan.layout_facts().unwrap().root(ROOT).unwrap();
    let projection = facts.projection();
    let emitted = plan
        .pane_records()
        .iter()
        .map(|pane| pane.item())
        .collect::<BTreeSet<_>>();
    assert_eq!(emitted.len(), plan.pane_records().len());
    for item in workspace.item_multiset().keys().copied() {
        assert_eq!(emitted.contains(&item), workspace.item_is_active(item));
        assert_eq!(
            facts
                .pane_sizing(crate::presentation::manifest::PaneSizingKey::new(
                    ROOT, item
                ))
                .is_some(),
            !workspace.item_is_hidden(item)
        );
    }
    for record in plan.tabs_records() {
        let bounds = record.bounds();
        let content = record.content_bounds();
        let height = config.tab_bar_height().min(bounds.height());
        assert_eq!(content.x(), bounds.x());
        assert_eq!(content.max(), bounds.max());
        assert_eq!(content.y(), bounds.y() + height);
        let bar = target_bar(plan, record.id().tabs);
        assert_eq!(bar.bounds().min(), bounds.min());
        assert_eq!(bar.bounds().max().x(), bounds.max().x());
        assert_eq!(bar.bounds().max().y(), content.y());
        if let Some(child) = workspace
            .visible_tab_selection(record.id().tabs)
            .and_then(TabKey::layout)
        {
            assert_eq!(
                projection.nodes[&child].rect, content,
                "each Tabs layer removes exactly its own header"
            );
        }
    }
    for (node, value) in projection.nodes.iter_preorder() {
        if !value.visible {
            continue;
        }
        if let Some(Node::Split { axis, children, .. }) = workspace.node(*node) {
            let children = children
                .iter()
                .map(|child| projection.nodes[child])
                .filter(|child| child.visible)
                .collect::<Vec<_>>();
            let mut end = match axis {
                Axis::Horizontal => value.rect.x(),
                Axis::Vertical => value.rect.y(),
            };
            for (index, child) in children.iter().enumerate() {
                let start = match axis {
                    Axis::Horizontal => child.rect.x(),
                    Axis::Vertical => child.rect.y(),
                };
                close(
                    start,
                    end + if index == 0 {
                        0.0
                    } else {
                        config.splitter_thickness()
                    },
                );
                end = match axis {
                    Axis::Horizontal => child.rect.max().x(),
                    Axis::Vertical => child.rect.max().y(),
                };
            }
            close(
                end,
                match axis {
                    Axis::Horizontal => value.rect.max().x(),
                    Axis::Vertical => value.rect.max().y(),
                },
            );
        }
    }
    for (_, grid) in projection.grids.iter() {
        if !projection.nodes[&grid.node].visible {
            continue;
        }
        for (index, child) in grid.children.iter().enumerate() {
            let child = &projection.nodes[child];
            if !child.visible {
                continue;
            }
            let column = &grid.columns.tracks[index % grid.shape.columns];
            let row = &grid.rows.tracks[index / grid.shape.columns];
            assert_eq!(
                child.slot,
                LogicalRect::from_min_max(
                    LogicalPoint::new(column.bounds.x(), row.bounds.y()).unwrap(),
                    LogicalPoint::new(column.bounds.max().x(), row.bounds.max().y()).unwrap()
                )
                .unwrap()
            );
        }
    }
}

fn union(left: LogicalRect, right: LogicalRect) -> LogicalRect {
    LogicalRect::from_min_max(
        LogicalPoint::new(left.x().min(right.x()), left.y().min(right.y())).unwrap(),
        LogicalPoint::new(
            left.max().x().max(right.max().x()),
            left.max().y().max(right.max().y()),
        )
        .unwrap(),
    )
    .unwrap()
}

fn close(actual: f64, expected: f64) {
    // Independent partition sums can accumulate f64 roundoff. Direct record
    // comparisons and the declared Fixed Grid coordinates remain exact.
    assert!(
        (actual - expected).abs()
            <= 32.0 * f64::EPSILON * actual.abs().max(expected.abs()).max(1.0),
        "{actual} != {expected}"
    );
}
