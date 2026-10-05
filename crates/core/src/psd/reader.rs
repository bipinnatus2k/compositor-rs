//! Port of `Compositor/IO/PSD/PSDReader.swift`: reads Photoshop `.psd` files
//! from Adobe's *Photoshop File Formats Specification* (2019 HTML edition:
//! File Header, Color Mode Data, Image Resources, Layer and Mask Information,
//! Image Data). Original implementation of the 8BPS header, layer records,
//! PackBits, and additional layer info. Not copied, transcribed, or adapted
//! from GIMP, psd-tools, or any other GPL-licensed PSD reader.

use std::collections::HashMap;

use uuid::Uuid;

use crate::adjust::hue_saturation::{ColorRange, RangeAdjustment};
use crate::adjustment::{
    AdjustmentKind, CurvePoint, CurvesSettings, HueSaturationSettings, LayerAdjustment, LevelRange,
    LevelsSettings,
};
use crate::geometry::{CGRect, CGSize};
use crate::raster::{Bitmap, GrayMask};

use super::channel_coder::PsdChannelCoder;
use super::types::{PsdDocument, PsdError, PsdLayerKind, PsdRecord, i16_at, u16_at, u32_at};
use super::vector::PsdVector;

pub struct PsdReader;

impl PsdReader {
    /// `PSDReader.matches(_ data:)`.
    pub fn matches(data: &[u8]) -> bool {
        data.len() >= 4 && &data[..4] == b"8BPS"
    }

    /// `PSDReader.read(_ data:)` with the default 100-megapixel budget.
    pub fn read(data: &[u8]) -> Result<PsdDocument, PsdError> {
        Self::read_with_budget(data, 100_000_000)
    }

    /// `PSDReader.read(_ data:remainingPixels:)`.
    pub fn read_with_budget(data: &[u8], remaining_pixels: i64) -> Result<PsdDocument, PsdError> {
        let mut cursor = PsdCursor { data, offset: 0 };
        if cursor.string(4)? != "8BPS" {
            return Err(PsdError::Unreadable);
        }
        let version = cursor.u16()?;
        if version != 1 {
            return Err(PsdError::UnsupportedVersion);
        }
        cursor.skip(6)?;
        let _channels = cursor.u16()?;
        let canvas_height = cursor.u32()? as i64;
        let canvas_width = cursor.u32()? as i64;
        let depth = cursor.u16()?;
        let mode = cursor.u16()?;
        if !(1..=30_000).contains(&canvas_width)
            || !(1..=30_000).contains(&canvas_height)
            || canvas_width * canvas_height > 100_000_000
        {
            return Err(PsdError::TooLarge);
        }
        if depth != 8 {
            return Err(PsdError::UnsupportedDepth);
        }
        if mode != 3 {
            return Err(PsdError::UnsupportedColorMode);
        }
        // Color mode data (empty for RGB).
        let color_mode_length = cursor.u32()? as usize;
        cursor.skip(color_mode_length)?;
        let resources_length = cursor.u32()? as usize;
        let resources_end = cursor.offset + resources_length;
        let mut resolution = 72.0f64;
        while cursor.offset + 12 <= resources_end {
            let signature = cursor.string(4)?;
            if signature != "8BIM" {
                break;
            }
            let id = cursor.u16()?;
            let name_length = cursor.u8()? as usize;
            cursor.skip(name_length)?;
            // Pascal names are padded to even length.
            if (name_length + 1) % 2 == 1 {
                cursor.skip(1)?;
            }
            let length = cursor.u32()? as usize;
            let data_start = cursor.offset;
            if id == 1005 && length >= 4 {
                // ResolutionInfo: fixed-point 16.16 horizontal density.
                resolution = cursor.u32()? as f64 / 65536.0;
                if !resolution.is_finite() || resolution < 1.0 {
                    resolution = 72.0;
                }
                resolution = resolution.min(9600.0).max(1.0);
            }
            cursor.offset = data_start + length;
            if length % 2 == 1 {
                cursor.skip(1)?;
            }
        }
        cursor.offset = resources_end;
        // Layer and mask information section.
        let layer_section = cursor.u32()? as usize;
        let _layer_section_end = cursor.offset + layer_section;
        if layer_section < 4 {
            return Ok(PsdDocument {
                width: canvas_width as usize,
                height: canvas_height as usize,
                resolution,
                layers: Vec::new(),
            });
        }
        let _layer_info_length = cursor.u32()?;
        let raw_count = cursor.i16()?;
        let count = raw_count.unsigned_abs() as usize;
        if count > 10_000 {
            return Err(PsdError::TooLarge);
        }
        let mut raw: Vec<RawLayer> = Vec::with_capacity(count);
        for _ in 0..count {
            raw.push(Self::read_record(&mut cursor)?);
        }
        let mut used_pixels: i64 = 0;
        for index in 0..raw.len() {
            Self::decode_channels(&mut cursor, &mut raw[index], remaining_pixels - used_pixels)?;
            if let Some(image) = &raw[index].image {
                used_pixels += (image.width * image.height) as i64;
            }
        }
        // Swift jumps the cursor to `layerSectionEnd` here, past any trailing
        // global layer mask info; there is nothing left to read, so the Rust
        // port simply drops the cursor.
        Self::assemble(
            raw,
            CGSize::new(canvas_width as f64, canvas_height as f64),
            resolution,
            remaining_pixels - used_pixels,
        )
    }

