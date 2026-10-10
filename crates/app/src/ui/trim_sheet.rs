//! Port of the Trim sheet's body (`TrimSheet.swift`): the Based On picker,
//! the four side checkboxes and the tolerance field.

use gpui_kit::component::input::Input;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, px};

use crate::sheets_ui::SheetRow;
use crate::state::ProjectWorkspaceView;
use crate::ui::{Dropdown, LabeledCheckbox, MenuEntry, app_dispatch, app_on};

impl ProjectWorkspaceView {
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
            .child(
                SheetRow::new("Based On").with_control(
                    Dropdown::new(
                        "trim-based-on",
                        match based_on {
                            1 => "Top Left Pixel Color",
                            2 => "Bottom Right Pixel Color",
                            _ => "Transparent Pixels",
                        },
                    )
                    .with_width(px(210.0))
                    .with_entries(vec![
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
                    ])
                    .with_dispatch(app_dispatch(&app)),
                ),
            )
            .child(
                SheetRow::new("Trim").with_control(
                    gpui_kit::div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(
                            LabeledCheckbox::new("trim-top", "Top")
                                .checked(sides[0])
                                .with_on_change(app_on(&app, make_side_handler(0))),
                        )
                        .child(
                            LabeledCheckbox::new("trim-bottom", "Bottom")
                                .checked(sides[1])
                                .with_on_change(app_on(&app, make_side_handler(1))),
                        )
                        .child(
                            LabeledCheckbox::new("trim-left", "Left")
                                .checked(sides[2])
                                .with_on_change(app_on(&app, make_side_handler(2))),
                        )
                        .child(
                            LabeledCheckbox::new("trim-right", "Right")
                                .checked(sides[3])
                                .with_on_change(app_on(&app, make_side_handler(3))),
                        ),
                ),
            )
            .child(
                SheetRow::new("Tolerance").with_control(
                    Input::new(&sheet.tolerance)
                        .id("trim-tolerance")
                        .w(px(80.0)),
                ),
            )
            .into_any_element()
    }
}

/// One side checkbox's write-back: only that side's flag flips.
fn make_side_handler(
    index: usize,
) -> impl Fn(&mut ProjectWorkspaceView, bool, &mut Window, &mut Context<ProjectWorkspaceView>) + 'static
{
    move |state, checked, _, cx| {
        if let Some(sheet) = state.sheet.as_mut() {
            sheet.sides[index] = checked;
        }
        state.notify_only(cx);
    }
}
