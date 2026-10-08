//! The sheet host: the centered overlay panel with its title bar and
//! footer, the per-sheet body dispatcher, the OK/Cancel actions, and the
//! shared [`SheetRow`]. Each sheet's content lives beside its Swift
//! source file (`ui/*_sheet.rs`); the arms with no Swift sheet view — the
//! port's NSAlert/NSSavePanel stand-ins — keep their bodies here.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, Window, canvas, point, px, rgba, size,
};

use compositor_core::canvas::{CanvasSizeOptions, TrimBasedOn, TrimOptions};

use compositor_core::geometry::CGSize;

use compositor_core::tools::brush::RasterEdit;

use crate::project::UnsavedChoice;

use crate::state::{AppState, Sheet};
use crate::ui::text_label;

fn sheet_title(kind: Sheet) -> &'static str {
    match kind {
        Sheet::ImageSize => "Image Size",
        Sheet::CanvasSize => "Canvas Size",
        Sheet::ExportPng => "Export PNG",
        Sheet::ExportJpeg => "Export JPEG",
        Sheet::SaveComp => "Save Project",
        Sheet::Trim => "Trim",
        Sheet::About => "About Compositor",
        // Confirm/Alert titles come from the presenting flow at render time.
        Sheet::Confirm => "Compositor",
        Sheet::Alert => "Compositor",
    }
}

pub(crate) fn parse_input_usize(input: &Entity<InputState>, cx: &gpui_kit::App) -> Option<usize> {
    input.read(cx).value().trim().parse::<usize>().ok()
}

fn parse_input_f64(input: &Entity<InputState>, cx: &gpui_kit::App) -> Option<f64> {
    input.read(cx).value().trim().parse::<f64>().ok()
}

/// One sheet row: the fixed 110px dim label column, vertical padding, and
/// the row's control (`CanvasSizeSheet` and friends' `sheet` rows).
#[derive(IntoElement)]
pub struct SheetRow {
    label: SharedString,
    control: Option<AnyElement>,
}

impl SheetRow {
    pub fn new(label: impl Into<SharedString>) -> Self {
        SheetRow {
            label: label.into(),
            control: None,
        }
    }

    pub fn with_control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    /// The row's control, when the caller's source may be absent (a sheet
    /// whose slider list ran out).
    pub fn with_optional_control(mut self, control: Option<impl IntoElement>) -> Self {
        self.control = control.map(|control| control.into_any_element());
        self
    }
}

impl RenderOnce for SheetRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
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
                    .child(text_label(self.label, true)),
            )
            .children(self.control)
    }
}

impl AppState {
    // MARK: Sheet rendering

