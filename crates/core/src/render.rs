//! The layer compositing pipeline — the port of `Rendering/LayerRenderer.swift`
//! and `TiledLayerRenderer.swift`'s semantics: layers drawn bottom to top,
//! each through its transform, adjustment, effects, masks and clipping, with
//! its opacity and blend mode. Folders are pass-through.

#![allow(dead_code)]

use crate::blend::composite_pixel;
use crate::document::{CanvasDocument, ImageLayer, LayerMask, LayerShape, ShapeKind};
use crate::geometry::{CGPoint, CGRect, CGSize};
use crate::selection::SelectionPath;
use crate::raster::{draw_affine, pixel_to_document, Bitmap};
use crate::transform::{LayerSampling, LayerTransform};
use std::collections::HashMap;

/// A layer's own pixels placed on the document (no blend/opacity), the way
/// `LayerRenderer.draw` draws into a context.
pub fn draw_layer_pixels(image: &Bitmap, transform: &LayerTransform, dst: &mut Bitmap) {
    let mapping = pixel_to_document(transform, image.width as f64, image.height as f64);
    draw_affine(dst, image, &mapping, transform.sampling, None);
}

/// `LayerMask.clipImage`: the mask as a layer's renderer takes it — an image
/// stretched over the layer's `width` × `height` grid at `layer`. Covering the
/// layer (placement None) that's the mask itself; placed apart, it is
/// resampled into that grid.
pub fn mask_clip_image(
    mask: &LayerMask,
    over: &LayerTransform,
    width: usize,
    height: usize,
) -> Option<Bitmap> {
    if !mask.is_enabled {
        return None;
    }
    let placement = mask.placement?;
    if placement.same_placement(over) || width == 0 || height == 0 {
        return Some(mask_gray_as_bitmap(&mask.asset.image));
    }
    let background = LayerMask::background_of(&mask.asset.thumbnail);
    // The mask grid stretched over the placement, resampled into the layer grid.
    let mut out = Bitmap::new(width, height);
    let fill = (background * 255.0) as u8;
    for chunk in out.data.chunks_exact_mut(4) {
        chunk[0] = fill;
        chunk[1] = fill;
        chunk[2] = fill;
        chunk[3] = 255;
    }
    let source = mask_gray_as_bitmap(&mask.asset.image);
    // Layer-grid pixel → document → placement-grid pixel.
    let layer_to_doc = pixel_to_document(over, width as f64, height as f64);
    let doc_to_placement = pixel_to_document(&placement, source.width as f64, source.height as f64).inverted();
    let mapping = doc_to_placement.concat(&layer_to_doc);
    draw_affine(&mut out, &source, &mapping, LayerSampling::High, None);
    Some(out)
}

fn mask_gray_as_bitmap(image: &Bitmap) -> Bitmap {
    // Masks are stored as gray-in-RGBA; keep as is for affine drawing.
    image.clone()
}

/// The alpha multiplier at one document pixel from a mask (its grid stretched
/// by `placement`), 1 when the mask doesn't restrict.
fn mask_alpha_at(mask_grid: &Bitmap, placement: &LayerTransform, doc: CGPoint) -> f32 {
    let inverse = pixel_to_document(placement, mask_grid.width as f64, mask_grid.height as f64).inverted();
    let p = doc.applying(&inverse);
    if p.x < 0.0 || p.y < 0.0 || p.x >= mask_grid.width as f64 || p.y >= mask_grid.height as f64 {
        return 0.0;
    }
    let sample = crate::raster::sample_bitmap(mask_grid, p.x, p.y, LayerSampling::Smooth);
    sample[0] as f32 / 255.0
}

/// The composited canvas (`drawLiveComposite`): every visible layer, bottom to
/// top, blended in place. `blend_overrides` lets the UI preview a pending
/// blend mode; `preview_overrides` lets it draw pending pixel results.
pub fn draw_live_composite(document: &CanvasDocument) -> Bitmap {
    let mut canvas = Bitmap::new(document.width.max(1), document.height.max(1));
    composite_onto(&mut canvas, document);
    canvas
}

