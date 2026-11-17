//! A finite lifecycle oracle: two rosters, logical item indices, selection and visibility.

use proptest::prelude::*;

use super::ITEMS;
use crate::close::plan::{CloseDecision, DeferredCloseDecision};
use crate::ids::{NodeKey, RootId, SurfaceId, TabKey};
use crate::model::{
    DockAnchor, DockspaceActionOutcome as Applied, DockspaceLayout, DockspaceNode,
    DockspaceRootLayout, DockspaceSurfaceLayout, DockspaceTabView,
};
use crate::policy::{CloseCapability, DockPolicy};
use crate::runtime::{
    DockspaceCloseOutcome as Closed, DockspaceCloseResolution as Resolution,
    DockspaceHostFrame as Frame, DockspaceSession as Session,
    DockspaceSubmittedAction as Submitted, HostFrameReport, HostInputOutcome as Input,
    SurfaceUnavailableReason,
};

const SURFACE: SurfaceId = SurfaceId::new(11);
const ROOT: RootId = RootId::new(11);
const BRANCH: usize = 7;
const DOCUMENT: usize = 2;

#[derive(Clone, Copy, Debug)]
enum Scope {
    Item,
    Tab,
    Tabs,
}

#[derive(Clone, Copy, Debug)]
enum Finish {
    Allow,
    Veto,
    DeferredAllow,
    DeferredVeto,
    Drop,
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Select(u8),
    Visibility(u8, bool),
    CloseItem(u8),
    Abort(Finish, u8),
}

fn scopes() -> impl Strategy<Value = Scope> {
    prop_oneof![Just(Scope::Item), Just(Scope::Tab), Just(Scope::Tabs)]
}

fn actions() -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => any::<u8>().prop_map(Action::Select),
        4 => (any::<u8>(), any::<bool>()).prop_map(|(i, visible)| Action::Visibility(i, visible)),
        2 => any::<u8>().prop_map(Action::CloseItem),
        3 => (prop_oneof![Just(Finish::Veto), Just(Finish::DeferredVeto), Just(Finish::Drop)], any::<u8>())
            .prop_map(|(finish, i)| Action::Abort(finish, i)),
    ]
}

struct Keys {
    leaf: NodeKey,
    tab: TabKey,
}

struct Model {
    outer: Vec<usize>,
    leaf: Vec<usize>,
    selected: usize,
    leaf_selected: usize,
    hidden: [bool; 5],
    focus: usize,
}

// Keep the current visible selection; otherwise use the visual gap, never MRU.
fn choice(roster: &[usize], selected: usize, visible: impl Fn(usize) -> bool) -> Option<usize> {
    if roster.contains(&selected) && visible(selected) {
        return Some(selected);
    }
    let gap = roster.iter().position(|item| *item == selected)?;
    roster[gap + 1..]
        .iter()
        .chain(roster[..gap].iter().rev())
        .copied()
        .find(|i| visible(*i))
}

impl Model {
    fn new() -> Self {
        Self {
            outer: vec![DOCUMENT, BRANCH, 3, 4],
            leaf: vec![0, 1],
            selected: DOCUMENT,
            leaf_selected: 0,
            hidden: [false; 5],
            focus: DOCUMENT,
        }
    }

    fn owned(&self) -> Vec<usize> {
        std::iter::once(DOCUMENT)
            .chain(self.leaf.iter().copied())
            .chain([3, 4])
            .collect()
    }

    fn active(&self) -> usize {
        if self.selected == BRANCH {
            self.leaf_selected
        } else {
            self.selected
        }
    }

    fn select(&mut self, item: usize) {
        if self.leaf.contains(&item) {
            self.leaf_selected = item;
            self.selected = BRANCH;
        } else {
            self.selected = item;
        }
        self.focus = item;
    }

    fn repair(&mut self, removed: &[usize]) {
        let visible = |i: usize| !removed.contains(&i) && !self.hidden[i];
        let leaf_next = choice(&self.leaf, self.leaf_selected, visible);
        self.selected = choice(&self.outer, self.selected, |i| {
            if i == BRANCH {
                self.leaf.iter().copied().any(visible)
            } else {
                visible(i)
            }
        })
        .unwrap();
        self.leaf.retain(|i| !removed.contains(i));
        if let Some(next) = leaf_next {
            self.leaf_selected = next;
        } else if removed.contains(&self.leaf_selected) {
            self.leaf_selected = self.leaf.first().copied().unwrap_or(0);
        }
        if self.leaf.is_empty() {
            self.outer.retain(|i| *i != BRANCH);
        }
    }