    fn read_record(cursor: &mut PsdCursor) -> Result<RawLayer, PsdError> {
        let mut layer = RawLayer::default();
        layer.top = cursor.i32()?;
        layer.left = cursor.i32()?;
        layer.bottom = cursor.i32()?;
        layer.right = cursor.i32()?;
        let channel_count = cursor.u16()? as usize;
        if channel_count > 56 {
            return Err(PsdError::TooLarge);
        }
        for _ in 0..channel_count {
            let id = cursor.i16()? as i64;
            let length = cursor.u32()? as i64;
            layer.channels.push((id, length));
        }
        if cursor.string(4)? != "8BIM" {
            return Err(PsdError::Truncated);
        }
        layer.blend_key = cursor.string(4)?;
        layer.opacity = cursor.u8()?;
        layer.clipping = cursor.u8()? != 0;
        let flags = cursor.u8()?;
        layer.hidden = (flags & 2) != 0;
        cursor.skip(1)?;
        // Extra data: mask, blending ranges, Pascal name, additional info.
        let extra_length = cursor.u32()? as usize;
        let extra_end = cursor.offset + extra_length;
        let mask_length = cursor.u32()? as usize;
        let mask_end = cursor.offset + mask_length;
        if mask_length >= 20 {
            layer.has_mask = true;
            layer.mask_top = cursor.i32()?;
            layer.mask_left = cursor.i32()?;
            layer.mask_bottom = cursor.i32()?;
            layer.mask_right = cursor.i32()?;
            layer.mask_default = cursor.u8()?;
            let mask_flags = cursor.u8()?;
            layer.mask_disabled = (mask_flags & 2) != 0;
            layer.mask_linked = (mask_flags & 1) == 0;
            layer.mask_from_render = (mask_flags & 8) != 0;
        }
        cursor.offset = mask_end;
        let ranges = cursor.u32()? as usize;
        cursor.skip(ranges)?;
        let name_count = cursor.u8()? as usize;
        let name_bytes = cursor.bytes(name_count)?;
        // `String(bytes:encoding:.macOSRoman) ?? String(bytes:encoding:.isoLatin1)`.
        layer.name = macos_roman_to_string(name_bytes);
        let name_pad = (4 - ((name_count + 1) % 4)) % 4;
        cursor.skip(name_pad)?;
        while cursor.offset + 12 <= extra_end {
            let signature = cursor.string(4)?;
            if signature != "8BIM" && signature != "8B64" {
                break;
            }
            let key = cursor.string(4)?;
            let length: usize = if signature == "8B64" {
                if cursor.offset + 8 > extra_end {
                    break;
                }
                let raw = (cursor.u32()? as u64) << 32 | cursor.u32()? as u64;
                if raw > i64::MAX as u64 {
                    return Err(PsdError::TooLarge);
                }
                raw as usize
            } else {
                cursor.u32()? as usize
            };
            let payload = cursor.bytes(length)?;
            if length % 2 == 1 {
                cursor.skip(1)?;
            }
            if key == "luni"
                && let Some(unicode) = unicode_name(payload)
            {
                layer.name = unicode;
            }
            if key == "iOpa"
                && let Some(&fill) = payload.first()
            {
                layer.fill = fill;
            }
            if (key == "lsct" || key == "lsdk") && payload.len() >= 4 {
                layer.section = Some(u32_at(payload, 0) as i64);
            }
            layer.extra.insert(key, payload.to_vec());
        }
        cursor.offset = extra_end;
        Ok(layer)
    }

