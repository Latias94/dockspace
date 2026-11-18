use thiserror::Error;

use crate::geometry::{LogicalRect, PhysicalRect};
use crate::ids::{EngineAuthorityDomainId, ItemId, RootId, SurfaceId, TabKey};

use super::WorkspaceVersion;

/// Refusal of an incremental logical-surface membership operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalSurfaceRejection {
    /// The requested surface already belongs to the current workspace.
    SurfaceAlreadyAttached,
    /// Another presentation currently owns this root identity.
    RootAlreadyOwned { root: RootId },
    /// The requested surface is absent.
    SurfaceUnavailable,
    /// Main or contained content must be closed or migrated first.
    SurfaceNotEmpty { items: usize },
    /// An exact native binding still owns this logical surface.
    NativeBindingPresent,
    /// A pending mutation, native creation or recovery references the request.
    RecoveryConflict,
    /// The current policy disallows changing the arrangement.
    PolicyDenied,
}

/// Center-docking intent for a captured whole tab group.
///
/// This changes the logical insertion only; it never replaces the captured
/// source, native reservation, release event or window lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TabGroupDockMode {
    /// Insert the group's direct children into the destination in their order.
    #[default]
    MergeTabs,
    /// Insert the complete group as one layout tab.
    PreserveLayout,
}

/// Stable item or central-region anchor used by product docking actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DockAnchor {
    /// The immediate inline Tabs owner, or the bare Pane itself.
    Item(ItemId),
    /// The central Tabs anchor declared by this root.
    Central(RootId),
    /// One currently live layout node; this identity alone does not grant authority.
    Node(crate::ids::NodeKey),
}

/// Physical side of a stable docking anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockEdge {
    /// Insert before the target along the horizontal axis.
    Left,
    /// Insert after the target along the horizontal axis.
    Right,
    /// Insert before the target along the vertical axis.
    Top,
    /// Insert after the target along the vertical axis.
    Bottom,
}

/// Payload share assigned by one edge docking action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockFraction(f64);

impl DockFraction {
    /// Creates a finite fraction strictly between zero and one.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDockFraction`] when `value` is non-finite or outside the open unit range.
    pub fn new(value: f64) -> Result<Self, InvalidDockFraction> {
        if value.is_finite() && value > 0.0 && value < 1.0 {
            Ok(Self(value))
        } else {
            Err(InvalidDockFraction { value })
        }
    }

    /// Returns the validated scalar.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// Failure to construct a [`DockFraction`].
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[error("dock fraction must be finite and strictly between zero and one, got {value}")]
pub struct InvalidDockFraction {
    /// Rejected scalar.
    pub value: f64,
}

/// Stable product placement for opening or moving one item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockPlacement {
    /// Insert into the Tabs container or bare Pane represented by the anchor.
    ///
    /// Moving an item within its current group does not implicitly reorder it to the end.
    Center(DockAnchor),
    /// Insert immediately before the named inline tab in its current group.
    Before(ItemId),
    /// Insert immediately after the named inline tab in its current group.
    After(ItemId),
    /// Split the anchor's real layout node at one branch-local edge.
    InnerEdge {
        /// Stable content, node or central-region anchor.
        anchor: DockAnchor,
        /// Insertion side.
        edge: DockEdge,
        /// Share assigned to the opened or moved item.
        fraction: DockFraction,
    },
    /// Split the complete target root at its outer boundary.
    OuterEdge {
        /// Stable target root.
        root: RootId,
        /// Insertion side.
        edge: DockEdge,
        /// Share assigned to the opened or moved item.
        fraction: DockFraction,
    },
    /// Make an existing rootless surface own the item as its main presentation.
    ///
    /// Opening new content or moving part of a root allocates a new root. Moving
    /// the complete source root preserves that root's stable identity.
    Main(SurfaceId),
}

/// Explicit outer-window placement for one programmatic native tear-off.
///
/// This is an application request rather than observed platform authority. The
/// native host may reject or adjust it, and must report the resulting exact
/// window facts through the normal managed-native lifecycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeWindowPlacement {
    outer_rect: PhysicalRect,
}

