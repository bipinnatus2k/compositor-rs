//! Port of `Compositor/Document/ImageAdjustments.swift` and the matching
//! kernels in `Rendering/AdjustPixels.c` (gradient map, black & white, color
//! balance, grain). Pixel buffers are premultiplied RGBA8, exactly as the C.

use serde::{Deserialize, Serialize};

use crate::raster::Bitmap;

pub fn clamp_double(value: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.min(hi).max(lo)
    } else {
        fallback
    }
}

/// A straight sRGB color stored with an adjustment, 0–1 per channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentColor {
    #[serde(default)]
    pub red: f64,
    #[serde(default)]
    pub green: f64,
    #[serde(default)]
    pub blue: f64,
}

impl AdjustmentColor {
    pub fn new(red: f64, green: f64, blue: f64) -> Self {
        AdjustmentColor { red, green, blue }
    }

    pub fn from_palette(c: crate::document::PaletteColor) -> Self {
        AdjustmentColor::new(c.red, c.green, c.blue)
    }

    pub fn is_valid(&self) -> bool {
        [self.red, self.green, self.blue]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }

    pub fn clamped(&self) -> Self {
        AdjustmentColor::new(
            clamp_double(self.red, 0.0, 1.0, 0.0),
            clamp_double(self.green, 0.0, 1.0, 0.0),
            clamp_double(self.blue, 0.0, 1.0, 0.0),
        )
    }
}

/// Photoshop's Exposure: `exposure` (stops) scales linear light and `offset`
/// shifts it, then gamma correction bends the result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExposureSettings {
    /// Stops of light, −20…20.
    #[serde(default)]
    pub exposure: f64,
    /// Added in linear light, −0.5…0.5.
    #[serde(default)]
    pub offset: f64,
    /// Gamma correction, 0.01…9.99; above 1 brightens the midtones.
    #[serde(default = "default_gamma")]
    pub gamma: f64,
}

fn default_gamma() -> f64 {
    1.0
}

impl Default for ExposureSettings {
    fn default() -> Self {
        ExposureSettings {
            exposure: 0.0,
            offset: 0.0,
            gamma: 1.0,
        }
    }
}

impl ExposureSettings {
    pub const EXPOSURE_RANGE: (f64, f64) = (-20.0, 20.0);
    pub const OFFSET_RANGE: (f64, f64) = (-0.5, 0.5);
    pub const GAMMA_RANGE: (f64, f64) = (0.01, 9.99);

    pub fn is_valid(&self) -> bool {
        (Self::EXPOSURE_RANGE.0..=Self::EXPOSURE_RANGE.1).contains(&self.exposure)
            && (Self::OFFSET_RANGE.0..=Self::OFFSET_RANGE.1).contains(&self.offset)
            && (Self::GAMMA_RANGE.0..=Self::GAMMA_RANGE.1).contains(&self.gamma)
    }

    pub fn normalized(&self) -> Self {
        ExposureSettings {
            exposure: clamp_double(
                self.exposure,
                Self::EXPOSURE_RANGE.0,
                Self::EXPOSURE_RANGE.1,
                0.0,
            ),
            offset: clamp_double(self.offset, Self::OFFSET_RANGE.0, Self::OFFSET_RANGE.1, 0.0),
            gamma: clamp_double(self.gamma, Self::GAMMA_RANGE.0, Self::GAMMA_RANGE.1, 1.0),
        }
    }

    /// Each channel's output (0–1) for each input byte, decoded to linear
    /// light and encoded back.
    pub fn table(&self) -> [f32; 256] {
        let scale = 2.0f64.powf(self.exposure);
        let mut table = [0.0f32; 256];
        for (index, out) in table.iter_mut().enumerate() {
            let encoded = index as f64 / 255.0;
            let mut linear = if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            };
            linear = (linear * scale + self.offset)
                .max(0.0)
                .powf(1.0 / self.gamma);
            let output = if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            *out = output.min(1.0).max(0.0) as f32;
        }
        table
    }

    pub fn apply(&self, image: &Bitmap) -> Bitmap {
        // Swift guards isValid before applying; invalid settings are a no-op.
        if !self.is_valid() {
            return image.clone();
        }
        let mut result = image.clone();
        let t = self.table();
        let mut tables = [0.0f32; 768];
        for c in 0..3 {
            tables[c * 256..c * 256 + 256].copy_from_slice(&t);
        }
        super::levels::levels_apply(&mut result.data, &tables);
        result
    }
}

