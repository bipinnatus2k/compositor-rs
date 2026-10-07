//! Port of the Curves control surface (`CurvesControls.swift`): the curve
//! canvas — hit-testing, drag begin/move, point insert and remove — plus
//! the gpui canvas view that paints and drags the points.

use gpui_kit::{
    Bounds, Context, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    Pixels, Point, Styled, Window, canvas, point, px, rgba, size,
};

use compositor_core::adjust::{CurvePoint, CurvesSettings, LevelsChannel};

use crate::state::AppState;

/// Starting a drag on the curve canvas (`CurvesControls.swift`'s
/// `dragging == nil` branch).
#[derive(Debug, PartialEq)]
pub enum CurveDragBegin {
    /// The click landed on an existing point; drag it.
    Grab(usize),
    /// The click landed on empty canvas; a point was inserted (kept sorted
    /// by x) and the drag moves it.
    Added {
        points: Vec<CurvePoint>,
        index: usize,
    },
    /// Nothing to drag: the 32-point cap, the x range, or a blocked x.
    None,
}

impl AppState {
    // MARK: Curves canvas (CurvesControls.swift's DragGesture)

    /// The channel picker: switching clears the selection, exactly as the
    /// Swift `onChange(of: settings.channel)` does.
    pub fn curves_set_channel(&mut self, channel: LevelsChannel, cx: &mut Context<Self>) {
        if let Some(editor) = self.curves_editor.as_mut() {
            editor.settings.channel = channel;
            editor.selected = None;
            editor.dragging = None;
        }
        self.notify_only(cx);
    }

    /// Pointer down on the curve canvas: grab the nearest point, or add a
    /// new one on empty canvas, then start dragging it.
    pub fn curves_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_mut() else {
            return;
        };
        let (x, y) = curve_point_from_event(event.position, editor.canvas_bounds);
        let channel = editor.settings.channel.index();
        let points = editor.settings.channels[channel].clone();
        match curve_drag_begin(&points, x, y) {
            CurveDragBegin::None => editor.dragging = None,
            CurveDragBegin::Grab(index) => {
                editor.dragging = Some(index);
                editor.selected = Some(index);
            }
            CurveDragBegin::Added { points, index } => {
                editor.settings.channels[channel] = points;
                editor.dragging = Some(index);
                editor.selected = Some(index);
            }
        }
        self.curves_drag_to(x, y, cx);
    }

    /// Pointer move while a drag is live: the point follows, the preview
    /// re-renders.
    pub fn curves_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_ref() else {
            return;
        };
        if editor.dragging.is_none() {
            return;
        }
        let (x, y) = curve_point_from_event(event.position, editor.canvas_bounds);
        self.curves_drag_to(x, y, cx);
    }

    /// Pointer up: the drag ends (the selection stays).
    pub fn curves_mouse_up(&mut self, _event: &MouseUpEvent, cx: &mut Context<Self>) {
        let was_dragging = self
            .curves_editor
            .as_mut()
            .map(|editor| editor.dragging.take().is_some())
            .unwrap_or(false);
        if was_dragging {
            self.notify_only(cx);
        }
    }

    /// The "Remove point" button: interior points only, as in Swift
    /// (`selected > 0 && selected < points.count-1`).
    pub fn curves_remove_point(&mut self, cx: &mut Context<Self>) {
        let removed = {
            let Some(editor) = self.curves_editor.as_mut() else {
                return;
            };
            let Some(index) = editor.selected else {
                return;
            };
            let channel = editor.settings.channel.index();
            let points = &mut editor.settings.channels[channel];
            let interior = index > 0 && index + 1 < points.len();
            if interior {
                points.remove(index);
                editor.selected = None;
            }
            interior
        };
        if removed {
            self.apply_curves_preview(cx);
        }
    }

    /// The "Reset curve" button: the channel's points return to the
    /// identity diagonal.
    pub fn curves_reset(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_mut() else {
            return;
        };
        let channel = editor.settings.channel.index();
        editor.settings.channels[channel] = CurvesSettings::default().channels[channel].clone();
        editor.selected = None;
        editor.dragging = None;
        self.apply_curves_preview(cx);
    }

    /// Folds a pointer position into the dragged point and refreshes the
    /// preview.
    fn curves_drag_to(&mut self, x: f64, y: f64, cx: &mut Context<Self>) {
        let Some(editor) = self.curves_editor.as_mut() else {
            return;
        };
        let Some(index) = editor.dragging else {
            return;
        };
        let channel = editor.settings.channel.index();
        let moved = curve_drag_move(&editor.settings.channels[channel], index, x, y);
        editor.settings.channels[channel] = moved;
        self.apply_curves_preview(cx);
    }
}

