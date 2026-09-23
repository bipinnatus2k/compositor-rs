//! Raster primitives: RGBA8 bitmaps and 8-bit grayscale masks, with the drawing
//! operations the Swift code gets from Core Graphics (`BrushRaster`, CGContext).
//!
//! Bitmaps are premultiplied-alpha sRGB, exactly as `CGContext` with
//! `premultipliedLast` byte order stores them. Masks are plain grayscale
//! (white reveals) with no alpha, top-left origin.

use crate::geometry::{CGFloat, CGAffineTransform, CGRect};
pub use crate::transform::LayerSampling;

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
/// An RGBA8 premultiplied sRGB bitmap, top-left origin.
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    /// `width * height * 4` bytes, R G B A premultiplied.
    pub data: Vec<u8>,
}

impl Bitmap {
    pub fn new(width: usize, height: usize) -> Self {
        Bitmap {
            width,
            height,
            data: vec![0; width * height * 4],
        }
    }

    pub fn new_clearing(width: usize, height: usize, rgba: [u8; 4]) -> Self {
        // Premultiply for storage.
        let a = rgba[3] as u32;
        let px = [
            ((rgba[0] as u32 * a + 127) / 255) as u8,
            ((rgba[1] as u32 * a + 127) / 255) as u8,
            ((rgba[2] as u32 * a + 127) / 255) as u8,
            rgba[3],
        ];
        let mut data = vec![0u8; width * height * 4];
        for chunk in data.chunks_exact_mut(4) {
            chunk.copy_from_slice(&px);
        }
        Bitmap {
            width,
            height,
            data,
        }
    }

    pub fn from_raw(width: usize, height: usize, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), width * height * 4);
        Bitmap {
            width,
            height,
            data,
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, px: [u8; 4]) {
        let i = (y * self.width + x) * 4;
        self.data[i..i + 4].copy_from_slice(&px);
    }

    pub fn bytes(&self) -> usize {
        self.width * self.height * 4
    }

    pub fn bounds(&self) -> CGRect {
        CGRect::new(0.0, 0.0, self.width as CGFloat, self.height as CGFloat)
    }

    /// A premultiplied RGBA pixel as straight (unpremultiplied) floats 0–1.
    pub fn straight_f32(&self, x: usize, y: usize) -> [f32; 4] {
        let px = self.pixel(x, y);
        unpremultiply_f32(px)
    }

    /// Writes a straight-alpha color, premultiplying on the way in.
    pub fn set_straight_f32(&mut self, x: usize, y: usize, c: [f32; 4]) {
        self.set_pixel(x, y, premultiply_f32(c));
    }

    /// `CGContext.clear(rect)`.
    pub fn clear_rect(&mut self, rect: CGRect) {
        let r = rect.integral().intersection(&self.bounds());
        if r.is_null() || r.is_empty() {
            return;
        }
        for y in r.min_y() as usize..r.max_y() as usize {
            for x in r.min_x() as usize..r.max_x() as usize {
                self.set_pixel(x, y, [0, 0, 0, 0]);
            }
        }
    }

    /// Tight bounds of nonzero alpha, in integer pixels: port of `brush_alpha_bounds`
    /// (BrushPixels.c). Returns None when every pixel is transparent.
    pub fn alpha_bounds(&self) -> Option<CGRect> {
        let mut min_x = self.width as isize;
        let mut min_y = self.height as isize;
        let mut max_x = -1isize;
        let mut max_y = -1isize;
        for y in 0..self.height {
            let row = &self.data[y * self.width * 4..];
            for x in 0..self.width {
                if row[x * 4 + 3] != 0 {
                    if (x as isize) < min_x {
                        min_x = x as isize;
                    }
                    if x as isize > max_x {
                        max_x = x as isize;
                    }
                    if (y as isize) < min_y {
                        min_y = y as isize;
                    }
                    max_y = y as isize;
                }
            }
        }
        if max_x <= min_x || max_y <= min_y {
            return None;
        }
        Some(CGRect::new(
            min_x as CGFloat,
            min_y as CGFloat,
            (max_x - min_x + 1) as CGFloat,
            (max_y - min_y + 1) as CGFloat,
        ))
    }
}

