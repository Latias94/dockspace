//! Host-owned logical surface lifetime, shared by every publication path.

use crate::engine::*;

impl DockEngine {
    pub(crate) fn retain_host_surface(&mut self, surface: SurfaceId) -> Result<(), EngineError> {
        if self.workspace.surface(surface).is_none() {
            return Err(EngineError::HostSurfaceUnavailable { surface });
        }
        if !self.host_surfaces.contains(&surface) {
            self.advance_runtime_retention_revision()?;
            self.host_surfaces.insert(surface);
        }
        Ok(())
    }

    pub(super) fn retained_host_surfaces_for_publication(
        &self,
        candidate: &Workspace,
        authority: WorkspacePublicationAuthority,
    ) -> Result<BTreeSet<SurfaceId>, CommandError> {
        let mut retained = self.host_surfaces.clone();
        for &surface in &self.host_surfaces {
            if candidate.surface(surface).is_some() {
                continue;
            }
            let authorized_close = match authority {
                WorkspacePublicationAuthority::WindowRecoveryCommit { window } => self
                    .viewport
                    .viewport(surface)
                    .is_some_and(|record| record.binding().window() == window),
                _ => false,
            };
            if !authorized_close {
                return Err(CommandError::RequiredSurfaceRemoved { surface });
            }
            retained.remove(&surface);
        }
        Ok(retained)
    }

    pub(super) fn retained_host_surfaces_for_replacement(
        &self,
        candidate: &Workspace,
        continuations: &[crate::platform::frame::PreparedNativeRootContinuation],
    ) -> Result<BTreeSet<SurfaceId>, EngineError> {
        self.host_surfaces
            .iter()
            .map(|&surface| {
                let target = continuations
                    .iter()
                    .find(|continuation| continuation.predecessor().surface() == surface)
                    .map_or(surface, |continuation| continuation.target());
                candidate
                    .surface(target)
                    .map(|_| target)
                    .ok_or(EngineError::HostSurfaceUnavailable { surface })
            })
            .collect()
    }
}
