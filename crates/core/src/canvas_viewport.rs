//! Port of `Rendering/CanvasViewport.swift`: the canvas view transform.
//!
//! Document: pixels with top-left origin. View: points. Zoom 1 means actual
//! display pixels. The viewport lives on `EditorSession` (the Swift session's
//! `viewport` property, EditorSession.swift:170), so zoom/pan are per-tab
//! state that survives tab switches; `followsFit` keeps refitting until the
//! user zooms or pans by hand.

use crate::geometry::{CGFloat, CGPoint, CGRect, CGSize};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasViewport {
    pub view_size: CGSize,
    pub backing_scale: CGFloat,
    zoom: CGFloat,
    pub pan: CGSize,
    follows_fit: bool,
}

impl Default for CanvasViewport {
    fn default() -> Self {
        CanvasViewport {
            view_size: CGSize::ZERO,
            backing_scale: 1.0,
            zoom: 1.0,
            pan: CGSize::ZERO,
            follows_fit: true,
        }
    }
}

impl CanvasViewport {
    /// `zoomRange`: 0.001...32.
    pub const ZOOM_MIN: CGFloat = 0.001;
    pub const ZOOM_MAX: CGFloat = 32.0;
    /// `keyboardZoomLevels`: the stable steps Cmd-`+`/`-` walk through.
    pub const KEYBOARD_ZOOM_LEVELS: [CGFloat; 17] = [
        0.125,
        1.0 / 6.0,
        0.25,
        1.0 / 3.0,
        0.5,
        2.0 / 3.0,
        1.0,
        1.25,
        1.5,
        2.0,
        3.0,
        4.0,
        5.0,
        6.0,
        8.0,
        12.0,
        16.0,
    ];

    /// `pointsPerPixel`.
    pub fn points_per_pixel(&self) -> CGFloat {
        self.zoom / self.backing_scale
    }

    /// `zoom` (Swift `private(set) var zoom` — read from anywhere, written
    /// only by `set_zoom`/`fit`).
    pub fn zoom(&self) -> CGFloat {
        self.zoom
    }

    /// `followsFit` (Swift `private(set) var followsFit`).
    pub fn is_following_fit(&self) -> bool {
        self.follows_fit
    }

    /// `center`.
    pub fn center(&self) -> CGPoint {
        CGPoint::new(self.view_size.width / 2.0, self.view_size.height / 2.0)
    }

    /// `documentRect(_:)`: the document rect in view points.
    pub fn document_rect(&self, size: CGSize) -> CGRect {
        let scaled = CGSize::new(
            size.width * self.points_per_pixel(),
            size.height * self.points_per_pixel(),
        );
        let center = self.center();
        CGRect::new(
            center.x - scaled.width / 2.0 + self.pan.width,
            center.y - scaled.height / 2.0 + self.pan.height,
            scaled.width,
            scaled.height,
        )
    }

    /// `documentPoint(from:documentSize:)`.
    pub fn document_point(&self, point: CGPoint, document_size: CGSize) -> CGPoint {
        let origin = self.document_rect(document_size).origin;
        let per_pixel = self.points_per_pixel();
        CGPoint::new(
            (point.x - origin.x) / per_pixel,
            (point.y - origin.y) / per_pixel,
        )
    }

    /// `viewPoint(from:documentSize:)`.
    pub fn view_point(&self, point: CGPoint, document_size: CGSize) -> CGPoint {
        let origin = self.document_rect(document_size).origin;
        let per_pixel = self.points_per_pixel();
        CGPoint::new(
            origin.x + point.x * per_pixel,
            origin.y + point.y * per_pixel,
        )
    }

    /// `fit(documentSize:)`: fill the view with the document, panning to
    /// center. Small documents zoom up (the `/` is against `max(1, size-96)`,
    /// not clamped below 1); huge ones clamp into `zoomRange`. While
    /// `followsFit` the viewport keeps refitting on resize.
    pub fn fit(&mut self, document_size: CGSize) {
        if self.view_size.width <= 0.0 || self.view_size.height <= 0.0 {
            self.follows_fit = true;
            return;
        }
        let target = (1.0f64.max(self.view_size.width - 96.0) / document_size.width)
            .min(1.0f64.max(self.view_size.height - 96.0) / document_size.height)
            * self.backing_scale;
        self.zoom = Self::clamp_zoom(target);
        self.pan = CGSize::ZERO;
        self.follows_fit = true;
    }

