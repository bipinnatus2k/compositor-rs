//! The color picker panel (`UI/ColorPickerSheet.swift`) as its own view
//! entity (ADR 0003 step D, ticket 41): the drawn saturation/brightness
//! field, the vertical hue strip with its arrow markers, the new-color
//! preview swatch, the R/G/B and hex entries and OK/Cancel, laid out exactly
//! like Swift's `HStack { saturationBrightnessField; hueStrip; … }`. The
//! working color is the session's `color_picker.hsb`, so nothing lands until
//! OK; field/strip drags feed the same state the former sliders did,
//! including an effect color's live preview. The text entries are the
//! panel's own fields — Swift's `@State hexDraft` and the number TextFields
//! (ColorPickerSheet.swift:10, 104-140), the former
//! `ProjectWorkspaceView.picker_ui`.
//!
//! The hue/saturation/brightness sliders that predated the drawn
//! field are gone — the field and the hue strip feed the same state.

use std::sync::Arc;

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::{
    App, AppContext, Bounds, Context, Corners, Entity, Focusable, Global, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PathBuilder, Pixels, Point, Render, RenderImage, RenderOnce, Styled, WeakEntity,
    Window, canvas, point, px, rgba, size,
};
use image::Frame;

use compositor_core::document::{PaletteColor, PickerHSB};
use compositor_core::geometry::CGFloat;
use compositor_core::session::{ColorPickerTarget, EditorSession};

use crate::state::ProjectWorkspaceView;
use crate::ui::floating_panel::{panel_on_drag_move, panel_on_press};
use crate::ui::{ClickHandler, DragMoveHandler, PressHandler, app_on_click, text_label};

/// `fieldSize` (ColorPickerSheet.swift): the SV field's edge and the hue
/// strip's length, in points.
const FIELD_SIZE: f32 = 256.0;
/// `stripWidth`: the hue bar's width, with `padding(.horizontal, 7)` around it.
const STRIP_WIDTH: f32 = 20.0;
const STRIP_PAD: f32 = 7.0;
/// The SV field's marker: a 12pt circle ringed white over a black rim
/// (lineWidth 1.5 white, 0.75 black padded -0.75).
const SV_MARKER: f32 = 12.0;
/// The hue arrows: 7×10 triangles flanking the strip.
const HUE_ARROW_W: f32 = 7.0;
const HUE_ARROW_H: f32 = 10.0;
/// Gradients are generated as bitmaps at twice their logical size and scaled
/// down on paint, staying sharp at 2× backing scale.
const BACKING_SCALE: f32 = 2.0;

/// The color picker panel as its own view entity (Swift
/// `ColorPickerSheet`'s edit state; ticket 41). Its widgets are the text
/// entries — the R/G/B channels and the hex field — the field and the hue
/// strip write the session's `color_picker.hsb` directly through the drags
/// below. Created by the shell's `open_color_picker_ui` /
/// `open_text_color_picker_ui`, dropped on close.
pub struct ColorPickerPanelView {
    session: Entity<EditorSession>,
    /// The root view (the window shell): the commit/close flow stays
    /// there (the OK/Cancel read the hex first, then close the session's
    /// picker).
    app: WeakEntity<ProjectWorkspaceView>,
    /// The R/G/B channel entries (`fields`' channel rows): a draft of each
    /// channel's 0-255 integer; a valid edit writes `hsb.set_rgb` with the
    /// other two channels kept (Swift's binding `set`, :124-137).
    red: Entity<InputState>,
    green: Entity<InputState>,
    blue: Entity<InputState>,
    /// The hex entry (`ColorPickerSheet.swift:10`'s `@State hexDraft`); a
    /// well-formed entry overrides the field at commit.
    hex: Entity<InputState>,
}

impl ColorPickerPanelView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        color: PaletteColor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text_field = |value: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let red = text_field(channel_text(color.red), window, cx);
        let green = text_field(channel_text(color.green), window, cx);
        let blue = text_field(channel_text(color.blue), window, cx);
        let hex = text_field(color.hex(), window, cx);
        ColorPickerPanelView {
            session,
            app,
            red,
            green,
            blue,
            hex,
        }
    }

    /// The hex entry's text, for the shell's commit (the one panel read from
    /// outside this view — the commit reads the hex before closing).
    pub(crate) fn hex_text(&self, cx: &gpui_kit::App) -> String {
        self.hex.read(cx).value().to_string()
    }

    /// Press on the SV field: start its drag and jump to the pointer
    /// (`DragGesture(minimumDistance: 0)` on `saturationBrightnessField`).
    fn sv_drag_begin(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.default_global::<PickerFieldState>().drag = Some(PickerDrag::Sv);
        self.sv_drag(event.position, window, cx);
    }

    /// Press on the hue strip: start its drag and jump to the pointer
    /// (`DragGesture(minimumDistance: 0)` on `hueStrip`).
    fn hue_drag_begin(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.default_global::<PickerFieldState>().drag = Some(PickerDrag::Hue);
        self.hue_drag(event.position, window, cx);
    }

    /// Dragging on the SV field sets saturation from x and brightness from
    /// the flipped y (ColorPickerSheet.swift:64-67).
    fn sv_drag(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = cx
            .try_global::<PickerFieldState>()
            .and_then(|state| state.sv_bounds)
        else {
            return;
        };
        let (s, v) = sv_from_position(position, bounds);
        self.picker_apply(
            |hsb| {
                hsb.saturation = s;
                hsb.brightness = v;
            },
            window,
            cx,
        );
    }

    /// Dragging on the hue strip sets hue from the flipped y
    /// (ColorPickerSheet.swift:89-91).
    fn hue_drag(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = cx
            .try_global::<PickerFieldState>()
            .and_then(|state| state.hue_bounds)
        else {
            return;
        };
        let hue = hue_from_position(position, bounds);
        self.picker_apply(|hsb| hsb.hue = hue, window, cx);
    }

    /// The one path every picker edit takes — the field and hue strip
    /// drags (the former sliders' subscription): mutate `color_picker.hsb`,
    /// preview an effect's color live (ColorPalette.swift:113-116), keep
    /// the hex field on the working color when it is not being edited
    /// (Swift's `onChange(of: color)`), then redraw.
    fn picker_apply(
        &mut self,
        apply: impl FnOnce(&mut PickerHSB),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_effect = matches!(
            self.session
                .read(cx)
                .color_picker
                .as_ref()
                .map(|picker| &picker.target),
            Some(ColorPickerTarget::Effect { .. })
        );
        self.session.update(cx, |session, _| {
            if let Some(picker) = session.color_picker.as_mut() {
                apply(&mut picker.hsb);
            }
            if is_effect {
                session.preview_effect_color();
            }
        });
        let color = self
            .session
            .read(cx)
            .color_picker
            .as_ref()
            .map(|picker| picker.color());
        // The text entries ride the working color while not being edited —
        // Swift's `onChange(of: color)` for the hex draft and the channel
        // rows' value bindings. A focused entry is the user's draft, left
        // alone (except on a hex commit, which normalizes it). `set_value`
        // emits no `InputEvent::Change` (gpui-base sets `emit_events`
        // false), so a write never re-enters the subscriptions.
        for (field, text) in [
            (&self.red, color.as_ref().map(|c| channel_text(c.red))),
            (&self.green, color.as_ref().map(|c| channel_text(c.green))),
            (&self.blue, color.as_ref().map(|c| channel_text(c.blue))),
            (&self.hex, color.as_ref().map(|c| c.hex())),
        ] {
            let Some(text) = text else {
                continue;
            };
            let focused = field.read(cx).focus_handle(cx).is_focused(window);
            if focused || field.read(cx).value() == text.as_str() {
                continue;
            }
            field.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.touch(cx);
    }

    /// The panel's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on the shell (ticket 39's field audit).
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }
}

