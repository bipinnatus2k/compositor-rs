//! Port of `LayerBlendMode` (`LayerAppearance.swift`) and the blend math the
//! Swift code gets from Core Image (`SeparableBlend.swift`, `CGContext` blend
//! modes). All 25 modes are computed with the standard PDF/Photoshop formulas
//! Core Image implements, in sRGB space.
//!
//! Colors are straight (unpremultiplied) floats 0–1 with alpha.

use serde::{Deserialize, Serialize};

/// Photoshop's modes in menu order (the Swift enum declaration order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerBlendMode {
    #[serde(rename = "Normal")]
    Normal,
    #[serde(rename = "Darken")]
    Darken,
    #[serde(rename = "Multiply")]
    Multiply,
    #[serde(rename = "Color Burn")]
    ColorBurn,
    #[serde(rename = "Linear Burn")]
    LinearBurn,
    #[serde(rename = "Lighten")]
    Lighten,
    #[serde(rename = "Screen")]
    Screen,
    #[serde(rename = "Color Dodge")]
    ColorDodge,
    #[serde(rename = "Linear Dodge (Add)")]
    LinearDodge,
    #[serde(rename = "Overlay")]
    Overlay,
    #[serde(rename = "Soft Light")]
    SoftLight,
    #[serde(rename = "Hard Light")]
    HardLight,
    #[serde(rename = "Vivid Light")]
    VividLight,
    #[serde(rename = "Linear Light")]
    LinearLight,
    #[serde(rename = "Pin Light")]
    PinLight,
    #[serde(rename = "Hard Mix")]
    HardMix,
    #[serde(rename = "Difference")]
    Difference,
    #[serde(rename = "Exclusion")]
    Exclusion,
    #[serde(rename = "Subtract")]
    Subtract,
    #[serde(rename = "Divide")]
    Divide,
    #[serde(rename = "Hue")]
    Hue,
    #[serde(rename = "Saturation")]
    Saturation,
    #[serde(rename = "Color")]
    Color,
    #[serde(rename = "Luminosity")]
    Luminosity,
}

impl LayerBlendMode {
    pub const ALL: [LayerBlendMode; 24] = [
        LayerBlendMode::Normal,
        LayerBlendMode::Darken,
        LayerBlendMode::Multiply,
        LayerBlendMode::ColorBurn,
        LayerBlendMode::LinearBurn,
        LayerBlendMode::Lighten,
        LayerBlendMode::Screen,
        LayerBlendMode::ColorDodge,
        LayerBlendMode::LinearDodge,
        LayerBlendMode::Overlay,
        LayerBlendMode::SoftLight,
        LayerBlendMode::HardLight,
        LayerBlendMode::VividLight,
        LayerBlendMode::LinearLight,
        LayerBlendMode::PinLight,
        LayerBlendMode::HardMix,
        LayerBlendMode::Difference,
        LayerBlendMode::Exclusion,
        LayerBlendMode::Subtract,
        LayerBlendMode::Divide,
        LayerBlendMode::Hue,
        LayerBlendMode::Saturation,
        LayerBlendMode::Color,
        LayerBlendMode::Luminosity,
    ];

    /// Photoshop's grouping: darkening modes together, then lightening, then
    /// contrast, then the comparative ones, then the component modes. The menu
    /// draws a line between each group.
    pub const GROUPS: [&[LayerBlendMode]; 6] = [
        &[LayerBlendMode::Normal],
        &[
            LayerBlendMode::Darken,
            LayerBlendMode::Multiply,
            LayerBlendMode::ColorBurn,
            LayerBlendMode::LinearBurn,
        ],
        &[
            LayerBlendMode::Lighten,
            LayerBlendMode::Screen,
            LayerBlendMode::ColorDodge,
            LayerBlendMode::LinearDodge,
        ],
        &[
            LayerBlendMode::Overlay,
            LayerBlendMode::SoftLight,
            LayerBlendMode::HardLight,
            LayerBlendMode::VividLight,
            LayerBlendMode::LinearLight,
            LayerBlendMode::PinLight,
            LayerBlendMode::HardMix,
        ],
        &[
            LayerBlendMode::Difference,
            LayerBlendMode::Exclusion,
            LayerBlendMode::Subtract,
            LayerBlendMode::Divide,
        ],
        &[
            LayerBlendMode::Hue,
            LayerBlendMode::Saturation,
            LayerBlendMode::Color,
            LayerBlendMode::Luminosity,
        ],
    ];

