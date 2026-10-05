//! Port of `Compositor/CompositorTests/DistortTests.swift` (3),
//! `ShapeToolTests.swift` (4), `CloneStampTests.swift` (2) and the session
//! cases of `TypeToolTests.swift`.
//!
//! Per-case notes record what the Rust core cannot express yet:
//! - The Shape tool's drag flow (`beginShape`/`dragShape`/`finishShape`/
//!   `cancelShape`/`shapeDraft`) is not ported; the port drives
//!   `add_shape_layer` — what `finishShape` calls — and computes the drag
//!   rectangle inline where a case pins it.
//! - The Clone Stamp's stroke flow (`beginBrush` with clone settings, the
//!   missing-source `brushError`) is not ported; the port drives
//!   `begin_clone_stroke` (the alignment bookkeeping) and `RasterEdit`'s clone
//!   raster directly.
//! - `CloneStampTests.cloneStampKeepsItsOwnSoftBrushTip` needs per-tool brush
//!   tips switched by `selectTool`; the Rust session keeps one
//!   `brush_settings` and has no tool switch, so the case is not ported.
//! - Swift applies a pending transform on tool switching; the port calls
//!   `commit_transform`/`finish_text` — what the app's switch calls.

use compositor_core::document::{LayerShape, NavigationTool, ShapeKind, TextAlignment};
use compositor_core::geometry::{CGPoint, CGRect, CGSize};
use compositor_core::project::{
    ProjectData, document_from_project, project_from_document, save as save_project,
};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::session::EditorSession;
use compositor_core::tools::brush::{BrushSettings, RasterEdit};
use compositor_core::tools::distort::{Homography, is_convex, is_usable};
use compositor_core::transform::LayerTransform;

// MARK: - DistortTests

/// Swift `perspectiveMappingHitsTheCornersAndDegenerateShapesAreRefused`.
#[test]
fn perspective_mapping_hits_the_corners_and_degenerate_shapes_are_refused() {
    let shape = [
        CGPoint::new(10.0, 10.0),
        CGPoint::new(60.0, 10.0),
        CGPoint::new(30.0, 30.0),
        CGPoint::new(10.0, 30.0),
    ];
    let map = Homography::new(&shape);
    for (unit, corner) in [
        (CGPoint::new(0.0, 0.0), shape[0]),
        (CGPoint::new(1.0, 0.0), shape[1]),
        (CGPoint::new(1.0, 1.0), shape[2]),
        (CGPoint::new(0.0, 1.0), shape[3]),
    ] {
        let mapped = map.apply(unit);
        assert!(
            (mapped.x - corner.x).abs() < 1e-6 && (mapped.y - corner.y).abs() < 1e-6,
            "{unit:?} went to {mapped:?}"
        );
    }
    assert!(is_usable(&shape));
    assert!(is_convex(&shape));
    // A bow-tie is usable — it draws folded — but it is not a perspective.
    let bowtie = [shape[0], shape[2], shape[1], shape[3]];
    assert!(is_usable(&bowtie));
    assert!(!is_convex(&bowtie));
    // A collapsed corner leaves one half with nothing to draw.
    let collapsed = [shape[0], shape[0], shape[2], shape[3]];
    assert!(!is_usable(&collapsed));
    assert!(!is_convex(&collapsed));
}

/// A red square layer inserted and placed at `origin` × `size`.
fn red_layer(
    session: &mut EditorSession,
    width: usize,
    height: usize,
    origin: CGPoint,
    size: CGSize,
    name: &str,
) {
    let mut image = Bitmap::new(width, height);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[255, 0, 0, 255]);
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, name),
        None,
    );
    let id = session.active_layer_id.expect("inserted");
    session
        .document
        .as_mut()
        .unwrap()
        .layer_mut(id)
        .unwrap()
        .transform = LayerTransform::new(origin, size);
}

fn rendered(session: &EditorSession) -> Bitmap {
    draw_live_composite(session.document.as_ref().expect("document exists"))
}