impl NativeWindowPlacement {
    /// Creates one native-window placement request from validated physical bounds.
    #[must_use]
    pub const fn new(outer_rect: PhysicalRect) -> Self {
        Self { outer_rect }
    }

    /// Returns the requested desktop-physical outer bounds.
    #[must_use]
    pub const fn outer_rect(self) -> PhysicalRect {
        self.outer_rect
    }
}

/// One exact tabs group captured from a published dockspace session.
///
/// The capability freezes its source membership, order, selection and workspace
/// version. Moving a former member into another group never retargets this
/// value. Runtime graph identities remain private.
/// Cloning preserves the same captured group and version; it does not refresh
/// the capability against a changed workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTabsGroup {
    pub(crate) authority_domain: EngineAuthorityDomainId,
    pub(crate) expected: WorkspaceVersion,
    pub(crate) source: crate::graph::command::NodeSource,
}

impl PreparedTabsGroup {
    /// Returns the published version from which the complete group was captured.
    #[must_use]
    pub const fn expected_version(&self) -> WorkspaceVersion {
        self.expected
    }
}

/// One direct tab captured against its session, membership and revision.
///
/// A layout tab denotes its complete subtree. Content identities remain ItemId.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTab {
    pub(crate) authority_domain: EngineAuthorityDomainId,
    pub(crate) expected: WorkspaceVersion,
    pub(crate) source: crate::graph::command::TabSource,
}

impl PreparedTab {
    /// Returns the exact direct child represented by this capability.
    #[must_use]
    pub const fn key(&self) -> TabKey {
        self.source.tab()
    }

    /// Returns the workspace revision from which the tab was captured.
    #[must_use]
    pub const fn expected_version(&self) -> WorkspaceVersion {
        self.expected
    }
}

/// Existing content moved by a programmatic docking command.
///
/// Captured tabs and groups retain their original session and revision for every
/// destination. A root denotes its complete layout rather than a representative pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockMoveSource {
    /// Move one open item, leaving the rest of its group in place.
    Item(ItemId),
    /// Move one previously captured complete tabs group.
    Tabs(PreparedTabsGroup),
    /// Move one exact direct tab, preserving an entire layout subtree.
    Tab(PreparedTab),
    /// Move one complete root and all its layout descendants.
    ///
    /// Rehoming into contained, native or rootless main presentation retains the
    /// root identity. Docking into an existing layout may absorb it into the
    /// destination root; the committed outcome identifies both roots.
    Root(RootId),
}

/// Destination for existing content, shared by menus and programmatic commands.
///
/// These values request a semantic move; they do not confer pointer, native-window,
/// or presentation authority. Native transfers retain their source until first-live.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockMoveTarget {
    /// Dock into an existing layout or a rootless logical surface.
    Dock {
        /// Stable destination in the source's prepared workspace version.
        placement: DockPlacement,
        /// Center insertion of a captured group or a complete Tabs root.
        ///
        /// Item and direct-tab sources retain their exact scope. Edge placements
        /// and Pane/Split/Grid roots always retain their structural behavior.
        mode: TabGroupDockMode,
    },
    /// Float inside an existing logical surface using current core geometry.
    Contained {
        /// Logical surface that will contain the floating presentation.
        surface: SurfaceId,
    },
    /// Begin transfer to a newly managed native window.
    Native {
        /// Explicit desktop bounds, or core-derived placement from current facts.
        placement: Option<NativeWindowPlacement>,
    },
}

