//! Port of `IO/CanvasResizer.swift`, `IO/ImageResizer.swift`,
//! `Document/CanvasSize.swift`, `Document/Crop.swift` and
//! `Document/ImageTrim.swift`: canvas size, image size, crop and trim.

use crate::document::{CanvasDocument, ImageLayer};
use crate::geometry::{CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use crate::raster::{Bitmap, ImportedImage};
use crate::transform::LayerTransform;
use serde::{Deserialize, Serialize};

/// `CanvasUnit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CanvasUnit {
    #[default]
    #[serde(rename = "Pixels")]
    Pixels,
    #[serde(rename = "Percent")]
    Percent,
    #[serde(rename = "Inches")]
    Inches,
    #[serde(rename = "Centimeters")]
    Centimeters,
}

/// `CanvasSizeDraft`: the Canvas Size sheet's draft math.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasSizeDraft {
    pub original_width: usize,
    pub original_height: usize,
    pub resolution: f64,
    pub width: f64,
    pub height: f64,
    pub relative: bool,
    pub locked: bool,
    pub unit: CanvasUnit,
}

impl CanvasSizeDraft {
    pub fn new(width: usize, height: usize, resolution: f64) -> Self {
        CanvasSizeDraft {
            original_width: width,
            original_height: height,
            width: width as f64,
            height: height as f64,
            resolution,
            relative: false,
            locked: false,
            unit: CanvasUnit::Pixels,
        }
    }

    pub fn valid(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && (1.0..=30_000.0).contains(&self.width.round())
            && (1.0..=30_000.0).contains(&self.height.round())
    }

    pub fn displayed(&self, width_axis: bool) -> f64 {
        let original = if width_axis {
            self.original_width
        } else {
            self.original_height
        } as f64;
        let pixels = (if width_axis { self.width } else { self.height })
            - if self.relative { original } else { 0.0 };
        match self.unit {
            CanvasUnit::Pixels => pixels,
            CanvasUnit::Percent => pixels / original * 100.0,
            CanvasUnit::Inches => pixels / self.resolution,
            CanvasUnit::Centimeters => pixels / self.resolution * 2.54,
        }
    }

    pub fn set(&mut self, value: f64, width_axis: bool) {
        let original = if width_axis {
            self.original_width
        } else {
            self.original_height
        } as f64;
        let pixels = match self.unit {
            CanvasUnit::Pixels => value,
            CanvasUnit::Percent => value / 100.0 * original,
            CanvasUnit::Inches => value * self.resolution,
            CanvasUnit::Centimeters => value / 2.54 * self.resolution,
        };
        let value = pixels + if self.relative { original } else { 0.0 };
        if width_axis {
            self.width = value;
            if self.locked {
                self.height = value * self.original_height as f64 / self.original_width as f64;
            }
        } else {
            self.height = value;
            if self.locked {
                self.width = value * self.original_width as f64 / self.original_height as f64;
            }
        }
    }
}

/// The fill color for extended canvas area (`CanvasExtensionColor`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasExtensionColor {
    pub red: CGFloat,
    pub green: CGFloat,
    pub blue: CGFloat,
}

/// `CanvasSizeOptions`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasSizeOptions {
    pub width: usize,
    pub height: usize,
    /// Row-major, top-left through bottom-right.
    pub anchor: usize,
    pub fill: Option<CanvasExtensionColor>,
    /// Crop supplies an explicit document-space translation.
    pub content_offset: Option<CGPoint>,
}

impl CanvasSizeOptions {
    pub fn new(width: usize, height: usize) -> Self {
        CanvasSizeOptions {
            width,
            height,
            anchor: 4,
            fill: None,
            content_offset: None,
        }
    }

    pub fn offset(&self, from_width: usize, old_height: usize) -> CGPoint {
        if let Some(content_offset) = self.content_offset {
            return content_offset;
        }
        // Floor puts the extra pixel on the right/bottom when expanding, and
        // removes it from the left/top when shrinking around the center.
        CGPoint::new(
            ((self.width as f64 - from_width as f64) * (self.anchor % 3) as f64 / 2.0).floor(),
            ((self.height as f64 - old_height as f64) * (self.anchor / 3) as f64 / 2.0).floor(),
        )
    }
}

