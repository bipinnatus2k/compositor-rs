//! Port of `Compositor/Document/CameraRaw*.swift` — the Camera Raw filter —
//! and of the `adjust_camera_raw*` pixel kernels in `Rendering/AdjustPixels.c`.

use serde::{Deserialize, Serialize};

use super::curves::{CurvePoint, CurvesSettings};
use super::levels::{LevelsHistogramDisplay, levels_histogram};
use crate::geometry::CGRect;
use crate::raster::{Bitmap, LayerSampling, bilinear_sample, draw_over};

/// White balance on an already-rendered layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawWhiteBalance {
    #[default]
    #[serde(rename = "Custom")]
    Custom,
    #[serde(rename = "Auto")]
    Auto,
}

/// Glow's three looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawGlowStyle {
    #[default]
    #[serde(rename = "Diffusion")]
    Diffusion,
    #[serde(rename = "Bloom")]
    Bloom,
    #[serde(rename = "Halation")]
    Halation,
}

impl CameraRawGlowStyle {
    pub fn kernel_value(&self) -> i32 {
        match self {
            CameraRawGlowStyle::Diffusion => 0,
            CameraRawGlowStyle::Bloom => 1,
            CameraRawGlowStyle::Halation => 2,
        }
    }
}

/// Post-crop vignette styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawVignetteStyle {
    #[default]
    #[serde(rename = "Highlight Priority")]
    HighlightPriority,
    #[serde(rename = "Color Priority")]
    ColorPriority,
    #[serde(rename = "Paint Overlay")]
    PaintOverlay,
}

impl CameraRawVignetteStyle {
    pub fn kernel_value(&self) -> i32 {
        match self {
            CameraRawVignetteStyle::HighlightPriority => 0,
            CameraRawVignetteStyle::ColorPriority => 1,
            CameraRawVignetteStyle::PaintOverlay => 2,
        }
    }
}

/// Temporary clipping view while Option is held on a Light slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraRawClipping {
    /// Clipped channels lit on black. Exposure, Highlights, and Whites.
    Highlights = 1,
    /// Clipped channels dark on white. Shadows and Blacks.
    Shadows = 2,
}

/// Parametric or point curve panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawCurvePage {
    #[default]
    #[serde(rename = "Parametric")]
    Parametric,
    #[serde(rename = "Point")]
    Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawPointChannel {
    #[default]
    #[serde(rename = "RGB")]
    Rgb,
    #[serde(rename = "Red")]
    Red,
    #[serde(rename = "Green")]
    Green,
    #[serde(rename = "Blue")]
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawMixerPage {
    #[default]
    #[serde(rename = "HSL")]
    Hsl,
    #[serde(rename = "Color")]
    Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawMixerTab {
    #[default]
    #[serde(rename = "Hue")]
    Hue,
    #[serde(rename = "Saturation")]
    Saturation,
    #[serde(rename = "Luminance")]
    Luminance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawGradePage {
    #[default]
    #[serde(rename = "Three-way")]
    ThreeWay,
    #[serde(rename = "Shadows")]
    Shadows,
    #[serde(rename = "Midtones")]
    Midtones,
    #[serde(rename = "Highlights")]
    Highlights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawScopeMode {
    #[default]
    #[serde(rename = "Histogram")]
    Histogram,
    #[serde(rename = "Vectorscope")]
    Vectorscope,
}

/// Upright is Off or Guided: guided reads drawn lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawUprightMode {
    #[default]
    #[serde(rename = "Off")]
    Off,
    #[serde(rename = "Guided")]
    Guided,
}

/// Perspective bends far lines more; Rectilinear is the gentler take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawProjection {
    #[default]
    #[serde(rename = "Perspective")]
    Perspective,
    #[serde(rename = "Rectilinear")]
    Rectilinear,
}

/// Calibration process versions, strongest last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CameraRawProcessVersion {
    #[serde(rename = "Version 1")]
    Version1,
    #[serde(rename = "Version 2")]
    Version2,
    #[serde(rename = "Version 3")]
    Version3,
    #[serde(rename = "Version 4")]
    Version4,
    #[serde(rename = "Version 5")]
    Version5,
    #[default]
    #[serde(rename = "Version 6")]
    Version6,
}

impl CameraRawProcessVersion {
    pub fn kernel_value(&self) -> i32 {
        match self {
            CameraRawProcessVersion::Version1 => 1,
            CameraRawProcessVersion::Version2 => 2,
            CameraRawProcessVersion::Version3 => 3,
            CameraRawProcessVersion::Version4 => 4,
            CameraRawProcessVersion::Version5 => 5,
            CameraRawProcessVersion::Version6 => 6,
        }
    }
}

/// A guide line in normalized image coordinates, 0…1 from the lower-left of
/// the pixel grid (`CameraRawGeometryGuide`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CameraRawGeometryGuide {
    #[serde(default)]
    pub start_x: f64,
    #[serde(default)]
    pub start_y: f64,
    #[serde(default)]
    pub end_x: f64,
    #[serde(default)]
    pub end_y: f64,
}

fn default_curve_points() -> Vec<CurvePoint> {
    vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }]
}

fn default_10() -> f64 {
    10.0
}
fn default_30() -> f64 {
    30.0
}
fn default_04() -> f64 {
    0.4
}
fn default_75() -> f64 {
    75.0
}
fn default_100() -> f64 {
    100.0
}
fn default_270() -> f64 {
    270.0
}
fn default_310() -> f64 {
    310.0
}
fn default_60() -> f64 {
    60.0
}
fn default_120() -> f64 {
    120.0
}

fn default_mixer_eight() -> [f64; 8] {
    [0.0; 8]
}

/// `ImageAdjustmentPixels.clamp`: non-finite values take the fallback.
fn clamp_slider(value: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.min(hi).max(lo)
    } else {
        fallback
    }
}

/// Parametric regions and point curves (`CameraRawCurveSettings`). Amounts are
/// −100…100. Curve points use 0…1 on both axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraRawCurveSettings {
    #[serde(default)]
    pub shadows: f64,
    #[serde(default)]
    pub darks: f64,
    #[serde(default)]
    pub lights: f64,
    #[serde(default)]
    pub highlights: f64,
    /// Dividers, 0…100, kept in order. They set where each parametric slider hands off to the next.
    #[serde(default = "default_25")]
    pub shadow_split: f64,
    #[serde(default = "default_50")]
    pub dark_split: f64,
    #[serde(default = "default_75")]
    pub light_split: f64,
    #[serde(default = "default_curve_points")]
    pub rgb: Vec<CurvePoint>,
    #[serde(default = "default_curve_points")]
    pub red: Vec<CurvePoint>,
    #[serde(default = "default_curve_points")]
    pub green: Vec<CurvePoint>,
    #[serde(default = "default_curve_points")]
    pub blue: Vec<CurvePoint>,
    /// How much the composite curve also changes saturation. 0 keeps it to brightness.
    #[serde(default)]
    pub refine_saturation: f64,
}

/// `CameraRawCurveSettings.linear`.
pub fn linear_curve() -> Vec<CurvePoint> {
    vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }]
}

impl Default for CameraRawCurveSettings {
    fn default() -> Self {
        CameraRawCurveSettings {
            shadows: 0.0,
            darks: 0.0,
            lights: 0.0,
            highlights: 0.0,
            shadow_split: 25.0,
            dark_split: 50.0,
            light_split: 75.0,
            rgb: linear_curve(),
            red: linear_curve(),
            green: linear_curve(),
            blue: linear_curve(),
            refine_saturation: 0.0,
        }
    }
}

impl CameraRawCurveSettings {
    /// `CameraRawCurveSettings.mediumContrast`.
    pub fn medium_contrast() -> Vec<CurvePoint> {
        vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.25, y: 0.18 },
            CurvePoint { x: 0.75, y: 0.82 },
            CurvePoint { x: 1.0, y: 1.0 },
        ]
    }

    /// `CameraRawCurveSettings.strongContrast`.
    pub fn strong_contrast() -> Vec<CurvePoint> {
        vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.25, y: 0.10 },
            CurvePoint { x: 0.75, y: 0.90 },
            CurvePoint { x: 1.0, y: 1.0 },
        ]
    }

    pub fn is_linear(points: &[CurvePoint]) -> bool {
        points.len() == 2
            && points[0].x == 0.0
            && points[0].y == 0.0
            && points[1].x == 1.0
            && points[1].y == 1.0
    }

    pub fn adjusts(&self) -> bool {
        self.shadows != 0.0
            || self.darks != 0.0
            || self.lights != 0.0
            || self.highlights != 0.0
            || self.refine_saturation != 0.0
            || !Self::is_linear(&self.rgb)
            || !Self::is_linear(&self.red)
            || !Self::is_linear(&self.green)
            || !Self::is_linear(&self.blue)
    }

    /// Lifts or lowers the region a tone belongs to. Dividers are fractions of the tonal range.
    // `.min().max()` keeps the Swift original's order; clamp() differs on NaN.
    #[allow(clippy::manual_clamp)]
    pub fn parametric(&self, tone: f64) -> f64 {
        let shadow = self.shadow_split / 100.0;
        let dark = self.dark_split / 100.0;
        let light = self.light_split / 100.0;
        let (amount, lo, hi): (f64, f64, f64) = if tone < shadow {
            (self.shadows, 0.0, shadow)
        } else if tone < dark {
            (self.darks, shadow, dark)
        } else if tone < light {
            (self.lights, dark, light)
        } else {
            (self.highlights, light, 1.0)
        };
        let span = (hi - lo).max(0.02);
        let weight = 1.0 - (tone - (lo + hi) / 2.0).abs() / (span / 2.0);
        (tone + (amount / 100.0) * weight.max(0.0) * 0.22)
            .min(1.0)
            .max(0.0)
    }

    /// The region name a tone belongs to, as an index: 0 shadows, 1 darks,
    /// 2 lights, 3 highlights (`region(for:)`).
    pub fn region_index(&self, tone: f64) -> usize {
        if tone < self.shadow_split / 100.0 {
            0
        } else if tone < self.dark_split / 100.0 {
            1
        } else if tone < self.light_split / 100.0 {
            2
        } else {
            3
        }
    }

    fn point(&self, x: f64, points: &[CurvePoint]) -> f64 {
        if points.len() < 2 {
            return x;
        }
        let mut curve = CurvesSettings::default();
        curve.channels[0] = points
            .iter()
            .map(|p| CurvePoint {
                x: p.x * 255.0,
                y: p.y * 255.0,
            })
            .collect();
        curve.value(x * 255.0, 0) / 255.0
    }

    pub fn luma_table(&self) -> [f32; 256] {
        let mut table = [0.0f32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = self.point(self.parametric(i as f64 / 255.0), &self.rgb) as f32;
        }
        table
    }

    pub fn channel_table(&self, points: &[CurvePoint]) -> [f32; 256] {
        let mut table = [0.0f32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = self.point(i as f64 / 255.0, points) as f32;
        }
        table
    }

    /// Moves the nearest point of one channel vertically (`nudged(near:by:)`).
    // `.min().max()` keeps the Swift original's order; clamp() differs on NaN.
    #[allow(clippy::manual_clamp)]
    pub fn nudged(&self, channel: CameraRawPointChannel, near_tone: f64, delta: f64) -> Self {
        let mut result = self.clone();
        let source = match channel {
            CameraRawPointChannel::Rgb => &self.rgb,
            CameraRawPointChannel::Red => &self.red,
            CameraRawPointChannel::Green => &self.green,
            CameraRawPointChannel::Blue => &self.blue,
        };
        let Some(index) = source
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (source[a.0].x - near_tone)
                    .abs()
                    .total_cmp(&(source[b.0].x - near_tone).abs())
            })
            .map(|(i, _)| i)
        else {
            return self.clone();
        };
        let mut points = source.clone();
        points[index].y = (points[index].y + delta).min(1.0).max(0.0);
        match channel {
            CameraRawPointChannel::Rgb => result.rgb = points,
            CameraRawPointChannel::Red => result.red = points,
            CameraRawPointChannel::Green => result.green = points,
            CameraRawPointChannel::Blue => result.blue = points,
        }
        result
    }

    pub fn normalized(&self) -> Self {
        let mut result = self.clone();
        result.shadows = clamp_slider(self.shadows, -100.0, 100.0, 0.0);
        result.darks = clamp_slider(self.darks, -100.0, 100.0, 0.0);
        result.lights = clamp_slider(self.lights, -100.0, 100.0, 0.0);
        result.highlights = clamp_slider(self.highlights, -100.0, 100.0, 0.0);
        result.refine_saturation = clamp_slider(self.refine_saturation, -100.0, 100.0, 0.0);
        result.shadow_split = clamp_slider(self.shadow_split, 5.0, 90.0, 25.0);
        result.dark_split = clamp_slider(self.dark_split, result.shadow_split + 2.0, 95.0, 50.0);
        result.light_split = clamp_slider(self.light_split, result.dark_split + 2.0, 98.0, 75.0);
        result.rgb = Self::repair(&self.rgb);
        result.red = Self::repair(&self.red);
        result.green = Self::repair(&self.green);
        result.blue = Self::repair(&self.blue);
        result
    }

    // `.min().max()` keeps the Swift original's order; clamp() differs on NaN.
    #[allow(clippy::manual_clamp)]
    fn repair(points: &[CurvePoint]) -> Vec<CurvePoint> {
        let mut sorted: Vec<CurvePoint> = points
            .iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .cloned()
            .collect();
        sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
        if sorted.len() < 2 {
            return linear_curve();
        }
        let last = sorted.len() - 1;
        sorted[0].x = 0.0;
        sorted[0].y = sorted[0].y.min(1.0).max(0.0);
        sorted[last].x = 1.0;
        sorted[last].y = sorted[last].y.min(1.0).max(0.0);
        let mut kept: Vec<CurvePoint> = vec![sorted[0]];
        for point in &sorted[1..last] {
            let x = point.x.min(0.99).max(0.01);
            if x <= kept[kept.len() - 1].x + 0.01 {
                continue;
            }
            kept.push(CurvePoint {
                x,
                y: point.y.min(1.0).max(0.0),
            });
        }
        kept.push(sorted[last]);
        kept
    }
}

/// One picked color and how far its adjustment reaches (`CameraRawPointColor`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CameraRawPointColor {
    #[serde(default)]
    pub hue: f64,
    #[serde(default)]
    pub saturation: f64,
    #[serde(default)]
    pub luminance: f64,
    #[serde(default)]
    pub hue_shift: f64,
    #[serde(default)]
    pub saturation_shift: f64,
    #[serde(default)]
    pub luminance_shift: f64,
    #[serde(default = "default_30")]
    pub hue_range: f64,
    #[serde(default = "default_04")]
    pub saturation_range: f64,
    #[serde(default = "default_04")]
    pub luminance_range: f64,
    #[serde(default)]
    pub visualize: bool,
}

impl CameraRawPointColor {
    pub fn normalized(&self) -> Self {
        let mut result = *self;
        result.hue = clamp_slider(self.hue, 0.0, 360.0, 0.0);
        result.saturation = clamp_slider(self.saturation, 0.0, 1.0, 0.0);
        result.luminance = clamp_slider(self.luminance, 0.0, 1.0, 0.0);
        result.hue_shift = clamp_slider(self.hue_shift, -100.0, 100.0, 0.0);
        result.saturation_shift = clamp_slider(self.saturation_shift, -100.0, 100.0, 0.0);
        result.luminance_shift = clamp_slider(self.luminance_shift, -100.0, 100.0, 0.0);
        result.hue_range = clamp_slider(self.hue_range, 5.0, 180.0, 30.0);
        result.saturation_range = clamp_slider(self.saturation_range, 0.05, 1.0, 0.4);
        result.luminance_range = clamp_slider(self.luminance_range, 0.05, 1.0, 0.4);
        result
    }
}

/// Eight color families, each with hue, saturation, and luminance shifts of
/// −100…100 (`CameraRawMixerSettings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraRawMixerSettings {
    #[serde(default = "default_mixer_eight")]
    pub hue: [f64; 8],
    #[serde(default = "default_mixer_eight")]
    pub saturation: [f64; 8],
    #[serde(default = "default_mixer_eight")]
    pub luminance: [f64; 8],
    #[serde(default)]
    pub points: Vec<CameraRawPointColor>,
}

impl Default for CameraRawMixerSettings {
    fn default() -> Self {
        CameraRawMixerSettings {
            hue: [0.0; 8],
            saturation: [0.0; 8],
            luminance: [0.0; 8],
            points: Vec::new(),
        }
    }
}

impl CameraRawMixerSettings {
    /// `CameraRawMixerSettings.names`.
    pub const NAMES: [&'static str; 8] = [
        "Reds", "Oranges", "Yellows", "Greens", "Aquas", "Blues", "Purples", "Magentas",
    ];
    /// Family centers in degrees (`CameraRawMixerSettings.centers`).
    pub const CENTERS: [f64; 8] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0];

    pub fn adjusts(&self) -> bool {
        self.hue.iter().any(|v| *v != 0.0)
            || self.saturation.iter().any(|v| *v != 0.0)
            || self.luminance.iter().any(|v| *v != 0.0)
            || self.points.iter().any(|p| {
                p.hue_shift != 0.0 || p.saturation_shift != 0.0 || p.luminance_shift != 0.0
            })
    }

    /// How much each family shares a hue, in degrees. Neighbors overlap.
    pub fn weights_for_hue(degrees: f64) -> [f64; 8] {
        let mut result = [0.0; 8];
        for (i, center) in Self::CENTERS.iter().enumerate() {
            let mut distance = (degrees - center).abs();
            if distance > 180.0 {
                distance = 360.0 - distance;
            }
            result[i] = (1.0 - distance / 40.0).max(0.0);
        }
        result
    }

    /// The 24 floats `adjust_camera_raw_curve_color` reads as `mixer`.
    pub fn mixer_floats(&self) -> [f32; 24] {
        let mut out = [0.0f32; 24];
        for i in 0..8 {
            out[i] = (self.hue[i] / 100.0) as f32;
            out[8 + i] = (self.saturation[i] / 100.0) as f32;
            out[16 + i] = (self.luminance[i] / 100.0) as f32;
        }
        out
    }

    /// The 9 floats per point that `adjust_camera_raw_curve_color` reads as `points`.
    pub fn point_floats(&self) -> Vec<f32> {
        self.points
            .iter()
            .flat_map(|point| {
                [
                    point.hue / 360.0,
                    point.saturation,
                    point.luminance,
                    point.hue_shift / 100.0,
                    point.saturation_shift / 100.0,
                    point.luminance_shift / 100.0,
                    point.hue_range / 360.0,
                    point.saturation_range,
                    point.luminance_range,
                ]
                .map(|v| v as f32)
            })
            .collect()
    }

    pub fn normalized(&self) -> Self {
        let mut result = self.clone();
        result.hue = self.hue.map(|v| clamp_slider(v, -100.0, 100.0, 0.0));
        result.saturation = self.saturation.map(|v| clamp_slider(v, -100.0, 100.0, 0.0));
        result.luminance = self.luminance.map(|v| clamp_slider(v, -100.0, 100.0, 0.0));
        result.points = self.points.iter().take(8).map(|p| p.normalized()).collect();
        result
    }
}

/// One color-grading wheel (`CameraRawGradeWheel`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CameraRawGradeWheel {
    #[serde(default)]
    pub hue: f64,
    #[serde(default)]
    pub saturation: f64,
    #[serde(default)]
    pub luminance: f64,
}

impl CameraRawGradeWheel {
    pub fn normalized(&self) -> Self {
        CameraRawGradeWheel {
            hue: clamp_slider(self.hue, 0.0, 360.0, 0.0),
            saturation: clamp_slider(self.saturation, 0.0, 100.0, 0.0),
            luminance: clamp_slider(self.luminance, -100.0, 100.0, 0.0),
        }
    }
}

