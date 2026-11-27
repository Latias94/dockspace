//! Application-owned pane rendering contract.

use dockspace::model::{ItemId, RootId};
use egui::scroll_area::ScrollAreaOutput;
use egui::{Context, Id, Response, Ui, WidgetText};

use crate::tab::{Tab, TabBar, TabBarId, TabBarLayout, TabBarSlot, TabClose, TabVisuals};
use crate::{
    DockMenu, DockMenuAction, DockMenuButton, DragPreview, DropGuide, DropPreview, PaneSizing,
    Splitter, TabKey,
};

/// Ordinary application-content registrations produced by one pane callback.
///
/// Return this value directly from [`PaneView::ui`]; no per-item ID cache or
/// follow-up callback is required. An empty response declares no content, so
/// unknown widgets continue to block docking hover.
///
/// ```
/// # use egui_dockspace::PaneResponse;
/// # fn editor(ui: &mut egui::Ui, text: &mut String) -> PaneResponse {
/// let response = ui.text_edit_multiline(text);
/// PaneResponse::from(&response)
/// # }
/// ```
#[derive(Debug, Default)]
pub struct PaneResponse {
    first: Option<Id>,
    additional: Vec<Id>,
    internal_containers: Vec<Id>,
}

impl PaneResponse {
    /// Declares an ordinary content response created during this callback.
    ///
    /// Do not declare popup, modal, or foreign-overlay controls. The adapter
    /// still checks current-pass registration, layer, enablement and bounds;
    /// a declaration never changes a widget's ordinary egui input handling.
    /// See [`Self::with_content_ids`] for the registration requirements.
    #[must_use]
    pub fn with_content(self, response: &Response) -> Self {
        self.with_content_ids([response.id])
    }

    /// Declares a scroll area's own current-pass input registrations.
    ///
    /// Call after [`egui::ScrollArea::show`] with the pane's Ui and its output.
    /// Includes the background drag response and either axis's scrollbar only
    /// when egui registered them in this pass. The usual
    /// [`Self::with_content_ids`] eligibility checks still apply.
    ///
    /// Declare controls inside the scroll area separately; this method does not
    /// inspect `output.inner` or register arbitrary descendants.
    ///
    /// ```
    /// # use egui_dockspace::PaneResponse;
    /// # fn document(ui: &mut egui::Ui) -> PaneResponse {
    /// let scroll = egui::ScrollArea::both().show(ui, |ui| ui.button("Action"));
    /// PaneResponse::default()
    ///     .with_scroll_area(ui, &scroll)
    ///     .with_content(&scroll.inner)
    /// # }
    /// ```
    #[must_use]
    pub fn with_scroll_area<R>(mut self, ui: &Ui, output: &ScrollAreaOutput<R>) -> Self {
        let mut controls = [None; 3];
        let mut containers = Vec::new();
        ui.ctx().viewport(|viewport| {
            controls = [
                output.id.with("area"),
                output.id.with(0_usize),
                output.id.with(1_usize),
            ]
            .map(|id| viewport.this_pass.widgets.get(id).map(|_| id));
            // `ScrollArea::show` wraps `add_contents` in a `UiKind::ScrollArea`
            // child `Ui` and an internal `Frame::NONE` whose `allocate_space`
            // `Sense::hover()` widget is inserted after inner controls and
            // discarded by `.inner`. Track those non-interactive scroll wrappers
            // separately from public `content_widgets()` so native hover clearance
            // over declared content is not blocked by egui's own scroll container,
            // while undeclared interactive controls inside still block.
            let layer = ui.layer_id();
            let parent = ui.unique_id();
            let matches_scroll_content = |widget: &egui::WidgetRect| {
                !widget.sense.interactive()
                    && (widget.rect.size() - output.content_size).length_sq() <= 1.0
                    && output
                        .inner_rect
                        .expand(1.0)
                        .contains_rect(widget.interact_rect)
            };
            let mut scroll_containers = Vec::new();
            for widget in viewport.this_pass.widgets.get_layer(layer) {
                if widget.parent_id == parent && matches_scroll_content(widget) {
                    scroll_containers.push((widget.id, widget.rect));
                    containers.push(widget.id);
                }
            }
            for (container_id, container_rect) in scroll_containers {
                let mut frame_wrapper_found = false;
                let mut children = Vec::new();
                for widget in viewport.this_pass.widgets.get_layer(layer) {
                    if widget.parent_id == container_id && !widget.sense.interactive() {
                        if matches_scroll_content(widget) {
                            frame_wrapper_found = true;
                        }
                        if container_rect.expand(1.0).contains_rect(widget.rect) {
                            children.push(widget.id);
                        }
                    }
                }
                if frame_wrapper_found {
                    containers.extend(children);
                }
            }
        });
        self.internal_containers.extend(containers);
        self.with_content_ids(controls.into_iter().flatten())
    }