/// `CanvasResizer.resize`: re-places every layer for the new canvas size. Layer
/// pixels are preserved; the content shifts by the offset and the extension
/// area stays empty (the sheet's fill color is a document background).
pub fn resize_canvas(
    document: &CanvasDocument,
    options: &CanvasSizeOptions,
) -> Result<CanvasDocument, String> {
    // Defensive checks (`CanvasResizer.resize`).
    if !(1..=30_000).contains(&options.width)
        || !(1..=30_000).contains(&options.height)
        || options.anchor > 8
    {
        return Err("the canvas is too large".to_string());
    }
    let offset = options.offset(document.width, document.height);
    if !offset.x.is_finite()
        || !offset.y.is_finite()
        || offset.x.abs() > 1_000_000.0
        || offset.y.abs() > 1_000_000.0
    {
        return Err("the canvas offset is out of range".to_string());
    }
    // Nothing to do keeps everything, the selection included.
    if options.width == document.width
        && options.height == document.height
        && offset.x == 0.0
        && offset.y == 0.0
    {
        return Ok(document.clone());
    }
    let mut result = document.clone();
    result.width = options.width;
    result.height = options.height;
    for layer in result.layers.iter_mut() {
        layer.transform.origin = layer.transform.origin + offset;
        if !layer.transform.is_valid() {
            return Err("a layer would land out of range".to_string());
        }
        if let Some(mask) = layer.mask.as_mut()
            && let Some(placement) = mask.placement.as_mut()
        {
            placement.origin = placement.origin + offset;
        }
    }
    result.guides = document
        .guides
        .iter()
        .map(|g| g.offset(offset.x, offset.y))
        .collect();
    result.selection = None;
    // A colored extension is separate bottom-layer content; the old canvas
    // intersection stays transparent, holes in the artwork included.
    if let Some(color) = options.fill {
        let grew = options.width > document.width || options.height > document.height;
        if grew {
            let used: u64 = document
                .layers
                .iter()
                .filter_map(|l| l.asset.as_ref())
                .map(|a| a.image.width as u64 * a.image.height as u64)
                .sum();
            // Saturate: a session can hold assets beyond the store's load-time
            // budget, and `100_000_000 - used` must not underflow.
            if used >= 100_000_000
                || options.width * options.height > (100_000_000 - used) as usize
                || result.layers.len() >= 10_000
            {
                return Err("the canvas is too large".to_string());
            }
            if ![color.red, color.green, color.blue]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            {
                return Err("the extension color is out of range".to_string());
            }
            let mut extension = Bitmap::new(options.width, options.height);
            let fill = [
                ((color.red * 255.0).round() as f32 / 255.0),
                ((color.green * 255.0).round() as f32 / 255.0),
                ((color.blue * 255.0).round() as f32 / 255.0),
            ];
            for pixel in extension.data.chunks_exact_mut(4) {
                let a = 255.0f32;
                for c in 0..3 {
                    pixel[c] = (fill[c] * a).round() as u8;
                }
                pixel[3] = a.round() as u8;
            }
            // The old canvas area stays transparent, holes included.
            {
                let clear_rect = CGRect::new(
                    offset.x,
                    offset.y,
                    document.width as f64,
                    document.height as f64,
                );
                let x_lo = clear_rect.min_x().max(0.0) as usize;
                let y_lo = clear_rect.min_y().max(0.0) as usize;
                let x_hi = (clear_rect.max_x().ceil() as usize).min(extension.width);
                let y_hi = (clear_rect.max_y().ceil() as usize).min(extension.height);
                for y in y_lo..y_hi {
                    for x in x_lo..x_hi {
                        let o = (y * extension.width + x) * 4;
                        extension.data[o..o + 4].fill(0);
                    }
                }
            }
            let thumbnail = crate::raster::thumbnail_of(&extension);
            let mut layer = ImageLayer::with_asset(
                ImportedImage::with_thumbnail(extension, thumbnail, "Canvas Extension"),
                CGPoint::ZERO,
            );
            layer.name = "Canvas Extension".to_string();
            layer.transform = LayerTransform::new(
                CGPoint::ZERO,
                CGSize::new(options.width as f64, options.height as f64),
            );
            result.layers.insert(0, layer);
        }
    }
    Ok(result)
}

