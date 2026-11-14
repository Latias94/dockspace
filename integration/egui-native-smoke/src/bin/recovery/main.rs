//! Real-window fatal teardown followed by a fresh native application.
//!
//! Input consists of public application commands. This does not qualify physical
//! mouse delivery, cross-monitor DPI transitions or OS close-button requests.

mod journey;

use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use dockspace::model::{
    DockAnchor, DockPlacement, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, ItemId, PreparedTabsGroup, RootId, SurfaceId,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::{DockspaceSession, SurfaceUnavailableReason};
use eframe::egui;
use egui_dockspace::{DockStyle, PaneView};
use egui_dockspace_native::NativeDockspaceApp;

const ROOT: SurfaceId = SurfaceId::new(1);
const ITEMS: [ItemId; 4] = [
    ItemId::new(1),
    ItemId::new(2),
    ItemId::new(3),
    ItemId::new(4),
];

#[derive(Clone)]
struct Panes(Arc<Mutex<BTreeMap<ItemId, String>>>);

impl PaneView for Panes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        lock(&self.0)
            .contains_key(&item)
            .then(|| format!("Recovery {item}").into())
    }

    fn ui(&mut self, item: ItemId, ui: &mut egui::Ui) -> egui_dockspace::PaneResponse {
        ui.text_edit_multiline(lock(&self.0).get_mut(&item).unwrap());
        egui_dockspace::PaneResponse::default()
    }
}

type App = NativeDockspaceApp<Panes>;
type SharedApp = Arc<Mutex<App>>;

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

fn policy() -> DockPolicy {
    let mut policy = DockPolicy::default();
    policy.set_allow_native_surfaces(true);
    policy
}

fn app(layout: DockspaceLayout, panes: Panes) -> Result<App, Box<dyn Error>> {
    Ok(NativeDockspaceApp::new(
        egui::Id::unique("native-recovery"),
        DockspaceSession::from_layout(layout, policy())?,
        ROOT,
        panes,
        DockStyle::default(),
    )?)
}

fn run(app: SharedApp, fail: bool, stale: Option<PreparedTabsGroup>) -> Result<(), Box<dyn Error>> {
    let outcome = Arc::new(Mutex::new(None));
    let context = Arc::new(Mutex::new(None));
    let observed_context = Arc::clone(&context);
    let handler = lock(&app).native_host_handler();
    let options = eframe::NativeOptions {
        native_host: Some(Arc::clone(&handler)),
        viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 520.0]),
        ..Default::default()
    };
    let state = journey::Journey::new(Arc::clone(&app), Arc::clone(&outcome), fail, stale);
    eframe::run_native(
        "Dockspace recovery qualification",
        options,
        Box::new(move |creation| {
            *lock(&observed_context) = Some(creation.egui_ctx.clone());
            Ok(Box::new(state))
        }),
    )?;
    if fail {
        let context = lock(&context)
            .take()
            .expect("the real host attached its egui context");
        // Native destruction can enqueue final lifecycle facts while the event
        // loop exits. Complete those service boundaries before replacing it.
        for _ in 0..64 {
            if lock(&app).is_shutdown_drained() {
                break;
            }
            if handler.on_service(&context) == eframe::NativeHostWake::Wait {
                break;
            }
        }
    }
    lock(&outcome)
        .take()
        .unwrap_or_else(|| Err("the native application exited before the journey completed".into()))
        .map_err(|error| io::Error::other(error).into())
}

fn exercise() -> Result<(), Box<dyn Error>> {
    let panes = Panes(Arc::new(Mutex::new(
        ITEMS
            .into_iter()
            .map(|item| (item, format!("edited document {item}")))
            .collect(),
    )));
    let expected = lock(&panes.0).clone();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        ROOT,
        DockspaceRootLayout::new(RootId::new(1), DockspaceNode::tabs(ITEMS)),
    )])?;
    let old = Arc::new(Mutex::new(app(layout, panes.clone())?));
    let stale = lock(&old).capture_tabs_group(DockAnchor::Item(ITEMS[0]))?;
    run(Arc::clone(&old), true, None)?;

    let (cause, checkpoint) = {
        let old = lock(&old);
        let cause = old.with_errors(|primary, cleanup| {
            assert!(cleanup.is_empty(), "fatal cleanup failed: {cleanup:?}");
            format!(
                "{:?}",
                primary.expect("the first journey must inject a fatal error")
            )
        });
        assert!(
            old.is_shutdown_drained(),
            "shutdown is not drained: {:?}",
            old.pending_work()
        );
        assert_eq!(old.output_backlog().ordinary(), 0);
        assert_eq!(old.output_backlog().control(), 0);
        (cause, old.with_view(|view| view.to_layout()))
    };
    assert_eq!(*lock(&panes.0), expected);

    // This application's explicit recovery policy brings orphaned logical
    // surfaces into the Root. Native hosting is re-established by new commands;
    // a logical checkpoint does not carry old OS-window ownership.
    let mut session = DockspaceSession::from_layout(checkpoint, policy())?;
    let mut frame = session.begin_host_frame()?;
    for item in ITEMS {
        if frame.view().item(item).unwrap().surface() != ROOT {
            frame.dock_item_current(item, DockPlacement::After(ITEMS[0]))?;
        }
    }
    frame.complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)?;
    frame.commit()?;
    let recovered = NativeDockspaceApp::new(
        egui::Id::unique("recovered-native-app"),
        session,
        ROOT,
        panes.clone(),
        DockStyle::default(),
    )?;
    let recovered = Arc::new(Mutex::new(recovered));
    run(Arc::clone(&recovered), false, Some(stale))?;
    let recovered = lock(&recovered);
    recovered.with_errors(|primary, cleanup| {
        assert!(
            primary.is_none() && cleanup.is_empty(),
            "new runtime failed: {primary:?}, {cleanup:?}"
        )
    });
    assert_eq!(recovered.output_backlog().ordinary(), 0);
    assert_eq!(recovered.output_backlog().control(), 0);
    assert_eq!(*lock(&panes.0), expected);
    recovered.with_view(|view| {
        for item in ITEMS {
            assert_eq!(view.item(item).unwrap().surface(), ROOT);
        }
        let mut items = view.item(ITEMS[0]).unwrap().tabs().items().to_vec();
        items.sort_unstable();
        assert_eq!(items, ITEMS);
    });
    lock(&old).with_errors(|primary, cleanup| {
        assert_eq!(format!("{:?}", primary.unwrap()), cause);
        assert!(cleanup.is_empty());
    });
    println!(
        "recovery-smoke: PASS; two real native hosts; fatal cleanup, fresh windows, stale authority rejection and retained documents"
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let (finished, completion) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if matches!(
            completion.recv_timeout(Duration::from_secs(60)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            eprintln!("recovery-smoke: process watchdog expired");
            std::process::exit(1);
        }
    });
    let result = exercise();
    let _ = finished.send(());
    watchdog.join().expect("the process watchdog must join");
    result
}