/// Product-visible result of one accepted semantic docking action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockspaceActionOutcome {
    /// One exact direct tab was moved without widening its source scope.
    TabDocked {
        /// Direct child captured by the action.
        tab: TabKey,
        /// Root which originally owned the tab.
        source_root: RootId,
        /// Root which owns the resulting tab or split subtree.
        target_root: RootId,
        /// Every real descendant, in visual order.
        items: Vec<ItemId>,
        /// Whether durable state changed.
        changed: bool,
    },
    /// One exact direct tab entered a contained floating presentation.
    TabFloated {
        /// Direct child captured by the action.
        tab: TabKey,
        /// Original owner.
        source_root: RootId,
        /// Resulting root.
        root: RootId,
        /// Logical hosting surface.
        surface: SurfaceId,
        /// Contained presentation identity.
        floating: crate::ids::FloatingPresentationId,
        /// Every real descendant, in visual order.
        items: Vec<ItemId>,
        /// Whether durable state changed.
        changed: bool,
    },
    /// Selection was checked and optionally changed.
    Selected {
        /// Selected direct tab child.
        tab: TabKey,
        /// Whether durable selection or MRU state changed.
        changed: bool,
    },
    /// A real content item was activated through all of its containing tab groups.
    ItemActivated {
        /// Application content whose ancestor path was activated.
        item: ItemId,
        /// Whether any durable selection or MRU state changed.
        changed: bool,
    },
    /// One owned item was shown or hidden without changing its docking placement.
    ItemVisibilityChanged {
        /// Item whose visibility was checked.
        item: ItemId,
        /// Whether the item is now visible.
        visible: bool,
        /// Whether durable visibility state changed.
        changed: bool,
    },
    /// A new item was opened into one root.
    Opened {
        /// Opened item.
        item: ItemId,
        /// Root which now owns the item.
        root: RootId,
    },
    /// The requested item was already open and was deliberately left in place.
    Existing {
        /// Existing item.
        item: ItemId,
    },
    /// An existing item was docked into another semantic placement.
    Docked {
        /// Moved item.
        item: ItemId,
        /// Root which owned the item before the action.
        source_root: RootId,
        /// Root which owns the item after the action.
        target_root: RootId,
        /// Whether topology or tab order changed.
        changed: bool,
    },
    /// A captured complete tabs group was docked as one atomic payload.
    TabsDocked {
        /// Root which owned the group before the action.
        source_root: RootId,
        /// Root which owns the group after the action.
        target_root: RootId,
        /// Items moved in their captured tab order.
        items: Vec<ItemId>,
        /// Whether topology, presentation ownership, or tab order changed.
        changed: bool,
    },
    /// A complete root was merged into another placement or rehomed as one unit.
    RootDocked {
        /// Stable root moved as the payload authority.
        root: RootId,
        /// Root which owns the payload after the action.
        target_root: RootId,
        /// Stable items moved with the complete root in traversal order.
        items: Vec<ItemId>,
        /// Whether topology, presentation ownership, or tab order changed.
        changed: bool,
    },
    /// One item was moved into or repositioned within a contained presentation.
    Floated {
        /// Stable item used to address the product action.
        item: ItemId,
        /// Stable root which owns the resulting contained presentation.
        root: RootId,
        /// Surface which owns the contained presentation.
        surface: SurfaceId,
        /// Stable contained-presentation identity.
        floating: crate::ids::FloatingPresentationId,
        /// Whether topology, geometry, or stacking changed.
        changed: bool,
    },
    /// A captured complete tabs group entered a contained presentation.
    TabsFloated {
        /// Root which owned the group before the action.
        source_root: RootId,
        /// Root which owns the resulting contained presentation.
        root: RootId,
        /// Surface which owns the contained presentation.
        surface: SurfaceId,
        /// Resulting contained-presentation identity.
        floating: crate::ids::FloatingPresentationId,
        /// Items moved in their captured tab order.
        items: Vec<ItemId>,
        /// Whether topology, geometry, or stacking changed.
        changed: bool,
    },
    /// A complete root was moved into a contained presentation as one unit.
    RootFloated {
        /// Stable root moved as the payload authority.
        root: RootId,
        /// Surface which owns the contained presentation.
        surface: SurfaceId,
        /// Stable contained-presentation identity.
        floating: crate::ids::FloatingPresentationId,
        /// Stable items moved with the complete root in traversal order.
        items: Vec<ItemId>,
        /// Whether presentation ownership, geometry, or stacking changed.
        changed: bool,
    },
    /// A complete native root retained source ownership while contained admission was staged.
    RootFloatRequested {
        /// Stable root retained by the pending transfer.
        root: RootId,
        /// Native surface which remains authoritative until admission.
        source_surface: SurfaceId,
        /// Surface which must present the exact contained staging output.
        target_surface: SurfaceId,
        /// Stable items retained by the pending root transfer.
        items: Vec<ItemId>,
    },
    /// A complete native root retained source ownership while dock admission was staged.
    RootDockRequested {
        /// Stable root retained by the pending transfer.
        root: RootId,
        /// Native surface which remains authoritative until admission.
        source_surface: SurfaceId,
        /// Root which will own the payload after the presented transfer.
        target_root: RootId,
        /// Stable items retained by the pending root transfer.
        items: Vec<ItemId>,
    },
    /// One contained presentation's durable bounds were checked and optionally changed.
    ContainedBoundsUpdated {
        /// Stable root presented by the contained window.
        root: RootId,
        /// Surface which owns the contained presentation.
        surface: SurfaceId,
        /// Stable contained-presentation identity.
        floating: crate::ids::FloatingPresentationId,
        /// Whether durable bounds changed.
        changed: bool,
    },
    /// One contained presentation was checked and optionally raised to the front.
    ContainedRaised {
        /// Stable root presented by the contained window.
        root: RootId,
        /// Surface which owns the contained presentation.
        surface: SurfaceId,
        /// Stable contained-presentation identity.
        floating: crate::ids::FloatingPresentationId,
        /// Whether stacking order changed.
        changed: bool,
    },
    /// A payload entered the native create lifecycle without moving its content yet.
    NativeRootTearOffRequested {
        /// Destination root reserved for exact first-live presentation.
        ///
        /// Complete-root payloads retain their identity; partial payloads reserve a new root.
        root: RootId,
        /// Surface currently presenting the root.
        source_surface: SurfaceId,
        /// Fresh logical surface reserved for the native child.
        target_surface: SurfaceId,
        /// Stable items retained by the pending root transfer.
        items: Vec<ItemId>,
    },
}

