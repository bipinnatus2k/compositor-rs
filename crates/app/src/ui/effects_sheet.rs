//! Port of the Effects sheet (`EffectsSheet.swift`) as the floating
//! panel: open/OK/Cancel actions, the panel body and the stroke
//! segment / effect swatch helpers.

use gpui_kit::component::button::Button;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, px,
};

use compositor_core::effects::LayerEffectKind;

use crate::sheets_ui::SheetRow;
use crate::state::AppState;
use crate::ui::floating_panel::EffectsPanelState;
use crate::ui::floating_panel::{PanelPlacement, panel_names};
use crate::ui::sheets::SliderRole;
use crate::ui::text_label;

impl AppState {
    /// `session.effectsEditing` → effectsPanel, title = the effect's name
    /// (`ContentView.swift:217-224`). The caller (`add_effect`) has already
    /// started the editing session.
    pub fn open_effects_panel(&mut self, cx: &mut Context<Self>) {
        // No preview capture: every slider change is committed to the layer's
        // effects right away (one undo step per change, as Swift's
        // changeEffects does).
        let editing = self.tab().session.read(cx).editing_effects();
        let kind = self
            .tab()
            .session
            .read(cx)
            .effects_editing
            .map(|editing| editing.kind)
            .unwrap_or(LayerEffectKind::Stroke);
        let mut slider = |min: f64, max: f64, value: f64, index: usize| {
            self.make_slider(
                min as f32,
                max as f32,
                value as f32,
                SliderRole::Effect(index),
                cx,
            )
        };
        // Slider thumb ranges are the EffectsSheet's visible ranges; the
        // core `isValid` guard bounds every stored value.
        let sliders = match kind {
            LayerEffectKind::Stroke => vec![
                slider(0.0, 20.0, editing.stroke.map(|s| s.size).unwrap_or(4.0), 0),
                slider(
                    0.0,
                    100.0,
                    editing.stroke.map(|s| s.opacity * 100.0).unwrap_or(100.0),
                    1,
                ),
            ],
            LayerEffectKind::Shadow => vec![
                slider(
                    0.0,
                    100.0,
                    editing.shadow.map(|s| s.opacity * 100.0).unwrap_or(50.0),
                    0,
                ),
                slider(
                    -180.0,
                    180.0,
                    editing.shadow.map(|s| s.angle).unwrap_or(90.0),
                    1,
                ),
                slider(
                    0.0,
                    100.0,
                    editing.shadow.map(|s| s.distance).unwrap_or(20.0),
                    2,
                ),
                slider(
                    0.0,
                    100.0,
                    editing.shadow.map(|s| s.blur).unwrap_or(20.0),
                    3,
                ),
            ],
            LayerEffectKind::ColorOverlay => vec![slider(
                0.0,
                100.0,
                editing
                    .color_overlay
                    .map(|s| s.opacity * 100.0)
                    .unwrap_or(100.0),
                0,
            )],
            LayerEffectKind::InnerShadow => vec![
                slider(
                    0.0,
                    100.0,
                    editing
                        .inner_shadow
                        .map(|s| s.opacity * 100.0)
                        .unwrap_or(50.0),
                    0,
                ),
                slider(
                    -180.0,
                    180.0,
                    editing.inner_shadow.map(|s| s.angle).unwrap_or(90.0),
                    1,
                ),
                slider(
                    0.0,
                    50.0,
                    editing.inner_shadow.map(|s| s.distance).unwrap_or(10.0),
                    2,
                ),
                slider(
                    0.0,
                    100.0,
                    editing.inner_shadow.map(|s| s.blur).unwrap_or(10.0),
                    3,
                ),
            ],
            LayerEffectKind::OuterGlow => vec![
                slider(
                    0.0,
                    100.0,
                    editing.outer_glow.map(|s| s.size).unwrap_or(20.0),
                    0,
                ),
                slider(
                    0.0,
                    100.0,
                    editing
                        .outer_glow
                        .map(|s| s.opacity * 100.0)
                        .unwrap_or(75.0),
                    1,
                ),
            ],
            LayerEffectKind::InnerGlow => vec![
                slider(
                    0.0,
                    100.0,
                    editing.inner_glow.map(|s| s.size).unwrap_or(10.0),
                    0,
                ),
                slider(
                    0.0,
                    100.0,
                    editing
                        .inner_glow
                        .map(|s| s.opacity * 100.0)
                        .unwrap_or(75.0),
                    1,
                ),
            ],
        };
        let kind_index = LayerEffectKind::ALL
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(0);
        // Stroke's Outside/Inside position rides in `inside`.
        let inside = editing.stroke.map(|stroke| stroke.inside).unwrap_or(false);
        self.panel_widgets.effects = Some(EffectsPanelState {
            kind_index,
            sliders,
            inside,
        });
        self.panels.show(
            panel_names::EFFECTS_PANEL,
            kind.label(),
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `finishEffectsEditing(commit: false)` (`EffectsSheet.swift:20`).
    pub fn cancel_effects_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::EFFECTS_PANEL) {
            return;
        }
        self.tab()
            .session
            .update(cx, |session, _| session.finish_effects_editing(false));
        self.panel_widgets.effects = None;
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `finishEffectsEditing(commit: true)` (`EffectsSheet.swift:22`): every
    /// change was already a committed undo step; OK just ends the session.
    pub fn confirm_effects_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::EFFECTS_PANEL) {
            return;
        }
        self.tab()
            .session
            .update(cx, |session, _| session.finish_effects_editing(true));
        self.panel_widgets.effects = None;
        self.panels.close(panel_names::EFFECTS_PANEL);
        self.sync_panel_guard();
        self.touch(cx);
    }

    /// `EffectsSheet`'s body: the edited effect's controls. Stroke's Position
    /// is the segmented Outside/Inside picker (`EffectsSheet.swift:37-42`),
    /// rendered in the title row like Swift's headline row.
    pub(crate) fn effects_panel_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = self.panel_widgets.effects.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let kind = LayerEffectKind::ALL
            .get(panel.kind_index)
            .copied()
            .unwrap_or(LayerEffectKind::Stroke);
        let inside = panel.inside;
        let sliders = panel.sliders.clone();
        let color = self.tab().session.read(cx).editing_effects().color(kind);
        let title_row = |cx: &mut Context<Self>| {
            let mut row = gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(text_label(kind.label(), false).text_size(px(13.0)))
                .child(gpui_kit::div().flex_1());
            if kind == LayerEffectKind::Stroke {
                row = row
                    .child(stroke_segment_button(0, inside, cx))
                    .child(stroke_segment_button(1, inside, cx));
            }
            row
        };
        let mut body = gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(title_row(cx));
        match kind {
            LayerEffectKind::Stroke => {
                body = body
                    .child(SheetRow::new("Color").with_control(effect_swatch(color, kind, cx)))
                    .child(SheetRow::new("Size (px)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            LayerEffectKind::Shadow => {
                body = body
                    .child(SheetRow::new("Color").with_control(effect_swatch(color, kind, cx)))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Angle (°)").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Distance (px)").with_optional_control(
                        sliders.get(2).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Blur (px)").with_optional_control(
                        sliders.get(3).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            LayerEffectKind::ColorOverlay => {
                body = body
                    .child(SheetRow::new("Color").with_control(effect_swatch(color, kind, cx)))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            LayerEffectKind::InnerShadow => {
                body = body
                    .child(SheetRow::new("Color").with_control(effect_swatch(color, kind, cx)))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Angle (°)").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Distance (px)").with_optional_control(
                        sliders.get(2).map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Blur (px)").with_optional_control(
                        sliders.get(3).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            LayerEffectKind::OuterGlow | LayerEffectKind::InnerGlow => {
                body = body
                    .child(SheetRow::new("Color").with_control(effect_swatch(color, kind, cx)))
                    .child(SheetRow::new("Size (px)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
        }
        body.into_any_element()
    }
}

/// The Stroke position picker's segments in Swift `Picker` order
/// (`EffectsSheet.swift:39-41`: Outside, then Inside).
const STROKE_POSITION_SEGMENTS: [&str; 2] = ["Outside", "Inside"];
/// Whether `segment` (an index into [`STROKE_POSITION_SEGMENTS`]) is the
/// selected one for a stroke at `inside` — the picker's `get` binding
/// (`get: { effect.inside }` with `Text("Outside").tag(false)`,
/// `Text("Inside").tag(true)`).
fn stroke_segment_selected(inside: bool, segment: usize) -> bool {
    (segment == 1) == inside
}

/// The position a click on `segment` writes — the picker's `set` binding.
fn stroke_segment_target(segment: usize) -> bool {
    segment == 1
}

/// One segment of the Stroke position picker: two side-by-side toggle
/// buttons, the selected one on the accent (gpui-component's `selected`
/// button style), standing in for Swift's `NSSegmentedControl`-style
/// `.pickerStyle(.segmented)`.
fn stroke_segment_button(
    segment: usize,
    inside: bool,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    Button::new(SharedString::from(format!("stroke-position-{segment}")))
        .label(STROKE_POSITION_SEGMENTS[segment])
        .small()
        .selected(stroke_segment_selected(inside, segment))
        .on_click(cx.listener(move |state, _, _window, cx| {
            set_stroke_inside(state, stroke_segment_target(segment), cx);
        }))
        .into_any_element()
}

/// Stroke's Outside/Inside change goes through the session like every slider,
/// so the canvas and the undo trail follow (`EffectsSheet.swift`'s picker).
fn set_stroke_inside(state: &mut AppState, inside: bool, cx: &mut Context<AppState>) {
    state.tab().session.update(cx, |session, _| {
        session.change_effects(|effects| {
            if let Some(stroke) = effects.stroke.as_mut() {
                stroke.inside = inside;
            }
        })
    });
    if let Some(panel) = state.panel_widgets.effects.as_mut() {
        panel.inside = inside;
    }
    state.touch(cx);
}

/// The effect's color swatch, opening the color picker panel on click
/// (EffectsSheet's color rows). Unset components show the effect's default.
fn effect_swatch(
    color: Option<compositor_core::document::PaletteColor>,
    kind: LayerEffectKind,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    use gpui_kit::{StatefulInteractiveElement as _, Styled as _};
    let color = color.unwrap_or_default();
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    let rgb = (channel(color.red) << 16) | (channel(color.green) << 8) | channel(color.blue);
    gpui_kit::div()
        .id(SharedString::from(format!(
            "effect-swatch-{}",
            kind.label()
        )))
        .w(px(36.0))
        .h(px(18.0))
        .rounded_xs()
        .border_1()
        .border_color(gpui_kit::rgb(crate::ui::BORDER_INSET))
        .bg(gpui_kit::rgb(rgb))
        .on_click(cx.listener(move |state, _, _, cx| {
            state.open_effect_color_picker(kind, cx);
        }))
        .into_any_element()
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    /// The Stroke position segments match Swift's Picker tags
    /// (`EffectsSheet.swift:39-41`): Outside first, Inside second, exactly
    /// one selected at a time, and clicking the other segment switches.
    #[test]
    fn stroke_position_segments_match_swift_picker() {
        assert_eq!(STROKE_POSITION_SEGMENTS, ["Outside", "Inside"]);
        for (inside, selected) in [(false, 0), (true, 1)] {
            for segment in 0..STROKE_POSITION_SEGMENTS.len() {
                assert_eq!(
                    stroke_segment_selected(inside, segment),
                    segment == selected,
                    "inside={inside} segment={segment}"
                );
            }
        }
        // The click target mirrors the tag: segment 0 = Outside (false),
        // segment 1 = Inside (true). Clicking the selected segment keeps it.
        assert!(!stroke_segment_target(0));
        assert!(stroke_segment_target(1));
        for segment in 0..STROKE_POSITION_SEGMENTS.len() {
            assert!(
                stroke_segment_selected(stroke_segment_target(segment), segment),
                "target of segment {segment} must select it"
            );
        }
    }
}
