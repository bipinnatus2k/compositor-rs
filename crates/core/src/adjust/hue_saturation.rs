//! Port of `Compositor/Document/HueSaturation.swift` — the Hue/Saturation
//! adjustment (Photoshop's ⌘U), including the per-range bands, the colorize
//! mode, and the 33³ color cube (`CIColorCube`) the Swift filter renders
//! through. The same HSL math that Swift uses to fill the cube fills it here;
//! the cube is then applied with f32 trilinear interpolation, exactly as
//! `CIColorCube` does.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The six color ranges plus Master, as in Photoshop's Cmd+U.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorRange {
    #[default]
    #[serde(rename = "Master")]
    Master,
    #[serde(rename = "Reds")]
    Reds,
    #[serde(rename = "Yellows")]
    Yellows,
    #[serde(rename = "Greens")]
    Greens,
    #[serde(rename = "Cyans")]
    Cyans,
    #[serde(rename = "Blues")]
    Blues,
    #[serde(rename = "Magentas")]
    Magentas,
}

impl ColorRange {
    pub const ALL: [ColorRange; 7] = [
        ColorRange::Master,
        ColorRange::Reds,
        ColorRange::Yellows,
        ColorRange::Greens,
        ColorRange::Cyans,
        ColorRange::Blues,
        ColorRange::Magentas,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ColorRange::Master => "Master",
            ColorRange::Reds => "Reds",
            ColorRange::Yellows => "Yellows",
            ColorRange::Greens => "Greens",
            ColorRange::Cyans => "Cyans",
            ColorRange::Blues => "Blues",
            ColorRange::Magentas => "Magentas",
        }
    }

    /// Photoshop's starting hue band: falloff start, range start, range end, falloff end.
    pub fn default_band(&self) -> HueBand {
        match self {
            ColorRange::Master => HueBand::new(0.0, 0.0, 360.0, 360.0),
            ColorRange::Reds => HueBand::new(315.0, 345.0, 15.0, 45.0),
            ColorRange::Yellows => HueBand::new(15.0, 45.0, 75.0, 105.0),
            ColorRange::Greens => HueBand::new(75.0, 105.0, 135.0, 165.0),
            ColorRange::Cyans => HueBand::new(135.0, 165.0, 195.0, 225.0),
            ColorRange::Blues => HueBand::new(195.0, 225.0, 255.0, 285.0),
            ColorRange::Magentas => HueBand::new(255.0, 285.0, 315.0, 345.0),
        }
    }

    pub fn color_ranges() -> Vec<ColorRange> {
        ColorRange::ALL
            .iter()
            .copied()
            .filter(|c| *c != ColorRange::Master)
            .collect()
    }
}

/// A hue band in degrees, wrapping at 360: full strength between `range_start`
/// and `range_end`, fading to nothing at `falloff_start` and `falloff_end`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HueBand {
    #[serde(default)]
    pub falloff_start: f64,
    #[serde(default)]
    pub range_start: f64,
    #[serde(default = "default_360")]
    pub range_end: f64,
    #[serde(default = "default_360")]
    pub falloff_end: f64,
}

fn default_360() -> f64 {
    360.0
}

impl Default for HueBand {
    fn default() -> Self {
        HueBand::new(0.0, 0.0, 360.0, 360.0)
    }
}

impl HueBand {
    pub fn new(falloff_start: f64, range_start: f64, range_end: f64, falloff_end: f64) -> Self {
        HueBand {
            falloff_start,
            range_start,
            range_end,
            falloff_end,
        }
    }

    /// Degrees from `from` forward to `to`, always 0…360.
    pub fn forward(from: f64, to: f64) -> f64 {
        let delta = (to - from) % 360.0;
        if delta < 0.0 { delta + 360.0 } else { delta }
    }

    /// How strongly this band claims a hue: 1 inside the range, ramping
    /// linearly through each falloff shoulder, 0 outside.
    pub fn weight(&self, hue: f64) -> f64 {
        let span = Self::forward(self.falloff_start, self.falloff_end);
        if span <= 0.0 {
            return 1.0; // Master covers everything.
        }
        let position = Self::forward(self.falloff_start, hue);
        if position > span {
            return 0.0;
        }
        let ramp_in = Self::forward(self.falloff_start, self.range_start);
        let plateau_end = Self::forward(self.falloff_start, self.range_end);
        if position < ramp_in {
            return if ramp_in > 0.0 {
                position / ramp_in
            } else {
                1.0
            };
        }
        if position <= plateau_end {
            return 1.0;
        }
        let ramp_out = span - plateau_end;
        if ramp_out > 0.0 {
            (span - position) / ramp_out
        } else {
            1.0
        }
    }