/// Four color wheels plus how the three tonal wheels overlap and which end
/// they favor (`CameraRawGradingSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraRawGradingSettings {
    #[serde(default)]
    pub shadows: CameraRawGradeWheel,
    #[serde(default)]
    pub midtones: CameraRawGradeWheel,
    #[serde(default)]
    pub highlights: CameraRawGradeWheel,
    #[serde(default)]
    pub global: CameraRawGradeWheel,
    /// 0…100. Higher values let the three tonal wheels overlap more.
    #[serde(default = "default_50")]
    pub blending: f64,
    /// −100…100. Negative favors shadows, positive favors highlights.
    #[serde(default)]
    pub balance: f64,
}

impl Default for CameraRawGradingSettings {
    fn default() -> Self {
        CameraRawGradingSettings {
            shadows: CameraRawGradeWheel::default(),
            midtones: CameraRawGradeWheel::default(),
            highlights: CameraRawGradeWheel::default(),
            global: CameraRawGradeWheel::default(),
            blending: 50.0,
            balance: 0.0,
        }
    }
}

impl CameraRawGradingSettings {
    pub fn wheels(&self) -> [CameraRawGradeWheel; 4] {
        [self.shadows, self.midtones, self.highlights, self.global]
    }

    pub fn adjusts(&self) -> bool {
        self.wheels()
            .iter()
            .any(|w| w.saturation != 0.0 || w.luminance != 0.0)
    }

    /// The 12 floats `adjust_camera_raw_curve_color` reads as `grade`.
    pub fn grade_floats(&self) -> [f32; 12] {
        let mut out = [0.0f32; 12];
        for (i, wheel) in self.wheels().iter().enumerate() {
            out[i * 3] = (wheel.hue / 360.0) as f32;
            out[i * 3 + 1] = (wheel.saturation / 100.0) as f32;
            out[i * 3 + 2] = (wheel.luminance / 100.0) as f32;
        }
        out
    }

    pub fn normalized(&self) -> Self {
        let mut result = *self;
        result.shadows = self.shadows.normalized();
        result.midtones = self.midtones.normalized();
        result.highlights = self.highlights.normalized();
        result.global = self.global.normalized();
        result.blending = clamp_slider(self.blending, 0.0, 100.0, 50.0);
        result.balance = clamp_slider(self.balance, -100.0, 100.0, 0.0);
        result
    }

    pub fn applying(&self, shows: bool) -> Self {
        if shows { *self } else { Self::default() }
    }
}

/// Sharpening and manual noise reduction (`CameraRawDetailSettings`). Amount is
/// 0…150; the rest use Camera Raw's usual 0…100 ranges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraRawDetailSettings {
    #[serde(default)]
    pub sharpen_amount: f64,
    #[serde(default = "default_10")]
    pub sharpen_radius: f64,
    #[serde(default = "default_25")]
    pub sharpen_detail: f64,
    #[serde(default)]
    pub sharpen_masking: f64,
    #[serde(default)]
    pub noise_luminance: f64,
    #[serde(default = "default_50")]
    pub noise_luminance_detail: f64,
    #[serde(default)]
    pub noise_luminance_contrast: f64,
    #[serde(default)]
    pub noise_color: f64,
    #[serde(default = "default_50")]
    pub noise_color_detail: f64,
    #[serde(default = "default_50")]
    pub noise_color_smoothness: f64,
}

impl Default for CameraRawDetailSettings {
    fn default() -> Self {
        CameraRawDetailSettings {
            sharpen_amount: 0.0,
            sharpen_radius: 10.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            noise_luminance: 0.0,
            noise_luminance_detail: 50.0,
            noise_luminance_contrast: 0.0,
            noise_color: 0.0,
            noise_color_detail: 50.0,
            noise_color_smoothness: 50.0,
        }
    }
}

impl CameraRawDetailSettings {
    pub const SHARPEN_AMOUNT_RANGE: (f64, f64) = (0.0, 150.0);
    pub const UNIT_RANGE: (f64, f64) = (0.0, 100.0);

    pub fn adjusts_sharpening(&self) -> bool {
        self.sharpen_amount != 0.0
    }

    pub fn adjusts_noise(&self) -> bool {
        self.noise_luminance != 0.0 || self.noise_color != 0.0
    }

    pub fn adjusts(&self) -> bool {
        self.adjusts_sharpening() || self.adjusts_noise()
    }

    pub fn normalized(&self) -> Self {
        let mut result = *self;
        result.sharpen_amount = clamp_slider(
            self.sharpen_amount,
            Self::SHARPEN_AMOUNT_RANGE.0,
            Self::SHARPEN_AMOUNT_RANGE.1,
            0.0,
        );
        result.sharpen_radius = clamp_slider(self.sharpen_radius, 0.0, 100.0, 10.0);
        result.sharpen_detail = clamp_slider(self.sharpen_detail, 0.0, 100.0, 25.0);
        result.sharpen_masking = clamp_slider(self.sharpen_masking, 0.0, 100.0, 0.0);
        result.noise_luminance = clamp_slider(self.noise_luminance, 0.0, 100.0, 0.0);
        result.noise_luminance_detail = clamp_slider(self.noise_luminance_detail, 0.0, 100.0, 50.0);
        result.noise_luminance_contrast =
            clamp_slider(self.noise_luminance_contrast, 0.0, 100.0, 0.0);
        result.noise_color = clamp_slider(self.noise_color, 0.0, 100.0, 0.0);
        result.noise_color_detail = clamp_slider(self.noise_color_detail, 0.0, 100.0, 50.0);
        result.noise_color_smoothness = clamp_slider(self.noise_color_smoothness, 0.0, 100.0, 50.0);
        result
    }

    pub fn applying(&self, shows: bool) -> Self {
        if shows { *self } else { Self::default() }
    }
}

/// Lens profile toggles, manual distortion, defringe, and lens-vignetting
/// correction (`CameraRawOpticsSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraRawOpticsSettings {
    #[serde(default)]
    pub remove_chromatic_aberration: bool,
    #[serde(default)]
    pub enable_lens_profile: bool,
    #[serde(default = "default_100")]
    pub profile_distortion: f64,
    #[serde(default = "default_100")]
    pub profile_vignetting: f64,
    /// −100…100, same sign convention as the Lens Correction filter.
    #[serde(default)]
    pub distortion: f64,
    #[serde(default)]
    pub purple_amount: f64,
    /// Degrees on the color wheel, 0…360. The low handle must stay below the high handle.
    #[serde(default = "default_270")]
    pub purple_hue_low: f64,
    #[serde(default = "default_310")]
    pub purple_hue_high: f64,
    #[serde(default)]
    pub green_amount: f64,
    #[serde(default = "default_60")]
    pub green_hue_low: f64,
    #[serde(default = "default_120")]
    pub green_hue_high: f64,
    /// Brightens the corners to counter lens falloff. Midpoint is 0…100.
    #[serde(default)]
    pub vignette_amount: f64,
    #[serde(default = "default_50")]
    pub vignette_midpoint: f64,
}

impl Default for CameraRawOpticsSettings {
    fn default() -> Self {
        CameraRawOpticsSettings {
            remove_chromatic_aberration: false,
            enable_lens_profile: false,
            profile_distortion: 100.0,
            profile_vignetting: 100.0,
            distortion: 0.0,
            purple_amount: 0.0,
            purple_hue_low: 270.0,
            purple_hue_high: 310.0,
            green_amount: 0.0,
            green_hue_low: 60.0,
            green_hue_high: 120.0,
            vignette_amount: 0.0,
            vignette_midpoint: 50.0,
        }
    }
}

/// The defringe eyedropper's hue centers and half-width
/// (`CameraRaw.swift:498-500`).
pub const DEFRINGE_PURPLE_CENTER: f64 = 290.0;
pub const DEFRINGE_GREEN_CENTER: f64 = 90.0;
pub const DEFRINGE_SPAN: f64 = 25.0;

impl CameraRawOpticsSettings {
    pub const TONE_RANGE: (f64, f64) = (-100.0, 100.0);
    pub const UNIT_RANGE: (f64, f64) = (0.0, 100.0);
    pub const HUE_RANGE: (f64, f64) = (0.0, 360.0);

    pub fn adjusts(&self) -> bool {
        self.remove_chromatic_aberration
            || self.enable_lens_profile
            || self.distortion != 0.0
            || self.purple_amount != 0.0
            || self.green_amount != 0.0
            || self.vignette_amount != 0.0
    }

    pub fn normalized(&self) -> Self {
        let mut result = *self;
        result.profile_distortion = clamp_slider(self.profile_distortion, 0.0, 100.0, 100.0);
        result.profile_vignetting = clamp_slider(self.profile_vignetting, 0.0, 100.0, 100.0);
        result.distortion = clamp_slider(self.distortion, -100.0, 100.0, 0.0);
        result.purple_amount = clamp_slider(self.purple_amount, 0.0, 100.0, 0.0);
        result.green_amount = clamp_slider(self.green_amount, 0.0, 100.0, 0.0);
        result.vignette_amount = clamp_slider(self.vignette_amount, -100.0, 100.0, 0.0);
        result.vignette_midpoint = clamp_slider(self.vignette_midpoint, 0.0, 100.0, 50.0);
        result.purple_hue_low = clamp_slider(self.purple_hue_low, 0.0, 360.0, 270.0);
        result.purple_hue_high = clamp_slider(self.purple_hue_high, 0.0, 360.0, 310.0);
        result.green_hue_low = clamp_slider(self.green_hue_low, 0.0, 360.0, 60.0);
        result.green_hue_high = clamp_slider(self.green_hue_high, 0.0, 360.0, 120.0);
        if result.purple_hue_low > result.purple_hue_high {
            std::mem::swap(&mut result.purple_hue_low, &mut result.purple_hue_high);
        }
        if result.green_hue_low > result.green_hue_high {
            std::mem::swap(&mut result.green_hue_low, &mut result.green_hue_high);
        }
        result
    }

    pub fn applying(&self, shows: bool) -> Self {
        if shows { *self } else { Self::default() }
    }

    /// Combined radial distortion passed to `lens_distort`, matching the Lens
    /// Correction filter scale (`distortionK(profileStrength:)`).
    pub fn distortion_k(&self, profile_strength: f64) -> f64 {
        let manual = self.distortion / 100.0 * profile_strength;
        let profile = if self.enable_lens_profile {
            self.profile_distortion / 100.0 * profile_strength
        } else {
            0.0
        };
        manual + profile
    }
}

/// Geometry (upright) settings (`CameraRawGeometrySettings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraRawGeometrySettings {
    #[serde(default)]
    pub upright: CameraRawUprightMode,
    #[serde(default)]
    pub projection: CameraRawProjection,
    #[serde(default)]
    pub vertical: f64,
    #[serde(default)]
    pub horizontal: f64,
    #[serde(default)]
    pub rotate: f64,
    #[serde(default)]
    pub aspect: f64,
    #[serde(default)]
    pub scale: f64,
    #[serde(default)]
    pub offset_x: f64,
    #[serde(default)]
    pub offset_y: f64,
    #[serde(default)]
    pub constrain_crop: bool,
    #[serde(default)]
    pub guides: Vec<CameraRawGeometryGuide>,
}

impl Default for CameraRawGeometrySettings {
    fn default() -> Self {
        CameraRawGeometrySettings {
            upright: CameraRawUprightMode::Off,
            projection: CameraRawProjection::Perspective,
            vertical: 0.0,
            horizontal: 0.0,
            rotate: 0.0,
            aspect: 0.0,
            scale: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            constrain_crop: false,
            guides: Vec::new(),
        }
    }
}

impl CameraRawGeometrySettings {
    pub const TONE_RANGE: (f64, f64) = (-100.0, 100.0);
    pub const ROTATE_RANGE: (f64, f64) = (-45.0, 45.0);

    /// Guided only counts once a line is long enough to read. An empty Guided
    /// choice must not warp the picture.
    fn uses_guides(&self) -> bool {
        self.upright == CameraRawUprightMode::Guided
            && self
                .guides
                .iter()
                .any(|g| (g.end_x - g.start_x).hypot(g.end_y - g.start_y) > 0.01)
    }

    pub fn adjusts(&self) -> bool {
        self.uses_guides()
            || self.vertical != 0.0
            || self.horizontal != 0.0
            || self.rotate != 0.0
            || self.aspect != 0.0
            || self.scale != 0.0
            || self.offset_x != 0.0
            || self.offset_y != 0.0
    }

    pub fn normalized(&self) -> Self {
        let mut result = self.clone();
        result.vertical = clamp_slider(self.vertical, Self::TONE_RANGE.0, Self::TONE_RANGE.1, 0.0);
        result.horizontal =
            clamp_slider(self.horizontal, Self::TONE_RANGE.0, Self::TONE_RANGE.1, 0.0);
        result.rotate = clamp_slider(self.rotate, Self::ROTATE_RANGE.0, Self::ROTATE_RANGE.1, 0.0);
        result.aspect = clamp_slider(self.aspect, Self::TONE_RANGE.0, Self::TONE_RANGE.1, 0.0);
        result.scale = clamp_slider(self.scale, Self::TONE_RANGE.0, Self::TONE_RANGE.1, 0.0);
        result.offset_x = clamp_slider(self.offset_x, Self::TONE_RANGE.0, Self::TONE_RANGE.1, 0.0);
        result.offset_y = clamp_slider(self.offset_y, Self::TONE_RANGE.0, Self::TONE_RANGE.1, 0.0);
        result.guides = self
            .guides
            .iter()
            .filter(|g| (g.end_x - g.start_x).hypot(g.end_y - g.start_y) > 0.01)
            .cloned()
            .collect();
        result
    }

    pub fn applying(&self, shows: bool) -> Self {
        if shows { self.clone() } else { Self::default() }
    }

    /// Perspective and affine geometry on the pixel grid. Output matches the
    /// input size unless Constrain Crop trims empty edges (`apply(_:)`).
    pub fn apply(&self, image: &Bitmap) -> Bitmap {
        let settings = self.normalized();
        if !settings.adjusts() {
            return image.clone();
        }
        let width = image.width;
        let height = image.height;
        if width == 0 || height == 0 {
            return image.clone();
        }
        let (vertical, horizontal, rotate) = settings.effective_corrections();
        let corners = settings.output_corners(width, height, vertical, horizontal, rotate);
        let result = perspective_warp(image, &corners);
        if !settings.constrain_crop {
            return result;
        }
        let Some(crop) = result.alpha_bounds() else {
            return result;
        };
        let (crop_w, crop_h) = (crop.width(), crop.height());
        if crop_w < 1.0 || crop_h < 1.0 || !(crop_w < width as f64 || crop_h < height as f64) {
            return result;
        }
        // Crop the bounds out, then fit them back into the full frame.
        let x0 = crop.min_x().max(0.0) as usize;
        let y0 = crop.min_y().max(0.0) as usize;
        let crop_w = (crop_w as usize).min(width - x0);
        let crop_h = (crop_h as usize).min(height - y0);
        let mut cropped_data = Vec::with_capacity(crop_w * crop_h * 4);
        for y in y0..y0 + crop_h {
            let start = (y * width + x0) * 4;
            cropped_data.extend_from_slice(&result.data[start..start + crop_w * 4]);
        }
        let cropped = Bitmap::from_raw(crop_w, crop_h, cropped_data);
        let fit = (width as f64 / crop_w as f64).min(height as f64 / crop_h as f64);
        let mut fitted = Bitmap::new(width, height);
        let draw = CGRect::new(
            (width as f64 - crop_w as f64 * fit) / 2.0,
            (height as f64 - crop_h as f64 * fit) / 2.0,
            crop_w as f64 * fit,
            crop_h as f64 * fit,
        );
        draw_over(&mut fitted, &cropped, draw, LayerSampling::Smooth);
        fitted
    }

    fn effective_corrections(&self) -> (f64, f64, f64) {
        match self.upright {
            CameraRawUprightMode::Off => (self.vertical, self.horizontal, self.rotate),
            CameraRawUprightMode::Guided => {
                let guided = Self::guided_corrections(&self.guides);
                (
                    self.vertical + guided.0,
                    self.horizontal + guided.1,
                    self.rotate + guided.2,
                )
            }
        }
    }

    fn guided_corrections(guides: &[CameraRawGeometryGuide]) -> (f64, f64, f64) {
        let Some(first) = guides.first() else {
            return (0.0, 0.0, 0.0);
        };
        let dx = first.end_x - first.start_x;
        let dy = first.end_y - first.start_y;
        let length = dx.hypot(dy);
        if length <= 1e-4 {
            return (0.0, 0.0, 0.0);
        }
        let angle = dy.atan2(dx) * 180.0 / std::f64::consts::PI;
        let mut rotate = -angle;
        if rotate > 45.0 {
            rotate -= 90.0;
        } else if rotate < -45.0 {
            rotate += 90.0;
        }
        let mut vertical = 0.0;
        let mut horizontal = 0.0;
        if guides.len() > 1 {
            let second = &guides[1];
            let sx = second.end_x - second.start_x;
            let sy = second.end_y - second.start_y;
            let sl = sx.hypot(sy);
            if sl > 1e-4 {
                let a2 = sy.atan2(sx) * 180.0 / std::f64::consts::PI;
                vertical = if a2.abs() > 45.0 {
                    if a2 > 0.0 { 25.0 } else { -25.0 }
                } else {
                    0.0
                };
                horizontal = if a2.abs() <= 45.0 {
                    if a2 > 0.0 { 25.0 } else { -25.0 }
                } else {
                    0.0
                };
            }
        }
        (vertical, horizontal, rotate)
    }

    /// Corner positions with y measured upward from the bottom
    /// (`outputCorners(width:height:vertical:horizontal:rotation:)`).
    fn output_corners(
        &self,
        width: usize,
        height: usize,
        vertical: f64,
        horizontal: f64,
        rotation: f64,
    ) -> [(f64, f64); 4] {
        let w = width as f64;
        let h = height as f64;
        let strength = if self.projection == CameraRawProjection::Perspective {
            1.0
        } else {
            0.55
        };
        let v = vertical / 100.0 * w * 0.18 * strength;
        let hz = horizontal / 100.0 * h * 0.18 * strength;
        let aspect_scale = 1.0 + self.aspect / 200.0;
        let zoom = 1.0 + self.scale / 100.0;
        let shift_x = self.offset_x / 100.0 * w * 0.15;
        let shift_y = self.offset_y / 100.0 * h * 0.15;
        let mut top_left = (-v + shift_x, h + shift_y);
        let mut top_right = (w + v + shift_x, h + shift_y);
        let mut bottom_right = (w + hz + shift_x, -shift_y);
        let mut bottom_left = (-hz + shift_x, -shift_y);
        let center = (w / 2.0 + shift_x, h / 2.0 + shift_y);
        let radians = rotation * std::f64::consts::PI / 180.0;
        let rotated = |point: (f64, f64)| -> (f64, f64) {
            let (dx, dy) = (point.0 - center.0, point.1 - center.1);
            let (cosine, sine) = (radians.cos(), radians.sin());
            (
                center.0 + dx * cosine - dy * sine,
                center.1 + dx * sine + dy * cosine,
            )
        };
        top_left = rotated(top_left);
        top_right = rotated(top_right);
        bottom_right = rotated(bottom_right);
        bottom_left = rotated(bottom_left);
        if aspect_scale != 1.0 {
            let scaled = |point: (f64, f64)| -> (f64, f64) {
                (
                    center.0 + (point.0 - center.0) * aspect_scale,
                    center.1 + (point.1 - center.1) / aspect_scale,
                )
            };
            top_left = scaled(top_left);
            top_right = scaled(top_right);
            bottom_right = scaled(bottom_right);
            bottom_left = scaled(bottom_left);
        }
        if zoom != 1.0 {
            let zoomed = |point: (f64, f64)| -> (f64, f64) {
                (
                    center.0 + (point.0 - center.0) * zoom,
                    center.1 + (point.1 - center.1) * zoom,
                )
            };
            top_left = zoomed(top_left);
            top_right = zoomed(top_right);
            bottom_right = zoomed(bottom_right);
            bottom_left = zoomed(bottom_left);
        }
        [top_left, top_right, bottom_right, bottom_left]
    }
}