/// Gradient Map: each pixel's brightness picks a color between `shadows` and
/// `highlights` (the other way round when reversed); alpha is kept.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientMapSettings {
    #[serde(default = "default_black")]
    pub shadows: AdjustmentColor,
    #[serde(default = "default_white")]
    pub highlights: AdjustmentColor,
    #[serde(default)]
    pub reversed: bool,
}

fn default_black() -> AdjustmentColor {
    AdjustmentColor::new(0.0, 0.0, 0.0)
}
fn default_white() -> AdjustmentColor {
    AdjustmentColor::new(1.0, 1.0, 1.0)
}

impl Default for GradientMapSettings {
    fn default() -> Self {
        GradientMapSettings {
            shadows: default_black(),
            highlights: default_white(),
            reversed: false,
        }
    }
}

impl GradientMapSettings {
    pub fn is_valid(&self) -> bool {
        self.shadows.is_valid() && self.highlights.is_valid()
    }

    pub fn normalized(&self) -> Self {
        GradientMapSettings {
            shadows: self.shadows.clamped(),
            highlights: self.highlights.clamped(),
            reversed: self.reversed,
        }
    }

    pub fn apply(&self, image: &Bitmap) -> Bitmap {
        if !self.is_valid() {
            return image.clone();
        }
        let mut result = image.clone();
        adjust_gradient_map(&mut result, &self.table());
        result
    }

    /// The 256-entry RGB table the C kernel reads.
    pub fn table(&self) -> Vec<u8> {
        let (dark, light) = self.ends();
        fn channel(from: f64, to: f64, t: f64) -> u8 {
            let value = from + (to - from) * t;
            ((value * 255.0).round().min(255.0).max(0.0)) as u8
        }
        let mut table = Vec::with_capacity(256 * 3);
        for index in 0..=255usize {
            let t = index as f64 / 255.0;
            table.push(channel(dark.red, light.red, t));
            table.push(channel(dark.green, light.green, t));
            table.push(channel(dark.blue, light.blue, t));
        }
        table
    }

    fn ends(&self) -> (AdjustmentColor, AdjustmentColor) {
        if self.reversed {
            (self.highlights, self.shadows)
        } else {
            (self.shadows, self.highlights)
        }
    }
}

/// `adjust_gradient_map` (AdjustPixels.c): Rec.709 brightness picks the table
/// color; premultiplied pixels are unpremultiplied around the lookup.
pub fn adjust_gradient_map(bitmap: &mut Bitmap, table: &[u8]) {
    for p in bitmap.data.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 0 {
            continue;
        }
        let mut r = p[0] as u32;
        let mut g = p[1] as u32;
        let mut b = p[2] as u32;
        if a < 255 {
            r = (r * 255 + a / 2) / a;
            g = (g * 255 + a / 2) / a;
            b = (b * 255 + a / 2) / a;
            r = r.min(255);
            g = g.min(255);
            b = b.min(255);
        }
        let level = ((2126 * r + 7152 * g + 722 * b + 5000) / 10000) as usize;
        let color = &table[level.min(255) * 3..level.min(255) * 3 + 3];
        p[0] = ((color[0] as u32 * a + 127) / 255) as u8;
        p[1] = ((color[1] as u32 * a + 127) / 255) as u8;
        p[2] = ((color[2] as u32 * a + 127) / 255) as u8;
    }
}

