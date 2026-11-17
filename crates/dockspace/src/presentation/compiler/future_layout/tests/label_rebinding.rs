//! Retained intrinsic widths belong to their frozen labels as well as tab keys.

use super::*;
use crate::graph::TabLabel;
use crate::graph::command::{DockTarget, WorkspaceCommand};

#[test]
fn canonical_layout_kind_change_does_not_reuse_the_previous_label_intrinsic() {
    check_boundary_label_rebinding(None);
}

#[test]
fn canonical_layout_kind_change_retains_an_unchanged_named_label_intrinsic() {
    check_boundary_label_rebinding(Some("Tools"));
}

pub(super) fn check_boundary_label_rebinding(title: Option<&str>) {
    let retained = ItemId::new(2);
    let other_root = RootId::new(2);
    let other_surface = SurfaceId::new(2);
    let mut builder = Workspace::builder();
    let source = builder.insert_node(Node::tabs([MOVED]));
    let remaining = builder.insert_node(Node::pane(retained));
    let mut branch = Node::equal_split(Axis::Horizontal, [source, remaining]).unwrap();
    branch.set_layout_title(title.map(std::sync::Arc::from));
    let branch = builder.insert_node(branch);
    let layout_tab = TabKey::for_layout(branch);
    let target = builder.insert_node(Node::tabs_from_keys(
        vec![layout_tab, TabKey::for_item(ItemId::new(10))],
        Some(layout_tab),
    ));
    builder.set_root(ROOT, RootRecord::new(target).with_central(target));
    builder.set_surface(SURFACE, SurfacePresentation::with_main(ROOT));
    let other_target = builder.insert_node(Node::tabs([ItemId::new(20)]));
    builder.set_root(
        other_root,
        RootRecord::new(other_target).with_central(other_target),
    );
    builder.set_surface(other_surface, SurfacePresentation::with_main(other_root));
    let workspace = builder.build().unwrap();
    let policy = DockPolicy::default().snapshot(PolicyRevision::new(1));
    let config = DockPresentationConfig::builder()
        .splitter_thickness(4.0)
        .build()
        .unwrap();
    let presented = compile_label_measured(&workspace, &policy, &config, SURFACE);
    let other_plan = compile_label_measured(&workspace, &policy, &config, other_surface);
    assert_eq!(
        workspace.tab_label(target, layout_tab),
        Some(TabLabel::Layout {
            title: title.unwrap_or("Split"),
        })
    );
    assert!(workspace.item_is_active(MOVED));
    let payload = MovePayload::Item(workspace.capture_item_source(ROOT, source, MOVED).unwrap());
    let command = WorkspaceCommand::Move {
        payload: payload.clone(),
        target: DockTarget::Center(workspace.capture_tab_target(ROOT, target).unwrap()),
    };
    let mut candidate = workspace.clone();
    let report = WorkspaceTransaction::from_commands([command.clone()])
        .apply(&mut candidate, &policy)
        .expect("a direct item can merge into its containing ancestor Tabs");
    assert!(report.changed());
    candidate.validate().unwrap();
    assert_eq!(candidate.item_multiset(), workspace.item_multiset());
    assert!(candidate.node(source).is_none());
    assert!(matches!(candidate.node(branch), Some(Node::Tabs { .. })));
    assert_eq!(candidate.parent_link(ROOT, branch).unwrap().parent, target);
    assert_eq!(
        candidate.visible_tab_keys(target).as_ref(),
        [
            layout_tab,
            TabKey::for_item(ItemId::new(10)),
            TabKey::for_item(MOVED),
        ]
    );
    assert_eq!(
        candidate.visible_tab_keys(branch).as_ref(),
        [retained.into()]
    );
    assert_eq!(
        candidate.tab_label(target, layout_tab),
        Some(TabLabel::Layout {
            title: title.unwrap_or("Tabs"),
        })
    );
    assert_eq!(candidate.visible_tab_selection(target), Some(MOVED.into()));
    assert!(candidate.item_is_active(MOVED));
    assert!(!candidate.item_is_active(retained));
    let committed = compile_label_measured(&candidate, &policy, &config, SURFACE);
    let actual = target_bar(&committed, target)
        .members()
        .iter()
        .find(|member| member.tab().tab == layout_tab)
        .unwrap();
    let previous = target_bar(&presented, target)
        .members()
        .iter()
        .find(|member| member.tab().tab == layout_tab)
        .unwrap();
    if title.is_some() {
        assert_eq!(previous.full_bounds().width(), actual.full_bounds().width());
    } else {
        assert!(previous.full_bounds().width() > actual.full_bounds().width());
    }

    // Move the surviving boundary to another root to exercise fallback from
    // the original source facts, independently of the exact destination key.
    let mut relocated = candidate.clone();
    let relocate = WorkspaceCommand::Move {
        payload: MovePayload::Subtree(candidate.capture_node_source(ROOT, branch).unwrap()),
        target: DockTarget::Center(
            candidate
                .capture_tab_target(other_root, other_target)
                .unwrap(),
        ),
    };
    WorkspaceTransaction::from_commands([relocate])
        .apply(&mut relocated, &policy)
        .unwrap();
    relocated.validate().unwrap();
    assert_eq!(
        relocated.parent_link(other_root, branch).unwrap().parent,
        other_target
    );
    let lookup = super::super::candidate_tab_intrinsic;
    for (path, measured) in [
        (
            "target exact",
            lookup(
                &candidate, &presented, None, SURFACE, ROOT, target, layout_tab,
            ),
        ),
        (
            "source exact",
            lookup(
                &candidate,
                &other_plan,
                presented.layout_facts(),
                SURFACE,
                ROOT,
                target,
                layout_tab,
            ),
        ),
        (
            "target fallback",
            lookup(
                &relocated,
                &presented,
                None,
                other_surface,
                other_root,
                other_target,
                layout_tab,
            ),
        ),
        (
            "source fallback",
            lookup(
                &relocated,
                &other_plan,
                presented.layout_facts(),
                other_surface,
                other_root,
                other_target,
                layout_tab,
            ),
        ),
    ] {
        if title.is_some() {
            assert_eq!(measured.unwrap().content_width(), 112.0, "{path}");
        } else {
            assert!(
                matches!(
                    measured,
                    Err(super::super::FutureLayoutProjectionError::TabIntrinsicUnavailable { .. })
                ),
                "a changed frozen descriptor cannot supply the {path} intrinsic: {measured:?}",
            );
        }
    }

    let resolved = resolve_target(
        &workspace,
        &policy,
        &presented,
        &presented,
        payload,
        DropTargetId::Center {
            surface: SURFACE,
            root: ROOT,
            tabs: target,
        },
    );
    assert_eq!(resolved.command(), &command);
    let body = committed
        .pane_records()
        .iter()
        .find(|pane| pane.item() == MOVED)
        .unwrap();
    assert_eq!(resolved.visual().rect(), body.content_bounds());
    if title.is_some() {
        assert!(resolved.preview_tab_strip().is_some());
    } else {
        assert!(resolved.preview_tab_strip().is_none());
    }
    // An unmeasured changed label may omit the future strip. If emitted, every
    // width and following member must agree with fresh candidate measurement.
    if let Some(future) = resolved.preview_tab_strip() {
        let predicted = future
            .tabs
            .iter()
            .find(|tab| tab.key == layout_tab)
            .unwrap();
        assert_eq!(predicted.layout_title.as_deref(), title.or(Some("Tabs")));
        assert_eq!(predicted.full_bounds, actual.full_bounds());
        assert_future_tabs_match_committed(&resolved, &committed, target, &[MOVED]);
    }
}

