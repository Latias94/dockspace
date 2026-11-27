# Changelog

All notable changes to the public Dockspace crates (`dockspace`, `dockspace_winit`,
and `egui_dockspace`) are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
During `0.x`, breaking changes may occur on minor versions.

## [Unreleased]

### Added

- **`dockspace` (`0.1.0`, Apache-2.0)**:
  - Renderer-neutral docking workspace, layout solver, interaction state machine,
    close approval workflow, optional `motion` presentation geometry, and optional
    `serde` JSON document persistence (`#![forbid(unsafe_code)]`, WASM-compatible,
    MSRV Rust 1.95).
  - Composable `Pane` / `Tabs` / `Split` / `Grid` layout tree with named layout
    tabs, shared Grid tracks, explicit pane sizing constraints, and item
    visibility toggling.
  - Three-tier `dockspace::host` integration kit for third-party immediate-mode
    and retained-mode UI frameworks:
    - Tier 1: owned `'static` `SurfaceDisplayList`, typed `SurfaceProgramVisitor`,
      `SurfaceMetrics`, and `SurfaceMeasurer`.
    - Tier 2: `LogicalHost`, `LogicalPointerEvent` (including independent
      Click / Drag / Scroll lane resolution), fail-closed `PointerClearance`, and
      affine `LogicalPresentationToken` settlement.
    - Tier 3: `NativeReportDigest`, `NativeWindowRetirementState`, and curated
      native-window registry re-exports.
  - Runnable zero-dependency `custom_ui_host` example and standalone
    `integration/ratatui-demo` third-party TUI reference adapter with `TestBackend`
    visual + mouse tests.
- **`dockspace_winit` (`0.1.0`, MIT OR Apache-2.0)**:
  - Shared `SurfaceCoordinates`, `MouseInput`, and `RepaintScheduler` helpers for
    winit `0.30` hosts, plus optional `native` (`native-windows` / `native-geometry`)
    window registries and physical placement helpers.
- **`egui_dockspace` (`0.1.0`, MIT OR Apache-2.0)**:
  - Official `egui 0.37` single-surface docking adapter with complete-output
    `winit` / `wgpu` / `web` (WebAssembly) runners, `eframe` callback support,
    configurable motion, `AccessKit` accessibility, keyboard navigation, custom
    tab chrome callbacks, and `serde` workspace persistence.

### Notes

- **Licenses**: `dockspace` is licensed under `Apache-2.0`; `dockspace_winit`,
  `egui_dockspace`, and the unpublished experimental `egui_dockspace_native`
  adapter are dual-licensed under `MIT OR Apache-2.0`. See
  [THIRD_PARTY.md](THIRD_PARTY.md).
- **Experimental native multiview (`egui_dockspace_native`)**: remains
  unpublished (`publish = false`) and distributed via the separate fork workspace
  in `integration/egui-fork-workspace/Cargo.toml` pinned to `Latias94/egui` and
  `Latias94/winit`. See [docs/MAINTAINING.md](docs/MAINTAINING.md#experimental-native-adapter-and-fork-distribution-policy).
