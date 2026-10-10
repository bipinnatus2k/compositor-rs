//! Port of the Levels sheet (`LevelsSheet.swift`) as its own view entity
//! (ADR 0003 step D, ticket 41): the panel owns the five sliders, the
//! visible channel, the Preview toggle and the histogram — the sheet view's
//! `@State` — and renders its body and footer; the shell keeps the preview
//! pipelines and the commit flows, reached through the panel's seam.

use std::rc::Rc;

use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::{
    App, AppContext, Context, ElementId, Entity, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, Styled, Subscription, WeakEntity, Window, px,
};

use compositor_core::adjust::LevelsChannel;
use compositor_core::adjust::LevelsSettings;
use compositor_core::adjust::levels_auto::LevelsAuto;
use compositor_core::session::EditorSession;

use crate::sheets_ui::{HistogramView, SheetRow};
use crate::state::ProjectWorkspaceView;
use crate::ui::floating_panel::{
    PanelPlacement, panel_body_frame, panel_footer, panel_names, panel_on,
};
use crate::ui::sheets::panel_value;
use crate::ui::{Dropdown, LabeledCheckbox, MenuEntry, app_dispatch};

/// The Levels/Curves channel picker's labels (LevelsSheet.swift's Picker).
pub fn levels_channel_label(channel: LevelsChannel) -> &'static str {
    match channel {
        LevelsChannel::Rgb => "RGB",
        LevelsChannel::Red => "Red",
        LevelsChannel::Green => "Green",
        LevelsChannel::Blue => "Blue",
    }
}

/// The Levels panel as its own view entity (Swift `LevelsSheet`'s edit
/// state; ticket 41). It holds the session plus the transitional
/// `WeakEntity<ProjectWorkspaceView>` seam: the sliders, the Preview checkbox, the
/// channel picker and the Auto trio run here (they read this panel's own
/// widgets), while the preview recomputes and the commit/cancel pipelines
/// stay on the shell. The entity exists exactly while the panel is open —
/// created by `ProjectWorkspaceView::open_levels_panel`, dropped on close — so its
/// session can never go stale (tab switches are blocked while any panel is
/// open, and the root's workspace observation does not reach panels).
pub struct LevelsPanelView {
    /// The session the panel opened on. Unread today — the handlers funnel
    /// through the shell seam, and the panel's lifetime sits inside one
    /// modal session (tab switches blocked) so it can never go stale.
    /// Carried for the house view-entity convention (every view holds its
    /// session; the views that read it do); removal is a ticket-44 cleanup
    /// candidate.
    #[allow(dead_code)]
    session: Entity<EditorSession>,
    /// The root view (the window shell): the preview recomputes and
    /// the commit/cancel flows stay there (ticket 41's handler split).
    app: WeakEntity<ProjectWorkspaceView>,
    /// The five sliders: input black, gamma, input white, output black,
    /// output white.
    sliders: Vec<Entity<SliderState>>,
    /// The visible channel (0 = RGB).
    channel: usize,
    /// Swift's LevelsEdit.preview (LevelsSheet's Preview toggle): on when opened.
    preview: bool,
    /// The histogram computed at open (selection-weighted), feeding Auto.
    histogram: [f64; 1024],
    /// The full settings the Auto modes write wholesale; the visible
    /// channel's range is the sliders'.
    levels: LevelsSettings,
    subscriptions: Vec<Subscription>,
}