pub(super) fn compile_label_measured(
    workspace: &Workspace,
    policy: &DockPolicySnapshot,
    config: &DockPresentationConfig,
    surface: SurfaceId,
) -> PresentationPlan {
    let manifest = derive_scene_requirement_draft(
        EngineAuthorityDomainId::new_for_test(1),
        workspace,
        WorkspaceVersion::initial(),
        PresentationConfigRevision::new(1),
        policy,
        RequirementRevision::new(1),
        &workspace
            .surfaces()
            .map(|(surface, _)| (surface, SurfaceRequirementRevision::new(1)))
            .collect(),
    )
    .unwrap()
    .finalize(PopupPlaneRequirement::default())
    .unwrap();
    let requirements = manifest.surface(surface).unwrap();
    let mut measurements = SurfaceMeasurements::new(requirements.ticket());
    measurements
        .set_bounds(
            requirements.bounds(),
            Measurement::Measured(LogicalRect::new(0.0, 0.0, 1000.0, 600.0).unwrap()),
        )
        .unwrap();
    for key in requirements.pane_sizings() {
        measurements
            .insert_pane_sizing(key, Measurement::Measured(PaneSizing::default()))
            .unwrap();
    }
    for key in requirements.tab_intrinsics() {
        let width = match requirements.tab_label(key.tab()).unwrap() {
            TabLabel::Item(_) => 40.0,
            TabLabel::Layout { title: "Split" } => 240.0,
            TabLabel::Layout { title: "Tabs" } => 64.0,
            TabLabel::Layout { title: "Grid" } => 176.0,
            TabLabel::Layout { title: "Tools" } => 112.0,
            label => panic!("unexpected fixture label: {label:?}"),
        };
        measurements
            .insert_tab_intrinsic(
                key,
                Measurement::Measured(TabIntrinsic::new(width).unwrap()),
            )
            .unwrap();
    }
    for key in requirements.tab_strips() {
        measurements
            .insert_tab_strip(
                key,
                Measurement::Measured(TabStripMetrics::new(0.0, 0.0).unwrap()),
            )
            .unwrap();
    }
    compile_surface_plan(
        workspace,
        WorkspaceVersion::initial(),
        policy,
        config,
        &manifest,
        &measurements,
        &TabStripStateStore::default(),
        &[],
        None,
    )
    .unwrap()
    .finalize()
    .unwrap()
}
