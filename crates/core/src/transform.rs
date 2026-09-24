//! Port of `Compositor/Document/LayerTransform.swift`.

use crate::geometry::{rounded, CGFloat, CGAffineTransform, CGPoint, CGSize, PI};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerSampling {
    #[serde(rename = "Nearest")]
    Nearest,
    #[serde(rename = "Smooth")]
    Smooth,
    /// Swift's `LayerTransform.sampling` defaults to `.high`
    /// (LayerTransform.swift:24); the manifest default agrees.
    #[default]
    #[serde(rename = "High quality")]
    High,
}

impl LayerSampling {
    pub const ALL: [LayerSampling; 3] = [LayerSampling::Nearest, LayerSampling::Smooth, LayerSampling::High];

    pub fn label(&self) -> &'static str {
        match self {
            LayerSampling::Nearest => "Nearest",
            LayerSampling::Smooth => "Smooth",
            LayerSampling::High => "High quality",
        }
    }
}

/// Unrotated bounds in document pixels; rotation is clockwise around their center.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayerTransform {
    pub origin: CGPoint,
    pub size: CGSize,
    #[serde(default)]
    pub rotation: CGFloat,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    #[serde(default)]
    pub sampling: LayerSampling,
}

impl Default for LayerTransform {
    fn default() -> Self {
        LayerTransform {
            origin: CGPoint::ZERO,
            size: CGSize::ZERO,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            sampling: LayerSampling::High,
        }
    }
}

/// The eight transform handles in unit coordinates (corners and edge midpoints).
pub const HANDLES: [CGPoint; 8] = [
    CGPoint { x: 0.0, y: 0.0 },
    CGPoint { x: 0.5, y: 0.0 },
    CGPoint { x: 1.0, y: 0.0 },
    CGPoint { x: 1.0, y: 0.5 },
    CGPoint { x: 1.0, y: 1.0 },
    CGPoint { x: 0.5, y: 1.0 },
    CGPoint { x: 0.0, y: 1.0 },
    CGPoint { x: 0.0, y: 0.5 },
];

impl LayerTransform {
    pub fn new(origin: CGPoint, size: CGSize) -> Self {
        LayerTransform {
            origin,
            size,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            sampling: LayerSampling::High,
        }
    }

    pub fn center(&self) -> CGPoint {
        CGPoint::new(self.origin.x + self.size.width / 2.0, self.origin.y + self.size.height / 2.0)
    }

    pub fn radians(&self) -> CGFloat {
        (self.rotation % 360.0) * PI / 180.0
    }

    pub fn is_valid(&self) -> bool {
        [self.origin.x, self.origin.y, self.size.width, self.size.height, self.rotation]
            .iter()
            .all(|v| v.is_finite())
            && (1.0..=300_000.0).contains(&self.size.width)
            && (1.0..=300_000.0).contains(&self.size.height)
            && self.origin.x.abs() <= 1_000_000.0
            && self.origin.y.abs() <= 1_000_000.0
    }

    /// Where the unit point (`0…1` over the box) lands on the document.
    pub fn point(&self, unit: CGPoint) -> CGPoint {
        let x = (unit.x - 0.5) * self.size.width;
        let y = (unit.y - 0.5) * self.size.height;
        let r = self.radians();
        let (sr, cr) = r.sin_cos();
        CGPoint::new(
            self.center().x + x * cr - y * sr,
            self.center().y + x * sr + y * cr,
        )
    }

    pub fn contains(&self, p: CGPoint) -> bool {
        let x = p.x - self.center().x;
        let y = p.y - self.center().y;
        let r = self.radians();
        let (sr, cr) = r.sin_cos();
        (x * cr + y * sr).abs() <= self.size.width / 2.0 && (-x * sr + y * cr).abs() <= self.size.height / 2.0
    }

    /// Width as a percentage of the `pixel_size` it places (100% draws them 1:1).
    pub fn scale_percent(&self, pixel_size: CGSize) -> CGFloat {
        self.size.width / pixel_size.width.max(1.0) * 100.0
    }

