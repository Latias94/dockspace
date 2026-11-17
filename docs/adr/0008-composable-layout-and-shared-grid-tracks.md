# Composable content, layout tabs and shared Grid tracks

Status: accepted; implemented and integration validated.
Decision date: 2026-11-17.

## Context

Treating every pane as a Tabs leaf prevents an outer tab from selecting a complete
Split or Grid layout. It also makes bare content require invented tabs chrome,
ties content identity to selection, and encourages adapters to choose a descendant
item as the label for a whole group. A Grid lowered into independent row Splits
cannot guarantee that a column boundary moves every row together.

## Decision

The durable tree has four explicit node kinds: Pane, Tabs, Split and Grid. Pane
owns one real application item. Tabs has an ordered roster of typed Item or
Layout children; a Layout child references its actual subtree. Split and Grid
retain their real child owners. Nonempty singleton containers remain explicit
where their container identity carries layout semantics.

Graph structure, selection and presentation facts remain separate. The existing
root structure shares the immutable topology; live workspace selection chooses
the active branch. There is no second mutable presentation tree or authority
registry. Persistence records the complete hidden and inactive graph, titles and
user track shares, while the host supplies current geometry and item sizing.

### Real content and independent chrome

A content scene uses `PaneSceneId { root, item }`. Its immutable record also
freezes the actual immediate owner, offered slot, capped bounds, clip, layer and
active selected path. Bare Pane owns its real Pane node; an inline item owns its
direct Tabs node. Content identity alone cannot revive an old placement after an
item migrates between those owners.

Tabs has its own background/container record. Its header is a separate operation
when visible. Each Tabs layer deducts its own bar once; a bare Pane deducts none.
Every active bare Pane retains local docking targets without a fabricated Tabs
container. A main root with no discoverable bar reserves a separate controls
band outside application content for its layout actions.

Eligible inactive items still supply sizing metadata for candidate layouts.
Only the active branch supplies body callbacks, paint and input receivers. Each
header freezes its ancestors without its own choice, while a body or inner
divider includes its complete selected path. A junction validates every incident
handle's path. Receiver admission also checks actual ownership and structural
currentness against the live workspace.

Labels are frozen typed descriptors. Item labels name the real application item;
a Layout tab uses the referenced node's own title or its Tabs/Split/Grid fallback.
Changing an inner selection does not rename the outer layout tab. Menus,
contained titles, drag decorations and previews consume that same descriptor.
Whole-group preview widths use explicit intrinsic measurement questions; core
does not estimate text or ask an adapter to select a representative descendant.

### Shared rows and columns

Grid retains a row-major child roster, including hidden cells and a sparse final
row. It allocates each column and row once. Every cell at the same coordinate
borrows that shared prefix allocation; there are no hidden row Split nodes or
phantom Pane cells. Track minima use the largest incident active minimum. Caps
use an envelope so a capped cell does not constrain an uncapped peer sharing the
track. Individual content frames still apply their own caps inside their slots.

A 404 by 204 Grid with two rows, two columns and gap 4 therefore has four 200 by
100 cells. Moving the column boundary by 20 gives 220/180 in both rows; moving
the row boundary by 10 gives 110/90 in both columns. These independently specified
values are acceptance requirements for projection and actual input.

Fixed declares its dimensions. Auto scores tight rectangles deterministically
from the durable child count and the local offered aspect after gaps; hidden
cells do not change the shape. Ties prefer fewer unused cells, then the offered
orientation. One provisional offered-slot pass resolves Auto dimensions. That
map is frozen before exact postorder constraints and final allocation, without
a shape/constraint feedback loop. Stored shares are reused only when both axes
match the resolved shape; otherwise both axes use equal defaults.

### Boundary identity and resize

A boundary names its real owner using `SplitterOwner::Split(node)` or
`SplitterOwner::Grid { node, axis }`, plus its persistent boundary index and root.
A Grid boundary freezes both resolved dimensions and the actual child roster,
and spans the owner's complete perpendicular frame. Its arrays describe shared
tracks, rather than one arbitrary incident cell. Intersections use the ordinary
junction protocol.

`TrackResize` carries either Split weights or both Grid share axes. One atomic
batch validates every actual owner and authorizes every changed axis. A row and
column junction merges into a single Grid update. Transient projection overrides
borrow those existing payloads; they do not duplicate gesture authority. A
changed Auto map invalidates old boundary geometry before a new gesture can use
it. Preview geometry and the committed result must agree on the same resolved
map.

Fresh surface measurements may change an Auto Grid's resolved map during a
pointer resize. A changed map of any directly driven Grid cancels the complete
gesture, including both junction axes. Preparation recompiles the same measured
layout without transient shares; successful admission then cancels the exact
session and sample. Rejected or abandoned contributions cannot cancel a gesture.
Fixed grids and unchanged Auto maps retain their frozen resize solver across
measurement changes. Late release cannot commit shares from a cancelled map.

An ancestor resize may reflow Auto descendants while preserving every captured
driver boundary. A direct Grid driver must retain its frozen dimensions and cell
map. After both axes are merged, roots containing Auto reuse their immutable
numeric compiler inputs for one exact projection. The proposed tree must remain
feasible, and each direct boundary must equal its frozen position plus the
accepted displacement. A changed direct Grid shape or a new minimum that moves a
driver rejects the complete batch. This check adds no feedback solver or gesture
authority. Ordinary and Fixed-only roots retain no additional input snapshot.

Old-shape intervals remain conservative; the evaluator does not discover extra
space made available by reflow. A public Auto resize range is returned only when
the common evaluator proves both advertised endpoints. An unprovable endpoint
makes the range unavailable, without searching for a replacement bound. Small
valid actions remain available independently of whether the full range is known.

Preferred and Equalize sample the actual incident pair in the frozen piecewise
plan. A cap may transfer outward capacity to a farther track, so the ordinary
half-difference formula is valid only on the linear path. A junction requires
every incident pair on one axis to choose the same displacement.

An ordered Auto keyboard batch checks each trial against the common evaluator.
A geometrically infeasible atom retains the last accepted displacement; reversal
continues from that displacement. A single infeasible Auto action is unavailable.
Source or policy failure aborts preparation rather than retaining an earlier
step. A pointer release that retains an earlier accepted sample must revalidate
its source, policy and coordinate authority before delivery.

The ordinary flat Tabs/Split path retains its selection and linear-resize fast
paths. A composed motion transition without exact intermediate receiver geometry
settles the complete affected surface before accepting input. This includes bare
Pane nodes and ordinary roots sharing a surface with any composed root.

## Alternatives considered

| Alternative | Decision |
| --- | --- |
| Bare Pane represented as singleton Tabs | Rejected: invents chrome, selection and group semantics for application content. |
| Layout tab labeled by its selected descendant | Rejected: label and resource identity change when an inner tab changes. |
| Grid represented as row Splits | Rejected: columns do not share track geometry or one resize authority. |
| Repeatedly resolve Auto after exact constraints | Rejected: introduces a feedback loop and geometry-dependent instability. |
| Independent mutable presentation ownership registry | Rejected: duplicates the existing immutable compiled facts and graph authority. |

Related: [renderer-neutral core](0001-renderer-neutral-core.md),
[atomic frames and presentation](0002-atomic-frames-and-presentation.md),
[pane sizing and restoration](0007-pane-sizing-and-explicit-preferences.md), and
the [implementation plan](../plans/2026-11-17-1023-refactor-composable-layout-tree-plan.md).
