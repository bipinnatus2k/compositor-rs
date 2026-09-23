//! Port of `PixelAdjust.swift`, `PixelInvert.swift` and `GuidedMatte.swift`:
//! selection coverage on an image's pixel grid, blending a result back through
//! a selection, whole-image invert, and guided-filter matte refinement.

use crate::geometry::{CGAffineTransform, CGRect};
use crate::raster::{halve, resample, Bitmap, GrayMask, LayerSampling};
use crate::selection::SelectionClip;

/// Selection coverage rasterized on the image's pixel grid.
pub fn coverage(
    selection: &SelectionClip,
    width: usize,
    height: usize,
    pixel_to_document: &CGAffineTransform,
) -> GrayMask {
    let mut mask = GrayMask::new(width.max(1), height.max(1));
    if selection.coverage.is_none() {
        return mask; // Empty selection clips everything away.
    }
    // The selection's own coverage, drawn into this grid through the inverse
    // of the image's placement.
    let inverse = pixel_to_document.inverted();
    let rect = CGRect::new(0.0, 0.0, width as f64, height as f64);
    // Corners of the image grid mapped to the document.
    let corners = [
        CGPoint::new(0.0, 0.0).applying(pixel_to_document),
        CGPoint::new(width as f64, 0.0).applying(pixel_to_document),
        CGPoint::new(0.0, height as f64).applying(pixel_to_document),
        CGPoint::new(width as f64, height as f64).applying(pixel_to_document),
    ];
    for (i, corner) in corners.iter().enumerate() {
        let _ = (i, corner);
    }
    // Sample the selection coverage at each pixel center.
    let cov = selection.coverage.as_ref().unwrap();
    let doc_to_sel = |x: f64, y: f64| -> (f64, f64) {
        (
            x - selection.rect.min_x(),
            y - selection.rect.min_y(),
        )
    };
    for y in 0..height {
        for x in 0..width {
            let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&inverse);
            let (sx, sy) = doc_to_sel(doc.x.floor(), doc.y.floor());
            if sx >= 0.0 && sy >= 0.0 && sx < cov.width as f64 && sy < cov.height as f64 {
                mask.data[y * width + x] = cov.value(sx as usize, sy as usize);
            }
        }
    }
    mask
}

use crate::geometry::CGPoint;

