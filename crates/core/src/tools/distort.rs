//! Port of `Document/Distort.swift`: free distortion — the layer's four
//! corners move independently, and on apply the pixels (and mask) are
//! resampled into the new shape.

use crate::geometry::{CGAffineTransform, CGPoint, CGRect, CGSize};
use crate::raster::Bitmap;
use crate::selection::SelectionPath;
use crate::transform::LayerTransform;

/// The transform's corners in handle order: top-left, top-right, bottom-right,
/// bottom-left (`DistortWarp.corners`).
pub fn corners_of(transform: &LayerTransform) -> [CGPoint; 4] {
    transform.corners()
}

/// Four finite corners with some area to them (`DistortWarp.isUsable`).
pub fn is_usable(corners: &[CGPoint; 4]) -> bool {
    if corners
        .iter()
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1_000_000.0 || p.y.abs() > 1e6 as f64)
    {
        return false;
    }
    // Both halves need area, or one of them has nothing to draw.
    area(corners[0], corners[1], corners[2]).abs() > 0.01
        && area(corners[0], corners[2], corners[3]).abs() > 0.01
}

fn area(a: CGPoint, b: CGPoint, c: CGPoint) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// A shape a perspective warp can take: convex, wound consistently either way
/// (`DistortWarp.isConvex`).
pub fn is_convex(corners: &[CGPoint; 4]) -> bool {
    if !is_usable(corners) {
        return false;
    }
    let mut sign = 0.0f64;
    for index in 0..4 {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        let c = corners[(index + 2) % 4];
        let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        if cross.abs() <= 0.01 {
            return false;
        }
        let current = if cross < 0.0 { -1.0 } else { 1.0 };
        if sign == 0.0 {
            sign = current;
        } else if (cross < 0.0) != (sign < 0.0) {
            return false;
        }
    }
    true
}

/// The affine map taking three source points to three destination points.
fn affine_between(
    s0: CGPoint,
    s1: CGPoint,
    s2: CGPoint,
    t0: CGPoint,
    t1: CGPoint,
    t2: CGPoint,
) -> Option<CGAffineTransform> {
    let u = CGPoint::new(s1.x - s0.x, s1.y - s0.y);
    let v = CGPoint::new(s2.x - s0.x, s2.y - s0.y);
    let uu = CGPoint::new(t1.x - t0.x, t1.y - t0.y);
    let vv = CGPoint::new(t2.x - t0.x, t2.y - t0.y);
    let det = u.x * v.y - v.x * u.y;
    if det.abs() <= 1e-9 {
        return None;
    }
    let a = (uu.x * v.y - vv.x * u.y) / det;
    let c = (vv.x * u.x - uu.x * v.x) / det;
    let b = (uu.y * v.y - vv.y * u.y) / det;
    let d = (vv.y * u.x - uu.y * v.x) / det;
    Some(CGAffineTransform {
        a,
        b,
        c,
        d,
        tx: t0.x - (a * s0.x + c * s0.y),
        ty: t0.y - (b * s0.x + d * s0.y),
    })
}

/// The perspective mapping of the unit square onto `c`
/// (`DistortWarp.homography`): the corners land on `c` in handle order.
#[derive(Debug, Clone, Copy)]
pub struct Homography {
    a: f64,
    b: f64,
    d: f64,
    e: f64,
    g: f64,
    h: f64,
    x0: f64,
    y0: f64,
}

impl Homography {
    pub fn new(c: &[CGPoint; 4]) -> Self {
        let sx = c[0].x - c[1].x + c[2].x - c[3].x;
        let sy = c[0].y - c[1].y + c[2].y - c[3].y;
        let (mut g, mut h) = (0.0f64, 0.0f64);
        if sx.abs() > 1e-9 || sy.abs() > 1e-9 {
            let dx1 = c[1].x - c[2].x;
            let dx2 = c[3].x - c[2].x;
            let dy1 = c[1].y - c[2].y;
            let dy2 = c[3].y - c[2].y;
            let den = dx1 * dy2 - dx2 * dy1;
            if den.abs() > 1e-12 {
                g = (sx * dy2 - dx2 * sy) / den;
                h = (dx1 * sy - sx * dy1) / den;
            }
        }
        Homography {
            a: c[1].x - c[0].x + g * c[1].x,
            b: c[3].x - c[0].x + h * c[3].x,
            d: c[1].y - c[0].y + g * c[1].y,
            e: c[3].y - c[0].y + h * c[3].y,
            g,
            h,
            x0: c[0].x,
            y0: c[0].y,
        }
    }