/// Window pixels inside the curve canvas → curve units 0…255 with the y
/// axis flipped (CurvesControls' `position(_:)` inverse).
pub fn curve_point_from_event(position: Point<Pixels>, bounds: Bounds<Pixels>) -> (f64, f64) {
    let width = f32::from(bounds.size.width).max(1.0) as f64;
    let height = f32::from(bounds.size.height).max(1.0) as f64;
    let x = (f32::from(position.x) - f32::from(bounds.origin.x)) as f64;
    let y = (f32::from(position.y) - f32::from(bounds.origin.y)) as f64;
    let x = (x / width * 255.0).clamp(0.0, 255.0);
    let y = (255.0 - y / height * 255.0).clamp(0.0, 255.0);
    (x, y)
}

/// Curve units → window pixels (CurvesControls' `position(_:)`).
pub fn curve_value_to_screen(x: f64, y: f64, bounds: Bounds<Pixels>) -> (f32, f32) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    (
        f32::from(bounds.origin.x) + (x as f32 / 255.0) * width,
        f32::from(bounds.origin.y) + (1.0 - y as f32 / 255.0) * height,
    )
}

/// The nearest control point within 14 curve units (CurvesControls' hit
/// test: the distance runs in curve units, not pixels).
pub fn curve_hit_point(points: &[CurvePoint], x: f64, y: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, point) in points.iter().enumerate() {
        let distance = ((point.x - x).powi(2) + (point.y - y).powi(2)).sqrt();
        if distance < 14.0 && best.is_none_or(|(_, best)| distance < best) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| index)
}

/// Starting a drag: grab a nearby point, or add a new one on empty canvas
/// (up to 32 points, `x` in 2…253, and at least one input level from every
/// existing point), or nothing.
pub fn curve_drag_begin(points: &[CurvePoint], x: f64, y: f64) -> CurveDragBegin {
    if let Some(index) = curve_hit_point(points, x, y) {
        return CurveDragBegin::Grab(index);
    }
    if points.len() >= 32
        || x <= 1.0
        || x >= 254.0
        || points.iter().any(|point| (point.x - x).abs() <= 1.0)
    {
        return CurveDragBegin::None;
    }
    let mut points = points.to_vec();
    // The list stays sorted by x; no existing point sits within 1 of x.
    let index = points.partition_point(|point| point.x < x);
    points.insert(index, CurvePoint { x, y });
    CurveDragBegin::Added { points, index }
}

/// Dragging a point: `y` follows the pointer; interior points also follow
/// `x`, clamped between their neighbors (`p[i].x = min(next-1,
/// max(prev+1, x))`).
pub fn curve_drag_move(points: &[CurvePoint], index: usize, x: f64, y: f64) -> Vec<CurvePoint> {
    let mut points = points.to_vec();
    // Neighbors first: the slot borrow and the reads must not overlap.
    let (previous, next, interior) = match index {
        0 => (0.0, 0.0, false),
        i if i + 1 >= points.len() => (0.0, 0.0, false),
        i => (points[i - 1].x, points[i + 1].x, true),
    };
    if let Some(slot) = points.get_mut(index) {
        slot.y = y;
        if interior {
            slot.x = (next - 1.0).min((previous + 1.0).max(x));
        }
    }
    points
}

/// The curve canvas: a layout pass that records the bounds for hit testing,
/// and a paint pass drawing grid, spline and control points, which also
/// installs the window-level drag events (the drag keeps tracking when the
/// pointer leaves the canvas, as a SwiftUI DragGesture does).
pub(crate) fn curves_canvas_view(
    curves: &CurvesSettings,
    points: &[CurvePoint],
    selected: Option<usize>,
    cx: &mut Context<AppState>,
) -> gpui_kit::AnyElement {
    let app_bounds = cx.entity().downgrade();
    let app_move = cx.entity().downgrade();
    let app_up = cx.entity().downgrade();

    // Sample the spline once per frame, in curve units.
    let channel = curves.channel.index();
    let polyline: Vec<CurvePoint> = (0..=255)
        .map(|step| CurvePoint {
            x: step as f64,
            y: curves.value(step as f64, channel),
        })
        .collect();
    let points = points.to_vec();

    canvas(
        move |bounds, _window, cx| {
            app_bounds
                .update(cx, |state, _| {
                    if let Some(editor) = state.curves_editor.as_mut() {
                        editor.canvas_bounds = bounds;
                    }
                })
                .ok();
        },
        move |bounds, _prepaint, window, _cx| {
            paint_curve_canvas(bounds, &polyline, &points, selected, window);
            install_curve_drag_events(&app_move, &app_up, window);
        },
    )
    .size_full()
    .into_any_element()
}

