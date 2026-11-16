# Docking test design record after the ImGui comparison

Research checkpoint: 2026-11-16. **The original proposal and measurements describe
a historical baseline; the adopted implementation is recorded first below.**
This note builds on [the earlier investigation][R1] and
[the adapter testing guide][R2]. Baseline source inspection used Dockspace
`c9dd58a84bf7ba8b4d78e16e7ca9ab33ba08f218` and local ImGui Test Engine reference
`16b6efbb393fdef0dcac9b0794a8ee476606a152`. The subsequent implementation adopted
the independent short semantic oracle, mandatory workbench outcomes, explicit
central allocation and the small egui focus-handoff cleanup. The coordinating
agent supplied the targeted mutation results. Historical restored test counts
remain unchanged and must not be read as verification of the new implementation.
No physical backend qualification was performed for this note.

## Adopted implementation

### Independent short-command expectations

The new test-local [semantic model][C15] stores complete ordered groups, logical
selection, full local MRU, a retained central-group marker and hidden membership.
It does not reconstruct split topology or the constraint solver. It independently
predicts valid success/no-op, expected rejection and `changed` before the
[executor][C1] resolves fresh production authority for the same logical items.
Every ordered group, selection/history and hidden set is compared after each
step, including unaffected groups and visible-selection fallback.

The action grammar now includes visibility alongside select, reorder, open,
center/edge moves and resize. Rejections cover hidden selection, duplicate open
and out-of-range insertion, with the predicted error class/parameters. Model
prerequisites removed by shrinking may yield `Skipped`; a modeled item missing
from the SUT cannot. Coverage distinguishes skipped, expected rejection, applied
and changed actions. An all-skipped candidate is not fabricated success. A
separate constructive property guarantees an initial legal distinct-group center
move and checks that the always-owned source/target remain operable after a
generated prefix.

This closes the measured blanket-rejection gap below. It is still a deliberately
small default-policy semantic model, not a second docking/native engine. The
long command journey, explicit edge geometry checks, named MRU regressions,
frozen traversal comparisons and ordered native schedules remain useful.

### Required real-widget outcomes

Workbench [semantic expectations][W4] compare exact moved payload/order,
destination selection, source remainder, hidden members and unaffected groups,
plus unrelated floating geometry. The fixed [docking test][W6] requires actual
center/edge drops, floating and redocking in its unobscured fixture.

The [resize tests][W5] require an exposed divider and junction, check move-frame
versus release geometry, enforce clamps/perpendicular-axis preservation where
applicable, and require both axes to change for a corner drag. Required fixtures
exercise all four cardinal floating edges through pointer input and real egui
keyboard/AccessKit events. They require the known combined step and preserve
unrelated main content. Separate junction-axis fixtures require keyboard plus
AccessKit to advance exactly the requested axis.

[Mixed input batches][W2] compare the resulting layout with the same ordered
events delivered separately, preserve content/selection and require actual
progress for a numeric control with room. Consumption or equivalence alone can
no longer satisfy that progress check.

This stronger check found a production defect under minimum-size constraints:
the final pane rectangles matched, but a batched discrete round trip retained
latent weights while separately committed steps saved the achieved proportions.
The two-pane named regression
`clamped_mixed_resize_round_trip_preserves_frame_independent_durable_shares`
reproduced the failure before the fix (nextest run
`96f5a25f-5f06-4645-823d-39d0387b2e4e`); its wholly blocked counterpart passed.
The required contract compares durable size intent too, since later window
growth and persistence can expose this difference. A continuous pointer gesture
returning to its press position still restores its original weights. Final
verification of the production correction is recorded with the delivery gates.

Adaptive random operations report `NotApplicable`, `Blocked` or `Performed`.
`Blocked` remains diagnostic: it is not an independently justified rejection or
proof of a usable entry path. Mandatory fixed scenarios must return `Performed`;
a missing required guide/divider/edge cannot be excused by an earlier success.
The timed string trace remains diagnostic, without a general packet replayer.

