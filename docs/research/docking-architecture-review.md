# Docking architecture review against Dear ImGui

Baseline reviewed on 2026-11-16 against Dockspace
`c9dd58a84bf7ba8b4d78e16e7ca9ab33ba08f218`, local Dear ImGui
`d88f238d1d363ef387382b2e29ad12cf96f826ac`, and ImGui Test Engine
`16b6efbb393fdef0dcac9b0794a8ee476606a152`.
The original review and measurements below describe that baseline. The subsequent
2026-11-16 implementation has adopted the independent short semantic model,
mandatory workbench outcome checks, separate central allocation and the small
egui focus-handoff cleanup. Current contracts and remaining proposals are
identified separately; final verification evidence belongs to the implementation
checkpoint, not the historical baseline results.
The [source archive and verified references][research] distinguish user-supplied
investigation from primary evidence. The [test design record][testing] preserves
the original proposal, its measurements and the adopted changes.

## Judgment

Keep the current layout tree, content identities, atomic publication and exact
native lifetime protocols. The inspected code does not establish a need to
replace them. One production design change has a concrete independent use case:
separating an empty central anchor from its constraint-allocation preference.
That separation is now implemented as the explicit `RootAllocation` contract.
The subsequent sizing follow-up implements a complete public contract for
maximum/fixed sizes and explicit preferred-size restoration under
[ADR 0007][sizing]. Internal solver types alone did not establish that capability.

The baseline's more urgent reliability gap was in the tests' expected behavior:
a controlled mutation let the short property accept blanket rejection of legal
center docking, while the long journey detected it. The new short model predicts
results independently and rejects that mutation too. Both test layers remain;
this closes a measured oracle gap without proving all command sequences correct.

## What ImGui's rewrite discussion actually identifies

