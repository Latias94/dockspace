# Docking architecture review against Dear ImGui

Reviewed on 2026-11-16 against Dockspace
`c9dd58a84bf7ba8b4d78e16e7ca9ab33ba08f218`, local Dear ImGui
`d88f238d1d363ef387382b2e29ad12cf96f826ac`, and ImGui Test Engine
`16b6efbb393fdef0dcac9b0794a8ee476606a152`.
This is a research recommendation, not an adopted change to public contracts.
The [source archive and verified references][research] distinguish user-supplied
investigation from primary evidence. The [test design proposal][testing] follows
from the architectural findings below.

## Judgment

Keep the current layout tree, content identities, atomic publication and exact
native lifetime protocols. The inspected code does not establish a need to
replace them. One production design change has a concrete independent use case:
separate an empty central anchor from its constraint-allocation preference.
Maximum/fixed pane sizing is a capability decision that requires a complete
public contract. It is not already supported by the internal solver's types.

The more urgent reliability improvement is in the tests' expected behavior.
A controlled mutation confirmed that the short command property can accept the
blanket rejection of legal center docking. The long command journey detects
that mutation. Both facts matter: keep the long exploration while making the
short property's semantic expectations independent.

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

## Recommended production change: separate central responsibilities

`central_tabs` currently declares both a retainable empty docking leaf and a
remaining-space region [D8]. `RootRecord.central` drives canonical retention
and the solver's preference for absorbing or yielding constraint deltas [D9].
Merge/split permission is already separately configured by `CentralNodePolicy`.

The missing combination is concrete: keep a document landing area after its
last tab leaves, while all panes share a constraint-induced adjustment by
weight. Ordinary tabs cannot remain empty; the central declaration implicitly
selects the allocation preference.

Our solver differs from ImGui's `SizeRef` allocation. With normalized weights
and unconstrained panes, central and ordinary allocation can produce the same
result. A temporary solver probe confirmed this discriminating fixture:

| Available extent: 800; weights: `[0.25, 0.5, 0.25]` | Ordinary weighted allocation | Middle child is central |
| --- | --- | --- |
| No constraining minimum | `[200, 400, 200]` | `[200, 400, 200]` |
| Left child's minimum is 300 | `[300, 333.333…, 166.666…]` | `[300, 300, 200]` |

This is constraint redistribution, not a promise of fixed-width side panels.
The temporary probe exercised `solve_axis`; public measured-pane coverage is
still needed for a future policy change.

The recommended shape is an independently declared root allocation choice:
weighted redistribution, or preferential redistribution through the central
anchor. Keep the anchor as a semantic destination. Retention, allocation and
merge/split permissions must have separate documented meanings; the final type
names are not decided here. Recommend weighted redistribution as the default,
with central preference selected explicitly. Selecting central preference
without a declared central anchor should be a declaration error, not create an
implicit anchor.

Acceptance must cover both axes, nested anchors, all edge directions, hiding,
moving the last item and persistence. Moving or closing the last content item
preserves a retained main anchor, but retires an emptied contained source root.
Explicitly declared empty contained anchors remain valid layouts [D20]; changing
that rule is outside this refactor. Allocation choice must not change either
rule. Preserve the anchor across a split instead of inheriting it from the
direction of a Builder operation. The existing four-direction tests already
protect that identity [D10].

If implemented before first release, replace the overloaded public explanation
and update declaration, metadata, snapshot and documentation together. Keep
the unreleased format at version 1 as requested; do not add a legacy reader or
parallel interpretation for the old combined meaning. Select and document the
default explicitly, and persist the allocation choice without silently
reinterpreting the old payload.

## Capability decision: maximum and fixed pane sizes

Public measurement asks for `PaneMinimum` for visible items, including inactive
tabs; hidden items are omitted [D11]. The current visible selection supplies the
leaf minimum. Inactive metadata supports future drag layouts, not a maximum
over all tab minima [D12]. Changing selection can therefore change constraints.

There is no public pane maximum, preferred width or fixed-width contract.
Internal complete-constraint paths are partly test-only, while production uses
minimum measurements. A minimum of 320 does not promise an Inspector stays 320
wide when the host grows. Disabling splitter input does not provide that promise
either.

First distinguish a bounded application content view from a bounded docking
slot. An application can center, wrap or clip a narrow editor inside a larger
pane without changing the solver. ImGui's [constraint discussion][upstream-size]
explicitly considers that alternative. A slot that itself stays fixed or capped
across moves does require a core layout contract.

If maximum/fixed size is a first-release requirement, extend the existing pane
measurement seam and the shared solver. Define these rules first:

- Constraints describe content; chrome is composed by core. Invalid min/max
  combinations are rejected as measurement errors.
- Initially retain the current selected-tab rule. Do not silently intersect
  incompatible inactive-tab constraints.
- A fixed content extent can use equal minimum and maximum. Preferred extent
  is a separate soft policy and should not be disguised as a hard maximum.
- Current layout, future preview, splitter/junction solving and float/native
  preparation must use the same constraints.
- Define what remaining unallocated space looks like and whether it accepts
  docking when all maxima are saturated. Preserve an explicit policy for
  infeasible minima rather than treating smaller host bounds as a fatal error.
- Keep application-supplied measurement metadata separate from durable node
  identity and layout persistence; moving the item must move its constraint.

These are acceptance requirements for a capability, not a request to implement
a general constraint-policy language. Defer the feature if its users do not
need it; otherwise replace the minimum-only public path without a second solver.

## Lifecycle and focus: reduce repeated rules without collapsing states

Hidden content, approved content removal and native window destruction already
have different owners and tests [D3], [D13]. Single-visible-pane tab-bar policy
uses visible membership; turning it into ordinary title chrome would be a
presentation feature, not a reason to create a second topology [D14].

Similarly, several focus records do not imply competing truth. Selected tab,
pane focus request, actual framework focus and native activation are different
facts. `DeferredTabFocus` belongs to the current input run; the framework request
cache survives across runs. Keep their different lifetimes. The useful deletion
is the repeated postcommit handoff predicate in callback and owned integration;
move that rule into the existing focus module without changing when focus is
applied [D15]. This is a small cleanup, not a new state machine.

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

## Implementation order and completion

1. Strengthen the independent semantic test model and required UI outcomes as
   described in [the test proposal][testing]. Check success/rejection as well as
   atomicity, all ordered groups and unaffected state. Keep the existing long
   journey and bounded native schedules.
2. Separate central retention and allocation, with the production measurement
   fixture above, lifecycle/anchor preservation and snapshot tests. Remove the
   overloaded semantics in the same change.
3. Make the small egui focus-handoff cleanup behind the existing focus
   regressions. Preserve each integration's application phase.
4. Add maximum/fixed constraints only as a complete capability with the rules
   above. Extend public host and egui cases before claiming support.

A batch completes when the changed observable contract has a discriminating
test, relevant mutation is caught, fixed regressions and applicable existing
adapter gates pass, and the removed responsibility no longer lives in callers.
State counters should distinguish skipped, expected rejection, applied and
changed actions. No additional number of random seeds proves universal safety.

## Validation from this review

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

## References

[research]: docking-testing-and-imgui.md
[testing]: docking-testing-design-options.md
[publication]: ../adr/0002-atomic-frames-and-presentation.md
[motion]: ../adr/0005-motion-and-presentation-geometry.md
[upstream-rewrite]: https://github.com/ocornut/imgui/issues/7892
[upstream-size]: https://github.com/ocornut/imgui/issues/2849
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
