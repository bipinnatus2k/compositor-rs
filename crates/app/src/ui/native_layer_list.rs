//! The right panel as its own view entity: blend mode + opacity for the
//! active layer, the layer list (top-to-bottom), and the footer layer
//! actions — the port of Swift's `LayersPanel` + `NativeLayerList` +
//! `LayerAppearanceControls` + the footer menu family (ADR 0003 step C,
//! ticket 40). The window shell (`AppState`, the transitional root) keeps
//! the delete-ask flows, the panel openings, the status line and the
//! revision epoch; the panel reaches them through its
//! `app: WeakEntity<AppState>` seam until ticket 43 reshapes it.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::{
    App, AppContext, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, RenderImage, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, prelude::FluentBuilder as _, px, rgb,
};

use compositor_core::adjustment::AdjustmentKind;
use compositor_core::blend::LayerBlendMode;
use compositor_core::document::ImageLayer;
use compositor_core::effects::{LayerEffectKind, LayerEffectSelection};
use compositor_core::selection::SelectionMode;
use compositor_core::session::EditorSession;

use crate::state::{AppState, bgra_render_image, slider};
use crate::ui::{ClickHandler, Dropdown, MenuEntry, app_dispatch, text_label};

const THUMB: f32 = 30.0;

fn click_count(event: &ClickEvent) -> usize {
    match event {
        ClickEvent::Mouse(mouse) => mouse.up.click_count,
        _ => 0,
    }
}

/// Shift adds, Option subtracts, plain replaces — the thumbnail-click
/// selection cascade (NativeLayerList.swift:785-789), written once.
fn selection_mode(shift: bool, alt: bool) -> SelectionMode {
    if shift {
        SelectionMode::Add
    } else if alt {
        SelectionMode::Subtract
    } else {
        SelectionMode::Replace
    }
}

/// The footer mask button's disabled line (LayerMaskMenu.swift): the active
/// layer must be editable as a mask and carry none yet.
fn footer_mask_disabled(session: &EditorSession) -> bool {
    !session.can_edit_mask() || session.active_layer().is_some_and(|l| l.mask.is_some())
}

/// LayerMaskMenu.swift's help line: with a selection it becomes black.
fn footer_mask_tooltip(has_selection: bool) -> &'static str {
    if has_selection {
        "Add layer mask (the selection becomes black)"
    } else {
        "Add layer mask"
    }
}

/// A layer-row element's stable key (coding-guides "Stable identity"):
/// derived from the layer's UUID, never the mutable list index — a reorder
/// must not rebind a row's element state.
fn layer_key(part: &str, id: uuid::Uuid) -> SharedString {
    SharedString::from(format!("layer-{part}-{}", id))
}

/// The effect-row counterpart of `layer_key`: keyed by the layer's UUID and
/// the effect kind, never the row's position in the list.
fn effect_key(part: &str, id: uuid::Uuid, kind: LayerEffectKind) -> SharedString {
    SharedString::from(format!("effect-{part}-{:?}-{}", kind, id))
}

/// What an Option-drag off an effect row carries (the Swift pasteboard's
/// `com.compositor.layer-effect`, NativeLayerList.swift:46-47): the effect
/// to copy onto whichever layer row receives the drop.
#[derive(Clone, Copy)]
struct EffectDrag {
    layer_id: uuid::Uuid,
    kind: LayerEffectKind,
}

/// Swift only drags an effect with Option held, which GPUI's declarative
/// drag cannot gate at initiation — so a drag that did not start with
/// Option (and without Command) carries an invisible ghost, and nothing
/// answers its drop: the copy checks the modifiers again at drop time.
struct EffectDragGhost {
    label: SharedString,
    visible: bool,
}

impl Render for EffectDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div().when(self.visible, |chip| {
            chip.px_2()
                .py_0p5()
                .rounded_sm()
                .bg(gpui_kit::rgb(crate::ui::BG_PANEL))
                .border_1()
                .border_color(gpui_kit::rgb(crate::ui::BORDER_BRIGHT))
                .text_size(px(11.0))
                .text_color(gpui_kit::rgb(crate::ui::FG_TEXT))
                .child(self.label.clone())
        })
    }
}

/// Option without Command, the modifier gate every effect-drag decision
/// goes through (NativeLayerList.swift's `contains(.option),
/// !contains(.command)`).
fn option_drag_modifiers(modifiers: gpui_kit::Modifiers) -> bool {
    modifiers.alt && !modifiers.control && !modifiers.platform
}

/// What the trash acts on (`deleteLayerOrMask`, SelectionEdits.swift:
/// 71-75): a selected effect goes first, a targeted mask goes alone,
/// everything else deletes through the live-mask ask.
enum DeleteTarget {
    Effect,
    Mask,
    Layers,
}

fn delete_target(session: &compositor_core::session::EditorSession) -> DeleteTarget {
    if session.effect_selection.is_some() {
        DeleteTarget::Effect
    } else if session.is_mask_selected
        && session
            .active_layer()
            .map(|layer| layer.mask.is_some())
            .unwrap_or(false)
        && session.selected_layer_ids.len() <= 1
    {
        DeleteTarget::Mask
    } else {
        DeleteTarget::Layers
    }
}

/// The right panel as its own view entity (Swift `LayersPanel` — the
/// header's blend/opacity controls, the layer list and the footer menu
/// family; ADR 0003 step C, ticket 40). It observes the session so a
/// session notification re-renders the panel alone; the row/effect-row
/// RenderOnce components below are its rendering bodies (ADR 0002).
pub struct LayersPanelView {
    /// The active tab's session — the header readouts, the layer rows and
    /// the footer guards read it, and the panel's handlers mutate it
    /// directly (Swift's panel speaks to its session the same way).
    /// Re-pointed on tab changes by [`AppState::sync_session_observer`]
    /// (the per-tab rebuild lands with ticket 43).
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the delete-ask flows, the
    /// effects/adjustment panel openings and the touch/revision epoch stay
    /// there. No AppState dependencies beyond what the moved handlers
    /// already used.
    app: WeakEntity<AppState>,
    /// The layers-panel rename edit field (`renamingLayerID`'s widget half;
    /// the session flag itself lives in the core session since stage-D
    /// ticket 35). Built in [`Self::begin_rename`] — an input needs the
    /// window.
    rename_input: Option<Entity<InputState>>,
    /// The cached thumbnail textures for one layer row, keyed by layer and
    /// by whether the mask is the row's subject; values are (revision,
    /// image). The revision epoch is the app shell's (ticket 39's field
    /// audit) and is read through the seam as the cache key.
    thumbnails: HashMap<(uuid::Uuid, bool), (u64, Arc<RenderImage>)>,
    /// The active layer's opacity (Layers panel header,
    /// LayerAppearanceControls).
    opacity_slider: Entity<SliderState>,
    /// Subscriptions owned here: the header slider's writes and the rename
    /// input's commit events.
    subscriptions: Vec<Subscription>,
    /// The observe subscription on the active tab's session entity (ticket
    /// 40's item 1): a session-entity notify re-renders this panel alone.
    /// Re-attached whenever the active tab changes
    /// ([`Self::set_session`]); kept in a dedicated field — not
    /// `subscriptions` — so a tab switch drops the outgoing session's
    /// observation.
    session_observer: Option<(Entity<EditorSession>, Subscription)>,
}

