//! Interrupt a visible native preview, then verify cancellation or geometry refresh.

use dockspace::model::{FloatingPresentationId, RootId};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum Interruption {
    Escape,
    Deactivate,
    Resize,
    Zoom,
    TargetClose,
}

#[derive(Debug, PartialEq, Eq)]
struct PanePlacement {
    surface: SurfaceId,
    root: RootId,
    contained: Option<FloatingPresentationId>,
    items: Vec<ItemId>,
    selected: Option<ItemId>,
}

type Placements = BTreeMap<ItemId, PanePlacement>;

#[derive(Debug)]
enum Phase {
    Preview,
    Cancel {
        placements: Placements,
        signals: usize,
        releases: usize,
        geometry: Option<geometry::ChangedGeometry>,
    },
    Retired {
        placements: Placements,
        releases: usize,
    },
    Tail {
        placements: Placements,
        releases: usize,
        sent: bool,
    },
}

#[derive(Debug)]
pub(super) struct InterruptedDrag {
    kind: Interruption,
    phase: Phase,
}

impl InterruptedDrag {
    pub(super) fn new(kind: Interruption) -> Self {
        Self {
            kind,
            phase: Phase::Preview,
        }
    }

    pub(super) fn advance(
        &mut self,
        app: &NativeExampleApp,
        context: &egui::Context,
        observations: &Arc<Mutex<EditorObservations>>,
        (child, window, surface): (egui::ViewportId, u64, SurfaceId),
        expected: &[ItemId],
        external: &Receiver<String>,
    ) -> bool {
        let observed = observations.lock().unwrap();
        let preview_visible = observed
            .viewports
            .get(&child)
            .is_some_and(|sample| sample.preview_tabs.as_deref() == Some(expected));
        let child_closed = self.kind == Interruption::TargetClose
            && app
                .dockspace
                .with_view(|view| view.surface(surface).is_none());
        let preview_gone = observed
            .viewports
            .iter()
            .all(|(id, sample)| (child_closed && *id == child) || sample.preview_tabs.is_none());
        let releases = observed
            .viewports
            .values()
            .map(|sample| sample.releases)
            .sum();
        let signals = observed
            .viewports
            .values()
            .map(|sample| match self.kind {
                Interruption::Escape => sample.escape_presses,
                Interruption::Deactivate => sample.focus_losses,
                Interruption::Resize | Interruption::Zoom | Interruption::TargetClose => 0,
            })
            .sum::<usize>();
        match &mut self.phase {
            Phase::Preview if preview_visible && observed.root().primary_down => {
                let placements = placements(app);
                let action = match self.kind {
                    Interruption::Escape => "escape",
                    Interruption::Deactivate => "deactivate",
                    Interruption::Resize => "resize",
                    Interruption::Zoom => "zoom",
                    Interruption::TargetClose => "target-close",
                };
                eprintln!("editor-native-interrupt: {action} preview=visible primary=held");
                let geometry = match self.kind {
                    Interruption::Resize | Interruption::Zoom => Some(
                        geometry::ChangedGeometry::request(self.kind, context, &observed, child),
                    ),
                    // The driver activates the owned window's native close
                    // button, preserving the physical primary press.
                    Interruption::TargetClose | Interruption::Escape | Interruption::Deactivate => {
                        None
                    }
                };
                self.phase = Phase::Cancel {
                    placements,
                    signals,
                    releases,
                    geometry,
                };
            }
            Phase::Cancel {
                placements: before,
                signals: baseline,
                releases: original_releases,
                geometry,
            } if preview_gone
                && match self.kind {
                    Interruption::Escape | Interruption::Deactivate => signals > *baseline,
                    Interruption::Resize | Interruption::Zoom => geometry
                        .as_ref()
                        .unwrap()
                        .observed(context, &observed, child),
                    Interruption::TargetClose => child_closed,
                } =>
            {
                assert_eq!(
                    releases, *original_releases,
                    "cancel before the real release"
                );
                if self.kind == Interruption::TargetClose {
                    let recovered = placements(app);
                    assert_recovered(app, before, &recovered);
                    eprintln!("editor-native-interrupt-target-retired: {child:?} {window}");
                    self.phase = Phase::Retired {
                        placements: recovered,
                        releases,
                    };
                    return false;
                }
                assert_eq!(
                    &placements(app),
                    before,
                    "revoking a preview must preserve all tabs"
                );
                // Once the changed source has been presented, its original
                // desktop drag can acquire a fresh target on the real release.
                // No extra move or replacement press is needed.
                let tail = if geometry.is_some() {
                    "release-only"
                } else {
                    "move-release"
                };
                eprintln!(
                    "editor-native-interrupt-cancelled: preview=absent tabs=preserved tail={tail}"
                );
                self.phase = Phase::Tail {
                    placements: std::mem::take(before),
                    releases,
                    sent: false,
                };
            }
            Phase::Retired {
                placements: recovered,
                releases,
            } => match external.try_recv() {
                Ok(line) => {
                    assert_eq!(line.trim(), "native-interrupt-target-retirement-observed");
                    assert_eq!(&placements(app), recovered);
                    eprintln!(
                        "editor-native-interrupt-cancelled: preview=absent tabs=recovered tail=move-release"
                    );
                    self.phase = Phase::Tail {
                        placements: std::mem::take(recovered),
                        releases: *releases,
                        sent: false,
                    };
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(error) => panic!("the retired-window observer disconnected: {error}"),
            },
            Phase::Tail {
                placements: before,
                releases: baseline,
                sent,
            } => {
                if !*sent {
                    match external.try_recv() {
                        Ok(line) => {
                            assert_eq!(line.trim(), "native-interrupt-tail-sent");
                            *sent = true;
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(error) => panic!("the interrupt driver disconnected: {error}"),
                    }
                }
                if *sent && releases > *baseline && preview_gone {
                    assert_eq!(before.len(), 8);
                    if matches!(self.kind, Interruption::Resize | Interruption::Zoom) {
                        assert_continued(app, before, expected);
                        eprintln!(
                            "editor-native-interrupt-verified: geometry=refreshed continuation=true"
                        );
                        return true;
                    }
                    assert_eq!(&placements(app), before, "late move/up must not dock");
                    let surfaces = if self.kind == Interruption::TargetClose {
                        1
                    } else {
                        2
                    };
                    assert_eq!(
                        app.dockspace.with_view(|view| view.surfaces().count()),
                        surfaces
                    );
                    eprintln!("editor-native-interrupt-verified: late-tail=inert fresh-drag=next");
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub(super) fn recreates_child(&self) -> bool {
        self.kind == Interruption::TargetClose
    }

    pub(super) fn continues_drag(&self) -> bool {
        matches!(self.kind, Interruption::Resize | Interruption::Zoom)
    }
}

fn assert_continued(app: &NativeExampleApp, before: &Placements, expected: &[ItemId]) {
    let after = placements(app);
    let target = &before[&INSPECTOR];
    let source = &before[&HISTORY];
    let remaining: Vec<_> = source
        .items
        .iter()
        .copied()
        .filter(|item| *item != HISTORY)
        .collect();
    assert_eq!(after.len(), 8);
    for (&item, original) in before {
        let current = &after[&item];
        if expected.contains(&item) {
            assert_eq!(current.surface, target.surface);
            assert_eq!(current.root, target.root);
            assert_eq!(current.contained, target.contained);
            assert_eq!(current.items, expected);
            assert_eq!(current.selected, Some(HISTORY));
        } else if source.items.contains(&item) {
            assert_eq!(current.surface, source.surface);
            assert_eq!(current.root, source.root);
            assert_eq!(current.contained, source.contained);
            assert_eq!(current.items, remaining);
            assert!(
                current
                    .selected
                    .is_some_and(|item| remaining.contains(&item))
            );
        } else {
            assert_eq!(current, original, "unrelated panes must not change");
        }
    }
    assert_eq!(app.dockspace.with_view(|view| view.surfaces().count()), 2);
}

fn assert_recovered(app: &NativeExampleApp, before: &Placements, recovered: &Placements) {
    assert_eq!(before.len(), 8);
    assert_eq!(recovered.len(), 8);
    for (&item, original) in before {
        let current = &recovered[&item];
        if original.root == INSPECTOR_ROOT {
            assert_eq!(current.root, INSPECTOR_ROOT);
            assert_eq!(current.surface, SURFACE);
            assert_eq!(current.contained, Some(INSPECTOR_FLOATING));
            assert_eq!(current.items, original.items);
            assert_eq!(current.selected, original.selected);
        } else {
            assert_eq!(
                current, original,
                "closing the target cannot move the dragged source or any other pane"
            );
        }
    }
    app.dockspace.with_view(|view| {
        assert_eq!(view.surfaces().count(), 1);
        assert_eq!(view.surface(SURFACE).unwrap().contained_count(), 1);
        assert_eq!(
            view.contained(INSPECTOR_FLOATING)
                .unwrap()
                .root()
                .unwrap()
                .id(),
            INSPECTOR_ROOT
        );
    });
}

fn placements(app: &NativeExampleApp) -> Placements {
    app.dockspace.with_view(|view| {
        [
            OUTLINE, EDITOR, PREVIEW, INSPECTOR, CONSOLE, HISTORY, PROFILER, ASSETS,
        ]
        .into_iter()
        .map(|id| {
            let pane = view.item(id).expect("every original pane remains owned");
            (
                id,
                PanePlacement {
                    surface: pane.surface(),
                    root: pane.root(),
                    contained: pane.contained(),
                    items: pane.tabs().items().to_vec(),
                    selected: pane.tabs().selected(),
                },
            )
        })
        .collect()
    })
}
