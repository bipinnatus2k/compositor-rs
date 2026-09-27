//! Port of `Compositor/Document/Filters.swift` and the `Rendering/NoisePixels.c`,
//! `LensPixels.c` kernels: the Filter menu (Gaussian Blur, Motion Blur, Add
//! Noise, Lens Correction), running on a layer inside the selection.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::adjust::camera_raw::CameraRawSettings;
use crate::adjust::pixel as pixel_adjust;
use crate::adjust::{ExposureSettings, GradientMapSettings, GrainSettings};
use crate::adjust::{BlackWhiteSettings, ColorBalanceSettings, CurvesSettings};
use crate::geometry::{CGAffineTransform, CGPoint, CGRect};
use crate::raster::Bitmap;
use crate::selection::SelectionClip;

/// Filters from the Filter menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterKind {
    #[serde(rename = "Gaussian Blur")]
    GaussianBlur,
    #[serde(rename = "Motion Blur")]
    MotionBlur,
    #[serde(rename = "Add Noise")]
    AddNoise,
    #[serde(rename = "Lens Correction")]
    LensCorrection,
    #[serde(rename = "Camera Raw Filter")]
    CameraRaw,
    #[serde(rename = "Remove Background")]
    RemoveBackground,
    #[serde(rename = "Content-Aware Fill")]
    ContentAwareFill,
    #[serde(rename = "Curves")]
    Curves,
    #[serde(rename = "Exposure")]
    Exposure,
    #[serde(rename = "Gradient Map")]
    GradientMap,
    #[serde(rename = "Grain")]
    Grain,
    #[serde(rename = "Black & White")]
    BlackWhite,
    #[serde(rename = "Color Balance")]
    ColorBalance,
}

impl FilterKind {
    pub const ALL: [FilterKind; 13] = [
        FilterKind::GaussianBlur,
        FilterKind::MotionBlur,
        FilterKind::AddNoise,
        FilterKind::LensCorrection,
        FilterKind::CameraRaw,
        FilterKind::RemoveBackground,
        FilterKind::ContentAwareFill,
        FilterKind::Curves,
        FilterKind::Exposure,
        FilterKind::GradientMap,
        FilterKind::Grain,
        FilterKind::BlackWhite,
        FilterKind::ColorBalance,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            FilterKind::GaussianBlur => "Gaussian Blur",
            FilterKind::MotionBlur => "Motion Blur",
            FilterKind::AddNoise => "Add Noise",
            FilterKind::LensCorrection => "Lens Correction",
            FilterKind::CameraRaw => "Camera Raw Filter",
            FilterKind::RemoveBackground => "Remove Background",
            FilterKind::ContentAwareFill => "Content-Aware Fill",
            FilterKind::Curves => "Curves",
            FilterKind::Exposure => "Exposure",
            FilterKind::GradientMap => "Gradient Map",
            FilterKind::Grain => "Grain",
            FilterKind::BlackWhite => "Black & White",
            FilterKind::ColorBalance => "Color Balance",
        }
    }

    pub fn is_automatic(&self) -> bool {
        matches!(self, FilterKind::ContentAwareFill | FilterKind::RemoveBackground)
    }

    /// Color adjustments: in the Image menu (and editable as adjustment layers).
    pub fn is_image_adjustment(&self) -> bool {
        matches!(
            self,
            FilterKind::Curves
                | FilterKind::Exposure
                | FilterKind::GradientMap
                | FilterKind::Grain
                | FilterKind::BlackWhite
                | FilterKind::ColorBalance
        )
    }
}

/// Remove Background's two ways of working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BackgroundQuality {
    #[default]
    #[serde(rename = "Basic")]
    Basic,
    #[serde(rename = "Advanced")]
    Advanced,
}

