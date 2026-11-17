//! Shared visibility semantics through application controls and the real egui consumer.

use super::*;
use dockspace_host_conformance::scenario::LOCAL_PANE_VISIBILITY;
use egui_kittest::kittest::NodeT as _;
use std::collections::{BTreeMap, BTreeSet};

const ITEMS: [ItemId; 4] = [FIRST, SECOND, THIRD, FOURTH];

struct VisibilityPanes {
    documents: BTreeMap<ItemId, String>,
    rendered: BTreeSet<ItemId>,
}

impl Default for VisibilityPanes {
    fn default() -> Self {
        Self {
            documents: ITEMS
                .into_iter()
                .map(|item| (item, format!("Saved document {}", item.get())))
                .collect(),
            rendered: BTreeSet::new(),
        }
    }
}

impl PaneView for VisibilityPanes {
    fn title(&self, item: ItemId) -> Option<egui::WidgetText> {
        Panes.title(item)
    }

    fn focus_target(&self, item: ItemId) -> Option<egui::Id> {
        Some(egui::Id::unique(("product-visibility-document", item)))
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        self.rendered.insert(item);
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, document_fill(item));
        let target = self
            .focus_target(item)
            .expect("each document has a stable focus target");
        let label = ui.label(format!("Content {}", label(item)));
        let text = self
            .documents
            .get_mut(&item)
            .expect("the application owns the document");
        let response = ui
            .add(egui::TextEdit::singleline(text).id(target))
            .labelled_by(label.id);
        PaneResponse::from(&response)
    }
}

struct VisibilityState {
    dockspace: Dockspace,
    panes: VisibilityPanes,
    commands: Vec<(ItemId, bool)>,
    close_requests: Vec<DockspaceCloseRequest>,
}

type VisibilityHarness = Harness<'static, VisibilityState>;

fn label(item: ItemId) -> &'static str {
    match item {
        FIRST => "First",
        SECOND => "Second",
        THIRD => "Third",
        FOURTH => "Fourth",
        _ => panic!("unexpected visibility fixture item: {item:?}"),
    }
}

fn document_fill(item: ItemId) -> egui::Color32 {
    match item {
        FIRST => egui::Color32::from_rgb(37, 61, 83),
        SECOND => egui::Color32::from_rgb(71, 43, 97),
        THIRD => egui::Color32::from_rgb(53, 89, 47),
        FOURTH => egui::Color32::from_rgb(103, 67, 41),
        _ => panic!("unexpected visibility fixture item: {item:?}"),
    }
}

fn item_for_alias(alias: ItemAlias) -> ItemId {
    match alias {
        ItemAlias::Primary => FIRST,
        ItemAlias::Secondary => SECOND,
        ItemAlias::Contained => FOURTH,
    }
}

fn visibility_harness(
    id: &'static str,
    layout: DockspaceLayout,
    style: DockStyle,
) -> VisibilityHarness {
    let mut harness = Harness::builder()
        .with_size(vec2(800.0, 600.0))
        .with_max_steps(8)
        .build_ui_state(
            |ui, state: &mut VisibilityState| {
                ui.horizontal(|ui| {
                    for item in [FIRST, SECOND, FOURTH] {
                        let visible = state.dockspace.view().item(item).unwrap().is_visible();
                        for (caption, desired, enabled) in
                            [("Hide", false, visible), ("Show", true, !visible)]
                        {
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(format!("{caption} {}", label(item))),
                                )
                                .clicked()
                            {
                                let result = state
                                    .dockspace
                                    .set_item_visibility_current(item, desired)
                                    .expect("the application visibility command commits");
                                assert!(matches!(
                                    result.status(),
                                    DockspaceActionStatus::Applied(
                                        DockspaceActionOutcome::ItemVisibilityChanged {
                                            item: actual,
                                            visible: actual_visible,
                                            changed: true,
                                        }
                                    ) if *actual == item && *actual_visible == desired
                                ));
                                state.commands.push((item, desired));
                            }
                        }
                    }
                });
                state.panes.rendered.clear();
                let response = state
                    .dockspace
                    .show_single_surface(SURFACE, ui, &mut state.panes)
                    .expect("the visibility product frame advances");
                state
                    .close_requests
                    .extend(response.close_request_events().iter().cloned());
            },
            VisibilityState {
                dockspace: Dockspace::builder(id, layout)
                    .style(style)
                    .build()
                    .expect("the visibility product facade initializes"),
                panes: VisibilityPanes::default(),
                commands: Vec::new(),
                close_requests: Vec::new(),
            },
        );
    harness.run_steps(2);
    harness
}

