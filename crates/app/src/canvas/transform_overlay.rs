//! The transform box's geometry (`TransformOverlay.swift`): its eight
//! handles and rotation handle in view space, and the press hit-test over
//! them. The drawing half, the distortion's corners variant
//! (`init(corners:...)`) and `resizeCursor` (an `NSCursor` concern) stay
//! unported until the canvas renders and drives transform handles.

use compositor_core::geometry::CGPoint;
use compositor_core::transform::{DragMode, HANDLES, LayerTransform};

/// Port of `TransformOverlayGeometry` (TransformOverlay.swift:3-49).
///
/// Not wired into presses yet: the canvas has no resize/rotate/drag state
/// to drive, so its presses go through `transform_press_layer` alone.
/// Kept in its ported shape until that wiring lands — ticket 15's per-item
/// exemption (deliberately kept, no caller yet).
#[allow(dead_code)]
pub(crate) struct TransformOverlayGeometry {
    pub(crate) handles: [CGPoint; 8],
    pub(crate) rotation_handle: CGPoint,
    /// A distortion has no single rotation, so its rotation handle is
    /// hidden; the ordinary transform always shows it.
    pub(crate) shows_rotation: bool,
}

/// Same exemption as the struct: [`TransformOverlayGeometry::hit`] lands
/// with the transform-handle press wiring.
#[allow(dead_code)]
impl TransformOverlayGeometry {
    /// `init(transform:viewport:documentSize:)`: the handles at the
    /// transform's corners and edge midpoints in view space. `zoom`/`pan`
    /// are the canvas viewport's document-to-view mapping — the affine
    /// `CanvasViewport::view_point` applies (Swift
    /// `CanvasViewport.viewPoint(from:documentSize:)`); this unwired
    /// geometry keeps the caller-supplied zoom/pan pair until the
    /// transform-handle ticket wires it to the session's viewport.
    pub(crate) fn new(transform: LayerTransform, zoom: f64, pan: (f64, f64)) -> Self {
        let view = |p: CGPoint| CGPoint::new(p.x * zoom + pan.0, p.y * zoom + pan.1);
        let handles: [CGPoint; 8] = HANDLES.map(|unit| view(transform.point(unit)));
        // The rotation handle hangs off the top edge midpoint along the
        // box's rotated up (:11-12).
        let rotation_handle = CGPoint::new(
            handles[1].x + transform.radians().sin() * 28.0,
            handles[1].y - transform.radians().cos() * 28.0,
        );
        Self {
            handles,
            rotation_handle,
            shows_rotation: true,
        }
    }

    /// The drag a press starts at a view-space point (`hit`): the rotation
    /// handle first, then the handles, then the four edges — whose whole
    /// span drags that edge's midpoint handle (:26-41).
    pub(crate) fn hit(&self, point: CGPoint) -> Option<DragMode> {
        const RADIUS: f64 = 10.0;
        let near = |other: CGPoint| (point.x - other.x).hypot(point.y - other.y) <= RADIUS;
        if self.shows_rotation && near(self.rotation_handle) {
            return Some(DragMode::Rotate);
        }
        if let Some(index) = self.handles.iter().position(|h| near(*h)) {
            return Some(DragMode::Resize(index));
        }
        for (start, end, handle) in [(0, 2, 1), (2, 4, 3), (4, 6, 5), (6, 0, 7)] {
            let a = self.handles[start];
            let b = self.handles[end];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let length_squared = dx * dx + dy * dy;
            if length_squared <= 0.0 {
                continue;
            }
            let t = ((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared;
            if (0.0..=1.0).contains(&t)
                && (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy) <= RADIUS
            {
                return Some(DragMode::Resize(handle));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::TransformOverlayGeometry;
    use compositor_core::geometry::{CGPoint, CGSize};
    use compositor_core::transform::{DragMode, LayerSampling, LayerTransform};

    /// `hoverRegionsMatchRotatedEdgesCornersAndRotationHandle`
    /// (TransformTests.swift:110-128): on a rotated box the whole edge
    /// spans hit the edge's midpoint handle, the corners hit their own
    /// handle, the rotation handle hits rotate, and the box's center hits
    /// nothing. A fresh viewport showing a same-size document maps 1:1
    /// (`CanvasViewport.resize` fits it at zoom 1, zero pan).
    #[test]
    fn hover_regions_match_rotated_edges_corners_and_rotation_handle() {
        let transform = LayerTransform {
            origin: CGPoint::new(100.0, 100.0),
            size: CGSize::new(400.0, 300.0),
            rotation: 37.0,
            flip_x: false,
            flip_y: false,
            sampling: LayerSampling::High,
        };
        let geometry = TransformOverlayGeometry::new(transform, 1.0, (0.0, 0.0));
        for (start, end, expected) in [(0, 2, 1), (2, 4, 3), (4, 6, 5), (6, 0, 7)] {
            let (a, b) = (geometry.handles[start], geometry.handles[end]);
            let point = CGPoint::new(a.x * 0.75 + b.x * 0.25, a.y * 0.75 + b.y * 0.25);
            match geometry.hit(point) {
                Some(DragMode::Resize(index)) => assert_eq!(index, expected),
                _ => panic!("rotated edge was not recognized"),
            }
        }
        for index in [0, 2, 4, 6] {
            match geometry.hit(geometry.handles[index]) {
                Some(DragMode::Resize(hit)) => assert_eq!(hit, index),
                _ => panic!("corner was not recognized"),
            }
        }
        assert!(
            matches!(
                geometry.hit(geometry.rotation_handle),
                Some(DragMode::Rotate)
            ),
            "rotation handle was not recognized"
        );
        assert!(
            geometry.hit(transform.center()).is_none(),
            "the box's center starts no drag"
        );
    }
}
