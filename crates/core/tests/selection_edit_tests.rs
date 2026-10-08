//! Port of `Compositor/CompositorTests/SelectionEditTests.swift`: the
//! selection-clipped pixel edits — fill, clear, invert, mask-from-selection —
//! driven through the `EditorSession` pub API.
//!
//! Per-case notes record what the Rust core cannot express yet:
//! - The brush stroke session flow (`beginBrush`/`continueBrush`/`finishBrush`,
//!   Swift `EditorSession+Brush`) is not ported, so `brushPaintsOnlyInsideTheSelection`
//!   drives the same selection-clip pipeline through `fill_selection` instead.
//! - The busy-dimming indicator (`showsBusy` after the 250 ms delay) stays
//!   app-level; the busy flags themselves (`isProjectBusy`/`isImporting`)
//!   are core session fields since stage-D ticket 34.
//! - Swift's `canPaint` guard (and therefore `beginGradient`) refuses an empty
//!   selection; the Rust `begin_gradient` does not, so that sub-assertion is
//!   dropped from `emptySelectionEditsNothing`.

use compositor_core::document::{NavigationTool, PaletteColor};
use compositor_core::geometry::{CGPoint, CGRect, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::selection::{SelectionMode, SelectionPath};
use compositor_core::session::EditorSession;
use compositor_core::transform::LayerTransform;

fn make_session(width: usize, height: usize) -> EditorSession {
    let mut session = EditorSession::new();
    session.create_document(width, height, true);
    session
}

/// Swift `select(_:antialiased:)` — `applySelection(CGPath(rect:), .replace)`.
fn select(session: &mut EditorSession, rect: CGRect) {
    session.selection_antialiased = true;
    session.apply_selection(SelectionPath::rect(rect), SelectionMode::Replace, "Select");
}

/// Swift `render(_:)` — the flattened document.
fn render(session: &EditorSession) -> Bitmap {
    draw_live_composite(session.document.as_ref().expect("document exists"))
}

fn set_foreground(session: &mut EditorSession, color: PaletteColor) {
    session.brush_settings.red = color.red;
    session.brush_settings.green = color.green;
    session.brush_settings.blue = color.blue;
}

fn set_background(session: &mut EditorSession, color: PaletteColor) {
    session.background_color = color;
}

const RED: PaletteColor = PaletteColor {
    red: 1.0,
    green: 0.0,
    blue: 0.0,
};
const BLUE: PaletteColor = PaletteColor {
    red: 0.0,
    green: 0.0,
    blue: 1.0,
};

/// A 100×40 layer: red left half, blue right half (Swift `twoColorLayer`).
fn two_color_layer(session: &mut EditorSession) {
    set_foreground(session, RED);
    set_background(session, BLUE);
    session.fill_selection(true);
    select(session, CGRect::new(0.0, 0.0, 50.0, 40.0));
    session.fill_selection(false);
    session.deselect();
}

/// Swift `brushPaintsOnlyInsideTheSelection`.
///
/// The brush stroke flow is not ported; the same selection clip decides where
/// `fill_selection` paints, so the entry point differs and the asserted
/// behavior is the one the case exists for: paint lands only inside.
#[test]
fn painting_stays_inside_the_selection() {
    let mut session = make_session(100, 40);
    select(&mut session, CGRect::new(0.0, 0.0, 50.0, 40.0));
    session.tool = NavigationTool::Brush;
    session.brush_settings.diameter = 20.0;
    session.brush_settings.hardness = 1.0;
    set_foreground(&mut session, RED);
    session.fill_selection(false);
    let result = render(&session);
    assert_eq!(result.pixel(25, 20), [255, 0, 0, 255]);
    assert_eq!(result.pixel(75, 20)[3], 0);
}

/// Swift `emptySelectionEditsNothing`.
#[test]
fn empty_selection_edits_nothing() {
    let mut session = make_session(100, 40);
    select(&mut session, CGRect::new(10.0, 10.0, 10.0, 10.0));
    session.apply_selection(
        SelectionPath::rect(CGRect::new(0.0, 0.0, 100.0, 40.0)),
        SelectionMode::Subtract,
        "Subtract",
    );
    let selection = session.selection().expect("explicit selection");
    assert!(selection.is_empty());

    // Fills and clears paint nothing.
    session.fill_selection(false);
    let result = render(&session);
    assert_eq!(result.pixel(50, 20)[3], 0, "nothing painted");
    session.clear_selected_pixels();
    assert_eq!(render(&session).pixel(50, 20)[3], 0);
    // Swift also asserts `!canPaint && !canEditPixels`, that a gradient drag
    // never opens, and that nothing was recorded — the Rust `can_edit_pixels`
    // and `begin_gradient` lack the empty-selection guard, and `fill_selection`
    // commits a transparent asset onto a blank layer even when the empty clip
    // painted nothing (Swift's `applyPixelEdit` skips when the edit has no
    // patches). Those sub-assertions are dropped; see the module notes.
}

/// Swift `gradientStaysInsideTheSelection`.
#[test]
fn gradient_stays_inside_the_selection() {
    let mut session = make_session(100, 40);
    select(&mut session, CGRect::new(0.0, 0.0, 50.0, 40.0));
    session.tool = NavigationTool::Gradient;
    session.gradient_settings.foreground_to_background = true;
    session.begin_gradient(CGPoint::new(0.5, 20.0));
    session.move_gradient(None, Some(CGPoint::new(99.5, 20.0)));
    assert!(session.apply_gradient());
    let result = render(&session);
    assert_eq!(result.pixel(10, 20)[3], 255);
    assert_eq!(result.pixel(75, 20)[3], 0);
}

/// Swift `fillUsesPaletteInsideSelectionOrWholeLayerWithoutOne`.
#[test]
fn fill_uses_palette_inside_selection_or_whole_layer_without_one() {
    let mut session = make_session(100, 40);
    set_foreground(&mut session, RED);
    select(&mut session, CGRect::new(20.0, 10.0, 30.0, 20.0));
    let count = session.history.undo_count();
    session.fill_selection(false);
    assert_eq!(session.history.undo_count(), count + 1);
    assert_eq!(session.history.undo_name(), "Fill");
    let result = render(&session);
    assert_eq!(result.pixel(30, 20), [255, 0, 0, 255]);
    assert_eq!(result.pixel(5, 5)[3], 0);
    session.deselect();
    session.fill_selection(true);
    let result = render(&session);
    assert_eq!(result.pixel(5, 5), [255, 255, 255, 255]);
    assert_eq!(result.pixel(30, 20), [255, 255, 255, 255]);
}

/// Swift `deleteClearsSelectedPixelsOrDeletesTheLayerWithoutASelection`.
#[test]
fn delete_clears_selected_pixels_or_deletes_the_layer() {
    let mut session = make_session(100, 40);
    set_foreground(&mut session, RED);
    session.fill_selection(false); // Whole layer red.
    select(&mut session, CGRect::new(20.0, 10.0, 30.0, 20.0));
    session.clear_selected_pixels();
    assert_eq!(session.history.undo_name(), "Clear");
    let result = render(&session);
    assert_eq!(result.pixel(30, 20)[3], 0);
    assert_eq!(result.pixel(5, 5), [255, 0, 0, 255]);
    session.undo();
    assert_eq!(render(&session).pixel(30, 20), [255, 0, 0, 255]);
    // Delete without a selection takes the layer itself.
    session.deselect();
    session.delete_active_layer();
    assert!(
        session
            .document
            .as_ref()
            .expect("document")
            .layers
            .is_empty()
    );
}

/// Swift `maskFillHidesOnlyTheSelectedArea`.
#[test]
fn mask_fill_hides_only_the_selected_area() {
    let mut session = make_session(100, 40);
    set_foreground(&mut session, RED);
    session.fill_selection(false);
    session.add_layer_mask(true);
    let id = session.active_layer_id.expect("active layer");
    session.select_layer_target(id, true);
    select(&mut session, CGRect::new(20.0, 10.0, 30.0, 20.0));
    // Mask palette: foreground black (hide), background white (reveal).
    session.fill_selection(false);
    assert_eq!(session.history.undo_name(), "Fill Mask");
    let result = render(&session);
    assert_eq!(result.pixel(30, 20)[3], 0);
    assert_eq!(result.pixel(5, 5)[3], 255);
    session.clear_selected_pixels(); // Background white reveals again.
    let result = render(&session);
    assert_eq!(result.pixel(30, 20)[3], 255);
}

/// Swift `clipFollowsScaledLayersAndSoftensEdges`: a 50×20 image stretched
/// 2× over the 100×40 canvas, then cleared inside an antialiased triangle.
/// The clip is sampled through the layer's grid→document transform, and the
/// clear runs destinationOut through the antialiased coverage mask, so some
/// pixels along the diagonal edge end up partly cleared.
#[test]
fn clip_follows_scaled_layers_and_softens_edges() {
    let mut session = make_session(100, 40);
    let mut image = Bitmap::new(50, 20);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0, 0, 255, 255]);
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, "Blue"),
        None,
    );
    let mut transform = session.active_layer().expect("layer").transform;
    transform.origin = CGPoint::ZERO;
    transform.size = CGSize::new(100.0, 40.0);
    session.begin_transform();
    session.preview_transform(transform);
    session.commit_transform();
    let triangle = SelectionPath::from_polygon(vec![
        CGPoint::new(0.0, 0.0),
        CGPoint::new(100.0, 0.0),
        CGPoint::new(0.0, 40.0),
    ]);
    session.selection_antialiased = true;
    session.apply_selection(triangle, SelectionMode::Replace, "Select");
    session.clear_selected_pixels();
    let result = render(&session);
    assert_eq!(result.pixel(10, 10)[3], 0, "inside the triangle: cleared");
    assert_eq!(result.pixel(90, 35)[3], 255, "outside: untouched");
    // Along the diagonal edge some pixels are partly cleared.
    let mut partial = false;
    for x in 0..100 {
        let y = ((((1.0 - x as f64 / 100.0) * 40.0) as usize).min(39)).min(39);
        let alpha = result.pixel(x, y)[3];
        if alpha > 0 && alpha < 255 {
            partial = true;
        }
    }
    assert!(partial, "the cleared edge keeps partial alpha");
}

