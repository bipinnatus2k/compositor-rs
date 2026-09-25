//! Port of `Compositor/CompositorTests/ExportTests.swift` and
//! `JPEGExportTests.swift`: PNG export preserves dimensions, alpha,
//! orientation and transforms; layer order and visibility decide the output;
//! blank and oversized canvases behave; the JPEG encoder applies its matte
//! and quality.
//!
//! Swift exports a `ProjectSnapshot` through `ImageExporter`; the Rust
//! equivalent renders the same `CanvasDocument` with `draw_live_composite`
//! and encodes with `export::png_data` / `export::jpeg_data`. The two file
//! writes go through `std::fs` (`ImageExporter.exportPNG`/`write`'s atomic
//! rename is app-level and not part of the core API — noted in the porting
//! report). Decoded-colorspace assertions (`sRGB` metadata) have no
//! `image`-crate equivalent and are dropped; the JPEG identity check uses the
//! `public.jpeg` magic bytes.

use compositor_core::document::{CanvasDocument, ImageLayer};
use compositor_core::export::{check_export_size, jpeg_data, png_data, JpegOptions};
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::transform::{LayerSampling, LayerTransform};

/// Swift `snapshot(rotation:flip:)`: a 6×6 document with one layer holding a
/// 2×2 image whose left column is red, placed by a 4×4 transform at (1, 1)
/// with Nearest sampling.
fn red_strip_document(rotation: f64, flip: bool) -> CanvasDocument {
    let mut image = Bitmap::new(2, 2);
    for y in 0..2 {
        image.set_pixel(0, y, [255, 0, 0, 255]);
    }
    let mut layer = ImageLayer::with_asset(
        ImportedImage::with_thumbnail(image.clone(), image, "Red"),
        CGPoint::new(1.0, 1.0),
    );
    layer.transform = LayerTransform {
        origin: CGPoint::new(1.0, 1.0),
        size: CGSize::new(4.0, 4.0),
        rotation,
        flip_x: flip,
        flip_y: false,
        sampling: LayerSampling::Nearest,
    };
    let mut document = CanvasDocument::new(6, 6);
    document.layers.push(layer);
    document
}

fn export_png(document: &CanvasDocument) -> image::RgbaImage {
    let png = png_data(&draw_live_composite(document)).expect("png encodes");
    image::load_from_memory(&png).expect("png decodes").to_rgba8()
}

/// A pixel as straight (unpremultiplied) components 0–1, as
/// `NSBitmapImageRep.colorAt` reports them.
fn straight_at(image: &image::RgbaImage, x: u32, y: u32) -> [f64; 4] {
    let px = image.get_pixel(x, y);
    let a = px[3] as f64 / 255.0;
    if a <= 0.0 {
        return [0.0; 4];
    }
    [
        px[0] as f64 / 255.0 / a,
        px[1] as f64 / 255.0 / a,
        px[2] as f64 / 255.0 / a,
        a,
    ]
}

/// Swift `pngPreservesDimensionsAlphaOrientationAndTransforms`.
#[test]
fn png_preserves_dimensions_alpha_orientation_and_transforms() {
    for (rotation, flip, red_x, red_y, clear_x, clear_y) in [
        (0.0, false, 1u32, 1u32, 4u32, 1u32),
        (0.0, true, 4, 1, 1, 1),
        (90.0, false, 1, 1, 1, 4),
    ] {
        let image = export_png(&red_strip_document(rotation, flip));
        assert_eq!((image.width(), image.height()), (6, 6), "canvas size {rotation} {flip}");

        let red = straight_at(&image, red_x, red_y);
        assert!(red[0] > 0.99, "red at ({red_x}, {red_y}): {red:?}");
        assert_eq!(red[3], 1.0, "red is opaque at ({red_x}, {red_y}): {red:?}");

        let clear = straight_at(&image, clear_x, clear_y);
        assert_eq!(clear[3], 0.0, "transparent at ({clear_x}, {clear_y}): {clear:?}");

        let corner = straight_at(&image, 0, 0);
        assert_eq!(corner[3], 0.0, "corner stays transparent: {corner:?}");
    }
}

