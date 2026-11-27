# Platform support

`dockspace` is independent of a UI framework, renderer and window system.
Applications supply measurement, ordered input, rendering, scheduling and any
native resources. The [engine host guide](../crates/dockspace/docs/guide.md)
defines that contract; the [adapter testing guide](ADAPTER_TESTING.md) describes
how to exercise it from another framework.

`egui_dockspace` uses official egui for local surfaces and contained windows.
`egui_dockspace_native` is an experimental, unpublished integration with pinned
egui/eframe and winit forks. Its dependency revisions are defined in the
[native workspace](../integration/egui-fork-workspace/Cargo.toml) and lockfile.
Keep those patches out of the official workspace.

## Browser

`dockspace` and `egui_dockspace` build for `wasm32-unknown-unknown`, including
motion and optional layout serialization. The opt-in `egui_dockspace/web`
feature provides a complete-output winit/WGPU host for one existing HTML canvas.
It initializes the GPU asynchronously, skips WebGPU in non-secure contexts,
prefers WebGPU on HTTPS/localhost, and falls back to WebGL2 (replacing the
canvas when WebGPU already claimed its context) if initialization fails, times
out after five seconds, loses its device before the first frame, or cannot
present its first frame. Surfaces clamp to the device's `max_texture_dimension_2d`,
and runtime GPU device loss or `webglcontextlost` events surface cleanly through
`NativeError::DeviceLost`. A browser with WebGPU or WebGL2 and WebAssembly is
required.

The [browser workbench](../integration/egui-web-demo) reuses the desktop panes
and docking controls. It supports local tabs, splits, contained floating groups
and configurable motion. It observes `prefers-reduced-motion`, including live
changes. Separate browser windows and native multiview are outside its scope.
The [Pages workflow](../.github/workflows/pages.yml) builds the site and runs
real Chromium input journeys before producing the deployable artifact.

## Logical surfaces and native windows

A logical surface is not an OS window. One external Root window may host several
independently placed surfaces. A managed Child window hosts one surface, which
defaults to the entire client area. A surface may contain split panes and
contained floating roots.

- Publish the complete Root placement roster atomically, before measurement and
  encoding. Every placement has its own logical size, physical client origin,
  clip and presentation scale. Apply native DPI and placement transforms once.
- A custom Child window may publish the placement of its one docking region
  before measurement, including during first-live preparation. A geometry change
  requires fresh output for the same native binding.
- Report each physical window once. Source capture, hover, pane focus and exact
  presented output remain independent facts, including between sibling surfaces.
- Native close captures the complete window's content roster. Deferral or veto
  cannot close only a subset of its surfaces.
- Child creation preserves source ownership until every required placement has
  fresh, exact first-live presentation. Created, Prepared and show dispatch alone
  do not transfer content.
- Restore and layout reset may continue external Root windows with explicit
  surface mappings. Publish the replacement roster atomically, retain Child
  recovery anchors, and require fresh presentation and focus afterward.
- Removing one placement leaves a window with surviving placements intact.
  Window destruction affects the entire roster; each retired output/callback
  lane must quiesce before its lifetime is reclaimed.

The [native overview](../crates/egui_dockspace_native/README.md) and
[egui guide](../crates/egui_dockspace/docs/guide.md) explain the public entry points.
The [native composition ADR](adr/0003-native-window-composition.md) records why
these identities and publication boundaries are separate.

## Native backend capabilities

The adapter currently advertises the following capabilities in
[`capabilities.rs`](../crates/egui_dockspace_native/src/capabilities.rs).
An advertised capability means the backend can supply that class of fact;
individual observations can still be unavailable or `Unknown`.
The matrix describes available observations, not end-to-end adapter support.
Client-local readiness is a separate fact and does not add any of the global
capabilities in this matrix.