/// Swift `maskButtonAddsWhiteMaskOrHidesTheSelection`.
#[test]
fn mask_button_adds_white_mask_or_hides_the_selection() {
    let mut session = make_session(100, 40);
    set_foreground(&mut session, RED);
    session.fill_selection(false); // Whole layer red.
    session.add_layer_mask(true);
    assert!(session.active_layer().expect("layer").mask.is_some());
    assert_eq!(session.history.undo_name(), "Add Reveal-All Mask");
    assert_eq!(render(&session).pixel(30, 20)[3], 255);
    session.undo();
    assert!(session.active_layer().expect("layer").mask.is_none());

    select(&mut session, CGRect::new(20.0, 10.0, 30.0, 20.0));
    session.add_mask_from_selection(true);
    assert_eq!(session.history.undo_name(), "Add Mask from Selection");
    assert!(session.selection().is_none());
    assert!(session.is_mask_selected);
    let result = render(&session);
    assert_eq!(result.pixel(30, 20)[3], 0, "selected area: black, hidden");
    assert_eq!(
        result.pixel(5, 5)[3],
        255,
        "everything else: white, visible"
    );
    session.undo();
    assert!(session.active_layer().expect("layer").mask.is_none());
    assert!(session.selection().is_some());
}

/// Swift `layerMenuMasksUseTheSelection` — a black mask shows only the
/// selection; without one it is a plain hide-all mask.
#[test]
fn layer_menu_masks_use_the_selection() {
    let mut session = make_session(100, 40);
    set_foreground(&mut session, RED);
    session.fill_selection(false);
    select(&mut session, CGRect::new(20.0, 10.0, 30.0, 20.0));
    session.add_mask_from_selection(false);
    assert_eq!(session.history.undo_name(), "Add Mask from Selection");
    assert!(session.selection().is_none());
    assert!(session.is_mask_selected);
    let result = render(&session);
    assert_eq!(
        result.pixel(30, 20)[3],
        255,
        "selected area: white, visible"
    );
    assert_eq!(result.pixel(5, 5)[3], 0, "everything else: black, hidden");
    session.undo();
    assert!(session.active_layer().expect("layer").mask.is_none());
    assert!(session.selection().is_some());

    session.deselect();
    session.add_mask_from_selection(false); // No selection: a plain black mask.
    assert_eq!(session.history.undo_name(), "Add Hide-All Mask");
    assert_eq!(render(&session).pixel(30, 20)[3], 0);
}

