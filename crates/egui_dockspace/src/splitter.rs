//! Paint-only customization of divider handles and intersections.

use egui::{Color32, Id, Painter, Rect};

use crate::{DockspaceAxis, RootId, SurfaceId};

/// The geometry and input owned by a splitter control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitterKind {
    /// A divider that moves along the given axis.
    Divider(DockspaceAxis),
    /// An intersection that can resize its incident dividers together.
    Junction,
}

/// Paint state of one splitter control, supplied by [`crate::PaneView::splitter_ui`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Splitter {
    /// Stable egui widget ID, useful for per-control paint effects.
    pub id: Id,
    /// Surface containing the control.
    pub surface: SurfaceId,
    /// Docking root containing the control.
    pub root: RootId,
    /// Divider orientation or intersection.
    pub kind: SplitterKind,
    /// Default drawing bounds.
    pub rect: Rect,
    /// Clipped interaction bounds, which may be wider than the divider.
    pub hit_rect: Rect,
    /// Whether this control supports any resize input in the current host.
    pub enabled: bool,
    /// Whether the current host allows pointer-drag resizing here.
    pub drag_enabled: bool,
    /// Whether egui routes pointer hover to this control.
    pub hovered: bool,
    /// Keyboard focus, including either axis control of an intersection.
    pub focused: bool,
    /// Whether this exact control owns the active core resize gesture.
    pub dragging: bool,
    /// Resolved theme color. Idle intersections default to transparent.
    pub fill: Color32,
}

impl Splitter {
    /// Paints the default fill using the adapter's clipped painter.
    pub fn paint(&self, painter: &Painter) {
        if self.fill.a() > 0 {
            painter.rect_filled(self.rect, 0.0, self.fill);
        }
    }
}
