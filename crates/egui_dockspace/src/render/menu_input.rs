//! Route keys by event order before egui applies the packet's final focus.

use egui::accesskit::{Action, ActionRequest, TreeId};
use egui::{Context, Event, Id, Key, Memory, RawInput, Ui, ViewportId, ViewportIdMap};

#[derive(Default)]
struct MenuInput {
    viewports: ViewportIdMap<ViewportInput>,
}

#[derive(Default)]
struct ViewportInput {
    pass: u64,
    menus: Vec<Menu>,
    opening: Option<(Id, u64)>,
    routed: Vec<(Id, Intent)>,
    opening_keys: Vec<MenuKey>,
    focus_at_end: Option<Id>,
    focus_before_ui: Option<Id>,
    focus_owner: Option<Id>,
}

struct Menu {
    id: Id,
    rows: Vec<Id>,
    kind: MenuKind,
}

#[derive(Clone, Copy)]
pub(super) enum Intent {
    Focus(Id),
    Activate(Id),
    Dismiss(Id),
    Back(Id),
}

impl Intent {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Activate(_) | Self::Dismiss(_) | Self::Back(_))
    }
}

pub(super) struct RoutedInput {
    pub(super) intent: Intent,
    pub(super) retains_focus: bool,
}

pub(super) fn expect_tab_list(ui: &Ui, id: Id) {
    let frame = ui.ctx().cumulative_frame_nr();
    with_input(ui, |input| input.opening = Some((id, frame)));
}

pub(super) fn take_opening_keys(ui: &Ui, id: Id) -> (Vec<MenuKey>, bool) {
    let focused = ui.memory(Memory::focused);
    with_input(ui, |input| {
        if input.opening.is_none_or(|(owner, _)| owner != id) {
            return (Vec::new(), focused == Some(id));
        }
        input.opening = None;
        (
            std::mem::take(&mut input.opening_keys),
            input.focus_owner == Some(id) && input.focus_before_ui == focused,
        )
    })
}

pub(super) enum MenuKind {
    TabList,
    Product { anchor: Id, submenus: Vec<Id> },
}

pub(super) fn register(
    ui: &Ui,
    id: Id,
    rows: impl IntoIterator<Item = Id>,
    kind: MenuKind,
) -> Option<RoutedInput> {
    let focused = ui.memory(Memory::focused);
    with_input(ui, |input| {
        if !input.menus.iter().any(|menu| menu.id == id) {
            input.menus.push(Menu {
                id,
                rows: rows.into_iter().collect(),
                kind,
            });
        }
        let index = input.routed.iter().position(|(menu, _)| *menu == id)?;
        let intent = input.routed.remove(index).1;
        Some(RoutedInput {
            intent,
            // The application may explicitly move focus while building its UI.
            // The menu still applies its semantic input without reclaiming focus.
            retains_focus: input.focus_owner == Some(id) && input.focus_before_ui == focused,
        })
    })
}

fn with_input<T>(ui: &Ui, read: impl FnOnce(&mut ViewportInput) -> T) -> T {
    let context = ui.ctx();
    let pass = context.cumulative_pass_nr();
    let plugin = context.plugin_or_default::<MenuInput>();
    let mut plugin = plugin.lock();
    let input = plugin.viewports.entry(context.viewport_id()).or_default();
    if input.pass != pass {
        *input = ViewportInput {
            pass,
            ..Default::default()
        };
    }
    read(input)
}

