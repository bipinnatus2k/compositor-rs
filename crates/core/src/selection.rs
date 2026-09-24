//! Port of `Compositor/Document/Selection.swift`, `SelectionEdits.swift`,
//! `FloatingSelection.swift` and the selection outline model.
//!
//! Swift represents an outline as a `CGPath` with boolean operations. Here an
//! outline is a set of closed rings (polygons with holes), and the boolean
//! operations run on `geo`'s robust polygon engine. Coverage rasterization is
//! an antialiased scanline fill; feathering is a Gaussian blur with σ =
//! feather / 2, exactly as the Swift `DocumentSelection.coverage` does.

use crate::geometry::{CGFloat, CGAffineTransform, CGPoint, CGRect, CGSize};
use crate::raster::GrayMask;
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon as GeoPolygon};
use serde::{Deserialize, Serialize};

/// The Magic tool's modes: Wand selects pixels of a similar color, Object traces the
/// outline of whatever the click lands on. Tab switches between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WandMode {
    #[default]
    #[serde(rename = "Wand")]
    Wand,
    #[serde(rename = "Object")]
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LassoKind {
    #[default]
    #[serde(rename = "Freehand")]
    Freehand,
    #[serde(rename = "Polygonal")]
    Polygonal,
    /// The Marquee's outlines; not offered in the Lasso's Freehand/Polygonal choice.
    #[serde(rename = "Rectangle")]
    Rectangle,
    #[serde(rename = "Ellipse")]
    Ellipse,
}

impl LassoKind {
    pub const LASSO_CHOICES: [LassoKind; 2] = [LassoKind::Freehand, LassoKind::Polygonal];
    pub const MARQUEE_CHOICES: [LassoKind; 2] = [LassoKind::Rectangle, LassoKind::Ellipse];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SelectionMode {
    #[default]
    #[serde(rename = "New")]
    Replace,
    #[serde(rename = "Add")]
    Add,
    #[serde(rename = "Subtract")]
    Subtract,
}

/// A selection outline: closed rings in document pixels. Each ring's first
/// point is not repeated at the end.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SelectionPath {
    /// Outer rings and hole rings. Even-odd fill decides coverage.
    pub rings: Vec<Vec<CGPoint>>,
}

impl SelectionPath {
    pub fn from_polygon(points: Vec<CGPoint>) -> Self {
        SelectionPath { rings: vec![points] }
    }

    /// A rectangle outline.
    pub fn rect(rect: CGRect) -> Self {
        SelectionPath::from_polygon(vec![
            CGPoint::new(rect.min_x(), rect.min_y()),
            CGPoint::new(rect.max_x(), rect.min_y()),
            CGPoint::new(rect.max_x(), rect.max_y()),
            CGPoint::new(rect.min_x(), rect.max_y()),
        ])
    }

    /// An ellipse flattened into a polygon (CG adds Béziers; the flattened
    /// outline rasterizes identically at antialiasing resolution).
    pub fn ellipse(rect: CGRect) -> Self {
        const SEGMENTS: usize = 96;
        let cx = rect.mid_x();
        let cy = rect.mid_y();
        let rx = rect.width() / 2.0;
        let ry = rect.height() / 2.0;
        let mut points = Vec::with_capacity(SEGMENTS);
        for i in 0..SEGMENTS {
            let t = (i as f64) * std::f64::consts::TAU / SEGMENTS as f64;
            points.push(CGPoint::new(cx + rx * t.cos(), cy + ry * t.sin()));
        }
        SelectionPath::from_polygon(points)
    }

    pub fn from_geo(multi: &MultiPolygon<f64>) -> Self {
        let mut rings = Vec::new();
        for polygon in multi.iter() {
            rings.push(line_to_points(polygon.exterior()));
            for interior in polygon.interiors() {
                rings.push(line_to_points(interior));
            }
        }
        SelectionPath { rings }
    }

    pub fn to_geo(&self) -> MultiPolygon<f64> {
        polygons_from_rings(&self.rings)
    }

    pub fn is_empty(&self) -> bool {
        self.bounds().map(|b| b.is_empty() || b.is_null()).unwrap_or(true)
    }

