//! Shared compilation of exact window geometry and visibility observations.

use super::{NativeWindowFacts, NativeWindowPresentationState};
use crate::geometry::{PhysicalRect, PhysicalSize, ScaleFactor};

/// Client geometry observed by a window backend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeWindowClientGeometry {
    /// The client extent without a reliable desktop origin.
    Size(PhysicalSize),
    /// The complete client rectangle in physical desktop pixels.
    Bounds(PhysicalRect),
}

/// Independent geometry and visibility observations from one window sample.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct NativeWindowFactsSample {
    /// Observed client geometry.
    pub client: Option<NativeWindowClientGeometry>,
    /// Observed outer frame in physical desktop pixels.
    pub outer_bounds: Option<PhysicalRect>,
    /// The OS scale, used for native window geometry.
    pub native_scale: Option<ScaleFactor>,
    /// The renderer scale, including any application zoom.
    pub presentation_scale: Option<ScaleFactor>,
    /// Whether the OS reports this window as visible.
    pub visible: Option<bool>,
    /// Whether the OS reports this window as minimized.
    pub minimized: Option<bool>,
}

/// Compiles one live window sample without guessing independent properties.
///
/// Some platforms report visibility without a minimized state. A positive
/// visibility observation alone leaves desktop routing authority unknown.
/// Hosts can add exact local readiness and input facts to the result.
pub fn compile_native_window_facts(sample: NativeWindowFactsSample) -> NativeWindowFacts {
    let mut facts = NativeWindowFacts::live();
    if let Some(client) = sample.client {
        facts = match client {
            NativeWindowClientGeometry::Size(size) => facts.with_content_size(size),
            NativeWindowClientGeometry::Bounds(bounds) => facts
                .with_content_size(bounds.size())
                .with_content_bounds(bounds),
        };
    }
    if let Some(bounds) = sample.outer_bounds {
        facts = facts.with_outer_bounds(bounds);
    }
    if let Some(scale) = sample.native_scale {
        facts = facts.with_native_scale_factor(scale);
    }
    if let Some(scale) = sample.presentation_scale {
        facts = facts.with_presentation_scale_factor(scale);
    }
    let presentation = match (sample.visible, sample.minimized) {
        (_, Some(true)) => NativeWindowPresentationState::Minimized,
        (Some(false), _) => NativeWindowPresentationState::Hidden,
        (Some(true), Some(false)) => NativeWindowPresentationState::Visible,
        _ => return facts,
    };
    facts.with_presentation(presentation)
}