    pub fn apply(&self, p: CGPoint) -> CGPoint {
        let w = self.g * p.x + self.h * p.y + 1.0;
        CGPoint::new(
            (self.a * p.x + self.b * p.y + self.x0) / w,
            (self.d * p.x + self.e * p.y + self.y0) / w,
        )
    }

    /// The perspective mapping in reverse: a point the unit square maps to,
    /// taken back to its position on the unit square.
    pub fn apply_inverse(&self, p: CGPoint) -> CGPoint {
        let det = (self.a - self.g * p.x) * (self.e - self.h * p.y)
            - (self.b - self.h * p.x) * (self.d - self.g * p.y);
        if det.abs() < 1e-12 {
            return CGPoint::new(f64::NAN, f64::NAN);
        }
        let x = ((p.x - self.x0) * (self.e - self.h * p.y)
            - (self.b - self.h * p.x) * (p.y - self.y0))
            / det;
        let y = ((self.a - self.g * p.x) * (p.y - self.y0)
            - (self.d - self.g * p.y) * (p.x - self.x0))
            / det;
        CGPoint::new(x, y)
    }
}

/// Where each corner of the image's own pixels lands: a flipped layer shows
/// its pixels mirrored, so they go to the opposite corners of the shape
/// (`imageCorners`). Returns (top-left, top-right, bottom-right, bottom-left).
fn image_corners(
    corners: &[CGPoint; 4],
    flip_x: bool,
    flip_y: bool,
) -> (CGPoint, CGPoint, CGPoint, CGPoint) {
    let corner = |x: usize, y: usize| {
        let u = if flip_x { 1 - x } else { x };
        let v = if flip_y { 1 - y } else { y };
        corners[[0, 1, 3, 2][v * 2 + u]]
    };
    (corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1))
}

/// `image`, shown through `transform`, resampled so its corners land on
/// `corners`. Returns the warped pixels over the shape's whole-pixel bounds
/// and the axis-aligned transform for them. `limit` caps the longest side for
/// previews. A convex shape warps in perspective; a folded one warps as two
/// triangles along its diagonal.
pub fn warp(
    image: &Bitmap,
    transform: &LayerTransform,
    corners: &[CGPoint; 4],
    is_mask: bool,
    limit: Option<f64>,
) -> Result<(Bitmap, LayerTransform), String> {
    if !is_usable(corners) {
        return Err("invalid distortion".to_string());
    }
    let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_x = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let max_y = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let min_x = min_x.floor();
    let min_y = min_y.floor();
    let bounds = CGRect::new(min_x, min_y, max_x.ceil() - min_x, max_y.ceil() - min_y);
    if bounds.width() < 1.0 || bounds.height() < 1.0 {
        return Err("invalid distortion".to_string());
    }
    if bounds.width() > 30_000.0 || bounds.height() > 30_000.0 || bounds.width() * bounds.height() > 100_000_000.0 {
        return Err("the distorted shape is too large".to_string());
    }
    let placed = LayerTransform {
        origin: CGPoint::new(bounds.min_x(), bounds.min_y()),
        size: CGSize::new(bounds.width(), bounds.height()),
        ..*transform
    };
    // A uniform 1 × 1 mask already covers any shape.
    if is_mask && image.width == 1 && image.height == 1 {
        return Ok((image.clone(), placed));
    }
    let factor = limit
        .map(|cap| (cap / bounds.width().max(bounds.height())).min(1.0))
        .unwrap_or(1.0);
    let width = ((bounds.width() * factor).ceil() as usize).max(1);
    let height = ((bounds.height() * factor).ceil() as usize).max(1);
    let target = image_corners(corners, transform.flip_x, transform.flip_y);
    // Output-space corners: the shape over the warp's own pixel grid.
    let place = |p: CGPoint| CGPoint::new((p.x - min_x) * factor, (p.y - min_y) * factor);
    let dst = (place(target.0), place(target.1), place(target.2), place(target.3));
    // The source's own pixel corners, in the same order.
    let (sw, sh) = (image.width as f64, image.height as f64);
    let src = (
        CGPoint::new(0.0, 0.0),
        CGPoint::new(sw, 0.0),
        CGPoint::new(sw, sh),
        CGPoint::new(0.0, sh),
    );
    let mut out = Bitmap::new(width, height);
    if is_convex(corners) {
        // Perspective: the unit square maps to the source pixels and to the
        // output shape; every output pixel samples the source through the two
        // mappings composed, trusting only pixels inside the shape.
        let to_src = Homography::new(&[src.0, src.1, src.2, src.3]);
        let to_dst = Homography::new(&[dst.0, dst.1, dst.2, dst.3]);
        for y in 0..height {
            for x in 0..width {
                let p = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5);
                if !point_in_quad(dst, p) {
                    continue;
                }
                let unit = to_dst.apply_inverse(p);
                let source = to_src.apply(unit);
                sample_into(image, source, is_mask, &mut out, x, y, 1.0);
            }
        }
    } else {
        // A folded shape: each half either side of the TL–BR diagonal is taken
        // there by its own affine map, hard-edged along the diagonal.
        let halves = [
            ((src.0, src.1, src.2), (dst.0, dst.1, dst.2)),
            ((src.0, src.2, src.3), (dst.0, dst.2, dst.3)),
        ];
        for y in 0..height {
            for x in 0..width {
                let p = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5);
                for (from, to) in &halves {
                    if !point_in_triangle(p, to.0, to.1, to.2) {
                        continue;
                    }
                    if let Some(map) = affine_between(from.0, from.1, from.2, to.0, to.1, to.2) {
                        // Output pixel → source pixel: the affine's inverse.
                        let inverse = map.inverted();
                        let source = CGPoint::new(
                            inverse.a * p.x + inverse.c * p.y + inverse.tx,
                            inverse.b * p.x + inverse.d * p.y + inverse.ty,
                        );
                        sample_into(image, source, is_mask, &mut out, x, y, 1.0);
                    }
                    break;
                }
            }
        }
    }
    Ok((out, placed))
}

