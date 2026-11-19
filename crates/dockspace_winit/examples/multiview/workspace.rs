//! The application chooses commands; core owns layout and recovery decisions.

use dockspace::{
    geometry::{LogicalRect, PhysicalRect},
    model::{
        DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
        ItemId, NativeWindowPlacement, RootId, SurfaceId,
    },
    policy::DockPolicy,
    runtime::{
        DockMoveSource, DockspaceHostFrame, DockspaceSession, HostFrameReport,
        NativeHostCapabilities, NativeHostCapability, NativePointerRoster, NativeReceiverAnswer,
        NativeWindowBinding, SurfacePaintPlan,
    },
};

use super::AppResult;

#[cfg(test)]
#[path = "workspace/tests.rs"]
mod tests;

pub(super) const MAIN: SurfaceId = SurfaceId::new(1);
pub(super) const DOCUMENT: ItemId = ItemId::new(1);
pub(super) const INSPECTOR: ItemId = ItemId::new(5);

pub(super) fn session() -> AppResult<DockspaceSession> {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        MAIN,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::central_tabs([DOCUMENT, ItemId::new(2)]), 3.0),
                    (DockspaceNode::tabs([INSPECTOR]), 1.0),
                ],
            )?,
        ),
    )])?;
    let mut policy = DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    let mut session = DockspaceSession::from_layout(layout, policy)?;
    session.retain_host_surface(MAIN)?;
    // Stock winit cannot establish a complete physical pointer roster. This
    // example exercises keyboard commands and native presentation, not dragging.
    session.enable_managed_native_host(NativePointerRoster::Unknown)?;
    let capabilities = NativeHostCapabilities::none_supported()
        .with(NativeHostCapability::NativeWindowLifecycle)
        .with(NativeHostCapability::AuthoritativeInventory)
        .with(NativeHostCapability::CloseCancellation)
        .with(NativeHostCapability::GlobalWindowPlacement);
    session.configure_managed_native_capabilities(capabilities)?;
    Ok(session)
}

pub(super) fn idle(session: &mut DockspaceSession) -> AppResult<HostFrameReport> {
    Ok(session
        .begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)?
        .commit()?)
}

pub(super) fn prepare_window_paint<'frame>(
    frame: &'frame mut DockspaceHostFrame<'_>,
    window: NativeWindowBinding,
    bounds: LogicalRect,
) -> AppResult<Option<SurfacePaintPlan<'frame>>> {
    let surface = {
        let mut bindings = frame.native_surface_bindings_for_window(window);
        let Some(binding) = bindings.next() else {
            return Ok(None);
        };
        if bindings.next().is_some() {
            return Err("this example renders one logical surface per native window".into());
        }
        binding.surface()
    };
    // The platform can request redraw before the child enters the paint roster.
    if !frame.surfaces().contains(&surface) {
        return Ok(None);
    }
    frame.begin_encoding_pass()?;
    super::paint::measure(frame, surface, bounds)?;
    Ok(frame.paint_plan(surface)?)
}

pub(super) fn detach(
    session: &mut DockspaceSession,
    placement: PhysicalRect,
) -> AppResult<HostFrameReport> {
    let command = session.native_window_command(
        DockMoveSource::Item(INSPECTOR),
        NativeWindowPlacement::new(placement),
    )?;
    let Ok(prepared) = command.into_prepared() else {
        return idle(session);
    };
    let mut frame = session.begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)?;
    let action = frame.submit_prepared_action(prepared)?;
    let report = frame.commit()?;
    if !matches!(
        report.submitted_action_outcome(action),
        Some(dockspace::runtime::HostInputOutcome::ProductPresentationActionRequested { .. })
    ) {
        return Err(format!(
            "native tear-off rejected: {:?}",
            report.submitted_action_outcome(action)
        )
        .into());
    }
    Ok(report)
}

pub(super) fn dock_back(session: &mut DockspaceSession) -> AppResult<HostFrameReport> {
    let root = session
        .view()
        .item(INSPECTOR)
        .ok_or("Inspector is missing")?
        .root();
    let command = session.presentation_commands(root).dock_back()?;
    let Ok(action) = command.into_prepared() else {
        return idle(session);
    };
    let mut frame = session.begin_native_host_frame(|_| NativeReceiverAnswer::Unknown)?;
    let submitted = frame.submit_prepared_action(action)?;
    let report = frame.commit()?;
    if !matches!(
        report.submitted_action_outcome(submitted),
        Some(
            dockspace::runtime::HostInputOutcome::ProductActionApplied { .. }
                | dockspace::runtime::HostInputOutcome::ProductPresentationActionRequested { .. }
        )
    ) {
        return Err(format!(
            "Dock Back rejected: {:?}",
            report.submitted_action_outcome(submitted)
        )
        .into());
    }
    Ok(report)
}
