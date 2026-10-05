//! The `.comp` manifest JSON model, ported from `IO/ProjectStore.swift`
//! (`ProjectManifest`, `ProjectLayerRecord`) and the Codable shapes it embeds
//! (`Document/LayerTransform.swift`, `TypeTool.swift`, `ShapeTool.swift`,
//! `LayerEffects.swift`, `Guides.swift`).
//!
//! Serde keys match the Swift JSON encoder exactly (camelCase; the sampling
//! strings `Nearest`/`Smooth`/`High quality`; blend mode names like
//! `Color Burn`). UUIDs are written uppercase hyphenated, as Swift's
//! `uuidString` does, so the derived asset filenames
//! (`<UUID>.png`, `<UUID>.mask.png`) are accepted by both implementations.
//! Optional fields are omitted when nil, matching Swift's synthesized
//! `encodeIfPresent`.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

use crate::blend::LayerBlendMode;
use crate::document::{CanvasGuide, GuideAxis, LayerShape, LayerTextStyle, ShapeKind};
use crate::effects::{
    ColorOverlayEffect, InnerGlowEffect, InnerShadowEffect, LayerEffects, OuterGlowEffect,
    ShadowEffect, StrokeEffect,
};
use crate::geometry::{CGFloat, CGPoint, CGSize};
use crate::transform::{LayerSampling, LayerTransform};

/// `UTType.compositorProject`'s identifier and the manifest `format` value.
pub const FORMAT: &str = "com.compositor.project";
/// New saves write the highest version the Swift app writes.
pub const CURRENT_VERSION: i64 = 9;
/// Readable versions are 1...`MAX_VERSION` (`ProjectStore`'s `(1...9)` check).
pub const MIN_VERSION: i64 = 1;
pub const MAX_VERSION: i64 = 9;

/// `<UUID>.png` under `images/` for a layer's image asset.
pub fn image_file_name(id: Uuid) -> String {
    format!("{}.png", uuid_string(id))
}

/// `<UUID>.mask.png` under `images/` for a layer's mask asset.
pub fn mask_file_name(id: Uuid) -> String {
    format!("{}.mask.png", uuid_string(id))
}

/// A layer or guide UUID as Swift writes it: uppercase, hyphenated.
fn uuid_string(id: Uuid) -> String {
    id.to_string().to_uppercase()
}

pub(crate) mod serde_upper_uuid {
    //! UUIDs as uppercase hyphenated strings (Swift's `uuidString`); parsing is
    //! case-insensitive, exactly like `UUID(uuidString:)`.
    use super::{Deserialize, Deserializer, Serializer, Uuid, de};

    pub fn serialize<S: Serializer>(id: &Uuid, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::uuid_string(*id))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Uuid, D::Error> {
        let text = String::deserialize(d)?;
        Uuid::parse_str(&text).map_err(de::Error::custom)
    }
}

pub(crate) mod serde_upper_uuid_option {
    //! Optional UUIDs: JSON null (or a missing key) decodes to None.
    use super::{Deserialize, Deserializer, Serializer, Uuid, de};