impl LayersPanelView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        // LayerAppearanceControls' opacity slider: 0…100 %, whole steps —
        // the same seed `AppState::new` used when the slider lived there.
        let opacity_slider = cx.new(|_| slider(0.0, 100.0, 1.0, 100.0));
        let mut panel = LayersPanelView {
            session,
            app,
            rename_input: None,
            thumbnails: HashMap::new(),
            opacity_slider,
            subscriptions: Vec::new(),
            session_observer: None,
        };
        panel.subscribe_opacity_slider(cx);
        panel.attach_session_observer(cx);
        panel
    }

    /// The header slider's writes (`subscribe_sliders`' opacity slice moved
    /// with the slider, ticket 40): Change begins an opacity edit and
    /// tracks it, Release folds it closed; both bump the repaint epoch.
    fn subscribe_opacity_slider(&mut self, cx: &mut Context<Self>) {
        let opacity_slider = self.opacity_slider.clone();
        let subscription = cx.subscribe(
            &opacity_slider,
            |this, _slider, event: &SliderEvent, cx: &mut Context<Self>| {
                match event {
                    SliderEvent::Change(value) => {
                        this.session.update(cx, |session, _| {
                            session.begin_opacity_edit();
                            session.set_layer_opacity((value.start() / 100.0) as f64);
                        });
                    }
                    SliderEvent::Release(value) => {
                        this.session.update(cx, |session, _| {
                            session.set_layer_opacity((value.start() / 100.0) as f64);
                            session.finish_opacity_edit();
                        });
                    }
                }
                this.touch(cx);
                cx.notify();
            },
        );
        self.subscriptions.push(subscription);
    }

    /// Observes the session entity so its notifications re-render this
    /// panel alone (ticket 40's item 1). Drops the outgoing observation.
    fn attach_session_observer(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let subscription = cx.observe(&session, |_, _, cx| cx.notify());
        self.session_observer = Some((session, subscription));
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view; the per-tab rebuild itself lands with ticket
    /// 43): the observation follows, so the non-active tab is not observed.
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>, cx: &mut Context<Self>) {
        if self
            .session_observer
            .as_ref()
            .is_some_and(|(observed, _)| observed.entity_id() == session.entity_id())
        {
            return;
        }
        self.session = session;
        self.attach_session_observer(cx);
    }

    /// The panel's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// a panel interaction that changed the session asks the shell through
    /// the seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// The header slider entity, for `AppState::opacity_digit`'s Move-tool
    /// sync — the one write to it from outside this view.
    pub(crate) fn opacity_slider(&self) -> Entity<SliderState> {
        self.opacity_slider.clone()
    }

    /// Drops the rename input; the caller (`AppState::cancel_active`'s
    /// Escape chain) already cleared `renaming_layer_id` through the
    /// session.
    pub(crate) fn discard_rename_input(&mut self) {
        self.rename_input = None;
    }

    /// Puts the active layer's opacity into the header slider.
    fn sync_opacity_slider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(opacity) = self.session.read(cx).active_layer().map(|l| l.opacity) else {
            return;
        };
        let value = (opacity * 100.0) as f32;
        let slider = self.opacity_slider.clone();
        slider.update(cx, |state, cx| {
            state.set_value(
                gpui_kit::component::slider::SliderValue::Single(value),
                window,
                cx,
            )
        });
    }

    /// The layer-effect kinds (LayersPanel.swift's sparkles menu), disabled
    /// unless the active layer can take effects.
    fn effects_menu_entries(&self, cx: &App) -> Vec<MenuEntry> {
        let enabled = self.session.read(cx).can_edit_effects();
        LayerEffectKind::ALL
            .iter()
            .map(|kind| {
                let kind = *kind;
                // Adding an effect opens its editor panel — an app-side flow,
                // reached through the seam.
                MenuEntry::new(format!("{}…", kind.label()), move |state, window, cx| {
                    state.add_effect(kind, window, cx);
                })
                .disabled(!enabled)
            })
            .collect()
    }

    /// The 12 adjustment kinds (LayersPanel.swift's half-circle menu): one
    /// entry per kind; choosing one adds an adjustment layer. Entries use
    /// the kinds' raw labels, no ellipsis, like Swift's footer menu.
    fn adjustment_menu_entries(&self, cx: &App) -> Vec<MenuEntry> {
        let enabled = self.session.read(cx).can_edit_layers();
        AdjustmentKind::ALL
            .iter()
            .map(|kind| {
                let kind = *kind;
                // The session mutation closes over the panel's session; only
                // the repaint epoch goes through the seam.
                let session = self.session.clone();
                MenuEntry::new(kind.label(), move |state, _, cx| {
                    session.update(cx, |session, _| session.add_adjustment(kind));
                    state.touch(cx);
                })
                .disabled(!enabled)
            })
            .collect()
    }

    /// The 24 blend modes, grouped with separators like the original menu.
    fn blend_menu_entries(&self, cx: &App) -> Vec<MenuEntry> {
        let current = self
            .session
            .read(cx)
            .active_layer()
            .map(|layer| layer.blend_mode);
        let mut entries = Vec::new();
        for (group_index, group) in LayerBlendMode::GROUPS.iter().enumerate() {
            if group_index > 0 {
                entries.push(MenuEntry::separator());
            }
            for mode in group.iter() {
                let mode = *mode;
                // The session mutation closes over the panel's session; only
                // the repaint epoch goes through the seam.
                let session = self.session.clone();
                entries.push(
                    MenuEntry::new(mode.label(), move |state, _, cx| {
                        session.update(cx, |session, _| session.set_layer_blend_mode(mode));
                        state.touch(cx);
                    })
                    .checked(current == Some(mode)),
                );
            }
        }
        entries
    }

    /// The cached thumbnail texture for one layer row, keyed by layer and by
    /// whether the mask is the row's subject (the mask thumbnail swaps in).
    fn thumbnail_image(
        &mut self,
        id: uuid::Uuid,
        layer: &ImageLayer,
        cx: &App,
    ) -> Option<Arc<RenderImage>> {
        let mask_selected = self.session.read(cx).is_mask_selected && layer.mask.is_some();
        let revision = self
            .app
            .upgrade()
            .map(|app| app.read(cx).revision)
            .unwrap_or(0);
        if let Some((rev, image)) = self.thumbnails.get(&(id, mask_selected))
            && *rev == revision
        {
            return Some(image.clone());
        }
        let document = self.session.read(cx).document.as_ref()?;
        let canvas =
            compositor_core::geometry::CGSize::new(document.width as f64, document.height as f64);
        // CanvasThumbnail: the pixels (or the mask) sit on a canvas-shaped
        // frame, whatever the layer's own bounds.
        let thumbnail = if mask_selected {
            compositor_core::render::mask_canvas_thumbnail(
                layer.mask.as_ref()?,
                &layer.mask_transform(),
                canvas,
            )?
        } else {
            compositor_core::render::layer_canvas_thumbnail(layer, canvas)?
        };
        let image = bgra_render_image(&thumbnail);
        self.thumbnails
            .insert((id, mask_selected), (revision, image.clone()));
        Some(image)
    }

    // MARK: Rename

    pub(crate) fn begin_rename(
        &mut self,
        id: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .session
            .read(cx)
            .document
            .as_ref()
            .and_then(|d| d.layer(id))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let subscription = cx.subscribe(
            &input,
            |this: &mut Self, _input, event: &InputEvent, cx: &mut Context<Self>| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_rename(cx),
                _ => {}
            },
        );
        self.subscriptions.push(subscription);
        self.session
            .update(cx, |session, _| session.renaming_layer_id = Some(id));
        self.rename_input = Some(input);
        cx.notify();
    }

    pub(crate) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let id = self.session.read(cx).renaming_layer_id;
        if let (Some(id), Some(input)) = (id, self.rename_input.clone()) {
            let text = input.read(cx).value().to_string();
            self.session.update(cx, |session, _| {
                session.rename_layer(id, &text);
                session.renaming_layer_id = None;
            });
        }
        self.rename_input = None;
        self.touch(cx);
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, _| session.renaming_layer_id = None);
        self.rename_input = None;
        cx.notify();
    }
}