    /// Transparency, R, G, B, and the user mask. Spot and other extra IDs are skipped before decode.
    const UNPACKED_CHANNEL_IDS: [i64; 5] = [-1, 0, 1, 2, -2];

    fn decode_channels(
        cursor: &mut PsdCursor,
        layer: &mut RawLayer,
        remaining_pixels: i64,
    ) -> Result<(), PsdError> {
        let mut planes: HashMap<i64, Vec<u8>> = HashMap::new();
        let width = (layer.right as i64 - layer.left as i64).max(0) as usize;
        let height = (layer.bottom as i64 - layer.top as i64).max(0) as usize;
        let mask_width = (layer.mask_right as i64 - layer.mask_left as i64).max(0) as usize;
        let mask_height = (layer.mask_bottom as i64 - layer.mask_top as i64).max(0) as usize;
        let budget = remaining_pixels.max(0);
        if width > 0
            && height > 0
            && (width > 30_000 || height > 30_000 || (width * height) as i64 > budget)
        {
            return Err(PsdError::TooLarge);
        }
        if layer.has_mask
            && mask_width > 0
            && mask_height > 0
            && (mask_width > 30_000
                || mask_height > 30_000
                || (mask_width * mask_height) as i64 > budget)
        {
            return Err(PsdError::TooLarge);
        }
        for &(id, length) in &layer.channels {
            let start = cursor.offset;
            if Self::UNPACKED_CHANNEL_IDS.contains(&id) && length >= 2 {
                let compression = cursor.u16()? as i64;
                let payload = cursor.bytes((length - 2) as usize)?;
                let is_mask = id == -2;
                let w = if is_mask { mask_width } else { width };
                let h = if is_mask { mask_height } else { height };
                if w > 0 && h > 0 {
                    planes.insert(id, PsdChannelCoder::decode(compression, w, h, payload)?);
                }
            }
            // The Swift `defer` advances past the channel even when skipped.
            cursor.offset = start + length.max(0) as usize;
        }
        if layer.has_mask
            && mask_width > 0
            && mask_height > 0
            && let Some(gray) = planes.get(&-2)
            && gray.len() >= mask_width * mask_height
        {
            layer.mask_image = Some(GrayMask::from_raw(mask_width, mask_height, gray.clone()));
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        let opaque = vec![255u8; width * height];
        let black = vec![0u8; width * height];
        let red = planes.get(&0).unwrap_or(&black);
        let green = planes.get(&1).unwrap_or(&black);
        let blue = planes.get(&2).unwrap_or(&black);
        let alpha = planes.get(&-1).unwrap_or(&opaque);
        if red.len() < width * height
            || green.len() < width * height
            || blue.len() < width * height
            || alpha.len() < width * height
        {
            return Err(PsdError::Truncated);
        }
        layer.image = Some(PsdChannelCoder::rgba_bitmap(
            width, height, red, green, blue, alpha,
        )?);
        Ok(())
    }

    fn assemble(
        raw: Vec<RawLayer>,
        canvas: CGSize,
        resolution: f64,
        remaining_pixels: i64,
    ) -> Result<PsdDocument, PsdError> {
        let mut result: Vec<PsdRecord> = Vec::new();
        let mut groups: Vec<Uuid> = Vec::new();
        let mut remaining = remaining_pixels.max(0);
        for mut layer in raw {
            // Photoshop stores groups bottom-to-top: type 3 divider, then children, then the folder (type 1/2).
            if layer.section == Some(3) {
                groups.push(Uuid::new_v4());
                continue;
            }
            let is_group = layer.section == Some(1) || layer.section == Some(2);
            let id = if is_group {
                groups.pop().unwrap_or_else(Uuid::new_v4)
            } else {
                Uuid::new_v4()
            };
            let mut record = PsdRecord::new(
                id,
                if layer.name.is_empty() {
                    "Layer"
                } else {
                    layer.name.as_str()
                },
            );
            record.parent_id = groups.last().copied();
            record.is_group = is_group;
            record.is_visible = !layer.hidden;
            record.blend_key =
                if is_group && (layer.blend_key == "pass" || layer.blend_key == "norm") {
                    "pass".to_string()
                } else {
                    layer.blend_key.clone()
                };
            record.clipping = layer.clipping;
            record.kind = Self::kind(&layer, is_group);
            let has_effects = record.kind == PsdLayerKind::Effects
                || layer
                    .extra
                    .keys()
                    .any(|k| k == "lfx2" || k == "lrFX" || k == "lmfx");
            if has_effects && layer.fill != 255 {
                record.opacity = layer.opacity as f64 / 255.0;
            } else {
                record.opacity = (layer.opacity as f64 / 255.0) * (layer.fill as f64 / 255.0);
            }
            record.bounds = if is_group {
                CGRect::new(0.0, 0.0, canvas.width, canvas.height)
            } else {
                CGRect::new(
                    layer.left as f64,
                    layer.top as f64,
                    (layer.right as i64 - layer.left as i64).max(0) as f64,
                    (layer.bottom as i64 - layer.top as i64).max(0) as f64,
                )
            };
            record.image = if is_group { None } else { layer.image.take() };
            if !is_group {
                if let Some(live) = PsdVector::live(&layer.extra, canvas, remaining)? {
                    record.image = Some(live.image);
                    record.bounds = live.bounds;
                    record.shape = Some(live.style);
                    record.shape_notes = live.notes;
                    record.kind = PsdLayerKind::Vector;
                    remaining = (remaining
                        - (record.image.as_ref().unwrap().width
                            * record.image.as_ref().unwrap().height)
                            as i64)
                        .max(0);
                } else if record.image.is_none()
                    && let Some(raster) = PsdVector::raster(&layer.extra, canvas, remaining)?
                {
                    remaining =
                        (remaining - (raster.image.width * raster.image.height) as i64).max(0);
                    record.image = Some(raster.image);
                    record.bounds = raster.bounds;
                    record.kind = PsdLayerKind::Vector;
                }
            }
            record.mask = if layer.mask_from_render {
                None
            } else {
                layer.mask_image.take()
            };
            record.mask_enabled = !layer.mask_disabled;
            record.mask_linked = layer.mask_linked;
            if !is_group {
                record.adjustment = PsdAdjustments::parse(&layer.extra);
            }
            if record.adjustment.is_some() {
                record.kind = PsdLayerKind::Adjustment;
            }
            result.push(record);
        }
        if !groups.is_empty() {
            return Err(PsdError::Truncated);
        }
        Ok(PsdDocument {
            width: canvas.width as usize,
            height: canvas.height as usize,
            resolution,
            layers: result,
        })
    }

    fn kind(layer: &RawLayer, is_group: bool) -> PsdLayerKind {
        if is_group {
            return PsdLayerKind::Group;
        }
        if layer
            .extra
            .keys()
            .any(|k| k == "TySh" || k == "tySh" || k == "txt2")
        {
            return PsdLayerKind::Text;
        }
        if layer
            .extra
            .keys()
            .any(|k| k == "vmsk" || k == "vsms" || k == "vogk")
        {
            return PsdLayerKind::Vector;
        }
        if layer.extra.keys().any(|k| k == "SoLd" || k == "SoLE") {
            return PsdLayerKind::SmartObject;
        }
        if layer
            .extra
            .keys()
            .any(|k| k == "lfx2" || k == "lrFX" || k == "lmfx")
        {
            return PsdLayerKind::Effects;
        }
        if layer
            .extra
            .keys()
            .any(|k| Self::ADJUSTMENT_KEYS.contains(&k.as_str()))
        {
            return PsdLayerKind::Adjustment;
        }
        PsdLayerKind::Raster
    }

    pub const ADJUSTMENT_KEYS: [&'static str; 16] = [
        "levl", "curv", "hue2", "hue ", "expA", "grdm", "brit", "blnc", "nvrt", "thrs", "post",
        "mixr", "selc", "blwh", "phfl", "vibA",
    ];
}

/// One layer record before assembly (`PSDReader.RawLayer`).
struct RawLayer {
    name: String,
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    opacity: u8,
    fill: u8,
    clipping: bool,
    hidden: bool,
    blend_key: String,
    channels: Vec<(i64, i64)>,
    extra: HashMap<String, Vec<u8>>,
    mask_top: i32,
    mask_left: i32,
    mask_bottom: i32,
    mask_right: i32,
    mask_default: u8,
    mask_disabled: bool,
    mask_linked: bool,
    mask_from_render: bool,
    has_mask: bool,
    section: Option<i64>,
    image: Option<Bitmap>,
    mask_image: Option<GrayMask>,
}

impl Default for RawLayer {
    fn default() -> Self {
        RawLayer {
            name: String::new(),
            top: 0,
            left: 0,
            bottom: 0,
            right: 0,
            opacity: 255,
            fill: 255,
            clipping: false,
            hidden: false,
            blend_key: "norm".to_string(),
            channels: Vec::new(),
            extra: HashMap::new(),
            mask_top: 0,
            mask_left: 0,
            mask_bottom: 0,
            mask_right: 0,
            mask_default: 255,
            mask_disabled: false,
            mask_linked: true,
            mask_from_render: false,
            has_mask: false,
            section: None,
            image: None,
            mask_image: None,
        }
    }
}

/// `String(bytes:encoding:.macOSRoman)` for Pascal-string layer names: bytes
/// 0x00-0x7F are ASCII; 0x80-0xFF follow Apple's Mac OS Roman table (Unicode
/// consortium `ROMAN.TXT`). Roman assigns all 256 bytes, so the decode never
/// fails and the Swift ISO Latin 1 fallback is unreachable; `from_utf8_lossy`
/// turned every high byte into U+FFFD instead.
fn macos_roman_to_string(bytes: &[u8]) -> String {
    const HIGH: [u16; 128] = [
        0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, // 0x80-0x87
        0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8, // 0x88-0x8F
        0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, // 0x90-0x97
        0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, // 0x98-0x9F
        0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, // 0xA0-0xA7
        0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8, // 0xA8-0xAF
        0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211, // 0xB0-0xB7
        0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, // 0xB8-0xBF
        0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, // 0xC0-0xC7
        0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, // 0xC8-0xCF
        0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA, // 0xD0-0xD7
        0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, // 0xD8-0xDF
        0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, // 0xE0-0xE7
        0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4, // 0xE8-0xEF
        0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, // 0xF0-0xF7
        0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7, // 0xF8-0xFF
    ];
    bytes
        .iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap()
            }
        })
        .collect()
}

