//! The modal sheets themselves: the centered overlay panel, per-sheet bodies,
//! and the OK/Cancel actions (including PNG export, .comp save and open).

use std::cell::RefCell;
use std::collections::BTreeMap;

use gpui_kit::{
    canvas, px, point, prelude::FluentBuilder as _, rgba, size, Bounds, Context, Entity,
    InteractiveElement, IntoElement, Keystroke, KeystrokeEvent, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Point, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::{Disableable, Sizable};

use compositor_core::canvas::{CanvasSizeOptions, TrimBasedOn, TrimOptions};
use compositor_core::adjust::{CurvePoint, CurvesSettings, LevelsChannel};



use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::Bitmap;

use compositor_core::tools::brush::RasterEdit;

use crate::project::UnsavedChoice;

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
        SheetKind::ImageSize => "Image Size",
        SheetKind::CanvasSize => "Canvas Size",
        SheetKind::ExportPng => "Export PNG",
        SheetKind::ExportJpeg => "Export JPEG",
        SheetKind::SaveComp => "Save Project",
        SheetKind::Trim => "Trim",
        SheetKind::About => "About Compositor",
        // Confirm/Alert titles come from the presenting flow at render time.
        SheetKind::Confirm => "Compositor",
        SheetKind::Alert => "Compositor",
    }
}

pub(crate) fn parse_input_usize(input: &Entity<InputState>, cx: &gpui_kit::App) -> Option<usize> {
    input.read(cx).value().trim().parse::<usize>().ok()
}

fn parse_input_f64(input: &Entity<InputState>, cx: &gpui_kit::App) -> Option<f64> {
    input.read(cx).value().trim().parse::<f64>().ok()
}