Strengthening the outcome checks also exposed a driver error: a globally
nearest guide with the requested label could belong to a foreground floating
root. The test-only workbench hooks now observe public guide/preview data for
the current completed pass. The driver checks destination root, pane, direction,
action and active eligibility before release. A named overlapping-root fixture
first requires source acquisition, then verifies that targeting the covered main
pane cancels without releasing into the floating guide. This corrects the
driver's precondition; it does not establish a production wrong-target defect.

### Explicit central allocation

The [architecture review][R3] records the adopted production contract:
`central_tabs` retains an empty semantic anchor; default `RootAllocation::Weighted`
and explicit `PreferCentral` independently select constraint redistribution;
`CentralNodePolicy` separately controls merge/split. PreferCentral requires an
anchor. Required snapshot allocation, fingerprints, root movement and lifecycle
preserve the choice without a legacy default.

[Measured-pane tests][C16] use the discriminating weighted/central fixture on
both axes, including nested and empty/occupied anchors. [Preview tests][C17]
compare future content bounds with committed content bounds. Hidden/restored
central allocation is exercised under both choices [C18]. The frozen reference
implementation was left unchanged; prior central fixtures select PreferCentral
explicitly. This does not expose maximum/fixed/preferred pane sizes.

## Follow-up mutation evidence

The coordinating agent repeated the temporary blanket rejection of every
`DockTarget::Center` with seed `1340171702` (nextest run
`2f75e4ef-e19d-47ac-82a5-3c3892700ee8`). Both short properties **failed** with
`model-valid CenterMove ... was rejected`. The constructive property reduced to
`prefix = []`; its fixed first operation is the legal cross-group move
`item 3 -> item 1`. The ordinary property reduced to one legal no-op,
`CenterMove item 1 -> item 1`. That ordinary minimal counterexample does not
establish a cross-group move, but correctly rejects blanket denial of a valid
no-op too.

Production bytes were restored exactly in cleanup, and no new persisted
failure-seed file was created. This is evidence that the new oracle catches this
specific mutation. The restored implementation gates below are separate evidence.

## Restored implementation validation

The final implementation was checked on 2026-11-16, after the production resize
correction and the workbench target-admission/module extraction changes. Cargo
builds ran serially with two build jobs; nextest used two test threads. The
workspace toolchain is Rust 1.100.0. Consumer harness rows explicitly identify
their Rust 1.95.0 compatibility runs. Counts overlap across configurations and
must not be added as distinct scenarios.

| Configuration | Result | Nextest run ID |
| --- | --- | --- |
| Workspace, all features and all targets | 2,898 passed, 3 skipped | `ec2950a7-dffa-4588-8b60-d043203b1d84` |
| Core library, no default features | 2,270 passed, 3 skipped | `6f0d5e29-c7ad-46cf-b15b-7a1129f6793c` |
| egui library, no default features | 331 passed | `2971b2e4-f125-44df-a57f-ec052f5dfd14` |
| Native workspace, all features and all targets | 824 passed, 2 skipped | `089cfd98-d762-4b37-a24c-2764a5bd4ef2` |
| Public host harness, default features, Rust 1.95.0 | 411 passed | `eb2bd635-87fd-4d5b-a512-6d706e6fffb8` |
| Public host harness, serde and motion, Rust 1.95.0 | 471 passed | `6bd00d48-56fe-447a-b0b3-285d498bcbc1` |
| Official egui harness, all features and all targets, Rust 1.95.0 | 41 passed | `313e8bb3-0134-4083-b001-9b3e4b48c38c` |
| Product harness, default features and all targets, Rust 1.95.0 | 142 passed | `778a69f1-65de-4066-8e5a-8a14b8ba1f67` |
| Product harness, serde and all targets, Rust 1.95.0 | 147 passed | `9c8efe7e-0dc9-40f0-802d-637e4c113eb3` |

These restored gates pass both the clamped mixed-input regression that failed
before the production fix and the two short properties that failed under the
temporary center-rejection mutation. The mutation is absent from the final code.

Additional passing checks: workspace correctness/suspicious Clippy gates;
warnings-as-errors rustdoc for the three public crates; 17 workspace doctests;
the core/egui wasm32 library build with egui serde; native library compilation
without default features; root/native formatting; core dependency boundaries;
and local file targets in the changed Markdown. Clippy's selected-category gate
is not a claim of zero warnings. No new physical OS qualification was performed.