/// The panel-entity twin of `ui::app_on_click`: run a handler inside the
/// panel entity (the footer and thumbnail callbacks are the panel's own
/// now, ticket 40).
fn panel_on_click(
    panel: &WeakEntity<LayersPanelView>,
    handler: impl Fn(&mut LayersPanelView, &ClickEvent, &mut Window, &mut Context<LayersPanelView>)
    + 'static,
) -> ClickHandler {
    let panel = panel.clone();
    let handler = Rc::new(handler);
    Rc::new(move |event, window, cx| {
        let handler = handler.clone();
        panel
            .update(cx, move |panel, cx| handler(panel, event, window, cx))
            .ok();
    })
}

/// The right-click activation: the row becomes the active layer, with the
/// header's blend/opacity readouts following it, before any menu item runs.
fn row_activate(
    panel: &WeakEntity<LayersPanelView>,
    id: uuid::Uuid,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<gpui_kit::component::menu::PopupMenu>,
) {
    panel
        .update(cx, |panel, cx| {
            // Swift's menu activation runs selectLayerTarget -> selectLayer,
            // which moves the ACTIVE layer without touching selectedLayerIDs
            // (NativeLayerList.swift:763, EditorSession.swift:325-331): a right
            // click on a row already inside a multi-selection keeps that
            // selection, so the menu's Delete acts on all of it.
            panel.session.update(cx, |session, _| {
                if session.selected_layer_ids.contains(&id) {
                    session.active_layer_id = Some(id);
                    session.is_mask_selected = false;
                } else {
                    session.select_layer(Some(id));
                }
            });
            panel.sync_opacity_slider(window, cx);
            panel.touch(cx);
        })
        .ok();
}

/// The shared thumbnail cell: square, rounded, hairline border; the accent
/// ring marks the edit target, a disabled mask reads dimmed, and an image
/// fills the cell when one is on hand.
#[derive(IntoElement)]
pub struct LayerThumbnailButton {
    id: SharedString,
    is_target: bool,
    dimmed: bool,
    image: Option<Arc<RenderImage>>,
    on_click: ClickHandler,
}

impl LayerThumbnailButton {
    pub fn new(
        id: SharedString,
        is_target: bool,
        dimmed: bool,
        image: Option<Arc<RenderImage>>,
    ) -> Self {
        LayerThumbnailButton {
            id,
            is_target,
            dimmed,
            image,
            on_click: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_click(mut self, on_click: ClickHandler) -> Self {
        self.on_click = on_click;
        self
    }
}

impl RenderOnce for LayerThumbnailButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_click = self.on_click;
        gpui_kit::div()
            .id(self.id)
            .w(px(THUMB))
            .h(px(THUMB))
            .flex_shrink_0()
            .rounded_sm()
            .overflow_hidden()
            .border_1()
            .border_color(gpui_kit::rgb(crate::ui::BORDER_THUMB))
            .bg(rgb(crate::ui::FG_DISABLED))
            .when(self.is_target, |cell| {
                cell.border_2()
                    .border_color(gpui_kit::rgb(crate::ui::ACCENT))
            })
            .when(self.dimmed, |cell| cell.opacity(0.4))
            .when_some(self.image, |cell, image| {
                cell.child(gpui_kit::img(image).size_full())
            })
            .on_click(move |event, window, cx| on_click(event, window, cx))
    }
}

