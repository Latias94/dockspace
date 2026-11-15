# Browser workbench

The same eight-pane editor as the desktop workbench, compiled to WebAssembly.
Tabs, nested splits, contained floating groups and configurable motion run in
one canvas. Separate browser windows and native multiview are outside this
demo's scope.

The `egui_dockspace/web` host owns the complete egui output and renderer
settlement. Its WGPU initialization is asynchronous; it prefers WebGPU and
falls back to WebGL2. Application panes and toolbar code are shared directly
with the desktop example. The demo observes live `prefers-reduced-motion`
changes; manual toolbar choices persist until another system preference change.

## Build and preview

From the repository root, with Rust 1.95+, Python 3.11+, and the CLI version
matching this workspace's lockfile:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
python3 scripts/build_site.py
python3 -m http.server 8080 --directory target/site
```

Open <http://localhost:8080/> for the homepage and
<http://localhost:8080/demo/egui/> for the workbench. HTTPS or localhost is needed
for WebGPU. Add `--dev` to the build command for an unoptimized local build.
Build output is contained in `target`; no frontend bundler or external CDN is
required. The script verifies the installed `wasm-bindgen` version before
building. Update the CLI, lockfile and Pages workflow together when upgrading.

## Browser tests

After building, use Node 24+:

```sh
cd integration/egui-web-demo
npm ci
npx playwright install chromium
npm test
```

Tests run one Chromium worker with real mouse and keyboard input, against the
generated site under a project-path prefix. They cover merges, floating-group
retirement, splitters, focus cancellation and recovery, retained edits,
intermediate and disabled motion, live reduced-motion changes, initialization
failure and native device scale two. `?test=1` enables read-only geometry
and state diagnostics emitted after exact renderer presentation; it adds no
input or layout mutation endpoint. Ordinary demo loads omit those diagnostics.

The [Pages workflow](../../.github/workflows/pages.yml) builds and tests pull
requests, and deploys successful main builds. Configure the repository's
**Settings → Pages → Source → GitHub Actions** before its first deployment.
The artifact contains a static homepage and `demo/egui/` route, so links also
work under a GitHub project path such as `/dockspace/`.