fn assert_shared_state(
    harness: &VisibilityHarness,
    hidden: &BTreeSet<ItemId>,
    weights: &[f64],
    floating: LogicalRect,
    documents: &BTreeMap<ItemId, String>,
) {
    let state = harness.state();
    assert_eq!(
        state.panes.documents, *documents,
        "visibility must retain application content"
    );
    assert!(
        state.close_requests.is_empty(),
        "visibility must never request content destruction"
    );
    assert_eq!(surface_ids(&state.dockspace), [SURFACE]);
    assert_eq!(
        root_split(&state.dockspace),
        Some((
            DockspaceAxis::Horizontal,
            vec![vec![FIRST, SECOND], vec![THIRD]]
        )),
        "hiding an entire leaf must retain its durable split and tab order",
    );
    assert_eq!(root_weights(&state.dockspace).unwrap(), weights);
    assert_eq!(contained_rect(&state.dockspace), floating);
    for item in ITEMS {
        let view = state
            .dockspace
            .view()
            .item(item)
            .expect("hidden documents remain owned");
        let visible = !hidden.contains(&item);
        assert_eq!(view.is_visible(), visible);
        assert_eq!(view.surface(), SURFACE);
        assert_eq!(
            view.root(),
            if item == FOURTH { FLOATING_ROOT } else { ROOT }
        );
        assert_eq!(view.contained(), (item == FOURTH).then_some(FLOATING));
        assert_eq!(view.tab_index(), usize::from(item == SECOND));
        assert_eq!(
            harness
                .query_by_role_and_label(Role::Tab, label(item))
                .is_some(),
            visible,
            "the accessible tab roster must reflect visibility for {item:?}",
        );
        assert_eq!(
            state.panes.rendered.contains(&item),
            visible && view.is_selected(),
            "only selected visible content may receive a UI callback",
        );
        if !visible {
            assert_eq!(rect_fill_count(harness.output(), document_fill(item)), 0);
            assert!(
                harness
                    .query_by_role_and_label(Role::TextInput, &format!("Content {}", label(item)))
                    .is_none(),
                "hidden content must leave the live accessibility tree",
            );
        } else if view.is_selected() {
            assert!(rect_fill_count(harness.output(), document_fill(item)) > 0);
        }
    }
    assert_eq!(
        harness
            .query_by_role_and_label(Role::TitleBar, "Fourth")
            .is_some(),
        !hidden.contains(&FOURTH),
        "the contained window disappears while retaining its logical bounds",
    );
    assert_eq!(
        harness
            .query_by_role_and_label(Role::Splitter, "Resize panes")
            .is_some(),
        !(hidden.contains(&FIRST) && hidden.contains(&SECOND)),
        "an entirely hidden split child must leave no phantom splitter",
    );
}

