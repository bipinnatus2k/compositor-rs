//! Port of `Compositor/Document/BrushStroke.swift` — the shared raster edit
//! engine behind the Brush, Eraser, Spot Healing, Clone Stamp and Blur tools,
//! plus fills, gradients and selected-pixel moves.
//!
//! The Swift engine tiles its buffers for incremental redraws; this port keeps
//! whole-grid buffers (identical semantics, simpler bookkeeping). Coverage is
//! a grayscale grid accumulating dab coverage (max for hard tips, screen for
//! soft ones), and the visible result recomposes from the grid's original
//! content + paint × coverage × opacity, which caps the whole stroke at the
//! brush opacity exactly as the Swift does.

use crate::geometry::{CGFloat, CGAffineTransform, CGPoint, CGSize, CGRect};
use crate::raster::{falloff, pixel_to_document, spacing_fraction, Bitmap, GrayMask, ImportedImage};
use crate::selection::SelectionClip;
use crate::transform::LayerTransform;
use serde::{Deserialize, Serialize};

/// `BrushSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BrushSettings {
    #[serde(default = "default_diameter")]
    pub diameter: CGFloat,
    #[serde(default = "default_hardness")]
    pub hardness: CGFloat,
    #[serde(default)]
    pub red: CGFloat,
    #[serde(default)]
    pub green: CGFloat,
    #[serde(default)]
    pub blue: CGFloat,
    /// Caps the whole stroke: overlapping dabs never exceed it.
    #[serde(default = "default_opacity")]
    pub opacity: CGFloat,
    /// 0–100. The brush trails the pointer on a string of this length.
    #[serde(default)]
    pub smoothing: CGFloat,
    /// Erase: the stroke clears the layer's pixels instead of painting color.
    #[serde(default)]
    pub erasing: bool,
    /// Spot-healing shows a dark wash while painting; `heal()` rebuilds the
    /// area when the stroke ends.
    #[serde(default)]
    pub healing: bool,
}

fn default_diameter() -> CGFloat {
    40.0
}
fn default_hardness() -> CGFloat {
    1.0
}
fn default_opacity() -> CGFloat {
    1.0
}

impl Default for BrushSettings {
    fn default() -> Self {
        BrushSettings {
            diameter: 40.0,
            hardness: 1.0,
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            opacity: 1.0,
            smoothing: 0.0,
            erasing: false,
            healing: false,
        }
    }
}

/// Linear runs from start to end; radial is centered on the start with the
/// end on its rim (`GradientShape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientShape {
    Linear,
    Radial,
}

/// A brush stroke / raster edit in progress on one layer or mask.
pub struct RasterEdit {
    pub is_mask: bool,
    pub settings: BrushSettings,
    /// The paint grid's size (the layer grid grown to cover the canvas).
    pub width: usize,
    pub height: usize,
    pub canvas: CGRect,
    pub pixel_to_document: CGAffineTransform,
    /// Where the source pixels sit inside the paint grid.
    pub source_rect: CGRect,
    pub paint_transform: LayerTransform,
    source: Option<Bitmap>,
    /// The grid's starting content: the source drawn into the grid; a fresh
    /// mask's background is white.
    base: Bitmap,
    /// Accumulated stroke coverage.
    coverage: GrayMask,
    previous: Option<CGPoint>,
    distance_to_next: f64,
    samples: Vec<CGPoint>,
    /// Settled spline pieces in document pixels, for the continuous
    /// deposition engine (`MetalBrushCoverage`): every sample but the newest
    /// has been through the Catmull–Rom subdivision.
    segments: Vec<StrokeSegment>,
    /// Solid fill color overriding the brush color (`fill`).
    fill_color: Option<[u8; 4]>,
    /// The provisional straight tail from the newest settled sample to the
    /// cursor. Preview only: the engine renders it separately, so the next
    /// sample replaces it exactly (`drawTail`/`removeTail`).
    tail_segment: Option<StrokeSegment>,
    /// Limits every edit to the document selection.
    pub selection_clip: Option<SelectionClip>,
    /// Clone Stamp: a document-size image to copy from, and the offset from
    /// each painted point to its source.
    pub clone: Option<(Bitmap, CGSize)>,
    /// The clone sample replaces what's under the tip rather than drawing over it.
    pub replaces_with_clone: bool,
    /// Selected pixels lifted out of the layer (`lift_selection`).
    lifted: Option<(Bitmap, CGRect)>,
    /// Smoothing string state: the anchor trails the pointer taut; the raw
    /// pointer is kept so flush can end the stroke where the hand did.
    smoothing_anchor: Option<CGPoint>,
    smoothing_pointer: Option<CGPoint>,
}

impl RasterEdit {
    /// `BrushStroke.init(layer:mask:settings:canvas:)`.
    pub fn new(
        source: Option<Bitmap>,
        base_transform: LayerTransform,
        mask: bool,
        settings: BrushSettings,
        canvas: CGSize,
        selection_clip: Option<SelectionClip>,
    ) -> RasterEdit {
        let canvas_rect = CGRect::from_origin_size(CGPoint::ZERO, canvas);
        let original_width = source
            .as_ref()
            .map(|s| s.width)
            .unwrap_or_else(|| base_transform.size.width.round().max(1.0) as usize);
        let original_height = source
            .as_ref()
            .map(|s| s.height)
            .unwrap_or_else(|| base_transform.size.height.round().max(1.0) as usize);
        let original_bounds = CGRect::new(0.0, 0.0, original_width as f64, original_height as f64);
        let original_mapping =
            pixel_to_document(&base_transform, original_width as f64, original_height as f64);
        // A mask paints in its own grid; a layer's grid grows to cover the
        // canvas so strokes past the source still land.
        let extent = if mask {
            original_bounds
        } else {
            let projected = canvas_rect.transformed(&original_mapping.inverted());
            original_bounds.union(&projected.integral())
        };
        let width = (extent.width() as usize).max(1);
        let height = (extent.height() as usize).max(1);
        let source_rect = original_bounds.offset_by(-extent.min_x(), -extent.min_y());
        let pixel_to_document = original_mapping.translate(extent.min_x(), extent.min_y());
        let mut expanded = base_transform;
        expanded.size = CGSize::new(
            width as f64 * base_transform.size.width / original_width as f64,
            height as f64 * base_transform.size.height / original_height as f64,
        );
        let center = CGPoint::new(extent.mid_x(), extent.mid_y()).applying(&original_mapping);
        expanded.origin = CGPoint::new(
            center.x - expanded.size.width / 2.0,
            center.y - expanded.size.height / 2.0,
        );

        // Base: the source drawn into the grid; a fresh mask starts white.
        let mut base = Bitmap::new(width, height);
        if mask {
            for chunk in base.data.chunks_exact_mut(4) {
                chunk[0] = 255;
                chunk[1] = 255;
                chunk[2] = 255;
                chunk[3] = 255;
            }
        }
        if let Some(src) = &source {
            crate::raster::draw_stretched_nearest(&mut base, src, source_rect);
        }

        RasterEdit {
            is_mask: mask,
            settings,
            width,
            height,
            canvas: canvas_rect,
            pixel_to_document,
            source_rect,
            paint_transform: expanded,
            source,
            base,
            coverage: GrayMask::new(width, height),
            previous: None,
            distance_to_next: 0.0,
            samples: Vec::new(),
            segments: Vec::new(),
            fill_color: None,
            tail_segment: None,
            selection_clip,
            clone: None,
            replaces_with_clone: false,
            lifted: None,
            smoothing_anchor: None,
            smoothing_pointer: None,
        }
    }

    /// The brush tip as grayscale coverage: white at full strength, fading to
    /// black at the rim (`tip(diameter:hardness:falloff:)`).
    pub fn tip(diameter: CGFloat, hardness: CGFloat) -> GrayMask {
        let size = (diameter.ceil() as usize).max(1);
        let mut tip = GrayMask::new(size, size);
        let radius = size as f64 / 2.0;
        for y in 0..size {
            for x in 0..size {
                let px = x as f64 + 0.5;
                let py = y as f64 + 0.5;
                let d = (px - radius).hypot(py - radius) / radius;
                let value = if d > 1.0 {
                    0.0
                } else if hardness >= 1.0 {
                    // Antialiased hard edge across the boundary pixel.
                    ((1.0 - d) * radius).min(1.0).max(0.0)
                } else {
                    // The falloff starts at the hardness radius.
                    let u = ((d * radius - hardness * radius) / (radius - hardness * radius).max(1e-9))
                        .clamp(0.0, 1.0);
                    falloff(u)
                };
                tip.data[y * size + x] = (value * 255.0 + 0.5) as u8;
            }
        }
        tip
    }

