//! Port of `Compositor/IO/PSD/PSDVector.swift`: rasterizes Photoshop vector
//! masks (`vmsk`/`vsms`) and maps fill rectangles/ellipses onto live shape
//! layers, from Adobe's 2019 Photoshop File Formats Specification (additional
//! layer information: `vmsk`, `vogk`, `SoCo`, `vstk`).
//!
//! The Swift rasterizes through Core Graphics (nonzero-winding fills, miter
//! strokes, anti-aliasing); the scanline rasterizer at the bottom of this file
//! stands in for `CGContext`.

use std::collections::HashMap;

use crate::document::{PaletteColor, ShapeKind};
use crate::geometry::{CGPoint, CGRect, CGSize};
use crate::raster::{Bitmap, source_over_u8};

use super::types::{LayerShapeStyle, PsdError, ascii_at, f64_at, find_offset, i16_at, i32_at};

/// `EditorSession.maxShapePixels`.
pub const MAX_SHAPE_PIXELS: i64 = 100_000_000;

pub struct PsdVector;

/// `PSDVector.Raster`: a rasterized vector mask and where it sits on the canvas.
#[derive(Debug, Clone)]
pub struct RasterOut {
    pub image: Bitmap,
    pub bounds: CGRect,
}

/// `PSDVector.Live`: a shape layer that stays editable.
#[derive(Debug, Clone)]
pub struct Live {
    pub style: LayerShapeStyle,
    pub bounds: CGRect,
    pub image: Bitmap,
    pub notes: Vec<String>,
}

/// `PSDVector.Origination`.
struct Origination {
    kind: ShapeKind,
    bounds: CGRect,
    corner_radius: f64,
    notes: Vec<String>,
}

impl PsdVector {
    /// `PSDVector.live(extra:canvas:remainingPixels:)`.
    pub fn live(
        extra: &HashMap<String, Vec<u8>>,
        canvas: CGSize,
        remaining_pixels: i64,
    ) -> Result<Option<Live>, PsdError> {
        let stroke = extra.get("vstk");
        let fill_enabled = stroke
            .and_then(|d| bool_field(d, "fillEnabled"))
            .unwrap_or_else(|| extra.contains_key("SoCo"));
        let stroke_enabled = stroke
            .and_then(|d| bool_field(d, "strokeEnabled"))
            .unwrap_or(false);
        if !fill_enabled {
            return Ok(None);
        }
        let Some(soco) = extra.get("SoCo") else {
            return Ok(None);
        };
        let Some(fill) = rgb(soco) else {
            return Ok(None);
        };
        let origin = match origination(extra.get("vogk")) {
            Some(found) => Some(found),
            None => sharp_rect(
                extra
                    .get("vmsk")
                    .or_else(|| extra.get("vsms"))
                    .map(|d| d.as_slice()),
                canvas,
            ),
        };
        let Some(origin) = origin else {
            return Ok(None);
        };
        let mut box_ = origin.bounds.integral();
        if !box_.origin.x.is_finite() || !box_.origin.y.is_finite() {
            return Ok(None);
        }
        let Some((width, height)) = pixel_size(box_.size, remaining_pixels)? else {
            return Ok(None);
        };
        box_.size = CGSize::new(width as f64, height as f64);
        let style = LayerShapeStyle::new(origin.kind, fill.0, fill.1, fill.2, origin.corner_radius);
        let image = shape_image(style.kind, box_.size, style.color(), style.corner_radius);
        let mut notes: Vec<String> = Vec::new();
        if stroke_enabled {
            notes.push(
                "The Photoshop stroke isn\u{2019}t supported on shape layers and was omitted."
                    .to_string(),
            );
        }
        notes.extend(origin.notes);
        Ok(Some(Live {
            style,
            bounds: box_,
            image,
            notes,
        }))
    }

    /// `PSDVector.raster(extra:canvas:remainingPixels:)`.
    pub fn raster(
        extra: &HashMap<String, Vec<u8>>,
        canvas: CGSize,
        remaining_pixels: i64,
    ) -> Result<Option<RasterOut>, PsdError> {
        let Some(mask) = extra.get("vmsk").or_else(|| extra.get("vsms")) else {
            return Ok(None);
        };
        let Some(path) = PsdPath::parse(mask, canvas) else {
            return Ok(None);
        };
        let fill = extra.get("SoCo").and_then(|d| rgb(d));
        let stroke = extra.get("vstk");
        let fill_enabled = stroke
            .and_then(|d| bool_field(d, "fillEnabled"))
            .unwrap_or(fill.is_some());
        let stroke_enabled = stroke
            .and_then(|d| bool_field(d, "strokeEnabled"))
            .unwrap_or(false);
        let stroke_color = stroke.and_then(|d| rgb(d));
        let stroke_width = stroke
            .and_then(|d| unit_field(d, "strokeStyleLineWidth", 0))
            .unwrap_or(1.0);
        if !(fill_enabled && fill.is_some()) && !(stroke_enabled && stroke_color.is_some()) {
            return Ok(None);
        }
        if !stroke_width.is_finite() {
            return Ok(None);
        }
        if stroke_enabled && !(0.0..=30_000.0).contains(&stroke_width) {
            return Err(PsdError::TooLarge);
        }
        let mut box_ = path.bounding_box().unwrap_or(CGRect::ZERO);
        if stroke_enabled {
            let inset = (stroke_width / 2.0 + 1.0).ceil();
            box_ = box_.inset_by(-inset, -inset);
        }
        box_ = box_.integral();
        if !box_.origin.x.is_finite() || !box_.origin.y.is_finite() {
            return Ok(None);
        }
        let Some((width, height)) = pixel_size(box_.size, remaining_pixels)? else {
            return Ok(None);
        };
        let mut image = Bitmap::new(width, height);
        // `context.translateBy(x: -box.minX, y: -box.minY)`.
        let subpaths = path.flatten();
        let shifted: Vec<Vec<CGPoint>> = subpaths
            .iter()
            .map(|(points, _)| {
                points
                    .iter()
                    .map(|p| CGPoint::new(p.x - box_.min_x(), p.y - box_.min_y()))
                    .collect()
            })
            .collect();
        let closed: Vec<bool> = subpaths.iter().map(|(_, closed)| *closed).collect();
        if fill_enabled && let Some(fill_color) = fill {
            let color = PaletteColor::new(fill_color.0, fill_color.1, fill_color.2);
            fill_polygons(&mut image, &shifted, color);
        }
        if stroke_enabled && let Some(stroke_rgb) = stroke_color {
            // CG strokes a closed subpath with the same miter join at the
            // closing corner (`closeSubpath`, `setLineJoin(.miter)`,
            // `setMiterLimit(10)`, bevel past the limit) as at every other
            // vertex; pass `closed` so the last-to-first corner is joined
            // rather than butt-capped the way an open polyline ends.
            for (points, closed) in shifted.iter().zip(&closed) {
                let outline = stroke_outline(points, *closed, stroke_width, 10.0);
                if !outline.is_empty() {
                    fill_polygons(
                        &mut image,
                        &[outline],
                        PaletteColor::new(stroke_rgb.0, stroke_rgb.1, stroke_rgb.2),
                    );
                }
            }
        }
        Ok(Some(RasterOut {
            image,
            bounds: CGRect::new(box_.min_x(), box_.min_y(), width as f64, height as f64),
        }))
    }
}