/// Black & White: not a desaturation, but a choice of how bright each family
/// of colors becomes in gray (Photoshop's weights).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlackWhiteSettings {
    /// Photoshop's defaults.
    #[serde(default = "default_40")]
    pub reds: f64,
    #[serde(default = "default_60")]
    pub yellows: f64,
    #[serde(default = "default_40")]
    pub greens: f64,
    #[serde(default = "default_60")]
    pub cyans: f64,
    #[serde(default = "default_20")]
    pub blues: f64,
    #[serde(default = "default_80")]
    pub magentas: f64,
    /// Color the result while keeping its tones, for a sepia or a cyanotype.
    #[serde(default)]
    pub tint: bool,
    #[serde(default = "default_40d")]
    pub tint_hue: f64,
    #[serde(default = "default_20d")]
    pub tint_saturation: f64,
}

fn default_40() -> f64 {
    40.0
}
fn default_60() -> f64 {
    60.0
}
fn default_20() -> f64 {
    20.0
}
fn default_80() -> f64 {
    80.0
}
fn default_40d() -> f64 {
    40.0
}
fn default_20d() -> f64 {
    20.0
}

impl Default for BlackWhiteSettings {
    fn default() -> Self {
        BlackWhiteSettings {
            reds: 40.0,
            yellows: 60.0,
            greens: 40.0,
            cyans: 60.0,
            blues: 20.0,
            magentas: 80.0,
            tint: false,
            tint_hue: 40.0,
            tint_saturation: 20.0,
        }
    }
}

impl BlackWhiteSettings {
    pub const RANGE: (f64, f64) = (-200.0, 300.0);

    pub fn is_valid(&self) -> bool {
        [
            self.reds,
            self.yellows,
            self.greens,
            self.cyans,
            self.blues,
            self.magentas,
        ]
        .iter()
        .all(|v| v.is_finite() && (Self::RANGE.0..=Self::RANGE.1).contains(v))
            && self.tint_hue.is_finite()
            && (0.0..=360.0).contains(&self.tint_hue)
            && self.tint_saturation.is_finite()
            && (0.0..=100.0).contains(&self.tint_saturation)
    }

    pub fn apply(&self, image: &Bitmap) -> Bitmap {
        if !self.is_valid() {
            return image.clone();
        }
        let mut result = image.clone();
        let weights: [f32; 6] = [
            (self.reds / 100.0) as f32,
            (self.yellows / 100.0) as f32,
            (self.greens / 100.0) as f32,
            (self.cyans / 100.0) as f32,
            (self.blues / 100.0) as f32,
            (self.magentas / 100.0) as f32,
        ];
        adjust_black_white(
            &mut result,
            &weights,
            self.tint,
            self.tint_hue,
            self.tint_saturation / 100.0,
        );
        result
    }
}

/// `adjust_black_white` (AdjustPixels.c).
pub fn adjust_black_white(
    bitmap: &mut Bitmap,
    weights: &[f32; 6],
    tint: bool,
    tint_hue: f64,
    tint_saturation: f64,
) {
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f32;
        if alpha == 0.0 {
            continue;
        }
        let r = (p[0] as f32 * 255.0 / alpha).min(255.0) / 255.0;
        let g = (p[1] as f32 * 255.0 / alpha).min(255.0) / 255.0;
        let b = (p[2] as f32 * 255.0 / alpha).min(255.0) / 255.0;
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let md = r + g + b - mx - mn;
        // weights: 0 red, 1 yellow, 2 green, 3 cyan, 4 blue, 5 magenta
        let (primary, secondary);
        if mx == r {
            primary = 0;
            secondary = if g >= b { 1 } else { 5 };
        } else if mx == g {
            primary = 2;
            secondary = if r >= b { 1 } else { 3 };
        } else {
            primary = 4;
            secondary = if g >= r { 3 } else { 5 };
        }
        let mut gray = mn + (md - mn) * weights[secondary] + (mx - md) * weights[primary];
        gray = gray.min(1.0).max(0.0);
        let mut out = [gray, gray, gray];
        if tint && tint_saturation > 0.0 {
            // The gray becomes the lightness of a color at the chosen hue.
            let c = (1.0 - (2.0 * gray as f64 - 1.0).abs()) * tint_saturation;
            let hp = (tint_hue % 360.0) / 60.0;
            let xx = c * (1.0 - (hp % 2.0 - 1.0).abs());
            let (r1, g1, b1);
            if hp < 1.0 {
                r1 = c;
                g1 = xx;
                b1 = 0.0;
            } else if hp < 2.0 {
                r1 = xx;
                g1 = c;
                b1 = 0.0;
            } else if hp < 3.0 {
                r1 = 0.0;
                g1 = c;
                b1 = xx;
            } else if hp < 4.0 {
                r1 = 0.0;
                g1 = xx;
                b1 = c;
            } else if hp < 5.0 {
                r1 = xx;
                g1 = 0.0;
                b1 = c;
            } else {
                r1 = c;
                g1 = 0.0;
                b1 = xx;
            }
            let m = gray as f64 - c / 2.0;
            out = [
                (r1 + m).min(1.0).max(0.0) as f32,
                (g1 + m).min(1.0).max(0.0) as f32,
                (b1 + m).min(1.0).max(0.0) as f32,
            ];
        }
        p[0] = (out[0] * alpha).round().min(alpha).max(0.0) as u8;
        p[1] = (out[1] * alpha).round().min(alpha).max(0.0) as u8;
        p[2] = (out[2] * alpha).round().min(alpha).max(0.0) as u8;
    }
}