    /// Declares the exact IDs of additional ordinary content registrations.
    ///
    /// Use this for composite controls whose child responses are created inside
    /// nested egui callbacks. Include only registrations from this pane's
    /// current callback, including any scrolling controls and hover-only content.
    ///
    /// The adapter accepts enabled registrations on the pane's own layer. Their
    /// already-clipped interaction rectangles must be finite, positive, and fully
    /// contained within the pane's valid content bounds. Hover-only registrations
    /// keep their original sense and cannot clear an overlapping unknown
    /// interactive widget. Popup, modal, and foreign-overlay controls must not
    /// be declared, even when their geometry fits.
    #[must_use]
    pub fn with_content_ids(mut self, ids: impl IntoIterator<Item = Id>) -> Self {
        let mut ids = ids.into_iter();
        if self.first.is_none() {
            self.first = ids.next();
        }
        self.additional.extend(ids);
        self
    }

    /// Returns the ordinary content IDs declared by this callback.
    #[must_use]
    pub fn content_widgets(&self) -> impl Iterator<Item = Id> + '_ {
        self.first
            .into_iter()
            .chain(self.additional.iter().copied())
    }

    pub(crate) fn internal_container_widgets(&self) -> impl Iterator<Item = Id> + '_ {
        self.internal_containers.iter().copied()
    }
}

impl From<&Response> for PaneResponse {
    fn from(response: &Response) -> Self {
        Self::default().with_content(response)
    }
}

/// Exact adapter observation of one application's pane focus target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFocusState {
    /// The application cannot currently prove whether this pane owns focus.
    Unknown,
    /// The pane's registered focus target currently owns focus.
    Focused,
    /// The application authoritatively reports that this pane does not own focus.
    Unfocused,
}

/// Resolves and renders application-owned panes by stable item identity.
///
/// The adapter borrows this object while processing a frame. Implementors
/// retain ownership of pane state; the docking workspace stores stable
/// [`ItemId`] content identities and layout metadata.
///
/// Rendering and customization run synchronously within the current egui pass.
/// Command delivery follows [`Self::on_menu_action`]. Start background
/// work in application-owned tasks, apply completed results on the UI thread,
/// and request a repaint. Do not wait for I/O or retain a [`Ui`] across tasks.
/// egui may repeat or discard a pass; application state is not rolled back with
/// the docking frame. Queue commands from callbacks and apply them after the
/// docking call succeeds, accounting for repeated invocations.
///
/// Boxed providers, including borrowed and non-`Send` trait objects, delegate
/// every callback to their inner provider. Native hosts may add their own
/// threading and lifetime requirements.
pub trait PaneView {
    /// Returns the current title for `item`.
    ///
    /// Explicit rich-text and layout-job colors are preserved in tabs, overflow
    /// rows and drag previews unless the corresponding docking text color or
    /// the local egui `override_text_color` is set. Docking overrides take
    /// precedence over the egui override. Uncolored text follows the current
    /// interaction state; disabled UI still applies egui's standard tint.
    ///
    /// Returning `None` reports that the application registry has no entry for
    /// the item. The adapter can then render a recoverable missing-pane state.
    /// The single-surface owners retain each measured title and its text layout
    /// for painting in that same pass. A later pass reads the title again.
    fn title(&self, item: ItemId) -> Option<WidgetText>;

    /// Returns the display title for a direct layout tab.
    ///
    /// The default comes from the layout model. This query supplies visible
    /// tabs, overflow rows, drag previews and accessibility. The key is local
    /// to the current workspace; use a model name for persistent naming.
    fn layout_tab_title(&self, _key: TabKey, default_title: &str) -> WidgetText {
        default_title.into()
    }

    /// Measures tab content before docking padding and the built-in close button.
    ///
    /// The default is the title width. Reserve extra width here when `tab_ui`
    /// adds icons, badges or buttons. Return a finite, non-negative width in
    /// egui points for either an application or a layout tab. This query may run
    /// before painting or for future drop geometry. Title metadata still supplies
    /// hover text, accessibility, overflow rows and drag previews.
    fn tab_content_width(&self, _key: TabKey, _ui: &Ui, title: &egui::Galley) -> f32 {
        title.size().x
    }

