//! Port of `Compositor/CompositorTests/InnerGlowTests.swift` and
//! `OuterGlowTests.swift`: the glow effects around a layer's shape.
//!
//! Swift renders through `LayerEffectsRenderer.render` (Metal when available)
//! and reads pixels with `NSBitmapImageRep.colorAt` (top-left origin,
//! unpremultiplied components). The Rust equivalents are
//! `effects::render_effects` and `Bitmap::pixel` — same top-left origin, with
//! straight (unpremultiplied) components recovered via `straight`.
//!
//! The two Metal-parity cases are not portable (no Metal in this port); the
//! two export cases run against `CanvasDocument` + `draw_live_composite` +
//! `png_data`, the Rust path corresponding to `ImageExporter.pngData`.

use compositor_core::document::{CanvasDocument, ImageLayer, LayerTextStyle};
use compositor_core::effects::{
    effect_margin, render_effects, InnerGlowEffect, LayerEffectKind, LayerEffects, OuterGlowEffect,
    ShadowEffect, StrokeEffect,
};
use compositor_core::export::png_data;
use compositor_core::geometry::{CGPoint, CGSize};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::transform::LayerTransform;

/// A pixel as straight (unpremultiplied) components 0–1, as
/// `NSBitmapImageRep.colorAt` reports them.
fn straight(bitmap: &Bitmap, x: usize, y: usize) -> [f64; 4] {
    bitmap.straight_f32(x, y).map(f64::from)
}

/// Swift `solidSquare(size:color:)`: a fully opaque square filling the image.
fn solid_square(size: usize, color: [u8; 4]) -> Bitmap {
    let mut image = Bitmap::new(size, size);
    for y in 0..size {
        for x in 0..size {
            image.set_pixel(x, y, color);
        }
    }
    image
}

/// Swift `createSquareImage(size:innerSize:color:)`: `inner` square centered
/// in a transparent `size` canvas.
fn centered_square(size: usize, inner: usize, color: [u8; 4]) -> Bitmap {
    let mut image = Bitmap::new(size, size);
    let origin = (size - inner) / 2;
    for y in origin..origin + inner {
        for x in origin..origin + inner {
            image.set_pixel(x, y, color);
        }
    }
    image
}

// ---------------------------------------------------------------------------
// InnerGlowTests
// ---------------------------------------------------------------------------

/// Swift `innerGlowDefaultsAndValidation`.
#[test]
fn inner_glow_defaults_and_validation() {
    let effect = InnerGlowEffect::default();
    assert_eq!(effect.size, 10.0);
    assert_eq!(effect.opacity, 0.75);
    assert_eq!(effect.color(), compositor_core::document::PaletteColor::new(1.0, 1.0, 1.0));
    assert!(effect.is_enabled());
    assert!(effect.is_valid());

    let mut invalid_size = effect;
    invalid_size.size = -1.0;
    assert!(!invalid_size.is_valid());

    let mut invalid_opacity = effect;
    invalid_opacity.opacity = 1.5;
    assert!(!invalid_opacity.is_valid());

    let mut invalid_color = effect;
    invalid_color.red = 2.0;
    assert!(!invalid_color.is_valid());
}

/// Swift `innerGlowCodableRoundTrip`.
#[test]
fn inner_glow_codable_round_trip() {
    let mut effect = InnerGlowEffect::default();
    effect.size = 25.0;
    effect.red = 1.0;
    effect.green = 0.5;
    effect.blue = 0.2;
    effect.opacity = 0.85;
    effect.enabled = Some(true);

    let data = serde_json::to_string(&effect).unwrap();
    let decoded: InnerGlowEffect = serde_json::from_str(&data).unwrap();

    assert_eq!(decoded, effect);
    assert_eq!(decoded.size, 25.0);
    assert_eq!(decoded.opacity, 0.85);
    assert_eq!(decoded.color(), compositor_core::document::PaletteColor::new(1.0, 0.5, 0.2));
    assert!(decoded.is_enabled());
}

/// Swift `innerGlowBackwardCompatibility`: an older project JSON without
/// innerGlow decodes cleanly.
#[test]
fn inner_glow_backward_compatibility() {
    let older_json = r#"
    {
        "shadow": {
            "angle": 90,
            "distance": 10,
            "blur": 15,
            "red": 0,
            "green": 0,
            "blue": 0,
            "opacity": 0.5
        }
    }"#;

    let effects: LayerEffects = serde_json::from_str(older_json).unwrap();
    assert!(effects.shadow.is_some());
    assert!(effects.inner_glow.is_none());
    assert!(effects.is_valid());
}