    /// `resize(to:backingScale:documentSize:)`: preserve the center document
    /// point when moving between displays; refit while following fit.
    pub fn resize(&mut self, size: CGSize, backing_scale: CGFloat, document_size: Option<CGSize>) {
        let old_scale = self.points_per_pixel();
        self.view_size = size;
        self.backing_scale = backing_scale.max(1.0);
        // Swift: `if followsFit, let documentSize`.
        if self.follows_fit
            && let Some(document_size) = document_size
        {
            self.fit(document_size);
            return;
        }
        let ratio = self.points_per_pixel() / old_scale;
        self.pan = CGSize::new(self.pan.width * ratio, self.pan.height * ratio);
    }

    /// `setZoom(_:anchoredAt:documentSize:)`: keep the document point under
    /// `anchor` pinned while zooming; any manual zoom ends the fit follow.
    pub fn set_zoom(&mut self, value: CGFloat, anchor: CGPoint, document_size: CGSize) {
        if !value.is_finite() {
            return;
        }
        let pixel = self.document_point(anchor, document_size);
        self.zoom = Self::clamp_zoom(value);
        let moved = self.view_point(pixel, document_size);
        self.pan.width += anchor.x - moved.x;
        self.pan.height += anchor.y - moved.y;
        self.follows_fit = false;
    }

    /// `keyboardZoomTarget(by:)`: the next stable keyboard zoom level.
    pub fn keyboard_zoom_target(&self, step: i32) -> CGFloat {
        if step == 0 {
            return self.zoom;
        }
        let tolerance = (self.zoom.abs() * 0.000_000_001).max(0.000_000_001);
        if step > 0 {
            return Self::KEYBOARD_ZOOM_LEVELS
                .iter()
                .copied()
                .find(|level| *level > self.zoom + tolerance)
                .unwrap_or(self.zoom);
        }
        Self::KEYBOARD_ZOOM_LEVELS
            .iter()
            .rev()
            .copied()
            .find(|level| *level < self.zoom - tolerance)
            .unwrap_or(self.zoom)
    }

    /// `translate(by:)`: a pan by hand ends the fit follow.
    pub fn translate(&mut self, delta: CGSize) {
        self.pan.width += delta.width;
        self.pan.height += delta.height;
        self.follows_fit = false;
    }