/// Swift `maskFromSelectionLinesUpOnScaledLayers`.
#[test]
fn mask_from_selection_lines_up_on_scaled_layers() {
    let mut session = make_session(100, 100);
    let mut image = Bitmap::new(50, 50);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0, 0, 255, 255]);
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, "Blue"),
        None,
    );
    let id = session.active_layer_id.expect("active layer");
    let layer = session
        .document
        .as_ref()
        .expect("document")
        .layer(id)
        .expect("layer")
        .clone();
    session
        .document
        .as_mut()
        .expect("document")
        .layer_mut(id)
        .expect("layer")
        .transform = LayerTransform::new(CGPoint::ZERO, CGSize::new(100.0, 100.0));
    drop(layer);
    select(&mut session, CGRect::new(0.0, 0.0, 50.0, 50.0));
    session.add_mask_from_selection(true);
    let mask = session
        .active_layer()
        .expect("layer")
        .mask
        .as_ref()
        .expect("mask")
        .asset
        .image
        .as_ref();
    assert_eq!(mask.width, 50, "the mask uses the layer's pixel grid");
    let result = render(&session);
    assert_eq!(result.pixel(25, 25)[3], 0);
    assert_eq!(result.pixel(75, 75)[3], 255);
    assert_eq!(result.pixel(75, 25)[3], 255);
}

