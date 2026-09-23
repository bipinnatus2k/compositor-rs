//! Port of the adjustment filters: `Levels.swift`, `Curves.swift`,
//! `HueSaturation.swift`, `ImageAdjustments.swift`, `CameraRaw*.swift`, and
//! the C pixel kernels in `Rendering/*.c`.

pub mod camera_raw;
pub mod curves;
pub mod hue_saturation;
pub mod image_adjustments;
pub mod levels;
pub mod pixel;

pub use curves::{CurvePoint, CurvesSettings};
pub use hue_saturation::{
    adjust_hsl, hue_response, ColorRange, HueBand, HueSampleMode, HueSaturationSettings,
    RangeAdjustment,
};
pub use image_adjustments::{
    adjust_black_white, adjust_color_balance, adjust_gradient_map, adjust_grain, AdjustmentColor,
    BlackWhiteSettings, ColorBalanceSettings, ExposureSettings, GradientMapSettings, GrainSettings,
};
pub use levels::{
    levels_apply, levels_histogram, LevelRange, LevelsChannel, LevelsSettings,
};

/// Applies `HueSaturationSettings` to a whole bitmap (adjustment layers do
/// this on their layer's pixels).
pub fn hue_saturation_apply(bitmap: &mut crate::raster::Bitmap, settings: &HueSaturationSettings) {
    if settings.is_identity() {
        return;
    }
    let response = hue_response(settings);
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f32;
        if alpha == 0.0 {
            continue;
        }
        // Unpremultiply, adjust, premultiply — CIColorCube does this itself.
        let a = p[3] as f64;
        let r = (p[0] as f64 * 255.0 / a).min(255.0) / 255.0;
        let g = (p[1] as f64 * 255.0 / a).min(255.0) / 255.0;
        let b = (p[2] as f64 * 255.0 / a).min(255.0) / 255.0;
        let (nr, ng, nb) = adjust_hsl(r, g, b, settings, Some(&response));
        for (i, v) in [nr, ng, nb].iter().enumerate() {
            p[i] = ((v * 255.0).round() as u8).min(p[3]);
        }
    }
}

/// Applies `LevelsSettings` to a whole bitmap through per-channel tables.
pub fn levels_apply_settings(bitmap: &mut crate::raster::Bitmap, settings: &LevelsSettings) {
    if settings.is_identity() {
        return;
    }
    let mut tables = [0.0f32; 768];
    for channel in [LevelsChannel::Red, LevelsChannel::Green, LevelsChannel::Blue] {
        for v in 0..256usize {
            tables[(channel.index() - 1) * 256 + v] =
                settings.apply(v as f64 / 255.0, channel) as f32;
        }
    }
    levels_apply(&mut bitmap.data, &tables);
}
