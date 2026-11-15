# Native editor release qualification

The actual eight-pane `egui_dockspace_native` **basic example** is a maintained
release gate alongside the separate native smoke applications. Its composition
includes real editable panes, scrolling, contained windows and native children.
Passing a simplified smoke application does not qualify this editor composition.

[`qualify_editor.py`](qualify_editor.py) invokes the two existing finite drivers
sequentially and fails if either fails. It does not build anything, submit
programmatic docking commands, infer Rust behavior or replace their assertions.
Fatal runtime diagnostics fail the editor drivers even if a completion marker
was already printed and the application subsequently exits with status zero.

| Driver | Required behavior |
| --- | --- |
| [`editor_docking_x11.py`](editor_docking_x11.py) | Seven fresh editor processes: five inactive-tab merges into Outline, Console, Inspector, Preview and Profiler; two Inspector/Preview drain orders followed by both nested divider drags. Verify actual membership, selection, presentation, retained content and quiescence. |
| [`editor_x11.py`](editor_x11.py) | Real text edits and OS close through native first-live, Dock Back, close/Allow/reopen, locking and Default reset; preserved application text and final native quiescence. A serde build also covers JSON restore, missing content/reload and replacement of two live children. |

The runner always enables the existing `EGUI_DOCKSPACE_EDITOR_ROOT_REBIND=1`
extension: cross-surface reset and fresh Root edits, plus saved-document and
preset rebinds with serde. The driver independently observes that the original
Root receives no `DestroyNotify`, and checks retired child X11 resources.
Lifecycle commands inside that workflow remain programmatic where its driver
documents them. All seven docking cases supply actual pointer input. Each drain
journey keeps primary pressed while leaving and re-entering the editor client
area during its first tab drag. After both tabs merge into Outline, the observer
requires the contained shell and old root to disappear, all eight original items
and their text to survive, and both neighboring panes to change size for each
divider drag. Divider presses start inside the interaction padding but outside
the visible one-point line. This checks OS window re-entry with a held button;
it does not emulate a remote desktop disconnect or a missing release callback.

The lifecycle driver also checks descriptive OS titles for Inspector and the
simultaneous Inspector/Assets children while retaining its PID, exact window ID,
and destruction assertions. Unicode captions are compared in Python rather than
through the window-search tool's locale-sensitive regular expressions.

The separate `basic --command-smoke` journey exercises eleven docking commands
and applies four styles while a native child is live: geometry, visual-only,
unchanged, and restored defaults. It checks each terminal result, fresh child
content bounds, the unchanged physical window ID, and retained application data
before completing the second native round trip. It then drains the central
contained group and the remaining floating pane, requiring all floating shells
to retire while application content remains intact.

## Run against a built editor

Build serially from the repository root, using the repository target directory:

```text
cargo build --manifest-path integration/egui-fork-workspace/Cargo.toml --package egui_dockspace_native --example basic --all-features --locked -j1
```

Run on a dedicated X11 display with `xdotool`, `xprop` and an EWMH window manager
with a published work area. The Python drivers need Python 3.10 or later; Root
continuity also loads `libX11`. Do not share the display with another input test.
The runner fixes X11 scale to 1 and enables the Root continuity extension.

With an existing Xvfb/Openbox session, including the native qualification Docker
environment, reuse that display and pass the built executable:

```text
python3 integration/egui-native-smoke/qualify_editor.py /native-target/debug/examples/basic
```

On Linux without an existing display, start an isolated Xvfb and let the runner
own Openbox. Software GL is configured by the caller in CI or the container.

```text
xvfb-run -a --server-args="-screen 0 1280x1024x24 +extension RANDR +extension XTEST" python3 integration/egui-native-smoke/qualify_editor.py --start-openbox integration/egui-fork-workspace/target/debug/examples/basic
```