impl Render for LayersPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let mut panel = gpui_kit::div()
            .flex()
            .flex_col()
            .w(px(248.0))
            .flex_shrink_0()
            .bg(crate::ui::panel_bg())
            .border_l_1()
            .border_color(gpui_kit::black().opacity(0.4));

        // Header: title + blend mode + opacity.
        let session = self.session.read(cx);
        let blend_label = session
            .active_layer()
            .map(|layer| session.displayed_blend_mode(layer).label().to_string())
            .unwrap_or_else(|| "Normal".to_string());
        let opacity = session
            .active_layer()
            .map(|layer| layer.opacity)
            .unwrap_or(1.0);

        panel = panel.child(
            gpui_kit::div()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .py_1p5()
                .border_b_1()
                .border_color(gpui_kit::black().opacity(0.4))
                .child(
                    text_label("Layers", false)
                        .text_size(px(12.0))
                        .flex_shrink_0(),
                )
                .child(
                    gpui_kit::div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(text_label("Blend", true))
                        .child(
                            Dropdown::new("blend-mode", blend_label)
                                .with_width(px(130.0))
                                .with_entries(self.blend_menu_entries(cx))
                                .with_dispatch(app_dispatch(&self.app)),
                        )
                        .child(text_label("Opacity", true))
                        .child(gpui_kit::div().w(px(60.0)).child(
                            gpui_kit::component::slider::Slider::new(&self.opacity_slider),
                        ))
                        .child(text_label(format!("{:.0}%", opacity * 100.0), false)),
                ),
        );

        // The layer list, top to bottom. The rows are the hierarchy's
        // (`layerRows`, session.rs layer_rows: `hierarchy_entries(true, …)`):
        // `top_first` reverses siblings, so a document without groups
        // enumerates exactly like the former flat `layers.iter().rev()` pass,
        // and a collapsed group simply omits its children.
        let mut list = gpui_kit::div()
            .id("layer-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let (collapsed, document, active_id) = {
            let session = self.session.read(cx);
            (
                session.collapsed_group_ids.clone(),
                session.document.clone(),
                session.active_layer_id,
            )
        };
        if let Some(document) = document {
            for entry in document.hierarchy_entries(true, &collapsed) {
                let layer = entry.layer;
                let id = layer.id;
                let is_active = active_id == Some(id);
                let row = self.render_layer_row(layer, is_active, entry.depth, cx);
                list = list.child(row);
            }
        } else {
            list = list.child(gpui_kit::div().p_3().child(text_label("No document", true)));
        }
        panel = panel.child(list);

        // Footer buttons (Lucide glyphs; tooltips keep the wording). The set
        // mirrors Swift's footer (LayersPanel.swift:38-60): add, group, mask
        // menu, effects, adjustment menu, then the trash — no duplicate or
        // move buttons (those live in the menus and on the shortcuts).
        let fx_entries = self.effects_menu_entries(cx);
        let adjustment_entries = self.adjustment_menu_entries(cx);
        // The trash's label follows the target (deleteLayerOrMask,
        // SelectionEdits.swift:71-75) — the same decision its action makes.
        let delete_tooltip = {
            let session = self.session.read(cx);
            match delete_target(session) {
                DeleteTarget::Effect => "Delete selected effect",
                DeleteTarget::Mask => "Delete layer mask",
                DeleteTarget::Layers => {
                    if session.selected_layer_ids.len() > 1 {
                        "Delete selected layers"
                    } else {
                        "Delete selected layer"
                    }
                }
            }
        };
        // The footer's disabled lines are Swift's (LayersPanel.swift:38-60,
        // LayerMaskMenu.swift): edit guards per button, the mask retiring
        // once the active layer carries one.
        let footer = self.session.read(cx);
        let layers_enabled = footer.can_edit_layers();
        let fx_enabled = footer.can_edit_effects();
        let mask_disabled = footer_mask_disabled(footer);
        let mask_tooltip = footer_mask_tooltip(footer.selection().is_some());
        let trash_disabled = !layers_enabled || footer.active_layer().is_none();
        panel = panel.child(
            gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap_1()
                .px_1()
                .py_1()
                .border_t_1()
                .border_color(gpui_kit::black().opacity(0.4))
                .child(
                    FooterButton::new(
                        "layer-add",
                        IconName::Plus,
                        "New blank layer (⇧⌘N)",
                        "New blank layer",
                        !layers_enabled,
                    )
                    .with_on_click(panel_on_click(&this, |panel, _, _, cx| {
                        panel
                            .session
                            .update(cx, |session, _| session.add_blank_layer());
                        panel.touch(cx);
                    })),
                )
                .child(
                    FooterButton::new(
                        "layer-group",
                        IconName::FolderPlus,
                        "Group selected layers (⌘G)",
                        "New folder",
                        !layers_enabled,
                    )
                    .with_on_click(panel_on_click(&this, |panel, _, _, cx| {
                        // The footer groups the SELECTION (LayerGroups.swift:
                        // 119 groupSelectedLayers), not an empty folder above
                        // the active layer.
                        panel
                            .session
                            .update(cx, |session, _| session.group_selected_layers());
                        panel.touch(cx);
                    })),
                )
                .child(
                    FooterButton::new(
                        "layer-mask",
                        IconName::SquareDashedBottom,
                        mask_tooltip,
                        "Add layer mask",
                        mask_disabled,
                    )
                    .with_on_click(panel_on_click(&this, |panel, _, _, cx| {
                        // addMask (LayerMask.swift:233): all white, or with a
                        // selection, the selection black.
                        panel
                            .session
                            .update(cx, |session, _| session.add_mask_from_selection(true));
                        panel.touch(cx);
                    })),
                )
                // The effects menu (LayersPanel.swift's sparkles): one entry
                // per effect kind; choosing one adds it to the active layer
                // and opens its editor sheet.
                .child(
                    Button::new("layer-fx")
                        .icon(Icon::new(IconName::Sparkles))
                        .accessibility_label("Layer effects")
                        .ghost()
                        .small()
                        .tooltip("Layer effects: stroke and drop shadow")
                        .disabled(!fx_enabled)
                        // Same caret affordance as the ui::dropdown helper
                        // (gpui-component button.rs dropdown_caret).
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let app = self.app.clone();
                            move |menu, _, _| crate::ui::build_menu(&app, menu, &fx_entries)
                        }),
                )
                .child(
                    Button::new("layer-adjustment")
                        .icon(Icon::new(IconName::Contrast))
                        .accessibility_label("New adjustment layer")
                        .ghost()
                        .small()
                        .tooltip("New adjustment layer")
                        .disabled(!layers_enabled)
                        .dropdown_caret(true)
                        .dropdown_menu({
                            let app = self.app.clone();
                            move |menu, _, _| crate::ui::build_menu(&app, menu, &adjustment_entries)
                        }),
                )
                // deleteLayerOrMask (SelectionEdits.swift:71-75): a selected
                // effect goes first; a targeted mask goes alone; otherwise
                // every selected layer, through the live-mask ask.
                .child(
                    FooterButton::new(
                        "layer-del",
                        IconName::Trash,
                        delete_tooltip,
                        delete_tooltip,
                        trash_disabled,
                    )
                    .with_on_click(panel_on_click(
                        &this,
                        |panel, _, window, cx| {
                            let target = delete_target(panel.session.read(cx));
                            match target {
                                DeleteTarget::Effect => {
                                    panel
                                        .session
                                        .update(cx, |session, _| session.remove_selected_effect());
                                    panel.touch(cx);
                                }
                                DeleteTarget::Mask => {
                                    panel
                                        .session
                                        .update(cx, |session, _| session.delete_layer_mask());
                                    panel.touch(cx);
                                }
                                DeleteTarget::Layers => {
                                    panel
                                        .app
                                        .update(cx, |app, cx| app.request_delete_layer(window, cx))
                                        .ok();
                                }
                            }
                        },
                    )),
                ),
        );

        panel.into_any_element()
    }
}

