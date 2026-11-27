//! Bounded physical-input journey observed after the exact WGPU output is presented.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use egui::{Pos2, RawInput, Rect, accesskit::Orientation, accesskit::Role};
use egui_dockspace::winit::NativeFrameReport;
use egui_dockspace::{Dockspace, ItemId, RootId, SurfacePresentationResult, WorkspaceVersion};

use super::editor::{CONSOLE, DemoPanes, EDITOR, INSPECTOR, MAIN_ROOT, OUTLINE, PREVIEW, SURFACE};

const INSERTED: &str = "waylandinput";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Preview,
    Inspector,
    Resize,
    Scale,
    Edit,
    Scroll,
    Complete,
}

pub(super) struct PhysicalSmoke {
    started: Instant,
    ready_at: Instant,
    phase: Phase,
    announced: bool,
    guide_sent: bool,
    primary_down: bool,
    releases: usize,
    texts: usize,
    wheels: usize,
    baseline: usize,
    target: Rect,
    source: Option<Pos2>,
    hover_confirmed: bool,
    weights: [u64; 2],
    width: f32,
    scroll: f32,
    content_version: Option<WorkspaceVersion>,
    scale_change: Option<ScaleChange>,
    original: BTreeMap<ItemId, String>,
    frames: usize,
}

struct ScaleChange {
    target: f32,
    logical_size: egui::Vec2,
    zoom: f32,
    version: WorkspaceVersion,
    weights: [u64; 2],
}

fn configured_target_scale(current: f32) -> Option<f32> {
    if std::env::var("DOCKSPACE_PHYSICAL_SCALE_CHANGE").as_deref() != Ok("1") {
        return None;
    }
    match std::env::var("DOCKSPACE_PHYSICAL_TARGET_SCALE") {
        Ok(raw) => raw
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0),
        Err(_) => Some(current * 2.0),
    }
}

impl PhysicalSmoke {
    pub(super) fn new(panes: &DemoPanes) -> Self {
        let original = panes
            .panes
            .iter()
            .map(|(&id, p)| (id, p.text.clone()))
            .collect();
        Self {
            started: Instant::now(),
            ready_at: Instant::now() + Duration::from_millis(250),
            phase: Phase::Preview,
            announced: false,
            guide_sent: false,
            primary_down: false,
            releases: 0,
            texts: 0,
            wheels: 0,
            baseline: 0,
            target: Rect::NOTHING,
            source: None,
            hover_confirmed: false,
            weights: [0; 2],
            width: 0.0,
            scroll: 0.0,
            content_version: None,
            scale_change: None,
            original,
            frames: 0,
        }
    }