impl LevelsPanelView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<ProjectWorkspaceView>,
        histogram: [f64; 1024],
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = LevelsPanelView {
            session,
            app,
            sliders: Vec::new(),
            channel: 0,
            preview: true,
            histogram,
            levels: LevelsSettings::default(),
            subscriptions: Vec::new(),
        };
        panel.sliders = vec![
            panel.make_slider(0.0, 254.0, 0.0, cx),
            panel.make_slider(0.1, 9.99, 1.0, cx),
            panel.make_slider(1.0, 255.0, 255.0, cx),
            panel.make_slider(0.0, 255.0, 0.0, cx),
            panel.make_slider(0.0, 255.0, 255.0, cx),
        ];
        panel
    }

    /// Creates one of the panel's sliders. Every slider funnels its changes
    /// into [`Self::slider_changed`] — the write-back reads all five
    /// sliders, so no per-slider role is needed.
    fn make_slider(
        &mut self,
        min: f32,
        max: f32,
        value: f32,
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

    /// A slider moved: the shell recomputes the live preview from this
    /// panel's settings (`apply_slider`'s Levels branch).
    fn slider_changed(&mut self, _value: f32, cx: &mut Context<Self>) {
        let settings = self.levels_settings(cx);
        let preview = self.preview;
        self.app
            .update(cx, |app, cx| {
                app.apply_levels_preview(settings, preview, cx)
            })
            .ok();
    }

    /// The panel's five slider entities, input black → output white. Read
    /// seam for the ticket-37 invalidation scenario tests, which fire real
    /// `SliderEvent`s at the first slider (the path a drag produces).
    #[cfg(test)]
    pub(crate) fn sliders(&self) -> &[Entity<SliderState>] {
        &self.sliders
    }

    /// The Preview checkbox flipped (or Option-P): the shell's refresh path,
    /// which — unlike a slider change — repaints on the adjustment path too
    /// (`refresh_levels_preview`'s exact shape).
    pub(crate) fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview = !self.preview;
        self.refresh_preview(cx);
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let settings = self.levels_settings(cx);
        let preview = self.preview;
        self.app
            .update(cx, |app, cx| {
                app.refresh_levels_preview(settings, preview, cx)
            })
            .ok();
    }

    /// The channel dropdown's write-back: only the visible channel changes,
    /// exactly as the sheet's `set_channel` did.
    pub(crate) fn set_channel(&mut self, channel: usize, cx: &mut Context<Self>) {
        self.channel = channel;
        cx.notify();
    }

    /// The panel's settings: the Auto modes set every channel's range at
    /// once; those keep their values while the sliders stay the source of
    /// truth for the visible channel's range.
    fn levels_settings(&self, cx: &App) -> LevelsSettings {
        let mut settings = self.levels;
        settings.channel = match self.channel {
            1 => LevelsChannel::Red,
            2 => LevelsChannel::Green,
            3 => LevelsChannel::Blue,
            _ => LevelsChannel::Rgb,
        };
        let mut range = settings.current();
        range.black = panel_value(&self.sliders, cx, 0) as f64;
        range.gamma = panel_value(&self.sliders, cx, 1) as f64;
        range.white = panel_value(&self.sliders, cx, 2) as f64;
        range.output_black = panel_value(&self.sliders, cx, 3) as f64;
        range.output_white = panel_value(&self.sliders, cx, 4) as f64;
        settings.set_current(range);
        settings
    }

    /// `autoLevels` (LevelsAutomatic.swift): the mode's settings replace the
    /// edit's wholesale, then the visible sliders follow the current
    /// channel's range and the preview refreshes, exactly as manual slider
    /// edits do.
    fn auto_mode(&mut self, mode: LevelsAuto, window: &mut Window, cx: &mut Context<Self>) {
        let settings = mode.settings(&self.histogram);
        let values = levels_auto_slider_values(&settings, self.channel);
        self.levels = settings;
        for (index, value) in values.into_iter().enumerate() {
            if let Some(slider) = self.sliders.get(index) {
                let slider = slider.clone();
                slider.update(cx, |state, cx| {
                    state.set_value(SliderValue::Single(value), window, cx)
                });
            }
        }
        self.slider_changed(values[0], cx);
    }
}

