//! Port of the document model: `EditorSession.swift` (`ImageLayer`,
//! `CanvasDocument`, `NavigationTool`), `ColorPalette.swift` (`PaletteColor`),
//! `Guides.swift` (`CanvasGuide`, `LayoutGrid`), `LayerMask.swift`
//! (`LayerMask`), `TypeTool.swift` (`LayerText`), `ShapeTool.swift`
//! (`LayerShape`), `LayerAdjustment.swift`, `LayerEffects.swift`.

use crate::geometry::{CGFloat, CGPoint, CGSize};
use crate::raster::{Bitmap, ImportedImage};
use crate::selection::DocumentSelection;
use crate::transform::LayerTransform;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

pub use crate::blend::LayerBlendMode;

/// `PaletteColor`: a straight sRGB color, 0–1 per channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaletteColor {
    pub red: CGFloat,
    pub green: CGFloat,
    pub blue: CGFloat,
}

impl Default for PaletteColor {
    fn default() -> Self {
        PaletteColor::BLACK
    }
}

impl PaletteColor {
    pub const BLACK: PaletteColor = PaletteColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
    };
    pub const WHITE: PaletteColor = PaletteColor {
        red: 1.0,
        green: 1.0,
        blue: 1.0,
    };

    pub fn new(red: CGFloat, green: CGFloat, blue: CGFloat) -> Self {
        PaletteColor { red, green, blue }
    }

    /// Snaps to the 8-bit values that painting and export actually store.
    pub fn quantized(&self) -> PaletteColor {
        PaletteColor {
            red: (self.red * 255.0).round() / 255.0,
            green: (self.green * 255.0).round() / 255.0,
            blue: (self.blue * 255.0).round() / 255.0,
        }
    }

    pub fn hex(&self) -> String {
        format!(
            "{:02X}{:02X}{:02X}",
            (self.red * 255.0).round() as u8,
            (self.green * 255.0).round() as u8,
            (self.blue * 255.0).round() as u8
        )
    }

    /// Accepts `RRGGBB` or shorthand `RGB`, with or without a leading `#`.
    pub fn from_hex(text: &str) -> Option<PaletteColor> {
        let text = text.trim();
        let text = text.strip_prefix('#').unwrap_or(text);
        let expanded: String = if text.len() == 3 {
            text.chars().flat_map(|c| [c, c]).collect()
        } else {
            text.to_string()
        };
        if expanded.len() != 6 {
            return None;
        }
        let value = u32::from_str_radix(&expanded, 16).ok()?;
        Some(PaletteColor {
            red: ((value >> 16) & 0xFF) as f64 / 255.0,
            green: ((value >> 8) & 0xFF) as f64 / 255.0,
            blue: (value & 0xFF) as f64 / 255.0,
        })
    }
}

/// `Hue in degrees, saturation and brightness 0…1` (`PickerHSB`), the color
/// picker's source of truth so hue survives dragging through grays and black.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickerHSB {
    pub hue: CGFloat,
    pub saturation: CGFloat,
    pub brightness: CGFloat,
}

impl PickerHSB {
    pub fn new(hue: CGFloat, saturation: CGFloat, brightness: CGFloat) -> Self {
        PickerHSB {
            hue,
            saturation,
            brightness,
        }
    }

    pub fn from_rgb(color: PaletteColor) -> Self {
        let mut hsb = PickerHSB {
            hue: 0.0,
            saturation: 0.0,
            brightness: 0.0,
        };
        hsb.set_rgb(color);
        hsb
    }

    pub fn rgb(&self) -> PaletteColor {
        let h = ((self.hue % 360.0 + 360.0) % 360.0) / 60.0;
        let c = self.brightness * self.saturation;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let m = self.brightness - c;
        let (r, g, b) = match h as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        PaletteColor::new(r + m, g + m, b + m)
    }

    /// Updates from RGB while keeping the previous hue for grays and the
    /// previous saturation for black.
    pub fn set_rgb(&mut self, color: PaletteColor) {
        let high = color.red.max(color.green).max(color.blue);
        let low = color.red.min(color.green).min(color.blue);
        let delta = high - low;
        self.brightness = high;
        if high > 0.0 {
            self.saturation = delta / high;
        }
        if delta <= 0.0 {
            return;
        }
        let mut h;
        if high == color.red {
            h = (color.green - color.blue) / delta;
        } else if high == color.green {
            h = (color.blue - color.red) / delta + 2.0;
        } else {
            h = (color.red - color.green) / delta + 4.0;
        }
        h *= 60.0;
        self.hue = if h < 0.0 { h + 360.0 } else { h };
    }
}

