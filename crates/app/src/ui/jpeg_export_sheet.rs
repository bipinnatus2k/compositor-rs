//! Port of the JPEG export sheet's body (`JPEGExportSheet.swift`): the path
//! field, the live encoded preview well, the quality slider and the matte
//! dropdown.

use gpui_kit::component::input::Input;
use gpui_kit::component::slider::Slider;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, prelude::FluentBuilder as _, px};

use crate::sheets_ui::SheetRow;
use crate::state::ProjectWorkspaceView;
use crate::ui::{Dropdown, MenuEntry, app_dispatch, text_label};

impl ProjectWorkspaceView {
    /// The JPEG export sheet's body: path field, the live encoded preview
    /// (JPEGExportSheet's ZStack + ByteCountFormatter row), the quality
    /// slider and the matte dropdown with its flatten note.
    pub(crate) fn jpeg_export_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let sheet = self.sheet.as_ref().unwrap();
        let matte = sheet.preset;
        let quality = {
            let cx_app: &gpui_kit::App = cx;
            self.sheet_value(cx_app, 0)
        };
        let quality_text = format!("{}%", (quality * 100.0).round() as i32);
        let sliders = sheet.sliders.clone();
        // The live encoded preview and its status line: OK stays disabled
        // until the preview matches the options.
        let preview = sheet.jpeg_preview.clone();
        let ready = sheet.jpeg_ready == Some((quality as f64, matte));
        let status = if ready {
            format!(
                "{} · encoded preview, fitted to window",
                sheet.jpeg_size_label
            )
        } else {
            "Updating preview…".to_string()
        };

        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                SheetRow::new("Path")
                    .with_control(Input::new(&sheet.path).id("sheet-path").w(px(270.0))),
            )
            .child(
                gpui_kit::div()
                    .w(px(280.0))
                    .h(px(160.0))
                    .bg(gpui_kit::rgb(crate::ui::BG_INSET))
                    .rounded_sm()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(preview, |frame, image| {
                        frame.child(gpui_kit::img(image).max_w_full().max_h_full())
                    }),
            )
            .child(SheetRow::new("").with_control(text_label(status, true)))
            .child(SheetRow::new("Quality").with_control(
                SheetRow::new(quality_text).with_optional_control(
                    sliders.first().map(|slider| Slider::new(slider).flex_1()),
                ),
            ))
            .child(
                SheetRow::new("Matte").with_control(
                    Dropdown::new("jpeg-matte", if matte == 1 { "Black" } else { "White" })
                        .with_width(px(140.0))
                        .with_entries(vec![
                            MenuEntry::new("White", |state: &mut ProjectWorkspaceView, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.preset = 0;
                                }
                                state.refresh_jpeg_preview(cx);
                            }),
                            MenuEntry::new("Black", |state: &mut ProjectWorkspaceView, _, cx| {
                                if let Some(sheet) = state.sheet.as_mut() {
                                    sheet.preset = 1;
                                }
                                state.refresh_jpeg_preview(cx);
                            }),
                        ])
                        .with_dispatch(app_dispatch(&app)),
                ),
            )
            .child(SheetRow::new("").with_control(text_label(
                "Transparency flattens onto the matte color. JPEG has no alpha.",
                true,
            )))
            .into_any_element()
    }
}
