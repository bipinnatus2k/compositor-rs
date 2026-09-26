//! The modal sheets themselves: the centered overlay panel, per-sheet bodies,
//! and the OK/Cancel actions (including PNG export, .comp save and open).

use gpui_kit::{
    canvas, px, point, prelude::FluentBuilder as _, rgba, size, Bounds, Context, Entity,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, PathBuilder, Pixels, Point, SharedString, StatefulInteractiveElement, Styled,
    Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::{Disableable, Sizable};

use compositor_core::canvas::{CanvasSizeOptions, TrimBasedOn, TrimOptions};
use compositor_core::adjust::{CurvePoint, CurvesSettings, LevelsChannel};
use compositor_core::document::PaletteColor;
use compositor_core::effects::LayerEffectKind;
use compositor_core::filters::FilterKind;
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::Bitmap;
use compositor_core::session::SelectionAmountOperation;
use compositor_core::tools::brush::RasterEdit;

use crate::sheets::SliderRole;
use crate::state::{AppState, SheetKind};
use crate::ui::{dropdown, labeled_checkbox, text_label, MenuEntry};

/// Starting a drag on the curve canvas (`CurvesControls.swift`'s
/// `dragging == nil` branch).
#[derive(Debug, PartialEq)]
pub enum CurveDragBegin {
    /// The click landed on an existing point; drag it.
    Grab(usize),
    /// The click landed on empty canvas; a point was inserted (kept sorted
    /// by x) and the drag moves it.
    Added { points: Vec<CurvePoint>, index: usize },
    /// Nothing to drag: the 32-point cap, the x range, or a blocked x.
    None,
}

fn sheet_title(kind: SheetKind) -> &'static str {
    match kind {
        SheetKind::NewCanvas => "New Document",
        SheetKind::ImageSize => "Image Size",
        SheetKind::CanvasSize => "Canvas Size",
        SheetKind::Levels => "Levels",
        SheetKind::HueSaturation => "Hue/Saturation",
        SheetKind::Effects => "Layer Effects",
        SheetKind::Filter(FilterKind::GaussianBlur) => "Gaussian Blur",
        SheetKind::Filter(FilterKind::MotionBlur) => "Motion Blur",
        SheetKind::Filter(FilterKind::AddNoise) => "Add Noise",
        SheetKind::Filter(FilterKind::LensCorrection) => "Lens Correction",
        SheetKind::Filter(FilterKind::CameraRaw) => "Camera Raw Filter",
        SheetKind::Filter(FilterKind::Curves) => "Curves",
        SheetKind::Filter(_) => "Filter",
        SheetKind::Amount(SelectionAmountOperation::Feather) => "Feather Selection",
        SheetKind::Amount(SelectionAmountOperation::Expand) => "Expand Selection",
        SheetKind::Amount(SelectionAmountOperation::Contract) => "Contract Selection",
        SheetKind::ExportPng => "Export PNG",
        SheetKind::ExportJpeg => "Export JPEG",
        SheetKind::Import => "Import Images",
        SheetKind::SaveComp => "Save Project",
        SheetKind::Open => "Open",
        SheetKind::Trim => "Trim",
        SheetKind::About => "About Compositor",
        SheetKind::Shortcuts => "Keyboard Shortcuts",
    }
}

fn parse_input_usize(input: &Entity<InputState>, cx: &gpui_kit::App) -> Option<usize> {
    input.read(cx).value().trim().parse::<usize>().ok()
}

fn parse_input_f64(input: &Entity<InputState>, cx: &gpui_kit::App) -> Option<f64> {
    input.read(cx).value().trim().parse::<f64>().ok()
}

fn sheet_row(label: &str, control: impl gpui_kit::IntoElement) -> gpui_kit::Div {
    gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .py_0p5()
        .child(
            gpui_kit::div()
                .w(px(110.0))
                .flex_shrink_0()
                .child(text_label(label.to_string(), true)),
        )
        .child(control)
}

impl AppState {
    // MARK: Sheet rendering

