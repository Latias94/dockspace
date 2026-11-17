# Godot docking design and Dockspace responsibilities

Research checkpoint: 2026-11-17. Godot source is pinned to
`203bf0a23fd7d90a860917e66de08e20d2596244` (2026-11-16).
Dockspace references describe the composable Pane/Tabs/Split/Grid working tree
reviewed at this checkpoint, not the earlier foundation commit.
This comparison used local history and source; neither application was built or
launched. It identifies design boundaries and product differences, not new test
results or measured interaction quality.

## What changed in Godot

The main rework is [Rework editor docks][rework], committed 2025-09-29 and merged
2025-10-01 in PR #106503. It introduces `EditorDock` and moves tool metadata and
plugin integration into that object. Its author date is 2025-05-16; that date
should not be mistaken for when the change landed.

[Replace DockSlot with DockTabContainer][containers], committed 2026-01-29,
extracts common slot behavior. [Refactor dock slot selector][selector], committed
2026-02-16 and merged 2026-02-17, extracts the miniature selector's drawing,
input and rectangle cache. Its author date is 2025-12-27.

These changes unify editor tools and placement controls. They do not introduce
a viewport-independent composable layout graph. Godot's [slot selector][slots]
uses editor-defined positions; `DockSlotGrid` selects those
positions. Its name does not imply shared row/column tracks like Dockspace Grid.

## Identity and lifecycle

Godot's live dock identity is an `EditorDock` object. Its title, persistence key,
default slot, transient/global flags and window wrapper are separate fields.
The [public tool contract][dock-api] distinguishes registering a plugin tool,
closing it, reopening it, and freeing it when the plugin unloads.
The persistence key can fall back to the displayed title; Dockspace should keep
its stronger explicit external-key association instead of adopting that fallback.

| Concern | Dockspace owner | Consequence |
| --- | --- | --- |
| Application content | `ItemId` and the application's pane catalog | Renaming or moving a tool does not change its identity. |
| Direct tab | `TabKey::Item` or a layout key | A layout tab owns a complete child layout, not a representative descendant. |
| Live container | Opaque `NodeKey` | Useful for exact queries/capture; not a permanent application location. |
| Topology and presentation | `RootId`, `SurfaceId`, `FloatingPresentationId` | Moving a complete root and retiring a native presentation are separate operations. |
| Persistent content keys | [ExternalItemKeyMap][item-keys] | Closed content can reopen with the same item identity. |
| Movable scope | [PreparedTabsGroup and PreparedTab][actions] | Stored actions cannot retarget a former descendant after membership changes. |

The equivalent of a registered Godot tool is an application catalog entry plus
its `ItemId` and provider state. It does not require an arena node for each
ordinary tab or a separate persistent container registry.

## Close, hide, activation and recovery

Godot's [manager lifecycle][manager] retains a closed tool object under a hidden
parent and remembers its slot and tab index. The Editor Docks menu can reopen it
or focus it. Closing a floating window returns a dock to its remembered slot
when one exists. This tool-panel path is distinct from document-save approval.

Dockspace intentionally provides separate contracts:

- Hide retains ownership and placement; showing does not implicitly activate
  every ancestor or focus a native window.
- SelectItem atomically selects the real Tabs ancestry and rejects hidden content.
- Content close removes ownership after the application's decision. Exact group,
  layout-tab, root and surface scopes include hidden and inactive descendants.
- A layout tab containing central cannot close; whole-group content close can
  retain the protected central path.
- Reopen retains the external identity but requires an explicit placement.

These responsibilities belong to [graph operations][graph], [close scopes][close]
and the [public action API][actions]. A permanent tool editor can implement
reversible dismissal with hide/show and explicit activation. Document closes
must retain their existing approval and ownership-removal semantics.

Native window close already has [an explicit adapter policy][native-close].
The default recovers managed child presentation; an external root retains its
logical layout for application shutdown. Cancel, RetainLayout and CloseContent
are explicit alternatives. Ordinary content close is not native-window close.

## Composition and floating groups

Godot's inspected [window creation path][windows] wraps one `EditorDock` in each
floating `WindowWrapper`. That tool can contain complex UI, but its controls are
not thereby first-class movable layout groups in the editor dock manager.
Its miniature selector and margin drops target existing slots.

Dockspace's [declaration API][layout] owns a broader structural contract:

- Pane is bare content without an implied tab bar.
- Ordinary Tabs inline real items; composed Tabs retain actual layout children.
- Split is an ordered N-ary partition; Grid has one row-major roster and shared
  tracks, with Fixed or Auto dimensions.
