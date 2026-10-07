//! Port of the selection-amount panel's surface
//! (`LassoControls.swift`, whose slider companion the amount field
//! cites): open/OK/Cancel actions, the panel body and the amount
//! parsing/slider mapping shared by feather/expand/contract.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::{AnyElement, AppContext, Context, IntoElement, ParentElement, Styled, Window, px};

use compositor_core::session::SelectionAmountOperation;

use crate::sheets_ui::{parse_input_usize, sheet_row};
use crate::state::AppState;
use crate::ui::floating_panel::AmountPanelState;
use crate::ui::floating_panel::{PanelPlacement, panel_names};
use crate::ui::text_label;

impl AppState {
    /// `session.selectionAmountOperation` → selectionAmountPanel, title
    /// "{Operation} Selection" (`ContentView.swift:227-234`). The operation
    /// rides on the core session, which is what blocks project operations
    /// while the panel is open.
    pub fn open_amount_panel(
        &mut self,
        operation: SelectionAmountOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab().session.selection_amount_operation.is_some() {
            return;
        }
        self.tab().session.selection_amount_operation = Some(operation);
        let session = &self.tab().session;
        let amount = match operation {
            SelectionAmountOperation::Expand => session.selection_expand_amount,
            SelectionAmountOperation::Contract => session.selection_contract_amount,
            SelectionAmountOperation::Feather => session.selection_feather_amount,
        };
        let tolerance = cx.new(|cx| InputState::new(window, cx).default_value(amount.to_string()));
        let maximum = amount_maximum(operation);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(maximum as f32)
                .step(1.0)
                .default_value(amount as f32)
        });
        // Slider → field: the slider writes the input's text
        // (`SelectionAmountSheet`: `set: { input = String(Int($0.rounded())) }`).
        let watched = tolerance.clone();
        let slider_subscription = cx.subscribe_in(
            &slider,
            window,
            move |_this, _slider, event: &SliderEvent, window, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => value.end(),
                };
                let text = amount_text_for_slider(value, maximum);
                watched.update(cx, |state, cx| {
                    if state.value() != text {
                        state.set_value(text, window, cx);
                    }
                });
            },
        );
        self.subscriptions.push(slider_subscription);
        // Field → slider: the thumb re-derives from the parsed amount and
        // jumps to 1 while the text is invalid (`get: { Double(amount ?? 1) }`).
        // `InputState::set_value` and `SliderState::set_value` both suppress
        // their events, so the two-way sync cannot ping-pong.
        let watched = slider.clone();
        let input_subscription = cx.subscribe_in(
            &tolerance,
            window,
            move |_this, input, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    // Read through the same entity the event came from.
                    let text = input.read(cx).value().to_string();
                    let value = amount_slider_value(&text, maximum);
                    watched.update(cx, |state, cx| {
                        state.set_value(SliderValue::Single(value), window, cx);
                    });
                }
            },
        );
        self.subscriptions.push(input_subscription);
        self.panel_widgets.amount = Some(AmountPanelState {
            operation,
            tolerance,
            slider,
        });
        let title = match operation {
            SelectionAmountOperation::Feather => "Feather Selection",
            SelectionAmountOperation::Expand => "Expand Selection",
            SelectionAmountOperation::Contract => "Contract Selection",
        };
        self.panels.show(
            panel_names::SELECTION_AMOUNT_PANEL,
            title,
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `onClose = { session.selectionAmountOperation = nil }`
    /// (`ContentView.swift:230`): nothing was committed.
    pub fn cancel_amount_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::SELECTION_AMOUNT_PANEL) {
            return;
        }
        self.tab().session.selection_amount_operation = None;
        self.panel_widgets.amount = None;
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// OK (`SelectionAmountSheet.swift:224`): runs the operation the session
    /// is holding. The footer keeps OK disabled while the amount is outside
    /// the operation's range, so this always runs with a valid amount.
    pub fn confirm_amount_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::SELECTION_AMOUNT_PANEL) {
            return;
        }
        let amount = {
            let cx_app: &gpui_kit::App = cx;
            self.panel_widgets
                .amount
                .as_ref()
                .map(|panel| parse_input_usize(&panel.tolerance, cx_app).unwrap_or(0))
                .unwrap_or(0) as i64
        };
        self.tab().session.confirm_selection_amount(amount);
        self.panel_widgets.amount = None;
        self.panels.close(panel_names::SELECTION_AMOUNT_PANEL);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// The amount field's validity, driving the footer's disabled OK:
    /// a whole number within the operation's range
    /// (`SelectionAmountSheet.amount`, maximum 250 for Feather, else 500).
    pub fn amount_input_valid(&self, cx: &gpui_kit::App) -> bool {
        let Some(panel) = self.panel_widgets.amount.as_ref() else {
            return false;
        };
        parse_amount(
            &panel.tolerance.read(cx).value(),
            amount_maximum(panel.operation),
        )
        .is_some()
    }

    /// `SelectionAmountSheet`'s body: the Amount field with its 1…maximum
    /// companion slider (`LassoControls.swift:220-231`), plus the invalid
    /// hint line. Field and slider stay in two-way sync.
    pub(crate) fn amount_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = self.panel_widgets.amount.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let tolerance = panel.tolerance.clone();
        let slider = panel.slider.clone();
        let maximum = amount_maximum(panel.operation);
        let cx_app: &gpui_kit::App = cx;
        let valid = parse_amount(&tolerance.read(cx_app).value(), maximum).is_some();
        let mut body = gpui_kit::div().flex().flex_col().gap_1().child(sheet_row(
            "Amount",
            gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_1()
                .child(
                    gpui_kit::div()
                        .flex_1()
                        .min_w_0()
                        .child(Slider::new(&slider)),
                )
                .child(Input::new(&tolerance).id("amount-value").w(px(64.0))),
        ));
        if !valid {
            body = body.child(text_label(
                format!("Enter a whole number from 1 to {maximum} px."),
                true,
            ));
        }
        body.into_any_element()
    }
}