    /// Paints one tab, optionally adding application widgets to its content area.
    ///
    /// Call [`Tab::paint`] for defaults, or use its background and title helpers
    /// separately. The Ui starts at `tab.content_rect`, with a clip covering
    /// only the visible tab. The adapter retains the tab response,
    /// close control, keyboard handling, focus cues and accessibility.
    ///
    /// Render synchronously and respect the supplied clip. This method is
    /// skipped for missing panes and a dragged source gap.
    /// Application widgets remain ordinary egui widgets; no [`PaneResponse`]
    /// registration is needed or accepted for tab chrome.
    /// Return the final appearance so built-in focus and close cues follow it.
    /// `tab.key` identifies the direct tab; `tab.item()` is present only for
    /// real application content. Layout tabs use [`Self::layout_tab_title`].
    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        tab.paint(ui)
    }

    /// Paints a visible close icon without taking ownership of its interaction.
    fn tab_close_ui(&mut self, ui: &Ui, close: &TabClose<'_>) {
        close.paint(ui);
    }

    /// Paints a divider or intersection into its clipped allocation.
    ///
    /// Call [`Splitter::paint`] for the default appearance. The adapter owns
    /// hit testing, cursor, focus, double-click, keyboard and accessibility.
    /// While resizing, `splitter.dragging` reflects the core gesture owner.
    fn splitter_ui(&mut self, painter: &egui::Painter, splitter: &Splitter) {
        splitter.paint(painter);
    }

    /// Paints a layout, tab-group or tab-list menu button.
    ///
    /// Call [`DockMenuButton::paint`] for defaults, or replace the background
    /// and icon within the supplied clipped Ui. The descriptor includes the
    /// existing response and current expanded state. The adapter retains input,
    /// popup placement, the keyboard focus outline and accessibility. Do not
    /// register another widget or change focus or popup state here.
    fn menu_button_ui(&mut self, ui: &Ui, button: &DockMenuButton<'_>) {
        button.paint(ui);
    }

    /// Edits the title and ordered rows of an open presentation menu page.
    ///
    /// Retain, reorder or relabel the supplied rows, or insert a
    /// [`crate::DockMenuItem::custom`] row. Built-in commands retain their
    /// current core availability. The adapter handles input, focus and
    /// accessibility for all rows. This query can repeat during a run.
    fn customize_menu(&self, _menu: &mut DockMenu) {}

    /// Receives the selected application row after a successful docking update.
    ///
    /// A menu activation is delivered once per original egui run, including
    /// retried passes. Queue workspace mutations for the application to apply
    /// outside this callback. For `show_single_surface`, delivery happens
    /// during the enclosing egui pass after this dockspace has committed.
    /// Native hosts call under the pane registry lock and wake the root update
    /// for queued application work. Queue commands for later rather than
    /// re-entering the host from here.
    fn on_menu_action(&mut self, _action: DockMenuAction) {}

    /// Paints the source's drag ghost without adding interactive widgets.
    ///
    /// The destination is customized separately with [`Self::drop_preview_ui`].
    fn drag_preview_ui(&mut self, ui: &Ui, preview: &DragPreview) {
        preview.paint(ui);
    }

    /// Paints one directional docking-guide button without registering input.
    ///
    /// Use [`DropGuide::paint`] for defaults, customize its appearance, or draw
    /// a replacement within its rectangle. Its stable ID can key egui color or
    /// opacity animations. Core retains hit testing, eligibility and the active
    /// destination; the adapter retains accessibility. Do not create widgets,
    /// change layout or paint outside the supplied clip and layer.
    fn drop_guide_ui(&mut self, ui: &Ui, guide: &DropGuide) {
        guide.paint(ui);
    }

    /// Paints the resulting docking area; target annotations are opt-in.
    ///
    /// Core supplies the resolved target; do not infer it from overlap with the
    /// future area. The default only calls [`DropPreview::paint`]. Applications
    /// may add an outline with `paint_target`, an icon through the painter, or
    /// their own localized text through `paint_label`. The callback must not
    /// add interactive widgets or change layout.
    fn drop_preview_ui(&mut self, ui: &Ui, preview: &DropPreview) {
        preview.paint(ui);
    }

    /// Measures fixed and scrolling application slots in one group's tab bar.
    ///
    /// Return finite, non-negative widths. Defaults reserve no space and do
    /// not invoke `tab_bar_ui`. Use egui font/spacing measurements here rather
    /// than a previous paint pass; core may shrink fixed slot allocations.
    fn tab_bar_layout(&self, _bar: TabBarId, _ui: &Ui) -> TabBarLayout {
        TabBarLayout::default()
    }

    /// Renders one visible, core-allocated application tab-bar slot.
    ///
    /// All slots use a left-to-right, vertically centered Ui. It is clipped to
    /// the actual allocation and disabled when tab-bar interaction is disabled.
    /// Widgets own their input and cannot initiate a docking gesture. This uses
    /// the same synchronous rendering contract as [`Self::tab_ui`]. Queue
    /// requested workspace edits in application state and apply them after the
    /// docking call returns.
    fn tab_bar_ui(&mut self, _ui: &mut Ui, _bar: TabBar<'_>, _slot: TabBarSlot) {}

    /// Renders the application content associated with `item`.
    ///
    /// Only a currently selected pane whose identity and position match the
    /// retained content slot is rendered. A relocated or replaced pane is skipped
    /// until matching geometry is available. Bootstrap, stale, or changing
    /// layouts may render through a disabled child UI. [`Ui::is_enabled`] controls
    /// widget input; it does not prevent application state changes. Once pane
    /// geometry is current, ordinary content remains enabled even when docking
    /// interaction is unavailable.
    ///
    /// The supplied Ui is clipped to the pane's allocated content area. Content
    /// does not expand the docking allocation. Choose wrapping, an
    /// [`egui::ScrollArea`], or canvas navigation here when content can exceed
    /// that area; the adapter does not impose a scrolling container on panes.
    ///
    /// # Painting
    ///
    /// Paint synchronously on the supplied Ui's layer, respecting its opacity
    /// and clip, including when a drag temporarily hides the source pane. The
    /// standard [`Ui::dnd_drag_source`] preview layer is also supported for the
    /// actual dragged widget. Drawing on other layers, changing earlier shapes,
    /// or modifying an existing layer transform makes caller-owned presentation
    /// settlement unavailable. Do not defer drawing beyond this callback.
    /// Application paint callbacks must also honor the host renderer contract;
    /// shape comparison cannot inspect their rendering.
    ///
    /// # Content input
    ///
    /// Return ordinary content registrations through [`PaneResponse`]. Accepted
    /// registrations allow docking hover and containing-window activation while
    /// preserving normal widget input. Follow [`PaneResponse::with_content_ids`]
    /// for eligibility and overlay exclusions. The adapter reads registrations
    /// before the child UI is dropped; application state need not retain IDs.
    /// These declarations describe application ownership and do not infer an
    /// arbitrary widget tree or native receiver.
    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse;

    /// Renders optional placeholder content inside a root's empty central area.
    ///
    /// Called when `root`'s designated central container is empty (for example,
    /// `DockspaceNode::central_tabs([])`, or after all tabs in the central
    /// group have been closed or moved out). The supplied `Ui` is clipped to
    /// the central container's content bounds and follows the same synchronous,
    /// append-only painting contract as [`Self::ui`].
    ///
    /// Return ordinary content registrations through [`PaneResponse`] so
    /// docking hover and drop guides stay clear over the placeholder while
    /// controls inside it remain interactive. Queue workspace commands such as
    /// `open_item_current(..., DockPlacement::Center(DockAnchor::Central(root)))`
    /// and apply them after the docking update returns.
    ///
    /// The default renders nothing, leaving the workspace background fill.
    fn empty_central_ui(&mut self, _root: RootId, _ui: &mut Ui) -> PaneResponse {
        PaneResponse::default()
    }

    /// Returns the stable egui focus identity registered by `item`'s pane UI.
    ///
    /// The adapter uses this identity to request or surrender focus, and to keep
    /// an already focused selected pane alive while its UI is skipped. Merely
    /// selecting the item's tab, rendering the pane, or issuing a request never
    /// acknowledges a core pane-focus intent. In a caller-owned non-root
    /// viewport, terminal confirmation requires this identity to appear in the
    /// viewport's final AccessKit tree; otherwise the adapter reports the
    /// binding as unavailable rather than consulting root-viewport memory.
    fn focus_target(&self, _item: ItemId) -> Option<Id> {
        None
    }

    /// Opts this pane into keyboard delivery across a Dockspace focus transition.
    ///
    /// Use [`crate::pane_text_edit`] or wrap a custom editor in
    /// [`crate::with_pane_keyboard_input_at`]. The adapter delivers to the exact
    /// widget only while it remains focused and attached to this pane. A focus
    /// target of `None` with this opt-in means the target will be registered on
    /// the first draw. The default leaves keyboard events with egui.
    fn accepts_deferred_keyboard_input(&self, _item: ItemId) -> bool {
        false
    }

    /// Reports the pane's actual focus state after its UI has rendered.
    ///
    /// The default observes the exact [`Self::focus_target`] in egui memory.
    /// Implementors with a composite focus scope may override this method, but
    /// must return [`PaneFocusState::Unknown`] whenever that scope cannot be
    /// observed authoritatively.
    fn focus_state(&self, item: ItemId, context: &Context) -> PaneFocusState {
        let Some(target) = self.focus_target(item) else {
            return PaneFocusState::Unknown;
        };
        if context.input(|input| input.focused) && context.memory(|memory| memory.has_focus(target))
        {
            PaneFocusState::Focused
        } else {
            PaneFocusState::Unfocused
        }
    }

    /// Returns application content sizing in egui points, before docking chrome.
    ///
    /// This metadata is queried for every non-hidden tab, including inactive or
    /// unloaded content, so it must not depend on creating the pane UI. Hidden
    /// tabs are measured again when shown. Only the selected tab's sizing controls
    /// its current pane frame; other tabs supply exact potential drag layouts.
    ///
    /// Declare a useful content viewport, not the full document or scrollable
    /// content extent. Fixed sizes are equal minimum and maximum bounds. An
    /// explicit maximum can lower the style's minimum floor, so a fixed pane may
    /// be smaller than [`DockStyle::minimum_pane_size`](crate::style::DockStyle::minimum_pane_size).
    /// Preferred extents apply only to explicit preferred-size reset actions;
    /// they do not replace stored split weights during normal layout.
    ///
    /// Build and retain validated metadata with [`PaneSizing::builder`]. The
    /// default delegates minimum floors entirely to [`DockStyle`](crate::style::DockStyle)
    /// and imposes no maximum or preferred extent. Hosts still own available Ui
    /// and native-window bounds; undersized hosts follow the configured overflow
    /// policy, and content must respect its clip.
    fn sizing(&self, _item: ItemId) -> PaneSizing {
        PaneSizing::default()
    }
}

