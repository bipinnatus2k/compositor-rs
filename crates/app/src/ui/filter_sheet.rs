//! Port of the Filter sheet (`FilterSheet.swift`) as the floating
//! panel: open/OK/Cancel actions and the panel body (Camera Raw's
//! filter panel opens docked per Swift's placement).

use gpui_kit::component::button::Button;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, px};

use compositor_core::adjust::LevelsChannel;
use compositor_core::filters::FilterKind;

use crate::sheets_ui::SheetRow;
use crate::state::AppState;
use crate::ui::floating_panel::FilterPanelState;
use crate::ui::floating_panel::{PanelPlacement, panel_names};
use crate::ui::levels_sheet::levels_channel_label;
use crate::ui::sheets::SliderRole;
use crate::ui::{
    Dropdown, LabeledCheckbox, MenuEntry, app_dispatch, app_on, app_on_drag_move, app_on_drag_up,
    text_label,
};

impl AppState {
    /// `session.filterEdit != nil` → filterPanel (`ContentView.swift:236-246`):
    /// title = the filter's label; Camera Raw opens
    /// `DockedToMainWindowRight`, every other filter `Automatic`.
    pub fn open_filter_panel(&mut self, kind: FilterKind, cx: &mut Context<Self>) {
        self.begin_preview(cx);
        let mut panel = FilterPanelState {
            kind,
            sliders: Vec::new(),
            gaussian: false,
            monochromatic: false,
        };
        let defaults = compositor_core::filters::FilterSettings::default();
        match kind {
            FilterKind::GaussianBlur => {
                panel.sliders = vec![self.make_slider(
                    0.1,
                    250.0,
                    defaults.radius as f32,
                    SliderRole::Radius,
                    cx,
                )];
            }
            FilterKind::MotionBlur => {
                panel.sliders = vec![
                    self.make_slider(-90.0, 90.0, defaults.angle as f32, SliderRole::Angle, cx),
                    self.make_slider(
                        1.0,
                        2000.0,
                        defaults.distance as f32,
                        SliderRole::Distance,
                        cx,
                    ),
                ];
            }
            FilterKind::AddNoise => {
                panel.sliders = vec![self.make_slider(
                    0.1,
                    400.0,
                    defaults.amount as f32,
                    SliderRole::Amount,
                    cx,
                )];
            }
            FilterKind::LensCorrection => {
                panel.sliders =
                    vec![self.make_slider(-100.0, 100.0, 0.0, SliderRole::Distortion, cx)];
            }
            FilterKind::CameraRaw => {
                let group: [(f64, f64, f64); 14] = [
                    (-100.0, 100.0, 0.0), // Temperature
                    (-100.0, 100.0, 0.0), // Tint
                    (-5.0, 5.0, 0.0),     // Exposure
                    (-100.0, 100.0, 0.0), // Contrast
                    (-100.0, 100.0, 0.0), // Highlights
                    (-100.0, 100.0, 0.0), // Shadows
                    (-100.0, 100.0, 0.0), // Whites
                    (-100.0, 100.0, 0.0), // Blacks
                    (-100.0, 100.0, 0.0), // Vibrance
                    (-100.0, 100.0, 0.0), // Saturation
                    (-100.0, 100.0, 0.0), // Texture
                    (-100.0, 100.0, 0.0), // Clarity
                    (-100.0, 100.0, 0.0), // Dehaze
                    (-100.0, 100.0, 0.0), // Vignette Amount
                ];
                panel.sliders = group
                    .iter()
                    .enumerate()
                    .map(|(i, (min, max, value))| {
                        self.make_slider(
                            *min as f32,
                            *max as f32,
                            *value as f32,
                            SliderRole::CameraRaw(i),
                            cx,
                        )
                    })
                    .collect();
            }
            // Curves edits through `curves_editor`; the automatic filters
            // (Black & White, Color Balance, Exposure, Gradient Map, Grain)
            // run with their default settings and show no sliders.
            _ => {}
        }
        // A fresh filter replaces any previous editor state (the sheet dropped
        // `curves_editor` in `close_sheet`).
        self.curves_editor = if kind == FilterKind::Curves {
            Some(crate::state::CurvesEditor::new())
        } else {
            None
        };
        self.panel_widgets.filter = Some(panel);
        let placement = if kind == FilterKind::CameraRaw {
            PanelPlacement::DockedToMainWindowRight
        } else {
            PanelPlacement::Automatic
        };
        self.panels
            .show(panel_names::FILTER_PANEL, kind.label(), placement);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `cancelFilter` (`FilterSheet.swift`): an adjustment editing session
    /// restores the opening settings; the preview is dropped.
    pub fn cancel_filter_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::FILTER_PANEL) {
            return;
        }
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(false));
        }
        self.panel_widgets.filter = None;
        self.curves_editor = None;
        self.after_panel_editing_closed(cx);
    }

    /// `commitFilter` (`FilterSheet.swift`): the filter's previewed pixels
    /// become one undo step named after the filter.
    pub fn confirm_filter_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::FILTER_PANEL) {
            return;
        }
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(true));
        } else {
            let name = self
                .panel_widgets
                .filter
                .as_ref()
                .map(|panel| panel.kind.label())
                .unwrap_or("Filter")
                .to_string();
            self.commit_preview_pixels(&name, cx);
        }
        self.panel_widgets.filter = None;
        self.curves_editor = None;
        self.panels.close(panel_names::FILTER_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `FilterSheet`'s body: the open filter's controls.
    pub(crate) fn filter_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity().downgrade();
        let Some(panel) = self.panel_widgets.filter.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let kind = panel.kind;
        let sliders = panel.sliders.clone();
        let mut body = gpui_kit::div().flex().flex_col().gap_1();
        match kind {
            FilterKind::Curves => {
                // The curve editor (CurvesControls.swift): channel picker, the
                // curve canvas with its click/drag gesture, a selected-point
                // readout with Remove, and Reset. The preview runs through the
                // same filter pipeline as the slider sheets.
                if self.curves_editor.is_none() {
                    self.curves_editor = Some(crate::state::CurvesEditor::new());
                }
                let (curves, selected) = {
                    let editor = self.curves_editor.as_ref().unwrap();
                    (editor.settings.clone(), editor.selected)
                };
                let channel = curves.channel;
                let points = curves.channels[channel.index()].clone();
                let readout = selected
                    .and_then(|index| points.get(index))
                    .map(|point| {
                        format!("Input {} \u{b7} Output {}", point.x as i64, point.y as i64)
                    })
                    .unwrap_or_default();
                let can_remove =
                    selected.is_some_and(|index| index > 0 && index + 1 < points.len());
                body = body
                    .child(
                        SheetRow::new("Channel").with_control(
                            Dropdown::new("curves-channel", levels_channel_label(channel))
                                .with_width(px(90.0))
                                .with_entries(
                                    LevelsChannel::ALL
                                        .iter()
                                        .map(|choice| {
                                            let choice = *choice;
                                            MenuEntry::new(
                                                levels_channel_label(choice),
                                                move |state, _, cx| {
                                                    state.curves_set_channel(choice, cx);
                                                },
                                            )
                                            .checked(channel == choice)
                                        })
                                        .collect(),
                                )
                                .with_dispatch(app_dispatch(&app)),
                        ),
                    )
                    .child(
                        gpui_kit::div()
                            .id("curves-canvas")
                            .w_full()
                            .h(px(240.0))
                            .rounded_sm()
                            .overflow_hidden()
                            .border_1()
                            .border_color(gpui_kit::rgb(crate::ui::BORDER_INSET))
                            .on_mouse_down(
                                gpui_kit::MouseButton::Left,
                                cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                                    this.curves_mouse_down(event, cx);
                                }),
                            )
                            .child(
                                crate::ui::curves_controls::CurvesCanvas::new(
                                    curves, points, selected,
                                )
                                .with_on_drag_move(app_on_drag_move(
                                    &app,
                                    |state, event, _window, cx| {
                                        state.curves_mouse_move(event, cx);
                                    },
                                ))
                                .with_on_drag_up(app_on_drag_up(
                                    &app,
                                    |state, event, _window, cx| {
                                        state.curves_mouse_up(event, cx);
                                    },
                                )),
                            ),
                    )
                    .child(text_label("Click to add a point. Drag to adjust.", true))
                    .child(
                        gpui_kit::div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(text_label(readout, false))
                            .child(gpui_kit::div().flex_1())
                            .child(
                                Button::new("curves-remove")
                                    .label("Remove point")
                                    .small()
                                    .outline()
                                    .disabled(!can_remove)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.curves_remove_point(cx);
                                    })),
                            )
                            .child(
                                Button::new("curves-reset")
                                    .label("Reset curve")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.curves_reset(cx);
                                    })),
                            ),
                    );
            }
            FilterKind::GaussianBlur => {
                body = body.child(SheetRow::new("Radius").with_optional_control(
                    sliders.first().map(|slider| Slider::new(slider).flex_1()),
                ));
            }
            FilterKind::MotionBlur => {
                body = body
                    .child(SheetRow::new("Angle").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Distance").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            FilterKind::AddNoise => {
                let gaussian = panel.gaussian;
                let monochromatic = panel.monochromatic;
                body = body
                    .child(SheetRow::new("Amount").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(
                        SheetRow::new("Distribution").with_control(
                            LabeledCheckbox::new("noise-gaussian", "Gaussian")
                                .checked(gaussian)
                                .with_on_change(app_on(&app, |state, checked, _, cx| {
                                    if let Some(panel) = state.panel_widgets.filter.as_mut() {
                                        panel.gaussian = checked;
                                    }
                                    state.refresh_filter_preview(cx);
                                })),
                        ),
                    )
                    .child(
                        SheetRow::new("").with_control(
                            LabeledCheckbox::new("noise-mono", "Monochromatic")
                                .checked(monochromatic)
                                .with_on_change(app_on(&app, |state, checked, _, cx| {
                                    if let Some(panel) = state.panel_widgets.filter.as_mut() {
                                        panel.monochromatic = checked;
                                    }
                                    state.refresh_filter_preview(cx);
                                })),
                        ),
                    );
            }
            FilterKind::LensCorrection => {
                body = body.child(SheetRow::new("Distortion").with_optional_control(
                    sliders.first().map(|slider| Slider::new(slider).flex_1()),
                ));
            }
            FilterKind::CameraRaw => {
                body = body
                    .child(text_label("White Balance", false).mb_1())
                    .child(SheetRow::new("Temperature").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Tint").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(text_label("Light", false).mb_1())
                    .child(SheetRow::new("Exposure").with_optional_control(
                        sliders.get(2).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Contrast").with_optional_control(
                        sliders.get(3).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Highlights").with_optional_control(
                        sliders.get(4).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Shadows").with_optional_control(
                        sliders.get(5).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Whites").with_optional_control(
                        sliders.get(6).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Blacks").with_optional_control(
                        sliders.get(7).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(text_label("Color", false).mb_1())
                    .child(SheetRow::new("Vibrance").with_optional_control(
                        sliders.get(8).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Saturation").with_optional_control(
                        sliders.get(9).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(text_label("Effects", false).mb_1())
                    .child(SheetRow::new("Texture").with_optional_control(
                        sliders.get(10).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Clarity").with_optional_control(
                        sliders.get(11).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Dehaze").with_optional_control(
                        sliders.get(12).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Vignette").with_optional_control(
                        sliders.get(13).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            _ => {
                body = body.child(text_label(
                    "This filter runs with its default settings.",
                    true,
                ));
            }
        }
        body.into_any_element()
    }
}