/// Camera calibration settings (`CameraRawCalibrationSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraRawCalibrationSettings {
    #[serde(default)]
    pub process: CameraRawProcessVersion,
    #[serde(default)]
    pub shadow_tint: f64,
    #[serde(default)]
    pub red_hue: f64,
    #[serde(default)]
    pub red_saturation: f64,
    #[serde(default)]
    pub green_hue: f64,
    #[serde(default)]
    pub green_saturation: f64,
    #[serde(default)]
    pub blue_hue: f64,
    #[serde(default)]
    pub blue_saturation: f64,
}

impl Default for CameraRawCalibrationSettings {
    fn default() -> Self {
        CameraRawCalibrationSettings {
            process: CameraRawProcessVersion::Version6,
            shadow_tint: 0.0,
            red_hue: 0.0,
            red_saturation: 0.0,
            green_hue: 0.0,
            green_saturation: 0.0,
            blue_hue: 0.0,
            blue_saturation: 0.0,
        }
    }
}

impl CameraRawCalibrationSettings {
    pub const TONE_RANGE: (f64, f64) = (-100.0, 100.0);

    pub fn adjusts(&self) -> bool {
        self.shadow_tint != 0.0
            || self.red_hue != 0.0
            || self.red_saturation != 0.0
            || self.green_hue != 0.0
            || self.green_saturation != 0.0
            || self.blue_hue != 0.0
            || self.blue_saturation != 0.0
    }

    pub fn normalized(&self) -> Self {
        let mut result = *self;
        let (lo, hi) = Self::TONE_RANGE;
        result.shadow_tint = clamp_slider(self.shadow_tint, lo, hi, 0.0);
        result.red_hue = clamp_slider(self.red_hue, lo, hi, 0.0);
        result.red_saturation = clamp_slider(self.red_saturation, lo, hi, 0.0);
        result.green_hue = clamp_slider(self.green_hue, lo, hi, 0.0);
        result.green_saturation = clamp_slider(self.green_saturation, lo, hi, 0.0);
        result.blue_hue = clamp_slider(self.blue_hue, lo, hi, 0.0);
        result.blue_saturation = clamp_slider(self.blue_saturation, lo, hi, 0.0);
        result
    }

    pub fn applying(&self, shows: bool) -> Self {
        if shows { *self } else { Self::default() }
    }
}

fn default_50() -> f64 {
    50.0
}
fn default_25() -> f64 {
    25.0
}

/// Camera Raw Filter settings (`CameraRawSettings`). Defaults leave the image unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraRawSettings {
    #[serde(default)]
    pub white_balance: CameraRawWhiteBalance,
    /// Relative cool-to-warm, −100…100. Positive is warmer.
    #[serde(default)]
    pub temperature: f64,
    /// Green-to-magenta, −100…100. Positive is magenta.
    #[serde(default)]
    pub tint: f64,
    /// Stops of linear light, −5…5.
    #[serde(default)]
    pub exposure: f64,
    #[serde(default)]
    pub contrast: f64,
    #[serde(default)]
    pub highlights: f64,
    #[serde(default)]
    pub shadows: f64,
    #[serde(default)]
    pub whites: f64,
    #[serde(default)]
    pub blacks: f64,
    #[serde(default)]
    pub vibrance: f64,
    #[serde(default)]
    pub saturation: f64,
    /// Local contrast, −100…100. Texture is the finer band; Clarity is the broader one.
    #[serde(default)]
    pub texture: f64,
    #[serde(default)]
    pub clarity: f64,
    /// −100…100. Positive deepens contrast and saturation; negative lifts shadows and fades color.
    #[serde(default)]
    pub dehaze: f64,
    /// 0…100. Range, spread, and warmth are idle while this stays at zero.
    #[serde(default)]
    pub glow: f64,
    #[serde(default)]
    pub glow_style: CameraRawGlowStyle,
    #[serde(default)]
    pub glow_range: f64,
    #[serde(default)]
    pub glow_spread: f64,
    #[serde(default)]
    pub glow_warmth: f64,
    /// −100…100. Negative darkens the edges, positive lightens them. The center is left alone.
    #[serde(default)]
    pub vignette_amount: f64,
    #[serde(default)]
    pub vignette_style: CameraRawVignetteStyle,
    #[serde(default = "default_50")]
    pub vignette_midpoint: f64,
    #[serde(default)]
    pub vignette_roundness: f64,
    #[serde(default = "default_50")]
    pub vignette_feather: f64,
    /// Used only while `vignette_amount` darkens, and only for Highlight Priority.
    #[serde(default)]
    pub vignette_highlights: f64,
    /// 0…100. Zero adds no grain. Size is mapped into the shared grain kernel's pixel scale.
    #[serde(default)]
    pub grain_amount: f64,
    #[serde(default = "default_25")]
    pub grain_size: f64,
    #[serde(default = "default_50")]
    pub grain_roughness: f64,
    #[serde(default)]
    pub curve: CameraRawCurveSettings,
    #[serde(default)]
    pub mixer: CameraRawMixerSettings,
    #[serde(default)]
    pub grading: CameraRawGradingSettings,
    #[serde(default)]
    pub detail: CameraRawDetailSettings,
    #[serde(default)]
    pub optics: CameraRawOpticsSettings,
    #[serde(default)]
    pub geometry: CameraRawGeometrySettings,
    #[serde(default)]
    pub calibration: CameraRawCalibrationSettings,
}

impl Default for CameraRawSettings {
    fn default() -> Self {
        CameraRawSettings {
            white_balance: CameraRawWhiteBalance::Custom,
            temperature: 0.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            glow: 0.0,
            glow_style: CameraRawGlowStyle::Diffusion,
            glow_range: 0.0,
            glow_spread: 0.0,
            glow_warmth: 0.0,
            vignette_amount: 0.0,
            vignette_style: CameraRawVignetteStyle::HighlightPriority,
            vignette_midpoint: 50.0,
            vignette_roundness: 0.0,
            vignette_feather: 50.0,
            vignette_highlights: 0.0,
            grain_amount: 0.0,
            grain_size: 25.0,
            grain_roughness: 50.0,
            curve: CameraRawCurveSettings::default(),
            mixer: CameraRawMixerSettings::default(),
            grading: CameraRawGradingSettings::default(),
            detail: CameraRawDetailSettings::default(),
            optics: CameraRawOpticsSettings::default(),
            geometry: CameraRawGeometrySettings::default(),
            calibration: CameraRawCalibrationSettings::default(),
        }
    }
}

impl CameraRawSettings {
    pub const EXPOSURE_RANGE: (f64, f64) = (-5.0, 5.0);
    pub const TONE_RANGE: (f64, f64) = (-100.0, 100.0);
    pub const UNIT_RANGE: (f64, f64) = (0.0, 100.0);
    /// Share of a full warm/cool swing applied to red and blue.
    pub const TEMPERATURE_GAIN: f64 = 0.35;
    /// Magenta/green swing shared by red and blue.
    pub const TINT_RED_BLUE: f64 = 0.15;
    /// Magenta/green swing on green, opposite the other two channels.
    pub const TINT_GREEN: f64 = 0.30;

    pub fn adjusts_light(&self) -> bool {
        self.exposure != 0.0
            || self.contrast != 0.0
            || self.highlights != 0.0
            || self.shadows != 0.0
            || self.whites != 0.0
            || self.blacks != 0.0
    }

    pub fn adjusts_color(&self) -> bool {
        self.temperature != 0.0
            || self.tint != 0.0
            || self.vibrance != 0.0
            || self.saturation != 0.0
    }

    pub fn adjusts_effects(&self) -> bool {
        self.texture != 0.0
            || self.clarity != 0.0
            || self.dehaze != 0.0
            || self.glow != 0.0
            || self.vignette_amount != 0.0
            || self.grain_amount != 0.0
    }

    pub fn adjusts_curve(&self) -> bool {
        self.curve.adjusts()
    }
    pub fn adjusts_mixer(&self) -> bool {
        self.mixer.adjusts()
    }
    pub fn adjusts_grading(&self) -> bool {
        self.grading.adjusts()
    }
    pub fn adjusts_detail(&self) -> bool {
        self.detail.adjusts()
    }
    pub fn adjusts_optics(&self) -> bool {
        self.optics.adjusts()
    }
    pub fn adjusts_geometry(&self) -> bool {
        self.geometry.adjusts()
    }
    pub fn adjusts_calibration(&self) -> bool {
        self.calibration.adjusts()
    }

    pub fn is_identity(&self) -> bool {
        !self.adjusts_light()
            && !self.adjusts_color()
            && !self.adjusts_effects()
            && !self.adjusts_curve()
            && !self.adjusts_mixer()
            && !self.adjusts_grading()
            && !self.adjusts_detail()
            && !self.adjusts_optics()
            && !self.adjusts_geometry()
            && !self.adjusts_calibration()
    }

    pub fn is_valid(&self) -> bool {
        (Self::EXPOSURE_RANGE.0..=Self::EXPOSURE_RANGE.1).contains(&self.exposure)
            && self.exposure.is_finite()
            && [
                self.contrast,
                self.highlights,
                self.shadows,
                self.whites,
                self.blacks,
                self.temperature,
                self.tint,
                self.vibrance,
                self.saturation,
                self.texture,
                self.clarity,
                self.dehaze,
                self.glow_range,
                self.glow_spread,
                self.glow_warmth,
                self.vignette_amount,
                self.vignette_roundness,
            ]
            .iter()
            .all(|v| v.is_finite() && (Self::TONE_RANGE.0..=Self::TONE_RANGE.1).contains(v))
            && [
                self.glow,
                self.vignette_midpoint,
                self.vignette_feather,
                self.vignette_highlights,
                self.grain_amount,
                self.grain_size,
                self.grain_roughness,
            ]
            .iter()
            .all(|v| v.is_finite() && (Self::UNIT_RANGE.0..=Self::UNIT_RANGE.1).contains(v))
    }

    pub fn normalized(&self) -> Self {
        let mut result = self.clone();
        let (exp_lo, exp_hi) = Self::EXPOSURE_RANGE;
        let (lo, hi) = Self::TONE_RANGE;
        let (ulo, uhi) = Self::UNIT_RANGE;
        result.exposure = clamp_slider(self.exposure, exp_lo, exp_hi, 0.0);
        result.contrast = clamp_slider(self.contrast, lo, hi, 0.0);
        result.highlights = clamp_slider(self.highlights, lo, hi, 0.0);
        result.shadows = clamp_slider(self.shadows, lo, hi, 0.0);
        result.whites = clamp_slider(self.whites, lo, hi, 0.0);
        result.blacks = clamp_slider(self.blacks, lo, hi, 0.0);
        result.temperature = clamp_slider(self.temperature, lo, hi, 0.0);
        result.tint = clamp_slider(self.tint, lo, hi, 0.0);
        result.vibrance = clamp_slider(self.vibrance, lo, hi, 0.0);
        result.saturation = clamp_slider(self.saturation, lo, hi, 0.0);
        result.texture = clamp_slider(self.texture, lo, hi, 0.0);
        result.clarity = clamp_slider(self.clarity, lo, hi, 0.0);
        result.dehaze = clamp_slider(self.dehaze, lo, hi, 0.0);
        result.glow = clamp_slider(self.glow, ulo, uhi, 0.0);
        result.glow_range = clamp_slider(self.glow_range, lo, hi, 0.0);
        result.glow_spread = clamp_slider(self.glow_spread, lo, hi, 0.0);
        result.glow_warmth = clamp_slider(self.glow_warmth, lo, hi, 0.0);
        result.vignette_amount = clamp_slider(self.vignette_amount, lo, hi, 0.0);
        result.vignette_midpoint = clamp_slider(self.vignette_midpoint, ulo, uhi, 50.0);
        result.vignette_roundness = clamp_slider(self.vignette_roundness, lo, hi, 0.0);
        result.vignette_feather = clamp_slider(self.vignette_feather, ulo, uhi, 50.0);
        result.vignette_highlights = clamp_slider(self.vignette_highlights, ulo, uhi, 0.0);
        result.grain_amount = clamp_slider(self.grain_amount, ulo, uhi, 0.0);
        result.grain_size = clamp_slider(self.grain_size, ulo, uhi, 25.0);
        result.grain_roughness = clamp_slider(self.grain_roughness, ulo, uhi, 50.0);
        result.curve = self.curve.normalized();
        result.mixer = self.mixer.normalized();
        result.grading = self.grading.normalized();
        result.detail = self.detail.normalized();
        result.optics = self.optics.normalized();
        result.geometry = self.geometry.normalized();
        result.calibration = self.calibration.normalized();
        result
    }

    /// The grade with a panel's eye turned off: that group's amounts become
    /// zero and the rest stay.
    pub fn applying(
        &self,
        shows_light: bool,
        shows_color: bool,
        shows_effects: bool,
        shows_curve: bool,
        shows_mixer: bool,
        shows_grading: bool,
        shows_detail: bool,
        shows_optics: bool,
        shows_geometry: bool,
        shows_calibration: bool,
    ) -> Self {
        let mut result = self.clone();
        if !shows_light {
            result.exposure = 0.0;
            result.contrast = 0.0;
            result.highlights = 0.0;
            result.shadows = 0.0;
            result.whites = 0.0;
            result.blacks = 0.0;
        }
        if !shows_color {
            result.temperature = 0.0;
            result.tint = 0.0;
            result.vibrance = 0.0;
            result.saturation = 0.0;
        }
        if !shows_effects {
            result.texture = 0.0;
            result.clarity = 0.0;
            result.dehaze = 0.0;
            result.glow = 0.0;
            result.vignette_amount = 0.0;
            result.grain_amount = 0.0;
        }
        if !shows_curve {
            result.curve = CameraRawCurveSettings::default();
        }
        if !shows_mixer {
            result.mixer = CameraRawMixerSettings::default();
        }
        if !shows_grading {
            result.grading = CameraRawGradingSettings::default();
        }
        if !shows_detail {
            result.detail = CameraRawDetailSettings::default();
        }
        if !shows_optics {
            result.optics = CameraRawOpticsSettings::default();
        }
        if !shows_geometry {
            result.geometry = CameraRawGeometrySettings::default();
        }
        if !shows_calibration {
            result.calibration = CameraRawCalibrationSettings::default();
        }
        result
    }

    /// Camera Raw's 0…100 size, in the pixel scale `adjust_grain` already uses.
    pub fn grain_kernel_size(&self) -> f64 {
        0.5 + (self.grain_size / 100.0) * 19.5
    }

    /// Channel multipliers for `temperature` and `tint`. Neutral is 1, 1, 1.
    pub fn gains(&self) -> (f64, f64, f64) {
        let warm = self.temperature / 100.0;
        let magenta = self.tint / 100.0;
        (
            1.0 + Self::TEMPERATURE_GAIN * warm + Self::TINT_RED_BLUE * magenta,
            1.0 - Self::TINT_GREEN * magenta,
            1.0 - Self::TEMPERATURE_GAIN * warm + Self::TINT_RED_BLUE * magenta,
        )
    }

    /// Renders the grade onto a copy of `image` (`apply(clipping:scale:seed:
    /// visualizePointColor:sharpenMask:)`). `clipping` draws the Option-drag
    /// overlay instead of the grade. `scale` is preview pixels per layer pixel.
    pub fn apply(
        &self,
        image: &Bitmap,
        clipping: Option<CameraRawClipping>,
        scale: f64,
        seed: u32,
        visualize_point_color: i32,
        sharpen_mask: bool,
    ) -> Bitmap {
        // Swift guards isValid before applying (CameraRaw.swift:224).
        if !self.is_valid() {
            return image.clone();
        }
        let settings = self.normalized();
        if settings.is_identity()
            && clipping.is_none()
            && visualize_point_color < 0
            && !sharpen_mask
        {
            return image.clone();
        }
        let (red_gain, green_gain, blue_gain) = settings.gains();
        let mode = match clipping {
            Some(CameraRawClipping::Highlights) => 1,
            Some(CameraRawClipping::Shadows) => 2,
            None => 0,
        };
        let pixel_scale = if scale > 0.0 { scale } else { 1.0 };
        let paint_color = clipping.is_none()
            && !sharpen_mask
            && (settings.adjusts_curve()
                || settings.adjusts_mixer()
                || settings.adjusts_grading()
                || visualize_point_color >= 0);
        let paint_effects = clipping.is_none() && !sharpen_mask && settings.adjusts_effects();
        let paint_detail_optics = clipping.is_none()
            && (settings.adjusts_detail() || settings.adjusts_optics() || sharpen_mask);
        let mut result = if clipping.is_none()
            && !sharpen_mask
            && visualize_point_color < 0
            && settings.adjusts_geometry()
        {
            settings.geometry.apply(image)
        } else {
            image.clone()
        };
        if clipping.is_none() && !sharpen_mask && settings.adjusts_calibration() {
            settings.apply_calibration(&mut result);
        }
        if settings.adjusts_light() || settings.adjusts_color() || clipping.is_some() {
            adjust_camera_raw(
                &mut result,
                red_gain,
                green_gain,
                blue_gain,
                settings.exposure,
                settings.contrast,
                settings.highlights,
                settings.shadows,
                settings.whites,
                settings.blacks,
                settings.vibrance,
                settings.saturation,
                mode,
            );
        }
        if paint_color {
            settings.apply_curve_color(&mut result, visualize_point_color);
        }
        if paint_effects {
            if settings.texture != 0.0
                || settings.clarity != 0.0
                || settings.dehaze != 0.0
                || settings.glow != 0.0
                || settings.vignette_amount != 0.0
            {
                adjust_camera_raw_effects(
                    &mut result,
                    settings.texture,
                    settings.clarity,
                    settings.dehaze,
                    settings.glow,
                    settings.glow_style.kernel_value(),
                    settings.glow_range,
                    settings.glow_spread,
                    settings.glow_warmth,
                    settings.vignette_amount,
                    settings.vignette_midpoint,
                    settings.vignette_roundness,
                    settings.vignette_feather,
                    settings.vignette_highlights,
                    settings.vignette_style.kernel_value(),
                    pixel_scale,
                );
            }
            if settings.grain_amount > 0.0 {
                super::image_adjustments::adjust_grain(
                    &mut result,
                    settings.grain_amount,
                    settings.grain_kernel_size(),
                    settings.grain_roughness,
                    seed,
                    0.0,
                    0.0,
                    1.0 / pixel_scale,
                );
            }
        }
        if paint_detail_optics {
            settings.apply_detail_optics(
                &mut result,
                pixel_scale,
                crate::filters::LENS_STRENGTH,
                sharpen_mask,
            );
        }
        result
    }

    /// Runs Curve, then Color Mixer, then Color Grading
    /// (`applyCurveColor(pixels:width:height:stride:visualize:)`). `visualize`
    /// dims pixels outside that point color.
    fn apply_curve_color(&self, bitmap: &mut Bitmap, visualize: i32) {
        let curve = self.curve.normalized();
        let mixer = self.mixer.normalized();
        let grading = self.grading.normalized();
        let luma = curve.luma_table();
        let red = curve.channel_table(&curve.red);
        let green = curve.channel_table(&curve.green);
        let blue = curve.channel_table(&curve.blue);
        let mixer_floats = mixer.mixer_floats();
        let point_floats = mixer.point_floats();
        let grade = grading.grade_floats();
        let point_count = mixer.points.len() as i32;
        adjust_camera_raw_curve_color(
            bitmap,
            &luma,
            &red,
            &green,
            &blue,
            curve.refine_saturation / 100.0,
            &mixer_floats,
            point_count,
            &point_floats,
            &grade,
            grading.blending / 100.0,
            grading.balance / 100.0,
            visualize,
        );
    }

