# Custom UI host

Read **Engine host integration** in `crates/dockspace/docs/guide.md`,
`docs/ADAPTER_TESTING.md`, and
`integration/dockspace-host-harness/examples/engine_host.rs`.

## Commit, render and present

Own one `DockspaceSession` and follow four phases:

1. **Admit input.** Submit ordered input and semantic commands through its host
   frame. For a command batch, retain each `DockspaceSubmittedAction` and resolve
   it against that frame's `HostFrameReport::submitted_action_outcome`.
2. **Prepare and encode.** Use `DockspaceHostFrame::prepare_surface_with` to admit
   output, resolve logical measurements only when core permits a refresh, and
   obtain `SurfaceWork`. The resolver returns `Result<SurfaceMeasurementAnswer,
   E>` where `E: From<DockspaceRuntimeError>`. Gesture/preview geometry and full
   output budgets skip the resolver. Execute the complete ordered program;
   preserve clipping, ordering and `SourceGap` semantics. A retained UI stages
   owned geometry, pane references and stable `DockspaceVisualId` values. Use
   `visit_tab_resources` when preparing titles or icons for the complete program,
   including drag and popup decorations.
3. **Publish.** Confirm the execution receipt and commit before installing
   staged UI. Failed or abandoned candidates leave the installed artifact
   intact. Map every successful report's `repaint_surfaces()` to real UI/window
   invalidation, including input-only and settlement-only reports. Follow the
   core guide's [shared interface and invalidation flow](../../../crates/dockspace/docs/guide.md#engine-host-integration).
4. **Settle presentation.** Pair each committed output handle with its artifact
   and hit records. Consume it with
   `report_surface_presentation(Presented | Dropped)` after the renderer knows
   the result. Commit a subsequent frame and use its exact
   `surface_presentation_commits` and `bind_presented_receiver` to promote input
   authority. Sink acceptance, frame commit and submission alone do not prove
   presentation. An unsettled earlier output still bounds its stream even when
   later results arrive first.

Input-only and presentation-settlement frames can commit directly while
retaining the current measured scene. Explicitly defer a surface only when its
scene really is unavailable; deferring every unpainted surface can withdraw
geometry needed by native recovery. Use `measure_surface_with` for an
independent measurement pass, such as a native callback that must retain an
accepted plan for a later paint/output coordinator. Its resolver uses the same
fallible contract as `prepare_surface_with`; propagate invalid metadata or
renderer errors directly, and reserve `Unavailable` for content that is actually
unavailable. A failed resolver installs none of that invocation's answers.

Answer `SurfaceMeasurementRequest::PaneSizing` with validated content-only
`PaneSizing`, including metadata for inactive visible tabs. The selected visible
tab supplies the leaf's bounds; hidden items are requested again when shown.
Core composes chrome, so return content logical points rather than native outer
bounds. Maximum/fixed sizes may leave background inside a larger host.
Preferences act only through explicit `SurfaceSplitterAdjustment::Preferred`;
normal layout retains user weights. Follow the core guide's
[pane sizing contract](../../../crates/dockspace/docs/guide.md#pane-sizing)
for capped resize frontiers and infeasible minima.

Keep input admission, encoding retries and later updates distinct. A repeated
encoding pass uses `restart_encoding_pass` on the same candidate without
replaying input or advancing its observation time. Actions discovered while
encoding retain their witness through `DeferredSurfaceAction` and enter a later
update. An input-only commit consumes its accepted prefix even without painting.

Choose a reference with the same presentation timing:

- For a synchronous renderer, follow
  `crates/dockspace_winit/examples/software/app.rs` and its real `buffer.present()`
  result. `integration/dockspace-host-harness/examples/engine_host.rs` shows
  deferred owned records using only public core interfaces.
- For a retained renderer crossing framework hooks, follow
  `integration/gpui-harness/src/element.rs` and `presentation.rs`. Use the existing
  `DockspaceUpdate` to keep the same candidate until the framework accepts it;
  wait for the actual submission terminal before reporting its disposition.
- For egui pass retries, the core guide maps the renderer, complete-output owner
  and terminal receipt modules. Reuse their core contracts while supplying the
  new framework's own widget, layer and presentation facts.

## Route real input facts

Provide one total event order across logical surfaces and use each event's own
coordinates and available timing metadata. Helper samples stay untimed unless
the host supplies genuine observation metadata. A real callback observation time
is valid: use a consistent monotonic clock, retain its source, and preserve the
captured value through queuing and retries. Distinguish it from an original OS
timestamp. During capture, manipulation stays in the source surface while hover
may resolve to a different surface. Keep window DPI,
surface placement, UI zoom and framebuffer density distinct and apply each
coordinate conversion once.

Bind input to the actually presented artifact. Unknown overlays or receiver
facts remain unknown or blocked. At a continuation deadline, sample current
pointer, capture and receiver facts and drive the continuation; a timer does not
prove motion or presentation.

When the framework has proved that a pointer hit is unobstructed, use
`HostPointerHit::unobstructed_receiver_facts` to retain its independent click and
drag lanes and let core resolve the current hover target. The helper does not
establish framework clearance; use explicit blocked or unknown facts when an
overlay or capture prevents that proof.

Route a keyboard batch in event order. Use `prepare_tab_input_sequence` with
the focused tab as the header's starting point, or the selected tab for document
cycling. Once activation transfers ownership to pane content, leave later
events for that content. Menu confirmation and cancellation likewise terminate
the menu's prefix. Splitter sequence helpers apply each step's constraint before
preparing one final action; folding deltas first loses reversals at a limit.

Preserve wheel units, phase, momentum, device and modifiers when the host
provides them. Decide routing from the actual chrome/content receiver; a
clamped control may still own an edge even when it produces no action. Inertial
tail events keep their established owner rather than switching from content to
a tab strip merely because the pointer now lies over it.

Use `dockspace_winit` helpers when the host uses winit. Invalidate stale mappings
through resize/DPI gaps until a replacement output is actually presented, while
continuing to deliver releases and cancellation. Follow
`crates/dockspace_winit/examples/software.rs` and its `software/` modules for a
complete single-window renderer-neutral host. For OS windows, enable the crate's
`native` feature and follow `crates/dockspace_winit/examples/multiview/main.rs` and its
`multiview/` modules. The shared renderer is in `examples/support/paint.rs`.
See the crate's README for the native helpers and platform input requirements.
`MouseInput` samples mouse buttons and motion; wheel, touch and framework
keyboard/focus routing remain separate host input paths.

Keep rendering asynchronous through owned records and terminal reports when the
framework needs it. Host frames and pane callbacks remain synchronous; an async
runtime is an application choice.

## Verify the boundary

Reuse applicable `dockspace_host_conformance` scenario descriptions and provide
a thin executor through the host's real controls. Its existing core executors
are private tests; the catalog is not an external adapter runner. Start with
`integration/dockspace-host-harness/tests/shared_local_scenario/main.rs` for that seam.
For a deferred renderer, verify commit failure leaves installed UI intact,
dropped/out-of-order outputs settle correctly, and new hit records gain authority
only after their presentation report commits. Test capture between two surfaces
with identical local bounds and a DPI/zoom change. Add a real renderer/input
smoke for the framework; conformance fixtures establish the core contract.
Use `docs/ADAPTER_TESTING.md#interaction-entry-points` to compare alternative
controls for the same intent and to distinguish missing capabilities from bugs.

For OS windows, continue with [multiview](multiview.md).