impl Render for ColorPickerPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let picker = self.session.read(cx).color_picker.as_ref();
        let Some(picker) = picker else {
            return gpui_kit::div().into_any_element();
        };
        let color = picker.color();
        let hsb = picker.hsb;
        let red = self.red.clone();
        let green = self.green.clone();
        let blue = self.blue.clone();
        let hex = self.hex.clone();

        // The gradient bitmaps: one per working hue for the field, one strip
        // shared by every open (both at 2× backing).
        let sv_image = prepare_sv_image(cx, hsb.hue);
        let strip_image = prepare_strip_image(cx);

        ColorPickerSheet::new(
            color,
            red,
            green,
            blue,
            hex,
            SvField::new(hsb, sv_image)
                .with_on_press(panel_on_press(&this, |panel, event, window, cx| {
                    panel.sv_drag_begin(event, window, cx);
                }))
                .with_on_drag(panel_on_drag_move(&this, |panel, event, window, cx| {
                    panel.sv_drag(event.position, window, cx);
                })),
            HueStrip::new(hsb, strip_image)
                .with_on_press(panel_on_press(&this, |panel, event, window, cx| {
                    panel.hue_drag_begin(event, window, cx);
                }))
                .with_on_drag(panel_on_drag_move(&this, |panel, event, window, cx| {
                    panel.hue_drag(event.position, window, cx);
                })),
        )
        .with_on_ok(app_on_click(&self.app, |root, _, _, cx| {
            root.close_color_picker_ui(true, cx)
        }))
        .with_on_cancel(app_on_click(&self.app, |root, _, _, cx| {
            root.close_color_picker_ui(false, cx)
        }))
        .into_any_element()
    }
}

/// The picker panel's body (`ColorPickerSheet`'s layout): the SV field, the
/// hue strip and the right column with preview, OK/Cancel and hex entry.
/// The panel entity renders it (ticket 41); the OK/Cancel and the title-bar
/// ✕ route through `close_color_picker_ui`'s commit flag.
#[derive(IntoElement)]
pub struct ColorPickerSheet {
    color: PaletteColor,
    red: Entity<InputState>,
    green: Entity<InputState>,
    blue: Entity<InputState>,
    hex: Entity<InputState>,
    sv_field: SvField,
    hue_strip: HueStrip,
    on_ok: ClickHandler,
    on_cancel: ClickHandler,
}

impl ColorPickerSheet {
    pub fn new(
        color: PaletteColor,
        red: Entity<InputState>,
        green: Entity<InputState>,
        blue: Entity<InputState>,
        hex: Entity<InputState>,
        sv_field: SvField,
        hue_strip: HueStrip,
    ) -> Self {
        ColorPickerSheet {
            color,
            red,
            green,
            blue,
            hex,
            sv_field,
            hue_strip,
            on_ok: app_on_click_stub(),
            on_cancel: app_on_click_stub(),
        }
    }

    pub fn with_on_ok(mut self, on_ok: ClickHandler) -> Self {
        self.on_ok = on_ok;
        self
    }

    pub fn with_on_cancel(mut self, on_cancel: ClickHandler) -> Self {
        self.on_cancel = on_cancel;
        self
    }
}

/// The no-op click handler the component's builder starts from.
fn app_on_click_stub() -> ClickHandler {
    std::rc::Rc::new(|_: &gpui_kit::ClickEvent, _, _| {})
}

impl RenderOnce for ColorPickerSheet {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_ok = self.on_ok;
        let on_cancel = self.on_cancel;
        let value = ((self.color.red * 255.0).round() as u32) << 16
            | ((self.color.green * 255.0).round() as u32) << 8
            | (self.color.blue * 255.0).round() as u32;

