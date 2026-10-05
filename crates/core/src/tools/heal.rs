//! Port of `Rendering/HealPixels.c`: the Spot Healing Brush pixel engine —
//! coverage bounds, a membrane solve over the spot anchored to its ring, and
//! the Content-Aware / Proximity Match / Create Texture patch sources.

#![allow(dead_code)]

use crate::geometry::CGRect;
use crate::raster::{Bitmap, GrayMask};

const OUTSIDE: u8 = 0;
const RING: u8 = 1;
const HOLE: u8 = 2;

#[inline]
fn heal_hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

#[inline]
fn heal_unit(key: u32) -> f64 {
    (heal_hash(key) >> 8) as f64 / 16777216.0
}

/// Mean squared difference between the ring around the spot and the ring
/// around the patch offset by (dx, dy). Infinite when the patch would overlap
/// the spot or leave the image.
fn heal_score(
    rgba: &Bitmap,
    role: &[u8],
    wx0: i64,
    wy0: i64,
    ww: i64,
    wh: i64,
    dx: i64,
    dy: i64,
    w: i64,
    h: i64,
) -> f64 {
    if dx.abs() < ww && dy.abs() < wh {
        return f64::INFINITY;
    }
    if wx0 + dx < 0 || wy0 + dy < 0 || wx0 + ww + dx > w || wy0 + wh + dy > h {
        return f64::INFINITY;
    }
    let mut sum = 0.0f64;
    let mut n = 0u64;
    for y in 0..wh as usize {
        for x in 0..ww as usize {
            if role[y * ww as usize + x] != RING {
                continue;
            }
            let t = rgba.pixel((wx0 as usize) + x, (wy0 as usize) + y);
            let s = rgba.pixel(((wx0 + dx) as usize) + x, ((wy0 + dy) as usize) + y);
            for c in 0..4 {
                let d = t[c] as f64 - s[c] as f64;
                sum += d * d;
            }
            n += 1;
        }
    }
    if n > 0 { sum / n as f64 } else { f64::INFINITY }
}

/// Solves for smooth values over HOLE pixels, fixed to the RING values around
/// them. A coarser copy is solved first and used as the starting point, so
/// large spots settle in few passes.
fn heal_solve(value: &mut [f32], role: &[u8], w: i64, h: i64, depth: i32) {
    let mut iterations = 300i64;
    let (w, h) = (w as usize, h as usize);
    if w > 32 && h > 32 && depth < 16 {
        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let mut coarse = vec![0.0f32; cw * ch * 4];
        let mut coarse_role = vec![OUTSIDE; cw * ch];
        for y in 0..ch {
            for x in 0..cw {
                let mut known = 0usize;
                let mut hole = 0usize;
                let mut known_sum = [0.0f32; 4];
                let mut hole_sum = [0.0f32; 4];
                for j in 0..2usize {
                    for i in 0..2usize {
                        let fx = x * 2 + i;
                        let fy = y * 2 + j;
                        if fx >= w || fy >= h {
                            continue;
                        }
                        let p = fy * w + fx;
                        if role[p] == RING {
                            known += 1;
                            for c in 0..4 {
                                known_sum[c] += value[p * 4 + c];
                            }
                        } else if role[p] == HOLE {
                            hole += 1;
                            for c in 0..4 {
                                hole_sum[c] += value[p * 4 + c];
                            }
                        }
                    }
                }
                let q = y * cw + x;
                if known > 0 {
                    coarse_role[q] = RING;
                    for c in 0..4 {
                        coarse[q * 4 + c] = known_sum[c] / known as f32;
                    }
                } else if hole > 0 {
                    coarse_role[q] = HOLE;
                    for c in 0..4 {
                        coarse[q * 4 + c] = hole_sum[c] / hole as f32;
                    }
                }
            }
        }
        heal_solve(&mut coarse, &coarse_role, cw as i64, ch as i64, depth + 1);
        for y in 0..h {
            for x in 0..w {
                let p = y * w + x;
                let q = (y / 2) * cw + x / 2;
                if role[p] == HOLE && coarse_role[q] == HOLE {
                    value[p * 4..p * 4 + 4].copy_from_slice(&coarse[q * 4..q * 4 + 4]);
                }
            }
        }
        iterations = 40;
    }
    const OMEGA: f32 = 1.8;
    for _ in 0..iterations {
        for y in 0..h {
            for x in 0..w {
                let p = y * w + x;
                if role[p] != HOLE {
                    continue;
                }
                let mut sum = [0.0f32; 4];
                let mut n = 0usize;
                let neighbors: [(i64, i64); 4] = [
                    (x as i64 - 1, y as i64),
                    (x as i64 + 1, y as i64),
                    (x as i64, y as i64 - 1),
                    (x as i64, y as i64 + 1),
                ];
                for (nx, ny) in neighbors {
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let q = (ny as usize) * w + nx as usize;
                    if role[q] == OUTSIDE {
                        continue;
                    }
                    for c in 0..4 {
                        sum[c] += value[q * 4 + c];
                    }
                    n += 1;
                }
                if n == 0 {
                    continue;
                }
                for c in 0..4 {
                    value[p * 4 + c] += OMEGA * (sum[c] / n as f32 - value[p * 4 + c]);
                }
            }
        }
    }
}