| Capability | Linux X11 | macOS | Windows | Wayland | Other |
| --- | --- | --- | --- | --- | --- |
| Authoritative inventory | Yes | Yes | Yes | Yes | Yes |
| Pointer hit-test observation | Yes | Yes | Yes | Yes | Yes |
| Global focus observation | Yes | Yes | Yes | Yes | Yes |
| Close cancellation | Yes | Yes | Yes | Yes | Yes |
| Managed native lifecycle | Yes | Yes | Yes | No | No |
| Hovered window | Yes | Yes | No | No | No |
| Desktop pointer position | Yes | Yes | No | No | No |
| Authoritative button state | Yes | Yes | No | No | No |
| Global window placement | Yes | Yes | Yes | No | No |
| Work area | Conditional | Yes | Yes | No | No |
| Pointer hit-test control | Yes | Yes | Yes | Yes | No |
| Window activation control | Yes | Yes | Yes | No | No |

Missing facts remain `Unknown`. A client-local point cannot establish desktop
position, outside-all authority or another window's hover route. Local and
contained docking can operate without global coordinates or native enrollment.
Physical cross-window promotion requires the complete current source, capture,
hover, routing and presentation authority; a capability bit cannot substitute
for a missing observation.

### Linux X11

The automated reference profile uses Xvfb, Openbox, software Glow and XTest on
one X screen at native DPI 1. Shared Root tests also exercise independent
placement scales 1, 1.25 and 1.4. These are controlled placement transforms,
not physical mixed-monitor or mixed-DPI qualification.

Authoritative work-area publication is deliberately narrower than general X11
monitor support: it requires the first X screen, one active identity-transformed
CRTC covering the root, and complete verified WM/EWMH, RandR and Xsettings facts.
Cloned, overlapping or transformed configurations and missing authority remain
`Unknown`; a global EWMH work area is not treated as independent monitor areas.

### macOS and Windows