/// A user-placed alignment line. Horizontal guides sit at a document Y;
/// vertical at a document X.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CanvasGuide {
    pub id: Uuid,
    pub axis: GuideAxis,
    /// Document pixels: Y for a horizontal guide, X for a vertical one.
    pub position: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GuideAxis {
    #[serde(rename = "horizontal")]
    Horizontal,
    #[serde(rename = "vertical")]
    Vertical,
}

impl CanvasGuide {
    pub fn new(axis: GuideAxis, position: f64) -> Self {
        CanvasGuide {
            id: Uuid::new_v4(),
            axis,
            position,
        }
    }

    pub fn offset(&self, x: CGFloat, y: CGFloat) -> CanvasGuide {
        let mut guide = *self;
        guide.position += if guide.axis == GuideAxis::Vertical { x } else { y };
        guide
    }

    pub fn scaled(&self, x: CGFloat, y: CGFloat) -> CanvasGuide {
        let mut guide = *self;
        guide.position *= if guide.axis == GuideAxis::Vertical { x } else { y };
        guide
    }

    /// Mirrors this guide when it runs perpendicular to the flip.
    pub fn mirrored(&self, horizontally: bool, center: CGFloat) -> CanvasGuide {
        let mut guide = *self;
        if (horizontally && guide.axis == GuideAxis::Vertical)
            || (!horizontally && guide.axis == GuideAxis::Horizontal)
        {
            guide.position = 2.0 * center - guide.position;
        }
        guide
    }
}

/// Non-printing layout grid: a major line every 64 px, eight subdivisions.
pub struct LayoutGrid;

impl LayoutGrid {
    pub const SPACING: CGFloat = 64.0;
    pub const SUBDIVISIONS: usize = 8;

    pub fn step() -> CGFloat {
        Self::SPACING / Self::SUBDIVISIONS as CGFloat
    }

    /// Every grid line along a document edge, including subdivisions, in whole pixels.
    pub fn lines(along: CGFloat) -> Vec<CGFloat> {
        let step = Self::step();
        if along < 0.0 || step <= 0.0 {
            return vec![0.0];
        }
        let mut result = Vec::new();
        let mut value: CGFloat = 0.0;
        while value <= along + 0.001 {
            result.push(value.round());
            value += step;
        }
        result
    }

    pub fn is_major(value: CGFloat) -> bool {
        (value.round() % Self::SPACING as f64).abs() < 0.001
    }
}

/// Immutable, normalized layer-local coverage. Regular grayscale images use
/// white for reveal, black for hide, and intermediate gray for soft coverage.
/// Equality is the Swift one: the mask *image* compares by identity (`===`),
/// the rest by value.
#[derive(Debug, Clone)]
pub struct LayerMask {
    pub asset: ImportedImage,
    pub is_enabled: bool,
    /// Where the mask sits on the document once it has been moved apart from
    /// its layer; None while it covers the layer's own pixel grid.
    pub placement: Option<LayerTransform>,
    /// Linked, layer and mask move together; unlinked, each transforms on its own.
    pub is_linked: bool,
}

impl PartialEq for LayerMask {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.asset.image, &other.asset.image)
            && self.is_enabled == other.is_enabled
            && self.placement == other.placement
            && self.is_linked == other.is_linked
    }
}

