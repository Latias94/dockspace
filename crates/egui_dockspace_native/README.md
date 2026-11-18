# egui_dockspace_native

Experimental native-window hosting for `egui_dockspace`.

This unpublished adapter connects Dockspace to pinned eframe/winit forks. It
adds native window creation and recovery, per-window output reporting and
multiview input integration. The core remains the sole docking-state owner;
your application owns pane content.

The default `motion` feature uses the same core pane geometry and egui decoration
tracks as the official adapter. Same-root survivor reflow uses the shared 200 ms
tween; pointer-driven resizing remains immediate. New and removed panes do not
use entry or exit snapshots. Native window movement, including the solar-system
example, follows its own application clock and the desktop compositor.

For one OS window using official egui, start with
[`egui_dockspace`](../egui_dockspace) and its complete-output editor instead.

## Run the editor

From the repository root:

```sh
cargo run --manifest-path integration/egui-fork-workspace/Cargo.toml -p egui_dockspace_native --example basic --locked
```

Click **Open Inspector Window ↗** in the toolbar to move the floating Inspector
group into a child OS window, then **Dock Inspector Back** to return it. A tab
bar's window-outline menu also offers **Move Layout to New Window** and **Dock Back**
for the entire root. Its separate three-dot menu acts on one tab group. Closing
a child window recovers its panes by default.
Use **Pane inventory → Hide / Show** to temporarily remove panes while keeping
their layout slots. **Close** removes a slot; pane data remains in the inventory.
Add `--features serde` for application-owned named layout presets. These presets
live in the example process; pane data stays separate from layout documents.

For a docking or resize report, enable the example's operation log:

```sh
cargo run --manifest-path integration/egui-fork-workspace/Cargo.toml -p egui_dockspace_native --example basic --locked -- --event-log native-dockspace.log
```

Choose a new file for each run; an existing file is never overwritten. The log
records surface/viewport identities and geometry, pointer buttons, sampled pointer
positions, focus and modifier changes, Escape, action results and full error
details. Notifications include the current committed layout when observed;
these snapshots are not a historical replay recording. Text entry and pane
content are omitted. Fatal runtime errors print to the terminal once, followed
by any cleanup errors, even without `--event-log`.

Check the [platform matrix](../../docs/PLATFORM_SUPPORT.md) before choosing this
adapter. Programmatic window commands and physical cross-window dragging have
different requirements. Linux X11 and macOS have finite physical-input coverage;
physical drag-out to create a window, Windows cross-window input and arbitrary
mixed-monitor DPI need further validation. Stationary cross-window dwell and
autoscroll lack capture/delivery authority on macOS.

On Wayland, configured client-local readiness permits local and contained docking.
Managed native lifecycle and physical cross-window docking remain unavailable.
The matrix and linked qualification guides describe the tested configurations.

For a small multiview demonstration with a central Sun and three orbiting OS
windows:

```sh
cargo run --manifest-path integration/egui-fork-workspace/Cargo.toml -p egui_dockspace_native --example solar_system --locked
```

Pause the orbit to drag and dock windows by hand. Closing a planet window
recovers its pane; **Launch returned planets** creates new windows for panes
back in the main window. A combined multi-pane window stops orbiting so its
panes do not issue conflicting position requests. The example needs a backend
with native creation and desktop placement support. Add `-- --smoke` for a
finite real-window create/move/Dock-Back/recreate check. The smoke uses the
public programmatic recovery command; it does not simulate an OS close-button event.
One application clock requests movement at 60 Hz; child windows report their
actual geometry without starting independent animation timers. Use `--release`
when assessing motion and CPU cost. OS window movement is still scheduled by
the desktop compositor, independently of pane rendering.

## Integrate

Construct `NativeDockspaceApp` with a `DockspaceSession`, Root surface, pane
provider and matching `DockStyle`. Install its native host handler in the fork's
eframe options. The [basic example](examples/basic.rs) shows the complete setup.

Build the session from the same style's `presentation_config()`. Core's
renderer-neutral defaults are independent of egui's Compact metrics, and the
native constructor rejects disagreement instead of changing either side:

```rust,ignore
let style = egui_dockspace::DockStyle::default();
let session = DockspaceSession::from_layout_with_presentation_config(
    layout,
    policy,
    style.presentation_config()?,
)?;
let app = NativeDockspaceApp::new(instance_id, session, root_surface, panes, style)?;
```

For persistence, use `from_persistent_layout_with_presentation_config` or
`from_document_json_with_presentation_config` with that same configuration.
Live geometry edits go through `request_style`, so core measurements and egui
painting change together; visual-only overrides do not require new geometry.

