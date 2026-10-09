//! Port of the Filter sheet (`FilterSheet.swift`) as its own view entity
//! (ADR 0003 step D, ticket 41): which filter is open, its sliders, Add
//! Noise's two checkboxes and — folded in, since they share one open/close
//! and one preview path — the Curves editor state (`CurvesControls`'s
//! `settings`/`selected`/`dragging`, the former `AppState.curves_editor`),
//! plus the open-panel-time noise seed (fixed for the panel's whole session,
//! `FilterEdit.seed`, Filters.swift:223). The shell keeps the preview
//! recompute and the commit flows, reached through the panel's seam
//! (Camera Raw's filter panel opens docked per Swift's placement).

use gpui_kit::component::button::Button;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Subscription, WeakEntity, Window, px,
};

use compositor_core::filters::FilterKind;
use compositor_core::filters::FilterSettings;
use compositor_core::session::EditorSession;

use crate::sheets_ui::SheetRow;
use crate::state::{AppState, CurvesEditor, time_seed};
use crate::ui::floating_panel::{
    PanelPlacement, panel_body_frame, panel_footer, panel_names, panel_on, panel_on_drag_move,
    panel_on_drag_up,
};
use crate::ui::levels_sheet::levels_channel_label;
use crate::ui::sheets::panel_value;
use crate::ui::{Dropdown, LabeledCheckbox, MenuEntry, app_dispatch, text_label};

/// The filter panel as its own view entity (Swift `FilterSheet`'s edit
/// state plus `CurvesControls`' editor; ticket 41). Created by
/// `AppState::open_filter_panel`, dropped on close — which is what retires
/// the noise seed and the curves editor ("fixed while the panel is open"
/// semantics, unchanged). Its session can never go stale: tab switches are
/// blocked while any panel is open.
pub struct FilterPanelView {
    /// The session this panel edits. Unread today — every interaction
    /// funnels through the shell seam, and tab switches are blocked while a
    /// panel is open so it can never go stale; kept for ticket 43's per-tab
    /// rebuild.
    #[allow(dead_code)]
    session: Entity<EditorSession>,
    /// The window shell (the transitional root): the preview recompute and
    /// the commit/cancel flows stay there. `pub(crate)`: the curves gesture
    /// impl block lives in curves_controls.rs (the session.rs split-impl
    /// layout).
    pub(crate) app: WeakEntity<AppState>,
    /// Which filter is open.
    kind: FilterKind,
    /// The open filter's sliders (Camera Raw carries fourteen).
    sliders: Vec<Entity<SliderState>>,
    /// Add Noise's Distribution (Swift's Uniform/Gaussian picker).
    gaussian: bool,
    /// Add Noise's Monochromatic toggle.
    monochromatic: bool,
    /// The Curves editor (`CurvesControls.swift`'s `settings`/`selected`/
    /// `dragging`); `Some` only while Curves is the open filter.
    /// `pub(crate)`: the curves gesture impl block lives in
    /// curves_controls.rs (the session.rs split-impl layout).
    pub(crate) curves: Option<CurvesEditor>,
    /// The noise seed drawn when the panel opened: fixed for the session so
    /// changing Amount doesn't reshuffle the pattern, shared by the preview
    /// and the committed pixels (FilterEdit.seed, Filters.swift:223).
    seed: u32,
    subscriptions: Vec<Subscription>,
}

