//! Ordered execution of the canonical surface paint program.

use super::{receivers::receiver_role, *};

/// Opaque receipt that a host accepted one exact program from beginning to end.
///
/// The receipt is bound to the host-frame attempt, semantic scene, coordinate
/// capture, and transient interaction visuals from which the program was
/// produced. It is not cloneable and must be consumed by
/// [`DockspaceHostFrame::confirm_surface_painted`]. It proves complete emission
/// to the sink, not GPU submission or physical presentation. The host reports
/// the later output disposition through [`super::DockspaceSession::report_surface_presentation`].
#[must_use = "a completed surface program must be confirmed or explicitly discarded"]
pub struct SurfaceProgramExecutionReceipt {
    pub(super) encoding_pass: u64,
    pub(super) surface: SurfaceId,
    pub(super) attempt: HostPresentationAttemptId,
    pub(super) output: SurfacePresentationOutputTicket,
    pub(super) contribution: SurfaceContributionToken,
    pub(super) interaction: HostInteractionPresentation,
}

impl SurfaceProgramExecutionReceipt {
    /// Returns the sole logical surface executed by this receipt.
    #[must_use]
    pub const fn surface(&self) -> SurfaceId {
        self.surface
    }

    pub(in crate::runtime) const fn into_presentation_parts(
        self,
    ) -> (SurfaceContributionToken, HostInteractionPresentation) {
        (self.contribution, self.interaction)
    }
}

impl fmt::Debug for SurfaceProgramExecutionReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceProgramExecutionReceipt")
            .field("surface", &self.surface)
            .finish_non_exhaustive()
    }
}

/// Root presentation kind in the core-owned surface paint program.
#[derive(Debug, Clone, Copy)]
pub enum SurfacePaintProgramRootKind<'frame> {
    /// The surface's ordinary main root.
    Main,
    /// A contained-floating root painted in roster order.
    Contained(ContainedPaintRecord<'frame>),
}

