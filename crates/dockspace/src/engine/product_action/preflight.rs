//! Product availability uses the same staged candidate as action submission.

use super::*;
use crate::model::DockspaceActionAvailability;

impl DockEngine {
    pub(crate) fn prepare_product_action_if_available(
        &self,
        action: ProductAction,
    ) -> Result<Result<PreparedDockAction, DockspaceActionRejection>, EngineError> {
        let prepared = self.prepare_product_action(action);
        Ok(match self.action_availability(&prepared)? {
            DockspaceActionAvailability::Ready { .. } => Ok(prepared),
            DockspaceActionAvailability::Unavailable(reason) => Err(reason),
        })
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

    pub(super) fn stage_product_command(
        &self,
        input: InputSequence,
        command: &WorkspaceCommand,
        required_surfaces: &[SurfaceId],
        policy: &DockPolicySnapshot,
        authority: WorkspacePublicationAuthority,
    ) -> Result<(Option<StagedWorkspacePublication>, CommandApplication), EngineError> {
        let staged =
            self.stage_workspace_command_with_authority(input, policy, command, None, authority)?;
        if let Some(publication) = &staged.0 {
            if let Some(surface) =
                Self::first_missing_required_surface(&publication.workspace, required_surfaces)
            {
                return Ok((
                    None,
                    CommandApplication::Rejected(CommandError::RequiredSurfaceRemoved { surface }),
                ));
            }
            if matches!(authority, WorkspacePublicationAuthority::Ordinary)
                && let Err(error) = self.validate_application_command_identity_freshness(command)
            {
                return Ok((None, CommandApplication::Rejected(error)));
            }
        }
        Ok(staged)
    }

    pub(crate) fn action_availability(
        &self,
        prepared: &PreparedDockAction,
    ) -> Result<DockspaceActionAvailability, EngineError> {
        use DockspaceActionAvailability::{Ready, Unavailable};
        let (authority_domain, expected, action, required_surfaces) = prepared.parts();
        if authority_domain != self.authority_domain || expected != self.version {
            return Ok(Unavailable(DockspaceActionRejection::Conflict));
        }
        if self
            .product_presentation_source_root(action)
            .is_some_and(|root| self.root_has_pending_presentation_transition(root))
        {
            return Ok(Unavailable(DockspaceActionRejection::Conflict));
        }
        if let ProductAction::TearOffNative { source, placement } = action {
            return Ok(
                match self.validate_product_native_payload_tear_off_availability(
                    source,
                    *placement,
                    required_surfaces,
                    &self.policy,
                )? {
                    Ok(retired_surfaces) => Ready {
                        retired_surfaces,
                        requires_presentation: true,
                    },
                    Err(reason) => Unavailable(reason),
                },
            );
        }
        if let ProductAction::FloatRoot { root, surface, .. } = action
            && let Err(reason) = self.validate_product_root_float_availability(*root, *surface)
        {
            return Ok(Unavailable(reason));
        }
        let (plan, authority) = match self.compile_current_product_action(action) {
            Ok(plan) => plan,
            Err(reason) => return Ok(Unavailable(reason)),
        };
        let ProductActionPlan::Command { command, .. } = plan else {
            return Ok(
                match Self::first_missing_required_surface(&self.workspace, required_surfaces) {
                    Some(surface) => {
                        Unavailable(DockspaceActionRejection::SurfaceRequired { surface })
                    }
                    None => Ready {
                        retired_surfaces: Vec::new(),
                        requires_presentation: false,
                    },
                },
            );
        };
        if self
            .product_command_complete_root(&command)
            .is_some_and(|root| self.root_has_pending_presentation_transition(root))
        {
            return Ok(Unavailable(DockspaceActionRejection::Conflict));
        }
        let (publication, application) = self.stage_product_command(
            self.last_input,
            &command,
            required_surfaces,
            &self.policy,
            authority,
        )?;
        let CommandApplication::Applied { .. } = application else {
            let CommandApplication::Rejected(error) = application else {
                unreachable!()
            };
            return Ok(Unavailable(product_command_rejection(
                action.clone(),
                &error,
            )?));
        };
        let publication = publication.ok_or(EngineError::MissingCommandOutcome {
            input: self.last_input,
        })?;
        let requires_presentation =
            match self.validate_native_product_rehome(&command, &publication.workspace)? {
                Ok(requires) => requires,
                Err(reason) => return Ok(Unavailable(reason)),
            };
        Ok(Ready {
            retired_surfaces: self
                .workspace
                .surfaces()
                .map(|(surface, _)| surface)
                .filter(|surface| publication.workspace.surface(*surface).is_none())
                .collect(),
            requires_presentation,
        })
    }
}