    /// `boundingBoxOfPath`; None when there are no rings.
    pub fn bounds(&self) -> Option<CGRect> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for ring in &self.rings {
            for p in ring {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }
        if !min_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        Some(CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    pub fn contains(&self, point: CGPoint) -> bool {
        // Even-odd across all rings.
        let mut inside = false;
        for ring in &self.rings {
            let n = ring.len();
            if n < 3 {
                continue;
            }
            let mut j = n - 1;
            for i in 0..n {
                let a = ring[j];
                let b = ring[i];
                if ((b.y > point.y) != (a.y > point.y))
                    && (point.x < (a.x - b.x) * (point.y - b.y) / (a.y - b.y) + b.x)
                {
                    inside = !inside;
                }
                j = i;
            }
        }
        inside
    }

    pub fn union(&self, other: &SelectionPath) -> SelectionPath {
        SelectionPath::from_geo(&self.to_geo().union(&other.to_geo()))
    }

    pub fn intersection(&self, other: &SelectionPath) -> SelectionPath {
        SelectionPath::from_geo(&self.to_geo().intersection(&other.to_geo()))
    }

    pub fn subtract(&self, other: &SelectionPath) -> SelectionPath {
        SelectionPath::from_geo(&self.to_geo().difference(&other.to_geo()))
    }

    /// The band `width` wide around every edge (`path.copy(strokingWithWidth:)`
    /// with a round cap and join), for Expand and Contract. Each ring is
    /// buffered `width / 2` outward and `width / 2` inward; the band is the
    /// difference, unioned across rings.
    pub fn stroke_band(&self, width: CGFloat) -> SelectionPath {
        let d = width / 2.0;
        let mut band: Option<SelectionPath> = None;
        for ring in &self.rings {
            if ring.len() < 3 {
                continue;
            }
            let outward = offset_ring(ring, d, true);
            let inward = offset_ring(ring, d, false);
            if outward.len() < 3 || inward.len() < 3 {
                continue;
            }
            let outer = SelectionPath::from_polygon(outward);
            let inner = SelectionPath::from_polygon(inward);
            let piece = outer.subtract(&inner);
            band = Some(match band {
                None => piece,
                Some(existing) => existing.union(&piece),
            });
        }
        band.unwrap_or_default()
    }

    pub fn translated(&self, dx: CGFloat, dy: CGFloat) -> SelectionPath {
        SelectionPath {
            rings: self
                .rings
                .iter()
                .map(|ring| ring.iter().map(|p| CGPoint::new(p.x + dx, p.y + dy)).collect())
                .collect(),
        }
    }

    pub fn transformed(&self, t: &CGAffineTransform) -> SelectionPath {
        SelectionPath {
            rings: self
                .rings
                .iter()
                .map(|ring| ring.iter().map(|p| p.applying(t)).collect())
                .collect(),
        }
    }
}

/// Signed area (shoelace), positive when counterclockwise in y-down screen
/// coordinates — which is clockwise mathematically; only the sign matters here.
fn signed_area(ring: &[CGPoint]) -> f64 {
    let mut sum = 0.0;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        sum += a.x * b.y - b.x * a.y;
    }
    sum / 2.0
}

/// Offsets a closed ring by `d` pixels outward from (or inward into) the area
/// it encloses. Convex corners get round joins (as CGPath stroking does);
/// reflex corners join at the offset lines' intersection.
fn offset_ring(ring: &[CGPoint], d: f64, outward: bool) -> Vec<CGPoint> {
    let n = ring.len();
    if n < 3 || d <= 0.0 {
        return Vec::new();
    }
    let area = signed_area(ring);
    if area.abs() < 1e-12 {
        return Vec::new();
    }
    let ccw = area > 0.0;
    let sign: f64 = match (ccw, outward) {
        (true, true) | (false, false) => 1.0,
        _ => -1.0,
    };
    // Unit normal of edge p→q pointing to `sign` side: rotate direction 90°.
    let normal = |p: CGPoint, q: CGPoint| -> CGPoint {
        let dx = q.x - p.x;
        let dy = q.y - p.y;
        let len = dx.hypot(dy);
        if len < 1e-12 {
            CGPoint::new(0.0, 0.0)
        } else {
            CGPoint::new(sign * dy / len, -sign * dx / len)
        }
    };
    let mut result = Vec::with_capacity(n * 2);
    for i in 0..n {
        let prev = ring[(i + n - 1) % n];
        let cur = ring[i];
        let next = ring[(i + 1) % n];
        let n_prev = normal(prev, cur);
        let n_cur = normal(cur, next);
        let a = cur + CGPoint::new(n_prev.x * d, n_prev.y * d); // end of previous offset edge
        let b = cur + CGPoint::new(n_cur.x * d, n_cur.y * d); // start of next offset edge
        // Convexity: cross of (cur-prev) × (next-cur) relative to `sign`.
        let e1 = cur - prev;
        let e2 = next - cur;
        let cross = e1.x * e2.y - e1.y * e2.x;
        let convex = sign * cross > 0.0;
        if convex {
            // Round join: arc around `cur` from `a` to `b`, sweeping the short way.
            let a0 = (a.y - cur.y).atan2(a.x - cur.x);
            let a1 = (b.y - cur.y).atan2(b.x - cur.x);
            let mut delta = a1 - a0;
            while delta > std::f64::consts::PI {
                delta -= std::f64::consts::TAU;
            }
            while delta < -std::f64::consts::PI {
                delta += std::f64::consts::TAU;
            }
            let steps = ((delta.abs() / (std::f64::consts::PI / 8.0)).ceil() as usize).max(2).min(32);
            for s in 0..=steps {
                let t = a0 + delta * (s as f64 / steps as f64);
                result.push(CGPoint::new(cur.x + d * t.cos(), cur.y + d * t.sin()));
            }
        } else {
            // Reflex join: intersect the two offset lines.
            let dir_prev = CGPoint::new(-n_prev.y, n_prev.x); // along previous edge
            let dir_cur = CGPoint::new(-n_cur.y, n_cur.x);
            let denom = dir_prev.x * dir_cur.y - dir_prev.y * dir_cur.x;
            if denom.abs() < 1e-12 {
                result.push(a);
                result.push(b);
            } else {
                let t = ((b.x - a.x) * dir_cur.y - (b.y - a.y) * dir_cur.x) / denom;
                result.push(CGPoint::new(a.x + dir_prev.x * t, a.y + dir_prev.y * t));
            }
        }
    }
    result
}

fn line_to_points(line: &LineString<f64>) -> Vec<CGPoint> {    let mut pts: Vec<CGPoint> = line
        .coords()
        .map(|c| CGPoint::new(c.x, c.y))
        .collect();
    // Drop the closing duplicate point geo adds.
    if pts.len() > 1 {
        let first = pts[0];
        let last = *pts.last().unwrap();
        if (first.x - last.x).abs() < 1e-9 && (first.y - last.y).abs() < 1e-9 {
            pts.pop();
        }
    }
    pts
}

fn polygons_from_rings(rings: &[Vec<CGPoint>]) -> MultiPolygon<f64> {
    // Group rings into polygons: a ring inside another is its hole.
    let mut polys: Vec<GeoPolygon<f64>> = Vec::new();
    let to_line = |ring: &Vec<CGPoint>| -> LineString<f64> {
        let mut coords: Vec<Coord<f64>> = ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
        if let (Some(first), Some(last)) = (coords.first().copied(), coords.last().copied()) {
            if (first.x - last.x).abs() > 1e-9 || (first.y - last.y).abs() > 1e-9 {
                coords.push(first);
            }
        }
        LineString::from(coords)
    };
    for ring in rings {
        if ring.len() < 3 {
            continue;
        }
        let line = to_line(ring);
        let bounds = ring_bounds(ring);
        // Find an enclosing polygon already collected.
        let mut parent: Option<usize> = None;
        for (index, poly) in polys.iter().enumerate() {
            let outer = ring_bounds_poly(poly);
            if outer.contains_rect(&bounds) {
                parent = Some(index);
                break;
            }
        }
        match parent {
            Some(index) => polys[index].interiors_push(line),
            None => polys.push(GeoPolygon::new(line, vec![])),
        }
    }
    MultiPolygon::new(polys)
}

struct RingBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl RingBounds {
    fn contains_rect(&self, other: &RingBounds) -> bool {
        self.min_x <= other.min_x && self.min_y <= other.min_y && self.max_x >= other.max_x && self.max_y >= other.max_y
    }
}

fn ring_bounds(ring: &[CGPoint]) -> RingBounds {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in ring {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    RingBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

fn ring_bounds_poly(poly: &GeoPolygon<f64>) -> RingBounds {
    let pts: Vec<CGPoint> = poly
        .exterior()
        .coords()
        .map(|c| CGPoint::new(c.x, c.y))
        .collect();
    ring_bounds(&pts)
}

/// A document-space selection outline, clipped to the canvas. `None` on the
/// document means no selection; a selection whose path is empty is an explicit
/// empty selection, which later edits treat as "touch nothing".
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSelection {
    pub path: SelectionPath,
    pub antialiased: bool,
    /// How far the edge fades, in document pixels. 0 is a hard edge.
    pub feather: CGFloat,
}

impl Default for DocumentSelection {
    fn default() -> Self {
        DocumentSelection {
            path: SelectionPath::default(),
            antialiased: true,
            feather: 0.0,
        }
    }
}

impl DocumentSelection {
    pub fn new(path: SelectionPath) -> Self {
        DocumentSelection {
            path,
            antialiased: true,
            feather: 0.0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    /// Four Gaussian standard deviations retain the visible falloff outside the outline.
    pub fn coverage_bounds(&self) -> CGRect {
        // ceil(feather * 2), as in Selection.swift — ceil first would widen
        // non-integer feathers by an extra pixel.
        let inset = (self.feather * 2.0).ceil();
        self.path
            .bounds()
            .unwrap_or(CGRect::NULL)
            .inset_by(-inset, -inset)
    }

    /// Grayscale coverage at document resolution (white = selected), top-left origin.
    pub fn coverage(&self, width: usize, height: usize) -> GrayMask {
        let mut mask = rasterize(&self.path, width, height, self.antialiased || self.feather > 0.0);
        if self.feather > 0.0 {
            // A feathered edge fades either side of the outline, as Photoshop's does.
            mask = crate::raster::gaussian_blur_gray(&mask, self.feather / 2.0);
        }
        mask
    }

    /// Coverage for just the selected region of the canvas, ready to clip edits.
    pub fn clip(&self, canvas: CGSize) -> SelectionClip {
        let region = self
            .coverage_bounds()
            .inset_by(-1.0, -1.0)
            .integral()
            .intersection(&CGRect::from_origin_size(CGPoint::ZERO, canvas));
        if self.is_empty() || region.is_null() || region.width() < 1.0 || region.height() < 1.0 {
            return SelectionClip {
                rect: CGRect::ZERO,
                coverage: None,
            };
        }
        let local = DocumentSelection {
            path: self.path.translated(-region.min_x(), -region.min_y()),
            antialiased: self.antialiased,
            feather: self.feather,
        };
        let coverage = local.coverage(region.width() as usize, region.height() as usize);
        SelectionClip {
            rect: region,
            coverage: Some(coverage),
        }
    }
}

/// Selection coverage for one region of the document. Pixel edits multiply by
/// the coverage (soft edges blend partially); with no coverage (an empty
/// selection) it clips everything away.
#[derive(Clone, PartialEq)]
pub struct SelectionClip {
    pub rect: CGRect,
    pub coverage: Option<GrayMask>,
}

impl SelectionClip {
    /// The coverage value (0–1) at a document pixel, 0 outside the region.
    pub fn alpha_at(&self, x: usize, y: usize) -> f32 {
        match &self.coverage {
            None => 0.0,
            Some(mask) => {
                let lx = x as f64 - self.rect.min_x();
                let ly = y as f64 - self.rect.min_y();
                if lx < 0.0 || ly < 0.0 || lx >= mask.width as f64 || ly >= mask.height as f64 {
                    0.0
                } else {
                    mask.value(lx as usize, ly as usize) as f32 / 255.0
                }
            }
        }
    }
}

/// Antialiased scanline rasterization of an outline into a `width` × `height`
/// mask. Even-odd winding, matching `.winding` fill for the simple outlines
/// selections draw (the boolean engine never emits overlapping rings).
pub fn rasterize(path: &SelectionPath, width: usize, height: usize, antialiased: bool) -> GrayMask {
    let mut mask = GrayMask::new(width.max(1), height.max(1));
    if path.rings.is_empty() || width == 0 || height == 0 {
        return mask;
    }
    if antialiased {
        const SS: usize = 4; // 4×4 supersampling per pixel.
        let total = SS * SS;
        for y in 0..height {
            for x in 0..width {
                let mut hits = 0u32;
                for sy in 0..SS {
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64;
                    for sx in 0..SS {
                        let px = x as f64 + (sx as f64 + 0.5) / SS as f64;
                        if point_in_path(&path.rings, px, py) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    mask.data[y * width + x] = ((hits as u32 * 255 + total as u32 / 2) / total as u32) as u8;
                }
            }
        }
    } else {
        for y in 0..height {
            let py = y as f64 + 0.5;
            for x in 0..width {
                let px = x as f64 + 0.5;
                if point_in_path(&path.rings, px, py) {
                    mask.data[y * width + x] = 255;
                }
            }
        }
    }
    mask
}

fn point_in_path(rings: &[Vec<CGPoint>], px: f64, py: f64) -> bool {
    let mut inside = false;
    for ring in rings {
        let n = ring.len();
        if n < 3 {
            continue;
        }
        let mut j = n - 1;
        for i in 0..n {
            let a = ring[j];
            let b = ring[i];
            if ((b.y > py) != (a.y > py)) && (px < (a.x - b.x) * (py - b.y) / (a.y - b.y) + b.x) {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

/// `DragBox.rect(from:to:square:fromCenter:)` — the box a drag from `anchor` to
/// `point` spans, in whole pixels. Shared by the Marquee and the Shape tool.
pub fn drag_box_rect(anchor: CGPoint, point: CGPoint, square: bool, from_center: bool) -> CGRect {
    let mut dx = point.x.round() - anchor.x;
    let mut dy = point.y.round() - anchor.y;
    if square {
        let side = dx.abs().max(dy.abs());
        dx = if dx < 0.0 { -side } else { side };
        dy = if dy < 0.0 { -side } else { side };
    }
    if from_center {
        CGRect::new(
            anchor.x - dx.abs(),
            anchor.y - dy.abs(),
            dx.abs() * 2.0,
            dy.abs() * 2.0,
        )
    } else {
        CGRect::new(
            anchor.x.min(anchor.x + dx),
            anchor.y.min(anchor.y + dy),
            dx.abs(),
            dy.abs(),
        )
    }
}

/// A lasso outline being drawn, in document pixels. `cursor` is the polygonal
/// lasso's rubber-band end point.
#[derive(Clone, PartialEq)]
pub struct LassoDraft {
    pub points: Vec<CGPoint>,
    pub cursor: Option<CGPoint>,
    pub mode: SelectionMode,
    pub kind: LassoKind,
    /// The Rectangular Marquee's starting corner (or center), in whole pixels.
    pub anchor: Option<CGPoint>,
}

/// Cmd-T with a selection: the selected pixels float on a temporary layer,
/// edited with the normal transform handles, then merge back into their layer.
#[derive(Clone)]
pub struct FloatingTransform {
    pub source_id: uuid::Uuid,
    pub before: Box<crate::document::CanvasDocument>,
    pub before_active: Option<uuid::Uuid>,
    pub original: crate::transform::LayerTransform,
    pub pixel_size: CGSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path(x: f64, y: f64, w: f64, h: f64) -> SelectionPath {
        SelectionPath::rect(CGRect::new(x, y, w, h))
    }

    #[test]
    fn bounds_and_contains() {
        let p = rect_path(0.0, 0.0, 10.0, 10.0);
        assert_eq!(p.bounds(), Some(CGRect::new(0.0, 0.0, 10.0, 10.0)));
        assert!(p.contains(CGPoint::new(5.0, 5.0)));
        assert!(!p.contains(CGPoint::new(15.0, 5.0)));
    }

    #[test]
    fn union_and_subtract() {
        let a = rect_path(0.0, 0.0, 10.0, 10.0);
        let b = rect_path(5.0, 0.0, 10.0, 10.0);
        let u = a.union(&b);
        assert_eq!(u.bounds(), Some(CGRect::new(0.0, 0.0, 15.0, 10.0)));
        let d = a.subtract(&b);
        assert_eq!(d.bounds(), Some(CGRect::new(0.0, 0.0, 5.0, 10.0)));
        let i = a.intersection(&b);
        assert_eq!(i.bounds(), Some(CGRect::new(5.0, 0.0, 5.0, 10.0)));
    }

    #[test]
    fn subtract_can_leave_hole() {
        let a = rect_path(0.0, 0.0, 10.0, 10.0);
        let b = rect_path(3.0, 3.0, 4.0, 4.0);
        let d = a.subtract(&b);
        assert!(!d.is_empty());
        assert!(d.contains(CGPoint::new(1.0, 1.0)));
        assert!(!d.contains(CGPoint::new(5.0, 5.0)));
    }

    #[test]
    fn empty_from_degenerate() {
        let p = SelectionPath::from_polygon(vec![
            CGPoint::new(1.0, 1.0),
            CGPoint::new(1.0, 1.0),
            CGPoint::new(1.0, 1.0),
        ]);
        assert!(p.is_empty());
    }

    #[test]
    fn rasterize_rect_coverage() {
        let p = rect_path(2.0, 2.0, 4.0, 4.0);
        let mask = rasterize(&p, 10, 10, false);
        assert_eq!(mask.value(2, 2), 255);
        assert_eq!(mask.value(5, 5), 255);
        assert_eq!(mask.value(6, 6), 0);
        assert_eq!(mask.value(1, 1), 0);
    }

    #[test]
    fn rasterize_antialiased_edge_partial() {
        let p = rect_path(2.5, 2.5, 3.0, 3.0);
        let mask = rasterize(&p, 10, 10, true);
        let inside = mask.value(4, 4);
        assert_eq!(inside, 255);
        let edge = mask.value(2, 2);
        assert!(edge > 0 && edge < 255, "edge={edge}");
    }

    #[test]
    fn coverage_feather_softens() {
        let p = rect_path(2.0, 2.0, 8.0, 8.0);
        let sel = DocumentSelection {
            path: p,
            antialiased: true,
            feather: 2.0,
        };
        let mask = sel.coverage(16, 16);
        let center = mask.value(6, 6);
        assert_eq!(center, 255);
        // Edge pixel softer than a hard edge would be.
        let edge = mask.value(2, 6);
        assert!(edge > 0 && edge < 255, "edge={edge}");
    }

    #[test]
    fn clip_region_bounded_by_canvas() {
        let sel = DocumentSelection::new(rect_path(-5.0, -5.0, 20.0, 20.0));
        let clip = sel.clip(CGSize::new(10.0, 10.0));
        assert_eq!(clip.rect, CGRect::new(0.0, 0.0, 10.0, 10.0));
        assert!(clip.coverage.is_some());
    }

    #[test]
    fn empty_selection_clips_everything() {
        let sel = DocumentSelection::new(SelectionPath::default());
        let clip = sel.clip(CGSize::new(10.0, 10.0));
        assert!(clip.coverage.is_none());
        assert_eq!(clip.alpha_at(5, 5), 0.0);
    }

    #[test]
    fn expand_grows_outline() {
        let a = rect_path(4.0, 4.0, 2.0, 2.0);
        let band = a.stroke_band(2.0);
        let grown = a.union(&band);
        let b = grown.bounds().unwrap();
        assert!((b.min_x() - 3.0).abs() < 0.2 && (b.max_x() - 7.0).abs() < 0.2, "{b:?}");
    }

    #[test]
    fn contract_shrinks_outline() {
        let a = rect_path(2.0, 2.0, 10.0, 10.0);
        let band = a.stroke_band(2.0);
        let shrunk = a.subtract(&band);
        let b = shrunk.bounds().unwrap();
        assert!(b.min_x() >= 2.9, "{b:?}");
        assert!(b.max_x() <= 11.1, "{b:?}");
    }

    #[test]
    fn drag_box_square_and_center() {
        let anchor = CGPoint::new(10.0, 10.0);
        let rect = drag_box_rect(anchor, CGPoint::new(15.0, 12.3), false, false);
        assert_eq!(rect, CGRect::new(10.0, 10.0, 5.0, 2.0));
        let square = drag_box_rect(anchor, CGPoint::new(15.0, 12.0), true, false);
        assert_eq!(square, CGRect::new(10.0, 10.0, 5.0, 5.0));
        let center = drag_box_rect(anchor, CGPoint::new(13.0, 14.0), false, true);
        assert_eq!(center, CGRect::new(7.0, 6.0, 6.0, 8.0));
    }

    #[test]
    fn ellipse_contains_center_not_corners() {
        let e = SelectionPath::ellipse(CGRect::new(0.0, 0.0, 10.0, 10.0));
        assert!(e.contains(CGPoint::new(5.0, 5.0)));
        assert!(!e.contains(CGPoint::new(0.4, 0.4)));
    }

    #[test]
    fn translate_and_transform() {
        let p = rect_path(0.0, 0.0, 10.0, 10.0);
        let t = p.translated(5.0, 5.0);
        assert_eq!(t.bounds(), Some(CGRect::new(5.0, 5.0, 10.0, 10.0)));
        let m = CGAffineTransform::scale(2.0, 2.0);
        let s = p.transformed(&m);
        assert_eq!(s.bounds(), Some(CGRect::new(0.0, 0.0, 20.0, 20.0)));
    }
}