/// One operation in the core-owned surface paint program.
#[derive(Debug, Clone, Copy)]
pub enum SurfacePaintProgramOperation<'frame> {
    /// Paint one pane background and content host.
    Pane(PanePaintRecord<'frame>),
    /// Paint one independent Tabs container background.
    Tabs(TabsPaintRecord<'frame>),
    /// Paint one tab strip.
    TabBar(TabBarPaintRecord<'frame>),
    /// Paint one visible tab.
    Tab(TabPaintRecord<'frame>),
    /// Paint one splitter.
    Splitter(SplitterPaintRecord<'frame>),
    /// Paint one atomic splitter junction.
    SplitterJunction(SplitterJunctionPaintRecord<'frame>),
    /// Paint one tab-strip control.
    TabStripControl(TabStripControlPaintRecord),
    /// Paint the contained-window title and resize controls.
    ContainedControls(ContainedPaintRecord<'frame>),
    /// Paint one root-scoped presentation-menu anchor.
    PresentationMenuAnchor(PresentationMenuAnchorPaintRecord<'frame>),
}

impl SurfacePaintProgramOperation<'_> {
    /// Returns the exact stable visual identity executed by this operation.
    #[must_use]
    pub const fn visual_id(self) -> DockspaceVisualId {
        match self {
            Self::Pane(record) => record.visual_id(),
            Self::Tabs(record) => record.visual_id(),
            Self::TabBar(record) => record.visual_id(),
            Self::Tab(record) => record.visual_id(),
            Self::Splitter(record) => record.visual_id(),
            Self::SplitterJunction(record) => record.visual_id(),
            Self::TabStripControl(record) => record.visual_id(),
            Self::ContainedControls(record) => record.visual_id(),
            Self::PresentationMenuAnchor(record) => record.visual_id(),
        }
    }
}

/// One root header in the core-owned paint program.
#[derive(Debug, Clone, Copy)]
pub struct SurfacePaintProgramRoot<'frame> {
    pub(super) root: RootId,
    pub(super) kind: SurfacePaintProgramRootKind<'frame>,
    pub(super) label: crate::graph::TabLabel<'frame>,
}

impl<'frame> SurfacePaintProgramRoot<'frame> {
    /// Returns the stable root identity.
    #[must_use]
    pub const fn root(&self) -> RootId {
        self.root
    }

    /// Returns whether the root is main or contained-floating.
    #[must_use]
    pub const fn kind(&self) -> SurfacePaintProgramRootKind<'_> {
        self.kind
    }

    /// Returns the contained frame when this is a contained root.
    #[must_use]
    pub const fn contained(&self) -> Option<ContainedPaintRecord<'_>> {
        match self.kind {
            SurfacePaintProgramRootKind::Main => None,
            SurfacePaintProgramRootKind::Contained(record) => Some(record),
        }
    }

    /// Returns this root's own frozen descriptor, never a representative descendant.
    #[must_use]
    pub const fn label(&self) -> crate::graph::TabLabel<'frame> {
        self.label
    }
}

/// One linear step in the core-owned surface paint program.
#[derive(Debug, Clone, Copy)]
pub enum SurfacePaintProgramStep<'frame> {
    /// Starts one main or contained root segment.
    RootBegin {
        /// Core-owned root segment being opened.
        root: SurfacePaintProgramRoot<'frame>,
        /// Paint disposition for contained frame chrome; main roots are always visible.
        paint: SurfacePaintDisposition,
    },
    /// Executes one root-scoped operation.
    Operation {
        /// Core-owned root segment containing the operation.
        root: SurfacePaintProgramRoot<'frame>,
        /// Exact paint operation.
        operation: SurfacePaintProgramOperation<'frame>,
        /// Core-derived paint disposition for this exact operation.
        paint: SurfacePaintDisposition,
    },
    /// Ends the active root segment.
    RootEnd(SurfacePaintProgramRoot<'frame>),
    /// Paints one workspace-global popup backdrop.
    PopupBackdrop(TabListMenuBackdropPaintRecord),
    /// Paints one workspace-global tab-list menu.
    PopupMenu {
        /// Exact popup operation.
        menu: TabListMenuPaintRecord<'frame>,
        /// Core-derived disposition shared by its frame, rows, and scrollbar.
        paint: SurfacePaintDisposition,
    },
    /// Paints the current docking or rehome preview.
    DragPreview(DockspaceDragPreview<'frame>),
    /// Publishes one non-visual drop target through the ordered program.
    DropTarget(DropTargetPaintRecord<'frame>),
    /// Paints one guide cluster in core order.
    DropGuide {
        /// Structural guide identity and geometry compiled into the scene.
        structural: DropGuidePaintRecord<'frame>,
        /// Payload-specific visible guide state for the current drag point.
        affordance: Option<DropAffordanceClusterPaintRecord<'frame>>,
    },
    /// Paints the current contained transform preview.
    ContainedTransformPreview(DockspaceContainedTransformPreview<'frame>),
    /// Paints the current drag ghost before popup projection.
    DragDecoration(DockspaceDragDecoration<'frame>),
    /// Resolves typed interaction feedback after popup input has run.
    InteractionFeedback,
    /// Seals the ordered traversal for this exact semantic output.
    SurfaceEnd {
        /// Candidate output represented by every preceding operation.
        semantic_output: DockspaceSemanticOutput,
    },
}

/// Failure while resolving or executing one compiled paint program.
#[derive(Debug, thiserror::Error)]
pub enum SurfacePaintProgramVisitError<E> {
    /// The sealed program no longer matches its canonical record roster.
    #[error("invalid surface paint program for {surface:?}")]
    InvalidProgram {
        /// Surface whose program failed its internal invariant.
        surface: SurfaceId,
    },
    /// The renderer rejected one emitted step.
    #[error("surface paint sink failed: {0}")]
    Sink(#[source] E),
}

impl<E: Into<DockspaceRuntimeError>> SurfacePaintProgramVisitError<E> {
    /// Converts either failure lane into the renderer-neutral facade error.
    #[doc(hidden)]
    pub fn into_runtime_error(self) -> DockspaceRuntimeError {
        match self {
            Self::InvalidProgram { surface } => {
                DockspaceRuntimeError::surface_program_invariant(surface)
            }
            Self::Sink(error) => error.into(),
        }
    }
}

impl<'frame> SurfacePaintPlan<'frame> {
    /// Visits tab resource requests in their original surface-program order.
    ///
    /// Requests include visible tabs, popup rows, destination feedback, every
    /// future target member and the active drag ghost, including repeated items.
    /// This reads the sealed program without building
    /// interaction descriptors or granting a surface execution receipt.
    ///
    /// # Errors
    ///
    /// Returns an internal runtime error when the sealed program is invalid.
    /// Invalid programs are rejected before the visitor receives any request.
    pub fn visit_tab_resources(
        &self,
        mut visitor: impl FnMut(DockspaceVisualId, TabKey),
    ) -> Result<(), DockspaceRuntimeError> {
        if !self.program_valid {
            return Err(DockspaceRuntimeError::surface_program_invariant(
                self.surface,
            ));
        }
        self.plan
            .visit_surface_program(|step| {
                match step {
                    ResolvedSurfaceProgramStep::Operation {
                        operation: ResolvedSurfaceProgramOperation::Tab(record),
                        ..
                    } => visitor(
                        DockspaceVisualId(VisualIdentity::Tab(*record.id())),
                        record.id().tab,
                    ),
                    ResolvedSurfaceProgramStep::PopupMenu(menu) => {
                        for row in menu.rows() {
                            visitor(
                                DockspaceVisualId(VisualIdentity::Tab(row.tab())),
                                row.tab().tab,
                            );
                        }
                    }
                    ResolvedSurfaceProgramStep::RootBegin(root) => {
                        if let Some(crate::graph::TabLabel::Item(item)) = root.label() {
                            visitor(
                                DockspaceVisualId::from_pane_scene(
                                    crate::presentation::scene::PaneSceneId {
                                        root: root.root(),
                                        item,
                                    },
                                ),
                                TabKey::for_item(item),
                            );
                        }
                    }
                    ResolvedSurfaceProgramStep::DragDecoration => {
                        if let Some(decoration) = self.drag_decoration() {
                            visitor(decoration.source_visual(), decoration.key());
                        }
                    }
                    ResolvedSurfaceProgramStep::DragPreview => {
                        if let Some(preview) = self.drag_preview()
                            && let Some(target) = preview.target()
                        {
                            let (previous, next) = match target.action() {
                                DockspaceDropAction::InsertTab { previous, next, .. } => {
                                    (previous, next)
                                }
                                _ => (None, None),
                            };
                            for item in [target.selected_tab(), previous, next]
                                .into_iter()
                                .flatten()
                            {
                                visitor(target.visual_id(), item);
                            }
                            for tab in preview.tabs() {
                                visitor(target.visual_id(), tab.key());
                            }
                        }
                    }
                    ResolvedSurfaceProgramStep::Operation { .. }
                    | ResolvedSurfaceProgramStep::RootEnd(_)
                    | ResolvedSurfaceProgramStep::PopupBackdrop(_)
                    | ResolvedSurfaceProgramStep::DropTarget(_)
                    | ResolvedSurfaceProgramStep::DropGuide(_)
                    | ResolvedSurfaceProgramStep::ContainedTransformPreview
                    | ResolvedSurfaceProgramStep::InteractionFeedback
                    | ResolvedSurfaceProgramStep::SurfaceEnd => {}
                }
                Ok::<(), std::convert::Infallible>(())
            })
            .map_err(|_| DockspaceRuntimeError::surface_program_invariant(self.surface))
    }

    /// Inspects the single ordered program for this exact surface candidate.
    ///
    /// The visitor receives the core-owned order directly. Official adapters
    /// execute that order without rebuilding a second surface-wide schedule;
    /// test helpers and measurement code may collect a trace without gaining
    /// authority to confirm that a renderer executed the program. Any stale
    /// record identity fails the whole traversal before the visitor sees any
    /// step.
    ///
    /// # Errors
    ///
    /// Returns [`SurfacePaintProgramVisitError::InvalidProgram`] when the
    /// sealed tape no longer matches its canonical record roster, or
    /// [`SurfacePaintProgramVisitError::Sink`] when the renderer rejects a
    /// step.
    pub fn visit_surface_program<E>(
        self,
        visitor: impl FnMut(SurfacePaintProgramStep<'frame>) -> Result<(), E>,
    ) -> Result<(), SurfacePaintProgramVisitError<E>> {
        self.visit_surface_program_inner(visitor)
    }

    /// Executes the single ordered program for this exact surface candidate.
    ///
    /// Unlike [`Self::visit_surface_program`], successful execution returns an
    /// affine receipt which is the only capability accepted by
    /// [`DockspaceHostFrame::confirm_surface_painted`]. Renderer adapters must
    /// use this path only when their sink has accepted every program step.
    /// Accepting owned records for deferred rendering satisfies this contract;
    /// later presentation is a separate host report. Hosts preserve composition
    /// order and clipping when compiling or batching these records.
    ///
    /// # Errors
    ///
    /// Returns [`SurfacePaintProgramVisitError::InvalidProgram`] when the
    /// sealed tape no longer matches its canonical record roster, or
    /// [`SurfacePaintProgramVisitError::Sink`] when the renderer rejects a
    /// step. Neither failure mints a receipt.
    pub fn execute_surface_program<E>(
        self,
        visitor: impl FnMut(SurfacePaintProgramStep<'frame>) -> Result<(), E>,
    ) -> Result<SurfaceProgramExecutionReceipt, SurfacePaintProgramVisitError<E>> {
        self.encoding_started.set(true);
        self.visit_surface_program_inner(visitor)?;
        Ok(SurfaceProgramExecutionReceipt {
            encoding_pass: self.encoding_pass,
            surface: self.surface,
            attempt: self.attempt,
            output: self.output,
            contribution: self.contribution,
            interaction: self.interaction,
        })
    }

    fn visit_surface_program_inner<E>(
        &self,
        mut visitor: impl FnMut(SurfacePaintProgramStep<'frame>) -> Result<(), E>,
    ) -> Result<(), SurfacePaintProgramVisitError<E>> {
        if !self.program_valid {
            return Err(SurfacePaintProgramVisitError::InvalidProgram {
                surface: self.surface,
            });
        }

        let menu_open_bars = self.program_menu_open_bars();
        let mut targets = self.program_link.map(|link| link.targets());
        let mut sink_error = None;
        let traversal = self.plan.visit_surface_program(|step| {
            let target = if matches!(step, ResolvedSurfaceProgramStep::DropTarget(_)) {
                match targets.as_mut() {
                    Some(targets) => Some(targets.next().ok_or(())?),
                    None => None,
                }
            } else {
                None
            };
            let resolved = match self.resolve_surface_program_step(step, &menu_open_bars, target) {
                Ok(Some(step)) => step,
                Ok(None) => return Ok(()),
                Err(()) => return Err(()),
            };
            match visitor(resolved) {
                Ok(()) => Ok(()),
                Err(error) => {
                    sink_error = Some(error);
                    Err(())
                }
            }
        });
        if let Some(error) = sink_error {
            return Err(SurfacePaintProgramVisitError::Sink(error));
        }
        if traversal.is_err() || targets.is_some_and(|mut targets| targets.next().is_some()) {
            return Err(SurfacePaintProgramVisitError::InvalidProgram {
                surface: self.surface,
            });
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn validate_surface_program(&self) -> bool {
        crate::presentation::scene::validate_surface_paint_program(self.plan, self.hit_manifest)
    }

    pub(super) fn resolve_surface_program_step(
        &self,
        step: ResolvedSurfaceProgramStep<'frame>,
        menu_open_bars: &HashSet<TabBarSceneId>,
        linked_target: Option<(usize, LogicalPoint)>,
    ) -> Result<Option<SurfacePaintProgramStep<'frame>>, ()> {
        Ok(match step {
            ResolvedSurfaceProgramStep::RootBegin(root) => {
                let root = self.resolve_surface_program_root(root);
                let paint = root
                    .contained()
                    .map_or(SurfacePaintDisposition::Visible, |record| {
                        paint_disposition(self.drag_decoration, record.visual_id())
                    });
                Some(SurfacePaintProgramStep::RootBegin { root, paint })
            }
            ResolvedSurfaceProgramStep::Operation { root, operation } => {
                let root = self.resolve_surface_program_root(root);
                let operation = self
                    .resolve_surface_program_operation_record(operation, menu_open_bars)
                    .ok_or(())?;
                let paint = paint_disposition(self.drag_decoration, operation.visual_id());
                Some(SurfacePaintProgramStep::Operation {
                    root,
                    operation,
                    paint,
                })
            }
            ResolvedSurfaceProgramStep::RootEnd(root) => Some(SurfacePaintProgramStep::RootEnd(
                self.resolve_surface_program_root(root),
            )),
            ResolvedSurfaceProgramStep::PopupBackdrop(record) => {
                Some(SurfacePaintProgramStep::PopupBackdrop(
                    TabListMenuBackdropPaintRecord::new(self, *record),
                ))
            }
            ResolvedSurfaceProgramStep::PopupMenu(record) => {
                let backdrop = *self
                    .plan
                    .tab_list_menu_backdrop_records()
                    .iter()
                    .find(|backdrop| backdrop.session() == record.session())
                    .ok_or(())?;
                let menu = TabListMenuPaintRecord::new(self, record, backdrop);
                let paint = paint_disposition(self.drag_decoration, menu.visual_id());
                Some(SurfacePaintProgramStep::PopupMenu { menu, paint })
            }
            ResolvedSurfaceProgramStep::DragPreview => self
                .drag_preview()
                .map(SurfacePaintProgramStep::DragPreview),
            ResolvedSurfaceProgramStep::DropTarget(record) => {
                let (region, center) = if let Some((row, center)) = linked_target {
                    (self.hit_manifest.regions().get(row).ok_or(())?, center)
                } else {
                    let region = self
                        .hit_manifest
                        .region_for_kind(PresentationHitRegionKind::DropTarget(record.id()))
                        .ok_or(())?;
                    if region.is_passive()
                        || receiver_role(region.id().kind())
                            != Some(DockspaceReceiverRole::DropTarget)
                    {
                        return Err(());
                    }
                    let center =
                        crate::presentation::scene::paint_receiver_center(region.hit().rect())
                            .ok_or(())?;
                    (region, center)
                };
                Some(SurfacePaintProgramStep::DropTarget(DropTargetPaintRecord {
                    output: self.output,
                    region,
                    center,
                }))
            }
            ResolvedSurfaceProgramStep::DropGuide(record) => {
                let structural = DropGuidePaintRecord { record };
                let affordance = self.drop_affordance().and_then(|affordance| {
                    affordance
                        .clusters()
                        .find(|cluster| cluster.visual_id() == structural.visual_id())
                });
                Some(SurfacePaintProgramStep::DropGuide {
                    structural,
                    affordance,
                })
            }
            ResolvedSurfaceProgramStep::ContainedTransformPreview => self
                .contained_transform_preview()
                .map(SurfacePaintProgramStep::ContainedTransformPreview),
            ResolvedSurfaceProgramStep::DragDecoration => self
                .drag_decoration()
                .map(SurfacePaintProgramStep::DragDecoration),
            ResolvedSurfaceProgramStep::InteractionFeedback => {
                Some(SurfacePaintProgramStep::InteractionFeedback)
            }
            ResolvedSurfaceProgramStep::SurfaceEnd => Some(SurfacePaintProgramStep::SurfaceEnd {
                semantic_output: self.semantic_output(),
            }),
        })
    }

    fn resolve_surface_program_root(
        &self,
        root: ResolvedSurfaceProgramRoot<'frame>,
    ) -> SurfacePaintProgramRoot<'frame> {
        let kind = match root.contained_record() {
            None => SurfacePaintProgramRootKind::Main,
            Some(record) => {
                SurfacePaintProgramRootKind::Contained(ContainedPaintRecord::new(self, record))
            }
        };
        SurfacePaintProgramRoot {
            root: root.root(),
            kind,
            label: root
                .label()
                .unwrap_or(crate::graph::TabLabel::Layout { title: "Layout" }),
        }
    }

    pub(super) fn resolve_surface_program_operation_record(
        &self,
        operation: ResolvedSurfaceProgramOperation<'frame>,
        menu_open_bars: &HashSet<TabBarSceneId>,
    ) -> Option<SurfacePaintProgramOperation<'frame>> {
        Some(match operation {
            ResolvedSurfaceProgramOperation::Pane(record) => {
                SurfacePaintProgramOperation::Pane(PanePaintRecord {
                    record,
                    output: self.semantic_output(),
                })
            }
            ResolvedSurfaceProgramOperation::Tabs(record) => {
                SurfacePaintProgramOperation::Tabs(TabsPaintRecord {
                    record,
                    output: self.semantic_output(),
                })
            }
            ResolvedSurfaceProgramOperation::TabBar(record) => {
                SurfacePaintProgramOperation::TabBar(TabBarPaintRecord::new(self, record))
            }
            ResolvedSurfaceProgramOperation::Tab(record) => {
                SurfacePaintProgramOperation::Tab(TabPaintRecord::new(self, record))
            }
            ResolvedSurfaceProgramOperation::Splitter(record) => {
                SurfacePaintProgramOperation::Splitter(SplitterPaintRecord {
                    record,
                    output: self.semantic_output(),
                })
            }
            ResolvedSurfaceProgramOperation::SplitterJunction(record) => {
                SurfacePaintProgramOperation::SplitterJunction(SplitterJunctionPaintRecord {
                    record,
                    plan: self.plan,
                    output: self.semantic_output(),
                })
            }
            ResolvedSurfaceProgramOperation::TabStripControl(record) => {
                SurfacePaintProgramOperation::TabStripControl(TabStripControlPaintRecord::new(
                    self,
                    *record,
                    menu_open_bars.contains(&record.id().bar()),
                ))
            }
            ResolvedSurfaceProgramOperation::ContainedControls(record) => {
                SurfacePaintProgramOperation::ContainedControls(ContainedPaintRecord::new(
                    self, record,
                ))
            }
            ResolvedSurfaceProgramOperation::PresentationMenuAnchor(record) => {
                let bounds = record.ready_bounds()?;
                SurfacePaintProgramOperation::PresentationMenuAnchor(
                    PresentationMenuAnchorPaintRecord {
                        record,
                        bounds,
                        operable: self.plan.region_is_operable(bounds, record.layer()),
                        scope: ControlScope::from_scene(self.scene),
                    },
                )
            }
        })
    }
}