/// Straight-color pixels from a premultiplied RGBA8 quad.
#[inline]
pub fn unpremultiply_f32(px: [u8; 4]) -> [f32; 4] {
    let a = px[3] as f32 / 255.0;
    if a <= 0.0 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    [
        (px[0] as f32 / 255.0) / a,
        (px[1] as f32 / 255.0) / a,
        (px[2] as f32 / 255.0) / a,
        a,
    ]
}

/// Premultiplied RGBA8 from a straight color.
#[inline]
pub fn premultiply_f32(c: [f32; 4]) -> [u8; 4] {
    let a = c[3].clamp(0.0, 1.0);
    let m = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
    [m(c[0]), m(c[1]), m(c[2]), (a * 255.0 + 0.5) as u8]
}

/// An 8-bit grayscale mask, top-left origin, no alpha. White reveals.
#[derive(Clone, PartialEq)]
pub struct GrayMask {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl GrayMask {
    pub fn new(width: usize, height: usize) -> Self {
        GrayMask {
            width,
            height,
            data: vec![0; width * height],
        }
    }

    /// A uniform mask (`LayerMask.solid(revealing:)`).
    pub fn solid(revealing: bool) -> Self {
        GrayMask {
            width: 1,
            height: 1,
            data: vec![if revealing { 255 } else { 0 }],
        }
    }

    pub fn from_raw(width: usize, height: usize, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), width * height);
        GrayMask {
            width,
            height,
            data,
        }
    }

    pub fn value(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.width + x]
    }

    pub fn bounds(&self) -> CGRect {
        CGRect::new(0.0, 0.0, self.width as CGFloat, self.height as CGFloat)
    }
}

/// Port of `ImportedImage`: the source pixels, a small thumbnail, and the
/// display name. Images are immutable and shared; snapshots copy the `Arc`s.
#[derive(Clone, PartialEq)]
pub struct ImportedImage {
    pub image: Arc<Bitmap>,
    pub thumbnail: Arc<Bitmap>,
    pub name: String,
}

impl ImportedImage {
    pub fn new(image: Bitmap, name: impl Into<String>) -> Self {
        let thumbnail = thumbnail_of(&image);
        ImportedImage {
            image: Arc::new(image),
            thumbnail: Arc::new(thumbnail),
            name: name.into(),
        }
    }

    pub fn with_thumbnail(image: Bitmap, thumbnail: Bitmap, name: impl Into<String>) -> Self {
        ImportedImage {
            image: Arc::new(image),
            thumbnail: Arc::new(thumbnail),
            name: name.into(),
        }
    }

    pub fn width(&self) -> usize {
        self.image.width
    }

    pub fn height(&self) -> usize {
        self.image.height
    }
}

/// Thumbnail capped at 96 px on the long side (`min(1, 96 / max(w, h))` factor).
pub fn thumbnail_of(image: &Bitmap) -> Bitmap {
    let factor = (96.0 / image.width.max(image.height) as f64).min(1.0);
    let w = ((image.width as f64 * factor) as usize).max(1);
    let h = ((image.height as f64 * factor) as usize).max(1);
    resample(image, w, h, LayerSampling::High)
}

/// `BrushRaster.pixelToDocument` — the affine placing a `width` × `height` pixel
/// grid where `transform` puts it on the document.
pub fn pixel_to_document(transform: &crate::transform::LayerTransform, width: CGFloat, height: CGFloat) -> CGAffineTransform {
    CGAffineTransform::translation(transform.center().x, transform.center().y)
        .rotated_by(transform.radians())
        .scale_by(
            transform.size.width / width * if transform.flip_x { -1.0 } else { 1.0 },
            transform.size.height / height * if transform.flip_y { -1.0 } else { 1.0 },
        )
        .translate(-width / 2.0, -height / 2.0)
}

/// `BrushRaster.falloff(_:)` — soft-brush falloff across the region between the
/// hardness radius and the rim: a normalized Gaussian fading across the whole
/// radius that reaches zero at the rim.
pub fn falloff(u: CGFloat) -> CGFloat {
    let k: CGFloat = 2.5;
    let a = (-k * u * u).exp();
    let b = (-k).exp();
    ((a - b) / (1.0 - b)).max(0.0)
}

/// `BrushStroke.spacingFraction(_:)` — soft-tip deposition rate.
pub fn spacing_fraction(hardness: CGFloat) -> CGFloat {
    if hardless_or_hard(hardness) {
        0.015
    } else {
        0.025
    }
}

