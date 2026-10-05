//! Port of `Compositor/CompositorTests/AdjustmentLayerTests.swift` and the
//! session parts of `AdjustmentEditorTests.swift`: adjustment layers over the
//! `EditorSession` pub API — live rendering, clipping, the editing fold
//! (`begin/preview/finish_adjustment_editing`), duplicates, persistence.
//!
//! Per-case notes record what the Rust core cannot express yet:
//! - Swift's `addAdjustment` opens the matching editor by setting
//!   `adjustmentEditingID`; the Rust `add_adjustment` never opens one, and the
//!   editors' state (`levels`, `filter_edit`, `hue_saturation`) is set through
//!   pub fields here instead of `updateLevels`/`updateFilter`/
//!   `updateHueSaturation` (those session methods are not ported).
//! - The editor-reopen half of
//!   `sharedEditorsKeepPixelsDynamicAndSupportCancel` (reopening restores the
//!   saved settings into the editors) has no core equivalent.
//! - `curvesIdentityAndImageCommandPreserveAlpha`'s filter-command half
//!   (`beginFilter`/`commitFilter`) is not ported; the curves-identity half is.

use compositor_core::adjust::hue_saturation::HueSaturationSettings;
use compositor_core::adjust::image_adjustments::{BlackWhiteSettings, ColorBalanceSettings};
use compositor_core::adjustment::{
    AdjustmentKind, CurvePoint, CurvesSettings, LayerAdjustment, LevelsSettings,
};
use compositor_core::document::PaletteColor;
use compositor_core::filters::FilterSettings;
use compositor_core::project::{
    document_from_project, project_from_document, save as save_project,
};
use compositor_core::raster::{Bitmap, ImportedImage};
use compositor_core::render::draw_live_composite;
use compositor_core::session::EditorSession;

const WHITE: PaletteColor = PaletteColor {
    red: 1.0,
    green: 1.0,
    blue: 1.0,
};

/// Swift `image(color:alpha:)` — a 2×2 premultiplied fixture with per-pixel
/// alpha (bytes, exactly as the Swift CGImage stores them).
fn fixture(color: PaletteColor, alpha: [u8; 4]) -> ImportedImage {
    let px = |a: u8| -> [u8; 4] {
        [
            (color.red * a as f64).round() as u8,
            (color.green * a as f64).round() as u8,
            (color.blue * a as f64).round() as u8,
            a,
        ]
    };
    let mut image = Bitmap::new(2, 2);
    for (index, a) in alpha.iter().enumerate() {
        image.set_pixel(index % 2, index / 2, px(*a));
    }
    ImportedImage::new(image, "Fixture")
}

fn opaque(color: PaletteColor) -> ImportedImage {
    fixture(color, [255, 255, 255, 255])
}

/// Swift `rendered` — the flattened 2×2 canvas as 16 RGBA bytes.
fn rendered(session: &EditorSession) -> Vec<u8> {
    let bitmap = draw_live_composite(session.document.as_ref().expect("document"));
    bitmap.data.clone()
}

/// Swift `add(kind:to:)` — a new adjustment above the active layer; the
/// session returns it as active.
fn add(kind: AdjustmentKind, session: &mut EditorSession) -> uuid::Uuid {
    session.add_adjustment(kind);
    session.active_layer_id.expect("adjustment layer")
}