#[test]
fn shared_local_visibility_scenario_preserves_content_and_restores_layout_through_egui_controls() {
    let mut harness = visibility_harness(
        "product-shared-visibility",
        shared_semantic_layout(),
        DockStyle::default(),
    );
    assert_eq!(LOCAL_PANE_VISIBILITY.id(), ScenarioId::LocalPaneVisibility);
    assert_eq!(LOCAL_PANE_VISIBILITY.journey(), JourneyId::L08);
    assert_eq!(
        LOCAL_PANE_VISIBILITY.admission(&[
            ScenarioCapability::LocalTabSelection,
            ScenarioCapability::LocalItemVisibility,
            ScenarioCapability::LocalSplitterAdjustment,
            ScenarioCapability::FullContainedProjection,
        ]),
        ScenarioAdmission::Ready,
    );
    assert_eq!(
        LOCAL_PANE_VISIBILITY.admission(&[
            ScenarioCapability::LocalTabSelection,
            ScenarioCapability::LocalSplitterAdjustment,
            ScenarioCapability::FullContainedProjection,
        ]),
        ScenarioAdmission::Unsupported {
            missing: ScenarioCapability::LocalItemVisibility,
        },
    );
    harness
        .get_by_role_and_label(Role::TextInput, "Content Fourth")
        .set_text_value_accesskit("Edited contained document");
    harness.run_steps(2);
    assert_eq!(
        harness.state().panes.documents[&FOURTH],
        "Edited contained document"
    );
    let floating = contained_rect(&harness.state().dockspace);
    let floating_title = harness
        .get_by_role_and_label(Role::TitleBar, "Fourth")
        .rect();
    let floating_content = rect_with_fill(harness.output(), document_fill(FOURTH))
        .expect("the contained content paints its live bounds");
    let mut weights = root_weights(&harness.state().dockspace).unwrap();
    let mut documents = harness.state().panes.documents.clone();
    let mut hidden = BTreeSet::new();
    let mut observations = Vec::new();
    let mut secondary_content = None;
    assert_shared_state(&harness, &hidden, &weights, floating, &documents);

    for step in LOCAL_PANE_VISIBILITY.steps() {
        let observation = match (step.selector(), step.action()) {
            (SemanticSelector::Tab(ItemAlias::Secondary), ScenarioAction::Activate) => {
                harness
                    .get_by_role_and_label(Role::Tab, "Second")
                    .click_accesskit();
                harness.run_steps(2);
                assert!(
                    harness
                        .state()
                        .dockspace
                        .view()
                        .item(SECOND)
                        .unwrap()
                        .is_selected()
                );
                harness
                    .get_by_role_and_label(Role::TextInput, "Content Second")
                    .set_text_value_accesskit("Edited secondary document");
                harness.run_steps(2);
                assert_eq!(
                    harness.state().panes.documents[&SECOND],
                    "Edited secondary document"
                );
                documents = harness.state().panes.documents.clone();
                secondary_content = rect_with_fill(harness.output(), document_fill(SECOND));
                assert!(secondary_content.is_some());
                SemanticObservation::ItemSelected(ItemAlias::Secondary)
            }
            (SemanticSelector::Item(alias), ScenarioAction::SetVisible(visible)) => {
                let item = item_for_alias(alias);
                let command = format!("{} {}", if visible { "Show" } else { "Hide" }, label(item));
                let commands_before = harness.state().commands.len();
                harness
                    .get_by_role_and_label(Role::Button, &command)
                    .click();
                harness.run_steps(2);
                assert_eq!(harness.state().commands.len(), commands_before + 1);
                assert_eq!(harness.state().commands.last(), Some(&(item, visible)));
                if visible {
                    hidden.remove(&item);
                } else {
                    hidden.insert(item);
                }
                let observed = harness
                    .state()
                    .dockspace
                    .view()
                    .item(item)
                    .unwrap()
                    .is_visible();
                assert_eq!(observed, visible);
                if item == SECOND && visible {
                    assert_eq!(
                        rect_with_fill(harness.output(), document_fill(SECOND)),
                        secondary_content,
                        "restoring the whole group must recover its previous live content bounds",
                    );
                }
                SemanticObservation::ItemVisibilityChanged {
                    item: alias,
                    visible: observed,
                }
            }
            (SemanticSelector::Splitter(ScenarioAxis::Horizontal), ScenarioAction::Increment) => {
                harness
                    .get_by_role_and_label(Role::Splitter, "Resize panes")
                    .increment_accesskit();
                harness.run_steps(2);
                let after = root_weights(&harness.state().dockspace).unwrap();
                assert!(after[0] > weights[0]);
                assert!(after[1] < weights[1]);
                weights = after;
                SemanticObservation::SplitterAdjusted(ScenarioAxis::Horizontal)
            }
            (selector, action) => {
                panic!("unsupported visibility scenario step: {selector:?} {action:?}")
            }
        };
        assert_eq!(observation, step.expected());
        observations.push(observation);
        assert_shared_state(&harness, &hidden, &weights, floating, &documents);
    }
    assert_eq!(observations, LOCAL_PANE_VISIBILITY.expected_observations());
    assert!(hidden.is_empty());
    assert_eq!(harness.state().commands.len(), 6);
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TitleBar, "Fourth")
            .rect(),
        floating_title,
        "restored contained title geometry must match the original UI",
    );
    assert_eq!(
        rect_with_fill(harness.output(), document_fill(FOURTH)),
        Some(floating_content),
        "restored contained content must match the original painted bounds",
    );
    assert_eq!(
        harness
            .state()
            .dockspace
            .view()
            .item(FIRST)
            .unwrap()
            .tabs()
            .items()
            .collect::<Vec<_>>(),
        [FIRST, SECOND],
    );
    assert!(
        harness
            .state()
            .dockspace
            .view()
            .item(SECOND)
            .unwrap()
            .is_selected()
    );
}

