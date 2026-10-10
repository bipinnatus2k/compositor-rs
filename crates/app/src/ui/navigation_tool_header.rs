//! The left tool rail and the options header shell (NavigationToolHeader.swift
//! plus ContentView's `toolHeaders` group): the tool title and the dispatch
//! to each tool's controls. The per-tool control sets are view entities
//! (ADR 0003 step E, ticket 42) living beside their Swift sources
//! (`ui/brush_controls.rs`, `ui/lasso_controls.rs`, `ui/crop_controls.rs`,
//! `ui/shape_controls.rs`, `ui/type_controls.rs`,
//! `ui/transform_inspector.rs`); the NavigationToolHeader and
//! GradientControls entities stay here, where their arms were carved
//! (GradientControls has no file of its own in the port).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    px,
};

use compositor_core::document::NavigationTool;
use compositor_core::session::{BrushToolMode, EditorSession};
use compositor_core::tools::brush::GradientShape;

use crate::canvas::editor_canvas::CanvasDrag;
use crate::state::{AppState, slider};
use crate::ui::brush_controls::BrushControlsView;
use crate::ui::crop_controls::CropControlsView;
use crate::ui::lasso_controls::LassoControlsView;
use crate::ui::shape_controls::ShapeControlsView;
use crate::ui::transform_inspector::TransformInspectorView;
use crate::ui::type_controls::TypeControlsView;
use crate::ui::{
    AppCallback, ClickHandler, Dropdown, LabeledCheckbox, LabeledSlider, MenuEntry, app_dispatch,
    app_on, app_on_click, text_label,
};

/// The rail entries in display order: (tool, id letter, Lucide icon, name,
/// eraser-variant flag). The letter only names the button element (kept so
/// existing element ids are stable); the glyph comes from the bundled
/// Lucide set (`gpui_kit::assets`). Brush and Eraser share the Brush tool.
const TOOLS: [(NavigationTool, &str, IconName, &str, bool); 16] = [
    (
        NavigationTool::Move,
        "V",
        IconName::Move,
        "Move / Transform",
        false,
    ),
    (
        NavigationTool::Marquee,
        "M",
        IconName::SquareDashed,
        "Marquee",
        false,
    ),
    (NavigationTool::Lasso, "L", IconName::Lasso, "Lasso", false),
    (
        NavigationTool::Wand,
        "W",
        IconName::WandSparkles,
        "Magic Wand",
        false,
    ),
    (NavigationTool::Crop, "C", IconName::Crop, "Crop", false),
    (NavigationTool::Brush, "B", IconName::Brush, "Brush", false),
    (NavigationTool::Brush, "E", IconName::Eraser, "Eraser", true),
    (
        NavigationTool::SpotHealing,
        "J",
        IconName::Bandage,
        "Spot Healing Brush",
        false,
    ),
    (
        NavigationTool::CloneStamp,
        "S",
        IconName::Stamp,
        "Clone Stamp",
        false,
    ),
    (NavigationTool::Blur, "R", IconName::Droplet, "Smear", false),
    (
        NavigationTool::Gradient,
        "G",
        IconName::Blend,
        "Gradient",
        false,
    ),
    (NavigationTool::Shape, "U", IconName::Square, "Shape", false),
    (NavigationTool::Type, "T", IconName::Type, "Type", false),
    (
        NavigationTool::Eyedropper,
        "I",
        IconName::Pipette,
        "Eyedropper",
        false,
    ),
    (NavigationTool::Hand, "H", IconName::Hand, "Hand", false),
    (NavigationTool::Zoom, "Z", IconName::ZoomIn, "Zoom", false),
];