impl FilterPanelView {
    pub(crate) fn new(
        session: Entity<EditorSession>,
        app: WeakEntity<AppState>,
        kind: FilterKind,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = FilterPanelView {
            session,
            app,
            kind,
            sliders: Vec::new(),
            gaussian: false,
            monochromatic: false,
            // A fresh filter replaces any previous editor state (the sheet
            // dropped `curves_editor` in `close_sheet`).
            curves: if kind == FilterKind::Curves {
                Some(CurvesEditor::new())
            } else {
                None
            },
            seed: time_seed(),
            subscriptions: Vec::new(),
        };
        let defaults = FilterSettings::default();
        match kind {
            FilterKind::GaussianBlur => {
                panel.sliders = vec![panel.make_slider(0.1, 250.0, defaults.radius as f32, cx)];
            }
            FilterKind::MotionBlur => {
                panel.sliders = vec![
                    panel.make_slider(-90.0, 90.0, defaults.angle as f32, cx),
                    panel.make_slider(1.0, 2000.0, defaults.distance as f32, cx),
                ];
            }
            FilterKind::AddNoise => {
                panel.sliders = vec![panel.make_slider(0.1, 400.0, defaults.amount as f32, cx)];
            }
            FilterKind::LensCorrection => {
                panel.sliders = vec![panel.make_slider(-100.0, 100.0, 0.0, cx)];
            }
            FilterKind::CameraRaw => {
                let group: [(f32, f32, f32); 14] = [
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
                    .map(|(min, max, value)| panel.make_slider(*min, *max, *value, cx))
                    .collect();
            }
            // Curves edits through `curves`; the automatic filters
            // (Black & White, Color Balance, Exposure, Gradient Map, Grain)
            // run with their default settings and show no sliders.
            _ => {}
        }
        panel
    }

    /// Creates one of the panel's sliders. Every slider funnels its changes
    /// into [`Self::slider_changed`] — the write-back reads all sliders and
    /// the checkboxes, so no per-slider role is needed.
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

    /// A slider moved (or a noise checkbox flipped): the shell recomputes
    /// the live preview from this panel's settings (`apply_slider`'s filter
    /// and CameraRaw branches, `refresh_filter_preview` — one path, since
    /// the old tails agreed).
    fn slider_changed(&mut self, _value: f32, cx: &mut Context<Self>) {
        let kind = self.kind;
        let settings = self.filter_settings(cx);
        self.app
            .update(cx, |app, cx| {
                app.apply_filter_preview(kind, settings, cx);
                app.touch(cx);
            })
            .ok();
    }

    /// The open filter's settings from its sliders and checkboxes.
    pub(crate) fn filter_settings(&self, cx: &gpui_kit::App) -> FilterSettings {
        let mut settings = FilterSettings::default();
        match self.kind {
            FilterKind::GaussianBlur => {
                settings.radius = panel_value(&self.sliders, cx, 0) as f64;
            }
            FilterKind::MotionBlur => {
                settings.angle = panel_value(&self.sliders, cx, 0) as f64;
                settings.distance = panel_value(&self.sliders, cx, 1) as f64;
            }
            FilterKind::AddNoise => {
                settings.amount = panel_value(&self.sliders, cx, 0) as f64;
                settings.gaussian = self.gaussian;
                settings.monochromatic = self.monochromatic;
            }
            FilterKind::LensCorrection => {
                settings.distortion = panel_value(&self.sliders, cx, 0) as f64;
            }
            FilterKind::CameraRaw => {
                let value = |i: usize| panel_value(&self.sliders, cx, i) as f64;
                let raw = &mut settings.camera_raw;
                raw.temperature = value(0);
                raw.tint = value(1);
                raw.exposure = value(2);
                raw.contrast = value(3);
                raw.highlights = value(4);
                raw.shadows = value(5);
                raw.whites = value(6);
                raw.blacks = value(7);
                raw.vibrance = value(8);
                raw.saturation = value(9);
                raw.texture = value(10);
                raw.clarity = value(11);
                raw.dehaze = value(12);
                raw.vignette_amount = value(13);
            }
            _ => {}
        }
        settings
    }

    /// The noise seed the shell's preview pipeline uses while this panel is
    /// open (FilterEdit.seed): read through the registry, like the layers
    /// panel header's slider (the one read from outside this view).
    pub(crate) fn seed(&self) -> u32 {
        self.seed
    }

    /// The open filter's label, for the commit's undo name.
    pub(crate) fn kind_label(&self) -> &'static str {
        self.kind.label()
    }

    /// The panel's commit/cancel routes: the footer's OK/Cancel are
    /// panel-entity callbacks that reach the shell's pipelines through the
    /// seam (ticket 41's ruling 4).
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.cancel_filter_panel(cx))
            .ok();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| app.confirm_filter_panel(cx))
            .ok();
    }
}