/// Swift `distortingWarpsTheLayerIntoTheShapeAsOneUndoStep`. Swift also
/// asserts `persistent == true` after `beginDistort`; the Rust `begin_distort`
/// leaves the edit's persistence flag alone (the flag is render-only).
#[test]
fn distorting_warps_the_layer_into_the_shape_as_one_undo_step() {
    let mut session = EditorSession::new();
    session.create_document(100, 60, false);
    red_layer(
        &mut session,
        20,
        20,
        CGPoint::new(10.0, 10.0),
        CGSize::new(20.0, 20.0),
        "Red",
    );
    session.tool = NavigationTool::Move;
    session.begin_transform_persistent(false);
    session.begin_distort();
    {
        let edit = session.transform_edit.as_ref().expect("transform edit");
        assert_eq!(edit.corners.as_ref().map(|c| c.len()), Some(4));
    }
    let shape = [
        CGPoint::new(10.0, 10.0),
        CGPoint::new(60.0, 10.0),
        CGPoint::new(30.0, 30.0),
        CGPoint::new(10.0, 30.0),
    ];
    // A folded shape is a distortion in its own right, so the preview takes it.
    session.preview_corners([shape[0], shape[2], shape[1], shape[3]]);
    assert_eq!(
        session
            .transform_edit
            .as_ref()
            .unwrap()
            .corners
            .as_ref()
            .unwrap()[1],
        shape[2]
    );
    session.preview_corners(shape);
    let count = session.history.undo_count();
    session.commit_transform();
    assert!(session.transform_edit.is_none());
    assert_eq!(session.history.undo_count(), count + 1);
    let transform = session.active_layer().unwrap().transform;
    assert_eq!(transform.origin, CGPoint::new(10.0, 10.0));
    // Swift lands on exactly 50 × 20: Core Image antialiases the shape's
    // edges, so the rightmost partly covered column survives the trim. The
    // warp's analytic quad-pixel coverage matches that.
    assert_eq!(transform.size.width, 50.0, "{:?}", transform.size);
    assert_eq!(transform.size.height, 20.0);
    assert_eq!(transform.rotation, 0.0);

    let result = rendered(&session);
    assert_eq!(
        result.pixel(50, 12)[3],
        255,
        "inside the stretched top-right"
    );
    assert_eq!(result.pixel(15, 25)[3], 255);
    assert_eq!(result.pixel(50, 28)[3], 0, "outside the slanted right edge");
    assert_eq!(result.pixel(80, 12)[3], 0);

    session.undo();
    assert_eq!(
        session.active_layer().unwrap().transform.size,
        CGSize::new(20.0, 20.0)
    );
}

/// Swift `distortedLayerIsTrimmedToItsVisiblePixels` — after Apply the layer
/// hugs its visible pixels.
#[test]
fn distorted_layer_is_trimmed_to_its_visible_pixels() {
    let mut session = EditorSession::new();
    session.create_document(100, 60, false);
    // A 40 × 20 layer that is transparent except for a 10 × 10 red square.
    let mut image = Bitmap::new(40, 20);
    for y in 5..15 {
        for x in 15..25 {
            image.set_pixel(x, y, [255, 0, 0, 255]);
        }
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, "Stroke"),
        None,
    );
    let id = session.active_layer_id.expect("inserted");
    session
        .document
        .as_mut()
        .unwrap()
        .layer_mut(id)
        .unwrap()
        .transform = LayerTransform::new(CGPoint::new(10.0, 10.0), CGSize::new(40.0, 20.0));
    session.tool = NavigationTool::Move;
    session.begin_transform_persistent(false);
    session.begin_distort();
    session.preview_corners([
        CGPoint::new(10.0, 10.0),
        CGPoint::new(60.0, 10.0),
        CGPoint::new(50.0, 30.0),
        CGPoint::new(10.0, 30.0),
    ]);
    session.commit_transform();
    let transform = session.active_layer().unwrap().transform;
    // The shape's bounds are 50 × 20; the square's warp is far smaller.
    assert!(transform.size.width < 20.0, "layer bounds {transform:?}");
    assert!(transform.size.height <= 12.0, "layer bounds {transform:?}");
    assert!(transform.origin.x >= 20.0 && transform.origin.y >= 14.0);
    let result = rendered(&session);
    assert_eq!(
        result.pixel(30, 20)[3],
        255,
        "the square stays where the warp put it"
    );
}

