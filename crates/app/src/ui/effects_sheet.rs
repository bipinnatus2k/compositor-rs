//! Port of the Effects sheet (`EffectsSheet.swift`) as its own view entity
//! (ADR 0003 step D, ticket 41): which effect is being edited, its sliders
//! and Stroke's Outside/Inside position — the sheet view's edit state —
//! rendered with the body and footer; every slider change is a committed
//! undo step on the panel's own session, and the shell's status/epoch ride
//! the seam.

use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    px,
};

use compositor_core::effects::LayerEffectKind;
use compositor_core::session::EditorSession;

use crate::sheets_ui::SheetRow;
use crate::state::ProjectWorkspaceView;
use crate::ui::floating_panel::{
    PanelPlacement, panel_body_frame, panel_footer, panel_names, panel_on, panel_on_click,
};
use crate::ui::sheets::panel_value;
use crate::ui::{ClickHandler, text_label};

/// The Effects panel as its own view entity (Swift `EffectsSheet`'s edit
/// state; ticket 41). Created by `ProjectWorkspaceView::open_effects_panel`, dropped on
/// close; its session can never go stale (tab switches are blocked while
/// any panel is open).
pub struct EffectsPanelView {
    /// The session the panel opened on: the edited effect's readouts and
    /// every slider write go through it (one undo step per change, as
    /// Swift's changeEffects does).
    session: Entity<EditorSession>,
    /// The root view (the window shell): the commit/cancel flows
    /// and the repaint epoch stay there.
    app: WeakEntity<ProjectWorkspaceView>,
    /// Which effect is being edited (an index into `LayerEffectKind::ALL`).
    kind_index: usize,
    /// The edited effect's sliders.
    sliders: Vec<Entity<SliderState>>,
    /// Stroke's Outside/Inside position.
    inside: bool,
    subscriptions: Vec<Subscription>,
}

impl EffectsPanelView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        cx: &mut Context<Self>,
    ) -> Self {
        // No preview capture: every slider change is committed to the layer's
        // effects right away (one undo step per change, as Swift's
        // changeEffects does).
        let editing = session.read(cx).editing_effects();
        let kind = session
            .read(cx)
            .effects_editing
            .map(|editing| editing.kind)
            .unwrap_or(LayerEffectKind::Stroke);
        let mut panel = EffectsPanelView {
            session,
            app,
            kind_index: 0,
            sliders: Vec::new(),
            inside: false,
            subscriptions: Vec::new(),
        };
        let mut slider = |min: f64, max: f64, value: f64, index: usize| {
            panel.make_slider(min as f32, max as f32, value as f32, index, cx)
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
        panel.sliders = sliders;
        panel.kind_index = LayerEffectKind::ALL
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(0);
        // Stroke's Outside/Inside position rides in `inside`.
        panel.inside = editing.stroke.map(|stroke| stroke.inside).unwrap_or(false);
        panel
    }

    /// Creates one of the panel's sliders. Every slider funnels its changes
    /// into [`Self::apply_sliders`] — the write-back reads all sliders at
    /// once (EffectsSheet.swift's per-kind `changeEffects` closures), so the
    /// index only tags the slider as an effects one.
    fn make_slider(
        &mut self,
        min: f32,
        max: f32,
        value: f32,
        _index: usize,
        cx: &mut Context<Self>,
    ) -> Entity<SliderState> {
        let state = cx.new(|_| {
            crate::state::slider(min, max, if max - min > 12.0 { 1.0 } else { 0.1 }, value)
        });
        let subscription = cx.subscribe(&state, |this, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            this.slider_changed(value, cx);
        });
        self.subscriptions.push(subscription);
        state
    }

    /// A slider moved: the values land on the bound layer's effect — a real
    /// undo step per change through `changeEffects`, which is what makes the
    /// canvas preview live (`apply_effects_sliders`, moved with the panel).
    fn slider_changed(&mut self, _value: f32, cx: &mut Context<Self>) {
        self.apply_sliders(cx);
        // The old dispatcher's tail landed on top of `apply_effects_sliders`'s
        // own touch: the double bump is the exact old shape.
        self.touch(cx);
    }

    fn apply_sliders(&mut self, cx: &mut Context<Self>) {
        let cx_app: &gpui_kit::App = cx;
        let kind = self.edited_kind();
        let values: Vec<f64> = (0..self.sliders.len())
            .map(|index| panel_value(&self.sliders, cx_app, index) as f64)
            .collect();
        let value = |index: usize| values.get(index).copied().unwrap_or(0.0);
        self.session.update(cx, move |session, _| {
            session.change_effects(move |effects| match kind {
                LayerEffectKind::Stroke => {
                    if let Some(stroke) = effects.stroke.as_mut() {
                        stroke.size = value(0);
                        stroke.opacity = value(1) / 100.0;
                    }
                }
                LayerEffectKind::Shadow => {
                    if let Some(shadow) = effects.shadow.as_mut() {
                        shadow.opacity = value(0) / 100.0;
                        shadow.angle = value(1);
                        shadow.distance = value(2);
                        shadow.blur = value(3);
                    }
                }
                LayerEffectKind::ColorOverlay => {
                    if let Some(overlay) = effects.color_overlay.as_mut() {
                        overlay.opacity = value(0) / 100.0;
                    }
                }
                LayerEffectKind::InnerShadow => {
                    if let Some(inner) = effects.inner_shadow.as_mut() {
                        inner.opacity = value(0) / 100.0;
                        inner.angle = value(1);
                        inner.distance = value(2);
                        inner.blur = value(3);
                    }
                }
                LayerEffectKind::OuterGlow => {
                    if let Some(glow) = effects.outer_glow.as_mut() {
                        glow.size = value(0);
                        glow.opacity = value(1) / 100.0;
                    }
                }
                LayerEffectKind::InnerGlow => {
                    if let Some(glow) = effects.inner_glow.as_mut() {
                        glow.size = value(0);
                        glow.opacity = value(1) / 100.0;
                    }
                }
            });
        });
        self.touch(cx);
    }

    fn edited_kind(&self) -> LayerEffectKind {
        LayerEffectKind::ALL
            .get(self.kind_index)
            .copied()
            .unwrap_or(LayerEffectKind::Stroke)
    }

    /// Stroke's Outside/Inside change goes through the session like every
    /// slider, so the canvas and the undo trail follow (`EffectsSheet.swift`'s
    /// picker).
    fn set_stroke_inside(&mut self, inside: bool, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.change_effects(|effects| {
                if let Some(stroke) = effects.stroke.as_mut() {
                    stroke.inside = inside;
                }
            })
        });
        self.inside = inside;
        self.touch(cx);
    }

    /// The color swatch's click: opens the color picker on the effect's
    /// color (EffectsSheet.swift's `openEffectColorPicker`; guards and state
    /// live on the core session in `color_picker.rs`). Nothing is written
    /// into the layer until the picker closes.
    fn open_color_picker(&mut self, kind: LayerEffectKind, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, _| session.open_effect_color_picker(kind));
        self.touch(cx);
    }

    /// The panel's route to the app-side repaint epoch (`touch`): the frame
    /// cache and `revision` stay on the shell (ticket 39's field audit).
    fn touch(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| app.touch(cx)).ok();
    }

    /// The panel's commit/cancel routes: the footer's OK/Cancel are
    /// panel-entity callbacks that reach the shell's pipelines through the
    /// seam (ticket 41's ruling 4).
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.cancel_effects_panel(cx))
            .ok();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.confirm_effects_panel(cx))
            .ok();
    }
}