    pub fn render_sheet_overlay(&mut self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let kind = self.sheet.as_ref()?.kind;

        let title_bar = gpui_kit::div()
            .flex()
            .flex_row()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(gpui_kit::rgb(0x4a4a4a))
            .child(text_label(sheet_title(kind), false).text_size(px(13.0)));

        let body = gpui_kit::div()
            .flex()
            .flex_col()
            .px_3()
            .py_2()
            .gap_1()
            .child(self.sheet_body(cx));

        let footer = gpui_kit::div()
            .flex()
            .flex_row()
            .justify_end()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(gpui_kit::rgb(0x4a4a4a))
            .child(
                Button::new("sheet-cancel")
                    .label("Cancel")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_sheet(cx))),
            )
            .child(
                Button::new("sheet-ok")
                    .label("OK")
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_sheet(window, cx);
                    })),
            );

        let overlay = gpui_kit::div()
            .absolute()
            .inset_0()
            .bg(rgba(0x00000099))
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .child(
                gpui_kit::div()
                    .w(px(470.0))
                    .flex()
                    .flex_col()
                    .rounded_md()
                    .border_1()
                    .border_color(gpui_kit::rgb(0x4a4a4a))
                    .bg(gpui_kit::rgb(0x2d2d2d))
                    .overflow_hidden()
                    .child(title_bar)
                    .child(body)
                    .child(footer),
            );
        Some(overlay.into_any_element())
    }

    fn sheet_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return gpui_kit::div().into_any_element(),
        };
        let mut body = gpui_kit::div().flex().flex_col().gap_1();

        match kind {
            SheetKind::NewCanvas => {
                let sheet = self.sheet.as_ref().unwrap();
                let preset = sheet.preset;
                let background = sheet.background;
                let width = sheet.width.clone();
                let height = sheet.height.clone();
                let name = sheet.name.clone();
                body = body
                    .child(sheet_row(
                        "Preset",
                        dropdown(
                            &app,
                            "new-preset",
                            match preset {
                                1 => "Photo 3000 \u{d7} 2000",
                                2 => "Web 1280 \u{d7} 720",
                                _ => "Default 1920 \u{d7} 1080",
                            },
                            px(180.0),
                            vec![
                                MenuEntry::new("Default 1920 \u{d7} 1080", {
                                    let width = width.clone();
                                    let height = height.clone();
                                    move |state, window, cx| {
                                        set_inputs(state, &width, &height, 1920, 1080, window, cx);
                                    }
                                }),
                                MenuEntry::new("Photo 3000 \u{d7} 2000", {
                                    let width = width.clone();
                                    let height = height.clone();
                                    move |state, window, cx| {
                                        set_inputs(state, &width, &height, 3000, 2000, window, cx);
                                    }
                                }),
                                MenuEntry::new("Web 1280 \u{d7} 720", {
                                    let width = width.clone();
                                    let height = height.clone();
                                    move |state, window, cx| {
                                        set_inputs(state, &width, &height, 1280, 720, window, cx);
                                    }
                                }),
                            ],
                        ),
                    ))
                    .child(sheet_row(
                        "Width",
                        Input::new(&width).id("new-width").w(px(120.0)),
                    ))
                    .child(sheet_row(
                        "Height",
                        Input::new(&height).id("new-height").w(px(120.0)),
                    ))
                    .child(sheet_row(
                        "Name",
                        Input::new(&name).id("new-name").w(px(180.0)),
                    ))
                    .child(sheet_row(
                        "Background",
                        dropdown(
                            &app,
                            "new-background",
                            match background {
                                1 => "Black",
                                2 => "Transparent",
                                _ => "White",
                            },
                            px(120.0),
                            vec![
                                MenuEntry::new("White", |state, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.background = 0;
                                    }
                                    state.notify_only(cx);
                                }),
                                MenuEntry::new("Black", |state, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.background = 1;
                                    }
                                    state.notify_only(cx);
                                }),
                                MenuEntry::new("Transparent", |state, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.background = 2;
                                    }
                                    state.notify_only(cx);
                                }),
                            ],
                        ),
                    ));
                if preset != 0 {
                    if let Some(sheet) = self.sheet.as_mut() {
                        sheet.preset = preset;
                    }
                }
            }
            SheetKind::ImageSize => {
                let sheet = self.sheet.as_ref().unwrap();
                let percent = sheet.percent;
                let link = sheet.link;
                body = body
                    .child(sheet_row(
                        "Width",
                        Input::new(&sheet.width).id("size-width").w(px(120.0)),
                    ))
                    .child(sheet_row(
                        "Height",
                        Input::new(&sheet.height).id("size-height").w(px(120.0)),
                    ))
                    .child(sheet_row(
                        "Unit",
                        labeled_checkbox(
                            &app,
                            "size-percent",
                            "Percent",
                            percent,
                            |state, checked, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.percent = checked;
                                }
                                state.notify_only(cx);
                            },
                        )
                        .into_any_element(),
                    ))
                    .child(sheet_row(
                        "Resolution",
                        gpui_kit::div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(Input::new(&sheet.resolution).id("size-resolution").w(px(80.0)))
                            .child(text_label("Pixels/Inch", true)),
                    ))
                    .child(sheet_row(
                        "Constrain",
                        labeled_checkbox(
                            &app,
                            "size-link",
                            "Link width and height (keep aspect)",
                            link,
                            |state, checked, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.link = checked;
                                }
                                state.notify_only(cx);
                            },
                        )
                        .into_any_element(),
                    ));
            }
            SheetKind::CanvasSize => {
                let sheet = self.sheet.as_ref().unwrap();
                let anchor = sheet.anchor;
                let relative = sheet.relative;
                body = body
                    .child(sheet_row(
                        "Width",
                        Input::new(&sheet.width).id("canvas-width").w(px(120.0)),
                    ))
                    .child(sheet_row(
                        "Height",
                        Input::new(&sheet.height).id("canvas-height").w(px(120.0)),
                    ))
                    .child(sheet_row(
                        "Relative",
                        labeled_checkbox(
                            &app,
                            "canvas-relative",
                            "Relative (add to current size)",
                            relative,
                            |state, checked, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.relative = checked;
                                }
                                state.notify_only(cx);
                            },
                        )
                        .into_any_element(),
                    ))
                    .child(sheet_row("Anchor", anchor_picker(anchor, cx)));
            }
            SheetKind::Levels => {
                let sheet = self.sheet.as_ref().unwrap();
                let channel = sheet.channel;
                let histogram = sheet.histogram;
                let sliders = sheet.sliders.clone();
                let preview = sheet.checkbox_a;
                body = body
                    .child(sheet_row(
                        "Preview",
                        labeled_checkbox(
                            &app,
                            "levels-preview",
                            "Preview",
                            preview,
                            |state, checked, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.checkbox_a = checked;
                                }
                                state.refresh_levels_preview(cx);
                            },
                        )
                        .into_any_element(),
                    ))
                    .child(sheet_row(
                        "Channel",
                        dropdown(
                            &app,
                            "levels-channel",
                            match channel {
                                1 => "Red",
                                2 => "Green",
                                3 => "Blue",
                                _ => "RGB",
                            },
                            px(90.0),
                            vec![
                                MenuEntry::new("RGB", |state, _, cx| set_channel(state, 0, cx)),
                                MenuEntry::new("Red", |state, _, cx| set_channel(state, 1, cx)),
                                MenuEntry::new("Green", |state, _, cx| set_channel(state, 2, cx)),
                                MenuEntry::new("Blue", |state, _, cx| set_channel(state, 3, cx)),
                            ],
                        ),
                    ))
                    .child(
                        gpui_kit::div()
                            .w_full()
                            .h(px(96.0))
                            .rounded_sm()
                            .overflow_hidden()
                            .child(histogram_view(histogram)),
                    )
                    .child(labeled_slider_sheet(&sliders, 0, "Input Black"))
                    .child(labeled_slider_sheet(&sliders, 1, "Gamma"))
                    .child(labeled_slider_sheet(&sliders, 2, "Input White"))
                    .child(labeled_slider_sheet(&sliders, 3, "Output Black"))
                    .child(labeled_slider_sheet(&sliders, 4, "Output White"))
                    .child(sheet_row(
                        "Auto",
                        gpui_kit::div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(
                                Button::new("levels-auto-contrast")
                                    .label("Contrast")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.levels_auto_mode(
                                            compositor_core::adjust::levels_auto::LevelsAuto::Contrast,
                                            window,
                                            cx,
                                        );
                                    }))
                                    .into_any_element(),
                            )
                            .child(
                                Button::new("levels-auto-color")
                                    .label("Color")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.levels_auto_mode(
                                            compositor_core::adjust::levels_auto::LevelsAuto::Color,
                                            window,
                                            cx,
                                        );
                                    }))
                                    .into_any_element(),
                            )
                            .child(
                                Button::new("levels-auto-neutral")
                                    .label("Neutral")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.levels_auto_mode(
                                            compositor_core::adjust::levels_auto::LevelsAuto::Neutral,
                                            window,
                                            cx,
                                        );
                                    }))
                                    .into_any_element(),
                            ),
                    ));
            }
            SheetKind::HueSaturation => {
                let sheet = self.sheet.as_ref().unwrap();
                let colorize = sheet.colorize;
                let sliders = sheet.sliders.clone();
                body = body
                    .child(labeled_slider_sheet(&sliders, 0, "Hue"))
                    .child(labeled_slider_sheet(&sliders, 1, "Saturation"))
                    .child(labeled_slider_sheet(&sliders, 2, "Lightness"))
                    .child(sheet_row(
                        "Colorize",
                        labeled_checkbox(
                            &app,
                            "hue-colorize",
                            "Colorize",
                            colorize,
                            |state, checked, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.colorize = checked;
                                }
                                state.apply_slider(SliderRole::Hue, 0.0, cx);
                            },
                        )
                        .into_any_element(),
                    ));
            }
            SheetKind::Effects => {
                let sheet = self.sheet.as_ref().unwrap();
                // The edited kind rides in `channel`; see build_sheet.
                let kind = LayerEffectKind::ALL
                    .get(sheet.channel)
                    .copied()
                    .unwrap_or(LayerEffectKind::Stroke);
                let inside = sheet.checkbox_a;
                let sliders = sheet.sliders.clone();
                let color = self.session.editing_effects().color(kind);
                body = body.child(text_label(kind.label(), false).text_size(px(13.0)).mb_1());
                match kind {
                    // Position is the segmented Outside/Inside picker in
                    // EffectsSheet.swift; here it is a dropdown, stored in
                    // `checkbox_a` (false = Outside).
                    LayerEffectKind::Stroke => {
                        body = body
                            .child(sheet_row(
                                "Position",
                                dropdown(
                                    &app,
                                    "effect-position",
                                    if inside { "Inside" } else { "Outside" },
                                    px(110.0),
                                    vec![
                                        MenuEntry::new("Outside", |state, _, cx| {
                                            set_stroke_inside(state, false, cx)
                                        }),
                                        MenuEntry::new("Inside", |state, _, cx| {
                                            set_stroke_inside(state, true, cx)
                                        }),
                                    ],
                                ),
                            ))
                            .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                            .child(labeled_slider_sheet(&sliders, 0, "Size (px)"))
                            .child(labeled_slider_sheet(&sliders, 1, "Opacity (%)"));
                    }
                    LayerEffectKind::Shadow => {
                        body = body
                            .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                            .child(labeled_slider_sheet(&sliders, 0, "Opacity (%)"))
                            .child(labeled_slider_sheet(&sliders, 1, "Angle (°)"))
                            .child(labeled_slider_sheet(&sliders, 2, "Distance (px)"))
                            .child(labeled_slider_sheet(&sliders, 3, "Blur (px)"));
                    }
                    LayerEffectKind::ColorOverlay => {
                        body = body
                            .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                            .child(labeled_slider_sheet(&sliders, 0, "Opacity (%)"));
                    }
                    LayerEffectKind::InnerShadow => {
                        body = body
                            .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                            .child(labeled_slider_sheet(&sliders, 0, "Opacity (%)"))
                            .child(labeled_slider_sheet(&sliders, 1, "Angle (°)"))
                            .child(labeled_slider_sheet(&sliders, 2, "Distance (px)"))
                            .child(labeled_slider_sheet(&sliders, 3, "Blur (px)"));
                    }
                    LayerEffectKind::OuterGlow | LayerEffectKind::InnerGlow => {
                        body = body
                            .child(sheet_row("Color", effect_swatch(color, kind, cx)))
                            .child(labeled_slider_sheet(&sliders, 0, "Size (px)"))
                            .child(labeled_slider_sheet(&sliders, 1, "Opacity (%)"));
                    }
                }
            }
            SheetKind::Filter(filter) => {
                let sheet = self.sheet.as_ref().unwrap();
                let sliders = sheet.sliders.clone();
                match filter {
                    FilterKind::Curves => {
                        // The curve editor (CurvesControls.swift): channel
                        // picker, the curve canvas with its click/drag
                        // gesture, a selected-point readout with Remove, and
                        // Reset. The preview runs through the same filter
                        // pipeline as the slider sheets.
                        if self.curves_editor.is_none() {
                            self.curves_editor = Some(crate::state::CurvesEditor::new());
                        }
                        let (curves, selected) = {
                            let editor = self.curves_editor.as_ref().unwrap();
                            (editor.settings.clone(), editor.selected)
                        };
                        let channel = curves.channel;
                        let points = curves.channels[channel.index()].clone();
                        let readout = selected
                            .and_then(|index| points.get(index))
                            .map(|point| {
                                format!("Input {} \u{b7} Output {}", point.x as i64, point.y as i64)
                            })
                            .unwrap_or_default();
                        let can_remove = selected.is_some_and(|index| {
                            index > 0 && index + 1 < points.len()
                        });
                        body = body
                            .child(sheet_row(
                                "Channel",
                                dropdown(
                                    &app,
                                    "curves-channel",
                                    levels_channel_label(channel),
                                    px(90.0),
                                    LevelsChannel::ALL
                                        .iter()
                                        .map(|choice| {
                                            let choice = *choice;
                                            MenuEntry::new(
                                                levels_channel_label(choice),
                                                move |state, _, cx| {
                                                    state.curves_set_channel(choice, cx);
                                                },
                                            )
                                            .checked(channel == choice)
                                        })
                                        .collect(),
                                ),
                            ))
                            .child(
                                gpui_kit::div()
                                    .id("curves-canvas")
                                    .w_full()
                                    .h(px(240.0))
                                    .rounded_sm()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(gpui_kit::rgb(0x111111))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, event: &MouseDownEvent, _, cx| {
                                                this.curves_mouse_down(event, cx);
                                            },
                                        ),
                                    )
                                    .child(curves_canvas_view(
                                        &curves, &points, selected, cx,
                                    )),
                            )
                            .child(text_label(
                                "Click to add a point. Drag to adjust.",
                                true,
                            ))
                            .child(
                                gpui_kit::div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(text_label(readout, false))
                                    .child(gpui_kit::div().flex_1())
                                    .child(
                                        Button::new("curves-remove")
                                            .label("Remove point")
                                            .small()
                                            .outline()
                                            .disabled(!can_remove)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.curves_remove_point(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("curves-reset")
                                            .label("Reset curve")
                                            .small()
                                            .outline()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.curves_reset(cx);
                                            })),
                                    ),
                            );
                    }
                    FilterKind::GaussianBlur => {
                        body = body.child(labeled_slider_sheet(&sliders, 0, "Radius"));
                    }
                    FilterKind::MotionBlur => {
                        body = body
                            .child(labeled_slider_sheet(&sliders, 0, "Angle"))
                            .child(labeled_slider_sheet(&sliders, 1, "Distance"));
                    }
                    FilterKind::AddNoise => {
                        let gaussian = sheet.checkbox_a;
                        let monochromatic = sheet.checkbox_b;
                        body = body
                            .child(labeled_slider_sheet(&sliders, 0, "Amount"))
                            .child(sheet_row(
                                "Distribution",
                                labeled_checkbox(
                                    &app,
                                    "noise-gaussian",
                                    "Gaussian",
                                    gaussian,
                                    |state, checked, _, cx| {
                                        if let Some(sheet) = state.sheet.as_mut() {
                                            sheet.checkbox_a = checked;
                                        }
                                        state.refresh_filter_preview(cx);
                                    },
                                )
                                .into_any_element(),
                            ))
                            .child(sheet_row(
                                "",
                                labeled_checkbox(
                                    &app,
                                    "noise-mono",
                                    "Monochromatic",
                                    monochromatic,
                                    |state, checked, _, cx| {
                                        if let Some(sheet) = state.sheet.as_mut() {
                                            sheet.checkbox_b = checked;
                                        }
                                        state.refresh_filter_preview(cx);
                                    },
                                )
                                .into_any_element(),
                            ));
                    }
                    FilterKind::LensCorrection => {
                        body = body.child(labeled_slider_sheet(&sliders, 0, "Distortion"));
                    }
                    FilterKind::CameraRaw => {
                        body = body
                            .child(text_label("White Balance", false).mb_1())
                            .child(labeled_slider_sheet(&sliders, 0, "Temperature"))
                            .child(labeled_slider_sheet(&sliders, 1, "Tint"))
                            .child(text_label("Light", false).mb_1())
                            .child(labeled_slider_sheet(&sliders, 2, "Exposure"))
                            .child(labeled_slider_sheet(&sliders, 3, "Contrast"))
                            .child(labeled_slider_sheet(&sliders, 4, "Highlights"))
                            .child(labeled_slider_sheet(&sliders, 5, "Shadows"))
                            .child(labeled_slider_sheet(&sliders, 6, "Whites"))
                            .child(labeled_slider_sheet(&sliders, 7, "Blacks"))
                            .child(text_label("Color", false).mb_1())
                            .child(labeled_slider_sheet(&sliders, 8, "Vibrance"))
                            .child(labeled_slider_sheet(&sliders, 9, "Saturation"))
                            .child(text_label("Effects", false).mb_1())
                            .child(labeled_slider_sheet(&sliders, 10, "Texture"))
                            .child(labeled_slider_sheet(&sliders, 11, "Clarity"))
                            .child(labeled_slider_sheet(&sliders, 12, "Dehaze"))
                            .child(labeled_slider_sheet(&sliders, 13, "Vignette"));
                    }
                    _ => {
                        body = body.child(text_label(
                            "This filter runs with its default settings.",
                            true,
                        ));
                    }
                }
            }
            SheetKind::Amount(_) => {
                let sheet = self.sheet.as_ref().unwrap();
                body = body.child(sheet_row(
                    "Amount",
                    Input::new(&sheet.tolerance).id("amount-value").w(px(80.0)),
                ));
            }
            SheetKind::Import => {
                let sheet = self.sheet.as_ref().unwrap();
                body = body
                    .child(sheet_row(
                        "Path",
                        Input::new(&sheet.path).id("sheet-path").w(px(270.0)),
                    ))
                    .child(sheet_row(
                        "",
                        text_label(
                            "PNG/JPEG files import as new layers, centered; separate several paths with ';'. A .psd imports as Photoshop layers.",
                            true,
                        ),
                    ));
            }
            SheetKind::ExportPng | SheetKind::SaveComp | SheetKind::Open => {
                let sheet = self.sheet.as_ref().unwrap();
                let hint = match kind {
                    SheetKind::ExportPng => "The composited canvas is written as a PNG.",
                    SheetKind::SaveComp => "The project is saved as a .comp package folder.",
                    _ => "PNG/JPEG images open as a new layer; .comp folders open as projects.",
                };
                body = body
                    .child(sheet_row(
                        "Path",
                        Input::new(&sheet.path).id("sheet-path").w(px(270.0)),
                    ))
                    .child(sheet_row("", text_label(hint, true)));
            }
            SheetKind::ExportJpeg => {
                let sheet = self.sheet.as_ref().unwrap();
                let matte = sheet.preset;
                let quality = {
                    let cx_app: &gpui_kit::App = cx;
                    self.sheet_value(cx_app, 0)
                };
                let quality_text = format!("{}%", (quality * 100.0).round() as i32);
                let sliders = sheet.sliders.clone();
                body = body
                    .child(sheet_row(
                        "Path",
                        Input::new(&sheet.path).id("sheet-path").w(px(270.0)),
                    ))
                    .child(sheet_row("Quality", labeled_slider_sheet(&sliders, 0, &quality_text)))
                    .child(sheet_row(
                        "Matte",
                        dropdown(
                            &app,
                            "jpeg-matte",
                            if matte == 1 { "Black" } else { "White" },
                            px(140.0),
                            vec![
                                MenuEntry::new("White", |state: &mut AppState, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.preset = 0;
                                    }
                                    state.notify_only(cx);
                                }),
                                MenuEntry::new("Black", |state: &mut AppState, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.preset = 1;
                                    }
                                    state.notify_only(cx);
                                }),
                            ],
                        ),
                    ))
                    .child(sheet_row(
                        "",
                        text_label(
                            "Transparency flattens onto the matte color. JPEG has no alpha.",
                            true,
                        ),
                    ));
            }
            SheetKind::Trim => {
                let sheet = self.sheet.as_ref().unwrap();
                let based_on = sheet.based_on;
                let sides = sheet.sides;
                body = body
                    .child(sheet_row(
                        "Based On",
                        dropdown(
                            &app,
                            "trim-based-on",
                            match based_on {
                                1 => "Top Left Pixel Color",
                                2 => "Bottom Right Pixel Color",
                                _ => "Transparent Pixels",
                            },
                            px(210.0),
                            vec![
                                MenuEntry::new("Transparent Pixels", |state, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.based_on = 0;
                                    }
                                    state.notify_only(cx);
                                }),
                                MenuEntry::new("Top Left Pixel Color", |state, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.based_on = 1;
                                    }
                                    state.notify_only(cx);
                                }),
                                MenuEntry::new("Bottom Right Pixel Color", |state, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.based_on = 2;
                                    }
                                    state.notify_only(cx);
                                }),
                            ],
                        ),
                    ))
                    .child(sheet_row(
                        "Trim",
                        gpui_kit::div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(
                                labeled_checkbox(
                                    &app,
                                    "trim-top",
                                    "Top",
                                    sides[0],
                                    make_side_handler(0),
                                )
                                .into_any_element(),
                            )
                            .child(
                                labeled_checkbox(
                                    &app,
                                    "trim-bottom",
                                    "Bottom",
                                    sides[1],
                                    make_side_handler(1),
                                )
                                .into_any_element(),
                            )
                            .child(
                                labeled_checkbox(
                                    &app,
                                    "trim-left",
                                    "Left",
                                    sides[2],
                                    make_side_handler(2),
                                )
                                .into_any_element(),
                            )
                            .child(
                                labeled_checkbox(
                                    &app,
                                    "trim-right",
                                    "Right",
                                    sides[3],
                                    make_side_handler(3),
                                )
                                .into_any_element(),
                            ),
                    ))
                    .child(sheet_row(
                        "Tolerance",
                        Input::new(&sheet.tolerance).id("trim-tolerance").w(px(80.0)),
                    ));
            }
            SheetKind::About => {
                body = body.child(
                    gpui_kit::div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(text_label("Compositor", false).text_size(px(14.0)))
                        .child(text_label(
                            "A Photoshop-like image editor built on the compositor-core engine and GPUI.",
                            true,
                        )),
                );
            }
            SheetKind::Shortcuts => {
                body = body.child(text_label(SHORTCUTS_TEXT, true));
            }
        }
        body.into_any_element()
    }

    pub fn cancel_sheet(&mut self, cx: &mut Context<Self>) {
        // The Effects sheet's Cancel finishes editing without committing:
        // the edited effect returns to its opening state (Swift's
        // finishEffectsEditing(commit: false)).
        if self.sheet.as_ref().map(|sheet| sheet.kind) == Some(SheetKind::Effects) {
            self.session.finish_effects_editing(false);
        }
        // An adjustment editing session restores the opening settings.
        if self.session.adjustment_editing_id.is_some() {
            self.session.finish_adjustment_editing(false);
        }
        self.close_sheet(cx);
    }

    // MARK: Curves canvas (CurvesControls.swift's DragGesture)

    /// The channel picker: switching clears the selection, exactly as the
    /// Swift `onChange(of: settings.channel)` does.
    pub fn curves_set_channel(&mut self, channel: LevelsChannel, cx: &mut Context<Self>) {
        if let Some(editor) = self.curves_editor.as_mut() {
            editor.settings.channel = channel;
            editor.selected = None;
            editor.dragging = None;
        }
        self.notify_only(cx);
    }

    /// Pointer down on the curve canvas: grab the nearest point, or add a
    /// new one on empty canvas, then start dragging it.
    pub fn curves_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_mut() else {
            return;
        };
        let (x, y) = curve_point_from_event(event.position, editor.canvas_bounds);
        let channel = editor.settings.channel.index();
        let points = editor.settings.channels[channel].clone();
        match curve_drag_begin(&points, x, y) {
            CurveDragBegin::None => editor.dragging = None,
            CurveDragBegin::Grab(index) => {
                editor.dragging = Some(index);
                editor.selected = Some(index);
            }
            CurveDragBegin::Added { points, index } => {
                editor.settings.channels[channel] = points;
                editor.dragging = Some(index);
                editor.selected = Some(index);
            }
        }
        self.curves_drag_to(x, y, cx);
    }

    /// Pointer move while a drag is live: the point follows, the preview
    /// re-renders.
    pub fn curves_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_ref() else {
            return;
        };
        if editor.dragging.is_none() {
            return;
        }
        let (x, y) = curve_point_from_event(event.position, editor.canvas_bounds);
        self.curves_drag_to(x, y, cx);
    }

    /// Pointer up: the drag ends (the selection stays).
    pub fn curves_mouse_up(&mut self, _event: &MouseUpEvent, cx: &mut Context<Self>) {
        let was_dragging = self
            .curves_editor
            .as_mut()
            .map(|editor| editor.dragging.take().is_some())
            .unwrap_or(false);
        if was_dragging {
            self.notify_only(cx);
        }
    }

    /// The "Remove point" button: interior points only, as in Swift
    /// (`selected > 0 && selected < points.count-1`).
    pub fn curves_remove_point(&mut self, cx: &mut Context<Self>) {
        let removed = {
            let Some(editor) = self.curves_editor.as_mut() else {
                return;
            };
            let Some(index) = editor.selected else {
                return;
            };
            let channel = editor.settings.channel.index();
            let points = &mut editor.settings.channels[channel];
            let interior = index > 0 && index + 1 < points.len();
            if interior {
                points.remove(index);
                editor.selected = None;
            }
            interior
        };
        if removed {
            self.apply_curves_preview(cx);
        }
    }

    /// The "Reset curve" button: the channel's points return to the
    /// identity diagonal.
    pub fn curves_reset(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_mut() else {
            return;
        };
        let channel = editor.settings.channel.index();
        editor.settings.channels[channel] = CurvesSettings::default().channels[channel].clone();
        editor.selected = None;
        editor.dragging = None;
        self.apply_curves_preview(cx);
    }

    /// Folds a pointer position into the dragged point and refreshes the
    /// preview.
    fn curves_drag_to(&mut self, x: f64, y: f64, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_mut() else {
            return;
        };
        let Some(index) = editor.dragging else {
            return;
        };
        let channel = editor.settings.channel.index();
        let moved = curve_drag_move(&editor.settings.channels[channel], index, x, y);
        editor.settings.channels[channel] = moved;
        self.apply_curves_preview(cx);
    }

    // MARK: OK

    pub fn confirm_sheet(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // Editing an adjustment layer: the settings previewed live, OK keeps
        // them as one undo step and skips the layer-pixel commit paths.
        if self.session.adjustment_editing_id.is_some() {
            self.session.finish_adjustment_editing(true);
            self.close_sheet(cx);
            return;
        }
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return,
        };
        match kind {
            SheetKind::NewCanvas => {
                let (width, height, background) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        parse_input_usize(&sheet.width, cx_app).unwrap_or(1920),
                        parse_input_usize(&sheet.height, cx_app).unwrap_or(1080),
                        sheet.background,
                    )
                };
                self.session.create_document(width.max(1), height.max(1), true);
                match background {
                    0 => self.fill_active([255, 255, 255, 255], "Fill White", cx),
                    1 => self.fill_active([0, 0, 0, 255], "Fill Black", cx),
                    _ => {}
                }
                self.fit_for = (0, 0);
            }
            SheetKind::ImageSize => {
                let (mut width, mut height, resolution, percent) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        parse_input_usize(&sheet.width, cx_app).unwrap_or(0),
                        parse_input_usize(&sheet.height, cx_app).unwrap_or(0),
                        parse_input_f64(&sheet.resolution, cx_app).unwrap_or(72.0),
                        sheet.percent,
                    )
                };
                if percent {
                    let (w, h) = self.doc_size();
                    width = ((w * width as f64) / 100.0).round() as usize;
                    height = ((h * height as f64) / 100.0).round() as usize;
                }
                self.session.resize_image_to(
                    width,
                    height,
                    Some(resolution),
                    compositor_core::transform::LayerSampling::High,
                );
                self.fit_for = (0, 0);
            }
            SheetKind::CanvasSize => {
                let (mut width, mut height, relative, anchor) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        parse_input_usize(&sheet.width, cx_app).unwrap_or(0),
                        parse_input_usize(&sheet.height, cx_app).unwrap_or(0),
                        sheet.relative,
                        sheet.anchor,
                    )
                };
                if relative {
                    let (w, h) = self.doc_size();
                    width += w as usize;
                    height += h as usize;
                }
                let mut options = CanvasSizeOptions::new(width.max(1), height.max(1));
                options.anchor = anchor.min(8);
                self.session.resize_canvas_to(&options);
                self.fit_for = (0, 0);
            }
            SheetKind::Levels => self.commit_preview("Levels", cx),
            SheetKind::HueSaturation => self.commit_preview("Hue/Saturation", cx),
            SheetKind::Effects => {
                // Every change was already a committed undo step; OK just
                // ends the editing session and keeps the effects.
                self.session.finish_effects_editing(true);
            }
            SheetKind::Filter(filter) => self.commit_preview(filter.label(), cx),
            SheetKind::Amount(_) => {
                let amount = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    parse_input_usize(&sheet.tolerance, cx_app).unwrap_or(0)
                } as i64;
                self.session.confirm_selection_amount(amount);
            }
            SheetKind::ExportPng => {
                let path = self.sheet_path(cx);
                self.export_png(path, cx);
            }
            SheetKind::ExportJpeg => {
                let (path, quality, matte) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        sheet.path.read(cx_app).value().to_string(),
                        sheet.sliders.first().map(|s| s.read(cx_app).value().end()).unwrap_or(0.85) as f64,
                        sheet.preset,
                    )
                };
                self.export_jpeg(path, quality, matte, cx);
            }
            SheetKind::Import => {
                let path = self.sheet_path(cx);
                let paths: Vec<std::path::PathBuf> = path
                    .split(';')
                    .map(|part| part.trim())
                    .filter(|part| !part.is_empty())
                    .map(std::path::PathBuf::from)
                    .collect();
                let failures = self.session.import_images(&paths, None);
                if failures.is_empty() {
                    if !paths.is_empty() {
                        self.set_status(format!("Imported {}", path), cx);
                    }
                } else {
                    self.set_status(failures.join("; "), cx);
                }
                self.fit_for = (0, 0);
            }
            SheetKind::SaveComp => {
                let path = self.sheet_path(cx);
                self.save_comp(path, cx);
            }
            SheetKind::Open => {
                let path = self.sheet_path(cx);
                self.open_document(&path, cx);
            }
            SheetKind::Trim => {
                let (based_on, sides, tolerance) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        sheet.based_on,
                        sheet.sides,
                        parse_input_usize(&sheet.tolerance, cx_app).unwrap_or(0).min(255) as u8,
                    )
                };
                let options = TrimOptions {
                    based_on: match based_on {
                        1 => TrimBasedOn::TopLeftPixelColor,
                        2 => TrimBasedOn::BottomRightPixelColor,
                        _ => TrimBasedOn::TransparentPixels,
                    },
                    top: sides[0],
                    bottom: sides[1],
                    left: sides[2],
                    right: sides[3],
                    tolerance,
                };
                if !self.session.trim(&options) {
                    self.set_status("Nothing to trim", cx);
                }
                self.fit_for = (0, 0);
            }
            SheetKind::About | SheetKind::Shortcuts => {}
        }
        self.close_sheet(cx);
    }

    fn sheet_path(&self, cx: &Context<Self>) -> String {
        self.sheet
            .as_ref()
            .map(|sheet| sheet.path.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Fills the active layer with one color, honoring the selection.
    pub fn fill_active(&mut self, color: [u8; 4], name: &str, cx: &mut Context<Self>) {
        let Some(document) = self.session.document.clone() else { return };
        let Some(id) = self.session.active_layer_id else { return };
        let Some(layer) = document.layer(id) else { return };
        if layer.is_group {
            return;
        }
        let settings = compositor_core::tools::brush::BrushSettings::default();
        let edit = RasterEdit::new(
            layer.asset.as_ref().map(|a| a.image.as_ref().clone()),
            layer.transform,
            false,
            settings,
            CGSize::new(document.width as f64, document.height as f64),
            document.selection.as_ref().map(|s| s.clip(document.size())),
        );
        let result = edit.fill(color);
        self.commit_bitmap(result, layer.transform, name, cx);
    }

    // MARK: File operations

    pub fn export_png(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(document) = self.session.document.clone() else {
            self.set_status("Nothing to export", cx);
            return;
        };
        let composite = compositor_core::render::draw_live_composite(&document);
        let mut data = composite.data.clone();
        for pixel in data.chunks_exact_mut(4) {
            let alpha = pixel[3] as u32;
            if alpha == 0 {
                continue;
            }
            for channel in 0..3 {
                pixel[channel] = ((pixel[channel] as u32 * 255) / alpha).min(255) as u8;
            }
        }
        if let Some(image) =
            image::RgbaImage::from_raw(composite.width as u32, composite.height as u32, data)
        {
            match image.save(&path) {
                Ok(()) => self.set_status(format!("Exported {}", path), cx),
                Err(error) => self.set_status(format!("Export failed: {}", error), cx),
            }
        }
    }

    /// `ImageExporter.jpeg` + `write`: the composite flattens onto the matte
    /// and encodes at `quality` (0…1). `matte`: 0 = white, 1 = black.
    pub fn export_jpeg(
        &mut self,
        path: String,
        quality: f64,
        matte: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.session.document.clone() else {
            self.set_status("Nothing to export", cx);
            return;
        };
        let composite = compositor_core::render::draw_live_composite(&document);
        if let Err(error) =
            compositor_core::export::check_export_size(composite.width, composite.height)
        {
            self.set_status(format!("Export failed: {}", error), cx);
            return;
        }
        let options = compositor_core::export::JpegOptions {
            quality,
            red: if matte == 1 { 0.0 } else { 1.0 },
            green: if matte == 1 { 0.0 } else { 1.0 },
            blue: if matte == 1 { 0.0 } else { 1.0 },
        };
        let data = match compositor_core::export::jpeg_data(&composite, &options) {
            Some(data) => data,
            None => {
                self.set_status("Export failed: the image could not be encoded.", cx);
                return;
            }
        };
        match std::fs::write(&path, data) {
            Ok(()) => self.set_status(format!("Exported {}", path), cx),
            Err(error) => self.set_status(format!("Export failed: {}", error), cx),
        }
    }

    pub fn save_comp(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(document) = self.session.document.clone() else {
            self.set_status("Nothing to save", cx);
            return;
        };
        let dir = std::path::PathBuf::from(path);
        let data = compositor_core::project::project_from_document(&document);
        match std::fs::create_dir_all(&dir) {
            Ok(()) => match compositor_core::project::save(&dir, &data) {
                Ok(()) => {
                    self.set_status(format!("Saved {}", dir.display()), cx);
                    self.session.mark_saved();
                }
                Err(error) => self.set_status(format!("Save failed: {}", error), cx),
            },
            Err(error) => self.set_status(format!("Save failed: {}", error), cx),
        }
    }

    /// Opens an image (PNG/JPEG) as a new layer, or a .comp package folder.
    pub fn open_document(&mut self, path: &str, cx: &mut Context<Self>) {
        let dir = std::path::PathBuf::from(path);
        if dir.is_dir() || path.ends_with(".comp") {
            match compositor_core::project::load(&dir) {
                Ok(data) => match compositor_core::project::document_from_project(&data) {
                    Ok((document, active)) => {
                        self.session.active_layer_id = active.or_else(|| {
                            document.layers.last().map(|layer| layer.id)
                        });
                        self.session.selected_layer_ids = self
                            .session
                            .active_layer_id
                            .map(|id| vec![id])
                            .unwrap_or_default();
                        self.session.document = Some(document);
                        self.session.history.reset();
                        self.session.mark_saved();
                        self.fit_for = (0, 0);
                        self.set_status(format!("Opened {}", dir.display()), cx);
                    }
                    Err(error) => self.set_status(format!("Open failed: {}", error), cx),
                },
                Err(error) => self.set_status(format!("Open failed: {}", error), cx),
            }
            return;
        }
        // A Photoshop file goes through PsdReader + PsdDocumentBuilder
        // (`insertPhotoshop`): fresh document or wrapping group, with a note
        // per conversion.
        if path.to_ascii_lowercase().ends_with(".psd") {
            let data = match std::fs::read(path) {
                Ok(data) => data,
                Err(error) => {
                    self.set_status(format!("Open failed: {}", error), cx);
                    return;
                }
            };
            let name = std::path::Path::new(path)
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "Photoshop".to_string());
            match self.session.import_photoshop_data(&name, &data) {
                Ok(notes) => {
                    self.fit_for = (0, 0);
                    self.set_status(
                        if notes > 0 {
                            format!(
                                "Imported {} ({} conversion note{})",
                                path,
                                notes,
                                if notes == 1 { "" } else { "s" }
                            )
                        } else {
                            format!("Imported {}", path)
                        },
                        cx,
                    );
                }
                Err(error) => self.set_status(format!("Open failed: {}", error), cx),
            }
            return;
        }
        match image::open(path) {
            Ok(image) => {
                let rgba = image.to_rgba8();
                let (width, height) = (rgba.width() as usize, rgba.height() as usize);
                let mut bitmap = Bitmap::new(width, height);
                bitmap.data = rgba.into_raw();
                for pixel in bitmap.data.chunks_exact_mut(4) {
                    let alpha = pixel[3] as u32;
                    for channel in 0..3 {
                        pixel[channel] = ((pixel[channel] as u32 * alpha + 127) / 255) as u8;
                    }
                }
                let name = std::path::Path::new(path)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Image".to_string());
                if self.session.document.is_none() {
                    self.session.create_document(width, height, true);
                }
                let layer = compositor_core::document::ImageLayer::with_asset(
                    compositor_core::raster::ImportedImage::new(bitmap, name),
                    CGPoint::ZERO,
                );
                self.session.history.begin(
                    "Open",
                    &self.session.document,
                    self.session.active_layer_id,
                );
                let id = layer.id;
                if let Some(document) = self.session.document.as_mut() {
                    let slot = document.layers.len();
                    document.layers.insert(slot, layer);
                }
                self.session.select_layer(Some(id));
                self.session
                    .history
                    .end(&self.session.document, self.session.active_layer_id);
                self.fit_for = (0, 0);
                self.set_status(format!("Opened {}", path), cx);
            }
            Err(error) => self.set_status(format!("Open failed: {}", error), cx),
        }
    }
}

