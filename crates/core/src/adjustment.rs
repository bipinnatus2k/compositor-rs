//! Port of `Compositor/Document/LayerAdjustment.swift`: adjustment layers.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::geometry::CGRect;
use crate::raster::Bitmap;

pub use crate::adjust::camera_raw::CameraRawSettings;
pub use crate::adjust::curves::{CurvePoint, CurvesSettings};
pub use crate::adjust::hue_saturation::HueSaturationSettings;
pub use crate::adjust::image_adjustments::{
    AdjustmentColor, BlackWhiteSettings, ColorBalanceSettings, ExposureSettings,
    GradientMapSettings, GrainSettings,
};
pub use crate::adjust::levels::{LevelRange, LevelsSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdjustmentKind {
    #[serde(rename = "Hue/Saturation")]
    Hsv,
    #[serde(rename = "Levels")]
    Levels,
    #[serde(rename = "Curves")]
    Curves,
    #[serde(rename = "Exposure")]
    Exposure,
    #[serde(rename = "Gradient Map")]
    GradientMap,
    #[serde(rename = "Grain")]
    Grain,
    #[serde(rename = "Add Noise")]
    AddNoise,
    #[serde(rename = "Gaussian Blur")]
    GaussianBlur,
    #[serde(rename = "Motion Blur")]
    MotionBlur,
    #[serde(rename = "Invert")]
    Invert,
    #[serde(rename = "Black & White")]
    BlackWhite,
    #[serde(rename = "Color Balance")]
    ColorBalance,
}

impl AdjustmentKind {
    pub const ALL: [AdjustmentKind; 12] = [
        AdjustmentKind::Hsv,
        AdjustmentKind::Levels,
        AdjustmentKind::Curves,
        AdjustmentKind::Exposure,
        AdjustmentKind::GradientMap,
        AdjustmentKind::Grain,
        AdjustmentKind::AddNoise,
        AdjustmentKind::GaussianBlur,
        AdjustmentKind::MotionBlur,
        AdjustmentKind::Invert,
        AdjustmentKind::BlackWhite,
        AdjustmentKind::ColorBalance,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AdjustmentKind::Hsv => "Hue/Saturation",
            AdjustmentKind::Levels => "Levels",
            AdjustmentKind::Curves => "Curves",
            AdjustmentKind::Exposure => "Exposure",
            AdjustmentKind::GradientMap => "Gradient Map",
            AdjustmentKind::Grain => "Grain",
            AdjustmentKind::AddNoise => "Add Noise",
            AdjustmentKind::GaussianBlur => "Gaussian Blur",
            AdjustmentKind::MotionBlur => "Motion Blur",
            AdjustmentKind::Invert => "Invert",
            AdjustmentKind::BlackWhite => "Black & White",
            AdjustmentKind::ColorBalance => "Color Balance",
        }
    }

    /// Every kind but Invert opens an editor when its layer is double-clicked.
    pub fn is_editable(&self) -> bool {
        *self != AdjustmentKind::Invert
    }

    /// The filter panel that edits this kind; Levels and Hue/Saturation have
    /// panels of their own.
    pub fn filter_kind(&self) -> Option<crate::filters::FilterKind> {
        use crate::filters::FilterKind;
        match self {
            AdjustmentKind::Curves => Some(FilterKind::Curves),
            AdjustmentKind::BlackWhite => Some(FilterKind::BlackWhite),
            AdjustmentKind::ColorBalance => Some(FilterKind::ColorBalance),
            AdjustmentKind::Exposure => Some(FilterKind::Exposure),
            AdjustmentKind::GradientMap => Some(FilterKind::GradientMap),
            AdjustmentKind::Grain => Some(FilterKind::Grain),
            AdjustmentKind::GaussianBlur => Some(FilterKind::GaussianBlur),
            AdjustmentKind::MotionBlur => Some(FilterKind::MotionBlur),
            AdjustmentKind::AddNoise => Some(FilterKind::AddNoise),
            AdjustmentKind::Hsv | AdjustmentKind::Levels | AdjustmentKind::Invert => None,
        }
    }
}