// MARK: - ShapeToolTests

fn shape_session() -> EditorSession {
    let mut session = EditorSession::new();
    session.create_document(100, 80, true);
    session.tool = NavigationTool::Shape;
    session.brush_settings = BrushSettings {
        red: 1.0,
        ..BrushSettings::default()
    };
    session
}

/// Swift `drag(from:to:square:fromCenter:)` — the app's drag computes the
/// rectangle; `finishShape` adds the shape layer.
fn drag_rect(start: CGPoint, end: CGPoint, square: bool, from_center: bool) -> CGRect {
    let mut width = end.x - start.x;
    let mut height = end.y - start.y;
    if from_center {
        width *= 2.0;
        height *= 2.0;
    }
    if square {
        let side = width.abs().max(height.abs());
        width = if width < 0.0 { -side } else { side };
        height = if height < 0.0 { -side } else { side };
    }
    let origin = if from_center {
        CGPoint::new(start.x - width / 2.0, start.y - height / 2.0)
    } else {
        start
    };
    CGRect::new(origin.x, origin.y, width.abs(), height.abs())
}

fn fill_shape(kind: ShapeKind, corner_radius: f64, session: &EditorSession) -> LayerShape {
    let foreground = session.palette_color(false);
    LayerShape {
        kind,
        corner_radius,
        line_width: 4.0,
        red: foreground.red,
        green: foreground.green,
        blue: foreground.blue,
        filled: true,
        start: CGPoint::ZERO,
        end: CGPoint::new(1.0, 1.0),
    }
}

/// Swift `rectangleFillsANewLayerWithTheForegroundColorAsOneUndoStep`.
#[test]
fn rectangle_fills_a_new_layer_with_the_foreground_color_as_one_undo_step() {
    let mut session = shape_session();
    session.select_all();
    let count = session.history.undo_count();
    let rect = drag_rect(
        CGPoint::new(10.0, 10.0),
        CGPoint::new(40.0, 30.0),
        false,
        false,
    );
    let name = session.next_shape_name(ShapeKind::Rectangle);
    let id = session
        .add_shape_layer(fill_shape(ShapeKind::Rectangle, 0.0, &session), rect, name)
        .expect("rectangle lands");
    let document = session.document.as_ref().expect("document");
    let names: Vec<&str> = document.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["Layer 1", "Rectangle 1"]);
    assert_eq!(session.active_layer().unwrap().name, "Rectangle 1");
    assert_eq!(session.history.undo_count(), count + 1);
    let layer = document.layer(id).unwrap();
    assert_eq!(layer.transform.origin, CGPoint::new(10.0, 10.0));
    assert_eq!(layer.transform.size, CGSize::new(30.0, 20.0));
    // Unlike Paste, drawing a shape keeps the selection.
    assert!(session.selection().is_some());
    let result = rendered(&session);
    assert_eq!(result.pixel(25, 20), [255, 0, 0, 255]);
    assert_eq!(result.pixel(10, 10), [255, 0, 0, 255]);
    assert_eq!(result.pixel(39, 29), [255, 0, 0, 255]);
    assert_eq!(result.pixel(9, 20)[3], 0);
    assert_eq!(result.pixel(40, 20)[3], 0);
    assert_eq!(result.pixel(25, 30)[3], 0);

    let rect = drag_rect(
        CGPoint::new(60.0, 10.0),
        CGPoint::new(70.0, 20.0),
        false,
        false,
    );
    let name = session.next_shape_name(ShapeKind::Rectangle);
    session
        .add_shape_layer(fill_shape(ShapeKind::Rectangle, 0.0, &session), rect, name)
        .expect("second rectangle lands");
    assert_eq!(session.active_layer().unwrap().name, "Rectangle 2");
    session.undo();
    session.undo();
    let names: Vec<&str> = session
        .document
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, vec!["Layer 1"]);
}

