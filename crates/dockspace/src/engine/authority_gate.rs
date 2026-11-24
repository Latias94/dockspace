//! Presentation authority for acquiring or delivering a drag target.

use crate::engine::*;
use crate::input::receiver::NativeWindowHitReceipt;
use crate::interaction::ActiveDrag;

impl DockEngine {
    pub(in crate::engine) fn drag_source_presentation_allows_targeting(
        &self,
        drag: &ActiveDrag,
    ) -> bool {
        if !matches!(drag.origin, FrozenDragOrigin::Workspace) {
            return true;
        }
        let surface = drag.source_surface;
        let matches_coordinates = |coordinates| {
            Self::coordinate_capture_matches_current(
                coordinates,
                self.viewport.viewport(surface),
                self.viewport.surface_coordinate_authority(surface),
            )
        };
        let captured_coordinates = drag.continuation.as_ref().and_then(|continuation| {
            match (&continuation.session, &continuation.draft.source) {
                (
                    SceneGestureSession::Drag(session),
                    SceneGestureContinuationSource::Drag {
                        source_surface,
                        coordinates,
                        ..
                    },
                ) if *session == drag.session && *source_surface == surface => Some(*coordinates),
                _ => None,
            }
        });
        captured_coordinates.is_some_and(matches_coordinates)
            || self
                .interaction_projection(surface)
                .is_some_and(|projection| {
                    matches_coordinates(projection.output().coordinate_capture())
                })
    }

    pub(in crate::engine) fn drag_exit_without_source_presentation(
        drag: &ActiveDrag,
        position: ManipulationPositionKey,
        desktop_route: Option<DesktopRouteValidation>,
        native_hit: Option<NativeWindowHitReceipt>,
    ) -> Option<PreviewEvaluation> {
        // The current receiver can revoke a latched target without source
        // geometry. Acquiring its replacement still needs it.
        let outside_all = matches!(
            desktop_route,
            Some(DesktopRouteValidation::Known(
                ValidatedDesktopRoute::NoWindow { .. }
            ))
        );
        let blocked = matches!(
            desktop_route,
            Some(DesktopRouteValidation::Known(
                ValidatedDesktopRoute::Foreign { .. }
            ))
        ) || matches!(native_hit, Some(NativeWindowHitReceipt::Blocked));
        let left_target = matches!(native_hit, Some(NativeWindowHitReceipt::WindowBackground))
            || position.surface().is_some_and(|surface| {
                drag.target_acquisition()
                    .is_some_and(|target| target.scene().surface() != surface)
            });
        let status = if blocked {
            PreviewResolutionStatus::OpaqueBlocker
        } else if outside_all || left_target {
            PreviewResolutionStatus::Unavailable
        } else {
            return None;
        };
        let decision = PreviewDecision::Clear(status);
        Some(if outside_all {
            PreviewEvaluation::outside_all(decision)
        } else {
            PreviewEvaluation::without_affordance(decision)
        })
    }

    pub(in crate::engine) fn frozen_drag_origin_is_current(
        &self,
        origin: FrozenDragOrigin,
    ) -> bool {
        let FrozenDragOrigin::Contained(origin) = origin else {
            return true;
        };
        self.workspace.presentation_for_root(origin.root)
            == Some(crate::graph::workspace::RootPresentationOwner::Contained {
                surface: origin.surface,
                floating: origin.floating,
            })
            && self
                .workspace
                .contained_floating(origin.floating)
                .is_some_and(|record| {
                    record.root == origin.root && record.rect == origin.source_rect
                })
    }

    pub(in crate::engine) fn drag_preview_authority_is_current(&self, drag: &ActiveDrag) -> bool {
        drag.preview().is_some_and(|preview| {
            self.drag_target_presentation_is_current(
                drag,
                preview.public().token().scene(),
                preview.proof(),
                preview.target_layout_basis(),
            )
        })
    }

    fn drag_target_presentation_is_current(
        &self,
        drag: &ActiveDrag,
        scene: SurfaceSceneStamp,
        proof: &PreviewProof,
        target_layout_basis: Option<SurfaceSceneStamp>,
    ) -> bool {
        if matches!(proof, PreviewProof::Native { .. }) {
            return true;
        }
        if let PreviewProof::Contained { proposal, .. } = proof
            && self.current_contained_projection_is_owned(proposal.surface())
            && self
                .scene()
                .ready_candidate(proposal.surface())
                .is_some_and(|candidate| {
                    candidate.plan().bounds() == proposal.placement().surface_bounds()
                })
        {
            // Source movement remains valid when beginning the drag changes
            // destination guides. It carries no acquired docking target.
            return true;
        }
        if self
            .interaction_projection(scene.surface())
            .is_some_and(|projection| {
                projection.plan_stamp() == scene
                    && (target_layout_basis.is_none()
                        || (target_layout_basis == Some(scene)
                            && projection.plan().contained_projection().is_none()))
            })
        {
            return true;
        }
        self.presentation_authority
            .scene
            .ready_candidate(scene.surface())
            .is_some_and(|candidate| {
                candidate.stamp() == scene
                    || (candidate
                        .plan()
                        .contained_projection()
                        .is_some_and(|projection| {
                            projection.owner
                                == crate::presentation::scene::ContainedLayoutOwner::Drag(
                                    drag.session,
                                )
                                && projection.target_basis.is_some()
                        })
                        && target_layout_basis
                            == self.drag_target_layout_basis(scene.surface(), drag.session))
            })
    }

    pub(in crate::engine) fn drag_release_cancellation(
        &self,
        drag: &ActiveDrag,
        evaluation: &PreviewEvaluation,
    ) -> Option<InteractionCancelReason> {
        match &evaluation.decision {
            PreviewDecision::Publish { scene, proof, .. }
                if !self.drag_target_presentation_is_current(
                    drag,
                    *scene,
                    proof,
                    evaluation.target_layout_basis,
                ) =>
            {
                Some(InteractionCancelReason::UnknownTargetAuthority)
            }
            PreviewDecision::Clear(PreviewResolutionStatus::UnknownAuthority) => {
                Some(InteractionCancelReason::UnknownTargetAuthority)
            }
            PreviewDecision::Clear(PreviewResolutionStatus::OpaqueBlocker) => {
                Some(InteractionCancelReason::OpaquePointerBlocker)
            }
            PreviewDecision::Cancel(reason) => Some(*reason),
            _ => None,
        }
    }
}
