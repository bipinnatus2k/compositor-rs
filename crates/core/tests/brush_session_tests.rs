//! Port of `Compositor/CompositorTests/BrushTests.swift`: the brush's raster
//! behavior — stroke geometry, soft tips, opacity caps, mask painting, and the
//! hardness/opacity keys.
//!
//! The Swift cases drive `EditorSession.beginBrush/continueBrush/finishBrush`
//! (`EditorSession+Brush`). That session flow is not ported yet — the Rust
//! core exposes the engine behind it, `tools::brush::RasterEdit`, whose
//! `append`/`flush`/`result`/`paint_snapshot` are exactly what the session
//! calls. Every case here therefore drives `RasterEdit` directly and renders
//! its `paint_snapshot` through the compositor; the session halves of the
//! cases (undo/redo of a stroke, save/reload, `cancelBrush` restoring the
//! document, `brushStroke`/`brushError` state) have no core entry and are
//! recorded in the module notes per test:
//! - `bracketKeysReachTheBrushWhereverFocusIsExceptTextFields` and
//!   `shiftBracketsStepHardnessFromTheCanvas` are app key routing (`]`/`[` /
//!   Shift variants); only the core `change_brush_hardness` and
//!   `type_opacity_digit` halves are portable.
//! - `largeBlankCanvasOnlyAllocatesTouchedTilesUntilCommit` has no
//!   equivalent: the Rust engine keeps whole-grid buffers by design, so the
//!   tiling assertion would only measure memory.

use compositor_core::document::{CanvasDocument, ImageLayer, LayerMask, NavigationTool};
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::session::EditorSession;
use compositor_core::tools::brush::{BrushSettings, RasterEdit};
use compositor_core::transform::LayerTransform;

fn settings(diameter: f64, hardness: f64, red: f64, green: f64, blue: f64) -> BrushSettings {
    BrushSettings {
        diameter,
        hardness,
        red,
        green,
        blue,
        ..BrushSettings::default()
    }
}

/// A paint edit over a blank canvas-sized layer (Swift `BrushStroke(layer:
/// …)` with no source): grid pixels equal document pixels.
fn blank_edit(width: usize, height: usize, settings: BrushSettings) -> RasterEdit {
    RasterEdit::new(
        None,
        LayerTransform::new(CGPoint::ZERO, CGSize::new(width as f64, height as f64)),
        false,
        settings,
        CGSize::new(width as f64, height as f64),
        None,
    )
}

/// Renders the edit's committed snapshot as the only layer of its canvas.
fn render_edit(edit: &RasterEdit, width: usize, height: usize) -> Bitmap {
    let (asset, transform, _) = edit.paint_snapshot();
    let mut layer = ImageLayer::with_asset(asset, transform.origin);
    layer.transform = transform;
    let mut document = CanvasDocument::new(width, height);
    document.layers.push(layer);
    draw_live_composite(&document)
}

/// Swift `continuousStrokeCrossesTilesAndCommitsOneUndo` — the raster half.
/// The Rust engine keeps whole-grid buffers, so there are no tiles to cross;
/// the asserted behavior is the unbroken stroke and the clean background. The
/// one-undo commit is session flow (module notes).
#[test]
fn continuous_stroke_crosses_the_canvas() {
    let mut edit = blank_edit(600, 80, settings(20.0, 1.0, 1.0, 0.0, 0.0));
    edit.append(CGPoint::new(20.0, 40.0));
    edit.append(CGPoint::new(580.0, 40.0));
    assert!(edit.has_paint());
    let preview = edit.result();
    for x in [20usize, 255, 256, 511, 512, 579] {
        assert_eq!(preview.pixel(x, 40), [255, 0, 0, 255], "x={x}");
    }
    edit.flush();
    let result = render_edit(&edit, 600, 80);
    for x in [20usize, 255, 256, 511, 512, 579] {
        assert_eq!(result.pixel(x, 40), [255, 0, 0, 255], "x={x}");
    }
    assert_eq!(result.pixel(300, 0)[3], 0);
}

