//! Port of the session's mask extensions: `LayerMask.swift` and
//! `LiveLayerMask.swift`.

use super::*;

impl EditorSession {
    // MARK: Masks

    /// `canEditMask`: canEditLayers && one selected layer.
    pub fn can_edit_mask(&self) -> bool {
        self.can_edit_layers()
            && self.selected_layer_ids.len() == 1
            && self.active_layer().is_some()
    }

    pub fn add_layer_mask(&mut self, revealing: bool) {
        if !self.can_edit_mask()
            || self
                .active_layer()
                .map(|l| l.mask.is_some())
                .unwrap_or(true)
        {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        self.finish_opacity_edit();
        self.begin_edit(if revealing {
            "Add Reveal-All Mask"
        } else {
            "Add Hide-All Mask"
        });
        if let Some(doc) = self.document.as_mut() {
            doc.layer_mut(id).unwrap().mask = Some(LayerMask::solid(revealing));
        }
        self.is_mask_selected = true;
        self.end_edit();
    }

    pub fn toggle_layer_mask(&mut self) {
        if !self.can_edit_mask() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        let enabled = self
            .active_layer()
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.is_enabled);
        if let Some(enabled) = enabled {
            self.finish_opacity_edit();
            self.begin_edit(if enabled {
                "Disable Layer Mask"
            } else {
                "Enable Layer Mask"
            });
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().mask.as_mut().unwrap().is_enabled = !enabled;
            }
            self.end_edit();
        }
    }

    pub fn delete_layer_mask(&mut self) {
        if !self.can_edit_mask() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        if self
            .active_layer()
            .map(|l| l.mask.is_some())
            .unwrap_or(false)
        {
            self.finish_opacity_edit();
            self.begin_edit("Delete Layer Mask");
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().mask = None;
            }
            self.is_mask_selected = false;
            self.end_edit();
        }
    }

    /// `selectLayerTarget` (LayerMask.swift:222-228) — a click on a layer's or
    /// folder's pixel/mask thumbnail. The effect selection always leaves first,
    /// then the pending gradient resolves, the layer selects, and the mask
    /// becomes the target only when the layer has one. The busy states the
    /// Swift guard also checks (`isProjectBusy`, `isImporting`,
    /// `brushStroke != nil`) are app-side and not modeled here.
    pub fn select_layer_target(&mut self, id: Uuid, mask: bool) {
        self.effect_selection = None;
        self.resolve_gradient();
        self.select_layer(Some(id));
        self.is_mask_selected = mask
            && self
                .active_layer()
                .map(|l| l.mask.is_some())
                .unwrap_or(false);
    }

    /// `canCopyMask` (LayerMask.swift:290-298) — whether an Option-drag can
    /// drop a copy of `source`'s mask on `target`: two different layers, the
    /// source wearing a mask, and the target not a folder.
    pub fn can_copy_mask(&self, source: Uuid, target: Uuid) -> bool {
        if !self.can_edit_layers() || source == target {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        if document
            .layer(source)
            .and_then(|l| l.mask.as_ref())
            .is_none()
        {
            return false;
        }
        match document.layer(target) {
            Some(layer) => !layer.is_group,
            None => false,
        }
    }

    /// `copyMask` (LayerMask.swift:300-311) — Option-dragging a mask thumbnail
    /// onto another layer: a copy of the mask, sitting where it sits on the
    /// document, replacing any mask the layer had. The target selects with the
    /// mask as its target, in one undo step named for the copy.
    pub fn copy_mask(&mut self, source: Uuid, target: Uuid) {
        if !self.can_copy_mask(source, target) {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(from) = document.layer(source).cloned() else {
            return;
        };
        let Some(mut mask) = from.mask.clone() else {
            return;
        };
        if document.layer_index(target).is_none() {
            return;
        }
        self.commit_transform();
        self.finish_opacity_edit();
        mask.placement = Some(from.mask_transform());
        let had_mask = self
            .document
            .as_ref()
            .and_then(|d| d.layer(target))
            .map(|l| l.mask.is_some())
            .unwrap_or(false);
        self.begin_edit(if had_mask {
            "Replace Layer Mask"
        } else {
            "Copy Layer Mask"
        });
        if let Some(doc) = self.document.as_mut()
            && let Some(layer) = doc.layer_mut(target)
        {
            layer.mask = Some(mask);
        }
        self.select_layer(Some(target));
        self.is_mask_selected = true;
        self.end_edit();
    }

    /// `toggleMaskLink` (LayerMask.swift:312-322) — the link between a layer
    /// and its mask: linked they move together; unlinked each transforms on
    /// its own, as in Photoshop.
    pub fn toggle_mask_link(&mut self, id: Uuid) {
        if !self.can_edit_layers() {
            return;
        }
        let linked = self
            .document
            .as_ref()
            .and_then(|d| d.layer(id))
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.is_linked);
        let Some(linked) = linked else {
            return;
        };
        self.commit_transform();
        self.finish_opacity_edit();
        self.begin_edit(if linked {
            "Unlink Layer Mask"
        } else {
            "Link Layer Mask"
        });
        if let Some(doc) = self.document.as_mut()
            && let Some(layer) = doc.layer_mut(id)
            && let Some(mask) = layer.mask.as_mut()
        {
            mask.is_linked = !linked;
        }
        self.end_edit();
    }

    /// `displayedMaskPlacement` (LayerMask.swift:324-338) — where `layer`'s
    /// mask shows right now: None while it covers the layer's (displayed)
    /// pixel grid — a pending mask transform's draft, or where a pending layer
    /// transform leaves it (carried along when linked). The Content-Aware
    /// Fill branch is filter-edit state the core does not model yet.
    pub fn displayed_mask_placement(&self, layer: &ImageLayer) -> Option<LayerTransform> {
        let mask = layer.mask.as_ref()?;
        let Some(edit) = self.transform_edit.as_ref() else {
            return mask.placement;
        };
        if let Some(group) = edit.group.as_ref() {
            let Some(original) = group.original(layer.id) else {
                return mask.placement;
            };
            // A distortion carries a mask covering a linked layer with it; any
            // other mask stays put.
            if edit.corners.is_some() {
                return if mask.is_linked && mask.placement.is_none() {
                    None
                } else {
                    Some(mask.placement.unwrap_or(layer.transform))
                };
            }
            let moved = original.following(&group.box_, &edit.draft);
            return mask.placement_moving_layer(&layer.transform, &moved);
        }
        if edit.layer_id != layer.id || edit.floating.is_some() {
            return mask.placement;
        }
        if edit.mask {
            return if edit.draft.same_placement(&layer.transform) {
                None
            } else {
                Some(edit.draft)
            };
        }
        if edit.corners.is_some() {
            return if mask.is_linked && mask.placement.is_none() {
                None
            } else {
                Some(mask.placement.unwrap_or(layer.transform))
            };
        }
        mask.placement_moving_layer(&layer.transform, &edit.draft)
    }

    // MARK: Live masks (LiveLayerMask.swift)

    /// `LiveMaskGraph.validate`: along every clipping chain the source exists,
    /// is a non-group pixel layer without an adjustment, and the chain has no
    /// cycle. Returns false on any violation.
    fn live_mask_chain_ok(document: &CanvasDocument, extra: Option<(Uuid, Uuid)>) -> bool {
        use std::collections::HashSet;
        let mut records: HashSet<Uuid> = HashSet::new();
        for layer in &document.layers {
            if !records.insert(layer.id) {
                return false;
            }
        }
        for layer in &document.layers {
            let mut path: HashSet<Uuid> = HashSet::new();
            let mut current = Some(layer.id);
            let source_of = |id: Uuid, document: &CanvasDocument| -> Option<Uuid> {
                if extra.map(|(target, _source)| target == id).unwrap_or(false) {
                    Some(extra.unwrap().1)
                } else {
                    document.layer(id).and_then(|l| l.mask_source_id)
                }
            };
            while let Some(id) = current {
                if path.len() >= 256 || !path.insert(id) {
                    return false;
                }
                if let Some(source) = source_of(id, document) {
                    let Some(record) = document.layer(source) else {
                        return false;
                    };
                    if record.is_group || record.adjustment.is_some() {
                        return false;
                    }
                    current = Some(source);
                } else {
                    current = None;
                }
            }
        }
        true
    }

    /// `canLinkMask(source:target:)`.
    pub fn can_link_mask(&self, source: Uuid, target: Uuid) -> bool {
        if !self.can_edit_layers() || source == target {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let ok_kind = |id: Uuid, require_no_adjustment: bool| {
            document
                .layer(id)
                .map(|l| !l.is_group && (!require_no_adjustment || l.adjustment.is_none()))
                .unwrap_or(false)
        };
        if !ok_kind(source, true) || !ok_kind(target, false) {
            return false;
        }
        // The link must keep every clipping chain valid.
        Self::live_mask_chain_ok(document, Some((target, source)))
    }

    /// `linkMask(source:target:)` — a clipping mask from `source`'s coverage.
    pub fn link_mask(&mut self, source: Uuid, target: Uuid) -> bool {
        if !self.can_link_mask(source, target) {
            return false;
        }
        if self
            .document
            .as_ref()
            .and_then(|d| d.layer(target))
            .and_then(|l| l.mask_source_id)
            == Some(source)
        {
            return true;
        }
        self.begin_edit("Create Clipping Mask");
        if let Some(document) = self.document.as_mut()
            && let Some(layer) = document.layer_mut(target)
        {
            layer.mask_source_id = Some(source);
        }
        self.end_edit();
        true
    }

    /// `removeLiveMask(from:)` — releasing a base releases the clipped
    /// children above it that share that base; releasing a child leaves lower
    /// siblings untouched. One "Release Clipping Mask" undo step.
    pub fn remove_live_mask(&mut self, target: Uuid) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(target_layer) = document.layer(target) else {
            return;
        };
        let Some(source) = target_layer.mask_source_id else {
            return;
        };
        let siblings: Vec<Uuid> = document
            .layers
            .iter()
            .filter(|l| l.parent_id == target_layer.parent_id)
            .map(|l| l.id)
            .collect();
        let Some(target_index) = siblings.iter().position(|id| *id == target) else {
            return;
        };
        let mut releases = Vec::new();
        for id in &siblings[target_index..] {
            let keeps =
                *id == target || document.layer(*id).and_then(|l| l.mask_source_id) == Some(source);
            if !keeps {
                break;
            }
            releases.push(*id);
        }
        self.begin_edit("Release Clipping Mask");
        if let Some(document) = self.document.as_mut() {
            for id in releases {
                if let Some(layer) = document.layer_mut(id) {
                    layer.mask_source_id = None;
                }
            }
        }
        self.end_edit();
    }

    /// `canToggleClippingMask` — clipped already, or the next lower sibling
    /// (or its base) can supply the clipping.
    pub fn can_toggle_clipping_mask(&self, id: Uuid) -> bool {
        if !self.can_edit_layers() {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(layer) = document.layer(id) else {
            return false;
        };
        if layer.is_group {
            return false;
        }
        if layer.mask_source_id.is_some() {
            return true;
        }
        let siblings: Vec<&ImageLayer> = document
            .layers
            .iter()
            .filter(|l| l.parent_id == layer.parent_id)
            .collect();
        let Some(index) = siblings.iter().position(|l| l.id == id) else {
            return false;
        };
        if index == 0 || siblings[index - 1].is_group {
            return false;
        }
        let below = siblings[index - 1];
        let source = below.mask_source_id.unwrap_or(below.id);
        self.can_link_mask(source, id)
    }

    /// Option-click (`toggleClippingMask`): clip to the next lower sibling,
    /// sharing its base when it is already clipped; clipped already, release.
    pub fn toggle_clipping_mask(&mut self, id: Uuid) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = document.layer(id) else {
            return;
        };
        if layer.is_group {
            return;
        }
        if layer.mask_source_id.is_some() {
            self.remove_live_mask(id);
            return;
        }
        let siblings: Vec<&ImageLayer> = document
            .layers
            .iter()
            .filter(|l| l.parent_id == layer.parent_id)
            .collect();
        let Some(index) = siblings.iter().position(|l| l.id == id) else {
            return;
        };
        if index == 0 {
            return;
        }
        let below = siblings[index - 1];
        if below.is_group {
            return;
        }
        let source = below.mask_source_id.unwrap_or(below.id);
        self.link_mask(source, id);
    }

    /// `LiveMaskBaker.bake`: the live dependency (the clipping source chain)
    /// baked into the target's pixels; the target's own mask and appearance
    /// stay live. Returns the baked pixels.
    pub fn bake_live_mask(&self, target: Uuid) -> Option<ImportedImage> {
        let document = self.document.as_ref()?;
        let layer = document.layer(target)?;
        let original = layer.asset.as_ref()?;
        let width = original.image.width;
        let height = original.image.height;
        let mut canvas = original.image.as_ref().clone();
        // The source chain, lowest first.
        let mut chain = Vec::new();
        let mut current = layer.mask_source_id;
        let mut depth = 0;
        while let Some(id) = current {
            if depth >= 64 {
                return None;
            }
            let Some(source) = document.layer(id) else {
                break;
            };
            chain.push(source);
            current = source.mask_source_id;
            depth += 1;
        }
        for source in chain.into_iter().rev() {
            let Some(asset) = source.asset.as_ref() else {
                continue;
            };
            let mut placed = Bitmap::new(width, height);
            crate::render::draw_layer_pixels(asset.image.as_ref(), &source.transform, &mut placed);
            // A raster mask on the source multiplies in, as the bake's
            // `LayerRenderer.draw` does.
            if let Some(mask) = &source.mask
                && mask.is_enabled
                && let Some(grid) =
                    crate::render::mask_clip_image(mask, &source.transform, width, height)
            {
                for i in 0..(width * height) {
                    let coverage = grid.pixel(i % width, i / width)[0] as f32 / 255.0;
                    let o = i * 4;
                    for c in 0..4 {
                        placed.data[o + c] = (placed.data[o + c] as f32 * coverage) as u8;
                    }
                }
            }
            // Source-over onto the target's pixels.
            crate::raster::draw_over(
                &mut canvas,
                &placed,
                crate::geometry::CGRect::new(0.0, 0.0, width as f64, height as f64),
                crate::transform::LayerSampling::High,
            );
        }
        Some(ImportedImage::new(canvas, original.name.clone()))
    }

    /// `deleteWithLiveMaskChoice` — when deleted layers supply live masks to
    /// layers that stay: `bake` keeps the masked appearance in pixels,
    /// `!bake` unlinks. Returns false (having done nothing) when no layer
    /// depends on the deleted ones. The bake/unlink dialog is the app's.
    pub fn delete_with_live_mask_choice(&mut self, ids: &[Uuid], bake: bool) -> bool {
        let Some(document) = self.document.clone() else {
            return false;
        };
        let mut removed = std::collections::HashSet::new();
        for id in ids {
            removed.extend(document.descendant_ids(*id));
            removed.insert(*id);
        }
        let targets: Vec<Uuid> = document
            .layers
            .iter()
            .filter(|l| {
                !removed.contains(&l.id)
                    && l.mask_source_id
                        .map(|s| removed.contains(&s))
                        .unwrap_or(false)
            })
            .map(|l| l.id)
            .collect();
        if targets.is_empty() {
            return false;
        }
        let baked: std::collections::HashMap<Uuid, ImportedImage> = if bake {
            targets
                .iter()
                .filter_map(|t| self.bake_live_mask(*t).map(|image| (*t, image)))
                .collect()
        } else {
            std::collections::HashMap::new()
        };
        self.delete_selected_layers_with_baked(ids, baked);
        true
    }

    /// `finishDeletingLayers` — the deletion that carries baked pixels for
    /// dependents. Several ids delete in one "Delete Layers" undo step whose
    /// after-snapshot includes the baked pixels (LiveLayerMask.swift:149-153:
    /// the passed ids are deleted, not the selection).
    fn delete_selected_layers_with_baked(
        &mut self,
        ids: &[Uuid],
        baked: std::collections::HashMap<Uuid, ImportedImage>,
    ) {
        if ids.len() <= 1 {
            if let Some(id) = ids.first() {
                self.delete_layer_with_baked(*id, baked);
            }
            return;
        }
        self.begin_edit("Delete Layers");
        for id in ids {
            self.delete_layer_with_baked(*id, baked.clone());
        }
        self.end_edit();
    }
}
