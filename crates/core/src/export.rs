//! Port of `IO/ImageExporter.swift`: export size guards, the PNG encoder, and
//! the JPEG encoder with its matte flatten and quality option.

use crate::raster::Bitmap;

/// `JPEGOptions` — the matte is the background for transparency; the quality
/// runs 0…1 as in the Swift options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JpegOptions {
    pub quality: f64,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

impl Default for JpegOptions {
    fn default() -> Self {
        JpegOptions {
            quality: 0.85,
            red: 1.0,
            green: 1.0,
            blue: 1.0,
        }
    }
}

/// `ExportError.tooLarge`: canvases up to 100 megapixels and 30,000 pixels
/// per side.
pub fn check_export_size(width: usize, height: usize) -> Result<(), &'static str> {
    if !(1..=30_000).contains(&width)
        || !(1..=30_000).contains(&height)
        || width * height > 100_000_000
    {
        return Err("Image export supports canvases up to 100 megapixels and 30,000 pixels per side.");
    }
    Ok(())
}

/// Flattens the composited (premultiplied) canvas onto an opaque matte the
/// way `ImageExporter.jpeg` draws the image over the filled matte: the
/// premultiplied source-over of the matte behind the pixels.
pub fn flatten_on_matte(composite: &Bitmap, red: f64, green: f64, blue: f64) -> Bitmap {
    let matte = [
        ((red * 255.0).round() as u32).clamp(0, 255),
        ((green * 255.0).round() as u32).clamp(0, 255),
        ((blue * 255.0).round() as u32).clamp(0, 255),
    ];
    let mut out = composite.data.clone();
    for pixel in out.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha == 255 {
            continue;
        }
        let rest = 255 - alpha;
        for (channel, m) in matte.iter().enumerate() {
            pixel[channel] = (((pixel[channel] as u32 * 255 + m * rest + 127) / 255) as usize)
                .min(255) as u8;
        }
        pixel[3] = 255;
    }
    Bitmap {
        width: composite.width,
        height: composite.height,
        data: out,
    }
}

/// Straight-alpha PNG bytes for the composited canvas (`pngData`; the DPI
/// metadata property has no image-crate counterpart).
pub fn png_data(composite: &Bitmap) -> Option<Vec<u8>> {
    let image = to_rgba_image(composite);
    let mut buffer = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut buffer), image::ImageFormat::Png)
        .ok()?;
    Some(buffer)
}

/// `pngData` with the document resolution as a pHYs chunk (Swift's
/// `kCGImagePropertyDPIWidth/Height` properties).
pub fn png_data_with_dpi(composite: &Bitmap, dpi: f64) -> Option<Vec<u8>> {
    let png = png_data(composite)?;
    Some(inject_png_dpi(&png, dpi))
}

/// Inserts a pHYs chunk (pixels per meter, unit 1) right after IHDR, where
/// the encoder put none.
fn inject_png_dpi(png: &[u8], dpi: f64) -> Vec<u8> {
    const SIGNATURE: usize = 8;
    if png.len() < SIGNATURE + 12 {
        return png.to_vec();
    }
    let pixels_per_meter = (dpi / 0.0254).round().max(1.0) as u32;
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(b"pHYs");
    chunk.extend_from_slice(&pixels_per_meter.to_be_bytes());
    chunk.extend_from_slice(&pixels_per_meter.to_be_bytes());
    chunk.push(1); // unit: meter
    let crc = crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    // The first chunk is IHDR; pHYs must precede any IDAT.
    let first_length = u32::from_be_bytes([png[8], png[9], png[10], png[11]]) as usize;
    let insert_at = SIGNATURE + 12 + first_length;
    if insert_at > png.len() {
        return png.to_vec();
    }
    let mut out = Vec::with_capacity(png.len() + chunk.len());
    out.extend_from_slice(&png[..insert_at]);
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[insert_at..]);
    out
}