/// Swift `ellipseLeavesItsCornersClearWithShiftCircleAndOptionFromCenter`.
#[test]
fn ellipse_leaves_its_corners_clear() {
    let mut session = shape_session();
    session.toggle_shape_kind();
    assert_eq!(session.shape_kind, ShapeKind::Ellipse);
    let rect = drag_rect(
        CGPoint::new(50.0, 40.0),
        CGPoint::new(60.0, 45.0),
        true,
        true,
    );
    assert_eq!(rect, CGRect::new(40.0, 30.0, 20.0, 20.0));
    let name = session.next_shape_name(ShapeKind::Ellipse);
    session
        .add_shape_layer(fill_shape(ShapeKind::Ellipse, 0.0, &session), rect, name)
        .expect("ellipse lands");
    assert_eq!(session.active_layer().unwrap().name, "Ellipse 1");
    assert_eq!(
        session.active_layer().unwrap().transform.origin,
        CGPoint::new(40.0, 30.0)
    );
    assert_eq!(
        session.active_layer().unwrap().transform.size,
        CGSize::new(20.0, 20.0)
    );
    let result = rendered(&session);
    assert_eq!(result.pixel(50, 40), [255, 0, 0, 255]);
    assert!(result.pixel(41, 40)[3] > 0);
    assert!(result.pixel(50, 31)[3] > 0);
    assert_eq!(
        result.pixel(40, 30)[3],
        0,
        "outside the circle, inside its box"
    );
    assert_eq!(result.pixel(59, 49)[3], 0);
}

/// Swift `aClickEscapeOrToolSwitchMakesNoLayer` — the no-drag halves; the
/// draft/Escape flow (`shapeDraft`, `cancelShape`) is app-side (module notes).
#[test]
fn a_click_makes_no_layer() {
    let mut session = shape_session();
    let count = session.history.undo_count();
    // A click: a zero-sized rectangle is refused.
    let click = drag_rect(
        CGPoint::new(20.0, 20.0),
        CGPoint::new(20.0, 20.0),
        false,
        false,
    );
    assert!(
        session
            .add_shape_layer(
                fill_shape(ShapeKind::Rectangle, 0.0, &session),
                click,
                "Rectangle 1".into()
            )
            .is_none()
    );
    assert_eq!(session.history.undo_count(), count);
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
}

/// Swift `roundedRectanglesFollowTheRadiusAndClampToAPill`.
#[test]
fn rounded_rectangles_follow_the_radius_and_clamp_to_a_pill() {
    let mut session = shape_session();
    let rect = drag_rect(
        CGPoint::new(10.0, 10.0),
        CGPoint::new(50.0, 40.0),
        false,
        false,
    );
    let name = session.next_shape_name(ShapeKind::RoundedRectangle);
    session.shape_corner_radius = 8.0;
    // The Rust core splits Swift's rectangle-with-radius into its own kind.
    session
        .add_shape_layer(
            fill_shape(ShapeKind::RoundedRectangle, 8.0, &session),
            rect,
            name,
        )
        .expect("rounded rectangle lands");
    let rect = drag_rect(
        CGPoint::new(55.0, 50.0),
        CGPoint::new(95.0, 70.0),
        false,
        false,
    );
    let name = session.next_shape_name(ShapeKind::RoundedRectangle);
    session.shape_corner_radius = 500.0; // 40 × 20: the pill radius is 10.
    session
        .add_shape_layer(
            fill_shape(ShapeKind::RoundedRectangle, 500.0, &session),
            rect,
            name,
        )
        .expect("pill lands");
    let result = rendered(&session);
    assert_eq!(result.pixel(10, 10)[3], 0, "the corner is cut away");
    assert_eq!(result.pixel(11, 11)[3], 0);
    assert_eq!(result.pixel(13, 13)[3], 255, "inside the rounded corner");
    assert_eq!(result.pixel(30, 10)[3], 255, "straight edges stay full");
    assert_eq!(result.pixel(30, 25), [255, 0, 0, 255]);
    assert_eq!(result.pixel(55, 50)[3], 0, "the pill's corner is round");
    assert_eq!(result.pixel(75, 60), [255, 0, 0, 255]);
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 3);
}