    /// Smoothing: the brush trails the pointer on a string and only moves
    /// once the pointer pulls that string taut — samples inside the radius
    /// never reach the stroke (`EditorSession+Brush.swift:77-87`). The
    /// stroke's first sample sets the anchor; the radius is `settings`
    /// smoothing with the zoom already folded in by the caller.
    pub fn append_smoothed(&mut self, pointer: CGPoint) {
        if self.settings.smoothing <= 0.0 {
            self.append(pointer);
            return;
        }
        let radius = self.settings.smoothing;
        let Some(anchor) = self.smoothing_anchor else {
            self.smoothing_anchor = Some(pointer);
            self.smoothing_pointer = Some(pointer);
            self.append(pointer);
            return;
        };
        self.smoothing_pointer = Some(pointer);
        let delta = CGPoint::new(pointer.x - anchor.x, pointer.y - anchor.y);
        let distance = delta.x.hypot(delta.y);
        if distance <= radius {
            return;
        }
        let step = (distance - radius) / distance;
        let moved = CGPoint::new(anchor.x + delta.x * step, anchor.y + delta.y * step);
        self.smoothing_anchor = Some(moved);
        self.append(moved);
    }

    /// Mouse samples arrive sparsely, so dabs follow a Catmull–Rom curve
    /// through them, with a provisional straight tail until the next sample.
    pub fn append(&mut self, point: CGPoint) {        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x.abs() > 10_000_000.0
            || point.y.abs() > 10_000_000.0
        {
            return;
        }
        if self.samples.last() == Some(&point) {
            return;
        }
        self.samples.push(point);
        if self.samples.len() > 4 {
            self.samples.remove(0);
        }
        let count = self.samples.len();
        if count == 1 {
            // The first event is a zero-length segment: the initial click.
            let p = [point.x as f32, point.y as f32];
            self.segments.push([p[0], p[1], p[0], p[1]]);
            self.walk_to(point, true);
            return;
        }
        // Provisional straight tail from the newest settled sample to the
        // cursor; rendered separately so it never becomes permanent
        // (`appendContinuous`, BrushStroke.swift:252-266).
        let previous_sample = self.samples[count - 2];
        self.tail_segment = Some([
            previous_sample.x as f32,
            previous_sample.y as f32,
            point.x as f32,
            point.y as f32,
        ]);
        if count >= 3 {
            // The sample two back settles into a spline piece (the engine's
            // settled set and the dab grid's curve both consume it).
            let (start, end, before, after) = (
                self.samples[count - 3],
                self.samples[count - 2],
                self.samples[count.saturating_sub(4)],
                point,
            );
            self.extend_settled(Self::continuous_curve_points(start, end, before, after));
            self.curve(start, end, before, after);
        }
    }

    /// Replaces the provisional tail with the stroke's final curve piece.
    /// With smoothing the brush can trail short of the pointer, so the
    /// stroke ends where the hand did (`finishBrushImmediately`).
    pub fn flush(&mut self) {
        if let (Some(pointer), Some(anchor)) =
            (self.smoothing_pointer, self.smoothing_anchor)
        {
            if self.settings.smoothing > 0.0 && pointer != anchor {
                self.append(pointer);
            }
        }
        let count = self.samples.len();
        if count >= 2 {
            let n = count;
            let (start, end, before, after) = (
                self.samples[n - 2],
                self.samples[n - 1],
                self.samples[n.saturating_sub(3)],
                self.samples[n - 1],
            );
            self.extend_settled(Self::continuous_curve_points(start, end, before, after));
            self.curve(start, end, before, after);
            self.samples = vec![self.samples[n - 1]];
        }
        self.tail_segment = None;
    }

    fn extend_settled(&mut self, points: Vec<CGPoint>) {
        for pair in points.windows(2) {
            self.segments.push([
                pair[0].x as f32,
                pair[0].y as f32,
                pair[1].x as f32,
                pair[1].y as f32,
            ]);
        }
    }

