//! Port of `IO/RawImporter.swift`: camera-RAW import. `RawImporter.matches`
//! gates by UTType conformance to `.rawImage` (~30 formats); `develop` runs
//! CIRAWFilter's default (as-shot white balance, camera color space -> sRGB,
//! RGBA8). The Rust equivalent runs rawler's default develop — the same
//! as-shot pipeline — into an 8-bit sRGB bitmap that flows through the
//! regular import pipeline. The develop sliders (exposure/Kelvin/tint) and
//! the RawDevelop sheet are a separate, unported surface.

use std::path::Path;

use crate::raster::Bitmap;

/// `RawImporter.matches`: the file extension is one of the camera-RAW
/// formats. The list mirrors rawler's supported decoders (upper-case, as
/// Swift's UTType raw-image conformance list).
pub fn is_raw(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_uppercase())
        .map(|ext| rawler::decoders::supported_extensions().contains(&&*ext))
        .unwrap_or(false)
}

/// `RawImporter.develop`: decode with the default develop — as-shot white
/// balance and the camera color matrix into 8-bit sRGB (the Swift
/// CIRAWFilter output is RGBA8 sRGB).
pub fn decode_raw(path: &Path) -> Option<Bitmap> {
    let image =
        rawler::analyze::raw_to_srgb(path, &rawler::decoders::RawDecodeParams::default()).ok()?;
    let rgba = image.to_rgba8();
    let mut bitmap = Bitmap::new(rgba.width() as usize, rgba.height() as usize);
    bitmap.data.copy_from_slice(rgba.as_raw());
    Some(bitmap)
}

#[cfg(test)]
fn decode_raw_error(path: &Path) -> Option<String> {
    rawler::analyze::raw_to_srgb(path, &rawler::decoders::RawDecodeParams::default())
        .err()
        .map(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_whitelist_by_extension() {
        // The Swift UTType list: case-insensitive, no content sniffing.
        assert!(is_raw(Path::new("photo.dng")));
        assert!(is_raw(Path::new("IMG_0001.CR3")));
        assert!(is_raw(Path::new("DSC.NEF")));
        assert!(is_raw(Path::new("a.RAF")));
        assert!(is_raw(Path::new("x.3fr")));
        assert!(is_raw(Path::new("no.extension.raw")));
        // Non-RAW formats stay out of the importer.
        assert!(!is_raw(Path::new("photo.png")));
        assert!(!is_raw(Path::new("photo.jpg")));
        assert!(!is_raw(Path::new("photo.psd")));
        assert!(!is_raw(Path::new("noextension")));
        assert!(!is_raw(Path::new("")));
    }

    /// A minimal uncompressed CFA DNG (TIFF container) decodes through the
    /// full develop pipeline into a non-empty sRGB bitmap.
    #[test]
    fn minimal_dng_develops_to_srgb() {
        let path = std::env::temp_dir().join("compositor-raw-test.dng");
        std::fs::write(&path, minimal_cfa_dng()).expect("writes fixture");
        let decoded = match decode_raw(&path) {
            Some(bitmap) => bitmap,
            None => panic!("rawler decodes the DNG: {:?}", decode_raw_error(&path)),
        };
        assert_eq!((decoded.width, decoded.height), (16, 16));
        // The develop output is opaque sRGB (RAW has no alpha).
        assert_eq!(decoded.pixel(0, 0)[3], 255);
        let _ = std::fs::remove_file(&path);
    }

    /// The import pipeline develops a RAW file into a layer: the DNG flows
    /// through `import_images` like any other image (RawImporter is inside
    /// the regular importer, not a separate flow).
    #[test]
    fn import_images_accepts_a_dng() {
        let mut session = crate::session::EditorSession::new();
        let path = std::env::temp_dir().join("compositor-raw-import.dng");
        std::fs::write(&path, minimal_cfa_dng()).expect("writes fixture");
        let failures = session.import_images(&[path.clone()], None);
        assert!(failures.is_empty(), "{failures:?}");
        let document = session.document.as_ref().expect("import created the canvas");
        assert_eq!(document.layers.len(), 1);
        let image = document.layers[0]
            .asset
            .as_ref()
            .expect("the DNG became a layer")
            .image
            .as_ref();
        assert_eq!((image.width, image.height), (16, 16));
        let _ = std::fs::remove_file(&path);
    }

    /// 16x16 RGGB CFA, 16-bit, uncompressed, with the DNG-version and CFA
    /// tags rawler's DNG decoder requires (the demosaic needs more than a
    /// couple of rows to interpolate). TIFF layout: header, strip data,
    /// IFD (sorted by tag), then the out-of-line values the 4-byte entry
    /// fields cannot hold inline (the model string and the 4-short CFA
    /// pattern).
    pub(crate) fn minimal_cfa_dng() -> Vec<u8> {
        const SIDE: usize = 16;
        let pixels: Vec<u16> = (0..SIDE * SIDE)
            .map(|i| (4000u32 + (i as u32 * 2117) % 60000) as u16)
            .collect();
        let pixel_bytes: Vec<u8> = pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
        const HEADER: usize = 8;
        const ENTRIES: u16 = 13;
        let ifd_offset = (HEADER + pixel_bytes.len()) as u32;
        let ifd_size = 2 + ENTRIES as usize * 12 + 4;
        let values_offset = ifd_offset as usize + ifd_size;
        let model = b"TestCam\0";
        let model_offset = values_offset as u32;
        let cfa_offset = model_offset + model.len() as u32;

        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"II");
        out.extend_from_slice(&42u16.to_le_bytes());
        out.extend_from_slice(&ifd_offset.to_le_bytes());
        out.extend_from_slice(&pixel_bytes);

        let mut ifd: Vec<u8> = Vec::new();
        ifd.extend_from_slice(&ENTRIES.to_le_bytes());
        // A SHORT entry packs two values per 4-byte field (low half first);
        // anything larger moves to the out-of-line values area.
        let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
            ifd.extend_from_slice(&tag.to_le_bytes());
            ifd.extend_from_slice(&kind.to_le_bytes());
            ifd.extend_from_slice(&count.to_le_bytes());
            ifd.extend_from_slice(&value.to_le_bytes());
        };
        let shorts = |a: u16, b: u16| a as u32 | ((b as u32) << 16);
        entry(256, 3, 1, SIDE as u32); // ImageWidth
        entry(257, 3, 1, SIDE as u32); // ImageLength
        entry(258, 3, 1, 16); // BitsPerSample
        entry(259, 3, 1, 1); // Compression = none
        entry(262, 3, 1, 32803); // PhotometricInterpretation = CFA
        entry(273, 4, 1, HEADER as u32); // StripOffsets
        entry(277, 3, 1, 1); // SamplesPerPixel
        entry(278, 3, 1, SIDE as u32); // RowsPerStrip
        entry(279, 4, 1, pixel_bytes.len() as u32); // StripByteCounts
        entry(33421, 3, 2, shorts(2, 2)); // CFARepeatPatternDim
        entry(33422, 3, 4, cfa_offset); // CFAPattern (out of line)
        // DNGVersion = 1.4.0.0: four BYTEs inline, low byte first.
        entry(50706, 1, 4, 1 | (4 << 8));
        entry(50708, 2, model.len() as u32, model_offset); // UniqueCameraModel
        ifd.extend_from_slice(&0u32.to_le_bytes()); // next IFD = none

        out.extend_from_slice(&ifd);
        out.extend_from_slice(model);
        for v in [0u16, 1, 1, 2] {
            out.extend_from_slice(&v.to_le_bytes()); // CFAPattern = RGGB
        }
        out
    }
}