Pane providers implement `PaneView + Send + 'static` because deferred viewport
callbacks retain them. `Box<dyn PaneView + Send>` is supported. Registry/view
callbacks hold the runtime lock: keep them short, perform I/O outside them, and
submit commands after they return.

Submit `NativeCommand` through `request_command` and retain the returned
`NativeActionReceipt`. Service turns advance accepted work even when the Root
is not painting. Only an executing request or a real presentation wait keeps
admission Busy; completed, unread receipts do not block another command,
saving or quiescence. Each receipt receives only its own result. Dropping a
receipt intentionally ignores completion without cancelling the request.
If a request reports `NativeActionFailure`, inspect `interruption()` to distinguish
failure before core commit, incomplete settlement after commit, and an unresolved
presentation at terminal shutdown. A committed interruption must not be retried
as though the original action never ran.

```rust,ignore
let receipt = app.request_command(command)?;
// Keep `receipt` in the application operation that submitted it.
match receipt.try_recv() {
    Ok(result) => { /* handle this command's exact terminal result */ }
    Err(std::sync::mpsc::TryRecvError::Empty) => { /* wait for host progress */ }
    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
        // Already consumed, or the runtime ended without observing completion.
        // This does not prove native resource cleanup.
    }
}
```

Use `NativeCommand::Move { source, target }` for all existing-content moves.
`DockMoveSource` distinguishes one item, a captured direct tab, a captured tabs
group and a complete root. `DockMoveTarget` chooses an explicit dock placement,
a contained float on an existing surface, or a managed native window. The
native target accepts `Some(placement)` for requested desktop bounds or `None`
for placement derived from current core geometry and work-area facts.

A `Dock` target includes `TabGroupDockMode`. With a captured `Tabs` source,
`MergeTabs` inserts its direct children as neighboring tabs; `PreserveLayout`
inserts the complete group as one layout tab. Center and before/after targets
honor that mode; edge targets retain their structural split behavior. Captures
keep their original session and workspace version in both modes.

For the layout menu's **Float Layout**, use a `Root` source and
`Contained { surface }` target. This keeps the complete Pane/Tabs/Split/Grid
structure, stable root identity, layout titles, track shares and tab selections.
The core derives geometry and retains native ownership until exact target
presentation. Missing geometry, unavailable destinations and policy restrictions
arrive through the request's receipt. `DockRootBack` remains a separate command
whose return destination is derived and revalidated by core.

For one direct tab, obtain its `TabKey` through `with_view` and call
`capture_tab(key)` after the callback returns. Use the captured value as
`DockMoveSource::Tab` or submit `NativeCommand::CloseTab`. A layout tab carries
all hidden and inactive descendants. Storage, cloning and admission never
refresh its captured session or version. A standalone Pane has no direct tab
to capture. Close requests return application decision tokens through their
receipt; no close is approved on the application's behalf.

Apply completed pane data through `with_panes_mut` and request repaint on the
affected egui viewport.

`NativeCommand::SetItemVisibility { item, visible }` retains the pane's tab order,
split proportions and window ownership. Showing does not activate the window or
request focus; submit `SelectItem` separately when needed. Entirely hidden roots
keep their native windows until an explicit host lifecycle action changes them.

This crate uses the separate [fork workspace](../../integration/egui-fork-workspace/Cargo.toml).
Do not patch the official adapter workspace to run it. Current dependency pins
are maintained in that manifest and lockfile.

Use `NativeDockspaceApp::with_input` with the same `egui_dockspace::DockInputConfig`
as the official adapter. It controls hold-to-enable/disable docking and the
gesture-cancel shortcut; default bindings are Shift and Escape. Queue runtime
changes through `request_input` and retain its own receipt, like other
application requests.
Native cancellation follows callback order with pointer release, including when
no UI pass occurs between them. Cancellation reports `ExplicitCancellation`;
later rendering does not apply the same key to a new gesture. The native journal
stores only a bounded key control, while text and IME remain egui application
input. If a managed close cannot enter the bounded queue, the host synchronously
vetoes that exact request so ordinary eframe shutdown cannot bypass its policy.

Use `with_motion(settings, preference)` before host attachment, or
`request_motion(settings, preference)` at runtime. Both use the core's
`motion::{MotionSettings, MotionPreference}`. The returned request receipt
reports `NativeActionStatus::MotionChanged` after the configuration commits.
For example, disable geometry motion with:

