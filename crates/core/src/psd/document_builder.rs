//! Port of `Compositor/IO/PSD/PSDDocumentBuilder.swift`: maps the parsed PSD
//! document onto the app model — the layer tree, groups, masks, the blend-mode
//! subset, fill rectangles/ellipses as shape layers, and the conversion report.

use std::collections::HashMap;

use uuid::Uuid;

use crate::document::{ImageLayer, LayerBlendMode, LayerMask};
use crate::geometry::{CGPoint, CGSize};
use crate::raster::{Bitmap, GrayMask, ImportedImage, thumbnail_of};
use crate::transform::{LayerSampling, LayerTransform};

use super::types::{PsdConversion, PsdDocument, PsdLayerKind, blend_mode_from_psd};

/// `PSDImport`: everything the app needs to apply the import.
#[derive(Clone)]
pub struct PsdImport {
    pub width: usize,
    pub height: usize,
    pub resolution: f64,
    pub layers: Vec<ImageLayer>,
    pub conversions: Vec<PsdConversion>,
}

pub struct PsdDocumentBuilder;

impl PsdDocumentBuilder {
    /// `PSDDocumentBuilder.assets(from:)`: the layer images keyed by record id.
    pub fn assets(document: &PsdDocument) -> HashMap<Uuid, ImportedImage> {
        let mut result: HashMap<Uuid, ImportedImage> = HashMap::new();
        for record in &document.layers {
            if let Some(image) = &record.image {
                result.insert(record.id, imported(image, &record.name));
            }
        }
        result
    }

    /// `PSDDocumentBuilder.makeImport(_ document:)` with no prepared assets.
    pub fn make_import(document: &PsdDocument) -> PsdImport {
        Self::make_import_with_assets(document, &HashMap::new())
    }

