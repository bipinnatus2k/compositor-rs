//! Port of the Canvas Size sheet's body (`CanvasSizeSheet.swift`): the
//! width/height fields, the Relative checkbox and the nine-cell anchor grid.

use gpui_kit::component::input::Input;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, prelude::FluentBuilder as _, px,
};

use crate::sheets_ui::SheetRow;
use crate::state::AppState;
use crate::ui::{LabeledCheckbox, app_on_change};

impl AppState {
    /// The Canvas Size sheet's body: width/height fields, the Relative
    /// checkbox and the anchor grid (which corner grows toward).
    pub(crate) fn canvas_size_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let sheet = self.sheet.as_ref().unwrap();
        let anchor = sheet.anchor;
        let relative = sheet.relative;

        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                SheetRow::new("Width")
                    .with_control(Input::new(&sheet.width).id("canvas-width").w(px(120.0))),
            )
            .child(
                SheetRow::new("Height")
                    .with_control(Input::new(&sheet.height).id("canvas-height").w(px(120.0))),
            )
            .child(
                SheetRow::new("Relative").with_control(
                    LabeledCheckbox::new("canvas-relative", "Relative (add to current size)")
                        .checked(relative)
                        .with_on_change(app_on_change(&app, |state, checked, _, cx| {
                            if let Some(sheet) = state.sheet.as_mut() {
                                sheet.relative = checked;
                            }
                            state.notify_only(cx);
                        })),
                ),
            )
            .child(SheetRow::new("Anchor").with_control(anchor_picker(anchor, cx)))
            .into_any_element()
    }
}

/// The anchor grid: nine cells picking the edge the canvas grows from.
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
                .border_color(gpui_kit::rgb(crate::ui::FG_DISABLED))
                .when(index == anchor, |cell| {
                    cell.bg(gpui_kit::rgb(crate::ui::ACCENT))
                })
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