/// `spot_heal` — heals the covered (nonzero coverage) area of `rgba` in place.
/// `mode`: 0 = Content-Aware, 1 = Create Texture, 2 = Proximity Match.
/// Returns false when there is nothing to heal (no coverage or no ring).
pub fn spot_heal(
    rgba: &mut Bitmap,
    coverage: &GrayMask,
    opacity: f32,
    mode: i32,
    seed: u32,
) -> bool {
    let w = rgba.width as i64;
    spot_heal_offset(rgba, coverage, opacity, mode, seed, 0, 0, w)
}

/// The kernel's grain keys hash `(iy * w + ix)` in the sub image's own
/// coordinates. A caller that crops a smaller work region out of a larger
/// heal region (BrushStroke.heal) can keep the keys — and therefore the
/// generated grain — identical to the full-region call by passing the
/// original region's coordinates: `iy + key_dy` and `ix + key_dx` must equal
/// the pixel's position in the full region, and `key_w` its width.
pub(crate) fn spot_heal_offset(
    rgba: &mut Bitmap,
    coverage: &GrayMask,
    opacity: f32,
    mode: i32,
    seed: u32,
    key_dx: i64,
    key_dy: i64,
    key_w: i64,
) -> bool {
    let w = rgba.width as i64;
    let h = rgba.height as i64;
    let bounds = match crate::raster::coverage_bounds(coverage) {
        Some(b) => b,
        None => return false,
    };
    let bx0 = bounds.min_x() as i64;
    let by0 = bounds.min_y() as i64;
    let bx1 = bounds.max_x() as i64;
    let by1 = bounds.max_y() as i64;
    if bx1 <= bx0 {
        return false;
    }
    let bw = bx1 - bx0;
    let bh = by1 - by0;
    let size = bw.max(bh);
    let mut ring = size / 8;
    ring = ring.clamp(2, 16);
    // Work box: the spot plus its ring, clipped to the image.
    let wx0 = (bx0 - ring).max(0);
    let wy0 = (by0 - ring).max(0);
    let wx1 = (bx1 + ring).min(w);
    let wy1 = (by1 + ring).min(h);
    let ww = wx1 - wx0;
    let wh = wy1 - wy0;
    let wn = (ww * wh) as usize;

    let mut role = vec![OUTSIDE; wn];
    let mut near = vec![false; wn];
    let mut prefix = vec![0i64; ww.max(wh) as usize + 1];

    for y in 0..wh {
        for x in 0..ww {
            let cov = coverage.value((wx0 + x) as usize, (wy0 + y) as usize);
            role[(y * ww + x) as usize] = if cov != 0 { HOLE } else { OUTSIDE };
        }
    }
    // The ring: pixels within `ring` of the spot (a square dilation, row pass
    // then column pass).
    for y in 0..wh {
        let y = y as usize;
        prefix[0] = 0;
        for x in 0..ww {
            let x = x as usize;
            prefix[x + 1] = prefix[x]
                + if role[y * ww as usize + x] == HOLE {
                    1i64
                } else {
                    0i64
                };
        }
        for x in 0..ww {
            let x = x as usize;
            let lo = ((x as i64) - ring).max(0) as usize;
            let hi = ((x as i64) + ring + 1).min(ww) as usize;
            near[y * ww as usize + x] = prefix[hi] - prefix[lo] > 0;
        }
    }
    for x in 0..ww {
        let x = x as usize;
        prefix[0] = 0;
        for y in 0..wh {
            let y = y as usize;
            prefix[y + 1] = prefix[y]
                + if near[y * ww as usize + x] {
                    1i64
                } else {
                    0i64
                };
        }
        for y in 0..wh {
            let y = y as usize;
            let lo = ((y as i64 - ring).max(0.0_f64 as i64)) as usize;
            let hi = ((y as i64 + ring + 1).min(wh)) as usize;
            let idx = y * ww as usize + x;
            if role[idx] == OUTSIDE && prefix[hi.min(prefix.len() - 1)] - prefix[lo] > 0 {
                role[idx] = RING;
            }
        }
    }
    let ring_count = role.iter().filter(|&&r| r == RING).count();
    if ring_count == 0 {
        return false;
    }

    // Source patch for Content-Aware and Proximity Match.
    let mut ox = 0i64;
    let mut oy = 0i64;
    let mut have_source = false;
    if mode != 1 {
        const FACTORS: [f64; 5] = [1.05, 1.35, 1.75, 2.25, 2.8];
        let count = if mode == 2 { 2 } else { 5 };
        let mut best = f64::INFINITY;
        for f in 0..count {
            for a in 0..24 {
                let angle = a as f64 * std::f64::consts::PI / 12.0;
                let dx = (angle.cos() * FACTORS[f] * ww as f64).round() as i64;
                let dy = (angle.sin() * FACTORS[f] * wh as f64).round() as i64;
                let mut score = heal_score(rgba, &role, wx0, wy0, ww, wh, dx, dy, w, h);
                if !score.is_finite() {
                    continue;
                }
                score *= if mode == 2 {
                    1.0 + 0.6 * f as f64
                } else {
                    1.0 + 0.1 * f as f64
                };
                if score < best {
                    best = score;
                    ox = dx;
                    oy = dy;
                }
            }
        }
        if best.is_finite() {
            // Fine-tune the alignment so repeating texture lines up.
            let (cx, cy) = (ox, oy);
            let mut refined = heal_score(rgba, &role, wx0, wy0, ww, wh, cx, cy, w, h);
            for j in -3..=3i64 {
                for i in -3..=3i64 {
                    let score = heal_score(rgba, &role, wx0, wy0, ww, wh, cx + i, cy + j, w, h);
                    if score < refined {
                        refined = score;
                        ox = cx + i;
                        oy = cy + j;
                    }
                }
            }
            have_source = true;
        }
    }

    // Membrane: the edge difference between the original and the patch (or the
    // original itself for a smooth fill), spread across the spot.
    let mut mean = [0.0f64; 4];
    let mut detail = [0.0f64; 3];
    let mut value = vec![0.0f32; wn * 4];
    for y in 0..wh {
        for x in 0..ww {
            let p = (y * ww + x) as usize;
            if role[p] != RING {
                value[p * 4..p * 4 + 4].fill(0.0);
                continue;
            }
            let ix = (wx0 + x) as usize;
            let iy = (wy0 + y) as usize;
            let t = rgba.pixel(ix, iy);
            let s = if have_source {
                Some(rgba.pixel((ix as i64 + ox) as usize, (iy as i64 + oy) as usize))
            } else {
                None
            };
            for c in 0..4 {
                value[p * 4 + c] = t[c] as f32 - s.map(|s| s[c]).unwrap_or(0) as f32;
                mean[c] += value[p * 4 + c] as f64;
            }
            if !have_source {
                // Fine detail around the spot: each pixel against the average
                // of its neighbours.
                for c in 0..3 {
                    let mut around = 0.0f64;
                    let mut n = 0i64;
                    let offsets: [(i64, i64); 4] = [
                        (ix as i64 - 1, iy as i64),
                        (ix as i64 + 1, iy as i64),
                        (ix as i64, iy as i64 - 1),
                        (ix as i64, iy as i64 + 1),
                    ];
                    for (ox2, oy2) in offsets {
                        if ox2 < 0 || oy2 < 0 || ox2 >= w || oy2 >= h {
                            continue;
                        }
                        around += rgba.pixel(ox2 as usize, oy2 as usize)[c] as f64;
                        n += 1;
                    }
                    if n > 0 {
                        let d = t[c] as f64 - around / n as f64;
                        detail[c] += d * d;
                    }
                }
            }
        }
    }
    for c in 0..4 {
        mean[c] /= ring_count as f64;
    }
    for p in 0..wn {
        if role[p] == HOLE {
            for c in 0..4 {
                value[p * 4 + c] = mean[c] as f32;
            }
        }
    }
    heal_solve(&mut value, &role, ww, wh, 0);
    for c in 0..3 {
        detail[c] = (detail[c] / ring_count as f64).sqrt() * 0.9;
    }

    for y in 0..wh {
        for x in 0..ww {
            let p = (y * ww + x) as usize;
            if role[p] != HOLE {
                continue;
            }
            let ix = (wx0 + x) as usize;
            let iy = (wy0 + y) as usize;
            let t = rgba.pixel(ix, iy);
            let s = if have_source {
                Some(rgba.pixel((ix as i64 + ox) as usize, (iy as i64 + oy) as usize))
            } else {
                None
            };
            let amount = coverage.value(ix, iy) as f64 / 255.0 * opacity as f64;
            let mut grain = 0.0f64;
            if !have_source {
                let key = heal_hash(
                    seed ^ heal_hash(((iy as i64 + key_dy) * key_w + ix as i64 + key_dx) as u32),
                );
                let u1 = heal_unit(key);
                let u2 = heal_unit(key ^ 0x68e31da4);
                grain = (-2.0 * (1.0 - u1).ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            }
            let mut out = [0.0f64; 4];
            for c in 0..4 {
                let healed = s.map(|s| s[c] as f64).unwrap_or(0.0)
                    + value[p * 4 + c] as f64
                    + if c < 3 { grain * detail[c] } else { 0.0 };
                out[c] = t[c] as f64 + (healed - t[c] as f64) * amount;
            }
            let alpha = out[3].clamp(0.0, 255.0);
            let a_out = alpha.round() as u8;
            let mut px = [0u8; 4];
            px[3] = a_out;
            for c in 0..3 {
                let v = out[c].clamp(0.0, a_out as f64);
                px[c] = v.round() as u8;
            }
            rgba.set_pixel(ix, iy, px);
        }
    }
    true
}

/// The trimmed work region for a heal: the full `region` a BrushStroke.heal
/// would copy is the painted bounds grown by `(max side + 32) * 3.2`, but the
/// kernel only ever reads the work box plus patch sources within
/// `2.8 × work box + 3` of it. `rect` (absolute canvas coordinates, integral,
/// always within `region`) is the smallest crop that still contains every
/// pixel the kernel can read while keeping patch validity — "the source box
/// lies inside the sub image" — identical to the full region, and the
/// `key_*` fields reproduce the full region's grain-key coordinates.
pub(crate) struct TrimmedHeal {
    pub rect: CGRect,
    pub key_dx: i64,
    pub key_dy: i64,
    pub key_w: i64,
}

/// Computes the trimmed region, replicating `spot_heal`'s work-box derivation
/// from the painted bounds. None when anything is degenerate (the caller then
/// copies the full region).
pub(crate) fn trim_heal_region(painted: CGRect, region: CGRect) -> Option<TrimmedHeal> {
    let rw = region.width() as i64;
    let rh = region.height() as i64;
    if rw <= 0 || rh <= 0 {
        return None;
    }
    // Painted bounds relative to the region, exactly as `spot_heal` derives
    // them from the cropped coverage mask (max side is the exclusive bound).
    let bx0 = painted.min_x() as i64 - region.min_x() as i64;
    let by0 = painted.min_y() as i64 - region.min_y() as i64;
    let bx1 = bx0 + painted.width() as i64;
    let by1 = by0 + painted.height() as i64;
    if bx1 <= bx0 || by1 <= by0 {
        return None;
    }
    let ring = ((bx1 - bx0).max(by1 - by0) / 8).clamp(2, 16);
    let wx0 = (bx0 - ring).max(0);
    let wy0 = (by0 - ring).max(0);
    let wx1 = (bx1 + ring).min(rw);
    let wy1 = (by1 + ring).min(rh);
    let ww = wx1 - wx0;
    let wh = wy1 - wy0;
    if ww <= 0 || wh <= 0 {
        return None;
    }
    // Patch offsets: |dx| ≤ round(2.8·ww) + 3, |dy| ≤ round(2.8·wh) + 3; the
    // padding covers that extent with slack. Grain/detail reads reach one
    // pixel past the work box, well inside the padding.
    let pad_x = (2.8f64 * ww as f64 + 4.0).ceil() as i64;
    let pad_y = (2.8f64 * wh as f64 + 4.0).ceil() as i64;
    let rx = region.min_x() as i64;
    let ry = region.min_y() as i64;
    let nx0 = (rx + wx0 - pad_x).max(rx);
    let ny0 = (ry + wy0 - pad_y).max(ry);
    let nx1 = (rx + wx1 + pad_x).min(rx + rw);
    let ny1 = (ry + wy1 + pad_y).min(ry + rh);
    if nx0 >= nx1 || ny0 >= ny1 {
        return None;
    }
    Some(TrimmedHeal {
        rect: CGRect::new(
            nx0 as f64,
            ny0 as f64,
            (nx1 - nx0) as f64,
            (ny1 - ny0) as f64,
        ),
        key_dx: nx0 - rx,
        key_dy: ny0 - ry,
        key_w: rw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heals_a_spot_to_match_its_surroundings() {
        // A red field with a soft dark spot in the middle.
        let mut img = Bitmap::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                img.set_pixel(x, y, [220, 30, 30, 255]);
            }
        }
        let mut cov = GrayMask::new(16, 16);
        for y in 6..10 {
            for x in 6..10 {
                cov.data[y * 16 + x] = 255;
                img.set_pixel(x, y, [40, 20, 20, 255]);
            }
        }
        assert!(spot_heal(&mut img, &cov, 1.0, 0, 42));
        let healed = img.pixel(7, 7);
        assert!(healed[0] > 180, "{healed:?}");
        // Untouched pixels stay put.
        assert_eq!(img.pixel(0, 0), [220, 30, 30, 255]);
    }

    #[test]
    fn no_coverage_is_noop() {
        let mut img = Bitmap::new(4, 4);
        let cov = GrayMask::new(4, 4);
        assert!(!spot_heal(&mut img, &cov, 1.0, 0, 1));
    }

    #[test]
    fn opacity_blends_partially() {
        let mut img = Bitmap::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                img.set_pixel(x, y, [220, 30, 30, 255]);
            }
        }
        let mut cov = GrayMask::new(16, 16);
        for y in 6..10 {
            for x in 6..10 {
                cov.data[y * 16 + x] = 255;
                img.set_pixel(x, y, [40, 20, 20, 255]);
            }
        }
        let mut half = img.clone();
        assert!(spot_heal(&mut half, &cov, 0.5, 0, 42));
        spot_heal(&mut img, &cov, 1.0, 0, 42);
        let h = half.pixel(7, 7)[0] as f64;
        let f = img.pixel(7, 7)[0] as f64;
        let o = 40.0;
        assert!((h - (o + (f - o) * 0.5)).abs() < 3.0, "{h} vs {f}");
    }

    #[test]
    fn smooth_fill_adds_grain_for_texture_mode() {
        // Create Texture (mode 1) with no source patch synthesizes grain.
        let mut img = Bitmap::new(40, 40);
        for y in 0..40 {
            for x in 0..40 {
                img.set_pixel(x, y, [128, 128, 128, 255]);
            }
        }
        let mut cov = GrayMask::new(40, 40);
        for y in 16..24 {
            for x in 16..24 {
                cov.data[y * 40 + x] = 255;
                img.set_pixel(x, y, [64, 64, 64, 255]);
            }
        }
        let mut a = img.clone();
        let mut b = img.clone();
        assert!(spot_heal(&mut a, &cov, 1.0, 1, 1));
        assert!(spot_heal(&mut b, &cov, 1.0, 1, 2));
        // Different seeds → different grain.
        assert_ne!(a.pixel(18, 18), b.pixel(18, 18));
        assert!(a.pixel(18, 18)[0] > 64);
    }

    #[test]
    fn ring_reaches_both_sides_of_tall_boxes() {
        // HealPixels.c clamps the row pass of the ring dilation to the box
        // width; clamping it to the height instead thinned the ring along the
        // right edge of work boxes taller than wide, so the spot's right
        // columns healed from the left surroundings only.
        let mut img = Bitmap::new(40, 48);
        for y in 0..48 {
            for x in 0..40 {
                img.set_pixel(x, y, [200, 40, 40, 255]);
            }
        }
        // A bright bar just right of the spot: inside its work box, exactly
        // where the right-hand ring anchors belong.
        for y in 12..36 {
            for x in 22..24 {
                img.set_pixel(x, y, [250, 60, 60, 255]);
            }
        }
        let mut cov = GrayMask::new(40, 48);
        // A 4×20 spot → work box 8×24 (wh > ww), ring 2.
        for y in 14..34 {
            for x in 18..22 {
                cov.data[y * 40 + x] = 255;
                img.set_pixel(x, y, [40, 10, 10, 255]);
            }
        }
        assert!(spot_heal(&mut img, &cov, 1.0, 1, 7));
        let left = img.pixel(18, 24)[0] as i32;
        let right = img.pixel(21, 24)[0] as i32;
        assert!(right > left + 15, "left={left} right={right}");
        assert!(
            right > 215,
            "right column must pick up its own surround, got {right}"
        );
    }

    #[test]
    fn single_row_and_single_pixel_spots_heal() {
        // HealPixels.c accepts one-pixel-wide or tall coverage; rejecting it
        // made the spot-heal brush silently do nothing on 1px strokes.
        let mut img = Bitmap::new(24, 24);
        for y in 0..24 {
            for x in 0..24 {
                img.set_pixel(x, y, [200, 40, 40, 255]);
            }
        }
        let mut cov = GrayMask::new(24, 24);
        for x in 8..14 {
            cov.data[12 * 24 + x] = 255;
            img.set_pixel(x, 12, [30, 10, 10, 255]);
        }
        assert!(
            spot_heal(&mut img, &cov, 1.0, 1, 3),
            "a single-row spot must heal"
        );
        assert!(img.pixel(10, 12)[0] > 150, "{:?}", img.pixel(10, 12));

        let mut img = Bitmap::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                img.set_pixel(x, y, [200, 40, 40, 255]);
            }
        }
        let mut cov = GrayMask::new(16, 16);
        cov.data[8 * 16 + 8] = 255;
        img.set_pixel(8, 8, [30, 10, 10, 255]);
        assert!(
            spot_heal(&mut img, &cov, 1.0, 1, 3),
            "a single-pixel spot must heal"
        );
        assert!(img.pixel(8, 8)[0] > 150, "{:?}", img.pixel(8, 8));
    }
}