    pub fn handles(&self) -> [f64; 4] {
        [
            self.falloff_start,
            self.range_start,
            self.range_end,
            self.falloff_end,
        ]
    }

    /// A band centered on one hue, keeping this band's core and shoulder widths.
    pub fn centered_on(&self, hue: f64) -> HueBand {
        let core = Self::forward(self.range_start, self.range_end);
        let leading = Self::forward(self.falloff_start, self.range_start);
        let trailing = Self::forward(self.range_end, self.falloff_end);
        fn wrap(value: f64) -> f64 {
            let r = value % 360.0;
            if r < 0.0 { r + 360.0 } else { r }
        }
        let start = wrap(hue - core / 2.0);
        HueBand::new(
            wrap(start - leading),
            start,
            wrap(start + core),
            wrap(start + core + trailing),
        )
    }

    /// Widens the band so this hue is fully inside it, moving whichever edge is nearer.
    pub fn include(&mut self, hue: f64) {
        if self.weight(hue) >= 1.0 {
            return;
        }
        let shoulder_in = Self::forward(self.falloff_start, self.range_start);
        let shoulder_out = Self::forward(self.range_end, self.falloff_end);
        let before_start = Self::forward(hue, self.range_start);
        let after_end = Self::forward(self.range_end, hue);
        if before_start <= after_end {
            self.range_start = hue;
            self.falloff_start = hue - shoulder_in;
        } else {
            self.range_end = hue;
            self.falloff_end = hue + shoulder_out;
        }
        self.normalize();
    }

    /// Narrows the band so this hue falls outside it entirely, shoulder included.
    pub fn exclude(&mut self, hue: f64) {
        if self.weight(hue) <= 0.0 {
            return;
        }
        let shoulder_in = Self::forward(self.falloff_start, self.range_start);
        let shoulder_out = Self::forward(self.range_end, self.falloff_end);
        let from_start = Self::forward(self.falloff_start, hue);
        let to_end = Self::forward(hue, self.falloff_end);
        if from_start <= to_end {
            self.falloff_start = hue + 1.0;
            self.range_start = hue + 1.0 + shoulder_in;
        } else {
            self.falloff_end = hue - 1.0;
            self.range_end = hue - 1.0 - shoulder_out;
        }
        self.normalize();
    }

    /// Keeps all four handles in 0…360 and the band under a full circle.
    fn normalize(&mut self) {
        fn wrap(value: f64) -> f64 {
            let r = value % 360.0;
            if r < 0.0 { r + 360.0 } else { r }
        }
        self.falloff_start = wrap(self.falloff_start);
        self.range_start = wrap(self.range_start);
        self.range_end = wrap(self.range_end);
        self.falloff_end = wrap(self.falloff_end);
        if Self::forward(self.falloff_start, self.falloff_end) > 350.0 {
            self.falloff_end = wrap(self.falloff_start + 350.0);
        }
    }

    /// Moves one handle, keeping the four in order and the band under a full circle.
    pub fn set_handle(&mut self, index: usize, degrees: f64) {
        let mut updated = *self;
        let value = ((degrees % 360.0) + 360.0) % 360.0;
        match index {
            0 => updated.falloff_start = value,
            1 => updated.range_start = value,
            2 => updated.range_end = value,
            _ => updated.falloff_end = value,
        }
        let span = Self::forward(updated.falloff_start, updated.falloff_end);
        let to_start = Self::forward(updated.falloff_start, updated.range_start);
        let to_end = Self::forward(updated.falloff_start, updated.range_end);
        if !(span > 1.0 && span <= 350.0 && to_start <= to_end && to_end <= span) {
            return;
        }
        *self = updated;
    }
}

/// Which eyedropper is armed while the Hue/Saturation panel is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HueSampleMode {
    #[default]
    Replace,
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct RangeAdjustment {
    #[serde(default)]
    pub hue: f64,
    #[serde(default)]
    pub saturation: f64,
    #[serde(default)]
    pub lightness: f64,
}