Both backends have compile coverage for the native adapter. macOS also has
real-window programmatic lifecycle coverage and a [physical docking
driver](../integration/egui-native-smoke/README.md#run-docking-input-on-macos).
Its physical journeys cover tab merges, contained-root retirement,
held-button client re-entry, subsequent divider resizing and physical Root/Child round trips.
The cross-window profile uses nonoverlapping windows on a 1920 × 1080 desktop
at native scale 1. It verifies ordinary empty-child retirement,
empty central-area retention and native close through presentation recovery.
Escape and application deactivation cancel a visible cross-window preview before
the real release. Late movement/release preserves every pane placement, and fresh
native round trips still complete. All eight panes and their text survive, and
the original Root's OS window identity remains unchanged.
Separate cases resize the source or change egui zoom from 1 to 1.25 while held,
then require successful docking using fresh geometry and presentation. Another
closes the target through its actual native accessibility button while held,
checks recovery and physical retirement, and repeats docking with a new Child.
Real canvas, slider, text selection and scrollbar drags also cover application
deactivation, outside-client release, inert late movement and fresh recovery.
These checks do not emulate remote desktop disconnects or
qualify mixed-monitor DPI, arbitrary tear-off or third-party windows.

macOS assigns a view-scoped identity to each real primary press. Current pointer
queries wait for queued callbacks and native dispatch to drain. A global sample
with all buttons up may cancel that exact old press, without synthesizing a
release or drop. A held button alone cannot prove uninterrupted capture or
delivery: stationary observations keep those facts `Unknown`, and explicit
capture acquisition remains unsupported. Stationary cross-window dwell and
autoscroll are therefore not qualified by these moving-pointer journeys.

Native capture revocation cancels the affected viewport's egui content input as
well as docking. It does not synthesize a physical release, and a focus transfer
between application windows alone does not cancel input. Button callbacks use
their own surface position, including after cursor exit, instead of requiring a
preceding move. A replacement macOS press precedes its position callback so that
the position cannot advance the old gesture. Windows suppresses only the
synchronous no-owner capture notification caused by its own final button release;
external capture loss still cancels. These event-ordering and content-input
contracts have deterministic regression coverage; Windows has cross-compile
coverage rather than physical qualification.

Windows checks the actual thread capture owner before releasing it, so an old
window's late button-up cannot release a newer window's capture. Its live HWND
roster distinguishes managed owners from foreign windows without accessing
foreign userdata. The isolated Windows ownership regression is compiled here;
execute it on a Windows desktop before claiming physical qualification.

Windows lacks hovered-window, desktop-pointer and button-state authority and
does not have physical cross-window qualification. Keep paths unavailable when
their required facts are missing; test-driver permissions cannot supply those
backend facts.

The [solar-system example](../crates/egui_dockspace_native/examples/solar_system.rs)
provides a finite programmatic check: three real children present and move,
pause, return one child through Dock Back, then create and move a fresh child.
This exercises live geometry and lifecycle; it does not simulate physical
cross-window dragging or an OS close-button event.

### Wayland and other backends

The official `egui_dockspace` winit host supports local and contained docking
without desktop coordinates. Its finite physical-input profile runs the actual
eight-pane workbench under an isolated headless Sway/wlroots compositor, with
XWayland disabled. Real virtual-seat input merges two tabs, retires the contained
Inspector root, resizes a divider, types into Editor and scrolls Console. A second
run changes the real output scale from 1 to 2, preserves logical client size,
UI zoom and docking state, then repeats content input at the new client scale.
A third run starts at fractional output scale 1.5 (`2400 × 1800`) for docking
and divider drags, changes live to fractional scale 1.25 (`2000 × 1500`),
verifies preserved logical size, zoom and layout, and repeats text and scroll
input at 1.25.
See the [Wayland qualification commands](../integration/egui-native-smoke/README.md#run-physical-input-on-wayland).
These software-rendered, single-output Sway checks do not qualify native
multiview, GNOME/KDE CSD compositors, or mixed-monitor DPI.

The fork-backed `egui_dockspace_native` adapter uses `NativeLocalReadiness` to
admit work in a live, configured client's coordinate system. `Ready` permits
local measurement and encoding without claiming OS visibility. Local input
receivers still require fresh geometry and an exact renderer `Presented` report;
readiness alone cannot make an encoded output interactive. Unknown visibility
remains `Unknown`, and neither readiness nor renderer presentation supplies
desktop placement, another window's hover route or global routing authority.
Explicit `Hidden`, `Minimized` or local `Unavailable` observations veto local
work. Revoked authority invalidates old receivers, including when the numeric
geometry is unchanged, and readmission requires fresh presentation.

A separate native `basic` profile is available for physical Wayland qualification.
Its seven journeys require five tab merges, both contained-root drain orders,
held-button client exit/re-entry and resizing on both axes. They also check all
eight panes and application text, using real seat input and the application's
committed state. This finite headless Sway profile uses real xdg-shell windows
with XWayland disabled. The adapter still lacks managed native lifecycle,
global placement and work-area authority
on Wayland, so this local/contained profile does not qualify native multiview.

Other backends retain only the common observations shown above. New backend
support needs actual input and presentation tests as well as compile coverage.

## Verification and limits

The default [CI workflow](../.github/workflows/ci.yml) checks core behavior,
independent consumers, feature combinations, native compilation/tests and a
small real-renderer publication/backpressure smoke.
It also runs an official WGPU surface-recovery journey under X11/software GL,
injecting acquisition failures before real resource recovery and presentation.
The separate [native qualification workflow](../.github/workflows/native-qualification.yml)
runs the real-window, X11 and Wayland input journeys on demand, including separate
official-host and fork-backed native local-docking profiles. Run it before a
release or when changing native input, output ownership, lifecycle or dependency pins.
The [native qualification guide](../integration/egui-native-smoke/README.md)
contains the local commands and environment requirements.

Recording-host `Presented` reports test the public contract. Headless egui tests
exercise widgets and mapping. Compile checks establish build compatibility.
Real-window tests exercise their declared renderer/backend and journeys. None
of these results extends to an untested platform or proves GPU scanout latency.
Keep complete run logs, source revisions and environment details in CI artifacts
or release attachments rather than maintaining a dated source-tree ledger.