// MARK: Helpers

/// The Levels/Curves channel picker's labels (LevelsSheet.swift's Picker).
pub fn levels_channel_label(channel: LevelsChannel) -> &'static str {
    match channel {
        LevelsChannel::Rgb => "RGB",
        LevelsChannel::Red => "Red",
        LevelsChannel::Green => "Green",
        LevelsChannel::Blue => "Blue",
    }
}

/// Window pixels inside the curve canvas → curve units 0…255 with the y
/// axis flipped (CurvesControls' `position(_:)` inverse).
pub fn curve_point_from_event(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
) -> (f64, f64) {
    let width = f32::from(bounds.size.width).max(1.0) as f64;
    let height = f32::from(bounds.size.height).max(1.0) as f64;
    let x = (f32::from(position.x) - f32::from(bounds.origin.x)) as f64;
    let y = (f32::from(position.y) - f32::from(bounds.origin.y)) as f64;
    let x = (x / width * 255.0).clamp(0.0, 255.0);
    let y = (255.0 - y / height * 255.0).clamp(0.0, 255.0);
    (x, y)
}

/// Curve units → window pixels (CurvesControls' `position(_:)`).
pub fn curve_value_to_screen(x: f64, y: f64, bounds: Bounds<Pixels>) -> (f32, f32) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    (
        f32::from(bounds.origin.x) + (x as f32 / 255.0) * width,
        f32::from(bounds.origin.y) + (1.0 - y as f32 / 255.0) * height,
    )
}

