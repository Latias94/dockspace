use super::*;

use crate::geometry::{
    LogicalPoint, LogicalRect, LogicalSize, PhysicalPoint, PhysicalRect, ScaleFactor,
};
use crate::model::{
    DockspaceAxis, DockspaceContainedLayout, DockspaceLayout, DockspaceNode, DockspaceRootLayout,
    DockspaceSurfaceLayout, FloatingPresentationId, ItemId, RootId, SurfaceId, TabLabel,
};
use crate::policy::DockPolicy;
use crate::runtime::{
    ContainedResizeDirection, DockspaceDropDirection, DockspaceFocusTarget,
    DockspacePaneFocusObservation, DockspaceSession, HostWindowToken, NativeHostCapabilities,
    NativeHostCapability, NativePointerRoster, NativeSurfacePlacement, NativeWindowFacts,
    NativeWindowInputState, NativeWindowPresentationState, NativeWorkAreaRoster,
    SurfacePaintDisposition, SurfacePaintProgramRoot, SurfacePaintProgramVisitError,
    SurfacePointerButton, SurfacePointerCapture, SurfacePointerEvent, SurfacePresentationResult,
    SurfaceScrollDelta, SurfaceUnavailableReason, TabStripControlKind,
};

const SURFACE_A: SurfaceId = SurfaceId::new(1);
const SURFACE_B: SurfaceId = SurfaceId::new(2);
const ROOT_A: RootId = RootId::new(1);
const ROOT_B: RootId = RootId::new(2);
const ITEM_1: ItemId = ItemId::new(1);
const ITEM_2: ItemId = ItemId::new(2);
const ITEM_3: ItemId = ItemId::new(3);
const ITEM_4: ItemId = ItemId::new(4);

fn two_tab_layout() -> DockspaceLayout {
    DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE_A,
        DockspaceRootLayout::new(
            ROOT_A,
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::tabs([ITEM_1, ITEM_2]), 2.0),
                    (DockspaceNode::pane(ITEM_3), 1.0),
                ],
            )
            .unwrap(),
        ),
    )])
    .unwrap()
}

fn rect_center(rect: LogicalRect) -> LogicalPoint {
    LogicalPoint::new(
        rect.x() + rect.width() * 0.5,
        rect.y() + rect.height() * 0.5,
    )
    .unwrap()
}

