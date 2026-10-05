//! Port of `Compositor/CompositorTests/DownsampleTests.swift`: shrinking
//! layers by large factors must stay sharp and clean, as Photoshop's
//! resampling does.
//!
//! Swift calls `DownsampleCache` (an LRU of Lanczos halvings keyed by image
//! identity) and draws through `LayerRenderer`. The Rust public equivalents
//! are `downsample_level` (the cache's level table), `halve` (the sharp
//! halving itself) and `draw_affine` with `LayerSampling::High` (the renderer
//! draw path, which runs the same halving chain internally). The cache's
//! object-identity assertions (`===`) have no Rust counterpart — the halving
//! is a pure function — so "copies are cached" is asserted as determinism.

use compositor_core::geometry::CGAffineTransform;
use compositor_core::raster::{Bitmap, downsample_level, draw_affine, halve, resample};
use compositor_core::transform::LayerSampling;

/// An opaque RGBA image whose pixel colors come from `gray(x, y)`
/// (Swift `image(width:height:gray:)`).
fn gray_image(width: usize, height: usize, gray: impl Fn(usize, usize) -> u8) -> Bitmap {
    let mut image = Bitmap::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let value = gray(x, y);
            image.set_pixel(x, y, [value, value, value, 255]);
        }
    }
    image
}

/// Draws `image` shrunk by `scale` into a `width` × `height` canvas with High
/// quality sampling, exactly the Swift helper `drawn(_:width:height:)` (a
/// `LayerRenderer.draw` of the whole image scaled to fill the context).
fn drawn(image: &Bitmap, width: usize, height: usize, scale: f64) -> Bitmap {
    let mut out = Bitmap::new(width, height);
    draw_affine(
        &mut out,
        image,
        &CGAffineTransform::scale(scale, scale),
        LayerSampling::High,
        None,
    );
    out
}

/// Swift `halvingsAreReusedAndOnlyUsedForLargeReductions`.
#[test]
fn halvings_are_reused_and_only_used_for_large_reductions() {
    let source = gray_image(1024, 512, |x, _| if x % 2 == 0 { 0 } else { 255 });

    // "half size and up draws the image itself": at factor 0.6 the cache
    // hands back the image itself, i.e. no halving applies.
    assert_eq!(downsample_level(0.6), 0);

    // At 0.125 (an eighth) three halvings are used: 1024×512 → 128×64.
    assert_eq!(downsample_level(0.125), 3);
    let eighth = halve(&halve(&halve(&source)));
    assert_eq!(eighth.width, 128, "eighth width");
    assert_eq!(eighth.height, 64, "eighth height");

    // "0.3 uses the half, leaving less than 2× to Core Graphics": exactly one
    // halving, so the copy is 512 wide.
    assert_eq!(downsample_level(0.3), 1);
    assert_eq!(halve(&source).width, 512);

    // "copies are cached": the cache returns the very same copy — in Rust the
    // halving is pure, so the same level reproduces the same bytes.
    let again = halve(&halve(&halve(&source)));
    assert_eq!(again.data, eighth.data);
}

/// Swift `aHardEdgeStaysSharpShrunkEightTimes`.
#[test]
fn a_hard_edge_stays_sharp_shrunk_eight_times() {
    let source = gray_image(4096, 64, |x, _| if x < 2048 { 0 } else { 255 });
    let pixels = drawn(&source, 512, 8, 0.125);
    let row: Vec<u8> = (0..512).map(|x| pixels.pixel(x, 4)[0]).collect();
    let soft = row.iter().filter(|v| **v > 40 && **v < 215).count();
    assert!(soft <= 3, "the edge smears across {soft} pixels");
    assert!(row[250] < 10, "black side stays black: {}", row[250]);
    assert!(row[262] > 245, "white side stays white: {}", row[262]);
}

/// Swift `fineStripesAverageToFlatGrayWithoutShimmer`.
#[test]
fn fine_stripes_average_to_flat_gray_without_shimmer() {
    let source = gray_image(2048, 256, |x, _| if x % 2 == 0 { 0 } else { 255 });
    let pixels = drawn(&source, 256, 32, 0.125);
    // The outermost pixels fade into the transparent edge; judge the inside.
    let values: Vec<f64> = (4..252).map(|x| pixels.pixel(x, 16)[0] as f64).collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let spread =
        (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / values.len() as f64).sqrt();
    assert!((mean - 127.5).abs() < 8.0, "mean {mean}");
    assert!(
        spread < 6.0,
        "stripes shimmer after shrinking: spread {spread}"
    );
}

/// Swift `translucentEdgesStayValidAndMasksStayGray`.
///
/// The second half runs against a gray-in-RGBA mask bitmap — this port keeps
/// masks as grayscale-in-RGBA (`LayerMask` assets), so "stays monochrome with
/// no alpha" means the halved copy keeps R = G = B with alpha 255.
#[test]
fn translucent_edges_stay_valid_and_masks_stay_gray() {
    // A 512×512 canvas with a half-opaque white square in the middle
    // (`context.fill(CGRect(128, 128, 256, 256))` with white @ 0.5).
    let mut translucent = Bitmap::new(512, 512);
    for y in 128..384 {
        for x in 128..384 {
            translucent.set_pixel(x, y, [127, 127, 127, 128]);
        }
    }
    // `DownsampleCache.shared.image(translucent, drawnAt: 0.25)`: two sharp
    // halvings and an exact final pass — `resample` with High runs both.
    let shrunk = resample(&translucent, 128, 128, LayerSampling::High);
    assert_eq!((shrunk.width, shrunk.height), (128, 128));
    for i in 0..(shrunk.width * shrunk.height) {
        assert!(
            shrunk.data[i * 4] <= shrunk.data[i * 4 + 3],
            "no color above its alpha at pixel {i}"
        );
    }

    // The mask: left half revealing, halved twice — it must stay a plain gray
    // level map (no alpha), 128 wide. A grayscale mask has no alpha, so in
    // this port's gray-in-RGBA representation the hidden half is opaque black.
    let mut mask = Bitmap::new(512, 512);
    for y in 0..512 {
        for x in 0..512 {
            mask.set_pixel(
                x,
                y,
                if x < 256 {
                    [255, 255, 255, 255]
                } else {
                    [0, 0, 0, 255]
                },
            );
        }
    }
    let level = halve(&halve(&mask));
    assert_eq!(level.width, 128);
    for px in level.data.chunks_exact(4) {
        assert_eq!(px[0], px[1], "gray stays gray");
        assert_eq!(px[1], px[2], "gray stays gray");
        assert_eq!(px[3], 255, "a mask carries no alpha");
    }
}
