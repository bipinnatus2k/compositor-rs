//! Port of `Compositor/Document/SmudgeLiquify.swift` (`WarpStroke`): the
//! Smudge and Liquify brushes. The stroke works on a document-size working
//! copy of the layer as shown, changing it dab by dab; the caller paints the
//! finished result into the layer's own pixels along the stroke's path (see
//! `EditorSession.finishWarp` — reproduced in `finish_into_layer`).

use crate::geometry::{CGPoint, CGSize};
use crate::raster::Bitmap;
use crate::session::BlurToolMode;

/// A Smudge or Liquify stroke in progress.
pub struct WarpStroke {
    /// The layer asset the warp started on, for the finish-time identity
    /// guard (`current.asset?.image === warp.layer.asset?.image`).
    pub layer_asset: Option<std::sync::Arc<crate::raster::Bitmap>>,
    pub layer_transform: crate::transform::LayerTransform,
    pub mode: BlurToolMode,
    pub diameter: f64,
    pub hardness: f64,
    pub strength: f64,
    pub width: usize,
    pub height: usize,
    /// The working copy, changed dab by dab.
    pixels: Bitmap,
    /// Every dab's center, for painting the result into the layer.
    pub points: Vec<CGPoint>,
    last: Option<CGPoint>,
    /// Smudge: the color the brush carries, a (2r+1)² RGBA square.
    carried: Vec<f32>,
    scratch: Vec<f32>,
}

impl WarpStroke {
    /// `WarpStroke.init`. `surface` is the layer as the canvas shows it, at
    /// document size (the composite of the layer's own pixels through its
    /// transform). Settings come from the brush: diameter, hardness, opacity
    /// (as strength).
    pub fn new(surface: Bitmap, mode: BlurToolMode, settings: &crate::tools::brush::BrushSettings) -> WarpStroke {
        let diameter = settings.diameter.max(2.0);
        let hardness = settings.hardness.clamp(0.0, 0.98);
        let strength = settings.opacity.clamp(0.01, 1.0);
        WarpStroke {
            layer_asset: None,
            layer_transform: crate::transform::LayerTransform::new(
                crate::geometry::CGPoint::ZERO,
                crate::geometry::CGSize::new(surface.width as f64, surface.height as f64),
            ),
            mode,
            diameter,
            hardness,
            strength,
            width: surface.width,
            height: surface.height,
            pixels: surface,
            points: Vec::new(),
            last: None,
            carried: Vec::new(),
            scratch: Vec::new(),
        }
    }

    fn radius(&self) -> i64 {
        (self.diameter / 2.0).ceil() as i64
    }

    /// How much a dab moves pixels at a distance `u` (0 center, 1 rim).
    fn weight(&self, u: f32) -> f32 {
        if u >= 1.0 {
            return 0.0;
        }
        let h = self.hardness as f32;
        if u <= h {
            return 1.0;
        }
        let t = (1.0 - u) / (1.0 - h);
        t * t * (3.0 - 2.0 * t)
    }