    fn visibility(&mut self, item: usize, visible: bool) {
        self.hidden[item] = !visible;
        self.repair(&[]);
        if !visible && self.focus == item {
            self.focus = self.active();
        }
    }

    fn close(&mut self, removed: &[usize]) {
        self.repair(removed);
        for item in removed {
            self.hidden[*item] = false;
        }
        if removed.contains(&self.focus) {
            self.focus = self.active();
        }
    }

    fn check(&self, session: &Session, keys: &Keys) {
        let view = session.view();
        let root = view.root(ROOT).unwrap();
        let owned = self.owned();
        assert_eq!(
            root.items().collect::<Vec<_>>(),
            owned.iter().map(|i| ITEMS[*i]).collect::<Vec<_>>()
        );
        assert_eq!(
            root.active_items().collect::<Vec<_>>(),
            [ITEMS[self.active()]]
        );
        for (i, id) in ITEMS[..5].iter().copied().enumerate() {
            assert_eq!(view.item(id).is_some(), owned.contains(&i));
            assert_eq!(view.item_is_hidden(id), self.hidden[i]);
            if let Some(item) = view.item(id) {
                assert_eq!(item.is_active(), self.active() == i);
            }
        }
        let leaf = view.node(keys.leaf);
        assert_eq!(leaf.is_some(), !self.leaf.is_empty());
        if let Some(leaf) = leaf {
            assert_eq!(
                leaf.tabs().unwrap().items().collect::<Vec<_>>(),
                self.leaf.iter().map(|i| ITEMS[*i]).collect::<Vec<_>>()
            );
            assert_eq!(
                leaf.tabs().unwrap().selected(),
                Some(TabKey::for_item(ITEMS[self.leaf_selected]))
            );
        }
        assert_eq!(
            session.pane_focus_request(SURFACE).unwrap().item(),
            Some(ITEMS[self.focus])
        );
        session.workspace().validate().unwrap();
    }
}

fn fixture() -> (Session, Keys) {
    let leaf = DockspaceNode::tabs([ITEMS[0], ITEMS[1]]).named("Leaf");
    let middle = DockspaceNode::composed_tabs([leaf]).named("Middle");
    let tools = DockspaceNode::composed_tabs([middle]).named("Tools");
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE,
        DockspaceRootLayout::new(
            ROOT,
            DockspaceNode::composed_tabs([
                DockspaceNode::pane(ITEMS[DOCUMENT]),
                tools,
                DockspaceNode::pane(ITEMS[3]),
                DockspaceNode::pane(ITEMS[4]),
            ]),
        ),
    )])
    .unwrap();
    let mut policy = DockPolicy::default();
    policy.set_close_capability(CloseCapability::DeferredAllowed);
    let session = Session::from_layout(layout, policy).unwrap();
    let outer = session
        .view()
        .root(ROOT)
        .unwrap()
        .content()
        .unwrap()
        .tabs()
        .unwrap();
    let DockspaceTabView::Layout(tools) = outer.children().nth(1).unwrap() else {
        panic!("Tools must be a layout tab")
    };
    let DockspaceTabView::Layout(middle) = tools.tabs().unwrap().children().next().unwrap() else {
        panic!("Middle must be a layout tab")
    };
    let leaf = session.view().item(ITEMS[0]).unwrap().tabs().unwrap().key();
    let keys = Keys {
        leaf,
        tab: middle.tabs().unwrap().tabs()[0],
    };
    (session, keys)
}

fn commit(mut frame: Frame<'_>) -> HostFrameReport {
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap()
}

fn snapshot(
    session: &Session,
) -> (
    super::Workspace,
    crate::model::WorkspaceVersion,
    Option<crate::runtime::DockspacePaneFocusRequest>,
) {
    (
        session.workspace().clone(),
        session.version(),
        session.pane_focus_request(SURFACE),
    )
}

