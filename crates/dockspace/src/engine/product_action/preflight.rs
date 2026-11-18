//! Read-only product-action availability and candidate surface retention.

use super::*;

impl DockEngine {
    pub(crate) fn prepare_product_action_if_available(
        &self,
        action: ProductAction,
    ) -> Result<Result<PreparedDockAction, DockspaceActionRejection>, EngineError> {
        if self
            .product_presentation_source_root(&action)
            .is_some_and(|root| self.root_has_pending_presentation_transition(root))
        {
            return Ok(Err(DockspaceActionRejection::Conflict));
        }
        if let ProductAction::TearOffNative { source, placement } = &action {
            return self
                .validate_product_native_payload_tear_off_availability(
                    source,
                    *placement,
                    &self.policy,
                )
                .map(|availability| availability.map(|()| self.prepare_product_action(action)));
        }
        if let ProductAction::FloatRoot { root, surface, .. } = action
            && let Err(reason) = self.validate_product_root_float_availability(root, surface)
        {
            return Ok(Err(reason));
        }
        let plan = self.compile_current_product_action(&action);
        let plan = match plan {
            Ok((plan, _)) => plan,
            Err(reason) => return Ok(Err(reason)),
        };
        if let ProductActionPlan::Command { command, .. } = &plan
            && self
                .product_command_complete_root(&command)
                .is_some_and(|root| self.root_has_pending_presentation_transition(root))
        {
            return Ok(Err(DockspaceActionRejection::Conflict));
        }
        if let ProductActionPlan::Command { command, .. } = &plan
            && let Err(error) = crate::graph::operation::authorize_workspace_command(
                &self.workspace,
                &self.policy,
                &command,
            )
        {
            return Ok(Err(product_command_rejection(action, &error)?));
        }
        Ok(Ok(self.prepare_product_action(action)))
    }

    fn compile_current_product_action(
        &self,
        action: &ProductAction,
    ) -> Result<(ProductActionPlan, WorkspacePublicationAuthority), DockspaceActionRejection> {
        if let ProductAction::DockBackRoot { root } = action
            && let Some(surface) = self.native_main_dock_back_target(*root)?
        {
            return self.compile_product_native_main_dock_back(*root, surface);
        }
        self.compile_product_action(action.clone())
            .map(|plan| (plan, WorkspacePublicationAuthority::Ordinary))
    }

    pub(crate) fn action_removes_surface(
        &self,
        prepared: &PreparedDockAction,
        surface: SurfaceId,
    ) -> Result<bool, EngineError> {
        let (authority_domain, expected, action) = prepared.parts();
        if authority_domain != self.authority_domain
            || expected != self.version
            || self.workspace.surface(surface).is_none()
        {
            return Ok(false);
        }
        // Native creation owns a separate first-live/recovery transaction. It
        // keeps its original source until the exact child presentation commits.
        if matches!(action, ProductAction::TearOffNative { .. }) {
            return Ok(false);
        }
        if self
            .product_presentation_source_root(action)
            .is_some_and(|root| self.root_has_pending_presentation_transition(root))
        {
            return Ok(false);
        }
        let plan = self.compile_current_product_action(action);
        let Ok((ProductActionPlan::Command { command, .. }, publication_authority)) = plan else {
            return Ok(false);
        };
        if self
            .product_command_complete_root(&command)
            .is_some_and(|root| self.root_has_pending_presentation_transition(root))
            || self
                .validate_application_command_identity_freshness(&command)
                .is_err()
        {
            return Ok(false);
        }
        let candidate = match self
            .workspace
            .prepare_transaction(&self.policy, std::slice::from_ref(&command))
        {
            Ok(prepared) => prepared.into_snapshot_parts().0,
            Err(TransactionError::Command { index: 0, source })
                if source.is_expected_rejection() =>
            {
                return Ok(false);
            }
            Err(source) => return Err(EngineError::WorkspaceCommandPreflight { source }),
        };
        if self
            .first_pending_presentation_source_mismatch(&candidate, publication_authority)
            .is_some()
            || self
                .first_workspace_publication_mismatch(&candidate, None, None)
                .is_some()
        {
            return Ok(false);
        }
        match self.stage_workspace_publication_with_authority(
            candidate,
            &self.policy,
            publication_authority,
        ) {
            Ok(publication) => Ok(publication.workspace.surface(surface).is_none()),
            Err(source) if source.is_expected_rejection() => Ok(false),
            Err(source) => Err(EngineError::WorkspaceCommandPreflight {
                source: TransactionError::Command { index: 0, source },
            }),
        }
    }
}