impl DockspaceActionOutcome {
    /// Returns whether this outcome changed durable workspace state.
    #[must_use]
    pub const fn changed(&self) -> bool {
        match self {
            Self::TabDocked { changed, .. }
            | Self::TabFloated { changed, .. }
            | Self::Selected { changed, .. }
            | Self::ItemActivated { changed, .. }
            | Self::ItemVisibilityChanged { changed, .. }
            | Self::Docked { changed, .. }
            | Self::TabsDocked { changed, .. }
            | Self::RootDocked { changed, .. }
            | Self::Floated { changed, .. }
            | Self::TabsFloated { changed, .. }
            | Self::RootFloated { changed, .. }
            | Self::ContainedBoundsUpdated { changed, .. }
            | Self::ContainedRaised { changed, .. } => *changed,
            Self::Opened { .. } => true,
            Self::Existing { .. }
            | Self::RootFloatRequested { .. }
            | Self::RootDockRequested { .. }
            | Self::NativeRootTearOffRequested { .. } => false,
        }
    }
}

/// Stable, actionable rejection category for one product item action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DockspaceActionRejection {
    /// The item to select or move is not currently open.
    #[error("item {item} is not open")]
    ItemUnavailable {
        /// Missing item.
        item: ItemId,
    },
    /// The item must be shown before it can be selected.
    #[error("item {item} is hidden")]
    ItemHidden {
        /// Hidden item.
        item: ItemId,
    },
    /// The item is open but its root is not presented as contained floating content.
    #[error("item {item} is not in a contained presentation")]
    ItemNotContained {
        /// Item whose containing presentation was requested.
        item: ItemId,
    },
    /// The stable target no longer names an available tabs or root anchor.
    #[error("dock anchor {anchor:?} is unavailable")]
    AnchorUnavailable {
        /// Missing anchor.
        anchor: DockAnchor,
    },
    /// The stable target root is unavailable.
    #[error("dock root {root} is unavailable")]
    RootUnavailable {
        /// Missing root.
        root: RootId,
    },
    /// The root has no core-derived destination for a default dock-back action.
    #[error("dock root {root} has no available default dock-back destination")]
    DockBackUnavailable {
        /// Root whose current presentation cannot be docked back automatically.
        root: RootId,
    },
    /// The requested rootless main-surface destination is unavailable.
    #[error("surface {surface} cannot accept a main root")]
    MainSurfaceUnavailable {
        /// Missing or occupied surface.
        surface: SurfaceId,
    },
    /// The requested surface is unavailable for contained presentation.
    #[error("surface {surface} cannot accept contained content")]
    SurfaceUnavailable {
        /// Missing target surface.
        surface: SurfaceId,
    },
    /// Current policy rejects the requested source, target, or mutation class.
    #[error("current docking policy rejects the action")]
    PolicyDenied,
    /// Current topology changed or otherwise conflicts with the requested action.
    #[error("current docking topology conflicts with the action")]
    Conflict,
    /// No fresh stable presentation identity remains available.
    #[error("stable presentation identity space is exhausted")]
    IdentityExhausted,
    /// The managed native host cannot currently create a child surface.
    #[error("native child-window creation is unavailable")]
    NativeUnavailable,
    /// The source surface has no exact current presentation geometry.
    #[error("surface {surface} has no current presentation geometry")]
    PresentationUnavailable {
        /// Source surface lacking exact presentation geometry.
        surface: SurfaceId,
    },
}