impl LayerMask {
    pub fn solid(revealing: bool) -> LayerMask {
        let image = Bitmap::new_clearing(1, 1, [
            if revealing { 255 } else { 0 },
            if revealing { 255 } else { 0 },
            if revealing { 255 } else { 0 },
            255,
        ]);
        LayerMask {
            asset: ImportedImage::new(image, "Layer Mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        }
    }

    /// The same mask with new pixels (in its own grid).
    pub fn replacing(&self, asset: ImportedImage) -> LayerMask {
        LayerMask {
            asset,
            is_enabled: self.is_enabled,
            placement: self.placement,
            is_linked: self.is_linked,
        }
    }

    /// Where the mask sits once its layer moves from `old` to `new`.
    pub fn placement_moving_layer(&self, old: &LayerTransform, new: &LayerTransform) -> Option<LayerTransform> {
        // A uniform mask looks the same wherever it sits.
        if self.asset.image.width <= 1 && self.asset.image.height <= 1 {
            return None;
        }
        let moved = if self.is_linked {
            self.placement.map(|p| p.following(old, new))
        } else {
            Some(self.placement.unwrap_or(*old))
        };
        moved.filter(|m| !m.same_placement(new))
    }

    /// What a mask shows beyond its pixels once placed apart from its layer:
    /// white or black, whichever most of its edge is.
    pub fn background_of(thumbnail: &Bitmap) -> CGFloat {
        let mut total = 0u64;
        let mut count = 0u64;
        for y in 0..thumbnail.height {
            for x in 0..thumbnail.width {
                if y == 0 || y == thumbnail.height - 1 || x == 0 || x == thumbnail.width - 1 {
                    total += thumbnail.pixel(x, y)[0] as u64;
                    count += 1;
                }
            }
        }
        if total * 2 >= count * 255 {
            1.0
        } else {
            0.0
        }
    }
}

/// Port of `LayerText` (`TypeTool.swift`). Equality: style by value, image
/// by identity.
#[derive(Debug, Clone)]
pub struct LayerText {
    pub style: LayerTextStyle,
    pub image: Arc<Bitmap>,
}

impl PartialEq for LayerText {
    fn eq(&self, other: &Self) -> bool {
        self.style == other.style && std::sync::Arc::ptr_eq(&self.image, &other.image)
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextAlignment {
    #[default]
    #[serde(rename = "Left")]
    Left,
    #[serde(rename = "Center")]
    Center,
    #[serde(rename = "Right")]
    Right,
}

/// Port of `LayerTextStyle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerTextStyle {
    #[serde(default = "default_text_content")]
    pub content: String,
    #[serde(default = "default_text_font")]
    pub font_name: String,
    #[serde(default = "default_text_size")]
    pub font_size: CGFloat,
    #[serde(default)]
    pub red: CGFloat,
    #[serde(default)]
    pub green: CGFloat,
    #[serde(default)]
    pub blue: CGFloat,
    #[serde(default)]
    pub alignment: TextAlignment,
    #[serde(default)]
    pub tracking: CGFloat,
    /// Baseline to baseline, in layer pixels. 0 is Auto: 120% of the font size.
    #[serde(default)]
    pub leading: CGFloat,
    /// Fixed paragraph bounds in layer pixels. None supports older point text.
    #[serde(default)]
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

impl Default for LayerTextStyle {
    fn default() -> Self {
        LayerTextStyle {
            content: "Text".to_string(),
            font_name: "Helvetica".to_string(),
            font_size: 72.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alignment: TextAlignment::Left,
            tracking: 0.0,
            leading: 0.0,
            box_size: None,
        }
    }
}

impl LayerTextStyle {
    /// The gap between the text and its box, in layer pixels.
    pub const PADDING: CGFloat = 12.0;

    pub fn auto_leading(&self) -> CGFloat {
        self.font_size * 1.2
    }

    pub fn line_height(&self) -> CGFloat {
        if self.leading > 0.0 {
            self.leading
        } else {
            self.auto_leading()
        }
    }

    pub fn box_is_valid(&self) -> bool {
        match self.box_size {
            None => true,
            Some(box_size) => {
                box_size.width.is_finite()
                    && box_size.height.is_finite()
                    && (16.0..=30_000.0).contains(&box_size.width)
                    && (16.0..=30_000.0).contains(&box_size.height)
                    && box_size.width * box_size.height <= 100_000_000.0
            }
        }
    }

    pub fn is_valid(&self) -> bool {
        self.content.chars().count() <= 100_000
            && self.box_is_valid()
            && self.font_size.is_finite()
            && (1.0..=2000.0).contains(&self.font_size)
            && [self.red, self.green, self.blue]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.tracking.is_finite()
            && (-100.0..=1000.0).contains(&self.tracking)
            && self.leading.is_finite()
            && (0.0..=5000.0).contains(&self.leading)
    }
}

/// Port of `LayerShape` (`ShapeTool.swift`): a rectangle, rounded rectangle,
/// ellipse or line the Shape tool drew, kept editable rather than rasterized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerShape {
    pub kind: ShapeKind,
    /// Corner radius in pixels for rectangles.
    #[serde(default)]
    pub corner_radius: CGFloat,
    /// A Line's thickness in document pixels.
    #[serde(default = "default_line_width")]
    pub line_width: CGFloat,
    pub red: CGFloat,
    pub green: CGFloat,
    pub blue: CGFloat,
    #[serde(default = "default_true")]
    pub filled: bool,
    /// A Line's ends as fractions of the layer box, so a scaled line still
    /// runs between the same two places (`LayerShapeStyle.start`/`end`).
    #[serde(default)]
    pub start: CGPoint,
    #[serde(default = "default_line_end")]
    pub end: CGPoint,
}

fn default_line_end() -> CGPoint {
    CGPoint::new(1.0, 1.0)
}

impl LayerShape {
    /// A shape can cover up to 100 megapixels (`maxShapePixels`).
    pub const MAX_SHAPE_PIXELS: usize = 100_000_000;
}

fn default_line_width() -> CGFloat {
    4.0
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShapeKind {
    #[default]
    #[serde(rename = "Rectangle")]
    Rectangle,
    #[serde(rename = "Rounded Rectangle")]
    RoundedRectangle,
    #[serde(rename = "Ellipse")]
    Ellipse,
    #[serde(rename = "Line")]
    Line,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 4] = [
        ShapeKind::Rectangle,
        ShapeKind::RoundedRectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::RoundedRectangle => "Rounded Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Line => "Line",
        }
    }
}

/// Port of `ImageLayer` (`EditorSession.swift`). Equality follows Swift's
/// `ImageLayer.==`: the asset's image compares by identity (`===`), the rest
/// by value.
#[derive(Debug, Clone)]
pub struct ImageLayer {
    pub id: Uuid,
    pub asset: Option<ImportedImage>,
    pub transform: LayerTransform,
    pub name: String,
    pub is_visible: bool,
    pub parent_id: Option<Uuid>,
    pub is_group: bool,
    pub opacity: f64,
    pub blend_mode: LayerBlendMode,
    pub mask_source_id: Option<Uuid>,
    pub mask: Option<LayerMask>,
    pub adjustment: Option<crate::adjustment::LayerAdjustment>,
    /// Set on layers the Shape tool made.
    pub shape: Option<LayerShape>,
    pub effects: Option<crate::effects::LayerEffects>,
    pub text: Option<LayerText>,
}

impl PartialEq for ImageLayer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.is_visible == other.is_visible
            && self.transform == other.transform
            && match (&self.asset, &other.asset) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(&a.image, &b.image),
                (None, None) => true,
                _ => false,
            }
            && self.parent_id == other.parent_id
            && self.is_group == other.is_group
            && self.opacity == other.opacity
            && self.blend_mode == other.blend_mode
            && self.mask == other.mask
            && self.mask_source_id == other.mask_source_id
            && self.adjustment == other.adjustment
            && self.shape == other.shape
            && self.text == other.text
            && self.effects == other.effects
    }
}