/// Hue is −180…180 (0…360 when colorizing), Saturation −100…100 (0…100
/// colorizing), Lightness −100…100. Each color range keeps its own values;
/// Master applies everywhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HueSaturationSettings {
    /// Which range the sliders and spectrum edit.
    #[serde(default)]
    pub range: ColorRange,
    #[serde(default)]
    pub colorize: bool,
    /// Applies the selected range to everything *outside* its band instead.
    #[serde(default)]
    pub invert_range: bool,
    #[serde(default)]
    pub adjustments: HashMap<ColorRange, RangeAdjustment>,
    #[serde(default = "default_bands")]
    pub bands: HashMap<ColorRange, HueBand>,
}

fn default_bands() -> HashMap<ColorRange, HueBand> {
    ColorRange::ALL
        .iter()
        .map(|c| (*c, c.default_band()))
        .collect()
}

impl Default for HueSaturationSettings {
    fn default() -> Self {
        HueSaturationSettings {
            range: ColorRange::Master,
            colorize: false,
            invert_range: false,
            adjustments: HashMap::new(),
            bands: default_bands(),
        }
    }
}

impl HueSaturationSettings {
    pub fn new(hue: f64, saturation: f64, lightness: f64, colorize: bool) -> Self {
        let mut result = HueSaturationSettings::default();
        result.adjustments.insert(
            ColorRange::Master,
            RangeAdjustment {
                hue,
                saturation,
                lightness,
            },
        );
        result.colorize = colorize;
        result
    }

    /// Photoshop's starting point when Colorize is switched on.
    pub fn colorize_start() -> Self {
        HueSaturationSettings::new(0.0, 25.0, 0.0, true)
    }

    fn adjustment_mut(&mut self) -> &mut RangeAdjustment {
        self.adjustments.entry(self.range).or_default()
    }

    pub fn hue(&self) -> f64 {
        self.adjustments
            .get(&self.range)
            .map(|a| a.hue)
            .unwrap_or(0.0)
    }

    pub fn set_hue(&mut self, value: f64) {
        self.adjustment_mut().hue = value;
    }

    pub fn saturation(&self) -> f64 {
        self.adjustments
            .get(&self.range)
            .map(|a| a.saturation)
            .unwrap_or(0.0)
    }

    pub fn set_saturation(&mut self, value: f64) {
        self.adjustment_mut().saturation = value;
    }

    pub fn lightness(&self) -> f64 {
        self.adjustments
            .get(&self.range)
            .map(|a| a.lightness)
            .unwrap_or(0.0)
    }

    pub fn set_lightness(&mut self, value: f64) {
        self.adjustment_mut().lightness = value;
    }

    pub fn band(&self) -> HueBand {
        self.bands
            .get(&self.range)
            .copied()
            .unwrap_or_else(|| self.range.default_band())
    }

    pub fn set_band(&mut self, band: HueBand) {
        self.bands.insert(self.range, band);
    }

    pub fn is_identity(&self) -> bool {
        !self.colorize
            && self
                .adjustments
                .values()
                .all(|a| *a == RangeAdjustment::default())
    }

    /// How much a range applies to one hue: Master everywhere, others through their band.
    pub fn weight(&self, color_range: ColorRange, hue: f64) -> f64 {
        if color_range == ColorRange::Master {
            return 1.0;
        }
        let weight = self
            .bands
            .get(&color_range)
            .copied()
            .unwrap_or_else(|| color_range.default_band())
            .weight(hue);
        if self.invert_range && color_range == self.range {
            1.0 - weight
        } else {
            weight
        }
    }
}

/// `HueSaturationFilter.hueResponse` — how much every range shifts a given hue.
pub type HueResponse = (f64, f64, f64);

pub fn hue_response(settings: &HueSaturationSettings) -> Vec<HueResponse> {
    (0..=360)
        .map(|degree| {
            let mut response: HueResponse = (0.0, 0.0, 0.0);
            // Sum in a fixed order: Swift's Dictionary iteration order varies
            // too, but a deterministic order keeps float sums reproducible
            // across runs and platforms.
            let mut order: Vec<&ColorRange> = settings.adjustments.keys().collect();
            order.sort_by_key(|r| {
                ColorRange::ALL
                    .iter()
                    .position(|a| a == *r)
                    .unwrap_or(usize::MAX)
            });
            for color_range in order {
                let adjustment = &settings.adjustments[color_range];
                if *adjustment == RangeAdjustment::default() {
                    continue;
                }
                let weight = settings.weight(*color_range, degree as f64);
                if weight <= 0.0 {
                    continue;
                }
                response.0 += adjustment.hue * weight;
                response.1 += adjustment.saturation * weight;
                response.2 += adjustment.lightness * weight;
            }
            response
        })
        .collect()
}