impl Render for FilterPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let app = self.app.clone();
        let kind = self.kind;
        let sliders = self.sliders.clone();
        let mut body = gpui_kit::div().flex().flex_col().gap_1();
        match kind {
            FilterKind::Curves => {
                // The curve editor (CurvesControls.swift): channel picker, the
                // curve canvas with its click/drag gesture, a selected-point
                // readout with Remove, and Reset. The preview runs through the
                // same filter pipeline as the slider sheets.
                if self.curves.is_none() {
                    self.curves = Some(CurvesEditor::new());
                }
                let (curves, selected) = {
                    let editor = self.curves.as_ref().unwrap();
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
                                    compositor_core::adjust::LevelsChannel::ALL
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
                                // The dropdown keeps the app-shell dispatch
                                // (ticket 40's transitional seam).
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
                                .with_on_drag_move(panel_on_drag_move(
                                    &this,
                                    |panel, event, _window, cx| {
                                        panel.curves_mouse_move(event, cx);
                                    },
                                ))
                                .with_on_drag_up(panel_on_drag_up(
                                    &this,
                                    |panel, event, _window, cx| {
                                        panel.curves_mouse_up(event, cx);
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
                let gaussian = self.gaussian;
                let monochromatic = self.monochromatic;
                body = body
                    .child(SheetRow::new("Amount").with_optional_control(
                        sliders.first().map(|slider| Slider::new(slider).flex_1()),
                    ))
                    .child(
                        SheetRow::new("Distribution").with_control(
                            LabeledCheckbox::new("noise-gaussian", "Gaussian")
                                .checked(gaussian)
                                .with_on_change(panel_on(&this, |panel, checked, _, cx| {
                                    panel.gaussian = checked;
                                    panel.slider_changed(0.0, cx);
                                })),
                        ),
                    )
                    .child(
                        SheetRow::new("").with_control(
                            LabeledCheckbox::new("noise-mono", "Monochromatic")
                                .checked(monochromatic)
                                .with_on_change(panel_on(&this, |panel, checked, _, cx| {
                                    panel.monochromatic = checked;
                                    panel.slider_changed(0.0, cx);
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
        let footer = panel_footer(
            panel_names::FILTER_PANEL,
            &this,
            false,
            |panel, _, cx| panel.cancel(cx),
            |panel, _, cx| panel.confirm(cx),
        );
        panel_body_frame(body.into_any_element(), footer)
    }
}

impl AppState {
    /// `session.filterEdit != nil` → filterPanel (`ContentView.swift:236-246`):
    /// title = the filter's label; Camera Raw opens
    /// `DockedToMainWindowRight`, every other filter `Automatic`.
    pub fn open_filter_panel(&mut self, kind: FilterKind, cx: &mut Context<Self>) {
        self.begin_preview(cx);
        let session = self.tab().session.clone();
        let app = cx.entity().downgrade();
        let panel = cx.new(|cx| FilterPanelView::new(session, app, kind, cx));
        self.panel_views.insert(
            panel_names::FILTER_PANEL,
            crate::ui::floating_panel::PanelContent::Filter(panel),
        );
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
    /// restores the opening settings; the preview is dropped. The panel
    /// entity — the curves editor and the session's noise seed with it —
    /// dies here.
    pub fn cancel_filter_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panels.close(panel_names::FILTER_PANEL) {
            return;
        }
        if self.tab().session.read(cx).adjustment_editing_id.is_some() {
            self.tab()
                .session
                .update(cx, |session, _| session.finish_adjustment_editing(false));
        }
        self.panel_views.remove(panel_names::FILTER_PANEL);
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
                .filter_view()
                .map(|panel| panel.read(cx).kind_label().to_string())
                .unwrap_or("Filter".to_string());
            self.commit_preview_pixels(&name, cx);
        }
        self.panel_views.remove(panel_names::FILTER_PANEL);
        self.panels.close(panel_names::FILTER_PANEL);
        self.after_panel_editing_closed(cx);
    }
}