    /// Both sides set to `percent` of `pixel_size`, keeping the center (and rotation and flips).
    pub fn scaled(&self, percent: CGFloat, pixel_size: CGSize) -> LayerTransform {
        let mut result = *self;
        result.size = CGSize::new(pixel_size.width * percent / 100.0, pixel_size.height * percent / 100.0);
        result.origin = CGPoint::new(
            self.center().x - result.size.width / 2.0,
            self.center().y - result.size.height / 2.0,
        );
        result
    }

    /// Whole pixels and whole degrees: what dragging, scaling and rotating leave behind.
    pub fn rounded(&self) -> LayerTransform {
        let mut result = *self;
        result.origin = CGPoint::new(rounded(self.origin.x), rounded(self.origin.y));
        result.size = CGSize::new(rounded(self.size.width).max(1.0), rounded(self.size.height).max(1.0));
        result.rotation = rounded(self.rotation);
        result
    }

    /// The unit square (0…1, y down) mapped where this transform places a layer on the document.
    pub fn unit_to_document(&self) -> CGAffineTransform {
        crate::raster::pixel_to_document(self, 1.0, 1.0)
    }

    /// A transform placing the unit square as `map` does — a rotated, maybe flipped rectangle.
    /// Keeps this transform's sampling.
    pub fn placing(&self, map: &CGAffineTransform) -> LayerTransform {
        let sign: CGFloat = if self.flip_x { -1.0 } else { 1.0 };
        let angle = (map.b * sign).atan2(map.a * sign);
        let along = -map.c * angle.sin() + map.d * angle.cos();
        let middle = CGPoint::new(0.5, 0.5).applying(map);
        let mut result = *self;
        result.size = CGSize::new(map.a.hypot(map.b), along.abs());
        let degrees = angle * 180.0 / PI;
        result.rotation = degrees + ((self.rotation - degrees) / 360.0).round() * 360.0;
        result.flip_y = along < 0.0;
        result.origin = CGPoint::new(middle.x - result.size.width / 2.0, middle.y - result.size.height / 2.0);
        result
    }

    /// This placement carried along as a layer moves from `old` to `new`.
    pub fn following(&self, old: &LayerTransform, new: &LayerTransform) -> LayerTransform {
        if old == new {
            return *self;
        }
        // A plain move carries exactly.
        if old.size == new.size
            && old.rotation == new.rotation
            && old.flip_x == new.flip_x
            && old.flip_y == new.flip_y
        {
            let mut moved = *self;
            moved.origin.x += new.origin.x - old.origin.x;
            moved.origin.y += new.origin.y - old.origin.y;
            return moved;
        }
        self.placing(
            &self
                .unit_to_document()
                .concat(&old.unit_to_document().inverted())
                .concat(&new.unit_to_document()),
        )
    }

    /// The same place on the document, whatever the sampling.
    pub fn same_placement(&self, other: &LayerTransform) -> bool {
        let mut copy = *self;
        copy.sampling = other.sampling;
        copy == *other
    }

    /// Corners of the box in document pixels, in `HANDLES` order (0, 2, 4, 6).
    pub fn corners(&self) -> [CGPoint; 4] {
        [
            self.point(CGPoint::new(0.0, 0.0)),
            self.point(CGPoint::new(1.0, 0.0)),
            self.point(CGPoint::new(1.0, 1.0)),
            self.point(CGPoint::new(0.0, 1.0)),
        ]
    }
}

/// Several layers transformed together: the upright box around them when the edit began (what
/// the draft edits), and each one's transform then.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformGroup {
    pub box_: LayerTransform,
    pub originals: Vec<(uuid::Uuid, LayerTransform)>,
}

impl TransformGroup {
    pub fn original(&self, id: uuid::Uuid) -> Option<&LayerTransform> {
        self.originals.iter().find(|(k, _)| *k == id).map(|(_, v)| v)
    }
}