/// `ImageResizer.resize`: resamples every layer (and mask) into its scaled
/// bounding box — a rotated layer lands axis-aligned, with the requested
/// sampling. Uniform 1×1 masks and masks placed apart keep their pixels; the
/// placement scales with the canvas. Same-size requests only set the
/// resolution. Returns None when a dimension, resolution, or pixel budget is
/// out of range.
pub fn resize_image(
    document: &CanvasDocument,
    new_width: usize,
    new_height: usize,
    resolution: Option<f64>,
    sampling: crate::transform::LayerSampling,
) -> Option<CanvasDocument> {
    if !(1..=30_000).contains(&new_width) || !(1..=30_000).contains(&new_height) {
        return None;
    }
    if let Some(dpi) = resolution
        && (!dpi.is_finite() || !(1.0..=9600.0).contains(&dpi))
    {
        return None;
    }
    if new_width == document.width && new_height == document.height {
        // Same size: only the resolution changes; layers, guides and the
        // selection stay put.
        let mut same = document.clone();
        if let Some(dpi) = resolution {
            same.resolution = dpi;
        }
        return Some(same);
    }
    if new_width * new_height > 100_000_000 {
        return None;
    }
    let sx = new_width as f64 / document.width as f64;
    let sy = new_height as f64 / document.height as f64;
    let mut result = document.clone();
    result.width = new_width;
    result.height = new_height;
    if let Some(dpi) = resolution {
        result.resolution = dpi;
    }
    result.guides = document.guides.iter().map(|g| g.scaled(sx, sy)).collect();
    result.selection = None;
    let scaling = CGAffineTransform::scale(sx, sy);
    let mut used_pixels = 0u64;
    let mut used_mask_pixels = 0u64;
    for layer in result.layers.iter_mut() {
        // Each transformed layer rasterizes into its scaled bounding box:
        // nonuniform scaling of a rotated box would shear a
        // width/height/angle transform.
        let corners: Vec<CGPoint> = layer
            .transform
            .corners()
            .iter()
            .map(|p| CGPoint::new(p.x * sx, p.y * sy))
            .collect();
        let left = corners
            .iter()
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min)
            .floor();
        let top = corners
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min)
            .floor();
        let width = (corners
            .iter()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            - left) as usize;
        let height = (corners
            .iter()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            - top) as usize;
        let mut transform = LayerTransform::new(
            CGPoint::new(left, top),
            CGSize::new(width as f64, height as f64),
        );
        transform.sampling = sampling;
        if !transform.is_valid() || width == 0 || height == 0 {
            return None;
        }
        if layer.asset.is_some() {
            if !(1..=30_000).contains(&width)
                || !(1..=30_000).contains(&height)
                || used_pixels >= 100_000_000
                || width as u64 * height as u64 > 100_000_000 - used_pixels
            {
                return None;
            }
            used_pixels += width as u64 * height as u64;
            if let Some(asset) = layer.asset.as_ref() {
                let source_transform = LayerTransform {
                    sampling,
                    ..layer.transform
                };
                let mapping = scaling
                    .concat(&crate::raster::pixel_to_document(
                        &source_transform,
                        asset.image.width as f64,
                        asset.image.height as f64,
                    ))
                    .translate(-left, -top);
                let mut out = Bitmap::new(width, height);
                crate::raster::draw_affine(
                    &mut out,
                    asset.image.as_ref(),
                    &mapping,
                    sampling,
                    None,
                );
                let thumbnail = crate::raster::thumbnail_of(&out);
                layer.asset = Some(ImportedImage::with_thumbnail(
                    out,
                    thumbnail,
                    asset.name.clone(),
                ));
            }
        }
        // Keep the old transform until the mask below has been resampled from
        // it: the mask pixels live on the layer's old pixel grid.
        if let Some(mask) = layer.mask.as_mut() {
            let uniform = mask.asset.image.width == 1 && mask.asset.image.height == 1;
            if uniform || mask.placement.is_some() {
                // Uniform masks are resolution independent; a mask placed
                // apart keeps its pixels while the placement scales.
                if let Some(placement) = mask.placement.as_mut() {
                    let unit_to_document = placement.unit_to_document();
                    *placement = placement.placing(&unit_to_document.concat(&scaling));
                }
            } else {
                if used_mask_pixels >= 100_000_000
                    || width as u64 * height as u64 > 100_000_000 - used_mask_pixels
                {
                    return None;
                }
                used_mask_pixels += width as u64 * height as u64;
                // The mask pixels live on the layer's *old* pixel grid; map
                // from that, not from the new (scaled) box — building the
                // source transform from the new transform would scale twice.
                let source_transform = LayerTransform {
                    sampling,
                    ..layer.transform
                };
                let mapping = scaling
                    .concat(&crate::raster::pixel_to_document(
                        &source_transform,
                        mask.asset.image.width as f64,
                        mask.asset.image.height as f64,
                    ))
                    .translate(-left, -top);
                let mut out = Bitmap::new(width, height);
                crate::raster::draw_affine(
                    &mut out,
                    mask.asset.image.as_ref(),
                    &mapping,
                    sampling,
                    None,
                );
                let thumbnail = crate::raster::thumbnail_of(&out);
                mask.asset = ImportedImage::with_thumbnail(out, thumbnail, mask.asset.name.clone());
            }
        }
        layer.transform = transform;
    }
    Some(result)
}

/// `CropGeometry`.
pub struct CropGeometry;

impl CropGeometry {
    pub fn snapped(rect: CGRect) -> CGRect {
        let rect = standardize(rect);
        let x = rect.min_x().round();
        let y = rect.min_y().round();
        CGRect::new(
            x,
            y,
            (rect.max_x().round() - x).max(1.0),
            (rect.max_y().round() - y).max(1.0),
        )
    }

    pub fn valid(rect: CGRect) -> bool {
        [rect.min_x(), rect.min_y(), rect.width(), rect.height()]
            .iter()
            .all(|v| v.is_finite())
            && (1.0..=30_000.0).contains(&rect.width())
            && (1.0..=30_000.0).contains(&rect.height())
            && rect.min_x().abs() <= 1_000_000.0
            && rect.min_y().abs() <= 1_000_000.0
    }

