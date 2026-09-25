//! Port of `Compositor/Document/MaskTracing.swift`: turns raster coverage
//! into a selection outline along exact pixel edges. Cmd-clicking a mask
//! thumbnail loads its dark (hidden) areas as the selection, Cmd-clicking a
//! layer thumbnail loads its opaque pixels, and Subject Removal loads what
//! its mask shows — all four Swift entries trace through the same routine.
//!
//! The trace collects the directed unit edges between selected and
//! unselected pixels and stitches them into loops along the pixel-corner
//! grid; outer boundaries come out clockwise and holes counterclockwise, so
//! the winding fill rule (`SelectionPath::contains`, `rasterize`) reproduces
//! exactly the traced pixels. Only corner vertices survive: vertices that
//! continue in a straight line are dropped, the same collapse the C wand
//! tracer performs. `None` when nothing passes the test.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::geometry::CGPoint;
use crate::raster::{Bitmap, GrayMask};
use crate::selection::SelectionPath;

/// `MaskTracing.darkPixels(in:)`: outline of a mask's pixels darker than 50%
/// gray — what a mask hides (`value < 128`).
pub fn dark_pixels(mask: &GrayMask) -> Option<SelectionPath> {
    trace(mask.width, mask.height, |x, y| mask.value(x, y) < 128)
}

/// `MaskTracing.whitePixels(in:)`: outline of a mask's pixels lighter than
/// 50% gray — what a mask shows (`value >= 128`).
pub fn white_pixels(mask: &GrayMask) -> Option<SelectionPath> {
    trace(mask.width, mask.height, |x, y| mask.value(x, y) >= 128)
}

/// `MaskTracing.opaquePixels(in:)`: outline of an image's pixels that are at
/// least 50% opaque — a layer's visible pixels, ignoring its mask, as in
/// Photoshop (`alpha >= 128`).
pub fn opaque_pixels(image: &Bitmap) -> Option<SelectionPath> {
    trace(image.width, image.height, |x, y| image.pixel(x, y)[3] >= 128)
}