For a release, use the all-features build so the persistence journeys execute.
To additionally qualify the non-serde profile, rebuild without `--all-features`
and rerun the same command; the existing lifecycle driver checks its declared
profile and exact edit count. Retain the complete console log, source commit or
dirty inventory, dependency pins, build features and display environment with
the release evidence. A success line is printed only after both drivers pass.
These are X11 reference-backend results; other platforms require their own
physical-input qualification.

## Run docking input on macOS

On a dedicated macOS desktop,
[`editor_docking_macos.py`](editor_docking_macos.py) provides sixteen
application-observed journeys: the same seven local docking and divider cases,
seven real Root/Child docking journeys and two ordinary-content journeys.

```text
python3 integration/egui-native-smoke/editor_docking_macos.py integration/egui-fork-workspace/target/debug/examples/basic
```

Build the editor first with the command above. Grant the launching process
Accessibility permission in **System Settings → Privacy & Security → Accessibility**.
This driver uses CoreGraphics mouse/keyboard events and process-scoped
Accessibility requests for activation and hiding. It needs Python 3.10 or later; no
additional Python packages are required. Leave the desktop available while it
owns mouse input, unlock it, and start with egui zoom at its default 1.0. Only
the dedicated zoom-change case permits 1.25. It releases the primary button, restores the
pointer and terminates only its own remaining editor processes during cleanup.
Console and operation logs are retained in the temporary directory printed at
startup.

The seven native cases require a 1920 × 1080 desktop at native scale 1. They place
nonoverlapping windows and verify each input point against the owned OS window
inventory. Setup creates the Inspector child with the public Move to New Window
command. All four subsequent docking moves use CoreGraphics mouse input:
History moves Root → Child → Root, then Inspector and Preview return in both
orders. Except in the dedicated geometry cases, the first successful drag pauses
outside both windows until the application observes the original primary button
held there. The target uses its current selected content, including when
accumulated labels overflow the tab strip. Before release, the observer reports
the actual rendered center guide inside that content. This avoids treating a
narrow pane's geometric center as the merge button when its guide cluster moves
around outer targets.

`native-inspector-first` keeps the example's central Inspector area: after the
last tab leaves, the empty child must remain. A real click on its standard AppKit
close button invokes the default RecoverPresentation policy, restoring the exact
empty central root as a contained target. `native-preview-first` starts with an
ordinary Inspector tabs root and requires automatic retirement after draining.
Both require the exact Child OS window to disappear while the original Root
survives, and preserve all eight pane identities, tab order, selection and text.
The application waits for that independent OS observation before it exits.
Use repeated `--case` options to select individual journeys.

`native-cancel-escape` and `native-cancel-deactivate` begin with a held cross-window
drag and an application-observed drop preview. They respectively press Escape
and hide/reactivate the owned application. The preview must disappear before
the real mouse release, with all eight pane placements unchanged. Late movement
and release must remain inert. Each case then repeats the four successful native
docking moves and the central-root close/recovery checks from
`native-inspector-first`. A same-application focus transfer during normal docking
must continue to work; losing one window's keyboard focus is not application
deactivation. Visibility and activation requests use `AXHidden` and `AXFrontmost`
on the owned editor process, with actual hidden/active state observed before
continuing. Native focus and capture callbacks still drive cancellation.

`native-change-resize` and `native-change-zoom` change the Root client to
1100 × 740 or egui zoom to 1.25 while the original cross-window drag is held.
They observe the changed geometry, revoked preview and fresh presentation before
releasing. Desktop dragging survives coordinate changes: the original release
must complete the first move against current authority, then the remaining
three physical moves and exact Child retirement must succeed. The zoom profile
converts egui desktop points back to native logical points and checks that both
OS clients retain their physical size. Other profiles still reject nondefault
egui zoom.

`native-cancel-target-close` performs the owned Child's actual accessibility
close-button action while primary remains held. It requires physical window
retirement and exact presentation recovery before sending the old input tail.
It then creates a new Child with a fresh viewport and completes four physical
moves. Ordinary `ViewportCommand::Close` is not used to emulate an OS close
callback.

