//! Port of `CompositorTests/PSDFixture.swift`: builds tiny Photoshop files for
//! reader tests. Not part of the app; Compositor does not write PSD. This is
//! the byte-level writer to the extent one exists in the Swift project, and it
//! makes the round-trip tests below byte-exact.

use uuid::Uuid;

use crate::raster::{Bitmap, GrayMask};
use crate::transform::LayerSampling;

use super::channel_coder::PsdChannelCoder;
use super::types::{PsdDocument, PsdError, PsdLayerKind, PsdRecord};

pub struct PsdFixture;

/// One layer about to be serialized (`PSDFixture.Prepared`).
struct Prepared {
    record: PsdRecord,
    is_divider: bool,
    channels: Vec<(i16, Vec<u8>)>,
    top: i64,
    left: i64,
    bottom: i64,
    right: i64,
    mask_top: i64,
    mask_left: i64,
    mask_bottom: i64,
    mask_right: i64,
}

impl PsdFixture {
    /// `PSDFixture.data(_:composite:)`: a complete `.psd` byte stream.
    pub fn data(document: &PsdDocument, composite: &Bitmap) -> Result<Vec<u8>, PsdError> {
        let width = document.width;
        let height = document.height;
        if !(1..=30_000).contains(&width) || !(1..=30_000).contains(&height) {
            return Err(PsdError::TooLarge);
        }
        let mut file = PsdBuffer::default();
        file.string("8BPS");
        file.u16(1);
        file.bytes(&[0; 6]);
        file.u16(4);
        file.u32(height as u32);
        file.u32(width as u32);
        file.u16(8);
        file.u16(3);
        file.u32(0);
        let resources = resolution_resource(document.resolution);
        file.u32(resources.len() as u32);
        file.bytes(&resources);
        let layers = layer_section(document)?;
        file.u32(layers.len() as u32);
        file.bytes(&layers);
        append_composite(&mut file, composite, width, height)?;
        Ok(file.data)
    }
}

/// File order is bottom-to-top. Photoshop groups are type 3, children, then type 1/2.
fn emit(
    document: &PsdDocument,
    parent: Option<Uuid>,
    prepared: &mut Vec<Prepared>,
) -> Result<(), PsdError> {
    for record in &document.layers {
        if record.parent_id != parent {
            continue;
        }
        if record.is_group {
            prepared.push(empty_layer(
                "</Layer group>",
                "norm",
                3,
                true,
                1.0,
                parent,
                None,
                None,
                true,
            )?);
            emit(document, Some(record.id), prepared)?;
            prepared.push(empty_layer(
                &record.name,
                if record.blend_key == "pass" {
                    "pass"
                } else {
                    &record.blend_key
                },
                1,
                record.is_visible,
                record.opacity,
                record.parent_id,
                Some(record.id),
                record.mask.as_ref(),
                record.mask_enabled,
            )?);
        } else {
            prepared.push(layer_prepared(record)?);
        }
    }
    Ok(())
}

fn layer_section(document: &PsdDocument) -> Result<Vec<u8>, PsdError> {
    let mut prepared: Vec<Prepared> = Vec::new();
    emit(document, None, &mut prepared)?;
    if prepared.len() > i16::MAX as usize {
        return Err(PsdError::TooLarge);
    }
    let mut records = PsdBuffer::default();
    records.i16(prepared.len() as i16);
    let mut payloads = PsdBuffer::default();
    for item in &prepared {
        write_record(&mut records, item);
        for channel in &item.channels {
            payloads.bytes(&channel.1);
        }
    }
    let mut info = PsdBuffer::default();
    info.u32(0);
    info.bytes(&records.data);
    info.bytes(&payloads.data);
    if info.data.len() % 2 == 1 {
        info.u8(0);
    }
    // Patch the placeholder with the real layer info length.
    let layer_bytes = (info.data.len() - 4) as u32;
    info.data[0..4].copy_from_slice(&layer_bytes.to_be_bytes());
    let mut section = PsdBuffer::default();
    section.bytes(&info.data);
    section.u32(0);
    Ok(section.data)
}