    /// Sharpening, noise reduction, and optics
    /// (`applyDetailOptics(pixels:width:height:stride:scale:profileStrength:
    /// sharpenMask:)`).
    fn apply_detail_optics(
        &self,
        bitmap: &mut Bitmap,
        scale: f64,
        profile_strength: f64,
        sharpen_mask: bool,
    ) {
        let detail = self.detail.normalized();
        let optics = self.optics.normalized();
        if sharpen_mask {
            adjust_camera_raw_sharpen_mask_overlay(
                bitmap,
                detail.sharpen_radius,
                detail.sharpen_detail,
                detail.sharpen_masking,
                scale,
            );
            return;
        }
        if optics.adjusts() {
            adjust_camera_raw_optics(
                bitmap,
                if optics.remove_chromatic_aberration {
                    1
                } else {
                    0
                },
                if optics.enable_lens_profile { 1 } else { 0 },
                optics.profile_distortion,
                optics.profile_vignetting,
                optics.distortion_k(profile_strength),
                optics.purple_amount,
                optics.purple_hue_low,
                optics.purple_hue_high,
                optics.green_amount,
                optics.green_hue_low,
                optics.green_hue_high,
                optics.vignette_amount,
                optics.vignette_midpoint,
                scale,
            );
        }
        if detail.adjusts() {
            adjust_camera_raw_detail(
                bitmap,
                detail.sharpen_amount,
                detail.sharpen_radius,
                detail.sharpen_detail,
                detail.sharpen_masking,
                detail.noise_luminance,
                detail.noise_luminance_detail,
                detail.noise_luminance_contrast,
                detail.noise_color,
                detail.noise_color_detail,
                detail.noise_color_smoothness,
                scale,
            );
        }
    }

    /// Calibration sliders on an already-rendered layer
    /// (`applyCalibration(pixels:width:height:stride:)`).
    fn apply_calibration(&self, bitmap: &mut Bitmap) {
        let calibration = self.calibration.normalized();
        if !calibration.adjusts() {
            return;
        }
        adjust_camera_raw_calibration(
            bitmap,
            calibration.shadow_tint,
            calibration.red_hue,
            calibration.red_saturation,
            calibration.green_hue,
            calibration.green_saturation,
            calibration.blue_hue,
            calibration.blue_saturation,
            calibration.process.kernel_value(),
        );
    }

    /// Temperature and tint that bring one linear-light pixel to neutral, using
    /// the same gains `apply` multiplies (`neutralize(linearRed:green:blue:)`).
    pub fn neutralize_linear(red: f64, green: f64, blue: f64) -> Option<(f64, f64)> {
        if red <= 1e-4 || green <= 1e-4 || blue <= 1e-4 {
            return None;
        }
        let a1 = Self::TEMPERATURE_GAIN * red;
        let b1 = Self::TINT_RED_BLUE * red + Self::TINT_GREEN * green;
        let c1 = green - red;
        let a2 = -Self::TEMPERATURE_GAIN * blue;
        let b2 = Self::TINT_RED_BLUE * blue + Self::TINT_GREEN * green;
        let c2 = green - blue;
        let determinant = a1 * b2 - a2 * b1;
        if determinant.abs() <= 1e-8 {
            return None;
        }
        let warm = (c1 * b2 - c2 * b1) / determinant;
        let magenta = (a1 * c2 - a2 * c1) / determinant;
        if !warm.is_finite() || !magenta.is_finite() {
            return None;
        }
        Some((warm * 100.0, magenta * 100.0))
    }

    /// `neutralize(straightRed:green:blue:)`.
    pub fn neutralize_straight(red: f64, green: f64, blue: f64) -> Option<(f64, f64)> {
        Self::neutralize_linear(
            decode_linear(red),
            decode_linear(green),
            decode_linear(blue),
        )
    }

    /// Gray-world balance of the opaque pixels (`autoBalance(of:)`). Nil when
    /// the image has no coverage or no solution.
    pub fn auto_balance(image: &Bitmap) -> Option<(f64, f64)> {
        if image.width == 0 || image.height == 0 {
            return None;
        }
        let mut red = 0.0;
        let mut green = 0.0;
        let mut blue = 0.0;
        let mut count = 0.0;
        for px in image.data.chunks_exact(4) {
            let alpha = px[3] as f64;
            if alpha == 0.0 {
                continue;
            }
            red += decode_linear((px[0] as f64 / alpha).min(1.0));
            green += decode_linear((px[1] as f64 / alpha).min(1.0));
            blue += decode_linear((px[2] as f64 / alpha).min(1.0));
            count += 1.0;
        }
        if count <= 0.0 {
            return None;
        }
        Self::neutralize_linear(red / count, green / count, blue / count)
    }
}

/// Standard RGB → hue in degrees (0…360); gray returns 0
/// (`CameraRaw.hueDegrees`).
pub fn hue_degrees(red: f64, green: f64, blue: f64) -> f64 {
    let max_c = red.max(green).max(blue);
    let min_c = red.min(green).min(blue);
    let chroma = max_c - min_c;
    if chroma <= 1e-6 {
        return 0.0;
    }
    let hue = if max_c == red {
        (green - blue) / chroma
    } else if max_c == green {
        2.0 + (blue - red) / chroma
    } else {
        4.0 + (red - green) / chroma
    };
    let mut degrees = hue * 60.0;
    if degrees < 0.0 {
        degrees += 360.0;
    }
    degrees
}

/// The hue of one premultiplied pixel; `None` for a fully transparent sample
/// (`sampleCameraRawDefringe`'s `bytes[3] > 0` guard).
pub fn straight_pixel_hue(pixel: [u8; 4]) -> Option<f64> {
    let alpha = pixel[3] as f64;
    if alpha <= 0.0 {
        return None;
    }
    let r = (pixel[0] as f64 / alpha).min(1.0);
    let g = (pixel[1] as f64 / alpha).min(1.0);
    let b = (pixel[2] as f64 / alpha).min(1.0);
    Some(hue_degrees(r, g, b))
}

/// `sampleCameraRawDefringe`'s optics update: centers the purple or green
/// defringe range on the sampled hue, whichever center is nearer; an amount
/// still at zero starts at 50. `optics` ends normalized.
pub fn apply_defringe_sample(optics: &mut CameraRawOpticsSettings, hue: f64) {
    if (hue - DEFRINGE_PURPLE_CENTER).abs() < (hue - DEFRINGE_GREEN_CENTER).abs() {
        optics.purple_hue_low = hue - DEFRINGE_SPAN;
        optics.purple_hue_high = hue + DEFRINGE_SPAN;
        if optics.purple_amount == 0.0 {
            optics.purple_amount = 50.0;
        }
    } else {
        optics.green_hue_low = hue - DEFRINGE_SPAN;
        optics.green_hue_high = hue + DEFRINGE_SPAN;
        if optics.green_amount == 0.0 {
            optics.green_amount = 50.0;
        }
    }
    *optics = optics.normalized();
}

/// `applyCameraRawAutoWhiteBalance`: selects the Auto mode and solves the
/// temperature/tint for `image`. False when the solver had nothing to work
/// with.
pub fn apply_auto_white_balance(settings: &mut CameraRawSettings, image: &Bitmap) -> bool {
    settings.white_balance = CameraRawWhiteBalance::Auto;
    match CameraRawSettings::auto_balance(image) {
        Some((temperature, tint)) => {
            settings.temperature = temperature;
            settings.tint = tint;
            true
        }
        None => false,
    }
}

/// `ImageAdjustmentPixels`/`CameraRawSettings.decode`: encoded sRGB to linear light.
pub fn decode_linear(encoded: f64) -> f64 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod eyedropper_tests {
    use super::*;

    #[test]
    fn hue_degrees_walks_the_color_wheel() {
        assert!((hue_degrees(1.0, 0.0, 0.0) - 0.0).abs() < 1e-9, "red");
        assert!((hue_degrees(0.0, 1.0, 0.0) - 120.0).abs() < 1e-9, "green");
        assert!((hue_degrees(0.0, 0.0, 1.0) - 240.0).abs() < 1e-9, "blue");
        assert!((hue_degrees(1.0, 1.0, 0.5) - 60.0).abs() < 1e-9, "yellow");
        assert_eq!(hue_degrees(0.5, 0.5, 0.5), 0.0, "gray has no hue");
        // Magenta wraps past 360.
        assert!((hue_degrees(1.0, 0.0, 1.0) - 300.0).abs() < 1e-9);
    }

    #[test]
    fn defringe_sample_centers_the_nearer_range() {
        // A purple fringe (near 290) centers the purple handles.
        let mut optics = CameraRawOpticsSettings::default();
        apply_defringe_sample(&mut optics, 300.0);
        assert!((optics.purple_hue_low - 275.0).abs() < 1e-9, "{optics:?}");
        assert!((optics.purple_hue_high - 325.0).abs() < 1e-9);
        assert!(
            (optics.purple_amount - 50.0).abs() < 1e-9,
            "an untouched amount starts at 50"
        );
        assert_eq!(optics.green_amount, 0.0);
        // A green fringe (near 90) centers the green handles instead.
        let mut optics = CameraRawOpticsSettings::default();
        apply_defringe_sample(&mut optics, 100.0);
        assert!((optics.green_hue_low - 75.0).abs() < 1e-9);
        assert!((optics.green_hue_high - 125.0).abs() < 1e-9);
        assert_eq!(optics.purple_amount, 0.0);
        // Normalized handles stay on the color wheel even across 0/360.
        let mut optics = CameraRawOpticsSettings::default();
        apply_defringe_sample(&mut optics, 359.0);
        assert!(
            (0.0..=360.0).contains(&optics.purple_hue_low),
            "{}",
            optics.purple_hue_low
        );
        assert!(
            (0.0..=360.0).contains(&optics.purple_hue_high),
            "{}",
            optics.purple_hue_high
        );
    }

    #[test]
    fn straight_pixel_hue_skips_transparent_pixels() {
        assert_eq!(straight_pixel_hue([255, 0, 0, 0]), None);
        assert!(straight_pixel_hue([128, 0, 0, 128]).is_some());
    }

    #[test]
    fn auto_white_balance_selects_auto_and_solves() {
        // A gray field is already balanced: temperature and tint stay near 0.
        let mut gray = Bitmap::new(8, 8);
        for pixel in gray.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[128, 128, 128, 255]);
        }
        let mut settings = CameraRawSettings::default();
        assert!(apply_auto_white_balance(&mut settings, &gray));
        assert_eq!(settings.white_balance, CameraRawWhiteBalance::Auto);
        assert!(settings.temperature.abs() < 5.0, "{}", settings.temperature);
        assert!(settings.tint.abs() < 5.0, "{}", settings.tint);
        // An empty image has nothing to solve.
        let empty = Bitmap::new(4, 4);
        let mut settings = CameraRawSettings::default();
        assert!(!apply_auto_white_balance(&mut settings, &empty));
    }
}

// ---------------------------------------------------------------------------
// Pixel kernels ported from Rendering/AdjustPixels.c.
// ---------------------------------------------------------------------------

#[inline]
fn camera_clamp(value: f64) -> f64 {
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

#[inline]
fn srgb_to_linear(encoded: f64) -> f64 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
fn linear_to_srgb(linear: f64) -> f64 {
    if linear <= 0.0 {
        return 0.0;
    }
    if linear >= 1.0 {
        return 1.0;
    }
    if linear <= 0.0031308 {
        return linear * 12.92;
    }
    1.055 * linear.powf(1.0 / 2.4) - 0.055
}

#[inline]
fn rec709(r: f64, g: f64, b: f64) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Moves r, g, b so their Rec. 709 luminance becomes `target`, keeping the hue.
/// Pure black cannot be scaled, so a lift paints neutral light of that luminance.
fn scale_luminance(rgb: &mut [f64; 3], target: f64) {
    let target = camera_clamp(target);
    let y = rec709(rgb[0], rgb[1], rgb[2]);
    if (target - y).abs() < 1e-8 {
        return;
    }
    if y < 1e-8 {
        if target > y {
            *rgb = [target; 3];
        }
        return;
    }
    let scale = target / y;
    rgb[0] = camera_clamp(rgb[0] * scale);
    rgb[1] = camera_clamp(rgb[1] * scale);
    rgb[2] = camera_clamp(rgb[2] * scale);
}

fn tone_highlights(y: f64, amount: f64) -> f64 {
    let t = camera_clamp((y - 0.5) / 0.5);
    let weight = t * t;
    if amount >= 0.0 {
        camera_clamp(y + amount * weight * (1.0 - y))
    } else {
        camera_clamp(y + amount * weight * (y - 0.5))
    }
}

fn tone_shadows(y: f64, amount: f64) -> f64 {
    let t = camera_clamp((0.5 - y) / 0.5);
    let weight = t * t;
    if amount >= 0.0 {
        camera_clamp(y + amount * weight * (0.5 - y))
    } else {
        camera_clamp(y + amount * weight * y)
    }
}

/// The top quarter is the white point: +1 maps 0.875 to 1, −1 pulls everything above 0.75 down to 0.75.
fn tone_whites(y: f64, amount: f64) -> f64 {
    if y <= 0.75 {
        return y;
    }
    camera_clamp(0.75 + (y - 0.75) * (1.0 + amount))
}

/// The bottom quarter is the black point. Negative amounts crush toward 0; positive ones lift toward 0.25.
fn tone_blacks(y: f64, amount: f64) -> f64 {
    if y >= 0.25 {
        return y;
    }
    camera_clamp(0.25 + (y - 0.25) * (1.0 - amount))
}

fn vibrance_and_saturation(rgb: &mut [f64; 3], vibrance: f64, saturation: f64) {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let mut lum = rec709(r, g, b);
    let maxc = r.max(g.max(b));
    let minc = r.min(g.min(b));
    let chroma = maxc - minc;
    let sat = if maxc <= 1e-8 { 0.0 } else { chroma / maxc };
    let mut hue = 0.0;
    if chroma > 1e-8 {
        if r >= g && r >= b {
            hue = 60.0 * ((g - b) / chroma % 6.0);
        } else if g >= r && g >= b {
            hue = 60.0 * ((b - r) / chroma + 2.0);
        } else {
            hue = 60.0 * ((r - g) / chroma + 4.0);
        }
        if hue < 0.0 {
            hue += 360.0;
        }
    }
    let mut skin = 0.0;
    if (10.0..=50.0).contains(&hue) {
        skin = if hue <= 30.0 {
            (hue - 10.0) / 20.0
        } else {
            (50.0 - hue) / 20.0
        };
        skin *= camera_clamp((sat - 0.15) / 0.35);
    }
    let mut amount = vibrance * (1.0 - sat);
    if vibrance > 0.0 {
        amount *= 1.0 - 0.7 * skin;
    }
    let mut factor = 1.0 + amount;
    rgb[0] = camera_clamp(lum + (rgb[0] - lum) * factor);
    rgb[1] = camera_clamp(lum + (rgb[1] - lum) * factor);
    rgb[2] = camera_clamp(lum + (rgb[2] - lum) * factor);
    lum = rec709(rgb[0], rgb[1], rgb[2]);
    factor = 1.0 + saturation;
    rgb[0] = camera_clamp(lum + (rgb[0] - lum) * factor);
    rgb[1] = camera_clamp(lum + (rgb[1] - lum) * factor);
    rgb[2] = camera_clamp(lum + (rgb[2] - lum) * factor);
}

#[inline]
fn write_premultiplied(p: &mut [u8], rgb: [f64; 3], alpha: f64) {
    p[0] = (rgb[0] * alpha).round().min(alpha).max(0.0) as u8;
    p[1] = (rgb[1] * alpha).round().min(alpha).max(0.0) as u8;
    p[2] = (rgb[2] * alpha).round().min(alpha).max(0.0) as u8;
}

/// Unpremultiplied straight color of a pixel, clamped to 1 like the C kernels.
#[inline]
fn straight_at(p: &[u8], alpha: f64) -> [f64; 3] {
    [
        (p[0] as f64 / alpha).min(1.0),
        (p[1] as f64 / alpha).min(1.0),
        (p[2] as f64 / alpha).min(1.0),
    ]
}

#[inline]
fn row_straight(p: &[u8], alpha: f64) -> [f64; 3] {
    let full = 255.0f64;
    [
        full.min(p[0] as f64 * 255.0 / alpha) / 255.0,
        full.min(p[1] as f64 * 255.0 / alpha) / 255.0,
        full.min(p[2] as f64 * 255.0 / alpha) / 255.0,
    ]
}

pub fn adjust_camera_raw(
    bitmap: &mut Bitmap,
    red_gain: f64,
    green_gain: f64,
    blue_gain: f64,
    exposure: f64,
    contrast: f64,
    highlights: f64,
    shadows: f64,
    whites: f64,
    blacks: f64,
    vibrance: f64,
    saturation: f64,
    clipping: i32,
) {
    let light = exposure.exp2();
    let contrast_scale = 1.0 + contrast / 100.0;
    let highlight_amount = highlights / 100.0;
    let shadow_amount = shadows / 100.0;
    let white_amount = whites / 100.0;
    let black_amount = blacks / 100.0;
    let vibrance_amount = vibrance / 100.0;
    let saturation_amount = saturation / 100.0;
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f64;
        if alpha == 0.0 {
            continue;
        }
        let rgb = row_straight(p, alpha);
        let mut r = camera_clamp(srgb_to_linear(rgb[0]) * red_gain * light);
        let mut g = camera_clamp(srgb_to_linear(rgb[1]) * green_gain * light);
        let mut b = camera_clamp(srgb_to_linear(rgb[2]) * blue_gain * light);
        r = camera_clamp(0.5 + (linear_to_srgb(r) - 0.5) * contrast_scale);
        g = camera_clamp(0.5 + (linear_to_srgb(g) - 0.5) * contrast_scale);
        b = camera_clamp(0.5 + (linear_to_srgb(b) - 0.5) * contrast_scale);
        let mut rgb = [r, g, b];
        let mut y = rec709(rgb[0], rgb[1], rgb[2]);
        scale_luminance(&mut rgb, tone_highlights(y, highlight_amount));
        y = rec709(rgb[0], rgb[1], rgb[2]);
        scale_luminance(&mut rgb, tone_shadows(y, shadow_amount));
        y = rec709(rgb[0], rgb[1], rgb[2]);
        scale_luminance(&mut rgb, tone_whites(y, white_amount));
        y = rec709(rgb[0], rgb[1], rgb[2]);
        scale_luminance(&mut rgb, tone_blacks(y, black_amount));
        vibrance_and_saturation(&mut rgb, vibrance_amount, saturation_amount);
        if clipping == 1 {
            let rc = rgb[0] >= 254.5 / 255.0;
            let gc = rgb[1] >= 254.5 / 255.0;
            let bc = rgb[2] >= 254.5 / 255.0;
            rgb[0] = if rc { 1.0 } else { 0.0 };
            rgb[1] = if gc { 1.0 } else { 0.0 };
            rgb[2] = if bc { 1.0 } else { 0.0 };
        } else if clipping == 2 {
            let rc = rgb[0] <= 0.5 / 255.0;
            let gc = rgb[1] <= 0.5 / 255.0;
            let bc = rgb[2] <= 0.5 / 255.0;
            if rc || gc || bc {
                rgb[0] = if rc { 0.0 } else { 1.0 };
                rgb[1] = if gc { 0.0 } else { 1.0 };
                rgb[2] = if bc { 0.0 } else { 1.0 };
            } else {
                rgb = [1.0; 3];
            }
        }
        write_premultiplied(p, rgb, alpha);
    }
}