#[test]
fn hiding_an_open_menu_member_removes_its_row_and_rejects_old_accesskit_activation() {
    let mut harness = visibility_harness(
        "product-visibility-open-menu",
        overflow_layout(),
        DockStyle {
            tab_min_width: 320.0,
            tab_max_width: 320.0,
            ..Default::default()
        },
    );
    harness
        .get_by_role_and_label(Role::Button, "List tabs")
        .click_accesskit();
    harness.run_steps(2);
    let (old_row, tree) = harness
        .get_by_role_and_label(Role::MenuItem, "Second")
        .accesskit_node()
        .locate();
    assert_eq!(tree, TreeId::ROOT);
    harness
        .get_by_role_and_label(Role::Button, "Hide Second")
        .click_accesskit();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Menu, "Open tabs")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::MenuItem, "Second")
            .is_none()
    );
    assert!(
        harness
            .state()
            .dockspace
            .view()
            .item(SECOND)
            .unwrap()
            .is_hidden()
    );
    let before = harness.state().dockspace.version();
    harness.event(accesskit_action(old_row, Action::Focus));
    harness.event(accesskit_action(old_row, Action::Click));
    harness.run_steps(2);
    assert_eq!(harness.state().dockspace.version(), before);
    assert!(
        harness
            .state()
            .dockspace
            .view()
            .item(FIRST)
            .unwrap()
            .is_selected()
    );
    assert!(harness.state().close_requests.is_empty());
    assert!(
        harness
            .query_by_role_and_label(Role::MenuItem, "Second")
            .is_none()
    );

    harness
        .get_by_role_and_label(Role::Button, "Show Second")
        .click_accesskit();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::MenuItem, "Second")
        .click_accesskit();
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .dockspace
            .view()
            .item(SECOND)
            .unwrap()
            .is_selected()
    );
    assert_eq!(
        tab_items(&harness.state().dockspace),
        [FIRST, SECOND, THIRD, FOURTH]
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Menu, "Open tabs")
            .is_none()
    );
}

#[test]
fn hidden_tab_rejects_its_previous_close_control_until_the_application_shows_it() {
    let mut harness = visibility_harness(
        "product-visibility-old-close",
        shared_semantic_layout(),
        DockStyle::default(),
    );
    harness
        .get_by_role_and_label(Role::Tab, "Second")
        .click_accesskit();
    harness.run_steps(2);
    let (old_close, tree) = harness
        .get_by_role_and_label(Role::Button, "Close Second")
        .accesskit_node()
        .locate();
    assert_eq!(tree, TreeId::ROOT);
    harness
        .get_by_role_and_label(Role::Button, "Hide Second")
        .click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Close Second")
            .is_none()
    );
    let before = harness.state().dockspace.version();
    harness.event(accesskit_action(old_close, Action::Focus));
    harness.event(accesskit_action(old_close, Action::Click));
    harness.run_steps(2);
    assert_eq!(harness.state().dockspace.version(), before);
    assert!(harness.state().close_requests.is_empty());
    let hidden = harness.state().dockspace.view().item(SECOND).unwrap();
    assert!(hidden.is_hidden());
    assert_eq!(hidden.tab_index(), 1);
    assert_eq!(hidden.tabs().items().collect::<Vec<_>>(), [FIRST, SECOND]);

    harness
        .get_by_role_and_label(Role::Button, "Show Second")
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, "Close Second")
        .click_accesskit();
    harness.run_steps(2);
    let requests = &harness.state().close_requests;
    assert_eq!(
        requests.len(),
        1,
        "restored controls must recover ordinary close semantics"
    );
    assert_eq!(requests[0].plan().items()[0].item(), SECOND);
    assert!(harness.state().dockspace.view().item(SECOND).is_some());
}
