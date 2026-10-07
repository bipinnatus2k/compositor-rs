//! Port of the session's text tool extension (`TypeTool.swift`).

use super::*;

impl EditorSession {
    // MARK: Text (TypeTool.swift)

    /// `beginText(at:)` — a click on an existing text layer edits it; on
    /// empty canvas it starts new point text in the foreground color.
    pub fn begin_text(&mut self, point: CGPoint, new_layer: bool) {
        if !self.can_edit_layers() || self.text_draft.is_some() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if !point.x.is_finite() || !point.y.is_finite() {
            return;
        }
        let visible = document.effective_visible_ids();
        let target = if new_layer {
            None
        } else {
            document
                .layers
                .iter()
                .rev()
                .find(|l| {
                    visible.contains(&l.id)
                        && l.live_text().is_some()
                        && l.transform.contains(point)
                })
                .map(|l| l.id)
        };
        let target_layer = target.and_then(|id| document.layer(id)).cloned();
        if let Some(target_id) = target {
            self.select_layer(Some(target_id));
        }
        let mut style = target_layer
            .as_ref()
            .and_then(|l| l.live_text().map(|text| text.style.clone()))
            .unwrap_or_else(|| self.text_defaults.clone());
        if target.is_none() {
            style.content = String::new();
            if !self.is_mask_selected {
                let foreground = self.palette_color(false);
                style.red = foreground.red;
                style.green = foreground.green;
                style.blue = foreground.blue;
            }
            style.box_size = None;
        }
        self.tool = NavigationTool::Type;
        self.text_draft = Some(TextDraft {
            document_id: document.id,
            layer_id: target,
            origin: target_layer.as_ref().map(|l| l.origin()).unwrap_or(point),
            transform: target_layer.as_ref().map(|l| l.transform),
            style,
        });
    }

    /// `beginText(in:)` — dragging a box sets the paragraph bounds.
    pub fn begin_text_in(&mut self, rect: CGRect) {
        if !self.can_edit_layers() || self.text_draft.is_some() {
            return;
        }
        if !rect.width().is_finite() || !rect.height().is_finite() {
            return;
        }
        let mut style = self.text_defaults.clone();
        style.box_size = Some(CGSize::new(
            rect.width().round().max(16.0),
            rect.height().round().max(16.0),
        ));
        if !style.box_is_valid() {
            self.brush_error =
                Some("That text box exceeds the 30,000-pixel or 100-megapixel limit.".into());
            return;
        }
        let box_size = style.box_size;
        self.begin_text(rect.origin, true);
        if let Some(draft) = self.text_draft.as_mut() {
            draft.style.box_size = box_size;
        }
    }

    /// `editActiveText` — edits the active layer when it is live text.
    pub fn edit_active_text(&mut self) {
        if !self.can_edit_layers() || self.text_draft.is_some() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        let Some(text) = layer.live_text().cloned() else {
            return;
        };
        self.tool = NavigationTool::Type;
        self.text_draft = Some(TextDraft {
            document_id: document.id,
            layer_id: Some(layer.id),
            origin: layer.origin(),
            transform: Some(layer.transform),
            style: text.style,
        });
    }

    /// `applyText` — rasterizes the draft into the layer it edits (or a new
    /// text layer), as one undo step. Empty new text is a quiet no-op.
    pub fn apply_text(&mut self, draft: TextDraft) -> bool {
        if self.document.as_ref().map(|d| d.id) != Some(draft.document_id)
            || !draft.style.is_valid()
        {
            return false;
        }
        let pending = self.text_draft.take();
        if !self.can_edit_layers() {
            self.text_draft = pending;
            return false;
        }
        let succeeded = self.apply_text_inner(&draft);
        if succeeded {
            self.text_draft = None;
            self.canvas_focus_request += 1;
        } else {
            self.text_draft = pending;
        }
        succeeded
    }