#[inline]
fn clamped_index(index: isize, limit: usize) -> usize {
    if index < 0 {
        return 0;
    }
    if index as usize >= limit {
        return limit - 1;
    }
    index as usize
}

/// Edge-clamped box blur (`box_blur_plane`). `dst` may not alias `src`.
fn box_blur_plane(src: &[f32], dst: &mut [f32], width: usize, height: usize, radius: i32) {
    if radius < 1 {
        dst.copy_from_slice(src);
        return;
    }
    let mut temp = vec![0.0f32; width * height];
    let window = (radius * 2 + 1) as f64;
    let radius = radius as isize;
    for y in 0..height {
        let mut sum = 0.0f64;
        for k in -radius..=radius {
            sum += src[y * width + clamped_index(k, width)] as f64;
        }
        for x in 0..width {
            temp[y * width + x] = (sum / window) as f32;
            sum += src[y * width + clamped_index(x as isize + radius + 1, width)] as f64;
            sum -= src[y * width + clamped_index(x as isize - radius, width)] as f64;
        }
    }
    for x in 0..width {
        let mut sum = 0.0f64;
        for k in -radius..=radius {
            sum += temp[clamped_index(k, height) * width + x] as f64;
        }
        for y in 0..height {
            dst[y * width + x] = (sum / window) as f32;
            sum += temp[clamped_index(y as isize + radius + 1, height) * width + x] as f64;
            sum -= temp[clamped_index(y as isize - radius, height) * width + x] as f64;
        }
    }
}

fn effects_radius(base: f64, scale: f64) -> i32 {
    let mut radius = base * if scale > 0.0 { scale } else { 1.0 };
    if radius < 1.0 {
        radius = 1.0;
    }
    if radius > 64.0 {
        radius = 64.0;
    }
    radius.round() as i32
}

fn effects_dehaze(rgb: &mut [f64; 3], amount: f64) {
    let d = amount / 100.0;
    let y = rec709(rgb[0], rgb[1], rgb[2]);
    let contrast = 1.0 + 0.8 * d;
    let pivot = 0.45 - 0.1 * if d > 0.0 { d } else { 0.0 };
    let mut y2 = camera_clamp(pivot + (y - 0.45) * contrast);
    if d < 0.0 {
        y2 = camera_clamp(y2 + (-d) * (1.0 - y2) * 0.45);
    } else {
        y2 = camera_clamp(y2 - d * (0.4 - y2).max(0.0));
    }
    scale_luminance(rgb, y2);
    let y2 = rec709(rgb[0], rgb[1], rgb[2]);
    let sat = 1.0 + 0.7 * d;
    rgb[0] = camera_clamp(y2 + (rgb[0] - y2) * sat);
    rgb[1] = camera_clamp(y2 + (rgb[1] - y2) * sat);
    rgb[2] = camera_clamp(y2 + (rgb[2] - y2) * sat);
}

fn effects_vignette(
    rgb: &mut [f64; 3],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    amount: f64,
    midpoint: f64,
    roundness: f64,
    feather: f64,
    highlights: f64,
    style: i32,
) {
    if amount == 0.0 || width == 0 || height == 0 {
        return;
    }
    let nx = ((x as f64 + 0.5) / width as f64) * 2.0 - 1.0;
    let ny = ((y as f64 + 0.5) / height as f64) * 2.0 - 1.0;
    let square = nx.abs().max(ny.abs());
    let circle = nx.hypot(ny) / 2.0f64.sqrt();
    let shape = (1.0 - roundness / 100.0) * 0.5;
    let dist = circle + (square - circle) * shape;
    let start = (midpoint / 100.0) * 0.85;
    let mut soft = feather / 100.0;
    if soft < 0.05 {
        soft = 0.05;
    }
    let mut t = (dist - start) / soft;
    t = camera_clamp(t);
    let mask = t * t * (3.0 - 2.0 * t);
    let mut effect = (amount / 100.0) * mask;
    // Highlight Priority eases a darkening vignette off bright pixels. The other styles do not.
    if effect < 0.0 && style == 0 {
        let bright = camera_clamp((rec709(rgb[0], rgb[1], rgb[2]) - 0.45) / 0.55);
        effect *= 1.0 - (highlights / 100.0) * bright;
    }
    if effect < 0.0 {
        let factor = 1.0 + effect;
        rgb[0] *= factor;
        rgb[1] *= factor;
        rgb[2] *= factor;
    } else if effect > 0.0 {
        rgb[0] = rgb[0] + (1.0 - rgb[0]) * effect;
        rgb[1] = rgb[1] + (1.0 - rgb[1]) * effect;
        rgb[2] = rgb[2] + (1.0 - rgb[2]) * effect;
    }
    if style == 1 && mask > 0.0 {
        let lum = rec709(rgb[0], rgb[1], rgb[2]);
        let sat = 1.0 - 0.75 * mask * (amount / 100.0).abs();
        rgb[0] = camera_clamp(lum + (rgb[0] - lum) * sat);
        rgb[1] = camera_clamp(lum + (rgb[1] - lum) * sat);
        rgb[2] = camera_clamp(lum + (rgb[2] - lum) * sat);
    }
}

/// Paints clipped shadows blue and clipped highlights red
/// (`adjust_camera_raw_clip_overlay`).
pub fn adjust_camera_raw_clip_overlay(bitmap: &mut Bitmap, shadows: i32, highlights: i32) {
    if shadows == 0 && highlights == 0 {
        return;
    }
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f64;
        if alpha == 0.0 {
            continue;
        }
        let mut rgb = straight_at(p, alpha);
        if shadows != 0 && (rgb[0] <= 0.5 / 255.0 || rgb[1] <= 0.5 / 255.0 || rgb[2] <= 0.5 / 255.0)
        {
            rgb[0] *= 0.35;
            rgb[1] *= 0.35;
            rgb[2] = rgb[2] * 0.35 + 0.65;
        }
        if highlights != 0
            && (rgb[0] >= 254.5 / 255.0 || rgb[1] >= 254.5 / 255.0 || rgb[2] >= 254.5 / 255.0)
        {
            rgb[0] = rgb[0] * 0.35 + 0.65;
            rgb[1] *= 0.35;
            rgb[2] *= 0.35;
        }
        write_premultiplied(p, rgb, alpha);
    }
}

pub fn adjust_camera_raw_effects(
    bitmap: &mut Bitmap,
    texture: f64,
    clarity: f64,
    dehaze: f64,
    glow: f64,
    glow_style: i32,
    glow_range: f64,
    glow_spread: f64,
    glow_warmth: f64,
    vignette_amount: f64,
    vignette_midpoint: f64,
    vignette_roundness: f64,
    vignette_feather: f64,
    vignette_highlights: f64,
    vignette_style: i32,
    scale: f64,
) {
    let width = bitmap.width;
    let height = bitmap.height;
    if width == 0 || height == 0 {
        return;
    }
    if texture == 0.0 && clarity == 0.0 && dehaze == 0.0 && !(glow > 0.0) && vignette_amount == 0.0
    {
        return;
    }
    let count = width * height;
    let mut fine: Option<Vec<f32>> = None;
    let mut coarse: Option<Vec<f32>> = None;
    let mut glow_plane: Option<Vec<f32>> = None;
    if texture != 0.0 || clarity != 0.0 || glow > 0.0 {
        // The C keeps this plane alive for the whole kernel; only the
        // blurred derivatives are read back per pixel.
        let mut plane = vec![0.0f32; count];
        for y in 0..height {
            for x in 0..width {
                let p = &bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
                let alpha = p[3] as f64;
                if alpha == 0.0 {
                    plane[y * width + x] = 0.0;
                    continue;
                }
                let rgb = straight_at(p, alpha);
                plane[y * width + x] = rec709(rgb[0], rgb[1], rgb[2]) as f32;
            }
        }
        if texture != 0.0 {
            let mut blurred = vec![0.0f32; count];
            box_blur_plane(
                &plane,
                &mut blurred,
                width,
                height,
                effects_radius(1.0, scale),
            );
            fine = Some(blurred);
        }
        if clarity != 0.0 {
            let mut blurred = vec![0.0f32; count];
            box_blur_plane(
                &plane,
                &mut blurred,
                width,
                height,
                effects_radius(4.0, scale),
            );
            coarse = Some(blurred);
        }
        if glow > 0.0 {
            let spread = glow_spread / 100.0;
            let base = if glow_style == 1 { 2.0 } else { 5.0 };
            let mut widened = base * (1.0 + spread);
            if widened < 1.0 {
                widened = 1.0;
            }
            let glow_radius = effects_radius(widened, scale);
            let threshold = (0.55 + 0.4 * (glow_range / 100.0)) as f32;
            let mut source = vec![0.0f32; count];
            let mut denom = 1.0f32 - threshold;
            if denom < 0.05f32 {
                denom = 0.05;
            }
            for (i, slot) in source.iter_mut().enumerate() {
                let mut t = (plane[i] - threshold) / denom;
                if t < 0.0 {
                    t = 0.0;
                }
                if t > 1.0 {
                    t = 1.0;
                }
                *slot = t;
            }
            let mut blurred = vec![0.0f32; count];
            box_blur_plane(&source, &mut blurred, width, height, glow_radius);
            glow_plane = Some(blurred);
        }
    }
    let warmth = glow_warmth / 100.0;
    let (glow_red, glow_green, glow_blue, glow_gain) = if glow_style == 2 {
        // Halation's fringe is red. Warmth pushes it further that way, rather than toward yellow or blue.
        (1.0, 0.35 - 0.3 * warmth, 0.2 - 0.2 * warmth, 1.0)
    } else {
        (
            0.75 + 0.25 * warmth,
            0.6 + 0.2 * warmth,
            0.75 - 0.6 * warmth,
            if glow_style == 1 { 1.4 } else { 1.0 },
        )
    };
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let p = &mut bitmap.data[index * 4..index * 4 + 4];
            let alpha = p[3] as f64;
            if alpha == 0.0 {
                continue;
            }
            let mut rgb = straight_at(p, alpha);
            if fine.is_some() || coarse.is_some() {
                let tone = rec709(rgb[0], rgb[1], rgb[2]);
                let mut detail = 0.0;
                if let Some(fine) = &fine {
                    detail += (texture / 100.0) * (tone - fine[index] as f64);
                }
                if let Some(coarse) = &coarse {
                    detail += (clarity / 100.0) * (tone - coarse[index] as f64);
                }
                if detail != 0.0 {
                    scale_luminance(&mut rgb, camera_clamp(tone + detail));
                }
            }
            if dehaze != 0.0 {
                effects_dehaze(&mut rgb, dehaze);
            }
            if glow > 0.0
                && let Some(glow_plane) = &glow_plane
            {
                let add = glow_plane[index] as f64 * (glow / 100.0) * glow_gain;
                rgb[0] = camera_clamp(rgb[0] + add * glow_red);
                rgb[1] = camera_clamp(rgb[1] + add * glow_green);
                rgb[2] = camera_clamp(rgb[2] + add * glow_blue);
            }
            effects_vignette(
                &mut rgb,
                x,
                y,
                width,
                height,
                vignette_amount,
                vignette_midpoint,
                vignette_roundness,
                vignette_feather,
                vignette_highlights,
                vignette_style,
            );
            write_premultiplied(p, rgb, alpha);
        }
    }
}

fn lut_at(lut: &[f32], value: f64) -> f64 {
    let scaled = camera_clamp(value) * 255.0;
    let lo = scaled as i32;
    let hi = if lo < 255 { lo + 1 } else { 255 };
    let t = scaled - lo as f64;
    lut[lo as usize] as f64 + (lut[hi as usize] as f64 - lut[lo as usize] as f64) * t
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let maxc = r.max(g.max(b));
    let minc = r.min(g.min(b));
    let l = (maxc + minc) * 0.5;
    let d = maxc - minc;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let mut h = if maxc == r {
        (g - b) / d % 6.0
    } else if maxc == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h /= 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    (h, s, l)
}

fn hue_to_rgb(p: f64, q: f64, t: f64) -> f64 {
    let mut t = t;
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s <= 1e-6 {
        return [l; 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    [
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    ]
}

fn circular_distance(a: f64, b: f64) -> f64 {
    let d = (a - b).abs();
    if d > 0.5 { 1.0 - d } else { d }
}

const MIXER_CENTERS: [f64; 8] = [
    0.0,
    30.0 / 360.0,
    60.0 / 360.0,
    120.0 / 360.0,
    180.0 / 360.0,
    240.0 / 360.0,
    270.0 / 360.0,
    300.0 / 360.0,
];

fn point_weight(h: f64, s: f64, l: f64, point: &[f32]) -> f64 {
    let hue_half = (point[6] as f64).max(0.01);
    let sat_half = (point[7] as f64).max(0.01);
    let lum_half = (point[8] as f64).max(0.01);
    let hue_w = 1.0 - circular_distance(h, point[0] as f64) / hue_half;
    let sat_w = 1.0 - (s - point[1] as f64).abs() / sat_half;
    let lum_w = 1.0 - (l - point[2] as f64).abs() / lum_half;
    if hue_w < 0.0 || sat_w < 0.0 || lum_w < 0.0 {
        return 0.0;
    }
    hue_w * sat_w * lum_w
}

pub fn adjust_camera_raw_curve_color(
    bitmap: &mut Bitmap,
    luma_lut: &[f32],
    red_lut: &[f32],
    green_lut: &[f32],
    blue_lut: &[f32],
    refine_saturation: f64,
    mixer: &[f32],
    point_count: i32,
    points: &[f32],
    grade: &[f32],
    blending: f64,
    balance: f64,
    visualize: i32,
) {
    // The four grading wheels' colors depend only on the constant grade
    // values; computing them per pixel re-ran the trig-heavy hsl_to_rgb
    // up to 4x for every pixel of the image.
    let wheel_colors: [[f64; 3]; 4] =
        std::array::from_fn(|wheel| hsl_to_rgb(grade[wheel * 3] as f64, 1.0, 0.5));
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f64;
        if alpha == 0.0 {
            continue;
        }
        let mut rgb = straight_at(p, alpha);
        let tone = rec709(rgb[0], rgb[1], rgb[2]);
        let mapped = lut_at(luma_lut, tone);
        scale_luminance(&mut rgb, mapped);
        if refine_saturation != 0.0 && tone > 1e-4 {
            let factor = 1.0 + refine_saturation * (mapped / tone - 1.0);
            let lum = rec709(rgb[0], rgb[1], rgb[2]);
            rgb[0] = camera_clamp(lum + (rgb[0] - lum) * factor);
            rgb[1] = camera_clamp(lum + (rgb[1] - lum) * factor);
            rgb[2] = camera_clamp(lum + (rgb[2] - lum) * factor);
        }
        rgb[0] = lut_at(red_lut, rgb[0]);
        rgb[1] = lut_at(green_lut, rgb[1]);
        rgb[2] = lut_at(blue_lut, rgb[2]);
        let (mut h, mut s, mut l) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
        let source_hue = h;
        let source_sat = s;
        let source_lum = l;
        let mut hue_delta = 0.0;
        let mut sat_delta = 0.0;
        let mut lum_delta = 0.0;
        let mut weight_sum = 0.0;
        for i in 0..8 {
            let dist = circular_distance(h, MIXER_CENTERS[i]);
            let w = 1.0 - dist / (40.0 / 360.0);
            if w <= 0.0 {
                continue;
            }
            hue_delta += mixer[i] as f64 * w * (30.0 / 360.0);
            sat_delta += mixer[8 + i] as f64 * w;
            lum_delta += mixer[16 + i] as f64 * w * 0.25;
            weight_sum += w;
        }
        if weight_sum > 1.0 {
            hue_delta /= weight_sum;
            sat_delta /= weight_sum;
            lum_delta /= weight_sum;
        }
        h += hue_delta;
        if h < 0.0 {
            h += 1.0;
        }
        if h >= 1.0 {
            h -= 1.0;
        }
        s = camera_clamp(s * (1.0 + sat_delta));
        l = camera_clamp(l + lum_delta);
        for i in 0..point_count {
            let base = i as usize * 9;
            let point = &points[base..base + 9];
            let w = point_weight(h, s, l, point);
            if w <= 0.0 {
                continue;
            }
            h += point[3] as f64 * w * (30.0 / 360.0);
            s = camera_clamp(s * (1.0 + point[4] as f64 * w));
            l = camera_clamp(l + point[5] as f64 * w * 0.25);
        }
        if h < 0.0 {
            h += 1.0;
        }
        if h >= 1.0 {
            h -= 1.0;
        }
        rgb = hsl_to_rgb(h, s, l);
        // Balance moves the crossover between the shadow and highlight wheels. Toward highlights
        // it has to move down, so more of the picture counts as highlight and the shadow wheel
        // loses its hold; the other sign strengthened the shadow tint it was meant to weaken.
        let split = 0.5 - balance * 0.2;
        let reach = 0.12 + blending * 0.38;
        let y = rec709(rgb[0], rgb[1], rgb[2]);
        let mut shadow_w = camera_clamp((split + reach - y) / (reach * 2.0).max(0.05));
        let mut highlight_w = camera_clamp((y - (split - reach)) / (reach * 2.0).max(0.05));
        let mut mid_w = camera_clamp(1.0 - (y - split).abs() / (0.35 + reach));
        let sum = shadow_w + mid_w + highlight_w;
        if sum > 1e-4 {
            shadow_w /= sum;
            mid_w /= sum;
            highlight_w /= sum;
        }
        let weights = [shadow_w, mid_w, highlight_w, 1.0];
        for wheel in 0..4usize {
            let _wh = grade[wheel * 3] as f64;
            let ws = grade[wheel * 3 + 1] as f64;
            let wl = grade[wheel * 3 + 2] as f64;
            let w = weights[wheel];
            if w <= 0.0 || (ws <= 0.0 && wl == 0.0) {
                continue;
            }
            let wheel_rgb = wheel_colors[wheel];
            rgb[0] = camera_clamp(rgb[0] + (wheel_rgb[0] - 0.5) * ws * w * 0.85);
            rgb[1] = camera_clamp(rgb[1] + (wheel_rgb[1] - 0.5) * ws * w * 0.85);
            rgb[2] = camera_clamp(rgb[2] + (wheel_rgb[2] - 0.5) * ws * w * 0.85);
            if wl != 0.0 {
                let lum = rec709(rgb[0], rgb[1], rgb[2]);
                scale_luminance(&mut rgb, camera_clamp(lum + wl * 0.25 * w));
            }
        }
        if visualize >= 0 && (visualize as usize) < point_count as usize {
            let base = visualize as usize * 9;
            let point = &points[base..base + 9];
            if point_weight(source_hue, source_sat, source_lum, point) <= 0.05 {
                rgb[0] *= 0.35;
                rgb[1] *= 0.35;
                rgb[2] *= 0.35;
            }
        }
        write_premultiplied(p, rgb, alpha);
    }
}

fn detail_radius(slider: f64, scale: f64) -> f64 {
    let base = 0.5 + (slider / 100.0) * 2.5;
    let mut radius = base * if scale > 0.0 { scale } else { 1.0 };
    if radius < 0.5 {
        radius = 0.5;
    }
    if radius > 64.0 {
        radius = 64.0;
    }
    radius
}

fn pixel_hue_deg(r: f64, g: f64, b: f64) -> f64 {
    let maxc = r.max(g.max(b));
    let minc = r.min(g.min(b));
    let chroma = maxc - minc;
    if chroma < 1e-6 {
        return 0.0;
    }
    let mut hue = if maxc == r {
        (g - b) / chroma % 6.0
    } else if maxc == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    hue *= 60.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    hue
}

fn hue_in_range(hue: f64, low: f64, high: f64) -> bool {
    if low <= high {
        hue >= low && hue <= high
    } else {
        hue >= low || hue <= high
    }
}

fn sharpen_edge_at(
    luma: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    radius: i32,
) -> f32 {
    let radius = radius.max(1) as isize;
    let center = luma[y * width + x];
    let mut sum = 0.0f32;
    let mut count = 0i32;
    for &dy in &[-radius, 0, radius] {
        for &dx in &[-radius, 0, radius] {
            if dx == 0 && dy == 0 {
                continue;
            }
            let sx = x as isize + dx;
            let sy = y as isize + dy;
            if sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
                continue;
            }
            sum += (luma[sy as usize * width + sx as usize] - center).abs();
            count += 1;
        }
    }
    if count > 0 { sum / count as f32 } else { 0.0 }
}

