//! Port of `Compositor/Document/ObjectSelection.swift`: selects the
//! foreground object under a clicked point and traces it into the document's
//! normal path-based selection.
//!
//! Swift asks Vision's `VNGenerateForegroundInstanceMaskRequest` which pixels
//! belong to the object (an instance-index mask plus a low-resolution soft
//! mask for the clicked instance) and then does all the real shaping itself:
//! an edge-preserving upsample of the coarse mask against the full image
//! (`CIEdgePreserveUpsampleFilter`, spatial σ 5 / luma σ 0.15), a 50% gray
//! threshold, an erode/dilate edge offset of at most 10 one-pixel steps, the
//! Magic Wand's boundary tracer, and optional RDP-simplify (tolerance 1.6) +
//! Chaikin smoothing (3 rounds of 0.25/0.75 corner cutting). Vision is the
//! one piece that cannot be ported, so it is an input: the caller supplies
//! the clicked instance's coarse soft mask (`select`) — `instance_index`
//! keeps the click-to-instance lookup for callers that have the index mask.

#![allow(dead_code)]

use crate::geometry::{CGPoint, CGSize};
use crate::raster::{Bitmap, GrayMask};
use crate::selection::SelectionPath;
use crate::tools::wand::wand_trace;

/// The Object mode's options-bar settings (`ObjectSelectionSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectSelectionSettings {
    /// Read the visible composite rather than just the active layer.
    pub sample_all_layers: bool,
    /// Positive values erode the detected mask inward; negative values
    /// expand it outward.
    pub edge_offset: i32,
}

impl Default for ObjectSelectionSettings {
    fn default() -> Self {
        ObjectSelectionSettings {
            sample_all_layers: true,
            edge_offset: 0,
        }
    }
}

/// `CIEdgePreserveUpsampleFilter.inputSpatialSigma` — spatial reach of the
/// refinement kernel, in coarse-mask cells.
const SPATIAL_SIGMA: f64 = 5.0;
/// `CIEdgePreserveUpsampleFilter.inputLumaSigma` — how much a guide-luma
/// difference (0–1) damps a kernel sample.
const LUMA_SIGMA: f64 = 0.15;
/// A refined pixel joins the object at 50% gray (`$0 >= 128 ? 255 : 0`).
const REFINEMENT_THRESHOLD: u8 = 128;
/// `adjusted` runs at most this many erode/dilate steps (`min(10, …)`), the
/// same cap the session applies to the setting itself.
const MAX_OFFSET_STEPS: i32 = 10;
/// `simplifyClosed` tolerance — the stair steps shorter than this vanish.
const SIMPLIFY_TOLERANCE: f64 = 1.6;
/// `chaikin` rounds applied after simplification.
const CHAIKIN_ITERATIONS: usize = 3;

/// `ObjectSelection.select(in:at:edgeOffset:smoothEdges:)`: the outline, in
/// the image's top-left pixel coordinates, of the object `coarse` masks —
/// the clicked instance's low-resolution soft mask, already resolved from
/// Vision's instance lookup. `None` when the point is outside the image or
/// nothing is selected. The refinement snaps the coarse boundary to the
/// image's edges; `edge_offset` then erodes (> 0) or dilates (< 0) the
/// binary mask up to 10 pixels, and `smooth_edges` rounds the stair steps.
pub fn select(
    image: &Bitmap,
    coarse: &GrayMask,
    point: CGPoint,
    edge_offset: i32,
    smooth_edges: bool,
) -> Option<SelectionPath> {
    let width = image.width;
    let height = image.height;
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    let x = point.x.floor();
    let y = point.y.floor();
    if x < 0.0 || y < 0.0 || x as usize >= width || y as usize >= height {
        return None;
    }
    let binary = edge_preserved_binary_mask(coarse, image, width, height);
    let mask = adjusted(&binary, width, height, edge_offset);
    let outline = outline_of_mask(&mask, width, height)?;
    Some(if smooth_edges {
        smoothed(&outline)
    } else {
        outline
    })
}

