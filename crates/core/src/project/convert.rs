//! Conversions between the package format and the live document model:
//! `EditorSession.projectSnapshot()` / `installProject(_:)`
//! (`Document/EditorSession+Projects.swift`), `ProjectSnapshot.mask(for:)`
//! (`Document/LayerMask.swift`) and the `LayerText.loaded` / `LayerShape.loaded`
//! helpers.

use std::collections::HashMap;

use uuid::Uuid;

use super::ProjectData;
use super::manifest::{
    ManifestEffects, ManifestGuide, ManifestShape, ManifestTextStyle, ManifestTransform,
    ProjectLayerRecord, ProjectManifest, image_file_name, mask_file_name,
};
use super::store::validate;
use crate::blend::LayerBlendMode;
use crate::document::{
    CanvasDocument, ImageLayer, LayerMask, LayerShape, LayerText, LayerTextStyle,
};
use crate::effects::LayerEffects;
use crate::raster::{Bitmap, GrayMask, ImportedImage};
use crate::transform::LayerTransform;

use super::ProjectError;

/// `EditorSession.installProject`: the validated package as a live document,
/// plus the restored active layer. Re-validates, so a hand-built `ProjectData`
/// cannot smuggle in damaged metadata.
pub fn document_from_project(
    data: &ProjectData,
) -> Result<(CanvasDocument, Option<Uuid>), ProjectError> {
    validate(&data.manifest)?;
    let manifest = &data.manifest;
    let guides = manifest
        .guides
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(crate::document::CanvasGuide::from)
        .collect();
    let layers = manifest
        .layers
        .iter()
        .map(|record| layer_from_record(data, record))
        .collect();
    let document = CanvasDocument {
        id: manifest.document_id,
        width: manifest.width as usize,
        height: manifest.height as usize,
        resolution: manifest.resolution.unwrap_or(72.0),
        layers,
        guides,
        selection: None,
    };
    Ok((document, manifest.active_layer_id))
}

/// `ProjectSnapshot.mask(for:)`: a layer's raster mask, missing when the record
/// has no mask file or the package has no such mask asset. Missing flags fall
/// back to enabled and linked (older projects).
fn mask_for(data: &ProjectData, record: &ProjectLayerRecord) -> Option<LayerMask> {
    record.mask_file.as_ref()?;
    let gray = data.masks.get(&record.id)?;
    // The document model keeps mask pixels in an RGBA bitmap; a grayscale mask
    // becomes neutral RGB at full alpha.
    let mut bitmap = Bitmap::new(gray.width, gray.height);
    for (index, &value) in gray.data.iter().enumerate() {
        let offset = index * 4;
        bitmap.data[offset] = value;
        bitmap.data[offset + 1] = value;
        bitmap.data[offset + 2] = value;
        bitmap.data[offset + 3] = 255;
    }
    Some(LayerMask {
        asset: ImportedImage::new(bitmap, record.name.clone()),
        is_enabled: record.mask_enabled.unwrap_or(true),
        placement: record.mask_placement.map(LayerTransform::from),
        is_linked: record.mask_linked.unwrap_or(true),
    })
}

fn layer_from_record(data: &ProjectData, record: &ProjectLayerRecord) -> ImageLayer {
    let asset = data
        .images
        .get(&record.id)
        .map(|bitmap| ImportedImage::new(bitmap.clone(), record.name.clone()));
    // `LayerText.loaded` / `LayerShape.loaded`: both need the layer's image,
    // and text must be valid to display.
    let text = record
        .text
        .as_ref()
        .filter(|style| LayerTextStyle::from((*style).clone()).is_valid());
    let text = text.zip(asset.as_ref()).map(|(style, asset)| LayerText {
        style: LayerTextStyle::from(style.clone()),
        image: asset.image.clone(),
    });
    let shape = record
        .shape
        .as_ref()
        .zip(asset.as_ref())
        .map(|(style, _)| LayerShape::from(style));
    ImageLayer {
        id: record.id,
        asset,
        transform: LayerTransform::from(record.transform),
        name: record.name.clone(),
        is_visible: record.is_visible,
        parent_id: record.parent_id,
        is_group: record.is_group == Some(true),
        opacity: record.opacity.unwrap_or(1.0),
        blend_mode: record.blend_mode.unwrap_or(LayerBlendMode::Normal),
        mask_source_id: record.mask_source_id,
        mask: mask_for(data, record),
        adjustment: record.adjustment.clone(),
        shape,
        effects: record.effects.as_ref().map(LayerEffects::from),
        text,
    }
}