pub fn adjust_camera_raw_sharpen_mask_overlay(
    bitmap: &mut Bitmap,
    sharpen_radius: f64,
    sharpen_detail: f64,
    sharpen_masking: f64,
    scale: f64,
) {
    let width = bitmap.width;
    let height = bitmap.height;
    if width == 0 || height == 0 {
        return;
    }
    let count = width * height;
    let mut luma = vec![0.0f32; count];
    for y in 0..height {
        for x in 0..width {
            let p = &bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
            let alpha = p[3] as f64;
            if alpha == 0.0 {
                luma[y * width + x] = 0.0;
                continue;
            }
            let rgb = straight_at(p, alpha);
            luma[y * width + x] = rec709(rgb[0], rgb[1], rgb[2]) as f32;
        }
    }
    let radius = effects_radius(detail_radius(sharpen_radius, scale), 1.0);
    let threshold = (sharpen_masking / 100.0) * 0.35;
    let detail_boost = 0.5 + sharpen_detail / 100.0;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let p = &mut bitmap.data[index * 4..index * 4 + 4];
            let alpha = p[3] as f64;
            if alpha == 0.0 {
                continue;
            }
            let edge = sharpen_edge_at(&luma, width, height, x, y, radius);
            let mask = camera_clamp(
                (edge as f64 * detail_boost - threshold) / (0.35 - threshold * 0.5).max(0.04),
            );
            let gray = (mask * alpha).round() as i64 as u8;
            p[0] = gray;
            p[1] = gray;
            p[2] = gray;
        }
    }
}

pub fn adjust_camera_raw_detail(
    bitmap: &mut Bitmap,
    sharpen_amount: f64,
    sharpen_radius: f64,
    sharpen_detail: f64,
    sharpen_masking: f64,
    noise_luminance: f64,
    noise_luminance_detail: f64,
    noise_luminance_contrast: f64,
    noise_color: f64,
    noise_color_detail: f64,
    noise_color_smoothness: f64,
    scale: f64,
) {
    let width = bitmap.width;
    let height = bitmap.height;
    if width == 0 || height == 0 {
        return;
    }
    if sharpen_amount == 0.0 && noise_luminance == 0.0 && noise_color == 0.0 {
        return;
    }
    let count = width * height;
    let mut luma = vec![0.0f32; count];
    for y in 0..height {
        for x in 0..width {
            let p = &bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
            let alpha = p[3] as f64;
            if alpha == 0.0 {
                luma[y * width + x] = 0.0;
                continue;
            }
            let rgb = straight_at(p, alpha);
            luma[y * width + x] = rec709(rgb[0], rgb[1], rgb[2]) as f32;
        }
    }
    let mut work = vec![0.0f32; count];
    if noise_luminance > 0.0 {
        let radius = effects_radius(1.0 + noise_luminance / 50.0, scale);
        box_blur_plane(&luma, &mut work, width, height, radius);
        let strength = noise_luminance / 100.0;
        let preserve = noise_luminance_detail / 100.0;
        let contrast = noise_luminance_contrast / 100.0;
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let p = &mut bitmap.data[index * 4..index * 4 + 4];
                let alpha = p[3] as f64;
                if alpha == 0.0 {
                    continue;
                }
                let edge = sharpen_edge_at(&luma, width, height, x, y, 1);
                let local = strength * (1.0 - preserve * (edge as f64 * 6.0).min(1.0));
                let blurred = work[index];
                let mut target =
                    (luma[index] as f64 * (1.0 - local) + blurred as f64 * local) as f32;
                if contrast != 0.0 {
                    target = (target as f64
                        + contrast * 0.25 * (luma[index] as f64 - blurred as f64))
                        as f32;
                }
                luma[index] = target;
                let mut rgb = straight_at(p, alpha);
                scale_luminance(&mut rgb, target as f64);
                write_premultiplied(p, rgb, alpha);
            }
        }
    }
    if noise_color > 0.0 {
        let radius = effects_radius(1.0 + noise_color_smoothness / 40.0, scale);
        let mut chroma = vec![0.0f32; count];
        for y in 0..height {
            for x in 0..width {
                let p = &bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
                let alpha = p[3] as f64;
                if alpha == 0.0 {
                    continue;
                }
                let rgb = straight_at(p, alpha);
                let (_, s, _) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
                chroma[y * width + x] = s as f32;
            }
        }
        let mut chroma_blur = vec![0.0f32; count];
        box_blur_plane(&chroma, &mut chroma_blur, width, height, radius);
        let strength = noise_color / 100.0;
        let preserve = noise_color_detail / 100.0;
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let p = &mut bitmap.data[index * 4..index * 4 + 4];
                let alpha = p[3] as f64;
                if alpha == 0.0 {
                    continue;
                }
                let edge = (chroma[index] - chroma_blur[index]).abs();
                let local = strength * (1.0 - preserve * (edge as f64 * 4.0).min(1.0));
                let sat = chroma[index] * (1.0 - local as f32) + chroma_blur[index] * local as f32;
                let mut rgb = straight_at(p, alpha);
                let (h, _, l) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
                rgb = hsl_to_rgb(h, sat as f64, l);
                write_premultiplied(p, rgb, alpha);
            }
        }
    }
    if sharpen_amount > 0.0 {
        for y in 0..height {
            for x in 0..width {
                let p = &bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
                let alpha = p[3] as f64;
                if alpha == 0.0 {
                    continue;
                }
                let rgb = straight_at(p, alpha);
                luma[y * width + x] = rec709(rgb[0], rgb[1], rgb[2]) as f32;
            }
        }
        let radius = effects_radius(detail_radius(sharpen_radius, scale), 1.0);
        box_blur_plane(&luma, &mut work, width, height, radius);
        let amount = sharpen_amount / 100.0;
        let detail_mix = sharpen_detail / 100.0;
        let threshold = (sharpen_masking / 100.0) * 0.35;
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let p = &mut bitmap.data[index * 4..index * 4 + 4];
                let alpha = p[3] as f64;
                if alpha == 0.0 {
                    continue;
                }
                let edge = sharpen_edge_at(&luma, width, height, x, y, radius);
                let mask = camera_clamp(
                    (edge as f64 * (0.5 + detail_mix) - threshold)
                        / (0.35 - threshold * 0.5).max(0.04),
                );
                let high = (luma[index] - work[index]) as f64;
                let sharpened =
                    camera_clamp(luma[index] as f64 + high * amount * mask * (0.5 + detail_mix));
                let mut rgb = straight_at(p, alpha);
                scale_luminance(&mut rgb, sharpened);
                write_premultiplied(p, rgb, alpha);
            }
        }
    }
}

fn optics_defringe(
    rgb: &mut [f64; 3],
    purple_amount: f64,
    purple_low: f64,
    purple_high: f64,
    green_amount: f64,
    green_low: f64,
    green_high: f64,
) {
    let hue = pixel_hue_deg(rgb[0], rgb[1], rgb[2]);
    let maxc = rgb[0].max(rgb[1].max(rgb[2]));
    let minc = rgb[0].min(rgb[1].min(rgb[2]));
    let chroma = maxc - minc;
    if chroma < 1e-6 {
        return;
    }
    let sat = chroma / maxc;
    let mut reduce: f64 = 0.0;
    if purple_amount > 0.0 && hue_in_range(hue, purple_low, purple_high) {
        reduce = reduce.max(purple_amount / 100.0);
    }
    if green_amount > 0.0 && hue_in_range(hue, green_low, green_high) {
        reduce = reduce.max(green_amount / 100.0);
    }
    if reduce <= 0.0 {
        return;
    }
    let lum = rec709(rgb[0], rgb[1], rgb[2]);
    let factor = 1.0 - reduce * sat;
    rgb[0] = camera_clamp(lum + (rgb[0] - lum) * factor);
    rgb[1] = camera_clamp(lum + (rgb[1] - lum) * factor);
    rgb[2] = camera_clamp(lum + (rgb[2] - lum) * factor);
}

fn optics_chromatic(bitmap: &mut Bitmap, strength: f64) {
    if strength <= 0.0 {
        return;
    }
    let width = bitmap.width;
    let height = bitmap.height;
    let copy = bitmap.data.clone();
    let cx = width as f64 * 0.5;
    let cy = height as f64 * 0.5;
    let max_r = cx.hypot(cy);
    for y in 0..height {
        for x in 0..width {
            let p = &mut bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
            let alpha = p[3] as f64;
            if alpha == 0.0 {
                continue;
            }
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            let radial = dx.hypot(dy) / max_r;
            let shift = strength * radial * radial * 2.5;
            let rx = (x as f64 - shift).round() as isize;
            let bx = (x as f64 + shift).round() as isize;
            let pr = &copy[(y * width + clamped_index(rx, width)) * 4
                ..(y * width + clamped_index(rx, width)) * 4 + 4];
            let pb = &copy[(y * width + clamped_index(bx, width)) * 4
                ..(y * width + clamped_index(bx, width)) * 4 + 4];
            let g = (copy[(y * width + x) * 4 + 1] as f64 / alpha).min(1.0);
            let r = (pr[0] as f64 / (pr[3] as f64).max(1.0)).min(1.0);
            let b = (pb[2] as f64 / (pb[3] as f64).max(1.0)).min(1.0);
            write_premultiplied(p, [r, g, b], alpha);
        }
    }
}

fn optics_vignette_correct(
    rgb: &mut [f64; 3],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    amount: f64,
    midpoint: f64,
) {
    if amount == 0.0 || width == 0 || height == 0 {
        return;
    }
    let nx = ((x as f64 + 0.5) / width as f64) * 2.0 - 1.0;
    let ny = ((y as f64 + 0.5) / height as f64) * 2.0 - 1.0;
    let dist = nx.hypot(ny) / 2.0f64.sqrt();
    let start = (midpoint / 100.0) * 0.85;
    let t = camera_clamp((dist - start) / 0.35);
    let mask = t * t * (3.0 - 2.0 * t);
    let lift = (amount / 100.0) * mask;
    if lift > 0.0 {
        rgb[0] = camera_clamp(rgb[0] + (1.0 - rgb[0]) * lift);
        rgb[1] = camera_clamp(rgb[1] + (1.0 - rgb[1]) * lift);
        rgb[2] = camera_clamp(rgb[2] + (1.0 - rgb[2]) * lift);
    } else {
        let factor = 1.0 + lift;
        rgb[0] *= factor;
        rgb[1] *= factor;
        rgb[2] *= factor;
    }
}

pub fn adjust_camera_raw_optics(
    bitmap: &mut Bitmap,
    remove_chromatic: i32,
    lens_profile: i32,
    profile_distortion: f64,
    profile_vignetting: f64,
    distortion_k: f64,
    purple_amount: f64,
    purple_hue_low: f64,
    purple_hue_high: f64,
    green_amount: f64,
    green_hue_low: f64,
    green_hue_high: f64,
    vignette_amount: f64,
    vignette_midpoint: f64,
    scale: f64,
) {
    let width = bitmap.width;
    let height = bitmap.height;
    if width == 0 || height == 0 {
        return;
    }
    // `profile_distortion` and `scale` are part of the C signature but unused there too.
    let _ = (profile_distortion, scale);
    let profile_vignette = if lens_profile != 0 {
        profile_vignetting / 100.0
    } else {
        0.0
    };
    let vignette = vignette_amount + profile_vignette * 35.0;
    if distortion_k != 0.0 {
        *bitmap = crate::filters::lens_distort(bitmap, distortion_k);
    }
    if remove_chromatic != 0 {
        optics_chromatic(bitmap, 0.45);
    }
    if purple_amount == 0.0 && green_amount == 0.0 && vignette == 0.0 {
        return;
    }
    for y in 0..height {
        for x in 0..width {
            let p = &mut bitmap.data[(y * width + x) * 4..(y * width + x) * 4 + 4];
            let alpha = p[3] as f64;
            if alpha == 0.0 {
                continue;
            }
            let mut rgb = straight_at(p, alpha);
            optics_defringe(
                &mut rgb,
                purple_amount,
                purple_hue_low,
                purple_hue_high,
                green_amount,
                green_hue_low,
                green_hue_high,
            );
            optics_vignette_correct(&mut rgb, x, y, width, height, vignette, vignette_midpoint);
            write_premultiplied(p, rgb, alpha);
        }
    }
}

pub fn adjust_camera_raw_calibration(
    bitmap: &mut Bitmap,
    shadow_tint: f64,
    red_hue: f64,
    red_saturation: f64,
    green_hue: f64,
    green_saturation: f64,
    blue_hue: f64,
    blue_saturation: f64,
    process_version: i32,
) {
    let width = bitmap.width;
    let height = bitmap.height;
    if width == 0 || height == 0 {
        return;
    }
    let version_scale = if process_version <= 1 {
        0.55
    } else if process_version == 2 {
        0.65
    } else if process_version == 3 {
        0.75
    } else if process_version == 4 {
        0.85
    } else if process_version == 5 {
        0.92
    } else {
        1.0
    };
    let tint = shadow_tint / 100.0 * version_scale;
    let rh = red_hue / 100.0 * (15.0 / 360.0) * version_scale;
    let rs = red_saturation / 100.0 * 0.45 * version_scale;
    let gh = green_hue / 100.0 * (15.0 / 360.0) * version_scale;
    let gs = green_saturation / 100.0 * 0.45 * version_scale;
    let bh = blue_hue / 100.0 * (15.0 / 360.0) * version_scale;
    let bs = blue_saturation / 100.0 * 0.45 * version_scale;
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f64;
        if alpha == 0.0 {
            continue;
        }
        let src = straight_at(p, alpha);
        let (mut h, mut s, l) = rgb_to_hsl(src[0], src[1], src[2]);
        if l < 0.35 && tint != 0.0 {
            h += tint * 0.06;
            if h < 0.0 {
                h += 1.0;
            }
            if h >= 1.0 {
                h -= 1.0;
            }
        }
        let maxc = src[0].max(src[1].max(src[2]));
        let minc = src[0].min(src[1].min(src[2]));
        if maxc - minc > 1e-5 {
            if src[0] >= src[1] && src[0] >= src[2] {
                h += rh;
                s = camera_clamp(s * (1.0 + rs));
            } else if src[1] >= src[0] && src[1] >= src[2] {
                h += gh;
                s = camera_clamp(s * (1.0 + gs));
            } else {
                h += bh;
                s = camera_clamp(s * (1.0 + bs));
            }
            if h < 0.0 {
                h += 1.0;
            }
            if h >= 1.0 {
                h -= 1.0;
            }
        }
        let rgb = hsl_to_rgb(h, s, l);
        write_premultiplied(p, rgb, alpha);
    }
}

/// Warps the image so its corners land on `corners`
/// (Core Image's `CIPerspectiveTransform`): bilinear sampling through the
/// inverse homography. Corners are [top left, top right, bottom right,
/// bottom left] with y measured upward from the bottom. Samples whose source
/// coordinate falls off the image come out transparent (premultiplied-clear
/// black), like Core Image — no edge smear.
fn perspective_warp(image: &Bitmap, corners: &[(f64, f64); 4]) -> Bitmap {
    let w = image.width as f64;
    let h = image.height as f64;
    // To raster coordinates, y measured downward from the top.
    let p00 = (corners[0].0, h - corners[0].1);
    let p10 = (corners[1].0, h - corners[1].1);
    let p11 = (corners[2].0, h - corners[2].1);
    let p01 = (corners[3].0, h - corners[3].1);
    // Forward homography: input pixel (0,0) → p00, (w,0) → p10, (w,h) → p11, (0,h) → p01.
    let dx1 = p10.0 - p11.0;
    let dy1 = p10.1 - p11.1;
    let dx2 = p01.0 - p11.0;
    let dy2 = p01.1 - p11.1;
    let sx = p00.0 - p10.0 + p11.0 - p01.0;
    let sy = p00.1 - p10.1 + p11.1 - p01.1;
    let den = dx1 * dy2 - dy1 * dx2;
    let mut out = Bitmap::new(image.width, image.height);
    if den.abs() < 1e-12 {
        return image.clone();
    }
    let g = (sx * dy2 - sy * dx2) / den;
    let hhom = (dx1 * sy - dy1 * sx) / den;
    let a = p10.0 - p00.0 + g * p10.0;
    let b = p01.0 - p00.0 + hhom * p01.0;
    let c = p00.0;
    let d = p10.1 - p00.1 + g * p10.1;
    let e = p01.1 - p00.1 + hhom * p01.1;
    let f = p00.1;
    // Normalized (u,v) → quad; as a 3×3 on pixel coordinates.
    let forward = [[a / w, b / h, c], [d / w, e / h, f], [g / w, hhom / h, 1.0]];
    let Some(inverse) = invert3(&forward) else {
        return image.clone();
    };
    for y in 0..image.height {
        for x in 0..image.width {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let sw = inverse[2][0] * px + inverse[2][1] * py + inverse[2][2];
            if sw.abs() < 1e-12 {
                continue;
            }
            let sx = (inverse[0][0] * px + inverse[0][1] * py + inverse[0][2]) / sw;
            let sy = (inverse[1][0] * px + inverse[1][1] * py + inverse[1][2]) / sw;
            // Core Image leaves samples that miss the source transparent
            // instead of clamping to the edge pixels. Pixel centers live on
            // [0, w−1] × [0, h−1]; anything else stays premultiplied-clear.
            if sx < 0.0 || sy < 0.0 || sx > w - 1.0 || sy > h - 1.0 {
                continue;
            }
            let sample = bilinear_sample(image, sx, sy);
            out.set_pixel(x, y, sample);
        }
    }
    out
}

fn invert3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / det,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / det,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / det,
        ],
    ])
}

/// One RGB histogram and a hue/saturation vectorscope of the same graded pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraRawScope {
    pub red: Vec<f64>,
    pub green: Vec<f64>,
    pub blue: Vec<f64>,
    /// Density from the center outward. Index `y * SCOPE_SIDE + x`.
    pub vectorscope: Vec<f64>,
}

impl CameraRawScope {
    pub const BIN_COUNT: usize = 256;
    pub const SCOPE_SIDE: usize = 64;

    pub fn peak(&self) -> f64 {
        let peaks = [
            LevelsHistogramDisplay::scale(&self.red),
            LevelsHistogramDisplay::scale(&self.green),
            LevelsHistogramDisplay::scale(&self.blue),
        ];
        peaks.into_iter().fold(0.0f64, f64::max)
    }