/// `instanceIndex`: an instance mask stores 0 for background and instance
/// indices (1…) for objects; the click's value picks the instance. `None` on
/// background or a degenerate size.
pub fn instance_index(instance_mask: &GrayMask, point: CGPoint, image_size: CGSize) -> Option<u8> {
    let width = instance_mask.width;
    let height = instance_mask.height;
    if width == 0 || height == 0 || image_size.width <= 0.0 || image_size.height <= 0.0 {
        return None;
    }
    let x =
        ((point.x / image_size.width * width as f64) as i64).clamp(0, width as i64 - 1) as usize;
    let y =
        ((point.y / image_size.height * height as f64) as i64).clamp(0, height as i64 - 1) as usize;
    let value = instance_mask.value(x, y);
    if value == 0 { None } else { Some(value) }
}

/// `edgePreservedBinaryMask`: the coarse mask upsampled to full size with
/// the image as the edge guide and thresholded at 50% gray. The upsample is
/// joint bilateral: each pixel averages the coarse values around it,
/// weighted by spatial distance (σ 5) and by the guide's luma difference
/// (σ 0.15), so the coarse boundary bends to the image's edges while noise
/// off the coarse region cannot pull it. Apple's kernel is closed; this is
/// the standard formulation the filter implements, with Rec. 709 luma on
/// the guide's (sRGB) bytes.
fn edge_preserved_binary_mask(
    coarse: &GrayMask,
    guide: &Bitmap,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let refined = joint_bilateral_upsample(coarse, guide, width, height);
    refined
        .into_iter()
        .map(|value| {
            if value >= REFINEMENT_THRESHOLD {
                255
            } else {
                0
            }
        })
        .collect()
}

fn joint_bilateral_upsample(
    coarse: &GrayMask,
    guide: &Bitmap,
    width: usize,
    height: usize,
) -> Vec<u8> {
    if width == 0 || height == 0 || coarse.width == 0 || coarse.height == 0 {
        return vec![0; width * height];
    }
    let radius = (SPATIAL_SIGMA * 2.0).ceil() as i64;
    let two_spatial = 2.0 * SPATIAL_SIGMA * SPATIAL_SIGMA;
    let two_luma = 2.0 * LUMA_SIGMA * LUMA_SIGMA;
    let (cw, ch) = (coarse.width as f64, coarse.height as f64);
    let mut out = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            // The pixel's center mapped into the coarse grid.
            let cx = (x as f64 + 0.5) * cw / width as f64 - 0.5;
            let cy = (y as f64 + 0.5) * ch / height as f64 - 0.5;
            let base_x = cx.round() as i64;
            let base_y = cy.round() as i64;
            let luma = guide_luma(guide, x, y);
            let mut sum = 0.0f64;
            let mut weight_sum = 0.0f64;
            for dy in -radius..=radius {
                let sample_y = base_y + dy;
                if sample_y < 0 || sample_y >= coarse.height as i64 {
                    continue;
                }
                for dx in -radius..=radius {
                    let sample_x = base_x + dx;
                    if sample_x < 0 || sample_x >= coarse.width as i64 {
                        continue;
                    }
                    // Guide luma under the coarse sample, where its value
                    // was measured.
                    let guide_x = (((sample_x as f64 + 0.5) * width as f64 / cw - 0.5).round()
                        as i64)
                        .clamp(0, width as i64 - 1) as usize;
                    let guide_y = (((sample_y as f64 + 0.5) * height as f64 / ch - 0.5).round()
                        as i64)
                        .clamp(0, height as i64 - 1) as usize;
                    let dl = luma - guide_luma(guide, guide_x, guide_y);
                    let ddx = sample_x as f64 - cx;
                    let ddy = sample_y as f64 - cy;
                    let weight = (-(ddx * ddx + ddy * ddy) / two_spatial).exp()
                        * (-(dl * dl) / two_luma).exp();
                    sum += weight * coarse.value(sample_x as usize, sample_y as usize) as f64;
                    weight_sum += weight;
                }
            }
            out[y * width + x] = if weight_sum > 0.0 {
                (sum / weight_sum).round() as u8
            } else {
                0
            };
        }
    }
    out
}