fn tool_title(tool: NavigationTool, erasing: bool) -> &'static str {
    match tool {
        NavigationTool::Move => "Move",
        NavigationTool::Marquee => "Marquee",
        NavigationTool::Lasso => "Lasso",
        NavigationTool::Wand => "Magic",
        NavigationTool::Crop => "Crop",
        NavigationTool::Brush => {
            if erasing {
                "Eraser"
            } else {
                "Brush"
            }
        }
        NavigationTool::SpotHealing => "Spot Healing",
        NavigationTool::CloneStamp => "Clone Stamp",
        NavigationTool::Blur => "Smear",
        NavigationTool::Gradient => "Gradient",
        NavigationTool::Shape => "Shape",
        NavigationTool::Type => "Type",
        NavigationTool::Eyedropper => "Eyedropper",
        NavigationTool::Hand => "Hand",
        NavigationTool::Zoom => "Zoom",
        NavigationTool::Idle => "No tool",
    }
}

/// EditorSession.swift:347-351: entering Crop with no frame presets the
/// full-document frame and resets the ratio choice to Free, so a ratio
/// left over from an earlier crop never reshapes the preset. The preset
/// itself moved into `EditorSession::select_tool` (ticket 16); this shim
/// only keeps the ticket-04 guard test's call shape.
#[cfg(test)]
fn crop_entry_preset(session: &mut EditorSession, tool: NavigationTool) {
    session.select_tool(tool);
}

