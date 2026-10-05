//! Port of `Compositor/Document/Levels.swift` and `Rendering/LevelsPixels.c`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LevelsChannel {
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

impl LevelsChannel {
    pub const ALL: [LevelsChannel; 4] = [
        LevelsChannel::Rgb,
        LevelsChannel::Red,
        LevelsChannel::Green,
        LevelsChannel::Blue,
    ];

    pub fn index(&self) -> usize {
        match self {
            LevelsChannel::Rgb => 0,
            LevelsChannel::Red => 1,
            LevelsChannel::Green => 2,
            LevelsChannel::Blue => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LevelRange {
    #[serde(default)]
    pub black: f64,
    #[serde(default)]
    pub gamma: f64,
    #[serde(default = "default_white")]
    pub white: f64,
    #[serde(default)]
    pub output_black: f64,
    #[serde(default = "default_white")]
    pub output_white: f64,
}

fn default_white() -> f64 {
    255.0
}

impl Default for LevelRange {
    fn default() -> Self {
        LevelRange {
            black: 0.0,
            gamma: 1.0,
            white: 255.0,
            output_black: 0.0,
            output_white: 255.0,
        }
    }
}

impl LevelRange {
    pub fn normalized(&self) -> LevelRange {
        fn clamp(n: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
            if n.is_finite() {
                n.min(hi).max(lo)
            } else {
                fallback
            }
        }
        let mut result = *self;
        result.black = clamp(result.black, 0.0, 254.0, 0.0);
        result.white = clamp(result.white, result.black + 1.0, 255.0, 255.0);
        result.gamma = clamp(result.gamma, 0.1, 9.99, 1.0);
        result.output_black = clamp(result.output_black, 0.0, 255.0, 0.0);
        result.output_white = clamp(result.output_white, 0.0, 255.0, 255.0);
        result
    }

    /// Input 0–1 → output 0–1.
    pub fn apply(&self, value: f64) -> f64 {
        let s = self.normalized();
        let input = ((value * 255.0 - s.black) / (s.white - s.black))
            .min(1.0)
            .max(0.0);
        (s.output_black + input.powf(1.0 / s.gamma) * (s.output_white - s.output_black)) / 255.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct LevelsSettings {
    #[serde(default)]
    pub channel: LevelsChannel,
    #[serde(default = "default_ranges")]
    pub ranges: [LevelRange; 4],
}

fn default_ranges() -> [LevelRange; 4] {
    [LevelRange::default(); 4]
}

impl LevelsSettings {
    pub fn current(&self) -> LevelRange {
        self.ranges[self.channel.index()]
    }

    pub fn set_current(&mut self, value: LevelRange) {
        self.ranges[self.channel.index()] = value.normalized();
    }

    pub fn is_identity(&self) -> bool {
        self.ranges
            .iter()
            .all(|r| r.normalized() == LevelRange::default())
    }

    /// Individual channels, followed by the composite RGB adjustment.
    pub fn apply(&self, value: f64, channel: LevelsChannel) -> f64 {
        self.ranges[0].apply(self.ranges[channel.index()].apply(value))
    }
}

/// `levels_apply` (LevelsPixels.c): applies three 256-entry tables to the RGB
/// channels of a premultiplied RGBA buffer, unpremultiplying around the lookup.
pub fn levels_apply(pixels: &mut [u8], tables: &[f32; 768]) {
    for p in pixels.chunks_exact_mut(4) {
        let alpha = p[3] as f32;
        if alpha == 0.0 {
            continue;
        }
        for channel in 0..3 {
            let x = (255.0f32).min(p[channel] as f32 * 255.0 / alpha);
            let lo = x as usize;
            let hi = if lo < 255 { lo + 1 } else { 255 };
            let table = &tables[channel * 256..channel * 256 + 256];
            let result = table[lo] + (table[hi] - table[lo]) * (x - lo as f32);
            p[channel] = (result * alpha).round().clamp(0.0, alpha) as u8;
        }
    }
}

/// `levels_histogram` (LevelsPixels.c): fills 1024 bins — bin `[0..256]` is RGB
/// (the mean of the three channel histograms, not a luminance histogram), then
/// red, green, blue. `coverage` weights each pixel when a selection clips it.
pub fn levels_histogram(
    pixels: &crate::raster::Bitmap,
    coverage: Option<&crate::raster::GrayMask>,
    bins: &mut [f64; 1024],
) {
    for (i, px) in pixels.data.chunks_exact(4).enumerate() {
        if px[3] == 0 {
            continue;
        }
        let weight =
            px[3] as f64 / 255.0 * coverage.map(|c| c.data[i] as f64 / 255.0).unwrap_or(1.0);
        for channel in 0..3 {
            let value = ((px[channel] as f64 * 255.0 / px[3] as f64).round() as usize).min(255);
            bins[(channel + 1) * 256 + value] += weight;
            bins[value] += weight / 3.0;
        }
    }
}

/// Display-only vertical scaling. Keep linear bin ratios, but cap isolated
/// spikes so large solid backgrounds cannot flatten the useful distribution.
pub struct LevelsHistogramDisplay;

impl LevelsHistogramDisplay {
    pub fn scale(bins: &[f64]) -> f64 {
        let peak = bins
            .iter()
            .copied()
            .filter(|b| b.is_finite() && *b > 0.0)
            .fold(0.0, f64::max);
        if peak <= 0.0 {
            return 0.0;
        }
        let mut interior: Vec<f64> = bins
            .iter()
            .skip(1)
            .take(bins.len().saturating_sub(2))
            .copied()
            .filter(|b| b.is_finite() && *b > 0.0)
            .collect();
        if interior.is_empty() {
            return peak;
        }
        interior.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let typical_peak = interior[((interior.len() - 1) as f64 * 0.95) as usize];
        peak.min(typical_peak * 4.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_range_identity() {
        let r = LevelRange::default();
        assert!((r.apply(0.0) - 0.0).abs() < 1e-9);
        assert!((r.apply(1.0) - 1.0).abs() < 1e-9);
        assert!((r.apply(0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    // `0.70710678` is the ported XCTest's assertion literal, kept verbatim.
    #[allow(clippy::approx_constant)]
    fn level_range_gamma_brightens_midtones() {
        let r = LevelRange {
            gamma: 2.0,
            ..Default::default()
        };
        // 50% in, gamma 2 → sqrt(0.5) ≈ 0.707.
        assert!((r.apply(0.5) - 0.70710678).abs() < 1e-6);
    }

    #[test]
    fn level_range_in_out_points() {
        let r = LevelRange {
            black: 50.0,
            white: 200.0,
            output_black: 25.0,
            output_white: 230.0,
            ..Default::default()
        };
        assert!((r.apply(50.0 / 255.0) - 25.0 / 255.0).abs() < 1e-9);
        assert!((r.apply(200.0 / 255.0) - 230.0 / 255.0).abs() < 1e-9);
        assert_eq!(r.apply(-1.0), 25.0 / 255.0);
        assert_eq!(r.apply(2.0), 230.0 / 255.0);
    }

    #[test]
    fn normalization_clamps() {
        let r = LevelRange {
            black: 300.0,
            gamma: 100.0,
            white: -10.0,
            output_black: -5.0,
            output_white: 1000.0,
        }
        .normalized();
        assert_eq!(r.black, 254.0);
        assert_eq!(r.white, 255.0);
        assert_eq!(r.gamma, 9.99);
        assert_eq!(r.output_black, 0.0);
        assert_eq!(r.output_white, 255.0);
    }

    #[test]
    fn composite_applies_channel_then_rgb() {
        let mut s = LevelsSettings::default();
        s.ranges[1] = LevelRange {
            gamma: 2.0,
            ..Default::default()
        };
        s.ranges[0] = LevelRange {
            gamma: 2.0,
            ..Default::default()
        };
        let v = s.apply(0.25, LevelsChannel::Red);
        assert!((v - 0.5f64.sqrt()).abs() < 1e-6, "{v}");
    }

    #[test]
    fn levels_apply_table_lookup() {
        let mut table = [0.0f32; 768];
        for c in 0..3 {
            for v in 0..256 {
                table[c * 256 + v] = (v as f32 / 255.0) * 0.5; // halve everything
            }
        }
        let mut px = vec![255, 255, 255, 255, 128, 128, 128, 255];
        levels_apply(&mut px, &table);
        assert_eq!(px[0], 128); // 255*0.5 = 127.5 → 128
        assert_eq!(px[4], 64); // 128*0.5 = 64
    }

    #[test]
    fn levels_apply_preserves_alpha_and_unpremultiplies() {
        let mut table = [0.0f32; 768];
        for c in 0..3 {
            for v in 0..256 {
                table[c * 256 + v] = 1.0 - v as f32 / 255.0; // invert
            }
        }
        // 50% grey at 50% alpha (premultiplied = 64,64,64,128).
        let mut px = vec![64, 64, 64, 128];
        levels_apply(&mut px, &table);
        // Unpremultiplied 128/255 → inverted 127/255 → premultiplied ≈ 64.
        assert!(px[0] >= 62 && px[0] <= 66, "{}", px[0]);
        assert_eq!(px[3], 128);
    }

    #[test]
    fn histogram_bins_channels_and_rgb() {
        let mut b = crate::raster::Bitmap::new(1, 1);
        b.set_pixel(0, 0, [255, 0, 128, 255]);
        let mut bins = [0.0; 1024];
        levels_histogram(&b, None, &mut bins);
        assert!((bins[255] - 1.0 / 3.0).abs() < 1e-9); // RGB bin at 255
        assert!((bins[256 + 255] - 1.0).abs() < 1e-9); // red bin
        assert!((bins[2 * 256] - 1.0).abs() < 1e-9); // green bin
        assert!((bins[3 * 256 + 128] - 1.0).abs() < 1e-9); // blue bin
    }

    #[test]
    fn histogram_display_caps_spikes() {
        let mut bins = vec![0.0; 256];
        bins[0] = 1000.0;
        for b in bins.iter_mut().skip(1) {
            *b = 1.0;
        }
        let scale = LevelsHistogramDisplay::scale(&bins);
        // typical_peak*4 = 4 caps the 1000 spike.
        assert!((scale - 4.0).abs() < 1e-9);
    }
}
