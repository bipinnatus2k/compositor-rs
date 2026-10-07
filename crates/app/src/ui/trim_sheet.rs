//! Port of the Trim sheet's body (`TrimSheet.swift`): the Based On picker,
//! the four side checkboxes and the tolerance field.

use gpui_kit::component::input::Input;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, px};

use crate::sheets_ui::sheet_row;
use crate::state::AppState;
use crate::ui::{MenuEntry, dropdown, labeled_checkbox};

impl AppState {
    /// The Trim sheet's body: the Based On dropdown, the four side
    /// checkboxes and the tolerance field.
    pub(crate) fn trim_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let app = cx.entity().downgrade();
        let sheet = self.sheet.as_ref().unwrap();
        let based_on = sheet.based_on;
        let sides = sheet.sides;

        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
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
                        labeled_checkbox(&app, "trim-top", "Top", sides[0], make_side_handler(0))
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
                        labeled_checkbox(&app, "trim-left", "Left", sides[2], make_side_handler(2))
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
                Input::new(&sheet.tolerance)
                    .id("trim-tolerance")
                    .w(px(80.0)),
            ))
            .into_any_element()
    }
}

/// One side checkbox's write-back: only that side's flag flips.
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
