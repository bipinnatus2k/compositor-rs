//! Port of the session's selection extensions: `Selection.swift`,
//! `SelectionEdits.swift` (edits plus Cmd-drag / Cmd-arrow pixel moves) and
//! `SelectionClipboard.swift`.

use super::*;

impl EditorSession {
    // MARK: Selections

    pub fn selection_mode(&self, shift: bool, option: bool) -> SelectionMode {
        if option {
            SelectionMode::Subtract
        } else if shift {
            SelectionMode::Add
        } else {
            self.selection_mode_choice
        }
    }

    pub fn begin_lasso(&mut self, at: CGPoint, mode: SelectionMode) {
        if !self.tool.is_selection_tool() || self.tool == NavigationTool::Wand {
            return;
        }
        // Selection tools need an editable document (canEditSelection), and a
        // click never starts a draft while a selection move is in flight.
        if !self.can_edit_layers() || self.selection_move_origin.is_some() {
            return;
        }
        if self.tool == NavigationTool::Marquee {
            let anchor = CGPoint::new(at.x.round(), at.y.round());
            self.lasso_draft = Some(LassoDraft {
                points: vec![anchor],
                cursor: None,
                mode,
                kind: self.marquee_kind,
                anchor: Some(anchor),
            });
        } else {
            self.lasso_draft = Some(LassoDraft {
                points: vec![at],
                cursor: None,
                mode,
                kind: self.lasso_kind,
                anchor: None,
            });
        }
    }

    /// Marquee drag, snapped to whole pixels.
    pub fn drag_marquee(&mut self, to: CGPoint, square: bool, from_center: bool) {
        if !to.x.is_finite() || !to.y.is_finite() {
            return;
        }
        let Some(draft) = self.lasso_draft.as_mut() else {
            return;
        };
        if !matches!(draft.kind, LassoKind::Rectangle | LassoKind::Ellipse) {
            return;
        }
        let Some(anchor) = draft.anchor else { return };
        let rect = drag_box_rect(anchor, to, square, from_center);
        draft.points = vec![
            CGPoint::new(rect.min_x(), rect.min_y()),
            CGPoint::new(rect.max_x(), rect.min_y()),
            CGPoint::new(rect.max_x(), rect.max_y()),
            CGPoint::new(rect.min_x(), rect.max_y()),
        ];
    }

    /// Adds an outline point; points closer than a quarter pixel are skipped.
    pub fn extend_lasso(&mut self, to: CGPoint) {
        if !to.x.is_finite() || !to.y.is_finite() {
            return;
        }
        let Some(draft) = self.lasso_draft.as_mut() else {
            return;
        };
        if let Some(last) = draft.points.last()
            && last.distance_to(to) < 0.25
        {
            return;
        }
        draft.points.push(to);
    }

    pub fn move_lasso_cursor(&mut self, to: Option<CGPoint>) {
        if let Some(draft) = self.lasso_draft.as_mut() {
            draft.cursor = to;
        }
    }

    pub fn remove_last_lasso_point(&mut self) {
        if let Some(draft) = self.lasso_draft.as_mut() {
            draft.points.pop();
            if draft.points.is_empty() {
                self.lasso_draft = None;
            }
        }
    }

    pub fn cancel_lasso(&mut self) {
        self.lasso_draft = None;
    }

    pub fn toggle_marquee_kind(&mut self) {
        self.cancel_lasso();
        self.marquee_kind = if self.marquee_kind == LassoKind::Rectangle {
            LassoKind::Ellipse
        } else {
            LassoKind::Rectangle
        };
    }

    pub fn toggle_lasso_kind(&mut self) {
        self.cancel_lasso();
        self.lasso_kind = if self.lasso_kind == LassoKind::Freehand {
            LassoKind::Polygonal
        } else {
            LassoKind::Freehand
        };
    }

