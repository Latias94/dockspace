//! Floating-point cancellation does not create a track-capacity contact.

use super::*;
use crate::graph::{RootRecord, SurfacePresentation};
use crate::ids::{ItemId, SurfaceId};

const FAR_EXTENT: f64 = 99.999_999_999_999_94;
const NEAR_EXTENT: f64 = 355.5;
const OFFERED_EXTENT: f64 = 775.0;

fn tracks() -> FrozenTracks {
    let root = RootId::new(1);
    let mut builder = Workspace::builder();
    let children = [1, 2].map(|item| builder.insert_node(Node::pane(ItemId::new(item))));
    let owner = builder
        .insert_node(Node::split(Axis::Horizontal, children, [FAR_EXTENT, NEAR_EXTENT]).unwrap());
    builder.set_root(root, RootRecord::new(owner));
    builder.set_surface(SurfaceId::new(1), SurfacePresentation::with_main(root));
    let workspace = builder.build().unwrap();
    let Some(Node::Split { weights, .. }) = workspace.node(owner) else {
        panic!("the checked fixture must retain its two-track owner");
    };
    FrozenTracks {
        source: workspace.capture_node_source(root, owner).unwrap(),
        frame_extent: FAR_EXTENT + NEAR_EXTENT,
        children: children
            .into_iter()
            .enumerate()
            .map(|(index, node)| (node, index))
            .collect(),
        tracks: [FAR_EXTENT, NEAR_EXTENT]
            .map(|extent| Track {
                visible: true,
                minimum: 1.0,
                maximum: 1000.0,
                maximum_follows_frame: false,
                extent,
                gap: 0.0,
            })
            .to_vec(),
        weights: weights.clone(),
        central: None,
        grid: None,
    }
}

#[test]
fn an_unclipped_adjacent_track_retains_the_offered_frame_derivative() {
    let tracks = tracks();
    let remaining = OFFERED_EXTENT - (FAR_EXTENT + NEAR_EXTENT);
    assert_eq!(remaining, 319.500_000_000_000_06);
    assert_eq!(NEAR_EXTENT + remaining, 675.0);
    assert_ne!((NEAR_EXTENT + remaining) - NEAR_EXTENT, remaining);

    // The offered frame follows an ancestor boundary with unit derivative.
    // Its nearest track has spare capacity, so it absorbs the entire change.
    let (extents, starts) = tracks
        .allocate(
            Scalar::fixed(0.0),
            Scalar {
                value: OFFERED_EXTENT,
                slope: 1.0,
            },
            0.0,
            &[],
            remaining,
            ResizePropagation::Adjacent,
        )
        .unwrap();
    assert_eq!(extents, [FAR_EXTENT, 675.0]);
    assert!(extents[1] < tracks.tracks[1].maximum);
    assert_eq!(starts[0].slope, 0.0);
    assert_eq!(starts[1].slope, 0.0);
    assert_eq!(starts[2].value, OFFERED_EXTENT);
    assert_eq!(starts[2].slope - starts[1].slope, 1.0);
}

#[test]
fn outward_allocation_does_not_transfer_a_cancellation_residue_to_the_far_track() {
    let tracks = tracks();
    let (extents, starts) = tracks
        .allocate(
            Scalar::fixed(0.0),
            Scalar {
                value: OFFERED_EXTENT,
                slope: 1.0,
            },
            0.0,
            &[],
            OFFERED_EXTENT - (FAR_EXTENT + NEAR_EXTENT),
            ResizePropagation::Outward,
        )
        .unwrap();
    // Subtracting the near track's old extent leaves a numeric residue even
    // without clipping. It is not capacity overflow into the farther track.
    assert_eq!(extents[0].to_bits(), FAR_EXTENT.to_bits());
    assert_eq!(extents[1], 675.0);
    assert_eq!(starts[1].value.to_bits(), FAR_EXTENT.to_bits());
    assert_eq!(starts[1].slope, 0.0);
    assert_eq!(starts[2].value, OFFERED_EXTENT);
    assert_eq!(starts[2].slope, 1.0);
}

#[test]
fn an_admitted_rounded_minimum_retains_its_one_sided_derivative() {
    let mut tracks = tracks();
    tracks.tracks[1].minimum = 145.0;
    let offered = 244.999_999_999_999_9;
    let delta = offered - (FAR_EXTENT + NEAR_EXTENT);
    assert_eq!(NEAR_EXTENT + delta, 144.999_999_999_999_94);
    let (extents, starts) = tracks
        .allocate(
            Scalar::fixed(0.0),
            Scalar {
                value: offered,
                slope: 1.0,
            },
            0.0,
            &[],
            delta,
            ResizePropagation::Adjacent,
        )
        .unwrap();
    assert_eq!(extents[0].to_bits(), FAR_EXTENT.to_bits());
    assert_eq!(extents[1], 145.0, "the hard minimum is still enforced");
    assert_eq!(starts[1].slope, 0.0);
    assert_eq!(starts[2].slope, 1.0);
}

#[test]
fn a_final_outward_track_retains_the_inward_derivative_at_a_rounded_minimum() {
    let mut tracks = tracks();
    tracks.tracks[1].minimum = 145.0;
    let delta = 210.500_000_000_000_06;
    let (extents, starts) = tracks
        .allocate(
            Scalar::fixed(0.0),
            Scalar::fixed(FAR_EXTENT + NEAR_EXTENT),
            0.0,
            &[0],
            delta,
            ResizePropagation::Outward,
        )
        .unwrap();
    assert_eq!(extents[0], FAR_EXTENT + delta);
    assert_eq!(extents[1], 145.0, "the hard minimum is still enforced");
    assert_eq!(starts[1].slope, 1.0);
    assert_eq!(starts[2].slope - starts[1].slope, -1.0);
}

#[test]
fn outward_allocation_preserves_a_representable_spill_past_an_intermediate_cap() {
    let mut tracks = tracks();
    for track in &mut tracks.tracks {
        track.extent = 1.0;
        track.minimum = 0.25;
    }
    tracks.frame_extent = 2.0;
    tracks.tracks[1].maximum = 1.0;
    let spill = 2.0_f64.powi(-50);
    let offered = 2.0 + spill;
    assert!(spill < tolerance(1.0, 1.0 + spill));
    let (extents, starts) = tracks
        .allocate(
            Scalar::fixed(0.0),
            Scalar {
                value: offered,
                slope: 1.0,
            },
            0.0,
            &[],
            spill,
            ResizePropagation::Outward,
        )
        .unwrap();
    assert_eq!(extents, [1.0 + spill, 1.0]);
    assert_eq!(starts[1].value, 1.0 + spill);
    assert_eq!(starts[1].slope, 1.0);
    assert_eq!(starts[2].value, offered);
    assert_eq!(starts[2].slope, 1.0);
}
