//! The layer compositing pipeline — the port of `Rendering/LayerRenderer.swift`
//! and `TiledLayerRenderer.swift`'s semantics: layers drawn bottom to top,
//! each through its transform, adjustment, effects, masks and clipping, with
//! its opacity and blend mode. Folders are pass-through.

#![allow(dead_code)]

use crate::blend::composite_pixel;
use crate::document::{CanvasDocument, ImageLayer, LayerMask, LayerShape, ShapeKind};
use crate::geometry::{CGPoint, CGRect};
use crate::selection::SelectionPath;
use crate::raster::{draw_affine, pixel_to_document, Bitmap};
use crate::transform::{LayerSampling, LayerTransform};
use std::collections::HashMap;
use std::cell::RefCell;

thread_local! {
    /// `EffectsPreviewCache`: per-render effects results, keyed by layer,
    /// image identity and effect settings. Cleared when it grows past a few
    /// entries — a render pass touches few layers.
    static EFFECTS_CACHE: RefCell<Vec<(uuid::Uuid, usize, crate::effects::LayerEffects, (Bitmap, f64))>> =
        RefCell::new(Vec::new());
}

/// `LayerEffectsRenderer.cached` — renders (or reuses) the effects around a
/// layer's pixels. `image_key` identifies the pixels (the asset's Arc).
pub fn render_effects_cached(
    layer_id: uuid::Uuid,
    image: &Bitmap,
    image_key: usize,
    mask: Option<&crate::raster::GrayMask>,
    effects: &crate::effects::LayerEffects,
) -> (Bitmap, f64) {
    let hit = EFFECTS_CACHE.with(|c| {
        c.borrow()
            .iter()
            .find(|(id, key, cached, _)| *id == layer_id && *key == image_key && *cached == *effects)
            .map(|entry| entry.3.clone())
    });
    if let Some(result) = hit {
        return result;
    }
    let (grown, inset) = crate::effects::render_effects(image, mask, effects);
    EFFECTS_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() >= 8 {
            cache.clear();
        }
        cache.push((layer_id, image_key, effects.clone(), (grown.clone(), inset)));
    });
    (grown, inset)
}

pub fn clear_effects_cache() {
    EFFECTS_CACHE.with(|c| c.borrow_mut().clear());
}

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
    mask_clip_image_limited(mask, over, width, height, None)
}

/// `LayerMask.clipImage(placement:over:width:height:limit:)` — `limit` caps
/// the longest side of the resampled grid while a transform is interactive.
pub fn mask_clip_image_limited(
    mask: &LayerMask,
    over: &LayerTransform,
    width: usize,
    height: usize,
    limit: Option<usize>,
) -> Option<Bitmap> {
    if !mask.is_enabled {
        return None;
    }
    // Covering the layer (placement None) the mask is the grid itself; only a
    // mask moved apart is resampled into the layer grid.
    let placement = match mask.placement {
        Some(placement) => placement,
        None => return Some(mask_gray_as_bitmap(&mask.asset.image)),
    };
    let _ = limit;
    if placement.same_placement(over) || width == 0 || height == 0 {
        return Some(mask_gray_as_bitmap(&mask.asset.image));
    }
    let background = LayerMask::background_of(&mask.asset.thumbnail);
    // The mask grid stretched over the placement, resampled into the layer
    // grid; `limit` caps the longest side while a transform is interactive.
    let (width, height) = match limit {
        Some(cap) if width.max(height) > cap => {
            let factor = cap as f64 / width.max(height) as f64;
            (
                ((width as f64 * factor).round() as usize).max(1),
                ((height as f64 * factor).round() as usize).max(1),
            )
        }
        _ => (width, height),
    };
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
    clear_effects_cache();
    let order = document.render_layers();
    let by_id: HashMap<uuid::Uuid, &crate::document::ImageLayer> =
        document.layers.iter().map(|l| (l.id, l)).collect();
    let opacities: HashMap<uuid::Uuid, f64> = order
        .iter()
        .map(|l| (l.id, effective_opacity(l, &by_id)))
        .collect();
    let (stacks, stacked) = prepare_stacks(&order);
    let mut ctx = RenderContext {
        document,
        by_id,
        opacities,
        stacks,
        stacked,
        coverage: HashMap::new(),
        visiting: std::collections::HashSet::new(),
        canvas_size: (canvas.width, canvas.height),
    };
    let mut i = 0;
    while i < order.len() {
        let layer = order[i];
        if ctx.stacked.contains(&layer.id) {
            i += 1;
            continue;
        }
        // An adjustment layer reworks the composited content below it instead
        // of drawing pixels (`LiveMaskRenderer.drawComposite`).
        if let Some(adjustment) = &layer.adjustment {
            if layer.mask_source_id.is_none() {
                if adjustment.is_valid() {
                    apply_adjustment_layer(canvas, &mut ctx, layer);
                }
            }
            i += 1;
            continue;
        }
        if let Some(children) = ctx.stacks.get(&layer.id).cloned() {
            draw_clipping_stack(canvas, &mut ctx, layer, &children);
            i += 1 + children.len();
            continue;
        }
        let clip = layer
            .mask_source_id
            .and_then(|source_id| ctx.coverage_of(source_id).cloned());
        draw_layer_full(canvas, &ctx, layer, clip.as_ref());
        i += 1;
    }
}