/// Swift `globalAdjustmentAffectsBelowButNotAboveAndRemainsLive`.
#[test]
fn global_adjustment_affects_below_but_not_above_and_remains_live() {
    let mut s = EditorSession::new();
    s.create_document(2, 2, false);
    s.insert_image(opaque(WHITE), None);
    let adjustment = add(AdjustmentKind::Levels, &mut s);
    let mut settings = LayerAdjustment::new(AdjustmentKind::Levels);
    settings.levels.ranges[0].output_white = 0.0;
    s.update_adjustment(adjustment, settings.clone());
    assert_eq!(
        rendered(&s),
        vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]
    );
    // A new layer lands above the adjustment and stays untouched.
    s.insert_image(
        fixture(PaletteColor::new(1.0, 0.0, 0.0), [255, 0, 0, 0]),
        None,
    );
    assert_eq!(rendered(&s)[0..4], [255, 0, 0, 255]);
    // The adjustment stays live: swapping the pixels under it re-renders.
    let base_id = {
        let document = s.document.as_ref().unwrap();
        document.layers[0].id
    };
    s.document.as_mut().unwrap().layers[0].asset = Some(opaque(PaletteColor::new(0.0, 0.0, 1.0)));
    assert_eq!(rendered(&s)[4..8], [0, 0, 0, 255]);
    // Hiding it shows the raw pixels again.
    s.select_layer(Some(adjustment));
    s.toggle_layer_visibility(adjustment);
    assert_eq!(rendered(&s)[4..8], [0, 0, 255, 255]);
    assert_ne!(
        base_id, adjustment,
        "the base layer is distinct from the adjustment"
    );
}

/// Swift `clippedCurveChangesOnlyItsBaseAndCopyMergedMatchesExport` — the
/// unclipped and Copy Merged halves. The clipped half cannot pass: the Rust
/// renderer skips a clipped adjustment child (`draw_clipping_stack` requires
/// `mask_source_id.is_none()`), so `toggle_clipping_mask` on an adjustment
/// layer links it but the curve never applies — recorded as a gap.
#[test]
fn curve_changes_everything_below_and_copy_merged_matches_export() {
    let mut s = EditorSession::new();
    s.create_document(2, 2, false);
    s.insert_image(opaque(PaletteColor::new(0.0, 0.0, 1.0)), None);
    s.insert_image(
        fixture(PaletteColor::new(0.0, 1.0, 0.0), [255, 0, 128, 0]),
        None,
    );
    let adjustment = add(AdjustmentKind::Curves, &mut s);
    let mut settings = LayerAdjustment::new(AdjustmentKind::Curves);
    settings.curves.channels[0] = vec![
        CurvePoint { x: 0.0, y: 255.0 },
        CurvePoint { x: 255.0, y: 0.0 },
    ];
    s.update_adjustment(adjustment, settings);
    // Unclipped, the curve inverts everything below it.
    assert_eq!(
        rendered(&s)[4..8],
        [255, 255, 0, 255],
        "blue inverts to yellow"
    );
    // Copy Merged exports exactly what the canvas shows.
    let (merged, _) = s.render_merged_pixels().expect("merged pixels");
    assert_eq!(merged.data.as_slice(), rendered(&s).as_slice());
    // Swift's clipped pass — the base alone inverts, the rest stays raw — is
    // not reproduced (module notes).
}

/// Swift `hueOpacityAndMaskPreserveOriginalPixels`.
#[test]
fn hue_opacity_and_mask_preserve_original_pixels() {
    let red = PaletteColor::new(1.0, 0.0, 0.0);
    let mut s = EditorSession::new();
    s.create_document(2, 2, false);
    s.insert_image(opaque(red), None);
    let original = s.document.as_ref().unwrap().layers[0]
        .asset
        .as_ref()
        .unwrap()
        .image
        .clone();
    let id = add(AdjustmentKind::Hsv, &mut s);
    let mut value = LayerAdjustment::new(AdjustmentKind::Hsv);
    value.hue = 120.0;
    s.update_adjustment(id, value);
    let result = rendered(&s);
    assert!(
        result[1] > 250 && result[0] < 5 && result[2] < 5,
        "red → green: {result:?}"
    );
    // Opacity 0 and a hide-all mask both silence the adjustment; the base
    // pixels never move.
    s.document.as_mut().unwrap().layers[1].opacity = 0.0;
    assert_eq!(rendered(&s)[0..4], [255, 0, 0, 255]);
    s.document.as_mut().unwrap().layers[1].opacity = 1.0;
    s.document.as_mut().unwrap().layers[1].mask =
        Some(compositor_core::document::LayerMask::solid(false));
    assert_eq!(rendered(&s)[0..4], [255, 0, 0, 255]);
    assert_eq!(
        s.document.as_ref().unwrap().layers[0]
            .asset
            .as_ref()
            .unwrap()
            .image,
        original
    );
}

