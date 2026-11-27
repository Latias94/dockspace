# Maintaining Dockspace

## Names and repository structure

The product and GitHub repository are **Dockspace**
([`Latias94/dockspace`](https://github.com/Latias94/dockspace)). The primary
library is UI-framework-neutral; egui is an adapter.

Keep these names distinct:

| Name | Role |
| --- | --- |
| `dockspace` | Core crate and repository slug |
| `dockspace_winit` | Optional window input and repaint helpers for winit hosts |
| `egui_dockspace` | Official egui adapter crate |
| `egui_dockspace_native` | Experimental unpublished native adapter |
| `dockspace_host_conformance` | Shared test-scenario crate |

Keep one repository. Core, adapters and conformance scenarios currently evolve
together, and independent consumer workspaces already test their separation.
Splitting repositories would add release coordination before it has a concrete
benefit. Keep the official and fork-backed Cargo workspaces separate.

Use a concrete GitHub About description, such as "Renderer-neutral docking for
Rust editor UIs, with egui adapters." Suggested topics are `rust`, `docking`,
`egui`, `gui` and `editor`. Point the homepage at a maintained user guide or demo
once one is hosted; avoid linking an internal plan as the product homepage.

## Source modules

Keep state and the operations which advance it together. A file's length alone
does not justify another layer: inspect production code separately from tests.
Use Rust's ordinary `module.rs` and `module/` layout so source navigation follows
module ownership without `#[path]` aliases.

The public entry points are `geometry`, `model`, `policy`, `close`, `host`,
`runtime`, and the optional `motion` module. Keep raw protocols inside their
owning module; do not preserve obsolete root paths with compatibility re-exports.
Private protocol tests use the same internal paths as production code and do not
require test-only public module declarations.

| Area | Owner | Responsibilities |
| --- | --- | --- |
| Durable topology | `dockspace/src/graph/` | Graph storage, commands, checked transactions, canonicalization, validation and read-only queries |
| Ordered input | `dockspace/src/input/` | Provider ingress, pointer journals, receiver receipts and frozen output evidence for one journal reduction |
| Gestures and targeting | `dockspace/src/interaction/` | Gesture state, deadlines, target acquisition and immutable drop geometry/resolution |
| Presentation | `dockspace/src/presentation/` | Layout projection, measurement manifests, compiled scenes, tab strips, exact hit geometry and output observation |
| Platform lifecycle | `dockspace/src/platform/` | Provider authority, effects, observed coordinates, window bindings, focus and viewport coordination |
| Atomic publication | `dockspace/src/engine/` | Reducer transactions, pending mutation progress, logical surface recovery, commit events and resource retention |
| Host integration kit | `dockspace/src/host/` | Three-tier framework-neutral display lists, paint visitors, logical host driver and native multiview helpers |
| Persistence | `dockspace/src/persistence/` | Workspace snapshots, item-key mapping, documents and window placement preferences |
| Native pointer normalization | `egui_dockspace_native/src/pointer_event/` | Device enrollment, mouse button tails and scroll phases before core ingress |
| Framework hit translation | `egui_dockspace_native/src/receiver.rs` | Match exact presented egui artifacts and independently resolve delivery/hover lanes |

Directory grouping does not create another state owner. `DockEngine` remains
the single writer; its presentation authority, pending mutations and platform
coordinator keep their existing publication and cancellation responsibilities.
Shared geometry and stable identities remain small root modules. Public policy
and close modules explicitly export application-facing types while their rule
and plan implementations remain private to the crate.

Keep private tests in the matching `module/tests.rs` or narrowly named child
modules. Test helpers stay private to their module unless multiple production
consumers justify a shared interface. Moving a test must preserve its behavior
and must not widen production visibility just to keep it compiling.

## Documentation ownership

- The root README answers what the project does, how to try it and which host
  to choose. Keep it short and avoid dated implementation history.
- Crate READMEs describe installation, first use and customization. Packaged
  guides retain the complete host contracts.
- `docs/EXAMPLES.md` owns the run-command catalog and explains example limits.
- `CONTRIBUTING.md` owns development commands; CI owns the executable gates.
- `docs/adr/` records the few architectural decisions whose trade-offs are hard
  to recover from code. Keep current contracts in guides, not in old plans.
- Keep build outputs, benchmark samples and dated qualification logs outside the
  source tree. Attach relevant source/environment details to CI artifacts or
  releases. Results do not silently expand platform or performance claims.

## Registry releases

The first registry release includes `dockspace`, `dockspace_winit` and
`egui_dockspace`. Their versions follow `[workspace.package].version`; project
dependencies declare both a local path and the matching registry version. Keep
installation commands and version badges out of user documentation until those
packages actually exist on crates.io.

Use the pinned Rust 1.100 toolchain for release packaging. Cargo can verify and
publish multiple workspace packages in dependency order, including dependencies
not yet published to the registry. Do not maintain a separate temporary registry
or bypass package verification.

### Prepare and verify

1. Finalize package metadata, check declared license texts and third-party
   notices, and prepare release notes for the selected revision.
2. Pass the normal CI gates and the applicable
   [native qualification](PLATFORM_SUPPORT.md). Default and persistence consumers
   must both pass; keep platform claims within the verified scope.
3. Inspect archive contents with `cargo package -p <crate> --all-features
   --locked --list`. Include the core example and guide, the winit host README,
   and the egui application examples and guide.
4. Run the joint dry run from a clean checkout:

   ```sh
   cargo publish --dry-run --locked --all-features \
     -p dockspace -p dockspace_winit -p egui_dockspace
   ```

The [Release workflow](../.github/workflows/release.yml) verifies the same three
packages with `cargo package`, uploads the verified `.crate` archives from
`target/package`, and reuses the full CI workflow. Using the package command
keeps artifact collection on Cargo's documented archive path; publication dry
runs use temporary archives. Dispatch it on `main` with `publish=false` for a
release rehearsal. This default mode does not request a registry token or
upload packages to crates.io.

### First publication and Trusted Publishing

[crates.io currently requires an initial API-token publication](https://crates.io/docs/trusted-publishing)
before a crate owner can configure Trusted Publishing. For the first release,
use a short-lived token with the required publication permissions locally and
run the joint command above without `--dry-run`. Do not commit or log the token.
Verify all three registry packages and their generated documentation before
announcing the release or adding installation links.

After the first publication, configure a trusted publisher for **each** crate:

| crates.io setting | Value |
| --- | --- |
| Repository owner | `Latias94` |
| Repository name | `dockspace` |
| Workflow filename | `release.yml` |
| Environment | `crates-io` |

Create the matching GitHub environment with the desired reviewer and deployment
rules. The workflow grants `id-token: write` only to the publication job and
exchanges its OIDC identity through the pinned official
[`rust-lang/crates-io-auth-action`](https://github.com/rust-lang/crates-io-auth-action).
The action revokes the short-lived token when the job finishes. No long-lived
registry secret is required for subsequent releases.

To publish a subsequent version, update the shared version and matching project
dependency versions, commit, and create `v<version>` on the verified revision.
Dispatch Release at that tag with `publish=true`. The workflow requires the tag
to match the manifest version and its commit to belong to `main` history, then
passes CI and package verification before entering the `crates-io` environment.
Only the three public crates are selected for publication. Tag creation alone
does not publish anything.

### Experimental native adapter and fork distribution policy

`egui_dockspace_native` remains unpublished (`publish = false`) because it depends
on pinned `Latias94/egui` and `Latias94/winit` host contracts absent from the
official registry packages. These include raw event facts, completed-pass hit
authority, pointer capture commands and renderer output settlement. Workspace
`[patch.crates-io]` tables are application-level configuration and are not
inherited by downstream library crates.

Follow this distribution strategy for the `egui` and `winit` forks:

1. **Do not publish renamed forks to crates.io.** Publishing renamed `egui` /
   `eframe` / `epaint` / `winit` crates splits Rust types from the official
   crates.io `egui` ecosystem (`egui_extras`, `egui_plot`, and third-party
   widgets), defeating egui ecosystem compatibility while creating a permanent
   9-crate fork maintenance burden.
2. **Distribute forks via immutable Git tags + `[patch.crates-io]`.** Keep the
   original crate names so downstream applications retain full compatibility
   with crates.io egui widgets:
   - Maintain one release branch per supported upstream minor release on each
     fork (for example `dockspace-0.37` on `Latias94/egui` and
     `dockspace-0.30.13` on `Latias94/winit`).
   - Publish immutable release tags (`v0.37.0-dockspace.<N>` and
     `v0.30.13-dockspace.<N>`) rather than pointing users at moving branches.
   - Keep required native-seam patches clearly separated from optional
     performance experiments before cutting a release tag.
   - Publish a matching compatibility triplet (`dockspace` tag, `Latias94/egui`
     tag, `Latias94/winit` tag) and a copy-paste `[patch.crates-io]` snippet in
     each release note and in `crates/egui_dockspace_native/README.md`.
3. **Sync only on stable upstream minor releases.** Do not continuously track
   upstream `master`. Rebase/align when upstream cuts a stable minor release and
   re-verify `native-qualification.yml` before tagging the new triplet.
4. **Shrink the fork surface over time.** Upstream small, self-contained fixes
   first (starting with platform pointer/capture correctness fixes in `winit`
   and low-invasion observation/performance improvements in `egui`), and prefer
   moving window-loop integration out of the `eframe` fork where `dockspace_winit`
   plus `egui-wgpu` can own it directly.

Removing `publish = false` from `egui_dockspace_native` requires a fully
registry-resolvable dependency graph without workspace patches and verification
from a clean external consumer. Keep the locked native workspace and platform
qualification in place until the required host contracts exist upstream. The
first Dockspace registry release includes only `dockspace`, `dockspace_winit`
and `egui_dockspace`.