/// The nearest control point within 14 curve units (CurvesControls' hit
/// test: the distance runs in curve units, not pixels).
pub fn curve_hit_point(points: &[CurvePoint], x: f64, y: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, point) in points.iter().enumerate() {
        let distance = ((point.x - x).powi(2) + (point.y - y).powi(2)).sqrt();
        if distance < 14.0 && best.map_or(true, |(_, best)| distance < best) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| index)
}

/// Starting a drag: grab a nearby point, or add a new one on empty canvas
/// (up to 32 points, `x` in 2…253, and at least one input level from every
/// existing point), or nothing.
pub fn curve_drag_begin(points: &[CurvePoint], x: f64, y: f64) -> CurveDragBegin {
    if let Some(index) = curve_hit_point(points, x, y) {
        return CurveDragBegin::Grab(index);
    }
    if points.len() >= 32
        || x <= 1.0
        || x >= 254.0
        || points.iter().any(|point| (point.x - x).abs() <= 1.0)
    {
        return CurveDragBegin::None;
    }
    let mut points = points.to_vec();
    // The list stays sorted by x; no existing point sits within 1 of x.
    let index = points.partition_point(|point| point.x < x);
    points.insert(index, CurvePoint { x, y });
    CurveDragBegin::Added { points, index }
}