    /// `PSDDocumentBuilder.makeImport(_ document:assets:)`.
    pub fn make_import_with_assets(
        document: &PsdDocument,
        assets: &HashMap<Uuid, ImportedImage>,
    ) -> PsdImport {
        let mut conversions: Vec<PsdConversion> = Vec::new();
        let mut layers: Vec<ImageLayer> = Vec::new();
        let canvas = CGSize::new(document.width as f64, document.height as f64);
        for record in &document.layers {
            let mut notes: Vec<String> = Vec::new();
            match record.kind {
                PsdLayerKind::Text => {
                    notes.push(
                        "Editable Photoshop text becomes pixels and can\u{2019}t be retyped."
                            .to_string(),
                    );
                }
                PsdLayerKind::SmartObject => {
                    notes.push(
                        "The smart object was rasterized. Linked contents can\u{2019}t be edited."
                            .to_string(),
                    );
                }
                PsdLayerKind::Effects => {
                    notes.push(
                        "Layer effects were discarded, so the appearance may differ.".to_string(),
                    );
                }
                PsdLayerKind::Vector => {
                    if record.shape.is_some() {
                        notes.extend(record.shape_notes.iter().cloned());
                    } else {
                        notes.push("Vector shape was rasterized to pixels.".to_string());
                    }
                }
                PsdLayerKind::Other => {
                    notes.push("This Photoshop layer type isn\u{2019}t supported and was imported as pixels.".to_string());
                }
                PsdLayerKind::Raster | PsdLayerKind::Group | PsdLayerKind::Adjustment => {}
            }
            if record.is_group {
                if record.blend_key != "pass" && record.blend_key != "norm" {
                    notes.push(format!(
                        "Folder blend mode \u{201C}{}\u{201D} isn\u{2019}t supported. The folder will be pass-through.",
                        record.blend_key
                    ));
                }
            } else if blend_mode_from_psd(&record.blend_key).is_none() && record.blend_key != "pass"
            {
                notes.push(format!(
                    "Blend mode \u{201C}{}\u{201D} isn\u{2019}t supported and will be applied as Normal.",
                    record.blend_key.trim()
                ));
            }
            if record.kind == PsdLayerKind::Adjustment {
                if record.adjustment.is_none() {
                    notes.push(
                        "This adjustment type isn\u{2019}t supported and was skipped.".to_string(),
                    );
                } else {
                    notes
                        .push("Adjustment parameters may not match Photoshop exactly.".to_string());
                }
            }
            for note in &notes {
                conversions.push(PsdConversion::new(record.name.clone(), note.clone()));
            }
            if record.kind == PsdLayerKind::Adjustment && record.adjustment.is_none() {
                continue;
            }
            let opacity = record.opacity.clamp(0.0, 1.0);
            let blend_mode =
                blend_mode_from_psd(&record.blend_key).unwrap_or(LayerBlendMode::Normal);
            let mut layer = if record.is_group {
                // Folders carry an opacity of their own (1.1.6), which multiplies into what's inside
                // them just as Photoshop's group opacity does.
                ImageLayer {
                    id: record.id,
                    asset: None,
                    transform: LayerTransform::new(CGPoint::ZERO, canvas),
                    name: record.name.clone(),
                    is_visible: record.is_visible,
                    parent_id: record.parent_id,
                    is_group: true,
                    opacity,
                    blend_mode: LayerBlendMode::Normal,
                    mask_source_id: None,
                    mask: None,
                    adjustment: None,
                    shape: None,
                    effects: None,
                    text: None,
                }
            } else if let Some(adjustment) = record.adjustment.clone() {
                ImageLayer {
                    id: record.id,
                    asset: None,
                    transform: LayerTransform::new(CGPoint::ZERO, canvas),
                    name: record.name.clone(),
                    is_visible: record.is_visible,
                    parent_id: record.parent_id,
                    is_group: false,
                    opacity,
                    blend_mode,
                    mask_source_id: None,
                    mask: None,
                    adjustment: Some(adjustment),
                    shape: None,
                    effects: None,
                    text: None,
                }
            } else if let Some(image) = &record.image {
                let asset = assets
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| imported(image, &record.name));
                let origin = CGPoint::new(record.bounds.min_x(), record.bounds.min_y());
                let size = if record.bounds.size.width > 0.0 && record.bounds.size.height > 0.0 {
                    record.bounds.size
                } else {
                    CGSize::new(image.width as f64, image.height as f64)
                };
                ImageLayer {
                    id: record.id,
                    asset: Some(asset),
                    transform: LayerTransform::new(origin, size),
                    name: record.name.clone(),
                    is_visible: record.is_visible,
                    parent_id: record.parent_id,
                    is_group: false,
                    opacity,
                    blend_mode,
                    mask_source_id: None,
                    mask: None,
                    adjustment: None,
                    shape: record.shape.map(|style| style.to_layer_shape()),
                    effects: None,
                    text: None,
                }
            } else {
                ImageLayer {
                    id: record.id,
                    asset: None,
                    transform: LayerTransform::new(CGPoint::ZERO, canvas),
                    name: record.name.clone(),
                    is_visible: record.is_visible,
                    parent_id: record.parent_id,
                    is_group: false,
                    opacity,
                    blend_mode,
                    mask_source_id: None,
                    mask: None,
                    adjustment: None,
                    shape: None,
                    effects: None,
                    text: None,
                }
            };
            if let Some(mask_image) = &record.mask {
                // The reader always hands over 8-bit gray planes, so the
                // Swift `try? LayerMask.asset(from:)` failure path cannot fire.
                layer.mask = Some(LayerMask {
                    asset: layer_mask_asset(mask_image),
                    is_enabled: record.mask_enabled,
                    placement: None,
                    is_linked: record.mask_linked,
                });
            }
            layers.push(layer);
        }
        // Clipping: a clipped layer borrows the mask of the nearest unclipped
        // base below it in the same parent.
        let id_to_index: HashMap<Uuid, usize> = layers
            .iter()
            .enumerate()
            .map(|(index, layer)| (layer.id, index))
            .collect();
        let mut base_for_parent: HashMap<Option<Uuid>, Option<Uuid>> = HashMap::new();
        for record in &document.layers {
            let Some(&index) = id_to_index.get(&record.id) else {
                continue;
            };
            if record.clipping {
                let source = base_for_parent
                    .get(&record.parent_id)
                    .and_then(|base| *base);
                let supported = source
                    .and_then(|id| layers.iter().find(|layer| layer.id == id))
                    .map(|layer| !layer.is_group && layer.adjustment.is_none())
                    .unwrap_or(false);
                if let (Some(source), true) = (source, supported) {
                    layers[index].mask_source_id = Some(source);
                } else {
                    conversions.push(PsdConversion::new(
                        record.name.clone(),
                        "This clipping mask\u{2019}s base isn\u{2019}t supported, so clipping was skipped.",
                    ));
                }
            } else {
                let layer = &layers[index];
                if !layer.is_group && layer.adjustment.is_none() {
                    base_for_parent.insert(record.parent_id, Some(record.id));
                } else {
                    base_for_parent.insert(record.parent_id, None);
                }
            }
        }
        PsdImport {
            width: document.width,
            height: document.height,
            resolution: document.resolution,
            layers,
            conversions,
        }
    }
}

/// `PSDDocumentBuilder.imported(_:name:)` — the pixels plus an auto thumbnail.
fn imported(image: &Bitmap, name: &str) -> ImportedImage {
    ImportedImage::with_thumbnail(image.clone(), thumbnail_of(image), name)
}

/// `LayerMask.asset(from:)`: gray pixels become the RGBA asset, with a
/// thumbnail capped at 96 px on the long side.
fn layer_mask_asset(mask: &GrayMask) -> ImportedImage {
    let mut data = Vec::with_capacity(mask.data.len() * 4);
    for &value in &mask.data {
        data.extend_from_slice(&[value, value, value, 255]);
    }
    let image = Bitmap::from_raw(mask.width, mask.height, data);
    let factor = (96.0 / mask.width.max(mask.height) as f64).min(1.0);
    let width = ((mask.width as f64 * factor) as usize).max(1);
    let height = ((mask.height as f64 * factor) as usize).max(1);
    let thumbnail = crate::raster::resample(&image, width, height, LayerSampling::High);
    ImportedImage::with_thumbnail(image, thumbnail, "Layer Mask")
}
