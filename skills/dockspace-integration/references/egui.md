# egui integration

Read `crates/egui_dockspace/README.md` and the relevant sections of
`crates/egui_dockspace/docs/guide.md` in the selected revision.

## Own the complete output for drag docking

Use the `wgpu` feature and `egui_dockspace::winit::run_native` for the packaged
single-window runner. Start with `crates/egui_dockspace/examples/winit.rs`:
implement `NativeApp::update` and return the `DockspaceOutput` unchanged. The
runner renders it and settles its receipt. Use `frame_finished` for collected
application commands. The workbench is a reference for optional customization.
The `winit` feature exposes the event/platform-output bridge for an existing
event loop and renderer; the separate `dockspace_winit` crate is renderer-neutral.

When the application owns the egui run and renderer, follow the guide's
**Complete-output hosts** section. `run_single_surface` owns the complete run;
`run_single_surface_with_ui` adds application panels in that same run. Split
the resulting output with `into_parts()`, present the exact `FullOutput`, and
report the actual result on its receipt. Preserve deferred input returned by
`DockspaceError::into_deferred_input` and retry it in order after capacity frees.

An ordinary `eframe::App::ui` callback with `show_single_surface` supports a
smaller feature set because it lacks the final-output presentation boundary.
Check the response's capability reports. Use that callback path for its supported
interactions, or change the host when the application requires drag docking and
pane-focus confirmation. Adding another local input handler cannot supply the
missing presentation boundary.

## Pane UI and customization

Implement `PaneView` over the application's pane registry. Return the content
responses through `PaneResponse::from`, `with_content` or `with_content_ids` so
ordinary pane widgets can receive input and allow docking hover without making
unknown overlays transparent. This registration does not establish content
focus. Give the primary editor a stable ID and return it from `focus_target`;
override `focus_state` when a composite pane authoritatively tracks several
focusable children. UI callbacks may repeat across egui passes; record command
intents and perform external effects after the run succeeds. See
`examples/background_save.rs` for deferred close approval.

For a `ScrollArea`, use `PaneResponse::with_scroll_area(ui, &output)` to declare
its current scroll controls. Add its inner buttons and editors explicitly with
`with_content`; the helper does not register descendants or infer focus. Use
scrolling, wrapping or pan/zoom for content larger than its viewport. Return
validated content-only `PaneSizing` from `PaneView::sizing`; retain the value in
the pane registry rather than rebuilding fallible application settings during
painting. Minimums describe useful viewports. Maximums/fixed extents cap the
pane frame, while normal layout keeps stored weights. Configure
`DockInputConfig::splitter_double_click` for preferred restoration, equalization
or no reset; see the guide's
[pane sizing section](../../../crates/egui_dockspace/docs/guide.md#pane-rendering).

Derive geometry from the current egui style with `DockStyle::from_egui` or
`from_egui_with_density(..., DockDensity::Compact)`, then apply overrides. Keep
geometry, visuals and `DockInputConfig` bindings separate. For custom chrome,
start with the workbench's `tabs.rs`: `tab_ui`, `tab_close_ui`, `tab_bar_ui`,
`drag_preview_ui`, `drop_guide_ui` and `drop_preview_ui` retain core targeting and
adapter input behavior. Guide callbacks expose a stable ID for color/opacity
animation while core retains hit geometry. Paint the supplied candidate preview,
including its future tab order.

Use the facade's style update API so core geometry and adapter styling publish
together. When mapping a style into an existing session, use
`presentation_config_with(current)` to preserve core-only settings. Conversion
validates adapter metrics and core geometry through `DockStyleError`; core
diagnostics remain in `DockStyleError::Presentation` and the error source chain.

## Verify the selected host

Run the corresponding example or the consumer application with its actual
renderer. Check drag docking and tab reordering, clipping of custom controls,
keyboard/content focus and edited data surviving close/reopen. Test deferred
save failure and layout restore when those features are included. Callback-only
rendering tests do not establish complete-output interaction support. For custom
chrome, use the entry-point cases in
`docs/ADAPTER_TESTING.md#interaction-entry-points`, including batched keyboard
navigation, activation and wheel browsing of overflowing tabs.