impl ImageLayer {
    /// A layer holding imported pixels.
    pub fn with_asset(asset: ImportedImage, origin: CGPoint) -> Self {
        let size = CGSize::new(asset.image.width as CGFloat, asset.image.height as CGFloat);
        ImageLayer {
            id: Uuid::new_v4(),
            asset: Some(asset.clone()),
            transform: LayerTransform::new(origin, size),
            name: asset.name.clone(),
            is_visible: true,
            parent_id: None,
            is_group: false,
            opacity: 1.0,
            blend_mode: LayerBlendMode::Normal,
            mask_source_id: None,
            mask: None,
            adjustment: None,
            shape: None,
            effects: None,
            text: None,
        }
    }

    /// A blank layer; pixels are allocated when painting begins.
    pub fn blank(name: impl Into<String>, blank_size: CGSize) -> Self {
        ImageLayer {
            id: Uuid::new_v4(),
            asset: None,
            transform: LayerTransform::new(CGPoint::ZERO, blank_size),
            name: name.into(),
            is_visible: true,
            parent_id: None,
            is_group: false,
            opacity: 1.0,
            blend_mode: LayerBlendMode::Normal,
            mask_source_id: None,
            mask: None,
            adjustment: None,
            shape: None,
            effects: None,
            text: None,
        }
    }

