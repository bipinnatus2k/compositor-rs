//! Port of the Image Size sheet's body (`ImageSizeSheet.swift`): the width,
//! height, unit, resolution and constrain rows the sheet host re-renders.

use gpui_kit::component::input::Input;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, px};

use crate::sheets_ui::sheet_row;
use crate::state::AppState;
use crate::ui::{labeled_checkbox, text_label};

impl AppState {
    /// The Image Size sheet's body: width/height fields, the Percent unit
    /// checkbox, the resolution field and the aspect-link constraint.
    pub(crate) fn image_size_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let sheet = self.sheet.as_ref().unwrap();
        let percent = sheet.percent;
        let link = sheet.link;

        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
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
                    .child(
                        Input::new(&sheet.resolution)
                            .id("size-resolution")
                            .w(px(80.0)),
                    )
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
            ))
            .into_any_element()
    }
}