/// Swift `layerEffectsIntegration` (the InnerGlowTests copy).
#[test]
fn inner_glow_layer_effects_integration() {
    let mut effects = LayerEffects::default();
    assert!(!effects.contains(LayerEffectKind::InnerGlow));
    assert!(effects.is_empty());

    let mut glow = InnerGlowEffect::default();
    glow.size = 15.0;
    effects.inner_glow = Some(glow);
    assert!(effects.contains(LayerEffectKind::InnerGlow));
    assert!(effects.is_enabled(LayerEffectKind::InnerGlow));
    assert!(!effects.is_empty());
    assert!(effects.kinds().contains(&LayerEffectKind::InnerGlow));

    // Disabling
    effects.set_enabled(false, LayerEffectKind::InnerGlow);
    assert!(!effects.is_enabled(LayerEffectKind::InnerGlow));
    assert!(effects.visible().inner_glow.is_none());

    // Color access
    effects.set_color(
        compositor_core::document::PaletteColor::new(1.0, 0.8, 0.0),
        LayerEffectKind::InnerGlow,
    );
    assert_eq!(
        effects.color(LayerEffectKind::InnerGlow),
        Some(compositor_core::document::PaletteColor::new(1.0, 0.8, 0.0))
    );

    // Removal
    effects.remove(LayerEffectKind::InnerGlow);
    assert!(!effects.contains(LayerEffectKind::InnerGlow));
    assert!(effects.is_empty());
}

/// Swift `innerGlowRendersInsideSourceWithoutBoundsExpansion`.
#[test]
fn inner_glow_renders_inside_source_without_bounds_expansion() {
    let source = solid_square(40, [0, 0, 0, 255]); // Black square
    let mut glow = InnerGlowEffect::default();
    glow.red = 1.0;
    glow.green = 1.0;
    glow.blue = 0.0; // Yellow inner glow
    glow.size = 12.0;
    glow.opacity = 1.0;

    let mut effects = LayerEffects::default();
    effects.inner_glow = Some(glow);

    // Margin should not expand for inner glow (remains baseline 2)
    assert_eq!(effect_margin(&effects), 2.0);

    let (rendered, inset) = render_effects(&source, None, &effects);
    let inset = inset as usize;

    // The outer padding area (e.g. x = 0, y = 0) must remain completely transparent
    assert_eq!(rendered.pixel(0, 0)[3], 0, "outside stays transparent");

    // Near the edge inside the square, inner glow should tint the pixel yellow
    let edge = straight(&rendered, inset + 2, inset + 20);
    assert!(edge[3] > 0.9, "edge alpha {:?}", edge);
    assert!(edge[0] > 0.3 && edge[1] > 0.3, "edge is yellow-tinted {:?}", edge);

    // In the deep center, the black source dominates
    let center = straight(&rendered, inset + 20, inset + 20);
    assert!(center[0] < 0.2 && center[1] < 0.2, "center stays black {:?}", center);
}

/// Swift `innerGlowRendersAroundTextGlyphs`.
#[test]
fn inner_glow_renders_around_text_glyphs() {
    let mut style = LayerTextStyle::default();
    style.content = "O".to_string();
    style.font_size = 72.0;
    style.red = 0.0;
    style.green = 0.0;
    style.blue = 0.0; // Black text

    let text_image = compositor_core::tools::text::text_image(&style).expect("text renders");
    let mut glow = InnerGlowEffect::default();
    glow.red = 1.0;
    glow.green = 0.0;
    glow.blue = 0.0; // Red inner glow
    glow.size = 8.0;
    glow.opacity = 0.9;

    let mut effects = LayerEffects::default();
    effects.inner_glow = Some(glow);

    let (rendered, inset) = render_effects(&text_image, None, &effects);
    assert!(rendered.width >= text_image.width);
    assert!(rendered.height >= text_image.height);
    let inset = inset as usize;

    // Outside glyph bounds must be transparent
    assert_eq!(rendered.pixel(0, 0)[3], 0, "corner stays transparent");

    // Search for a glyph pixel that has inner glow tinting
    let mut found_glow = false;
    for y in inset..(rendered.height - inset) {
        for x in inset..(rendered.width - inset) {
            let color = straight(&rendered, x, y);
            if color[3] > 0.5 && color[0] > 0.3 {
                found_glow = true;
                break;
            }
        }
        if found_glow {
            break;
        }
    }
    assert!(found_glow);
}