/// Swift `softBrushProducesPartialAlphaAndCancelPreservesDocument` — the
/// alpha profile of a 0%-hardness tip; dropping the edit without committing is
/// the port's `cancelBrush`.
#[test]
fn soft_brush_produces_partial_alpha() {
    let mut edit = blank_edit(80, 80, settings(40.0, 0.0, 1.0, 0.0, 0.0));
    edit.append(CGPoint::new(40.0, 40.0));
    let live = edit.result();
    assert!(live.pixel(40, 40)[3] > 230, "{:?}", live.pixel(40, 40));
    // 0% hardness feathers along a Gaussian across the whole radius: about
    // half strength at half radius, faint near the rim, nothing beyond it.
    let half = live.pixel(50, 40)[3];
    assert!(half > 95 && half < 140, "{half}");
    assert!(live.pixel(57, 40)[3] < 40);
    assert_eq!(live.pixel(64, 40)[3], 0);
    // A stroke entirely off the canvas paints nothing.
    let mut off = blank_edit(80, 80, settings(40.0, 1.0, 1.0, 0.0, 0.0));
    off.append(CGPoint::new(-100.0, -100.0));
    off.flush();
    assert!(!off.has_paint());
    assert_eq!(off.result().pixel(40, 40)[3], 0);
}

/// Swift `softMaskPaintingPreviewMatchesCommitAndPersists` — the mask-raster
/// half. In the port the preview and the committed raster are one `result()`;
/// the save/reload half is project IO covered by the project tests.
#[test]
fn soft_mask_painting_hides_and_repaints() {
    // The mask grid starts white; a soft black dab hides through its feather.
    let mut edit = RasterEdit::new(
        None,
        LayerTransform::new(CGPoint::ZERO, CGSize::new(80.0, 80.0)),
        true,
        settings(40.0, 0.0, 0.0, 0.0, 0.0),
        CGSize::new(80.0, 80.0),
        None,
    );
    edit.append(CGPoint::new(40.0, 40.0));
    edit.flush();
    let mask = edit.result();
    // The same columns are compared against the committed render below.
    // Commit the grid as the mask of an opaque red layer.
    let (asset, transform, _) = edit.paint_snapshot();
    let mut layer = ImageLayer::with_asset(red(80, 80), CGPoint::ZERO);
    layer.transform = LayerTransform::new(CGPoint::ZERO, CGSize::new(80.0, 80.0));
    layer.mask = Some(LayerMask {
        asset,
        is_enabled: true,
        placement: Some(transform),
        is_linked: true,
    });
    let mut document = CanvasDocument::new(80, 80);
    document.layers.push(layer);
    let result = draw_live_composite(&document);
    for x in [10usize, 40, 54, 70] {
        let expected = mask.pixel(x, 40)[0];
        let alpha = result.pixel(x, 40)[3];
        // The committed mask is resampled (smooth) as it multiplies the layer,
        // a little softer than the raw grid bytes on the feather's slope.
        assert!(
            (alpha as i32 - expected as i32).abs() <= 10,
            "x={x}: {alpha} vs {expected}"
        );
    }
    assert!(result.pixel(40, 40)[3] < 25, "the center hides");
    // Painting white with a hard tip reveals again.
    let mut white = RasterEdit::new(
        None,
        LayerTransform::new(CGPoint::ZERO, CGSize::new(80.0, 80.0)),
        true,
        settings(200.0, 1.0, 1.0, 1.0, 1.0),
        CGSize::new(80.0, 80.0),
        None,
    );
    white.append(CGPoint::new(40.0, 40.0));
    white.flush();
    assert_eq!(white.result().pixel(40, 40)[0], 255);
}

/// Swift `brushStaysCircularOnNonuniformRotatedFlippedLayer`.
#[test]
fn brush_stays_circular_on_nonuniform_rotated_flipped_layer() {
    let mut source = Bitmap::new(100, 100);
    for y in 2..8 {
        for x in 2..8 {
            source.set_pixel(x, y, [255, 0, 0, 255]);
        }
    }
    let base_transform = LayerTransform {
        origin: CGPoint::new(25.0, -50.0),
        size: CGSize::new(50.0, 200.0),
        rotation: 90.0,
        flip_x: true,
        flip_y: true,
        sampling: compositor_core::transform::LayerSampling::Nearest,
    };
    let mut edit = RasterEdit::new(
        Some(source),
        base_transform,
        false,
        settings(16.0, 1.0, 0.0, 1.0, 0.0),
        CGSize::new(100.0, 100.0),
        None,
    );
    edit.append(CGPoint::new(50.0, 50.0));
    edit.flush();
    let result = render_edit(&edit, 100, 100);
    for (x, y) in [(50usize, 50usize), (54, 50), (50, 54)] {
        assert!(
            result.pixel(x, y)[1] > 240,
            "({x},{y}): {:?}",
            result.pixel(x, y)
        );
    }
    for (x, y) in [(61usize, 50usize), (50, 61)] {
        assert_eq!(result.pixel(x, y)[3], 0, "({x},{y}) stays clear");
    }
}

