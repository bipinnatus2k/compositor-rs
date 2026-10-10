//! Port of the selection tools' surfaces (`LassoControls.swift`): the
//! amount panel as its own view entity (ADR 0003 step D, ticket 41) —
//! `SelectionAmountSheet`'s `@State input` and its 1…maximum companion
//! slider — plus the amount parsing/slider mapping shared by
//! feather/expand/contract, and the option-bar controls the
//! Marquee/Lasso/Wand tools share.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription,
    WeakEntity, Window, px,
};

use compositor_core::selection::{LassoKind, WandMode};
use compositor_core::session::{EditorSession, SelectionAmountOperation};

use crate::sheets_ui::{SheetRow, parse_input_usize};
use crate::state::{AppState, slider};
use crate::ui::floating_panel::{PanelPlacement, panel_body_frame, panel_footer, panel_names};
use crate::ui::navigation_tool_header::menu_entries;
use crate::ui::{Dropdown, LabeledCheckbox, LabeledSlider, app_dispatch, app_on, text_label};

/// The selection-amount panel as its own view entity (Swift
/// `SelectionAmountSheet`'s edit state; ticket 41): the amount field with
/// its companion slider, kept in two-way sync by the panel's own
/// subscriptions. The session never appears here — the open/confirm
/// session mutations stay on the shell, so the panel holds only the seam
/// its footer routes through. Created by `AppState::open_amount_panel`,
/// dropped on close.
pub struct AmountPanelView {
    /// The window shell (the transitional root): the footer's commit/cancel
    /// flows stay there.
    app: WeakEntity<AppState>,
    /// Which operation is being sized (`SelectionAmountSheet.operation`).
    operation: SelectionAmountOperation,
    /// The amount field (`LassoControls.swift:184`'s `@State input`).
    tolerance: Entity<InputState>,
    /// The amount field's 1…maximum companion slider
    /// (`LassoControls.swift:224-228`).
    slider: Entity<SliderState>,
    /// Keep-alive for the field subscriptions: dropping a Subscription
    /// unsubscribes, so the bundle is held, never read.
    #[allow(dead_code)]
    subscriptions: Vec<Subscription>,
}

impl AmountPanelView {
    pub(crate) fn new(
        operation: SelectionAmountOperation,
        amount: i64,
        app: WeakEntity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let maximum = amount_maximum(operation);
        let tolerance = cx.new(|cx| InputState::new(window, cx).default_value(amount.to_string()));
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
        AmountPanelView {
            app,
            operation,
            tolerance,
            slider,
            subscriptions: vec![slider_subscription, input_subscription],
        }
    }

    /// The amount field's parse, for the shell's OK pipeline (the one panel
    /// read from outside this view — the commit needs the typed amount).
    pub(crate) fn amount_value(&self, cx: &gpui_kit::App) -> usize {
        parse_input_usize(&self.tolerance, cx).unwrap_or(0)
    }

    /// The amount field's validity, driving the footer's disabled OK:
    /// a whole number within the operation's range
    /// (`SelectionAmountSheet.amount`, maximum 250 for Feather, else 500).
    fn input_valid(&self, cx: &gpui_kit::App) -> bool {
        parse_amount(
            &self.tolerance.read(cx).value(),
            amount_maximum(self.operation),
        )
        .is_some()
    }

    /// The panel's commit/cancel routes: the footer's OK/Cancel are
    /// panel-entity callbacks that reach the shell's pipelines through the
    /// seam (ticket 41's ruling 4).
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.cancel_amount_panel(cx))
            .ok();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.confirm_amount_panel(cx))
            .ok();
    }
}