/// `PSDReader.unicodeName`: UTF-16 units with a NUL-trimmed decode.
fn unicode_name(data: &[u8]) -> Option<String> {
    if data.len() < 4 {
        return None;
    }
    let count = u32_at(data, 0) as usize;
    if count == 0 || data.len() < 4 + count * 2 {
        return None;
    }
    let mut units = Vec::with_capacity(count);
    for i in 0..count {
        let hi = data[4 + i * 2];
        let lo = data[5 + i * 2];
        units.push((hi as u16) << 8 | lo as u16);
    }
    Some(
        String::from_utf16_lossy(&units)
            .trim_matches('\0')
            .to_string(),
    )
}

/// `PSDAdjustments`: the three adjustment layers Compositor imports.
pub struct PsdAdjustments;

impl PsdAdjustments {
    pub fn parse(extra: &HashMap<String, Vec<u8>>) -> Option<LayerAdjustment> {
        if let Some(data) = extra.get("levl") {
            return levels(data);
        }
        if let Some(data) = extra.get("curv") {
            return curves(data);
        }
        if let Some(data) = extra.get("hue2").or_else(|| extra.get("hue ")) {
            return hue(data);
        }
        None
    }
}

fn levels(data: &[u8]) -> Option<LayerAdjustment> {
    if data.len() < 292 {
        return None;
    }
    let mut settings = LevelsSettings::default();
    for channel in 0..4 {
        let base = 2 + channel * 10;
        let input_black = u16_at(data, base) as f64;
        let input_white = u16_at(data, base + 2) as f64;
        let output_black = u16_at(data, base + 4) as f64;
        let output_white = u16_at(data, base + 6) as f64;
        let gamma = u16_at(data, base + 8) as f64 / 256.0;
        settings.ranges[channel] = LevelRange {
            black: input_black,
            gamma,
            white: input_white,
            output_black,
            output_white,
        }
        .normalized();
    }
    let mut adjustment = LayerAdjustment::new(AdjustmentKind::Levels);
    adjustment.levels = settings;
    Some(adjustment)
}