/// CRC-32 (IEEE 802.3, reflected), as PNG chunk checksums require.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// `ImageExporter.jpeg`: the composite flattened onto its matte, encoded with
/// the given quality.
pub fn jpeg_data(composite: &Bitmap, options: &JpegOptions) -> Option<Vec<u8>> {
    let mut flat = flatten_on_matte(composite, options.red, options.green, options.blue);
    let quality = ((options.quality.clamp(0.0, 1.0)) * 100.0).round() as u8;
    // The flatten is fully opaque, so dropping the alpha plane is lossless.
    let mut rgb = Vec::with_capacity(flat.width * flat.height * 3);
    for pixel in flat.data.chunks_exact_mut(4) {
        rgb.extend_from_slice(&pixel[0..3]);
    }
    let mut buffer = Vec::new();
    {
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality);
        encoder
            .encode(
                &rgb,
                flat.width as u32,
                flat.height as u32,
                image::ExtendedColorType::Rgb8,
            )
            .ok()?;
    }
    Some(buffer)
}

/// `jpeg` with the document resolution patched into the JFIF APP0 density
/// fields (Swift's DPI properties; the encoder always writes a JFIF APP0
/// with unit 0 and density 1).
pub fn jpeg_data_with_dpi(composite: &Bitmap, options: &JpegOptions, dpi: f64) -> Option<Vec<u8>> {
    let jpeg = jpeg_data(composite, options)?;
    Some(patch_jfif_density(&jpeg, dpi))
}

/// Rewrites the JFIF APP0's density to dots-per-inch. Leaves the buffer
/// untouched when the header isn't the JFIF segment the encoder writes.
fn patch_jfif_density(jpeg: &[u8], dpi: f64) -> Vec<u8> {
    let mut out = jpeg.to_vec();
    // SOI (FFD8) then APP0 (FFE0): "JFIF\0" at 6..11, units at 13,
    // X/Y density at 14..18.
    let is_jfif = out.len() >= 19
        && out[0] == 0xFF
        && out[1] == 0xD8
        && out[2] == 0xFF
        && out[3] == 0xE0
        && &out[6..11] == b"JFIF\0";
    if !is_jfif {
        return out;
    }
    let density = dpi.round().clamp(1.0, 65_535.0) as u16;
    out[13] = 1; // density unit: dots per inch
    out[14..16].copy_from_slice(&density.to_be_bytes());
    out[16..18].copy_from_slice(&density.to_be_bytes());
    out
}

/// `JPEGResult.preview` — a thumbnail at most 1000 pixels across.
pub fn preview_thumbnail(composite: &Bitmap) -> Bitmap {
    let longest = composite.width.max(composite.height) as f64;
    let limit = 1000.0;
    if longest <= limit {
        return composite.clone();
    }
    let scale = limit / longest;
    let width = ((composite.width as f64 * scale).round() as usize).max(1);
    let height = ((composite.height as f64 * scale).round() as usize).max(1);
    crate::raster::resample(composite, width, height, crate::transform::LayerSampling::High)
}

