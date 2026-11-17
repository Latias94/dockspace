# dockspace

A renderer-neutral docking library for Rust editor UIs.

Compose bare panes, tab groups, nested splits, shared-track grids and floating
presentations while keeping pane content in your application. A tab can contain
a complete layout. Dockspace handles layout, docking decisions,
interaction state, close approval and optional layout persistence. The host
supplies measurements, drawing, input, frame scheduling and OS resources.

**Pre-release:** use a path dependency from this checkout. Rust 1.95 or later is
required. Default features include no renderer or async runtime.

## Start with a layout

```rust
use dockspace::model::{
    DockspaceLayout, DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout,
    ItemId, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::DockspaceSession;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SurfaceId::new(1),
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::central_tabs([ItemId::new(1), ItemId::new(2)]),
        ),
    )])?;
    let session = DockspaceSession::from_layout(layout, DockPolicy::default())?;
    assert!(session.view().item(ItemId::new(1)).is_some());
    Ok(())
}
```

Run the [layout commands example](examples/layout_commands.rs) from the
repository root to select, open and move panes and inspect committed results:

```sh
cargo run -p dockspace --example layout_commands --locked
```

The [composition guide](docs/guide.md#composable-layouts) covers named layout
tabs, active versus owned content, shared Grid tracks and constrained resizing.

## Connect your host

Use `model` for stable pane/layout identities, `policy` for application rules,
and `runtime::DockspaceSession` for host integration.

1. Begin a host frame and submit ordered input and application commands.
2. Supply logical measurements and execute the returned surface program into
   your renderer or an owned display list.
3. Confirm complete program acceptance, then commit the frame.
4. Report whether the exact output was presented or dropped. Accept that report
   in a later committed frame before routing input against it.

A logical surface is a layout/input area; it need not be an OS window. Multiple
surfaces can share a host. Native window effects are explicit requests for the
host to execute and report, rather than hidden calls to an event loop.

The [independent engine-host example](../../integration/dockspace-host-harness/examples/engine_host.rs)
implements the complete recording-renderer path without egui:

```sh
cargo run --manifest-path integration/dockspace-host-harness/Cargo.toml --example engine_host --locked
```

Read the [host guide](docs/guide.md) for input, output ownership, native lifecycle,
close decisions and persistence contracts, and the
[adapter testing guide](../../docs/ADAPTER_TESTING.md) for reusable host scenarios.

## Customize

- Keep pane data in your own registry, keyed by `ItemId`.
- Configure move/close restrictions, compatibility and arrangement locking with
  `DockPolicy`; configure geometry and interaction timing with
  `DockPresentationConfig`. Use `set_allow_root_resize_axis` to restrict internal
  resizing to either axis for a complete root, independently of its floating
  window border.
- Layout weights, docking fractions and logical geometry use `f64` throughout
  the core and persistence. Convert to your renderer's coordinate type at the
  adapter boundary.
- Resolve close requests immediately or retain a deferred token while saving.
- Use `frame.set_item_visibility_current(item, false)` to hide an open pane and
  `true` to restore it. Hidden panes keep their tab order, split proportions and
  window ownership; close removes their layout slot. Query `is_visible()` on an
  item view or `visible_items()` on its tabs view. Showing does not request focus.
- Enable `serde` to save and restore layouts with stable pane keys. The
  application chooses storage and serializes its own pane data.
- The default `motion` feature provides interruptible presentation geometry.
  Custom hosts supply a monotonic clock and request frames while motion is
  active. Use `MotionMode::Off` for immediate geometry, omit `set_motion` for a
  static host, or disable default features to omit motion support. The
  [motion guide](docs/guide.md#motion) explains timing, reduced motion and retained
  UI integration.

## Features

| Feature | Purpose |
| --- | --- |
| `motion` (default) | Interruptible presentation geometry with a host-supplied clock; no renderer or platform dependency |
| `serde` | JSON layout documents with stable pane keys and optional native placement preferences |

With `default-features = false`, the core retains static docking, layouts and
native lifecycle without motion or persistence dependencies.

[Examples](../../docs/EXAMPLES.md) · [Platform support](../../docs/PLATFORM_SUPPORT.md)
· [Repository](../../README.md)

## License

Apache-2.0. See [third-party sources](../../THIRD_PARTY.md) for attribution.
