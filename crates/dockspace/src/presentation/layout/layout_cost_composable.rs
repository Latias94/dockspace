//! Opt-in shared-grid and inactive-layout attribution with analytic postconditions.

use std::num::NonZeroUsize;

use super::*;
use crate::ids::TabKey;
use crate::model::DockspaceGridShape;

struct Fixture {
    workspace: Workspace,
    root: NodeId,
    grid: NodeId,
    cells: Vec<(NodeId, ItemId)>,
    inline: Option<ItemId>,
    rows: usize,
    columns: usize,
}

fn fixture(count: usize, inactive: bool) -> Fixture {
    let (rows, columns) = match count {
        16 => (4, 4),
        128 => (8, 16),
        1024 => (32, 32),
        _ => panic!("unsupported composition workload size"),
    };
    let mut builder = Workspace::builder();
    let cells = (1..=count)
        .map(|index| {
            let item = ItemId::new(u64::try_from(index).unwrap());
            (builder.insert_node(Node::pane(item)), item)
        })
        .collect::<Vec<_>>();
    let grid = builder.insert_node(Node::Grid {
        shape: DockspaceGridShape::Fixed {
            rows: NonZeroUsize::new(rows).unwrap(),
            columns: NonZeroUsize::new(columns).unwrap(),
        },
        children: cells.iter().map(|(node, _)| *node).collect(),
        shares: None,
        layout_title: None,
    });
    let inline = inactive.then(|| ItemId::new(u64::try_from(count + 1).unwrap()));
    let root = inline.map_or(grid, |item| {
        builder.insert_node(Node::tabs_from_keys(
            vec![item.into(), TabKey::for_layout(grid)],
            Some(item.into()),
        ))
    });
    builder.set_root(ROOT, RootRecord::new(root));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(ROOT));
    Fixture {
        workspace: builder.build().unwrap(),
        root,
        grid,
        cells,
        inline,
        rows,
        columns,
    }
}