/// Swift `maskedImagePaintingUsesCoverageAndOpacityOnlyOnce`: the painted
/// stroke is fully opaque in the raster; the 0.5 layer opacity and the 0.5
/// mask each apply once.
#[test]
fn masked_image_painting_uses_coverage_and_opacity_once() {
    let mut edit = blank_edit(520, 40, settings(20.0, 1.0, 1.0, 0.0, 0.0));
    edit.append(CGPoint::new(240.0, 20.0));
    edit.append(CGPoint::new(280.0, 20.0));
    edit.flush();
    let (asset, transform, _) = edit.paint_snapshot();
    let mut layer = ImageLayer::with_asset(asset, transform.origin);
    layer.transform = transform;
    layer.opacity = 0.5;
    let mut gray = Bitmap::new(1, 1);
    gray.set_pixel(0, 0, [128, 128, 128, 255]);
    layer.mask = Some(LayerMask {
        asset: ImportedImage::new(gray, "Layer Mask"),
        is_enabled: true,
        placement: None,
        is_linked: true,
    });
    let mut document = CanvasDocument::new(520, 40);
    document.layers.push(layer);
    let result = draw_live_composite(&document);
    for x in [250usize, 255, 256, 260] {
        let alpha = result.pixel(x, 20)[3];
        assert!((alpha as i32 - 64).abs() <= 1, "x={x}: {alpha}");
    }
}

/// Swift `paintedBoundsTrimTilePaddingAndKeepSoftEdges` — the snapshot trims
/// to the painted bounds and keeps the soft rim; undo/redo is session flow.
#[test]
fn painted_bounds_trim_and_keep_soft_edges() {
    for hardness in [0.0, 1.0] {
        let mut edit = blank_edit(600, 200, settings(20.0, hardness, 1.0, 0.0, 0.0));
        edit.append(CGPoint::new(300.0, 100.0));
        edit.flush();
        let live = edit.result();
        let (asset, transform, _) = edit.paint_snapshot();
        let image = asset.image.as_ref().clone();
        assert!(image.width <= 20, "{}", image.width);
        assert!(image.height <= 20, "{}", image.height);
        assert!(transform.origin.x >= 290.0, "{:?}", transform.origin);
        assert!(transform.origin.y >= 90.0, "{:?}", transform.origin);
        // Every border row and column keeps a touched pixel.
        let mut edges = Vec::new();
        for y in [0, image.height - 1] {
            edges.push(
                (0..image.width)
                    .map(|x| image.pixel(x, y)[3])
                    .collect::<Vec<_>>(),
            );
        }
        for x in [0, image.width - 1] {
            edges.push(
                (0..image.height)
                    .map(|y| image.pixel(x, y)[3])
                    .collect::<Vec<_>>(),
            );
        }
        for edge in &edges {
            assert!(edge.iter().any(|a| *a > 0), "hardness {hardness}: {edge:?}");
        }
        // The committed snapshot renders exactly where the grid painted (the
        // compositor's color convention for semi-transparent pixels differs
        // per path, so alpha is the comparable channel).
        let mut layer = ImageLayer::with_asset(asset, transform.origin);
        layer.transform = transform;
        let mut document = CanvasDocument::new(600, 200);
        document.layers.push(layer);
        let result = draw_live_composite(&document);
        for x in 288..=312 {
            assert_eq!(
                result.pixel(x, 100)[3],
                live.pixel(x, 100)[3],
                "hardness {hardness} x={x}"
            );
        }
    }
}

/// Swift `opacityCapsTheWholeStrokeEvenWhereItOverlapsItself`.
#[test]
fn opacity_caps_the_whole_stroke_even_where_it_overlaps_itself() {
    let mut edit = blank_edit(
        200,
        80,
        BrushSettings {
            opacity: 0.5,
            ..settings(20.0, 1.0, 1.0, 0.0, 0.0)
        },
    );
    // Back and forth over the same pixels many times.
    edit.append(CGPoint::new(20.0, 40.0));
    for x in [180.0, 20.0, 180.0, 20.0, 100.0] {
        edit.append(CGPoint::new(x, 40.0));
    }
    edit.flush();
    let preview = edit.result();
    let center = preview.pixel(100, 40);
    assert!((center[3] as i32 - 128).abs() <= 1, "{center:?}");
    // Swift reads the premultiplied render (red 128 at alpha 128); the raster
    // keeps straight color, so the same pixel is red 255 at alpha 128 —
    // premultiplied back: 255 × 128 / 255 = 128.
    let premultiplied_red = center[0] as i32 * center[3] as i32 / 255;
    assert!((premultiplied_red - 128).abs() <= 1, "{center:?}");
    assert_eq!(center[1], 0);
    let result = render_edit(&edit, 200, 80);
    // Alpha survives the compositor exactly; its color convention for
    // semi-transparent pixels differs from the raster's (see the module
    // notes), so the render comparison sticks to the alpha channel.
    assert_eq!(result.pixel(100, 40)[3], center[3]);
    assert_eq!(result.pixel(100, 0)[3], 0);
}