fn curves(data: &[u8]) -> Option<LayerAdjustment> {
    if data.len() < 5 {
        return None;
    }
    let mut offset = 0usize;
    if data[offset] == 0 {
        offset += 1;
    }
    if offset + 2 > data.len() {
        return None;
    }
    let version = u16_at(data, offset);
    offset += 2;
    if version != 1 && version != 4 {
        return None;
    }
    if offset + 2 > data.len() {
        return None;
    }
    let count = u16_at(data, offset) as usize;
    offset += 2;
    let mut settings = CurvesSettings::default();
    for channel in 0..count.min(4) {
        if offset + 2 > data.len() {
            return None;
        }
        let points = u16_at(data, offset) as usize;
        offset += 2;
        let mut curve: Vec<CurvePoint> = Vec::new();
        for _ in 0..points {
            if offset + 4 > data.len() {
                return None;
            }
            let output = u16_at(data, offset) as f64;
            let input = u16_at(data, offset + 2) as f64;
            offset += 4;
            curve.push(CurvePoint {
                x: input.clamp(0.0, 255.0),
                y: output.clamp(0.0, 255.0),
            });
        }
        if curve.len() >= 2 {
            curve.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
            if curve.first().map(|p| p.x) != Some(0.0) {
                let y = curve.first().unwrap().y;
                curve.insert(0, CurvePoint { x: 0.0, y });
            }
            if curve.last().map(|p| p.x) != Some(255.0) {
                let y = curve.last().unwrap().y;
                curve.push(CurvePoint { x: 255.0, y });
            }
            settings.channels[channel] = curve;
        }
    }
    if !settings.is_valid() {
        return None;
    }
    let mut adjustment = LayerAdjustment::new(AdjustmentKind::Curves);
    adjustment.curves = settings;
    Some(adjustment)
}