impl<T: PaneView + ?Sized> PaneView for Box<T> {
    fn title(&self, item: ItemId) -> Option<WidgetText> {
        (**self).title(item)
    }

    fn ui(&mut self, item: ItemId, ui: &mut Ui) -> PaneResponse {
        (**self).ui(item, ui)
    }

    fn empty_central_ui(&mut self, root: RootId, ui: &mut Ui) -> PaneResponse {
        (**self).empty_central_ui(root, ui)
    }

    fn layout_tab_title(&self, key: TabKey, default_title: &str) -> WidgetText {
        (**self).layout_tab_title(key, default_title)
    }

    fn tab_content_width(&self, key: TabKey, ui: &Ui, title: &egui::Galley) -> f32 {
        (**self).tab_content_width(key, ui, title)
    }

    fn tab_ui(&mut self, ui: &mut Ui, tab: &Tab<'_>) -> TabVisuals {
        (**self).tab_ui(ui, tab)
    }

    fn tab_close_ui(&mut self, ui: &Ui, close: &TabClose<'_>) {
        (**self).tab_close_ui(ui, close);
    }

    fn splitter_ui(&mut self, painter: &egui::Painter, splitter: &Splitter) {
        (**self).splitter_ui(painter, splitter);
    }

    fn menu_button_ui(&mut self, ui: &Ui, button: &DockMenuButton<'_>) {
        (**self).menu_button_ui(ui, button);
    }