impl Render for AmountPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let maximum = amount_maximum(self.operation);
        let cx_app: &gpui_kit::App = cx;
        let valid = self.input_valid(cx_app);
        let body = gpui_kit::div().flex().flex_col().gap_1().child(
            SheetRow::new("Amount").with_control(
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
                            .child(Slider::new(&self.slider)),
                    )
                    .child(Input::new(&self.tolerance).id("amount-value").w(px(64.0))),
            ),
        );
        let body = if valid {
            body
        } else {
            body.child(text_label(
                format!("Enter a whole number from 1 to {maximum} px."),
                true,
            ))
        };
        let footer = panel_footer(
            panel_names::SELECTION_AMOUNT_PANEL,
            &this,
            !valid,
            |panel, _, cx| panel.cancel(cx),
            |panel, _, cx| panel.confirm(cx),
        );
        panel_body_frame(body.into_any_element(), footer)
    }
}

/// The selection tools' option bar (`LassoControls.swift`'s view): a view
/// entity since ticket 42 — it owns the wand tolerance slider entity and
/// its write subscription (`subscribe_sliders`' wand slice) and renders the
/// Marquee, Lasso and Wand arms. Only in the render tree while a selection
/// tool is active (Swift's `if session.tool.isSelectionTool`), so it keeps
/// no session observer; its session field is re-pointed on tab changes by
/// [`AppState::sync_session_observer`].
pub struct LassoControlsView {
    /// The active tab's session — the arm readouts and the tolerance slider
    /// write speak to it directly (`LassoControls`' `@Bindable session`).
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the repaint epoch and the
    /// shared header row stay there. No AppState dependencies beyond what
    /// the moved handlers already used.
    app: WeakEntity<AppState>,
    /// The wand tolerance slider (`LassoControls`' tolerance binding):
    /// seeded from the persisted tool default at construction, exactly as
    /// `AppState::new` seeded it when the field lived there.
    wand_slider: Entity<SliderState>,
    /// Keep-alive for the slider subscription: dropping a Subscription
    /// unsubscribes, so the bundle is held, never read.
    #[allow(dead_code)]
    subscriptions: Vec<Subscription>,
}

