//! Port of the crop tool's option bar (`CropControls.swift`): the ratio
//! picker and the Apply/Cancel pair for the in-canvas crop frame.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{Context, IntoElement, px};

use crate::state::AppState;
use crate::ui::dropdown;
use crate::ui::navigation_tool_header::menu_entries;

impl AppState {
    /// `CropControls.swift`'s bar controls: the aspect-ratio dropdown and
    /// the Apply/Cancel buttons (both dead while no crop frame exists).
    pub(crate) fn crop_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = cx.entity().downgrade();
        let (choice, has_crop) = {
            let session = self.tab().session.read(cx);
            (
                session.crop_ratio_choice.clone(),
                session.crop_rect.is_some(),
            )
        };
        let entries = menu_entries(
            [
                ("Free", "Free"),
                ("Original", "Original"),
                ("1:1", "1:1"),
                ("4:3", "4:3"),
                ("16:9", "16:9"),
            ],
            choice.as_str(),
            |session, value| session.crop_ratio_choice = value.to_string(),
        );
        vec![
            dropdown(&app, "crop-ratio", choice.clone(), px(92.0), entries).into_any_element(),
            Button::new("crop-apply")
                .label("Apply")
                .small()
                .primary()
                .disabled(!has_crop)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tab()
                        .session
                        .update(cx, |session, _| session.commit_crop());
                    this.touch(cx);
                }))
                .into_any_element(),
            Button::new("crop-cancel")
                .label("Cancel")
                .small()
                .disabled(!has_crop)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tab()
                        .session
                        .update(cx, |session, _| session.cancel_crop());
                    this.touch(cx);
                }))
                .into_any_element(),
        ]
    }
}