/// Everything one rendering pass shares: the document, the per-layer
/// folder-multiplied opacity, clipping stacks and the coverage cache.
struct RenderContext<'a> {
    document: &'a CanvasDocument,
    by_id: HashMap<uuid::Uuid, &'a crate::document::ImageLayer>,
    opacities: HashMap<uuid::Uuid, f64>,
    stacks: HashMap<uuid::Uuid, Vec<uuid::Uuid>>,
    stacked: std::collections::HashSet<uuid::Uuid>,
    coverage: HashMap<uuid::Uuid, Option<Bitmap>>,
    visiting: std::collections::HashSet<uuid::Uuid>,
    canvas_size: (usize, usize),
}

impl<'a> RenderContext<'a> {
    fn opacity_of(&self, id: uuid::Uuid) -> f64 {
        self.opacities.get(&id).copied().unwrap_or(1.0)
    }

    /// The full alpha the layer paints (`LiveMaskRenderer.coverage`): the
    /// layer as it draws itself — pixels, effects, masks, opacity, its own
    /// clipping source — regardless of its visibility or color.
    fn coverage_of(&mut self, id: uuid::Uuid) -> Option<&Bitmap> {
        if !self.coverage.contains_key(&id) {
            let filled = self.render_coverage(id);
            self.coverage.insert(id, filled);
        }
        self.coverage.get(&id).unwrap().as_ref()
    }

    fn render_coverage(&mut self, id: uuid::Uuid) -> Option<Bitmap> {
        if self.visiting.contains(&id) || self.visiting.len() >= 256 {
            return None;
        }
        let layer = *self.by_id.get(&id)?;
        self.visiting.insert(id);
        let clip = layer
            .mask_source_id
            .and_then(|source_id| self.coverage_of(source_id).cloned());
        let mut surface = Bitmap::new(self.canvas_size.0, self.canvas_size.1);
        draw_layer_full(&mut surface, self, layer, clip.as_ref());
        self.visiting.remove(&id);
        Some(surface)
    }
}

/// A folder's opacity multiplies into everything inside it (`LayerOpacity`);
/// what's inside is drawn straight onto what is below, never as a unit.
fn effective_opacity(
    layer: &crate::document::ImageLayer,
    by_id: &HashMap<uuid::Uuid, &crate::document::ImageLayer>,
) -> f64 {
    let mut opacity = layer.opacity;
    let mut id = layer.parent_id;
    let mut depth = 0;
    while let Some(current) = id {
        if depth >= 64 {
            break;
        }
        match by_id.get(&current) {
            Some(node) => {
                opacity *= node.opacity;
                id = node.parent_id;
            }
            None => break,
        }
        depth += 1;
    }
    opacity
}