/// Rejects sizes that would trap on `Int(...)` or exceed the 30,000 px /
/// remaining-pixel budget. `Ok(None)` means "not finite, give up quietly".
fn pixel_size(size: CGSize, remaining_pixels: i64) -> Result<Option<(usize, usize)>, PsdError> {
    if !size.width.is_finite() || !size.height.is_finite() {
        return Ok(None);
    }
    let max_dimension = 30_000.0f64;
    if size.width.abs() > max_dimension || size.height.abs() > max_dimension {
        return Err(PsdError::TooLarge);
    }
    let budget = MAX_SHAPE_PIXELS.min(remaining_pixels.max(0));
    if size.width * size.height > budget as f64 {
        return Err(PsdError::TooLarge);
    }
    let width = (size.width as i64).max(1) as usize;
    let height = (size.height as i64).max(1) as usize;
    if (width * height) as i64 > budget {
        return Err(PsdError::TooLarge);
    }
    Ok(Some((width, height)))
}

/// Photoshop `vogk` origination: 1/2 = rectangle (2 is rounded), 5 = ellipse.
fn origination(data: Option<&Vec<u8>>) -> Option<Origination> {
    let data = data?;
    let type_ = int32_field(data, "keyOriginType")?;
    let kind = match type_ {
        1 | 2 => ShapeKind::Rectangle,
        5 => ShapeKind::Ellipse,
        _ => return None,
    };
    let from = find_offset(data, b"keyOriginShapeBBox", 0).unwrap_or(0);
    let left = unit_field(data, "Left", from)?;
    let top = unit_field(data, "Top ", from)?;
    let right = unit_field(data, "Rght", from)?;
    let bottom = unit_field(data, "Btom", from)?;
    let bounds = CGRect::new(left, top, right - left, bottom - top);
    if bounds.size.width < 1.0
        || bounds.size.height < 1.0
        || !bounds.origin.x.is_finite()
        || !bounds.origin.y.is_finite()
        || !bounds.size.width.is_finite()
        || !bounds.size.height.is_finite()
    {
        return None;
    }
    let mut origin = Origination {
        kind,
        bounds,
        corner_radius: 0.0,
        notes: Vec::new(),
    };
    if kind == ShapeKind::Rectangle
        && let Some(radii_at) = find_offset(data, b"keyOriginRRectRadii", 0)
    {
        let keys = ["topLeft", "topRight", "bottomRight", "bottomLeft"];
        let radii: Vec<f64> = keys
            .iter()
            .filter_map(|key| unit_field(data, key, radii_at))
            .collect();
        if radii.len() == 4 {
            let lo = radii.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = radii.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if hi - lo > 0.5 {
                return None;
            }
            origin.corner_radius = hi;
        }
    }
    Some(origin)
}

/// Four straight-corner anchors infer a rectangle without origination.
fn sharp_rect(data: Option<&[u8]>, canvas: CGSize) -> Option<Origination> {
    let data = data?;
    let path = PsdPath::parse(data, canvas)?;
    let mut offset = 8usize;
    let mut remaining = 0i64;
    let mut anchors: Vec<CGPoint> = Vec::new();
    let mut sharp = true;
    while offset + 26 <= data.len() {
        let type_ = i16_at(data, offset);
        let body = &data[offset + 2..offset + 26];
        offset += 26;
        match type_ {
            0 | 3 => {
                if !anchors.is_empty() {
                    return None;
                }
                remaining = i16_at(body, 0) as i64;
            }
            1 | 2 | 4 | 5 => {
                if remaining <= 0 || body.len() < 24 {
                    continue;
                }
                remaining -= 1;
                let incoming = point(body, 0, canvas);
                let anchor = point(body, 8, canvas);
                let outgoing = point(body, 16, canvas);
                if distance(incoming, anchor) > 0.5 || distance(outgoing, anchor) > 0.5 {
                    sharp = false;
                }
                anchors.push(anchor);
            }
            _ => continue,
        }
    }
    if !sharp || anchors.len() != 4 {
        return None;
    }
    let box_ = path.bounding_box()?;
    if box_.size.width < 1.0 || box_.size.height < 1.0 {
        return None;
    }
    Some(Origination {
        kind: ShapeKind::Rectangle,
        bounds: box_,
        corner_radius: 0.0,
        notes: Vec::new(),
    })
}