// MARK: - CloneStampTests

/// Swift `copiesTheSourceUnderTheBrushKeepingAlignmentUntilItIsTurnedOff` —
/// the alignment bookkeeping and the clone raster, minus the stroke session
/// flow and its one-undo commit (module notes).
#[test]
fn clone_copies_the_source_under_the_brush_keeping_alignment() {
    let mut session = EditorSession::new();
    session.create_document(80, 40, false);
    // Left half red with a green square (x 10–19, y 15–24), right half blue.
    let mut image = Bitmap::new(80, 40);
    for y in 0..40 {
        for x in 0..80 {
            let color = if (10..20).contains(&x) && (15..25).contains(&y) {
                [0, 255, 0, 255]
            } else if x < 40 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            image.set_pixel(x, y, color);
        }
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image.clone(), "Colors"),
        None,
    );
    session.tool = NavigationTool::CloneStamp;
    session.brush_settings = BrushSettings {
        diameter: 6.0,
        hardness: 1.0,
        ..BrushSettings::default()
    };
    // Nothing to copy until a source is set.
    assert!(
        session
            .begin_clone_stroke(CGPoint::new(60.0, 20.0))
            .is_none()
    );
    let count = session.history.undo_count();

    session.clone_source = Some(CGPoint::new(15.0, 20.0));
    let offset = session
        .begin_clone_stroke(CGPoint::new(60.0, 20.0))
        .expect("offset");
    assert_eq!(offset, CGSize::new(-45.0, 0.0));
    let stroke = |session: &EditorSession, at: CGPoint, offset: CGSize| {
        let layer = session.active_layer().unwrap().clone();
        let mut edit = RasterEdit::new(
            Some(layer.asset.as_ref().unwrap().image.as_ref().clone()),
            layer.transform,
            false,
            session.brush_settings,
            CGSize::new(80.0, 40.0),
            None,
        );
        edit.clone = Some((image.clone(), offset));
        edit.replaces_with_clone = true;
        edit.append(at);
        edit.append(CGPoint::new(at.x + 0.5, at.y));
        edit.flush();
        edit.result()
    };
    let after = stroke(&session, CGPoint::new(60.0, 20.0), offset);
    assert_eq!(
        after.pixel(60, 20),
        [0, 255, 0, 255],
        "the green source, under the brush"
    );
    assert_eq!(after.pixel(70, 5), [0, 0, 255, 255], "untouched elsewhere");

    // Aligned: the next stroke keeps the offset (−45, 0), so (66, 20) copies
    // red from (21, 20).
    let kept = session
        .begin_clone_stroke(CGPoint::new(66.0, 20.0))
        .expect("offset");
    assert_eq!(kept, CGSize::new(-45.0, 0.0));
    let after = stroke(&session, CGPoint::new(66.0, 20.0), kept);
    assert_eq!(after.pixel(66, 20), [255, 0, 0, 255]);

    // Not aligned: every stroke starts at the source again.
    session.clone_settings.aligned = false;
    let fresh = session
        .begin_clone_stroke(CGPoint::new(50.0, 10.0))
        .expect("offset");
    assert_eq!(fresh, CGSize::new(-35.0, 10.0));
    let after = stroke(&session, CGPoint::new(50.0, 10.0), fresh);
    assert_eq!(after.pixel(50, 10), [0, 255, 0, 255]);
    // The strokes are painted through RasterEdit directly (no session commit
    // in this ported half), so history is untouched; the clone offset state
    // is what the case exercises.
    assert_eq!(
        session.history.undo_count(),
        count,
        "no undo steps recorded"
    );
}

// CloneStampTests.cloneStampKeepsItsOwnSoftBrushTip is not ported: it needs
// per-tool brush tips switched by `selectTool`, which the Rust session does
// not model (one `brush_settings`, no tool switch).

// MARK: - TypeToolTests (session cases)

fn type_session() -> EditorSession {
    let mut session = EditorSession::new();
    session.create_document(800, 600, true);
    session.tool = NavigationTool::Type;
    session
}

