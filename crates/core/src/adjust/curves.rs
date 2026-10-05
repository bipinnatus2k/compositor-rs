//! Port of `Compositor/Document/Curves.swift`.

use serde::{Deserialize, Serialize};

use super::levels::LevelsChannel;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurvesSettings {
    #[serde(default)]
    pub channel: LevelsChannel,
    #[serde(default = "default_channels")]
    pub channels: Vec<Vec<CurvePoint>>,
}

fn default_channels() -> Vec<Vec<CurvePoint>> {
    vec![
        default_curve(),
        default_curve(),
        default_curve(),
        default_curve(),
    ]
}

fn default_curve() -> Vec<CurvePoint> {
    vec![
        CurvePoint { x: 0.0, y: 0.0 },
        CurvePoint { x: 255.0, y: 255.0 },
    ]
}

impl Default for CurvesSettings {
    fn default() -> Self {
        CurvesSettings {
            channel: LevelsChannel::Rgb,
            channels: default_channels(),
        }
    }
}

impl CurvesSettings {
    pub fn is_valid(&self) -> bool {
        self.channels.len() == 4
            && self.channels.iter().all(|points| {
                (2..=32).contains(&points.len())
                    && points.first().map(|p| p.x) == Some(0.0)
                    && points.last().map(|p| p.x) == Some(255.0)
                    && points.iter().all(|p| {
                        p.x.is_finite()
                            && p.y.is_finite()
                            && (0.0..=255.0).contains(&p.x)
                            && (0.0..=255.0).contains(&p.y)
                    })
                    && points.windows(2).all(|w| w[0].x < w[1].x)
            })
    }

    /// Shape-preserving cubic Hermite interpolation avoids overshoot between
    /// handles (the exact spline from Curves.swift).
    pub fn value(&self, x: f64, channel: usize) -> f64 {
        let p = &self.channels[channel];
        let i = p
            .iter()
            .rposition(|pt| pt.x <= x)
            .unwrap_or(0)
            .min(p.len() - 2);
        let d: Vec<f64> = p
            .windows(2)
            .map(|w| (w[1].y - w[0].y) / (w[1].x - w[0].x))
            .collect();
        let slope = |j: usize| -> f64 {
            if j == 0 {
                return d[0];
            }
            if j == p.len() - 1 {
                return *d.last().unwrap();
            }
            if d[j - 1] * d[j] <= 0.0 {
                return 0.0;
            }
            2.0 / (1.0 / d[j - 1] + 1.0 / d[j])
        };
        let h = p[i + 1].x - p[i].x;
        let t = ((x - p[i].x) / h).min(1.0).max(0.0);
        let y = (2.0 * t * t * t - 3.0 * t * t + 1.0) * p[i].y
            + (t * t * t - 2.0 * t * t + t) * h * slope(i)
            + (-2.0 * t * t * t + 3.0 * t * t) * p[i + 1].y
            + (t * t * t - t * t) * h * slope(i + 1);
        y.min(255.0).max(0.0)
    }

    /// Applies the per-channel curves through the composite RGB curve, as the
    /// Swift `apply` does (channels 1…3 through channel 0).
    pub fn apply(&self, image: &crate::raster::Bitmap) -> crate::raster::Bitmap {
        // Swift guards isValid before applying (Curves.swift:34); invalid
        // settings leave the image untouched.
        if !self.is_valid() {
            return image.clone();
        }
        let mut result = image.clone();
        let mut table = [0.0f32; 768];
        for channel in 1..4usize {
            for v in 0..256usize {
                table[(channel - 1) * 256 + v] =
                    (self.value(self.value(v as f64, channel), 0) / 255.0) as f32;
            }
        }
        super::levels::levels_apply(&mut result.data, &table);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_curve() {
        let c = CurvesSettings::default();
        assert!(c.is_valid());
        for v in [0.0, 64.0, 128.0, 255.0] {
            assert!((c.value(v, 0) - v).abs() < 1e-9, "v={v}");
        }
    }

    #[test]
    fn mono_brighten_midpoint() {
        let mut c = CurvesSettings::default();
        c.channels[0] = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 128.0, y: 192.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        assert!(c.is_valid());
        assert_eq!(c.value(128.0, 0), 192.0);
        assert_eq!(c.value(0.0, 0), 0.0);
        assert_eq!(c.value(255.0, 0), 255.0);
        // No overshoot: shape-preserving Hermite keeps values inside the hull.
        assert!(c.value(64.0, 0) >= 64.0 && c.value(64.0, 0) <= 192.0);
    }

    #[test]
    fn plateau_is_clamped() {
        let mut c = CurvesSettings::default();
        c.channels[0] = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 100.0, y: 100.0 },
            CurvePoint { x: 155.0, y: 155.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        assert_eq!(c.value(100.0, 0), 100.0);
    }

    #[test]
    fn invalid_shapes_rejected() {
        let mut c = CurvesSettings::default();
        c.channels[0] = vec![
            CurvePoint { x: 10.0, y: 0.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        assert!(!c.is_valid()); // must start at 0
        c.channels[0] = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 100.0, y: 0.0 },
            CurvePoint { x: 50.0, y: 255.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        assert!(!c.is_valid()); // x not increasing
    }

    #[test]
    fn apply_changes_pixels() {
        let mut img = crate::raster::Bitmap::new(1, 1);
        img.set_pixel(0, 0, [128, 128, 128, 255]);
        let c = CurvesSettings::default();
        let out = c.apply(&img);
        assert_eq!(out.pixel(0, 0)[0], 128);
    }
}