    pub fn serialize<S: Serializer>(id: &Option<Uuid>, s: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => s.serialize_str(&super::uuid_string(*id)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Uuid>, D::Error> {
        let text = Option::<String>::deserialize(d)?;
        match text {
            None => Ok(None),
            Some(text) => Uuid::parse_str(&text).map(Some).map_err(de::Error::custom),
        }
    }
}

fn default_format() -> String {
    FORMAT.to_string()
}
fn default_version() -> i64 {
    CURRENT_VERSION
}
fn default_color_space() -> String {
    "sRGB".to_string()
}

/// `ProjectManifest`: the whole `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectManifest {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(rename = "colorSpace", default = "default_color_space")]
    pub color_space: String,
    /// Older version-1 projects default to 72 pixels/inch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<f64>,
    #[serde(rename = "documentID", with = "serde_upper_uuid")]
    pub document_id: Uuid,
    pub width: i64,
    pub height: i64,
    #[serde(
        rename = "activeLayerID",
        with = "serde_upper_uuid_option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub active_layer_id: Option<Uuid>,
    /// Bottom to top.
    #[serde(default)]
    pub layers: Vec<ProjectLayerRecord>,
    /// Alignment guides. Missing on versions 1-7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guides: Option<Vec<ManifestGuide>>,
}

impl ProjectManifest {
    pub fn new(document_id: Uuid, width: i64, height: i64) -> Self {
        ProjectManifest {
            format: default_format(),
            version: CURRENT_VERSION,
            color_space: default_color_space(),
            resolution: None,
            document_id,
            width,
            height,
            active_layer_id: None,
            layers: Vec::new(),
            guides: None,
        }
    }
}

/// `ProjectLayerRecord`: one layer, bottom to top in the array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectLayerRecord {
    #[serde(with = "serde_upper_uuid")]
    pub id: Uuid,
    pub name: String,
    #[serde(rename = "isVisible")]
    pub is_visible: bool,
    pub transform: ManifestTransform,
    /// `<UUID>.png`; blank layers and groups have no image asset.
    #[serde(rename = "imageFile", default, skip_serializing_if = "Option::is_none")]
    pub image_file: Option<String>,
    /// Version 2+; root nodes have no parent.
    #[serde(
        rename = "parentID",
        with = "serde_upper_uuid_option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_id: Option<Uuid>,
    /// Version 2+.
    #[serde(rename = "isGroup", default, skip_serializing_if = "Option::is_none")]
    pub is_group: Option<bool>,
    /// Version 3+; missing defaults to fully opaque.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Version 3+; missing defaults to Normal.
    #[serde(rename = "blendMode", default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<LayerBlendMode>,
    /// Version 4+ (6+ on groups): `<UUID>.mask.png`.
    #[serde(rename = "maskFile", default, skip_serializing_if = "Option::is_none")]
    pub mask_file: Option<String>,
    /// Version 4+; defaults to true when a mask exists.
    #[serde(
        rename = "maskEnabled",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mask_enabled: Option<bool>,
    /// Version 5+: a non-group layer supplying live alpha.
    #[serde(
        rename = "maskSourceID",
        with = "serde_upper_uuid_option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mask_source_id: Option<Uuid>,
    /// Version 7+; adjustment layers carry no image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<crate::adjustment::LayerAdjustment>,
    /// A mask moved apart from its layer: where it sits on the document.
    #[serde(
        rename = "maskPlacement",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mask_placement: Option<ManifestTransform>,
    /// Nil (older projects) is linked.
    #[serde(
        rename = "maskLinked",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mask_linked: Option<bool>,
    /// A shape layer's shape, drawn again when the layer is scaled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ManifestShape>,
    /// The stroke and drop shadow drawn around the layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<ManifestEffects>,
    /// Editable text metadata; the PNG remains the display fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<ManifestTextStyle>,
}

/// Swift's `LayerTransform` JSON: `origin` `{x, y}`, `size` `{width, height}`,
/// `rotation`, `flipX`, `flipY`, `sampling`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ManifestTransform {
    pub origin: CGPoint,
    pub size: CGSize,
    #[serde(default)]
    pub rotation: CGFloat,
    #[serde(rename = "flipX", default)]
    pub flip_x: bool,
    #[serde(rename = "flipY", default)]
    pub flip_y: bool,
    #[serde(default = "default_sampling")]
    pub sampling: LayerSampling,
}

fn default_sampling() -> LayerSampling {
    LayerSampling::High
}

impl ManifestTransform {
    /// `LayerTransform.isValid`: finite values, size 1...300,000, origin within
    /// a million pixels.
    pub fn is_valid(&self) -> bool {
        [
            self.origin.x,
            self.origin.y,
            self.size.width,
            self.size.height,
            self.rotation,
        ]
        .iter()
        .all(|v| v.is_finite())
            && (1.0..=300_000.0).contains(&self.size.width)
            && (1.0..=300_000.0).contains(&self.size.height)
            && self.origin.x.abs() <= 1_000_000.0
            && self.origin.y.abs() <= 1_000_000.0
    }
}

impl From<&LayerTransform> for ManifestTransform {
    fn from(t: &LayerTransform) -> Self {
        ManifestTransform {
            origin: t.origin,
            size: t.size,
            rotation: t.rotation,
            flip_x: t.flip_x,
            flip_y: t.flip_y,
            sampling: t.sampling,
        }
    }
}

impl From<ManifestTransform> for LayerTransform {
    fn from(m: ManifestTransform) -> Self {
        LayerTransform {
            origin: m.origin,
            size: m.size,
            rotation: m.rotation,
            flip_x: m.flip_x,
            flip_y: m.flip_y,
            sampling: m.sampling,
        }
    }
}

/// Swift's `LayerTextStyle` JSON (`fontName`, `fontSize`, `boxSize`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestTextStyle {
    #[serde(default = "default_text_content")]
    pub content: String,
    #[serde(rename = "fontName", default = "default_text_font")]
    pub font_name: String,
    #[serde(rename = "fontSize", default = "default_text_size")]
    pub font_size: CGFloat,
    #[serde(default)]
    pub red: CGFloat,
    #[serde(default)]
    pub green: CGFloat,
    #[serde(default)]
    pub blue: CGFloat,
    #[serde(default)]
    pub alignment: crate::document::TextAlignment,
    #[serde(default)]
    pub tracking: CGFloat,
    /// Baseline to baseline, in layer pixels. 0 is Auto: 120% of the font size.
    #[serde(default)]
    pub leading: CGFloat,
    /// Fixed paragraph bounds in layer pixels. None supports older point text.
    #[serde(rename = "boxSize", default, skip_serializing_if = "Option::is_none")]
    pub box_size: Option<CGSize>,
}