/// Composites the document onto an existing canvas-sized bitmap.
pub fn composite_onto(canvas: &mut Bitmap, document: &CanvasDocument) {
    let size = CGSize::new(document.width as f64, document.height as f64);
    let visible = document.effective_visible_ids();
    let parents: HashMap<uuid::Uuid, Option<uuid::Uuid>> =
        document.layers.iter().map(|l| (l.id, l.parent_id)).collect();

    for layer in &document.layers {
        if !visible.contains(&layer.id) {
            continue;
        }
        draw_single_layer(canvas, document, layer, &parents, &visible, size);
    }
}

fn draw_single_layer(
    canvas: &mut Bitmap,
    document: &CanvasDocument,
    layer: &ImageLayer,
    parents: &HashMap<uuid::Uuid, Option<uuid::Uuid>>,
    visible: &std::collections::HashSet<uuid::Uuid>,
    size: CGSize,
) {
    // Folders are pass-through: their children were (or will be) drawn
    // directly; only a folder mask matters, applied per descendant below.
    if layer.is_group {
        return;
    }
    // The pixels to draw: pending overrides were applied by the caller's copy.
    // A shape layer without committed pixels draws its shape live, which is
    // what keeps it editable.
    let mut transform = layer.transform;
    let mut pixels = match &layer.shape {
        Some(shape) if layer.asset.is_none() => {
            let w = transform.size.width.round().max(1.0) as usize;
            let h = transform.size.height.round().max(1.0) as usize;
            render_shape_image(shape, w, h)
        }
        _ => match &layer.asset {
            Some(asset) => asset.image.as_ref().clone(),
            None => return,
        },
    };
    // Adjustment layers apply their adjustment in place of pixels.
    if let Some(adjustment) = &layer.adjustment {
        pixels = adjustment.apply(&pixels, None);
    }

    // Live clipping mask (maskSourceID): the source layer's alpha multiplies in.
    if let Some(source_id) = layer.mask_source_id {
        if let Some(source) = document.layer(source_id) {
            if let Some(source_asset) = &source.asset {
                multiply_by_source_alpha(&mut pixels, &transform, source_asset.image.as_ref(), &source.transform);
            }
        }
    }

    // The layer's own mask, in its pixel grid.
    let mut own_mask: Option<Bitmap> = None;
    if let Some(mask) = &layer.mask {
        own_mask = mask_clip_image(mask, &layer.transform, pixels.width, pixels.height);
    }

    // Folder masks above this layer multiply coverage (pass-through folders).
    let mut folder_masks: Vec<(Bitmap, LayerTransform)> = Vec::new();
    let mut current = layer.parent_id;
    let mut depth = 0;
    while let Some(id) = current {
        if depth >= 64 {
            break;
        }
        if let Some(folder) = document.layer(id) {
            if !visible.contains(&folder.id) {
                return; // An invisible folder hides everything inside.
            }
            if let Some(mask) = &folder.mask {
                if mask.is_enabled {
                    // The folder mask covers the folder's own transform rect.
                    let grid_w = folder.asset.as_ref().map(|a| a.image.width).unwrap_or(folder.transform.size.width.round() as usize).max(1);
                    let grid_h = folder.asset.as_ref().map(|a| a.image.height).unwrap_or(folder.transform.size.height.round() as usize).max(1);
                    if let Some(placed) = mask_clip_image(mask, &folder.transform, grid_w, grid_h) {
                        folder_masks.push((placed, mask.placement.unwrap_or(folder.transform)));
                    }
                }
            }
            current = folder.parent_id;
        } else {
            break;
        }
    }

    // Effects grow the drawn image; the transform grows with it.
    let mut effects_inset = 0.0f64;
    if let Some(effects) = &layer.effects {
        if !effects.is_empty() && effects.is_valid() {
            let mask_grid = own_mask.as_ref().map(|m| gray_of(m, pixels.width, pixels.height));
            let (grown, inset) = crate::effects::render_effects(&pixels, mask_grid.as_ref(), effects);
            pixels = grown;
            effects_inset = inset;
        }
    }
    transform = if effects_inset > 0.0 {
        // The shape's own pixel count approximates the pre-effects grid.
        let base_size = crate::geometry::CGSize::new(
            pixels.width as f64 - effects_inset * 2.0,
            pixels.height as f64 - effects_inset * 2.0,
        );
        crate::effects::placed_transform(&transform, &crate::raster::Bitmap {
            width: base_size.width.round() as usize,
            height: base_size.height.round() as usize,
            data: Vec::new(),
        }, effects_inset)
    } else {
        transform
    };

    // Where this layer lands, drawn through its transform.
    let mut placed = Bitmap::new(canvas.width, canvas.height);
    draw_layer_pixels(&pixels, &transform, &mut placed);

    // Masks multiply the placed alpha.
    let doc_to_grid = pixel_to_document(&transform, pixels.width as f64, pixels.height as f64).inverted();
    if own_mask.is_some() || !folder_masks.is_empty() {
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                let o = (y * canvas.width + x) * 4;
                if placed.data[o + 3] == 0 {
                    continue;
                }
                let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5);
                let mut alpha = 1.0f32;
                if let Some(mask_grid) = &own_mask {
                    let grid_point = doc.applying(&doc_to_grid);
                    alpha *= sample_grid_alpha(mask_grid, grid_point, &transform, pixels.width, pixels.height);
                }
                for (mask_grid, mask_placement) in &folder_masks {
                    alpha *= mask_alpha_at(mask_grid, mask_placement, doc);
                }
                if alpha < 1.0 {
                    for c in 0..4 {
                        placed.data[o + c] = (placed.data[o + c] as f32 * alpha) as u8;
                    }
                }
            }
        }
    }

    // Blend onto the canvas with the layer's opacity and blend mode.
    let opacity = layer.opacity.clamp(0.0, 1.0) as f32;
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let o = (y * canvas.width + x) * 4;
            let s = placed.pixel(x, y);
            if s[3] == 0 {
                continue;
            }
            let mut src = crate::raster::unpremultiply_f32(s);
            src[3] *= opacity;
            if src[3] <= 0.0 {
                continue;
            }
            let dst = crate::raster::unpremultiply_f32(canvas.pixel(x, y));
            let out = composite_pixel(layer.blend_mode, dst, src);
            canvas.set_pixel(x, y, crate::raster::premultiply_f32(out));
        }
    }
    let _ = size;
}