impl egui::plugin::Plugin for MenuInput {
    fn debug_name(&self) -> &'static str {
        "egui_dockspace::menu_input"
    }

    fn input_hook(&mut self, context: &Context, raw: &mut RawInput) {
        let viewport = raw.viewport_id;
        let Some(input) = self.viewports.get_mut(&viewport) else {
            return;
        };
        let pass = context.cumulative_pass_nr_for(viewport);
        let previous = std::mem::replace(
            input,
            ViewportInput {
                pass,
                ..Default::default()
            },
        );
        if previous.pass.checked_add(1) != Some(pass)
            || !raw.focused
            || raw
                .events
                .iter()
                .any(|event| matches!(event, Event::WindowFocused(false)))
        {
            return;
        }
        let initial_focus = if context.viewport_id() == viewport {
            context.memory(Memory::focused)
        } else {
            previous.focus_at_end
        };
        let frame = context.cumulative_frame_nr_for(viewport);
        input.opening = previous.opening.filter(|&(id, started)| {
            initial_focus == Some(id) && started <= frame && frame - started <= 1
        });
        if input.opening.is_none()
            && !raw
                .events
                .iter()
                .any(|event| accesskit_focus_target(event).is_some())
        {
            return;
        }
        let initial = initial_focus.and_then(|initial| {
            previous
                .menus
                .iter()
                .find(|menu| menu.owns(initial))
                .map(|menu| (menu, initial))
        });
        let mut cursor = initial;
        let mut opening_cursor = input.opening.map(|(id, _)| id);
        let mut focus_owner = initial.map(|(menu, _)| menu.id).or(opening_cursor);
        let mut consumed_keys = Vec::new();
        let mut menu_focus_events = Vec::new();
        let mut app_key_before_menu_focus = false;
        let mut defer_menu_focus = false;
        for (slot, event) in raw.events.iter().enumerate() {
            if let Some(target) = accesskit_focus_target(event) {
                let target_menu = previous.menus.iter().find_map(|menu| {
                    menu.targets()
                        .find(|id| id.accesskit_id() == target)
                        .map(|id| (menu, id))
                });
                if let Some((menu, row)) = target_menu {
                    if input
                        .routed
                        .iter()
                        .any(|(id, intent)| *id == menu.id && intent.is_terminal())
                    {
                        consumed_keys.push(slot);
                        continue;
                    }
                    menu_focus_events.push(slot);
                    defer_menu_focus = app_key_before_menu_focus;
                    cursor = target_menu;
                    opening_cursor = None;
                    focus_owner = Some(menu.id);
                    set_intent(&mut input.routed, menu.id, Intent::Focus(row));
                } else if context.viewport_for(viewport, |viewport| {
                    viewport.prev_pass.widgets.layers().any(|(_, widgets)| {
                        widgets.iter().any(|widget| {
                            widget.enabled
                                && widget.sense.is_focusable()
                                && widget.id.accesskit_id() == target
                        })
                    })
                }) {
                    cursor = None;
                    opening_cursor = None;
                    focus_owner = None;
                    defer_menu_focus = false;
                }
                continue;
            }
            if matches!(
                event,
                Event::PointerButton { pressed: true, .. }
                    | Event::Key {
                        key: Key::Tab,
                        pressed: true,
                        ..
                    }
            ) {
                cursor = None;
                opening_cursor = None;
                focus_owner = None;
                defer_menu_focus = true;
            }
            if opening_cursor.is_some() {
                if let Some(key) = MenuKey::from_event(event).filter(|key| key.is_tab_list()) {
                    input.opening_keys.push(key);
                    consumed_keys.push(slot);
                    if key.is_terminal() {
                        opening_cursor = None;
                    }
                } else if matches!(event, Event::Key { pressed: true, .. }) {
                    opening_cursor = None;
                }
                continue;
            }
            let Some((menu, row)) = cursor else {
                app_key_before_menu_focus |= matches!(
                    event,
                    Event::Key { pressed: true, .. } | Event::Text(_) | Event::Ime(_)
                );
                continue;
            };
            let Some(key) = MenuKey::from_event(event) else {
                // An application shortcut can move focus. Later keys must remain
                // available to egui until the callback chooses their receiver.
                if matches!(event, Event::Key { pressed: true, .. }) {
                    cursor = None;
                }
                continue;
            };
            let Some(intent) = menu.intent(row, key) else {
                continue;
            };
            consumed_keys.push(slot);
            if intent.is_terminal() {
                cursor = None;
            } else if let Intent::Focus(next) = intent {
                cursor = Some((menu, next));
            }
            set_intent(&mut input.routed, menu.id, intent);
        }
        input.focus_owner = focus_owner;
        let mut slot = 0;
        raw.events.retain(|_| {
            let keep = consumed_keys.binary_search(&slot).is_err()
                && (!defer_menu_focus || menu_focus_events.binary_search(&slot).is_err());
            slot += 1;
            keep
        });
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let context = ui.ctx();
        if let Some(input) = self.viewports.get_mut(&context.viewport_id()) {
            input.focus_before_ui = context.memory(Memory::focused);
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let context = ui.ctx();
        if let Some(input) = self.viewports.get_mut(&context.viewport_id()) {
            input.focus_at_end = context.memory(Memory::focused);
        }
    }

    fn output_hook(&mut self, context: &Context, _: &mut egui::FullOutput) {
        self.viewports.retain(|&viewport, _| {
            viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(viewport) != 0
        });
    }
}