fn default_text_content() -> String {
    "Text".to_string()
}
fn default_text_font() -> String {
    "Helvetica".to_string()
}
fn default_text_size() -> CGFloat {
    72.0
}

impl Default for ManifestTextStyle {
    fn default() -> Self {
        ManifestTextStyle {
            content: default_text_content(),
            font_name: default_text_font(),
            font_size: default_text_size(),
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alignment: crate::document::TextAlignment::Left,
            tracking: 0.0,
            leading: 0.0,
            box_size: None,
        }
    }
}

impl From<&LayerTextStyle> for ManifestTextStyle {
    fn from(style: &LayerTextStyle) -> Self {
        ManifestTextStyle {
            content: style.content.clone(),
            font_name: style.font_name.clone(),
            font_size: style.font_size,
            red: style.red,
            green: style.green,
            blue: style.blue,
            alignment: style.alignment,
            tracking: style.tracking,
            leading: style.leading,
            box_size: style.box_size,
        }
    }
}

impl From<ManifestTextStyle> for LayerTextStyle {
    fn from(m: ManifestTextStyle) -> Self {
        LayerTextStyle {
            content: m.content,
            font_name: m.font_name,
            font_size: m.font_size,
            red: m.red,
            green: m.green,
            blue: m.blue,
            alignment: m.alignment,
            tracking: m.tracking,
            leading: m.leading,
            box_size: m.box_size,
        }
    }
}

/// Swift's `LayerShapeStyle` JSON. The Rust `LayerShape` stores a line's
/// thickness in `line_width`; the Swift-only line endpoints (fractions of the
/// layer's box) have no Rust equivalent and are kept here for fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ManifestShape {
    pub kind: ShapeKind,
    #[serde(default)]
    pub red: CGFloat,
    #[serde(default)]
    pub green: CGFloat,
    #[serde(default)]
    pub blue: CGFloat,
    /// Corner radius in pixels for rectangles.
    #[serde(rename = "cornerRadius", default)]
    pub corner_radius: CGFloat,
    /// A Line's thickness; nil on other shapes.
    #[serde(rename = "lineWidth", default, skip_serializing_if = "Option::is_none")]
    pub line_width: Option<CGFloat>,
    /// A line's start as a fraction of the layer's box (0-1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<CGPoint>,
    /// A line's end as a fraction of the layer's box (0-1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<CGPoint>,
}

impl From<&LayerShape> for ManifestShape {
    fn from(shape: &LayerShape) -> Self {
        ManifestShape {
            kind: shape.kind,
            red: shape.red,
            green: shape.green,
            blue: shape.blue,
            corner_radius: shape.corner_radius,
            line_width: (shape.kind == ShapeKind::Line).then_some(shape.line_width),
            start: None,
            end: None,
        }
    }
}

impl From<&ManifestShape> for LayerShape {
    fn from(m: &ManifestShape) -> Self {
        LayerShape {
            kind: m.kind,
            corner_radius: m.corner_radius,
            line_width: m.line_width.unwrap_or(4.0),
            red: m.red,
            green: m.green,
            blue: m.blue,
            filled: m.kind != ShapeKind::Line,
            start: crate::geometry::CGPoint::ZERO,
            end: crate::geometry::CGPoint::new(1.0, 1.0),
        }
    }
}

/// Swift's `LayerEffects` JSON. The inner effect records reuse the crate's
/// `crate::effects` types, whose field names already match Swift; only the
/// container's camelCase keys need renaming.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ManifestEffects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowEffect>,
    #[serde(
        rename = "colorOverlay",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub color_overlay: Option<ColorOverlayEffect>,
    #[serde(
        rename = "innerShadow",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub inner_shadow: Option<InnerShadowEffect>,
    #[serde(rename = "outerGlow", default, skip_serializing_if = "Option::is_none")]
    pub outer_glow: Option<OuterGlowEffect>,
    #[serde(rename = "innerGlow", default, skip_serializing_if = "Option::is_none")]
    pub inner_glow: Option<InnerGlowEffect>,
}

impl From<&LayerEffects> for ManifestEffects {
    fn from(e: &LayerEffects) -> Self {
        ManifestEffects {
            stroke: e.stroke,
            shadow: e.shadow,
            color_overlay: e.color_overlay,
            inner_shadow: e.inner_shadow,
            outer_glow: e.outer_glow,
            inner_glow: e.inner_glow,
        }
    }
}

