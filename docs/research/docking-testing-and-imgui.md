# Docking reliability and ImGui design references

Research checkpoint: 2026-11-16. The user requested preservation of the supplied
testing and architecture investigation before further refactoring. The
[two original messages](docking-testing-and-imgui-source.md) are archived verbatim;
their claims are research input, not an adopted specification.

## Testing decisions

Keep exploration, reduction, regression and platform qualification distinct.
Random input cannot exhaust the reachable docking and native lifecycle states.
A discovered seed is evidence of a failure; several seeds can share one root
cause. Preserve a named minimal regression for each confirmed root instead of
treating each additional seed as another independent defect.

The existing test layers already cover different seams:

| Layer | Existing basis | Bounded improvement |
| --- | --- | --- |
| Core commands | `command_model.rs`: 5,000 operations, semantic postconditions, strict topology and membership checks, rejected-transaction atomicity | Short proptest sequences with logical item selectors and standard shrinking |
| Geometry and input properties | `model_properties.rs` and `pointer_journal_properties.rs` | Preserve independent invariants and existing regression persistence |
| Real egui widgets | Workbench journeys, product harness and adapter regressions | Keep the full ordered trace, including failing and idle packets; manually reduce failures to real-widget regressions |
| Native protocols | Coordinator journeys and bounded producer interleavings | Preserve producer order and original affine owners; inject real late results at explicit checkpoints |
| Physical backends | Existing X11/macOS qualification drivers | Verify actual focus, hit testing, placement, capture and presentation in the owning backend |

The command-sequence shrinker must re-resolve current source and target authority
from stable logical items. If shrinking removes a prerequisite, skip that exact
action; do not retarget it or replace it with a selection. The short sequences
do not replace long-run operation coverage. A core shrinker cannot reduce an egui
hit-test failure or qualify native OS callbacks.

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
- [Docking tests](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_suite/imgui_tests_docking.cpp),
  [viewport tests](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_suite/imgui_tests_viewports.cpp)
  and the [mock backend](https://github.com/ocornut/imgui_test_engine/blob/main/shared/imgui_app.cpp)
  provide named semantic checks and mock platform operation. The local reference
  backend still lists resistance to move/resize as a testing gap; it is not a
  complete window-manager oracle.

The supplied observations about [#9123](https://github.com/ocornut/imgui/issues/9123),
[#2849](https://github.com/ocornut/imgui/issues/2849), the current upstream CI
matrix, and the latest release status require separate verification before they
justify implementation changes or public claims.

## Comparison with Dockspace

| Concern | Current Dockspace contract | Remaining question |
| --- | --- | --- |
| Long-term docking intent | `DockAnchor::Item` and `DockAnchor::Central`; current-candidate resolution, no public persistent NodeId target | Add directional/largest/recent-region queries only for a concrete application need |
| Group capability | Captured group scope is checked; moving its former item anchor must not silently redirect it | Preserve these semantics through future conveniences |
| Central behavior | Ordinary tab roots are supported; `CentralNodePolicy` separates merge/split permissions | `central_tabs` still combines retaining an empty leaf with remaining-space allocation |
| Pane sizing | Public host measurement supplies `PaneMinimum` | A general pane maximum/fixed-size policy and tab-constraint conflicts are not yet an adopted contract |
| Visibility | Logical item visibility is distinct from item removal; shared visibility scenarios check preserved layout and restored controls | Verify single-visible-pane chrome against explicit product expectations before changing it |
| Native resources | Typed effects and observed bindings are distinct from semantic publication and first-live presentation | Continue testing denied/adjusted placement and late platform callbacks in each real backend |
| Motion | Logical targets and immutable animated output geometry are distinct | Overlap input must choose executable semantics consistently with the exact presented stack |

There is no evidence here that a second layout tree, universal host trait or new
mock-window-manager layer is required. A concrete need for an empty central
anchor with proportional growth, or a fixed-width Inspector across moves, would
justify separating empty-anchor retention from space-allocation policy. Define
constraint aggregation, conflict behavior and split/merge inheritance before
adding such a contract. The project is unreleased, so a necessary breaking change
can replace the existing semantics rather than adding compatibility layers.

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
