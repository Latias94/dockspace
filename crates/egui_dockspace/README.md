# egui_dockspace

Docking layouts for egui, backed by the renderer-neutral `dockspace` core.

Add tabs, splitters, docking guides and floating groups. Keep pane state in your
application and render it through `PaneView`. The adapter supplies egui styling,
keyboard/AccessKit actions and application-controlled close requests.

**Pre-release:** use a path dependency from this checkout. Requires Rust 1.95
and egui 0.37. Native OS windows use a separate experimental adapter.

## Run the workbench

From the repository root:

```sh
cargo run -p egui_dockspace --example workbench --features wgpu --locked
```

Drag tabs onto a pane to merge or split it, resize dividers, and close or restore
documents. Hold **Shift** while dragging a tab to float it inside the window.
A tab bar's six-dot grip moves its tab group; the same grip in a floating title
bar moves or docks the whole floating window, including its splits.

The workbench starts with Rounded tabs. Use **Tab style** and **Custom tabs** to
compare chrome, and the motion toolbar to try timing, curves and reduced motion.
Enable **Sizing → Pane sizing demo** to try a capped Inspector, a fixed-height
Console, and double-clicking the Outline divider to restore its preferred width. The
[workbench pane code](examples/workbench/editor.rs) demonstrates these features;
start with the [small editor](examples/winit.rs) when integrating your own app.

## Host choices

