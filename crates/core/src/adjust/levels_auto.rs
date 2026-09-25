//! Port of `Compositor/Document/LevelsAutomatic.swift`: the Levels sheet's
//! Auto modes, which derive `LevelsSettings` from a `levels_histogram`.

use crate::adjust::levels::{LevelRange, LevelsSettings};

/// The Levels sheet's Auto buttons (`LevelsAuto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelsAuto {
    /// One shared input interval for all three channels, so their
    /// relationships survive.
    Contrast,
    /// An independent interval per channel.
    Color,
    /// Per-channel intervals plus a neutral midtone gamma per channel.
    Neutral,
}

impl LevelsAuto {
    pub const ALL: [LevelsAuto; 3] = [LevelsAuto::Contrast, LevelsAuto::Color, LevelsAuto::Neutral];

    /// The button label (`LevelsAuto`'s raw value).
    pub fn label(&self) -> &'static str {
        match self {
            LevelsAuto::Contrast => "Contrast",
            LevelsAuto::Color => "Color",
            LevelsAuto::Neutral => "Color + neutral midtones",
        }
    }

    /// `settings(histogram:)`: reads the 1024 bins of `levels_histogram` —
    /// `[0..256]` is the RGB mean, then red, green, blue — the same four
    /// 256-bin groups the Swift version receives. The composite RGB range
    /// (index 0) is only set by `Contrast`; `Color` and `Neutral` leave it
    /// untouched.
    pub fn settings(&self, histogram: &[f64; 1024]) -> LevelsSettings {
        let mut result = LevelsSettings::default();
        if *self == LevelsAuto::Contrast {
            // A shared interval preserves channel relationships.
            let limits: Vec<(f64, f64)> = (1..4).filter_map(|c| endpoints(channel_bins(histogram, c))).collect();
            let low = limits.iter().map(|limit| limit.0).fold(f64::INFINITY, f64::min);
            let high = limits.iter().map(|limit| limit.1).fold(f64::NEG_INFINITY, f64::max);
            if low < high {
                result.ranges[0] = LevelRange {
                    black: low,
                    white: high,
                    ..LevelRange::default()
                };
            }
        } else {
            for c in 1..4 {
                let bins = channel_bins(histogram, c);
                let Some((low, high)) = endpoints(bins) else {
                    continue;
                };
                let mut range = LevelRange {
                    black: low,
                    white: high,
                    ..LevelRange::default()
                };
                if *self == LevelsAuto::Neutral {
                    // Center the mean of the mapped histogram on 0.5.
                    let total: f64 = bins.iter().sum();
                    let mean = bins
                        .iter()
                        .enumerate()
                        .map(|(index, bin)| range.apply(index as f64 / 255.0) * bin)
                        .sum::<f64>()
                        / total;
                    if mean > 0.0 && mean < 1.0 {
                        range.gamma = 9.99f64.min(0.1f64.max(mean.ln() / 0.5f64.ln()));
                    }
                }
                result.ranges[c] = range;
            }
        }
        result
    }
}

fn channel_bins(histogram: &[f64; 1024], channel: usize) -> &[f64; 256] {
    histogram[channel * 256..channel * 256 + 256]
        .try_into()
        .expect("histogram holds four 256-bin groups")
}

