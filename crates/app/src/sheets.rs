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
use compositor_core::filters::{FilterKind, FilterSettings};

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
                sheet.sliders = vec![
                    self.make_slider(0.0, 254.0, 0.0, SliderRole::LevelsInputBlack, cx),
                    self.make_slider(0.1, 9.99, 1.0, SliderRole::LevelsGamma, cx),
                    self.make_slider(1.0, 255.0, 255.0, SliderRole::LevelsWhite, cx),
                    self.make_slider(0.0, 255.0, 0.0, SliderRole::LevelsOutBlack, cx),
                    self.make_slider(0.0, 255.0, 255.0, SliderRole::LevelsOutWhite, cx),
                ];
                if let Some(pixels) = self.preview_base.clone() {
                    compositor_core::adjust::levels_histogram(
                        pixels.as_ref(),
                        None,
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
            SheetKind::Filter(filter) => {
                self.begin_preview();
                let mut sheet = SheetState { ..base };
                match filter {
                    FilterKind::GaussianBlur => {
                        sheet.sliders = vec![self.make_slider(
                            0.1, 250.0, 4.0, SliderRole::Radius, cx,
                        )];
                    }
                    FilterKind::MotionBlur => {
                        sheet.sliders = vec![
                            self.make_slider(-90.0, 90.0, 0.0, SliderRole::Angle, cx),
                            self.make_slider(1.0, 2000.0, 20.0, SliderRole::Distance, cx),
                        ];
                    }
                    FilterKind::AddNoise => {
                        sheet.sliders = vec![self.make_slider(
                            0.1, 400.0, 12.0, SliderRole::Amount, cx,
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
                let settings = self.levels_settings_from_sheet(cx);
                if let Some(base) = self.preview_base.clone() {
                    let mut adjusted = base.as_ref().clone();
                    levels_apply_settings(&mut adjusted, &settings);
                    self.preview = Some(std::sync::Arc::new(adjusted));
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
                    self.preview = Some(std::sync::Arc::new(adjusted));
                }
            }
            SliderRole::CameraRaw(_) => {
                let settings = self.filter_settings_from_sheet(cx);
                self.apply_filter_preview(FilterKind::CameraRaw, settings);
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

    fn levels_settings_from_sheet(&self, cx: &gpui_kit::App) -> LevelsSettings {
        let mut settings = LevelsSettings::default();
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

    /// Levels Auto: maps the 0.5% and 99.5% histogram percentiles to the
    /// input black/white sliders.
    pub fn levels_auto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(histogram) = self.sheet.as_ref().map(|sheet| sheet.histogram) else {
            return;
        };
        let total: f64 = histogram.iter().sum();
        if total <= 0.0 {
            return;
        }
        let mut cumulative = 0.0;
        let mut low = 0usize;
        let mut high = 1023usize;
        for (index, value) in histogram.iter().enumerate() {
            cumulative += value;
            if cumulative >= total * 0.005 {
                low = index;
                break;
            }
        }
        cumulative = 0.0;
        for (index, value) in histogram.iter().enumerate() {
            cumulative += value;
            if cumulative >= total * 0.995 {
                high = index;
                break;
            }
        }
        let scale = 255.0 / 1023.0;
        let low_value = (low as f32 * scale).clamp(0.0, 254.0);
        let high_value = (high as f32 * scale).clamp(1.0, 255.0);
        for (index, value) in [(0usize, low_value), (2usize, high_value)] {
            if let Some(slider) = self.sheet.as_ref().and_then(|s| s.sliders.get(index)) {
                let slider = slider.clone();
                slider.update(cx, |state, cx| {
                    state.set_value(SliderValue::Single(value), window, cx)
                });
            }
        }
        self.apply_slider(SliderRole::LevelsInputBlack, low_value, cx);
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
}