/// Swift `softStrokeBuildsCoverageWhileKeepingItsFeatheredRim`.
#[test]
fn soft_stroke_builds_coverage_while_keeping_its_feathered_rim() {
    let mut single = blank_edit(200, 80, settings(40.0, 0.0, 1.0, 0.0, 0.0));
    single.append(CGPoint::new(100.0, 40.0));
    let one = single.result().pixel(100, 50)[3];
    let mut stroke = blank_edit(200, 80, settings(40.0, 0.0, 1.0, 0.0, 0.0));
    stroke.append(CGPoint::new(20.0, 40.0));
    stroke.append(CGPoint::new(180.0, 40.0));
    stroke.flush();
    let many = stroke.result().pixel(100, 50)[3];
    assert!(many as i32 > one as i32 + 60, "{many} vs {one}");
    assert!(many > 0, "the overlapping stroke still deposits coverage");
}

/// Swift `spacedDabsLeaveNoVisibleRippleAlongTheStroke`.
#[test]
fn spaced_dabs_leave_no_visible_ripple_along_the_stroke() {
    for hardness in [0.0, 0.5, 1.0] {
        let mut edit = blank_edit(900, 300, settings(120.0, hardness, 1.0, 0.0, 0.0));
        edit.append(CGPoint::new(100.0, 150.0));
        edit.append(CGPoint::new(800.0, 150.0));
        edit.flush();
        let image = edit.result();
        // Along the middle of the stroke, and again nearer its edge.
        for offset in [0usize, 30, 50] {
            let run = (300..=600)
                .map(|x| image.pixel(x, 150 + offset)[3])
                .collect::<Vec<_>>();
            let ripple = run.iter().max().unwrap() - run.iter().min().unwrap();
            assert!(
                ripple <= 16,
                "hardness {hardness} offset {offset} rippled by {ripple}"
            );
        }
    }
}

/// Swift `sparseMouseSamplesFollowACurveInsteadOfStraightChords`.
#[test]
fn sparse_mouse_samples_follow_a_curve() {
    let mut edit = blank_edit(300, 300, settings(4.0, 1.0, 1.0, 0.0, 0.0));
    let center = CGPoint::new(150.0, 150.0);
    let radius = 100.0;
    let on_circle = |degrees: f64| {
        CGPoint::new(
            center.x + degrees.to_radians().cos() * radius,
            center.y + degrees.to_radians().sin() * radius,
        )
    };
    edit.append(on_circle(0.0));
    for degrees in (30..=180).step_by(30) {
        edit.append(on_circle(degrees as f64));
    }
    edit.flush();
    let result = render_edit(&edit, 300, 300);
    // A straight chord between 30° and 60° passes 3.4 px inside the arc,
    // farther than this 2 px brush reaches; the curve passes through the arc.
    for degrees in [45.0, 75.0, 105.0, 135.0] {
        let point = on_circle(degrees);
        let alpha = result.pixel(point.x as usize, point.y as usize)[3];
        assert!(alpha > 0, "arc at {degrees}°: {alpha}");
    }
}

/// Swift `liveStrokeReachesNewestSampleAndTailIsReplacedExactly`.
#[test]
fn live_stroke_reaches_newest_sample_and_tail_is_replaced_exactly() {
    let mut edit = blank_edit(300, 120, settings(8.0, 1.0, 1.0, 0.0, 0.0));
    edit.append(CGPoint::new(20.0, 60.0));
    edit.append(CGPoint::new(150.0, 20.0));
    edit.append(CGPoint::new(280.0, 60.0));
    // No lag: the provisional tail already reaches the cursor.
    let live = edit.result();
    assert_eq!(live.pixel(278, 60)[3], 255);
    // The committed stroke is the smooth curve; the straight tail left
    // nothing behind.
    edit.flush();
    let result = render_edit(&edit, 300, 120);
    assert_eq!(result.pixel(215, 40)[3], 0, "the chord midpoint is empty");
    assert_eq!(result.pixel(278, 60)[3], 255);
}