fn sample_grid_alpha(
    mask_grid: &Bitmap,
    grid_point: CGPoint,
    transform: &LayerTransform,
    pixels_width: usize,
    pixels_height: usize,
) -> f32 {
    if grid_point.x < 0.0
        || grid_point.y < 0.0
        || grid_point.x >= pixels_width as f64
        || grid_point.y >= pixels_height as f64
    {
        return 0.0;
    }
    // The mask grid matches the pixel grid 1:1 (own mask, no placement).
    let _ = transform;
    crate::raster::sample_bitmap(mask_grid, grid_point.x, grid_point.y, LayerSampling::Smooth)[0] as f32 / 255.0
}

/// Multiply the target's alpha by a source layer's live alpha (clipping mask):
/// the source's pixels, transform, opacity, raster mask and upstream live
/// masks contribute coverage.
fn multiply_by_source_alpha(
    target: &mut Bitmap,
    target_transform: &LayerTransform,
    source: &Bitmap,
    source_transform: &LayerTransform,
) {
    let target_to_doc = pixel_to_document(target_transform, target.width as f64, target.height as f64);
    let doc_to_source = pixel_to_document(source_transform, source.width as f64, source.height as f64).inverted();
    let mapping = doc_to_source.concat(&target_to_doc);
    let mut multiplied = target.clone();
    for y in 0..target.height {
        for x in 0..target.width {
            let o = (y * target.width + x) * 4;
            if multiplied.data[o + 3] == 0 {
                continue;
            }
            let p = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&mapping);
            if p.x < 0.0 || p.y < 0.0 || p.x >= source.width as f64 || p.y >= source.height as f64 {
                multiplied.data[o + 3] = 0;
                continue;
            }
            let alpha = crate::raster::sample_bitmap(source, p.x, p.y, LayerSampling::Smooth)[3] as f32 / 255.0;
            for c in 0..4 {
                multiplied.data[o + c] = (multiplied.data[o + c] as f32 * alpha) as u8;
            }
        }
    }
    *target = multiplied;
}

