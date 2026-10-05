//! CoreGraphics geometry primitives, ported 1:1 (`CGPoint`, `CGSize`, `CGRect`,
//! `CGAffineTransform`). Coordinates are `f64` (Swift's `CGFloat`).

use serde::{Deserialize, Serialize};

pub type CGFloat = f64;

pub const PI: CGFloat = std::f64::consts::PI;

/// Swift's `.rounded()` rounds half away from zero; Rust's `f64::round` does the same.
pub fn rounded(v: CGFloat) -> CGFloat {
    v.round()
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CGPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

impl CGPoint {
    pub const ZERO: CGPoint = CGPoint { x: 0.0, y: 0.0 };

    pub fn new(x: CGFloat, y: CGFloat) -> Self {
        Self { x, y }
    }

    pub fn hypot(a: CGFloat, b: CGFloat) -> CGFloat {
        a.hypot(b)
    }

    pub fn distance_to(self, other: CGPoint) -> CGFloat {
        (self.x - other.x).hypot(self.y - other.y)
    }

    /// `CGPoint.applying(_:)`
    pub fn applying(self, t: &CGAffineTransform) -> CGPoint {
        CGPoint {
            x: t.a * self.x + t.c * self.y + t.tx,
            y: t.b * self.x + t.d * self.y + t.ty,
        }
    }
}

impl std::ops::Add for CGPoint {
    type Output = CGPoint;
    fn add(self, o: CGPoint) -> CGPoint {
        CGPoint::new(self.x + o.x, self.y + o.y)
    }
}

impl std::ops::Sub for CGPoint {
    type Output = CGPoint;
    fn sub(self, o: CGPoint) -> CGPoint {
        CGPoint::new(self.x - o.x, self.y - o.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CGSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

impl CGSize {
    pub const ZERO: CGSize = CGSize {
        width: 0.0,
        height: 0.0,
    };

    pub fn new(width: CGFloat, height: CGFloat) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

impl Default for CGRect {
    fn default() -> Self {
        CGRect::NULL
    }
}

impl CGRect {
    pub const NULL: CGRect = CGRect {
        origin: CGPoint {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
    pub const ZERO: CGRect = CGRect {
        origin: CGPoint::ZERO,
        size: CGSize::ZERO,
    };

    pub fn new(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) -> Self {
        Self {
            origin: CGPoint::new(x, y),
            size: CGSize::new(width, height),
        }
    }

    pub fn from_origin_size(origin: CGPoint, size: CGSize) -> Self {
        Self { origin, size }
    }

    pub fn is_null(&self) -> bool {
        self.origin.x.is_infinite() || self.origin.y.is_infinite()
    }

    pub fn is_empty(&self) -> bool {
        self.size.width <= 0.0 || self.size.height <= 0.0
    }

    pub fn is_infinite(&self) -> bool {
        self.origin.x == f64::NEG_INFINITY
            || self.origin.y == f64::NEG_INFINITY
            || self.size.width == f64::INFINITY
            || self.size.height == f64::INFINITY
    }

    pub fn min_x(&self) -> CGFloat {
        self.origin.x
    }
    pub fn min_y(&self) -> CGFloat {
        self.origin.y
    }
    pub fn mid_x(&self) -> CGFloat {
        self.origin.x + self.size.width / 2.0
    }
    pub fn mid_y(&self) -> CGFloat {
        self.origin.y + self.size.height / 2.0
    }
    pub fn max_x(&self) -> CGFloat {
        self.origin.x + self.size.width
    }
    pub fn max_y(&self) -> CGFloat {
        self.origin.y + self.size.height
    }
    pub fn width(&self) -> CGFloat {
        self.size.width
    }
    pub fn height(&self) -> CGFloat {
        self.size.height
    }

    pub fn contains_point(&self, p: CGPoint) -> bool {
        !self.is_null()
            && p.x >= self.min_x()
            && p.x < self.max_x()
            && p.y >= self.min_y()
            && p.y < self.max_y()
    }

    /// `CGRect.intersection` (returns NULL when they don't overlap).
    pub fn intersection(&self, other: &CGRect) -> CGRect {
        if self.is_null() || other.is_null() {
            return CGRect::NULL;
        }
        let x0 = self.min_x().max(other.min_x());
        let x1 = self.max_x().min(other.max_x());
        let y0 = self.min_y().max(other.min_y());
        let y1 = self.max_y().min(other.max_y());
        if x1 < x0 || y1 < y0 {
            return CGRect::NULL;
        }
        CGRect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// `CGRect.union`
    pub fn union(&self, other: &CGRect) -> CGRect {
        if self.is_null() {
            return *other;
        }
        if other.is_null() {
            return *self;
        }
        let x0 = self.min_x().min(other.min_x());
        let y0 = self.min_y().min(other.min_y());
        let x1 = self.max_x().max(other.max_x());
        let y1 = self.max_y().max(other.max_y());
        CGRect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// `CGRect.integral`: the smallest integer rect containing self.
    pub fn integral(&self) -> CGRect {
        if self.is_null() {
            return CGRect::NULL;
        }
        let x0 = self.min_x().floor();
        let y0 = self.min_y().floor();
        let x1 = self.max_x().ceil();
        let y1 = self.max_y().ceil();
        CGRect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn inset_by(&self, dx: CGFloat, dy: CGFloat) -> CGRect {
        if self.is_null() {
            return CGRect::NULL;
        }
        let w = self.size.width - dx * 2.0;
        let h = self.size.height - dy * 2.0;
        if w < 0.0 || h < 0.0 {
            return CGRect::NULL;
        }
        CGRect::new(self.origin.x + dx, self.origin.y + dy, w, h)
    }

    pub fn offset_by(&self, dx: CGFloat, dy: CGFloat) -> CGRect {
        let mut r = *self;
        r.origin = r.origin + CGPoint::new(dx, dy);
        r
    }

    pub fn contains(&self, other: &CGRect) -> bool {
        other.intersection(self) == *other
    }

    /// `CGRect.applying(_:)` — the rect's corners through the transform, boxed.
    pub fn transformed(&self, t: &CGAffineTransform) -> CGRect {
        if self.is_null() {
            return CGRect::NULL;
        }
        let corners = [
            CGPoint::new(self.min_x(), self.min_y()).applying(t),
            CGPoint::new(self.max_x(), self.min_y()).applying(t),
            CGPoint::new(self.max_x(), self.max_y()).applying(t),
            CGPoint::new(self.min_x(), self.max_y()).applying(t),
        ];
        let min_x = corners[0]
            .x
            .min(corners[1].x)
            .min(corners[2].x)
            .min(corners[3].x);
        let max_x = corners[0]
            .x
            .max(corners[1].x)
            .max(corners[2].x)
            .max(corners[3].x);
        let min_y = corners[0]
            .y
            .min(corners[1].y)
            .min(corners[2].y)
            .min(corners[3].y);
        let max_y = corners[0]
            .y
            .max(corners[1].y)
            .max(corners[2].y)
            .max(corners[3].y);
        CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }
}

/// `CGAffineTransform`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CGAffineTransform {
    pub a: CGFloat,
    pub b: CGFloat,
    pub c: CGFloat,
    pub d: CGFloat,
    pub tx: CGFloat,
    pub ty: CGFloat,
}

impl Default for CGAffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl CGAffineTransform {
    pub const IDENTITY: CGAffineTransform = CGAffineTransform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub fn translation(x: CGFloat, y: CGFloat) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: x,
            ty: y,
        }
    }

    pub fn scale(sx: CGFloat, sy: CGFloat) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// `CGAffineTransform(rotationAngle:)` — rotates clockwise in a y-down coordinate system.
    pub fn rotation(angle: CGFloat) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// `CGAffineTransform.concat` / `concatenating`: the result applies
    /// `other` to the point first, then `self` (`p' = M_self · M_other · p`),
    /// exactly as Core Graphics does. This makes the builder chain
    /// `t.translate(..).rotated_by(..)` apply each new operation *before* the
    /// accumulated transform, matching `CGAffineTransformTranslate` & friends.
    pub fn concat(&self, other: &CGAffineTransform) -> CGAffineTransform {
        CGAffineTransform {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            tx: self.a * other.tx + self.c * other.ty + self.tx,
            ty: self.b * other.tx + self.d * other.ty + self.ty,
        }
    }

    pub fn inverted(&self) -> CGAffineTransform {
        let det = self.a * self.d - self.b * self.c;
        if det == 0.0 {
            return *self;
        }
        let inv_det = 1.0 / det;
        let a = self.d * inv_det;
        let b = -self.b * inv_det;
        let c = -self.c * inv_det;
        let d = self.a * inv_det;
        CGAffineTransform {
            a,
            b,
            c,
            d,
            tx: -(self.tx * a + self.ty * c),
            ty: -(self.tx * b + self.ty * d),
        }
    }

    pub fn translate(&self, x: CGFloat, y: CGFloat) -> CGAffineTransform {
        self.concat(&CGAffineTransform::translation(x, y))
    }

    pub fn scale_by(&self, sx: CGFloat, sy: CGFloat) -> CGAffineTransform {
        self.concat(&CGAffineTransform::scale(sx, sy))
    }

    pub fn rotated_by(&self, angle: CGFloat) -> CGAffineTransform {
        self.concat(&CGAffineTransform::rotation(angle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_intersection_null_when_disjoint() {
        let a = CGRect::new(0.0, 0.0, 10.0, 10.0);
        let b = CGRect::new(20.0, 20.0, 5.0, 5.0);
        assert!(a.intersection(&b).is_null());
        let c = CGRect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersection(&c), CGRect::new(5.0, 5.0, 5.0, 5.0));
    }

    #[test]
    fn rect_union_covers_both() {
        let a = CGRect::new(0.0, 0.0, 10.0, 10.0);
        let b = CGRect::new(20.0, 20.0, 5.0, 5.0);
        assert_eq!(a.union(&b), CGRect::new(0.0, 0.0, 25.0, 25.0));
    }

    #[test]
    fn rect_integral_expands() {
        let a = CGRect::new(0.5, 0.5, 2.2, 2.2);
        assert_eq!(a.integral(), CGRect::new(0.0, 0.0, 3.0, 3.0));
    }

    #[test]
    fn affine_inverts_roundtrip() {
        let t = CGAffineTransform::translation(10.0, 20.0)
            .rotated_by(0.7)
            .scale_by(2.0, 3.0);
        let p = CGPoint::new(3.0, 4.0);
        let q = p.applying(&t).applying(&t.inverted());
        assert!((q.x - p.x).abs() < 1e-9 && (q.y - p.y).abs() < 1e-9);
    }

    #[test]
    fn point_applying_translation() {
        let t = CGAffineTransform::translation(5.0, -3.0);
        assert_eq!(CGPoint::new(1.0, 1.0).applying(&t), CGPoint::new(6.0, -2.0));
    }
}
