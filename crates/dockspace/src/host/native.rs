//! Shared native-multiview report synchronization and retirement helpers.
//!
//! These renderer-neutral utilities sit on top of [`NativeWindows`],
//! [`crate::runtime::NativeEffectOutbox`], and [`HostFrameReport`] so every
//! multi-window UI framework handles root continuations, placement detachments,
//! deduplicated window repaints, close requests, and exact quiescence
//! preconditions identically.

use std::fmt;
use std::hash::Hash;

use crate::model::SurfaceId;
use crate::runtime::{
    DockspaceClosePlan, DockspaceRuntimeError, DockspaceSession, HostFrameReport, HostInputOutcome,
    NativeCloseEffectAcknowledgement, NativeEffectRequest, NativeRootContinuation,
    NativeSurfaceBinding, NativeSurfaceCloseRequest, NativeWindowBinding, NativeWindowFacts,
    NativeWindowRegistryError, NativeWindows,
};

/// Structured native-lifecycle outputs extracted from one committed [`HostFrameReport`].
///
/// Consumes affine native effects in core order while preserving root
/// continuations, placement detachments, first-live admissions, close requests
/// from both close and snapshot observations, close plans, and dirty surfaces.
#[derive(Debug)]
pub struct NativeReportDigest {
    effects: Vec<NativeEffectRequest>,
    root_continuations: Vec<NativeRootContinuation>,
    placement_detachments: Vec<NativeSurfaceBinding>,
    admissions: Vec<NativeSurfaceBinding>,
    close_requests: Vec<NativeSurfaceCloseRequest>,
    close_plans: Vec<DockspaceClosePlan>,
    repaint_surfaces: Vec<SurfaceId>,
    native_input_pending: bool,
}

impl NativeReportDigest {
    /// Extracts native lifecycle outputs from `report`, taking its affine native
    /// effects in exact core order.
    #[must_use = "dispatch or retain effects and retire detached bindings from the digest"]
    pub fn take_from(report: &mut HostFrameReport) -> Self {
        let effects = report.take_native_effects();
        let root_continuations = report.native_root_continuations().to_vec();
        let placement_detachments = report.native_placement_detachments().to_vec();
        let admissions = report.native_admissions().to_vec();
        let repaint_surfaces = report.repaint_surfaces().to_vec();
        let native_input_pending = report.native_input_pending();

        let mut close_requests = Vec::new();
        let mut close_plans = Vec::new();
        for outcome in report.inputs() {
            match outcome {
                HostInputOutcome::NativeCloseObservationApplied {
                    close_requests: edges,
                }
                | HostInputOutcome::NativePlatformSnapshotApplied {
                    close_requests: edges,
                } => {
                    close_requests.extend_from_slice(edges);
                }
                HostInputOutcome::NativeSurfaceCloseRequested { plan, .. } => {
                    close_plans.push(plan.clone());
                }
                _ => {}
            }
        }

        Self {
            effects,
            root_continuations,
            placement_detachments,
            admissions,
            close_requests,
            close_plans,
            repaint_surfaces,
            native_input_pending,
        }
    }

    /// Takes the ordered affine native effects emitted by this frame.
    #[must_use = "dispatch, reject, or retain each native effect request"]
    pub fn take_effects(&mut self) -> Vec<NativeEffectRequest> {
        std::mem::take(&mut self.effects)
    }

    /// Returns the remaining queued native effects without consuming them.
    #[must_use]
    pub fn effects(&self) -> &[NativeEffectRequest] {
        &self.effects
    }

    /// Returns committed root continuations whose OS window continues under a successor binding.
    #[must_use]
    pub fn root_continuations(&self) -> &[NativeRootContinuation] {
        &self.root_continuations
    }

    /// Returns exact placements detached without destroying their shared window.
    #[must_use]
    pub fn placement_detachments(&self) -> &[NativeSurfaceBinding] {
        &self.placement_detachments
    }

    /// Returns native bindings that crossed the first-live admission barrier.
    #[must_use]
    pub fn admissions(&self) -> &[NativeSurfaceBinding] {
        &self.admissions
    }

    /// Returns unresolved native close edges reported by either close or snapshot observations.
    #[must_use]
    pub fn close_requests(&self) -> &[NativeSurfaceCloseRequest] {
        &self.close_requests
    }

    /// Returns core-owned close plans created for native close requests in this frame.
    #[must_use]
    pub fn close_plans(&self) -> &[DockspaceClosePlan] {
        &self.close_plans
    }

    /// Returns logical surfaces requiring paint after this frame.
    #[must_use]
    pub fn repaint_surfaces(&self) -> &[SurfaceId] {
        &self.repaint_surfaces
    }

    /// Returns whether earlier accepted native records still await another poll/frame.
    #[must_use]
    pub const fn native_input_pending(&self) -> bool {
        self.native_input_pending
    }

    /// Applies committed root continuations to `windows` and returns every retired
    /// placement binding (continuation predecessors and placement detachments)
    /// that must drain outputs and callbacks before reporting quiescence.
    ///
    /// # Errors
    ///
    /// Returns [`NativeWindowRegistryError`] if a root continuation conflicts with
    /// the current window resource map.
    pub fn apply_to_windows<K, T>(
        &self,
        windows: &mut NativeWindows<K, T>,
    ) -> Result<Vec<NativeSurfaceBinding>, NativeWindowRegistryError<K>>
    where
        K: Eq + Hash + Copy + fmt::Debug,
    {
        let mut retired =
            Vec::with_capacity(self.root_continuations.len() + self.placement_detachments.len());
        for continuation in &self.root_continuations {
            windows.continue_root(*continuation)?;
            retired.push(continuation.predecessor());
        }
        retired.extend_from_slice(&self.placement_detachments);
        Ok(retired)
    }

