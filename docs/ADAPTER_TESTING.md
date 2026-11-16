# UI adapter integration and testing

`dockspace` supports immediate rendering and retained UI integration through the
same public `model`, `policy`, and `runtime` interfaces. Core dependencies contain
no UI or platform framework. The independent
[public host harness](../integration/dockspace-host-harness/README.md) demonstrates
owned rendering records and input submission outside egui. Egui is the main
reference adapter. The independent [GPUI harness](../integration/gpui-harness/README.md)
adds a retained consumer with actual pane views, input and exact-frame submission
using a neighboring Open GPUI checkout. Its single-window scope does not qualify
GPUI native multiview. Recording tests do not qualify a framework's renderer or
operating-system integration.

## Adapter responsibilities

| Boundary | Core supplies | Adapter supplies and verifies |
| --- | --- | --- |
| Measurement | Typed measurement requests and logical layout | Text, chrome and pane measurements in logical units |
| Rendering | Ordered surface program, stable visual IDs, clipping and content requests | Drawing or retained-node updates with the complete ordering and clipping |
| Hit testing | Receiver descriptors and semantic target resolution | Actual framework delivery, overlay occlusion and current hover facts |
| Input | Ordered journal, gesture reduction and source/hover routes | Event-time coordinates, capture facts and one total order across surfaces |
| Presentation | Exact output handles, settlement and receiver authority | The real result of presenting each exact artifact, including dropped output |
| Time and focus | Continuation deadlines and typed pane-focus requests | A monotonic clock, fresh observations at wake time and framework focus execution |
| Native hosting | Typed effects, bindings and lifecycle obligations | Window creation, client/desktop mapping, physical focus and destruction observations |

Use `measure_surface_with` for measurements and
`SurfacePaintPlan::execute_surface_program` for the complete ordered program.
An adapter can draw during traversal or copy the required data into owned
records for later consumption. `DockspaceVisualId` can key retained nodes.
Preserve `SourceGap` identity and semantic geometry while suppressing its ordinary
visible pixels. Do not retain frame borrows or expose private engine reducers.

Application commands and prepared surface actions work without a pointer producer
or native registration. Add those capabilities when the adapter implements their
facts; a single-window adapter need not implement native hosting first.

For a center merge or tab insertion, `DockspaceDragPreview::tab_bar` and `tabs`
expose the future strip computed during the existing candidate preflight. Paint
its members in order, using their visible and title clips. `visit_tab_resources`
includes future members even when they are clipped out of the current strip.
These records grant no input authority; the currently presented layout remains
the target-selection basis. Split targets do not add future tab shapes.

## Retained UI publication

Keep candidate node changes and hit records separate from the currently installed
artifact until the host-frame commit succeeds:

1. Measure and traverse the complete surface program into staged owned records.
   Keep rendering and hit records associated with the same exact output.
2. Pass the execution receipt to `confirm_surface_painted`, account for unpainted
   surfaces when an explicit unavailable result is needed, then commit the host
   frame. Input-only and presentation-settlement frames can commit directly.
   A receipt proves sink acceptance; it does not prove presentation. Discard
   staged changes if the candidate fails.
3. Install or enqueue the committed artifact using the framework's own rendering
   boundary. Keep its output handle until that exact artifact is presented or
   dropped. Do not promote its new hit records to input authority yet.
4. Consume the handle with `report_surface_presentation(Presented | Dropped)`
   after observing the actual result. A failed or abandoned render is dropped.
5. Commit a subsequent host frame. Match `surface_presentation_commits` to the
   exact artifacts, retire obsolete records and promote admitted hit records.
   Only then can `bind_presented_receiver` authorize the matching descriptor.

Deferred output capacity is ordinary backpressure. A host can still commit input
without producing a new output. Later presentation results cannot bypass an
unsettled earlier output in the same stream. Keep this behavior when the framework
coalesces updates or renders asynchronously; the
[engine host example](../integration/dockspace-host-harness/examples/engine_host.rs)
demonstrates owned records leaving the frame borrow before exact settlement.