/// The selection-amount slider's upper bound (`SelectionAmountSheet.maximum`:
/// 250 for Feather, 500 otherwise).
pub(crate) fn amount_maximum(operation: SelectionAmountOperation) -> i64 {
    if operation == SelectionAmountOperation::Feather {
        250
    } else {
        500
    }
}

/// Parses the amount field: a whole number in 1…maximum
/// (`SelectionAmountSheet.amount`).
pub(crate) fn parse_amount(text: &str, maximum: i64) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .parse::<i64>()
        .ok()
        .filter(|value| (1..=maximum).contains(value))
}

/// The slider thumb position for the amount text — the slider binding's
/// getter, `Double(amount ?? 1)` (`LassoControls.swift:225`): an invalid
/// field parks the thumb at 1.
pub(crate) fn amount_slider_value(text: &str, maximum: i64) -> f32 {
    parse_amount(text, maximum).unwrap_or(1) as f32
}

/// The amount text for a slider position — the binding's setter,
/// `String(Int($0.rounded()))`, clamped to the operation's range.
pub(crate) fn amount_text_for_slider(value: f32, maximum: i64) -> String {
    format!("{}", (value.round() as i64).clamp(1, maximum))
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    /// The selection-amount bounds (`SelectionAmountSheet.maximum`).
    #[test]
    fn amount_maximum_follows_the_operation() {
        assert_eq!(amount_maximum(SelectionAmountOperation::Feather), 250);
        assert_eq!(amount_maximum(SelectionAmountOperation::Expand), 500);
        assert_eq!(amount_maximum(SelectionAmountOperation::Contract), 500);
    }

    /// The amount field's parse rule: a whole number in 1…maximum
    /// (`SelectionAmountSheet.amount`).
    #[test]
    fn parse_amount_bounds_and_whole_numbers() {
        assert_eq!(parse_amount("10", 500), Some(10));
        assert_eq!(parse_amount(" 42 ", 500), Some(42));
        assert_eq!(parse_amount("1", 500), Some(1));
        assert_eq!(parse_amount("500", 500), Some(500));
        assert_eq!(parse_amount("250", 250), Some(250));
        // Out of range, zero, negative, fractional, junk.
        assert_eq!(parse_amount("0", 500), None);
        assert_eq!(parse_amount("501", 500), None);
        assert_eq!(parse_amount("-3", 500), None);
        assert_eq!(parse_amount("2.5", 500), None);
        assert_eq!(parse_amount("px", 500), None);
        assert_eq!(parse_amount("", 500), None);
    }

    /// Field ↔ slider sync (`LassoControls.swift:224-228`): the thumb
    /// re-derives from the parsed text (parking at 1 while invalid) and the
    /// text re-derives from the rounded thumb (clamped into range), so a
    /// round trip through both is the identity for valid values.
    #[test]
    fn amount_field_and_slider_stay_in_sync() {
        for maximum in [250, 500] {
            for value in [1, 2, 37, maximum - 1, maximum] {
                let text = value.to_string();
                let thumb = amount_slider_value(&text, maximum);
                assert_eq!(amount_text_for_slider(thumb, maximum), text);
            }
        }
        // Invalid text parks the thumb at 1 (Swift's `amount ?? 1`).
        assert_eq!(amount_slider_value("nonsense", 500), 1.0);
        assert_eq!(amount_slider_value("0", 500), 1.0);
        assert_eq!(amount_slider_value("9999", 500), 1.0);
        // The thumb clamps: dragging past the maximum writes the maximum.
        assert_eq!(amount_text_for_slider(750.0, 500), "500");
        assert_eq!(amount_text_for_slider(0.0, 500), "1");
        // Fractional thumb positions round (`Int($0.rounded())`).
        assert_eq!(amount_text_for_slider(10.4, 500), "10");
        assert_eq!(amount_text_for_slider(10.6, 500), "11");
    }
}