/// `PSDVector.rgb(_:)`: the `SoCo` solid-color descriptor.
pub fn rgb(data: &[u8]) -> Option<(f64, f64, f64)> {
    let r = double_field(data, "Rd  ")?;
    let g = double_field(data, "Grn ")?;
    let b = double_field(data, "Bl  ")?;
    fn channel(value: f64) -> f64 {
        if value > 1.0 {
            value.clamp(0.0, 255.0) / 255.0
        } else {
            value.clamp(0.0, 1.0)
        }
    }
    Some((channel(r), channel(g), channel(b)))
}

fn bool_field(data: &[u8], key: &str) -> Option<bool> {
    let start = find_offset(data, key.as_bytes(), 0)?;
    let type_at = start + key.len();
    if type_at + 5 > data.len() || ascii_at(data, type_at, 4) != "bool" {
        return None;
    }
    Some(data[type_at + 4] != 0)
}

fn unit_field(data: &[u8], key: &str, from: usize) -> Option<f64> {
    let key_at = find_offset(data, key.as_bytes(), from)?;
    let unit_at = find_offset(data, b"UntF", key_at)?;
    // "UntF" + 4-byte unit + 8-byte double; a truncated descriptor must read
    // as absent (Swift's `double(at:in:)` returns nil) rather than panic.
    if unit_at + 16 > data.len() {
        return None;
    }
    Some(f64_at(data, unit_at + 8))
}

fn int32_field(data: &[u8], key: &str) -> Option<i32> {
    let start = find_offset(data, key.as_bytes(), 0)?;
    let type_at = start + key.len();
    if type_at + 8 > data.len() || ascii_at(data, type_at, 4) != "long" {
        return None;
    }
    Some(i32_at(data, type_at + 4))
}

fn double_field(data: &[u8], key: &str) -> Option<f64> {
    let start = find_offset(data, key.as_bytes(), 0)?;
    let type_at = start + key.len();
    if type_at + 12 > data.len() || ascii_at(data, type_at, 4) != "doub" {
        return None;
    }
    Some(f64_at(data, type_at + 4))
}

/// `PSDVector.point`: one 24-byte knot body holds incoming/anchor/outgoing as
/// (y, x) fixed-point thirds of 0x1000000.
fn point(bytes: &[u8], at: usize, canvas: CGSize) -> CGPoint {
    let y = i32_at(bytes, at) as f64 / 0x1000000 as f64;
    let x = i32_at(bytes, at + 4) as f64 / 0x1000000 as f64;
    CGPoint::new(x * canvas.width, y * canvas.height)
}

fn distance(a: CGPoint, b: CGPoint) -> f64 {
    ((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y)).sqrt()
}

// MARK: - Path

/// The subset of `CGPath` the vector mask parsing needs.
#[derive(Debug, Clone)]
enum PathElement {
    Move(CGPoint),
    Curve {
        to: CGPoint,
        c1: CGPoint,
        c2: CGPoint,
    },
    Close,
}

/// `PSDVector.path(from:canvas:)`: 26-byte records of selector + knot bodies.
#[derive(Debug, Clone)]
pub struct PsdPath {
    elements: Vec<PathElement>,
}

impl PsdPath {
    pub fn parse(data: &[u8], canvas: CGSize) -> Option<PsdPath> {
        if data.len() < 8 || canvas.width <= 0.0 || canvas.height <= 0.0 {
            return None;
        }
        let mut elements: Vec<PathElement> = Vec::new();
        let mut offset = 8usize;
        let mut remaining = 0i64;
        let mut closed = true;
        let mut first = true;
        let mut previous_out = CGPoint::ZERO;
        while offset + 26 <= data.len() {
            let type_ = i16_at(data, offset);
            let body = &data[offset + 2..offset + 26];
            offset += 26;
            match type_ {
                // 0/3: closed/open subpath length records; 1/2/4/5: knots.
                0 | 3 => {
                    if !first && closed {
                        elements.push(PathElement::Close);
                    }
                    remaining = i16_at(body, 0) as i64;
                    closed = type_ == 0;
                    first = true;
                }
                1 | 2 | 4 | 5 => {
                    if remaining <= 0 || body.len() < 24 {
                        continue;
                    }
                    remaining -= 1;
                    let incoming = point(body, 0, canvas);
                    let anchor = point(body, 8, canvas);
                    let outgoing = point(body, 16, canvas);
                    if first {
                        elements.push(PathElement::Move(anchor));
                        first = false;
                    } else {
                        elements.push(PathElement::Curve {
                            to: anchor,
                            c1: previous_out,
                            c2: incoming,
                        });
                    }
                    previous_out = outgoing;
                }
                _ => continue,
            }
        }
        if !first && closed {
            elements.push(PathElement::Close);
        }
        if elements.is_empty() {
            None
        } else {
            Some(PsdPath { elements })
        }
    }

