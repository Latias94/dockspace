//! Scoped repaint changes for scene authority and independently owned gesture visuals.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
struct DragSourceVisual {
    surface: SurfaceId,
    pointer_surface: Option<SurfaceId>,
    session: crate::interaction::DragSessionId,
    pointer: Option<crate::geometry::LogicalPoint>,
}

impl DragSourceVisual {
    fn is_on(self, surface: SurfaceId) -> bool {
        self.surface == surface || self.pointer_surface == Some(surface)
    }
}

/// Borrows the visual state without cloning payloads, guides, or preview tab projections.
struct TransientVisuals<'engine> {
    source: Option<DragSourceVisual>,
    guides: Option<&'engine DropAffordance>,
    preview: Option<&'engine crate::interaction::InteractionPreview>,
    contained: Option<&'engine crate::interaction::ContainedTransformPreview>,
}

impl<'engine> TransientVisuals<'engine> {
    fn capture(engine: &'engine DockEngine) -> Self {
        let drag = match engine.interaction.status() {
            InteractionStatus::Dragging { session } => engine.interaction.active_drag(session).ok(),
            _ => None,
        };
        let source = drag.map(|drag| DragSourceVisual {
            surface: drag.source_surface,
            // A pointer ghost can remain on a blocked destination even when it
            // has neither a docking preview nor any guide cluster.
            pointer_surface: drag
                .last_advanced_position()
                .and_then(ManipulationPositionKey::surface),
            session: drag.session,
            pointer: drag.current_pointer(),
        });
        // A committed release may retain its exact guides until presentation
        // settles, even though the active gesture has already become idle.
        let guides = drag
            .or_else(|| {
                engine
                    .pending_mutation
                    .as_ref()
                    .and_then(PendingMutation::drag_release)
                    .map(|pending| &pending.drag)
            })
            .and_then(crate::interaction::ActiveDrag::drop_affordance);
        Self {
            source,
            guides,
            preview: engine.presentation_preview(),
            contained: engine.presentation_contained_transform_preview(),
        }
    }

    fn changed_on(&self, after: &Self, surface: SurfaceId) -> bool {
        self.source.filter(|source| source.is_on(surface))
            != after.source.filter(|source| source.is_on(surface))
            || self.guides.filter(|guides| guides.surface() == surface)
                != after.guides.filter(|guides| guides.surface() == surface)
            || self
                .preview
                .filter(|preview| preview.visual().surface() == surface)
                != after
                    .preview
                    .filter(|preview| preview.visual().surface() == surface)
            || self
                .contained
                .filter(|preview| preview.surface() == surface)
                != after
                    .contained
                    .filter(|preview| preview.surface() == surface)
    }
}

impl DockEngine {
    pub(super) fn surface_scene_deltas(before: &Self, after: &Self) -> Vec<SurfaceSceneDelta> {
        let before_visuals = TransientVisuals::capture(before);
        let after_visuals = TransientVisuals::capture(after);
        let before = &before.presentation_authority.scene;
        let after = &after.presentation_authority.scene;
        let surfaces = before
            .surfaces()
            .map(|(surface, _)| *surface)
            .chain(after.surfaces().map(|(surface, _)| *surface))
            .collect::<BTreeSet<_>>();
        surfaces
            .into_iter()
            .filter_map(|surface| {
                let before_authority = before.interaction_authority(surface);
                let after_authority = after.interaction_authority(surface);
                let before = before
                    .surface(surface)
                    .map(|scene| Self::surface_scene_authority(scene, before_authority));
                let after = after
                    .surface(surface)
                    .map(|scene| Self::surface_scene_authority(scene, after_authority));
                let transient_visuals_changed = before_visuals.changed_on(&after_visuals, surface);
                (before != after || transient_visuals_changed).then(|| {
                    SurfaceSceneDelta::new(surface, before, after, transient_visuals_changed)
                })
            })
            .collect()
    }

    fn surface_scene_authority(
        scene: &SurfaceScene,
        interaction: Option<PresentedSurfaceAuthority>,
    ) -> (
        SurfaceSceneStamp,
        SurfaceSceneStateKind,
        Option<PresentedSurfaceAuthority>,
    ) {
        let kind = match scene {
            SurfaceScene::Ready(_) => SurfaceSceneStateKind::Ready,
            SurfaceScene::Stale(_) => SurfaceSceneStateKind::Stale,
            SurfaceScene::Bootstrap(_) => SurfaceSceneStateKind::Bootstrap,
        };
        (scene.stamp(), kind, interaction)
    }
}
