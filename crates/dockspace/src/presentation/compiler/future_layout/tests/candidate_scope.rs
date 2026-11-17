//! Preview ownership follows the checked candidate, including legal no-op moves.

use super::*;
use crate::graph::command::{CommandOutcome, DockFraction, DockTarget, Edge, WorkspaceCommand};

const SECOND: ItemId = ItemId::new(2);
const INACTIVE: ItemId = ItemId::new(3);
const RETAINED: ItemId = ItemId::new(10);

fn config() -> DockPresentationConfig {
    DockPresentationConfig::builder()
        .splitter_thickness(4.0)
        .build()
        .unwrap()
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
    LogicalRect::new(x, y, width, height).unwrap()
}

#[test]
fn preserve_layout_group_self_gap_is_suppressed_without_changing_the_graph() {
    check_preserve_layout_group_self_gap(false);
}

#[test]
fn central_preserve_layout_group_self_gap_is_a_reachable_exact_noop() {
    check_preserve_layout_group_self_gap(true);
}

fn check_preserve_layout_group_self_gap(central_source: bool) {
    let mut builder = Workspace::builder();
    let source = builder.insert_node(Node::tabs([MOVED, SECOND]));
    let target = builder.insert_node(Node::tabs([ItemId::new(11)]));
    let row = builder.insert_node(Node::equal_split(Axis::Horizontal, [source, target]).unwrap());
    let top = builder.insert_node(Node::tabs([RETAINED]));
    let split = builder.insert_node(Node::equal_split(Axis::Vertical, [top, row]).unwrap());
    builder.set_root(
        ROOT,
        RootRecord::new(split).with_central(if central_source { source } else { target }),
    );
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    let workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = config();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    let mut states = TabStripStateStore::default();
    let presented = compile(&workspace, &policy, &config, strip, &mut states);
    assert_eq!(
        target_bar(&presented, source).bounds(),
        rect(0.0, 302.0, 498.0, 28.0)
    );

    // PreserveLayout converts the group into this exact Subtree payload before
    // transaction preflight; returning it to its own group does not nest it.
    let payload = MovePayload::Subtree(workspace.capture_node_source(ROOT, source).unwrap());
    let command = WorkspaceCommand::Move {
        payload: payload.clone(),
        target: DockTarget::TabGap {
            target: workspace.capture_tab_target(ROOT, source).unwrap(),
            index: 1,
        },
    };
    let mut candidate = workspace.clone();
    let report = WorkspaceTransaction::from_commands([command.clone()])
        .apply(&mut candidate, &policy)
        .expect("returning an entire group to itself is a legal graph no-op");
    assert!(!report.changed());
    assert!(matches!(
        report.outcomes(),
        [CommandOutcome::Moved { items, source_root: ROOT, target_root: ROOT, changed: false }]
            if items.as_slice() == [MOVED, SECOND]
    ));
    assert_eq!(candidate, workspace);
    assert_eq!(
        candidate.visible_tab_keys(source).as_ref(),
        [MOVED, SECOND].map(TabKey::for_item)
    );
    assert_eq!(candidate.collect_items_in_subtree(source), [MOVED, SECOND]);
    assert_eq!(candidate.visible_tab_selection(source), Some(MOVED.into()));
    let committed = compile(&candidate, &policy, &config, strip, &mut states);
    assert_eq!(committed.pane_records(), presented.pane_records());
    assert_eq!(committed.tab_bar_records(), presented.tab_bar_records());
    assert_eq!(committed.tab_records(), presented.tab_records());

    let target_id = DropTargetId::TabGap {
        surface: SURFACE,
        root: ROOT,
        tabs: source,
        index: 1,
    };
    let target_record = presented.drop_target(target_id).unwrap();
    let hit = target_record.region().rect();
    // The ordinary resolver checks the root's outer band before magnetic
    // content basins. Keep this direct gap inside the root, beyond that band.
    let point =
        LogicalPoint::new(hit.x() + hit.width() * 0.5, hit.y() + hit.height() * 0.5).unwrap();
    let outer_extent = config.magnetic_outer_band_maximum();
    assert!(point.x() > outer_extent && point.x() < 1000.0 - outer_extent);
    assert!(point.y() > outer_extent && point.y() < 600.0 - outer_extent);
    let version = WorkspaceVersion::initial();
    let index = WorkspaceIndex::build(&workspace, version).unwrap();
    assert!(
        index
            .capture_complete_root_source(&workspace, version, &payload)
            .unwrap()
            .is_none()
    );
    let query = resolve_presented_drop(
        SurfaceSceneStamp::new(
            presented.measurement_ticket().unwrap(),
            SurfaceSceneRevision::new(1),
        ),
        &presented,
        presented.layout_facts(),
        &workspace,
        version,
        &index,
        &policy,
        DragSessionId::new(version.epoch(), DragGeneration::new(1)),
        payload,
        None,
        point,
    )
    .expect("a legal self-gap query must not fail during preview projection");
    let resolution = query.into_evaluation_parts().0;
    if !central_source {
        // A disappearing source leaf is suppressed before target preflight.
        assert!(matches!(resolution, DropResolution::KnownNone(_)));
        return;
    }
    // Central anchors remain destinations, including their own direct gaps.
    let DropResolution::Resolved(resolved) = resolution else {
        panic!("a central source's self-gap must resolve: {resolution:?}");
    };
    assert_eq!(resolved.target_id(), target_id);
    assert_eq!(resolved.command(), &command);
    assert_eq!(resolved.visual(), target_record.visual());
    assert!(resolved.preview_tab_strip().is_none());
    assert_eq!(candidate, workspace);
}