/// Writes one output pixel: samples `image` at `source` (bilinear), fading by
/// `coverage`.
fn sample_into(image: &Bitmap, source: CGPoint, is_mask: bool, out: &mut Bitmap, x: usize, y: usize, coverage: f64) {
    if coverage <= 0.0 {
        return;
    }
    let o = (y * out.width + x) * 4;
    if source.x < -1.0 || source.y < -1.0 || source.x > image.width as f64 || source.y > image.height as f64 {
        return;
    }
    let sample = crate::raster::sample_bitmap(
        image,
        source.x.clamp(0.0, image.width as f64 - 1.0),
        source.y.clamp(0.0, image.height as f64 - 1.0),
        crate::transform::LayerSampling::Smooth,
    );
    if is_mask {
        let v = (sample[0] as f64 * coverage) as u8;
        out.data[o] = v;
        out.data[o + 1] = v;
        out.data[o + 2] = v;
        out.data[o + 3] = 255;
    } else if coverage >= 1.0 {
        out.data[o..o + 4].copy_from_slice(&sample);
    } else {
        for c in 0..4 {
            out.data[o + c] = (sample[c] as f64 * coverage) as u8;
        }
    }
}

fn point_in_triangle(p: CGPoint, a: CGPoint, b: CGPoint, c: CGPoint) -> bool {
    let d1 = area(p, a, b);
    let d2 = area(p, b, c);
    let d3 = area(p, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

/// Even-odd point-in-quad, so folded shapes still clip correctly.
fn point_in_quad(quad: (CGPoint, CGPoint, CGPoint, CGPoint), p: CGPoint) -> bool {
    let pts = [quad.0, quad.1, quad.2, quad.3];
    let mut inside = false;
    let mut j = 3;
    for i in 0..4 {
        let (a, b) = (pts[i], pts[j]);
        if ((a.y > p.y) != (b.y > p.y))
            && (p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `DistortWarp.warpTrimmed`: a full-resolution warp cropped to its visible
/// pixels. Returns the cropped pixels, the transform placing them, and the
/// crop rect (in the warp's pixels, for cropping a mask to match).
pub fn warp_trimmed(
    image: &Bitmap,
    transform: &LayerTransform,
    corners: &[CGPoint; 4],
) -> Result<(Bitmap, LayerTransform, CGRect), String> {
    let (warped, placed) = warp(image, transform, corners, false, None)?;
    let full = CGRect::new(0.0, 0.0, warped.width as f64, warped.height as f64);
    let crop = warped.alpha_bounds().unwrap_or(full);
    if crop.width() < 1.0 || crop.height() < 1.0 || crop == full {
        return Ok((warped, placed, full));
    }
    let cropped = crate::filters::crop_bitmap(&warped, &crop);
    let mut placed = placed;
    placed.origin = CGPoint::new(placed.origin.x + crop.min_x(), placed.origin.y + crop.min_y());
    placed.size = CGSize::new(crop.width(), crop.height());
    Ok((cropped, placed, crop))
}

/// `DistortWarp.carried`: where `placement`'s corners land when the
/// perspective taking `transform`'s corners to `corners` is applied around it
/// too — how a linked mask placed apart distorts with the layer.
pub fn carried(placement: &LayerTransform, by: &LayerTransform, to: &[CGPoint; 4]) -> [CGPoint; 4] {
    let center = by.center();
    let to_unit = CGAffineTransform::translation(center.x, center.y)
        .concat(&CGAffineTransform::rotation(by.radians()))
        .concat(&CGAffineTransform::scale(by.size.width, by.size.height))
        .concat(&CGAffineTransform::translation(-0.5, -0.5))
        .inverted();
    let map = Homography::new(to);
    placement
        .corners()
        .map(|p| map.apply(p.applying(&to_unit)))
}

/// `DistortWarp.warpMask`: a mask warped like `warp`, but `background` (its
/// tone past its pixels) outside the shape instead of black.
pub fn warp_mask(
    image: &Bitmap,
    transform: &LayerTransform,
    corners: &[CGPoint; 4],
    background: f64,
    limit: Option<f64>,
) -> Result<(Bitmap, LayerTransform), String> {
    let (warped, placed) = warp(image, transform, corners, true, limit)?;
    if background <= 0.0 {
        return Ok((warped, placed));
    }
    let fill = ((background * 255.0).round() as u8).clamp(0, 255);
    let mut out = warped.clone();
    for y in 0..out.height {
        for x in 0..out.width {
            let o = (y * out.width + x) * 4;
            let p = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5);
            let (min_x, min_y) = (placed.origin.x, placed.origin.y);
            let sx = (placed.size.width / out.width as f64).max(f64::EPSILON);
            let sy = (placed.size.height / out.height as f64).max(f64::EPSILON);
            let doc = CGPoint::new(min_x + p.x * sx, min_y + p.y * sy);
            let quad = (
                corners[0],
                corners[1],
                corners[2],
                corners[3],
            );
            if !point_in_quad(quad, doc) {
                out.data[o] = fill;
                out.data[o + 1] = fill;
                out.data[o + 2] = fill;
                out.data[o + 3] = 255;
            }
        }
    }
    Ok((out, placed))
}

/// `DistortWarp.mapPath`: carries an outline drawn over the original pixels
/// (placed by `pixel_to_document`) into the distorted shape.
pub fn map_path(
    path: &SelectionPath,
    pixel_to_document: &CGAffineTransform,
    pixel_size: CGSize,
    transform: &LayerTransform,
    corners: &[CGPoint; 4],
) -> Option<SelectionPath> {
    if !is_convex(corners) || pixel_size.width <= 0.0 || pixel_size.height <= 0.0 {
        return None;
    }
    let to_pixels = pixel_to_document.inverted();
    let map = Homography::new(corners);
    let carry = |point: CGPoint| -> CGPoint {
        let pixel = point.applying(&to_pixels);
        let mut u = pixel.x / pixel_size.width;
        let mut v = pixel.y / pixel_size.height;
        if transform.flip_x {
            u = 1.0 - u;
        }
        if transform.flip_y {
            v = 1.0 - v;
        }
        map.apply(CGPoint::new(u, v))
    };
    Some(SelectionPath {
        rings: path
            .rings
            .iter()
            .map(|ring| ring.iter().map(|p| carry(*p)).collect())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::CGSize;

    fn unit_transform() -> LayerTransform {
        LayerTransform::new(CGPoint::ZERO, CGSize::new(40.0, 40.0))
    }

    fn red_block(width: usize, height: usize) -> Bitmap {
        let mut image = Bitmap::new(width, height);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel[0] = 255;
            pixel[3] = 255;
        }
        image
    }

    fn moved_corners(dx: f64, dy: f64, skew: f64) -> [CGPoint; 4] {
        [
            CGPoint::new(dx, dy),
            CGPoint::new(40.0 + dx + skew, dy),
            CGPoint::new(40.0 + dx, 40.0 + dy),
            CGPoint::new(dx, 40.0 + dy),
        ]
    }

    #[test]
    fn usability_and_convexity() {
        assert!(is_usable(&corners_of(&unit_transform())));
        assert!(is_convex(&corners_of(&unit_transform())));
        // A corner dragged past its neighbour folds the shape.
        let folded = [
            CGPoint::new(50.0, 0.0),
            CGPoint::new(0.0, 0.0),
            CGPoint::new(40.0, 40.0),
            CGPoint::new(0.0, 40.0),
        ];
        assert!(is_usable(&folded), "folded still has area");
        assert!(!is_convex(&folded));
        // A collapsed shape is unusable.
        let collapsed = [
            CGPoint::new(0.0, 0.0),
            CGPoint::new(40.0, 0.0),
            CGPoint::new(20.0, 0.0),
            CGPoint::new(0.0, 40.0),
        ];
        assert!(!is_usable(&collapsed));
    }

    #[test]
    fn warp_perspective_moves_pixels_with_the_corners() {
        let image = red_block(40, 40);
        let transform = unit_transform();
        let corners = moved_corners(10.0, 5.0, 0.0);
        let (warped, placed) = warp(&image, &transform, &corners, false, None).unwrap();
        assert_eq!(placed.origin, CGPoint::new(10.0, 5.0));
        assert_eq!((warped.width, warped.height), (40, 40));
        // The whole bounds sit inside the quad, so everything is opaque red.
        assert_eq!(warped.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(warped.pixel(35, 35), [255, 0, 0, 255]);
    }

    #[test]
    fn warp_folded_still_draws_both_halves() {
        let mut striped = Bitmap::new(40, 40);
        for y in 0..40 {
            for x in 0..40 {
                let v = if x < 20 { 255 } else { 0 };
                striped.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        let folded = [
            CGPoint::new(50.0, 0.0),
            CGPoint::new(0.0, 0.0),
            CGPoint::new(40.0, 40.0),
            CGPoint::new(0.0, 40.0),
        ];
        let (warped, _) = warp(&striped, &unit_transform(), &folded, false, None).unwrap();
        // Both halves of the fold carry opaque pixels.
        let left = warped.pixel(45, 5);
        let right = warped.pixel(30, 35);
        assert!(left[3] > 200, "{left:?}");
        assert!(right[3] > 200, "{right:?}");
    }

    #[test]
    fn warp_trims_to_visible_pixels() {
        let mut sparse = Bitmap::new(40, 40);
        for y in 10..20 {
            for x in 10..30 {
                sparse.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        let (cropped, placed, crop) = warp_trimmed(&sparse, &unit_transform(), &corners_of(&unit_transform())).unwrap();
        assert!((18..=22).contains(&cropped.width), "{}", cropped.width);
        assert!((8..=12).contains(&cropped.height), "{}", cropped.height);
        assert!((placed.origin.x - 10.0).abs() <= 1.5, "{:?}", placed.origin);
        assert!((placed.origin.y - 10.0).abs() <= 1.5, "{:?}", placed.origin);
        assert!((19.0..=21.0).contains(&crop.width()), "{}", crop.width());
        assert!((9.0..=11.0).contains(&crop.height()), "{}", crop.height());
    }

    #[test]
    fn carried_maps_the_placement_through_the_perspective() {
        let transform = unit_transform();
        let placement = LayerTransform::new(CGPoint::new(5.0, 5.0), CGSize::new(20.0, 20.0));
        let corners = moved_corners(10.0, 0.0, 0.0);
        let carried = carried(&placement, &transform, &corners);
        // The placement's top-left sits at (5,5) in the layer → (15,0) after.
        assert!((carried[0].x - 15.0).abs() < 1e-6, "{:?}", carried[0]);
        assert!((carried[0].y - 5.0).abs() < 1e-6, "{:?}", carried[0]);
    }

    #[test]
    fn map_path_carries_an_outline() {
        let path = SelectionPath::rect(CGRect::new(10.0, 10.0, 20.0, 20.0));
        let transform = unit_transform();
        let to_doc = crate::raster::pixel_to_document(&transform, 40.0, 40.0);
        let corners = moved_corners(10.0, 0.0, 0.0);
        let carried = map_path(&path, &to_doc, CGSize::new(40.0, 40.0), &transform, &corners).unwrap();
        let bounds = carried.bounds().unwrap();
        assert!((bounds.min_x() - 20.0).abs() < 1.0, "{bounds:?}");
        assert!((bounds.min_y() - 10.0).abs() < 1.0, "{bounds:?}");
        // Non-convex shapes carry nothing.
        let folded = [
            CGPoint::new(50.0, 0.0),
            CGPoint::new(0.0, 0.0),
            CGPoint::new(40.0, 40.0),
            CGPoint::new(0.0, 40.0),
        ];
        assert!(map_path(&path, &to_doc, CGSize::new(40.0, 40.0), &transform, &folded).is_none());
    }
}