impl LayersPanelView {
    fn render_layer_row(
        &mut self,
        layer: &ImageLayer,
        is_active: bool,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = layer.id;
        let visible = layer.is_visible;
        let is_group = layer.is_group;
        let has_mask = layer.mask.is_some();
        let mask_enabled = layer.mask.as_ref().map(|m| m.is_enabled).unwrap_or(false);
        let (mask_selected, collapsed) = {
            let session = self.session.read(cx);
            (
                session.is_mask_selected,
                session.collapsed_group_ids.contains(&id),
            )
        };
        let in_folder = layer.parent_id.is_some();
        let renaming = self.session.read(cx).renaming_layer_id == Some(id);
        // Swift indents one 24px step per hierarchy level, capped at 8, and
        // a clipped layer steps in another 24 — the same distance a folder
        // step takes, and the two add up (NativeLayerList.swift:625-631).
        let clipped_to = layer.mask_source_id;
        let indent = (depth.min(8) as f32) * 24.0 + if clipped_to.is_some() { 24.0 } else { 0.0 };
        let panel = cx.entity().downgrade();

        let mut row = gpui_kit::div()
            .id(layer_key("row", id))
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .mx_1()
            .rounded_sm()
            .on_click(cx.listener(move |this, event, window, cx| {
                let double = click_count(event) >= 2;
                if this.session.read(cx).renaming_layer_id == Some(id) {
                    return;
                }
                this.session
                    .update(cx, |session, _| session.select_layer(Some(id)));
                this.sync_opacity_slider(window, cx);
                if double {
                    // A row double-click renames — folders included
                    // (renameClickedLayer, NativeLayerList.swift:135-146);
                    // expansion belongs to the chevron alone.
                    this.begin_rename(id, window, cx);
                }
                this.touch(cx);
            }))
            .when(is_active, |row| {
                row.bg(gpui_kit::rgba(crate::ui::ACCENT_WASH))
            });

        // Indent: one 24px step per level; the folder's chevron closes the
        // indent region.
        row = row.child(gpui_kit::div().w(px(indent)).flex_shrink_0());
        if is_group {
            row = row.child(
                gpui_kit::div()
                    .id(layer_key("chevron", id))
                    // A generous hit area: an 18px box inside a 24px indent
                    // step is easy to miss (Swift widens footer hit areas
                    // for the same reason, NativeLayerList.swift:79-82).
                    .w(px(24.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session
                            .update(cx, |session, _| session.toggle_group_expansion(id));
                        this.touch(cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(14.0))
                        .text_color(gpui_kit::rgb(crate::ui::FG_DIM)),
                    ),
            );
        }

        // Eye (Lucide eye / eye-off, both in the bundled set).
        row = row.child(
            gpui_kit::div()
                .id(layer_key("eye", id))
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session
                        .update(cx, |session, _| session.toggle_layer_visibility(id));
                    this.touch(cx);
                }))
                .child(
                    Icon::new(if visible {
                        IconName::Eye
                    } else {
                        IconName::EyeOff
                    })
                    .size(px(14.0))
                    .text_color(if visible {
                        gpui_kit::rgb(crate::ui::FG_TEXT)
                    } else {
                        gpui_kit::rgb(crate::ui::FG_DISABLED)
                    }),
                ),
        );

        // Thumbnail. A plain click makes the layer the edit target
        // (selectImage); Cmd-click loads the layer's (or the mask's) pixels
        // as the selection, Shift adds and Option subtracts (MaskTracing).
        // The active edit target's thumbnail carries the accent border
        // (NativeLayerList.swift:712-715).
        let thumbnail = self.thumbnail_image(id, layer, cx);
        let layer_is_target = is_active && !(mask_selected && has_mask);
        // What a thumbnail double-click opens (NativeLayerList.swift:141-143).
        let holds_text = layer.text.is_some();
        let editable_adjustment = layer
            .adjustment
            .as_ref()
            .map(|a| a.kind.is_editable())
            .unwrap_or(false);
        row = row.child(
            LayerThumbnailButton::new(layer_key("thumb", id), layer_is_target, false, thumbnail)
                .with_on_click(panel_on_click(&panel, move |panel, event, window, cx| {
                    // A double-click on the thumbnail opens what the layer holds:
                    // its text, or an editable adjustment's editor (Invert falls
                    // through); anything else renames it.
                    if click_count(event) >= 2 {
                        panel
                            .session
                            .update(cx, |session, _| session.select_layer_target(id, false));
                        if holds_text {
                            panel
                                .session
                                .update(cx, |session, _| session.edit_active_text());
                        } else if editable_adjustment {
                            // Swift points adjustmentEditingID at the layer
                            // unconditionally (NativeLayerList.swift:143); a
                            // refused begin (busy) opens nothing, renames never.
                            // Opening the editor panel is an app-side flow.
                            panel
                                .app
                                .update(cx, |app, cx| {
                                    app.edit_layer_adjustment(id, cx);
                                })
                                .ok();
                        } else {
                            panel.begin_rename(id, window, cx);
                        }
                        panel.touch(cx);
                        cx.stop_propagation();
                        return;
                    }
                    let modifiers = event.modifiers();
                    if !modifiers.control && !modifiers.platform {
                        panel
                            .session
                            .update(cx, |session, _| session.select_layer_target(id, false));
                        panel.sync_opacity_slider(window, cx);
                        panel.touch(cx);
                        cx.stop_propagation();
                        return;
                    }
                    let mode = selection_mode(modifiers.shift, modifiers.alt);
                    if panel.session.read(cx).is_mask_selected {
                        panel
                            .session
                            .update(cx, |session, _| session.load_mask_selection(id, mode));
                    } else {
                        panel
                            .session
                            .update(cx, |session, _| session.load_layer_selection(id, mode));
                    }
                    panel.touch(cx);
                })),
        );

        // The mask thumbnail (the row's maskThumbnail): the mask placed on
        // its canvas frame at the layer thumbnail's size; a plain click makes
        // the mask the edit target, Cmd-click loads its coverage as the
        // selection. A disabled mask reads dimmed.
        if layer.mask.is_some() {
            let mask_image = self.mask_thumbnail_image(layer, cx);
            row = row.child(
                LayerThumbnailButton::new(
                    layer_key("mask-thumb", id),
                    is_active && mask_selected,
                    !mask_enabled,
                    mask_image,
                )
                .with_on_click(panel_on_click(
                    &panel,
                    move |panel, event, window, cx| {
                        let modifiers = event.modifiers();
                        if !modifiers.control && !modifiers.platform {
                            // selectMask (NativeLayerList.swift:793-796): the
                            // mask becomes the edit target; Shift also toggles
                            // it on/off.
                            panel.session.update(cx, |session, _| {
                                session.select_layer_target(id, true);
                                if modifiers.shift {
                                    session.toggle_layer_mask();
                                }
                            });
                            panel.sync_opacity_slider(window, cx);
                            panel.touch(cx);
                            cx.stop_propagation();
                            return;
                        }
                        let mode = selection_mode(modifiers.shift, modifiers.alt);
                        panel
                            .session
                            .update(cx, |session, _| session.load_mask_selection(id, mode));
                        panel.touch(cx);
                    },
                )),
            );
        }

        // Name (or the rename input).
        let mut name_cell = gpui_kit::div().flex_1().min_w_0();
        if renaming {
            if let Some(input) = self.rename_input.clone() {
                name_cell = name_cell.child(
                    gpui_kit::component::input::Input::new(&input)
                        .id("rename-input")
                        .h(px(20.0))
                        .text_size(px(11.0)),
                );
            }
        } else {
            // The name carries the clip arrow; the subtitle reads what the
            // row holds, or what it clips to (NativeLayerList.swift:682-688).
            let name = format!(
                "{}{}",
                if clipped_to.is_some() { "↳ " } else { "" },
                layer.name
            );
            let clip_tooltip = clipped_to.map(|source| {
                let source_name = self
                    .session
                    .read(cx)
                    .document
                    .as_ref()
                    .and_then(|document| document.layer(source))
                    .map(|l| l.name.clone())
                    .unwrap_or_else(|| "Missing source".to_string());
                format!(
                    "Clipping mask based on {source_name}. Option-click the bottom of its row to release."
                )
            });
            let subtitle = if let Some(source) = clipped_to {
                let source_name = self
                    .session
                    .read(cx)
                    .document
                    .as_ref()
                    .and_then(|document| document.layer(source))
                    .map(|l| l.name.clone())
                    .unwrap_or_else(|| "Missing source".to_string());
                format!("Clipped to {source_name}")
            } else if layer.text.is_some() {
                "Text".to_string()
            } else if layer.adjustment.is_some() {
                "Adjustment · Double-click to edit".to_string()
            } else if is_group {
                "Folder".to_string()
            } else {
                format!(
                    "{} × {} px",
                    layer.transform.size.width.round() as i64,
                    layer.transform.size.height.round() as i64
                )
            };
            name_cell = name_cell.child(
                gpui_kit::div()
                    .id(layer_key("name", id))
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .on_click(
                        cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                this.begin_rename(id, window, cx);
                                // A double-click on the name is a rename, not the
                                // row's folder toggle.
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .child(
                        gpui_kit::div()
                            .text_size(px(11.0))
                            .text_color(gpui_kit::rgb(crate::ui::FG_TEXT))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(SharedString::from(name)),
                    )
                    .child({
                        let mut line = gpui_kit::div()
                            .id(layer_key("subtitle", id))
                            .text_size(px(10.0))
                            .text_color(gpui_kit::rgb(crate::ui::FG_DIM))
                            .overflow_hidden()
                            .text_ellipsis();
                        if let Some(tip) = clip_tooltip.clone() {
                            line = line.tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                    .build(window, cx)
                            });
                        }
                        line.child(SharedString::from(subtitle))
                    }),
            );
        }
        row = row.child(name_cell);

        // The row's right-click menu (NativeLayerList.swift:604-613, cropped
        // to what the session models). Opening it first activates the row —
        // Swift's menu actions all run through selectImage().
        let menu_panel = panel.clone();
        let row = row.context_menu(move |menu, window, cx| {
            row_activate(&menu_panel, id, window, cx);
            let menu = menu
                .item(PopupMenuItem::new("Rename…").on_click({
                    let panel = menu_panel.clone();
                    move |_, window, cx| {
                        panel
                            .update(cx, |panel, cx| panel.begin_rename(id, window, cx))
                            .ok();
                    }
                }))
                .item({
                    let panel = menu_panel.clone();
                    PopupMenuItem::new(if visible { "Hide" } else { "Show" }).on_click(
                        move |_, _, cx| {
                            panel
                                .update(cx, |panel, cx| {
                                    panel.session.update(cx, |session, _| {
                                        session.toggle_layer_visibility(id)
                                    });
                                    panel.touch(cx);
                                })
                                .ok();
                        },
                    )
                })
                .separator();
            let menu = if has_mask {
                menu.item({
                    let panel = menu_panel.clone();
                    PopupMenuItem::new(if mask_enabled {
                        "Disable Mask"
                    } else {
                        "Enable Mask"
                    })
                    .on_click(move |_, _, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel
                                    .session
                                    .update(cx, |session, _| session.toggle_layer_mask());
                                panel.touch(cx);
                            })
                            .ok();
                    })
                })
                .item({
                    let panel = menu_panel.clone();
                    PopupMenuItem::new("Delete Mask").on_click(move |_, _, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel
                                    .session
                                    .update(cx, |session, _| session.delete_layer_mask());
                                panel.touch(cx);
                            })
                            .ok();
                    })
                })
            } else {
                menu.item({
                    let panel = menu_panel.clone();
                    PopupMenuItem::new("Add White Mask").on_click(move |_, _, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel
                                    .session
                                    .update(cx, |session, _| session.add_layer_mask(true));
                                panel.touch(cx);
                            })
                            .ok();
                    })
                })
                .item({
                    let panel = menu_panel.clone();
                    PopupMenuItem::new("Add Black Mask").on_click(move |_, _, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel
                                    .session
                                    .update(cx, |session, _| session.add_layer_mask(false));
                                panel.touch(cx);
                            })
                            .ok();
                    })
                })
            };
            // Release Clipping Mask sits after the mask items for every row
            // (NativeLayerList.swift:607); the session no-ops when the layer
            // clips nothing.
            let menu = menu.item({
                let panel = menu_panel.clone();
                PopupMenuItem::new("Release Clipping Mask").on_click(move |_, _, cx| {
                    panel
                        .update(cx, |panel, cx| {
                            panel
                                .session
                                .update(cx, |session, _| session.remove_live_mask(id));
                            panel.touch(cx);
                        })
                        .ok();
                })
            });
            let menu = menu.separator();
            let menu = if in_folder {
                menu.item({
                    let panel = menu_panel.clone();
                    PopupMenuItem::new("Move Out of Folder").on_click(move |_, _, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel.session.update(cx, |session, _| {
                                    session.move_active_layer_out_of_group()
                                });
                                panel.touch(cx);
                            })
                            .ok();
                    })
                })
            } else {
                menu
            };
            menu.item({
                let panel = menu_panel.clone();
                PopupMenuItem::new("Delete Layer / Folder").on_click(move |_, window, cx| {
                    // deleteLayerOrMask: deletes the mask when it is the target,
                    // and asks about live-mask dependents otherwise
                    // (request_delete_layer).
                    panel
                        .update(cx, |panel, cx| {
                            panel
                                .app
                                .update(cx, |app, cx| app.request_delete_layer(window, cx))
                                .ok();
                        })
                        .ok();
                })
            })
        });

        // The per-effect rows (LayerEffectRow, NativeLayerList.swift:849-940,
        // built per kind in LayerCell.configure:620-629): one 24px line under
        // the layer row per carried effect, each with its own eye. Their
        // clicks must not fall through to the layer row — a double-click
        // there would begin a rename.
        let effect_kinds = layer
            .effects
            .as_ref()
            .map(|effects| effects.kinds())
            .unwrap_or_default();
        let effect_selection = self.session.read(cx).selected_effect();
        let mut entry = gpui_kit::div().flex().flex_col().child(row);
        for kind in effect_kinds {
            entry = entry.child(self.render_effect_row(layer, kind, indent, effect_selection, cx));
        }

        // The entry answers an effect's Option-drag (validateDrop/acceptDrop,
        // NativeLayerList.swift:156-203): the copy lands on the row under the
        // pointer, so the whole entry — layer row and effect rows — counts.
        entry
            .drag_over::<EffectDrag>(move |style, drag: &EffectDrag, window, cx| {
                // Only an Option-drag copies, and only onto a layer that can
                // take the effect — highlight those targets alone.
                let copies = option_drag_modifiers(window.modifiers())
                    && panel
                        .upgrade()
                        .map(|panel| {
                            panel.read(cx).session.read(cx).can_copy_effect(
                                drag.kind,
                                drag.layer_id,
                                id,
                            )
                        })
                        .unwrap_or(false);
                if copies {
                    style.bg(gpui_kit::rgba(crate::ui::ACCENT_WASH))
                } else {
                    style
                }
            })
            .on_drop(cx.listener(move |this, drag: &EffectDrag, window, cx| {
                // acceptDrop (NativeLayerList.swift:184-191) runs the copy
                // through canCopyEffect/copyEffect; the modifier gate is
                // re-checked here because GPUI cannot decline a drag that
                // did not start with Option.
                if !option_drag_modifiers(window.modifiers()) {
                    return;
                }
                this.session.update(cx, |session, _| {
                    session.copy_effect(drag.kind, drag.layer_id, id)
                });
                this.sync_opacity_slider(window, cx);
                this.touch(cx);
            }))
            .into_any_element()
    }

    /// One effect row (`LayerEffectRow`): the kind's eye — each press one
    /// undo step through `toggleEffect` (LayerEffects.swift:350-355) — the
    /// kind's name, click to select and double-click to open the effect's
    /// editor panel (`selectEffect(_:on:editing:)`, LayerEffects.swift:
    /// 264-277), and an Option-drag offering the effect for copying
    /// (`mouseDragged`, NativeLayerList.swift:912-929).
    fn render_effect_row(
        &self,
        layer: &ImageLayer,
        kind: LayerEffectKind,
        indent: f32,
        effect_selection: Option<LayerEffectSelection>,
        cx: &Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = layer.id;
        let enabled = layer
            .effects
            .as_ref()
            .map(|effects| effects.is_enabled(kind))
            .unwrap_or(false);
        let selected = effect_selection == Some(LayerEffectSelection { layer_id: id, kind });
        let tooltip: SharedString = format!(
            "Click to select; double-click to edit; Option-drag to copy {}",
            kind.label().to_lowercase()
        )
        .into();

        gpui_kit::div()
            .id(effect_key("row", id, kind))
            .flex()
            .flex_row()
            .items_center()
            .h(px(24.0))
            .mx_1()
            .rounded_sm()
            // The Swift row's eye sits 38px in — its own 8px leading, the
            // 20px eye, a 10px gap — plus the layer row's indent step
            // (NativeLayerList.swift:875); the 4px row margin makes 34 here.
            .pl(px(34.0 + indent))
            .when(selected, |row| {
                row.bg(gpui_kit::rgba(crate::ui::ACCENT_WASH))
            })
            .on_click(cx.listener(move |this, event, window, cx| {
                // `select(editing:)` — a click selects the effect, a
                // double-click also binds the editor panel to it; the
                // panel itself opens off `effectsEditing` exactly as it
                // does for a fresh add.
                let editing = click_count(event) >= 2;
                this.session
                    .update(cx, |session, _| session.select_effect(kind, id, editing));
                if editing
                    && this.session.read(cx).effects_editing
                        == Some(LayerEffectSelection { layer_id: id, kind })
                {
                    // Opening the floating panel is an app-side flow.
                    this.app
                        .update(cx, |app, cx| app.open_effects_panel(cx))
                        .ok();
                }
                this.sync_opacity_slider(window, cx);
                this.touch(cx);
                cx.stop_propagation();
            }))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .on_drag(
                EffectDrag { layer_id: id, kind },
                // The drag ghost (Swift snapshots the row): a small chip
                // naming the effect — but only when the drag began with
                // Option, the only drag a drop will answer.
                move |_, _, window, cx| {
                    let visible = option_drag_modifiers(window.modifiers());
                    cx.new(|_| EffectDragGhost {
                        label: kind.label().into(),
                        visible,
                    })
                },
            )
            // The eye toggles visibility alone — an eye click must not select
            // the effect (NativeLayerList.swift:935).
            .child(
                gpui_kit::div()
                    .id(effect_key("eye", id, kind))
                    .w(px(18.0))
                    .h(px(18.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.session
                            .update(cx, |session, _| session.toggle_effect(kind, id));
                        this.touch(cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        Icon::new(if enabled {
                            IconName::Eye
                        } else {
                            IconName::EyeOff
                        })
                        .size(px(14.0))
                        // Swift keeps the eye at the secondary tint in both
                        // states (NativeLayerList.swift:866); the glyph
                        // carries the state.
                        .text_color(gpui_kit::rgb(crate::ui::FG_DIM)),
                    ),
            )
            .child(
                gpui_kit::div()
                    .ml_1()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.0))
                    .text_color(if enabled {
                        gpui_kit::rgb(crate::ui::FG_TEXT)
                    } else {
                        gpui_kit::rgb(crate::ui::FG_DIM)
                    })
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(SharedString::from(kind.label())),
            )
            .into_any_element()
    }

    /// The row's mask thumbnail: the mask's own pixels placed where they sit
    /// on a canvas-shaped frame filled with the mask's edge tone
    /// (`CanvasThumbnail.mask`, render.rs mask_canvas_thumbnail) — the same
    /// producer `thumbnail_image` uses when the mask is the row's subject.
    fn mask_thumbnail_image(
        &self,
        layer: &ImageLayer,
        cx: &gpui_kit::App,
    ) -> Option<Arc<RenderImage>> {
        let mask = layer.mask.as_ref()?;
        let document = self.session.read(cx).document.as_ref()?;
        let canvas =
            compositor_core::geometry::CGSize::new(document.width as f64, document.height as f64);
        let thumbnail =
            compositor_core::render::mask_canvas_thumbnail(mask, &layer.mask_transform(), canvas)?;
        Some(crate::state::bgra_render_image(&thumbnail))
    }
}