/// The guide's Rec. 709 luma in 0–1.
fn guide_luma(image: &Bitmap, x: usize, y: usize) -> f64 {
    let [r, g, b, _] = image.pixel(x, y);
    (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0
}

/// `adjusted(binaryMask:width:height:edgeOffset:)`: up to 10 erosions
/// (positive offset) or dilations (negative), one pixel each.
pub fn adjusted(mask: &[u8], width: usize, height: usize, edge_offset: i32) -> Vec<u8> {
    let mask = mask.to_vec();
    let steps = edge_offset.abs().min(MAX_OFFSET_STEPS);
    if steps == 0 || width == 0 || height == 0 {
        return mask;
    }
    let mut mask = mask;
    for _ in 0..steps {
        mask = if edge_offset > 0 {
            eroded(&mask, width, height)
        } else {
            dilated(&mask, width, height)
        };
    }
    mask
}

/// `eroded`: a set pixel survives only when its whole 3×3 block is set.
fn eroded(mask: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = mask.to_vec();
    for y in 0..height {
        for x in 0..width {
            if mask[y * width + x] == 0 {
                continue;
            }
            let mut keep = true;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    if mask[ny * width + nx] == 0 {
                        keep = false;
                    }
                }
            }
            result[y * width + x] = if keep { 255 } else { 0 };
        }
    }
    result
}

/// `dilated`: an empty pixel fills when any neighbor in its 3×3 block is set.
fn dilated(mask: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = mask.to_vec();
    for y in 0..height {
        for x in 0..width {
            if mask[y * width + x] != 0 {
                continue;
            }
            let mut fill = false;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    if mask[ny * width + nx] != 0 {
                        fill = true;
                    }
                }
            }
            if fill {
                result[y * width + x] = 255;
            }
        }
    }
    result
}

/// `MagicWand.outline(of:width:height:)`: the outline of a mask's nonzero
/// pixels along exact pixel edges (winding rule); `None` when empty.
pub fn outline_of_mask(mask: &[u8], width: usize, height: usize) -> Option<SelectionPath> {
    if width == 0 || height == 0 || mask.len() != width * height {
        return None;
    }
    let (points, loops) = wand_trace(mask, width, height).ok()?;
    if loops.is_empty() {
        return None;
    }
    let mut path = SelectionPath::default();
    let mut index = 0usize;
    for length in loops {
        let len = length as usize;
        path.rings.push(
            points[index..index + len]
                .iter()
                .map(|(x, y)| CGPoint::new(*x as f64, *y as f64))
                .collect(),
        );
        index += len;
    }
    Some(path)
}

/// `smoothed`: rounds off the one-pixel stair steps a binary trace leaves.
/// Each ring is simplified (Ramer–Douglas–Peucker from a stable extreme,
/// tolerance 1.6) and then Chaikin-smoothed 3 rounds; the winding and
/// subpath order are preserved, so holes continue to subtract from the
/// selected region.
pub fn smoothed(path: &SelectionPath) -> SelectionPath {
    let mut result = SelectionPath::default();
    for subpath in &path.rings {
        if subpath.len() < 3 {
            continue;
        }
        let simplified = simplify_closed(subpath, SIMPLIFY_TOLERANCE);
        let points = chaikin(&simplified, CHAIKIN_ITERATIONS);
        if points.is_empty() {
            continue;
        }
        result.rings.push(points);
    }
    result
}