/// Port of `TransformEdit`.
#[derive(Clone)]
pub struct TransformEdit {
    pub layer_id: uuid::Uuid,
    pub draft: LayerTransform,
    pub persistent: bool,
    /// Set when transforming selected pixels (Cmd-T with a selection) rather than a layer.
    pub floating: Option<crate::selection::FloatingTransform>,
    /// Set once a handle is Cmd-dragged: the four corners (document pixels, handle order) move
    /// freely, and Apply resamples the pixels into that shape.
    pub corners: Option<[CGPoint; 4]>,
    /// Set when an unlinked mask is selected: the edit places the mask alone (its `placement`).
    pub mask: bool,
    /// Set when several layers are selected: the draft is the box around them all, and each follows it.
    pub group: Option<TransformGroup>,
}

/// Port of `TransformDrag`.
#[derive(Clone)]
pub enum DragMode {
    Move,
    Resize(usize),
    Rotate,
    Distort(usize),
}

pub struct TransformDrag {
    pub original: LayerTransform,
    pub start: CGPoint,
    pub mode: DragMode,
    /// The distortion's corners when the drag began; None for an ordinary transform.
    pub original_corners: Option<[CGPoint; 4]>,
}

impl TransformDrag {
    /// Corners after dragging to `point`: a corner handle moves its corner, an edge handle both of
    /// that edge's corners, and the body the whole shape. None when the drag isn't distorting.
    pub fn corners_at(&self, point: CGPoint, shift: bool) -> Option<[CGPoint; 4]> {
        let mut result = self.original_corners?;
        let mut dx = point.x - self.start.x;
        let mut dy = point.y - self.start.y;
        // Shift keeps what's being dragged on one axis.
        if shift {
            if dx.abs() >= dy.abs() {
                dy = 0.0;
            } else {
                dx = 0.0;
            }
        }
        let moved: Vec<usize> = match self.mode {
            DragMode::Distort(index) => {
                if index % 2 == 0 {
                    vec![index / 2]
                } else {
                    vec![index / 2, (index / 2 + 1) % 4]
                }
            }
            DragMode::Move => vec![0, 1, 2, 3],
            _ => return None,
        };
        for corner in moved {
            result[corner].x += dx;
            result[corner].y += dy;
        }
        Some(result)
    }

    pub fn updated(
        &self,
        point: CGPoint,
        lock_ratio: bool,
        shift: bool,
        option: bool,
    ) -> LayerTransform {
        let mut result = self.original;
        match self.mode {
            DragMode::Distort(_) => {}
            DragMode::Move => {
                let mut dx = point.x - self.start.x;
                let mut dy = point.y - self.start.y;
                if shift {
                    if dx.abs() >= dy.abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                result.origin.x += dx;
                result.origin.y += dy;
            }
            DragMode::Rotate => {
                let center = self.original.center();
                let delta = (point.y - center.y).atan2(point.x - center.x)
                    - (self.start.y - center.y).atan2(self.start.x - center.x);
                result.rotation += delta * 180.0 / PI;
                if shift {
                    result.rotation = (result.rotation / 15.0).round() * 15.0;
                }
            }
            DragMode::Resize(index) => {
                let handle = HANDLES[index];
                let anchor_unit = if option {
                    CGPoint::new(0.5, 0.5)
                } else {
                    CGPoint::new(1.0 - handle.x, 1.0 - handle.y)
                };
                let anchor = self.original.point(anchor_unit);
                // Use the initial handle plus pointer delta to avoid a jump on grab.
                let initial_handle = self.original.point(handle);
                let dx = initial_handle.x + point.x - self.start.x - anchor.x;
                let dy = initial_handle.y + point.y - self.start.y - anchor.y;
                // Center-to-handle distances cover half the size on each axis.
                let span: CGFloat = if option { 2.0 } else { 1.0 };
                let r = self.original.radians();
                let (sr, cr) = r.sin_cos();
                let local_x = (dx * cr + dy * sr) * span;
                let local_y = (-dx * sr + dy * cr) * span;
                let sx = handle.x * 2.0 - 1.0;
                let sy = handle.y * 2.0 - 1.0;
                // Dragging a handle past the opposite side turns the layer over rather than
                // stopping at nothing: the size stays positive and the layer is flipped on that
                // axis, as a negative scale would.
                let raw_width = if sx == 0.0 { self.original.size.width } else { local_x * sx };
                let raw_height = if sy == 0.0 { self.original.size.height } else { local_y * sy };
                let mirrored_x = raw_width < 0.0;
                let mirrored_y = raw_height < 0.0;
                let mut width = raw_width.abs().max(1.0);
                let mut height = raw_height.abs().max(1.0);
                if lock_ratio != shift {
                    let factor: CGFloat = if sx == 0.0 {
                        height / self.original.size.height
                    } else if sy == 0.0 {
                        width / self.original.size.width
                    } else {
                        // Project onto the original diagonal for proportional scaling.
                        (1.0 / self.original.size.width.min(self.original.size.height))
                            .max(
                                (local_x * sx * self.original.size.width
                                    + local_y * sy * self.original.size.height)
                                    / (self.original.size.width * self.original.size.width
                                        + self.original.size.height * self.original.size.height),
                            )
                    };
                    width = self.original.size.width * factor;
                    height = self.original.size.height * factor;
                }
                result.size = CGSize::new(width, height);
                if mirrored_x {
                    result.flip_x = !result.flip_x;
                }
                if mirrored_y {
                    result.flip_y = !result.flip_y;
                }
                // Turned over, the box lies on the other side of the anchor.
                let offset_x = (0.5 - anchor_unit.x) * width * if mirrored_x { -1.0 } else { 1.0 };
                let offset_y = (0.5 - anchor_unit.y) * height * if mirrored_y { -1.0 } else { 1.0 };
                let center = CGPoint::new(
                    anchor.x + offset_x * cr - offset_y * sr,
                    anchor.y + offset_x * sr + offset_y * cr,
                );
                result.origin = CGPoint::new(center.x - width / 2.0, center.y - height / 2.0);
            }
        }
        if result.is_valid() {
            result
        } else {
            self.original
        }
    }
}

/// Port of `TransformSnap`.
pub struct TransformSnap;

impl TransformSnap {
    /// How close, in screen points, a guide comes before it snaps.
    pub const DISTANCE: CGFloat = 10.0;