/// The options header's shared "(value, label) → checked menu entry"
/// dropdown shape: one entry per value in the given order, the checkmark
/// on the entry matching `current` (a `String` field compares against its
/// `&'static str` presets), and a click assigning the value to the session
/// field `set` names. The per-tool control modules share it.
pub(crate) fn menu_entries<V, C, S>(
    values: impl IntoIterator<Item = (V, &'static str)>,
    current: C,
    set: S,
) -> Vec<MenuEntry>
where
    V: Copy + 'static,
    C: PartialEq<V>,
    S: Fn(&mut EditorSession, V) + Clone + 'static,
{
    values
        .into_iter()
        .map(|(value, label)| {
            let assign = set.clone();
            MenuEntry::new(label, move |state, _, cx| {
                state
                    .tab()
                    .session
                    .update(cx, |session, _| assign(session, value));
                state.touch(cx);
            })
            .checked(current == value)
        })
        .collect()
}

impl AppState {
    pub fn set_tool(&mut self, tool: NavigationTool, cx: &mut Context<Self>) {
        // `selectTool` (EditorSession.swift:332-352) lives on the session
        // now, with its guards (:334) — the busy flags are session fields
        // (stage-D ticket 34). What stays here: the stroke/warp drags are
        // canvas gesture state (canvas view entity since ticket 39), and the
        // adjustment editor is the port's stand-in for Swift's `levels`
        // sheet. A refused switch (a failing text apply) leaves the shape
        // drag up, like every session-side gate.
        let dragging = self.canvas_view.read(cx).drag.as_ref();
        if matches!(
            dragging,
            Some(CanvasDrag::Stroke(..)) | Some(CanvasDrag::Warp(..))
        ) {
            return;
        }
        if self.current_tab().session.read(cx).is_project_busy
            || self
                .current_tab()
                .session
                .read(cx)
                .adjustment_editing_id
                .is_some()
        {
            return;
        }
        if !self
            .tab()
            .session
            .update(cx, |session, _| session.select_tool(tool))
        {
            return;
        }
        // A shape drag dies with the switch (Swift's cancelShape :335); it
        // lives on the canvas, so dropping it here is the whole cancel.
        if self
            .canvas_view
            .read(cx)
            .drag
            .as_ref()
            .is_some_and(|drag| matches!(drag, CanvasDrag::Shape { .. }))
        {
            self.canvas_view.update(cx, |canvas, _| canvas.drag = None);
        }
        self.touch(cx);
    }

    pub fn set_tool_eraser(&mut self, cx: &mut Context<Self>) {
        self.set_tool(NavigationTool::Brush, cx);
        self.tab()
            .session
            .update(cx, |session, _| session.brush_mode = BrushToolMode::Erase);
        self.touch(cx);
    }

    // MARK: Tool rail

    pub fn render_tool_rail(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let (active, eraser_active) = {
            let session = self.tab().session.read(cx);
            (
                session.tool,
                session.tool == NavigationTool::Brush && session.brush_mode == BrushToolMode::Erase,
            )
        };

        let mut rail = gpui_kit::div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(44.0))
            .flex_shrink_0()
            .py_1()
            .bg(crate::ui::panel_bg());

        for (tool, letter, icon, name, eraser_entry) in TOOLS {
            let selected = if eraser_entry {
                eraser_active
            } else if tool == NavigationTool::Brush {
                active == NavigationTool::Brush && !eraser_active
            } else {
                active == tool
            };
            rail = rail.child(
                ToolButton::new(tool, letter, icon, name, eraser_entry, selected).with_on_click(
                    app_on_click(&app, move |state, _, _, cx| {
                        if eraser_entry {
                            state.set_tool_eraser(cx);
                        } else {
                            state.set_tool(tool, cx);
                        }
                    }),
                ),
            );
        }

        // Foreground / background swatches: clicking either opens the color
        // picker on it (ColorPalette.swift:60; the X key still swaps).
        let foreground = self.tab().session.read(cx).palette_color(false);
        let background = self.tab().session.read(cx).palette_color(true);
        rail = rail.child(
            gpui_kit::div()
                .mt_2()
                .id("palette")
                .w(px(28.0))
                .h(px(28.0))
                .relative()
                .child(
                    Swatch::new("swatch-background", 6.0, 6.0, background).with_on_click(
                        app_on_click(&app, |state, _, window, cx| {
                            state.open_color_picker_ui(true, window, cx);
                        }),
                    ),
                )
                .child(
                    Swatch::new("swatch-foreground", 0.0, 0.0, foreground).with_on_click(
                        app_on_click(&app, |state, _, window, cx| {
                            state.open_color_picker_ui(false, window, cx);
                        }),
                    ),
                ),
        );

        rail.into_any_element()
    }

    // MARK: Options header

    /// The options header row (Swift `toolHeaders`' shared bar): the title
    /// plus the active tool's controls. Each per-tool control set is a view
    /// entity (ticket 42); only the active tool's entity is consulted per
    /// pass, mirroring Swift's `if session.tool == ...` branches — the
    /// entities are persistent (`AppState` fields) and re-pointed on tab
    /// changes by [`AppState::sync_session_observer`].
    pub fn render_options_header(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (tool, erasing) = {
            let session = self.tab().session.read(cx);
            (session.tool, session.brush_mode == BrushToolMode::Erase)
        };

        let mut header = gpui_kit::div()
            // Stateful so the bar can scroll sideways (overflow_x_scroll is a
            // StatefulInteractiveElement method, gpui div.rs).
            .id("options-header")
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            // ToolHeaderStyle.swift:5 (height 42) and BrushControls.swift:111
            // (horizontal padding 18).
            .h(px(42.0))
            .px(px(18.0))
            .gap_3()
            .flex_shrink_0()
            // Dense bars (Type, Move) run out of width; scroll sideways
            // instead of clipping the tail controls.
            .overflow_x_scroll()
            .bg(crate::ui::panel_bg());

        header = header.child(
            text_label(tool_title(tool, erasing), false)
                .w(px(82.0))
                // ToolHeaderStyle.swift:6: the title is 13pt semibold.
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0(),
        );

        match tool {
            NavigationTool::Brush
            | NavigationTool::SpotHealing
            | NavigationTool::Blur
            | NavigationTool::CloneStamp => {
                header = header.children(
                    self.brush_header
                        .update(cx, BrushControlsView::brush_controls),
                );
            }
            NavigationTool::Marquee => {
                header = header.children(
                    self.lasso_header
                        .update(cx, LassoControlsView::marquee_controls),
                );
            }
            NavigationTool::Lasso => {
                header = header.children(
                    self.lasso_header
                        .update(cx, LassoControlsView::lasso_controls),
                );
            }
            NavigationTool::Wand => {
                header = header.children(
                    self.lasso_header
                        .update(cx, LassoControlsView::wand_controls),
                );
            }
            NavigationTool::Gradient => {
                // Swift home: GradientControls.swift; the entity lives here,
                // where the arm was carved (no file of its own in the port).
                header = header.children(
                    self.gradient_header
                        .update(cx, GradientControlsView::gradient_controls),
                );
            }
            NavigationTool::Zoom | NavigationTool::Hand => {
                header = header.children(
                    self.navigation_header
                        .update(cx, NavigationToolHeaderView::controls),
                );
            }
            NavigationTool::Crop => {
                header =
                    header.children(self.crop_header.update(cx, CropControlsView::crop_controls));
            }
            _ => {}
        }

        if tool == NavigationTool::Move {
            header = header.children(
                self.transform_header
                    .update(cx, TransformInspectorView::transform_inspector),
            );
        }
        if tool == NavigationTool::Type {
            header = header.children(self.type_header.update(cx, TypeControlsView::type_controls));
        }
        if tool == NavigationTool::Shape {
            header = header.children(
                self.shape_header
                    .update(cx, ShapeControlsView::shape_controls),
            );
        }

        header.into_any_element()
    }
}

