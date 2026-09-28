//! Port of `Compositor/Document/Selection.swift`, `SelectionEdits.swift`,
//! `FloatingSelection.swift` and the selection outline model.
//!
//! Swift represents an outline as a `CGPath` with boolean operations. Here an
//! outline is a set of closed rings (polygons with holes), and the boolean
//! operations run on `geo`'s robust polygon engine. Coverage rasterization is
//! an analytic area fill: each pixel stores the exact area of its square
//! where the nonzero winding number is inside — the same integral Core
//! Graphics evaluates when it antialiases `fillPath(using: .winding)`.
//! Feathering is a Gaussian blur with σ = feather / 2 over edge-clamped
//! samples (`CIImage.clampedToExtent`, not zero padding), exactly as the
//! Swift `DocumentSelection.coverage` does.

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
    /// Outer rings and hole rings; coverage follows the nonzero winding
    /// rule (see `rasterize`/`winding_contains`) — not even-odd.
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
        // CGPath.contains uses the winding rule: crossings count with sign
        // (down +1, up -1) and any nonzero total is inside.
        winding_contains(&self.rings, point.x, point.y)
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
        // Every ring contributes its band and the pieces union — a tiny ring
        // whose inner offset degenerates is covered outright, so contracting
        // still removes it.
        let d = width / 2.0;
        let mut band: Option<SelectionPath> = None;
        for ring in &self.rings {
            if ring.len() < 3 {
                continue;
            }
            let outward = offset_ring(ring, d, true);
            let inward = offset_ring(ring, d, false);
            let degenerate = inward.len() >= 3
                && signed_area(&inward).abs() >= signed_area(ring).abs();
            let piece = if outward.len() < 3 {
                continue;
            } else if inward.len() < 3 || degenerate {
                // The ring is thinner than the band, or its inner offset
                // flipped inside out on a tiny ring: band all the way across.
                let mut outward = outward;
                SelectionPath::from_polygon(outward)
            } else {
                let outer = SelectionPath::from_polygon(outward);
                let inner = SelectionPath::from_polygon(inward);
                outer.subtract(&inner)
            };
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
            let steps = ((delta.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).max(2).min(64);
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

fn line_to_points(line: &LineString<f64>) -> Vec<CGPoint> {
    let mut pts: Vec<CGPoint> = line
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
    // Group rings into a containment forest: each ring's parent is the
    // *smallest-area* ring that contains it — never merely the first
    // enclosing outline seen, which mispairs three-level nesting (an island
    // inside a hole became a second hole and inverted the selection). Even
    // nesting depth is a polygon's exterior, odd depth its hole, and islands
    // inside holes are polygons of their own — the same arbitrary nesting
    // CGPath's winding fill handles natively.
    let valid: Vec<&Vec<CGPoint>> = rings.iter().filter(|ring| ring.len() >= 3).collect();
    let count = valid.len();
    let areas: Vec<f64> = valid.iter().map(|ring| signed_area(ring).abs()).collect();
    let bounds: Vec<RingBounds> = valid.iter().map(|ring| ring_bounds(ring)).collect();
    let mut parent: Vec<Option<usize>> = vec![None; count];
    for (j, ring) in valid.iter().enumerate() {
        let mut best: Option<usize> = None;
        for (i, container) in valid.iter().enumerate() {
            if i == j || areas[i] <= areas[j] || !bounds[i].contains_rect(&bounds[j]) {
                // A container strictly encloses its child, so its area — and
                // its bounding box — must be the larger one.
                continue;
            }
            if ring_contains(container, ring) {
                best = Some(match best {
                    Some(k) if areas[k] <= areas[i] => k,
                    _ => i,
                });
            }
        }
        parent[j] = best;
    }
    let depth = |start: usize| -> usize {
        let mut steps = 0;
        let mut j = start;
        while let Some(ancestor) = parent[j] {
            steps += 1;
            j = ancestor;
        }
        steps
    };
    let to_line = |ring: &Vec<CGPoint>| -> LineString<f64> {
        let mut coords: Vec<Coord<f64>> = ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
        if let (Some(first), Some(last)) = (coords.first().copied(), coords.last().copied()) {
            if (first.x - last.x).abs() > 1e-9 || (first.y - last.y).abs() > 1e-9 {
                coords.push(first);
            }
        }
        LineString::from(coords)
    };
    let mut polys: Vec<GeoPolygon<f64>> = Vec::new();
    for (j, ring) in valid.iter().enumerate() {
        if depth(j) % 2 != 0 {
            continue; // A hole; it is already its parent's interior.
        }
        let interiors: Vec<LineString<f64>> = (0..count)
            .filter(|k| parent[*k] == Some(j))
            .map(|k| to_line(valid[k]))
            .collect();
        polys.push(GeoPolygon::new(to_line(ring), interiors));
    }
    MultiPolygon::new(polys)
}

/// Whether `ring` lies within `container`: every vertex inside or on the
/// container's boundary, and at least one strictly inside. Outlines produced
/// by tracing or the boolean engine never cross, so rings are disjoint or
/// nested and a vertex test settles containment; requiring every vertex
/// inside keeps a ring that merely shares a corner (diagonal pixels trace
/// rings that touch at one vertex) from counting as contained.
fn ring_contains(container: &[CGPoint], ring: &[CGPoint]) -> bool {
    let mut strictly_inside = false;
    for p in ring {
        if ring_winding(container, p.x, p.y) != 0 {
            strictly_inside = true;
        } else if !point_on_ring(container, *p) {
            return false;
        }
    }
    strictly_inside
}

/// Whether `p` lies on one of the ring's edges (to floating tolerance).
fn point_on_ring(ring: &[CGPoint], p: CGPoint) -> bool {
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        if point_on_segment(ring[j], ring[i], p) {
            return true;
        }
        j = i;
    }
    false
}

fn point_on_segment(a: CGPoint, b: CGPoint, p: CGPoint) -> bool {
    let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    if cross.abs() > 1e-9 {
        return false;
    }
    let along = (p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y);
    if along < -1e-9 {
        return false;
    }
    let length2 = (b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y);
    along <= length2 + 1e-9
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
            // A feathered edge fades either side of the outline, as Photoshop's
            // does. `gaussian_blur_gray` takes the standard deviation — σ =
            // feather / 2, the same parameter Swift passes to
            // `applyingGaussianBlur(sigma:)` — and clamps samples at the mask
            // border, which is exactly what `CIImage.clampedToExtent` does
            // (the blur never reads zeros past the edge), so a selection that
            // runs off the rasterized area keeps full coverage to the border.
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

    /// The coverage value (0–1) at a document *point* (pixel centers included),
    /// 0 outside the region. Brush edits sample through their layer's
    /// pixel→document transform, so the clip lands correctly on scaled layers.
    pub fn alpha_at_document(&self, x: f64, y: f64) -> f32 {
        match &self.coverage {
            None => 0.0,
            Some(mask) => {
                let lx = x - self.rect.min_x();
                let ly = y - self.rect.min_y();
                if lx < 0.0 || ly < 0.0 || lx >= mask.width as f64 || ly >= mask.height as f64 {
                    0.0
                } else {
                    mask.value(lx as usize, ly as usize) as f32 / 255.0
                }
            }
        }
    }
}

/// Rasterization of an outline into a `width` × `height` mask. Winding fill,
/// matching `.winding` fill (the boolean engine never emits overlapping rings;
/// same-winding overlaps stay inside, as the tests pin down).
///
/// Antialiased mode is an *analytic area fill*, not a sampled approximation:
/// each pixel stores the exact area of its square where the winding number is
/// nonzero — the same per-pixel integral Core Graphics evaluates when it
/// antialiases `fillPath(using: .winding)`. An edge pixel that is 7/8 inside
/// therefore stores 223, where a 4×4 supersample grid (whose samples all land
/// on the inside) would round up to a hard 255.
pub fn rasterize(path: &SelectionPath, width: usize, height: usize, antialiased: bool) -> GrayMask {
    let mut mask = GrayMask::new(width.max(1), height.max(1));
    if path.rings.is_empty() || width == 0 || height == 0 {
        return mask;
    }
    if antialiased {
        rasterize_antialiased(&path.rings, &mut mask);
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
    winding_contains(rings, px, py)
}

/// One outline edge clipped to a scanline row. Its x is linear in y
/// (`x_ref` is x at the row top; `slope` is dx/dy), and `dir` is the winding
/// sign the `.winding` rule assigns (downward edges +1, upward −1) — the same
/// convention `winding_contains` applies at a point.
struct RowEdge {
    x_ref: f64,
    slope: f64,
    dir: i32,
    /// The edge's y extent within the row.
    y_lo: f64,
    y_hi: f64,
}

/// Analytic area-coverage fill of `rings` (nonzero winding rule) into `mask`.
///
/// Per scanline row, the row is split at every vertex and every pair of edge
/// crossings, so each y-band has a constant active edge set in a constant
/// left-to-right order; the inside region is then a disjoint set of trapezoids
/// bounded by linear edges. Each trapezoid's exact overlap with every pixel
/// column it touches is integrated in closed form and accumulated, so the
/// accumulated value per pixel *is* its covered area (≤ 1), rounded once to
/// 0–255 at the end of the row.
fn rasterize_antialiased(rings: &[Vec<CGPoint>], mask: &mut GrayMask) {
    let width = mask.width;
    let height = mask.height;
    let mut acc = vec![0.0f64; width];
    for row in 0..height {
        let y0 = row as f64;
        let y1 = y0 + 1.0;
        let mut edges: Vec<RowEdge> = Vec::new();
        for ring in rings {
            let n = ring.len();
            if n < 3 {
                continue;
            }
            let mut j = n - 1;
            for i in 0..n {
                let a = ring[j];
                let b = ring[i];
                // Same crossing convention as `winding_contains`: horizontal
                // edges never cross the scanline.
                if a.y != b.y {
                    let (e_lo, e_hi) = if a.y < b.y { (a.y, b.y) } else { (b.y, a.y) };
                    if e_hi > y0 && e_lo < y1 {
                        let slope = (b.x - a.x) / (b.y - a.y);
                        edges.push(RowEdge {
                            x_ref: a.x + (y0 - a.y) * slope,
                            slope,
                            dir: if a.y < b.y { 1 } else { -1 },
                            y_lo: e_lo.max(y0),
                            y_hi: e_hi.min(y1),
                        });
                    }
                }
                j = i;
            }
        }
        if !edges.is_empty() {
            fill_row_coverage(&edges, y0, &mut acc);
        }
        let out = &mut mask.data[row * width..row * width + width];
        for (x, slot) in out.iter_mut().enumerate() {
            let v = (acc[x].min(1.0) * 255.0).round() as u8;
            *slot = v;
        }
        acc.fill(0.0);
    }
}

/// Accumulates the covered area of every pixel column across one scanline row.
fn fill_row_coverage(edges: &[RowEdge], y0: f64, acc: &mut [f64]) {
    let y1 = y0 + 1.0;
    // Vertex events: split where an edge starts or ends inside the row, so
    // every band has a fixed active edge set.
    let mut events = vec![y0, y1];
    for e in edges {
        if e.y_lo > y0 && e.y_lo < y1 {
            events.push(e.y_lo);
        }
        if e.y_hi > y0 && e.y_hi < y1 {
            events.push(e.y_hi);
        }
    }
    sort_dedup(&mut events);
    for pair in events.windows(2) {
        let (ya, yb) = (pair[0], pair[1]);
        if yb - ya < 1e-12 {
            continue;
        }
        let active: Vec<&RowEdge> = edges
            .iter()
            .filter(|e| e.y_lo <= ya + 1e-9 && e.y_hi >= yb - 1e-9)
            .collect();
        if active.len() < 2 {
            continue;
        }
        // Two edges' crossings swap order where their x(y) meet; split there
        // too, so each sub-band's inside spans are clean trapezoids.
        let mut cuts = vec![ya, yb];
        for (i, e1) in active.iter().enumerate() {
            for e2 in &active[i + 1..] {
                let ds = e1.slope - e2.slope;
                if ds.abs() > 1e-12 {
                    // x_ref is x at the row top y0, not at ya.
                    let y_cut = y0 + (e2.x_ref - e1.x_ref) / ds;
                    if y_cut > ya + 1e-12 && y_cut < yb - 1e-12 {
                        cuts.push(y_cut);
                    }
                }
            }
        }
        sort_dedup(&mut cuts);
        for pair in cuts.windows(2) {
            let (ca, cb) = (pair[0], pair[1]);
            if cb - ca < 1e-12 {
                continue;
            }
            let ym = 0.5 * (ca + cb);
            let mut crossings: Vec<(f64, i32, f64)> = active
                .iter()
                .map(|e| (e.x_ref + e.slope * (ym - y0), e.dir, e.slope))
                .collect();
            // total_cmp: ring coordinates are upstream-guarded against NaN, but a
            // stray NaN must not panic the sort.
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            // Sweep right to left: the span between crossings i-1 and i is
            // inside when the winding number to its right is nonzero — the
            // same rule `winding_contains` applies at a point (a closed ring's
            // crossings sum to zero, so everything left of the first crossing
            // is outside).
            let mut suffix = 0i32;
            for i in (1..crossings.len()).rev() {
                suffix += crossings[i].1;
                if suffix == 0 {
                    continue;
                }
                let (xl, slope_l) = (crossings[i - 1].0, crossings[i - 1].2);
                let (xr, slope_r) = (crossings[i].0, crossings[i].2);
                let xl_ca = xl + slope_l * (ca - ym);
                let xr_ca = xr + slope_r * (ca - ym);
                let xl_cb = xl + slope_l * (cb - ym);
                let xr_cb = xr + slope_r * (cb - ym);
                let x_min = xl_ca.min(xl_cb).min(xr_ca).min(xr_cb).floor();
                let x_max = xl_ca.max(xl_cb).max(xr_ca).max(xr_cb).ceil();
                let first = (x_min.max(0.0) as usize).min(acc.len());
                let last = (x_max.max(0.0) as usize).min(acc.len());
                if first >= last {
                    continue;
                }
                for (col, slot) in acc[first..last].iter_mut().enumerate() {
                    let cx = (first + col) as f64;
                    *slot += span_column_overlap(
                        xl_ca, slope_l, xr_ca, slope_r, cx, cx + 1.0, ca, cb,
                    );
                }
            }
        }
    }
}

/// Exact ∫ over y ∈ [ya, yb] of the overlap between the pixel column
/// [col_min, col_max] and the span between two linear edges x = xl(y),
/// x = xr(y) (given as value at ya plus slope). The overlap is piecewise
/// linear in y and every corner — either edge meeting either column side —
/// is a breakpoint, so the trapezoid rule over those breakpoints integrates
/// it exactly.
fn span_column_overlap(
    xl0: f64,
    sxl: f64,
    xr0: f64,
    sxr: f64,
    col_min: f64,
    col_max: f64,
    ya: f64,
    yb: f64,
) -> f64 {
    let xl_yb = xl0 + sxl * (yb - ya);
    let xr_yb = xr0 + sxr * (yb - ya);
    // Cheap exact reject: the span stays entirely on one side of the column
    // across the whole band (a diagonal span that merely crosses into the
    // column part of the time must still be integrated).
    if xr0.max(xr_yb) <= col_min || xl0.min(xl_yb) >= col_max {
        return 0.0;
    }
    let overlap = |y: f64| -> f64 {
        let xl = xl0 + sxl * (y - ya);
        let xr = xr0 + sxr * (y - ya);
        (xr.min(col_max) - xl.max(col_min)).max(0.0)
    };
    let mut cuts = vec![ya, yb];
    for (x0, s) in [(xl0, sxl), (xr0, sxr)] {
        if s.abs() > 1e-15 {
            for bound in [col_min, col_max] {
                let t = ya + (bound - x0) / s;
                if t > ya + 1e-12 && t < yb - 1e-12 {
                    cuts.push(t);
                }
            }
        }
    }
    cuts.sort_by(|a, b| a.total_cmp(b));
    let mut area = 0.0;
    for pair in cuts.windows(2) {
        area += 0.5 * (overlap(pair[0]) + overlap(pair[1])) * (pair[1] - pair[0]);
    }
    area
}

/// Sorts ascending and merges near-duplicates (vertex ys computed from
/// different edges can differ by rounding noise).
fn sort_dedup(values: &mut Vec<f64>) {
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup_by(|a, b| *a - *b < 1e-9);
}

/// The nonzero winding rule (`fillPath(using: .winding)` /
/// `path.contains(point, using: .winding)`): each edge crossing the ray adds
/// +1 when it points down and -1 when it points up; nonzero is inside.
fn winding_contains(rings: &[Vec<CGPoint>], px: f64, py: f64) -> bool {
    let mut winding = 0i32;
    for ring in rings {
        winding += ring_winding(ring, px, py);
    }
    winding != 0
}

/// One ring's winding-number contribution at a point: +1 per crossing of an
/// edge pointing down, -1 for up.
fn ring_winding(ring: &[CGPoint], px: f64, py: f64) -> i32 {
    let n = ring.len();
    if n < 3 {
        return 0;
    }
    let mut winding = 0i32;
    let mut j = n - 1;
    for i in 0..n {
        let a = ring[j];
        let b = ring[i];
        if (a.y > py) != (b.y > py) {
            let x_at = (a.x - b.x) * (py - b.y) / (a.y - b.y) + b.x;
            if px < x_at {
                winding += if a.y < b.y { 1 } else { -1 };
            }
        }
        j = i;
    }
    winding
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
    fn rasterize_exact_fractional_edge_coverage() {
        // An edge at x = 2.125 leaves exactly 7/8 of the pixel covered; the
        // analytic fill stores round(0.875 * 255) = 223 where the old 4×4
        // sample grid (all its samples land right of 2.125) rounded up to a
        // hard 255. Same for the 1/8 sliver on the far side.
        let p = rect_path(2.125, 2.0, 4.0, 4.0);
        let mask = rasterize(&p, 8, 8, true);
        assert_eq!(mask.value(4, 3), 255);
        assert_eq!(mask.value(2, 3), 223);
        assert_eq!(mask.value(6, 3), 32);
        assert_eq!(mask.value(1, 3), 0);
        assert_eq!(mask.value(7, 3), 0);
    }

    #[test]
    fn rasterize_exact_diagonal_coverage() {
        // Right triangle with hypotenuse x + y = 2.25: pixel areas under a
        // diagonal are integrated exactly, not sampled. (0.71875 → 183 and
        // the corner sliver 0.03125 → 8; a 4×4 grid gives 175 and 0.)
        let p = SelectionPath::from_polygon(vec![
            CGPoint::new(0.0, 0.0),
            CGPoint::new(2.25, 0.0),
            CGPoint::new(0.0, 2.25),
        ]);
        let mask = rasterize(&p, 2, 2, true);
        assert_eq!(mask.value(0, 0), 255);
        assert_eq!(mask.value(1, 0), 183);
        assert_eq!(mask.value(0, 1), 183);
        assert_eq!(mask.value(1, 1), 8);
    }

    #[test]
    fn rasterize_winding_overlap_and_hole_exact() {
        // The winding rule through the analytic fill: nested same-winding
        // rings stay inside (winding 2 ≠ 0) right through the inner ring, and
        // the reversed ring is a hole with exactly zero coverage.
        let square = |x0: f64, y0: f64, x1: f64, y1: f64, reverse: bool| {
            let mut ring = vec![
                CGPoint::new(x0, y0),
                CGPoint::new(x1, y0),
                CGPoint::new(x1, y1),
                CGPoint::new(x0, y1),
            ];
            if reverse {
                ring.reverse();
            }
            ring
        };
        let overlap = SelectionPath {
            rings: vec![
                square(0.0, 0.0, 16.0, 16.0, false),
                square(6.0, 6.0, 10.0, 10.0, false),
            ],
        };
        let mask = rasterize(&overlap, 16, 16, true);
        assert_eq!(mask.value(3, 8), 255);
        assert_eq!(mask.value(6, 8), 255); // even-odd would hollow this out
        let hole = SelectionPath {
            rings: vec![
                square(0.0, 0.0, 16.0, 16.0, false),
                square(6.0, 6.0, 10.0, 10.0, true),
            ],
        };
        let mask = rasterize(&hole, 16, 16, true);
        assert_eq!(mask.value(3, 8), 255);
        assert_eq!(mask.value(5, 8), 255); // inside the outer ring, outside the hole
        assert_eq!(mask.value(6, 8), 0);
        assert_eq!(mask.value(8, 8), 0);
    }

    #[test]
    fn feathered_coverage_clamps_at_mask_edges() {
        // clampedToExtent: a selection wider than the rasterized area stays at
        // full coverage everywhere after the blur — zero padding would dim the
        // borders toward the outside (a corner would lose about half the
        // kernel's mass per axis).
        let sel = DocumentSelection {
            path: rect_path(-8.0, -8.0, 32.0, 32.0),
            antialiased: true,
            feather: 4.0,
        };
        let mask = sel.coverage(16, 16);
        assert!(mask.data.iter().all(|v| *v == 255), "a clamped blur keeps a full mask full");
    }

    #[test]
    fn feather_falloff_matches_gaussian_sigma() {
        // σ = feather / 2: with feather 2 (σ = 1) a vertical outline at x = 4
        // blurs to ≈ 255·Φ(±0.5) on the flank pixels — 77 outside, 178 inside
        // (the continuous step response). A kernel sized by radius instead of
        // σ would flatten these toward 128.
        let sel = DocumentSelection {
            path: rect_path(4.0, 0.0, 10.0, 8.0),
            antialiased: true,
            feather: 2.0,
        };
        let mask = sel.coverage(12, 8);
        assert!(mask.value(6, 4) >= 248, "interior settles: {}", mask.value(6, 4));
        let left = mask.value(3, 4);
        let right = mask.value(4, 4);
        assert!(left > 55 && left < 100, "left={left}");
        assert!(right > 155 && right < 200, "right={right}");
        assert!(mask.value(2, 4) < 30, "outer flank decays: {}", mask.value(2, 4));
        assert!(mask.value(5, 4) > 220, "inner flank recovers: {}", mask.value(5, 4));
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
    fn same_winding_nested_rings_stay_inside() {
        // The winding rule: a second ring winding the *same* way doubles the
        // winding number — the area stays inside (even-odd would punch a
        // hole). A ring winding the opposite way is a hole.
        let square = |x0: f64, y0: f64, x1: f64, y1: f64, reverse: bool| {
            let mut ring = vec![
                CGPoint::new(x0, y0),
                CGPoint::new(x1, y0),
                CGPoint::new(x1, y1),
                CGPoint::new(x0, y1),
            ];
            if reverse {
                ring.reverse();
            }
            ring
        };
        let path = SelectionPath {
            rings: vec![
                square(0.0, 0.0, 100.0, 100.0, false),
                square(20.0, 20.0, 80.0, 80.0, false), // same winding: still inside
            ],
        };
        assert!(path.contains(CGPoint::new(50.0, 50.0)));
        let path = SelectionPath {
            rings: vec![
                square(0.0, 0.0, 100.0, 100.0, false),
                square(20.0, 20.0, 80.0, 80.0, true), // opposite winding: hole
            ],
        };
        assert!(!path.contains(CGPoint::new(50.0, 50.0)));
    }

    /// A rectangular ring, `reverse` flipping it to the opposite winding.
    fn square_ring(x0: f64, y0: f64, x1: f64, y1: f64, reverse: bool) -> Vec<CGPoint> {
        let mut ring = vec![
            CGPoint::new(x0, y0),
            CGPoint::new(x1, y0),
            CGPoint::new(x1, y1),
            CGPoint::new(x0, y1),
        ];
        if reverse {
            ring.reverse();
        }
        ring
    }

    #[test]
    fn three_level_nesting_keeps_the_island_inside_the_hole() {
        // Outer CCW, middle CW (a hole), inner CCW: an island inside the
        // hole. The old grouping took the first enclosing bounding box, so
        // the island became a second hole of the outer and inverted the
        // selection; the smallest-container rule pairs it on its own.
        let path = SelectionPath {
            rings: vec![
                square_ring(0.0, 0.0, 40.0, 40.0, false),
                square_ring(10.0, 10.0, 30.0, 30.0, true),
                square_ring(16.0, 16.0, 24.0, 24.0, false),
            ],
        };
        let multi = path.to_geo();
        assert_eq!(multi.iter().count(), 2, "the island is a polygon of its own");
        let polygons: Vec<_> = multi.iter().collect();
        assert_eq!(polygons[0].interiors().len(), 1, "only the middle ring is the outer's hole");
        assert_eq!(polygons[1].interiors().len(), 0);
        // Round-tripped back through the rings, the winding fill shows outer
        // and island solid with the middle ring empty.
        let mask = rasterize(&SelectionPath::from_geo(&multi), 40, 40, false);
        assert_eq!(mask.value(5, 5), 255);
        assert_eq!(mask.value(12, 12), 0, "the middle ring is a hole");
        assert_eq!(mask.value(20, 20), 255, "the island survives");
        assert_eq!(mask.value(27, 27), 0);
    }

    #[test]
    fn sibling_holes_share_one_parent() {
        let path = SelectionPath {
            rings: vec![
                square_ring(0.0, 0.0, 40.0, 40.0, false),
                square_ring(4.0, 4.0, 12.0, 12.0, true),
                square_ring(28.0, 28.0, 36.0, 36.0, true),
            ],
        };
        let multi = path.to_geo();
        assert_eq!(multi.iter().count(), 1);
        let polygon = multi.iter().next().unwrap();
        assert_eq!(polygon.interiors().len(), 2);
        let mask = rasterize(&SelectionPath::from_geo(&multi), 40, 40, false);
        assert_eq!(mask.value(8, 8), 0);
        assert_eq!(mask.value(32, 32), 0);
        assert_eq!(mask.value(20, 20), 255);
    }

    #[test]
    fn hole_inside_island_nests_to_full_depth() {
        // Four levels: outer, hole, island in the hole, hole in the island.
        // Each ring must pair with the smallest container, not the first.
        let path = SelectionPath {
            rings: vec![
                square_ring(0.0, 0.0, 50.0, 50.0, false),
                square_ring(5.0, 5.0, 45.0, 45.0, true),
                square_ring(10.0, 10.0, 40.0, 40.0, false),
                square_ring(20.0, 20.0, 30.0, 30.0, true),
            ],
        };
        let multi = path.to_geo();
        let polygons: Vec<_> = multi.iter().collect();
        assert_eq!(polygons.len(), 2);
        assert_eq!(polygons[0].interiors().len(), 1);
        assert_eq!(polygons[1].interiors().len(), 1);
        let mask = rasterize(&SelectionPath::from_geo(&multi), 50, 50, false);
        assert_eq!(mask.value(2, 25), 255);
        assert_eq!(mask.value(7, 25), 0);
        assert_eq!(mask.value(15, 25), 255, "the island is solid again");
        assert_eq!(mask.value(25, 25), 0, "the island's own hole is empty");
    }

    #[test]
    fn island_in_hole_survives_subsequent_booleans() {
        // The real workflow that broke: a donut gains an island in its hole,
        // and a later edit regroups the combined path's rings. With the old
        // first-enclosing-bounds pick the island turned into a second hole
        // and the edit erased it.
        let donut = rect_path(0.0, 0.0, 40.0, 40.0).subtract(&rect_path(10.0, 10.0, 30.0, 30.0));
        let island = rect_path(15.0, 15.0, 5.0, 5.0);
        let combined = donut.union(&island);
        let clipped = combined.subtract(&rect_path(0.0, 0.0, 40.0, 4.0));
        let mask = rasterize(&clipped, 40, 40, false);
        assert_eq!(mask.value(2, 2), 0, "the subtracted strip is gone");
        assert_eq!(mask.value(2, 7), 255, "the rest of the rim stays");
        assert_eq!(mask.value(17, 17), 255, "the island survives the re-grouping");
        assert_eq!(mask.value(12, 12), 0);
    }

    #[test]
    fn rings_touching_at_a_corner_stay_disjoint() {
        // Two diagonal one-pixel squares trace rings sharing one vertex;
        // neither contains the other, so both are exteriors.
        let path = SelectionPath {
            rings: vec![
                square_ring(0.0, 0.0, 1.0, 1.0, false),
                square_ring(1.0, 1.0, 2.0, 2.0, false),
            ],
        };
        let multi = path.to_geo();
        assert_eq!(multi.iter().count(), 2);
        for polygon in multi.iter() {
            assert_eq!(polygon.interiors().len(), 0);
        }
        let mask = rasterize(&SelectionPath::from_geo(&multi), 2, 2, false);
        assert_eq!(mask.value(0, 0), 255);
        assert_eq!(mask.value(1, 1), 255);
        assert_eq!(mask.value(1, 0), 0);
        assert_eq!(mask.value(0, 1), 0);
    }

    #[test]
    fn ellipse_area_matches_the_box() {
        // The kappa-flattened oval covers close to the analytic ellipse area.
        let path = SelectionPath::ellipse(CGRect::new(0.0, 0.0, 100.0, 100.0));
        let mask = rasterize(&path, 100, 100, false);
        let covered: u64 = mask.data.iter().map(|v| u64::from(*v > 127)).sum();
        let analytic = (std::f64::consts::PI / 4.0 * 100.0 * 100.0).round() as u64;
        let drift = (covered as f64 - analytic as f64).abs() / analytic as f64;
        assert!(drift < 0.02, "covered={covered} analytic={analytic}");
        // A wide flat ellipse keeps its shape too.
        let wide = SelectionPath::ellipse(CGRect::new(0.0, 0.0, 100.0, 20.0));
        let mask = rasterize(&wide, 100, 20, false);
        let covered: u64 = mask.data.iter().map(|v| u64::from(*v > 127)).sum();
        let analytic = (std::f64::consts::PI / 4.0 * 100.0 * 20.0).round() as u64;
        let drift = (covered as f64 - analytic as f64).abs() / analytic as f64;
        assert!(drift < 0.03, "covered={covered} analytic={analytic}");
    }

    #[test]
    fn tiny_ring_band_covers_the_whole_ring() {
        // A 2×2 ring is thinner than any real band: stroking covers it
        // outright, so contracting removes it entirely.
        let tiny = SelectionPath::from_polygon(vec![
            CGPoint::new(10.0, 10.0),
            CGPoint::new(12.0, 10.0),
            CGPoint::new(12.0, 12.0),
            CGPoint::new(10.0, 12.0),
        ]);
        let band = tiny.stroke_band(6.0);
        assert!(band.contains(CGPoint::new(11.0, 11.0)), "the tiny ring bands");
    }

    #[test]
    fn falloff_matches_formula() {
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
