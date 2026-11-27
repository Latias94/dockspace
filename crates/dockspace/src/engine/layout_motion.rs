//! Rollbackable layout trajectories over already validated target plans.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::motion::{MotionFrame, RectMotion, Transition};
use crate::presentation::layout::TrackWeightOverride;
use crate::presentation::scene::{ContainedLayoutProjection, SurfacePlanScene, TabBarSceneId};

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct LayoutMotionState {
    pub(super) frame: Option<MotionFrame>,
    pub(super) surfaces: BTreeMap<SurfaceId, Arc<SurfaceMotionState>>,
    immediate_resizes: BTreeMap<SurfaceId, SurfaceMeasurementTicket>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SurfaceMotionState {
    pub(super) target: Arc<PresentationPlan>,
    coordinates: SurfaceCoordinateCapture,
    frame: MotionFrame,
    tracks: BTreeMap<ItemId, PaneMotion>,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneMotion {
    root: RootId,
    motion: RectMotion,
    rank: usize,
}

impl DockEngine {
    pub(super) fn settle_resize_motion(&mut self, surface: SurfaceId) {
        if let Some(requirements) = self.presentation_requirements().surface(surface) {
            let ticket = requirements.ticket();
            // Keep the target immediate across dropped outputs and preparation
            // retries, until this surface's requirements actually change.
            self.presentation_authority
                .motion
                .immediate_resizes
                .insert(surface, ticket);
        }
    }

    pub(crate) fn set_motion_frame(&mut self, frame: MotionFrame) {
        self.presentation_authority.motion.frame = Some(frame);
        let requirements = &self.presentation_authority.presentation_requirements;
        self.presentation_authority
            .motion
            .immediate_resizes
            .retain(|surface, ticket| {
                requirements
                    .surface(*surface)
                    .is_some_and(|current| current.ticket() == *ticket)
            });
        let epoch = self.version.epoch();
        let workspace = &self.workspace;
        self.presentation_authority
            .motion
            .surfaces
            .retain(|surface, state| {
                workspace.surface(*surface).is_some()
                    && state
                        .target
                        .measurement_ticket()
                        .is_some_and(|ticket| ticket.workspace_epoch() == epoch)
            });
    }

    pub(crate) fn surface_motion_is_active(&self, surface: SurfaceId) -> bool {
        self.presentation_authority
            .motion
            .surfaces
            .get(&surface)
            .is_some_and(|state| state.active)
    }

    pub(super) fn motion_needs_refresh(&self, surface: SurfaceId) -> bool {
        let motion = &self.presentation_authority.motion;
        let (Some(frame), Some(state)) = (motion.frame, motion.surfaces.get(&surface)) else {
            return false;
        };
        frame.settings != state.frame.settings
            || frame.preference != state.frame.preference
            || (state.active && frame.now != state.frame.now)
    }

    pub(super) fn cached_motion_target(
        &self,
        token: SurfaceContributionToken,
        measurements: &SurfaceMeasurements,
        contained: Option<ContainedLayoutProjection>,
        resize: &[TrackWeightOverride<'_>],
    ) -> Option<Arc<PresentationPlan>> {
        // Local tab-strip edits invalidate Ready without changing measurements.
        // Reuse only while the scene still retains that prepared geometry.
        self.scene().ready_candidate(token.surface())?;
        let target = &self
            .presentation_authority
            .motion
            .surfaces
            .get(&token.surface())?
            .target;
        let layout = target.layout_facts()?;
        (target.measurement_ticket() == Some(token.ticket())
            && target.popup()
                == self
                    .presentation_authority
                    .tab_strip_states
                    .popup_requirement()
            && target.popup()
                == self
                    .presentation_authority
                    .presentation_requirements
                    .popup()
            && layout.config() == &self.presentation_authority.presentation_config
            && layout.measurements().as_ref() == measurements
            && target.contained_projection() == contained
            && target.resize_projection().len() == resize.len()
            && target
                .resize_projection()
                .iter()
                .zip(resize)
                .all(|(old, current)| {
                    old.source() == current.source() && old.weights() == current.weights()
                }))
        .then(|| Arc::clone(target))
    }

    pub(super) fn prepare_layout_motion(
        &self,
        mut contribution: PreparedSurfaceContribution,
    ) -> Result<PreparedSurfaceContribution, SurfaceContributionPrepareError> {
        let surface = contribution.surface();
        let Some(frame) = self.presentation_authority.motion.frame else {
            return Ok(contribution);
        };
        let PreparedSurfaceContributionState::Ready { plan: target } = &contribution.state else {
            return Ok(contribution);
        };
        // Prospective native content must retain one exact first-live candidate.
        if self.scene().native_first_live_projection(surface).is_some() {
            return Ok(contribution);
        }
        let old = self.presentation_authority.motion.surfaces.get(&surface);
        let coordinates = contribution.token.coordinates;
        let prior = self
            .scene()
            .surface(surface)
            .and_then(SurfaceScene::confirmed_presentation)
            .filter(|scene| scene.coordinate_capture() == coordinates)
            .filter(|scene| scene.plan().bounds() == target.bounds());
        let has_composition = |plan: &PresentationPlan| {
            plan.layout_facts().is_some_and(|facts| {
                facts
                    .roots()
                    .any(|root| root.projection().nodes.topology_has_composition())
            })
        };
        let transition = if self
            .presentation_authority
            .motion
            .immediate_resizes
            .get(&surface)
            == Some(&contribution.token.ticket())
            || self.interaction.active_surface() == Some(surface)
            || target.popup().visual_for_surface(surface).is_some()
            || target.contained_projection().is_some()
            || !target.resize_projection().is_empty()
            || has_composition(target)
            || prior.is_some_and(|scene| {
                scene
                    .plan()
                    .contained_projection()
                    .is_some_and(|projection| {
                        target
                            .contained_records()
                            .iter()
                            .any(|contained| contained.root() == projection.root)
                    })
                    || !scene.plan().resize_projection().is_empty()
                    || has_composition(scene.plan())
            }) {
            Transition::Instant
        } else {
            frame.settings.layout_transition(frame.preference)
        };
        if transition == Transition::Instant {
            return Ok(contribution);
        }
        if let Some(old) = old
            && Arc::ptr_eq(target, &old.target)
            && !old.active
            && frame.settings == old.frame.settings
            && frame.preference == old.frame.preference
        {
            contribution.next_motion = Some(Arc::clone(old));
            return Ok(contribution);
        }
        let prior_snapshot =
            std::cell::LazyCell::new(|| prior.map(presented_snapshot).unwrap_or_default());
        let target_main_root = self
            .workspace
            .surface(surface)
            .and_then(|presentation| presentation.main_root);
        let target_tab_bars: BTreeMap<TabBarSceneId, (ItemId, LogicalRect)> = target
            .pane_records()
            .iter()
            .filter_map(|pane| {
                pane.containing_tab_bar()
                    .map(|bar| (bar, (pane.id().item, pane.layout_bounds())))
            })
            .collect();
        let continuing = old.filter(|state| {
            state.coordinates == coordinates
                && state.target.bounds() == target.bounds()
                && state
                    .target
                    .measurement_ticket()
                    .is_some_and(|ticket| ticket.workspace_epoch() == self.version.epoch())
        });
        let mut tracks = BTreeMap::new();
        let mut rectangles = BTreeMap::new();
        let mut ranks = BTreeMap::new();
        let mut active = false;
        let mut geometry_changed = false;
        let count = target.pane_records().len();
        for (ordinal, pane) in target.pane_records().iter().enumerate() {
            let item = pane.id().item;
            let layout_bounds = pane.layout_bounds();
            let mut motion = continuing
                .and_then(|state| state.tracks.get(&item))
                .filter(|track| track.root == pane.id().root)
                .filter(|_| {
                    continuing
                        .is_some_and(|state| Arc::ptr_eq(target, &state.target) || state.active)
                })
                .map(|track| track.motion)
                .or_else(|| {
                    initial_pane_rect(pane, &prior_snapshot, &target_tab_bars, target_main_root)
                        .map(RectMotion::new)
                })
                .unwrap_or_else(|| RectMotion::new(layout_bounds));
            motion.retarget(layout_bounds, transition, frame.now);
            let sample = motion.sample(frame.now);
            let moving = sample.rect() != layout_bounds;
            active |= sample.is_active();
            geometry_changed |= moving;
            rectangles.insert(pane.id(), sample.rect());
            // Preserve target roster order; move each complete travelling pane
            // above stationary peers rather than ranking controls separately.
            let rank = continuing
                .filter(|state| Arc::ptr_eq(target, &state.target))
                .and_then(|state| state.tracks.get(&item))
                .map_or_else(
                    || ordinal.saturating_add(if sample.is_active() { count } else { 0 }),
                    |track| track.rank,
                );
            ranks.insert(pane.id(), rank);
            tracks.insert(
                item,
                PaneMotion {
                    root: pane.id().root,
                    motion,
                    rank,
                },
            );
        }
        let state = Arc::new(SurfaceMotionState {
            target: Arc::clone(target),
            coordinates,
            frame,
            tracks,
            active,
        });
        if geometry_changed {
            let sampled = crate::presentation::compiler::materialize_presentation_sample(
                &self.workspace,
                &self.policy,
                &self.presentation_authority.presentation_requirements,
                &self.presentation_authority.tab_strip_states,
                target,
                &rectangles,
                &ranks,
            )
            .map_err(SurfaceContributionPrepareError::Compilation)?;
            contribution.state = PreparedSurfaceContributionState::Ready {
                plan: Arc::new(sampled),
            };
        }
        contribution.next_motion = Some(state);
        Ok(contribution)
    }
}

#[derive(Clone, Copy)]
struct PriorPane {
    root: RootId,
    tab_bar: Option<TabBarSceneId>,
    bounds: LogicalRect,
    clip: LogicalRect,
}

#[derive(Default)]
struct PriorMotionSnapshot {
    panes: BTreeMap<ItemId, PriorPane>,
    tab_bars: BTreeMap<TabBarSceneId, LogicalRect>,
}

fn presented_snapshot(scene: &SurfacePlanScene) -> PriorMotionSnapshot {
    let mut snapshot = PriorMotionSnapshot::default();
    for pane in scene.plan().pane_records() {
        let bounds = pane.layout_bounds();
        let tab_bar = pane.containing_tab_bar();
        snapshot.panes.insert(
            pane.id().item,
            PriorPane {
                root: pane.id().root,
                tab_bar,
                bounds,
                clip: pane.clip(),
            },
        );
        if let Some(bar) = tab_bar {
            snapshot.tab_bars.insert(bar, bounds);
        }
    }
    snapshot
}

fn initial_pane_rect(
    pane: &crate::presentation::scene::PaneRecord,
    prior: &PriorMotionSnapshot,
    target_tab_bars: &BTreeMap<TabBarSceneId, (ItemId, LogicalRect)>,
    target_main_root: Option<RootId>,
) -> Option<LogicalRect> {
    let item = pane.id().item;
    let root = pane.id().root;
    let layout_bounds = pane.layout_bounds();
    let target_bar = pane.containing_tab_bar();

    if let Some(&prior_pane) = prior.panes.get(&item) {
        if prior_pane.root == root {
            if let Some(prior_bar) = prior_pane.tab_bar
                && target_bar != Some(prior_bar)
                && let Some(&(survivor_item, survivor_target)) = target_tab_bars.get(&prior_bar)
                && survivor_item != item
                && survivor_target != prior_pane.bounds
                && let Some(seam_start) =
                    split_seam_start(prior_pane.bounds, survivor_target, layout_bounds)
            {
                return Some(seam_start);
            }
            return Some(prior_pane.bounds);
        }
        if Some(root) == target_main_root
            && !target_bar.is_some_and(|bar| prior.tab_bars.contains_key(&bar))
            && prior_pane.clip.width() > 0.0
            && prior_pane.clip.height() > 0.0
        {
            return Some(prior_pane.clip);
        }
    }

    // A newly revealed tab inside an existing tab group inherits that group's
    // prior bounds so surviving siblings resize smoothly instead of snapping.
    if let Some(bar) = target_bar
        && let Some(&prior_bar_bounds) = prior.tab_bars.get(&bar)
    {
        return Some(prior_bar_bounds);
    }

    // A newly visible pane split horizontally off an existing tab group opens
    // from the shared outer edge so neither pane overlaps the other's tab bar.
    for (&bar, &prior_bar_bounds) in &prior.tab_bars {
        if bar.root != root || Some(bar) == target_bar {
            continue;
        }
        let Some(&(_, survivor_target)) = target_tab_bars.get(&bar) else {
            continue;
        };
        if survivor_target == prior_bar_bounds {
            continue;
        }
        if let Some(seam_start) =
            horizontal_split_seam_start(prior_bar_bounds, survivor_target, layout_bounds)
        {
            return Some(seam_start);
        }
    }

    None
}

fn horizontal_split_seam_start(
    prior: LogicalRect,
    survivor: LogicalRect,
    split: LogicalRect,
) -> Option<LogicalRect> {
    const EPSILON: f64 = 1.0;
    let same_vertical = (split.y() - prior.y()).abs() <= EPSILON
        && (split.max().y() - prior.max().y()).abs() <= EPSILON
        && (survivor.y() - prior.y()).abs() <= EPSILON
        && (survivor.max().y() - prior.max().y()).abs() <= EPSILON;
    if !same_vertical {
        return None;
    }
    let seam_width = split.width().min(1.0);
    if (survivor.x() - prior.x()).abs() <= EPSILON
        && (split.max().x() - prior.max().x()).abs() <= EPSILON
        && survivor.max().x() <= split.x() + EPSILON
    {
        return LogicalRect::new(
            prior.max().x() - seam_width,
            split.y(),
            seam_width,
            split.height(),
        )
        .ok();
    }
    if (split.x() - prior.x()).abs() <= EPSILON
        && (survivor.max().x() - prior.max().x()).abs() <= EPSILON
        && split.max().x() <= survivor.x() + EPSILON
    {
        return LogicalRect::new(prior.x(), split.y(), seam_width, split.height()).ok();
    }
    None
}

fn split_seam_start(
    prior: LogicalRect,
    survivor: LogicalRect,
    split: LogicalRect,
) -> Option<LogicalRect> {
    if let Some(horizontal) = horizontal_split_seam_start(prior, survivor, split) {
        return Some(horizontal);
    }
    const EPSILON: f64 = 1.0;
    let same_horizontal = (split.x() - prior.x()).abs() <= EPSILON
        && (split.max().x() - prior.max().x()).abs() <= EPSILON
        && (survivor.x() - prior.x()).abs() <= EPSILON
        && (survivor.max().x() - prior.max().x()).abs() <= EPSILON;
    if same_horizontal
        && ((survivor.y() <= split.y() && survivor.max().y() <= split.y() + EPSILON)
            || (split.y() <= survivor.y() && split.max().y() <= survivor.y() + EPSILON))
        && split.y() >= prior.y() - EPSILON
        && split.max().y() <= prior.max().y() + EPSILON
    {
        return Some(split);
    }
    None
}

impl CoreHostFrame {
    pub(crate) fn set_motion_frame(&mut self, frame: MotionFrame) {
        self.candidate.set_motion_frame(frame);
    }
}

impl HostFrameView<'_> {
    pub(crate) fn motion_frame(self) -> Option<MotionFrame> {
        self.engine.presentation_authority.motion.frame
    }
}

#[cfg(test)]
mod tests;