| Integration | Capabilities | Start here |
| --- | --- | --- |
| Optional winit/WGPU runner | Full docking in one OS window; owns rendering and output settlement | [Use winit](#use-winit) |
| Your host owns `RawInput`, the complete egui run and final `FullOutput` | Full docking with your renderer | `Dockspace::run_single_surface_with_ui`; [host guide](docs/guide.md#complete-output-hosts) |
| Ordinary `eframe::App::ui` callback | Rendering, selection, close, menus and keyboard resize; drag docking/focus confirmation unavailable | `Dockspace::show_single_surface`; [basic example](examples/basic.rs) |
| Browser canvas | Full docking and contained floating groups in one canvas | [Run in a browser](#run-in-a-browser) |
| Fork-backed eframe with native windows | Experimental multiview and native lifecycle | [Native adapter](../egui_dockspace_native) and [platform support](../../docs/PLATFORM_SUPPORT.md) |

A complete-output host reports presentation after the renderer presents that
exact output. A callback alone cannot supply that observation.

## Render your panes

Implement `PaneView` on your application registry. The layout stores `ItemId`
values; your registry keeps documents, editors and other pane state.

```rust
use egui_dockspace::{ItemId, PaneResponse, PaneView};

struct Documents {
    text: String,
}

impl PaneView for Documents {
    fn title(&self, _item: ItemId) -> Option<egui::WidgetText> {
        Some("Document".into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        let editor = ui.add(egui::TextEdit::multiline(&mut self.text).id(editor_id(item)));
        PaneResponse::from(&editor)
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(editor_id(item))
    }
}

fn editor_id(item: ItemId) -> egui::Id {
    egui::Id::unique(("documents-editor", item))
}
```

Return the content responses created in that call so ordinary widgets retain
their input ownership while docking. Composite panes can use `with_content` or
`with_content_ids`. A stable `focus_target` lets tab activation enter the editor;
override `focus_state` when several child widgets share the pane's focus scope.
Returning `None` from `title` keeps an
unavailable pane's layout slot. `Box<dyn PaneView>` is supported.

Use `ScrollArea`, wrapping or canvas navigation for content larger than its pane.
`with_scroll_area` registers a scroll area's own controls; add its inner content
responses separately.
`minimum_size` describes a useful viewport. When the whole layout cannot fit,
choose clipping or proportional compression through `DockStyle::pane_overflow_policy`.

Callbacks are synchronous and may repeat across egui passes. Run I/O in
application-owned background tasks, transfer finished data to the UI thread and
request a repaint. The [background-save example](examples/background_save.rs)
keeps a pane open if saving fails.

## Use winit

For a small runnable editor with full drag docking:

```sh
cargo run -p egui_dockspace --example winit --features wgpu --locked
```

Add the adapter from your checkout:

```toml
[dependencies]
dockspace = { path = "../dockspace/crates/dockspace" }
egui_dockspace = { path = "../dockspace/crates/egui_dockspace", features = ["wgpu"] }
egui = "=0.37.0"
```

The `wgpu` feature provides `winit::run_native`. Implement `NativeApp::update`
by returning the output from `Dockspace::run_single_surface`; the runner handles
the window loop, renderer and presentation receipt. Use `frame_finished` for
application commands after rendering. [The small editor](examples/winit.rs)
shows the complete setup.

Enable `winit` instead for `WinitIntegration` with your existing event loop and
renderer. Enable `accesskit` for system accessibility. Winit, WGPU and eframe
remain optional; [host setup](docs/guide.md#winit-and-browser-hosts) covers backend
selection, accessibility initialization and window configuration.

For another UI framework, [`dockspace_winit`](../dockspace_winit) supplies shared
mouse, coordinate and repaint helpers without an egui or renderer dependency.

## Run in a browser

Enable `web` for `winit::run_web` with an existing HTML canvas, using the same
`NativeApp` callbacks as the desktop runner.
[Try the browser workbench](https://frankorz.com/dockspace/demo/egui/), which
reuses the desktop panes and motion controls. Docking and floating groups stay
inside one canvas. Follow the [source and build instructions](../../integration/egui-web-demo)
to run it locally, or [browser setup](docs/guide.md#browser-startup-and-preferences)
to integrate startup and the system's reduced-motion preference.

## Configure motion

Motion is enabled by default for layout changes and drop previews. Dividers and
drag proxies follow the pointer directly. To disable geometry motion at runtime:

```rust
use egui_dockspace::{
    Dockspace,
    motion::{MotionMode, MotionSettings},
};

fn disable_motion(dockspace: &mut Dockspace) {
    dockspace.set_motion_settings(MotionSettings {
        mode: MotionMode::Off,
        ..Default::default()
    });
}
```

Use `default-features = false` to omit motion, then enable only the integrations
and Linux backends you need. Each effect supports immediate geometry, a tween
with configurable duration and curve, or a critical spring. The
[motion guide](docs/guide.md#motion) covers custom Bezier curves, interrupted
transitions, system preferences and custom preview painters.

## Styling

`DockStyle` follows the current egui theme. Use `DockStyle::from_egui(ui)` for
font-derived metrics or `from_egui_with_density(ui, DockDensity::Compact)` for
denser chrome. Choose a tab recipe, then adjust its fields:

```rust
use egui_dockspace::{DockStyle, TabStylePreset};

let mut style = DockStyle::default();
style.apply_tab_preset(TabStylePreset::Rounded);
style.tab_bar_height = 26.0;
style.tab_horizontal_padding = 12.0;
style.tab_background_inset = egui::vec2(3.0, 2.0);
```

`TabStylePreset::{Dockspace, Rounded, EguiTiles}` provide theme-relative starting
points. Rounded uses inset backgrounds, rounded corners and a thin selected
border. Background insets change appearance while retaining title, close-button
and hit geometry. `PaneView` callbacks customize tabs, tab-bar controls, close icons,
guides and previews while retaining the adapter's input and accessibility.
The [styling guide](docs/guide.md#styling) explains presets, overrides and callback
geometry; [workbench tabs](examples/workbench/tabs.rs) show a working example.

Configure gestures and shortcuts with `DockInputConfig`. `DockPolicy` controls
move/close permissions and resize rules. See [input configuration](docs/guide.md#pointer-input)
for modifier matching, tab cycling, middle-click close and splitter shortcuts.

## More examples

These use an ordinary eframe callback and teach application workflows. Use the
workbench for drag docking. Run commands from the repository root.

| Example | Run command |
| --- | --- |
| [Basic pane rendering](examples/basic.rs) | `cargo run -p egui_dockspace --example basic --features example-app --locked` |
| [Background saving](examples/background_save.rs) | `cargo run -p egui_dockspace --example background_save --features example-app --locked` |
| [Layout presets and autosave](examples/workspace_persistence.rs) | `cargo run -p egui_dockspace --example workspace_persistence --features example-app,serde --locked` |

With `serde`, `save_document_json` and `from_document_json` save and reopen layouts.
Reconnect pane data with `item_id_for_external_key`; the application owns file
storage and pane contents. The [persistence guide](docs/guide.md#persistence)
explains missing panes, layout reset and the example's save behavior.

[All examples](../../docs/EXAMPLES.md) · [Integration guide](docs/guide.md)
· [Native OS windows](../egui_dockspace_native/README.md)

## Features

| Feature | Purpose |
| --- | --- |
| `motion` (default) | Interruptible geometry with per-effect timing, curves and runtime disable/reduced-motion controls |
| `serde` | Layout document save/restore; pane data and file storage remain application-owned |
| `example-app` | Build the packaged eframe examples |
| `winit` | Input and platform-output bridge for an existing winit event loop and renderer |
| `wgpu` | Complete-output single-window runner and renderer using official WGPU; includes `winit` |
| `web` | Complete-output browser runner on `wasm32-unknown-unknown`; includes `wgpu` |
| `accesskit` | System accessibility delivery and action input for the winit integration |
| `x11` (default) | X11 backend for enabled winit, WGPU or eframe integrations |
| `wayland` (default) | Wayland backend for enabled winit, WGPU or eframe integrations |
| `wayland-csd-adwaita` (default) | Adwaita client decorations for the winit runner; includes `wayland` |
| `native-render-support` | Internal seam for the unpublished native adapter |

Default Linux backend features take effect only when a window integration is
enabled. They add no window loop or renderer on their own.

## License

MIT OR Apache-2.0. See [third-party sources](../../THIRD_PARTY.md) for attribution.