        // The right column (`VStack(width: 180, height: fieldSize)`): preview
        // and OK/Cancel on top, hex entry below.
        let side = gpui_kit::div()
            .w(px(180.0))
            .h(px(FIELD_SIZE))
            .flex()
            .flex_col()
            .items_start()
            .child(
                gpui_kit::div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap_4()
                    .child(
                        gpui_kit::div()
                            .w(px(64.0))
                            .h(px(64.0))
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(rgba(0x00000099))
                            .bg(gpui_kit::rgb(value)),
                    )
                    .child(
                        gpui_kit::div()
                            .w(px(90.0))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                Button::new("picker-ok")
                                    .label("OK")
                                    .small()
                                    .primary()
                                    .on_click(move |event, window, cx| on_ok(event, window, cx)),
                            )
                            .child(
                                Button::new("picker-cancel")
                                    .label("Cancel")
                                    .small()
                                    .on_click(move |event, window, cx| {
                                        on_cancel(event, window, cx)
                                    }),
                            ),
                    ),
            )
            .child(gpui_kit::div().h(px(12.0)))
            // The fields grid (`fields`, :104-119): the R/G/B channel rows
            // and the hex entry — 14pt labels, 52pt (hex 84pt) entries,
            // 8pt horizontal and 6pt vertical spacing.
            .child(
                gpui_kit::div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(channel_row("R", self.red, 52.0))
                    .child(channel_row("G", self.green, 52.0))
                    .child(channel_row("B", self.blue, 52.0))
                    .child(channel_row("#", self.hex, 84.0)),
            )
            // `Text("Click the canvas to sample")` (:37-38): a caption in
            // the secondary color, 8pt above it.
            .child(
                text_label("Click the canvas to sample", true)
                    .text_size(px(11.0))
                    .pt_2(),
            );

        // Swift's `HStack(alignment: .top, spacing: 14)` with 20pt padding.
        gpui_kit::div()
            .flex()
            .flex_row()
            .gap_3p5()
            .p_5()
            .child(self.sv_field)
            .child(self.hue_strip)
            .child(side)
    }
}

/// One fields-grid row (`channelRow`/the hex `GridRow`): the 14pt leading
/// label and the entry at the row's width.
fn channel_row(label: &'static str, field: Entity<InputState>, width: f32) -> gpui_kit::Div {
    gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            gpui_kit::div()
                .w(px(14.0))
                .flex_shrink_0()
                .child(text_label(label, false)),
        )
        .child(gpui_kit::div().w(px(width)).child(field))
}

/// A channel entry's text: the 0-255 integer Swift's binding get shows
/// (`Int((channel * 255).rounded())`).
fn channel_text(channel: CGFloat) -> String {
    format!("{}", (channel * 255.0).round() as i64)
}

/// The saturation/brightness field (`ColorPickerSheet`'s
/// `saturationBrightnessField`): the gradient canvas sized `fieldSize`²,
/// a press-capture drag, and the double-ringed marker at the working s/v.
#[derive(IntoElement)]
pub struct SvField {
    hsb: PickerHSB,
    image: Arc<RenderImage>,
    on_press: PressHandler,
    on_drag: DragMoveHandler,
}

impl SvField {
    pub fn new(hsb: PickerHSB, image: Arc<RenderImage>) -> Self {
        SvField {
            hsb,
            image,
            on_press: std::rc::Rc::new(|_, _, _| {}),
            on_drag: std::rc::Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_press(mut self, on_press: PressHandler) -> Self {
        self.on_press = on_press;
        self
    }

    pub fn with_on_drag(mut self, on_drag: DragMoveHandler) -> Self {
        self.on_drag = on_drag;
        self
    }
}

impl RenderOnce for SvField {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_press = self.on_press;
        let on_drag = self.on_drag;
        gpui_kit::div()
            .id("picker-sv-field")
            .w(px(FIELD_SIZE))
            .h(px(FIELD_SIZE))
            .flex_shrink_0()
            .border_1()
            .border_color(rgba(0x00000099))
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| on_press(event, window, cx),
            )
            .child(sv_field_canvas(self.image, self.hsb, on_drag))
    }
}

fn sv_field_canvas(
    image: Arc<RenderImage>,
    hsb: PickerHSB,
    on_drag: DragMoveHandler,
) -> gpui_kit::AnyElement {
    canvas(
        // Layout: record the field's window bounds for hit testing.
        move |bounds, _window, cx| {
            cx.default_global::<PickerFieldState>().sv_bounds = Some(bounds);
        },
        move |bounds, _prepaint, window, _cx| {
            // The bitmap was generated at 2×; painting it into the logical
            // bounds scales it down sharply.
            let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
            paint_sv_marker(window, bounds, hsb);
            install_sv_drag_events(&on_drag, window);
        },
    )
    .size_full()
    .into_any_element()
}

/// The hue strip (`ColorPickerSheet`'s `hueStrip`): the 34pt-wide frame
/// around the 20pt bar (7pt padding each side for the arrow markers), a
/// press-capture drag, the 360→0 downward gradient and the arrow pair at
/// the working hue.
#[derive(IntoElement)]
pub struct HueStrip {
    hsb: PickerHSB,
    image: Arc<RenderImage>,
    on_press: PressHandler,
    on_drag: DragMoveHandler,
}

impl HueStrip {
    pub fn new(hsb: PickerHSB, image: Arc<RenderImage>) -> Self {
        HueStrip {
            hsb,
            image,
            on_press: std::rc::Rc::new(|_, _, _| {}),
            on_drag: std::rc::Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_press(mut self, on_press: PressHandler) -> Self {
        self.on_press = on_press;
        self
    }

    pub fn with_on_drag(mut self, on_drag: DragMoveHandler) -> Self {
        self.on_drag = on_drag;
        self
    }
}

impl RenderOnce for HueStrip {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_press = self.on_press;
        let on_drag = self.on_drag;
        gpui_kit::div()
            .id("picker-hue-strip")
            .w(px(STRIP_WIDTH + STRIP_PAD * 2.0))
            .h(px(FIELD_SIZE))
            .flex_shrink_0()
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| on_press(event, window, cx),
            )
            .child(hue_strip_canvas(self.image, self.hsb, on_drag))
    }
}