#[test]
fn display_list_compiles_owned_scene_and_supports_fail_closed_pointer_input() {
    let mut host = LogicalHost::from_layout(two_tab_layout(), DockPolicy::default()).unwrap();
    let bounds = LogicalRect::new(0.0, 0.0, 900.0, 600.0).unwrap();
    let metrics = SurfaceMetrics::new(bounds)
        .with_pane_sizing(LogicalSize::new(80.0, 60.0).unwrap())
        .with_default_tab_width(110.0)
        .unwrap();

    // Paint into an owned `'static` display list using a dynamic tab width closure.
    let (display_list, token, paint_report) = match host
        .paint_display_list_with(
            SURFACE_A,
            metrics.with_tab_width(|label| match label {
                TabLabel::Item(item) if item == ITEM_2 => 140.0,
                TabLabel::Item(_) => 90.0,
                TabLabel::Layout { title } => (title.len() as f64) * 8.0,
            }),
        )
        .unwrap()
    {
        LogicalPaintOutcome::Ready {
            payload,
            presentation,
            report,
        } => (payload, presentation, report),
        LogicalPaintOutcome::Deferred { .. } => panic!("fresh host must not defer"),
    };

    assert_eq!(display_list.surface(), SURFACE_A);
    assert_eq!(display_list.panes().count(), 2);
    assert_eq!(display_list.tab_bars().count(), 1);
    assert_eq!(display_list.tabs().count(), 2);
    assert_eq!(display_list.splitters().count(), 1);
    assert!(!token.is_empty());

    let tab_1 = display_list
        .tabs()
        .find(|tab| tab.item == Some(ITEM_1))
        .unwrap();
    let tab_2 = display_list
        .tabs()
        .find(|tab| tab.item == Some(ITEM_2))
        .unwrap();
    assert!(tab_2.full_bounds.width() > tab_1.full_bounds.width());
    let tab_2_center = rect_center(tab_2.visible_bounds);

    // Before presentation settles, clicking cannot activate the tab.
    host.submit_pointer_batch([
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();
    assert!(host.view().item(ITEM_1).unwrap().is_active());

    // Settle presentation and verify merged repaint helper works cleanly.
    let settled = host
        .finish_presentation(token, SurfacePresentationResult::Presented)
        .unwrap();
    let _merged = LogicalHost::merge_repaint_surfaces(&[&paint_report, &settled]);

    // Blocked clearance (framework overlay/widget covers the tab) must fail closed.
    host.submit_pointer_batch([
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            PointerClearance::Blocked,
        ),
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            PointerClearance::Blocked,
        ),
    ])
    .unwrap();
    assert!(host.view().item(ITEM_1).unwrap().is_active());

    // Unobstructed click activates ITEM_2 and emits a pane focus request.
    host.submit_pointer_batch([
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();
    assert!(host.view().item(ITEM_2).unwrap().is_active());

    // Repaint and settle so the pane focus request is emitted for the newly presented tab.
    let mut simple_metrics = SurfaceMetrics::new(bounds);
    let token = match host
        .paint_display_list(SURFACE_A, &mut simple_metrics)
        .unwrap()
    {
        LogicalPaintOutcome::Ready { presentation, .. } => presentation,
        LogicalPaintOutcome::Deferred { .. } => panic!("expected ready paint"),
    };
    host.finish_presentation(token, SurfacePresentationResult::Presented)
        .unwrap();

    if let Some(request) = host.pane_focus_request(SURFACE_A).unwrap() {
        assert_eq!(request.target(), DockspaceFocusTarget::Pane(ITEM_2));
        let observation = request.prepare_observation(DockspacePaneFocusObservation::Focused);
        host.submit_pane_focus_observation(observation).unwrap();
    }
}

#[test]
fn dropped_presentation_does_not_authorize_pointer_routing() {
    let mut host = LogicalHost::from_layout(two_tab_layout(), DockPolicy::default()).unwrap();
    let mut metrics = SurfaceMetrics::new(LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap());

    let (display_list, token) = match host.paint_display_list(SURFACE_A, &mut metrics).unwrap() {
        LogicalPaintOutcome::Ready {
            payload,
            presentation,
            ..
        } => (payload, presentation),
        LogicalPaintOutcome::Deferred { .. } => panic!("expected ready paint"),
    };
    let tab_2_center = rect_center(
        display_list
            .tabs()
            .find(|tab| tab.item == Some(ITEM_2))
            .unwrap()
            .visible_bounds,
    );

    host.finish_presentation(token, SurfacePresentationResult::Dropped)
        .unwrap();

    host.submit_pointer_batch([
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonPressed(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::mouse(
            SURFACE_A,
            Some(tab_2_center),
            SurfacePointerEvent::ButtonReleased(SurfacePointerButton::Primary),
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();
    assert!(host.view().item(ITEM_1).unwrap().is_active());
}

#[test]
fn visitor_supports_minimal_overrides_and_propagates_sink_errors() {
    struct CountingVisitor {
        panes: Vec<ItemId>,
        contained_roots: usize,
        fail_on_splitter: bool,
    }

    impl SurfaceProgramVisitor for CountingVisitor {
        type Error = &'static str;

        fn root_begin(
            &mut self,
            root: SurfacePaintProgramRoot<'_>,
            _paint: SurfacePaintDisposition,
        ) -> Result<(), Self::Error> {
            if root.contained().is_some() {
                self.contained_roots += 1;
            }
            Ok(())
        }

        fn pane(
            &mut self,
            _root: SurfacePaintProgramRoot<'_>,
            pane: crate::runtime::PanePaintRecord<'_>,
            _paint: SurfacePaintDisposition,
        ) -> Result<(), Self::Error> {
            self.panes.push(pane.item());
            Ok(())
        }

        fn splitter(
            &mut self,
            _root: SurfacePaintProgramRoot<'_>,
            _splitter: crate::runtime::SplitterPaintRecord<'_>,
            _paint: SurfacePaintDisposition,
        ) -> Result<(), Self::Error> {
            if self.fail_on_splitter {
                return Err("splitter draw failed");
            }
            Ok(())
        }
    }

    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE_A,
        DockspaceRootLayout::new(
            ROOT_A,
            DockspaceNode::split(
                DockspaceAxis::Horizontal,
                [
                    (DockspaceNode::tabs([ITEM_1]), 1.0),
                    (DockspaceNode::pane(ITEM_2), 1.0),
                ],
            )
            .unwrap(),
        ),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(ROOT_B, DockspaceNode::tabs([ITEM_3])),
        LogicalRect::new(40.0, 40.0, 240.0, 160.0).unwrap(),
    ))])
    .unwrap();

    let mut host = LogicalHost::from_layout(layout, DockPolicy::default()).unwrap();
    let mut metrics = SurfaceMetrics::new(LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap());

    // Failing visitor aborts paint without committing painted output.
    let mut failing = CountingVisitor {
        panes: Vec::new(),
        contained_roots: 0,
        fail_on_splitter: true,
    };
    let err = host
        .paint_with_visitor(SURFACE_A, &mut metrics, &mut failing)
        .unwrap_err();
    assert!(matches!(
        err,
        LogicalVisitorPaintError::Visit(SurfacePaintProgramVisitError::Sink(
            "splitter draw failed"
        ))
    ));

    // Succeeding visitor receives panes and contained floating roots.
    let mut ok_visitor = CountingVisitor {
        panes: Vec::new(),
        contained_roots: 0,
        fail_on_splitter: false,
    };
    let token = match host
        .paint_with_visitor(SURFACE_A, &mut metrics, &mut ok_visitor)
        .unwrap()
    {
        LogicalPaintOutcome::Ready { presentation, .. } => presentation,
        LogicalPaintOutcome::Deferred { .. } => panic!("expected ready paint"),
    };
    assert_eq!(ok_visitor.contained_roots, 1);
    assert_eq!(ok_visitor.panes, vec![ITEM_1, ITEM_2, ITEM_3]);
    host.finish_presentation(token, SurfacePresentationResult::Presented)
        .unwrap();
}

#[test]
fn native_report_digest_deduplicates_dirty_windows_and_enforces_quiescence_gates() {
    let layout = DockspaceLayout::new([
        DockspaceSurfaceLayout::new(
            SURFACE_A,
            DockspaceRootLayout::new(ROOT_A, DockspaceNode::tabs([ITEM_1, ITEM_2])),
        ),
        DockspaceSurfaceLayout::new(
            SURFACE_B,
            DockspaceRootLayout::new(ROOT_B, DockspaceNode::tabs([ITEM_3, ITEM_4])),
        ),
    ])
    .unwrap();

    let mut session = DockspaceSession::from_layout(layout, DockPolicy::default()).unwrap();
    session
        .enable_managed_native_host(NativePointerRoster::Unknown)
        .unwrap();
    session
        .configure_managed_native_capabilities(
            NativeHostCapabilities::none_supported()
                .with(NativeHostCapability::AuthoritativeInventory),
        )
        .unwrap();

    // Register SURFACE_A on OS window 77, then host both SURFACE_A and SURFACE_B on it.
    let shared_token = HostWindowToken::new(77);
    session
        .register_native_root(SURFACE_A, shared_token)
        .unwrap();
    let mut frame = session
        .begin_native_host_frame(|_| crate::runtime::NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    let mut report = frame.commit().unwrap();
    let digest = NativeReportDigest::take_from(&mut report);
    let binding_a = session
        .native_window_bindings()
        .next()
        .expect("registered shared window");

    session
        .report_managed_native_snapshot(
            [(
                binding_a,
                NativeWindowFacts::live()
                    .with_content_bounds(PhysicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap())
                    .with_native_scale_factor(ScaleFactor::new(1.0).unwrap())
                    .with_presentation_scale_factor(ScaleFactor::new(1.0).unwrap())
                    .with_input(NativeWindowInputState::ReceivesInput, None)
                    .with_presentation(NativeWindowPresentationState::Visible),
            )],
            NativeWorkAreaRoster::Unknown,
        )
        .unwrap();
    let placements = [
        NativeSurfacePlacement::new(
            SURFACE_A,
            LogicalSize::new(400.0, 600.0).unwrap(),
            PhysicalPoint::new(0.0, 0.0).unwrap(),
            ScaleFactor::new(1.0).unwrap(),
            PhysicalRect::new(0.0, 0.0, 400.0, 600.0).unwrap(),
        ),
        NativeSurfacePlacement::new(
            SURFACE_B,
            LogicalSize::new(400.0, 600.0).unwrap(),
            PhysicalPoint::new(400.0, 0.0).unwrap(),
            ScaleFactor::new(1.0).unwrap(),
            PhysicalRect::new(400.0, 0.0, 400.0, 600.0).unwrap(),
        ),
    ];
    let mut frame = session
        .begin_native_host_frame(|_| crate::runtime::NativeReceiverAnswer::Unknown)
        .unwrap();
    frame
        .set_native_surface_placements(binding_a, &placements)
        .unwrap();
    frame
        .complete_unpainted_surfaces(SurfaceUnavailableReason::Deferred)
        .unwrap();
    frame.commit().unwrap();

    let mut windows: NativeWindows<u64, ()> = NativeWindows::new();
    windows.insert(binding_a, 77, ()).unwrap();

    // Dirty BOTH SURFACE_A and SURFACE_B in one frame; dirty_windows must return window 77 once.
    let mut frame = session
        .begin_native_host_frame(|_| crate::runtime::NativeReceiverAnswer::Unknown)
        .unwrap();
    frame.select_item_current(ITEM_2).unwrap();
    frame.select_item_current(ITEM_4).unwrap();
    let mut report = frame.commit().unwrap();
    let mut digest_after_select = NativeReportDigest::take_from(&mut report);
    assert!(digest_after_select.take_effects().is_empty());
    assert_eq!(
        digest_after_select.repaint_surfaces(),
        &[SURFACE_A, SURFACE_B]
    );
    let dirty = digest_after_select.dirty_windows(&session, &windows);
    assert_eq!(dirty, vec![(binding_a, 77)]);
    assert!(digest.apply_to_windows(&mut windows).unwrap().is_empty());

    // Retirement state must withhold destruction facts and quiescence until every gate holds.
    let mut retirement = NativeWindowRetirementState::from_session(&session, binding_a, None);
    assert_eq!(retirement.placements().len(), 2);
    assert!(retirement.destruction_facts().is_none());
    assert!(!retirement.is_ready_for_quiescence());
    assert!(!retirement.report_quiescence_if_ready(&mut session).unwrap());

    retirement.mark_native_destroyed();
    assert_eq!(
        retirement.destruction_facts(),
        Some(NativeWindowFacts::destroyed())
    );
    // Publish destruction so core retires the window bindings before quiescence is accepted.
    session
        .report_managed_native_snapshot(
            [(binding_a, retirement.destruction_facts().unwrap())],
            NativeWorkAreaRoster::Unknown,
        )
        .unwrap();
    session
        .begin_native_host_frame(|_| crate::runtime::NativeReceiverAnswer::Unknown)
        .unwrap()
        .commit()
        .unwrap();

    // Still missing outputs_retired and callbacks_sealed!
    assert!(!retirement.report_quiescence_if_ready(&mut session).unwrap());
    retirement.mark_outputs_retired();
    assert!(!retirement.report_quiescence_if_ready(&mut session).unwrap());
    retirement.mark_callbacks_sealed();
    assert!(retirement.is_ready_for_quiescence());
    assert!(retirement.report_quiescence_if_ready(&mut session).unwrap());
    assert!(retirement.is_quiescent());
}

fn paint_and_settle(
    host: &mut LogicalHost,
    surface: SurfaceId,
    metrics: &mut SurfaceMetrics,
) -> SurfaceDisplayList {
    let (display_list, token) = match host.paint_display_list(surface, metrics).unwrap() {
        LogicalPaintOutcome::Ready {
            payload,
            presentation,
            ..
        } => (payload, presentation),
        LogicalPaintOutcome::Deferred { reason, .. } => {
            panic!("unexpected deferred paint: {reason:?}")
        }
    };
    host.finish_presentation(token, SurfacePresentationResult::Presented)
        .unwrap();
    display_list
}

#[test]
fn logical_host_tab_drag_splits_pane_and_emits_drag_display_steps() {
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE_A,
        DockspaceRootLayout::new(ROOT_A, DockspaceNode::tabs([ITEM_1, ITEM_2])),
    )])
    .unwrap();
    let mut host = LogicalHost::from_layout(layout, DockPolicy::default()).unwrap();
    let bounds = LogicalRect::new(0.0, 0.0, 800.0, 600.0).unwrap();
    let mut metrics = SurfaceMetrics::new(bounds);

    let initial = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert_eq!(initial.splitters().count(), 0);
    let tab_2 = initial
        .tabs()
        .find(|tab| tab.item == Some(ITEM_2))
        .expect("second tab visible");
    let start = rect_center(tab_2.drag_bounds);
    let pane_bounds = initial
        .panes()
        .next()
        .unwrap()
        .content_geometry
        .layout_bounds();
    let mid = rect_center(pane_bounds);

    // Begin drag on tab 2 and move into the pane center to activate drag preview and drop guides.
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            start,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        )
        .with_capture(SurfacePointerCapture::ProviderEndpoint),
        LogicalPointerEvent::moved(SURFACE_A, mid, PointerClearance::Unobstructed)
            .with_capture(SurfacePointerCapture::ProviderEndpoint),
    ])
    .unwrap();
    assert_eq!(
        host.active_manipulation_surface(DEFAULT_MOUSE_POINTER),
        Some(SURFACE_A)
    );
    assert!(host.has_active_manipulation(DEFAULT_MOUSE_POINTER));

    let during_drag = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        during_drag.drag_preview().is_some(),
        "active tab drag must emit OwnedDragPreview"
    );
    assert!(
        during_drag.drag_decoration().is_some(),
        "active tab drag must emit OwnedDragDecoration"
    );
    let right_target = during_drag
        .drop_guides()
        .flat_map(|guide| guide.targets.iter())
        .find(|target| target.direction == DockspaceDropDirection::Right)
        .expect("active drag over pane exposes Right split guide target");
    let right_point = rect_center(right_target.hit_bounds);

    // Hover the Right drop guide target, repaint to verify live affordance, then release to split.
    host.submit_pointer(
        LogicalPointerEvent::moved(SURFACE_A, right_point, PointerClearance::Unobstructed)
            .with_capture(SurfacePointerCapture::ProviderEndpoint),
    )
    .unwrap();
    let hovering_guide = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        hovering_guide
            .drop_guides()
            .filter_map(|guide| guide.affordance.as_ref())
            .flat_map(|cluster| cluster.targets.iter())
            .any(|target| target.direction == DockspaceDropDirection::Right && target.is_active),
        "hovering the Right guide target must mark it active in OwnedDropAffordanceTarget"
    );

    host.submit_pointer(LogicalPointerEvent::released(
        SURFACE_A,
        right_point,
        SurfacePointerButton::Primary,
        PointerClearance::Unobstructed,
    ))
    .unwrap();
    assert!(!host.has_active_manipulation(DEFAULT_MOUSE_POINTER));

    let after_drop = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert_eq!(after_drop.panes().count(), 2);
    assert_eq!(after_drop.splitters().count(), 1);
    let pane_1_x = after_drop
        .panes()
        .find(|pane| pane.item == ITEM_1)
        .unwrap()
        .content_geometry
        .layout_bounds()
        .x();
    let pane_2_x = after_drop
        .panes()
        .find(|pane| pane.item == ITEM_2)
        .unwrap()
        .content_geometry
        .layout_bounds()
        .x();
    assert!(
        pane_2_x > pane_1_x,
        "dropping ITEM_2 on Right must place it to the right of ITEM_1"
    );
}