/// Dragging a point: `y` follows the pointer; interior points also follow
/// `x`, clamped between their neighbors (`p[i].x = min(next-1,
/// max(prev+1, x))`).
pub fn curve_drag_move(points: &[CurvePoint], index: usize, x: f64, y: f64) -> Vec<CurvePoint> {
    let mut points = points.to_vec();
    // Neighbors first: the slot borrow and the reads must not overlap.
    let (previous, next, interior) = match index {
        0 => (0.0, 0.0, false),
        i if i + 1 >= points.len() => (0.0, 0.0, false),
        i => (points[i - 1].x, points[i + 1].x, true),
    };
    if let Some(slot) = points.get_mut(index) {
        slot.y = y;
        if interior {
            slot.x = (next - 1.0).min((previous + 1.0).max(x));
        }
    }
    points
}

/// The curve canvas: a layout pass that records the bounds for hit testing,
/// and a paint pass drawing grid, spline and control points, which also
/// installs the window-level drag events (the drag keeps tracking when the
/// pointer leaves the canvas, as a SwiftUI DragGesture does).
fn curves_canvas_view(
    curves: &CurvesSettings,
    points: &[CurvePoint],
    selected: Option<usize>,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    let app_bounds = cx.entity().downgrade();
    let app_move = cx.entity().downgrade();
    let app_up = cx.entity().downgrade();

    // Sample the spline once per frame, in curve units.
    let channel = curves.channel.index();
    let polyline: Vec<CurvePoint> = (0..=255)
        .map(|step| CurvePoint {
            x: step as f64,
            y: curves.value(step as f64, channel),
        })
        .collect();
    let points = points.to_vec();

    canvas(
        move |bounds, _window, cx| {
            app_bounds
                .update(cx, |state, _| {
                    if let Some(editor) = state.curves_editor.as_mut() {
                        editor.canvas_bounds = bounds;
                    }
                })
                .ok();
        },
        move |bounds, _prepaint, window, _cx| {
            paint_curve_canvas(bounds, &polyline, &points, selected, window);
            install_curve_drag_events(&app_move, &app_up, window);
        },
    )
    .size_full()
    .into_any_element()
}