/// Color Balance: shifts color towards one end of each opposing pair,
/// separately for shadows, midtones and highlights.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ColorBalanceSettings {
    #[serde(default)]
    pub shadow_cyan_red: f64,
    #[serde(default)]
    pub shadow_magenta_green: f64,
    #[serde(default)]
    pub shadow_yellow_blue: f64,
    #[serde(default)]
    pub mid_cyan_red: f64,
    #[serde(default)]
    pub mid_magenta_green: f64,
    #[serde(default)]
    pub mid_yellow_blue: f64,
    #[serde(default)]
    pub highlight_cyan_red: f64,
    #[serde(default)]
    pub highlight_magenta_green: f64,
    #[serde(default)]
    pub highlight_yellow_blue: f64,
    #[serde(default = "default_true_literal")]
    pub preserve_luminosity: bool,
}

fn default_true_literal() -> bool {
    true
}

impl ColorBalanceSettings {
    pub const RANGE: (f64, f64) = (-100.0, 100.0);

    fn all(&self) -> [f64; 9] {
        [
            self.shadow_cyan_red,
            self.shadow_magenta_green,
            self.shadow_yellow_blue,
            self.mid_cyan_red,
            self.mid_magenta_green,
            self.mid_yellow_blue,
            self.highlight_cyan_red,
            self.highlight_magenta_green,
            self.highlight_yellow_blue,
        ]
    }

    pub fn is_valid(&self) -> bool {
        self.all()
            .iter()
            .all(|v| v.is_finite() && (Self::RANGE.0..=Self::RANGE.1).contains(v))
    }

    pub fn is_identity(&self) -> bool {
        self.all().iter().all(|v| *v == 0.0)
    }

    pub fn apply(&self, image: &Bitmap) -> Bitmap {
        if !self.is_valid() {
            return image.clone();
        }
        let mut result = image.clone();
        if self.is_identity() {
            return result;
        }
        let shadows = [
            (self.shadow_cyan_red / 100.0) as f32,
            (self.shadow_magenta_green / 100.0) as f32,
            (self.shadow_yellow_blue / 100.0) as f32,
        ];
        let midtones = [
            (self.mid_cyan_red / 100.0) as f32,
            (self.mid_magenta_green / 100.0) as f32,
            (self.mid_yellow_blue / 100.0) as f32,
        ];
        let highlights = [
            (self.highlight_cyan_red / 100.0) as f32,
            (self.highlight_magenta_green / 100.0) as f32,
            (self.highlight_yellow_blue / 100.0) as f32,
        ];
        adjust_color_balance(
            &mut result,
            &shadows,
            &midtones,
            &highlights,
            self.preserve_luminosity,
        );
        result
    }
}