#[test]
fn logical_host_splitter_drag_resizes_adjacent_panes() {
    let mut host = LogicalHost::from_layout(two_tab_layout(), DockPolicy::default()).unwrap();
    let mut metrics = SurfaceMetrics::new(LogicalRect::new(0.0, 0.0, 900.0, 600.0).unwrap());

    let before = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let splitter = before.splitters().next().expect("horizontal splitter");
    let left_width_before = before
        .panes()
        .find(|pane| pane.item == ITEM_1)
        .unwrap()
        .content_geometry
        .layout_bounds()
        .width();
    let start = rect_center(splitter.hit_bounds);
    let dragged = LogicalPoint::new(start.x() - 120.0, start.y()).unwrap();

    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            start,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        )
        .with_capture(SurfacePointerCapture::ProviderEndpoint),
        LogicalPointerEvent::moved(SURFACE_A, dragged, PointerClearance::Unobstructed)
            .with_capture(SurfacePointerCapture::ProviderEndpoint),
        LogicalPointerEvent::released(
            SURFACE_A,
            dragged,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();

    let after = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let left_width_after = after
        .panes()
        .find(|pane| pane.item == ITEM_1)
        .unwrap()
        .content_geometry
        .layout_bounds()
        .width();
    assert!(
        left_width_after < left_width_before - 50.0,
        "dragging splitter left must shrink left pane ({left_width_after} vs {left_width_before})"
    );
}