/// Every filter's settings; each filter reads only its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterSettings {
    /// Gaussian Blur radius in layer pixels (the blur's standard deviation), 0.1–250.
    #[serde(default = "default_radius")]
    pub radius: f64,
    /// Motion Blur direction in degrees, counterclockwise from horizontal, −90–90.
    #[serde(default)]
    pub angle: f64,
    /// Motion Blur streak length in layer pixels, 1–2000.
    #[serde(default = "default_distance")]
    pub distance: f64,
    /// Add Noise strength as Photoshop's percentage, 0.1–400.
    #[serde(default = "default_amount")]
    pub amount: f64,
    /// Add Noise distribution: Gaussian (more speckled) instead of Uniform.
    #[serde(default)]
    pub gaussian: bool,
    /// Add Noise changes brightness only, the same amount on every channel.
    #[serde(default)]
    pub monochromatic: bool,
    /// Lens Correction's Remove Distortion, −100–100.
    #[serde(default)]
    pub distortion: f64,
    #[serde(default)]
    pub curves: CurvesSettings,
    #[serde(default)]
    pub exposure: ExposureSettings,
    #[serde(default)]
    pub gradient_map: GradientMapSettings,
    #[serde(default)]
    pub grain: GrainSettings,
    #[serde(default)]
    pub black_white: BlackWhiteSettings,
    #[serde(default)]
    pub color_balance: ColorBalanceSettings,
    #[serde(default)]
    pub camera_raw: CameraRawSettings,
    /// Remove Background: Basic is the quick subject mask; Advanced refines it.
    #[serde(default)]
    pub background_quality: BackgroundQuality,
    /// Remove Background: how far the mask is pulled onto the image's own edges.
    #[serde(default = "default_refine")]
    pub refine_edges: f64,
    /// Remove Background: pushes the mask's grays toward black and white, 0–100.
    #[serde(default = "default_matte")]
    pub matte_contrast: f64,
    /// Remove Background: contracts (negative) or expands (positive) the edge.
    #[serde(default)]
    pub shift_edge: f64,
}

impl Default for FilterSettings {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

fn default_radius() -> f64 {
    1.0
}
fn default_distance() -> f64 {
    10.0
}
fn default_amount() -> f64 {
    10.0
}
fn default_refine() -> f64 {
    12.0
}
fn default_matte() -> f64 {
    25.0
}

impl FilterSettings {
    pub fn normalized(&self) -> Self {
        fn clamp(value: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
            if value.is_finite() {
                value.min(hi).max(lo)
            } else {
                fallback
            }
        }
        let mut result = self.clone();
        result.radius = clamp(self.radius, 0.1, 250.0, 1.0);
        result.angle = clamp(self.angle, -90.0, 90.0, 0.0);
        result.distance = clamp(self.distance, 1.0, 2000.0, 10.0);
        result.amount = clamp(self.amount, 0.1, 400.0, 10.0);
        result.distortion = clamp(self.distortion, -100.0, 100.0, 0.0);
        result.refine_edges = clamp(self.refine_edges, 0.0, 40.0, 12.0);
        result.matte_contrast = clamp(self.matte_contrast, 0.0, 100.0, 25.0);
        result.shift_edge = clamp(self.shift_edge, -10.0, 10.0, 0.0);
        result.exposure = self.exposure.normalized();
        result.gradient_map = self.gradient_map.normalized();
        result.grain = self.grain.normalized();
        result.camera_raw = self.camera_raw.normalized();
        result
    }
}

/// A filter run request, mirroring `FilterJob`.
pub struct FilterJob<'a> {
    pub kind: FilterKind,
    pub image: &'a Bitmap,
    pub settings: FilterSettings,
    /// Pixels in `image` per original layer pixel.
    pub scale: f64,
    pub selection: Option<SelectionClip>,
    pub mapping: CGAffineTransform,
    /// Add Noise's random pattern.
    pub seed: u32,
    /// Canvas-space origin used by live adjustment layers.
    pub noise_origin: CGPoint,
    pub camera_raw_clipping: Option<crate::adjust::camera_raw::CameraRawClipping>,
    pub visualize_point_color: i32,
    pub sharpen_mask: bool,
}

