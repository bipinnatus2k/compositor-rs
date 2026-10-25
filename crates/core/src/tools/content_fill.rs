//! Port of `Rendering/ContentFill.c` and `Document/ContentFill.swift`:
//! content-aware fill by coherent offset propagation plus randomized patch
//! search, spreading inward from the selection's boundary.

use crate::geometry::CGAffineTransform;
use crate::raster::Bitmap;
use crate::selection::SelectionClip;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
}

/// Mean squared difference between the known pixels around `p` and those
/// around candidate donor `q` (an (2r+1)² patch).
fn patch_match(pixels: &Bitmap, known: &[u8], w: i64, h: i64, p: i64, q: i64, radius: i64) -> f64 {
    let (px, py) = (p % w, p / w);
    let (qx, qy) = (q % w, q / w);
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let x = px + dx;
            let y = py + dy;
            let sx = qx + dx;
            let sy = qy + dy;
            if x < 0 || y < 0 || x >= w || y >= h || sx < 0 || sy < 0 || sx >= w || sy >= h {
                continue;
            }
            let (x, y, sx, sy) = (x as usize, y as usize, sx as usize, sy as usize);
            if known[y * w as usize + x] == 0 {
                continue;
            }
            let a = pixels.pixel(x, y);
            let b = pixels.pixel(sx, sy);
            for c in 0..4 {
                let d = a[c] as f64 - b[c] as f64;
                sum += d * d;
            }
            count += 1;
        }
    }
    if count > 0 {
        sum / count as f64
    } else {
        f64::MAX
    }
}

/// `content_fill` — fills the masked area of `pixels` in place from its own
/// known texture. Returns false when there are no donor pixels.
pub fn content_fill(pixels: &mut Bitmap, mask: &[u8]) -> bool {
    let w = pixels.width as i64;
    let h = pixels.height as i64;
    let n = (w * h) as usize;
    let mut known = vec![0u8; n];
    let mut target = vec![0u8; n];
    let mut valid = vec![0u8; n];
    let mut queued = vec![0u8; n];
    let mut donors: Vec<i64> = vec![0; n + 1];
    let mut queue: Vec<i64> = vec![0; n];
    let mut chosen: Vec<i64> = vec![-1; n];
    let radius: i64 = if w >= 5 && h >= 5 { 2 } else { 0 };
    let mut missing = 0usize;
    let mut donor_count = 0usize;

    // Selected pixels are filled. Unselected opaque pixels are the image to
    // match and copy from; unselected transparent ones are left as they are.
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) as usize;
            target[p] = (mask[(y as usize) * pixels.width + x as usize] != 0) as u8;
            known[p] = (target[p] == 0 && pixels.pixel(x as usize, y as usize)[3] == 255) as u8;
            if target[p] == 1 {
                missing += 1;
            }
        }
    }
    if missing == 0 {
        return true;
    }
    // Donors: known pixels whose whole (2r+1)² neighborhood is known.
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            if known[p as usize] == 0 {
                continue;
            }
            let mut ok = true;
            for dy in -radius..=radius {
                if !ok {
                    break;
                }
                for dx in -radius..=radius {
                    let sx = x + dx;
                    let sy = y + dy;
                    if sx < 0 || sy < 0 || sx >= w || sy >= h || known[(sy * w + sx) as usize] == 0
                    {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                valid[p as usize] = 1;
                donors[donor_count] = p;
                donor_count += 1;
            }
        }
    }
    if donor_count == 0 {
        return false;
    }
    // Seed the queue with target pixels touching known ones.
    let mut head = 0usize;
    let mut tail = 0usize;
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            if target[p as usize] == 1
                && ((x > 0 && known[(p - 1) as usize] == 1)
                    || (x + 1 < w && known[(p + 1) as usize] == 1)
                    || (y > 0 && known[(p - w) as usize] == 1)
                    || (y + 1 < h && known[(p + w) as usize] == 1))
            {
                queue[tail] = p;
                tail += 1;
                queued[p as usize] = 1;
            }
        }
    }
    let mut rng = Rng(0x6d2b79f5);
    let mut scan = 0usize;
    loop {
        while head < tail {
            let p = queue[head];
            head += 1;
            let (x, y) = (p % w, p / w);
            let mut best = -1i64;
            let mut score = f64::MAX;
            let neighbors: [i64; 4] = [
                if x > 0 { p - 1 } else { -1 },
                if x + 1 < w { p + 1 } else { -1 },
                if y > 0 { p - w } else { -1 },
                if y + 1 < h { p + w } else { -1 },
            ];
            // Propagate coherent source offsets, then refine with randomized
            // patch search.
            for k in 0..28 {
                let mut q: i64 = -1;
                if k < 4 {
                    let t = neighbors[k];
                    if t >= 0 {
                        let chosen_t = if chosen[t as usize] >= 0 {
                            chosen[t as usize]
                        } else {
                            t
                        };
                        q = chosen_t + (p - t);
                    }
                } else {
                    q = donors[rng.next() as usize % donor_count];
                }
                if q < 0 || (q as usize) >= n || valid[q as usize] == 0 {
                    continue;
                }
                let s = patch_match(pixels, &known, w, h, p, q, radius);
                if best < 0 || s < score {
                    score = s;
                    best = q;
                }
            }
            if best < 0 {
                best = donors[0];
            }
            let mut r = 64i64;
            while r >= 1 {
                let qx = best % w + (rng.next() as i64 % (2 * r + 1)) - r;
                let qy = best / w + (rng.next() as i64 % (2 * r + 1)) - r;
                r /= 2;
                if qx < 0 || qy < 0 || qx >= w || qy >= h || valid[(qy * w + qx) as usize] == 0 {
                    continue;
                }
                let q = qy * w + qx;
                let s = patch_match(pixels, &known, w, h, p, q, radius);
                if s < score {
                    score = s;
                    best = q;
                }
            }
            let src = pixels.pixel((best % w) as usize, (best / w) as usize);
            pixels.set_pixel(x as usize, y as usize, src);
            known[p as usize] = 1;
            chosen[p as usize] = best;
            for k in 0..4 {
                let q = neighbors[k];
                if q >= 0
                    && target[q as usize] == 1
                    && known[q as usize] == 0
                    && queued[q as usize] == 0
                {
                    queued[q as usize] = 1;
                    if tail < n {
                        queue[tail] = q;
                        tail += 1;
                    }
                }
            }
        }
        // A selected area that only transparency touches starts from the best
        // random donor, then spreads.
        while scan < n && (target[scan] == 0 || known[scan] == 1) {
            scan += 1;
        }
        if scan >= n {
            break;
        }
        queue[tail] = scan as i64;
        tail += 1;
        queued[scan] = 1;
    }
    donor_count > 0
}