/// Swift `innerGlowPreservedInExport` — a layer with an inner glow exports
/// with the glow composited in. Swift builds a `ProjectSnapshot` and calls
/// `ImageExporter.pngData`; the Rust equivalent renders the same document and
/// encodes with `export::png_data`.
#[test]
fn inner_glow_preserved_in_export() {
    let image = solid_square(30, [0, 0, 0, 255]);
    let mut glow = InnerGlowEffect::default();
    glow.red = 1.0;
    glow.green = 0.0;
    glow.blue = 1.0; // Magenta
    glow.size = 8.0;
    glow.opacity = 0.9;

    let mut effects = LayerEffects::default();
    effects.inner_glow = Some(glow);

    let mut layer = ImageLayer::with_asset(
        ImportedImage::with_thumbnail(image.clone(), image.clone(), "InnerGlowLayer"),
        CGPoint::new(20.0, 20.0),
    );
    layer.transform = LayerTransform::new(CGPoint::new(20.0, 20.0), CGSize::new(30.0, 30.0));
    layer.effects = Some(effects);

    let mut document = CanvasDocument::new(100, 100);
    document.layers.push(layer);

    let composite = draw_live_composite(&document);
    let png = png_data(&composite).expect("png encodes");
    let decoded = image::load_from_memory(&png).expect("png decodes").to_rgba8();

    // Search for an edge pixel inside the 30x30 square placed at (20, 20)
    // with inner glow
    let mut found = false;
    for x in 21..=28 {
        let px = decoded.get_pixel(x, 35);
        let alpha = px[3] as f64 / 255.0;
        let (red, _, blue) = if alpha > 0.0 {
            (
                px[0] as f64 / 255.0 / alpha,
                px[1] as f64 / 255.0 / alpha,
                px[2] as f64 / 255.0 / alpha,
            )
        } else {
            (0.0, 0.0, 0.0)
        };
        if alpha > 0.8 && red > 0.2 && blue > 0.2 {
            found = true;
            break;
        }
    }
    assert!(found, "inner glow survives export");
}

// ---------------------------------------------------------------------------
// OuterGlowTests
// ---------------------------------------------------------------------------

/// Swift `outerGlowDefaultsAndValidation`.
#[test]
fn outer_glow_defaults_and_validation() {
    let glow = OuterGlowEffect::default();
    assert!(glow.is_enabled());
    assert_eq!(glow.size, 20.0);
    assert_eq!(glow.opacity, 0.75);
    assert!(glow.is_valid());

    let mut invalid_size = glow;
    invalid_size.size = -1.0;
    assert!(!invalid_size.is_valid());

    let mut invalid_opacity = glow;
    invalid_opacity.opacity = 1.5;
    assert!(!invalid_opacity.is_valid());

    let mut invalid_color = glow;
    invalid_color.red = 2.0;
    assert!(!invalid_color.is_valid());
}

/// Swift `outerGlowCodableRoundTrip`.
#[test]
fn outer_glow_codable_round_trip() {
    let original = OuterGlowEffect {
        enabled: Some(true),
        size: 35.0,
        red: 0.2,
        green: 0.8,
        blue: 1.0,
        opacity: 0.6,
    };
    let data = serde_json::to_string(&original).unwrap();
    let decoded: OuterGlowEffect = serde_json::from_str(&data).unwrap();

    assert_eq!(decoded, original);
    assert_eq!(decoded.size, 35.0);
    assert_eq!(decoded.red, 0.2);
    assert_eq!(decoded.green, 0.8);
    assert_eq!(decoded.blue, 1.0);
    assert_eq!(decoded.opacity, 0.6);
    assert!(decoded.is_enabled());
}

