//! Port of `Compositor/CompositorTests/TypeToolTests.swift`: the Type tool's
//! typesetting and rasterizing behavior.
//!
//! The Swift cases drive an `EditorSession` (`beginText`, `applyText`,
//! `editActiveText`, undo/redo, tool switching). The Rust port's session lives
//! in `session.rs` (being reworked in parallel), so those case bodies are not
//! ported here; the cases that only need the pure typesetting API
//! (`tools::text`) and the document/render model are. See the per-test
//! comments for exactly which Swift case each covers.

use compositor_core::document::{CanvasDocument, ImageLayer, LayerTextStyle};
use compositor_core::geometry::CGPoint;
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::tools::text::{text_box_size, text_image};

/// Swift `rasterHasTransparentBackgroundAndColoredGlyphs`.
#[test]
fn raster_has_transparent_background_and_colored_glyphs() {
    let mut style = LayerTextStyle::default();
    style.content = "TYPE".to_string();
    style.red = 1.0;
    let image = text_image(&style).expect("text renders");

    let mut ink = 0u32;
    let mut clear = 0u32;
    for px in image.data.chunks_exact(4) {
        if px[3] == 0 {
            clear += 1;
        } else {
            ink += 1;
            assert!(
                px[0] > 0 && px[1] == 0 && px[2] == 0,
                "ink is pure red: {px:?}"
            );
        }
    }
    assert!(ink > 100, "glyph pixels exist: {ink}");
    assert!(clear > 100, "background stays transparent: {clear}");
}

/// Swift `clippingToTextExportsColoredGlyphsOnTransparency`.
///
/// The Swift case builds the text layer and its clipped color fill through an
/// `EditorSession`; here the same document is built directly (a text raster
/// layer with a red fill layer above it clipped to it) and exported through
/// the compositor, which is what `ImageExporter.render` does.
#[test]
fn clipping_to_text_exports_colored_glyphs_on_transparency() {
    let style = LayerTextStyle {
        content: "Text".to_string(),
        ..Default::default()
    };
    let text = text_image(&style).expect("text renders");
    let size = text_box_size(&style);
    assert_eq!(
        size.width.ceil() as usize,
        text.width,
        "raster uses the box size"
    );
    assert_eq!(
        size.height.ceil() as usize,
        text.height,
        "raster uses the box size"
    );

    // The fill: a red rectangle covering the text layer's whole box, clipped
    // to the text layer (`addPixelLayer` + `linkMask(source:target:)`).
    let mut fill = Bitmap::new(text.width, text.height);
    for px in fill.data.chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 0, 0, 255]);
    }
    let text_layer = ImageLayer::with_asset(
        ImportedImage::with_thumbnail(text.clone(), text.clone(), "Text"),
        CGPoint::new(0.0, 0.0),
    );
    let mut fill_layer = ImageLayer::with_asset(
        ImportedImage::with_thumbnail(fill.clone(), fill, "Clipped color"),
        CGPoint::new(0.0, 0.0),
    );
    fill_layer.mask_source_id = Some(text_layer.id);

    let mut document = CanvasDocument::new(800, 600);
    document.layers.push(text_layer);
    document.layers.push(fill_layer);

    let rendered = draw_live_composite(&document);
    let mut ink = 0u32;
    let mut clear = 0u32;
    for px in rendered.data.chunks_exact(4) {
        if px[3] == 0 {
            clear += 1;
        } else if px[3] == 255 {
            ink += 1;
            assert_eq!(px[0], 255, "opaque ink is saturated red: {px:?}");
            assert_eq!(px[1], 0, "opaque ink has no green: {px:?}");
        }
    }
    assert!(ink > 100, "glyph pixels exist: {ink}");
    assert!(
        clear > 100,
        "canvas stays transparent off the glyphs: {clear}"
    );
}

/// Swift `invalidAndStaleDraftsDoNotChangeDocument`, reduced to its pure
/// typesetting half. The draft/apply lifecycle (`applyText` refusing NaN
/// sizes, an empty box, or a stale draft against a new document) is session
/// logic; the validation those calls consult (`LayerTextStyle.isValid`) is
/// checked here.
#[test]
fn invalid_draft_styles_are_rejected() {
    let mut draft = LayerTextStyle {
        content: "Text".to_string(),
        ..Default::default()
    };
    draft.font_size = f64::NAN;
    assert!(!draft.is_valid(), "NaN font size is invalid");

    draft.font_size = 72.0;
    draft.box_size = Some(compositor_core::geometry::CGSize::new(0.0, 100.0));
    assert!(!draft.is_valid(), "an empty box is invalid");

    draft.box_size = Some(compositor_core::geometry::CGSize::new(360.0, 160.0));
    draft.content = "Valid".to_string();
    assert!(draft.is_valid(), "a normal draft validates");
}