impl Menu {
    fn anchor(&self) -> Option<Id> {
        match self.kind {
            MenuKind::TabList => None,
            MenuKind::Product { anchor, .. } => Some(anchor),
        }
    }

    fn targets(&self) -> impl Iterator<Item = Id> + '_ {
        self.rows.iter().copied().chain(self.anchor())
    }

    fn owns(&self, id: Id) -> bool {
        self.targets().any(|target| target == id)
    }

    fn intent(&self, target: Id, key: MenuKey) -> Option<Intent> {
        let row = self.rows.iter().position(|&row| row == target);
        let last = self.rows.len().checked_sub(1);
        match key {
            MenuKey::Navigate(direction) => {
                let last = last?;
                let next = match self.kind {
                    MenuKind::TabList => direction.destination(row?, last),
                    MenuKind::Product { .. } => direction.wrapping_destination(row, last),
                };
                Some(Intent::Focus(self.rows[next]))
            }
            MenuKey::Activate => row.map(|_| Intent::Activate(target)),
            MenuKey::Dismiss => Some(Intent::Dismiss(target)),
            MenuKey::Back => self.anchor().map(Intent::Back),
            MenuKey::EnterSubmenu => match &self.kind {
                MenuKind::Product { submenus, .. } if submenus.contains(&target) => {
                    Some(Intent::Activate(target))
                }
                _ => None,
            },
        }
    }
}

pub(super) fn accesskit_focus_target(event: &Event) -> Option<egui::accesskit::NodeId> {
    match event {
        Event::AccessKitActionRequest(ActionRequest {
            action: Action::Focus,
            target_node,
            target_tree: TreeId::ROOT,
            data: None,
        }) => Some(*target_node),
        _ => None,
    }
}

fn set_intent(routed: &mut Vec<(Id, Intent)>, menu: Id, intent: Intent) {
    if let Some((_, previous)) = routed.iter_mut().find(|(id, _)| *id == menu) {
        if !previous.is_terminal() {
            *previous = intent;
        }
    } else {
        routed.push((menu, intent));
    }
}

#[derive(Clone, Copy)]
pub(super) enum MenuKey {
    Navigate(Navigation),
    Activate,
    Dismiss,
    Back,
    EnterSubmenu,
}

#[derive(Clone, Copy)]
pub(super) enum Navigation {
    Previous,
    Next,
    First,
    Last,
}

impl MenuKey {
    pub(super) fn is_terminal(self) -> bool {
        matches!(self, Self::Activate | Self::Dismiss)
    }

    pub(super) fn is_tab_list(self) -> bool {
        !matches!(self, Self::Back | Self::EnterSubmenu)
    }

    pub(super) fn from_event(event: &Event) -> Option<Self> {
        let Event::Key {
            key,
            modifiers,
            pressed: true,
            ..
        } = event
        else {
            return None;
        };
        if !modifiers.matches_logically(egui::Modifiers::NONE) {
            return None;
        }
        Some(match key {
            Key::ArrowUp => Self::Navigate(Navigation::Previous),
            Key::ArrowDown => Self::Navigate(Navigation::Next),
            Key::Home => Self::Navigate(Navigation::First),
            Key::End => Self::Navigate(Navigation::Last),
            Key::Enter | Key::Space => Self::Activate,
            Key::Escape => Self::Dismiss,
            Key::ArrowLeft => Self::Back,
            Key::ArrowRight => Self::EnterSubmenu,
            _ => return None,
        })
    }
}

impl Navigation {
    pub(super) fn destination(self, current: usize, last: usize) -> usize {
        match self {
            Self::Previous => current.saturating_sub(1),
            Self::Next => current.saturating_add(1).min(last),
            Self::First => 0,
            Self::Last => last,
        }
    }

    pub(super) fn wrapping_destination(self, current: Option<usize>, last: usize) -> usize {
        match self {
            Self::Previous => current.and_then(|row| row.checked_sub(1)).unwrap_or(last),
            Self::Next => current.map_or(0, |row| if row < last { row + 1 } else { 0 }),
            Self::First => 0,
            Self::Last => last,
        }
    }
}