#[test]
fn logical_host_tab_strip_wheel_scroll_and_popup_menu_selection() {
    let items: Vec<ItemId> = (1..=8).map(ItemId::new).collect();
    let last_item = *items.last().unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE_A,
        DockspaceRootLayout::new(ROOT_A, DockspaceNode::tabs(items)),
    )])
    .unwrap();
    let mut host = LogicalHost::from_layout(layout, DockPolicy::default()).unwrap();
    let mut metrics = SurfaceMetrics::new(LogicalRect::new(0.0, 0.0, 320.0, 260.0).unwrap())
        .with_default_tab_width(110.0)
        .unwrap()
        .with_default_overflow_controls();

    let initial = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let bar = initial.tab_bars().next().expect("overflowing tab bar");
    assert!(
        bar.maximum_scroll_offset > 0.0,
        "8 tabs of 110px in 320px strip must overflow"
    );
    assert_eq!(bar.scroll_offset, 0.0);
    let first_tab_center = rect_center(initial.tabs().next().unwrap().visible_bounds);

    // Blocked wheel over the tab strip must fail closed and leave scroll_offset at 0.
    host.submit_wheel(
        SURFACE_A,
        first_tab_center,
        SurfaceScrollDelta::LogicalPoints { x: -80.0, y: 0.0 },
        PointerClearance::Blocked,
    )
    .unwrap();
    let after_blocked = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert_eq!(after_blocked.tab_bars().next().unwrap().scroll_offset, 0.0);

    // Unobstructed wheel directly over a tab resolves the independent Scroll lane and scrolls.
    host.submit_wheel(
        SURFACE_A,
        first_tab_center,
        SurfaceScrollDelta::LogicalPoints { x: -80.0, y: 0.0 },
        PointerClearance::Unobstructed,
    )
    .unwrap();
    let after_scroll = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        after_scroll.tab_bars().next().unwrap().scroll_offset > 0.0,
        "unobstructed wheel over overflowing tab strip must advance scroll_offset"
    );

    // Click the TabListMenu control to open the owned popup backdrop & menu rows.
    let menu_button = after_scroll
        .tab_strip_controls()
        .find(|control| control.kind == TabStripControlKind::TabListMenu)
        .expect("overflowing strip exposes TabListMenu control");
    let menu_btn_center = rect_center(menu_button.hit_bounds);
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            menu_btn_center,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::released(
            SURFACE_A,
            menu_btn_center,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();

    let with_popup = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        with_popup
            .steps()
            .iter()
            .any(|step| matches!(step, SurfaceDisplayStep::PopupBackdrop(_))),
        "open tab list menu must emit PopupBackdrop"
    );
    let menu = with_popup
        .popup_menus()
        .next()
        .expect("open tab list menu must emit OwnedPopupMenu");
    let target_row = menu
        .rows
        .iter()
        .find(|row| row.item == Some(last_item) && row.interaction_bounds.is_some())
        .or_else(|| {
            menu.rows
                .iter()
                .rev()
                .find(|row| row.item != Some(ITEM_1) && row.interaction_bounds.is_some())
        })
        .expect("popup menu exposes clickable tab rows");
    let chosen_item = target_row.item.unwrap();
    let row_center = rect_center(target_row.interaction_bounds.unwrap());

    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            row_center,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::released(
            SURFACE_A,
            row_center,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();
    assert!(
        host.view().item(chosen_item).unwrap().is_active(),
        "clicking popup menu row must activate its tab"
    );
    let after_select = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        after_select.popup_menus().next().is_none(),
        "selecting a tab closes the popup menu"
    );
}