    /// `box` moved so that whichever of its left, center or right lands nearest an `xs` target
    /// does, and the same vertically — each axis on its own, and only within `tolerance`
    /// document pixels. The targets it landed on come back too, to draw a line along.
    pub fn offset(
        box_: &crate::geometry::CGRect,
        xs: &[CGFloat],
        ys: &[CGFloat],
        tolerance: CGFloat,
    ) -> (crate::geometry::CGSize, Option<CGFloat>, Option<CGFloat>) {
        let horizontal = Self::shift(&[box_.min_x(), box_.mid_x(), box_.max_x()], xs, tolerance);
        let vertical = Self::shift(&[box_.min_y(), box_.mid_y(), box_.max_y()], ys, tolerance);
        (
            crate::geometry::CGSize::new(horizontal.0, vertical.0),
            horizontal.1,
            vertical.1,
        )
    }

    /// The smallest move that puts one of `guides` on one of `targets`, and the target it met.
    fn shift(guides: &[CGFloat], targets: &[CGFloat], tolerance: CGFloat) -> (CGFloat, Option<CGFloat>) {
        let mut best: Option<(CGFloat, CGFloat)> = None;
        for &guide_value in guides {
            for &target in targets {
                let mv = target - guide_value;
                if mv.abs() > tolerance {
                    continue;
                }
                if let Some(current) = best {
                    if current.0.abs() <= mv.abs() {
                        continue;
                    }
                }
                best = Some((mv, target));
            }
        }
        (best.map(|b| b.0).unwrap_or(0.0), best.map(|b| b.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_and_points() {
        let t = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 50.0));
        assert_eq!(t.center(), CGPoint::new(50.0, 25.0));
        assert_eq!(t.point(CGPoint::new(0.0, 0.0)), CGPoint::new(0.0, 0.0));
        assert_eq!(t.point(CGPoint::new(1.0, 1.0)), CGPoint::new(100.0, 50.0));
    }

    #[test]
    fn rotated_point() {
        let mut t = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        t.rotation = 90.0;
        // Rotation is clockwise in y-down space: top-left (0,0) goes to top-right corner.
        let p = t.point(CGPoint::new(0.0, 0.0));
        assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 0.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn contains_rotated() {
        let mut t = LayerTransform::new(CGPoint::new(10.0, 10.0), CGSize::new(100.0, 100.0));
        t.rotation = 45.0;
        assert!(t.contains(CGPoint::new(60.0, 10.0)));
        assert!(!t.contains(CGPoint::new(15.0, 15.0)));
    }

    #[test]
    fn scale_keeps_center() {
        let mut t = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 50.0));
        t.rotation = 30.0;
        let s = t.scaled(50.0, CGSize::new(100.0, 50.0));
        assert_eq!(s.center(), t.center());
        assert_eq!(s.size, CGSize::new(50.0, 25.0));
        assert_eq!(t.scale_percent(CGSize::new(100.0, 50.0)), 100.0);
    }

