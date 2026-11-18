use dockspace::geometry::PhysicalRect;

use dockspace::model::{
    DockAnchor, DockMoveSource, DockMoveTarget, DockPlacement, DockspaceGridShape, DockspaceLayout,
    DockspaceNode, DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, NativeWindowPlacement,
    RootId, SurfaceId, TabGroupDockMode,
};
use dockspace::policy::DockPolicy;
use dockspace::runtime::DockspaceSession;
use eframe::egui;
use egui_dockspace::{DockStyle, PaneResponse, PaneView};
use egui_dockspace_native::{NativeActionRequestError, NativeCommand, NativeDockspaceApp};

const SURFACE: SurfaceId = SurfaceId::new(1);
const FIRST: ItemId = ItemId::new(1);
const SECOND: ItemId = ItemId::new(2);

struct Panes(String);

impl PaneView for Panes {
    fn title(&self, _item: ItemId) -> Option<egui::WidgetText> {
        Some("Command consumer".into())
    }

    fn ui(&mut self, _item: ItemId, ui: &mut egui::Ui) -> PaneResponse {
        PaneResponse::from(&ui.text_edit_multiline(&mut self.0))
    }
}

fn app() -> NativeDockspaceApp<Panes> {
    app_with_panes(Panes("unsaved application text".to_owned()))
}

fn app_with_panes<P: PaneView + Send + 'static>(panes: P) -> NativeDockspaceApp<P> {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(RootId::new(1), DockspaceNode::central_tabs([FIRST, SECOND])),
    )])
    .unwrap();
    let session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        egui_dockspace::DockStyle::default()
            .presentation_config()
            .unwrap(),
    )
    .unwrap();
    NativeDockspaceApp::new(
        egui::Id::unique("public-native-command-consumer"),
        session,
        SURFACE,
        panes,
        DockStyle::default(),
    )
    .unwrap()
}

#[test]
fn boxed_dynamic_provider_works_with_the_public_native_owner() {
    let panes: Box<dyn PaneView + Send> = Box::new(Panes("dynamic provider".to_owned()));
    let app = app_with_panes(panes);
    assert_eq!(
        app.with_panes(|panes| panes.title(FIRST).unwrap().text().to_owned()),
        "Command consumer"
    );
    let receipt = app
        .request_command(NativeCommand::SelectItem { item: SECOND })
        .unwrap();
    assert_eq!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    assert_eq!(
        app.request_command(NativeCommand::SelectItem { item: FIRST })
            .map(|_| ())
            .map_err(|error| error.reason()),
        Err(NativeActionRequestError::Busy)
    );
}

#[test]
fn a_stored_command_shares_bounded_admission_without_mutating_application_data() {
    use std::error::Error;

    let app = app();
    let command = NativeCommand::Move {
        source: DockMoveSource::Item(FIRST),
        target: DockMoveTarget::Contained { surface: SURFACE },
    };

    let receipt = app.request_command(command.clone()).unwrap();
    let error = app.request_command(command.clone()).unwrap_err();
    assert_eq!(error.reason(), NativeActionRequestError::Busy);
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<NativeActionRequestError>(),
        Some(&NativeActionRequestError::Busy)
    );
    assert_eq!(error.into_command(), command);
    assert_eq!(
        app.request_command(NativeCommand::Move {
            source: DockMoveSource::Item(FIRST),
            target: DockMoveTarget::Dock {
                placement: DockPlacement::Center(DockAnchor::Item(SECOND)),
                mode: TabGroupDockMode::MergeTabs
            }
        })
        .map(|_| ())
        .map_err(|error| error.reason()),
        Err(NativeActionRequestError::Busy)
    );
    assert_eq!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    assert_eq!(
        app.with_panes(|panes| panes.0.clone()),
        "unsaved application text"
    );
    assert_eq!(
        app.with_view(|view| view.item(FIRST).unwrap().root()),
        RootId::new(1)
    );
}