## Original recommended direction (historical)

First, give short command properties an independent semantic oracle: ordered
tab groups, selection, visibility and unaffected groups, with success/rejection
predicted before executing production code. Keep tree validation and geometry
assertions as additional checks. A measured mutation below demonstrates why
membership and atomicity alone are insufficient, while also showing that the
existing long journey detects that particular failure.

Next, require admitted workbench interactions to produce their intended payload,
selection and geometry. Reuse existing public scenarios and thin executors for
scope/visibility combinations. Later, compose explicit noncompliant platform
observations with the existing native schedules. These are implementation stages,
not severity levels. No general testing framework, second layout engine,
universal GUI protocol or authoritative coverage registry is proposed.

## Architecture questions translated into discriminating tests

The upstream concerns about historical node identity, central allocation and
window-driven policy directions remain research input, not a completed Docking
v3 design or a requirement for Dockspace to adopt those policies [R1].

| Concern | Discriminating Dockspace test | Evidence and boundary |
| --- | --- | --- |
| Historical node identity after split/merge | Resolve a current item/central anchor after topology changes; reject a captured group whose scope changed; preserve unrelated groups. | Existing same-candidate/captured-group tests [C8], [C5]; ImGui's tab-amend test describes ambiguous node ownership [U1]. Preserve logical semantics without requiring stable internal tree shape. |
| Central retention versus remaining-space allocation | Both axes/directions, nested central, empty/occupied central, then hide/show and viewport growth/restoration; assert actual extents under the current policy. | Dockspace already has numeric central/ancestor tests [C9]. ImGui disables some horizontal resize expectations behind `IMGUI_BROKEN_TESTS` [U2]. This does not establish a new fixed/max-pane contract. |
| Visible entry versus logical payload | Compare item, group grip/trailing space, complete contained title and application action with hidden siblings; check exact payload, source remainder, order, selection and extent. | ImGui substitutes node/window operations and distinguishes desired/current hidden-payload behavior [U3]. Dockspace already tests partial-payload geometry and complete-title movement [C10]; extend state combinations. |
| Selection versus usable focus | Send keyboard input to a concrete pane widget after each entry; distinguish header focus, activation and native observed focus. | ImGui compares content/tab/title/menu entries but also disables focus expectations [U4]. Dockspace has pane-focus regressions [C10] and dispatch-versus-focus tests [N4]. |
| Requested versus observed platform behavior | Independently vary requested/actual placement and focus; delay/drop exact outputs before admitting a child. | ImGui's mock stores requested position/size/focus directly and leaves resistance as TODOs [U6]. Dockspace already distinguishes placement, Created/prepared/first-live and focus dispatch [N3], [N4], [C7]. |

## Baseline evidence and oracle gaps (historical)

### Commands: independent membership is not complete semantic expectation

The 5,000-step command journey checks strict topology, exact membership,
operation-specific outcomes, edge direction/fraction, and at least one successful
state change per operation. The short property adds ordinary proptest shrinking
and failure persistence [C1], [C2]. These are meaningful checks worth retaining.

Its independent expected state is nevertheless only `BTreeSet<ItemId>`.
`ModelDelta` comes from the actual workspace used to generate the command.
`CenterMove` checks reported item/root, target containment and recapturability,
without full source/target order, selection/MRU or unaffected-group comparison.
Every short-property `Err` is accepted if the workspace is unchanged [C1], [C2].

The coordinating agent measured a temporary production mutation at the start of
`topology.rs::move_payload`: every `DockTarget::Center` returned
`Err(CommandError::Invariant)` [C12]. Nextest run
`9e2b7727-081c-4f78-bdd7-cc82482ceb61` produced:

| Target | Measured result | Interpretation |
| --- | --- | --- |
| Short command property | **PASS**, 128 cases, fixed seed `1340171702` | The property accepted blanket center rejection under this run. |
| 5,000-step command journey | **FAIL**, `command_model.rs:749`, `committed > STEPS * 9 / 10` | The long journey caught the degraded success rate. This is not a blind spot of the entire suite. |
| Normalized central numeric probe | **PASS** | This mutation does not invalidate that sizing probe; a passing geometric check does not establish command success. |

