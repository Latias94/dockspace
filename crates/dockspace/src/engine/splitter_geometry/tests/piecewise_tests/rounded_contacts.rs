//! Outward capacity checks must use the precision of their presented coordinates.

use super::*;
use proptest::prelude::*;

#[test]
fn outward_resize_recovers_from_a_rounded_minimum_at_a_large_origin() {
    check_rounded_contact(true);
}

#[test]
fn outward_resize_recovers_from_a_rounded_maximum_at_a_large_origin() {
    check_rounded_contact(false);
}

fn check_rounded_contact(at_minimum: bool) {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let b = tabs(&mut builder, 2);
        let split = builder.insert_node(
            Node::split(
                axis,
                [a, b],
                if at_minimum { [0.1, 0.9] } else { [0.5, 0.5] },
            )
            .unwrap(),
        );
        let (minimum, maximum, delta, expected) = if at_minimum {
            (200.1, 2000.0, 10.0, 210.1)
        } else {
            (20.0, 200.3, -10.0, 190.3)
        };
        let fixture = Fixture::with_propagation(
            finish(builder, split, None),
            bounds_along(axis, 1_000_000.3, 1001.0),
            BTreeMap::from([
                (a, constraints_along(axis, minimum, maximum)),
                (b, constraints_along(axis, 20.0, 2000.0)),
            ]),
            1.0,
            ResizePropagation::Outward,
        );
        let actual = rect_extent(fixture.projection.nodes[&a].rect, axis);
        assert!(if at_minimum {
            actual < minimum
        } else {
            actual > maximum
        });
        let groups = prepare_resize_axis_groups(
            &fixture.workspace,
            &fixture.plan,
            vec![fixture.handle(split, 0)],
        )
        .expect("a presented rounding residue must allow movement away from the contact");
        assert!(groups[0].piecewise.is_some());
        let updates = resize_axis_group_updates(&groups, delta_along(axis, delta))
            .expect("the inward displacement is feasible");
        let projected = fixture.project_updates(&updates);
        close(rect_extent(projected.nodes[&a].rect, axis), expected);
        let returned = fixture.project_updates(
            &resize_axis_group_updates(&groups, [0.0, 0.0])
                .expect("returning to the press geometry is feasible"),
        );
        assert_eq!(returned, *fixture.projection);
    }
}

#[test]
fn nested_outward_zero_displacement_keeps_original_track_weights() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for capped in [false, true] {
            let mut builder = Workspace::builder();
            let a = tabs(&mut builder, 1);
            let b = tabs(&mut builder, 2);
            let peer = tabs(&mut builder, 3);
            let cross_peer = tabs(&mut builder, 4);
            let inner = builder.insert_node(Node::split(axis, [a, b], [1.0, 1.0]).unwrap());
            let cross_axis = match axis {
                Axis::Horizontal => Axis::Vertical,
                Axis::Vertical => Axis::Horizontal,
            };
            let across =
                builder.insert_node(Node::equal_split(cross_axis, [inner, cross_peer]).unwrap());
            let outer = builder.insert_node(Node::split(axis, [across, peer], [1.0, 4.0]).unwrap());
            let fixture = Fixture::with_mode(
                finish(builder, outer, None),
                bounds_along(axis, 1_000_000.3, if capped { 1002.5 } else { 1001.5 }),
                BTreeMap::from([
                    (
                        a,
                        constraints_along(axis, 20.0, if capped { 80.1 } else { 2000.0 }),
                    ),
                    (
                        b,
                        constraints_along(axis, 20.0, if capped { 119.2 } else { 2000.0 }),
                    ),
                    (peer, constraints_along(axis, 20.0, 2000.0)),
                    (cross_peer, constraints_along(axis, 20.0, 2000.0)),
                ]),
                1.0,
                ResizePropagation::Outward,
                capped,
            );
            assert_ne!(
                fixture.projection.splits.get(&outer).unwrap().children[0].extent,
                rect_extent(fixture.projection.nodes[&inner].rect, axis),
                "the parent track and child rectangle must exercise distinct rounding paths"
            );
            let groups = fixture.groups(&[(outer, 0)]);
            let updates = resize_axis_group_updates(&groups, [0.0, 0.0]).unwrap();
            assert!(updates.iter().all(|update| update.source().node() == outer));
            assert_eq!(fixture.project_updates(&updates), *fixture.projection);
        }
    }
}

