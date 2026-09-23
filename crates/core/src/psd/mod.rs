//! Port of `Compositor/IO/PSD/*`: a Photoshop `.psd` reader (8-bit RGB only),
//! the PackBits channel coder, vector-mask parsing, and the document builder
//! that maps PSD concepts onto the app model. The `fixture` module is the port
//! of the Swift test-support byte builder, kept so round trips stay byte-exact.

#![allow(dead_code)]

pub mod channel_coder;
pub mod document_builder;
pub mod fixture;
pub mod reader;
pub mod types;
pub mod vector;

pub use channel_coder::PsdChannelCoder;
pub use document_builder::{PsdDocumentBuilder, PsdImport};
pub use fixture::PsdFixture;
pub use reader::{PsdAdjustments, PsdReader};
pub use types::{blend_mode_from_psd, PsdConversion, PsdDocument, PsdError, PsdLayerKind, PsdRecord};
pub use vector::{PsdVector, MAX_SHAPE_PIXELS};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{LayerBlendMode, ShapeKind};
    use crate::geometry::{CGRect, CGSize};
    use crate::raster::{Bitmap, GrayMask};
    use std::collections::HashMap;
    use uuid::Uuid;

    const DEFAULT_PIXEL_BUDGET: i64 = 100_000_000;

    /// A solid-color premultiplied bitmap (`colorImage` in the Swift tests).
    fn color_image(width: usize, height: usize, red: f64, green: f64, blue: f64, alpha: f64) -> Bitmap {
        Bitmap::new_clearing(
            width,
            height,
            [
                (red * alpha * 255.0 + 0.5) as u8,
                (green * alpha * 255.0 + 0.5) as u8,
                (blue * alpha * 255.0 + 0.5) as u8,
                (alpha * 255.0 + 0.5) as u8,
            ],
        )
    }

    fn gray_image(width: usize, height: usize, value: f64) -> GrayMask {
        GrayMask::from_raw(width, height, vec![(value * 255.0 + 0.5) as u8; width * height])
    }

    fn push16(data: &mut Vec<u8>, value: u16) {
        data.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i16(data: &mut Vec<u8>, value: i16) {
        push16(data, value as u16);
    }

    fn push32(data: &mut Vec<u8>, value: u32) {
        data.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i32(data: &mut Vec<u8>, value: i32) {
        push32(data, value as u32);
    }

    fn header(version: u16, width: u32, height: u32, depth: u16, mode: u16) -> Vec<u8> {
        let mut data = b"8BPS".to_vec();
        push16(&mut data, version);
        data.extend_from_slice(&[0; 6]);
        push16(&mut data, 3);
        push32(&mut data, height);
        push32(&mut data, width);
        push16(&mut data, depth);
        push16(&mut data, mode);
        data
    }

    #[test]
    fn round_trip_layers_order_visibility_opacity_and_blend() {
        let red = color_image(2, 2, 1.0, 0.0, 0.0, 1.0);
        let blue = color_image(2, 2, 0.0, 0.0, 1.0, 1.0);
        let composite = color_image(4, 4, 0.0, 0.0, 0.0, 0.0);
        let mut bottom = PsdRecord::new(Uuid::new_v4(), "Red");
        bottom.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        bottom.image = Some(red);
        bottom.opacity = 0.5;
        bottom.blend_key = "mul ".to_string();
        let mut top = PsdRecord::new(Uuid::new_v4(), "Blue");
        top.bounds = CGRect::new(2.0, 0.0, 2.0, 2.0);
        top.image = Some(blue);
        top.is_visible = false;
        let document = PsdDocument {
            width: 4,
            height: 4,
            resolution: 144.0,
            layers: vec![bottom, top],
        };
        let data = PsdFixture::data(&document, &composite).unwrap();
        assert_eq!(&data[..4], b"8BPS");
        let document = PsdReader::read(&data).unwrap();
        assert_eq!(document.width, 4);
        assert_eq!(document.height, 4);
        assert_eq!(document.resolution, 144.0);
        let names: Vec<&str> = document.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Red", "Blue"]);
        assert!(document.layers[0].is_visible);
        assert!(!document.layers[1].is_visible);
        assert!((document.layers[0].opacity - 0.5).abs() < 0.01);
        assert_eq!(document.layers[0].blend_key, "mul ");
        assert_eq!(document.layers[0].image.as_ref().unwrap().width, 2);
        let imported = PsdDocumentBuilder::make_import(&document);
        assert!(imported.conversions.is_empty());
        let imported_names: Vec<&str> = imported.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(imported_names, ["Red", "Blue"]);
        assert_eq!(imported.layers[0].blend_mode, LayerBlendMode::Multiply);
        assert_eq!(imported.layers[1].is_visible, false);
    }

    #[test]
    fn round_trip_groups_masks_and_clipping() {
        let fill = color_image(2, 2, 0.0, 1.0, 0.0, 1.0);
        let clipped = color_image(2, 2, 1.0, 1.0, 0.0, 1.0);
        let mask = gray_image(2, 2, 1.0);
        let composite = color_image(4, 4, 0.0, 0.0, 0.0, 0.0);
        let group_id = Uuid::new_v4();
        let mut group = PsdRecord::new(group_id, "Stack");
        group.is_group = true;
        group.blend_key = "pass".to_string();
        let mut base = PsdRecord::new(Uuid::new_v4(), "Base");
        base.parent_id = Some(group_id);
        base.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        base.image = Some(fill);
        base.mask = Some(mask);
        let mut child = PsdRecord::new(Uuid::new_v4(), "Clipped");
        child.parent_id = Some(group_id);
        child.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        child.image = Some(clipped);
        child.clipping = true;
        let document = PsdDocument {
            width: 4,
            height: 4,
            resolution: 72.0,
            layers: vec![group, base, child],
        };
        let data = PsdFixture::data(&document, &composite).unwrap();
        let imported = PsdDocumentBuilder::make_import(&PsdReader::read(&data).unwrap());
        assert!(imported.conversions.is_empty());
        assert!(imported.layers.iter().any(|l| l.is_group && l.name == "Stack"));
        let folder = imported.layers.iter().find(|l| l.is_group).unwrap();
        let imported_base = imported.layers.iter().find(|l| l.name == "Base").unwrap();
        let imported_child = imported.layers.iter().find(|l| l.name == "Clipped").unwrap();
        assert_eq!(imported_base.parent_id, Some(folder.id));
        assert_eq!(imported_child.parent_id, Some(folder.id));
        assert!(imported_base.mask.is_some());
        assert_eq!(imported_child.mask_source_id, Some(imported_base.id));
    }

    #[test]
    fn imported_groups_follow_photoshop_lsct_order() {
        let fill = color_image(2, 2, 0.0, 1.0, 0.0, 1.0);
        let composite = color_image(4, 4, 0.0, 0.0, 0.0, 0.0);
        let group_id = Uuid::new_v4();
        let mut group = PsdRecord::new(group_id, "Stack");
        group.is_group = true;
        let mut child = PsdRecord::new(Uuid::new_v4(), "Base");
        child.parent_id = Some(group_id);
        child.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        child.image = Some(fill);
        let document = PsdDocument {
            width: 4,
            height: 4,
            resolution: 72.0,
            layers: vec![group, child],
        };
        let data = PsdFixture::data(&document, &composite).unwrap();
        assert_eq!(lsct_types(&data), [3, 1]);
        let imported = PsdDocumentBuilder::make_import(&PsdReader::read(&data).unwrap());
        let folder = imported.layers.iter().find(|l| l.is_group).unwrap();
        assert_eq!(folder.name, "Stack");
        assert!(imported
            .layers
            .iter()
            .find(|l| l.name == "Base")
            .map(|l| l.parent_id == Some(folder.id))
            .unwrap_or(false));
    }

    #[test]
    fn oversized_layer_bounds_are_rejected() {
        assert_eq!(
            PsdReader::read(&oversized_layer_file(8, 8, 30_000, 30_000)).unwrap_err(),
            PsdError::TooLarge
        );
        let fill = color_image(20, 20, 1.0, 0.0, 0.0, 1.0);
        let mut layer = PsdRecord::new(Uuid::new_v4(), "Huge");
        layer.bounds = CGRect::new(0.0, 0.0, 20.0, 20.0);
        layer.image = Some(fill.clone());
        let document = PsdDocument {
            width: 20,
            height: 20,
            resolution: 72.0,
            layers: vec![layer],
        };
        let data = PsdFixture::data(&document, &fill).unwrap();
        assert_eq!(
            PsdReader::read_with_budget(&data, 50).unwrap_err(),
            PsdError::TooLarge
        );
    }

    #[test]
    fn unused_spot_channels_are_skipped_before_decode() {
        let pixels = vec![255u8; 4];
        let mut channels: Vec<(i16, Vec<u8>)> = Vec::new();
        for id in [-1i16, 0, 1, 2] {
            channels.push((id, raw_channel(&pixels)));
        }
        // Compression 99 would throw if these planes were unpacked. 52 extras fill the 56-channel cap.
        let bogus = vec![0, 99, 0, 0];
        for id in 3i16..=54 {
            channels.push((id, bogus.clone()));
        }
        let document = PsdReader::read(&layer_file(8, 8, 2, 2, &channels)).unwrap();
        assert_eq!(document.layers.len(), 1);
        assert_eq!(document.layers[0].image.as_ref().unwrap().width, 2);
        assert_eq!(document.layers[0].image.as_ref().unwrap().height, 2);
    }

    #[test]
    fn unsupported_compression_on_color_channels_is_still_rejected() {
        let pixels = vec![255u8; 4];
        let channels: Vec<(i16, Vec<u8>)> = vec![
            (-1, raw_channel(&pixels)),
            (0, vec![0, 99, 0, 0]),
            (1, raw_channel(&pixels)),
            (2, raw_channel(&pixels)),
        ];
        assert_eq!(
            PsdReader::read(&layer_file(8, 8, 2, 2, &channels)).unwrap_err(),
            PsdError::UnsupportedCompression
        );
    }

    /// The Swift test checks file magic through temp files; bytes in memory here.
    #[test]
    fn matches_requires_photoshop_magic() {
        assert!(!PsdReader::matches(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(PsdReader::matches(b"8BPS"));
        assert!(!PsdReader::matches(b"8BP"));
    }

    /// An unsupported key has to be one Photoshop has and Compositor doesn't:
    /// Dissolve scatters pixels by opacity rather than blending, and comes in as Normal.
    #[test]
    fn unsupported_blend_produces_conversion_report() {
        let fill = color_image(2, 2, 1.0, 0.0, 0.0, 1.0);
        let mut layer = PsdRecord::new(Uuid::new_v4(), "Dissolved");
        layer.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        layer.image = Some(fill.clone());
        layer.blend_key = "diss".to_string();
        let document = PsdDocument {
            width: 2,
            height: 2,
            resolution: 72.0,
            layers: vec![layer],
        };
        let data = PsdFixture::data(&document, &fill).unwrap();
        let imported = PsdDocumentBuilder::make_import(&PsdReader::read(&data).unwrap());
        assert!(!imported.conversions.is_empty());
        assert!(imported
            .conversions
            .iter()
            .any(|c| c.layer_name == "Dissolved" && c.message.contains("diss")));
        assert_eq!(imported.layers.first().unwrap().blend_mode, LayerBlendMode::Normal);
    }

    #[test]
    fn soft_light_imports_without_conversion() {
        let fill = color_image(2, 2, 1.0, 0.0, 0.0, 1.0);
        let mut layer = PsdRecord::new(Uuid::new_v4(), "Soft");
        layer.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        layer.image = Some(fill.clone());
        layer.blend_key = "sLit".to_string();
        let document = PsdDocument {
            width: 2,
            height: 2,
            resolution: 72.0,
            layers: vec![layer],
        };
        let data = PsdFixture::data(&document, &fill).unwrap();
        let imported = PsdDocumentBuilder::make_import(&PsdReader::read(&data).unwrap());
        assert!(imported.conversions.is_empty());
        assert_eq!(imported.layers.first().unwrap().blend_mode, LayerBlendMode::SoftLight);
    }

    /// Folders took an opacity of their own in 1.1.6, so it imports onto the folder.
    #[test]
    fn folder_opacity_imports_onto_the_folder() {
        let fill = color_image(2, 2, 0.0, 1.0, 0.0, 1.0);
        let composite = color_image(4, 4, 0.0, 0.0, 0.0, 0.0);
        let group_id = Uuid::new_v4();
        let mut group = PsdRecord::new(group_id, "Stack");
        group.is_group = true;
        group.opacity = 0.5;
        let mut child = PsdRecord::new(Uuid::new_v4(), "Base");
        child.parent_id = Some(group_id);
        child.bounds = CGRect::new(0.0, 0.0, 2.0, 2.0);
        child.image = Some(fill);
        let document = PsdDocument {
            width: 4,
            height: 4,
            resolution: 72.0,
            layers: vec![group, child],
        };
        let data = PsdFixture::data(&document, &composite).unwrap();
        let imported = PsdDocumentBuilder::make_import(&PsdReader::read(&data).unwrap());
        let folder = imported.layers.iter().find(|l| l.is_group).unwrap();
        // Photoshop stores opacity in one byte, so a half-opaque group comes back as 128/255.
        assert!((folder.opacity - 0.5).abs() < 0.01);
        assert!(!imported
            .conversions
            .iter()
            .any(|c| c.layer_name == "Stack" && c.message.contains("opacity")));
    }

    #[test]
    fn unsupported_headers_are_rejected() {
        assert_eq!(
            PsdReader::read(&header(2, 8, 8, 8, 3)).unwrap_err(),
            PsdError::UnsupportedVersion
        );
        assert_eq!(
            PsdReader::read(&header(1, 8, 8, 8, 4)).unwrap_err(),
            PsdError::UnsupportedColorMode
        );
        assert_eq!(
            PsdReader::read(&header(1, 8, 8, 16, 3)).unwrap_err(),
            PsdError::UnsupportedDepth
        );
        assert_eq!(
            PsdReader::read(&header(1, 30_001, 10, 8, 3)).unwrap_err(),
            PsdError::TooLarge
        );
    }

    // MARK: - Vector tests (`PSDRoundTripTests` vector half)

    fn base64_decode(input: &str) -> Vec<u8> {
        fn value(c: u8) -> u32 {
            match c {
                b'A'..=b'Z' => (c - b'A') as u32,
                b'a'..=b'z' => (c - b'a' + 26) as u32,
                b'0'..=b'9' => (c - b'0' + 52) as u32,
                b'+' => 62,
                b'/' => 63,
                _ => 0,
            }
        }
        let bytes: Vec<u8> = input.bytes().filter(|b| !b"= \r\n".contains(b)).collect();
        let mut out = Vec::with_capacity(bytes.len() * 3 / 4 + 3);
        for chunk in bytes.chunks(4) {
            let mut n = 0u32;
            for (i, &c) in chunk.iter().enumerate() {
                n |= value(c) << (18 - 6 * i);
            }
            out.push((n >> 16) as u8);
            if chunk.len() > 1 {
                out.push((n >> 8) as u8);
            }
            if chunk.len() > 2 {
                out.push(n as u8);
            }
        }
        out
    }

    /// `PSDVectorFixtures`.
    mod vector_fixtures {
        use super::base64_decode;
        use crate::geometry::CGSize;
        use std::collections::HashMap;

        pub fn canvas() -> CGSize {
            CGSize::new(1920.0, 1080.0)
        }

        pub fn circle() -> HashMap<String, Vec<u8>> {
            HashMap::from([
                (
                    "vmsk".to_string(),
                    base64_decode("AAAAAwAAAAAABgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAABAAEAAAAAAAAAAAAAAAAAAAAAAAAAAQDHWSAAXn4FAMdZIABSZmYAx1kgAEZOxwABALX4sAA8iIgAoHlcADyIiACK+gkAPIiIAAEAeZmZAEZOxwB5mZkAUmZmAHmZmQBefgUAAQCK+gkAaEREAKB5XABoREQAtfiwAGhERAAA"),
                ),
                (
                    "SoCo".to_string(),
                    base64_decode("AAAAEAAAAAEAAAAAAABudWxsAAAAAQAAAABDbHIgT2JqYwAAAAEAAAAAAABSR0JDAAAAAwAAAABSZCAgZG91YgAAAAAAAAAAAAAAAEdybiBkb3ViQFuAAAAAAAAAAAAAQmwgIGRvdWJAb+AAAAAAAA=="),
                ),
                (
                    "vstk".to_string(),
                    base64_decode("AAAAEAAAAAEAAAAAAAtzdHJva2VTdHlsZQAAABAAAAASc3Ryb2tlU3R5bGVWZXJzaW9ubG9uZwAAAAIAAAANc3Ryb2tlRW5hYmxlZGJvb2wAAAAAC2ZpbGxFbmFibGVkYm9vbAEAAAAUc3Ryb2tlU3R5bGVMaW5lV2lkdGhVbnRGI1B4bEAjvQfpGNNeAAAAGXN0cm9rZVN0eWxlTGluZURhc2hPZmZzZXRVbnRGI1BudAAAAAAAAAAAAAAAFXN0cm9rZVN0eWxlTWl0ZXJMaW1pdGRvdWJAWQAAAAAAAAAAABZzdHJva2VTdHlsZUxpbmVDYXBUeXBlZW51bQAAABZzdHJva2VTdHlsZUxpbmVDYXBUeXBlAAAAEnN0cm9rZVN0eWxlQnV0dENhcAAAABdzdHJva2VTdHlsZUxpbmVKb2luVHlwZWVudW0AAAAXc3Ryb2tlU3R5bGVMaW5lSm9pblR5cGUAAAAUc3Ryb2tlU3R5bGVNaXRlckpvaW4AAAAYc3Ryb2tlU3R5bGVMaW5lQWxpZ25tZW50ZW51bQAAABhzdHJva2VTdHlsZUxpbmVBbGlnbm1lbnQAAAAWc3Ryb2tlU3R5bGVBbGlnbkNlbnRlcgAAABRzdHJva2VTdHlsZVNjYWxlTG9ja2Jvb2wAAAAAF3N0cm9rZVN0eWxlU3Ryb2tlQWRqdXN0Ym9vbAAAAAAWc3Ryb2tlU3R5bGVMaW5lRGFzaFNldFZsTHMAAAAAAAAAFHN0cm9rZVN0eWxlQmxlbmRNb2RlZW51bQAAAABCbG5NAAAAAE5ybWwAAAASc3Ryb2tlU3R5bGVPcGFjaXR5VW50RiNQcmNAWQAAAAAAAAAAABJzdHJva2VTdHlsZUNvbnRlbnRPYmpjAAAAAQAAAAAAD3NvbGlkQ29sb3JMYXllcgAAAAEAAAAAQ2xyIE9iamMAAAABAAAAAAAAUkdCQwAAAAMAAAAAUmQgIGRvdWJAb+AAAAAAAAAAAABHcm4gZG91YkBv4AAAAAAAAAAAAEJsICBkb3ViAAAAAAAAAAAAAAAVc3Ryb2tlU3R5bGVSZXNvbHV0aW9uZG91YkBSAAAAAAAA"),
                ),
            ])
        }

        pub fn rectangle() -> HashMap<String, Vec<u8>> {
            HashMap::from([
                (
                    "vmsk".to_string(),
                    base64_decode("AAAAAwAAAAAABgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAABAAEAAAAAAAAAAAAAAAAAAAAAAAAAAQAkREQA1CIiACRERADUIiIAJEREANQiIgABAE9oSwDUIiIAT2hLANQiIgBPaEsA1CIiAAEAT2hLAH4AAABPaEsAfgAAAE9oSwB+AAAAAQAkREQAfgAAACRERAB+AAAAJEREAH4AAAAA"),
                ),
                (
                    "SoCo".to_string(),
                    base64_decode("AAAAEAAAAAEAAAAAAABudWxsAAAAAQAAAABDbHIgT2JqYwAAAAEAAAAAAABSR0JDAAAAAwAAAABSZCAgZG91YgAAAAAAAAAAAAAAAEdybiBkb3ViAAAAAAAAAAAAAAAAQmwgIGRvdWIAAAAAAAAAAA=="),
                ),
                (
                    "vstk".to_string(),
                    base64_decode("AAAAEAAAAAEAAAAAAAtzdHJva2VTdHlsZQAAABAAAAASc3Ryb2tlU3R5bGVWZXJzaW9ubG9uZwAAAAIAAAANc3Ryb2tlRW5hYmxlZGJvb2wBAAAAC2ZpbGxFbmFibGVkYm9vbAEAAAAUc3Ryb2tlU3R5bGVMaW5lV2lkdGhVbnRGI1B4bEAjvQfpGNNeAAAAGXN0cm9rZVN0eWxlTGluZURhc2hPZmZzZXRVbnRGI1BudAAAAAAAAAAAAAAAFXN0cm9rZVN0eWxlTWl0ZXJMaW1pdGRvdWJAWQAAAAAAAAAAABZzdHJva2VTdHlsZUxpbmVDYXBUeXBlZW51bQAAABZzdHJva2VTdHlsZUxpbmVDYXBUeXBlAAAAEnN0cm9rZVN0eWxlQnV0dENhcAAAABdzdHJva2VTdHlsZUxpbmVKb2luVHlwZWVudW0AAAAXc3Ryb2tlU3R5bGVMaW5lSm9pblR5cGUAAAAUc3Ryb2tlU3R5bGVNaXRlckpvaW4AAAAYc3Ryb2tlU3R5bGVMaW5lQWxpZ25tZW50ZW51bQAAABhzdHJva2VTdHlsZUxpbmVBbGlnbm1lbnQAAAAWc3Ryb2tlU3R5bGVBbGlnbkNlbnRlcgAAABRzdHJva2VTdHlsZVNjYWxlTG9ja2Jvb2wAAAAAF3N0cm9rZVN0eWxlU3Ryb2tlQWRqdXN0Ym9vbAAAAAAWc3Ryb2tlU3R5bGVMaW5lRGFzaFNldFZsTHMAAAAAAAAAFHN0cm9rZVN0eWxlQmxlbmRNb2RlZW51bQAAAABCbG5NAAAAAE5ybWwAAAASc3Ryb2tlU3R5bGVPcGFjaXR5VW50RiNQcmNAWQAAAAAAAAAAABJzdHJva2VTdHlsZUNvbnRlbnRPYmpjAAAAAQAAAAAAD3NvbGlkQ29sb3JMYXllcgAAAAEAAAAAQ2xyIE9iamMAAAABAAAAAAAAUkdCQwAAAAMAAAAAUmQgIGRvdWJAb+AAAAAAAAAAAABHcm4gZG91YkBv4AAAAAAAAAAAAEJsICBkb3ViAAAAAAAAAAAAAAAVc3Ryb2tlU3R5bGVSZXNvbHV0aW9uZG91YkBSAAAAAAAA"),
                ),
            ])
        }
    }

    fn vector_mask(canvas: CGSize, corners: &[(f64, f64)]) -> Vec<u8> {
        let mut data = Vec::new();
        push32(&mut data, 3);
        push32(&mut data, 0);
        push_i16(&mut data, 6);
        data.extend_from_slice(&[0; 24]);
        push_i16(&mut data, 8);
        data.extend_from_slice(&[0; 24]);
        push_i16(&mut data, 0);
        push_i16(&mut data, corners.len() as i16);
        data.extend_from_slice(&[0; 22]);
        for (px, py) in corners {
            let y = ((py / canvas.height) * 16_777_216.0) as i32;
            let x = ((px / canvas.width) * 16_777_216.0) as i32;
            push_i16(&mut data, 1);
            for _ in 0..3 {
                push_i32(&mut data, y);
                push_i32(&mut data, x);
            }
        }
        data
    }

    fn color_descriptor(red: f64, green: f64, blue: f64) -> Vec<u8> {
        let mut data = b"RGBC".to_vec();
        for (key, value) in [("Rd  ", red), ("Grn ", green), ("Bl  ", blue)] {
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(b"doub");
            data.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        data
    }

    fn solid_color(red: f64, green: f64, blue: f64) -> Vec<u8> {
        color_descriptor(red, green, blue)
    }

    fn stroke_style(fill: bool, stroke: bool, width: f64, red: f64, green: f64, blue: f64) -> Vec<u8> {
        let mut data = Vec::new();
        for (key, value) in [("strokeEnabled", stroke), ("fillEnabled", fill)] {
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(b"bool");
            data.push(value as u8);
        }
        data.extend_from_slice(b"strokeStyleLineWidth");
        data.extend_from_slice(b"UntF");
        data.extend_from_slice(b"#Pxl");
        data.extend_from_slice(&width.to_bits().to_be_bytes());
        data.extend_from_slice(&color_descriptor(red, green, blue));
        data
    }

    fn origination_data(kind: u32, rect: CGRect, radii: &[f64]) -> Vec<u8> {
        let mut data = Vec::new();
        let append_unit = |data: &mut Vec<u8>, key: &str, value: f64| {
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(b"UntF");
            data.extend_from_slice(b"#Pxl");
            data.extend_from_slice(&value.to_bits().to_be_bytes());
        };
        data.extend_from_slice(b"keyOriginType");
        data.extend_from_slice(b"long");
        push32(&mut data, kind);
        data.extend_from_slice(b"keyOriginShapeBBox");
        append_unit(&mut data, "Left", rect.min_x());
        append_unit(&mut data, "Top ", rect.min_y());
        append_unit(&mut data, "Rght", rect.max_x());
        append_unit(&mut data, "Btom", rect.max_y());
        if radii.len() == 4 {
            data.extend_from_slice(b"keyOriginRRectRadii");
            for (key, value) in [
                ("topLeft", radii[0]),
                ("topRight", radii[1]),
                ("bottomRight", radii[2]),
                ("bottomLeft", radii[3]),
            ] {
                append_unit(&mut data, key, value);
            }
        }
        data
    }

    #[test]
    fn vector_mask_is_rasterized_with_fill_and_stroke() {
        let canvas = CGSize::new(200.0, 200.0);
        let mut extra: HashMap<String, Vec<u8>> = HashMap::new();
        extra.insert(
            "vmsk".to_string(),
            vector_mask(
                canvas,
                &[(120.0, 30.0), (120.0, 80.0), (20.0, 80.0), (20.0, 30.0)],
            ),
        );
        extra.insert("SoCo".to_string(), solid_color(0.0, 110.0, 255.0));
        extra.insert(
            "vstk".to_string(),
            stroke_style(true, false, 1.0, 255.0, 255.0, 0.0),
        );
        let raster = PsdVector::raster(&extra, canvas, DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert!(raster.bounds.size.width >= 99.0 && raster.bounds.size.height >= 49.0);
        assert!(raster.image.width >= 99 && raster.image.height >= 49);
        extra.insert(
            "vstk".to_string(),
            stroke_style(true, true, 10.0, 255.0, 255.0, 0.0),
        );
        extra.insert("SoCo".to_string(), solid_color(0.0, 0.0, 0.0));
        let stroked = PsdVector::raster(&extra, canvas, DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert!(stroked.bounds.size.width > raster.bounds.size.width);
        assert!(stroked.bounds.size.height > raster.bounds.size.height);
    }

    #[test]
    fn photoshop_shape_extras_rasterize_in_place() {
        let canvas = vector_fixtures::canvas();
        let circle = PsdVector::raster(&vector_fixtures::circle(), canvas, DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert!((circle.bounds.mid_x() - 618.0).abs() < 8.0);
        assert!((circle.bounds.mid_y() - 677.0).abs() < 8.0);
        assert!((circle.bounds.size.width - 328.0).abs() < 12.0);
        assert!((circle.bounds.size.height - 328.0).abs() < 12.0);
        let rectangle = PsdVector::raster(&vector_fixtures::rectangle(), canvas, DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert!(rectangle.bounds.size.width > 640.0);
        assert!(rectangle.bounds.size.height > 170.0);
        assert!((rectangle.bounds.mid_x() - 1268.0).abs() < 20.0);
        assert!((rectangle.bounds.mid_y() - 244.0).abs() < 20.0);
    }

    #[test]
    fn fill_ellipse_imports_as_a_live_shape() {
        let mut extra = vector_fixtures::circle();
        extra.insert(
            "vogk".to_string(),
            origination_data(5, CGRect::new(454.0, 513.0, 328.0, 328.0), &[]),
        );
        let live = PsdVector::live(&extra, vector_fixtures::canvas(), DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert_eq!(live.style.kind, ShapeKind::Ellipse);
        assert!((live.style.green - 110.0 / 255.0).abs() < 0.01);
        assert!((live.style.blue - 1.0).abs() < 0.01);
        assert!(live.notes.is_empty());
        assert!((live.bounds.min_x() - 454.0).abs() < 1.0 && (live.bounds.size.width - 328.0).abs() < 1.0);
        let mut record = PsdRecord::new(Uuid::new_v4(), "cercle-bleu");
        record.kind = PsdLayerKind::Vector;
        record.image = Some(live.image.clone());
        record.bounds = live.bounds;
        record.shape = Some(live.style);
        let imported = PsdDocumentBuilder::make_import(&PsdDocument {
            width: 1920,
            height: 1080,
            resolution: 72.0,
            layers: vec![record],
        });
        let layer = imported.layers.first().unwrap();
        // The Rust model keeps no per-shape pixel cache: a live shape is the
        // style plus the layer's own asset (the Swift `liveShape` identity).
        assert_eq!(layer.shape.as_ref().unwrap().kind, ShapeKind::Ellipse);
        assert!(layer.asset.is_some());
        assert!(imported.conversions.is_empty());
        assert_eq!(layer.asset.as_ref().unwrap().image.width, 328);
        assert_eq!(layer.asset.as_ref().unwrap().image.height, 328);
    }

    #[test]
    fn stroked_rectangle_imports_as_a_live_shape_and_reports_the_stroke() {
        let mut extra = vector_fixtures::rectangle();
        extra.insert(
            "vogk".to_string(),
            origination_data(
                2,
                CGRect::new(945.0, 153.0, 646.0, 182.0),
                &[0.0, 0.0, 0.0, 0.0],
            ),
        );
        let live = PsdVector::live(&extra, vector_fixtures::canvas(), DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert_eq!(live.style.kind, ShapeKind::Rectangle);
        assert_eq!(live.style.corner_radius, 0.0);
        assert!(live.notes.iter().any(|note| note.contains("stroke")));
        let mut record = PsdRecord::new(Uuid::new_v4(), "rectangle-contour-jaune");
        record.kind = PsdLayerKind::Vector;
        record.image = Some(live.image);
        record.bounds = live.bounds;
        record.shape = Some(live.style);
        record.shape_notes = live.notes;
        let imported = PsdDocumentBuilder::make_import(&PsdDocument {
            width: 1920,
            height: 1080,
            resolution: 72.0,
            layers: vec![record],
        });
        let layer = imported.layers.first().unwrap();
        assert_eq!(layer.shape.as_ref().unwrap().kind, ShapeKind::Rectangle);
        assert!(imported
            .conversions
            .iter()
            .any(|c| c.layer_name == "rectangle-contour-jaune" && c.message.contains("stroke")));
        assert!(!imported.conversions.iter().any(|c| c.message.contains("rasterized")));
    }

    #[test]
    fn four_sharp_corners_infer_a_rectangle_without_origination() {
        let canvas = CGSize::new(200.0, 200.0);
        let mut extra: HashMap<String, Vec<u8>> = HashMap::new();
        extra.insert(
            "vmsk".to_string(),
            vector_mask(
                canvas,
                &[(120.0, 30.0), (120.0, 80.0), (20.0, 80.0), (20.0, 30.0)],
            ),
        );
        extra.insert("SoCo".to_string(), solid_color(0.0, 110.0, 255.0));
        extra.insert(
            "vstk".to_string(),
            stroke_style(true, false, 1.0, 255.0, 255.0, 0.0),
        );
        let live = PsdVector::live(&extra, canvas, DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .unwrap();
        assert_eq!(live.style.kind, ShapeKind::Rectangle);
        assert!(live.notes.is_empty());
        assert!(live.bounds.size.width >= 99.0 && live.bounds.size.height >= 49.0);
    }

    #[test]
    fn huge_origination_size_is_rejected_without_trapping() {
        let mut extra: HashMap<String, Vec<u8>> = HashMap::new();
        extra.insert(
            "vogk".to_string(),
            origination_data(5, CGRect::new(0.0, 0.0, 1e20, 1e20), &[]),
        );
        extra.insert("SoCo".to_string(), solid_color(0.0, 110.0, 255.0));
        extra.insert(
            "vstk".to_string(),
            stroke_style(true, false, 1.0, 255.0, 255.0, 0.0),
        );
        assert_eq!(
            PsdVector::live(&extra, vector_fixtures::canvas(), DEFAULT_PIXEL_BUDGET).unwrap_err(),
            PsdError::TooLarge
        );
    }

    #[test]
    fn non_finite_origination_size_is_ignored() {
        let mut extra: HashMap<String, Vec<u8>> = HashMap::new();
        extra.insert(
            "vogk".to_string(),
            origination_data(5, CGRect::new(10.0, 10.0, f64::INFINITY, 100.0), &[]),
        );
        extra.insert("SoCo".to_string(), solid_color(0.0, 110.0, 255.0));
        extra.insert(
            "vstk".to_string(),
            stroke_style(true, false, 1.0, 255.0, 255.0, 0.0),
        );
        assert!(PsdVector::live(&extra, vector_fixtures::canvas(), DEFAULT_PIXEL_BUDGET)
            .unwrap()
            .is_none());
    }

    #[test]
    fn huge_stroke_width_is_rejected_without_trapping() {
        let canvas = CGSize::new(200.0, 200.0);
        let mut extra: HashMap<String, Vec<u8>> = HashMap::new();
        extra.insert(
            "vmsk".to_string(),
            vector_mask(
                canvas,
                &[(120.0, 30.0), (120.0, 80.0), (20.0, 80.0), (20.0, 30.0)],
            ),
        );
        extra.insert("SoCo".to_string(), solid_color(0.0, 0.0, 0.0));
        extra.insert(
            "vstk".to_string(),
            stroke_style(true, true, 1e20, 255.0, 255.0, 0.0),
        );
        assert_eq!(
            PsdVector::raster(&extra, canvas, DEFAULT_PIXEL_BUDGET).unwrap_err(),
            PsdError::TooLarge
        );
    }

    // MARK: - Raw file builders from the Swift tests

    fn raw_channel(plane: &[u8]) -> Vec<u8> {
        let mut data = vec![0, 0];
        data.extend_from_slice(plane);
        data
    }

    fn lsct_types(data: &[u8]) -> Vec<u32> {
        let mut types = Vec::new();
        let mut search = 0usize;
        while let Some(found) = super::types::find_offset(data, b"lsct", search) {
            let upper = found + 4;
            if upper + 8 > data.len() {
                break;
            }
            let type_at = upper + 4;
            types.push(super::types::u32_at(data, type_at));
            search = upper;
        }
        types
    }

    fn oversized_layer_file(width: u32, height: u32, layer_width: i32, layer_height: i32) -> Vec<u8> {
        layer_file(
            width,
            height,
            layer_width,
            layer_height,
            &[
                (-1, vec![0, 0]),
                (0, vec![0, 0]),
                (1, vec![0, 0]),
                (2, vec![0, 0]),
            ],
        )
    }

    fn layer_file(
        canvas_width: u32,
        canvas_height: u32,
        layer_width: i32,
        layer_height: i32,
        channels: &[(i16, Vec<u8>)],
    ) -> Vec<u8> {
        let mut data = header(1, canvas_width, canvas_height, 8, 3);
        push32(&mut data, 0);
        push32(&mut data, 0);
        let mut records = Vec::new();
        push_i16(&mut records, 1);
        push_i32(&mut records, 0);
        push_i32(&mut records, 0);
        push_i32(&mut records, layer_height);
        push_i32(&mut records, layer_width);
        push16(&mut records, channels.len() as u16);
        let mut payloads = Vec::new();
        for (id, payload) in channels {
            push_i16(&mut records, *id);
            push32(&mut records, payload.len() as u32);
            payloads.extend_from_slice(payload);
        }
        records.extend_from_slice(b"8BIMnorm");
        records.extend_from_slice(&[255, 0, 0, 0]);
        push32(&mut records, 12);
        push32(&mut records, 0);
        push32(&mut records, 0);
        records.push(3);
        records.extend_from_slice(b"Big");
        let mut info = Vec::new();
        push32(&mut info, (records.len() + payloads.len()) as u32);
        info.extend_from_slice(&records);
        info.extend_from_slice(&payloads);
        push32(&mut info, 0);
        push32(&mut data, info.len() as u32);
        data.extend_from_slice(&info);
        data
    }
}
