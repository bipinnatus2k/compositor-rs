//! Port of the Move tool's option bar (`TransformInspector.swift`): the
//! toggles, the labeled numeric fields, flip, the sampling dropdown and the
//! Cancel/Apply pair.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{Context, Entity, FontWeight, IntoElement, ParentElement, Styled, WeakEntity, px};

use compositor_core::transform::{LayerSampling, LayerTransform};

use crate::state::AppState;
use crate::ui::navigation_tool_header::checkbox;
use crate::ui::{div_row, text_label};

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

        let controls = vec![
            text_label(if mask { "Transform Mask" } else { "Transform" }, false)
                // Section title: ToolHeaderStyle.swift:6 (13 semibold).
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0()
                .into_any_element(),
            checkbox(
                &app,
                "transform-auto-select",
                "Auto-select",
                self.tab().session.read(cx).transform_auto_select,
                |state, checked, _, cx| {
                    // transformAutoSelect (EditorSession.swift:198). The
                    // canvas press reads it when the next Move press lands.
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.transform_auto_select = checked);
                    state.persist_tool_defaults(cx);
                    state.touch(cx);
                },
            ),
            checkbox(
                &app,
                "transform-show-controls",
                "Show Controls",
                self.tab().session.read(cx).shows_transform_controls,
                |state, checked, _, cx| {
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.shows_transform_controls = checked);
                    state.touch(cx);
                },
            ),
            number_field("transform-x", "X", &self.transform_x_input, px(52.0)).into_any_element(),
            number_field("transform-y", "Y", &self.transform_y_input, px(52.0)).into_any_element(),
            number_field("transform-w", "W", &self.transform_w_input, px(52.0)).into_any_element(),
            number_field("transform-h", "H", &self.transform_h_input, px(52.0)).into_any_element(),
            checkbox(
                &app,
                "transform-lock-ratio",
                "Lock",
                self.tab().session.read(cx).locks_transform_ratio,
                |state, checked, _, cx| {
                    state
                        .tab()
                        .session
                        .update(cx, |session, _| session.locks_transform_ratio = checked);
                    state.touch(cx);
                },
            ),
            number_field(
                "transform-scale",
                "Scale %",
                &self.transform_scale_input,
                px(56.0),
            )
            .into_any_element(),
            number_field(
                "transform-rotation",
                "\u{b0}",
                &self.transform_rotation_input,
                px(48.0),
            )
            .into_any_element(),
            sampling_dropdown(&app, self.transform_displayed(cx), fields_enabled),
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
fn number_field(
    id: &'static str,
    label: &str,
    state: &Entity<InputState>,
    width: gpui_kit::Pixels,
) -> gpui_kit::Div {
    div_row()
        .child(text_label(label.to_string(), true))
        .child(Input::new(state).id(id).small().w(width))
}

/// The sampling dropdown: grays out while nothing can transform, or while
/// a corner distort owns the edit.
fn sampling_dropdown(
    app: &WeakEntity<AppState>,
    value: Option<LayerTransform>,
    enabled: bool,
) -> gpui_kit::AnyElement {
    let current = value
        .map(|value| value.sampling)
        .unwrap_or(LayerSampling::High);
    let entries: Vec<crate::ui::MenuEntry> = LayerSampling::ALL
        .iter()
        .map(|sampling| {
            let sampling = *sampling;
            crate::ui::MenuEntry::new(sampling.label(), move |state, _, cx| {
                state.transform_change(move |value| value.sampling = sampling, cx);
            })
            .checked(current == sampling)
        })
        .collect();
    let app = app.clone();
    let button = Button::new("transform-sampling")
        .label(current.label())
        .outline()
        .small()
        // Same caret affordance as the ui::dropdown helper (gpui-component
        // button.rs dropdown_caret).
        .dropdown_caret(true)
        .w(px(120.0))
        .disabled(!enabled);
    button
        .dropdown_menu(move |menu, _, _| crate::ui::build_menu(&app, menu, &entries))
        .into_any_element()
}