```rust,ignore
use egui_dockspace_native::motion::{MotionMode, MotionPreference, MotionSettings};

let receipt = app.request_motion(
    MotionSettings { mode: MotionMode::Off, ..Default::default() },
    MotionPreference::Unknown,
)?;
```

Tune each effect with `Transition::Instant`, `Tween { duration, curve }`, or
`CriticalSpring { response }`; custom validated Bezier curves use `CubicBezier`.
The preference defaults to `Unknown`: no portable OS preference is inferred.
Supply `Reduced` from application platform integration, or enable
`unknown_prefers_reduced_motion` for a conservative fallback. A zero egui
animation time also disables geometry motion. Off and Reduced settle immediately
and schedule no motion continuation. To omit motion entirely, use
`default-features = false` for this crate, or `--no-default-features` when running
its examples.

Each native outer update samples the existing monotonic clock once; repeated egui
passes retain the same sample. Motion settings do not invalidate measurements.
Animations wait for exact output terminal callbacks and stop requesting frames
after settlement. A pending output or capacity barrier does not start a polling
timer. Prospective native first-live output stays static until the existing
ownership barrier admits it. This does not animate OS-window bounds or promise
atomic cross-window content flights.

Use `with_notification_handler` to observe the same `DockspaceNotification`
as headless hosts and the single-surface egui adapter. Each notification combines
layout and native-placement summaries with detailed lifecycle events. Use
`notification.document_changed()` for debounced persistence: split resizing and
confirmed OS-window movement, size and native scale can request a save without
a detailed lifecycle event. The optional synchronous
callback receives only future nonempty publications, including native service
work and final UI passes. Pending transfers, discarded passes and rejected
actions do not produce completion notifications. The adapter retains no event
history. Callbacks run in commit order after internal settlement releases the
runtime lock. Shared runtime queries and queued commands are safe; keep handlers
short and schedule file I/O through application-owned workers. OS-window geometry
stays outside the docking-layout version. Unknown placement is not guessed;
first-live admission and geometry becoming authoritative again are also announced.
The callback's second argument, `NativeNotificationContext`, provides scoped
`with_view`, `request_command` and, with `serde`, `save_document_json` access.
Commands follow the same bounded execution admission; each accepted callback
request returns its own receipt. Saving reads committed state and is not blocked
by completed, unread receipts.

`pending_work()` explains `is_quiescent()` with an allocation-free snapshot of
current wait reasons. Inspect `contains(NativeWaitReason::Focus)` or iterate the
active reasons; debug formatting prints their names. This includes executing
application requests, pending presentations and native window retirement, while
ordinary stable repaint outputs remain idle. The query consumes no work and
retains no history. Inspect `output_backlog()` separately for output-owner counts.

## Failure recovery

Recover at the layer which lost authority:

| Condition | Recovery |
| --- | --- |
| DPI, UI scale, window position or client size changes | Publish current window facts, cancel invalid interactions, then measure and present fresh controls. Returning to the same numeric geometry still requires fresh authority. |
| Speculative UI pass is discarded before publication | Drop the candidate and retry against committed state. Preserve ordered input; do not replay application text or IME events. |
| An exact committed output is discarded | Report that original output as `Dropped` and let its settlement advance. Receiver authority requires a later presented output. |
| Fatal protocol or internal runtime error | Keep the cause and quarantine semantic input. Drain cleanup while the accepted prefix remains reducible; otherwise physically terminate and dispose of the complete old host generation as failed before creating a fresh runtime. |

The public view remains readable during a fatal stop. Capture an owned logical
checkpoint without taking the application command slot or enabling `serde`:

```rust,ignore
let checkpoint = app.with_view(|view| view.to_layout());
```

This captures the current committed layout, including tab selection, split
weights, central regions and contained-window order. Queued commands and live
drag/resize previews are excluded. Cleanup can commit further content recovery,
so capture at the boundary whose layout the application intends to retain.
Pane data, policy, style, presentation settings and document sidecars remain
application-owned or in their existing persistence interfaces.

For a fatal stop whose cleanup input remains reducible, keep driving cleanup until
`is_shutdown_drained()` returns true, and settle the remaining `output_backlog()`
owners. Reconstruct a `DockspaceSession` from the checkpoint and the application's
configuration, then create a fresh
`NativeDockspaceApp` and host handler. Old receivers, prepared actions, output
handles and window bindings belong to the old runtime. An in-place restart
which clears the error would leave those outstanding owners unresolved.