    /// Closes the outline and combines it with the current selection.
    pub fn finish_lasso(&mut self) {
        let Some(draft) = self.lasso_draft.take() else {
            return;
        };
        let outline = if draft.kind == LassoKind::Ellipse && draft.points.len() == 4 {
            let xs: Vec<f64> = draft.points.iter().map(|p| p.x).collect();
            let ys: Vec<f64> = draft.points.iter().map(|p| p.y).collect();
            let min_x = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_x = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min_y = ys.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_y = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            SelectionPath::ellipse(CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y))
        } else {
            if draft.points.len() < 3 {
                if draft.mode == SelectionMode::Replace {
                    self.deselect();
                }
                return;
            }
            SelectionPath::from_polygon(draft.points.clone())
        };
        let bounds_valid = outline
            .bounds()
            .map(|b| b.width() > 0.0 && b.height() > 0.0)
            .unwrap_or(false);
        if !bounds_valid {
            if draft.mode == SelectionMode::Replace {
                self.deselect();
            }
            return;
        }
        let name = match draft.kind {
            LassoKind::Freehand => "Lasso",
            LassoKind::Polygonal => "Polygonal Lasso",
            LassoKind::Ellipse => "Elliptical Marquee",
            LassoKind::Rectangle => "Rectangular Marquee",
        };
        self.apply_selection(outline, draft.mode, name);
    }

    pub fn apply_selection(&mut self, shape: SelectionPath, mode: SelectionMode, name: &str) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let canvas = SelectionPath::rect(CGRect::from_origin_size(CGPoint::ZERO, document.size()));
        let clipped = shape.intersection(&canvas);
        let result = match mode {
            SelectionMode::Replace => clipped,
            SelectionMode::Add => match &document.selection {
                Some(current) => current.path.union(&clipped),
                None => clipped,
            },
            SelectionMode::Subtract => {
                let Some(current) = &document.selection else {
                    return;
                };
                current.path.subtract(&clipped)
            }
        };
        self.set_selection(
            Some(DocumentSelection {
                path: result,
                antialiased: self.selection_antialiased,
                feather: 0.0,
            }),
            name,
        );
    }

    pub fn set_selection(&mut self, value: Option<DocumentSelection>, name: &str) {
        {
            let current = self.document.as_ref().and_then(|d| d.selection.clone());
            if current == value {
                return;
            }
        }
        self.begin_edit(name);
        if let Some(doc) = self.document.as_mut() {
            doc.selection = value;
        }
        self.end_edit();
    }

    /// Cmd-click on a mask thumbnail: the mask's black (hidden) areas become
    /// the selection (MaskTracing.swift:76-85). Shift adds; Option subtracts.
    pub fn load_mask_selection(&mut self, layer_id: Uuid, mode: SelectionMode) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !self.can_edit_layers() {
            return;
        }
        let Some(layer) = document.layer(layer_id) else {
            return;
        };
        let Some(mask) = &layer.mask else {
            return;
        };
        let image = mask.asset.image.as_ref();
        let gray = crate::raster::GrayMask::from_raw(
            image.width,
            image.height,
            image.data.chunks_exact(4).map(|px| px[0]).collect(),
        );
        let Some(traced) = crate::tools::mask_tracing::dark_pixels(&gray) else {
            self.brush_error = Some("Nothing selected".to_string());
            return;
        };
        let to_document = crate::raster::pixel_to_document(
            &layer.mask_transform(),
            image.width as f64,
            image.height as f64,
        );
        let outline = traced.transformed(&to_document);
        self.apply_selection(outline, mode, "Load Mask Selection");
    }

    /// Cmd-click on a layer thumbnail: the layer's visible (≥ 50% opaque)
    /// pixels become the selection, ignoring its mask, as in Photoshop
    /// (MaskTracing.swift:87-95). Shift adds; Option subtracts.
    pub fn load_layer_selection(&mut self, layer_id: Uuid, mode: SelectionMode) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !self.can_edit_layers() {
            return;
        }
        let Some(layer) = document.layer(layer_id) else {
            return;
        };
        if layer.is_group {
            return;
        }
        let Some(asset) = &layer.asset else {
            self.brush_error = Some("Nothing selected".to_string());
            return;
        };
        let image = asset.image.as_ref();
        let Some(traced) = crate::tools::mask_tracing::opaque_pixels(image) else {
            self.brush_error = Some("Nothing selected".to_string());
            return;
        };
        let to_document = crate::raster::pixel_to_document(
            &layer.transform,
            image.width as f64,
            image.height as f64,
        );
        let outline = traced.transformed(&to_document);
        self.apply_selection(outline, mode, "Load Layer Selection");
    }

    pub fn select_all(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        self.set_selection(
            Some(DocumentSelection::new(SelectionPath::rect(
                CGRect::from_origin_size(CGPoint::ZERO, document.size()),
            ))),
            "Select All",
        );
    }

    pub fn deselect(&mut self) {
        if self.selection().is_some() {
            self.set_selection(None, "Deselect");
        }
    }

    pub fn invert_selection(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(current) = document.selection.clone() else {
            return;
        };
        let canvas = SelectionPath::rect(CGRect::from_origin_size(CGPoint::ZERO, document.size()));
        self.set_selection(
            Some(DocumentSelection {
                path: canvas.subtract(&current.path),
                antialiased: current.antialiased,
                feather: current.feather,
            }),
            "Inverse",
        );
    }

    /// Arrow-key nudge: 1 px, or 10 px with Shift. Each press is one undo step.
    pub fn nudge_selection(&mut self, dx: f64, dy: f64) {
        if self.selection().is_none() {
            return;
        }
        // Arrow-key nudges move whole pixels.
        let dx = dx.round();
        let dy = dy.round();
        self.begin_edit("Move Selection");
        if let Some(doc) = self.document.as_mut()
            && let Some(selection) = doc.selection.as_mut()
        {
            selection.path = selection.path.translated(dx, dy);
        }
        self.end_edit();
    }

    pub fn can_modify_selection(&self) -> bool {
        self.selection().map(|s| !s.is_empty()).unwrap_or(false)
            && self.can_edit_layers()
            && self.lasso_draft.is_none()
    }

    pub fn expand_selection(&mut self, amount: i64) {
        self.resize_selection(amount as f64, "Expand Selection");
    }

    pub fn contract_selection(&mut self, amount: i64) {
        self.resize_selection(-(amount as f64), "Contract Selection");
    }

    fn resize_selection(&mut self, delta: f64, name: &str) {
        if !self.can_modify_selection() || delta == 0.0 || delta.abs() > 500.0 {
            return;
        }
        let current = self.document.as_ref().unwrap().selection.clone().unwrap();
        // A stroke of width 2|δ| centered on the outline extends |δ| per side.
        let band = current.path.stroke_band(delta.abs() * 2.0);
        let result = if delta > 0.0 {
            let grown = current.path.union(&band);
            if let Some(document) = &self.document {
                let canvas =
                    SelectionPath::rect(CGRect::from_origin_size(CGPoint::ZERO, document.size()));
                grown.intersection(&canvas)
            } else {
                grown
            }
        } else {
            current.path.subtract(&band)
        };
        self.set_selection(
            Some(DocumentSelection {
                path: result,
                antialiased: current.antialiased,
                feather: current.feather,
            }),
            name,
        );
    }

    /// Softens the current selection's edge (`Feather`).
    pub fn feather_selection(&mut self, amount: i64) {
        if !self.can_modify_selection() || amount <= 0 {
            return;
        }
        let current = self.document.as_ref().unwrap().selection.clone().unwrap();
        let softened = (current.feather * current.feather + amount as f64 * amount as f64).sqrt();
        self.set_selection(
            Some(DocumentSelection {
                path: current.path.clone(),
                antialiased: current.antialiased,
                feather: softened.min(250.0),
            }),
            "Feather Selection",
        );
    }

    pub fn confirm_selection_amount(&mut self, amount: i64) {
        let Some(operation) = self.selection_amount_operation else {
            return;
        };
        let limit = if operation == SelectionAmountOperation::Feather {
            250
        } else {
            500
        };
        if !(1..=limit).contains(&amount) {
            return;
        }
        self.selection_amount_operation = None;
        match operation {
            SelectionAmountOperation::Expand => {
                self.selection_expand_amount = amount;
                self.expand_selection(amount);
            }
            SelectionAmountOperation::Contract => {
                self.selection_contract_amount = amount;
                self.contract_selection(amount);
            }
            SelectionAmountOperation::Feather => {
                self.selection_feather_amount = amount;
                self.feather_selection(amount);
            }
        }
    }

    /// The Magic Wand: selects pixels similar to the one at `point`, read from
    /// the active layer (or the composite), combined by `mode`.
    pub fn magic_wand(&mut self, point: CGPoint, mode: SelectionMode) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if point.x < 0.0
            || point.y < 0.0
            || point.x >= document.width as f64
            || point.y >= document.height as f64
        {
            return;
        }
        let sample = self.selection_sample(&document);
        let x = point.x.floor() as usize;
        let y = point.y.floor() as usize;
        let mut mask = vec![0u8; document.width * document.height];
        let count = crate::tools::wand::wand_mask(
            &sample,
            (x, y),
            self.wand_settings.sample_radius,
            self.wand_settings.tolerance.clamp(0, 255),
            self.wand_settings.contiguous,
            &mut mask,
        );
        let Some(count) = count else {
            return;
        };
        if count <= 0 {
            if mode == SelectionMode::Replace {
                self.deselect();
            }
            return;
        }
        let outline = match crate::tools::wand::wand_trace(&mask, document.width, document.height) {
            Ok((points, loops)) => {
                let mut path = SelectionPath::default();
                let mut index = 0usize;
                for length in loops {
                    let len = length as usize;
                    path.rings.push(
                        points[index..index + len]
                            .iter()
                            .map(|(x, y)| CGPoint::new(*x as f64, *y as f64))
                            .collect(),
                    );
                    index += len;
                }
                path
            }
            // The outline walk bails on an untraceably busy selection
            // (MagicWand.tooDetailed); that is an error to show, not silence.
            Err(_) => {
                self.brush_error = Some(
                    "The Magic Wand can't trace this selection — try a lower tolerance.".into(),
                );
                return;
            }
        };
        if mode == SelectionMode::Replace {
            self.set_selection(
                Some(DocumentSelection {
                    path: outline,
                    antialiased: self.selection_antialiased,
                    feather: 0.0,
                }),
                "Magic Wand",
            );
        } else {
            self.apply_selection(outline, mode, "Magic Wand");
        }
    }

    /// What selection-from-image tools read, at document size: the composite,
    /// or the active layer's own pixels without its mask. A folder or blank
    /// layer reads as transparent (selectionSample) — the wand then matches
    /// the empty canvas.
    pub fn selection_sample(&self, document: &CanvasDocument) -> Bitmap {
        if self.wand_settings.sample_all_layers {
            crate::render::draw_live_composite(document)
        } else {
            let mut out = Bitmap::new(document.width, document.height);
            if let Some(layer) = self.active_layer()
                && !layer.is_group
                && let Some(asset) = layer.asset.as_ref()
            {
                crate::render::draw_layer_pixels(asset.image.as_ref(), &layer.transform, &mut out);
            }
            out
        }
    }

    // MARK: Selection edits (SelectionEdits.swift)

    /// Whether the active layer (or its mask) can take a fill or clear right
    /// now (`canEditPixels`).
    pub fn can_edit_pixels(&self) -> bool {
        self.can_edit_layers() && self.active_layer().map(|l| !l.is_group).unwrap_or(false)
    }

    /// `makeRasterEdit(for:)` — the active layer's pixels, or its mask's grid
    /// when the mask is the target.
    fn raster_edit_for_active(&self, document: &CanvasDocument) -> Option<RasterEdit> {
        let id = self.active_layer_id?;
        self.raster_edit_for_layer(document, id, self.is_mask_selected)
    }

    /// `makeRasterEdit(for: layer)` for an arbitrary target — a pending
    /// gradient commits to the layer its draft captured, not to whichever
    /// layer is active when the commit happens.
    //
    // `pub(super)`: the Gradient section (moving out separately) commits to
    // the draft's captured layer.
    pub(super) fn raster_edit_for_layer(
        &self,
        document: &CanvasDocument,
        layer_id: Uuid,
        is_mask: bool,
    ) -> Option<RasterEdit> {
        let layer = document.layer(layer_id)?;
        if layer.is_group {
            return None;
        }
        let (source, transform) = if is_mask {
            let uniform = layer
                .mask
                .as_ref()
                .map(|m| m.asset.image.width <= 1 && m.asset.image.height <= 1)
                .unwrap_or(true);
            (
                // A uniform mask has no pixels to keep: the grid starts white
                // and covers the whole layer.
                layer
                    .mask
                    .as_ref()
                    .filter(|m| !uniform || (m.asset.image.width > 1 && m.asset.image.height > 1))
                    .map(|m| m.asset.image.as_ref().clone()),
                layer.mask_transform(),
            )
        } else {
            (
                layer.asset.as_ref().map(|a| a.image.as_ref().clone()),
                layer.transform,
            )
        };
        let clip = self.selection().map(|s| s.clip(document.size()));
        Some(RasterEdit::new(
            source,
            transform,
            is_mask,
            self.brush_settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        ))
    }

    /// `commitRasterEdit` — the composed grid becomes the targeted layer's (or
    /// mask's) pixels as one undo step.
    fn commit_pixel_edit(&mut self, edit: &RasterEdit, composed: &Bitmap, name: &str) {
        let Some(id) = self.active_layer_id else {
            return;
        };
        self.commit_pixel_edit_for_layer(id, edit, composed, name);
    }

    // `pub(super)`: the Gradient section (moving out separately) commits
    // through this to the draft's captured layer.
    pub(super) fn commit_pixel_edit_for_layer(
        &mut self,
        id: Uuid,
        edit: &RasterEdit,
        composed: &Bitmap,
        name: &str,
    ) {
        let (mut image, transform, _) = edit.snapshot_of(composed);
        self.begin_edit(name);
        if let Some(document) = self.document.as_mut()
            && let Some(layer) = document.layer_mut(id)
        {
            if edit.is_mask {
                let mask = match layer.mask.as_ref() {
                    Some(m) => m.replacing(image),
                    None => LayerMask {
                        asset: image,
                        ..LayerMask::solid(true)
                    },
                };
                layer.mask = Some(mask);
            } else {
                image.name = layer.name.clone();
                layer.asset = Some(image);
                layer.transform = transform;
            }
        }
        self.end_edit();
    }

    /// `fillSelection(with:)` — fills the selection with the foreground or
    /// background color as one undo step. With no selection it fills the whole
    /// layer; on a mask the palette is black/white, so this reveals or hides.
    pub fn fill_selection(&mut self, background: bool) {
        if !self.can_edit_pixels() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(edit) = self.raster_edit_for_active(&document) else {
            return;
        };
        let value = self.palette_color(background);
        let color = if edit.is_mask {
            let v = (value.red * 255.0).round() as u8;
            [v, v, v, 255]
        } else {
            [
                (value.red * 255.0).round() as u8,
                (value.green * 255.0).round() as u8,
                (value.blue * 255.0).round() as u8,
                255,
            ]
        };
        let composed = edit.fill(color);
        self.commit_pixel_edit(
            &edit,
            &composed,
            if edit.is_mask { "Fill Mask" } else { "Fill" },
        );
    }

    /// Delete with a selection: image pixels become transparent; on a mask the
    /// selection fills with the background color (`clearSelectedPixels`).
    pub fn clear_selected_pixels(&mut self) {
        if self.selection().is_none() || !self.can_edit_pixels() {
            return;
        }
        if self.is_mask_selected {
            self.fill_selection(true);
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(edit) = self.raster_edit_for_active(&document) else {
            return;
        };
        if edit.source().is_none() {
            return;
        }
        let composed = edit.clear_pixels();
        self.commit_pixel_edit(&edit, &composed, "Clear");
    }

    // MARK: Moving selected pixels (SelectionEdits.swift's Cmd-drag / Cmd-arrow)

    /// `beginPixelMove`'s guard: `canPaint` for the image target (one visible
    /// layer of its own, no adjustment) with a non-empty selection. The mask
    /// target is refused — masks have no pixels to lift as a drag.
    fn can_pixel_move(&self) -> bool {
        if !self.can_edit_layers() || self.is_mask_selected {
            return false;
        }
        if self.selected_layer_ids.len() != 1 {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(layer) = self.active_layer() else {
            return false;
        };
        if layer.is_group || layer.adjustment.is_some() || layer.asset.is_none() {
            return false;
        }
        if !document.effective_visible_ids().contains(&layer.id) {
            return false;
        }
        self.selection().map(|s| !s.is_empty()).unwrap_or(false)
    }

    /// `beginPixelMove(duplicate:)` — starts moving the selected image
    /// pixels; false when there is nothing to move (no selection, a mask
    /// target, or no pixels under the selection). The pixels lift out of the
    /// layer's raster right away; the document only changes when the move
    /// commits. This is the temporary Move tool's raster-level lift, not
    /// Cmd-T's floating selection (see `begin_selection_transform`).
    /// Moves the outline only (never pixels). The whole drag is one undo
    /// step (`Selection.swift:263-268`).
    pub fn begin_selection_move(&mut self) -> bool {
        let inside = matches!(&self.document, Some(document)
            if document
                .selection
                .as_ref()
                .map(|s| !s.path.is_empty())
                .unwrap_or(false));
        if !inside || !self.can_edit_layers() || self.selection_move_origin.is_some() {
            return false;
        }
        self.begin_edit("Move Selection");
        self.selection_move_origin = self.selection().cloned();
        true
    }

    /// Offsets the outline from the drag's start, rounded to whole pixels so
    /// edges stay crisp. The outline is not re-clipped, so it can leave the
    /// canvas and come back intact (`Selection.swift:271-279`).
    pub fn move_selection_outline(&mut self, offset: CGSize) {
        let Some(origin) = &self.selection_move_origin else {
            return;
        };
        let shift = crate::geometry::CGAffineTransform::translation(
            offset.width.round(),
            offset.height.round(),
        );
        let path = origin.path.transformed(&shift);
        if let Some(document) = self.document.as_mut() {
            document.selection = Some(DocumentSelection {
                path,
                antialiased: origin.antialiased,
                feather: origin.feather,
            });
        }
    }

    pub fn end_selection_move(&mut self) {
        if self.selection_move_origin.is_none() {
            return;
        }
        self.selection_move_origin = None;
        self.end_edit();
    }

    pub fn begin_pixel_move(&mut self, duplicate: bool) -> bool {
        if self.pixel_move.is_some() || !self.can_pixel_move() {
            return false;
        }
        let Some(document) = self.document.clone() else {
            return false;
        };
        let Some(mut edit) = self.raster_edit_for_active(&document) else {
            return false;
        };
        if !edit.lift_selection() {
            return false;
        }
        self.finish_opacity_edit();
        let Some(origin) = self.selection().cloned() else {
            return false;
        };
        self.pixel_move = Some(PixelMove {
            raster: edit,
            origin,
            duplicate,
            offset: CGSize::ZERO,
        });
        true
    }

    /// `movePixels(by:)` — previews the pixels `offset` document pixels
    /// (whole pixels) away. The stored selection stays put until the commit;
    /// the outline draws from `pixel_move.moved_selection()`.
    pub fn drag_pixel_move(&mut self, offset: CGSize) {
        let rounded = CGSize::new(offset.width.round(), offset.height.round());
        let Some(move_state) = self.pixel_move.as_mut() else {
            return;
        };
        if move_state
            .raster
            .move_lifted(rounded, move_state.duplicate)
            .is_none()
        {
            // As in Swift: a rebuild that fails cancels the whole move.
            self.pixel_move = None;
            self.brush_error = Some("The pixels could not be moved.".into());
            return;
        }
        move_state.offset = rounded;
    }

    /// `displayedSelection` — the outline to draw: during a pixel move, the
    /// original shifted by the drag; while a floating selection transform is
    /// open, the selection where the floating pixels now sit.
    pub fn displayed_selection(&self) -> Option<DocumentSelection> {
        if let Some(move_state) = self.pixel_move.as_ref() {
            return Some(move_state.moved_selection());
        }
        if let Some(edit) = self.transform_edit.as_ref()
            && let Some(matrix) = Self::floating_selection_transform(edit)
            && let Some(selection) = self.selection()
        {
            return Some(DocumentSelection {
                path: selection.path.transformed(&matrix),
                antialiased: selection.antialiased,
                feather: selection.feather,
            });
        }
        self.selection().cloned()
    }

    /// `finishPixelMove` — commits the pixels and the moved outline together
    /// as one "Move Pixels" (or "Duplicate Pixels") undo step. A drag that
    /// never moved commits nothing, as in Swift's `offset != .zero` check.
    pub fn end_pixel_move(&mut self) {
        let Some(move_state) = self.pixel_move.take() else {
            return;
        };
        if move_state.offset == CGSize::ZERO {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        let Some(composed) = move_state.moved_pixels() else {
            self.brush_error = Some("The pixels could not be moved.".into());
            return;
        };
        let (mut image, transform, _) = move_state.raster.snapshot_of(&composed);
        let moved = move_state.moved_selection();
        self.begin_edit(if move_state.duplicate {
            "Duplicate Pixels"
        } else {
            "Move Pixels"
        });
        if let Some(document) = self.document.as_mut() {
            if let Some(layer) = document.layer_mut(id) {
                image.name = layer.name.clone();
                layer.asset = Some(image);
                layer.transform = transform;
            }
            document.selection = Some(moved);
        }
        self.end_edit();
    }

    /// `cancelPixelMove` — the document was never modified during the drag,
    /// so dropping the lifted pixels restores everything.
    pub fn cancel_pixel_move(&mut self) {
        self.pixel_move = None;
    }

    /// `nudgePixels(dx:dy:)` — Cmd-arrow moves the selected pixels 1 px
    /// (10 px with Shift, folded into dx/dy by the caller) as one undo step.
    /// Swift beeps app-side when there is nothing to move.
    pub fn nudge_pixels(&mut self, dx: f64, dy: f64) {
        if !self.begin_pixel_move(false) {
            return;
        }
        self.drag_pixel_move(CGSize::new(dx, dy));
        self.end_pixel_move();
    }

    /// `canInvert` — the core conditions; the app conjuncts its own UI state.
    pub fn can_invert(&self) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some(layer) = self.active_layer() else {
            return false;
        };
        if self.selected_layer_ids.len() != 1 || (layer.is_group && !self.is_mask_selected) {
            return false;
        }
        if !document.effective_visible_ids().contains(&layer.id) {
            return false;
        }
        if self.selection().map(|s| s.is_empty()).unwrap_or(false) {
            return false;
        }
        if self.is_mask_selected {
            layer.mask.as_ref().map(|m| m.is_enabled).unwrap_or(false)
        } else {
            layer.asset.is_some()
        }
    }

    /// Cmd-I: inverts the layer's colors (transparency kept) or its mask,
    /// inside the selection or across the whole layer without one.
    pub fn invert_pixels(&mut self) {
        if !self.can_invert() {
            return;
        }
        self.commit_transform();
        if !self.can_invert() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        let mask = self.is_mask_selected;
        let Some(mut image) = (if mask {
            layer.mask.as_ref().map(|m| m.asset.image.as_ref().clone())
        } else {
            layer.asset.as_ref().map(|a| a.image.as_ref().clone())
        }) else {
            return;
        };
        self.finish_opacity_edit();
        let clip = self.selection().map(|s| s.clip(document.size()));
        // A uniform 1×1 mask can't hold a partial selection; give it the
        // layer's pixel grid first (expandedUniformMask).
        if mask && clip.is_some() && image.width <= 1 && image.height <= 1 {
            let width = layer
                .asset
                .as_ref()
                .map(|a| a.image.width)
                .unwrap_or(layer.size().width.round() as usize);
            let height = layer
                .asset
                .as_ref()
                .map(|a| a.image.height)
                .unwrap_or(layer.size().height.round() as usize);
            if width > 0 && height > 0 && width * height <= 100_000_000 {
                let v = image.pixel(0, 0)[0];
                let mut expanded = Bitmap::new(width, height);
                for chunk in expanded.data.chunks_exact_mut(4) {
                    chunk[0] = v;
                    chunk[1] = v;
                    chunk[2] = v;
                    chunk[3] = 255;
                }
                image = expanded;
            }
        }
        let grid_transform = if mask {
            layer.mask_transform()
        } else {
            layer.transform
        };
        let to_doc = crate::raster::pixel_to_document(
            &grid_transform,
            image.width as f64,
            image.height as f64,
        );
        for y in 0..image.height {
            for x in 0..image.width {
                let coverage = match &clip {
                    Some(clip) => {
                        let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&to_doc);
                        if doc.x < 0.0
                            || doc.y < 0.0
                            || doc.x >= document.width as f64
                            || doc.y >= document.height as f64
                        {
                            0.0
                        } else {
                            clip.alpha_at(doc.x.floor() as usize, doc.y.floor() as usize)
                        }
                    }
                    None => 1.0,
                };
                if coverage <= 0.0 {
                    continue;
                }
                let o = (y * image.width + x) * 4;
                for c in 0..3 {
                    let inverted = 255 - image.data[o + c];
                    image.data[o + c] = (inverted as f32 * coverage
                        + image.data[o + c] as f32 * (1.0 - coverage))
                        .round() as u8;
                }
            }
        }
        self.begin_edit(if mask { "Invert Mask" } else { "Invert" });
        if let Some(document) = self.document.as_mut()
            && let Some(current) = document.layer_mut(layer.id)
        {
            if mask {
                if let Some(m) = current.mask.as_ref() {
                    current.mask = Some(m.replacing(ImportedImage::new(image, layer.name.clone())));
                }
            } else {
                current.asset = Some(ImportedImage::new(image, layer.name.clone()));
            }
        }
        self.end_edit();
    }

    /// `addMask(revealing:)` — with a selection the mask starts as the inverse
    /// of the reveal state (LayerMask.swift:233-260: "so a white mask hides
    /// the selection"): `revealing` fills white, then knocks the selection out
    /// to black, so only the unselected pixels show; `!revealing` hides only
    /// the selected pixels. Without a selection it is the plain reveal/hide
    /// mask.
    pub fn add_mask_from_selection(&mut self, revealing: bool) {
        if self.selection().is_none() {
            self.add_layer_mask(revealing);
            return;
        }
        if !self.can_edit_mask()
            || self
                .active_layer()
                .map(|l| l.mask.is_some())
                .unwrap_or(true)
        {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        let width = layer
            .asset
            .as_ref()
            .map(|a| a.image.width)
            .unwrap_or(layer.size().width.round() as usize);
        let height = layer
            .asset
            .as_ref()
            .map(|a| a.image.height)
            .unwrap_or(layer.size().height.round() as usize);
        if width == 0 || height == 0 || width * height > 100_000_000 {
            // Swift throws ProjectError.tooLarge here and surfaces it through
            // brushError (LayerMask.swift:241, :259) — the status line reads
            // it; returning silently would leave the user guessing.
            self.brush_error = Some(
                "This project exceeds the supported canvas, layer, file-size, or 100-megapixel image limit."
                    .to_string(),
            );
            return;
        }
        let base = if revealing { 255u8 } else { 0u8 };
        let fill = 255 - base;
        let mut grid = Bitmap::new(width, height);
        for chunk in grid.data.chunks_exact_mut(4) {
            chunk[0] = base;
            chunk[1] = base;
            chunk[2] = base;
            chunk[3] = 255;
        }
        if let Some(clip) = self.selection().map(|s| s.clip(document.size())) {
            let to_doc =
                crate::raster::pixel_to_document(&layer.transform, width as f64, height as f64);
            for y in 0..height {
                for x in 0..width {
                    let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&to_doc);
                    if doc.x < 0.0
                        || doc.y < 0.0
                        || doc.x >= document.width as f64
                        || doc.y >= document.height as f64
                    {
                        continue;
                    }
                    let a = clip.alpha_at(doc.x.floor() as usize, doc.y.floor() as usize);
                    if a <= 0.0 {
                        continue;
                    }
                    let o = (y * width + x) * 4;
                    let v = grid.data[o] as f32;
                    let value = (v * (1.0 - a) + fill as f32 * a).round() as u8;
                    grid.data[o] = value;
                    grid.data[o + 1] = value;
                    grid.data[o + 2] = value;
                }
            }
        }
        let id = layer.id;
        self.finish_opacity_edit();
        self.begin_edit("Add Mask from Selection");
        if let Some(document) = self.document.as_mut() {
            if let Some(current) = document.layer_mut(id) {
                current.mask = Some(LayerMask {
                    asset: ImportedImage::new(grid, "Layer Mask"),
                    is_enabled: true,
                    placement: None,
                    is_linked: true,
                });
            }
            document.selection = None;
        }
        self.is_mask_selected = true;
        self.end_edit();
    }

    /// `addAdjustment(kind:)` — a new adjustment layer above the active layer,
    /// parented like a sibling.
    pub fn add_adjustment(&mut self, kind: crate::adjustment::AdjustmentKind) {
        use crate::adjustment::{AdjustmentKind, LayerAdjustment};
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if document.layers.len() >= 10_000 {
            return;
        }
        let mut layer = ImageLayer::blank(kind.label(), document.size());
        let mut adjustment = LayerAdjustment::new(kind);
        // A new Gradient Map runs from the foreground to the background color,
        // as in Photoshop; Grain gets a pattern of its own.
        if kind == AdjustmentKind::GradientMap {
            let mut settings = crate::adjust::GradientMapSettings::default();
            settings.shadows =
                crate::adjust::AdjustmentColor::from_palette(self.palette_color(false));
            settings.highlights =
                crate::adjust::AdjustmentColor::from_palette(self.palette_color(true));
            adjustment.gradient_map_settings = Some(settings);
        }
        if kind == AdjustmentKind::Grain {
            let mut settings = crate::adjust::GrainSettings::default();
            settings.seed = rand_seed();
            adjustment.grain_settings = Some(settings);
        }
        if kind == AdjustmentKind::AddNoise {
            adjustment.noise_seed = Some(rand_seed());
        }
        layer.adjustment = Some(adjustment);
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let index = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        self.begin_edit(&format!("New {} Adjustment", kind.label()));
        if let Some(document) = self.document.as_mut() {
            let id = layer.id;
            document
                .layers
                .insert(index.min(document.layers.len()), layer);
            self.active_layer_id = Some(id);
        }
        self.end_edit();
    }

    /// `updateAdjustment(id:value:)`.
    pub fn update_adjustment(&mut self, id: Uuid, value: crate::adjustment::LayerAdjustment) {
        if !value.is_valid() {
            return;
        }
        if let Some(document) = self.document.as_mut()
            && let Some(layer) = document.layer_mut(id)
        {
            layer.adjustment = Some(value);
        }
    }

    // MARK: Clipboard (SelectionClipboard.swift)

    /// Whole-pixel bounds of what Copy takes: the selection, or the whole
    /// canvas without one. Boolean ops leave float noise, so round with a
    /// tolerance rather than letting it add a whole pixel.
    pub fn selection_copy_region(&self) -> Option<CGRect> {
        let document = self.document.as_ref()?;
        let canvas = CGRect::new(0.0, 0.0, document.width as f64, document.height as f64);
        let bounds = self
            .selection()
            .map(|s| s.coverage_bounds())
            .unwrap_or(canvas);
        let tolerance = 0.001;
        let min_x = (bounds.min_x() + tolerance).floor();
        let min_y = (bounds.min_y() + tolerance).floor();
        let region = CGRect::new(
            min_x,
            min_y,
            (bounds.max_x() - tolerance).ceil() - min_x,
            (bounds.max_y() - tolerance).ceil() - min_y,
        )
        .intersection(&canvas);
        if region.is_null() || region.width() < 1.0 || region.height() < 1.0 {
            return None;
        }
        Some(region)
    }

    pub fn can_copy_pixels(&self) -> bool {
        if !self.can_edit_layers() || self.selection().map(|s| s.is_empty()).unwrap_or(false) {
            return false;
        }
        match self.active_layer() {
            Some(layer) if self.is_mask_selected => layer.mask.is_some(),
            Some(layer) if !layer.is_group => layer.asset.is_some(),
            _ => false,
        }
    }

    /// The active layer's pixels (or mask as opaque gray) exactly as they sit
    /// on the canvas, clipped to the selection (soft edges kept), or the whole
    /// canvas without one (`renderSelectedPixels`).
    pub fn render_selected_pixels(&self, mask: bool) -> Option<(Bitmap, CGRect)> {
        let document = self.document.as_ref()?;
        let clip = self.selection().map(|s| s.clip(document.size()));
        if let Some(clip) = &clip {
            clip.coverage.as_ref()?;
        }
        let region = self.selection_copy_region()?;
        let layer = self.active_layer()?;
        let mut out = Bitmap::new(region.width() as usize, region.height() as usize);
        if mask {
            let owned = layer.mask.as_ref()?;
            // Outside a placed mask the background shows: transparent for a
            // mask covering its layer, the thumbnail's edge gray otherwise.
            let placement = layer.mask_transform();
            let background = if layer.mask.as_ref().and_then(|m| m.placement).is_none() {
                0.0
            } else {
                LayerMask::background_of(&owned.asset.thumbnail)
            };
            let fill = (background * 255.0).round() as u8;
            for chunk in out.data.chunks_exact_mut(4) {
                chunk[0] = fill;
                chunk[1] = fill;
                chunk[2] = fill;
                chunk[3] = 255;
            }
            let source = owned.asset.image.as_ref().clone();
            let mapping = crate::raster::pixel_to_document(
                &placement,
                source.width as f64,
                source.height as f64,
            )
            .translate(-region.min_x(), -region.min_y());
            crate::raster::draw_affine(&mut out, &source, &mapping, LayerSampling::High, None);
        } else {
            let image = layer.asset.as_ref()?.image.as_ref().clone();
            let mapping = crate::raster::pixel_to_document(
                &layer.transform,
                image.width as f64,
                image.height as f64,
            )
            .translate(-region.min_x(), -region.min_y());
            crate::raster::draw_affine(&mut out, &image, &mapping, layer.transform.sampling, None);
        }
        // Soft selection edges multiply the copy.
        if let Some(clip) = &clip {
            for y in 0..out.height {
                for x in 0..out.width {
                    let dx = region.min_x() as usize + x;
                    let dy = region.min_y() as usize + y;
                    let a = clip.alpha_at(dx, dy);
                    if a < 1.0 {
                        let o = (y * out.width + x) * 4;
                        for c in 0..4 {
                            out.data[o + c] = (out.data[o + c] as f32 * a).round() as u8;
                        }
                    }
                }
            }
        }
        Some((out, region))
    }

    /// Copy Merged: the selection across every visible layer, composited as
    /// the canvas shows it (`renderMergedPixels`).
    pub fn render_merged_pixels(&self) -> Option<(Bitmap, CGRect)> {
        let document = self.document.as_ref()?;
        let clip = self.selection().map(|s| s.clip(document.size()));
        if let Some(clip) = &clip {
            clip.coverage.as_ref()?;
        }
        let region = self.selection_copy_region()?;
        let composite = crate::render::draw_live_composite(document);
        let mut out = Bitmap::new(region.width() as usize, region.height() as usize);
        for y in 0..out.height {
            for x in 0..out.width {
                let dx = region.min_x() as usize + x;
                let dy = region.min_y() as usize + y;
                let mut px = composite.pixel(dx, dy);
                if let Some(clip) = &clip {
                    let a = clip.alpha_at(dx, dy);
                    for c in 0..4 {
                        px[c] = (px[c] as f32 * a) as u8;
                    }
                }
                out.set_pixel(x, y, px);
            }
        }
        Some((out, region))
    }

    /// Cmd-C: copies the selected pixels (or the whole layer).
    pub fn copy_selection(&mut self) {
        if !self.can_copy_pixels() {
            return;
        }
        if let Some((image, region)) = self.render_selected_pixels(self.is_mask_selected) {
            self.pixel_clipboard = Some(PixelClipboard {
                image,
                origin: region.origin,
            });
        }
    }

    /// Shift-Cmd-C: the selection across every visible layer.
    pub fn copy_merged_selection(&mut self) {
        if !self.can_edit_layers() || self.selection().map(|s| s.is_empty()).unwrap_or(false) {
            return;
        }
        if let Some((image, region)) = self.render_merged_pixels() {
            self.pixel_clipboard = Some(PixelClipboard {
                image,
                origin: region.origin,
            });
        }
    }

    /// Cmd-X: copy, then clear the selected pixels.
    pub fn cut_selection(&mut self) {
        if self.selection().is_none() || !self.can_copy_pixels() {
            return;
        }
        self.copy_selection();
        self.clear_selected_pixels();
    }

    /// Cmd-V: pastes as a new layer above the active one, back where it was
    /// copied from (`paste`; the system pasteboard is the app's concern).
    pub fn paste_pixels(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(clipboard) = self.pixel_clipboard.as_ref() else {
            return;
        };
        let image = clipboard.image.clone();
        let origin = clipboard.origin;
        let name = self.next_layer_name();
        self.add_pixel_layer(image, origin, &name, "Paste");
    }

    /// Cmd-J (Layer via Copy): the selection's pixels become a new layer in
    /// place; with no selection the whole layer is duplicated.
    pub fn layer_via_copy(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(layer) = self.active_layer() else {
            return;
        };
        if layer.is_group || self.selection().map(|s| s.is_empty()).unwrap_or(false) {
            return;
        }
        if self.selection().is_none() {
            self.duplicate_active_layer();
            return;
        }
        let mask = self.is_mask_selected;
        if let Some((image, region)) = self.render_selected_pixels(mask) {
            let name = self.next_layer_name();
            self.add_pixel_layer(image, region.origin, &name, "Layer via Copy");
        }
    }

    /// Inserts pixels as a new layer above the active one (inside its folder),
    /// in one undo step. Pasting drops the selection (`addPixelLayer`).
    pub fn add_pixel_layer(&mut self, image: Bitmap, origin: CGPoint, name: &str, edit_name: &str) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let mut layer = ImageLayer::with_asset(ImportedImage::new(image, name), origin);
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let index = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        self.finish_opacity_edit();
        self.begin_edit(edit_name);
        if let Some(document) = self.document.as_mut() {
            let id = layer.id;
            document
                .layers
                .insert(index.min(document.layers.len()), layer);
            document.selection = None;
            self.active_layer_id = Some(id);
            self.selected_layer_ids = vec![id];
        }
        self.end_edit();
    }

    pub fn next_layer_name(&self) -> String {
        let names: std::collections::HashSet<String> = self
            .document
            .as_ref()
            .map(|d| d.layers.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();
        let mut number = 1;
        while names.contains(&format!("Layer {number}")) {
            number += 1;
        }
        format!("Layer {number}")
    }
}
