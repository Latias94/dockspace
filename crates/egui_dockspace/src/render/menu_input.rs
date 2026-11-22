//! Route keys by event order before egui applies the packet's final focus.

use egui::accesskit::{Action, ActionRequest, NodeId, TreeId};
use egui::{Context, Event, Id, Key, Memory, RawInput, Ui, ViewportId, ViewportIdMap, WidgetRects};

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
    after_application: Option<AfterApplication>,
    delivery: Option<FocusedInput>,
    focus_at_end: Option<Id>,
    initial_focus: Option<Id>,
    initial_menu: Option<Id>,
    focus_before_ui: Option<Id>,
    focus_owner: Option<Id>,
}

struct AfterApplication {
    menu: Id,
    keys: Vec<Event>,
    release_slots: Vec<usize>,
}

struct FocusedInput {
    receiver: Option<Id>,
    keys: Vec<Event>,
}

struct Menu {
    id: Id,
    rows: Vec<Id>,
    kind: MenuKind,
}

#[derive(Clone, Copy)]
pub(super) enum Intent {
    Focus(Id),
    Click(Id),
    Activate(Id),
    Dismiss(Id),
    Back(Id),
}

impl Intent {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Click(_) | Self::Activate(_) | Self::Dismiss(_) | Self::Back(_)
        )
    }
}

pub(super) struct RoutedInput {
    pub(super) intent: Intent,
    pub(super) retains_focus: bool,
}

pub(super) fn awaits_application(ui: &Ui, menu: Id) -> bool {
    with_input(ui, |input| {
        input
            .after_application
            .as_ref()
            .is_some_and(|pending| pending.menu == menu)
    })
}

pub(super) fn initial_focus(ui: &Ui) -> Option<Id> {
    with_input(ui, |input| input.initial_focus)
}

