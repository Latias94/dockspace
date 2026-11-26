//! Physical resources and OS observations, separate from the docking workspace.

use std::{
    collections::{BTreeMap, VecDeque},
    num::NonZeroU32,
    rc::Rc,
};

use dockspace::{
    close::CloseDecision,
    geometry::{PhysicalRect, ScaleFactor},
    host::{NativeReportDigest, NativeWindowRetirementState},
    runtime::{
        DockspaceClosePhase, DockspaceClosePlan, DockspaceSession, HostFrameReport,
        NativeCloseDisposition, NativeCloseEffectAcknowledgement, NativeCloseState,
        NativeCreateEffectAcknowledgement, NativeCreateReadiness, NativeDispatchFailure,
        NativeEffectAcknowledgement, NativeEffectOperation, NativeEffectRequest,
        NativeHostErrorKind, NativeSurfaceCloseAction, NativeSurfaceRole, NativeUnsupportedReason,
        NativeWindowBinding, NativeWindowFacts, NativeWorkAreaRoster,
    },
};
use dockspace_winit::{
    backend::{
        dpi::PhysicalSize,
        event_loop::{ActiveEventLoop, OwnedDisplayHandle},
        window::{Window, WindowId},
    },
    native::{
        NativeWindows, WindowPlacementError, observe_window, request_outer_bounds,
        window_attributes,
    },
};

use super::{AppResult, workspace};

pub(super) struct WindowState {
    // Drop renderer resources before releasing the final native window owner.
    pub surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
    pub window: Rc<Window>,
    prepared_size: Option<PhysicalSize<u32>>,
    create: Option<NativeCreateEffectAcknowledgement>,
    child: bool,
    failure: Option<String>,
    pending_placement: Option<PhysicalRect>,
}