fn select(session: &mut Session, model: &mut Model, item: usize) {
    let mut frame = session.begin_host_frame().unwrap();
    let action = frame.select_item_current(ITEMS[item]).unwrap();
    assert!(
        matches!(commit(frame).submitted_action_outcome(action), Some(Input::ProductActionApplied(
        Applied::ItemActivated { item: actual, .. })) if *actual == ITEMS[item])
    );
    model.select(item);
}

fn visibility(session: &mut Session, model: &mut Model, item: usize, visible: bool) {
    let focus = session.pane_focus_request(SURFACE);
    let changed = model.hidden[item] == visible;
    let mut frame = session.begin_host_frame().unwrap();
    let action = frame
        .set_item_visibility_current(ITEMS[item], visible)
        .unwrap();
    assert_eq!(
        commit(frame).submitted_action_outcome(action),
        Some(&Input::ProductActionApplied(
            Applied::ItemVisibilityChanged {
                item: ITEMS[item],
                visible,
                changed
            }
        ))
    );
    model.visibility(item, visible);
    if visible {
        assert_eq!(
            session.pane_focus_request(SURFACE),
            focus,
            "show must not request focus"
        );
    }
}

fn decision(
    report: &HostFrameReport,
    action: Submitted,
    expected: Resolution,
    outcome: Option<Closed>,
) {
    let Some(Input::CloseDecisionProcessed {
        resolution,
        application,
        changed,
        ..
    }) = report.submitted_action_outcome(action)
    else {
        panic!(
            "missing exact close decision: {:?}",
            report.submitted_action_outcome(action)
        )
    };
    assert_eq!(*resolution, expected);
    assert_eq!(*changed, outcome.is_some());
    assert_eq!(*application, outcome.map(Ok));
}