/// Swift `createEditCancelAndUndo`.
#[test]
fn text_create_edit_and_undo() {
    let mut session = type_session();
    let before = session.history.undo_count();
    session.begin_text(CGPoint::new(30.0, 40.0), true);
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
    let mut draft = session.text_draft.clone().expect("draft");
    draft.style.content = "Hello\nCompositor".to_string();
    draft.style.font_size = 48.0;
    assert!(session.apply_text(draft.clone()));
    let active = session.active_layer().unwrap().clone();
    assert_eq!(active.live_text().unwrap().style, draft.style);
    assert_eq!(active.origin(), CGPoint::new(30.0, 40.0));
    assert_eq!(session.history.undo_count(), before + 1);
    session.edit_active_text();
    assert!(session.text_draft.is_some());
    session.text_draft = None;
    assert_eq!(session.history.undo_count(), before + 1);
    session.edit_active_text();
    let mut draft = session.text_draft.clone().expect("draft");
    draft.style.content = "Changed".to_string();
    assert!(session.apply_text(draft));
    session.undo();
    assert_eq!(
        session
            .active_layer()
            .unwrap()
            .live_text()
            .unwrap()
            .style
            .content,
        "Hello\nCompositor"
    );
    session.undo();
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
    session.redo();
    assert!(session.active_layer().unwrap().live_text().is_some());
}

/// Swift `transformsDuplicatesAndClippingKeepTextEditable`.
#[test]
fn transforms_duplicates_and_clipping_keep_text_editable() {
    let mut session = type_session();
    session.begin_text(CGPoint::new(20.0, 20.0), true);
    let mut draft = session.text_draft.clone().expect("draft");
    draft.style.content = "Text".to_string();
    assert!(session.apply_text(draft));
    let id = session.active_layer_id.expect("text layer");
    session
        .document
        .as_mut()
        .unwrap()
        .layer_mut(id)
        .unwrap()
        .transform
        .rotation = 30.0;
    session
        .document
        .as_mut()
        .unwrap()
        .layer_mut(id)
        .unwrap()
        .transform
        .size
        .width *= 2.0;
    let old = session.active_layer().unwrap().transform;
    session.edit_active_text();
    let mut draft = session.text_draft.clone().expect("draft");
    draft.style.content = "Longer text".to_string();
    assert!(session.apply_text(draft));
    let updated = session.active_layer().unwrap().transform;
    assert_eq!(updated.rotation, 30.0);
    assert!((updated.point(CGPoint::ZERO).x - old.point(CGPoint::ZERO).x).abs() < 0.001);
    assert!((updated.point(CGPoint::ZERO).y - old.point(CGPoint::ZERO).y).abs() < 0.001);
    session.duplicate_active_layer();
    assert_eq!(
        session
            .active_layer()
            .unwrap()
            .live_text()
            .unwrap()
            .style
            .content,
        "Longer text"
    );
    let target = session.active_layer_id.expect("duplicate");
    assert!(session.link_mask(id, target));
    assert_eq!(session.active_layer().unwrap().mask_source_id, Some(id));
    assert!(
        session
            .document
            .as_ref()
            .unwrap()
            .layer(id)
            .unwrap()
            .live_text()
            .is_some()
    );
    assert_ne!(updated, old, "the re-edit refreshes the transform");
}