/// The grid (5×5, white 12%), the spline (2px white) and the control points
/// (8px squares, the selection in the accent color), matching
/// CurvesControls.swift's Canvas body.
fn paint_curve_canvas(
    bounds: Bounds<Pixels>,
    polyline: &[CurvePoint],
    points: &[CurvePoint],
    selected: Option<usize>,
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let origin_x = f32::from(bounds.origin.x);
    let origin_y = f32::from(bounds.origin.y);
    for step in 0..=4 {
        let fraction = step as f32 / 4.0;
        window.paint_quad(gpui_kit::fill(
            Bounds::<Pixels>::new(
                point(px(origin_x + fraction * width), px(origin_y)),
                size(px(1.0), px(height)),
            ),
            rgba(0xffffff1f),
        ));
        window.paint_quad(gpui_kit::fill(
            Bounds::<Pixels>::new(
                point(px(origin_x), px(origin_y + fraction * height)),
                size(px(width), px(1.0)),
            ),
            rgba(0xffffff1f),
        ));
    }
    let screen: Vec<Point<Pixels>> = polyline
        .iter()
        .map(|value| {
            let (x, y) = curve_value_to_screen(value.x, value.y, bounds);
            point(px(x), px(y))
        })
        .collect();
    if screen.len() >= 2 {
        let mut builder = PathBuilder::stroke(px(2.0));
        builder.add_polygon(&screen, false);
        if let Ok(path) = builder.build() {
            window.paint_path(path, rgba(0xffffffff));
        }
    }
    for (index, value) in points.iter().enumerate() {
        let (x, y) = curve_value_to_screen(value.x, value.y, bounds);
        let rect = Bounds::<Pixels>::new(point(px(x - 4.0), px(y - 4.0)), size(px(8.0), px(8.0)));
        window.paint_quad(gpui_kit::fill(
            rect,
            if selected == Some(index) {
                gpui_kit::rgb(crate::ui::ACCENT)
            } else {
                gpui_kit::rgb(0xffffff)
            },
        ));
    }
}

/// Window-level move/up handlers so an in-progress drag keeps following the
/// pointer outside the canvas (`canvas.rs`'s install_drag_events pattern).
fn install_curve_drag_events(
    app_move: &gpui_kit::WeakEntity<AppState>,
    app_up: &gpui_kit::WeakEntity<AppState>,
    window: &mut Window,
) {
    let app_move = app_move.clone();
    let app_up = app_up.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
        app_move
            .update(cx, |state, cx| state.curves_mouse_move(event, cx))
            .ok();
    });
    window.on_mouse_event(move |event: &MouseUpEvent, _phase, _window, cx| {
        app_up
            .update(cx, |state, cx| state.curves_mouse_up(event, cx))
            .ok();
    });
}

#[cfg(test)]
mod tests {
    use super::{
        CurveDragBegin, curve_drag_begin, curve_drag_move, curve_hit_point, curve_point_from_event,
        curve_value_to_screen,
    };
    use compositor_core::adjust::CurvePoint;
    use gpui_kit::{Bounds, Pixels, point, px, size};

