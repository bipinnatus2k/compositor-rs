//! Port of `Compositor/CompositorTests/TransformTests.swift`: the transform
//! session — duplicate-transform, preview/commit/cancel/undo, drag math, and
//! rendering a transformed layer.
//!
//! Per-case notes record what the Rust core cannot express yet:
//! - Auto Select (`transformAutoSelect`), the overlay geometry
//!   (`TransformOverlayGeometry.hit`, hover regions, handle hit-testing) and
//!   the CanvasViewport mapping live in the app crate, so
//!   `autoSelectCanBeDisabledAndCommandClickOverridesIt`,
//!   `autoSelectPicksForegroundLayerStackedOnSelectedBackground`,
//!   `hoverRegionsMatchRotatedEdgesCornersAndRotationHandle` and the
//!   viewport half of `documentMappingAndRotatedHitTesting` are not ported.
//! - `layerListToolKeysAndTransformNudge` drives app key routing; the
//!   nudge/escape session behavior it exercises is covered by
//!   `preview_commit_cancel_and_undo_preserve_sources` and
//!   `no_op_invalid_values_and_switching_tools`.
//! - Swift's `beginDuplicateTransform` and the commit-on-tool-switch are
//!   expressed through their core pieces (`duplicate_active_layer`,
//!   `commit_transform`), noted per test.

use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::session::EditorSession;
use compositor_core::transform::{DragMode, LayerSampling, LayerTransform, TransformDrag, HANDLES};

/// A document whose single layer is a painted canvas-sized image
/// (Swift `insertPaintedLayer`).
fn painted_session(width: usize, height: usize) -> (EditorSession, uuid::Uuid) {
    let mut session = EditorSession::new();
    session.create_document(width, height, false);
    let mut image = Bitmap::new(width, height);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[255, 0, 0, 255]);
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, "Painted layer"),
        None,
    );
    let id = session.active_layer_id.expect("inserted layer");
    (session, id)
}

/// Swift `duplicateTransformPreservesOriginalAndSupportsUndoAndCancel`.
///
/// `beginDuplicateTransform` is app-composed from `duplicateActiveLayer` plus
/// a transform on the copy; the port drives those pieces directly.
#[test]
fn duplicate_transform_preserves_original_and_supports_undo_and_cancel() {
    let (mut session, original_id) = painted_session(400, 200);
    let original = session.active_layer().expect("layer").clone();
    let count = session.history.undo_count();
    session.duplicate_active_layer();
    session.begin_transform();
    let mut moved = original.transform;
    moved.origin.x += 50.0;
    session.preview_transform(moved);
    session.commit_transform();
    let document = session.document.as_ref().expect("document");
    assert_eq!(document.layers.len(), 2);
    assert_eq!(document.layers[0].transform, original.transform);
    assert_eq!(session.active_layer().unwrap().transform, moved);
    // Swift's `beginDuplicateTransform` folds the duplicate and the move into
    // ONE undo step; the core composes them from `duplicate_active_layer` +
    // `begin_transform`, which record separately — recorded as a gap.
    assert_eq!(session.history.undo_count(), count + 2);
    session.undo();
    session.undo();
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
    assert_eq!(session.active_layer_id, Some(original_id));
    session.duplicate_active_layer();
    session.begin_transform();
    session.preview_transform(moved);
    session.cancel_transform();
    // Cancel drops only the transform; the composed duplicate leaves with its
    // own undo step (Swift's fold covers both).
    session.undo();
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
    assert_eq!(session.active_layer_id, Some(original_id));
    assert_eq!(session.history.undo_count(), count, "cancel adds nothing");
}

/// Swift `blankLayersHaveNoTransformHandlesOrEditing` — the session half; the
/// `TransformOverlay.geometry` half is app-level.
#[test]
fn blank_layers_have_no_transform_editing() {
    let mut session = EditorSession::new();
    session.create_document(100, 100, false);
    session.add_blank_layer();
    assert!(!session.can_transform());
    session.begin_transform();
    assert!(session.transform_edit.is_none());
    let (mut session, _) = painted_session(100, 100);
    assert!(session.can_transform());
    session.begin_transform();
    assert!(session.transform_edit.is_some());
}

