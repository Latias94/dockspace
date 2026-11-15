//! Interrupt a visible native drop preview, reject its tail, then allow a fresh drag.

use dockspace::model::{FloatingPresentationId, RootId};

use super::*;

#[derive(Clone, Copy, Debug)]
pub(in super::super) enum Interruption {
    Escape,
    Deactivate,
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
        observations: &Arc<Mutex<EditorObservations>>,
        child: egui::ViewportId,
        expected: &[ItemId],
        external: &Receiver<String>,
    ) -> bool {
        let observed = observations.lock().unwrap();
        let preview_visible = observed
            .viewports
            .get(&child)
            .is_some_and(|sample| sample.preview_tabs.as_deref() == Some(expected));
        let preview_gone = observed
            .viewports
            .values()
            .all(|sample| sample.preview_tabs.is_none());
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
            })
            .sum::<usize>();
        match &mut self.phase {
            Phase::Preview if preview_visible && observed.root().primary_down => {
                let placements = placements(app);
                let action = match self.kind {
                    Interruption::Escape => "escape",
                    Interruption::Deactivate => "deactivate",
                };
                eprintln!("editor-native-interrupt: {action} preview=visible primary=held");
                self.phase = Phase::Cancel {
                    placements,
                    signals,
                    releases,
                };
            }
            Phase::Cancel {
                placements: before,
                signals: baseline,
                releases: original_releases,
            } if signals > *baseline && preview_gone => {
                assert_eq!(
                    &placements(app),
                    before,
                    "cancellation must preserve all tabs"
                );
                assert_eq!(
                    releases, *original_releases,
                    "cancel before the real release"
                );
                eprintln!("editor-native-interrupt-cancelled: preview=absent tabs=preserved");
                self.phase = Phase::Tail {
                    placements: std::mem::take(before),
                    releases,
                    sent: false,
                };
            }
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
                    assert_eq!(&placements(app), before, "late move/up must not dock");
                    assert_eq!(before.len(), 8);
                    assert_eq!(app.dockspace.with_view(|view| view.surfaces().count()), 2);
                    eprintln!("editor-native-interrupt-verified: late-tail=inert fresh-drag=next");
                    return true;
                }
            }
            _ => {}
        }
        false
    }
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