    /// Counts the graded image. Fully transparent pixels are skipped (`make(_:)`).
    pub fn make(image: &Bitmap) -> Option<Self> {
        if image.width == 0 || image.height == 0 {
            return None;
        }
        let mut bins = [0.0f64; 1024];
        levels_histogram(image, None, &mut bins);
        let side = Self::SCOPE_SIDE;
        let mut scope = vec![0.0f64; side * side];
        for px in image.data.chunks_exact(4) {
            let alpha = px[3] as f64;
            if alpha == 0.0 {
                continue;
            }
            let red = (px[0] as f64 / alpha).min(1.0);
            let green = (px[1] as f64 / alpha).min(1.0);
            let blue = (px[2] as f64 / alpha).min(1.0);
            let max_channel = red.max(green.max(blue));
            let min_channel = red.min(green.min(blue));
            let chroma = max_channel - min_channel;
            if !(chroma > 1e-4 && max_channel > 1e-4) {
                continue;
            }
            let mut hue = if max_channel == red {
                (green - blue) / chroma
            } else if max_channel == green {
                2.0 + (blue - red) / chroma
            } else {
                4.0 + (red - green) / chroma
            };
            hue /= 6.0;
            if hue < 0.0 {
                hue += 1.0;
            }
            let angle = hue * 2.0 * std::f64::consts::PI;
            let saturation = chroma / max_channel;
            let plot_x = 0.5 + angle.cos() * saturation * 0.48;
            let plot_y = 0.5 + angle.sin() * saturation * 0.48;
            let column = ((plot_x * side as f64) as isize).clamp(0, side as isize - 1) as usize;
            let row = ((plot_y * side as f64) as isize).clamp(0, side as isize - 1) as usize;
            scope[row * side + column] += alpha / 255.0;
        }
        let mut red = vec![0.0; Self::BIN_COUNT];
        red.copy_from_slice(&bins[256..512]);
        let mut green = vec![0.0; Self::BIN_COUNT];
        green.copy_from_slice(&bins[512..768]);
        let mut blue = vec![0.0; Self::BIN_COUNT];
        blue.copy_from_slice(&bins[768..1024]);
        Some(Self {
            red,
            green,
            blue,
            vectorscope: scope,
        })
    }

    /// Paints clipped shadows blue and clipped highlights red. The histogram is
    /// counted before this (`overlay(_:shadows:highlights:)`).
    pub fn overlay(image: &Bitmap, shadows: bool, highlights: bool) -> Bitmap {
        clip_overlay(image, shadows, highlights)
    }
}