#[test]
fn stored_native_payload_commands_and_group_capture_share_the_public_slot() {
    let placement =
        NativeWindowPlacement::new(PhysicalRect::new(900.0, 100.0, 480.0, 320.0).unwrap());
    for kind in 0..6 {
        let app = app();
        let group = app.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
        let source = match kind {
            0 | 3 => DockMoveSource::Item(SECOND),
            1 | 4 => DockMoveSource::Tabs(group.clone()),
            _ => DockMoveSource::Root(RootId::new(1)),
        };
        let command = NativeCommand::Move {
            source,
            target: DockMoveTarget::Native {
                placement: (kind < 3).then_some(placement),
            },
        };
        let receipt = app.request_command(command.clone()).unwrap();
        assert_eq!(
            app.request_command(command)
                .map(|_| ())
                .map_err(|error| error.reason()),
            Err(NativeActionRequestError::Busy)
        );
        assert_eq!(
            app.request_command(NativeCommand::SelectItem { item: FIRST })
                .map(|_| ())
                .map_err(|error| error.reason()),
            Err(NativeActionRequestError::Busy)
        );
        assert_eq!(
            app.capture_tabs_group(DockAnchor::Item(SECOND)).unwrap(),
            group
        );
        assert!(
            app.capture_tabs_group(DockAnchor::Item(ItemId::new(99)))
                .is_err()
        );
        assert_eq!(
            receipt.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        assert_eq!(
            app.with_view(|view| view
                .item(FIRST)
                .unwrap()
                .tabs()
                .expect("item is an inline tab")
                .items()
                .collect::<Vec<_>>()),
            [FIRST, SECOND]
        );
        assert_eq!(
            app.with_view(|view| view.item(SECOND).unwrap().surface()),
            SURFACE
        );
        assert_eq!(
            app.with_panes(|panes| panes.0.clone()),
            "unsaved application text"
        );
    }
}

#[test]
fn another_apps_captured_group_uses_the_same_public_command_slot() {
    let source = app();
    let destination = app();
    let group = source.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap();
    let original = group.clone();
    let receipt = destination
        .request_command(NativeCommand::Move {
            source: DockMoveSource::Tabs(group),
            target: DockMoveTarget::Native { placement: None },
        })
        .unwrap();
    assert_eq!(
        destination
            .request_command(NativeCommand::SelectItem { item: SECOND })
            .map(|_| ())
            .map_err(|error| error.reason()),
        Err(NativeActionRequestError::Busy)
    );
    assert_eq!(
        source.capture_tabs_group(DockAnchor::Item(FIRST)).unwrap(),
        original
    );
    assert_eq!(
        destination.with_view(|view| view.item(FIRST).unwrap().surface()),
        SURFACE
    );
    assert_eq!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    assert!(destination.error_kind().is_none());
}

#[test]
fn public_layout_tab_capture_is_read_only_and_busy_admission_returns_the_exact_command() {
    let bare = ItemId::new(3);
    let hidden = ItemId::new(4);
    let tools = DockspaceNode::grid(
        DockspaceGridShape::Auto,
        [
            DockspaceNode::tabs([SECOND, hidden]),
            DockspaceNode::pane(bare),
        ],
    )
    .unwrap()
    .named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            RootId::new(1),
            DockspaceNode::composed_tabs([DockspaceNode::pane(FIRST), tools]),
        ),
    )])
    .unwrap()
    .with_hidden_items([hidden])
    .unwrap();
    let style = DockStyle::default();
    let session = DockspaceSession::from_layout_with_presentation_config(
        layout,
        DockPolicy::default(),
        style.presentation_config().unwrap(),
    )
    .unwrap();
    let app = NativeDockspaceApp::new(
        egui::Id::unique("public-composite-tab-command"),
        session,
        SURFACE,
        Panes("unsaved application text".to_owned()),
        style,
    )
    .unwrap();
    let key = app.with_view(|view| {
        view.root(RootId::new(1))
            .unwrap()
            .content()
            .unwrap()
            .tabs()
            .unwrap()
            .tabs()[1]
    });
    let tab = app.capture_tab(key).unwrap();
    assert_eq!(tab.key(), key);
    assert!(app.capture_tab(bare.into()).is_none());
    assert!(app.capture_tab(FIRST.into()).is_some());
    assert!(
        !app.pending_work()
            .contains(egui_dockspace_native::NativeWaitReason::ApplicationAction)
    );
    let receipt = app
        .request_command(NativeCommand::Move {
            source: DockMoveSource::Tab(tab.clone()),
            target: DockMoveTarget::Contained { surface: SURFACE },
        })
        .unwrap();
    assert_eq!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    let command = NativeCommand::CloseTab { tab: tab.clone() };
    let error = app.request_command(command.clone()).unwrap_err();
    assert_eq!(error.reason(), NativeActionRequestError::Busy);
    assert_eq!(error.into_command(), command);
    assert_eq!(app.capture_tab(key), Some(tab));
    assert_eq!(
        app.with_view(|view| view
            .root(RootId::new(1))
            .unwrap()
            .items()
            .collect::<Vec<_>>()),
        [FIRST, SECOND, hidden, bare]
    );
    assert!(app.with_view(|view| view.item(hidden).unwrap().is_hidden()));
    assert_eq!(
        app.with_panes(|panes| panes.0.clone()),
        "unsaved application text"
    );
}