- Explicit container titles belong to the layout, independent of selected content.
- Subtree/root center docking retains one layout tab; explicit group merge moves
  direct children. Alt-preserve chooses the former for a group gesture.

Do not flatten floating layouts, introduce proxy items, or add implicit headers
to imitate Godot's simpler tool-manager scope. Owned content and active content
remain separate queries, including during source retirement and native recovery.

## Persistence and editor state

Godot saves slot rosters, selected tab indices, closed tools, floating rectangles
and split offsets. It also invokes tool-specific config hooks. Missing registered
tools can be skipped; disabled/transient tools have editor-specific restore rules.
The surrounding editor supplies autosave, named presets and default restoration.

Dockspace's [atomic document][document] binds the strict v1 graph, external-key
history, generation and native placement preferences. Restoring validates a
candidate before publication. Saved placement hints do not grant live window or
input authority. The [persistence example][presets] demonstrates named presets
and explicit reopen using these public APIs.

The application owns content data, tool availability, file paths and autosave
timing. Godot's permissive config loading is not a reason to silently drop
unknown content, normalize malformed graphs, or add a legacy reader to strict v1.

## Default UX and concrete product differences

| Capability | Existing implementation | Remaining responsibility |
| --- | --- | --- |
| Pointer docking | Complete-output single-window host or `run_single_surface` | A plain egui callback lacks output authority for drag docking; choose the appropriate host. |
| Native windows | Separate fork-backed `egui_dockspace_native` adapter | Packaging/integration remains experimental and unpublished; do not imply default availability. |
| Tool recovery | Stable keys, visibility, activation, reopen and reset APIs | The editor shell supplies an always-available closed/hidden tool inventory and any deleted-location history. |
| Layout presets and tool state | Atomic document/reset APIs and examples | Application storage and per-tool state schemas remain outside graph. |
| Placement discovery | Guides and exact root/group/tab/pane menus | A miniature selector or global tool shortcut catalog is optional editor UX. |
| Grid creation | Declarative Grid and shared-track interaction | Default menus do not author a new Grid; Godot's selector does not provide this either. |

The [egui host contract][egui-host] documents the complete-output requirement.
Default scope-specific menus already expose actions for bare panes and complete
layouts. `PaneView`, `DockStyle`, `DockInputConfig` and `DockPolicy` provide paint,
sizing, shortcut and permission customization without transferring input authority.
Godot's fixed slots, plugin role flags and singleton manager belong to its editor;
they are not missing renderer-neutral graph features.

The transferable priorities are honest capability presentation, clear action
scope, an application-owned recovery path, and source-aware floating geometry.
This source comparison establishes no new P1 core defect or need for another
identity layer, viewport registry, compatibility reader or fixed-slot model.

[rework]: https://github.com/godotengine/godot/commit/97b398cba15d66ebb20c44a4deead469d897a5e0
[containers]: https://github.com/godotengine/godot/commit/4274f8b543b1680d90000278d7c984da718bb7f4
[selector]: https://github.com/godotengine/godot/commit/1e8c4d395fc1dc4d0fa2b7bda655ec5f9b4c2678
[manager]: https://github.com/godotengine/godot/blob/203bf0a23fd7d90a860917e66de08e20d2596244/editor/docks/editor_dock_manager.cpp#L662
[slots]: https://github.com/godotengine/godot/blob/203bf0a23fd7d90a860917e66de08e20d2596244/editor/docks/editor_dock_manager.cpp#L1061
[windows]: https://github.com/godotengine/godot/blob/203bf0a23fd7d90a860917e66de08e20d2596244/editor/docks/editor_dock_manager.cpp#L283
[dock-api]: https://github.com/godotengine/godot/blob/203bf0a23fd7d90a860917e66de08e20d2596244/doc/classes/EditorDock.xml#L76
[item-keys]: ../../crates/dockspace/src/persistence/item_key.rs
[actions]: ../../crates/dockspace/src/model/action.rs
[graph]: ../../crates/dockspace/src/graph/operation/topology.rs
[close]: ../../crates/dockspace/src/close/plan/content.rs
[native-close]: ../../crates/egui_dockspace_native/src/close_control.rs
[layout]: ../../crates/dockspace/src/model/layout.rs
[document]: ../../crates/dockspace/src/persistence/document.rs
[presets]: ../../crates/egui_dockspace/examples/workspace_persistence.rs
[egui-host]: ../../crates/egui_dockspace/src/lib.rs
