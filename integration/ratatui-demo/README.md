# `dockspace-ratatui-demo`

Standalone third-party UI framework integration built exclusively on `dockspace::host`
(`LogicalHost`, `SurfaceMetrics`, `SurfaceDisplayList`, `PointerClearance`, and
`LogicalPresentationToken`) with [`ratatui`](https://ratatui.rs/).

## Run interactively in a terminal

```bash
cargo run --manifest-path integration/ratatui-demo/Cargo.toml
```

Controls:
- Click tabs to switch active panes.
- Drag a tab into a pane to show docking drop guides (`[←] [→] [↑] [↓] [●]`) and release to split or merge.
- Drag any splitter (`│` / `─`) to resize panes.
- Scroll the mouse wheel over an overflowing tab strip, or click `▼` to open the tab-list popup menu.
- Press `m` to toggle a framework modal overlay demonstrating fail-closed `PointerClearance::Blocked`.
- Press `r` to reset the workspace layout, or `q` to quit.

## Run headless smoke & visual integration tests

```bash
cargo run --manifest-path integration/ratatui-demo/Cargo.toml -- --smoke
cargo test --manifest-path integration/ratatui-demo/Cargo.toml
```