fn layer_prepared(record: &PsdRecord) -> Result<Prepared, PsdError> {
    let image = record.image.as_ref();
    let width = image.map(|i| i.width).unwrap_or(0);
    let height = image.map(|i| i.height).unwrap_or(0);
    let left = (record.bounds.min_x().round()) as i64;
    let top = (record.bounds.min_y().round()) as i64;
    let mut channels: Vec<(i16, Vec<u8>)> = Vec::new();
    match image.filter(|_| width > 0 && height > 0) {
        Some(image) => {
            let (red, green, blue, alpha) = planes(image);
            for (id, plane) in [(-1i16, alpha), (0, red), (1, green), (2, blue)] {
                channels.push((id, channel_payload(&plane, width, height)));
            }
        }
        None => channels = empty_channels(),
    }
    if let Some(mask) = &record.mask {
        channels.push((-2, channel_payload(&mask.data, mask.width, mask.height)));
    }
    Ok(Prepared {
        record: record.clone(),
        is_divider: false,
        channels,
        top,
        left,
        bottom: top + height as i64,
        right: left + width as i64,
        mask_top: top,
        mask_left: left,
        mask_bottom: top + record.mask.as_ref().map(|m| m.height as i64).unwrap_or(0),
        mask_right: left + record.mask.as_ref().map(|m| m.width as i64).unwrap_or(0),
    })
}

#[allow(clippy::too_many_arguments)]
fn empty_layer(
    name: &str,
    blend_key: &str,
    section: i64,
    visible: bool,
    opacity: f64,
    parent: Option<Uuid>,
    id: Option<Uuid>,
    mask: Option<&GrayMask>,
    mask_enabled: bool,
) -> Result<Prepared, PsdError> {
    let mut record = PsdRecord::new(id.unwrap_or_else(Uuid::new_v4), name);
    record.parent_id = parent;
    record.is_group = section != 3;
    record.is_visible = visible;
    record.opacity = opacity;
    record.blend_key = blend_key.to_string();
    record.mask = mask.cloned();
    record.mask_enabled = mask_enabled;
    record.kind = PsdLayerKind::Group;
    let mut channels = empty_channels();
    let mut mask_bottom = 0i64;
    let mut mask_right = 0i64;
    if let Some(mask) = mask {
        channels.push((-2, channel_payload(&mask.data, mask.width, mask.height)));
        mask_bottom = mask.height as i64;
        mask_right = mask.width as i64;
    }
    Ok(Prepared {
        record,
        is_divider: section == 3,
        channels,
        top: 0,
        left: 0,
        bottom: 0,
        right: 0,
        mask_top: 0,
        mask_left: 0,
        mask_bottom,
        mask_right,
    })
}

fn empty_channels() -> Vec<(i16, Vec<u8>)> {
    vec![
        (-1, vec![0, 0]),
        (0, vec![0, 0]),
        (1, vec![0, 0]),
        (2, vec![0, 0]),
    ]
}

fn channel_payload(plane: &[u8], width: usize, height: usize) -> Vec<u8> {
    let (compression, encoded) = PsdChannelCoder::encode_plane(plane, width, height);
    let mut data = Vec::with_capacity(encoded.len() + 2);
    data.push((compression >> 8) as u8);
    data.push((compression & 0xff) as u8);
    data.extend_from_slice(&encoded);
    data
}