    pub fn origin(&self) -> CGPoint {
        self.transform.origin
    }

    pub fn size(&self) -> CGSize {
        self.transform.size
    }

    /// Where the mask's pixels sit on the document: its own placement, else the layer's.
    pub fn mask_transform(&self) -> LayerTransform {
        self.mask
            .as_ref()
            .and_then(|m| m.placement)
            .unwrap_or(self.transform)
    }

    /// A text layer that is still text: its cached raster must be its pixels.
    pub fn live_text(&self) -> Option<&LayerText> {
        let text = self.text.as_ref()?;
        let asset = self.asset.as_ref()?;
        if Arc::ptr_eq(&text.image, &asset.image) {
            Some(text)
        } else {
            None
        }
    }
}

/// Port of `CanvasDocument`.
#[derive(Clone, PartialEq)]
pub struct CanvasDocument {
    pub id: Uuid,
    pub width: usize,
    pub height: usize,
    pub resolution: f64,
    /// Bottom to top.
    pub layers: Vec<ImageLayer>,
    /// User-placed alignment lines. Saved with the project; undo covers them.
    pub guides: Vec<CanvasGuide>,
    /// Part of the document so undo/redo covers selection changes. Not saved to disk.
    pub selection: Option<DocumentSelection>,
}

/// `LayerHierarchy.Entry` — a Layers-panel row: the layer, its depth, and
/// whether it shows.
pub struct HierarchyEntry<'a> {
    pub layer: &'a ImageLayer,
    pub depth: usize,
    pub visible: bool,
}

impl CanvasDocument {
    pub fn new(width: usize, height: usize) -> Self {
        CanvasDocument {
            id: Uuid::new_v4(),
            width,
            height,
            resolution: 72.0,
            layers: Vec::new(),
            guides: Vec::new(),
            selection: None,
        }
    }

    pub fn size(&self) -> CGSize {
        CGSize::new(self.width as CGFloat, self.height as CGFloat)
    }

    /// Geometry limit (`validDimension`).
    pub fn valid_dimension(value: &str) -> Option<usize> {
        let trimmed = value.trim();
        match trimmed.parse::<usize>() {
            Ok(n) if (1..=30_000).contains(&n) => Some(n),
            _ => None,
        }
    }