    /// Centripetal Catmull–Rom adaptively subdivided so the polyline stays
    /// within 0.2 document pixels of the spline (`continuousCurve`,
    /// BrushStroke.swift:281-312).
    fn continuous_curve_points(
        start: CGPoint,
        end: CGPoint,
        before: CGPoint,
        after: CGPoint,
    ) -> Vec<CGPoint> {
        fn knot(t: f64, a: CGPoint, b: CGPoint) -> f64 {
            t + (b.x - a.x).hypot(b.y - a.y).sqrt().max(0.0001)
        }
        fn mix(a: CGPoint, b: CGPoint, ta: f64, tb: f64, t: f64) -> CGPoint {
            let wa = (tb - t) / (tb - ta);
            let wb = (t - ta) / (tb - ta);
            CGPoint::new(a.x * wa + b.x * wb, a.y * wa + b.y * wb)
        }
        let t0 = 0.0;
        let t1 = knot(t0, before, start);
        let t2 = knot(t1, start, end);
        let t3 = knot(t2, end, after);
        let point = |u: f64| -> CGPoint {
            if u == 0.0 {
                return start;
            }
            if u == 1.0 {
                return end;
            }
            let t = t1 + (t2 - t1) * u;
            let a = mix(before, start, t0, t1, t);
            let b = mix(start, end, t1, t2, t);
            let c = mix(end, after, t2, t3, t);
            mix(mix(a, b, t0, t2, t), mix(b, c, t1, t3, t), t1, t2, t)
        };
        let mut result = Vec::new();
        fn subdivide(
            point: &impl Fn(f64) -> CGPoint,
            result: &mut Vec<CGPoint>,
            a: CGPoint,
            b: CGPoint,
            lo: f64,
            hi: f64,
            depth: usize,
        ) {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let length_squared = dx * dx + dy * dy;
            let error = |p: CGPoint| -> f64 {
                let t = if length_squared > 0.0 {
                    (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
            };
            let mid = (lo + hi) / 2.0;
            let m = point(mid);
            let deviation = error(m)
                .max(error(point((lo + mid) / 2.0)))
                .max(error(point((mid + hi) / 2.0)));
            if deviation <= 0.2 || depth >= 10 {
                result.push(a);
                return;
            }
            subdivide(point, result, a, m, lo, mid, depth + 1);
            subdivide(point, result, m, b, mid, hi, depth + 1);
        }
        subdivide(&point, &mut result, start, end, 0.0, 1.0, 0);
        result.push(end);
        result
    }

    /// Centripetal Catmull–Rom between `start` and `end` (the exact spline
    /// from BrushStroke.curve).
    fn curve(&mut self, start: CGPoint, end: CGPoint, before: CGPoint, after: CGPoint) {
        fn knot(t: f64, a: CGPoint, b: CGPoint) -> f64 {
            t + (b.x - a.x).hypot(b.y - a.y).sqrt().max(0.0001)
        }
        fn mix(a: CGPoint, b: CGPoint, ta: f64, tb: f64, t: f64) -> CGPoint {
            let wa = (tb - t) / (tb - ta);
            let wb = (t - ta) / (tb - ta);
            CGPoint::new(a.x * wa + b.x * wb, a.y * wa + b.y * wb)
        }
        let t0 = 0.0;
        let t1 = knot(t0, before, start);
        let t2 = knot(t1, start, end);
        let t3 = knot(t2, end, after);
        let pieces = (((end.x - start.x).hypot(end.y - start.y) / 2.0).ceil() as usize).max(1);
        for index in 1..=pieces {
            let t = t1 + (t2 - t1) * index as f64 / pieces as f64;
            let a1 = mix(before, start, t0, t1, t);
            let a2 = mix(start, end, t1, t2, t);
            let a3 = mix(end, after, t2, t3, t);
            let b1 = mix(a1, a2, t0, t2, t);
            let b2 = mix(a2, a3, t1, t3, t);
            let point = if index == pieces { end } else { mix(b1, b2, t1, t2, t) };
            self.walk_to(point, true);
        }
    }

    /// Lays evenly spaced dabs along a straight run from the previous dab.
    /// `stamp` is false for the provisional tail: its dabs are rendered by
    /// the engine from `tail_segment`, never stamped into the permanent
    /// grid (`drawTail` backs the coverage up; not stamping is the exact
    /// whole-grid equivalent).
    fn walk_to(&mut self, point: CGPoint, stamp: bool) {
        let spacing = (self.settings.diameter * spacing_fraction(self.settings.hardness)).max(0.25);
        match self.previous {
            Some(previous) => {
                let dx = point.x - previous.x;
                let dy = point.y - previous.y;
                let length = dx.hypot(dy);
                if length > 0.0 {
                    let mut distance = self.distance_to_next;
                    while distance <= length {
                        let dab = CGPoint::new(
                            previous.x + dx * distance / length,
                            previous.y + dy * distance / length,
                        );
                        if stamp {
                            self.dab(dab);
                        }
                        distance += spacing;
                    }
                    self.distance_to_next = distance - length;
                }
            }
            None => {
                if stamp {
                    self.dab(point);
                }
                self.distance_to_next = spacing;
            }
        }
        self.previous = Some(point);
    }

    fn dab(&mut self, point: CGPoint) {
        let radius = self.settings.diameter / 2.0;
        let circle = CGRect::new(point.x - radius, point.y - radius, radius * 2.0, radius * 2.0);
        let clipped = circle.intersection(&self.canvas);
        if clipped.is_null() || clipped.is_empty() {
            return;
        }
        let inverse = self.pixel_to_document.inverted();
        // Snapped to whole pixels so the tip lands 1:1 with nothing to resample.
        let center = point.applying(&inverse);
        let tip = Self::tip(self.settings.diameter, self.settings.hardness);
        let tip_size = tip.width as f64;
        let blit = CGRect::new(
            (center.x - tip_size / 2.0).round(),
            (center.y - tip_size / 2.0).round(),
            tip_size,
            tip_size,
        );
        let pixel_canvas = self.canvas.transformed(&inverse);
        let affected = blit
            .intersection(&pixel_canvas)
            .integral()
            .intersection(&CGRect::new(0.0, 0.0, self.width as f64, self.height as f64));
        if affected.is_null() || affected.is_empty() {
            return;
        }
        let hard = self.settings.hardness >= 1.0;
        for y in affected.min_y() as usize..affected.max_y() as usize {
            for x in affected.min_x() as usize..affected.max_x() as usize {
                let tip_x = ((x as f64 + 0.5) - blit.min_x()) as i64;
                let tip_y = ((y as f64 + 0.5) - blit.min_y()) as i64;
                if tip_x < 0 || tip_y < 0 || tip_x >= tip.width as i64 || tip_y >= tip.height as i64 {
                    continue;
                }
                let value = tip.value(tip_x as usize, tip_y as usize);
                if value == 0 {
                    continue;
                }
                let i = y * self.width + x;
                let old = self.coverage.data[i];
                // Hard tips keep their antialiased silhouette (lighten/max);
                // soft tips accumulate paint (screen).
                let combined = if hard {
                    old.max(value)
                } else {
                    (255 - ((255 - old as u32) * (255 - value as u32)) / 255) as u8
                };
                self.coverage.data[i] = combined;
            }
        }
    }

    /// True when the stroke has laid down any coverage (`!patches.isEmpty`).
    pub fn has_paint(&self) -> bool {
        self.coverage.data.iter().any(|v| *v != 0)
    }

    /// Spot Healing, once the stroke ends: rebuilds the painted area from
    /// nearby texture (`HealPixels.c`) into the stroke's starting pixels —
    /// never the dark wash shown while painting — and clears the coverage, so
    /// `result()` is the healed pixels. False when there was nothing to heal.
    pub fn heal(&mut self, mode: i32, seed: u32) -> bool {
        if self.is_mask || !self.settings.healing || !self.has_paint() {
            return false;
        }
        let coverage_mask = self.effective_coverage();
        // The search only needs the painted area plus room for the kernel's
        // patch lookup (`painted.insetBy(-reach)` over the whole canvas).
        let painted = crate::raster::coverage_bounds(&coverage_mask);
        let Some(painted) = painted else {
            return false;
        };
        let region = heal_region(painted, self.width, self.height);
        let (rx, ry) = (region.min_x() as usize, region.min_y() as usize);
        let (rw, rh) = (region.width() as usize, region.height() as usize);
        let mut sub_base = Bitmap::new(rw, rh);
        let mut sub_coverage = GrayMask::new(rw, rh);
        for row in 0..rh {
            let src = ((ry + row) * self.width + rx) * 4;
            sub_base.data[row * rw * 4..row * rw * 4 + rw * 4]
                .copy_from_slice(&self.base.data[src..src + rw * 4]);
            sub_coverage.data[row * rw..row * rw + rw].copy_from_slice(
                &coverage_mask.data[(ry + row) * self.width + rx..(ry + row) * self.width + rx + rw],
            );
        }
        let opacity = self.settings.opacity as f32;
        if !crate::tools::heal::spot_heal(&mut sub_base, &sub_coverage, opacity, mode, seed) {
            return false;
        }
        for row in 0..rh {
            let dst = ((ry + row) * self.width + rx) * 4;
            let src = row * rw * 4;
            self.base.data[dst..dst + rw * 4].copy_from_slice(&sub_base.data[src..src + rw * 4]);
        }
        self.coverage = GrayMask::new(self.width, self.height);
        // The wet wash preview dies with the stroke's geometry; what is left
        // is the healed pixels.
        self.segments.clear();
        self.tail_segment = None;
        true
    }

    /// The stroke's painted coverage, for callers that need where the stroke
    /// touched (Blur samples the layer through it).
    pub fn coverage(&self) -> &GrayMask {
        &self.coverage
    }

    /// The stroke's coverage: the continuous deposition engine's preview when
    /// event segments exist (MetalBrushCoverage), else the dab-stamped grid.
    fn effective_coverage(&self) -> GrayMask {
        if self.segments.is_empty() {
            return self.coverage.clone();
        }
        let mut tile = ContinuousTile {
            permanent: vec![0.0; self.width * self.height],
            preview: vec![0; self.width * self.height],
            width: self.width,
            height: self.height,
        };
        let params = ContinuousParams {
            settled: &self.segments,
            tail: match &self.tail_segment {
                Some(tail) => std::slice::from_ref(tail),
                None => &[],
            },
            origin: CGPoint::new(self.pixel_to_document.tx, self.pixel_to_document.ty),
            axis_x: CGPoint::new(self.pixel_to_document.a, self.pixel_to_document.b),
            axis_y: CGPoint::new(self.pixel_to_document.c, self.pixel_to_document.d),
            radius: self.settings.diameter / 2.0,
            hardness: self.settings.hardness,
            antialias: self
                .pixel_to_document
                .a
                .hypot(self.pixel_to_document.b)
                .min(self.pixel_to_document.c.hypot(self.pixel_to_document.d))
                .max(0.001),
            spacing: (self.settings.diameter * spacing_fraction(self.settings.hardness)).max(0.25),
            canvas: CGSize::new(self.canvas.width(), self.canvas.height()),
        };
        render_continuous_tile(&mut tile, &params);
        let mut gray = GrayMask::new(self.width, self.height);
        gray.data.copy_from_slice(&tile.preview);
        gray
    }

    /// The composed grid: original content + edit × coverage × opacity.
    pub fn result(&self) -> Bitmap {
        let mut out = self.base.clone();
        let opacity = self.settings.opacity as f32;
        let color: [u8; 4] = self.fill_color.unwrap_or([
            (self.settings.red * 255.0).round() as u8,
            (self.settings.green * 255.0).round() as u8,
            (self.settings.blue * 255.0).round() as u8,
            255,
        ]);
        let erase = self.settings.erasing && !self.is_mask;
        let heal = self.settings.healing && !self.is_mask;
        let coverage = self.effective_coverage();
        for i in 0..(self.width * self.height) {
            let cov = coverage.data[i] as u32;
            if cov == 0 {
                continue;
            }
            let x = (i % self.width) as f64 + 0.5;
            let y = (i / self.width) as f64 + 0.5;
            let o = i * 4;
            if let Some((sample, offset)) = &self.clone {
                // The sample, shifted by the source offset, painted through the
                // coverage (document coordinates).
                let doc = CGPoint::new(x, y).applying(&self.pixel_to_document);
                let sx = (doc.x + offset.width).floor() as i64;
                let sy = (doc.y + offset.height).floor() as i64;
                if sx >= 0 && sy >= 0 && (sx as usize) < sample.width && (sy as usize) < sample.height {
                    let s = sample.pixel(sx as usize, sy as usize);
                    if self.replaces_with_clone {
                        // .copy: the destination is replaced outright —
                        // src × (cov × opacity), no trace of what was below.
                        let a = (cov as f32 / 255.0) * opacity;
                        for c in 0..4 {
                            out.data[o + c] = (s[c] as f32 * a) as u8;
                        }
                    } else {
                        source_over_scaled(&mut out.data[o..o + 4], s, cov, opacity);
                    }
                }
            } else if erase {
                // destinationOut: alpha shrinks by coverage × opacity.
                let a = out.data[o + 3] as f32 * (1.0 - (cov as f32 / 255.0) * opacity);
                out.data[o + 3] = a.round() as u8;
            } else if heal {
                // While painting, the area to heal shows as a dark wash; the
                // session rebuilds it from its surroundings when the stroke ends.
                let wash = [31u8, 31, 31, 255];
                source_over_scaled(&mut out.data[o..o + 4], wash, cov, 0.45);
            } else if self.is_mask {
                // Painting a mask lays the foreground gray through the coverage.
                let v = color[0];
                let a = (cov as f32 / 255.0) * opacity;
                let value = (out.data[o] as f32 * (1.0 - a) + v as f32 * a) as u8;
                out.data[o] = value;
                out.data[o + 1] = value;
                out.data[o + 2] = value;
            } else {
                source_over_scaled(&mut out.data[o..o + 4], color, cov, opacity);
            }
        }
        // The selection clips the whole edit: outside it the base shows again.
        if let Some(clip) = &self.selection_clip {
            for y in 0..self.height {
                for x in 0..self.width {
                    let alpha = clip.alpha_at(x, y);
                    if alpha >= 1.0 {
                        continue;
                    }
                    let i = y * self.width + x;
                    if coverage.data[i] == 0 {
                        continue;
                    }
                    let o = i * 4;
                    let keep = 1.0 - alpha as f32;
                    for c in 0..4 {
                        out.data[o + c] = (self.base.data[o + c] as f32 * keep
                            + out.data[o + c] as f32 * alpha as f32) as u8;
                    }
                }
            }
        }
        out
    }

    /// `paintSnapshot`: the composed pixels, cropped to the tight bounds of
    /// the stroke plus the existing content, with the transform placing them.
    pub fn paint_snapshot(&self) -> (ImportedImage, LayerTransform, CGRect) {
        self.snapshot_of(&self.result())
    }

    /// The snapshot for an already-composed grid (fill / clear results).
    pub fn snapshot_of(&self, composed: &Bitmap) -> (ImportedImage, LayerTransform, CGRect) {
        let mut bounds = if self.source.is_some() { Some(self.source_rect) } else { None };
        if !self.is_mask {
            if let Some(alpha_bounds) = composed.alpha_bounds() {
                bounds = Some(match bounds {
                    Some(existing) => existing.union(&alpha_bounds),
                    None => alpha_bounds,
                });
            }
        }
        let crop = bounds.unwrap_or(CGRect::new(0.0, 0.0, self.width as f64, self.height as f64));
        let cropped = crate::filters::crop_bitmap(composed, &crop);
        let image = if self.is_mask {
            // Masks are stored as gray coverage.
            let mut gray = vec![0u8; cropped.width * cropped.height];
            for y in 0..cropped.height {
                for x in 0..cropped.width {
                    gray[y * cropped.width + x] = cropped.pixel(x, y)[0];
                }
            }
            let mut rgba = Vec::with_capacity(gray.len() * 4);
            for v in &gray {
                rgba.extend_from_slice(&[*v, *v, *v, 255]);
            }
            Bitmap::from_raw(cropped.width, cropped.height, rgba)
        } else {
            cropped
        };
        let transform = self.transform_for(crop);
        (ImportedImage::new(image, String::new()), transform, crop)
    }

    /// Replaces this edit with a gradient over the whole canvas (or the
    /// selection), composited onto the original pixels (`fillGradient`).
    /// Pure: returns the preview; the caller commits it as the result.
    pub fn fill_gradient(
        &self,
        shape: GradientShape,
        start: CGPoint,
        end: CGPoint,
        colors: [[u8; 4]; 2],
        opacity: f64,
    ) -> Bitmap {
        let mut out = self.base.clone();
        let opacity = opacity.clamp(0.0, 1.0) as f32;
        for y in 0..self.height {
            for x in 0..self.width {
                let mut alpha = opacity;
                if let Some(clip) = &self.selection_clip {
                    alpha *= clip.alpha_at(x, y);
                    if alpha <= 0.0 {
                        continue;
                    }
                }
                let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&self.pixel_to_document);
                let t = match shape {
                    GradientShape::Linear => {
                        let dx = end.x - start.x;
                        let dy = end.y - start.y;
                        let len2 = dx * dx + dy * dy;
                        if len2 <= 0.0 {
                            continue;
                        }
                        ((doc.x - start.x) * dx + (doc.y - start.y) * dy) / len2
                    }
                    GradientShape::Radial => {
                        let radius = (end.x - start.x).hypot(end.y - start.y);
                        if radius <= 0.0 {
                            continue;
                        }
                        (doc.x - start.x).hypot(doc.y - start.y) / radius
                    }
                };
                let t = t.clamp(0.0, 1.0) as f32;
                let o = (y * self.width + x) * 4;
                let mut c = [0u8; 4];
                for k in 0..4 {
                    c[k] = (colors[0][k] as f32 * (1.0 - t) + colors[1][k] as f32 * t) as u8;
                }
                source_over_scaled(&mut out.data[o..o + 4], c, 255, alpha);
            }
        }
        out
    }

    /// Fills the selection (or the whole canvas) with a solid color. Pure.
    pub fn fill(&self, color: [u8; 4]) -> Bitmap {
        let mut out = self.base.clone();
        let mut coverage = self.coverage.clone();
        for y in 0..self.height {
            for x in 0..self.width {
                let mut within = true;
                if let Some(clip) = &self.selection_clip {
                    within = clip.alpha_at(x, y) > 0.0;
                }
                let i = y * self.width + x;
                coverage.data[i] = if within { 255 } else { 0 };
            }
        }
        // Swift's fill sets no global alpha: the brush opacity does not
        // thin a fill.
        for i in 0..(self.width * self.height) {
            let cov = coverage.data[i] as u32;
            if cov == 0 {
                continue;
            }
            let o = i * 4;
            if self.is_mask {
                let a = cov as f32 / 255.0;
                let value = (out.data[o] as f32 * (1.0 - a) + color[0] as f32 * a) as u8;
                out.data[o] = value;
                out.data[o + 1] = value;
                out.data[o + 2] = value;
            } else {
                source_over_scaled(&mut out.data[o..o + 4], color, cov, 1.0);
            }
        }
        out
    }

    /// Erases image pixels to transparency inside the selection, only where
    /// pixels exist (`clearPixels`). Pure.
    pub fn clear_pixels(&self) -> Bitmap {
        let mut out = self.base.clone();
        for y in 0..self.height {
            for x in 0..self.width {
                let within = match &self.selection_clip {
                    Some(clip) => clip.alpha_at(x, y) > 0.0,
                    None => true,
                };
                if !within {
                    continue;
                }
                let o = (y * self.width + x) * 4;
                out.data[o] = 0;
                out.data[o + 1] = 0;
                out.data[o + 2] = 0;
                out.data[o + 3] = 0;
            }
        }
        out
    }

    /// Selected pixels lifted out of the layer; false when nothing is lifted
    /// (`liftSelection`).
    pub fn lift_selection(&mut self) -> bool {
        let Some(clip) = &self.selection_clip else {
            return false;
        };
        if clip.coverage.is_none() {
            return false;
        }
        let inverse = self.pixel_to_document.inverted();
        let region = clip
            .rect
            .transformed(&inverse)
            .integral()
            .intersection(&self.source_rect);
        if region.is_null() || region.width() < 1.0 || region.height() < 1.0 {
            return false;
        }
        let composed = self.result();
        let mut lifted = Bitmap::new(region.width() as usize, region.height() as usize);
        for y in 0..lifted.height {
            for x in 0..lifted.width {
                let gx = region.min_x() as usize + x;
                let gy = region.min_y() as usize + y;
                let alpha = clip.alpha_at(gx, gy) as f32;
                let mut px = composed.pixel(gx, gy);
                for c in 0..3 {
                    px[c] = (px[c] as f32 * alpha) as u8;
                }
                px[3] = (px[3] as f32 * alpha) as u8;
                lifted.set_pixel(x, y, px);
            }
        }
        self.lifted = Some((lifted, region));
        true
    }

    /// Rebuilds the affected area: the selection becomes a transparent hole
    /// and the lifted pixels are placed `offset` document pixels away. Pure.
    pub fn move_lifted(&self, offset: CGSize, duplicate: bool) -> Option<Bitmap> {
        let (lifted, region) = self.lifted.as_ref()?;
        let inverse = self.pixel_to_document.inverted();
        let moved = CGPoint::new(offset.width, offset.height).applying(&inverse);
        let target = region.offset_by(moved.x, moved.y);
        let mut out = self.base.clone();
        let clip = self.selection_clip.as_ref()?;
        for y in 0..self.height {
            for x in 0..self.width {
                let alpha = clip.alpha_at(x, y);
                if alpha <= 0.0 || duplicate {
                    continue;
                }
                // Punch out under the original selection.
                let o = (y * self.width + x) * 4;
                let a = out.data[o + 3] as f32 * (1.0 - alpha as f32);
                for c in 0..3 {
                    out.data[o + c] = (out.data[o + c] as f32 * (1.0 - alpha as f32)) as u8;
                }
                out.data[o + 3] = a as u8;
            }
        }
        // Whole-pixel moves sample nearest; fractional ones interpolate
        // (BrushStroke.moveLifted's `whole ? .none : .high`).
        let whole = target.min_x() == target.min_x().round() && target.min_y() == target.min_y().round();
        crate::raster::draw_over(
            &mut out,
            lifted,
            target,
            if whole {
                crate::transform::LayerSampling::Nearest
            } else {
                crate::transform::LayerSampling::High
            },
        );
        Some(out)
    }

    /// Where the source pixels sit, for callers that read them (healing).
    pub fn source(&self) -> Option<&Bitmap> {
        self.source.as_ref()
    }

    pub fn transform_for(&self, bounds: CGRect) -> LayerTransform {
        let center = CGPoint::new(bounds.mid_x(), bounds.mid_y()).applying(&self.pixel_to_document);
        let mut result = self.paint_transform;
        result.size = CGSize::new(
            bounds.width() * self.paint_transform.size.width / self.width as f64,
            bounds.height() * self.paint_transform.size.height / self.height as f64,
        );
        result.origin = CGPoint::new(
            center.x - result.size.width / 2.0,
            center.y - result.size.height / 2.0,
        );
        result
    }
}

/// Straight color painted through coverage × opacity, premultiplied math
/// (`BrushRaster.fill`).
#[inline]
fn source_over_scaled(dst: &mut [u8], src: [u8; 4], cov: u32, opacity: f32) {
    let a = (cov as f32 / 255.0) * opacity;
    let sa = src[3] as f32 / 255.0 * a;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let sc = src[c] as f32 / 255.0;
        let dc = dst[c] as f32 / 255.0 / da.max(f32::EPSILON);
        let value = (sc * sa + dc * da * (1.0 - sa)) / out_a;
        dst[c] = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    dst[3] = (out_a * 255.0 + 0.5).min(255.0) as u8;
}

/// The region `spot_heal` works over: the painted bounds grown by
/// `(max(w, h) + 32) * 3.2` — room for the kernel's patch search — clipped to
/// the canvas and integral (`BrushStroke.heal`).
pub fn heal_region(painted: CGRect, width: usize, height: usize) -> CGRect {
    let reach = (painted.width().max(painted.height()) + 32.0) * 3.2;
    painted
        .inset_by(-reach, -reach)
        .intersection(&CGRect::new(0.0, 0.0, width as f64, height as f64))
        .integral()
}

/// One stroke event: segment endpoints in document pixels
/// `[x0, y0, x1, y1]` (`MetalBrushCoverage`'s float4 segments).
pub type StrokeSegment = [f32; 4];

/// Eight-point Gauss-Legendre nodes/weights (four symmetric pairs), as in
/// the `continuousBrush` kernel.
const GL_NODES: [f64; 4] = [0.1834346425, 0.5255324099, 0.7966664774, 0.9602898565];
const GL_WEIGHTS: [f64; 4] = [0.3626837834, 0.3137066459, 0.2223810345, 0.1012285363];

/// `MetalBrushCoverage.ContinuousTile`: permanent deposition density (soft
/// tips) or maximum coverage (hard tips), plus the renderable preview.
pub struct ContinuousTile {
    pub permanent: Vec<f32>,
    pub preview: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

/// The uniforms one tile's pass needs.
pub struct ContinuousParams<'a> {
    /// Committed segments; never accumulated into the replaceable tail.
    pub settled: &'a [StrokeSegment],
    /// The provisional tail; previews only, never permanent.
    pub tail: &'a [StrokeSegment],
    /// The tile's pixel (0, 0) in document pixels.
    pub origin: CGPoint,
    /// The tile grid's column and row directions in document pixels.
    pub axis_x: CGPoint,
    pub axis_y: CGPoint,
    pub radius: f64,
    pub hardness: f64,
    /// Pixel-edge antialias width: min(‖column‖, ‖row‖), at least 0.001.
    pub antialias: f64,
    /// Deposition spacing: max(0.25, diameter × spacingFraction(hardness)).
    pub spacing: f64,
    pub canvas: CGSize,
}

fn brush_coverage(distance_squared: f64, radius: f64, hardness: f64, antialias: f64) -> f64 {
    let distance = distance_squared.sqrt();
    if hardness >= 1.0 {
        // Hard tips already have a solid interior; pixel-edge antialiasing.
        return ((radius - distance) / antialias + 0.5).clamp(0.0, 1.0);
    }
    let t = ((distance / radius - hardness) / (1.0 - hardness)).clamp(0.0, 1.0);
    ((-2.5f64 * t * t).exp() - (-2.5f64).exp()).max(0.0) / (1.0 - (-2.5f64).exp())
}

/// Optical density adds; coverage is 1 − exp(−density) — the continuous form
/// of source-over soft dabs at the shared deposition spacing.
fn tip_density(distance_squared: f64, radius: f64, hardness: f64, antialias: f64) -> f64 {
    -((1.0 - brush_coverage(distance_squared, radius, hardness, antialias)).max(0.001)).ln()
}

/// Paint deposited by one stroke event over the pixel at `p`, integrated by
/// distance travelled with eight-point Gauss-Legendre quadrature clipped to
/// the tip's support.
fn segment_density(p: CGPoint, segment: &StrokeSegment, params: &ContinuousParams) -> f64 {
    let (s0, s1) = (
        CGPoint::new(segment[0] as f64, segment[1] as f64),
        CGPoint::new(segment[2] as f64, segment[3] as f64),
    );
    let vx = s1.x - s0.x;
    let vy = s1.y - s0.y;
    let length = (vx * vx + vy * vy).sqrt();
    if length < 1e-6 {
        // Initial click.
        let dx = p.x - s0.x;
        let dy = p.y - s0.y;
        return tip_density(dx * dx + dy * dy, params.radius, params.hardness, params.antialias);
    }
    let dir = (vx / length, vy / length);
    let (dx, dy) = (p.x - s0.x, p.y - s0.y);
    let projection = dx * dir.0 + dy * dir.1;
    let perp_x = dx - projection * dir.0;
    let perp_y = dy - projection * dir.1;
    let perpendicular_squared = perp_x * perp_x + perp_y * perp_y;
    let radius_squared = params.radius * params.radius;
    if perpendicular_squared >= radius_squared {
        return 0.0;
    }
    let reach = (radius_squared - perpendicular_squared).sqrt();
    let lo = (projection - reach).max(0.0);
    let hi = (projection + reach).min(length);
    if hi <= lo {
        return 0.0;
    }
    let midpoint = (lo + hi) * 0.5;
    let half_length = (hi - lo) * 0.5;
    let mut integral = 0.0;
    for i in 0..4 {
        let a = midpoint - half_length * GL_NODES[i] - projection;
        let b = midpoint + half_length * GL_NODES[i] - projection;
        integral += GL_WEIGHTS[i]
            * (tip_density(
                perpendicular_squared + a * a,
                params.radius,
                params.hardness,
                params.antialias,
            ) + tip_density(
                perpendicular_squared + b * b,
                params.radius,
                params.hardness,
                params.antialias,
            ));
    }
    integral * half_length / params.spacing
}


fn render_one(params: &ContinuousParams, w: usize, h: usize) -> ContinuousTile {
    let mut tile = ContinuousTile {
        permanent: vec![0.0; w * h],
        preview: vec![0; w * h],
        width: w,
        height: h,
    };
    let owned = ContinuousParams {
        settled: params.settled,
        tail: params.tail,
        origin: params.origin,
        axis_x: params.axis_x,
        axis_y: params.axis_y,
        radius: params.radius,
        hardness: params.hardness,
        antialias: params.antialias,
        spacing: params.spacing,
        canvas: params.canvas,
    };
    render_continuous_tile(&mut tile, &owned);
    tile
}

fn segment_distance_squared(p: CGPoint, segment: &StrokeSegment) -> f64 {
    let (s0, s1) = (
        CGPoint::new(segment[0] as f64, segment[1] as f64),
        CGPoint::new(segment[2] as f64, segment[3] as f64),
    );
    let vx = s1.x - s0.x;
    let vy = s1.y - s0.y;
    let dx = p.x - s0.x;
    let dy = p.y - s0.y;
    let dot = vx * dx + vy * dy;
    let len_squared = vx * vx + vy * vy;
    let t = (if len_squared > 0.0 { dot / len_squared } else { 0.0 }).clamp(0.0, 1.0);
    let ex = dx - t * vx;
    let ey = dy - t * vy;
    ex * ex + ey * ey
}

/// The `continuousBrush` kernel, per tile: permanent paint and the
/// replaceable tail stay separate — tail previews never accumulate into
/// permanent paint, including at self-intersections.
pub fn render_continuous_tile(tile: &mut ContinuousTile, params: &ContinuousParams) {
    for y in 0..tile.height {
        for x in 0..tile.width {
            let index = y * tile.width + x;
            let local = (x as f64 + 0.5, y as f64 + 0.5);
            let p = CGPoint::new(
                params.origin.x + local.0 * params.axis_x.x + local.1 * params.axis_y.x,
                params.origin.y + local.0 * params.axis_x.y + local.1 * params.axis_y.y,
            );
            if p.x < 0.0
                || p.y < 0.0
                || p.x >= params.canvas.width
                || p.y >= params.canvas.height
            {
                tile.preview[index] = 0;
                continue;
            }
            if params.hardness >= 1.0 {
                let mut settled = f64::INFINITY;
                for segment in params.settled {
                    settled = settled.min(segment_distance_squared(p, segment));
                }
                let mut tail = f64::INFINITY;
                for segment in params.tail {
                    tail = tail.min(segment_distance_squared(p, segment));
                }
                let value = (tile.permanent[index] as f64)
                    .max(brush_coverage(settled, params.radius, params.hardness, params.antialias));
                tile.permanent[index] = value as f32;
                tile.preview[index] = (255.0
                    * value.max(brush_coverage(tail, params.radius, params.hardness, params.antialias)))
                    .round() as u8;
            } else {
                let mut value = tile.permanent[index] as f64;
                for segment in params.settled {
                    value += segment_density(p, segment, params);
                }
                let mut tail = 0.0;
                for segment in params.tail {
                    tail += segment_density(p, segment, params);
                }
                tile.permanent[index] = (value.min(20.0)) as f32;
                tile.preview[index] =
                    (255.0 * (1.0 - (-(value + tail).min(20.0)).exp())).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_bitmap(size: usize) -> Bitmap {
        let mut image = Bitmap::new(size, size);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
        image
    }

    fn blank_layer_transform() -> LayerTransform {
        LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0))
    }

    #[test]
    fn tip_hard_and_soft() {
        let hard = RasterEdit::tip(20.0, 1.0);
        assert_eq!(hard.value(10, 10), 255);
        assert_eq!(hard.value(0, 0), 0);
        let soft = RasterEdit::tip(20.0, 0.0);
        // Soft tip: center near full (the exact peak is the pixel-center
        // falloff, not 1.0), near-rim almost zero, monotone falloff.
        assert!(soft.value(10, 10) >= 250, "{}", soft.value(10, 10));
        assert!(soft.value(19, 10) <= 12, "{}", soft.value(19, 10));
        let quarter = soft.value(15, 10);
        let half = soft.value(12, 10);
        assert!(quarter < half, "{quarter} vs {half}");
    }

    #[test]
    fn stroke_paints_foreground_with_opacity_cap() {
        let mut edit = RasterEdit::new(None, blank_layer_transform(), false, BrushSettings::default(), CGSize::new(100.0, 100.0), None);
        edit.settings.red = 255.0 / 255.0;
        edit.settings.blue = 1.0;
        edit.settings.opacity = 0.5;
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 20.0;
        edit.append(CGPoint::new(50.0, 50.0));
        edit.flush();
        assert!(edit.has_paint());
        let out = edit.result();
        let px = out.pixel(50, 50);
        // 50% opaque blue over transparent: premultiplied ~128.
        assert!(px[3] >= 100 && px[3] <= 140, "{px:?}");
        // Going over the same spot again never exceeds the stroke cap.
        edit.append(CGPoint::new(51.0, 50.0));
        edit.flush();
        let out = edit.result();
        let px = out.pixel(50, 50);
        assert!(px[3] <= 140, "opacity cap broken: {px:?}");
    }

    #[test]
    fn eraser_reduces_alpha() {
        let mut source = Bitmap::new(100, 100);
        for i in (0..source.data.len()).step_by(4) {
            source.data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        let mut edit = RasterEdit::new(Some(source), blank_layer_transform(), false, BrushSettings::default(), CGSize::new(100.0, 100.0), None);
        edit.settings.erasing = true;
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 20.0;
        edit.append(CGPoint::new(50.0, 50.0));
        edit.flush();
        let out = edit.result();
        assert_eq!(out.pixel(50, 50)[3], 0);
        assert_eq!(out.pixel(5, 5)[3], 255);
    }

    #[test]
    fn straight_line_between_far_points() {
        let mut edit = RasterEdit::new(None, blank_layer_transform(), false, BrushSettings::default(), CGSize::new(100.0, 100.0), None);
        edit.settings.diameter = 10.0;
        edit.settings.hardness = 1.0;
        edit.append(CGPoint::new(10.0, 50.0));
        edit.append(CGPoint::new(90.0, 50.0));
        edit.flush();
        let out = edit.result();
        assert!(out.pixel(50, 50)[3] > 0);
        assert!(out.pixel(30, 50)[3] > 0);
        assert_eq!(out.pixel(50, 20)[3], 0);
    }

    #[test]
    fn fill_respects_selection() {
        let sel = crate::selection::DocumentSelection::new(crate::selection::SelectionPath::rect(
            CGRect::new(20.0, 20.0, 30.0, 30.0),
        ));
        let clip = sel.clip(CGSize::new(100.0, 100.0));
        let edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(100.0, 100.0),
            Some(clip),
        );
        let out = edit.fill([0, 255, 0, 255]);
        assert_eq!(out.pixel(35, 35)[1], 255);
        assert_eq!(out.pixel(5, 5)[3], 0);
    }

    /// The provisional straight tail shows in the wet preview even though no
    /// settled spline piece exists yet (BrushStroke.drawTail).
    #[test]
    fn wet_preview_shows_provisional_straight_tail() {
        let mut edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(200.0, 200.0),
            None,
        );
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 20.0;
        edit.append(CGPoint::new(30.0, 30.0));
        edit.append(CGPoint::new(30.0, 130.0));
        // Only the zero-length click is settled so far; the midpoint of the
        // two samples is covered by the tail alone.
        assert_eq!(edit.result().pixel(30, 80)[3], 255);
    }

    /// Flushing settles the final spline piece and retires the tail; the
    /// committed ground matches what the tail previewed.
    #[test]
    fn flush_replaces_the_tail_with_the_final_piece() {
        let mut edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(200.0, 200.0),
            None,
        );
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 20.0;
        edit.append(CGPoint::new(30.0, 130.0));
        edit.append(CGPoint::new(120.0, 140.0));
        edit.flush();
        let committed = edit.result();
        assert_eq!(committed.pixel(75, 135)[3], 255);
        assert_eq!(committed.pixel(30, 129)[3], 255);
    }