/// `simplifyClosed`: Ramer–Douglas–Peucker on a closed ring. The ring is
/// rotated to start at its leftmost (then topmost) vertex so the open-line
/// simplifier cannot eat the whole contour. Fewer than 4 points pass
/// through untouched; a collapse back below 3 points restores the input.
fn simplify_closed(input: &[CGPoint], tolerance: f64) -> Vec<CGPoint> {
    let mut points = input.to_vec();
    if points.len() >= 2 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 4 {
        return points;
    }
    let start = (0..points.len())
        .min_by(|&lhs, &rhs| {
            points[lhs]
                .x
                .total_cmp(&points[rhs].x)
                .then(points[lhs].y.total_cmp(&points[rhs].y))
        })
        .unwrap_or(0);
    let rotated: Vec<CGPoint> = points[start..]
        .iter()
        .chain(points[..start].iter())
        .copied()
        .collect();
    let mut open = rotated.clone();
    open.push(rotated[0]);
    let mut open = simplify_open(&open, 0, open.len() - 1, tolerance);
    if open.len() >= 2 && open.first() == open.last() {
        open.pop();
    }
    if open.len() >= 3 { open } else { points }
}

/// `simplifyOpen`: Ramer–Douglas–Peucker between two fixed endpoints — split
/// at the vertex farthest from the chord while that distance exceeds the
/// tolerance, else keep only the endpoints.
fn simplify_open(points: &[CGPoint], first: usize, last: usize, tolerance: f64) -> Vec<CGPoint> {
    if last <= first + 1 {
        return vec![points[first], points[last]];
    }
    let mut farthest = first + 1;
    let mut greatest_distance = 0.0f64;
    for index in first + 1..last {
        let distance = perpendicular_distance(&points[index], &points[first], &points[last]);
        if distance > greatest_distance {
            greatest_distance = distance;
            farthest = index;
        }
    }
    if greatest_distance <= tolerance {
        return vec![points[first], points[last]];
    }
    let mut left = simplify_open(points, first, farthest, tolerance);
    let right = simplify_open(points, farthest, last, tolerance);
    left.pop();
    left.extend(right);
    left
}

/// `perpendicularDistance`: distance from `p` to the line through a–b.
fn perpendicular_distance(p: &CGPoint, a: &CGPoint, b: &CGPoint) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length = dx.hypot(dy);
    if length <= 0.0 {
        return (p.x - a.x).hypot(p.y - a.y);
    }
    ((dy * p.x - dx * p.y + b.x * a.y - b.y * a.x) / length).abs()
}

