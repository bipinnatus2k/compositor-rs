//! Port of `Rendering/WandPixels.c`: the Magic Wand — color matching with a
//! seed-radius reference, scanline flood fill, and outline tracing.

#![allow(dead_code)]

use crate::raster::Bitmap;

/// Outlines with more pixel edges than this are refused: the path would be
/// too slow to draw.
pub const WAND_EDGE_LIMIT: usize = 8_000_000;

#[inline]
fn wand_matches(p: &[u8; 4], reference: &[i32; 4], tolerance: i32) -> bool {
    for c in 0..4 {
        let d = p[c] as i32 - reference[c];
        if d < -tolerance || d > tolerance {
            return false;
        }
    }
    true
}

/// `wand_mask`: fills `mask` with 255 where pixels match. Returns the matched
/// count, or None when the seed is out of bounds. `radius` averages a square
/// around the seed into the reference color; `contiguous` flood-fills instead
/// of matching everywhere.
pub fn wand_mask(
    rgba: &Bitmap,
    seed: (usize, usize),
    radius: usize,
    tolerance: i32,
    contiguous: bool,
    mask: &mut [u8],
) -> Option<i64> {
    let width = rgba.width;
    let height = rgba.height;
    if width == 0 || height == 0 {
        return Some(0);
    }
    mask[..width * height].fill(0);
    let (seed_x, seed_y) = seed;
    if seed_x >= width || seed_y >= height {
        return None;
    }
    let x0 = seed_x.saturating_sub(radius);
    let x1 = (seed_x + radius).min(width - 1);
    let y0 = seed_y.saturating_sub(radius);
    let y1 = (seed_y + radius).min(height - 1);
    let mut sums = [0u64; 4];
    let mut samples = 0u64;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let px = rgba.pixel(x, y);
            for c in 0..4 {
                sums[c] += px[c] as u64;
            }
            samples += 1;
        }
    }
    let mut reference = [0i32; 4];
    for c in 0..4 {
        reference[c] = ((sums[c] + samples / 2) / samples) as i32;
    }

    let mut count = 0i64;
    if !contiguous {
        for y in 0..height {
            for x in 0..width {
                if wand_matches(&rgba.pixel(x, y), &reference, tolerance) {
                    mask[y * width + x] = 255;
                    count += 1;
                }
            }
        }
        return Some(count);
    }

    // Scanline flood fill: each popped seed fills its whole horizontal run,
    // then pushes one seed per matching run in the rows above and below.
    let mut stack: Vec<(usize, usize)> = Vec::with_capacity(4096);
    stack.push((seed_x, seed_y));
    while let Some((x, y)) = stack.pop() {
        if mask[y * width + x] != 0 || !wand_matches(&rgba.pixel(x, y), &reference, tolerance) {
            continue;
        }
        let mut left = x;
        let mut right = x;
        while left > 0 && mask[y * width + left - 1] == 0 && wand_matches(&rgba.pixel(left - 1, y), &reference, tolerance) {
            left -= 1;
        }
        while right + 1 < width && mask[y * width + right + 1] == 0 && wand_matches(&rgba.pixel(right + 1, y), &reference, tolerance) {
            right += 1;
        }
        for v in &mut mask[y * width + left..y * width + right + 1] {
            *v = 255;
        }
        count += (right - left + 1) as i64;
        for side in 0..2 {
            if side == 0 && y == 0 {
                continue;
            }
            if side == 1 && y + 1 >= height {
                continue;
            }
            let ny = if side == 0 { y - 1 } else { y + 1 };
            let mut in_run = false;
            for nx in left..=right {
                let candidate =
                    mask[ny * width + nx] == 0 && wand_matches(&rgba.pixel(nx, ny), &reference, tolerance);
                if candidate && !in_run {
                    stack.push((nx, ny));
                }
                in_run = candidate;
            }
        }
    }
    Some(count)
}