/// `noise_add_at` (NoisePixels.c).
pub fn noise_add_at(
    bitmap: &mut Bitmap,
    amount: f32,
    gaussian: bool,
    monochromatic: bool,
    seed: u32,
    origin_x: i64,
    origin_y: i64,
) {
    let spread = amount / 100.0 * 127.5;
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let p = &mut bitmap.data[(y * bitmap.width + x) * 4..(y * bitmap.width + x) * 4 + 4];
            let alpha = p[3] as u32;
            if alpha == 0 {
                continue;
            }
            let px = (origin_x + x as i64) as u32;
            let py = (origin_y + y as i64) as u32;
            let base = noise_hash(seed ^ noise_hash(px.wrapping_mul(0x9e3779b9) ^ noise_hash(py.wrapping_mul(0x85ebca6b))));
            for c in 0..3usize {
                let key = if monochromatic { base } else { base.wrapping_add((c as u32).wrapping_mul(0x9e3779b9)) };
                let n = if gaussian {
                    // Box–Muller: two uniform values make one normally distributed one.
                    let u1 = noise_unit(key);
                    let u2 = noise_unit(key ^ 0x68e31da4);
                    (-2.0 * (1.0 - u1).ln()).sqrt() * (6.2831853 * u2).cos() * spread * (2.0 / 3.0)
                } else {
                    (noise_unit(key) * 2.0 - 1.0) * spread
                };
                let value = p[c] as f32 * 255.0 / alpha as f32 + n;
                let value = value.clamp(0.0, 255.0);
                p[c] = (value * alpha as f32 / 255.0).round() as u8;
            }
        }
    }
}

/// A well-mixed 32-bit hash.
#[inline]
fn noise_hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

/// Uniform in [0, 1).
#[inline]
fn noise_unit(key: u32) -> f32 {
    (noise_hash(key) >> 8) as f32 * (1.0 / 16777216.0)
}

/// `lens_distort` (LensPixels.c): radial distortion with bilinear sampling.
pub fn lens_distort(source: &Bitmap, k: f64) -> Bitmap {
    let width = source.width;
    let height = source.height;
    let mut destination = Bitmap::new(width, height);
    let cx = width as f64 * 0.5;
    let cy = height as f64 * 0.5;
    let half_diagonal2 = cx * cx + cy * cy;
    for y in 0..height {
        let dy = y as f64 + 0.5 - cy;
        for x in 0..width {
            let dx = x as f64 + 0.5 - cx;
            let scale = 1.0 - k * (dx * dx + dy * dy) / half_diagonal2;
            // Source position in pixel-center coordinates.
            let sx = cx + dx * scale - 0.5;
            let sy = cy + dy * scale - 0.5;
            let fx0 = sx.floor();
            let fy0 = sy.floor();
            let fx = sx - fx0;
            let fy = sy - fy0;
            let x0 = fx0 as i64;
            let y0 = fy0 as i64;
            let mut sums = [0.0f64; 4];
            for j in 0..2i64 {
                let row = y0 + j;
                if row < 0 || row >= height as i64 {
                    continue;
                }
                let wy = if j != 0 { fy } else { 1.0 - fy };
                if wy == 0.0 {
                    continue;
                }
                for i in 0..2i64 {
                    let column = x0 + i;
                    if column < 0 || column >= width as i64 {
                        continue;
                    }
                    let weight = wy * (if i != 0 { fx } else { 1.0 - fx });
                    if weight == 0.0 {
                        continue;
                    }
                    let p = source.pixel(column as usize, row as usize);
                    for c in 0..4 {
                        sums[c] += weight * p[c] as f64;
                    }
                }
            }
            let out_x = (y * width + x) * 4;
            for c in 0..4 {
                destination.data[out_x + c] = (sums[c] + 0.5) as u8;
            }
        }
    }
    destination
}

/// `CIMotionBlur`'s radius per pixel of streak length: an even streak of
/// length d spreads d / √12.
pub const MOTION_RADIUS_PER_PIXEL: f64 = 0.2886751345948129; // 1 / √12

/// Remove Distortion at ±100 moves the image's corners by this share of their
/// distance from the center.
pub const LENS_STRENGTH: f64 = 0.35;

