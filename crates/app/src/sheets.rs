//! Modal sheet plumbing: opening sheets, slider wiring and the live preview
//! that stands in for the active layer's pixels. The sheet UI itself lives in
//! `sheets_ui.rs`.

use gpui_kit::{AppContext, Context, Entity, Window};
use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::{SliderEvent, SliderState, SliderValue};

use compositor_core::adjust::{
    hue_saturation_apply, levels_apply_settings, ColorRange, LevelsChannel, LevelsSettings,
    RangeAdjustment,
};
use compositor_core::adjust::levels_auto::LevelsAuto;
use compositor_core::effects::LayerEffectKind;
use compositor_core::filters::{FilterKind, FilterSettings};
use compositor_core::geometry::CGAffineTransform;
use compositor_core::raster::Bitmap;
use compositor_core::selection::SelectionClip;
use compositor_core::transform::LayerTransform;

use crate::state::{AppState, SheetKind, SheetState};

/// A slider role: which setting a sheet slider writes back to.
#[derive(Clone, Copy, PartialEq)]
pub enum SliderRole {
    LevelsInputBlack,
    LevelsGamma,
    LevelsWhite,
    LevelsOutBlack,
    LevelsOutWhite,
    Hue,
    Saturation,
    Lightness,
    Radius,
    Angle,
    Distance,
    Amount,
    Distortion,
    CameraRaw(usize),
    JpegQuality,
    /// One Effects-sheet slider, by its position in the sheet's slider list.
    /// The edited `LayerEffectKind` rides in `SheetState.channel`, and the
    /// write-back reads every slider of the sheet at once, so the index only
    /// tags the slider as an effects one (EffectsSheet.swift's per-kind
    /// `changeEffects` closures).
    Effect(usize),
}

impl AppState {
    // MARK: Opening

    pub fn open_sheet(&mut self, kind: SheetKind, window: &mut Window, cx: &mut Context<Self>) {
        let sheet = self.build_sheet(kind, window, cx);
        self.sheet = Some(sheet);
        self.touch(cx);
    }