pub(crate) fn sheet_row(label: &str, control: impl gpui_kit::IntoElement) -> gpui_kit::Div {
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
        // The Shortcuts panel's editor state (baseline draft + recording row +
        // keystroke interceptor) dies with its panel: drop it as soon as the
        // Shortcuts panel is not open.
        if !self
            .panels
            .is_visible(crate::panels::panel_names::KEYBOARD_SHORTCUTS)
        {
            Self::shortcuts_editor_take();
        }
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

        // The Shortcuts sheet swaps the generic OK for its own footer
        // (KeyboardShortcuts.swift:270-278): Restore Defaults, Cancel (which
        // discards the draft) and Done (which saves it).
        // The JPEG export's OK stays disabled until the live encoded preview
        // matches the current options (`readyOptions != options`).
        let ok_disabled = kind == SheetKind::ExportJpeg
            && self
                .sheet
                .as_ref()
                .map(|sheet| sheet.jpeg_ready != Some((self.sheet_value(cx, 0) as f64, sheet.preset)))
                .unwrap_or(false);
        let footer = if kind == SheetKind::Confirm && self.delete_ask_ids.is_some() {
            // The live-mask delete ask (deleteWithLiveMaskChoice's NSAlert,
            // LiveLayerMask.swift:118-121): Bake and Delete / Cancel /
            // Remove Links and Delete.
            gpui_kit::div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(gpui_kit::rgb(0x4a4a4a))
                .child(
                    Button::new("delete-ask-bake")
                        .label("Bake and Delete")
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resolve_delete_ask(true, cx);
                        })),
                )
                .child(
                    Button::new("delete-ask-cancel")
                        .label("Cancel")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resolve_delete_ask_cancel(cx);
                        })),
                )
                .child(
                    Button::new("delete-ask-unlink")
                        .label("Remove Links and Delete")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resolve_delete_ask(false, cx);
                        })),
                )
        } else if kind == SheetKind::Confirm {
            // `confirmReplacement` (:257-259): Save / Cancel / Don't Save.
            gpui_kit::div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(gpui_kit::rgb(0x4a4a4a))
                .child(
                    Button::new("confirm-save")
                        .label("Save")
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.resolve_confirm_choice(UnsavedChoice::Save, window, cx);
                        })),
                )
                .child(
                    Button::new("confirm-cancel")
                        .label("Cancel")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resolve_confirm_cancel(cx);
                        })),
                )
                .child(
                    Button::new("confirm-dont-save")
                        .label("Don't Save")
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.resolve_confirm_choice(UnsavedChoice::DontSave, window, cx);
                        })),
                )
        } else if kind == SheetKind::Alert {
            gpui_kit::div()
                .flex()
                .flex_row()
                .justify_end()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(gpui_kit::rgb(0x4a4a4a))
                .child(
                    Button::new("alert-ok")
                        .label("OK")
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dismiss_error_alert(cx);
                        })),
                )
        } else {
            gpui_kit::div()
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
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_sheet(window, cx)
                        })),
                )
                .child(
                    Button::new("sheet-ok")
                        .label("OK")
                        .small()
                        .primary()
                        .disabled(ok_disabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.confirm_sheet(window, cx);
                        })),
                )
        };

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
            SheetKind::ExportPng | SheetKind::SaveComp => {
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
                // The live encoded preview and its status line
                // (JPEGExportSheet's ZStack + ByteCountFormatter row): OK
                // stays disabled until the preview matches the options.
                let preview = sheet.jpeg_preview.clone();
                let ready = sheet.jpeg_ready == Some((quality as f64, matte));
                let status = if ready {
                    format!("{} · encoded preview, fitted to window", sheet.jpeg_size_label)
                } else {
                    "Updating preview…".to_string()
                };
                body = body
                    .child(sheet_row(
                        "Path",
                        Input::new(&sheet.path).id("sheet-path").w(px(270.0)),
                    ))
                    .child(
                        gpui_kit::div()
                            .w(px(280.0))
                            .h(px(160.0))
                            .bg(gpui_kit::rgb(0x1f1f1f))
                            .rounded_sm()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(preview, |frame, image| {
                                frame.child(gpui_kit::img(image).max_w_full().max_h_full())
                            }),
                    )
                    .child(sheet_row("", text_label(status, true)))
                    .child(sheet_row("Quality", labeled_slider(&sliders, 0, &quality_text)))
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
                                    state.refresh_jpeg_preview(cx);
                                }),
                                MenuEntry::new("Black", |state: &mut AppState, _, cx| {
                                    if let Some(sheet) = state.sheet.as_mut() {
                                        sheet.preset = 1;
                                    }
                                    state.refresh_jpeg_preview(cx);
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
            SheetKind::Confirm | SheetKind::Alert => {
                let message = self.sheet.as_ref().map(|s| s.message.clone()).unwrap_or_default();
                for line in message.lines() {
                    body = body.child(text_label(line.to_string(), false));
                }
                if kind == SheetKind::Confirm {
                    if self.delete_ask_ids.is_some() {
                        // The delete ask's informative text (the alert's
                        // `informativeText`, LiveLayerMask.swift:117).
                        body = body.child(text_label(crate::flows::DELETE_LIVE_MASK_INFO, true));
                    } else {
                        body = body.child(text_label(crate::project::UNSAVED_CHANGES_INFO, true));
                    }
                }
            }
        }
        body.into_any_element()
    }

    pub fn cancel_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The Confirm sheet's Cancel aborts the close/quit flow; the
        // Alert's Cancel is just OK; the controller-driven panels release
        // their pending (with no answer).
        match self.sheet.as_ref().map(|sheet| sheet.kind) {
            Some(SheetKind::Confirm) => {
                // The delete ask's Cancel deletes nothing; the unsaved-changes
                // confirm's Cancel aborts the close/quit flow.
                if self.delete_ask_ids.is_some() {
                    self.resolve_delete_ask_cancel(cx);
                } else {
                    self.resolve_confirm_cancel(cx);
                }
                return;
            }
            Some(SheetKind::Alert) => {
                self.dismiss_error_alert(cx);
                return;
            }
            Some(
                SheetKind::SaveComp
                | SheetKind::ExportPng
                | SheetKind::ExportJpeg
                | SheetKind::ImageSize
                | SheetKind::CanvasSize
                | SheetKind::Trim,
            ) => {
                self.cancel_pending_sheet(window, cx);
                return;
            }
            _ => {}
        }
        // An adjustment editing session restores the opening settings.
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(false);
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

    pub fn confirm_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A finish step may open an Alert (a controller error) in place of
        // this sheet; only close when the sheet is still the one confirmed.
        let kind_before = self.sheet.as_ref().map(|sheet| sheet.kind);
        // Editing an adjustment layer: the settings previewed live, OK keeps
        // them as one undo step and skips the layer-pixel commit paths.
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(true);
            self.close_sheet(cx);
            return;
        }
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return,
        };
        match kind {
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
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_image_size(
                        session,
                        busy,
                        Some(width),
                        Some(height),
                        Some(resolution),
                        compositor_core::transform::LayerSampling::High,
                    );
                });
                self.request_refit();
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
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_canvas_size(session, busy, Some(&options));
                });
                self.request_refit();
            }
            SheetKind::ExportPng => {
                let url = path_or_none(&self.sheet_path(cx));
                self.finish_pending_export_png(url, window, cx);
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
                // The quality of this export starts the next one
                // (JPEGExportSheet's UserDefaults write at Export press).
                crate::settings::lock_global()
                    .set_number(quality, "jpegExportQuality");
                crate::settings::lock_global().save().ok();
                // Export writes the ready preview's own bytes
                // (`finish(result?.data)`); a stale preview encodes fresh.
                let data = {
                    let sheet = self.sheet.as_ref().unwrap();
                    match (&sheet.jpeg_data, sheet.jpeg_ready) {
                        (Some(data), Some(ready)) if ready == (quality, matte) => Some(data.clone()),
                        _ => None,
                    }
                };
                let data = match data {
                    Some(data) => data,
                    None => {
                        let Some(document) = self.current_tab().session.document.clone() else {
                            self.set_status("Nothing to export", cx);
                            return;
                        };
                        let composite = compositor_core::render::draw_live_composite(&document);
                        if let Err(error) = compositor_core::export::check_export_size(
                            composite.width,
                            composite.height,
                        ) {
                            self.set_status(format!("Export failed: {}", error), cx);
                            return;
                        }
                        let options = compositor_core::export::JpegOptions {
                            quality,
                            red: if matte == 1 { 0.0 } else { 1.0 },
                            green: if matte == 1 { 0.0 } else { 1.0 },
                            blue: if matte == 1 { 0.0 } else { 1.0 },
                        };
                        match compositor_core::export::jpeg_data_with_dpi(
                            &composite,
                            &options,
                            document.resolution,
                        ) {
                            Some(encoded) => encoded,
                            None => {
                                self.set_status("Export failed: the image could not be encoded.", cx);
                                return;
                            }
                        }
                    }
                };
                let url = path_or_none(&path);
                self.finish_pending_export_jpeg(url, &data, window, cx);
            }
            SheetKind::SaveComp => {
                let url = path_or_none(&self.sheet_path(cx));
                self.finish_pending_save(url, window, cx);
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
                self.with_current_tab(|controller, session, busy| {
                    controller.finish_trim(session, busy, Some(&options));
                });
                self.request_refit();
            }
            SheetKind::About | SheetKind::Confirm | SheetKind::Alert => {}
        }
        if self.sheet.as_ref().map(|sheet| sheet.kind) == kind_before {
            self.close_sheet(cx);
        }
    }

    fn sheet_path(&self, cx: &Context<Self>) -> String {
        self.sheet
            .as_ref()
            .map(|sheet| sheet.path.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Fills the active layer with one color, honoring the selection.
    pub fn fill_active(&mut self, color: [u8; 4], name: &str, cx: &mut Context<Self>) {
        let Some(document) = self.tab().session.document.clone() else { return };
        let Some(id) = self.tab().session.active_layer_id else { return };
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
        self.commit_bitmap(result, id, layer.transform, name, cx);
    }

    // MARK: File operations

    /// Opens an image (PNG/JPEG) as a new layer, or a .comp package folder.
    pub fn open_document(&mut self, path: &str, cx: &mut Context<Self>) {
        let dir = std::path::PathBuf::from(path);
        if dir.is_dir() || path.ends_with(".comp") {
            match compositor_core::project::load(&dir) {
                Ok(data) => match compositor_core::project::document_from_project(&data) {
                    Ok((document, active)) => {
                        self.tab().session.active_layer_id = active.or_else(|| {
                            document.layers.last().map(|layer| layer.id)
                        });
                        self.tab().session.selected_layer_ids = self
                            .tab().session
                            .active_layer_id
                            .map(|id| vec![id])
                            .unwrap_or_default();
                        self.tab().session.document = Some(document);
                        self.tab().session.history.reset();
                        self.tab().session.mark_saved();
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
            match self.tab().session.import_photoshop_data(&name, &data) {
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
                if self.tab().session.document.is_none() {
                    self.tab().session.create_document(width, height, true);
                }
                let layer = compositor_core::document::ImageLayer::with_asset(
                    compositor_core::raster::ImportedImage::new(bitmap, name),
                    CGPoint::ZERO,
                );
                let id = layer.id;
                self.history_edit("Open", move |session| {
                    if let Some(document) = session.document.as_mut() {
                        let slot = document.layers.len();
                        document.layers.insert(slot, layer);
                    }
                    session.select_layer(Some(id));
                });
                self.fit_for = (0, 0);
                self.set_status(format!("Opened {}", path), cx);
            }
            Err(error) => self.set_status(format!("Open failed: {}", error), cx),
        }
    }
}

// MARK: Shortcuts settings sheet (KeyboardShortcuts.swift:129-319)

/// The Shortcuts sheet's editor state — Swift's `@State draft` and
/// `@State recording` plus the recorder's key grab (KeyboardShortcutsSheet's
/// fields, lines 230-232). It lives beside the sheet instead of in
/// `SheetState` because that struct (state.rs) is shared with every other
/// sheet and the port keeps its shape frozen; it is thread-local because the
/// recorder's live keystroke interceptor (`Subscription`) is not `Sync`, and
/// every one of these touch points — rendering, clicks, key dispatch — runs
/// on the main thread.
struct ShortcutsSheetEditor {
    /// Overrides when the sheet opened. The sheet edits the live registry as
    /// its draft (each row goes through `set_override`/`reset`); Cancel
    /// restores this snapshot, Done saves it.
    baseline: BTreeMap<String, String>,
    /// Catalog id of the row recording a new chord, if any.
    recording: Option<String>,
    /// The app-wide keystroke interceptor that feeds the recorder while the
    /// sheet is open; dropping it releases the keyboard.
    _capture: Option<Subscription>,
}

thread_local! {
    static SHORTCUTS_SHEET: RefCell<Option<ShortcutsSheetEditor>> =
        const { RefCell::new(None) };
}

/// What the recorder does with one captured key press
/// (ShortcutRecorder's `keyDown`, KeyboardShortcuts.swift:310-317).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecorderCapture {
    /// A single key chord: finish the recording and assign it.
    Finish(String),
    /// A bare modifier press or an unparsable chord: keep recording (Swift's
    /// `NSSound.beep()`).
    Ignore,
}

/// Classifies a captured keystroke's text (gpui `Keystroke::unparse` form,
/// e.g. `"ctrl-shift-s"`) for the recorder.
pub fn recorder_capture(chord_text: &str) -> RecorderCapture {
    // gpui synthesizes keystrokes keyed by the bare modifier's name when a
    // modifier is pressed alone; those never finish a recording.
    if matches!(chord_text, "control" | "alt" | "shift" | "platform" | "function") {
        return RecorderCapture::Ignore;
    }
    // Exactly one chord, no `key->char` test syntax — the same shape
    // `shortcuts::set_override` will accept.
    if chord_text.split_whitespace().count() > 1 {
        return RecorderCapture::Ignore;
    }
    match Keystroke::parse(chord_text) {
        Ok(chord) if !chord.key.is_empty() && chord.key_char.is_none() => {
            RecorderCapture::Finish(chord.unparse())
        }
        _ => RecorderCapture::Ignore,
    }
}

/// Restores an overrides snapshot into the live registry (the Cancel path).
/// Entries were normalized when they were first stored, so `set_override`
/// cannot fail here; failures would leave that entry at its default.
pub fn restore_overrides(baseline: BTreeMap<String, String>) {
    let mut registry = crate::shortcuts::lock_global();
    registry.reset_all();
    for (id, chord) in &baseline {
        let _ = registry.set_override(id, chord);
    }
}

/// A row's chord for display (`ShortcutChord.label`): an unparsable stored
/// chord prints raw instead of failing the sheet.
fn display_chord(chord: &str) -> String {
    Keystroke::parse(chord)
        .map(|parsed| crate::shortcuts::chord_label(&parsed))
        .unwrap_or_else(|_| chord.to_string())
}

impl AppState {
    /// Creates the sheet's editor on first body render: the baseline is the
    /// registry's overrides, and the app-wide keystroke interceptor is
    /// installed for the sheet's lifetime (it only acts while a row is
    /// recording).
    fn shortcuts_editor_init(&mut self, cx: &mut Context<Self>) {
        let initialized = SHORTCUTS_SHEET.with_borrow(|slot| slot.is_some());
        if initialized {
            return;
        }
        let baseline = self.shortcuts().overrides().clone();
        let app = cx.entity().downgrade();
        let subscription = cx.intercept_keystrokes(move |event: &KeystrokeEvent, _, cx| {
            shortcuts_handle_keystroke(&app, event, cx);
        });
        SHORTCUTS_SHEET.with_borrow_mut(|slot| {
            *slot = Some(ShortcutsSheetEditor {
                baseline,
                recording: None,
                _capture: Some(subscription),
            });
        });
    }

    /// Drops the sheet's editor state (and with it the keystroke
    /// interceptor). Called from `render_sheet_overlay` whenever the Shortcuts
    /// sheet is not the open one.
    fn shortcuts_editor_take() -> Option<ShortcutsSheetEditor> {
        SHORTCUTS_SHEET.with_borrow_mut(|slot| slot.take())
    }

    /// The id of the row currently recording, if any.
    pub(crate) fn shortcuts_recording_id() -> Option<String> {
        SHORTCUTS_SHEET.with_borrow(|slot| {
            slot.as_ref().and_then(|editor| editor.recording.clone())
        })
    }

    /// Discards the draft: the registry returns to its state when the sheet
    /// opened, and the editor state drops.
    pub(crate) fn shortcuts_cancel_draft() {
        let baseline = Self::shortcuts_editor_take().map(|editor| editor.baseline);
        if let Some(baseline) = baseline {
            restore_overrides(baseline);
        }
    }

    /// A row's chord button was clicked: that row records the next keystroke.
    pub fn shortcuts_start_recording(&mut self, id: &'static str, cx: &mut Context<Self>) {
        SHORTCUTS_SHEET.with_borrow_mut(|slot| {
            if let Some(editor) = slot.as_mut() {
                editor.recording = Some(id.to_string());
            }
        });
        self.notify_only(cx);
    }

    /// The footer's Done: persists the draft (`ShortcutSettings.save`) and
    /// closes. The keymap itself is rebuilt at startup, so new chords take
    /// effect immediately (the keymap rebuilds from the registry).
    pub(crate) fn shortcuts_done(&mut self, cx: &mut Context<Self>) {
        match self.shortcuts().save() {
            Ok(()) => self.set_status("Shortcuts saved", cx),
            Err(error) => self.set_status(format!("Saving shortcuts failed: {error}"), cx),
        }
        Self::shortcuts_editor_take();
        self.close_shortcuts_panel(cx);
    }

    /// The sheet body (KeyboardShortcutsSheet's `body`): an intro line, the
    /// catalog grouped like Swift's ForEach over "Menus", "Canvas & Layers"
    /// and "Text Editing", and the conflict readout above the footer.
    pub(crate) fn shortcuts_sheet_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        self.shortcuts_editor_init(cx);
        let problem = self.shortcuts().validate().err();
        let recording = Self::shortcuts_recording_id();

        // Precompute every row's data while the registry is locked, then
        // build the elements.
        let rows: Vec<(&'static crate::shortcuts::ShortcutSpec, String, bool, bool)> = {
            let registry = self.shortcuts();
            let mut rows = Vec::new();
            for group in [
                crate::shortcuts::Group::Menus,
                crate::shortcuts::Group::CanvasLayers,
                crate::shortcuts::Group::TextEditing,
            ] {
                for entry in crate::shortcuts::entries_in_group(group) {
                    let chord = display_chord(&registry.chord(entry.id));
                    let customized = registry.is_customized(entry.id);
                    let is_recording = recording.as_deref() == Some(entry.id);
                    rows.push((entry, chord, customized, is_recording));
                }
            }
            rows
        };

        let mut list = gpui_kit::div()
            .id("shortcuts-list")
            .flex()
            .flex_col()
            .gap_0p5()
            .max_h(px(380.0))
            .overflow_y_scroll();
        let mut current_group: Option<crate::shortcuts::Group> = None;
        for (entry, chord, customized, is_recording) in rows {
            if current_group != Some(entry.group) {
                current_group = Some(entry.group);
                list = list.child(
                    text_label(entry.group.label(), false)
                        .text_size(px(12.0))
                        .mt_2(),
                );
            }
            list = list.child(shortcut_row(entry, &chord, customized, is_recording, cx));
        }
        list = list.child(
            text_label(
                "Text fields keep standard editing keys. The shortcut editor itself uses Return to save and Esc to cancel when not recording.",
                true,
            )
            .mt_2(),
        );

        let mut column = gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(text_label(
                "Click a shortcut, then press its new key combination. Changes apply immediately.",
                true,
            ))
            .child(list);
        if let Some(problem) = problem {
            column = column.child(
                text_label(problem.to_string(), false).text_color(gpui_kit::rgb(0xe0a24a)),
            );
        }
        column.into_any_element()
    }
}

/// One catalog row: the action's title, its chord button (click to record)
/// and a per-row Reset (`ShortcutRecorder` framed at 150x26 in Swift).
fn shortcut_row(
    entry: &'static crate::shortcuts::ShortcutSpec,
    chord: &str,
    customized: bool,
    is_recording: bool,
    cx: &mut Context<AppState>,
) -> gpui_kit::Div {
    let id = entry.id;
    gpui_kit::div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .py_0p5()
        .child(
            text_label(entry.title.to_string(), false)
                .flex_1()
                .min_w_0()
                .text_ellipsis(),
        )
        .child(
            Button::new(SharedString::from(format!("sc-chord-{id}")))
                .label(if is_recording {
                    "Press keys…".to_string()
                } else {
                    chord.to_string()
                })
                .small()
                .outline()
                .when(is_recording, |button| {
                    button.text_color(gpui_kit::rgb(crate::ui::ACCENT))
                })
                .on_click(cx.listener(move |state, _, _, cx| {
                    state.shortcuts_start_recording(id, cx);
                })),
        )
        .child(
            Button::new(SharedString::from(format!("sc-reset-{id}")))
                .label("Reset")
                .small()
                .outline()
                .disabled(!customized)
                .on_click(cx.listener(move |state, _, _, cx| {
                    state.shortcuts().reset(id);
                    state.notify_only(cx);
                })),
        )
}

/// The recorder's keystroke hook (KeyboardShortcuts.swift:306-317's
/// `performKeyEquivalent`): while a row records, every keystroke is swallowed
/// (`cx.stop_propagation`, which also keeps actions from dispatching); a
/// single-key chord is assigned to the row and ends the recording, a bare
/// modifier or garbage keeps waiting.
fn shortcuts_handle_keystroke(
    app: &WeakEntity<AppState>,
    event: &KeystrokeEvent,
    cx: &mut gpui_kit::App,
) {
    let chord_text = event.keystroke.unparse();
    let finished = SHORTCUTS_SHEET.with_borrow_mut(|slot| {
        let Some(editor) = slot.as_mut() else {
            return None;
        };
        if editor.recording.is_none() {
            // The sheet is open but nothing records: the keystroke is
            // nobody's business.
            return None;
        }
        // Recording: swallow the keystroke either way.
        cx.stop_propagation();
        match recorder_capture(&chord_text) {
            RecorderCapture::Ignore => None,
            RecorderCapture::Finish(chord) => {
                let id = editor.recording.take().unwrap();
                Some((id, chord))
            }
        }
    });
    let Some((id, chord)) = finished else { return };
    let app = app.clone();
    let _ = app.update(cx, move |state, cx| {
        if let Err(problem) = state.shortcuts().set_override(&id, &chord) {
            state.set_status(problem.to_string(), cx);
        } else {
            state.notify_only(cx);
        }
    });
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
pub(crate) fn curves_canvas_view(
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
pub(crate) fn labeled_slider(
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
pub(crate) fn histogram_view(histogram: [f64; 1024]) -> impl IntoElement {
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

#[cfg(test)]
mod tests {
    use super::{
        curve_drag_begin, curve_drag_move, curve_hit_point, curve_point_from_event,
        curve_value_to_screen, display_chord, recorder_capture, restore_overrides,
        levels_channel_label, CurveDragBegin, RecorderCapture,
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

    #[test]
    fn recorder_finishes_on_single_chords() {
        // Plain keys, modifiers, arrows and punctuation all finish the
        // recording, normalized to the canonical chord text.
        assert_eq!(recorder_capture("v"), RecorderCapture::Finish("v".into()));
        assert_eq!(
            recorder_capture("ctrl-shift-s"),
            RecorderCapture::Finish("ctrl-shift-s".into())
        );
        assert_eq!(
            recorder_capture("shift-u"),
            RecorderCapture::Finish("shift-u".into())
        );
        assert_eq!(
            recorder_capture("alt-delete"),
            RecorderCapture::Finish("alt-delete".into())
        );
        // A capital letter parses as the shifted key (KeyboardShortcuts.swift's
        // `key.uppercased()` inverse).
        assert_eq!(
            recorder_capture("A"),
            RecorderCapture::Finish("shift-a".into())
        );
    }

    #[test]
    fn recorder_ignores_bare_modifiers_and_garbage() {
        // gpui synthesizes these keystrokes when a modifier is pressed alone:
        // the recording keeps waiting (Swift's `NSSound.beep()`).
        for modifier in ["control", "alt", "shift", "platform", "function"] {
            assert_eq!(
                recorder_capture(modifier),
                RecorderCapture::Ignore,
                "bare {modifier}"
            );
        }
        // Multi-key sequences and the test-only `key->char` syntax never
        // finish a recording either.
        assert_eq!(
            recorder_capture("ctrl-x ctrl-c"),
            RecorderCapture::Ignore
        );
        assert_eq!(recorder_capture("a->b"), RecorderCapture::Ignore);
        assert_eq!(recorder_capture(""), RecorderCapture::Ignore);
    }

    #[test]
    fn restore_overrides_returns_the_registry_to_its_baseline() {
        use crate::shortcuts::ShortcutRegistry;

        // The Cancel path restores the process-wide registry, so remember
        // whatever it held and put it back at the end (the test never saves
        // to disk).
        let original = {
            let registry = crate::shortcuts::lock_global();
            registry.overrides().clone()
        };
        let result = std::panic::catch_unwind(|| {
            // The draft's opening state: one override.
            let mut baseline_registry = ShortcutRegistry::new();
            baseline_registry
                .set_override("edit.undo", "ctrl-alt-z")
                .unwrap();
            let baseline = baseline_registry.overrides().clone();

            // The live registry has drifted since the sheet opened.
            {
                let mut registry = crate::shortcuts::lock_global();
                registry.set_override("tool.brush", "k").unwrap();
                registry.set_override("edit.redo", "ctrl-y").unwrap();
            }

            restore_overrides(baseline);

            let registry = crate::shortcuts::lock_global();
            assert_eq!(registry.chord("edit.undo"), "ctrl-alt-z");
            assert_eq!(registry.chord("tool.brush"), "b", "drift is discarded");
            assert_eq!(registry.chord("edit.redo"), "ctrl-shift-z", "drift is discarded");
            assert!(registry.validate().is_ok());
        });
        restore_overrides(original);
        result.unwrap();
    }

    #[test]
    fn display_chord_uses_platform_notation() {
        assert_eq!(display_chord("ctrl-shift-s"), "Ctrl+Shift+S");
        assert_eq!(display_chord("escape"), "Esc");
        assert_eq!(display_chord("v"), "V");
        // Unparsable stored chords print raw instead of failing the sheet.
        assert_eq!(display_chord("???"), "???");
    }
}

/// A sheet path field's value as an optional save target (empty = Cancel).
fn path_or_none(path: &str) -> Option<std::path::PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(trimmed))
    }
}