/// An adjustment layer's settings (`LayerAdjustment`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerAdjustment {
    pub kind: AdjustmentKind,
    #[serde(default)]
    pub hue: f64,
    #[serde(default)]
    pub saturation: f64,
    #[serde(default)]
    pub lightness: f64,
    #[serde(default)]
    pub colorize: bool,
    /// Optional so projects saved before range-aware HSV still decode.
    #[serde(default)]
    pub hsv_settings: Option<HueSaturationSettings>,
    #[serde(default)]
    pub levels: LevelsSettings,
    #[serde(default)]
    pub curves: CurvesSettings,
    #[serde(default)]
    pub exposure_settings: Option<ExposureSettings>,
    #[serde(default)]
    pub gradient_map_settings: Option<GradientMapSettings>,
    #[serde(default)]
    pub grain_settings: Option<GrainSettings>,
    #[serde(default)]
    pub black_white_settings: Option<BlackWhiteSettings>,
    #[serde(default)]
    pub color_balance_settings: Option<ColorBalanceSettings>,
    #[serde(default)]
    pub blur_radius: Option<f64>,
    #[serde(default)]
    pub motion_angle: Option<f64>,
    #[serde(default)]
    pub motion_distance: Option<f64>,
    #[serde(default)]
    pub noise_amount: Option<f64>,
    #[serde(default)]
    pub noise_gaussian: Option<bool>,
    #[serde(default)]
    pub noise_monochromatic: Option<bool>,
    #[serde(default)]
    pub noise_seed: Option<u32>,
}

impl LayerAdjustment {
    pub fn new(kind: AdjustmentKind) -> Self {
        LayerAdjustment {
            kind,
            hue: 0.0,
            saturation: 0.0,
            lightness: 0.0,
            colorize: false,
            hsv_settings: None,
            levels: LevelsSettings::default(),
            curves: CurvesSettings::default(),
            exposure_settings: None,
            gradient_map_settings: None,
            grain_settings: None,
            black_white_settings: None,
            color_balance_settings: None,
            blur_radius: None,
            motion_angle: None,
            motion_distance: None,
            noise_amount: None,
            noise_gaussian: None,
            noise_monochromatic: None,
            noise_seed: None,
        }
    }

    pub fn resolved_hsv(&self) -> HueSaturationSettings {
        self.hsv_settings.clone().unwrap_or_else(|| {
            let mut s = HueSaturationSettings::default();
            s.colorize = self.colorize;
            s.set_hue(self.hue);
            s.set_saturation(self.saturation);
            s.set_lightness(self.lightness);
            s
        })
    }

    pub fn exposure(&self) -> ExposureSettings {
        self.exposure_settings.unwrap_or_default()
    }

    pub fn set_exposure(&mut self, value: ExposureSettings) {
        self.exposure_settings = Some(value);
    }

    pub fn gradient_map(&self) -> GradientMapSettings {
        self.gradient_map_settings.unwrap_or_default()
    }

    pub fn set_gradient_map(&mut self, value: GradientMapSettings) {
        self.gradient_map_settings = Some(value);
    }

    pub fn grain(&self) -> GrainSettings {
        self.grain_settings.unwrap_or_default()
    }

    pub fn set_grain(&mut self, value: GrainSettings) {
        self.grain_settings = Some(value);
    }

    pub fn black_white(&self) -> BlackWhiteSettings {
        self.black_white_settings.unwrap_or_default()
    }

    pub fn set_black_white(&mut self, value: BlackWhiteSettings) {
        self.black_white_settings = Some(value);
    }

    pub fn color_balance(&self) -> ColorBalanceSettings {
        self.color_balance_settings.unwrap_or_default()
    }

    pub fn set_color_balance(&mut self, value: ColorBalanceSettings) {
        self.color_balance_settings = Some(value);
    }

    pub fn gaussian_radius(&self) -> f64 {
        self.blur_radius.unwrap_or(10.0)
    }

    pub fn set_gaussian_radius(&mut self, value: f64) {
        self.blur_radius = Some(value);
    }

    pub fn resolved_motion_angle(&self) -> f64 {
        self.motion_angle.unwrap_or(0.0)
    }

    pub fn set_motion_angle(&mut self, value: f64) {
        self.motion_angle = Some(value);
    }

    pub fn resolved_motion_distance(&self) -> f64 {
        self.motion_distance.unwrap_or(10.0)
    }

    pub fn set_motion_distance(&mut self, value: f64) {
        self.motion_distance = Some(value);
    }

    pub fn resolved_noise_amount(&self) -> f64 {
        self.noise_amount.unwrap_or(10.0)
    }

    pub fn set_noise_amount(&mut self, value: f64) {
        self.noise_amount = Some(value);
    }

    pub fn resolved_noise_gaussian(&self) -> bool {
        self.noise_gaussian.unwrap_or(false)
    }

    pub fn resolved_noise_monochromatic(&self) -> bool {
        self.noise_monochromatic.unwrap_or(false)
    }

    pub fn resolved_noise_seed(&self) -> u32 {
        self.noise_seed.unwrap_or(0)
    }