    fn diagonal() -> Vec<CurvePoint> {
        vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ]
    }

    fn canvas_bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.0), px(20.0)), size(px(255.0), px(255.0)))
    }

    #[test]
    fn event_position_maps_to_curve_units() {
        let bounds = canvas_bounds();
        // The canvas origin is curve (0, 255); the bottom right is (255, 0).
        assert_eq!(
            curve_point_from_event(point(px(10.0), px(20.0)), bounds),
            (0.0, 255.0)
        );
        assert_eq!(
            curve_point_from_event(point(px(265.0), px(275.0)), bounds),
            (255.0, 0.0)
        );
        // The center is the diagonal's midpoint, and clamping keeps drags
        // outside the canvas in range.
        assert_eq!(
            curve_point_from_event(point(px(135.0), px(145.0)), bounds),
            (125.0, 130.0)
        );
        assert_eq!(
            curve_point_from_event(point(px(-50.0), px(-50.0)), bounds),
            (0.0, 255.0)
        );
    }

    #[test]
    fn screen_mapping_roundtrips_the_canvas_center() {
        let bounds = canvas_bounds();
        let (sx, sy) = curve_value_to_screen(128.0, 128.0, bounds);
        let (x, y) = curve_point_from_event(point(px(sx), px(sy)), bounds);
        assert!((x - 128.0).abs() < 1e-9, "x = {x}");
        assert!((y - 128.0).abs() < 1e-9, "y = {y}");
    }

    #[test]
    fn hit_test_finds_the_nearest_point_within_fourteen_units() {
        let points = diagonal();
        // On the diagonal at (100, 100): 141 units from each endpoint.
        assert_eq!(curve_hit_point(&points, 100.0, 100.0), None);
        let near_start = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 5.0, y: 5.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        // (3, 3) sits ~3 units from (5, 5) and ~4.2 from (0, 0): the nearer
        // point wins.
        assert_eq!(curve_hit_point(&near_start, 3.0, 3.0), Some(1));
        // Ties keep the earlier point, as Swift's min(by:) does.
        let tie = vec![
            CurvePoint { x: 10.0, y: 10.0 },
            CurvePoint { x: 14.0, y: 10.0 },
        ];
        assert_eq!(curve_hit_point(&tie, 12.0, 10.0), Some(0));
    }

    #[test]
    fn drag_begin_grabs_grabs_and_refuses() {
        let points = diagonal();
        // Near an endpoint: a grab.
        match curve_drag_begin(&points, 2.0, 3.0) {
            CurveDragBegin::Grab(0) => {}
            other => panic!("expected grab, got {other:?}"),
        }
        // Empty canvas in the middle: a new sorted point.
        match curve_drag_begin(&points, 128.0, 60.0) {
            CurveDragBegin::Added { points, index } => {
                assert_eq!(index, 1);
                assert_eq!(points.len(), 3);
                assert_eq!(points[1], CurvePoint { x: 128.0, y: 60.0 });
                assert!(points.windows(2).all(|w| w[0].x < w[1].x));
            }
            other => panic!("expected added, got {other:?}"),
        }
        // Swift's guards: x sits strictly inside 1…254 ...
        assert!(matches!(
            curve_drag_begin(&points, 1.0, 60.0),
            CurveDragBegin::None
        ));
        assert!(matches!(
            curve_drag_begin(&points, 254.0, 60.0),
            CurveDragBegin::None
        ));
        // ... and within one input level of no existing point's x.
        let with_mid = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 128.0, y: 64.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        assert!(matches!(
            curve_drag_begin(&with_mid, 128.5, 100.0),
            CurveDragBegin::None
        ));
        // 32 points: nothing more is added.
        let mut full: Vec<CurvePoint> = (0..31)
            .map(|i| CurvePoint {
                x: i as f64 * 8.0 + 0.5,
                y: 0.0,
            })
            .collect();
        full.push(CurvePoint { x: 255.0, y: 255.0 });
        assert_eq!(full.len(), 32);
        assert!(matches!(
            curve_drag_begin(&full, 128.0, 60.0),
            CurveDragBegin::None
        ));
    }

    #[test]
    fn drag_move_follows_y_and_clamps_interior_x() {
        // A sorted fixture: interior point at 128 with neighbors 0 and 200.
        let points = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 128.0, y: 100.0 },
            CurvePoint { x: 200.0, y: 220.0 },
            CurvePoint { x: 255.0, y: 255.0 },
        ];
        // Interior point: y is free, x clamps between neighbors.
        let moved = curve_drag_move(&points, 1, 250.0, 42.0);
        assert_eq!(moved[1].y, 42.0);
        assert_eq!(moved[1].x, 199.0); // min(next-1, max(prev+1, x))
        let moved = curve_drag_move(&points, 1, 1.0, 42.0);
        assert_eq!(moved[1].x, 1.0); // pinned at prev+1
        // Endpoints keep their x: the curve stays anchored at 0 and 255. (The
        // verbatim y is safe because the pointer conversion clamps to 0…255.)
        let moved = curve_drag_move(&points, 0, 128.0, 10.0);
        assert_eq!(moved[0].x, 0.0);
        assert_eq!(moved[0].y, 10.0);
        // An out-of-range index is a no-op.
        assert_eq!(curve_drag_move(&points, 9, 10.0, 10.0), points);
    }
}