An additional temporary deterministic witness used the initially legal center
move `item 3 -> item 1`, whose source and target are distinct groups. It confirmed
the injected error was reached, then called the existing short checker: that
checker returned `true` and all its assertions passed with the workspace
unchanged. The witness and short property both passed in nextest run
`13a7bfcf-846c-41b4-b049-81b9a427dc04`.

All temporary mutation/probe/witness source was removed. The final restored
command-model, layout-solver and frozen-traversal selection passed **50 tests**
with all features and the same seed (nextest run
`6c8ee9f0-fb61-42d4-af66-ad3fa25dcba0`). These are test-oracle measurements, not a
discovered production bug or a reliability percentage. At that historical
checkpoint the independent oracle proposed below had not been implemented; its
adopted implementation and new mutation evidence are recorded above.

Other mutation thought experiments remain **hypothetical**:

| Fault | Why the baseline short property could pass | Required check |
| --- | --- | --- |
| Move an unrelated unselected tab as well | Requested item still reaches the target; topology and global membership can remain valid. | Compare every ordered group, including outside the action's write set. |
| Corrupt destination selection or source order/MRU | Target containment and recapturability do not establish those semantics. | Independently predict activation and successor/history; existing MRU regressions specify the distinction [C6]. |
| Return a false edge no-op | The assertion branches on production `changed`; an unchanged workspace satisfies its no-op branch. | Require a change for independently known distinct-source/target cases. Keep the existing four-direction/three-fraction fixture, which already does so [C2]. |
| Execute no actions | `Select(9)` in the initial fixture resolves to `None`; the loop ignores `false`. | Distinguish skips from applied transitions; separately construct guaranteed-applicable cases. |

The long generator also falls back from exhausted opens to select, and from a
missing splitter to open/select. Counters describe the resulting `ModelDelta`,
not independently requested operation/state pairs. More seeds cannot add close,
visibility, group/root payloads, policies or stale captures to this grammar
[C1], [C2]. This limits generated combinations, not the named tests elsewhere.

### Properties: repeatability and conservation leave semantic freedom

`model_properties.rs` checks axis determinism, min/max bounds and finite
nonnegative accounting/conservation, but a wrong proportional allocation can
satisfy those conditions. Its axis calls omit a central index. Its generated
workspace projection uses chain-shaped layouts, loose uniform constraints and
one large viewport, comparing two calls to the same projector [C3].

These limits do not describe the full geometry suite. Test-only `reference.rs`
retains 871 lines of frozen pre-refactor solver/traversal [C13].
`topology_tests.rs` compares random central/noncentral axis inputs with the frozen
solver, including bitwise size/overflow/unallocated checks. Its dense-layout
property generates N-ary depth-three trees, optional central leaves and complete
min/max constraints, comparing constrained and minimum-only projections with the
frozen traversal [C14].

Keep that implementation as an optimization regression oracle for now. It proves
equivalence to prior behavior, not correctness of new allocation semantics, and
must not be updated alongside production merely to restore green tests. For a
deliberate allocation change, first add explicit formulas for the new contract,
then decide which frozen comparisons and old code can be pruned.

Existing numeric tests already cover weights, clamps, overflow, saturation and
nested central behavior [C9]. Add bounded generated cases with elementary
answers: unconstrained proportions, two/three children with one saturated child,
and a simple central remainder. Also assert those answers through measured panes;
do not implement another general constraint solver as the oracle.

Pointer-journal properties generate terminal `StreamCancelled` segments for
rollback, retry watermarks and incarnation compaction. Their fixed seed and
nonpersistent grammar [C4] do not supply generated press/drag/release coverage.

### Shared scenarios: identical labels can hide unequal checks

Semantic aliases/capabilities, admission before input and independently resolved
controls are sound foundations [S1]. The recording consumer uses public prepared
actions and exact presentation settlement [S2]; egui drives real semantic
controls [S3]. Visibility checks actual content/restored geometry [S6], and L03
checks basin, selection, payload order, one release mutation and quiescence [S4].