/// `HueSaturationFilter.adjust` — one pixel's HSL adjustment.
pub fn adjust_hsl(
    red: f64,
    green: f64,
    blue: f64,
    settings: &HueSaturationSettings,
    response: Option<&[HueResponse]>,
) -> (f64, f64, f64) {
    let (mut hue, mut saturation, mut lightness) = to_hsl(red, green, blue);
    let lightness_amount;
    if settings.colorize {
        hue = settings.hue() % 360.0;
        saturation = (settings.saturation() / 100.0).min(1.0).max(0.0);
        lightness_amount = settings.lightness() / 100.0;
    } else {
        let owned: Vec<HueResponse>;
        let table: &[HueResponse] = match response {
            Some(r) => r,
            None => {
                owned = hue_response(settings);
                &owned
            }
        };
        let sampled = table[(hue.round() as usize).min(table.len() - 1)];
        lightness_amount = sampled.2 / 100.0;
        hue = (hue + sampled.0) % 360.0;
        if hue < 0.0 {
            hue += 360.0;
        }
        // Multiplicative, so neutral grays stay neutral.
        saturation = (saturation * (1.0 + sampled.1 / 100.0)).min(1.0).max(0.0);
    }
    // Lightness pulls toward white above 0 and toward black below, reaching either at ±100.
    let amount = lightness_amount.min(1.0).max(-1.0);
    lightness = if amount >= 0.0 {
        lightness + (1.0 - lightness) * amount
    } else {
        lightness * (1.0 + amount)
    };
    to_rgb(hue, saturation, lightness.min(1.0).max(0.0))
}

/// The hue a spectrum swatch becomes, for the "after" bar.
pub fn shifted_hue(hue: f64, settings: &HueSaturationSettings) -> f64 {
    let mut shift = 0.0;
    for (color_range, adjustment) in &settings.adjustments {
        if adjustment.hue != 0.0 {
            shift += adjustment.hue * settings.weight(*color_range, hue);
        }
    }
    let shifted = (hue + shift) % 360.0;
    if shifted < 0.0 {
        shifted + 360.0
    } else {
        shifted
    }
}

/// `HueSaturationFilter.dimension` (HueSaturation.swift:234) — 33 points per
/// axis: fast to build, smooth enough.
pub const CUBE_DIMENSION: usize = 33;

/// The color cube Swift hands to `CIColorCube` (`HueSaturationFilter.cube`,
/// HueSaturation.swift:274-293). Entries are unpremultiplied sRGB RGBA
/// quadruples ordered with red varying fastest — the layout `CIColorCube`
/// expects — and stored as f32 because that is the arithmetic CIColorCube
/// quantizes and interpolates in.
#[derive(Debug, Clone)]
pub struct ColorCube {
    dimension: usize,
    /// `dimension * dimension * dimension * 4` values, red fastest, blue slowest.
    data: Vec<f32>,
}