impl From<&ManifestEffects> for LayerEffects {
    fn from(m: &ManifestEffects) -> Self {
        LayerEffects {
            stroke: m.stroke,
            shadow: m.shadow,
            color_overlay: m.color_overlay,
            inner_shadow: m.inner_shadow,
            outer_glow: m.outer_glow,
            inner_glow: m.inner_glow,
        }
    }
}

/// Swift's `CanvasGuide` JSON: `id`, `axis` (`horizontal`/`vertical`),
/// `position`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ManifestGuide {
    #[serde(with = "serde_upper_uuid")]
    pub id: Uuid,
    pub axis: GuideAxis,
    /// Document pixels: Y for a horizontal guide, X for a vertical one.
    pub position: f64,
}

impl From<&CanvasGuide> for ManifestGuide {
    fn from(guide: &CanvasGuide) -> Self {
        ManifestGuide {
            id: guide.id,
            axis: guide.axis,
            position: guide.position,
        }
    }
}

impl From<&ManifestGuide> for CanvasGuide {
    fn from(m: &ManifestGuide) -> Self {
        CanvasGuide {
            id: m.id,
            axis: m.axis,
            position: m.position,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_keys_match_swift_json() {
        let mut manifest = ProjectManifest::new(Uuid::new_v4(), 100, 80);
        let record = ProjectLayerRecord {
            id: Uuid::new_v4(),
            name: "Layer".to_string(),
            is_visible: true,
            transform: ManifestTransform::from(&LayerTransform::new(
                CGPoint::new(1.0, 2.0),
                CGSize::new(10.0, 20.0),
            )),
            image_file: None,
            parent_id: None,
            is_group: Some(false),
            opacity: Some(0.5),
            blend_mode: Some(LayerBlendMode::ColorBurn),
            mask_file: None,
            mask_enabled: None,
            mask_source_id: None,
            adjustment: None,
            mask_placement: None,
            mask_linked: None,
            shape: None,
            effects: None,
            text: None,
        };
        let id = record.id;
        manifest.layers.push(record);
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        for key in [
            "\"documentID\"",
            "\"colorSpace\"",
            "\"isVisible\"",
            "\"isGroup\"",
            "\"blendMode\": \"Color Burn\"",
        ] {
            assert!(json.contains(key), "missing {key} in:\n{json}");
        }
        // UUIDs are uppercase, image files uppercase .png, and omitted
        // optionals stay out of the JSON.
        assert!(json.contains(&format!(
            "\"documentID\": \"{}\"",
            uuid_string(manifest.document_id)
        )));
        assert!(json.contains(&format!("\"id\": \"{}\"", uuid_string(id))));
        assert!(!json.contains("maskFile"));
        assert!(!json.contains("parentID"));
        assert!(!json.contains("imageFile"));
        let back: ProjectManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, manifest);
    }

    #[test]
    fn transform_keys_match_swift_json() {
        let mut t = LayerTransform::new(CGPoint::new(1.0, 2.0), CGSize::new(30.0, 40.0));
        t.rotation = 38.0;
        t.flip_x = true;
        t.sampling = LayerSampling::High;
        let json = serde_json::to_string(&ManifestTransform::from(&t)).unwrap();
        for key in [
            "\"origin\":{\"x\":1.0,\"y\":2.0}",
            "\"size\":{\"width\":30.0,\"height\":40.0}",
            "\"rotation\":38.0",
            "\"flipX\":true",
            "\"flipY\":false",
            "\"sampling\":\"High quality\"",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }

    #[test]
    fn effects_keys_match_swift_json() {
        let mut effects = LayerEffects::default();
        effects.color_overlay = Some(ColorOverlayEffect::default());
        effects.inner_shadow = Some(InnerShadowEffect::default());
        let json = serde_json::to_string(&ManifestEffects::from(&effects)).unwrap();
        assert!(json.contains("\"colorOverlay\""), "{json}");
        assert!(json.contains("\"innerShadow\""), "{json}");
        let back: ManifestEffects = serde_json::from_str(&json).unwrap();
        assert_eq!(&LayerEffects::from(&back), &effects);
    }

    #[test]
    fn mask_file_names_are_uppercase() {
        let id = Uuid::parse_str("aabbccdd-1122-3344-5566-778899aabbcc").unwrap();
        assert_eq!(
            image_file_name(id),
            "AABBCCDD-1122-3344-5566-778899AABBCC.png"
        );
        assert_eq!(
            mask_file_name(id),
            "AABBCCDD-1122-3344-5566-778899AABBCC.mask.png"
        );
    }
}