/// `EditorSession.projectSnapshot()`: the live document as a package. The
/// active layer is session state, so it is left unset; callers copy it from the
/// session when there is one.
pub fn project_from_document(doc: &CanvasDocument) -> ProjectData {
    let mut images = HashMap::new();
    let mut masks = HashMap::new();
    let layers = doc
        .layers
        .iter()
        .map(|layer| {
            let mask_record = layer.mask.as_ref();
            if let Some(asset) = &layer.asset {
                images.insert(layer.id, asset.image.as_ref().clone());
            }
            if let Some(mask) = mask_record {
                masks.insert(layer.id, gray_of(&mask.asset.image));
            }
            ProjectLayerRecord {
                id: layer.id,
                name: layer.name.clone(),
                is_visible: layer.is_visible,
                transform: ManifestTransform::from(&layer.transform),
                image_file: layer.asset.as_ref().map(|_| image_file_name(layer.id)),
                parent_id: layer.parent_id,
                is_group: Some(layer.is_group),
                opacity: Some(layer.opacity),
                blend_mode: Some(layer.blend_mode),
                mask_file: mask_record.map(|_| mask_file_name(layer.id)),
                mask_enabled: mask_record.map(|mask| mask.is_enabled),
                mask_source_id: layer.mask_source_id,
                adjustment: layer.adjustment.clone(),
                mask_placement: mask_record
                    .and_then(|mask| mask.placement)
                    .map(|p| ManifestTransform::from(&p)),
                mask_linked: mask_record.map(|mask| mask.is_linked),
                shape: layer.shape.as_ref().map(ManifestShape::from),
                effects: layer.effects.as_ref().map(ManifestEffects::from),
                // Only text that is still live is saved; a rasterized text
                // layer keeps its pixels but drops the metadata, exactly as
                // `layer.liveText?.style` does.
                text: layer
                    .live_text()
                    .map(|text| ManifestTextStyle::from(&text.style)),
            }
        })
        .collect();
    ProjectData {
        manifest: ProjectManifest {
            format: super::manifest::FORMAT.to_string(),
            version: super::manifest::CURRENT_VERSION,
            color_space: "sRGB".to_string(),
            resolution: Some(doc.resolution),
            document_id: doc.id,
            width: doc.width as i64,
            height: doc.height as i64,
            active_layer_id: None,
            layers,
            // An empty guide list is omitted, as in the Swift snapshot.
            guides: if doc.guides.is_empty() {
                None
            } else {
                Some(doc.guides.iter().map(ManifestGuide::from).collect())
            },
        },
        images,
        masks,
    }
}