fn check_rect(digest: &mut u64, actual: LogicalRect, expected: LogicalRect) {
    for (actual, expected) in [actual.x(), actual.y(), actual.width(), actual.height()]
        .into_iter()
        .zip([
            expected.x(),
            expected.y(),
            expected.width(),
            expected.height(),
        ])
    {
        assert!((actual - expected).abs() <= 1e-8, "{actual} != {expected}");
        assert!(expected >= 0.0 && expected.fract() == 0.0 && expected <= 65_536.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let integer = expected as u64;
        for byte in integer.to_le_bytes() {
            *digest = (*digest ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    }
}

fn validate(fixture: &Fixture, projection: &LayoutProjection, bounds: LogicalRect) -> u64 {
    assert_eq!(projection.nodes.len(), fixture.workspace.nodes().count());
    assert!(projection.splits.is_empty());
    assert_eq!(projection.grids.iter().count(), 1);
    let grid = projection.grids.get(&fixture.grid).unwrap();
    assert_eq!(grid.shape.rows, fixture.rows);
    assert_eq!(grid.shape.columns, fixture.columns);
    assert_eq!(
        grid.children.as_ref(),
        fixture
            .cells
            .iter()
            .map(|(node, _)| *node)
            .collect::<Vec<_>>()
    );
    let inactive = fixture.inline.is_some();
    let zero = LogicalRect::new(bounds.x(), bounds.y(), 0.0, 0.0).unwrap();
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    let root = projection.nodes[&fixture.root];
    assert!(root.visible);
    check_rect(&mut digest, root.slot, bounds);
    check_rect(&mut digest, root.rect, bounds);
    let frame = projection.nodes[&fixture.grid];
    assert_eq!(frame.visible, !inactive);
    let expected_grid = if inactive { zero } else { bounds };
    check_rect(&mut digest, frame.slot, expected_grid);
    check_rect(&mut digest, frame.rect, expected_grid);
    let width =
        (bounds.width() - SEPARATOR * (fixture.columns - 1) as f64) / fixture.columns as f64;
    let height = (bounds.height() - SEPARATOR * (fixture.rows - 1) as f64) / fixture.rows as f64;
    for (index, &(node, item)) in fixture.cells.iter().enumerate() {
        let actual = projection.nodes[&node];
        assert_eq!(projection.nodes.pane_item(&node), Some(item));
        assert_eq!(fixture.workspace.item_is_active(item), !inactive);
        assert_eq!(actual.visible, !inactive);
        let expected = if inactive {
            zero
        } else {
            LogicalRect::new(
                bounds.x() + (index % fixture.columns) as f64 * (width + SEPARATOR),
                bounds.y() + (index / fixture.columns) as f64 * (height + SEPARATOR),
                width,
                height,
            )
            .unwrap()
        };
        check_rect(&mut digest, actual.slot, expected);
        check_rect(&mut digest, actual.rect, expected);
    }
    if let Some(inline) = fixture.inline {
        assert!(fixture.workspace.item_is_active(inline));
        let Some(Node::Tabs { selected, .. }) = fixture.workspace.node(fixture.root) else {
            panic!("the inactive-grid workload needs its real outer Tabs owner");
        };
        assert_eq!(*selected, Some(inline.into()));
    }
    digest
}

#[test]
#[ignore = "opt-in composition attribution; uses DOCKSPACE_LAYOUT_COST_* settings"]
fn composable_layout_cost() {
    let path = std::env::var_os("DOCKSPACE_LAYOUT_COST_OUT")
        .expect("the composition workload requires an unused CSV output path");
    let samples = setting("DOCKSPACE_LAYOUT_COST_SAMPLES", 96);
    let warmup = setting("DOCKSPACE_LAYOUT_COST_WARMUP", 16);
    let panes =
        std::env::var("DOCKSPACE_LAYOUT_COST_PANES").unwrap_or_else(|_| "16,128,1024".into());
    let mut output = BufWriter::new(std::fs::File::create_new(path).unwrap());
    writeln!(output, "product,panes,case,sample,preparation_ns,elapsed_ns,nodes,leaves,splits,width,height,geometry_digest").unwrap();
    assert!(samples > 0);
    for count in panes
        .split(',')
        .map(|value| value.parse::<usize>().unwrap())
    {
        for case in ["grid_full", "inactive_layout_tab"] {
            let fixture = fixture(count, case == "inactive_layout_tab");
            let workspace = &fixture.workspace;
            let nodes = workspace.nodes().count();
            // Count actual terminal nodes, not a lowered grid or synthetic tab groups.
            // The inactive case owns one extra inline item in its nonterminal Tabs.
            let leaves = workspace
                .nodes()
                .filter(|(_, node)| node.child_nodes().next().is_none())
                .count();
            let splits = workspace
                .nodes()
                .filter(|(_, node)| matches!(node, Node::Split { .. }))
                .count();
            assert_eq!(nodes, count + 1 + usize::from(fixture.inline.is_some()));
            assert_eq!(leaves, count);
            assert_eq!(splits, 0);
            let constraints = workspace
                .nodes()
                .filter(|(_, node)| matches!(node, Node::Pane { .. } | Node::Tabs { .. }))
                .map(|(node, _)| {
                    (
                        node,
                        Constraints::new(
                            LogicalSize::new(0.0, 0.0).unwrap(),
                            LogicalSize::new(65_536.0, 65_536.0).unwrap(),
                        )
                        .unwrap(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let metrics = LayoutMetrics::new(SEPARATOR).unwrap();
            for iteration in 0..warmup.checked_add(samples).unwrap() {
                let bounds = LogicalRect::new(0.0, 0.0, WIDTHS[iteration % 2], HEIGHT).unwrap();
                let prepare = Instant::now();
                let measured = measure_root(
                    black_box(workspace),
                    ROOT,
                    LeafMeasurements::Constraints(black_box(&constraints)),
                    metrics,
                )
                .unwrap();
                let preparation_ns = prepare.elapsed().as_nanos();
                let start = Instant::now();
                let projection = black_box(measured).project(black_box(bounds), &[]).unwrap();
                black_box(&projection);
                let elapsed_ns = start.elapsed().as_nanos();
                let digest = validate(&fixture, &projection, bounds);
                if iteration >= warmup {
                    writeln!(output, "dockspace,{count},{case},{},{preparation_ns},{elapsed_ns},{nodes},{leaves},{splits},{},{},{digest:016x}",
                        iteration - warmup, bounds.width(), bounds.height()).unwrap();
                }
                drop(projection);
            }
        }
    }
    output.flush().unwrap();
}