    fn make_input(
        &self,
        default: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| InputState::new(window, cx).default_value(default))
    }

    fn make_slider(
        &mut self,
        min: f32,
        max: f32,
        value: f32,
        role: SliderRole,
        cx: &mut Context<Self>,
    ) -> Entity<SliderState> {
        let state = cx.new(|_| {
            SliderState::new()
                .min(min)
                .max(max)
                .step(if max - min > 12.0 { 1.0 } else { 0.1 })
                .default_value(value)
        });
        let subscription = cx.subscribe(&state, move |this, _slider, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => value.start(),
            };
            this.apply_slider(role, value, cx);
        });
        self.subscriptions.push(subscription);
        state
    }

    fn build_sheet(
        &mut self,
        kind: SheetKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SheetState {
        let (doc_w, doc_h) = {
            let size = self.doc_size();
            (size.0 as usize, size.1 as usize)
        };
        let base = SheetState {
            kind,
            name: self.make_input("Untitled", window, cx),
            width: self.make_input(&doc_w.to_string(), window, cx),
            height: self.make_input(&doc_h.to_string(), window, cx),
            path: self.make_input("export.png", window, cx),
            resolution: self.make_input("72", window, cx),
            tolerance: self.make_input("2", window, cx),
            percent: false,
            link: true,
            relative: false,
            anchor: 4,
            background: 0,
            preset: 0,
            channel: 0,
            sliders: Vec::new(),
            checkbox_a: false,
            checkbox_b: false,
            colorize: false,
            based_on: 0,
            sides: [true, true, true, true],
            histogram: [0.0; 1024],
            levels: LevelsSettings::default(),
        };
        match kind {
            SheetKind::NewCanvas => SheetState {
                width: self.make_input("1920", window, cx),
                height: self.make_input("1080", window, cx),
                name: self.make_input("Untitled", window, cx),
                ..base
            },
            SheetKind::ImageSize => SheetState {
                resolution: self.make_input(
                    &self
                        .session
                        .document
                        .as_ref()
                        .map(|d| d.resolution as usize)
                        .unwrap_or(72)
                        .to_string(),
                    window,
                    cx,
                ),
                link: true,
                ..base
            },
            SheetKind::CanvasSize => base,
            SheetKind::Levels => {
                self.begin_preview();
                let mut sheet = SheetState { ..base };
                // Swift's LevelsEdit.preview (LevelsSheet's Preview toggle): on when opened.
                sheet.checkbox_a = true;
                sheet.sliders = vec![
                    self.make_slider(0.0, 254.0, 0.0, SliderRole::LevelsInputBlack, cx),
                    self.make_slider(0.1, 9.99, 1.0, SliderRole::LevelsGamma, cx),
                    self.make_slider(1.0, 255.0, 255.0, SliderRole::LevelsWhite, cx),
                    self.make_slider(0.0, 255.0, 0.0, SliderRole::LevelsOutBlack, cx),
                    self.make_slider(0.0, 255.0, 255.0, SliderRole::LevelsOutWhite, cx),
                ];
                if let Some(pixels) = self.preview_base.clone() {
                    // With a selection, bins are weighted by the selection's
                    // coverage on the layer's pixel grid (LevelsFilter.histogram,
                    // Levels.swift:90-96).
                    let coverage = histogram_coverage(
                        self.preview_base_transform,
                        self.selection_clip(),
                        pixels.width,
                        pixels.height,
                    );
                    compositor_core::adjust::levels_histogram(
                        pixels.as_ref(),
                        coverage.as_ref(),
                        &mut sheet.histogram,
                    );
                }
                sheet
            }
            SheetKind::HueSaturation => {
                self.begin_preview();
                let mut sheet = SheetState { ..base };
                sheet.sliders = vec![
                    self.make_slider(-180.0, 180.0, 0.0, SliderRole::Hue, cx),
                    self.make_slider(-100.0, 100.0, 0.0, SliderRole::Saturation, cx),
                    self.make_slider(-100.0, 100.0, 0.0, SliderRole::Lightness, cx),
                ];
                sheet
            }
            SheetKind::Effects => {
                // No preview capture: every slider change is committed to the
                // layer's effects right away (one undo step per change, as
                // Swift's changeEffects does), and the canvas renders effects
                // through core render.
                let editing = self.session.editing_effects();
                let kind = self
                    .session
                    .effects_editing
                    .map(|editing| editing.kind)
                    .unwrap_or(LayerEffectKind::Stroke);
                let mut sheet = SheetState { ..base };
                // `channel` stores which LayerEffectKind this sheet edits (an
                // index into `LayerEffectKind::ALL`); Levels only reads the
                // field while a Levels sheet is open, so it is free here.
                sheet.channel = LayerEffectKind::ALL
                    .iter()
                    .position(|candidate| *candidate == kind)
                    .unwrap_or(0);
                // Stroke's Outside/Inside position rides in `checkbox_a`.
                sheet.checkbox_a = editing.stroke.map(|stroke| stroke.inside).unwrap_or(false);
                // Slider thumb ranges are the EffectsSheet's visible ranges.
                // Swift's wider typed-input ranges (`inputRange`) have no
                // text field in this UI, and the core `isValid` guard still
                // bounds every stored value.
                let mut slider = |min: f64, max: f64, value: f64, index: usize| {
                    self.make_slider(min as f32, max as f32, value as f32, SliderRole::Effect(index), cx)
                };
                sheet.sliders = match kind {
                    LayerEffectKind::Stroke => vec![
                        slider(0.0, 20.0, editing.stroke.map(|s| s.size).unwrap_or(4.0), 0),
                        slider(0.0, 100.0, editing.stroke.map(|s| s.opacity * 100.0).unwrap_or(100.0), 1),
                    ],
                    LayerEffectKind::Shadow => vec![
                        slider(0.0, 100.0, editing.shadow.map(|s| s.opacity * 100.0).unwrap_or(50.0), 0),
                        slider(-180.0, 180.0, editing.shadow.map(|s| s.angle).unwrap_or(90.0), 1),
                        slider(0.0, 100.0, editing.shadow.map(|s| s.distance).unwrap_or(20.0), 2),
                        slider(0.0, 100.0, editing.shadow.map(|s| s.blur).unwrap_or(20.0), 3),
                    ],
                    LayerEffectKind::ColorOverlay => vec![slider(
                        0.0,
                        100.0,
                        editing.color_overlay.map(|s| s.opacity * 100.0).unwrap_or(100.0),
                        0,
                    )],
                    LayerEffectKind::InnerShadow => vec![
                        slider(0.0, 100.0, editing.inner_shadow.map(|s| s.opacity * 100.0).unwrap_or(50.0), 0),
                        slider(-180.0, 180.0, editing.inner_shadow.map(|s| s.angle).unwrap_or(90.0), 1),
                        slider(0.0, 50.0, editing.inner_shadow.map(|s| s.distance).unwrap_or(10.0), 2),
                        slider(0.0, 100.0, editing.inner_shadow.map(|s| s.blur).unwrap_or(10.0), 3),
                    ],
                    LayerEffectKind::OuterGlow => vec![
                        slider(0.0, 100.0, editing.outer_glow.map(|s| s.size).unwrap_or(20.0), 0),
                        slider(0.0, 100.0, editing.outer_glow.map(|s| s.opacity * 100.0).unwrap_or(75.0), 1),
                    ],
                    LayerEffectKind::InnerGlow => vec![
                        slider(0.0, 100.0, editing.inner_glow.map(|s| s.size).unwrap_or(10.0), 0),
                        slider(0.0, 100.0, editing.inner_glow.map(|s| s.opacity * 100.0).unwrap_or(75.0), 1),
                    ],
                };
                sheet
            }
            SheetKind::Filter(filter) => {
                self.begin_preview();
                let mut sheet = SheetState { ..base };
                let defaults = FilterSettings::default();
                match filter {
                    FilterKind::GaussianBlur => {
                        sheet.sliders = vec![self.make_slider(
                            0.1, 250.0, defaults.radius as f32, SliderRole::Radius, cx,
                        )];
                    }
                    FilterKind::MotionBlur => {
                        sheet.sliders = vec![
                            self.make_slider(-90.0, 90.0, defaults.angle as f32, SliderRole::Angle, cx),
                            self.make_slider(1.0, 2000.0, defaults.distance as f32, SliderRole::Distance, cx),
                        ];
                    }
                    FilterKind::AddNoise => {
                        sheet.sliders = vec![self.make_slider(
                            0.1, 400.0, defaults.amount as f32, SliderRole::Amount, cx,
                        )];
                    }
                    FilterKind::LensCorrection => {
                        sheet.sliders = vec![self.make_slider(
                            -100.0, 100.0, 0.0, SliderRole::Distortion, cx,
                        )];
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
                        sheet.sliders = group
                            .iter()
                            .enumerate()
                            .map(|(i, (min, max, value))| {
                                self.make_slider(*min as f32, *max as f32, *value as f32, SliderRole::CameraRaw(i), cx)
                            })
                            .collect();
                    }
                    _ => {}
                }
                sheet
            }
            SheetKind::Amount(_) => SheetState {
                tolerance: self.make_input("2", window, cx),
                ..base
            },
            SheetKind::ExportPng => SheetState {
                path: self.make_input("export.png", window, cx),
                ..base
            },
            SheetKind::ExportJpeg => SheetState {
                path: self.make_input("export.jpg", window, cx),
                sliders: vec![self.make_slider(0.0, 1.0, 0.85, SliderRole::JpegQuality, cx)],
                // 0 = white matte, 1 = black.
                preset: 0,
                ..base
            },
            SheetKind::Import => SheetState {
                path: self.make_input("./image.png", window, cx),
                ..base
            },
            SheetKind::SaveComp => SheetState {
                path: self.make_input("./project.comp", window, cx),
                ..base
            },
            SheetKind::Open => SheetState {
                path: self.make_input("./image.png", window, cx),
                ..base
            },
            SheetKind::Trim | SheetKind::About | SheetKind::Shortcuts => base,
        }
    }

    /// Captures the active layer's pixels so adjustment sheets can preview.
    fn begin_preview(&mut self) {
        self.preview_base = self
            .session
            .active_layer()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()));
        self.preview_base_transform = self.session.active_layer().map(|layer| layer.transform);
        self.preview = self.preview_base.clone();
    }

    /// Reads one of the current sheet's slider values.
    pub(crate) fn sheet_value(&self, cx: &gpui_kit::App, index: usize) -> f32 {
        self.sheet
            .as_ref()
            .and_then(|sheet| sheet.sliders.get(index))
            // Single-value sliders keep the live value in `end`; `start` is always 0.
            .map(|slider| slider.read(cx).value().end())
            .unwrap_or(0.0)
    }

    /// Writes a sheet slider back to its setting and refreshes the preview.
    pub fn apply_slider(&mut self, role: SliderRole, _value: f32, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet.as_ref() else { return };
        match role {
            SliderRole::JpegQuality => {} // read at confirm time
            SliderRole::LevelsInputBlack
            | SliderRole::LevelsGamma
            | SliderRole::LevelsWhite
            | SliderRole::LevelsOutBlack
            | SliderRole::LevelsOutWhite => {
                let preview = sheet.checkbox_a;
                if let Some(base) = self.preview_base.clone() {
                    if preview {
                        let settings = self.levels_settings_from_sheet(cx);
                        let mut adjusted = base.as_ref().clone();
                        levels_apply_settings(&mut adjusted, &settings);
                        adjusted = self.blend_adjusted(adjusted);
                        self.preview = Some(std::sync::Arc::new(adjusted));
                    } else {
                        self.preview = Some(base);
                    }
                }
            }
            SliderRole::Hue | SliderRole::Saturation | SliderRole::Lightness => {
                let colorize = sheet.colorize;
                let hue = self.sheet_value(cx, 0) as f64;
                let saturation = self.sheet_value(cx, 1) as f64;
                let lightness = self.sheet_value(cx, 2) as f64;
                let mut settings = compositor_core::adjust::HueSaturationSettings::new(
                    hue,
                    saturation,
                    lightness,
                    colorize,
                );
                if colorize {
                    settings.adjustments.insert(
                        ColorRange::Master,
                        RangeAdjustment {
                            hue,
                            // Colorize's saturation is an HSL amount 0…1
                            // (HueSaturation.swift:301); zero must stay zero.
                            saturation: (saturation / 100.0).clamp(0.0, 1.0),
                            lightness,
                        },
                    );
                }
                if let Some(base) = self.preview_base.clone() {
                    let mut adjusted = base.as_ref().clone();
                    hue_saturation_apply(&mut adjusted, &settings);
                    adjusted = self.blend_adjusted(adjusted);
                    self.preview = Some(std::sync::Arc::new(adjusted));
                }
            }
            SliderRole::CameraRaw(_) => {
                let settings = self.filter_settings_from_sheet(cx);
                self.apply_filter_preview(FilterKind::CameraRaw, settings);
            }
            SliderRole::Effect(_) => {
                self.apply_effects_sliders(cx);
            }
            SliderRole::Radius
            | SliderRole::Angle
            | SliderRole::Distance
            | SliderRole::Amount
            | SliderRole::Distortion => {
                let settings = self.filter_settings_from_sheet(cx);
                let kind = match sheet.kind {
                    SheetKind::Filter(kind) => kind,
                    _ => FilterKind::GaussianBlur,
                };
                self.apply_filter_preview(kind, settings);
            }
        }
        self.revision += 1;
        cx.notify();
    }

    /// Rebuilds the Levels preview after the Preview checkbox flips: unchecked
    /// shows the untouched pixels, exactly as Swift's `edit.preview` does.
    pub fn refresh_levels_preview(&mut self, cx: &mut Context<Self>) {
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        let preview = self.sheet.as_ref().map(|s| s.checkbox_a).unwrap_or(true);
        if preview {
            let settings = self.levels_settings_from_sheet(cx);
            let mut adjusted = base.as_ref().clone();
            levels_apply_settings(&mut adjusted, &settings);
            adjusted = self.blend_adjusted(adjusted);
            self.preview = Some(std::sync::Arc::new(adjusted));
        } else {
            self.preview = Some(base);
        }
        self.revision += 1;
        cx.notify();
    }

    fn levels_settings_from_sheet(&self, cx: &gpui_kit::App) -> LevelsSettings {
        // The Auto modes set every channel's range at once; those keep their
        // values while the sliders stay the source of truth for the visible
        // channel's range.
        let mut settings = self.sheet.as_ref().map(|sheet| sheet.levels).unwrap_or_default();
        settings.channel = match self.sheet.as_ref().map(|s| s.channel).unwrap_or(0) {
            1 => LevelsChannel::Red,
            2 => LevelsChannel::Green,
            3 => LevelsChannel::Blue,
            _ => LevelsChannel::Rgb,
        };
        let mut range = settings.current();
        range.black = self.sheet_value(cx, 0) as f64;
        range.gamma = self.sheet_value(cx, 1) as f64;
        range.white = self.sheet_value(cx, 2) as f64;
        range.output_black = self.sheet_value(cx, 3) as f64;
        range.output_white = self.sheet_value(cx, 4) as f64;
        settings.set_current(range);
        settings
    }

    pub(crate) fn filter_settings_from_sheet(&self, cx: &gpui_kit::App) -> FilterSettings {
        let mut settings = FilterSettings::default();
        let Some(sheet) = self.sheet.as_ref() else { return settings };
        match sheet.kind {
            SheetKind::Filter(FilterKind::GaussianBlur) => {
                settings.radius = self.sheet_value(cx, 0) as f64;
            }
            SheetKind::Filter(FilterKind::MotionBlur) => {
                settings.angle = self.sheet_value(cx, 0) as f64;
                settings.distance = self.sheet_value(cx, 1) as f64;
            }
            SheetKind::Filter(FilterKind::AddNoise) => {
                settings.amount = self.sheet_value(cx, 0) as f64;
                settings.gaussian = sheet.checkbox_a;
                settings.monochromatic = sheet.checkbox_b;
            }
            SheetKind::Filter(FilterKind::LensCorrection) => {
                settings.distortion = self.sheet_value(cx, 0) as f64;
            }
            SheetKind::Filter(FilterKind::CameraRaw) => {
                let value = |i: usize| self.sheet_value(cx, i) as f64;
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

    /// The Levels sheet's Auto button: Contrast, the shared-interval mode.
    pub fn levels_auto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.levels_auto_mode(LevelsAuto::Contrast, window, cx);
    }

    /// `autoLevels` (LevelsAutomatic.swift): the mode's settings replace the
    /// edit's wholesale, then the visible sliders follow the current channel's
    /// range and the preview refreshes, exactly as manual slider edits do.
    pub fn levels_auto_mode(
        &mut self,
        mode: LevelsAuto,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (histogram, channel) = {
            let Some(sheet) = self.sheet.as_ref() else { return };
            (sheet.histogram, sheet.channel)
        };
        let settings = mode.settings(&histogram);
        let values = levels_auto_slider_values(&settings, channel);
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.levels = settings;
        }
        for (index, value) in values.into_iter().enumerate() {
            if let Some(slider) = self.sheet.as_ref().and_then(|s| s.sliders.get(index)) {
                let slider = slider.clone();
                slider.update(cx, |state, cx| {
                    state.set_value(SliderValue::Single(value), window, cx)
                });
            }
        }
        self.apply_slider(SliderRole::LevelsInputBlack, values[0], cx);
    }

    /// Blends an adjustment result back over the untouched base through the
    /// selection, as `LevelsFilter.run`/`HueSaturationFilter.run` do before
    /// the pixels reach the layer (Levels.swift:80-82, HueSaturation.swift:
    /// 244-250). Without a selection the adjustment stands alone.
    fn blend_adjusted(&self, adjusted: Bitmap) -> Bitmap {
        let Some(base) = self.preview_base.as_deref() else {
            return adjusted;
        };
        blend_through_selection(base, adjusted, self.preview_base_transform, self.selection_clip())
    }

    /// Re-runs the noise filters when their checkboxes change.
    pub fn refresh_filter_preview(&mut self, cx: &mut Context<Self>) {
        let (settings, kind) = {
            let Some(sheet) = self.sheet.as_ref() else { return };
            (self.filter_settings_from_sheet(cx), match sheet.kind {
                SheetKind::Filter(kind) => kind,
                _ => return,
            })
        };
        self.apply_filter_preview(kind, settings);
        self.revision += 1;
        cx.notify();
    }

    // MARK: Effects sheet

    /// The layers-panel effects menu: adds the effect (which selects and
    /// starts editing it) and opens the Effects sheet for it. No sheet when
    /// the add was refused (no editable pixel layer active).
    pub fn add_effect(
        &mut self,
        kind: LayerEffectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session.add_effect(kind);
        if self.session.effects_editing.map(|editing| editing.kind) == Some(kind) {
            self.open_sheet(SheetKind::Effects, window, cx);
        }
        self.touch(cx);
    }

    /// The Effects sheet's color swatch: opens the color picker on the
    /// effect's color (EffectsSheet.swift's `openEffectColorPicker`; guards
    /// and state live on the core session in `color_picker.rs`). Nothing is
    /// written into the layer until the picker closes.
    pub fn open_effect_color_picker(&mut self, kind: LayerEffectKind, cx: &mut Context<Self>) {
        self.session.open_effect_color_picker(kind);
        self.touch(cx);
    }

    /// Pushes the Effects sheet's sliders into the bound layer's effect — a
    /// real undo step per change through `changeEffects`, which is what makes
    /// the canvas preview live.
    pub fn apply_effects_sliders(&mut self, cx: &mut Context<Self>) {
        let (kind, values) = {
            let cx_app: &gpui_kit::App = cx;
            let Some(sheet) = self.sheet.as_ref() else { return };
            let kind = LayerEffectKind::ALL
                .get(sheet.channel)
                .copied()
                .unwrap_or(LayerEffectKind::Stroke);
            let values: Vec<f64> = (0..sheet.sliders.len())
                .map(|index| self.sheet_value(cx_app, index) as f64)
                .collect();
            (kind, values)
        };
        let value = |index: usize| values.get(index).copied().unwrap_or(0.0);
        self.session.change_effects(move |effects| match kind {
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
        self.touch(cx);
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

/// The placement transform `adjust::pixel::coverage` samples by: it maps each
/// pixel center through `mapping⁻¹` before reading the selection, so the
/// `pixel_to_document` placement has to be handed to it inverted — then every
/// pixel is sampled at its own document position (`PixelAdjust.coverage`,
/// Levels.swift:90-96).
fn coverage_mapping(
    transform: Option<LayerTransform>,
    width: usize,
    height: usize,
) -> CGAffineTransform {
    let mapping = transform
        .map(|t| compositor_core::raster::pixel_to_document(&t, width as f64, height as f64))
        .unwrap_or(CGAffineTransform::IDENTITY);
    mapping.inverted()
}

/// `PixelAdjust.blend(over:through:)`: coverage × adjusted + (1 − coverage) ×
/// original, with the selection rasterized on the layer's pixel grid through
/// its placement. No selection keeps the adjustment whole.
fn blend_through_selection(
    base: &Bitmap,
    adjusted: Bitmap,
    transform: Option<LayerTransform>,
    clip: Option<SelectionClip>,
) -> Bitmap {
    let Some(clip) = clip else {
        return adjusted;
    };
    let mapping = coverage_mapping(transform, base.width, base.height);
    compositor_core::adjust::pixel::blend(&adjusted, base, &clip, &mapping)
}

/// The Levels histogram's per-pixel weights: the selection's coverage on the
/// layer's own pixel grid (`PixelAdjust.coverage(...)` in
/// `LevelsFilter.histogram`, Levels.swift:90-96).
fn histogram_coverage(
    transform: Option<LayerTransform>,
    clip: Option<SelectionClip>,
    width: usize,
    height: usize,
) -> Option<compositor_core::raster::GrayMask> {
    let clip = clip?;
    let mapping = coverage_mapping(transform, width, height);
    Some(compositor_core::adjust::pixel::coverage(
        &clip, width, height, &mapping,
    ))
}

#[cfg(test)]
mod tests {
    use super::{blend_through_selection, histogram_coverage, levels_auto_slider_values};
    use compositor_core::adjust::levels_auto::LevelsAuto;
    use compositor_core::filters::FilterSettings;
    use compositor_core::geometry::{CGPoint, CGRect, CGSize};
    use compositor_core::raster::Bitmap;
    use compositor_core::selection::{DocumentSelection, SelectionPath};
    use compositor_core::transform::LayerTransform;

    /// A selection covering document x 2…6 in a 10×1 canvas.
    fn strip_clip() -> compositor_core::selection::SelectionClip {
        let selection = DocumentSelection::new(SelectionPath::rect(CGRect::new(2.0, 0.0, 4.0, 1.0)));
        selection.clip(CGSize::new(10.0, 1.0))
    }

    fn solid(width: usize, value: [u8; 4]) -> Bitmap {
        let mut bitmap = Bitmap::new(width, 1);
        for x in 0..width {
            bitmap.set_pixel(x, 0, value);
        }
        bitmap
    }

    #[test]
    fn blend_through_selection_limits_the_adjustment() {
        let base = solid(10, [0, 0, 0, 255]);
        let adjusted = solid(10, [255, 255, 255, 255]);
        let blended = blend_through_selection(&base, adjusted, None, Some(strip_clip()));
        // Fully inside the selection the adjustment wins; outside, the base.
        assert_eq!(blended.pixel(3, 0), [255, 255, 255, 255]);
        assert_eq!(blended.pixel(8, 0), [0, 0, 0, 255]);
        assert_eq!(blended.pixel(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn blend_through_selection_follows_the_layer_placement() {
        let base = solid(10, [0, 0, 0, 255]);
        let adjusted = solid(10, [255, 255, 255, 255]);
        // The same selection over a layer shifted two pixels right: the
        // layer's pixels sitting under document x 2…6 are the ones adjusted,
        // with the selection's soft edge falling at the rim.
        let transform = LayerTransform::new(CGPoint::new(2.0, 0.0), CGSize::new(10.0, 1.0));
        let blended = blend_through_selection(&base, adjusted, Some(transform), Some(strip_clip()));
        assert_eq!(blended.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(blended.pixel(2, 0), [255, 255, 255, 255]);
        let rim = blended.pixel(3, 0)[0];
        assert!((100..=155).contains(&rim), "rim {rim}");
        assert_eq!(blended.pixel(4, 0), [0, 0, 0, 255]);
        assert_eq!(blended.pixel(8, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn blend_without_selection_keeps_the_adjustment_whole() {
        let base = solid(4, [0, 0, 0, 255]);
        let adjusted = solid(4, [255, 255, 255, 255]);
        let blended = blend_through_selection(&base, adjusted.clone(), None, None);
        for x in 0..4 {
            assert_eq!(blended.pixel(x, 0), [255, 255, 255, 255]);
        }
    }

    #[test]
    fn histogram_coverage_places_the_selection_on_the_layer_grid() {
        assert!(histogram_coverage(None, None, 10, 1).is_none());
        let coverage = histogram_coverage(None, Some(strip_clip()), 10, 1).unwrap();
        assert_eq!(coverage.value(3, 0), 255);
        assert_eq!(coverage.value(8, 0), 0);
        // A layer shifted two pixels right reads the selection from the grid
        // positions that actually sit under it.
        let transform = LayerTransform::new(CGPoint::new(2.0, 0.0), CGSize::new(10.0, 1.0));
        let shifted = histogram_coverage(Some(transform), Some(strip_clip()), 10, 1).unwrap();
        assert_eq!(shifted.value(1, 0), 255);
        assert_eq!(shifted.value(4, 0), 0);
        assert_eq!(shifted.value(6, 0), 0);
    }

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
        assert_eq!(levels_auto_slider_values(&contrast, 0), [20.0, 1.0, 230.0, 0.0, 255.0]);
        assert_eq!(levels_auto_slider_values(&contrast, 1), [0.0, 1.0, 255.0, 0.0, 255.0]);
        // Color's per-channel ranges show up when that channel is visible.
        let color = LevelsAuto::Color.settings(&bins);
        assert_eq!(levels_auto_slider_values(&color, 3), [60.0, 1.0, 230.0, 0.0, 255.0]);
    }

    #[test]
    fn filter_slider_defaults_match_filters_swift() {
        let defaults = FilterSettings::default();
        assert_eq!(defaults.radius, 1.0);
        assert_eq!(defaults.distance, 10.0);
        assert_eq!(defaults.amount, 10.0);
    }
}

