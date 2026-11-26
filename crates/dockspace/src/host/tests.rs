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
    DockspaceFocusTarget, DockspacePaneFocusObservation, DockspaceSession, HostWindowToken,
    NativeHostCapabilities, NativeHostCapability, NativePointerRoster, NativeSurfacePlacement,
    NativeWindowFacts, NativeWindowInputState, NativeWindowPresentationState, NativeWorkAreaRoster,
    SurfacePaintDisposition, SurfacePaintProgramRoot, SurfacePaintProgramVisitError,
    SurfacePointerButton, SurfacePointerEvent, SurfacePresentationResult, SurfaceUnavailableReason,
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