/// How much a tone belongs to the shadows, midtones and highlights (AdjustPixels.c).
fn tonal_weights(v: f32) -> (f32, f32, f32) {
    const A: f32 = 0.25;
    const B: f32 = 0.333;
    const SCALE: f32 = 0.7;
    let mut s = (v - B) / -A + 0.5;
    let mut h = (v + B - 1.0) / A + 0.5;
    s = s.min(1.0).max(0.0);
    h = h.min(1.0).max(0.0);
    let m1 = ((v - B) / A + 0.5).min(1.0).max(0.0);
    let m2 = ((v + B - 1.0) / -A + 0.5).min(1.0).max(0.0);
    (s * SCALE, m1 * m2 * SCALE, h * SCALE)
}

/// `adjust_color_balance` (AdjustPixels.c).
pub fn adjust_color_balance(
    bitmap: &mut Bitmap,
    shadows: &[f32; 3],
    midtones: &[f32; 3],
    highlights: &[f32; 3],
    preserve_luminosity: bool,
) {
    for p in bitmap.data.chunks_exact_mut(4) {
        let alpha = p[3] as f32;
        if alpha == 0.0 {
            continue;
        }
        let mut c = [0.0f32; 3];
        for i in 0..3 {
            c[i] = (p[i] as f32 * 255.0 / alpha).min(255.0) / 255.0;
        }
        let before = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        for i in 0..3 {
            let (s, m, h) = tonal_weights(c[i]);
            c[i] += shadows[i] * s + midtones[i] * m + highlights[i] * h;
            c[i] = c[i].min(1.0).max(0.0);
        }
        if preserve_luminosity {
            let after = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
            if after > 0.0001 {
                let ratio = before / after;
                for v in c.iter_mut() {
                    *v = (*v * ratio).min(1.0).max(0.0);
                }
            }
        }
        for i in 0..3 {
            p[i] = (c[i] * alpha).round().min(alpha).max(0.0) as u8;
        }
    }
}

/// Film grain: brightness noise, strongest in the midtones, fixed in document
/// space by `seed`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GrainSettings {
    /// Strength, 0–100.
    #[serde(default = "default_grain_amount")]
    pub amount: f64,
    /// Grain scale in document pixels, 0.5–20.
    #[serde(default = "default_grain_size")]
    pub size: f64,
    /// 0–100: how much smaller, irregular detail roughens the main particles.
    #[serde(default = "default_grain_roughness")]
    pub roughness: f64,
    #[serde(default)]
    pub seed: u32,
}

fn default_grain_amount() -> f64 {
    25.0
}
fn default_grain_size() -> f64 {
    1.5
}
fn default_grain_roughness() -> f64 {
    50.0
}

impl Default for GrainSettings {
    fn default() -> Self {
        GrainSettings {
            amount: 25.0,
            size: 1.5,
            roughness: 50.0,
            seed: 0,
        }
    }
}

impl GrainSettings {
    pub const AMOUNT_RANGE: (f64, f64) = (0.0, 100.0);
    pub const SIZE_RANGE: (f64, f64) = (0.5, 20.0);
    pub const ROUGHNESS_RANGE: (f64, f64) = (0.0, 100.0);

    pub fn is_valid(&self) -> bool {
        (Self::AMOUNT_RANGE.0..=Self::AMOUNT_RANGE.1).contains(&self.amount)
            && (Self::SIZE_RANGE.0..=Self::SIZE_RANGE.1).contains(&self.size)
            && (Self::ROUGHNESS_RANGE.0..=Self::ROUGHNESS_RANGE.1).contains(&self.roughness)
    }

    pub fn normalized(&self) -> Self {
        GrainSettings {
            amount: clamp_double(
                self.amount,
                Self::AMOUNT_RANGE.0,
                Self::AMOUNT_RANGE.1,
                25.0,
            ),
            size: clamp_double(self.size, Self::SIZE_RANGE.0, Self::SIZE_RANGE.1, 1.5),
            roughness: clamp_double(
                self.roughness,
                Self::ROUGHNESS_RANGE.0,
                Self::ROUGHNESS_RANGE.1,
                50.0,
            ),
            seed: self.seed,
        }
    }