#[test]
fn logical_host_contained_floating_drag_move_and_resize_emit_preview_and_commit() {
    let initial_rect = LogicalRect::new(80.0, 60.0, 260.0, 180.0).unwrap();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE_A,
        DockspaceRootLayout::new(ROOT_A, DockspaceNode::pane(ITEM_1)),
    )
    .with_contained(DockspaceContainedLayout::new(
        FloatingPresentationId::new(1),
        DockspaceRootLayout::new(ROOT_B, DockspaceNode::tabs([ITEM_2])),
        initial_rect,
    ))])
    .unwrap();

    let mut host = LogicalHost::from_layout(layout, DockPolicy::default()).unwrap();
    let mut metrics = SurfaceMetrics::new(LogicalRect::new(0.0, 0.0, 900.0, 650.0).unwrap());

    let before = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let frame = before
        .contained_frames()
        .next()
        .expect("contained floating frame emitted");
    let title_start = rect_center(frame.title_drag_bounds);
    let title_moved = LogicalPoint::new(title_start.x() + 120.0, title_start.y() + 80.0).unwrap();

    // Drag the contained title bar with docking disabled so it stays floating and moves.
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            title_start,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        )
        .with_docking_enabled(false)
        .with_capture(SurfacePointerCapture::ProviderEndpoint),
        LogicalPointerEvent::moved(SURFACE_A, title_moved, PointerClearance::Unobstructed)
            .with_docking_enabled(false)
            .with_capture(SurfacePointerCapture::ProviderEndpoint),
    ])
    .unwrap();
    let moving = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        moving.contained_transform_preview().is_some() || moving.drag_preview().is_some(),
        "dragging contained title bar must emit transform or drag preview"
    );

    host.submit_pointer(
        LogicalPointerEvent::released(
            SURFACE_A,
            title_moved,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        )
        .with_docking_enabled(false),
    )
    .unwrap();
    let moved = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let moved_frame = moved.contained_frames().next().unwrap();
    assert!(
        moved_frame.outer_bounds.x() > initial_rect.x() + 50.0,
        "contained window x must move right ({} vs {})",
        moved_frame.outer_bounds.x(),
        initial_rect.x()
    );

    // Resize the contained floating window from its East edge.
    let east = moved_frame
        .resize_edges
        .iter()
        .find(|edge| edge.direction == ContainedResizeDirection::East && edge.operable)
        .expect("contained frame exposes operable East resize edge");
    let resize_start = rect_center(east.hit_bounds);
    let resize_end = LogicalPoint::new(resize_start.x() + 90.0, resize_start.y()).unwrap();
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            resize_start,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        )
        .with_capture(SurfacePointerCapture::ProviderEndpoint),
        LogicalPointerEvent::moved(SURFACE_A, resize_end, PointerClearance::Unobstructed)
            .with_capture(SurfacePointerCapture::ProviderEndpoint),
    ])
    .unwrap();
    let resizing = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    assert!(
        resizing.contained_transform_preview().is_some(),
        "resizing contained floating window must emit OwnedContainedTransformPreview"
    );
    host.submit_pointer(LogicalPointerEvent::released(
        SURFACE_A,
        resize_end,
        SurfacePointerButton::Primary,
        PointerClearance::Unobstructed,
    ))
    .unwrap();
    let resized = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let resized_frame = resized.contained_frames().next().unwrap();
    assert!(
        resized_frame.outer_bounds.width() > moved_frame.outer_bounds.width() + 50.0,
        "resizing East edge must widen contained window ({} vs {})",
        resized_frame.outer_bounds.width(),
        moved_frame.outer_bounds.width()
    );
}