    fn customize_menu(&self, menu: &mut DockMenu) {
        (**self).customize_menu(menu);
    }

    fn on_menu_action(&mut self, action: DockMenuAction) {
        (**self).on_menu_action(action);
    }

    fn drag_preview_ui(&mut self, ui: &Ui, preview: &DragPreview) {
        (**self).drag_preview_ui(ui, preview);
    }

    fn drop_guide_ui(&mut self, ui: &Ui, guide: &DropGuide) {
        (**self).drop_guide_ui(ui, guide);
    }

    fn drop_preview_ui(&mut self, ui: &Ui, preview: &DropPreview) {
        (**self).drop_preview_ui(ui, preview);
    }

    fn tab_bar_layout(&self, bar: TabBarId, ui: &Ui) -> TabBarLayout {
        (**self).tab_bar_layout(bar, ui)
    }

    fn tab_bar_ui(&mut self, ui: &mut Ui, bar: TabBar<'_>, slot: TabBarSlot) {
        (**self).tab_bar_ui(ui, bar, slot);
    }

    fn focus_target(&self, item: ItemId) -> Option<Id> {
        (**self).focus_target(item)
    }

    fn accepts_deferred_keyboard_input(&self, item: ItemId) -> bool {
        (**self).accepts_deferred_keyboard_input(item)
    }