fn close(
    session: &mut Session,
    model: &mut Model,
    keys: &Keys,
    scope: Scope,
    finish: Finish,
    pick: u8,
) {
    let item = model.leaf[usize::from(pick) % model.leaf.len()];
    let removed = if matches!(scope, Scope::Item) {
        vec![item]
    } else {
        model.leaf.clone()
    };
    let prepared = match scope {
        Scope::Item => session.prepare_close_item(ITEMS[item]),
        Scope::Tab => session.prepare_close_tab(session.capture_tab(keys.tab).unwrap()),
        Scope::Tabs => session.prepare_close_tabs(
            session
                .capture_tabs_group(DockAnchor::Node(keys.leaf))
                .unwrap(),
        ),
    };
    let mut before = snapshot(session);
    let mut frame = session.begin_host_frame().unwrap();
    let requested = frame.submit_prepared_close_request(prepared).unwrap();
    let report = commit(frame);
    let plan = match report.submitted_action_outcome(requested).unwrap() {
        Input::CloseRequested {
            plan,
            reused: false,
            ..
        } => plan.clone(),
        outcome => panic!("model-valid close rejected: {outcome:?}"),
    };
    let request = plan.request();
    assert_eq!(
        plan.items().iter().map(|i| i.item()).collect::<Vec<_>>(),
        removed.iter().map(|i| ITEMS[*i]).collect::<Vec<_>>()
    );
    assert_eq!(snapshot(session), before);
    if matches!(finish, Finish::Drop) {
        let mut frame = session.begin_host_frame().unwrap();
        for item in plan.items() {
            frame
                .resolve_close(request, item.token(), CloseDecision::Allow)
                .unwrap();
        }
        for i in &removed {
            assert!(frame.view().item(ITEMS[*i]).is_none());
        }
        drop(frame);
        assert_eq!(snapshot(session), before);
        assert_eq!(
            session.pending_content_close_plans().collect::<Vec<_>>(),
            [plan.clone()]
        );
    }
    // Successful multi-item scope approval already has a dedicated named matrix.
    assert_eq!(plan.items().len(), 1);
    let item = plan.items()[0];
    let mut frame = session.begin_host_frame().unwrap();
    let deferred = matches!(finish, Finish::DeferredAllow | Finish::DeferredVeto);
    let allow = matches!(finish, Finish::Allow | Finish::DeferredAllow);
    let vote = if deferred {
        CloseDecision::Deferred
    } else if allow {
        CloseDecision::Allow
    } else {
        CloseDecision::Veto
    };
    let mut action = frame.resolve_close(request, item.token(), vote).unwrap();
    let mut report = commit(frame);
    if deferred {
        let pending = session.pending_content_close_plans().collect::<Vec<_>>();
        assert_eq!(pending.len(), 1);
        let continuation = pending[0].items()[0].deferred_token().unwrap();
        decision(
            &report,
            action,
            Resolution::Deferred {
                request,
                item: item.item(),
                continuation,
            },
            None,
        );
        assert_eq!(snapshot(session), before);
        visibility(session, model, 3, pick & 1 == 0);
        let pending = session.pending_content_close_plans().collect::<Vec<_>>();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].items().last().unwrap().deferred_token(),
            Some(continuation)
        );
        model.check(session, keys);
        before = snapshot(session);
        let mut frame = session.begin_host_frame().unwrap();
        let vote = if allow {
            DeferredCloseDecision::Allow
        } else {
            DeferredCloseDecision::Veto
        };
        action = frame
            .continue_deferred_close(request, continuation, vote)
            .unwrap();
        report = commit(frame);
    }
    if allow {
        let items = removed.iter().map(|i| ITEMS[*i]).collect();
        let outcome = match scope {
            Scope::Item => Closed::ItemClosed {
                item: ITEMS[removed[0]],
                root: ROOT,
            },
            Scope::Tab => Closed::TabClosed {
                root: ROOT,
                tab: keys.tab,
                items,
            },
            Scope::Tabs => Closed::TabsClosed { root: ROOT, items },
        };
        decision(
            &report,
            action,
            Resolution::Approved { request },
            Some(outcome),
        );
        model.close(&removed);
    } else {
        decision(
            &report,
            action,
            Resolution::Vetoed {
                request,
                item: item.item(),
            },
            None,
        );
        assert_eq!(
            snapshot(session),
            before,
            "veto must preserve descendants and focus"
        );
    }
    assert_eq!(session.pending_content_close_plans().count(), 0);
    model.check(session, keys);
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::with_cases(96) })]

    #[test]
    fn nested_lifecycle_sequences_preserve_visibility_close_scope_and_focus(
        sequence in prop::collection::vec(actions(), 2..33), final_scope in scopes(),
        final_pick in any::<u8>(), final_deferred in any::<bool>(),
    ) {
        let (mut session, keys) = fixture();
        let mut model = Model::new();
        select(&mut session, &mut model, DOCUMENT);
        for action in sequence {
            let owned = model.owned();
            match action {
                Action::Select(pick) => {
                    let visible = owned.into_iter().filter(|i| !model.hidden[*i]).collect::<Vec<_>>();
                    select(&mut session, &mut model, visible[usize::from(pick) % visible.len()]);
                }
                Action::Visibility(pick, visible) => {
                    let hideable = owned.into_iter().filter(|i| *i != DOCUMENT).collect::<Vec<_>>();
                    visibility(&mut session, &mut model, hideable[usize::from(pick) % hideable.len()], visible);
                }
                Action::CloseItem(pick) => {
                    // Preserve one nested item for the mandatory cleanup boundary.
                    let finish = if model.leaf.len() == 1 { Finish::Veto } else { Finish::Allow };
                    close(&mut session, &mut model, &keys, Scope::Item, finish, pick);
                }
                Action::Abort(finish, pick) => close(&mut session, &mut model, &keys, Scope::Item, finish, pick),
            }
            model.check(&session, &keys);
        }
        // Every sequence exhausts the last nested descendant, then accepts fresh legal input.
        if model.leaf.len() > 1 { close(&mut session, &mut model, &keys, Scope::Item, Finish::Allow, final_pick); }
        let last = model.leaf[0];
        visibility(&mut session, &mut model, last, true);
        select(&mut session, &mut model, last);
        visibility(&mut session, &mut model, last, false);
        model.check(&session, &keys);
        visibility(&mut session, &mut model, last, true);
        select(&mut session, &mut model, DOCUMENT);
        select(&mut session, &mut model, last);
        close(&mut session, &mut model, &keys, final_scope,
            if final_deferred { Finish::DeferredAllow } else { Finish::Allow }, final_pick);
        visibility(&mut session, &mut model, 3, true);
        select(&mut session, &mut model, 3);
        visibility(&mut session, &mut model, 3, false);
        model.check(&session, &keys);
        visibility(&mut session, &mut model, 3, true);
        select(&mut session, &mut model, DOCUMENT);
        model.check(&session, &keys);
    }
}