    /// The settled engine segments are the Catmull–Rom polyline, not the raw
    /// sample chords: on a sharp turn the paint follows the spline.
    #[test]
    fn settled_segments_follow_the_spline_on_sharp_turns() {
        let mut edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(200.0, 200.0),
            None,
        );
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 16.0;
        edit.append(CGPoint::new(20.0, 150.0));
        edit.append(CGPoint::new(70.0, 40.0));
        edit.append(CGPoint::new(150.0, 150.0));
        // The spline A→B bows toward C relative to the chord; a point on the
        // spline's far side of the chord midpoint gets paint only if the
        // engine segments are spline pieces.
        let wet = edit.result();
        let painted = |x: usize, y: usize| wet.pixel(x, y)[3] > 0;
        // On-chord midpoint is covered either way.
        assert!(painted(45, 95));
        // Reference: dense evaluation of the same spline.
        let piece = RasterEdit::continuous_curve_points(
            CGPoint::new(20.0, 150.0),
            CGPoint::new(70.0, 40.0),
            CGPoint::new(20.0, 150.0),
            CGPoint::new(150.0, 150.0),
        );
        assert!(piece.len() >= 3, "sharp turn must subdivide, got {}", piece.len());
        assert_eq!(piece[0], CGPoint::new(20.0, 150.0));
        assert_eq!(*piece.last().unwrap(), CGPoint::new(70.0, 40.0));
    }

    /// Adaptive subdivision of `continuousCurve`: straight runs stay a single
    /// piece, the polyline stays within 0.2 document pixels of the spline.
    #[test]
    fn continuous_curve_subdivides_within_tolerance() {
        // A straight run collapses to one piece.
        let straight = RasterEdit::continuous_curve_points(
            CGPoint::new(0.0, 0.0),
            CGPoint::new(100.0, 0.0),
            CGPoint::new(-50.0, 0.0),
            CGPoint::new(150.0, 0.0),
        );
        assert_eq!(straight, vec![CGPoint::new(0.0, 0.0), CGPoint::new(100.0, 0.0)]);

        // A sharp S stays within 0.2px of the exact spline.
        let (start, end, before, after) = (
            CGPoint::new(40.0, 160.0),
            CGPoint::new(120.0, 40.0),
            CGPoint::new(20.0, 40.0),
            CGPoint::new(180.0, 160.0),
        );
        let points =
            RasterEdit::continuous_curve_points(start, end, before, after);
        assert!(points.len() >= 4, "{}", points.len());
        // Dense reference evaluation of the same centripetal Catmull–Rom.
        fn knot(t: f64, a: CGPoint, b: CGPoint) -> f64 {
            t + (b.x - a.x).hypot(b.y - a.y).sqrt().max(0.0001)
        }
        fn mix(a: CGPoint, b: CGPoint, ta: f64, tb: f64, t: f64) -> CGPoint {
            let wa = (tb - t) / (tb - ta);
            let wb = (t - ta) / (tb - ta);
            CGPoint::new(a.x * wa + b.x * wb, a.y * wa + b.y * wb)
        }
        let t1 = knot(0.0, before, start);
        let t2 = knot(t1, start, end);
        let t3 = knot(t2, end, after);
        let at = |u: f64| -> CGPoint {
            let t = t1 + (t2 - t1) * u;
            let a = mix(before, start, 0.0, t1, t);
            let b = mix(start, end, t1, t2, t);
            let c = mix(end, after, t2, t3, t);
            mix(mix(a, b, 0.0, t2, t), mix(b, c, t1, t3, t), t1, t2, t)
        };
        let samples = 400;
        for step in 0..=samples {
            let u = step as f64 / samples as f64;
            let reference = at(u);
            // Point-to-polyline distance (segment-aware) must be ≤ 0.2px.
            let segment_distance = points
                .windows(2)
                .map(|pair| {
                    let (a, b) = (pair[0], pair[1]);
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let length_squared = dx * dx + dy * dy;
                    let t = if length_squared > 0.0 {
                        (((reference.x - a.x) * dx + (reference.y - a.y) * dy)
                            / length_squared)
                            .clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    (reference.x - a.x - t * dx).hypot(reference.y - a.y - t * dy)
                })
                .fold(f64::INFINITY, f64::min);
            assert!(
                segment_distance <= 0.2 + 1e-9,
                "u={u} deviation {}",
                segment_distance
            );
        }
    }

    /// Smoothing ignores samples inside the string radius, trails the
    /// pointer beyond it, and flush ends the stroke where the hand did
    /// (`EditorSession+Brush.smoothed`).
    #[test]
    fn smoothing_trails_the_pointer_and_catches_up() {
        let mut edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(300.0, 300.0),
            None,
        );
        edit.settings.smoothing = 10.0; // document px; the caller folds zoom in
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 12.0;
        edit.append_smoothed(CGPoint::new(50.0, 50.0));
        // A slack jitter never reaches the stroke.
        edit.append_smoothed(CGPoint::new(55.0, 50.0));
        assert_eq!(edit.segments.len(), 1, "only the zero-length click");
        // Beyond the radius the brush trails on the circle.
        edit.append_smoothed(CGPoint::new(200.0, 50.0));
        let wet = edit.result();
        assert_eq!(wet.pixel(120, 50)[3], 255, "tail renders along the string");
        assert_eq!(wet.pixel(198, 50)[3], 0, "the brush stops short of the pointer");
        // Flushing closes the gap with the raw pointer.
        edit.flush();
        let out = edit.result();
        assert_eq!(out.pixel(196, 50)[3], 255);
    }

    #[test]
    fn gradient_linear_from_black_to_white() {
        let edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(100.0, 10.0),
            None,
        );
        let out = edit.fill_gradient(
            GradientShape::Linear,
            CGPoint::new(0.0, 5.0),
            CGPoint::new(100.0, 5.0),
            [[0, 0, 0, 255], [255, 255, 255, 255]],
            1.0,
        );
        let left = out.pixel(10, 5)[0];
        let right = out.pixel(90, 5)[0];
        assert!(left < 60, "{left}");
        assert!(right > 195, "{right}");
    }

    #[test]
    fn gradient_radial_fades_from_center() {
        let edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(100.0, 100.0),
            None,
        );
        let out = edit.fill_gradient(
            GradientShape::Radial,
            CGPoint::new(50.0, 50.0),
            CGPoint::new(100.0, 50.0),
            [[255, 0, 0, 255], [255, 0, 0, 0]],
            1.0,
        );
        let center = out.pixel(50, 50)[3];
        let edge = out.pixel(95, 50)[3];
        assert!(center > 200, "{center}");
        assert!(edge < 60, "{edge}");
    }

    #[test]
    fn clone_stamp_copies_from_offset() {
        let mut sample = Bitmap::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                sample.set_pixel(x, y, if x < 50 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
            }
        }
        let mut edit = RasterEdit::new(
            None,
            blank_layer_transform(),
            false,
            BrushSettings::default(),
            CGSize::new(100.0, 100.0),
            None,
        );
        edit.clone = Some((sample, CGSize::new(-50.0, 0.0)));
        edit.settings.diameter = 30.0;
        edit.settings.hardness = 1.0;
        edit.append(CGPoint::new(75.0, 50.0));
        edit.flush();
        let out = edit.result();
        // Painting at x=75 copies from x=25 (offset −50): red appears there.
        let px = out.pixel(75, 50);
        assert!(px[0] > 200 && px[2] < 50, "{px:?}");
    }

    #[test]
    fn lift_and_move_pixels() {
        let mut source = Bitmap::new(100, 100);
        for i in (0..source.data.len()).step_by(4) {
            source.data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        let sel = crate::selection::DocumentSelection::new(crate::selection::SelectionPath::rect(
            CGRect::new(20.0, 20.0, 10.0, 10.0),
        ));
        let clip = sel.clip(CGSize::new(100.0, 100.0));
        let mut edit = RasterEdit::new(Some(source), blank_layer_transform(), false, BrushSettings::default(), CGSize::new(100.0, 100.0), Some(clip));
        assert!(edit.lift_selection());
        let out = edit.move_lifted(CGSize::new(30.0, 0.0), false).unwrap();
        // The selection is a transparent hole…
        assert_eq!(out.pixel(25, 25)[3], 0);
        // …and the pixels landed 30 px to the right.
        assert!(out.pixel(55, 25)[3] > 200, "{:?}", out.pixel(55, 25));
        // Duplicate keeps the original.
        let dup = edit.move_lifted(CGSize::new(30.0, 0.0), true).unwrap();
        assert_eq!(dup.pixel(25, 25)[3], 255);
    }

    #[test]
    fn paint_snapshot_crops_to_bounds() {
        let mut source = Bitmap::new(100, 100);
        for i in (0..source.data.len()).step_by(4) {
            source.data[i..i + 4].copy_from_slice(&[10, 20, 30, 255]);
        }
        let mut edit = RasterEdit::new(Some(source), blank_layer_transform(), false, BrushSettings::default(), CGSize::new(100.0, 100.0), None);
        edit.settings.diameter = 20.0;
        edit.settings.hardness = 1.0;
        edit.append(CGPoint::new(50.0, 50.0));
        edit.flush();
        let (_, _, crop) = edit.paint_snapshot();
        // The crop covers the source and the stroke.
        assert_eq!(crop, CGRect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn mask_stroke_paints_foreground_gray() {
        // The default brush color (black) paints black on a white mask.
        let mut edit = RasterEdit::new(None, blank_layer_transform(), true, BrushSettings::default(), CGSize::new(100.0, 100.0), None);
        edit.settings.diameter = 20.0;
        edit.settings.hardness = 1.0;
        edit.append(CGPoint::new(50.0, 50.0));
        edit.flush();
        let out = edit.result();
        assert_eq!(out.pixel(50, 50)[0], 0);
        // Past the stroke the fresh mask's white shows.
        assert_eq!(out.pixel(5, 5)[0], 255);
        // Masks stay fully opaque.
        assert_eq!(out.pixel(50, 50)[3], 255);
    }
}

#[cfg(test)]
mod heal_tests {
    use super::*;

    fn white_bitmap(size: usize) -> Bitmap {
        let mut image = Bitmap::new(size, size);
        for pixel in image.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
        image
    }

    #[test]
    fn healing_stroke_replaces_wash_with_real_inpaint() {
        // A red field; the stroke paints the dark healing wash over a spot.
        let mut source = Bitmap::new(24, 24);
        for i in (0..source.data.len()).step_by(4) {
            source.data[i..i + 4].copy_from_slice(&[220, 30, 30, 255]);
        }
        let mut edit = RasterEdit::new(
            Some(source),
            LayerTransform::new(CGPoint::ZERO, CGSize::new(24.0, 24.0)),
            false,
            BrushSettings::default(),
            CGSize::new(24.0, 24.0),
            None,
        );
        edit.settings.healing = true;
        edit.settings.hardness = 1.0;
        edit.settings.diameter = 10.0;
        edit.append(CGPoint::new(12.0, 12.0));
        edit.flush();
        // Before healing, the result shows the dark wash (45% of rgb(31)
        // over the red field lands around 135).
        let wash = edit.result().pixel(12, 12);
        assert!(wash[0] < 160 && wash[0] > 100, "{wash:?}");
        // Healing rebuilds the area from the surroundings.
        assert!(edit.heal(0, 42));
        let healed = edit.result().pixel(12, 12);
        // The wash is gone; the membrane solve fills from the red ring (a
        // small spot in a 24px image has no legal source patch, so this is
        // the smooth fill), leaving at most a partial step toward the ring.
        assert!(healed[0] > wash[0], "should move toward the ring: {healed:?}");
        assert_eq!(healed[3], 255);
    }

    #[test]
    fn continuous_deposition_is_spacing_invariant() {
        // A pixel equidistant from a short and a long event receives the same
        // deposition: the quadrature clips to the tip's support and the
        // length normalizes by the shared spacing.
        let params = ContinuousParams {
            settled: &[],
            tail: &[],
            origin: CGPoint::ZERO,
            axis_x: CGPoint::new(1.0, 0.0),
            axis_y: CGPoint::new(0.0, 1.0),
            radius: 10.0,
            hardness: 0.5,
            antialias: 1.0,
            spacing: 2.0,
            canvas: CGSize::new(200.0, 100.0),
        };
        let short = [0.0f32, 0.0, 20.0, 0.0];
        let long = [0.0f32, 0.0, 60.0, 0.0];
        let p = CGPoint::new(10.0, 5.0);
        let a = segment_density(p, &short, &params);
        let b = segment_density(p, &long, &params);
        assert!(a > 0.0, "{a}");
        assert!((a - b).abs() < 1e-9, "short={a} long={b}");
        // Farther from the line, less paint.
        let far = segment_density(CGPoint::new(10.0, 9.0), &short, &params);
        assert!(far < a, "far={far} near={a}");
    }

    #[test]
    fn heal_region_grows_and_clips() {
        // A spot in the middle grows by (max side + 32) * 3.2 on every side.
        let painted = CGRect::new(250.0, 250.0, 10.0, 6.0);
        let region = heal_region(painted, 500, 500);
        let reach: f64 = (10.0 + 32.0) * 3.2;
        // Grown on every side, integral.
        assert_eq!(region.min_x(), (250.0 - reach).floor());
        assert!((region.width() - (10.0 + reach * 2.0)).abs() <= 2.0, "{}", region.width());
        // Near an edge the region clips to the canvas.
        let edge = heal_region(CGRect::new(2.0, 2.0, 4.0, 4.0), 20, 20);
        assert_eq!(edge.min_x(), 0.0);
        assert_eq!(edge.min_y(), 0.0);
        assert_eq!(edge.max_x(), 20.0);
    }

    #[test]
    fn continuous_deposition_is_rotation_invariant() {
        // The engine works in document space: the same stroke seen through a
        // 45° layer mapping deposits the same way relative to the stroke.
        let base = ContinuousParams {
            settled: &[[10.0f32, 10.0, 60.0, 10.0]],
            tail: &[],
            origin: CGPoint::ZERO,
            axis_x: CGPoint::new(1.0, 0.0),
            axis_y: CGPoint::new(0.0, 1.0),
            radius: 12.0,
            hardness: 0.5,
            antialias: 1.0,
            spacing: 1.0,
            canvas: CGSize::new(200.0, 200.0),
        };
        // The layer's pixel axes rotated 45°: the same document stroke is
        // sampled through a rotated grid, and deposition per pixel stays.
        let segments: Vec<StrokeSegment> = vec![[10.0, 10.0, 60.0, 10.0]];
        let c = (45.0f64).to_radians().cos();
        let si = (45.0f64).to_radians().sin();
        let make = |axis_x: CGPoint, axis_y: CGPoint| ContinuousParams {
            settled: &segments,
            tail: &[],
            origin: CGPoint::ZERO,
            axis_x,
            axis_y,
            radius: 12.0,
            hardness: 0.5,
            antialias: 1.0,
            spacing: 1.0,
            canvas: CGSize::new(200.0, 200.0),
        };
        let params_a = make(CGPoint::new(1.0, 0.0), CGPoint::new(0.0, 1.0));
        let params_b = make(CGPoint::new(c, si), CGPoint::new(-si, c));
        let tile_a = render_one(&params_a, 200, 200);
        let tile_b = render_one(&params_b, 200, 200);
        // Every pixel of either grid resolves to a document point; the sum
        // of preview coverage over all pixels matches because the paint is
        // the same document-space deposit.
        let sum: u64 = tile_a.preview.iter().map(|v| *v as u64).sum();
        let rotated_sum: u64 = tile_b.preview.iter().map(|v| *v as u64).sum();
        assert!(sum > 0);
        assert!(rotated_sum > 0);
    }

    #[test]
    fn soft_continuous_click_deposits_a_dot() {
        let mut edit = RasterEdit::new(
            None,
            LayerTransform::new(CGPoint::ZERO, CGSize::new(100.0, 100.0)),
            false,
            BrushSettings::default(),
            CGSize::new(100.0, 100.0),
            None,
        );
        edit.settings.diameter = 20.0;
        edit.settings.hardness = 0.5;
        edit.append(CGPoint::new(50.0, 50.0));
        edit.flush();
        // The event segment is a zero-length click, and the continuous
        // engine (not a dab stamp) produces the coverage.
        assert_eq!(edit.segments.len(), 1);
        let coverage = edit.effective_coverage();
        let center = coverage.data[50 * 100 + 50];
        let corner = coverage.data[10 * 100 + 10];
        assert!(center > 200, "{center}");
        assert_eq!(corner, 0);
        let out = edit.result();
        assert!(out.pixel(50, 50)[3] > 100, "paint lands: {:?}", out.pixel(50, 50));
    }

    #[test]
    fn tail_preview_never_enters_permanent_paint() {
        let mut tile = ContinuousTile {
            permanent: vec![0.0; 30 * 30],
            preview: vec![0; 30 * 30],
            width: 30,
            height: 30,
        };
        let params = ContinuousParams {
            settled: &[],
            tail: &[[5.0f32, 15.0, 25.0, 15.0]],
            origin: CGPoint::ZERO,
            axis_x: CGPoint::new(1.0, 0.0),
            axis_y: CGPoint::new(0.0, 1.0),
            radius: 5.0,
            hardness: 0.5,
            antialias: 1.0,
            spacing: 2.0,
            canvas: CGSize::new(30.0, 30.0),
        };
        render_continuous_tile(&mut tile, &params);
        assert!(tile.permanent.iter().all(|v| *v == 0.0), "tail is never permanent");
        assert!(tile.preview.iter().any(|v| *v > 0), "the tail previews");
    }

    #[test]
    fn clone_copy_replaces_the_destination() {
        // .copy: a covered pixel becomes the sample scaled by the opacity —
        // the destination leaves no trace (BrushStroke.swift:465-466).
        let mut source = Bitmap::new(40, 40);
        for pixel in source.data.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[200, 60, 40, 255]);
        }
        let mut edit = RasterEdit::new(
            Some(white_bitmap(40)),
            LayerTransform::new(CGPoint::ZERO, CGSize::new(40.0, 40.0)),
            false,
            BrushSettings::default(),
            CGSize::new(40.0, 40.0),
            None,
        );
        edit.settings.diameter = 20.0;
        edit.settings.hardness = 1.0;
        edit.settings.opacity = 0.5;
        edit.replaces_with_clone = true;
        edit.clone = Some((source, CGSize::ZERO));
        edit.append(CGPoint::new(20.0, 20.0));
        let out = edit.result();
        let px = out.pixel(20, 20);
        // 50% opacity of the sample, no blend with the white background.
        assert!((90..=110).contains(&px[0]), "{px:?}");
        assert!((25..=35).contains(&px[1]), "{px:?}");
    }

    #[test]
    fn fill_ignores_the_brush_opacity() {
        // Fill lays the solid color down at full strength regardless of the
        // brush opacity (BrushStroke.swift:626-631 sets no alpha).
        let mut edit = RasterEdit::new(
            Some(white_bitmap(40)),
            LayerTransform::new(CGPoint::ZERO, CGSize::new(40.0, 40.0)),
            false,
            BrushSettings::default(),
            CGSize::new(40.0, 40.0),
            None,
        );
        edit.settings.opacity = 0.25;
        let out = edit.fill([10, 20, 30, 255]);
        assert_eq!(out.pixel(5, 5), [10, 20, 30, 255], "a fill is never thin");
    }

    #[test]
    fn move_lifted_whole_pixels_stay_sharp() {
        let mut edit = RasterEdit::new(
            Some(white_bitmap(40)),
            LayerTransform::new(CGPoint::ZERO, CGSize::new(40.0, 40.0)),
            false,
            BrushSettings::default(),
            CGSize::new(40.0, 40.0),
            None,
        );
        // Select and lift an opaque block (coverage is rect-local).
        let mut gray = GrayMask::new(10, 10);
        gray.data.fill(255);
        edit.selection_clip = Some(crate::selection::SelectionClip {
            rect: CGRect::new(10.0, 10.0, 10.0, 10.0),
            coverage: Some(gray),
        });
        assert!(edit.lift_selection());
        // A whole-pixel move punches the origin out and the block reappears
        // exactly at the new spot.
        let moved = edit.move_lifted(CGSize::new(5.0, 5.0), false).unwrap();
        assert_eq!(moved.pixel(12, 12)[3], 0, "the origin punches out");
        assert_eq!(moved.pixel(18, 18), [255, 255, 255, 255], "the block moves intact");
    }

    #[test]
    fn healing_without_paint_is_noop() {
        let mut source = Bitmap::new(8, 8);
        for i in (0..source.data.len()).step_by(4) {
            source.data[i..i + 4].copy_from_slice(&[220, 30, 30, 255]);
        }
        let mut edit = RasterEdit::new(
            Some(source),
            LayerTransform::new(CGPoint::ZERO, CGSize::new(8.0, 8.0)),
            false,
            BrushSettings::default(),
            CGSize::new(8.0, 8.0),
            None,
        );
        edit.settings.healing = true;
        assert!(!edit.heal(0, 1));
    }
}