#[test]
fn dropping_the_runtime_disconnects_an_uncommitted_receipt() {
    let app = app();
    let receipt = app
        .request_command(NativeCommand::SelectItem { item: SECOND })
        .unwrap();
    assert_eq!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    drop(app);
    assert_eq!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    );
}

fn service_until_idle(host: &dyn eframe::NativeHostHandler, context: &egui::Context) {
    for _ in 0..16 {
        match host.on_service(context) {
            eframe::NativeHostWake::Wait => return,
            eframe::NativeHostWake::ServiceAfterCurrent => {}
            wake => panic!("unexpected service wake: {wake:?}"),
        }
    }
    panic!("native service did not drain the accepted application request");
}

#[test]
fn public_service_completes_independent_receipts_without_root_paint_or_result_reads() {
    use egui_dockspace_native::NativeActionStatus;

    let app = app();
    let host = app.native_host_handler();
    let context = egui::Context::default();
    let pass = context.cumulative_pass_nr_for(egui::ViewportId::ROOT);
    let first = app
        .request_command(NativeCommand::SetArrangementLocked { locked: true })
        .unwrap();
    service_until_idle(host.as_ref(), &context);
    assert!(app.arrangement_locked());
    let second = app
        .request_command(NativeCommand::SetArrangementLocked { locked: false })
        .unwrap();
    service_until_idle(host.as_ref(), &context);
    assert!(!app.arrangement_locked());
    assert!(matches!(
        second.try_recv(),
        Ok(Ok(NativeActionStatus::ArrangementLockChanged {
            locked: false,
            changed: true,
            ..
        }))
    ));
    assert!(matches!(
        first.try_recv(),
        Ok(Ok(NativeActionStatus::ArrangementLockChanged {
            locked: true,
            changed: true,
            ..
        }))
    ));
    assert_eq!(
        first.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    );
    assert_eq!(
        second.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    );
    assert_eq!(context.cumulative_pass_nr_for(egui::ViewportId::ROOT), pass);
}

#[test]
fn dropping_a_receipt_does_not_cancel_the_accepted_request() {
    let app = app();
    let host = app.native_host_handler();
    drop(
        app.request_command(NativeCommand::SetArrangementLocked { locked: true })
            .unwrap(),
    );
    let error = app
        .request_command(NativeCommand::SelectItem { item: FIRST })
        .unwrap_err();
    assert_eq!(error.reason(), NativeActionRequestError::Busy);
    service_until_idle(host.as_ref(), &egui::Context::default());
    assert!(app.arrangement_locked());
    drop(
        app.request_command(NativeCommand::SelectItem { item: FIRST })
            .unwrap(),
    );
}