    /// A frame dragged from `start` to `end` — or, `symmetric` (Option), grown
    /// out from `start` as its center.
    pub fn create(from: CGPoint, to: CGPoint, ratio: Option<f64>, symmetric: bool) -> CGRect {
        let mut dx = to.x - from.x;
        let mut dy = to.y - from.y;
        if let Some(ratio) = ratio {
            if dx.abs() > dy.abs() * ratio {
                dy = if dy < 0.0 { -1.0 } else { 1.0 } * dx.abs() / ratio;
            } else {
                dx = if dx < 0.0 { -1.0 } else { 1.0 } * dy.abs() * ratio;
            }
        }
        if symmetric {
            return Self::snapped(CGRect::new(
                from.x - dx.abs(),
                from.y - dy.abs(),
                dx.abs() * 2.0,
                dy.abs() * 2.0,
            ));
        }
        Self::snapped(CGRect::new(
            from.x.min(from.x + dx),
            from.y.min(from.y + dy),
            dx.abs(),
            dy.abs(),
        ))
    }
}

fn standardize(rect: CGRect) -> CGRect {
    let x = rect.size.width.min(0.0);
    let y = rect.size.height.min(0.0);
    CGRect::new(
        if x < 0.0 { rect.max_x() } else { rect.min_x() },
        if y < 0.0 { rect.max_y() } else { rect.min_y() },
        rect.size.width.abs(),
        rect.size.height.abs(),
    )
}

/// `CropDrag`.
pub struct CropDrag {
    pub start: CGPoint,
    pub original: CGRect,
    pub mode: CropDragMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropDragMode {
    Create,
    Move,
    Resize(usize),
}

impl CropDrag {
    pub fn updated(&self, point: CGPoint, ratio: Option<f64>, symmetric: bool) -> CGRect {
        match self.mode {
            CropDragMode::Create => CropGeometry::create(self.start, point, ratio, symmetric),
            CropDragMode::Move => CropGeometry::snapped(
                self.original
                    .offset_by(point.x - self.start.x, point.y - self.start.y),
            ),
            CropDragMode::Resize(index) => {
                let transform = LayerTransform::new(self.original.origin, self.original.size);
                let drag = crate::transform::TransformDrag {
                    original: transform,
                    start: self.start,
                    mode: crate::transform::DragMode::Resize(index),
                    original_corners: None,
                };
                let next = drag.updated(point, ratio.is_some(), false, symmetric);
                CropGeometry::snapped(CGRect::from_origin_size(next.origin, next.size))
            }
        }
    }
}

/// Crop edges snap to nearby layer and canvas edges while dragging
/// (`CropSnap`).
pub struct CropSnap {
    /// Document x and y positions to snap to.
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    /// How close, in document pixels, an edge must come to snap.
    pub tolerance: f64,
}

impl CropSnap {
    fn nearest(&self, value: f64, targets: &[f64]) -> Option<f64> {
        let mut best: Option<f64> = None;
        for target in targets {
            let distance = (target - value).abs();
            if distance > self.tolerance {
                continue;
            }
            if let Some(current) = best
                && (current - value).abs() <= distance
            {
                continue;
            }
            best = Some(*target);
        }
        best
    }

