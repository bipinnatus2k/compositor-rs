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
    use crate::session::EditorSession;

    /// The shared document fixture (CompositorTests.swift:7): 1920×1080.
    fn document() -> CGSize {
        CGSize::new(1920.0, 1080.0)
    }

    fn viewport(view_width: f64, view_height: f64) -> CanvasViewport {
        CanvasViewport {
            view_size: CGSize::new(view_width, view_height),
            ..CanvasViewport::default()
        }
    }

    /// `actualPixelsAndRoundTrip(backing:)` (CompositorTests.swift:19-33):
    /// zoom/translate/round-trip holds at backing scales 1 and 2, and the
    /// document rect's backing-pixel width equals the document width times
    /// the zoom.
    #[test]
    fn actual_pixels_and_round_trip() {
        for backing in [1.0, 2.0] {
            let mut vp = CanvasViewport::default();
            vp.resize(CGSize::new(800.0, 600.0), backing, None);
            for zoom in [0.25, 1.0, 3.75] {
                vp.set_zoom(zoom, vp.center(), document());
                vp.translate(CGSize::new(73.5, -44.25));
                let pixel = CGPoint::new(183.25, 837.5);
                let view = vp.view_point(pixel, document());
                let result = vp.document_point(view, document());
                assert!((result.x - pixel.x).abs() < 0.000_001);
                assert!((result.y - pixel.y).abs() < 0.000_001);
                assert!(
                    (vp.document_rect(document()).size.width * backing - document().width * zoom)
                        .abs()
                        < 0.000_001
                );
            }
        }
    }

    /// `zoomKeepsCursorPixelFixed` (:35-44): the document point under the
    /// anchor survives a setZoom.
    #[test]
    fn zoom_keeps_cursor_pixel_fixed() {
        let mut vp = CanvasViewport::default();
        vp.resize(CGSize::new(1000.0, 700.0), 2.0, Some(document()));
        let anchor = CGPoint::new(157.0, 221.0);
        let before = vp.document_point(anchor, document());
        vp.set_zoom(4.0, anchor, document());
        let after = vp.document_point(anchor, document());
        assert!((before.x - after.x).abs() < 0.000_001);
        assert!((before.y - after.y).abs() < 0.000_001);
    }

    /// `keyboardZoomKeepsPannedViewportCenterFixed` (:46-64).
    #[test]
    fn keyboard_zoom_keeps_panned_viewport_center_fixed() {
        let mut session = EditorSession::new();
        session
            .viewport
            .resize(CGSize::new(1000.0, 800.0), 1.0, None);
        session.create_document(3000, 2000, false);
        session.zoom(1.0, None);
        session.viewport.translate(CGSize::new(-620.0, 185.0));

        let center = session.viewport.center();
        let canvas = CGSize::new(3000.0, 2000.0);
        let before = session.viewport.document_point(center, canvas);
        session.zoom_keyboard(1);
        let after = session.viewport.document_point(center, canvas);

        assert_eq!(session.viewport.zoom(), 1.25);
        assert!((before.x - after.x).abs() < 0.000_001);
        assert!((before.y - after.y).abs() < 0.000_001);
        assert!((session.viewport.view_point(after, canvas).x - center.x).abs() < 0.000_001);
        assert!((session.viewport.view_point(after, canvas).y - center.y).abs() < 0.000_001);
    }

    /// `keyboardZoomUsesStableStopsAndClampsAtTheEnds` (:66-85).
    #[test]
    fn keyboard_zoom_uses_stable_stops_and_clamps_at_the_ends() {
        let mut session = EditorSession::new();
        session
            .viewport
            .resize(CGSize::new(1000.0, 800.0), 1.0, None);
        session.create_document(3000, 2000, false);
        session.zoom(0.5, None);

        session.zoom_keyboard(1);
        assert!((session.viewport.zoom() - 2.0 / 3.0).abs() < 0.000_001);
        session.zoom_keyboard(1);
        assert_eq!(session.viewport.zoom(), 1.0);
        session.zoom_keyboard(1);
        assert_eq!(session.viewport.zoom(), 1.25);

        let first = CanvasViewport::KEYBOARD_ZOOM_LEVELS[0];
        let last =
            CanvasViewport::KEYBOARD_ZOOM_LEVELS[CanvasViewport::KEYBOARD_ZOOM_LEVELS.len() - 1];
        session.zoom(first, None);
        session.zoom_keyboard(-1);
        assert_eq!(session.viewport.zoom(), first);
        session.zoom(last, None);
        session.zoom_keyboard(1);
        assert_eq!(session.viewport.zoom(), last);
    }

    /// `keyboardZoomRoundTripDoesNotDriftAfterTenSteps` (:87-104).
    #[test]
    fn keyboard_zoom_round_trip_does_not_drift_after_ten_steps() {
        let mut session = EditorSession::new();
        session
            .viewport
            .resize(CGSize::new(1200.0, 900.0), 2.0, None);
        session.create_document(4000, 3000, false);
        session.zoom(0.5, None);
        session.viewport.translate(CGSize::new(-430.0, 275.0));

        let center = session.viewport.center();
        let canvas = CGSize::new(4000.0, 3000.0);
        let before = session.viewport.document_point(center, canvas);
        for _ in 0..10 {
            session.zoom_keyboard(1);
        }
        for _ in 0..10 {
            session.zoom_keyboard(-1);
        }
        let after = session.viewport.document_point(center, canvas);

        assert_eq!(session.viewport.zoom(), 0.5);
        assert!((before.x - after.x).abs() < 0.000_001);
        assert!((before.y - after.y).abs() < 0.000_001);
    }

    /// `keyboardZoomKeepsSmallCanvasCentered` (:106-123).
    #[test]
    fn keyboard_zoom_keeps_small_canvas_centered() {
        let mut session = EditorSession::new();
        session
            .viewport
            .resize(CGSize::new(1200.0, 900.0), 1.0, None);
        session.create_document(200, 100, false);
        session.zoom(0.5, None);

        let center = session.viewport.center();
        let canvas = CGSize::new(200.0, 100.0);
        let before = session.viewport.document_point(center, canvas);
        session.zoom_keyboard(1);
        let after = session.viewport.document_point(center, canvas);

        assert!((session.viewport.zoom() - 2.0 / 3.0).abs() < 0.000_001);
        assert!((before.x - 100.0).abs() < 0.000_001);
        assert!((before.y - 50.0).abs() < 0.000_001);
        assert!((before.x - after.x).abs() < 0.000_001);
        assert!((before.y - after.y).abs() < 0.000_001);
    }

    /// `keyboardZoomFromFitUsesNextStopAndLeavesFitMode` (:125-141).
    #[test]
    fn keyboard_zoom_from_fit_uses_next_stop_and_leaves_fit_mode() {
        let mut session = EditorSession::new();
        session
            .viewport
            .resize(CGSize::new(1000.0, 800.0), 1.0, None);
        session.create_document(1500, 1000, false);
        session.fit();

        let center = session.viewport.center();
        let canvas = CGSize::new(1500.0, 1000.0);
        let before = session.viewport.document_point(center, canvas);
        session.zoom_keyboard(1);
        let after = session.viewport.document_point(center, canvas);

        assert!((session.viewport.zoom() - 2.0 / 3.0).abs() < 0.000_001);
        assert!(!session.viewport.is_following_fit());
        assert!((before.x - after.x).abs() < 0.000_001);
        assert!((before.y - after.y).abs() < 0.000_001);
    }

    /// `fitAndResizeModes` (:143-162).
    #[test]
    fn fit_and_resize_modes() {
        let mut vp = CanvasViewport::default();
        vp.resize(CGSize::new(800.0, 600.0), 2.0, Some(document()));
        let rect = vp.document_rect(document());
        assert!(rect.size.width <= 704.000_001);
        assert!(rect.size.height <= 504.000_001);
        assert_eq!(rect.origin.x + rect.size.width / 2.0, 400.0);
        assert_eq!(rect.origin.y + rect.size.height / 2.0, 300.0);
        vp.translate(CGSize::new(60.0, -35.0));
        let before = vp.document_point(vp.center(), document());
        let zoom = vp.zoom();
        vp.resize(CGSize::new(1200.0, 800.0), 1.0, Some(document()));
        let after = vp.document_point(vp.center(), document());
        assert_eq!(vp.zoom(), zoom);
        assert!((before.x - after.x).abs() < 0.000_001);
        assert!((before.y - after.y).abs() < 0.000_001);
        vp.fit(document());
        assert_eq!(vp.pan, CGSize::ZERO);
        assert!(vp.is_following_fit());
    }

    /// `limitsAndNewDocumentReset` (:164-179).
    #[test]
    fn limits_and_new_document_reset() {
        let mut session = EditorSession::new();
        session
            .viewport
            .resize(CGSize::new(800.0, 600.0), 2.0, None);
        session.create_document(1920, 1080, false);
        session.zoom(1000.0, None);
        assert_eq!(session.viewport.zoom(), CanvasViewport::ZOOM_MAX);
        session.zoom(0.0, None);
        assert_eq!(session.viewport.zoom(), CanvasViewport::ZOOM_MIN);
        session.viewport.translate(CGSize::new(999.0, 888.0));
        session.create_document(400, 300, false);
        assert!(session.viewport.is_following_fit());
        assert_eq!(session.viewport.pan, CGSize::ZERO);
        assert_eq!(session.document.as_ref().unwrap().width, 400);
        session.create_document(0, 200, false);
        assert_eq!(session.document.as_ref().unwrap().width, 400);
    }

    // The ports above carry the Swift suite. The extras below cover guards
    // the Swift suite exercises only indirectly.

    #[test]
    fn fit_without_a_view_keeps_following_fit() {
        // `fit`'s `guard viewSize... else { followsFit = true }`: an unsized
        // view stays pending so the first layout pass fits.
        let mut vp = CanvasViewport::default();
        vp.set_zoom(4.0, CGPoint::ZERO, CGSize::new(100.0, 100.0));
        vp.fit(CGSize::new(100.0, 100.0));
        assert!(vp.is_following_fit());
    }

    #[test]
    fn document_and_view_points_round_trip_at_the_zoom_limits() {
        for zoom in [CanvasViewport::ZOOM_MIN, 1.5, CanvasViewport::ZOOM_MAX] {
            let mut vp = viewport(800.0, 600.0);
            vp.set_zoom(zoom, vp.center(), document());
            let doc = CGPoint::new(137.5, 42.25);
            let view = vp.view_point(doc, document());
            let back = vp.document_point(view, document());
            assert!((doc.x - back.x).abs() < 1e-6, "zoom {zoom}");
            assert!((doc.y - back.y).abs() < 1e-6, "zoom {zoom}");
        }
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
    fn resize_clamps_backing_scale_at_one() {
        let mut vp = viewport(800.0, 600.0);
        vp.resize(CGSize::new(800.0, 600.0), 0.5, None);
        assert_eq!(vp.backing_scale, 1.0);
    }
}