#[derive(IntoElement)]
pub struct FooterButton {
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    accessibility: &'static str,
    disabled: bool,
    on_click: ClickHandler,
}

impl FooterButton {
    pub fn new(
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        accessibility: &'static str,
        disabled: bool,
    ) -> Self {
        FooterButton {
            id,
            icon,
            tooltip,
            accessibility,
            disabled,
            on_click: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_click(mut self, on_click: ClickHandler) -> Self {
        self.on_click = on_click;
        self
    }
}

impl RenderOnce for FooterButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Button::new(self.id)
            .icon(Icon::new(self.icon))
            // Swift sets these labels explicitly (LayersPanel.swift:38-60);
            // group's differs from its help text ("New folder").
            .accessibility_label(self.accessibility)
            .ghost()
            .small()
            .tooltip(self.tooltip)
            .disabled(self.disabled)
            .on_click(move |event, window, cx| (self.on_click)(event, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        effect_key, footer_mask_disabled, layer_key, option_drag_modifiers, selection_mode,
    };
    use compositor_core::effects::LayerEffectKind;
    use compositor_core::selection::SelectionMode;
    use compositor_core::session::EditorSession;
    use gpui_kit::Modifiers;

    /// The footer mask button's three retired states (LayerMaskMenu.swift):
    /// no active layer, a layer already carrying a mask, or a session that
    /// cannot edit masks.
    #[test]
    fn footer_mask_button_disabled_states() {
        let mut session = EditorSession::new();
        // No document, no active layer: nothing to mask.
        assert!(footer_mask_disabled(&session));

        session.create_document(10, 10, true);
        session.add_blank_layer();
        // A bare layer takes a mask...
        assert!(!footer_mask_disabled(&session));
        // ...and once it carries one, the button retires.
        session.add_layer_mask(true);
        assert!(footer_mask_disabled(&session));
    }

    /// Row keys come from the layer's UUID (coding-guides "Stable
    /// identity"), so a reorder can never rebind a row's state: the same id
    /// always yields the same key, different ids never collide.
    #[test]
    fn layer_keys_come_from_ids_not_positions() {
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        assert_eq!(layer_key("row", a), layer_key("row", a));
        assert_ne!(layer_key("row", a), layer_key("row", b));
    }

    /// Effect-row keys are stable the same way, and name the kind: the same
    /// layer and kind always collide with themselves alone, and two kinds on
    /// one layer never share a key.
    #[test]
    fn effect_keys_come_from_layer_id_and_kind() {
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        assert_eq!(
            effect_key("row", a, LayerEffectKind::Stroke),
            effect_key("row", a, LayerEffectKind::Stroke)
        );
        assert_ne!(
            effect_key("row", a, LayerEffectKind::Stroke),
            effect_key("row", b, LayerEffectKind::Stroke)
        );
        assert_ne!(
            effect_key("row", a, LayerEffectKind::Stroke),
            effect_key("row", a, LayerEffectKind::Shadow)
        );
    }

    /// An effect-drag copies only with Option and without Command — on this
    /// platform either of the Command seats (control, platform) vetoes it
    /// (NativeLayerList.swift:905, 918).
    #[test]
    fn option_drag_requires_alt_without_command() {
        let plain = Modifiers::default();
        assert!(!option_drag_modifiers(plain));
        assert!(option_drag_modifiers(Modifiers {
            alt: true,
            ..Modifiers::default()
        }));
        assert!(!option_drag_modifiers(Modifiers {
            alt: true,
            control: true,
            ..Modifiers::default()
        }));
        assert!(!option_drag_modifiers(Modifiers {
            alt: true,
            platform: true,
            ..Modifiers::default()
        }));
    }

    /// Shift adds, Option subtracts, plain replaces; shift wins when both
    /// modifiers ride along (NativeLayerList.swift:785-789).
    #[test]
    fn selection_mode_follows_shift_then_alt() {
        assert!(matches!(
            selection_mode(false, false),
            SelectionMode::Replace
        ));
        assert!(matches!(selection_mode(true, false), SelectionMode::Add));
        assert!(matches!(
            selection_mode(false, true),
            SelectionMode::Subtract
        ));
        assert!(matches!(selection_mode(true, true), SelectionMode::Add));
    }
}