    pub fn label(&self) -> &'static str {
        match self {
            LayerBlendMode::Normal => "Normal",
            LayerBlendMode::Darken => "Darken",
            LayerBlendMode::Multiply => "Multiply",
            LayerBlendMode::ColorBurn => "Color Burn",
            LayerBlendMode::LinearBurn => "Linear Burn",
            LayerBlendMode::Lighten => "Lighten",
            LayerBlendMode::Screen => "Screen",
            LayerBlendMode::ColorDodge => "Color Dodge",
            LayerBlendMode::LinearDodge => "Linear Dodge (Add)",
            LayerBlendMode::Overlay => "Overlay",
            LayerBlendMode::SoftLight => "Soft Light",
            LayerBlendMode::HardLight => "Hard Light",
            LayerBlendMode::VividLight => "Vivid Light",
            LayerBlendMode::LinearLight => "Linear Light",
            LayerBlendMode::PinLight => "Pin Light",
            LayerBlendMode::HardMix => "Hard Mix",
            LayerBlendMode::Difference => "Difference",
            LayerBlendMode::Exclusion => "Exclusion",
            LayerBlendMode::Subtract => "Subtract",
            LayerBlendMode::Divide => "Divide",
            LayerBlendMode::Hue => "Hue",
            LayerBlendMode::Saturation => "Saturation",
            LayerBlendMode::Color => "Color",
            LayerBlendMode::Luminosity => "Luminosity",
        }
    }
}

