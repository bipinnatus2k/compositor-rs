//! Port of the Image Size sheet's body (`ImageSizeSheet.swift`): the width,
//! height, unit, resolution and constrain rows the sheet host re-renders.

use gpui_kit::component::input::Input;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, px};

use crate::sheets_ui::SheetRow;
use crate::state::ProjectWorkspaceView;
use crate::ui::{LabeledCheckbox, app_on, text_label};

impl ProjectWorkspaceView {
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
            .child(
                SheetRow::new("Width")
                    .with_control(Input::new(&sheet.width).id("size-width").w(px(120.0))),
            )
            .child(
                SheetRow::new("Height")
                    .with_control(Input::new(&sheet.height).id("size-height").w(px(120.0))),
            )
            .child(
                SheetRow::new("Unit").with_control(
                    LabeledCheckbox::new("size-percent", "Percent")
                        .checked(percent)
                        .with_on_change(app_on(&app, |state, checked, _, cx| {
                            if let Some(sheet) = state.sheet.as_mut() {
                                sheet.percent = checked;
                            }
                            state.notify_only(cx);
                        })),
                ),
            )
            .child(
                SheetRow::new("Resolution").with_control(
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
                ),
            )
            .child(
                SheetRow::new("Constrain").with_control(
                    LabeledCheckbox::new("size-link", "Link width and height (keep aspect)")
                        .checked(link)
                        .with_on_change(app_on(&app, |state, checked, _, cx| {
                            if let Some(sheet) = state.sheet.as_mut() {
                                sheet.link = checked;
                            }
                            state.notify_only(cx);
                        })),
                ),
            )
            .into_any_element()
    }
}