fn hue_strip_canvas(
    image: Arc<RenderImage>,
    hsb: PickerHSB,
    on_drag: DragMoveHandler,
) -> gpui_kit::AnyElement {
    canvas(
        move |bounds, _window, cx| {
            cx.default_global::<PickerFieldState>().hue_bounds = Some(bounds);
        },
        move |bounds, _prepaint, window, _cx| {
            // The bar sits inside the frame's horizontal padding; the arrows
            // live in the padding itself.
            let bar = Bounds::<Pixels>::new(
                point(
                    px(f32::from(bounds.origin.x) + STRIP_PAD),
                    px(f32::from(bounds.origin.y)),
                ),
                size(
                    px(f32::from(bounds.size.width) - STRIP_PAD * 2.0),
                    px(f32::from(bounds.size.height)),
                ),
            );
            let _ = window.paint_image(bar, bar, Corners::default(), image, 0, false);
            paint_outline(window, bar, rgba(0x00000099));
            paint_hue_arrows(window, bounds, hsb.hue);
            install_hue_drag_events(&on_drag, window);
        },
    )
    .size_full()
    .into_any_element()
}

// MARK: Drag plumbing (`canvas.rs`'s window-level drag events pattern)

/// Window-level move/up handlers so an in-progress drag keeps following the
/// pointer outside its control, as a SwiftUI DragGesture does. Each control
/// installs a handler that acts only while its own drag is live, so the two
/// never double-apply.
fn install_sv_drag_events(on_drag: &DragMoveHandler, window: &mut Window) {
    let on_drag = on_drag.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, _phase, window, cx| {
        let dragging = cx
            .try_global::<PickerFieldState>()
            .is_some_and(|state| state.drag == Some(PickerDrag::Sv));
        if dragging {
            on_drag(event, window, cx);
        }
    });
    window.on_mouse_event(move |_event: &MouseUpEvent, _phase, _window, cx| {
        let state = cx.default_global::<PickerFieldState>();
        if state.drag == Some(PickerDrag::Sv) {
            state.drag = None;
        }
    });
}

fn install_hue_drag_events(on_drag: &DragMoveHandler, window: &mut Window) {
    let on_drag = on_drag.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, _phase, window, cx| {
        let dragging = cx
            .try_global::<PickerFieldState>()
            .is_some_and(|state| state.drag == Some(PickerDrag::Hue));
        if dragging {
            on_drag(event, window, cx);
        }
    });
    window.on_mouse_event(move |_event: &MouseUpEvent, _phase, _window, cx| {
        let state = cx.default_global::<PickerFieldState>();
        if state.drag == Some(PickerDrag::Hue) {
            state.drag = None;
        }
    });
}

/// Which control an in-progress drag started on; a drag owns the pointer
/// until mouse-up even when it leaves its control, like SwiftUI's gesture.
#[derive(Clone, Copy, PartialEq)]
enum PickerDrag {
    Sv,
    Hue,
}

/// The picker's layout→event bridge and live-drag channel: the canvas
/// bounds are written during layout and read by the pointer handlers (the
/// same bridge `CurvesCanvasLayout` provides the curves canvas), and `drag`
/// marks which control owns the pointer until mouse-up. A module-private
/// gpui global is the explicit owner — window-level handlers cannot reach
/// element state (ADR 0002 keeps this bridge; ticket 41).
#[derive(Default)]
struct PickerFieldState {
    sv_bounds: Option<Bounds<Pixels>>,
    hue_bounds: Option<Bounds<Pixels>>,
    drag: Option<PickerDrag>,
}

impl Global for PickerFieldState {}

/// The gradient bitmaps' cache, shared by every open picker (a sanctioned
/// shared service: regenerating the strip per frame would redraw identical
/// gradients every render).
#[derive(Default)]
struct GradientCache {
    /// Integer hue the cached SV bitmap was generated for.
    sv_key: i32,
    sv: Option<Arc<RenderImage>>,
    strip: Option<Arc<RenderImage>>,
}

impl Global for GradientCache {}

/// The SV field bitmap, cached by hue: dragging s/v reuses it, moving through
/// hues regenerates at most once per degree.
fn prepare_sv_image(cx: &mut App, hue: f64) -> Arc<RenderImage> {
    let key = sv_cache_key(hue);
    let cache = cx.default_global::<GradientCache>();
    if cache.sv.is_none() || cache.sv_key != key {
        let (width, height) = backing_size(FIELD_SIZE, FIELD_SIZE);
        cache.sv = Some(bgra_image(
            sv_field_bitmap(key as f64, width as usize, height as usize),
            width,
            height,
        ));
        cache.sv_key = key;
    }
    cache.sv.clone().expect("just populated")
}

/// The hue strip bitmap; its gradient is the same for every open, so it is
/// generated once.
fn prepare_strip_image(cx: &mut App) -> Arc<RenderImage> {
    let cache = cx.default_global::<GradientCache>();
    if cache.strip.is_none() {
        let (width, height) = backing_size(STRIP_WIDTH, FIELD_SIZE);
        cache.strip = Some(bgra_image(
            hue_strip_bitmap(width as usize, height as usize),
            width,
            height,
        ));
    }
    cache.strip.clone().expect("just populated")
}

fn backing_size(logical_w: f32, logical_h: f32) -> (u32, u32) {
    (
        (logical_w * BACKING_SCALE).round() as u32,
        (logical_h * BACKING_SCALE).round() as u32,
    )
}

/// GPUI sprites are BGRA (`state.rs`'s `bgra_render_image` does the same
/// channel swap on core bitmaps).
fn bgra_image(data: Vec<u8>, width: u32, height: u32) -> Arc<RenderImage> {
    let buffer = image::ImageBuffer::from_raw(width, height, data).expect("sizes match data");
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}

// MARK: Painting