/// Swift `opacityAppliesToMaskPainting`.
#[test]
fn opacity_applies_to_mask_painting() {
    let mut session = EditorSession::new();
    session.create_document(80, 80, true);
    // Opaque red layer over the whole canvas.
    session.brush_settings = settings(200.0, 1.0, 1.0, 0.0, 0.0);
    session.fill_selection(false);
    session.add_layer_mask(true);
    let id = session.active_layer_id.expect("layer");
    session.select_layer_target(id, true);

    // Paint black on the mask at half opacity.
    let mut edit = RasterEdit::new(
        None,
        session.active_layer().unwrap().mask_transform(),
        true,
        BrushSettings {
            opacity: 0.5,
            ..settings(20.0, 1.0, 0.0, 0.0, 0.0)
        },
        CGSize::new(80.0, 80.0),
        None,
    );
    edit.append(CGPoint::new(40.0, 40.0));
    edit.append(CGPoint::new(42.0, 40.0));
    edit.flush();
    let (asset, _, _) = edit.paint_snapshot();
    let layer = session.active_layer().unwrap().clone();
    session
        .document
        .as_mut()
        .unwrap()
        .layer_mut(id)
        .unwrap()
        .mask = Some(layer.mask.unwrap().replacing(asset));
    let result = draw_live_composite(session.document.as_ref().unwrap());
    let center = result.pixel(40, 40)[3];
    assert!((center as i32 - 128).abs() <= 1, "{center}");
    assert_eq!(result.pixel(5, 5)[3], 255);
}

/// Swift `shiftBracketsStepHardnessByQuarters`.
#[test]
fn shift_brackets_step_hardness_by_quarters() {
    let mut session = EditorSession::new();
    session.create_document(600, 80, false);
    session.brush_settings.hardness = 0.8;
    session.change_brush_hardness(true);
    assert_eq!(session.brush_settings.hardness, 1.0);
    session.change_brush_hardness(true);
    assert_eq!(session.brush_settings.hardness, 1.0);
    session.brush_settings.hardness = 0.8;
    session.change_brush_hardness(false);
    assert_eq!(session.brush_settings.hardness, 0.75);
    for _ in 0..5 {
        session.change_brush_hardness(false);
    }
    assert_eq!(session.brush_settings.hardness, 0.0);
    session.change_brush_hardness(true);
    assert_eq!(session.brush_settings.hardness, 0.25);
}

/// Swift `numberKeysSetBrushAndGradientOpacity`.
#[test]
fn number_keys_set_brush_and_gradient_opacity() {
    let mut session = EditorSession::new();
    session.create_document(600, 80, false);
    session.tool = NavigationTool::Brush;
    session.type_opacity_digit(5, 10.0);
    assert_eq!(session.brush_settings.opacity, 0.5);
    session.type_opacity_digit(0, 20.0);
    assert_eq!(session.brush_settings.opacity, 1.0);
    session.type_opacity_digit(4, 30.0);
    session.type_opacity_digit(5, 30.3);
    assert_eq!(session.brush_settings.opacity, 0.45);
    session.type_opacity_digit(0, 40.0);
    session.type_opacity_digit(5, 40.2);
    assert_eq!(session.brush_settings.opacity, 0.05);
    session.tool = NavigationTool::Gradient;
    session.type_opacity_digit(1, 50.0);
    assert_eq!(session.gradient_settings.opacity, 0.1);
    assert_eq!(session.brush_settings.opacity, 0.05);
    session.tool = NavigationTool::Move;
    session.type_opacity_digit(3, 60.0);
    assert_eq!(session.gradient_settings.opacity, 0.1);
}

/// Swift `foldersHiddenLayersAndDisabledMasksRejectPainting` — the folder
/// guard is the one the core models; the hidden-layer and disabled-mask
/// guards live in Swift's `canPaint`, which the Rust `can_edit_pixels` does
/// not carry (module notes).
#[test]
fn folders_reject_painting() {
    let mut session = EditorSession::new();
    session.create_document(600, 80, false);
    session.add_group();
    assert!(
        !session.can_edit_pixels(),
        "a folder has no pixels to paint"
    );
}

fn red(width: usize, height: usize) -> ImportedImage {
    let mut image = Bitmap::new(width, height);
    for chunk in image.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[255, 0, 0, 255]);
    }
    ImportedImage::new(image, "Red")
}