    /// `CGPath.boundingBoxOfPath`: every point, Bézier control points included.
    pub fn bounding_box(&self) -> Option<CGRect> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut any = false;
        let mut add = |p: CGPoint| {
            any = true;
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        };
        for element in &self.elements {
            match element {
                PathElement::Move(p) => add(*p),
                PathElement::Curve { to, c1, c2 } => {
                    add(*c1);
                    add(*c2);
                    add(*to);
                }
                PathElement::Close => {}
            }
        }
        if !any {
            return None;
        }
        Some(CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    /// Subpaths as polylines; closed subpaths repeat their first point so both
    /// fill and stroke see the closing edge.
    pub fn flatten(&self) -> Vec<(Vec<CGPoint>, bool)> {
        let mut result: Vec<(Vec<CGPoint>, bool)> = Vec::new();
        let mut current: Vec<CGPoint> = Vec::new();
        let mut start_point = CGPoint::ZERO;
        let mut current_point = CGPoint::ZERO;
        for element in &self.elements {
            match element {
                PathElement::Move(p) => {
                    if !current.is_empty() {
                        result.push((std::mem::take(&mut current), false));
                    }
                    start_point = *p;
                    current_point = *p;
                    current.push(*p);
                }
                PathElement::Curve { to, c1, c2 } => {
                    let from = current_point;
                    flatten_cubic_into(&mut current, from, *c1, *c2, *to);
                    current_point = *to;
                }
                PathElement::Close => {
                    if current.len() >= 2 {
                        current.push(start_point);
                        result.push((std::mem::take(&mut current), true));
                    }
                }
            }
        }
        if current.len() >= 2 {
            result.push((current, false));
        }
        result
    }
}

// MARK: - Rasterization

const KAPPA: f64 = 0.5522847498307936;

/// `EditorSession.shapeImage`: the shape filling its box, anti-aliased where
/// it curves.
pub fn shape_image(
    kind: ShapeKind,
    size: CGSize,
    color: PaletteColor,
    corner_radius: f64,
) -> Bitmap {
    let width = (size.width.max(0.0) as usize).max(1);
    let height = (size.height.max(0.0) as usize).max(1);
    let mut image = Bitmap::new(width, height);
    let (w, h) = (width as f64, height as f64);
    let poly = match kind {
        ShapeKind::Ellipse => ellipse_poly(w, h),
        // A rectangle's corners round by its radius (`ShapeKind.path`); PSD
        // import never produces lines, so Line fills the box.
        ShapeKind::Line | ShapeKind::Rectangle | ShapeKind::RoundedRectangle => {
            rounded_rectangle_poly(w, h, corner_radius)
        }
    };
    fill_polygons(&mut image, &[poly], color);
    image
}

fn rectangle_poly(w: f64, h: f64) -> Vec<CGPoint> {
    vec![
        CGPoint::new(0.0, 0.0),
        CGPoint::new(w, 0.0),
        CGPoint::new(w, h),
        CGPoint::new(0.0, h),
    ]
}

fn rounded_rectangle_poly(w: f64, h: f64, corner_radius: f64) -> Vec<CGPoint> {
    let radius = corner_radius.max(0.0).min(w / 2.0).min(h / 2.0);
    if radius <= 0.0 {
        return rectangle_poly(w, h);
    }
    let k = KAPPA * radius;
    let mut pts: Vec<CGPoint> = Vec::new();
    pts.push(CGPoint::new(radius, 0.0));
    pts.push(CGPoint::new(w - radius, 0.0));
    flatten_cubic_into(
        &mut pts,
        CGPoint::new(w - radius, 0.0),
        CGPoint::new(w - radius + k, 0.0),
        CGPoint::new(w, radius - k),
        CGPoint::new(w, radius),
    );
    pts.push(CGPoint::new(w, h - radius));
    flatten_cubic_into(
        &mut pts,
        CGPoint::new(w, h - radius),
        CGPoint::new(w, h - radius + k),
        CGPoint::new(w - radius + k, h),
        CGPoint::new(w - radius, h),
    );
    pts.push(CGPoint::new(radius, h));
    flatten_cubic_into(
        &mut pts,
        CGPoint::new(radius, h),
        CGPoint::new(radius - k, h),
        CGPoint::new(0.0, h - radius + k),
        CGPoint::new(0.0, h - radius),
    );
    pts.push(CGPoint::new(0.0, radius));
    flatten_cubic_into(
        &mut pts,
        CGPoint::new(0.0, radius),
        CGPoint::new(0.0, radius - k),
        CGPoint::new(radius - k, 0.0),
        CGPoint::new(radius, 0.0),
    );
    pts
}

fn ellipse_poly(w: f64, h: f64) -> Vec<CGPoint> {
    let cx = w / 2.0;
    let cy = h / 2.0;
    let kx = KAPPA * w / 2.0;
    let ky = KAPPA * h / 2.0;
    let right = CGPoint::new(cx + w / 2.0, cy);
    let bottom = CGPoint::new(cx, cy + h / 2.0);
    let left = CGPoint::new(cx - w / 2.0, cy);
    let top = CGPoint::new(cx, cy - h / 2.0);
    let mut pts: Vec<CGPoint> = vec![right];
    flatten_cubic_into(
        &mut pts,
        right,
        CGPoint::new(cx + w / 2.0, cy + ky),
        CGPoint::new(cx + kx, cy + h / 2.0),
        bottom,
    );
    flatten_cubic_into(
        &mut pts,
        bottom,
        CGPoint::new(cx - kx, cy + h / 2.0),
        CGPoint::new(cx - w / 2.0, cy + ky),
        left,
    );
    flatten_cubic_into(
        &mut pts,
        left,
        CGPoint::new(cx - w / 2.0, cy - ky),
        CGPoint::new(cx - kx, cy - h / 2.0),
        top,
    );
    flatten_cubic_into(
        &mut pts,
        top,
        CGPoint::new(cx + kx, cy - h / 2.0),
        CGPoint::new(cx + w / 2.0, cy - ky),
        right,
    );
    pts
}

fn flatten_cubic_into(
    out: &mut Vec<CGPoint>,
    from: CGPoint,
    c1: CGPoint,
    c2: CGPoint,
    to: CGPoint,
) {
    let approx = distance(c1, from) + distance(c2, c1) + distance(to, c2);
    let steps = ((approx / 2.0).ceil() as usize).clamp(4, 48);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        out.push(cubic_point(from, c1, c2, to, t));
    }
}

fn cubic_point(p0: CGPoint, p1: CGPoint, p2: CGPoint, p3: CGPoint, t: f64) -> CGPoint {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    CGPoint::new(
        a * p0.x + b * p1.x + c * p2.x + d * p3.x,
        a * p0.y + b * p1.y + c * p2.y + d * p3.y,
    )
}

