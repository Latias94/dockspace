use super::*;

/// Derives the complete structural measurement inventory from core-owned topology.
pub(crate) fn derive_scene_requirement_draft(
    authority_domain: EngineAuthorityDomainId,
    workspace: &Workspace,
    workspace_version: WorkspaceVersion,
    config: PresentationConfigRevision,
    policy: &DockPolicySnapshot,
    revision: RequirementRevision,
    surface_revisions: &BTreeMap<SurfaceId, SurfaceRequirementRevision>,
) -> Result<SceneRequirementDraft, SceneRequirementDerivationError> {
    let workspace_index = Arc::new(
        WorkspaceIndex::build(workspace, workspace_version)
            .map_err(|source| SceneRequirementDerivationError::WorkspaceIndex { source })?,
    );
    derive_scene_requirement_draft_with_index(
        authority_domain,
        workspace,
        workspace_version,
        workspace_index,
        config,
        policy,
        revision,
        surface_revisions,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_scene_requirement_draft_with_index(
    authority_domain: EngineAuthorityDomainId,
    workspace: &Workspace,
    workspace_version: WorkspaceVersion,
    workspace_index: Arc<WorkspaceIndex>,
    config: PresentationConfigRevision,
    policy: &DockPolicySnapshot,
    revision: RequirementRevision,
    surface_revisions: &BTreeMap<SurfaceId, SurfaceRequirementRevision>,
) -> Result<SceneRequirementDraft, SceneRequirementDerivationError> {
    if workspace_index.version() != workspace_version {
        return Err(
            SceneRequirementDerivationError::WorkspaceIndexVersionMismatch {
                expected: workspace_version,
                actual: workspace_index.version(),
            },
        );
    }
    if let Some((surface, _)) = workspace
        .surfaces()
        .find(|(surface, _)| !surface_revisions.contains_key(surface))
    {
        return Err(SceneRequirementDerivationError::MissingSurfaceRequirementRevision { surface });
    }
    let mut surfaces = BTreeMap::new();
    for (surface, presentation) in workspace.surfaces() {
        let mut pane_sizings = BTreeSet::new();
        let mut tab_intrinsics = BTreeSet::new();
        let mut tab_strips = BTreeSet::new();
        let mut tab_bars = BTreeMap::new();
        let mut roots = Vec::with_capacity(presentation.contained.len().saturating_add(1));
        roots.extend(presentation.main_root);
        for floating in presentation.contained.iter().copied() {
            let root = workspace
                .contained_floating(floating)
                .ok_or(
                    SceneRequirementDerivationError::MissingContainedPresentation {
                        surface,
                        floating,
                    },
                )?
                .root;
            roots.push(root);
        }

        for root in roots {
            derive_root_requirements(
                workspace,
                &workspace_index,
                workspace_version,
                policy,
                surface,
                root,
                &mut pane_sizings,
                &mut tab_intrinsics,
                &mut tab_strips,
                &mut tab_bars,
            )?;
        }

        let surface_revision = *surface_revisions.get(&surface).ok_or(
            SceneRequirementDerivationError::MissingSurfaceRequirementRevision { surface },
        )?;
        let ticket = SurfaceMeasurementTicket::new(
            authority_domain,
            workspace_version.epoch(),
            config,
            policy.revision(),
            surface_revision,
            surface,
        );
        let requirements =
            SurfaceRequirements::new(ticket, pane_sizings, tab_intrinsics, tab_strips, tab_bars)
                .map_err(SceneRequirementDerivationError::Requirement)?;
        surfaces.insert(surface, requirements);
    }

    SceneRequirementDraft::new(
        authority_domain,
        workspace_version,
        workspace_index,
        config,
        policy.revision(),
        revision,
        surfaces,
    )
    .map_err(SceneRequirementDerivationError::Manifest)
}
fn derive_root_requirements(
    workspace: &Workspace,
    workspace_index: &WorkspaceIndex,
    workspace_version: WorkspaceVersion,
    policy: &DockPolicySnapshot,
    surface: SurfaceId,
    root: RootId,
    pane_sizings: &mut BTreeSet<PaneSizingKey>,
    tab_intrinsics: &mut BTreeSet<TabIntrinsicKey>,
    tab_strips: &mut BTreeSet<TabStripKey>,
    tab_bars: &mut BTreeMap<TabBarSceneId, TabBarRequirement>,
) -> Result<(), SceneRequirementDerivationError> {
    let root_node = workspace
        .root(root)
        .ok_or(SceneRequirementDerivationError::MissingRoot { surface, root })?
        .node;
    let mut pending = vec![root_node];
    let mut visited = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !visited.insert(node) {
            return Err(SceneRequirementDerivationError::RepeatedNode {
                surface,
                root,
                node,
            });
        }
        match workspace
            .node(node)
            .ok_or(SceneRequirementDerivationError::MissingNode {
                surface,
                root,
                node,
            })? {
            Node::Pane { item } => {
                if !workspace.item_is_hidden(*item) {
                    pane_sizings.insert(PaneSizingKey::new(root, *item));
                }
            }
            Node::Tabs { items, .. } => {
                if !workspace.node_has_visible_content(node) {
                    continue;
                }
                let visible_items = workspace.visible_tab_keys(node);
                pane_sizings.extend(
                    visible_items
                        .iter()
                        .filter_map(|tab| tab.item())
                        .map(|item| PaneSizingKey::new(root, item)),
                );
                pending.extend(items.iter().rev().filter_map(|tab| tab.layout()));
                let target = workspace_index
                    .capture_tab_target(workspace, workspace_version, root, node)
                    .map_err(
                        |_| SceneRequirementDerivationError::TabBarTargetCaptureFailed {
                            surface,
                            root,
                            tabs: node,
                        },
                    )?;
                if target.surface() != surface {
                    return Err(
                        SceneRequirementDerivationError::TabBarTargetSurfaceMismatch {
                            root,
                            tabs: node,
                            expected: surface,
                            actual: target.surface(),
                        },
                    );
                }
                let target_rule = Some(target.rule());
                let id = TabBarSceneId { root, tabs: node };
                let tab_bar_policy = policy.tab_bar_declaration(surface, target_rule);
                // A contained title can move this whole root even when its local
                // tab strip is hidden or interaction-disabled.
                let prospective =
                    (!visible_items.is_empty()).then_some(crate::ids::TabKey::for_layout(node));
                let mut labels = visible_items
                    .iter()
                    .copied()
                    .filter_map(|tab| {
                        Some((
                            tab,
                            crate::presentation::scene::FrozenTabLabel::from_node(
                                workspace.node(tab.layout()?)?,
                            ),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                if let Some(tab) = prospective {
                    labels.insert(
                        tab,
                        crate::presentation::scene::FrozenTabLabel::from_node(
                            workspace.node(node).ok_or(
                                SceneRequirementDerivationError::MissingNode {
                                    surface,
                                    root,
                                    node,
                                },
                            )?,
                        ),
                    );
                    tab_intrinsics.insert(TabIntrinsicKey::new(
                        surface,
                        TabSceneId {
                            root,
                            tabs: node,
                            tab,
                        },
                    ));
                }
                tab_bars.insert(
                    id,
                    TabBarRequirement::new(
                        id,
                        tab_bar_policy,
                        visible_items
                            .iter()
                            .copied()
                            .map(|tab| {
                                let capability = tab_close_capability(workspace, policy, root, tab);
                                (tab, capability)
                            })
                            .collect(),
                    )
                    .with_layout_labels(labels),
                );
                // Auto-hidden bars retain the measurements required to reveal
                // them and to preview a merge before its first visible frame.
                if tab_bar_policy.visibility() != TabBarVisibility::Hidden {
                    tab_strips.insert(TabStripKey::new(surface, id));
                }
                if tab_bar_policy.visibility() != TabBarVisibility::Hidden
                    || tab_bar_policy.interaction() == TabBarInteraction::Enabled
                {
                    tab_intrinsics.extend(visible_items.iter().copied().map(|item| {
                        TabIntrinsicKey::new(
                            surface,
                            TabSceneId {
                                root,
                                tabs: node,
                                tab: item,
                            },
                        )
                    }));
                }
            }
            Node::Split { children, .. } | Node::Grid { children, .. } => {
                pending.extend(children.iter().rev().copied());
            }
        }
    }
    Ok(())
}

/// Resolves close chrome for one complete direct child without choosing a representative item.
pub(crate) fn tab_close_capability(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    root: RootId,
    tab: crate::ids::TabKey,
) -> crate::policy::CloseCapability {
    use crate::policy::CloseCapability;
    if let Some(item) = tab.item() {
        return policy.pane_close_capability(item);
    }
    if workspace.layout_tab_contains_central(root, tab) {
        return CloseCapability::Disabled;
    }
    let Some(node) = tab.layout() else {
        return CloseCapability::Disabled;
    };
    let items = workspace.collect_items_in_subtree(node);
    if items.is_empty() {
        return CloseCapability::Disabled;
    }
    let mut aggregate = CloseCapability::DeferredAllowed;
    for item in items {
        match policy.pane_close_capability(item) {
            CloseCapability::Disabled => return CloseCapability::Disabled,
            CloseCapability::Immediate => aggregate = CloseCapability::Immediate,
            CloseCapability::DeferredAllowed => {}
        }
    }
    aggregate
}

/// Failure to derive an exact semantic measurement inventory.
#[derive(Debug, Clone, PartialEq, Error)]
pub(crate) enum SceneRequirementDerivationError {
    /// The manifest cannot issue a new semantic requirement revision.
    #[error("aggregate requirement revision is exhausted")]
    RequirementRevisionExhausted,
    /// A caller attempted to reuse an index outside its exact workspace revision.
    #[error("workspace index version {actual:?} does not match {expected:?}")]
    WorkspaceIndexVersionMismatch {
        /// Exact workspace version being derived.
        expected: WorkspaceVersion,
        /// Exact workspace version frozen into the index.
        actual: WorkspaceVersion,
    },
    /// The exact revision-bound structural index could not be constructed.
    #[error("workspace index construction failed: {source}")]
    WorkspaceIndex {
        /// Structural or presentation invariant rejected by index construction.
        #[source]
        source: CommandError,
    },
    /// A workspace surface had no independently tracked requirement revision.
    #[error("surface {surface} has no requirement revision")]
    MissingSurfaceRequirementRevision {
        /// Surface omitted by the revision inventory.
        surface: SurfaceId,
    },
    /// A surface roster named a missing contained presentation.
    #[error("surface {surface} references missing contained presentation {floating}")]
    MissingContainedPresentation {
        /// Owning surface.
        surface: SurfaceId,
        /// Missing contained presentation.
        floating: FloatingPresentationId,
    },
    /// A surface-owned root record was absent.
    #[error("surface {surface} references missing root {root}")]
    MissingRoot {
        /// Owning surface.
        surface: SurfaceId,
        /// Missing root.
        root: RootId,
    },
    /// A reachable graph node was absent.
    #[error("root {root} on surface {surface} references missing node {node:?}")]
    MissingNode {
        /// Owning surface.
        surface: SurfaceId,
        /// Owning root.
        root: RootId,
        /// Missing node.
        node: NodeId,
    },
    /// A node was reached twice, indicating a cycle or shared subtree.
    #[error("root {root} on surface {surface} reaches node {node:?} more than once")]
    RepeatedNode {
        /// Owning surface.
        surface: SurfaceId,
        /// Owning root.
        root: RootId,
        /// Repeated node.
        node: NodeId,
    },
    /// A captured pane-local target disagreed with its roster surface.
    #[error(
        "tab bar for root {root} tabs {tabs:?} belongs to surface {actual}, expected {expected}"
    )]
    TabBarTargetSurfaceMismatch {
        /// Owning root.
        root: RootId,
        /// Tabs leaf whose target facts were incoherent.
        tabs: NodeId,
        /// Surface currently being derived.
        expected: SurfaceId,
        /// Surface frozen by the target capture.
        actual: SurfaceId,
    },
    /// A selected tabs leaf could not produce authoritative target facts.
    #[error(
        "selected tab bar for root {root} tabs {tabs:?} on surface {surface} cannot be captured"
    )]
    TabBarTargetCaptureFailed {
        /// Surface being derived.
        surface: SurfaceId,
        /// Owning root.
        root: RootId,
        /// Selected tabs leaf.
        tabs: NodeId,
    },
    /// A derived per-surface requirement was internally incoherent.
    #[error(transparent)]
    Requirement(#[from] RequirementBuildError),
    /// The aggregate exact surface inventory was internally incoherent.
    #[error(transparent)]
    Manifest(#[from] ManifestBuildError),
}
