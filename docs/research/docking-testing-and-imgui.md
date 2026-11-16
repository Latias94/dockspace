# Docking reliability and ImGui design references

Research checkpoint: 2026-11-16. The initial architecture/test review used
Dockspace `c9dd58a84bf7ba8b4d78e16e7ca9ab33ba08f218`. Its findings and mutation
measurements are historical baseline evidence. The subsequent implementation
adopted the independent short semantic oracle, mandatory workbench outcomes,
explicit central allocation and the small egui focus-handoff cleanup. Source
coverage below describes those changes; recorded baseline test counts do not
qualify the new implementation.

The user requested preservation of the supplied investigation before further
refactoring. The [two original messages](docking-testing-and-imgui-source.md) are
archived verbatim; their claims are research input, not an adopted specification.

The follow-up [architecture review](docking-architecture-review.md) compares
these concerns with the current implementation, separates adopted contracts from
remaining capabilities, and maps them to ImGui's proposed rewrite concerns. The
[test design record](docking-testing-design-options.md) preserves the original
oracle gap and proposal alongside the implementation and follow-up mutation.
Its [restored implementation gates](docking-testing-design-options.md#restored-implementation-validation)
record the final verification separately from the baseline experiments.
ImGui's candidate directions remain research input, not a finalized v3 contract.

## Testing decisions

Keep exploration, reduction, regression and platform qualification distinct.
Random input cannot exhaust the reachable docking and native lifecycle states.
A discovered seed is evidence of a failure; several seeds can share one root
cause. Preserve a named minimal regression for each confirmed root instead of
treating each additional seed as another independent defect.

The existing test layers already cover different seams:

| Layer | Existing basis | Bounded improvement |
| --- | --- | --- |
| Core commands | `command_model.rs`: 5,000 operations, semantic postconditions, strict topology and membership checks, rejected-transaction atomicity | Adopted: short proptest sequences with independent ordered groups, selection, full local MRU, hidden state, expected rejection and `changed`; logical selectors and standard shrinking |
| Geometry and input properties | `model_properties.rs` and `pointer_journal_properties.rs` | Preserve independent invariants and existing regression persistence |
| Real egui widgets | Workbench journeys, product harness and adapter regressions | Adopted: required docking/divider/junction/floating-edge outcomes and mixed keyboard/AccessKit geometry checks; preserve the complete diagnostic trace |
| Native protocols | Coordinator journeys and bounded producer interleavings | Preserve producer order and original affine owners; inject real late results at explicit checkpoints |
| Physical backends | Existing X11/macOS qualification drivers | Verify actual focus, hit testing, placement, capture and presentation in the owning backend |

The command-sequence shrinker must re-resolve current source and target authority
from stable logical items. If shrinking removes a prerequisite, skip that exact
action; do not retarget it or replace it with a selection. The short sequences
do not replace long-run operation coverage. A core shrinker cannot reduce an egui
hit-test failure or qualify native OS callbacks.

### Adopted tests and their limits

The short [semantic model](../../crates/dockspace/src/behavior_tests/command_model/shrinkable/semantics.rs)
predicts each action before production authority is resolved. The
[executor/properties](../../crates/dockspace/src/behavior_tests/command_model/shrinkable.rs)
compare every ordered group, logical and visible selection, full local MRU and
hidden membership, then require the predicted result class and `changed` value.
Missing model prerequisites may be skipped after shrinking; missing SUT authority
for a modeled prerequisite is a failure. Rejections are independently predicted,
not accepted merely because state stayed unchanged. A constructive property
always exercises a valid distinct-group center move. The follow-up mutation
record in the [test design note](docking-testing-design-options.md#follow-up-mutation-evidence)
shows both short properties now catch the previously accepted blanket rejection.

Workbench [move expectations](../../crates/egui_dockspace/examples/workbench/tests/randomized/semantics.rs)
check exact payload order, destination selection, source remainder, hidden state
and unrelated groups. The
[fixed docking journey](../../crates/egui_dockspace/examples/workbench/tests/randomized/docking.rs)
requires exposed center/edge drops, actual floating and redocking. Dedicated
[resize fixtures](../../crates/egui_dockspace/examples/workbench/tests/randomized/resize.rs)
require divider movement, both axes of a junction, and all four cardinal floating
edges. They check direct-manipulation geometry and keyboard/AccessKit progress,
including exact combined steps where the fixed roomy fixture determines them.
[Mixed semantic input](../../crates/egui_dockspace/examples/workbench/tests/randomized/semantic_input.rs)
compares batched versus separate ordered events and requires actual progress when
a numeric control has room.

Adaptive exploration distinguishes `NotApplicable`, `Blocked` and `Performed`.
`Blocked` is a diagnostic result, not proof that the operation was validly denied
or that its interaction entry works. Required fixed fixtures must return
`Performed`; prior random successes cannot excuse a missing mandatory control.
Source coverage is not final gate evidence or physical backend qualification.

Classify stalled cases against the owning contract:

- A recoverable cancellation must release its gesture and permit fresh input.
- A pending native transfer may legitimately wait for its exact presentation or
  effect result after the pointer becomes idle.
- Permanently contradictory ingress preserves the rejected FIFO head and follows
  the failed-generation contract. It must not fabricate successful drain.

Useful invariants include unique item ownership, valid topology, atomic candidate
publication, exact presented receiver authority, irreversible revocation of stale
callbacks, and semantic persistence round trips. Apply each at its proper
checkpoint. Animated presentation rectangles can overlap; they need not form a
strict split partition at every sample.

Complete a review batch when its minimal regressions, fixed journey corpus,
relevant bounded schedules and adapter gates pass. Broaden exploration for a new
capability or missing transition, rather than increasing seed count indefinitely.
No numeric reliability claim follows from the current corpus. The executable
policy lives in [adapter testing](../ADAPTER_TESTING.md#exploration-reduction-and-completion).

## Verified ImGui evidence

Primary sources were checked on 2026-11-16. These sources explain design problems
and candidate directions; they do not establish a finalized Docking v3 design.

- [Issue #7892, sections 6.4–6.6](https://github.com/ocornut/imgui/issues/7892)
  is the **2024-08-15** anniversary discussion. It describes docking identity,
  builder and central-node problems; considers windows dictating node behavior;
  and separately discusses multi-viewport changes. Its existence is not evidence
  of a completed v3 implementation in 2026.
- [Omar's #5161 response](https://github.com/ocornut/imgui/issues/5161#issuecomment-1087337677)
  explains why arbitrary split/merge makes long-term node meaning ambiguous.
  Finding a current node by the desired location is suggested instead of relying
  on a historical node ID. A persistent tag also does not guarantee location or
  membership.
- [Omar's #5209 response](https://github.com/ocornut/imgui/issues/5209#issuecomment-1102272275)
  explains central-node remaining-space allocation and why split direction
  changes which window inherits it. Disabling central nodes and splitting their
  properties are proposed; window-driven behavior is a candidate direction.
- [Docking tests](https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_docking.cpp),
  [viewport tests](https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_viewports.cpp)
  and the [mock backend](https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/shared/imgui_app.cpp)
  provide named semantic checks and mock platform operation. The local reference
  backend still lists resistance to move/resize as a testing gap; it is not a
  complete window-manager oracle.

- [Issue #9123](https://github.com/ocornut/imgui/issues/9123) contains Omar's
  2026-01-08 explanation that a lone visible window can retain its docking
  membership while ideally appearing as a standalone window. He identifies
  existing implementation complexity; this does not prescribe another topology.
- [Issue #2849](https://github.com/ocornut/imgui/issues/2849) discusses incompatible
  per-window constraints, selection-dependent node resizing, and constraining
  content inside an unchanged docking slot as an alternative. These are product
  choices; a pane maximum must not silently acquire one of these meanings.

The current upstream CI matrix and latest release status were not requalified
for the follow-up architecture review. Do not infer them from the supplied
investigation or the pinned local references.

## Comparison with Dockspace

| Concern | Current Dockspace contract | Remaining question |
| --- | --- | --- |
| Long-term docking intent | `DockAnchor::Item` and `DockAnchor::Central`; current-candidate resolution, no public persistent NodeId target | Add directional/largest/recent-region queries only for a concrete application need |
| Group capability | Captured group scope is checked; moving its former item anchor must not silently redirect it | Preserve these semantics through future conveniences |
| Declarative builder and persistence | Complete declarations compile fresh runtime structure; snapshots use local node numbers | Reopening a removed item still needs explicit placement; stable content identity is not an automatic historical-slot lookup |
| Central behavior | Adopted: `central_tabs` retains the anchor; default `RootAllocation::Weighted` and explicit `PreferCentral` select allocation; `CentralNodePolicy` separately controls merge/split | PreferCentral requires an anchor. It redistributes constraint deltas, not fixed-width sidebars. Snapshots require the field, with no legacy fallback |
| Pane sizing | Public host measurement supplies complete `PaneSizing`; selected visible tab defines the leaf's min/max/fixed bounds; preferences apply on explicit restoration | [ADR 0007](../adr/0007-pane-sizing-and-explicit-preferences.md) defines slot/frame background and infeasible minima. Resize across capped contact changes and general tab-conflict aggregation remain outside that contract |
| Visibility | Hide retains logical placement; close removes it. `AutoHideSingle` uses visible membership | This is not universal ordinary-window title chrome; reusing an item ID does not recreate its removed slot |
| Native resources | Typed effects and observed bindings are distinct from semantic publication and first-live presentation | Continue testing denied/adjusted placement and late platform callbacks in each real backend |
| Motion | Logical targets and immutable animated output geometry are distinct | Overlap input must choose executable semantics consistently with the exact presented stack |

The [current central tests](../../crates/dockspace/src/behavior_tests/central_allocation.rs)
measure both allocation choices on both axes, with empty/occupied and nested
anchors, minimum constraints, lifecycle and persistence. The
[future-layout test](../../crates/dockspace/src/presentation/compiler/future_layout/tests/allocation.rs)
compares preview content bounds with committed geometry. Hidden-anchor tests
cover both strategies and restoration. The frozen solver/traversal is retained
unchanged as an optimization comparison, with explicit fixtures for the prior
central preference.

There is no evidence here that a second layout tree, universal host trait or new
mock-window-manager layer is required. The adopted central split removes a
concrete overloaded contract. The subsequent sizing follow-up uses the same
measurement seam and solver, with selected-tab bounds, capped frames and explicit
preference restoration under ADR 0007. It leaves general conflict aggregation and
resize through capped contact changes outside the adopted capability.

### Multi-viewport boundaries

The current native contracts distinguish requests from observations, semantic
placement from physical lifetime, and Created/Prepared from exact first-live
admission. Focus dispatch alone does not establish observed focus. The
[native platform boundary](../../crates/egui_dockspace_native/README.md#run-the-editor)
still excludes managed Wayland lifecycle/cross-window docking. Windows physical
cross-window input, physical drag-out creation and arbitrary mixed-monitor DPI
need further validation; macOS stationary cross-window dwell/autoscroll lack
capture/delivery authority. Finite X11/macOS qualification does not establish
all-platform support. Logical rootless surfaces are distinct from an egui native
application with no root OS viewport; that adapter still requires its root.

## Related regression coverage

The reproduced defects were reduced to the following contracts and regressions:

1. Retire physical endpoints without reclaiming presentation streams that a
   legitimate pending child still references.
2. Preserve ordered resize/scroll steps and menu terminals within mixed input
   packets, including reversals at constrained endpoints.
3. Respect a newer accepted framework focus instead of replaying an older pane
   focus side effect.
4. Reject unrepresentable occluded guide placements without discarding valid
   smaller candidates or hiding intrinsically invalid geometry.
5. Resolve executable click/drag overlaps using the exact presented stack; keep
   real framework evidence and do not hide a core conflict in an adapter blocker.
6. Keep the shrinkable command model and complete timed UI trace alongside the
   fixed journeys and applicable workspace/consumer gates. Test outcomes must be
   rechecked against the current revision; a recorded result does not qualify a
   different platform or future change.

Further research should start with the remaining questions in the comparison
table. It should not assume that ImGui's exploratory v3 direction is a required
feature or that passing mock tests expands platform support.
