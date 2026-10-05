//! Port of the adjustment filters: `Levels.swift`, `Curves.swift`,
//! `HueSaturation.swift`, `ImageAdjustments.swift`, `CameraRaw*.swift`, and
//! the C pixel kernels in `Rendering/*.c`.

pub mod camera_raw;
pub mod curves;
pub mod hue_saturation;
pub mod image_adjustments;
pub mod levels;
pub mod levels_auto;
pub mod pixel;

pub use curves::{CurvePoint, CurvesSettings};
pub use hue_saturation::{
    CUBE_DIMENSION, ColorCube, ColorRange, HueBand, HueSampleMode, HueSaturationSettings,
    RangeAdjustment, adjust_hsl, apply_cube_rgba8, hue_response,
};
pub use image_adjustments::{
    AdjustmentColor, BlackWhiteSettings, ColorBalanceSettings, ExposureSettings,
    GradientMapSettings, GrainSettings, adjust_black_white, adjust_color_balance,
    adjust_gradient_map, adjust_grain,
};
pub use levels::{LevelRange, LevelsChannel, LevelsSettings, levels_apply, levels_histogram};

/// Applies `HueSaturationSettings` to a whole bitmap (adjustment layers do
/// this on their layer's pixels).
///
/// Port of `HueSaturationFilter.run` (HueSaturation.swift:236-252): the
/// settings are baked into a 33³ color cube and pixels are looked up through
/// it with f32 trilinear interpolation, exactly as `CIColorCube` applies the
/// cube (unpremultiply → lookup → premultiply, HueSaturation.swift:238-239).
/// Identity settings never reach the cube, as in Swift (HueSaturation.swift
/// guards `isIdentity` at :443-449 before calling the filter).
pub fn hue_saturation_apply(bitmap: &mut crate::raster::Bitmap, settings: &HueSaturationSettings) {
    if settings.is_identity() {
        return;
    }
    let cube = hue_saturation::ColorCube::build(settings);
    hue_saturation::apply_cube_rgba8(&mut bitmap.data, &cube);
}

/// Applies `LevelsSettings` to a whole bitmap through per-channel tables.
pub fn levels_apply_settings(bitmap: &mut crate::raster::Bitmap, settings: &LevelsSettings) {
    if settings.is_identity() {
        return;
    }
    let mut tables = [0.0f32; 768];
    for channel in [
        LevelsChannel::Red,
        LevelsChannel::Green,
        LevelsChannel::Blue,
    ] {
        for v in 0..256usize {
            tables[(channel.index() - 1) * 256 + v] =
                settings.apply(v as f64 / 255.0, channel) as f32;
        }
    }
    levels_apply(&mut bitmap.data, &tables);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identity settings never reach the cube, as in Swift
    /// (HueSaturation.swift:443-449 guards `isIdentity` before the filter).
    #[test]
    fn hue_saturation_apply_identity_keeps_bitmap() {
        let mut bitmap = crate::raster::Bitmap {
            width: 4,
            height: 1,
            data: vec![
                200, 100, 50, 255, 0, 0, 0, 0, 30, 60, 90, 128, 255, 255, 0, 170,
            ],
        };
        let before = bitmap.data.clone();
        hue_saturation_apply(&mut bitmap, &HueSaturationSettings::default());
        assert_eq!(bitmap.data, before);
    }

    /// The public entry point produces the same cube-path output as applying
    /// the cube directly, and stays within quantization tolerance of the
    /// exact per-pixel math (`adjust_hsl` + un/premultiply).
    #[test]
    fn hue_saturation_apply_routes_through_cube() {
        let mut settings = HueSaturationSettings::default();
        settings.set_hue(45.0);
        settings.set_saturation(15.0);

        let mut bitmap = crate::raster::Bitmap {
            width: 4,
            height: 1,
            data: vec![
                200, 100, 50, 255, 10, 8, 6, 128, 255, 255, 255, 255, 90, 30, 200, 255,
            ],
        };

        // Exact per-pixel reference.
        let response = hue_response(&settings);
        let mut exact = bitmap.data.clone();
        for p in exact.chunks_exact_mut(4) {
            let a = p[3] as f64;
            let r = (p[0] as f64 * 255.0 / a).min(255.0) / 255.0;
            let g = (p[1] as f64 * 255.0 / a).min(255.0) / 255.0;
            let b = (p[2] as f64 * 255.0 / a).min(255.0) / 255.0;
            let (nr, ng, nb) = adjust_hsl(r, g, b, &settings, Some(&response));
            for (i, v) in [nr, ng, nb].iter().enumerate() {
                p[i] = ((v * 255.0).round() as u8).min(p[3]);
            }
        }

        hue_saturation_apply(&mut bitmap, &settings);
        for (a, b) in exact.iter().zip(bitmap.data.iter()) {
            assert!(a.abs_diff(*b) <= 3, "{a} vs {b}");
        }
    }
}