/// The grid (5×5, white 12%), the spline (2px white) and the control points
/// (8px squares, the selection in the accent color), matching
/// CurvesControls.swift's Canvas body.
fn paint_curve_canvas(
    bounds: Bounds<Pixels>,
    polyline: &[CurvePoint],
    points: &[CurvePoint],
    selected: Option<usize>,
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let origin_x = f32::from(bounds.origin.x);
    let origin_y = f32::from(bounds.origin.y);
    for step in 0..=4 {
        let fraction = step as f32 / 4.0;
        window.paint_quad(gpui_kit::fill(
            Bounds::<Pixels>::new(
                point(px(origin_x + fraction * width), px(origin_y)),
                size(px(1.0), px(height)),
            ),
            rgba(0xffffff1f),
        ));
        window.paint_quad(gpui_kit::fill(
            Bounds::<Pixels>::new(
                point(px(origin_x), px(origin_y + fraction * height)),
                size(px(width), px(1.0)),
            ),
            rgba(0xffffff1f),
        ));
    }
    let screen: Vec<Point<Pixels>> = polyline
        .iter()
        .map(|value| {
            let (x, y) = curve_value_to_screen(value.x, value.y, bounds);
            point(px(x), px(y))
        })
        .collect();
    if screen.len() >= 2 {
        let mut builder = PathBuilder::stroke(px(2.0));
        builder.add_polygon(&screen, false);
        if let Ok(path) = builder.build() {
            window.paint_path(path, rgba(0xffffffff));
        }
    }
    for (index, value) in points.iter().enumerate() {
        let (x, y) = curve_value_to_screen(value.x, value.y, bounds);
        let rect = Bounds::<Pixels>::new(
            point(px(x - 4.0), px(y - 4.0)),
            size(px(8.0), px(8.0)),
        );
        window.paint_quad(gpui_kit::fill(
            rect,
            if selected == Some(index) {
                gpui_kit::rgb(crate::ui::ACCENT)
            } else {
                gpui_kit::rgb(0xffffff)
            },
        ));
    }
}