fn hardless_or_hard(hardness: CGFloat) -> bool {
    hardness >= 1.0
}

/// Samples `src` at a source-space point with the given quality.
/// Coordinates are relative to the source bitmap's top-left origin.
pub fn sample_bitmap(src: &Bitmap, x: f64, y: f64, sampling: LayerSampling) -> [u8; 4] {
    match sampling {
        LayerSampling::Nearest => {
            let xi = clamp_i(x.floor() as isize, 0, src.width as isize - 1);
            let yi = clamp_i(y.floor() as isize, 0, src.height as isize - 1);
            src.pixel(xi as usize, yi as usize)
        }
        LayerSampling::Smooth | LayerSampling::High => {
            // Bilinear; "High" additionally pre-shrinks when drawn far below 1:1
            // (draw callers handle that via `mip_level`), so both sample the same here.
            bilinear_sample(src, x, y)
        }
    }
}

fn clamp_i(v: isize, lo: isize, hi: isize) -> isize {
    v.max(lo).min(hi)
}

pub fn bilinear_sample(src: &Bitmap, x: f64, y: f64) -> [u8; 4] {
    if src.width == 0 || src.height == 0 {
        return [0, 0, 0, 0];
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let ix = x0 as isize;
    let iy = y0 as isize;
    let get = |xi: isize, yi: isize| -> [f32; 4] {
        let xi = clamp_i(xi, 0, src.width as isize - 1) as usize;
        let yi = clamp_i(yi, 0, src.height as isize - 1) as usize;
        let p = src.pixel(xi, yi);
        [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
    };
    let p00 = get(ix, iy);
    let p10 = get(ix + 1, iy);
    let p01 = get(ix, iy + 1);
    let p11 = get(ix + 1, iy + 1);
    let fx = fx as f32;
    let fy = fy as f32;
    let mut out = [0u8; 4];
    for i in 0..4 {
        let top = p00[i] * (1.0 - fx) + p10[i] * fx;
        let bottom = p01[i] * (1.0 - fx) + p11[i] * fx;
        out[i] = (top * (1.0 - fy) + bottom * fy + 0.5) as u8;
    }
    out
}

/// Resamples a bitmap to `w` × `h`. `High` uses successive box halving before
/// bilinear when shrinking (the port of `DownsampleCache` sharp halving +
/// CG high interpolation); `Smooth` is plain bilinear; `Nearest` point-samples.
pub fn resample(src: &Bitmap, w: usize, h: usize, sampling: LayerSampling) -> Bitmap {
    let mut out = Bitmap::new(w.max(1), h.max(1));
    if w == 0 || h == 0 || src.width == 0 || src.height == 0 {
        return out;
    }
    match sampling {
        LayerSampling::Nearest => {
            for y in 0..out.height {
                let sy = ((y as f64 + 0.5) * src.height as f64 / h as f64) as isize;
                for x in 0..out.width {
                    let sx = ((x as f64 + 0.5) * src.width as f64 / w as f64) as isize;
                    out.set_pixel(x, y, src.pixel(sx as usize, sy as usize));
                }
            }
        }
        LayerSampling::Smooth | LayerSampling::High => {
            let mut shrunk: Bitmap;
            let mut steps = 0usize;
            if sampling == LayerSampling::High {
                // Halve while more than 2× the target on an axis.
                let mut cur = src.clone();
                while (cur.width / 2).max(1) >= w.max(2) && (cur.height / 2).max(1) >= h.max(2)
                    && cur.width > 2
                    && cur.height > 2
                {
                    cur = halve(&cur);
                    steps += 1;
                    if steps > 20 {
                        break;
                    }
                }
                shrunk = cur;
            } else {
                shrunk = src.clone();
            }
            let sw = shrunk.width as f64;
            let sh = shrunk.height as f64;
            for y in 0..out.height {
                let sy = (y as f64 + 0.5) * sh / h as f64 - 0.5;
                for x in 0..out.width {
                    let sx = (x as f64 + 0.5) * sw / w as f64 - 0.5;
                    out.set_pixel(x, y, bilinear_sample(&shrunk, sx, sy));
                }
            }
        }
    }
    out
}

/// One box-filter halving step (`DownsampleCache`'s sharp halving).
pub fn halve(src: &Bitmap) -> Bitmap {
    let w = (src.width / 2).max(1);
    let h = (src.height / 2).max(1);
    let mut out = Bitmap::new(w, h);
    for y in 0..h {
        let y0 = (y * 2).min(src.height - 1);
        let y1 = (y * 2 + 1).min(src.height - 1);
        for x in 0..w {
            let x0 = (x * 2).min(src.width - 1);
            let x1 = (x * 2 + 1).min(src.width - 1);
            let p00 = src.pixel(x0, y0);
            let p10 = src.pixel(x1, y0);
            let p01 = src.pixel(x0, y1);
            let p11 = src.pixel(x1, y1);
            let mut px = [0u8; 4];
            for i in 0..4 {
                px[i] = ((p00[i] as u32 + p10[i] as u32 + p01[i] as u32 + p11[i] as u32 + 2) / 4) as u8;
            }
            out.set_pixel(x, y, px);
        }
    }
    out
}

/// Draws `src` stretched over `rect` in `dst`'s coordinate space with nearest
/// sampling (`BrushRaster.draw` with `interpolationQuality = .none`).
pub fn draw_stretched_nearest(dst: &mut Bitmap, src: &Bitmap, rect: CGRect) {
    if rect.is_null() || rect.is_empty() || src.width == 0 || src.height == 0 {
        return;
    }
    let clip = rect.integral().intersection(&dst.bounds());
    if clip.is_null() || clip.is_empty() {
        return;
    }
    for y in clip.min_y() as usize..clip.max_y() as usize {
        let v = (y as f64 - rect.min_y()) / rect.height();
        for x in clip.min_x() as usize..clip.max_x() as usize {
            let u = (x as f64 - rect.min_x()) / rect.width();
            let sx = (u * src.width as f64) as isize;
            let sy = (v * src.height as f64) as isize;
            if sx < 0 || sy < 0 || sx >= src.width as isize || sy >= src.height as isize {
                continue;
            }
            let px = src.pixel(sx as usize, sy as usize);
            // Blend mode .copy in BrushRaster.draw for color: replace.
            dst.set_pixel(x, y, px);
        }
    }
}

/// Composite `src` onto `dst` with source-over, stretched over `rect` with the
/// given sampling (the general `context.draw(image, in: rect)`).
pub fn draw_over(dst: &mut Bitmap, src: &Bitmap, rect: CGRect, sampling: LayerSampling) {
    if rect.is_null() || rect.is_empty() || src.width == 0 || src.height == 0 {
        return;
    }
    let clip = rect.integral().intersection(&dst.bounds());
    if clip.is_null() || clip.is_empty() {
        return;
    }
    for y in clip.min_y() as usize..clip.max_y() as usize {
        let v = (y as f64 + 0.5 - rect.min_y()) / rect.height();
        for x in clip.min_x() as usize..clip.max_x() as usize {
            let u = (x as f64 + 0.5 - rect.min_x()) / rect.width();
            let sx = u * src.width as f64 - 0.5;
            let sy = v * src.height as f64 - 0.5;
            let s = sample_bitmap(src, sx, sy, sampling);
            let d = dst.pixel(x, y);
            let out = source_over_u8(d, s);
            dst.set_pixel(x, y, out);
        }
    }
}

/// Premultiplied source-over on u8 quads.
#[inline]
pub fn source_over_u8(d: [u8; 4], s: [u8; 4]) -> [u8; 4] {
    let sa = s[3] as u32;
    if sa == 255 {
        return s;
    }
    let da = (d[3] as u32 * (255 - sa) + 127) / 255;
    let mix = |dc: u32, sc: u32| -> u8 {
        ((sc * sa + dc * (255 - sa) + 127) / 255).min(255) as u8
    };
    [
        mix(d[0] as u32, s[0] as u32),
        mix(d[1] as u32, s[1] as u32),
        mix(d[2] as u32, s[2] as u32),
        (sa + da).min(255) as u8,
    ]
}

/// Draws `src` onto `dst` through an affine transform mapping source pixels to
/// destination space (inverse-mapped sampling), i.e. `context.concatenate(t);
/// context.draw(image, …)`. `clip` restricts where it draws.
pub fn draw_affine(
    dst: &mut Bitmap,
    src: &Bitmap,
    transform: &CGAffineTransform,
    sampling: LayerSampling,
    clip: Option<CGRect>,
) {
    let inverse = transform.inverted();
    let clip_rect = clip.unwrap_or(dst.bounds()).intersection(&dst.bounds());
    if clip_rect.is_null() || clip_rect.is_empty() {
        return;
    }
    // High-quality downscale: pre-shrink the source by the transform's scale.
    let scale_x = transform.a.hypot(transform.b);
    let scale_y = transform.c.hypot(transform.d);
    let mut source = src.clone();
    let mut t = *transform;
    if sampling == LayerSampling::High && (scale_x < 0.5 || scale_y < 0.5) {
        let mut factor = 1.0f64;
        while scale_x * factor <= 0.5 || scale_y * factor <= 0.5 {
            let next = Bitmap {
                width: source.width / 2,
                height: source.height / 2,
                data: Vec::new(),
            };
            if next.width < 1 || next.height < 1 {
                break;
            }
            source = halve(&source);
            factor *= 2.0;
        }
        // Pre-shrink applies to the source before the placement: p' = t * shrink.
        t = t.concat(&CGAffineTransform::scale(factor, factor));
    }
    let inverse = t.inverted();
    // Integer-aligned 1:1 placement draws without resampling, as Core
    // Graphics' fast path does (no half-pixel edge bleed).
    let exact = t.b.abs() < 1e-9
        && t.c.abs() < 1e-9
        && (t.a - 1.0).abs() < 1e-9
        && (t.d - 1.0).abs() < 1e-9
        && t.tx.fract() == 0.0
        && t.ty.fract() == 0.0;
    for y in clip_rect.min_y() as usize..clip_rect.max_y() as usize {
        for x in clip_rect.min_x() as usize..clip_rect.max_x() as usize {
            let p = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&inverse);
            if p.x < 0.0 || p.y < 0.0 || p.x >= source.width as f64 || p.y >= source.height as f64 {
                continue;
            }
            let s = if exact {
                source.pixel(p.x as usize, p.y as usize)
            } else {
                sample_bitmap(&source, p.x, p.y, sampling)
            };
            let d = dst.pixel(x, y);
            dst.set_pixel(x, y, source_over_u8(d, s));
        }
    }
    use crate::geometry::CGPoint;
}

/// Separable Gaussian blur on RGBA (alpha included, color channels blurred
/// independently — premultiplied data so this stays sane). `sigma` in pixels.
/// Like Core Image's `applyingGaussianBlur`, the result keeps the same size
/// (input clamped at the edges).
pub fn gaussian_blur_rgba(src: &Bitmap, sigma: f64) -> Bitmap {
    if sigma <= 0.0 {
        return src.clone();
    }
    let radius = (sigma * 4.0).ceil() as usize;
    let kernel = gaussian_kernel(sigma, radius);
    let mut tmp = Bitmap::new(src.width, src.height);
    let mut dst = Bitmap::new(src.width, src.height);
    // Horizontal pass into tmp.
    for y in 0..src.height {
        for x in 0..src.width {
            let mut acc = [0.0f32; 4];
            for (i, k) in kernel.iter().enumerate() {
                let sx = clamp_mirror_index(x as isize + i as isize - radius as isize, src.width as isize);
                let px = src.pixel(sx as usize, y);
                for c in 0..4 {
                    acc[c] += px[c] as f32 * *k as f32;
                }
            }
            let out: [u8; 4] = acc.map(|v| (v + 0.5) as u8);
            tmp.set_pixel(x, y, out);
        }
    }
    // Vertical pass into dst.
    for y in 0..src.height {
        for x in 0..src.width {
            let mut acc = [0.0f32; 4];
            for (i, k) in kernel.iter().enumerate() {
                let sy = clamp_mirror_index(y as isize + i as isize - radius as isize, src.height as isize);
                let px = tmp.pixel(x, sy as usize);
                for c in 0..4 {
                    acc[c] += px[c] as f32 * *k as f32;
                }
            }
            let out: [u8; 4] = acc.map(|v| (v + 0.5) as u8);
            dst.set_pixel(x, y, out);
        }
    }
    dst
}

/// Gaussian blur on a grayscale mask.
pub fn gaussian_blur_gray(src: &GrayMask, sigma: f64) -> GrayMask {
    if sigma <= 0.0 {
        return src.clone();
    }
    let radius = (sigma * 4.0).ceil() as usize;
    let kernel = gaussian_kernel(sigma, radius);
    let mut tmp = GrayMask::new(src.width, src.height);
    let mut dst = GrayMask::new(src.width, src.height);
    for y in 0..src.height {
        for x in 0..src.width {
            let mut acc = 0.0f32;
            for (i, k) in kernel.iter().enumerate() {
                let sx = clamp_mirror_index(x as isize + i as isize - radius as isize, src.width as isize);
                acc += src.value(sx as usize, y) as f32 * *k as f32;
            }
            tmp.data[y * src.width + x] = (acc + 0.5) as u8;
        }
    }
    for y in 0..src.height {
        for x in 0..src.width {
            let mut acc = 0.0f32;
            for (i, k) in kernel.iter().enumerate() {
                let sy = clamp_mirror_index(y as isize + i as isize - radius as isize, src.height as isize);
                acc += tmp.value(x, sy as usize) as f32 * *k as f32;
            }
            dst.data[y * src.width + x] = (acc + 0.5) as u8;
        }
    }
    dst
}

fn clamp_mirror_index(i: isize, len: isize) -> isize {
    if len <= 0 {
        return 0;
    }
    if i < 0 {
        0
    } else if i >= len {
        len - 1
    } else {
        i
    }
}

fn gaussian_kernel(sigma: f64, radius: usize) -> Vec<f32> {
    let mut kernel = vec![0.0f32; radius * 2 + 1];
    let mut sum = 0.0f64;
    for (i, k) in kernel.iter_mut().enumerate() {
        let d = i as f64 - radius as f64;
        let v = (-(d * d) / (2.0 * sigma * sigma)).exp();
        *k = v as f32;
        sum += v;
    }
    for k in kernel.iter_mut() {
        *k = (*k as f64 / sum) as f32;
    }
    kernel
}

/// Tight bounds of nonzero coverage in a mask region: port of
/// `heal_coverage_bounds` (HealPixels.c). Returns None when empty.
pub fn coverage_bounds(mask: &GrayMask) -> Option<CGRect> {
    let mut min_x = mask.width as isize;
    let mut min_y = mask.height as isize;
    let mut max_x = -1isize;
    let mut max_y = -1isize;
    for y in 0..mask.height {
        let row = &mask.data[y * mask.width..];
        for x in 0..mask.width {
            if row[x] != 0 {
                if (x as isize) < min_x {
                    min_x = x as isize;
                }
                if x as isize > max_x {
                    max_x = x as isize;
                }
                if (y as isize) < min_y {
                    min_y = y as isize;
                }
                max_y = y as isize;
            }
        }
    }
    if max_x <= min_x || max_y <= min_y {
        return None;
    }
    Some(CGRect::new(
        min_x as CGFloat,
        min_y as CGFloat,
        (max_x - min_x + 1) as CGFloat,
        (max_y - min_y + 1) as CGFloat,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::{LayerSampling, LayerTransform};
    use crate::geometry::{CGPoint, CGSize};

    #[test]
    fn falloff_matches_formula() {
        // exp(-2.5)/1-exp(-2.5) normalizes to 0 at u = 1 and 1 at u = 0.
        assert!((falloff(0.0) - 1.0).abs() < 1e-9);
        assert!(falloff(1.0).abs() < 1e-9);
        let mid = falloff(0.5);
        let expected: f64 = ((-2.5f64 * 0.25).exp() - (-2.5f64).exp()) / (1.0 - (-2.5f64).exp());
        assert!((mid - expected).abs() < 1e-9);
    }

    #[test]
    fn spacing_fractions() {
        assert_eq!(spacing_fraction(1.0), 0.015);
        assert_eq!(spacing_fraction(0.0), 0.025);
    }

    #[test]
    fn pixel_to_document_identity_unit() {
        let t = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 50.0));
        let m = pixel_to_document(&t, 100.0, 50.0);
        // Pixel (0,0) maps to document (0,0); (100,50) maps to (100,50).
        let a = CGPoint::new(0.0, 0.0).applying(&m);
        assert!((a.x - 0.0).abs() < 1e-9 && (a.y - 0.0).abs() < 1e-9);
        let b = CGPoint::new(100.0, 50.0).applying(&m);
        assert!((b.x - 100.0).abs() < 1e-9 && (b.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn pixel_to_document_flip_x_mirrors() {
        let mut t = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 50.0));
        t.flip_x = true;
        let m = pixel_to_document(&t, 100.0, 50.0);
        let p = CGPoint::new(100.0, 0.0).applying(&m);
        assert!((p.x - 0.0).abs() < 1e-9, "{p:?}");
        let q = CGPoint::new(0.0, 0.0).applying(&m);
        assert!((q.x - 100.0).abs() < 1e-9, "{q:?}");
    }

    #[test]
    fn alpha_bounds_finds_tight_box() {
        let mut b = Bitmap::new(10, 10);
        b.set_pixel(3, 4, [10, 20, 30, 255]);
        b.set_pixel(7, 8, [0, 0, 0, 1]);
        let bounds = b.alpha_bounds().unwrap();
        assert_eq!(bounds, crate::geometry::CGRect::new(3.0, 4.0, 5.0, 5.0));
    }

    #[test]
    fn alpha_bounds_none_when_transparent() {
        let b = Bitmap::new(5, 5);
        assert!(b.alpha_bounds().is_none());
    }

    #[test]
    fn source_over_u8_opaque_replaces() {
        assert_eq!(source_over_u8([1, 2, 3, 4], [9, 9, 9, 255]), [9, 9, 9, 255]);
    }

    #[test]
    fn source_over_u8_transparent_keeps() {
        assert_eq!(source_over_u8([200, 100, 50, 255], [0, 0, 0, 0]), [200, 100, 50, 255]);
    }

    #[test]
    fn halve_averages_quads() {
        let mut b = Bitmap::new(2, 2);
        b.set_pixel(0, 0, [0, 0, 0, 0]);
        b.set_pixel(1, 0, [100, 100, 100, 100]);
        b.set_pixel(0, 1, [100, 100, 100, 100]);
        b.set_pixel(1, 1, [100, 100, 100, 100]);
        let h = halve(&b);
        assert_eq!(h.pixel(0, 0)[0], 75);
    }

    #[test]
    fn resample_nearest_picks_centers() {
        let mut b = Bitmap::new(2, 1);
        b.set_pixel(0, 0, [10, 0, 0, 255]);
        b.set_pixel(1, 0, [200, 0, 0, 255]);
        let out = resample(&b, 4, 1, LayerSampling::Nearest);
        assert_eq!(out.pixel(0, 0)[0], 10);
        assert_eq!(out.pixel(3, 0)[0], 200);
    }

    #[test]
    fn gaussian_blur_gray_spreads() {
        let mut m = GrayMask::from_raw(21, 1, vec![0; 21]);
        m.data[10] = 255;
        let blurred = gaussian_blur_gray(&m, 1.0);
        let peak = blurred.value(10, 0);
        assert!(peak < 255 && peak > 100);
        assert!(blurred.value(8, 0) > 0 && blurred.value(8, 0) < peak);
        // Symmetric.
        assert_eq!(blurred.value(9, 0), blurred.value(11, 0));
        // Preserves total mass approximately.
        let sum: u64 = blurred.data.iter().map(|&v| v as u64).sum();
        assert!((sum as i64 - 255).abs() < 10, "sum={sum}");
    }

    #[test]
    fn thumbnail_caps_long_side_at_96() {
        let b = Bitmap::new(300, 150);
        let t = thumbnail_of(&b);
        assert_eq!(t.width.max(t.height), 96);
        let ratio = t.width as f64 / t.height as f64;
        assert!((ratio - 2.0).abs() < 0.05);
    }

    #[test]
    fn draw_over_composites() {
        let mut dst = Bitmap::new_clearing(2, 1, [255, 0, 0, 255]);
        let mut src = Bitmap::new(1, 1);
        src.set_pixel(0, 0, [0, 255, 0, 128]);
        draw_over(&mut dst, &src, crate::geometry::CGRect::new(1.0, 0.0, 1.0, 1.0), LayerSampling::Nearest);
        assert_eq!(dst.pixel(0, 0)[0], 255);
        assert_eq!(dst.pixel(1, 0), [127, 128, 0, 255]);
    }
}