/// Swift `NavigationToolHeader`'s Zoom/Hand content: a view entity since
/// ticket 42. The Swift view's `zoomText`/`displayedZoomText` drafts and
/// stepper (NavigationToolHeader.swift:5-8) have no Rust counterpart — the
/// port renders the four zoom buttons and the pan hint, so nothing hangs
/// beside the seam (recorded, not invented; ticket 43's per-tab rebuild is
/// where a draft would land if the field were ever ported).
pub struct NavigationToolHeaderView {
    /// The active tab's session — the branch pick reads its tool.
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the zoom commands route
    /// through it (`zoom_fit`, the keyboard zoom levels). No AppState
    /// dependencies beyond what the moved handlers already used.
    app: WeakEntity<AppState>,
}

impl NavigationToolHeaderView {
    pub(crate) fn new(session: Entity<EditorSession>, app: WeakEntity<AppState>) -> Self {
        NavigationToolHeaderView { session, app }
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel).
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// `NavigationToolHeader.swift`'s body: the four zoom buttons on Zoom,
    /// the pan hint on Hand.
    pub(crate) fn controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        match self.session.read(cx).tool {
            NavigationTool::Zoom => {
                let on_zoom = app_on(&app, |state, command, _, cx| match command {
                    ZoomCommand::Fit => state.zoom_fit(cx),
                    ZoomCommand::ActualSize => {
                        // Actual Pixels: `session.zoom(to: 1)` (CompositorApp.swift:88).
                        state
                            .tab()
                            .session
                            .update(cx, |session, _| session.zoom(1.0, None));
                        state.touch(cx);
                    }
                    ZoomCommand::In => {
                        state
                            .tab()
                            .session
                            .update(cx, |session, _| session.zoom_keyboard(1));
                        state.touch(cx);
                    }
                    ZoomCommand::Out => {
                        state
                            .tab()
                            .session
                            .update(cx, |session, _| session.zoom_keyboard(-1));
                        state.touch(cx);
                    }
                });
                vec![
                    ZoomButton::new("zoom-fit", ZoomCommand::Fit)
                        .with_on_zoom(on_zoom.clone())
                        .into_any_element(),
                    ZoomButton::new("zoom-100", ZoomCommand::ActualSize)
                        .with_on_zoom(on_zoom.clone())
                        .into_any_element(),
                    ZoomButton::new("zoom-in", ZoomCommand::In)
                        .with_on_zoom(on_zoom.clone())
                        .into_any_element(),
                    ZoomButton::new("zoom-out", ZoomCommand::Out)
                        .with_on_zoom(on_zoom)
                        .into_any_element(),
                ]
            }
            NavigationTool::Hand => {
                vec![text_label("Drag to pan · scroll to zoom", true).into_any_element()]
            }
            _ => Vec::new(),
        }
    }
}