/// `MaskTracing.trace`: outline, in the image's top-left pixel coordinates,
/// of the `width` × `height` pixels whose value passes `test`. One ring per
/// boundary loop; `None` when no pixel passes.
pub fn trace(width: usize, height: usize, selected: impl Fn(usize, usize) -> bool) -> Option<SelectionPath> {
    if width == 0 || height == 0 {
        return None;
    }
    let inside = |x: i64, y: i64| -> bool {
        x >= 0 && y >= 0 && x < width as i64 && y < height as i64 && selected(x as usize, y as usize)
    };
    // Directed unit edges between selected and unselected pixels, keyed by
    // their start vertex on the (width + 1) × (height + 1) corner grid. The
    // insertion order fixes which way loops part where they touch at a
    // corner; `pop` takes the newest, exactly as Swift's `popLast` does.
    let stride = width + 1;
    let mut outgoing: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut edge = |x0: usize, y0: usize, x1: usize, y1: usize| {
        outgoing.entry(y0 * stride + x0).or_default().push(y1 * stride + x1);
    };
    for y in 0..height {
        for x in 0..width {
            if !selected(x, y) {
                continue;
            }
            let (xi, yi) = (x as i64, y as i64);
            if !inside(xi, yi - 1) {
                edge(x, y, x + 1, y);
            }
            if !inside(xi + 1, yi) {
                edge(x + 1, y, x + 1, y + 1);
            }
            if !inside(xi, yi + 1) {
                edge(x + 1, y + 1, x, y + 1);
            }
            if !inside(xi - 1, yi) {
                edge(x, y + 1, x, y);
            }
        }
    }
    if outgoing.is_empty() {
        return None;
    }
    let mut path = SelectionPath::default();
    while let Some((&start, _)) = outgoing.iter().next() {
        // Stitch edges into a loop. Every vertex's in- and out-degrees match
        // by construction, so the walk always returns to its start; a loop
        // with no outgoing edge left (impossible here) would be discarded,
        // as in Swift.
        let mut loop_indices: Vec<usize> = Vec::new();
        let mut current = start;
        loop {
            let Some(ends) = outgoing.get_mut(&current) else { break };
            let Some(end) = ends.pop() else { break };
            if ends.is_empty() {
                outgoing.remove(&current);
            }
            loop_indices.push(current);
            current = end;
            if current == start {
                break;
            }
        }
        // Keep only corners: drop vertices that continue in a straight line.
        let count = loop_indices.len();
        if count < 3 {
            continue;
        }
        let mut corners: Vec<CGPoint> = Vec::with_capacity(count);
        for (index, &vertex) in loop_indices.iter().enumerate() {
            let previous = loop_indices[(index + count - 1) % count];
            let following = loop_indices[(index + 1) % count];
            let (x, y) = ((vertex % stride) as i64, (vertex / stride) as i64);
            let in_x = x - (previous % stride) as i64;
            let in_y = y - (previous / stride) as i64;
            let out_x = (following % stride) as i64 - x;
            let out_y = (following / stride) as i64 - y;
            if in_x != out_x || in_y != out_y {
                corners.push(CGPoint::new(x as f64, y as f64));
            }
        }
        if corners.len() >= 3 {
            path.rings.push(corners);
        }
    }
    if path.rings.is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::CGRect;
    use crate::selection::rasterize;

    fn gray_from_values(width: usize, height: usize, values: &[u8]) -> GrayMask {
        GrayMask::from_raw(width, height, values.to_vec())
    }

    #[test]
    fn dark_and_white_split_a_half_black_half_white_mask() {
        // Left black, right white: dark selects the left half and white the
        // right, each along exact pixel edges.
        let mut mask = GrayMask::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                mask.data[y * 4 + x] = if x < 2 { 0 } else { 255 };
            }
        }
        let dark = dark_pixels(&mask).unwrap();
        assert_eq!(dark.bounds(), Some(CGRect::new(0.0, 0.0, 2.0, 2.0)));
        let covered = rasterize(&dark, 4, 2, false);
        assert_eq!(covered.value(0, 0), 255);
        assert_eq!(covered.value(1, 1), 255);
        assert_eq!(covered.value(2, 0), 0);
        assert_eq!(covered.value(3, 1), 0);

        let white = white_pixels(&mask).unwrap();
        assert_eq!(white.bounds(), Some(CGRect::new(2.0, 0.0, 2.0, 2.0)));
        let covered = rasterize(&white, 4, 2, false);
        assert_eq!(covered.value(2, 0), 255);
        assert_eq!(covered.value(3, 1), 255);
        assert_eq!(covered.value(1, 1), 0);
    }

    #[test]
    fn threshold_boundary_pixel_at_128_goes_to_the_white_side() {
        // darkPixels is `< 128`, whitePixels `>= 128`: a column of 127s is
        // dark, columns of 128s are white, and nothing darker shows up on
        // the white side.
        let mask = gray_from_values(3, 2, &[127, 128, 128, 127, 128, 128]);
        let dark = dark_pixels(&mask).unwrap();
        assert_eq!(dark.bounds(), Some(CGRect::new(0.0, 0.0, 1.0, 2.0)));
        let white = white_pixels(&mask).unwrap();
        assert_eq!(white.bounds(), Some(CGRect::new(1.0, 0.0, 2.0, 2.0)));
        // All black: dark takes everything; all white: dark finds nothing.
        let full_white = gray_from_values(2, 2, &[255; 4]);
        assert!(dark_pixels(&full_white).is_none());
        let full_black = gray_from_values(2, 2, &[0; 4]);
        assert!(white_pixels(&full_black).is_none());
    }

    #[test]
    fn white_mask_with_black_square_traces_a_hole_on_exact_pixel_edges() {
        // The Cmd-click mask test's shape: white everywhere except a black
        // square, with a smaller white square back inside it (a hole in the
        // hole). The winding fill of the traced rings must reproduce the
        // pixels exactly.
        let mut mask = GrayMask::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                let black_square = (20..60).contains(&x) && (30..70).contains(&y);
                let white_square = (30..40).contains(&x) && (40..50).contains(&y);
                mask.data[y * 100 + x] = if black_square && !white_square { 0 } else { 255 };
            }
        }
        let path = white_pixels(&mask).unwrap();
        assert_eq!(path.bounds(), Some(CGRect::new(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(path.rings.len(), 3, "outer, black square, white square");
        let covered = rasterize(&path, 100, 100, false);
        assert_eq!(covered.value(25, 35), 0, "black: hidden");
        assert_eq!(covered.value(35, 45), 255, "white square back inside the black one");
        assert_eq!(covered.value(80, 80), 255, "white surroundings");
        assert_eq!(covered.value(59, 69), 0, "the black square's last pixel is hidden");
        assert_eq!(covered.value(60, 70), 255, "exact pixel edges: 60,70 is white-side");
    }

    #[test]
    fn transparent_layer_traces_nothing_and_opaque_ring_has_a_hole() {
        // The Cmd-click layer test's shape: an opaque square with a cleared
        // center and one 25%-alpha corner pixel under the threshold.
        let mut image = Bitmap::new(50, 50);
        for y in 10..40 {
            for x in 10..40 {
                image.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        for y in 20..30 {
            for x in 20..30 {
                image.set_pixel(x, y, [0, 0, 0, 0]);
            }
        }
        image.set_pixel(0, 0, [255, 0, 0, 64]);
        assert!(opaque_pixels(&Bitmap::new(50, 50)).is_none(), "a blank layer selects nothing");
        let path = opaque_pixels(&image).unwrap();
        assert_eq!(path.bounds(), Some(CGRect::new(10.0, 10.0, 30.0, 30.0)));
        assert!(path.contains(CGPoint::new(15.0, 15.0)), "opaque ring");
        assert!(!path.contains(CGPoint::new(25.0, 25.0)), "transparent center");
        assert!(!path.contains(CGPoint::new(0.5, 0.5)), "the 25% pixel is under the threshold");
    }

    #[test]
    fn diagonal_pixels_stay_selected_through_a_shared_corner() {
        // Two diagonal pixels trace loops that touch at one corner; the
        // traced path still covers exactly those two pixels.
        let mask = gray_from_values(2, 2, &[255, 0, 0, 255]);
        let path = white_pixels(&mask).unwrap();
        let covered = rasterize(&path, 2, 2, false);
        assert_eq!(covered.value(0, 0), 255);
        assert_eq!(covered.value(1, 1), 255);
        assert_eq!(covered.value(1, 0), 0);
        assert_eq!(covered.value(0, 1), 0);
    }

    #[test]
    fn straight_runs_collapse_to_corners() {
        // A solid rectangle's outline is its four corners, however large.
        let mut mask = GrayMask::new(20, 10);
        for value in mask.data.iter_mut() {
            *value = 255;
        }
        for y in 3..8 {
            for x in 2..15 {
                mask.data[y * 20 + x] = 0;
            }
        }
        let path = dark_pixels(&mask).unwrap();
        assert_eq!(path.rings.len(), 1);
        assert_eq!(path.rings[0].len(), 4);
        assert_eq!(path.bounds(), Some(CGRect::new(2.0, 3.0, 13.0, 5.0)));
    }
}