    pub(super) fn observe_input(&mut self, input: &RawInput) {
        for event in &input.events {
            match event {
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    self.primary_down = *pressed;
                    self.releases += usize::from(!pressed);
                }
                egui::Event::Text(_) | egui::Event::Paste(_) => self.texts += 1,
                egui::Event::MouseWheel { .. } => self.wheels += 1,
                _ => {}
            }
        }
    }

    pub(super) fn require_complete(&self) -> Result<(), String> {
        (self.phase == Phase::Complete)
            .then_some(())
            .ok_or_else(|| format!("physical workbench stopped before {:?}", self.phase))
    }

    pub(super) fn finish_frame(
        &mut self,
        panes: &DemoPanes,
        dock: &Dockspace,
        frame: &NativeFrameReport<'_>,
    ) -> Result<bool, String> {
        if self.started.elapsed() > Duration::from_secs(60) {
            let detail = (self.phase, self.primary_down, self.releases, dock.view());
            return Err(format!(
                "physical workbench timed out: {detail:?}; scale={}; input={:?}; focus={:?}",
                frame.context.pixels_per_point(),
                frame
                    .context
                    .input(|input| (input.focused, input.pointer.hover_pos())),
                frame.context.memory(|memory| memory.focused())
            ));
        }
        if frame.presentation != SurfacePresentationResult::Presented {
            return Ok(false);
        }
        self.frames += 1;
        let owns_all = self
            .original
            .keys()
            .all(|id| dock.view().item(*id).is_some());
        if frame.primitives.is_empty() || panes.panes.len() != 8 || !owns_all {
            return Err("physical workbench lost painted content or an original pane".into());
        }
        for (&id, text) in &self.original {
            if (id != EDITOR || !matches!(self.phase, Phase::Edit | Phase::Scroll))
                && panes.panes[&id].text != *text
            {
                return Err(format!(
                    "physical workbench changed unrelated text for {id:?}"
                ));
            }
        }
        let Some(update) = frame.accesskit_update else {
            return Ok(false);
        };
        let mut tabs = BTreeMap::new();
        let mut guides = Vec::new();
        let mut splitters = Vec::new();
        for (_, node) in &update.nodes {
            let Some(bounds) = node.bounds() else {
                continue;
            };
            let rect = Rect::from_min_max(
                egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
            );
            match (node.role(), node.label()) {
                (Role::Tab, Some(title)) => {
                    tabs.insert(title, rect);
                }
                (Role::Label, Some("Dockspace center drop guide")) => guides.push(rect),
                (Role::Splitter, Some("Resize panes"))
                    if !node.is_disabled() && node.orientation() == Some(Orientation::Vertical) =>
                {
                    splitters.push(rect);
                }
                _ => {}
            }
        }
        let scale = frame.context.pixels_per_point();
        if !self.announced {
            if Instant::now() < self.ready_at || self.primary_down || dock.has_active_gesture() {
                return Ok(false);
            }
            self.baseline = self.releases;
            match self.phase {
                Phase::Preview | Phase::Inspector => {
                    let (title, label) = if self.phase == Phase::Preview {
                        ("Preview", "preview")
                    } else {
                        ("Inspector", "inspector")
                    };
                    let Some(source) = tabs.get(title) else {
                        return Ok(false);
                    };
                    let Some(target) = outline_bounds(panes, dock) else {
                        return Ok(false);
                    };
                    self.target = target;
                    if self.phase == Phase::Preview {
                        let size = frame.context.viewport_rect().size() * scale;
                        eprintln!(
                            "workbench-physical-ready: {} {} {scale}",
                            size.x.round(),
                            size.y.round()
                        );
                    }
                    let from = source.min + source.size() * 0.35;
                    self.source = Some(from);
                    pointer(label, from, target.center(), scale);
                }
                Phase::Resize => {
                    let Some(target) = outline_bounds(panes, dock) else {
                        return Ok(false);
                    };
                    let Some(divider) = splitters
                        .iter()
                        .filter(|rect| rect.height() > target.height() * 0.8)
                        .min_by(|a, b| {
                            a.center()
                                .distance_sq(target.right_center())
                                .total_cmp(&b.center().distance_sq(target.right_center()))
                        })
                    else {
                        return Ok(false);
                    };
                    self.width = target.width();
                    self.weights = top_weights(dock);
                    self.source = Some(divider.center());
                    pointer(
                        "resize",
                        divider.center(),
                        divider.center() + egui::vec2(60.0, 0.0),
                        scale,
                    );
                }
                Phase::Scale => {
                    let target = configured_target_scale(scale)
                        .ok_or_else(|| "invalid DOCKSPACE_PHYSICAL_TARGET_SCALE".to_owned())?;
                    self.scale_change = Some(ScaleChange {
                        target,
                        logical_size: frame.context.viewport_rect().size(),
                        zoom: frame.context.zoom_factor(),
                        version: dock.version(),
                        weights: top_weights(dock),
                    });
                    eprintln!("workbench-scale-change: {scale} {target}");
                }
                Phase::Edit => {
                    let Some(edit) = panes.panes[&EDITOR].editor else {
                        return Ok(false);
                    };
                    self.content_version = Some(dock.version());
                    self.baseline = self.texts;
                    point("workbench-text", edit.center(), scale);
                }
                Phase::Scroll => {
                    let Some(bounds) = panes.panes[&CONSOLE].scroll_bounds else {
                        return Ok(false);
                    };
                    self.scroll = panes.panes[&CONSOLE].scroll_offset;
                    self.baseline = self.wheels;
                    point("workbench-scroll", bounds.center(), scale);
                }
                Phase::Complete => return Ok(true),
            }
            self.announced = true;
            return Ok(false);
        }
        if let Some(source) = self.source {
            // Give the UI a presented hover frame before pressing, then wait
            // for the actual press before moving. Startup scheduling and seat
            // focus must not be approximated by a fixed driver delay.
            let (focused, hovered, pressed) = frame.context.input(|input| {
                (
                    input.focused,
                    input.pointer.hover_pos(),
                    input
                        .pointer
                        .press_origin()
                        .filter(|_| input.pointer.primary_down()),
                )
            });
            if !self.hover_confirmed
                && focused
                && hovered.is_some_and(|point| point.distance(source) <= 1.0)
            {
                self.hover_confirmed = true;
                eprintln!("workbench-press:");
            } else if self.hover_confirmed
                && self.primary_down
                && pressed.is_some_and(|point| point.distance(source) <= 1.0)
            {
                self.source = None;
                eprintln!("workbench-drag:");
            }
            return Ok(false);
        }
        match self.phase {
            Phase::Preview | Phase::Inspector => {
                if self.primary_down
                    && !self.guide_sent
                    && let Some(guide) = guides
                        .iter()
                        .filter(|rect| self.target.contains(rect.center()))
                        .min_by(|a, b| {
                            a.center()
                                .distance_sq(self.target.center())
                                .total_cmp(&b.center().distance_sq(self.target.center()))
                        })
                {
                    point("workbench-target", guide.center(), scale);
                    self.guide_sent = true;
                }
                let expected: &[ItemId] = if self.phase == Phase::Preview {
                    &[OUTLINE, PREVIEW]
                } else {
                    &[OUTLINE, PREVIEW, INSPECTOR]
                };
                let view = dock.view();
                let merged = view
                    .item(OUTLINE)
                    .unwrap()
                    .tabs()
                    .expect("fixture item is an inline tab");
                if self.guide_sent
                    && !self.primary_down
                    && self.releases > self.baseline
                    && !dock.has_active_gesture()
                    && merged.items().collect::<Vec<_>>() == expected
                    && merged.selected().and_then(|tab| tab.item()) == expected.last().copied()
                {
                    assert_eq!(
                        view.item(EDITOR)
                            .unwrap()
                            .tabs()
                            .expect("item is an inline tab")
                            .items()
                            .collect::<Vec<_>>(),
                        [EDITOR]
                    );
                    if self.phase == Phase::Inspector {
                        assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 0);
                        assert!(view.root(RootId::new(2)).is_none());
                    }
                    self.next(if self.phase == Phase::Preview {
                        Phase::Inspector
                    } else {
                        Phase::Resize
                    });
                }
            }
            Phase::Resize
                if !self.primary_down
                    && self.releases > self.baseline
                    && !dock.has_active_gesture()
                    && top_weights(dock) != self.weights =>
            {
                if outline_bounds(panes, dock)
                    .is_some_and(|rect| (rect.width() - self.width).abs() > 4.0)
                {
                    self.next(
                        if std::env::var("DOCKSPACE_PHYSICAL_SCALE_CHANGE").as_deref() == Ok("1") {
                            Phase::Scale
                        } else {
                            Phase::Edit
                        },
                    );
                }
            }
            Phase::Scale => {
                let before = self.scale_change.as_ref().unwrap();
                if dock.version() != before.version || top_weights(dock) != before.weights {
                    return Err("physical output scaling changed the docking layout".into());
                }
                if (scale - before.target).abs() < 0.01
                    && !self.primary_down
                    && !dock.has_active_gesture()
                {
                    let size = frame.context.viewport_rect().size();
                    if (size - before.logical_size).abs().max_elem() > 1.0
                        || (frame.context.zoom_factor() - before.zoom).abs() > 0.001
                    {
                        return Err(
                            "physical output scaling changed logical client size or UI zoom".into(),
                        );
                    }
                    eprintln!(
                        "workbench-scale-verified: {scale} logical_size=preserved layout=preserved"
                    );
                    self.next(Phase::Edit);
                }
            }
            Phase::Edit if self.texts > self.baseline && edited(panes, &self.original) => {
                self.next(Phase::Scroll)
            }
            Phase::Scroll
                if self.wheels > self.baseline
                    && panes.panes[&CONSOLE].scroll_offset > self.scroll + 1.0 =>
            {
                if !edited(panes, &self.original) || self.content_version != Some(dock.version()) {
                    return Err(
                        "physical content input changed text unexpectedly or mutated docking"
                            .into(),
                    );
                }
                self.next(Phase::Complete);
                eprintln!(
                    "workbench-physical: PASS; merges=2 contained_retired=true resized=true text={INSERTED} wheel=true panes=8 presented_frames={}",
                    self.frames
                );
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.announced = false;
        self.guide_sent = false;
        self.hover_confirmed = false;
        self.ready_at = Instant::now() + Duration::from_millis(250);
    }
}