/// Swift `GradientControls`' option bar: a view entity since ticket 42 — it
/// owns the opacity slider entity and its write subscription
/// (`subscribe_sliders`' gradient slice). The struct lives here, where the
/// Gradient arm was carved into this file pending its own carve (the Swift
/// source is its own view; no Rust file was ever made for it). Only in the
/// render tree while the Gradient tool is active, so it keeps no session
/// observer; its session field is re-pointed on tab changes by
/// [`AppState::sync_session_observer`].
pub struct GradientControlsView {
    /// The active tab's session — the settings readouts and the checkbox
    /// writes speak to it directly (`GradientControls`' `@Bindable
    /// session`).
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the repaint epoch and the
    /// shared header row stay there. No AppState dependencies beyond what
    /// the moved handlers already used.
    app: WeakEntity<AppState>,
    /// The opacity slider (`GradientControls`' opacity binding): seeded from
    /// the persisted tool default at construction, exactly as
    /// `AppState::new` seeded it when the field lived there.
    gradient_slider: Entity<SliderState>,
    /// Keep-alive for the slider subscription: dropping a Subscription
    /// unsubscribes, so the bundle is held, never read.
    #[allow(dead_code)]
    subscriptions: Vec<Subscription>,
}

impl GradientControlsView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let opacity = session.read(cx).gradient_settings.opacity;
        let gradient_slider = cx.new(|_| slider(1.0, 100.0, 1.0, (opacity * 100.0) as f32));
        let mut view = GradientControlsView {
            session,
            app,
            gradient_slider,
            subscriptions: Vec::new(),
        };
        view.subscribe_sliders(cx);
        view
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel). The slider
    /// subscription reads this field at event time, so the re-point keeps
    /// it writing to the active tab.
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// The shell's Opacity-digit sync route (`AppState::opacity_digit` —
    /// the one write to it from outside this view).
    pub(crate) fn opacity_slider(&self) -> Entity<SliderState> {
        self.gradient_slider.clone()
    }

    /// Keeps the session's gradient opacity in sync with the slider
    /// (`subscribe_sliders`' gradient slice, moved with the slider): a
    /// Change or Release writes the value and bumps the repaint epoch.
    fn subscribe_sliders(&mut self, cx: &mut Context<Self>) {
        let gradient_slider = self.gradient_slider.clone();
        let subscription = cx.subscribe(
            &gradient_slider,
            |this, _slider, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                };
                this.session.update(cx, |session, _| {
                    session.gradient_settings.opacity = (value / 100.0) as f64
                });
                this.touch(cx);
            },
        );
        self.subscriptions.push(subscription);
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// an interaction that changed the session asks the shell through the
    /// seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// `GradientControls.swift`'s bar controls: the shape dropdown, the two
    /// color-route checkboxes and the opacity slider.
    pub(crate) fn gradient_controls(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let settings = self.session.read(cx).gradient_settings;
        vec![
            Dropdown::new(
                "gradient-shape",
                match settings.shape {
                    GradientShape::Linear => "Linear",
                    GradientShape::Radial => "Radial",
                },
            )
            .with_width(px(84.0))
            .with_entries(menu_entries(
                [
                    (GradientShape::Linear, "Linear"),
                    (GradientShape::Radial, "Radial"),
                ],
                settings.shape,
                |session, value| session.gradient_settings.shape = value,
            ))
            .with_dispatch(app_dispatch(&app))
            .into_any_element(),
            LabeledCheckbox::new("gradient-fg-bg", "Foreground to Background")
                .checked(settings.foreground_to_background)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state.tab().session.update(cx, |session, _| {
                        session.gradient_settings.foreground_to_background = checked
                    });
                    state.touch(cx);
                }))
                .into_any_element(),
            LabeledCheckbox::new("gradient-reverse", "Reverse")
                .checked(settings.reversed)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state.tab().session.update(cx, |session, _| {
                        session.gradient_settings.reversed = checked
                    });
                    state.touch(cx);
                }))
                .into_any_element(),
            LabeledSlider::new(
                "Opacity",
                self.gradient_slider.clone(),
                format!("{:.0}%", settings.opacity * 100.0),
                px(80.0),
            )
            .into_any_element(),
        ]
    }
}