/// The field's marker: a white 1.5pt ring inside a 0.75pt black rim, centered
/// on the working s/v (`Circle().strokeBorder(.white…).background(…black…)`,
/// 12pt).
fn paint_sv_marker(window: &mut Window, bounds: Bounds<Pixels>, hsb: PickerHSB) {
    let (x, y) = sv_marker_center(hsb.saturation, hsb.brightness, bounds);
    let center = point(px(x), px(y));
    let rim = SV_MARKER / 2.0 + 0.75;
    for (radius, width, color) in [
        (rim, 0.75, rgba(0x000000ff)),
        (SV_MARKER / 2.0, 1.5, rgba(0xffffffff)),
    ] {
        let mut builder = PathBuilder::stroke(px(width));
        builder.add_polygon(&circle_points(center, radius), true);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
}

/// The strip's marker: a pair of 7×10 triangles pointing into the bar from
/// the padding, at the working hue's row (`HueArrow`, filled `.primary`).
fn paint_hue_arrows(window: &mut Window, bounds: Bounds<Pixels>, hue: f64) {
    let y = hue_marker_y(hue, bounds);
    let left_x = f32::from(bounds.origin.x);
    let right_x = f32::from(bounds.origin.x) + f32::from(bounds.size.width);
    let half = HUE_ARROW_H / 2.0;
    let left = [
        point(px(left_x), px(y - half)),
        point(px(left_x + HUE_ARROW_W), px(y)),
        point(px(left_x), px(y + half)),
    ];
    let right = [
        point(px(right_x), px(y - half)),
        point(px(right_x - HUE_ARROW_W), px(y)),
        point(px(right_x), px(y + half)),
    ];
    for arrow in [left, right] {
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&arrow, true);
        if let Ok(path) = builder.build() {
            window.paint_path(path, rgba(0xffffffff));
        }
    }
}

/// A 1pt inset outline (`strokeBorder`), as four quads.
fn paint_outline(window: &mut Window, bounds: Bounds<Pixels>, color: gpui_kit::Rgba) {
    let x = f32::from(bounds.origin.x);
    let y = f32::from(bounds.origin.y);
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    for rect in [
        Bounds::<Pixels>::new(point(px(x), px(y)), size(px(w), px(1.0))),
        Bounds::<Pixels>::new(point(px(x), px(y + h - 1.0)), size(px(w), px(1.0))),
        Bounds::<Pixels>::new(point(px(x), px(y)), size(px(1.0), px(h))),
        Bounds::<Pixels>::new(point(px(x + w - 1.0), px(y)), size(px(1.0), px(h))),
    ] {
        window.paint_quad(gpui_kit::fill(rect, color));
    }
}

// MARK: Geometry (pure, hit tests and markers)

/// Window position inside the SV field → `(saturation, brightness)`, y
/// flipped: left→0 right→1, top→1 bottom→0, clamped
/// (`saturationBrightnessField`'s DragGesture inverse).
pub fn sv_from_position(position: Point<Pixels>, bounds: Bounds<Pixels>) -> (f64, f64) {
    let width = f32::from(bounds.size.width).max(1.0) as f64;
    let height = f32::from(bounds.size.height).max(1.0) as f64;
    let x = (f32::from(position.x) - f32::from(bounds.origin.x)) as f64;
    let y = (f32::from(position.y) - f32::from(bounds.origin.y)) as f64;
    (
        (x / width).clamp(0.0, 1.0),
        (1.0 - y / height).clamp(0.0, 1.0),
    )
}

/// Window position inside the hue strip → hue degrees: 360 at the top down to
/// 0 at the bottom (`hue = (1 - y/fieldSize) * 360`), clamped.
pub fn hue_from_position(position: Point<Pixels>, bounds: Bounds<Pixels>) -> f64 {
    let height = f32::from(bounds.size.height).max(1.0) as f64;
    let y = (f32::from(position.y) - f32::from(bounds.origin.y)) as f64;
    (1.0 - (y / height).clamp(0.0, 1.0)) * 360.0
}

/// s/v → the marker's window center (`position(x: s·fieldSize,
/// y: (1-b)·fieldSize)`).
fn sv_marker_center(s: f64, v: f64, bounds: Bounds<Pixels>) -> (f32, f32) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    (
        f32::from(bounds.origin.x) + s as f32 * width,
        f32::from(bounds.origin.y) + (1.0 - v as f32) * height,
    )
}

/// hue → the arrows' window y (`markerY = (1 - hue/360) * fieldSize`).
fn hue_marker_y(hue: f64, bounds: Bounds<Pixels>) -> f32 {
    f32::from(bounds.origin.y) + (1.0 - hue / 360.0) as f32 * f32::from(bounds.size.height)
}

/// A closed polygon approximating a circle, starting at its rightmost point.
fn circle_points(center: Point<Pixels>, radius: f32) -> Vec<Point<Pixels>> {
    const SEGMENTS: usize = 24;
    (0..SEGMENTS)
        .map(|step| {
            let angle = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            point(
                px(f32::from(center.x) + radius * angle.cos()),
                px(f32::from(center.y) + radius * angle.sin()),
            )
        })
        .collect()
}

// MARK: Gradient bitmaps (pure)

/// The cache key for the SV field bitmap: the working hue rounded to a degree
/// and wrapped into 0…359 (360° and 0° are the same red).
fn sv_cache_key(hue: f64) -> i32 {
    (hue.round() as i64).rem_euclid(360) as i32
}

/// One row-major BGRA byte per channel; red sits at offset 2.
fn write_bgra(data: &mut Vec<u8>, red: f64, green: f64, blue: f64) {
    data.push((blue * 255.0).round() as u8);
    data.push((green * 255.0).round() as u8);
    data.push((red * 255.0).round() as u8);
    data.push(255);
}