Keep source manipulation and hover routing independent during capture, including
across surfaces with identical local bounds. Map window DPI and framebuffer
density separately. Unknown widgets or overlays cannot assert a clear hover lane.
At a continuation deadline, observe the current pointer, capture and hit facts;
do not replay an old event or synthesize motion to satisfy a timer.

Keep an empty semantic lane distinct from framework delivery being suppressed.
`NativeReceiverAnswer::EmptyLane` proves that the queried point has no semantic
winner for that lane in the exact presented output. `Blocked` reports framework
occlusion, including a drag-only docking control suppressing a click over a pane.
`Unknown` reports missing or ambiguous evidence. A framework's missing click
receiver does not by itself prove an empty core Click lane. Resolve click and
drag independently and test their translation through the framework's actual
completed hit graph.

Exercise tails after terminal input, not only well-paired gestures: duplicate
release, cancelled input followed by stale window events, provider reset, and a
fresh press after recoverable cases. Check that each semantic terminal is emitted
at most once and the next legal interaction succeeds after cancellation or a
properly rejected stale callback. Deduplicate physical callbacks only when their
actual callback identity proves duplication. A contradictory raw edge already
accepted into the immutable core FIFO follows the permanent-failure contract
below; it need not permit another gesture in that session. Random seeds cannot
cover a callback sequence which the generator's grammar excludes.

Cancellation must also repaint the old preview, guides and drag decoration when
the layout version and scene stamps stay unchanged. Verify both source and target,
including a blocked target which paints only a pointer ghost. An unrelated window
must stay idle, and the next unchanged frame must not request another repaint.
Exercise pane content through the framework too: cancellation must clear held
controls without synthesizing clicks or releases, preserve another viewport's drag
payload, and reject an old button's release during a new button's interaction.
Cursor exit and a same-application focus transfer alone do not prove capture loss.

For pointer resizing, check pane and handle geometry in the move frame before
release, while the durable workspace version remains unchanged. Also send a
release at a new position without a preceding move, and a complete gesture in
one input batch. A dropped preview must not add a structural transition after
release. Cancellation must restore the committed partition and accept a fresh
press. Run these checks with default motion and without the motion feature.
Exercise junctions through actual hits as well: both axes must update together,
clamp independently and return to the press geometry without accumulating drift.
Cancellation after chrome or viewport changes must restore current committed
geometry even when the preview was not presented or the recovery output is
dropped. Later structural changes must retain their normal motion policy.

Vary native DPI, presentation scale, client physical size and desktop origin
independently. Test a change followed by a return to the original numeric
geometry: old receivers and cancelled gestures must remain revoked. Raw client
pixel coordinates can stay valid across a presentation-scale change while the
surface's hit geometry requires fresh presentation. Include delayed output
terminals and a successful new gesture after remeasurement.

Compose these checks across the adapter's real seams as well. A callback test
which supplies its own receiver answers cannot detect disagreement between a
framework's completed hit graph and the core geometry. The native editor
regressions render actual egui widgets, translate raw callbacks, drain both tabs
from a contained root and then resize both nested dividers through fresh hits.
They include lost releases, candidate retries and a late dropped output. Separate
device-removal journeys reject old mouse and scroll tails before completing the
next docking gesture. These deterministic journeys do not qualify OS event
delivery or a GPU presentation backend.

Supplement random seeds with bounded producer interleavings. The native pending
release suite enumerates 25 schedules across source/target changes, six geometry
or visibility changes and two output results (600 cases). Each producer retains
its own order; the protected output head follows revocation. Assert the exact
cancellation boundary, prohibit a tail from bypassing its head, and complete a
fresh docking gesture after every schedule. Returning to earlier numeric
geometry must not restore a revoked receiver.