#[derive(IntoElement)]
pub struct ToolButton {
    letter: &'static str,
    icon: IconName,
    name: &'static str,
    selected: bool,
    // Carried with the button for parity with the TOOLS row, but the render
    // never reads them: the eraser branch lives in the caller's `on_click`
    // glue, which captures the row's own values.
    #[allow(dead_code)]
    tool: NavigationTool,
    #[allow(dead_code)]
    eraser: bool,
    on_click: ClickHandler,
}

impl ToolButton {
    pub fn new(
        tool: NavigationTool,
        letter: &'static str,
        icon: IconName,
        name: &'static str,
        eraser: bool,
        selected: bool,
    ) -> Self {
        ToolButton {
            tool,
            letter,
            icon,
            name,
            eraser,
            selected,
            on_click: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_click(mut self, on_click: ClickHandler) -> Self {
        self.on_click = on_click;
        self
    }
}

impl RenderOnce for ToolButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Button::new(SharedString::from(format!("tool-{}", self.letter)))
            .icon(Icon::new(self.icon))
            // Icon-only buttons have no label to read aloud; the tooltip name
            // doubles as the announced one.
            .accessibility_label(self.name)
            .ghost()
            .compact()
            .selected(self.selected)
            .tooltip(self.name)
            .on_click(move |event, window, cx| (self.on_click)(event, window, cx))
    }
}

#[derive(IntoElement)]
pub struct Swatch {
    id: SharedString,
    offset_x: f32,
    offset_y: f32,
    color: compositor_core::document::PaletteColor,
    on_click: ClickHandler,
}

impl Swatch {
    pub fn new(
        id: impl Into<SharedString>,
        offset_x: f32,
        offset_y: f32,
        color: compositor_core::document::PaletteColor,
    ) -> Self {
        Swatch {
            id: id.into(),
            offset_x,
            offset_y,
            color,
            on_click: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_click(mut self, on_click: ClickHandler) -> Self {
        self.on_click = on_click;
        self
    }
}

impl RenderOnce for Swatch {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_click = self.on_click;
        let value = ((self.color.red * 255.0).round() as u32) << 16
            | ((self.color.green * 255.0).round() as u32) << 8
            | (self.color.blue * 255.0).round() as u32;
        gpui_kit::div()
            .id(self.id)
            .absolute()
            .top(px(self.offset_y))
            .left(px(self.offset_x))
            .w(px(18.0))
            .h(px(18.0))
            .rounded_sm()
            .border_1()
            .border_color(gpui_kit::rgb(crate::ui::BORDER_BRIGHT))
            .bg(gpui_kit::rgb(value))
            .on_click(move |event, window, cx| on_click(event, window, cx))
    }
}

/// What one zoom header button does (the zoom toolbar's Fit / 100% / +
/// / -). The command is explicit; the label is only the button's face.
#[derive(Clone, Copy)]
pub(crate) enum ZoomCommand {
    Fit,
    ActualSize,
    In,
    Out,
}

impl ZoomCommand {
    fn label(self) -> &'static str {
        match self {
            ZoomCommand::Fit => "Fit",
            ZoomCommand::ActualSize => "100%",
            ZoomCommand::In => "+",
            ZoomCommand::Out => "-",
        }
    }
}

#[derive(IntoElement)]
pub struct ZoomButton {
    id: &'static str,
    command: ZoomCommand,
    on_zoom: AppCallback<ZoomCommand>,
}

