//! The modal sheets themselves: the centered overlay panel, per-sheet bodies,
//! and the OK/Cancel actions (including PNG export, .comp save and open).

use gpui_kit::{
    canvas, px, point, prelude::FluentBuilder as _, rgba, size, Context, Entity,
    InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::Sizable;

use compositor_core::canvas::{CanvasSizeOptions, TrimBasedOn, TrimOptions};
use compositor_core::filters::FilterKind;
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::Bitmap;
use compositor_core::session::SelectionAmountOperation;
use compositor_core::tools::brush::RasterEdit;

use crate::sheets::SliderRole;
use crate::state::{AppState, SheetKind};
use crate::ui::{dropdown, labeled_checkbox, text_label, MenuEntry};

fn sheet_title(kind: SheetKind) -> &'static str {
    match kind {
        SheetKind::NewCanvas => "New Document",
        SheetKind::ImageSize => "Image Size",
        SheetKind::CanvasSize => "Canvas Size",
        SheetKind::Levels => "Levels",
        SheetKind::HueSaturation => "Hue/Saturation",
        SheetKind::Filter(FilterKind::GaussianBlur) => "Gaussian Blur",
        SheetKind::Filter(FilterKind::MotionBlur) => "Motion Blur",
        SheetKind::Filter(FilterKind::AddNoise) => "Add Noise",
        SheetKind::Filter(FilterKind::LensCorrection) => "Lens Correction",
        SheetKind::Filter(FilterKind::CameraRaw) => "Camera Raw Filter",
        SheetKind::Filter(_) => "Filter",
        SheetKind::Amount(SelectionAmountOperation::Feather) => "Feather Selection",
        SheetKind::Amount(SelectionAmountOperation::Expand) => "Expand Selection",
        SheetKind::Amount(SelectionAmountOperation::Contract) => "Contract Selection",
        SheetKind::ExportPng => "Export PNG",
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
                body = body
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
                        Button::new("levels-auto")
                            .label("Auto")
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.levels_auto(window, cx);
                            }))
                            .into_any_element(),
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
            SheetKind::Filter(filter) => {
                let sheet = self.sheet.as_ref().unwrap();
                let sliders = sheet.sliders.clone();
                match filter {
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
        self.close_sheet(cx);
    }

    // MARK: OK

    pub fn confirm_sheet(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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
                self.session.resize_image_to(width, height, Some(resolution));
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