/// The polygon Core Graphics strokes around `poly` with miter joins and butt
/// caps (`setLineJoin(.miter)`, `setMiterLimit(10)`, `setLineCap(.butt)`).
fn stroke_outline(poly: &[CGPoint], closed: bool, width: f64, miter_limit: f64) -> Vec<CGPoint> {
    let half = width / 2.0;
    let n = poly.len();
    if n < 2 || half <= 0.0 || !width.is_finite() {
        return Vec::new();
    }
    let segment_count = if closed { n } else { n - 1 };
    let dirs: Vec<(usize, CGPoint)> = (0..segment_count)
        .filter_map(|i| {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len <= 1e-9 {
                None
            } else {
                Some((i, CGPoint::new(dx / len, dy / len)))
            }
        })
        .collect();
    if dirs.is_empty() {
        return Vec::new();
    }
    let mut right: Vec<CGPoint> = Vec::new();
    let mut left: Vec<CGPoint> = Vec::new();
    let (first_index, first_dir) = dirs[0];
    let start = poly[first_index];
    let first_normal = CGPoint::new(-first_dir.y * half, first_dir.x * half);
    right.push(CGPoint::new(
        start.x - first_normal.x,
        start.y - first_normal.y,
    ));
    left.push(CGPoint::new(
        start.x + first_normal.x,
        start.y + first_normal.y,
    ));
    for side_index in 0..dirs.len() {
        let (i, d1) = dirs[side_index];
        let end = poly[(i + 1) % n];
        let has_joint = closed || side_index + 1 < dirs.len();
        let n1 = CGPoint::new(-d1.y * half, d1.x * half);
        if has_joint {
            let (_, d2) = dirs[(side_index + 1) % dirs.len()];
            let n2 = CGPoint::new(-d2.y * half, d2.x * half);
            joint(&mut left, end, n1, n2, d1, d2, 1.0, width, miter_limit);
            joint(
                &mut right,
                end,
                CGPoint::new(-n1.x, -n1.y),
                CGPoint::new(-n2.x, -n2.y),
                d1,
                d2,
                -1.0,
                width,
                miter_limit,
            );
        } else {
            left.push(CGPoint::new(end.x + n1.x, end.y + n1.y));
            right.push(CGPoint::new(end.x - n1.x, end.y - n1.y));
        }
    }
    let mut outline = Vec::with_capacity(left.len() + right.len());
    outline.extend_from_slice(&right);
    outline.extend(left.iter().rev());
    outline
}

/// One side of a segment joint: a miter point within the limit, a bevel past it.
#[allow(clippy::too_many_arguments)]
fn joint(
    chain: &mut Vec<CGPoint>,
    vertex: CGPoint,
    n1: CGPoint,
    n2: CGPoint,
    d1: CGPoint,
    d2: CGPoint,
    side: f64,
    width: f64,
    miter_limit: f64,
) {
    let a = CGPoint::new(vertex.x + n1.x, vertex.y + n1.y);
    let b = CGPoint::new(vertex.x + n2.x, vertex.y + n2.y);
    let cross = d1.x * d2.y - d1.y * d2.x;
    if cross * side <= 1e-12 {
        // Inner side of the turn (or straight through): push both offsets.
        chain.push(a);
        chain.push(b);
        return;
    }
    // Outer side: intersect the two offset segment lines for the miter point.
    let det = -cross;
    if det.abs() <= 1e-12 {
        chain.push(a);
        chain.push(b);
        return;
    }
    let rhs_x = b.x - a.x;
    let rhs_y = b.y - a.y;
    let t = (-rhs_x * d2.y + d2.x * rhs_y) / det;
    let miter = CGPoint::new(a.x + t * d1.x, a.y + t * d1.y);
    let miter_length = distance(miter, vertex);
    if miter_length / width > miter_limit {
        chain.push(a);
        chain.push(b);
    } else {
        chain.push(miter);
    }
}