/// `chaikin`: corner cutting with the 0.25 / 0.75 ratios, `iterations`
/// rounds — each round doubles the vertex count.
pub fn chaikin(input: &[CGPoint], iterations: usize) -> Vec<CGPoint> {
    let mut points = input.to_vec();
    if points.len() >= 2 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return points;
    }
    for _ in 0..iterations {
        let mut next: Vec<CGPoint> = Vec::with_capacity(points.len() * 2);
        for index in 0..points.len() {
            let a = points[index];
            let b = points[(index + 1) % points.len()];
            next.push(CGPoint::new(
                a.x * 0.75 + b.x * 0.25,
                a.y * 0.75 + b.y * 0.25,
            ));
            next.push(CGPoint::new(
                a.x * 0.25 + b.x * 0.75,
                a.y * 0.25 + b.y * 0.75,
            ));
        }
        points = next;
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::CGRect;
    use crate::selection::rasterize;

    /// A 32×32 guide: white field, black square 8..24, mid-gray noise
    /// specks off the square; a matching 8×8 coarse mask marks the square's
    /// cells (2..6) only.
    struct Scene {
        image: Bitmap,
        coarse: GrayMask,
    }

    fn scene() -> Scene {
        let mut image = Bitmap::new(32, 32);
        let mut coarse = GrayMask::new(8, 8);
        for y in 0..32 {
            for x in 0..32 {
                let in_square = (8..24).contains(&x) && (8..24).contains(&y);
                let speck = (x == 2 && y == 2) || (x == 28 && y == 4) || (x == 4 && y == 27);
                let gray = if in_square {
                    0
                } else if speck {
                    128
                } else {
                    255
                };
                image.set_pixel(x, y, [gray, gray, gray, 255]);
            }
        }
        for y in 2..6 {
            for x in 2..6 {
                coarse.data[y * 8 + x] = 255;
            }
        }
        Scene { image, coarse }
    }

    #[test]
    fn object_selection_converges_to_the_square_and_ignores_noise() {
        // The coarse mask claims only the square's cells; the guide's luma
        // snaps the refinement to the square's edges and the mid-gray noise
        // (luma 0.5, half a unit from both the object and the field) stays
        // under the 50% threshold.
        let scene = scene();
        let path = select(
            &scene.image,
            &scene.coarse,
            CGPoint::new(12.0, 12.0),
            0,
            false,
        )
        .unwrap();
        assert_eq!(path.bounds(), Some(CGRect::new(8.0, 8.0, 16.0, 16.0)));
        let covered = rasterize(&path, 32, 32, false);
        for y in 0..32 {
            for x in 0..32 {
                let expected = u8::from((8..24).contains(&x) && (8..24).contains(&y)) * 255;
                assert_eq!(
                    covered.value(x, y),
                    expected,
                    "pixel ({x},{y}) follows the guide's square"
                );
            }
        }
    }

    #[test]
    fn click_outside_the_image_or_an_empty_mask_selects_nothing() {
        let scene = scene();
        assert!(
            select(
                &scene.image,
                &scene.coarse,
                CGPoint::new(32.0, 12.0),
                0,
                false
            )
            .is_none()
        );
        assert!(
            select(
                &scene.image,
                &scene.coarse,
                CGPoint::new(-1.0, 12.0),
                0,
                false
            )
            .is_none()
        );
        assert!(
            select(
                &scene.image,
                &scene.coarse,
                CGPoint::new(f64::NAN, 2.0),
                0,
                false
            )
            .is_none()
        );
        let empty = GrayMask::new(8, 8);
        assert!(select(&scene.image, &empty, CGPoint::new(12.0, 12.0), 0, false).is_none());
    }

    #[test]
    fn edge_offset_erodes_and_dilates_by_whole_pixels() {
        let scene = scene();
        let grown = select(
            &scene.image,
            &scene.coarse,
            CGPoint::new(12.0, 12.0),
            -2,
            false,
        )
        .unwrap();
        assert_eq!(
            grown.bounds(),
            Some(CGRect::new(6.0, 6.0, 20.0, 20.0)),
            "dilation expands 2 out"
        );
        let shrunk = select(
            &scene.image,
            &scene.coarse,
            CGPoint::new(12.0, 12.0),
            2,
            false,
        )
        .unwrap();
        assert_eq!(
            shrunk.bounds(),
            Some(CGRect::new(10.0, 10.0, 12.0, 12.0)),
            "erosion pulls 2 in"
        );
        let slight = select(
            &scene.image,
            &scene.coarse,
            CGPoint::new(12.0, 12.0),
            7,
            false,
        )
        .unwrap();
        assert_eq!(
            slight.bounds(),
            Some(CGRect::new(15.0, 15.0, 2.0, 2.0)),
            "7 erosions leave a 2×2 core"
        );
        // Eight 1-pixel erosions erase the 16-pixel square; a larger offset
        // clamps to the same 10-step cap and cannot bring anything back.
        assert!(
            select(
                &scene.image,
                &scene.coarse,
                CGPoint::new(12.0, 12.0),
                8,
                false
            )
            .is_none()
        );
        assert!(
            select(
                &scene.image,
                &scene.coarse,
                CGPoint::new(12.0, 12.0),
                11,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn instance_index_picks_the_clicked_instance() {
        // A 4×4 instance mask over an 8×8 image: cells store instance ids.
        let mut mask = GrayMask::new(4, 4);
        mask.data[1 * 4 + 2] = 3;
        let index = instance_index(&mask, CGPoint::new(4.0, 3.0), CGSize::new(8.0, 8.0));
        assert_eq!(index, Some(3));
        assert_eq!(
            instance_index(&mask, CGPoint::new(1.0, 1.0), CGSize::new(8.0, 8.0)),
            None,
            "background"
        );
        // Off-image clicks clamp into the mask (the top-right cell is 0).
        assert_eq!(
            instance_index(&mask, CGPoint::new(99.0, 3.0), CGSize::new(8.0, 8.0)),
            None
        );
        assert_eq!(
            instance_index(
                &GrayMask::new(0, 0),
                CGPoint::new(1.0, 1.0),
                CGSize::new(8.0, 8.0)
            ),
            None
        );
    }

    #[test]
    fn chaikin_doubles_vertices_each_round_and_keeps_the_cut_ratios() {
        let square = vec![
            CGPoint::new(0.0, 0.0),
            CGPoint::new(8.0, 0.0),
            CGPoint::new(8.0, 8.0),
            CGPoint::new(0.0, 8.0),
        ];
        let once = chaikin(&square, 1);
        assert_eq!(once.len(), 8, "one round doubles the vertices");
        // The two cuts of edge a→b land at 0.75a + 0.25b and 0.25a + 0.75b.
        assert_eq!(once[0], CGPoint::new(2.0, 0.0));
        assert_eq!(once[1], CGPoint::new(6.0, 0.0));
        let thrice = chaikin(&square, 3);
        assert_eq!(
            thrice.len(),
            32,
            "three rounds double the vertices three times"
        );
    }

    #[test]
    fn simplification_drops_jaggies_within_the_tolerance_and_keeps_real_corners() {
        // A shallow bump 0.5 off the chord: under tolerance 1.6 it collapses
        // to the endpoints; under 0.4 the bump survives.
        let bump = vec![
            CGPoint::new(0.0, 0.0),
            CGPoint::new(4.0, 0.5),
            CGPoint::new(8.0, 0.0),
            CGPoint::new(8.0, 8.0),
            CGPoint::new(0.0, 8.0),
        ];
        let loose = simplify_closed(&bump, 1.6);
        assert_eq!(
            loose,
            vec![
                CGPoint::new(0.0, 0.0),
                CGPoint::new(8.0, 0.0),
                CGPoint::new(8.0, 8.0),
                CGPoint::new(0.0, 8.0)
            ]
        );
        let tight = simplify_closed(&bump, 0.4);
        assert!(
            tight.contains(&CGPoint::new(4.0, 0.5)),
            "a corner beyond the tolerance stays"
        );
        // Fewer than 4 points pass through untouched.
        let tiny = vec![
            CGPoint::new(0.0, 0.0),
            CGPoint::new(5.0, 5.0),
            CGPoint::new(9.0, 1.0),
        ];
        assert_eq!(simplify_closed(&tiny, 1.6), tiny);
    }

    #[test]
    fn smoothing_keeps_holes_subtracting() {
        let mut image = Bitmap::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                // Black square 8..24 with a white hole 16..20 punched in it.
                let hole = (16..20).contains(&x) && (16..20).contains(&y);
                let gray = if (8..24).contains(&x) && (8..24).contains(&y) && !hole {
                    0
                } else {
                    255
                };
                image.set_pixel(x, y, [gray, gray, gray, 255]);
            }
        }
        // The coarse mask claims the square's cells, minus the hole's cell.
        let mut coarse = GrayMask::new(8, 8);
        for y in 2..6 {
            for x in 2..6 {
                coarse.data[y * 8 + x] = 255;
            }
        }
        coarse.data[4 * 8 + 4] = 0;
        let path = select(&image, &coarse, CGPoint::new(16.0, 16.0), 0, true).unwrap();
        assert!(
            path.contains(CGPoint::new(10.0, 10.0)),
            "the object's interior survives smoothing"
        );
        assert!(
            !path.contains(CGPoint::new(18.0, 18.0)),
            "the hole keeps subtracting"
        );
        // Smoothing bends the stair steps: the outline no longer sits on
        // whole-pixel corners.
        let ring = &path.rings[0];
        assert!(
            ring.iter()
                .any(|p| p.x.fract() != 0.0 || p.y.fract() != 0.0),
            "Chaikin cuts the corners"
        );
    }
}