#[test]
fn capped_nested_tracks_keep_their_baseline_until_the_offered_slot_reaches_the_cap() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut builder = Workspace::builder();
        let a = tabs(&mut builder, 1);
        let b = tabs(&mut builder, 2);
        let peer = tabs(&mut builder, 3);
        let cross_peer = tabs(&mut builder, 4);
        let inner = builder.insert_node(Node::split(axis, [a, b], [1.0, 1.0]).unwrap());
        let cross_axis = match axis {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
        let across =
            builder.insert_node(Node::equal_split(cross_axis, [inner, cross_peer]).unwrap());
        let outer = builder.insert_node(Node::equal_split(axis, [peer, across]).unwrap());
        let fixture = Fixture::with_mode(
            finish(builder, outer, None),
            bounds_along(axis, 1_000_000.3, 1001.0),
            BTreeMap::from([
                (a, constraints_along(axis, 20.0, 80.1)),
                (b, constraints_along(axis, 20.0, 119.2)),
                (peer, constraints_along(axis, 20.0, 2000.0)),
                (cross_peer, constraints_along(axis, 20.0, 2000.0)),
            ]),
            1.0,
            ResizePropagation::Outward,
            true,
        );
        let groups = fixture.groups(&[(outer, 0)]);
        for delta in [0.0, 150.0, 300.0, 0.0] {
            let updates = resize_axis_group_updates(&groups, delta_along(axis, delta)).unwrap();
            let projected = fixture.project_updates(&updates);
            let actual = rect_extent(projected.nodes[&inner].rect, axis);
            close(actual, if delta == 300.0 { 200.0 } else { 200.3 });
            if delta != 300.0 {
                assert!(updates.iter().all(|update| update.source().node() == outer));
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn translated_outward_resizes_keep_the_same_capacity_and_zero_displacement_intent(
        horizontal in any::<bool>(),
        negative in any::<bool>(),
        width in 600_u16..2000,
        fraction in 0_u8..8,
        weights in proptest::array::uniform3(1_u8..8),
        boundary in 0_usize..2,
    ) {
        let axis = if horizontal { Axis::Horizontal } else { Axis::Vertical };
        let mut builder = Workspace::builder();
        let cells = [1, 2, 3].map(|item| tabs(&mut builder, item));
        let split = builder.insert_node(Node::split(axis, cells, weights.map(f64::from)).unwrap());
        let workspace = finish(builder, split, None);
        let constraints = BTreeMap::from([
            (cells[0], constraints_along(axis, 100.1, 300.3)),
            (cells[1], constraints_along(axis, 80.3, 2000.0)),
            (cells[2], constraints_along(axis, 50.7, 2000.0)),
        ]);
        let width = f64::from(width) + f64::from(fraction) * 0.125;
        let origin = if negative { -1_000_000.3 } else { 1_000_000.3 };
        let fixture = |origin| Fixture::with_propagation(
            workspace.clone(),
            bounds_along(axis, origin, width),
            constraints.clone(),
            0.3,
            ResizePropagation::Outward,
        );
        let local = fixture(0.0);
        let translated = fixture(origin);
        let original_weights = match workspace.node(split).unwrap() {
            Node::Split { weights, .. } => weights,
            _ => unreachable!(),
        };
        let local_group = local.groups(&[(split, boundary)]);
        let translated_group = translated.groups(&[(split, boundary)]);
        for (local_limit, translated_limit) in [
            (local_group[0].common_delta.minimum, translated_group[0].common_delta.minimum),
            (local_group[0].common_delta.maximum, translated_group[0].common_delta.maximum),
        ] {
            prop_assert!((local_limit - translated_limit).abs() < 1.0e-7);
        }
        for delta in [0.0, 17.125, -17.125, 1.0e6, -1.0e6, 0.0] {
            let local_updates = resize_axis_group_updates(&local_group, delta_along(axis, delta)).unwrap();
            let translated_updates = resize_axis_group_updates(&translated_group, delta_along(axis, delta)).unwrap();
            if delta == 0.0 {
                for updates in [&local_updates, &translated_updates] {
                    let update = updates.iter().find(|update| update.source().node() == split).unwrap();
                    prop_assert_eq!(update.split_weights().unwrap(), original_weights.as_slice());
                }
            }
            let local_result = local.project_updates(&local_updates);
            let translated_result = translated.project_updates(&translated_updates);
            for cell in cells {
                let local_extent = rect_extent(local_result.nodes[&cell].rect, axis);
                let translated_extent = rect_extent(translated_result.nodes[&cell].rect, axis);
                prop_assert!((local_extent - translated_extent).abs() < 1.0e-7);
            }
        }
    }
}