/// `PixelFilter.run` — applies one filter to `job.image`.
pub fn run_filter(job: &FilterJob) -> Bitmap {
    let settings = job.settings.normalized();
    let image = job.image;
    let mut result: Bitmap = match job.kind {
        FilterKind::Curves => settings.curves.apply(image),
        FilterKind::Exposure => settings.exposure.apply(image),
        FilterKind::GradientMap => settings.gradient_map.apply(image),
        FilterKind::BlackWhite => settings.black_white.apply(image),
        FilterKind::ColorBalance => settings.color_balance.apply(image),
        FilterKind::CameraRaw => settings.camera_raw.apply(
            image,
            job.camera_raw_clipping,
            job.scale,
            job.seed,
            job.visualize_point_color,
            job.sharpen_mask,
        ),
        // Grain sits in layer pixels; the job's seed gives each application its own pattern.
        FilterKind::Grain => settings.grain.apply(image, CGPoint::ZERO, 1.0 / job.scale, Some(job.seed)),
        FilterKind::RemoveBackground => crate::tools::subject_removal::run(image, &settings),
        FilterKind::ContentAwareFill => crate::tools::content_fill::run(image),
        FilterKind::GaussianBlur => {
            // Filters.swift: the blur runs on the whole image and is cropped
            // back to the extent. CI's input is transparent outside its finite
            // extent, so the blur spreads into transparent room and content at
            // the bitmap's own edge softens outwards instead of the border
            // smearing inwards. Grow the bitmap by transparent padding, blur
            // the grown image (the edge clamp in `gaussian_blur_rgba` then only
            // ever lands in the transparent pad), and trim the pad away.
            let sigma = settings.radius * job.scale;
            let pad = (sigma * 3.0).ceil().max(1.0) as usize;
            let grown = grow_to_fit(image, pad);
            let blurred = crate::raster::gaussian_blur_rgba(&grown, sigma);
            crop_bitmap(
                &blurred,
                &CGRect::new(pad as f64, pad as f64, image.width as f64, image.height as f64),
            )
        }
        FilterKind::MotionBlur => motion_blur(image, settings.distance * job.scale, settings.angle),
        FilterKind::AddNoise => {
            let mut noisy = image.clone();
            noise_add_at(
                &mut noisy,
                settings.amount as f32,
                settings.gaussian,
                settings.monochromatic,
                job.seed,
                job.noise_origin.x.floor() as i64,
                job.noise_origin.y.floor() as i64,
            );
            noisy
        }
        FilterKind::LensCorrection => lens_distort(image, settings.distortion / 100.0 * LENS_STRENGTH),
    };
    if let Some(selection) = &job.selection {
        result = pixel_adjust::blend(&result, image, selection, &job.mapping);
    }
    result
}

/// `CIMotionBlur`-equivalent: a Gaussian-tapered streak along `angle` degrees
/// counterclockwise from horizontal (Core Image's kernel is not an even box;
/// the doc previously claimed "even streak" — it tapers like CI at
/// `MOTION_RADIUS_PER_PIXEL` × length). Edge samples clamp like Core Image's
/// default sampler, so border pixels keep their energy.
pub fn motion_blur(image: &Bitmap, distance: f64, angle_degrees: f64) -> Bitmap {
    if distance < 1.0 {
        return image.clone();
    }
    let radius = (distance * MOTION_RADIUS_PER_PIXEL).ceil() as i64;
    if radius < 1 {
        return image.clone();
    }
    // Core Image's y axis points up, so its counterclockwise angle matches
    // Photoshop's: in our y-down pixels the direction vector is (cos, -sin).
    let radians = angle_degrees * std::f64::consts::PI / 180.0;
    let dir_x = radians.cos();
    let dir_y = -radians.sin();
    // CIMotionBlur tapers like a Gaussian whose spread is about its radius:
    // weight each tap by exp(-t^2 / 2sigma^2) with sigma = radius, normalized.
    let sigma = radius as f64;
    let mut weights = vec![0.0f64; (radius * 2 + 1) as usize];
    let mut total = 0.0f64;
    for i in -radius..=radius {
        let w = (-(i as f64) * (i as f64) / (2.0 * sigma * sigma)).exp();
        weights[(i + radius) as usize] = w;
        total += w;
    }
    for w in weights.iter_mut() {
        *w /= total;
    }
    let mut acc = vec![0.0f32; image.width * image.height * 4];
    for i in -radius..=radius {
        let dx = dir_x * i as f64;
        let dy = dir_y * i as f64;
        let weight = weights[(i + radius) as usize] as f32;
        if weight == 0.0 {
            continue;
        }
        for y in 0..image.height {
            for x in 0..image.width {
                // Clamp to the edge (Core Image's default sampler) instead of
                // dropping the tap — dropped taps darkened the borders.
                let sx = (x as f64 + dx).clamp(0.0, (image.width - 1) as f64);
                let sy = (y as f64 + dy).clamp(0.0, (image.height - 1) as f64);
                let p = crate::raster::bilinear_sample(image, sx, sy);
                let o = (y * image.width + x) * 4;
                for c in 0..4 {
                    acc[o + c] += p[c] as f32 * weight;
                }
            }
        }
    }
    let mut out = image.clone();
    for (o, v) in out.data.iter_mut().zip(acc.iter()) {
        *o = (*v + 0.5) as u8;
    }
    out
}

