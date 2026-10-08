//! Port of the Hue/Saturation sheet (`HueSaturationSheet.swift`) as
//! the floating panel: open/OK/Cancel actions and the panel body.

use gpui_kit::component::slider::Slider;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Styled};

use crate::sheets_ui::SheetRow;
use crate::state::AppState;
use crate::ui::floating_panel::HueSaturationPanelState;
use crate::ui::floating_panel::{PanelPlacement, panel_names};
use crate::ui::sheets::SliderRole;
use crate::ui::{LabeledCheckbox, app_on};

impl AppState {
    /// `session.hueSaturation != nil` → adjustmentPanel, title
    /// "Hue/Saturation" (`ContentView.swift:208-215`).
    pub fn open_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        self.begin_preview(cx);
        let panel = HueSaturationPanelState {
            sliders: vec![
                self.make_slider(-180.0, 180.0, 0.0, SliderRole::Hue, cx),
                self.make_slider(-100.0, 100.0, 0.0, SliderRole::Saturation, cx),
                self.make_slider(-100.0, 100.0, 0.0, SliderRole::Lightness, cx),
            ],
            colorize: false,
        };
        self.panel_widgets.hue_saturation = Some(panel);
        self.panels.show(
            panel_names::ADJUSTMENT_PANEL,
            "Hue/Saturation",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `cancelHueSaturation` (`ContentView.swift:210`).
    pub fn cancel_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::ADJUSTMENT_PANEL) {
            return;
        }
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(false));
        }
        self.panel_widgets.hue_saturation = None;
        self.after_panel_editing_closed(cx);
    }

    pub fn confirm_hue_saturation_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::ADJUSTMENT_PANEL) {
            return;
        }
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(true));
        } else {
            self.commit_preview_pixels("Hue/Saturation", cx);
        }
        self.panel_widgets.hue_saturation = None;
        self.panels.close(panel_names::ADJUSTMENT_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `HueSaturationSheet`'s body: three sliders plus the Colorize toggle.
    pub(crate) fn hue_saturation_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(panel) = self.panel_widgets.hue_saturation.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let colorize = panel.colorize;
        let sliders = panel.sliders.clone();
        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                SheetRow::new("Hue").with_optional_control(
                    sliders.first().map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Saturation").with_optional_control(
                    sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Lightness").with_optional_control(
                    sliders.get(2).map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Colorize").with_control(
                    LabeledCheckbox::new("hue-colorize", "Colorize")
                        .checked(colorize)
                        .with_on_change(app_on(&app, |state, checked, _, cx| {
                            if let Some(panel) = state.panel_widgets.hue_saturation.as_mut() {
                                panel.colorize = checked;
                            }
                            state.apply_slider(SliderRole::Hue, 0.0, cx);
                        })),
                ),
            )
            .into_any_element()
    }
}
