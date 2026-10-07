//! Port of the Levels sheet (`LevelsSheet.swift`). The sheet body lands
//! here as it is carved out of sheets_ui.rs; today this holds the shared
//! Levels UI helpers.

use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Styled, px};

use compositor_core::adjust::LevelsChannel;
use compositor_core::adjust::levels_auto::LevelsAuto;

use crate::sheets_ui::{histogram_view, labeled_slider, sheet_row};
use crate::state::AppState;
use crate::ui::floating_panel::{LevelsPanelState, PanelPlacement, panel_names};
use crate::ui::sheets::SliderRole;
use crate::ui::{MenuEntry, dropdown, labeled_checkbox};

/// The Levels/Curves channel picker's labels (LevelsSheet.swift's Picker).
pub fn levels_channel_label(channel: LevelsChannel) -> &'static str {
    match channel {
        LevelsChannel::Rgb => "RGB",
        LevelsChannel::Red => "Red",
        LevelsChannel::Green => "Green",
        LevelsChannel::Blue => "Blue",
    }
}

impl AppState {
    /// `session.levels != nil` → levelsPanel (`ContentView.swift:198-206`),
    /// title "Levels", `Automatic` placement. Opening captures the active
    /// layer for the live preview and computes the histogram, exactly as the
    /// sheet's `build_sheet` did.
    pub fn open_levels_panel(&mut self, cx: &mut Context<Self>) {
        self.begin_preview();
        let mut panel = LevelsPanelState {
            sliders: vec![
                self.make_slider(0.0, 254.0, 0.0, SliderRole::LevelsInputBlack, cx),
                self.make_slider(0.1, 9.99, 1.0, SliderRole::LevelsGamma, cx),
                self.make_slider(1.0, 255.0, 255.0, SliderRole::LevelsWhite, cx),
                self.make_slider(0.0, 255.0, 0.0, SliderRole::LevelsOutBlack, cx),
                self.make_slider(0.0, 255.0, 255.0, SliderRole::LevelsOutWhite, cx),
            ],
            channel: 0,
            // Swift's LevelsEdit.preview (LevelsSheet's Preview toggle): on when opened.
            preview: true,
            histogram: [0.0; 1024],
            levels: compositor_core::adjust::LevelsSettings::default(),
        };
        if let Some(pixels) = self.preview_base.clone() {
            // With a selection, bins are weighted by the selection's coverage
            // on the layer's pixel grid (LevelsFilter.histogram,
            // Levels.swift:90-96).
            let coverage = crate::ui::sheets::histogram_coverage(
                self.preview_base_transform,
                self.tab().session.selection().cloned(),
                pixels.width,
                pixels.height,
            );
            compositor_core::adjust::levels_histogram(
                pixels.as_ref(),
                coverage.as_ref(),
                &mut panel.histogram,
            );
        }
        self.panel_widgets.levels = Some(panel);
        self.panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `cancelLevels` (`LevelsSheet.swift:79`): an adjustment editing session
    /// restores the opening settings; a pixel-layer session just drops the
    /// preview.
    pub fn cancel_levels_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::LEVELS_PANEL) {
            return;
        }
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(false);
        }
        self.panel_widgets.levels = None;
        self.after_panel_editing_closed(cx);
    }

    /// `commitLevels` (`LevelsSheet.swift:82`): the settings previewed live
    /// are kept as one undo step.
    pub fn confirm_levels_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::LEVELS_PANEL) {
            return;
        }
        // Editing an adjustment layer: the previewed settings are the commit.
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.finish_adjustment_editing(true);
        } else {
            self.commit_preview_pixels("Levels", cx);
        }
        self.panel_widgets.levels = None;
        self.panels.close(panel_names::LEVELS_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `LevelsSheet`'s body: Preview toggle, channel picker, histogram, five
    /// sliders, and the Auto trio (`LevelsAutomatic.swift`'s modes).
    pub(crate) fn levels_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(panel) = self.panel_widgets.levels.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let channel = panel.channel;
        let histogram = panel.histogram;
        let sliders = panel.sliders.clone();
        let preview = panel.preview;

        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(sheet_row(
                "Preview",
                labeled_checkbox(
                    &app,
                    "levels-preview",
                    "Preview",
                    preview,
                    |state, checked, _, cx| {
                        if let Some(panel) = state.panel_widgets.levels.as_mut() {
                            panel.preview = checked;
                        }
                        state.refresh_levels_preview(cx);
                    },
                )
                .into_any_element(),
            ))
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
                        MenuEntry::new("RGB", |state, _, cx| set_levels_channel(state, 0, cx)),
                        MenuEntry::new("Red", |state, _, cx| set_levels_channel(state, 1, cx)),
                        MenuEntry::new("Green", |state, _, cx| set_levels_channel(state, 2, cx)),
                        MenuEntry::new("Blue", |state, _, cx| set_levels_channel(state, 3, cx)),
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
            .child(labeled_slider(&sliders, 0, "Input Black"))
            .child(labeled_slider(&sliders, 1, "Gamma"))
            .child(labeled_slider(&sliders, 2, "Input White"))
            .child(labeled_slider(&sliders, 3, "Output Black"))
            .child(labeled_slider(&sliders, 4, "Output White"))
            .child(sheet_row(
                "Auto",
                gpui_kit::div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(auto_button(
                        "levels-auto-contrast",
                        "Contrast",
                        LevelsAuto::Contrast,
                        cx,
                    ))
                    .child(auto_button(
                        "levels-auto-color",
                        "Color",
                        LevelsAuto::Color,
                        cx,
                    ))
                    .child(auto_button(
                        "levels-auto-neutral",
                        "Neutral",
                        LevelsAuto::Neutral,
                        cx,
                    )),
            ))
            .into_any_element()
    }
}

/// The Levels channel dropdown's write-back: only the visible channel
/// changes, exactly as the sheet's `set_channel` did.
fn set_levels_channel(state: &mut AppState, channel: usize, cx: &mut Context<AppState>) {
    if let Some(panel) = state.panel_widgets.levels.as_mut() {
        panel.channel = channel;
    }
    state.notify_only(cx);
}

/// One Auto button (`LevelsSheet.swift:64`): the mode's settings replace the
/// edit's wholesale, then the sliders and the preview follow.
fn auto_button(
    id: &'static str,
    label: &'static str,
    mode: LevelsAuto,
    cx: &mut Context<AppState>,
) -> AnyElement {
    Button::new(id)
        .label(label)
        .small()
        .outline()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.levels_auto_mode(mode, window, cx);
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::levels_channel_label;
    use compositor_core::adjust::LevelsChannel;

    #[test]
    fn channel_labels_match_the_swift_picker() {
        assert_eq!(levels_channel_label(LevelsChannel::Rgb), "RGB");
        assert_eq!(levels_channel_label(LevelsChannel::Red), "Red");
        assert_eq!(levels_channel_label(LevelsChannel::Green), "Green");
        assert_eq!(levels_channel_label(LevelsChannel::Blue), "Blue");
    }
}