pub(super) fn started_outside(ui: &Ui, menu: Id, owns_focus: impl FnOnce(Id) -> bool) -> bool {
    with_input(ui, |input| {
        input.initial_menu != Some(menu) && input.initial_focus.is_some_and(|id| !owns_focus(id))
    })
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
        input.initial_focus = initial_focus;
        input.focus_before_ui = initial_focus;
        let initial = initial_focus.and_then(|initial| {
            previous
                .menus
                .iter()
                .find(|menu| menu.owns(initial))
                .map(|menu| (menu, initial))
        });
        input.initial_menu = initial.map(|(menu, _)| menu.id);
        let delivering = previous.delivery.is_some_and(|delivery| {
            let current = initial_focus.is_some()
                && initial_focus == delivery.receiver
                && !raw.events.iter().any(changes_keyboard_owner);
            raw.events.splice(
                ..0,
                delivery
                    .keys
                    .into_iter()
                    .filter(|event| current || matches!(event, Event::Key { pressed: false, .. })),
            );
            current
        });
        let frame = context.cumulative_frame_nr_for(viewport);
        input.opening = previous.opening.filter(|&(id, started)| {
            initial_focus == Some(id) && started <= frame && frame - started <= 1
        });
        if !delivering
            && input.opening.is_none()
            && !raw.events.iter().any(|event| {
                accesskit_focus_target(event).is_some() || accesskit_click_target(event).is_some()
            })
        {
            return;
        }
        let mut cursor = initial;
        let mut opening_cursor = input.opening.map(|(id, _)| id);
        let mut focus_owner = initial.map(|(menu, _)| menu.id).or(opening_cursor);
        let mut consumed_keys = Vec::new();
        let mut menu_focus_events = Vec::new();
        let mut app_key_before_menu_focus = false;
        let mut defer_menu_focus = false;
        for (slot, event) in raw.events.iter().enumerate() {
            if accesskit_click_target(event).is_some() {
                let target = context.viewport_for(viewport, |previous| {
                    live_accesskit_click_target(event, &previous.prev_pass.widgets)
                });
                if let Some((menu, row)) = target.and_then(|target| {
                    previous
                        .menus
                        .iter()
                        .find_map(|menu| Some((menu, menu.row(target)?)))
                }) {
                    input.after_application = None;
                    set_intent(&mut input.routed, menu.id, Intent::Click(row));
                    if cursor.is_some_and(|(owner, _)| owner.id == menu.id) {
                        cursor = None;
                    }
                    if matches!(menu.kind, MenuKind::Product { .. }) {
                        focus_owner = Some(menu.id);
                    }
                    continue;
                }
                // Application code can change focus while handling the click.
                // Deliver its key suffix once, after the application runs.
                let owner = cursor
                    .take()
                    .map(|(menu, _)| menu.id)
                    .or_else(|| input.after_application.as_ref().map(|pending| pending.menu));
                if let Some(menu) = owner {
                    input.after_application = Some(AfterApplication {
                        menu,
                        keys: Vec::new(),
                        release_slots: Vec::new(),
                    });
                }
                opening_cursor = None;
                focus_owner = None;
                continue;
            }
            if let Some(target) = accesskit_focus_target(event) {
                let target_menu = previous.menus.iter().find_map(|menu| {
                    menu.targets()
                        .find(|id| id.accesskit_id() == target)
                        .map(|id| (menu, id))
                });
                if let Some((menu, row)) = target_menu {
                    input.after_application = None;
                    if input
                        .routed
                        .iter()
                        .any(|(id, intent)| *id == menu.id && intent.is_terminal())
                    {
                        consumed_keys.push(slot);
                        cursor = target_menu;
                        continue;
                    }
                    menu_focus_events.push(slot);
                    defer_menu_focus = app_key_before_menu_focus;
                    cursor = target_menu;
                    opening_cursor = None;
                    focus_owner = Some(menu.id);
                    input.focus_before_ui = Some(row);
                    set_intent(&mut input.routed, menu.id, Intent::Focus(row));
                } else if let Some(target) = context.viewport_for(viewport, |viewport| {
                    viewport
                        .prev_pass
                        .widgets
                        .layers()
                        .find_map(|(_, widgets)| {
                            widgets
                                .iter()
                                .find(|widget| {
                                    widget.enabled
                                        && widget.sense.is_focusable()
                                        && widget.id.accesskit_id() == target
                                })
                                .map(|widget| widget.id)
                        })
                }) {
                    cursor = None;
                    opening_cursor = None;
                    input.after_application = None;
                    focus_owner = None;
                    input.focus_before_ui = Some(target);
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
                input.after_application = None;
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
            if let Some(pending) = &mut input.after_application {
                if MenuKey::from_event(event).is_some() {
                    pending.keys.push(event.clone());
                    consumed_keys.push(slot);
                    continue;
                }
                if let Event::Key {
                    key,
                    pressed: false,
                    ..
                } = event
                    && pending.keys.iter().any(|held| {
                        matches!(held, Event::Key { key: held, pressed: true, .. } if held == key)
                    })
                {
                    pending.keys.push(event.clone());
                    pending.release_slots.push(slot);
                    continue;
                }
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
        let held_releases = input
            .after_application
            .as_ref()
            .map_or(&[][..], |pending| &pending.release_slots);
        let mut slot = 0;
        raw.events.retain(|_| {
            let keep = consumed_keys.binary_search(&slot).is_err()
                && held_releases.binary_search(&slot).is_err()
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
            input.finish(context, context.viewport_id());
        }
    }

    fn output_hook(&mut self, context: &Context, _: &mut egui::FullOutput) {
        let viewport = context.viewport_id();
        if let Some(input) = self.viewports.get_mut(&viewport)
            && input.pass.checked_add(1) == Some(context.cumulative_pass_nr_for(viewport))
        {
            input.finish(context, viewport);
        }
        self.viewports.retain(|&viewport, _| {
            viewport == ViewportId::ROOT || context.cumulative_pass_nr_for(viewport) != 0
        });
    }
}

impl ViewportInput {
    fn finish(&mut self, context: &Context, viewport: ViewportId) {
        self.focus_at_end = context.memory(Memory::focused);
        if let Some(pending) = self.after_application.take()
            && !pending.keys.is_empty()
        {
            self.delivery = Some(FocusedInput {
                receiver: self.focus_at_end,
                keys: pending.keys,
            });
            context.request_repaint_of(viewport);
        }
    }
}

fn changes_keyboard_owner(event: &Event) -> bool {
    accesskit_focus_target(event).is_some()
        || accesskit_click_target(event).is_some()
        || matches!(
            event,
            Event::PointerButton { pressed: true, .. }
                | Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    ..
                }
        )
}

impl Menu {
    fn row(&self, target: NodeId) -> Option<Id> {
        self.rows
            .iter()
            .copied()
            .find(|id| id.accesskit_id() == target)
    }

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

pub(super) fn accesskit_focus_target(event: &Event) -> Option<NodeId> {
    accesskit_target(event, Action::Focus)
}

fn accesskit_click_target(event: &Event) -> Option<NodeId> {
    accesskit_target(event, Action::Click)
}

pub(super) fn live_accesskit_click_target(event: &Event, widgets: &WidgetRects) -> Option<NodeId> {
    accesskit_click_target(event).filter(|target| {
        widgets.layers().any(|(_, widgets)| {
            widgets.iter().any(|widget| {
                widget.enabled
                    && widget.visible
                    && widget.sense.senses_click()
                    && widget.id.accesskit_id() == *target
            })
        })
    })
}

fn accesskit_target(event: &Event, wanted: Action) -> Option<NodeId> {
    match event {
        Event::AccessKitActionRequest(ActionRequest {
            action,
            target_node,
            target_tree: TreeId::ROOT,
            data: None,
        }) if *action == wanted => Some(*target_node),
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