    pub fn layer(&self, id: Uuid) -> Option<&ImageLayer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: Uuid) -> Option<&mut ImageLayer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn layer_index(&self, id: Uuid) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// IDs of layers that are visible, counting folders: a hidden folder hides
    /// everything inside it without changing the children's flags.
    pub fn effective_visible_ids(&self) -> HashSet<Uuid> {
        let parents: std::collections::HashMap<Uuid, Option<Uuid>> =
            self.layers.iter().map(|l| (l.id, l.parent_id)).collect();
        let mut visible = HashSet::new();
        for layer in &self.layers {
            let mut current = Some(layer.id);
            let mut steps = 0;
            let mut ok = true;
            while let Some(id) = current {
                if steps > 64 {
                    ok = false;
                    break;
                }
                if let Some(l) = self.layer(id) {
                    if !l.is_visible {
                        ok = false;
                        break;
                    }
                }
                current = parents.get(&id).copied().flatten();
                steps += 1;
            }
            if ok {
                visible.insert(layer.id);
            }
        }
        visible
    }

    /// `LayerHierarchy.entries`: depth-first, parents before children,
    /// siblings in array order (top first reverses siblings only). Groups list
    /// even when collapsed; their children are then omitted. An invisible
    /// folder marks its subtree invisible without hiding the folder itself.
    pub fn hierarchy_entries(
        &self,
        top_first: bool,
        collapsed: &std::collections::HashSet<Uuid>,
    ) -> Vec<HierarchyEntry<'_>> {
        let mut children: std::collections::HashMap<Option<Uuid>, Vec<&ImageLayer>> =
            std::collections::HashMap::new();
        for layer in &self.layers {
            children.entry(layer.parent_id).or_default().push(layer);
        }
        let mut out: Vec<HierarchyEntry<'_>> = Vec::new();
        let mut stack = vec![(None, 0usize, true)];
        while let Some((parent, depth, visible)) = stack.pop() {
            let Some(siblings) = children.get(&parent) else {
                continue;
            };
            let mut ordered: Vec<&ImageLayer> = siblings.iter().copied().collect();
            if top_first {
                ordered.reverse();
            }
            // Push in reverse so the first sibling is visited first.
            for layer in ordered.iter().rev() {
                let effective = visible && layer.is_visible;
                out.push(HierarchyEntry {
                    layer,
                    depth,
                    visible: effective,
                });
                if layer.is_group && !collapsed.contains(&layer.id) {
                    stack.push((Some(layer.id), depth + 1, effective));
                }
            }
        }
        out
    }

    /// Depth-first draw order (`CanvasDocument.renderLayers`): parents before
    /// children, siblings in array order, folders skipped, and any layer under
    /// a missing or hidden ancestor skipped with it.
    pub fn render_layers(&self) -> Vec<&ImageLayer> {
        let mut children: std::collections::HashMap<Option<Uuid>, Vec<&ImageLayer>> =
            std::collections::HashMap::new();
        for layer in &self.layers {
            children.entry(layer.parent_id).or_default().push(layer);
        }
        let mut out = Vec::new();
        visit_depth_first(None, &children, &mut out, 0);
        out
    }

    /// Every descendant of `id`, including itself.
    pub fn descendant_ids(&self, id: Uuid) -> HashSet<Uuid> {
        let mut result = HashSet::new();
        result.insert(id);
        // Multiple passes so arbitrarily deep trees fold in.
        loop {
            let mut grew = false;
            for layer in &self.layers {
                if let Some(parent) = layer.parent_id {
                    if result.contains(&parent) && result.insert(layer.id) {
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        result
    }
}

/// `LayerHierarchy.entries` walk: one level of the depth-first render order.
fn visit_depth_first<'a>(
    parent: Option<Uuid>,
    children: &std::collections::HashMap<Option<Uuid>, Vec<&'a ImageLayer>>,
    out: &mut Vec<&'a ImageLayer>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    if let Some(siblings) = children.get(&parent) {
        for layer in siblings {
            if !layer.is_visible {
                continue;
            }
            if !layer.is_group {
                out.push(layer);
            }
            visit_depth_first(Some(layer.id), children, out, depth + 1);
        }
    }
}

/// The tools in the tool rail (`NavigationTool`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NavigationTool {
    #[default]
    Move,
    Marquee,
    Lasso,
    Wand,
    Crop,
    Brush,
    SpotHealing,
    CloneStamp,
    Blur,
    Gradient,
    Shape,
    Type,
    Eyedropper,
    Hand,
    Zoom,
    /// No tool (A): nothing in the tool rail is selected and canvas clicks do nothing.
    Idle,
}

impl NavigationTool {
    pub const ALL: [NavigationTool; 15] = [
        NavigationTool::Move,
        NavigationTool::Marquee,
        NavigationTool::Lasso,
        NavigationTool::Wand,
        NavigationTool::Crop,
        NavigationTool::Brush,
        NavigationTool::SpotHealing,
        NavigationTool::CloneStamp,
        NavigationTool::Blur,
        NavigationTool::Gradient,
        NavigationTool::Shape,
        NavigationTool::Type,
        NavigationTool::Eyedropper,
        NavigationTool::Hand,
        NavigationTool::Zoom,
    ];

    /// Tools that paint with the brush tip, sharing its size, hardness, opacity, and keys.
    pub fn is_brush_tool(&self) -> bool {
        matches!(
            self,
            NavigationTool::Brush
                | NavigationTool::SpotHealing
                | NavigationTool::CloneStamp
                | NavigationTool::Blur
        )
    }