    fn focus_state(&self, item: ItemId, context: &Context) -> PaneFocusState {
        (**self).focus_state(item, context)
    }

    fn sizing(&self, item: ItemId) -> PaneSizing {
        (**self).sizing(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPanes {
        locked: ItemId,
    }

    impl PaneView for TestPanes {
        fn title(&self, item: ItemId) -> Option<WidgetText> {
            (item != self.locked).then(|| "Pane".into())
        }

        fn ui(&mut self, _item: ItemId, _ui: &mut Ui) -> PaneResponse {
            PaneResponse::default()
        }
    }

    fn accepts_trait_object(_panes: &mut dyn PaneView) {}

    #[test]
    fn pane_view_is_object_safe_and_reports_missing_items() {
        let locked = ItemId::new(7);
        let mut panes = TestPanes { locked };

        accepts_trait_object(&mut panes);
        assert!(panes.title(locked).is_none());
        assert!(panes.title(ItemId::new(8)).is_some());
    }

    #[test]
    fn default_sizing_defers_to_adapter_style() {
        let panes = TestPanes {
            locked: ItemId::new(7),
        };

        assert_eq!(panes.sizing(ItemId::new(8)), PaneSizing::default());
    }

    #[test]
    fn missing_focus_provider_stays_unknown() {
        let panes = TestPanes {
            locked: ItemId::new(7),
        };

        assert_eq!(
            panes.focus_state(ItemId::new(8), &Context::default()),
            PaneFocusState::Unknown
        );
    }

    #[test]
    fn boxed_borrowed_provider_preserves_custom_focus_and_painting() {
        use std::{cell::Cell, rc::Rc};

        struct BorrowedPanes<'a> {
            title: &'a str,
            paints: Rc<Cell<usize>>,
            sizing: PaneSizing,
        }

        impl PaneView for BorrowedPanes<'_> {
            fn title(&self, _item: ItemId) -> Option<WidgetText> {
                Some(self.title.into())
            }

            fn ui(&mut self, _item: ItemId, ui: &mut Ui) -> PaneResponse {
                self.paints.set(self.paints.get() + 1);
                PaneResponse::from(&ui.label(self.title))
            }

            fn empty_central_ui(&mut self, _root: RootId, ui: &mut Ui) -> PaneResponse {
                self.paints.set(self.paints.get() + 10);
                PaneResponse::from(&ui.label("Empty central"))
            }

            fn focus_state(&self, _item: ItemId, _context: &Context) -> PaneFocusState {
                PaneFocusState::Focused
            }

            fn focus_target(&self, item: ItemId) -> Option<Id> {
                Some(Id::unique(("boxed-provider", item)))
            }

            fn sizing(&self, _item: ItemId) -> PaneSizing {
                self.sizing
            }
        }

        let title = String::from("Borrowed provider");
        let paints = Rc::new(Cell::new(0));
        let sizing = PaneSizing::builder()
            .min_width(80.0)
            .max_width(320.0)
            .preferred_width(160.0)
            .fixed_height(40.0)
            .build()
            .unwrap();
        let mut boxed: Box<dyn PaneView + '_> = Box::new(BorrowedPanes {
            title: &title,
            paints: Rc::clone(&paints),
            sizing,
        });
        // Coercing the box itself exercises its implementation, not deref lookup.
        let provider: &mut dyn PaneView = &mut boxed;
        let item = ItemId::new(8);
        let context = Context::default();
        assert_eq!(provider.title(item).unwrap().text(), title);
        assert_eq!(provider.sizing(item), sizing);
        assert_eq!(
            provider.focus_target(item),
            Some(Id::unique(("boxed-provider", item)))
        );
        assert_eq!(
            provider.focus_state(item, &context),
            PaneFocusState::Focused
        );
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let response = provider.ui(item, ui);
            assert_eq!(response.content_widgets().count(), 1);
            let empty = provider.empty_central_ui(RootId::new(1), ui);
            assert_eq!(empty.content_widgets().count(), 1);
        });
        output.textures_delta.clear();
        assert!(paints.get() >= 11);
    }
}