#[derive(Clone, Copy)]
enum GroupEdge {
    InnerRight,
    OuterBottom,
}

#[test]
fn composable_group_inner_edge_preview_includes_the_outer_tabs_header() {
    check_composable_group_edge(false, GroupEdge::InnerRight, rect(502.0, 0.0, 498.0, 600.0));
}

#[test]
fn composable_group_outer_edge_preview_includes_the_outer_tabs_header() {
    check_composable_group_edge(
        false,
        GroupEdge::OuterBottom,
        rect(0.0, 302.0, 1000.0, 298.0),
    );
}

#[test]
fn central_composable_group_edge_preview_uses_the_new_carrier_instead_of_the_empty_anchor() {
    check_composable_group_edge(true, GroupEdge::InnerRight, rect(752.0, 0.0, 248.0, 600.0));
}

#[test]
fn flattened_subtree_edge_preview_retains_the_complete_nested_tabs_headers() {
    let source_surface = SurfaceId::new(2);
    let source_root = RootId::new(2);
    let second_inactive = ItemId::new(4);
    let mut builder = Workspace::builder();
    let left_inner = builder.insert_node(Node::tabs([MOVED, INACTIVE]));
    let right_inner = builder.insert_node(Node::tabs([SECOND, second_inactive]));
    let left_tab = TabKey::for_layout(left_inner);
    let right_tab = TabKey::for_layout(right_inner);
    let left_outer = builder.insert_node(Node::tabs_from_keys(vec![left_tab], Some(left_tab)));
    let right_outer = builder.insert_node(Node::tabs_from_keys(vec![right_tab], Some(right_tab)));
    let source = builder
        .insert_node(Node::equal_split(Axis::Horizontal, [left_outer, right_outer]).unwrap());
    let target = builder.insert_node(Node::tabs([RETAINED]));
    let remainder = builder.insert_node(Node::tabs([ItemId::new(11)]));
    let destination =
        builder.insert_node(Node::equal_split(Axis::Horizontal, [target, remainder]).unwrap());
    builder.set_root(ROOT, RootRecord::new(destination).with_central(target));
    builder.set_root(source_root, RootRecord::new(source));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    builder.set_surface(source_surface, SurfacePresentation::with_main(source_root));
    let workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = config();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    let mut states = TabStripStateStore::default();
    let target_plan = compile(&workspace, &policy, &config, strip, &mut states);
    let source_plan = compile_on_surface(
        &workspace,
        &policy,
        &config,
        strip,
        &mut states,
        source_surface,
    );
    let payload_items = [MOVED, INACTIVE, SECOND, second_inactive];
    assert_eq!(workspace.collect_items_in_subtree(source), payload_items);
    let payload = MovePayload::Subtree(workspace.capture_node_source(source_root, source).unwrap());
    let command = WorkspaceCommand::Move {
        payload: payload.clone(),
        target: DockTarget::InnerEdge(
            workspace
                .capture_inner_edge_target(
                    ROOT,
                    target,
                    Edge::Right,
                    DockFraction::new(0.5).unwrap(),
                )
                .unwrap(),
        ),
    };
    let mut candidate = workspace.clone();
    let report = WorkspaceTransaction::from_commands([command.clone()])
        .apply(&mut candidate, &policy)
        .expect("same-axis insertion may flatten the source while retaining nested Tabs");
    assert!(report.changed());
    assert!(matches!(
        report.outcomes(),
        [CommandOutcome::Moved { items, source_root: from, target_root: ROOT, changed: true }]
            if *from == source_root && items.as_slice() == payload_items
    ));
    candidate.validate().unwrap();
    assert!(candidate.node(source).is_none());
    assert!(candidate.root(source_root).is_none());
    assert_eq!(candidate.root(ROOT).unwrap().node, destination);
    assert!(
        matches!(candidate.node(destination), Some(Node::Split { axis: Axis::Horizontal, children, .. })
        if children.as_slice() == [target, left_outer, right_outer, remainder])
    );
    assert_eq!(candidate.item_multiset(), workspace.item_multiset());
    for (outer, inner, tab, active, inactive) in [
        (left_outer, left_inner, left_tab, MOVED, INACTIVE),
        (right_outer, right_inner, right_tab, SECOND, second_inactive),
    ] {
        assert_eq!(candidate.parent_link(ROOT, inner).unwrap().parent, outer);
        assert_eq!(
            candidate.parent_link(ROOT, outer).unwrap().parent,
            destination
        );
        assert_eq!(candidate.visible_tab_keys(outer).as_ref(), [tab]);
        assert_eq!(
            candidate.collect_items_in_subtree(outer),
            [active, inactive]
        );
        assert_eq!(
            candidate.locate_item_by_id(active).unwrap().owner_node(),
            inner
        );
        assert!(candidate.item_is_active(active));
        assert!(!candidate.item_is_active(inactive));
    }
    let committed = compile(&candidate, &policy, &config, strip, &mut states);
    let expected = rect(251.0, 0.0, 251.0, 600.0);
    // Flattened shares are [1/4, 1/8, 1/8, 1/2] over 1000 - 3*4.
    // Both moved branches retain an outer and an inner 28-unit header.
    for (outer, inner, item, x) in [
        (left_outer, left_inner, MOVED, 251.0),
        (right_outer, right_inner, SECOND, 378.5),
    ] {
        assert_eq!(
            target_bar(&committed, outer).bounds(),
            rect(x, 0.0, 123.5, 28.0)
        );
        assert_eq!(
            target_bar(&committed, inner).bounds(),
            rect(x, 28.0, 123.5, 28.0)
        );
        let container = committed
            .tabs_records()
            .iter()
            .find(|tabs| tabs.id().tabs == outer)
            .unwrap();
        assert_eq!(container.bounds(), rect(x, 0.0, 123.5, 600.0));
        let pane = committed
            .pane_records()
            .iter()
            .find(|pane| pane.item() == item)
            .unwrap();
        assert_eq!(pane.bounds(), rect(x, 56.0, 123.5, 544.0));
    }
    let resolved = resolve_target(
        &workspace,
        &policy,
        &target_plan,
        &source_plan,
        payload,
        DropTargetId::InnerEdge {
            surface: SURFACE,
            root: ROOT,
            node: target,
            edge: Edge::Right,
        },
    );
    assert_eq!(resolved.command(), &command);
    assert!(resolved.preview_tab_strip().is_none());
    assert_eq!(resolved.visual().rect(), expected);
}