/// Non-zero-winding fill with analytic (Core Graphics-style) anti-aliasing,
/// composited source-over.
///
/// Rather than sampling a fixed sub-pixel grid, each pixel accumulates the
/// exact signed area of the winding region inside it. Working one pixel row
/// band `[y, y+1)` at a time, every edge active in the band contributes
/// `delta · ∫ clamp(px+1 − max(px, x_edge(t)), 0, 1) dt` to column `px` —
/// the trapezoid it sweeps across the column — split into a partial-height
/// winding term plus an in-column trapezoid; columns right of an edge's span
/// receive the full band height through a prefix-summed winding event. The
/// pixel's gray level is the |signed area| clamped to 1: a half-covered edge
/// pixel is exactly 0.5 instead of a 16-step grid estimate, and a simple
/// polygon's total coverage equals its analytic area.
///
/// Declared substitution (rust-vs-swift-diff-report §9.2-3): the Swift fills
/// through Core Graphics' analytic anti-aliased winding rasterizer; this
/// scanline integrator is the file's stand-in `CGContext` (see the module
/// docs), an intentional replacement rather than a bug to be made bit-exact.
/// Signed-area accumulation is the standard analytic nonzero rasterizer:
/// opposite windings overlapping inside one pixel cancel, and any excess
/// (|area| > 1) clamps to solid, matching CG outside degenerate
/// self-overlaps.
fn fill_polygons(target: &mut Bitmap, polys: &[Vec<CGPoint>], color: PaletteColor) {
    let mut edges: Vec<[f64; 4]> = Vec::new();
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for poly in polys {
        if poly.len() < 2 {
            continue;
        }
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if a.x == b.x && a.y == b.y {
                continue;
            }
            if a.y != b.y {
                edges.push([a.x, a.y, b.x, b.y]);
            }
            min_x = min_x.min(a.x).min(b.x);
            min_y = min_y.min(a.y).min(b.y);
            max_x = max_x.max(a.x).max(b.x);
            max_y = max_y.max(a.y).max(b.y);
        }
    }
    if edges.is_empty() {
        return;
    }
    let x0 = ((min_x.floor().max(0.0)) as usize).min(target.width);
    let x1 = ((max_x.ceil() + 1.0).max(0.0) as usize).min(target.width);
    let y0 = ((min_y.floor().max(0.0)) as usize).min(target.height);
    let y1 = ((max_y.ceil() + 1.0).max(0.0) as usize).min(target.height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let cols = x1 - x0;
    // Coverage only over the shape's bounding window: allocating the whole
    // target bitmap cost 4 bytes per canvas pixel for a shape that may cover
    // a corner of it.
    let rows = y1 - y0;
    let mut coverage = vec![0.0f32; rows * cols];
    // Per-band accumulators indexed relative to x0. `area_row` holds the
    // signed trapezoid each straddling edge sweeps inside the column,
    // `height_row` the partial band height over which its winding already
    // applies, and `full_events` marks where the full band height starts
    // (one slot past the window so right-of-window edges can park safely).
    let mut area_row = vec![0.0f64; cols];
    let mut height_row = vec![0.0f64; cols];
    let mut full_events = vec![0.0f64; cols + 1];
    // Edge segments active in the current band: x at the band's top and
    // bottom, the clipped t-range, and the winding direction.
    let mut segs: Vec<[f64; 5]> = Vec::new();
    for y in y0..y1 {
        segs.clear();
        for edge in &edges {
            let (ax, ay, bx, by) = (edge[0], edge[1], edge[2], edge[3]);
            // Half-open span [min(ay,by), max(ay,by)) so shared vertices
            // count once, clipped to this band.
            let t0 = ay.min(by).max(y as f64);
            let t1 = ay.max(by).min(y as f64 + 1.0);
            if t0 >= t1 {
                continue;
            }
            let x_at = |t: f64| ax + (t - ay) * (bx - ax) / (by - ay);
            segs.push([x_at(t0), x_at(t1), t0, t1, if by > ay { 1.0 } else { -1.0 }]);
        }
        if segs.is_empty() {
            continue;
        }
        area_row.fill(0.0);
        height_row.fill(0.0);
        full_events.fill(0.0);
        for &[xa, xb, s0, s1, delta] in &segs {
            let height = s1 - s0;
            let slope = (xb - xa) / height;
            // Columns the segment straddles get the exact trapezoid and the
            // partial-height winding; every column from the last one on gets
            // the full band height through the event below. Segments entirely
            // left of the window (clipped shapes, negative coordinates) fire
            // their event at the first column so their winding still counts.
            let last = (xa.max(xb).floor() as isize).min(x1 as isize - 1);
            for px in (xa.min(xb).floor() as isize).max(x0 as isize)..=last {
                let left = px as f64;
                // Band height over which the edge's winding applies in this
                // column: where the edge sits at or left of the pixel's left
                // boundary (an edge exactly on it counts, matching the old
                // `crossing <= sample` rule).
                let (h, area) = if slope == 0.0 {
                    let h = if xa <= left { height } else { 0.0 };
                    let area = if left < xa && xa < left + 1.0 {
                        (left + 1.0 - xa) * height
                    } else {
                        0.0
                    };
                    (h, area)
                } else {
                    let t_left = s0 + (left - xa) / slope;
                    let h = if xb > xa {
                        t_left.max(s0).min(s1) - s0
                    } else {
                        s1 - t_left.max(s0).min(s1)
                    };
                    // ∫ (left + 1 − x(t)) dt over {left < x(t) < left+1}:
                    // the exact sliver the edge sweeps through the column.
                    let t_right = s0 + (left + 1.0 - xa) / slope;
                    let (lo, hi) = if t_left <= t_right {
                        (t_left, t_right)
                    } else {
                        (t_right, t_left)
                    };
                    let lo = lo.max(s0);
                    let hi = hi.min(s1);
                    let area = if lo < hi {
                        let x_lo = xa + (lo - s0) * slope;
                        let x_hi = xa + (hi - s0) * slope;
                        (hi - lo) * (left + 1.0) - (x_lo + x_hi) * (hi - lo) * 0.5
                    } else {
                        0.0
                    };
                    (h, area)
                };
                let idx = (px - x0 as isize) as usize;
                height_row[idx] += delta * h;
                area_row[idx] += delta * area;
            }
            let event = (last + 1).max(x0 as isize).min(x1 as isize);
            full_events[(event - x0 as isize) as usize] += delta * height;
        }
        // Sweep the band left to right: the pixel's signed winding area is
        // the full-height winding accumulated so far plus this column's
        // partial-height and trapezoid terms.
        let row_start = (y - y0) * cols;
        let mut full = 0.0f64;
        for px in x0..x1 {
            let idx = px - x0;
            full += full_events[idx];
            let a = (full + height_row[idx] + area_row[idx]).abs().min(1.0) as f32;
            coverage[row_start + idx] = a;
        }
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let c = coverage[(y - y0) * cols + (x - x0)].min(1.0);
            if c <= 0.0 {
                continue;
            }
            let cf = c as f64;
            let px = [
                (color.red * cf * 255.0 + 0.5) as u8,
                (color.green * cf * 255.0 + 0.5) as u8,
                (color.blue * cf * 255.0 + 0.5) as u8,
                (cf * 255.0 + 0.5) as u8,
            ];
            let dst = target.pixel(x, y);
            target.set_pixel(x, y, source_over_u8(dst, px));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ShapeKind;

    #[test]
    fn unit_field_on_truncated_descriptor_is_none() {
        // "UntF" present but fewer than 16 bytes remain after its start: the
        // read must fail gracefully instead of panicking on the u64 slice.
        let mut data = b"Left".to_vec();
        data.extend_from_slice(b"\0\0UntF");
        data.extend_from_slice(b"#Pxl");
        data.extend_from_slice(&[0, 1, 2, 3]); // only 4 of the 8 double bytes
        assert_eq!(unit_field(&data, "Left", 0), None);
        // A complete descriptor still reads.
        let mut full = b"Left\0\0UntF#Pxl".to_vec();
        full.extend_from_slice(&100.0f64.to_be_bytes());
        assert_eq!(unit_field(&full, "Left", 0), Some(100.0));
    }

    #[test]
    fn shape_image_fills_the_box() {
        let image = shape_image(
            ShapeKind::Ellipse,
            CGSize::new(10.0, 6.0),
            PaletteColor::new(1.0, 0.0, 0.0),
            0.0,
        );
        // Center is solid red; a corner stays transparent.
        assert_eq!(image.pixel(5, 3)[0], 255);
        assert_eq!(image.pixel(5, 3)[3], 255);
        assert_eq!(image.pixel(0, 0)[3], 0);
    }

    #[test]
    fn path_bounding_box_and_flatten() {
        // Version/flags, an initial-fill record, an open-length record naming
        // two knots, then those knots (incoming, anchor, outgoing as y/x pairs).
        // Note selector 3 is a *length* record here (as in the Swift switch);
        // open subpath knots come in as 4.
        let canvas = CGSize::new(100.0, 100.0);
        let mut data = vec![0, 0, 0, 3, 0, 0, 0, 0];
        let mut record = |selector: i16, body: &[u8]| {
            data.extend_from_slice(&selector.to_be_bytes());
            let mut padded = body.to_vec();
            padded.resize(24, 0);
            data.extend_from_slice(&padded);
        };
        record(6, &[]);
        record(3, &2i16.to_be_bytes());
        let knot = |anchor: (i32, i32)| {
            let mut body = Vec::new();
            for (y, x) in [anchor, anchor, anchor] {
                body.extend_from_slice(&y.to_be_bytes());
                body.extend_from_slice(&x.to_be_bytes());
            }
            body
        };
        record(4, &knot((0x0080_0000, 0x0040_0000))); // anchor (25, 50)
        record(4, &knot((0x0080_0000, 0x00C0_0000))); // anchor (75, 50)
        let path = PsdPath::parse(&data, canvas).unwrap();
        let box_ = path.bounding_box().unwrap();
        assert!((box_.min_x() - 25.0).abs() < 1e-9);
        assert!((box_.min_y() - 50.0).abs() < 1e-9);
        assert_eq!(box_.size.width, 50.0);
        assert_eq!(box_.size.height, 0.0);
        let subpaths = path.flatten();
        assert_eq!(subpaths.len(), 1);
        // The move plus the flattened curve between the two anchors.
        assert!(subpaths[0].0.len() >= 2);
        assert!((subpaths[0].0[0].x - 25.0).abs() < 1e-9);
        assert!((subpaths[0].0[subpaths[0].0.len() - 1].x - 75.0).abs() < 1e-9);
    }

    #[test]
    fn rgb_descriptor_reads_doubles() {
        let data = crate_color(0.0, 110.0 / 255.0, 1.0);
        let (r, g, b) = rgb(&data).unwrap();
        assert!(r.abs() < 1e-9);
        assert!((g - 110.0 / 255.0).abs() < 1e-12);
        assert!((b - 1.0).abs() < 1e-12);
    }

    /// A `"RGBC"` descriptor like the Swift test's `colorDescriptor`.
    pub(crate) fn crate_color(red: f64, green: f64, blue: f64) -> Vec<u8> {
        let mut data = b"RGBC".to_vec();
        for (key, value) in [("Rd  ", red), ("Grn ", green), ("Bl  ", blue)] {
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(b"doub");
            data.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        data
    }

    fn has_point(outline: &[CGPoint], x: f64, y: f64) -> bool {
        outline
            .iter()
            .any(|p| (p.x - x).abs() < 1e-9 && (p.y - y).abs() < 1e-9)
    }

    /// Right triangle (2,2)-(10,2)-(2,10) with `flatten`'s repeated first
    /// point, stroked at width 2 with CG's miter limit 10.
    fn triangle_with_repeated_point() -> Vec<CGPoint> {
        vec![
            CGPoint::new(2.0, 2.0),
            CGPoint::new(10.0, 2.0),
            CGPoint::new(2.0, 10.0),
            CGPoint::new(2.0, 2.0),
        ]
    }

    #[test]
    fn closed_triangle_stroke_joins_the_closing_corner() {
        // The closing joint at (2,2) sits where the left edge's offset line
        // (x = 2 + 1) crosses the bottom edge's offset line (y = 2 + 1).
        let outline = stroke_outline(&triangle_with_repeated_point(), true, 2.0, 10.0);
        assert!(
            has_point(&outline, 3.0, 3.0),
            "outline {outline:?} lacks the closing miter point (3, 3)"
        );
        // The offset feet (3,2) are replaced by the miter point, unlike the
        // old open-polyline behavior.
        assert!(!has_point(&outline, 3.0, 2.0));
        // Stroking the same ring as an open polyline (the previous behavior)
        // butt-caps the corner instead of joining it.
        let open = stroke_outline(&triangle_with_repeated_point(), false, 2.0, 10.0);
        assert!(!has_point(&open, 3.0, 3.0));
        assert!(has_point(&open, 3.0, 2.0));
    }

    #[test]
    fn closed_triangle_stroke_bevels_past_the_miter_limit() {
        // The closing miter is sqrt(2) long, so 0.7071x the width; limit 0.5
        // falls back to the bevel feet instead of the intersection point.
        let outline = stroke_outline(&triangle_with_repeated_point(), true, 2.0, 0.5);
        assert!(
            has_point(&outline, 3.0, 2.0),
            "outline {outline:?} lacks the bevel foot (3, 2)"
        );
        assert!(has_point(&outline, 2.0, 3.0));
        assert!(!has_point(&outline, 3.0, 3.0));
    }

    #[test]
    fn raster_stroke_covers_the_closed_path_closing_corner() {
        // 64 so the fixed-point thirds decode exactly (2^24 / 64 = 2^18).
        let canvas = CGSize::new(64.0, 64.0);
        let fixed = |value: f64, span: f64| (value / span * 16_777_216.0).round() as i32;
        let knot = |ax: f64, ay: f64| {
            let (fy, fx) = (fixed(ay, canvas.height), fixed(ax, canvas.width));
            let mut body = Vec::new();
            for _ in 0..3 {
                body.extend_from_slice(&fy.to_be_bytes());
                body.extend_from_slice(&fx.to_be_bytes());
            }
            body
        };
        let record = |selector: i16, body: &[u8]| {
            let mut out = selector.to_be_bytes().to_vec();
            out.resize(26, 0);
            out[2..body.len().min(24) + 2].copy_from_slice(body);
            out
        };
        // Closed square (2,2)-(8,8) as a `vmsk`: length record naming four
        // knots, then the knots.
        let mut mask = vec![0, 0, 0, 3, 0, 0, 0, 0];
        let mut count = 4i16.to_be_bytes().to_vec();
        count.resize(24, 0);
        mask.extend_from_slice(&record(0, &count));
        for (x, y) in [(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)] {
            mask.extend_from_slice(&record(1, &knot(x, y)));
        }
        // Stroke-only `vstk`: enabled, width 2, opaque red.
        let mut stroke = b"strokeEnabled".to_vec();
        stroke.extend_from_slice(b"bool");
        stroke.push(1);
        stroke.extend_from_slice(b"fillEnabled");
        stroke.extend_from_slice(b"bool");
        stroke.push(0);
        stroke.extend_from_slice(b"strokeStyleLineWidth");
        stroke.extend_from_slice(b"UntF");
        stroke.extend_from_slice(b"#Pxl");
        stroke.extend_from_slice(&2.0f64.to_bits().to_be_bytes());
        stroke.extend_from_slice(&crate_color(1.0, 0.0, 0.0)[4..]);
        let mut extra = HashMap::new();
        extra.insert("vmsk".to_string(), mask);
        extra.insert("vstk".to_string(), stroke);
        let out = PsdVector::raster(&extra, canvas, MAX_SHAPE_PIXELS)
            .unwrap()
            .unwrap();
        assert_eq!(out.bounds.origin.x, 0.0);
        assert_eq!(out.bounds.size.width, 10.0);
        // The 2px ring covers [1,9]x[1,9]. The closing corner pixel is painted
        // now that the ring closes with a join (butt caps left it empty; a
        // bevel still covers half of it, a miter all of it).
        assert!(
            out.image.pixel(1, 1)[3] >= 100,
            "closing corner alpha {}",
            out.image.pixel(1, 1)[3]
        );
        assert_eq!(out.image.pixel(5, 1)[3], 255);
        assert_eq!(out.image.pixel(5, 5)[3], 0);
    }

    #[test]
    fn fill_polygons_antialiases_a_half_pixel_diagonal_edge() {
        // Right triangle (0,0)-(10,0)-(0,10): the hypotenuse x + y = 10
        // crosses the ten diagonal pixels (9,0)...(0,9) mid-pixel, so each
        // must land strictly between empty and solid (partial coverage proves
        // the anti-aliasing; the analytic integrator puts each at exactly
        // 128), and the summed coverage must track the analytic area 50
        // within 1.5% — only u8 quantization noise remains.
        let mut image = Bitmap::new(10, 10);
        let poly = vec![
            CGPoint::new(0.0, 0.0),
            CGPoint::new(10.0, 0.0),
            CGPoint::new(0.0, 10.0),
        ];
        fill_polygons(&mut image, &[poly], PaletteColor::new(1.0, 1.0, 1.0));
        let mut total = 0u64;
        for y in 0..10 {
            for x in 0..10 {
                let alpha = image.pixel(x, y)[3];
                total += alpha as u64;
                if x + y == 9 {
                    assert!(
                        alpha > 0 && alpha < 255,
                        "diagonal pixel ({x}, {y}) alpha {alpha} shows no anti-aliasing"
                    );
                }
            }
        }
        assert_eq!(image.pixel(2, 2)[3], 255, "interior pixel must stay solid");
        assert_eq!(image.pixel(9, 9)[3], 0, "outside pixel must stay empty");
        let covered = total as f64 / 255.0;
        let area = 10.0 * 10.0 / 2.0;
        let error = (covered - area).abs() / area;
        assert!(
            error < 0.015,
            "total coverage {covered} vs analytic area {area} off by {:.1}%",
            error * 100.0
        );
    }

    #[test]
    fn fill_polygons_matches_the_analytic_triangle_area() {
        // Right triangle (0,0)-(10,0)-(0,10) on a 10×10 mask: the analytic
        // integrator must keep every hypotenuse pixel strictly inside
        // (0, 255) — each diagonal pixel is exactly half covered, so 128 —
        // keep the interior solid, and sum to within 1.5% of the analytic
        // area 50 (45 solid pixels + 10 halves → 50.02, quantization only).
        let mut image = Bitmap::new(10, 10);
        let poly = vec![
            CGPoint::new(0.0, 0.0),
            CGPoint::new(10.0, 0.0),
            CGPoint::new(0.0, 10.0),
        ];
        fill_polygons(&mut image, &[poly], PaletteColor::new(1.0, 1.0, 1.0));
        let mut total = 0u64;
        for y in 0..10 {
            for x in 0..10 {
                let alpha = image.pixel(x, y)[3];
                total += alpha as u64;
                if x + y == 9 {
                    assert!(
                        alpha > 0 && alpha < 255,
                        "diagonal pixel ({x}, {y}) alpha {alpha} shows no anti-aliasing"
                    );
                }
            }
        }
        assert_eq!(image.pixel(2, 2)[3], 255, "interior pixel must stay solid");
        assert_eq!(image.pixel(9, 9)[3], 0, "outside pixel must stay empty");
        assert_eq!(
            image.pixel(9, 0)[3],
            128,
            "half-covered pixel must be exactly half"
        );
        let covered = total as f64 / 255.0;
        let area = 10.0 * 10.0 / 2.0;
        let error = (covered - area).abs() / area;
        assert!(
            error < 0.015,
            "total coverage {covered} vs analytic area {area} off by {:.1}%",
            error * 100.0
        );
    }
}
