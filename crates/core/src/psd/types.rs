//! Port of `Compositor/IO/PSD/PSDTypes.swift`: the PSD errors, the parsed
//! document model, and the PSD blend-key → app blend-mode mapping.

use crate::adjustment::LayerAdjustment;
use crate::document::{LayerBlendMode, PaletteColor, ShapeKind};
use crate::geometry::CGRect;
use crate::raster::{Bitmap, GrayMask};
use uuid::Uuid;

/// `PSDError` plus the two `ImageImportError` cases `PSDReader` throws
/// (`unreadable` for a bad signature, `tooLarge` for the pixel budgets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsdError {
    /// `PSDError.truncated`
    Truncated,
    /// `PSDError.unsupportedVersion` — large-document `.psb` files.
    UnsupportedVersion,
    /// `PSDError.unsupportedColorMode`
    UnsupportedColorMode,
    /// `PSDError.unsupportedDepth`
    UnsupportedDepth,
    /// `PSDError.unsupportedCompression`
    UnsupportedCompression,
    /// `ImageImportError.unreadable`
    Unreadable,
    /// `ImageImportError.tooLarge`
    TooLarge,
}

impl PsdError {
    pub fn error_description(&self) -> &'static str {
        match self {
            PsdError::Truncated => {
                "The Photoshop file could not be read. It may be damaged or incomplete."
            }
            PsdError::UnsupportedVersion => {
                "Large Document (.psb) Photoshop files aren\u{2019}t supported."
            }
            PsdError::UnsupportedColorMode => "Only 8-bit RGB Photoshop files can be imported.",
            PsdError::UnsupportedDepth => "Only 8-bit RGB Photoshop files can be imported.",
            PsdError::UnsupportedCompression => {
                "This Photoshop file uses a layer compression method that isn\u{2019}t supported."
            }
            PsdError::Unreadable => {
                "The image could not be read. It may be damaged or unavailable."
            }
            PsdError::TooLarge => {
                "This import exceeds the current 100-megapixel document budget or 30,000-pixel side limit."
            }
        }
    }
}

impl std::fmt::Display for PsdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.error_description())
    }
}

impl std::error::Error for PsdError {}

/// One row of the conversion report (`PSDConversion`).
#[derive(Debug, Clone, PartialEq)]
pub struct PsdConversion {
    pub id: Uuid,
    pub layer_name: String,
    pub message: String,
}

impl PsdConversion {
    pub fn new(layer_name: impl Into<String>, message: impl Into<String>) -> Self {
        PsdConversion {
            id: Uuid::new_v4(),
            layer_name: layer_name.into(),
            message: message.into(),
        }
    }
}

/// `PSDDocument`: what the reader produced from one `.psd` file.
#[derive(Clone)]
pub struct PsdDocument {
    pub width: usize,
    pub height: usize,
    pub resolution: f64,
    /// Bottom to top, including folders. Hidden section dividers are not stored.
    pub layers: Vec<PsdRecord>,
}

impl std::fmt::Debug for PsdDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PsdDocument")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("resolution", &self.resolution)
            .field("layers", &self.layers)
            .finish()
    }
}

/// `PSDRecord`: one layer or folder, with its decoded pixels.
#[derive(Clone)]
pub struct PsdRecord {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub is_group: bool,
    pub is_visible: bool,
    pub opacity: f64,
    pub blend_key: String,
    pub clipping: bool,
    pub bounds: CGRect,
    pub image: Option<Bitmap>,
    pub mask: Option<GrayMask>,
    pub mask_enabled: bool,
    pub mask_linked: bool,
    pub adjustment: Option<LayerAdjustment>,
    pub kind: PsdLayerKind,
    pub shape: Option<LayerShapeStyle>,
    pub shape_notes: Vec<String>,
}

impl std::fmt::Debug for PsdRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PsdRecord")
            .field("id", &self.id)
            .field("parent_id", &self.parent_id)
            .field("name", &self.name)
            .field("is_group", &self.is_group)
            .field("is_visible", &self.is_visible)
            .field("opacity", &self.opacity)
            .field("blend_key", &self.blend_key)
            .field("clipping", &self.clipping)
            .field("bounds", &self.bounds)
            .field("image", &self.image.as_ref().map(|i| (i.width, i.height)))
            .field("mask", &self.mask.as_ref().map(|m| (m.width, m.height)))
            .field("mask_enabled", &self.mask_enabled)
            .field("mask_linked", &self.mask_linked)
            .field("adjustment", &self.adjustment)
            .field("kind", &self.kind)
            .field("shape", &self.shape)
            .field("shape_notes", &self.shape_notes)
            .finish()
    }
}

impl PsdRecord {
    /// `PSDRecord(id:name:)` with the Swift field defaults.
    pub fn new(id: Uuid, name: impl Into<String>) -> Self {
        PsdRecord {
            id,
            parent_id: None,
            name: name.into(),
            is_group: false,
            is_visible: true,
            opacity: 1.0,
            blend_key: "norm".to_string(),
            clipping: false,
            bounds: CGRect::ZERO,
            image: None,
            mask: None,
            mask_enabled: true,
            mask_linked: true,
            adjustment: None,
            kind: PsdLayerKind::Raster,
            shape: None,
            shape_notes: Vec::new(),
        }
    }