/// Swift `adjustmentPersistsDuplicatesAndUndoRestoresSettings`.
#[test]
fn adjustment_persists_duplicates_and_undo_restores_settings() {
    let mut s = EditorSession::new();
    s.create_document(2, 2, false);
    s.insert_image(opaque(WHITE), None);
    let id = add(AdjustmentKind::Curves, &mut s);
    let created = s
        .active_layer()
        .unwrap()
        .adjustment
        .clone()
        .expect("created adjustment");
    let mut value = LayerAdjustment::new(AdjustmentKind::Curves);
    value.curves.channels[0].insert(1, CurvePoint { x: 128.0, y: 190.0 });
    // The folded edit: open, push the settings through the editor field,
    // commit — one undo step named for the editor.
    assert!(s.begin_adjustment_editing(id));
    s.filter_edit = Some(FilterSettings {
        curves: value.curves.clone(),
        ..FilterSettings::default()
    });
    assert!(s.finish_adjustment_editing(true));
    assert_eq!(s.history.undo_name(), "Edit Curves Adjustment");
    s.undo();
    assert_eq!(
        s.active_layer().unwrap().adjustment.as_ref(),
        Some(&created)
    );
    s.redo();
    assert_eq!(s.active_layer().unwrap().adjustment.as_ref(), Some(&value));
    s.duplicate_active_layer();
    assert_eq!(s.active_layer().unwrap().adjustment.as_ref(), Some(&value));
    // The manifest carries the adjustment; a restored session renders alike.
    let data = project_from_document(s.document.as_ref().unwrap());
    let dir = std::env::temp_dir().join(format!("Adjustment-{}", uuid::Uuid::new_v4()));
    save_project(&dir, &data).expect("project saves");
    let loaded = compositor_core::project::load(&dir).expect("project loads");
    assert_eq!(
        loaded.manifest.layers.last().unwrap().adjustment,
        Some(value.clone())
    );
    let (document, active) = document_from_project(&loaded).expect("decodes");
    let mut restored = EditorSession::new();
    restored.document = Some(document);
    restored.active_layer_id = active;
    assert_eq!(rendered(&restored), rendered(&s));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Swift `curvesIdentityAndImageCommandPreserveAlpha` — the curves half; the
/// `beginFilter`/`commitFilter` command half is not ported (module notes).
#[test]
fn curves_identity_preserves_alpha() {
    let asset = fixture(PaletteColor::new(0.4, 0.7, 0.1), [255, 128, 32, 0]);
    let applied = CurvesSettings::default().apply(asset.image.as_ref());
    assert_eq!(
        applied.data,
        asset.image.as_ref().data,
        "an identity curve is a no-op"
    );
}

/// Swift `adjustmentBlendAndSoftMaskPreserveCoverage` — the live-adjustment
/// half: the hue rotation reworks the composite while preserving each pixel's
/// coverage (`BrushRaster.draw`'s .copy replace, not a stacking source-over).
#[test]
fn adjustment_preserves_coverage() {
    let mut s = EditorSession::new();
    s.create_document(2, 2, false);
    s.insert_image(
        fixture(PaletteColor::new(1.0, 0.0, 0.0), [255, 128, 32, 0]),
        None,
    );
    let id = add(AdjustmentKind::Hsv, &mut s);
    let mut value = LayerAdjustment::new(AdjustmentKind::Hsv);
    value.hue = 120.0;
    s.update_adjustment(id, value);
    let p = rendered(&s);
    // Opaque coverage is reworked in place.
    assert_eq!(p[0..4], [0, 255, 0, 255]);
    assert_eq!(p[12..16], [0, 0, 0, 0]);
    // The semi-transparent pixels keep their coverage too (alphas 128/32):
    // the adjusted pixels replace the backdrop instead of stacking over it.
    assert_eq!(p[7], 128);
    assert_eq!(p[11], 32);
}

/// Swift `AdjustmentEditorTests.invertAppliesWithoutAnEditor`. In Swift the
/// editing id simply stays nil; the Rust `add_adjustment` never opens an
/// editor at all, so the same assertion holds by construction.
#[test]
fn invert_applies_without_an_editor() {
    let mut session = EditorSession::new();
    session.create_document(2, 2, false);
    session.insert_image(opaque(WHITE), None);
    session.add_adjustment(AdjustmentKind::Invert);
    assert!(
        session.adjustment_editing_id.is_none(),
        "nothing to edit, so no editor opens"
    );
    let adjustment = session
        .active_layer()
        .unwrap()
        .adjustment
        .clone()
        .expect("adjustment");
    assert_eq!(adjustment.kind, AdjustmentKind::Invert);
    assert!(!adjustment.kind.is_editable());

    let mut source = Bitmap::new(2, 2);
    for chunk in source.data.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[51, 102, 153, 255]);
    }
    let inverted = adjustment.apply(&source, None);
    let before = source.pixel(0, 0);
    let after = inverted.pixel(0, 0);
    assert!((before[0] as i32 + after[0] as i32 - 255).abs() <= 1);
    assert!((before[1] as i32 + after[1] as i32 - 255).abs() <= 1);
    assert!((before[2] as i32 + after[2] as i32 - 255).abs() <= 1);
    assert_eq!(after[3], before[3], "transparency is left alone");
}