impl ColorCube {
    /// `HueSaturationFilter.cube` — every lattice corner converted to HSL,
    /// adjusted, and back, through the exact per-pixel math (`adjust_hsl`, the
    /// port of Swift's `adjust`). The per-hue response is sampled once, exactly
    /// as Swift does, to keep the ~36k entries cheap.
    pub fn build(settings: &HueSaturationSettings) -> ColorCube {
        let response = hue_response(settings);
        let dimension = CUBE_DIMENSION;
        let mut data = vec![0.0f32; dimension * dimension * dimension * 4];
        let mut index = 0;
        let step = (dimension - 1) as f64;
        for blue in 0..dimension {
            for green in 0..dimension {
                for red in 0..dimension {
                    let (r, g, b) = adjust_hsl(
                        red as f64 / step,
                        green as f64 / step,
                        blue as f64 / step,
                        settings,
                        Some(&response),
                    );
                    data[index] = r as f32;
                    data[index + 1] = g as f32;
                    data[index + 2] = b as f32;
                    data[index + 3] = 1.0;
                    index += 4;
                }
            }
        }
        ColorCube { dimension, data }
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Offset of a lattice point's RGBA quadruple: blue outermost, red innermost,
    /// matching both the Swift builder loop and CIColorCube's data order.
    fn index(&self, red: usize, green: usize, blue: usize) -> usize {
        ((blue * self.dimension + green) * self.dimension + red) * 4
    }

    /// `CIColorCube`'s lookup: locate the 8 surrounding lattice points
    /// (lattice coordinate = color × (dimension − 1)) and trilinearly
    /// interpolate between them in f32.
    pub fn sample(&self, red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
        let last = self.dimension - 1;
        let max = last as f32;
        let axis = |value: f32| -> (usize, usize, f32) {
            let scaled = value.clamp(0.0, 1.0) * max;
            let low = (scaled as usize).min(last);
            let high = (low + 1).min(last);
            (low, high, scaled - low as f32)
        };
        let (r0, r1, fr) = axis(red);
        let (g0, g1, fg) = axis(green);
        let (b0, b1, fb) = axis(blue);
        let corner = |r: usize, g: usize, b: usize, c: usize| {
            let i = self.index(r, g, b) + c;
            self.data[i]
        };
        let mut out = [0.0f32; 3];
        for c in 0..3 {
            let c000 = corner(r0, g0, b0, c);
            let c100 = corner(r1, g0, b0, c);
            let c010 = corner(r0, g1, b0, c);
            let c110 = corner(r1, g1, b0, c);
            let c001 = corner(r0, g0, b1, c);
            let c101 = corner(r1, g0, b1, c);
            let c011 = corner(r0, g1, b1, c);
            let c111 = corner(r1, g1, b1, c);
            let c00 = c000 + (c100 - c000) * fr;
            let c10 = c010 + (c110 - c010) * fr;
            let c01 = c001 + (c101 - c001) * fr;
            let c11 = c011 + (c111 - c011) * fr;
            let c0 = c00 + (c10 - c00) * fg;
            let c1 = c01 + (c11 - c01) * fg;
            out[c] = c0 + (c1 - c0) * fb;
        }
        (out[0], out[1], out[2])
    }

    /// Builds a cube from raw RGBA data (test helper: `dimension` × 4 f32
    /// values per entry, red varying fastest).
    #[cfg(test)]
    fn from_data(dimension: usize, data: Vec<f32>) -> ColorCube {
        assert_eq!(data.len(), dimension * dimension * dimension * 4);
        ColorCube { dimension, data }
    }
}

/// Applies the cube to a whole RGBA8 premultiplied buffer the way
/// `CIColorCube` does (HueSaturation.swift:238-239): unpremultiply around the
/// lookup, interpolate in f32, premultiply again. Fully transparent pixels
/// stay untouched, as in the old per-pixel path.
pub fn apply_cube_rgba8(data: &mut [u8], cube: &ColorCube) {
    for p in data.chunks_exact_mut(4) {
        let a = p[3] as f32;
        if a == 0.0 {
            continue;
        }
        let r = p[0] as f32 * 255.0 / a / 255.0;
        let g = p[1] as f32 * 255.0 / a / 255.0;
        let b = p[2] as f32 * 255.0 / a / 255.0;
        let (nr, ng, nb) = cube.sample(r, g, b);
        p[0] = (nr * 255.0).round().clamp(0.0, a) as u8;
        p[1] = (ng * 255.0).round().clamp(0.0, a) as u8;
        p[2] = (nb * 255.0).round().clamp(0.0, a) as u8;
    }
}

fn to_hsl(red: f64, green: f64, blue: f64) -> (f64, f64, f64) {
    let high = red.max(green).max(blue);
    let low = red.min(green).min(blue);
    let lightness = (high + low) / 2.0;
    let delta = high - low;
    if delta <= 0.0 {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let mut hue;
    if high == red {
        hue = (green - blue) / delta;
    } else if high == green {
        hue = (blue - red) / delta + 2.0;
    } else {
        hue = (red - green) / delta + 4.0;
    }
    hue *= 60.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    (hue, saturation.min(1.0), lightness)
}

fn to_rgb(hue: f64, saturation: f64, lightness: f64) -> (f64, f64, f64) {
    if saturation <= 0.0 {
        return (lightness, lightness, lightness);
    }
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let base = lightness - chroma / 2.0;
    let (red, green, blue) = match sector as i32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    (
        (red + base).min(1.0).max(0.0),
        (green + base).min(1.0).max(0.0),
        (blue + base).min(1.0).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_weights() {
        let reds = ColorRange::Reds.default_band();
        assert_eq!(reds.weight(0.0), 1.0); // inside range (345…15 wraps)
        assert_eq!(reds.weight(359.0), 1.0);
        assert!(reds.weight(340.0) > 0.0 && reds.weight(340.0) < 1.0);
        assert_eq!(reds.weight(100.0), 0.0);
    }

    #[test]
    fn master_covers_all() {
        let master = ColorRange::Master.default_band();
        for hue in [0.0, 90.0, 180.0, 270.0, 359.9] {
            assert_eq!(master.weight(hue), 1.0);
        }
    }

    #[test]
    fn centered_keeps_widths() {
        let reds = ColorRange::Reds.default_band();
        let c = reds.centered_on(180.0);
        assert_eq!(
            HueBand::forward(c.range_start, c.range_end),
            HueBand::forward(reds.range_start, reds.range_end)
        );
        assert_eq!(c.weight(180.0), 1.0);
    }

    #[test]
    fn include_and_exclude() {
        let mut reds = ColorRange::Reds.default_band();
        assert_eq!(reds.weight(100.0), 0.0);
        reds.include(100.0);
        assert_eq!(reds.weight(100.0), 1.0);
        reds.exclude(100.0);
        assert_eq!(reds.weight(100.0), 0.0);
    }

    #[test]
    fn set_handle_keeps_order() {
        let mut reds = ColorRange::Reds.default_band();
        reds.set_handle(1, 100.0); // would invert range start/end ordering
        // The invariant (rangeStart ≤ rangeEnd forward from falloff) is preserved or the move rejected.
        let to_start = HueBand::forward(reds.falloff_start, reds.range_start);
        let to_end = HueBand::forward(reds.falloff_start, reds.range_end);
        assert!(to_start <= to_end);
    }

    #[test]
    fn settings_slider_routing() {
        let mut s = HueSaturationSettings::default();
        s.range = ColorRange::Blues;
        s.set_hue(-30.0);
        assert_eq!(s.hue(), -30.0);
        assert_eq!(s.adjustments.get(&ColorRange::Master).map(|a| a.hue), None);
        assert!(s.is_identity() == false);
    }

    #[test]
    fn identity_is_noop() {
        let s = HueSaturationSettings::default();
        assert!(s.is_identity());
        let (r, g, b) = adjust_hsl(0.5, 0.25, 0.75, &s, None);
        assert!((r - 0.5).abs() < 1e-9 && (g - 0.25).abs() < 1e-9 && (b - 0.75).abs() < 1e-9);
    }

    #[test]
    fn hue_shift_rotates() {
        let mut s = HueSaturationSettings::default();
        s.set_hue(180.0);
        let (r, g, b) = adjust_hsl(1.0, 0.0, 0.0, &s, None);
        // Red shifts to cyan.
        assert!(g > 0.9 && b > 0.9 && r < 0.1, "{r} {g} {b}");
    }

    #[test]
    fn colorize_fills_with_hue() {
        let s = HueSaturationSettings::colorize_start();
        let (r, g, b) = adjust_hsl(0.2, 0.8, 0.4, &s, None);
        // A single hue comes out: no dominant-unpredictable channels, but not gray.
        assert!((r - g).abs() > 1e-6 || (g - b).abs() > 1e-6);
    }

    #[test]
    fn lightness_extremes() {
        let mut s = HueSaturationSettings::default();
        s.set_lightness(100.0);
        assert_eq!(adjust_hsl(0.3, 0.4, 0.5, &s, None), (1.0, 1.0, 1.0));
        s.set_lightness(-100.0);
        assert_eq!(adjust_hsl(0.3, 0.4, 0.5, &s, None), (0.0, 0.0, 0.0));
    }

    #[test]
    fn saturation_neutral_stays_neutral() {
        let mut s = HueSaturationSettings::default();
        s.set_saturation(50.0);
        let (r, g, b) = adjust_hsl(0.5, 0.5, 0.5, &s, None);
        assert!((r - 0.5).abs() < 1e-9 && (g - 0.5).abs() < 1e-9 && (b - 0.5).abs() < 1e-9);
    }

    #[test]
    fn invert_range_applies_outside() {
        let mut s = HueSaturationSettings::default();
        s.range = ColorRange::Reds;
        s.set_hue(60.0);
        // At hue 0 (inside the red band), weight 0.
        let (r0, _, _) = adjust_hsl(1.0, 0.0, 0.0, &s, None);
        assert!(r0 > 0.9);
        s.invert_range = true;
        // Now reds get no shift and everything else does: red stays red.
        let (r1, g1, b1) = adjust_hsl(1.0, 0.0, 0.0, &s, None);
        assert!(r1 > 0.9 && g1 < 0.1 && b1 < 0.1);
    }

    #[test]
    fn shifted_hue_matches_settings() {
        let mut s = HueSaturationSettings::default();
        s.set_hue(30.0);
        assert_eq!(shifted_hue(10.0, &s), 40.0);
    }

    /// The old per-pixel render path (exact f64 HSL, no cube), kept as the
    /// reference for the LUT-equivalence test: unpremultiply, `adjust_hsl`,
    /// premultiply.
    fn exact_pixel_apply(data: &[u8], settings: &HueSaturationSettings) -> Vec<u8> {
        let response = hue_response(settings);
        let mut out = data.to_vec();
        for p in out.chunks_exact_mut(4) {
            let a = p[3] as f64;
            if a == 0.0 {
                continue;
            }
            let r = (p[0] as f64 * 255.0 / a).min(255.0) / 255.0;
            let g = (p[1] as f64 * 255.0 / a).min(255.0) / 255.0;
            let b = (p[2] as f64 * 255.0 / a).min(255.0) / 255.0;
            let (nr, ng, nb) = adjust_hsl(r, g, b, settings, Some(&response));
            for (i, v) in [nr, ng, nb].iter().enumerate() {
                p[i] = ((v * 255.0).round() as u8).min(p[3]);
            }
        }
        out
    }

    /// A synthetic premultiplied RGBA8 image spanning the gamut, including
    /// translucent pixels (stored color ≤ alpha, as real premultiplied data is).
    fn synthetic_rgba8(width: u32, height: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let alpha = [255u8, 255, 170, 80][(x % 4) as usize];
                let unpremultiplied = [
                    ((x * 17) % 256) as u32,
                    ((y * 23) % 256) as u32,
                    ((x * 13 + y * 7) % 256) as u32,
                ];
                for u in unpremultiplied {
                    data.push(((u * alpha as u32 + 127) / 255) as u8);
                }
                data.push(alpha);
            }
        }
        data
    }

    /// §7.1-2 (a): with identity settings the cube must map every lattice
    /// point to itself (within f32 epsilon), alpha must be 1 everywhere, and
    /// the entry order must be blue outermost / red innermost.
    #[test]
    fn cube_identity_maps_every_lattice_point_to_itself() {
        let cube = ColorCube::build(&HueSaturationSettings::default());
        assert_eq!(cube.dimension(), CUBE_DIMENSION);
        assert_eq!(
            cube.data().len(),
            CUBE_DIMENSION * CUBE_DIMENSION * CUBE_DIMENSION * 4
        );
        let step = (CUBE_DIMENSION - 1) as f64;
        let mut index = 0;
        for blue in 0..CUBE_DIMENSION {
            for green in 0..CUBE_DIMENSION {
                for red in 0..CUBE_DIMENSION {
                    let expected = [red as f64 / step, green as f64 / step, blue as f64 / step];
                    for c in 0..3 {
                        let got = cube.data()[index + c] as f64;
                        assert!(
                            (got - expected[c]).abs() < 1e-6,
                            "lattice ({red},{green},{blue}) channel {c}: {got} != {}",
                            expected[c]
                        );
                    }
                    assert_eq!(cube.data()[index + 3], 1.0);
                    index += 4;
                }
            }
        }
        // Spot-check the index arithmetic against the sequential walk above.
        for &(r, g, b) in &[
            (1usize, 0usize, 0usize),
            (0, 5, 7),
            (32, 32, 32),
            (2, 31, 17),
        ] {
            let i = cube.index(r, g, b);
            let expected = [r as f64 / 32.0, g as f64 / 32.0, b as f64 / 32.0];
            for c in 0..3 {
                assert!((cube.data()[i + c] as f64 - expected[c]).abs() < 1e-6);
            }
        }
    }

    /// §7.1-2 (b): sampling strictly between lattice points blends the
    /// neighbors. With a cube that stores its own lattice coordinate per
    /// channel, the blend has an exactly representable f32 expectation.
    #[test]
    fn trilinear_sample_blends_between_lattice_points() {
        let dimension = CUBE_DIMENSION;
        let mut data = vec![0.0f32; dimension * dimension * dimension * 4];
        for b in 0..dimension {
            for g in 0..dimension {
                for r in 0..dimension {
                    let i = ((b * dimension + g) * dimension + r) * 4;
                    data[i] = r as f32 / 32.0;
                    data[i + 1] = g as f32 / 32.0;
                    data[i + 2] = b as f32 / 32.0;
                    data[i + 3] = 1.0;
                }
            }
        }
        let cube = ColorCube::from_data(dimension, data);

        // 0.515625 × 32 = 16.5: halfway between lattice 16 and 17 on green,
        // with red and blue exactly on lattice points 8 and 24.
        let got = cube.sample(0.25, 0.515625, 0.75);
        assert_eq!(got.0, 0.25);
        assert_eq!(got.1, 0.515625); // (16/32 + 17/32) / 2, exact in f32
        assert_eq!(got.2, 0.75);

        // Fractional on all three axes at once.
        let got = cube.sample(0.515625, 0.515625, 0.515625);
        assert_eq!(got.0, 0.515625);
        assert_eq!(got.1, 0.515625);
        assert_eq!(got.2, 0.515625);

        // Off-lattice midpoint of asymmetric lattice values (red 10↔11).
        let got = cube.sample(10.5 / 32.0, 0.0, 0.0);
        assert_eq!(got.0, 10.5 / 32.0);

        // Inputs outside 0…1 clamp to the cube's edge lattice points.
        let got = cube.sample(-0.5, 1.5, 0.5);
        assert_eq!(got.0, 0.0);
        assert_eq!(got.1, 1.0);
        assert_eq!(got.2, 0.5);
    }

    /// §7.1-2 (c): the cube path (what Swift's CIColorCube does) stays within
    /// a couple of 1/255 steps of the exact per-pixel f64 math — the residual
    /// is the f32 quantization of the 33-point LUT plus trilinear blending.
    #[test]
    fn cube_apply_matches_exact_math_within_quantization_tolerance() {
        let mut settings = HueSaturationSettings::default();
        settings.set_hue(30.0);
        settings.set_saturation(20.0);
        settings.set_lightness(-10.0);

        let data = synthetic_rgba8(16, 16);
        let exact = exact_pixel_apply(&data, &settings);

        let cube = ColorCube::build(&settings);
        let mut lut = data.clone();
        apply_cube_rgba8(&mut lut, &cube);

        let mut max_diff = 0u8;
        for (a, b) in exact.iter().zip(lut.iter()) {
            max_diff = max_diff.max(a.abs_diff(*b));
        }
        assert!(
            max_diff <= 3,
            "cube output diverges from exact math by {max_diff}/255"
        );
    }

    /// A per-range adjustment through the default reds band. Pixels whose
    /// whole lattice cell sits on one side of the band edge track the exact
    /// math within rounding; across the whole image the bound is wider
    /// because the band's hue edge falls inside single lattice cells, so
    /// near-edge pixels trilinearly blend adjusted and unadjusted corners —
    /// the CIColorCube behavior being ported, not a defect of this code (the
    /// measured worst case for these settings is 18/255).
    #[test]
    fn cube_apply_stays_close_for_range_adjustments() {
        let mut settings = HueSaturationSettings::default();
        settings.range = ColorRange::Reds;
        settings.set_hue(-60.0);
        settings.set_saturation(-30.0);
        settings.set_lightness(15.0);

        let data = synthetic_rgba8(16, 16);
        let exact = exact_pixel_apply(&data, &settings);
        let cube = ColorCube::build(&settings);
        let mut lut = data.clone();
        apply_cube_rgba8(&mut lut, &cube);

        let mut max_diff = 0u8;
        for (a, b) in exact.iter().zip(lut.iter()) {
            max_diff = max_diff.max(a.abs_diff(*b));
        }
        assert!(
            max_diff <= 20,
            "worst band-edge blend diverges from exact math by {max_diff}/255"
        );

        // Pure red (hue 0, deep inside the band: every lattice corner has
        // g = b = 0) and pure green (hue 120, far outside it) are exact
        // within rounding.
        let mut probes = vec![255u8, 0, 0, 255, 0, 255, 0, 255];
        let probe_exact = exact_pixel_apply(&probes, &settings);
        apply_cube_rgba8(&mut probes, &cube);
        for (a, b) in probe_exact.iter().zip(probes.iter()) {
            assert!(a.abs_diff(*b) <= 2, "probe {a} vs {b}");
        }
    }

    /// Colorize bakes one hue into every lattice entry; the LUT must still
    /// track the exact math.
    #[test]
    fn cube_apply_matches_exact_math_for_colorize() {
        let settings = HueSaturationSettings::colorize_start();
        let data = synthetic_rgba8(16, 16);
        let exact = exact_pixel_apply(&data, &settings);
        let cube = ColorCube::build(&settings);
        let mut lut = data.clone();
        apply_cube_rgba8(&mut lut, &cube);

        let mut max_diff = 0u8;
        for (a, b) in exact.iter().zip(lut.iter()) {
            max_diff = max_diff.max(a.abs_diff(*b));
        }
        assert!(
            max_diff <= 3,
            "cube output diverges from exact math by {max_diff}/255"
        );
    }
}