    /// `PSDRecord.blendMode`.
    pub fn blend_mode(&self) -> Option<LayerBlendMode> {
        blend_mode_from_psd(&self.blend_key)
    }
}

/// `PSDLayerKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsdLayerKind {
    Raster,
    Group,
    Adjustment,
    Text,
    SmartObject,
    Effects,
    Vector,
    Other,
}

/// `LayerBlendMode.fromPSD(_:)`. Dissolve, Darker Color and Lighter Color are
/// deliberately absent: Compositor has no equivalent, so they fall through to
/// Normal and say so in the conversion report.
pub fn blend_mode_from_psd(key: &str) -> Option<LayerBlendMode> {
    match key {
        "norm" => Some(LayerBlendMode::Normal),
        "mul " => Some(LayerBlendMode::Multiply),
        "scrn" => Some(LayerBlendMode::Screen),
        "over" => Some(LayerBlendMode::Overlay),
        "sLit" => Some(LayerBlendMode::SoftLight),
        "dark" => Some(LayerBlendMode::Darken),
        "lite" => Some(LayerBlendMode::Lighten),
        "diff" => Some(LayerBlendMode::Difference),
        "div " => Some(LayerBlendMode::ColorDodge),
        "idiv" => Some(LayerBlendMode::ColorBurn),
        "hue " => Some(LayerBlendMode::Hue),
        "sat " => Some(LayerBlendMode::Saturation),
        "colr" => Some(LayerBlendMode::Color),
        "lum " => Some(LayerBlendMode::Luminosity),
        "lbrn" => Some(LayerBlendMode::LinearBurn),
        "lddg" => Some(LayerBlendMode::LinearDodge),
        "hLit" => Some(LayerBlendMode::HardLight),
        "vLit" => Some(LayerBlendMode::VividLight),
        "lLit" => Some(LayerBlendMode::LinearLight),
        "pLit" => Some(LayerBlendMode::PinLight),
        "hMix" => Some(LayerBlendMode::HardMix),
        "smud" => Some(LayerBlendMode::Exclusion),
        "fsub" => Some(LayerBlendMode::Subtract),
        "fdiv" => Some(LayerBlendMode::Divide),
        _ => None,
    }
}

/// Port of `LayerShapeStyle` (`ShapeTool.swift`), the subset PSD import maps
/// onto the app model. The Rust `document::LayerShape` keeps no pixel cache,
/// so the shape's raster stays in the layer's own asset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerShapeStyle {
    pub kind: ShapeKind,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    /// Document pixels, whatever size the shape is scaled to.
    pub corner_radius: f64,
}

impl LayerShapeStyle {
    pub fn new(kind: ShapeKind, red: f64, green: f64, blue: f64, corner_radius: f64) -> Self {
        LayerShapeStyle {
            kind,
            red,
            green,
            blue,
            corner_radius,
        }
    }

    pub fn color(&self) -> PaletteColor {
        PaletteColor::new(self.red, self.green, self.blue)
    }

    /// `LayerShape(style:image:)` — the model default line width, always filled.
    pub fn to_layer_shape(&self) -> crate::document::LayerShape {
        crate::document::LayerShape {
            kind: self.kind,
            corner_radius: self.corner_radius,
            line_width: 4.0,
            red: self.red,
            green: self.green,
            blue: self.blue,
            filled: true,
            start: crate::geometry::CGPoint::ZERO,
            end: crate::geometry::CGPoint::new(1.0, 1.0),
        }
    }
}

// Shared big-endian byte helpers (the Swift code reads everything manually).

pub(crate) fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([data[at], data[at + 1]])
}

pub(crate) fn i16_at(data: &[u8], at: usize) -> i16 {
    i16::from_be_bytes([data[at], data[at + 1]])
}

pub(crate) fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}

pub(crate) fn i32_at(data: &[u8], at: usize) -> i32 {
    i32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}

pub(crate) fn u64_at(data: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(data[at..at + 8].try_into().unwrap())
}

pub(crate) fn f64_at(data: &[u8], at: usize) -> f64 {
    f64::from_bits(u64_at(data, at))
}

/// `PSDVector.offset(of:in:from:)`: the first position of `needle` at or after
/// `start`.
pub(crate) fn find_offset(data: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if start >= data.len() || needle.is_empty() || data.len() - start < needle.len() {
        return None;
    }
    data[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + start)
}

/// `PSDVector.ascii(_:_:_:)`: bytes as ASCII, or "" when any is not.
pub(crate) fn ascii_at(data: &[u8], at: usize, count: usize) -> String {
    let slice = data.get(at..at + count).unwrap_or(&[]);
    if slice.iter().all(|&b| b.is_ascii()) {
        String::from_utf8_lossy(slice).into_owned()
    } else {
        String::new()
    }
}
