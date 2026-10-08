//! Port of the Move tool's option bar (`TransformInspector.swift`): the
//! toggles, the labeled numeric fields, flip, the sampling dropdown and the
//! Cancel/Apply pair.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    App, Context, Entity, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce, SharedString,
    Styled, Window, px,
};

use compositor_core::transform::LayerSampling;

use crate::state::AppState;
use crate::ui::{Dropdown, LabeledCheckbox, MenuEntry, app_dispatch, app_on, div_row, text_label};

impl AppState {
    /// `TransformInspector.swift`'s bar controls: toggles, the numeric
    /// fields, flip, sampling, then Cancel/Apply.
    pub(crate) fn transform_inspector(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let app = cx.entity().downgrade();
        let edit = self.tab().session.read(cx).transform_edit.as_ref();
        let has_edit = edit.is_some();
        let corners = edit.map(|edit| edit.corners.is_some()).unwrap_or(false);
        let mask = edit.map(|edit| edit.mask).unwrap_or_else(|| {
            let session = self.tab().session.read(cx);
            session.is_mask_selected
                && session
                    .active_layer()
                    .and_then(|layer| layer.mask.as_ref().map(|mask| !mask.is_linked))
                    .unwrap_or(false)
        });
        // The fields gray out while nothing can transform, or while a
        // corner distort owns the edit (the handles are the controls).
        let fields_enabled = (self.tab().session.read(cx).can_transform() || has_edit) && !corners;
        // The sampling dropdown: grays out under the same conditions.
        let current = self
            .transform_displayed(cx)
            .map(|value| value.sampling)
            .unwrap_or(LayerSampling::High);
        let sampling_entries: Vec<MenuEntry> = LayerSampling::ALL
            .iter()
            .map(|sampling| {
                let sampling = *sampling;
                MenuEntry::new(sampling.label(), move |state, _, cx| {
                    state.transform_change(move |value| value.sampling = sampling, cx);
                })
                .checked(current == sampling)
            })
            .collect();

        let controls = vec![
            text_label(if mask { "Transform Mask" } else { "Transform" }, false)
                // Section title: ToolHeaderStyle.swift:6 (13 semibold).
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0()
                .into_any_element(),
            LabeledCheckbox::new("transform-auto-select", "Auto-select")
                .checked(self.tab().session.read(cx).transform_auto_select)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    // transformAutoSelect (EditorSession.swift:198). The
                    // canvas press reads it when the next Move press lands.
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.transform_auto_select = checked);
                    state.persist_tool_defaults(cx);
                    state.touch(cx);
                }))
                .into_any_element(),
            LabeledCheckbox::new("transform-show-controls", "Show Controls")
                .checked(self.tab().session.read(cx).shows_transform_controls)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.shows_transform_controls = checked);
                    state.touch(cx);
                }))
                .into_any_element(),
            NumberField::new("transform-x", "X", self.transform_x_input.clone(), px(52.0))
                .into_any_element(),
            NumberField::new("transform-y", "Y", self.transform_y_input.clone(), px(52.0))
                .into_any_element(),
            NumberField::new("transform-w", "W", self.transform_w_input.clone(), px(52.0))
                .into_any_element(),
            NumberField::new("transform-h", "H", self.transform_h_input.clone(), px(52.0))
                .into_any_element(),
            LabeledCheckbox::new("transform-lock-ratio", "Lock")
                .checked(self.tab().session.read(cx).locks_transform_ratio)
                .with_on_change(app_on(&app, |state, checked, _, cx| {
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.locks_transform_ratio = checked);
                    state.touch(cx);
                }))
                .into_any_element(),
            NumberField::new(
                "transform-scale",
                "Scale %",
                self.transform_scale_input.clone(),
                px(56.0),
            )
            .into_any_element(),
            NumberField::new(
                "transform-rotation",
                "\u{b0}",
                self.transform_rotation_input.clone(),
                px(48.0),
            )
            .into_any_element(),
            Dropdown::new("transform-sampling", current.label())
                .with_width(px(120.0))
                .disabled(!fields_enabled)
                .with_entries(sampling_entries)
                .with_dispatch(app_dispatch(&app))
                .into_any_element(),
            Button::new("transform-flip-h")
                .label("Flip H")
                .small()
                .outline()
                .disabled(!fields_enabled)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.transform_flip(true, window, cx);
                }))
                .into_any_element(),
            Button::new("transform-flip-v")
                .label("Flip V")
                .small()
                .outline()
                .disabled(!fields_enabled)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.transform_flip(false, window, cx);
                }))
                .into_any_element(),
            Button::new("transform-cancel")
                .label("Cancel")
                .small()
                .disabled(!has_edit)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.tab()
                        .session
                        .update(cx, |session, _| session.cancel_transform());
                    this.sync_transform_inputs(window, cx);
                    this.touch(cx);
                }))
                .into_any_element(),
            Button::new("transform-apply")
                .label("Apply")
                .small()
                .primary()
                .disabled(!has_edit)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.tab()
                        .session
                        .update(cx, |session, _| session.commit_transform());
                    this.sync_transform_inputs(window, cx);
                    this.touch(cx);
                }))
                .into_any_element(),
        ];
        controls
    }
    /// Flip H / Flip V (`TransformInspector`'s flip buttons): one transform
    /// change each, then the fields follow.
    fn transform_flip(
        &mut self,
        horizontal: bool,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.transform_change(
            move |value| {
                if horizontal {
                    value.flip_x = !value.flip_x;
                } else {
                    value.flip_y = !value.flip_y;
                }
            },
            cx,
        );
        self.sync_transform_inputs(window, cx);
    }
}

/// One of the inspector's labeled numeric fields (TransformValueField's
/// `label + rounded field`). Typing writes through the field's Change
/// subscription; nothing here touches the session.
#[derive(IntoElement)]
pub struct NumberField {
    id: &'static str,
    label: SharedString,
    state: Entity<InputState>,
    width: Pixels,
}

impl NumberField {
    pub fn new(
        id: &'static str,
        label: impl Into<SharedString>,
        state: Entity<InputState>,
        width: Pixels,
    ) -> Self {
        NumberField {
            id,
            label: label.into(),
            state,
            width,
        }
    }
}

impl RenderOnce for NumberField {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div_row()
            .child(text_label(self.label.to_string(), true))
            .child(Input::new(&self.state).id(self.id).small().w(self.width))
    }
}