    pub fn apply(
        &self,
        image: &Bitmap,
        origin: crate::geometry::CGPoint,
        units_per_pixel: f64,
        seed: Option<u32>,
    ) -> Bitmap {
        let mut result = image.clone();
        if !self.is_valid() || !units_per_pixel.is_finite() || units_per_pixel <= 0.0 {
            return result;
        }
        if self.amount <= 0.0 {
            return result;
        }
        let pattern = seed.unwrap_or(self.seed);
        adjust_grain(
            &mut result,
            self.amount,
            self.size,
            self.roughness,
            pattern,
            origin.x,
            origin.y,
            units_per_pixel,
        );
        result
    }
}

#[inline]
fn mix32(x: u32) -> u32 {
    let mut x = x;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

/// A value in −1…1 for an integer lattice point, fixed by the point and the seed.
#[inline]
fn lattice(ix: i64, iy: i64, seed: u32) -> f32 {
    let h = mix32(
        (ix as u32).wrapping_mul(0x9E3779B1) ^ mix32((iy as u32).wrapping_mul(0x85EBCA77) ^ seed),
    );
    (h & 0xFFFF) as f32 / 65535.0 + (h >> 16) as f32 / 65535.0 - 1.0
}

/// Smooth seeded noise whose features follow `scale` document pixels.
#[inline]
fn grain_field(u: f64, v: f64, scale: f64, seed: u32) -> f32 {
    let cell_x = (u / scale).floor();
    let cell_y = (v / scale).floor();
    let mut tx = (u / scale - cell_x) as f32;
    let mut ty = (v / scale - cell_y) as f32;
    tx = tx * tx * (3.0 - 2.0 * tx);
    ty = ty * ty * (3.0 - 2.0 * ty);
    let ix = cell_x as i64;
    let iy = cell_y as i64;
    let n00 = lattice(ix, iy, seed);
    let n10 = lattice(ix + 1, iy, seed);
    let n01 = lattice(ix, iy + 1, seed);
    let n11 = lattice(ix + 1, iy + 1, seed);
    let top = n00 + (n10 - n00) * tx;
    let bottom = n01 + (n11 - n01) * tx;
    (top + (bottom - top) * ty) * 1.6
}

#[inline]
fn clamp255(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value > 255.0 {
        255.0
    } else {
        value
    }
}

/// `adjust_grain` (AdjustPixels.c).
pub fn adjust_grain(
    bitmap: &mut Bitmap,
    amount: f64,
    size: f64,
    roughness: f64,
    seed: u32,
    origin_x: f64,
    origin_y: f64,
    units_per_pixel: f64,
) {
    if !(amount > 0.0) || !(units_per_pixel > 0.0) {
        return;
    }
    let size = if size > 0.0 { size } else { 1.0 };
    let strength = (if amount > 100.0 { 1.0 } else { amount / 100.0 }) as f32 * 0.35 * 255.0;
    let rough = (roughness.clamp(0.0, 100.0) / 100.0) as f32;
    let fine_seed = mix32(seed ^ 0xA511E9B3);
    let detail_size = (size * 0.35).max(0.5);
    for y in 0..bitmap.height {
        let v = origin_y + (y as f64 + 0.5) * units_per_pixel;
        for x in 0..bitmap.width {
            let i = (y * bitmap.width + x) * 4;
            let p = &mut bitmap.data[i..i + 4];
            let a = p[3] as u32;
            if a == 0 {
                continue;
            }
            let u = origin_x + (x as f64 + 0.5) * units_per_pixel;
            let smooth = grain_field(u, v, size, seed);
            let fine = grain_field(u, v, detail_size, fine_seed);
            let noise = smooth + (fine - smooth) * rough;
            let unpremultiply = if a == 255 { 1.0 } else { 255.0 / a as f32 };
            let r = p[0] as f32 * unpremultiply;
            let g = p[1] as f32 * unpremultiply;
            let b = p[2] as f32 * unpremultiply;
            let mut level = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
            if level > 1.0 {
                level = 1.0;
            }
            // Film grain shows most in the midtones.
            let delta = noise * strength * (0.4 + 2.4 * level * (1.0 - level));
            let coverage = a as f32 / 255.0;
            p[0] = (clamp255(r + delta) * coverage + 0.5) as u8;
            p[1] = (clamp255(g + delta) * coverage + 0.5) as u8;
            p[2] = (clamp255(b + delta) * coverage + 0.5) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PaletteColor;

    #[test]
    fn exposure_identity_table() {
        let e = ExposureSettings::default();
        let t = e.table();
        for v in [0, 64, 128, 255] {
            assert!((t[v] - v as f32 / 255.0).abs() < 1e-4);
        }
    }

    #[test]
    fn exposure_one_stop_brightens() {
        let e = ExposureSettings {
            exposure: 1.0,
            ..Default::default()
        };
        let t = e.table();
        // 50% sRGB ≈ 0.214 linear → ×2 → 0.428 → ≈ 68% sRGB.
        assert!(t[128] > 0.6 && t[128] < 0.75, "{}", t[128]);
        // White and black stay put (offset 0).
        assert_eq!(t[0], 0.0);
        assert!((t[255] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn exposure_gamma_changes_mids() {
        let e = ExposureSettings {
            gamma: 2.0,
            ..Default::default()
        };
        let t = e.table();
        assert!(t[128] > 0.68 && t[128] < 0.74); // 0.5^(1/2) ≈ 0.707
    }

    #[test]
    fn gradient_map_black_to_white_keeps_tone() {
        let g = GradientMapSettings::default();
        let mut img = Bitmap::new(3, 1);
        img.set_pixel(0, 0, [0, 0, 0, 255]);
        img.set_pixel(1, 0, [128, 128, 128, 255]);
        img.set_pixel(2, 0, [255, 255, 255, 255]);
        let out = g.apply(&img);
        assert_eq!(out.pixel(0, 0)[0], 0);
        // Rec.709 level of 50% grey rounds to 128.
        assert_eq!(out.pixel(1, 0)[0], 128);
        assert_eq!(out.pixel(2, 0)[0], 255);
    }

    #[test]
    fn gradient_map_reversed_flips() {
        let g = GradientMapSettings {
            reversed: true,
            ..Default::default()
        };
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [0, 0, 0, 255]);
        let out = g.apply(&img);
        assert_eq!(out.pixel(0, 0)[0], 255);
    }

    #[test]
    fn gradient_map_tinted_ends() {
        let g = GradientMapSettings {
            shadows: AdjustmentColor::new(1.0, 0.0, 0.0),
            highlights: AdjustmentColor::new(0.0, 0.0, 1.0),
            reversed: false,
        };
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [0, 0, 0, 255]);
        let out = g.apply(&img);
        assert_eq!(out.pixel(0, 0)[0], 255);
        assert_eq!(out.pixel(0, 0)[2], 0);
    }

    #[test]
    fn gradient_map_preserves_alpha() {
        let g = GradientMapSettings::default();
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [255, 255, 255, 128]);
        let out = g.apply(&img);
        assert_eq!(out.pixel(0, 0)[3], 128);
        // Straight white stays white.
        assert_eq!(out.pixel(0, 0)[0], 128);
    }

    #[test]
    fn black_white_defaults_separate_red_green() {
        let bw = BlackWhiteSettings::default();
        let mut img = Bitmap::new(2, 1);
        img.set_pixel(0, 0, [255, 0, 0, 255]); // pure red
        img.set_pixel(1, 0, [0, 255, 0, 255]); // pure green
        let out = bw.apply(&img);
        // Pure red: gray = 0*yellow + 1*reds(40%) = 102. Pure green likewise 102.
        let red_gray = out.pixel(0, 0)[0] as f64;
        let green_gray = out.pixel(1, 0)[0] as f64;
        assert!((red_gray - 102.0).abs() < 2.0, "{red_gray}");
        assert!((green_gray - 102.0).abs() < 2.0, "{green_gray}");
        // Orange separates: gray = 0.502*yellow(60%) + 0.498*reds(40%) ~ 128.
        let mut img2 = Bitmap::new(1, 1);
        img2.set_pixel(0, 0, [255, 128, 0, 255]);
        let orange = bw.apply(&img2).pixel(0, 0)[0] as f64;
        assert!((orange - 128.0).abs() < 2.0, "{orange}");
        // Yellow (255,255,0): mid = green = 1.0 takes the secondary weight
        // (g>=b picks yellow, 0.6) = 153.
        let mut img3 = Bitmap::new(1, 1);
        img3.set_pixel(0, 0, [255, 255, 0, 255]);
        let yellow = bw.apply(&img3).pixel(0, 0)[0] as f64;
        assert!((yellow - 153.0).abs() < 2.0, "{yellow}");
    }

    #[test]
    fn black_white_neutral_stays() {
        let bw = BlackWhiteSettings::default();
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [128, 128, 128, 255]);
        let out = bw.apply(&img);
        assert_eq!(out.pixel(0, 0)[0], 128);
    }

    #[test]
    fn black_white_tint_hues() {
        let bw = BlackWhiteSettings {
            tint: true,
            tint_hue: 40.0,
            tint_saturation: 20.0,
            ..Default::default()
        };
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [128, 128, 128, 255]);
        let out = bw.apply(&img);
        let [r, g, b, _] = out.pixel(0, 0);
        assert!(r > g && g > b, "{r} {g} {b}"); // sepia-ish
        // Luminance approximately preserved.
        let lum = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
        assert!((lum - 128.0).abs() < 15.0, "{lum}");
    }

    #[test]
    fn color_balance_identity_is_noop() {
        let cb = ColorBalanceSettings::default();
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [90, 120, 150, 255]);
        let out = cb.apply(&img);
        assert_eq!(out.pixel(0, 0), [90, 120, 150, 255]);
    }

    #[test]
    fn color_balance_warms_highlights() {
        let cb = ColorBalanceSettings {
            highlight_yellow_blue: -60.0, // towards yellow: red+green, blue down
            preserve_luminosity: true,
            ..Default::default()
        };
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [220, 220, 220, 255]);
        let out = cb.apply(&img);
        let [r, g, b, _] = out.pixel(0, 0);
        assert!(b < r, "{r} {g} {b}");
        let lum_before = 0.299 * 220.0 + 0.587 * 220.0 + 0.114 * 220.0;
        let lum_after = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
        assert!((lum_before - lum_after).abs() < 6.0);
    }

    #[test]
    fn grain_changes_and_seeds() {
        let mut img = Bitmap::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                img.set_pixel(x, y, [128, 128, 128, 255]);
            }
        }
        let g = GrainSettings::default();
        let a = g.apply(&img, crate::geometry::CGPoint::ZERO, 1.0, Some(42));
        assert_ne!(a, img);
        // Deterministic by seed.
        let b = g.apply(&img, crate::geometry::CGPoint::ZERO, 1.0, Some(42));
        assert_eq!(a, b);
        // Different seed differs.
        let c = g.apply(&img, crate::geometry::CGPoint::ZERO, 1.0, Some(7));
        assert_ne!(a, c);
        // Zero amount is a no-op.
        let z = GrainSettings {
            amount: 0.0,
            ..Default::default()
        };
        assert_eq!(
            z.apply(&img, crate::geometry::CGPoint::ZERO, 1.0, None),
            img
        );
    }

    #[test]
    fn grain_respects_document_origin() {
        let mut img = Bitmap::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.set_pixel(x, y, [128, 128, 128, 255]);
            }
        }
        let g = GrainSettings::default();
        let at_origin = g.apply(&img, crate::geometry::CGPoint::new(0.0, 0.0), 1.0, Some(1));
        let shifted = g.apply(
            &img,
            crate::geometry::CGPoint::new(100.0, 0.0),
            1.0,
            Some(1),
        );
        assert_ne!(at_origin, shifted);
    }

    #[test]
    fn adjustment_color_validation() {
        assert!(AdjustmentColor::new(0.0, 0.5, 1.0).is_valid());
        assert!(!AdjustmentColor::new(0.0, 0.5, 1.1).is_valid());
        assert_eq!(
            AdjustmentColor::from_palette(PaletteColor::WHITE),
            AdjustmentColor::new(1.0, 1.0, 1.0)
        );
    }
}