/// `ContentFill.run`: fill the selection region of `image` from its own
/// texture. Returns None when there is no usable source.
pub fn run_fill(
    image: &Bitmap,
    selection: &SelectionClip,
    mapping: &CGAffineTransform,
) -> Option<Bitmap> {
    let w = image.width;
    let h = image.height;
    let mut pixels = image.clone();
    // The selection coverage on this grid: white where the fill goes.
    let cov = crate::adjust::pixel::coverage(selection, w, h, mapping);
    let mask = cov.data.clone();
    if !content_fill(&mut pixels, &mask) {
        return None;
    }
    Some(pixels)
}

/// `ContentFill.run` entry used by `filters::run_filter`.
pub fn run(image: &Bitmap) -> Bitmap {
    // Standalone entry (no selection): nothing to synthesize from.
    image.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{CGPoint, CGRect, CGSize};
    use crate::selection::{DocumentSelection, SelectionPath};

    fn rect_clip() -> SelectionClip {
        let sel = DocumentSelection::new(SelectionPath::rect(CGRect::new(6.0, 6.0, 4.0, 4.0)));
        sel.clip(CGSize::new(16.0, 16.0))
    }

    #[test]
    fn fills_selection_from_surroundings() {
        let mut img = Bitmap::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                img.set_pixel(x, y, [10, 200, 30, 255]);
            }
        }
        // Punch a hole in the middle (the "selection").
        for y in 6..10 {
            for x in 6..10 {
                img.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        let clip = rect_clip();
        let identity = CGAffineTransform::IDENTITY;
        let filled = run_fill(&img, &clip, &identity).unwrap();
        let p = filled.pixel(7, 7);
        // The fill is synthesized from the surrounding green field.
        assert!(p[1] > 150, "{p:?}");
        assert_eq!(filled.pixel(0, 0), [10, 200, 30, 255]);
    }

    #[test]
    fn no_known_pixels_returns_none() {
        let mut img = Bitmap::new(4, 4);
        for i in (0..img.data.len()).step_by(4) {
            img.data[i + 3] = 0; // all transparent: no donors
        }
        let sel = DocumentSelection::new(SelectionPath::rect(CGRect::new(1.0, 1.0, 2.0, 2.0)));
        let clip = sel.clip(CGSize::new(4.0, 4.0));
        assert!(run_fill(&img, &clip, &CGAffineTransform::IDENTITY).is_none());
    }

    #[test]
    fn noise_field_preserves_texture_statistics() {
        // Deterministic per-pixel noise; the fill should not paint flat.
        let mut img = Bitmap::new(24, 24);
        for y in 0..24 {
            for x in 0..24 {
                let v = ((x * 37 + y * 91) % 251) as u8;
                img.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        let sel = DocumentSelection::new(SelectionPath::rect(CGRect::new(8.0, 8.0, 6.0, 6.0)));
        let clip = sel.clip(CGSize::new(24.0, 24.0));
        let filled = run_fill(&img, &clip, &CGAffineTransform::IDENTITY).unwrap();
        // Varied pixels inside the filled area.
        let a = filled.pixel(9, 9)[0];
        let b = filled.pixel(12, 11)[0];
        assert_ne!(a, b, "fill should not be flat: {a} vs {b}");
        let _ = CGPoint::ZERO;
    }
}
