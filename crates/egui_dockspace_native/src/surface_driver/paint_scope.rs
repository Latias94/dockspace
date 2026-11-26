//! One runtime-owned content namespace across native surfaces and viewports.

use dockspace::model::{SurfaceId, WorkspaceVersion};
use dockspace::runtime::NativeSurfaceBinding;
use eframe::{NativeOutputToken, egui};
use egui_dockspace::{DockStyle, PaneView};

use super::composition::NativePaintMapping;
use crate::error::NativeRuntimeError;
use crate::host_frame::{NativeHostFrame, NativeSurfacePaint};
use crate::pass_actions::NativePassActions;

/// Runtime construction chooses the namespace once. Surface callbacks cannot
/// replace it with a placement or viewport identity when rendering pane content.
#[derive(Clone, Copy)]
pub(super) struct NativePaintScope {
    instance: egui::Id,
}

impl NativePaintScope {
    pub(super) const fn new(instance: egui::Id) -> Self {
        Self { instance }
    }

    pub(super) const fn instance(self) -> egui::Id {
        self.instance
    }

    pub(super) fn child_ui(
        self,
        mapping: NativePaintMapping,
        parent: &mut egui::Ui,
        surface: SurfaceId,
    ) -> egui::Ui {
        mapping.child_ui(parent, self.instance, surface)
    }

    pub(super) fn workspace_focus_id(self, ui: &egui::Ui, surface: SurfaceId) -> egui::Id {
        ui.make_persistent_id((self.instance, "workspace-focus", surface))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the existing touch-menu handoff retains its exact output, owner and version"
    )]
    pub(super) fn stage_touch_menu(
        self,
        actions: &mut NativePassActions,
        token: NativeOutputToken,
        owner: Option<NativeSurfaceBinding>,
        version: WorkspaceVersion,
        ui: &egui::Ui,
        surface: SurfaceId,
    ) {
        actions.stage_touch_menu(token, owner, version, ui, self.instance, surface);
    }

    pub(super) fn paint_surface(
        self,
        frame: &mut NativeHostFrame<'_>,
        surface: SurfaceId,
        ui: &mut egui::Ui,
        panes: &mut dyn PaneView,
        style: &DockStyle,
    ) -> Result<NativeSurfacePaint, NativeRuntimeError> {
        frame.paint_surface(self.instance, surface, ui, panes, style)
    }

    pub(super) fn retain_surface_input_state(
        self,
        frame: &NativeHostFrame<'_>,
        context: &egui::Context,
        surface: SurfaceId,
        panes: &dyn PaneView,
    ) {
        frame.retain_surface_input_state(context, self.instance, surface, panes);
    }
}