/// The SV field, per `saturationBrightnessField`: a horizontal gradient from
/// white to the working hue's full-saturation color, overlaid by a vertical
/// gradient from clear to black. Composing the two per pixel gives
/// `column_color × (1 - y/(h-1))`, so the corners are white, pure hue, black,
/// black.
pub fn sv_field_bitmap(hue: f64, width: usize, height: usize) -> Vec<u8> {
    let pure = PickerHSB::new(hue, 1.0, 1.0).rgb();
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    // Per column: white → pure hue across x (the bottom gradient).
    let mut columns = Vec::with_capacity(width * 3);
    for x in 0..width {
        let t = if width <= 1 {
            0.0
        } else {
            x as f64 / (width - 1) as f64
        };
        columns.push(lerp(1.0, pure.red, t));
        columns.push(lerp(1.0, pure.green, t));
        columns.push(lerp(1.0, pure.blue, t));
    }
    // Per row: fade by brightness down y (the top gradient).
    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let v = if height <= 1 {
            1.0
        } else {
            1.0 - y as f64 / (height - 1) as f64
        };
        for column in columns.chunks_exact(3) {
            write_bgra(&mut data, column[0] * v, column[1] * v, column[2] * v);
        }
    }
    data
}

/// One stop color of the strip gradient: the hue at full saturation and
/// brightness (`PickerHSB(hue:…, saturation: 1, brightness: 1).rgb`).
fn strip_stop(hue: f64) -> (f64, f64, f64) {
    let color = PickerHSB::new(hue, 1.0, 1.0).rgb();
    (color.red, color.green, color.blue)
}

/// The strip's color at `t` (0 = top row, 1 = bottom): SwiftUI's
/// `LinearGradient` interpolates each sRGB channel between the evenly spaced
/// stops `stride(from: 360, through: 0, by: -60)` — in color space, not
/// around the hue wheel.
pub fn strip_color_at(t: f64) -> (f64, f64, f64) {
    let stops: Vec<_> = (0..=6)
        .map(|step| strip_stop(360.0 - step as f64 * 60.0))
        .collect();
    let scaled = (t.clamp(0.0, 1.0) * 6.0) as usize;
    let index = scaled.min(5);
    let f = (t.clamp(0.0, 1.0) * 6.0) - index as f64;
    let (a, b) = (stops[index], stops[index + 1]);
    (
        a.0 + (b.0 - a.0) * f,
        a.1 + (b.1 - a.1) * f,
        a.2 + (b.2 - a.2) * f,
    )
}

/// The hue bar: one column of the strip gradient repeated across the width,
/// 360 at the top row through 0 at the bottom.
pub fn hue_strip_bitmap(width: usize, height: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let t = if height <= 1 {
            0.0
        } else {
            y as f64 / (height - 1) as f64
        };
        let (red, green, blue) = strip_color_at(t);
        for _ in 0..width {
            write_bgra(&mut data, red, green, blue);
        }
    }
    data
}

