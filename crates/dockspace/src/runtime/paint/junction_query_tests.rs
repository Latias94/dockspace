use super::*;
use crate::graph::{Node, RootRecord, Workspace};
use crate::runtime::{DockspaceSession, SurfaceSplitterAdjustment, UniformSurfaceMetrics};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(2);
const AXES: [DockspaceAxis; 2] = [DockspaceAxis::Horizontal, DockspaceAxis::Vertical];

fn measured_grid(panes: u64, root: RootId, allowed: [bool; 2]) -> DockspaceSession {
    let mut builder = Workspace::builder();
    let mut nodes = (1..=panes)
        .map(|item| builder.insert_node(Node::tabs([ItemId::new(item)])))
        .collect::<Vec<_>>();
    let mut axis = Axis::Horizontal;
    while nodes.len() > 1 {
        nodes = nodes
            .chunks_exact(2)
            .map(|pair| builder.insert_node(Node::equal_split(axis, pair.iter().copied()).unwrap()))
            .collect();
        axis = match axis {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
    }
    builder.set_root(root, RootRecord::new(nodes[0]));
    builder.set_surface(SURFACE, crate::graph::SurfacePresentation::with_main(root));
    let mut policy = crate::policy::DockPolicy::default();
    for (axis, allowed) in AXES.into_iter().zip(allowed) {
        policy.set_allow_resize_axis(axis, allowed);
    }
    let mut session = DockspaceSession::from_workspace_for_test(builder.build().unwrap(), policy)
        .expect("the alternating grid initializes");
    let mut frame = session.begin_host_frame().unwrap();
    frame
        .measure_surface(
            SURFACE,
            UniformSurfaceMetrics::new(
                LogicalRect::new(0.0, 0.0, 2048.0, 2048.0).unwrap(),
                LogicalSize::new(32.0, 24.0).unwrap(),
                48.0,
            )
            .unwrap(),
        )
        .unwrap();
    frame.commit().unwrap();
    session
}

// Preserve the original scan and sorted/deduplicated arm traversal independently.
fn frozen_axis_operable(
    plan: SurfacePaintPlan<'_>,
    junction: SplitterJunctionPaintRecord<'_>,
    axis: DockspaceAxis,
) -> bool {
    let axis: Axis = axis.into();
    let Some(record) = plan
        .plan
        .splitter_junction_records()
        .iter()
        .find(|record| record.id() == junction.record.id())
    else {
        return false;
    };
    let directions = match axis {
        Axis::Horizontal => [
            SplitterJunctionDirection::North,
            SplitterJunctionDirection::South,
        ],
        Axis::Vertical => [
            SplitterJunctionDirection::East,
            SplitterJunctionDirection::West,
        ],
    };
    let mut ids = directions
        .into_iter()
        .filter_map(|direction| record.id().arm(direction))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    !ids.is_empty()
        && ids.into_iter().all(|id| {
            plan.plan
                .splitter_record(id)
                .is_some_and(|splitter| splitter.axis() == axis && splitter.operable())
        })
}

#[test]
fn canonical_grid_junction_queries_match_frozen_scans_on_both_axes() {
    let mut saw_shared_arm = false;
    let mut saw_distinct_arms = false;
    for panes in [4, 16] {
        for allowed in [[true, true], [false, true], [true, false], [false, false]] {
            let mut session = measured_grid(panes, ROOT, allowed);
            let mut frame = session.begin_host_frame().unwrap();
            let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
            assert!(
                plan.plan
                    .splitter_records()
                    .windows(2)
                    .all(|pair| pair[0].id() < pair[1].id())
            );
            assert!(
                plan.plan
                    .splitter_junction_records()
                    .windows(2)
                    .all(|pair| pair[0].id() < pair[1].id())
            );
            assert!(plan.splitter_junctions().len() > 0);
            for junction in plan.splitter_junctions() {
                let id = junction.record.id();
                for (axis, expected) in AXES.into_iter().zip(allowed) {
                    assert!(junction.has_axis(axis));
                    assert_eq!(
                        plan.splitter_junction_axis_operable(junction, axis),
                        frozen_axis_operable(plan, junction, axis)
                    );
                    assert_eq!(
                        plan.splitter_junction_axis_operable(junction, axis),
                        expected
                    );
                }
                for (first, second) in [
                    (
                        SplitterJunctionDirection::North,
                        SplitterJunctionDirection::South,
                    ),
                    (
                        SplitterJunctionDirection::East,
                        SplitterJunctionDirection::West,
                    ),
                ] {
                    if let (Some(first), Some(second)) = (id.arm(first), id.arm(second)) {
                        saw_shared_arm |= first == second;
                        saw_distinct_arms |= first != second;
                    }
                }
            }
        }
    }
    assert!(
        saw_shared_arm,
        "a through handle occupies two opposite arms"
    );
    assert!(saw_distinct_arms, "separate handles meet on the same axis");
}

#[test]
fn grid_control_hash_work_is_bounded_per_handle_and_junction() {
    use std::hash::{Hash, Hasher};
    use std::num::NonZeroUsize;

    use crate::model::{
        DockspaceGridShape, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
        DockspaceSurfaceLayout,
    };

    #[derive(Default)]
    struct HashBytes(usize);

    impl Hasher for HashBytes {
        fn finish(&self) -> u64 {
            self.0 as u64
        }

        fn write(&mut self, bytes: &[u8]) {
            self.0 += bytes.len();
        }
    }

    let hash_size = |id: DockspaceControlId| {
        let mut bytes = HashBytes::default();
        id.hash(&mut bytes);
        bytes.0
    };
    let bytes_for_grid = |side: usize| {
        let tracks = NonZeroUsize::new(side).unwrap();
        let content = DockspaceNode::grid(
            DockspaceGridShape::Fixed {
                rows: tracks,
                columns: tracks,
            },
            (1..=side * side).map(|item| DockspaceNode::pane(ItemId::new(item as u64))),
        )
        .unwrap();
        let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
            SURFACE,
            DockspaceRootLayout::new(ROOT, content),
        )])
        .unwrap();
        let mut session = DockspaceSession::from_layout(layout, Default::default()).unwrap();
        let mut frame = session.begin_host_frame().unwrap();
        frame
            .measure_surface(
                SURFACE,
                UniformSurfaceMetrics::new(
                    LogicalRect::new(0.0, 0.0, 2048.0, 2048.0).unwrap(),
                    LogicalSize::new(32.0, 24.0).unwrap(),
                    48.0,
                )
                .unwrap(),
            )
            .unwrap();
        let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
        let handles = plan.splitters().collect::<Vec<_>>();
        let junctions = plan.splitter_junctions().collect::<Vec<_>>();
        assert_eq!(handles.len(), 2 * (side - 1));
        assert_eq!(junctions.len(), (side - 1).pow(2));
        let handle = hash_size(handles[0].input_id());
        let junction = hash_size(junctions[0].input_id());
        assert!(
            handles
                .iter()
                .all(|record| hash_size(record.input_id()) == handle)
        );
        assert!(
            junctions
                .iter()
                .all(|record| hash_size(record.input_id()) == junction)
        );
        (handle, junction)
    };

    assert_eq!(bytes_for_grid(8), bytes_for_grid(16));
}

