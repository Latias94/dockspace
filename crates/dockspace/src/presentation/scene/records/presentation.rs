//! Root-scoped presentation command menu anchor records.

use crate::geometry::LogicalRect;
use crate::ids::{FloatingPresentationId, RootId};
use crate::interaction::targeting::targets::SceneLayerKey;
use crate::presentation::hit::region::HitRegion;

use super::TabBarSceneId;

/// Structural host of one root-scoped presentation command menu anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PresentationMenuAnchorHost {
    /// The sole selected tab bar of a main or native-main root.
    TabBar(TabBarSceneId),
    /// An independent controls band outside a main root's content allocation.
    RootControls,
    /// The title chrome of one contained-floating presentation.
    ContainedTitle(FloatingPresentationId),
}

/// Core-compiled exact draw and hit geometry for one root presentation menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationMenuAnchorRecord {
    root: RootId,
    host: PresentationMenuAnchorHost,
    bounds: Option<LogicalRect>,
    reserved_bounds: Option<LogicalRect>,
    hit: Option<HitRegion>,
    layer: SceneLayerKey,
}

impl PresentationMenuAnchorRecord {
    pub(crate) const fn ready(
        root: RootId,
        host: PresentationMenuAnchorHost,
        bounds: LogicalRect,
        layer: SceneLayerKey,
    ) -> Self {
        Self {
            root,
            host,
            bounds: Some(bounds),
            reserved_bounds: None,
            hit: Some(HitRegion::new(bounds)),
            layer,
        }
    }

    pub(crate) const fn compacted(
        root: RootId,
        host: PresentationMenuAnchorHost,
        layer: SceneLayerKey,
    ) -> Self {
        Self {
            root,
            host,
            bounds: None,
            reserved_bounds: None,
            hit: None,
            layer,
        }
    }

    pub(crate) const fn with_reserved_bounds(mut self, reserved: LogicalRect) -> Self {
        self.reserved_bounds = Some(reserved);
        self
    }

    /// Returns an independent control band's allocation, when this anchor owns one.
    #[must_use]
    pub const fn reserved_bounds(self) -> Option<LogicalRect> {
        self.reserved_bounds
    }

    /// Returns the root whose presentation commands this anchor opens.
    #[must_use]
    pub const fn root(self) -> RootId {
        self.root
    }

    /// Returns the exact structural chrome host selected by core.
    #[must_use]
    pub const fn host(self) -> PresentationMenuAnchorHost {
        self.host
    }

    /// Returns the exact draw bounds reserved by core, or `None` when compacted.
    #[must_use]
    pub const fn ready_bounds(self) -> Option<LogicalRect> {
        self.bounds
    }

    /// Returns the exact half-open interaction region, or `None` when compacted.
    #[must_use]
    pub const fn ready_hit(self) -> Option<HitRegion> {
        self.hit
    }

    /// Returns whether this root currently publishes usable menu chrome.
    #[must_use]
    #[cfg(test)]
    pub const fn is_ready(self) -> bool {
        self.bounds.is_some()
    }

    /// Returns the roster-derived semantic layer.
    #[must_use]
    pub const fn layer(self) -> SceneLayerKey {
        self.layer
    }
}