/// Window-level move/up handlers so an in-progress drag keeps following the
/// pointer outside the canvas (`canvas.rs`'s install_drag_events pattern).
fn install_curve_drag_events(
    app_move: &gpui_kit::WeakEntity<AppState>,
    app_up: &gpui_kit::WeakEntity<AppState>,
    window: &mut Window,
) {
    let app_move = app_move.clone();
    let app_up = app_up.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
        app_move
            .update(cx, |state, cx| state.curves_mouse_move(event, cx))
            .ok();
    });
    window.on_mouse_event(move |event: &MouseUpEvent, _phase, _window, cx| {
        app_up
            .update(cx, |state, cx| state.curves_mouse_up(event, cx))
            .ok();
    });
}

fn set_inputs(
    state: &mut AppState,
    width: &Entity<InputState>,
    height: &Entity<InputState>,
    w: usize,
    h: usize,
    window: &mut Window,
    cx: &mut Context<AppState>,
) {
    width.update(cx, |input, cx| input.set_value(w.to_string(), window, cx));
    height.update(cx, |input, cx| {
        input.set_value(h.to_string(), window, cx)
    });
    if let Some(sheet) = state.sheet.as_mut() {
        sheet.preset = match (w, h) {
            (3000, 2000) => 1,
            (1280, 720) => 2,
            _ => 0,
        };
    }
    state.notify_only(cx);
}

fn set_channel(state: &mut AppState, channel: usize, cx: &mut Context<AppState>) {
    if let Some(sheet) = state.sheet.as_mut() {
        sheet.channel = channel;
    }
    state.notify_only(cx);
}

/// Stroke position (EffectsSheet's segmented Outside/Inside picker): the
/// change goes through the session like every slider, so the canvas and the
/// undo trail follow.
fn set_stroke_inside(state: &mut AppState, inside: bool, cx: &mut Context<AppState>) {
    state.session.change_effects(|effects| {
        if let Some(stroke) = effects.stroke.as_mut() {
            stroke.inside = inside;
        }
    });
    if let Some(sheet) = state.sheet.as_mut() {
        sheet.checkbox_a = inside;
    }
    state.touch(cx);
}

/// The effect's color chip (EffectsSheet.swift's swatch); clicking opens the
/// app's color picker on it.
fn effect_swatch(
    color: Option<PaletteColor>,
    kind: LayerEffectKind,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    let color = color.unwrap_or_default();
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    let rgb = (channel(color.red) << 16) | (channel(color.green) << 8) | channel(color.blue);
    gpui_kit::div()
        .id(SharedString::from(format!("effect-swatch-{}", kind.label())))
        .w(px(36.0))
        .h(px(18.0))
        .rounded_xs()
        .border_1()
        .border_color(gpui_kit::rgb(0x111111))
        .bg(gpui_kit::rgb(rgb))
        .on_click(cx.listener(move |state, _, _, cx| {
            state.open_effect_color_picker(kind, cx);
        }))
        .into_any_element()
}

fn make_side_handler(
    index: usize,
) -> impl Fn(&mut AppState, bool, &mut Window, &mut Context<AppState>) + 'static {
    move |state, checked, _, cx| {
        if let Some(sheet) = state.sheet.as_mut() {
            sheet.sides[index] = checked;
        }
        state.notify_only(cx);
    }
}

fn anchor_picker(anchor: usize, cx: &mut Context<AppState>) -> gpui_kit::AnyElement {
    let mut grid = gpui_kit::div().flex().flex_row().gap_0p5();
    for index in 0..9 {
        grid = grid.child(
            gpui_kit::div()
                .id(SharedString::from(format!("anchor-{}", index)))
                .w(px(26.0))
                .h(px(26.0))
                .rounded_xs()
                .border_1()
                .border_color(gpui_kit::rgb(0x555555))
                .when(index == anchor, |cell| cell.bg(gpui_kit::rgb(0x4da6ff)))
                .on_click(cx.listener(move |state, _, _, cx| {
                    if let Some(sheet) = state.sheet.as_mut() {
                        sheet.anchor = index;
                    }
                    state.notify_only(cx);
                })),
        );
    }
    grid.into_any_element()
}

/// A labeled slider row bound to one of the sheet's sliders.
fn labeled_slider_sheet(
    sliders: &[Entity<SliderState>],
    index: usize,
    label: &str,
) -> gpui_kit::AnyElement {
    let row = gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .py_0p5()
        .child(
            gpui_kit::div()
                .w(px(110.0))
                .flex_shrink_0()
                .child(text_label(label.to_string(), true)),
        );
    match sliders.get(index) {
        Some(slider) => row
            .child(gpui_kit::div().flex_1().child(Slider::new(slider)))
            .into_any_element(),
        None => row.into_any_element(),
    }
}