#[test]
fn a_foreign_junction_cannot_use_current_receiver_or_axis_authority() {
    let mut origin = measured_grid(4, ROOT, [true, true]);
    let mut current = measured_grid(4, ROOT, [false, true]);
    let mut unrelated = measured_grid(4, RootId::new(99), [true, true]);
    let mut origin_frame = origin.begin_host_frame().unwrap();
    let mut current_frame = current.begin_host_frame().unwrap();
    let mut unrelated_frame = unrelated.begin_host_frame().unwrap();
    let origin = origin_frame.paint_plan(SURFACE).unwrap().unwrap();
    let current = current_frame.paint_plan(SURFACE).unwrap().unwrap();
    let unrelated = unrelated_frame.paint_plan(SURFACE).unwrap().unwrap();
    let foreign = origin.splitter_junctions().next().unwrap();
    let local = current.splitter_junctions().next().unwrap();
    assert_eq!(foreign.visual_id(), local.visual_id());
    assert!(foreign.record.operable());
    assert!(current.receiver_for_splitter_junction(foreign).is_none());
    assert!(!current.splitter_junction_operable(foreign));
    for (axis, expected) in AXES.into_iter().zip([false, true]) {
        assert!(!current.splitter_junction_axis_operable(foreign, axis));
        assert!(
            current
                .prepare_splitter_junction_adjustment(
                    foreign,
                    axis,
                    SurfaceSplitterAdjustment::Increment
                )
                .is_none()
        );
        assert_eq!(
            current.splitter_junction_axis_operable(local, axis),
            expected
        );
        assert!(!unrelated.splitter_junction_axis_operable(foreign, axis));
    }
}

#[test]
fn either_distinct_arm_must_exist_and_match_the_axis() {
    let mut session = measured_grid(4, ROOT, [true, true]);
    let mut frame = session.begin_host_frame().unwrap();
    let plan = frame.paint_plan(SURFACE).unwrap().unwrap();
    let junction = plan.splitter_junctions().next().unwrap();
    let id = junction.record.id();
    let (axis, arms) = [
        (
            DockspaceAxis::Horizontal,
            [
                id.arm(SplitterJunctionDirection::North),
                id.arm(SplitterJunctionDirection::South),
            ],
        ),
        (
            DockspaceAxis::Vertical,
            [
                id.arm(SplitterJunctionDirection::East),
                id.arm(SplitterJunctionDirection::West),
            ],
        ),
    ]
    .into_iter()
    .find_map(|(axis, arms)| match arms {
        [Some(first), Some(second)] if first != second => Some((axis, [first, second])),
        _ => None,
    })
    .expect("the grid has two distinct handles on one junction axis");
    for arm in arms {
        for wrong_axis in [false, true] {
            // Keep the canonical order while exercising a missing or mismatched handle.
            let mut altered = PresentationPlan::new(SURFACE, plan.plan.bounds());
            altered.push_splitter_junction_record(*junction.record);
            for splitter in plan.plan.splitter_records() {
                if *splitter.id() != arm {
                    altered.push_splitter_record(splitter.clone());
                } else if wrong_axis {
                    altered.push_splitter_record(SplitterRecord::new(
                        *splitter.id(),
                        splitter.draw_bounds(),
                        splitter.hit(),
                        match splitter.axis() {
                            Axis::Horizontal => Axis::Vertical,
                            Axis::Vertical => Axis::Horizontal,
                        },
                        splitter.before_bounds(),
                        splitter.after_bounds(),
                        splitter.before_minimum_extent(),
                        splitter.after_minimum_extent(),
                        splitter.before_maximum_extent(),
                        splitter.after_maximum_extent(),
                        splitter.child_extents().to_vec(),
                        splitter.central_index(),
                        splitter.weights().to_vec(),
                        splitter.layer(),
                        splitter.operable(),
                    ));
                }
            }
            let altered = SurfacePaintPlan {
                plan: &altered,
                ..plan
            };
            assert!(!altered.splitter_junction_axis_operable(junction, axis));
            assert_eq!(
                altered.splitter_junction_axis_operable(junction, axis),
                frozen_axis_operable(altered, junction, axis)
            );
        }
    }
}