impl Render for LevelsPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let app = self.app.clone();
        let channel = self.channel;
        let histogram = self.histogram;
        let sliders = self.sliders.clone();
        let preview = self.preview;

        let body = gpui_kit::div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                SheetRow::new("Preview").with_control(
                    LabeledCheckbox::new("levels-preview", "Preview")
                        .checked(preview)
                        .with_on_change(panel_on(&this, |panel, checked, _, cx| {
                            panel.preview = checked;
                            panel.refresh_preview(cx);
                        })),
                ),
            )
            .child(
                SheetRow::new("Channel").with_control(
                    Dropdown::new(
                        "levels-channel",
                        match channel {
                            1 => "Red",
                            2 => "Green",
                            3 => "Blue",
                            _ => "RGB",
                        },
                    )
                    .with_width(px(90.0))
                    .with_entries(vec![
                        MenuEntry::new("RGB", move |state, _, cx| set_levels_channel(state, 0, cx)),
                        MenuEntry::new("Red", move |state, _, cx| set_levels_channel(state, 1, cx)),
                        MenuEntry::new("Green", move |state, _, cx| {
                            set_levels_channel(state, 2, cx)
                        }),
                        MenuEntry::new("Blue", move |state, _, cx| {
                            set_levels_channel(state, 3, cx)
                        }),
                    ])
                    // The dropdown keeps the app-shell dispatch (ticket 40's
                    // transitional seam; ticket 43 retires it with ui.rs).
                    .with_dispatch(app_dispatch(&app)),
                ),
            )
            .child(
                gpui_kit::div()
                    .w_full()
                    .h(px(96.0))
                    .rounded_sm()
                    .overflow_hidden()
                    .child(HistogramView::new(histogram)),
            )
            .child(
                SheetRow::new("Input Black").with_optional_control(
                    sliders.first().map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Gamma").with_optional_control(
                    sliders.get(1).map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Input White").with_optional_control(
                    sliders.get(2).map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Output Black").with_optional_control(
                    sliders.get(3).map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Output White").with_optional_control(
                    sliders.get(4).map(|slider| Slider::new(slider).flex_1()),
                ),
            )
            .child(
                SheetRow::new("Auto").with_control(
                    gpui_kit::div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(
                            AutoButton::new(
                                "levels-auto-contrast",
                                "Contrast",
                                LevelsAuto::Contrast,
                            )
                            .with_on_auto(panel_on(
                                &this,
                                |panel, mode, window, cx| {
                                    panel.auto_mode(mode, window, cx);
                                },
                            )),
                        )
                        .child(
                            AutoButton::new("levels-auto-color", "Color", LevelsAuto::Color)
                                .with_on_auto(panel_on(&this, |panel, mode, window, cx| {
                                    panel.auto_mode(mode, window, cx);
                                })),
                        )
                        .child(
                            AutoButton::new("levels-auto-neutral", "Neutral", LevelsAuto::Neutral)
                                .with_on_auto(panel_on(&this, |panel, mode, window, cx| {
                                    panel.auto_mode(mode, window, cx);
                                })),
                        ),
                ),
            )
            .into_any_element();
        let footer = panel_footer(
            panel_names::LEVELS_PANEL,
            &self.app,
            false,
            |root, _, cx| root.cancel_levels_panel(cx),
            |root, _, cx| root.confirm_levels_panel(cx),
        );
        panel_body_frame(body, footer)
    }
}

/// The Levels channel dropdown's write-back: a menu entry is
/// ProjectWorkspaceView-flavored, so it lands on the shell and reads the panel through
/// the content registry (the ticket-40 transitional pattern for `MenuEntry`
/// dropdowns).
fn set_levels_channel(
    state: &mut ProjectWorkspaceView,
    channel: usize,
    cx: &mut Context<ProjectWorkspaceView>,
) {
    if let Some(panel) = state.levels_view() {
        panel.update(cx, |panel, cx| panel.set_channel(channel, cx));
    }
}

/// Slider values (input black, gamma, input white, output black, output
/// white) shown for `channel` after Auto replaces the settings.
fn levels_auto_slider_values(settings: &LevelsSettings, channel: usize) -> [f32; 5] {
    let range = settings.ranges[channel.min(3)];
    [
        range.black as f32,
        range.gamma as f32,
        range.white as f32,
        range.output_black as f32,
        range.output_white as f32,
    ]
}

/// One Auto button (`LevelsSheet.swift:64`): the mode's settings replace
/// the edit's wholesale, then the sliders and the preview follow; a click
/// reports the mode.
#[derive(IntoElement)]
pub struct AutoButton {
    id: ElementId,
    label: SharedString,
    mode: LevelsAuto,
    on_auto: crate::ui::AppCallback<LevelsAuto>,
}

impl AutoButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, mode: LevelsAuto) -> Self {
        AutoButton {
            id: id.into(),
            label: label.into(),
            mode,
            on_auto: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_on_auto(mut self, on_auto: crate::ui::AppCallback<LevelsAuto>) -> Self {
        self.on_auto = on_auto;
        self
    }
}

impl RenderOnce for AutoButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let on_auto = self.on_auto;
        Button::new(self.id)
            .label(self.label)
            .small()
            .outline()
            .on_click(move |_, window, cx| on_auto(self.mode, window, cx))
    }
}