Oracle strength still varies. Core L08 verifies an outcome before appending
`step.expected()`; its splitter branch only checks the first weight grew.
The egui L08 splitter check is also directional. Close checks a first plan item
rather than uniformly requiring exact scope and unchanged content. The external
consumer requires a one-item close plan and both splitter directions [S2], [S3],
[S5]. Appending expected labels after assertions is not inherently wrong, but
the final label comparison cannot supply an omitted assertion.

`SplitterAdjusted(Horizontal)` does not distinguish a wrong step, cross-axis
movement or unrelated-window drift. Check declared relations, preserved geometry,
limits and entry-path equivalence; do not invent a universal pixel step where
the contract permits adapter-specific values. Keep measurements in thin executors.

### Workbench: adaptive exploration can tolerate a broken branch

The workbench uses actual egui widgets and samples completed hits at the output's
own time. It checks uniqueness, recovery paths and floating extent [W1], but:

- Missing source/guide can return `false`; the main loop usually ignores the
  split/merge/float/redock result. Whole-journey minimums only require one earlier
  split, merge and float, so later state-specific failures may pass.
- Split success uses group count; merge uses containment; float uses contained
  ownership. These do not establish exact payload, selection or extent.
- Resize may return without an exposed control; once started, it checks gesture
  acquisition/termination without the expected geometry. Mixed batches check
  consumption/focus without their numeric result [W1], [W2].

Existing deterministic tests already compare batched/separate inputs, reversals,
limits and discarded passes [W3]. Reuse their semantic checks on changing layouts.

`Replay` stores `Vec<String>` with failing and idle packets, then prints on panic.
The seed reruns adaptive choices; no trace parser/executor exists, and its two
warm-up frames are outside the trace [W1]. It is a diagnostic trace, not directly
replayable input. A small test-local packet executor can help reduction if needed.

### Native: strong schedules with deliberately supplied platform facts

The three-window journey requires all directed pairs, delivery every round,
cancellation, preserved anchors and drained backlog. The bounded schedules cover
25 producer-preserving orders × two endpoints × six changes × two results =
600 cases, checking exact settlement/cancellation, revocation and a fresh dock
[N1], [N2]. Preserve affine owners; generic callback deletion would weaken them.

This journey derives coordinates from public plans and supplies clear hover
lanes plus clip/artifact window hits. It injects global focus before release and
requires no focus effects [N1], [N5]. It cannot qualify real hit-graph/OS overlap,
denied focus or adjusted child placement. Its trace retains only 20 entries and
formats diagnostics rather than replaying callbacks [N1].

It is nevertheless false that all native doubles blindly grant requests.
Existing tests dispatch focus without observing it, fail creation, preserve
fractional requested placement without fabricated geometry, and separate Created,
preparation and first-live [N3], [N4], [C7]. Actual egui/callback regressions also
exist [R2]. Improve composition and state variation rather than recreating them.

ImGui's mock immediately assigns position/size/focus, always reports nonminimized
and DPI 1, and provides no physical-presentation evidence; moving/resizing
resistance remains TODO [U6]. Viewport tests are not registered without enabled
viewports and both backend flags [U5]. Green counts need configuration context.

## Original model design: deliberately smaller than the engine

This section preserves the design that motivated the implementation above.
The delivered model compares full local MRU rather than only selected-item
membership. Broader public-session scope/policy combinations remain follow-up
work; this section does not imply they are already generated.

Use test-local ordered groups, logical selection, visibility and presentation
aliases; represent a retained empty central slot by a fixture alias. Add a small
MRU list only where successor/close semantics require it [C6]. Compare complete
ownership and unaffected groups, preserving tab order within each group.

Start with select/reorder/open/center under default policy. Retain edge/resize
checks and add success requirements for unambiguous fixtures. Add visibility
through a small public-session property using existing scenario aliases.
The public contract distinguishes logical/visible selection and preserves hidden
membership, tab order, weights and ownership [C11]. `is_selected()` cannot stand
in for both forms of selection.

For each requested action:

1. **Determine preconditions from the independent expected model, never the
   SUT.** Only a prerequisite absent from that model after shrinking permits
   `Skipped(MissingPrerequisite)`. Intentional stale capture or denied policy
   predicts rejection; a valid fresh action predicts success/no-op.
2. Resolve production authority for the same logical selector. **If the model
   expects an item/group/target but the SUT cannot resolve it, fail, never skip.**
   Do not retarget or refresh the expected state from the observed workspace.
3. Compare result class and stable rejection reason, exact predicted semantics
   on success, and unchanged semantics on rejection. Then check graph validity
   and report/state consistency independently.
4. Count requested, skipped, expected-rejected, applied and changed separately.
   At the baseline, `apply_action == true` included rejected transactions [C1].

Keep all-skipped shrink candidates valid and visibly accounted; do not retarget
`Select(9)` or fail merely because a prerequisite was removed. Add separate
constructive fixtures that guarantee applicable center success, expected rejection
and visibility transitions. Avoid all-sequence `prop_assume`, which conceals
vacuity and disrupts reduction.

The oracle does not construct split trees, solve constraints or model receivers,
native identities or affine tokens. Normalizing actual SUT state is necessary;
deriving expected preconditions or results from it defeats independence.

## Implementation stages and current status

| Stage | Delivery status | Acceptance or remaining boundary |
| --- | --- | --- |
| Independent expectations | Adopted: flat ordered groups/selection/full MRU/hidden state, expected rejection and `changed`; existing proptest shrinking and long journey retained | Both short properties catch blanket center rejection. Generated visibility uses the command seam; this does not claim every public-session policy/scope combination is generated. |
| Mandatory admitted UI semantics | Adopted: explicit attempt results, independent move expectations and required docking/divider/junction/four-edge fixtures | Fixed scenarios require success. Random `Blocked` counts are diagnostics. Move/release geometry and keyboard/AccessKit progress have explicit checks. Final restored gates are separate evidence. |
| Scope/visibility combinations | Partly covered by the model and existing named/scenario tests; broader combined generation remains optional | Preserve exact ownership/order/selection and hidden members. Actual UI input and capability admission need their own assertions; labels alone are insufficient. |
| Numeric allocation | Adopted: measured central weighted/preferred fixtures, lifecycle/persistence/fingerprint tests and future-preview comparison; shared solver retained | The elementary discriminating fixture establishes this contract. It does not establish a new fixed/max-size capability or a general independent constraint solver. |
| Packet replay | Not implemented; diagnostic timed trace retained | Add a test-local exact packet executor only if reduction needs it. No cross-version protocol or serialized authority. |
| Composed platform results | Existing named noncompliant-result tests and ordered schedules retained; further composition remains follow-up | Do not fabricate focus, geometry or first-live. Preserve source ownership until exact admission and respect each cancellation/failure contract. |
| Physical backend qualification | No new physical qualification claimed by this note | Actual placement/focus/input/presentation qualifies only the exercised backend/configuration. |

Semantic assertions precede replay investment. The historical production
mutation/restored baseline and follow-up mutation are recorded separately above.
Further optional oracle checks may corrupt unrelated membership/selection or consume resize
without its semantic change. Each must fail its owning test; no mutation
framework is needed.

## Completion and unsettled product policy

Use the existing completion/gate policy [R2]: targeted regressions/properties,
fixed journeys and relevant bounded schedules/adapter gates, with serial bounded
Cargo work. Record backend/configuration; more seeds alone are not completion.
Recoverable cancellation, legitimate pending external results and permanently
rejected FIFO ingress have different terminal contracts; retain those distinctions.

Fixed/max/preferred pane sizing is adopted by the subsequent
[sizing decision](../adr/0007-pane-sizing-and-explicit-preferences.md), with
selected-tab constraints and explicit preference restoration. General tab
conflict aggregation and piecewise capped-edge resizing remain outside that
contract. Empty-central retention/allocation is separate, with the adopted
contract and tests above. Define each new observable
result before adding its discriminating fixture. Stronger test oracles do not
make an unimplemented product capability part of the public contract.

## Source references

Local links identify the relevant source file or contract in the working tree.
Historical sections describe those files at the baseline revision recorded at
the top; the adopted sections refer to the subsequent implementation. ImGui
links are pinned to the inspected reference revision so a later upstream edit
does not silently change the cited evidence.

