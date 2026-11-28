//! Renderer-neutral JSON layout persistence with stable external pane keys.
//!
//! Demonstrates how to decouple saved layouts from application pane content
//! without any UI framework or windowing dependency:
//! - Allocate persistent pane identities through `DockspaceDocumentBootstrap`.
//! - Mutate layout topology and observe `report.notification().document_changed()`.
//! - Save JSON via `DockspaceSession::save_document_json`.
//! - Cold-start restore via `DockspaceSession::from_document_json` and reconnect
//!   application data through `item_id_for_external_key`.
//! - Reject untrusted external keys cleanly without partial state changes.
//!
//! Run from the repository root:
//! `cargo run -p dockspace --example persistence --features serde --locked`

use dockspace::model::{
    DockAnchor, DockPlacement, DockspaceAxis, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{
    DockspaceDocumentBootstrap, DockspaceDocumentId, DockspaceSession, SurfaceUnavailableReason,
};

const SURFACE: SurfaceId = SurfaceId::new(1);
const ROOT: RootId = RootId::new(1);
const DOCUMENT_ID: DockspaceDocumentId = DockspaceDocumentId::from_bytes(*b"core-persist-doc");
const KNOWN_KEYS: [&str; 3] = ["pane/outline", "pane/editor", "pane/console"];

fn run_persistence_round_trip() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // 1. Register stable application keys before building the initial layout.
    let mut bootstrap = DockspaceDocumentBootstrap::new(DOCUMENT_ID);
    let outline = bootstrap.ensure_item("pane/outline")?;
    let editor = bootstrap.ensure_item("pane/editor")?;
    let console = bootstrap.ensure_item("pane/console")?;

    let split = DockspaceNode::split(
        DockspaceAxis::Horizontal,
        [
            (DockspaceNode::tabs([outline]), 0.25),
            (DockspaceNode::central_tabs([editor, console]), 0.75),
        ],
    )?;
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(ROOT, split),
    )])?;
    let mut session =
        DockspaceSession::from_persistent_layout(layout, DockPolicy::default(), bootstrap)?;

    // 2. Move Console into Outline's group and hide Outline while keeping its slot.
    let mut frame = session.begin_host_frame()?;
    frame.dock_item_current(console, DockPlacement::Center(DockAnchor::Item(outline)))?;
    frame.set_item_visibility_current(outline, false)?;
    frame.complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)?;
    let report = frame.commit()?;
    assert!(report.notification().document_changed());

    // 3. Serialize only layout structure and external keys (never pane business data).
    let json = session.save_document_json()?;

    // 4. Cold-start restore into a brand-new session, validating allowed keys.
    let restored =
        DockspaceSession::from_document_json(&json, DockPolicy::default(), |document_id, key| {
            document_id == DOCUMENT_ID && KNOWN_KEYS.contains(&key)
        })?;
    let restored_outline = restored
        .item_id_for_external_key("pane/outline")
        .expect("outline key restored");
    let restored_console = restored
        .item_id_for_external_key("pane/console")
        .expect("console key restored");
    assert!(!restored.view().item(restored_outline).unwrap().is_visible());
    assert_eq!(
        restored
            .view()
            .item(restored_console)
            .unwrap()
            .tabs()
            .expect("console is in a tabs group")
            .items()
            .collect::<Vec<_>>(),
        vec![restored_outline, restored_console]
    );

    // 5. Rejecting an unrecognized key fails cleanly without accepting the document.
    let rejected = DockspaceSession::from_document_json(&json, DockPolicy::default(), |_, key| {
        key != "pane/console"
    });
    assert!(rejected.is_err());

    Ok(json)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = run_persistence_round_trip()?;
    println!(
        "Saved and restored persistent dockspace document ({} bytes).",
        json.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run_persistence_round_trip;

    #[test]
    fn core_persistence_example_round_trips_and_validates_keys() {
        let json = run_persistence_round_trip().expect("core persistence round-trip succeeds");
        assert!(!json.is_empty());
    }
}