fn edited(panes: &DemoPanes, original: &BTreeMap<ItemId, String>) -> bool {
    let text = &panes.panes[&EDITOR].text;
    text.matches(INSERTED).count() == 1 && text.replacen(INSERTED, "", 1) == original[&EDITOR]
}

fn pointer(label: &str, from: Pos2, to: Pos2, scale: f32) {
    let (from, to) = (from * scale, to * scale);
    eprintln!(
        "workbench-pointer: {label} {} {} {} {}",
        from.x.round(),
        from.y.round(),
        to.x.round(),
        to.y.round()
    );
}

fn point(label: &str, point: Pos2, scale: f32) {
    let point = point * scale;
    eprintln!("{label}: {} {}", point.x.round(), point.y.round());
}

fn top_weights(dock: &Dockspace) -> [u64; 2] {
    let root = dock.view().root(MAIN_ROOT).unwrap();
    let rows = root.content().unwrap().split().unwrap();
    let top = rows.children().next().unwrap().split().unwrap();
    let mut weights = top.weights();
    [
        weights.next().unwrap().to_bits(),
        weights.next().unwrap().to_bits(),
    ]
}

fn outline_bounds(panes: &DemoPanes, dock: &Dockspace) -> Option<Rect> {
    let tabs = dock.view().item(OUTLINE)?.tabs()?;
    panes.panes[&tabs.selected().and_then(|tab| tab.item())?].content
}