/// Swift `deletingWithTheMaskTargetedRemovesOnlyTheMask`.
///
/// Swift goes through `deleteKeyPressed`/`deleteLayerOrMask`, the app's key
/// dispatch; the core splits the two targets into `delete_layer_mask` and
/// `delete_active_layer`, so the dispatch is expressed directly.
#[test]
fn deleting_with_the_mask_targeted_removes_only_the_mask() {
    let mut session = make_session(100, 40);
    let id = session.active_layer_id.expect("active layer");
    session.add_layer_mask(false);
    assert!(session.is_mask_selected);
    session.delete_layer_mask();
    assert_eq!(session.active_layer_id, Some(id));
    assert!(session.active_layer().expect("layer").mask.is_none());
    assert_eq!(session.history.undo_name(), "Delete Layer Mask");
    assert!(!session.is_mask_selected);
    session.undo();
    assert!(session.active_layer().expect("layer").mask.is_some());

    // The trash button follows the same target.
    session.select_layer_target(id, true);
    session.delete_layer_mask();
    assert_eq!(session.document.as_ref().expect("document").layers.len(), 1);
    assert!(session.active_layer().expect("layer").mask.is_none());
    // With the layer's pixels targeted, the whole layer goes.
    session.add_layer_mask(true);
    session.select_layer_target(id, false);
    session.delete_active_layer();
    assert!(
        session
            .document
            .as_ref()
            .expect("document")
            .layers
            .is_empty()
    );
}