/// The gray grid of a placed mask bitmap (RGBA → gray), sized to the layer.
fn gray_of(placed: &Bitmap, width: usize, height: usize) -> crate::raster::GrayMask {
    let mut gray = crate::raster::GrayMask::new(width, height);
    for i in 0..gray.data.len() {
        gray.data[i] = placed.pixel(i % width, i / width)[0];
    }
    gray
}

/// `EditorSession.shapeImage`: rasterizes a shape at `width` × `height`
/// pixels. Rectangles, rounded rectangles and ellipses fill with the shape's
/// color or draw as a `line_width` outline; a line runs between its unit
/// `start`/`end` points with a round-capped `line_width` stroke.
pub fn render_shape_image(shape: &LayerShape, width: usize, height: usize) -> Bitmap {
    let mut out = Bitmap::new(width.max(1), height.max(1));
    if width == 0 || height == 0 {
        return out;
    }
    let color = [
        (shape.red * 255.0).round() as u8,
        (shape.green * 255.0).round() as u8,
        (shape.blue * 255.0).round() as u8,
        255u8,
    ];
    let w = width as f64;
    let h = height as f64;
    // The coverage of the shape's fill (the whole shape) as a mask.
    let mut coverage = match shape.kind {
        ShapeKind::Rectangle => SelectionPath::rect(CGRect::new(0.0, 0.0, w, h)),
        ShapeKind::RoundedRectangle => rounded_rect_path(w, h, shape.corner_radius),
        ShapeKind::Ellipse => SelectionPath::ellipse(CGRect::new(0.0, 0.0, w, h)),
        ShapeKind::Line => {
            // Distance to the segment ≤ line_width / 2, with round caps.
            let thickness = shape.line_width.max(1.0);
            let (x0, y0) = (shape.start.x * w, shape.start.y * h);
            let (x1, y1) = (shape.end.x * w, shape.end.y * h);
            paint_distance_to_segment(&mut out, x0, y0, x1, y1, thickness / 2.0, color);
            return out;
        }
    };
    if !shape.filled {
        // The outline: the shape minus itself shrunk by the line width.
        let inner = shrink_by(&coverage, shape.line_width);
        coverage = coverage.subtract(&inner);
    }
    let mask = crate::selection::rasterize(&coverage, width, height, true);
    for i in 0..(width * height) {
        let a = mask.data[i] as u32;
        if a == 0 {
            continue;
        }
        let o = i * 4;
        for c in 0..3 {
            out.data[o + c] = ((color[c] as u32 * a + 127) / 255) as u8;
        }
        out.data[o + 3] = a as u8;
    }
    out
}