/// Session- and revision-bound product docking action.
///
/// A prepared action preserves the exact published workspace version from
/// which its stable placement was derived. The core validates its opaque
/// authority domain before turning it into reducer input.
/// Preparing or dropping the value does not mutate the workspace. Submission
/// consumes it; it deliberately implements neither `Clone` nor `Copy`.
#[derive(Debug)]
#[must_use = "a prepared docking action must be submitted or deliberately discarded"]
pub struct PreparedDockAction {
    authority_domain: EngineAuthorityDomainId,
    expected: WorkspaceVersion,
    action: ProductAction,
}

impl PreparedDockAction {
    pub(crate) const fn new(
        authority_domain: EngineAuthorityDomainId,
        expected: WorkspaceVersion,
        action: ProductAction,
    ) -> Self {
        Self {
            authority_domain,
            expected,
            action,
        }
    }

    /// Returns the published workspace version from which this action was prepared.
    #[must_use]
    pub const fn expected_version(&self) -> WorkspaceVersion {
        self.expected
    }

    pub(crate) const fn parts(
        &self,
    ) -> (EngineAuthorityDomainId, WorkspaceVersion, &ProductAction) {
        (self.authority_domain, self.expected, &self.action)
    }

    pub(crate) fn into_parts(self) -> (EngineAuthorityDomainId, WorkspaceVersion, ProductAction) {
        (self.authority_domain, self.expected, self.action)
    }
}

/// A prepared product action was submitted to a different dockspace session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("prepared docking action belongs to another dockspace authority domain")]
pub(crate) struct PreparedDockActionAuthorityMismatch;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProductAction {
    SelectItem {
        item: ItemId,
    },
    SetItemVisibility {
        item: ItemId,
        visible: bool,
    },
    OpenItem {
        item: ItemId,
        placement: DockPlacement,
    },
    DockItem {
        item: ItemId,
        placement: DockPlacement,
    },
    DockTab {
        tab: PreparedTab,
        placement: DockPlacement,
    },
    FloatTab {
        tab: PreparedTab,
        surface: SurfaceId,
    },
    DockTabs {
        group: PreparedTabsGroup,
        placement: DockPlacement,
        mode: TabGroupDockMode,
    },
    DockRoot {
        root: RootId,
        placement: DockPlacement,
        mode: TabGroupDockMode,
    },
    DockBackRoot {
        root: RootId,
    },
    TearOffNative {
        source: DockMoveSource,
        placement: Option<NativeWindowPlacement>,
    },
    FloatRoot {
        root: RootId,
        surface: SurfaceId,
        rect: Option<LogicalRect>,
    },
    FloatItem {
        item: ItemId,
        surface: SurfaceId,
    },
    FloatTabs {
        group: PreparedTabsGroup,
        surface: SurfaceId,
    },
    RaiseContained {
        item: ItemId,
    },
    BringContainedIntoView {
        item: ItemId,
    },
}
