//! Native service progress independent from the Root paint callback.

use super::*;

impl<P: PaneView> NativeRuntimeState<P> {
    fn application_service_ready(&self) -> bool {
        self.shutdown.is_none()
            && self.coordinator.dispatch_bridge().accepts_new_work()
            && self.application_actions.has_queued()
            && !self.pass_actions.has_deferred_surface_actions()
            && !self.root_placements.has_pending_input_or_declaration()
    }

    pub(crate) fn service_native_events(
        &mut self,
        context: &egui::Context,
    ) -> Result<NativeHostWake, NativeRuntimeError> {
        self.abandon_unfinalized_output_passes(context)?;
        if self.owned_update.is_some() {
            self.pending_service = true;
            return Ok(NativeHostWake::ServiceAfterCurrent);
        }
        if self.shutdown.is_some() {
            let mut budget = self.coordinator.dispatch_bridge().service_budget();
            let advance = self
                .coordinator
                .advance_shutdown_boundary_with_close_policy(
                    context,
                    self.close_policy,
                    self.root_surface,
                    &mut budget,
                );
            let (progress, errors) = self.apply_shutdown_advance(context, advance);
            for error in errors {
                self.record_cleanup_error(error);
            }
            return Ok(
                if !self.is_shutdown_drained() && (progress || budget.is_exhausted()) {
                    NativeHostWake::ServiceAfterCurrent
                } else {
                    NativeHostWake::Wait
                },
            );
        }
        let mut budget = self.coordinator.dispatch_bridge().service_budget();
        Ok(
            if self.service_pending_prefixes(context, None, &mut budget)? {
                NativeHostWake::Wait
            } else {
                NativeHostWake::ServiceAfterCurrent
            },
        )
    }

    pub(super) fn service_pending_prefixes(
        &mut self,
        context: &egui::Context,
        token: Option<NativeOutputToken>,
        budget: &mut NativeInputBudget,
    ) -> Result<bool, NativeRuntimeError> {
        if self.owned_update.is_some() {
            self.pending_service = true;
            return Ok(false);
        }
        self.pending_service = false;
        let bridge = self.coordinator.dispatch_bridge();
        self.abandon_unfinalized_output_passes(context)?;
        self.coordinator
            .drive_pointer_capture(context, budget, self.shutdown.is_none())?;
        if self.coordinator.drive_close_policy(
            context,
            self.close_policy,
            self.root_surface,
            budget,
        )? {
            self.pass_actions
                .retire_unavailable(self.coordinator.session());
            self.settle_internal_application_transitions()?;
        }
        loop {
            let before = budget.remaining_records();
            let pending_output = self.coordinator.has_pending_output_prefix();
            let effect = self.coordinator.try_report_pending_effect_results()?;
            let quiescence = token.is_none_or(|token| token.viewport_id() == ViewportId::ROOT)
                && self.coordinator.try_report_retirement_quiescence()?;
            let reduced = self
                .coordinator
                .reduce_callback_prefix(context, budget)?
                .is_some();
            if let Some(error) = self.coordinator.take_callback_error() {
                return Err(error);
            }
            let mut invalidations = if bridge.pending_no_frame_count() == 0 {
                crate::coordinator::PreparedNoFrameInvalidations::default()
            } else {
                self.coordinator.prepare_no_frame_invalidations()?
            };
            let progress = pending_output
                || effect
                || quiescence
                || reduced
                || matches!(bridge.admission_dispatch(), NativeDispatch::Cancel { .. })
                || self.coordinator.session().has_pending_native_input()
                || bridge.callback_boundary_pending();
            let pending_create = self.shutdown.is_none()
                && bridge.accepts_new_work()
                && self.coordinator.session().has_pending_native_create();
            // A service turn can publish application work without painting Root.
            // Already admitted UI input and placement declarations still own
            // their original paint boundary and must precede that publication.
            let application_ready = self.application_service_ready();
            if !progress && invalidations.ready.is_empty() {
                if !pending_create && !application_ready {
                    break;
                }
                if budget.is_exhausted() {
                    return Ok(false);
                }
            }
            let published = self.coordinator.session().version();
            let Some(mut frame) = self.coordinator.poll_resolved_host_frame(context, budget)?
            else {
                return Ok(false);
            };
            invalidations.select_for_frame(&frame.frame, !frame.native_input_pending());
            for &viewport in &invalidations.repaint_viewports {
                context.request_repaint_once_of(viewport);
            }
            let mut application_action =
                if application_ready && !frame.native_input_pending() && !budget.is_exhausted() {
                    self.application_actions
                        .take()
                        .map(|action| action.submit(&mut frame.frame, published))
                        .transpose()?
                        .map(|submission| submission.finish(&mut frame.frame))
                        .transpose()?
                } else {
                    None
                };
            let advanced_create = if self.shutdown.is_none()
                && bridge.accepts_new_work()
                && !frame.native_input_pending()
                && !budget.is_exhausted()
                && application_action.is_none()
            {
                frame.frame.advance_next_native_surface_create()?
            } else {
                false
            };
            if advanced_create || application_action.is_some() {
                budget.complete_unsequenced_records(1);
            } else if !progress && invalidations.ready.is_empty() {
                // Waiting for fresh facts or cleanup must not publish empty frames
                // or keep the service loop polling an unchanged create intent.
                drop(frame);
                if application_ready {
                    return Ok(false);
                }
                break;
            }
            crate::coordinator::defer_no_frame_invalidations(
                &mut frame.frame,
                &invalidations.ready,
            )?;
            frame.complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)?;
            let report = frame.commit_publishing_root(&mut self.root_surface)?;
            self.coordinator
                .finish_no_frame_invalidations(&invalidations.ready);
            self.publish_application_configuration(context, &mut application_action, &report)?;
            self.settle_published_report(
                context,
                token,
                application_action,
                report,
                budget,
                false,
            )?;
            self.settle_internal_application_transitions()?;
            let pending_create = self.shutdown.is_none()
                && bridge.accepts_new_work()
                && self.coordinator.session().has_pending_native_create();
            if !self.coordinator.has_pending_callback_records()
                && !self.coordinator.session().has_pending_native_input()
                && !self.coordinator.has_pending_output_prefix()
                && !bridge.callback_boundary_pending()
                && !matches!(bridge.admission_dispatch(), NativeDispatch::Cancel { .. })
                && !pending_create
                && !self.application_service_ready()
            {
                // Committing progress can free ingress capacity or the last host
                // reference for a retirement candidate. Retry once; a candidate
                // still waiting for external facts makes no progress and waits.
                if progress
                    && token.is_none_or(|token| token.viewport_id() == ViewportId::ROOT)
                    && bridge.next_retirement_quiescence_candidate(None).is_some()
                {
                    return Ok(false);
                }
                break;
            }
            if budget.is_exhausted() || budget.remaining_records() == before {
                return Ok(false);
            }
        }
        if self.coordinator.has_pending_callback_records()
            || self.coordinator.session().has_pending_native_input()
            || bridge.callback_boundary_pending()
            || matches!(bridge.admission_dispatch(), NativeDispatch::Cancel { .. })
        {
            return Ok(false);
        }
        if token.is_none_or(|token| token.viewport_id() == ViewportId::ROOT)
            && let Some(prepared) = self.coordinator.prepare_committed_retirements()?
        {
            for abandoned in self.coordinator.commit_retirements(prepared) {
                self.pass_actions
                    .abandon(abandoned)
                    .map_err(map_pass_action_error)?;
            }
            context.request_repaint_of(ViewportId::ROOT);
        }
        bridge.request_pending_global_focus(context);
        Ok(true)
    }
}
