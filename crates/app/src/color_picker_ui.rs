//! The color picker panel (`UI/ColorPickerSheet.swift`, first cut): hue,
//! saturation and brightness sliders, a preview swatch, hex entry and
//! OK/Cancel. The Swift panel's drawn saturation/brightness field and hue
//! strip become sliders in this UI's widget vocabulary; the working color is
//! the session's `color_picker.hsb`, so nothing lands until OK.

use gpui_kit::{
    px, rgba, AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Styled, Window,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::Sizable;

use compositor_core::session::ColorPickerTarget;

use crate::state::AppState;
use crate::ui::text_label;

/// The open picker's widgets; `None` while no picker is open.
pub struct PickerUi {
    pub hue: Entity<SliderState>,
    pub saturation: Entity<SliderState>,
    pub brightness: Entity<SliderState>,
    pub hex: Entity<InputState>,
}

impl AppState {
    /// Opens the picker on a palette swatch and builds its widgets
    /// (ColorPalette.swift:60-63).
    pub fn open_color_picker_ui(
        &mut self,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.color_picker.is_some() || !self.session.can_edit_palette() {
            return;
        }
        self.session.open_color_picker(background);
        let Some(picker) = self.session.color_picker.as_ref() else {
            return;
        };
        let hsb = picker.hsb;
        let hue = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(360.0)
                .step(1.0)
                .default_value(hsb.hue as f32)
        });
        let saturation = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value((hsb.saturation * 100.0) as f32)
        });
        let brightness = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value((hsb.brightness * 100.0) as f32)
        });
        let hex = cx.new(|cx| {
            InputState::new(window, cx).default_value(picker.color().hex())
        });
        for (slider, apply) in [
            (hue.clone(), (|hsb: &mut compositor_core::document::PickerHSB, v: f64| hsb.hue = v)
                as fn(&mut compositor_core::document::PickerHSB, f64)),
            (
                saturation.clone(),
                |hsb: &mut compositor_core::document::PickerHSB, v: f64| {
                    hsb.saturation = v / 100.0;
                },
            ),
            (
                brightness.clone(),
                |hsb: &mut compositor_core::document::PickerHSB, v: f64| {
                    hsb.brightness = v / 100.0;
                },
            ),
        ] {
            let subscription = cx.subscribe(
                &slider,
                move |state: &mut Self,
                      _slider,
                      event: &SliderEvent,
                      cx: &mut Context<Self>| {
                    let value = match event {
                        SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
                    };
                    let Some(picker) = state.session.color_picker.as_mut() else {
                        return;
                    };
                    apply(&mut picker.hsb, value as f64);
                    // An effect's color previews live on the layer
                    // (ColorPalette.swift:113-118).
                    if matches!(
                        picker.target,
                        ColorPickerTarget::Effect { .. }
                    ) {
                        state.session.preview_effect_color();
                    }
                    state.touch(cx);
                },
            );
            self.subscriptions.push(subscription);
        }
        self.picker_ui = Some(PickerUi {
            hue,
            saturation,
            brightness,
            hex,
        });
        self.touch(cx);
    }

    /// OK/Cancel (ColorPickerSheet.swift:24-27): commit reads the hex field
    /// first, then the session writes (or drops) the color.
    pub fn close_color_picker_ui(&mut self, commit: bool, cx: &mut Context<Self>) {
        if commit {
            let text = self
                .picker_ui
                .as_ref()
                .map(|ui| ui.hex.read(cx).value().to_string())
                .unwrap_or_default();
            // A well-formed hex entry overrides the sliders (Swift's
            // commitHex on submit).
            if let Some(color) = compositor_core::document::PaletteColor::from_hex(&text) {
                if let Some(picker) = self.session.color_picker.as_mut() {
                    picker.hsb = compositor_core::document::PickerHSB::from_rgb(color);
                }
            }
            self.session.close_color_picker(true);
        } else {
            self.session.close_color_picker(false);
        }
        self.picker_ui = None;
        self.touch(cx);
    }
}

/// The picker overlay, rendered above the workspace like the sheets.
pub fn render_color_picker(
    state: &mut AppState,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let picker = state.session.color_picker.as_ref()?;
    let title = picker.target.title();
    let color = picker.color();
    let Some(ui) = state.picker_ui.as_ref() else {
        return None;
    };
    let hue = ui.hue.clone();
    let saturation = ui.saturation.clone();
    let brightness = ui.brightness.clone();
    let hex = ui.hex.clone();

    let value = ((color.red * 255.0).round() as u32) << 16
        | ((color.green * 255.0).round() as u32) << 8
        | (color.blue * 255.0).round() as u32;
    let row = |label: &str, slider: Entity<SliderState>| {
        gpui_kit::div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .py_0p5()
            .child(
                gpui_kit::div()
                    .w(px(90.0))
                    .flex_shrink_0()
                    .child(text_label(label.to_string(), true)),
            )
            .child(gpui_kit::div().flex_1().child(Slider::new(&slider)))
    };

    let body = gpui_kit::div()
        .flex()
        .flex_row()
        .gap_3()
        .px_3()
        .py_2()
        .child(
            gpui_kit::div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .child(row("Hue", hue))
                .child(row("Saturation", saturation))
                .child(row("Brightness", brightness))
                .child(
                    gpui_kit::div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .py_0p5()
                        .child(
                            gpui_kit::div()
                                .w(px(90.0))
                                .flex_shrink_0()
                                .child(text_label("Hex", true)),
                        )
                        .child(hex),
                ),
        )
        .child(
            gpui_kit::div()
                .w(px(64.0))
                .h(px(64.0))
                .rounded_sm()
                .border_1()
                .border_color(gpui_kit::rgb(0x606060))
                .bg(gpui_kit::rgb(value)),
        );

    let footer = gpui_kit::div()
        .flex()
        .flex_row()
        .justify_end()
        .gap_2()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(gpui_kit::rgb(0x4a4a4a))
        .child(
            Button::new("picker-cancel")
                .label("Cancel")
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_color_picker_ui(false, cx);
                })),
        )
        .child(
            Button::new("picker-ok")
                .label("OK")
                .small()
                .primary()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_color_picker_ui(true, cx);
                })),
        );

    let overlay = gpui_kit::div()
        .absolute()
        .inset_0()
        .bg(rgba(0x00000099))
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .child(
            gpui_kit::div()
                .w(px(420.0))
                .flex()
                .flex_col()
                .rounded_md()
                .border_1()
                .border_color(gpui_kit::rgb(0x4a4a4a))
                .bg(gpui_kit::rgb(0x2d2d2d))
                .overflow_hidden()
                .child(
                    gpui_kit::div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(gpui_kit::rgb(0x4a4a4a))
                        .child(text_label(title, false).text_size(px(13.0))),
                )
                .child(body)
                .child(footer),
        );
    Some(overlay.into_any_element())
}