fn clamp_i32(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn write_record(buffer: &mut PsdBuffer, item: &Prepared) {
    let record = &item.record;
    buffer.i32(clamp_i32(item.top));
    buffer.i32(clamp_i32(item.left));
    buffer.i32(clamp_i32(item.bottom));
    buffer.i32(clamp_i32(item.right));
    buffer.u16(item.channels.len() as u16);
    for channel in &item.channels {
        buffer.i16(channel.0);
        buffer.u32(channel.1.len() as u32);
    }
    buffer.string("8BIM");
    let mut key = record.blend_key.clone();
    key.push_str("    ");
    key.truncate(4);
    buffer.string(&key);
    buffer.u8(((record.opacity * 255.0).round() as i64).clamp(0, 255) as u8);
    buffer.u8(if record.clipping { 1 } else { 0 });
    buffer.u8(if record.is_visible { 0 } else { 2 });
    buffer.u8(0);
    let extra = extra_data(item);
    buffer.u32(extra.len() as u32);
    buffer.bytes(&extra);
}

fn extra_data(item: &Prepared) -> Vec<u8> {
    let mut extra = PsdBuffer::default();
    let record = &item.record;
    if record.mask.is_some() && item.mask_right > item.mask_left && item.mask_bottom > item.mask_top
    {
        extra.u32(20);
        extra.i32(clamp_i32(item.mask_top));
        extra.i32(clamp_i32(item.mask_left));
        extra.i32(clamp_i32(item.mask_bottom));
        extra.i32(clamp_i32(item.mask_right));
        extra.u8(255);
        let mut flags: u8 = if record.mask_linked { 0 } else { 1 };
        if !record.mask_enabled {
            flags |= 2;
        }
        extra.u8(flags);
        extra.u16(0);
    } else {
        extra.u32(0);
    }
    extra.u32(0);
    let pascal: Vec<u8> = record.name.as_bytes().iter().take(255).copied().collect();
    extra.u8(pascal.len() as u8);
    extra.bytes(&pascal);
    let name_bytes = 1 + pascal.len();
    let pad = (4 - (name_bytes % 4)) % 4;
    extra.bytes(&vec![0u8; pad]);
    write_additional(&mut extra, "luni", &luni(&record.name));
    if record.is_group || item.is_divider {
        let section: u8 = if item.is_divider { 3 } else { 1 };
        let mut payload = vec![0, 0, 0, section];
        payload.extend_from_slice(b"8BIM");
        let blend = if item.is_divider {
            "norm".to_string()
        } else {
            let base = if record.blend_key == "pass" {
                "pass"
            } else {
                record.blend_key.as_str()
            };
            format!("{base}    ")
        };
        blend
            .as_bytes()
            .iter()
            .take(4)
            .for_each(|&b| payload.push(b));
        write_additional(&mut extra, "lsct", &payload);
    }
    extra.data
}

fn write_additional(buffer: &mut PsdBuffer, key: &str, payload: &[u8]) {
    buffer.string("8BIM");
    buffer.string(key);
    buffer.u32(payload.len() as u32);
    buffer.bytes(payload);
    if payload.len() % 2 == 1 {
        buffer.u8(0);
    }
}

fn luni(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut data = Vec::with_capacity(4 + units.len() * 2);
    data.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for unit in units {
        data.extend_from_slice(&unit.to_be_bytes());
    }
    data
}

fn resolution_resource(resolution: f64) -> Vec<u8> {
    let mut resource = PsdBuffer::default();
    resource.string("8BIM");
    resource.u16(1005);
    resource.u8(0);
    resource.u8(0);
    resource.u32(16);
    let fixed = (resolution.clamp(1.0, 9600.0) * 65536.0).round() as u32;
    resource.u32(fixed);
    resource.u16(1);
    resource.u16(1);
    resource.u32(fixed);
    resource.u16(1);
    resource.u16(1);
    resource.data
}

fn append_composite(
    file: &mut PsdBuffer,
    composite: &Bitmap,
    width: usize,
    height: usize,
) -> Result<(), PsdError> {
    // Swift redraws the image into a width×height context; tests pass matching
    // bitmaps, so a resample covers the general case.
    let flattened = if composite.width == width && composite.height == height {
        composite.clone()
    } else {
        crate::raster::resample(composite, width, height, LayerSampling::High)
    };
    let (red, green, blue, alpha) = planes(&flattened);
    file.u16(1);
    let mut counts = Vec::new();
    let mut packed = Vec::new();
    for plane in [&red, &green, &blue, &alpha] {
        let (_compression, encoded) = PsdChannelCoder::encode_plane(plane, width, height);
        counts.extend_from_slice(&encoded[..height * 2]);
        packed.extend_from_slice(&encoded[height * 2..]);
    }
    file.bytes(&counts);
    file.bytes(&packed);
    Ok(())
}

/// Premultiplied RGBA, first row at the top of the image.
fn planes(image: &Bitmap) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let width = image.width;
    let height = image.height;
    let mut red = vec![0u8; width * height];
    let mut green = vec![0u8; width * height];
    let mut blue = vec![0u8; width * height];
    let mut alpha = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let p = image.pixel(x, y);
            let a = p[3] as u32;
            alpha[i] = p[3];
            if a == 0 {
                red[i] = 0;
                green[i] = 0;
                blue[i] = 0;
            } else {
                red[i] = ((p[0] as u32 * 255 + a / 2) / a).min(255) as u8;
                green[i] = ((p[1] as u32 * 255 + a / 2) / a).min(255) as u8;
                blue[i] = ((p[2] as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    (red, green, blue, alpha)
}

#[derive(Default)]
struct PsdBuffer {
    data: Vec<u8>,
}

impl PsdBuffer {
    fn u8(&mut self, value: u8) {
        self.data.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    fn i16(&mut self, value: i16) {
        self.u16(value as u16);
    }

    fn u32(&mut self, value: u32) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.u32(value as u32);
    }

    fn bytes(&mut self, value: &[u8]) {
        self.data.extend_from_slice(value);
    }

    fn string(&mut self, value: &str) {
        self.data.extend_from_slice(value.as_bytes());
    }
}