/// `endpoints`: first and last bins whose cumulative count passes the 0.1%
/// clip, or `None` when the channel is empty or collapses to one bin.
fn endpoints(bins: &[f64; 256]) -> Option<(f64, f64)> {
    let total: f64 = bins.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let clip = total * 0.001;
    let mut sum = 0.0;
    let mut low = 0usize;
    let mut high = 255usize;
    for (index, bin) in bins.iter().enumerate() {
        sum += bin;
        if sum > clip {
            low = index;
            break;
        }
    }
    sum = 0.0;
    for index in (0..256).rev() {
        sum += bins[index];
        if sum > clip {
            high = index;
            break;
        }
    }
    (low < high).then_some((low as f64, high as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjust::levels::{LevelsChannel, levels_histogram};

    /// Two populated bins per channel, further apart on every channel:
    /// red 20/210, green 40/220, blue 60/230.
    fn spread_histogram() -> [f64; 1024] {
        let mut bins = [0.0; 1024];
        for c in 1..4 {
            bins[c * 256 + 20 * c] = 100.0;
            bins[c * 256 + 200 + c * 10] = 100.0;
        }
        bins
    }

    #[test]
    fn contrast_shares_one_interval_across_channels() {
        let linked = LevelsAuto::Contrast.settings(&spread_histogram());
        assert_eq!(linked.ranges[0].black, 20.0);
        assert_eq!(linked.ranges[0].white, 230.0);
        // The channel ranges stay at their defaults.
        assert!(linked.ranges[1..4].iter().all(|r| *r == LevelRange::default()));
    }

    #[test]
    fn color_sets_each_channel_independently() {
        let color = LevelsAuto::Color.settings(&spread_histogram());
        assert_eq!(color.ranges[1].black, 20.0);
        assert_eq!(color.ranges[2].black, 40.0);
        assert_eq!(color.ranges[3].black, 60.0);
        assert_eq!(color.ranges[3].white, 230.0);
        assert_eq!(color.ranges[0], LevelRange::default());
    }

    #[test]
    fn neutral_centers_the_histogram_mean() {
        // The two bins sit at the range's endpoints, so the mapped mean is
        // exactly 0.5 and the gamma is exactly 1.
        let neutral = LevelsAuto::Neutral.settings(&spread_histogram());
        assert_eq!(neutral.ranges[1].black, 20.0);
        assert_eq!(neutral.ranges[1].gamma, 1.0);
    }

    #[test]
    fn neutral_gamma_is_log_mean_over_log_half() {
        let mut bins = [0.0; 1024];
        // Bright-skewed: one dark bin keeps the interval valid, then 56 bins
        // crowd the top of the range.
        bins[256 + 10] = 1.0;
        for i in 200..=255 {
            bins[256 + i] = 1.0;
        }
        let neutral = LevelsAuto::Neutral.settings(&bins);
        let range = neutral.ranges[1];
        assert_eq!(range.black, 10.0);
        assert_eq!(range.white, 255.0);
        // The mean is taken with the interval's gamma still at 1.
        let linear = LevelRange {
            black: 10.0,
            white: 255.0,
            ..LevelRange::default()
        };
        let mean = (0..=255)
            .map(|i| linear.apply(i as f64 / 255.0) * bins[256 + i])
            .sum::<f64>()
            / 57.0;
        assert!((mean - 0.872180).abs() < 1e-5, "{mean}");
        assert!((range.gamma - mean.ln() / 0.5f64.ln()).abs() < 1e-9);
        assert!((range.gamma - 0.1973).abs() < 0.002, "{}", range.gamma);
    }

    #[test]
    fn neutral_gamma_clamps_bright_means_to_one_tenth() {
        // Almost every count maps to the white end, so the raw gamma runs
        // past the normalized floor and lands on exactly 0.1.
        let mut bins = [0.0; 1024];
        bins[256 + 10] = 3.0;
        bins[256 + 255] = 2000.0;
        assert_eq!(LevelsAuto::Neutral.settings(&bins).ranges[1].gamma, 0.1);
    }

    #[test]
    fn neutral_gamma_stays_within_the_normalized_bounds() {
        // Counts crowd the black end: gamma rises well past 1 but never
        // reaches the 9.99 cap.
        let mut bins = [0.0; 1024];
        bins[256 + 0] = 2000.0;
        bins[256 + 10] = 10.0;
        bins[256 + 255] = 1.0;
        let gamma = LevelsAuto::Neutral.settings(&bins).ranges[1].gamma;
        assert!(gamma > 0.1 && gamma < 9.99, "{gamma}");
        assert!((gamma - 7.514).abs() < 0.05, "{gamma}");
    }

    #[test]
    fn empty_and_collapsed_histograms_are_identity() {
        let empty = [0.0; 1024];
        for mode in LevelsAuto::ALL {
            assert!(mode.settings(&empty).is_identity(), "{:?}", mode);
        }
        // A single populated bin per channel has no usable interval.
        let mut solid = [0.0; 1024];
        for c in 1..4 {
            solid[c * 256 + 128] = 5.0;
        }
        for mode in LevelsAuto::ALL {
            assert!(mode.settings(&solid).is_identity(), "{:?}", mode);
        }
    }

    #[test]
    fn endpoints_clip_rare_tails() {
        // One count at bin 0 is under 0.1% of the total: the interval starts
        // at the first bin that pushes the running sum past the clip.
        let mut bins = [0.0; 256];
        bins[0] = 1.0;
        bins[10] = 1000.0;
        bins[250] = 1000.0;
        assert_eq!(endpoints(&bins), Some((10.0, 250.0)));
        // Symmetric single-bin histograms collapse to no interval.
        let mut single = [0.0; 256];
        single[128] = 5.0;
        assert_eq!(endpoints(&single), None);
        assert_eq!(endpoints(&[0.0; 256]), None);
    }

    #[test]
    fn contrast_on_image_histogram_shares_one_interval() {
        let mut bitmap = crate::raster::Bitmap::new(4, 1);
        for (x, v) in [10u8, 20, 200, 240].into_iter().enumerate() {
            bitmap.set_pixel(x, 0, [v, v, v, 255]);
        }
        let mut bins = [0.0; 1024];
        levels_histogram(&bitmap, None, &mut bins);
        let settings = LevelsAuto::Contrast.settings(&bins);
        assert_eq!(settings.ranges[0].black, 10.0);
        assert_eq!(settings.ranges[0].white, 240.0);
        // The clipped endpoints map to the full output range, channels first
        // then the composite range.
        assert_eq!(settings.apply(10.0 / 255.0, LevelsChannel::Red), 0.0);
        assert_eq!(settings.apply(240.0 / 255.0, LevelsChannel::Red), 1.0);
    }

    #[test]
    fn transparent_pixels_do_not_feed_the_endpoints() {
        let mut bitmap = crate::raster::Bitmap::new(3, 1);
        bitmap.set_pixel(0, 0, [0, 0, 0, 0]);
        bitmap.set_pixel(1, 0, [64, 64, 64, 255]);
        bitmap.set_pixel(2, 0, [192, 192, 192, 255]);
        let mut bins = [0.0; 1024];
        levels_histogram(&bitmap, None, &mut bins);
        let settings = LevelsAuto::Contrast.settings(&bins);
        assert_eq!(settings.ranges[0].black, 64.0);
        assert_eq!(settings.ranges[0].white, 192.0);
    }

    #[test]
    fn pure_black_white_and_gray_images_stay_identity() {
        for value in [0u8, 255] {
            let mut bitmap = crate::raster::Bitmap::new(2, 2);
            for y in 0..2 {
                for x in 0..2 {
                    bitmap.set_pixel(x, y, [value, value, value, 255]);
                }
            }
            let mut bins = [0.0; 1024];
            levels_histogram(&bitmap, None, &mut bins);
            for mode in LevelsAuto::ALL {
                assert!(mode.settings(&bins).is_identity(), "{:?} on {}", mode, value);
            }
        }
        // Mid-gray: every count lands in one bin, so the interval collapses.
        let mut bitmap = crate::raster::Bitmap::new(2, 1);
        bitmap.set_pixel(0, 0, [128, 128, 128, 255]);
        bitmap.set_pixel(1, 0, [128, 128, 128, 255]);
        let mut bins = [0.0; 1024];
        levels_histogram(&bitmap, None, &mut bins);
        for mode in LevelsAuto::ALL {
            assert!(mode.settings(&bins).is_identity(), "{:?}", mode);
        }
    }

    #[test]
    fn contrast_spanning_full_range_is_still_identity() {
        // black + white: the interval is 0…255, which is the default range.
        let mut bitmap = crate::raster::Bitmap::new(2, 1);
        bitmap.set_pixel(0, 0, [0, 0, 0, 255]);
        bitmap.set_pixel(1, 0, [255, 255, 255, 255]);
        let mut bins = [0.0; 1024];
        levels_histogram(&bitmap, None, &mut bins);
        assert_eq!(LevelsAuto::Contrast.settings(&bins), LevelsSettings::default());
    }
}