/// A document-model mask bitmap (gray replicated into RGBA at full alpha) back
/// to its grayscale coverage.
fn gray_of(image: &Bitmap) -> GrayMask {
    GrayMask::from_raw(
        image.width,
        image.height,
        image.data.chunks_exact(4).map(|pixel| pixel[0]).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::store::{load, save};
    use super::*;
    use crate::document::{CanvasGuide, GuideAxis, ShapeKind};
    use crate::effects::{
        ColorOverlayEffect, InnerShadowEffect, LayerEffects, ShadowEffect, StrokeEffect,
    };
    use crate::geometry::{CGPoint, CGSize};
    use crate::raster::ImportedImage;
    use crate::transform::LayerSampling;

    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "CompositorProjectConvert-{tag}-{}-{}",
                std::process::id(),
                Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> Bitmap {
        let mut bitmap = Bitmap::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                bitmap.set_pixel(x, y, [200, 100, 50, 255]);
            }
        }
        bitmap
    }

    fn styled_document() -> (CanvasDocument, Uuid, Uuid) {
        let mut doc = CanvasDocument::new(60, 60);
        doc.resolution = 300.0;
        let mut image =
            ImageLayer::with_asset(ImportedImage::new(fixture(), "Photo"), CGPoint::ZERO);
        image.name = "Photo".into();
        image.opacity = 0.8;
        image.blend_mode = crate::blend::LayerBlendMode::Multiply;
        image.effects = Some(LayerEffects {
            stroke: Some(StrokeEffect {
                size: 8.0,
                red: 0.1,
                green: 0.8,
                blue: 0.2,
                opacity: 0.9,
                inside: false,
                ..StrokeEffect::default()
            }),
            shadow: Some(ShadowEffect {
                angle: 45.0,
                distance: 15.0,
                blur: 10.0,
                red: 0.2,
                green: 0.2,
                blue: 0.3,
                opacity: 0.75,
                ..ShadowEffect::default()
            }),
            color_overlay: Some(ColorOverlayEffect {
                red: 0.9,
                green: 0.1,
                blue: 0.4,
                opacity: 0.65,
                ..ColorOverlayEffect::default()
            }),
            inner_shadow: Some(InnerShadowEffect {
                angle: 135.0,
                distance: 6.0,
                blur: 4.0,
                red: 0.05,
                green: 0.05,
                blue: 0.05,
                opacity: 0.5,
                ..InnerShadowEffect::default()
            }),
            ..LayerEffects::default()
        });
        let mut gray = GrayMask::new(2, 1);
        gray.data[0] = 255;
        gray.data[1] = 0;
        image.mask = Some(LayerMask {
            asset: ImportedImage::new(mask_bitmap(&gray), "Layer Mask"),
            is_enabled: false,
            placement: Some(LayerTransform::new(
                CGPoint::new(5.0, 6.0),
                CGSize::new(7.0, 8.0),
            )),
            is_linked: false,
        });
        let image_id = image.id;

        let mut folder = ImageLayer::blank("Folder 1", doc.size());
        folder.is_group = true;
        folder.opacity = 0.5;
        let folder_id = folder.id;
        let mut text = ImageLayer::blank("Text", doc.size());
        text.parent_id = Some(folder.id);
        text.asset = Some(ImportedImage::new(fixture(), "Text"));
        text.transform = LayerTransform {
            sampling: LayerSampling::Smooth,
            ..LayerTransform::new(CGPoint::new(10.0, 10.0), CGSize::new(40.0, 20.0))
        };
        text.text = Some(LayerText {
            style: LayerTextStyle {
                content: "Hello world".into(),
                font_name: "Helvetica".into(),
                font_size: 24.0,
                red: 1.0,
                green: 0.5,
                blue: 0.0,
                alignment: crate::document::TextAlignment::Center,
                tracking: 2.0,
                leading: 30.0,
                box_size: Some(CGSize::new(200.0, 100.0)),
            },
            image: text.asset.as_ref().unwrap().image.clone(),
        });
        let mut shape = ImageLayer::blank("Shape", doc.size());
        shape.asset = Some(ImportedImage::new(fixture(), "Shape"));
        shape.shape = Some(LayerShape {
            kind: ShapeKind::Ellipse,
            corner_radius: 0.0,
            line_width: 4.0,
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            filled: true,
            start: crate::geometry::CGPoint::ZERO,
            end: crate::geometry::CGPoint::new(1.0, 1.0),
        });
        doc.layers = vec![folder, text, shape, image];
        doc.guides = vec![CanvasGuide::new(GuideAxis::Vertical, 30.0)];
        (doc, image_id, folder_id)
    }

    fn mask_bitmap(gray: &GrayMask) -> Bitmap {
        let mut bitmap = Bitmap::new(gray.width, gray.height);
        for (index, &value) in gray.data.iter().enumerate() {
            bitmap.set_pixel(
                index % gray.width,
                index / gray.width,
                [value, value, value, 255],
            );
        }
        bitmap
    }

    /// The Swift test `projectRoundTripPreservesAllLayerEffects`, plus masks,
    /// text, shape, groups and guides: everything survives save, load and
    /// installation.
    #[test]
    fn round_trip_preserves_all_layer_styling() {
        let root = TempRoot::new("styling");
        let (doc, image_id, folder_id) = styled_document();
        let mut data = project_from_document(&doc);
        data.manifest.active_layer_id = Some(image_id);
        let url = root.0.join("Styling.comp");
        save(&url, &data).unwrap();
        let loaded = load(&url).unwrap();
        let (reopened, active) = document_from_project(&loaded).unwrap();
        assert_eq!(active, Some(image_id));
        assert_eq!(reopened.resolution, 300.0);
        assert_eq!(reopened.guides, doc.guides);
        assert_eq!(
            reopened.layers.iter().map(|l| l.id).collect::<Vec<_>>(),
            doc.layers.iter().map(|l| l.id).collect::<Vec<_>>()
        );
        let restored = reopened.layer(image_id).unwrap();
        // Effects.
        let effects = restored.effects.as_ref().unwrap();
        let stroke = effects.stroke.as_ref().unwrap();
        assert_eq!(stroke.size, 8.0);
        assert!(!stroke.inside);
        assert_eq!(stroke.opacity, 0.9);
        assert!((stroke.red - 0.1).abs() < 0.001 && (stroke.green - 0.8).abs() < 0.001);
        let shadow = effects.shadow.as_ref().unwrap();
        assert_eq!(shadow.angle, 45.0);
        assert_eq!(shadow.distance, 15.0);
        assert_eq!(shadow.blur, 10.0);
        assert_eq!(shadow.opacity, 0.75);
        let overlay = effects.color_overlay.as_ref().unwrap();
        assert_eq!(overlay.opacity, 0.65);
        assert!((overlay.red - 0.9).abs() < 0.001 && (overlay.blue - 0.4).abs() < 0.001);
        let inner = effects.inner_shadow.as_ref().unwrap();
        assert_eq!(inner.angle, 135.0);
        assert_eq!(inner.distance, 6.0);
        assert_eq!(inner.blur, 4.0);
        assert_eq!(inner.opacity, 0.5);
        // Mask: coverage, disabled flag, placement and link state.
        let mask = restored.mask.as_ref().unwrap();
        assert!(!mask.is_enabled);
        assert!(!mask.is_linked);
        assert_eq!(
            mask.placement,
            Some(LayerTransform::new(
                CGPoint::new(5.0, 6.0),
                CGSize::new(7.0, 8.0)
            ))
        );
        assert_eq!(mask.asset.image.width, 2);
        assert_eq!(mask.asset.image.pixel(0, 0)[0], 255);
        assert_eq!(mask.asset.image.pixel(1, 0)[0], 0);
        // Appearance.
        assert_eq!(restored.opacity, 0.8);
        assert_eq!(restored.blend_mode, crate::blend::LayerBlendMode::Multiply);
        // Text.
        let text_layer = reopened.layers.iter().find(|l| l.text.is_some()).unwrap();
        let text = text_layer.text.as_ref().unwrap();
        assert_eq!(text.style.content, "Hello world");
        assert_eq!(text.style.font_size, 24.0);
        assert_eq!(text.style.alignment, crate::document::TextAlignment::Center);
        assert_eq!(text.style.tracking, 2.0);
        assert_eq!(text.style.leading, 30.0);
        assert_eq!(text.style.box_size, Some(CGSize::new(200.0, 100.0)));
        // Shape.
        let shape_layer = reopened.layers.iter().find(|l| l.shape.is_some()).unwrap();
        let shape = shape_layer.shape.as_ref().unwrap();
        assert_eq!(shape.kind, ShapeKind::Ellipse);
        assert_eq!(shape.red, 1.0);
        // Folder opacity survives (a dimmed folder saves and reopens).
        let folder = reopened.layer(folder_id).unwrap();
        assert!(folder.is_group);
        assert_eq!(folder.opacity, 0.5);
        // The text layer is still live text over its own pixels.
        assert!(text_layer.live_text().is_some());
    }

    #[test]
    fn rasterized_text_drops_metadata_on_save() {
        let mut doc = CanvasDocument::new(40, 40);
        let mut layer = ImageLayer::blank("Text", doc.size());
        let image = std::sync::Arc::new(fixture());
        layer.asset = Some(ImportedImage {
            image: image.clone(),
            thumbnail: std::sync::Arc::new(Bitmap::new(1, 1)),
            name: "Text".into(),
        });
        layer.text = Some(LayerText {
            style: LayerTextStyle::default(),
            // A stale raster: the layer was redrawn as plain pixels.
            image: std::sync::Arc::new(Bitmap::new(2, 2)),
        });
        doc.layers = vec![layer];
        let data = project_from_document(&doc);
        assert!(
            data.manifest.layers[0].text.is_none(),
            "stale text is dropped"
        );
    }

    #[test]
    fn mask_links_survive_without_placement() {
        let root = TempRoot::new("mask-links");
        let mut doc = CanvasDocument::new(40, 40);
        let mut base = ImageLayer::with_asset(ImportedImage::new(fixture(), "Base"), CGPoint::ZERO);
        let mut clipped = ImageLayer::blank("Clipped", doc.size());
        clipped.mask_source_id = Some(base.id);
        let base_id = base.id;
        let clipped_id = clipped.id;
        doc.layers = vec![base, clipped];
        let mut data = project_from_document(&doc);
        data.manifest.active_layer_id = Some(clipped_id);
        let url = root.0.join("Clipping.comp");
        save(&url, &data).unwrap();
        let loaded = load(&url).unwrap();
        let record = loaded
            .manifest
            .layers
            .iter()
            .find(|l| l.id == clipped_id)
            .unwrap();
        assert_eq!(record.mask_source_id, Some(base_id));
        let (reopened, _) = document_from_project(&loaded).unwrap();
        assert_eq!(
            reopened.layer(clipped_id).unwrap().mask_source_id,
            Some(base_id)
        );
        let _ = base_id;
    }
}
