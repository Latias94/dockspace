use dockspace::geometry::PhysicalRect;
use dockspace::model::{
    DockAnchor, DockPlacement, DockspaceGridShape, DockspaceLayout, DockspaceNode,
    DockspaceRootLayout, DockspaceSurfaceLayout, ItemId, NativeTearOffSource,
    NativeWindowPlacement, RootId, SurfaceId,
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
    app.request_command(NativeCommand::SelectItem { item: SECOND })
        .unwrap();
    assert_eq!(
        app.request_command(NativeCommand::SelectItem { item: FIRST })
            .map_err(|error| error.reason()),
        Err(NativeActionRequestError::Busy)
    );
}

#[test]
fn a_stored_command_shares_bounded_admission_without_mutating_application_data() {
    use std::error::Error;

    let app = app();
    let command = NativeCommand::FloatItem {
        item: FIRST,
        surface: SURFACE,
    };

    app.request_command(command.clone()).unwrap();
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
        app.request_command(NativeCommand::DockItem {
            item: FIRST,
            placement: DockPlacement::Center(DockAnchor::Item(SECOND)),
        })
        .map_err(|error| error.reason()),
        Err(NativeActionRequestError::Busy)
    );
    assert!(app.take_action_status().is_none());
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
            0 | 3 => NativeTearOffSource::Item(SECOND),
            1 | 4 => NativeTearOffSource::Tabs(group.clone()),
            _ => NativeTearOffSource::Root(RootId::new(1)),
        };
        let command = match kind {
            0..=2 => NativeCommand::MoveToNewWindow { source, placement },
            3 | 4 => NativeCommand::MoveToNewWindowDefault { source },
            _ => NativeCommand::MoveRootToNewWindow {
                root: RootId::new(1),
                placement,
            },
        };
        app.request_command(command.clone()).unwrap();
        assert_eq!(
            app.request_command(command).map_err(|error| error.reason()),
            Err(NativeActionRequestError::Busy)
        );
        assert_eq!(
            app.request_command(NativeCommand::SelectItem { item: FIRST })
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
        assert!(app.take_action_status().is_none());
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
    destination
        .request_command(NativeCommand::MoveToNewWindowDefault {
            source: NativeTearOffSource::Tabs(group),
        })
        .unwrap();
    assert_eq!(
        destination
            .request_command(NativeCommand::SelectItem { item: SECOND })
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
    assert!(destination.take_action_status().is_none());
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
    assert!(app.take_action_status().is_none());
    app.request_command(NativeCommand::FloatTab {
        tab: tab.clone(),
        surface: SURFACE,
    })
    .unwrap();
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