/// The Levels histogram, painted as thin bars.
fn histogram_view(histogram: [f64; 1024]) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.paint_quad(gpui_kit::fill(bounds, gpui_kit::rgb(0x1b1b1b)));
            let peak = histogram.iter().cloned().fold(0.0f64, f64::max);
            if peak <= 0.0 {
                return;
            }
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let buckets = width.max(1.0) as usize;
            for x in 0..buckets {
                let index = ((x as f64 / buckets as f64) * 1023.0) as usize;
                let value = (histogram[index] / peak).clamp(0.0, 1.0);
                let bar = (value * f64::from(height)).round() as f32;
                if bar < 1.0 {
                    continue;
                }
                window.paint_quad(gpui_kit::fill(
                    gpui_kit::Bounds::<gpui_kit::Pixels>::new(
                        point(
                            px(f32::from(bounds.origin.x) + x as f32),
                            px(f32::from(bounds.origin.y) + height - bar),
                        ),
                        size(px(1.0), px(bar)),
                    ),
                    rgba(0xccccccdd),
                ));
            }
        },
    )
    .size_full()
}

const SHORTCUTS_TEXT: &str = "Tools: V Move \u{b7} M Marquee \u{b7} L Lasso \u{b7} W Wand \u{b7} C Crop \u{b7} B/E Brush \u{b7} J Healing \u{b7} S Clone \u{b7} R Smear \u{b7} G Gradient \u{b7} U Shape \u{b7} T Type \u{b7} I Eyedropper \u{b7} H Hand \u{b7} Z Zoom\nColors: X swap \u{b7} D default\nEdit: Ctrl+Z undo \u{b7} Ctrl+Shift+Z redo \u{b7} Ctrl+T transform \u{b7} Ctrl+E merge\nSelect: Ctrl+A all \u{b7} Ctrl+D deselect \u{b7} Ctrl+Shift+I inverse\nView: Ctrl+R rulers \u{b7} Ctrl+' grid \u{b7} Ctrl+= / Ctrl+- zoom \u{b7} Ctrl+0 fit\nLayers: Ctrl+Shift+N new \u{b7} Ctrl+J duplicate \u{b7} Ctrl+G group \u{b7} Delete remove\nCanvas: [ ] brush size \u{b7} arrows nudge \u{b7} Tab cycles the tool mode";

#[cfg(test)]
mod tests {
    use super::{
        curve_drag_begin, curve_drag_move, curve_hit_point, curve_point_from_event,
        curve_value_to_screen, levels_channel_label, CurveDragBegin,
    };
    use compositor_core::adjust::{CurvePoint, LevelsChannel};
    use gpui_kit::{point, px, size, Bounds, Pixels};

    fn diagonal() -> Vec<CurvePoint> {
        vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 255.0, y: 255.0 }]
    }

    fn canvas_bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.0), px(20.0)), size(px(255.0), px(255.0)))
    }

    #[test]
    fn event_position_maps_to_curve_units() {
        let bounds = canvas_bounds();
        // The canvas origin is curve (0, 255); the bottom right is (255, 0).
        assert_eq!(curve_point_from_event(point(px(10.0), px(20.0)), bounds), (0.0, 255.0));
        assert_eq!(curve_point_from_event(point(px(265.0), px(275.0)), bounds), (255.0, 0.0));
        // The center is the diagonal's midpoint, and clamping keeps drags
        // outside the canvas in range.
        assert_eq!(curve_point_from_event(point(px(135.0), px(145.0)), bounds), (125.0, 130.0));
        assert_eq!(curve_point_from_event(point(px(-50.0), px(-50.0)), bounds), (0.0, 255.0));
    }

    #[test]
    fn screen_mapping_roundtrips_the_canvas_center() {
        let bounds = canvas_bounds();
        let (sx, sy) = curve_value_to_screen(128.0, 128.0, bounds);
        let (x, y) = curve_point_from_event(point(px(sx), px(sy)), bounds);
        assert!((x - 128.0).abs() < 1e-9, "x = {x}");
        assert!((y - 128.0).abs() < 1e-9, "y = {y}");
    }

    #[test]
    fn hit_test_finds_the_nearest_point_within_fourteen_units() {
        let points = diagonal();
        // On the diagonal at (100, 100): 141 units from each endpoint.
        assert_eq!(curve_hit_point(&points, 100.0, 100.0), None);
        let near_start = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 5.0, y: 5.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        // (3, 3) sits ~3 units from (5, 5) and ~4.2 from (0, 0): the nearer
        // point wins.
        assert_eq!(curve_hit_point(&near_start, 3.0, 3.0), Some(1));
        // Ties keep the earlier point, as Swift's min(by:) does.
        let tie = vec![
            CurvePoint { x: 10.0, y: 10.0 },
            CurvePoint { x: 14.0, y: 10.0 },
        ];
        assert_eq!(curve_hit_point(&tie, 12.0, 10.0), Some(0));
    }

    #[test]
    fn drag_begin_grabs_grabs_and_refuses() {
        let points = diagonal();
        // Near an endpoint: a grab.
        match curve_drag_begin(&points, 2.0, 3.0) {
            CurveDragBegin::Grab(0) => {}
            other => panic!("expected grab, got {other:?}"),
        }
        // Empty canvas in the middle: a new sorted point.
        match curve_drag_begin(&points, 128.0, 60.0) {
            CurveDragBegin::Added { points, index } => {
                assert_eq!(index, 1);
                assert_eq!(points.len(), 3);
                assert_eq!(points[1], CurvePoint { x: 128.0, y: 60.0 });
                assert!(points.windows(2).all(|w| w[0].x < w[1].x));
            }
            other => panic!("expected added, got {other:?}"),
        }
        // Swift's guards: x sits strictly inside 1…254 ...
        assert!(matches!(
            curve_drag_begin(&points, 1.0, 60.0),
            CurveDragBegin::None
        ));
        assert!(matches!(
            curve_drag_begin(&points, 254.0, 60.0),
            CurveDragBegin::None
        ));
        // ... and within one input level of no existing point's x.
        let with_mid = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 128.0, y: 64.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        assert!(matches!(
            curve_drag_begin(&with_mid, 128.5, 100.0),
            CurveDragBegin::None
        ));
        // 32 points: nothing more is added.
        let mut full: Vec<CurvePoint> = (0..31)
            .map(|i| CurvePoint {
                x: i as f64 * 8.0 + 0.5,
                y: 0.0,
            })
            .collect();
        full.push(CurvePoint { x: 255.0, y: 255.0 });
        assert_eq!(full.len(), 32);
        assert!(matches!(
            curve_drag_begin(&full, 128.0, 60.0),
            CurveDragBegin::None
        ));
    }

    #[test]
    fn drag_move_follows_y_and_clamps_interior_x() {
        // A sorted fixture: interior point at 128 with neighbors 0 and 200.
        let points = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 128.0, y: 100.0 },
            CurvePoint { x: 200.0, y: 220.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        // Interior point: y is free, x clamps between neighbors.
        let moved = curve_drag_move(&points, 1, 250.0, 42.0);
        assert_eq!(moved[1].y, 42.0);
        assert_eq!(moved[1].x, 199.0); // min(next-1, max(prev+1, x))
        let moved = curve_drag_move(&points, 1, 1.0, 42.0);
        assert_eq!(moved[1].x, 1.0); // pinned at prev+1
        // Endpoints keep their x: the curve stays anchored at 0 and 255. (The
        // verbatim y is safe because the pointer conversion clamps to 0…255.)
        let moved = curve_drag_move(&points, 0, 128.0, 10.0);
        assert_eq!(moved[0].x, 0.0);
        assert_eq!(moved[0].y, 10.0);
        // An out-of-range index is a no-op.
        assert_eq!(curve_drag_move(&points, 9, 10.0, 10.0), points);
    }

    #[test]
    fn channel_labels_match_the_swift_picker() {
        assert_eq!(levels_channel_label(LevelsChannel::Rgb), "RGB");
        assert_eq!(levels_channel_label(LevelsChannel::Red), "Red");
        assert_eq!(levels_channel_label(LevelsChannel::Green), "Green");
        assert_eq!(levels_channel_label(LevelsChannel::Blue), "Blue");
    }
}