    fn apply_text_inner(&mut self, draft: &TextDraft) -> bool {
        // Blank new text: nothing to make.
        if draft.layer_id.is_none() && draft.style.content.trim().is_empty() {
            return true;
        }
        let Ok(image) = crate::tools::text::text_image(&draft.style) else {
            return false;
        };
        // The asset and the live text share one image, as `liveText`'s
        // identity check requires.
        let image = std::sync::Arc::new(image);
        let text = crate::document::LayerText {
            style: draft.style.clone(),
            image: image.clone(),
        };
        let Some(document) = self.document.clone() else {
            return false;
        };
        if let Some(layer_id) = draft.layer_id {
            let Some(_index) = document.layer_index(layer_id) else {
                return false;
            };
            let Some(layer) = document.layer(layer_id) else {
                return false;
            };
            let Some(live) = layer.live_text() else {
                return false;
            };
            let Some(asset) = layer.asset.as_ref() else {
                return false;
            };
            if live.style == draft.style
                && (draft.transform.is_none() || draft.transform == Some(layer.transform))
            {
                return true;
            }
            let mut transform = draft.transform.unwrap_or(layer.transform);
            // Keep the transformed upper-left corner and the user's scale,
            // rotation and flips.
            let anchor = transform.point(CGPoint::ZERO);
            if draft.transform.is_none() || draft.style.box_size.is_none() {
                transform.size = CGSize::new(
                    image.width as f64 * transform.size.width / asset.image.width as f64,
                    image.height as f64 * transform.size.height / asset.image.height as f64,
                );
                let moved = transform.point(CGPoint::ZERO);
                transform.origin.x += anchor.x - moved.x;
                transform.origin.y += anchor.y - moved.y;
            }
            if !transform.is_valid() {
                return false;
            }
            self.begin_edit("Edit Text");
            if let Some(doc) = self.document.as_mut()
                && let Some(current) = doc.layer_mut(layer_id)
            {
                let name = asset.name.clone();
                let mask_placement = current
                    .mask
                    .as_ref()
                    .filter(|m| m.placement.is_none())
                    .map(|_| current.mask_transform());
                if let Some(placement) = mask_placement
                    && let Some(mask) = current.mask.as_mut()
                {
                    mask.placement = Some(placement);
                }
                let thumbnail = crate::raster::thumbnail_of(&image);
                current.asset = Some(ImportedImage {
                    image,
                    thumbnail: std::sync::Arc::new(thumbnail),
                    name,
                });
                current.text = Some(text);
                current.transform = transform;
            }
            self.end_edit();
            return true;
        }
        // A brand-new text layer above the active one.
        let name = crate::tools::text::layer_name(&draft.style.content);
        let thumbnail = crate::raster::thumbnail_of(&image);
        let asset = ImportedImage {
            image: image.clone(),
            thumbnail: std::sync::Arc::new(thumbnail),
            name: name.clone(),
        };
        let mut layer = ImageLayer::with_asset(asset, draft.origin);
        layer.text = Some(text);
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let layer_id = layer.id;
        let insertion = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        self.begin_edit("New Text Layer");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(insertion.min(doc.layers.len()), layer);
        }
        self.active_layer_id = Some(layer_id);
        self.selected_layer_ids = vec![layer_id];
        self.end_edit();
        self.text_defaults = draft.style.clone();
        true
    }

    /// `finishText` — tool switches, layer switches and Save apply the
    /// pending draft.
    pub fn finish_text(&mut self) -> bool {
        let Some(draft) = self.text_draft.clone() else {
            return true;
        };
        self.apply_text(draft)
    }

    pub fn cancel_text(&mut self) {
        self.text_draft = None;
        self.canvas_focus_request += 1;
    }

    /// `recolorText` — Fill paints a text layer's letters, keeping it
    /// editable. False when the layer isn't live text.
    pub fn recolor_text(&mut self, id: Uuid, color: PaletteColor) -> bool {
        if !self.can_edit_layers() {
            return false;
        }
        let Some(layer) = self.document.as_ref().and_then(|d| d.layer(id)).cloned() else {
            return false;
        };
        let Some(text) = layer.live_text().cloned() else {
            return false;
        };
        let Some(asset) = layer.asset.as_ref() else {
            return false;
        };
        let mut style = text.style.clone();
        if style.red == color.red && style.green == color.green && style.blue == color.blue {
            return true;
        }
        style.red = color.red;
        style.green = color.green;
        style.blue = color.blue;
        if !style.is_valid() {
            return false;
        }
        let Ok(image) = crate::tools::text::text_image(&style) else {
            return false;
        };
        self.finish_opacity_edit();
        self.begin_edit("Fill Text");
        if let Some(document) = self.document.as_mut()
            && let Some(current) = document.layer_mut(id)
        {
            let name = asset.name.clone();
            let image = std::sync::Arc::new(image);
            let thumbnail = crate::raster::thumbnail_of(&image);
            current.asset = Some(ImportedImage {
                image: image.clone(),
                thumbnail: std::sync::Arc::new(thumbnail),
                name,
            });
            current.text = Some(crate::document::LayerText { style, image });
        }
        self.end_edit();
        true
    }

    /// `currentTextStyle` — the draft wins, then the active layer, then the
    /// defaults.
    pub fn current_text_style(&self) -> crate::document::LayerTextStyle {
        self.text_draft
            .as_ref()
            .map(|draft| draft.style.clone())
            .or_else(|| {
                self.active_layer()
                    .and_then(|l| l.live_text().map(|text| text.style.clone()))
            })
            .unwrap_or_else(|| self.text_defaults.clone())
    }

    /// `changeTextStyle` — edits the pending draft, the active live-text
    /// layer (starting a draft), or the defaults.
    pub fn change_text_style(&mut self, change: impl FnOnce(&mut crate::document::LayerTextStyle)) {
        if self.text_draft.is_none() && self.active_layer().and_then(|l| l.live_text()).is_some() {
            self.edit_active_text();
        }
        if let Some(draft) = self.text_draft.as_mut() {
            // Like the defaults branch below, the change must not stick when
            // it leaves the style invalid: work on a copy and only commit a
            // valid result into the open draft.
            let mut style = draft.style.clone();
            change(&mut style);
            if style.is_valid() {
                draft.style = style;
            }
        } else {
            let mut style = self.text_defaults.clone();
            change(&mut style);
            if style.is_valid() {
                self.text_defaults = style;
            }
        }
    }
}