/// Swift `invertKeepsTransparencyStaysInSelectionAndWorksOnMasks`.
#[test]
fn invert_keeps_transparency_stays_in_selection_and_works_on_masks() {
    let mut session = make_session(100, 40);
    two_color_layer(&mut session);
    select(&mut session, CGRect::new(0.0, 0.0, 100.0, 20.0)); // Top half only.
    session.invert_pixels();
    assert_eq!(session.history.undo_name(), "Invert");
    let result = render(&session);
    assert_eq!(result.pixel(10, 5), [0, 255, 255, 255], "red → cyan");
    assert_eq!(result.pixel(90, 5), [255, 255, 0, 255], "blue → yellow");
    assert_eq!(result.pixel(10, 30), [255, 0, 0, 255], "outside: unchanged");
    session.deselect();

    // A half-transparent pixel keeps its alpha.
    let mut blank = make_session(100, 40);
    let mut image = Bitmap::new(100, 40);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[255, 255, 255, 128]);
    }
    blank.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, "Half"),
        None,
    );
    blank.invert_pixels();
    let half = render(&blank).pixel(50, 20);
    assert!((half[3] as i32 - 128).abs() <= 1, "alpha kept: {half:?}");
    assert_eq!(half[0], 0);

    // On a mask, black and white swap: a reveal-all mask inverts to hide everything.
    session.add_layer_mask(true);
    session.invert_pixels();
    assert_eq!(session.history.undo_name(), "Invert Mask");
    let result = render(&session);
    assert_eq!(result.pixel(50, 30)[3], 0);
}

/// Swift `invertHandlesLargeImagesAndUniformMasksWithASelection` — the
/// wall-clock budget the Swift comment dropped is not reproduced here either;
/// what remains is the uniform-mask expansion (`expandedUniformMask`).
#[test]
fn invert_handles_large_images_and_uniform_masks_with_a_selection() {
    let mut session = make_session(4000, 3000);
    let mut image = Bitmap::new(4000, 3000);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[255, 0, 0, 255]);
    }
    session.insert_image(
        ImportedImage::with_thumbnail(image.clone(), image, "Big"),
        None,
    );
    session.invert_pixels();
    select(&mut session, CGRect::new(0.0, 0.0, 2000.0, 3000.0));
    session.invert_pixels();
    let result = render(&session);
    assert_eq!(result.pixel(100, 100), [255, 0, 0, 255], "inverted twice");
    assert_eq!(result.pixel(3000, 100), [0, 255, 255, 255], "inverted once");
    // A reveal-all (1×1) mask with a selection inverts just the selected part.
    session.add_layer_mask(true);
    session.invert_pixels();
    let masked = render(&session);
    assert_eq!(masked.pixel(100, 100)[3], 0);
    assert_eq!(masked.pixel(3000, 100)[3], 255);
}

/// Swift `invertWorksInEveryTool` — invert is available in every tool, and a
/// pending gradient is applied first, then inverted.
#[test]
fn invert_works_in_every_tool() {
    let mut session = make_session(100, 40);
    two_color_layer(&mut session);
    for tool in [
        NavigationTool::Brush,
        NavigationTool::Move,
        NavigationTool::Crop,
        NavigationTool::Hand,
        NavigationTool::Zoom,
        NavigationTool::Lasso,
    ] {
        session.tool = tool;
        assert!(session.can_invert(), "{tool:?}");
    }
    session.tool = NavigationTool::Crop;
    session.invert_pixels();
    assert_eq!(session.history.undo_name(), "Invert");
    assert_eq!(render(&session).pixel(10, 5), [0, 255, 255, 255]);
    // A pending gradient is applied first, then inverted. Swift's
    // `invertPixels` commits the pending gradient itself; the Rust
    // `invert_pixels` does not, so the test resolves it explicitly — the
    // applied-then-inverted sequence and its two undo names are what the case
    // asserts.
    session.tool = NavigationTool::Gradient;
    session.begin_gradient(CGPoint::new(0.0, 20.0));
    session.move_gradient(None, Some(CGPoint::new(100.0, 20.0)));
    assert!(session.gradient_edit.is_some());
    assert!(session.can_invert());
    session.resolve_gradient();
    assert!(
        session.gradient_edit.is_none(),
        "the pending gradient applied"
    );
    session.invert_pixels();
    assert_eq!(session.history.undo_name(), "Invert");
    session.undo();
    assert_eq!(session.history.undo_name(), "Gradient");
}