    /// Tools that draw and edit selections, sharing modifiers, moving, and nudging.
    pub fn is_selection_tool(&self) -> bool {
        matches!(
            self,
            NavigationTool::Marquee | NavigationTool::Lasso | NavigationTool::Wand
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            NavigationTool::Type => "Type (T)",
            NavigationTool::Eyedropper => "Eyedropper (I)",
            NavigationTool::Marquee => "Marquee (M)",
            NavigationTool::Lasso => "Lasso (L)",
            NavigationTool::Wand => "Magic (W) · Tab switches Wand and Object",
            NavigationTool::Brush => "Brush (B) · Eraser (E)",
            NavigationTool::SpotHealing => "Spot Healing Brush (J)",
            NavigationTool::CloneStamp => "Clone Stamp (S) · Option-click sets the source",
            NavigationTool::Blur => "Smear (R)",
            NavigationTool::Gradient => "Gradient (G)",
            NavigationTool::Shape => "Shape (U) · Shift-U switches Rectangle/Ellipse",
            NavigationTool::Crop => "Crop (C)",
            NavigationTool::Move => "Move / Transform (V)",
            NavigationTool::Hand => "Hand (H)",
            // Swift's label chain falls through to Zoom for idle too.
            NavigationTool::Zoom | NavigationTool::Idle => "Zoom (Z)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_mask_equality_is_image_identity() {
        let image = Bitmap::new(4, 4);
        let make = |image: Bitmap| crate::document::LayerMask {
            asset: crate::raster::ImportedImage::new(image, "M"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        };
        let a = make(image.clone());
        let b = make(image.clone());
        assert_ne!(a, b, "same bytes, different mask");
        assert_eq!(a, a.clone(), "the same mask equals itself");
    }

    #[test]
    fn layer_text_equality_is_image_identity() {
        let make = |image: Bitmap| LayerText {
            style: LayerTextStyle::default(),
            image: std::sync::Arc::new(image),
        };
        let a = make(Bitmap::new(4, 4));
        let b = make(Bitmap::new(4, 4));
        assert_ne!(a, b);
        assert_eq!(a, a.clone(), "the same image equals itself");
    }

    fn palette_hex_roundtrip() {
        let c = PaletteColor::new(1.0, 0.5, 0.0);
        let hex = c.hex();
        assert_eq!(hex, "FF8000");
        let back = PaletteColor::from_hex(&hex).unwrap();
        assert_eq!(back.quantized(), c.quantized());
        assert_eq!(PaletteColor::from_hex("#F0B").unwrap(), PaletteColor::from_hex("FF00BB").unwrap());
        assert!(PaletteColor::from_hex("zzzzzz").is_none());
    }

    #[test]
    fn picker_hsb_roundtrip() {
        let c = PaletteColor::new(0.2, 0.4, 0.8);
        let hsb = PickerHSB::from_rgb(c);
        let back = hsb.rgb();
        assert!((back.red - c.red).abs() < 1e-9);
        assert!((back.green - c.green).abs() < 1e-9);
        assert!((back.blue - c.blue).abs() < 1e-9);
    }

    #[test]
    fn picker_hsb_keeps_hue_through_gray() {
        let mut hsb = PickerHSB::from_rgb(PaletteColor::new(1.0, 0.0, 0.0));
        assert_eq!(hsb.hue, 0.0);
        hsb.set_rgb(PaletteColor::new(0.5, 0.5, 0.5));
        // Gray keeps the previous hue and drops saturation.
        assert_eq!(hsb.hue, 0.0);
        assert!((hsb.saturation - 0.5 / 0.5 + 1.0).abs() > 0.0 || hsb.saturation == 0.0);
        assert!((hsb.saturation - 0.0).abs() < 1e-9);
    }

    #[test]
    fn guides_offset_scale_mirror() {
        let g = CanvasGuide::new(GuideAxis::Vertical, 100.0);
        assert_eq!(g.offset(10.0, 5.0).position, 110.0);
        assert_eq!(g.scaled(2.0, 2.0).position, 200.0);
        assert_eq!(g.mirrored(true, 50.0).position, 0.0);
        let h = CanvasGuide::new(GuideAxis::Horizontal, 100.0);
        assert_eq!(h.offset(10.0, 5.0).position, 105.0);
        assert_eq!(h.mirrored(true, 50.0).position, 100.0);
    }

    #[test]
    fn layout_grid_lines() {
        let lines = LayoutGrid::lines(20.0);
        assert_eq!(lines, vec![0.0, 8.0, 16.0]);
        assert!(!LayoutGrid::is_major(8.0));
        assert!(LayoutGrid::is_major(64.0));
    }

    #[test]
    fn valid_dimension_bounds() {
        assert_eq!(CanvasDocument::valid_dimension("100"), Some(100));
        assert_eq!(CanvasDocument::valid_dimension(" 100 "), Some(100));
        assert_eq!(CanvasDocument::valid_dimension("0"), None);
        assert_eq!(CanvasDocument::valid_dimension("30001"), None);
        assert_eq!(CanvasDocument::valid_dimension("abc"), None);
    }

    #[test]
    fn effective_visibility_inherits() {
        let mut doc = CanvasDocument::new(10, 10);
        let mut group = ImageLayer::blank("Group", doc.size());
        group.is_group = true;
        let group_id = group.id;
        let mut child = ImageLayer::blank("Layer", doc.size());
        child.parent_id = Some(group_id);
        let child_id = child.id;
        doc.layers = vec![group, child];
        let visible = doc.effective_visible_ids();
        assert!(visible.contains(&group_id) && visible.contains(&child_id));
        doc.layer_mut(group_id).unwrap().is_visible = false;
        let visible = doc.effective_visible_ids();
        assert!(!visible.contains(&group_id));
        assert!(!visible.contains(&child_id));
        // Child flag unchanged.
        assert!(doc.layer(child_id).unwrap().is_visible);
    }

    #[test]
    fn descendants_fold() {
        let mut doc = CanvasDocument::new(10, 10);
        let mut a = ImageLayer::blank("A", doc.size());
        a.is_group = true;
        let mut b = ImageLayer::blank("B", doc.size());
        b.is_group = true;
        b.parent_id = Some(a.id);
        let mut c = ImageLayer::blank("C", doc.size());
        c.parent_id = Some(b.id);
        let a_id = a.id;
        doc.layers = vec![a, b, c];
        let ids = doc.descendant_ids(a_id);
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn layer_mask_placement_logic() {
        let mut layer = ImageLayer::blank("L", CGSize::new(100.0, 100.0));
        layer.transform.origin = CGPoint::new(10.0, 10.0);
        let mut big_image = Bitmap::new(50, 50);
        big_image.set_pixel(0, 0, [255, 255, 255, 255]);
        let mut mask = LayerMask::solid(true);
        mask.asset = ImportedImage::new(big_image, "Layer Mask");
        // Linked with no placement: stays nil (covering the layer).
        assert_eq!(mask.placement_moving_layer(&layer.transform, &layer.transform), None);
        let mut new_t = layer.transform;
        new_t.origin = CGPoint::new(20.0, 20.0);
        assert_eq!(mask.placement_moving_layer(&layer.transform, &new_t), None);
        // Unlinked: stays where it was (the layer's old transform).
        mask.is_linked = false;
        let placed = mask.placement_moving_layer(&layer.transform, &new_t).unwrap();
        assert_eq!(placed, layer.transform);
    }

    #[test]
    fn uniform_mask_has_no_placement() {
        let mask = LayerMask::solid(true);
        let old = LayerTransform::new(CGPoint::ZERO, CGSize::new(10.0, 10.0));
        let mut new_t = old;
        new_t.origin = CGPoint::new(5.0, 5.0);
        assert_eq!(mask.placement_moving_layer(&old, &new_t), None);
    }

    #[test]
    fn text_style_defaults_and_validation() {
        let style = LayerTextStyle::default();
        assert!(style.is_valid());
        assert_eq!(style.line_height(), 72.0 * 1.2);
        let mut big = LayerTextStyle::default();
        big.font_size = 5000.0;
        assert!(!big.is_valid());
    }

    #[test]
    fn live_text_requires_matching_image() {
        let mut layer = ImageLayer::blank("T", CGSize::new(10.0, 10.0));
        let image = std::sync::Arc::new(Bitmap::new(4, 4));
        layer.asset = Some(ImportedImage { image: image.clone(), thumbnail: std::sync::Arc::new(Bitmap::new(1, 1)), name: "T".to_string() });
        layer.text = Some(LayerText {
            style: LayerTextStyle::default(),
            image: image.clone(),
        });
        assert!(layer.live_text().is_some());
        layer.text.as_mut().unwrap().image = std::sync::Arc::new(Bitmap::new(4, 4));
        assert!(layer.live_text().is_none());
    }
}