#[test]
fn visitor_receives_drag_and_popup_steps_matching_display_list() {
    #[derive(Default)]
    struct OverlayVisitor {
        drag_previews: usize,
        drop_guides: usize,
        drag_decorations: usize,
        popup_backdrops: usize,
        popup_menus: usize,
        controls: usize,
    }

    impl SurfaceProgramVisitor for OverlayVisitor {
        type Error = std::convert::Infallible;

        fn tab_strip_control(
            &mut self,
            _root: SurfacePaintProgramRoot<'_>,
            _control: crate::runtime::TabStripControlPaintRecord,
            _paint: SurfacePaintDisposition,
        ) -> Result<(), Self::Error> {
            self.controls += 1;
            Ok(())
        }

        fn popup_backdrop(
            &mut self,
            _backdrop: crate::runtime::TabListMenuBackdropPaintRecord,
        ) -> Result<(), Self::Error> {
            self.popup_backdrops += 1;
            Ok(())
        }

        fn popup_menu(
            &mut self,
            _menu: crate::runtime::TabListMenuPaintRecord<'_>,
            _paint: SurfacePaintDisposition,
        ) -> Result<(), Self::Error> {
            self.popup_menus += 1;
            Ok(())
        }

        fn drag_preview(
            &mut self,
            _preview: crate::runtime::DockspaceDragPreview<'_>,
        ) -> Result<(), Self::Error> {
            self.drag_previews += 1;
            Ok(())
        }

        fn drop_guide(
            &mut self,
            _structural: crate::runtime::DropGuidePaintRecord<'_>,
            _affordance: Option<crate::runtime::DropAffordanceClusterPaintRecord<'_>>,
        ) -> Result<(), Self::Error> {
            self.drop_guides += 1;
            Ok(())
        }

        fn drag_decoration(
            &mut self,
            _decoration: crate::runtime::DockspaceDragDecoration<'_>,
        ) -> Result<(), Self::Error> {
            self.drag_decorations += 1;
            Ok(())
        }
    }

    let items: Vec<ItemId> = (1..=6).map(ItemId::new).collect();
    let layout = DockspaceLayout::new([DockspaceSurfaceLayout::new(
        SURFACE_A,
        DockspaceRootLayout::new(ROOT_A, DockspaceNode::tabs(items)),
    )])
    .unwrap();
    let mut host = LogicalHost::from_layout(layout, DockPolicy::default()).unwrap();
    let mut metrics = SurfaceMetrics::new(LogicalRect::new(0.0, 0.0, 320.0, 260.0).unwrap())
        .with_default_tab_width(110.0)
        .unwrap()
        .with_default_overflow_controls();

    let initial = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let menu_btn = initial
        .tab_strip_controls()
        .find(|control| control.kind == TabStripControlKind::TabListMenu)
        .expect("tab list menu button");
    let menu_btn_center = rect_center(menu_btn.hit_bounds);
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            menu_btn_center,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::released(
            SURFACE_A,
            menu_btn_center,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();

    let mut popup_visitor = OverlayVisitor::default();
    let token = match host
        .paint_with_visitor(SURFACE_A, &mut metrics, &mut popup_visitor)
        .unwrap()
    {
        LogicalPaintOutcome::Ready { presentation, .. } => presentation,
        LogicalPaintOutcome::Deferred { .. } => panic!("expected ready paint"),
    };
    assert!(popup_visitor.controls >= 1);
    assert_eq!(popup_visitor.popup_backdrops, 1);
    assert_eq!(popup_visitor.popup_menus, 1);
    host.finish_presentation(token, SurfacePresentationResult::Presented)
        .unwrap();

    // Dismiss popup by clicking backdrop outside the menu, then drag a tab and verify visitor.
    let dismiss = LogicalPoint::new(10.0, 250.0).unwrap();
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            dismiss,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
        LogicalPointerEvent::released(
            SURFACE_A,
            dismiss,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        ),
    ])
    .unwrap();
    let dismissed = paint_and_settle(&mut host, SURFACE_A, &mut metrics);
    let tab_start = rect_center(dismissed.tabs().next().unwrap().drag_bounds);
    let pane_mid = rect_center(
        dismissed
            .panes()
            .next()
            .unwrap()
            .content_geometry
            .layout_bounds(),
    );
    host.submit_pointer_batch([
        LogicalPointerEvent::pressed(
            SURFACE_A,
            tab_start,
            SurfacePointerButton::Primary,
            PointerClearance::Unobstructed,
        )
        .with_capture(SurfacePointerCapture::ProviderEndpoint),
        LogicalPointerEvent::moved(SURFACE_A, pane_mid, PointerClearance::Unobstructed)
            .with_capture(SurfacePointerCapture::ProviderEndpoint),
    ])
    .unwrap();

    let mut drag_visitor = OverlayVisitor::default();
    let token = match host
        .paint_with_visitor(SURFACE_A, &mut metrics, &mut drag_visitor)
        .unwrap()
    {
        LogicalPaintOutcome::Ready { presentation, .. } => presentation,
        LogicalPaintOutcome::Deferred { .. } => panic!("expected ready paint"),
    };
    assert_eq!(drag_visitor.drag_previews, 1);
    assert!(drag_visitor.drop_guides >= 1);
    assert_eq!(drag_visitor.drag_decorations, 1);
    host.finish_presentation(token, SurfacePresentationResult::Presented)
        .unwrap();
}