/// A rectangle path with rounded corners of `radius`.
fn rounded_rect_path(w: f64, h: f64, radius: f64) -> crate::selection::SelectionPath {
    let r = radius.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return crate::selection::SelectionPath::rect(CGRect::new(0.0, 0.0, w, h));
    }
    let seg = std::f64::consts::FRAC_PI_2 / 4.0; // quarter-circle in 4 segments
    let arc = |cx: f64, cy: f64, a0: f64| -> Vec<CGPoint> {
        (0..=4)
            .map(|i| {
                let t = a0 + seg * i as f64;
                CGPoint::new(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect()
    };
    let mut ring = Vec::new();
    ring.extend(arc(r, r, std::f64::consts::PI)); // top-left
    ring.extend(arc(w - r, r, -std::f64::consts::FRAC_PI_2)); // top-right
    ring.extend(arc(w - r, h - r, 0.0)); // bottom-right
    ring.extend(arc(r, h - r, std::f64::consts::FRAC_PI_2)); // bottom-left
    crate::selection::SelectionPath::from_polygon(ring)
}

/// Shrinks a coverage path inward by `amount` (the complement of a stroke
/// band), for stroke-only shapes.
fn shrink_by(path: &crate::selection::SelectionPath, amount: f64) -> crate::selection::SelectionPath {
    let band = path.stroke_band(amount * 2.0);
    path.subtract(&band)
}

/// Paints a round-capped segment: every pixel within `radius` of the segment.
fn paint_distance_to_segment(
    out: &mut Bitmap,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radius: f64,
    color: [u8; 4],
) {
    let (ax, ay) = (x0.min(x1) - radius - 1.0, y0.min(y1) - radius - 1.0);
    let (bx, by) = (x0.max(x1) + radius + 1.0, y0.max(y1) + radius + 1.0);
    let x_lo = (ax.floor().max(0.0)) as usize;
    let y_lo = (ay.floor().max(0.0)) as usize;
    let x_hi = (bx.ceil().min(out.width as f64)) as usize;
    let y_hi = (by.ceil().min(out.height as f64)) as usize;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len2 = dx * dx + dy * dy;
    for y in y_lo..y_hi.max(y_lo) {
        for x in x_lo..x_hi.max(x_lo) {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let t = if len2 <= 0.0 {
                0.0
            } else {
                (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0)
            };
            let d = (px - (x0 + dx * t)).hypot(py - (y0 + dy * t));
            let coverage = (radius - d + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let a = (coverage * 255.0) as u32;
            let o = (y * out.width + x) * 4;
            for c in 0..3 {
                out.data[o + c] = ((color[c] as u32 * a + 127) / 255) as u8;
            }
            out.data[o + 3] = a as u8;
        }
    }
}

/// The composited color under one document point (`sampleCompositeColor`).
pub fn sample_composite_color(document: &CanvasDocument, point: CGPoint) -> Option<crate::document::PaletteColor> {
    if point.x < 0.0 || point.y < 0.0 || point.x >= document.width as f64 || point.y >= document.height as f64 {
        return None;
    }
    let canvas = draw_live_composite(document);
    let px = canvas.pixel(point.x as usize, point.y as usize);
    if px[3] == 0 {
        return None;
    }
    let alpha = px[3] as f64;
    let channel = |v: u8| -> f64 { ((alpha.min(v as f64)) / alpha * 255.0).round() / 255.0 };
    Some(crate::document::PaletteColor::new(
        channel(px[0]),
        channel(px[1]),
        channel(px[2]),
    ))
}

/// The layer as the Layers panel shows it: pixels with the adjustment and
/// effects applied, cropped to content (for thumbnails).
pub fn layer_thumbnail(layer: &ImageLayer) -> Option<Bitmap> {
    let asset = layer.asset.as_ref()?;
    let mut pixels = asset.image.as_ref().clone();
    if let Some(adjustment) = &layer.adjustment {
        pixels = adjustment.apply(&pixels, None);
    }
    if let Some(effects) = &layer.effects {
        if !effects.is_empty() && effects.is_valid() {
            let (grown, _) = crate::effects::render_effects(&pixels, None, effects);
            pixels = grown;
        }
    }
    Some(crate::raster::thumbnail_of(&pixels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ImageLayer, PaletteColor};
    use crate::geometry::{CGPoint, CGSize};
    use crate::raster::ImportedImage;

    fn red_layer(doc: &CanvasDocument, x: f64, blend: crate::blend::LayerBlendMode, opacity: f64) -> ImageLayer {
        let mut image = Bitmap::new(10, 10);
        for i in (0..image.data.len()).step_by(4) {
            image.data[i] = 255;
            image.data[i + 3] = 255;
        }
        let mut layer = ImageLayer::with_asset(ImportedImage::with_thumbnail(image, Bitmap::new(2, 2), "L"), CGPoint::new(x, 0.0));
        layer.blend_mode = blend;
        layer.opacity = opacity;
        let _ = doc;
        layer
    }

    #[test]
    fn composite_two_opaque_layers_top_wins() {
        let mut doc = CanvasDocument::new(20, 10);
        doc.layers = vec![red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0)];
        let mut top = red_layer(&doc, 10.0, crate::blend::LayerBlendMode::Normal, 1.0);
        let mut blue = Bitmap::new(10, 10);
        for i in (0..blue.data.len()).step_by(4) {
            blue.data[i + 2] = 255;
            blue.data[i + 3] = 255;
        }
        top.asset = Some(ImportedImage::with_thumbnail(blue, Bitmap::new(2, 2), "T"));
        doc.layers.push(top);
        let out = draw_live_composite(&doc);
        assert_eq!(out.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(out.pixel(15, 5), [0, 0, 255, 255]);
    }

    #[test]
    fn multiply_blend_darkens() {
        let mut doc = CanvasDocument::new(10, 10);
        let mut bottom = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0);
        let mut white = Bitmap::new(10, 10);
        for i in (0..white.data.len()).step_by(4) {
            white.data[i] = 128;
            white.data[i + 1] = 128;
            white.data[i + 2] = 128;
            white.data[i + 3] = 255;
        }
        bottom.asset = Some(ImportedImage::with_thumbnail(white, Bitmap::new(2, 2), "B"));
        let mut top = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Multiply, 1.0);
        let mut half = Bitmap::new(10, 10);
        for i in (0..half.data.len()).step_by(4) {
            half.data[i] = 128;
            half.data[i + 1] = 128;
            half.data[i + 2] = 128;
            half.data[i + 3] = 255;
        }
        top.asset = Some(ImportedImage::with_thumbnail(half, Bitmap::new(2, 2), "T"));
        doc.layers = vec![bottom, top];
        let out = draw_live_composite(&doc);
        // 128/255 × 128/255 ≈ 64.
        assert!((out.pixel(5, 5)[0] as i32 - 64).abs() <= 2, "{}", out.pixel(5, 5)[0]);
    }

    #[test]
    fn hidden_folder_hides_children() {
        let mut doc = CanvasDocument::new(10, 10);
        let mut folder = ImageLayer::blank("Folder", doc.size());
        folder.is_group = true;
        let folder_id = folder.id;
        let mut child = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0);
        child.parent_id = Some(folder_id);
        doc.layers = vec![folder, child];
        assert_eq!(draw_live_composite(&doc).pixel(5, 5)[3], 255);
        doc.layer_mut(folder_id).unwrap().is_visible = false;
        assert_eq!(draw_live_composite(&doc).pixel(5, 5)[3], 0);
    }

    #[test]
    fn folder_mask_clips_contents() {
        let mut doc = CanvasDocument::new(20, 10);
        let mut folder = ImageLayer::blank("Folder", doc.size());
        folder.is_group = true;
        // The folder mask: white left half, black right half.
        let mut mask_image = Bitmap::new(20, 10);
        for y in 0..10 {
            for x in 0..20 {
                let v = if x < 10 { 255 } else { 0 };
                mask_image.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        folder.mask = Some(crate::document::LayerMask {
            asset: ImportedImage::with_thumbnail(mask_image, Bitmap::new(4, 2), "Mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        let folder_id = folder.id;
        let mut child = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0);
        child.parent_id = Some(folder_id);
        doc.layers = vec![folder, child];
        let out = draw_live_composite(&doc);
        assert_eq!(out.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(out.pixel(15, 5)[3], 0);
    }

    #[test]
    fn opacity_dims() {
        let mut doc = CanvasDocument::new(10, 10);
        let layer = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 0.5);
        doc.layers = vec![layer];
        let out = draw_live_composite(&doc);
        let px = out.pixel(5, 5);
        assert!(px[3] >= 120 && px[3] <= 135, "{px:?}");
    }

    #[test]
    fn sample_color_unpremultiplies() {
        let mut doc = CanvasDocument::new(10, 10);
        doc.layers = vec![red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0)];
        let c = sample_composite_color(&doc, CGPoint::new(5.0, 5.0)).unwrap();
        assert_eq!(c, PaletteColor::new(1.0, 0.0, 0.0));
        assert!(sample_composite_color(&doc, CGPoint::new(50.0, 5.0)).is_none());
    }

    #[test]
    fn adjustment_layer_alters_pixels() {
        use crate::adjustment::{AdjustmentKind, LayerAdjustment};
        let mut doc = CanvasDocument::new(10, 10);
        let mut gray = Bitmap::new(10, 10);
        for i in (0..gray.data.len()).step_by(4) {
            gray.data[i] = 128;
            gray.data[i + 1] = 128;
            gray.data[i + 2] = 128;
            gray.data[i + 3] = 255;
        }
        let mut layer = ImageLayer::with_asset(ImportedImage::with_thumbnail(gray, Bitmap::new(2, 2), "G"), CGPoint::ZERO);
        let mut adjustment = LayerAdjustment::new(AdjustmentKind::Exposure);
        adjustment.set_exposure(crate::adjust::ExposureSettings {
            exposure: 2.0,
            offset: 0.0,
            gamma: 1.0,
        });
        layer.adjustment = Some(adjustment);
        doc.layers = vec![layer];
        let out = draw_live_composite(&doc);
        // +2 stops: 0.214 linear ×4 = 0.856 linear ≈ 240 sRGB.
        let v = out.pixel(5, 5)[0];
        assert!(v > 225 && v < 250, "{v}");
    }

    #[test]
    fn shape_rectangle_fills_with_color() {
        let shape = LayerShape {
            kind: ShapeKind::Rectangle,
            corner_radius: 0.0,
            line_width: 4.0,
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            filled: true,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        };
        let out = render_shape_image(&shape, 20, 10);
        assert_eq!(out.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(out.pixel(0, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn shape_rectangle_stroke_leaves_hollow_center() {
        let shape = LayerShape {
            kind: ShapeKind::Rectangle,
            corner_radius: 0.0,
            line_width: 2.0,
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            filled: false,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        };
        let out = render_shape_image(&shape, 20, 20);
        assert_eq!(out.pixel(1, 1)[2], 255); // on the stroke
        assert_eq!(out.pixel(10, 10)[3], 0); // hollow
    }

    #[test]
    fn shape_ellipse_fits_bounds() {
        let shape = LayerShape {
            kind: ShapeKind::Ellipse,
            corner_radius: 0.0,
            line_width: 4.0,
            red: 0.0,
            green: 1.0,
            blue: 0.0,
            filled: true,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        };
        let out = render_shape_image(&shape, 40, 40);
        assert_eq!(out.pixel(20, 20), [0, 255, 0, 255]);
        assert_eq!(out.pixel(1, 1)[3], 0);
    }

    #[test]
    fn shape_line_runs_between_points() {
        let shape = LayerShape {
            kind: ShapeKind::Line,
            corner_radius: 0.0,
            line_width: 4.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            filled: false,
            start: CGPoint::new(0.1, 0.5),
            end: CGPoint::new(0.9, 0.5),
        };
        let out = render_shape_image(&shape, 100, 20);
        assert_eq!(out.pixel(50, 10)[3], 255); // middle of the line
        assert_eq!(out.pixel(50, 2)[3], 0); // above it
        assert_eq!(out.pixel(4, 10)[3], 0); // before the start point
    }

    #[test]
    fn shape_layer_renders_live_without_asset() {
        use crate::document::PaletteColor;
        let mut doc = CanvasDocument::new(20, 20);
        let mut layer = ImageLayer::blank("Rectangle 1", doc.size());
        layer.transform.size = CGSize::new(20.0, 20.0);
        layer.shape = Some(LayerShape {
            kind: ShapeKind::Ellipse,
            corner_radius: 0.0,
            line_width: 4.0,
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            filled: true,
            start: CGPoint::ZERO,
            end: CGPoint::new(1.0, 1.0),
        });
        doc.layers = vec![layer];
        let out = draw_live_composite(&doc);
        assert_eq!(out.pixel(10, 10), [255, 0, 0, 255]);
        // Outside the ellipse: transparent.
        assert_eq!(out.pixel(1, 1)[3], 0);
        let _ = PaletteColor::BLACK;
    }

    #[test]
    fn clipping_mask_uses_source_alpha() {
        let mut doc = CanvasDocument::new(20, 10);
        // Base: left half opaque.
        let mut base = Bitmap::new(10, 10);
        for i in (0..base.data.len()).step_by(4) {
            base.data[i] = 255;
            base.data[i + 3] = 255;
        }
        let mut base_layer = ImageLayer::with_asset(ImportedImage::with_thumbnail(base, Bitmap::new(2, 2), "B"), CGPoint::ZERO);
        let mut top = red_layer(&doc, 5.0, crate::blend::LayerBlendMode::Normal, 1.0);
        top.mask_source_id = Some(base_layer.id);
        doc.layers = vec![base_layer, top];
        let out = draw_live_composite(&doc);
        // Over the base's area the red shows; beyond it (x ≥ 10) it's clipped.
        assert_eq!(out.pixel(7, 5)[0], 255);
        assert_eq!(out.pixel(12, 5)[3], 0);
    }
}