/// `PixelFilter.trimmed`: `image` cropped to the pixels that are actually
/// there, with the transform that keeps them in place.
pub fn trimmed(image: &Bitmap, placed: crate::transform::LayerTransform) -> (Bitmap, crate::transform::LayerTransform) {
    let full = image.bounds();
    let crop = match image.alpha_bounds() {
        Some(bounds) => bounds,
        None => return (image.clone(), placed),
    };
    if crop.width() < 1.0 || crop.height() < 1.0 || crop == full {
        return (image.clone(), placed);
    }
    let cropped = crop_bitmap(image, &crop);
    let mut result = placed;
    result.size = crate::geometry::CGSize::new(
        crop.width() * placed.size.width / full.width(),
        crop.height() * placed.size.height / full.height(),
    );
    let to_document = crate::raster::pixel_to_document(&placed, full.width(), full.height());
    let middle = CGPoint::new(crop.mid_x(), crop.mid_y()).applying(&to_document);
    result.origin = CGPoint::new(middle.x - result.size.width / 2.0, middle.y - result.size.height / 2.0);
    (cropped, result)
}

/// Crops a bitmap to an integer rect.
pub fn crop_bitmap(image: &Bitmap, rect: &CGRect) -> Bitmap {
    let r = rect.integral().intersection(&image.bounds());
    if r.is_null() || r.is_empty() {
        return Bitmap::new(1, 1);
    }
    let x0 = r.min_x() as usize;
    let y0 = r.min_y() as usize;
    let w = r.width() as usize;
    let h = r.height() as usize;
    let mut out = Bitmap::new(w, h);
    for y in 0..h {
        let src = ((y0 + y) * image.width + x0) * 4;
        let dst = y * w * 4;
        out.data[dst..dst + w * 4].copy_from_slice(&image.data[src..src + w * 4]);
    }
    out
}