/// `wand_trace`: walks the boundary of a mask into loops of grid-corner
/// points. Returns (points, loop lengths) or an error kind (1 = too many
/// edges, 2 = out of memory — collapsed to None/false here).
pub fn wand_trace(mask: &[u8], width: usize, height: usize) -> Result<(Vec<(i32, i32)>, Vec<i32>), TraceError> {
    const EAST: u8 = 1;
    const SOUTH: u8 = 2;
    const WEST: u8 = 4;
    const NORTH: u8 = 8;

    #[inline]
    fn turn_right(d: u8) -> u8 {
        if d == NORTH {
            EAST
        } else {
            d << 1
        }
    }
    #[inline]
    fn turn_left(d: u8) -> u8 {
        if d == EAST {
            NORTH
        } else {
            d >> 1
        }
    }

    if width == 0 || height == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let stride = width + 1;
    let vertices = stride * (height + 1);
    let mut out = vec![0u8; vertices];
    let mut edges = 0usize;
    for y in 0..height {
        for x in 0..width {
            if mask[y * width + x] == 0 {
                continue;
            }
            let mut add = |v: usize, d: u8, out: &mut Vec<u8>, edges: &mut usize| {
                if out[v] & d == 0 {
                    out[v] |= d;
                    *edges += 1;
                }
            };
            if y == 0 || mask[(y - 1) * width + x] == 0 {
                add(y * stride + x, EAST, &mut out, &mut edges);
            }
            if x + 1 == width || mask[y * width + x + 1] == 0 {
                add(y * stride + x + 1, SOUTH, &mut out, &mut edges);
            }
            if y + 1 == height || mask[(y + 1) * width + x] == 0 {
                add((y + 1) * stride + x + 1, WEST, &mut out, &mut edges);
            }
            if x == 0 || mask[y * width + x - 1] == 0 {
                add((y + 1) * stride + x, NORTH, &mut out, &mut edges);
            }
        }
        if edges > WAND_EDGE_LIMIT {
            return Err(TraceError::TooManyEdges);
        }
    }

    let mut points: Vec<(i32, i32)> = Vec::with_capacity(1024);
    let mut loops: Vec<i32> = Vec::with_capacity(256);
    for start in 0..vertices {
        while out[start] != 0 {
            let first = points.len();
            let mut v = start;
            let mut heading: u8 = 0;
            let mut initial: u8 = 0;
            loop {
                let bits = out[v];
                // Where two loops meet at a corner, turning right keeps them apart.
                let d: u8 = if heading == 0 {
                    bits & bits.wrapping_neg()
                } else if bits & turn_right(heading) != 0 {
                    turn_right(heading)
                } else if bits & heading != 0 {
                    heading
                } else if bits & turn_left(heading) != 0 {
                    turn_left(heading)
                } else {
                    bits & bits.wrapping_neg()
                };
                if d == 0 {
                    break;
                }
                out[v] &= !d;
                if d != heading {
                    points.push(((v % stride) as i32, (v / stride) as i32));
                }
                if heading == 0 {
                    initial = d;
                }
                heading = d;
                v = match d {
                    EAST => v + 1,
                    WEST => v - 1,
                    SOUTH => v + stride,
                    _ => v - stride,
                };
                if v == start {
                    break;
                }
            }
            // The start is a corner unless the loop arrives on the heading it left with.
            if heading == initial && points.len() > first {
                points.remove(first);
            }
            loops.push((points.len() - first) as i32);
        }
    }
    Ok((points, loops))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceError {
    TooManyEdges,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(color: [u8; 4]) -> Bitmap {
        let mut b = Bitmap::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                b.set_pixel(x, y, color);
            }
        }
        b
    }

    #[test]
    fn wand_matches_similar_colors() {
        let img = solid([100, 100, 100, 255]);
        let mut mask = vec![0u8; 64];
        let count = wand_mask(&img, (4, 4), 1, 10, true, &mut mask).unwrap();
        assert_eq!(count, 64);
        assert!(mask.iter().all(|v| *v == 255));
    }

    #[test]
    fn wand_respects_tolerance() {
        let mut img = solid([100, 100, 100, 255]);
        img.set_pixel(2, 2, [200, 200, 200, 255]);
        let mut mask = vec![0u8; 64];
        let count = wand_mask(&img, (4, 4), 1, 10, true, &mut mask).unwrap();
        assert_eq!(count, 63);
        assert_eq!(mask[2 * 8 + 2], 0);
        // Non-contiguous still skips the outlier (out of tolerance).
        let mut mask2 = vec![0u8; 64];
        let count2 = wand_mask(&img, (0, 0), 1, 10, false, &mut mask2).unwrap();
        assert_eq!(count2, 63);
    }

    #[test]
    fn wand_seed_radius_averages() {
        let mut img = solid([0, 0, 0, 255]);
        img.set_pixel(4, 4, [200, 200, 200, 255]);
        // With radius 1 seeded on the field the reference averages to ~22, so
        // tolerance 30 covers the black field but the flood stops at the
        // bright pixel (its value misses the reference by 178). Seeded ON the
        // bright pixel, the seed itself misses: nothing matches.
        let mut mask = vec![0u8; 64];
        let count = wand_mask(&img, (0, 0), 1, 30, true, &mut mask).unwrap();
        assert_eq!(count, 63);
        let mut mask_bright = vec![0u8; 64];
        let count_bright = wand_mask(&img, (4, 4), 1, 30, true, &mut mask_bright).unwrap();
        assert_eq!(count_bright, 0);
        // Radius 0 samples the seed exactly: only the bright pixel's run matches.
        let mut mask2 = vec![0u8; 64];
        let count2 = wand_mask(&img, (4, 4), 0, 5, false, &mut mask2).unwrap();
        assert_eq!(count2, 1);
    }

    #[test]
    fn wand_out_of_bounds_is_none() {
        let img = solid([0, 0, 0, 255]);
        let mut mask = vec![0u8; 64];
        assert!(wand_mask(&img, (9, 4), 1, 10, true, &mut mask).is_none());
    }

    #[test]
    fn trace_single_pixel_square() {
        let mut mask = vec![0u8; 64];
        mask[3 * 8 + 3] = 255;
        let (points, loops) = wand_trace(&mask, 8, 8).unwrap();
        assert_eq!(loops, vec![4]);
        assert_eq!(points.len(), 4);
        // Corners of pixel (3,3).
        let expected = [(3, 3), (4, 3), (4, 4), (3, 4)];
        for e in expected {
            assert!(points.contains(&(e.0, e.1)), "{points:?}");
        }
    }

    #[test]
    fn trace_two_by_two_and_two_regions() {
        let mut mask = vec![0u8; 64];
        for y in 2..4 {
            for x in 2..4 {
                mask[y * 8 + x] = 255;
            }
        }
        mask[6 * 8 + 6] = 255;
        let (points, loops) = wand_trace(&mask, 8, 8).unwrap();
        assert_eq!(loops.len(), 2);
        assert_eq!(loops.iter().sum::<i32>(), points.len() as i32);
        // Straight runs collapse: a solid block's outline is its four corners.
        assert!(loops.iter().all(|l| *l == 4), "{loops:?}");
    }

    #[test]
    fn trace_empty_mask() {
        let mask = vec![0u8; 64];
        let (points, loops) = wand_trace(&mask, 8, 8).unwrap();
        assert!(points.is_empty() && loops.is_empty());
    }
}
