//! Port of the session's crop and resize extensions: `Crop.swift` and the
//! `CanvasSize`/`ImageSize`/`Trim` operations.

use super::*;

impl EditorSession {
    // MARK: Crop

    pub fn cancel_crop(&mut self) {
        self.crop_rect = None;
        self.crop_drag = None;
    }

    pub fn begin_crop_drag(&mut self, start: CGPoint, mode: CropDragMode) {
        let original = self.crop_rect.unwrap_or_else(|| {
            CGRect::from_origin_size(CGPoint::ZERO, self.document_size().unwrap_or(CGSize::ZERO))
        });
        self.crop_drag = Some(CropDrag {
            start,
            original,
            mode,
        });
        if self.crop_rect.is_none() {
            self.crop_rect = Some(original);
        }
    }

    pub fn drag_crop(&mut self, point: CGPoint, ratio: Option<f64>, symmetric: bool) {
        let Some(drag) = &self.crop_drag else {
            return;
        };
        let mut updated = drag.updated(point, ratio, symmetric);
        // Edges snap to nearby layer and canvas edges (EditorCanvas.dragCrop):
        // 8 document pixels of tolerance, unless snapping is off.
        if self.snapping_enabled {
            let (xs, ys) = self.crop_snap_targets();
            let snap = crate::canvas::CropSnap {
                xs,
                ys,
                tolerance: 8.0,
            };
            updated = snap.apply(updated, drag, point, ratio, symmetric);
        }
        self.crop_rect = Some(updated);
    }

    /// What crop edges snap to: the canvas and other layers' edges, without
    /// centers (`cropSnapTargets`).
    pub fn crop_snap_targets(&self) -> (Vec<f64>, Vec<f64>) {
        let Some(document) = self.document.as_ref() else {
            return (Vec::new(), Vec::new());
        };
        let mut xs = vec![0.0, document.width as f64];
        let mut ys = vec![0.0, document.height as f64];
        for layer in document.render_layers() {
            if layer.asset.is_none() {
                continue;
            }
            let corners = layer.transform.corners();
            let min_x = corners
                .iter()
                .map(|p| p.x)
                .fold(f64::INFINITY, f64::min)
                .round();
            let max_x = corners
                .iter()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max)
                .round();
            let min_y = corners
                .iter()
                .map(|p| p.y)
                .fold(f64::INFINITY, f64::min)
                .round();
            let max_y = corners
                .iter()
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max)
                .round();
            xs.push(min_x);
            xs.push(max_x);
            ys.push(min_y);
            ys.push(max_y);
        }
        (xs, ys)
    }

    pub fn end_crop_drag(&mut self) {
        self.crop_drag = None;
        if let Some(rect) = self.crop_rect
            && !CropGeometry::valid(rect)
        {
            self.crop_rect = None;
        }
    }

    pub fn crop_ratio(&self) -> Option<f64> {
        match self.crop_ratio_choice.as_str() {
            "Original" => self
                .document
                .as_ref()
                .map(|d| d.width as f64 / d.height as f64),
            "1:1" => Some(1.0),
            "4:3" => Some(4.0 / 3.0),
            "16:9" => Some(16.0 / 9.0),
            _ => None,
        }
    }

    pub fn commit_crop(&mut self) {
        let Some(rect) = self.crop_rect else {
            return;
        };
        if !CropGeometry::valid(rect) {
            return;
        }
        let mut options = CanvasSizeOptions::new(rect.width() as usize, rect.height() as usize);
        options.content_offset = Some(CGPoint::new(-rect.min_x(), -rect.min_y()));
        if let Some(doc) = self.document.take() {
            if let Ok(resized) = resize_canvas(&doc, &options) {
                self.begin_edit("Crop");
                self.document = Some(resized);
                self.end_edit();
            } else {
                self.document = Some(doc);
            }
        }
        self.crop_rect = None;
    }

    // MARK: Canvas / Image size, trim

    pub fn resize_canvas_to(&mut self, options: &CanvasSizeOptions) -> bool {
        // A no-op resize changes nothing, not even the selection.
        if let Some(document) = self.document.as_ref() {
            let offset = options.offset(document.width, document.height);
            if options.width == document.width
                && options.height == document.height
                && offset.x == 0.0
                && offset.y == 0.0
            {
                return false;
            }
        }
        let Some(document) = self.document.take() else {
            return false;
        };
        match resize_canvas(&document, options) {
            Ok(resized) => {
                self.begin_edit("Canvas Size");
                self.document = Some(resized);
                self.end_edit();
                true
            }
            Err(_) => {
                self.document = Some(document);
                false
            }
        }
    }

    pub fn resize_image_to(
        &mut self,
        width: usize,
        height: usize,
        resolution: Option<f64>,
        sampling: crate::transform::LayerSampling,
    ) -> bool {
        let Some(document) = self.document.clone() else {
            return false;
        };
        // The resolution-only path still applies (one undo step, selection
        // kept); anything rejected leaves the document untouched.
        match resize_image(&document, width, height, resolution, sampling) {
            Some(resized) => {
                self.begin_edit("Image Size");
                self.document = Some(resized);
                self.end_edit();
                true
            }
            None => false,
        }
    }

    /// Returns false when nothing remained to trim.
    pub fn trim(&mut self, options: &TrimOptions) -> bool {
        if let Some(doc) = self.document.clone()
            && let Some(trimmed) = trim_document(&doc, options)
        {
            self.begin_edit("Trim");
            self.document = Some(trimmed);
            self.end_edit();
            return true;
        }
        false
    }

    /// `displayedTransform(for:)` — the transform a layer draws with right
    /// now: the pending transform draft while it is being transformed. For a
    /// group edit each member follows the box to the draft
    /// (`pendingTransform`, EditorSession.swift:498-502).
    pub fn displayed_transform(&self, layer: &ImageLayer) -> LayerTransform {
        if let Some(edit) = self.transform_edit.as_ref()
            && edit.draft.is_valid()
            && !edit.mask
        {
            if let Some(group) = edit.group.as_ref() {
                if let Some(original) = group.original(layer.id) {
                    return original.following(&group.box_, &edit.draft);
                }
            } else if edit.layer_id == layer.id {
                return edit.draft;
            }
        }
        layer.transform
    }

    /// `toggleShapeKind` — cycles the Shape tool's draw kind
    /// (Rectangle → Ellipse → Rounded → Rectangle).
    pub fn toggle_shape_kind(&mut self) {
        use crate::document::ShapeKind;
        self.shape_kind = match self.shape_kind {
            ShapeKind::Rectangle => ShapeKind::Ellipse,
            ShapeKind::Ellipse => ShapeKind::RoundedRectangle,
            ShapeKind::RoundedRectangle | ShapeKind::Line => ShapeKind::Rectangle,
        };
    }
}