    /// Moving the frame snaps its closest edges and keeps its size; creating
    /// or resizing snaps only the edges on the side being dragged — mirrored
    /// about the center when `symmetric`. With a fixed ratio only moves snap,
    /// so the ratio stays exact.
    pub fn apply(
        &self,
        rect: CGRect,
        drag: &CropDrag,
        point: CGPoint,
        ratio: Option<f64>,
        symmetric: bool,
    ) -> CGRect {
        if self.tolerance <= 0.0 {
            return rect;
        }
        if matches!(drag.mode, CropDragMode::Move) {
            let shift = |edges: [f64; 2], targets: &[f64]| -> f64 {
                let mut best: Option<f64> = None;
                for edge in edges {
                    // The offset from the edge to its nearest snap target.
                    if let Some(delta) = self.nearest(edge, targets).map(|target| target - edge) {
                        best = Some(match best {
                            Some(current) if current.abs() < delta.abs() => current,
                            _ => delta,
                        });
                    }
                }
                best.unwrap_or(0.0)
            };
            let dx = shift([rect.min_x(), rect.max_x()], &self.xs);
            let dy = shift([rect.min_y(), rect.max_y()], &self.ys);
            return rect.offset_by(dx, dy);
        }
        let (horizontal, vertical) = match drag.mode {
            CropDragMode::Create => {
                if ratio.is_some() {
                    return rect;
                }
                (true, true)
            }
            CropDragMode::Resize(index) => {
                if ratio.is_some() {
                    return rect;
                }
                let handle = crate::transform::HANDLES[index];
                (handle.x != 0.5, handle.y != 0.5)
            }
            _ => (false, false),
        };
        let mut result = rect;
        // The dragged edge is the one on the pointer's side.
        if horizontal {
            if (point.x - result.min_x()).abs() <= (point.x - result.max_x()).abs() {
                if let Some(x) = self.nearest(result.min_x(), &self.xs)
                    && x < result.max_x()
                {
                    result = CGRect::new(x, result.min_y(), result.max_x() - x, result.height());
                }
            } else if let Some(x) = self.nearest(result.max_x(), &self.xs)
                && x > result.min_x()
            {
                result = CGRect::new(
                    result.min_x(),
                    result.min_y(),
                    x - result.min_x(),
                    result.height(),
                );
            }
        }
        if vertical {
            if (point.y - result.min_y()).abs() <= (point.y - result.max_y()).abs() {
                if let Some(y) = self.nearest(result.min_y(), &self.ys)
                    && y < result.max_y()
                {
                    result = CGRect::new(result.min_x(), y, result.width(), result.max_y() - y);
                }
            } else if let Some(y) = self.nearest(result.max_y(), &self.ys)
                && y > result.min_y()
            {
                result = CGRect::new(
                    result.min_x(),
                    result.min_y(),
                    result.width(),
                    y - result.min_y(),
                );
            }
        }
        if symmetric {
            // The snapped (dragged) edge sets the half size; the opposite edge
            // mirrors it about the center.
            let mut center = CGPoint::new(drag.original.mid_x(), drag.original.mid_y());
            if matches!(drag.mode, CropDragMode::Create) {
                center = drag.start;
            }
            if horizontal {
                let half = if point.x >= center.x {
                    result.max_x() - center.x
                } else {
                    center.x - result.min_x()
                };
                if half >= 0.5 {
                    result =
                        CGRect::new(center.x - half, result.min_y(), half * 2.0, result.height());
                }
            }
            if vertical {
                let half = if point.y >= center.y {
                    result.max_y() - center.y
                } else {
                    center.y - result.min_y()
                };
                if half >= 0.5 {
                    result =
                        CGRect::new(result.min_x(), center.y - half, result.width(), half * 2.0);
                }
            }
        }
        result
    }
}

/// `TrimBasedOn` / `TrimOptions` (`ImageTrim.swift`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TrimBasedOn {
    #[default]
    #[serde(rename = "Transparent Pixels")]
    TransparentPixels,
    #[serde(rename = "Top Left Pixel Color")]
    TopLeftPixelColor,
    #[serde(rename = "Bottom Right Pixel Color")]
    BottomRightPixelColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrimOptions {
    pub based_on: TrimBasedOn,
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
    pub tolerance: u8,
}

impl Default for TrimOptions {
    fn default() -> Self {
        TrimOptions {
            based_on: TrimBasedOn::TransparentPixels,
            top: true,
            bottom: true,
            left: true,
            right: true,
            tolerance: 0,
        }
    }
}

impl TrimOptions {
    pub fn trims_any(&self) -> bool {
        self.top || self.bottom || self.left || self.right
    }
}

/// `ImageTrim.calculateTrimRect`: the crop rectangle that trims `image`;
/// None when nothing remains.
pub fn calculate_trim_rect(image: &Bitmap, options: &TrimOptions) -> Option<CGRect> {
    if !options.trims_any() || image.width == 0 || image.height == 0 {
        return None;
    }
    let width = image.width;
    let height = image.height;
    match options.based_on {
        TrimBasedOn::TransparentPixels => {
            let bounds = image.alpha_bounds()?;
            let min_x = if options.left {
                bounds.min_x() as usize
            } else {
                0
            };
            let min_y = if options.top {
                bounds.min_y() as usize
            } else {
                0
            };
            let max_x = if options.right {
                bounds.max_x() as usize
            } else {
                width
            };
            let max_y = if options.bottom {
                bounds.max_y() as usize
            } else {
                height
            };
            if max_x <= min_x || max_y <= min_y {
                return None;
            }
            Some(CGRect::new(
                min_x as f64,
                min_y as f64,
                (max_x - min_x) as f64,
                (max_y - min_y) as f64,
            ))
        }
        TrimBasedOn::TopLeftPixelColor => calculate_color_trim_rect(image, (0, 0), options),
        TrimBasedOn::BottomRightPixelColor => {
            calculate_color_trim_rect(image, (width - 1, height - 1), options)
        }
    }
}

fn calculate_color_trim_rect(
    image: &Bitmap,
    sample: (usize, usize),
    options: &TrimOptions,
) -> Option<CGRect> {
    let target = image.pixel(sample.0, sample.1);
    let tol = options.tolerance as i32;
    let matches = |x: usize, y: usize| -> bool {
        let p = image.pixel(x, y);
        (p[0] as i32 - target[0] as i32).abs() <= tol
            && (p[1] as i32 - target[1] as i32).abs() <= tol
            && (p[2] as i32 - target[2] as i32).abs() <= tol
            && (p[3] as i32 - target[3] as i32).abs() <= tol
    };
    let (width, height) = (image.width, image.height);
    let mut left = width;
    let mut right = 0usize;
    let mut top = height;
    let mut bottom = 0usize;
    for y in 0..height {
        let mut first = 0usize;
        while first < width && matches(first, y) {
            first += 1;
        }
        if first == width {
            continue;
        }
        let mut last = width;
        while last > first && matches(last - 1, y) {
            last -= 1;
        }
        if first < left {
            left = first;
        }
        if last > right {
            right = last;
        }
        if y < top {
            top = y;
        }
        bottom = y + 1;
    }
    if right == 0 {
        return None;
    }
    let min_x = if options.left { left } else { 0 };
    let min_y = if options.top { top } else { 0 };
    let max_x = if options.right { right } else { width };
    let max_y = if options.bottom { bottom } else { height };
    if max_x <= min_x || max_y <= min_y {
        return None;
    }
    Some(CGRect::new(
        min_x as f64,
        min_y as f64,
        (max_x - min_x) as f64,
        (max_y - min_y) as f64,
    ))
}

/// `ImageTrim.trim`: trims the canvas via the crop rect (None when nothing remains).
pub fn trim(document: &CanvasDocument, options: &TrimOptions) -> Option<CanvasDocument> {
    let raster = crate::render::draw_live_composite(document);
    let rect = calculate_trim_rect(&raster, options)?;
    let full_w = document.width;
    let full_h = document.height;
    if rect.width() as usize == full_w
        && rect.height() as usize == full_h
        && rect.min_x() == 0.0
        && rect.min_y() == 0.0
    {
        return Some(document.clone());
    }
    let mut options = CanvasSizeOptions::new(rect.width() as usize, rect.height() as usize);
    options.content_offset = Some(CGPoint::new(-rect.min_x(), -rect.min_y()));
    resize_canvas(document, &options).ok()
}

/// `LayerTransform.mirrored` (`LayerFlip.swift`).
pub fn mirrored_transform(
    transform: &LayerTransform,
    horizontally: bool,
    axis: CGFloat,
) -> LayerTransform {
    let mut result = *transform;
    if horizontally {
        result.flip_x = !result.flip_x;
        result.origin.x = 2.0 * axis - transform.center().x - transform.size.width / 2.0;
    } else {
        result.flip_y = !result.flip_y;
        result.origin.y = 2.0 * axis - transform.center().y - transform.size.height / 2.0;
    }
    result.rotation = -transform.rotation;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::CanvasGuide;
    use crate::transform::LayerSampling;

    #[test]
    fn draft_units_roundtrip() {
        let mut draft = CanvasSizeDraft::new(1000, 500, 72.0);
        draft.unit = CanvasUnit::Inches;
        draft.set(5.0, true);
        assert_eq!(draft.width, 360.0);
        assert!((draft.displayed(true) - 5.0).abs() < 1e-9);
        draft.unit = CanvasUnit::Percent;
        draft.relative = true;
        // 110% relative: 550 px added to the original 500 → 1050.
        draft.set(110.0, false);
        assert!((draft.height - 1050.0).abs() < 1e-9);
        assert!((draft.displayed(false) - 110.0).abs() < 1e-9);
    }

    #[test]
    fn locked_axis_scales_other_side() {
        let mut draft = CanvasSizeDraft::new(1000, 500, 72.0);
        draft.locked = true;
        draft.set(2000.0, true);
        assert_eq!(draft.height, 1000.0);
    }

    #[test]
    fn anchor_offsets() {
        let options = CanvasSizeOptions::new(1100, 600);
        // Center anchor (4): floor(100*1/2), floor(100*1/2).
        assert_eq!(options.offset(1000, 500), CGPoint::new(50.0, 50.0));
        let top_left = CanvasSizeOptions {
            anchor: 0,
            ..CanvasSizeOptions::new(1100, 600)
        };
        assert_eq!(top_left.offset(1000, 500), CGPoint::new(0.0, 0.0));
        let explicit = CanvasSizeOptions {
            content_offset: Some(CGPoint::new(-30.0, -20.0)),
            ..CanvasSizeOptions::new(1100, 600)
        };
        assert_eq!(explicit.offset(1000, 500), CGPoint::new(-30.0, -20.0));
    }

    #[test]
    fn resize_canvas_moves_layers_and_guides() {
        let mut doc = CanvasDocument::new(100, 100);
        let mut layer = ImageLayer::blank("L", doc.size());
        layer.transform.origin = CGPoint::new(10.0, 10.0);
        doc.layers = vec![layer];
        doc.guides = vec![CanvasGuide::new(crate::document::GuideAxis::Vertical, 50.0)];
        let options = CanvasSizeOptions::new(120, 120);
        let out = resize_canvas(&doc, &options).unwrap();
        assert_eq!((out.width, out.height), (120, 120));
        // Center anchor (4): offset floor(20×1/2) = 10.
        assert_eq!(out.layers[0].transform.origin, CGPoint::new(20.0, 20.0));
        assert_eq!(out.guides[0].position, 60.0);
    }

    #[test]
    fn resize_image_resamples_layers() {
        let mut doc = CanvasDocument::new(100, 100);
        let mut image = Bitmap::new(50, 50);
        image.set_pixel(0, 0, [255, 0, 0, 255]);
        let mut layer =
            ImageLayer::with_asset(ImportedImage::new(image, "L"), CGPoint::new(25.0, 25.0));
        layer.transform.size = CGSize::new(50.0, 50.0);
        doc.layers = vec![layer];
        let out = resize_image(&doc, 200, 200, Some(144.0), LayerSampling::High).unwrap();
        assert_eq!((out.width, out.height), (200, 200));
        assert_eq!(out.resolution, 144.0);
        // The layer's own pixels scale by the same factor: 50 → 100.
        assert_eq!(out.layers[0].asset.as_ref().unwrap().image.width, 100);
        assert_eq!(out.layers[0].transform.size, CGSize::new(100.0, 100.0));
        assert_eq!(out.layers[0].transform.origin, CGPoint::new(50.0, 50.0));
    }

    #[test]
    fn same_size_resize_only_sets_resolution() {
        let mut doc = CanvasDocument::new(100, 100);
        doc.resolution = 72.0;
        doc.guides = vec![CanvasGuide::new(crate::document::GuideAxis::Vertical, 50.0)];
        let out = resize_image(&doc, 100, 100, Some(300.0), LayerSampling::High).unwrap();
        assert_eq!(out.resolution, 300.0);
        assert_eq!(out.guides.len(), 1, "guides stay put");
        assert_eq!(out.layers.len(), doc.layers.len());
    }

    #[test]
    fn resize_keeps_uniform_and_placed_masks() {
        let mut doc = CanvasDocument::new(100, 100);
        let mut layer = ImageLayer::blank("L", doc.size());
        let mut mask = crate::document::LayerMask::solid(true);
        // A placed mask keeps its pixels; only the placement scales.
        mask.placement = Some(LayerTransform::new(
            CGPoint::new(10.0, 10.0),
            CGSize::new(50.0, 50.0),
        ));
        layer.mask = Some(mask);
        doc.layers = vec![layer];
        let out = resize_image(&doc, 200, 200, None, LayerSampling::High).unwrap();
        let mask = out.layers[0].mask.as_ref().unwrap();
        assert_eq!(mask.asset.image.width, 1, "uniform mask untouched");
        let placement = mask.placement.unwrap();
        assert_eq!(
            placement.size,
            CGSize::new(100.0, 100.0),
            "placement scales"
        );
    }

    #[test]
    fn resize_resamples_in_place_masks_from_the_old_grid() {
        // A non-uniform mask covering the layer lives on the layer's old
        // pixel grid; Image Size must resample it once (old grid → new grid),
        // not scale it twice (which throws the mask off the bitmap).
        let mut doc = CanvasDocument::new(10, 10);
        let mut layer = ImageLayer::blank("L", doc.size());
        let mut mask_image = Bitmap::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                mask_image.data[(y * 4 + x) * 4] = (x * 40 + y * 20) as u8;
                mask_image.data[(y * 4 + x) * 4 + 3] = 255;
            }
        }
        layer.mask = Some(crate::document::LayerMask {
            asset: ImportedImage::with_thumbnail(mask_image, Bitmap::new(2, 2), "Mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        doc.layers = vec![layer];
        let out = resize_image(&doc, 20, 20, None, LayerSampling::High).unwrap();
        let mask = out.layers[0].mask.as_ref().unwrap();
        assert_eq!((mask.asset.image.width, mask.asset.image.height), (20, 20));
        // New-grid pixel (5,5) samples the old mask around source (2.2, 2.2):
        // a single resample lands near v(2,2)=80 (Lanczos ringing aside).
        // The double-transform bug pushed the sample far off the source and
        // clamped it to the edge pixel v(3,3)=180.
        let v = mask.asset.image.pixel(5, 5)[0];
        assert!(
            (40..=140).contains(&v),
            "in-place mask resampled once, got {}",
            v
        );
    }

    #[test]
    fn resize_rejects_out_of_range_values() {
        let doc = CanvasDocument::new(100, 100);
        assert!(resize_image(&doc, 0, 100, None, LayerSampling::High).is_none());
        assert!(resize_image(&doc, 100, 30_001, None, LayerSampling::High).is_none());
        assert!(resize_image(&doc, 100, 100, Some(9601.0), LayerSampling::High).is_none());
        assert!(resize_image(&doc, 30_001, 30_001, None, LayerSampling::High).is_none());
    }

    #[test]
    fn resize_canvas_lays_a_colored_extension_below() {
        let mut doc = CanvasDocument::new(100, 100);
        let mut layer = ImageLayer::blank("L", doc.size());
        layer.transform.origin = CGPoint::new(10.0, 10.0);
        doc.layers = vec![layer];
        let mut options = CanvasSizeOptions::new(150, 150);
        options.anchor = 0;
        options.fill = Some(CanvasExtensionColor {
            red: 0.2,
            green: 0.4,
            blue: 0.8,
        });
        let out = resize_canvas(&doc, &options).unwrap();
        assert_eq!((out.width, out.height), (150, 150));
        // The extension is the bottom layer, filled where the old canvas
        // wasn't, transparent over it.
        let extension = &out.layers[0];
        assert_eq!(extension.name, "Canvas Extension");
        let image = extension.asset.as_ref().unwrap().image.as_ref();
        assert_eq!(image.pixel(140, 140)[3], 255, "new area is filled");
        assert_eq!(image.pixel(5, 5)[3], 0, "the old area stays transparent");
        // The content moved with the anchor.
        assert_eq!(out.layers[1].transform.origin, CGPoint::new(10.0, 10.0));
    }

    #[test]
    fn resize_canvas_rejects_bad_requests() {
        let doc = CanvasDocument::new(100, 100);
        let mut options = CanvasSizeOptions::new(0, 100);
        options.anchor = 0;
        assert!(resize_canvas(&doc, &options).is_err());
        let options = CanvasSizeOptions::new(30_001, 100);
        assert!(resize_canvas(&doc, &options).is_err());
        // Same size and no offset is an accepted no-op.
        let options = CanvasSizeOptions::new(100, 100);
        assert!(resize_canvas(&doc, &options).is_ok());
    }

    #[test]
    fn crop_geometry_create_and_snap() {
        let rect = CropGeometry::create(
            CGPoint::new(10.0, 10.0),
            CGPoint::new(30.5, 25.0),
            None,
            false,
        );
        assert_eq!(rect, CGRect::new(10.0, 10.0, 21.0, 15.0));
        let symmetric = CropGeometry::create(
            CGPoint::new(10.0, 10.0),
            CGPoint::new(15.0, 12.0),
            None,
            true,
        );
        assert_eq!(symmetric, CGRect::new(5.0, 8.0, 10.0, 4.0));
        let ratio = CropGeometry::create(
            CGPoint::new(0.0, 0.0),
            CGPoint::new(40.0, 10.0),
            Some(2.0),
            false,
        );
        assert!(
            (ratio.width() / ratio.height() - 2.0).abs() < 1e-9,
            "{ratio:?}"
        );
    }

    #[test]
    fn trim_transparent_pixels() {
        let mut doc = CanvasDocument::new(20, 20);
        let mut image = Bitmap::new(20, 20);
        for y in 5..15 {
            for x in 4..12 {
                let mut px = [0u8; 4];
                if y >= 5 && y < 15 && x >= 4 && x < 12 {
                    px = [255, 0, 0, 255];
                }
                image.set_pixel(x, y, px);
            }
        }
        let layer = ImageLayer::with_asset(ImportedImage::new(image, "L"), CGPoint::ZERO);
        doc.layers = vec![layer];
        let trimmed = trim(&doc, &TrimOptions::default()).unwrap();
        assert_eq!((trimmed.width, trimmed.height), (8, 10));
        assert_eq!(trimmed.layers[0].transform.origin, CGPoint::new(-4.0, -5.0));
    }

    #[test]
    fn trim_by_color_with_tolerance() {
        let mut image = Bitmap::new(10, 10);
        for y in 0..10 {
            for x in 0..10 {
                image.set_pixel(x, y, [250, 250, 250, 255]);
            }
        }
        image.set_pixel(8, 7, [0, 0, 0, 255]);
        let options = TrimOptions {
            based_on: TrimBasedOn::TopLeftPixelColor,
            tolerance: 10,
            ..TrimOptions::default()
        };
        let rect = calculate_trim_rect(&image, &options).unwrap();
        assert_eq!(rect, CGRect::new(8.0, 7.0, 1.0, 1.0));
        // With faint background variation, zero tolerance keeps it.
        let mut image2 = Bitmap::new(10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let v = 250 - ((x + y) % 3) as u8;
                image2.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        image2.set_pixel(8, 7, [0, 0, 0, 255]);
        let strict = TrimOptions {
            based_on: TrimBasedOn::TopLeftPixelColor,
            ..TrimOptions::default()
        };
        let rect2 = calculate_trim_rect(&image2, &strict).unwrap();
        assert!(rect2.width() > 5.0);
    }

    #[test]
    fn mirrored_flips_about_axis() {
        let t = LayerTransform::new(CGPoint::new(10.0, 0.0), CGSize::new(20.0, 20.0));
        let m = mirrored_transform(&t, true, 50.0);
        assert!(m.flip_x);
        assert!(!m.flip_y);
        assert_eq!(m.origin.x, 2.0 * 50.0 - 20.0 - 10.0);
        assert_eq!(m.rotation, 0.0);
        let mut rotated = t;
        rotated.rotation = 30.0;
        assert_eq!(mirrored_transform(&rotated, true, 50.0).rotation, -30.0);
    }
}