    /// Document-pixel halo needed so a partial canvas redraw can sample
    /// beyond its dirty rectangle.
    pub fn sampling_margin(&self) -> f64 {
        match self.kind {
            AdjustmentKind::GaussianBlur => self.gaussian_radius() * 3.0 + 2.0,
            AdjustmentKind::MotionBlur => self.resolved_motion_distance() / 2.0 + 2.0,
            _ => 0.0,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.hue.is_finite()
            && self.saturation.is_finite()
            && self.lightness.is_finite()
            && self.hue.abs() <= 360.0
            && self.saturation.abs() <= 100.0
            && self.lightness.abs() <= 100.0
            // The per-range subdivision settings (`resolvedHSV`): each range's
            // sliders obey the same bounds as the master ones, and every band
            // handle must be a real number (LayerAdjustment.swift:129-136).
            && self.resolved_hsv().adjustments.values().all(|a| {
                a.hue.is_finite()
                    && a.hue.abs() <= 360.0
                    && a.saturation.is_finite()
                    && a.saturation.abs() <= 100.0
                    && a.lightness.is_finite()
                    && a.lightness.abs() <= 100.0
            })
            && self
                .resolved_hsv()
                .bands
                .values()
                .all(|b| b.handles().iter().all(|h| h.is_finite()))
            && self.levels.ranges.len() == 4
            && self.levels.ranges.iter().all(|r| *r == r.normalized())
            && self.curves.is_valid()
            && self.exposure().is_valid()
            && self.gradient_map().is_valid()
            && self.grain().is_valid()
            && self.black_white().is_valid()
            && self.color_balance().is_valid()
            && self.gaussian_radius().is_finite()
            && (0.1..=250.0).contains(&self.gaussian_radius())
            && self.resolved_motion_angle().is_finite()
            && (-90.0..=90.0).contains(&self.resolved_motion_angle())
            && self.resolved_motion_distance().is_finite()
            && (1.0..=2000.0).contains(&self.resolved_motion_distance())
            && self.resolved_noise_amount().is_finite()
            && (0.1..=400.0).contains(&self.resolved_noise_amount())
    }

    /// Applies the adjustment to an image (the whole layer at one unit per
    /// pixel when `region` is None).
    pub fn apply(&self, image: &Bitmap, region: Option<CGRect>) -> Bitmap {
        match self.kind {
            AdjustmentKind::Hsv => {
                let settings = self.resolved_hsv();
                let mut result = image.clone();
                crate::adjust::hue_saturation_apply(&mut result, &settings);
                result
            }
            AdjustmentKind::Levels => {
                let mut result = image.clone();
                crate::adjust::levels_apply_settings(&mut result, &self.levels);
                result
            }
            AdjustmentKind::Curves => self.curves.apply(image),
            AdjustmentKind::BlackWhite => self.black_white().apply(image),
            AdjustmentKind::ColorBalance => self.color_balance().apply(image),
            AdjustmentKind::Exposure => self.exposure().apply(image),
            AdjustmentKind::GradientMap => self.gradient_map().apply(image),
            AdjustmentKind::Grain => {
                let region = region.unwrap_or(CGRect::new(
                    0.0,
                    0.0,
                    image.width as f64,
                    image.height as f64,
                ));
                self.grain().apply(
                    image,
                    region.origin,
                    region.width() / (image.width).max(1) as f64,
                    None,
                )
            }
            AdjustmentKind::GaussianBlur => {
                crate::raster::gaussian_blur_rgba(image, self.gaussian_radius())
            }
            AdjustmentKind::MotionBlur => crate::filters::motion_blur(
                image,
                self.resolved_motion_distance(),
                self.resolved_motion_angle(),
            ),
            AdjustmentKind::AddNoise => {
                let mut result = image.clone();
                crate::filters::noise_add_at(
                    &mut result,
                    self.resolved_noise_amount() as f32,
                    self.resolved_noise_gaussian(),
                    self.resolved_noise_monochromatic(),
                    self.resolved_noise_seed(),
                    region.map(|r| r.min_x().floor() as i64).unwrap_or(0),
                    region.map(|r| r.min_y().floor() as i64).unwrap_or(0),
                );
                result
            }
            AdjustmentKind::Invert => crate::adjust::pixel::invert(
                image,
                &crate::geometry::CGAffineTransform::IDENTITY,
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        for kind in AdjustmentKind::ALL {
            let a = LayerAdjustment::new(kind);
            assert!(a.is_valid(), "{kind:?}");
        }
    }

    #[test]
    fn resolved_hsv_falls_back_to_scalars() {
        let mut a = LayerAdjustment::new(AdjustmentKind::Hsv);
        a.hue = 30.0;
        a.saturation = 10.0;
        let hsv = a.resolved_hsv();
        assert_eq!(hsv.hue(), 30.0);
        assert_eq!(hsv.saturation(), 10.0);
    }

    #[test]
    fn hsv_range_adjustments_are_validated() {
        // Each range's subdivision sliders obey the same bounds as the master
        // ones (LayerAdjustment.swift:129-133).
        let mut a = LayerAdjustment::new(AdjustmentKind::Hsv);
        assert!(a.is_valid());
        let mut settings = a.resolved_hsv();
        settings.adjustments.insert(
            crate::adjust::ColorRange::Reds,
            crate::adjust::RangeAdjustment {
                hue: 400.0,
                saturation: 0.0,
                lightness: 0.0,
            },
        );
        a.hsv_settings = Some(settings);
        assert!(!a.is_valid(), "a range hue past ±360 is invalid");
        let mut settings = a.resolved_hsv();
        settings.adjustments.insert(
            crate::adjust::ColorRange::Reds,
            crate::adjust::RangeAdjustment {
                hue: 0.0,
                saturation: 200.0,
                lightness: -150.0,
            },
        );
        a.hsv_settings = Some(settings);
        assert!(
            !a.is_valid(),
            "range saturation/lightness past ±100 are invalid"
        );
        // Back inside the bounds the settings are fine again.
        let mut settings = a.resolved_hsv();
        settings.adjustments.insert(
            crate::adjust::ColorRange::Reds,
            crate::adjust::RangeAdjustment {
                hue: 180.0,
                saturation: 100.0,
                lightness: -100.0,
            },
        );
        a.hsv_settings = Some(settings);
        assert!(a.is_valid());
        // A non-finite value is invalid whatever its size.
        let mut settings = a.resolved_hsv();
        settings.adjustments.insert(
            crate::adjust::ColorRange::Blues,
            crate::adjust::RangeAdjustment {
                hue: f64::NAN,
                saturation: 0.0,
                lightness: 0.0,
            },
        );
        a.hsv_settings = Some(settings);
        assert!(!a.is_valid());
    }

    #[test]
    fn hsv_band_handles_must_be_finite() {
        let mut a = LayerAdjustment::new(AdjustmentKind::Hsv);
        let mut settings = a.resolved_hsv();
        let band = crate::adjust::HueBand {
            falloff_start: f64::NAN,
            ..crate::adjust::ColorRange::Reds.default_band()
        };
        settings.bands.insert(crate::adjust::ColorRange::Reds, band);
        a.hsv_settings = Some(settings);
        assert!(!a.is_valid(), "a NaN band handle is invalid");
    }

    #[test]
    fn blur_radius_defaults_to_ten() {
        let a = LayerAdjustment::new(AdjustmentKind::GaussianBlur);
        assert_eq!(a.gaussian_radius(), 10.0);
        assert_eq!(a.sampling_margin(), 32.0);
    }

    #[test]
    fn sampling_margin_by_kind() {
        assert_eq!(
            LayerAdjustment::new(AdjustmentKind::Levels).sampling_margin(),
            0.0
        );
        let m = LayerAdjustment::new(AdjustmentKind::MotionBlur);
        assert_eq!(m.sampling_margin(), 7.0);
    }

    #[test]
    fn serde_roundtrip_preserves_optionals() {
        let mut a = LayerAdjustment::new(AdjustmentKind::Exposure);
        a.set_exposure(ExposureSettings {
            exposure: 1.5,
            offset: 0.0,
            gamma: 1.0,
        });
        let json = serde_json::to_string(&a).unwrap();
        let back: LayerAdjustment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.exposure().exposure, 1.5);
    }

    #[test]
    fn old_json_without_new_fields_decodes() {
        let json = r#"{"kind": "Invert"}"#;
        let a: LayerAdjustment = serde_json::from_str(json).unwrap();
        assert_eq!(a.kind, AdjustmentKind::Invert);
        assert!(a.is_valid());
    }

    #[test]
    fn filter_kinds_map() {
        assert_eq!(
            AdjustmentKind::Curves.filter_kind(),
            Some(crate::filters::FilterKind::Curves)
        );
        assert_eq!(AdjustmentKind::Levels.filter_kind(), None);
        assert_eq!(AdjustmentKind::Invert.filter_kind(), None);
        assert!(!AdjustmentKind::Invert.is_editable());
        assert!(AdjustmentKind::Levels.is_editable());
    }
}