impl WindowState {
    pub fn new(event_loop: &ActiveEventLoop, window: Window, child: bool) -> Self {
        let window = Rc::new(window);
        let renderer = softbuffer::Context::new(event_loop.owned_display_handle())
            .and_then(|context| softbuffer::Surface::new(&context, Rc::clone(&window)));
        let (surface, failure) = match renderer {
            Ok(surface) => (Some(surface), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let mut state = Self {
            surface,
            window,
            prepared_size: None,
            create: None,
            child,
            failure,
            pending_placement: None,
        };
        state.prepare();
        state
    }

    pub fn prepare(&mut self) {
        let size = self.window.inner_size();
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            self.prepared_size = None;
            return;
        };
        if self.prepared_size != Some(size) {
            self.prepared_size = None;
            match surface.resize(width, height) {
                Ok(()) => self.prepared_size = Some(size),
                Err(error) => {
                    self.failure = Some(error.to_string());
                    self.surface = None;
                }
            }
        }
    }

    pub fn preparation_failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn apply_initial_placement(&mut self) -> AppResult<()> {
        let Some(bounds) = self.pending_placement else {
            return Ok(());
        };
        if self.failure.is_some() || self.window.is_visible() != Some(true) {
            return Ok(());
        }
        match request_outer_bounds(&self.window, bounds) {
            Ok(_) => self.pending_placement = None,
            Err(
                WindowPlacementError::GeometryChanged { .. }
                | WindowPlacementError::GeometryUnavailable { .. },
            ) => {
                self.window.request_redraw();
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn facts(&mut self) -> AppResult<NativeWindowFacts> {
        self.prepare();
        let mut facts =
            observe_window(&self.window, ScaleFactor::new(self.window.scale_factor())?)?;
        if self.child {
            facts = facts.with_create_readiness(
                if self.failure.is_some() {
                    NativeCreateReadiness::Failed
                } else if self.pending_placement.is_none()
                    && self.prepared_size.is_some()
                    && self.window.is_visible() == Some(true)
                    && self.window.is_minimized() == Some(false)
                {
                    NativeCreateReadiness::Prepared
                } else {
                    NativeCreateReadiness::Created
                },
                self.create,
            );
        }
        Ok(facts)
    }
}

pub(super) struct Host {
    pub windows: NativeWindows<WindowState>,
    effects: VecDeque<NativeEffectRequest>,
    retiring: BTreeMap<WindowId, NativeWindowRetirementState>,
    pending_records: bool,
    last_snapshot: Vec<(NativeWindowBinding, NativeWindowFacts)>,
    close_action: NativeSurfaceCloseAction,
    close_plans: Vec<DockspaceClosePlan>,
    pub creations: usize,
    pub destructions: usize,
    pub fail_next_preparation: bool,
}

impl Host {
    pub fn new() -> Self {
        Self {
            windows: NativeWindows::new(),
            effects: VecDeque::new(),
            retiring: BTreeMap::new(),
            pending_records: false,
            last_snapshot: Vec::new(),
            close_action: NativeSurfaceCloseAction::RecoverPresentation,
            close_plans: Vec::new(),
            creations: 0,
            destructions: 0,
            fail_next_preparation: false,
        }
    }

    pub fn accept(
        &mut self,
        session: &mut DockspaceSession,
        mut report: HostFrameReport,
    ) -> AppResult<()> {
        // Retain the entire ordered suffix before doing anything fallible.
        let mut digest = NativeReportDigest::take_from(&mut report);
        self.effects.extend(digest.take_effects());
        for retired in digest.apply_to_windows(&mut self.windows)? {
            session.report_native_binding_quiescence(retired)?;
            self.pending_records = true;
        }
        for plan in digest.close_plans() {
            if self.close_action == NativeSurfaceCloseAction::RetainLayout
                && plan.phase() == DockspaceClosePhase::AwaitingDecision
            {
                self.close_plans.push(plan.clone());
            }
        }
        for close in digest.close_requests() {
            if close.binding().surface() != workspace::MAIN {
                session.request_native_surface_close(*close, self.close_action)?;
                self.pending_records = true;
            }
        }
        for (binding, _) in digest.dirty_windows(session, &self.windows) {
            if let Some(state) = self.windows.get(binding) {
                state.window.request_redraw();
            }
        }
        Ok(())
    }

    /// Publishes complete observations only when all physical resources can be
    /// sampled, or their destruction has actually been delivered by winit.
    pub fn observe(&mut self, session: &mut DockspaceSession) -> AppResult<bool> {
        if self
            .retiring
            .values()
            .any(|entry| !entry.is_native_destroyed())
        {
            return Ok(false);
        }
        let mut facts = Vec::new();
        for (binding, _, state) in self.windows.iter_mut() {
            facts.push((binding, state.facts()?));
        }
        for entry in self.retiring.values() {
            if let Some(destroyed) = entry.destruction_facts() {
                facts.push((entry.window(), destroyed));
            }
        }
        if facts == self.last_snapshot {
            return Ok(false);
        }
        session
            .report_managed_native_snapshot(facts.iter().copied(), NativeWorkAreaRoster::Unknown)?;
        self.last_snapshot = facts;
        Ok(true)
    }

    /// Commits observations and drains destroyed routes, including bounded ingress retries.
    pub fn settle_observations(&mut self, session: &mut DockspaceSession) -> AppResult<()> {
        loop {
            self.observe(session)?;
            let report = workspace::idle(session)?;
            self.records_committed();
            self.accept(session, report)?;
            // observe() withholds a complete snapshot until every destruction arrives.
            if self
                .retiring
                .values()
                .all(NativeWindowRetirementState::is_native_destroyed)
            {
                self.drain_destroyed(session)?;
            }
            if !self.pending_records {
                return Ok(());
            }
        }
    }

    pub fn dispatch(
        &mut self,
        event_loop: &ActiveEventLoop,
        session: &mut DockspaceSession,
    ) -> AppResult<()> {
        if !self.close_plans.is_empty() {
            let mut frame = session
                .begin_native_host_frame(|_| dockspace::runtime::NativeReceiverAnswer::Unknown)?;
            // This example has no editable pane data. A real application asks
            // its document owners or saves before resolving these exact tokens.
            for plan in &self.close_plans {
                for item in plan.items() {
                    frame.resolve_close(plan.request(), item.token(), CloseDecision::Allow)?;
                }
            }
            let report = frame.commit()?;
            self.close_plans.clear();
            self.accept(session, report)?;
        }
        while let Some(request) = self.effects.pop_front() {
            let operation = request.operation().clone();
            match operation {
                NativeEffectOperation::CreateWindow {
                    binding,
                    placement,
                    role,
                }
                | NativeEffectOperation::RequestReplacement {
                    binding,
                    placement,
                    role,
                } => {
                    let attributes = window_attributes(
                        Window::default_attributes()
                            .with_title("Dockspace | Inspector | D: dock back | close: recover"),
                        placement,
                    )?;
                    let dispatch = request.begin_dispatch();
                    let window = match event_loop.create_window(attributes) {
                        Ok(window) => window,
                        Err(error) => {
                            session.report_native_effect_result(
                                dispatch.not_applied(NativeDispatchFailure::AdapterRejected),
                            )?;
                            return Err(error.into());
                        }
                    };
                    let id = window.id();
                    // Keep the successfully created window owned even if later
                    // renderer preparation fails, so compensation can destroy it.
                    let mut state =
                        WindowState::new(event_loop, window, role == NativeSurfaceRole::Child);
                    // X11 decorations and client size can change during mapping.
                    // Process initial window events before compensating their extents.
                    state.pending_placement = Some(placement);
                    if std::mem::take(&mut self.fail_next_preparation) {
                        state.surface = None;
                        state.prepared_size = None;
                        state.failure =
                            Some("smoke: renderer preparation deliberately failed".into());
                    }
                    let Some(NativeEffectAcknowledgement::Create(ack)) = dispatch.accepted() else {
                        return Err(
                            "create dispatch did not return its readiness capability".into()
                        );
                    };
                    state.create = Some(ack);
                    state.window.request_redraw();
                    self.windows.insert(binding, id, state)?;
                    self.creations += 1;
                }
                NativeEffectOperation::ReleaseChild { binding }
                | NativeEffectOperation::CompensatingClose { binding } => {
                    let dispatch = request.begin_dispatch();
                    let Some(NativeEffectAcknowledgement::Close(ack)) = dispatch.accepted() else {
                        return Err(
                            "close dispatch did not return its destruction capability".into()
                        );
                    };
                    self.retire(session, binding.window(), Some(ack))?;
                }
                NativeEffectOperation::ResolveNativeClose {
                    close,
                    resolution: NativeCloseDisposition::Accept,
                } => {
                    let dispatch = request.begin_dispatch();
                    let Some(NativeEffectAcknowledgement::Close(ack)) = dispatch.accepted() else {
                        return Err("accepted native close has no destruction capability".into());
                    };
                    self.retire(session, close.binding().window(), Some(ack))?;
                }
                NativeEffectOperation::ResolveNativeClose {
                    close,
                    resolution: NativeCloseDisposition::Cancel,
                } => {
                    let acknowledgement = request.begin_dispatch().accepted();
                    let Some(NativeEffectAcknowledgement::Close(ack)) = acknowledgement else {
                        return Err("cancelled native close has no observation capability".into());
                    };
                    session.publish_native_close(
                        close.binding(),
                        NativeCloseState::Clear,
                        Some(ack),
                    )?;
                }
                NativeEffectOperation::CancelRootClose { binding } => {
                    let Some(NativeEffectAcknowledgement::Close(ack)) =
                        request.begin_dispatch().accepted()
                    else {
                        return Err("root close cancellation has no observation capability".into());
                    };
                    session.publish_native_close(binding, NativeCloseState::Clear, Some(ack))?;
                }
                NativeEffectOperation::RetainChild { .. } => {
                    let _ = request.begin_dispatch().accepted();
                }
                NativeEffectOperation::RequestFocus { binding } => {
                    if let Some(state) = self.windows.get(binding) {
                        let dispatch = request.begin_dispatch();
                        state.window.focus_window();
                        let _ = dispatch.accepted();
                    } else {
                        session.report_native_effect_result(
                            request.not_applied(NativeDispatchFailure::WindowUnavailable),
                        )?;
                    }
                }
                _ => {
                    session.report_native_effect_result(
                        request.unsupported(NativeUnsupportedReason::BackendUnsupported),
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn retire(
        &mut self,
        session: &DockspaceSession,
        binding: NativeWindowBinding,
        close: Option<NativeCloseEffectAcknowledgement>,
    ) -> AppResult<()> {
        let state = self
            .windows
            .remove(binding)
            .ok_or("retiring window resources are missing")?;
        let id = state.window.id();
        let mut retirement = NativeWindowRetirementState::from_session(session, binding, close);
        // All outputs in this synchronous example are already settled before retiring.
        retirement.mark_outputs_retired();
        self.retiring.insert(id, retirement);
        // No window clones or deferred renderer work survive this point. This
        // is a close command; only the later Destroyed event proves destruction.
        drop(state);
        Ok(())
    }

    pub fn destroyed(&mut self, id: WindowId) -> bool {
        let Some(entry) = self.retiring.get_mut(&id) else {
            return false;
        };
        entry.mark_native_destroyed();
        entry.mark_callbacks_sealed();
        self.destructions += 1;
        true
    }

    pub fn drain_destroyed(&mut self, session: &mut DockspaceSession) -> AppResult<()> {
        let destroyed: Vec<_> = self
            .retiring
            .iter()
            .filter_map(|(id, entry)| entry.is_ready_for_quiescence().then_some(*id))
            .collect();
        for id in destroyed {
            let entry = self
                .retiring
                .get_mut(&id)
                .ok_or("destroyed route is missing")?;
            // All outputs were synchronously settled and the final OS event is
            // consumed. Retain the unrecorded suffix if ingress is full.
            let remaining_before = entry.placements().len();
            match entry.report_quiescence_if_ready(session) {
                Ok(true) => {
                    if entry.placements().len() < remaining_before {
                        self.pending_records = true;
                    }
                    let window = entry.window();
                    self.last_snapshot.retain(|(binding, _)| *binding != window);
                    self.retiring.remove(&id);
                }
                Ok(false) => {}
                Err(error) => {
                    if entry.placements().len() < remaining_before {
                        self.pending_records = true;
                    }
                    if error.native_kind() == Some(NativeHostErrorKind::InputCapacity)
                        && self.pending_records
                    {
                        return Ok(());
                    }
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }

    pub fn idle(&self) -> bool {
        self.retiring.is_empty() && !self.has_work()
    }

    pub fn has_work(&self) -> bool {
        !self.effects.is_empty() || self.pending_records || !self.close_plans.is_empty()
    }

    pub fn records_committed(&mut self) {
        self.pending_records = false;
    }

    pub fn begin_shutdown(&mut self) {
        // Shutdown preserves layout independently of a recovery window's
        // visibility and its ability to accept another presentation.
        self.close_action = NativeSurfaceCloseAction::RetainLayout;
    }

    pub fn root_minimized(&self) -> bool {
        self.windows
            .iter()
            .any(|(_, _, state)| !state.child && state.window.is_minimized() == Some(true))
    }

    pub fn child(&self) -> Option<NativeWindowBinding> {
        self.windows
            .iter()
            .find_map(|(binding, _, state)| state.child.then_some(binding))
    }

    pub fn child_placement(&self) -> AppResult<PhysicalRect> {
        let (_, _, root) = self
            .windows
            .iter()
            .find(|(_, _, state)| !state.child)
            .ok_or("main window is missing")?;
        let window = &root.window;
        let origin = window.outer_position()?;
        let scale = window.scale_factor();
        Ok(PhysicalRect::new(
            f64::from(origin.x) + 120.0 * scale,
            f64::from(origin.y) + 100.0 * scale,
            420.0 * scale,
            300.0 * scale,
        )?)
    }
}

#[cfg(test)]
mod tests;