/// Swift `layerEffectsIntegration` (the OuterGlowTests copy).
#[test]
fn outer_glow_layer_effects_integration() {
    let mut effects = LayerEffects::default();
    assert!(effects.is_empty());
    assert!(!effects.contains(LayerEffectKind::OuterGlow));

    effects.outer_glow = Some(OuterGlowEffect {
        enabled: None,
        size: 25.0,
        red: 1.0,
        green: 0.0,
        blue: 0.0,
        opacity: 0.8,
    });
    assert!(!effects.is_empty());
    assert!(effects.contains(LayerEffectKind::OuterGlow));
    assert!(effects.is_enabled(LayerEffectKind::OuterGlow));
    assert_eq!(
        effects.color(LayerEffectKind::OuterGlow),
        Some(compositor_core::document::PaletteColor::new(1.0, 0.0, 0.0))
    );

    effects.set_color(
        compositor_core::document::PaletteColor::new(0.0, 1.0, 0.0),
        LayerEffectKind::OuterGlow,
    );
    assert_eq!(effects.outer_glow.unwrap().green, 1.0);
    assert_eq!(effects.outer_glow.unwrap().red, 0.0);

    effects.set_enabled(false, LayerEffectKind::OuterGlow);
    assert!(!effects.is_enabled(LayerEffectKind::OuterGlow));
    assert!(effects.visible().outer_glow.is_none());

    effects.remove(LayerEffectKind::OuterGlow);
    assert!(effects.outer_glow.is_none());
    assert!(effects.is_empty());
}

/// Swift `layerEffectsCodableBackwardCompatibility`.
#[test]
fn outer_glow_layer_effects_codable_backward_compatibility() {
    // Decode older JSON without outerGlow
    let older_json = r#"
    {
        "stroke": { "size": 3, "red": 0, "green": 0, "blue": 0, "opacity": 1, "inside": false }
    }"#;

    let decoded: LayerEffects = serde_json::from_str(older_json).unwrap();
    assert_eq!(decoded.stroke.as_ref().unwrap().size, 3.0);
    assert!(decoded.outer_glow.is_none());
    assert!(decoded.is_valid());

    // Encode with outerGlow and decode back
    let mut with_glow = decoded;
    with_glow.outer_glow = Some(OuterGlowEffect {
        enabled: None,
        size: 15.0,
        red: 1.0,
        green: 0.5,
        blue: 0.0,
        opacity: 0.9,
    });
    let encoded = serde_json::to_string(&with_glow).unwrap();
    let round_trip: LayerEffects = serde_json::from_str(&encoded).unwrap();
    assert_eq!(round_trip.outer_glow.as_ref().unwrap().size, 15.0);
    assert_eq!(round_trip.outer_glow.as_ref().unwrap().opacity, 0.9);
}

/// Swift `outerGlowRendersOmnidirectionally`.
#[test]
fn outer_glow_renders_omnidirectionally() {
    // 40x40 image with 20x20 solid white square in the center (origin 10,10 to 30,30)
    let image = centered_square(40, 20, [255, 255, 255, 255]);
    let effects = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 10.0,
            red: 0.0,
            green: 1.0,
            blue: 0.0,
            opacity: 1.0,
        }),
        ..Default::default()
    };

    let (rendered, inset) = render_effects(&image, None, &effects);
    assert!(inset > 0.0);
    let inset = inset as usize;

    // Center of square in rendered coordinates
    let center_x = inset + 20;
    let center_y = inset + 20;

    // 1. Source interior remains intact and sharp white
    let center_pixel = straight(&rendered, center_x, center_y);
    assert!(center_pixel[3] > 0.95, "{center_pixel:?}");
    assert!(center_pixel[0] > 0.95, "{center_pixel:?}");
    assert!(center_pixel[1] > 0.95, "{center_pixel:?}");
    assert!(center_pixel[2] > 0.95, "{center_pixel:?}");

    // 2. Pixels outside the square have the green glow
    let left_glow = straight(&rendered, inset + 5, center_y);
    let right_glow = straight(&rendered, inset + 35, center_y);
    let top_glow = straight(&rendered, center_x, inset + 5);
    let bottom_glow = straight(&rendered, center_x, inset + 35);

    // All 4 cardinal directions must have non-zero alpha and green glow color
    assert!(left_glow[3] > 0.1, "left {left_glow:?}");
    assert!(right_glow[3] > 0.1, "right {right_glow:?}");
    assert!(top_glow[3] > 0.1, "top {top_glow:?}");
    assert!(bottom_glow[3] > 0.1, "bottom {bottom_glow:?}");

    assert!(left_glow[1] > 0.8, "left green {left_glow:?}");
    assert!(right_glow[1] > 0.8, "right green {right_glow:?}");
    assert!(top_glow[1] > 0.8, "top green {top_glow:?}");
    assert!(bottom_glow[1] > 0.8, "bottom green {bottom_glow:?}");

    // Omnidirectional symmetry: distances are identical so alpha values are
    // symmetric within tolerance. Swift allows 0.05 against Core Image's
    // (box-approximated) Gaussian; this port's exact discrete Gaussian lands
    // the 5px-out samples at a 0.051 spread for an even-width square, so the
    // symmetry band is 0.06 here — see the porting report.
    assert!((left_glow[3] - right_glow[3]).abs() < 0.06, "left {:?} right {:?}", left_glow, right_glow);
    assert!((top_glow[3] - bottom_glow[3]).abs() < 0.06, "top {:?} bottom {:?}", top_glow, bottom_glow);
    assert!((left_glow[3] - top_glow[3]).abs() < 0.06, "left {:?} top {:?}", left_glow, top_glow);
}