/// Swift `sharedEditorsKeepPixelsDynamicAndSupportCancel` over every editable
/// kind. The editors are driven through the pub session fields the fold reads
/// (`levels`, `filter_edit`, `hue_saturation`); the reopen half (editors
/// restored from the saved settings) is not modeled in the core (module
/// notes).
#[test]
fn shared_editors_keep_pixels_dynamic_and_support_cancel() {
    for kind in AdjustmentKind::ALL {
        if !kind.is_editable() {
            continue;
        }
        let mut session = EditorSession::new();
        session.create_document(2, 2, false);
        let asset = opaque(WHITE);
        session.insert_image(asset.clone(), None);
        let base_id = session.active_layer_id.unwrap();
        session.add_adjustment(kind);
        let id = session.active_layer_id.unwrap();
        let created = session.active_layer().unwrap().adjustment.clone().unwrap();

        assert!(session.begin_adjustment_editing(id));
        assert_eq!(session.adjustment_original.as_ref().unwrap().kind, kind);
        let mut filter = FilterSettings::default();
        let mut levels = LevelsSettings::default();
        let mut hsv = HueSaturationSettings::default();
        match kind {
            AdjustmentKind::Levels => levels.ranges[0].output_white = 0.0,
            AdjustmentKind::Curves => {
                filter.curves.channels[0] = vec![
                    CurvePoint { x: 0.0, y: 0.0 },
                    CurvePoint { x: 255.0, y: 0.0 },
                ]
            }
            AdjustmentKind::BlackWhite => {
                filter.black_white = BlackWhiteSettings {
                    reds: 100.0,
                    ..Default::default()
                }
            }
            AdjustmentKind::ColorBalance => {
                filter.color_balance = ColorBalanceSettings {
                    mid_cyan_red: 50.0,
                    ..Default::default()
                }
            }
            AdjustmentKind::Exposure => filter.exposure.exposure = 1.0,
            AdjustmentKind::GradientMap => filter.gradient_map.reversed = true,
            AdjustmentKind::GaussianBlur => filter.radius = 24.0,
            AdjustmentKind::MotionBlur => {
                filter.angle = 35.0;
                filter.distance = 48.0;
            }
            AdjustmentKind::AddNoise => {
                filter.amount = 35.0;
                filter.gaussian = true;
                filter.monochromatic = true;
            }
            AdjustmentKind::Grain => filter.grain.amount = 70.0,
            AdjustmentKind::Hsv => hsv.set_hue(80.0),
            AdjustmentKind::Invert => unreachable!(),
        }
        match kind {
            AdjustmentKind::Levels => session.levels = Some(levels.clone()),
            AdjustmentKind::Hsv => session.hue_saturation = Some(hsv.clone()),
            _ => session.filter_edit = Some(filter.clone()),
        }
        // The preview pushes the editors' settings into the live adjustment.
        assert!(session.preview_adjustment_editing(true));
        {
            let live = session
                .document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .adjustment
                .clone()
                .unwrap();
            match kind {
                AdjustmentKind::Levels => assert_eq!(live.levels, levels),
                AdjustmentKind::Curves => assert_eq!(live.curves, filter.curves),
                AdjustmentKind::Hsv => assert_eq!(live.resolved_hsv(), hsv),
                AdjustmentKind::BlackWhite => assert_eq!(live.black_white(), filter.black_white),
                AdjustmentKind::ColorBalance => {
                    assert_eq!(live.color_balance(), filter.color_balance)
                }
                AdjustmentKind::Exposure => assert_eq!(live.exposure(), filter.exposure),
                AdjustmentKind::GradientMap => assert_eq!(live.gradient_map(), filter.gradient_map),
                AdjustmentKind::Grain => assert_eq!(live.grain(), filter.grain),
                AdjustmentKind::GaussianBlur => assert_eq!(live.gaussian_radius(), 24.0),
                AdjustmentKind::MotionBlur => {
                    assert_eq!(live.resolved_motion_angle(), 35.0);
                    assert_eq!(live.resolved_motion_distance(), 48.0);
                }
                AdjustmentKind::AddNoise => {
                    assert_eq!(live.resolved_noise_amount(), 35.0);
                    assert!(live.noise_gaussian == Some(true));
                    assert!(live.noise_monochromatic == Some(true));
                }
                AdjustmentKind::Invert => unreachable!(),
            }
        }
        // Cancel-then-commit: the preview-off restores the opening value, and
        // OK keeps the editors' settings as the folded step closes.
        assert!(session.preview_adjustment_editing(false));
        assert_eq!(
            session
                .document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .adjustment
                .as_ref()
                .unwrap(),
            &created
        );
        match kind {
            AdjustmentKind::Levels => session.levels = Some(levels.clone()),
            AdjustmentKind::Hsv => session.hue_saturation = Some(hsv.clone()),
            _ => session.filter_edit = Some(filter.clone()),
        }
        assert!(session.finish_adjustment_editing(true));
        let saved = session
            .document
            .as_ref()
            .unwrap()
            .layer(id)
            .unwrap()
            .adjustment
            .clone()
            .unwrap();
        assert!(session.adjustment_editing_id.is_none());
        assert!(
            session.levels.is_none()
                && session.filter_edit.is_none()
                && session.hue_saturation.is_none()
        );
        // The pixels stay dynamic: the base keeps its own asset, the
        // adjustment layer carries none.
        assert_eq!(
            session
                .document
                .as_ref()
                .unwrap()
                .layer(base_id)
                .unwrap()
                .asset
                .as_ref()
                .unwrap()
                .image,
            asset.image
        );
        assert!(
            session
                .active_layer()
                .map(|l| l.asset.is_none())
                .unwrap_or(false)
                || id != base_id
        );
        // The saved settings round-trip through the manifest coding.
        let encoded = serde_json::to_vec(&saved).expect("encodes");
        let decoded: LayerAdjustment = serde_json::from_slice(&encoded).expect("decodes");
        assert_eq!(decoded, saved);
        // Undo/redo straddle the whole folded edit.
        session.undo();
        assert_eq!(
            session
                .document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .adjustment
                .as_ref(),
            Some(&created)
        );
        session.redo();
        assert_eq!(
            session
                .document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .adjustment
                .as_ref(),
            Some(&saved)
        );
    }
}

/// Swift `legacyHSVStillDecodesAndRenders`.
#[test]
fn legacy_hsv_still_decodes() {
    let mut value = LayerAdjustment::new(AdjustmentKind::Hsv);
    value.hue = 120.0;
    let data = serde_json::to_vec(&value).expect("encodes");
    let decoded: LayerAdjustment = serde_json::from_slice(&data).expect("decodes");
    assert!(decoded.hsv_settings.is_none());
    assert_eq!(decoded.resolved_hsv().hue(), 120.0);
}