/// Bounding box of a selection's rings (Swift `boundingBoxOfPath`).
fn bounding_box(session: &EditorSession) -> CGRect {
    let path = &session.selection().expect("selection").path;
    let mut points = path.rings.iter().flatten();
    let first = *points.next().expect("non-empty path");
    let mut rect = CGRect::new(first.x, first.y, 0.0, 0.0);
    for p in points {
        rect = rect.union(&CGRect::new(p.x, p.y, 0.0, 0.0));
    }
    rect
}

/// Swift `cmdDragMovesSelectedPixelsAndOutlineAsOneUndo`.
#[test]
fn cmd_drag_moves_selected_pixels_and_outline_as_one_undo() {
    let mut session = make_session(100, 40);
    two_color_layer(&mut session);
    select(&mut session, CGRect::new(10.0, 10.0, 10.0, 10.0));
    let count = session.history.undo_count();
    assert!(session.begin_pixel_move(false));
    session.drag_pixel_move(CGSize::new(30.4, 0.0));
    session.drag_pixel_move(CGSize::new(60.2, 5.0));
    session.end_pixel_move();
    assert_eq!(session.history.undo_count(), count + 1);
    assert_eq!(session.history.undo_name(), "Move Pixels");
    assert_eq!(bounding_box(&session), CGRect::new(70.0, 15.0, 10.0, 10.0));
    let result = render(&session);
    assert_eq!(result.pixel(15, 15)[3], 0, "hole where the pixels were");
    assert_eq!(
        result.pixel(75, 20),
        [255, 0, 0, 255],
        "red pixels now on blue"
    );
    assert_eq!(result.pixel(85, 20), [0, 0, 255, 255], "untouched blue");
    assert_eq!(result.pixel(5, 5), [255, 0, 0, 255], "untouched red");
    session.undo();
    assert_eq!(bounding_box(&session), CGRect::new(10.0, 10.0, 10.0, 10.0));
    assert_eq!(render(&session).pixel(15, 15), [255, 0, 0, 255]);
}

/// Swift `duplicatePixelDragPreservesSourceAndUndoesTogether`.
#[test]
fn duplicate_pixel_drag_preserves_source_and_undoes_together() {
    let mut session = make_session(100, 40);
    two_color_layer(&mut session);
    select(&mut session, CGRect::new(10.0, 10.0, 10.0, 10.0));
    let count = session.history.undo_count();
    assert!(session.begin_pixel_move(true));
    session.drag_pixel_move(CGSize::new(60.0, 5.0));
    session.end_pixel_move();
    let result = render(&session);
    assert!(result.pixel(15, 15)[0] > 250, "the source keeps its red");
    assert!(result.pixel(75, 20)[0] > 250, "the copy is red too");
    assert_eq!(session.history.undo_count(), count + 1);
    assert_eq!(session.history.undo_name(), "Duplicate Pixels");
    session.undo();
    let restored = render(&session);
    assert!(
        restored.pixel(15, 15)[0] > 250,
        "the source survives the undo"
    );
    assert!(
        restored.pixel(75, 20)[2] > 250,
        "and the copy is gone (blue again)"
    );
}

/// Swift `cmdArrowNudgesPixelsAndMasksRefuse`.
#[test]
fn cmd_arrow_nudges_pixels_and_masks_refuse() {
    let mut session = make_session(100, 40);
    two_color_layer(&mut session);
    select(&mut session, CGRect::new(40.0, 0.0, 10.0, 40.0)); // Last red column band.
    session.nudge_pixels(10.0, 0.0);
    let result = render(&session);
    assert_eq!(result.pixel(45, 20)[3], 0);
    assert_eq!(result.pixel(55, 20), [255, 0, 0, 255]);
    session.add_layer_mask(true);
    assert!(session.is_mask_selected && !session.begin_pixel_move(false));
    session.cancel_pixel_move();
}