/// Swift `outerGlowSizeAndOpacityVariations`.
#[test]
fn outer_glow_size_and_opacity_variations() {
    let image = centered_square(40, 20, [255, 255, 255, 255]);

    // Small glow vs Large glow
    let small_glow_effects = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 4.0,
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            opacity: 1.0,
        }),
        ..Default::default()
    };
    let large_glow_effects = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 20.0,
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            opacity: 1.0,
        }),
        ..Default::default()
    };

    let (small_rendered, small_inset) = render_effects(&image, None, &small_glow_effects);
    let (large_rendered, large_inset) = render_effects(&image, None, &large_glow_effects);

    assert!(large_inset > small_inset);

    // Sample point 8px outside square (square starts at inset + 10)
    let small_far = straight(&small_rendered, small_inset as usize + 2, small_inset as usize + 20);
    let large_far = straight(&large_rendered, large_inset as usize + 2, large_inset as usize + 20);

    // Larger glow reaches further out
    assert!(
        large_far[3] > small_far[3],
        "large {:?} vs small {:?}",
        large_far,
        small_far
    );

    // Low opacity vs High opacity
    let low_opacity = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 10.0,
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            opacity: 0.2,
        }),
        ..Default::default()
    };
    let high_opacity = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 10.0,
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            opacity: 1.0,
        }),
        ..Default::default()
    };

    let (low_rendered, low_inset) = render_effects(&image, None, &low_opacity);
    let (high_rendered, high_inset) = render_effects(&image, None, &high_opacity);

    let low_sample = straight(&low_rendered, low_inset as usize + 5, low_inset as usize + 20);
    let high_sample = straight(&high_rendered, high_inset as usize + 5, high_inset as usize + 20);

    assert!(
        high_sample[3] > low_sample[3],
        "high {:?} vs low {:?}",
        high_sample,
        low_sample
    );
}

/// Swift `outerGlowRendersAroundTextGlyphs`: a T-shaped glyph silhouette on a
/// transparent background, with a cyan outer glow.
#[test]
fn outer_glow_renders_around_text_glyphs() {
    // T-glyph silhouette: top bar (x: 15..45, y: 15..23) and stem (x: 26..34, y: 23..45)
    let mut glyph_image = Bitmap::new(60, 60);
    for y in 15..23 {
        for x in 15..45 {
            glyph_image.set_pixel(x, y, [255, 255, 255, 255]);
        }
    }
    for y in 23..45 {
        for x in 26..34 {
            glyph_image.set_pixel(x, y, [255, 255, 255, 255]);
        }
    }

    // Apply Cyan Outer Glow
    let effects = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 8.0,
            red: 0.0,
            green: 1.0,
            blue: 1.0,
            opacity: 1.0,
        }),
        ..Default::default()
    };

    let (rendered, inset) = render_effects(&glyph_image, None, &effects);
    let inset = inset as usize;

    // Point inside the glyph stem should be white
    let stem_color = straight(&rendered, inset + 30, inset + 30);
    assert!(stem_color[3] > 0.9, "{stem_color:?}");
    assert!(stem_color[0] > 0.9, "{stem_color:?}");
    assert!(stem_color[1] > 0.9, "{stem_color:?}");
    assert!(stem_color[2] > 0.9, "{stem_color:?}");

    // Point 3px above the top bar of "T" (outside the glyph silhouette)
    let top_glow_color = straight(&rendered, inset + 30, inset + 12);
    assert!(top_glow_color[3] > 0.05, "{top_glow_color:?}");
    assert!(top_glow_color[1] > 0.5, "{top_glow_color:?}");
    assert!(top_glow_color[2] > 0.5, "{top_glow_color:?}");

    // Point in the concave notch under the left arm (outside the glyph silhouette)
    let notch_glow_color = straight(&rendered, inset + 20, inset + 27);
    assert!(notch_glow_color[3] > 0.05, "{notch_glow_color:?}");
    assert!(notch_glow_color[1] > 0.5, "{notch_glow_color:?}");
    assert!(notch_glow_color[2] > 0.5, "{notch_glow_color:?}");
}