/// `LiveMaskRenderer.prepareStacks`: a base (no clipping source, no
/// adjustment) followed by consecutive layers clipped to it, all with the same
/// parent, forms a stack. The whole stack shares the base's coverage and lands
/// with the base's blend mode.
fn prepare_stacks(
    order: &[&crate::document::ImageLayer],
) -> (
    HashMap<uuid::Uuid, Vec<uuid::Uuid>>,
    std::collections::HashSet<uuid::Uuid>,
) {
    let mut stacks: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
    let mut stacked = std::collections::HashSet::new();
    for (index, base) in order.iter().enumerate() {
        if base.mask_source_id.is_some() || base.adjustment.is_some() {
            continue;
        }
        let mut children = Vec::new();
        for child in order.iter().skip(index + 1) {
            if child.mask_source_id != Some(base.id) || child.parent_id != base.parent_id {
                break;
            }
            children.push(child.id);
        }
        if !children.is_empty() {
            stacks.insert(base.id, children.clone());
            stacked.extend(children);
        }
    }
    (stacks, stacked)
}

/// Draws one layer as it draws itself (`EditorCanvas.drawOwn` +
/// `LayerRenderer.draw`): pixels through the transform, effects with the mask
/// baked in, then the own mask, folder masks and an optional live-clipping
/// coverage multiplying where it lands, and finally its blend mode at its
/// effective opacity. `clip` is a canvas-sized alpha from a clipping source.
fn draw_layer_full(
    target: &mut Bitmap,
    ctx: &RenderContext,
    layer: &crate::document::ImageLayer,
    clip: Option<&Bitmap>,
) {
    let mut transform = layer.transform;
    // A shape layer without committed pixels draws its shape live, which is
    // what keeps it editable.
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
    // The layer's own mask, in its pixel grid.
    let mut own_mask = layer
        .mask
        .as_ref()
        .and_then(|mask| mask_clip_image(mask, &layer.transform, pixels.width, pixels.height));

    // Effects grow the drawn image; the mask is baked into the effect render
    // and drawn with `mask: nil`, so it must not clip the result again.
    // Results come from the per-render cache (EffectsPreviewCache).
    if let Some(effects) = &layer.effects {
        if !effects.is_empty() && effects.is_valid() {
            let mask_grid = own_mask
                .as_ref()
                .map(|m| gray_of(m, pixels.width, pixels.height));
            let image_key = layer
                .asset
                .as_ref()
                .map(|a| std::sync::Arc::as_ptr(&a.image) as usize)
                .unwrap_or(0);
            let (grown, inset) =
                render_effects_cached(layer.id, &pixels, image_key, mask_grid.as_ref(), effects);
            pixels = grown;
            own_mask = None;
            if inset > 0.0 {
                // The grown grid lands where the layer was: the transform
                // grows by the margin around the same center.
                transform = crate::effects::placed_transform(&layer.transform, &pixels, inset);
            }
        }
    }

    // Folder masks above this layer multiply coverage (pass-through folders).
    let mut folder_masks: Vec<(Bitmap, LayerTransform)> = Vec::new();
    let mut current = layer.parent_id;
    let mut depth = 0;
    while let Some(id) = current {
        if depth >= 64 {
            break;
        }
        if let Some(folder) = ctx.document.layer(id) {
            if let Some(mask) = &folder.mask {
                if mask.is_enabled {
                    // The folder mask covers the folder's own transform rect.
                    let grid_w = folder
                        .asset
                        .as_ref()
                        .map(|a| a.image.width)
                        .unwrap_or(folder.transform.size.width.round() as usize)
                        .max(1);
                    let grid_h = folder
                        .asset
                        .as_ref()
                        .map(|a| a.image.height)
                        .unwrap_or(folder.transform.size.height.round() as usize)
                        .max(1);
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

    // Where this layer lands, drawn through its transform.
    let mut placed = Bitmap::new(target.width, target.height);
    draw_layer_pixels(&pixels, &transform, &mut placed);

    // The own mask, folder masks and a live clipping source all multiply
    // coverage where the layer lands.
    if own_mask.is_some() || !folder_masks.is_empty() || clip.is_some() {
        let doc_to_grid = pixel_to_document(&transform, pixels.width as f64, pixels.height as f64).inverted();
        for y in 0..target.height {
            for x in 0..target.width {
                let o = (y * target.width + x) * 4;
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
                if let Some(coverage) = clip {
                    let co = (y * target.width + x) * 4;
                    alpha *= coverage.data[co + 3] as f32 / 255.0;
                }
                if alpha < 1.0 {
                    for c in 0..4 {
                        placed.data[o + c] = (placed.data[o + c] as f32 * alpha) as u8;
                    }
                }
            }
        }
    }

    // Blend onto the target with the layer's effective opacity and blend mode.
    let opacity = ctx.opacity_of(layer.id);
    blend_bitmap_onto(target, &placed, layer.blend_mode, opacity as f32);
}

/// An adjustment layer (`LiveMaskRenderer.adjust`): the adjustment applies to
/// the composited content below, its blend mode blends the adjusted content
/// against the original at full coverage, its effective opacity mixes the two,
/// and its own mask scopes where any of it applies.
fn apply_adjustment_layer(
    target: &mut Bitmap,
    ctx: &RenderContext,
    layer: &crate::document::ImageLayer,
) {
    let adjustment = match &layer.adjustment {
        Some(a) => a,
        None => return,
    };
    let original = target.clone();
    let mut adjusted = adjustment.apply(&original, None);
    let mode = layer.blend_mode;
    if mode != crate::blend::LayerBlendMode::Normal {
        // Blend colors at full coverage, then restore the original alpha —
        // source-over of two translucent copies would thicken soft edges.
        for i in 0..original.data.len() / 4 {
            let o = i * 4;
            if original.data[o + 3] == 0 {
                continue;
            }
            let cb = crate::raster::unpremultiply_f32(original.pixel_at(o));
            let cs = crate::raster::unpremultiply_f32(adjusted.pixel_at(o));
            let blended = crate::blend::blend_color(mode, cb[..3].try_into().unwrap(), cs[..3].try_into().unwrap());
            let px = crate::raster::premultiply_f32([blended[0], blended[1], blended[2], cb[3]]);
            adjusted.data[o..o + 4].copy_from_slice(&px);
        }
    }
    // The adjustment's own mask scopes the effect (`adjustmentClip`).
    let scope = layer.mask.as_ref().filter(|m| m.is_enabled).map(|m| {
        let placement = m.placement.unwrap_or(layer.transform);
        (
            mask_gray_as_bitmap(&m.asset.image),
            placement,
        )
    });
    let opacity = ctx.opacity_of(layer.id) as f32;
    for y in 0..target.height {
        for x in 0..target.width {
            let o = (y * target.width + x) * 4;
            let mut src = adjusted.pixel_at(o);
            if scope.is_some() {
                let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5);
                let m = scope
                    .as_ref()
                    .map(|(grid, placement)| mask_alpha_at(grid, placement, doc))
                    .unwrap_or(1.0);
                for c in 0..4 {
                    src[c] = (src[c] as f32 * m) as u8;
                }
            }
            if opacity < 1.0 {
                // The pending adjustment shows at the layer's effective
                // opacity; below that the original content shows through.
                for c in 0..4 {
                    let mixed = original.data[o + c] as f32
                        + (src[c] as f32 - original.data[o + c] as f32) * opacity;
                    src[c] = mixed.round().clamp(0.0, 255.0) as u8;
                }
            }
            if src[3] == 0 {
                continue;
            }
            let s = crate::raster::unpremultiply_f32(src);
            let dst = crate::raster::unpremultiply_f32(target.pixel_at(o));
            let out = composite_pixel(crate::blend::LayerBlendMode::Normal, dst, s);
            target.data[o..o + 4].copy_from_slice(&crate::raster::premultiply_f32(out));
        }
    }
}

/// A clipping stack (`LiveMaskRenderer.drawComposite`): the base and its
/// consecutive clipped children draw into a shared surface, adjustment
/// children rework that surface in place, and the surface lands masked by the
/// base's coverage with the base's blend mode.
fn draw_clipping_stack(
    canvas: &mut Bitmap,
    ctx: &mut RenderContext,
    base: &crate::document::ImageLayer,
    children: &[uuid::Uuid],
) {
    let mut group = Bitmap::new(canvas.width, canvas.height);
    draw_layer_full(&mut group, ctx, base, None);
    // The stack shows only where the base had coverage.
    let coverage: Vec<u8> = group.data.iter().skip(3).step_by(4).copied().collect();
    for child_id in children {
        let child = match ctx.by_id.get(child_id) {
            Some(child) => *child,
            None => continue,
        };
        if child.adjustment.is_some() {
            if child.adjustment.as_ref().unwrap().is_valid() && child.mask_source_id.is_none() {
                apply_adjustment_layer(&mut group, ctx, child);
            }
        } else {
            draw_layer_full(&mut group, ctx, child, None);
        }
    }
    for (i, a) in coverage.iter().enumerate() {
        let o = i * 4;
        let m = *a as f32 / 255.0;
        if m < 1.0 {
            for c in 0..4 {
                group.data[o + c] = (group.data[o + c] as f32 * m) as u8;
            }
        }
    }
    // The group lands with the base's blend mode at full strength; the base's
    // effective opacity is already inside the group's coverage.
    blend_bitmap_onto(canvas, &group, base.blend_mode, 1.0);
}

/// Source-over of a premultiplied bitmap onto the canvas in `mode` at
/// `opacity` (the tail of the old draw path).
fn blend_bitmap_onto(canvas: &mut Bitmap, placed: &Bitmap, mode: crate::blend::LayerBlendMode, opacity: f32) {
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let o = (y * canvas.width + x) * 4;
            let s = placed.pixel_at(o);
            if s[3] == 0 {
                continue;
            }
            let mut src = crate::raster::unpremultiply_f32(s);
            src[3] *= opacity;
            if src[3] <= 0.0 {
                continue;
            }
            let dst = crate::raster::unpremultiply_f32(canvas.pixel_at(o));
            let out = composite_pixel(mode, dst, src);
            canvas.data[o..o + 4].copy_from_slice(&crate::raster::premultiply_f32(out));
        }
    }
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
        // The child spans the whole canvas so the folder mask is what clips it.
        let mut child = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0);
        child.transform.size = CGSize::new(20.0, 10.0);
        let mut wide = Bitmap::new(20, 10);
        for i in (0..wide.data.len()).step_by(4) {
            wide.data[i] = 255;
            wide.data[i + 3] = 255;
        }
        child.asset = Some(ImportedImage::with_thumbnail(wide, Bitmap::new(4, 2), "W"));
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
    fn adjustment_layer_applies_to_content_below() {
        // An adjustment layer reworks the composited content under it
        // (LiveMaskRenderer.adjust); it draws no pixels of its own.
        use crate::adjustment::{AdjustmentKind, LayerAdjustment};
        let mut doc = CanvasDocument::new(10, 10);
        let mut gray = Bitmap::new(10, 10);
        for i in (0..gray.data.len()).step_by(4) {
            gray.data[i] = 128;
            gray.data[i + 1] = 128;
            gray.data[i + 2] = 128;
            gray.data[i + 3] = 255;
        }
        let mut adjustment = LayerAdjustment::new(AdjustmentKind::Exposure);
        adjustment.set_exposure(crate::adjust::ExposureSettings {
            exposure: 2.0,
            offset: 0.0,
            gamma: 1.0,
        });
        let mut bottom = ImageLayer::with_asset(ImportedImage::with_thumbnail(gray, Bitmap::new(2, 2), "G"), CGPoint::ZERO);
        bottom.name = "G".into();
        let mut top = ImageLayer::blank("Exposure", doc.size());
        top.adjustment = Some(adjustment.clone());
        doc.layers = vec![bottom, top];
        let out = draw_live_composite(&doc);
        // +2 stops: 0.214 linear ×4 = 0.856 linear ≈ 240 sRGB.
        let v = out.pixel(5, 5)[0];
        assert!(v > 225 && v < 250, "{v}");
        assert_eq!(out.pixel(5, 5)[3], 255);
        // With nothing below, an adjustment layer leaves the canvas empty.
        let mut empty = CanvasDocument::new(10, 10);
        let mut alone = ImageLayer::blank("Exposure", empty.size());
        alone.adjustment = Some(adjustment);
        empty.layers = vec![alone];
        assert_eq!(draw_live_composite(&empty).pixel(5, 5)[3], 0);
    }

    #[test]
    fn adjustment_mask_scopes_the_adjustment() {
        use crate::adjustment::{AdjustmentKind, LayerAdjustment};
        let mut doc = CanvasDocument::new(20, 10);
        let mut gray = Bitmap::new(20, 10);
        for i in (0..gray.data.len()).step_by(4) {
            gray.data[i] = 128;
            gray.data[i + 1] = 128;
            gray.data[i + 2] = 128;
            gray.data[i + 3] = 255;
        }
        let mut adjustment = LayerAdjustment::new(AdjustmentKind::Exposure);
        adjustment.set_exposure(crate::adjust::ExposureSettings {
            exposure: 2.0,
            offset: 0.0,
            gamma: 1.0,
        });
        let mut layer = ImageLayer::blank("Exposure", doc.size());
        layer.adjustment = Some(adjustment);
        // White left half, black right half: only the left half brightens.
        let mut mask_image = Bitmap::new(20, 10);
        for y in 0..10 {
            for x in 0..20 {
                let v = if x < 10 { 255 } else { 0 };
                mask_image.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        layer.mask = Some(crate::document::LayerMask {
            asset: ImportedImage::with_thumbnail(mask_image, Bitmap::new(4, 2), "Mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        doc.layers = vec![
            ImageLayer::with_asset(ImportedImage::with_thumbnail(gray, Bitmap::new(4, 2), "G"), CGPoint::ZERO),
            layer,
        ];
        let out = draw_live_composite(&doc);
        let lit = out.pixel(5, 5)[0];
        assert!(lit > 225, "{lit}");
        assert_eq!(out.pixel(15, 5)[0], 128);
    }

    #[test]
    fn folder_opacity_multiplies_into_children() {
        // A layer at 50% in a folder at 50% shows at 25% (LayerOpacity).
        let mut doc = CanvasDocument::new(10, 10);
        let mut folder = ImageLayer::blank("Folder", doc.size());
        folder.is_group = true;
        folder.opacity = 0.5;
        let folder_id = folder.id;
        let mut child = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 0.5);
        child.parent_id = Some(folder_id);
        doc.layers = vec![folder, child];
        let out = draw_live_composite(&doc);
        let a = out.pixel(5, 5)[3];
        assert!(a >= 55 && a <= 75, "{a}");
    }

    #[test]
    fn clipping_stack_shares_base_coverage() {
        // Base: red, left half. Child: blue over the whole canvas, clipped to
        // the base. The whole stack shows only where the base had alpha, and
        // the child lands over the base within that coverage.
        let mut doc = CanvasDocument::new(20, 10);
        let mut base = Bitmap::new(10, 10);
        for i in (0..base.data.len()).step_by(4) {
            base.data[i] = 255;
            base.data[i + 3] = 255;
        }
        let base_layer = ImageLayer::with_asset(ImportedImage::with_thumbnail(base, Bitmap::new(2, 2), "B"), CGPoint::ZERO);
        let mut blue_img = Bitmap::new(20, 10);
        for i in (0..blue_img.data.len()).step_by(4) {
            blue_img.data[i + 2] = 255;
            blue_img.data[i + 3] = 255;
        }
        let mut child = ImageLayer::with_asset(ImportedImage::with_thumbnail(blue_img, Bitmap::new(4, 2), "C"), CGPoint::ZERO);
        child.mask_source_id = Some(base_layer.id);
        doc.layers = vec![base_layer, child];
        let out = draw_live_composite(&doc);
        assert_eq!(out.pixel(5, 5), [0, 0, 255, 255]);
        assert_eq!(out.pixel(15, 5)[3], 0);
    }

    #[test]
    fn effects_are_not_masked_twice() {
        // With effects the mask is baked into the effect render and the result
        // is drawn mask-free (EditorCanvas draws effects `mask: nil`); a soft
        // mask edge must keep its coverage, not lose it to a second multiply.
        use crate::effects::{ColorOverlayEffect, LayerEffects};
        let mut doc = CanvasDocument::new(16, 10);
        let mut layer = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0);
        let mut mask_image = Bitmap::new(10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let v = match x {
                    0..=4 => 255u8,
                    5 => 128,
                    _ => 0,
                };
                mask_image.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        layer.mask = Some(crate::document::LayerMask {
            asset: ImportedImage::with_thumbnail(mask_image, Bitmap::new(4, 2), "Mask"),
            is_enabled: true,
            placement: None,
            is_linked: true,
        });
        let mut overlay = ColorOverlayEffect::default();
        overlay.enabled = Some(true);
        layer.effects = Some(LayerEffects {
            color_overlay: Some(overlay),
            ..Default::default()
        });
        doc.layers = vec![layer];
        let out = draw_live_composite(&doc);
        // Inside the mask the overlay color shows at full coverage.
        let solid = out.pixel(2, 5);
        assert_eq!(solid[3], 255);
        // The soft edge keeps a single application of the mask: the overlay's
        // source-over lifts the 50% pixel to ~75% (192), and a second mask
        // multiply would square it back to ~96.
        let soft = out.pixel(5, 5);
        assert!(
            (170..=210).contains(&soft[3]),
            "soft mask edge alpha {}, single mask expected",
            soft[3]
        );
        assert_eq!(out.pixel(8, 5)[3], 0);
    }

    #[test]
    fn shape_rectangle_fills_with_color() {
    #[test]
    fn effects_cache_hits_for_identical_input() {
        let mut doc = CanvasDocument::new(10, 10);
        let mut layer = red_layer(&doc, 0.0, crate::blend::LayerBlendMode::Normal, 1.0);
        let mut overlay = crate::effects::ColorOverlayEffect::default();
        overlay.enabled = Some(true);
        layer.effects = Some(crate::effects::LayerEffects {
            color_overlay: Some(overlay),
            ..Default::default()
        });
        doc.layers = vec![layer];
        // Two passes over the same document: the cache must return the same
        // result for the second (that is its whole purpose).
        let first = draw_live_composite(&doc);
        let second = draw_live_composite(&doc);
        assert_eq!(first.pixel(5, 5), second.pixel(5, 5));
    }

    #[test]
    fn mask_clip_limit_caps_the_longest_side() {
        let mut doc = CanvasDocument::new(40, 40);
        let mut layer = ImageLayer::blank("L", doc.size());
        layer.transform = LayerTransform::new(CGPoint::ZERO, CGSize::new(40.0, 40.0));
        let mut mask_image = Bitmap::new(40, 40);
        for pixel in mask_image.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
        layer.mask = Some(crate::document::LayerMask {
            asset: ImportedImage::with_thumbnail(
                mask_image,
                Bitmap::new(4, 4),
                "Mask",
            ),
            is_enabled: true,
            placement: Some(LayerTransform::new(CGPoint::ZERO, CGSize::new(40.0, 40.0))),
            is_linked: true,
        });
        doc.layers = vec![layer];
        let layer = &doc.layers[0];
        let full = mask_clip_image(layer.mask.as_ref().unwrap(), &layer.transform, 40, 40)
            .expect("full mask");
        assert_eq!((full.width, full.height), (40, 40));
        let limited = mask_clip_image_limited(layer.mask.as_ref().unwrap(), &layer.transform, 40, 40, Some(10))
            .expect("limited mask");
        assert_eq!((limited.width, limited.height), (10, 10), "limit caps the grid");
    }

    #[test]
    fn effects_guard_degrades_huge_results() {
        // A layer that would rasterize effects over 100 megapixels draws
        // without them instead (LayerEffects.swift:441).
        let image = Bitmap::new(10_000, 10_000);
        let mut effects = crate::effects::LayerEffects::default();
        let mut shadow = crate::effects::ShadowEffect {
            enabled: Some(true),
            ..crate::effects::ShadowEffect::default()
        };
        shadow.distance = 5.0;
        effects.shadow = Some(shadow);
        let (out, inset) = crate::effects::render_effects(&image, None, &effects);
        assert_eq!(inset, 0.0, "guard degrades to no effects");
        assert_eq!((out.width, out.height), (10_000, 10_000));
    }

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