fn hue(data: &[u8]) -> Option<LayerAdjustment> {
    if data.len() < 4 {
        return None;
    }
    let colorize = data[2] != 0;
    let mut settings = HueSaturationSettings::default();
    settings.colorize = colorize;
    let ranges = [
        ColorRange::Master,
        ColorRange::Reds,
        ColorRange::Yellows,
        ColorRange::Greens,
        ColorRange::Cyans,
        ColorRange::Blues,
        ColorRange::Magentas,
    ];
    let mut offset = 4usize;
    for range in ranges {
        if offset + 6 > data.len() {
            break;
        }
        let h = i16_at(data, offset);
        let s = i16_at(data, offset + 2);
        let l = i16_at(data, offset + 4);
        offset += 6;
        settings.adjustments.insert(
            range,
            RangeAdjustment {
                hue: h as f64,
                saturation: s as f64,
                lightness: l as f64,
            },
        );
    }
    let mut adjustment = LayerAdjustment::new(AdjustmentKind::Hsv);
    adjustment.hsv_settings = Some(settings);
    Some(adjustment)
}

/// The byte cursor (`PSDReader.PSDCursor`): big-endian reads that throw
/// `truncated` past the end.
struct PsdCursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> PsdCursor<'a> {
    fn need(&self, count: usize) -> Result<(), PsdError> {
        if self.offset + count <= self.data.len() {
            Ok(())
        } else {
            Err(PsdError::Truncated)
        }
    }

    fn skip(&mut self, count: usize) -> Result<(), PsdError> {
        self.need(count)?;
        self.offset += count;
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, PsdError> {
        self.need(1)?;
        let value = self.data[self.offset];
        self.offset += 1;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, PsdError> {
        self.need(2)?;
        let value = u16_at(self.data, self.offset);
        self.offset += 2;
        Ok(value)
    }

    fn i16(&mut self) -> Result<i16, PsdError> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32, PsdError> {
        self.need(4)?;
        let value = u32_at(self.data, self.offset);
        self.offset += 4;
        Ok(value)
    }

    fn i32(&mut self) -> Result<i32, PsdError> {
        Ok(self.u32()? as i32)
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8], PsdError> {
        self.need(count)?;
        let slice = &self.data[self.offset..self.offset + count];
        self.offset += count;
        Ok(slice)
    }

    /// Strict ASCII like Swift's `String(bytes:encoding:.ascii)`; "" when any
    /// byte is not ASCII.
    fn string(&mut self, count: usize) -> Result<String, PsdError> {
        let bytes = self.bytes(count)?;
        if bytes.iter().all(|&b| b.is_ascii()) {
            Ok(String::from_utf8_lossy(bytes).into_owned())
        } else {
            Ok(String::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_reads_big_endian_and_truncates() {
        let data = [0x01, 0x02, 0x00, 0x00, 0x00, 0x7F];
        let mut cursor = PsdCursor {
            data: &data,
            offset: 0,
        };
        assert_eq!(cursor.u16().unwrap(), 0x0102);
        assert_eq!(cursor.i32().unwrap(), 0x7F);
        assert_eq!(cursor.u8().unwrap_err(), PsdError::Truncated);
    }

    #[test]
    fn unicode_name_trims_nul_units() {
        let mut data = vec![0, 0, 0, 4];
        for unit in [0x0041u16, 0, 0x0042, 0] {
            data.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(unicode_name(&data).as_deref(), Some("A\u{0}B"));
    }

    #[test]
    fn adjustment_keys_cover_the_reader_set() {
        assert!(PsdReader::ADJUSTMENT_KEYS.contains(&"levl"));
        assert!(PsdReader::ADJUSTMENT_KEYS.contains(&"vibA"));
        assert!(!PsdReader::ADJUSTMENT_KEYS.contains(&"SoCo"));
    }

    #[test]
    fn macos_roman_decodes_high_bytes() {
        // 0x80 is 'Ä' (U+00C4) and 0x8E is 'é' (U+00E9) in Mac OS Roman;
        // from_utf8_lossy turned both into U+FFFD. (0xE9 is 'È' in MacRoman —
        // the é of ISO Latin 1 — which is exactly why the table is needed.)
        assert_eq!(macos_roman_to_string(&[0x80]), "\u{00C4}");
        assert_eq!(macos_roman_to_string(&[0x8E]), "\u{00E9}");
        assert_eq!(macos_roman_to_string(b"Caf\x8E"), "Caf\u{00E9}");
        assert_eq!(macos_roman_to_string(&[0xE9]), "\u{00C8}");
        // ASCII passes through untouched.
        assert_eq!(macos_roman_to_string(b"Layer 1"), "Layer 1");
        // Spot checks from Apple's table: left double quote, euro, Apple logo.
        assert_eq!(macos_roman_to_string(&[0xD2]), "\u{201C}");
        assert_eq!(macos_roman_to_string(&[0xDB]), "\u{20AC}");
        assert_eq!(macos_roman_to_string(&[0xF0]), "\u{F8FF}");
        // Roman assigns every byte, so nothing decodes to U+FFFD.
        for b in 0x80u8..=0xFF {
            let decoded = macos_roman_to_string(&[b]);
            assert!(!decoded.contains('\u{FFFD}'), "0x{b:02X} became U+FFFD");
        }
    }

    #[test]
    fn pascal_layer_name_decodes_macos_roman() {
        // A layer record whose Pascal name is "Café" with a raw high byte:
        // read_record must decode it via macOSRoman, not lose it to U+FFFD.
        let name = b"Caf\x8E";
        let name_pad = (4 - ((name.len() + 1) % 4)) % 4;
        let extra_length = 4 + 4 + 1 + name.len() + name_pad;
        let mut data = Vec::new();
        for value in [0i32, 0, 20, 20] {
            data.extend_from_slice(&value.to_be_bytes());
        }
        data.extend_from_slice(&0u16.to_be_bytes()); // channel count
        data.extend_from_slice(b"8BIM");
        data.extend_from_slice(b"norm");
        data.push(255); // opacity
        data.push(0); // clipping
        data.push(0); // flags
        data.push(0); // pad
        data.extend_from_slice(&(extra_length as u32).to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes()); // mask length
        data.extend_from_slice(&0u32.to_be_bytes()); // blending ranges
        data.push(name.len() as u8);
        data.extend_from_slice(name);
        data.extend(std::iter::repeat(0u8).take(name_pad));
        let mut cursor = PsdCursor {
            data: &data,
            offset: 0,
        };
        let layer = PsdReader::read_record(&mut cursor).unwrap();
        assert_eq!(layer.name, "Caf\u{00E9}");
    }
}