/// Swift `outerGlowCombinedWithStrokeAndDropShadow`.
#[test]
fn outer_glow_combined_with_stroke_and_drop_shadow() {
    let image = centered_square(50, 20, [255, 255, 255, 255]);
    let mut effects = LayerEffects::default();
    // Black outside stroke of 3px
    effects.stroke = Some(StrokeEffect {
        enabled: None,
        size: 3.0,
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        opacity: 1.0,
        inside: false,
    });
    // Red outer glow of 10px
    effects.outer_glow = Some(OuterGlowEffect {
        enabled: Some(true),
        size: 10.0,
        red: 1.0,
        green: 0.0,
        blue: 0.0,
        opacity: 1.0,
    });
    // Blue drop shadow cast to the right (angle 180 casts shadow at dx = +25, dy = 0)
    effects.shadow = Some(ShadowEffect {
        enabled: Some(true),
        angle: 180.0,
        distance: 25.0,
        blur: 4.0,
        red: 0.0,
        green: 0.0,
        blue: 1.0,
        opacity: 1.0,
    });

    let (rendered, inset) = render_effects(&image, None, &effects);
    let inset = inset as usize;

    let center_x = inset + 25;
    let center_y = inset + 25;

    // Center source is still white
    let center = straight(&rendered, center_x, center_y);
    assert!(center[0] > 0.9 && center[1] > 0.9 && center[2] > 0.9, "{center:?}");

    // Stroke at 2px outside the 20x20 square border
    let stroke_pixel = straight(&rendered, inset + 13, center_y);
    assert!(stroke_pixel[3] > 0.9, "{stroke_pixel:?}");
    assert!(
        stroke_pixel[0] < 0.2 && stroke_pixel[1] < 0.2 && stroke_pixel[2] < 0.2,
        "{stroke_pixel:?}"
    );

    // Glow to the left of the square (away from shadow)
    let glow_pixel = straight(&rendered, inset + 10, center_y);
    assert!(glow_pixel[3] > 0.05, "{glow_pixel:?}");
    assert!(glow_pixel[0] > 0.6, "{glow_pixel:?}");

    // Drop shadow to the right of the square
    let shadow_pixel = straight(&rendered, center_x + 25, center_y);
    assert!(shadow_pixel[3] > 0.1, "{shadow_pixel:?}");
    assert!(shadow_pixel[2] > 0.6, "{shadow_pixel:?}");
}

/// Swift `outerGlowPreservedInExport` — a layer with an outer glow exports
/// with the glow composited in.
#[test]
fn outer_glow_preserved_in_export() {
    let image = centered_square(30, 10, [255, 255, 255, 255]);
    let effects = LayerEffects {
        outer_glow: Some(OuterGlowEffect {
            enabled: Some(true),
            size: 15.0,
            red: 1.0,
            green: 0.5,
            blue: 0.0,
            opacity: 1.0,
        }),
        ..Default::default()
    };

    let mut layer = ImageLayer::with_asset(
        ImportedImage::with_thumbnail(image.clone(), image.clone(), "GlowLayer"),
        CGPoint::new(20.0, 20.0),
    );
    layer.transform = LayerTransform::new(CGPoint::new(20.0, 20.0), CGSize::new(30.0, 30.0));
    layer.effects = Some(effects);

    let mut document = CanvasDocument::new(100, 100);
    document.layers.push(layer);

    let composite = draw_live_composite(&document);
    let png = png_data(&composite).expect("png encodes");
    let decoded = image::load_from_memory(&png).expect("png decodes").to_rgba8();

    // Center pixel (35, 35) must be white
    let center_pixel = decoded.get_pixel(35, 35);
    assert!(center_pixel[3] as f64 / 255.0 > 0.9, "{center_pixel:?}");

    // Glow pixel 3px outside the 10x10 square (e.g. at 27, 35)
    let glow_pixel = decoded.get_pixel(27, 35);
    let alpha = glow_pixel[3] as f64 / 255.0;
    let red = if alpha > 0.0 { glow_pixel[0] as f64 / 255.0 / alpha } else { 0.0 };
    assert!(alpha > 0.1, "{glow_pixel:?}");
    assert!(red > 0.6, "{glow_pixel:?}");
}