impl ProjectWorkspaceView {
    /// The palette swatch's picker opener (`ColorPickerSheet.swift:167-181`):
    /// arm the session's picker, put the panel entity (with the color's hex
    /// prefilled) in the registry, and show the shared floating panel —
    /// first open centered on the canvas, afterwards wherever it was last
    /// left.
    pub fn open_color_picker_ui(
        &mut self,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .is_some()
            || !self
                .workspace
                .read(cx)
                .current()
                .session
                .read(cx)
                .can_edit_palette()
        {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.open_color_picker(background))
        });
        let color = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .as_ref()
            .map(|picker| picker.color());
        let Some(color) = color else {
            return;
        };
        let title = self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .color_picker
            .as_ref()
            .map(|picker| picker.target.title())
            .unwrap_or_default();
        let session = self.workspace.read(cx).current().session.clone();
        let app = cx.entity().downgrade();
        // A stale drag from a previous open must not survive.
        cx.default_global::<PickerFieldState>().drag = None;
        let panel = cx.new(|cx| ColorPickerPanelView::new(session, app, color, window, cx));
        self.panel_views.insert(
            crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL,
            crate::ui::floating_panel::PanelContent::ColorPicker(panel),
        );
        self.panels.show(
            crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL,
            title,
            crate::ui::floating_panel::PanelPlacement::Automatic,
        );
        self.sync_panel_guard(cx);
        self.touch(cx);
    }

    /// The color picker's OK (commit) / Cancel / title-bar ✕ routes
    /// (`ColorPickerSheet`'s commitHex + `onClose` = Cancel): a well-formed
    /// hex entry overrides the field on commit, then the picker session
    /// closes either way.
    pub fn close_color_picker_ui(&mut self, commit: bool, cx: &mut Context<Self>) {
        if commit {
            let text = match self
                .panel_views
                .get(crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL)
            {
                Some(crate::ui::floating_panel::PanelContent::ColorPicker(panel)) => {
                    panel.read(cx).hex_text(cx)
                }
                _ => String::new(),
            };
            if let Some(color) = PaletteColor::from_hex(&text) {
                self.workspace.update(cx, |ws, cx| {
                    ws.current_mut().session.update(cx, |session, _| {
                        if let Some(picker) = session.color_picker.as_mut() {
                            picker.hsb = PickerHSB::from_rgb(color);
                        }
                    })
                });
            }
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.close_color_picker(commit))
        });
        self.panel_views
            .remove(crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL);
        self.panels
            .close(crate::ui::floating_panel::panel_names::COLOR_PICKER_PANEL);
        self.sync_panel_guard(cx);
        cx.default_global::<PickerFieldState>().drag = None;
        self.touch(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ColorPickerPanelView, circle_points, hue_from_position, hue_marker_y, hue_strip_bitmap,
        strip_color_at, sv_cache_key, sv_field_bitmap, sv_from_position, sv_marker_center,
    };
    use crate::state::ProjectWorkspaceView;
    use compositor_core::document::PickerHSB;
    use compositor_core::session::EditorSession;
    use gpui_kit::{Bounds, Pixels, point, px, size};

    fn field_bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.0), px(20.0)), size(px(256.0), px(256.0)))
    }

    fn strip_bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(0.0), px(0.0)), size(px(34.0), px(256.0)))
    }

    #[test]
    fn sv_hit_test_matches_swift_axes() {
        let bounds = field_bounds();
        // Left edge is saturation 0; right is 1. Top is brightness 1; the
        // bottom is 0. Off-canvas presses clamp.
        assert_eq!(
            sv_from_position(point(px(10.0), px(20.0)), bounds),
            (0.0, 1.0)
        );
        assert_eq!(
            sv_from_position(point(px(266.0), px(276.0)), bounds),
            (1.0, 0.0)
        );
        assert_eq!(
            sv_from_position(point(px(138.0), px(148.0)), bounds),
            (0.5, 0.5)
        );
        assert_eq!(
            sv_from_position(point(px(-500.0), px(-500.0)), bounds),
            (0.0, 1.0)
        );
        assert_eq!(
            sv_from_position(point(px(500.0), px(500.0)), bounds),
            (1.0, 0.0)
        );
    }

    #[test]
    fn sv_hit_test_and_marker_round_trip() {
        let bounds = field_bounds();
        for (s, v) in [(0.0, 0.0), (0.25, 0.75), (0.5, 0.5), (1.0, 1.0)] {
            let (x, y) = sv_marker_center(s, v, bounds);
            let (back_s, back_v) = sv_from_position(point(px(x), px(y)), bounds);
            assert!((back_s - s).abs() < 1e-6, "s = {back_s}");
            assert!((back_v - v).abs() < 1e-6, "v = {back_v}");
        }
    }

    #[test]
    fn hue_hit_test_runs_360_to_0_downward() {
        let bounds = strip_bounds();
        // Swift: hue = (1 - y/fieldSize) * 360, clamped; x is ignored.
        assert_eq!(hue_from_position(point(px(3.0), px(0.0)), bounds), 360.0);
        assert_eq!(hue_from_position(point(px(30.0), px(128.0)), bounds), 180.0);
        assert_eq!(hue_from_position(point(px(17.0), px(256.0)), bounds), 0.0);
        assert_eq!(hue_from_position(point(px(17.0), px(-40.0)), bounds), 360.0);
        assert_eq!(hue_from_position(point(px(17.0), px(400.0)), bounds), 0.0);
    }

    #[test]
    fn hue_marker_y_is_the_hit_tests_inverse() {
        let bounds = strip_bounds();
        for hue in [0.0, 60.0, 180.0, 300.0, 360.0] {
            let y = hue_marker_y(hue, bounds);
            let back = hue_from_position(point(px(17.0), px(y)), bounds);
            // The marker's y is an f32; a 60° round trip drifts by ~1e-5°.
            assert!((back - hue).abs() < 1e-3, "hue {hue} → y {y} → {back}");
        }
    }

    #[test]
    fn sv_bitmap_corners_are_white_hue_black_black() {
        // 512×512 = the 2× backing of the 256pt field; BGRA rows.
        let (width, height) = (512usize, 512usize);
        let data = sv_field_bitmap(0.0, width, height);
        assert_eq!(data.len(), width * height * 4);
        let pixel =
            |x: usize, y: usize| (data[(y * width + x) * 4..(y * width + x) * 4 + 4]).to_vec();
        // Top left: white. Top right: pure red. Bottom row: black. (BGRA.)
        assert_eq!(pixel(0, 0), vec![255, 255, 255, 255]);
        assert_eq!(pixel(width - 1, 0), vec![0, 0, 255, 255]);
        assert_eq!(pixel(0, height - 1), vec![0, 0, 0, 255]);
        assert_eq!(pixel(width - 1, height - 1), vec![0, 0, 0, 255]);
        // Every pixel is opaque.
        assert!(data.chunks_exact(4).all(|px| px[3] == 255));
    }

    #[test]
    fn sv_bitmap_matches_the_hsb_conversion() {
        // The composed two-gradient result equals PickerHSB::rgb at the same
        // s/v within the 8-bit rounding step.
        let hue = 210.0;
        let (width, height) = (256usize, 256usize);
        let data = sv_field_bitmap(hue, width, height);
        for (x, y) in [
            (0usize, 0usize),
            (width - 1, 0),
            (width / 2, height / 2),
            (width - 1, height * 3 / 4),
        ] {
            let expected = PickerHSB::new(
                hue,
                x as f64 / (width - 1) as f64,
                1.0 - y as f64 / (height - 1) as f64,
            )
            .rgb();
            let px = &data[(y * width + x) * 4..(y * width + x) * 4 + 4];
            assert!(
                ((expected.blue * 255.0).round() - px[0] as f64).abs() <= 1.0,
                "b at {x},{y}"
            );
            assert!(
                ((expected.green * 255.0).round() - px[1] as f64).abs() <= 1.0,
                "g at {x},{y}"
            );
            assert!(
                ((expected.red * 255.0).round() - px[2] as f64).abs() <= 1.0,
                "r at {x},{y}"
            );
        }
    }

    #[test]
    fn strip_bitmap_is_red_at_both_ends_and_cyan_in_the_middle() {
        // 513 rows so the exact middle row (256) sits at t = 0.5.
        let (width, height) = (40usize, 513usize);
        let data = hue_strip_bitmap(width, height);
        assert_eq!(data.len(), width * height * 4);
        let pixel =
            |x: usize, y: usize| (data[(y * width + x) * 4..(y * width + x) * 4 + 4]).to_vec();
        // 360° at the top and 0° at the bottom are both red; the middle row
        // is 180° cyan. Columns are constant. (BGRA.)
        assert_eq!(pixel(0, 0), vec![0, 0, 255, 255]);
        assert_eq!(pixel(width - 1, 0), vec![0, 0, 255, 255]);
        assert_eq!(pixel(width / 2, height / 2), vec![255, 255, 0, 255]);
        assert_eq!(pixel(width / 2, height - 1), vec![0, 0, 255, 255]);
    }

    #[test]
    fn strip_color_interpolates_between_stops_in_srgb() {
        // Halfway between the 360° (red) and 300° (magenta) stops.
        let (r, g, b) = strip_color_at(1.0 / 12.0);
        assert!((r - 1.0).abs() < 1e-9);
        assert!(g.abs() < 1e-9);
        assert!((b - 0.5).abs() < 1e-9);
        // The endpoints land on their stops exactly.
        assert_eq!(strip_color_at(0.0), (1.0, 0.0, 0.0));
        assert_eq!(strip_color_at(1.0), (1.0, 0.0, 0.0));
        assert_eq!(strip_color_at(0.5), (0.0, 1.0, 1.0));
    }

    #[test]
    fn sv_cache_key_quantizes_the_hue_and_wraps() {
        assert_eq!(sv_cache_key(0.0), 0);
        assert_eq!(sv_cache_key(12.4), 12);
        assert_eq!(sv_cache_key(12.6), 13);
        // 359.6° rounds to 360°, the same red as 0°.
        assert_eq!(sv_cache_key(359.6), 0);
        assert_eq!(sv_cache_key(-0.2), 0);
    }

    #[test]
    fn circle_points_ring_the_center() {
        let center = point(px(100.0), px(50.0));
        let points = circle_points(center, 6.0);
        assert_eq!(points.len(), 24);
        // The first vertex sits a radius to the right; every vertex is a
        // radius away from the center.
        assert!((f32::from(points[0].x) - 106.0).abs() < 1e-4);
        assert!((f32::from(points[0].y) - 50.0).abs() < 1e-4);
        for vertex in &points {
            let dx = f32::from(vertex.x) - 100.0;
            let dy = f32::from(vertex.y) - 50.0;
            assert!((dx * dx + dy * dy - 36.0).abs() < 1e-3);
        }
    }

    /// The live input path, exercised end to end with real text input
    /// (gpui-kit `ColorPickerState`'s test shape): focusing a channel entry
    /// and typing re-enters `picker_apply` and moves the working color.
    #[gpui_kit::test]
    fn typing_in_a_channel_field_updates_the_working_color(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{AppContext as _, WeakEntity};
        cx.update(gpui_kit::init);

        let session = cx.new(|_| {
            let mut session = EditorSession::new();
            session.open_color_picker(false);
            session
        });
        let color = session.read_with(cx, |session, _| {
            session.color_picker.as_ref().expect("open").color()
        });
        let app = WeakEntity::<ProjectWorkspaceView>::new_invalid();
        let (panel, cx) = cx.add_window_view(|window, cx| {
            ColorPickerPanelView::new(session.clone(), app, color, window, cx)
        });

        let red = panel.read_with(cx, |panel, _| panel.red.clone());
        cx.update(|window, cx| {
            red.update(cx, |input, cx| {
                input.focus(window, cx);
                // What the focus handler does in a real window (`InputEvent::
                // Focus`): select the pre-filled draft so typing replaces it.
                // A headless draw never delivers that event, so the test
                // performs the selection itself.
                input.select_all(window, cx);
            });
        });
        // `input` sets `key_char`, the character path real keyboards take
        // (a bare `simulate_keystrokes` dispatches no text).
        cx.update(|window, cx| window.input("128", cx));

        let draft = red.read_with(cx, |input, _| input.value().to_string());
        session.read_with(cx, |session, _| {
            let picker = session.color_picker.as_ref().expect("open");
            assert!(
                (picker.color().red - 128.0 / 255.0).abs() < 1e-6,
                "red channel: {}, draft: {:?}",
                picker.color().red,
                draft
            );
        });
    }

    /// The hex entry follows gpui-kit `ColorPickerState`: a parseable draft
    /// previews as it is typed, and Return commits — a malformed one leaves
    /// the color alone and snaps the draft back.
    #[gpui_kit::test]
    fn typing_and_committing_hex_updates_the_working_color(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{AppContext as _, WeakEntity};
        cx.update(gpui_kit::init);

        let session = cx.new(|_| {
            let mut session = EditorSession::new();
            session.open_color_picker(false);
            session
        });
        let color = session.read_with(cx, |session, _| {
            session.color_picker.as_ref().expect("open").color()
        });
        let app = WeakEntity::<ProjectWorkspaceView>::new_invalid();
        let (panel, cx) = cx.add_window_view(|window, cx| {
            ColorPickerPanelView::new(session.clone(), app, color, window, cx)
        });

        let hex = panel.read_with(cx, |panel, _| panel.hex.clone());
        cx.update(|window, cx| {
            hex.update(cx, |input, cx| {
                input.focus(window, cx);
                input.select_all(window, cx);
            });
        });
        cx.update(|window, cx| window.input("ff0000", cx));

        session.read_with(cx, |session, _| {
            let picker = session.color_picker.as_ref().expect("open");
            assert!(
                (picker.color().red - 1.0).abs() < 1e-6,
                "live hex preview: {}",
                picker.color().red
            );
        });

        // A malformed draft reverts on Return without touching the color.
        // (Select-all first: backspacing through `ff0000` would legitimately
        // parse the `ff0` shorthand — Swift's `PaletteColor(hex:)` accepts
        // `RGB` too — and change the working color mid-way.)
        cx.update(|window, cx| {
            hex.update(cx, |input, cx| input.select_all(window, cx));
        });
        cx.update(|window, cx| window.input("nope", cx));
        cx.update(|window, cx| window.press("enter", cx));
        session.read_with(cx, |session, _| {
            let picker = session.color_picker.as_ref().expect("open");
            assert!((picker.color().red - 1.0).abs() < 1e-6, "nope is not red");
        });
        let draft = hex.read_with(cx, |input, _| input.value().to_string());
        assert_eq!(
            draft, "FF0000",
            "the draft snapped back to the working color"
        );
    }
}