    pub fn render_sheet_overlay(&mut self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        // The Shortcuts panel's editor state (baseline draft + recording row +
        // keystroke interceptor) dies with its panel: drop it as soon as the
        // Shortcuts panel is not open.
        if !self
            .panels
            .is_visible(crate::ui::floating_panel::panel_names::KEYBOARD_SHORTCUTS)
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
            .border_color(gpui_kit::rgb(crate::ui::BORDER))
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
        let ok_disabled = kind == Sheet::ExportJpeg
            && self
                .sheet
                .as_ref()
                .map(|sheet| {
                    sheet.jpeg_ready != Some((self.sheet_value(cx, 0) as f64, sheet.preset))
                })
                .unwrap_or(false);
        let footer = if kind == Sheet::Confirm && self.delete_ask_ids.is_some() {
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
                .border_color(gpui_kit::rgb(crate::ui::BORDER))
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
        } else if kind == Sheet::Confirm {
            // `confirmReplacement` (:257-259): Save / Cancel / Don't Save.
            gpui_kit::div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(gpui_kit::rgb(crate::ui::BORDER))
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
        } else if kind == Sheet::Alert {
            gpui_kit::div()
                .flex()
                .flex_row()
                .justify_end()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(gpui_kit::rgb(crate::ui::BORDER))
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
                .border_color(gpui_kit::rgb(crate::ui::BORDER))
                .child(
                    Button::new("sheet-cancel")
                        .label("Cancel")
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| this.cancel_sheet(window, cx))),
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
            .bg(rgba(crate::ui::SCRIM))
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
                    .border_color(gpui_kit::rgb(crate::ui::BORDER))
                    .bg(gpui_kit::rgb(crate::ui::BG_PANEL))
                    .overflow_hidden()
                    .child(title_bar)
                    .child(body)
                    .child(footer),
            );
        Some(overlay.into_any_element())
    }

    /// The body dispatcher: each sheet's content lives beside its Swift
    /// source file. The three arms with no Swift sheet file — the port's
    /// NSAlert/NSSavePanel stand-ins — keep their bodies in this host.
    fn sheet_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return gpui_kit::div().into_any_element(),
        };
        match kind {
            Sheet::ImageSize => self.image_size_body(cx),
            Sheet::CanvasSize => self.canvas_size_body(cx),
            Sheet::ExportPng | Sheet::SaveComp => self.path_prompt_body(cx),
            Sheet::ExportJpeg => self.jpeg_export_body(cx),
            Sheet::Trim => self.trim_body(cx),
            Sheet::About => self.about_body(cx),
            Sheet::Confirm | Sheet::Alert => self.alert_body(cx),
        }
    }

    /// The PNG-export / .comp-save path prompt. Swift drives both flows
    /// through `NSSavePanel` (ImageExporter.swift / ProjectStore.swift) and
    /// has no sheet view for them; the port's path field is the recorded
    /// stand-in, so the body lives with the sheet host.
    fn path_prompt_body(&mut self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return gpui_kit::div().into_any_element(),
        };
        let sheet = self.sheet.as_ref().unwrap();
        let hint = match kind {
            Sheet::ExportPng => "The composited canvas is written as a PNG.",
            Sheet::SaveComp => "The project is saved as a .comp package folder.",
            _ => "PNG/JPEG images open as a new layer; .comp folders open as projects.",
        };
        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                SheetRow::new("Path")
                    .with_control(Input::new(&sheet.path).id("sheet-path").w(px(270.0))),
            )
            .child(SheetRow::new("").with_control(text_label(hint, true)))
            .into_any_element()
    }

    /// The About sheet's body: the app name and one tagline (Swift shows the
    /// standard about window instead — the deviation H-ui-app.md records).
    fn about_body(&mut self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_2()
            .child(text_label("Compositor", false).text_size(px(14.0)))
            .child(text_label(
                "A Photoshop-like image editor built on the compositor-core engine and GPUI.",
                true,
            ))
            .into_any_element()
    }

    /// The Confirm/Alert message body: the alert's `messageText` lines plus
    /// the Confirm sheet's informative line.
    fn alert_body(&mut self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return gpui_kit::div().into_any_element(),
        };
        let message = self
            .sheet
            .as_ref()
            .map(|s| s.message.clone())
            .unwrap_or_default();
        let mut body = gpui_kit::div().flex().flex_col().gap_1();
        for line in message.lines() {
            body = body.child(text_label(line.to_string(), false));
        }
        if kind == Sheet::Confirm {
            if self.delete_ask_ids.is_some() {
                // The delete ask's informative text (the alert's
                // `informativeText`, LiveLayerMask.swift:117).
                body = body.child(text_label(crate::flows::DELETE_LIVE_MASK_INFO, true));
            } else {
                body = body.child(text_label(crate::project::UNSAVED_CHANGES_INFO, true));
            }
        }
        body.into_any_element()
    }

    pub fn cancel_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The Confirm sheet's Cancel aborts the close/quit flow; the
        // Alert's Cancel is just OK; the controller-driven panels release
        // their pending (with no answer).
        match self.sheet.as_ref().map(|sheet| sheet.kind) {
            Some(Sheet::Confirm) => {
                // The delete ask's Cancel deletes nothing; the unsaved-changes
                // confirm's Cancel aborts the close/quit flow.
                if self.delete_ask_ids.is_some() {
                    self.resolve_delete_ask_cancel(cx);
                } else {
                    self.resolve_confirm_cancel(cx);
                }
                return;
            }
            Some(Sheet::Alert) => {
                self.dismiss_error_alert(cx);
                return;
            }
            Some(
                Sheet::SaveComp
                | Sheet::ExportPng
                | Sheet::ExportJpeg
                | Sheet::ImageSize
                | Sheet::CanvasSize
                | Sheet::Trim,
            ) => {
                self.cancel_pending_sheet(window, cx);
                return;
            }
            _ => {}
        }
        // An adjustment editing session restores the opening settings.
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(false));
        }
        self.close_sheet(cx);
    }

    // MARK: OK

    pub fn confirm_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A finish step may open an Alert (a controller error) in place of
        // this sheet; only close when the sheet is still the one confirmed.
        let kind_before = self.sheet.as_ref().map(|sheet| sheet.kind);
        // Editing an adjustment layer: the settings previewed live, OK keeps
        // them as one undo step and skips the layer-pixel commit paths.
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(true));
            self.close_sheet(cx);
            return;
        }
        let kind = match self.sheet.as_ref() {
            Some(sheet) => sheet.kind,
            None => return,
        };
        match kind {
            Sheet::ImageSize => {
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
                    let (w, h) = self.doc_size(cx);
                    width = ((w * width as f64) / 100.0).round() as usize;
                    height = ((h * height as f64) / 100.0).round() as usize;
                }
                self.with_current_tab(cx, |controller, session| {
                    controller.finish_image_size(
                        session,
                        Some(width),
                        Some(height),
                        Some(resolution),
                        compositor_core::transform::LayerSampling::High,
                    );
                });
                self.request_refit();
            }
            Sheet::CanvasSize => {
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
                    let (w, h) = self.doc_size(cx);
                    width += w as usize;
                    height += h as usize;
                }
                let mut options = CanvasSizeOptions::new(width.max(1), height.max(1));
                options.anchor = anchor.min(8);
                self.with_current_tab(cx, |controller, session| {
                    controller.finish_canvas_size(session, Some(&options));
                });
                self.request_refit();
            }
            Sheet::ExportPng => {
                let url = path_or_none(&self.sheet_path(cx));
                self.finish_pending_export_png(url, window, cx);
            }
            Sheet::ExportJpeg => {
                let (path, quality, matte) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        sheet.path.read(cx_app).value().to_string(),
                        sheet
                            .sliders
                            .first()
                            .map(|s| s.read(cx_app).value().end())
                            .unwrap_or(0.85) as f64,
                        sheet.preset,
                    )
                };
                // The quality of this export starts the next one
                // (JPEGExportSheet's UserDefaults write at Export press).
                crate::settings::lock_global().set_number(quality, "jpegExportQuality");
                crate::settings::lock_global().save().ok();
                // Export writes the ready preview's own bytes
                // (`finish(result?.data)`); a stale preview encodes fresh.
                let data = {
                    let sheet = self.sheet.as_ref().unwrap();
                    match (&sheet.jpeg_data, sheet.jpeg_ready) {
                        (Some(data), Some(ready)) if ready == (quality, matte) => {
                            Some(data.clone())
                        }
                        _ => None,
                    }
                };
                let data = match data {
                    Some(data) => data,
                    None => {
                        let Some(document) = self.current_tab().session.read(cx).document.clone()
                        else {
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
                                self.set_status(
                                    "Export failed: the image could not be encoded.",
                                    cx,
                                );
                                return;
                            }
                        }
                    }
                };
                let url = path_or_none(&path);
                self.finish_pending_export_jpeg(url, &data, window, cx);
            }
            Sheet::SaveComp => {
                let url = path_or_none(&self.sheet_path(cx));
                self.finish_pending_save(url, window, cx);
            }
            Sheet::Trim => {
                let (based_on, sides, tolerance) = {
                    let cx_app: &gpui_kit::App = cx;
                    let sheet = self.sheet.as_ref().unwrap();
                    (
                        sheet.based_on,
                        sheet.sides,
                        parse_input_usize(&sheet.tolerance, cx_app)
                            .unwrap_or(0)
                            .min(255) as u8,
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
                self.with_current_tab(cx, |controller, session| {
                    controller.finish_trim(session, Some(&options));
                });
                self.request_refit();
            }
            Sheet::About | Sheet::Confirm | Sheet::Alert => {}
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
        let Some(document) = self.tab().session.read(cx).document.clone() else {
            return;
        };
        let Some(id) = self.tab().session.read(cx).active_layer_id else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
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
}

// MARK: Helpers

/// The Levels histogram, painted as thin bars.
#[derive(IntoElement)]
pub struct HistogramView {
    histogram: [f64; 1024],
}

impl HistogramView {
    pub fn new(histogram: [f64; 1024]) -> Self {
        HistogramView { histogram }
    }
}

impl RenderOnce for HistogramView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let histogram = self.histogram;
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