#[inline]
fn b_separable(cb: f32, cs: f32, mode: LayerBlendMode) -> f32 {
    match mode {
        LayerBlendMode::Normal => cs,
        LayerBlendMode::Darken => cb.min(cs),
        LayerBlendMode::Multiply => cb * cs,
        LayerBlendMode::ColorBurn => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        LayerBlendMode::LinearBurn => (cb + cs - 1.0).max(0.0),
        LayerBlendMode::Lighten => cb.max(cs),
        LayerBlendMode::Screen => cb + cs - cb * cs,
        LayerBlendMode::ColorDodge => {
            if cb <= 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        LayerBlendMode::LinearDodge => (cb + cs).min(1.0),
        LayerBlendMode::Overlay => b_separable(cs, cb, LayerBlendMode::HardLight),
        LayerBlendMode::SoftLight => {
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                let d = if cb <= 0.25 {
                    ((16.0 * cb - 12.0) * cb + 4.0) * cb
                } else {
                    cb.sqrt()
                };
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
        LayerBlendMode::HardLight => {
            if cs <= 0.5 {
                cb * (2.0 * cs)
            } else {
                cb + (2.0 * cs - 1.0) - cb * (2.0 * cs - 1.0)
            }
        }
        LayerBlendMode::VividLight => {
            if cs <= 0.5 {
                b_separable(cb, (2.0 * cs).min(1.0), LayerBlendMode::ColorBurn)
            } else {
                b_separable(cb, (2.0 * (1.0 - cs)).min(1.0), LayerBlendMode::ColorDodge)
            }
        }
        LayerBlendMode::LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
        LayerBlendMode::PinLight => {
            if cs <= 0.5 {
                cb.min(2.0 * cs)
            } else {
                cb.max(2.0 * (cs - 0.5))
            }
        }
        LayerBlendMode::HardMix => {
            let vivid = b_separable(cb, cs, LayerBlendMode::VividLight);
            if vivid < 0.5 {
                0.0
            } else {
                1.0
            }
        }
        LayerBlendMode::Difference => (cb - cs).abs(),
        LayerBlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
        LayerBlendMode::Subtract => (cb - cs).max(0.0),
        LayerBlendMode::Divide => {
            if cs <= 0.0 {
                if cb <= 0.0 {
                    0.0
                } else {
                    1.0
                }
            } else {
                (cb / cs).min(1.0)
            }
        }
        LayerBlendMode::Hue | LayerBlendMode::Saturation | LayerBlendMode::Color | LayerBlendMode::Luminosity => {
            unreachable!("non-separable handled separately")
        }
    }
}

fn luminosity(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = luminosity(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        let l_n = l - n;
        let mut out = c;
        for v in out.iter_mut() {
            *v = l + (*v - l) * l / l_n;
        }
        return out;
    }
    if x > 1.0 {
        let l_x = l - x;
        let denom = 1.0 - l_x;
        if denom.abs() < f32::EPSILON {
            return [l, l, l];
        }
        let mut out = c;
        for v in out.iter_mut() {
            *v = l + (*v - l) * (1.0 - l) / denom;
        }
        return out;
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - luminosity(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// Smallest component, middle component, largest component indices.
fn order(c: [f32; 3]) -> (usize, usize, usize) {
    if c[0] <= c[1] {
        if c[1] <= c[2] {
            (0, 1, 2)
        } else if c[0] <= c[2] {
            (0, 2, 1)
        } else {
            (2, 0, 1)
        }
    } else if c[0] <= c[2] {
        (1, 0, 2)
    } else if c[1] <= c[2] {
        (1, 2, 0)
    } else {
        (2, 1, 0)
    }
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let (min_i, mid_i, max_i) = order(c);
    let cmin = c[min_i];
    let cmid = c[mid_i];
    let cmax = c[max_i];
    let mut out = [0.0f32; 3];
    if cmax > cmin {
        out[mid_i] = (cmid - cmin) * s / (cmax - cmin);
        out[max_i] = s;
    }
    out[min_i] = 0.0;
    out
}

fn b_non_separable(cb: [f32; 3], cs: [f32; 3], mode: LayerBlendMode) -> [f32; 3] {
    match mode {
        LayerBlendMode::Hue => set_lum(set_sat(cs, sat(cb)), luminosity(cb)),
        LayerBlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), luminosity(cb)),
        LayerBlendMode::Color => set_lum(cs, luminosity(cb)),
        LayerBlendMode::Luminosity => set_lum(cb, luminosity(cs)),
        _ => [
            b_separable(cb[0], cs[0], mode),
            b_separable(cb[1], cs[1], mode),
            b_separable(cb[2], cs[2], mode),
        ],
    }
}

/// The blended color `B(cb, cs)` for one pixel.
pub fn blend_color(mode: LayerBlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    match mode {
        LayerBlendMode::Hue | LayerBlendMode::Saturation | LayerBlendMode::Color | LayerBlendMode::Luminosity => {
            b_non_separable(cb, cs, mode)
        }
        _ => {
            let mut out = [0.0f32; 3];
            for i in 0..3 {
                out[i] = b_separable(cb[i], cs[i], mode);
            }
            out
        }
    }
}

/// Composites one source pixel onto a backdrop pixel in `mode` (the PDF
/// compositing equations Core Image implements): the source color is first
/// blended with the backdrop where the backdrop has coverage, then
/// source-overed with the source's alpha.
pub fn composite_pixel(
    mode: LayerBlendMode,
    cb: [f32; 4], // straight backdrop
    cs: [f32; 4], // straight source
) -> [f32; 4] {
    let ab = cb[3];
    let as_ = cs[3];
    let blended = if ab > 0.0 {
        blend_color(mode, [cb[0], cb[1], cb[2]], [cs[0], cs[1], cs[2]])
    } else {
        [cs[0], cs[1], cs[2]]
    };
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        let co = blended[i] * as_ + cb[i] * ab * (1.0 - as_);
        out[i] = co;
    }
    out[3] = as_ + ab * (1.0 - as_);
    if out[3] > 0.0 {
        out[0] /= out[3];
        out[1] /= out[3];
        out[2] /= out[3];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn serde_names_match_manifest() {
        let mode: LayerBlendMode = serde_json::from_str("\"Linear Dodge (Add)\"").unwrap();
        assert_eq!(mode, LayerBlendMode::LinearDodge);
        assert_eq!(serde_json::to_string(&LayerBlendMode::ColorBurn).unwrap(), "\"Color Burn\"");
    }

    #[test]
    fn normal_passthrough() {
        let out = composite_pixel(LayerBlendMode::Normal, [0.2, 0.4, 0.6, 1.0], [0.8, 0.2, 0.1, 1.0]);
        assert_eq!(out, [0.8, 0.2, 0.1, 1.0]);
    }

    #[test]
    fn multiply_blacks() {
        let out = blend_color(LayerBlendMode::Multiply, [0.5, 0.5, 0.5], [0.5, 0.5, 0.5]);
        assert_eq!(out, [0.25, 0.25, 0.25]);
    }

    #[test]
    fn screen_lightens_to_one_when_white() {
        let out = blend_color(LayerBlendMode::Screen, [0.4, 0.4, 0.4], [1.0, 1.0, 1.0]);
        assert_eq!(out, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn color_burn_ignores_transparency_correctly() {
        // 80% grey over 40% grey burns to 25% in Photoshop (per SeparableBlend.swift).
        let out = blend_color(LayerBlendMode::ColorBurn, [0.4, 0.4, 0.4], [0.8, 0.8, 0.8]);
        assert!(close(out[0], 0.25), "{out:?}");
    }

    #[test]
    fn color_dodge_reaches_one() {
        // 80% grey over 40% grey dodges to 100%.
        let out = blend_color(LayerBlendMode::ColorDodge, [0.4, 0.4, 0.4], [0.8, 0.8, 0.8]);
        assert!(close(out[0], 1.0), "{out:?}");
    }

    #[test]
    fn linear_burn_and_dodge() {
        let burn = blend_color(LayerBlendMode::LinearBurn, [0.6, 0.6, 0.6], [0.5, 0.5, 0.5]);
        assert!(close(burn[0], 0.1));
        let dodge = blend_color(LayerBlendMode::LinearDodge, [0.6, 0.6, 0.6], [0.5, 0.5, 0.5]);
        assert!(close(dodge[0], 1.0));
    }

    #[test]
    fn hard_mix_is_binary() {
        let dark = blend_color(LayerBlendMode::HardMix, [0.4, 0.4, 0.4], [0.3, 0.3, 0.3]);
        assert_eq!(dark, [0.0, 0.0, 0.0]);
        let light = blend_color(LayerBlendMode::HardMix, [0.6, 0.6, 0.6], [0.7, 0.7, 0.7]);
        assert_eq!(light, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn subtract_and_divide() {
        let sub = blend_color(LayerBlendMode::Subtract, [0.5, 0.5, 0.5], [0.2, 0.2, 0.2]);
        assert!(close(sub[0], 0.3));
        let div = blend_color(LayerBlendMode::Divide, [0.5, 0.5, 0.5], [0.5, 0.5, 0.5]);
        assert!(close(div[0], 1.0));
    }

    #[test]
    fn difference_symmetry() {
        let a = blend_color(LayerBlendMode::Difference, [0.7, 0.1, 0.4], [0.2, 0.9, 0.4]);
        assert!(close(a[0], 0.5) && close(a[1], 0.8) && close(a[2], 0.0), "{a:?}");
    }

    #[test]
    fn exclusion() {
        let out = blend_color(LayerBlendMode::Exclusion, [0.5, 0.5, 0.5], [0.5, 0.5, 0.5]);
        assert!(close(out[0], 0.5));
    }

    #[test]
    fn luminosity_mode_keeps_backdrop_lum() {
        // Color mode takes the source's color with the backdrop's luminosity.
        let cb = [0.9, 0.1, 0.1]; // red, lum = .3*.9+.59*.1+.11*.1 = .34
        let cs = [0.0, 1.0, 0.0]; // green
        let out = blend_color(LayerBlendMode::Color, cb, cs);
        let l = 0.3 * out[0] + 0.59 * out[1] + 0.11 * out[2];
        assert!(close(l, luminosity(cb)), "l={l}");
        // Hue should stay greenish.
        assert!(out[1] > out[0] && out[1] > out[2]);
    }

    #[test]
    fn overlay_soft_light_midpoints() {
        let out = blend_color(LayerBlendMode::Overlay, [0.5, 0.5, 0.5], [0.5, 0.5, 0.5]);
        assert!(close(out[0], 0.5));
        let soft = blend_color(LayerBlendMode::SoftLight, [0.5, 0.5, 0.5], [0.5, 0.5, 0.5]);
        assert!(close(soft[0], 0.5));
    }

    #[test]
    fn composite_alpha_blending() {
        // 50% transparent white over opaque black in Normal mode → mid grey.
        let out = composite_pixel(LayerBlendMode::Normal, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]);
        assert!(close(out[3], 1.0));
        assert!(close(out[0], 0.5));
    }

    #[test]
    fn vivid_light_extremes() {
        // Vivid light with black source burns to 0.
        let out = blend_color(LayerBlendMode::VividLight, [0.5, 0.5, 0.5], [0.0, 0.0, 0.0]);
        assert!(close(out[0], 0.0), "{out:?}");
    }

    #[test]
    fn pin_light() {
        let a = blend_color(LayerBlendMode::PinLight, [0.8, 0.8, 0.8], [0.2, 0.2, 0.2]);
        assert!(close(a[0], 0.4)); // min(0.8, 0.4)
        let b = blend_color(LayerBlendMode::PinLight, [0.8, 0.8, 0.8], [0.9, 0.9, 0.9]);
        assert!(close(b[0], 0.8)); // max(0.8, 0.8)
    }

    #[test]
    fn groups_match_menu() {
        assert_eq!(LayerBlendMode::ALL.len(), 24);
        let group_total: usize = LayerBlendMode::GROUPS.iter().map(|g| g.len()).sum();
        assert_eq!(group_total, 24);
    }
}
