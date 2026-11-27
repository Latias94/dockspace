# dockspace_winit

**Shared `winit 0.30` input, coordinate, repaint, and optional native-window helpers for custom [`dockspace`](https://github.com/Latias94/dockspace/tree/main/crates/dockspace) hosts** — with zero UI-framework or GPU dependencies and no owned event loop.

[Core Crate](https://github.com/Latias94/dockspace/tree/main/crates/dockspace) ·
[Host Guide](https://github.com/Latias94/dockspace/blob/main/crates/dockspace/docs/guide.md) ·
[Platform Support](https://github.com/Latias94/dockspace/blob/main/docs/PLATFORM_SUPPORT.md) ·
[Examples](https://github.com/Latias94/dockspace/blob/main/docs/EXAMPLES.md) ·
[Changelog](https://github.com/Latias94/dockspace/blob/main/CHANGELOG.md)

`Rust 1.95+ (MSRV)` · `#![forbid(unsafe_code)]` · `winit 0.30` · `No GUI or GPU deps` · `Pre-release`

> **When to use this crate:** Use `dockspace_winit` when building your own `winit`-based renderer or UI adapter on top of `dockspace`.
> - Building an **`egui`** app? Enable `egui_dockspace`'s `winit` or `wgpu` feature instead ([`egui_dockspace`](https://github.com/Latias94/dockspace/tree/main/crates/egui_dockspace)) so input is not routed twice.
> - Building a **GPUI** host? GPUI uses its own platform layer and does not need this crate.

---

## Quickstart

Until `0.1.0` is published on crates.io, depend on a local checkout:

```toml
[dependencies]
dockspace = { path = "../dockspace/crates/dockspace" }
dockspace_winit = { path = "../dockspace/crates/dockspace_winit" }
```

To enable multi-OS-window registries and physical placement helpers, turn on the optional `native` feature:

```toml
dockspace_winit = { path = "../dockspace/crates/dockspace_winit", features = ["native"] }
```

### Core Helpers

- **`SurfaceCoordinates`** — Maps window-client physical pixels into surface logical points using an explicit origin and pixels-per-point scale (include UI zoom in that scale when applicable; window DPI alone may not suffice).
- **`MouseInput`** — Samples one window's mouse events and retains cursor/modifier state. Cursor departure, window movement, resizing, and DPI changes invalidate position with an unknown-position move while preserving an ongoing drag; focus loss reports platform cancellation and destruction reports binding retirement.
- **`RepaintScheduler`** — Retains the earliest absolute deadline and consumes due requests once. Capture the deadline before posting a repaint through a proxy.
- **`backend`** — Re-exports the matching `winit` version.

---

## Try the Runnable Hosts

Run either software-rendered `winit` host from the repository root (no UI framework required):

```sh
# Single-window software host (tabs, splits, floating Inspector, zoom +/-, R to reset)
cargo run -p dockspace_winit --example software --features example-app --locked

# Multi-OS-window host (N: child window, D: Dock Back, C: close & recover, Esc: exit)
cargo run -p dockspace_winit --example multiview --features native,example-app --locked
```

- **Single-window `software` host** ([`app.rs`](https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/examples/software/app.rs), [`editor.rs`](https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/examples/software/editor.rs), [`paint.rs`](https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/examples/support/paint.rs)): draws the core's ordered surface program directly with `softbuffer` and a bitmap font. Hold **Shift** to float a tab, press **+/-** for UI zoom, **R** to restore layout, or **Escape** to cancel. Add `-- --smoke` for a finite synthetic-event check with real software presentation (requires a desktop session).
- **Multi-window `multiview` host** ([`app.rs`](https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/examples/multiview/app.rs), [`host.rs`](https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/examples/multiview/host.rs), [`workspace.rs`](https://github.com/Latias94/dockspace/blob/main/crates/dockspace_winit/examples/multiview/workspace.rs)): exercises creation, renderer `Prepared` facts, exact first-live presentation, Dock Back, close recovery, and ordered destruction. Add `-- --smoke` to verify compensating cleanup after forced preparation failure, three live child windows, minimized-root shutdown, and drained destruction (runs on macOS, Windows, and Linux X11).

---

## Wire Into Your `ApplicationHandler`

Connect `MouseInput`, `RepaintScheduler`, and `SurfaceCoordinates` inside an existing `winit` event loop without introducing a second loop. Here surface logical units follow window logical pixels; a zoomed UI must supply its actual pixels-per-point scale:

```rust,no_run
use std::time::Instant;

use dockspace::runtime::SurfacePointerPosition;
use dockspace_winit::{MouseInput, RepaintScheduler, SurfaceCoordinates, backend};
use backend::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

#[derive(Default)]
struct Host {
    window: Option<Window>,
    mouse: MouseInput,
    repaint: RepaintScheduler,
    presented_coordinates: Option<SurfaceCoordinates>,
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = event_loop.create_window(Window::default_attributes()).unwrap();
            window.request_redraw();
            self.window = Some(window);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.as_ref().filter(|window| window.id() == id) else {
            return;
        };
        if matches!(&event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            self.presented_coordinates = None;
        }
        if let Some(sample) = self.mouse.on_window_event(&event) {
            let position = self.presented_coordinates.map_or(
                SurfacePointerPosition::Unknown,
                |coordinates| sample.logical_position(coordinates).unwrap(),
            );
            // Route sample.event and position with your exact presented receiver/capture facts.
            let _ = (sample.event, position, sample.modifiers);
            window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                self.repaint.frame_started(Instant::now());
                // Run your UI and renderer, then report its exact presentation result.
                // After it presents, discard cursor state sampled during any mapping gap,
                // submit that invalidation, and store the output's actual coordinates.
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.repaint.take_due(Instant::now()) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        event_loop.set_control_flow(
            self.repaint.deadline().map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }
}
```

---

## Multiple Windows & Optional `native` Helpers

Compose helpers per window: keep `MouseInput` and `RepaintScheduler` in each host window's state and a `SurfaceCoordinates` mapping per logical surface placement. Route `WindowEvent` by `WindowId` while one `DockspaceSession` owns the shared workspace and the host supplies a single total input order across windows. Use the earliest outstanding deadline across windows for the event loop wait.

Enabling **`native`** (or `native-windows` and `native-geometry` independently) adds:

- **`NativeWindows<T>`** — Associates application resources (no `Clone` or `Send` bounds) with exact core-issued `NativeWindowBinding` values, rejecting conflicts without losing resources and following committed Root continuations. Resolve `WindowId` at event delivery; retain the binding for delayed callbacks. *(Specialization of `dockspace::runtime::NativeWindows<K, T>`.)*
- **`observe_window`** — Samples actual client/outer geometry, native DPI, and known visibility/minimization. Supply the renderer's presentation scale separately; unavailable positions and zero-sized drawable geometry remain unknown.
- **`window_attributes` & `request_outer_bounds`** — Apply physical placements while compensating observed decoration extents after initial window events. Retry `WindowPlacementError::GeometryChanged` after processing subsequent events if the client resized during sampling; requested placements still require a subsequent observation.

See the [engine host guide](https://github.com/Latias94/dockspace/blob/main/crates/dockspace/docs/guide.md), [platform matrix](https://github.com/Latias94/dockspace/blob/main/docs/PLATFORM_SUPPORT.md), and [integration skill](https://github.com/Latias94/dockspace/blob/main/skills/dockspace-integration/SKILL.md) for multiview contracts and platform limits.

---

## Host Responsibilities & Input Contracts

- **Authority stays in the host** — Pointer identities, receiver delivery/capture facts, exact output presentation reports (`Presented` / `Dropped`), touch, scroll, keyboard shortcuts, accessibility, and resource retirement remain host responsibilities. `MouseInput` samples supply no OS timestamp or delivery proof; submit each physical input once.
- **Coordinate gaps & invalidations** — Route input using the coordinate mapping of the exact presented output. During a resize or scale gap, keep positions unknown while still forwarding releases and cancellations, and submit every position-invalidation sample through the ordinary input lane (withdraws stale hover without asserting pointer-outside-desktop). Call `mouse.invalidate_position()` whenever your own coordinate mapping changes and submit the returned sample.
- **Modifier-only updates** — `ModifiersChanged` updates the modifier snapshot for later mouse samples and returns `None`. When modifiers control docking activation, evaluate your policy in the host's ordered stream, submit `SurfacePointerEvent::DockingEnabledChanged` for any active pointer gesture, and request a redraw.
- **Repaint deadlines** — Forward repaint requests to `repaint.request(deadline)` on the event-loop thread; starting a frame clears satisfied deadlines while preserving future ones.
- **Stock `winit` limits** — Stock `winit` supplies neither usable monitor work areas nor authoritative global cross-window hover/capture facts; physical cross-window dragging requires additional platform integration.

---

## Cargo Features

| Feature | Purpose | Default |
| --- | --- | --- |
| `rwh_06` | Raw window/display handle traits for application renderers | Yes |
| `x11` | `winit` X11 backend; forwards X11 to `softbuffer` when enabled | Yes |
| `wayland` | `winit` Wayland backend (dynamic loading); forwards to `softbuffer` when enabled | Yes |
| `wayland-csd-adwaita` | Optional Wayland client decorations; implies `wayland` | Yes |
| `native-windows` | Shared native-window resource registry specialized for `winit::window::WindowId` | No |
| `native-geometry` | Actual geometry observations and physical placement helpers | No |
| `native` | Convenience combination of `native-windows` + `native-geometry` | No |
| `example-app` | Software renderer, bitmap font, and raw-window-handle deps for examples | No |

Use `default-features = false` to select a single Linux backend (macOS and Windows use their native backends and need neither `x11` nor `wayland`):

```toml
dockspace_winit = { path = "../dockspace/crates/dockspace_winit", default-features = false, features = ["x11", "native-windows"] }
```

## License

Dual-licensed under **MIT OR Apache-2.0**. See [THIRD_PARTY.md](https://github.com/Latias94/dockspace/blob/main/THIRD_PARTY.md) for attribution.