impl ZoomButton {
    pub(crate) fn new(id: &'static str, command: ZoomCommand) -> Self {
        ZoomButton {
            id,
            command,
            on_zoom: Rc::new(|_, _, _| {}),
        }
    }

    pub(crate) fn with_on_zoom(mut self, on_zoom: AppCallback<ZoomCommand>) -> Self {
        self.on_zoom = on_zoom;
        self
    }
}

impl RenderOnce for ZoomButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_zoom = self.on_zoom;
        let command = self.command;
        Button::new(self.id)
            .label(command.label())
            .outline()
            .small()
            .on_click(move |_, window, cx| on_zoom(command, window, cx))
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::{crop_entry_preset, menu_entries, tool_title};
    use compositor_core::document::NavigationTool;
    use compositor_core::geometry::CGRect;
    use compositor_core::session::EditorSession;

    #[test]
    fn tool_title_names_tools_and_eraser_variant() {
        assert_eq!(tool_title(NavigationTool::Move, false), "Move");
        assert_eq!(tool_title(NavigationTool::Idle, false), "No tool");
        // The brush has two modes depending on the eraser toggle.
        assert_eq!(tool_title(NavigationTool::Brush, false), "Brush");
        assert_eq!(tool_title(NavigationTool::Brush, true), "Eraser");
    }

    /// The shared dropdown builder yields the same menu items and checked
    /// state as the per-tool constructions it replaced: one entry per
    /// value in the given order, the checkmark on the current one, every
    /// entry enabled and no separators.
    #[test]
    fn menu_entries_keep_the_item_order_and_checked_state() {
        use compositor_core::session::SpotHealingMode;

        let entries = menu_entries(
            [
                (SpotHealingMode::ContentAware, "Content Aware"),
                (SpotHealingMode::CreateTexture, "Create Texture"),
                (SpotHealingMode::ProximityMatch, "Proximity Match"),
            ],
            SpotHealingMode::CreateTexture,
            |session, value| session.spot_healing_mode = value,
        );
        let labels: Vec<_> = entries.iter().map(|entry| entry.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Content Aware", "Create Texture", "Proximity Match"]
        );
        let checked: Vec<_> = entries.iter().map(|entry| entry.checked).collect();
        assert_eq!(checked, [false, true, false]);
        assert!(
            entries
                .iter()
                .all(|entry| entry.enabled && !entry.separator)
        );

        // A String field compares by content against its &'static str
        // presets, exactly like the inline crop construction did.
        let entries = menu_entries(
            [
                ("Free", "Free"),
                ("Original", "Original"),
                ("1:1", "1:1"),
                ("4:3", "4:3"),
                ("16:9", "16:9"),
            ],
            "4:3".to_string(),
            |session, value| session.crop_ratio_choice = value.to_string(),
        );
        let checked: Vec<_> = entries.iter().map(|entry| entry.checked).collect();
        assert_eq!(checked, [false, false, false, true, false]);
    }

    /// EditorSession.swift:347-351: entering Crop with no frame presets the
    /// full-document frame AND resets the ratio choice to Free — a ratio
    /// left over from an earlier crop must not reshape the preset.
    #[test]
    fn crop_entry_preset_resets_a_stale_ratio_choice() {
        let mut session = EditorSession::new();
        session.create_document(800, 600, false);
        session.crop_ratio_choice = "4:3".to_string();

        crop_entry_preset(&mut session, NavigationTool::Crop);
        assert_eq!(session.crop_ratio_choice, "Free");
        assert_eq!(session.crop_rect, Some(CGRect::new(0.0, 0.0, 800.0, 600.0)));

        // An existing frame is never replaced, and its ratio choice stands.
        let framed = CGRect::new(10.0, 10.0, 100.0, 100.0);
        session.crop_rect = Some(framed);
        session.crop_ratio_choice = "1:1".to_string();
        crop_entry_preset(&mut session, NavigationTool::Crop);
        assert_eq!(session.crop_rect, Some(framed));
        assert_eq!(session.crop_ratio_choice, "1:1");
    }
}