/// Swift `orderVisibilityClippingAndAtomicOverwrite`: the top layer wins when
/// visible, and exporting to the same URL twice overwrites it.
#[test]
fn order_visibility_and_overwrite() {
    let mut document = red_strip_document(0.0, false);

    let mut blue = Bitmap::new(1, 1);
    blue.set_pixel(0, 0, [0, 0, 255, 255]);
    let mut blue_layer = ImageLayer::with_asset(
        ImportedImage::with_thumbnail(blue.clone(), blue, "Blue"),
        CGPoint::new(-2.0, -2.0),
    );
    blue_layer.transform = LayerTransform {
        origin: CGPoint::new(-2.0, -2.0),
        size: CGSize::new(10.0, 10.0),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        sampling: LayerSampling::Nearest,
    };
    document.layers.push(blue_layer.clone());

    let url = std::env::temp_dir().join(format!("Export-{}.png", uuid::Uuid::new_v4()));
    for visible in [true, false] {
        document.layers[1].is_visible = visible;
        let png = png_data(&draw_live_composite(&document)).expect("png encodes");
        std::fs::write(&url, &png).expect("export writes");
        let stored = std::fs::read(&url).expect("overwritten file reads back");
        let image = image::load_from_memory(&stored).expect("decodes").to_rgba8();
        let pixel = straight_at(&image, 1, 1);
        if visible {
            assert!(pixel[2] > 0.99, "visible blue wins: {pixel:?}");
        } else {
            assert!(pixel[0] > 0.99, "hidden blue reveals red: {pixel:?}");
        }
        assert_eq!((image.width(), image.height()), (6, 6));
    }
    let _ = std::fs::remove_file(&url);
}

/// Swift `blankCanvasAndOversizedCanvas`.
#[test]
fn blank_canvas_and_oversized_canvas() {
    let blank = CanvasDocument::new(2, 2);
    let image = export_png(&blank);
    let pixel = straight_at(&image, 1, 1);
    assert_eq!(pixel[3], 0.0, "a blank canvas exports transparent: {pixel:?}");

    // A 30,000 × 30,000 canvas breaches the 100-megapixel guard
    // (`ExportError.tooLarge` ← `check_export_size`).
    assert!(check_export_size(30_000, 30_000).is_err());
    assert!(check_export_size(2, 2).is_ok());
}

/// Swift `JPEGExportTests.transparencyUsesChosenMatteAndProducesOpaqueSRGB`.
#[test]
fn jpeg_transparency_uses_chosen_matte_and_produces_opaque_pixels() {
    // The rendered raster of an empty 20×12 document: fully transparent.
    let raster = draw_live_composite(&CanvasDocument::new(20, 12));
    for options in [
        JpegOptions::default(),
        JpegOptions {
            quality: 1.0,
            red: 0.0,
            green: 0.0,
            blue: 1.0,
        },
    ] {
        let data = jpeg_data(&raster, &options).expect("jpeg encodes");
        assert_eq!(&data[0..2], &[0xFF, 0xD8], "public.jpeg magic bytes");
        let decoded = image::load_from_memory(&data).expect("decodes").to_rgba8();
        assert_eq!((decoded.width(), decoded.height()), (20, 12));
        let pixel = decoded.get_pixel(0, 0);
        assert_eq!(pixel[3], 255, "the matte makes it opaque: {pixel:?}");
        assert!((pixel[0] as f64 / 255.0 - options.red).abs() < 0.03, "{pixel:?}");
        assert!((pixel[1] as f64 / 255.0 - options.green).abs() < 0.03, "{pixel:?}");
        assert!((pixel[2] as f64 / 255.0 - options.blue).abs() < 0.03, "{pixel:?}");
    }
}

/// Swift `JPEGExportTests.qualityChangesBytesAndDecodedPixels`.
#[test]
fn jpeg_quality_changes_bytes_and_decoded_pixels() {
    let mut raster = Bitmap::new(128, 128);
    for y in 0..128 {
        for x in 0..128 {
            let red = ((x * 37 + y * 17) % 256) as u8;
            let green = ((x * 11 + y * 53) % 256) as u8;
            let blue = ((x * 79 + y * 7) % 256) as u8;
            raster.set_pixel(x, y, [red, green, blue, 255]);
        }
    }
    let low = jpeg_data(
        &raster,
        &JpegOptions {
            quality: 0.1,
            ..Default::default()
        },
    )
    .unwrap();
    let high = jpeg_data(
        &raster,
        &JpegOptions {
            quality: 1.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(low.len() < high.len(), "{} vs {}", low.len(), high.len());

    let low_bitmap = image::load_from_memory(&low).expect("decodes").to_rgba8();
    let high_bitmap = image::load_from_memory(&high).expect("decodes").to_rgba8();
    let mut difference = 0.0;
    for y in 0..16 {
        for x in 0..16 {
            let a = low_bitmap.get_pixel(x, y);
            let b = high_bitmap.get_pixel(x, y);
            difference += (a[0] as f64 - b[0] as f64).abs();
        }
    }
    assert!(difference > 1.0, "quality changes decoded pixels: {difference}");

    let url = std::env::temp_dir().join(format!("JPEG-{}.jpg", uuid::Uuid::new_v4()));
    std::fs::write(&url, &high).expect("write");
    assert_eq!(std::fs::read(&url).expect("read back"), high, "the bytes round-trip");
    let _ = std::fs::remove_file(&url);
}