impl ProjectWorkspaceView {
    /// `showLevelsPanel` (`LevelsSheet` via the panel-spawning onChange):
    /// capture the preview base, compute the histogram, and put the panel
    /// entity in the registry.
    pub fn open_levels_panel(&mut self, cx: &mut Context<Self>) {
        self.begin_preview(cx);
        // With a selection, bins are weighted by the selection's coverage on
        // the layer's pixel grid (LevelsFilter.histogram, Levels.swift:90-96).
        let mut histogram = [0.0; 1024];
        if let Some(pixels) = self.preview_base.clone() {
            let coverage = crate::ui::sheets::histogram_coverage(
                self.preview_base_transform,
                self.workspace
                    .read(cx)
                    .current()
                    .session
                    .read(cx)
                    .selection()
                    .cloned(),
                pixels.width,
                pixels.height,
            );
            compositor_core::adjust::levels_histogram(
                pixels.as_ref(),
                coverage.as_ref(),
                &mut histogram,
            );
        }
        let session = self.workspace.read(cx).current().session.clone();
        let app = cx.entity().downgrade();
        let panel = cx.new(|cx| LevelsPanelView::new(session, app, histogram, cx));
        self.panel_views.insert(
            panel_names::LEVELS_PANEL,
            crate::ui::floating_panel::PanelContent::Levels(panel),
        );
        self.panels.show(
            panel_names::LEVELS_PANEL,
            "Levels",
            PanelPlacement::Automatic,
        );
        self.sync_panel_guard(cx);
        self.touch(cx);
    }

    /// `onClose` semantics = Cancel (`cancelLevels`): the panel closes and
    /// an adjustment edit in progress is discarded.
    pub fn cancel_levels_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::LEVELS_PANEL) {
            return;
        }
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.finish_adjustment_editing(false))
            });
        }
        self.panel_views.remove(panel_names::LEVELS_PANEL);
        self.after_panel_editing_closed(cx);
    }

    /// `commitLevels` (`LevelsSheet.swift:82`): the settings previewed live
    /// are kept as one undo step.
    pub fn confirm_levels_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.is_visible(panel_names::LEVELS_PANEL) {
            return;
        }
        // Editing an adjustment layer: the previewed settings are the commit.
        if self
            .workspace
            .read(cx)
            .current()
            .session
            .read(cx)
            .adjustment_editing_id
            .is_some()
        {
            self.workspace.update(cx, |ws, cx| {
                ws.current_mut()
                    .session
                    .update(cx, |session, _| session.finish_adjustment_editing(true))
            });
        } else {
            self.commit_preview_pixels("Levels", cx);
        }
        self.panel_views.remove(panel_names::LEVELS_PANEL);
        self.panels.close(panel_names::LEVELS_PANEL);
        self.after_panel_editing_closed(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{levels_auto_slider_values, levels_channel_label};
    use compositor_core::adjust::LevelsChannel;
    use compositor_core::adjust::levels_auto::LevelsAuto;

    #[test]
    fn channel_labels_match_the_swift_picker() {
        assert_eq!(levels_channel_label(LevelsChannel::Rgb), "RGB");
        assert_eq!(levels_channel_label(LevelsChannel::Red), "Red");
        assert_eq!(levels_channel_label(LevelsChannel::Green), "Green");
        assert_eq!(levels_channel_label(LevelsChannel::Blue), "Blue");
    }

    /// (Re-hosted with `levels_auto_slider_values` from sheets.rs, ticket 41.)
    #[test]
    fn auto_slider_values_follow_the_current_channel() {
        let mut bins = [0.0; 1024];
        for c in 1..4 {
            bins[c * 256 + 20 * c] = 100.0;
            bins[c * 256 + 200 + c * 10] = 100.0;
        }
        // Contrast touches only the composite range: the RGB sliders move,
        // a channel's sliders stay put.
        let contrast = LevelsAuto::Contrast.settings(&bins);
        assert_eq!(
            levels_auto_slider_values(&contrast, 0),
            [20.0, 1.0, 230.0, 0.0, 255.0]
        );
        assert_eq!(
            levels_auto_slider_values(&contrast, 1),
            [0.0, 1.0, 255.0, 0.0, 255.0]
        );
        // Color's per-channel ranges show up when that channel is visible.
        let color = LevelsAuto::Color.settings(&bins);
        assert_eq!(
            levels_auto_slider_values(&color, 3),
            [60.0, 1.0, 230.0, 0.0, 255.0]
        );
    }
}