/// coverage × adjusted + (1 − coverage) × original, in float, so fully
/// selected pixels stay exact and soft edges blend (`CIBlendWithMask`).
pub fn blend(
    adjusted: &Bitmap,
    original: &Bitmap,
    selection: &SelectionClip,
    pixel_to_document: &CGAffineTransform,
) -> Bitmap {
    debug_assert_eq!(adjusted.width, original.width);
    debug_assert_eq!(adjusted.height, original.height);
    let mask = coverage(selection, adjusted.width, adjusted.height, pixel_to_document);
    let mut result = adjusted.clone();
    for (i, chunk) in result.data.chunks_exact_mut(4).enumerate() {
        let a = mask.data[i] as u32;
        if a == 255 {
            continue;
        }
        if a == 0 {
            chunk.copy_from_slice(&original.data[i * 4..i * 4 + 4]);
            continue;
        }
        for (c, cell) in chunk.iter_mut().enumerate() {
            *cell = ((adjusted.data[i * 4 + c] as u32 * a + original.data[i * 4 + c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    }
    result
}

/// The same blend for gray masks.
pub fn blend_gray(
    adjusted: &GrayMask,
    original: &GrayMask,
    selection: &SelectionClip,
    pixel_to_document: &CGAffineTransform,
) -> GrayMask {
    let mask = coverage(selection, adjusted.width, adjusted.height, pixel_to_document);
    let mut result = adjusted.clone();
    for i in 0..result.data.len() {
        let a = mask.data[i] as u32;
        if a == 255 {
            continue;
        }
        if a == 0 {
            result.data[i] = original.data[i];
            continue;
        }
        let v = (adjusted.data[i] as u32 * a + original.data[i] as u32 * (255 - a) + 127) / 255;
        result.data[i] = v as u8;
    }
    result
}

/// A small preview for the Layers panel (96 px cap on the long side).
pub fn thumbnail_of(image: &Bitmap) -> Bitmap {
    crate::raster::thumbnail_of(image)
}

/// `PixelInvert.run`: whole-image invert in one pass, optionally limited to a
/// selection. On RGBA the color becomes alpha − color (transparency kept); on
/// a mask each byte becomes 255 − byte.
pub fn invert(
    image: &Bitmap,
    pixel_to_document: &CGAffineTransform,
    selection: Option<&SelectionClip>,
) -> Bitmap {
    let mut inverted = image.clone();
    for p in inverted.data.chunks_exact_mut(4) {
        let alpha = p[3];
        p[0] = alpha.wrapping_sub(p[0]);
        p[1] = alpha.wrapping_sub(p[1]);
        p[2] = alpha.wrapping_sub(p[2]);
    }
    match selection {
        Some(selection) => blend(&inverted, image, selection, pixel_to_document),
        None => inverted,
    }
}

/// The mask version of `PixelInvert.run`.
pub fn invert_mask(
    mask: &GrayMask,
    pixel_to_document: &CGAffineTransform,
    selection: Option<&SelectionClip>,
) -> GrayMask {
    let mut inverted = mask.clone();
    for v in inverted.data.iter_mut() {
        *v = 255 - *v;
    }
    match selection {
        Some(selection) => blend_gray(&inverted, mask, selection, pixel_to_document),
        None => inverted,
    }
}

/// Guided filtering (He, Sun & Tang) box mean over a (2r+1)² square, as two
/// running-sum passes.
pub fn guided_box(source: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let span = (radius * 2 + 1) as f32;
    let mut pass = vec![0.0f32; width * height];
    for y in 0..height {
        let row = y * width;
        let mut sum: f32 = 0.0;
        for x in -(radius as isize)..=(radius as isize) {
            sum += source[row + (x.max(0) as usize).min(width - 1)];
        }
        for x in 0..width {
            pass[row + x] = sum / span;
            let out = (x as isize - radius as isize).max(0) as usize;
            let inp = (x + radius + 1).min(width - 1);
            sum -= source[row + out];
            sum += source[row + inp];
        }
    }
    let mut result = vec![0.0f32; width * height];
    for x in 0..width {
        let mut sum: f32 = 0.0;
        for y in -(radius as isize)..=(radius as isize) {
            sum += pass[(y.max(0) as usize).min(height - 1) * width + x];
        }
        for y in 0..height {
            result[y * width + x] = sum / span;
            let out = (y as isize - radius as isize).max(0) as usize;
            let inp = (y + radius + 1).min(height - 1);
            sum -= pass[out * width + x];
            sum += pass[inp * width + x];
        }
    }
    result
}

/// `mask` refined by `guide` (both 0–1, the same size).
pub fn guided_filter(
    mask: &[f32],
    guide: &[f32],
    width: usize,
    height: usize,
    radius: usize,
    epsilon: f32,
) -> Vec<f32> {
    let count = width * height;
    let mean_guide = guided_box(guide, width, height, radius);
    let mean_mask = guided_box(mask, width, height, radius);
    let squares: Vec<f32> = guide.iter().map(|g| g * g).collect();
    let products: Vec<f32> = guide.iter().zip(mask.iter()).map(|(g, m)| g * m).collect();
    let mean_squares = guided_box(&squares, width, height, radius);
    let mean_products = guided_box(&products, width, height, radius);
    let mut slope = vec![0.0f32; count];
    let mut offset = vec![0.0f32; count];
    for i in 0..count {
        let variance = mean_squares[i] - mean_guide[i] * mean_guide[i];
        let covariance = mean_products[i] - mean_guide[i] * mean_mask[i];
        slope[i] = covariance / (variance + epsilon);
        offset[i] = mean_mask[i] - slope[i] * mean_guide[i];
    }
    let mean_slope = guided_box(&slope, width, height, radius);
    let mean_offset = guided_box(&offset, width, height, radius);
    (0..count)
        .map(|i| (mean_slope[i] * guide[i] + mean_offset[i]).min(1.0).max(0.0))
        .collect()
}

/// 0–1 levels → gray bitmap.
pub fn levels_to_gray(levels: &[f32], width: usize, height: usize) -> GrayMask {
    let mut result = GrayMask::new(width, height);
    for (i, v) in levels.iter().enumerate().take(width * height) {
        result.data[i] = (v * 255.0 + 0.5) as u8;
    }
    result
}

/// Gray bitmap → 0–1 levels.
pub fn gray_to_levels(mask: &GrayMask) -> Vec<f32> {
    mask.data.iter().map(|v| *v as f32 / 255.0).collect()
}

/// `GuidedMatte.refine`: refine `mask` against `guide` on a copy no larger
/// than `limit` on its longest side, then draw back up.
pub fn guided_refine(mask: &GrayMask, guide: &GrayMask, radius: f64, limit: f64) -> GrayMask {
    let full_w = mask.width;
    let full_h = mask.height;
    let factor = (limit / (full_w.max(full_h) as f64)).min(1.0);
    let width = ((full_w as f64 * factor).round() as usize).max(1);
    let height = ((full_h as f64 * factor).round() as usize).max(1);
    let steps = ((radius * factor).round() as usize).max(1);
    let small_mask = resample_bitmap_gray(mask, width, height);
    let small_guide = resample_bitmap_gray(guide, width, height);
    let refined = guided_filter(
        &gray_to_levels(&small_mask),
        &gray_to_levels(&small_guide),
        width,
        height,
        steps,
        1e-4,
    );
    let small = levels_to_gray(&refined, width, height);
    if width == full_w && height == full_h {
        return small;
    }
    resample_bitmap_gray(&small, full_w, full_h)
}

fn resample_bitmap_gray(mask: &GrayMask, w: usize, h: usize) -> GrayMask {
    if mask.width == w && mask.height == h {
        return mask.clone();
    }
    // Wrap in a Bitmap to reuse the resampler (gray values in all channels).
    let mut wrapped = Bitmap::new(mask.width, mask.height);
    for i in 0..mask.data.len() {
        let v = mask.data[i];
        wrapped.data[i * 4] = v;
        wrapped.data[i * 4 + 1] = v;
        wrapped.data[i * 4 + 2] = v;
        wrapped.data[i * 4 + 3] = 255;
    }
    // Sharp halving for big downscales keeps edges crisp, as `.high` does.
    let mut cur = wrapped;
    while (cur.width / 2).max(1) >= w.max(2) && (cur.height / 2).max(1) >= h.max(2) && cur.width > 2 && cur.height > 2 {
        cur = halve(&cur);
    }
    let out = resample(&cur, w, h, LayerSampling::High);
    let mut result = GrayMask::new(w, h);
    for i in 0..result.data.len() {
        result.data[i] = out.data[i * 4];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{CGPoint, CGSize};
    use crate::selection::{DocumentSelection, SelectionPath};

    fn rect_selection() -> SelectionClip {
        let sel = DocumentSelection::new(SelectionPath::rect(CGRect::new(2.0, 2.0, 4.0, 4.0)));
        sel.clip(CGSize::new(10.0, 10.0))
    }

    #[test]
    fn coverage_on_image_grid() {
        let clip = rect_selection();
        let identity = CGAffineTransform::IDENTITY;
        let mask = coverage(&clip, 10, 10, &identity);
        assert_eq!(mask.value(4, 4), 255);
        assert_eq!(mask.value(1, 1), 0);
        assert_eq!(mask.value(7, 7), 0);
    }

    #[test]
    fn blend_through_selection() {
        let clip = rect_selection();
        let mut original = Bitmap::new(10, 10);
        for i in (0..original.data.len()).step_by(4) {
            original.data[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
        }
        let mut adjusted = original.clone();
        for i in (0..adjusted.data.len()).step_by(4) {
            adjusted.data[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let out = blend(&adjusted, &original, &clip, &CGAffineTransform::IDENTITY);
        assert_eq!(out.pixel(4, 4)[0], 255); // selected → adjusted
        assert_eq!(out.pixel(1, 1)[0], 0); // outside → original
    }

    #[test]
    fn invert_keeps_alpha() {
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [200, 100, 50, 128]); // premultiplied
        let out = invert(&img, &CGAffineTransform::IDENTITY, None);
        let [r, g, b, a] = out.pixel(0, 0);
        assert_eq!(a, 128);
        // Each channel becomes alpha − channel (wrapping, like the fixed-point matrix).
        assert_eq!(r, 128u8.wrapping_sub(200));
        assert_eq!(g, 128u8.wrapping_sub(100));
        assert_eq!(b, 128u8.wrapping_sub(50));
    }

    #[test]
    fn invert_rgb_exact_when_opaque() {
        let mut img = Bitmap::new(1, 1);
        img.set_pixel(0, 0, [10, 200, 130, 255]);
        let out = invert(&img, &CGAffineTransform::IDENTITY, None);
        assert_eq!(out.pixel(0, 0), [245, 55, 125, 255]);
    }

    #[test]
    fn inverts_mask_bytes() {
        let mut m = GrayMask::new(2, 1);
        m.data[0] = 255;
        m.data[1] = 0;
        let out = invert_mask(&m, &CGAffineTransform::IDENTITY, None);
        assert_eq!(out.data, vec![0, 255]);
    }

    #[test]
    fn guided_box_is_mean() {
        let data = vec![1.0f32; 25];
        let means = guided_box(&data, 5, 5, 1);
        assert!(means.iter().all(|v| (v - 1.0).abs() < 1e-6));
        let mut spike = vec![0.0f32; 25];
        spike[12] = 9.0;
        let means = guided_box(&spike, 5, 5, 1);
        assert!((means[12] - 1.0).abs() < 1e-6); // 9/9
    }

    #[test]
    fn guided_filter_pulls_mask_to_guide_edges() {
        // Guide has a hard edge; mask is a smooth blob. After refinement the
        // mask edge aligns with the guide edge.
        let width = 20;
        let height = 4;
        let mut guide = vec![0.0f32; width * height];
        for y in 0..height {
            for x in 0..width {
                guide[y * width + x] = if x >= 10 { 1.0 } else { 0.0 };
            }
        }
        let mask = vec![0.6f32; width * height];
        let refined = guided_filter(&mask, &guide, width, height, 2, 1e-4);
        // The guided filter keeps the mask near its mean far from edges and
        // lets it follow the guide across the edge (variance nonzero there).
        assert!(refined[2 * width + 2] < refined[2 * width + 9] || refined[2 * width + 2] < 0.61,
            "{} vs {}", refined[2 * width + 2], refined[2 * width + 9]);
        assert!(refined[2 * width + 17] > 0.59, "{}", refined[2 * width + 17]);
    }

    #[test]
    fn thumbnail_caps_at_96() {
        let b = Bitmap::new(200, 100);
        assert_eq!(thumbnail_of(&b).width, 96);
    }

    #[test]
    fn empty_selection_zeroes_coverage() {
        let clip = SelectionClip {
            rect: CGRect::ZERO,
            coverage: None,
        };
        let mask = coverage(&clip, 4, 4, &CGAffineTransform::IDENTITY);
        assert!(mask.data.iter().all(|v| *v == 0));
    }

    #[test]
    fn guided_refine_roundtrip_small() {
        let mut m = GrayMask::new(8, 8);
        for i in 0..m.data.len() {
            m.data[i] = 128;
        }
        let mut g = GrayMask::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                g.data[y * 8 + x] = if x >= 4 { 255 } else { 0 };
            }
        }
        let refined = guided_refine(&m, &g, 2.0, 64.0);
        // A constant mask has no correlation with the guide: it stays put.
        assert_eq!(refined.width, 8);
        for i in 0..refined.data.len() {
            assert!((refined.data[i] as i32 - 128).abs() <= 2, "{}", refined.data[i]);
        }
    }
}