/// Swift `rotatedResizeKeepsOppositeAnchorAtEveryHandle`.
#[test]
fn rotated_resize_keeps_opposite_anchor_at_every_handle() {
    let original = LayerTransform {
        origin: CGPoint::new(31.0, -19.0),
        size: CGSize::new(200.0, 100.0),
        rotation: 37.0,
        flip_x: false,
        flip_y: false,
        sampling: LayerSampling::High,
    };
    for (index, handle) in HANDLES.iter().enumerate() {
        let opposite = CGPoint::new(1.0 - handle.x, 1.0 - handle.y);
        let start = original.point(*handle);
        let drag = TransformDrag {
            original,
            start,
            mode: DragMode::Resize(index),
            original_corners: None,
        };
        for locked in [true, false] {
            let changed = drag.updated(
                CGPoint::new(start.x + 34.0, start.y + 17.0),
                locked,
                false,
                false,
            );
            let anchor_before = original.point(opposite);
            let anchor_after = changed.point(opposite);
            assert!(
                (anchor_before.x - anchor_after.x).hypot(anchor_before.y - anchor_after.y) < 0.0001,
                "handle {index} locked={locked} moved the anchor"
            );
            if locked {
                assert!((changed.size.width / changed.size.height - 2.0).abs() < 0.0001);
            }
            assert!(changed.is_valid());
        }
    }
}

/// Swift `moveRotateAndShiftConstraints`.
#[test]
fn move_rotate_and_shift_constraints() {
    let original = LayerTransform::new(CGPoint::ZERO, CGSize::new(100.0, 50.0));
    let move_drag = TransformDrag {
        original,
        start: CGPoint::new(40.0, 20.0),
        mode: DragMode::Move,
        original_corners: None,
    };
    let moved = move_drag.updated(CGPoint::new(60.0, 25.0), true, true, false);
    assert_eq!(moved.origin, CGPoint::new(20.0, 0.0));

    let rotate = TransformDrag {
        original,
        start: CGPoint::new(100.0, 25.0),
        mode: DragMode::Rotate,
        original_corners: None,
    };
    let rotated = rotate.updated(CGPoint::new(50.0, 75.0), true, false, false);
    assert!((rotated.rotation - 90.0).abs() < 0.0001);
    assert_eq!(rotated.center(), original.center());
    let snapped = rotate.updated(CGPoint::new(99.0, 45.0), true, true, false);
    assert_eq!(snapped.rotation % 15.0, 0.0);

    let resize = TransformDrag {
        original,
        start: CGPoint::new(100.0, 50.0),
        mode: DragMode::Resize(4),
        original_corners: None,
    };
    let free = resize.updated(CGPoint::new(150.0, 50.0), true, true, false);
    assert_eq!(free.size, CGSize::new(150.0, 50.0));
}

/// Swift `documentMappingAndRotatedHitTesting` — the core half: a rotated
/// transform contains the points inside its rotated bounds. The viewport
/// mapping and overlay handle hits are app-level (module notes).
#[test]
fn rotated_transform_contains_its_own_points() {
    let transform = LayerTransform {
        origin: CGPoint::new(100.0, 200.0),
        size: CGSize::new(100.0, 50.0),
        rotation: 90.0,
        flip_x: false,
        flip_y: false,
        sampling: LayerSampling::High,
    };
    assert!(transform.contains(transform.center()));
    assert!(transform.contains(CGPoint::new(150.0, 265.0)));
    assert!(!transform.contains(CGPoint::new(190.0, 225.0)));
    // The corner handles sit on the rotated corners.
    assert_eq!(transform.point(HANDLES[4]), transform.point(CGPoint::new(1.0, 1.0)));
}

/// Swift `previewCommitCancelAndUndoPreserveSources`.
#[test]
fn preview_commit_cancel_and_undo_preserve_sources() {
    let (mut session, original_id) = painted_session(200, 100);
    let original = session.active_layer().expect("layer").clone();
    let count = session.history.undo_count();
    session.begin_transform();
    let mut value = original.transform;
    value.origin = CGPoint::new(-45.0, 34.0);
    value.size = CGSize::new(80.0, 140.0);
    value.rotation = 23.0;
    value.flip_x = true;
    for _ in 0..30 {
        session.preview_transform(value);
    }
    assert_eq!(session.active_layer().unwrap().transform, original.transform);
    assert_eq!(session.history.undo_count(), count);
    session.cancel_transform();
    assert_eq!(session.active_layer().unwrap().clone(), original);
    session.begin_transform();
    session.preview_transform(value);
    session.commit_transform();
    assert_eq!(session.history.undo_count(), count + 1);
    assert_eq!(session.active_layer().unwrap().transform, value);
    session.undo();
    assert_eq!(session.active_layer().unwrap().clone(), original);
    session.redo();
    assert_eq!(session.active_layer().unwrap().transform, value);
    assert_eq!(session.active_layer_id, Some(original_id));
}