fn check_composable_group_edge(central_source: bool, edge: GroupEdge, expected: LogicalRect) {
    let mut builder = Workspace::builder();
    let first = builder.insert_node(Node::pane(MOVED));
    let second = builder.insert_node(Node::pane(SECOND));
    let inner = builder.insert_node(Node::equal_split(Axis::Horizontal, [first, second]).unwrap());
    let layout_tab = TabKey::for_layout(inner);
    let source = builder.insert_node(Node::tabs_from_keys(
        vec![layout_tab, TabKey::for_item(INACTIVE)],
        Some(layout_tab),
    ));
    let target = builder.insert_node(Node::tabs([RETAINED]));
    let split = builder.insert_node(Node::equal_split(Axis::Horizontal, [source, target]).unwrap());
    builder.set_root(
        ROOT,
        RootRecord::new(split).with_central(if central_source { source } else { target }),
    );
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    let workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = config();
    let strip = TabStripMetrics::new(0.0, 0.0).unwrap();
    let mut states = TabStripStateStore::default();
    let presented = compile(&workspace, &policy, &config, strip, &mut states);
    assert_eq!(
        target_bar(&presented, source).bounds(),
        rect(0.0, 0.0, 498.0, 28.0)
    );
    assert_eq!(
        workspace.collect_items_in_subtree(source),
        [MOVED, SECOND, INACTIVE]
    );
    assert!(workspace.item_is_active(MOVED));
    assert!(workspace.item_is_active(SECOND));
    assert!(!workspace.item_is_active(INACTIVE));

    let payload = MovePayload::Tabs(workspace.capture_node_source(ROOT, source).unwrap());
    let fraction = DockFraction::new(0.5).unwrap();
    let (target_id, destination) = match edge {
        GroupEdge::InnerRight => (
            DropTargetId::InnerEdge {
                surface: SURFACE,
                root: ROOT,
                node: target,
                edge: Edge::Right,
            },
            DockTarget::InnerEdge(
                workspace
                    .capture_inner_edge_target(ROOT, target, Edge::Right, fraction)
                    .unwrap(),
            ),
        ),
        GroupEdge::OuterBottom => (
            DropTargetId::OuterEdge {
                surface: SURFACE,
                root: ROOT,
                node: split,
                edge: Edge::Bottom,
            },
            DockTarget::OuterEdge(
                workspace
                    .capture_outer_edge_target(ROOT, Edge::Bottom, fraction)
                    .unwrap(),
            ),
        ),
    };
    let command = WorkspaceCommand::Move {
        payload: payload.clone(),
        target: destination,
    };
    let mut candidate = workspace.clone();
    let report = WorkspaceTransaction::from_commands([command.clone()])
        .apply(&mut candidate, &policy)
        .expect("the group edge move must pass graph authority and central rules");
    assert!(report.changed());
    assert!(matches!(
        report.outcomes(),
        [CommandOutcome::Moved { items, source_root: ROOT, target_root: ROOT, changed: true }]
            if items.as_slice() == [MOVED, SECOND, INACTIVE]
    ));
    candidate.validate().unwrap();
    assert_eq!(candidate.item_multiset(), workspace.item_multiset());
    let carrier = candidate.parent_link(ROOT, inner).unwrap().parent;
    assert!(matches!(candidate.node(carrier), Some(Node::Tabs { .. })));
    assert_eq!(candidate.node_root(carrier), Some(ROOT));
    assert_eq!(
        candidate.collect_items_in_subtree(carrier),
        [MOVED, SECOND, INACTIVE]
    );
    assert_eq!(
        candidate.visible_tab_keys(carrier).as_ref(),
        [layout_tab, TabKey::for_item(INACTIVE)]
    );
    assert_eq!(candidate.visible_tab_selection(carrier), Some(layout_tab));
    assert_eq!(
        candidate.locate_item_by_id(MOVED).unwrap().owner_node(),
        first
    );
    assert_eq!(
        candidate.locate_item_by_id(SECOND).unwrap().owner_node(),
        second
    );
    assert_eq!(
        candidate.locate_item_by_id(INACTIVE).unwrap().owner_node(),
        carrier
    );
    assert!(candidate.item_is_active(MOVED));
    assert!(candidate.item_is_active(SECOND));
    assert!(!candidate.item_is_active(INACTIVE));
    if central_source {
        assert_ne!(carrier, source);
        assert_eq!(candidate.root(ROOT).unwrap().central, Some(source));
        assert_eq!(candidate.node_root(source), Some(ROOT));
        assert!(matches!(
            candidate.node(source),
            Some(Node::Tabs { items, selected: None, mru, .. }) if items.is_empty() && mru.is_empty()
        ));
        assert!(candidate.collect_items_in_subtree(source).is_empty());
    } else {
        assert_eq!(carrier, source);
    }

    let committed = compile(&candidate, &policy, &config, strip, &mut states);
    let container = committed
        .tabs_records()
        .iter()
        .find(|tabs| tabs.id().tabs == carrier)
        .unwrap();
    assert_eq!(container.bounds(), expected);
    assert_eq!(
        target_bar(&committed, carrier).bounds(),
        rect(expected.x(), expected.y(), expected.width(), 28.0)
    );
    let child_width = (expected.width() - 4.0) * 0.5;
    for (item, x) in [
        (MOVED, expected.x()),
        (SECOND, expected.x() + child_width + 4.0),
    ] {
        let pane = committed
            .pane_records()
            .iter()
            .find(|pane| pane.item() == item)
            .unwrap();
        assert_eq!(
            pane.bounds(),
            rect(
                x,
                expected.y() + 28.0,
                child_width,
                expected.height() - 28.0
            )
        );
    }
    assert!(
        !committed
            .pane_records()
            .iter()
            .any(|pane| pane.item() == INACTIVE)
    );
    if central_source {
        let anchor = committed
            .tabs_records()
            .iter()
            .find(|tabs| tabs.id().tabs == source)
            .unwrap();
        assert_eq!(anchor.bounds(), rect(0.0, 0.0, 496.0, 600.0));
        assert_ne!(anchor.bounds(), expected);
    }

    let resolved = resolve_target(
        &workspace, &policy, &presented, &presented, payload, target_id,
    );
    assert_eq!(resolved.command(), &command);
    assert!(resolved.preview_tab_strip().is_none());
    assert_eq!(resolved.visual().rect(), expected);
    assert_eq!(resolved.visual().rect(), container.bounds());
    assert_ne!(resolved.visual().rect().y(), expected.y() + 28.0);
}