fn to_rgba_image(composite: &Bitmap) -> image::RgbaImage {
    // The canvas is premultiplied; image wants straight alpha.
    let mut data = composite.data.clone();
    for pixel in data.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha == 0 {
            continue;
        }
        for channel in 0..3 {
            // Round to the nearest straight value instead of truncating.
            pixel[channel] = (((pixel[channel] as u32 * 255) + alpha / 2) / alpha).min(255) as u8;
        }
    }
    image::RgbaImage::from_raw(composite.width as u32, composite.height as u32, data)
        .expect("size matches data")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red_on_transparent() -> Bitmap {
        // Premultiplied half-covered red, as the canvas stores it.
        let mut image = Bitmap::new(8, 8);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel[0] = 128;
            pixel[3] = 128;
        }
        image
    }

    #[test]
    fn export_size_guards() {
        assert!(check_export_size(1, 1).is_ok());
        assert!(check_export_size(30_000, 30_000).is_err(), "over 100 MP");
        assert!(check_export_size(0, 10).is_err());
        assert!(check_export_size(30_001, 1).is_err());
    }

    #[test]
    fn matte_flatten_blends_premultiplied() {
        // Half-covered red over white: the premultiplied red (128) plus the
        // matte showing through — straight pink, fully opaque.
        let composite = red_on_transparent();
        let flat = flatten_on_matte(&composite, 1.0, 1.0, 1.0);
        let px = flat.pixel(0, 0);
        assert_eq!(px[3], 255);
        // 128 + 255·(127/255) ≈ 255·… expected 128 + 127 = 255? No: straight
        // red 255 at 50% over white = (255·0.5 + 255·0.5) in premultiplied
        // terms: stored 128 red + white 255·(127/255) → 128+127 = 255.
        assert!(px[0] > 240, "red stays bright over white: {:?}", px);
        assert!(px[1] > 110 && px[1] < 130, "white shows through: {:?}", px);
        let dark = flatten_on_matte(&composite, 0.0, 0.0, 0.0);
        let px = dark.pixel(0, 0);
        assert!(px[0] <= 130 && px[0] >= 120, "black matte keeps it dark: {:?}", px);
        assert!(px[1] <= 3, "{:?}", px);
    }

    #[test]
    fn jpeg_round_trips_size_and_quality() {
        let composite = red_on_transparent();
        let data = jpeg_data(&composite, &JpegOptions::default()).expect("encodes");
        assert!(data.len() > 0);
        let decoded = image::load_from_memory(&data).expect("decodes");
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
        // Fully-opaque white pixels survive a high-quality round trip.
        let mut opaque = Bitmap::new(4, 4);
        for pixel in opaque.data.chunks_exact_mut(4) {
            pixel[0] = 250;
            pixel[1] = 250;
            pixel[2] = 250;
            pixel[3] = 255;
        }
        let data = jpeg_data(&opaque, &JpegOptions { quality: 1.0, ..Default::default() }).unwrap();
        let decoded = image::load_from_memory(&data).unwrap().to_rgba8();
        let px = decoded.get_pixel(1, 1);
        assert!(px[0] > 230, "high quality keeps near-white: {:?}", px);
    }

    #[test]
    fn preview_scales_to_a_thousand_pixels() {
        let composite = Bitmap::new(2400, 1200);
        let preview = preview_thumbnail(&composite);
        assert_eq!(preview.width, 1000);
        assert_eq!(preview.height, 500);
        let small = Bitmap::new(800, 600);
        assert_eq!(preview_thumbnail(&small).width, 800, "small canvases pass through");
    }

    #[test]
    fn png_encodes_with_alpha() {
        let composite = red_on_transparent();
        let data = png_data(&composite).expect("encodes");
        let decoded = image::load_from_memory(&data).expect("decodes").to_rgba8();
        let px = decoded.get_pixel(0, 0);
        assert_eq!(px[3], 128, "alpha survives PNG");
        assert_eq!(px[0], 255, "straight red channel");
    }

    /// The pHYs chunk lands after IHDR with the resolution in pixels per
    /// meter, and the file still decodes.
    #[test]
    fn png_dpi_injects_a_phys_chunk() {
        let composite = red_on_transparent();
        let data = png_data_with_dpi(&composite, 144.0).expect("encodes");
        // First chunk after the signature is IHDR (13 data bytes).
        assert_eq!(&data[12..16], b"IHDR");
        let ihdr_length = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let phys = &data[8 + 12 + ihdr_length..8 + 12 + ihdr_length + 21];
        assert_eq!(&phys[4..8], b"pHYs");
        let ppm = u32::from_be_bytes([phys[8], phys[9], phys[10], phys[11]]);
        assert_eq!(ppm, (144.0_f64 / 0.0254).round() as u32);
        assert_eq!(phys[16], 1, "unit is the meter");
        let crc = u32::from_be_bytes([phys[17], phys[18], phys[19], phys[20]]);
        assert_eq!(crc, crc32(&phys[4..17]), "chunk checksum valid");
        let decoded = image::load_from_memory(&data).expect("still decodes");
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
    }

    /// The JFIF APP0 density fields carry the resolution in dots per inch.
    #[test]
    fn jpeg_dpi_patches_the_jfif_density() {
        let composite = red_on_transparent();
        let data = jpeg_data_with_dpi(&composite, &JpegOptions::default(), 300.0).expect("encodes");
        assert_eq!(data[13], 1, "density unit: inch");
        let density = u16::from_be_bytes([data[14], data[15]]);
        assert_eq!(density, 300);
        let y = u16::from_be_bytes([data[16], data[17]]);
        assert_eq!(y, 300);
        let decoded = image::load_from_memory(&data).expect("still decodes");
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
    }
}