/// Swift `saveReopenAndRasterize` — the manifest round trip; Swift's
/// `installProject` is expressed through the pub document fields.
#[test]
fn text_saves_reopens_and_rasterizes() {
    let mut session = type_session();
    session.begin_text(CGPoint::ZERO, true);
    let mut draft = session.text_draft.clone().expect("draft");
    draft.style.content = "Café 日本語\nSecond line".to_string();
    draft.style.alignment = TextAlignment::Right;
    draft.style.tracking = 3.0;
    assert!(session.apply_text(draft.clone()));
    let data = project_from_document(session.document.as_ref().unwrap());
    let dir = std::env::temp_dir().join(format!("{}", uuid::Uuid::new_v4()));
    save_project(&dir, &data).expect("saves");
    let loaded: ProjectData = compositor_core::project::load(&dir).expect("loads");
    let (document, active) = document_from_project(&loaded).expect("decodes");
    // `project_from_document` leaves the manifest's active layer unset (a
    // gap vs Swift's snapshot), so the reopen falls back to the top layer.
    let top = document.layers.last().map(|l| l.id);
    let mut reopened = type_session();
    reopened.document = Some(document);
    reopened.active_layer_id = active.or(top);
    assert_eq!(
        reopened.active_layer().unwrap().live_text().unwrap().style,
        draft.style
    );
    // Replacing the pixels rasterizes the layer: the live text goes.
    let index = reopened
        .document
        .as_ref()
        .unwrap()
        .layer_index(reopened.active_layer_id.unwrap())
        .unwrap();
    let mut replacement = Bitmap::new(10, 10);
    for chunk in replacement.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[255, 0, 0, 255]);
    }
    reopened.document.as_mut().unwrap().layers[index].asset =
        Some(ImportedImage::new(replacement, "Painted"));
    reopened.document.as_mut().unwrap().layers[index].text = None;
    assert!(reopened.active_layer().unwrap().live_text().is_none());
    // The manifest no longer carries the text either.
    let saved = project_from_document(reopened.document.as_ref().unwrap());
    assert!(saved.manifest.layers[index].text.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Swift `paragraphBoxAndToolSwitchCommitEditableText`. Swift commits the
/// pending draft on tool switches; the port calls `finish_text`, which is
/// what the app's switch invokes.
#[test]
fn paragraph_box_and_tool_switch_commit_editable_text() {
    let mut session = type_session();
    session.begin_text_in(CGRect::new(40.0, 60.0, 200.0, 120.0));
    assert_eq!(session.text_draft.as_ref().unwrap().style.content, "");
    session.text_draft.as_mut().unwrap().style.content =
        "Text that wraps inside its paragraph box".to_string();
    session.finish_text(); // The tool switch applies it.
    session.tool = NavigationTool::Brush;
    assert!(session.text_draft.is_none());
    assert_eq!(
        session.active_layer().unwrap().size(),
        CGSize::new(200.0, 120.0)
    );
    assert_eq!(
        session
            .active_layer()
            .unwrap()
            .live_text()
            .unwrap()
            .style
            .box_size,
        Some(CGSize::new(200.0, 120.0))
    );
    session.tool = NavigationTool::Type;
    session.edit_active_text();
    session.text_draft.as_mut().unwrap().style.content = "Edited on canvas".to_string();
    session.cancel_text();
    assert_eq!(
        session
            .active_layer()
            .unwrap()
            .live_text()
            .unwrap()
            .style
            .content,
        "Text that wraps inside its paragraph box"
    );
}

/// Swift `emptyNewParagraphIsDiscarded`.
#[test]
fn empty_new_paragraph_is_discarded() {
    let mut session = type_session();
    let count = session.document.as_ref().unwrap().layers.len();
    session.begin_text_in(CGRect::new(0.0, 0.0, 100.0, 100.0));
    session.finish_text(); // The tool switch applies it.
    assert_eq!(session.document.as_ref().unwrap().layers.len(), count);
    assert!(session.text_draft.is_none());
}

/// Swift `invalidAndStaleDraftsDoNotChangeDocument`.
#[test]
fn invalid_and_stale_drafts_do_not_change_document() {
    let mut session = type_session();
    session.begin_text(CGPoint::ZERO, true);
    let mut draft = session.text_draft.clone().expect("draft");
    draft.style.content = "Text".to_string();
    draft.style.font_size = f64::NAN;
    assert!(!session.apply_text(draft.clone()));
    draft.style.font_size = 72.0;
    draft.style.box_size = Some(CGSize::new(0.0, 100.0));
    assert!(!session.apply_text(draft.clone()));
    draft.style.box_size = Some(CGSize::new(360.0, 160.0));
    draft.style.content = "Valid".to_string();
    // A draft against a document that has since been replaced is stale.
    session.text_draft = None;
    session.create_document(100, 100, true);
    assert!(!session.apply_text(draft));
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
}