    /// Maps [`Self::repaint_surfaces`] onto deduplicated live OS windows in `windows`.
    ///
    /// Multiple dirty logical surfaces hosted by the same physical window emit
    /// that window once.
    #[must_use]
    pub fn dirty_windows<K, T>(
        &self,
        session: &DockspaceSession,
        windows: &NativeWindows<K, T>,
    ) -> Vec<(NativeWindowBinding, K)>
    where
        K: Eq + Hash + Copy + fmt::Debug,
    {
        if self.repaint_surfaces.is_empty() {
            return Vec::new();
        }
        windows
            .iter()
            .filter_map(|(binding, window, _)| {
                session
                    .native_surface_bindings_for_window(binding)
                    .any(|surface| self.repaint_surfaces.contains(&surface.surface()))
                    .then_some((binding, window))
            })
            .collect()
    }
}

/// Exact quiescence tracker for one retiring native OS window.
///
/// Native destruction alone does not prove host quiescence: the host must also
/// retire every painted output and seal every callback route before calling
/// [`Self::report_quiescence_if_ready`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeWindowRetirementState {
    window: NativeWindowBinding,
    placements: Vec<NativeSurfaceBinding>,
    close_acknowledgement: Option<NativeCloseEffectAcknowledgement>,
    native_destroyed: bool,
    outputs_retired: bool,
    callbacks_sealed: bool,
    quiescence_reported: bool,
}

impl NativeWindowRetirementState {
    /// Begins tracking retirement for `window` and all of its retained surface placements.
    #[must_use]
    pub fn new(
        window: NativeWindowBinding,
        placements: impl IntoIterator<Item = NativeSurfaceBinding>,
        close_acknowledgement: Option<NativeCloseEffectAcknowledgement>,
    ) -> Self {
        Self {
            window,
            placements: placements.into_iter().collect(),
            close_acknowledgement,
            native_destroyed: false,
            outputs_retired: false,
            callbacks_sealed: false,
            quiescence_reported: false,
        }
    }

    /// Captures all currently retained placements for `window` directly from `session`.
    #[must_use]
    pub fn from_session(
        session: &DockspaceSession,
        window: NativeWindowBinding,
        close_acknowledgement: Option<NativeCloseEffectAcknowledgement>,
    ) -> Self {
        Self::new(
            window,
            session.retained_native_surface_bindings_for_window(window),
            close_acknowledgement,
        )
    }

    /// Returns the retiring physical window binding.
    #[must_use]
    pub const fn window(&self) -> NativeWindowBinding {
        self.window
    }

    /// Returns the exact placements retained for quiescence reporting.
    #[must_use]
    pub fn placements(&self) -> &[NativeSurfaceBinding] {
        &self.placements
    }

    /// Records the platform's actual native window destruction event.
    pub fn mark_native_destroyed(&mut self) {
        self.native_destroyed = true;
    }

    /// Records that every painted output belonging to this window has settled or retired.
    pub fn mark_outputs_retired(&mut self) {
        self.outputs_retired = true;
    }

    /// Records that every platform callback and route capable of naming this window is sealed.
    pub fn mark_callbacks_sealed(&mut self) {
        self.callbacks_sealed = true;
    }

    /// Returns whether the platform has reported actual native destruction.
    #[must_use]
    pub const fn is_native_destroyed(&self) -> bool {
        self.native_destroyed
    }

    /// Returns destruction facts for a managed native snapshot once native
    /// destruction has actually occurred, or `None` while still waiting.
    #[must_use]
    pub fn destruction_facts(&self) -> Option<NativeWindowFacts> {
        if !self.native_destroyed {
            return None;
        }
        Some(self.close_acknowledgement.map_or_else(
            NativeWindowFacts::destroyed,
            NativeWindowFacts::destroyed_after,
        ))
    }

    /// Returns whether all three quiescence preconditions (native destruction,
    /// output retirement, and callback sealing) have been satisfied.
    #[must_use]
    pub const fn is_ready_for_quiescence(&self) -> bool {
        self.native_destroyed && self.outputs_retired && self.callbacks_sealed
    }

    /// Returns whether quiescence has already been reported for all retained placements.
    #[must_use]
    pub const fn is_quiescent(&self) -> bool {
        self.quiescence_reported
    }

    /// Reports permanent binding quiescence to `session` once all preconditions hold.
    ///
    /// Returns `Ok(true)` when quiescence was reported (or was already reported)
    /// and the entry can be discarded; returns `Ok(false)` while waiting for
    /// native destruction, output retirement, or callback sealing.
    ///
    /// # Errors
    ///
    /// Returns any core error from [`DockspaceSession::report_native_binding_quiescence`],
    /// retaining remaining unreported placements so the host can retry.
    pub fn report_quiescence_if_ready(
        &mut self,
        session: &mut DockspaceSession,
    ) -> Result<bool, DockspaceRuntimeError> {
        if self.quiescence_reported {
            return Ok(true);
        }
        if !self.is_ready_for_quiescence() {
            return Ok(false);
        }
        while let Some(binding) = self.placements.last().copied() {
            session.report_native_binding_quiescence(binding)?;
            self.placements.pop();
        }
        self.quiescence_reported = true;
        Ok(true)
    }
}