/// Expands `image` by `pad` transparent pixels on every side, with the
/// original pixels landing at (pad, pad): the room a Core Image blur spreads
/// into (`CIImage(cgImage:)` is transparent outside its extent).
fn grow_to_fit(image: &Bitmap, pad: usize) -> Bitmap {
    let mut out = Bitmap::new(image.width + pad * 2, image.height + pad * 2);
    for y in 0..image.height {
        let src = y * image.width * 4;
        let dst = ((pad + y) * out.width + pad) * 4;
        out.data[dst..dst + image.width * 4]
            .copy_from_slice(&image.data[src..src + image.width * 4]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::LayerSampling;

    #[test]
    fn settings_normalize_defaults() {
        let s = FilterSettings::default();
        assert_eq!(s.radius, 1.0);
        assert_eq!(s.distance, 10.0);
        assert_eq!(s.amount, 10.0);
        let n = s.normalized();
        assert_eq!(n.radius, 1.0);
    }

    #[test]
    fn settings_normalize_clamps() {
        let mut s = FilterSettings::default();
        s.radius = 1000.0;
        s.distance = 0.0;
        s.amount = -5.0;
        s.refine_edges = 100.0;
        let n = s.normalized();
        assert_eq!(n.radius, 250.0);
        assert_eq!(n.distance, 1.0);
        assert_eq!(n.amount, 0.1);
        assert_eq!(n.refine_edges, 40.0);
    }

    #[test]
    fn noise_deterministic_and_changes_pixels() {
        let mut img = Bitmap::new(4, 4);
        for i in (0..img.data.len()).step_by(4) {
            img.data[i..i + 4].copy_from_slice(&[128, 128, 128, 255]);
        }
        let a = {
            let mut c = img.clone();
            noise_add_at(&mut c, 20.0, false, false, 42, 0, 0);
            c
        };
        let b = {
            let mut c = img.clone();
            noise_add_at(&mut c, 20.0, false, false, 42, 0, 0);
            c
        };
        assert_eq!(a, b);
        assert_ne!(a, img);
        // Monochromatic shifts all channels of a pixel equally.
        let m = {
            let mut c = img.clone();
            noise_add_at(&mut c, 20.0, false, true, 42, 0, 0);
            c
        };
        let p = m.pixel(1, 1);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
    }

    #[test]
    fn noise_origin_shuffles_pattern() {
        let mut img = Bitmap::new(4, 4);
        for i in (0..img.data.len()).step_by(4) {
            img.data[i..i + 4].copy_from_slice(&[128, 128, 128, 255]);
        }
        let mut a = img.clone();
        noise_add_at(&mut a, 50.0, false, true, 7, 0, 0);
        let mut b = img.clone();
        noise_add_at(&mut b, 50.0, false, true, 7, 100, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn lens_distort_identity_at_zero() {
        let mut img = Bitmap::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.set_pixel(x, y, [(x * 40 + y) as u8, 0, 0, 255]);
            }
        }
        let out = lens_distort(&img, 0.0);
        assert_eq!(out, img);
    }

    #[test]
    fn lens_distort_moves_corners() {
        // Positive k straightens barrel distortion: corners pull inward, so
        // the corner pixel samples toward the center.
        let mut img = Bitmap::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                img.set_pixel(x, y, [x as u8, 0, 0, 255]);
            }
        }
        let out = lens_distort(&img, 0.35);
        // Corner (0,0) now samples a position moved toward the center (x>0).
        assert!(out.pixel(0, 0)[0] > 0);
    }

    #[test]
    fn gaussian_blur_spreads_and_keeps_size() {
        let mut img = Bitmap::new(21, 1);
        img.set_pixel(10, 0, [255, 255, 255, 255]);
        let out = crate::raster::gaussian_blur_rgba(&img, 1.5);
        assert_eq!(out.width, 21);
        // A delta's peak is the kernel's peak: 255 / (sigma*sqrt(2*pi)) ~ 68.
        let peak = out.pixel(10, 0)[0];
        assert!(peak > 50 && peak < 120, "{peak}");
        assert!(out.pixel(8, 0)[0] > 0 && out.pixel(8, 0)[0] < peak);
        assert_eq!(out.pixel(8, 0)[0], out.pixel(12, 0)[0]);
    }

    /// A Gaussian Blur job at scale 1 with no selection.
    fn gaussian_job(image: &Bitmap, radius: f64) -> FilterJob<'_> {
        FilterJob {
            kind: FilterKind::GaussianBlur,
            image,
            settings: FilterSettings {
                radius,
                ..Default::default()
            },
            scale: 1.0,
            selection: None,
            mapping: CGAffineTransform::IDENTITY,
            seed: 0,
            noise_origin: CGPoint::ZERO,
            camera_raw_clipping: None,
            visualize_point_color: -1,
            sharpen_mask: false,
        }
    }

    #[test]
    fn gaussian_spreads_into_transparent_room() {
        // CI semantics: the blur draws from the transparent room around the
        // block, so the block softens and bleeds outward without a border
        // being smeared through it.
        let mut img = Bitmap::new(100, 100);
        for y in 40..60 {
            for x in 40..60 {
                img.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let out = run_filter(&gaussian_job(&img, 6.0));
        assert_eq!((out.width, out.height), (100, 100), "trim keeps the input size");
        // The block's core stays strongly opaque.
        assert!(out.pixel(50, 50)[3] > 180, "core: {:?}", out.pixel(50, 50));
        // Just outside the block the blur has spread into the transparent room.
        assert!(out.pixel(63, 50)[3] > 0, "outward spread: {:?}", out.pixel(63, 50));
        // The far corner stays empty.
        assert_eq!(out.pixel(2, 2)[3], 0);
    }

    #[test]
    fn gaussian_edge_content_spreads_outward() {
        // A fully opaque 50x50 image with 10 transparent rows appended: the
        // bitmap's top edge is content. The blur decays it into the transparent
        // room instead of the edge clamp holding the border at full alpha.
        let mut img = Bitmap::new(50, 60);
        for y in 0..50 {
            for x in 0..50 {
                img.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let out = run_filter(&gaussian_job(&img, 0.5));
        let edge = out.pixel(25, 0)[3];
        assert!(edge > 0 && edge < 255, "top edge decays outward: {edge}");
        // Deeper in, the content stays fully solid.
        assert_eq!(out.pixel(25, 12)[3], 255);
    }

    #[test]
    fn motion_blur_taper_decays_with_distance() {
        // A single bright dot: the core stays bright, the trail decays
        // monotonically and symmetrically (CIMotionBlur's tapered kernel).
        let mut img = Bitmap::new(60, 60);
        for y in 28..33 {
            for x in 28..33 {
                img.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let out = motion_blur(&img, 20.0, 0.0);
        let core = out.pixel(30, 30)[0];
        let near = out.pixel(38, 30)[0];
        let mid = out.pixel(44, 30)[0];
        let far = out.pixel(50, 30)[0];
        // The Gaussian core keeps only ~1/3 of a 5px dot's energy at
        // sigma = radius = 6 — the taper is the point.
        assert!(core > near && near > mid && mid >= far,
            "taper decays: {core}/{near}/{mid}/{far}");
        assert!(far <= 20, "far tail is nearly clear: {far}");
        assert!(
            i64::from(near) - i64::from(out.pixel(22, 30)[0]) <= 8,
            "symmetric about the source: {:?} vs {:?}",
            near,
            out.pixel(22, 30)
        );
    }

    #[test]
    fn motion_blur_streaks_along_angle() {
        let mut img = Bitmap::new(21, 21);
        img.set_pixel(10, 10, [255, 0, 0, 255]);
        let horizontal = motion_blur(&img, 12.0, 0.0);
        // A horizontal streak leaves (13,10) painted, (10,13) untouched.
        assert!(horizontal.pixel(13, 10)[3] > 0);
        assert_eq!(horizontal.pixel(10, 13)[3], 0);
        let vertical = motion_blur(&img, 12.0, 90.0);
        assert!(vertical.pixel(10, 13)[3] > 0);
        assert_eq!(vertical.pixel(13, 10)[3], 0);
    }

    #[test]
    fn motion_blur_keeps_border_energy() {
        // A fully opaque image stays fully opaque after blurring: edge taps
        // clamp (Core Image's sampler) instead of being dropped, which used
        // to fade the borders toward transparent black.
        let mut img = Bitmap::new(40, 40);
        for pixel in img.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[200, 100, 50, 255]);
        }
        let out = motion_blur(&img, 30.0, 35.0);
        for (i, o) in [0, 39, 40 * 40 - 1, 40 * 20].iter().enumerate() {
            let p = out.pixel(o % 40, o / 40);
            assert!(
                (p[0] as i32 - 200).abs() <= 2
                    && (p[1] as i32 - 100).abs() <= 2
                    && (p[2] as i32 - 50).abs() <= 2
                    && p[3] == 255,
                "border pixel {i} kept its energy: {p:?}"
            );
        }
    }

    #[test]
    fn trimmed_crops_to_content() {
        let mut img = Bitmap::new(10, 10);
        img.set_pixel(3, 4, [255, 255, 255, 255]);
        img.set_pixel(6, 7, [255, 255, 255, 255]);
        let placed = crate::transform::LayerTransform::new(CGPoint::new(10.0, 10.0), crate::geometry::CGSize::new(10.0, 10.0));
        let (cropped, transform) = trimmed(&img, placed);
        assert_eq!(cropped.width, 4);
        assert_eq!(cropped.height, 4);
        assert_eq!(transform.origin, CGPoint::new(13.0, 14.0));
    }

    #[test]
    fn crop_bitmap_takes_rect() {
        let mut img = Bitmap::new(4, 4);
        img.set_pixel(2, 2, [9, 9, 9, 255]);
        let c = crop_bitmap(&img, &CGRect::new(1.0, 1.0, 3.0, 2.0));
        assert_eq!((c.width, c.height), (3, 2));
        assert_eq!(c.pixel(1, 1)[0], 9);
    }

    #[test]
    fn resample_alias_used() {
        let _ = LayerSampling::High;
    }
}