`content-cancel-deactivate` and `content-release-outside` exercise the actual
canvas, Zoom slider, Document text selection and Console scrollbar. Every control
must change under a real held drag before interruption. Deactivation must clear
held/dragged/button state before the real release; outside release must arrive
through the native callback. Two subsequent observed pointer movements must
leave the value unchanged, and a fresh physical drag must change it again.
Layout and all eight application texts remain unchanged.

This covers local tab merges,
contained-root retirement, divider hit padding, held-button client re-entry,
Escape/application-deactivation cancellation with late input tails, changing
geometry, closing/recreating the target, ordinary content input and the specified
moving-pointer cross-window paths.
It does not emulate a remote desktop disconnect, suppress a release callback,
or qualify arbitrary tear-off, stationary dwell/autoscroll, mixed-monitor DPI or
the separate X11 lifecycle driver. Stationary macOS samples do not establish
capture or delivery continuity; see the
[platform capability matrix](../../docs/PLATFORM_SUPPORT.md#macos-and-windows).

## Run physical input on Wayland

The official `egui_dockspace` workbench has a separate finite profile for local
and contained docking under headless Sway/wlroots. It runs on Linux, including
a Docker Desktop Linux VM on macOS, without a physical display or GPU passthrough.
Install Sway, `wtype`, `grim`, Mesa software Vulkan and the normal Wayland build
dependencies. On Debian/Ubuntu:

```sh
sudo apt-get install --no-install-recommends sway wtype grim mesa-vulkan-drivers libgl1-mesa-dri libwayland-dev libxkbcommon-x11-0
cargo build -p egui_dockspace --example workbench --features wgpu,accesskit --locked -j2
cargo build --manifest-path integration/egui-fork-workspace/Cargo.toml -p egui-native-smoke --bin wayland_seat --locked -j2
```

Run as an **unprivileged Linux user**. In a container, give that user access to
the checkout and built executables; no input-device mount or privileged mode
is needed. Each run owns its compositor, runtime directory and seat devices:

```sh
python3 integration/egui-native-smoke/with_sway.py --timeout 90 --seat-helper integration/egui-fork-workspace/target/debug/wayland_seat -- python3 integration/egui-native-smoke/editor_docking_wayland.py target/debug/examples/workbench
python3 integration/egui-native-smoke/with_sway.py --timeout 90 --seat-helper integration/egui-fork-workspace/target/debug/wayland_seat -- python3 integration/egui-native-smoke/editor_docking_wayland.py target/debug/examples/workbench --change-scale
```

The driver requires the application's exact owned `xdg_shell` toplevel and
disables XWayland and `DISPLAY`. A real virtual pointer sends complete protocol
frames for movement, primary-button and wheel input; `wtype` supplies keyboard
input. Sway IPC supplies test-only window inventory, geometry, activation and
output scaling. The production adapter receives only normal Wayland callbacks;
compositor coordinates never become backend capabilities.

The actual presented workbench reports tab, center-guide and divider locations
through AccessKit. Before dragging, the driver waits for presented source hover
and the original held press, rather than relying on fixed startup delays.
The driver drags Preview and the contained Inspector into
Outline, resizes their divider, types `waylandinput` into Editor and scrolls
Console. The application requires exact tab order and selection, retirement of
the old contained root, changed split weights and pane width, one exact text
insertion, actual scroll movement, all eight original panes and unchanged
unrelated text. Content input must leave the docking version unchanged.

`--change-scale` changes the real headless output from 1600 × 1200 at scale 1 to
3200 × 2400 at scale 2. The application must observe the new client scale while
preserving logical client size, UI zoom and docking state. Subsequent text and
wheel coordinates use that actual client scale. Fractional and mixed-monitor
DPI are separate qualification work. Each run retains compositor/app logs,
window/output inventories and screenshots in its printed artifact directories;
use `--artifacts` on each driver to choose them explicitly.

This profile qualifies the official local/contained host, not native multiview.
The fork-backed native adapter returns `Unsupported` on Wayland because it
cannot establish OS window presentation authority from unknown visibility.
It no longer waits indefinitely for a first Ready output. The deterministic
regression also requires quarantine cleanup to consume retained Wayland rosters.

## Fatal teardown and fresh-host recovery

The separate `egui-native-recovery-smoke` binary runs two native hosts in one
process. The first creates two Child windows, changes Root client size and egui
zoom, and deliberately supplies an invalid composition roster. It waits for the
specific fatal diagnostic, closes the windows programmatically, and services
late destruction callbacks after the event loop returns. Before replacement it
requires drained cleanup, zero output backlog, no cleanup errors and preserved
application documents.

The application copies the logical checkpoint, explicitly brings orphaned
surfaces into Root, and creates a new runtime and handler. The new host must
reject an old prepared action, create two fresh Child windows, and dock their
contents back with successful terminal results. All four documents must remain
in Root's tabs. The old diagnostic must remain intact. Each journey has a
25-second deadline and the process has a separate 60-second watchdog.

Run on a native graphical desktop:

```text
cargo run --manifest-path integration/egui-fork-workspace/Cargo.toml --package egui-native-smoke --bin egui-native-recovery-smoke --locked
```

The deliberate `InvalidWindowSnapshot` error is expected; success ends with
`recovery-smoke: PASS;`. The journey uses actual framework output and window
lifecycle callbacks with public application commands. It does not qualify
physical mouse delivery, OS close-button requests, cross-monitor DPI changes or
automatic restoration of OS window placement. The checkpoint is logical state;
the caller still owns documents and the policy for restoring native hosting.

## CI integration

The default [CI workflow](../../.github/workflows/ci.yml) runs native workspace
tests, including smoke unit tests, two real-renderer publication/backpressure
checks, and macOS/Windows compile checks. The separate
[Native qualification workflow](../../.github/workflows/native-qualification.yml)
is manually dispatched from GitHub Actions. Run it before a release and when
changing native input, output ownership, lifecycle or dependency pins. It builds
the applications once in the native workspace's target directory and runs:

- Actual renderer/output, text backpressure, shared Root, capture and lifecycle
  boundary tests under Xvfb/Openbox, including their ignored real-window cases.
- Public pane commands and style changes in the eight-pane editor.
- Three moving child windows, pause, programmatic Dock Back and fresh-window
  recreation through `solar_system --smoke`; OS close-button delivery is separate.
- Both editor input drivers with persistence and Root continuity enabled.
- Programmatic native redock and all three native close policies.
- Fatal multiwindow teardown followed by a new host and fresh docking commands.
- Physical X11/XTest drag-out and redock.
- A separate isolated Sway job exercises official-host physical docking,
  content input and a real output scale change, retaining logs and screenshots.

The workflow uploads run logs with the source revision and toolchain as an
artifact. Dependency pins and enabled features are available from that source
and workflow. It does not re-run the dependency forks' internal test suites;
those belong to the fork repositories.

[`with_openbox.py`](with_openbox.py) owns window-manager startup, readiness and
cleanup for a single command on a caller-supplied isolated display. It does not
build applications or infer their behavior. [`close_x11.py`](close_x11.py)
waits for each close-ready child and sends Alt-F4 through the window manager;
the smoke application checks the close policy and terminal state. Both remain
directly runnable for diagnosis:

```text
xvfb-run -a python3 integration/egui-native-smoke/with_openbox.py -- python3 integration/egui-native-smoke/close_x11.py integration/egui-fork-workspace/target/debug/egui-native-smoke
```

CI or the caller owns Xvfb, software GL configuration, build scheduling and the
selected executable. Do not share the display with another input test.