[Issue #7892][upstream-rewrite], opened in August 2024, identifies moving/merging
node identities, the DockBuilder interface, central-node properties, constraints
and single-window behavior as rewrite concerns. Window-driven node behavior is
a possible direction, not a finalized v3 specification. Its separate viewport
discussion warns against assuming the window manager obeys requests.

The pinned implementation makes those responsibilities visible in one
`ImGuiDockNode`: topology, windows, tab bar, position/size/reference size,
host/visible window, viewport references, focus history and field-authority
flags coexist [I1]. Split/merge transfers flags and settings references; when
a floating group becomes a single window, its viewport can transfer to that
window [I2]. This is concrete coupling worth comparing, rather than evidence
that binary split trees themselves are defective.

ImGui also distinguishes platform requests from observed position/size. Its
mock backend simplifies setters and leaves resistance to move/resize as TODOs
[I3]. Neither project can treat an accepted request as physical evidence.

## Responsibilities to preserve

| Concern | Current owner and evidence | Consequence for a refactor |
| --- | --- | --- |
| Content identity and current docking intent | `ItemId`; `DockAnchor::Item` resolves its current tabs in the candidate workspace [D1]. | A historical structural node is not a permanent business location. |
| Captured group scope | `PreparedTabsGroup` freezes group membership/order/selection and workspace authority [D1], [D2]. | Moving the former item anchor must not redirect an old group capability. |
| Durable topology and visibility | Workspace owns membership, selection, weights and hidden items; visible projection omits hidden content [D3]. | Hiding content preserves its placement; approved close removes content. |
| Target and animated geometry | Target layout and presentation sampling are separate under ADR 0005 [motion]. | Animation cannot grant input authority or delay semantic ownership changes. |
| Input authority | The admitted exact Presented artifact, under ADR 0002 [publication]. | Newly sampled geometry, an old widget ID or a successful render call is insufficient. |
| Physical window lifetime | Window binding identity is distinct from logical Surface placement [D4]. | Retiring one Surface does not necessarily destroy its shared native window. |
| Native creation | The create saga keeps source ownership until exact first-live admission [D5]. | Created, Prepared and Presented cannot collapse into a `window_exists` boolean. |
| Platform facts | Typed effects request work; actual observations and terminal results settle it [D6]. | Unknown or denied focus/placement must not be replaced by desired values. |

Persistence already uses snapshot-local node integers and allocates fresh
runtime nodes on restore [D7]. The declarative builder allocates structure only
when the complete declaration is compiled [D8]. These choices avoid requiring
applications to manage mutable NodeIds or Builder call order.

## Adopted contract: separate central responsibilities

At the baseline, `central_tabs` combined a retainable empty docking leaf with
preferential constraint redistribution. The implemented declaration now separates
three responsibilities [D8], [D23]:

- `DockspaceNode::central_tabs` declares the semantic docking anchor and permits
  an empty leaf. It does not select an allocation preference.
- `DockspaceRootLayout::allocation(RootAllocation::Weighted)` is the default.
  Eligible children share constraint-induced redistribution by weight.
- `RootAllocation::PreferCentral` explicitly lets the declared central branch
  absorb or yield constraint deltas first. The declaration rejects this option
  when no central leaf exists.
- `CentralNodePolicy` independently controls merge/split permission. Allocation
  does not grant permission or change root lifetime.

Our solver differs from ImGui's `SizeRef` allocation. With normalized weights
and unconstrained panes, both allocation choices can produce the same result.
The original temporary solver probe established this discriminating fixture;
the same numbers now have public measured-pane coverage [D24]:

| Available extent: 800; weights: `[0.25, 0.5, 0.25]` | `Weighted` | Explicit `PreferCentral` on the middle branch |
| --- | --- | --- |
| No constraining minimum | `[200, 400, 200]` | `[200, 400, 200]` |
| Left child's minimum is 300 | `[300, 333.333…, 166.666…]` | `[300, 300, 200]` |

This is constraint redistribution, not a promise of fixed-width side panels.
Both choices use the shared current/future geometry solver. Projection still
validates the declared anchor under `Weighted`; only explicit `PreferCentral`
supplies its visible branch as the preferred solver route. Hidden central
ownership supplies no preferred visible row. Hiding and showing content must
preserve that distinction [D25], [D26].

Measured tests cover both axes, nested anchors, empty/occupied anchors and
constrained/unconstrained space. Additional tests cover hiding/restoring the
central branch, allocation-only fingerprint invalidation, persistence and the
last-content lifecycle [D24]–[D26]. Moving the last content item preserves a
retained main anchor but retires an emptied contained source root.
Explicitly declared empty contained anchors remain valid layouts [D20]; changing
that rule is outside this refactor. Existing four-direction tests protect anchor
identity across splits [D10]. A complete root rehome/promotion carries its root
allocation; a newly created root starts with `Weighted`.

`DockspaceRootView::allocation()` exposes the choice. Snapshots require and preserve the
explicit allocation field; they do not silently default an old payload. The
unreleased format remains version 1, with no compatibility reader. Structure
fingerprints include allocation, so old presented authority cannot survive a
change in allocation merely because membership stayed the same [D7], [D24], [D27].

## Adopted contract: complete pane sizing

At the baseline, production measurement was minimum-only even though a
test-oriented solver path accepted complete constraints. The follow-up replaces
that public measurement with `PaneSizing` [D11], [D33]. Each content axis has
validated minimum, optional maximum and optional preferred extent; equal bounds
express a fixed size. The existing shared solver receives the full declaration.

All visible items, including inactive tabs, supply complete metadata. The current
visible selection supplies its leaf's declaration; inactive declarations support
future drag layouts [D12]. Hidden items are omitted until shown again. There is
no implicit intersection of incompatible tabs: switching tabs may change bounds.
Sizing remains transient application data rather than durable node properties.

[ADR 0007][sizing] specifies content/chrome composition, selected-tab behavior,
slot/frame geometry, infeasible minima and explicit preference restoration.
Parents offer slots; capped pane frames anchor at their top-left and leave
background after saturation. That background has no pane receiver. Current and
candidate layouts retain the same declarations. Native outer placement remains
an observed host fact and can exceed a capped content frame.

Normal frames preserve user weights. `SurfaceSplitterAdjustment::Preferred`
consumes preferences only on explicit request, then commits ordinary weights.
There is no persistent automatic/manual sizing mode. The egui double-click
binding can restore preferences, equalize or be disabled [D34].

The linear resize frontier still has a defined limit: when a capped descendant
does not reach a moving slot edge, that contact change is unavailable. Its handle
publishes no resize receiver; independent complete axes remain usable [D35].
Supporting resize across such contact changes needs a piecewise-frontier design,
not weaker validation. Directional placement, remembered closed-item slots and
arbitrary conflict policies remain separate product decisions.

## Lifecycle and focus: reduce repeated rules without collapsing states

Hidden content, approved content removal and native window destruction already
have different owners and tests [D3], [D13]. Single-visible-pane tab-bar policy
uses visible membership; turning it into ordinary title chrome would be a
presentation feature, not a reason to create a second topology [D14].

Similarly, several focus records do not imply competing truth. Selected tab,
pane focus request, actual framework focus and native activation are different
facts. `DeferredTabFocus` belongs to the current input run; the framework request
cache survives across runs. Keep their different lifetimes. The implemented
`DeferredTabFocus::handoff_after_commit` removes the repeated postcommit predicate
from callback and owned integration. It still requires handled tab input and a
new committed request; neither integration's focus application phase changes
[D15], [D28]. This is a small cleanup, not a new state machine.

Do not combine normal and fatal native settlement into a mode-driven facade.
They already share close/admission rules [D16]. Fatal postcommit settlement
continues through close and admission settlement and aggregates errors;
preparation and commit failures may return partial progress [D21]. Another
orchestration layer would hide those differences with little deletion benefit.

Recent retention failures instead identify a particular cross-lifecycle rule:
a quiesced endpoint may still have a staging child borrowing its presentation.
The existing retained-emission set now includes pending native staging [D17].
Likewise, fatal cleanup has an independent service path when UI rendering stops.
Keep the regressions for these checkpoints [D18], [D22]. Test reset, late Created,
compensating close and retirement together; do not add a second registry of
authority or pretend every subsystem should finish at pointer release.

The test-only frozen layout traversal is another deliberate retained artifact,
not a parallel production implementation. Existing properties compare central
and N-ary layouts against it [D19]. Keep that optimization regression oracle;
new behavior needs explicit expectations and cannot be established merely by
updating the old and new implementations to agree.

## Delivery against ImGui's proposed rewrite concerns

These classifications compare observable contracts with the problems in
[#7892][upstream-rewrite], not with a finalized upstream v3 specification.

| ImGui concern and primary evidence | Dockspace status | Implemented behavior and remaining limit |
| --- | --- | --- |
| Mutable node identity versus long-term location, [#5161][upstream-identity] | Already avoided at the public seam | `ItemId` and current item/central anchors express content intent; internal `NodeId` is not a persistent public docking target. Captured group authority freezes its scope [D1], [D2]. A stable item does not promise a permanent screen location; directional/largest/recent-region lookup remains absent. |
| DockBuilder exposes mutable structure/call order, [#7892][upstream-rewrite] | Main coupling already avoided | Complete declarative layouts compile once; snapshots use local node numbers and restore fresh runtime nodes [D7], [D8]. Applications still choose explicit placement for programmatic open and cannot infer historic placement from an item ID. |
| Central node combines retention, allocation and special behavior, [#5209][upstream-central] | Completed in this batch | Empty anchor retention, `Weighted`/explicit `PreferCentral` allocation and `CentralNodePolicy` permissions have independent meanings, with measured current/future and lifecycle checks [D23]–[D27]. Central preference does not promise fixed sidebars. |
| Windows dictate node behavior, [#7892][upstream-rewrite] and [#5209][upstream-central] | Explicit ownership, with bounded policies | Item operation/close policy and complete sizing follow content; root allocation belongs to the root. The selected visible tab supplies the leaf bounds; inactive metadata supports future layouts [D11], [D12]. Tabs are not silently intersected. A general aggregation/conflict-policy language is absent. |
| Docked size constraints, [#2849][upstream-size] | Implemented by the sizing follow-up | `PaneSizing` supports minimum/maximum/fixed content bounds and explicit preferred restoration through one measurement seam [D33], [D34]. Slot/frame separation defines saturation background. The frozen linear frontier rejects capped contact changes; this is not a general piecewise resize solver [D35]. |
| One visible docked window resembles an ordinary window, [#9123][upstream-single] | Partial presentation support | `AutoHideSingle` uses visible membership, while logical hidden membership remains intact [D3], [D14]. It is not a universal conversion to ordinary native title chrome. |
| Hidden/closed content and remembered docking | Product boundary | Hide preserves a slot; approved close removes the item [D3], [D13]. Reusing the same `ItemId` does not automatically restore a deleted historical slot; reopening needs explicit placement. |
| Automated tests support structural change, pinned Test Engine [I3] | Strengthened in this batch | The short oracle now predicts ordered groups, selection, full local MRU, hidden state, rejection and `changed`; mandatory workbench docking/resize cases require actual outcomes [D29]–[D32]. Long exploration, frozen geometry comparisons and bounded native schedules remain. This is not a proof over all sequences. |

### Multi-viewport remains a separate comparison

Upstream's multi-viewport v2 discussion in [#7892][upstream-rewrite] concerns
platform authority as well as docking. Dockspace already separates requested
effects from observed placement/focus, logical surface placement from physical
window lifetime, and Created/Prepared from exact first-live admission [D4]–[D6].
Successful focus dispatch is not observed focus. These contracts address a class
of assumptions; they do not make every backend physically qualified.

The [native adapter's platform boundary][native-boundary] still applies:
Wayland permits configured client-local/contained docking but not managed native
lifecycle or physical cross-window docking. Windows physical cross-window input,
physical drag-out creation and arbitrary mixed-monitor DPI need further
validation. On macOS, stationary cross-window dwell/autoscroll lack the required
capture/delivery authority. Existing X11/macOS qualification covers finite
scenarios and configurations only. Logical rootless surfaces also do not imply
that the egui native adapter supports running without its root OS viewport.

## Adopted batch and remaining scope

The short semantic oracle, mandatory UI outcomes, central split and focus-handoff
deduplication are implemented. The [test design record][testing] distinguishes
their source coverage from historical measurements and records the
[restored implementation gates](docking-testing-design-options.md#restored-implementation-validation).
The complete sizing follow-up is recorded in [ADR 0007][sizing]; its validation is
separate from the historical baseline measurements below. Optional trace replay
and further composed platform observations remain bounded follow-up work,
not prerequisites silently added to this batch.

A batch completes when the changed observable contract has a discriminating
test, relevant mutation is caught, fixed regressions and applicable existing
adapter gates pass, and the removed responsibility no longer lives in callers.
State counters should distinguish skipped, expected rejection, applied and
changed actions. No additional number of random seeds proves universal safety.

## Historical baseline validation

The temporary central probe passed with both the unconstrained and constrained
numbers above. Temporarily rejecting every `DockTarget::Center` in the production
move reducer left the 128-case short property green, while the 5,000-step journey
failed its greater-than-90% success assertion. The mutation and numeric probe
were removed. A second temporary deterministic witness confirmed that the legal
move of item 3 into item 1's distinct group reached the injected error, yet the
short checker's assertions still passed with the workspace unchanged. That
mutation and witness were removed too. The final restored command-model,
layout-solver and frozen-traversal selection passed 50 tests with
`--all-features`, seed `1340171702`, two build jobs and two test threads
(nextest run `6c8ee9f0-fb61-42d4-af66-ad3fa25dcba0`). These are oracle/solver
experiments, not newly reproduced product defects or physical-platform
qualification.

## Follow-up mutation evidence

The coordinating agent repeated the blanket center-rejection mutation after the
independent oracle was implemented. Both short properties now failed with a
`model-valid CenterMove ... was rejected` diagnostic at seed `1340171702`
(nextest run `2f75e4ef-e19d-47ac-82a5-3c3892700ee8`). The constructive property
reduced to `prefix = []` and retains its fixed, distinct-group move
`item 3 -> item 1`. The ordinary property reduced to a single legal no-op,
`CenterMove item 1 -> item 1`; its minimized example must not be described as a
cross-group move. Production bytes were restored exactly, and no new persisted
failure-seed file was created. These measurements establish that this mutation
is caught; they do not replace the final restored implementation gates.

## References

[research]: docking-testing-and-imgui.md
[testing]: docking-testing-design-options.md
[publication]: ../adr/0002-atomic-frames-and-presentation.md
[motion]: ../adr/0005-motion-and-presentation-geometry.md
[sizing]: ../adr/0007-pane-sizing-and-explicit-preferences.md
[upstream-rewrite]: https://github.com/ocornut/imgui/issues/7892
[upstream-size]: https://github.com/ocornut/imgui/issues/2849
[upstream-identity]: https://github.com/ocornut/imgui/issues/5161#issuecomment-1087337677
[upstream-central]: https://github.com/ocornut/imgui/issues/5209#issuecomment-1102272275
[upstream-single]: https://github.com/ocornut/imgui/issues/9123
[native-boundary]: ../../crates/egui_dockspace_native/README.md#run-the-editor
[I1]: https://github.com/ocornut/imgui/blob/d88f238d1d363ef387382b2e29ad12cf96f826ac/imgui_internal.h#L2127
[I2]: https://github.com/ocornut/imgui/blob/d88f238d1d363ef387382b2e29ad12cf96f826ac/imgui.cpp#L19454
[I3]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/shared/imgui_app.cpp#L1478
[D1]: ../../crates/dockspace/src/model/action.rs#L27
[D2]: ../../crates/dockspace/src/behavior_tests/product_actions/tabs_group.rs#L109
[D3]: ../../crates/dockspace/src/graph/visibility.rs#L50
[D4]: ../../crates/dockspace/src/platform/viewport.rs#L145
[D5]: ../../crates/dockspace/src/platform/frame/native_create.rs#L200
[D6]: ../../crates/dockspace/src/runtime/native_effect.rs#L316
[D7]: ../../crates/dockspace/src/persistence/snapshot.rs#L1
[D8]: ../../crates/dockspace/src/model/layout.rs#L176
[D9]: ../../crates/dockspace/src/presentation/layout.rs#L1449
[D10]: ../../crates/dockspace/src/graph/operation/tests/contained_central.rs#L323
[D11]: ../../crates/dockspace/src/runtime/measurement.rs#L72
[D12]: ../../crates/dockspace/src/presentation/compiler/surface/measurement.rs#L102
[D13]: ../../crates/dockspace_host_conformance/src/tests/pane_visibility.rs#L170
[D14]: ../../crates/dockspace/src/engine/tests/tab_strip/visibility.rs#L284
[D15]: ../../crates/egui_dockspace/src/facade/callback.rs#L278
[D16]: ../../crates/egui_dockspace_native/src/coordinator.rs#L1174
[D17]: ../../crates/dockspace/src/engine/retention.rs#L13
[D18]: ../../crates/egui_dockspace_native/src/coordinator/tests/ingress_create.rs#L2106
[D19]: ../../crates/dockspace/src/presentation/layout/topology_tests.rs#L55
[D20]: ../../crates/dockspace/src/behavior_tests/product_model.rs#L71
[D21]: ../../crates/egui_dockspace_native/src/coordinator/shutdown.rs#L266
[D22]: ../../crates/egui_dockspace_native/src/surface_driver/tests/shutdown_service.rs#L6
[D23]: ../../crates/dockspace/src/model/layout.rs#L41
[D24]: ../../crates/dockspace/src/behavior_tests/central_allocation.rs
[D25]: ../../crates/dockspace/src/presentation/layout/visibility_tests.rs#L115
[D26]: ../../crates/dockspace/src/presentation/compiler/future_layout/tests/allocation.rs
[D27]: ../../crates/dockspace/src/model/view.rs#L339
[D28]: ../../crates/egui_dockspace/src/focus.rs#L123
[D29]: ../../crates/dockspace/src/behavior_tests/command_model/shrinkable/semantics.rs
[D30]: ../../crates/dockspace/src/behavior_tests/command_model/shrinkable.rs
[D31]: ../../crates/egui_dockspace/examples/workbench/tests/randomized/semantics.rs
[D32]: ../../crates/egui_dockspace/examples/workbench/tests/randomized/resize.rs
[D33]: ../../crates/dockspace/src/geometry/sizing.rs
[D34]: ../../crates/dockspace/src/behavior_tests/runtime_surface_actions/splitter_preferences.rs
[D35]: ../../crates/dockspace/src/presentation/layout/resize_contact.rs