Test fatal recovery beyond the final UI pass. The
[native recovery smoke](../integration/egui-native-smoke/README.md#fatal-teardown-and-fresh-host-recovery)
keeps the original handler through late destruction service, requires the old
runtime to drain, then starts a new host using owned logical state and retained
documents. Unit tests also require incomplete destruction, uncommitted create
failures and occupied prepaid roster buffers to retain their obligations. A
handler detach is not a substitute for destruction evidence.

Test permanently rejected ingress separately from recoverable fatal cleanup.
`native_drag_tear_off.rs` in the public host harness repeats a contradictory raw
release after a committed create request, retains the rejected FIFO head and
appends its original effect result plus exact binding-retirement/destruction
facts. None can pass the rejected head or become a successful drain. A fresh
session rejects the old result and returns its original ownership. Whole-host
physical termination remains backend qualification; see the
[failed-generation contract](../crates/dockspace/docs/guide.md#permanently-rejected-native-ingress).

For interaction-only invalidation, use the public report rather than global
repaint. The public `host_pointer_report_` cases switch hover from B to C with
unchanged layout, remove a blocked destination's ghost on cancellation and
refresh equivalent presented outputs. Assert the exact old/new surface set,
unrelated surfaces staying idle and provenance-only updates staying quiet.

Native placement tests must preserve real Foreign hover while separately
authorizing native tear-off. Verify same-resource adoption and retained source
content until exact first-live, including atomic press/move/release, discarded
candidates, stale drag identities, revoked work areas, framework blockers,
capture loss and late cancelled outputs. A mocked Foreign route does not qualify
OS hit testing, point-scoped z-order or physical presentation.

## Interaction entry points

Build an entry-point inventory from the union of this adapter and its reference
UI. Starting only from the adapter's existing controls cannot reveal a missing
workflow. Distinguish unsupported capabilities, intentional behavior differences
and defects before assigning severity.

For each entry, record its object scope, availability, preview, commit and cancel
behavior. A tab, tab group, complete root and native surface are different scopes.
Compare alternative entries for the same intent through actual framework input:

| Intent | Entries and cases to compare |
| --- | --- |
| Select or focus | Tab body, header keyboard input, accessibility Focus and Click, trailing strip space, floating title |
| Move content | Item drag, group grip, trailing space, contained title, application command and presentation menu |
| Close | Tab close, middle click, layout close, OS close and deferred application decisions |
| Browse tabs | Wheel, overflow controls, tab list, keyboard navigation and application-owned drag payloads |
| Resize | Splitter, junction, floating edge, keyboard, accessibility and native frame |

Vary single/multiple tabs, nested splits, an empty central slot and an empty main
surface. For partial transfers, assert the moved payload's geometry and constraints
as well as ownership; keeping every item does not prove the right object moved.
For focus, check where the next keyboard input is delivered, not just selection
or z-order. Accessibility Focus must keep header navigation available; explicit
activation may request pane-content focus.

Include several key presses in one raw batch, direction reversals, a round trip,
and navigation followed by activation. Preserve their order and submit one final
candidate-bound action where intermediate states need not be published. Repeat
the batch across discarded passes and in a new independent run. A boolean
`consume_key` result can hide multiple presses even when separate-frame tests pass.

Native OS title movement is a geometry observation, not proof of a docking drag.
Test client chrome separately and declare when the adapter has no whole-root
drag entry point. A command-only workaround does not qualify that missing entry.

## Reusable automatic tests

| Layer | Existing asset | What passing proves |
| --- | --- | --- |
| Core host conformance | [Scenario catalog and private executors](../crates/dockspace_host_conformance/README.md) | Core behavior through public APIs under declared capabilities |
| Independent public consumer | [Recording host tests](../integration/dockspace-host-harness/tests/) | External API usability, owned output, routing and settlement contracts |
| Concrete UI entry point | [egui product harness](../integration/egui-product-harness/tests/product.rs) | Actual egui measurement, widgets, accessibility and input mapping |
| Retained UI entry point | [GPUI harness](../integration/gpui-harness/README.md) | GPUI views, event delivery, candidate rollback and exact test-renderer outcomes |
| Native physical integration | [X11 and macOS qualification drivers](../integration/egui-native-smoke/README.md) | Only the exercised backend, environment and physical journeys |

`dockspace_host_conformance` is an unpublished test crate. Its shared `scenario`
module describes semantic selectors, actions, capabilities and observations. Its
existing core executors are private tests, not a generic external adapter runner.
The journey catalog names coverage requirements; not every entry is a directly
reusable executable scenario.

The [external L08 consumer](../integration/dockspace-host-harness/tests/shared_local_scenario/main.rs)
is a small example for a new adapter. It consumes
`LOCAL_TAB_CLOSE_SPLITTER_AND_CONTAINED`, resolves controls in the current public
paint plan, applies prepared actions and checks selection, close requests,
splitter movement and contained resize before recording observations. It uses
the complete recording sink and exact presentation settlement. It declares its
local capabilities and rejects `NATIVE_CREATE_REDOCK_RETIREMENT` before input.
The egui executor consumes the same semantic scenario through actual accessibility
controls and keyboard input. Neither executor shares runtime IDs or output authority.

`LOCAL_PANE_VISIBILITY` adds a second L08 scenario. It hides selected content,
hides the complete tab group, restores it, adjusts its splitter, and hides and
restores a contained presentation. The recording host checks measurement,
painting and receiver inventories; the egui executor drives application-owned
Hide/Show buttons and checks actual content, accessibility and restored bounds.
Both verify durable tab order, split weights and contained placement. Visibility
is an application command, so an adapter need not add built-in Hide/Show chrome.
Declare `LocalItemVisibility` before executing this scenario.

For a new framework, keep the semantic scenario and replace the thin execution
layer with that framework's real controls. An early headless consumer can use
prepared actions, but UI qualification must also drive the actual input bridge.
Record observed state or feedback after checking it; do not copy expected
observations into the result without checking the behavior.

Extend coverage in this order:

1. Run L08 through real tab, close, splitter and contained-resize controls, then
   run `LOCAL_PANE_VISIBILITY` through the application's visibility controls.
2. Run `MAGNETIC_DOCKING_BASINS` (L03): five basins, three normalized inset samples
   and four cadences, for 60 cases. Resolve points against current framework
   bounds rather than copying egui coordinates.
3. Exercise rollback, stale or foreign receivers, dropped and late outputs,
   overlay occlusion, independent source/hover routing and deferred consumption.
   A late output containing a now-hidden pane must not restore that pane's input
   authority. After showing it, require a fresh output and successful interaction.
4. Verify continuation scheduling and pane-focus observations through the actual
   framework. Add N01 lifecycle and physical journeys when native support exists.

Declare unsupported capabilities before scheduling a journey. Passing local
recording scenarios does not imply native or physical authority. A new adapter
needs no new core mutation path, shared GUI driver language or alternate state
machine to reuse these tests.

## Local gates

Run these commands from the repository root, serially. The two independent
workspaces retain their own lockfiles. Existing CI includes all host test targets
in default and `serde` configurations.

```console
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0
cargo nextest run -p dockspace_host_conformance --lib --locked --test-threads=1
cargo nextest run --manifest-path integration/dockspace-host-harness/Cargo.toml --test shared_local_scenario --locked --test-threads=1
cargo nextest run --manifest-path integration/dockspace-host-harness/Cargo.toml --test shared_local_scenario --features serde --locked --test-threads=1
cargo nextest run --manifest-path integration/egui-product-harness/Cargo.toml --test product --locked --test-threads=1 -E 'test(shared_local_interaction_scenario) | test(shared_l03_complete_magnetic_docking_matrix)'
python3 scripts/check_core_dependencies.py
```

The dependency check covers core normal/build dependencies across features and
targets. It does not certify an adapter's dependency graph or runtime behavior.