[R1]: docking-testing-and-imgui.md
[R2]: ../ADAPTER_TESTING.md
[R3]: docking-architecture-review.md
[C1]: ../../crates/dockspace/src/behavior_tests/command_model/shrinkable.rs#L62
[C2]: ../../crates/dockspace/src/behavior_tests/command_model.rs#L506
[C3]: ../../crates/dockspace/src/behavior_tests/model_properties.rs#L8
[C4]: ../../crates/dockspace/src/behavior_tests/pointer_journal_properties.rs#L150
[C5]: ../../crates/dockspace/src/behavior_tests/product_actions/tabs_group.rs#L109
[C6]: ../../crates/dockspace/src/behavior_tests/tab_mru.rs#L146
[C7]: ../../crates/dockspace_host_conformance/src/tests.rs#L1397
[C8]: ../../crates/dockspace/src/behavior_tests/product_actions.rs#L1587
[C9]: ../../crates/dockspace/src/behavior_tests/layout_solver.rs#L35
[C10]: ../../integration/egui-product-harness/tests/product/drag_entry_paths.rs#L157
[C11]: ../../crates/dockspace/src/model/layout.rs#L416
[C12]: ../../crates/dockspace/src/graph/operation/topology.rs#L236
[C13]: ../../crates/dockspace/src/presentation/layout/reference.rs#L1
[C14]: ../../crates/dockspace/src/presentation/layout/topology_tests.rs#L55
[C15]: ../../crates/dockspace/src/behavior_tests/command_model/shrinkable/semantics.rs
[C16]: ../../crates/dockspace/src/behavior_tests/central_allocation.rs
[C17]: ../../crates/dockspace/src/presentation/compiler/future_layout/tests/allocation.rs
[C18]: ../../crates/dockspace/src/presentation/layout/visibility_tests.rs#L115
[S1]: ../../crates/dockspace_host_conformance/src/scenario.rs#L1
[S2]: ../../integration/dockspace-host-harness/tests/shared_local_scenario/main.rs#L180
[S3]: ../../integration/egui-product-harness/tests/product.rs#L4726
[S4]: ../../integration/egui-product-harness/tests/product/magnetic.rs#L596
[S5]: ../../crates/dockspace_host_conformance/src/tests.rs#L1031
[S6]: ../../integration/egui-product-harness/tests/product/visibility.rs#L240
[W1]: ../../crates/egui_dockspace/examples/workbench/tests/randomized.rs#L17
[W2]: ../../crates/egui_dockspace/examples/workbench/tests/randomized/semantic_input.rs#L9
[W3]: ../../crates/egui_dockspace/src/facade/chrome_tests/splitters.rs#L165
[W4]: ../../crates/egui_dockspace/examples/workbench/tests/randomized/semantics.rs
[W5]: ../../crates/egui_dockspace/examples/workbench/tests/randomized/resize.rs
[W6]: ../../crates/egui_dockspace/examples/workbench/tests/randomized/docking.rs
[N1]: ../../crates/egui_dockspace_native/src/coordinator/tests/docking_enabled/randomized.rs#L17
[N2]: ../../crates/egui_dockspace_native/src/coordinator/tests/docking_enabled/randomized/last_pane/presentation_order/endpoint_revocation/schedules.rs#L1
[N3]: ../../crates/egui_dockspace_native/src/coordinator/tests/ingress_create.rs#L730
[N4]: ../../crates/egui_dockspace_native/src/coordinator/tests/focus.rs#L527
[N5]: ../../crates/egui_dockspace_native/src/coordinator/tests/docking_enabled.rs#L553
[U1]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_docking.cpp#L2294
[U2]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_docking.cpp#L1802
[U3]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_docking.cpp#L1932
[U4]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_docking.cpp#L1316
[U5]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/imgui_test_suite/imgui_tests_viewports.cpp#L44
[U6]: https://github.com/ocornut/imgui_test_engine/blob/16b6efbb393fdef0dcac9b0794a8ee476606a152/shared/imgui_app.cpp#L1474