    #[test]
    fn plain_move_carries_masks() {
        let old = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(10.0, 10.0));
        let mut new = old;
        new.origin = CGPoint::new(5.0, 7.0);
        let placement = LayerTransform::new(CGPoint::new(1.0, 2.0), CGSize::new(5.0, 5.0));
        let moved = placement.following(&old, &new);
        assert_eq!(moved.origin, CGPoint::new(6.0, 9.0));
        assert_eq!(moved.size, placement.size);
    }

    #[test]
    fn resize_drag_ratio_lock() {
        let original = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        let drag = TransformDrag {
            original,
            start: original.point(CGPoint::new(1.0, 1.0)),
            mode: DragMode::Resize(4),
            original_corners: None,
        };
        // Drag bottom-right handle to 150,100: with ratio locked both sides scale together.
        let result = drag.updated(CGPoint::new(150.0, 100.0), true, false, false);
        assert!((result.size.width - result.size.height).abs() < 1e-6);
        assert!(result.size.width > 100.0);
    }

    #[test]
    fn rotate_snap_to_15_with_shift() {
        let original = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        let center = original.center();
        let start = CGPoint::new(center.x + 100.0, center.y);
        let drag = TransformDrag {
            original,
            start,
            mode: DragMode::Rotate,
            original_corners: None,
        };
        // 20° clockwise.
        let p = CGPoint::new(
            center.x + 100.0 * (20.0_f64 * PI / 180.0).cos(),
            center.y + 100.0 * (20.0_f64 * PI / 180.0).sin(),
        );
        let result = drag.updated(p, true, true, false);
        assert_eq!(result.rotation, 15.0);
    }

    #[test]
    fn snap_offset_finds_nearest() {
        let box_ = crate::geometry::CGRect::new(0.0, 0.0, 10.0, 10.0);
        let (offset, x, _y) = TransformSnap::offset(&box_, &[4.0], &[100.0], 5.0);
        // The box's center (5) is nearest the guide (4): it moves -1.
        assert_eq!(offset.width, -1.0);
        assert_eq!(x, Some(4.0));
    }

    #[test]
    fn following_via_placing_survives_scale() {
        let old = LayerTransform::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        let mut new = LayerTransform::new(CGPoint::new(50.0, 50.0), CGSize::new(200.0, 200.0));
        new.rotation = 45.0;
        let placement = LayerTransform::new(CGPoint::new(10.0, 20.0), CGSize::new(40.0, 40.0));
        let moved = placement.following(&old, &new);
        assert!(moved.is_valid());
        // `following` rebuilds the placement through the composite
        // place * oldInverse * new (`placing`), so its center is
        // place(oldInverse(new(0.5, 0.5))) and its size doubles with the box.
        assert!((moved.size.width - 80.0).abs() < 1e-6, "{:?}", moved.size);
        assert!((moved.size.height - 80.0).abs() < 1e-6);
        let center = moved.center();
        assert!((center.x - 70.0).abs() < 1e-6 && (center.y - 80.0).abs() < 1e-6, "{center:?}");
        // The rotation follows the box's.
        assert!(((moved.rotation - 45.0).abs() < 1e-6), "{}", moved.rotation);
        // A plain move carries exactly (covered elsewhere); a scale+rotate is
        // reconstructed without shear.
        assert!(!moved.flip_x && !moved.flip_y);
    }
}
