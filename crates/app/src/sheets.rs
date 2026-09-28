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
        if kind == SheetKind::ExportJpeg {
            self.refresh_jpeg_preview(cx);
        }
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

    /// Creates a panel slider with the given range and initial value. The
    /// subscription funnels every change into `apply_slider` with the role,
    /// so the write-back does not care which panel the slider lives in.
    pub(crate) fn make_slider(
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

    /// Builds the modal sheet's widget bundle. The tool dialogs that became
    /// floating panels (`panels_ui`) no longer build sheets here; what
    /// remains are the modal dialogs of the two-phase flows.
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
            width: self.make_input(&doc_w.to_string(), window, cx),
            height: self.make_input(&doc_h.to_string(), window, cx),
            path: self.make_input("export.png", window, cx),
            resolution: self.make_input("72", window, cx),
            tolerance: self.make_input("2", window, cx),
            percent: false,
            link: true,
            relative: false,
            anchor: 4,
            preset: 0,
            sliders: Vec::new(),
            based_on: 0,
            sides: [true, true, true, true],
            jpeg_preview: None,
            jpeg_ready: None,
            jpeg_data: None,
            jpeg_size_label: String::new(),
            message: String::new(),
            confirm_step: None,
        };
        match kind {
            SheetKind::ImageSize => {
                let resolution = self
                    .current_tab()
                    .session
                    .document
                    .as_ref()
                    .map(|d| d.resolution as usize)
                    .unwrap_or(72)
                    .to_string();
                SheetState {
                    resolution: self.make_input(&resolution, window, cx),
                    link: true,
                    ..base
                }
            }
            SheetKind::CanvasSize => base,
            SheetKind::ExportPng => SheetState {
                path: self.make_input("export.png", window, cx),
                ..base
            },
            SheetKind::ExportJpeg => SheetState {
                path: self.make_input("export.jpg", window, cx),
                // The quality of the last export, which the next one starts
                // from (JPEGExportSheet.init's UserDefaults read).
                sliders: vec![self.make_slider(
                    0.0,
                    1.0,
                    crate::settings::lock_global().number("jpegExportQuality", 0.85) as f32,
                    SliderRole::JpegQuality,
                    cx,
                )],
                // 0 = white matte, 1 = black.
                preset: 0,
                ..base
            },
            SheetKind::SaveComp => SheetState {
                path: self.make_input("./project.comp", window, cx),
                ..base
            },
            SheetKind::Trim | SheetKind::About | SheetKind::Confirm | SheetKind::Alert => base,
        }
    }

    /// Captures the active layer's pixels so adjustment panels can preview.
    pub(crate) fn begin_preview(&mut self) {
        self.preview_base = self
            .tab().session
            .active_layer()
            .and_then(|layer| layer.asset.as_ref().map(|asset| asset.image.clone()));
        self.preview_base_transform = self.tab().session.active_layer().map(|layer| layer.transform);
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

    /// Reads one of a panel bundle's slider values.
    pub(crate) fn panel_value(
        sliders: &[Entity<SliderState>],
        cx: &gpui_kit::App,
        index: usize,
    ) -> f32 {
        sliders
            .get(index)
            // Single-value sliders keep the live value in `end`; `start` is always 0.
            .map(|slider| slider.read(cx).value().end())
            .unwrap_or(0.0)
    }

    /// Writes a slider back to its setting and refreshes the preview. Panel
    /// sliders (`panels_ui`) route by role to their panel's widget bundle;
    /// the JPEG quality slider is the one modal sheet slider left.
    pub fn apply_slider(&mut self, role: SliderRole, _value: f32, cx: &mut Context<Self>) {
        match role {
            SliderRole::JpegQuality => {
                self.refresh_jpeg_preview(cx);
            }
            SliderRole::LevelsInputBlack
            | SliderRole::LevelsGamma
            | SliderRole::LevelsWhite
            | SliderRole::LevelsOutBlack
            | SliderRole::LevelsOutWhite => {
                let Some(panel) = self.panel_widgets.levels.as_ref() else {
                    return;
                };
                let preview = panel.preview;
                // Editing an adjustment layer: sliders feed the session's
                // Levels editor, which previews through the live composite.
                if self.tab().session.adjustment_editing_id.is_some() {
                    self.tab().session.levels = Some(self.levels_settings_from_panel(cx));
                    self.tab().session.preview_adjustment_editing(preview);
                    return;
                }
                if let Some(base) = self.preview_base.clone() {
                    if preview {
                        let settings = self.levels_settings_from_panel(cx);
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
                let Some(panel) = self.panel_widgets.hue_saturation.as_ref() else {
                    return;
                };
                let colorize = panel.colorize;
                let hue = Self::panel_value(&panel.sliders, cx, 0) as f64;
                let saturation = Self::panel_value(&panel.sliders, cx, 1) as f64;
                let lightness = Self::panel_value(&panel.sliders, cx, 2) as f64;
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
                // The adjustment-layer editor previews through the session.
                if self.tab().session.adjustment_editing_id.is_some() {
                    self.tab().session.hue_saturation = Some(settings);
                    self.tab().session.preview_adjustment_editing(true);
                    return;
                }
                if let Some(base) = self.preview_base.clone() {
                    let mut adjusted = base.as_ref().clone();
                    hue_saturation_apply(&mut adjusted, &settings);
                    adjusted = self.blend_adjusted(adjusted);
                    self.preview = Some(std::sync::Arc::new(adjusted));
                }
            }
            SliderRole::CameraRaw(_) => {
                let settings = self.filter_settings_from_panel(cx);
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
                let Some(panel) = self.panel_widgets.filter.as_ref() else {
                    return;
                };
                let kind = panel.kind;
                let settings = self.filter_settings_from_panel(cx);
                self.apply_filter_preview(kind, settings);
            }
        }
        self.revision += 1;
        cx.notify();
    }

    /// Rebuilds the Levels preview after the Preview checkbox flips: unchecked
    /// shows the untouched pixels, exactly as Swift's `edit.preview` does.
    pub fn refresh_levels_preview(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.panel_widgets.levels.as_ref() else {
            return;
        };
        let preview = panel.preview;
        if self.tab().session.adjustment_editing_id.is_some() {
            self.tab().session.levels = Some(self.levels_settings_from_panel(cx));
            self.tab().session.preview_adjustment_editing(preview);
            self.revision += 1;
            cx.notify();
            return;
        }
        let Some(base) = self.preview_base.clone() else {
            return;
        };
        if preview {
            let settings = self.levels_settings_from_panel(cx);
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

    /// The Levels panel's settings: the Auto modes set every channel's range
    /// at once; those keep their values while the sliders stay the source of
    /// truth for the visible channel's range.
    fn levels_settings_from_panel(&self, cx: &gpui_kit::App) -> LevelsSettings {
        let mut settings = self
            .panel_widgets
            .levels
            .as_ref()
            .map(|panel| panel.levels)
            .unwrap_or_default();
        settings.channel = match self.panel_widgets.levels.as_ref().map(|panel| panel.channel) {
            Some(1) => LevelsChannel::Red,
            Some(2) => LevelsChannel::Green,
            Some(3) => LevelsChannel::Blue,
            _ => LevelsChannel::Rgb,
        };
        let sliders = self
            .panel_widgets
            .levels
            .as_ref()
            .map(|panel| panel.sliders.clone())
            .unwrap_or_default();
        let mut range = settings.current();
        range.black = Self::panel_value(&sliders, cx, 0) as f64;
        range.gamma = Self::panel_value(&sliders, cx, 1) as f64;
        range.white = Self::panel_value(&sliders, cx, 2) as f64;
        range.output_black = Self::panel_value(&sliders, cx, 3) as f64;
        range.output_white = Self::panel_value(&sliders, cx, 4) as f64;
        settings.set_current(range);
        settings
    }

    /// The open filter panel's settings from its sliders and checkboxes.
    pub(crate) fn filter_settings_from_panel(&self, cx: &gpui_kit::App) -> FilterSettings {
        let mut settings = FilterSettings::default();
        let Some(panel) = self.panel_widgets.filter.as_ref() else {
            return settings;
        };
        match panel.kind {
            FilterKind::GaussianBlur => {
                settings.radius = Self::panel_value(&panel.sliders, cx, 0) as f64;
            }
            FilterKind::MotionBlur => {
                settings.angle = Self::panel_value(&panel.sliders, cx, 0) as f64;
                settings.distance = Self::panel_value(&panel.sliders, cx, 1) as f64;
            }
            FilterKind::AddNoise => {
                settings.amount = Self::panel_value(&panel.sliders, cx, 0) as f64;
                settings.gaussian = panel.gaussian;
                settings.monochromatic = panel.monochromatic;
            }
            FilterKind::LensCorrection => {
                settings.distortion = Self::panel_value(&panel.sliders, cx, 0) as f64;
            }
            FilterKind::CameraRaw => {
                let value = |i: usize| Self::panel_value(&panel.sliders, cx, i) as f64;
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

    /// The Levels panel's Auto button: Contrast, the shared-interval mode.
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
            let Some(panel) = self.panel_widgets.levels.as_ref() else {
                return;
            };
            (panel.histogram, panel.channel)
        };
        let settings = mode.settings(&histogram);
        let values = levels_auto_slider_values(&settings, channel);
        if let Some(panel) = self.panel_widgets.levels.as_mut() {
            panel.levels = settings;
        }
        for (index, value) in values.into_iter().enumerate() {
            if let Some(slider) = self
                .panel_widgets
                .levels
                .as_ref()
                .and_then(|panel| panel.sliders.get(index))
            {
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
        blend_through_selection(base, adjusted, self.preview_base_transform, self.current_tab().session.selection().cloned())
    }

    /// Re-runs the noise filters when their checkboxes change.
    pub fn refresh_filter_preview(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.panel_widgets.filter.as_ref() else {
            return;
        };
        let kind = panel.kind;
        let settings = self.filter_settings_from_panel(cx);
        self.apply_filter_preview(kind, settings);
        self.revision += 1;
        cx.notify();
    }

    // MARK: Effects panel

    /// The layers-panel effects menu: adds the effect (which selects and
    /// starts editing it) and opens the Effects panel for it. No panel when
    /// the add was refused (no editable pixel layer active).
    pub fn add_effect(
        &mut self,
        kind: LayerEffectKind,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab().session.add_effect(kind);
        if self.tab().session.effects_editing.map(|editing| editing.kind) == Some(kind) {
            self.open_effects_panel(cx);
        }
        self.touch(cx);
    }

    /// The Effects sheet's color swatch: opens the color picker on the
    /// effect's color (EffectsSheet.swift's `openEffectColorPicker`; guards
    /// and state live on the core session in `color_picker.rs`). Nothing is
    /// written into the layer until the picker closes.
    pub fn open_effect_color_picker(&mut self, kind: LayerEffectKind, cx: &mut Context<Self>) {
        self.tab().session.open_effect_color_picker(kind);
        self.touch(cx);
    }

    /// Pushes the Effects panel's sliders into the bound layer's effect — a
    /// real undo step per change through `changeEffects`, which is what makes
    /// the canvas preview live.
    pub fn apply_effects_sliders(&mut self, cx: &mut Context<Self>) {
        let (kind, values) = {
            let cx_app: &gpui_kit::App = cx;
            let Some(panel) = self.panel_widgets.effects.as_ref() else { return };
            let kind = LayerEffectKind::ALL
                .get(panel.kind_index)
                .copied()
                .unwrap_or(LayerEffectKind::Stroke);
            let values: Vec<f64> = (0..panel.sliders.len())
                .map(|index| Self::panel_value(&panel.sliders, cx_app, index) as f64)
                .collect();
            (kind, values)
        };
        let value = |index: usize| values.get(index).copied().unwrap_or(0.0);
        self.tab().session.change_effects(move |effects| match kind {
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

/// The placement transform `adjust::pixel::coverage` samples by: every pixel
/// center is mapped through it into the document before reading the
/// selection (`PixelAdjust.coverage`'s `concatenate(pixelToDocument.inverted())`,
/// Levels.swift:90-96).
fn coverage_mapping(
    transform: Option<LayerTransform>,
    width: usize,
    height: usize,
) -> CGAffineTransform {
    transform
        .map(|t| compositor_core::raster::pixel_to_document(&t, width as f64, height as f64))
        .unwrap_or(CGAffineTransform::IDENTITY)
}

/// `PixelAdjust.blend(over:through:)`: coverage × adjusted + (1 − coverage) ×
/// original, with the selection rasterized on the layer's pixel grid through
/// its placement. No selection keeps the adjustment whole.
fn blend_through_selection(
    base: &Bitmap,
    adjusted: Bitmap,
    transform: Option<LayerTransform>,
    selection: Option<compositor_core::selection::DocumentSelection>,
) -> Bitmap {
    let Some(selection) = selection else {
        return adjusted;
    };
    let mapping = coverage_mapping(transform, base.width, base.height);
    compositor_core::adjust::pixel::blend_path(&adjusted, base, &selection, &mapping)
}

/// The Levels histogram's per-pixel weights: the selection's coverage on the
/// layer's own pixel grid (`PixelAdjust.coverage(...)` in
/// `LevelsFilter.histogram`, Levels.swift:90-96).
pub(crate) fn histogram_coverage(
    transform: Option<LayerTransform>,
    selection: Option<compositor_core::selection::DocumentSelection>,
    width: usize,
    height: usize,
) -> Option<compositor_core::raster::GrayMask> {
    let selection = selection?;
    let mapping = coverage_mapping(transform, width, height);
    Some(compositor_core::adjust::pixel::coverage_path(
        &selection,
        width,
        height,
        &mapping,
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
    fn strip_selection() -> compositor_core::selection::DocumentSelection {
        DocumentSelection::new(SelectionPath::rect(CGRect::new(2.0, 0.0, 4.0, 1.0)))
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
        let blended = blend_through_selection(&base, adjusted, None, Some(strip_selection()));
        // Fully inside the selection the adjustment wins; outside — right up
        // against the boundary — the base.
        for x in 2..6 {
            assert_eq!(blended.pixel(x, 0), [255, 255, 255, 255], "column {x}");
        }
        for x in [0, 1, 6, 8] {
            assert_eq!(blended.pixel(x, 0), [0, 0, 0, 255], "column {x}");
        }
    }

    #[test]
    fn blend_through_selection_follows_the_layer_placement() {
        let base = solid(10, [0, 0, 0, 255]);
        let adjusted = solid(10, [255, 255, 255, 255]);
        // Document x 2.5…6.5 over a layer shifted two pixels right: the
        // layer's pixels sitting under the selection adjust, and each rim
        // pixel blends at exactly half coverage.
        let transform = LayerTransform::new(CGPoint::new(2.0, 0.0), CGSize::new(10.0, 1.0));
        let selection = DocumentSelection::new(SelectionPath::rect(CGRect::new(2.5, 0.0, 4.0, 1.0)));
        let blended = blend_through_selection(&base, adjusted, Some(transform), Some(selection));
        assert_eq!(blended.pixel(0, 0), [128, 128, 128, 255]);
        assert_eq!(blended.pixel(2, 0), [255, 255, 255, 255]);
        assert_eq!(blended.pixel(3, 0), [255, 255, 255, 255]);
        assert_eq!(blended.pixel(4, 0), [128, 128, 128, 255]);
        assert_eq!(blended.pixel(5, 0), [0, 0, 0, 255]);
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
        let coverage = histogram_coverage(None, Some(strip_selection()), 10, 1).unwrap();
        assert_eq!(coverage.value(3, 0), 255);
        assert_eq!(coverage.value(8, 0), 0);
        // A layer shifted two pixels right reads the selection from the grid
        // positions that actually sit under it.
        let transform = LayerTransform::new(CGPoint::new(2.0, 0.0), CGSize::new(10.0, 1.0));
        let shifted = histogram_coverage(Some(transform), Some(strip_selection()), 10, 1).unwrap();
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