A permanently rejected accepted FIFO prefix is a different terminal failure:
later destruction observations cannot make `is_shutdown_drained()` true by
passing that prefix. This adapter does not automatically reconstruct that case.
The owning backend must freeze admission and retain the failed generation while
all physical resources, callbacks, renderer outputs and asynchronous handoffs
terminate. Only then can the complete generation be disposed of as failed and
a fresh host be created. Clearing individual affine results, detaching the
handler or assuming that requested closes completed is insufficient. See the
[core failure contract](../dockspace/docs/guide.md#permanently-rejected-native-ingress).

Cleanup also advances through the existing handler's `on_service(&context)`;
it does not require another UI output. Keep the handler and egui context alive
long enough to service destruction callbacks queued while `run_native` returns.
Honor `Wait` when more host facts are required instead of polling indefinitely.
Once every materialized window has committed its exact destruction, admitted
creates have terminated and input/output owners have settled, the adapter can
submit the final empty live-window inventory using the established backend.
Detaching the handler alone is not evidence of window destruction. This derived
inventory requires a previously committed backend roster; a failure before that
baseline can still process an already queued first roster during cleanup, even
when its geometry only revokes inventory authority. If no backend roster has
been captured, the host must still supply those facts.

The [real-window recovery smoke](../../integration/egui-native-smoke/README.md#fatal-teardown-and-fresh-host-recovery)
demonstrates the complete replacement flow and its application-owned document
and hosting policy.

A logical checkpoint preserves multiple surfaces, but `NativeDockspaceApp::new`
registers only its designated Root. Supply Root composition placements and
explicit Child hosting through the existing native APIs. OS-window ownership,
placements and document keys require the separately saved hosting policy or
document persistence flow; `to_layout()` does not recreate them automatically.

## Window configuration

`with_child_window_options` and `set_child_window_options` configure managed
Child window decorations, Windows taskbar presence, stacking level and icon.
Use `with_child_window_options_for(surface, options)` or
`set_child_window_options_for(surface, options)` for a stable `SurfaceId` override;
unset fields inherit the current global options. `child_window_options_for(surface)`
returns the resolved configuration. Overrides can be installed before creation
and survive replacement or restoration of the same surface.
`clear_child_window_options_for(surface)` removes an override and resumes the
global configuration. After inheritance, unset fields use the same defaults for
live windows and replacements: decorations and Windows taskbar presence are
enabled, the stacking level is `Normal`, and no window icon is set. Removing a
custom icon explicitly clears it; an unset icon does not inherit the Root icon.
`NativeChildWindowOptions::default()` keeps all fields unset so surface overrides
can inherit every global field. Windows taskbar presence is creation-only:
changing it does not rebuild a live window and applies at its next creation or
replacement. Other changes apply on the next Root UI update, so request a Root
repaint when changing them outside an update. Creation and recovery
placement, visibility and ownership remain coordinated by the native lifecycle.
Applications may request ordinary movement of a live window through the host;
fresh OS observations determine its actual geometry. Configure the external
Root window through eframe's `NativeOptions::viewport`. Child titles combine the
active real content in visual order, following every ancestor Tabs choice and
including bare Panes. Hidden and inactive content is omitted; an empty window
uses a surface caption. Titles have no library-brand suffix.

Pane controls use one runtime content namespace across surfaces. Their egui IDs
and stored widget state follow the pane during native docking. Use stable pane
IDs for application controls and avoid adding the current SurfaceId or
ViewportId to those content IDs. Surface chrome still uses placement-specific
identities. Application-owned deferred viewports can coexist with the installed
native handler and keep their ordinary eframe lifecycle.

Window ordering has two scopes:

- `NativeCommand::RaiseContained` raises a floating root inside its logical
  surface. It does not change OS window order.
- `NativeCommand::SelectItem` activates an item through its Tabs ancestors and
  requests activation of its native window. Actual focus remains a platform
  observation.
- `NativeChildWindowOptions::window_level` requests `Normal`, `AlwaysOnTop` or
  `AlwaysOnBottom` globally or for one overridden surface. The Root uses its own
  viewport builder. A level does not specify order between windows at that level.

The adapter currently has no relative OS-window ordering API. `Child` describes
lifecycle ownership; it does not establish a native owner/transient-parent
relationship or guarantee that a child stays above the Root. Those policies
require explicit platform support.

[Examples](../../docs/EXAMPLES.md) · [Development](../../CONTRIBUTING.md)
· [Native qualification](../../integration/egui-native-smoke/README.md)

## License

MIT OR Apache-2.0. See [third-party sources](../../THIRD_PARTY.md).