/// The Option-drag clipping paint: shadows blue, highlights red.
pub fn clip_overlay(image: &Bitmap, shadows: bool, highlights: bool) -> Bitmap {
    let mut result = image.clone();
    adjust_camera_raw_clip_overlay(&mut result, shadows as i32, highlights as i32);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgba: [u8; 4]) -> Bitmap {
        Bitmap::new_clearing(w, h, rgba)
    }

    /// Straight RGBA rows, top row first (`pixels(_:)`).
    fn straight_pixels(image: &Bitmap) -> Vec<[u8; 4]> {
        image
            .data
            .chunks_exact(4)
            .map(|p| {
                let a = p[3] as u32;
                if a == 0 {
                    return [0, 0, 0, 0];
                }
                let s = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
                [s(p[0]), s(p[1]), s(p[2]), p[3]]
            })
            .collect()
    }

    fn chroma(px: &[u8]) -> i32 {
        let (mut mx, mut mn) = (px[0], px[0]);
        for v in &px[..3] {
            mx = mx.max(*v);
            mn = mn.min(*v);
        }
        (mx - mn) as i32
    }

    fn peak_index(bins: &[f64]) -> usize {
        bins.iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(usize::MAX)
    }

    /// Wide step from 40 to 200 (`step()`), 24 × 4.
    fn step_image() -> Bitmap {
        let mut b = Bitmap::new(24, 4);
        for y in 0..4 {
            for x in 0..24 {
                let v = if x < 12 { 40 } else { 200 };
                b.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        b
    }

    fn checker(width: usize, height: usize) -> Bitmap {
        let mut b = Bitmap::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let light = ((x / 2) + (y / 2)) % 2 == 0;
                let v = if light { 230 } else { 26 };
                b.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        b
    }

    #[test]
    fn defaults_leave_pixels_and_alpha_alone() {
        let input = solid(4, 4, [64, 64, 64, 128]);
        let result = CameraRawSettings::default().apply(&input, None, 1.0, 0, -1, false);
        assert_eq!(straight_pixels(&result), straight_pixels(&input));
        let mut broken = CameraRawSettings::default();
        broken.exposure = f64::NAN;
        broken.temperature = 400.0;
        assert!(!broken.is_valid());
        assert_eq!(broken.normalized().exposure, 0.0);
        assert_eq!(broken.normalized().temperature, 100.0);
    }

    #[test]
    fn serde_round_trip_keeps_defaults() {
        let json = serde_json::to_string(&CameraRawSettings::default()).unwrap();
        let back: CameraRawSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CameraRawSettings::default());
        let mut settings = CameraRawSettings::default();
        settings.exposure = 1.0;
        settings.mixer.hue[0] = -40.0;
        settings.mixer.points.push(CameraRawPointColor {
            hue: 300.0,
            visualize: true,
            ..Default::default()
        });
        settings.geometry.guides.push(CameraRawGeometryGuide {
            start_x: 0.1,
            start_y: 0.2,
            end_x: 0.8,
            end_y: 0.9,
        });
        let json = serde_json::to_string(&settings).unwrap();
        let back: CameraRawSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn exposure_adds_one_stop_and_contrast_pivots_mid_gray() {
        let input = solid(4, 4, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.exposure = 1.0;
        let brighter = straight_pixels(&settings.apply(&input, None, 1.0, 0, -1, false))[0];
        assert!(
            (brighter[0] as i32 - 176).abs() <= 2,
            "+1 stop: {brighter:?}"
        );
        assert_eq!(brighter[0], brighter[1]);
        assert_eq!(brighter[1], brighter[2]);
        let translucent = solid(4, 4, [128, 128, 128, 128]);
        let kept = settings.apply(&translucent, None, 1.0, 0, -1, false);
        assert_eq!(straight_pixels(&kept)[0][3], 128);

        let mut pair = Bitmap::new(2, 1);
        pair.set_pixel(0, 0, [64, 64, 64, 255]);
        pair.set_pixel(1, 0, [192, 192, 192, 255]);
        settings = CameraRawSettings::default();
        settings.contrast = 100.0;
        let pushed = straight_pixels(&settings.apply(&pair, None, 1.0, 0, -1, false));
        assert!(
            pushed[0][0] < 10 && pushed[1][0] > 250,
            "contrast +100: {pushed:?}"
        );
        settings.contrast = -100.0;
        let flat = straight_pixels(&settings.apply(&pair, None, 1.0, 0, -1, false));
        assert!((flat[0][0] as i32 - 128).abs() <= 2 && (flat[1][0] as i32 - 128).abs() <= 2);
    }

    #[test]
    fn tonal_sliders_move_the_end_they_name() {
        let mut bright_and_mid = Bitmap::new(2, 1);
        bright_and_mid.set_pixel(0, 0, [230, 230, 230, 255]);
        bright_and_mid.set_pixel(1, 0, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.highlights = -100.0;
        let recovered = straight_pixels(&settings.apply(&bright_and_mid, None, 1.0, 0, -1, false));
        assert!(
            recovered[0][0] < 200,
            "highlights −100 darkens: {:?}",
            recovered[0]
        );
        assert!((recovered[1][0] as i32 - 128).abs() <= 2);

        settings = CameraRawSettings::default();
        settings.whites = 100.0;
        let clipped = straight_pixels(&settings.apply(&bright_and_mid, None, 1.0, 0, -1, false));
        assert_eq!(clipped[0][0], 255, "whites +100 clips");
        assert!((clipped[1][0] as i32 - 128).abs() <= 2);
        let viz = straight_pixels(&settings.apply(
            &bright_and_mid,
            Some(CameraRawClipping::Highlights),
            1.0,
            0,
            -1,
            false,
        ));
        assert_eq!(viz[0], [255, 255, 255, 255]);
        assert_eq!(
            viz[1],
            [0, 0, 0, 255],
            "highlight clipping is not the grade"
        );

        let mut dark_and_mid = Bitmap::new(2, 1);
        dark_and_mid.set_pixel(0, 0, [20, 20, 20, 255]);
        dark_and_mid.set_pixel(1, 0, [128, 128, 128, 255]);
        settings = CameraRawSettings::default();
        settings.shadows = 100.0;
        let opened = straight_pixels(&settings.apply(&dark_and_mid, None, 1.0, 0, -1, false));
        assert!(opened[0][0] > 50, "shadows +100 opens: {:?}", opened[0]);
        assert!((opened[1][0] as i32 - 128).abs() <= 2);

        settings = CameraRawSettings::default();
        settings.blacks = -100.0;
        let crushed = straight_pixels(&settings.apply(&dark_and_mid, None, 1.0, 0, -1, false));
        assert!(crushed[0][0] < 20, "blacks −100 crushes: {:?}", crushed[0]);
        assert!((crushed[1][0] as i32 - 128).abs() <= 2);
        let shadow_viz = straight_pixels(&settings.apply(
            &dark_and_mid,
            Some(CameraRawClipping::Shadows),
            1.0,
            0,
            -1,
            false,
        ));
        assert_eq!(shadow_viz[0], [0, 0, 0, 255]);
        assert_eq!(shadow_viz[1], [255, 255, 255, 255]);
    }

    #[test]
    fn temperature_warms_and_tint_moves_toward_magenta() {
        let input = solid(4, 4, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.temperature = 100.0;
        let warm = straight_pixels(&settings.apply(&input, None, 1.0, 0, -1, false))[0];
        assert!(
            warm[0] > 128 && warm[2] < 128 && warm[0] > warm[2],
            "warm: {warm:?}"
        );
        settings = CameraRawSettings::default();
        settings.tint = 100.0;
        let magenta = straight_pixels(&settings.apply(&input, None, 1.0, 0, -1, false))[0];
        assert!(magenta[1] < 128 && magenta[1] < magenta[0] && magenta[1] < magenta[2]);
    }

    #[test]
    fn vibrance_favors_dull_colors_and_saturation_doubles() {
        let dull_green = solid(4, 4, [77, 153, 77, 255]);
        let saturated_green = solid(4, 4, [20, 200, 20, 255]);
        let skin = solid(4, 4, [153, 115, 77, 255]);
        let mut settings = CameraRawSettings::default();
        settings.vibrance = 100.0;
        let graded = |img: &Bitmap| {
            let px = straight_pixels(&settings.apply(img, None, 1.0, 0, -1, false))[0];
            chroma(&px)
        };
        let dull_before = chroma(&straight_pixels(&dull_green)[0]);
        let saturated_before = chroma(&straight_pixels(&saturated_green)[0]);
        let skin_before = chroma(&straight_pixels(&skin)[0]);
        let dull_delta = graded(&dull_green) - dull_before;
        let saturated_delta = graded(&saturated_green) - saturated_before;
        let skin_delta = graded(&skin) - skin_before;
        assert_eq!(dull_before, skin_before);
        assert!(
            dull_delta > saturated_delta + 10,
            "dull {dull_delta} vs saturated {saturated_delta}"
        );
        assert!(
            dull_delta > skin_delta + 10,
            "skin protected: {skin_delta} vs {dull_delta}"
        );

        let red = solid(4, 4, [160, 120, 120, 255]);
        let blue = solid(4, 4, [100, 100, 140, 255]);
        settings = CameraRawSettings::default();
        settings.saturation = 100.0;
        let red_before = chroma(&straight_pixels(&red)[0]) as f64;
        let blue_before = chroma(&straight_pixels(&blue)[0]) as f64;
        let red_after =
            chroma(&straight_pixels(&settings.apply(&red, None, 1.0, 0, -1, false))[0]) as f64;
        let blue_after =
            chroma(&straight_pixels(&settings.apply(&blue, None, 1.0, 0, -1, false))[0]) as f64;
        let red_ratio = red_after / red_before;
        let blue_ratio = blue_after / blue_before;
        assert!((red_ratio - 2.0).abs() < 0.15 && (blue_ratio - 2.0).abs() < 0.15);
    }

    #[test]
    fn eyedropper_neutralizes_a_warm_pixel() {
        let warm = solid(8, 8, [160, 140, 120, 255]);
        let before = straight_pixels(&warm)[0];
        let solved =
            CameraRawSettings::neutralize_straight(160.0 / 255.0, 140.0 / 255.0, 120.0 / 255.0)
                .unwrap();
        let mut settings = CameraRawSettings::default();
        settings.temperature = solved.0;
        settings.tint = solved.1;
        let neutral = straight_pixels(&settings.apply(&warm, None, 1.0, 0, -1, false))[0];
        assert!(
            chroma(&neutral) < chroma(&before) / 2,
            "eyedropper pulls the cast in: {before:?} → {neutral:?}"
        );
        // A missing channel has no solution.
        assert!(CameraRawSettings::neutralize_linear(0.5, 0.0, 0.2).is_none());
        // Auto matches the solid color; a transparent layer leaves the sliders alone.
        let auto = CameraRawSettings::auto_balance(&warm).unwrap();
        let mut settings = CameraRawSettings::default();
        settings.temperature = auto.0;
        settings.tint = auto.1;
        let averaged = straight_pixels(&settings.apply(&warm, None, 1.0, 0, -1, false))[0];
        assert!(
            chroma(&averaged) < chroma(&before) / 2,
            "auto: {averaged:?}"
        );
        assert!(CameraRawSettings::auto_balance(&Bitmap::new(4, 4)).is_none());
    }

    #[test]
    fn hidden_group_is_left_out() {
        let input = solid(8, 8, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.exposure = 1.0;
        settings.temperature = 40.0;
        let hidden_light =
            settings.applying(false, true, true, true, true, true, true, true, true, true);
        let source = straight_pixels(&input);
        let color_only = straight_pixels(&hidden_light.apply(&input, None, 1.0, 0, -1, false));
        assert_ne!(color_only, source);
        assert_eq!(hidden_light.exposure, 0.0);
        assert_eq!(hidden_light.temperature, 40.0);
        let neither =
            settings.applying(false, false, true, true, true, true, true, true, true, true);
        assert!(neither.is_identity());
        let unchanged = straight_pixels(&neither.apply(&input, None, 1.0, 0, -1, false));
        assert_eq!(unchanged, source);
    }

    #[test]
    fn is_identity_tracks_each_group() {
        let mut s = CameraRawSettings::default();
        assert!(s.is_identity());
        s.exposure = 1.0;
        assert!(!s.is_identity() && s.adjusts_light());
        let mut s = CameraRawSettings::default();
        s.tint = 5.0;
        assert!(s.adjusts_color() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.glow = 10.0;
        assert!(s.adjusts_effects() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.curve.shadows = 10.0;
        assert!(s.adjusts_curve() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.mixer.saturation[3] = 10.0;
        assert!(s.adjusts_mixer() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.grading.shadows.saturation = 10.0;
        assert!(s.adjusts_grading() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.detail.sharpen_amount = 10.0;
        assert!(s.adjusts_detail() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.optics.distortion = 10.0;
        assert!(s.adjusts_optics() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.geometry.rotate = 3.0;
        assert!(s.adjusts_geometry() && !s.is_identity());
        let mut s = CameraRawSettings::default();
        s.calibration.red_hue = 10.0;
        assert!(s.adjusts_calibration() && !s.is_identity());
        // Applying(false) resets each group to defaults.
        let off = s.applying(true, true, true, true, true, true, true, true, true, false);
        assert!(off.is_identity());
    }

    #[test]
    fn gains_and_grain_kernel_size_math() {
        assert_eq!(CameraRawSettings::default().gains(), (1.0, 1.0, 1.0));
        let mut s = CameraRawSettings::default();
        s.temperature = 100.0;
        s.tint = 100.0;
        let (r, g, b) = s.gains();
        assert!((r - 1.5).abs() < 1e-9);
        assert!((g - 0.7).abs() < 1e-9);
        assert!((b - 0.8).abs() < 1e-9);
        let mut s = CameraRawSettings::default();
        s.grain_size = 0.0;
        assert!((s.grain_kernel_size() - 0.5).abs() < 1e-9);
        s.grain_size = 100.0;
        assert!((s.grain_kernel_size() - 20.0).abs() < 1e-9);
        s.grain_size = 25.0;
        assert!((s.grain_kernel_size() - 5.375).abs() < 1e-9);
    }

    #[test]
    fn normalized_clamps_sub_settings() {
        let mut s = CameraRawSettings::default();
        s.curve.shadows = 500.0;
        s.curve.shadow_split = 1.0;
        s.curve.dark_split = 200.0;
        s.mixer.hue[0] = -300.0;
        s.mixer.points.push(CameraRawPointColor {
            hue: 400.0,
            hue_range: 1.0,
            ..Default::default()
        });
        s.detail.sharpen_amount = 900.0;
        s.detail.sharpen_radius = 800.0;
        s.optics.distortion = -900.0;
        s.optics.purple_hue_low = 350.0;
        s.optics.purple_hue_high = 10.0;
        s.geometry.rotate = 90.0;
        s.calibration.blue_saturation = 700.0;
        let n = s.normalized();
        assert_eq!(n.curve.shadows, 100.0);
        assert_eq!(n.curve.shadow_split, 5.0);
        assert_eq!(n.curve.dark_split, 95.0);
        assert_eq!(n.mixer.hue[0], -100.0);
        assert_eq!(n.mixer.points[0].hue, 360.0);
        assert_eq!(n.mixer.points[0].hue_range, 5.0);
        assert_eq!(n.detail.sharpen_amount, 150.0);
        assert_eq!(n.detail.sharpen_radius, 100.0);
        assert_eq!(n.optics.distortion, -100.0);
        assert_eq!(n.optics.purple_hue_low, 10.0, "swapped handles");
        assert_eq!(n.optics.purple_hue_high, 350.0);
        assert_eq!(n.geometry.rotate, 45.0);
        assert_eq!(n.calibration.blue_saturation, 100.0);
    }

    #[test]
    fn curve_parametric_targets_its_region() {
        let curve = CameraRawCurveSettings::default();
        // Default dividers 25/50/75.
        assert_eq!(curve.region_index(0.1), 0);
        assert_eq!(curve.region_index(0.3), 1);
        assert_eq!(curve.region_index(0.6), 2);
        assert_eq!(curve.region_index(0.9), 3);
        let lifted = CameraRawCurveSettings {
            shadows: 100.0,
            ..Default::default()
        };
        let mapped = lifted.parametric(0.12);
        assert!(mapped > 0.12, "shadows +100 lifts a dark tone: {mapped}");
        assert!((lifted.parametric(0.62) - 0.62).abs() < 1e-9, "lights stay");
        // A linear point curve is the identity.
        let table = lifted.luma_table();
        for (i, v) in table.iter().enumerate() {
            let expected = lifted.parametric(i as f64 / 255.0);
            assert!((*v as f64 - expected).abs() < 1e-3);
        }
        // Nudging moves the nearest point.
        let nudged = lifted.nudged(CameraRawPointChannel::Rgb, 0.5, 0.1);
        assert!(
            (nudged.rgb[1].y - 1.0).abs() < 1e-9,
            "the (1,1) handle clamps at 1"
        );
    }

    #[test]
    fn mixer_weights_overlap_neighbors() {
        let weights = CameraRawMixerSettings::weights_for_hue(25.0);
        // 25° sits between the red (0°) and orange (30°) centers.
        assert!((weights[0] - 0.375).abs() < 1e-9);
        assert!((weights[1] - 0.875).abs() < 1e-9);
        assert_eq!(weights[4], 0.0);
        let opposite = CameraRawMixerSettings::weights_for_hue(180.0);
        assert_eq!(opposite[0], 0.0);
        assert!((opposite[4] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn curve_mixer_and_grading_change_only_their_own_tones() {
        let dark = solid(4, 4, [31, 31, 31, 255]);
        let light = solid(4, 4, [158, 158, 158, 255]);
        let mut settings = CameraRawSettings::default();
        settings.curve.shadows = 100.0;
        let graded =
            |img: &Bitmap| straight_pixels(&settings.apply(img, None, 1.0, 0, -1, false))[0];
        let dark_gain = graded(&dark)[0] as i32 - 31;
        let light_gain = graded(&light)[0] as i32 - 158;
        assert!(
            dark_gain > light_gain + 8,
            "parametric shadows: {dark_gain} vs {light_gain}"
        );

        settings = CameraRawSettings::default();
        settings.curve.rgb = CameraRawCurveSettings::strong_contrast();
        let contrasted = straight_pixels(&settings.apply(
            &solid(4, 4, [64, 64, 64, 255]),
            None,
            1.0,
            0,
            -1,
            false,
        ))[0][0];
        assert!(
            contrasted < 55,
            "strong contrast pulls a dark midtone down: {contrasted}"
        );

        settings = CameraRawSettings::default();
        settings.mixer.hue[0] = 100.0;
        let shifted = straight_pixels(&settings.apply(
            &solid(4, 4, [255, 0, 0, 255]),
            None,
            1.0,
            0,
            -1,
            false,
        ))[0];
        assert!(
            shifted[1] > shifted[2],
            "reds hue moves red toward orange: {shifted:?}"
        );

        settings = CameraRawSettings::default();
        settings.grading.shadows.saturation = 100.0;
        let graded_dark = straight_pixels(&settings.apply(&dark, None, 1.0, 0, -1, false))[0];
        let graded_light = straight_pixels(&settings.apply(
            &solid(4, 4, [255, 255, 255, 255]),
            None,
            1.0,
            0,
            -1,
            false,
        ))[0];
        assert!(
            graded_dark[0] > graded_dark[1] + 5,
            "shadow grading tints dark: {graded_dark:?}"
        );
        assert!((graded_light[0] as i32 - graded_light[1] as i32).abs() <= 2);
        settings.grading.balance = 100.0;
        let balanced = straight_pixels(&settings.apply(&dark, None, 1.0, 0, -1, false))[0];
        assert!(
            (balanced[0] as i32 - balanced[1] as i32)
                < (graded_dark[0] as i32 - graded_dark[1] as i32),
            "balance toward highlights weakens the shadow tint"
        );

        settings = CameraRawSettings::default();
        settings.curve.shadows = 100.0;
        let hidden = settings.applying(true, true, true, false, true, true, true, true, true, true);
        let out = straight_pixels(&hidden.apply(&dark, None, 1.0, 0, -1, false));
        assert_eq!(out, straight_pixels(&dark));
    }

    #[test]
    fn texture_and_clarity_sharpen_an_edge_and_leave_a_flat_field() {
        let flat = solid(4, 4, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.texture = 100.0;
        settings.clarity = 100.0;
        let flat_result = straight_pixels(&settings.apply(&flat, None, 1.0, 0, -1, false));
        assert_eq!(
            flat_result,
            straight_pixels(&flat),
            "flat field has no local contrast"
        );

        let edge = step_image();
        let original = straight_pixels(&edge);
        settings = CameraRawSettings::default();
        settings.texture = 100.0;
        let textured = straight_pixels(&settings.apply(&edge, None, 1.0, 0, -1, false));
        assert_eq!(textured[0][0], original[0][0], "far from the edge");
        assert_eq!(
            textured[8][0], original[8][0],
            "texture's fine radius does not reach"
        );
        assert_ne!(textured[11][0], original[11][0], "the edge itself moves");

        settings = CameraRawSettings::default();
        settings.clarity = 100.0;
        let clarified = straight_pixels(&settings.apply(&edge, None, 1.0, 0, -1, false));
        assert_eq!(clarified[0][0], original[0][0]);
        assert_ne!(
            clarified[8][0], original[8][0],
            "clarity's wider radius reaches further"
        );
        settings.clarity = -100.0;
        let softened = straight_pixels(&settings.apply(&edge, None, 1.0, 0, -1, false));
        let hard_gap = (original[12][0] as i32 - original[11][0] as i32).abs();
        let soft_gap = (softened[12][0] as i32 - softened[11][0] as i32).abs();
        assert!(
            soft_gap < hard_gap,
            "negative clarity pulls the step together"
        );
    }

    #[test]
    fn dehaze_deepens_or_lifts_and_keeps_alpha() {
        let dark = solid(4, 4, [30, 30, 30, 255]);
        let pale = solid(4, 4, [180, 150, 150, 255]);
        let mut settings = CameraRawSettings::default();
        settings.dehaze = 100.0;
        let deepened = straight_pixels(&settings.apply(&dark, None, 1.0, 0, -1, false))[0];
        assert!(deepened[0] < 30, "positive dehaze darkens: {deepened:?}");
        let pale_before = straight_pixels(&pale)[0];
        let pale_after = straight_pixels(&settings.apply(&pale, None, 1.0, 0, -1, false))[0];
        assert!(
            chroma(&pale_after) > chroma(&pale_before),
            "raises saturation"
        );
        settings.dehaze = -100.0;
        let lifted = straight_pixels(&settings.apply(&dark, None, 1.0, 0, -1, false))[0];
        assert!(lifted[0] > 30, "negative dehaze lifts: {lifted:?}");
        let faded = straight_pixels(&settings.apply(&pale, None, 1.0, 0, -1, false))[0];
        assert!(chroma(&faded) < chroma(&pale_before), "lowers saturation");
        let translucent = solid(4, 4, [30, 30, 30, 128]);
        settings.dehaze = 100.0;
        let dehazed = settings.apply(&translucent, None, 1.0, 0, -1, false);
        assert_eq!(dehazed.pixel(0, 0)[3], 128, "alpha survives");
    }

    #[test]
    fn glow_is_idle_at_zero_and_halation_is_redder_than_diffusion() {
        let mut spot = Bitmap::new(21, 21);
        for y in 0..21 {
            for x in 0..21 {
                spot.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        for y in 8..13 {
            for x in 8..13 {
                spot.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let spot_pixels = straight_pixels(&spot);
        let mut settings = CameraRawSettings::default();
        settings.glow_warmth = 100.0;
        settings.glow_range = 100.0;
        let idle = straight_pixels(&settings.apply(&spot, None, 1.0, 0, -1, false));
        assert_eq!(
            idle, spot_pixels,
            "range and warmth do nothing until Glow is raised"
        );

        settings.glow = 100.0;
        settings.glow_style = CameraRawGlowStyle::Diffusion;
        let diffusion = straight_pixels(&settings.apply(&spot, None, 1.0, 0, -1, false));
        settings.glow_style = CameraRawGlowStyle::Halation;
        let halation = straight_pixels(&settings.apply(&spot, None, 1.0, 0, -1, false));
        let fringe = 10 * 21 + 15;
        assert!(
            diffusion[fringe][0] > diffusion[0][0],
            "glow brightens the neighborhood"
        );
        assert_eq!(halation[0], [0, 0, 0, 255]);
        let diffusion_red = diffusion[fringe][0] as i32 - diffusion[fringe][1] as i32;
        let halation_red = halation[fringe][0] as i32 - halation[fringe][1] as i32;
        assert!(
            halation_red > diffusion_red,
            "halation is redder at the same warmth"
        );
    }

    #[test]
    fn vignette_darkens_corners_and_highlights_only_while_darkening() {
        let gray_field = solid(9, 9, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.vignette_amount = -100.0;
        let darkened = straight_pixels(&settings.apply(&gray_field, None, 1.0, 0, -1, false));
        let center = darkened[4 * 9 + 4][0];
        let corner = darkened[0][0];
        assert!(
            (center as i32 - 128).abs() <= 2,
            "the center stays: {center}"
        );
        assert!(corner < center - 40, "the corner darkens: {corner}");

        let white = solid(9, 9, [255, 255, 255, 255]);
        settings.vignette_highlights = 100.0;
        settings.vignette_style = CameraRawVignetteStyle::HighlightPriority;
        let protected = straight_pixels(&settings.apply(&white, None, 1.0, 0, -1, false))[0][0];
        settings.vignette_highlights = 0.0;
        let exposed = straight_pixels(&settings.apply(&white, None, 1.0, 0, -1, false))[0][0];
        assert!(protected > exposed + 40, "highlights holds a bright corner");
        settings.vignette_highlights = 100.0;
        settings.vignette_style = CameraRawVignetteStyle::PaintOverlay;
        let painted = straight_pixels(&settings.apply(&white, None, 1.0, 0, -1, false))[0][0];
        assert!(painted < protected, "paint overlay does not use Highlights");

        settings = CameraRawSettings::default();
        settings.vignette_amount = 100.0;
        settings.vignette_highlights = 0.0;
        let plain = straight_pixels(&settings.apply(&gray_field, None, 1.0, 0, -1, false));
        settings.vignette_highlights = 100.0;
        let with_highlights =
            straight_pixels(&settings.apply(&gray_field, None, 1.0, 0, -1, false));
        assert_eq!(
            plain, with_highlights,
            "highlights is idle while lightening"
        );
    }

    #[test]
    fn grain_is_stable_and_drops_with_the_effects_eye() {
        let field = solid(16, 16, [128, 128, 128, 255]);
        let field_pixels = straight_pixels(&field);
        let mut settings = CameraRawSettings::default();
        settings.grain_size = 40.0;
        settings.grain_roughness = 80.0;
        let silent = straight_pixels(&settings.apply(&field, None, 1.0, 4, -1, false));
        assert_eq!(
            silent, field_pixels,
            "size and roughness do nothing at amount zero"
        );
        settings.grain_amount = 70.0;
        let first = settings.apply(&field, None, 1.0, 4, -1, false);
        let second = settings.apply(&field, None, 1.0, 4, -1, false);
        assert_eq!(
            first.data, second.data,
            "the same seed keeps the same grain"
        );
        assert_ne!(straight_pixels(&first), field_pixels);
        let px = straight_pixels(&first)[0];
        assert!(
            px[0] == px[1] && px[1] == px[2],
            "grain moves brightness only"
        );

        let edge = step_image();
        let edge_pixels = straight_pixels(&edge);
        let mut settings = CameraRawSettings::default();
        settings.texture = 100.0;
        settings.grain_amount = 50.0;
        let shown = straight_pixels(&settings.apply(&edge, None, 1.0, 2, -1, false));
        let hidden = settings.applying(true, true, false, true, true, true, true, true, true, true);
        assert!(hidden.is_identity());
        let hidden_pixels = straight_pixels(&hidden.apply(&edge, None, 1.0, 2, -1, false));
        assert_eq!(hidden_pixels, edge_pixels);
        assert_ne!(shown, edge_pixels);
    }

    #[test]
    fn box_blur_plane_averages_with_clamped_edges() {
        let src: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let mut dst = vec![0.0f32; 16];
        box_blur_plane(&src, &mut dst, 4, 4, 1);
        // Interior average of the 3×3 window around index 5: 0..10 mean 5.
        assert!((dst[5] - 5.0).abs() < 1e-3, "{}", dst[5]);
        // Radius 0 copies.
        box_blur_plane(&src, &mut dst, 4, 4, 0);
        assert_eq!(dst, src);
    }

    #[test]
    fn clip_overlay_marks_clipped_pixels() {
        let black = solid(4, 4, [0, 0, 0, 255]);
        let white = solid(4, 4, [255, 255, 255, 255]);
        let shadowed = clip_overlay(&black, true, false);
        assert!(
            shadowed.pixel(0, 0)[2] > shadowed.pixel(0, 0)[0],
            "clipped shadows paint blue"
        );
        let highlighted = clip_overlay(&white, false, true);
        assert!(
            highlighted.pixel(0, 0)[0] > highlighted.pixel(0, 0)[2],
            "clipped highlights paint red"
        );
        let untouched = clip_overlay(&black, false, false);
        assert_eq!(untouched.data, black.data);
    }

    #[test]
    fn sharpen_mask_overlay_is_grayscale() {
        let edge = step_image();
        let mut settings = CameraRawSettings::default();
        settings.detail.sharpen_amount = 100.0;
        settings.detail.sharpen_masking = 50.0;
        assert!(settings.detail.adjusts());
        let mask = settings.apply(&edge, None, 1.0, 0, -1, true);
        for px in mask.data.chunks_exact(4) {
            assert_eq!(px[0], px[1]);
            assert_eq!(px[1], px[2], "mask preview is grayscale");
        }
    }

    #[test]
    fn detail_sharpening_and_noise_change_pixels() {
        let edge = step_image();
        let mut settings = CameraRawSettings::default();
        settings.detail.sharpen_amount = 150.0;
        settings.detail.sharpen_radius = 50.0;
        let sharpened = settings.apply(&edge, None, 1.0, 0, -1, false);
        assert_ne!(
            sharpened.data, edge.data,
            "sharpening changes the step image"
        );

        // The sharpened edge overshoots: the bright side rises, the dark side falls.
        let before = straight_pixels(&edge);
        let after = straight_pixels(&sharpened);
        assert!(after[10][0] < before[10][0] && after[13][0] > before[13][0]);

        let flat = solid(8, 8, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.detail.noise_luminance = 80.0;
        let smoothed = settings.apply(&flat, None, 1.0, 0, -1, false);
        assert_eq!(
            straight_pixels(&smoothed),
            straight_pixels(&flat),
            "luminance NR leaves a flat field"
        );
    }

    #[test]
    fn defringe_reduces_purple_fringes() {
        let purple = solid(4, 4, [204, 51, 230, 255]);
        let before = straight_pixels(&purple)[0];
        let mut settings = CameraRawSettings::default();
        settings.optics.purple_amount = 100.0;
        settings.optics.purple_hue_low = 250.0;
        settings.optics.purple_hue_high = 320.0;
        let defringed = straight_pixels(&settings.apply(&purple, None, 1.0, 0, -1, false))[0];
        assert!(
            chroma(&defringed) < chroma(&before),
            "purple defringe lowers chroma"
        );
        // Out-of-range hues keep their chroma.
        settings.optics.purple_hue_low = 10.0;
        settings.optics.purple_hue_high = 20.0;
        let untouched = straight_pixels(&settings.apply(&purple, None, 1.0, 0, -1, false))[0];
        assert_eq!(chroma(&untouched), chroma(&before));
    }

    #[test]
    fn optics_distortion_resamples_pixels() {
        let stepped = checker(24, 24);
        let mut settings = CameraRawSettings::default();
        settings.optics.distortion = 100.0;
        let warped = settings.apply(&stepped, None, 1.0, 0, -1, false);
        assert_ne!(warped.data, stepped.data, "distortion resamples pixels");
        // Off-center pixels move; the center stays.
        let corner_before = stepped.pixel(0, 0);
        let corner_after = warped.pixel(0, 0);
        assert_ne!(corner_before, corner_after);
    }

    #[test]
    fn calibration_shifts_primaries() {
        let red = solid(4, 4, [255, 0, 0, 255]);
        let before = straight_pixels(&red)[0];
        let mut settings = CameraRawSettings::default();
        settings.calibration.red_hue = 80.0;
        let after = straight_pixels(&settings.apply(&red, None, 1.0, 0, -1, false))[0];
        assert_ne!(after, before, "calibration shifts a pure red");
        let hidden = settings.applying(true, true, true, true, true, true, true, true, true, false);
        let hidden_pixels = straight_pixels(&hidden.apply(&red, None, 1.0, 0, -1, false))[0];
        assert_eq!(hidden_pixels, before);
    }

    #[test]
    fn geometry_warps_a_checker_and_leaves_an_unguided_picture() {
        let grid = checker(12, 12);
        let grid_pixels = straight_pixels(&grid);
        // Guided without a drawn line must not invent a warp.
        let mut settings = CameraRawSettings::default();
        settings.geometry.upright = CameraRawUprightMode::Guided;
        assert!(!settings.adjusts_geometry());
        assert_eq!(
            straight_pixels(&settings.apply(&grid, None, 1.0, 0, -1, false)),
            grid_pixels
        );

        settings.geometry.guides.push(CameraRawGeometryGuide {
            start_x: 0.1,
            start_y: 0.15,
            end_x: 0.9,
            end_y: 0.8,
        });
        assert!(settings.adjusts_geometry());
        let guided = settings.geometry.apply(&grid);
        assert_ne!(
            straight_pixels(&guided),
            grid_pixels,
            "the guide rotates the picture"
        );

        let mut settings = CameraRawSettings::default();
        settings.geometry.vertical = 40.0;
        let warped = settings.geometry.apply(&grid);
        assert_ne!(straight_pixels(&warped), grid_pixels);
        assert_eq!(warped.width, grid.width);
        assert_eq!(warped.height, grid.height);
        // Constrain Crop trims transparent margins and refits.
        settings.geometry.constrain_crop = true;
        let fitted = settings.geometry.apply(&grid);
        assert_eq!(fitted.width, grid.width);
        assert!(
            fitted.pixel(0, 0)[3] > 0 || fitted.pixel(11, 11)[3] > 0,
            "refit fills the frame"
        );
    }

    /// §7.1-7: like `CIPerspectiveTransform`, output whose inverse mapping
    /// lands off the image must be transparent (premultiplied-clear), not the
    /// edge color smeared outward. A constant picture warped into a small
    /// projective trapezoid leaves the rest of the frame clear.
    #[test]
    fn perspective_warp_leaves_off_image_samples_transparent() {
        let red = solid(8, 8, [200, 100, 50, 255]);
        // y measured upward from the bottom: the picture shrinks into a
        // trapezoid in the lower left of the raster grid (top edge shorter
        // than the bottom edge, so the homography is genuinely projective).
        let corners = [
            (1.0, 3.0), // top left
            (3.0, 3.0), // top right
            (3.0, 0.0), // bottom right
            (0.0, 0.0), // bottom left
        ];
        let warped = perspective_warp(&red, &corners);
        assert_eq!(warped.width, 8);
        assert_eq!(warped.height, 8);
        // Far from the trapezoid the frame is premultiplied-transparent black.
        assert_eq!(warped.pixel(0, 0), [0, 0, 0, 0]);
        // Right of the trapezoid (this used to clamp to the edge color).
        assert_eq!(warped.pixel(6, 1), [0, 0, 0, 0]);
        // Above it, past the top edge (raster row 0).
        assert_eq!(warped.pixel(1, 0), [0, 0, 0, 0]);
        // Inside the trapezoid the constant source color comes through exact.
        assert_eq!(warped.pixel(1, 6), [200, 100, 50, 255]);
        assert_eq!(warped.pixel(2, 6), [200, 100, 50, 255]);
    }

    #[test]
    fn scope_make_counts_histogram_and_vectorscope() {
        let black = solid(4, 4, [0, 0, 0, 255]);
        let white = solid(4, 4, [255, 255, 255, 255]);
        let black_scope = CameraRawScope::make(&black).unwrap();
        let white_scope = CameraRawScope::make(&white).unwrap();
        assert_eq!(peak_index(&black_scope.red), 0);
        assert_eq!(peak_index(&white_scope.red), 255);

        let gray = solid(4, 4, [128, 128, 128, 255]);
        let mut settings = CameraRawSettings::default();
        settings.exposure = 1.0;
        let shifted =
            CameraRawScope::make(&settings.apply(&gray, None, 1.0, 0, -1, false)).unwrap();
        assert!(
            peak_index(&shifted.red) > 128,
            "exposure moves the midtones toward white"
        );

        // One opaque red pixel: red bin full, green and blue at zero.
        let red = solid(1, 1, [255, 0, 0, 255]);
        let scope = CameraRawScope::make(&red).unwrap();
        assert!((scope.red[255] - 1.0).abs() < 1e-9);
        assert!((scope.green[0] - 1.0).abs() < 1e-9);
        assert!((scope.blue[0] - 1.0).abs() < 1e-9);
        let hottest = peak_index(&scope.vectorscope);
        assert!(
            hottest % CameraRawScope::SCOPE_SIDE > CameraRawScope::SCOPE_SIDE / 2,
            "red sits right"
        );
        assert!(scope.peak() > 0.0);

        let empty = Bitmap::new(0, 0);
        assert!(CameraRawScope::make(&empty).is_none());
    }
}