    /// Continues the stroke to `point`, dabbing along the way.
    pub fn append(&mut self, point: CGPoint) {
        let Some(from) = self.last else {
            self.last = Some(point);
            if self.mode == BlurToolMode::Smudge {
                self.pick_up(point);
            }
            return;
        };
        let distance = (point.x - from.x).hypot(point.y - from.y);
        let spacing = (self.diameter * if self.mode == BlurToolMode::Smudge { 0.08 } else { 0.025 }).max(1.0);
        if distance < spacing {
            return;
        }
        let steps = (distance / spacing).ceil() as usize;
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let next = CGPoint::new(
                from.x + (point.x - from.x) * t,
                from.y + (point.y - from.y) * t,
            );
            if self.mode == BlurToolMode::Smudge {
                self.smudge(next);
            } else {
                self.push(previous, next);
            }
            self.points.push(next);
            previous = next;
        }
        self.last = Some(point);
    }

    /// Smudge picks up the color under the brush to carry along.
    fn pick_up(&mut self, center: CGPoint) {
        let r = self.radius();
        let side = (2 * r + 1) as usize;
        self.carried = vec![0.0; side * side * 4];
        let cx = center.x.round() as i64;
        let cy = center.y.round() as i64;
        for dy in -r..=r {
            let y = cy + dy;
            if y < 0 || y >= self.height as i64 {
                continue;
            }
            for dx in -r..=r {
                let x = cx + dx;
                if x < 0 || x >= self.width as i64 {
                    continue;
                }
                let p = ((y as usize) * self.width + x as usize) * 4;
                let c = (((dy + r) as usize) * side + (dx + r) as usize) * 4;
                for k in 0..4 {
                    self.carried[c + k] = self.pixels.data[p + k] as f32;
                }
            }
        }
    }

    fn smudge(&mut self, center: CGPoint) {
        let r = self.radius();
        let side = (2 * r + 1) as usize;
        let cx = center.x.round() as i64;
        let cy = center.y.round() as i64;
        let keep = self.strength as f32;
        let inv_r = 1.0 / (self.diameter / 2.0) as f32;
        for dy in -r..=r {
            let y = cy + dy;
            if y < 0 || y >= self.height as i64 {
                continue;
            }
            for dx in -r..=r {
                let x = cx + dx;
                if x < 0 || x >= self.width as i64 {
                    continue;
                }
                let w = self.weight(((dx * dx + dy * dy) as f32).sqrt() * inv_r);
                if w <= 0.0 {
                    continue;
                }
                let p = ((y as usize) * self.width + x as usize) * 4;
                let c = (((dy + r) as usize) * side + (dx + r) as usize) * 4;
                for k in 0..4 {
                    let under = self.pixels.data[p + k] as f32;
                    let painted = under + (self.carried[c + k] - under) * w;
                    self.pixels.data[p + k] = painted.round().clamp(0.0, 255.0) as u8;
                    // The brush picks up some of what it just left, more the
                    // weaker the smudge.
                    self.carried[c + k] = painted + (self.carried[c + k] - painted) * keep;
                }
            }
        }
    }

    /// Forward warp: pixels under the brush move with it, most at its center,
    /// fading to none at its rim.
    fn push(&mut self, a: CGPoint, b: CGPoint) {
        let r = self.radius();
        let shift = (
            ((b.x - a.x) * self.strength) as f32,
            ((b.y - a.y) * self.strength) as f32,
        );
        let margin = (shift.0.abs().max(shift.1.abs())).ceil() as i64 + 2;
        let cx = b.x.round() as i64;
        let cy = b.y.round() as i64;
        // A copy of the area as it was before this dab, which the dab samples from.
        let x0 = (cx - r - margin).max(0);
        let x1 = (cx + r + margin).min(self.width as i64 - 1);
        let y0 = (cy - r - margin).max(0);
        let y1 = (cy + r + margin).min(self.height as i64 - 1);
        if x0 > x1 || y0 > y1 {
            return;
        }
        let cw = (x1 - x0 + 1) as usize;
        let ch = (y1 - y0 + 1) as usize;
        if self.scratch.len() < cw * ch * 4 {
            self.scratch = vec![0.0; cw * ch * 4];
        }
        for y in 0..ch {
            for x in 0..cw {
                let p = (((y + y0 as usize) * self.width) + x + x0 as usize) * 4;
                let s = (y * cw + x) * 4;
                for k in 0..4 {
                    self.scratch[s + k] = self.pixels.data[p + k] as f32;
                }
            }
        }
        let inv_r = 1.0 / (self.diameter / 2.0) as f32;
        for dy in -r..=r {
            let y = cy + dy;
            if y < y0 || y > y1 {
                continue;
            }
            for dx in -r..=r {
                let x = cx + dx;
                if x < x0 || x > x1 {
                    continue;
                }
                let w = self.weight(((dx * dx + dy * dy) as f32).sqrt() * inv_r);
                if w <= 0.0 {
                    continue;
                }
                // Bilinear sample of the old pixels, from behind the travel.
                let sx = ((x - x0) as f32 - shift.0 * w).clamp(0.0, (cw - 1) as f32);
                let sy = ((y - y0) as f32 - shift.1 * w).clamp(0.0, (ch - 1) as f32);
                let ix = (sx as usize).min(cw - 2);
                let iy = (sy as usize).min(ch - 2);
                let fx = sx - ix as f32;
                let fy = sy - iy as f32;
                let p = ((y as usize) * self.width + x as usize) * 4;
                let s00 = (iy * cw + ix) * 4;
                let s10 = s00 + 4;
                let s01 = s00 + cw * 4;
                let s11 = s01 + 4;
                for k in 0..4 {
                    let top = self.scratch[s00 + k] + (self.scratch[s10 + k] - self.scratch[s00 + k]) * fx;
                    let bottom = self.scratch[s01 + k] + (self.scratch[s11 + k] - self.scratch[s01 + k]) * fx;
                    self.pixels.data[p + k] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// The working copy as it stands (for the canvas preview).
    pub fn image(&self) -> &Bitmap {
        &self.pixels
    }

    pub fn into_image(self) -> Bitmap {
        self.pixels
    }
}

/// `EditorSession.finishWarp`: paints the finished result into the layer's own
/// pixels along the stroke, with a hard tip a little wider than the brush
/// covering everything the stroke moved. `base_transform`/`source` describe
/// the target layer; the returned bitmap and transform are what to commit.
pub fn finish_warp_into_layer(
    result: &Bitmap,
    points: &[CGPoint],
    diameter: f64,
    source: Option<crate::raster::Bitmap>,
    base_transform: crate::transform::LayerTransform,
    canvas: CGSize,
    edit_name: &str,
    selection_clip: Option<crate::selection::SelectionClip>,
) -> Option<(crate::raster::ImportedImage, crate::transform::LayerTransform, &'static str)> {
    if points.is_empty() {
        return None;
    }
    let mut settings = crate::tools::brush::BrushSettings::default();
    settings.diameter = diameter + 4.0;
    settings.hardness = 1.0;
    settings.opacity = 1.0;
    let mut stroke = crate::tools::brush::RasterEdit::new(
        source,
        base_transform,
        false,
        settings,
        canvas,
        selection_clip,
    );
    stroke.clone = Some((result.clone(), CGSize::ZERO));
    stroke.replaces_with_clone = true;
    for point in points {
        stroke.append(*point);
    }
    stroke.flush();
    if !stroke.has_paint() {
        return None;
    }
    let (snapshot, transform, _) = stroke.paint_snapshot();
    Some((snapshot, transform, static_name(edit_name)))
}

fn static_name(name: &str) -> &'static str {
    match name {
        "Smudge" => "Smudge",
        "Liquify" => "Liquify",
        "Blur" => "Blur",
        _ => "Smear",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::LayerTransform;
    use crate::tools::brush::BrushSettings;

    fn gray_surface(w: usize, h: usize, v: u8) -> Bitmap {
        Bitmap::new_clearing(w, h, [v, v, v, 255])
    }

    fn settings(diameter: f64) -> BrushSettings {
        BrushSettings {
            diameter,
            hardness: 0.9,
            opacity: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn smudge_drays_dark_into_light() {
        // Left half dark, right half light; smudge from the boundary outward
        // carries dark pixels into the light side.
        let mut surface = gray_surface(64, 32, 220);
        for y in 0..32 {
            for x in 0..32 {
                surface.set_pixel(x, y, [40, 40, 40, 255]);
            }
        }
        let mut stroke = WarpStroke::new(surface, BlurToolMode::Smudge, &settings(24.0));
        // The brush picks up at the stroke start (dark, x<32) and drags it
        // into the light side.
        stroke.append(CGPoint::new(32.0, 16.0));
        for x in [40.0, 48.0, 56.0] {
            stroke.append(CGPoint::new(x, 16.0));
        }
        assert!(!stroke.points.is_empty());
        let image = stroke.image();
        // Pixels ahead of the drag were light; smudging carries dark into them.
        let at_52 = image.pixel(52, 16)[0];
        assert!(at_52 < 200, "smudge should carry dark: {at_52}");
        // The dark source region stays dark.
        assert!(image.pixel(10, 16)[0] < 60);
    }

    #[test]
    fn liquify_pushes_pixels() {
        let mut surface = gray_surface(64, 32, 220);
        for y in 0..32 {
            for x in 0..16 {
                surface.set_pixel(x, y, [30, 30, 30, 255]);
            }
        }
        let mut stroke = WarpStroke::new(surface, BlurToolMode::Liquify, &settings(24.0));
        stroke.append(CGPoint::new(16.0, 16.0));
        stroke.append(CGPoint::new(48.0, 16.0));
        let image = stroke.image();
        // Content moved right: the formerly-sharp boundary is smeared out.
        let moved = image.pixel(20, 16)[0];
        assert!(moved < 200, "dark pixels pushed right: {moved}");
    }

    #[test]
    fn stroke_records_dab_points() {
        let stroke_surface = gray_surface(64, 64, 128);
        let mut stroke = WarpStroke::new(stroke_surface, BlurToolMode::Liquify, &settings(20.0));
        stroke.append(CGPoint::new(10.0, 10.0));
        stroke.append(CGPoint::new(50.0, 10.0));
        stroke.append(CGPoint::new(50.0, 50.0));
        assert!(stroke.points.len() >= 4);
    }

    #[test]
    fn tiny_moves_are_ignored() {
        let stroke_surface = gray_surface(64, 64, 128);
        let mut stroke = WarpStroke::new(stroke_surface, BlurToolMode::Smudge, &settings(20.0));
        stroke.append(CGPoint::new(10.0, 10.0));
        stroke.append(CGPoint::new(10.5, 10.5));
        assert!(stroke.points.is_empty());
    }

    #[test]
    fn finish_paints_into_layer() {
        let warped = gray_surface(64, 64, 90);
        let mut points = Vec::new();
        for i in 0..5 {
            points.push(CGPoint::new(10.0 + i as f64 * 8.0, 32.0));
        }
        let base = LayerTransform::new(CGPoint::ZERO, CGSize::new(64.0, 64.0));
        let (image, _transform, name) =
            finish_warp_into_layer(&warped, &points, 20.0, Some(gray_surface(64, 64, 200)), base, CGSize::new(64.0, 64.0), "Smudge", None).unwrap();
        assert_eq!(name, "Smudge");
        // The result painted the warped pixels through the stroke path.
        assert!(image.image.pixel(32, 32)[0] < 200, "{:?}", image.image.pixel(32, 32));
    }
}