impl LassoControlsView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let tolerance = session.read(cx).wand_settings.tolerance;
        let wand_slider = cx.new(|_| slider(0.0, 255.0, 1.0, tolerance as f32));
        let mut view = LassoControlsView {
            session,
            app,
            wand_slider,
            subscriptions: Vec::new(),
        };
        view.subscribe_sliders(cx);
        view
    }

    /// Re-points the session on tab changes (`AppState::sync_session_observer`,
    /// same as the canvas view and the layers panel). The slider
    /// subscription reads this field at event time, so the re-point keeps
    /// it writing to the active tab.
    pub(crate) fn set_session(&mut self, session: Entity<EditorSession>) {
        self.session = session;
    }

    /// Keeps the session's wand tolerance in sync with the slider
    /// (`subscribe_sliders`' wand slice, moved with the slider): a Change
    /// or Release writes the value and bumps the repaint epoch.
    fn subscribe_sliders(&mut self, cx: &mut Context<Self>) {
        let wand_slider = self.wand_slider.clone();
        let subscription = cx.subscribe(&wand_slider, |this, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            this.session.update(cx, |session, _| {
                session.wand_settings.tolerance = value as i32
            });
            this.touch(cx);
        });
        self.subscriptions.push(subscription);
    }

    /// The view's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on AppState (ticket 39's field audit), so
    /// an interaction that changed the session asks the shell through the
    /// seam.
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// `LassoControls.swift`'s Marquee controls: the Rectangle/Ellipse
    /// shape picker.
    pub(crate) fn marquee_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let kind = self.session.read(cx).marquee_kind;
        vec![
            Dropdown::new(
                "marquee-kind",
                if kind == LassoKind::Ellipse {
                    "Ellipse"
                } else {
                    "Rectangle"
                },
            )
            .with_width(px(100.0))
            .with_entries(menu_entries(
                [
                    (LassoKind::Rectangle, "Rectangle"),
                    (LassoKind::Ellipse, "Ellipse"),
                ],
                kind,
                |session, value| session.marquee_kind = value,
            ))
            .with_dispatch(app_dispatch(&app))
            .into_any_element(),
        ]
    }

    /// `LassoControls.swift`'s Lasso controls: the Freehand/Polygonal
    /// kind picker.
    pub(crate) fn lasso_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let kind = self.session.read(cx).lasso_kind;
        vec![
            Dropdown::new(
                "lasso-kind",
                if kind == LassoKind::Polygonal {
                    "Polygonal"
                } else {
                    "Freehand"
                },
            )
            .with_width(px(100.0))
            .with_entries(menu_entries(
                [
                    (LassoKind::Freehand, "Freehand"),
                    (LassoKind::Polygonal, "Polygonal"),
                ],
                kind,
                |session, value| session.lasso_kind = value,
            ))
            .with_dispatch(app_dispatch(&app))
            .into_any_element(),
        ]
    }

    /// `LassoControls.swift`'s Wand controls: the Wand/Object mode picker,
    /// the tolerance slider and the sample checkboxes.
    pub(crate) fn wand_controls(&mut self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let app = self.app.clone();
        let (mode, settings) = {
            let session = self.session.read(cx);
            (session.wand_mode, session.wand_settings)
        };
        vec![
            Dropdown::new(
                "wand-mode",
                match mode {
                    WandMode::Wand => "Wand",
                    WandMode::Object => "Object",
                },
            )
            .with_width(px(84.0))
            .with_entries(menu_entries(
                [(WandMode::Wand, "Wand"), (WandMode::Object, "Object")],
                mode,
                |session, value| session.wand_mode = value,
            ))
            .with_dispatch(app_dispatch(&app))
            .into_any_element(),
            LabeledSlider::new(
                "Tolerance",
                self.wand_slider.clone(),
                format!("{}", settings.tolerance),
                px(90.0),
            )
            .into_any_element(),
            LabeledCheckbox::new("wand-contiguous", "Contiguous")
                .checked(settings.contiguous)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.wand_settings.contiguous = checked);
                    state.touch(cx);
                }))
                .into_any_element(),
            LabeledCheckbox::new("wand-all-layers", "Sample All Layers")
                .checked(settings.sample_all_layers)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state.tab().session.update(cx, |session, _| {
                        session.wand_settings.sample_all_layers = checked
                    });
                    state.touch(cx);
                }))
                .into_any_element(),
        ]
    }
}

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
        if self
            .tab()
            .session
            .read(cx)
            .selection_amount_operation
            .is_some()
        {
            return;
        }
        let amount = self.tab().session.update(cx, |session, _| {
            session.selection_amount_operation = Some(operation);
            match operation {
                SelectionAmountOperation::Expand => session.selection_expand_amount,
                SelectionAmountOperation::Contract => session.selection_contract_amount,
                SelectionAmountOperation::Feather => session.selection_feather_amount,
            }
        });
        let app = cx.entity().downgrade();
        let panel = cx.new(|cx| AmountPanelView::new(operation, amount, app, window, cx));
        self.panel_views.insert(
            panel_names::SELECTION_AMOUNT_PANEL,
            crate::ui::floating_panel::PanelContent::Amount(panel),
        );
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
        self.tab()
            .session
            .update(cx, |session, _| session.selection_amount_operation = None);
        self.panel_views.remove(panel_names::SELECTION_AMOUNT_PANEL);
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
            self.amount_view()
                .map(|panel| panel.read(cx_app).amount_value(cx_app))
                .unwrap_or(0) as i64
        };
        self.tab()
            .session
            .update(cx, |session, _| session.confirm_selection_amount(amount));
        self.panel_views.remove(panel_names::SELECTION_AMOUNT_PANEL);
        self.panels.close(panel_names::SELECTION_AMOUNT_PANEL);
        self.sync_panel_guard();
        self.touch(cx);
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