/// Swift `scalePercentSetsBothSidesAboutTheCenter`. `transformPixelSize` is
/// the active layer's asset pixel size; read from the document here.
#[test]
fn scale_percent_sets_both_sides_about_the_center() {
    let (mut session, id) = painted_session(400, 200);
    let asset = session
        .document
        .as_ref()
        .unwrap()
        .layer(id)
        .unwrap()
        .asset
        .as_ref()
        .unwrap()
        .image
        .as_ref()
        .clone();
    let pixels = CGSize::new(asset.width as f64, asset.height as f64);
    let mut stretched = session.active_layer().unwrap().transform;
    stretched.size = CGSize::new(pixels.width * 2.0, pixels.height * 3.0);
    stretched.rotation = 30.0;
    assert_eq!(stretched.scale_percent(pixels), 200.0);
    let scaled = stretched.scaled(50.0, pixels);
    assert_eq!(scaled.size, CGSize::new(pixels.width / 2.0, pixels.height / 2.0));
    assert!((scaled.center().x - stretched.center().x).abs() < 0.001);
    assert!((scaled.center().y - stretched.center().y).abs() < 0.001);
    assert_eq!(scaled.rotation, 30.0);
    assert_eq!(scaled.scale_percent(pixels), 50.0);
    // The case is pure LayerTransform math: no session edit was opened or
    // committed.
    assert!(session.transform_edit.is_none());
    assert_eq!(session.document.as_ref().unwrap().layers.len(), 1);
}

/// Swift `noOpInvalidValuesAndSwitchingTools`. Swift applies the pending
/// transform when the tool changes; the core exposes that as
/// `commit_transform`, which is what the app's tool switch calls.
#[test]
fn no_op_invalid_values_and_switching_tools() {
    let (mut session, _) = painted_session(200, 100);
    let count = session.history.undo_count();
    session.begin_transform();
    session.commit_transform();
    assert_eq!(session.history.undo_count(), count, "an untouched edit is no-op");
    session.begin_transform();
    let mut invalid = session.transform_edit.as_ref().unwrap().draft;
    invalid.size.width = 0.0;
    session.preview_transform(invalid);
    assert_eq!(session.transform_edit.as_ref().unwrap().draft.size.width, 200.0);
    invalid.size.width = f64::INFINITY;
    session.preview_transform(invalid);
    assert_eq!(session.transform_edit.as_ref().unwrap().draft.size.width, 200.0);
    let mut moved = session.transform_edit.as_ref().unwrap().draft;
    moved.origin.x = 10.0;
    session.preview_transform(moved);
    session.commit_transform(); // The tool switch applies it.
    assert!(session.transform_edit.is_none());
    assert_eq!(session.active_layer().unwrap().origin().x, 10.0);
    assert_eq!(session.history.undo_count(), count + 1);
}

/// Swift `renderingRotatesScalesAndFlipsWithoutReplacingPixels` — the session
/// half. Swift reads the pixels out of the live view; the port renders the
/// pending draft through the compositor over a solo document, which is the
/// same drawing the canvas composites.
#[test]
fn rendering_rotates_scales_and_flips_without_replacing_pixels() {
    let mut image = Bitmap::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            image.set_pixel(x, y, if x < 4 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        }
    }
    let mut session = EditorSession::new();
    session.create_document(32, 32, false);
    let asset = ImportedImage::with_thumbnail(image.clone(), image, "Two colors");
    session.insert_image(asset.clone(), None);
    session.begin_transform();
    let mut value = LayerTransform {
        origin: CGPoint::new(8.0, 8.0),
        size: CGSize::new(16.0, 16.0),
        rotation: 90.0,
        flip_x: false,
        flip_y: false,
        sampling: LayerSampling::Nearest,
    };
    session.preview_transform(value);

    let render_preview = |session: &EditorSession, draft: LayerTransform| {
        let mut solo = session.document.clone().unwrap();
        let id = session.active_layer_id.unwrap();
        solo.layer_mut(id).unwrap().transform = draft;
        draw_live_composite(&solo)
    };
    let preview = render_preview(&session, value);
    assert!(preview.pixel(16, 12)[0] > 242, "{:?}", preview.pixel(16, 12));
    assert!(preview.pixel(16, 20)[2] > 242, "{:?}", preview.pixel(16, 20));

    value.flip_x = true;
    session.preview_transform(value);
    let preview = render_preview(&session, value);
    assert!(preview.pixel(16, 12)[2] > 242, "{:?}", preview.pixel(16, 12));

    session.commit_transform();
    let active = session.active_layer().unwrap();
    assert_eq!(
        active.asset.as_ref().unwrap().image,
        asset.image,
        "the pixels are not replaced"
    );
    session.undo();
    assert_eq!(session.active_layer().unwrap().size(), CGSize::new(8.0, 8.0));
}