impl Render for EffectsPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let kind = self.edited_kind();
        let inside = self.inside;
        let sliders = self.sliders.clone();
        let color = self.session.read(cx).editing_effects().color(kind);
        let mut body = gpui_kit::div().flex().flex_col().gap_1().child(
            // Stroke's Position is the segmented Outside/Inside picker
            // (`EffectsSheet.swift:37-42`), rendered in the title row like
            // Swift's headline row.
            gpui_kit::div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(text_label(kind.label(), false).text_size(px(13.0)))
                .child(gpui_kit::div().flex_1())
                .children((kind == LayerEffectKind::Stroke).then(|| {
                    StrokePositionPicker::new(inside)
                        .with_on_pick(panel_on(&this, |panel, inside, _, cx| {
                            panel.set_stroke_inside(inside, cx)
                        }))
                })),
        );
        match kind {
            LayerEffectKind::Stroke => {
                body = body
                    .child(
                        SheetRow::new("Color").with_control(
                            EffectSwatch::new(kind, color)
                                .with_on_click(panel_on_click(&this, move |panel, _, _, cx| {
                                    panel.open_color_picker(kind, cx)
                                })),
                        ),
                    )
                    .child(SheetRow::new("Size (px)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            LayerEffectKind::Shadow => {
                body = body
                    .child(
                        SheetRow::new("Color").with_control(
                            EffectSwatch::new(kind, color)
                                .with_on_click(panel_on_click(&this, move |panel, _, _, cx| {
                                    panel.open_color_picker(kind, cx)
                                })),
                        ),
                    )
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
                    .child(
                        SheetRow::new("Color").with_control(
                            EffectSwatch::new(kind, color)
                                .with_on_click(panel_on_click(&this, move |panel, _, _, cx| {
                                    panel.open_color_picker(kind, cx)
                                })),
                        ),
                    )
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
            LayerEffectKind::InnerShadow => {
                body = body
                    .child(
                        SheetRow::new("Color").with_control(
                            EffectSwatch::new(kind, color)
                                .with_on_click(panel_on_click(&this, move |panel, _, _, cx| {
                                    panel.open_color_picker(kind, cx)
                                })),
                        ),
                    )
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
                    .child(
                        SheetRow::new("Color").with_control(
                            EffectSwatch::new(kind, color)
                                .with_on_click(panel_on_click(&this, move |panel, _, _, cx| {
                                    panel.open_color_picker(kind, cx)
                                })),
                        ),
                    )
                    .child(SheetRow::new("Size (px)").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(SheetRow::new("Opacity (%)").with_optional_control(
                        sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                    ));
            }
        }
        let footer = panel_footer(
            panel_names::EFFECTS_PANEL,
            &this,
            false,
            |panel, _, cx| panel.cancel(cx),
            |panel, _, cx| panel.confirm(cx),
        );
        panel_body_frame(body.into_any_element(), footer)
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

/// The Stroke position picker (`EffectsSheet.swift`'s segmented Picker,
/// `get: { effect.inside }` with Outside/Inside tags): two side-by-side
/// toggle buttons, the selected one on the accent (gpui-component's
/// `selected` button style); a click reports the picked `inside` value.
#[derive(IntoElement)]
pub struct StrokePositionPicker {
    inside: bool,
    on_pick: crate::ui::AppCallback<bool>,
}

impl StrokePositionPicker {
    pub fn new(inside: bool) -> Self {
        StrokePositionPicker {
            inside,
            on_pick: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_pick(mut self, on_pick: crate::ui::AppCallback<bool>) -> Self {
        self.on_pick = on_pick;
        self
    }
}

impl RenderOnce for StrokePositionPicker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_pick = self.on_pick;
        gpui_kit::div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .children((0..STROKE_POSITION_SEGMENTS.len()).map(move |segment| {
                let on_pick = on_pick.clone();
                Button::new(SharedString::from(format!("stroke-position-{segment}")))
                    .label(STROKE_POSITION_SEGMENTS[segment])
                    .small()
                    .selected(stroke_segment_selected(self.inside, segment))
                    .on_click(move |_, window, cx| {
                        on_pick(stroke_segment_target(segment), window, cx)
                    })
            }))
    }
}

/// The effect's color swatch (`EffectsSheet`'s color rows): the effect's
/// color, or its default when unset; clicking opens the color picker.
#[derive(IntoElement)]
pub struct EffectSwatch {
    kind: LayerEffectKind,
    color: compositor_core::document::PaletteColor,
    on_click: ClickHandler,
}

impl EffectSwatch {
    pub fn new(
        kind: LayerEffectKind,
        color: Option<compositor_core::document::PaletteColor>,
    ) -> Self {
        EffectSwatch {
            kind,
            color: color.unwrap_or_default(),
            on_click: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_click(mut self, on_click: ClickHandler) -> Self {
        self.on_click = on_click;
        self
    }
}

impl RenderOnce for EffectSwatch {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_click = self.on_click;
        let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        let rgb = (channel(self.color.red) << 16)
            | (channel(self.color.green) << 8)
            | channel(self.color.blue);
        gpui_kit::div()
            .id(SharedString::from(format!(
                "effect-swatch-{}",
                self.kind.label()
            )))
            .w(px(36.0))
            .h(px(18.0))
            .rounded_xs()
            .border_1()
            .border_color(gpui_kit::rgb(crate::ui::BORDER_INSET))
            .bg(gpui_kit::rgb(rgb))
            .on_click(move |event, window, cx| on_click(event, window, cx))
    }
}

impl ProjectWorkspaceView {
    /// `session.effectsEditing` → effectsPanel, title = the effect's name
    /// (`ContentView.swift:217-224`). The caller (`add_effect`) has already
    /// started the editing session; the panel reads it at construction.
    pub fn open_effects_panel(&mut self, cx: &mut Context<Self>) {
        let session = self.workspace.read(cx).current().session.clone();
        let app = cx.entity().downgrade();
        let panel = cx.new(|cx| EffectsPanelView::new(session, app, cx));
        self.panel_views.insert(
            panel_names::EFFECTS_PANEL,
            crate::ui::floating_panel::PanelContent::Effects(panel),
        );
        // The title follows the edited kind, as the old open read it.
        let title = {
            let kind = self
                .workspace
                .read(cx)
                .current()
                .session
                .read(cx)
                .effects_editing;
            kind.map(|editing| editing.kind)
                .unwrap_or(LayerEffectKind::Stroke)
        }
        .label();
        self.panels
            .show(panel_names::EFFECTS_PANEL, title, PanelPlacement::Automatic);
        self.sync_panel_guard(cx);
        self.touch(cx);
    }

    /// `finishEffectsEditing(commit: false)` (`EffectsSheet.swift:20`).
    pub fn cancel_effects_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::EFFECTS_PANEL) {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.finish_effects_editing(false))
        });
        self.panel_views.remove(panel_names::EFFECTS_PANEL);
        self.sync_panel_guard(cx);
        self.touch(cx);
    }

    /// `finishEffectsEditing(commit: true)` (`EffectsSheet.swift:22`): every
    /// change was already a committed undo step; OK just ends the session.
    pub fn confirm_effects_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::EFFECTS_PANEL) {
            return;
        }
        self.workspace.update(cx, |ws, cx| {
            ws.current_mut()
                .session
                .update(cx, |session, _| session.finish_effects_editing(true))
        });
        self.panel_views.remove(panel_names::EFFECTS_PANEL);
        self.panels.close(panel_names::EFFECTS_PANEL);
        self.sync_panel_guard(cx);
        self.touch(cx);
    }
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
