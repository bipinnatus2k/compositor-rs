//! Port of `Compositor/Document/HueSaturation.swift` — the Hue/Saturation
//! adjustment (Photoshop's ⌘U), including the per-range bands, the colorize
//! mode, and the exact HSL conversion the Swift filter applies.

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
        if delta < 0.0 {
            delta + 360.0
        } else {
            delta
        }
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
            return if ramp_in > 0.0 { position / ramp_in } else { 1.0 };
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
        [self.falloff_start, self.range_start, self.range_end, self.falloff_end]
    }

    /// A band centered on one hue, keeping this band's core and shoulder widths.
    pub fn centered_on(&self, hue: f64) -> HueBand {
        let core = Self::forward(self.range_start, self.range_end);
        let leading = Self::forward(self.falloff_start, self.range_start);
        let trailing = Self::forward(self.range_end, self.falloff_end);
        fn wrap(value: f64) -> f64 {
            let r = value % 360.0;
            if r < 0.0 {
                r + 360.0
            } else {
                r
            }
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
            if r < 0.0 {
                r + 360.0
            } else {
                r
            }
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
    ColorRange::ALL.iter().map(|c| (*c, c.default_band())).collect()
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
        self.adjustments.get(&self.range).map(|a| a.hue).unwrap_or(0.0)
    }

    pub fn set_hue(&mut self, value: f64) {
        self.adjustment_mut().hue = value;
    }

    pub fn saturation(&self) -> f64 {
        self.adjustments.get(&self.range).map(|a| a.saturation).unwrap_or(0.0)
    }

    pub fn set_saturation(&mut self, value: f64) {
        self.adjustment_mut().saturation = value;
    }

    pub fn lightness(&self) -> f64 {
        self.adjustments.get(&self.range).map(|a| a.lightness).unwrap_or(0.0)
    }

    pub fn set_lightness(&mut self, value: f64) {
        self.adjustment_mut().lightness = value;
    }

    pub fn band(&self) -> HueBand {
        self.bands.get(&self.range).copied().unwrap_or_else(|| self.range.default_band())
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
            for (color_range, adjustment) in &settings.adjustments {
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
        let sampled = table[(hue.round() as usize).min(table.len() - 1).max(0)];
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
        assert_eq!(HueBand::forward(c.range_start, c.range_end), HueBand::forward(reds.range_start, reds.range_end));
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
}