    /// `clamp(_:)` into `zoomRange`.
    fn clamp_zoom(value: CGFloat) -> CGFloat {
        value.clamp(Self::ZOOM_MIN, Self::ZOOM_MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport(view_width: f64, view_height: f64) -> CanvasViewport {
        CanvasViewport {
            view_size: CGSize::new(view_width, view_height),
            ..CanvasViewport::default()
        }
    }

    #[test]
    fn fit_centers_and_scales_the_document() {
        // `fit`: zoom = min(max(1, w-96)/dw, max(1, h-96)/dh), pan zero.
        let mut vp = viewport(800.0, 600.0);
        vp.fit(CGSize::new(400.0, 300.0));
        assert_eq!(vp.zoom(), 504.0 / 300.0);
        assert_eq!(vp.pan, CGSize::ZERO);
        assert!(vp.is_following_fit());
        // The document rect is centered: center (400, 300) minus the scaled
        // size's half (672×504) gives origin (64, 48).
        let rect = vp.document_rect(CGSize::new(400.0, 300.0));
        assert_eq!(rect.origin, CGPoint::new(64.0, 48.0));
    }

    #[test]
    fn fit_zooms_small_documents_up() {
        // A small document fills the view (Swift does not clamp fit below 1).
        let mut vp = viewport(800.0, 600.0);
        vp.fit(CGSize::new(100.0, 100.0));
        assert_eq!(vp.zoom(), 504.0 / 100.0);
    }

    #[test]
    fn fit_without_a_view_keeps_following_fit() {
        // `guard viewSize... else { followsFit = true }`: an unsized view
        // stays pending so the first layout pass fits.
        let mut vp = CanvasViewport::default();
        vp.set_zoom(4.0, CGPoint::ZERO, CGSize::new(100.0, 100.0));
        vp.fit(CGSize::new(100.0, 100.0));
        assert!(vp.is_following_fit());
    }

    #[test]
    fn set_zoom_pins_the_anchor_point() {
        // `setZoom`: the document point under the anchor stays put.
        let mut vp = viewport(800.0, 600.0);
        vp.fit(CGSize::new(400.0, 300.0));
        let anchor = CGPoint::new(300.0, 200.0);
        let doc_before = vp.document_point(anchor, CGSize::new(400.0, 300.0));
        vp.set_zoom(2.0, anchor, CGSize::new(400.0, 300.0));
        assert_eq!(vp.zoom(), 2.0);
        assert!(!vp.is_following_fit());
        let doc_after = vp.document_point(anchor, CGSize::new(400.0, 300.0));
        assert!((doc_before.x - doc_after.x).abs() < 1e-9);
        assert!((doc_before.y - doc_after.y).abs() < 1e-9);
    }

    #[test]
    fn set_zoom_clamps_into_zoom_range() {
        let mut vp = viewport(800.0, 600.0);
        vp.set_zoom(1_000.0, vp.center(), CGSize::new(400.0, 300.0));
        assert_eq!(vp.zoom(), CanvasViewport::ZOOM_MAX);
        vp.set_zoom(0.0, vp.center(), CGSize::new(400.0, 300.0));
        assert_eq!(vp.zoom(), CanvasViewport::ZOOM_MIN);
        // Non-finite values are refused.
        vp.set_zoom(f64::NAN, vp.center(), CGSize::new(400.0, 300.0));
        assert_eq!(vp.zoom(), CanvasViewport::ZOOM_MIN);
    }

    #[test]
    fn keyboard_zoom_steps_walk_the_level_table() {
        let mut vp = viewport(800.0, 600.0);
        assert_eq!(vp.keyboard_zoom_target(1), 1.25);
        assert_eq!(vp.keyboard_zoom_target(-1), 2.0 / 3.0);
        assert_eq!(vp.keyboard_zoom_target(0), 1.0);
        vp.set_zoom(1.25, vp.center(), CGSize::new(100.0, 100.0));
        assert_eq!(vp.keyboard_zoom_target(1), 1.5);
        // Past the top of the table the zoom stays.
        vp.set_zoom(16.0, vp.center(), CGSize::new(100.0, 100.0));
        assert_eq!(vp.keyboard_zoom_target(1), 16.0);
        vp.set_zoom(0.125, vp.center(), CGSize::new(100.0, 100.0));
        assert_eq!(vp.keyboard_zoom_target(-1), 0.125);
    }

    #[test]
    fn resize_refits_while_following_fit_and_keeps_center_otherwise() {
        // Following fit: a resize refits to the new view.
        let mut vp = viewport(800.0, 600.0);
        vp.fit(CGSize::new(400.0, 300.0));
        vp.resize(
            CGSize::new(1000.0, 800.0),
            1.0,
            Some(CGSize::new(400.0, 300.0)),
        );
        assert_eq!(vp.zoom(), 904.0 / 400.0);
        // Manual zoom: a resize keeps the center document point (pan scales
        // by the points-per-pixel ratio).
        let mut vp = viewport(800.0, 600.0);
        vp.fit(CGSize::new(400.0, 300.0));
        vp.set_zoom(2.0, vp.center(), CGSize::new(400.0, 300.0));
        let center_doc = vp.document_point(vp.center(), CGSize::new(400.0, 300.0));
        vp.resize(
            CGSize::new(1000.0, 800.0),
            1.0,
            Some(CGSize::new(400.0, 300.0)),
        );
        let center_after = vp.document_point(vp.center(), CGSize::new(400.0, 300.0));
        assert!((center_doc.x - center_after.x).abs() < 1e-9);
        assert!((center_doc.y - center_after.y).abs() < 1e-9);
    }

    #[test]
    fn resize_clamps_backing_scale_at_one() {
        let mut vp = viewport(800.0, 600.0);
        vp.resize(CGSize::new(800.0, 600.0), 0.5, None);
        assert_eq!(vp.backing_scale, 1.0);
    }

    #[test]
    fn translate_pans_and_ends_the_fit_follow() {
        let mut vp = viewport(800.0, 600.0);
        vp.fit(CGSize::new(400.0, 300.0));
        vp.translate(CGSize::new(12.0, -30.0));
        assert_eq!(vp.pan, CGSize::new(12.0, -30.0));
        assert!(!vp.is_following_fit());
    }

    #[test]
    fn document_and_view_points_round_trip() {
        let mut vp = viewport(800.0, 600.0);
        vp.set_zoom(1.5, vp.center(), CGSize::new(400.0, 300.0));
        let doc = CGPoint::new(137.5, 42.25);
        let view = vp.view_point(doc, CGSize::new(400.0, 300.0));
        let back = vp.document_point(view, CGSize::new(400.0, 300.0));
        assert!((doc.x - back.x).abs() < 1e-9);
        assert!((doc.y - back.y).abs() < 1e-9);
    }
}
